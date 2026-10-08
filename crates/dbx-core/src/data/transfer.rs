pub use dbx_sql::value_literals::quote_string_literal;
pub use dbx_sql::value_literals::{
    format_ch_array_element, format_ch_array_sql_literal, format_pg_array_element, format_pg_array_sql_literal,
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

use futures::{SinkExt, StreamExt};

#[cfg(test)]
#[path = "transfer/rebuild_tests.rs"]
mod rebuild_tests;

#[cfg(test)]
#[path = "transfer/iris_tests.rs"]
mod iris_tests;

mod db2;
mod ddl_plan;
mod structure_plan;

use crate::connection::{config_for_pool_key, AppState, PoolKind};
use crate::db;

use crate::models::connection::{ConnectionConfig, DatabaseType};
use crate::object_source_sql::{build_executable_object_source_statements, EditableObjectSourceSqlInput};
use crate::query::{
    is_dbx_query_timeout_error, pool_error_action, query_timeout_duration, should_discard_pool_after_query_timeout,
    wait_for_query_opt, PoolErrorAction, QueryExecutionOptions, StreamProgressClock, AGENT_PROTOCOL_MAX_ROWS,
};
use crate::sql::{split_sql_statements, split_sql_statements_for_database};
use crate::sql_dialect::{
    normalize_len_params, qualified_transfer_table, quote_transfer_identifier, transfer_column_identifier,
};

static CANCELLED: std::sync::LazyLock<RwLock<HashSet<String>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashSet::new()));

static MYSQL_COLLATE_CLAUSE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r"(?i)\bCOLLATE\s*=?\s*([A-Za-z0-9_]+)\b").expect("valid MySQL COLLATE clause regex")
});

pub async fn ensure_transfer_source_types_supported(
    state: &AppState,
    request: &TransferRequest,
    source_pool_key: &str,
) -> Result<(), String> {
    if matches!(request.content, TransferContent::StructureOnly) {
        return Ok(());
    }
    let is_doris = {
        let configs = state.configs.read().await;
        false
    };
    {
        return Ok(());
    }
}

fn ensure_transfer_columns_supported(
    request: &TransferRequest,
    is_doris_source: bool,
    table: &str,
    columns: &[db::ColumnInfo],
) -> Result<(), String> {
    {
        return Ok(());
    }
}
// An inline FK constraint *definition line*: an optional `CONSTRAINT <name>`
// prefix followed by `FOREIGN KEY (`. Anchored to the line start so column
// definitions whose COMMENT/DEFAULT strings mention "foreign key" never match.
static INLINE_FOREIGN_KEY_CONSTRAINT_LINE_RE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    Regex::new(r#"(?i)^\s*(?:CONSTRAINT\s+(?:`[^`]*`|"[^"]*"|\S+)\s+)?FOREIGN\s+KEY\s*\("#)
        .expect("valid inline foreign key constraint line regex")
});

// Upper bound for a single generated INSERT/upsert statement. Raised from
// 512 KiB so one `batchSize` page normally becomes one multi-row INSERT.
// MySQL-family targets additionally cap batches by the live
// `max_allowed_packet` of the target server (see transfer_write_mysql_hard_limit).
const MAX_TRANSFER_WRITE_SQL_BYTES: usize = 90 * 1024 * 1024;
/// Conservative write-batch cap when the target's max_allowed_packet cannot be
/// queried (mirrors the pre-batching era limit so a failed probe cannot produce
/// statements larger than what stock MySQL accepts).
const TRANSFER_WRITE_SQL_FALLBACK_BYTES: usize = 512 * 1024;

const TRANSFER_TARGET_TABLE_LOOKUP_LIMIT: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SqlBatchLimits {
    max_rows: usize,
    target_sql_bytes: usize,
    hard_sql_bytes: Option<usize>,
}

impl SqlBatchLimits {
    pub(crate) fn for_database(db_type: &DatabaseType, requested_max_rows: usize) -> Self {
        let max_rows = requested_max_rows.max(1).min(match db_type {
            _ => usize::MAX,
        });
        let target_sql_bytes = match db_type {
            _ => MAX_TRANSFER_WRITE_SQL_BYTES,
        };
        Self { max_rows, target_sql_bytes, hard_sql_bytes: None }
    }

    pub(crate) fn with_hard_sql_bytes(mut self, hard_sql_bytes: Option<usize>) -> Self {
        self.hard_sql_bytes = hard_sql_bytes;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TransferMode {
    #[default]
    Append,
    Overwrite,
    Upsert,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TransferTableNameCase {
    #[default]
    Preserve,
    Lower,
    Upper,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum TransferOwnershipPolicy {
    #[default]
    Preserve,
    Skip,
    ReassignMissing,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TransferContent {
    #[default]
    StructureAndData,
    StructureOnly,
    DataOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransferObjectKind {
    #[default]
    Table,
    View,
    MaterializedView,
    Procedure,
    Function,
    Trigger,
    Sequence,
    Event,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TransferObjectSelection {
    pub object_type: TransferObjectKind,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferObjectFamily {
    Mysql,
}

pub fn transfer_object_family(db_type: &DatabaseType) -> Option<TransferObjectFamily> {
    match db_type {
        DatabaseType::Mysql => Some(TransferObjectFamily::Mysql),

        _ => None,
    }
}

pub fn is_same_transfer_family(a: &DatabaseType, b: &DatabaseType) -> bool {
    match (transfer_object_family(a), transfer_object_family(b)) {
        (Some(fa), Some(fb)) => fa == fb,
        _ => false,
    }
}

pub fn transfer_object_kinds_for_family(family: &TransferObjectFamily) -> Vec<TransferObjectKind> {
    use TransferObjectKind::*;
    match family {
        TransferObjectFamily::Mysql => vec![Table, View, Procedure, Function, Trigger, Event],
    }
}

pub fn transfer_object_kinds(db_type: &DatabaseType) -> Vec<TransferObjectKind> {
    match transfer_object_family(db_type) {
        Some(family) => transfer_object_kinds_for_family(&family),
        None => Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    pub transfer_id: String,
    pub source_connection_id: String,
    pub source_database: String,
    pub source_schema: String,
    pub source_catalog: Option<String>,
    pub target_connection_id: String,
    pub target_database: String,
    pub target_schema: String,
    pub target_catalog: Option<String>,
    pub tables: Vec<String>,
    pub create_table: bool,
    #[serde(default)]
    pub content: TransferContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objects: Option<Vec<TransferObjectSelection>>,
    #[serde(default)]
    pub mode: TransferMode,
    #[serde(default)]
    pub target_table_name_case: TransferTableNameCase,
    #[serde(default = "default_quote_target_column_names")]
    pub quote_target_column_names: bool,
    #[serde(default)]
    pub ownership_policy: TransferOwnershipPolicy,
    pub batch_size: usize,
    /// Optional per-table source filter for this transfer.
    ///
    /// Key = source table name; value is either a bare `WHERE` predicate
    /// (`id <= 90000`) or a complete source `SELECT`
    /// (`select * from t_order where id <= 90000`) used as a derived table.
    /// Missing or empty entries transfer the whole table. Only MySQL- and
    /// PostgreSQL-family sources are supported (see
    /// `transfer_table_filter_supported`).
    #[serde(default)]
    pub table_filters: HashMap<String, String>,
    /// When true, rename the target table to a backup before creating it from the
    /// source structure. Only after the transfer succeeds is the backup dropped.
    /// Requires `create_table = true` and `content != DataOnly`.
    #[serde(default)]
    pub drop_target_before_create: bool,
    /// Production database confirmation for drop_target_before_create.
    #[serde(default)]
    pub drop_target_confirmed: bool,
}

fn default_quote_target_column_names() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferOwnershipPreview {
    pub missing_owners: Vec<String>,
    pub target_owner: String,
    pub rebuild: Option<TransferRebuildPreview>,
    /// Structure statements a structure-only transfer is about to run. Absent for every
    /// other content mode (`dataOnly` has no DDL, `structureAndData` keeps its current
    /// behavior). Built without executing any DDL, but not a frozen script: `start_transfer`
    /// re-reads source and target metadata before executing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structure: Option<TransferStructurePreview>,
}

/// SQL plan preview for a `drop_target_before_create` (rebuild) transfer.
///
/// Built without executing any DDL: it reuses the same resolution and DDL planning the
/// actual pass runs, so the confirmation dialog shows the real rename/create/cleanup
/// statements and the real source→target→backup mapping.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRebuildPreview {
    pub sql: String,
    pub tables: Vec<TransferRebuildPreviewTable>,
    /// The backup (rename) phase on its own, for callers that place the structure preview
    /// between the rename and the cleanup phases instead of showing the combined `sql`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_sql: Option<String>,
    /// The cleanup (drop backups) phase on its own. `sql` stays the combined plan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_sql: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRebuildPreviewTable {
    pub source_table: String,
    pub target_table: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_table: Option<String>,
}

/// SQL plan preview for a structure-only transfer.
///
/// Plans rather than executes: every statement comes from the generator the execution pass
/// uses for the same operation, so preview and execution can never disagree on the DDL they
/// render. It is still a preview — the target is re-inspected when the transfer starts.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferStructurePreview {
    pub sql: String,
    pub tables: Vec<TransferStructurePreviewTable>,
    pub operations: Vec<TransferStructureOperation>,
}

/// One user-facing logical operation produced by the structure transfer planner.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TransferStructureOperationKind {
    CreateSchema,
    CreateTable,
    SkipExistingTable,
    RebuildTable,
    CreateIndex,
    AddForeignKey,
    CreateSequence,
    BindSequence,
    AddComment,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferStructureOperation {
    pub kind: TransferStructureOperationKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_table: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_table: Option<String>,
}

impl TransferStructureOperation {
    pub(crate) fn table(kind: TransferStructureOperationKind, source_table: &str, target_table: &str) -> Self {
        Self {
            kind,
            object_name: None,
            source_table: Some(source_table.to_string()),
            target_table: Some(target_table.to_string()),
        }
    }

    pub(crate) fn object(
        kind: TransferStructureOperationKind,
        object_name: &str,
        source_table: &str,
        target_table: &str,
    ) -> Self {
        Self {
            kind,
            object_name: Some(object_name.to_string()),
            source_table: Some(source_table.to_string()),
            target_table: Some(target_table.to_string()),
        }
    }

    pub(crate) fn schema(schema: &str) -> Self {
        Self {
            kind: TransferStructureOperationKind::CreateSchema,
            object_name: Some(schema.to_string()),
            source_table: None,
            target_table: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferStructurePreviewTable {
    pub source_table: String,
    pub target_table: String,
    /// The target table already exists and this transfer plans no structure DDL for it,
    /// mirroring the execution pass' skip when the target table is present.
    pub preexisting: bool,
    /// The structure the transfer plans for this table, or an explanatory comment when the
    /// execution pass will skip the table entirely.
    pub sql: String,
}

#[derive(Debug, Clone, Copy)]
enum TransferObjectSelectionMode<'a> {
    LegacyUnspecified,
    Explicit(&'a [TransferObjectSelection]),
}

impl<'a> TransferObjectSelectionMode<'a> {
    fn selections(self) -> &'a [TransferObjectSelection] {
        match self {
            Self::LegacyUnspecified => &[],
            Self::Explicit(selections) => selections,
        }
    }

    fn filter_supported(self, supported: &[TransferObjectKind]) -> Option<Vec<TransferObjectSelection>> {
        match self {
            Self::LegacyUnspecified => None,
            Self::Explicit(selections) => Some(
                selections.iter().filter(|selection| supported.contains(&selection.object_type)).cloned().collect(),
            ),
        }
    }
}

impl TransferRequest {
    fn object_selection_mode(&self) -> TransferObjectSelectionMode<'_> {
        match self.objects.as_deref() {
            Some(selections) => TransferObjectSelectionMode::Explicit(selections),
            None => TransferObjectSelectionMode::LegacyUnspecified,
        }
    }

    pub fn target_table_name(&self, source_table: &str) -> String {
        match self.target_table_name_case {
            TransferTableNameCase::Preserve => source_table.to_string(),
            TransferTableNameCase::Lower => source_table.to_lowercase(),
            TransferTableNameCase::Upper => source_table.to_uppercase(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub transfer_id: String,
    pub table: String,
    pub table_index: usize,
    pub total_tables: usize,
    pub rows_transferred: u64,
    pub total_rows: Option<u64>,
    pub status: TransferStatus,
    pub error: Option<String>,
    pub terminal: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferStatus {
    Running,
    TableDone,
    Done,
    Error,
    Cancelled,
}

pub fn quote_identifier(name: &str, db_type: &DatabaseType) -> String {
    quote_transfer_identifier(name, db_type)
}

fn quote_identifier_with_identifier_quote(
    name: &str,
    db_type: &DatabaseType,
    identifier_quote: Option<&str>,
) -> String {
    crate::sql_dialect::quote_table_data_identifier(Some(*db_type), name, identifier_quote)
}

/// Resolve an optional catalog for external-catalog routing in the transfer
/// pipeline.  Returns `Some(catalog)` only when:
///   - the catalog is non-empty and not a built-in catalog name, AND
///   - the database type is Doris/StarRocks, or MySQL (StarRocks/Doris are often
///     saved as `db_type=mysql` with a matching `driver_profile`; the transfer UI
///     only sends `sourceCatalog`/`targetCatalog` for catalog-capable connections).
///
/// Built-in catalogs: Doris `internal`, StarRocks `default_catalog`. Prefer
/// [`resolve_external_transfer_catalog_for_config`] when a full connection
/// config is available (also matches `driver_profile=starrocks|doris`).
pub fn resolve_external_transfer_catalog<'a>(catalog: Option<&'a str>, db_type: &DatabaseType) -> Option<&'a str> {
    None
}

/// Like [`resolve_external_transfer_catalog`], but also treats MySQL connections
/// with a Doris/StarRocks `driver_profile` as catalog-capable.
pub fn resolve_external_transfer_catalog_for_config<'a>(
    catalog: Option<&'a str>,
    config: &crate::models::connection::ConnectionConfig,
) -> Option<&'a str> {
    None
}

fn normalize_external_catalog_name(catalog: Option<&str>) -> Option<&str> {
    let catalog = catalog?.trim();
    if catalog.is_empty() || catalog.eq_ignore_ascii_case("internal") || catalog.eq_ignore_ascii_case("default_catalog")
    {
        return None;
    }
    Some(catalog)
}

/// Create (or reuse) a transfer pool for the given connection/database/catalog.
///
/// For Doris/StarRocks external catalogs the pool is created with
/// `catalog=<name>` in the connection URL params so mysql_async runs
/// `SET catalog` **before** `USE <database>` during setup. The handshake does
/// not send the external database name (mysql_async strips the path when a
/// catalog is present), which is what previously caused `Unknown database`.
pub async fn ensure_transfer_pool(
    state: &AppState,
    connection_id: &str,
    database: &str,
    catalog: Option<&str>,
) -> Result<String, String> {
    let config = {
        let configs = state.configs.read().await;
        configs.get(connection_id).cloned().ok_or_else(|| format!("Connection config not found: {connection_id}"))?
    };
    {
        state.get_or_create_pool(connection_id, Some(database)).await
    }
}

pub fn qualified_table(table: &str, schema: &str, db_type: &DatabaseType, catalog: Option<&str>) -> String {
    // Only use 3-part catalog-qualified names for Doris/StarRocks external catalogs.
    let effective_catalog = None;
    qualified_transfer_table(table, schema, db_type, effective_catalog)
}

fn qualified_table_with_identifier_quote(
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
    identifier_quote: Option<&str>,
) -> String {
    if crate::sql_dialect::uses_connection_identifier_quote(Some(*db_type), identifier_quote) {
        crate::sql_dialect::table_data_qualified_table_name(
            Some(*db_type),
            (!schema.trim().is_empty()).then_some(schema),
            table,
            identifier_quote,
        )
    } else {
        qualified_table(table, schema, db_type, catalog)
    }
}

pub fn validate_transfer_target_table_names(request: &TransferRequest) -> Result<(), String> {
    let mut targets: HashMap<String, String> = HashMap::new();
    for source_table in &request.tables {
        let target_table = request.target_table_name(source_table);
        if let Some(first_source) = targets.insert(target_table.clone(), source_table.clone()) {
            return Err(format!(
                "Target table name collision after case conversion: '{first_source}' and '{source_table}' both map to '{target_table}'"
            ));
        }
    }
    Ok(())
}

/// Kinds that may be transferred between different database families.
/// Only DDL shapes that can be mechanically rewritten are allowed:
/// views (CREATE ... VIEW ... AS SELECT) and sequences (CREATE SEQUENCE).
/// Sequences additionally require both sides to support the type (MySQL does not).
/// Cross-family transfer is only supported between the MySQL, SQL Server and
/// Oracle/Dameng families; the Postgres family is not a validated source or
/// target for the cross-family DDL pipeline (the executor rejects it).
pub fn cross_family_transferable_object_kinds(source: &DatabaseType, target: &DatabaseType) -> Vec<TransferObjectKind> {
    use TransferObjectKind::*;
    if is_same_transfer_family(source, target) {
        return transfer_object_kinds(source);
    }
    // Narrow the matrix to validated directions: MySQL, SQL Server and
    // Oracle/Dameng may act as either side. Postgres (and anything else) is
    // excluded — the executor rejects Postgres sources and no dialect-aware
    // conversion is validated for it.
    let family_supported =
        |db_type: &DatabaseType| matches!(transfer_object_family(db_type), Some(TransferObjectFamily::Mysql));
    if !family_supported(source) || !family_supported(target) {
        return Vec::new();
    }
    // Cross-family VIEW transfer is disabled: convert_cross_family_object_ddl
    // only rewrites the DDL wrapper, identifier quoting and schema qualifiers —
    // it does not translate the view query body, so source-specific constructs
    // (IFNULL, TOP, GETDATE, …) would execute unchanged on an incompatible
    // target. Only sequences are allowed: their CREATE SEQUENCE statements are
    // plain DDL (no query body) and the small set of dialect differences
    // (AS <type>, NOCYCLE/NOCACHE) is converted and tested.
    let source_kinds = transfer_object_kinds(source);
    let target_kinds = transfer_object_kinds(target);
    let mut allowed = Vec::new();
    if source_kinds.contains(&Sequence) && target_kinds.contains(&Sequence) {
        allowed.push(Sequence);
    }
    allowed
}

/// Rewrites a source DDL fragment to the target family's quoting style and
/// schema qualifier. Prefix normalization (DEFINER/ALGORITHM/FORCE/WITH
/// SCHEMABINDING/AS <type>) is applied per source family.
pub fn convert_cross_family_object_ddl(
    source_family: &TransferObjectFamily,
    target_family: &TransferObjectFamily,
    kind: &TransferObjectKind,
    source_schema: &str,
    target_schema: &str,
    ddl: &str,
) -> String {
    let mut sql = ddl.trim().to_string();
    match kind {
        TransferObjectKind::View => match source_family {
            TransferObjectFamily::Mysql => {
                sql = strip_mysql_definer(&sql);
                sql = strip_sql_view_prefix(&sql);
            }

            _ => {}
        },
        TransferObjectKind::Sequence => match source_family {
            _ => {}
        },
        _ => {}
    }
    // Mask string literals and comments before any identifier/schema
    // rewrite, then restore them afterwards: regexes that re-quote
    // identifiers must never touch text inside strings or comments. MySQL
    // double-quoted text is a string literal (unless ANSI_QUOTES is on).
    let (masked, restores) = protect_sql_literals(&sql, matches!(source_family, TransferObjectFamily::Mysql));
    sql = rewrite_identifiers_to_target(&masked, target_family);
    if !source_schema.is_empty() && !target_schema.is_empty() && !source_schema.eq_ignore_ascii_case(target_schema) {
        sql = rewrite_cross_family_schema_qualifier(&sql, target_family, source_schema, target_schema);
    }
    if kind == &TransferObjectKind::View {
        sql = qualify_cross_family_view_target(&sql, target_family, target_schema);
    }
    for (placeholder, original) in restores {
        sql = sql.replace(&placeholder, &original);
    }
    sql
}

/// Locates string literals and comments in SQL: single-quoted strings
/// (with `''` and backslash escapes), MySQL double-quoted strings when
/// `double_quote_is_string` is set, `--`/`#` line comments and `/* */`
/// block comments. Returns byte ranges `(start, end)` of those spans.
fn sql_non_code_spans_with_mysql_identifiers(
    sql: &str,
    double_quote_is_string: bool,
    backtick_is_identifier: bool,
) -> Vec<(usize, usize)> {
    let bytes = sql.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let starts_non_code = match b {
            b'\'' => true,
            b'"' if double_quote_is_string => true,
            b'`' if backtick_is_identifier => true,
            b'-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => true,
            b'#' => true, // MySQL line comment
            b'/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => true,
            _ => false,
        };
        if !starts_non_code {
            i += 1;
            continue;
        }
        let start = i;
        i = match b {
            b'\'' | b'"' | b'`' => {
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        i += 2; // backslash escape (MySQL style)
                        continue;
                    }
                    if bytes[i] == b && i + 1 < bytes.len() && bytes[i + 1] == b {
                        i += 2; // '' / "" doubled quote
                        continue;
                    }
                    if bytes[i] == b {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                i
            }
            b'-' | b'#' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                i
            }
            _ => {
                i += 2; // /* ... */
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                (i + 2).min(bytes.len())
            }
        };
        spans.push((start, i));
    }
    spans
}

fn sql_non_code_spans(sql: &str, double_quote_is_string: bool) -> Vec<(usize, usize)> {
    sql_non_code_spans_with_mysql_identifiers(sql, double_quote_is_string, false)
}

/// Applies `f` to every code span of `sql`; string literals and comments
/// (see `sql_non_code_spans`) pass through verbatim so rewrites never touch
/// text inside them.
fn map_sql_code_spans<F>(sql: &str, double_quote_is_string: bool, mut f: F) -> String
where
    F: FnMut(&str) -> String,
{
    let spans = sql_non_code_spans(sql, double_quote_is_string);
    let mut out = String::with_capacity(sql.len());
    let mut prev = 0;
    for (start, end) in spans {
        if start > prev {
            out.push_str(&f(&sql[prev..start]));
        }
        out.push_str(&sql[start..end]);
        prev = end;
    }
    if prev < sql.len() {
        out.push_str(&f(&sql[prev..]));
    }
    out
}

fn map_mysql_ddl_code_spans<F>(sql: &str, mut f: F) -> String
where
    F: FnMut(&str) -> String,
{
    let spans = sql_non_code_spans_with_mysql_identifiers(sql, true, true);
    let mut out = String::with_capacity(sql.len());
    let mut prev = 0;
    for (start, end) in spans {
        if start > prev {
            out.push_str(&f(&sql[prev..start]));
        }
        out.push_str(&sql[start..end]);
        prev = end;
    }
    if prev < sql.len() {
        out.push_str(&f(&sql[prev..]));
    }
    out
}

/// Masks string literals and comments with placeholders so subsequent regex
/// rewrites cannot touch them, and returns the restore map to swap the
/// original text back afterwards.
fn protect_sql_literals(sql: &str, double_quote_is_string: bool) -> (String, Vec<(String, String)>) {
    let spans = sql_non_code_spans(sql, double_quote_is_string);
    let mut out = String::with_capacity(sql.len());
    let mut restores = Vec::new();
    let mut prev = 0;
    for (start, end) in spans {
        if start > prev {
            out.push_str(&sql[prev..start]);
        }
        let placeholder = format!("__DBX_LIT_{}__", restores.len());
        out.push_str(&placeholder);
        restores.push((placeholder, sql[start..end].to_string()));
        prev = end;
    }
    if prev < sql.len() {
        out.push_str(&sql[prev..]);
    }
    (out, restores)
}

/// Aligns a cross-family view DDL with the target schema: the `CREATE VIEW`
/// target and bare table references in the body (`FROM`/`JOIN`/`INTO`/`UPDATE`)
/// are qualified with the target schema, matching how table transfer creates
/// tables (`"schema"."table"`). References that already carry a prefix are left
/// untouched. String literals and comments are preserved verbatim (see
/// `map_sql_code_spans`).
fn qualify_cross_family_view_target(sql: &str, target_family: &TransferObjectFamily, target_schema: &str) -> String {
    if target_schema.is_empty() {
        return sql.to_string();
    }
    let (open, close) = match target_family {
        TransferObjectFamily::Mysql => ("`", "`"),

        _ => return sql.to_string(),
    };
    let qo = regex::escape(open);
    let qc = regex::escape(close);
    // Identifiers may contain any characters (including CJK) except the
    // closing quote of the target dialect, since every identifier has been
    // normalized to the target quoting style by this point.
    let ident = match target_family {
        TransferObjectFamily::Mysql => r#"[^`]+"#,

        _ => "[A-Za-z_][A-Za-z0-9_]*",
    };
    let create_re = Regex::new(&format!(r"(?i)\bCREATE\s+VIEW\s+(?:{qo}({ident}){qc}\.)?{qo}({ident}){qc}")).unwrap();
    let mut out = create_re
        .replace_all(sql, |caps: &regex::Captures| {
            let name = &caps[2];
            if matches!(caps.get(1), Some(m) if !m.as_str().is_empty()) {
                caps[0].to_string()
            } else {
                format!("CREATE VIEW {open}{target_schema}{close}.{open}{name}{close}")
            }
        })
        .to_string();
    // bare references after FROM/JOIN/INTO/UPDATE/TABLE get the schema prefix;
    // already-prefixed references (group 1) stay as-is
    let ref_re =
        Regex::new(&format!(r"(?i)\b(from|join|into|update|table)\s+(?:{qo}({ident}){qc}\.)?{qo}({ident}){qc}"))
            .unwrap();
    out = ref_re
        .replace_all(&out, |caps: &regex::Captures| {
            let name = &caps[3];
            if matches!(caps.get(2), Some(m) if !m.as_str().is_empty()) {
                caps[0].to_string()
            } else {
                format!("{} {open}{target_schema}{close}.{open}{name}{close}", &caps[1])
            }
        })
        .to_string();
    out
}

/// Collapses `CREATE [ALGORITHM=..] [DEFINER=..] [SQL SECURITY ..] VIEW`,
/// `CREATE OR REPLACE [FORCE] [NONEDITIONABLE] VIEW` and plain `CREATE VIEW`
/// into a bare `CREATE VIEW` prefix usable on every target family.
fn strip_sql_view_prefix(sql: &str) -> String {
    if let Some(pos) = sql.find(" VIEW ") {
        let head = &sql[..pos];
        if head.trim_start().starts_with("CREATE") {
            return format!("CREATE VIEW{}", &sql[pos + 5..]);
        }
    }
    sql.to_string()
}

/// Rewrites backtick / double-quote / bracket identifier quoting to the
/// target family's style. Only identifiers in code positions are rewritten:
/// string literals and comments are preserved verbatim (see
/// `map_sql_code_spans`). The source family decides whether double-quoted
/// text is a string literal (MySQL, unless ANSI_QUOTES is on) or an
/// identifier (SQL Server / Oracle).
fn rewrite_identifiers_to_target(sql: &str, target: &TransferObjectFamily) -> String {
    let (open, close, pattern) = match target {
        TransferObjectFamily::Mysql => ("`", "`", r#""([^"]+)"|\[([^\]]+)\]"#),

        _ => return sql.to_string(),
    };
    let re = Regex::new(pattern).unwrap();
    re.replace_all(sql, |caps: &regex::Captures| {
        let name = caps.get(1).map(|m| m.as_str()).or_else(|| caps.get(2).map(|m| m.as_str())).unwrap_or("");
        format!("{open}{name}{close}")
    })
    .to_string()
}

/// Rewrites `{quote}{source_schema}{quote}.` qualifiers to the target schema
/// in the target family's quoting style.
fn rewrite_cross_family_schema_qualifier(
    sql: &str,
    target: &TransferObjectFamily,
    source_schema: &str,
    target_schema: &str,
) -> String {
    let (open, close) = match target {
        TransferObjectFamily::Mysql => ("`", "`"),

        _ => return sql.to_string(),
    };
    let source = format!("{open}{}{close}.", regex::escape(source_schema));
    let target = format!("{open}{target_schema}{close}.");
    sql.replace(&source, &target)
}

pub fn validate_transfer_request(request: &TransferRequest) -> Result<(), String> {
    validate_transfer_target_table_names(request)?;
    let selection_mode = request.object_selection_mode();
    if matches!(request.content, TransferContent::DataOnly) && !selection_mode.selections().is_empty() {
        return Err("仅数据模式不传输非表对象".to_string());
    }
    if request.drop_target_before_create
        && (matches!(request.content, TransferContent::DataOnly) || !request.create_table)
    {
        return Err("drop_target_before_create requires structure transfer (create_table and content != DataOnly). \
             Data-only mode does not create tables, so a dropped target would not be rebuilt."
            .to_string());
    }
    for selection in selection_mode.selections() {
        if selection.names.is_empty() {
            return Err(format!("Object selection for {:?} is empty", selection.object_type));
        }
        for name in &selection.names {
            if name.trim().is_empty() || name.contains('\0') {
                return Err(format!("Invalid object name: {name:?}"));
            }
        }
    }
    for (table, raw) in &request.table_filters {
        if table.trim().is_empty() {
            return Err("Table filter contains an empty table name".to_string());
        }
        parse_transfer_table_filter(raw)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedTransferTargetTable {
    name: String,
    preexisting: bool,
}

fn json_scalar_to_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn mysql_lower_case_table_names_from_result(result: &db::QueryResult) -> Option<u8> {
    let row = result.rows.first()?;
    row.get(1).or_else(|| row.first()).and_then(json_scalar_to_string)?.trim().parse::<u8>().ok()
}

async fn target_table_lookup_is_case_insensitive(
    state: &AppState,
    target_pool_key: &str,
    target_db_type: &DatabaseType,
) -> bool {
    {}

    let result = match execute_on_pool(state, target_pool_key, "SHOW VARIABLES LIKE 'lower_case_table_names'").await {
        Ok(result) => result,
        Err(error) => {
            log::debug!("[transfer] failed to read MySQL lower_case_table_names: {error}");
            return false;
        }
    };

    // MySQL lower_case_table_names=1/2 means table lookup is case-insensitive.
    // Prefer the metadata name so generated INSERT/TRUNCATE SQL keeps the target
    // table's declared case instead of the source-derived request case.
    mysql_lower_case_table_names_from_result(&result).is_some_and(|value| value != 0)
}

fn existing_transfer_target_table_name(
    requested_name: &str,
    tables: &[db::TableInfo],
    allow_case_insensitive_match: bool,
) -> Option<String> {
    if let Some(table) = tables.iter().find(|table| table.name == requested_name) {
        return Some(table.name.clone());
    }
    if !allow_case_insensitive_match {
        return None;
    }
    tables.iter().find(|table| table.name.eq_ignore_ascii_case(requested_name)).map(|table| table.name.clone())
}

const TRANSFER_PROGRESS_CHANNEL_CAPACITY: usize = 16;

fn try_send_transfer_progress(sender: &tokio::sync::mpsc::Sender<TransferProgress>, progress: TransferProgress) {
    let _ = sender.try_send(progress);
}

struct AbortTransferTaskOnDrop(tokio::task::AbortHandle);

impl Drop for AbortTransferTaskOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[allow(clippy::too_many_arguments)]
async fn list_transfer_tables_isolated(
    state: Arc<AppState>,
    connection_id: String,
    database: String,
    schema: String,
    catalog: Option<String>,
    db_type: DatabaseType,
    table: String,
    limit: usize,
) -> Result<Vec<db::TableInfo>, String> {
    let task = tokio::spawn(async move {
        {
            crate::schema::list_tables_core(
                &state,
                &connection_id,
                &database,
                &schema,
                Some(&table),
                Some(limit),
                None,
                None,
                None,
            )
            .await
        }
    });
    let _abort_on_drop = AbortTransferTaskOnDrop(task.abort_handle());
    task.await.map_err(|error| format!("Transfer table metadata task failed: {error}"))?
}

async fn resolve_transfer_target_table_name(
    state: &Arc<AppState>,
    request: &TransferRequest,
    source_table: &str,
    target_pool_key: &str,
    target_db_type: &DatabaseType,
    _source_catalog: Option<&str>,
    target_catalog: Option<&str>,
) -> ResolvedTransferTargetTable {
    let requested_name = request.target_table_name(source_table);
    {}

    let allow_case_insensitive_match =
        target_table_lookup_is_case_insensitive(state, target_pool_key, target_db_type).await;

    // Route through the catalog-aware path when targeting an external
    // Doris/StarRocks catalog — otherwise the lookup runs against the
    // default / internal catalog and can miss or misidentify the table.
    let tables = list_transfer_tables_isolated(
        state.clone(),
        request.target_connection_id.clone(),
        request.target_database.clone(),
        request.target_schema.clone(),
        target_catalog.map(str::to_string),
        *target_db_type,
        requested_name.clone(),
        TRANSFER_TARGET_TABLE_LOOKUP_LIMIT,
    )
    .await
    .unwrap_or_else(|error| {
        log::debug!("[transfer] failed to resolve target table metadata for {requested_name}: {error}");
        Vec::new()
    });

    if let Some(existing_name) =
        existing_transfer_target_table_name(&requested_name, &tables, allow_case_insensitive_match)
    {
        ResolvedTransferTargetTable { name: existing_name, preexisting: true }
    } else {
        ResolvedTransferTargetTable { name: requested_name, preexisting: false }
    }
}

/// Returns a SQL statement selecting 1 row when `name` exists in `schema`
/// on the target side; None-kind support per family mirrors
/// `transfer_object_kinds`.
pub fn target_object_exists_sql(
    db_type: &DatabaseType,
    schema: &str,
    name: &str,
    kind: &TransferObjectKind,
) -> Result<String, String> {
    let schema = quote_string_literal(schema);
    let name = quote_string_literal(name);
    let q = |literal: &str| literal.to_string();
    let sql = match (transfer_object_family(db_type), kind) {
        (Some(TransferObjectFamily::Mysql), TransferObjectKind::Table | TransferObjectKind::View) => format!(
            "SELECT 1 FROM information_schema.TABLES WHERE TABLE_SCHEMA = {schema} AND TABLE_NAME = {name} \
             AND TABLE_TYPE {} 'VIEW'",
            if matches!(kind, TransferObjectKind::View) { "=" } else { "<>" }
        ),
        (Some(TransferObjectFamily::Mysql), TransferObjectKind::Procedure | TransferObjectKind::Function) => format!(
            "SELECT 1 FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = {schema} AND ROUTINE_NAME = {name} \
             AND ROUTINE_TYPE = {}",
            q(if matches!(kind, TransferObjectKind::Procedure) { "'PROCEDURE'" } else { "'FUNCTION'" })
        ),
        (Some(TransferObjectFamily::Mysql), TransferObjectKind::Trigger) => format!(
            "SELECT 1 FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = {schema} AND TRIGGER_NAME = {name}"
        ),
        (Some(TransferObjectFamily::Mysql), TransferObjectKind::Event) => {
            format!("SELECT 1 FROM information_schema.EVENTS WHERE EVENT_SCHEMA = {schema} AND EVENT_NAME = {name}")
        }

        _ => return Err(format!("Object existence check not supported for {:?} {:?}", db_type, kind)),
    };
    Ok(sql)
}

/// Remove `DEFINER=`user`@`host`` tokens from MySQL DDL (they are not
/// transferable and frequently reference accounts that don't exist on target).
pub fn strip_mysql_definer(ddl: &str) -> String {
    let re = Regex::new(r"(?i)\bDEFINER\s*=\s*`[^`]*`@`[^`]*`\s*").unwrap();
    re.replace_all(ddl, "").to_string()
}

/// Rewrite backtick-qualified `schema`.`name` references from `source_schema`
/// to `target_schema` in MySQL DDL.
pub fn rewrite_mysql_schema_qualifier(ddl: &str, source_schema: &str, target_schema: &str) -> String {
    if source_schema == target_schema || source_schema.is_empty() {
        return ddl.to_string();
    }
    let re = Regex::new(&format!(r"`{}`\.", regex::escape(source_schema))).unwrap();
    re.replace_all(ddl, &format!("`{}`.", target_schema)).to_string()
}

pub fn mysql_trigger_ddl(
    schema: &str,
    name: &str,
    timing: &str,
    manipulation: &str,
    table: &str,
    statement: &str,
) -> String {
    format!(
        "CREATE TRIGGER `{name}` {timing} {manipulation} ON `{schema}`.`{table}` FOR EACH ROW {statement}",
        name = name,
        timing = timing,
        manipulation = manipulation,
        schema = schema,
        table = table,
        statement = statement.trim()
    )
}

pub fn mysql_event_ddl(_schema: &str, name: &str, status: &str, schedule: &str, body: &str) -> String {
    format!(
        "CREATE EVENT `{name}` ON SCHEDULE {schedule} {status} DO {body}",
        name = name,
        schedule = schedule,
        status = status,
        body = body.trim()
    )
}

/// Builds the query that fetches DDL for one MySQL object.
/// - View/Procedure/Function → `SHOW CREATE ...`
/// - Trigger → information_schema.TRIGGERS row (timing/manipulation/table/
///   statement) via `mysql_trigger_ddl`.
/// - Event → information_schema.EVENTS row via `mysql_event_ddl`.
pub fn mysql_object_source_query(kind: &TransferObjectKind, database: &str, name: &str) -> Result<String, String> {
    let db = quote_string_literal(database);
    let n = quote_string_literal(name);
    let ddl = match kind {
        TransferObjectKind::View => format!("SHOW CREATE VIEW `{database}`.`{name}`"),
        TransferObjectKind::Procedure => format!("SHOW CREATE PROCEDURE `{database}`.`{name}`"),
        TransferObjectKind::Function => format!("SHOW CREATE FUNCTION `{database}`.`{name}`"),
        TransferObjectKind::Trigger => format!(
            "SELECT TRIGGER_NAME, ACTION_TIMING, EVENT_MANIPULATION, EVENT_OBJECT_TABLE, ACTION_STATEMENT \
             FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = {db} AND TRIGGER_NAME = {n}"
        ),
        TransferObjectKind::Event => format!(
            "SELECT EVENT_NAME, STATUS, EXECUTE_AT, INTERVAL_VALUE, INTERVAL_FIELD, EVENT_DEFINITION \
             FROM information_schema.EVENTS WHERE EVENT_SCHEMA = {db} AND EVENT_NAME = {n}"
        ),
        _ => return Err(format!("MySQL object source not supported for {:?}", kind)),
    };
    Ok(ddl)
}

/// Maps a MySQL DDL query result row to a single DDL string.
/// SHOW CREATE column index: view=1, routine=2 (same convention as
/// `schema::mysql_object_source_ddl_column_index`); triggers and events are
/// assembled from information_schema cells.
pub fn mysql_object_ddl_from_result(
    kind: &TransferObjectKind,
    database: &str,
    rows: &[Vec<serde_json::Value>],
) -> Result<String, String> {
    let row = rows.first().ok_or_else(|| format!("No rows returned for MySQL {:?} DDL", kind))?;
    let cell = |idx: usize| -> Result<&str, String> {
        row.get(idx)
            .and_then(|value| value.as_str())
            .ok_or_else(|| format!("Missing column {idx} in MySQL {:?} DDL result", kind))
    };
    match kind {
        TransferObjectKind::View => Ok(cell(1)?.to_string()),
        TransferObjectKind::Procedure | TransferObjectKind::Function => Ok(cell(2)?.to_string()),
        TransferObjectKind::Trigger => {
            let name = cell(0)?;
            let timing = cell(1)?;
            let manipulation = cell(2)?;
            let table = cell(3)?;
            let statement = cell(4)?;
            Ok(mysql_trigger_ddl(database, name, timing, manipulation, table, statement))
        }
        TransferObjectKind::Event => {
            let name = cell(0)?;
            let status = cell(1)?;
            let execute_at = cell(2)?;
            let interval_value = cell(3)?;
            let interval_field = cell(4)?;
            let body = cell(5)?;
            let schedule = if interval_value.is_empty() && interval_field.is_empty() {
                format!("AT {execute_at}")
            } else {
                format!("EVERY {interval_value} {interval_field}")
            };
            Ok(mysql_event_ddl(database, name, status, &schedule, body))
        }
        _ => Err(format!("MySQL object DDL extraction not supported for {:?}", kind)),
    }
}

pub(crate) fn normalize_integer_literal(
    value: &str,
    db_type: &DatabaseType,
    column_type: Option<&str>,
) -> Option<String> {
    None
}

/// Strips validated en-US thousands separators from a numeric literal for numeric target
/// columns. Only standard 3-digit grouping is accepted ("1,234", "12,345,678"); malformed
/// grouping ("1,23,4", "1,,234") or any non-numeric character returns None so the original
/// text reaches the database and keeps its existing validation error instead of being
/// silently coerced. Values without a comma are left untouched.
pub(crate) fn normalize_thousands_numeric_literal(
    value: &str,
    db_type: &DatabaseType,
    column_type: Option<&str>,
) -> Option<String> {
    {
        return None;
    }
}

pub(crate) fn is_identity_column_extra(extra: Option<&str>) -> bool {
    extra.is_some_and(|value| {
        let normalized = value.trim().to_ascii_lowercase();
        normalized.contains("identity") || normalized.contains("auto_increment") || normalized.contains("autoincrement")
    })
}

pub(crate) fn is_mysql_generated_column_extra(extra: Option<&str>) -> bool {
    extra.is_some_and(|value| {
        let mut parts = value.split_whitespace();
        let Some(first) = parts.next() else {
            return false;
        };
        if first.eq_ignore_ascii_case("generated") {
            return true;
        }
        matches!(first.to_ascii_lowercase().as_str(), "virtual" | "stored" | "persistent")
            && parts.next().is_some_and(|part| part.eq_ignore_ascii_case("generated"))
    })
}

#[cfg(test)]
fn selected_columns_include_identity_extras(columns: &[String], column_extras: &[Option<String>]) -> bool {
    columns
        .iter()
        .enumerate()
        .any(|(index, _)| is_identity_column_extra(column_extras.get(index).and_then(|extra| extra.as_deref())))
}

fn selected_columns_include_identity_columns(columns: &[String], all_columns: &[db::ColumnInfo]) -> bool {
    all_columns.iter().any(|column| {
        is_identity_column_extra(column.extra.as_deref())
            && columns.iter().any(|name| name.eq_ignore_ascii_case(&column.name))
    })
}

fn is_mysql_non_insertable_transfer_column(column: &db::ColumnInfo, source_db_type: &DatabaseType) -> bool {
    true && is_mysql_generated_column_extra(column.extra.as_deref())
}

fn writable_transfer_columns(
    columns: &[db::ColumnInfo],
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
) -> Vec<db::ColumnInfo> {
    columns
        .iter()
        .filter(|column| !false && !is_mysql_non_insertable_transfer_column(column, source_db_type))
        .cloned()
        .collect()
}

fn mysql_generated_only_transfer(
    columns: &[db::ColumnInfo],
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
) -> bool {
    matches!((source_db_type, target_db_type), (DatabaseType::Mysql, DatabaseType::Mysql))
        && !columns.is_empty()
        && columns.iter().all(|column| is_mysql_generated_column_extra(column.extra.as_deref()))
}

fn transfer_column_names_match(
    target_db_type: &DatabaseType,
    quote_target_column_names: bool,
    left: &str,
    right: &str,
) -> bool {
    {
        // Unquoted GaussDB/openGauss identifiers fold to lowercase server-side,
        // so with quoting disabled a table created from mixed-case source
        // columns reports folded names back from the catalog.
        left.eq_ignore_ascii_case(right)
    }
}

/// Maps the source column names written in INSERT/COPY SQL onto the target
/// table's declared column names.
///
/// Write SQL quotes column names, which makes the identifier case-sensitive on
/// targets that fold unquoted identifiers (Oracle and OceanBase Oracle fold to
/// uppercase, PostgreSQL to lowercase). A target table that already exists —
/// typically created outside DBX with unquoted DDL — therefore rejects the
/// source-cased name (`ORA-00904: invalid identifier`, #9320) even though the
/// column exists. Reusing the catalog's declared name keeps the statement on a
/// column that really exists; an exact match still wins so case-sensitive
/// targets that do have the source-cased column keep addressing it.
fn resolve_transfer_target_column_names(col_names: &[String], target_columns: &[db::ColumnInfo]) -> Vec<String> {
    col_names
        .iter()
        .map(|name| {
            target_columns
                .iter()
                .find(|column| column.name == *name)
                .or_else(|| target_columns.iter().find(|column| column.name.eq_ignore_ascii_case(name)))
                .map(|column| column.name.clone())
                .unwrap_or_else(|| name.clone())
        })
        .collect()
}

fn missing_transfer_target_columns(
    target_columns: &[db::ColumnInfo],
    col_names: &[String],
    target_db_type: &DatabaseType,
    quote_target_column_names: bool,
) -> Vec<String> {
    col_names
        .iter()
        .filter(|name| {
            !target_columns.iter().any(|column| {
                transfer_column_names_match(target_db_type, quote_target_column_names, name, &column.name)
            })
        })
        .cloned()
        .collect()
}

fn target_column_can_be_omitted(column: &db::ColumnInfo, target_db_type: &DatabaseType) -> bool {
    let extra = column.extra.as_deref().unwrap_or_default().trim().to_ascii_lowercase();
    column.is_nullable
        || column.column_default.as_deref().is_some_and(|value| !value.trim().is_empty())
        || extra.contains("generated")
        || extra.contains("identity")
        || extra.contains("auto_increment")
        || extra.contains("autoincrement")
        || extra.contains("computed")
        || (false)
}

fn required_unmapped_transfer_target_columns(
    target_columns: &[db::ColumnInfo],
    col_names: &[String],
    target_db_type: &DatabaseType,
    quote_target_column_names: bool,
) -> Vec<String> {
    target_columns
        .iter()
        .filter(|column| {
            !target_column_can_be_omitted(column, target_db_type)
                && !col_names.iter().any(|name| {
                    transfer_column_names_match(target_db_type, quote_target_column_names, name, &column.name)
                })
        })
        .map(|column| column.name.clone())
        .collect()
}

/// Fails fast when a preexisting target table's structure can't accept the
/// source columns. DBX never alters an existing target table's columns, so
/// skipping this check (structure-only transfers used to silently skip it,
/// #7660) leaves the transfer reporting success while the target quietly stays
/// out of sync with the source.
fn validate_preexisting_target_columns(
    target_columns: &[db::ColumnInfo],
    col_names: &[String],
    target_db_type: &DatabaseType,
    quote_target_column_names: bool,
    target_table: &str,
) -> Result<(), String> {
    let missing = missing_transfer_target_columns(target_columns, col_names, target_db_type, quote_target_column_names);
    if !missing.is_empty() {
        return Err(format!(
            "Target table '{target_table}' already exists with a different structure and is missing column(s) \
             {} present in the source table. DBX does not alter an existing target table's columns during \
             transfer — drop the target table or adjust its structure to match the source first.",
            missing.join(", ")
        ));
    }

    let required =
        required_unmapped_transfer_target_columns(target_columns, col_names, target_db_type, quote_target_column_names);
    if !required.is_empty() {
        return Err(format!(
            "Target table '{target_table}' already exists with a different structure and has required column(s) \
             {} that are not present in the source table and have no default or generated value. DBX does not \
             alter an existing target table's columns during transfer — drop the target table or adjust its \
             structure to match the source first.",
            required.join(", ")
        ));
    }
    Ok(())
}

fn transfer_key_columns(columns: &[db::ColumnInfo], db_type: &DatabaseType) -> Vec<String> {
    let uses_unique_key_model = false;
    columns.iter().filter(|column| column.is_primary_key || (false)).map(|column| column.name.clone()).collect()
}

async fn execute_transfer_write_statement(
    state: &AppState,
    target_pool_key: &str,
    sql: &str,
    target_db_type: &DatabaseType,
    table: &str,
    schema: &str,
    needs_identity_insert: bool,
) -> Result<(), String> {
    {
        execute_on_pool(state, target_pool_key, sql).await?;
        return Ok(());
    }
}

fn is_mysql_family_target(target_db: &DatabaseType) -> bool {
    matches!(target_db, DatabaseType::Mysql)
}

fn supports_deferred_mysql_foreign_keys(target_db: &DatabaseType) -> bool {
    is_mysql_family_target(target_db) && crate::table_structure_sql::supports_foreign_keys(*target_db)
}

/// Engines whose ordinary string literals keep a backslash literal, so a
/// transferred value must not have its backslashes doubled: `C:\tmp` has to stay
/// `'C:\tmp'` instead of becoming `'C:\\tmp'`. Mirrors the SQL export path
/// (`quote_export_sql_string_for_database`) and the grid's copy-as-SQL path,
/// which both only double backslashes for dialects whose escape table has one.
fn keeps_backslash_literal(target_db: &DatabaseType) -> bool {
    false
}

fn is_mysql_numeric_base_type(data_type: &str) -> bool {
    let normalized = data_type.trim().to_ascii_lowercase();
    let base = normalized.split(['(', ' ']).next().unwrap_or("");
    matches!(
        base,
        "tinyint"
            | "smallint"
            | "mediumint"
            | "int"
            | "integer"
            | "bigint"
            | "decimal"
            | "numeric"
            | "float"
            | "double"
            | "real"
            | "bit"
            | "year"
    )
}

fn is_mysql_function_default(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("NULL") {
        return true;
    }
    let upper = trimmed.to_ascii_uppercase();
    if upper == "CURRENT_TIMESTAMP" || upper.starts_with("CURRENT_TIMESTAMP(") {
        return true;
    }
    if upper == "LOCALTIME" || upper.starts_with("LOCALTIME(") {
        return true;
    }
    if upper == "LOCALTIMESTAMP" || upper.starts_with("LOCALTIMESTAMP(") {
        return true;
    }
    matches!(upper.as_str(), "CURRENT_DATE" | "CURRENT_TIME" | "NOW()" | "UTC_TIMESTAMP()" | "UUID()")
}

fn looks_like_numeric_literal(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    trimmed.parse::<i64>().is_ok()
        || trimmed.parse::<u64>().is_ok()
        || trimmed.parse::<f64>().is_ok_and(|value| value.is_finite())
}

fn format_mysql_default_literal(raw: &str, data_type: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.eq_ignore_ascii_case("NULL") {
        return "NULL".to_string();
    }
    if is_mysql_function_default(trimmed) {
        return trimmed.to_string();
    }
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        return trimmed.to_string();
    }
    if is_mysql_numeric_base_type(data_type) && looks_like_numeric_literal(trimmed) {
        return trimmed.to_string();
    }
    format!("'{}'", trimmed.replace('\'', "''"))
}

fn column_default_clause(
    column: &db::ColumnInfo,
    source_schema: &str,
    target_schema: &str,
    source_db: &DatabaseType,
    target_db: &DatabaseType,
) -> Option<String> {
    {}
    if is_mysql_family_target(target_db) {
        let default_value = column.column_default.as_deref()?.trim();
        if default_value.is_empty() {
            return None;
        }
        return Some(format!("DEFAULT {}", format_mysql_default_literal(default_value, &column.data_type)));
    }
    None
}

#[derive(Debug, Default, PartialEq, Eq)]
struct MysqlExtraClauses {
    auto_increment: bool,
    on_update: Option<String>,
}

fn parse_mysql_extra_clauses(extra: Option<&str>) -> MysqlExtraClauses {
    let mut result = MysqlExtraClauses::default();
    let Some(raw) = extra else {
        return result;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return result;
    }

    let lowered = trimmed.to_ascii_lowercase();
    if lowered.contains("auto_increment") {
        result.auto_increment = true;
    }

    let pattern = Regex::new(r"(?i)\bon\s+update\s+(.+)$").expect("valid mysql on-update regex");
    if let Some(captures) = pattern.captures(trimmed) {
        let raw_expr = captures.get(1).map(|m| m.as_str()).unwrap_or("");
        let cleaned = raw_expr.trim().trim_end_matches([',', ';', ' ']).trim();
        if !cleaned.is_empty() {
            result.on_update = Some(cleaned.to_string());
        }
    }

    result
}

/// Groups foreign keys by constraint name, preserving first-seen order — MySQL
/// and Postgres both report one row per (constraint, column) pair for
/// multi-column foreign keys, so callers need the columns regrouped by
/// constraint before they can emit one `ADD CONSTRAINT` statement per key.
fn group_foreign_keys_by_constraint_name(foreign_keys: &[db::ForeignKeyInfo]) -> Vec<(&str, Vec<&db::ForeignKeyInfo>)> {
    let mut grouped: HashMap<&str, Vec<&db::ForeignKeyInfo>> = HashMap::new();
    let mut order: Vec<&str> = Vec::new();

    for foreign_key in foreign_keys {
        if !grouped.contains_key(foreign_key.name.as_str()) {
            order.push(foreign_key.name.as_str());
        }
        grouped.entry(foreign_key.name.as_str()).or_default().push(foreign_key);
    }

    order.into_iter().filter_map(|name| grouped.remove(name).map(|group| (name, group))).collect()
}

/// Builds deferred `ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY` statements for a
/// MySQL-family target table from structured source foreign key metadata.
///
/// Used instead of inline `CREATE TABLE ... FOREIGN KEY` so table creation order
/// never has to satisfy foreign key dependencies — this is what makes transferring
/// tables with a foreign key cycle (or any dependency the sort couldn't fully
/// resolve) possible at all, mirroring the existing Postgres transfer path.
fn generate_mysql_foreign_key_alter_statements(
    foreign_keys: &[db::ForeignKeyInfo],
    request: &TransferRequest,
    target_table: &str,
    target_db_type: &DatabaseType,
) -> Vec<String> {
    // MySQL has no separate "schema" concept — `database` doubles as the schema,
    // and callers that leave `source_schema` empty (the common case for MySQL
    // transfers) still need something to compare `ForeignKeyInfo.ref_schema`
    // against. Mirrors `mysql_table_metadata_catalog`'s schema-or-database
    // fallback (crates/dbx-core/src/schema/mod.rs), which is private to that module.
    let source_database = if request.source_schema.trim().is_empty() {
        request.source_database.as_str()
    } else {
        request.source_schema.as_str()
    };

    let full_table = quote_identifier(target_table, target_db_type);
    let mut statements = Vec::new();
    for (name, group) in group_foreign_keys_by_constraint_name(foreign_keys) {
        let columns = group
            .iter()
            .map(|foreign_key| quote_identifier(&foreign_key.column, target_db_type))
            .collect::<Vec<_>>()
            .join(", ");
        let ref_columns = group
            .iter()
            .map(|foreign_key| quote_identifier(&foreign_key.ref_column, target_db_type))
            .collect::<Vec<_>>()
            .join(", ");
        let referenced_table = match group[0].ref_schema.as_deref() {
            // Referenced table lives in the same database this transfer is
            // reading from, so it's part of (or expected to be part of) this
            // transfer batch — resolve its target-side name the same way every
            // other transferred table's name is resolved (case rules, etc.).
            Some(ref_schema) if ref_schema == source_database => {
                quote_identifier(&request.target_table_name(&group[0].ref_table), target_db_type)
            }
            // Genuine cross-database foreign key pointing outside the transfer's
            // selected tables: that table was never created or renamed by this
            // transfer, so reference it by its original database/name, assumed
            // to already exist unchanged on the target server.
            Some(ref_schema) => {
                format!(
                    "{}.{}",
                    quote_identifier(ref_schema, target_db_type),
                    quote_identifier(&group[0].ref_table, target_db_type)
                )
            }
            None => quote_identifier(&request.target_table_name(&group[0].ref_table), target_db_type),
        };
        let mut statement = format!(
            "ALTER TABLE {full_table} ADD CONSTRAINT {} FOREIGN KEY ({columns}) REFERENCES {referenced_table} ({ref_columns})",
            quote_identifier(name, target_db_type)
        );
        if let Some(on_delete) = group[0].on_delete.as_deref() {
            statement.push_str(&format!(" ON DELETE {on_delete}"));
        }
        if let Some(on_update) = group[0].on_update.as_deref() {
            statement.push_str(&format!(" ON UPDATE {on_update}"));
        }
        statements.push(statement);
    }

    statements
}

#[derive(Debug, Clone, Default)]
struct PostgresTableDependencySelection {
    extension_names: Vec<String>,
    enum_type_names: Vec<String>,
    domain_names: Vec<String>,
}

impl PostgresTableDependencySelection {
    fn type_names(&self) -> Vec<String> {
        let mut names = self.enum_type_names.iter().chain(&self.domain_names).cloned().collect::<Vec<_>>();
        names.sort();
        names.dedup();
        names
    }
}

fn json_string_cell(row: &[serde_json::Value], index: usize) -> Option<String> {
    row.get(index).and_then(|value| value.as_str().map(str::to_string))
}

fn ensure_sql_statement_terminated(sql: &str) -> String {
    let trimmed = sql.trim();
    if trimmed.ends_with(';') {
        trimmed.to_string()
    } else {
        format!("{trimmed};")
    }
}

pub fn escape_value(val: &serde_json::Value, db_type: &DatabaseType) -> String {
    escape_value_typed(val, db_type, None)
}

pub fn escape_value_typed(val: &serde_json::Value, db_type: &DatabaseType, column_type: Option<&str>) -> String {
    {}
    match val {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(b) => match db_type {
            DatabaseType::Mysql => {
                if *b {
                    if column_type.is_some_and(is_mysql_bit_type) {
                        "b'1'".to_string()
                    } else {
                        "1".to_string()
                    }
                } else if column_type.is_some_and(is_mysql_bit_type) {
                    "b'0'".to_string()
                } else {
                    "0".to_string()
                }
            }

            _ => {
                if *b {
                    "TRUE".to_string()
                } else {
                    "FALSE".to_string()
                }
            }
        },
        serde_json::Value::Number(n) => {
            {}
            if let Some(integer_literal) = normalize_integer_literal(&n.to_string(), db_type, column_type) {
                return integer_literal;
            }
            match db_type {
                DatabaseType::Mysql => {
                    if column_type.is_some_and(is_mysql_bit_type) {
                        format!("b'{}'", n)
                    } else {
                        n.to_string()
                    }
                }
                _ => n.to_string(),
            }
        }
        serde_json::Value::String(s) => {
            {}
            {}
            if let Some(integer_literal) = normalize_integer_literal(s, db_type, column_type) {
                return integer_literal;
            }
            {}
            if let Some(binary_literal) = format_mysql_binary_sql_literal(s, db_type, column_type) {
                return binary_literal;
            }
            {}
            {}
            if let Some(numeric_literal) = format_mysql_numeric_string_literal(s, db_type, column_type) {
                return numeric_literal;
            }
            {}
            {}

            let literal = format_literal_string(s, db_type, column_type);
            {}
            let escaped = if false || keeps_backslash_literal(db_type) {
                literal.replace('\'', "''")
            } else {
                literal.replace('\\', "\\\\").replace('\'', "''")
            };
            match db_type {
                DatabaseType::Mysql if column_type.is_some_and(is_mysql_bit_type) => {
                    format!("b'{escaped}'")
                }

                _ => format!("'{escaped}'"),
            }
        }
        serde_json::Value::Array(arr) => {
            {}
            match db_type {
                _ => format_pg_array_sql_literal(arr),
            }
        }
        _ => {
            let s = val.to_string();
            {}
            {}
            format!("'{}'", s.replace('\\', "\\\\").replace('\'', "''"))
        }
    }
}

fn is_mysql_bit_type(column_type: &str) -> bool {
    let trimmed = column_type.trim();
    let lower = trimmed.to_ascii_lowercase();
    lower == "bit" || lower.starts_with("bit(") || lower.starts_with("bit ")
}

fn is_mysql_numeric_string_literal_database(db_type: &DatabaseType) -> bool {
    matches!(db_type, DatabaseType::Mysql)
}

fn is_mysql_non_bit_numeric_type(column_type: &str) -> bool {
    is_mysql_numeric_base_type(column_type) && !is_mysql_bit_type(column_type)
}

fn format_mysql_numeric_string_literal(
    value: &str,
    db_type: &DatabaseType,
    column_type: Option<&str>,
) -> Option<String> {
    if !is_mysql_numeric_string_literal_database(db_type) || !column_type.is_some_and(is_mysql_non_bit_numeric_type) {
        return None;
    }
    let trimmed = value.trim();
    if looks_like_numeric_literal(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn is_binary_transfer_column_type(column_type: &str) -> bool {
    let lower = column_type.trim().to_ascii_lowercase();
    let base = lower.split(['(', ' ', '\t', '\n']).next().unwrap_or("");
    matches!(base, "binary" | "varbinary" | "blob" | "tinyblob" | "mediumblob" | "longblob" | "bytea" | "image")
}

fn format_mysql_binary_sql_literal(value: &str, db_type: &DatabaseType, column_type: Option<&str>) -> Option<String> {
    if !matches!(db_type, DatabaseType::Mysql) || !column_type.is_some_and(is_binary_transfer_column_type) {
        return None;
    }

    let trimmed = value.trim();
    let hex = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X"))?;
    if hex.as_bytes().iter().all(|byte| byte.is_ascii_hexdigit()) {
        Some(if hex.is_empty() { "X''".to_string() } else { format!("0x{hex}") })
    } else {
        None
    }
}

fn format_literal_string(value: &str, db_type: &DatabaseType, column_type: Option<&str>) -> String {
    if is_mysql_datetime_literal_database(db_type) && column_type.map(is_temporal_column_type).unwrap_or(true) {
        normalize_mysql_temporal_literal(value, column_type).unwrap_or_else(|| value.to_string())
    } else {
        value.to_string()
    }
}

fn is_mysql_datetime_literal_database(db_type: &DatabaseType) -> bool {
    matches!(db_type, DatabaseType::Mysql)
}

fn normalize_mysql_temporal_literal(value: &str, column_type: Option<&str>) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() < 20 || !is_mysql_datetime_base(bytes) {
        return None;
    }

    let rest = &value[19..];
    let (fraction, offset) = if let Some(after_dot) = rest.strip_prefix('.') {
        let digit_count = after_dot.bytes().take_while(|b| b.is_ascii_digit()).count();
        if digit_count == 0 {
            return None;
        }
        let fraction_len = 1 + digit_count;
        (&rest[..fraction_len.min(7)], &rest[fraction_len..])
    } else {
        ("", rest)
    };

    if !is_timezone_suffix(offset) {
        return None;
    }

    match temporal_column_kind(column_type) {
        Some("date") => Some(value[..10].to_string()),
        Some("time") => Some(format!("{}{}", &value[11..19], fraction)),
        _ => Some(format!("{} {}{}", &value[..10], &value[11..19], fraction)),
    }
}

fn is_temporal_column_type(column_type: &str) -> bool {
    temporal_column_kind(Some(column_type)).is_some()
}

fn temporal_column_kind(column_type: Option<&str>) -> Option<&'static str> {
    let base = column_type?.trim().to_ascii_lowercase();
    let base = base.split(['(', ':', ' ']).next().unwrap_or("");
    match base {
        "date" => Some("date"),
        "time" => Some("time"),
        "datetime" | "timestamp" => Some("datetime"),
        _ => None,
    }
}

fn is_mysql_datetime_base(bytes: &[u8]) -> bool {
    matches!(
        bytes,
        [
            y0,
            y1,
            y2,
            y3,
            b'-',
            m0,
            m1,
            b'-',
            d0,
            d1,
            sep,
            h0,
            h1,
            b':',
            min0,
            min1,
            b':',
            s0,
            s1,
            ..
        ] if y0.is_ascii_digit()
            && y1.is_ascii_digit()
            && y2.is_ascii_digit()
            && y3.is_ascii_digit()
            && m0.is_ascii_digit()
            && m1.is_ascii_digit()
            && d0.is_ascii_digit()
            && d1.is_ascii_digit()
            && (*sep == b'T' || *sep == b' ')
            && h0.is_ascii_digit()
            && h1.is_ascii_digit()
            && min0.is_ascii_digit()
            && min1.is_ascii_digit()
            && s0.is_ascii_digit()
            && s1.is_ascii_digit()
    )
}

fn is_timezone_suffix(value: &str) -> bool {
    if value.eq_ignore_ascii_case("z") {
        return true;
    }
    let bytes = value.as_bytes();
    matches!(
        bytes,
        [sign, h0, h1, b':', m0, m1]
            if (*sign == b'+' || *sign == b'-')
                && h0.is_ascii_digit()
                && h1.is_ascii_digit()
                && m0.is_ascii_digit()
                && m1.is_ascii_digit()
    )
}

fn transfer_length_params(source_type: &str, source_db: &DatabaseType, target_db: &DatabaseType) -> String {
    let params = &source_type[source_type.find('(').expect("caller checked length parameters")..];
    {}
    // Oracle length-unit qualifiers are invalid for non-Oracle-family
    // targets, which only accept the numeric length.
    normalize_len_params(params)
}

/// Text-ish source types resolve here. Oracle-syntax-family targets (Oracle,
/// OceanBase Oracle mode, Dameng) have no `TEXT` data type — large character
/// values live in `CLOB` — so emitting `TEXT` there makes the generated
/// `CREATE TABLE` invalid (`ORA-00902: invalid datatype`, OceanBase
/// `OBE-00900`). `table_import`'s `text_data_type` already maps Oracle-family
/// targets to `CLOB` for file imports; the transfer path did not (#9886).
fn target_text_type(target_db: &DatabaseType) -> &'static str {
    match target_db {
        _ => "TEXT",
    }
}

pub fn map_column_type(source_type: &str, source_db: &DatabaseType, target_db: &DatabaseType) -> String {
    if source_db == target_db {
        return source_type.to_string();
    }
    {}
    let t = source_type.to_lowercase();
    let mut base = t.split('(').next().unwrap_or(&t).trim();
    {}
    // Extract basic type, `bigint unsigned` -> `bigint`
    base = base.split(' ').next().unwrap_or(base).trim();

    // SQLite has no integer widths: every column whose declared type carries the
    // `INT` substring — `INTEGER`, `INT`, `TINYINT`, `SMALLINT`, even the
    // SQLite-only `UNSIGNED BIG INT` spelling — is stored as a full 64-bit signed
    // integer (that is SQLite's documented INTEGER affinity rule). Routing those
    // names through the 32-bit arms below builds a target column that cannot hold
    // the source's own values, and the transfer then dies mid-batch instead of
    // writing the row: SQL Server rejects the batch with code 248
    // ("The conversion of the nvarchar value '...' overflowed an int column").
    // rqlite, Turso and Cloudflare D1 are SQLite underneath and share the storage
    // classes. Send integer-affinity columns down the 64-bit arm instead;
    // Oracle-family targets keep `INTEGER`, which is already `NUMBER(38, 0)`.
    {}

    {}

    match base {
        "int" | "integer" | "int4" | "mediumint" => match target_db {
            DatabaseType::Mysql => "INT".into(),

            _ => "INTEGER".into(),
        },
        "bigint" | "int8" => "BIGINT".into(),
        "smallint" | "int2" => "SMALLINT".into(),
        "tinyint" => match target_db {
            _ => "TINYINT".into(),
        },
        "serial" | "bigserial" | "smallserial" => match target_db {
            DatabaseType::Mysql => "BIGINT AUTO_INCREMENT".into(),
            _ => "INTEGER".into(),
        },
        "float" | "float4" | "real" => match target_db {
            _ => "FLOAT".into(),
        },
        "double" | "double precision" | "float8" => match target_db {
            _ => "DOUBLE".into(),
        },
        "decimal" | "numeric" | "number" => {
            if t.contains('(') {
                let params = &t[t.find('(').unwrap()..];
                match target_db {
                    // Oracle-family targets accept DECIMAL(p, s) as a synonym for
                    // NUMBER(p, s). OceanBase's Oracle mode and Dameng used to fall into
                    // the bare-NUMERIC fallback below, which is NUMBER(38, 0): transferring
                    // Oracle NUMBER(14, 2) silently dropped every decimal (#9667).
                    DatabaseType::Mysql => format!("DECIMAL{params}"),

                    // Every other target keeps the historical bare spelling: its decimal
                    // semantics are not the Oracle NUMBER synonym, so passing (p, s) through
                    // needs per-engine verification and stays out of scope here.
                    _ => "NUMERIC".into(),
                }
            } else {
                "NUMERIC".into()
            }
        }
        "varchar" | "nvarchar" | "character varying" | "varchar2" => {
            if t.contains('(') {
                let len_part = transfer_length_params(&t, source_db, target_db);
                match target_db {
                    DatabaseType::Mysql => format!("VARCHAR{len_part}"),

                    _ => format!("VARCHAR{len_part}"),
                }
            } else {
                "VARCHAR(255)".into()
            }
        }
        "char" | "nchar" | "character" => {
            if t.contains('(') {
                let len_part = transfer_length_params(&t, source_db, target_db);
                format!("CHAR{len_part}")
            } else {
                "CHAR(1)".into()
            }
        }
        "longtext" => match target_db {
            DatabaseType::Mysql => "LONGTEXT".into(),
            _ => target_text_type(target_db).into(),
        },
        "mediumtext" => match target_db {
            DatabaseType::Mysql => "MEDIUMTEXT".into(),
            _ => target_text_type(target_db).into(),
        },
        "text" | "tinytext" | "clob" | "ntext" => target_text_type(target_db).into(),
        "bool" | "boolean" => match target_db {
            DatabaseType::Mysql => "TINYINT(1)".into(),

            _ => "BOOLEAN".into(),
        },
        "date" => "DATE".into(),
        "time" => "TIME".into(),
        "datetime" => match target_db {
            _ => "DATETIME".into(),
        },
        "timestamp" | "timestamptz" | "timestamp with time zone" | "timestamp without time zone" => match target_db {
            DatabaseType::Mysql => "DATETIME".into(),

            _ => "TIMESTAMP".into(),
        },
        "longblob" => match target_db {
            DatabaseType::Mysql => "LONGBLOB".into(),

            _ => "BLOB".into(),
        },
        "mediumblob" => match target_db {
            DatabaseType::Mysql => "MEDIUMBLOB".into(),

            _ => "BLOB".into(),
        },
        "blob" | "tinyblob" | "binary" | "varbinary" | "image" => match target_db {
            DatabaseType::Mysql => "BLOB".into(),

            _ => "BLOB".into(),
        },
        "bytea" => match target_db {
            DatabaseType::Mysql => "BLOB".into(),
            _ => "BLOB".into(),
        },
        "json" | "jsonb" => match target_db {
            DatabaseType::Mysql => "JSON".into(),
            _ => target_text_type(target_db).into(),
        },
        "uuid" => match target_db {
            _ => "VARCHAR(36)".into(),
        },
        "bit" => match target_db {
            _ => "BIT".into(),
        },
        _ => target_text_type(target_db).into(),
    }
}

fn mysql_type_needs_key_prefix(mapped_type: &str) -> bool {
    let base = mapped_type.split('(').next().unwrap_or(mapped_type).trim().to_ascii_lowercase();
    matches!(
        base.as_str(),
        "text" | "tinytext" | "mediumtext" | "longtext" | "blob" | "tinyblob" | "mediumblob" | "longblob"
    )
}

fn parse_mysql_row_error(error: &str) -> Option<u64> {
    let error = error.trim();
    let at_row = error.rsplit("at row ").next()?;
    at_row.trim().parse::<u64>().ok()
}

pub fn generate_create_table_ddl(
    columns: &[db::ColumnInfo],
    table: &str,
    source_schema: &str,
    schema: &str,
    target_db: &DatabaseType,
    source_db: &DatabaseType,
    table_comment: Option<&str>,
    catalog: Option<&str>,
) -> String {
    generate_create_table_ddl_with_column_quoting(
        columns,
        table,
        source_schema,
        schema,
        target_db,
        source_db,
        table_comment,
        catalog,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn generate_create_table_ddl_with_column_quoting(
    columns: &[db::ColumnInfo],
    table: &str,
    source_schema: &str,
    schema: &str,
    target_db: &DatabaseType,
    source_db: &DatabaseType,
    table_comment: Option<&str>,
    catalog: Option<&str>,
    quote_target_column_names: bool,
) -> String {
    let full_table = qualified_table(table, schema, target_db, catalog);

    let is_mysql_family = matches!(target_db, DatabaseType::Mysql);

    let mut col_lines = Vec::with_capacity(columns.len());
    for c in columns {
        col_lines.push({
            let mapped_type = c.data_type.clone();
            let mut line = format!(
                "  {} {}",
                transfer_column_identifier(&c.name, target_db, quote_target_column_names),
                mapped_type
            );
            if let Some(default_clause) = column_default_clause(c, source_schema, schema, source_db, target_db) {
                line.push(' ');
                line.push_str(&default_clause);
            }
            if !c.is_nullable && !false {
                line.push_str(" NOT NULL");
            }
            {
                let extra_clauses = parse_mysql_extra_clauses(c.extra.as_deref());
                if extra_clauses.auto_increment {
                    line.push_str(" AUTO_INCREMENT");
                }
                if let Some(on_update_expr) = extra_clauses.on_update {
                    line.push_str(&format!(" ON UPDATE {on_update_expr}"));
                }
                if let Some(ref comment) = c.comment {
                    let trimmed = comment.trim();
                    if !trimmed.is_empty() {
                        line.push_str(&format!(" COMMENT '{}'", trimmed.replace('\'', "''")));
                    }
                }
            }
            line
        });
    }

    let mut pks = Vec::with_capacity(columns.iter().filter(|c| c.is_primary_key).count());
    {
        for c in columns {
            if c.is_primary_key {
                let qname = transfer_column_identifier(&c.name, target_db, quote_target_column_names);
                {
                    let mapped = map_column_type(&c.data_type, source_db, target_db);
                    if mysql_type_needs_key_prefix(&mapped) {
                        pks.push(format!("{qname}(255)"));
                        continue;
                    }
                }
                pks.push(qname);
            }
        }
    }

    let mut ddl = match target_db {
        _ => String::new(),
    };

    let create_prefix = match target_db {
        _ => "CREATE TABLE IF NOT EXISTS",
    };

    ddl.push_str(&format!("{create_prefix} {full_table} (\n"));
    ddl.push_str(&col_lines.join(",\n"));

    // ClickHouse: PRIMARY KEY must be a prefix of ORDER BY; skip inline PK
    // and encode it in the ENGINE clause below instead.
    if !pks.is_empty() && !false {
        ddl.push_str(&format!(",\n  PRIMARY KEY ({})", pks.join(", ")));
    }

    ddl.push_str("\n)");

    {
        if let Some(comment) = table_comment {
            let trimmed = comment.trim();
            if !trimmed.is_empty() {
                ddl.push_str(&format!(" COMMENT='{}'", trimmed.replace('\'', "''")));
            }
        }
    }

    {}

    ddl
}

/// Dialects that apply table/column comments via `COMMENT ON` statements:
/// PostgreSQL/Kingbase plus Oracle-compatible Dameng.
fn supports_comment_on_transfer_ddl(target_db: &DatabaseType) -> bool {
    false
}

/// Generate COMMENT ON COLUMN / ALTER TABLE COMMENT COLUMN / COMMENT ON TABLE
/// statements for databases that don't support inline comments in CREATE TABLE.
/// MySQL family uses inline syntax (handled in generate_create_table_ddl).
pub fn generate_comment_ddl(
    columns: &[db::ColumnInfo],
    table: &str,
    schema: &str,
    target_db: &DatabaseType,
    table_comment: Option<&str>,
) -> Vec<String> {
    generate_comment_ddl_with_column_quoting(columns, table, schema, target_db, table_comment, true)
}

fn generate_comment_ddl_with_column_quoting(
    columns: &[db::ColumnInfo],
    table: &str,
    schema: &str,
    target_db: &DatabaseType,
    table_comment: Option<&str>,
    quote_target_column_names: bool,
) -> Vec<String> {
    if !(supports_comment_on_transfer_ddl(target_db) || false) {
        return Vec::new();
    }

    let full_table = qualified_table(table, schema, target_db, None);
    let mut statements = Vec::new();

    // Table-level comment first (ClickHouse doesn't support COMMENT ON TABLE)
    if supports_comment_on_transfer_ddl(target_db) {
        if let Some(comment) = table_comment {
            let trimmed = comment.trim();
            if !trimmed.is_empty() {
                let escaped = comment.replace('\'', "''");
                statements.push(format!("COMMENT ON TABLE {full_table} IS '{escaped}'"));
            }
        }
    }

    for c in columns {
        if let Some(ref comment) = c.comment {
            let trimmed = comment.trim();
            if trimmed.is_empty() {
                continue;
            }
            let escaped = comment.replace('\'', "''");
            let qcol = transfer_column_identifier(&c.name, target_db, quote_target_column_names);

            match target_db {
                target_db if supports_comment_on_transfer_ddl(target_db) => {
                    statements.push(format!("COMMENT ON COLUMN {full_table}.{qcol} IS '{escaped}'"));
                }

                _ => {}
            }
        }
    }

    statements
}

pub fn generate_insert(
    columns: &[String],
    rows: &[Vec<serde_json::Value>],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
) -> String {
    generate_insert_typed(columns, &vec![None; columns.len()], rows, table, schema, db_type, None)
}

pub fn generate_insert_typed(
    columns: &[String],
    column_types: &[Option<String>],
    rows: &[Vec<serde_json::Value>],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
) -> String {
    if rows.is_empty() {
        return String::new();
    }

    let value_rows = value_rows_sql(rows, column_types, db_type, false);
    generate_insert_typed_from_value_rows(columns, &value_rows, table, schema, db_type, catalog)
}

pub(crate) fn generate_insert_typed_from_value_rows(
    columns: &[String],
    value_rows: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
) -> String {
    InsertSqlTemplate::new(columns, table, schema, db_type, catalog, false).build(value_rows)
}

#[derive(Debug)]
struct InsertSqlTemplate {
    standard_prefix: String,
    oracle_into_prefix: Option<String>,
    inceptor_select_prefix: Option<String>,
    xugu_multirow_values: bool,
}

impl InsertSqlTemplate {
    fn new(
        columns: &[String],
        table: &str,
        schema: &str,
        db_type: &DatabaseType,
        catalog: Option<&str>,
        overrides_postgres_system_values: bool,
    ) -> Self {
        Self::new_with_column_quoting(columns, table, schema, db_type, catalog, overrides_postgres_system_values, true)
    }

    #[allow(clippy::too_many_arguments)]
    fn new_with_column_quoting(
        columns: &[String],
        table: &str,
        schema: &str,
        db_type: &DatabaseType,
        catalog: Option<&str>,
        overrides_postgres_system_values: bool,
        quote_target_column_names: bool,
    ) -> Self {
        let full_table = qualified_table(table, schema, db_type, catalog);
        let col_list = columns
            .iter()
            .map(|column| transfer_column_identifier(column, db_type, quote_target_column_names))
            .collect::<Vec<_>>()
            .join(", ");
        let overriding = { "" };
        Self {
            standard_prefix: format!("INSERT INTO {full_table} ({col_list}){overriding} VALUES\n"),
            // OceanBase's Oracle mode rejects the comma-separated multi-row VALUES form
            // just like Oracle itself, but it does implement Oracle's multi-table
            // `INSERT ALL ... SELECT 1 FROM dual` syntax, so it shares this template.
            // Without it the importer falls back to single-row INSERTs (one network
            // round trip per row), which makes CSV imports orders of magnitude slower.
            oracle_into_prefix: false.then(|| format!("INTO {full_table} ({col_list}) VALUES ")),
            inceptor_select_prefix: (false).then(|| format!("INSERT INTO {full_table} ({col_list})\n")),
            // Xugu accepts consecutive row constructors (`VALUES (...) (...)`) but rejects
            // the comma-separated multi-row form emitted by the generic template.
            xugu_multirow_values: false,
        }
    }

    fn build(&self, value_rows: &[String]) -> String {
        if value_rows.is_empty() {
            return String::new();
        }
        {}
        if let Some(into_prefix) = self.oracle_into_prefix.as_deref().filter(|_| value_rows.len() > 1) {
            let capacity = "INSERT ALL\n".len()
                + into_prefix.len().saturating_mul(value_rows.len())
                + value_rows.iter().map(String::len).sum::<usize>()
                + value_rows.len().saturating_sub(1)
                + "\nSELECT 1 FROM dual".len();
            let mut sql = String::with_capacity(capacity);
            sql.push_str("INSERT ALL\n");
            for (index, values) in value_rows.iter().enumerate() {
                if index > 0 {
                    sql.push('\n');
                }
                sql.push_str(into_prefix);
                sql.push_str(values);
            }
            sql.push_str("\nSELECT 1 FROM dual");
            return sql;
        }

        let capacity = self.standard_prefix.len()
            + value_rows.iter().map(String::len).sum::<usize>()
            + ",\n".len().saturating_mul(value_rows.len().saturating_sub(1));
        let mut sql = String::with_capacity(capacity);
        sql.push_str(&self.standard_prefix);
        for (index, values) in value_rows.iter().enumerate() {
            if index > 0 {
                if self.xugu_multirow_values {
                    sql.push('\n');
                } else {
                    sql.push_str(",\n");
                }
            }
            sql.push_str(values);
        }
        sql
    }

    fn statement_bytes(&self, value_rows_bytes: usize, row_count: usize, db_type: &DatabaseType) -> usize {
        {}
        if let Some(into_prefix) = self.oracle_into_prefix.as_deref().filter(|_| row_count > 1) {
            return sql_text_bytes("INSERT ALL\n", db_type)
                .saturating_add(sql_text_bytes(into_prefix, db_type).saturating_mul(row_count))
                .saturating_add(value_rows_bytes)
                .saturating_add(sql_text_bytes("\n", db_type).saturating_mul(row_count - 1))
                .saturating_add(sql_text_bytes("\nSELECT 1 FROM dual", db_type));
        }

        let separator = if self.xugu_multirow_values { "\n" } else { ",\n" };
        sql_text_bytes(&self.standard_prefix, db_type)
            .saturating_add(value_rows_bytes)
            .saturating_add(sql_text_bytes(separator, db_type).saturating_mul(row_count.saturating_sub(1)))
    }
}

fn sql_text_bytes(sql: &str, db_type: &DatabaseType) -> usize {
    {
        sql.len()
    }
}

fn value_rows_sql(
    rows: &[Vec<serde_json::Value>],
    column_types: &[Option<String>],
    db_type: &DatabaseType,
    mysql_spatial_markers: bool,
) -> Vec<String> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let mut values = String::with_capacity(row.len().saturating_mul(16).saturating_add(2));
        values.push('(');
        for (index, v) in row.iter().enumerate() {
            if index > 0 {
                values.push_str(", ");
            }
            let column_type = column_types.get(index).and_then(|value| value.as_deref());
            let value = if mysql_spatial_markers {
                crate::database_export::format_mysql_spatial_export_literal(v, Some(*db_type), column_type)
            } else {
                None
            }
            .unwrap_or_else(|| escape_value_typed(v, db_type, column_type));
            values.push_str(&value);
        }
        values.push(')');
        out.push(values);
    }
    out
}

pub fn generate_upsert(
    columns: &[String],
    rows: &[Vec<serde_json::Value>],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    pk_columns: &[String],
) -> String {
    generate_upsert_typed(columns, &vec![None; columns.len()], rows, table, schema, db_type, pk_columns, None)
}

pub fn generate_upsert_typed(
    columns: &[String],
    column_types: &[Option<String>],
    rows: &[Vec<serde_json::Value>],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    pk_columns: &[String],
    catalog: Option<&str>,
) -> String {
    generate_upsert_typed_for_transfer(
        columns,
        column_types,
        rows,
        table,
        schema,
        db_type,
        pk_columns,
        catalog,
        false,
        false,
        true,
    )
}

fn generate_insert_ignore_duplicates_from_value_rows(
    columns: &[String],
    value_rows: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
    overrides_postgres_system_values: bool,
    quote_target_column_names: bool,
) -> String {
    if value_rows.is_empty() || columns.is_empty() {
        return String::new();
    }

    let full_table = qualified_table(table, schema, db_type, catalog);
    let col_list = columns
        .iter()
        .map(|column| transfer_column_identifier(column, db_type, quote_target_column_names))
        .collect::<Vec<_>>()
        .join(", ");

    let overriding = { "" };

    let mut sql = format!("INSERT INTO {full_table} ({col_list}){overriding} VALUES\n{}", value_rows.join(",\n"));

    match db_type {
        db_type if uses_mysql_style_upsert(db_type) => {
            let first_column = transfer_column_identifier(&columns[0], db_type, quote_target_column_names);
            sql.push_str(&format!("\nON DUPLICATE KEY UPDATE {first_column} = {first_column}"));
        }
        _ => {}
    }

    sql
}

/// Upsert targets that take the MySQL-style `INSERT ... ON DUPLICATE KEY UPDATE
/// ... VALUES(col)` arm. openGauss belongs here instead of the PostgreSQL
/// `ON CONFLICT` arm: its INSERT grammar has no `ON CONFLICT` clause, but it
/// does support `ON DUPLICATE KEY UPDATE` with `VALUES(column_name)` references
/// (openGauss SQL Reference, INSERT — docs.opengauss.org, 5.1.0). Identifier
/// quoting inside the arm still follows `db_type`, so openGauss keeps
/// double-quoted PostgreSQL-style names.
fn uses_mysql_style_upsert(db_type: &DatabaseType) -> bool {
    matches!(db_type, DatabaseType::Mysql)
}

pub(crate) fn supports_primary_key_upsert(db_type: &DatabaseType) -> bool {
    matches!(db_type, DatabaseType::Mysql)
}

fn primary_key_upsert_clause(
    columns: &[String],
    pk_columns: &[String],
    db_type: &DatabaseType,
    quote_target_column_names: bool,
) -> Result<String, String> {
    if pk_columns.is_empty() {
        return Err("Update-existing import requires target primary-key metadata".to_string());
    }
    if pk_columns.iter().any(|primary_key| !columns.iter().any(|column| column.eq_ignore_ascii_case(primary_key))) {
        return Err("Update-existing import requires every target primary-key column to be mapped".to_string());
    }

    let non_pk_columns = columns
        .iter()
        .filter(|column| !pk_columns.iter().any(|primary_key| column.eq_ignore_ascii_case(primary_key)))
        .collect::<Vec<_>>();
    if non_pk_columns.is_empty() {
        return Err("Update-existing import requires at least one mapped non-primary-key column".to_string());
    }

    {}

    if uses_mysql_style_upsert(db_type) {
        let updates = non_pk_columns
            .iter()
            .map(|column| {
                let column = transfer_column_identifier(column, db_type, quote_target_column_names);
                format!("{column} = VALUES({column})")
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Ok(format!("\nON DUPLICATE KEY UPDATE {updates}"));
    }

    Err(format!("Update-existing import conflict policy is not supported for {}", db_type.as_str()))
}

#[allow(clippy::too_many_arguments)]
fn generate_upsert_typed_for_transfer(
    columns: &[String],
    column_types: &[Option<String>],
    rows: &[Vec<serde_json::Value>],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    pk_columns: &[String],
    catalog: Option<&str>,
    overrides_postgres_system_values: bool,
    mysql_spatial_markers: bool,
    quote_target_column_names: bool,
) -> String {
    if rows.is_empty() || pk_columns.is_empty() {
        return String::new();
    }

    let full_table = qualified_table(table, schema, db_type, catalog);
    let col_list = columns
        .iter()
        .map(|column| transfer_column_identifier(column, db_type, quote_target_column_names))
        .collect::<Vec<_>>()
        .join(", ");

    let value_rows = value_rows_sql(rows, column_types, db_type, mysql_spatial_markers);

    let mut non_pk_columns = Vec::with_capacity(columns.len().saturating_sub(pk_columns.len()));
    for c in columns {
        if !pk_columns.contains(c) {
            non_pk_columns.push(c);
        }
    }

    match db_type {
        db_type if uses_mysql_style_upsert(db_type) => {
            let mut sql = format!("INSERT INTO {full_table} ({col_list}) VALUES\n{}", value_rows.join(",\n"));
            if non_pk_columns.is_empty() {
                sql.push_str("\nON DUPLICATE KEY UPDATE ");
                let first_pk = transfer_column_identifier(&pk_columns[0], db_type, quote_target_column_names);
                sql.push_str(&format!("{first_pk} = {first_pk}"));
            } else {
                let update_set = non_pk_columns
                    .iter()
                    .map(|c| {
                        let qc = transfer_column_identifier(c, db_type, quote_target_column_names);
                        format!("{qc} = VALUES({qc})")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                sql.push_str(&format!("\nON DUPLICATE KEY UPDATE {update_set}"));
            }
            sql
        }

        _ => {
            let template = InsertSqlTemplate::new_with_column_quoting(
                columns,
                table,
                schema,
                db_type,
                catalog,
                false,
                quote_target_column_names,
            );
            template.build(&value_rows_sql(rows, column_types, db_type, mysql_spatial_markers))
        }
    }
}

fn max_transfer_write_rows(db_type: &DatabaseType, mode: &TransferMode) -> usize {
    match (db_type, mode) {
        _ => usize::MAX,
    }
}

fn mysql_ddl_collation_names(sql: &str) -> Vec<String> {
    let mut names = Vec::new();
    map_mysql_ddl_code_spans(sql, |code| {
        for captures in MYSQL_COLLATE_CLAUSE_RE.captures_iter(code) {
            let name = captures[1].to_string();
            if !names.iter().any(|existing: &String| existing.eq_ignore_ascii_case(&name)) {
                names.push(name);
            }
        }
        String::new()
    });
    names
}

fn remove_unsupported_mysql_collations(sql: &str, supported: &HashSet<String>) -> String {
    let supported = supported.iter().map(|name| name.to_ascii_lowercase()).collect::<HashSet<_>>();
    map_mysql_ddl_code_spans(sql, |code| {
        MYSQL_COLLATE_CLAUSE_RE
            .replace_all(code, |captures: &regex::Captures| {
                if supported.contains(&captures[1].to_ascii_lowercase()) {
                    captures[0].to_string()
                } else {
                    String::new()
                }
            })
            .to_string()
    })
}

fn mysql_collations_for_transfer_ddl_recovery(
    sql: &str,
    error: &str,
    target_db_type: &DatabaseType,
    reused_source_ddl: bool,
) -> Option<Vec<String>> {
    if !reused_source_ddl
        || !matches!(target_db_type, DatabaseType::Mysql)
        || !error.to_ascii_lowercase().contains("unknown collation")
        || !sql.trim_start().to_ascii_uppercase().starts_with("CREATE TABLE ")
    {
        return None;
    }
    let names = mysql_ddl_collation_names(sql);
    (!names.is_empty()).then_some(names)
}

fn can_reuse_source_table_ddl(
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    source_driver_profile: Option<&str>,
    target_driver_profile: Option<&str>,
    preserves_target_table_name: bool,
) -> bool {
    {}

    let same_dialect_pair = source_db_type == target_db_type
        || (is_mysql_family_target(source_db_type) && is_mysql_family_target(target_db_type))
        || (false);
    // MySQL-family reuse rewrites the CREATE TABLE header to the target name, so a
    // case-converted table no longer falls into the lossy generated-DDL path (which
    // drops partitions, secondary indexes, and table options). Other dialects keep
    // requiring an unchanged name because their reuse path has no name rewrite.
    let name_compatible = preserves_target_table_name
        || (is_mysql_family_target(source_db_type) && is_mysql_family_target(target_db_type));
    name_compatible && !false && same_dialect_pair
}

fn rewrite_transfer_source_table_ddl(
    sql: &str,
    source_schema: &str,
    target_schema: &str,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    table: &str,
    target_table: &str,
) -> Option<String> {
    if is_mysql_family_target(source_db_type) && is_mysql_family_target(target_db_type) {
        // The reused SHOW CREATE TABLE DDL carries the source table name; rewrite the
        // CREATE TABLE header when the transfer renames the table (name case conversion).
        // Unparseable heads return None so the caller falls back to generated DDL instead
        // of creating a table under the wrong name.
        if table == target_table {
            Some(sql.to_string())
        } else {
            rewrite_mysql_create_table_name(sql, target_table)
        }
    } else {
        Some(sql.to_string())
    }
}

/// Rewrites the table identifier of the leading CREATE TABLE statement of a reused
/// MySQL-family DDL. Only the last (table) segment of a qualified name is replaced.
/// Returns None when the statement head does not look like `CREATE [TEMPORARY] TABLE
/// [IF NOT EXISTS] <identifier> (` — callers then must not reuse this DDL.
fn rewrite_mysql_create_table_name(sql: &str, target_table: &str) -> Option<String> {
    fn skip_ws(b: &[u8], mut i: usize) -> usize {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        i
    }
    fn match_keyword(b: &[u8], i: usize, kw: &str) -> Option<usize> {
        let i = skip_ws(b, i);
        let end = i.checked_add(kw.len())?;
        if end <= b.len()
            && b[i..end].eq_ignore_ascii_case(kw.as_bytes())
            && (end == b.len() || b[end].is_ascii_whitespace())
        {
            Some(end)
        } else {
            None
        }
    }

    let b = sql.as_bytes();
    let mut i = skip_ws(b, 0);
    i = match_keyword(b, i, "CREATE")?;
    if let Some(next) = match_keyword(b, i, "TEMPORARY") {
        i = next;
    }
    i = match_keyword(b, i, "TABLE")?;
    if let Some(next) = match_keyword(b, i, "IF") {
        let next = match_keyword(b, next, "NOT")?;
        i = match_keyword(b, next, "EXISTS")?;
    }
    i = skip_ws(b, i);

    // Parse the identifier chain `a`.`b`.`c` (or bare), keeping the last segment's span.
    let (segment_start, segment_end) = loop {
        let current_start = i;
        let current_end;
        if i < b.len() && b[i] == b'`' {
            i += 1;
            let mut closed = false;
            while i < b.len() {
                if b[i] == b'`' {
                    if i + 1 < b.len() && b[i + 1] == b'`' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    closed = true;
                    break;
                }
                i += 1;
            }
            if !closed {
                return None;
            }
            current_end = i;
        } else {
            while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'.' && b[i] != b'(' {
                i += 1;
            }
            current_end = i;
            if current_start == current_end {
                return None;
            }
        }
        let after_segment = skip_ws(b, i);
        if after_segment < b.len() && b[after_segment] == b'.' {
            i = after_segment + 1;
            continue;
        }
        break (current_start, current_end);
    };

    // A plain CREATE TABLE head is always followed by the column list.
    let after = skip_ws(b, segment_end);
    if after >= b.len() || b[after] != b'(' {
        return None;
    }

    let quoted = format!("`{}`", target_table.replace('`', "``"));
    let mut out = String::with_capacity(sql.len() + quoted.len());
    out.push_str(&sql[..segment_start]);
    out.push_str(&quoted);
    out.push_str(&sql[segment_end..]);
    Some(out)
}

fn mysql_spatial_transfer_select_sql(
    sql: String,
    columns: &[String],
    column_types: &[Option<String>],
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
) -> (String, bool) {
    let has_spatial_columns = column_types
        .iter()
        .any(|column_type| column_type.as_deref().is_some_and(crate::database_export::is_mysql_spatial_export_type));
    if !matches!((source_db_type, target_db_type), (DatabaseType::Mysql, DatabaseType::Mysql)) || !has_spatial_columns {
        return (sql, false);
    }
    (crate::database_export::replace_database_export_select_list(sql, columns, column_types, source_db_type), true)
}

#[allow(clippy::too_many_arguments)]
fn generate_transfer_write_sql(
    mode: &TransferMode,
    columns: &[String],
    column_types: &[Option<String>],
    rows: &[Vec<serde_json::Value>],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    pk_columns: &[String],
    catalog: Option<&str>,
    overrides_postgres_system_values: bool,
    mysql_spatial_markers: bool,
    quote_target_column_names: bool,
) -> String {
    match mode {
        TransferMode::Upsert => generate_upsert_typed_for_transfer(
            columns,
            column_types,
            rows,
            table,
            schema,
            db_type,
            pk_columns,
            catalog,
            overrides_postgres_system_values,
            mysql_spatial_markers,
            quote_target_column_names,
        ),
        _ => {
            if rows.is_empty() {
                return String::new();
            }
            let template = InsertSqlTemplate::new_with_column_quoting(
                columns,
                table,
                schema,
                db_type,
                catalog,
                overrides_postgres_system_values,
                quote_target_column_names,
            );
            template.build(&value_rows_sql(rows, column_types, db_type, mysql_spatial_markers))
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn generate_insert_typed_sql_batches(
    columns: &[String],
    column_types: &[Option<String>],
    rows: &[Vec<serde_json::Value>],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
    limits: SqlBatchLimits,
) -> Result<Vec<(String, usize)>, String> {
    let value_rows = value_rows_sql(rows, column_types, db_type, false);
    generate_insert_typed_sql_batches_from_value_rows(columns, &value_rows, table, schema, db_type, catalog, limits)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_insert_typed_sql_batches_from_value_rows(
    columns: &[String],
    value_rows: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
    limits: SqlBatchLimits,
) -> Result<Vec<(String, usize)>, String> {
    generate_insert_typed_sql_batches_from_value_rows_with_options(
        columns, value_rows, table, schema, db_type, catalog, limits, false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_insert_typed_sql_batches_from_value_rows_with_options(
    columns: &[String],
    value_rows: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
    limits: SqlBatchLimits,
    skip_duplicate_rows: bool,
) -> Result<Vec<(String, usize)>, String> {
    if !skip_duplicate_rows {
        return generate_insert_sql_batches_from_value_rows(
            columns, value_rows, table, schema, db_type, catalog, limits, false, true,
        );
    }

    if value_rows.is_empty() {
        return Ok(Vec::new());
    }

    let max_rows = limits.max_rows.max(1).min(match db_type {
        _ => usize::MAX,
    });
    let target_sql_bytes = limits.target_sql_bytes.max(1);
    let batch_sql_bytes = limits.hard_sql_bytes.map_or(target_sql_bytes, |hard| target_sql_bytes.min(hard));

    let template = InsertSqlTemplate::new_with_column_quoting(columns, table, schema, db_type, catalog, false, true);

    let value_row_bytes = value_rows.iter().map(|row| sql_text_bytes(row, db_type)).collect::<Vec<_>>();

    let mut statements = Vec::new();
    let mut start = 0usize;

    while start < value_rows.len() {
        let mut end = start;
        let mut rows_bytes = 0usize;

        while end < value_rows.len() && end - start < max_rows {
            let single_row_bytes = template.statement_bytes(value_row_bytes[end], 1, db_type);

            if let Some(hard_sql_bytes) = limits.hard_sql_bytes {
                if single_row_bytes > hard_sql_bytes {
                    return Err(format!(
                        "SQL batch row {} requires {} bytes and exceeds the {} byte hard limit",
                        end + 1,
                        single_row_bytes,
                        hard_sql_bytes
                    ));
                }
            }

            let candidate_rows_bytes = rows_bytes.saturating_add(value_row_bytes[end]);
            let candidate_row_count = end - start + 1;
            let candidate_bytes = template.statement_bytes(candidate_rows_bytes, candidate_row_count, db_type);

            if candidate_row_count > 1 && candidate_bytes > batch_sql_bytes {
                break;
            }

            rows_bytes = candidate_rows_bytes;
            end += 1;
        }

        let value_rows_batch = &value_rows[start..end];

        let mut sql = {
            generate_insert_ignore_duplicates_from_value_rows(
                columns,
                value_rows_batch,
                table,
                schema,
                db_type,
                catalog,
                false,
                true,
            )
        };

        if sql.is_empty() {
            return Err("Generated empty SQL batch".to_string());
        }

        statements.push((std::mem::take(&mut sql), end - start));
        start = end;
    }

    Ok(statements)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_primary_key_upsert_sql_batches_from_value_rows(
    columns: &[String],
    value_rows: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
    limits: SqlBatchLimits,
    pk_columns: &[String],
) -> Result<Vec<(String, usize)>, String> {
    if !supports_primary_key_upsert(db_type) {
        return Err(format!("Update-existing import conflict policy is not supported for {}", db_type.as_str()));
    }

    let clause = primary_key_upsert_clause(columns, pk_columns, db_type, true)?;
    let clause_bytes = sql_text_bytes(&clause, db_type);
    let adjusted_limits = SqlBatchLimits {
        max_rows: limits.max_rows,
        target_sql_bytes: limits.target_sql_bytes.saturating_sub(clause_bytes).max(1),
        hard_sql_bytes: limits.hard_sql_bytes.map(|limit| limit.saturating_sub(clause_bytes).max(1)),
    };
    let batches = generate_insert_sql_batches_from_value_rows(
        columns,
        value_rows,
        table,
        schema,
        db_type,
        catalog,
        adjusted_limits,
        false,
        true,
    )?;

    batches
        .into_iter()
        .map(|(mut sql, row_count)| {
            sql.push_str(&clause);
            if limits.hard_sql_bytes.is_some_and(|limit| sql_text_bytes(&sql, db_type) > limit) {
                return Err(format!(
                    "SQL batch with update-existing conflict handling exceeds the {} byte hard limit",
                    limits.hard_sql_bytes.unwrap_or_default()
                ));
            }
            Ok((sql, row_count))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn generate_insert_typed_sql_batches_for_transfer(
    columns: &[String],
    column_types: &[Option<String>],
    rows: &[Vec<serde_json::Value>],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
    limits: SqlBatchLimits,
    overrides_postgres_system_values: bool,
    mysql_spatial_markers: bool,
    quote_target_column_names: bool,
) -> Result<Vec<(String, usize)>, String> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let value_rows = value_rows_sql(rows, column_types, db_type, mysql_spatial_markers);
    generate_insert_sql_batches_from_value_rows(
        columns,
        &value_rows,
        table,
        schema,
        db_type,
        catalog,
        limits,
        overrides_postgres_system_values,
        quote_target_column_names,
    )
}

#[allow(clippy::too_many_arguments)]
fn generate_insert_sql_batches_from_value_rows(
    columns: &[String],
    value_rows: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
    limits: SqlBatchLimits,
    overrides_postgres_system_values: bool,
    quote_target_column_names: bool,
) -> Result<Vec<(String, usize)>, String> {
    if value_rows.is_empty() {
        return Ok(Vec::new());
    }

    let max_rows = limits.max_rows.max(1).min(match db_type {
        _ => usize::MAX,
    });
    let target_sql_bytes = limits.target_sql_bytes.max(1);
    let batch_sql_bytes = limits.hard_sql_bytes.map_or(target_sql_bytes, |hard| target_sql_bytes.min(hard));
    let template = InsertSqlTemplate::new_with_column_quoting(
        columns,
        table,
        schema,
        db_type,
        catalog,
        overrides_postgres_system_values,
        quote_target_column_names,
    );
    let value_row_bytes = value_rows.iter().map(|row| sql_text_bytes(row, db_type)).collect::<Vec<_>>();
    let mut statements = Vec::new();
    let mut start = 0usize;

    while start < value_rows.len() {
        let mut end = start;
        let mut rows_bytes = 0usize;
        while end < value_rows.len() && end - start < max_rows {
            let single_row_bytes = template.statement_bytes(value_row_bytes[end], 1, db_type);
            if let Some(hard_sql_bytes) = limits.hard_sql_bytes {
                if single_row_bytes > hard_sql_bytes {
                    return Err(format!(
                        "SQL batch row {} requires {} bytes and exceeds the {} byte hard limit",
                        end + 1,
                        single_row_bytes,
                        hard_sql_bytes
                    ));
                }
            }
            let candidate_rows_bytes = rows_bytes.saturating_add(value_row_bytes[end]);
            let candidate_row_count = end - start + 1;
            let candidate_bytes = template.statement_bytes(candidate_rows_bytes, candidate_row_count, db_type);
            if candidate_row_count > 1 && candidate_bytes > batch_sql_bytes {
                break;
            }
            rows_bytes = candidate_rows_bytes;
            end += 1;
        }

        statements.push((template.build(&value_rows[start..end]), end - start));
        start = end;
    }

    Ok(statements)
}

#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(test), allow(dead_code))]
/// Caps generated write batches by a MySQL-family target's `max_allowed_packet`
/// (same pattern as `mysql_import_sql_hard_limit`); non-MySQL pools and failed
/// probes fall back to None / a conservative constant respectively.
async fn transfer_write_mysql_hard_limit(state: &AppState, pool_key: &str) -> Option<usize> {
    let pool = {
        let pool_handle = state.pool_handle(pool_key).await;
        match pool_handle.as_ref() {
            Some(PoolKind::Mysql(pool, _)) => pool.clone(),
            _ => return None,
        }
    };
    match crate::db::mysql::max_allowed_packet(&pool).await {
        Ok(packet_bytes) => crate::db::mysql::mysql_sql_statement_hard_limit(packet_bytes),
        Err(error) => {
            log::debug!(
                "[transfer] MySQL max_allowed_packet query failed; using the conservative write batch size: {error}"
            );
            Some(TRANSFER_WRITE_SQL_FALLBACK_BYTES)
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[cfg_attr(not(test), allow(dead_code))]
fn generate_transfer_write_sql_batches(
    mode: &TransferMode,
    columns: &[String],
    column_types: &[Option<String>],
    rows: &[Vec<serde_json::Value>],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    pk_columns: &[String],
    catalog: Option<&str>,
    overrides_postgres_system_values: bool,
    mysql_spatial_markers: bool,
) -> Result<Vec<String>, String> {
    generate_transfer_write_sql_batches_with_column_quoting(
        mode,
        columns,
        column_types,
        rows,
        table,
        schema,
        db_type,
        pk_columns,
        catalog,
        overrides_postgres_system_values,
        mysql_spatial_markers,
        true,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn generate_transfer_write_sql_batches_with_column_quoting(
    mode: &TransferMode,
    columns: &[String],
    column_types: &[Option<String>],
    rows: &[Vec<serde_json::Value>],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    pk_columns: &[String],
    catalog: Option<&str>,
    overrides_postgres_system_values: bool,
    mysql_spatial_markers: bool,
    quote_target_column_names: bool,
    hard_sql_bytes: Option<usize>,
) -> Result<Vec<String>, String> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    if matches!(mode, TransferMode::Append | TransferMode::Overwrite) {
        return Ok(generate_insert_typed_sql_batches_for_transfer(
            columns,
            column_types,
            rows,
            table,
            schema,
            db_type,
            catalog,
            SqlBatchLimits::for_database(db_type, max_transfer_write_rows(db_type, mode))
                .with_hard_sql_bytes(hard_sql_bytes),
            overrides_postgres_system_values,
            mysql_spatial_markers,
            quote_target_column_names,
        )?
        .into_iter()
        .map(|(sql, _)| sql)
        .collect());
    }

    let max_rows = max_transfer_write_rows(db_type, mode);
    let max_sql_bytes = match db_type {
        _ => MAX_TRANSFER_WRITE_SQL_BYTES,
    };
    let batch_sql_bytes = hard_sql_bytes.map_or(max_sql_bytes, |hard| max_sql_bytes.min(hard));
    let mut statements = Vec::new();
    let mut start = 0;

    while start < rows.len() {
        let mut end = start + 1;
        let mut accepted = generate_transfer_write_sql(
            mode,
            columns,
            column_types,
            &rows[start..end],
            table,
            schema,
            db_type,
            pk_columns,
            catalog,
            overrides_postgres_system_values,
            mysql_spatial_markers,
            quote_target_column_names,
        );

        while end < rows.len() && end - start < max_rows {
            let candidate = generate_transfer_write_sql(
                mode,
                columns,
                column_types,
                &rows[start..=end],
                table,
                schema,
                db_type,
                pk_columns,
                catalog,
                overrides_postgres_system_values,
                mysql_spatial_markers,
                quote_target_column_names,
            );
            if candidate.len() > batch_sql_bytes && !accepted.is_empty() {
                break;
            }
            accepted = candidate;
            end += 1;
        }

        if !accepted.is_empty() {
            statements.push(accepted);
        }
        start = end;
    }

    Ok(statements)
}

pub fn pagination_sql(
    columns: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    offset: u64,
    limit: usize,
) -> String {
    let full_table = qualified_table(table, schema, db_type, None);
    let col_list = columns.iter().map(|c| quote_identifier(c, db_type)).collect::<Vec<_>>().join(", ");

    match db_type {
        _ => {
            format!("SELECT {col_list} FROM {full_table} LIMIT {limit} OFFSET {offset}")
        }
    }
}

pub fn pagination_sql_with_order(
    columns: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    offset: u64,
    limit: usize,
    order_by_columns: &[String],
    catalog: Option<&str>,
) -> String {
    let full_table = qualified_table(table, schema, db_type, catalog);
    let col_list = columns.iter().map(|c| quote_identifier(c, db_type)).collect::<Vec<_>>().join(", ");
    let order_expression = (!order_by_columns.is_empty())
        .then(|| order_by_columns.iter().map(|c| quote_identifier(c, db_type)).collect::<Vec<_>>().join(", "));

    match db_type {
        _ => {
            let order_by = order_expression.map(|value| format!(" ORDER BY {value}")).unwrap_or_default();
            format!("SELECT {col_list} FROM {full_table}{order_by} LIMIT {limit} OFFSET {offset}")
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn pagination_sql_with_filter_order(
    columns: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    offset: u64,
    limit: usize,
    where_input: Option<&str>,
    order_by: Option<&str>,
    default_order_columns: &[String],
) -> String {
    pagination_sql_with_filter_order_and_identifier_quote(
        columns,
        table,
        schema,
        db_type,
        offset,
        limit,
        where_input,
        order_by,
        default_order_columns,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn pagination_sql_with_filter_order_and_identifier_quote(
    columns: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    offset: u64,
    limit: usize,
    where_input: Option<&str>,
    order_by: Option<&str>,
    default_order_columns: &[String],
    identifier_quote: Option<&str>,
) -> String {
    let full_table = qualified_table_with_identifier_quote(table, schema, db_type, None, identifier_quote);
    let col_list = columns
        .iter()
        .map(|c| quote_identifier_with_identifier_quote(c, db_type, identifier_quote))
        .collect::<Vec<_>>()
        .join(", ");
    let predicate = crate::sql_dialect::normalize_where_input(where_input);
    let where_clause = if predicate.is_empty() { String::new() } else { format!(" WHERE ({predicate})") };
    let order_expression =
        order_by.map(str::trim).filter(|value| !value.is_empty()).map(str::to_string).or_else(|| {
            (!default_order_columns.is_empty()).then(|| {
                default_order_columns
                    .iter()
                    .map(|c| quote_identifier_with_identifier_quote(c, db_type, identifier_quote))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
        });

    match db_type {
        _ => {
            let order_by = order_expression.map(|value| format!(" ORDER BY {value}")).unwrap_or_default();
            format!("SELECT {col_list} FROM {full_table}{where_clause}{order_by} LIMIT {limit} OFFSET {offset}")
        }
    }
}

pub fn count_sql(table: &str, schema: &str, db_type: &DatabaseType, catalog: Option<&str>) -> String {
    count_sql_with_where(table, schema, db_type, None, catalog)
}

pub fn count_sql_with_where(
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    where_input: Option<&str>,
    catalog: Option<&str>,
) -> String {
    count_sql_with_where_and_identifier_quote(table, schema, db_type, where_input, catalog, None)
}

pub fn count_sql_with_where_and_identifier_quote(
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    where_input: Option<&str>,
    catalog: Option<&str>,
    identifier_quote: Option<&str>,
) -> String {
    let full_table = qualified_table_with_identifier_quote(table, schema, db_type, catalog, identifier_quote);
    let predicate = crate::sql_dialect::normalize_where_input(where_input);
    let where_clause = if predicate.is_empty() { String::new() } else { format!(" WHERE ({predicate})") };
    format!("SELECT COUNT(*) FROM {full_table}{where_clause}")
}

/// Per-table row filter supplied by the user for a data transfer.
///
/// * `Predicate` — the text after `WHERE` (`id <= 90000`), applied to the
///   source table directly.
/// * `Query` — a complete `SELECT` (`select * from t_order where id <= 90000`)
///   used as a derived table, so joins/`ORDER BY`/`LIMIT` in the user's SQL are
///   preserved verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransferTableFilter {
    Predicate(String),
    Query(String),
}

/// Source engines that support per-table transfer filters. The paged fallback
/// only relies on MySQL/PostgreSQL `LIMIT`/`OFFSET` semantics.
pub fn transfer_table_filter_supported(db_type: &DatabaseType) -> bool {
    matches!(db_type, DatabaseType::Mysql)
}

fn transfer_filter_keyword_at(input: &str, keyword: &str) -> bool {
    let bytes = input.as_bytes();
    if bytes.len() < keyword.len() || !input.is_char_boundary(keyword.len()) {
        return false;
    }
    if !input[..keyword.len()].eq_ignore_ascii_case(keyword) {
        return false;
    }
    match bytes.get(keyword.len()) {
        None => true,
        Some(byte) => !(byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$'),
    }
}

fn transfer_filter_starts_with_query(input: &str) -> bool {
    let mut rest = input.trim_start();
    // Skip leading SQL comments so `-- filter\nselect ...` still counts as a query.
    loop {
        if let Some(after) = rest.strip_prefix("--") {
            match after.find('\n') {
                Some(index) => rest = after[index + 1..].trim_start(),
                None => return false,
            }
            continue;
        }
        if let Some(after) = rest.strip_prefix('#') {
            match after.find('\n') {
                Some(index) => rest = after[index + 1..].trim_start(),
                None => return false,
            }
            continue;
        }
        if let Some(after) = rest.strip_prefix("/*") {
            match after.find("*/") {
                Some(index) => rest = after[index + 2..].trim_start(),
                None => return false,
            }
            continue;
        }
        break;
    }
    transfer_filter_keyword_at(rest, "select") || transfer_filter_keyword_at(rest, "with")
}

/// Parses one user-supplied filter. Empty input means "no filter" (full table).
pub fn parse_transfer_table_filter(raw: &str) -> Result<Option<TransferTableFilter>, String> {
    let trimmed = raw.trim().trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.contains(';') {
        return Err("Table filter must be a single statement; remove the extra ';' separators".to_string());
    }
    if transfer_filter_starts_with_query(trimmed) {
        return Ok(Some(TransferTableFilter::Query(trimmed.to_string())));
    }
    let predicate = crate::sql_dialect::normalize_where_input(Some(trimmed));
    let predicate = predicate.trim();
    if predicate.is_empty() {
        return Ok(None);
    }
    if predicate.contains(';') {
        return Err("Table filter must be a single statement; remove the extra ';' separators".to_string());
    }
    Ok(Some(TransferTableFilter::Predicate(predicate.to_string())))
}

fn transfer_table_filter_for(request: &TransferRequest, table: &str) -> Result<Option<TransferTableFilter>, String> {
    match request.table_filters.get(table) {
        Some(raw) => parse_transfer_table_filter(raw),
        None => Ok(None),
    }
}

fn transfer_filter_count_sql(
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
    filter: &TransferTableFilter,
) -> String {
    match filter {
        TransferTableFilter::Predicate(predicate) => {
            count_sql_with_where(table, schema, db_type, Some(predicate.as_str()), catalog)
        }
        TransferTableFilter::Query(query) => format!("SELECT COUNT(*) FROM ({query}) AS dbx_transfer_src"),
    }
}

/// Builds one paged source read for a filtered table.
///
/// MySQL/PostgreSQL share `LIMIT`/`OFFSET`, and the PK `ORDER BY` keeps OFFSET
/// paging deterministic. Keyset/ctid/COPY paging are disabled while a filter is
/// present because they build their own `WHERE`/`FROM` and would ignore it.
#[allow(clippy::too_many_arguments)]
fn transfer_filter_page_sql(
    columns: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
    filter: &TransferTableFilter,
    offset: u64,
    limit: usize,
    order_by_columns: &[String],
    empty_row_only: bool,
) -> String {
    let select_list = if empty_row_only {
        "1".to_string()
    } else {
        columns.iter().map(|column| quote_identifier(column, db_type)).collect::<Vec<_>>().join(", ")
    };
    let (from_sql, predicate) = match filter {
        TransferTableFilter::Predicate(predicate) => {
            (qualified_table(table, schema, db_type, catalog), Some(predicate.trim().to_string()))
        }
        TransferTableFilter::Query(query) => (format!("({query}) AS dbx_transfer_src"), None),
    };
    let where_clause = match predicate.as_deref() {
        Some(predicate) if !predicate.is_empty() => format!(" WHERE ({predicate})"),
        _ => String::new(),
    };
    let order_clause = if order_by_columns.is_empty() {
        String::new()
    } else {
        let columns =
            order_by_columns.iter().map(|column| quote_identifier(column, db_type)).collect::<Vec<_>>().join(", ");
        format!(" ORDER BY {columns}")
    };
    format!("SELECT {select_list} FROM {from_sql}{where_clause}{order_clause} LIMIT {limit} OFFSET {offset}")
}

pub fn keyset_pagination_sql(
    columns: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    primary_keys: &[String],
    last_pk_values: &[serde_json::Value],
    limit: usize,
) -> String {
    keyset_pagination_sql_with_identifier_quote(
        columns,
        table,
        schema,
        db_type,
        primary_keys,
        last_pk_values,
        limit,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn keyset_pagination_sql_with_identifier_quote(
    columns: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    primary_keys: &[String],
    last_pk_values: &[serde_json::Value],
    limit: usize,
    identifier_quote: Option<&str>,
) -> String {
    let full_table = qualified_table_with_identifier_quote(table, schema, db_type, None, identifier_quote);
    let col_list = columns
        .iter()
        .map(|c| quote_identifier_with_identifier_quote(c, db_type, identifier_quote))
        .collect::<Vec<_>>()
        .join(", ");
    let order = primary_keys
        .iter()
        .map(|pk| format!("{} ASC", quote_identifier_with_identifier_quote(pk, db_type, identifier_quote)))
        .collect::<Vec<_>>()
        .join(", ");

    let where_clause = keyset_where_clause(primary_keys, last_pk_values, db_type, identifier_quote);

    match db_type {
        _ => {
            format!("SELECT {col_list} FROM {full_table}{where_clause} ORDER BY {order} LIMIT {limit}")
        }
    }
}

fn keyset_where_clause(
    primary_keys: &[String],
    last_pk_values: &[serde_json::Value],
    db_type: &DatabaseType,
    identifier_quote: Option<&str>,
) -> String {
    if primary_keys.is_empty() || last_pk_values.is_empty() {
        return String::new();
    }

    let quoted_keys = primary_keys
        .iter()
        .map(|pk| quote_identifier_with_identifier_quote(pk, db_type, identifier_quote))
        .collect::<Vec<_>>();
    let literals = last_pk_values.iter().map(|v| value_to_sql_literal(v, db_type)).collect::<Vec<_>>();
    let comparison_count = quoted_keys.len().min(literals.len());
    if comparison_count == 0 {
        return String::new();
    }

    let mut clauses = Vec::with_capacity(comparison_count);
    for index in 0..comparison_count {
        let mut parts = Vec::with_capacity(index + 1);
        for prefix_index in 0..index {
            parts.push(format!("{} = {}", quoted_keys[prefix_index], literals[prefix_index]));
        }
        parts.push(format!("{} > {}", quoted_keys[index], literals[index]));
        if parts.len() == 1 {
            clauses.push(parts.remove(0));
        } else {
            clauses.push(format!("({})", parts.join(" AND ")));
        }
    }

    if clauses.len() == 1 {
        format!(" WHERE {}", clauses[0])
    } else {
        format!(" WHERE ({})", clauses.join(" OR "))
    }
}

fn value_to_sql_literal(value: &serde_json::Value, _db_type: &DatabaseType) -> String {
    match value {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => quote_string_literal(s),
        _ => quote_string_literal(&value.to_string()),
    }
}

/// Source dialects whose transfer read loop may page with a keyset cursor
/// (`WHERE (pk...) > <last page's keys>`) instead of `LIMIT n OFFSET m`.
/// Keyset paging renders the cursor values as SQL text literals, so a dialect
/// is only enabled once that rendering has been audited for it; the Postgres
/// family shares quoting and implicit-cast rules and is covered first. Other
/// dialects keep OFFSET paging (each page rescans and discards the rows before
/// it, which is quadratic in table size) until their literal rules are audited.
fn transfer_keyset_pagination_supported(db_type: &DatabaseType) -> bool {
    matches!(db_type, DatabaseType::Mysql)
}

/// Column types whose keyset cursor value round-trips through a SQL text
/// literal in a `>` comparison. Exotic types (arrays, interval, bytea, money,
/// network/range types...) keep OFFSET paging: their JSON form does not
/// reliably re-parse as the same value, and a failed cast aborting the
/// transfer mid-way is worse than a slow scan.
fn postgres_keyset_column_type_supported(data_type: &str) -> bool {
    false
}

/// Dialect-aware keyset column-type gate. The Postgres family is covered by
/// [`postgres_keyset_column_type_supported`]; MySQL-family, SQLite and SQL Server
/// primary-key types that round-trip through `value_to_sql_literal` are enabled
/// here. Binary types stay OFF — their JSON form is lossy — so those keys keep
/// OFFSET paging.
fn keyset_column_type_supported(db_type: &DatabaseType, data_type: &str) -> bool {
    match db_type {
        DatabaseType::Mysql => mysql_keyset_column_type_supported(data_type),

        _ => false,
    }
}

fn mysql_keyset_column_type_supported(data_type: &str) -> bool {
    let base = data_type.trim().to_ascii_lowercase();
    let base = base.split('(').next().unwrap_or("").trim();
    const SUPPORTED: &[&str] = &[
        "int",
        "integer",
        "tinyint",
        "smallint",
        "mediumint",
        "bigint",
        "char",
        "varchar",
        "date",
        "datetime",
        "timestamp",
        "year",
        "decimal",
        "numeric",
    ];
    SUPPORTED.iter().any(|prefix| base.starts_with(prefix))
}

/// Resolves the source primary key columns to their positions in the selected
/// column list. Returns None — meaning the read loop keeps OFFSET paging — when
/// the dialect is not keyset-capable, when a key column is not among the
/// transferred columns (its cursor value could not be read back), or when a key
/// column's type cannot round-trip through a text literal.
fn transfer_keyset_column_indexes(
    columns: &[db::ColumnInfo],
    primary_keys: &[String],
    db_type: &DatabaseType,
) -> Option<Vec<usize>> {
    if primary_keys.is_empty() || !transfer_keyset_pagination_supported(db_type) {
        return None;
    }
    primary_keys
        .iter()
        .map(|pk| {
            let index = columns.iter().position(|column| column.name == *pk)?;
            keyset_column_type_supported(db_type, &columns[index].data_type).then_some(index)
        })
        .collect()
}

/// Reads the keyset cursor (the primary key values ordering the pages) from the
/// last row of a page. A NULL component means the metadata overstated the key
/// (for example a nullable unique column reported as a key): the caller must
/// fall back to OFFSET paging, which stays consistent because it keeps ordering
/// by the same key columns.
fn keyset_cursor_from_last_row(
    rows: &[Vec<serde_json::Value>],
    key_indexes: &[usize],
) -> Option<Vec<serde_json::Value>> {
    let last = rows.last()?;
    key_indexes
        .iter()
        .map(|&index| {
            let value = last.get(index).cloned().unwrap_or(serde_json::Value::Null);
            (!value.is_null()).then_some(value)
        })
        .collect()
}

/// Outcome of advancing the keyset cursor from the page just read.
enum KeysetAdvance {
    /// The cursor moved to the page's last row; the next page continues after it.
    Advanced,
    /// A key component was NULL, so the key metadata does not allow keyset
    /// paging (for example a nullable unique column reported as a key). The
    /// caller falls back to OFFSET paging for the remaining pages, which stays
    /// consistent because it keeps ordering by the same key columns.
    FallBackToOffset,
}

/// Advances the keyset cursor from the page just read. Returns Err when the
/// cursor did not move, which would re-read the same page forever.
fn advance_keyset_cursor(
    cursor: &mut Vec<serde_json::Value>,
    rows: &[Vec<serde_json::Value>],
    key_indexes: &[usize],
    table: &str,
) -> Result<KeysetAdvance, String> {
    if rows.is_empty() {
        return Ok(KeysetAdvance::Advanced);
    }
    match keyset_cursor_from_last_row(rows, key_indexes) {
        Some(next) if next == *cursor => {
            Err(format!("Transfer stalled for table '{table}': keyset pagination did not advance past key {next:?}"))
        }
        Some(next) => {
            *cursor = next;
            Ok(KeysetAdvance::Advanced)
        }
        None => Ok(KeysetAdvance::FallBackToOffset),
    }
}

/// Whether a table copy may stream through the PostgreSQL COPY protocol
/// (`COPY ... TO STDOUT` on the source piped into `COPY ... FROM STDIN` on the
/// target) instead of the paged SELECT + multi-row INSERT loop.
///
/// Requirements:
/// - both endpoints speak a PostgreSQL-compatible dialect over the native
///   PostgreSQL pool (PostgreSQL, openGauss, KingbaseES);
/// - the effective write mode is append or overwrite — upsert needs
///   `ON CONFLICT`, which COPY cannot express;
/// - no column needs INSERT-only handling such as `OVERRIDING SYSTEM VALUE`
///   for GENERATED ALWAYS identity columns.
///
/// When any requirement fails — or when the COPY stream errors at runtime —
/// the transfer falls back to the existing paged INSERT path unchanged.
fn transfer_copy_fast_path_supported(
    pg_compat_transfer: bool,
    effective_mode: &TransferMode,
    overrides_postgres_system_values: bool,
) -> bool {
    pg_compat_transfer
        && matches!(effective_mode, TransferMode::Append | TransferMode::Overwrite)
        && !overrides_postgres_system_values
}

/// Counts records in a chunk of COPY text-format data. Field values escape a
/// literal newline as the two-byte sequence `\n`, so every raw `0x0A` byte is
/// a record separator.
fn count_copy_text_rows(chunk: &[u8]) -> u64 {
    chunk.iter().filter(|byte| **byte == b'\n').count() as u64
}

/// How often the COPY pipe polls the transfer cancellation flag between chunks.
const COPY_CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

pub async fn execute_on_pool(state: &AppState, pool_key: &str, sql: &str) -> Result<db::QueryResult, String> {
    execute_on_pool_with_options(state, pool_key, sql, None, TransferExecutionSafety::WriteNoReplay).await
}

pub async fn execute_read_on_pool(state: &AppState, pool_key: &str, sql: &str) -> Result<db::QueryResult, String> {
    execute_read_on_pool_with_max_rows(state, pool_key, sql, None).await
}

pub async fn execute_read_on_pool_with_max_rows(
    state: &AppState,
    pool_key: &str,
    sql: &str,
    max_rows: Option<usize>,
) -> Result<db::QueryResult, String> {
    execute_on_pool_with_options(state, pool_key, sql, max_rows, TransferExecutionSafety::ReadOnlyRetryable).await
}

async fn execute_transfer_ddl_on_pool(
    state: &AppState,
    pool_key: &str,
    sql: &str,
    db_type: &DatabaseType,
) -> Result<(), String> {
    for statement in transfer_ddl_statements(sql, db_type) {
        execute_on_pool(state, pool_key, &statement).await?;
    }
    Ok(())
}

async fn supported_mysql_transfer_collations(
    state: &AppState,
    pool_key: &str,
    names: &[String],
) -> Result<HashSet<String>, String> {
    let names = names.iter().map(|name| quote_string_literal(name)).collect::<Vec<_>>().join(", ");
    let sql = format!("SELECT COLLATION_NAME FROM information_schema.COLLATIONS WHERE COLLATION_NAME IN ({names})");
    let result = execute_on_pool(state, pool_key, &sql).await?;
    Ok(result.rows.iter().filter_map(|row| json_string_cell(row, 0)).map(|name| name.to_ascii_lowercase()).collect())
}

async fn execute_transfer_create_table_ddl_on_pool(
    state: &AppState,
    pool_key: &str,
    sql: &str,
    db_type: &DatabaseType,
    reused_source_ddl: bool,
) -> Result<(), String> {
    let original_error = match execute_transfer_ddl_on_pool(state, pool_key, sql, db_type).await {
        Ok(()) => return Ok(()),
        Err(error) => error,
    };
    let Some(collations) = mysql_collations_for_transfer_ddl_recovery(sql, &original_error, db_type, reused_source_ddl)
    else {
        return Err(original_error);
    };
    let supported = supported_mysql_transfer_collations(state, pool_key, &collations)
        .await
        .map_err(|error| format!("{original_error}; failed to inspect target MySQL collations: {error}"))?;
    let rewritten = remove_unsupported_mysql_collations(sql, &supported);
    if rewritten == sql {
        return Err(format!("{original_error}; target MySQL reports all referenced collations as supported"));
    }

    let unsupported =
        collations.iter().filter(|name| !supported.contains(&name.to_ascii_lowercase())).cloned().collect::<Vec<_>>();
    log::warn!("[transfer] retrying target table DDL without unsupported MySQL collations: {}", unsupported.join(", "));
    execute_transfer_ddl_on_pool(state, pool_key, &rewritten, db_type)
        .await
        .map_err(|error| format!("{original_error}; retry without unsupported MySQL collations failed: {error}"))
}

fn transfer_table_already_exists_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("already exists")
        || lower.contains("there is already")
        || lower.contains("duplicate_table")
        || lower.contains("42p07")
        || error.contains("已经存在")
        || error.contains("已存在")
}

fn transfer_create_table_created(result: Result<(), String>, error_prefix: &str) -> Result<bool, String> {
    match result {
        Ok(_) => Ok(true),
        Err(e) if transfer_table_already_exists_error(&e) => Ok(false),
        Err(e) => Err(format!("{error_prefix}: {e}")),
    }
}

fn transfer_ddl_statements(sql: &str, db_type: &DatabaseType) -> Vec<String> {
    {
        vec![sql.to_string()]
    }
}

/// Strips inline `CONSTRAINT ... FOREIGN KEY ... REFERENCES ...` lines from a
/// `CREATE TABLE` statement, fixing up a trailing comma only when removing the
/// foreign key leaves one immediately before the table's closing parenthesis.
/// Dialect-agnostic: relies only on the definition-line shape shared by Postgres
/// and MySQL-family DDL dumps.
///
/// Only genuine constraint definition lines match (`CONSTRAINT <name> FOREIGN
/// KEY (` / bare `FOREIGN KEY (`); a column line whose COMMENT or DEFAULT text
/// merely mentions the words must survive (#7660).
fn strip_inline_foreign_key_constraint_lines(statement: &str) -> String {
    strip_inline_foreign_key_constraint_lines_collecting(statement).0
}

/// Same as [`strip_inline_foreign_key_constraint_lines`], but also returns every
/// removed `CONSTRAINT ... FOREIGN KEY ...` clause (whitespace-trimmed, trailing
/// comma dropped) so callers can re-create each constraint later — e.g. database
/// export defers PostgreSQL foreign keys to the end of the script, where restore
/// order no longer has to satisfy foreign key dependencies (issue #10575).
pub(crate) fn strip_inline_foreign_key_constraint_lines_collecting(statement: &str) -> (String, Vec<String>) {
    if !statement.trim_start().to_ascii_uppercase().starts_with("CREATE TABLE ") {
        return (statement.to_string(), Vec::new());
    }

    let mut lines: Vec<String> = Vec::new();
    let mut removed_foreign_keys: Vec<String> = Vec::new();
    for line in statement.lines() {
        if INLINE_FOREIGN_KEY_CONSTRAINT_LINE_RE.is_match(line) {
            removed_foreign_keys.push(line.trim().trim_end_matches(',').trim_end().to_string());
            continue;
        }
        lines.push(line.to_string());
    }

    if !removed_foreign_keys.is_empty() {
        if let Some(closing_index) = lines.iter().rposition(|line| line.trim_start().starts_with(')')) {
            if let Some(previous) = lines[..closing_index].iter_mut().rfind(|line| !line.trim().is_empty()) {
                let trimmed_len = previous.trim_end_matches(char::is_whitespace).len();
                if previous[..trimmed_len].ends_with(',') {
                    previous.remove(trimmed_len - 1);
                }
            }
        }
    }

    (lines.join("\n"), removed_foreign_keys)
}

pub async fn execute_on_pool_with_max_rows(
    state: &AppState,
    pool_key: &str,
    sql: &str,
    max_rows: Option<usize>,
) -> Result<db::QueryResult, String> {
    execute_on_pool_with_options(state, pool_key, sql, max_rows, TransferExecutionSafety::WriteNoReplay).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TransferExecutionSafety {
    ReadOnlyRetryable,
    WriteNoReplay,
}

fn transfer_pool_error_action(
    safety: TransferExecutionSafety,
    db_type: Option<DatabaseType>,
    err: &str,
) -> PoolErrorAction {
    match (safety, pool_error_action(db_type, err)) {
        (TransferExecutionSafety::WriteNoReplay, PoolErrorAction::ReconnectAndRetry) => PoolErrorAction::Discard,
        (_, action) => action,
    }
}

/// A structured Agent quarantine makes the current logical session unusable, but
/// does not authorize replaying the operation whose outcome is unknown. Prepare a
/// replacement only for later transfer work. Keep legacy/untyped failures and
/// runtime replacement on their existing, more conservative paths.
fn should_prepare_fresh_agent_transfer_session(db_type: Option<DatabaseType>, error: &str) -> bool {
    {
        return false;
    }
}

/// Whether a discarded transfer pool has to be replaced before the next statement runs.
///
/// Every error that reaches the `Discard` arm tears the pool down: the failed statement
/// is never replayed (its outcome on the server is unknown), and a driver whose query
/// timed out may still own a checked-out connection. The pool key, though, is shared by
/// every table of the transfer, so leaving it missing turns one transient failure into a
/// *misleading* "Connection not found" on the next table instead of that table running
/// (or failing) on its own. Prepare a replacement for native drivers; agent/JDBC
/// sessions keep their stricter rule, where recovery stays gated on the structured
/// quarantine decision.
fn should_reconnect_discarded_transfer_pool(db_type: Option<DatabaseType>) -> bool {
    db_type.is_some_and(|db_type| !false)
}

/// Resolve the pool a transfer statement runs on, re-creating it when it is gone.
///
/// A bulk transfer lives for tens of minutes and shares one pool key across every table,
/// so a pool that disappears between two statements used to abort the run with a
/// misleading `Connection not found` that says nothing about the database being
/// unreachable. The pool can legitimately be gone: `execute_on_pool_once` drops the pool
/// of a driver whose query timed out, the connection keepalive tears down a pool whose
/// ping failed or timed out, and every other transfer statement reuses the same key.
/// Rebuilding it here is safe -- the statement has not run yet, so nothing is replayed --
/// and keeps the transfer working on a fresh connection instead of failing the table.
async fn ensure_transfer_statement_pool(state: &AppState, pool_key: &str) -> Result<PoolKind, String> {
    if let Some(pool) = state.pool_handle(pool_key).await {
        return Ok(pool);
    }
    let (connection_id, database, _, _) = transfer_pool_context(state, pool_key).await;
    let Some(connection_id) = connection_id else {
        return Err("Connection not found".to_string());
    };
    let catalog = catalog_from_pool_key(pool_key).map(str::to_string);
    let client_session_id = client_session_id_from_pool_key(pool_key).map(str::to_string);
    state
        .get_or_create_pool_for_session_with_catalog(
            &connection_id,
            database.as_deref(),
            catalog.as_deref(),
            client_session_id.as_deref(),
        )
        .await
        .map_err(|error| format!("Connection not found: {error}"))?;
    state.pool_handle(pool_key).await.ok_or_else(|| "Connection not found".to_string())
}

async fn transfer_pool_context(
    state: &AppState,
    pool_key: &str,
) -> (Option<String>, Option<String>, Option<DatabaseType>, Option<u64>) {
    let configs = state.configs.read().await;
    let config = config_for_pool_key(pool_key, &configs);
    (
        config.map(|config| config.id.clone()),
        database_from_pool_key(pool_key).map(str::to_string),
        config.map(|config| config.db_type),
        config.map(|config| config.effective_query_timeout_secs()),
    )
}

fn client_session_id_from_pool_key(pool_key: &str) -> Option<&str> {
    pool_key.split_once(":session:").map(|(_, session)| session).filter(|session| !session.is_empty())
}

fn is_transfer_query_timeout(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    is_dbx_query_timeout_error(&lower) || lower.contains("查询超时") || lower.contains("查詢逾時")
}

async fn execute_on_pool_with_options(
    state: &AppState,
    pool_key: &str,
    sql: &str,
    max_rows: Option<usize>,
    safety: TransferExecutionSafety,
) -> Result<db::QueryResult, String> {
    let (connection_id, database, db_type, _query_timeout_secs) = transfer_pool_context(state, pool_key).await;
    let client_session_id = client_session_id_from_pool_key(pool_key).map(str::to_string);
    let mut current_pool_key = pool_key.to_string();

    for attempt in 0..2 {
        let result = execute_on_pool_once(state, &current_pool_key, sql, max_rows).await;
        let Some(error) = result.as_ref().err() else {
            return result;
        };

        match transfer_pool_error_action(safety, db_type, error) {
            PoolErrorAction::Keep => return result,
            PoolErrorAction::Discard => {
                state.remove_pool_by_key(&current_pool_key).await;
                // `WriteNoReplay` deliberately downgrades a recoverable connection
                // error to `Discard` here so this specific write is never replayed
                // (its outcome on the server is unknown). But leaving the pool
                // torn down does not just affect this one statement: a bulk
                // multi-table transfer keeps reusing this same `pool_key` for
                // every remaining table, so once one table hits a transient
                // connection drop (a momentary "too many connections"/"connection
                // closed" from the target server under load), every later table
                // immediately fails too with "Connection not found" / "Pool not
                // found" -- turning one blip into a cascade for the rest of the
                // run. Re-establish a fresh pool under the same key so the next
                // caller isn't handed a known-dead connection; this never retries
                // the failed statement above, only prepares the pool for
                // whichever statement runs next.
                let prepare_agent_session = should_prepare_fresh_agent_transfer_session(db_type, error);
                let reconnect_pool = should_reconnect_discarded_transfer_pool(db_type);
                if reconnect_pool || prepare_agent_session {
                    if let Some(connection_id) = connection_id.as_deref() {
                        let catalog = catalog_from_pool_key(&current_pool_key).map(str::to_string);
                        let recovery = if prepare_agent_session {
                            state
                                .get_or_create_pool_for_session_with_catalog(
                                    connection_id,
                                    database.as_deref(),
                                    catalog.as_deref(),
                                    client_session_id.as_deref(),
                                )
                                .await
                        } else {
                            state
                                .reconnect_pool_for_session_with_catalog(
                                    connection_id,
                                    database.as_deref(),
                                    catalog.as_deref(),
                                    client_session_id.as_deref(),
                                )
                                .await
                        };
                        if let Err(reconnect_error) = recovery {
                            // Preserve the failed operation's original error and unknown
                            // outcome. The replacement is only for later transfer work.
                            log::warn!(
                                "[transfer] failed to prepare a fresh pool after discarding '{current_pool_key}': {reconnect_error}"
                            );
                        }
                    }
                }
                return result;
            }
            PoolErrorAction::ReconnectAndRetry if attempt == 0 => {
                let Some(connection_id) = connection_id.as_deref() else {
                    state.remove_pool_by_key(&current_pool_key).await;
                    return result;
                };
                let catalog = catalog_from_pool_key(&current_pool_key).map(str::to_string);
                current_pool_key = state
                    .reconnect_pool_for_session_with_catalog(
                        connection_id,
                        database.as_deref(),
                        catalog.as_deref(),
                        client_session_id.as_deref(),
                    )
                    .await?;
            }
            PoolErrorAction::ReconnectAndRetry => {
                state.remove_pool_by_key(&current_pool_key).await;
                return result;
            }
        }
    }

    unreachable!("transfer pool execution retry loop runs at most twice")
}

async fn execute_on_pool_once(
    state: &AppState,
    pool_key: &str,
    sql: &str,
    max_rows: Option<usize>,
) -> Result<db::QueryResult, String> {
    let (_connection_id, _database, db_type, query_timeout_secs) = transfer_pool_context(state, pool_key).await;
    let query_timeout = query_timeout_duration(query_timeout_secs);

    // Read-only check: block transfer operations in readonly mode.
    crate::query::check_read_only_for_connection(state, pool_key, sql).await?;
    let pool = ensure_transfer_statement_pool(state, pool_key).await?;

    // Transfer reads run under the per-connection operation budget. Drivers that
    // expose an incremental result stream (MySQL, PostgreSQL, SQLite, SQL Server)
    // use a *progress-aware* budget: the configured query timeout is an inactivity
    // window reset for every row the server delivers, so transferring a large
    // table is no longer cancelled just for exceeding the timeout in total. Drivers
    // whose protocol returns the whole result in one shot — ClickHouse, InfluxDB,
    // the Agent/JDBC path, external drivers and the DuckDB sidecar worker — expose
    // no incremental progress, so they keep the plain wall-clock timeout.
    let result = match &pool {
        PoolKind::Mysql(p, mode) => {
            let p = p.clone();
            let bare = false;
            // Row-returning reads run under a progress-aware budget: the timeout
            // resets for every row the server delivers, so a large table is no
            // longer cancelled just for taking longer than the timeout overall.
            let progress_clock = Arc::new(StreamProgressClock::new());
            db::mysql::execute_query_with_max_rows_progress(
                &p,
                sql,
                false,
                max_rows,
                Default::default(),
                progress_clock,
                query_timeout,
            )
            .await
        }

        _ => Err("Unsupported database type for transfer".to_string()),
    };
    drop(pool);
    if result.as_ref().is_err_and(|error| is_transfer_query_timeout(error))
        && should_discard_pool_after_query_timeout(db_type)
    {
        // A timed-out native driver future may still own a checked-out
        // connection. Discard the pool so a late server response cannot be
        // reused by the next transfer statement. Drivers that keep their pool on
        // a timeout (`pool_error_action` -> `Keep`, e.g. the embedded SQLite
        // worker) must not lose it here either: the pool key is shared by every
        // table in the transfer, so dropping it would make all later tables fail
        // with "Connection not found".
        state.remove_pool_by_key(pool_key).await;
    }
    result
}

fn database_from_pool_key(pool_key: &str) -> Option<&str> {
    let base = pool_key.split_once(":session:").map(|(base, _)| base).unwrap_or(pool_key);
    let base = base.split_once(":catalog:").map(|(base, _)| base).unwrap_or(base);
    base.split_once(':').map(|(_, database)| database).filter(|database| !database.is_empty())
}

fn catalog_from_pool_key(pool_key: &str) -> Option<&str> {
    let base = pool_key.split_once(":session:").map(|(base, _)| base).unwrap_or(pool_key);
    base.split_once(":catalog:").map(|(_, catalog)| catalog).filter(|catalog| !catalog.is_empty())
}

pub async fn get_db_type(state: &AppState, connection_id: &str) -> Result<DatabaseType, String> {
    let configs = state.configs.read().await;
    configs
        .get(connection_id)
        .map(effective_transfer_database_type)
        .ok_or_else(|| format!("Connection config not found: {connection_id}"))
}

fn effective_transfer_database_type(config: &ConnectionConfig) -> DatabaseType {
    {
        return config.db_type;
    }
}

pub async fn get_columns_for_transfer(
    state: &AppState,
    pool_key: &str,
    _connection_id: &str,
    database: &str,
    schema: &str,
    table: &str,
    catalog: Option<&str>,
) -> Result<Vec<db::ColumnInfo>, String> {
    let pool_handle = state.pool_handle(pool_key).await;

    {}
    {}
    {}
    {}
    {}
    {}
    let pool = pool_handle.as_ref().ok_or("Pool not found")?;
    let schema = schema.to_string();
    let table = table.to_string();
    match pool {
        PoolKind::Mysql(p, _) => {
            let p = p.clone();
            let catalog = normalize_external_catalog_name(catalog).map(str::to_string);
            {
                db::mysql::get_columns(&p, &schema, &table).await
            }
        }

        _ => Err("Unsupported database type".to_string()),
    }
}

pub fn ordered_transfer_object_kinds(kinds: Vec<TransferObjectKind>) -> Vec<TransferObjectKind> {
    let rank = |kind: &TransferObjectKind| match kind {
        TransferObjectKind::Table => 0,
        TransferObjectKind::Sequence => 1,
        TransferObjectKind::View => 2,
        TransferObjectKind::MaterializedView => 2,
        TransferObjectKind::Function => 3,
        TransferObjectKind::Procedure => 4,
        TransferObjectKind::Trigger => 5,
        TransferObjectKind::Event => 6,
    };
    let mut kinds = kinds;
    kinds.sort_by_key(rank);
    kinds
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferObjectOutcome {
    pub transferred: Vec<String>,
    pub skipped: Vec<String>,
    pub failed: Vec<String>,
}

pub fn selected_object_names(selections: &[TransferObjectSelection], kind: &TransferObjectKind) -> Vec<String> {
    selections.iter().filter(|s| &s.object_type == kind).flat_map(|s| s.names.clone()).collect::<Vec<_>>()
}

/// Whether a kind participates in a transfer. `None` is the legacy PG→PG fallback;
/// `Some([])` is an explicit empty selection and selects no kinds.
pub fn object_kind_selected_or_defaulted(
    selections: Option<&[TransferObjectSelection]>,
    kind: &TransferObjectKind,
) -> bool {
    selections.is_none_or(|selections| !selected_object_names(selections, kind).is_empty())
}

pub fn should_copy_data(content: &TransferContent) -> bool {
    !matches!(content, TransferContent::StructureOnly)
}

/// Whether non-table schema-object transfer should run for a request.
/// Data-only transfers never include schema objects. In structure modes,
/// newer clients send an explicit `objects` selection; for those the answer
/// is simply whether any object was selected. PG→PG keeps the legacy default:
/// even an *empty* selection still transfers all views, functions, triggers,
/// policies, ownership and grants (the old table-selection flow always did).
pub fn should_transfer_schema_objects(
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    request: &TransferRequest,
) -> bool {
    if matches!(request.content, TransferContent::DataOnly) {
        return false;
    }
    match request.object_selection_mode() {
        TransferObjectSelectionMode::Explicit(selections) => {
            // A table-only selection is already handled by the table transfer pass.
            // Explicit empty selections must never enter a legacy all-objects path.
            selections
                .iter()
                .any(|selection| selection.object_type != TransferObjectKind::Table && !selection.names.is_empty())
        }
        TransferObjectSelectionMode::LegacyUnspecified => {
            {}
            false
        }
    }
}

/// Transfers selected non-table objects from source to target.
/// Skips objects that already exist on the target; counts them in the
/// outcome. Executes in dependency order (sequence → view → function →
/// procedure → trigger → event). Errors are collected per object and the
/// transfer continues.
pub async fn transfer_schema_objects<F>(
    state: &AppState,
    request: &TransferRequest,
    source_pool_key: &str,
    target_pool_key: &str,
    progress_callback: F,
) -> Result<TransferObjectOutcome, String>
where
    F: FnMut(TransferProgress),
{
    if matches!(request.content, TransferContent::DataOnly) {
        return Ok(TransferObjectOutcome::default());
    }
    let source_db_type = get_db_type(state, &request.source_connection_id).await?;
    let target_db_type = get_db_type(state, &request.target_connection_id).await?;
    if !should_transfer_schema_objects(&source_db_type, &target_db_type, request) {
        return Ok(TransferObjectOutcome::default());
    }
    if !is_same_transfer_family(&source_db_type, &target_db_type) {
        return transfer_cross_family_schema_objects(
            state,
            request,
            source_pool_key,
            target_pool_key,
            progress_callback,
        )
        .await;
    }
    // Same-family path: drop selections whose object type is not transferable
    // for the source family (defense in depth — the UI already filters disabled
    // types at the request boundary, but requests can also arrive from older
    // clients or be crafted directly).
    let mut filtered_request = request.clone();
    if let Some(family) = transfer_object_family(&source_db_type) {
        let supported = transfer_object_kinds_for_family(&family);
        filtered_request.objects = request.object_selection_mode().filter_supported(&supported);
    }
    match transfer_object_family(&source_db_type) {
        Some(TransferObjectFamily::Mysql) => {
            transfer_mysql_schema_objects(state, &filtered_request, source_pool_key, target_pool_key, progress_callback)
                .await
        }

        None => Ok(TransferObjectOutcome::default()),
    }
}

async fn transfer_mysql_schema_objects<F>(
    state: &AppState,
    request: &TransferRequest,
    source_pool_key: &str,
    target_pool_key: &str,
    mut progress_callback: F,
) -> Result<TransferObjectOutcome, String>
where
    F: FnMut(TransferProgress),
{
    let mut outcome = TransferObjectOutcome::default();
    let source_db = &request.source_database;
    let target_db =
        if request.target_database.trim().is_empty() { source_db.as_str() } else { request.target_database.as_str() };
    let order = ordered_transfer_object_kinds(
        request.object_selection_mode().selections().iter().map(|s| s.object_type).collect(),
    );
    for kind in order {
        for name in selected_object_names(request.object_selection_mode().selections(), &kind) {
            if is_cancelled(&request.transfer_id).await {
                return Err("Cancelled".to_string());
            }
            let table = format!("schema object: {name}");
            let mut progress = |outcome: &mut TransferObjectOutcome, status: TransferStatus, error: Option<String>| {
                progress_callback(TransferProgress {
                    transfer_id: request.transfer_id.clone(),
                    table: table.clone(),
                    table_index: request.tables.len(),
                    total_tables: request.tables.len(),
                    rows_transferred: (outcome.transferred.len() + outcome.skipped.len()) as u64,
                    total_rows: None,
                    status,
                    error,
                    terminal: false,
                });
            };
            // skip if the target already has it
            let exists_sql = target_object_exists_sql(&DatabaseType::Mysql, target_db, &name, &kind)?;
            let exists = !execute_on_pool(state, target_pool_key, &exists_sql).await?.rows.is_empty();
            if exists {
                outcome.skipped.push(format!("{kind:?}:{name}"));
                progress(&mut outcome, TransferStatus::Running, None);
                continue;
            }
            let query = mysql_object_source_query(&kind, source_db, &name)?;
            let result = execute_on_pool(state, source_pool_key, &query).await?;
            let raw_ddl = mysql_object_ddl_from_result(&kind, source_db, &result.rows)?;
            let ddl = strip_mysql_definer(&raw_ddl);
            let ddl = rewrite_mysql_schema_qualifier(&ddl, source_db, target_db);
            match execute_on_pool(state, target_pool_key, &ddl).await {
                Ok(_) => {
                    outcome.transferred.push(format!("{kind:?}:{name}"));
                    progress(&mut outcome, TransferStatus::Running, None);
                }
                Err(e) => {
                    outcome.failed.push(format!("{kind:?}:{name}"));
                    progress(&mut outcome, TransferStatus::Error, Some(e));
                }
            }
        }
    }
    Ok(outcome)
}

/// Transfers non-table objects across different database families.
/// Only mechanically rewriteable kinds (views, sequences) are allowed;
/// anything else is rejected up front with a descriptive error.
async fn transfer_cross_family_schema_objects<F>(
    state: &AppState,
    request: &TransferRequest,
    source_pool_key: &str,
    target_pool_key: &str,
    mut progress_callback: F,
) -> Result<TransferObjectOutcome, String>
where
    F: FnMut(TransferProgress),
{
    let mut outcome = TransferObjectOutcome::default();
    let source_db_type = get_db_type(state, &request.source_connection_id).await?;
    let target_db_type = get_db_type(state, &request.target_connection_id).await?;
    let allowed = cross_family_transferable_object_kinds(&source_db_type, &target_db_type);
    let unsupported: Vec<String> = request
        .object_selection_mode()
        .selections()
        .iter()
        .filter(|selection| !allowed.contains(&selection.object_type))
        .map(|selection| format!("{:?}", selection.object_type))
        .collect();
    if !unsupported.is_empty() {
        return Err(format!("跨库非表对象传输暂不支持该类型，不支持: {}", unsupported.join(", ")));
    }
    let source_family = transfer_object_family(&source_db_type).ok_or("unsupported source family")?;
    let target_family = transfer_object_family(&target_db_type).ok_or("unsupported target family")?;
    let resolve_schema = |schema: &str, database: &str, db_type: &DatabaseType| -> String {
        if !schema.trim().is_empty() {
            return schema.to_string();
        }
        match transfer_object_family(db_type) {
            _ => database.to_string(),
        }
    };
    let source_schema = resolve_schema(&request.source_schema, &request.source_database, &source_db_type);
    let target_schema = resolve_schema(&request.target_schema, &request.target_database, &target_db_type);
    let order = ordered_transfer_object_kinds(
        request.object_selection_mode().selections().iter().map(|s| s.object_type).collect(),
    );
    for kind in order {
        for name in selected_object_names(request.object_selection_mode().selections(), &kind) {
            if is_cancelled(&request.transfer_id).await {
                return Err("Cancelled".to_string());
            }
            let table = format!("schema object: {name}");
            let mut progress = |outcome: &mut TransferObjectOutcome, status: TransferStatus, error: Option<String>| {
                progress_callback(TransferProgress {
                    transfer_id: request.transfer_id.clone(),
                    table: table.clone(),
                    table_index: request.tables.len(),
                    total_tables: request.tables.len(),
                    rows_transferred: (outcome.transferred.len() + outcome.skipped.len()) as u64,
                    total_rows: None,
                    status,
                    error,
                    terminal: false,
                });
            };
            let exists_sql = target_object_exists_sql(&target_db_type, &target_schema, &name, &kind)?;
            let exists = !execute_on_pool(state, target_pool_key, &exists_sql).await?.rows.is_empty();
            if exists {
                outcome.skipped.push(format!("{kind:?}:{name}"));
                progress(&mut outcome, TransferStatus::Running, None);
                continue;
            }
            let query = match source_family {
                TransferObjectFamily::Mysql => mysql_object_source_query(&kind, &source_schema, &name)?,
            };
            let result = execute_on_pool(state, source_pool_key, &query).await?;
            let raw_ddl = match source_family {
                TransferObjectFamily::Mysql => mysql_object_ddl_from_result(&kind, &source_schema, &result.rows)?,
            };
            let ddl = convert_cross_family_object_ddl(
                &source_family,
                &target_family,
                &kind,
                &source_schema,
                &target_schema,
                &raw_ddl,
            );
            match execute_on_pool(state, target_pool_key, &ddl).await {
                Ok(_) => {
                    outcome.transferred.push(format!("{kind:?}:{name}"));
                    progress(&mut outcome, TransferStatus::Running, None);
                }
                Err(e) => {
                    let e = if kind == TransferObjectKind::View
                        && (e.contains("无效的表或视图名") || e.contains("table or view does not exist"))
                    {
                        format!("{e}（视图引用的基表可能未在目标库中，请同时选择视图依赖的表或先传输这些表）")
                    } else {
                        e
                    };
                    outcome.failed.push(format!("{kind:?}:{name}"));
                    progress(&mut outcome, TransferStatus::Error, Some(e));
                }
            }
        }
    }
    Ok(outcome)
}

/// Build the SQL plan preview for a `drop_target_before_create` transfer.
///
/// Pure planning and deliberately lightweight: it resolves target names, derives backup
/// names, and renders the rename/drop statements — but it does *not* read per-table column
/// metadata or prepare the full CREATE DDL, so a large selection previews quickly and the
/// confirmation dialog stays readable. The destructive steps (rename aside, drop the backup
/// after success) are the ones shown; the CREATE step is summarized rather than expanded.
async fn build_rebuild_preview(
    state: &Arc<AppState>,
    request: &TransferRequest,
    target_db_type: &DatabaseType,
    target_pool_key: &str,
) -> Result<TransferRebuildPreview, String> {
    // Resolve target names first, exactly like the rename pre-pass, so the preview cannot
    // disagree with execution about which table exists or what it will be renamed to.
    let mut resolved: Vec<(String, String, bool)> = Vec::with_capacity(request.tables.len());
    for table in &request.tables {
        let ResolvedTransferTargetTable { name, preexisting } = resolve_transfer_target_table_name(
            state,
            request,
            table,
            target_pool_key,
            target_db_type,
            request.source_catalog.as_deref(),
            request.target_catalog.as_deref(),
        )
        .await;
        resolved.push((table.clone(), name, preexisting));
    }

    // Refuse the same dependencies the rename pre-pass refuses, before showing any plan.
    let target_names = resolved.iter().map(|(_, name, _)| name.clone()).collect::<Vec<_>>();
    crate::transfer_rebuild::ensure_no_external_table_dependencies(
        state,
        target_pool_key,
        &request.target_database,
        &request.target_schema,
        &target_names,
        *target_db_type,
    )
    .await?;

    let mut tables = Vec::with_capacity(resolved.len());
    let mut rename_statements: Vec<String> = Vec::new();
    let mut drop_statements: Vec<String> = Vec::new();

    for (table, target_table, preexisting) in &resolved {
        let backup_table = if *preexisting {
            let backup = crate::transfer_rebuild::backup_table_name(
                *target_db_type,
                &request.transfer_id,
                &format!("{}.{}", request.source_schema, table),
                target_table,
            )?;
            let rename_sql =
                crate::db_admin_sql::build_rename_object_sql(crate::db_admin_sql::RenameObjectSqlOptions {
                    database_type: Some(*target_db_type),
                    object_type: crate::db_admin_sql::DatabaseObjectType::Table,
                    schema: if request.target_schema.is_empty() { None } else { Some(request.target_schema.clone()) },
                    old_name: target_table.clone(),
                    new_name: backup.clone(),
                })?;
            rename_statements.push(rename_sql);
            let drop_sql = crate::db_admin_sql::build_drop_table_sql(crate::db_admin_sql::TableAdminSqlOptions {
                database_type: Some(*target_db_type),
                schema: if request.target_schema.is_empty() { None } else { Some(request.target_schema.clone()) },
                table_name: backup.clone(),
                cascade: Some(true),
                identifier_quote: None,
            });
            drop_statements.push(drop_sql);
            Some(backup)
        } else {
            None
        };

        tables.push(TransferRebuildPreviewTable {
            source_table: table.clone(),
            target_table: target_table.clone(),
            backup_table,
        });
    }

    let mut phases: Vec<String> = Vec::new();
    let backup_sql = if rename_statements.is_empty() {
        None
    } else {
        Some(format!("-- 1. Backup existing target tables\n{}", rename_statements.join(";\n")))
    };
    let cleanup_sql = if drop_statements.is_empty() {
        None
    } else {
        Some(format!("-- 3. Drop backups after success\n{}", drop_statements.join(";\n")))
    };
    if let Some(sql) = &backup_sql {
        phases.push(sql.clone());
    }
    phases.push(format!(
        "-- 2. Recreate the {} selected table(s) from the source structure and transfer the selected data",
        resolved.len()
    ));
    if let Some(sql) = &cleanup_sql {
        phases.push(sql.clone());
    }

    Ok(TransferRebuildPreview { sql: phases.join("\n\n"), tables, backup_sql, cleanup_sql })
}

pub async fn preview_transfer_ownership(
    state: &Arc<AppState>,
    request: &TransferRequest,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    source_pool_key: &str,
    target_pool_key: &str,
) -> Result<TransferOwnershipPreview, String> {
    // PostgreSQL-compatible transfers report role ownership gaps for the confirmation flow.
    let (missing_owners, target_owner) = { (Vec::new(), String::new()) };

    // Rebuild transfers additionally expose the rename/create/cleanup SQL plan so the
    // confirmation dialog shows the real statements it is about to run.
    let rebuild = if request.drop_target_before_create {
        Some(build_rebuild_preview(state, request, target_db_type, target_pool_key).await?)
    } else {
        None
    };

    // Structure-only transfers expose the statements their create pass will run. The other
    // content modes keep their existing previews: data-only runs no DDL, and
    // structure-and-data is deliberately unchanged here.
    let structure = if matches!(request.content, TransferContent::StructureOnly) {
        Some(
            structure_plan::build_structure_preview(
                state,
                request,
                source_db_type,
                target_db_type,
                source_pool_key,
                target_pool_key,
            )
            .await?,
        )
    } else {
        None
    };

    Ok(TransferOwnershipPreview { missing_owners, target_owner, rebuild, structure })
}

pub async fn is_cancelled(transfer_id: &str) -> bool {
    CANCELLED.read().await.contains(transfer_id)
}

pub async fn set_cancelled(transfer_id: &str) {
    CANCELLED.write().await.insert(transfer_id.to_string());
}

pub async fn clear_cancelled(transfer_id: &str) {
    CANCELLED.write().await.remove(transfer_id);
}

/// Fetches full foreign key metadata for each of `tables`, one
/// `list_foreign_keys_core` call per table. Always inserts an entry per input
/// table (even when it has zero foreign keys), so callers can use
/// `HashMap::get` to distinguish "checked, no FKs" from "not fetched" — the
/// latter tells `transfer_table` it needs to fall back to a live query.
async fn fetch_foreign_keys_for_tables(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    tables: &[String],
) -> Result<HashMap<String, Vec<db::ForeignKeyInfo>>, String> {
    let mut result = HashMap::new();
    for table in tables {
        let fks = crate::schema::list_foreign_keys_core(state, connection_id, database, schema, table).await?;
        result.insert(table.clone(), fks);
    }
    Ok(result)
}

/// Sort table names by foreign key dependency, also returning the full foreign
/// key metadata fetched along the way (keyed by table name) so callers doing a
/// data transfer don't have to re-query the same metadata per table later.
///
/// When `parents_first` is true (data transfer / SQL export), referenced (parent)
/// tables come before referencing (child) tables so inserts don't violate FK
/// constraints.
///
/// When `parents_first` is false (batch drop), referencing (child) tables come
/// first so they are dropped before the tables they reference.
///
/// Uses Kahn's algorithm for topological sort; tables involved in cycles keep
/// their original relative order after all cycle-free tables.
///
/// The returned map is empty when `tables.len() <= 1` (no fetch needed to sort)
/// or when `connection_id` is a native Postgres connection (dependencies there
/// come from a single batched `list_table_dependencies` query that doesn't
/// build per-table `ForeignKeyInfo` — Postgres transfers don't consult this map).
pub async fn sort_tables_by_fk_dependency_with_foreign_keys(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    tables: &[String],
    parents_first: bool,
) -> Result<(Vec<String>, HashMap<String, Vec<db::ForeignKeyInfo>>), String> {
    if tables.len() <= 1 {
        return Ok((tables.to_vec(), HashMap::new()));
    }

    let db_type = state
        .configs
        .read()
        .await
        .get(connection_id)
        .map(|config| config.db_type)
        .ok_or_else(|| format!("Connection config not found: {connection_id}"))?;

    let (dependencies, foreign_keys_by_table) = {
        let foreign_keys_by_table =
            fetch_foreign_keys_for_tables(state, connection_id, database, schema, tables).await?;
        let dependencies = foreign_keys_by_table
            .iter()
            .flat_map(|(table, fks)| fks.iter().map(move |fk| (table.clone(), fk.ref_table.clone())))
            .collect::<Vec<_>>();
        (dependencies, foreign_keys_by_table)
    };

    Ok((sort_table_names_by_dependencies(tables, &dependencies, parents_first), foreign_keys_by_table))
}

/// Sort table names by foreign key dependency. See
/// `sort_tables_by_fk_dependency_with_foreign_keys` for the full behavior —
/// this is a thin wrapper that discards the fetched foreign key metadata, kept
/// for callers that only need table order (batch drop, database export).
pub async fn sort_tables_by_fk_dependency(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: &str,
    tables: &[String],
    parents_first: bool,
) -> Result<Vec<String>, String> {
    sort_tables_by_fk_dependency_with_foreign_keys(state, connection_id, database, schema, tables, parents_first)
        .await
        .map(|(sorted, _)| sorted)
}

pub(crate) fn sort_table_names_by_dependencies(
    tables: &[String],
    dependencies: &[(String, String)],
    parents_first: bool,
) -> Vec<String> {
    let table_set: HashSet<&str> = tables.iter().map(|table| table.as_str()).collect();

    // Build in-degree and dependents graph.
    // parents_first=true:  edge ref_table → table     (parent before child)
    // parents_first=false: edge table → ref_table      (child before parent)
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for table in tables {
        in_degree.entry(table.as_str()).or_insert(0);
    }
    let mut seen_dependencies = HashSet::new();
    for (table, ref_table) in dependencies {
        if !table_set.contains(table.as_str()) || !table_set.contains(ref_table.as_str()) {
            continue;
        }
        if !seen_dependencies.insert((table.as_str(), ref_table.as_str())) {
            continue;
        }
        if parents_first {
            // FK-bearing table depends on ref_table — parent comes first.
            *in_degree.entry(table.as_str()).or_insert(0) += 1;
            dependents.entry(ref_table.as_str()).or_default().push(table.as_str());
        } else {
            // ref_table depends on FK-bearing table — child comes first.
            *in_degree.entry(ref_table.as_str()).or_insert(0) += 1;
            dependents.entry(table.as_str()).or_default().push(ref_table.as_str());
        }
    }

    // Kahn's algorithm.
    let mut queue: std::collections::VecDeque<&str> = tables
        .iter()
        .map(String::as_str)
        .filter(|table| in_degree.get(table).copied().unwrap_or_default() == 0)
        .collect();

    let mut sorted: Vec<String> = Vec::new();
    while let Some(table) = queue.pop_front() {
        sorted.push(table.to_string());
        if let Some(deps) = dependents.get(table) {
            for &dependent in deps {
                let deg = in_degree.get_mut(dependent).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(dependent);
                }
            }
        }
    }

    // Append any tables left behind by cycles in their original order.
    if sorted.len() < tables.len() {
        let sorted_set: HashSet<&str> = sorted.iter().map(|s| s.as_str()).collect();
        let mut remaining: Vec<String> = Vec::new();
        for table in tables {
            if !sorted_set.contains(table.as_str()) {
                remaining.push(table.clone());
            }
        }
        sorted.extend(remaining);
    }

    sorted
}

#[derive(Default)]
struct HiveServerTransferCursor {
    started: bool,
    session_id: Option<String>,
}

fn transfer_cursor_sql(
    columns: &[String],
    table: &str,
    schema: &str,
    db_type: &DatabaseType,
    catalog: Option<&str>,
) -> String {
    let full_table = qualified_table(table, schema, db_type, catalog);
    let col_list = columns.iter().map(|column| quote_identifier(column, db_type)).collect::<Vec<_>>().join(", ");
    format!("SELECT {col_list} FROM {full_table}")
}

fn transfer_upsert_falls_back_to_append(db_type: &DatabaseType) -> bool {
    false
}

fn transfer_clear_table_sql(table: &str, schema: &str, db_type: &DatabaseType, catalog: Option<&str>) -> String {
    let full_table = qualified_table(table, schema, db_type, catalog);
    match db_type {
        _ => format!("TRUNCATE TABLE {full_table}"),
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn rename_tables_to_backup<F>(
    state: &Arc<AppState>,
    request: &TransferRequest,
    tables: &[String],
    target_db_type: DatabaseType,
    target_pool_key: &str,
    mut progress_callback: F,
) -> Result<HashMap<String, String>, String>
where
    F: FnMut(TransferProgress),
{
    let source_pool_key = ensure_transfer_pool(
        state,
        &request.source_connection_id,
        &request.source_database,
        request.source_catalog.as_deref(),
    )
    .await?;
    ensure_transfer_source_types_supported(state, request, &source_pool_key).await?;
    let total_tables = tables.len();

    // Resolve target names first so the fail-fast check below sees the names that will
    // actually be renamed (target name casing and existing-table matching included).
    let mut resolved: Vec<(String, String, bool)> = Vec::with_capacity(total_tables);
    for table in tables {
        if is_cancelled(&request.transfer_id).await {
            return Err("Cancelled".to_string());
        }
        let ResolvedTransferTargetTable { name, preexisting } = resolve_transfer_target_table_name(
            state,
            request,
            table,
            target_pool_key,
            &target_db_type,
            request.source_catalog.as_deref(),
            request.target_catalog.as_deref(),
        )
        .await;
        resolved.push((table.clone(), name, preexisting));
    }

    // Refuse before the first rename: a foreign key from outside the collection would
    // follow the rename onto the backup table, and a dependent view would be rewritten
    // against the backup by the server itself — neither can be repaired later in the
    // transfer.
    let target_names = resolved.iter().map(|(_, name, _)| name.clone()).collect::<Vec<_>>();
    crate::transfer_rebuild::ensure_no_external_table_dependencies(
        state,
        target_pool_key,
        &request.target_database,
        &request.target_schema,
        &target_names,
        target_db_type,
    )
    .await?;

    // Preflight every backup name before renaming anything. Renames are not transactional
    // across tables, so a collision discovered halfway through would leave the earlier
    // tables already renamed aside — the failure the all-or-nothing check exists to prevent.
    let mut plan: Vec<(String, String, String)> = Vec::new();
    for (table, target_table, preexisting) in &resolved {
        if !preexisting {
            log::info!("[transfer] rename pre-pass: target table {target_table} does not exist, nothing to back up");
            continue;
        }
        let backup_name = crate::transfer_rebuild::backup_table_name(
            target_db_type,
            &request.transfer_id,
            &format!("{}.{}", request.source_schema, table),
            target_table,
        )?;

        // The backup name is derived, not user-supplied, so a hit means an earlier run of
        // this same transfer left one behind. Never overwrite it: it may be the only copy
        // of the original table (mirrors sqlite_rebuild.rs:498-511).
        let backup_exists = {
            let lookup = list_transfer_tables_isolated(
                state.clone(),
                request.target_connection_id.clone(),
                request.target_database.clone(),
                request.target_schema.clone(),
                request.target_catalog.clone(),
                target_db_type,
                backup_name.clone(),
                1,
            )
            .await?;
            !lookup.is_empty()
        };
        if backup_exists {
            return Err(format!(
                "Backup table name '{}' already exists in the target database. Cannot proceed with \
                 drop_target_before_create; remove the existing backup manually or retry the transfer.",
                backup_name
            ));
        }
        plan.push((table.clone(), target_table.clone(), backup_name));
    }

    // Persist the recovery plan before the first mutation so a crash mid-rename still
    // leaves enough information on disk to find every backup table.
    crate::transfer_rebuild::persist_rebuild_plan(state, request, target_db_type, &plan).await?;

    // Execute the renames. Any failure after the first successful rename leaves retained
    // backups behind; the annotate pass below appends every one of them to the error.
    let mut backup_names: HashMap<String, String> = HashMap::new();
    let rename_pass = async {
        for (i, (table, target_table, backup_name)) in plan.iter().enumerate() {
            if is_cancelled(&request.transfer_id).await {
                return Err("Cancelled".to_string());
            }

            progress_callback(TransferProgress {
                transfer_id: request.transfer_id.clone(),
                table: format!("rename: {table}"),
                table_index: i,
                total_tables,
                rows_transferred: i as u64,
                total_rows: Some(total_tables as u64),
                status: TransferStatus::Running,
                error: None,
                terminal: false,
            });

            log::info!("[transfer] rename pre-pass: renaming {target_table} to backup {backup_name}");

            let rename_sql =
                crate::db_admin_sql::build_rename_object_sql(crate::db_admin_sql::RenameObjectSqlOptions {
                    database_type: Some(target_db_type),
                    object_type: crate::db_admin_sql::DatabaseObjectType::Table,
                    schema: if request.target_schema.is_empty() { None } else { Some(request.target_schema.clone()) },
                    old_name: target_table.clone(),
                    new_name: backup_name.clone(),
                })?;

            execute_on_pool(state, target_pool_key, &rename_sql).await.map_err(|e| {
                format!("Failed to rename target table '{target_table}' to backup '{backup_name}' in pre-pass: {e}")
            })?;

            backup_names.insert(table.clone(), backup_name.clone());

            // PostgreSQL family: `ALTER TABLE ... RENAME TO` does NOT rename the table's
            // indexes, constraints, or owned sequences. They keep their original
            // schema-scoped identifiers, so the main pass's `CREATE INDEX IF NOT EXISTS`
            // would silently no-op and a rebuilt serial column would share the backup's
            // sequence. Rename them aside now; the hash covers each object's own identity,
            // so indexes of the same name on different tables never collide.
            {
                crate::transfer_rebuild::record_rebuild_step(state, &request.transfer_id, table, Vec::new()).await?;
            }
        }
        Ok(())
    };
    if let Err(error) = rename_pass.await {
        if error == "Cancelled" {
            return Err(error);
        }
        // Every table already renamed stays as a backup. The message is the user's map
        // back to their data, and the journal persisted above holds the same list on disk.
        let mut annotated = error;
        for backup_name in backup_names.values() {
            let qualified = qualified_table(
                backup_name,
                &request.target_schema,
                &target_db_type,
                request.target_catalog.as_deref(),
            );
            annotated = crate::transfer_rebuild::annotate_error_with_retained_backup(annotated, &qualified);
        }
        return Err(annotated);
    }

    // MySQL family: free the constraint names the backups still hold before the caller
    // runs the deferred foreign key ALTERs. Doing it here — inside the pre-pass, after
    // every table is renamed — keeps the core API self-contained: both desktop and Web
    // and any direct caller run the deferred ALTERs after this returns, and MySQL
    // constraint names are unique per database, so an unreleased name would collide with
    // the rebuilt table's re-created constraint.
    free_backup_foreign_key_names(state, request, target_db_type, target_pool_key, &backup_names).await?;

    Ok(backup_names)
}

/// Create (or skip) the target table for a single-table transfer.
///
/// Extracted from [`transfer_table_inner`] so the rebuild path can create the table from the
/// source DDL *before* reading the source column list. When the source column read fails, a
/// rebuild still leaves the freshly-created (empty) target table plus the retained backup
/// behind — the recovery contract the failure path relies on (drop the empty table, rename
/// the backup back).
#[allow(clippy::too_many_arguments)]
async fn create_transfer_target_table(
    state: &Arc<AppState>,
    request: &TransferRequest,
    table: &str,
    target_table: &str,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    source_pool_key: &str,
    target_pool_key: &str,
    known_foreign_keys: &HashMap<String, Vec<db::ForeignKeyInfo>>,
    pending_fk_alters: &mut Vec<(String, String)>,
    target_table_preexisting: &mut bool,
    target_renamed_to_backup: bool,
    columns: &[db::ColumnInfo],
    table_comment: Option<&str>,
    pg_compat_transfer: bool,
    preserves_target_table_name: bool,
) -> Result<(), String> {
    {}

    // The pre-pass renamed the target away, so the name is free again. Resetting the
    // flag is what keeps the index / foreign key / PG schema restore paths — all
    // gated on `!target_table_preexisting` — from silently skipping.
    if target_renamed_to_backup {
        *target_table_preexisting = false;
    }

    if *target_table_preexisting {
        log::info!("[transfer] target table {target_table} already exists, skipping create-table DDL");
        return Ok(());
    }

    // Shared DDL planning: the same helper the ownership preview uses, so the
    // confirmation dialog and the actual creation can never disagree on the DDL
    // or the names inside it. Rebuild mode additionally fails closed here when
    // source metadata cannot be trusted.
    let prepared = ddl_plan::prepare_table_ddl(
        state,
        request,
        table,
        target_table,
        source_db_type,
        target_db_type,
        source_pool_key,
        columns,
        table_comment,
        known_foreign_keys,
    )
    .await?;
    let reused_source_ddl = prepared.reused_source_ddl;
    let ddl = prepared.ddl;
    let ddl = { ddl };
    let deferred_fk_alters = prepared.deferred_fk_alters;
    log::info!("[transfer] creating target table: {}", ddl.chars().take(200).collect::<String>());
    let target_table_created = transfer_create_table_created(
        execute_transfer_create_table_ddl_on_pool(state, target_pool_key, &ddl, target_db_type, reused_source_ddl)
            .await,
        "Failed to create table",
    )?;

    Ok(())
}

/// Transfer a single table. Returns rows transferred.
/// `progress_callback` is invoked for progress updates.
///
/// `preexisting_backup_names` carries the output of [`rename_tables_to_backup`] and is
/// required whenever `drop_target_before_create` is set — this pass only checks whether the
/// table was renamed aside, and never renames or drops anything itself. Removing the backups
/// is [`drop_backup_tables`], after every table has succeeded.
#[allow(clippy::too_many_arguments)]
async fn transfer_table_inner<F>(
    state: &Arc<AppState>,
    request: &TransferRequest,
    table: &str,
    table_index: usize,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    source_pool_key: &str,
    target_pool_key: &str,
    known_foreign_keys: &HashMap<String, Vec<db::ForeignKeyInfo>>,
    pending_fk_alters: &mut Vec<(String, String)>,
    preexisting_backup_names: Option<&HashMap<String, String>>,
    mut progress_callback: F,
) -> Result<u64, String>
where
    F: FnMut(TransferProgress),
{
    {}
    {}

    let table_filter = transfer_table_filter_for(request, table)?;
    if table_filter.is_some() && should_copy_data(&request.content) && !transfer_table_filter_supported(source_db_type)
    {
        return Err(format!(
            "Table filters are not supported for source database '{}'; only the MySQL and PostgreSQL families are supported",
            source_db_type.as_str()
        ));
    }

    let total_tables = request.tables.len();
    let pg_compat_transfer = false;
    let ResolvedTransferTargetTable { name: target_table, preexisting: mut target_table_preexisting } =
        resolve_transfer_target_table_name(
            state,
            request,
            table,
            target_pool_key,
            target_db_type,
            request.source_catalog.as_deref(),
            request.target_catalog.as_deref(),
        )
        .await;
    let preserves_target_table_name = target_table == table;

    // Did the rename pre-pass move this table's target aside? False when the option is off,
    // or when the target table did not exist, so nothing was renamed. Removing the backup is
    // `drop_backup_tables`' job once the whole table loop has succeeded.
    let target_renamed_to_backup =
        match (request.drop_target_before_create, preexisting_backup_names) {
            (false, _) => false,
            (true, Some(backup_map)) => backup_map.contains_key(table),
            // rename_tables_to_backup owns the rename; without it the target table would
            // still be in place and this pass would quietly append into it.
            (true, None) => return Err(
                "drop_target_before_create requires the rename pre-pass: call rename_tables_to_backup and pass its \
                 result to transfer_table."
                    .to_string(),
            ),
        };

    // PostgreSQL's table list filter is fuzzy, so it cannot identify the
    // requested table's comment when similarly named tables exist.
    let table_comment = {
        // Keep the list-tables metadata chain on its own task stack, just like
        // the target-table lookup above.
        list_transfer_tables_isolated(
            state.clone(),
            request.source_connection_id.clone(),
            request.source_database.clone(),
            request.source_schema.clone(),
            request.source_catalog.clone(),
            *source_db_type,
            table.to_string(),
            1,
        )
        .await
        .unwrap_or_default()
        .into_iter()
        .next()
        .and_then(|table| table.comment)
    };

    // Get source columns (deduplicate by name).
    //
    // A rebuild reads its CREATE DDL from the source connection (not the source pool), so a
    // column-read failure must still create the target table first: that leaves the recovery
    // contract intact — an empty new table plus the retained backup — so the user can drop the
    // empty table and rename the backup back.
    let columns: Vec<db::ColumnInfo> = match get_columns_for_transfer(
        state,
        source_pool_key,
        &request.source_connection_id,
        &request.source_database,
        &request.source_schema,
        table,
        request.source_catalog.as_deref(),
    )
    .await
    {
        Ok(raw) => {
            let mut seen = std::collections::HashSet::new();
            raw.into_iter().filter(|c| seen.insert(c.name.clone())).collect()
        }
        Err(error) => {
            if request.drop_target_before_create && target_renamed_to_backup {
                let mut preexisting = target_table_preexisting;
                if let Err(create_error) = create_transfer_target_table(
                    state,
                    request,
                    table,
                    &target_table,
                    source_db_type,
                    target_db_type,
                    source_pool_key,
                    target_pool_key,
                    known_foreign_keys,
                    pending_fk_alters,
                    &mut preexisting,
                    true,
                    &[],
                    table_comment.as_deref(),
                    false,
                    preserves_target_table_name,
                )
                .await
                {
                    return Err(format!(
                        "{error} Additionally, the rebuild could not create the target table: {create_error}"
                    ));
                }
            }
            return Err(error);
        }
    };

    if columns.is_empty() {
        return Err(format!("No columns found for table {table}"));
    }

    let is_doris_source = {
        let configs = state.configs.read().await;
        false
    };
    ensure_transfer_columns_supported(request, false, table, &columns)?;

    let writable_columns = writable_transfer_columns(&columns, source_db_type, target_db_type);
    let default_rows_only = mysql_generated_only_transfer(&columns, source_db_type, target_db_type);
    if writable_columns.is_empty() && !default_rows_only {
        return Err(format!("No writable columns found for table {table}"));
    }

    let mut col_names: Vec<String> = writable_columns.iter().map(|c| c.name.clone()).collect();
    let mut col_types: Vec<Option<String>> = writable_columns.iter().map(|c| Some(c.data_type.clone())).collect();
    let primary_key_columns = transfer_key_columns(&writable_columns, source_db_type);
    if should_copy_data(&request.content) {
        log::info!("[transfer] {} has {} columns, counting rows...", table, columns.len());
    }

    let total_rows = if should_copy_data(&request.content) {
        // Count source rows only for data-bearing transfers.
        let sql = match table_filter.as_ref() {
            Some(filter) => transfer_filter_count_sql(
                table,
                &request.source_schema,
                source_db_type,
                request.source_catalog.as_deref(),
                filter,
            ),
            None => count_sql(table, &request.source_schema, source_db_type, request.source_catalog.as_deref()),
        };
        match execute_on_pool(state, source_pool_key, &sql).await {
            Ok(result) => result.rows.first().and_then(|r| r.first()).and_then(|v| match v {
                serde_json::Value::Number(n) => n.as_u64(),
                serde_json::Value::String(s) => s.parse::<u64>().ok(),
                _ => None,
            }),
            Err(e) => {
                log::warn!("[transfer] count failed for {}: {}", table, e);
                None
            }
        }
    } else {
        None
    };
    log::info!("[transfer] {} total_rows={:?}", table, total_rows);

    let server_side_complex_copy = false;
    {}

    // Create table on target if requested
    if request.create_table {
        create_transfer_target_table(
            state,
            request,
            table,
            &target_table,
            source_db_type,
            target_db_type,
            source_pool_key,
            target_pool_key,
            known_foreign_keys,
            pending_fk_alters,
            &mut target_table_preexisting,
            target_renamed_to_backup,
            &columns,
            table_comment.as_deref(),
            false,
            preserves_target_table_name,
        )
        .await?;
    }

    let should_restore_postgres_table_schema = false;

    // Structure-only transfer: complete the table's post-create schema DDL,
    // then skip everything data-related.
    if !should_copy_data(&request.content) {
        if request.create_table && target_table_preexisting {
            let target_columns = get_columns_for_transfer(
                state,
                target_pool_key,
                &request.target_connection_id,
                &request.target_database,
                &request.target_schema,
                &target_table,
                request.target_catalog.as_deref(),
            )
            .await
            .map_err(|error| {
                format!("Failed to inspect target table '{target_table}' columns before transfer: {error}")
            })?;
            validate_preexisting_target_columns(
                &target_columns,
                &col_names,
                target_db_type,
                request.quote_target_column_names,
                &target_table,
            )?;
        }
        {}
        return Ok(0);
    }

    // A preexisting target also needs its columns read, even for a data-only
    // transfer: the write SQL has to address the target's declared column
    // names, which can differ from the source in case (#9320).
    //
    // SQL Server belongs to the always-read set for its identity flags, not just
    // for a preexisting target: a freshly created target reuses the source DDL
    // (`can_reuse`), which carries the source's `IDENTITY` clause, so the new
    // target is an identity target too. `writes_identity_insert_columns` decides
    // whether the batch needs the `SET IDENTITY_INSERT` wrapper, and without the
    // target metadata it stays false — SQL Server then rejects the explicit
    // identity values with 544.
    let needs_target_columns = default_rows_only
        || target_table_preexisting
        || (request.mode == TransferMode::Upsert && !transfer_upsert_falls_back_to_append(target_db_type))
        || false;
    let target_columns = if needs_target_columns {
        get_columns_for_transfer(
            state,
            target_pool_key,
            &request.target_connection_id,
            &request.target_database,
            &request.target_schema,
            &target_table,
            request.target_catalog.as_deref(),
        )
        .await
        .map_err(|error| format!("Failed to inspect target table '{target_table}' columns before transfer: {error}"))?
    } else {
        Vec::new()
    };

    // Empty-column INSERTs are safe only when the target also computes every
    // value. Reject incompatible data-only targets before an overwrite truncates
    // them; otherwise ordinary columns could silently receive defaults instead.
    if default_rows_only && !mysql_generated_only_transfer(&target_columns, target_db_type, target_db_type) {
        return Err(format!(
            "Target table '{target_table}' must contain only generated columns for default-row transfer"
        ));
    }

    // The user asked DBX to sync structure (create_table), but the target
    // table already existed so the create-table DDL above was skipped (see
    // "skipping create-table DDL" above). If the untouched target structure
    // can't accept the planned insert, fail fast here instead of truncating
    // the target's existing data and then hitting an opaque driver error.
    //
    // Skip this validation when drop_target_before_create is true: the original
    // target table was renamed to a backup and a fresh table matching the source
    // structure was just created, so structural incompatibility is not possible.
    if request.create_table && target_table_preexisting && !request.drop_target_before_create {
        validate_preexisting_target_columns(
            &target_columns,
            &col_names,
            target_db_type,
            request.quote_target_column_names,
            &target_table,
        )?;
    }

    {}

    // Read SQL keeps the source names (the source table is untouched); write SQL
    // has to use the names the target table actually declares.
    let write_col_names = if target_table_preexisting && !target_columns.is_empty() {
        resolve_transfer_target_column_names(&col_names, &target_columns)
    } else {
        col_names.clone()
    };

    {}

    {}

    // Truncate target if overwrite mode (only when not rebuilding the table).
    // When drop_target_before_create is true, the target table was just created
    // and is already empty, so TRUNCATE is unnecessary.
    if request.mode == TransferMode::Overwrite && !request.drop_target_before_create {
        let truncate_sql = transfer_clear_table_sql(
            &target_table,
            &request.target_schema,
            target_db_type,
            request.target_catalog.as_deref(),
        );
        execute_on_pool(state, target_pool_key, &truncate_sql).await.map_err(|e| format!("Failed to truncate: {e}"))?;
    }

    // Determine effective mode and PK columns for upsert
    let (effective_mode, pk_columns) = if request.mode == TransferMode::Upsert {
        if transfer_upsert_falls_back_to_append(target_db_type) {
            log::warn!("[transfer] upsert not supported for {:?}, falling back to append", target_db_type);
            (TransferMode::Append, vec![])
        } else {
            let pks: Vec<String> = transfer_key_columns(&target_columns, target_db_type)
                .into_iter()
                .filter(|name| col_names.iter().any(|column_name| column_name.eq_ignore_ascii_case(name)))
                .collect();
            if pks.is_empty() {
                log::warn!("[transfer] table {} has no primary key, falling back to append", table);
                (TransferMode::Append, vec![])
            } else {
                (TransferMode::Upsert, pks)
            }
        }
    } else {
        (request.mode.clone(), vec![])
    };

    let writes_identity_insert_columns = false;
    let overrides_postgres_system_values = false;
    // Transfer data in batches
    let batch_size = if request.batch_size == 0 { 1000 } else { request.batch_size };
    let mut offset: u64 = 0;
    let mut total_transferred: u64 = 0;

    {}

    // COPY fast path: PG-family append/overwrite transfers stream the whole
    // table through the COPY protocol instead of paged SELECT + multi-row
    // INSERT — no per-batch statement parsing, no JSON round-trip. Any failure
    // is atomic (the target's COPY statement aborts), so the paged INSERT loop
    // below runs unchanged as a fallback.
    let mut copy_rows: Option<u64> = None;
    {}
    // Keyset paging state: when the source can page by key cursor, each page
    // seeks with `WHERE (pk...) > <cursor>` instead of OFFSET, which rescans
    // and discards every previously read row (quadratic in table size). Falls
    // back to OFFSET (keeping the same key ordering) when the key metadata
    // does not hold up mid-table.
    let mut keyset_indexes = if table_filter.is_some() {
        None
    } else {
        transfer_keyset_column_indexes(&writable_columns, &primary_key_columns, source_db_type)
    };
    let mut keyset_cursor: Vec<serde_json::Value> = Vec::new();
    // A single Agent cursor keeps Hive-family rows in one query execution. Inceptor
    // rejects the generic LIMIT/OFFSET form, just like the other Agent cursor paths.
    // Re-running LIMIT/OFFSET pages is unstable for tables without a unique key.
    let use_hive_server_cursor = false;
    let hive_server_transfer_sql = use_hive_server_cursor.then(|| {
        transfer_cursor_sql(
            &col_names,
            table,
            &request.source_schema,
            source_db_type,
            request.source_catalog.as_deref(),
        )
    });
    let mut hive_server_cursor = HiveServerTransferCursor::default();
    // Key-less PostgreSQL heaps page by `ctid` windows instead of OFFSET: an
    // OFFSET page re-reads every row before it, so a ten-million-row table gets
    // slower as it runs and never finishes. Tables with a usable key keep the
    // keyset cursor above, and the COPY fast path already covers the
    // PostgreSQL-to-PostgreSQL case.

    // Query once for the whole table: caps every write batch page below.
    let write_hard_limit = transfer_write_mysql_hard_limit(state, target_pool_key).await;
    let transfer_result: Result<(), String> = async {
        if copy_rows.is_some() {
            // The COPY fast path already streamed the whole table.
            return Ok(());
        }
        loop {
            if is_cancelled(&request.transfer_id).await {
                return Err("Cancelled".to_string());
            }

            let (mut result, mysql_spatial_markers) = {
                let sql = if let Some(filter) = table_filter.as_ref() {
                    transfer_filter_page_sql(
                        &col_names,
                        table,
                        &request.source_schema,
                        source_db_type,
                        request.source_catalog.as_deref(),
                        filter,
                        offset,
                        batch_size,
                        &primary_key_columns,
                        default_rows_only,
                    )
                } else if default_rows_only {
                    // Preserve row multiplicity without reading generated values
                    // (which must never be assigned on the target).
                    let source_table = qualified_table(
                        table,
                        &request.source_schema,
                        source_db_type,
                        request.source_catalog.as_deref(),
                    );
                    format!("SELECT 1 FROM {source_table} LIMIT {batch_size} OFFSET {offset}")
                } else if keyset_indexes.is_some() {
                    keyset_pagination_sql(
                        &col_names,
                        table,
                        &request.source_schema,
                        source_db_type,
                        &primary_key_columns,
                        &keyset_cursor,
                        batch_size,
                    )
                } else {
                    pagination_sql_with_order(
                        &col_names,
                        table,
                        &request.source_schema,
                        source_db_type,
                        offset,
                        batch_size,
                        &primary_key_columns,
                        request.source_catalog.as_deref(),
                    )
                };
                let (sql, mysql_spatial_markers) =
                    mysql_spatial_transfer_select_sql(sql, &col_names, &col_types, source_db_type, target_db_type);
                // Cap the result at `batch_size` (not the 10k default row limit), so a
                // large batch is never truncated into looking like a short final page.
                (
                    execute_on_pool_with_max_rows(state, source_pool_key, &sql, Some(batch_size)).await?,
                    mysql_spatial_markers,
                )
            };
            let has_more = result.has_more;
            let row_count = result.rows.len();

            if row_count == 0 {
                // A `ctid` window can be empty — its rows were deleted, or the
                // previous page ended exactly on the window boundary — while
                // the windows after it still hold rows, so only stop once the
                // heap has been walked to its end.

                {}
                break;
            }

            // Drops the trailing `ctid` cursor column from every row before the
            // rows reach the INSERT builder, and reports whether this page
            // consumed the last window of the heap.

            if let Some(indexes) = keyset_indexes.as_deref() {
                match advance_keyset_cursor(&mut keyset_cursor, &result.rows, indexes, table)? {
                    KeysetAdvance::Advanced => {}
                    KeysetAdvance::FallBackToOffset => {
                        log::warn!(
                            "[transfer] {table}: NULL value in a key column at row {offset}; \
                             falling back to OFFSET paging for the remaining rows"
                        );
                        keyset_indexes = None;
                    }
                }
            }

            if default_rows_only {
                // The existing batching formatter emits () for each empty row:
                // MySQL INSERT INTO table () VALUES (), (). Keep its size limits,
                // progress accounting and cancellation checks for this path too.
                for row in &mut result.rows {
                    row.clear();
                }
            }
            let write_statements = generate_transfer_write_sql_batches_with_column_quoting(
                &effective_mode,
                &write_col_names,
                &col_types,
                &result.rows,
                &target_table,
                &request.target_schema,
                target_db_type,
                &pk_columns,
                request.target_catalog.as_deref(),
                false,
                mysql_spatial_markers,
                request.quote_target_column_names,
                write_hard_limit,
            )?;
            for (statement_index, batch_sql) in write_statements.iter().enumerate() {
                execute_transfer_write_statement(
                    state,
                    target_pool_key,
                    batch_sql,
                    target_db_type,
                    &target_table,
                    &request.target_schema,
                    false,
                )
                .await
                .map_err(|e| {
                    let absolute_row = parse_mysql_row_error(&e).map(|row| offset + row);
                    match absolute_row {
                        Some(row) => format!(
                            "Insert failed for table '{target_table}' at row {row} (chunk {} of {}): {e}",
                            statement_index + 1,
                            write_statements.len()
                        ),
                        None => format!(
                            "Insert failed for table '{target_table}' at offset {offset}, chunk {} of {}: {e}",
                            statement_index + 1,
                            write_statements.len()
                        ),
                    }
                })?;
            }

            total_transferred += row_count as u64;
            log::info!("[transfer] {} batch +{} rows (total {})", table, row_count, total_transferred);
            offset += row_count as u64;

            progress_callback(TransferProgress {
                transfer_id: request.transfer_id.clone(),
                table: table.to_string(),
                table_index,
                total_tables,
                rows_transferred: total_transferred,
                total_rows,
                status: TransferStatus::Running,
                error: None,
                terminal: false,
            });

            if (false) || (!use_hive_server_cursor && row_count < batch_size) {
                break;
            }
        }
        Ok(())
    }
    .await;

    transfer_result?;

    {}

    {}

    Ok(total_transferred)
}

/// Free the constraint names the backups are still holding (MySQL family).
///
/// MySQL constraint names are unique per database. The rebuilt tables re-create their
/// foreign keys under the source constraint names through the deferred `ADD CONSTRAINT`
/// statements, but every backup still holds a constraint with that exact name — the
/// rename pre-pass moves tables, not constraint names. Each backup constraint is
/// atomically replaced (single `ALTER` statement) with a derived backup name, keeping
/// the referenced table — already redirected to the referenced backup by the rename —
/// and the ON UPDATE/DELETE rules intact.
///
/// No-op for non-MySQL targets (constraint names are per-table there) and for pools the
/// MySQL metadata reader cannot reach.
pub async fn free_backup_foreign_key_names(
    state: &Arc<AppState>,
    request: &TransferRequest,
    target_db_type: DatabaseType,
    target_pool_key: &str,
    backup_names: &HashMap<String, String>,
) -> Result<(), String> {
    if backup_names.is_empty() || !supports_deferred_mysql_foreign_keys(&target_db_type) {
        return Ok(());
    }
    let pool = {
        let pool_handle = state.pool_handle(target_pool_key).await;
        match pool_handle.as_ref() {
            Some(PoolKind::Mysql(pool, _)) => pool.clone(),
            _ => return Ok(()),
        }
    };
    for backup_table in backup_names.values() {
        if is_cancelled(&request.transfer_id).await {
            return Err("Cancelled".to_string());
        }
        let foreign_keys = db::mysql::list_foreign_keys(&pool, &request.target_database, backup_table).await?;
        for (name, group) in group_foreign_keys_by_constraint_name(&foreign_keys) {
            if name.contains(crate::transfer_rebuild::BACKUP_TABLE_MARKER) {
                // Already freed by an earlier attempt; re-reading metadata after a
                // partial success must not try to free the derived name again.
                continue;
            }
            let new_name = crate::transfer_rebuild::backup_table_name(
                target_db_type,
                &request.transfer_id,
                &format!("{}{}#constraint:{}", request.source_schema, backup_table, name),
                name,
            )?;
            let columns = group
                .iter()
                .map(|foreign_key| quote_identifier(&foreign_key.column, &target_db_type))
                .collect::<Vec<_>>()
                .join(", ");
            let ref_columns = group
                .iter()
                .map(|foreign_key| quote_identifier(&foreign_key.ref_column, &target_db_type))
                .collect::<Vec<_>>()
                .join(", ");
            let referenced_table = match group[0].ref_schema.as_deref() {
                // Same-database reference: after the rename the referenced name already
                // points at the referenced table's backup, so keep it as-is.
                Some(ref_schema) if ref_schema == request.target_database => {
                    quote_identifier(&group[0].ref_table, &target_db_type)
                }
                // Cross-database reference: nothing in this transfer renamed it.
                Some(ref_schema) => {
                    format!(
                        "{}.{}",
                        quote_identifier(ref_schema, &target_db_type),
                        quote_identifier(&group[0].ref_table, &target_db_type)
                    )
                }
                None => quote_identifier(&group[0].ref_table, &target_db_type),
            };
            let mut statement = format!(
                "ALTER TABLE {} DROP FOREIGN KEY {}, ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} ({})",
                quote_identifier(backup_table, &target_db_type),
                quote_identifier(name, &target_db_type),
                quote_identifier(&new_name, &target_db_type),
                columns,
                referenced_table,
                ref_columns,
            );
            if let Some(on_delete) = group[0].on_delete.as_deref() {
                statement.push_str(&format!(" ON DELETE {on_delete}"));
            }
            if let Some(on_update) = group[0].on_update.as_deref() {
                statement.push_str(&format!(" ON UPDATE {on_update}"));
            }
            execute_on_pool(state, target_pool_key, &statement).await.map_err(|e| {
                format!("Failed to free constraint name '{name}' on backup table '{backup_table}': {e}")
            })?;
        }
    }
    Ok(())
}

/// Drop every foreign key constraint the backup tables still hold.
///
/// The rename pre-pass can move a foreign key cycle into the backup set, and a cycle has
/// no valid sequential `DROP TABLE` order at all. Dropping the constraints on the backups
/// breaks only backup-to-backup edges: the rebuilt tables' restored constraints and every
/// external table are untouched. Individual failures are logged and skipped — the
/// following drop loop reports the tables it could not remove.
async fn break_backup_foreign_key_graph(
    state: &Arc<AppState>,
    request: &TransferRequest,
    target_db_type: DatabaseType,
    target_pool_key: &str,
    backup_names: &HashMap<String, String>,
) {
    // MySQL family: constraint names were already freed by [`free_backup_foreign_key_names`]
    // when it ran; this re-lists whatever constraints remain (including renamed ones) and
    // drops them outright, because the tables themselves are about to be dropped.
    let mysql_pool = {
        let pool_handle = state.pool_handle(target_pool_key).await;
        match pool_handle.as_ref() {
            Some(PoolKind::Mysql(pool, _)) => Some(pool.clone()),
            _ => None,
        }
    };
    let postgres_family = false;
    for backup_table in backup_names.values() {
        if is_cancelled(&request.transfer_id).await {
            return;
        }
        if let Some(pool) = &mysql_pool {
            match db::mysql::list_foreign_keys(pool, &request.target_database, backup_table).await {
                Ok(foreign_keys) => {
                    for name in foreign_keys.iter().map(|fk| fk.name.as_str()).collect::<Vec<_>>() {
                        let drop_fk = format!(
                            "ALTER TABLE {} DROP FOREIGN KEY {}",
                            quote_identifier(backup_table, &target_db_type),
                            quote_identifier(name, &target_db_type)
                        );
                        if let Err(error) = execute_on_pool(state, target_pool_key, &drop_fk).await {
                            log::warn!(
                                "[transfer] failed to drop foreign key {name} on backup {backup_table}: {error}"
                            );
                        }
                    }
                }
                Err(error) => log::warn!("[transfer] failed to list foreign keys on backup {backup_table}: {error}"),
            }
        } else {
        }
    }
}

/// Drop the backups left behind by [`rename_tables_to_backup`].
///
/// Two ordering rules make this a post-loop step rather than per-table cleanup, and neither
/// shows up until a foreign key connects two selected tables:
///
/// - A backup can still be referenced by another backup. Renaming children first keeps each
///   pair consistent, which also means `DROP TABLE parent_bak` fails while `child_bak` exists.
///   So the drops run children first — and the backup-to-backup foreign key graph is broken
///   first, because a foreign key cycle among the backups has no valid drop order at all.
/// - The backups must be gone before the transfer reports success, but only after the
///   deferred foreign key ALTERs and the selected schema objects have been restored: on
///   MySQL the rebuilt tables cannot take over the source constraint names until
///   [`free_backup_foreign_key_names`] has freed them from the backups.
///
/// `drop_order` is the children-first list handed to the rename pre-pass. A backup whose table
/// is missing from it is still dropped, in name order, so none can leak.
pub async fn drop_backup_tables(
    state: &Arc<AppState>,
    request: &TransferRequest,
    target_db_type: DatabaseType,
    target_pool_key: &str,
    backup_names: &HashMap<String, String>,
    drop_order: &[String],
) -> Result<(), String> {
    let mut ordered: Vec<&String> = drop_order.iter().filter(|table| backup_names.contains_key(*table)).collect();
    let mut leftovers: Vec<&String> = backup_names.keys().filter(|table| !drop_order.contains(*table)).collect();
    leftovers.sort();
    ordered.extend(leftovers);

    break_backup_foreign_key_graph(state, request, target_db_type, target_pool_key, backup_names).await;

    let mut retained: Vec<String> = Vec::new();
    for table in ordered {
        let Some(backup_name) = backup_names.get(table) else {
            continue;
        };
        let drop_sql = crate::db_admin_sql::build_drop_table_sql(crate::db_admin_sql::TableAdminSqlOptions {
            database_type: Some(target_db_type),
            schema: if request.target_schema.is_empty() { None } else { Some(request.target_schema.clone()) },
            table_name: backup_name.clone(),
            // Rendered for the PostgreSQL family only — `build_drop_table_sql` drops the
            // keyword everywhere else. The backup-to-backup foreign key graph has already
            // been broken explicitly, so CASCADE is a narrow safety net for objects that
            // metadata could not see, not the mechanism the cleanup relies on.
            cascade: Some(true),
            identifier_quote: None,
        });
        match execute_on_pool(state, target_pool_key, &drop_sql).await {
            Ok(_) => log::info!("[transfer] dropped backup table {backup_name}"),
            Err(error) => {
                log::error!("[transfer] failed to drop backup table {backup_name}: {error}");
                retained.push(format!("{backup_name} ({error})"));
            }
        }
    }

    if retained.is_empty() {
        // The whole rebuild — backups, renamed objects, cleanup — succeeded. The recovery
        // journal has nothing left to describe.
        if let Err(error) = crate::transfer_rebuild::complete_rebuild_journal(state, &request.transfer_id).await {
            log::warn!("[transfer] failed to remove the rebuild recovery journal: {error}");
        }
        return Ok(());
    }
    Err(format!(
        "Transfer completed, but {} backup table(s) could not be dropped and still occupy space: {}. \
         Remove them manually to finish cleanup.",
        retained.len(),
        retained.join(", ")
    ))
}

/// Transfer one table on its own Tokio task so the large transfer future and
/// nested driver metadata futures do not share a single worker stack.
///
/// Pass the [`rename_tables_to_backup`] result as `preexisting_backup_names` whenever
/// `drop_target_before_create` is set; the transfer errors out without it.
#[allow(clippy::too_many_arguments)]
pub async fn transfer_table<F>(
    state: &Arc<AppState>,
    request: &TransferRequest,
    table: &str,
    table_index: usize,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    source_pool_key: &str,
    target_pool_key: &str,
    known_foreign_keys: &HashMap<String, Vec<db::ForeignKeyInfo>>,
    pending_fk_alters: &mut Vec<(String, String)>,
    preexisting_backup_names: Option<&HashMap<String, String>>,
    mut progress_callback: F,
) -> Result<u64, String>
where
    F: FnMut(TransferProgress),
{
    let state = state.clone();
    let request = request.clone();
    let table = table.to_string();
    let source_db_type = *source_db_type;
    let target_db_type = *target_db_type;
    let source_pool_key = source_pool_key.to_string();
    let target_pool_key = target_pool_key.to_string();
    let known_foreign_keys = known_foreign_keys
        .get(&table)
        .map(|foreign_keys| HashMap::from([(table.clone(), foreign_keys.clone())]))
        .unwrap_or_default();
    let preexisting_backup_names = preexisting_backup_names.cloned();
    // Kept outside the spawned task: `request` moves into it, and the error path below
    // still needs the backup's qualified name to point the user at their data.
    let backup_for_this_table = request
        .drop_target_before_create
        .then(|| preexisting_backup_names.as_ref().and_then(|names| names.get(&table).cloned()))
        .flatten();
    let request_target_schema = request.target_schema.clone();
    let request_target_catalog = request.target_catalog.clone();
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(TRANSFER_PROGRESS_CHANNEL_CAPACITY);

    let mut task = tokio::spawn(async move {
        let mut task_pending_fk_alters = Vec::new();
        let result = transfer_table_inner(
            &state,
            &request,
            &table,
            table_index,
            &source_db_type,
            &target_db_type,
            &source_pool_key,
            &target_pool_key,
            &known_foreign_keys,
            &mut task_pending_fk_alters,
            preexisting_backup_names.as_ref(),
            move |progress| {
                try_send_transfer_progress(&progress_tx, progress);
            },
        )
        .await;
        (result, task_pending_fk_alters)
    });
    let _abort_on_drop = AbortTransferTaskOnDrop(task.abort_handle());

    loop {
        tokio::select! {
            biased;
            Some(progress) = progress_rx.recv() => progress_callback(progress),
            result = &mut task => {
                let (result, task_pending_fk_alters) =
                    result.map_err(|error| format!("Transfer table task failed: {error}"))?;
                pending_fk_alters.extend(task_pending_fk_alters);
                // Every failure past the rename pre-pass leaves the original table under its
                // backup name. This is the last place that still holds the name, so the note
                // is attached here rather than at each of the inner `?` sites. Cancellation
                // keeps its exact discriminator: the callers match on the bare "Cancelled"
                // string to render a cancelled (not failed) terminal progress event.
                return match (result, backup_for_this_table.as_deref()) {
                    (Err(error), Some(backup_name)) if error != "Cancelled" => {
                        Err(crate::transfer_rebuild::annotate_error_with_retained_backup(
                            error,
                            &qualified_table(backup_name, &request_target_schema, &target_db_type, request_target_catalog.as_deref()),
                        ))
                    }
                    (result, _) => result,
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_transfer_agent_trace(path: &std::path::Path) -> Vec<serde_json::Value> {
        std::fs::read_to_string(path).unwrap().lines().map(|line| serde_json::from_str(line).unwrap()).collect()
    }

    #[test]
    fn transfer_query_timeout_errors_are_classified() {
        assert!(is_transfer_query_timeout("Query timed out after 1 seconds"));
        assert!(is_transfer_query_timeout("查询超时 (1s)"));
        assert!(is_transfer_query_timeout("查詢逾時 (1s)"));
        assert!(!is_transfer_query_timeout("Connection timed out while loading tables"));
    }

    use serde_json::json;

    fn test_column(name: &str, data_type: &str) -> db::ColumnInfo {
        db::ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            is_nullable: true,
            column_default: None,
            is_primary_key: false,
            extra: None,
            comment: None,
            numeric_precision: None,
            numeric_scale: None,
            character_maximum_length: None,
            enum_values: None,
            ..Default::default()
        }
    }

    fn test_table(name: &str) -> db::TableInfo {
        db::TableInfo {
            name: name.to_string(),
            table_type: "TABLE".to_string(),
            valid: None,
            comment: None,
            parent_schema: None,
            parent_name: None,
        }
    }

    fn test_query_result(rows: Vec<Vec<serde_json::Value>>) -> db::QueryResult {
        db::QueryResult {
            columns: Vec::new(),
            column_types: Vec::new(),
            column_sortables: Vec::new(),
            spatial_columns: vec![],
            spatial_values: vec![],
            rows,
            affected_rows: 0,
            execution_time_ms: 0,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        }
    }

    #[test]
    fn table_dependency_sort_places_parents_before_children() {
        let tables = vec!["audit".to_string(), "users".to_string(), "orders".to_string()];
        let dependencies =
            vec![("audit".to_string(), "orders".to_string()), ("orders".to_string(), "users".to_string())];

        assert_eq!(
            sort_table_names_by_dependencies(&tables, &dependencies, true),
            vec!["users".to_string(), "orders".to_string(), "audit".to_string()]
        );
        assert_eq!(
            sort_table_names_by_dependencies(&tables, &dependencies, false),
            vec!["audit".to_string(), "orders".to_string(), "users".to_string()]
        );
    }

    /// The rename pre-pass of `drop_target_before_create` consumes the `parents_first =
    /// false` order, and a foreign key cycle has no such order. Cycle members must still
    /// come out — appended in their original order — because dropping them would silently
    /// leave those tables un-renamed and the main pass would append into the old table.
    #[test]
    fn table_dependency_sort_keeps_cycle_members_instead_of_dropping_them() {
        let tables = vec!["employees".to_string(), "departments".to_string(), "regions".to_string()];
        // employees <-> departments is a cycle; regions is free of foreign keys.
        let dependencies = vec![
            ("employees".to_string(), "departments".to_string()),
            ("departments".to_string(), "employees".to_string()),
        ];

        for parents_first in [true, false] {
            let sorted = sort_table_names_by_dependencies(&tables, &dependencies, parents_first);
            assert_eq!(sorted.len(), tables.len(), "cycle members were dropped (parents_first={parents_first})");
            assert_eq!(
                sorted,
                vec!["regions".to_string(), "employees".to_string(), "departments".to_string()],
                "the acyclic table sorts first, then the cycle in input order (parents_first={parents_first})"
            );
        }
    }

    #[test]
    fn table_dependency_sort_ignores_duplicates_and_out_of_scope_tables() {
        let tables = vec!["orders".to_string(), "users".to_string(), "logs".to_string()];
        let dependencies = vec![
            ("orders".to_string(), "users".to_string()),
            ("orders".to_string(), "users".to_string()),
            ("logs".to_string(), "external_users".to_string()),
        ];

        assert_eq!(
            sort_table_names_by_dependencies(&tables, &dependencies, true),
            vec!["users".to_string(), "logs".to_string(), "orders".to_string()]
        );
    }

    async fn test_app_state() -> (AppState, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("dbx-transfer-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = crate::persistence::test_storage::open(&dir.join("storage.db")).await.unwrap();
        let state =
            AppState::new_with_plugin_dir_and_app_version(storage, dir.join("plugins"), env!("CARGO_PKG_VERSION"));
        (state, dir)
    }

    #[test]
    fn transfer_content_defaults_to_structure_and_data() {
        let request: TransferRequest = serde_json::from_value(serde_json::json!({
            "transferId": "t1", "sourceConnectionId": "s", "sourceDatabase": "db",
            "sourceSchema": "public", "targetConnectionId": "t", "targetDatabase": "db",
            "targetSchema": "public", "tables": ["a"], "createTable": true,
            "mode": "append", "targetTableNameCase": "preserve", "batchSize": 1000
        }))
        .unwrap();
        assert_eq!(request.content, TransferContent::StructureAndData);
        assert_eq!(request.objects, None);
        assert!(request.quote_target_column_names);
    }

    #[test]
    fn transfer_request_distinguishes_missing_empty_and_selected_objects() {
        let mut base = serde_json::json!({
            "transferId": "t1", "sourceConnectionId": "s", "sourceDatabase": "db",
            "sourceSchema": "public", "targetConnectionId": "t", "targetDatabase": "db",
            "targetSchema": "public", "tables": ["a"], "createTable": true,
            "mode": "append", "targetTableNameCase": "preserve", "batchSize": 1000
        });
        let legacy: TransferRequest = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(legacy.objects, None);
        assert!(serde_json::to_value(&legacy).unwrap().get("objects").is_none());

        base["objects"] = serde_json::json!([]);
        let explicit_empty: TransferRequest = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(explicit_empty.objects, Some(Vec::new()));
        assert_eq!(serde_json::to_value(&explicit_empty).unwrap()["objects"], serde_json::json!([]));

        base["objects"] = serde_json::json!([{"objectType": "VIEW", "names": ["v1"]}]);
        let explicit_selection: TransferRequest = serde_json::from_value(base).unwrap();
        assert_eq!(
            explicit_selection.objects,
            Some(vec![TransferObjectSelection {
                object_type: TransferObjectKind::View,
                names: vec!["v1".to_string()],
            }]),
        );
    }

    #[test]
    fn transfer_request_serializes_new_fields_camel_case() {
        let request = TransferRequest {
            table_filters: std::collections::HashMap::new(),
            transfer_id: "t1".to_string(),
            source_connection_id: "s".to_string(),
            source_database: "db".to_string(),
            source_schema: "public".to_string(),
            source_catalog: None,
            target_connection_id: "t".to_string(),
            target_database: "db".to_string(),
            target_schema: "public".to_string(),
            target_catalog: None,
            tables: vec!["a".to_string()],
            create_table: true,
            content: TransferContent::StructureOnly,
            objects: Some(vec![TransferObjectSelection {
                object_type: TransferObjectKind::View,
                names: vec!["v1".to_string()],
            }]),
            mode: TransferMode::Append,
            target_table_name_case: TransferTableNameCase::Preserve,
            quote_target_column_names: true,
            ownership_policy: TransferOwnershipPolicy::Preserve,
            batch_size: 1000,
            drop_target_before_create: false,
            drop_target_confirmed: false,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["content"], "structureOnly");
        assert_eq!(json["objects"][0]["objectType"], "VIEW");
        assert_eq!(json["objects"][0]["names"][0], "v1");
        assert_eq!(json["quoteTargetColumnNames"], true);
    }

    mod transfer_family_tests {
        use super::*;

        #[test]
        fn object_kind_fallback_distinguishes_unspecified_from_explicit_empty() {
            let view = TransferObjectKind::View;
            assert!(object_kind_selected_or_defaulted(None, &view));
            assert!(!object_kind_selected_or_defaulted(Some(&[]), &view));
            assert!(object_kind_selected_or_defaulted(
                Some(&[TransferObjectSelection { object_type: view, names: vec!["v1".into()] }]),
                &view,
            ));
        }
    }

    mod transfer_validation_tests {
        use super::*;

        #[test]
        fn validates_content_and_object_rules() {
            let base = TransferRequest {
                table_filters: std::collections::HashMap::new(),
                transfer_id: "t".into(),
                source_connection_id: "s".into(),
                source_database: "db".into(),
                source_schema: "public".into(),
                source_catalog: None,
                target_connection_id: "t".into(),
                target_database: "db".into(),
                target_schema: "public".into(),
                target_catalog: None,
                tables: vec!["a".into()],
                create_table: true,
                mode: TransferMode::Append,
                target_table_name_case: TransferTableNameCase::Preserve,
                quote_target_column_names: true,
                ownership_policy: TransferOwnershipPolicy::Preserve,
                batch_size: 1000,
                content: TransferContent::DataOnly,
                objects: Some(Vec::new()),
                drop_target_before_create: false,
                drop_target_confirmed: false,
            };
            assert!(validate_transfer_request(&base).is_ok());

            let with_objects = TransferRequest {
                objects: Some(vec![TransferObjectSelection {
                    object_type: TransferObjectKind::View,
                    names: vec!["v".into()],
                }]),
                ..base.clone()
            };
            // DataOnly + objects → error
            assert!(validate_transfer_request(&with_objects).is_err());

            let structure_only = TransferRequest { content: TransferContent::StructureOnly, ..base.clone() };
            assert!(validate_transfer_request(&structure_only).is_ok());
        }

        #[test]
        fn rejects_drop_target_before_create_with_data_only() {
            let base = TransferRequest {
                table_filters: std::collections::HashMap::new(),
                transfer_id: "t".into(),
                source_connection_id: "s".into(),
                source_database: "db".into(),
                source_schema: "public".into(),
                source_catalog: None,
                target_connection_id: "t".into(),
                target_database: "db".into(),
                target_schema: "public".into(),
                target_catalog: None,
                tables: vec!["orders".into()],
                create_table: true,
                mode: TransferMode::Append,
                target_table_name_case: TransferTableNameCase::Preserve,
                quote_target_column_names: true,
                ownership_policy: TransferOwnershipPolicy::Preserve,
                batch_size: 1000,
                content: TransferContent::StructureAndData,
                objects: Some(Vec::new()),
                drop_target_before_create: false,
                drop_target_confirmed: false,
            };

            // drop_target_before_create=false → valid
            assert!(validate_transfer_request(&base).is_ok());

            // drop_target_before_create=true + StructureAndData → valid
            let with_drop = TransferRequest { drop_target_before_create: true, ..base.clone() };
            assert!(validate_transfer_request(&with_drop).is_ok());

            // drop_target_before_create=true + StructureOnly → valid
            let structure_only = TransferRequest {
                content: TransferContent::StructureOnly,
                drop_target_before_create: true,
                ..base.clone()
            };
            assert!(validate_transfer_request(&structure_only).is_ok());

            // drop_target_before_create=true + DataOnly → error
            let data_only =
                TransferRequest { content: TransferContent::DataOnly, drop_target_before_create: true, ..base.clone() };
            let err = validate_transfer_request(&data_only).unwrap_err();
            assert!(
                err.contains("drop_target_before_create") && err.contains("DataOnly"),
                "expected error to mention drop_target_before_create and DataOnly, got: {err}"
            );
        }
    }

    mod transfer_existence_tests {
        use super::*;
    }

    mod transfer_cross_family_tests {
        use super::*;
        use TransferObjectKind::*;
    }
    mod transfer_mysql_ddl_tests {
        use super::*;

        #[test]
        fn strips_mysql_definer_clauses() {
            assert_eq!(strip_mysql_definer("CREATE DEFINER=`u`@`%` VIEW v AS SELECT 1"), "CREATE VIEW v AS SELECT 1");
            assert_eq!(
                strip_mysql_definer("CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`localhost` VIEW v AS SELECT 1"),
                "CREATE ALGORITHM=UNDEFINED VIEW v AS SELECT 1"
            );
            assert_eq!(strip_mysql_definer("CREATE PROCEDURE p() BEGIN END"), "CREATE PROCEDURE p() BEGIN END");
        }

        #[test]
        fn rewrites_mysql_schema_qualifiers() {
            assert_eq!(
                rewrite_mysql_schema_qualifier("CREATE VIEW `src`.`v` AS SELECT 1 FROM `src`.`t`", "src", "dst"),
                "CREATE VIEW `dst`.`v` AS SELECT 1 FROM `dst`.`t`"
            );
        }

        #[test]
        fn assembles_mysql_trigger_and_event_ddl() {
            let trigger = mysql_trigger_ddl("shop", "trg1", "BEFORE", "INSERT", "users", "SET NEW.updated = NOW()");
            assert_eq!(
                trigger,
                "CREATE TRIGGER `trg1` BEFORE INSERT ON `shop`.`users` FOR EACH ROW SET NEW.updated = NOW()"
            );
            let event = mysql_event_ddl("shop", "ev1", "ENABLE", "EVERY 1 DAY", "DELETE FROM logs");
            assert_eq!(event, "CREATE EVENT `ev1` ON SCHEDULE EVERY 1 DAY ENABLE DO DELETE FROM logs");
        }

        #[test]
        fn strips_inline_foreign_keys_from_mysql_create_table() {
            let ddl = "CREATE TABLE `child` (\n  `id` int NOT NULL,\n  `parent_id` int NOT NULL,\n  PRIMARY KEY (`id`),\n  KEY `fk_child_parent` (`parent_id`),\n  CONSTRAINT `fk_child_parent` FOREIGN KEY (`parent_id`) REFERENCES `parent` (`id`)\n) ENGINE=InnoDB";

            let stripped = strip_inline_foreign_key_constraint_lines(ddl);

            assert!(!stripped.to_ascii_uppercase().contains("FOREIGN KEY"), "{stripped}");
            assert!(stripped.contains("KEY `fk_child_parent` (`parent_id`)"), "{stripped}");
        }

        #[test]
        fn keeps_columns_whose_comment_mentions_foreign_key() {
            // Regression for #7660: the deferred-FK DDL rewrite used to drop any
            // line containing " FOREIGN KEY ", so column definitions whose
            // COMMENT text merely mentions the words silently vanished from the
            // created target table.
            let ddl = "CREATE TABLE `title_task` (\n  `id` varchar(32) NOT NULL,\n  `review_result` varchar(100) DEFAULT NULL,\n  `review_mode` tinyint NOT NULL DEFAULT '0' COMMENT 'foreign key of review flow',\n  `score_snapshot_json` text COMMENT 'snapshot json, see foreign key docs',\n  `task_status` int DEFAULT '0',\n  PRIMARY KEY (`id`),\n  KEY `idx_task_status` (`task_status`),\n  CONSTRAINT `fk_title_task_org` FOREIGN KEY (`org_id`) REFERENCES `org` (`id`)\n) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4";

            let stripped = strip_inline_foreign_key_constraint_lines(ddl);

            assert!(stripped.contains("`review_mode` tinyint NOT NULL DEFAULT '0'"), "{stripped}");
            assert!(stripped.contains("`score_snapshot_json` text"), "{stripped}");
            assert!(!stripped.contains("FOREIGN KEY"), "{stripped}");
            assert!(stripped.contains("KEY `idx_task_status` (`task_status`)"), "{stripped}");
        }

        #[test]
        fn strips_only_foreign_key_constraint_lines_regardless_of_other_mentions() {
            // A CHECK constraint whose expression mentions the words stays; a
            // nameless inline `FOREIGN KEY (` line (defensive: some MySQL
            // dialects emit it) is still stripped.
            let ddl = "CREATE TABLE `t` (\n  `id` int NOT NULL,\n  `kind` varchar(10) NOT NULL,\n  CONSTRAINT `chk_kind` CHECK (`kind` <> 'foreign key'),\n  FOREIGN KEY (`id`) REFERENCES `parent` (`id`)\n) ENGINE=InnoDB";

            let stripped = strip_inline_foreign_key_constraint_lines(ddl);

            assert!(stripped.contains("CONSTRAINT `chk_kind` CHECK (`kind` <> 'foreign key')"), "{stripped}");
            assert!(!stripped.contains("REFERENCES"), "{stripped}");
        }

        #[test]
        fn generates_deferred_mysql_foreign_key_alter_statements() {
            let foreign_keys = vec![db::ForeignKeyInfo {
                name: "fk_child_parent".to_string(),
                column: "parent_id".to_string(),
                ref_schema: None,
                ref_table: "parent".to_string(),
                ref_column: "id".to_string(),
                on_update: None,
                on_delete: Some("CASCADE".to_string()),
            }];
            let request = test_transfer_request(vec!["child", "parent"]);

            let statements =
                generate_mysql_foreign_key_alter_statements(&foreign_keys, &request, "child", &DatabaseType::Mysql);

            assert_eq!(
                statements,
                vec![
                    "ALTER TABLE `child` ADD CONSTRAINT `fk_child_parent` FOREIGN KEY (`parent_id`) REFERENCES `parent` (`id`) ON DELETE CASCADE"
                        .to_string()
                ]
            );
        }

        #[test]
        fn same_database_mysql_foreign_key_applies_target_table_name_rules() {
            // test_transfer_request's source_schema is "source_schema" — the
            // referenced table's ref_schema matching that exactly is what marks it
            // as part of this transfer batch (see mysql_table_metadata_catalog-style
            // fallback in generate_mysql_foreign_key_alter_statements).
            let foreign_keys = vec![db::ForeignKeyInfo {
                name: "fk_child_parent".to_string(),
                column: "parent_id".to_string(),
                ref_schema: Some("source_schema".to_string()),
                ref_table: "Parent".to_string(),
                ref_column: "id".to_string(),
                on_update: None,
                on_delete: None,
            }];
            let mut request = test_transfer_request(vec!["child", "Parent"]);
            request.target_table_name_case = TransferTableNameCase::Lower;

            let statements =
                generate_mysql_foreign_key_alter_statements(&foreign_keys, &request, "child", &DatabaseType::Mysql);

            // Referenced table is in-batch, so its target-side name (lowercased per
            // target_table_name_case) is used, unqualified — it lives in whatever
            // database this ALTER TABLE already runs against.
            assert_eq!(
                statements,
                vec![
                    "ALTER TABLE `child` ADD CONSTRAINT `fk_child_parent` FOREIGN KEY (`parent_id`) REFERENCES `parent` (`id`)"
                        .to_string()
                ]
            );
        }

        #[test]
        fn cross_database_mysql_foreign_key_keeps_original_schema_and_name() {
            let foreign_keys = vec![db::ForeignKeyInfo {
                name: "fk_child_parent".to_string(),
                column: "parent_id".to_string(),
                ref_schema: Some("other_db".to_string()),
                ref_table: "Parent".to_string(),
                ref_column: "id".to_string(),
                on_update: None,
                on_delete: None,
            }];
            let mut request = test_transfer_request(vec!["child"]);
            // Even with a target-side rename policy configured, a table outside the
            // transfer batch (different database) must not be renamed — we never
            // created or renamed it, so it must be referenced exactly as it exists
            // on the target server.
            request.target_table_name_case = TransferTableNameCase::Lower;

            let statements =
                generate_mysql_foreign_key_alter_statements(&foreign_keys, &request, "child", &DatabaseType::Mysql);

            assert_eq!(
                statements,
                vec![
                    "ALTER TABLE `child` ADD CONSTRAINT `fk_child_parent` FOREIGN KEY (`parent_id`) REFERENCES `other_db`.`Parent` (`id`)"
                        .to_string()
                ]
            );
        }

        #[test]
        fn group_foreign_keys_by_constraint_name_preserves_first_seen_order() {
            let foreign_keys = vec![
                db::ForeignKeyInfo {
                    name: "fk_b".to_string(),
                    column: "b1".to_string(),
                    ref_schema: None,
                    ref_table: "t2".to_string(),
                    ref_column: "id".to_string(),
                    on_update: None,
                    on_delete: None,
                },
                db::ForeignKeyInfo {
                    name: "fk_a".to_string(),
                    column: "a1".to_string(),
                    ref_schema: None,
                    ref_table: "t3".to_string(),
                    ref_column: "id".to_string(),
                    on_update: None,
                    on_delete: None,
                },
                // Second column of the same multi-column fk_a constraint — must be
                // grouped with the first, not treated as a new constraint.
                db::ForeignKeyInfo {
                    name: "fk_a".to_string(),
                    column: "a2".to_string(),
                    ref_schema: None,
                    ref_table: "t3".to_string(),
                    ref_column: "id2".to_string(),
                    on_update: None,
                    on_delete: None,
                },
            ];

            let grouped = group_foreign_keys_by_constraint_name(&foreign_keys);

            assert_eq!(grouped.len(), 2);
            assert_eq!(grouped[0].0, "fk_b");
            assert_eq!(grouped[0].1.len(), 1);
            assert_eq!(grouped[1].0, "fk_a");
            assert_eq!(grouped[1].1.len(), 2);
            assert_eq!(grouped[1].1[0].column, "a1");
            assert_eq!(grouped[1].1[1].column, "a2");
        }

        #[test]
        fn foreign_key_cycle_survives_dependency_sort_via_deferred_alters() {
            // A <-> B mutual reference has no valid CREATE TABLE order at all — the
            // dependency sort can only push both to the back (see
            // table_dependency_sort_ignores_duplicates_and_out_of_scope_tables-style
            // cycle handling); the transfer must not depend on ordering to succeed,
            // only on foreign keys being added after every table exists.
            let tables = vec!["b_department".to_string(), "c_employee".to_string()];
            let dependencies = vec![
                ("b_department".to_string(), "c_employee".to_string()),
                ("c_employee".to_string(), "b_department".to_string()),
            ];

            let sorted = sort_table_names_by_dependencies(&tables, &dependencies, true);

            // Whatever order the sort settles on, both tables are present — proving
            // table creation alone can proceed regardless of the cycle, which is the
            // property the deferred-ALTER approach relies on.
            assert_eq!(sorted.len(), 2);
            assert!(sorted.contains(&"b_department".to_string()));
            assert!(sorted.contains(&"c_employee".to_string()));
        }
    }

    mod transfer_sqlserver_source_tests {
        use super::*;
        use serde_json::json;
    }
    mod transfer_mysql_source_tests {
        use super::*;

        #[test]
        fn builds_mysql_object_source_queries() {
            let sql = mysql_object_source_query(&TransferObjectKind::View, "shop", "v1").unwrap();
            assert!(sql.contains("SHOW CREATE VIEW"));
            let sql = mysql_object_source_query(&TransferObjectKind::Trigger, "shop", "trg1").unwrap();
            assert!(sql.contains("information_schema.TRIGGERS"));
            assert!(sql.contains("TRIGGER_NAME = 'trg1'"));
            let sql = mysql_object_source_query(&TransferObjectKind::Event, "shop", "ev1").unwrap();
            assert!(sql.contains("information_schema.EVENTS"));
            assert!(sql.contains("EVENT_NAME = 'ev1'"));
        }

        #[test]
        fn extracts_mysql_object_ddl_from_result() {
            let view = mysql_object_ddl_from_result(
                &TransferObjectKind::View,
                "shop",
                &[vec![serde_json::json!("v1"), serde_json::json!("CREATE VIEW `shop`.`v1` AS SELECT 1")]],
            )
            .unwrap();
            assert_eq!(view, "CREATE VIEW `shop`.`v1` AS SELECT 1");

            let routine = mysql_object_ddl_from_result(
                &TransferObjectKind::Procedure,
                "shop",
                &[vec![
                    serde_json::json!("p"),
                    serde_json::json!("PROCEDURE"),
                    serde_json::json!("CREATE PROCEDURE p() BEGIN END"),
                ]],
            )
            .unwrap();
            assert_eq!(routine, "CREATE PROCEDURE p() BEGIN END");

            let trigger = mysql_object_ddl_from_result(
                &TransferObjectKind::Trigger,
                "shop",
                &[vec![
                    serde_json::json!("trg1"),
                    serde_json::json!("BEFORE"),
                    serde_json::json!("INSERT"),
                    serde_json::json!("users"),
                    serde_json::json!("SET NEW.updated = NOW()"),
                ]],
            )
            .unwrap();
            assert_eq!(
                trigger,
                "CREATE TRIGGER `trg1` BEFORE INSERT ON `shop`.`users` FOR EACH ROW SET NEW.updated = NOW()"
            );

            let event = mysql_object_ddl_from_result(
                &TransferObjectKind::Event,
                "shop",
                &[vec![
                    serde_json::json!("ev1"),
                    serde_json::json!("ENABLE"),
                    serde_json::json!("2026-01-01"),
                    serde_json::json!("1"),
                    serde_json::json!("DAY"),
                    serde_json::json!("DELETE FROM logs"),
                ]],
            )
            .unwrap();
            assert_eq!(event, "CREATE EVENT `ev1` ON SCHEDULE EVERY 1 DAY ENABLE DO DELETE FROM logs");
        }
    }

    mod transfer_oracle_source_tests {
        use super::*;
    }

    mod transfer_executor_tests {
        use super::*;

        #[test]
        fn orders_object_selections_by_dependency() {
            let kinds = vec![
                TransferObjectKind::Trigger,
                TransferObjectKind::View,
                TransferObjectKind::Sequence,
                TransferObjectKind::Event,
                TransferObjectKind::Procedure,
                TransferObjectKind::Function,
            ];
            let ordered = ordered_transfer_object_kinds(kinds);
            assert_eq!(ordered[0], TransferObjectKind::Sequence);
            assert_eq!(ordered[1], TransferObjectKind::View);
            assert_eq!(ordered[2], TransferObjectKind::Function);
            assert_eq!(ordered[3], TransferObjectKind::Procedure);
            assert_eq!(ordered[4], TransferObjectKind::Trigger);
            assert_eq!(ordered[5], TransferObjectKind::Event);
        }
    }

    mod transfer_mysql_executor_tests {
        use super::*;

        #[test]
        fn extracts_selected_names_by_kind() {
            let selections = vec![
                TransferObjectSelection { object_type: TransferObjectKind::View, names: vec!["v1".into()] },
                TransferObjectSelection { object_type: TransferObjectKind::View, names: vec!["v2".into()] },
                TransferObjectSelection { object_type: TransferObjectKind::Trigger, names: vec!["t1".into()] },
            ];
            let views = selected_object_names(&selections, &TransferObjectKind::View);
            assert_eq!(views, vec!["v1", "v2"]);
            assert!(selected_object_names(&selections, &TransferObjectKind::Event).is_empty());
        }
    }

    mod transfer_oracle_executor_tests {
        use super::*;
    }

    mod transfer_postgres_executor_tests {
        use super::*;
    }

    mod transfer_content_mode_tests {
        use super::*;

        #[test]
        fn structure_only_skips_data_steps() {
            assert!(should_copy_data(&TransferContent::StructureAndData));
            assert!(should_copy_data(&TransferContent::DataOnly));
            assert!(!should_copy_data(&TransferContent::StructureOnly));
        }
    }
    fn test_transfer_request(tables: Vec<&str>) -> TransferRequest {
        TransferRequest {
            table_filters: std::collections::HashMap::new(),
            transfer_id: "transfer-1".to_string(),
            source_connection_id: "source".to_string(),
            source_database: "source_db".to_string(),
            source_schema: "source_schema".to_string(),
            source_catalog: None,
            target_connection_id: "target".to_string(),
            target_database: "target_db".to_string(),
            target_schema: "target_schema".to_string(),
            target_catalog: None,
            tables: tables.into_iter().map(str::to_string).collect(),
            create_table: true,
            content: TransferContent::default(),
            objects: Some(Vec::new()),
            mode: TransferMode::Append,
            target_table_name_case: TransferTableNameCase::Preserve,
            quote_target_column_names: true,
            ownership_policy: TransferOwnershipPolicy::Preserve,
            batch_size: 1000,
            drop_target_before_create: false,
            drop_target_confirmed: false,
        }
    }

    #[test]
    fn transfer_request_defaults_preserve_table_name_case() {
        let request: TransferRequest = serde_json::from_value(json!({
            "transferId": "transfer-1",
            "sourceConnectionId": "source",
            "sourceDatabase": "source_db",
            "sourceSchema": "source_schema",
            "targetConnectionId": "target",
            "targetDatabase": "target_db",
            "targetSchema": "target_schema",
            "tables": ["ORDERS"],
            "createTable": true,
            "mode": "append",
            "batchSize": 1000
        }))
        .unwrap();

        assert_eq!(request.target_table_name_case, TransferTableNameCase::Preserve);
        assert_eq!(request.target_table_name("ORDERS"), "ORDERS");
    }

    #[test]
    fn transfer_existing_target_table_name_prefers_exact_case() {
        let tables = vec![test_table("orders"), test_table("Orders")];

        assert_eq!(existing_transfer_target_table_name("Orders", &tables, true), Some("Orders".to_string()));
    }

    #[test]
    fn transfer_existing_target_table_name_respects_case_sensitive_targets() {
        let tables = vec![test_table("Orders")];

        assert_eq!(existing_transfer_target_table_name("orders", &tables, false), None);
        assert_eq!(existing_transfer_target_table_name("orders", &tables, true), Some("Orders".to_string()));
    }

    #[test]
    fn transfer_existing_target_table_name_ignores_contains_matches() {
        let tables = vec![test_table("archived_orders"), test_table("orders_backup")];

        assert_eq!(existing_transfer_target_table_name("orders", &tables, true), None);
    }

    #[test]
    fn parses_mysql_lower_case_table_names_values() {
        let string_result = test_query_result(vec![vec![json!("lower_case_table_names"), json!("2")]]);
        let numeric_result = test_query_result(vec![vec![json!("lower_case_table_names"), json!(1)]]);
        let empty_result = test_query_result(Vec::new());

        assert_eq!(mysql_lower_case_table_names_from_result(&string_result), Some(2));
        assert_eq!(mysql_lower_case_table_names_from_result(&numeric_result), Some(1));
        assert_eq!(mysql_lower_case_table_names_from_result(&empty_result), None);
    }

    #[test]
    fn transfer_table_name_case_transforms_target_names() {
        let mut request = test_transfer_request(vec!["ORDERS"]);
        request.target_table_name_case = TransferTableNameCase::Lower;
        assert_eq!(request.target_table_name("ORDERS"), "orders");

        request.target_table_name_case = TransferTableNameCase::Upper;
        assert_eq!(request.target_table_name("orders"), "ORDERS");
    }

    #[test]
    fn transfer_table_name_case_detects_target_collisions() {
        let mut request = test_transfer_request(vec!["ORDERS", "orders"]);
        request.target_table_name_case = TransferTableNameCase::Lower;

        let error = validate_transfer_target_table_names(&request).unwrap_err();
        assert!(error.contains("both map to 'orders'"));
    }

    #[test]
    fn detects_identity_extras_for_selected_columns() {
        assert!(selected_columns_include_identity_extras(
            &[String::from("id"), String::from("name")],
            &[Some(String::from("identity")), None],
        ));
        assert!(selected_columns_include_identity_extras(
            &[String::from("id")],
            &[Some(String::from("auto_increment"))],
        ));
        assert!(!selected_columns_include_identity_extras(
            &[String::from("name")],
            &[None, Some(String::from("identity"))],
        ));
    }

    #[test]
    fn detects_selected_identity_columns_from_target_metadata() {
        let target_columns = vec![
            db::ColumnInfo { extra: Some("identity".to_string()), ..test_column("ID", "INT") },
            test_column("NAME", "VARCHAR(20)"),
        ];

        assert!(selected_columns_include_identity_columns(&[String::from("id")], &target_columns));
        assert!(!selected_columns_include_identity_columns(&[String::from("name")], &target_columns));
    }

    #[test]
    fn mysql_generated_only_transfer_default_rows_use_existing_batch_limits() {
        let rows = vec![Vec::new(); 5];
        let batches = generate_insert_typed_sql_batches(
            &[],
            &[],
            &rows,
            "all`generated",
            "target",
            &DatabaseType::Mysql,
            None,
            SqlBatchLimits { max_rows: 2, target_sql_bytes: 1024, hard_sql_bytes: None },
        )
        .unwrap();
        assert_eq!(
            batches,
            vec![
                ("INSERT INTO `all``generated` () VALUES\n(),\n()".into(), 2),
                ("INSERT INTO `all``generated` () VALUES\n(),\n()".into(), 2),
                ("INSERT INTO `all``generated` () VALUES\n()".into(), 1),
            ]
        );
        for mode in [TransferMode::Append, TransferMode::Overwrite] {
            assert!(generate_transfer_write_sql_batches(
                &mode,
                &[],
                &[],
                &[],
                "empty",
                "target",
                &DatabaseType::Mysql,
                &[],
                None,
                false,
                false,
            )
            .unwrap()
            .is_empty());
        }
    }

    #[test]
    fn mysql_writable_transfer_columns_skip_only_generated_columns() {
        let columns = vec![
            test_column("id", "int"),
            db::ColumnInfo { extra: Some("DEFAULT_GENERATED".to_string()), ..test_column("created_at", "timestamp") },
            db::ColumnInfo { extra: Some("auto_increment".to_string()), ..test_column("sequence_id", "bigint") },
            db::ColumnInfo {
                extra: Some("VIRTUAL GENERATED".to_string()),
                ..test_column("virtual_total", "decimal(10,2)")
            },
            db::ColumnInfo { extra: Some("stored generated".to_string()), ..test_column("stored_hash", "varchar(64)") },
            db::ColumnInfo {
                extra: Some("PERSISTENT GENERATED".to_string()),
                ..test_column("persistent_total", "decimal(10,2)")
            },
            db::ColumnInfo { extra: Some("GENERATED ALWAYS".to_string()), ..test_column("explicit_generated", "int") },
            db::ColumnInfo {
                extra: Some("on update CURRENT_TIMESTAMP".to_string()),
                ..test_column("updated_at", "timestamp")
            },
        ];

        let writable = writable_transfer_columns(&columns, &DatabaseType::Mysql, &DatabaseType::Mysql);

        assert_eq!(
            writable.iter().map(|column| column.name.as_str()).collect::<Vec<_>>(),
            vec!["id", "created_at", "sequence_id", "updated_at"]
        );
        assert_eq!(columns.len(), 8, "DDL metadata must retain generated columns");
    }

    #[test]
    fn transfer_target_column_validation_reports_columns_absent_from_target() {
        let target_columns = vec![test_column("id", "int"), test_column("name", "varchar(32)")];
        let col_names = vec!["id".to_string(), "name".to_string(), "extra_col".to_string()];

        assert_eq!(
            missing_transfer_target_columns(&target_columns, &col_names, &DatabaseType::Mysql, true),
            vec!["extra_col".to_string()]
        );
    }

    #[test]
    fn write_column_names_reuse_preexisting_target_case() {
        // MySQL source columns are lowercase while the preexisting Oracle target
        // declares them uppercase, so the write SQL must address ID/NAME (#9320).
        let target_columns = vec![test_column("ID", "NUMBER"), test_column("NAME", "VARCHAR2")];
        let col_names = vec!["id".to_string(), "name".to_string()];

        assert_eq!(
            resolve_transfer_target_column_names(&col_names, &target_columns),
            vec!["ID".to_string(), "NAME".to_string()]
        );
    }

    #[test]
    fn write_column_names_prefer_exact_target_match() {
        // A case-sensitive target can declare both `id` and `ID`; the exact match
        // wins so DBX keeps addressing the column the source name refers to.
        let target_columns = vec![test_column("ID", "NUMBER"), test_column("id", "NUMBER")];
        let col_names = vec!["id".to_string(), "ID".to_string()];

        assert_eq!(
            resolve_transfer_target_column_names(&col_names, &target_columns),
            vec!["id".to_string(), "ID".to_string()]
        );
    }

    #[test]
    fn write_column_names_keep_source_name_without_target_match() {
        let target_columns = vec![test_column("ID", "NUMBER")];
        let col_names = vec!["id".to_string(), "missing".to_string()];

        assert_eq!(
            resolve_transfer_target_column_names(&col_names, &target_columns),
            vec!["ID".to_string(), "missing".to_string()]
        );
    }

    #[test]
    fn transfer_target_column_validation_rejects_required_unmapped_columns() {
        let target_columns = vec![
            test_column("id", "int"),
            db::ColumnInfo { is_nullable: false, ..test_column("required_code", "varchar(32)") },
        ];
        let col_names = vec!["id".to_string()];

        assert_eq!(
            required_unmapped_transfer_target_columns(&target_columns, &col_names, &DatabaseType::Mysql, true),
            vec!["required_code".to_string()]
        );
    }

    #[test]
    fn mysql_create_table_includes_column_comments() {
        let cols = vec![
            db::ColumnInfo { comment: Some("用户ID".to_string()), is_primary_key: true, ..test_column("id", "int") },
            db::ColumnInfo {
                comment: Some("用户姓名".to_string()),
                is_nullable: false,
                ..test_column("name", "VARCHAR(100)")
            },
            db::ColumnInfo { comment: None, ..test_column("age", "int") },
        ];

        let ddl =
            generate_create_table_ddl(&cols, "users", "", "", &DatabaseType::Mysql, &DatabaseType::Mysql, None, None);

        assert!(ddl.contains("COMMENT '用户ID'"));
        assert!(ddl.contains("COMMENT '用户姓名'"));
        assert!(!ddl.contains("`age` INT COMMENT")); // no comment for age
        assert!(ddl.contains("`name` VARCHAR(100) NOT NULL COMMENT '用户姓名'"));
        assert!(ddl.contains("PRIMARY KEY (`id`)"));
    }

    #[test]
    fn mysql_create_table_includes_table_comment() {
        let cols = vec![db::ColumnInfo { is_primary_key: true, ..test_column("id", "int") }];

        let ddl = generate_create_table_ddl(
            &cols,
            "users",
            "",
            "",
            &DatabaseType::Mysql,
            &DatabaseType::Mysql,
            Some("用户表"),
            None,
        );

        assert!(ddl.contains(") COMMENT='用户表'"));
    }

    #[test]
    fn mysql_comment_ddl_stays_empty_inline_only() {
        let cols = vec![db::ColumnInfo { comment: Some("主键".to_string()), ..test_column("id", "int") }];

        let stmts = generate_comment_ddl(&cols, "items", "db", &DatabaseType::Mysql, Some("项目表"));

        assert!(stmts.is_empty());
    }

    #[test]
    fn transfer_create_table_result_treats_existing_table_as_preexisting() {
        assert!(!transfer_create_table_created(
            Err("ERROR: relation \"items\" already exists (SQLSTATE 42P07)".to_string()),
            "create"
        )
        .unwrap());
        assert!(!transfer_create_table_created(Err("错误: 关系 \"items\" 已经存在".to_string()), "create").unwrap());
        assert!(transfer_create_table_created(Ok(()), "create").unwrap());
        assert_eq!(
            transfer_create_table_created(Err("permission denied for schema public".to_string()), "create")
                .unwrap_err(),
            "create: permission denied for schema public"
        );
    }

    #[test]
    fn mysql_reused_ddl_renames_create_table_header_for_case_conversion() {
        let ddl = "CREATE TABLE `orders_plain` (\n  `id` int NOT NULL,\n  PRIMARY KEY (`id`)\n) \
ENGINE=InnoDB DEFAULT CHARSET=utf8mb4\nPARTITION BY RANGE (TO_DAYS(`created_day`))\n(\
PARTITION p_old VALUES LESS THAN (TO_DAYS('2026-01-01')))";

        let rewritten = rewrite_transfer_source_table_ddl(
            ddl,
            "dbx_src",
            "dbx_dst",
            &DatabaseType::Mysql,
            &DatabaseType::Mysql,
            "orders_plain",
            "ORDERS_PLAIN",
        )
        .unwrap();

        assert!(rewritten.starts_with("CREATE TABLE `ORDERS_PLAIN` ("));
        assert!(rewritten.contains("PARTITION BY RANGE (TO_DAYS(`created_day`))"));
        assert!(rewritten.contains("ENGINE=InnoDB"));
        assert!(!rewritten.contains("`orders_plain`"));
    }

    #[test]
    fn mysql_reused_ddl_keeps_identity_when_target_name_matches() {
        let ddl = "CREATE TABLE `orders` (`id` int)";
        assert_eq!(
            rewrite_transfer_source_table_ddl(
                ddl,
                "s",
                "t",
                &DatabaseType::Mysql,
                &DatabaseType::Mysql,
                "orders",
                "orders"
            ),
            Some(ddl.to_string())
        );
    }

    #[test]
    fn mysql_create_table_header_rename_handles_statement_shapes() {
        let rewrite = |ddl: &str| rewrite_mysql_create_table_name(ddl, "TARGET");

        // SHOW CREATE TABLE form with parenthesized column list.
        assert_eq!(
            rewrite("CREATE TABLE `src` (\n  `id` int\n)"),
            Some("CREATE TABLE `TARGET` (\n  `id` int\n)".to_string())
        );
        // No space before the column list.
        assert_eq!(rewrite("create table `src`(`id` int)"), Some("create table `TARGET`(`id` int)".to_string()));
        // TEMPORARY + IF NOT EXISTS.
        assert_eq!(
            rewrite("CREATE TEMPORARY TABLE IF NOT EXISTS `src` (`id` int)"),
            Some("CREATE TEMPORARY TABLE IF NOT EXISTS `TARGET` (`id` int)".to_string())
        );
        // Qualified name: only the table segment is replaced.
        assert_eq!(
            rewrite("CREATE TABLE `prod_db`.`src` (`id` int)"),
            Some("CREATE TABLE `prod_db`.`TARGET` (`id` int)".to_string())
        );
        // Escaped backticks inside the source identifier.
        assert_eq!(rewrite("CREATE TABLE `od``d` (`id` int)"), Some("CREATE TABLE `TARGET` (`id` int)".to_string()));
        // Backtick inside the target identifier is escaped.
        assert_eq!(
            rewrite_mysql_create_table_name("CREATE TABLE `src` (`id` int)", "ta`rget"),
            Some("CREATE TABLE `ta``rget` (`id` int)".to_string())
        );
        // Anything that is not a plain CREATE TABLE head is rejected.
        assert_eq!(rewrite("ALTER TABLE `src` ADD COLUMN `x` int"), None);
        assert_eq!(rewrite("CREATE TABLE `src` LIKE `other`"), None);
        assert_eq!(rewrite("CREATE TABLE `src`"), None);
        assert_eq!(rewrite("CREATE TABLE `unterminated (`id` int)"), None);
        assert_eq!(rewrite(""), None);
    }

    #[test]
    fn mysql_transfer_collation_recovery_only_reads_ddl_code() {
        let ddl = r#"CREATE TABLE `COLLATE utf8mb4_identifier_ci` (
  `id` bigint NOT NULL,
  `note` varchar(255) COMMENT 'COLLATE utf8mb4_literal_ci',
  `name` varchar(64) COLLATE utf8mb4_0900_ai_ci,
  `legacy` varchar(64) collate = utf8mb4_unicode_ci
) DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci
/* COLLATE utf8mb4_comment_ci */"#;

        assert_eq!(
            mysql_ddl_collation_names(ddl),
            vec!["utf8mb4_0900_ai_ci".to_string(), "utf8mb4_unicode_ci".to_string()]
        );
    }

    #[test]
    fn mysql_transfer_collation_recovery_removes_only_unsupported_clauses() {
        let ddl = r#"CREATE TABLE `items` (
  `name` varchar(64) COLLATE utf8mb4_0900_ai_ci COMMENT 'COLLATE utf8mb4_0900_ai_ci',
  `legacy` varchar(64) COLLATE utf8mb4_unicode_ci
) DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci COMMENT='items'"#;
        let supported = HashSet::from(["utf8mb4_unicode_ci".to_string()]);

        let rewritten = remove_unsupported_mysql_collations(ddl, &supported);

        assert!(!rewritten.contains("varchar(64) COLLATE utf8mb4_0900_ai_ci"));
        assert!(!rewritten.contains("utf8mb4 COLLATE=utf8mb4_0900_ai_ci"));
        assert!(rewritten.contains("COLLATE utf8mb4_unicode_ci"));
        assert!(rewritten.contains("COMMENT 'COLLATE utf8mb4_0900_ai_ci'"));
        assert!(rewritten.contains("DEFAULT CHARSET=utf8mb4"));
        assert!(rewritten.contains("COMMENT='items'"));
    }

    #[test]
    fn mysql_unique_columns_do_not_become_transfer_keys() {
        let columns = vec![db::ColumnInfo { is_unique: true, ..test_column("email", "varchar(255)") }];

        assert!(transfer_key_columns(&columns, &DatabaseType::Mysql).is_empty());
    }

    #[test]
    fn keyset_cursor_reads_key_values_from_last_row() {
        let rows = vec![vec![json!(1), json!("a")], vec![json!(2), json!("b")]];

        assert_eq!(keyset_cursor_from_last_row(&rows, &[0]), Some(vec![json!(2)]));
        assert_eq!(keyset_cursor_from_last_row(&rows, &[1]), Some(vec![json!("b")]));
        assert_eq!(keyset_cursor_from_last_row(&rows, &[0, 1]), Some(vec![json!(2), json!("b")]));
        // Missing column index reads as NULL → no keyset cursor.
        assert_eq!(keyset_cursor_from_last_row(&rows, &[5]), None);
        assert_eq!(keyset_cursor_from_last_row(&Vec::new(), &[0]), None);
        let null_rows = vec![vec![json!(1), serde_json::Value::Null]];
        assert_eq!(keyset_cursor_from_last_row(&null_rows, &[1]), None);
    }

    #[test]
    fn advance_keyset_cursor_detects_stall_and_null_fallback() {
        let mut cursor = Vec::new();
        let rows = vec![vec![json!(1)], vec![json!(2)]];

        assert!(matches!(advance_keyset_cursor(&mut cursor, &rows, &[0], "t"), Ok(KeysetAdvance::Advanced)));
        assert_eq!(cursor, vec![json!(2)]);
        // Re-reading the same page must fail instead of looping forever.
        assert!(advance_keyset_cursor(&mut cursor, &rows, &[0], "t").is_err());
        // NULL keys degrade to OFFSET paging.
        let null_rows = vec![vec![serde_json::Value::Null]];
        assert!(matches!(
            advance_keyset_cursor(&mut cursor, &null_rows, &[0], "t"),
            Ok(KeysetAdvance::FallBackToOffset)
        ));
        // Empty pages leave the cursor untouched.
        assert!(matches!(advance_keyset_cursor(&mut cursor, &Vec::new(), &[0], "t"), Ok(KeysetAdvance::Advanced)));
    }

    #[test]
    fn count_copy_text_rows_counts_separators_not_escaped_newlines() {
        // COPY text format escapes newlines inside field values as `\n`, so
        // only raw 0x0A bytes are record separators.
        assert_eq!(count_copy_text_rows(b"id\tname\n1\ttwo\nlines\n"), 3);
        assert_eq!(count_copy_text_rows(b"1\ta\\nb\n2\t\\\\N\n"), 2);
        assert_eq!(count_copy_text_rows(b""), 0);
    }

    #[test]
    fn mysql_insert_normalizes_rfc3339_datetime_strings() {
        let sql = generate_insert_typed(
            &[String::from("insurance_start_time")],
            &[Some(String::from("datetime"))],
            &[vec![json!("2026-05-12T00:00:00+00:00")]],
            "policies",
            "",
            &DatabaseType::Mysql,
            None,
        );

        assert_eq!(sql, "INSERT INTO `policies` (`insurance_start_time`) VALUES\n('2026-05-12 00:00:00')");
    }

    #[test]
    fn mysql_insert_uses_column_types_for_temporal_literals() {
        let sql = generate_insert_typed(
            &[String::from("dt"), String::from("raw_text"), String::from("d"), String::from("t")],
            &[
                Some(String::from("datetime")),
                Some(String::from("varchar(64)")),
                Some(String::from("date")),
                Some(String::from("time")),
            ],
            &[vec![
                json!("2026-05-12T00:00:00+00:00"),
                json!("2026-05-12T00:00:00+00:00"),
                json!("2026-05-12T00:00:00+00:00"),
                json!("2026-05-12T09:30:45+00:00"),
            ]],
            "policies",
            "",
            &DatabaseType::Mysql,
            None,
        );

        assert_eq!(
            sql,
            "INSERT INTO `policies` (`dt`, `raw_text`, `d`, `t`) VALUES\n('2026-05-12 00:00:00', '2026-05-12T00:00:00+00:00', '2026-05-12', '09:30:45')"
        );
    }

    #[test]
    fn mysql_insert_formats_numeric_strings_from_numeric_columns_as_numeric_literals() {
        let sql = generate_insert_typed(
            &[
                String::from("id"),
                String::from("amount"),
                String::from("quantity"),
                String::from("text_id"),
                String::from("bad_number"),
                String::from("missing"),
            ],
            &[
                Some(String::from("bigint(20)")),
                Some(String::from("decimal(10,2)")),
                Some(String::from("int unsigned")),
                Some(String::from("varchar(64)")),
                Some(String::from("bigint(20)")),
                Some(String::from("bigint(20)")),
            ],
            &[vec![
                json!("1234567890123"),
                json!("12.34"),
                json!("42"),
                json!("123"),
                json!("not-a-number"),
                serde_json::Value::Null,
            ]],
            "orders",
            "",
            &DatabaseType::Mysql,
            None,
        );

        assert_eq!(
            sql,
            "INSERT INTO `orders` (`id`, `amount`, `quantity`, `text_id`, `bad_number`, `missing`) VALUES\n(1234567890123, 12.34, 42, '123', 'not-a-number', NULL)"
        );
    }

    #[test]
    fn mysql_upsert_formats_numeric_strings_from_numeric_columns_as_numeric_literals() {
        let sql = generate_upsert_typed(
            &[String::from("id"), String::from("amount")],
            &[Some(String::from("bigint(20)")), Some(String::from("decimal(10,2)"))],
            &[vec![json!("1234567890123"), json!("12.34")]],
            "orders",
            "",
            &DatabaseType::Mysql,
            &[String::from("id")],
            None,
        );

        assert_eq!(
            sql,
            "INSERT INTO `orders` (`id`, `amount`) VALUES\n(1234567890123, 12.34)\nON DUPLICATE KEY UPDATE `amount` = VALUES(`amount`)"
        );
    }

    #[test]
    fn mysql_insert_formats_blob_prefixed_hex_as_binary_literal() {
        let sql = generate_insert_typed(
            &[String::from("id"), String::from("payload"), String::from("empty_blob"), String::from("note")],
            &[
                Some(String::from("int")),
                Some(String::from("MEDIUMBLOB")),
                Some(String::from("blob")),
                Some(String::from("varchar(64)")),
            ],
            &[vec![json!(1), json!("0x0001ABff"), json!("0X"), json!("0x0001ABff")]],
            "files",
            "",
            &DatabaseType::Mysql,
            None,
        );

        assert_eq!(
            sql,
            r#"INSERT INTO `files` (`id`, `payload`, `empty_blob`, `note`) VALUES
(1, 0x0001ABff, X'', '0x0001ABff')"#
        );
    }

    #[test]
    fn mysql_insert_keeps_invalid_blob_hex_as_string_literal() {
        let sql = generate_insert_typed(
            &[String::from("id"), String::from("payload")],
            &[Some(String::from("int")), Some(String::from("mediumblob"))],
            &[vec![json!(1), json!("0xnothex")]],
            "files",
            "",
            &DatabaseType::Mysql,
            None,
        );

        assert_eq!(
            sql,
            r#"INSERT INTO `files` (`id`, `payload`) VALUES
(1, '0xnothex')"#
        );
    }

    #[test]
    fn mysql_insert_keeps_backslash_escape_style() {
        let sql = generate_insert_typed(
            &[String::from("path")],
            &[Some(String::from("varchar(255)"))],
            &[vec![json!(r#"C:\tmp\file.txt"#)]],
            "files",
            "",
            &DatabaseType::Mysql,
            None,
        );

        assert_eq!(
            sql,
            r#"INSERT INTO `files` (`path`) VALUES
('C:\\tmp\\file.txt')"#
        );
    }

    #[test]
    fn transfer_write_sql_batches_split_large_insert_statements() {
        let rows = (0..4).map(|index| vec![json!(index), json!("x".repeat(180 * 1024))]).collect::<Vec<_>>();
        // A MySQL-family target's max_allowed_packet translates into a hard
        // batch cap; the same 4x180 KiB page must split under that cap.
        let statements = generate_transfer_write_sql_batches_with_column_quoting(
            &TransferMode::Append,
            &[String::from("id"), String::from("payload")],
            &[Some(String::from("int")), Some(String::from("text"))],
            &rows,
            "events",
            "",
            &DatabaseType::Mysql,
            &[],
            None,
            false,
            false,
            true,
            Some(512 * 1024),
        )
        .unwrap();

        assert!(statements.len() > 1);
        assert!(statements.iter().all(|sql| sql.starts_with("INSERT INTO `events`")));
    }

    #[test]
    fn mysql_sql_batch_allows_one_row_over_soft_target() {
        let rows = vec![vec![json!("x".repeat(256))]];
        let limits = SqlBatchLimits { max_rows: 100, target_sql_bytes: 128, hard_sql_bytes: Some(1024) };

        let batches = generate_insert_typed_sql_batches(
            &[String::from("payload")],
            &[Some(String::from("text"))],
            &rows,
            "events",
            "",
            &DatabaseType::Mysql,
            None,
            limits,
        )
        .unwrap();

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].1, 1);
    }

    #[test]
    fn mysql_sql_batch_rejects_one_row_over_known_hard_limit() {
        let rows = vec![vec![json!("x".repeat(256))]];
        let limits = SqlBatchLimits { max_rows: 100, target_sql_bytes: 128, hard_sql_bytes: Some(200) };

        let error = generate_insert_typed_sql_batches(
            &[String::from("payload")],
            &[Some(String::from("text"))],
            &rows,
            "events",
            "",
            &DatabaseType::Mysql,
            None,
            limits,
        )
        .unwrap_err();

        assert!(error.contains("row 1"));
        assert!(error.contains("200 byte hard limit"));
    }

    #[test]
    fn transfer_write_sql_batches_keep_existing_upsert_sql_shape() {
        let statements = generate_transfer_write_sql_batches(
            &TransferMode::Upsert,
            &[String::from("id"), String::from("name")],
            &[Some(String::from("int")), Some(String::from("varchar(64)"))],
            &[vec![json!(1), json!("Ada")]],
            "users",
            "",
            &DatabaseType::Mysql,
            &[String::from("id")],
            None,
            false,
            false,
        )
        .unwrap();

        assert_eq!(statements.len(), 1);
        assert!(statements[0].contains("ON DUPLICATE KEY UPDATE"));
    }

    #[test]
    fn mysql_spatial_transfer_reuses_validated_wkb_markers_for_all_modes() {
        let columns = [String::from("id"), String::from("location"), String::from("name")];
        let column_types = [Some(String::from("int")), Some(String::from("point")), Some(String::from("varchar(32)"))];
        let rows = [vec![json!(1), json!("DBX_WKB:4326:0101000000000000000000F03F0000000000000040"), json!("alpha")]];

        for mode in [TransferMode::Append, TransferMode::Overwrite, TransferMode::Upsert] {
            let statements = generate_transfer_write_sql_batches(
                &mode,
                &columns,
                &column_types,
                &rows,
                "places",
                "",
                &DatabaseType::Mysql,
                &[String::from("id")],
                None,
                false,
                true,
            )
            .unwrap();

            assert_eq!(statements.len(), 1);
            assert!(statements[0].contains("ST_GeomFromWKB(0x0101000000000000000000F03F0000000000000040, 4326)"));
            assert!(statements[0].contains("'alpha'"));
            if mode == TransferMode::Upsert {
                assert!(statements[0].contains("ON DUPLICATE KEY UPDATE"));
            }
        }
    }

    #[test]
    fn mysql_spatial_transfer_rejects_invalid_markers_and_keeps_public_insert_shape() {
        let invalid = json!("DBX_WKB:4326:0101000000");
        let transfer = generate_transfer_write_sql_batches(
            &TransferMode::Append,
            &[String::from("location")],
            &[Some(String::from("point"))],
            &[vec![invalid.clone()]],
            "places",
            "",
            &DatabaseType::Mysql,
            &[],
            None,
            false,
            true,
        )
        .unwrap();
        let public = generate_insert_typed(
            &[String::from("location")],
            &[Some(String::from("point"))],
            &[vec![invalid]],
            "places",
            "",
            &DatabaseType::Mysql,
            None,
        );

        assert_eq!(transfer, vec!["INSERT INTO `places` (`location`) VALUES\n('DBX_WKB:4326:0101000000')"]);
        assert_eq!(public, transfer[0]);
        assert!(!transfer[0].contains("ST_GeomFromWKB"));
    }

    #[test]
    fn mysql_insert_can_skip_duplicate_rows() {
        let batches = generate_insert_typed_sql_batches_from_value_rows_with_options(
            &[String::from("id"), String::from("name")],
            &[String::from("(42, 'Ada')"), String::from("(43, 'Bob')")],
            "users",
            "public",
            &DatabaseType::Mysql,
            None,
            SqlBatchLimits { max_rows: 100, target_sql_bytes: 1024, hard_sql_bytes: None },
            true,
        )
        .unwrap();

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].1, 2);
        assert!(batches[0].0.contains("ON DUPLICATE KEY UPDATE"));
        assert!(batches[0].0.contains("`id` = `id`"));
        assert!(batches[0].0.contains("(42, 'Ada')"));
        assert!(batches[0].0.contains("(43, 'Bob')"));
    }

    #[test]
    fn database_from_pool_key_handles_session_scoped_keys() {
        assert_eq!(database_from_pool_key("conn:analytics"), Some("analytics"));
        assert_eq!(database_from_pool_key("conn:analytics:session:editor-1"), Some("analytics"));
        assert_eq!(database_from_pool_key("conn"), None);
        assert_eq!(database_from_pool_key("conn:analytics:catalog:hive"), Some("analytics"));
        assert_eq!(database_from_pool_key("conn:catalog:hive"), None);
        assert_eq!(catalog_from_pool_key("conn:catalog:hive"), Some("hive"));
        assert_eq!(catalog_from_pool_key("conn:ads:catalog:paimon"), Some("paimon"));
        assert_eq!(catalog_from_pool_key("conn:ads"), None);
    }

    #[test]
    fn map_column_type_preserves_longtext_for_mysql_target() {
        assert_eq!(map_column_type("longtext", &DatabaseType::Mysql, &DatabaseType::Mysql), "longtext");
    }

    #[test]
    fn map_column_type_preserves_mediumtext_for_mysql_target() {
        assert_eq!(map_column_type("mediumtext", &DatabaseType::Mysql, &DatabaseType::Mysql), "mediumtext");
    }

    #[test]
    fn map_column_type_preserves_longblob_for_mysql_target() {
        assert_eq!(map_column_type("longblob", &DatabaseType::Mysql, &DatabaseType::Mysql), "longblob");
    }

    #[test]
    fn map_column_type_preserves_mediumblob_for_mysql_target() {
        assert_eq!(map_column_type("mediumblob", &DatabaseType::Mysql, &DatabaseType::Mysql), "mediumblob");
    }

    #[test]
    fn map_column_type_preserves_same_database_type() {
        assert_eq!(map_column_type("int unsigned", &DatabaseType::Mysql, &DatabaseType::Mysql), "int unsigned");
        assert_eq!(
            map_column_type("int unsigned zerofill", &DatabaseType::Mysql, &DatabaseType::Mysql),
            "int unsigned zerofill"
        );
        assert_eq!(map_column_type("bigint unsigned", &DatabaseType::Mysql, &DatabaseType::Mysql), "bigint unsigned");
        assert_eq!(
            map_column_type("bigint unsigned zerofill", &DatabaseType::Mysql, &DatabaseType::Mysql),
            "bigint unsigned zerofill"
        );
    }

    #[test]
    fn parse_mysql_row_error_extracts_row_number() {
        let err = "ERROR 22001 (1406): Data too long column 'content' at row 8";
        assert_eq!(parse_mysql_row_error(err), Some(8));
    }

    #[test]
    fn parse_mysql_row_error_returns_none_for_non_mysql_error() {
        assert_eq!(parse_mysql_row_error("some other error"), None);
    }

    #[test]
    fn mysql_create_table_preserves_auto_increment_primary_key() {
        let cols = vec![
            db::ColumnInfo {
                is_primary_key: true,
                is_nullable: false,
                extra: Some("auto_increment".to_string()),
                ..test_column("id", "INT")
            },
            db::ColumnInfo { is_nullable: false, ..test_column("name", "varchar(64)") },
        ];

        let ddl =
            generate_create_table_ddl(&cols, "users", "", "", &DatabaseType::Mysql, &DatabaseType::Mysql, None, None);

        assert!(ddl.contains("`id` INT NOT NULL AUTO_INCREMENT"), "ddl: {ddl}");
        assert!(ddl.contains("PRIMARY KEY (`id`)"), "ddl: {ddl}");
    }

    #[test]
    fn mysql_create_table_preserves_numeric_default_zero() {
        let cols = vec![db::ColumnInfo {
            is_nullable: false,
            column_default: Some("0".to_string()),
            ..test_column("status", "tinyint")
        }];

        let ddl =
            generate_create_table_ddl(&cols, "items", "", "", &DatabaseType::Mysql, &DatabaseType::Mysql, None, None);

        assert!(ddl.contains("DEFAULT 0"), "ddl: {ddl}");
        assert!(!ddl.contains("'0'"), "ddl should not quote numeric default: {ddl}");
    }

    #[test]
    fn mysql_create_table_quotes_string_default_with_escape() {
        let cols =
            vec![db::ColumnInfo { column_default: Some("o'clock".to_string()), ..test_column("label", "varchar(32)") }];

        let ddl =
            generate_create_table_ddl(&cols, "items", "", "", &DatabaseType::Mysql, &DatabaseType::Mysql, None, None);

        assert!(ddl.contains("DEFAULT 'o''clock'"), "ddl: {ddl}");
    }

    #[test]
    fn mysql_create_table_keeps_current_timestamp_default_and_on_update() {
        let cols = vec![db::ColumnInfo {
            is_nullable: false,
            column_default: Some("CURRENT_TIMESTAMP".to_string()),
            extra: Some("DEFAULT_GENERATED on update CURRENT_TIMESTAMP".to_string()),
            ..test_column("updated_at", "timestamp")
        }];

        let ddl =
            generate_create_table_ddl(&cols, "items", "", "", &DatabaseType::Mysql, &DatabaseType::Mysql, None, None);

        assert!(ddl.contains("DEFAULT CURRENT_TIMESTAMP"), "ddl: {ddl}");
        assert!(ddl.contains("ON UPDATE CURRENT_TIMESTAMP"), "ddl: {ddl}");
        assert!(ddl.contains("NOT NULL"), "ddl: {ddl}");
        assert!(!ddl.contains("DEFAULT_GENERATED"), "ddl should not leak DEFAULT_GENERATED: {ddl}");
    }

    #[test]
    fn mysql_create_table_keeps_current_timestamp_with_fsp() {
        let cols = vec![db::ColumnInfo {
            is_nullable: false,
            column_default: Some("CURRENT_TIMESTAMP(6)".to_string()),
            ..test_column("created_at", "timestamp(6)")
        }];

        let ddl =
            generate_create_table_ddl(&cols, "items", "", "", &DatabaseType::Mysql, &DatabaseType::Mysql, None, None);

        assert!(ddl.contains("DEFAULT CURRENT_TIMESTAMP(6)"), "ddl: {ddl}");
    }

    #[test]
    fn mysql_create_table_emits_on_update_without_default() {
        let cols = vec![db::ColumnInfo {
            is_nullable: false,
            extra: Some("on update CURRENT_TIMESTAMP(3)".to_string()),
            ..test_column("touched_at", "timestamp(3)")
        }];

        let ddl =
            generate_create_table_ddl(&cols, "items", "", "", &DatabaseType::Mysql, &DatabaseType::Mysql, None, None);

        assert!(ddl.contains("ON UPDATE CURRENT_TIMESTAMP(3)"), "ddl: {ddl}");
        assert!(!ddl.contains("DEFAULT"), "ddl should not emit DEFAULT when none was set: {ddl}");
    }

    #[test]
    fn parse_transfer_table_filter_accepts_predicates_and_strips_where_prefix() {
        assert_eq!(parse_transfer_table_filter(""), Ok(None));
        assert_eq!(parse_transfer_table_filter("   "), Ok(None));
        assert_eq!(parse_transfer_table_filter(";"), Ok(None));
        assert_eq!(
            parse_transfer_table_filter("age > 30"),
            Ok(Some(TransferTableFilter::Predicate("age > 30".to_string())))
        );
        assert_eq!(
            parse_transfer_table_filter("age > 30;"),
            Ok(Some(TransferTableFilter::Predicate("age > 30".to_string())))
        );
        assert_eq!(
            parse_transfer_table_filter("WHERE status = 'active'"),
            Ok(Some(TransferTableFilter::Predicate("status = 'active'".to_string())))
        );
    }

    #[test]
    fn parse_transfer_table_filter_accepts_full_queries_with_comments() {
        assert_eq!(
            parse_transfer_table_filter("SELECT * FROM orders WHERE amount > 10"),
            Ok(Some(TransferTableFilter::Query("SELECT * FROM orders WHERE amount > 10".to_string())))
        );
        assert_eq!(
            parse_transfer_table_filter("WITH recent AS (SELECT 1) SELECT * FROM recent"),
            Ok(Some(TransferTableFilter::Query("WITH recent AS (SELECT 1) SELECT * FROM recent".to_string())))
        );
        // Leading comments are skipped when detecting a query shape.
        assert_eq!(
            parse_transfer_table_filter("-- filtered rows\nSELECT * FROM orders"),
            Ok(Some(TransferTableFilter::Query("-- filtered rows\nSELECT * FROM orders".to_string())))
        );
    }

    #[test]
    fn parse_transfer_table_filter_rejects_multiple_statements() {
        assert!(parse_transfer_table_filter("a = 1; b = 2").is_err());
        assert!(parse_transfer_table_filter("SELECT 1; SELECT 2").is_err());
    }

    #[test]
    fn transfer_progress_queue_has_bounded_capacity() {
        let (sender, receiver) = tokio::sync::mpsc::channel(TRANSFER_PROGRESS_CHANNEL_CAPACITY);
        for rows_transferred in 0..=TRANSFER_PROGRESS_CHANNEL_CAPACITY {
            try_send_transfer_progress(
                &sender,
                TransferProgress {
                    transfer_id: "transfer".to_string(),
                    table: "table".to_string(),
                    table_index: 0,
                    total_tables: 1,
                    rows_transferred: rows_transferred as u64,
                    total_rows: None,
                    status: TransferStatus::Running,
                    error: None,
                    terminal: false,
                },
            );
        }

        assert_eq!(receiver.len(), TRANSFER_PROGRESS_CHANNEL_CAPACITY);
    }
}
