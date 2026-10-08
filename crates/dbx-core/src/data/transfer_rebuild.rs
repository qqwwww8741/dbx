//! Support for the transfer option `drop_target_before_create`.
//!
//! The transfer path has no usable transaction (see `transfer::execute_on_pool`: every
//! statement checks out a fresh connection), so the target table is renamed to a backup
//! instead of dropped. The backup is dropped only after the whole transfer succeeds; any
//! failure leaves it in place for manual recovery.
//!
//! This module owns the backup identifier: deriving it, and keeping it inside the target
//! dialect's identifier budget.

use sha2::{Digest, Sha256};
use sqlparser::ast::{visit_relations, ObjectNamePart, Statement, TableFactor, Visit, Visitor};
use sqlparser::dialect::{DuckDbDialect, MySqlDialect, SQLiteDialect};
use sqlparser::parser::Parser;
use std::collections::BTreeSet;
use std::ops::ControlFlow;

use crate::connection::AppState;
use crate::db_admin_sql::{supports_object_rename, DatabaseObjectType};
use crate::models::connection::DatabaseType;
use crate::production_safety::is_production_database;
use crate::sql_dialect::DialectCapabilityDescriptor;

/// Marker that identifies a table as a DBX transfer backup.
pub const BACKUP_TABLE_MARKER: &str = "__dbx_bak_";

/// Error prefix returned when a production target needs the destructive confirmation.
/// The frontend matches on this to raise its confirmation dialog instead of a plain error.
pub const DROP_TARGET_CONFIRMATION_REQUIRED: &str = "TRANSFER_DROP_TARGET_CONFIRMATION_REQUIRED";

/// Error prefix returned when the target dialect is outside the supported set.
pub const DROP_TARGET_UNSUPPORTED_DATABASE: &str = "TRANSFER_DROP_TARGET_UNSUPPORTED_DATABASE";

/// Error prefix returned when tables outside the transfer reference a target table.
/// Not recoverable by confirming: the user has to widen the selection or drop the
/// foreign keys, so the frontend surfaces it as a plain error with the table list.
pub const DROP_TARGET_EXTERNAL_FOREIGN_KEYS: &str = "TRANSFER_DROP_TARGET_EXTERNAL_FOREIGN_KEYS";

/// Error prefix for dependent objects that cannot be redirected during a rebuild.
pub const DROP_TARGET_EXTERNAL_DEPENDENCIES: &str = "TRANSFER_DROP_TARGET_EXTERNAL_DEPENDENCIES";

/// Dialects cleared for `drop_target_before_create`.
///
/// The excluded engines fall into three groups: no table object (MongoDB), a rebuild that
/// silently loses engine metadata (ClickHouse ENGINE/ORDER BY/TTL, QuestDB designated
/// timestamp), and managed-table DROP that also deletes the warehouse data files
/// (Hive/Spark/Kyuubi/Impala/Argo).
///
/// Oracle, Dameng and OceanBase-Oracle are additionally excluded even though they support
/// table rename: their constraint and index names are schema-unique, and the rename
/// pre-pass has not yet learned to release those names on the backup tables, so a rebuilt
/// table reusing the source DDL would collide (ORA-00955 / "already an object named").
const DROP_TARGET_SUPPORTED: &[DatabaseType] = &[DatabaseType::Mysql];

/// Hex characters of the derived hash appended after [`BACKUP_TABLE_MARKER`].
const BACKUP_HASH_LEN: usize = 8;

/// Identifier budget used when the dialect descriptor reports no limit. Deliberately the
/// tightest real limit in the descriptor table (Oracle) so an unknown target cannot
/// produce an over-long name.
const FALLBACK_MAX_IDENTIFIER_BYTES: usize = 30;

/// Identifier byte budget for `database_type`.
///
/// Measured in bytes even for dialects that count characters (MySQL): bytes are the
/// stricter reading, and over-truncating only makes the backup name shorter.
pub fn max_identifier_bytes(database_type: DatabaseType) -> usize {
    let reported = DialectCapabilityDescriptor::capabilities_for_database_type(database_type).max_identifier_length;
    if reported == 0 {
        FALLBACK_MAX_IDENTIFIER_BYTES
    } else {
        reported as usize
    }
}

/// Derive the backup table name for one source table inside one transfer.
///
/// Stable for a given `(transfer_id, qualified_source)` pair, so a retried step inside the
/// same transfer targets the same backup. Distinct source tables never collide even when
/// their names truncate to the same stem, because the hash covers the full qualified name.
pub fn backup_table_name(
    database_type: DatabaseType,
    transfer_id: &str,
    qualified_source: &str,
    target_table_name: &str,
) -> Result<String, String> {
    let mut hasher = Sha256::new();
    hasher.update(transfer_id.as_bytes());
    hasher.update([0x1f]);
    hasher.update(qualified_source.as_bytes());
    let digest = format!("{:x}", hasher.finalize());
    let suffix = format!("{BACKUP_TABLE_MARKER}{}", &digest[..BACKUP_HASH_LEN]);

    let budget = max_identifier_bytes(database_type);
    if budget <= suffix.len() {
        return Err(format!(
            "Cannot derive a backup table name for {target_table_name}: {} allows only {budget} identifier bytes, \
             and the backup suffix needs {}.",
            database_type.as_str(),
            suffix.len() + 1
        ));
    }
    let stem = truncate_on_char_boundary(target_table_name, budget - suffix.len());
    Ok(format!("{stem}{suffix}"))
}

/// Truncate to at most `max_bytes` bytes without splitting a UTF-8 character.
fn truncate_on_char_boundary(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Append the retained backup table to a failure message.
///
/// Once the rename pre-pass has run, every later failure leaves the original table behind
/// under its backup name. The name is derived from the transfer id and is not stored
/// anywhere, so an error that omits it leaves the user with no way to find their data.
///
/// Idempotent: a message that already names the backup — the drop-backup failure path does
/// — is returned unchanged instead of naming it twice.
pub fn annotate_error_with_retained_backup(error: String, qualified_backup: &str) -> String {
    if error.contains(qualified_backup) {
        return error;
    }
    format!("{error} The original target table was kept as backup '{qualified_backup}'; rename it back to recover.")
}

/// Whether `database_type` is cleared for `drop_target_before_create`.
///
/// Membership in [`DROP_TARGET_SUPPORTED`] is necessary but not sufficient: the backup step
/// needs a table rename, so a dialect that loses rename support also loses this option.
pub fn supports_drop_target_before_create(database_type: DatabaseType) -> bool {
    DROP_TARGET_SUPPORTED.contains(&database_type)
        && supports_object_rename(Some(database_type), DatabaseObjectType::Table)
}

/// Storage key prefix for the persisted rebuild recovery plan.
///
/// The journal lives in the existing state store (no schema change), keyed by transfer id,
/// so a crash mid-rename still leaves every backup findable on disk.
const REBUILD_JOURNAL_KEY_PREFIX: &str = "transfer-rebuild:";

/// One renamed table inside the persisted recovery plan.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RebuildRecoveryStep {
    /// Name of the table in the transfer selection.
    pub source_table: String,
    /// Actual target-side name that was renamed away.
    pub target_table: String,
    /// Backup name the target table now lives under.
    pub backup_table: String,
    /// Whether the table rename already executed.
    #[serde(default)]
    pub renamed: bool,
    /// Index/sequence renames executed on the backup table, as `kind old -> new`.
    #[serde(default)]
    pub renamed_objects: Vec<String>,
}

/// Persisted recovery plan for one rebuild transfer.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RebuildRecoveryJournal {
    pub transfer_id: String,
    pub target_connection_id: String,
    pub target_database: String,
    pub target_schema: String,
    pub database_type: String,
    pub steps: Vec<RebuildRecoveryStep>,
}

fn rebuild_journal_key(transfer_id: &str) -> String {
    format!("{REBUILD_JOURNAL_KEY_PREFIX}{transfer_id}")
}

/// Persist the full recovery plan before the first rename executes.
///
/// `plan` carries `(source_table, target_table, backup_table)` for every table the
/// pre-pass intends to rename. Writing it before any mutation guarantees the journal
/// describes at least as much as has been renamed, never less.
pub(crate) async fn persist_rebuild_plan(
    state: &AppState,
    request: &crate::transfer::TransferRequest,
    database_type: DatabaseType,
    plan: &[(String, String, String)],
) -> Result<(), String> {
    if plan.is_empty() {
        return Ok(());
    }
    let journal = RebuildRecoveryJournal {
        transfer_id: request.transfer_id.clone(),
        target_connection_id: request.target_connection_id.clone(),
        target_database: request.target_database.clone(),
        target_schema: request.target_schema.clone(),
        database_type: database_type.as_str().to_string(),
        steps: plan
            .iter()
            .map(|(source_table, target_table, backup_table)| RebuildRecoveryStep {
                source_table: source_table.clone(),
                target_table: target_table.clone(),
                backup_table: backup_table.clone(),
                renamed: false,
                renamed_objects: Vec::new(),
            })
            .collect(),
    };
    let bytes =
        serde_json::to_vec(&journal).map_err(|e| format!("Failed to serialize the rebuild recovery plan: {e}"))?;
    state
        .storage
        .save_state(&rebuild_journal_key(&request.transfer_id), &bytes, "application/json")
        .await
        .map_err(|e| format!("Failed to persist the rebuild recovery plan: {e}"))
}

/// Mark one table's renames as executed inside the persisted plan.
///
/// A missing journal is tolerated silently: the plan may not have been persisted for
/// transfers that renamed nothing, and the journal is best-effort metadata — it must
/// never fail the transfer it is trying to make recoverable.
pub(crate) async fn record_rebuild_step(
    state: &AppState,
    transfer_id: &str,
    source_table: &str,
    renamed_objects: Vec<String>,
) -> Result<(), String> {
    let result = record_rebuild_step_inner(state, transfer_id, source_table, renamed_objects).await;
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            log::warn!("[transfer] failed to update the rebuild recovery journal: {error}");
            Ok(())
        }
    }
}

async fn record_rebuild_step_inner(
    state: &AppState,
    transfer_id: &str,
    source_table: &str,
    renamed_objects: Vec<String>,
) -> Result<(), String> {
    let key = rebuild_journal_key(transfer_id);
    let Some((bytes, _content_type)) = state.storage.load_state(&key).await.map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let mut journal: RebuildRecoveryJournal =
        serde_json::from_slice(&bytes).map_err(|e| format!("Failed to parse the rebuild recovery journal: {e}"))?;
    if let Some(step) = journal.steps.iter_mut().find(|step| step.source_table == source_table) {
        step.renamed = true;
        step.renamed_objects = renamed_objects;
    }
    let bytes = serde_json::to_vec(&journal).map_err(|e| e.to_string())?;
    state.storage.save_state(&key, &bytes, "application/json").await
}

/// Delete the journal after the whole rebuild — including backup cleanup — succeeded.
///
/// Kept `pub`: the cleanup step lives in `transfer.rs` and both desktop and Web callers
/// run it through the same path.
pub(crate) async fn complete_rebuild_journal(state: &AppState, transfer_id: &str) -> Result<(), String> {
    state.storage.delete_state(&rebuild_journal_key(transfer_id)).await
}

/// Gate `drop_target_before_create` before a transfer starts.
///
/// Both the Tauri command and the web route call this, so the desktop app and the HTTP API
/// cannot drift apart on which targets are allowed or when confirmation is demanded.
pub async fn ensure_drop_target_allowed(
    state: &AppState,
    target_connection_id: &str,
    target_database: &str,
    target_database_type: DatabaseType,
    drop_target_before_create: bool,
    drop_target_confirmed: bool,
) -> Result<(), String> {
    if !drop_target_before_create {
        return Ok(());
    }
    if !supports_drop_target_before_create(target_database_type) {
        return Err(format!(
            "{DROP_TARGET_UNSUPPORTED_DATABASE}: dropping the target table before creating it is not supported for \
             {}.",
            target_database_type.as_str()
        ));
    }
    let production = {
        let configs = state.configs.read().await;
        configs
            .get(target_connection_id)
            .map(|config| is_production_database(config, target_database))
            // Fail closed: an unknown target connection is treated as production.
            .unwrap_or(true)
    };
    if production && !drop_target_confirmed {
        return Err(format!(
            "{DROP_TARGET_CONFIRMATION_REQUIRED}: rebuilding tables in production database '{target_database}' \
             requires explicit confirmation."
        ));
    }
    Ok(())
}

/// One incoming foreign key held by a table outside the transfer collection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExternalIncomingForeignKey {
    /// Schema (MySQL: database) that owns the referencing table.
    pub referencing_schema: String,
    /// Table holding the foreign key.
    pub referencing_table: String,
    /// Table inside the transfer collection that the foreign key points at.
    pub referenced_table: String,
    pub constraint_name: String,
}

/// Fail-fast gate: refuse the rebuild when a table outside the transfer collection
/// points a foreign key at one of the tables about to be renamed.
///
/// MySQL `RENAME TABLE` and PostgreSQL `ALTER TABLE ... RENAME` both keep incoming
/// foreign keys attached to the renamed table, so after the pre-pass the external
/// table's foreign key would reference the backup instead of the rebuilt table —
/// and dropping the backup at the end would then fail or, worse, silently leave the
/// external table pointing at a table that is about to disappear. Neither outcome is
/// recoverable from inside the transfer, so the transfer refuses to start.
///
/// Tables inside the collection are excluded: their foreign keys are re-created from
/// the source structure by the main pass (`pending_fk_alters`).
pub async fn ensure_no_external_incoming_foreign_keys(
    state: &AppState,
    target_pool_key: &str,
    target_database: &str,
    target_schema: &str,
    target_tables: &[String],
    target_database_type: DatabaseType,
) -> Result<(), String> {
    let blocking = detect_external_incoming_foreign_keys(
        state,
        target_pool_key,
        target_database,
        target_schema,
        target_tables,
        target_database_type,
    )
    .await?;
    match describe_external_incoming_foreign_keys(&blocking) {
        Some(message) => Err(message),
        None => Ok(()),
    }
}

/// Check incoming foreign keys and dependent views before the first rename.
///
/// This is the only dependency gate: it runs before any table is renamed aside, because a
/// foreign key from outside the collection or a dependent view would follow the rename onto
/// the backup. Cleanup later relies on `break_backup_foreign_key_graph` plus restrictive
/// drops, not on a second pass of this check.
pub async fn ensure_no_external_table_dependencies(
    state: &AppState,
    target_pool_key: &str,
    target_database: &str,
    target_schema: &str,
    target_tables: &[String],
    target_database_type: DatabaseType,
) -> Result<(), String> {
    if target_tables.is_empty() {
        return Ok(());
    }
    ensure_no_external_incoming_foreign_keys(
        state,
        target_pool_key,
        target_database,
        target_schema,
        target_tables,
        target_database_type,
    )
    .await?;

    let blocking = detect_dependent_views(
        state,
        target_pool_key,
        target_database,
        target_schema,
        target_tables,
        target_database_type,
    )
    .await?;
    if blocking.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{DROP_TARGET_EXTERNAL_DEPENDENCIES}: rebuilding the target tables would invalidate dependent objects or \
         leave them attached to the backup tables. These objects must be handled explicitly before rebuilding: {}",
        blocking.into_iter().collect::<Vec<_>>().join("; ")
    ))
}

async fn detect_dependent_views(
    state: &AppState,
    pool_key: &str,
    database: &str,
    schema: &str,
    target_tables: &[String],
    database_type: DatabaseType,
) -> Result<BTreeSet<String>, String> {
    {}
    let sql = dependent_views_sql(database_type, database, schema, target_tables).ok_or_else(|| {
        format!("Cannot safely inspect dependent views for {} before rebuilding target tables", database_type.as_str())
    })?;
    let result = match read_dependency_metadata(state, pool_key, &sql, 4).await {
        Ok(result) => result,
        // `information_schema.VIEW_TABLE_USAGE` arrived with MySQL 8.0. Every MySQL 5.7
        // target fails this lookup (ER_UNKNOWN_TABLE 1109/42S02), which used to abort the
        // whole rebuild even when the target held no views at all. Inspect the stored view
        // definitions instead: still fail-closed, just without the 8.0-only catalog.
        Err(error) if matches!(database_type, DatabaseType::Mysql) && mysql_view_usage_catalog_missing(&error) => {
            log::info!(
                "MySQL-compatible target has no information_schema.VIEW_TABLE_USAGE; \
                 inspecting stored view definitions instead"
            );
            return detect_parsed_view_dependencies(state, pool_key, database, schema, target_tables, database_type)
                .await;
        }
        Err(error) => {
            return Err(format!("Failed to inspect dependent views before rebuilding target tables: {error}"))
        }
    };
    result
        .rows
        .iter()
        .map(|row| {
            let owner = dependency_metadata_text(row, 0)?;
            let name = dependency_metadata_text(row, 1)?;
            let referenced = dependency_metadata_text(row, 2)?;
            let kind = dependency_metadata_text(row, 3)?;
            Ok(format!("{kind} {owner}.{name} -> {schema}.{referenced}"))
        })
        .collect()
}

/// Native dependency catalogs retain the referenced object identity across renames.
/// Do not exclude a view because its name happens to occur in `target_tables`: the
/// transfer's table selection does not authorize rewriting a target-only view.
fn dependent_views_sql(
    database_type: DatabaseType,
    database: &str,
    schema: &str,
    target_tables: &[String],
) -> Option<String> {
    let names = target_tables.iter().map(|table| quote_sql_literal(table)).collect::<Vec<_>>().join(", ");
    match database_type {
        DatabaseType::Mysql => Some(format!(
            "SELECT DISTINCT VIEW_SCHEMA, VIEW_NAME, TABLE_NAME, 'view' \
             FROM information_schema.VIEW_TABLE_USAGE \
             WHERE {database_match} AND {table_match}",
            database_match = mysql_metadata_name_matches("TABLE_SCHEMA", &[database]),
            table_match = mysql_metadata_name_matches(
                "TABLE_NAME",
                &target_tables.iter().map(String::as_str).collect::<Vec<_>>()
            ),
        )),

        _ => None,
    }
}

/// `information_schema.VIEW_TABLE_USAGE` exists from MySQL 8.0 on only. Older
/// MySQL-compatible servers answer the lookup with ER_UNKNOWN_TABLE: MySQL 5.7 reports
/// `ERROR 1109 (42S02): Unknown table 'VIEW_TABLE_USAGE' in information_schema`, and a
/// server with `lower_case_table_names=1` (the Windows default) echoes the name
/// lowercased. Match the table name so an unrelated `42S02` keeps reporting as an error.
fn mysql_view_usage_catalog_missing(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("view_table_usage")
        && (lower.contains("1109")
            || lower.contains("1146")
            || lower.contains("42s02")
            || lower.contains("unknown table"))
}

/// Stored definitions for the fallback path. Mirrors [`dependent_views_sql`]'s MySQL
/// branch: only views of the transferred database can be broken by rebuilding one of its
/// tables, so the schema filter keeps the parse surface identical.
fn mysql_view_definition_dependencies_sql(database: &str) -> String {
    format!(
        "SELECT '', TABLE_SCHEMA, TABLE_NAME, VIEW_DEFINITION FROM information_schema.VIEWS \
         WHERE {database_match}",
        database_match = mysql_metadata_name_matches("TABLE_SCHEMA", &[database]),
    )
}

async fn detect_parsed_view_dependencies(
    state: &AppState,
    pool_key: &str,
    database: &str,
    schema: &str,
    target_tables: &[String],
    database_type: DatabaseType,
) -> Result<BTreeSet<String>, String> {
    let (database, schema, sql) = {
        // `VIEW_DEFINITION` is the resolved SELECT, and a MySQL database *is* the schema
        // DBX rebuilds in, so the row's `TABLE_SCHEMA` decides which namespace an
        // unqualified relation belongs to. The 8.0 catalog path keys on the database, so
        // the fallback keeps the same spelling: dropping the database here would leave
        // qualified references unmatched whenever the caller passes an empty schema.
        (database.to_string(), schema.to_string(), mysql_view_definition_dependencies_sql(database))
    };
    let result = read_dependency_metadata(state, pool_key, &sql, 4).await?;
    let mut blocking = BTreeSet::new();
    for row in &result.rows {
        let view_database = dependency_metadata_text(row, 0)?;
        let view_schema = dependency_metadata_text(row, 1)?;
        let view_name = dependency_metadata_text(row, 2)?;
        let qualified = if view_database.is_empty() {
            format!("{view_schema}.{view_name}")
        } else {
            format!("{view_database}.{view_schema}.{view_name}")
        };
        let ddl = dependency_metadata_text(row, 3)
            .map_err(|error| format!("Cannot safely inspect dependent view {qualified}: {error}"))?;
        let references =
            { mysql_view_definition_target_references(ddl, &database, &schema, view_schema, target_tables) }
                .map_err(|error| format!("Cannot safely inspect dependent view {qualified}: {error}"))?;
        for referenced in references {
            blocking.insert(format!("view {qualified} -> {schema}.{referenced}"));
        }
    }
    Ok(blocking)
}

/// MySQL stores only the resolved `SELECT` in `information_schema.VIEWS.VIEW_DEFINITION`
/// (the `CREATE VIEW` wrapper and any `WITH CHECK OPTION` live outside that column), so
/// the fallback parses the statement body instead of a complete DDL.
fn mysql_view_definition_target_references(
    definition: &str,
    database: &str,
    schema: &str,
    view_schema: &str,
    target_tables: &[String],
) -> Result<BTreeSet<String>, String> {
    let dialect = MySqlDialect {};
    let statements = Parser::parse_sql(&dialect, definition).map_err(|error| error.to_string())?;
    let [Statement::Query(query)] = statements.as_slice() else {
        return Err("the catalog did not return a complete view definition".to_string());
    };
    view_query_target_references(query, DatabaseType::Mysql, database, schema, view_schema, target_tables)
}

/// Shared relation walk for a parsed view body.
fn view_query_target_references(
    query: &sqlparser::ast::Query,
    database_type: DatabaseType,
    database: &str,
    schema: &str,
    view_schema: &str,
    target_tables: &[String],
) -> Result<BTreeSet<String>, String> {
    if let ControlFlow::Break(error) = query.visit(&mut StaticViewRelations) {
        return Err(error.to_string());
    }
    let mut references = BTreeSet::new();
    let visited = visit_relations(query, |relation| {
        let names = relation
            .0
            .iter()
            .map(|part| match part {
                ObjectNamePart::Identifier(identifier) => Ok(identifier.value.as_str()),
                _ => Err("a view relation uses an unsupported dynamic identifier".to_string()),
            })
            .collect::<Result<Vec<_>, _>>();
        let names = match names {
            Ok(names) => names,
            Err(error) => return ControlFlow::Break(error),
        };
        let (qualifiers, table) = match names.split_last() {
            Some((table, qualifiers)) => (qualifiers, *table),
            None => return ControlFlow::Break("a view relation has no object name".to_string()),
        };
        let same_namespace = {
            // MySQL stores relations with their database name. DBX's `schema` and
            // `database` are the same namespace for MySQL, so either spelling of the
            // target proves an unqualified or single-qualifier reference points at it.
            match qualifiers {
                [] => view_schema.eq_ignore_ascii_case(schema) || view_schema.eq_ignore_ascii_case(database),
                [qualified] => qualified.eq_ignore_ascii_case(schema) || qualified.eq_ignore_ascii_case(database),
                _ => return ControlFlow::Break("a MySQL view relation has an unsupported qualified name".to_string()),
            }
        };
        if same_namespace {
            if let Some(target) = target_tables.iter().find(|target| target.eq_ignore_ascii_case(table)) {
                references.insert(target.clone());
            }
        }
        ControlFlow::Continue(())
    });
    match visited {
        ControlFlow::Break(error) => Err(error),
        ControlFlow::Continue(()) => Ok(references),
    }
}

/// Table functions/macros can hide relation names in strings (`query_table`) or
/// their own definitions. The view AST alone cannot prove those references safe.
struct StaticViewRelations;

impl Visitor for StaticViewRelations {
    type Break = &'static str;

    fn pre_visit_table_factor(&mut self, table: &TableFactor) -> ControlFlow<Self::Break> {
        match table {
            TableFactor::Table { args: None, .. } | TableFactor::Derived { .. } | TableFactor::NestedJoin { .. } => {
                ControlFlow::Continue(())
            }
            _ => ControlFlow::Break(
                "the view contains a table function or table expression whose dependencies cannot be inspected safely",
            ),
        }
    }
}

/// Render the fail-fast error for a non-empty set of blocking foreign keys.
///
/// `None` when nothing blocks. Grouped by referencing table so a table holding several
/// constraints reads as one entry, and ordered so the message is stable across runs
/// (`BTreeMap` + sorted constraints) — the frontend shows this string verbatim.
fn describe_external_incoming_foreign_keys(blocking: &[ExternalIncomingForeignKey]) -> Option<String> {
    if blocking.is_empty() {
        return None;
    }
    let mut grouped = std::collections::BTreeMap::<String, Vec<String>>::new();
    for fk in blocking {
        grouped
            .entry(format!("{}.{}", fk.referencing_schema, fk.referencing_table))
            .or_default()
            .push(format!("{} -> {}", fk.constraint_name, fk.referenced_table));
    }
    let described = grouped
        .into_iter()
        .map(|(table, mut constraints)| {
            constraints.sort();
            constraints.dedup();
            format!("{table} ({})", constraints.join(", "))
        })
        .collect::<Vec<_>>();

    Some(format!(
        "{DROP_TARGET_EXTERNAL_FOREIGN_KEYS}: {} table(s) outside this transfer reference the target tables, and \
         renaming a referenced table would move those foreign keys onto the backup table. Add the listed tables to \
         the transfer, or drop their foreign keys first: {}",
        described.len(),
        described.join("; ")
    ))
}

/// List incoming foreign keys held by tables outside `target_tables`.
///
/// SQLite also rewrites incoming foreign keys when a table is renamed, even if
/// enforcement is currently disabled. Never infer safety from `foreign_keys=OFF`.
pub async fn detect_external_incoming_foreign_keys(
    state: &AppState,
    target_pool_key: &str,
    target_database: &str,
    target_schema: &str,
    target_tables: &[String],
    target_database_type: DatabaseType,
) -> Result<Vec<ExternalIncomingForeignKey>, String> {
    if target_tables.is_empty() {
        return Ok(Vec::new());
    }

    let target_database = { target_database };

    let target_schema = { target_schema };
    let sql = external_incoming_foreign_keys_sql(target_database_type, target_database, target_schema, target_tables)
        .ok_or_else(|| {
        format!(
            "Cannot safely inspect incoming foreign keys for {} before rebuilding target tables: {}",
            target_database_type.as_str(),
            target_tables.join(", ")
        )
    })?;

    let result = read_dependency_metadata(state, target_pool_key, &sql, 4)
        .await
        .map_err(|e| format!("Failed to check incoming foreign keys on the target database: {e}"))?;

    let mut rows = result
        .rows
        .iter()
        .map(|row| {
            Ok(ExternalIncomingForeignKey {
                referencing_schema: dependency_metadata_text(row, 0)?.to_string(),
                referencing_table: dependency_metadata_text(row, 1)?.to_string(),
                referenced_table: dependency_metadata_text(row, 2)?.to_string(),
                constraint_name: dependency_metadata_text(row, 3)?.to_string(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    rows.sort();
    rows.dedup();
    Ok(rows)
}

async fn read_dependency_metadata(
    state: &AppState,
    pool_key: &str,
    sql: &str,
    expected_columns: usize,
) -> Result<crate::db::QueryResult, String> {
    let result = crate::transfer::execute_read_on_pool(state, pool_key, sql).await?;
    validate_dependency_metadata(&result, expected_columns)?;
    Ok(result)
}

fn validate_dependency_metadata(result: &crate::db::QueryResult, expected_columns: usize) -> Result<(), String> {
    if result.truncated || result.has_more {
        return Err("Target dependency metadata was truncated; a complete dependency check is required before rebuilding tables".to_string());
    }
    if result.columns.len() != expected_columns {
        return Err(format!(
            "Cannot safely inspect target dependencies: expected {expected_columns} metadata columns, received {}",
            result.columns.len()
        ));
    }
    if result.rows.iter().any(|row| row.len() != expected_columns) {
        return Err("Cannot safely inspect target dependencies: metadata contains an incomplete row".to_string());
    }
    Ok(())
}

fn dependency_metadata_text(row: &[serde_json::Value], index: usize) -> Result<&str, String> {
    row.get(index).and_then(serde_json::Value::as_str).ok_or_else(|| {
        format!("Cannot safely inspect target dependencies: metadata column {} is missing or is not text", index + 1)
    })
}

/// Build the metadata query returning `(referencing_schema, referencing_table,
/// referenced_table, constraint_name)` for every foreign key that points at
/// `target_tables` from outside that set.
///
/// `None` for dialects without a reliable metadata query. Split out from the async caller so the
/// generated SQL — including the same-schema exclusion that keeps the transfer's own
/// tables out of the result — is unit-testable without a live database.
fn external_incoming_foreign_keys_sql(
    database_type: DatabaseType,
    database: &str,
    schema: &str,
    target_tables: &[String],
) -> Option<String> {
    let name_list = target_tables.iter().map(|table| quote_sql_literal(table)).collect::<Vec<_>>().join(", ");
    match database_type {
        // MySQL family: schemas are databases, so the referenced side is keyed by
        // REFERENCED_TABLE_SCHEMA. Reading KEY_COLUMN_USAGE alone avoids the
        // catalog-wide scan a join with TABLE_CONSTRAINTS triggers on MySQL 5.7
        // (same reason as db::mysql::list_foreign_keys).
        DatabaseType::Mysql => Some(format!(
            "SELECT DISTINCT TABLE_SCHEMA, TABLE_NAME, REFERENCED_TABLE_NAME, CONSTRAINT_NAME \
             FROM information_schema.KEY_COLUMN_USAGE \
             WHERE {target_database_match} AND {target_table_match} \
               AND NOT ({source_database_match} AND {source_table_match})",
            target_database_match = mysql_metadata_name_matches("REFERENCED_TABLE_SCHEMA", &[database]),
            target_table_match = mysql_metadata_name_matches(
                "REFERENCED_TABLE_NAME",
                &target_tables.iter().map(String::as_str).collect::<Vec<_>>()
            ),
            source_database_match = mysql_metadata_name_matches("TABLE_SCHEMA", &[database]),
            source_table_match = mysql_metadata_name_matches(
                "TABLE_NAME",
                &target_tables.iter().map(String::as_str).collect::<Vec<_>>()
            ),
        )),

        _ => None,
    }
}

/// information_schema name columns have their own collation, which can equate
/// `orders` and `Orders` even when lower_case_table_names=0 makes them different
/// tables. Apply the server's table-name rules to both the selected and external
/// sides, keeping byte-sensitive comparison after folding where it is required.
fn mysql_metadata_name_matches(column: &str, names: &[&str]) -> String {
    let literals = names.iter().map(|name| quote_mysql_metadata_literal(name)).collect::<Vec<_>>();
    let exact = literals.join(", ");
    let folded = literals.iter().map(|literal| format!("LOWER({literal})")).collect::<Vec<_>>().join(", ");
    format!(
        "((@@lower_case_table_names = 0 AND BINARY {column} IN ({exact})) \
          OR (@@lower_case_table_names <> 0 AND BINARY LOWER({column}) IN ({folded})))"
    )
}

fn quote_mysql_metadata_literal(value: &str) -> String {
    if !value.contains('\\') {
        return quote_sql_literal(value);
    }
    // A hex literal has the same value with and without NO_BACKSLASH_ESCAPES.
    let hex = value.as_bytes().iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    format!("CONVERT(X'{hex}' USING utf8mb4)")
}

/// Quote a value as a SQL string literal. Local to this module so the metadata
/// queries above never interpolate a raw identifier.
fn quote_sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_metadata_queries_preserve_quotes_and_backslashes_without_sql_mode_assumptions() {
        let sql =
            external_incoming_foreign_keys_sql(DatabaseType::Mysql, "shop", "shop", &["x\\' OR 1=1 --".to_string()])
                .unwrap();
        assert!(sql.contains("CONVERT(X'785c27204f5220313d31202d2d' USING utf8mb4)"), "{sql}");
        let statements = Parser::parse_sql(&sqlparser::dialect::MySqlDialect {}, &sql).unwrap();
        assert_eq!(statements.len(), 1);
        assert!(matches!(statements.first(), Some(Statement::Query(_))));
    }

    #[test]
    fn mysql_view_usage_catalog_absence_is_recognized_without_hiding_other_failures() {
        for message in [
            "Server error: `ERROR 1109 (42S02): Unknown table 'VIEW_TABLE_USAGE' in information_schema`",
            "Server error: `ERROR 1109 (42S02): Unknown table 'view_table_usage' in information_schema`",
            "Server error: `ERROR 1146 (42S02): Table 'information_schema.view_table_usage' doesn't exist`",
        ] {
            assert!(mysql_view_usage_catalog_missing(message), "{message}");
        }
        for message in [
            "Server error: `ERROR 1146 (42S02): Table 'information_schema.VIEWS' doesn't exist`",
            "Server error: `ERROR 1045 (28000): Access denied for user 'root'@'localhost' (using password: YES)`",
            "Server error: `ERROR 1142 (42000): SELECT command denied to user 'dbx'@'%' for table 'VIEW_TABLE_USAGE'`",
            "connection reset by peer",
        ] {
            assert!(!mysql_view_usage_catalog_missing(message), "{message}");
        }
    }

    #[test]
    fn mysql_view_definitions_are_parsed_for_target_references() {
        // Captured from MySQL 5.7.43: `information_schema.VIEWS.VIEW_DEFINITION` names the
        // resolved database on every relation and expands `SELECT *` into explicit columns.
        let targets = vec!["orders".to_string(), "customers".to_string()];
        let qualified = mysql_view_definition_target_references(
            "select `shop`.`orders`.`id` AS `id`,`shop`.`orders`.`total` AS `total` \
             from `shop`.`orders` where (`shop`.`orders`.`id` > 0)",
            "shop",
            "shop",
            "shop",
            &targets,
        )
        .unwrap();
        assert_eq!(qualified.into_iter().collect::<Vec<_>>(), vec!["orders".to_string()]);

        // A bare API call can reach the MySQL fallback with an empty target schema; the
        // qualified reference must then match on the database spelling alone, exactly as
        // the 8.0 catalog path filters `TABLE_SCHEMA` by database.
        let empty_schema =
            mysql_view_definition_target_references("select `id` from `shop`.`orders`", "shop", "", "shop", &targets)
                .unwrap();
        assert_eq!(empty_schema.into_iter().collect::<Vec<_>>(), vec!["orders".to_string()]);

        let unqualified = mysql_view_definition_target_references(
            "select `id` from `shop`.`orders` join `customers` on 1 = 1",
            "shop",
            "shop",
            "shop",
            &targets,
        )
        .unwrap();
        assert_eq!(unqualified.into_iter().collect::<Vec<_>>(), vec!["customers".to_string(), "orders".to_string()]);

        let other_database = mysql_view_definition_target_references(
            "select `id` from `archive`.`orders`",
            "shop",
            "shop",
            "shop",
            &targets,
        )
        .unwrap();
        assert!(
            other_database.is_empty(),
            "a view of the transferred database cannot be broken by another database's table"
        );

        let unrelated = mysql_view_definition_target_references(
            "select `id` from `orders_summary`",
            "shop",
            "shop",
            "shop",
            &targets,
        )
        .unwrap();
        assert!(unrelated.is_empty());

        // MySQL has no three-part relation name, so the reference cannot be resolved and
        // the check must fail closed instead of reporting "no dependency".
        assert!(mysql_view_definition_target_references(
            "select `id` from `shop`.`extra`.`orders`",
            "shop",
            "shop",
            "shop",
            &targets,
        )
        .is_err());
    }

    #[test]
    fn mysql_view_definition_fallback_query_targets_the_transferred_database() {
        let sql = mysql_view_definition_dependencies_sql("shop");
        assert!(sql.contains("FROM information_schema.VIEWS"), "{sql}");
        assert!(!sql.contains("VIEW_TABLE_USAGE"), "{sql}");
        assert!(sql.contains("'shop'"), "{sql}");
        assert!(!sql.contains("TABLE_NAME IN"), "every view of the database has to be parsed: {sql}");
        let statements = Parser::parse_sql(&MySqlDialect {}, &sql).unwrap();
        assert_eq!(statements.len(), 1);

        assert!(mysql_view_definition_dependencies_sql("sh'op").contains("'sh''op'"));
    }

    const LONG_TABLE: &str = "customer_order_line_item_revision_history";

    fn name(db: DatabaseType, table: &str) -> String {
        backup_table_name(db, "transfer-1", &format!("shop.{table}"), table).unwrap()
    }

    #[test]
    fn same_source_and_transfer_is_idempotent() {
        let first = name(DatabaseType::Mysql, "orders");
        let second = name(DatabaseType::Mysql, "orders");
        assert_eq!(first, second);

        let other_transfer = backup_table_name(DatabaseType::Mysql, "transfer-2", "shop.orders", "orders").unwrap();
        assert_ne!(first, other_transfer, "a different transfer must not reuse the same backup name");
    }

    #[test]
    fn failure_messages_point_at_the_retained_backup() {
        let annotated = annotate_error_with_retained_backup(
            "Failed to insert batch: duplicate key.".to_string(),
            "`shop`.`orders__dbx_bak_1a2b3c4d`",
        );
        assert!(annotated.starts_with("Failed to insert batch: duplicate key."), "original error must lead");
        assert!(annotated.contains("`shop`.`orders__dbx_bak_1a2b3c4d`"), "backup name is missing: {annotated}");
    }

    #[test]
    fn a_message_that_already_names_the_backup_is_left_alone() {
        // The drop-backup failure path builds its own message; annotating it again would
        // print the same table twice.
        let original = "Transfer completed successfully, but failed to drop backup table \
                        'orders__dbx_bak_1a2b3c4d': permission denied."
            .to_string();
        assert_eq!(annotate_error_with_retained_backup(original.clone(), "orders__dbx_bak_1a2b3c4d"), original);
    }

    #[test]
    fn supported_targets_all_have_table_rename() {
        for db in DROP_TARGET_SUPPORTED {
            assert!(
                supports_drop_target_before_create(*db),
                "{db:?} is on the allow-list but cannot rename a table, so the backup step would fail"
            );
        }
    }

    fn sql(db: DatabaseType) -> Option<String> {
        external_incoming_foreign_keys_sql(db, "shop", "public", &["orders".to_string(), "customers".to_string()])
    }

    #[test]
    fn blocking_foreign_keys_are_grouped_per_referencing_table() {
        let fk = |table: &str, referenced: &str, constraint: &str| ExternalIncomingForeignKey {
            referencing_schema: "public".to_string(),
            referencing_table: table.to_string(),
            referenced_table: referenced.to_string(),
            constraint_name: constraint.to_string(),
        };
        assert_eq!(describe_external_incoming_foreign_keys(&[]), None, "no blockers must not produce an error");

        let message = describe_external_incoming_foreign_keys(&[
            fk("invoices", "orders", "fk_invoice_order"),
            fk("audit_log", "orders", "fk_audit_order"),
            fk("invoices", "customers", "fk_invoice_customer"),
        ])
        .expect("blockers must produce an error");

        assert!(message.starts_with(DROP_TARGET_EXTERNAL_FOREIGN_KEYS), "{message}");
        // Two referencing tables, not three constraints — the count drives the wording.
        assert!(message.contains("2 table(s)"), "{message}");
        // BTreeMap ordering keeps the message stable for snapshot-style frontend tests.
        assert!(
            message.contains(
                "public.audit_log (fk_audit_order -> orders); public.invoices (fk_invoice_customer -> \
                 customers, fk_invoice_order -> orders)"
            ),
            "{message}"
        );
    }
}
