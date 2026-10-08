use dbx_sql_core::value_literals::quote_string_literal;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use log;
use rayon::prelude::*;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::models::connection::DatabaseType;
use crate::sql_dialect::ddl_profile::{profile_for, AutoIncSyntax, DdlDialectProfile};
use crate::sql_dialect::descriptor::DialectKind;
use crate::sql_dialect::inference::{ColumnType, DefaultTypeInferenceEngine, TypeInferenceEngine};
use crate::sql_dialect::type_rewrite::{
    apply_auto_inc_to_column_def, column_is_auto_increment, rewrite_column_type, type_looks_integer, AutoIncColumnBuild,
};
use crate::sql_parser::ast_filter::AstTransmitFilter;
use crate::types::{
    ColumnInfo, ForeignKeyInfo, FunctionInfo, IndexInfo, OwnerInfo, RuleInfo, SequenceInfo, TableInfo, TriggerInfo,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnAddPosition {
    First,
    After(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDiff {
    #[serde(rename = "type")]
    pub diff_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ColumnInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<ColumnInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add_position: Option<ColumnAddPosition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexDiff {
    #[serde(rename = "type")]
    pub diff_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<IndexInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<IndexInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyDiff {
    #[serde(rename = "type")]
    pub diff_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<ForeignKeyInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<ForeignKeyInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDiff {
    #[serde(rename = "type")]
    pub diff_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<TriggerInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TriggerInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionDiff {
    #[serde(rename = "type")]
    pub diff_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<FunctionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<FunctionInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceDiff {
    #[serde(rename = "type")]
    pub diff_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SequenceInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<SequenceInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleDiff {
    #[serde(rename = "type")]
    pub diff_type: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<RuleInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<RuleInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnerDiff {
    #[serde(rename = "type")]
    pub diff_type: String,
    pub object_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<OwnerInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<OwnerInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TableDiff {
    #[serde(rename = "type")]
    pub diff_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_type: Option<String>,
    pub name: String,
    /// Source-side display/key name. `target_name` is set for an explicit
    /// source→target table mapping and is the physical target used by DDL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<ColumnDiff>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexes: Option<Vec<IndexDiff>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreign_keys: Option<Vec<ForeignKeyDiff>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triggers: Option<Vec<TriggerDiff>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ddl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_ddl: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_table_comment: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_table_comment: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_sql: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSchemaDetail {
    pub name: String,
    #[serde(default)]
    pub columns: Vec<ColumnInfo>,
    #[serde(default)]
    pub indexes: Vec<IndexInfo>,
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKeyInfo>,
    #[serde(default)]
    pub triggers: Vec<TriggerInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ddl: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ParamStrategy {
    Preserve,
    Strip,
    Custom,
}

fn default_param_strategy() -> ParamStrategy {
    ParamStrategy::Preserve
}

/// A custom field type mapping override: source_type → target_type.
/// Used when source and target database types differ.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldMapping {
    pub source_type: String,
    pub target_type: String,
    #[serde(default = "default_param_strategy")]
    pub param_strategy: ParamStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_params: Option<String>,
}

/// Canonicalizes a handful of ANSI-SQL type synonyms that name the same
/// underlying type across dialects (e.g. Postgres/Kingbase report the base
/// type as `character varying` via `format_type()`, while the field-mapping
/// UI's type catalog lists the shorter `varchar`). Without this, a user
/// mapping configured against one spelling silently never matches a column
/// reported under the other.
fn canonical_type_name(base: &str) -> std::borrow::Cow<'_, str> {
    match base.trim().to_ascii_uppercase().as_str() {
        "CHARACTER VARYING" => std::borrow::Cow::Borrowed("VARCHAR"),
        "CHARACTER" => std::borrow::Cow::Borrowed("CHAR"),
        _ => std::borrow::Cow::Owned(base.trim().to_ascii_uppercase()),
    }
}

/// Finds the mapping for `base_type`, preferring an exact (case-insensitive)
/// match over an alias match. The field-mapping panel auto-generates one row
/// per catalog type — `char`, `character`, `varchar` and `character varying`
/// commonly coexist as separate rows with independently chosen targets — so
/// treating aliases as interchangeable on the first hit alone would let an
/// earlier row (e.g. `char`) silently shadow a later exact row (e.g.
/// `character`) that shares the same canonical name. Alias matching is only
/// a fallback for when no row exactly names the reported type.
fn find_mapping<'a>(mappings: &'a [FieldMapping], base_type: &str) -> Option<&'a FieldMapping> {
    mappings
        .iter()
        .find(|m| m.source_type.eq_ignore_ascii_case(base_type))
        .or_else(|| mappings.iter().find(|m| canonical_type_name(&m.source_type) == canonical_type_name(base_type)))
}

/// Splices a driver-reported column length back into its type string when
/// the type name itself omits it. Postgres/Kingbase can report a varying
/// column as bare `character varying` (no length) via `format_type()` while
/// still exposing the real length separately as `character_maximum_length`.
/// Without this, the cross-dialect rewrite below has no way to tell "no
/// length was ever declared" (safe to default) apart from "the length just
/// isn't embedded in this dialect's type string" (issue #8011) — silently
/// falling back to a generic default in the latter case would replace a
/// known-correct length with a possibly wrong one.
fn with_known_length(source_type: &str, character_maximum_length: Option<i32>) -> String {
    let trimmed = source_type.trim();
    if trimmed.contains('(') {
        return trimmed.to_string();
    }
    // `character_maximum_length` is populated by several drivers for types
    // where it does NOT mean "declared length in this position" — MySQL's
    // information_schema fills it in for TEXT/BLOB family columns (e.g.
    // TEXT -> 65535), and Oracle's DATA_LENGTH is filled in for every
    // column, including DATE (byte length, e.g. 7) and NUMBER. Splicing
    // those in verbatim is wrong twice over: `TEXT(65535)` is silently
    // *reinterpreted* as MEDIUMTEXT by MySQL (real DB verified), and
    // `DATE(7)` sent to a MySQL target is a straight syntax error.
    //
    // CHAR/CHARACTER/NCHAR belong on this whitelist alongside the VARCHAR
    // family, unlike in type_rewrite's *default*-to-255 list: that list
    // invents a length out of thin air (where CHAR must be excluded — a
    // bare CHAR is already valid MySQL, meaning CHAR(1)), whereas this
    // function only *restores* a length the driver already reported
    // separately. MySQL's own information_schema does this for CHAR too
    // (DATA_TYPE="char", CHARACTER_MAXIMUM_LENGTH=10 for a CHAR(10) column,
    // real DB verified) — excluding CHAR here would silently truncate a
    // real CHAR(10) column down to CHAR(1). Mirrors the same whitelist
    // `columnDDLDataType` uses in agents/drivers/kingbase-go/kingbase_metadata.go.
    let base_upper = trimmed.to_ascii_uppercase();
    if !matches!(
        base_upper.as_str(),
        "VARCHAR" | "CHARACTER VARYING" | "NVARCHAR" | "CHAR" | "CHARACTER" | "NCHAR" | "VARCHAR2" | "NVARCHAR2"
    ) {
        return trimmed.to_string();
    }
    match character_maximum_length {
        Some(len) if len > 0 => format!("{trimmed}({len})"),
        _ => trimmed.to_string(),
    }
}

/// Numeric base types whose `(precision[, scale])` is part of the *declared* type on every
/// dialect that spells the name that way. Oracle's `ALL_TAB_COLUMNS` — and therefore every
/// Oracle introspection path, native agent and JDBC alike — reports a bare `NUMBER` with
/// `DATA_PRECISION`/`DATA_SCALE` beside it, so comparing the type strings alone cannot tell
/// `NUMBER(10,2)` from `NUMBER(12,2)` and the difference is silently dropped from the
/// result (#9261). Integer families are deliberately excluded: MySQL's `information_schema`
/// fills `numeric_precision` in for `int` (10) and rendering that back as `int(10)` would
/// invent a display width that a Postgres target rejects.
const NUMERIC_TYPES_WITH_DECLARED_PRECISION: [&str; 5] = ["NUMBER", "NUMERIC", "DECIMAL", "DEC", "FIXED"];

/// Counterpart of [`with_known_length`] for the numeric family: splices a driver-reported
/// precision/scale back into a type name that omits them. A negative scale (`NUMBER(10,-2)`
/// on Oracle) is preserved because it changes the value range, while a scale of zero is
/// left out — `NUMBER(10)` and `NUMBER(10,0)` are the same column.
fn with_known_numeric_precision(
    source_type: &str,
    numeric_precision: Option<i32>,
    numeric_scale: Option<i32>,
) -> String {
    let trimmed = source_type.trim();
    if trimmed.contains('(') {
        return trimmed.to_string();
    }
    let base = trimmed.split([' ', '(']).next().unwrap_or_default().to_ascii_uppercase();
    if !NUMERIC_TYPES_WITH_DECLARED_PRECISION.contains(&base.as_str()) {
        return trimmed.to_string();
    }
    let Some(precision) = numeric_precision.filter(|value| *value > 0) else {
        return trimmed.to_string();
    };
    match numeric_scale {
        Some(scale) if scale != 0 => format!("{trimmed}({precision},{scale})"),
        _ => format!("{trimmed}({precision})"),
    }
}

/// The declared shape of a column: the driver's type string with every length/precision it
/// reports *beside* the string spliced back in. Comparison needs it so that a changed
/// precision or length is a difference at all, and scripts need it so that an emitted
/// `ADD COLUMN`/`MODIFY` keeps the parameters the user declared.
fn declared_column_type(column: &ColumnInfo) -> String {
    with_known_numeric_precision(
        &with_known_length(&column.data_type, column.character_maximum_length),
        column.numeric_precision,
        column.numeric_scale,
    )
}

impl FieldMapping {
    pub fn apply<'a>(mappings: &'a [FieldMapping], source_type: &str) -> Option<&'a str> {
        let base_type = source_type.split('(').next().unwrap_or(source_type).trim();
        find_mapping(mappings, base_type).map(|m| m.target_type.as_str())
    }

    pub fn apply_with_params(mappings: &[FieldMapping], source_type: &str, target_kind: DialectKind) -> Option<String> {
        let trimmed = source_type.trim();
        let base_type = trimmed.split('(').next().unwrap_or(trimmed);
        let source_params = &trimmed[base_type.len()..];
        let matched = find_mapping(mappings, base_type)?;

        let result = match matched.param_strategy {
            ParamStrategy::Strip => Some(matched.target_type.clone()),
            ParamStrategy::Custom => match &matched.custom_params {
                Some(params) if !params.is_empty() => {
                    let p = params.trim();
                    // Normalize: wrap bare params (e.g. "100") in parentheses so the
                    // generated type becomes e.g. `character(100)` rather than `character100`.
                    let formatted = if p.starts_with('(') { p.to_string() } else { format!("({})", p) };
                    Some(format!("{}{}", matched.target_type, formatted))
                }
                _ => Some(matched.target_type.clone()),
            },
            ParamStrategy::Preserve => {
                let supports = type_supports_params(target_kind, &matched.target_type);
                let has_params = !source_params.is_empty();
                if has_params && supports {
                    Some(format!("{}{}", matched.target_type, source_params))
                } else {
                    log::info!(
                        "apply_with_params[Preserve] source={} target={} strategy={:?} has_params={} supports_params={} -> bare {}",
                        source_type, matched.target_type, matched.param_strategy, has_params, supports, matched.target_type
                    );
                    Some(matched.target_type.clone())
                }
            }
        };
        log::info!(
            "apply_with_params source={} target_type={} strategy={:?} result={:?}",
            source_type,
            matched.target_type,
            matched.param_strategy,
            result
        );
        result
    }
}

fn type_supports_params(kind: DialectKind, type_name: &str) -> bool {
    crate::sql_dialect::dialect_loader::register_core_dialects();
    let registry = crate::sql_dialect::dialect_loader::DialectRegistry::global();
    let all = registry.get_all_by_kind(kind);
    if all.is_empty() {
        return true;
    }
    all.iter().any(|loaded| {
        loaded.yaml.types.iter().any(|t| {
            (t.name.eq_ignore_ascii_case(type_name) || t.aliases.iter().any(|a| a.eq_ignore_ascii_case(type_name)))
                && (t.has_length || t.has_precision || t.max_precision.is_some())
        })
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiffTableMapping {
    pub source_table: String,
    pub target_table: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiffPreparationOptions {
    #[serde(default)]
    pub source_tables: Vec<TableInfo>,
    #[serde(default)]
    pub target_tables: Vec<TableInfo>,
    #[serde(default)]
    pub source_details: Vec<TableSchemaDetail>,
    #[serde(default)]
    pub target_details: Vec<TableSchemaDetail>,
    #[serde(default)]
    pub source_functions: Vec<FunctionInfo>,
    #[serde(default)]
    pub target_functions: Vec<FunctionInfo>,
    #[serde(default)]
    pub source_sequences: Vec<SequenceInfo>,
    #[serde(default)]
    pub target_sequences: Vec<SequenceInfo>,
    #[serde(default)]
    pub source_rules: Vec<RuleInfo>,
    #[serde(default)]
    pub target_rules: Vec<RuleInfo>,
    #[serde(default)]
    pub source_owners: Vec<OwnerInfo>,
    #[serde(default)]
    pub target_owners: Vec<OwnerInfo>,
    pub database_type: DatabaseType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_schema: Option<String>,
    #[serde(default)]
    pub ignore_comments: bool,
    #[serde(default)]
    pub cascade_delete: bool,
    #[serde(default)]
    pub compare_column_order: bool,
    #[serde(default = "default_compare_charset")]
    pub compare_charset: bool,
    #[serde(default)]
    pub ignore_table_name_case: bool,
    #[serde(default)]
    pub ignore_column_name_case: bool,
    #[serde(default)]
    pub detect_renames: bool,
    #[serde(default)]
    pub detect_table_renames: bool,
    #[serde(default)]
    pub rename_threshold: f64,
    #[serde(default)]
    pub enable_rollback: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub batch_patterns: Vec<BatchPattern>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_dialect: Option<DialectKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_dialect: Option<DialectKind>,
    #[serde(default)]
    pub compatibility_threshold: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_permissions: Vec<PermissionInfo>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_permissions: Vec<PermissionInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shard_strategy: Option<ShardStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_constraint: Option<ResourceConstraint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub field_mappings: Vec<FieldMapping>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub table_mappings: Vec<SchemaDiffTableMapping>,
}

const fn default_compare_charset() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MissingRollbackObject {
    pub kind: String,
    pub name: String,
    pub table: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RollbackCompleteness {
    #[serde(rename = "complete")]
    Complete,
    #[serde(rename = "incomplete")]
    Incomplete,
}

impl RollbackCompleteness {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiffPreparation {
    pub diffs: Vec<TableDiff>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub function_diffs: Vec<FunctionDiff>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sequence_diffs: Vec<SequenceDiff>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule_diffs: Vec<RuleDiff>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owner_diffs: Vec<OwnerDiff>,
    pub sync_sql: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_sync_sql: Option<String>,
    /// Whether rollback SQL is complete enough to execute safely.
    #[serde(default = "default_rollback_complete")]
    pub rollback_completeness: RollbackCompleteness,
    /// Objects that could not be reconstructed for rollback (e.g. triggers without body).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_rollback_objects: Vec<MissingRollbackObject>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rename_candidates: Vec<RenameCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_graph: Option<RollbackGraph>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compatibility_warnings: Vec<ColumnCompatibilityWarning>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_diffs: Vec<PermissionDiff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_sync_sql: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependency_graph: Option<DependencyGraph>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaSyncSqlPlan {
    pub sync_sql: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rollback_sync_sql: Option<String>,
    pub rollback_completeness: RollbackCompleteness,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_rollback_objects: Vec<MissingRollbackObject>,
}

fn default_rollback_complete() -> RollbackCompleteness {
    RollbackCompleteness::Complete
}

// ============================================================================
// Phase 4.1: Dependency Graph & Rename Detection
// ============================================================================

/// Regex-based text scanning for table references in SQL/DDL text.
/// Used as fallback when no live DB query (YAML metadata_queries.dependencies) is available.
fn extract_ddl_references(sql: &str, known_tables: &HashSet<&str>) -> Vec<String> {
    let upper = sql.to_uppercase();
    let mut refs: Vec<String> = Vec::new();

    for table in known_tables {
        let table_up = table.to_uppercase();
        // Match after SQL keywords that indicate table references
        let patterns = [
            format!(" FROM {table_up}"),
            format!(" JOIN {table_up}"),
            format!(" INTO {table_up}"),
            format!(" TABLE {table_up}"),
            format!(" REFERENCES {table_up}"),
            format!(" UPDATE {table_up}"),
            format!("DELETE FROM {table_up}"),
            format!("FROM {table_up} ("),
            format!(" {table_up}."),
        ];
        if patterns.iter().any(|p| upper.contains(p.as_str())) {
            refs.push(table.to_string());
        }
    }

    refs
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyNode {
    pub table_name: String,
    pub depends_on: Vec<String>,
    pub depended_by: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyGraph {
    pub nodes: HashMap<String, DependencyNode>,
    pub topological_order: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageReport {
    pub level1_score: f64,
    pub level2_score: f64,
    pub composite_score: f64,
    pub level1_covered: u64,
    pub level1_total: u64,
    pub level2_covered: u64,
    pub level2_total: u64,
    pub uncovered_edges: Vec<UncoveredEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncoveredEdge {
    pub from_table: String,
    pub to_table: String,
    pub level: u32,
}

impl DependencyGraph {
    pub fn build(details: &[TableSchemaDetail], tables: &[TableInfo]) -> Self {
        Self::build_with_functions(details, tables, &[], &[])
    }

    pub fn build_with_options(
        details: &[TableSchemaDetail],
        tables: &[TableInfo],
        ignore_table_name_case: bool,
    ) -> Self {
        Self::build_with_functions_and_options(details, tables, &[], &[], ignore_table_name_case)
    }

    /// Extended build: also extracts dependencies from view DDLs, triggers, and function/sequence definitions.
    /// Falls back to regex-based text scanning when no live DB query is available.
    pub fn build_with_functions(
        details: &[TableSchemaDetail],
        tables: &[TableInfo],
        functions: &[FunctionInfo],
        sequences: &[SequenceInfo],
    ) -> Self {
        Self::build_with_functions_and_options(details, tables, functions, sequences, false)
    }

    pub fn build_with_functions_and_options(
        details: &[TableSchemaDetail],
        tables: &[TableInfo],
        functions: &[FunctionInfo],
        _sequences: &[SequenceInfo],
        ignore_table_name_case: bool,
    ) -> Self {
        let table_names: HashSet<&str> =
            tables.iter().filter(|t| !t.table_type.contains("VIEW")).map(|t| t.name.as_str()).collect();
        let view_names: HashSet<&str> =
            tables.iter().filter(|t| t.table_type.contains("VIEW")).map(|t| t.name.as_str()).collect();
        let all_names: HashSet<&str> = tables.iter().map(|t| t.name.as_str()).collect();

        let mut nodes: HashMap<String, DependencyNode> = all_names
            .iter()
            .map(|name| {
                (
                    name.to_string(),
                    DependencyNode { table_name: name.to_string(), depends_on: Vec::new(), depended_by: Vec::new() },
                )
            })
            .collect();

        let detail_map: HashMap<&str, &TableSchemaDetail> = details.iter().map(|d| (d.name.as_str(), d)).collect();
        let function_by_name: HashMap<&str, &FunctionInfo> = functions.iter().map(|f| (f.name.as_str(), f)).collect();

        // Phase 1: FK-based dependencies (existing logic)
        for table_name in &table_names {
            if let Some(detail) = detail_map.get(table_name) {
                for fk in &detail.foreign_keys {
                    let referenced_table = if table_names.contains(fk.ref_table.as_str()) {
                        Some(fk.ref_table.as_str())
                    } else if ignore_table_name_case {
                        let mut candidates =
                            table_names.iter().copied().filter(|name| name.eq_ignore_ascii_case(&fk.ref_table));
                        let candidate = candidates.next();
                        candidate.filter(|_| candidates.next().is_none())
                    } else {
                        None
                    };

                    if let Some(referenced_table) = referenced_table {
                        if let Some(node) = nodes.get_mut(*table_name) {
                            if !node.depends_on.iter().any(|name| name == referenced_table) {
                                node.depends_on.push(referenced_table.to_string());
                            }
                        }
                        if let Some(ref_node) = nodes.get_mut(referenced_table) {
                            if !ref_node.depended_by.iter().any(|name| name == *table_name) {
                                ref_node.depended_by.push((*table_name).to_string());
                            }
                        }
                    }
                }
            }
        }

        // Phase 2: View DDL text scanning
        for view_name in &view_names {
            if let Some(detail) = detail_map.get(view_name) {
                if let Some(ddl) = &detail.ddl {
                    let refs = extract_ddl_references(ddl, &table_names);
                    for ref_table in refs {
                        if let Some(node) = nodes.get_mut(*view_name) {
                            if !node.depends_on.contains(&ref_table) {
                                node.depends_on.push(ref_table.clone());
                            }
                        }
                        if let Some(ref_node) = nodes.get_mut(&ref_table) {
                            if !ref_node.depended_by.iter().any(|d| d == *view_name) {
                                ref_node.depended_by.push((*view_name).to_string());
                            }
                        }
                    }
                }
            }
        }

        // Phase 3: Trigger statement text scanning
        for table_name in &all_names {
            if let Some(detail) = detail_map.get(table_name) {
                for trigger in &detail.triggers {
                    if let Some(stmt) = &trigger.statement {
                        let refs = extract_ddl_references(stmt, &table_names);
                        for ref_table in refs {
                            if let Some(node) = nodes.get_mut(*table_name) {
                                if !node.depends_on.contains(&ref_table) {
                                    node.depends_on.push(ref_table.clone());
                                }
                            }
                            if let Some(ref_node) = nodes.get_mut(&ref_table) {
                                if !ref_node.depended_by.iter().any(|d| d == *table_name) {
                                    ref_node.depended_by.push((*table_name).to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        // Phase 4: Function definition text scanning
        for (_func_name, func) in &function_by_name {
            let refs = extract_ddl_references(&func.definition, &table_names);
            for ref_table in &refs {
                if let Some(ref_node) = nodes.get_mut(ref_table) {
                    if !ref_node.depended_by.iter().any(|d| d == _func_name) {
                        ref_node.depended_by.push(_func_name.to_string());
                    }
                }
            }
        }

        let topological_order = Self::topological_sort(&nodes);
        DependencyGraph { nodes, topological_order }
    }

    fn topological_sort(nodes: &HashMap<String, DependencyNode>) -> Vec<String> {
        let mut in_degree: HashMap<&str, usize> = nodes.keys().map(|k| (k.as_str(), 0usize)).collect();
        for node in nodes.values() {
            in_degree.entry(node.table_name.as_str()).or_insert(0);
            for _dep in &node.depends_on {
                *in_degree.entry(node.table_name.as_str()).or_insert(0) += 1;
            }
        }

        let mut queue: VecDeque<&str> = in_degree.iter().filter(|(_, &deg)| deg == 0).map(|(&name, _)| name).collect();

        let mut result = Vec::new();
        while let Some(name) = queue.pop_front() {
            result.push(name.to_string());
            if let Some(node) = nodes.get(name) {
                for dependent in &node.depended_by {
                    if let Some(deg) = in_degree.get_mut(dependent.as_str()) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push_back(dependent.as_str());
                        }
                    }
                }
            }
        }

        if result.len() != nodes.len() {
            let remaining: Vec<String> = nodes.keys().filter(|k| !result.contains(k)).cloned().collect();
            result.extend(remaining);
        }

        result
    }

    pub fn build_order(&self) -> Vec<String> {
        self.topological_order.clone()
    }

    pub fn drop_order(&self) -> Vec<String> {
        let mut order = self.topological_order.clone();
        order.reverse();
        order
    }

    pub fn coverage_score(&self, diffed_tables: &[String]) -> f64 {
        self.coverage_score_level1(diffed_tables)
    }

    pub fn coverage_score_level1(&self, diffed_tables: &[String]) -> f64 {
        if self.nodes.is_empty() {
            return 1.0;
        }
        let diffed_set: HashSet<&str> = diffed_tables.iter().map(|s| s.as_str()).collect();
        let mut covered_edges = 0u64;
        let mut total_edges = 0u64;

        for node in self.nodes.values() {
            for dep in &node.depends_on {
                total_edges += 1;
                if diffed_set.contains(node.table_name.as_str()) && diffed_set.contains(dep.as_str()) {
                    covered_edges += 1;
                }
            }
        }

        if total_edges == 0 {
            1.0
        } else {
            covered_edges as f64 / total_edges as f64
        }
    }

    pub fn coverage_score_level2(&self, diffed_tables: &[String]) -> f64 {
        if self.nodes.is_empty() {
            return 1.0;
        }
        let diffed_set: HashSet<&str> = diffed_tables.iter().map(|s| s.as_str()).collect();

        let mut transitive_edges = 0u64;
        let mut covered_transitive = 0u64;

        for node in self.nodes.values() {
            let table_name = node.table_name.as_str();
            if !diffed_set.contains(table_name) {
                continue;
            }
            for indirect in &node.depends_on {
                if let Some(inner) = self.nodes.get(indirect) {
                    for grand in &inner.depends_on {
                        transitive_edges += 1;
                        if diffed_set.contains(table_name) && diffed_set.contains(grand.as_str()) {
                            covered_transitive += 1;
                        }
                    }
                }
            }
        }

        if transitive_edges == 0 {
            1.0
        } else {
            covered_transitive as f64 / transitive_edges as f64
        }
    }

    pub fn composite_coverage_score(&self, diffed_tables: &[String]) -> CoverageReport {
        let diffed_set: HashSet<&str> = diffed_tables.iter().map(|s| s.as_str()).collect();

        let (l1_covered, l1_total) = self.count_edges(diffed_tables, &diffed_set, 1);
        let (l2_covered, l2_total) = self.count_transitive_edges(diffed_tables, &diffed_set);

        let l1_score = if l1_total == 0 { 1.0 } else { l1_covered as f64 / l1_total as f64 };
        let l2_score = if l2_total == 0 { 1.0 } else { l2_covered as f64 / l2_total as f64 };

        let composite_score = 0.6 * l1_score + 0.4 * l2_score;

        let uncovered = self.collect_uncovered_edges(diffed_tables, &diffed_set);

        CoverageReport {
            level1_score: l1_score,
            level2_score: l2_score,
            composite_score,
            level1_covered: l1_covered,
            level1_total: l1_total,
            level2_covered: l2_covered,
            level2_total: l2_total,
            uncovered_edges: uncovered,
        }
    }

    fn count_edges(&self, _diffed_tables: &[String], diffed_set: &HashSet<&str>, _level: u32) -> (u64, u64) {
        let mut covered = 0u64;
        let mut total = 0u64;
        for node in self.nodes.values() {
            for dep in &node.depends_on {
                total += 1;
                if diffed_set.contains(node.table_name.as_str()) && diffed_set.contains(dep.as_str()) {
                    covered += 1;
                }
            }
        }
        (covered, total)
    }

    fn count_transitive_edges(&self, _diffed_tables: &[String], diffed_set: &HashSet<&str>) -> (u64, u64) {
        let mut covered = 0u64;
        let mut total = 0u64;
        for node in self.nodes.values() {
            let table_name = node.table_name.as_str();
            if !diffed_set.contains(table_name) {
                continue;
            }
            for indirect in &node.depends_on {
                if let Some(inner) = self.nodes.get(indirect) {
                    for grand in &inner.depends_on {
                        total += 1;
                        if diffed_set.contains(table_name) && diffed_set.contains(grand.as_str()) {
                            covered += 1;
                        }
                    }
                }
            }
        }
        (covered, total)
    }

    fn collect_uncovered_edges(&self, _diffed_tables: &[String], diffed_set: &HashSet<&str>) -> Vec<UncoveredEdge> {
        let mut uncovered = Vec::new();
        for node in self.nodes.values() {
            for dep in &node.depends_on {
                let both_covered = diffed_set.contains(node.table_name.as_str()) && diffed_set.contains(dep.as_str());
                if !both_covered {
                    uncovered.push(UncoveredEdge {
                        from_table: node.table_name.clone(),
                        to_table: dep.clone(),
                        level: 1,
                    });
                }
            }
            for indirect in &node.depends_on {
                if let Some(inner) = self.nodes.get(indirect) {
                    for grand in &inner.depends_on {
                        let all_covered =
                            diffed_set.contains(node.table_name.as_str()) && diffed_set.contains(grand.as_str());
                        if !all_covered {
                            uncovered.push(UncoveredEdge {
                                from_table: node.table_name.clone(),
                                to_table: grand.clone(),
                                level: 2,
                            });
                        }
                    }
                }
            }
        }
        uncovered
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameCandidate {
    pub source_name: String,
    pub target_name: String,
    pub score: f64,
    pub column_jaccard: f64,
    pub type_similarity: f64,
}

fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.intersection(b).count();
    let union = a.union(b).count();
    if union == 0 {
        1.0
    } else {
        intersection as f64 / union as f64
    }
}

fn column_type_similarity(source_cols: &[ColumnInfo], target_cols: &[ColumnInfo]) -> f64 {
    if source_cols.is_empty() || target_cols.is_empty() {
        return 0.0;
    }
    let engine = DefaultTypeInferenceEngine;
    let source_map: HashMap<&str, &ColumnInfo> = source_cols.iter().map(|c| (c.name.as_str(), c)).collect();
    let target_map: HashMap<&str, &ColumnInfo> = target_cols.iter().map(|c| (c.name.as_str(), c)).collect();
    let common_names: HashSet<&str> = source_map.keys().filter(|k| target_map.contains_key(**k)).copied().collect();

    if common_names.is_empty() {
        return 0.0;
    }

    let total: f64 = common_names
        .iter()
        .map(|name| {
            let s = ColumnType::parse(&source_map[name].data_type);
            let t = ColumnType::parse(&target_map[name].data_type);
            engine.type_compatibility_score(&s, &t)
        })
        .sum();
    total / common_names.len() as f64
}

pub fn detect_renames(
    removed: &[String],
    added: &[String],
    source_details: &[TableSchemaDetail],
    target_details: &[TableSchemaDetail],
    threshold: f64,
) -> Vec<RenameCandidate> {
    let source_detail_map: HashMap<&str, &TableSchemaDetail> =
        source_details.iter().map(|d| (d.name.as_str(), d)).collect();
    let target_detail_map: HashMap<&str, &TableSchemaDetail> =
        target_details.iter().map(|d| (d.name.as_str(), d)).collect();

    let mut candidates = Vec::new();
    for target_name in removed {
        let Some(target_detail) = target_detail_map.get(target_name.as_str()) else { continue };
        for source_name in added {
            let Some(source_detail) = source_detail_map.get(source_name.as_str()) else { continue };

            let col_names_source: HashSet<String> = source_detail.columns.iter().map(|c| c.name.clone()).collect();
            let col_names_target: HashSet<String> = target_detail.columns.iter().map(|c| c.name.clone()).collect();
            let column_jaccard = jaccard_similarity(&col_names_target, &col_names_source);

            if column_jaccard < threshold {
                continue;
            }

            let type_sim = column_type_similarity(&target_detail.columns, &source_detail.columns);

            let score = column_jaccard * 0.6 + type_sim * 0.4;

            if score >= threshold {
                candidates.push(RenameCandidate {
                    source_name: source_name.clone(),
                    target_name: target_name.clone(),
                    score,
                    column_jaccard,
                    type_similarity: type_sim,
                });
            }
        }
    }

    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    let mut final_candidates = Vec::new();
    let mut used_target: HashSet<String> = HashSet::new();
    let mut used_source: HashSet<String> = HashSet::new();

    for c in &candidates {
        if !used_target.contains(&c.target_name) && !used_source.contains(&c.source_name) {
            final_candidates.push(c.clone());
            used_target.insert(c.target_name.clone());
            used_source.insert(c.source_name.clone());
        }
    }

    final_candidates
}

// ============================================================================
// Phase 4.2: Batch Naming Pattern Recognition
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPattern {
    pub pattern: String,
    pub is_regex: bool,
    pub description: String,
}

pub fn diff_names_with_patterns(
    source: &[String],
    target: &[String],
    patterns: &[BatchPattern],
) -> (Vec<String>, Vec<String>, Vec<String>, Vec<Vec<String>>) {
    let (added, removed, common) = diff_names(source, target);

    let mut pattern_matches: Vec<Vec<String>> = Vec::new();
    for pattern in patterns {
        let mut matches = Vec::new();
        if pattern.is_regex {
            if let Ok(re) = Regex::new(&pattern.pattern) {
                for name in source {
                    if re.is_match(name) {
                        matches.push(name.clone());
                    }
                }
            }
        } else {
            let glob_pattern = pattern.pattern.replace('*', ".*").replace('?', ".");
            if let Ok(re) = Regex::new(&format!("^{}$", glob_pattern)) {
                for name in source {
                    if re.is_match(name) {
                        matches.push(name.clone());
                    }
                }
            }
        }
        if !matches.is_empty() {
            pattern_matches.push(matches);
        }
    }

    (added, removed, common, pattern_matches)
}

pub fn detect_pattern_conflicts(patterns: &[BatchPattern], names: &[String]) -> Vec<Vec<String>> {
    let mut conflicts = Vec::new();
    for i in 0..patterns.len() {
        for j in (i + 1)..patterns.len() {
            let pi = &patterns[i];
            let pj = &patterns[j];
            let pattern_i =
                if pi.is_regex { pi.pattern.clone() } else { pi.pattern.replace('*', ".*").replace('?', ".") };
            let pattern_j =
                if pj.is_regex { pj.pattern.clone() } else { pj.pattern.replace('*', ".*").replace('?', ".") };

            let re_i = Regex::new(&format!("^{}$", pattern_i));
            let re_j = Regex::new(&format!("^{}$", pattern_j));
            if let (Ok(ri), Ok(rj)) = (re_i, re_j) {
                for name in names {
                    if ri.is_match(name) && rj.is_match(name) {
                        conflicts.push(vec![pi.description.clone(), pj.description.clone()]);
                        break;
                    }
                }
            }
        }
    }
    conflicts
}

// ============================================================================
// Phase 4.3: Dialect-Aware Type Compatibility Scoring
// ============================================================================

pub fn diff_columns_with_compatibility(
    source: &[ColumnInfo],
    target: &[ColumnInfo],
    ignore_comments: bool,
    compare_column_order: bool,
    source_dialect: DialectKind,
    target_dialect: DialectKind,
    compatibility_threshold: f64,
    field_mappings: &[FieldMapping],
) -> (Vec<ColumnDiff>, Vec<ColumnCompatibilityWarning>) {
    diff_columns_with_compatibility_options(
        source,
        target,
        ignore_comments,
        compare_column_order,
        source_dialect,
        target_dialect,
        compatibility_threshold,
        field_mappings,
        false,
        source_dialect == DialectKind::Mysql && target_dialect == DialectKind::Mysql,
    )
}

#[allow(clippy::too_many_arguments)]
fn diff_columns_with_compatibility_options(
    source: &[ColumnInfo],
    target: &[ColumnInfo],
    ignore_comments: bool,
    compare_column_order: bool,
    source_dialect: DialectKind,
    target_dialect: DialectKind,
    compatibility_threshold: f64,
    field_mappings: &[FieldMapping],
    ignore_column_name_case: bool,
    compare_charset: bool,
) -> (Vec<ColumnDiff>, Vec<ColumnCompatibilityWarning>) {
    use crate::sql_dialect::descriptor::TypeMappingMatrix;

    let matrix = TypeMappingMatrix::for_dialects(source_dialect, target_dialect);
    let engine = DefaultTypeInferenceEngine;

    let basic_diffs = diff_columns_with_identifier_options(
        source,
        target,
        ignore_comments,
        compare_column_order,
        false,
        0.5,
        None,
        None,
        ignore_column_name_case,
        compare_charset,
    );

    let mut warnings = Vec::new();
    let mut enhanced_diffs = Vec::new();

    for diff in basic_diffs {
        let mut warning = None;

        if diff.diff_type == "modified" {
            if let (Some(src), Some(tgt)) = (&diff.source, &diff.target) {
                let src_parsed = ColumnType::parse(&src.data_type);
                let tgt_parsed = ColumnType::parse(&tgt.data_type);
                let compatibility = engine.type_compatibility_score(&src_parsed, &tgt_parsed);

                let (mapped_type, requires_cast) = if let Some(user_target) =
                    FieldMapping::apply_with_params(field_mappings, &src.data_type, target_dialect)
                {
                    (user_target, false)
                } else {
                    matrix.convert_type(&tgt.data_type)
                };

                let risk = if compatibility >= 0.9 {
                    ColumnConversionRisk::None
                } else if compatibility >= 0.7 {
                    ColumnConversionRisk::Low
                } else if compatibility >= 0.5 {
                    ColumnConversionRisk::Medium
                } else {
                    ColumnConversionRisk::High
                };

                if compatibility < compatibility_threshold {
                    warning = Some(ColumnCompatibilityWarning {
                        column_name: diff.name.clone(),
                        source_type: src.data_type.clone(),
                        target_type: tgt.data_type.clone(),
                        compatibility_score: compatibility,
                        suggested_mapping: mapped_type,
                        requires_cast,
                        risk,
                    });
                }
            }
        }

        enhanced_diffs.push(diff);
        if let Some(w) = warning {
            warnings.push(w);
        }
    }

    (enhanced_diffs, warnings)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnCompatibilityWarning {
    pub column_name: String,
    pub source_type: String,
    pub target_type: String,
    pub compatibility_score: f64,
    pub suggested_mapping: String,
    pub requires_cast: bool,
    pub risk: ColumnConversionRisk,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ColumnConversionRisk {
    None,
    Low,
    Medium,
    High,
}

// ============================================================================
// Phase 4.4: Bidirectional Diff & Rollback Graph
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffNode {
    pub table_diff: TableDiff,
    pub direction: DiffDirection,
    pub dependency_order: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DiffDirection {
    Forward,
    Rollback,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackGraph {
    pub forward_nodes: Vec<DiffNode>,
    pub rollback_nodes: Vec<DiffNode>,
    pub is_consistent: bool,
    pub consistency_issues: Vec<String>,
}

impl RollbackGraph {
    pub fn from_forward_diffs(
        forward_diffs: &[TableDiff],
        renames: &[RenameCandidate],
        dep_graph: &DependencyGraph,
    ) -> Self {
        let mut forward_nodes = Vec::new();
        let mut rollback_nodes = Vec::new();
        let consistency_issues = Vec::new();

        let rename_map: HashMap<&str, &RenameCandidate> = renames.iter().map(|r| (r.target_name.as_str(), r)).collect();
        let rename_reverse: HashMap<&str, &str> =
            renames.iter().map(|r| (r.source_name.as_str(), r.target_name.as_str())).collect();

        let order_map: HashMap<&str, usize> =
            dep_graph.topological_order.iter().enumerate().map(|(i, name)| (name.as_str(), i)).collect();

        for diff in forward_diffs {
            let order = order_map.get(diff.name.as_str()).copied().unwrap_or(usize::MAX);

            let (rename_source, rename_target, rename_score) = if diff.diff_type == "added" {
                if let Some(rc) = rename_reverse.get(diff.name.as_str()) {
                    (Some(rc.to_string()), Some(diff.name.clone()), None)
                } else {
                    (None, None, None)
                }
            } else if diff.diff_type == "removed" {
                if let Some(rc) = rename_map.get(diff.name.as_str()) {
                    (Some(diff.name.clone()), Some(rc.source_name.clone()), Some(rc.score))
                } else {
                    (None, None, None)
                }
            } else {
                (None, None, None)
            };

            forward_nodes.push(DiffNode {
                table_diff: diff.clone(),
                direction: DiffDirection::Forward,
                dependency_order: order,
                rename_source,
                rename_target,
                rename_score,
            });

            let rollback_diff = Self::invert_diff(diff);
            rollback_nodes.push(DiffNode {
                table_diff: rollback_diff,
                direction: DiffDirection::Rollback,
                dependency_order: order,
                rename_source: None,
                rename_target: None,
                rename_score: None,
            });
        }

        RollbackGraph { forward_nodes, rollback_nodes, is_consistent: false, consistency_issues }
    }

    fn invert_diff_type(dt: &str) -> &str {
        match dt {
            "added" => "removed",
            "removed" => "added",
            "renamed" => "renamed",
            _ => "modified",
        }
    }

    fn invert_change_string(ch: &str) -> String {
        let Some((before, after)) = ch.split_once(" → ") else {
            return ch.to_string();
        };
        if let Some((kind, value)) = before.split_once(": ") {
            format!("{kind}: {after} → {value}")
        } else {
            format!("{after} → {before}")
        }
    }

    fn invert_columns(cols: &[ColumnDiff]) -> Vec<ColumnDiff> {
        cols.iter()
            .map(|c| {
                let inverted_name = if c.diff_type == "renamed" {
                    c.target.as_ref().map(|t| t.name.clone()).unwrap_or_else(|| c.name.clone())
                } else {
                    c.name.clone()
                };
                ColumnDiff {
                    diff_type: Self::invert_diff_type(&c.diff_type).to_string(),
                    name: inverted_name,
                    source: c.target.clone(),
                    target: c.source.clone(),
                    changes: c.changes.iter().map(|ch| Self::invert_change_string(ch)).collect(),
                    add_position: c.add_position.clone(),
                }
            })
            .collect()
    }

    fn invert_indexes(idxs: &[IndexDiff]) -> Vec<IndexDiff> {
        idxs.iter()
            .map(|i| IndexDiff {
                diff_type: Self::invert_diff_type(&i.diff_type).to_string(),
                name: i.name.clone(),
                source: i.target.clone(),
                target: i.source.clone(),
                changes: i.changes.clone(),
            })
            .collect()
    }

    fn invert_fks(fks: &[ForeignKeyDiff]) -> Vec<ForeignKeyDiff> {
        fks.iter()
            .map(|fk| ForeignKeyDiff {
                diff_type: Self::invert_diff_type(&fk.diff_type).to_string(),
                name: fk.name.clone(),
                source: fk.target.clone(),
                target: fk.source.clone(),
                changes: fk.changes.clone(),
            })
            .collect()
    }

    fn invert_triggers(trgs: &[TriggerDiff]) -> Vec<TriggerDiff> {
        trgs.iter()
            .map(|t| TriggerDiff {
                diff_type: Self::invert_diff_type(&t.diff_type).to_string(),
                name: t.name.clone(),
                source: t.target.clone(),
                target: t.source.clone(),
                changes: t.changes.clone(),
            })
            .collect()
    }

    fn invert_diff(diff: &TableDiff) -> TableDiff {
        let inverted_type = Self::invert_diff_type(&diff.diff_type).to_string();

        let inverted_columns = diff.columns.as_ref().map(|cols| Self::invert_columns(cols));
        let inverted_indexes = diff.indexes.as_ref().map(|idxs| Self::invert_indexes(idxs));
        let inverted_fks = diff.foreign_keys.as_ref().map(|fks| Self::invert_fks(fks));
        let inverted_triggers = diff.triggers.as_ref().map(|trgs| Self::invert_triggers(trgs));

        let (source_comment, target_comment) = match inverted_type.as_str() {
            "added" => (diff.target_table_comment.clone(), diff.source_table_comment.clone()),
            "removed" => (diff.source_table_comment.clone(), diff.target_table_comment.clone()),
            _ => (diff.target_table_comment.clone(), diff.source_table_comment.clone()),
        };
        let recreates_removed_table =
            diff.diff_type == "removed" && inverted_type == "added" && diff.object_type.as_deref() == Some("table");

        TableDiff {
            diff_type: inverted_type,
            object_type: diff.object_type.clone(),
            target_name: diff.target_name.clone(),
            name: diff.name.clone(),
            columns: inverted_columns,
            indexes: inverted_indexes,
            foreign_keys: inverted_fks,
            triggers: inverted_triggers,
            // Rollback recreation must use the structured snapshot first. Keep
            // native target DDL isolated as a same-target-dialect fallback.
            ddl: if recreates_removed_table { None } else { diff.target_ddl.clone() },
            target_ddl: if recreates_removed_table { diff.target_ddl.clone() } else { diff.ddl.clone() },
            source_table_comment: source_comment,
            target_table_comment: target_comment,
            sync_sql: None,
        }
    }

    pub fn validate_consistency(&mut self) -> bool {
        self.consistency_issues.clear();

        for fwd in &self.forward_nodes {
            let has_rollback = self.rollback_nodes.iter().any(|rbk| {
                rbk.table_diff.name == fwd.table_diff.name
                    && matches!(
                        (fwd.table_diff.diff_type.as_str(), rbk.table_diff.diff_type.as_str()),
                        ("added", "removed") | ("removed", "added") | ("modified", "modified") | ("none", "none")
                    )
            });

            if !has_rollback {
                self.consistency_issues.push(format!(
                    "No rollback entry for forward {}: {}",
                    fwd.table_diff.diff_type, fwd.table_diff.name
                ));
            }

            let rollback_of_rollback: Vec<_> = self
                .rollback_nodes
                .iter()
                .filter(|rbk| rbk.table_diff.name == fwd.table_diff.name)
                .map(|rbk| Self::invert_diff(&rbk.table_diff))
                .collect();

            for ror in &rollback_of_rollback {
                if ror.diff_type != fwd.table_diff.diff_type {
                    self.consistency_issues.push(format!(
                        "Forward∘Rollback mismatch for {}: forward={}, rollback∘rollback={}",
                        fwd.table_diff.name, fwd.table_diff.diff_type, ror.diff_type
                    ));
                }
            }
        }

        self.is_consistent = self.consistency_issues.is_empty();
        self.is_consistent
    }
}

pub fn generate_rollback_sync_sql(
    rollback_graph: &RollbackGraph,
    db_type: DatabaseType,
    schema: Option<&str>,
    cascade_delete: bool,
) -> String {
    generate_rollback_sync_sql_with_missing(rollback_graph, db_type, schema, cascade_delete).0
}

pub fn generate_rollback_sync_sql_with_missing(
    rollback_graph: &RollbackGraph,
    db_type: DatabaseType,
    schema: Option<&str>,
    cascade_delete: bool,
) -> (String, Vec<MissingRollbackObject>) {
    let rollback_diffs: Vec<TableDiff> = rollback_graph.rollback_nodes.iter().map(|n| n.table_diff.clone()).collect();
    generate_schema_sync_sql_inner(&rollback_diffs, &[], &[], &[], &[], db_type, schema, cascade_delete, None, &[])
}

// ============================================================================
// Phase 4.5: Shard-Parallel Comparison
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardStrategy {
    pub shard_count: usize,
    pub shard_by: ShardBy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShardBy {
    Table,
    Schema,
    RoundRobin,
}

pub fn shard_diff(options: &SchemaDiffPreparationOptions, shard_strategy: &ShardStrategy) -> Vec<TableDiff> {
    let table_count = options.source_tables.len().max(options.target_tables.len());
    let shard_count = shard_strategy.shard_count.min(table_count.max(1));

    if shard_count <= 1 {
        return diff_schema(options);
    }

    let source_table_names: Vec<&str> =
        options.source_tables.iter().filter(|t| !t.table_type.contains("VIEW")).map(|t| t.name.as_str()).collect();
    let source_view_names: Vec<&str> =
        options.source_tables.iter().filter(|t| t.table_type.contains("VIEW")).map(|t| t.name.as_str()).collect();
    let _target_table_names: Vec<&str> =
        options.target_tables.iter().filter(|t| !t.table_type.contains("VIEW")).map(|t| t.name.as_str()).collect();
    let _target_view_names: Vec<&str> =
        options.target_tables.iter().filter(|t| t.table_type.contains("VIEW")).map(|t| t.name.as_str()).collect();

    let source_all: Vec<&str> = source_table_names.iter().chain(source_view_names.iter()).copied().collect();
    let shards: Vec<Vec<&str>> = match &shard_strategy.shard_by {
        ShardBy::Table | ShardBy::RoundRobin => {
            let mut s: Vec<Vec<&str>> = vec![Vec::new(); shard_count];
            for (i, name) in source_all.iter().enumerate() {
                s[i % shard_count].push(*name);
            }
            s
        }
        ShardBy::Schema => {
            let mut schema_groups: HashMap<&str, Vec<&str>> = HashMap::new();
            for table in &options.source_tables {
                let schema = table.parent_schema.as_deref().unwrap_or("default");
                schema_groups.entry(schema).or_default().push(table.name.as_str());
            }
            let mut s: Vec<Vec<&str>> = vec![Vec::new(); shard_count];
            for (i, (_schema, names)) in schema_groups.iter().enumerate() {
                s[i % shard_count].extend(names);
            }
            s
        }
    };

    let shard_results: Vec<Vec<TableDiff>> = shards
        .par_iter()
        .filter(|shard| !shard.is_empty())
        .map(|shard| {
            let shard_set: HashSet<&str> = shard.iter().copied().collect();
            let shard_options = SchemaDiffPreparationOptions {
                source_tables: options
                    .source_tables
                    .iter()
                    .filter(|t| shard_set.contains(t.name.as_str()))
                    .cloned()
                    .collect(),
                target_tables: options
                    .target_tables
                    .iter()
                    .filter(|t| {
                        shard_set.contains(t.name.as_str())
                            || options.table_mappings.iter().any(|mapping| {
                                shard_set.contains(mapping.source_table.as_str()) && mapping.target_table == t.name
                            })
                    })
                    .cloned()
                    .collect(),
                source_details: options
                    .source_details
                    .iter()
                    .filter(|d| shard_set.contains(d.name.as_str()))
                    .cloned()
                    .collect(),
                target_details: options
                    .target_details
                    .iter()
                    .filter(|d| {
                        shard_set.contains(d.name.as_str())
                            || options.table_mappings.iter().any(|mapping| {
                                shard_set.contains(mapping.source_table.as_str()) && mapping.target_table == d.name
                            })
                    })
                    .cloned()
                    .collect(),
                source_functions: options.source_functions.clone(),
                target_functions: options.target_functions.clone(),
                source_sequences: options.source_sequences.clone(),
                target_sequences: options.target_sequences.clone(),
                source_rules: options.source_rules.clone(),
                target_rules: options.target_rules.clone(),
                source_owners: options.source_owners.clone(),
                target_owners: options.target_owners.clone(),
                database_type: options.database_type,
                target_schema: options.target_schema.clone(),
                ignore_comments: options.ignore_comments,
                cascade_delete: options.cascade_delete,
                compare_column_order: options.compare_column_order,
                compare_charset: options.compare_charset,
                ignore_table_name_case: options.ignore_table_name_case,
                ignore_column_name_case: options.ignore_column_name_case,
                source_dialect: options.source_dialect,
                target_dialect: options.target_dialect,
                table_mappings: options.table_mappings.clone(),
                ..Default::default()
            };
            diff_schema(&shard_options)
        })
        .collect();

    let mut merged: Vec<TableDiff> = Vec::new();
    for shard_result in shard_results {
        merged.extend(shard_result);
    }

    merged.sort_by(|a, b| a.name.cmp(&b.name));
    merged.dedup_by(|a, b| a.name == b.name && a.diff_type == b.diff_type);
    merged
}

// ============================================================================
// Phase 4.6: Permission & Role-Aware Sync
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionInfo {
    pub grantee: String,
    pub object_type: String,
    pub object_name: String,
    pub privilege: String,
    pub is_grantable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDiff {
    #[serde(rename = "type")]
    pub diff_type: String,
    pub grantee: String,
    pub object_name: String,
    pub privilege: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<PermissionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<PermissionInfo>,
}

pub fn diff_permissions(source: &[PermissionInfo], target: &[PermissionInfo]) -> Vec<PermissionDiff> {
    let mut diffs = Vec::new();
    let target_map: HashMap<(&str, &str, &str), &PermissionInfo> =
        target.iter().map(|p| ((p.grantee.as_str(), p.object_name.as_str(), p.privilege.as_str()), p)).collect();
    let source_map: HashMap<(&str, &str, &str), &PermissionInfo> =
        source.iter().map(|p| ((p.grantee.as_str(), p.object_name.as_str(), p.privilege.as_str()), p)).collect();

    for sp in source {
        let key = (sp.grantee.as_str(), sp.object_name.as_str(), sp.privilege.as_str());
        if !target_map.contains_key(&key) {
            diffs.push(PermissionDiff {
                diff_type: "added".to_string(),
                grantee: sp.grantee.clone(),
                object_name: sp.object_name.clone(),
                privilege: sp.privilege.clone(),
                source: Some(sp.clone()),
                target: None,
            });
        }
    }

    for tp in target {
        let key = (tp.grantee.as_str(), tp.object_name.as_str(), tp.privilege.as_str());
        if !source_map.contains_key(&key) {
            diffs.push(PermissionDiff {
                diff_type: "removed".to_string(),
                grantee: tp.grantee.clone(),
                object_name: tp.object_name.clone(),
                privilege: tp.privilege.clone(),
                source: None,
                target: Some(tp.clone()),
            });
        }
    }

    diffs
}

pub fn generate_permission_sync_sql(diffs: &[PermissionDiff], db_type: DatabaseType, schema: Option<&str>) -> String {
    let mut lines: Vec<String> = Vec::new();
    let profile = profile_for(db_type);

    for diff in diffs {
        match diff.diff_type.as_str() {
            "added" => {
                if let Some(source) = &diff.source {
                    if profile.grant_uses_mysql_user_syntax {
                        let object_path = if let Some(sch) = schema {
                            format!("`{}`.`{}`", sch.replace('`', "``"), source.object_name.replace('`', "``"))
                        } else {
                            format!("`{}`", source.object_name.replace('`', "``"))
                        };
                        let with_grant = if source.is_grantable { " WITH GRANT OPTION" } else { "" };
                        let grantee_escaped = source.grantee.replace('\'', "''");
                        lines.push(format!(
                            "GRANT {} ON {} TO '{}'{};",
                            source.privilege, object_path, grantee_escaped, with_grant
                        ));
                    } else {
                        let obj_escaped = source.object_name.replace('"', "\"\"");
                        let object_path = if let Some(sch) = schema {
                            format!("{} \"{}\".\"{}\"", source.object_type, sch, obj_escaped)
                        } else {
                            format!("{} \"{}\"", source.object_type, obj_escaped)
                        };
                        let with_grant = if source.is_grantable { " WITH GRANT OPTION" } else { "" };
                        let grantee_escaped = source.grantee.replace('"', "\"\"");
                        lines.push(format!(
                            "GRANT {} ON {} TO \"{}\"{};",
                            source.privilege, object_path, grantee_escaped, with_grant
                        ));
                    }
                }
            }
            "removed" => {
                if let Some(target) = &diff.target {
                    if profile.grant_uses_mysql_user_syntax {
                        let object_path = if let Some(sch) = schema {
                            format!("`{}`.`{}`", sch.replace('`', "``"), target.object_name.replace('`', "``"))
                        } else {
                            format!("`{}`", target.object_name.replace('`', "``"))
                        };
                        let grantee_escaped = target.grantee.replace('\'', "''");
                        lines.push(format!(
                            "REVOKE {} ON {} FROM '{}';",
                            target.privilege, object_path, grantee_escaped
                        ));
                    } else {
                        let obj_escaped = target.object_name.replace('"', "\"\"");
                        let object_path = if let Some(sch) = schema {
                            format!("{} \"{}\".\"{}\"", target.object_type, sch, obj_escaped)
                        } else {
                            format!("{} \"{}\"", target.object_type, obj_escaped)
                        };
                        let grantee_escaped = target.grantee.replace('"', "\"\"");
                        lines.push(format!(
                            "REVOKE {} ON {} FROM \"{}\";",
                            target.privilege, object_path, grantee_escaped
                        ));
                    }
                }
            }
            _ => {}
        }
    }

    lines.join("\n")
}

// ============================================================================
// Phase 4.7: Metadata Resource-Aware Scheduling
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceConstraint {
    pub max_concurrent_connections: usize,
    pub max_memory_mb: u64,
    pub max_tables_per_batch: usize,
    pub throttle_delay_ms: u64,
}

impl Default for ResourceConstraint {
    fn default() -> Self {
        Self { max_concurrent_connections: 4, max_memory_mb: 512, max_tables_per_batch: 50, throttle_delay_ms: 100 }
    }
}

#[derive(Debug, Clone)]
pub struct AdaptiveScheduler {
    pub constraint: ResourceConstraint,
    pub current_connections: usize,
    pub estimated_table_count: usize,
}

impl AdaptiveScheduler {
    pub fn new(constraint: ResourceConstraint, table_count: usize) -> Self {
        Self { constraint, current_connections: 0, estimated_table_count: table_count }
    }

    pub fn optimal_batch_size(&self) -> usize {
        let conn_limit = self.constraint.max_concurrent_connections;
        let mem_limit = self.constraint.max_memory_mb as usize * 50;
        let table_limit = self.constraint.max_tables_per_batch;

        let batches = self.estimated_table_count.max(1);
        let per_batch = (self.estimated_table_count / conn_limit).max(1);

        per_batch.min(mem_limit / batches).min(table_limit)
    }

    pub fn recommended_shard_count(&self) -> usize {
        let per_batch = self.optimal_batch_size();
        let count = (self.estimated_table_count as f64 / per_batch as f64).ceil() as usize;
        count.min(self.constraint.max_concurrent_connections).max(1)
    }

    pub fn throttle_delay_ms(&self) -> u64 {
        self.constraint.throttle_delay_ms
    }
}

// ============================================================================
// Phase 4: Extended SchemaDiffPreparationOptions & SchemaDiffPreparation
// ============================================================================

impl Default for SchemaDiffPreparationOptions {
    fn default() -> Self {
        Self {
            source_tables: Vec::new(),
            target_tables: Vec::new(),
            source_details: Vec::new(),
            target_details: Vec::new(),
            source_functions: Vec::new(),
            target_functions: Vec::new(),
            source_sequences: Vec::new(),
            target_sequences: Vec::new(),
            source_rules: Vec::new(),
            target_rules: Vec::new(),
            source_owners: Vec::new(),
            target_owners: Vec::new(),
            database_type: DatabaseType::Mysql,
            target_schema: None,
            ignore_comments: false,
            cascade_delete: false,
            compare_column_order: false,
            compare_charset: true,
            ignore_table_name_case: false,
            ignore_column_name_case: false,
            detect_renames: false,
            detect_table_renames: false,
            rename_threshold: 0.5,
            enable_rollback: false,
            batch_patterns: Vec::new(),
            source_dialect: None,
            target_dialect: None,
            compatibility_threshold: 0.5,
            source_permissions: Vec::new(),
            target_permissions: Vec::new(),
            shard_strategy: None,
            resource_constraint: None,
            field_mappings: Vec::new(),
            table_mappings: Vec::new(),
        }
    }
}

// Add new optional fields to SchemaDiffPreparationOptions
// These are added as separate impl blocks to avoid breaking existing construction sites
impl SchemaDiffPreparationOptions {
    pub fn with_rename_detection(mut self, detect: bool, threshold: f64) -> Self {
        self.detect_renames = detect;
        self.rename_threshold = threshold;
        self
    }

    pub fn with_rollback(mut self, enable: bool) -> Self {
        self.enable_rollback = enable;
        self
    }

    pub fn with_batch_patterns(mut self, patterns: Vec<BatchPattern>) -> Self {
        self.batch_patterns = patterns;
        self
    }

    pub fn with_dialects(mut self, source: Option<DialectKind>, target: Option<DialectKind>) -> Self {
        self.source_dialect = source;
        self.target_dialect = target;
        self
    }

    pub fn with_compatibility_threshold(mut self, threshold: f64) -> Self {
        self.compatibility_threshold = threshold;
        self
    }

    pub fn with_permissions(mut self, source: Vec<PermissionInfo>, target: Vec<PermissionInfo>) -> Self {
        self.source_permissions = source;
        self.target_permissions = target;
        self
    }

    pub fn with_shard_strategy(mut self, strategy: ShardStrategy) -> Self {
        self.shard_strategy = Some(strategy);
        self
    }

    pub fn with_resource_constraint(mut self, constraint: ResourceConstraint) -> Self {
        self.resource_constraint = Some(constraint);
        self
    }

    pub fn with_field_mappings(mut self, mappings: Vec<FieldMapping>) -> Self {
        self.field_mappings = mappings;
        self
    }

    pub fn with_table_mappings(mut self, mappings: Vec<SchemaDiffTableMapping>) -> Self {
        self.table_mappings = mappings;
        self
    }
}

pub fn prepare_schema_diff(options: SchemaDiffPreparationOptions) -> SchemaDiffPreparation {
    if !options.field_mappings.is_empty() {
        log::info!("prepare_schema_diff field_mappings:");
        for m in &options.field_mappings {
            log::info!(
                "  {} -> {} (strategy={:?}, custom={:?})",
                m.source_type,
                m.target_type,
                m.param_strategy,
                m.custom_params
            );
        }
        log::info!("  source_dialect={:?} target_dialect={:?}", options.source_dialect, options.target_dialect);
    }

    let dialect_str = options.source_dialect.map(|d| d.label().to_string()).unwrap_or_else(|| "generic".to_string());
    let options = AstTransmitFilter::filter_diff_preparation_options(options, &dialect_str);

    let dep_graph = DependencyGraph::build_with_options(
        &options.source_details,
        &options.source_tables,
        options.ignore_table_name_case,
    );

    let mut diffs = if let Some(ref strategy) = options.shard_strategy {
        shard_diff(&options, strategy)
    } else {
        diff_schema(&options)
    };

    let rename_candidates = if options.detect_renames && options.detect_table_renames {
        let removed: Vec<String> = diffs.iter().filter(|d| d.diff_type == "removed").map(|d| d.name.clone()).collect();
        let added: Vec<String> = diffs.iter().filter(|d| d.diff_type == "added").map(|d| d.name.clone()).collect();
        let candidates = detect_renames(
            &removed,
            &added,
            &options.source_details,
            &options.target_details,
            options.rename_threshold,
        );

        let target_renamed: HashSet<&str> = candidates.iter().map(|r| r.target_name.as_str()).collect();
        let source_renamed: HashSet<&str> = candidates.iter().map(|r| r.source_name.as_str()).collect();

        diffs.retain(|d| {
            !((d.diff_type == "removed" && target_renamed.contains(d.name.as_str()))
                || (d.diff_type == "added" && source_renamed.contains(d.name.as_str())))
        });

        for c in &candidates {
            let source_detail = options.source_details.iter().find(|d| d.name == c.source_name);
            let target_detail = options.target_details.iter().find(|d| d.name == c.target_name);
            diffs.push(TableDiff {
                diff_type: "renamed".to_string(),
                object_type: Some("table".to_string()),
                target_name: Some(c.target_name.clone()),
                name: c.source_name.clone(),
                columns: None,
                indexes: None,
                foreign_keys: None,
                triggers: None,
                ddl: source_detail.and_then(|d| d.ddl.clone()),
                target_ddl: target_detail.and_then(|d| d.ddl.clone()),
                source_table_comment: None,
                target_table_comment: None,
                sync_sql: None,
            });
        }

        candidates
    } else {
        Vec::new()
    };

    let compatibility_warnings = if options.source_dialect.is_some() || options.target_dialect.is_some() {
        let src_dialect = options.source_dialect.unwrap_or(DialectKind::Mysql);
        let tgt_dialect = options.target_dialect.unwrap_or(DialectKind::Mysql);
        let mut all_warnings = Vec::new();
        for diff in &diffs {
            if diff.diff_type == "modified" {
                if let Some(source_detail) = options.source_details.iter().find(|d| d.name == diff.name) {
                    let target_name = diff.target_name.as_deref().unwrap_or(&diff.name);
                    if let Some(target_detail) = options.target_details.iter().find(|d| d.name == target_name) {
                        let (_, warnings) = diff_columns_with_compatibility_options(
                            &source_detail.columns,
                            &target_detail.columns,
                            options.ignore_comments,
                            options.compare_column_order,
                            src_dialect,
                            tgt_dialect,
                            options.compatibility_threshold,
                            &options.field_mappings,
                            options.ignore_column_name_case,
                            compare_mysql_column_charset(&options),
                        );
                        all_warnings.extend(warnings);
                    }
                }
            }
        }
        all_warnings
    } else {
        Vec::new()
    };

    let rollback_graph = if options.enable_rollback {
        let mut graph = RollbackGraph::from_forward_diffs(&diffs, &rename_candidates, &dep_graph);
        let _ = graph.validate_consistency();
        Some(graph)
    } else {
        None
    };

    let function_diffs = diff_functions(&options.source_functions, &options.target_functions);
    let sequence_diffs = diff_sequences(&options.source_sequences, &options.target_sequences);
    let rule_diffs = diff_rules(&options.source_rules, &options.target_rules);
    let owner_diffs = diff_owners(&options.source_owners, &options.target_owners);

    for diff in &mut diffs {
        let (sync_sql, _) = generate_schema_sync_sql_inner(
            std::slice::from_ref(diff),
            &[],
            &[],
            &[],
            &[],
            options.database_type,
            options.target_schema.as_deref(),
            options.cascade_delete,
            options.source_dialect,
            &options.field_mappings,
        );
        if !sync_sql.is_empty() {
            diff.sync_sql = Some(sync_sql);
        }
    }

    let (sync_sql, _) = generate_schema_sync_sql_inner(
        &diffs,
        &function_diffs,
        &sequence_diffs,
        &rule_diffs,
        &owner_diffs,
        options.database_type,
        options.target_schema.as_deref(),
        options.cascade_delete,
        options.source_dialect,
        &options.field_mappings,
    );

    let (rollback_sync_sql, missing_rollback_objects) = match &rollback_graph {
        Some(graph) => {
            let (sql, missing) = generate_rollback_sync_sql_with_missing(
                graph,
                options.database_type,
                options.target_schema.as_deref(),
                options.cascade_delete,
            );
            (Some(sql), missing)
        }
        None => (None, Vec::new()),
    };
    let rollback_completeness = if missing_rollback_objects.is_empty() {
        RollbackCompleteness::Complete
    } else {
        RollbackCompleteness::Incomplete
    };

    let permission_diffs = if !options.source_permissions.is_empty() || !options.target_permissions.is_empty() {
        diff_permissions(&options.source_permissions, &options.target_permissions)
    } else {
        Vec::new()
    };

    let permission_sync_sql = if !permission_diffs.is_empty() {
        Some(generate_permission_sync_sql(&permission_diffs, options.database_type, options.target_schema.as_deref()))
    } else {
        None
    };

    SchemaDiffPreparation {
        diffs,
        function_diffs,
        sequence_diffs,
        rule_diffs,
        owner_diffs,
        sync_sql,
        rollback_sync_sql,
        rollback_completeness,
        missing_rollback_objects,
        rename_candidates,
        rollback_graph,
        compatibility_warnings,
        permission_diffs,
        permission_sync_sql,
        dependency_graph: Some(dep_graph),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SchemaDiffTableNameResolution {
    pairs: Vec<(String, String)>,
    source_only: Vec<String>,
    target_only: Vec<String>,
}

fn identifiers_equal(left: &str, right: &str, ignore_case: bool) -> bool {
    if ignore_case {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

fn resolve_schema_diff_table_names(
    source_names: &[String],
    target_names: &[String],
    mappings: &[SchemaDiffTableMapping],
    ignore_table_name_case: bool,
) -> SchemaDiffTableNameResolution {
    let target_set: HashSet<&str> = target_names.iter().map(String::as_str).collect();
    let mut explicit_by_source: HashMap<&str, &str> = HashMap::new();
    for mapping in mappings {
        if !mapping.source_table.is_empty() && !mapping.target_table.is_empty() {
            explicit_by_source.entry(mapping.source_table.as_str()).or_insert(mapping.target_table.as_str());
        }
    }

    // Reserve valid explicit targets before automatic matching so an explicit mapping
    // wins even when another source table appears earlier in the input list.
    let mut explicit_targets_by_source = HashMap::new();
    let mut reserved_explicit_targets = HashSet::new();
    for source_name in source_names {
        let Some(target_name) = explicit_by_source.get(source_name.as_str()).copied() else { continue };
        if target_set.contains(target_name) && reserved_explicit_targets.insert(target_name) {
            explicit_targets_by_source.insert(source_name.clone(), target_name.to_string());
        }
    }

    let mut resolved_targets_by_source = explicit_targets_by_source.clone();
    let mut used_targets: HashSet<&str> = reserved_explicit_targets.clone();

    // Resolve every exact match before considering case-insensitive candidates.
    // This keeps an exact match from losing its target to an earlier case-only match.
    for source_name in source_names {
        if resolved_targets_by_source.contains_key(source_name) {
            continue;
        }
        if target_set.contains(source_name.as_str()) && used_targets.insert(source_name.as_str()) {
            resolved_targets_by_source.insert(source_name.clone(), source_name.clone());
        }
    }

    if ignore_table_name_case {
        for source_name in source_names {
            if resolved_targets_by_source.contains_key(source_name) {
                continue;
            }
            let mut candidates = target_names.iter().filter(|target_name| {
                !used_targets.contains(target_name.as_str()) && identifiers_equal(source_name, target_name, true)
            });
            let candidate = candidates.next();
            if let Some(target_name) = candidate.filter(|_| candidates.next().is_none()) {
                used_targets.insert(target_name.as_str());
                resolved_targets_by_source.insert(source_name.clone(), target_name.clone());
            }
        }
    }

    let mut pairs = Vec::new();
    let mut source_only = Vec::new();
    for source_name in source_names {
        if let Some(target_name) = resolved_targets_by_source.get(source_name) {
            pairs.push((source_name.clone(), target_name.clone()));
        } else {
            source_only.push(source_name.clone());
        }
    }

    let target_only = target_names.iter().filter(|name| !used_targets.contains(name.as_str())).cloned().collect();
    SchemaDiffTableNameResolution { pairs, source_only, target_only }
}

fn compare_mysql_column_charset(options: &SchemaDiffPreparationOptions) -> bool {
    if !options.compare_charset || !matches!(options.database_type, DatabaseType::Mysql) {
        return false;
    }

    let target_dialect =
        options.target_dialect.unwrap_or_else(|| DialectKind::from_database_type(options.database_type));
    let source_dialect = options.source_dialect.unwrap_or(target_dialect);
    source_dialect == DialectKind::Mysql && target_dialect == DialectKind::Mysql
}

fn diff_schema(options: &SchemaDiffPreparationOptions) -> Vec<TableDiff> {
    let source_details: HashMap<&str, &TableSchemaDetail> =
        options.source_details.iter().map(|detail| (detail.name.as_str(), detail)).collect();
    let target_details: HashMap<&str, &TableSchemaDetail> =
        options.target_details.iter().map(|detail| (detail.name.as_str(), detail)).collect();
    let source_table_comments: HashMap<&str, Option<String>> =
        options.source_tables.iter().map(|table| (table.name.as_str(), table.comment.clone())).collect();
    let target_table_comments: HashMap<&str, Option<String>> =
        options.target_tables.iter().map(|table| (table.name.as_str(), table.comment.clone())).collect();

    let source_table_names: Vec<String> = options
        .source_tables
        .iter()
        .filter(|table| !table.table_type.contains("VIEW"))
        .map(|table| table.name.clone())
        .collect();
    let target_table_names: Vec<String> = options
        .target_tables
        .iter()
        .filter(|table| !table.table_type.contains("VIEW"))
        .map(|table| table.name.clone())
        .collect();
    let source_view_names: Vec<String> = options
        .source_tables
        .iter()
        .filter(|table| table.table_type.contains("VIEW"))
        .map(|table| table.name.clone())
        .collect();
    let target_view_names: Vec<String> = options
        .target_tables
        .iter()
        .filter(|table| table.table_type.contains("VIEW"))
        .map(|table| table.name.clone())
        .collect();

    let table_resolution = resolve_schema_diff_table_names(
        &source_table_names,
        &target_table_names,
        &options.table_mappings,
        options.ignore_table_name_case,
    );
    let view_resolution = resolve_schema_diff_table_names(
        &source_view_names,
        &target_view_names,
        &options.table_mappings,
        options.ignore_table_name_case,
    );
    let table_pairs = table_resolution.pairs.clone();
    let mut result = Vec::new();

    // A foreign key whose `ref_table` is itself one of the tables being compared is a
    // same-database self-reference. Its `ref_schema` is always the literal source/target
    // database name (MySQL's information_schema reports it unconditionally, even for
    // self-references), so source and target will almost always disagree even though the
    // relationship is structurally identical. Clearing it here makes such FKs compare and
    // regenerate against the *other side's own* database instead of being flagged as
    // "different" and then rewritten to literally reference the source database name.
    // Genuine cross-database references (ref_table not part of this database) are left as-is.
    let source_table_name_set: HashSet<&str> = source_table_names.iter().map(String::as_str).collect();
    let target_table_name_set: HashSet<&str> = target_table_names.iter().map(String::as_str).collect();

    for name in table_resolution.source_only {
        let source_detail = source_details.get(name.as_str());
        result.push(TableDiff {
            diff_type: "added".to_string(),
            object_type: Some("table".to_string()),
            target_name: None,
            name,
            ddl: source_detail.and_then(|detail| detail.ddl.clone()),
            target_ddl: None,
            columns: source_detail.map(|detail| {
                detail
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(index, c)| ColumnDiff {
                        diff_type: "added".to_string(),
                        name: c.name.clone(),
                        source: Some(c.clone()),
                        target: None,
                        changes: vec![],
                        add_position: Some(column_add_position(&detail.columns, index)),
                    })
                    .collect()
            }),
            indexes: source_detail.map(|detail| {
                detail
                    .indexes
                    .iter()
                    .map(|i| IndexDiff {
                        diff_type: "added".to_string(),
                        name: i.name.clone(),
                        source: Some(i.clone()),
                        target: None,
                        changes: vec![],
                    })
                    .collect()
            }),
            foreign_keys: source_detail.map(|detail| {
                detail
                    .foreign_keys
                    .iter()
                    .map(|fk| {
                        let fk = normalize_mapped_foreign_key(
                            fk,
                            &source_table_name_set,
                            &target_table_name_set,
                            &table_pairs,
                            &options.table_mappings,
                            options.ignore_table_name_case,
                        );
                        ForeignKeyDiff {
                            diff_type: "added".to_string(),
                            name: fk.name.clone(),
                            source: Some(fk),
                            target: None,
                            changes: vec![],
                        }
                    })
                    .collect()
            }),
            triggers: source_detail.and_then(|detail| {
                if detail.triggers.is_empty() {
                    None
                } else {
                    Some(
                        detail
                            .triggers
                            .iter()
                            .map(|t| TriggerDiff {
                                diff_type: "added".to_string(),
                                name: t.name.clone(),
                                source: Some(t.clone()),
                                target: None,
                                changes: vec![],
                            })
                            .collect(),
                    )
                }
            }),
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        });
    }

    for name in table_resolution.target_only {
        let name_clone = name.clone();
        let target_detail = target_details.get(name_clone.as_str()).copied();
        result.push(TableDiff {
            diff_type: "removed".to_string(),
            object_type: Some("table".to_string()),
            target_name: None,
            name,
            columns: target_detail.map(|detail| {
                detail
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| ColumnDiff {
                        diff_type: "removed".to_string(),
                        name: column.name.clone(),
                        source: None,
                        target: Some(column.clone()),
                        changes: vec![],
                        add_position: Some(column_add_position(&detail.columns, index)),
                    })
                    .collect()
            }),
            indexes: target_detail.map(|detail| {
                detail
                    .indexes
                    .iter()
                    .map(|index| IndexDiff {
                        diff_type: "removed".to_string(),
                        name: index.name.clone(),
                        source: None,
                        target: Some(index.clone()),
                        changes: vec![],
                    })
                    .collect()
            }),
            foreign_keys: target_detail.map(|detail| {
                detail
                    .foreign_keys
                    .iter()
                    .map(|foreign_key| ForeignKeyDiff {
                        diff_type: "removed".to_string(),
                        name: foreign_key.name.clone(),
                        source: None,
                        target: Some(foreign_key.clone()),
                        changes: vec![],
                    })
                    .collect()
            }),
            triggers: target_detail.map(|detail| {
                detail
                    .triggers
                    .iter()
                    .map(|trigger| TriggerDiff {
                        diff_type: "removed".to_string(),
                        name: trigger.name.clone(),
                        source: None,
                        target: Some(trigger.clone()),
                        changes: vec![],
                    })
                    .collect()
            }),
            ddl: None,
            target_ddl: target_detail.and_then(|detail| detail.ddl.clone()),
            source_table_comment: None,
            target_table_comment: target_table_comments.get(name_clone.as_str()).cloned(),
            sync_sql: None,
        });
    }

    for name in view_resolution.source_only {
        let name_clone = name.clone();
        result.push(TableDiff {
            diff_type: "added".to_string(),
            object_type: Some("view".to_string()),
            target_name: None,
            name,
            columns: None,
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: source_details.get(name_clone.as_str()).and_then(|detail| detail.ddl.clone()),
            target_ddl: None,
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        });
    }

    for name in view_resolution.target_only {
        let name_clone = name.clone();
        result.push(TableDiff {
            diff_type: "removed".to_string(),
            object_type: Some("view".to_string()),
            target_name: None,
            name,
            columns: None,
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: target_details.get(name_clone.as_str()).and_then(|detail| detail.ddl.clone()),
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        });
    }

    for (name, target_name) in view_resolution.pairs {
        let Some(source_ddl) = source_details.get(name.as_str()).and_then(|detail| detail.ddl.as_ref()) else {
            continue;
        };
        let Some(target_ddl) = target_details.get(target_name.as_str()).and_then(|detail| detail.ddl.as_ref()) else {
            continue;
        };
        if !view_definitions_differ(source_ddl, target_ddl, options.source_dialect, options.target_dialect) {
            continue;
        }

        result.push(TableDiff {
            diff_type: "modified".to_string(),
            object_type: Some("view".to_string()),
            target_name: (name != target_name).then_some(target_name),
            name,
            columns: None,
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: Some(source_ddl.clone()),
            target_ddl: Some(target_ddl.clone()),
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        });
    }

    for (name, target_name) in table_resolution.pairs {
        let Some(source) = source_details.get(name.as_str()) else { continue };
        let Some(target) = target_details.get(target_name.as_str()) else { continue };
        let column_diffs = diff_columns_with_identifier_options(
            &source.columns,
            &target.columns,
            options.ignore_comments,
            options.compare_column_order,
            options.detect_renames,
            options.rename_threshold,
            options.source_dialect,
            options.target_dialect,
            options.ignore_column_name_case,
            compare_mysql_column_charset(options),
        );
        let index_diffs = diff_indexes_with_options(&source.indexes, &target.indexes, options.ignore_column_name_case);
        let normalized_source_fks: Vec<ForeignKeyInfo> = source
            .foreign_keys
            .iter()
            .map(|fk| {
                normalize_mapped_foreign_key(
                    fk,
                    &source_table_name_set,
                    &target_table_name_set,
                    &table_pairs,
                    &options.table_mappings,
                    options.ignore_table_name_case,
                )
            })
            .collect();
        let normalized_target_fks: Vec<ForeignKeyInfo> = target
            .foreign_keys
            .iter()
            .map(|fk| normalize_self_referencing_fk(fk, &target_table_name_set, options.ignore_table_name_case))
            .collect();
        let foreign_key_diffs = diff_foreign_keys_with_options(
            &normalized_source_fks,
            &normalized_target_fks,
            options.ignore_table_name_case,
            options.ignore_column_name_case,
        );
        let trigger_diffs = diff_triggers(&source.triggers, &target.triggers);
        let source_comment = source_table_comments.get(name.as_str()).cloned().unwrap_or(None);
        let target_comment = target_table_comments.get(target_name.as_str()).cloned().unwrap_or(None);
        let comment_changed = !options.ignore_comments
            && source_comment.clone().unwrap_or_default() != target_comment.clone().unwrap_or_default();

        let has_diff = !column_diffs.is_empty()
            || !index_diffs.is_empty()
            || !foreign_key_diffs.is_empty()
            || !trigger_diffs.is_empty()
            || comment_changed;

        result.push(TableDiff {
            diff_type: if has_diff { "modified".to_string() } else { "none".to_string() },
            object_type: Some("table".to_string()),
            target_name: (name != target_name).then_some(target_name.clone()),
            name,
            columns: if has_diff { (!column_diffs.is_empty()).then_some(column_diffs) } else { None },
            indexes: if has_diff { (!index_diffs.is_empty()).then_some(index_diffs) } else { None },
            foreign_keys: if has_diff { (!foreign_key_diffs.is_empty()).then_some(foreign_key_diffs) } else { None },
            triggers: if has_diff { (!trigger_diffs.is_empty()).then_some(trigger_diffs) } else { None },
            ddl: source.ddl.clone(),
            target_ddl: target.ddl.clone(),
            source_table_comment: if has_diff { comment_changed.then_some(source_comment) } else { None },
            target_table_comment: if has_diff { comment_changed.then_some(target_comment) } else { None },
            sync_sql: None,
        });
    }

    result.retain(|diff| diff.diff_type != "none");
    result
}

fn diff_names(source: &[String], target: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let source_set: HashSet<&str> = source.iter().map(String::as_str).collect();
    let target_set: HashSet<&str> = target.iter().map(String::as_str).collect();
    (
        source.iter().filter(|name| !target_set.contains(name.as_str())).cloned().collect(),
        target.iter().filter(|name| !source_set.contains(name.as_str())).cloned().collect(),
        source.iter().filter(|name| target_set.contains(name.as_str())).cloned().collect(),
    )
}

/// Whether two captured view definitions differ.
///
/// Only dialects whose captured text `dbx` can normalize are compared: a view body is
/// dialect-specific SQL, so a cross-dialect pair (or an engine whose text nobody taught
/// this function to read) is left alone rather than reported as a difference.
fn view_definitions_differ(
    source_ddl: &str,
    target_ddl: &str,
    source_dialect: Option<DialectKind>,
    target_dialect: Option<DialectKind>,
) -> bool {
    if source_dialect != target_dialect {
        return false;
    }

    match source_dialect {
        Some(DialectKind::Mysql) => normalize_mysql_view_ddl(source_ddl) != normalize_mysql_view_ddl(target_ddl),

        _ => false,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MysqlViewTokenKind {
    Atom,
    Symbol,
}

fn normalize_mysql_view_ddl(ddl: &str) -> String {
    let ddl = strip_mysql_view_definer(ddl);
    let schema = mysql_view_schema(&ddl);
    let mut normalized = String::with_capacity(ddl.len());
    let mut previous = None;
    let mut pending_whitespace = false;
    let bytes = ddl.as_bytes();
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index].is_ascii_whitespace() {
            pending_whitespace = true;
            index += 1;
            continue;
        }

        let (end, kind, replacement) = match bytes[index] {
            b'\'' | b'"' => {
                let end = mysql_quoted_token_end(&ddl, index);
                (end, MysqlViewTokenKind::Atom, None)
            }
            b'`' => {
                let end = mysql_quoted_token_end(&ddl, index);
                let identifier = decode_mysql_quoted_identifier(&ddl[index..end]);
                let is_schema_qualifier =
                    schema.as_deref() == Some(identifier.as_str()) && ddl[end..].trim_start().starts_with('.');
                (end, MysqlViewTokenKind::Atom, is_schema_qualifier.then_some("`__dbx_schema__`"))
            }
            b'#' => {
                let end = ddl[index..].find('\n').map_or(bytes.len(), |offset| index + offset);
                (end, MysqlViewTokenKind::Atom, None)
            }
            b'-' if bytes.get(index + 1) == Some(&b'-')
                && bytes.get(index + 2).is_some_and(|next| next.is_ascii_whitespace()) =>
            {
                let end = ddl[index..].find('\n').map_or(bytes.len(), |offset| index + offset);
                (end, MysqlViewTokenKind::Atom, None)
            }
            b'/' if bytes.get(index + 1) == Some(&b'*') => {
                let end = ddl[index + 2..].find("*/").map_or(bytes.len(), |offset| index + 2 + offset + 2);
                (end, MysqlViewTokenKind::Atom, None)
            }
            byte if view_ddl_symbol(byte) => (index + 1, MysqlViewTokenKind::Symbol, None),
            _ => {
                let mut end = index + 1;
                while end < bytes.len()
                    && !bytes[end].is_ascii_whitespace()
                    && !matches!(bytes[end], b'\'' | b'"' | b'`' | b'#')
                    && !view_ddl_symbol(bytes[end])
                {
                    end += 1;
                }
                (end, MysqlViewTokenKind::Atom, None)
            }
        };

        if pending_whitespace && previous == Some(kind) {
            normalized.push(' ');
        }
        normalized.push_str(replacement.unwrap_or(&ddl[index..end]));
        previous = Some(kind);
        pending_whitespace = false;
        index = end;
    }

    normalized
}

/// Punctuation shared by the view-text tokenizers: MySQL and Oracle agree on every
/// operator and delimiter the server can put between two atoms, so one predicate keeps
/// the two normalizers from drifting apart.
fn view_ddl_symbol(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')'
            | b'['
            | b']'
            | b'{'
            | b'}'
            | b','
            | b'.'
            | b';'
            | b'+'
            | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'<'
            | b'>'
            | b'='
            | b'!'
            | b'|'
            | b'&'
            | b'^'
            | b'~'
            | b'?'
            | b':'
            | b'@'
    )
}

fn mysql_view_schema(ddl: &str) -> Option<String> {
    let view = find_mysql_header_keyword(ddl, "VIEW", ddl.len())?;
    let mut index = skip_ascii_whitespace(ddl, view + "VIEW".len());
    let (identifier, end) = parse_mysql_identifier(ddl, index)?;
    index = skip_ascii_whitespace(ddl, end);
    (ddl.as_bytes().get(index) == Some(&b'.')).then_some(identifier)
}

fn strip_mysql_view_definer(ddl: &str) -> String {
    let Some(view) = find_mysql_header_keyword(ddl, "VIEW", ddl.len()) else {
        return ddl.to_string();
    };
    let Some(definer) = find_mysql_header_keyword(ddl, "DEFINER", view) else {
        return ddl.to_string();
    };
    let mut index = skip_ascii_whitespace(ddl, definer + "DEFINER".len());
    if ddl.as_bytes().get(index) != Some(&b'=') {
        return ddl.to_string();
    }
    index = skip_ascii_whitespace(ddl, index + 1);

    let Some(mut end) = parse_mysql_definer_principal(ddl, index) else {
        return ddl.to_string();
    };
    end = skip_ascii_whitespace(ddl, end);
    if ddl.as_bytes().get(end) == Some(&b'@') {
        end = skip_ascii_whitespace(ddl, end + 1);
        let Some(host_end) = parse_mysql_definer_principal(ddl, end) else {
            return ddl.to_string();
        };
        end = host_end;
    } else if ddl[index..end].eq_ignore_ascii_case("CURRENT_USER") {
        let open = skip_ascii_whitespace(ddl, end);
        if ddl.as_bytes().get(open) == Some(&b'(') {
            let close = skip_ascii_whitespace(ddl, open + 1);
            if ddl.as_bytes().get(close) == Some(&b')') {
                end = close + 1;
            }
        }
    } else {
        return ddl.to_string();
    }

    end = skip_ascii_whitespace(ddl, end);
    let mut stripped = String::with_capacity(ddl.len() - (end - definer));
    stripped.push_str(&ddl[..definer]);
    stripped.push_str(&ddl[end..]);
    stripped
}

fn parse_mysql_definer_principal(ddl: &str, index: usize) -> Option<usize> {
    match *ddl.as_bytes().get(index)? {
        b'`' | b'\'' | b'"' => Some(mysql_quoted_token_end(ddl, index)),
        _ => {
            let mut end = index;
            while let Some(byte) = ddl.as_bytes().get(end) {
                if byte.is_ascii_whitespace() || matches!(byte, b'@' | b'(' | b')') {
                    break;
                }
                end += 1;
            }
            (end > index).then_some(end)
        }
    }
}

fn parse_mysql_identifier(ddl: &str, index: usize) -> Option<(String, usize)> {
    if ddl.as_bytes().get(index) == Some(&b'`') {
        let end = mysql_quoted_token_end(ddl, index);
        return Some((decode_mysql_quoted_identifier(&ddl[index..end]), end));
    }

    let mut end = index;
    while let Some(byte) = ddl.as_bytes().get(end) {
        if byte.is_ascii_whitespace() || view_ddl_symbol(*byte) {
            break;
        }
        end += 1;
    }
    (end > index).then(|| (ddl[index..end].to_string(), end))
}

fn decode_mysql_quoted_identifier(identifier: &str) -> String {
    identifier.strip_prefix('`').and_then(|value| value.strip_suffix('`')).unwrap_or(identifier).replace("``", "`")
}

fn find_mysql_header_keyword(ddl: &str, keyword: &str, limit: usize) -> Option<usize> {
    let bytes = ddl.as_bytes();
    let mut index = 0;
    while index < limit {
        match bytes[index] {
            b'\'' | b'"' | b'`' => {
                index = mysql_quoted_token_end(ddl, index);
            }
            byte if byte.is_ascii_alphabetic() || byte == b'_' => {
                let start = index;
                index += 1;
                while index < limit && (bytes[index].is_ascii_alphanumeric() || matches!(bytes[index], b'_' | b'$')) {
                    index += 1;
                }
                if ddl[start..index].eq_ignore_ascii_case(keyword) {
                    return Some(start);
                }
            }
            _ => index += 1,
        }
    }
    None
}

fn mysql_quoted_token_end(ddl: &str, start: usize) -> usize {
    let bytes = ddl.as_bytes();
    let quote = bytes[start];
    let mut index = start + 1;
    while index < bytes.len() {
        if bytes[index] == b'\\' && quote != b'`' {
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == quote {
            if bytes.get(index + 1) == Some(&quote) {
                index += 2;
                continue;
            }
            return index + 1;
        }
        index += 1;
    }
    bytes.len()
}

fn skip_ascii_whitespace(input: &str, mut index: usize) -> usize {
    while input.as_bytes().get(index).is_some_and(|byte| byte.is_ascii_whitespace()) {
        index += 1;
    }
    index
}

pub fn diff_columns(source: &[ColumnInfo], target: &[ColumnInfo]) -> Vec<ColumnDiff> {
    diff_columns_with_options(source, target, false, false, false, 0.5)
}

/// Signature of a MySQL column type used to decide whether an integer display
/// width difference is real or just MySQL echoing back its own default width.
struct MysqlIntegerTypeSignature {
    /// Whole normalized type string, used to compare non-integer types as-is.
    normalized: String,
    /// `"{base} {suffix}"` (e.g. `"int unsigned"`) when `normalized` is a
    /// recognized integer type, regardless of whether a width is present.
    integer_key: Option<String>,
    /// The explicit display width, when present on a recognized integer type.
    width: Option<u32>,
}

fn parse_mysql_integer_type(data_type: &str) -> MysqlIntegerTypeSignature {
    let normalized = data_type.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    let is_integer_base =
        |base: &str| matches!(base, "tinyint" | "smallint" | "mediumint" | "int" | "integer" | "bigint" | "year");
    let Some(open) = normalized.find('(') else {
        let base = normalized.split(' ').next().unwrap_or(&normalized);
        let integer_key = is_integer_base(base).then(|| normalized.clone());
        return MysqlIntegerTypeSignature { normalized, integer_key, width: None };
    };
    let Some(close) = normalized[open + 1..].find(')').map(|index| open + 1 + index) else {
        return MysqlIntegerTypeSignature { normalized, integer_key: None, width: None };
    };
    let base = normalized[..open].trim();
    let width_str = normalized[open + 1..close].trim();
    if !is_integer_base(base) || width_str.is_empty() || !width_str.bytes().all(|byte| byte.is_ascii_digit()) {
        return MysqlIntegerTypeSignature { normalized, integer_key: None, width: None };
    }
    let suffix = normalized[close + 1..].trim();
    let integer_key = Some(if suffix.is_empty() { base.to_string() } else { format!("{base} {suffix}") });
    let width = width_str.parse().ok();
    MysqlIntegerTypeSignature { normalized, integer_key, width }
}

fn column_types_equal_for_dialects(
    source_type: &str,
    target_type: &str,
    source_dialect: Option<DialectKind>,
    target_dialect: Option<DialectKind>,
) -> bool {
    if source_type.eq_ignore_ascii_case(target_type) {
        return true;
    }
    if source_dialect != Some(DialectKind::Mysql) || target_dialect != Some(DialectKind::Mysql) {
        return false;
    }
    let source = parse_mysql_integer_type(source_type);
    let target = parse_mysql_integer_type(target_type);
    match (source.integer_key, target.integer_key) {
        // Same integer family (e.g. both "int unsigned"): a display width present on only one
        // side is MySQL filling in its own default and not a real difference, but two explicit,
        // differing widths (e.g. int(11) vs int(15)) are a genuine schema difference.
        (Some(source_key), Some(target_key)) => {
            source_key == target_key
                && match (source.width, target.width) {
                    (Some(a), Some(b)) => a == b,
                    _ => true,
                }
        }
        (None, None) => source.normalized == target.normalized,
        _ => false,
    }
}

fn mysql_charset_value(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn mysql_charset_values_differ(source: Option<&str>, target: Option<&str>) -> bool {
    match (mysql_charset_value(source), mysql_charset_value(target)) {
        (Some(source), Some(target)) => !source.eq_ignore_ascii_case(target),
        _ => false,
    }
}

fn column_type_similarity_score(source_type: &str, target_type: &str) -> f64 {
    let s = ColumnType::parse(source_type).base_type.to_ascii_lowercase();
    let t = ColumnType::parse(target_type).base_type.to_ascii_lowercase();
    if s == t {
        return 1.0;
    }
    let exact_matches = [
        ("int", "integer"),
        ("integer", "int"),
        ("float", "real"),
        ("real", "float"),
        ("double", "double precision"),
        ("double precision", "double"),
        ("bool", "boolean"),
        ("boolean", "bool"),
        ("timestamp", "datetime"),
        ("datetime", "timestamp"),
    ];
    if exact_matches.contains(&(s.as_str(), t.as_str())) {
        return 1.0;
    }
    let integer_family = ["tinyint", "smallint", "mediumint", "int", "integer", "bigint", "serial", "bigserial"];
    let text_family = ["char", "varchar", "text", "tinytext", "mediumtext", "longtext", "clob", "nclob"];
    if integer_family.contains(&s.as_str()) && integer_family.contains(&t.as_str()) {
        return 0.8;
    }
    if text_family.contains(&s.as_str()) && text_family.contains(&t.as_str()) {
        return 0.8;
    }
    0.0
}

fn diff_columns_with_options(
    source: &[ColumnInfo],
    target: &[ColumnInfo],
    ignore_comments: bool,
    compare_column_order: bool,
    detect_renames: bool,
    rename_threshold: f64,
) -> Vec<ColumnDiff> {
    diff_columns_with_dialect_options(
        source,
        target,
        ignore_comments,
        compare_column_order,
        detect_renames,
        rename_threshold,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn diff_columns_with_dialect_options(
    source: &[ColumnInfo],
    target: &[ColumnInfo],
    ignore_comments: bool,
    compare_column_order: bool,
    detect_renames: bool,
    rename_threshold: f64,
    source_dialect: Option<DialectKind>,
    target_dialect: Option<DialectKind>,
) -> Vec<ColumnDiff> {
    diff_columns_with_identifier_options(
        source,
        target,
        ignore_comments,
        compare_column_order,
        detect_renames,
        rename_threshold,
        source_dialect,
        target_dialect,
        false,
        source_dialect == Some(DialectKind::Mysql) && target_dialect == Some(DialectKind::Mysql),
    )
}

fn resolve_column_matches(
    source: &[ColumnInfo],
    target: &[ColumnInfo],
    ignore_column_name_case: bool,
) -> Vec<Option<usize>> {
    let mut matches = vec![None; source.len()];
    let mut used_targets = HashSet::new();

    // Exact matches always win, even when a case-insensitive candidate is also available.
    for (source_index, source_column) in source.iter().enumerate() {
        if let Some(target_index) = target.iter().enumerate().find_map(|(target_index, target_column)| {
            (target_column.name == source_column.name && !used_targets.contains(&target_index)).then_some(target_index)
        }) {
            matches[source_index] = Some(target_index);
            used_targets.insert(target_index);
        }
    }

    if ignore_column_name_case {
        for (source_index, source_column) in source.iter().enumerate() {
            if matches[source_index].is_some() {
                continue;
            }
            let mut candidates = target.iter().enumerate().filter(|(target_index, target_column)| {
                !used_targets.contains(target_index) && target_column.name.eq_ignore_ascii_case(&source_column.name)
            });
            let candidate = candidates.next().map(|(target_index, _)| target_index);
            if let Some(target_index) = candidate.filter(|_| candidates.next().is_none()) {
                matches[source_index] = Some(target_index);
                used_targets.insert(target_index);
            }
        }
    }

    matches
}

#[allow(clippy::too_many_arguments)]
fn diff_columns_with_identifier_options(
    source: &[ColumnInfo],
    target: &[ColumnInfo],
    ignore_comments: bool,
    compare_column_order: bool,
    detect_renames: bool,
    rename_threshold: f64,
    source_dialect: Option<DialectKind>,
    target_dialect: Option<DialectKind>,
    ignore_column_name_case: bool,
    compare_charset: bool,
) -> Vec<ColumnDiff> {
    let mut diffs = Vec::new();
    let column_matches = resolve_column_matches(source, target, ignore_column_name_case);
    let can_compare_order =
        compare_column_order && source.len() == target.len() && column_matches.iter().all(Option::is_some);

    for (source_index, source_column) in source.iter().enumerate() {
        if let Some(target_index) = column_matches[source_index] {
            let target_column = &target[target_index];
            let mut changes = Vec::new();
            // Compare the *declared* type, not the raw driver string: Oracle reports
            // `NUMBER` with the precision/scale in separate fields, so `NUMBER(10,2)` and
            // `NUMBER(12,2)` would otherwise look identical (#9261).
            let source_type = declared_column_type(source_column);
            let target_type = declared_column_type(target_column);
            if !column_types_equal_for_dialects(&source_type, &target_type, source_dialect, target_dialect) {
                changes.push(format!("type: {target_type} → {source_type}"));
            }
            if source_column.is_nullable != target_column.is_nullable {
                changes.push(format!(
                    "nullable: {} → {}",
                    if target_column.is_nullable { "YES" } else { "NO" },
                    if source_column.is_nullable { "YES" } else { "NO" }
                ));
            }
            if source_column.column_default.as_deref().unwrap_or_default()
                != target_column.column_default.as_deref().unwrap_or_default()
            {
                changes.push(format!(
                    "default: {} → {}",
                    target_column.column_default.as_deref().unwrap_or("NULL"),
                    source_column.column_default.as_deref().unwrap_or("NULL")
                ));
            }
            if compare_charset
                && mysql_charset_values_differ(
                    source_column.character_set.as_deref(),
                    target_column.character_set.as_deref(),
                )
            {
                changes.push(format!(
                    "character set: {} → {}",
                    mysql_charset_value(target_column.character_set.as_deref()).unwrap_or_default(),
                    mysql_charset_value(source_column.character_set.as_deref()).unwrap_or_default()
                ));
            }
            if compare_charset
                && mysql_charset_values_differ(source_column.collation.as_deref(), target_column.collation.as_deref())
            {
                changes.push(format!(
                    "collation: {} → {}",
                    mysql_charset_value(target_column.collation.as_deref()).unwrap_or_default(),
                    mysql_charset_value(source_column.collation.as_deref()).unwrap_or_default()
                ));
            }
            if !ignore_comments
                && source_column.comment.as_deref().unwrap_or_default()
                    != target_column.comment.as_deref().unwrap_or_default()
            {
                changes.push(format!(
                    "comment: {} → {}",
                    target_column.comment.as_deref().unwrap_or_default(),
                    source_column.comment.as_deref().unwrap_or_default()
                ));
            }
            if can_compare_order && source_index != target_index {
                changes.push(format!("order: {} → {}", target_index + 1, source_index + 1));
            }
            if !changes.is_empty() {
                diffs.push(ColumnDiff {
                    diff_type: "modified".to_string(),
                    name: source_column.name.clone(),
                    source: Some(source_column.clone()),
                    target: Some((*target_column).clone()),
                    changes,
                    add_position: None,
                });
            }
        } else {
            diffs.push(ColumnDiff {
                diff_type: "added".to_string(),
                name: source_column.name.clone(),
                source: Some(source_column.clone()),
                target: None,
                changes: Vec::new(),
                add_position: Some(column_add_position(source, source_index)),
            });
        }
    }

    for (target_index, target_column) in target.iter().enumerate() {
        if !column_matches.iter().flatten().any(|matched_index| *matched_index == target_index) {
            diffs.push(ColumnDiff {
                diff_type: "removed".to_string(),
                name: target_column.name.clone(),
                source: None,
                target: Some(target_column.clone()),
                changes: Vec::new(),
                add_position: Some(column_add_position(target, target_index)),
            });
        }
    }

    if detect_renames && rename_threshold > 0.0 {
        let removed_indices: Vec<usize> =
            diffs.iter().enumerate().filter(|(_, d)| d.diff_type == "removed").map(|(i, _)| i).collect();
        let added_indices: Vec<usize> =
            diffs.iter().enumerate().filter(|(_, d)| d.diff_type == "added").map(|(i, _)| i).collect();

        let mut matched_added: HashSet<usize> = HashSet::new();
        let mut matched_removed: HashSet<usize> = HashSet::new();
        let mut rename_pairs: Vec<(usize, usize, f64)> = Vec::new();

        for &ri in &removed_indices {
            if let Some(removed_col) = &diffs[ri].target {
                let mut best_score = 0.0_f64;
                let mut best_ai = None;
                for &ai in &added_indices {
                    if matched_added.contains(&ai) {
                        continue;
                    }
                    if let Some(added_col) = &diffs[ai].source {
                        let type_score = column_type_similarity_score(&removed_col.data_type, &added_col.data_type);
                        if type_score < rename_threshold {
                            continue;
                        }
                        let mut score = type_score;
                        if removed_col.is_nullable == added_col.is_nullable {
                            score *= 1.0;
                        } else {
                            score *= 0.8;
                        }
                        if score > best_score {
                            best_score = score;
                            best_ai = Some(ai);
                        }
                    }
                }
                if let Some(ai) = best_ai {
                    rename_pairs.push((ri, ai, best_score));
                    matched_removed.insert(ri);
                    matched_added.insert(ai);
                }
            }
        }

        for (ri, ai, _score) in &rename_pairs {
            let old_name = diffs[*ri].name.clone();
            let old_col = diffs[*ri].target.clone().unwrap();
            let new_col = diffs[*ai].source.clone().unwrap();
            let new_name = new_col.name.clone();

            diffs[*ri] = ColumnDiff {
                diff_type: "renamed".to_string(),
                name: new_name.clone(),
                source: Some(new_col),
                target: Some(old_col),
                changes: vec![format!("{} → {}", old_name, new_name)],
                add_position: None,
            };
            diffs[*ai] = ColumnDiff {
                diff_type: "_matched_rename".to_string(),
                name: String::new(),
                source: None,
                target: None,
                changes: Vec::new(),
                add_position: None,
            };
        }

        diffs.retain(|d| d.diff_type != "_matched_rename");
    }

    diffs
}

fn column_add_position(columns: &[ColumnInfo], index: usize) -> ColumnAddPosition {
    if index == 0 {
        ColumnAddPosition::First
    } else {
        ColumnAddPosition::After(columns[index - 1].name.clone())
    }
}

pub fn diff_indexes(source: &[IndexInfo], target: &[IndexInfo]) -> Vec<IndexDiff> {
    diff_indexes_with_options(source, target, false)
}

fn index_columns_equal(source: &IndexInfo, target: &IndexInfo, ignore_column_name_case: bool) -> bool {
    source.columns.len() == target.columns.len()
        && source.columns.iter().enumerate().all(|(index, source_column)| {
            let target_column = &target.columns[index];
            let source_is_expression = source.key_is_expression.get(index).copied().unwrap_or(false);
            let target_is_expression = target.key_is_expression.get(index).copied().unwrap_or(false);
            if source_is_expression || target_is_expression {
                source_column == target_column
            } else {
                identifiers_equal(source_column, target_column, ignore_column_name_case)
            }
        })
}

fn identifier_lists_equal(left: &[String], right: &[String], ignore_column_name_case: bool) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| identifiers_equal(left, right, ignore_column_name_case))
}

/// Names a server hands out on its own. Oracle names the index behind an *unnamed*
/// PRIMARY KEY / UNIQUE constraint `SYS_C<number>`, and that number is per-object, so two
/// structurally identical tables never agree on it — while Dameng and SQLite do the same
/// with `SYS_C…`/`sqlite_autoindex_…` shapes. Those names carry no user intent and must not
/// be compared as if they did.
fn index_name_is_server_generated(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    if let Some(digits) = upper.strip_prefix("SYS_C") {
        return !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
    }
    upper.starts_with("SQLITE_AUTOINDEX_")
}

/// Everything that identifies an index apart from its name. Used to pair up two indexes
/// whose names differ only because the server generated them.
fn index_signatures_equal(source: &IndexInfo, target: &IndexInfo, ignore_column_name_case: bool) -> bool {
    source.is_unique == target.is_unique
        && source.index_type == target.index_type
        && source.filter == target.filter
        && source.column_opclasses == target.column_opclasses
        && source.key_options == target.key_options
        && index_columns_equal(source, target, ignore_column_name_case)
        && identifier_lists_equal(
            &source.included_columns.clone().unwrap_or_default(),
            &target.included_columns.clone().unwrap_or_default(),
            ignore_column_name_case,
        )
}

/// Pairs source indexes with the target index that is *the same index under a
/// server-generated name*, so comparing two structurally identical tables does not report
/// a phantom "index added + index removed" pair (#9261). Returns the target position for
/// every source position that was paired this way.
fn match_server_generated_indexes(
    source: &[IndexInfo],
    target: &[IndexInfo],
    ignore_column_name_case: bool,
) -> Vec<Option<usize>> {
    let source_names: HashSet<&str> = source.iter().map(|index| index.name.as_str()).collect();
    let target_names: HashSet<&str> = target.iter().map(|index| index.name.as_str()).collect();
    let mut matched_targets: HashSet<usize> = HashSet::new();
    let mut pairs: Vec<Option<usize>> = vec![None; source.len()];

    for (source_position, source_index) in source.iter().enumerate() {
        if source_index.is_primary || target_names.contains(source_index.name.as_str()) {
            continue;
        }
        let Some(target_position) = target.iter().enumerate().find_map(|(position, target_index)| {
            if matched_targets.contains(&position)
                || target_index.is_primary
                || source_names.contains(target_index.name.as_str())
            {
                return None;
            }
            // Only a server-generated name may stand in for a user-chosen one; two
            // deliberately different names stay a real difference.
            if !index_name_is_server_generated(&source_index.name)
                && !index_name_is_server_generated(&target_index.name)
            {
                return None;
            }
            index_signatures_equal(source_index, target_index, ignore_column_name_case).then_some(position)
        }) else {
            continue;
        };
        matched_targets.insert(target_position);
        pairs[source_position] = Some(target_position);
    }

    pairs
}

fn diff_indexes_with_options(
    source: &[IndexInfo],
    target: &[IndexInfo],
    ignore_column_name_case: bool,
) -> Vec<IndexDiff> {
    let mut diffs = Vec::new();
    let target_map: HashMap<&str, &IndexInfo> = target.iter().map(|index| (index.name.as_str(), index)).collect();
    let source_map: HashMap<&str, &IndexInfo> = source.iter().map(|index| (index.name.as_str(), index)).collect();
    let server_generated_pairs = match_server_generated_indexes(source, target, ignore_column_name_case);
    let paired_targets: HashSet<usize> = server_generated_pairs.iter().flatten().copied().collect();

    for (source_position, source_index) in source.iter().enumerate() {
        if source_index.is_primary {
            continue;
        }
        let Some(target_index) = target_map.get(source_index.name.as_str()) else {
            if server_generated_pairs[source_position].is_some() {
                continue;
            }
            diffs.push(IndexDiff {
                diff_type: "added".to_string(),
                name: source_index.name.clone(),
                source: Some(source_index.clone()),
                target: None,
                changes: Vec::new(),
            });
            continue;
        };

        let mut changes = Vec::new();
        if source_index.is_unique != target_index.is_unique {
            changes.push(format!(
                "unique: {} → {}",
                if target_index.is_unique { "YES" } else { "NO" },
                if source_index.is_unique { "YES" } else { "NO" }
            ));
        }
        if !index_columns_equal(source_index, target_index, ignore_column_name_case) {
            changes.push(format!("columns: {} → {}", target_index.columns.join(", "), source_index.columns.join(", ")));
        }
        if source_index.index_type.as_deref().unwrap_or_default()
            != target_index.index_type.as_deref().unwrap_or_default()
        {
            changes.push(format!(
                "type: {} → {}",
                target_index.index_type.as_deref().unwrap_or("default"),
                source_index.index_type.as_deref().unwrap_or("default")
            ));
        }
        if source_index.filter.as_deref().unwrap_or_default() != target_index.filter.as_deref().unwrap_or_default() {
            changes.push(format!(
                "filter: {} → {}",
                target_index.filter.as_deref().unwrap_or("none"),
                source_index.filter.as_deref().unwrap_or("none")
            ));
        }
        let source_included = source_index.included_columns.clone().unwrap_or_default();
        let target_included = target_index.included_columns.clone().unwrap_or_default();
        if !identifier_lists_equal(&source_included, &target_included, ignore_column_name_case) {
            changes.push(format!(
                "include: {} → {}",
                if target_included.is_empty() { "none".to_string() } else { target_included.join(", ") },
                if source_included.is_empty() { "none".to_string() } else { source_included.join(", ") }
            ));
        }
        if source_index.column_opclasses != target_index.column_opclasses {
            let fmt_opclass = |o: &Option<String>| -> String {
                o.as_deref().filter(|v| !v.is_empty()).unwrap_or("default").to_string()
            };
            let src_str = source_index.column_opclasses.iter().map(fmt_opclass).collect::<Vec<_>>().join(", ");
            let tgt_str = target_index.column_opclasses.iter().map(fmt_opclass).collect::<Vec<_>>().join(", ");
            changes.push(format!("opclass: {} → {}", tgt_str, src_str));
        }
        if !changes.is_empty() {
            diffs.push(IndexDiff {
                diff_type: "modified".to_string(),
                name: source_index.name.clone(),
                source: Some(source_index.clone()),
                target: Some((*target_index).clone()),
                changes,
            });
        }
    }

    for (target_position, target_index) in target.iter().enumerate() {
        if target_index.is_primary || paired_targets.contains(&target_position) {
            continue;
        }
        if !source_map.contains_key(target_index.name.as_str()) {
            diffs.push(IndexDiff {
                diff_type: "removed".to_string(),
                name: target_index.name.clone(),
                source: None,
                target: Some(target_index.clone()),
                changes: Vec::new(),
            });
        }
    }

    diffs
}

fn normalized_foreign_key_action(action: Option<&str>) -> Option<String> {
    action
        .map(|value| value.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_uppercase())
        .filter(|value| !value.is_empty())
}

fn normalize_self_referencing_fk(
    fk: &ForeignKeyInfo,
    own_table_names: &HashSet<&str>,
    ignore_table_name_case: bool,
) -> ForeignKeyInfo {
    let mut normalized = fk.clone();
    if fk.ref_schema.is_some()
        && own_table_names.iter().any(|table_name| identifiers_equal(table_name, &fk.ref_table, ignore_table_name_case))
    {
        normalized.ref_schema = None;
    }
    normalized
}

fn normalize_mapped_foreign_key(
    fk: &ForeignKeyInfo,
    source_table_names: &HashSet<&str>,
    target_table_names: &HashSet<&str>,
    table_pairs: &[(String, String)],
    mappings: &[SchemaDiffTableMapping],
    ignore_table_name_case: bool,
) -> ForeignKeyInfo {
    let mut normalized = normalize_self_referencing_fk(fk, source_table_names, ignore_table_name_case);
    let mapped_target = mappings
        .iter()
        .find(|mapping| {
            mapping.source_table == normalized.ref_table && target_table_names.contains(mapping.target_table.as_str())
        })
        .map(|mapping| mapping.target_table.as_str())
        .or_else(|| {
            table_pairs
                .iter()
                .find(|(source_table, _)| {
                    identifiers_equal(source_table, &normalized.ref_table, ignore_table_name_case)
                })
                .map(|(_, target_table)| target_table.as_str())
        });
    if let Some(mapped_target) = mapped_target {
        normalized.ref_table = mapped_target.to_string();
        normalized.ref_schema = None;
    }
    normalized
}

pub fn diff_foreign_keys(source: &[ForeignKeyInfo], target: &[ForeignKeyInfo]) -> Vec<ForeignKeyDiff> {
    diff_foreign_keys_with_options(source, target, false, false)
}

fn diff_foreign_keys_with_options(
    source: &[ForeignKeyInfo],
    target: &[ForeignKeyInfo],
    ignore_table_name_case: bool,
    ignore_column_name_case: bool,
) -> Vec<ForeignKeyDiff> {
    let mut diffs = Vec::new();
    let target_map: HashMap<&str, &ForeignKeyInfo> = target.iter().map(|fk| (fk.name.as_str(), fk)).collect();
    let source_map: HashMap<&str, &ForeignKeyInfo> = source.iter().map(|fk| (fk.name.as_str(), fk)).collect();

    for source_fk in source {
        let Some(target_fk) = target_map.get(source_fk.name.as_str()) else {
            diffs.push(ForeignKeyDiff {
                diff_type: "added".to_string(),
                name: source_fk.name.clone(),
                source: Some(source_fk.clone()),
                target: None,
                changes: Vec::new(),
            });
            continue;
        };

        let mut changes = Vec::new();
        if !identifiers_equal(&source_fk.column, &target_fk.column, ignore_column_name_case) {
            changes.push(format!("column: {} → {}", target_fk.column, source_fk.column));
        }
        if !identifiers_equal(&source_fk.ref_table, &target_fk.ref_table, ignore_table_name_case) {
            changes.push(format!("ref table: {} → {}", target_fk.ref_table, source_fk.ref_table));
        }
        if source_fk.ref_schema != target_fk.ref_schema {
            changes.push(format!(
                "ref schema: {} → {}",
                target_fk.ref_schema.as_deref().unwrap_or(""),
                source_fk.ref_schema.as_deref().unwrap_or("")
            ));
        }
        if !identifiers_equal(&source_fk.ref_column, &target_fk.ref_column, ignore_column_name_case) {
            changes.push(format!("ref column: {} → {}", target_fk.ref_column, source_fk.ref_column));
        }
        let source_on_delete = normalized_foreign_key_action(source_fk.on_delete.as_deref());
        let target_on_delete = normalized_foreign_key_action(target_fk.on_delete.as_deref());
        if source_on_delete != target_on_delete {
            changes.push(format!(
                "delete: {} → {}",
                target_on_delete.as_deref().unwrap_or(""),
                source_on_delete.as_deref().unwrap_or("")
            ));
        }
        let source_on_update = normalized_foreign_key_action(source_fk.on_update.as_deref());
        let target_on_update = normalized_foreign_key_action(target_fk.on_update.as_deref());
        if source_on_update != target_on_update {
            changes.push(format!(
                "update: {} → {}",
                target_on_update.as_deref().unwrap_or(""),
                source_on_update.as_deref().unwrap_or("")
            ));
        }
        if !changes.is_empty() {
            diffs.push(ForeignKeyDiff {
                diff_type: "modified".to_string(),
                name: source_fk.name.clone(),
                source: Some(source_fk.clone()),
                target: Some((*target_fk).clone()),
                changes,
            });
        }
    }

    for target_fk in target {
        if !source_map.contains_key(target_fk.name.as_str()) {
            diffs.push(ForeignKeyDiff {
                diff_type: "removed".to_string(),
                name: target_fk.name.clone(),
                source: None,
                target: Some(target_fk.clone()),
                changes: Vec::new(),
            });
        }
    }

    diffs
}

pub fn diff_triggers(source: &[TriggerInfo], target: &[TriggerInfo]) -> Vec<TriggerDiff> {
    let mut diffs = Vec::new();
    let target_map: HashMap<&str, &TriggerInfo> =
        target.iter().map(|trigger| (trigger.name.as_str(), trigger)).collect();
    let source_map: HashMap<&str, &TriggerInfo> =
        source.iter().map(|trigger| (trigger.name.as_str(), trigger)).collect();

    for source_trigger in source {
        let Some(target_trigger) = target_map.get(source_trigger.name.as_str()) else {
            diffs.push(TriggerDiff {
                diff_type: "added".to_string(),
                name: source_trigger.name.clone(),
                source: Some(source_trigger.clone()),
                target: None,
                changes: Vec::new(),
            });
            continue;
        };

        let mut changes = Vec::new();
        if source_trigger.event != target_trigger.event {
            changes.push(format!("event: {} → {}", target_trigger.event, source_trigger.event));
        }
        if source_trigger.timing != target_trigger.timing {
            changes.push(format!("timing: {} → {}", target_trigger.timing, source_trigger.timing));
        }
        if !changes.is_empty() {
            diffs.push(TriggerDiff {
                diff_type: "modified".to_string(),
                name: source_trigger.name.clone(),
                source: Some(source_trigger.clone()),
                target: Some((*target_trigger).clone()),
                changes,
            });
        }
    }

    for target_trigger in target {
        if !source_map.contains_key(target_trigger.name.as_str()) {
            diffs.push(TriggerDiff {
                diff_type: "removed".to_string(),
                name: target_trigger.name.clone(),
                source: None,
                target: Some(target_trigger.clone()),
                changes: Vec::new(),
            });
        }
    }

    diffs
}

/// Normalize a function definition for comparison by:
/// - Converting CRLF to LF
/// - Collapsing all whitespace (tabs, multiple spaces) to single spaces
/// - Trimming each line and rejoining
pub fn normalize_definition(def: &str) -> String {
    def.replace("\r\n", "\n")
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn diff_functions(source: &[FunctionInfo], target: &[FunctionInfo]) -> Vec<FunctionDiff> {
    let mut diffs = Vec::new();
    // Use (name, arguments) as key to support PostgreSQL function overloading
    let target_map: HashMap<(&str, &str), &FunctionInfo> =
        target.iter().map(|f| ((f.name.as_str(), f.arguments.as_str()), f)).collect();
    let source_map: HashMap<(&str, &str), &FunctionInfo> =
        source.iter().map(|f| ((f.name.as_str(), f.arguments.as_str()), f)).collect();

    for source_fn in source {
        let key = (source_fn.name.as_str(), source_fn.arguments.as_str());
        let Some(target_fn) = target_map.get(&key) else {
            diffs.push(FunctionDiff {
                diff_type: "added".to_string(),
                name: source_fn.name.clone(),
                source: Some(source_fn.clone()),
                target: None,
                changes: Vec::new(),
            });
            continue;
        };

        let mut changes = Vec::new();
        if source_fn.function_type != target_fn.function_type {
            changes.push(format!("type: {} → {}", target_fn.function_type, source_fn.function_type));
        }
        if source_fn.data_type != target_fn.data_type {
            changes.push(format!("return type: {} → {}", target_fn.data_type, source_fn.data_type));
        }
        if normalize_definition(&source_fn.definition) != normalize_definition(&target_fn.definition) {
            changes.push("definition changed".to_string());
        }
        if !changes.is_empty() {
            diffs.push(FunctionDiff {
                diff_type: "modified".to_string(),
                name: source_fn.name.clone(),
                source: Some(source_fn.clone()),
                target: Some((*target_fn).clone()),
                changes,
            });
        }
    }

    for target_fn in target {
        let key = (target_fn.name.as_str(), target_fn.arguments.as_str());
        if !source_map.contains_key(&key) {
            diffs.push(FunctionDiff {
                diff_type: "removed".to_string(),
                name: target_fn.name.clone(),
                source: None,
                target: Some(target_fn.clone()),
                changes: Vec::new(),
            });
        }
    }

    diffs
}

pub fn diff_sequences(source: &[SequenceInfo], target: &[SequenceInfo]) -> Vec<SequenceDiff> {
    let mut diffs = Vec::new();
    let target_map: HashMap<&str, &SequenceInfo> = target.iter().map(|s| (s.name.as_str(), s)).collect();
    let source_map: HashMap<&str, &SequenceInfo> = source.iter().map(|s| (s.name.as_str(), s)).collect();

    for source_seq in source {
        let Some(target_seq) = target_map.get(source_seq.name.as_str()) else {
            diffs.push(SequenceDiff {
                diff_type: "added".to_string(),
                name: source_seq.name.clone(),
                source: Some(source_seq.clone()),
                target: None,
                changes: Vec::new(),
            });
            continue;
        };

        let mut changes = Vec::new();
        if source_seq.data_type != target_seq.data_type {
            changes.push(format!("data_type: {} → {}", target_seq.data_type, source_seq.data_type));
        }
        if source_seq.start_value != target_seq.start_value {
            changes.push(format!("start: {} → {}", target_seq.start_value, source_seq.start_value));
        }
        if source_seq.min_value != target_seq.min_value {
            changes.push(format!("min: {} → {}", target_seq.min_value, source_seq.min_value));
        }
        if source_seq.max_value != target_seq.max_value {
            changes.push(format!("max: {} → {}", target_seq.max_value, source_seq.max_value));
        }
        if source_seq.increment != target_seq.increment {
            changes.push(format!("increment: {} → {}", target_seq.increment, source_seq.increment));
        }
        if source_seq.cycle != target_seq.cycle {
            changes.push(format!("cycle: {} → {}", target_seq.cycle, source_seq.cycle));
        }
        // Only compare last_value when both sides successfully retrieved it.
        // Avoid false positives when one side lacks permission (returns None).
        if let (Some(s), Some(t)) = (&source_seq.last_value, &target_seq.last_value) {
            if s != t {
                changes.push(format!("last_value: {} → {}", t, s));
            }
        }
        if !changes.is_empty() {
            diffs.push(SequenceDiff {
                diff_type: "modified".to_string(),
                name: source_seq.name.clone(),
                source: Some(source_seq.clone()),
                target: Some((*target_seq).clone()),
                changes,
            });
        }
    }

    for target_seq in target {
        if !source_map.contains_key(target_seq.name.as_str()) {
            diffs.push(SequenceDiff {
                diff_type: "removed".to_string(),
                name: target_seq.name.clone(),
                source: None,
                target: Some(target_seq.clone()),
                changes: Vec::new(),
            });
        }
    }

    diffs
}

pub fn diff_rules(source: &[RuleInfo], target: &[RuleInfo]) -> Vec<RuleDiff> {
    let mut diffs = Vec::new();
    let target_map: HashMap<&str, &RuleInfo> = target.iter().map(|r| (r.name.as_str(), r)).collect();
    let source_map: HashMap<&str, &RuleInfo> = source.iter().map(|r| (r.name.as_str(), r)).collect();

    for source_rule in source {
        let Some(target_rule) = target_map.get(source_rule.name.as_str()) else {
            diffs.push(RuleDiff {
                diff_type: "added".to_string(),
                name: source_rule.name.clone(),
                source: Some(source_rule.clone()),
                target: None,
                changes: Vec::new(),
            });
            continue;
        };

        let mut changes = Vec::new();
        if source_rule.definition != target_rule.definition {
            changes.push("definition changed".to_string());
        }
        if !changes.is_empty() {
            diffs.push(RuleDiff {
                diff_type: "modified".to_string(),
                name: source_rule.name.clone(),
                source: Some(source_rule.clone()),
                target: Some((*target_rule).clone()),
                changes,
            });
        }
    }

    for target_rule in target {
        if !source_map.contains_key(target_rule.name.as_str()) {
            diffs.push(RuleDiff {
                diff_type: "removed".to_string(),
                name: target_rule.name.clone(),
                source: None,
                target: Some(target_rule.clone()),
                changes: Vec::new(),
            });
        }
    }

    diffs
}

pub fn diff_owners(source: &[OwnerInfo], target: &[OwnerInfo]) -> Vec<OwnerDiff> {
    let mut diffs = Vec::new();
    let target_map: HashMap<&str, &OwnerInfo> = target.iter().map(|o| (o.object_name.as_str(), o)).collect();
    let _source_map: HashMap<&str, &OwnerInfo> = source.iter().map(|o| (o.object_name.as_str(), o)).collect();

    for source_owner in source {
        let Some(target_owner) = target_map.get(source_owner.object_name.as_str()) else {
            continue; // skip added/removed objects, only compare owners for common objects
        };

        let mut changes = Vec::new();
        if source_owner.owner != target_owner.owner {
            changes.push(format!("owner: {} → {}", target_owner.owner, source_owner.owner));
        }
        if !changes.is_empty() {
            diffs.push(OwnerDiff {
                diff_type: "modified".to_string(),
                object_name: source_owner.object_name.clone(),
                source: Some(source_owner.clone()),
                target: Some((*target_owner).clone()),
                changes,
            });
        }
    }

    diffs
}

fn quote_id(name: &str, db_type: DatabaseType) -> String {
    profile_for(db_type).quote_ident(name)
}

/// The `NOT NULL` / `DEFAULT ...` tail of a column definition, in the order the target
/// dialect accepts. Oracle's grammar is `datatype [DEFAULT expr] [NOT NULL]` and rejects
/// the reverse order with `ORA-00907`, while MySQL/Postgres/SQLite write the constraint
/// first, so the order is dialect data (see `column_default_precedes_not_null`).
fn column_modifier_tail(
    profile: &DdlDialectProfile,
    col: &ColumnInfo,
    declared_type: &str,
    db_type: DatabaseType,
    source_dialect: Option<DialectKind>,
    skip_default: bool,
) -> String {
    let not_null = if col.is_nullable { String::new() } else { " NOT NULL".to_string() };
    let default = if skip_default {
        String::new()
    } else {
        col.column_default.as_ref().map_or_else(String::new, |value| {
            format!(
                " DEFAULT {}",
                default_literal(
                    value,
                    declared_type,
                    effective_source_dialect(source_dialect, db_type),
                    col.extra.as_deref()
                )
            )
        })
    };
    if profile.column_default_precedes_not_null {
        format!("{default}{not_null}")
    } else {
        format!("{not_null}{default}")
    }
}

fn column_def(col: &ColumnInfo, db_type: DatabaseType, source_dialect: Option<DialectKind>) -> String {
    column_def_with_charset(col, db_type, source_dialect, false)
}

fn column_def_with_charset(
    col: &ColumnInfo,
    db_type: DatabaseType,
    source_dialect: Option<DialectKind>,
    include_charset: bool,
) -> String {
    {}
    let profile = profile_for(db_type);
    let mut definition = format!("{} {}", quote_id(&col.name, db_type), col.data_type);
    if include_charset && matches!(db_type, DatabaseType::Mysql) {
        if let Some(character_set) = mysql_charset_value(col.character_set.as_deref()) {
            definition.push_str(&format!(" CHARACTER SET {}", quote_id(character_set, db_type)));
        }
        if let Some(collation) = mysql_charset_value(col.collation.as_deref()) {
            definition.push_str(&format!(" COLLATE {}", quote_id(collation, db_type)));
        }
    }
    definition.push_str(&column_modifier_tail(&profile, col, &col.data_type, db_type, source_dialect, false));
    // Suffix-style auto-increment is only valid in MySQL-family ALTER clauses
    // (ADD/MODIFY/CHANGE). Other dialects' identity clauses are order-sensitive
    // inside ADD COLUMN, so keep omitting them outside the MySQL family.
    if column_is_auto_increment(col) && profile.alter_uses_modify_column {
        if let AutoIncSyntax::Suffix(suffix) = profile.auto_inc {
            definition.push_str(suffix);
        }
    }
    if profile.inline_column_comment {
        if let Some(comment) = &col.comment {
            definition.push_str(&format!(" COMMENT {}", quote_string_literal(comment)));
        }
    }
    definition
}

fn qualified_name(name: &str, db_type: DatabaseType, schema: Option<&str>) -> String {
    let schema = schema.map(str::trim).filter(|schema| !schema.is_empty()).or_else(|| (false).then_some("dbo"));
    schema
        .map(|schema| format!("{}.{}", quote_id(schema, db_type), quote_id(name, db_type)))
        .unwrap_or_else(|| quote_id(name, db_type))
}

/// `CREATE TABLE`-family header keywords that native source DDL may start
/// with, longest-prefix-first so e.g. `CREATE FOREIGN TABLE` isn't shadowed
/// by a naive `CREATE TABLE` match.
const TABLE_DDL_HEADER_KEYWORDS: &[&str] =
    &["CREATE FOREIGN TABLE", "CREATE UNLOGGED TABLE", "CREATE TEMPORARY TABLE", "CREATE TEMP TABLE", "CREATE TABLE"];

/// `CREATE VIEW`-family header keywords, covering the `OR REPLACE` and
/// `MATERIALIZED` variants emitted by the Postgres/MySQL view DDL builders.
const VIEW_DDL_HEADER_KEYWORDS: &[&str] = &[
    "CREATE MATERIALIZED VIEW",
    "CREATE OR REPLACE VIEW",
    // Oracle's `DBMS_METADATA` writes the `FORCE`/`EDITIONABLE` variants; without them an
    // Oracle view that exists only in the source kept the source schema in its header and
    // the sync script created it on the wrong schema (or failed on rights).
    "CREATE OR REPLACE FORCE EDITIONABLE VIEW",
    "CREATE OR REPLACE FORCE NONEDITIONABLE VIEW",
    "CREATE OR REPLACE EDITIONABLE VIEW",
    "CREATE OR REPLACE NONEDITIONABLE VIEW",
    "CREATE OR REPLACE FORCE VIEW",
    "CREATE OR REPLACE NOFORCE VIEW",
    "CREATE FORCE VIEW",
    "CREATE VIEW",
];

/// Case-insensitive ASCII prefix check that avoids allocating an uppercased
/// copy of `haystack` (which can be a whole multi-KB `CREATE TABLE` body) just
/// to compare its first few bytes against a short keyword.
fn starts_with_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    haystack.get(..needle.len()).is_some_and(|prefix| prefix.eq_ignore_ascii_case(needle))
}

/// Native DDL captured from the source connection is schema/database-qualified
/// against the *source* schema (see `render_postgres_table_ddl_with_partition_info`
/// and `build_view_ddl_sql`). When the diff engine reuses that DDL verbatim for a
/// same-dialect sync script, running it against a different target schema fails
/// outright — the statement still names the source schema, which may not even
/// exist on the target connection.
///
/// Rewrites the object name in a `CREATE ...` header to `qualified` when the
/// captured name is already schema/database-qualified (contains a `.`). An
/// unqualified name (e.g. MySQL DDL relying on the connection's current
/// database) is left untouched, since it already resolves correctly wherever
/// the sync script runs.
fn rewrite_ddl_header_qualifier(ddl: &str, header_keywords: &[&str], schema: Option<&str>, qualified: &str) -> String {
    let leading_ws = ddl.len() - ddl.trim_start().len();
    let body = &ddl[leading_ws..];
    let Some(keyword) = header_keywords.iter().find(|keyword| starts_with_ignore_ascii_case(body, keyword)) else {
        return ddl.to_string();
    };

    let mut idx = skip_ascii_whitespace(ddl, leading_ws + keyword.len());
    if starts_with_ignore_ascii_case(&ddl[idx..], "IF NOT EXISTS") {
        idx = skip_ascii_whitespace(ddl, idx + "IF NOT EXISTS".len());
    }

    let ident_start = idx;
    let Some(parsed) = parse_qualified_identifier(ddl, idx) else {
        return ddl.to_string();
    };
    let Some((schema_start, schema_end)) = parsed.schema_span else {
        // Unqualified name (e.g. MySQL DDL relying on the connection's
        // current database) already resolves correctly wherever the sync
        // script runs — leave it untouched.
        return ddl.to_string();
    };
    let embedded_schema = strip_identifier_quotes(&ddl[schema_start..schema_end]);
    if schema.map(str::trim).is_some_and(|schema| schema == embedded_schema) {
        // Already qualified with the target schema — avoid needlessly
        // reformatting DDL that's already correct.
        return ddl.to_string();
    }
    format!("{}{}{}", &ddl[..ident_start], qualified, &ddl[parsed.end..])
}

struct ParsedDdlName {
    end: usize,
    schema_span: Option<(usize, usize)>,
}

/// Parses a (possibly dotted, possibly quoted) identifier starting at `idx`.
/// Returns the byte offset just past it, plus the span of the schema/database
/// segment if the name was qualified, or `None` if `idx` isn't the start of
/// an identifier. Handles the quoting styles used across DDL dialects: double
/// quotes, backticks, and SQL Server brackets.
fn parse_qualified_identifier(ddl: &str, start: usize) -> Option<ParsedDdlName> {
    let mut idx = start;
    let mut schema_span = None;
    let mut segment_start = start;
    loop {
        let segment_end = parse_identifier_segment_end(ddl, idx)?;
        idx = segment_end;
        if ddl[idx..].starts_with('.') {
            schema_span = Some((segment_start, segment_end));
            idx += 1;
            segment_start = idx;
            continue;
        }
        break;
    }
    Some(ParsedDdlName { end: idx, schema_span })
}

/// Parses one identifier segment (quoted or bare) starting at `idx` and
/// returns the byte offset just past it, or `None` if `idx` isn't the start
/// of a segment.
fn parse_identifier_segment_end(ddl: &str, mut idx: usize) -> Option<usize> {
    let start = idx;
    let ch = ddl[idx..].chars().next()?;
    let closing_quote = match ch {
        '"' => Some('"'),
        '`' => Some('`'),
        '[' => Some(']'),
        _ => None,
    };
    if let Some(closing_quote) = closing_quote {
        idx += ch.len_utf8();
        loop {
            let next = ddl[idx..].chars().next()?;
            idx += next.len_utf8();
            if next == closing_quote {
                // SQL Server brackets escape a literal `]` the same way
                // double-quote/backtick identifiers escape their own quote
                // char: by doubling it (`[a]]b]` is the identifier `a]b`).
                if ddl[idx..].starts_with(closing_quote) {
                    idx += closing_quote.len_utf8();
                    continue;
                }
                break;
            }
        }
    } else if ch.is_alphanumeric() || ch == '_' {
        while let Some(next) = ddl[idx..].chars().next() {
            if next.is_alphanumeric() || next == '_' {
                idx += next.len_utf8();
            } else {
                break;
            }
        }
    } else {
        return None;
    }
    (idx != start).then_some(idx)
}

/// Strips a single matching pair of identifier-quoting characters (double
/// quotes, backticks, or brackets) from `segment`, undoubling an escaped
/// closing quote. Returns `segment` unchanged if it isn't quoted.
fn strip_identifier_quotes(segment: &str) -> String {
    let mut chars = segment.chars();
    let (Some(first), Some(last)) = (chars.next(), segment.chars().next_back()) else {
        return segment.to_string();
    };
    let matches = matches!((first, last), ('"', '"') | ('`', '`') | ('[', ']'));
    if !matches || segment.len() < 2 {
        return segment.to_string();
    }
    let inner = &segment[first.len_utf8()..segment.len() - last.len_utf8()];
    // Brackets escape their closing char the same way same-char quoting
    // does (`]]` -> `]`), just with a different open/close pair.
    inner.replace(&format!("{last}{last}"), &last.to_string())
}

fn drop_index_sql(table_name: &str, index_name: &str, db_type: DatabaseType, schema: Option<&str>) -> String {
    let profile = profile_for(db_type);
    let table = qualified_name(table_name, db_type, schema);
    {}
    let index = qualified_name(index_name, db_type, schema);
    {}
    if profile.drop_index_uses_on_table {
        format!("DROP INDEX {} ON {table};", quote_id(index_name, db_type))
    } else {
        format!("DROP INDEX IF EXISTS {index};")
    }
}

fn mysql_index_column_sql(column: &str) -> String {
    let trimmed = column.trim();
    // MySQL metadata represents a functional key part as an expression wrapped for CREATE INDEX.
    if trimmed.starts_with("((") && trimmed.ends_with("))") {
        trimmed.to_string()
    } else {
        quote_id(column, DatabaseType::Mysql)
    }
}

pub fn create_index_sql(table_name: &str, index: &IndexInfo, db_type: DatabaseType, schema: Option<&str>) -> String {
    use crate::sql_dialect::ddl_profile::IndexTypePlacement;
    let profile = profile_for(db_type);
    let table = qualified_name(table_name, db_type, schema);
    {}
    let mut columns = index
        .columns
        .iter()
        .enumerate()
        .map(|(i, column)| mysql_index_column_sql(column))
        .collect::<Vec<_>>()
        .join(", ");
    let source_index_type = index.index_type.as_deref().unwrap_or_default().trim();
    let index_type = { source_index_type.to_string() };
    {}
    let unique = if index.is_unique && !index_type.contains("COLUMNSTORE") { "UNIQUE " } else { "" };
    let (type_prefix, using_before_on, using_suffix) = if index_type.is_empty() {
        (String::new(), String::new(), String::new())
    } else {
        match profile.index_type_placement {
            IndexTypePlacement::None => (String::new(), String::new(), String::new()),
            IndexTypePlacement::TypePrefix => (format!("{index_type} "), String::new(), String::new()),
            IndexTypePlacement::UsingBeforeOn => (String::new(), format!(" USING {index_type}"), String::new()),
            IndexTypePlacement::UsingSuffix => (String::new(), String::new(), format!(" USING {index_type}")),
        }
    };
    let included_columns = index.included_columns.clone().unwrap_or_default();
    let include_clause = if !included_columns.is_empty() && profile.index_supports_include && !(false) {
        format!(
            " INCLUDE ({})",
            included_columns.iter().map(|column| quote_id(column, db_type)).collect::<Vec<_>>().join(", ")
        )
    } else {
        String::new()
    };
    {}
    let filter = if profile.index_supports_filter { index.filter.as_deref().unwrap_or_default() } else { "" };
    let filter_clause = if filter.is_empty() { String::new() } else { format!(" WHERE {filter}") };
    let comment = index.comment.as_deref().unwrap_or("");
    let comment_clause = if !comment.trim().is_empty() && profile.index_supports_comment {
        format!(" COMMENT {}", quote_string_literal(comment))
    } else {
        String::new()
    };
    {}
    if columns.is_empty() {
        return format!("-- Skip index {} on {table}: no index columns were available.", index.name);
    }
    // MySQL-style puts USING before ON and omits INCLUDE/WHERE placement used by PG/SS.
    if profile.drop_index_uses_on_table {
        format!(
            "CREATE {unique}{type_prefix}INDEX {}{using_before_on} ON {table} ({columns}){comment_clause};",
            quote_id(&index.name, db_type)
        )
    } else {
        format!(
            "CREATE {unique}{type_prefix}INDEX {} ON {table}{using_suffix} ({columns}){include_clause}{filter_clause};",
            quote_id(&index.name, db_type)
        )
    }
}

fn drop_foreign_key_sql(table_name: &str, fk_name: &str, db_type: DatabaseType, schema: Option<&str>) -> String {
    let profile = profile_for(db_type);
    let table = qualified_name(table_name, db_type, schema);
    let fk = quote_id(fk_name, db_type);
    if profile.drop_fk_as_foreign_key {
        format!("ALTER TABLE {table} DROP FOREIGN KEY {fk};")
    } else {
        format!("ALTER TABLE {table} DROP CONSTRAINT {fk};")
    }
}

fn add_foreign_key_sql(table_name: &str, fk: &ForeignKeyInfo, db_type: DatabaseType, schema: Option<&str>) -> String {
    add_foreign_key_sql_with_reference_separator(table_name, fk, db_type, schema, " ")
}

fn add_foreign_key_sql_with_reference_separator(
    table_name: &str,
    fk: &ForeignKeyInfo,
    db_type: DatabaseType,
    schema: Option<&str>,
    reference_separator: &str,
) -> String {
    let table = qualified_name(table_name, db_type, schema);
    let ref_table = qualified_name(&fk.ref_table, db_type, fk.ref_schema.as_deref().or(schema));
    let action = |kind: &str, value: Option<&String>| -> String {
        let Some(value) = value else {
            return String::new();
        };
        let normalized = value.trim().to_ascii_uppercase();
        {
            return format!(" ON {kind} {value}");
        }
    };
    let on_delete = action("DELETE", fk.on_delete.as_ref());
    let on_update = action("UPDATE", fk.on_update.as_ref());
    format!(
        "ALTER TABLE {table} ADD CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {ref_table}{reference_separator}({}){on_delete}{on_update};",
        quote_id(&fk.name, db_type),
        quote_id(&fk.column, db_type),
        quote_id(&fk.ref_column, db_type)
    )
}

fn target_table_name(diff: &TableDiff) -> &str {
    diff.target_name.as_deref().unwrap_or(&diff.name)
}

fn ddl_column_name(column: &ColumnDiff) -> &str {
    if let (Some(source), Some(target)) = (&column.source, &column.target) {
        if source.name.eq_ignore_ascii_case(&target.name) {
            return &target.name;
        }
    }
    &column.name
}

fn drop_object_sql(diff: &TableDiff, db_type: DatabaseType, schema: Option<&str>, cascade: &str) -> String {
    let object_type = if diff.object_type.as_deref() == Some("view") { "VIEW" } else { "TABLE" };
    let name = qualified_name(target_table_name(diff), db_type, schema);
    {}
    // Oracle only gained `IF EXISTS` in 23c, and Access/Firebird/Db2/Informix/HANA/Teradata
    // never had it; the comparison already knows the object exists on the target, so the
    // direct form is valid and sufficient — the same reasoning `drop_index_sql` uses.
    if !profile_for(db_type).drop_table_supports_if_exists {
        return format!("DROP {object_type} {name}{cascade};");
    }
    format!("DROP {object_type} IF EXISTS {name}{cascade};")
}

/// Bare temporal keywords that are defaults in their own right and must not be quoted.
const TEMPORAL_DEFAULT_KEYWORDS: [&str; 8] =
    ["current_timestamp", "current_date", "current_time", "now", "localtime", "localtimestamp", "getdate", "sysdate"];

/// Prefixes that introduce an already-quoted literal: SQL Server / Sybase `N'x'`,
/// MySQL `b'1'` and `x'1f'`, Postgres `e'\n'`.
const QUOTED_LITERAL_PREFIXES: [&str; 4] = ["n'", "b'", "x'", "e'"];

/// The dialect the column metadata came from.
///
/// The caller does not always declare one. When it does not, the comparison is
/// same-dialect, so the target database is also the source.
fn effective_source_dialect(source_dialect: Option<DialectKind>, db_type: DatabaseType) -> DialectKind {
    source_dialect.unwrap_or_else(|| DialectKind::from_database_type(db_type))
}

/// Render a column default as a SQL literal.
///
/// Drivers hand `column_default` back verbatim and they do not agree on its
/// shape, so the rule has to be bound to the dialect the value came from rather
/// than guessed from the value itself. The same bare token means different
/// things in different databases: `CURRENT_USER` is an expression on Postgres
/// and Oracle, while on MySQL a bare `CURRENT_USER` on a text column is the
/// literal string.
///
/// Only MySQL-family metadata strips the quotes from a string default, so it is
/// the only source that needs any repair here. Everywhere else the value already
/// arrives quoted, cast or wrapped, and is passed through untouched.
///
/// `table_structure_sql::util::format_default_for_sql` and
/// `transfer::format_mysql_default_literal` do the same job on their own paths;
/// both are private to their modules.
fn default_literal(default: &str, data_type: &str, source: DialectKind, extra: Option<&str>) -> String {
    let normalized = default.trim();

    // Every dialect except MySQL returns a string default already quoted, cast
    // or wrapped, so a bare token there is an expression and rewriting it would
    // change its meaning: Postgres `text DEFAULT CURRENT_USER`, Oracle
    // `varchar2 DEFAULT USER`, SQL Server `('x')`.
    if source != DialectKind::Mysql {
        return normalized.to_string();
    }

    // MySQL 8.0.13 and later flag an expression default in `EXTRA`. That marker
    // is authoritative, so consult it before looking at the value at all: a
    // default is a literal unless the server says it was generated.
    if extra.is_some_and(|value| value.to_ascii_uppercase().contains("DEFAULT_GENERATED")) {
        return normalized.to_string();
    }

    // MySQL writes an expression default wrapped in parentheses, `DEFAULT
    // (uuid())`, and reports it that way. The wrapping is the syntax, so it is
    // a reliable marker even when `EXTRA` is not populated. Note this asks
    // whether the value *is* parenthesised, not whether it merely contains a
    // parenthesis: the string default `a(b)` is not wrapped and is still
    // quoted.
    if normalized.starts_with('(') && normalized.ends_with(')') {
        return normalized.to_string();
    }

    // Already a literal: `'x'` or a prefixed form like `x'1f'`.
    let lowered = normalized.to_ascii_lowercase();
    if normalized.starts_with('\'') || QUOTED_LITERAL_PREFIXES.iter().any(|prefix| lowered.starts_with(prefix)) {
        return normalized.to_string();
    }

    let base_type = data_type.split('(').next().unwrap_or(data_type).trim().to_ascii_lowercase();
    let takes_text_literal =
        ["char", "text", "string", "clob", "enum", "set"].iter().any(|kind| base_type.contains(kind));
    let takes_binary_literal = ["binary", "blob", "bytea"].iter().any(|kind| base_type.contains(kind));
    let takes_temporal_literal = base_type.contains("date") || base_type.contains("time");

    // Before 8.0.13 there is no `EXTRA` marker and a temporal column was the
    // only place an expression default could appear.
    if takes_temporal_literal && is_temporal_keyword_default(normalized) {
        return normalized.to_string();
    }
    // A binary default is commonly reported as a hex literal, which is already
    // valid unquoted; a bare string on the same column still needs quoting.
    if takes_binary_literal && is_hex_literal(normalized) {
        return normalized.to_string();
    }
    if takes_text_literal || takes_binary_literal || takes_temporal_literal {
        // Deliberately no parenthesis check. MySQL reports the string default
        // `'a(b)'` as the bare value `a(b)`, and treating a parenthesis as proof
        // of a function call is what produced invalid `DEFAULT a(b)`.
        return format!("'{}'", default.replace('\'', "''"));
    }
    normalized.to_string()
}

/// A bare temporal default, with or without a precision argument, so
/// `CURRENT_TIMESTAMP(6)` and `LOCALTIME(3)` are recognised alongside the bare
/// keywords. `transfer::is_mysql_function_default` already accepts the
/// parenthesised forms and this mirrors it.
///
/// Deliberately keyed on the known keywords rather than on the presence of a
/// parenthesis, so a string default such as `a(b)` is still quoted.
fn is_temporal_keyword_default(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    TEMPORAL_DEFAULT_KEYWORDS.iter().any(|keyword| {
        let keyword = keyword.to_ascii_uppercase();
        upper == keyword || upper.strip_prefix(&keyword).is_some_and(|rest| rest.starts_with('('))
    })
}

/// `0x61`, the shape MySQL reports a binary default in.
fn is_hex_literal(value: &str) -> bool {
    let Some(digits) = value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")) else {
        return false;
    };
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_hexdigit())
}

fn column_comment_sql(
    table_name: &str,
    column_name: &str,
    comment: &str,
    db_type: DatabaseType,
    schema: Option<&str>,
) -> Vec<String> {
    let table = qualified_name(table_name, db_type, schema);
    {}
    let profile = profile_for(db_type);
    if profile.column_comment_via_modify_only {
        return vec![format!("-- Column comment for {column_name}: use ALTER TABLE ... MODIFY COLUMN to set comment")];
    }
    vec![format!("COMMENT ON COLUMN {table}.{} IS {};", quote_id(column_name, db_type), quote_string_literal(comment))]
}

fn table_comment_sql(table_name: &str, comment: &str, db_type: DatabaseType, schema: Option<&str>) -> Vec<String> {
    let profile = profile_for(db_type);
    let table = qualified_name(table_name, db_type, schema);
    {}
    if profile.table_comment_via_alter {
        vec![format!("ALTER TABLE {table} COMMENT = {};", quote_string_literal(comment))]
    } else {
        vec![format!("COMMENT ON TABLE {table} IS {};", quote_string_literal(comment))]
    }
}

fn create_trigger_sql(
    profile: &crate::sql_dialect::ddl_profile::DdlDialectProfile,
    name: &str,
    timing: &str,
    event: &str,
    table: &str,
    body: &str,
) -> String {
    use crate::sql_dialect::ddl_profile::TriggerTemplate;
    let qname = profile.quote_ident(name);
    match profile.trigger_template {
        TriggerTemplate::MysqlStyle => {
            format!("CREATE TRIGGER {qname} {timing} {event} ON {table} FOR EACH ROW BEGIN\n{body} END;")
        }
        TriggerTemplate::PostgresStyle => {
            format!(
                "CREATE TRIGGER {qname} {timing} {event} ON {table} FOR EACH ROW EXECUTE FUNCTION {};",
                body.trim_end_matches(';')
            )
        }
        TriggerTemplate::SqlServerStyle => {
            format!("CREATE TRIGGER {qname} ON {table} {timing} {event} AS BEGIN {body} END;")
        }
        TriggerTemplate::GenericRowBody => {
            format!("CREATE TRIGGER {qname} {timing} {event} ON {table} FOR EACH ROW BEGIN {body} END;")
        }
    }
}

/// PostgreSQL-family drivers read routines through `pg_get_functiondef`, which already
/// returns a complete `CREATE OR REPLACE FUNCTION name(args) ...` statement. Keep that
/// header verb and only swap in the target-qualified name.
fn native_create_routine_sql(definition: &str, qualified_name: &str) -> Option<String> {
    let trimmed = definition.trim().trim_end_matches(';').trim_end();
    let prefix = Regex::new(r"(?i)^CREATE\s+(?:OR\s+REPLACE\s+)?(?:FUNCTION|PROCEDURE)\b").ok()?.find(trimmed)?;
    let definition_after_verb = trimmed[prefix.end()..].trim_start();
    let arguments_start = definition_after_verb.find('(')?;
    Some(format!("{} {qualified_name}{};", prefix.as_str(), &definition_after_verb[arguments_start..]))
}

fn generate_create_table_sql(
    name: &str,
    columns: &[ColumnDiff],
    indexes: &[IndexDiff],
    foreign_keys: &[ForeignKeyDiff],
    table_comment: Option<&str>,
    db_type: DatabaseType,
    schema: Option<&str>,
    source_dialect: Option<DialectKind>,
    field_mappings: &[FieldMapping],
    triggers: &[TriggerInfo],
) -> (String, Vec<MissingRollbackObject>) {
    let mut lines = Vec::new();
    let target_dialect = DialectKind::from_database_type(db_type);
    let profile = profile_for(db_type);
    // Type rewrite: user mappings → profile type_map → DialectKind matrix → normalize.
    // Call sites must not branch on individual DatabaseType values.
    let map_type = |col: &ColumnInfo| -> String {
        let source_type = declared_column_type(col);
        if let Some(user_target) = FieldMapping::apply_with_params(field_mappings, &source_type, target_dialect) {
            return user_target;
        }
        rewrite_column_type(&source_type, db_type, source_dialect)
    };
    let table = qualified_name(name, db_type, schema);

    // Collect column definitions
    let mut col_defs = Vec::new();
    let mut pk_cols = Vec::new();
    let mut has_int_pk = false;
    let mut auto_col_name: Option<String> = None;

    for col_diff in columns {
        let Some(col) = &col_diff.source else {
            continue;
        };
        let col_name = quote_id(&col.name, db_type);
        let mapped_type = map_type(col);
        {}
        let is_int = type_looks_integer(&mapped_type);
        let auto_build = apply_auto_inc_to_column_def(&profile, &col_name, &mapped_type, col, is_int);

        match auto_build {
            AutoIncColumnBuild::Complete { def, .. } => {
                col_defs.push(def);
                if col.is_primary_key {
                    pk_cols.push(col_name);
                }
                continue;
            }
            AutoIncColumnBuild::AppendSuffix { suffix, skip_default, postgres_sequence } => {
                // SQLite accepts AUTOINCREMENT only on an exact INTEGER
                // PRIMARY KEY, so integer aliases must be normalized here too.
                let effective_type = { mapped_type.as_str() };
                let mut def = format!("{} {}", col_name, effective_type);
                def.push_str(&column_modifier_tail(&profile, col, &mapped_type, db_type, source_dialect, skip_default));
                if profile.inline_column_comment {
                    if let Some(comment) = col.comment.as_deref().filter(|c| !c.is_empty()) {
                        def.push_str(&format!(" COMMENT {}", quote_string_literal(comment)));
                    }
                }
                if !suffix.is_empty() {
                    // SQLite requires AUTOINCREMENT to be part of an inline
                    // INTEGER PRIMARY KEY declaration; it cannot be combined
                    // with the table-level PRIMARY KEY clause below.
                    {}
                    def.push_str(suffix);
                }
                if postgres_sequence {
                    has_int_pk = true;
                    auto_col_name = Some(col.name.clone());
                }
                col_defs.push(def);
                if col.is_primary_key && !(false) {
                    pk_cols.push(quote_id(&col.name, db_type));
                }
            }
            AutoIncColumnBuild::Normal { skip_default } => {
                let mut def = format!("{} {}", col_name, mapped_type);
                def.push_str(&column_modifier_tail(&profile, col, &mapped_type, db_type, source_dialect, skip_default));
                if profile.inline_column_comment {
                    if let Some(comment) = col.comment.as_deref().filter(|c| !c.is_empty()) {
                        def.push_str(&format!(" COMMENT {}", quote_string_literal(comment)));
                    }
                }
                col_defs.push(def);
                if col.is_primary_key {
                    pk_cols.push(quote_id(&col.name, db_type));
                }
            }
        }
    }

    if profile.foreign_keys_inline_in_create {
        for fk_diff in foreign_keys {
            let Some(fk) = &fk_diff.source else {
                continue;
            };
            let ref_table = qualified_name(&fk.ref_table, db_type, fk.ref_schema.as_deref().or(schema));
            let on_delete = fk.on_delete.as_ref().map(|action| format!(" ON DELETE {action}")).unwrap_or_default();
            let on_update = fk.on_update.as_ref().map(|action| format!(" ON UPDATE {action}")).unwrap_or_default();
            col_defs.push(format!(
                "CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {}({}){}{}",
                quote_id(&fk.name, db_type),
                quote_id(&fk.column, db_type),
                ref_table,
                quote_id(&fk.ref_column, db_type),
                on_delete,
                on_update
            ));
        }
    }

    let mut create = format!("CREATE TABLE {} (\n", table);
    create.push_str(&format!("  {}", col_defs.join(",\n  ")));

    if !pk_cols.is_empty() {
        create.push_str(&format!(",\n  PRIMARY KEY ({})", pk_cols.join(", ")));
    }

    create.push_str("\n);");
    lines.push(format!("-- Create table: {}", name));
    lines.push(create);
    lines.push(String::new());

    // Postgres identity / sequence
    if has_int_pk {
        if let Some(seq_col) = auto_col_name {
            let seq_name = format!("{}_{}_seq", name, seq_col);
            let quoted_seq = quote_id(&seq_name, db_type);
            let quoted_col = quote_id(&seq_col, db_type);
            lines.push(format!("CREATE SEQUENCE IF NOT EXISTS {} OWNED BY {}.{};", quoted_seq, table, quoted_col));
            lines.push(format!(
                "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT nextval('{}');",
                table, quoted_col, seq_name
            ));
            lines.push(format!("ALTER SEQUENCE {} START WITH 1;", quoted_seq));
            lines.push(String::new());
        }
    }

    // Indexes
    for idx_diff in indexes {
        let Some(idx) = &idx_diff.source else {
            continue;
        };
        if idx.is_primary {
            continue;
        }
        lines.push(create_index_sql(name, idx, db_type, schema));
    }
    if !indexes.is_empty() {
        lines.push(String::new());
    }

    // Foreign Keys (skipped when already inlined into CREATE TABLE via profile)
    for fk_diff in foreign_keys {
        if profile.foreign_keys_inline_in_create {
            continue;
        }
        let Some(fk) = &fk_diff.source else {
            continue;
        };
        lines.push(add_foreign_key_sql_with_reference_separator(name, fk, db_type, schema, ""));
    }
    if !foreign_keys.is_empty() {
        lines.push(String::new());
    }

    // Column comments
    for col_diff in columns {
        let Some(col) = &col_diff.source else {
            continue;
        };
        if let Some(comment) = &col.comment {
            if !comment.is_empty() && !profile.inline_column_comment {
                lines.extend(column_comment_sql(name, &col.name, comment, db_type, schema));
            }
        }
    }

    // Table comment
    if let Some(comment) = table_comment {
        if !comment.is_empty() {
            lines.extend(table_comment_sql(name, comment, db_type, schema));
        }
    }

    // Trigger recreation — collect structured missing objects (do not rely on SQL comments alone).
    let mut missing: Vec<MissingRollbackObject> = Vec::new();
    if !triggers.is_empty() {
        lines.push(String::new());
        for trigger in triggers {
            let event_desc = if trigger.event.to_uppercase().contains("INSERT") {
                "INSERT"
            } else if trigger.event.to_uppercase().contains("UPDATE") {
                "UPDATE"
            } else if trigger.event.to_uppercase().contains("DELETE") {
                "DELETE"
            } else {
                &trigger.event
            };
            let timing = &trigger.timing;

            if let Some(stmt) = &trigger.statement {
                if !stmt.trim().is_empty() {
                    let trimmed = stmt.trim().trim_end_matches(';');
                    {
                        let trigger_sql = create_trigger_sql(&profile, &trigger.name, timing, event_desc, &table, stmt);
                        {
                            lines.push(trigger_sql);
                        }
                    }
                } else {
                    missing.push(MissingRollbackObject {
                        kind: "trigger".to_string(),
                        name: trigger.name.clone(),
                        table: Some(name.to_string()),
                        reason: "trigger body is empty; cannot reconstruct CREATE TRIGGER".to_string(),
                    });
                }
            } else {
                missing.push(MissingRollbackObject {
                    kind: "trigger".to_string(),
                    name: trigger.name.clone(),
                    table: Some(name.to_string()),
                    reason: "trigger statement/body missing from schema snapshot".to_string(),
                });
            }
        }
        if !missing.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "-- WARNING: Rollback DDL is INCOMPLETE — one or more triggers on table '{}' could not be reconstructed.",
                name
            ));
            lines.push("-- Manual intervention required before executing this rollback script.".to_string());
            for m in &missing {
                lines.push(format!("-- missing {}: {} ({})", m.kind, m.name, m.reason));
            }
        }
    }

    (lines.join("\n"), missing)
}

fn append_sequence_diff_sql(
    lines: &mut Vec<String>,
    sequence_diffs: &[SequenceDiff],
    profile: DdlDialectProfile,
    db_type: DatabaseType,
    schema: Option<&str>,
    cascade: &str,
    should_render: impl Fn(&str) -> bool,
) {
    let mut matching = sequence_diffs.iter().filter(|diff| should_render(&diff.diff_type)).peekable();
    if matching.peek().is_none() {
        return;
    }

    lines.push(String::new());
    lines.push("-- Sequences".to_string());
    for diff in matching {
        match diff.diff_type.as_str() {
            "added" => {
                if let Some(source) = &diff.source {
                    if let Some(template) = profile.sequence_create_template {
                        lines.push(format!("-- Create sequence: {}", diff.name));
                        let name = qualified_name(&diff.name, db_type, schema);
                        let cycle = if source.cycle { "CYCLE" } else { "NO CYCLE" };
                        lines.push(DdlDialectProfile::render_template(
                            template,
                            &[
                                ("name", &name),
                                ("data_type", &source.data_type),
                                ("start_value", &source.start_value),
                                ("increment", &source.increment),
                                ("min_value", &source.min_value),
                                ("max_value", &source.max_value),
                                ("cycle", cycle),
                            ],
                        ));
                    } else {
                        lines.push(format!(
                            "-- Skip sequence {}: target database does not support sequence DDL generation",
                            diff.name
                        ));
                    }
                }
            }
            "removed" => {
                if let Some(template) = profile.sequence_drop_template {
                    lines.push(format!("-- Drop sequence: {}", diff.name));
                    let name = qualified_name(&diff.name, db_type, schema);
                    lines.push(DdlDialectProfile::render_template(template, &[("name", &name), ("cascade", cascade)]));
                } else {
                    lines.push(format!("-- Skip drop sequence {}: unsupported on target", diff.name));
                }
            }
            "modified" => {
                if let Some(source) = &diff.source {
                    if let Some(template) = profile.sequence_alter_template {
                        lines.push(format!("-- Alter sequence: {}", diff.name));
                        {}
                        let name = qualified_name(&diff.name, db_type, schema);
                        let cycle = if source.cycle { "CYCLE" } else { "NO CYCLE" };
                        lines.push(DdlDialectProfile::render_template(
                            template,
                            &[
                                ("name", &name),
                                ("data_type", &source.data_type),
                                ("start_value", &source.start_value),
                                ("increment", &source.increment),
                                ("min_value", &source.min_value),
                                ("max_value", &source.max_value),
                                ("cycle", cycle),
                            ],
                        ));
                    } else {
                        lines.push(format!("-- Skip alter sequence {}: unsupported on target", diff.name));
                    }
                }
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn generate_schema_sync_sql(
    diffs: &[TableDiff],
    function_diffs: &[FunctionDiff],
    sequence_diffs: &[SequenceDiff],
    rule_diffs: &[RuleDiff],
    owner_diffs: &[OwnerDiff],
    db_type: DatabaseType,
    schema: Option<&str>,
    cascade_delete: bool,
    source_dialect: Option<DialectKind>,
    field_mappings: &[FieldMapping],
) -> String {
    generate_schema_sync_sql_inner(
        diffs,
        function_diffs,
        sequence_diffs,
        rule_diffs,
        owner_diffs,
        db_type,
        schema,
        cascade_delete,
        source_dialect,
        field_mappings,
    )
    .0
}

#[allow(clippy::too_many_arguments)]
pub fn generate_schema_sync_sql_plan(
    diffs: &[TableDiff],
    function_diffs: &[FunctionDiff],
    sequence_diffs: &[SequenceDiff],
    rule_diffs: &[RuleDiff],
    owner_diffs: &[OwnerDiff],
    db_type: DatabaseType,
    schema: Option<&str>,
    cascade_delete: bool,
    source_dialect: Option<DialectKind>,
    field_mappings: &[FieldMapping],
    enable_rollback: bool,
) -> SchemaSyncSqlPlan {
    let (sync_sql, _) = generate_schema_sync_sql_inner(
        diffs,
        function_diffs,
        sequence_diffs,
        rule_diffs,
        owner_diffs,
        db_type,
        schema,
        cascade_delete,
        source_dialect,
        field_mappings,
    );

    let (rollback_sync_sql, missing_rollback_objects) = if enable_rollback {
        let dependency_graph = DependencyGraph { nodes: HashMap::new(), topological_order: Vec::new() };
        let rollback_graph = RollbackGraph::from_forward_diffs(diffs, &[], &dependency_graph);
        let (sql, missing) = generate_rollback_sync_sql_with_missing(&rollback_graph, db_type, schema, cascade_delete);
        (Some(sql), missing)
    } else {
        (None, Vec::new())
    };
    let rollback_completeness = if missing_rollback_objects.is_empty() {
        RollbackCompleteness::Complete
    } else {
        RollbackCompleteness::Incomplete
    };

    SchemaSyncSqlPlan { sync_sql, rollback_sync_sql, rollback_completeness, missing_rollback_objects }
}

/// Names of the objects a diff's own statements reference.
///
/// Foreign keys are read from the side that owns the definition: an `added`
/// object only has the source side, a `removed` object only the target side.
/// View definitions have no foreign keys, so their DDL is scanned for table
/// references — the same fallback `DependencyGraph::build_*` uses.
fn diff_referenced_objects(diff: &TableDiff, known: &HashSet<&str>) -> Vec<String> {
    fn push_unique(names: &mut Vec<String>, name: &str) {
        if !name.is_empty() && !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    }

    let mut names: Vec<String> = Vec::new();
    for foreign_key in diff.foreign_keys.as_deref().unwrap_or_default() {
        let info = match diff.diff_type.as_str() {
            "removed" => foreign_key.target.as_ref().or(foreign_key.source.as_ref()),
            _ => foreign_key.source.as_ref().or(foreign_key.target.as_ref()),
        };
        if let Some(info) = info {
            push_unique(&mut names, info.ref_table.as_str());
        }
    }

    if diff.object_type.as_deref() == Some("view") {
        if let Some(ddl) = diff.ddl.as_deref().or(diff.target_ddl.as_deref()) {
            // Identifier quoting would hide the reference from the keyword scan.
            let unquoted: String = ddl.chars().filter(|ch| !matches!(ch, '`' | '"' | '[' | ']')).collect();
            for name in extract_ddl_references(&unquoted, known) {
                push_unique(&mut names, name.as_str());
            }
        }
    }

    names
}

/// Orders table diffs so the generated script runs top-to-bottom on the target.
///
/// A `CREATE TABLE` fails when its foreign key points at a table that does not
/// exist yet, and a `DROP TABLE` fails while another table still references it.
/// Comparison order comes from the object list (alphabetical), so a child table
/// can precede its parent: the batch then aborts halfway and the target ends up
/// partially synced (#9761). Parents are therefore emitted before the tables
/// that reference them, and dropped after them.
///
/// Only diffs with a real dependency move; entries without one keep their
/// relative order, so plans without dependencies are unchanged.
fn order_diffs_for_execution(diffs: &[TableDiff]) -> Vec<&TableDiff> {
    let mut added: HashMap<&str, usize> = HashMap::new();
    let mut removed: HashMap<&str, usize> = HashMap::new();
    for (index, diff) in diffs.iter().enumerate() {
        match diff.diff_type.as_str() {
            "added" => {
                added.insert(diff.name.as_str(), index);
            }
            "removed" => {
                removed.insert(diff.name.as_str(), index);
            }
            _ => {}
        }
    }
    if added.is_empty() && removed.is_empty() {
        return diffs.iter().collect();
    }

    let known: HashSet<&str> = diffs.iter().map(|diff| diff.name.as_str()).collect();
    let references: Vec<Vec<String>> = diffs.iter().map(|diff| diff_referenced_objects(diff, &known)).collect();

    let mut successors: Vec<Vec<usize>> = vec![Vec::new(); diffs.len()];
    let mut in_degree = vec![0usize; diffs.len()];
    let mut seen_edges: HashSet<(usize, usize)> = HashSet::new();
    let mut add_edge = |from: usize, to: usize, successors: &mut Vec<Vec<usize>>, in_degree: &mut Vec<usize>| {
        if from != to && seen_edges.insert((from, to)) {
            successors[from].push(to);
            in_degree[to] += 1;
        }
    };

    for (index, diff) in diffs.iter().enumerate() {
        match diff.diff_type.as_str() {
            // Parents before children.
            "added" => {
                for dependency in &references[index] {
                    if let Some(&parent) = added.get(dependency.as_str()) {
                        add_edge(parent, index, &mut successors, &mut in_degree);
                    }
                }
            }
            // Children before parents, so no table is dropped while still referenced.
            "removed" => {
                for dependency in &references[index] {
                    if let Some(&parent) = removed.get(dependency.as_str()) {
                        add_edge(index, parent, &mut successors, &mut in_degree);
                    }
                }
            }
            // Modified tables emit `ALTER TABLE ... ADD/DROP FOREIGN KEY` inline,
            // so an FK added on a modified table still needs the referenced
            // table's CREATE first, and an FK dropped on a modified table must
            // run before the referenced table's DROP.
            _ => {
                for foreign_key in diff.foreign_keys.as_deref().unwrap_or_default() {
                    let info = foreign_key.source.as_ref().or(foreign_key.target.as_ref());
                    let Some(info) = info else { continue };
                    match foreign_key.diff_type.as_str() {
                        "added" => {
                            if let Some(&parent) = added.get(info.ref_table.as_str()) {
                                add_edge(parent, index, &mut successors, &mut in_degree);
                            }
                        }
                        "removed" => {
                            if let Some(&parent) = removed.get(info.ref_table.as_str()) {
                                add_edge(index, parent, &mut successors, &mut in_degree);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let mut ready: BTreeSet<usize> = (0..diffs.len()).filter(|index| in_degree[*index] == 0).collect();
    let mut ordered: Vec<usize> = Vec::with_capacity(diffs.len());
    let mut placed = vec![false; diffs.len()];
    while let Some(&index) = ready.iter().next() {
        ready.remove(&index);
        ordered.push(index);
        placed[index] = true;
        for &next in &successors[index] {
            in_degree[next] -= 1;
            if in_degree[next] == 0 {
                ready.insert(next);
            }
        }
    }
    // A dependency cycle cannot be ordered; keep the caller's order for it.
    for (index, was_placed) in placed.iter().enumerate() {
        if !was_placed {
            ordered.push(index);
        }
    }

    ordered.into_iter().map(|index| &diffs[index]).collect()
}

fn generate_schema_sync_sql_inner(
    diffs: &[TableDiff],
    function_diffs: &[FunctionDiff],
    sequence_diffs: &[SequenceDiff],
    rule_diffs: &[RuleDiff],
    owner_diffs: &[OwnerDiff],
    db_type: DatabaseType,
    schema: Option<&str>,
    cascade_delete: bool,
    source_dialect: Option<DialectKind>,
    field_mappings: &[FieldMapping],
) -> (String, Vec<MissingRollbackObject>) {
    let mut lines = Vec::new();
    let mut missing_objects: Vec<MissingRollbackObject> = Vec::new();
    let profile = profile_for(db_type);
    // SQL Server has no DROP ... CASCADE syntax (related constraints are handled explicitly
    // by the comparison plan), and Oracle spells the clause `CASCADE CONSTRAINTS` — which
    // `drop_table_supports_cascade` deliberately excludes — so an unknown dialect gets the
    // plain form instead of a clause its server would reject.
    let cascade = if cascade_delete && profile.drop_table_supports_cascade { " CASCADE" } else { "" };

    let map_type = |col: &ColumnInfo| -> String {
        let tgt = DialectKind::from_database_type(db_type);
        let source_type = declared_column_type(col);
        if let Some(user_target) = FieldMapping::apply_with_params(field_mappings, &source_type, tgt) {
            return user_target;
        }
        rewrite_column_type(&source_type, db_type, source_dialect)
    };
    let is_same_dialect =
        source_dialect.map(|source| source == DialectKind::from_database_type(db_type)).unwrap_or(false);

    append_sequence_diff_sql(&mut lines, sequence_diffs, profile, db_type, schema, cascade, |diff_type| {
        diff_type == "added"
    });

    for diff in order_diffs_for_execution(diffs) {
        let target_name = target_table_name(diff);
        let table = qualified_name(target_name, db_type, schema);

        if diff.diff_type == "added" && diff.object_type.as_deref() == Some("view") {
            if let Some(ddl) = &diff.ddl {
                if is_same_dialect || source_dialect.is_none() {
                    let ddl = rewrite_ddl_header_qualifier(ddl, VIEW_DDL_HEADER_KEYWORDS, schema, &table);
                    lines.push(format!("-- Create view: {}", diff.name));
                    lines.push(format!("{};", ddl.trim_end().trim_end_matches(';')));
                    lines.push(String::new());
                    continue;
                }
            }

            lines.push(format!("-- View exists only in source: {}", diff.name));
            if diff.ddl.is_some() {
                lines.push("-- Source view definition cannot be reused across different SQL dialects.".to_string());
            } else {
                lines.push("-- Source view definition is not available from this driver yet.".to_string());
            }
            lines.push(String::new());
            continue;
        }

        if diff.diff_type == "added" && diff.object_type.as_deref() != Some("view") {
            let has_structured_snapshot = diff.columns.as_ref().is_some_and(|columns| !columns.is_empty());
            let is_rollback_recreation = diff.ddl.is_none() && diff.target_ddl.is_some();
            if is_rollback_recreation {
                if has_structured_snapshot {
                    let trigger_infos: Vec<TriggerInfo> = diff
                        .triggers
                        .as_ref()
                        .map_or_else(Vec::new, |triggers| triggers.iter().filter_map(|t| t.source.clone()).collect());
                    let (generated, missing) = generate_create_table_sql(
                        target_name,
                        diff.columns.as_ref().map_or(&[] as &[ColumnDiff], |columns| columns.as_slice()),
                        diff.indexes.as_ref().map_or(&[] as &[IndexDiff], |indexes| indexes.as_slice()),
                        diff.foreign_keys
                            .as_ref()
                            .map_or(&[] as &[ForeignKeyDiff], |foreign_keys| foreign_keys.as_slice()),
                        diff.source_table_comment.as_ref().and_then(|comment| comment.as_deref()),
                        db_type,
                        schema,
                        None,
                        field_mappings,
                        &trigger_infos,
                    );
                    if !generated.is_empty() {
                        lines.push(generated);
                    }
                    missing_objects.extend(missing);
                } else if let Some(ddl) = diff.target_ddl.as_deref() {
                    // Inversion places only the removed target table's native
                    // DDL here, validating that it belongs to the dialect restored.
                    // It's already qualified against the same target schema this
                    // rollback re-runs against, so the rewrite below is normally a
                    // no-op — kept for symmetry with the other native-DDL
                    // passthrough sites in case that invariant ever changes.
                    let ddl = rewrite_ddl_header_qualifier(ddl, TABLE_DDL_HEADER_KEYWORDS, schema, &table);
                    lines.push(format!("-- Recreate table from native target DDL: {}", diff.name));
                    lines.push(format!("{};", ddl.trim_end_matches(';')));
                    lines.push(String::new());
                }
            } else if is_same_dialect
                || (source_dialect.is_none()
                    && diff.ddl.is_some()
                    && (profile.prefers_native_source_ddl || !has_structured_snapshot))
            {
                // Prefer native source DDL when the target profile wants it
                // (MySQL-family), or as fallback without a structured snapshot.
                if let Some(ddl) = &diff.ddl {
                    let ddl = rewrite_ddl_header_qualifier(ddl, TABLE_DDL_HEADER_KEYWORDS, schema, &table);
                    lines.push(format!("-- Create {}: {}", diff.object_type.as_deref().unwrap_or("table"), diff.name));
                    lines.push(format!("{};", ddl.trim_end().trim_end_matches(';')));
                    lines.push(String::new());
                } else if let Some(cols) = &diff.columns {
                    let trigger_infos: Vec<TriggerInfo> = diff
                        .triggers
                        .as_ref()
                        .map_or_else(Vec::new, |triggers| triggers.iter().filter_map(|t| t.source.clone()).collect());
                    let (gen, missing) = generate_create_table_sql(
                        target_name,
                        cols,
                        diff.indexes.as_ref().map_or(&[] as &[IndexDiff], |v| v.as_slice()),
                        diff.foreign_keys.as_ref().map_or(&[] as &[ForeignKeyDiff], |v| v.as_slice()),
                        diff.source_table_comment.as_ref().and_then(|c| c.as_deref()),
                        db_type,
                        schema,
                        source_dialect,
                        field_mappings,
                        &trigger_infos,
                    );
                    if !gen.is_empty() {
                        lines.push(gen);
                    }
                    missing_objects.extend(missing);
                }
            } else if has_structured_snapshot {
                // Cross-dialect → generate CREATE TABLE from column info
                let _cols: &[ColumnDiff] = diff.columns.as_ref().map_or(&[] as &[ColumnDiff], |v| v.as_slice());
                let _idxs: &[IndexDiff] = diff.indexes.as_ref().map_or(&[] as &[IndexDiff], |v| v.as_slice());
                let _fks: &[ForeignKeyDiff] =
                    diff.foreign_keys.as_ref().map_or(&[] as &[ForeignKeyDiff], |v| v.as_slice());
                let trigger_infos: Vec<TriggerInfo> = diff
                    .triggers
                    .as_ref()
                    .map_or_else(Vec::new, |triggers| triggers.iter().filter_map(|t| t.source.clone()).collect());
                let (gen, missing) = generate_create_table_sql(
                    target_name,
                    diff.columns.as_ref().map_or(&[] as &[ColumnDiff], |v| v.as_slice()),
                    diff.indexes.as_ref().map_or(&[] as &[IndexDiff], |v| v.as_slice()),
                    diff.foreign_keys.as_ref().map_or(&[] as &[ForeignKeyDiff], |v| v.as_slice()),
                    diff.source_table_comment.as_ref().and_then(|c| c.as_deref()),
                    db_type,
                    schema,
                    source_dialect,
                    field_mappings,
                    &trigger_infos,
                );
                if !gen.is_empty() {
                    lines.push(gen);
                }
                missing_objects.extend(missing);
            }
            continue;
        }

        if diff.diff_type == "removed" {
            lines.push(format!("-- Drop {}: {}", diff.object_type.as_deref().unwrap_or("table"), diff.name));
            lines.push(drop_object_sql(diff, db_type, schema, cascade));
            lines.push(String::new());
            continue;
        }

        if diff.diff_type != "modified" {
            continue;
        }

        let mut parts = Vec::new();
        let mut standalone_statements = Vec::new();
        if let Some(foreign_keys) = &diff.foreign_keys {
            for fk in foreign_keys {
                if fk.diff_type == "removed" || fk.diff_type == "modified" {
                    lines.push(drop_foreign_key_sql(target_name, &fk.name, db_type, schema));
                }
            }
        }
        {}

        if let Some(columns) = &diff.columns {
            let convert_col =
                |col: &ColumnInfo| -> ColumnInfo { ColumnInfo { data_type: map_type(col), ..col.clone() } };
            for column in columns {
                match column.diff_type.as_str() {
                    "added" => {
                        if let Some(source) = &column.source {
                            {}
                            let position = {
                                match &column.add_position {
                                    Some(ColumnAddPosition::First) => " FIRST".to_string(),
                                    Some(ColumnAddPosition::After(predecessor)) => {
                                        format!(" AFTER {}", quote_id(predecessor, db_type))
                                    }
                                    None => String::new(),
                                }
                            };
                            let definition = column_def(&convert_col(source), db_type, source_dialect);
                            // Oracle has no `COLUMN` keyword here: it parenthesizes the whole
                            // definition (`ADD (AMT NUMBER(12,2))`) instead.
                            parts.push(if profile.add_column_uses_column_keyword {
                                format!("  ADD COLUMN {definition}{position}")
                            } else {
                                format!("  ADD ({definition}){position}")
                            });
                        }
                    }
                    "removed" => {
                        parts.push(format!("  DROP COLUMN {}", quote_id(&column.name, db_type)));
                    }
                    "modified" => {
                        if let Some(source) = &column.source {
                            let target_column_name = ddl_column_name(column);
                            let mut mapped = convert_col(source);
                            if mapped.name != target_column_name {
                                mapped.name = target_column_name.to_string();
                            }
                            if profile.parenthesized_alter_column_clause {
                                // Oracle spells the whole column change as one
                                // `MODIFY (col definition)` clause, which also carries the
                                // `DEFAULT`/`NOT NULL` order the server accepts.
                                if column.changes.iter().any(|change| !change.starts_with("order:")) {
                                    let modify_keyword = profile.alter_modify_keyword();
                                    let mut definition = column_def(&mapped, db_type, source_dialect);
                                    // Re-stating a definition does *not* drop an existing
                                    // `NOT NULL` on Oracle (verified on 11g: the constraint
                                    // survives `MODIFY (c NUMBER(12,2))`), so relaxing a
                                    // column has to spell `NULL` out or the statement reports
                                    // success while the target keeps the old constraint.
                                    if source.is_nullable
                                        && column.changes.iter().any(|change| change.starts_with("nullable:"))
                                    {
                                        definition.push_str(" NULL");
                                    }
                                    parts.push(format!("  {modify_keyword} ({definition})"));
                                }
                            } else if profile.alter_uses_modify_column {
                                if column.changes.iter().any(|change| !change.starts_with("order:")) {
                                    let modify_keyword = profile.alter_modify_keyword();
                                    let include_charset = column.changes.iter().any(|change| {
                                        change.starts_with("character set:") || change.starts_with("collation:")
                                    });
                                    parts.push(format!(
                                        "  {modify_keyword} {}",
                                        column_def_with_charset(&mapped, db_type, source_dialect, include_charset)
                                    ));
                                }
                            } else {
                                let name = quote_id(target_column_name, db_type);
                                if column.changes.iter().any(|change| change.starts_with("type:")) {
                                    parts.push(format!("  ALTER COLUMN {name} TYPE {}", mapped.data_type));
                                }
                                if column.changes.iter().any(|change| change.starts_with("nullable:")) {
                                    parts.push(if source.is_nullable {
                                        format!("  ALTER COLUMN {name} DROP NOT NULL")
                                    } else {
                                        format!("  ALTER COLUMN {name} SET NOT NULL")
                                    });
                                }
                                if column.changes.iter().any(|change| change.starts_with("default:")) {
                                    parts.push(if let Some(default) = &source.column_default {
                                        format!(
                                            "  ALTER COLUMN {name} SET DEFAULT {}",
                                            default_literal(
                                                default,
                                                &mapped.data_type,
                                                effective_source_dialect(source_dialect, db_type),
                                                source.extra.as_deref()
                                            )
                                        )
                                    } else {
                                        format!("  ALTER COLUMN {name} DROP DEFAULT")
                                    });
                                }
                            }
                        }
                    }
                    "renamed" => {
                        if let (Some(source), Some(target_col)) = (&column.source, &column.target) {
                            use crate::sql_dialect::ddl_profile::RenameColumnSyntax;
                            let mapped = convert_col(source);
                            match profile.rename_column {
                                RenameColumnSyntax::MysqlChangeColumn => {
                                    let old_name = quote_id(&target_col.name, db_type);
                                    parts.push(format!(
                                        "  CHANGE COLUMN {} {}",
                                        old_name,
                                        column_def(&mapped, db_type, source_dialect)
                                    ));
                                }
                                RenameColumnSyntax::RenameColumn => {
                                    let old_name = quote_id(&target_col.name, db_type);
                                    let new_name = quote_id(&column.name, db_type);
                                    parts.push(format!("  RENAME COLUMN {old_name} TO {new_name}"));
                                    // Follow-up clauses keep the renamed column's shape in
                                    // sync; their spelling is dialect data because `TYPE` is
                                    // Postgres/ANSI grammar that Oracle rejects.
                                    if source.data_type.to_lowercase() != target_col.data_type.to_lowercase() {
                                        parts.push(profile.alter_column_type_clause(&new_name, &mapped.data_type));
                                    }
                                    if source.is_nullable != target_col.is_nullable {
                                        parts.push(
                                            profile.alter_column_nullability_clause(&new_name, source.is_nullable),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if !standalone_statements.is_empty() || !parts.is_empty() {
            lines.push(format!("-- Alter table: {}", diff.name));
            lines.extend(standalone_statements);
            if !parts.is_empty() {
                if profile.alter_batches_clauses {
                    lines.push(format!("ALTER TABLE {table}"));
                    lines.push(format!("{};", parts.join(",\n")));
                } else {
                    for part in parts {
                        lines.push(format!("ALTER TABLE {table}{part};"));
                    }
                }
            }
            lines.push(String::new());
        }

        if !profile.column_comment_via_modify_only {
            if let Some(columns) = &diff.columns {
                for column in columns {
                    if let Some(source) = &column.source {
                        if column.changes.iter().any(|change| change.starts_with("comment:")) {
                            lines.extend(column_comment_sql(
                                target_name,
                                ddl_column_name(column),
                                source.comment.as_deref().unwrap_or_default(),
                                db_type,
                                schema,
                            ));
                        }
                        if column.diff_type == "added" {
                            if let Some(comment) = &source.comment {
                                lines.extend(column_comment_sql(target_name, &column.name, comment, db_type, schema));
                            }
                        }
                        if column.diff_type == "renamed" {
                            if let Some(comment) = &source.comment {
                                lines.extend(column_comment_sql(target_name, &column.name, comment, db_type, schema));
                            }
                        }
                    }
                }
            }
        }

        if diff.source_table_comment.is_some() && diff.source_table_comment != diff.target_table_comment {
            let comment = diff.source_table_comment.as_ref().and_then(|comment| comment.as_deref()).unwrap_or_default();
            lines.extend(table_comment_sql(target_name, comment, db_type, schema));
        }

        if let Some(indexes) = &diff.indexes {
            for index in indexes {
                match index.diff_type.as_str() {
                    "added" => {
                        if let Some(source) = &index.source {
                            lines.push(create_index_sql(target_name, source, db_type, schema));
                        }
                    }
                    "removed" => {
                        lines.push(drop_index_sql(target_name, &index.name, db_type, schema));
                    }
                    "modified" => {
                        if let Some(source) = &index.source {
                            {
                                lines.push(drop_index_sql(target_name, &index.name, db_type, schema));
                            }
                            lines.push(create_index_sql(target_name, source, db_type, schema));
                        }
                    }
                    _ => {}
                }
            }
        }

        if let Some(foreign_keys) = &diff.foreign_keys {
            for fk in foreign_keys {
                if fk.diff_type == "added" || fk.diff_type == "modified" {
                    if let Some(source) = &fk.source {
                        lines.push(add_foreign_key_sql(target_name, source, db_type, schema));
                    }
                }
            }
        }

        if let Some(triggers) = &diff.triggers {
            for trigger in triggers {
                lines.push(format!(
                    "-- Trigger {}: {} on {}; review trigger definition manually.",
                    trigger.diff_type, trigger.name, diff.name
                ));
            }
        }

        if diff.indexes.as_ref().is_some_and(|indexes| !indexes.is_empty())
            || diff.foreign_keys.as_ref().is_some_and(|foreign_keys| !foreign_keys.is_empty())
            || diff.triggers.as_ref().is_some_and(|triggers| !triggers.is_empty())
        {
            lines.push(String::new());
        }

        if profile.warn_fk_needs_table_rebuild
            && diff.foreign_keys.as_ref().is_some_and(|foreign_keys| !foreign_keys.is_empty())
        {
            lines.push(format!("-- Foreign key synchronization may require table rebuild for: {}", diff.name));
            lines.push(String::new());
        }
    }

    // Function diffs — only emit executable SQL when profile has templates
    if !function_diffs.is_empty() {
        lines.push(String::new());
        lines.push("-- Functions".to_string());
        for diff in function_diffs {
            match diff.diff_type.as_str() {
                "added" | "modified" => {
                    if let Some(source) = &diff.source {
                        {}
                        if let Some(template) = profile.function_create_template {
                            let verb = if diff.diff_type == "added" { "Create" } else { "Alter" };
                            lines.push(format!("-- {verb} function: {}", diff.name));
                            {}
                            {
                                let name = qualified_name(&diff.name, db_type, schema);
                                if let Some(sql) = native_create_routine_sql(&source.definition, &name) {
                                    lines.push(sql);
                                    continue;
                                }
                            }
                            let create_kw = if profile.create_function_or_replace {
                                "CREATE OR REPLACE FUNCTION"
                            } else {
                                "CREATE FUNCTION"
                            };
                            let name = qualified_name(&diff.name, db_type, schema);
                            let function_sql = DdlDialectProfile::render_template(
                                template,
                                &[("create_kw", create_kw), ("name", &name), ("definition", &source.definition)],
                            );
                            {
                                lines.push(function_sql);
                            }
                        } else {
                            lines.push(format!(
                                "-- Skip function {}: target database does not support function DDL generation",
                                diff.name
                            ));
                        }
                    }
                }
                "removed" => {
                    if let Some(template) = profile.function_drop_template {
                        lines.push(format!("-- Drop function: {}", diff.name));
                        let name = qualified_name(&diff.name, db_type, schema);
                        lines.push(DdlDialectProfile::render_template(
                            template,
                            &[("name", &name), ("cascade", cascade)],
                        ));
                    } else {
                        lines.push(format!("-- Skip drop function {}: unsupported on target", diff.name));
                    }
                }
                _ => {}
            }
        }
    }

    append_sequence_diff_sql(&mut lines, sequence_diffs, profile, db_type, schema, cascade, |diff_type| {
        matches!(diff_type, "removed" | "modified")
    });

    // Rule diffs (PostgreSQL RULE)
    if !rule_diffs.is_empty() {
        lines.push(String::new());
        lines.push("-- Rules".to_string());
        for diff in rule_diffs {
            if profile.rule_drop_template.is_none() && !profile.supports_rule_ddl {
                lines.push(format!("-- Skip rule {}: target database does not support RULE DDL", diff.name));
                continue;
            }
            match diff.diff_type.as_str() {
                "added" => {
                    if let Some(source) = &diff.source {
                        if profile.supports_rule_ddl {
                            lines.push(format!("-- Create rule: {}", diff.name));
                            lines.push(source.definition.clone());
                        } else {
                            lines
                                .push(format!("-- Skip rule {}: target database does not support RULE DDL", diff.name));
                        }
                    }
                }
                "removed" => {
                    if let Some(template) = profile.rule_drop_template {
                        lines.push(format!("-- Drop rule: {}", diff.name));
                        // Removed diffs store the object on `target`; tests may put it on `source`.
                        if let Some(rule) = diff.source.as_ref().or(diff.target.as_ref()) {
                            let table_name = qualified_name(&rule.table_name, db_type, schema);
                            lines.push(DdlDialectProfile::render_template(
                                template,
                                &[("rule_name", &diff.name), ("table_name", &table_name), ("cascade", cascade)],
                            ));
                        }
                    } else {
                        lines.push(format!("-- Skip rule {}: target database does not support RULE DDL", diff.name));
                    }
                }
                "modified" => {
                    if let Some(source) = &diff.source {
                        if let Some(template) = profile.rule_drop_template {
                            lines.push(format!("-- Alter rule: {}", diff.name));
                            let table_name = qualified_name(&source.table_name, db_type, schema);
                            lines.push(DdlDialectProfile::render_template(
                                template,
                                &[("rule_name", &diff.name), ("table_name", &table_name), ("cascade", cascade)],
                            ));
                            lines.push(source.definition.clone());
                        } else {
                            lines
                                .push(format!("-- Skip rule {}: target database does not support RULE DDL", diff.name));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Owner diffs
    if !owner_diffs.is_empty() {
        lines.push(String::new());
        lines.push("-- Owners".to_string());
        for diff in owner_diffs {
            if let (Some(source), Some(_target)) = (&diff.source, &diff.target) {
                if let Some(template) = profile.owner_alter_template {
                    let object_type = match source.object_type.as_str() {
                        "TABLE" => "TABLE",
                        "VIEW" => "VIEW",
                        "SEQUENCE" => "SEQUENCE",
                        _ => "TABLE",
                    };
                    let name = qualified_name(&diff.object_name, db_type, schema);
                    lines.push(DdlDialectProfile::render_template(
                        template,
                        &[("object_type", object_type), ("name", &name), ("owner", &source.owner)],
                    ));
                } else {
                    lines.push(format!("-- Skip OWNER change for {}: unsupported on target", diff.object_name));
                }
            }
        }
    }

    (lines.join("\n").trim().to_string(), missing_objects)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(overrides: IndexInfo) -> IndexInfo {
        IndexInfo {
            name: if overrides.name.is_empty() { "idx_users_email".to_string() } else { overrides.name },
            columns: if overrides.columns.is_empty() { vec!["email".to_string()] } else { overrides.columns },
            is_unique: overrides.is_unique,
            is_primary: overrides.is_primary,
            filter: overrides.filter,
            index_type: overrides.index_type,
            included_columns: overrides.included_columns,
            comment: overrides.comment,
            key_is_expression: overrides.key_is_expression,
            column_opclasses: overrides.column_opclasses,
            key_options: overrides.key_options,
            constraint_backed: false,
        }
    }

    fn foreign_key(overrides: ForeignKeyInfo) -> ForeignKeyInfo {
        ForeignKeyInfo {
            name: if overrides.name.is_empty() { "orders_user_id_fk".to_string() } else { overrides.name },
            column: if overrides.column.is_empty() { "user_id".to_string() } else { overrides.column },
            ref_schema: overrides.ref_schema,
            ref_table: if overrides.ref_table.is_empty() { "users".to_string() } else { overrides.ref_table },
            ref_column: if overrides.ref_column.is_empty() { "id".to_string() } else { overrides.ref_column },
            on_update: overrides.on_update,
            on_delete: overrides.on_delete,
        }
    }

    fn column(name: &str, data_type: &str, comment: Option<&str>) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            resolved_schema: None,
            is_nullable: false,
            column_default: None,
            is_primary_key: false,
            is_unique: false,
            extra: None,
            comment: comment.map(str::to_string),
            numeric_precision: None,
            numeric_scale: None,
            character_maximum_length: None,
            metadata_capabilities: None,
            enum_values: None,
            character_set: None,
            collation: None,
        }
    }

    fn table_info(name: &str, table_type: &str) -> TableInfo {
        TableInfo {
            name: name.to_string(),
            table_type: table_type.to_string(),
            valid: None,
            comment: None,
            parent_schema: None,
            parent_name: None,
        }
    }

    fn schema_detail(name: &str, ddl: Option<&str>) -> TableSchemaDetail {
        TableSchemaDetail {
            name: name.to_string(),
            columns: vec![],
            indexes: vec![],
            foreign_keys: vec![],
            triggers: vec![],
            ddl: ddl.map(str::to_string),
        }
    }

    fn common_mysql_view_options(source_ddl: Option<&str>, target_ddl: Option<&str>) -> SchemaDiffPreparationOptions {
        SchemaDiffPreparationOptions {
            source_tables: vec![table_info("active_orders", "VIEW")],
            target_tables: vec![table_info("active_orders", "VIEW")],
            source_details: vec![schema_detail("active_orders", source_ddl)],
            target_details: vec![schema_detail("active_orders", target_ddl)],
            database_type: DatabaseType::Mysql,
            source_dialect: Some(DialectKind::Mysql),
            target_dialect: Some(DialectKind::Mysql),
            ..Default::default()
        }
    }

    fn mysql_charset_options(compare_charset: bool) -> SchemaDiffPreparationOptions {
        let mut source_column = column("name", "varchar(64)", None);
        source_column.character_set = Some("utf8mb4".to_string());
        source_column.collation = Some("utf8mb4_0900_ai_ci".to_string());
        let mut target_column = column("name", "varchar(64)", None);
        target_column.character_set = Some("latin1".to_string());
        target_column.collation = Some("latin1_swedish_ci".to_string());

        SchemaDiffPreparationOptions {
            source_tables: vec![table_info("users", "TABLE")],
            target_tables: vec![table_info("users", "TABLE")],
            source_details: vec![TableSchemaDetail {
                name: "users".to_string(),
                columns: vec![source_column],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
                triggers: Vec::new(),
                ddl: None,
            }],
            target_details: vec![TableSchemaDetail {
                name: "users".to_string(),
                columns: vec![target_column],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
                triggers: Vec::new(),
                ddl: None,
            }],
            database_type: DatabaseType::Mysql,
            source_dialect: Some(DialectKind::Mysql),
            target_dialect: Some(DialectKind::Mysql),
            compare_charset,
            ..Default::default()
        }
    }

    #[test]
    fn mysql_charset_comparison_is_enabled_by_default_and_serializes_as_camel_case() {
        let legacy: SchemaDiffPreparationOptions =
            serde_json::from_value(serde_json::json!({ "databaseType": "mysql" })).unwrap();
        assert!(legacy.compare_charset);

        let json = serde_json::to_value(legacy).unwrap();
        assert_eq!(json["compareCharset"], true);
        assert!(json.get("compare_charset").is_none());

        let disabled: SchemaDiffPreparationOptions =
            serde_json::from_value(serde_json::json!({ "databaseType": "mysql", "compareCharset": false })).unwrap();
        assert!(!disabled.compare_charset);
    }

    #[test]
    fn mysql_charset_only_difference_is_reported_when_enabled() {
        let result = prepare_schema_diff(mysql_charset_options(true));
        let columns = result.diffs[0].columns.as_ref().expect("column diff");

        assert_eq!(
            columns[0].changes,
            vec![
                "character set: latin1 → utf8mb4".to_string(),
                "collation: latin1_swedish_ci → utf8mb4_0900_ai_ci".to_string(),
            ]
        );
        assert!(result.sync_sql.contains("CHARACTER SET `utf8mb4` COLLATE `utf8mb4_0900_ai_ci`"));
    }

    #[test]
    fn mysql_charset_only_difference_is_suppressed_when_disabled_and_restored_when_reenabled() {
        assert!(prepare_schema_diff(mysql_charset_options(false)).diffs.is_empty());
        assert_eq!(prepare_schema_diff(mysql_charset_options(true)).diffs.len(), 1);
    }

    #[test]
    fn mysql_charset_missing_metadata_is_unknown() {
        let mut options = mysql_charset_options(true);
        options.target_details[0].columns[0].character_set = None;
        options.target_details[0].columns[0].collation = None;
        options.source_details[0].ddl =
            Some("CREATE TABLE users (name varchar(64)) DEFAULT CHARSET=utf8mb4".to_string());
        options.target_details[0].ddl =
            Some("CREATE TABLE users (name varchar(64)) DEFAULT CHARSET=latin1".to_string());

        assert!(prepare_schema_diff(options).diffs.is_empty());
    }

    #[test]
    fn disabling_mysql_charset_comparison_preserves_other_column_and_key_differences() {
        let mut options = mysql_charset_options(false);
        let source_column = &mut options.source_details[0].columns[0];
        source_column.data_type = "varchar(128)".to_string();
        source_column.is_nullable = false;
        source_column.column_default = Some("source".to_string());
        source_column.comment = Some("source comment".to_string());
        let target_column = &mut options.target_details[0].columns[0];
        target_column.is_nullable = true;
        target_column.column_default = Some("target".to_string());
        target_column.comment = Some("target comment".to_string());
        options.source_details[0].indexes.push(index(IndexInfo {
            name: "uq_users_name".to_string(),
            columns: vec!["name".to_string()],
            is_unique: true,
            is_primary: false,
            filter: None,
            index_type: None,
            included_columns: None,
            comment: None,
            key_is_expression: Vec::new(),
            column_opclasses: Vec::new(),
            key_options: Vec::new(),
            constraint_backed: false,
        }));

        let result = prepare_schema_diff(options);
        let table = &result.diffs[0];
        let column_changes = &table.columns.as_ref().expect("column diff")[0].changes;
        assert!(column_changes.iter().any(|change| change.starts_with("type:")));
        assert!(column_changes.iter().any(|change| change.starts_with("nullable:")));
        assert!(column_changes.iter().any(|change| change.starts_with("default:")));
        assert!(column_changes.iter().any(|change| change.starts_with("comment:")));
        assert!(!column_changes.iter().any(|change| change.starts_with("character set:")));
        assert!(!column_changes.iter().any(|change| change.starts_with("collation:")));
        assert!(table.indexes.as_ref().is_some_and(|indexes| {
            indexes.iter().any(|index| {
                index.diff_type == "added"
                    && index.source.as_ref().is_some_and(|info| info.is_unique && info.name == "uq_users_name")
            })
        }));
    }

    #[test]
    fn manual_table_mapping_compares_source_with_target_metadata_and_uses_target_ddl_name() {
        let result = prepare_schema_diff(SchemaDiffPreparationOptions {
            source_tables: vec![table_info("charge_records", "TABLE")],
            target_tables: vec![table_info("charge_record", "TABLE")],
            source_details: vec![TableSchemaDetail {
                name: "charge_records".to_string(),
                columns: vec![column("id", "int", None), column("amount", "decimal(12,2)", None)],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
                triggers: Vec::new(),
                ddl: Some("CREATE TABLE charge_records (id int, amount decimal(12,2));".to_string()),
            }],
            target_details: vec![TableSchemaDetail {
                name: "charge_record".to_string(),
                columns: vec![column("id", "int", None), column("amount", "decimal(10,2)", None)],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
                triggers: Vec::new(),
                ddl: Some("CREATE TABLE charge_record (id int, amount decimal(10,2));".to_string()),
            }],
            database_type: DatabaseType::Mysql,
            table_mappings: vec![SchemaDiffTableMapping {
                source_table: "charge_records".to_string(),
                target_table: "charge_record".to_string(),
            }],
            ..Default::default()
        });

        assert_eq!(result.diffs.len(), 1);
        let table_diff = &result.diffs[0];
        assert_eq!(table_diff.diff_type, "modified");
        assert_eq!(table_diff.name, "charge_records");
        assert_eq!(table_diff.target_name.as_deref(), Some("charge_record"));
        assert!(table_diff
            .columns
            .as_ref()
            .is_some_and(|columns| columns.iter().any(|column| column.name == "amount")));
        assert!(result.sync_sql.contains("ALTER TABLE `charge_record`"), "sync SQL: {}", result.sync_sql);
        assert!(!result.sync_sql.contains("ALTER TABLE `charge_records`"), "sync SQL: {}", result.sync_sql);
    }

    #[test]
    fn ignores_column_order_when_option_is_disabled() {
        let diffs = diff_columns_with_options(
            &[column("id", "int", None), column("name", "varchar(64)", None), column("status", "varchar(16)", None)],
            &[column("status", "varchar(16)", None), column("id", "int", None), column("name", "varchar(64)", None)],
            false,
            false,
            false,
            0.5,
        );

        assert!(diffs.is_empty());
    }

    #[test]
    fn detects_column_order_when_option_is_enabled() {
        let diffs = diff_columns_with_options(
            &[column("id", "int", None), column("name", "varchar(64)", None), column("status", "varchar(16)", None)],
            &[column("status", "varchar(16)", None), column("id", "int", None), column("name", "varchar(64)", None)],
            false,
            true,
            false,
            0.5,
        );

        assert_eq!(diffs.len(), 3);
        assert_eq!(diffs[0].changes, vec!["order: 2 → 1"]);
    }

    #[test]
    fn detects_column_rename_with_same_type() {
        let source = vec![
            column("id", "int", None),
            column("name2", "varchar(120)", None),
            column("del_flag", "tinyint", None),
            column("create_at", "datetime", None),
        ];
        let target =
            vec![column("id", "int", None), column("name", "varchar(120)", None), column("del_flag", "tinyint", None)];
        let diffs = diff_columns_with_options(&source, &target, false, false, true, 0.5);
        let renamed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "renamed").collect();
        let added: Vec<_> = diffs.iter().filter(|d| d.diff_type == "added").collect();
        let removed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "removed").collect();
        assert_eq!(renamed.len(), 1, "should detect one renamed column");
        assert_eq!(renamed[0].name, "name2");
        assert_eq!(renamed[0].changes, vec!["name → name2"]);
        assert_eq!(added.len(), 1, "should have one truly added column (create_at)");
        assert_eq!(added[0].name, "create_at");
        assert!(removed.is_empty(), "should have no removed columns");
    }

    #[test]
    fn detects_column_rename_with_compatible_type() {
        let source = vec![column("col_a", "varchar(64)", None), column("col_b", "int", None)];
        let target = vec![column("col_a_old", "varchar(100)", None), column("col_b", "int", None)];
        let diffs = diff_columns_with_options(&source, &target, false, false, true, 0.5);
        let renamed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "renamed").collect();
        assert_eq!(renamed.len(), 1, "should detect rename across varchar family");
        assert_eq!(renamed[0].changes, vec!["col_a_old → col_a"]);
    }

    #[test]
    fn no_rename_detection_when_disabled() {
        let source = vec![
            column("id", "int", None),
            column("name2", "varchar(120)", None),
            column("create_at", "datetime", None),
        ];
        let target = vec![column("id", "int", None), column("name", "varchar(120)", None)];
        let diffs = diff_columns_with_options(&source, &target, false, false, false, 0.5);
        let renamed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "renamed").collect();
        let added: Vec<_> = diffs.iter().filter(|d| d.diff_type == "added").collect();
        let removed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "removed").collect();
        assert!(renamed.is_empty(), "should not detect renames when disabled");
        assert_eq!(added.len(), 2);
        assert_eq!(removed.len(), 1);
    }

    #[test]
    fn rename_not_detected_with_unrelated_types() {
        let source = vec![column("col_a", "varchar(120)", None), column("col_b", "int", None)];
        let target = vec![column("col_old", "int", None), column("col_b", "int", None)];
        let diffs = diff_columns_with_options(&source, &target, false, false, true, 0.5);
        let renamed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "renamed").collect();
        assert!(renamed.is_empty(), "should not rename across unrelated types");
    }

    #[test]
    fn rename_with_rollback_graph_inversion() {
        let source = vec![column("id", "int", None), column("new_name", "varchar(120)", None)];
        let target = vec![column("id", "int", None), column("old_name", "varchar(120)", None)];
        let diffs = diff_columns_with_options(&source, &target, false, false, true, 0.5);
        let inverted = RollbackGraph::invert_columns(&diffs);
        let renamed_inv: Vec<_> = inverted.iter().filter(|d| d.diff_type == "renamed").collect();
        assert_eq!(renamed_inv.len(), 1, "inverted rename should exist");
        assert_eq!(renamed_inv[0].name, "old_name");
        assert_eq!(renamed_inv[0].changes, vec!["new_name → old_name"]);
    }

    // -- helpers ----------------------------------------------
    fn make_col_diffs(source: &[(&str, &str)], target: &[(&str, &str)], detect_renames: bool) -> Vec<ColumnDiff> {
        let s: Vec<ColumnInfo> = source.iter().map(|(n, t)| column(n, t, None)).collect();
        let t: Vec<ColumnInfo> = target.iter().map(|(n, t)| column(n, t, None)).collect();
        diff_columns_with_options(&s, &t, false, false, detect_renames, 0.5)
    }

    fn wrap_table_diff(name: &str, columns: Vec<ColumnDiff>) -> TableDiff {
        TableDiff {
            diff_type: "modified".to_string(),
            object_type: Some("table".to_string()),
            name: name.to_string(),
            target_name: None,
            columns: Some(columns),
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: None,
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        }
    }

    fn gen_sql(diff: TableDiff, db_type: DatabaseType, source_dialect: Option<DialectKind>) -> String {
        generate_schema_sync_sql(&[diff], &[], &[], &[], &[], db_type, None, false, source_dialect, &[])
    }

    // -- 1. Same-dialect: MySQL (backticks, MODIFY/CHANGE/ADD COLUMN) --
    #[test]
    fn mysql_same_dialect_rename_and_add() {
        let diffs = make_col_diffs(
            &[("id", "int(11)"), ("name2", "varchar(120)"), ("del_flag", "tinyint(2)"), ("create_at", "datetime")],
            &[("id", "int"), ("name", "varchar(120)"), ("del_flag", "tinyint")],
            true,
        );
        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);
        assert!(sql.contains("CHANGE COLUMN `name` `name2`"), "MySQL rename: {sql}");
        assert!(sql.contains("ADD COLUMN `create_at`"), "MySQL new col: {sql}");
        assert!(sql.contains("MODIFY COLUMN `id`"), "MySQL modify type: {sql}");
        assert!(!sql.contains("DROP COLUMN"), "MySQL no drop: {sql}");
    }

    #[test]
    fn mysql_add_columns_preserve_source_positions() {
        let diffs = make_col_diffs(
            &[("first", "int"), ("a", "int"), ("middle", "varchar(32)"), ("next", "int"), ("last", "int")],
            &[("a", "int"), ("last", "int")],
            false,
        );

        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);

        assert!(sql.contains("ADD COLUMN `first` int NOT NULL FIRST"), "first position: {sql}");
        assert!(sql.contains("ADD COLUMN `middle` varchar(32) NOT NULL AFTER `a`"), "middle position: {sql}");
        assert!(sql.contains("ADD COLUMN `next` int NOT NULL AFTER `middle`"), "consecutive additions: {sql}");
    }

    #[test]
    fn mysql_add_column_quotes_predecessor_and_keeps_trailing_position() {
        let diffs = make_col_diffs(
            &[("odd`name", "int"), ("middle", "int"), ("tail", "int"), ("new_tail", "int")],
            &[("odd`name", "int"), ("tail", "int")],
            false,
        );

        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);

        assert!(sql.contains("ADD COLUMN `middle` int NOT NULL AFTER `odd``name`"), "quoted predecessor: {sql}");
        assert!(sql.contains("ADD COLUMN `new_tail` int NOT NULL AFTER `tail`"), "trailing position: {sql}");
    }

    #[test]
    fn mysql_add_after_renamed_predecessor_uses_final_source_name() {
        let diffs =
            make_col_diffs(&[("new_name", "varchar(32)"), ("inserted", "int")], &[("old_name", "varchar(32)")], true);

        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);

        assert!(sql.contains("ADD COLUMN `inserted` int NOT NULL AFTER `new_name`"), "rename predecessor: {sql}");
        assert!(sql.contains("CHANGE COLUMN `old_name` `new_name`"), "rename remains present: {sql}");
    }

    #[test]
    fn mysql_manual_added_diff_without_position_keeps_legacy_sql() {
        let diff = ColumnDiff {
            diff_type: "added".into(),
            name: "legacy".into(),
            source: Some(column("legacy", "int", None)),
            target: None,
            changes: Vec::new(),
            add_position: None,
        };

        let sql = gen_sql(wrap_table_diff("t", vec![diff]), DatabaseType::Mysql, None);

        assert!(sql.contains("ADD COLUMN `legacy` int NOT NULL"), "legacy add: {sql}");
        assert!(!sql.contains(" FIRST"), "legacy diff has no position: {sql}");
        assert!(!sql.contains(" AFTER "), "legacy diff has no position: {sql}");
    }

    #[test]
    fn mysql_same_dialect_add_drop_modified() {
        let diffs = make_col_diffs(
            &[("id", "int"), ("new_col", "varchar(50)")],
            &[("id", "bigint"), ("old_col", "int")],
            false,
        );
        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);
        assert!(sql.contains("ADD COLUMN `new_col`"), "MySQL add: {sql}");
        assert!(sql.contains("DROP COLUMN `old_col`"), "MySQL drop: {sql}");
        assert!(sql.contains("MODIFY COLUMN `id`"), "MySQL modify: {sql}");
    }

    #[test]
    fn schema_qualified_mysql() {
        let diffs = make_col_diffs(&[("name2", "varchar(50)")], &[("name", "varchar(50)")], true);
        let table_diff = wrap_table_diff("users", diffs);
        let sql = generate_schema_sync_sql(
            &[table_diff],
            &[],
            &[],
            &[],
            &[],
            DatabaseType::Mysql,
            Some("mydb"),
            false,
            None,
            &[],
        );
        assert!(sql.contains("`mydb`.`users`"), "schema prefixed MySQL: {sql}");
    }

    #[test]
    fn multiple_concurrent_operations_mysql() {
        let diffs = make_col_diffs(
            &[("id", "int"), ("name2", "varchar(50)"), ("new_col", "text")],
            &[("id", "bigint"), ("name", "varchar(50)")],
            true,
        );
        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);
        assert!(sql.contains("CHANGE COLUMN"), "MySQL rename: {sql}");
        assert!(sql.contains("ADD COLUMN"), "MySQL add: {sql}");
        assert!(sql.contains("MODIFY COLUMN"), "MySQL modify: {sql}");
    }

    // -- 20. All-removed and all-added edge cases --
    #[test]
    fn all_columns_removed() {
        let diffs = make_col_diffs(&[], &[("old1", "int"), ("old2", "varchar(10)")], false);
        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);
        assert_eq!(sql.matches("DROP COLUMN").count(), 2, "two drops: {sql}");
    }

    #[test]
    fn detects_modified_indexes_not_only_added_or_removed_indexes() {
        let diffs = diff_indexes(
            &[index(IndexInfo {
                name: "idx_orders_status".to_string(),
                columns: vec!["status".to_string(), "created_at".to_string()],
                is_unique: false,
                is_primary: false,
                filter: None,
                index_type: None,
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
            &[index(IndexInfo {
                name: "idx_orders_status".to_string(),
                columns: vec!["status".to_string()],
                is_unique: true,
                is_primary: false,
                filter: None,
                index_type: None,
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
        );

        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].diff_type, "modified");
        assert_eq!(diffs[0].changes, vec!["unique: YES → NO", "columns: status → status, created_at"]);
    }

    #[test]
    fn detects_mysql_functional_index_changes_and_preserves_expression_ddl() {
        let functional_key_part = "((case when (`STATUS` = _utf8mb4'online') then _utf8mb4'online' else NULL end))";
        let source_index = index(IndexInfo {
            name: "test_UNIQUE".to_string(),
            columns: vec!["attr".to_string(), "attr2".to_string(), functional_key_part.to_string()],
            is_unique: true,
            is_primary: false,
            filter: None,
            index_type: None,
            included_columns: None,
            comment: None,
            key_is_expression: Vec::new(),
            column_opclasses: vec![],
            key_options: Vec::new(),
            constraint_backed: false,
        });
        let target_index = index(IndexInfo {
            name: "test_UNIQUE".to_string(),
            columns: vec!["attr".to_string(), "attr2".to_string()],
            is_unique: true,
            is_primary: false,
            filter: None,
            index_type: None,
            included_columns: None,
            comment: None,
            key_is_expression: Vec::new(),
            column_opclasses: vec![],
            key_options: Vec::new(),
            constraint_backed: false,
        });

        let diffs = diff_indexes(std::slice::from_ref(&source_index), &[target_index]);
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].diff_type, "modified");
        assert_eq!(diffs[0].changes, vec![format!("columns: attr, attr2 → attr, attr2, {functional_key_part}")]);

        let sql = generate_schema_sync_sql(
            &[TableDiff {
                diff_type: "modified".to_string(),
                object_type: Some("table".to_string()),
                name: "test".to_string(),
                target_name: None,
                columns: None,
                indexes: Some(diffs),
                foreign_keys: None,
                triggers: None,
                ddl: None,
                target_ddl: None,
                source_table_comment: None,
                target_table_comment: None,
                sync_sql: None,
            }],
            &[],
            &[],
            &[],
            &[],
            DatabaseType::Mysql,
            Some("dbx_issue_4114"),
            false,
            None,
            &[],
        );

        assert!(sql.contains("DROP INDEX `test_UNIQUE` ON `dbx_issue_4114`.`test`;"));
        assert!(sql.contains(&format!(
            "CREATE UNIQUE INDEX `test_UNIQUE` ON `dbx_issue_4114`.`test` (`attr`, `attr2`, {functional_key_part});"
        )));
        assert!(!sql.contains("`((case"));
    }

    #[test]
    fn detects_foreign_key_additions_removals_and_target_changes() {
        let diffs = diff_foreign_keys(
            &[
                foreign_key(ForeignKeyInfo {
                    name: "orders_user_id_fk".to_string(),
                    column: String::new(),
                    ref_schema: None,
                    ref_table: String::new(),
                    ref_column: String::new(),
                    on_update: None,
                    on_delete: None,
                }),
                foreign_key(ForeignKeyInfo {
                    name: "orders_account_id_fk".to_string(),
                    column: "account_id".to_string(),
                    ref_schema: None,
                    ref_table: "accounts".to_string(),
                    ref_column: String::new(),
                    on_update: None,
                    on_delete: None,
                }),
            ],
            &[
                foreign_key(ForeignKeyInfo {
                    name: "orders_user_id_fk".to_string(),
                    column: String::new(),
                    ref_schema: None,
                    ref_table: "members".to_string(),
                    ref_column: String::new(),
                    on_update: None,
                    on_delete: None,
                }),
                foreign_key(ForeignKeyInfo {
                    name: "orders_region_id_fk".to_string(),
                    column: "region_id".to_string(),
                    ref_schema: None,
                    ref_table: "regions".to_string(),
                    ref_column: String::new(),
                    on_update: None,
                    on_delete: None,
                }),
            ],
        );

        let summary: Vec<_> = diffs.iter().map(|diff| (diff.diff_type.as_str(), diff.name.as_str())).collect();
        assert_eq!(
            summary,
            vec![
                ("modified", "orders_user_id_fk"),
                ("added", "orders_account_id_fk"),
                ("removed", "orders_region_id_fk"),
            ]
        );
    }

    #[test]
    fn mysql_column_comment_changes_generate_modify_column_sql() {
        let diffs = vec![TableDiff {
            diff_type: "modified".to_string(),
            object_type: None,
            name: "users".to_string(),
            target_name: None,
            columns: Some(vec![ColumnDiff {
                diff_type: "modified".to_string(),
                name: "name".to_string(),
                source: Some(column("name", "varchar(64)", Some("用户姓名"))),
                target: Some(column("name", "varchar(64)", Some("Name"))),
                changes: vec!["comment: Name → 用户姓名".to_string()],
                add_position: None,
            }]),
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: None,
            source_table_comment: Some(Some("用户表".to_string())),
            target_table_comment: Some(Some("Users".to_string())),
            sync_sql: None,
        }];

        assert_eq!(
            generate_schema_sync_sql(&diffs, &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]),
            [
                "-- Alter table: users",
                "ALTER TABLE `users`",
                "  MODIFY COLUMN `name` varchar(64) NOT NULL COMMENT '用户姓名';",
                "",
                "ALTER TABLE `users` COMMENT = '用户表';",
            ]
            .join("\n")
        );
    }

    #[test]
    fn mysql_schema_sync_sql_qualifies_tables_with_target_database() {
        let diffs = vec![TableDiff {
            diff_type: "modified".to_string(),
            object_type: None,
            name: "notify_channel_config".to_string(),
            target_name: None,
            columns: Some(vec![ColumnDiff {
                diff_type: "modified".to_string(),
                name: "config_json".to_string(),
                source: Some(column("config_json", "json", Some("渠道配置"))),
                target: Some(column("config_json", "json", Some("Config"))),
                changes: vec!["comment: Config → 渠道配置".to_string()],
                add_position: None,
            }]),
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: None,
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        }];

        assert_eq!(
            generate_schema_sync_sql(
                &diffs,
                &[],
                &[],
                &[],
                &[],
                DatabaseType::Mysql,
                Some("target_db"),
                false,
                None,
                &[]
            ),
            [
                "-- Alter table: notify_channel_config",
                "ALTER TABLE `target_db`.`notify_channel_config`",
                "  MODIFY COLUMN `config_json` json NOT NULL COMMENT '渠道配置';",
            ]
            .join("\n")
        );
    }

    #[test]
    fn blank_target_schema_does_not_generate_empty_qualifier() {
        let diffs = vec![TableDiff {
            diff_type: "modified".to_string(),
            object_type: None,
            name: "notify_channel_config".to_string(),
            target_name: None,
            columns: Some(vec![ColumnDiff {
                diff_type: "modified".to_string(),
                name: "config_json".to_string(),
                source: Some(column("config_json", "json", Some("渠道配置"))),
                target: Some(column("config_json", "json", Some("Config"))),
                changes: vec!["comment: Config → 渠道配置".to_string()],
                add_position: None,
            }]),
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: None,
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        }];

        let sql =
            generate_schema_sync_sql(&diffs, &[], &[], &[], &[], DatabaseType::Mysql, Some("  "), false, None, &[]);

        assert!(sql.contains("ALTER TABLE `notify_channel_config`"));
        assert!(!sql.contains("``."));
    }

    #[test]
    fn ignore_comments_skips_column_and_table_comment_diffs() {
        let options = SchemaDiffPreparationOptions {
            source_tables: vec![TableInfo {
                name: "users".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: Some("用户表".to_string()),
                parent_schema: None,
                parent_name: None,
            }],
            target_tables: vec![TableInfo {
                name: "users".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: Some("Users".to_string()),
                parent_schema: None,
                parent_name: None,
            }],
            source_details: vec![TableSchemaDetail {
                name: "users".to_string(),
                columns: vec![column("name", "varchar(64)", Some("用户姓名"))],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
                triggers: Vec::new(),
                ddl: None,
            }],
            target_details: vec![TableSchemaDetail {
                name: "users".to_string(),
                columns: vec![column("name", "varchar(64)", Some("Name"))],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
                triggers: Vec::new(),
                ddl: None,
            }],
            source_functions: Vec::new(),
            target_functions: Vec::new(),
            source_sequences: Vec::new(),
            target_sequences: Vec::new(),
            source_rules: Vec::new(),
            target_rules: Vec::new(),
            source_owners: Vec::new(),
            target_owners: Vec::new(),
            database_type: DatabaseType::Mysql,
            target_schema: None,
            ignore_comments: true,
            cascade_delete: false,
            compare_column_order: false,
            ..Default::default()
        };

        let result = prepare_schema_diff(options);
        assert!(result.diffs.is_empty());
        assert!(result.sync_sql.is_empty());
    }

    #[test]
    fn prepare_schema_diff_attaches_per_table_sync_sql() {
        let options = SchemaDiffPreparationOptions {
            source_tables: vec![TableInfo {
                name: "users".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            }],
            target_tables: vec![TableInfo {
                name: "users".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            }],
            source_details: vec![TableSchemaDetail {
                name: "users".to_string(),
                columns: vec![column("name", "varchar(128)", None)],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
                triggers: Vec::new(),
                ddl: Some("CREATE TABLE `users` (`name` varchar(128));".to_string()),
            }],
            target_details: vec![TableSchemaDetail {
                name: "users".to_string(),
                columns: vec![column("name", "varchar(64)", None)],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
                triggers: Vec::new(),
                ddl: Some("CREATE TABLE `users` (`name` varchar(64));".to_string()),
            }],
            source_functions: Vec::new(),
            target_functions: Vec::new(),
            source_sequences: Vec::new(),
            target_sequences: Vec::new(),
            source_rules: Vec::new(),
            target_rules: Vec::new(),
            source_owners: Vec::new(),
            target_owners: Vec::new(),
            database_type: DatabaseType::Mysql,
            target_schema: None,
            ignore_comments: false,
            cascade_delete: false,
            compare_column_order: false,
            ..Default::default()
        };

        let result = prepare_schema_diff(options);
        let table_sync_sql = result.diffs[0].sync_sql.as_deref().unwrap_or_default();

        assert!(table_sync_sql.contains("ALTER TABLE `users`"));
        assert!(!table_sync_sql.contains("CREATE TABLE"));
    }

    #[test]
    fn schema_sync_plan_builds_matching_forward_and_rollback_sql_for_selected_children() {
        let selected_diff = TableDiff {
            diff_type: "modified".to_string(),
            object_type: Some("table".to_string()),
            name: "users".to_string(),
            target_name: None,
            columns: Some(vec![ColumnDiff {
                diff_type: "added".to_string(),
                name: "nickname".to_string(),
                source: Some(column("nickname", "varchar(64)", None)),
                target: None,
                changes: Vec::new(),
                add_position: None,
            }]),
            ..Default::default()
        };

        let plan = generate_schema_sync_sql_plan(
            &[selected_diff],
            &[],
            &[],
            &[],
            &[],
            DatabaseType::Mysql,
            Some("shop"),
            false,
            None,
            &[],
            true,
        );

        assert!(plan.sync_sql.contains("ADD COLUMN `nickname`"), "{}", plan.sync_sql);
        let rollback = plan.rollback_sync_sql.expect("rollback SQL");
        assert!(rollback.contains("DROP COLUMN `nickname`"), "{rollback}");
        assert_eq!(plan.rollback_completeness, RollbackCompleteness::Complete);
    }

    #[test]
    fn added_table_unqualified_native_ddl_is_left_unchanged() {
        // MySQL-family DDL that relies on the connection's current database
        // (no schema/database qualifier) already resolves correctly wherever
        // the sync script runs, so it must not be rewritten.
        let diffs = vec![TableDiff {
            diff_type: "added".to_string(),
            object_type: Some("table".to_string()),
            name: "orders".to_string(),
            ddl: Some("CREATE TABLE `orders` (\n  `id` int\n)".to_string()),
            ..TableDiff::default()
        }];

        let sql = generate_schema_sync_sql(
            &diffs,
            &[],
            &[],
            &[],
            &[],
            DatabaseType::Mysql,
            Some("shop"),
            false,
            Some(DialectKind::Mysql),
            &[],
        );

        assert!(sql.contains("CREATE TABLE `orders`"), "{sql}");
    }

    // ========================================================================
    // Phase 4.1: Dependency Graph Tests
    // ========================================================================

    #[test]
    fn dependency_graph_builds_dag_from_foreign_keys() {
        let details = vec![
            TableSchemaDetail {
                name: "orders".to_string(),
                columns: vec![],
                indexes: vec![],
                foreign_keys: vec![ForeignKeyInfo {
                    name: "fk_orders_users".to_string(),
                    column: "user_id".to_string(),
                    ref_schema: None,
                    ref_table: "users".to_string(),
                    ref_column: "id".to_string(),
                    on_update: None,
                    on_delete: None,
                }],
                triggers: vec![],
                ddl: None,
            },
            TableSchemaDetail {
                name: "users".to_string(),
                columns: vec![],
                indexes: vec![],
                foreign_keys: vec![],
                triggers: vec![],
                ddl: None,
            },
        ];
        let tables = vec![
            TableInfo {
                name: "orders".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            },
            TableInfo {
                name: "users".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            },
        ];

        let graph = DependencyGraph::build(&details, &tables);
        assert_eq!(graph.nodes.len(), 2);
        assert!(graph.nodes["orders"].depends_on.contains(&"users".to_string()));
        assert_eq!(graph.nodes["orders"].depends_on.len(), 1);
        assert_eq!(graph.nodes["users"].depends_on.len(), 0);
    }

    #[test]
    fn dependency_graph_topological_sort_drop_order() {
        let details = vec![
            TableSchemaDetail {
                name: "order_items".to_string(),
                columns: vec![],
                indexes: vec![],
                foreign_keys: vec![ForeignKeyInfo {
                    name: "fk_items_orders".to_string(),
                    column: "order_id".to_string(),
                    ref_schema: None,
                    ref_table: "orders".to_string(),
                    ref_column: "id".to_string(),
                    on_update: None,
                    on_delete: None,
                }],
                triggers: vec![],
                ddl: None,
            },
            TableSchemaDetail {
                name: "orders".to_string(),
                columns: vec![],
                indexes: vec![],
                foreign_keys: vec![ForeignKeyInfo {
                    name: "fk_orders_users".to_string(),
                    column: "user_id".to_string(),
                    ref_schema: None,
                    ref_table: "users".to_string(),
                    ref_column: "id".to_string(),
                    on_update: None,
                    on_delete: None,
                }],
                triggers: vec![],
                ddl: None,
            },
            TableSchemaDetail {
                name: "users".to_string(),
                columns: vec![],
                indexes: vec![],
                foreign_keys: vec![],
                triggers: vec![],
                ddl: None,
            },
        ];
        let tables = vec![
            TableInfo {
                name: "order_items".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            },
            TableInfo {
                name: "orders".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            },
            TableInfo {
                name: "users".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            },
        ];

        let graph = DependencyGraph::build(&details, &tables);
        let drop_order = graph.drop_order();

        let di = drop_order.iter().position(|n| n == "order_items").unwrap();
        let oi = drop_order.iter().position(|n| n == "orders").unwrap();
        assert!(di < oi, "order_items should be dropped before orders");
    }

    #[test]
    fn coverage_score_empty_graph_returns_one() {
        let graph = DependencyGraph { nodes: HashMap::new(), topological_order: vec![] };
        assert_eq!(graph.coverage_score(&[]), 1.0);
    }

    #[test]
    fn coverage_score_partial_coverage() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            DependencyNode { table_name: "a".to_string(), depends_on: vec!["b".to_string()], depended_by: vec![] },
        );
        nodes.insert(
            "b".to_string(),
            DependencyNode {
                table_name: "b".to_string(),
                depends_on: vec!["c".to_string()],
                depended_by: vec!["a".to_string()],
            },
        );
        nodes.insert(
            "c".to_string(),
            DependencyNode { table_name: "c".to_string(), depends_on: vec![], depended_by: vec!["b".to_string()] },
        );
        let graph =
            DependencyGraph { nodes, topological_order: vec!["c".to_string(), "b".to_string(), "a".to_string()] };

        let score = graph.coverage_score(&["a".to_string(), "b".to_string()]);
        assert!((score - 0.5).abs() < 0.01);
    }

    #[test]
    fn coverage_score_level2_transitive_edges() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            DependencyNode { table_name: "a".to_string(), depends_on: vec!["b".to_string()], depended_by: vec![] },
        );
        nodes.insert(
            "b".to_string(),
            DependencyNode {
                table_name: "b".to_string(),
                depends_on: vec!["c".to_string()],
                depended_by: vec!["a".to_string()],
            },
        );
        nodes.insert(
            "c".to_string(),
            DependencyNode { table_name: "c".to_string(), depends_on: vec![], depended_by: vec!["b".to_string()] },
        );
        let graph =
            DependencyGraph { nodes, topological_order: vec!["c".to_string(), "b".to_string(), "a".to_string()] };

        let l2_score = graph.coverage_score_level2(&["a".to_string(), "b".to_string(), "c".to_string()]);
        assert!((l2_score - 1.0).abs() < 0.01, "full coverage should give 1.0");
    }

    #[test]
    fn coverage_score_level2_partial() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            DependencyNode { table_name: "a".to_string(), depends_on: vec!["b".to_string()], depended_by: vec![] },
        );
        nodes.insert(
            "b".to_string(),
            DependencyNode {
                table_name: "b".to_string(),
                depends_on: vec!["c".to_string()],
                depended_by: vec!["a".to_string()],
            },
        );
        nodes.insert(
            "c".to_string(),
            DependencyNode { table_name: "c".to_string(), depends_on: vec![], depended_by: vec!["b".to_string()] },
        );
        let graph =
            DependencyGraph { nodes, topological_order: vec!["c".to_string(), "b".to_string(), "a".to_string()] };

        let l2_score = graph.coverage_score_level2(&["a".to_string(), "b".to_string()]);
        assert!((l2_score - 0.0).abs() < 0.01, "missing grandparent c means 0 transitive coverage");
    }

    #[test]
    fn composite_coverage_full_coverage() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            DependencyNode { table_name: "a".to_string(), depends_on: vec!["b".to_string()], depended_by: vec![] },
        );
        nodes.insert(
            "b".to_string(),
            DependencyNode {
                table_name: "b".to_string(),
                depends_on: vec!["c".to_string()],
                depended_by: vec!["a".to_string()],
            },
        );
        nodes.insert(
            "c".to_string(),
            DependencyNode { table_name: "c".to_string(), depends_on: vec![], depended_by: vec!["b".to_string()] },
        );
        let graph =
            DependencyGraph { nodes, topological_order: vec!["c".to_string(), "b".to_string(), "a".to_string()] };

        let report = graph.composite_coverage_score(&["a".to_string(), "b".to_string(), "c".to_string()]);
        assert!((report.level1_score - 1.0).abs() < 0.01);
        assert!((report.level2_score - 1.0).abs() < 0.01);
        assert!((report.composite_score - 1.0).abs() < 0.01);
        assert_eq!(report.level1_covered, 2);
        assert_eq!(report.level1_total, 2);
        assert_eq!(report.level2_covered, 1);
        assert_eq!(report.level2_total, 1);
    }

    #[test]
    fn composite_coverage_partial() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "a".to_string(),
            DependencyNode { table_name: "a".to_string(), depends_on: vec!["b".to_string()], depended_by: vec![] },
        );
        nodes.insert(
            "b".to_string(),
            DependencyNode {
                table_name: "b".to_string(),
                depends_on: vec!["c".to_string()],
                depended_by: vec!["a".to_string()],
            },
        );
        nodes.insert(
            "c".to_string(),
            DependencyNode { table_name: "c".to_string(), depends_on: vec![], depended_by: vec!["b".to_string()] },
        );
        let graph =
            DependencyGraph { nodes, topological_order: vec!["c".to_string(), "b".to_string(), "a".to_string()] };

        let report = graph.composite_coverage_score(&["a".to_string(), "b".to_string()]);
        assert!((report.level1_score - 0.5).abs() < 0.01);
        assert!((report.level2_score - 0.0).abs() < 0.01);
        assert!((report.composite_score - 0.3).abs() < 0.01, "0.6*0.5 + 0.4*0.0 = 0.3");
        assert_eq!(report.level1_covered, 1);
        assert_eq!(report.level1_total, 2);
        assert!(!report.uncovered_edges.is_empty());
    }

    #[test]
    fn composite_coverage_no_dependencies() {
        let mut nodes = HashMap::new();
        nodes.insert(
            "t1".to_string(),
            DependencyNode { table_name: "t1".to_string(), depends_on: vec![], depended_by: vec![] },
        );
        nodes.insert(
            "t2".to_string(),
            DependencyNode { table_name: "t2".to_string(), depends_on: vec![], depended_by: vec![] },
        );
        let graph = DependencyGraph { nodes, topological_order: vec!["t1".to_string(), "t2".to_string()] };

        let report = graph.composite_coverage_score(&["t1".to_string()]);
        assert!((report.level1_score - 1.0).abs() < 0.01);
        assert!((report.level2_score - 1.0).abs() < 0.01);
        assert!((report.composite_score - 1.0).abs() < 0.01);
        assert!(report.uncovered_edges.is_empty());
    }

    // ========================================================================
    // Phase 4.1: Rename Detection Tests
    // ========================================================================

    #[test]
    fn detect_renames_high_similarity_columns() {
        let source_details = vec![TableSchemaDetail {
            name: "users_old".to_string(),
            columns: vec![
                column("id", "int", None),
                column("name", "varchar(100)", None),
                column("email", "varchar(255)", None),
                column("created_at", "datetime", None),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            triggers: vec![],
            ddl: None,
        }];
        let target_details = vec![TableSchemaDetail {
            name: "users_new".to_string(),
            columns: vec![
                column("id", "integer", None),
                column("name", "varchar(100)", None),
                column("email", "varchar(255)", None),
                column("updated_at", "datetime", None),
            ],
            indexes: vec![],
            foreign_keys: vec![],
            triggers: vec![],
            ddl: None,
        }];

        let candidates = detect_renames(
            &["users_new".to_string()],
            &["users_old".to_string()],
            &source_details,
            &target_details,
            0.5,
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].source_name, "users_old");
        assert_eq!(candidates[0].target_name, "users_new");
        assert!(candidates[0].score >= 0.5);
    }

    #[test]
    fn detect_renames_low_similarity_below_threshold() {
        let source_details = vec![TableSchemaDetail {
            name: "users".to_string(),
            columns: vec![column("id", "int", None)],
            indexes: vec![],
            foreign_keys: vec![],
            triggers: vec![],
            ddl: None,
        }];
        let target_details = vec![TableSchemaDetail {
            name: "products".to_string(),
            columns: vec![column("sku", "varchar(50)", None), column("price", "decimal", None)],
            indexes: vec![],
            foreign_keys: vec![],
            triggers: vec![],
            ddl: None,
        }];

        let candidates =
            detect_renames(&["users".to_string()], &["products".to_string()], &source_details, &target_details, 0.5);

        assert!(candidates.is_empty());
    }

    #[test]
    fn jaccard_similarity_identical_sets() {
        let a: HashSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
        assert!((jaccard_similarity(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    // ========================================================================
    // Phase 4.2: Batch Naming Pattern Tests
    // ========================================================================

    #[test]
    fn batch_pattern_matching_wildcard() {
        let source = vec!["log_2024_01".to_string(), "log_2024_02".to_string(), "users".to_string()];
        let target = vec!["log_2024_03".to_string()];
        let patterns = vec![BatchPattern {
            pattern: "log_*".to_string(),
            is_regex: false,
            description: "all log tables".to_string(),
        }];

        let (_added, removed, common, match_results) = diff_names_with_patterns(&source, &target, &patterns);
        assert_eq!(removed, vec!["log_2024_03"]);
        assert_eq!(common.len(), 0);
        assert_eq!(match_results.len(), 1);
        assert_eq!(match_results[0].len(), 2);
    }

    #[test]
    fn batch_pattern_regex_matching() {
        let source = vec!["tbl_001".to_string(), "tbl_002".to_string(), "other".to_string()];
        let target = vec![];
        let patterns = vec![BatchPattern {
            pattern: r"tbl_\d{3}".to_string(),
            is_regex: true,
            description: "numbered tables".to_string(),
        }];

        let (_added, _removed, _common, match_results) = diff_names_with_patterns(&source, &target, &patterns);
        assert_eq!(match_results[0].len(), 2);
    }

    #[test]
    fn pattern_conflict_detection() {
        let patterns = vec![
            BatchPattern { pattern: "user_*".to_string(), is_regex: false, description: "user tables".to_string() },
            BatchPattern {
                pattern: "user_data".to_string(),
                is_regex: false,
                description: "specific user data".to_string(),
            },
        ];

        let names = vec!["user_data".to_string(), "user_log".to_string()];
        let conflicts = detect_pattern_conflicts(&patterns, &names);
        assert!(!conflicts.is_empty());
    }

    // ========================================================================
    // Phase 4.3: Type Compatibility Tests
    // ========================================================================

    #[test]
    fn diff_columns_with_compatibility_integer_family() {
        let (_diffs, warnings) = diff_columns_with_compatibility(
            &[column("id", "INT", None)],
            &[column("id", "BIGINT", None)],
            false,
            false,
            DialectKind::Mysql,
            DialectKind::Mysql,
            0.9,
            &[],
        );
        assert!(!warnings.is_empty());
        assert_eq!(warnings[0].risk, ColumnConversionRisk::Low);
    }

    #[test]
    fn diff_columns_with_compatibility_exact_match_no_warning() {
        let (_diffs, warnings) = diff_columns_with_compatibility(
            &[column("id", "INT", None)],
            &[column("id", "INT", None)],
            false,
            false,
            DialectKind::Mysql,
            DialectKind::Mysql,
            0.5,
            &[],
        );
        assert!(warnings.is_empty());
    }

    // ========================================================================
    // Phase 4.4: Bidirectional Diff & Rollback Tests
    // ========================================================================

    fn make_diff(diff_type: &str, name: &str) -> TableDiff {
        TableDiff {
            diff_type: diff_type.to_string(),
            object_type: Some("table".to_string()),
            name: name.to_string(),
            target_name: None,
            columns: None,
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: None,
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        }
    }

    #[test]
    fn rollback_graph_add_becomes_drop() {
        let diffs = vec![make_diff("added", "new_table")];
        let dep_graph = DependencyGraph { nodes: HashMap::new(), topological_order: vec![] };
        let graph = RollbackGraph::from_forward_diffs(&diffs, &[], &dep_graph);

        assert_eq!(graph.forward_nodes.len(), 1);
        assert_eq!(graph.rollback_nodes.len(), 1);
        assert_eq!(graph.forward_nodes[0].table_diff.diff_type, "added");
        assert_eq!(graph.rollback_nodes[0].table_diff.diff_type, "removed");
    }

    #[test]
    fn rollback_graph_remove_becomes_add() {
        let diffs = vec![make_diff("removed", "old_table")];
        let dep_graph = DependencyGraph { nodes: HashMap::new(), topological_order: vec![] };
        let graph = RollbackGraph::from_forward_diffs(&diffs, &[], &dep_graph);

        assert_eq!(graph.rollback_nodes[0].table_diff.diff_type, "added");
        assert_eq!(graph.rollback_nodes[0].table_diff.ddl, None);
    }

    fn rollback_removed_table_sql(
        database_type: DatabaseType,
        target_schema: Option<&str>,
        source_dialect: Option<DialectKind>,
        target_dialect: DialectKind,
        table_name: &str,
        table_comment: Option<&str>,
        target_detail: TableSchemaDetail,
    ) -> SchemaDiffPreparation {
        prepare_schema_diff(SchemaDiffPreparationOptions {
            target_tables: vec![TableInfo {
                name: table_name.to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: table_comment.map(str::to_string),
                parent_schema: None,
                parent_name: None,
            }],
            target_details: vec![target_detail],
            database_type,
            target_schema: target_schema.map(str::to_string),
            enable_rollback: true,
            source_dialect,
            target_dialect: Some(target_dialect),
            ..Default::default()
        })
    }

    #[test]
    fn dropped_mysql_table_rollback_preserves_defaults_comments_indexes_and_fk() {
        let table_name = "order-items";
        let result = rollback_removed_table_sql(
            DatabaseType::Mysql,
            Some("shop"),
            Some(DialectKind::Mysql),
            DialectKind::Mysql,
            table_name,
            Some("order item history"),
            TableSchemaDetail {
                name: table_name.to_string(),
                columns: vec![
                    ColumnInfo {
                        is_primary_key: true,
                        column_default: Some("(uuid())".to_string()),
                        ..column("item-id", "varchar(36)", Some("stable item id"))
                    },
                    ColumnInfo {
                        column_default: Some("'new'".to_string()),
                        ..column("status", "varchar(32)", Some("workflow state"))
                    },
                    column("user-id", "bigint", None),
                ],
                indexes: vec![index(IndexInfo {
                    name: "status-index".to_string(),
                    columns: vec!["status".to_string()],
                    is_unique: true,
                    is_primary: false,
                    filter: None,
                    index_type: Some("BTREE".to_string()),
                    included_columns: None,
                    comment: Some("status lookup".to_string()),
                    key_is_expression: Vec::new(),
                    column_opclasses: vec![],
                    key_options: Vec::new(),
                    constraint_backed: false,
                })],
                foreign_keys: vec![foreign_key(ForeignKeyInfo {
                    name: "user-fk".to_string(),
                    column: "user-id".to_string(),
                    ref_schema: Some("identity".to_string()),
                    ref_table: "users".to_string(),
                    ref_column: "id".to_string(),
                    on_update: Some("CASCADE".to_string()),
                    on_delete: Some("RESTRICT".to_string()),
                })],
                triggers: vec![],
                ddl: Some("CREATE TABLE native_mysql_fallback (ignored int)".to_string()),
            },
        );
        let rollback = result.rollback_sync_sql.unwrap();

        assert!(rollback.contains("CREATE TABLE `shop`.`order-items`"), "{rollback}");
        assert!(rollback.contains("`item-id` varchar(36) NOT NULL DEFAULT (uuid()) COMMENT 'stable item id'"));
        assert!(rollback.contains("PRIMARY KEY (`item-id`)"), "{rollback}");
        assert!(rollback.contains("CREATE UNIQUE INDEX `status-index` USING BTREE ON `shop`.`order-items` (`status`)"));
        assert!(rollback.contains("COMMENT 'status lookup'"), "{rollback}");
        assert!(rollback.contains("REFERENCES `identity`.`users`(`id`) ON DELETE RESTRICT ON UPDATE CASCADE"));
        assert!(rollback.contains("ALTER TABLE `shop`.`order-items` COMMENT = 'order item history'"));
        assert!(!rollback.contains("native_mysql_fallback"), "{rollback}");
    }

    #[test]
    fn dropped_table_incomplete_trigger_sets_structured_missing_objects() {
        let result = rollback_removed_table_sql(
            DatabaseType::Mysql,
            None,
            Some(DialectKind::Mysql),
            DialectKind::Mysql,
            "orders",
            None,
            TableSchemaDetail {
                name: "orders".to_string(),
                columns: vec![column("id", "int", None)],
                indexes: vec![],
                foreign_keys: vec![],
                triggers: vec![crate::types::TriggerInfo {
                    name: "trg_orders".to_string(),
                    event: "INSERT".to_string(),
                    timing: "AFTER".to_string(),
                    level: None,
                    condition: None,
                    language: None,
                    enabled: None,
                    valid: None,
                    comment: None,
                    created_at: None,
                    statement: None,
                }],
                ddl: None,
            },
        );

        assert_eq!(result.rollback_completeness, RollbackCompleteness::Incomplete);
        assert!(!result.missing_rollback_objects.is_empty());
        assert_eq!(result.missing_rollback_objects[0].kind, "trigger");
        assert_eq!(result.missing_rollback_objects[0].name, "trg_orders");
        assert!(result.missing_rollback_objects[0].table.as_deref() == Some("orders"));
    }

    #[test]
    fn rollback_graph_modified_stays_modified_swapped() {
        let source_col = column("name", "varchar(100)", None);
        let target_col = column("name", "varchar(50)", None);
        let diffs = vec![TableDiff {
            diff_type: "modified".to_string(),
            object_type: Some("table".to_string()),
            name: "users".to_string(),
            target_name: None,
            columns: Some(vec![ColumnDiff {
                diff_type: "modified".to_string(),
                name: "name".to_string(),
                source: Some(source_col.clone()),
                target: Some(target_col.clone()),
                changes: vec!["type: varchar(50) → varchar(100)".to_string()],
                add_position: None,
            }]),
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: None,
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        }];

        let dep_graph = DependencyGraph { nodes: HashMap::new(), topological_order: vec![] };
        let graph = RollbackGraph::from_forward_diffs(&diffs, &[], &dep_graph);

        let rollback = &graph.rollback_nodes[0];
        assert_eq!(rollback.table_diff.diff_type, "modified");
        let rb_cols = rollback.table_diff.columns.as_ref().unwrap();
        assert_eq!(rb_cols[0].diff_type, "modified");
        assert_eq!(rb_cols[0].source.as_ref().unwrap().data_type, "varchar(50)");
        assert_eq!(rb_cols[0].target.as_ref().unwrap().data_type, "varchar(100)");
        assert_eq!(rb_cols[0].changes, vec!["type: varchar(100) → varchar(50)"]);
    }

    #[test]
    fn rollback_consistency_validation() {
        let diffs = vec![make_diff("added", "t1"), make_diff("removed", "t2")];
        let dep_graph = DependencyGraph { nodes: HashMap::new(), topological_order: vec![] };
        let mut graph = RollbackGraph::from_forward_diffs(&diffs, &[], &dep_graph);
        assert!(graph.validate_consistency());
        assert!(graph.consistency_issues.is_empty());
    }

    // ========================================================================
    // Phase 4.6: Permission Tests
    // ========================================================================

    #[test]
    fn diff_permissions_detects_added_and_removed() {
        let source = vec![PermissionInfo {
            grantee: "app_user".to_string(),
            object_type: "TABLE".to_string(),
            object_name: "orders".to_string(),
            privilege: "SELECT".to_string(),
            is_grantable: false,
        }];
        let target = vec![PermissionInfo {
            grantee: "app_user".to_string(),
            object_type: "TABLE".to_string(),
            object_name: "orders".to_string(),
            privilege: "INSERT".to_string(),
            is_grantable: false,
        }];

        let diffs = diff_permissions(&source, &target);
        assert_eq!(diffs.len(), 2);
        assert!(diffs.iter().any(|d| d.diff_type == "added"));
        assert!(diffs.iter().any(|d| d.diff_type == "removed"));
    }

    #[test]
    fn generate_permission_sql_mysql() {
        let diffs = vec![PermissionDiff {
            diff_type: "added".to_string(),
            grantee: "app_user".to_string(),
            object_name: "orders".to_string(),
            privilege: "SELECT".to_string(),
            source: Some(PermissionInfo {
                grantee: "app_user".to_string(),
                object_type: "TABLE".to_string(),
                object_name: "orders".to_string(),
                privilege: "SELECT".to_string(),
                is_grantable: true,
            }),
            target: None,
        }];

        let sql = generate_permission_sync_sql(&diffs, DatabaseType::Mysql, Some("mydb"));
        assert!(sql.contains("GRANT SELECT ON `mydb`.`orders` TO 'app_user' WITH GRANT OPTION"));
    }

    // ========================================================================
    // Phase 4.7: Resource Scheduling Tests
    // ========================================================================

    #[test]
    fn adaptive_scheduler_optimal_batch_size() {
        let constraint = ResourceConstraint::default();
        let scheduler = AdaptiveScheduler::new(constraint, 400);
        let batch = scheduler.optimal_batch_size();
        assert!(batch > 0);
        assert!(batch <= 50);
    }

    #[test]
    fn adaptive_scheduler_shard_count() {
        let constraint = ResourceConstraint::default();
        let scheduler = AdaptiveScheduler::new(constraint, 200);
        let count = scheduler.recommended_shard_count();
        assert!(count >= 1);
        assert!(count <= 4);
    }

    // ========================================================================
    // Phase 4.8: Backward Compatibility Tests
    // ========================================================================

    #[test]
    fn new_options_default_values_do_not_affect_basic_diff() {
        let options = SchemaDiffPreparationOptions::default();
        let result = prepare_schema_diff(options);
        assert!(result.diffs.is_empty());
        assert!(result.sync_sql.is_empty());
        assert!(result.rollback_sync_sql.is_none());
        assert!(result.rename_candidates.is_empty());
        assert!(result.rollback_graph.is_none());
        assert!(result.compatibility_warnings.is_empty());
        assert!(result.permission_diffs.is_empty());
    }

    #[test]
    fn prepare_schema_diff_with_rename_detection() {
        let options = SchemaDiffPreparationOptions {
            source_tables: vec![TableInfo {
                name: "users_old".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            }],
            target_tables: vec![TableInfo {
                name: "users_new".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            }],
            source_details: vec![TableSchemaDetail {
                name: "users_old".to_string(),
                columns: vec![column("id", "int", None), column("name", "varchar(100)", None)],
                indexes: vec![],
                foreign_keys: vec![],
                triggers: vec![],
                ddl: None,
            }],
            target_details: vec![TableSchemaDetail {
                name: "users_new".to_string(),
                columns: vec![column("id", "int", None), column("name", "varchar(100)", None)],
                indexes: vec![],
                foreign_keys: vec![],
                triggers: vec![],
                ddl: None,
            }],
            source_functions: vec![],
            target_functions: vec![],
            source_sequences: vec![],
            target_sequences: vec![],
            source_rules: vec![],
            target_rules: vec![],
            source_owners: vec![],
            target_owners: vec![],
            database_type: DatabaseType::Mysql,
            target_schema: None,
            ignore_comments: false,
            cascade_delete: false,
            compare_column_order: false,
            detect_renames: true,
            detect_table_renames: true,
            rename_threshold: 0.5,
            ..Default::default()
        };

        let result = prepare_schema_diff(options);
        assert!(!result.rename_candidates.is_empty());
        assert!(result.rename_candidates[0].score >= 0.5);
    }

    #[test]
    fn prepare_schema_diff_with_rollback_generates_rollback_sql() {
        let options = SchemaDiffPreparationOptions {
            source_tables: vec![TableInfo {
                name: "new_table".to_string(),
                table_type: "BASE TABLE".to_string(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            }],
            target_tables: vec![],
            source_details: vec![TableSchemaDetail {
                name: "new_table".to_string(),
                columns: vec![column("id", "int", None)],
                indexes: vec![],
                foreign_keys: vec![],
                triggers: vec![],
                ddl: Some("CREATE TABLE new_table (id int);".to_string()),
            }],
            target_details: vec![],
            source_functions: vec![],
            target_functions: vec![],
            source_sequences: vec![],
            target_sequences: vec![],
            source_rules: vec![],
            target_rules: vec![],
            source_owners: vec![],
            target_owners: vec![],
            database_type: DatabaseType::Mysql,
            target_schema: None,
            ignore_comments: false,
            cascade_delete: false,
            compare_column_order: false,
            enable_rollback: true,
            ..Default::default()
        };

        let result = prepare_schema_diff(options);
        assert!(result.rollback_sync_sql.is_some());
        assert!(result.rollback_graph.is_some());
        let graph = result.rollback_graph.unwrap();
        assert!(graph.is_consistent);
        assert_eq!(graph.forward_nodes.len(), 1);
        assert_eq!(graph.rollback_nodes.len(), 1);
        assert_eq!(graph.rollback_nodes[0].table_diff.diff_type, "removed");
    }

    // -- 31. column_type_similarity_score unit tests --
    #[test]
    fn column_type_similarity_exact_match() {
        assert_eq!(column_type_similarity_score("int", "int"), 1.0);
        assert_eq!(column_type_similarity_score("VARCHAR(255)", "varchar(255)"), 1.0);
        assert_eq!(column_type_similarity_score("datetime", "datetime"), 1.0);
    }

    #[test]
    fn column_type_similarity_synonym() {
        assert_eq!(column_type_similarity_score("int", "integer"), 1.0);
        assert_eq!(column_type_similarity_score("boolean", "bool"), 1.0);
        assert_eq!(column_type_similarity_score("datetime", "timestamp"), 1.0);
        assert_eq!(column_type_similarity_score("double", "double precision"), 1.0);
    }

    #[test]
    fn column_type_similarity_family() {
        assert_eq!(column_type_similarity_score("tinyint", "bigint"), 0.8);
        assert_eq!(column_type_similarity_score("char", "text"), 0.8);
        assert_eq!(column_type_similarity_score("mediumtext", "clob"), 0.8);
    }

    #[test]
    fn column_type_similarity_unrelated() {
        assert_eq!(column_type_similarity_score("int", "varchar"), 0.0);
        assert_eq!(column_type_similarity_score("boolean", "text"), 0.0);
        assert_eq!(column_type_similarity_score("blob", "date"), 0.0);
    }

    #[test]
    fn column_type_similarity_parameterized_ignored() {
        assert_eq!(column_type_similarity_score("int(11)", "int(11)"), 1.0);
        assert_eq!(column_type_similarity_score("int(11)", "integer"), 1.0);
        assert_eq!(column_type_similarity_score("varchar(255)", "varchar(64)"), 1.0);
    }

    #[test]
    fn mysql_same_dialect_ignores_only_integer_display_widths() {
        let source = vec![
            column("id", "int(11) unsigned", None),
            column("status", "tinyint(4)", None),
            column("amount", "decimal(10,2)", None),
            column("name", "varchar(128)", None),
        ];
        let target = vec![
            column("id", "int unsigned", None),
            column("status", "tinyint", None),
            column("amount", "decimal(12,2)", None),
            column("name", "varchar(64)", None),
        ];

        let diffs = diff_columns_with_dialect_options(
            &source,
            &target,
            false,
            false,
            false,
            0.5,
            Some(DialectKind::Mysql),
            Some(DialectKind::Mysql),
        );

        assert_eq!(diffs.iter().map(|diff| diff.name.as_str()).collect::<Vec<_>>(), vec!["amount", "name"]);
        assert!(diffs.iter().all(|diff| diff.changes.iter().any(|change| change.starts_with("type:"))));
    }

    #[test]
    fn mysql_modify_column_preserves_explicit_auto_increment() {
        let mut source = column("id", "int", Some("new comment"));
        source.is_primary_key = true;
        source.extra = Some("auto_increment".to_string());
        let mut target = source.clone();
        target.comment = Some("old comment".to_string());
        let diff = ColumnDiff {
            diff_type: "modified".to_string(),
            name: "id".to_string(),
            source: Some(source),
            target: Some(target),
            changes: vec!["comment: old comment → new comment".to_string()],
            add_position: None,
        };

        let sql = gen_sql(wrap_table_diff("users", vec![diff]), DatabaseType::Mysql, Some(DialectKind::Mysql));

        assert!(
            sql.contains("MODIFY COLUMN `id` int NOT NULL AUTO_INCREMENT COMMENT 'new comment'"),
            "MySQL MODIFY must preserve AUTO_INCREMENT: {sql}"
        );
    }

    #[test]
    fn mysql_add_column_keeps_auto_increment_suffix() {
        let mut source = column("seq", "int", None);
        source.extra = Some("auto_increment".to_string());
        let diff = ColumnDiff {
            diff_type: "added".to_string(),
            name: "seq".to_string(),
            source: Some(source),
            target: None,
            changes: vec![],
            add_position: None,
        };

        let sql = gen_sql(wrap_table_diff("users", vec![diff]), DatabaseType::Mysql, Some(DialectKind::Mysql));

        assert!(sql.contains("AUTO_INCREMENT"), "MySQL ADD COLUMN must keep AUTO_INCREMENT: {sql}");
    }

    // -- 32. Multiple renames in one table --
    #[test]
    fn multiple_renames_in_one_table() {
        let diffs = make_col_diffs(
            &[("id", "int"), ("new_a", "varchar(50)"), ("new_b", "int")],
            &[("id", "int"), ("old_a", "varchar(50)"), ("old_b", "int")],
            true,
        );
        let renamed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "renamed").collect();
        assert_eq!(renamed.len(), 2, "should detect two renames: {renamed:?}");
        assert_eq!(renamed[0].name, "new_a");
        assert_eq!(renamed[1].name, "new_b");
    }

    // -- 33. Rename threshold edge cases --
    #[test]
    fn rename_threshold_zero_detects_all() {
        let s: Vec<ColumnInfo> = vec![column("a", "int", None), column("b2", "varchar(10)", None)];
        let t: Vec<ColumnInfo> = vec![column("a", "int", None), column("b1", "varchar(10)", None)];
        // rename detection is skipped when threshold <= 0.0, use a tiny threshold
        let diffs = diff_columns_with_options(&s, &t, false, false, true, 0.001);
        let renamed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "renamed").collect();
        assert_eq!(renamed.len(), 1, "threshold near-zero should detect: {renamed:?}");
    }

    #[test]
    fn rename_threshold_one_detects_exact_only() {
        let s: Vec<ColumnInfo> = vec![column("a", "varchar(10)", None), column("b2", "text", None)];
        let t: Vec<ColumnInfo> = vec![column("a", "varchar(10)", None), column("b1", "varchar(10)", None)];
        let diffs = diff_columns_with_options(&s, &t, false, false, true, 1.0);
        let renamed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "renamed").collect();
        assert_eq!(renamed.len(), 0, "threshold 1 should not match text≠varchar: {renamed:?}");
    }

    #[test]
    fn rename_threshold_mid_detects_family_only() {
        let s: Vec<ColumnInfo> = vec![column("a", "tinyint", None), column("b2", "int", None)];
        let t: Vec<ColumnInfo> = vec![column("a", "tinyint", None), column("b1", "bigint", None)];
        let diffs = diff_columns_with_options(&s, &t, false, false, true, 0.9);
        let renamed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "renamed").collect();
        assert_eq!(renamed.len(), 0, "threshold 0.9 should not match tinyint≠bigint: {renamed:?}");
        let diffs2 = diff_columns_with_options(&s, &t, false, false, true, 0.5);
        let renamed2: Vec<_> = diffs2.iter().filter(|d| d.diff_type == "renamed").collect();
        assert_eq!(renamed2.len(), 1, "threshold 0.5 should detect integer family: {renamed2:?}");
    }

    // -- 34. Default value changes --
    #[test]
    fn default_value_change_mysql() {
        let source = vec![ColumnInfo { column_default: Some("'guest'".into()), ..column("name", "varchar(50)", None) }];
        let target = vec![ColumnInfo { column_default: None, ..column("name", "varchar(50)", None) }];
        let diffs = diff_columns_with_options(&source, &target, false, false, false, 0.5);
        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);
        assert!(sql.contains("MODIFY COLUMN"), "default change: {sql}");
    }

    #[test]
    fn added_varchar_column_quotes_a_bare_default_mysql() {
        // MySQL's information_schema returns a string default unquoted, so the
        // generated DDL read `DEFAULT THE_VALUE` and the deploy failed.
        let source =
            vec![ColumnInfo { column_default: Some("THE_VALUE".into()), ..column("menu_type", "varchar(64)", None) }];
        let target: Vec<ColumnInfo> = vec![];
        let diffs = diff_columns_with_options(&source, &target, false, false, false, 0.5);
        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);
        assert!(sql.contains("DEFAULT 'THE_VALUE'"), "bare default must be quoted: {sql}");
    }

    #[test]
    fn default_literal_only_quotes_bare_values_that_need_it() {
        use DialectKind::Mysql;

        // MySQL strips the quotes from a string default, which is the whole
        // reason this function exists.
        assert_eq!(default_literal("THE_VALUE", "varchar(64)", Mysql, None), "'THE_VALUE'");
        assert_eq!(default_literal("it's", "text", Mysql, None), "'it''s'");
        assert_eq!(default_literal("2024-01-01", "date", Mysql, None), "'2024-01-01'");
        // `DEFAULT ''` previously emitted a bare `DEFAULT `.
        assert_eq!(default_literal("", "varchar(20)", Mysql, None), "''");
        // Untouched on MySQL: already quoted and numeric values.
        assert_eq!(default_literal("'guest'", "varchar(50)", Mysql, None), "'guest'");
        assert_eq!(default_literal("0", "bigint", Mysql, None), "0");
        assert_eq!(default_literal("NULL", "varchar(10)", Mysql, None), "'NULL'");
        assert_eq!(default_literal("null", "text", Mysql, None), "'null'");
        assert_eq!(default_literal("  spaced  ", "varchar(32)", Mysql, None), "'  spaced  '");
    }

    #[test]
    fn default_literal_uses_mysql_extra_to_tell_an_expression_from_a_string() {
        use DialectKind::Mysql;

        // 8.0.13+ marks an expression default in EXTRA, and that marker decides
        // it. Without the marker the value is a string, parentheses and all,
        // so a column declared `DEFAULT 'a(b)'` stops emitting invalid
        // `DEFAULT a(b)`.
        assert_eq!(default_literal("a(b)", "varchar(32)", Mysql, None), "'a(b)'");
        assert_eq!(default_literal("uuid()", "varchar(36)", Mysql, Some("DEFAULT_GENERATED")), "uuid()");
        // MySQL wraps an expression default in parentheses and reports it that
        // way, so the wrapping identifies it even when EXTRA is missing. That is
        // a different question from whether the value contains a parenthesis,
        // which is what `a(b)` above turns on.
        assert_eq!(default_literal("(uuid())", "varchar(36)", Mysql, None), "(uuid())");
        assert_eq!(default_literal("(now())", "datetime", Mysql, None), "(now())");
        assert_eq!(
            default_literal(
                "CURRENT_TIMESTAMP",
                "datetime",
                Mysql,
                Some("DEFAULT_GENERATED on update CURRENT_TIMESTAMP")
            ),
            "CURRENT_TIMESTAMP"
        );
        // Before 8.0.13 there is no marker, and a temporal column was the only
        // place an expression default could appear.
        assert_eq!(default_literal("CURRENT_TIMESTAMP", "datetime", Mysql, None), "CURRENT_TIMESTAMP");
    }

    #[test]
    fn default_literal_keeps_temporal_defaults_that_carry_a_precision() {
        use DialectKind::Mysql;

        // `TIMESTAMP(6) DEFAULT CURRENT_TIMESTAMP(6)` is valid MySQL. On a server
        // older than 8.0.13 it arrives with no EXTRA marker, so the fallback has
        // to accept the precision argument or the column is deployed with a
        // quoted string where an expression belongs.
        assert_eq!(default_literal("CURRENT_TIMESTAMP(6)", "timestamp(6)", Mysql, None), "CURRENT_TIMESTAMP(6)");
        assert_eq!(default_literal("NOW()", "datetime", Mysql, None), "NOW()");
        assert_eq!(default_literal("LOCALTIME(3)", "datetime(3)", Mysql, None), "LOCALTIME(3)");
        assert_eq!(default_literal("LOCALTIMESTAMP(3)", "timestamp(3)", Mysql, None), "LOCALTIMESTAMP(3)");
        assert_eq!(default_literal("current_timestamp(6)", "timestamp(6)", Mysql, None), "current_timestamp(6)");

        // The precision form must not become a general "contains a parenthesis"
        // rule again: a string default keeps its quotes.
        assert_eq!(default_literal("a(b)", "varchar(32)", Mysql, None), "'a(b)'");
        assert_eq!(default_literal("CURRENT_TIMESTAMPX(6)", "varchar(64)", Mysql, None), "'CURRENT_TIMESTAMPX(6)'");
    }

    #[test]
    fn default_literal_handles_set_and_binary_boundaries() {
        use DialectKind::Mysql;

        // A SET default is a bare comma-separated string.
        assert_eq!(default_literal("a,b", "set('a','b')", Mysql, None), "'a,b'");
        assert_eq!(default_literal("", "set('a','b')", Mysql, None), "''");
        // Binary defaults arrive as a hex literal, which is already valid
        // unquoted; a bare string on the same column still needs quoting.
        assert_eq!(default_literal("0x61", "varbinary(16)", Mysql, None), "0x61");
        assert_eq!(default_literal("abc", "binary(3)", Mysql, None), "'abc'");
        assert_eq!(default_literal("x'1f'", "blob", Mysql, None), "x'1f'");
        // Not hex, so not a hex literal.
        assert_eq!(default_literal("0xzz", "varbinary(8)", Mysql, None), "'0xzz'");
    }

    // -- 35. Column order changes --
    #[test]
    fn column_order_change_only_no_type_change() {
        let source = vec![column("id", "int", None), column("name", "text", None), column("age", "int", None)];
        let target = vec![column("age", "int", None), column("name", "text", None), column("id", "int", None)];
        let diffs = diff_columns_with_options(&source, &target, false, true, false, 0.5);
        assert!(!diffs.is_empty(), "should detect order changes");
        assert!(diffs.iter().all(|d| d.diff_type == "modified"), "all should be modified");
        assert!(diffs.iter().all(|d| d.changes.iter().any(|c| c.starts_with("order:"))), "all order changes");
    }

    // -- 36. prepare_schema_diff integration with source_dialect --
    #[test]
    fn prepare_schema_diff_with_source_dialect() {
        let options = SchemaDiffPreparationOptions {
            source_tables: vec![TableInfo {
                name: "users".into(),
                table_type: "BASE TABLE".into(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            }],
            target_tables: vec![TableInfo {
                name: "users".into(),
                table_type: "BASE TABLE".into(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            }],
            source_details: vec![TableSchemaDetail {
                name: "users".into(),
                columns: vec![column("name2", "varchar(100)", None), column("id", "int(11)", None)],
                indexes: vec![],
                foreign_keys: vec![],
                triggers: vec![],
                ddl: None,
            }],
            target_details: vec![TableSchemaDetail {
                name: "users".into(),
                columns: vec![column("name", "varchar(100)", None), column("id", "int", None)],
                indexes: vec![],
                foreign_keys: vec![],
                triggers: vec![],
                ddl: None,
            }],
            database_type: DatabaseType::Mysql,
            target_schema: None,
            ignore_comments: false,
            cascade_delete: false,
            compare_column_order: false,
            detect_renames: true,
            detect_table_renames: false,
            rename_threshold: 0.5,
            enable_rollback: false,
            source_dialect: Some(DialectKind::Mysql),
            target_dialect: Some(DialectKind::Mysql),
            ..Default::default()
        };
        let result = prepare_schema_diff(options);
        assert!(!result.diffs.is_empty(), "should have diffs");
        let sql = &result.sync_sql;
        assert!(sql.contains("CHANGE COLUMN"), "detected rename: {sql}");
        assert!(!sql.contains("DROP COLUMN"), "no false drop: {sql}");
    }

    #[test]
    fn index_and_rename_combined_mysql() {
        let col_diffs =
            make_col_diffs(&[("id", "int"), ("name2", "varchar(50)")], &[("id", "int"), ("name", "varchar(50)")], true);
        let table_diff = TableDiff {
            diff_type: "modified".to_string(),
            object_type: Some("table".to_string()),
            name: "t".to_string(),
            target_name: None,
            columns: Some(col_diffs),
            indexes: Some(vec![IndexDiff {
                diff_type: "removed".to_string(),
                name: "idx_old".to_string(),
                source: None,
                target: Some(index(IndexInfo {
                    name: "idx_old".to_string(),
                    columns: vec!["name".to_string()],
                    is_unique: false,
                    is_primary: false,
                    filter: None,
                    index_type: None,
                    included_columns: None,
                    comment: None,
                    key_is_expression: Vec::new(),
                    column_opclasses: vec![],
                    key_options: Vec::new(),
                    constraint_backed: false,
                })),
                changes: vec![],
            }]),
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: None,
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        };
        let sql =
            generate_schema_sync_sql(&[table_diff], &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);
        assert!(sql.contains("CHANGE COLUMN"), "rename: {sql}");
        assert!(sql.contains("DROP INDEX"), "drop index: {sql}");
    }

    // -- 40. Rollback SQL with column renames --
    #[test]
    fn rollback_graph_with_renames() {
        let diffs = vec![TableDiff {
            diff_type: "modified".to_string(),
            object_type: Some("table".to_string()),
            name: "t".to_string(),
            target_name: None,
            columns: Some(vec![ColumnDiff {
                diff_type: "renamed".to_string(),
                name: "new_name".to_string(),
                source: Some(column("new_name", "varchar(50)", None)),
                target: Some(column("old_name", "varchar(50)", None)),
                changes: vec!["old_name → new_name".to_string()],
                add_position: None,
            }]),
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: None,
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        }];
        let dep_graph = DependencyGraph::build(&[], &[]);
        let graph = RollbackGraph::from_forward_diffs(&diffs, &[], &dep_graph);
        let rollback_sql = generate_rollback_sync_sql(&graph, DatabaseType::Mysql, None, false);
        assert!(
            rollback_sql.contains("CHANGE COLUMN `new_name` `old_name`"),
            "rollback should reverse rename: {rollback_sql}"
        );
    }

    #[test]
    fn table_comment_change_mysql() {
        let table_diff = TableDiff {
            diff_type: "modified".to_string(),
            object_type: Some("table".to_string()),
            name: "t".to_string(),
            target_name: None,
            columns: None,
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: None,
            source_table_comment: Some(Some("new".to_string())),
            target_table_comment: Some(Some("old".to_string())),
            sync_sql: None,
        };
        let sql =
            generate_schema_sync_sql(&[table_diff], &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);
        assert!(sql.contains("COMMENT ="), "MySQL table comment: {sql}");
    }

    // -- 44. Detect renames function (table-level) --
    #[test]
    fn table_detect_renames_exact_match() {
        let removed = vec!["old_table".to_string()];
        let added = vec!["new_table".to_string()];
        let source_details = vec![TableSchemaDetail {
            name: "new_table".into(),
            columns: vec![column("id", "int", None), column("name", "text", None)],
            indexes: vec![],
            foreign_keys: vec![],
            triggers: vec![],
            ddl: None,
        }];
        let target_details = vec![TableSchemaDetail {
            name: "old_table".into(),
            columns: vec![column("id", "int", None), column("name", "text", None)],
            indexes: vec![],
            foreign_keys: vec![],
            triggers: vec![],
            ddl: None,
        }];
        let candidates = detect_renames(&removed, &added, &source_details, &target_details, 0.5);
        assert_eq!(candidates.len(), 1, "should detect table rename");
        assert_eq!(candidates[0].source_name, "new_table");
        assert_eq!(candidates[0].target_name, "old_table");
    }

    // -- 45. Column rename detection: greedy matching avoids conflicts --
    #[test]
    fn rename_greedy_matching() {
        let s: Vec<ColumnInfo> = vec![column("a", "int", None), column("b", "varchar(10)", None)];
        let t: Vec<ColumnInfo> = vec![column("x", "int", None), column("y", "int", None)];
        let diffs = diff_columns_with_options(&s, &t, false, false, true, 0.5);
        let renamed: Vec<_> = diffs.iter().filter(|d| d.diff_type == "renamed").collect();
        // Only one should be renamed (greedy: best score), the other stays added/removed
        assert!(renamed.len() <= 1, "greedy should avoid double matching: {renamed:?}");
    }

    // -- 46. Column precision/scale changes --
    #[test]
    fn column_precision_scale_change() {
        let s = vec![column("amount", "decimal(10,2)", None)];
        let t = vec![column("amount", "decimal(8,0)", None)];
        let diffs = diff_columns_with_options(&s, &t, false, false, false, 0.5);
        assert_eq!(diffs.len(), 1, "should detect precision change");
        assert!(
            diffs[0].changes.iter().any(|c| c.contains("decimal(8,0) → decimal(10,2)")),
            "precision diff: {:?}",
            diffs[0].changes
        );
    }

    #[test]
    fn column_precision_change_generates_sql() {
        let s = vec![column("price", "decimal(10,2)", None)];
        let t = vec![column("price", "decimal(8,2)", None)];
        let diffs = diff_columns_with_options(&s, &t, false, false, false, 0.5);
        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);
        assert!(sql.contains("decimal(10,2)"), "precision in sql: {sql}");
    }

    // -- 47. Column length changes --
    #[test]
    fn column_length_change_detected() {
        let s = vec![column("name", "varchar(255)", None)];
        let t = vec![column("name", "varchar(100)", None)];
        let diffs = diff_columns_with_options(&s, &t, false, false, false, 0.5);
        assert_eq!(diffs.len(), 1, "should detect length change");
        assert!(
            diffs[0].changes.iter().any(|c| c.contains("varchar(100) → varchar(255)")),
            "length diff: {:?}",
            diffs[0].changes
        );
    }

    // -- 48. Column comment changes with ignore_comments option --
    #[test]
    fn column_comment_change_detected() {
        let s = vec![column("name", "int", Some("new comment"))];
        let t = vec![column("name", "int", Some("old comment"))];
        let diffs = diff_columns_with_options(&s, &t, false, false, false, 0.5);
        assert_eq!(diffs.len(), 1, "should detect comment change");
        assert!(diffs[0].changes.iter().any(|c| c.starts_with("comment:")), "comment diff: {:?}", diffs[0].changes);
    }

    #[test]
    fn column_comment_ignored_when_option_set() {
        let s = vec![column("name", "int", Some("new"))];
        let t = vec![column("name", "int", Some("old"))];
        let diffs = diff_columns_with_options(&s, &t, true, false, false, 0.5);
        assert!(diffs.is_empty(), "should ignore comment when option set: {diffs:?}");
    }

    #[test]
    fn column_comment_change_mysql_sql() {
        let s = vec![column("name", "varchar(50)", Some("中文注释"))];
        let t = vec![column("name", "varchar(50)", Some("old"))];
        let diffs = diff_columns_with_options(&s, &t, false, false, false, 0.5);
        let sql = gen_sql(wrap_table_diff("t", diffs), DatabaseType::Mysql, None);
        assert!(sql.contains("COMMENT"), "MySQL comment: {sql}");
        assert!(sql.contains("中文注释"), "Chinese comment: {sql}");
    }

    #[test]
    fn table_comment_change_mysql_sql() {
        let table_diff = TableDiff {
            diff_type: "modified".to_string(),
            object_type: Some("table".to_string()),
            name: "t".to_string(),
            target_name: None,
            columns: None,
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl: None,
            target_ddl: None,
            source_table_comment: Some(Some("新表".to_string())),
            target_table_comment: Some(Some("旧表".to_string())),
            sync_sql: None,
        };
        let sql =
            generate_schema_sync_sql(&[table_diff], &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);
        assert!(sql.contains("COMMENT ="), "MySQL table comment: {sql}");
        assert!(sql.contains("新表"), "Chinese table comment: {sql}");
    }

    #[test]
    fn table_comment_ignored_with_option() {
        let options = SchemaDiffPreparationOptions {
            source_tables: vec![TableInfo {
                name: "t".into(),
                table_type: "BASE TABLE".into(),
                valid: None,
                comment: Some("new".into()),
                parent_schema: None,
                parent_name: None,
            }],
            target_tables: vec![TableInfo {
                name: "t".into(),
                table_type: "BASE TABLE".into(),
                valid: None,
                comment: Some("old".into()),
                parent_schema: None,
                parent_name: None,
            }],
            source_details: vec![TableSchemaDetail {
                name: "t".into(),
                columns: vec![],
                indexes: vec![],
                foreign_keys: vec![],
                triggers: vec![],
                ddl: None,
            }],
            target_details: vec![TableSchemaDetail {
                name: "t".into(),
                columns: vec![],
                indexes: vec![],
                foreign_keys: vec![],
                triggers: vec![],
                ddl: None,
            }],
            database_type: DatabaseType::Mysql,
            ignore_comments: true,
            ..Default::default()
        };
        let result = prepare_schema_diff(options);
        assert!(result.diffs.is_empty(), "should ignore table comment: {:?}", result.diffs);
    }

    // -- 49. Index type differences --
    #[test]
    fn index_type_diff_btree_vs_hash() {
        let diffs = diff_indexes(
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["a".into()],
                is_unique: false,
                is_primary: false,
                filter: None,
                index_type: Some("BTREE".into()),
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["a".into()],
                is_unique: false,
                is_primary: false,
                filter: None,
                index_type: Some("HASH".into()),
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
        );
        assert_eq!(diffs.len(), 1, "index type diff detected");
        assert!(diffs[0].changes.iter().any(|c| c.contains("type:")), "type change: {:?}", diffs[0].changes);
    }

    #[test]
    fn index_type_fulltext_detected() {
        let diffs = diff_indexes(
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["content".into()],
                is_unique: false,
                is_primary: false,
                filter: None,
                index_type: Some("FULLTEXT".into()),
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["content".into()],
                is_unique: false,
                is_primary: false,
                filter: None,
                index_type: None,
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
        );
        assert_eq!(diffs[0].changes.iter().filter(|c| c.contains("FULLTEXT")).count(), 1, "fulltext change");
    }

    // -- 50. Index column ordering --
    #[test]
    fn index_column_order_different() {
        let diffs = diff_indexes(
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["a".into(), "b".into()],
                is_unique: false,
                is_primary: false,
                filter: None,
                index_type: None,
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["b".into(), "a".into()],
                is_unique: false,
                is_primary: false,
                filter: None,
                index_type: None,
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
        );
        assert_eq!(diffs.len(), 1, "order diff detected");
        assert!(diffs[0].changes.iter().any(|c| c.contains("columns:")), "column order change: {:?}", diffs[0].changes);
    }

    // -- 51. Included columns in indexes --
    #[test]
    fn index_included_columns_diff() {
        let diffs = diff_indexes(
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["a".into()],
                is_unique: true,
                is_primary: false,
                filter: None,
                index_type: None,
                included_columns: Some(vec!["b".into(), "c".into()]),
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["a".into()],
                is_unique: true,
                is_primary: false,
                filter: None,
                index_type: None,
                included_columns: Some(vec!["b".into()]),
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
        );
        assert_eq!(diffs.len(), 1, "included columns diff detected");
        assert!(diffs[0].changes.iter().any(|c| c.contains("include:")), "include change: {:?}", diffs[0].changes);
    }

    #[test]
    fn index_included_columns_added() {
        let diffs = diff_indexes(
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["a".into()],
                is_unique: true,
                is_primary: false,
                filter: None,
                index_type: None,
                included_columns: Some(vec!["b".into()]),
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["a".into()],
                is_unique: true,
                is_primary: false,
                filter: None,
                index_type: None,
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
        );
        assert_eq!(diffs.len(), 1, "included added");
    }

    // -- 52. Filtered/partial indexes --
    #[test]
    fn index_filter_change() {
        let diffs = diff_indexes(
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["status".into()],
                is_unique: false,
                is_primary: false,
                filter: Some("status > 0".into()),
                index_type: None,
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
            &[index(IndexInfo {
                name: "idx_t".into(),
                columns: vec!["status".into()],
                is_unique: false,
                is_primary: false,
                filter: None,
                index_type: None,
                included_columns: None,
                comment: None,
                key_is_expression: Vec::new(),
                column_opclasses: vec![],
                key_options: Vec::new(),
                constraint_backed: false,
            })],
        );
        assert_eq!(diffs.len(), 1, "filter diff");
        assert!(diffs[0].changes.iter().any(|c| c.contains("filter:")), "filter change: {:?}", diffs[0].changes);
    }

    // -- 53. Multiple index operations in one diff --
    #[test]
    fn multiple_index_operations() {
        let diffs = diff_indexes(
            &[
                index(IndexInfo {
                    name: "idx_new".into(),
                    columns: vec!["a".into()],
                    is_unique: true,
                    is_primary: false,
                    filter: None,
                    index_type: None,
                    included_columns: None,
                    comment: None,
                    key_is_expression: Vec::new(),
                    column_opclasses: vec![],
                    key_options: Vec::new(),
                    constraint_backed: false,
                }),
                index(IndexInfo {
                    name: "idx_modified".into(),
                    columns: vec!["a".into(), "b".into()],
                    is_unique: false,
                    is_primary: false,
                    filter: None,
                    index_type: Some("BTREE".into()),
                    included_columns: None,
                    comment: None,
                    key_is_expression: Vec::new(),
                    column_opclasses: vec![],
                    key_options: Vec::new(),
                    constraint_backed: false,
                }),
            ],
            &[
                index(IndexInfo {
                    name: "idx_removed".into(),
                    columns: vec!["c".into()],
                    is_unique: false,
                    is_primary: false,
                    filter: None,
                    index_type: None,
                    included_columns: None,
                    comment: None,
                    key_is_expression: Vec::new(),
                    column_opclasses: vec![],
                    key_options: Vec::new(),
                    constraint_backed: false,
                }),
                index(IndexInfo {
                    name: "idx_modified".into(),
                    columns: vec!["a".into()],
                    is_unique: true,
                    is_primary: false,
                    filter: None,
                    index_type: None,
                    included_columns: None,
                    comment: None,
                    key_is_expression: Vec::new(),
                    column_opclasses: vec![],
                    key_options: Vec::new(),
                    constraint_backed: false,
                }),
            ],
        );
        assert_eq!(diffs.len(), 3, "add + modify + remove: {diffs:?}");
        let types: Vec<&str> = diffs.iter().map(|d| d.diff_type.as_str()).collect();
        assert!(types.contains(&"added"), "should have added");
        assert!(types.contains(&"removed"), "should have removed");
        assert!(types.contains(&"modified"), "should have modified");
    }

    // -- 54. Foreign key ref_table / ref_column changes --
    #[test]
    fn foreign_key_reference_table_change() {
        let diffs = diff_foreign_keys(
            &[foreign_key(ForeignKeyInfo {
                name: "fk_t".into(),
                column: "user_id".into(),
                ref_schema: None,
                ref_table: "users".into(),
                ref_column: "id".into(),
                on_update: None,
                on_delete: None,
            })],
            &[foreign_key(ForeignKeyInfo {
                name: "fk_t".into(),
                column: "user_id".into(),
                ref_schema: None,
                ref_table: "employees".into(),
                ref_column: "id".into(),
                on_update: None,
                on_delete: None,
            })],
        );
        assert_eq!(diffs.len(), 1, "ref_table change");
        assert!(diffs[0].changes.iter().any(|c| c.contains("ref table")), "ref table: {:?}", diffs[0].changes);
    }

    #[test]
    fn foreign_key_reference_column_change() {
        let diffs = diff_foreign_keys(
            &[foreign_key(ForeignKeyInfo {
                name: "fk_t".into(),
                column: "user_id".into(),
                ref_schema: None,
                ref_table: "users".into(),
                ref_column: "id".into(),
                on_update: None,
                on_delete: None,
            })],
            &[foreign_key(ForeignKeyInfo {
                name: "fk_t".into(),
                column: "user_id".into(),
                ref_schema: None,
                ref_table: "users".into(),
                ref_column: "uid".into(),
                on_update: None,
                on_delete: None,
            })],
        );
        assert_eq!(diffs.len(), 1, "ref_column change");
    }

    #[test]
    fn foreign_key_local_column_change() {
        let diffs = diff_foreign_keys(
            &[foreign_key(ForeignKeyInfo {
                name: "fk_t".into(),
                column: "user_id".into(),
                ref_schema: None,
                ref_table: "users".into(),
                ref_column: "id".into(),
                on_update: None,
                on_delete: None,
            })],
            &[foreign_key(ForeignKeyInfo {
                name: "fk_t".into(),
                column: "member_id".into(),
                ref_schema: None,
                ref_table: "users".into(),
                ref_column: "id".into(),
                on_update: None,
                on_delete: None,
            })],
        );
        assert_eq!(diffs.len(), 1, "local column change");
    }

    #[test]
    fn foreign_key_referential_action_change() {
        let diffs = diff_foreign_keys(
            &[foreign_key(ForeignKeyInfo {
                name: "fk_t".into(),
                column: "user_id".into(),
                ref_schema: Some("auth".into()),
                ref_table: "users".into(),
                ref_column: "id".into(),
                on_update: Some("cascade".into()),
                on_delete: Some("SET NULL".into()),
            })],
            &[foreign_key(ForeignKeyInfo {
                name: "fk_t".into(),
                column: "user_id".into(),
                ref_schema: Some("auth".into()),
                ref_table: "users".into(),
                ref_column: "id".into(),
                on_update: Some("NO ACTION".into()),
                on_delete: Some("RESTRICT".into()),
            })],
        );
        assert_eq!(diffs.len(), 1, "referential action change: {diffs:?}");
        assert!(diffs[0].changes.iter().any(|change| change == "delete: RESTRICT → SET NULL"));
        assert!(diffs[0].changes.iter().any(|change| change == "update: NO ACTION → CASCADE"));
    }

    // -- Regression: issue #7287 --
    // MySQL's information_schema always fills REFERENCED_TABLE_SCHEMA with the literal
    // database name, even for a foreign key that just self-references a table in its own
    // database. Comparing two differently-named databases (e.g. a dev copy vs prod) made
    // every such self-referencing FK look "changed" purely because the database names
    // differ, and the deploy script it generated pointed the target's FK at the *source*
    // database instead of leaving it self-referencing within the target.
    fn self_referencing_fk_options(
        source_ref_schema: &str,
        target_ref_schema: &str,
        source_on_delete: &str,
        target_on_delete: &str,
    ) -> SchemaDiffPreparationOptions {
        let table_infos = vec![
            TableInfo {
                name: "sys_organization".into(),
                table_type: "BASE TABLE".into(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            },
            TableInfo {
                name: "sys_user".into(),
                table_type: "BASE TABLE".into(),
                valid: None,
                comment: None,
                parent_schema: None,
                parent_name: None,
            },
        ];
        let cols = vec![column("id", "int(11)", None), column("leader_id", "int(11)", None)];
        let fk = |ref_schema: &str, on_delete: &str| ForeignKeyInfo {
            name: "sys_organization_ibfk_1".into(),
            column: "leader_id".into(),
            ref_schema: Some(ref_schema.to_string()),
            ref_table: "sys_user".into(),
            ref_column: "user_id".into(),
            on_update: Some("RESTRICT".into()),
            on_delete: Some(on_delete.to_string()),
        };
        SchemaDiffPreparationOptions {
            source_tables: table_infos.clone(),
            target_tables: table_infos,
            source_details: vec![
                TableSchemaDetail {
                    name: "sys_organization".into(),
                    columns: cols.clone(),
                    indexes: vec![],
                    foreign_keys: vec![fk(source_ref_schema, source_on_delete)],
                    triggers: vec![],
                    ddl: None,
                },
                TableSchemaDetail {
                    name: "sys_user".into(),
                    columns: vec![column("user_id", "int(11)", None)],
                    indexes: vec![],
                    foreign_keys: vec![],
                    triggers: vec![],
                    ddl: None,
                },
            ],
            target_details: vec![
                TableSchemaDetail {
                    name: "sys_organization".into(),
                    columns: cols.clone(),
                    indexes: vec![],
                    foreign_keys: vec![fk(target_ref_schema, target_on_delete)],
                    triggers: vec![],
                    ddl: None,
                },
                TableSchemaDetail {
                    name: "sys_user".into(),
                    columns: vec![column("user_id", "int(11)", None)],
                    indexes: vec![],
                    foreign_keys: vec![],
                    triggers: vec![],
                    ddl: None,
                },
            ],
            database_type: DatabaseType::Mysql,
            target_schema: Some("jinxinnuo_agent_db".into()),
            ignore_comments: false,
            cascade_delete: false,
            compare_column_order: false,
            detect_renames: true,
            detect_table_renames: false,
            rename_threshold: 0.5,
            enable_rollback: false,
            source_dialect: Some(DialectKind::Mysql),
            target_dialect: Some(DialectKind::Mysql),
            ..Default::default()
        }
    }

    #[test]
    fn self_referencing_fk_across_differently_named_databases_is_not_a_diff() {
        let options =
            self_referencing_fk_options("jinxinnuo_agent_db_test", "jinxinnuo_agent_db", "SET NULL", "SET NULL");
        let result = prepare_schema_diff(options);
        assert!(
            !result.sync_sql.contains("sys_organization_ibfk_1"),
            "same-database self-reference must not be resynced just because the two \
             database names differ: {}",
            result.sync_sql
        );
    }

    #[test]
    fn modified_self_referencing_fk_regenerates_against_target_database() {
        // A genuine change (ON DELETE) forces the FK to be resynced; the regenerated
        // REFERENCES clause must still point at the target's own database, not the source's.
        let options =
            self_referencing_fk_options("jinxinnuo_agent_db_test", "jinxinnuo_agent_db", "SET NULL", "CASCADE");
        let result = prepare_schema_diff(options);
        assert!(
            !result.sync_sql.contains("jinxinnuo_agent_db_test"),
            "must not reference the source database: {}",
            result.sync_sql
        );
        assert!(
            result.sync_sql.contains("REFERENCES `jinxinnuo_agent_db`.`sys_user`")
                || result.sync_sql.contains("REFERENCES `sys_user`"),
            "must reference the target database (or be left unqualified): {}",
            result.sync_sql
        );
    }

    #[test]
    fn genuine_cross_database_fk_reference_change_is_still_detected() {
        // `external_lookup` is not one of the tables being compared, so a differing
        // ref_schema here is a real cross-database reference change, not a same-database
        // self-reference — it must still be surfaced and regenerated with the source's value.
        let table_infos = vec![TableInfo {
            name: "orders".into(),
            table_type: "BASE TABLE".into(),
            valid: None,
            comment: None,
            parent_schema: None,
            parent_name: None,
        }];
        let cols = vec![column("id", "int(11)", None), column("region_id", "int(11)", None)];
        let make_detail = |ref_schema: &str| TableSchemaDetail {
            name: "orders".into(),
            columns: cols.clone(),
            indexes: vec![],
            foreign_keys: vec![ForeignKeyInfo {
                name: "orders_region_fk".into(),
                column: "region_id".into(),
                ref_schema: Some(ref_schema.to_string()),
                ref_table: "external_lookup".into(),
                ref_column: "id".into(),
                on_update: Some("RESTRICT".into()),
                on_delete: Some("RESTRICT".into()),
            }],
            triggers: vec![],
            ddl: None,
        };
        let options = SchemaDiffPreparationOptions {
            source_tables: table_infos.clone(),
            target_tables: table_infos,
            source_details: vec![make_detail("shared_lookup_db")],
            target_details: vec![make_detail("stale_lookup_db")],
            database_type: DatabaseType::Mysql,
            target_schema: Some("jinxinnuo_agent_db".into()),
            ignore_comments: false,
            cascade_delete: false,
            compare_column_order: false,
            detect_renames: true,
            detect_table_renames: false,
            rename_threshold: 0.5,
            enable_rollback: false,
            source_dialect: Some(DialectKind::Mysql),
            target_dialect: Some(DialectKind::Mysql),
            ..Default::default()
        };
        let result = prepare_schema_diff(options);
        assert!(
            result.sync_sql.contains("REFERENCES `shared_lookup_db`.`external_lookup`"),
            "genuine cross-database reference change must still be resynced to the source's \
             external database: {}",
            result.sync_sql
        );
    }

    // -- 56. Column order changes with comment option --
    #[test]
    fn column_order_ignored_when_disabled_but_comment_detected() {
        let s = vec![column("a", "int", Some("x")), column("b", "varchar(10)", None)];
        let t = vec![column("b", "varchar(10)", None), column("a", "int", None)];
        let diffs = diff_columns_with_options(&s, &t, false, false, false, 0.5);
        // order compare disabled, so only comment change on "a" should be detected
        assert!(!diffs.is_empty(), "comment change should be detected: {diffs:?}");
    }

    // -- 58. Column diff with all attributes different --
    #[test]
    fn column_all_attributes_changed() {
        let s = vec![ColumnInfo {
            name: "c".into(),
            data_type: "varchar(100)".into(),
            resolved_schema: None,
            is_nullable: true,
            column_default: Some("'default'".into()),
            comment: Some("new".into()),
            is_primary_key: false,
            is_unique: false,
            extra: None,
            numeric_precision: None,
            numeric_scale: None,
            character_maximum_length: None,
            metadata_capabilities: None,
            enum_values: None,
            character_set: None,
            collation: None,
        }];
        let t = vec![ColumnInfo {
            name: "c".into(),
            data_type: "varchar(50)".into(),
            resolved_schema: None,
            is_nullable: false,
            column_default: None,
            comment: Some("old".into()),
            is_primary_key: false,
            is_unique: false,
            extra: None,
            numeric_precision: None,
            numeric_scale: None,
            character_maximum_length: None,
            metadata_capabilities: None,
            enum_values: None,
            character_set: None,
            collation: None,
        }];
        let diffs = diff_columns_with_options(&s, &t, false, false, false, 0.5);
        assert_eq!(diffs.len(), 1, "all changes in one diff");
        let changes = &diffs[0].changes;
        assert!(changes.iter().any(|c| c.starts_with("type:")), "type: {changes:?}");
        assert!(changes.iter().any(|c| c.starts_with("nullable:")), "nullable: {changes:?}");
        assert!(changes.iter().any(|c| c.starts_with("default:")), "default: {changes:?}");
        assert!(changes.iter().any(|c| c.starts_with("comment:")), "comment: {changes:?}");
    }

    // -- 56. Foreign key with multiple changes (ref_table + ref_column) --
    #[test]
    fn foreign_key_multiple_changes() {
        let diffs = diff_foreign_keys(
            &[foreign_key(ForeignKeyInfo {
                name: "fk".into(),
                column: "id".into(),
                ref_schema: None,
                ref_table: "users".into(),
                ref_column: "id".into(),
                on_update: None,
                on_delete: None,
            })],
            &[foreign_key(ForeignKeyInfo {
                name: "fk".into(),
                column: "id".into(),
                ref_schema: None,
                ref_table: "employees".into(),
                ref_column: "uid".into(),
                on_update: None,
                on_delete: None,
            })],
        );
        assert_eq!(diffs.len(), 1, "multiple FK changes");
        assert!(diffs[0].changes.iter().any(|c| c.contains("ref table")), "ref table: {:?}", diffs[0].changes);
        assert!(diffs[0].changes.iter().any(|c| c.contains("ref column")), "ref column: {:?}", diffs[0].changes);
    }

    // -- 60. With and without ignore_comments on prepare_schema_diff --
    #[test]
    fn prepare_schema_diff_comment_option_toggle() {
        fn run_test(ignore: bool, expect_diffs: bool) {
            let options = SchemaDiffPreparationOptions {
                source_tables: vec![TableInfo {
                    name: "t".into(),
                    table_type: "BASE TABLE".into(),
                    valid: None,
                    comment: Some("new_comment".into()),
                    parent_schema: None,
                    parent_name: None,
                }],
                target_tables: vec![TableInfo {
                    name: "t".into(),
                    table_type: "BASE TABLE".into(),
                    valid: None,
                    comment: Some("old_comment".into()),
                    parent_schema: None,
                    parent_name: None,
                }],
                source_details: vec![TableSchemaDetail {
                    name: "t".into(),
                    columns: vec![column("c", "int", Some("col_new"))],
                    indexes: vec![],
                    foreign_keys: vec![],
                    triggers: vec![],
                    ddl: None,
                }],
                target_details: vec![TableSchemaDetail {
                    name: "t".into(),
                    columns: vec![column("c", "int", Some("col_old"))],
                    indexes: vec![],
                    foreign_keys: vec![],
                    triggers: vec![],
                    ddl: None,
                }],
                database_type: DatabaseType::Mysql,
                ignore_comments: ignore,
                ..Default::default()
            };
            let result = prepare_schema_diff(options);
            if expect_diffs {
                assert!(!result.diffs.is_empty(), "should have diffs when ignore={ignore}");
            } else {
                assert!(result.diffs.is_empty(), "should be empty when ignore={ignore}");
            }
        }
        run_test(true, false);
        run_test(false, true);
    }

    fn kind_to_db(kind: DialectKind) -> Option<DatabaseType> {
        match kind {
            DialectKind::Mysql => Some(DatabaseType::Mysql),

            _ => None,
        }
    }

    fn col_pk(name: &str, data_type: &str) -> ColumnInfo {
        ColumnInfo { is_primary_key: true, ..column(name, data_type, None) }
    }

    fn check_identifiers(sql: &str, tgt: DialectKind) {
        match tgt {
            DialectKind::Mysql => {
                assert!(sql.contains('`'), "{tgt:?} should use backticks");
            }

            _ => {
                assert!(!sql.contains('`'), "{tgt:?} should NOT use backticks: {sql}");
            }
        }
    }

    fn check_no_mysql_residue(sql: &str, tgt: DialectKind) {
        if !matches!(tgt, DialectKind::Mysql) {
            assert!(!sql.contains("ENGINE="), "residual ENGINE= in {tgt:?}: {sql}");
            assert!(!sql.contains("CHARSET"), "residual CHARSET in {tgt:?}: {sql}");
        }
    }

    fn check_auto_increment(sql: &str, tgt: DialectKind) {
        match tgt {
            DialectKind::Mysql => {
                assert!(sql.contains("AUTO_INCREMENT"), "{tgt:?} should have AUTO_INCREMENT: {sql}");
            }

            _ => {
                // Other dialects may or may not have auto-increment
            }
        }
    }

    fn check_type_conversion(sql: &str, src: DialectKind, tgt: DialectKind) {
        match (src, tgt) {
            _ => {}
        }
    }

    fn check_table_sql_structure(sql: &str, tgt: DialectKind) {
        assert!(sql.contains("CREATE TABLE"), "{tgt:?} missing CREATE TABLE");
        match tgt {
            DialectKind::Mysql => {
                assert!(sql.contains("PRIMARY KEY"), "{tgt:?} missing PK");
            }
            _ => {
                assert!(sql.contains("PRIMARY KEY"), "{tgt:?} missing PK: {sql}");
            }
        }
    }

    // -- S1: simple table (id INT PK AUTO_INCREMENT, name VARCHAR) --
    fn s1_diffs() -> Vec<ColumnDiff> {
        vec![
            ColumnDiff {
                diff_type: "added".into(),
                name: "id".into(),
                source: Some(ColumnInfo { extra: Some("auto_increment".into()), ..col_pk("id", "int") }),
                target: None,
                changes: vec![],
                add_position: None,
            },
            ColumnDiff {
                diff_type: "added".into(),
                name: "name".into(),
                source: Some(ColumnInfo {
                    name: "name".into(),
                    data_type: "varchar(100)".into(),
                    is_nullable: false,
                    ..column("name", "varchar(100)", None)
                }),
                target: None,
                changes: vec![],
                add_position: None,
            },
        ]
    }

    fn s1_table_diff(src_kind: DialectKind, tgt_kind: DialectKind) -> TableDiff {
        let Some(_db) = kind_to_db(tgt_kind) else { panic!("no db for {tgt_kind:?}") };
        let is_mysql_tgt = matches!(tgt_kind, DialectKind::Mysql);
        let ddl = if is_mysql_tgt {
            Some("CREATE TABLE `t` (`id` int NOT NULL AUTO_INCREMENT, `name` varchar(100) NOT NULL, PRIMARY KEY (`id`)) ENGINE=InnoDB".into())
        } else if src_kind == tgt_kind {
            Some(
                "CREATE TABLE \"t\" (\"id\" INTEGER NOT NULL, \"name\" varchar(100) NOT NULL, PRIMARY KEY (\"id\"));"
                    .into(),
            )
        } else {
            None
        };
        TableDiff {
            diff_type: "added".into(),
            object_type: Some("table".into()),
            name: "t".into(),
            target_name: None,
            columns: Some(s1_diffs()),
            indexes: None,
            foreign_keys: None,
            triggers: None,
            ddl,
            target_ddl: None,
            source_table_comment: None,
            target_table_comment: None,
            sync_sql: None,
        }
    }

    #[test]
    fn cross_dialect_s1_all_pairs_simple_table() {
        let kinds = vec![DialectKind::Mysql];
        for src in &kinds {
            for tgt in &kinds {
                let Some(db) = kind_to_db(*tgt) else { continue };
                let src_dialect = if src == tgt { None } else { Some(*src) };
                let td = s1_table_diff(*src, *tgt);
                let sql = generate_schema_sync_sql(&[td], &[], &[], &[], &[], db, None, false, src_dialect, &[]);
                check_table_sql_structure(&sql, *tgt);
                check_identifiers(&sql, *tgt);
                check_no_mysql_residue(&sql, *tgt);
                check_auto_increment(&sql, *tgt);
                check_type_conversion(&sql, *src, *tgt);
            }
        }
    }

    // -- S3: table with foreign keys --
    fn s3_diffs() -> (Vec<ColumnDiff>, Vec<ForeignKeyDiff>) {
        let cols = vec![
            ColumnDiff {
                diff_type: "added".into(),
                name: "id".into(),
                source: Some(col_pk("id", "int")),
                target: None,
                changes: vec![],
                add_position: None,
            },
            ColumnDiff {
                diff_type: "added".into(),
                name: "user_id".into(),
                source: Some(ColumnInfo {
                    name: "user_id".into(),
                    data_type: "int".into(),
                    is_nullable: false,
                    ..column("user_id", "int", None)
                }),
                target: None,
                changes: vec![],
                add_position: None,
            },
        ];
        let fks = vec![ForeignKeyDiff {
            diff_type: "added".into(),
            name: "fk_user".into(),
            source: Some(ForeignKeyInfo {
                name: "fk_user".into(),
                column: "user_id".into(),
                ref_schema: None,
                ref_table: "users".into(),
                ref_column: "id".into(),
                on_update: None,
                on_delete: Some("CASCADE".into()),
            }),
            target: None,
            changes: vec![],
        }];
        (cols, fks)
    }

    #[test]
    fn cross_dialect_s3_foreign_key_table() {
        let kinds = vec![DialectKind::Mysql];
        for src in &kinds {
            for tgt in &kinds {
                let Some(db) = kind_to_db(*tgt) else { continue };
                let src_dialect = if src == tgt { None } else { Some(*src) };
                let (cols, fks) = s3_diffs();
                let td = TableDiff {
                    diff_type: "added".into(),
                    object_type: Some("table".into()),
                    name: "t".into(),
                    target_name: None,
                    columns: Some(cols),
                    indexes: None,
                    foreign_keys: Some(fks),
                    triggers: None,
                    ddl: None,
                    target_ddl: None,
                    source_table_comment: None,
                    target_table_comment: None,
                    sync_sql: None,
                };
                let sql = generate_schema_sync_sql(&[td], &[], &[], &[], &[], db, None, false, src_dialect, &[]);
                check_table_sql_structure(&sql, *tgt);
                check_identifiers(&sql, *tgt);
                check_no_mysql_residue(&sql, *tgt);
                assert!(sql.contains("FOREIGN KEY"), "{tgt:?} FK constraint missing: {sql}");
                assert!(sql.contains("REFERENCES"), "{tgt:?} REFERENCES missing: {sql}");
            }
        }
    }

    #[test]
    fn field_mapping_matches_character_varying_alias() {
        // Kingbase/Postgres report a varchar column's base type as
        // `character varying` (via format_type()), not `varchar`. A user who
        // configures a mapping using the shorter, more common `varchar`
        // spelling must still match it (issue #8011) — previously an exact
        // string comparison meant such a mapping silently never fired
        // against the real `character varying` column, dropping the user's
        // chosen param strategy.
        let mappings = vec![FieldMapping {
            source_type: "varchar".into(),
            target_type: "varchar".into(),
            param_strategy: ParamStrategy::Custom,
            custom_params: Some("255".to_string()),
        }];
        let result = FieldMapping::apply_with_params(&mappings, "character varying", DialectKind::Mysql);
        assert_eq!(result, Some("varchar(255)".to_string()));

        let result = FieldMapping::apply_with_params(&mappings, "character varying(50)", DialectKind::Mysql);
        assert_eq!(result, Some("varchar(255)".to_string()));

        assert_eq!(FieldMapping::apply(&mappings, "character varying"), Some("varchar"));
    }

    #[test]
    fn field_mapping_exact_match_is_not_shadowed_by_an_alias() {
        // char/character/varchar/character varying commonly coexist as
        // separate auto-generated rows with independently chosen targets.
        // An exact match must win over an alias match picked up from an
        // earlier, unrelated row that merely shares the same canonical name
        // (issue #8011 review: aliasing broke this without a two-pass find).
        let mappings = vec![
            FieldMapping {
                source_type: "char".into(),
                target_type: "binary".into(),
                param_strategy: ParamStrategy::Preserve,
                custom_params: None,
            },
            FieldMapping {
                source_type: "character".into(),
                target_type: "text".into(),
                param_strategy: ParamStrategy::Preserve,
                custom_params: None,
            },
        ];
        assert_eq!(FieldMapping::apply(&mappings, "character"), Some("text"), "exact row must win over the char alias");
        assert_eq!(
            FieldMapping::apply(&mappings, "char"),
            Some("binary"),
            "exact row must win over the character alias"
        );
    }

    #[test]
    fn with_known_length_splices_reported_length_into_bare_type() {
        assert_eq!(with_known_length("varchar", Some(1000)), "varchar(1000)");
        assert_eq!(with_known_length("character varying", None), "character varying");
        assert_eq!(with_known_length("varchar(50)", Some(1000)), "varchar(50)", "explicit params are never overridden");
        assert_eq!(with_known_length("varchar", Some(0)), "varchar", "non-positive length is ignored");
        assert_eq!(with_known_length("varchar", Some(-1)), "varchar", "negative length (unbounded marker) is ignored");
    }

    #[test]
    fn with_known_length_ignores_non_length_bearing_types() {
        // `character_maximum_length` is populated by several drivers for
        // columns where it does not mean "declared length here" — MySQL's
        // information_schema fills it in for TEXT/BLOB (byte capacity, e.g.
        // TEXT -> 65535), and Oracle's DATA_LENGTH is filled in for every
        // column, DATE and NUMBER included (issue #8011 review round 2).
        // Splicing those in would silently reinterpret the type (MySQL turns
        // `TEXT(65535)` into MEDIUMTEXT) or produce invalid DDL (`DATE(7)`).
        assert_eq!(with_known_length("text", Some(65535)), "text");
        assert_eq!(with_known_length("date", Some(7)), "date");
        assert_eq!(with_known_length("number", Some(22)), "number");
    }

    #[test]
    fn with_known_length_restores_a_real_char_length() {
        // Unlike type_rewrite's *default*-to-255 list (which must exclude
        // CHAR — a bare CHAR is already valid, meaning CHAR(1)), this
        // function *restores* a length the driver already knows: MySQL's
        // information_schema reports a CHAR(10) column as DATA_TYPE="char"
        // with CHARACTER_MAXIMUM_LENGTH=10 (real DB verified). Using
        // length 1 here would make this assertion pass even with CHAR
        // wrongly excluded — CHAR(1) and bare CHAR mean the same thing — so
        // this deliberately uses a length where truncation would show up
        // (issue #8011 review round 3).
        assert_eq!(with_known_length("char", Some(10)), "char(10)");
        assert_eq!(with_known_length("character", Some(10)), "character(10)");
        assert_eq!(with_known_length("char", None), "char", "no known length means no invented one either");
    }

    #[test]
    fn mysql_same_dialect_add_column_keeps_text_type_when_length_metadata_is_present() {
        // MySQL's own information_schema reports a real character_maximum_length
        // for TEXT (65535) even though COLUMN_TYPE never carries it in
        // parentheses. Splicing it in verbatim would have this same-dialect
        // ADD COLUMN silently reinterpreted as MEDIUMTEXT by the server (real
        // MySQL 8.4.6 verified) instead of staying TEXT.
        let mut source_col = column("notes", "text", None);
        source_col.character_maximum_length = Some(65535);
        let diff = ColumnDiff {
            diff_type: "added".to_string(),
            name: "notes".to_string(),
            source: Some(source_col),
            target: None,
            changes: vec![],
            add_position: None,
        };
        let sql = gen_sql(wrap_table_diff("t", vec![diff]), DatabaseType::Mysql, Some(DialectKind::Mysql));
        assert!(sql.contains("text"), "expected the column to stay TEXT: {sql}");
        assert!(!sql.contains("(65535)"), "must not splice TEXT's byte capacity in as a length: {sql}");
    }

    #[test]
    fn detects_changed_common_mysql_view_definitions() {
        let options = common_mysql_view_options(
            Some("CREATE ALGORITHM=UNDEFINED DEFINER=`viewer_a`@`%` SQL SECURITY DEFINER VIEW `source_db`.`active_orders` AS select `source_db`.`orders`.`id` AS `id` from `source_db`.`orders` where (`source_db`.`orders`.`active` = 1)"),
            Some("CREATE ALGORITHM=UNDEFINED DEFINER=`viewer_b`@`%` SQL SECURITY DEFINER VIEW `target_db`.`active_orders` AS select `target_db`.`orders`.`id` AS `id` from `target_db`.`orders` where (`target_db`.`orders`.`active` = 0)"),
        );

        let result = prepare_schema_diff(options);

        assert_eq!(result.diffs.len(), 1);
        let diff = &result.diffs[0];
        assert_eq!(diff.diff_type, "modified");
        assert_eq!(diff.object_type.as_deref(), Some("view"));
        assert!(diff.ddl.as_deref().is_some_and(|ddl| ddl.contains("active` = 1")));
        assert!(diff.target_ddl.as_deref().is_some_and(|ddl| ddl.contains("active` = 0")));
        assert!(diff.sync_sql.is_none());
        assert!(result.sync_sql.is_empty());
    }

    #[test]
    fn ignores_mysql_view_environment_and_formatting_differences() {
        let result = prepare_schema_diff(common_mysql_view_options(
            Some("CREATE  ALGORITHM = UNDEFINED DEFINER = `viewer_a` @ `%` SQL SECURITY DEFINER VIEW `source_db` . `active_orders` AS select `source_db` . `orders` . `id` from `source_db` . `orders` where ( `source_db` . `orders` . `active` = 1 )"),
            Some("CREATE ALGORITHM=UNDEFINED DEFINER=`viewer_b`@`localhost` SQL SECURITY DEFINER VIEW `target_db`.`active_orders` AS select `target_db`.`orders`.`id` from `target_db`.`orders` where(`target_db`.`orders`.`active`=1)"),
        ));

        assert!(result.diffs.is_empty());
        assert!(result.sync_sql.is_empty());
    }

    #[test]
    fn preserves_mysql_view_literal_contents_during_comparison() {
        let common =
            "CREATE VIEW `source_db`.`active_orders` AS SELECT 'source_db.orders', 'a b' FROM `source_db`.`orders`";
        let changed_schema_literal =
            "CREATE VIEW `target_db`.`active_orders` AS SELECT 'target_db.orders', 'a b' FROM `target_db`.`orders`";
        let changed_literal_whitespace =
            "CREATE VIEW `target_db`.`active_orders` AS SELECT 'source_db.orders', 'a  b' FROM `target_db`.`orders`";

        assert!(view_definitions_differ(
            common,
            changed_schema_literal,
            Some(DialectKind::Mysql),
            Some(DialectKind::Mysql)
        ));
        assert!(view_definitions_differ(
            common,
            changed_literal_whitespace,
            Some(DialectKind::Mysql),
            Some(DialectKind::Mysql)
        ));
    }

    #[test]
    fn preserves_mysql_view_compound_operator_semantics() {
        let compact = "CREATE VIEW `app`.`active_orders` AS SELECT 1 <=> 1, 1 <= 2, 1 != 2";
        let split = "CREATE VIEW `app`.`active_orders` AS SELECT 1 < = > 1, 1 < = 2, 1 ! = 2";

        assert!(view_definitions_differ(compact, split, Some(DialectKind::Mysql), Some(DialectKind::Mysql)));
    }

    #[test]
    fn preserves_mysql_view_options_and_identifier_case() {
        let ddl = "CREATE ALGORITHM=MERGE SQL SECURITY INVOKER VIEW `app`.`active_orders` AS SELECT `OrderId` FROM `app`.`orders` WITH CASCADED CHECK OPTION";

        for changed in [
            ddl.replace("ALGORITHM=MERGE", "ALGORITHM=TEMPTABLE"),
            ddl.replace("SECURITY INVOKER", "SECURITY DEFINER"),
            ddl.replace("`OrderId`", "`orderid`"),
            ddl.replace("CASCADED", "LOCAL"),
        ] {
            assert!(view_definitions_differ(ddl, &changed, Some(DialectKind::Mysql), Some(DialectKind::Mysql)));
        }
    }

    #[test]
    fn sharded_diff_keeps_common_mysql_view_comparison() {
        let source_ddl = "CREATE VIEW `source_db`.`active_orders` AS SELECT 1";
        let target_ddl = "CREATE VIEW `target_db`.`active_orders` AS SELECT 2";
        let mut options = common_mysql_view_options(Some(source_ddl), Some(target_ddl));
        options.source_tables.push(table_info("other_view", "VIEW"));
        options.target_tables.push(table_info("other_view", "VIEW"));
        options.source_details.push(schema_detail("other_view", Some("CREATE VIEW other_view AS SELECT 1")));
        options.target_details.push(schema_detail("other_view", Some("CREATE VIEW other_view AS SELECT 1")));
        options.shard_strategy = Some(ShardStrategy { shard_count: 2, shard_by: ShardBy::RoundRobin });

        let result = prepare_schema_diff(options);

        assert_eq!(result.diffs.len(), 1);
        assert_eq!(result.diffs[0].name, "active_orders");
        assert_eq!(result.diffs[0].diff_type, "modified");
    }

    #[test]
    fn keeps_added_removed_views_and_table_view_name_boundaries() {
        let result = prepare_schema_diff(SchemaDiffPreparationOptions {
            source_tables: vec![table_info("source_view", "VIEW"), table_info("same_name", "BASE TABLE")],
            target_tables: vec![table_info("target_view", "VIEW"), table_info("same_name", "VIEW")],
            source_details: vec![
                schema_detail("source_view", Some("CREATE VIEW source_view AS SELECT 1")),
                schema_detail("same_name", Some("CREATE TABLE same_name (id int)")),
            ],
            target_details: vec![
                schema_detail("target_view", Some("CREATE VIEW target_view AS SELECT 1")),
                schema_detail("same_name", Some("CREATE VIEW same_name AS SELECT 1")),
            ],
            database_type: DatabaseType::Mysql,
            source_dialect: Some(DialectKind::Mysql),
            target_dialect: Some(DialectKind::Mysql),
            ..Default::default()
        });

        assert!(result.diffs.iter().any(|diff| diff.name == "source_view"
            && diff.diff_type == "added"
            && diff.object_type.as_deref() == Some("view")));
        assert!(result.diffs.iter().any(|diff| diff.name == "target_view"
            && diff.diff_type == "removed"
            && diff.object_type.as_deref() == Some("view")));
        assert!(result.diffs.iter().any(|diff| diff.name == "same_name"
            && diff.diff_type == "added"
            && diff.object_type.as_deref() == Some("table")));
        assert!(result.diffs.iter().any(|diff| diff.name == "same_name"
            && diff.diff_type == "removed"
            && diff.object_type.as_deref() == Some("view")));
    }

    #[test]
    fn mysql_sync_sql_uses_same_dialect_view_ddl() {
        let view_diff = TableDiff {
            diff_type: "added".into(),
            object_type: Some("view".into()),
            name: "active_users".into(),
            target_name: None,
            ddl: Some("CREATE VIEW `active_users` AS SELECT `id` FROM `users` WHERE `active` = 1;".into()),
            ..TableDiff::default()
        };

        let sql = generate_schema_sync_sql(
            &[view_diff],
            &[],
            &[],
            &[],
            &[],
            DatabaseType::Mysql,
            Some("app"),
            false,
            Some(DialectKind::Mysql),
            &[],
        );

        assert!(sql.contains("CREATE VIEW `active_users`"), "{sql}");
        assert!(!sql.contains("Source view definition is not available"), "{sql}");
        assert!(!sql.contains(";;"), "{sql}");
    }

    #[test]
    fn same_dialect_sync_sql_keeps_diagnostic_without_view_ddl() {
        let view_diff = TableDiff {
            diff_type: "added".into(),
            object_type: Some("view".into()),
            name: "active_users".into(),
            target_name: None,
            ..TableDiff::default()
        };

        let sql = generate_schema_sync_sql(
            &[view_diff],
            &[],
            &[],
            &[],
            &[],
            DatabaseType::Mysql,
            Some("app"),
            false,
            Some(DialectKind::Mysql),
            &[],
        );

        assert!(sql.contains("Source view definition is not available from this driver yet"), "{sql}");
    }

    #[test]
    fn mysql_skips_function_sequence_when_templates_absent() {
        let fn_diff = FunctionDiff {
            diff_type: "added".into(),
            name: "f1".into(),
            source: Some(FunctionInfo {
                name: "f1".into(),
                function_type: "FUNCTION".into(),
                data_type: "int".into(),
                definition: "RETURNS int RETURN 1".into(),
                arguments: "".into(),
            }),
            target: None,
            changes: vec![],
        };
        let sql = generate_schema_sync_sql(&[], &[fn_diff], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);
        assert!(sql.contains("-- Skip function f1"), "{sql}");
        assert!(!sql.contains("CREATE FUNCTION"), "{sql}");
    }

    fn added_table_diff(name: &str) -> TableDiff {
        TableDiff {
            diff_type: "added".into(),
            object_type: Some("table".into()),
            name: name.into(),
            ddl: Some(format!("CREATE TABLE `{name}` (\n  `id` int NOT NULL\n)")),
            ..Default::default()
        }
    }

    fn added_table_diff_referencing(name: &str, parent: &str) -> TableDiff {
        TableDiff {
            foreign_keys: Some(vec![ForeignKeyDiff {
                diff_type: "added".into(),
                name: format!("fk_{name}"),
                source: Some(ForeignKeyInfo {
                    name: format!("fk_{name}"),
                    column: "parent_id".into(),
                    ref_schema: None,
                    ref_table: parent.into(),
                    ref_column: "id".into(),
                    on_update: None,
                    on_delete: None,
                }),
                target: None,
                changes: Vec::new(),
            }]),
            ..added_table_diff(name)
        }
    }

    fn removed_table_diff_referencing(name: &str, parent: &str) -> TableDiff {
        TableDiff {
            diff_type: "removed".into(),
            object_type: Some("table".into()),
            name: name.into(),
            foreign_keys: Some(vec![ForeignKeyDiff {
                diff_type: "removed".into(),
                name: format!("fk_{name}"),
                source: None,
                target: Some(ForeignKeyInfo {
                    name: format!("fk_{name}"),
                    column: "parent_id".into(),
                    ref_schema: None,
                    ref_table: parent.into(),
                    ref_column: "id".into(),
                    on_update: None,
                    on_delete: None,
                }),
                changes: Vec::new(),
            }]),
            target_ddl: Some(format!("CREATE TABLE `{name}` (`id` int NOT NULL, `parent_id` int)")),
            ..Default::default()
        }
    }

    fn statement_position(sql: &str, marker: &str) -> usize {
        sql.find(marker).unwrap_or_else(|| panic!("missing {marker} in:\n{sql}"))
    }

    #[test]
    fn added_foreign_key_child_is_deployed_after_its_parent() {
        let diffs = vec![added_table_diff_referencing("child9761", "parent9761"), added_table_diff("parent9761")];

        let sql = generate_schema_sync_sql(&diffs, &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);

        assert!(
            statement_position(&sql, "-- Create table: parent9761")
                < statement_position(&sql, "-- Create table: child9761"),
            "parents must be created before the tables that reference them:\n{sql}"
        );
    }

    #[test]
    fn removed_foreign_key_child_is_dropped_before_its_parent() {
        let diffs = vec![
            removed_table_diff_referencing("zzz_child9761", "aaa_parent9761"),
            TableDiff {
                diff_type: "removed".into(),
                object_type: Some("table".into()),
                name: "aaa_parent9761".into(),
                target_ddl: Some("CREATE TABLE `aaa_parent9761` (`id` int NOT NULL)".into()),
                ..Default::default()
            },
        ];

        let sql = generate_schema_sync_sql(&diffs, &[], &[], &[], &[], DatabaseType::Mysql, None, true, None, &[]);

        assert!(
            statement_position(&sql, "-- Drop table: zzz_child9761")
                < statement_position(&sql, "-- Drop table: aaa_parent9761"),
            "referencing tables must be dropped first:\n{sql}"
        );
    }

    #[test]
    fn added_view_is_created_after_the_table_it_reads() {
        let view = TableDiff {
            diff_type: "added".into(),
            object_type: Some("view".into()),
            name: "aaa_view9761".into(),
            ddl: Some("CREATE VIEW `aaa_view9761` AS SELECT `id` FROM `zzz_table9761`".into()),
            ..Default::default()
        };
        let diffs = vec![view, added_table_diff("zzz_table9761")];

        let sql = generate_schema_sync_sql(&diffs, &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);

        assert!(
            statement_position(&sql, "-- Create table: zzz_table9761")
                < statement_position(&sql, "-- Create view: aaa_view9761"),
            "views must be created after the tables they read:\n{sql}"
        );
    }

    #[test]
    fn independent_diffs_keep_their_original_statement_order() {
        let diffs = vec![added_table_diff("bbb9761"), added_table_diff("aaa9761"), added_table_diff("ccc9761")];

        let sql = generate_schema_sync_sql(&diffs, &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);

        assert!(
            statement_position(&sql, "-- Create table: bbb9761") < statement_position(&sql, "-- Create table: aaa9761")
                && statement_position(&sql, "-- Create table: aaa9761")
                    < statement_position(&sql, "-- Create table: ccc9761"),
            "plans without dependencies must keep the caller's order:\n{sql}"
        );
    }

    #[test]
    fn modified_table_fk_added_to_new_table_waits_for_its_create() {
        let modified = TableDiff {
            diff_type: "modified".into(),
            object_type: Some("table".into()),
            name: "aaa_child_mod9761".into(),
            foreign_keys: Some(vec![ForeignKeyDiff {
                diff_type: "added".into(),
                name: "fk_aaa_child_mod9761".into(),
                source: Some(ForeignKeyInfo {
                    name: "fk_aaa_child_mod9761".into(),
                    column: "parent_id".into(),
                    ref_schema: None,
                    ref_table: "zzz_parent9761".into(),
                    ref_column: "id".into(),
                    on_update: None,
                    on_delete: None,
                }),
                target: None,
                changes: Vec::new(),
            }]),
            ..Default::default()
        };
        let diffs = vec![modified, added_table_diff("zzz_parent9761")];

        let sql = generate_schema_sync_sql(&diffs, &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);

        assert!(
            statement_position(&sql, "-- Create table: zzz_parent9761")
                < statement_position(&sql, "ALTER TABLE `aaa_child_mod9761` ADD CONSTRAINT"),
            "an ALTER on a modified table adding an FK to a new table must follow that table's CREATE:\n{sql}"
        );
    }

    #[test]
    fn modified_table_fk_dropped_runs_before_referenced_table_drop() {
        let modified = TableDiff {
            diff_type: "modified".into(),
            object_type: Some("table".into()),
            name: "aaa_child_mod9761".into(),
            foreign_keys: Some(vec![ForeignKeyDiff {
                diff_type: "removed".into(),
                name: "fk_aaa_child_mod9761".into(),
                source: None,
                target: Some(ForeignKeyInfo {
                    name: "fk_aaa_child_mod9761".into(),
                    column: "parent_id".into(),
                    ref_schema: None,
                    ref_table: "zzz_parent9761".into(),
                    ref_column: "id".into(),
                    on_update: None,
                    on_delete: None,
                }),
                changes: Vec::new(),
            }]),
            ..Default::default()
        };
        let removed_parent = TableDiff {
            diff_type: "removed".into(),
            object_type: Some("table".into()),
            name: "zzz_parent9761".into(),
            ..Default::default()
        };
        let diffs = vec![modified, removed_parent];

        let sql = generate_schema_sync_sql(&diffs, &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);

        assert!(
            statement_position(&sql, "ALTER TABLE `aaa_child_mod9761` DROP FOREIGN KEY")
                < statement_position(&sql, "-- Drop table: zzz_parent9761"),
            "an ALTER dropping an FK must run before the referenced table's DROP:\n{sql}"
        );
    }

    #[test]
    fn circular_foreign_keys_still_emit_every_create_statement() {
        let diffs = vec![
            added_table_diff_referencing("loop_a9761", "loop_b9761"),
            added_table_diff_referencing("loop_b9761", "loop_a9761"),
        ];

        let sql = generate_schema_sync_sql(&diffs, &[], &[], &[], &[], DatabaseType::Mysql, None, false, None, &[]);

        assert!(sql.contains("-- Create table: loop_a9761"), "{sql}");
        assert!(sql.contains("-- Create table: loop_b9761"), "{sql}");
    }

    fn typed_column(
        name: &str,
        data_type: &str,
        numeric_precision: Option<i32>,
        numeric_scale: Option<i32>,
        character_maximum_length: Option<i32>,
    ) -> ColumnInfo {
        ColumnInfo {
            is_nullable: true,
            numeric_precision,
            numeric_scale,
            character_maximum_length,
            ..column(name, data_type, None)
        }
    }

    fn named_index(name: &str, columns: &[&str], is_unique: bool) -> IndexInfo {
        index(IndexInfo {
            name: name.to_string(),
            columns: columns.iter().map(|column| (*column).to_string()).collect(),
            is_unique,
            is_primary: false,
            filter: None,
            index_type: Some("NORMAL".to_string()),
            included_columns: None,
            comment: None,
            key_is_expression: Vec::new(),
            column_opclasses: Vec::new(),
            key_options: Vec::new(),
            constraint_backed: false,
        })
    }

    /// MySQL 的 `int` 也会带 numeric_precision，但 `int(10)` 不是它的声明类型（跨方言会
    /// 变成非法 DDL），所以整数族不在拼接白名单里。
    #[test]
    fn integer_precision_metadata_is_not_spliced_into_the_type() {
        let result = prepare_schema_diff(SchemaDiffPreparationOptions {
            source_tables: vec![table_info("t", "TABLE")],
            target_tables: vec![table_info("t", "TABLE")],
            source_details: vec![TableSchemaDetail {
                name: "t".to_string(),
                columns: vec![typed_column("id", "int", Some(10), Some(0), None)],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
                triggers: Vec::new(),
                ddl: None,
            }],
            target_details: vec![TableSchemaDetail {
                name: "t".to_string(),
                columns: vec![typed_column("id", "int", Some(10), Some(0), None)],
                indexes: Vec::new(),
                foreign_keys: Vec::new(),
                triggers: Vec::new(),
                ddl: None,
            }],
            database_type: DatabaseType::Mysql,
            ..Default::default()
        });

        assert!(result.diffs.is_empty(), "{:?}", result.diffs);
        assert_eq!(declared_column_type(&typed_column("id", "int", Some(10), Some(0), None)), "int");
    }
}
