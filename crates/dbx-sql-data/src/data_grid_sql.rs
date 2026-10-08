use chrono::{Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

use crate::models::connection::DatabaseType;
use crate::sql_dialect::{
    firebird_rows_clause, quote_table_identifier, table_pagination_strategy, uses_single_row_insert_statements,
    uses_synthetic_row_id, uses_xugu_row_id, TablePaginationStrategy,
};
use crate::value_literals::{format_ch_array_sql_literal, format_pg_array_sql_literal};
use dbx_types::types::is_opaque_aggregate_state_type;

const DBX_ROWID_COLUMN: &str = "__DBX_ROWID";
pub const DBX_NEO4J_ELEMENT_ID_COLUMN: &str = "__DBX_ELEMENT_ID";
pub const DBX_TDENGINE_TBNAME_COLUMN: &str = "tbname";
const DATA_GRID_COLUMN_DISTINCT_VALUES_DEFAULT_LIMIT: usize = 1000;
const DATA_GRID_COLUMN_DISTINCT_VALUES_MAX_LIMIT: usize = 1000;
/// Alias for the single cell a keyless guard query returns.
const KEYLESS_GUARD_COUNT_ALIAS: &str = "dbx_keyless_row_matches";
const KEYLESS_AMBIGUOUS_ROW_ERROR: &str = "Cannot safely update or delete this row: the table has no primary key, so the row is identified by matching every column value, and more than one row in the table matches that condition. Add a primary key or unique index, or make the rows distinguishable, before editing.";
const KEYLESS_UNIDENTIFIABLE_ROW_ERROR: &str = "Cannot safely update or delete this row: the table has no primary key and none of the result columns map to a table column, so there is no condition that can target a single row. Add a primary key, or edit the table directly, before saving.";

const MYSQL_DATA_GRID_BATCH_MAX_ROWS: usize = 500;
const MYSQL_DATA_GRID_BATCH_TARGET_SQL_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct DataGridTableMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
    /// Doris / StarRocks multi-catalog: the database under the external
    /// catalog, used as the middle segment of the 3-part qualified name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table_name: String,
    #[serde(default)]
    pub primary_keys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<DataGridColumnInfo>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DataGridColumnInfo {
    pub name: String,
    #[serde(default)]
    pub data_type: String,
    #[serde(default)]
    pub is_nullable: bool,
    #[serde(default)]
    pub is_primary_key: bool,
    #[serde(default)]
    pub column_default: Option<String>,
    #[serde(default)]
    pub extra: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridSaveStatementOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
    /// Server version reported by the connection (for example `Neo4j/4.4.44`). Neo4j renamed the
    /// node identity function in 5.0, so the saved statements must address rows the same way the
    /// grid read them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    pub table_meta: DataGridTableMeta,
    pub columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_columns: Option<Vec<Option<String>>>,
    #[serde(default)]
    pub rows: Vec<Vec<Value>>,
    #[serde(default)]
    pub dirty_rows: Vec<(usize, Vec<(usize, Value)>)>,
    #[serde(default)]
    pub deleted_rows: Vec<usize>,
    #[serde(default)]
    pub new_rows: Vec<Vec<Value>>,
    /// `生成 SQL 时包含数据库名`: qualify the saved table with its database on
    /// engines that address tables as `database.table` (MySQL family,
    /// ClickHouse). Off by default so existing statements are unchanged.
    #[serde(default)]
    pub include_database_name: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridCopyUpdateStatementOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
    pub table_meta: DataGridTableMeta,
    pub columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_columns: Option<Vec<Option<String>>>,
    #[serde(default)]
    pub rows: Vec<Vec<Value>>,
    /// `生成 SQL 时包含数据库名`: qualify the copied table the same way the
    /// save statements and the copy-as-INSERT statements do. Defaults to true so
    /// callers that never strip the table metadata keep the historical shape.
    #[serde(default = "default_include_database_name")]
    pub include_database_name: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridCopyInsertStatementOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_meta: Option<DataGridTableMeta>,
    pub columns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_types: Option<Vec<Option<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_columns: Option<Vec<Option<String>>>,
    #[serde(default)]
    pub rows: Vec<Vec<Value>>,
    #[serde(default)]
    pub exclude_primary_keys: bool,
    #[serde(default)]
    pub include_computed_columns: bool,
    #[serde(default = "default_include_database_name")]
    pub include_database_name: bool,
    #[serde(default)]
    pub insert_mode: DataGridCopyInsertMode,
}

fn default_include_database_name() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "kebab-case")]
pub enum DataGridCopyInsertMode {
    #[default]
    Merged,
    RowByRow,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DataGridContextFilterMode {
    Equals,
    NotEquals,
    IsNull,
    IsNotNull,
    IsBlank,
    IsNotBlank,
    Like,
    NotLike,
    BeginsWith,
    EndsWith,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
    In,
    NotIn,
    Between,
    NotBetween,
}

fn supports_data_grid_context_filter_mode(
    database_type: Option<DatabaseType>,
    mode: DataGridContextFilterMode,
) -> bool {
    {}
    !false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridContextFilterConditionOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
    pub column_name: String,
    pub mode: DataGridContextFilterMode,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_info: Option<DataGridColumnInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridColumnValueFilterConditionOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
    pub column_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_info: Option<DataGridColumnInfo>,
    pub raw_value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridColumnValuesFilterConditionOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
    pub column_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_info: Option<DataGridColumnInfo>,
    #[serde(default)]
    pub values: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridColumnDistinctValuesSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table_name: String,
    pub column_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column_info: Option<DataGridColumnInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub where_input: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_counts: bool,
    #[serde(default)]
    pub exclude_nulls: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridCountSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog: Option<String>,
    /// Doris / StarRocks multi-catalog: the database under the external
    /// catalog, used as the middle segment of the 3-part qualified name when
    /// `schema` is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub table_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub where_input: Option<String>,
    /// Optional optimizer hint injected between SELECT and the select list.
    /// Example: "/*+ set(query_dop 32) */" for GaussDB parallel COUNT(*).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridConditionalUpdateSqlOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_type: Option<DatabaseType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier_quote: Option<String>,
    pub table_meta: DataGridTableMeta,
    pub column_name: String,
    pub value: Value,
    pub where_input: String,
}

/// A server-side check that must pass before a keyless save may run.
///
/// Without a primary key a row is identified by matching every column value,
/// so the same predicate can match physical rows the loaded page never saw.
/// `sql` counts, on the server, how many rows one of the predicates this save
/// actually sends to the database matches. The save must be refused with
/// `message` unless the returned count is at most `max_matched_rows`, and also
/// refused when the count cannot be obtained at all — an unverified keyless
/// write is exactly the ambiguous write this guard exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridSaveGuard {
    pub sql: String,
    pub max_matched_rows: u32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataGridSavePreparation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_error: Option<String>,
    pub statements: Vec<String>,
    pub rollback_statements: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execution_schema: Option<String>,
    /// Checks the caller must run — and pass — before executing `statements`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyless_guards: Vec<DataGridSaveGuard>,
}

pub fn prepare_data_grid_save(options: DataGridSaveStatementOptions) -> DataGridSavePreparation {
    prepare_data_grid_save_for_driver_profile(options, None)
}

pub fn prepare_data_grid_save_for_driver_profile(
    options: DataGridSaveStatementOptions,
    driver_profile: Option<&str>,
) -> DataGridSavePreparation {
    let validation_error = validate_data_grid_save(&options);
    if validation_error.is_some() {
        return DataGridSavePreparation {
            validation_error,
            statements: Vec::new(),
            rollback_statements: Vec::new(),
            execution_schema: data_grid_save_execution_schema(
                options.database_type,
                driver_profile,
                &options.table_meta,
            ),
            keyless_guards: Vec::new(),
        };
    }

    let mut keyless_guards = Vec::new();
    {}
    let statements = build_data_grid_save_statements(&options, driver_profile, &mut keyless_guards);
    DataGridSavePreparation {
        validation_error: None,
        statements,
        rollback_statements: build_data_grid_rollback_statements(&options, driver_profile),
        execution_schema: data_grid_save_execution_schema(options.database_type, driver_profile, &options.table_meta),
        keyless_guards,
    }
}

/// Relational SQL UPDATE/WHERE predicates are not meaningful for graph,
/// document, or time-series stores that don't speak relational SQL.
pub fn supports_relational_copy_predicates(database_type: Option<DatabaseType>) -> bool {
    !false
}

pub fn build_data_grid_copy_update_statements(options: DataGridCopyUpdateStatementOptions) -> Vec<String> {
    if !supports_relational_copy_predicates(options.database_type) {
        return Vec::new();
    }
    let primary_keys = &options.table_meta.primary_keys;
    if primary_keys.is_empty() {
        return Vec::new();
    }

    let save_columns = effective_copy_columns(options.source_columns.as_deref(), &options.columns);
    let column_info = options.table_meta.columns.as_deref().unwrap_or(&[]);
    let primary_key_indexes: Vec<Option<usize>> = primary_keys
        .iter()
        .map(|primary_key| find_column_index(options.database_type, &save_columns, primary_key))
        .collect();
    if primary_key_indexes.iter().any(Option::is_none) {
        return Vec::new();
    }
    let primary_key_indexes: Vec<usize> = primary_key_indexes.into_iter().flatten().collect();
    let primary_key_set: Vec<String> =
        primary_keys.iter().map(|primary_key| normalize_column_name(primary_key)).collect();
    let writable_indexes: Vec<(&str, usize, Option<&DataGridColumnInfo>)> = save_columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| Some((column.as_deref()?, index)))
        .filter(|(column, _)| !primary_key_set.contains(&normalize_column_name(column)))
        .filter(|(column, _)| !is_synthetic_row_id(options.database_type, Some(column)))
        .map(|(column, index)| (column, index, column_info_for(column_info, column)))
        .collect();
    let primary_key_info =
        primary_keys.iter().map(|primary_key| column_info_for(column_info, primary_key)).collect::<Vec<_>>();

    if writable_indexes
        .iter()
        .any(|(_, _, info)| info.is_some_and(|info| is_opaque_aggregate_state_type(&info.data_type)))
        || primary_key_info.iter().any(|info| info.is_some_and(|info| is_opaque_aggregate_state_type(&info.data_type)))
    {
        return Vec::new();
    }

    if writable_indexes.is_empty() {
        return Vec::new();
    }

    let table = data_grid_generated_table_name(
        options.database_type,
        options.table_meta.catalog.as_deref(),
        options.table_meta.schema.as_deref(),
        options.table_meta.database.as_deref(),
        &options.table_meta.table_name,
        options.identifier_quote.as_deref(),
        options.include_database_name,
    );
    let mut statements = Vec::new();
    for row in &options.rows {
        if primary_key_indexes.iter().any(|index| row.get(*index).unwrap_or(&Value::Null).is_null()) {
            continue;
        }
        let sets = writable_indexes
            .iter()
            .map(|(column, index, info)| {
                format!(
                    "{} = {}",
                    data_grid_identifier(options.database_type, column, options.identifier_quote.as_deref()),
                    format_grid_assignment_sql_literal(
                        row.get(*index).unwrap_or(&Value::Null),
                        options.database_type,
                        *info,
                        options.identifier_quote.as_deref(),
                    )
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        if sets.is_empty() {
            continue;
        }
        let where_clause = primary_keys
            .iter()
            .enumerate()
            .map(|(index, primary_key)| {
                build_column_predicate(
                    options.database_type,
                    primary_key,
                    row.get(primary_key_indexes[index]).unwrap_or(&Value::Null),
                    primary_key_info[index],
                    false,
                    options.identifier_quote.as_deref(),
                )
            })
            .collect::<Vec<_>>()
            .join(" AND ");
        statements.push(data_grid_statement(
            options.database_type,
            data_grid_update_sql(options.database_type, &table, &sets, &where_clause),
        ));
    }
    statements
}

pub fn build_data_grid_copy_insert_statement(options: DataGridCopyInsertStatementOptions) -> Option<String> {
    build_data_grid_copy_insert_statement_with_formatters(
        options,
        |reference| reference,
        format_grid_copy_insert_sql_literal,
    )
}

pub(crate) fn build_data_grid_copy_insert_statement_with_formatters(
    options: DataGridCopyInsertStatementOptions,
    format_reference: impl Fn(String) -> String,
    format_literal: impl Fn(&Value, Option<DatabaseType>, Option<&DataGridColumnInfo>, Option<&str>) -> String,
) -> Option<String> {
    let save_columns = effective_copy_columns(options.source_columns.as_deref(), &options.columns);
    let column_info = options.table_meta.as_ref().and_then(|meta| meta.columns.as_deref()).unwrap_or(&[]);
    let primary_key_set: Vec<String> = options
        .table_meta
        .as_ref()
        .map(|meta| meta.primary_keys.iter().map(|primary_key| normalize_column_name(primary_key)).collect())
        .unwrap_or_default();
    let insertable_columns: Vec<(&str, usize, Option<DataGridColumnInfo>)> = save_columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| Some((column.as_deref()?, index)))
        .map(|(column, index)| {
            let fallback_type =
                options.column_types.as_deref().and_then(|types| types.get(index)).and_then(|value| value.as_deref());
            (column, index, copy_column_info(column_info, column, fallback_type))
        })
        .filter(|(column, _, info)| {
            !is_grid_insert_omitted_column(
                options.database_type,
                info.as_ref(),
                Some(column),
                options.include_computed_columns,
            )
        })
        .collect();
    // "Exclude primary keys" keeps manually-assigned key columns: dropping a
    // non-auto-generated PK member loses data and yields an INSERT that cannot
    // satisfy NOT NULL. Only auto-generated (auto_increment/identity) keys are
    // excluded, matching DBeaver's SQLGeneratorInsert excludeAutoGeneratedColumn.
    let insert_columns: Vec<(&str, usize, Option<DataGridColumnInfo>)> = insertable_columns
        .iter()
        .filter(|(column, _, info)| {
            !options.exclude_primary_keys
                || !primary_key_set.contains(&normalize_column_name(column))
                || !info.as_ref().is_some_and(is_auto_generated_column)
        })
        .cloned()
        .collect();

    if insert_columns
        .iter()
        .any(|(_, _, info)| info.as_ref().is_some_and(|info| is_opaque_aggregate_state_type(&info.data_type)))
    {
        return None;
    }

    if insert_columns.is_empty() || options.rows.is_empty() {
        return None;
    }

    let table = options.table_meta.as_ref().map_or_else(
        || "table_name".to_string(),
        |meta| {
            // This copy option controls every namespace prefix, including
            // SQLite's main schema and external catalogs.
            if !options.include_database_name {
                return data_grid_identifier(
                    options.database_type,
                    &meta.table_name,
                    options.identifier_quote.as_deref(),
                );
            }
            // MySQL-compatible engines can store the database in either field.
            let use_database_fallback = match options.database_type {
                Some(DatabaseType::Mysql) => true,

                _ => false,
            };
            // MySQL table tabs store the namespace in `database` without a
            // schema. Explicit schemas still take priority for cross-database
            // sources; schema-based engines must not use this fallback.
            let schema = if use_database_fallback {
                meta.schema
                    .as_deref()
                    .filter(|schema| !schema.trim().is_empty())
                    .or_else(|| meta.database.as_deref().filter(|database| !database.trim().is_empty()))
            } else {
                meta.schema.as_deref()
            };
            // ClickHouse (`database.table`) and SQL Server
            // (`database.schema.table`) address tables across databases on the
            // same connection, so the setting resolves the full name for them.
            if let Some(qualified) = crate::sql_dialect::database_qualified_table_name(
                options.database_type,
                meta.catalog.as_deref(),
                schema,
                meta.database.as_deref(),
                &meta.table_name,
            ) {
                return qualified;
            }
            data_grid_qualified_table_name(
                options.database_type,
                meta.catalog.as_deref(),
                schema,
                meta.database.as_deref(),
                &meta.table_name,
                options.identifier_quote.as_deref(),
            )
        },
    );
    let table = format_reference(table);
    let columns = insert_columns
        .iter()
        .map(|(_, index, _)| {
            format_reference(data_grid_identifier(
                options.database_type,
                &options.columns[*index],
                options.identifier_quote.as_deref(),
            ))
        })
        .collect::<Vec<_>>()
        .join(", ");
    let value_rows = options
        .rows
        .iter()
        .map(|row| {
            format!(
                "({})",
                insert_columns
                    .iter()
                    .map(|(_, index, info)| {
                        format_literal(
                            row.get(*index).unwrap_or(&Value::Null),
                            options.database_type,
                            info.as_ref(),
                            options.identifier_quote.as_deref(),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>();
    let statements = if options.insert_mode == DataGridCopyInsertMode::RowByRow
        || options.database_type.is_some_and(uses_single_row_insert_statements)
    {
        value_rows.iter().map(|values| format!("INSERT INTO {table} ({columns}) VALUES {values};")).collect::<Vec<_>>()
    } else {
        vec![format!(
            "INSERT INTO {table} ({columns}) VALUES{}{};",
            if value_rows.len() == 1 { " " } else { "\n" },
            value_rows.join(",\n")
        )]
    };
    // SQL Server and Dameng reject explicit values for identity columns unless the
    // statement runs between `SET IDENTITY_INSERT <table> ON` and `OFF` (SQL Server
    // error 544), so a copied INSERT that carries the identity column must ship the
    // wrapper. Each statement is wrapped on its own so row-by-row copies stay
    // individually executable, matching the SQL export path.
    let needs_identity_insert_wrapper = false;
    {}
    Some(statements.join("\n"))
}

pub fn build_data_grid_context_filter_condition(options: DataGridContextFilterConditionOptions) -> Option<String> {
    if !supports_data_grid_context_filter_mode(options.database_type, options.mode) {
        return None;
    }

    let column = column_filter_ref(options.database_type, &options.column_name, options.identifier_quote.as_deref());
    let like_column = column_like_filter_ref(
        options.database_type,
        &options.column_name,
        options.column_info.as_ref(),
        options.identifier_quote.as_deref(),
    );
    let value = &options.value;
    match options.mode {
        DataGridContextFilterMode::IsNull => Some(format!("{column} IS NULL")),
        DataGridContextFilterMode::IsNotNull => Some(format!("{column} IS NOT NULL")),

        DataGridContextFilterMode::IsBlank => Some(format!("({column} IS NULL OR {column} = '')")),
        DataGridContextFilterMode::IsNotBlank => Some(format!("({column} IS NOT NULL AND {column} <> '')")),
        DataGridContextFilterMode::Equals if value.is_null() => Some(format!("{column} IS NULL")),
        DataGridContextFilterMode::NotEquals if value.is_null() => Some(format!("{column} IS NOT NULL")),
        DataGridContextFilterMode::Like => Some(format!(
            "{like_column} LIKE {}",
            format_grid_sql_literal(
                &Value::String(format!("%{}%", value_to_filter_text(value))),
                options.database_type,
                None
            )
        )),
        DataGridContextFilterMode::NotLike => Some(format!(
            "{like_column} NOT LIKE {}",
            format_grid_sql_literal(
                &Value::String(format!("%{}%", value_to_filter_text(value))),
                options.database_type,
                None
            )
        )),
        DataGridContextFilterMode::BeginsWith => Some(format!(
            "{like_column} LIKE {}",
            format_grid_sql_literal(
                &Value::String(format!("{}%", value_to_filter_text(value))),
                options.database_type,
                None
            )
        )),
        DataGridContextFilterMode::EndsWith => Some(format!(
            "{like_column} LIKE {}",
            format_grid_sql_literal(
                &Value::String(format!("%{}", value_to_filter_text(value))),
                options.database_type,
                None
            )
        )),
        DataGridContextFilterMode::LessThan => Some(format!(
            "{column} < {}",
            format_data_grid_context_filter_literal(
                value,
                options.database_type,
                &options.column_name,
                options.column_info.as_ref(),
                options.identifier_quote.as_deref(),
            )
        )),
        DataGridContextFilterMode::LessThanOrEqual => Some(format!(
            "{column} <= {}",
            format_data_grid_context_filter_literal(
                value,
                options.database_type,
                &options.column_name,
                options.column_info.as_ref(),
                options.identifier_quote.as_deref(),
            )
        )),
        DataGridContextFilterMode::GreaterThan => Some(format!(
            "{column} > {}",
            format_data_grid_context_filter_literal(
                value,
                options.database_type,
                &options.column_name,
                options.column_info.as_ref(),
                options.identifier_quote.as_deref(),
            )
        )),
        DataGridContextFilterMode::GreaterThanOrEqual => Some(format!(
            "{column} >= {}",
            format_data_grid_context_filter_literal(
                value,
                options.database_type,
                &options.column_name,
                options.column_info.as_ref(),
                options.identifier_quote.as_deref(),
            )
        )),
        DataGridContextFilterMode::In => build_data_grid_context_membership_filter_condition(
            &column,
            &options.values,
            options.database_type,
            &options.column_name,
            options.column_info.as_ref(),
            options.identifier_quote.as_deref(),
            false,
        ),
        DataGridContextFilterMode::NotIn => build_data_grid_context_membership_filter_condition(
            &column,
            &options.values,
            options.database_type,
            &options.column_name,
            options.column_info.as_ref(),
            options.identifier_quote.as_deref(),
            true,
        ),
        DataGridContextFilterMode::Between => build_data_grid_context_range_filter_condition(
            &column,
            value,
            options.end_value.as_ref(),
            options.database_type,
            &options.column_name,
            options.column_info.as_ref(),
            options.identifier_quote.as_deref(),
            false,
        ),
        DataGridContextFilterMode::NotBetween => build_data_grid_context_range_filter_condition(
            &column,
            value,
            options.end_value.as_ref(),
            options.database_type,
            &options.column_name,
            options.column_info.as_ref(),
            options.identifier_quote.as_deref(),
            true,
        ),
        DataGridContextFilterMode::Equals => Some(format!(
            "{column} = {}",
            format_data_grid_context_filter_literal(
                value,
                options.database_type,
                &options.column_name,
                options.column_info.as_ref(),
                options.identifier_quote.as_deref(),
            )
        )),
        DataGridContextFilterMode::NotEquals => Some(format!(
            "{column} <> {}",
            format_data_grid_context_filter_literal(
                value,
                options.database_type,
                &options.column_name,
                options.column_info.as_ref(),
                options.identifier_quote.as_deref(),
            )
        )),
    }
}

fn format_data_grid_context_filter_literal(
    value: &Value,
    database_type: Option<DatabaseType>,
    column_name: &str,
    column_info: Option<&DataGridColumnInfo>,
    identifier_quote: Option<&str>,
) -> String {
    {}
    format_grid_sql_literal_with_identifier_quote(value, database_type, column_info, identifier_quote)
}

fn build_data_grid_context_membership_filter_condition(
    column: &str,
    values: &[Value],
    database_type: Option<DatabaseType>,
    column_name: &str,
    column_info: Option<&DataGridColumnInfo>,
    identifier_quote: Option<&str>,
    negated: bool,
) -> Option<String> {
    if values.is_empty() {
        return None;
    }

    let mut has_null = false;
    let mut literals = Vec::new();
    let mut seen_literals = HashSet::new();
    for value in values {
        if value.is_null() {
            has_null = true;
            continue;
        }
        let literal =
            format_data_grid_context_filter_literal(value, database_type, column_name, column_info, identifier_quote);
        if seen_literals.insert(literal.clone()) {
            literals.push(literal);
        }
    }

    let membership = build_membership_predicate(column, &literals, database_type, negated);

    if negated {
        return match membership {
            Some(membership) => Some(format!("({column} IS NOT NULL AND {membership})")),
            None if has_null => Some(format!("{column} IS NOT NULL")),
            None => None,
        };
    }

    match membership {
        Some(membership) if has_null => Some(format!("({column} IS NULL OR {membership})")),
        Some(membership) => Some(membership),
        None if has_null => Some(format!("{column} IS NULL")),
        None => None,
    }
}

fn build_membership_predicate(
    column: &str,
    literals: &[String],
    database_type: Option<DatabaseType>,
    negated: bool,
) -> Option<String> {
    if literals.is_empty() {
        return None;
    }
    {}

    let operator = if negated { "NOT IN" } else { "IN" };
    {
        return Some(format!("{column} {operator} ({})", literals.join(", ")));
    }
}

fn build_data_grid_context_range_filter_condition(
    column: &str,
    start_value: &Value,
    end_value: Option<&Value>,
    database_type: Option<DatabaseType>,
    column_name: &str,
    column_info: Option<&DataGridColumnInfo>,
    identifier_quote: Option<&str>,
    negated: bool,
) -> Option<String> {
    let end_value = end_value?;
    if start_value.is_null() || end_value.is_null() {
        return None;
    }
    let start =
        format_data_grid_context_filter_literal(start_value, database_type, column_name, column_info, identifier_quote);
    let end =
        format_data_grid_context_filter_literal(end_value, database_type, column_name, column_info, identifier_quote);
    {}
    let operator = if negated { "NOT BETWEEN" } else { "BETWEEN" };
    Some(format!("{column} {operator} {start} AND {end}"))
}

pub fn build_data_grid_column_value_filter_condition(
    options: DataGridColumnValueFilterConditionOptions,
) -> Option<String> {
    let text = options.raw_value.trim();
    if text.is_empty() {
        return None;
    }
    let column = column_filter_ref(options.database_type, &options.column_name, options.identifier_quote.as_deref());
    if text.eq_ignore_ascii_case("null") {
        return Some(format!("{column} IS NULL"));
    }
    let value = parse_typed_filter_value(text, options.database_type, options.column_info.as_ref());
    Some(format!("{column} = {}", format_grid_sql_literal(&value, options.database_type, options.column_info.as_ref())))
}

pub fn build_data_grid_column_values_filter_condition(
    options: DataGridColumnValuesFilterConditionOptions,
) -> Option<String> {
    if options.values.is_empty() {
        return None;
    }

    let column = column_filter_ref(options.database_type, &options.column_name, options.identifier_quote.as_deref());
    let mut has_null = false;
    let mut literals = Vec::new();
    let mut seen_literals = HashSet::new();
    for value in &options.values {
        if value.is_null() {
            has_null = true;
            continue;
        }
        let literal = format_grid_sql_literal(value, options.database_type, options.column_info.as_ref());
        if seen_literals.insert(literal.clone()) {
            literals.push(literal);
        }
    }

    let mut predicates = Vec::new();
    if has_null {
        predicates.push(format!("{column} IS NULL"));
    }
    if literals.len() == 1 {
        predicates.push(format!("{column} = {}", literals[0]));
    } else if let Some(membership) = build_membership_predicate(&column, &literals, options.database_type, false) {
        predicates.push(membership);
    }

    match predicates.len() {
        0 => None,
        1 => predicates.into_iter().next(),
        _ => Some(format!("({})", predicates.join(" OR "))),
    }
}

pub fn build_data_grid_column_distinct_values_sql(options: DataGridColumnDistinctValuesSqlOptions) -> String {
    {}

    let limit = data_grid_column_distinct_values_limit(options.limit);
    let table = data_grid_qualified_table_name(
        options.database_type,
        options.catalog.as_deref(),
        options.schema.as_deref(),
        options.database.as_deref(),
        &options.table_name,
        options.identifier_quote.as_deref(),
    );
    let column = column_filter_ref(options.database_type, &options.column_name, options.identifier_quote.as_deref());
    let mut predicates = Vec::new();
    let predicate = crate::sql_dialect::normalize_where_input(options.where_input.as_deref());
    if !predicate.is_empty() {
        predicates.push(format!("({predicate})"));
    }
    if options.exclude_nulls {
        predicates.push(format!("{column} IS NOT NULL"));
    }
    if let Some(search_predicate) = data_grid_column_distinct_values_search_predicate(&options) {
        predicates.push(search_predicate);
    }
    let where_clause =
        if predicates.is_empty() { String::new() } else { format!(" WHERE {}", predicates.join(" AND ")) };
    let select_list = if options.include_counts {
        format!("{column} AS dbx_value, COUNT(*) AS dbx_count")
    } else {
        format!("{column} AS dbx_value")
    };
    let group_by = format!(" GROUP BY {column}");
    let order_by = if options.include_counts { " ORDER BY dbx_count DESC, dbx_value" } else { " ORDER BY dbx_value" };
    let from_clause = format!(" FROM {table}{where_clause}{group_by}{order_by}");

    match table_pagination_strategy(options.database_type) {
        TablePaginationStrategy::LimitOffset => {
            format!("SELECT {select_list}{from_clause} LIMIT {limit}")
        }
    }
}

pub fn build_data_grid_count_sql(options: DataGridCountSqlOptions) -> String {
    {}
    // Keep the reference identical to the one the grid's SELECT uses: Caché/IRIS
    // reject quoted ordinary names when delimited identifiers are disabled, so
    // the count must not be the only statement that quotes them (#8929).
    let table = data_grid_qualified_table_name(
        options.database_type,
        options.catalog.as_deref(),
        options.schema.as_deref(),
        options.database.as_deref(),
        &options.table_name,
        options.identifier_quote.as_deref(),
    );
    let predicate = crate::sql_dialect::normalize_where_input(options.where_input.as_deref());
    let where_clause = if predicate.is_empty() { String::new() } else { format!(" WHERE ({predicate})") };
    let hint = options.count_hint.as_deref().unwrap_or("");
    let hint_part = if hint.is_empty() { String::new() } else { format!(" {hint}") };
    format!("SELECT{hint_part} COUNT(*) AS cnt FROM {table}{where_clause}")
}

pub fn build_data_grid_conditional_update_sql(options: DataGridConditionalUpdateSqlOptions) -> Option<String> {
    if options.database_type != Some(DatabaseType::Mysql) {
        return None;
    }
    let predicate = crate::sql_dialect::normalize_where_input(Some(&options.where_input));
    if predicate.is_empty() {
        return None;
    }

    let columns = options.table_meta.columns.as_deref()?;
    let column_info = column_info_for(columns, &options.column_name)?;
    let primary_key_set: Vec<String> =
        options.table_meta.primary_keys.iter().map(|primary_key| normalize_column_name(primary_key)).collect();
    if primary_key_set.contains(&normalize_column_name(&column_info.name))
        || column_info.is_primary_key
        || is_grid_update_omitted_column(
            options.database_type,
            Some(column_info),
            Some(&column_info.name),
            &primary_key_set,
        )
    {
        return None;
    }

    let table = data_grid_qualified_table_name(
        options.database_type,
        options.table_meta.catalog.as_deref(),
        options.table_meta.schema.as_deref(),
        options.table_meta.database.as_deref(),
        &options.table_meta.table_name,
        options.identifier_quote.as_deref(),
    );
    let sets = format!(
        "{} = {}",
        data_grid_identifier(options.database_type, &column_info.name, options.identifier_quote.as_deref()),
        format_grid_save_sql_literal(
            &options.value,
            options.database_type,
            Some(column_info),
            options.identifier_quote.as_deref(),
        )
    );
    Some(data_grid_statement(
        options.database_type,
        data_grid_update_sql(options.database_type, &table, &sets, &format!("({predicate})")),
    ))
}

fn data_grid_column_distinct_values_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DATA_GRID_COLUMN_DISTINCT_VALUES_DEFAULT_LIMIT).clamp(1, DATA_GRID_COLUMN_DISTINCT_VALUES_MAX_LIMIT)
}

fn data_grid_column_distinct_values_search_predicate(
    options: &DataGridColumnDistinctValuesSqlOptions,
) -> Option<String> {
    let search = options.search_value.as_deref()?.trim();
    if search.is_empty() {
        return None;
    }
    if !options.column_info.as_ref().map(|column| is_textual_column_type(&column.data_type)).unwrap_or(true) && !false {
        let column =
            column_filter_ref(options.database_type, &options.column_name, options.identifier_quote.as_deref());
        let value = parse_typed_filter_value(search, options.database_type, options.column_info.as_ref());
        return Some(format!(
            "{column} = {}",
            format_grid_sql_literal(&value, options.database_type, options.column_info.as_ref())
        ));
    }
    let column = column_like_filter_ref(
        options.database_type,
        &options.column_name,
        options.column_info.as_ref(),
        options.identifier_quote.as_deref(),
    );
    let pattern = Value::String(format!("%{search}%"));
    Some(format!("{column} LIKE {}", format_grid_sql_literal(&pattern, options.database_type, None)))
}

fn validate_data_grid_save(options: &DataGridSaveStatementOptions) -> Option<String> {
    if let Some(error) = validate_opaque_aggregate_state_write(options) {
        return Some(error);
    }
    {}
    {}
    {}
    if let Some(error) = validate_inserted_primary_keys(options) {
        return Some(error);
    }
    {}
    if let Some(error) = validate_existing_row_primary_keys(options) {
        return Some(error);
    }
    {}
    {}
    if let Some(error) = validate_keyless_row_predicate(options) {
        return Some(error);
    }

    let save_columns = effective_columns(options);
    let not_null_columns: Vec<String> = options
        .table_meta
        .columns
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|column| {
            !column.is_nullable
                && column.column_default.is_none()
                && !is_auto_generated_column(column)
                && !is_non_identity_generated_column(Some(column))
                && !is_synthetic_row_id(options.database_type, Some(&column.name))
        })
        .map(|column| normalize_column_name(&column.name))
        .collect();
    {}

    if not_null_columns.is_empty() {
        return None;
    }

    for (_, changes) in &options.dirty_rows {
        for (column_index, value) in changes {
            let source_column = save_columns.get(*column_index).and_then(|column| column.as_deref());
            if is_null_write_to_not_null_column(options.database_type, &not_null_columns, source_column, value) {
                return Some(null_write_error(source_column.unwrap_or_default()));
            }
        }
    }

    // MySQL BEFORE INSERT triggers can populate omitted NOT NULL columns. New-row NULL values are
    // omitted from the generated INSERT, so let MySQL apply triggers or report missing required fields.
    if options.database_type != Some(DatabaseType::Mysql) {
        for row in &options.new_rows {
            for column_index in 0..options.columns.len() {
                let source_column = save_columns.get(column_index).and_then(|column| column.as_deref());
                if is_null_write_to_not_null_column(
                    options.database_type,
                    &not_null_columns,
                    source_column,
                    row.get(column_index).unwrap_or(&Value::Null),
                ) {
                    return Some(null_write_error(source_column.unwrap_or_default()));
                }
            }
        }
    }

    None
}

fn validate_opaque_aggregate_state_write(options: &DataGridSaveStatementOptions) -> Option<String> {
    let save_columns = effective_columns(options);
    let column_info = options.table_meta.columns.as_deref().unwrap_or(&[]);
    let opaque_indexes = save_columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| {
            let column = column.as_deref()?;
            column_info_for(column_info, column)
                .is_some_and(|info| is_opaque_aggregate_state_type(&info.data_type))
                .then_some(index)
        })
        .collect::<HashSet<_>>();
    if opaque_indexes.is_empty() {
        return None;
    }
    if options.dirty_rows.iter().any(|(_, changes)| changes.iter().any(|(index, _)| opaque_indexes.contains(index)))
        || options
            .new_rows
            .iter()
            .any(|row| opaque_indexes.iter().any(|index| row.get(*index).is_some_and(|value| !value.is_null())))
    {
        return Some("Doris aggregate-state columns are opaque and cannot be written automatically; use an explicit Doris state function instead.".to_string());
    }
    if options.table_meta.primary_keys.is_empty()
        && (!options.dirty_rows.is_empty() || !options.deleted_rows.is_empty())
        && options.dirty_rows.iter().map(|(index, _)| *index).chain(options.deleted_rows.iter().copied()).any(
            |row_index| {
                options.rows.get(row_index).is_some_and(|row| {
                    opaque_indexes.iter().any(|index| row.get(*index).is_some_and(|value| !value.is_null()))
                })
            },
        )
    {
        return Some("Cannot safely update or delete a keyless row whose predicate would contain an opaque Doris aggregate-state value.".to_string());
    }
    None
}

fn validate_existing_row_primary_keys(options: &DataGridSaveStatementOptions) -> Option<String> {
    let primary_keys = &options.table_meta.primary_keys;
    if primary_keys.is_empty() || (options.dirty_rows.is_empty() && options.deleted_rows.is_empty()) {
        return None;
    }

    let save_columns = effective_columns(options);
    let primary_key_indexes: Vec<Option<usize>> = primary_keys
        .iter()
        .map(|primary_key| find_column_index(options.database_type, &save_columns, primary_key))
        .collect();
    let missing_primary_keys = primary_keys
        .iter()
        .zip(&primary_key_indexes)
        .filter_map(|(primary_key, index)| index.is_none().then_some(primary_key.as_str()))
        .collect::<Vec<_>>();
    if !missing_primary_keys.is_empty() {
        return Some(format!(
            "Cannot safely update or delete rows because the query result does not include every primary key column (missing: {}). Refresh or rerun the query before saving.",
            missing_primary_keys.join(", ")
        ));
    }

    let primary_key_indexes = primary_key_indexes.into_iter().flatten().collect::<Vec<_>>();
    for row_index in
        options.dirty_rows.iter().map(|(row_index, _)| *row_index).chain(options.deleted_rows.iter().copied())
    {
        let Some(row) = options.rows.get(row_index) else {
            continue;
        };
        if let Some((primary_key, _)) =
            primary_keys.iter().zip(&primary_key_indexes).find(|(_, index)| row.get(**index).is_none_or(Value::is_null))
        {
            return Some(format!(
                "Cannot safely update or delete rows because primary key column \"{primary_key}\" has no value in the query result. Refresh or rerun the query before saving."
            ));
        }
    }

    None
}

/// Without a primary key a row can only be addressed by matching every column
/// value, and `build_row_where` drops columns that have no source column of
/// their own. When nothing is left, the generated predicate is empty and the
/// UPDATE/DELETE would target the whole table, so there is neither a reliable
/// row identifier nor anything a server-side check could count: refuse.
///
/// Whether a non-empty predicate really addresses a single physical row cannot
/// be decided here — the loaded page is not the table. That decision belongs to
/// the `keyless_guards` this preparation emits, which count the matches of the
/// exact predicate on the server.
fn validate_keyless_row_predicate(options: &DataGridSaveStatementOptions) -> Option<String> {
    if !options.table_meta.primary_keys.is_empty() || !uses_keyless_row_predicate(options.database_type) {
        return None;
    }
    let save_columns = effective_columns(options);
    let column_info = options.table_meta.columns.as_deref().unwrap_or(&[]);
    let touched_row_indexes =
        options.dirty_rows.iter().map(|(row_index, _)| *row_index).chain(options.deleted_rows.iter().copied());
    for row_index in touched_row_indexes {
        let Some(row) = options.rows.get(row_index) else {
            continue;
        };
        let predicate = build_row_where(
            options.database_type,
            &save_columns,
            row,
            column_info,
            options.identifier_quote.as_deref(),
        );
        if predicate.trim().is_empty() {
            return Some(KEYLESS_UNIDENTIFIABLE_ROW_ERROR.to_string());
        }
    }
    None
}

fn validate_inserted_primary_keys(options: &DataGridSaveStatementOptions) -> Option<String> {
    let primary_keys = &options.table_meta.primary_keys;
    if primary_keys.is_empty() || options.new_rows.is_empty() {
        return None;
    }

    let save_columns = effective_columns(options);
    let primary_key_indexes: Vec<Option<usize>> = primary_keys
        .iter()
        .map(|primary_key| find_column_index(options.database_type, &save_columns, primary_key))
        .collect();
    if primary_key_indexes.iter().any(Option::is_none) {
        return None;
    }
    let primary_key_indexes: Vec<usize> = primary_key_indexes.into_iter().flatten().collect();

    let mut existing_keys: Vec<String> = Vec::new();
    for row in &options.rows {
        if let Some(key) = primary_key_value_key(&primary_key_indexes, row) {
            existing_keys.push(key);
        }
    }

    let mut new_keys: Vec<String> = Vec::new();
    for row in &options.new_rows {
        let Some(key) = primary_key_value_key(&primary_key_indexes, row) else {
            continue;
        };
        if existing_keys.contains(&key) || new_keys.contains(&key) {
            return Some(duplicate_primary_key_error(
                primary_keys,
                &primary_key_indexes,
                row,
                existing_keys.contains(&key),
            ));
        }
        new_keys.push(key);
    }

    None
}

/// Builds the statements the save executes, and alongside them the server-side
/// guards for every keyless predicate those statements actually send. The guard
/// is derived from the same `where_clause` value the UPDATE/DELETE carries, so
/// the safety decision can never be made against a different set of columns or
/// values than the mutation itself uses.
fn build_data_grid_save_statements(
    options: &DataGridSaveStatementOptions,
    driver_profile: Option<&str>,
    keyless_guards: &mut Vec<DataGridSaveGuard>,
) -> Vec<String> {
    {}
    {}
    {}

    let save_columns = effective_columns(options);
    let column_info = options.table_meta.columns.as_deref().unwrap_or(&[]);
    let schema = crate::sql_dialect::table_data_schema(
        options.database_type,
        driver_profile,
        options.table_meta.schema.as_deref(),
    );
    let table = data_grid_save_table_name(options, schema);
    let mut statements = Vec::new();
    let primary_key_set: Vec<String> =
        options.table_meta.primary_keys.iter().map(|primary_key| normalize_column_name(primary_key)).collect();

    let guards_keyless_predicates = primary_key_set.is_empty() && uses_keyless_row_predicate(options.database_type);
    let mut guarded_predicates: Vec<String> = Vec::new();
    let guard_predicate = |predicate: &str, guarded: &mut Vec<String>| {
        if !guards_keyless_predicates || predicate.trim().is_empty() {
            return;
        }
        if !guarded.iter().any(|existing| existing == predicate) {
            guarded.push(predicate.to_string());
        }
    };

    let batch_mysql_writes = supports_mysql_data_grid_batch(options);
    let mut update_sets: Option<String> = None;
    let mut update_predicates = Vec::new();
    for (row_index, changes) in &options.dirty_rows {
        let Some(row) = options.rows.get(*row_index) else {
            continue;
        };
        let sets = changes
            .iter()
            .filter_map(|(column_index, value)| {
                let column = save_columns.get(*column_index)?.as_deref()?;
                if is_grid_update_omitted_column(
                    options.database_type,
                    column_info_for(column_info, column),
                    Some(column),
                    &primary_key_set,
                ) {
                    return None;
                }
                Some(format!(
                    "{} = {}",
                    data_grid_identifier(options.database_type, column, options.identifier_quote.as_deref()),
                    format_grid_save_sql_literal(
                        value,
                        options.database_type,
                        column_info_for(column_info, column),
                        options.identifier_quote.as_deref(),
                    )
                ))
            })
            .collect::<Vec<_>>()
            .join(", ");
        if sets.is_empty() {
            continue;
        }
        let where_clause = build_primary_key_where(
            options.database_type,
            &options.table_meta.primary_keys,
            &save_columns,
            row,
            column_info,
            options.identifier_quote.as_deref(),
        );
        guard_predicate(&where_clause, &mut guarded_predicates);
        if batch_mysql_writes {
            if update_sets.as_deref().is_some_and(|current| current != sets) {
                let current_sets = update_sets.take().unwrap_or_default();
                push_mysql_predicate_batches(
                    &mut statements,
                    &format!("UPDATE {table} SET {current_sets} WHERE "),
                    std::mem::take(&mut update_predicates),
                );
            }
            update_sets = Some(sets);
            update_predicates.push(where_clause);
        } else {
            statements.push(data_grid_statement(
                options.database_type,
                data_grid_update_sql(options.database_type, &table, &sets, &where_clause),
            ));
        }
    }
    if let Some(sets) = update_sets {
        push_mysql_predicate_batches(&mut statements, &format!("UPDATE {table} SET {sets} WHERE "), update_predicates);
    }

    let mut delete_predicates = Vec::new();
    for row_index in &options.deleted_rows {
        let Some(row) = options.rows.get(*row_index) else {
            continue;
        };
        let where_clause = build_primary_key_where(
            options.database_type,
            &options.table_meta.primary_keys,
            &save_columns,
            row,
            column_info,
            options.identifier_quote.as_deref(),
        );
        guard_predicate(&where_clause, &mut guarded_predicates);
        if batch_mysql_writes {
            delete_predicates.push(where_clause);
        } else {
            statements.push(data_grid_statement(
                options.database_type,
                data_grid_delete_sql(options.database_type, &table, &where_clause),
            ));
        }
    }
    if !delete_predicates.is_empty() {
        push_mysql_delete_batches(&mut statements, &format!("DELETE FROM {table} WHERE "), delete_predicates);
    }

    for row in &options.new_rows {
        {}
        let insert_pairs: Vec<(&str, &Value)> = save_columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| Some((column.as_deref()?, row.get(index).unwrap_or(&Value::Null))))
            .filter(|(column, value)| {
                let column_info = column_info_for(column_info, column);
                // Empty generated values must be omitted so the database can apply AUTO_INCREMENT/IDENTITY semantics.
                !column_info.is_some_and(is_auto_generated_column) || !grid_value_is_empty(value)
            })
            .filter(|(column, _)| {
                !is_grid_insert_omitted_column(
                    options.database_type,
                    column_info_for(column_info, column),
                    Some(column),
                    false,
                )
            })
            .filter(|(_, value)| !value.is_null())
            .collect();
        if insert_pairs.is_empty() {
            if options.database_type == Some(DatabaseType::Mysql) {
                statements
                    .push(data_grid_statement(options.database_type, format!("INSERT INTO {table} () VALUES ()")));
            }
            continue;
        }
        let columns = insert_pairs
            .iter()
            .map(|(column, _)| data_grid_identifier(options.database_type, column, options.identifier_quote.as_deref()))
            .collect::<Vec<_>>()
            .join(", ");
        let values = insert_pairs
            .iter()
            .map(|(column, value)| {
                format_grid_save_sql_literal(
                    value,
                    options.database_type,
                    column_info_for(column_info, column),
                    options.identifier_quote.as_deref(),
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        statements.push(data_grid_statement(options.database_type, {
            format!("INSERT INTO {table} ({columns}) VALUES ({values})")
        }));
    }

    keyless_guards.extend(guarded_predicates.into_iter().map(|predicate| DataGridSaveGuard {
        sql: format!("SELECT COUNT(*) AS {KEYLESS_GUARD_COUNT_ALIAS} FROM {table} WHERE ({predicate})"),
        max_matched_rows: 1,
        message: KEYLESS_AMBIGUOUS_ROW_ERROR.to_string(),
    }));

    statements
}

fn build_data_grid_rollback_statements(
    options: &DataGridSaveStatementOptions,
    driver_profile: Option<&str>,
) -> Vec<String> {
    {}
    {}
    {}
    {}

    let save_columns = effective_columns(options);
    let column_info = options.table_meta.columns.as_deref().unwrap_or(&[]);
    let schema = crate::sql_dialect::table_data_schema(
        options.database_type,
        driver_profile,
        options.table_meta.schema.as_deref(),
    );
    let table = data_grid_save_table_name(options, schema);
    let mut statements = Vec::new();

    for row in &options.new_rows {
        let where_clause = if options.database_type == Some(DatabaseType::Mysql) {
            build_mysql_insert_rollback_where(options, &save_columns, row, column_info)
        } else {
            let where_clause = build_save_row_where(
                options.database_type,
                &save_columns,
                row,
                column_info,
                options.identifier_quote.as_deref(),
            );
            (!where_clause.is_empty()).then_some(where_clause)
        };
        if let Some(where_clause) = where_clause {
            statements
                .push(data_grid_statement(options.database_type, format!("DELETE FROM {table} WHERE {where_clause}")));
        }
    }

    let batch_mysql_writes = supports_mysql_data_grid_batch(options);
    let mut deleted_insert_columns: Option<String> = None;
    let mut deleted_insert_values = Vec::new();
    for row_index in &options.deleted_rows {
        let Some(row) = options.rows.get(*row_index) else {
            continue;
        };
        {}
        let insert_pairs: Vec<(&str, &Value)> = save_columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| Some((column.as_deref()?, row.get(index).unwrap_or(&Value::Null))))
            .filter(|(column, _)| {
                !is_grid_insert_omitted_column(
                    options.database_type,
                    column_info_for(column_info, column),
                    Some(column),
                    false,
                )
            })
            .collect();
        let columns = insert_pairs
            .iter()
            .map(|(column, _)| data_grid_identifier(options.database_type, column, options.identifier_quote.as_deref()))
            .collect::<Vec<_>>()
            .join(", ");
        let values = insert_pairs
            .iter()
            .map(|(column, value)| {
                format_grid_assignment_sql_literal(
                    value,
                    options.database_type,
                    column_info_for(column_info, column),
                    options.identifier_quote.as_deref(),
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        if batch_mysql_writes {
            if deleted_insert_columns.as_deref().is_some_and(|current| current != columns) {
                let current_columns = deleted_insert_columns.take().unwrap_or_default();
                push_mysql_values_insert_batches(
                    &mut statements,
                    &table,
                    &current_columns,
                    std::mem::take(&mut deleted_insert_values),
                );
            }
            deleted_insert_columns = Some(columns);
            deleted_insert_values.push(format!("({values})"));
        } else {
            statements.push(data_grid_statement(options.database_type, {
                format!("INSERT INTO {table} ({columns}) VALUES ({values})")
            }));
        }
    }
    if let Some(columns) = deleted_insert_columns {
        push_mysql_values_insert_batches(&mut statements, &table, &columns, deleted_insert_values);
    }

    for (row_index, changes) in &options.dirty_rows {
        let Some(row) = options.rows.get(*row_index) else {
            continue;
        };
        let mut after_row = row.clone();
        for (column_index, value) in changes {
            if *column_index < after_row.len() {
                after_row[*column_index] = value.clone();
            }
        }
        let writable_changes: Vec<(&(usize, Value), &str)> = changes
            .iter()
            .filter_map(|change @ (column_index, _)| {
                let column = save_columns.get(*column_index)?.as_deref()?;
                if is_grid_update_omitted_column(
                    options.database_type,
                    column_info_for(column_info, column),
                    Some(column),
                    &[],
                ) {
                    return None;
                }
                Some((change, column))
            })
            .collect();
        let sets = writable_changes
            .iter()
            .map(|((column_index, _), column)| {
                format!(
                    "{} = {}",
                    data_grid_identifier(options.database_type, column, options.identifier_quote.as_deref()),
                    format_grid_assignment_sql_literal(
                        row.get(*column_index).unwrap_or(&Value::Null),
                        options.database_type,
                        column_info_for(column_info, column),
                        options.identifier_quote.as_deref(),
                    )
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        if sets.is_empty() {
            continue;
        }
        let mut predicates = vec![build_primary_key_where(
            options.database_type,
            &options.table_meta.primary_keys,
            &save_columns,
            &after_row,
            column_info,
            options.identifier_quote.as_deref(),
        )];
        predicates.extend(writable_changes.iter().map(|((_, value), column)| {
            build_save_column_predicate(
                options.database_type,
                column,
                value,
                column_info_for(column_info, column),
                true,
                options.identifier_quote.as_deref(),
            )
        }));
        statements.push(data_grid_statement(
            options.database_type,
            format!(
                "UPDATE {table} SET {sets} WHERE {}",
                predicates.into_iter().filter(|part| !part.is_empty()).collect::<Vec<_>>().join(" AND ")
            ),
        ));
    }

    statements
}

fn supports_mysql_data_grid_batch(options: &DataGridSaveStatementOptions) -> bool {
    // Keyless rows use full-row predicates, which are too wide and ambiguous to combine safely.
    options.database_type == Some(DatabaseType::Mysql) && !options.table_meta.primary_keys.is_empty()
}

fn push_mysql_predicate_batches(statements: &mut Vec<String>, prefix: &str, predicates: Vec<String>) {
    push_mysql_joined_batches(statements, prefix, predicates, " OR ", true);
}

/// Render a batch of delete predicates for MySQL. Rows deleted through the grid
/// usually differ only in a single primary-key equality predicate, so equal-key
/// runs collapse into `id IN (...)` instead of a chain of ORs (#8857). Anything
/// that is not a plain `column = literal` equality (compound keys, NULL checks,
/// BINARY comparisons, ...) falls back to the OR form, which stays correct for
/// every predicate shape.
fn push_mysql_delete_batches(statements: &mut Vec<String>, prefix: &str, predicates: Vec<String>) {
    const IN_JOINER: &str = ", ";

    struct InRun {
        column: Option<String>,
        values: Vec<String>,
        bytes: usize,
    }

    fn flush_in(run: &mut InRun, batches: &mut Vec<String>) {
        if let Some(column) = run.column.take() {
            if run.values.len() == 1 {
                batches.push(format!("{column} = {}", run.values[0]));
            } else {
                batches.push(format!("{column} IN ({})", run.values.join(IN_JOINER)));
            }
            run.values.clear();
            run.bytes = 0;
        }
    }

    let mut in_run = InRun { column: None, values: Vec::new(), bytes: 0 };
    let mut in_batches: Vec<String> = Vec::new();
    let mut pending_batches: Vec<String> = Vec::new();

    for predicate in &predicates {
        match parse_mysql_equality_predicate(predicate) {
            Some((column, literal)) => {
                let literal_bytes = literal.len() + IN_JOINER.len();
                let column_switch = in_run.column.as_deref().is_some_and(|existing| existing != column);
                let over_budget = in_run.bytes + literal_bytes > MYSQL_DATA_GRID_BATCH_TARGET_SQL_BYTES;
                let over_rows = in_run.values.len() >= MYSQL_DATA_GRID_BATCH_MAX_ROWS;
                if column_switch || over_budget || over_rows {
                    flush_in(&mut in_run, &mut in_batches);
                }
                if in_run.column.is_none() {
                    in_run.column = Some(column);
                }
                in_run.bytes += literal_bytes;
                in_run.values.push(literal);
            }
            None => {
                flush_in(&mut in_run, &mut in_batches);
                pending_batches.push(predicate.clone());
            }
        }
    }
    flush_in(&mut in_run, &mut in_batches);

    let mut batches = pending_batches;
    batches.extend(in_batches);
    if batches.len() == 1 {
        statements.push(format!("{prefix}{};", batches.remove(0)));
    } else if !batches.is_empty() {
        push_mysql_predicate_batches(statements, prefix, batches);
    }
}

/// Recognize predicates shaped exactly like `` `column` = literal `` (with or
/// without backticks) so they can merge into an IN list. Anything else —
/// compound-key AND chains, IS NULL, BINARY comparisons — returns None.
fn parse_mysql_equality_predicate(predicate: &str) -> Option<(String, String)> {
    let mut trimmed = predicate;
    while let Some(inner) = trimmed.strip_prefix('(').and_then(|inner| inner.strip_suffix(')')) {
        trimmed = inner;
    }
    let separator = trimmed.find(" = ")?;
    let column = trimmed.get(..separator)?.trim().to_string();
    if !column.starts_with('`') || !column.ends_with('`') || column.len() < 3 {
        return None;
    }
    let literal = trimmed.get(separator + 3..)?.trim().to_string();
    if literal.is_empty() || literal.eq_ignore_ascii_case("null") {
        return None;
    }
    // Compound-key predicates contain further AND/OR/IS clauses after the first
    // equality; merging those into an IN list would change the row targeting.
    let upper = literal.to_ascii_uppercase();
    if upper.contains(" AND ") || upper.contains(" OR ") || upper.starts_with("IS ") {
        return None;
    }
    Some((column, literal))
}

fn push_mysql_values_insert_batches(
    statements: &mut Vec<String>,
    table: &str,
    columns: &str,
    value_tuples: Vec<String>,
) {
    push_mysql_joined_batches(
        statements,
        &format!("INSERT INTO {table} ({columns}) VALUES "),
        value_tuples,
        ", ",
        false,
    );
}

fn push_mysql_joined_batches(
    statements: &mut Vec<String>,
    prefix: &str,
    parts: Vec<String>,
    separator: &str,
    wrap_multiple_parts: bool,
) {
    let mut batch = Vec::new();
    for part in parts {
        let next_len = joined_batch_sql_len(prefix, &batch, &part, separator, wrap_multiple_parts);
        if !batch.is_empty()
            && (batch.len() >= MYSQL_DATA_GRID_BATCH_MAX_ROWS || next_len > MYSQL_DATA_GRID_BATCH_TARGET_SQL_BYTES)
        {
            push_mysql_joined_batch_statement(
                statements,
                prefix,
                std::mem::take(&mut batch),
                separator,
                wrap_multiple_parts,
            );
        }
        batch.push(part);
    }
    if !batch.is_empty() {
        push_mysql_joined_batch_statement(statements, prefix, batch, separator, wrap_multiple_parts);
    }
}

fn joined_batch_sql_len(
    prefix: &str,
    current: &[String],
    next: &str,
    separator: &str,
    wrap_multiple_parts: bool,
) -> usize {
    let part_bytes = |part: &str| part.len() + usize::from(wrap_multiple_parts) * 2;
    if current.is_empty() {
        return prefix.len() + next.len() + 1;
    }
    let current_bytes = if current.len() == 1 && wrap_multiple_parts {
        current[0].len()
    } else {
        current.iter().map(|part| part_bytes(part)).sum::<usize>() + separator.len() * current.len().saturating_sub(1)
    };
    let existing_adjustment = if current.len() == 1 && wrap_multiple_parts { 2 } else { 0 };
    prefix.len() + current_bytes + existing_adjustment + separator.len() + part_bytes(next) + 1
}

fn push_mysql_joined_batch_statement(
    statements: &mut Vec<String>,
    prefix: &str,
    parts: Vec<String>,
    separator: &str,
    wrap_multiple_parts: bool,
) {
    let body = if parts.len() == 1 || !wrap_multiple_parts {
        parts.join(separator)
    } else {
        parts.into_iter().map(|part| format!("({part})")).collect::<Vec<_>>().join(separator)
    };
    statements.push(format!("{prefix}{body};"));
}

fn build_mysql_insert_rollback_where(
    options: &DataGridSaveStatementOptions,
    columns: &[Option<String>],
    row: &[Value],
    column_info: &[DataGridColumnInfo],
) -> Option<String> {
    if options.table_meta.primary_keys.is_empty() {
        return None;
    }

    for primary_key in &options.table_meta.primary_keys {
        let index = columns.iter().position(|column| column.as_deref() == Some(primary_key.as_str()))?;
        let value = row.get(index).unwrap_or(&Value::Null);
        let info = column_info_for(column_info, primary_key);
        if value.is_null()
            || empty_string_saves_as_null(value, info)
            || info.is_some_and(is_auto_generated_column)
            || info.is_some_and(|column| is_non_identity_generated_column(Some(column)))
        {
            // Generated or trigger-populated keys are unknown until after INSERT.
            // Do not emit a rollback predicate that cannot match the inserted row.
            return None;
        }
    }

    Some(build_primary_key_where(
        options.database_type,
        &options.table_meta.primary_keys,
        columns,
        row,
        column_info,
        options.identifier_quote.as_deref(),
    ))
}

pub fn effective_columns(options: &DataGridSaveStatementOptions) -> Vec<Option<String>> {
    let columns = match &options.source_columns {
        Some(source_columns) if source_columns.len() == options.columns.len() => source_columns.clone(),
        _ => options.columns.iter().map(|column| Some(column.clone())).collect(),
    };
    {
        return columns;
    }
}

fn effective_copy_columns(source_columns: Option<&[Option<String>]>, columns: &[String]) -> Vec<Option<String>> {
    match source_columns {
        Some(source_columns) if source_columns.len() == columns.len() => source_columns.to_vec(),
        _ => columns.iter().map(|column| Some(column.clone())).collect(),
    }
}

fn copy_column_info(
    column_info: &[DataGridColumnInfo],
    column: &str,
    fallback_type: Option<&str>,
) -> Option<DataGridColumnInfo> {
    if let Some(info) = column_info_for(column_info, column) {
        return Some(info.clone());
    }
    fallback_type.map(|data_type| DataGridColumnInfo {
        name: column.to_string(),
        data_type: data_type.to_string(),
        is_nullable: true,
        is_primary_key: false,
        column_default: None,
        extra: None,
    })
}

fn data_grid_save_execution_schema(
    database_type: Option<DatabaseType>,
    driver_profile: Option<&str>,
    table_meta: &DataGridTableMeta,
) -> Option<String> {
    {}
    crate::sql_dialect::table_data_schema(database_type, driver_profile, table_meta.schema.as_deref())
        .map(str::to_string)
}

pub fn normalize_data_grid_save_error(database_type: Option<DatabaseType>, error: &str) -> String {
    {}
    error.to_string()
}

pub(crate) fn format_grid_copy_insert_sql_literal(
    value: &Value,
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
    identifier_quote: Option<&str>,
) -> String {
    {}
    format_grid_assignment_sql_literal(value, database_type, column_info, identifier_quote)
}

fn format_grid_assignment_sql_literal(
    value: &Value,
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
    identifier_quote: Option<&str>,
) -> String {
    {}
    if (value.is_array() || value.is_object()) && is_json_document_column(column_info) {
        return format_grid_sql_literal_with_identifier_quote(
            &Value::String(value.to_string()),
            database_type,
            column_info,
            identifier_quote,
        );
    }
    format_grid_sql_literal_with_identifier_quote(value, database_type, column_info, identifier_quote)
}

fn is_json_document_column(column_info: Option<&DataGridColumnInfo>) -> bool {
    column_info.is_some_and(|column| {
        let data_type = column.data_type.trim();
        data_type.eq_ignore_ascii_case("json") || data_type.eq_ignore_ascii_case("jsonb")
    })
}

pub fn format_grid_sql_literal(
    value: &Value,
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
) -> String {
    format_grid_sql_literal_with_identifier_quote(value, database_type, column_info, None)
}

pub fn format_grid_sql_literal_with_identifier_quote(
    value: &Value,
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
    identifier_quote: Option<&str>,
) -> String {
    if value.is_null() {
        return "NULL".to_string();
    }
    // Boolean values on BIT columns use database-native boolean/bit literals.
    // This covers MySQL, SQL Server, and any other database where BIT
    // is a numeric/boolean type rather than a bit-string type like
    // PostgreSQL's bit(n).
    if let Some(value) = value.as_bool() {
        {}
        // SQL Server has no TRUE/FALSE literals (its boolean type is BIT, which
        // is_bit_literal_column already covers); any other column there still
        // needs numeric 1/0 instead of a literal.
        if is_bit_literal_column(database_type, column_info) || false {
            return if value { "1" } else { "0" }.to_string();
        }
        return if value { "TRUE" } else { "FALSE" }.to_string();
    }
    {}
    if is_mysql_bit_literal_column(database_type, column_info, identifier_quote) {
        if let Some(number) = value.as_number() {
            return number.to_string();
        }
        if let Some(text) = value.as_str().and_then(format_mysql_bit_literal_text) {
            return text;
        }
    }
    if let Some(number) = value.as_number() {
        return number.to_string();
    }
    if value.is_array() {
        return format_grid_sql_literal(&Value::String(value.to_string()), database_type, column_info);
    }
    let text = value.as_str().map_or_else(|| value.to_string(), ToString::to_string);
    {}
    if is_mysql_binary_literal_column(database_type, column_info) {
        if let Some(literal) = format_mysql_binary_literal_text(&text) {
            // DBX result values expose binary columns as prefixed hex; keep them
            // as MySQL hex literals so copied INSERT/UPDATE SQL round-trips bytes.
            return literal;
        }
    }
    {}
    {}
    if column_info.map(|column| is_numeric_type(&column.data_type)).unwrap_or(false) && is_numeric_literal(&text) {
        // BigDecimal/BigInteger cells cross JSON-RPC as strings so browsers cannot round them.
        return text;
    }
    {}
    if text.is_empty() {
        return { "''" }.to_string();
    }
    // MySQL geometry columns: wrap WKT text with ST_GeomFromText()
    if is_mysql_geometry_literal_database(database_type)
        && column_info.map(|column| is_geometry_column_type(&column.data_type)).unwrap_or(false)
    {
        let escaped = text.replace('\\', "\\\\").replace('\'', "''");
        return format!("ST_GeomFromText('{}')", escaped);
    }
    {}
    let literal_text = if is_mysql_datetime_literal_database(database_type)
        && column_info.map(|column| is_temporal_column_type(&column.data_type)).unwrap_or(true)
    {
        format_mysql_temporal_literal_text(&text, column_info.map(|column| column.data_type.as_str()))
    } else {
        text
    };
    {}
    {}
    let escaped_text = if false || keeps_literal_backslashes(database_type) {
        // These engines keep backslashes literal in ordinary string literals,
        // so only the quote delimiter needs escaping.
        literal_text.replace('\'', "''")
    } else {
        literal_text.replace('\\', "\\\\").replace('\'', "''")
    };
    let escaped = format!("'{escaped_text}'");
    escaped
}

/// Engines whose ordinary string literals keep a backslash literal: Oracle and
/// the engines that inherit its lexer for this purpose, plus the PostgreSQL
/// family, whose `standard_conforming_strings` default makes `'dir\'` a complete
/// string. Doubling the backslash there would copy `C:\\tmp` for a stored
/// `C:\tmp`. Mirrors the SQL export path, which only doubles backslashes for the
/// dialects whose escape table has one, and `keeps_backslash_literal` in
/// `dbx-core`'s transfer path.
fn keeps_literal_backslashes(database_type: Option<DatabaseType>) -> bool {
    false
}

fn format_grid_save_sql_literal(
    value: &Value,
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
    identifier_quote: Option<&str>,
) -> String {
    if empty_string_saves_as_null(value, column_info) {
        "NULL".to_string()
    } else {
        format_grid_assignment_sql_literal(value, database_type, column_info, identifier_quote)
    }
}

fn empty_string_saves_as_null(value: &Value, column_info: Option<&DataGridColumnInfo>) -> bool {
    value.as_str() == Some("")
        && column_info.is_some_and(|column| column.is_nullable && !is_textual_column_type(&column.data_type))
}

fn is_mysql_bit_literal_column(
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
    identifier_quote: Option<&str>,
) -> bool {
    (is_mysql_datetime_literal_database(database_type) || (false))
        && column_info.map(|column| is_bit_column_type(&column.data_type)).unwrap_or(false)
}

fn is_bit_literal_column(database_type: Option<DatabaseType>, column_info: Option<&DataGridColumnInfo>) -> bool {
    true && column_info.map(|column| is_bit_column_type(&column.data_type)).unwrap_or(false)
}

fn is_bit_column_type(data_type: &str) -> bool {
    let lower = data_type.to_ascii_lowercase();
    lower.split(|ch: char| !ch.is_ascii_alphanumeric()).any(|token| {
        // SQL Server/tiberius reports nullable BIT result columns as `bitn`.
        // They still need numeric 0/1 literals in generated UPDATE SQL.
        matches!(token, "bit" | "bitn")
    })
}

fn is_mysql_geometry_literal_database(database_type: Option<DatabaseType>) -> bool {
    matches!(database_type, Some(DatabaseType::Mysql))
}

fn is_mysql_binary_literal_column(
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
) -> bool {
    database_type == Some(DatabaseType::Mysql)
        && column_info.map(|column| is_mysql_binary_column_type(&column.data_type)).unwrap_or(false)
}

fn is_mysql_binary_column_type(data_type: &str) -> bool {
    let lower = data_type.trim().to_ascii_lowercase();
    let base = lower.split(['(', ':', ' ']).next().unwrap_or("").trim();
    matches!(base, "binary" | "varbinary" | "blob" | "tinyblob" | "mediumblob" | "longblob")
}

fn format_mysql_binary_literal_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let hex = trimmed.strip_prefix("0x")?;
    if hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Some(if hex.is_empty() { "X''".to_string() } else { trimmed.to_string() })
    } else {
        None
    }
}

fn is_geometry_column_type(data_type: &str) -> bool {
    let lower = data_type.to_ascii_lowercase();
    let base = lower.split('(').next().unwrap_or(&lower).trim();
    matches!(
        base,
        "geometry"
            | "point"
            | "linestring"
            | "polygon"
            | "multipoint"
            | "multilinestring"
            | "multipolygon"
            | "geometrycollection"
    )
}

fn format_mysql_bit_literal_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.eq_ignore_ascii_case("true") {
        return Some("1".to_string());
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return Some("0".to_string());
    }
    if trimmed.chars().all(|ch| ch.is_ascii_digit()) && !trimmed.is_empty() {
        return Some(if trimmed.len() == 1 {
            trimmed.to_string()
        } else if trimmed.chars().all(|ch| matches!(ch, '0' | '1')) {
            format!("b'{trimmed}'")
        } else {
            trimmed.to_string()
        });
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("b'") && trimmed.ends_with('\'') {
        let bits = &trimmed[2..trimmed.len() - 1];
        if !bits.is_empty() && bits.chars().all(|ch| matches!(ch, '0' | '1')) {
            return Some(format!("b'{bits}'"));
        }
    }
    None
}

fn is_mysql_datetime_literal_database(database_type: Option<DatabaseType>) -> bool {
    matches!(database_type, Some(DatabaseType::Mysql))
}

fn format_mysql_temporal_literal_text(text: &str, data_type: Option<&str>) -> String {
    let Some(captures) = regex_like_rfc3339(text) else {
        return text.to_string();
    };
    match temporal_column_kind(data_type) {
        Some("date") => captures.date,
        Some("time") => {
            format!("{}{}", captures.time, normalize_mysql_fractional_seconds(captures.fraction.as_deref()))
        }
        _ => format!(
            "{} {}{}",
            captures.date,
            captures.time,
            normalize_mysql_fractional_seconds(captures.fraction.as_deref())
        ),
    }
}

struct Rfc3339Parts {
    date: String,
    time: String,
    fraction: Option<String>,
    zone: String,
}

fn regex_like_rfc3339(text: &str) -> Option<Rfc3339Parts> {
    let bytes = text.as_bytes();
    if bytes.len() < 20 || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') {
        return None;
    }
    let separator = *bytes.get(10)?;
    if separator != b'T' && separator != b' ' {
        return None;
    }
    if bytes.get(13) != Some(&b':') || bytes.get(16) != Some(&b':') {
        return None;
    }
    let date = &text[0..10];
    let time = &text[11..19];
    let rest = &text[19..];
    let (fraction, zone) = if let Some(rest) = rest.strip_prefix('.') {
        let digit_count = rest.chars().take_while(|ch| ch.is_ascii_digit()).count();
        if digit_count == 0 || digit_count > 9 {
            return None;
        }
        (Some(format!(".{}", &rest[..digit_count])), &rest[digit_count..])
    } else {
        (None, rest)
    };
    if zone == "Z" || zone == "z" || is_timezone_offset(zone) {
        Some(Rfc3339Parts { date: date.to_string(), time: time.to_string(), fraction, zone: zone.to_string() })
    } else {
        None
    }
}

fn is_timezone_offset(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 6
        && matches!(bytes[0], b'+' | b'-')
        && bytes[3] == b':'
        && bytes[1].is_ascii_digit()
        && bytes[2].is_ascii_digit()
        && bytes[4].is_ascii_digit()
        && bytes[5].is_ascii_digit()
}

fn normalize_mysql_fractional_seconds(fraction: Option<&str>) -> String {
    match fraction {
        Some(fraction) if fraction.len() > 7 => fraction[..7].to_string(),
        Some(fraction) => fraction.to_string(),
        None => String::new(),
    }
}

fn is_temporal_column_type(data_type: &str) -> bool {
    temporal_column_kind(Some(data_type)).is_some()
}

fn temporal_column_kind(data_type: Option<&str>) -> Option<&'static str> {
    let base =
        data_type.unwrap_or("").trim().to_ascii_lowercase().split(['(', ':', ' ']).next().unwrap_or("").to_string();
    match base.as_str() {
        "date" => Some("date"),
        "time" => Some("time"),
        "datetime" | "timestamp" => Some("datetime"),
        _ => None,
    }
}

fn build_primary_key_where(
    database_type: Option<DatabaseType>,
    primary_keys: &[String],
    columns: &[Option<String>],
    row: &[Value],
    column_info: &[DataGridColumnInfo],
    identifier_quote: Option<&str>,
) -> String {
    if primary_keys.is_empty() && uses_keyless_row_predicate(database_type) {
        return build_row_where(database_type, columns, row, column_info, identifier_quote);
    }
    primary_keys
        .iter()
        .map(|primary_key| {
            let value = row
                .get(find_column_index(database_type, columns, primary_key).unwrap_or(usize::MAX))
                .unwrap_or(&Value::Null);
            build_column_predicate(
                database_type,
                primary_key,
                value,
                column_info_for(column_info, primary_key),
                false,
                identifier_quote,
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn build_row_where(
    database_type: Option<DatabaseType>,
    columns: &[Option<String>],
    row: &[Value],
    column_info: &[DataGridColumnInfo],
    identifier_quote: Option<&str>,
) -> String {
    columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| {
            let column = column.as_deref()?;
            if is_synthetic_row_id(database_type, Some(column)) {
                return None;
            }
            Some(build_column_predicate(
                database_type,
                column,
                row.get(index).unwrap_or(&Value::Null),
                column_info_for(column_info, column),
                true,
                identifier_quote,
            ))
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn build_save_row_where(
    database_type: Option<DatabaseType>,
    columns: &[Option<String>],
    row: &[Value],
    column_info: &[DataGridColumnInfo],
    identifier_quote: Option<&str>,
) -> String {
    columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| {
            let column = column.as_deref()?;
            if is_synthetic_row_id(database_type, Some(column)) {
                return None;
            }
            Some(build_save_column_predicate(
                database_type,
                column,
                row.get(index).unwrap_or(&Value::Null),
                column_info_for(column_info, column),
                true,
                identifier_quote,
            ))
        })
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub fn build_column_predicate(
    database_type: Option<DatabaseType>,
    column: &str,
    value: &Value,
    column_info: Option<&DataGridColumnInfo>,
    use_binary_text_comparison: bool,
    identifier_quote: Option<&str>,
) -> String {
    let ident = predicate_ident(database_type, column, identifier_quote);
    if value.is_null() {
        format!("{ident} IS NULL")
    } else if use_binary_text_comparison && uses_mysql_binary_text_predicate(database_type, value, column_info) {
        format!(
            "BINARY {ident} = {}",
            format_grid_assignment_sql_literal(value, database_type, column_info, identifier_quote)
        )
    } else {
        let literal = format_grid_assignment_sql_literal(value, database_type, column_info, identifier_quote);
        if use_binary_text_comparison {
            {}
        }
        format!("{ident} = {}", mysql_json_predicate_literal(literal, database_type, column_info))
    }
}

fn build_save_column_predicate(
    database_type: Option<DatabaseType>,
    column: &str,
    value: &Value,
    column_info: Option<&DataGridColumnInfo>,
    use_binary_text_comparison: bool,
    identifier_quote: Option<&str>,
) -> String {
    let ident = predicate_ident(database_type, column, identifier_quote);
    if value.is_null() || empty_string_saves_as_null(value, column_info) {
        format!("{ident} IS NULL")
    } else if use_binary_text_comparison && uses_mysql_binary_text_predicate(database_type, value, column_info) {
        format!(
            "BINARY {ident} = {}",
            format_grid_save_sql_literal(value, database_type, column_info, identifier_quote)
        )
    } else {
        let literal = format_grid_save_sql_literal(value, database_type, column_info, identifier_quote);
        if use_binary_text_comparison {
            {}
        }
        format!("{ident} = {}", mysql_json_predicate_literal(literal, database_type, column_info))
    }
}

fn mysql_json_predicate_literal(
    literal: String,
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
) -> String {
    if database_type == Some(DatabaseType::Mysql)
        && column_info.is_some_and(|column| column.data_type.trim().eq_ignore_ascii_case("json"))
    {
        format!("CAST({literal} AS JSON)")
    } else {
        literal
    }
}

fn data_grid_statement(database_type: Option<DatabaseType>, sql: String) -> String {
    {
        format!("{sql};")
    }
}

fn data_grid_update_sql(database_type: Option<DatabaseType>, table: &str, sets: &str, where_clause: &str) -> String {
    {
        format!("UPDATE {table} SET {sets} WHERE {where_clause}")
    }
}

fn data_grid_delete_sql(database_type: Option<DatabaseType>, table: &str, where_clause: &str) -> String {
    {
        format!("DELETE FROM {table} WHERE {where_clause}")
    }
}

fn uses_mysql_binary_text_predicate(
    database_type: Option<DatabaseType>,
    value: &Value,
    column_info: Option<&DataGridColumnInfo>,
) -> bool {
    database_type == Some(DatabaseType::Mysql)
        && value.is_string()
        && column_info.map(|column| is_textual_column_type(&column.data_type)).unwrap_or(false)
}

fn is_textual_column_type(data_type: &str) -> bool {
    let lower = data_type.trim().to_ascii_lowercase();
    let base = lower.split(['(', ':', ' ']).next().unwrap_or("").trim();
    matches!(
        base,
        "char"
            | "character"
            | "varchar"
            | "varchar2"
            | "nvarchar"
            | "nvarchar2"
            | "nchar"
            | "string"
            | "text"
            | "tinytext"
            | "mediumtext"
            | "longtext"
            | "ntext"
            | "clob"
            | "nclob"
            | "enum"
            | "set"
    ) || lower.starts_with("character varying")
        || lower.starts_with("national character varying")
}

fn is_synthetic_row_id(database_type: Option<DatabaseType>, name: Option<&str>) -> bool {
    uses_synthetic_row_id(database_type) && name.is_some_and(|name| name.eq_ignore_ascii_case(DBX_ROWID_COLUMN))
}

pub fn extra_is_auto_generated(extra: &str) -> bool {
    extra.to_ascii_lowercase().split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_').any(|part| {
        matches!(part, "auto_increment" | "autoincrement" | "identity" | "smallserial" | "serial" | "bigserial")
    })
}

pub fn is_auto_generated_column(column: &DataGridColumnInfo) -> bool {
    extra_is_auto_generated(column.extra.as_deref().unwrap_or(""))
        || column.column_default.as_deref().is_some_and(|default| default.to_ascii_lowercase().contains("nextval("))
}

fn grid_value_is_empty(value: &Value) -> bool {
    value.is_null() || value.as_str().is_some_and(str::is_empty)
}

pub fn is_grid_insert_omitted_column(
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
    name: Option<&str>,
    include_computed_columns: bool,
) -> bool {
    is_synthetic_row_id(database_type, name)
        || false
        || false
        || (!include_computed_columns && (is_non_identity_generated_column(column_info) || false))
}

fn is_grid_update_omitted_column(
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
    name: Option<&str>,
    primary_key_set: &[String],
) -> bool {
    is_synthetic_row_id(database_type, name) || false || is_non_identity_generated_column(column_info)
}

pub fn is_non_identity_generated_column(column_info: Option<&DataGridColumnInfo>) -> bool {
    let extra = column_info.and_then(|column| column.extra.as_deref()).unwrap_or("").to_ascii_lowercase();
    (extra.contains("generated always as") || extra.contains("virtual generated") || extra.contains("stored generated"))
        && !extra.contains("identity")
}

fn is_null_write_to_not_null_column(
    database_type: Option<DatabaseType>,
    not_null_columns: &[String],
    column: Option<&str>,
    value: &Value,
) -> bool {
    let Some(column) = column else {
        return false;
    };
    if is_synthetic_row_id(database_type, Some(column)) || false {
        return false;
    }
    value.is_null() && not_null_columns.iter().any(|not_null| not_null == &normalize_column_name(column))
}

fn find_column_index(database_type: Option<DatabaseType>, columns: &[Option<String>], target: &str) -> Option<usize> {
    if let Some(index) = columns.iter().position(|column| column.as_deref() == Some(target)) {
        return Some(index);
    }
    // PostgreSQL can have distinct `id` and quoted `"ID"` columns. Only
    // dialects whose result metadata is known to drift in case may fall back,
    // and even then a case-only match must be unique. Vastbase reports result
    // labels upper-cased (`ID`) while primary-key metadata keeps the stored
    // spelling (`id`), so the grid's primary-key badge and the save path have
    // to agree on the same column (#8797).
    {
        return None;
    }
}

fn primary_key_value_key(primary_key_indexes: &[usize], row: &[Value]) -> Option<String> {
    let values: Vec<Value> =
        primary_key_indexes.iter().map(|index| row.get(*index).cloned().unwrap_or(Value::Null)).collect();
    if values.iter().any(Value::is_null) {
        return None;
    }
    serde_json::to_string(&values).ok()
}

fn duplicate_primary_key_error(
    primary_keys: &[String],
    primary_key_indexes: &[usize],
    row: &[Value],
    matches_existing_row: bool,
) -> String {
    let key_summary = primary_keys
        .iter()
        .enumerate()
        .map(|(index, primary_key)| {
            format!(
                "{} = {}",
                primary_key,
                format_key_value_for_message(row.get(primary_key_indexes[index]).unwrap_or(&Value::Null))
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let source = if matches_existing_row { "the existing primary key" } else { "another new row's primary key" };
    format!("New row duplicates {source} ({key_summary}). Change the key before saving.")
}

fn format_key_value_for_message(value: &Value) -> String {
    if value.is_null() {
        return "NULL".to_string();
    }
    if let Some(value) = value.as_str() {
        return format!("\"{}\"", value.replace('"', "\\\""));
    }
    value.to_string()
}

fn normalize_column_name(name: &str) -> String {
    name.to_ascii_uppercase()
}

fn null_write_error(column: &str) -> String {
    format!("Column \"{column}\" does not allow NULL.")
}

fn predicate_ident(database_type: Option<DatabaseType>, name: &str, identifier_quote: Option<&str>) -> String {
    if is_synthetic_row_id(database_type, Some(name)) {
        {}
        "ROWIDTOCHAR(ROWID)".to_string()
    } else {
        data_grid_identifier(database_type, name, identifier_quote)
    }
}

pub fn quote_ident(database_type: Option<DatabaseType>, name: &str) -> String {
    quote_table_identifier(database_type, name)
}

pub fn qualified_table_name(database_type: Option<DatabaseType>, schema: Option<&str>, table_name: &str) -> String {
    crate::sql_dialect::qualified_table_name(database_type, schema, table_name)
}

fn data_grid_identifier(database_type: Option<DatabaseType>, name: &str, identifier_quote: Option<&str>) -> String {
    {}
    crate::sql_dialect::quote_table_data_identifier(database_type, name, identifier_quote)
}

/// Table reference for every generated data-grid SQL surface that honors the
/// `生成 SQL 时包含数据库名` setting: save / rollback / keyless-guard statements
/// and the copy-as-INSERT/UPDATE/SELECT statements.
///
/// With the setting on, engines whose active database is normally omitted get a
/// fully qualified name (`database.table`, or `database.schema.table` for SQL
/// Server, whose tables stay addressable across databases). Every other engine —
/// and the setting off — keeps the historical shape, so no existing SQL changes
/// shape unless the user opted in.
#[allow(clippy::too_many_arguments)]
pub fn data_grid_generated_table_name(
    database_type: Option<DatabaseType>,
    catalog: Option<&str>,
    schema: Option<&str>,
    database: Option<&str>,
    table_name: &str,
    identifier_quote: Option<&str>,
    include_database_name: bool,
) -> String {
    if include_database_name {
        if let Some(qualified) =
            crate::sql_dialect::database_qualified_table_name(database_type, catalog, schema, database, table_name)
        {
            return qualified;
        }
    }
    data_grid_qualified_table_name(database_type, catalog, schema, database, table_name, identifier_quote)
}

/// Table reference for the grid's save / rollback / keyless-guard statements.
///
/// Mirrors the data-table SELECT label and the copy-as-INSERT statements: when
/// `include_database_name` is on, engines addressed via `database.table`
/// (MySQL family, ClickHouse) get the database prefix — taken from
/// `table_meta.schema` when the table lives outside the connection's default
/// database, otherwise from `table_meta.database` — and SQL Server gets the
/// three-part `database.schema.table` form. Every other engine keeps the
/// historical shape.
fn data_grid_save_table_name(options: &DataGridSaveStatementOptions, schema: Option<&str>) -> String {
    data_grid_generated_table_name(
        options.database_type,
        options.table_meta.catalog.as_deref(),
        schema,
        options.table_meta.database.as_deref(),
        &options.table_meta.table_name,
        options.identifier_quote.as_deref(),
        options.include_database_name,
    )
}

pub fn data_grid_qualified_table_name(
    database_type: Option<DatabaseType>,
    catalog: Option<&str>,
    schema: Option<&str>,
    database: Option<&str>,
    table_name: &str,
    identifier_quote: Option<&str>,
) -> String {
    {}
    if crate::sql_dialect::uses_connection_identifier_quote(database_type, identifier_quote) {
        crate::sql_dialect::table_data_qualified_table_name(database_type, schema, table_name, identifier_quote)
    } else {
        crate::sql_dialect::qualified_table_name_with_catalog(database_type, catalog, schema, database, table_name)
    }
}

fn column_filter_ref(database_type: Option<DatabaseType>, column_name: &str, identifier_quote: Option<&str>) -> String {
    let quoted = predicate_ident(database_type, column_name, identifier_quote);
    {
        quoted
    }
}

fn column_like_filter_ref(
    database_type: Option<DatabaseType>,
    column_name: &str,
    column_info: Option<&DataGridColumnInfo>,
    identifier_quote: Option<&str>,
) -> String {
    let column = column_filter_ref(database_type, column_name, identifier_quote);
    {
        column
    }
}

fn value_to_filter_text(value: &Value) -> String {
    if let Some(value) = value.as_str() {
        value.to_string()
    } else if value.is_null() {
        String::new()
    } else {
        value.to_string()
    }
}

fn parse_typed_filter_value(
    text: &str,
    database_type: Option<DatabaseType>,
    column_info: Option<&DataGridColumnInfo>,
) -> Value {
    let unquoted = unwrap_matching_quotes(text);
    let data_type = column_info.map(|column| column.data_type.to_ascii_lowercase()).unwrap_or_default();
    if is_boolean_type(&data_type, database_type) && unquoted.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if is_boolean_type(&data_type, database_type) && unquoted.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if (is_numeric_type(&data_type) || data_type.is_empty()) && is_numeric_literal(&unquoted) {
        if let Ok(number) = unquoted.parse::<serde_json::Number>() {
            return Value::Number(number);
        }
    }
    Value::String(unquoted)
}

fn unwrap_matching_quotes(text: &str) -> String {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let Some(last) = text.chars().last() else {
        return String::new();
    };
    if text.len() >= 2 && ((first == '\'' && last == '\'') || (first == '"' && last == '"')) {
        text[1..text.len() - 1].to_string()
    } else {
        text.to_string()
    }
}

fn is_numeric_type(data_type: &str) -> bool {
    let lower = data_type.to_ascii_lowercase();
    const NUMERIC_TOKENS: &[&str] = &[
        "int",
        "integer",
        "bigint",
        "smallint",
        "tinyint",
        "mediumint",
        "serial",
        "number",
        "numeric",
        "decimal",
        "float",
        "double",
        "real",
        "money",
    ];
    lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| NUMERIC_TOKENS.contains(&token) || is_sized_numeric_token(token))
}

// Databases like Cloud Spanner (GoogleSQL) and ClickHouse spell their numeric
// types with an explicit bit width, e.g. INT64/FLOAT64/FLOAT32 or
// Int8/UInt64/Float64. Recognizing `int`/`uint`/`float` followed by digits keeps
// these as numeric SQL literals so filter values are not quoted as strings,
// which strongly typed engines reject (INT64 = STRING has no matching operator).
// `interval` and similar names stay excluded because they are not `int`+digits.
fn is_sized_numeric_token(token: &str) -> bool {
    for prefix in ["int", "uint", "float"] {
        if let Some(width) = token.strip_prefix(prefix) {
            if !width.is_empty() && width.bytes().all(|b| b.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

fn is_boolean_type(data_type: &str, database_type: Option<DatabaseType>) -> bool {
    let lower = data_type.to_ascii_lowercase();
    lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| matches!(token, "bool" | "boolean") || (matches!(token, "bit" | "bitn") && true))
}

fn is_numeric_literal(text: &str) -> bool {
    if text.trim() != text || text.is_empty() {
        return false;
    }
    text.parse::<f64>().is_ok_and(f64::is_finite)
        && text.chars().all(|ch| ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.' | 'e' | 'E'))
        && text.chars().any(|ch| ch.is_ascii_digit())
}

fn uses_keyless_row_predicate(database_type: Option<DatabaseType>) -> bool {
    matches!(database_type, Some(DatabaseType::Mysql))
}

pub fn column_info_for<'a>(columns: &'a [DataGridColumnInfo], name: &str) -> Option<&'a DataGridColumnInfo> {
    let normalized = normalize_column_name(name);
    columns.iter().find(|column| normalize_column_name(&column.name) == normalized)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn column(name: &str, data_type: &str, nullable: bool, extra: Option<&str>) -> DataGridColumnInfo {
        DataGridColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            is_nullable: nullable,
            is_primary_key: false,
            column_default: None,
            extra: extra.map(ToString::to_string),
        }
    }

    fn mysql_people_save_options(row_count: usize) -> DataGridSaveStatementOptions {
        DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "people".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "bigint", false, None), column("status", "varchar(32)", true, None)]),
            },
            columns: vec!["id".to_string(), "status".to_string()],
            source_columns: None,
            rows: (1..=row_count).map(|id| vec![json!(id), json!("active")]).collect(),
            dirty_rows: vec![],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        }
    }

    fn daily_stats_keyless_options() -> DataGridSaveStatementOptions {
        DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "daily_stats".to_string(),
                primary_keys: vec![],
                columns: Some(vec![column("stat_date", "TEXT", false, None), column("period", "TEXT", true, None)]),
            },
            columns: vec!["stat_date".to_string(), "period".to_string()],
            source_columns: None,
            rows: vec![vec![json!("2026-09-07"), Value::Null], vec![json!("2026-09-07"), Value::Null]],
            dirty_rows: vec![(0, vec![(1, json!("早上"))])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        }
    }
    #[test]
    fn mysql_data_grid_save_honors_include_database_name() {
        let mut options = mysql_people_save_options(1);
        options.table_meta.database = Some("appdb".to_string());
        options.table_meta.schema = None;
        options.dirty_rows = vec![(0, vec![(1, json!("blocked"))])];

        options.include_database_name = true;
        let qualified = prepare_data_grid_save(options.clone());
        assert_eq!(qualified.validation_error, None);
        assert_eq!(qualified.statements, vec!["UPDATE `appdb`.`people` SET `status` = 'blocked' WHERE `id` = 1;"]);
        assert_eq!(
            qualified.rollback_statements,
            vec!["UPDATE `appdb`.`people` SET `status` = 'active' WHERE `id` = 1 AND BINARY `status` = 'blocked';"]
        );

        options.include_database_name = false;
        let bare = prepare_data_grid_save(options);
        assert_eq!(bare.statements, vec!["UPDATE `people` SET `status` = 'blocked' WHERE `id` = 1;"]);
        assert_eq!(
            bare.rollback_statements,
            vec!["UPDATE `people` SET `status` = 'active' WHERE `id` = 1 AND BINARY `status` = 'blocked';"]
        );
    }

    #[test]
    fn mysql_data_grid_save_qualifies_insert_delete_and_keyless_guard() {
        let mut options = mysql_people_save_options(0);
        options.table_meta.database = Some("appdb".to_string());
        options.table_meta.schema = None;
        options.include_database_name = true;
        options.new_rows = vec![vec![json!(7), json!("created")]];
        options.deleted_rows = vec![];

        let inserted = prepare_data_grid_save(options.clone());
        assert_eq!(inserted.statements, vec!["INSERT INTO `appdb`.`people` (`id`, `status`) VALUES (7, 'created');"]);

        let mut keyless = options;
        keyless.new_rows = vec![];
        keyless.table_meta.primary_keys = vec![];
        keyless.deleted_rows = vec![0];
        let deleted = prepare_data_grid_save(keyless);
        assert!(
            deleted.statements.iter().all(|statement| statement.contains("`appdb`.`people`")),
            "every statement must carry the database qualifier: {:?}",
            deleted.statements
        );
        assert!(
            deleted.keyless_guards.iter().all(|guard| guard.sql.contains("`appdb`.`people`")),
            "the keyless guard counts rows in the same table reference: {:?}",
            deleted.keyless_guards
        );
    }

    /// A cross-database editable result (`SELECT * FROM db_9.users`) keeps its own
    /// namespace in `schema` while `database` still holds the connection's default
    /// database — the qualifier must follow the table, not the connection.
    #[test]
    fn mysql_data_grid_save_prefers_cross_database_schema() {
        let mut options = mysql_people_save_options(1);
        options.table_meta.schema = Some("db_9".to_string());
        options.table_meta.database = Some("appdb".to_string());
        options.dirty_rows = vec![(0, vec![(1, json!("blocked"))])];
        options.include_database_name = true;

        let result = prepare_data_grid_save(options);
        assert_eq!(result.statements, vec!["UPDATE `db_9`.`people` SET `status` = 'blocked' WHERE `id` = 1;"]);
        assert_eq!(result.execution_schema.as_deref(), Some("db_9"));
    }

    #[test]
    fn mysql_conditional_update_uses_typed_value_and_requires_a_writable_column_and_where() {
        let options = DataGridConditionalUpdateSqlOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "people".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "bigint", false, None), column("status", "varchar(32)", true, None)]),
            },
            column_name: "status".to_string(),
            value: json!("O'Reilly"),
            where_input: "WHERE tenant_id = 7;".to_string(),
        };

        assert_eq!(
            build_data_grid_conditional_update_sql(options.clone()),
            Some("UPDATE `app`.`people` SET `status` = 'O''Reilly' WHERE (tenant_id = 7);".to_string())
        );

        let mut condition_only = options.clone();
        condition_only.where_input = "tenant_id = 7".to_string();
        assert_eq!(
            build_data_grid_conditional_update_sql(condition_only),
            Some("UPDATE `app`.`people` SET `status` = 'O''Reilly' WHERE (tenant_id = 7);".to_string())
        );

        let mut empty_where = options.clone();
        empty_where.where_input = " ".to_string();
        assert_eq!(build_data_grid_conditional_update_sql(empty_where), None);

        let mut primary_key = options;
        primary_key.column_name = "id".to_string();
        assert_eq!(build_data_grid_conditional_update_sql(primary_key), None);
    }

    /// The right-click `复制 → SQL UPDATE 语句` path must honor
    /// `生成 SQL 时包含数据库名` exactly like the copy-as-INSERT statements do
    /// (issue #9262).
    #[test]
    fn copy_update_honors_include_database_name() {
        let options = DataGridCopyUpdateStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: Some("appdb".to_string()),
                schema: None,
                table_name: "people".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: None,
            },
            columns: vec!["id".to_string(), "status".to_string()],
            source_columns: None,
            rows: vec![vec![json!(1), json!("blocked")]],
            include_database_name: true,
        };
        assert_eq!(
            build_data_grid_copy_update_statements(options.clone()),
            vec!["UPDATE `appdb`.`people` SET `status` = 'blocked' WHERE `id` = 1;"]
        );
        // The frontend strips the metadata when the setting is off, but the
        // option itself must also fall back to the historical bare shape.
        assert_eq!(
            build_data_grid_copy_update_statements(DataGridCopyUpdateStatementOptions {
                include_database_name: false,
                ..options
            }),
            vec!["UPDATE `people` SET `status` = 'blocked' WHERE `id` = 1;"]
        );
    }

    #[test]
    fn builds_copy_insert_statement_without_primary_keys() {
        let statement = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: Some(DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "users".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![
                    DataGridColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        column_default: None,
                        extra: Some("auto_increment".to_string()),
                    },
                    DataGridColumnInfo {
                        name: "login_name".to_string(),
                        data_type: "varchar(64)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        column_default: None,
                        extra: None,
                    },
                    DataGridColumnInfo {
                        name: "display_name".to_string(),
                        data_type: "varchar(64)".to_string(),
                        is_nullable: true,
                        is_primary_key: false,
                        column_default: None,
                        extra: None,
                    },
                ]),
            }),
            columns: vec!["id".to_string(), "login_name".to_string(), "display_name".to_string()],
            column_types: None,
            source_columns: None,
            rows: vec![vec![json!(1), json!("ada"), json!("Ada")], vec![json!(2), json!("linus"), json!("Linus")]],
            exclude_primary_keys: true,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::Merged,
        });
        assert_eq!(
            statement.as_deref(),
            Some("INSERT INTO `users` (`login_name`, `display_name`) VALUES\n('ada', 'Ada'),\n('linus', 'Linus');")
        );
    }

    #[test]
    fn copy_insert_uses_display_columns_but_source_columns_for_metadata() {
        let statement = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: Some(DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "psn_basic_info".to_string(),
                primary_keys: vec![],
                columns: Some(vec![
                    DataGridColumnInfo {
                        name: "PSN_NO".to_string(),
                        data_type: "varchar(32)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        column_default: None,
                        extra: None,
                    },
                    DataGridColumnInfo {
                        name: "NAME".to_string(),
                        data_type: "varchar(32)".to_string(),
                        is_nullable: true,
                        is_primary_key: false,
                        column_default: None,
                        extra: None,
                    },
                ]),
            }),
            columns: vec!["psn".to_string(), "psn_no".to_string(), "name".to_string()],
            column_types: None,
            source_columns: Some(vec![
                Some("PSN_NO".to_string()),
                Some("PSN_NO".to_string()),
                Some("NAME".to_string()),
            ]),
            rows: vec![vec![json!("A-1"), json!("A-1"), json!("Ada")]],
            exclude_primary_keys: false,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::Merged,
        });
        assert_eq!(
            statement.as_deref(),
            Some("INSERT INTO `psn_basic_info` (`psn`, `psn_no`, `name`) VALUES ('A-1', 'A-1', 'Ada');")
        );
    }

    #[test]
    fn copy_insert_uses_source_columns_for_generated_column_exclusions() {
        let statement = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: Some(DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "users".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![
                    DataGridColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        column_default: None,
                        extra: Some("auto_increment".to_string()),
                    },
                    DataGridColumnInfo {
                        name: "display_name".to_string(),
                        data_type: "varchar(64)".to_string(),
                        is_nullable: true,
                        is_primary_key: false,
                        column_default: None,
                        extra: None,
                    },
                    DataGridColumnInfo {
                        name: "display_name_upper".to_string(),
                        data_type: "varchar(64)".to_string(),
                        is_nullable: true,
                        is_primary_key: false,
                        column_default: None,
                        extra: Some("virtual generated".to_string()),
                    },
                ]),
            }),
            columns: vec!["identifier".to_string(), "label".to_string(), "label_upper".to_string()],
            column_types: None,
            source_columns: Some(vec![
                Some("id".to_string()),
                Some("display_name".to_string()),
                Some("display_name_upper".to_string()),
            ]),
            rows: vec![vec![json!(1), json!("Ada"), json!("ADA")]],
            exclude_primary_keys: true,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::Merged,
        });
        assert_eq!(statement.as_deref(), Some("INSERT INTO `users` (`label`) VALUES ('Ada');"));
    }

    #[test]
    fn copy_update_keeps_source_columns_for_writeback() {
        let statements = build_data_grid_copy_update_statements(DataGridCopyUpdateStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "users".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: None,
            },
            columns: vec!["identifier".to_string(), "label".to_string()],
            source_columns: Some(vec![Some("id".to_string()), Some("display_name".to_string())]),
            rows: vec![vec![json!(1), json!("Ada")]],
            include_database_name: false,
        });
        assert_eq!(statements, vec!["UPDATE `users` SET `display_name` = 'Ada' WHERE `id` = 1;"]);
    }

    #[test]
    fn copy_insert_primary_key_exclusion_keeps_manual_composite_key_members() {
        let statement = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: Some(DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "move_statistic_product_daily".to_string(),
                primary_keys: vec!["id".to_string(), "stat_date".to_string()],
                columns: Some(vec![
                    DataGridColumnInfo {
                        name: "id".to_string(),
                        data_type: "bigint unsigned".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        column_default: None,
                        extra: Some("auto_increment".to_string()),
                    },
                    DataGridColumnInfo {
                        name: "stat_date".to_string(),
                        data_type: "date".to_string(),
                        is_nullable: false,
                        is_primary_key: true,
                        column_default: None,
                        extra: None,
                    },
                    DataGridColumnInfo {
                        name: "product_name".to_string(),
                        data_type: "varchar(255)".to_string(),
                        is_nullable: false,
                        is_primary_key: false,
                        column_default: Some("".to_string()),
                        extra: None,
                    },
                ]),
            }),
            columns: vec!["id".to_string(), "stat_date".to_string(), "product_name".to_string()],
            column_types: None,
            source_columns: None,
            rows: vec![vec![json!(1), json!("2026-08-18"), json!("sweater")]],
            exclude_primary_keys: true,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::Merged,
        });
        assert_eq!(
            statement.as_deref(),
            Some("INSERT INTO `move_statistic_product_daily` (`stat_date`, `product_name`) VALUES ('2026-08-18', 'sweater');")
        );
    }

    #[test]
    fn copy_insert_primary_key_exclusion_keeps_unknown_metadata_primary_keys() {
        // Without column metadata we cannot prove the key is auto-generated;
        // keep it rather than silently dropping NOT NULL data.
        let statement = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: Some(DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "users".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: None,
            }),
            columns: vec!["id".to_string(), "login_name".to_string()],
            column_types: None,
            source_columns: None,
            rows: vec![vec![json!(1), json!("ada")]],
            exclude_primary_keys: true,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::Merged,
        });
        assert_eq!(statement.as_deref(), Some("INSERT INTO `users` (`id`, `login_name`) VALUES (1, 'ada');"));
    }

    #[test]
    fn copy_insert_keeps_mysql_json_array_as_single_json_literal() {
        let statement = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: None,
            columns: vec!["data".to_string()],
            column_types: Some(vec![Some("json".to_string())]),
            source_columns: None,
            rows: vec![vec![json!([1, 2, 3])]],
            exclude_primary_keys: false,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::Merged,
        });

        assert_eq!(statement.as_deref(), Some("INSERT INTO table_name (`data`) VALUES ('[1,2,3]');"));
    }

    #[test]
    fn copy_insert_keeps_mysql_json_empty_array_as_single_json_literal() {
        let statement = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: None,
            columns: vec!["data".to_string()],
            column_types: Some(vec![Some("json".to_string())]),
            source_columns: None,
            rows: vec![vec![json!([])]],
            exclude_primary_keys: false,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::Merged,
        });

        assert_eq!(statement.as_deref(), Some("INSERT INTO table_name (`data`) VALUES ('[]');"));
    }

    #[test]
    fn copy_insert_keeps_mysql_json_object_as_single_json_literal() {
        let statement = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: None,
            columns: vec!["data".to_string()],
            column_types: Some(vec![Some("json".to_string())]),
            source_columns: None,
            rows: vec![vec![json!({"a": 1})]],
            exclude_primary_keys: false,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::Merged,
        });

        assert_eq!(statement.as_deref(), Some("INSERT INTO table_name (`data`) VALUES ('{\"a\":1}');"));
    }

    #[test]
    fn copy_update_formats_mysql_json_documents_as_compact_strings() {
        let statements = build_data_grid_copy_update_statements(DataGridCopyUpdateStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "documents".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "int", false, None), column("payload", "JSON", true, None)]),
            },
            columns: vec!["id".to_string(), "payload".to_string()],
            source_columns: None,
            rows: vec![
                vec![json!(1), json!([111, 222, 333])],
                vec![json!(2), json!({"nested": [1, 2]})],
                vec![json!(3), json!([])],
            ],
            include_database_name: false,
        });

        assert_eq!(
            statements,
            vec![
                "UPDATE `documents` SET `payload` = '[111,222,333]' WHERE `id` = 1;",
                "UPDATE `documents` SET `payload` = '{\"nested\":[1,2]}' WHERE `id` = 2;",
                "UPDATE `documents` SET `payload` = '[]' WHERE `id` = 3;",
            ]
        );
    }

    #[test]
    fn builds_copy_insert_without_primary_keys_when_primary_keys_are_hidden() {
        let statement = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: Some(DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "users".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: None,
            }),
            columns: vec!["login_name".to_string(), "display_name".to_string()],
            column_types: None,
            source_columns: None,
            rows: vec![vec![json!("ada"), json!("Ada")]],
            exclude_primary_keys: true,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::Merged,
        });

        assert_eq!(
            statement.as_deref(),
            Some("INSERT INTO `users` (`login_name`, `display_name`) VALUES ('ada', 'Ada');")
        );
    }

    #[test]
    fn builds_copy_insert_statement_row_by_row() {
        let statement = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: Some(DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "users".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: None,
            }),
            columns: vec!["id".to_string(), "login_name".to_string(), "display_name".to_string()],
            column_types: None,
            source_columns: None,
            rows: vec![vec![json!(1), json!("ada"), json!("Ada")], vec![json!(2), json!("linus"), json!("Linus")]],
            exclude_primary_keys: false,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::RowByRow,
        });
        assert_eq!(
            statement.as_deref(),
            Some(
                "INSERT INTO `users` (`id`, `login_name`, `display_name`) VALUES (1, 'ada', 'Ada');\nINSERT INTO `users` (`id`, `login_name`, `display_name`) VALUES (2, 'linus', 'Linus');"
            )
        );
    }

    #[test]
    fn mysql_copy_statements_preserve_blob_hex_literals() {
        let table_meta = DataGridTableMeta {
            catalog: None,
            database: None,
            schema: None,
            table_name: "reports".to_string(),
            primary_keys: vec!["id".to_string()],
            columns: Some(vec![column("id", "int", false, None), column("payload", "MEDIUMBLOB", true, None)]),
        };
        let columns = vec!["id".to_string(), "payload".to_string()];
        let rows = vec![vec![json!(1), json!("0x0001abff")], vec![json!(2), json!("0x")]];

        let insert = build_data_grid_copy_insert_statement(DataGridCopyInsertStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: Some(table_meta.clone()),
            columns: columns.clone(),
            column_types: None,
            source_columns: None,
            rows: rows.clone(),
            exclude_primary_keys: false,
            include_computed_columns: false,
            include_database_name: true,
            insert_mode: DataGridCopyInsertMode::Merged,
        });
        assert_eq!(
            insert.as_deref(),
            Some("INSERT INTO `reports` (`id`, `payload`) VALUES\n(1, 0x0001abff),\n(2, X'');")
        );

        let updates = build_data_grid_copy_update_statements(DataGridCopyUpdateStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta,
            columns,
            source_columns: None,
            rows,
            include_database_name: false,
        });
        assert_eq!(
            updates,
            vec![
                "UPDATE `reports` SET `payload` = 0x0001abff WHERE `id` = 1;",
                "UPDATE `reports` SET `payload` = X'' WHERE `id` = 2;"
            ]
        );
    }

    #[test]
    fn mysql_text_columns_keep_prefixed_hex_strings_quoted() {
        assert_eq!(
            format_grid_sql_literal(
                &json!("0x0001abff"),
                Some(DatabaseType::Mysql),
                Some(&column("note", "varchar(64)", true, None)),
            ),
            "'0x0001abff'"
        );
        assert_eq!(
            format_grid_sql_literal(
                &json!("0xnothex"),
                Some(DatabaseType::Mysql),
                Some(&column("payload", "blob", true, None)),
            ),
            "'0xnothex'"
        );
    }

    #[test]
    fn keeps_context_filter_mode_serialization_stable() {
        assert_eq!(serde_json::to_string(&DataGridContextFilterMode::IsNull).unwrap(), "\"is-null\"");
        assert_eq!(serde_json::to_string(&DataGridContextFilterMode::IsNotNull).unwrap(), "\"is-not-null\"");
        assert!(matches!(
            serde_json::from_str::<DataGridContextFilterMode>("\"is-null\"").unwrap(),
            DataGridContextFilterMode::IsNull
        ));
        assert!(matches!(
            serde_json::from_str::<DataGridContextFilterMode>("\"is-not-null\"").unwrap(),
            DataGridContextFilterMode::IsNotNull
        ));
        assert_eq!(serde_json::to_string(&DataGridContextFilterMode::IsBlank).unwrap(), "\"is-blank\"");
        assert_eq!(serde_json::to_string(&DataGridContextFilterMode::IsNotBlank).unwrap(), "\"is-not-blank\"");
    }

    #[test]
    fn context_membership_and_range_filters_require_complete_values() {
        let column_info = Some(column("score", "int", false, None));
        assert_eq!(
            build_data_grid_context_filter_condition(DataGridContextFilterConditionOptions {
                database_type: Some(DatabaseType::Mysql),
                identifier_quote: None,
                column_name: "score".to_string(),
                mode: DataGridContextFilterMode::In,
                value: Value::Null,
                values: Vec::new(),
                end_value: None,
                column_info: column_info.clone(),
            }),
            None
        );
        assert_eq!(
            build_data_grid_context_filter_condition(DataGridContextFilterConditionOptions {
                database_type: Some(DatabaseType::Mysql),
                identifier_quote: None,
                column_name: "score".to_string(),
                mode: DataGridContextFilterMode::Between,
                value: json!(10),
                values: Vec::new(),
                end_value: None,
                column_info: column_info.clone(),
            }),
            None
        );
        assert_eq!(
            build_data_grid_context_filter_condition(DataGridContextFilterConditionOptions {
                database_type: Some(DatabaseType::Mysql),
                identifier_quote: None,
                column_name: "score".to_string(),
                mode: DataGridContextFilterMode::Between,
                value: Value::Null,
                values: Vec::new(),
                end_value: Some(json!(20)),
                column_info: column_info.clone(),
            }),
            None
        );
        assert_eq!(
            build_data_grid_context_filter_condition(DataGridContextFilterConditionOptions {
                database_type: Some(DatabaseType::Mysql),
                identifier_quote: None,
                column_name: "score".to_string(),
                mode: DataGridContextFilterMode::NotBetween,
                value: json!(10),
                values: Vec::new(),
                end_value: Some(Value::Null),
                column_info,
            }),
            None
        );
    }

    #[test]
    fn context_filter_options_default_new_fields_when_deserializing_old_requests() {
        let options: DataGridContextFilterConditionOptions = serde_json::from_value(json!({
            "databaseType": "mysql",
            "columnName": "id",
            "mode": "equals",
            "value": 42,
        }))
        .unwrap();

        assert!(options.values.is_empty());
        assert_eq!(options.end_value, None);
    }

    #[test]
    fn formats_temporal_copy_literals() {
        assert_eq!(
            format_grid_sql_literal(&json!("2026-05-12T00:00:00+00:00"), Some(DatabaseType::Mysql), None),
            "'2026-05-12 00:00:00'"
        );
        assert_eq!(
            format_grid_sql_literal(&json!("2026-05-12T00:00:00.123456Z"), Some(DatabaseType::Mysql), None),
            "'2026-05-12 00:00:00.123456'"
        );
        assert_eq!(
            format_grid_sql_literal(&json!("2026-05-12 00:00:00.123456"), Some(DatabaseType::Mysql), None),
            "'2026-05-12 00:00:00.123456'"
        );
    }

    #[test]
    fn mysql_grouped_result_update_uses_physical_columns_and_primary_key() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "users".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "bigint", false, None), column("department", "varchar", false, None)]),
            },
            columns: vec!["用户ID".to_string(), "部门".to_string(), "订单数".to_string()],
            source_columns: Some(vec![Some("id".to_string()), Some("department".to_string()), None]),
            rows: vec![vec![json!(7), json!("sales"), json!(3)]],
            dirty_rows: vec![(0, vec![(1, json!("support"))])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["UPDATE `app`.`users` SET `department` = 'support' WHERE `id` = 7;"]);
        assert_eq!(
            result.rollback_statements,
            vec!["UPDATE `app`.`users` SET `department` = 'sales' WHERE `id` = 7 AND BINARY `department` = 'support';"]
        );
    }

    #[test]
    fn mysql_update_omits_virtual_and_stored_generated_columns() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "users".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![
                    column("id", "bigint", false, None),
                    column("virtual_label", "varchar", true, Some("VIRTUAL GENERATED")),
                    column("stored_label", "varchar", true, Some("STORED GENERATED")),
                    column("department", "varchar", false, None),
                ]),
            },
            columns: vec![
                "id".to_string(),
                "virtual_label".to_string(),
                "stored_label".to_string(),
                "department".to_string(),
            ],
            source_columns: Some(vec![
                Some("id".to_string()),
                Some("virtual_label".to_string()),
                Some("stored_label".to_string()),
                Some("department".to_string()),
            ]),
            rows: vec![vec![json!(7), json!("sales-7"), json!("SALES"), json!("sales")]],
            dirty_rows: vec![(0, vec![(1, json!("support-7")), (2, json!("SUPPORT")), (3, json!("support"))])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["UPDATE `app`.`users` SET `department` = 'support' WHERE `id` = 7;"]);
        assert_eq!(
            result.rollback_statements,
            vec!["UPDATE `app`.`users` SET `department` = 'sales' WHERE `id` = 7 AND BINARY `department` = 'support';"]
        );
    }

    #[test]
    fn mysql_join_result_delete_targets_only_the_resolved_source_primary_key() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("lims".to_string()),
                table_name: "lims_batchs_simple".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![
                    column("id", "bigint", false, None),
                    column("sno", "integer", true, None),
                    column("batchs_id", "bigint", false, None),
                    column("simple_id", "bigint", false, None),
                ]),
            },
            columns: vec!["id".to_string(), "sno".to_string(), "batchs_id".to_string(), "simple_id".to_string()],
            source_columns: Some(vec![
                Some("id".to_string()),
                Some("sno".to_string()),
                Some("batchs_id".to_string()),
                Some("simple_id".to_string()),
            ]),
            rows: vec![vec![json!(2658055), json!(4), json!(57485), json!(492045)]],
            dirty_rows: vec![],
            deleted_rows: vec![0],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["DELETE FROM `lims`.`lims_batchs_simple` WHERE `id` = 2658055;"]);
    }

    /// The predicate a guard counts must be byte-identical to the one its
    /// statement carries, otherwise the safety decision is made against a
    /// different set of columns or values than the mutation actually uses.
    fn assert_guards_cover_statement_predicates(preparation: &DataGridSavePreparation) {
        for guard in &preparation.keyless_guards {
            assert_eq!(guard.max_matched_rows, 1);
            let predicate = guard
                .sql
                .split_once(" WHERE (")
                .and_then(|(_, rest)| rest.strip_suffix(')'))
                .unwrap_or_else(|| panic!("guard is not a counting query: {}", guard.sql));
            assert!(
                preparation.statements.iter().any(|statement| statement.contains(&format!("WHERE {predicate}"))),
                "no statement carries the guarded predicate {predicate:?}"
            );
        }
        for statement in &preparation.statements {
            let Some((_, predicate)) = statement.split_once(" WHERE ") else {
                continue;
            };
            let predicate = predicate.trim_end_matches(';');
            assert!(
                preparation.keyless_guards.iter().any(|guard| guard.sql.ends_with(&format!("WHERE ({predicate})"))),
                "predicate {predicate:?} is sent to the database without a guard"
            );
        }
    }

    #[test]
    fn guards_keyless_predicate_without_the_columns_the_predicate_omits() {
        // `build_row_where` drops result columns that have no source column, so
        // a guard built from the visible values would decide uniqueness using
        // `total`, a value the mutation predicate never mentions.
        let mut options = daily_stats_keyless_options();
        options.table_meta.columns =
            Some(vec![column("stat_date", "TEXT", false, None), column("period", "TEXT", true, None)]);
        options.columns = vec!["stat_date".to_string(), "period".to_string(), "total".to_string()];
        options.source_columns = Some(vec![Some("stat_date".to_string()), Some("period".to_string()), None]);
        options.rows =
            vec![vec![json!("2026-09-07"), Value::Null, json!(1)], vec![json!("2026-09-07"), Value::Null, json!(2)]];
        let result = prepare_data_grid_save(options);

        assert_eq!(result.validation_error, None);
        assert_eq!(result.keyless_guards.len(), 1);
        assert!(!result.keyless_guards[0].sql.contains("total"));
        assert_guards_cover_statement_predicates(&result);
    }

    #[test]
    fn guards_keyless_delete_and_deduplicates_identical_predicates() {
        let mut options = daily_stats_keyless_options();
        options.dirty_rows = vec![];
        options.deleted_rows = vec![0, 1];
        let result = prepare_data_grid_save(options);

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements.len(), 2);
        // Both rows produce the same predicate; one guard covers both.
        assert_eq!(result.keyless_guards.len(), 1);
        assert_guards_cover_statement_predicates(&result);
    }

    #[test]
    fn omits_keyless_guards_when_the_table_has_a_primary_key() {
        let mut options = daily_stats_keyless_options();
        options.table_meta.primary_keys = vec!["stat_date".to_string()];
        let result = prepare_data_grid_save(options);

        assert_eq!(result.validation_error, None);
        assert!(result.keyless_guards.is_empty());
    }

    #[test]
    fn rejects_keyless_edit_when_no_column_can_address_the_row() {
        // Every result column is computed, so `build_row_where` produces an
        // empty predicate: there is neither a row identifier nor anything a
        // server-side count could check, and the write must stay disabled.
        let mut options = daily_stats_keyless_options();
        options.source_columns = Some(vec![None, None]);
        let result = prepare_data_grid_save(options);

        assert_eq!(result.validation_error.as_deref(), Some(KEYLESS_UNIDENTIFIABLE_ROW_ERROR));
        assert!(result.statements.is_empty());
        assert!(result.keyless_guards.is_empty());
    }

    #[test]
    fn batches_mysql_updates_with_identical_values() {
        let mut options = mysql_people_save_options(3);
        options.dirty_rows =
            vec![(0, vec![(1, Value::Null)]), (1, vec![(1, Value::Null)]), (2, vec![(1, Value::Null)])];

        let result = prepare_data_grid_save(options);

        assert_eq!(result.validation_error, None);
        assert_eq!(
            result.statements,
            vec!["UPDATE `app`.`people` SET `status` = NULL WHERE (`id` = 1) OR (`id` = 2) OR (`id` = 3);"]
        );
        assert_eq!(result.rollback_statements.len(), 3);
    }

    #[test]
    fn preserves_mysql_update_order_across_different_values() {
        let mut options = mysql_people_save_options(3);
        options.dirty_rows =
            vec![(0, vec![(1, Value::Null)]), (1, vec![(1, json!("archived"))]), (2, vec![(1, Value::Null)])];

        let result = prepare_data_grid_save(options);

        assert_eq!(
            result.statements,
            vec![
                "UPDATE `app`.`people` SET `status` = NULL WHERE `id` = 1;",
                "UPDATE `app`.`people` SET `status` = 'archived' WHERE `id` = 2;",
                "UPDATE `app`.`people` SET `status` = NULL WHERE `id` = 3;",
            ]
        );
    }

    #[test]
    fn batches_mysql_deletes_and_deleted_row_rollbacks() {
        let mut options = mysql_people_save_options(3);
        options.deleted_rows = vec![0, 1, 2];

        let result = prepare_data_grid_save(options);
        for statement in &result.statements {
            println!("statement: {statement}");
        }

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["DELETE FROM `app`.`people` WHERE `id` IN (1, 2, 3);"]);
        assert_eq!(
            result.rollback_statements,
            vec!["INSERT INTO `app`.`people` (`id`, `status`) VALUES (1, 'active'), (2, 'active'), (3, 'active');"]
        );
    }

    #[test]
    fn batches_mysql_compound_key_deletes_keep_or_form() {
        let mut options = mysql_people_save_options(3);
        options.table_meta.primary_keys = vec!["id".to_string(), "status".to_string()];
        options.deleted_rows = vec![0, 1];

        let result = prepare_data_grid_save(options);

        assert_eq!(result.validation_error, None);
        assert_eq!(
            result.statements,
            vec![
                "DELETE FROM `app`.`people` WHERE (`id` = 1 AND `status` = 'active') OR (`id` = 2 AND `status` = 'active');"
            ]
        );
    }

    #[test]
    fn chunks_large_mysql_data_grid_batches() {
        let mut options = mysql_people_save_options(MYSQL_DATA_GRID_BATCH_MAX_ROWS + 1);
        options.deleted_rows = (0..options.rows.len()).collect();

        let result = prepare_data_grid_save(options);

        // IN lists are far more compact than OR chains, so the batch fits in a
        // single statement up to the row limit.
        assert_eq!(result.statements.len(), 1);
        assert!(result.statements[0].contains("`id` IN ("));
        assert!(result.statements[0].contains("501"));
        assert_eq!(result.rollback_statements.len(), 2);
        assert!(result.rollback_statements[0].contains("(500, 'active')"));
        assert_eq!(
            result.rollback_statements[1],
            "INSERT INTO `app`.`people` (`id`, `status`) VALUES (501, 'active');"
        );
    }

    #[test]
    fn mysql_data_grid_batch_respects_target_sql_size() {
        let mut options = mysql_people_save_options(2);
        options.rows[0][0] = json!("a".repeat(MYSQL_DATA_GRID_BATCH_TARGET_SQL_BYTES / 2));
        options.rows[1][0] = json!("b".repeat(MYSQL_DATA_GRID_BATCH_TARGET_SQL_BYTES / 2));
        options.deleted_rows = vec![0, 1];

        let result = prepare_data_grid_save(options);

        // The IN list stays within the SQL byte budget, splitting into a
        // second statement once the accumulated literals would exceed it.
        assert_eq!(result.statements.len(), 2);
        assert!(result.statements.iter().all(|statement| statement.len() <= MYSQL_DATA_GRID_BATCH_TARGET_SQL_BYTES));
    }

    #[test]
    fn keeps_keyless_mysql_writes_row_by_row() {
        let mut options = mysql_people_save_options(2);
        options.table_meta.primary_keys.clear();
        options.deleted_rows = vec![0, 1];

        let result = prepare_data_grid_save(options);

        assert_eq!(result.statements.len(), 2);
        assert_eq!(result.rollback_statements.len(), 2);
        assert!(result.statements.iter().all(|statement| !statement.contains(" OR ")));
    }

    #[test]
    fn casts_keyless_mysql_json_row_predicates() {
        let options = DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "documents".to_string(),
                primary_keys: vec![],
                columns: Some(vec![column("payload", "JSON", false, None), column("note", "varchar(32)", true, None)]),
            },
            columns: vec!["payload".to_string(), "note".to_string()],
            source_columns: None,
            rows: vec![vec![json!(r#"{"name":"before"}"#), json!("row-a")]],
            dirty_rows: vec![(0, vec![(0, json!(r#"{"name":"after"}"#))])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        };

        let update = prepare_data_grid_save(options.clone());
        assert_eq!(update.validation_error, None);
        assert_eq!(
            update.statements,
            vec![
                r#"UPDATE `app`.`documents` SET `payload` = '{"name":"after"}' WHERE `payload` = CAST('{"name":"before"}' AS JSON) AND BINARY `note` = 'row-a';"#
            ]
        );
        assert_eq!(
            update.rollback_statements,
            vec![
                r#"UPDATE `app`.`documents` SET `payload` = '{"name":"before"}' WHERE `payload` = CAST('{"name":"after"}' AS JSON) AND BINARY `note` = 'row-a' AND `payload` = CAST('{"name":"after"}' AS JSON);"#
            ]
        );

        let delete = prepare_data_grid_save(DataGridSaveStatementOptions {
            dirty_rows: vec![],
            deleted_rows: vec![0],
            ..options
        });
        assert_eq!(delete.validation_error, None);
        assert_eq!(
            delete.statements,
            vec![
                r#"DELETE FROM `app`.`documents` WHERE `payload` = CAST('{"name":"before"}' AS JSON) AND BINARY `note` = 'row-a';"#
            ]
        );
        assert_eq!(
            delete.rollback_statements,
            vec![r#"INSERT INTO `app`.`documents` (`payload`, `note`) VALUES ('{"name":"before"}', 'row-a');"#]
        );
    }

    #[test]
    fn mysql_json_save_assignments_are_compact_and_predicates_stay_cast() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "documents".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "int", false, None), column("payload", "JSON", true, None)]),
            },
            columns: vec!["id".to_string(), "payload".to_string()],
            source_columns: None,
            rows: vec![vec![json!(1), json!({"before": true})]],
            dirty_rows: vec![(0, vec![(1, json!([]))])],
            deleted_rows: vec![],
            new_rows: vec![vec![json!(2), json!({"nested": [1, 2]})]],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(
            result.statements,
            vec![
                "UPDATE `app`.`documents` SET `payload` = '[]' WHERE `id` = 1;",
                "INSERT INTO `app`.`documents` (`id`, `payload`) VALUES (2, '{\"nested\":[1,2]}');",
            ]
        );
        assert_eq!(
            result.rollback_statements,
            vec![
                "DELETE FROM `app`.`documents` WHERE `id` = 2;",
                "UPDATE `app`.`documents` SET `payload` = '{\"before\":true}' WHERE `id` = 1 AND `payload` = CAST('[]' AS JSON);",
            ]
        );
    }

    #[test]
    fn saves_empty_nullable_mysql_numeric_cell_as_null() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "employees".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "int(11)", false, None), column("age", "int(11)", true, None)]),
            },
            columns: vec!["id".to_string(), "age".to_string()],
            source_columns: None,
            rows: vec![vec![json!(2), json!(36)]],
            dirty_rows: vec![(0, vec![(1, json!(""))])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["UPDATE `employees` SET `age` = NULL WHERE `id` = 2;"]);
        assert_eq!(
            result.rollback_statements,
            vec!["UPDATE `employees` SET `age` = 36 WHERE `id` = 2 AND `age` IS NULL;"]
        );
    }

    #[test]
    fn saves_json_null_bigint_cell_as_unquoted_sql_null() {
        // Regression for #9970: bulk-editing a bigint column to NULL must emit
        // `= NULL`, never `= 'NULL'` / `= ''` (ERROR 1366 on MySQL/OceanBase).
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "orders".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "bigint", false, None), column("detail_id", "bigint", true, None)]),
            },
            columns: vec!["id".to_string(), "detail_id".to_string()],
            source_columns: None,
            rows: vec![vec![json!(7), json!(42)]],
            dirty_rows: vec![(0, vec![(1, Value::Null)])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["UPDATE `orders` SET `detail_id` = NULL WHERE `id` = 7;"]);
    }

    #[test]
    fn mysql_conditional_update_null_bigint_is_unquoted_sql_null() {
        let statement = build_data_grid_conditional_update_sql(DataGridConditionalUpdateSqlOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "orders".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "bigint", false, None), column("detail_id", "bigint", true, None)]),
            },
            column_name: "detail_id".to_string(),
            value: Value::Null,
            where_input: "tenant_id = 1".to_string(),
        });

        assert_eq!(statement, Some("UPDATE `orders` SET `detail_id` = NULL WHERE (tenant_id = 1);".to_string()));
    }

    #[test]
    fn keeps_empty_nullable_mysql_text_cell_as_empty_string() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "employees".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "int(11)", false, None), column("name", "varchar(50)", true, None)]),
            },
            columns: vec!["id".to_string(), "name".to_string()],
            source_columns: None,
            rows: vec![vec![json!(2), json!("Ada")]],
            dirty_rows: vec![(0, vec![(1, json!(""))])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["UPDATE `employees` SET `name` = '' WHERE `id` = 2;"]);
        assert_eq!(
            result.rollback_statements,
            vec!["UPDATE `employees` SET `name` = 'Ada' WHERE `id` = 2 AND BINARY `name` = '';"]
        );
    }

    #[test]
    fn preserves_mysql_text_cell_line_breaks() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "employees".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "int(11)", false, None), column("name", "varchar(50)", true, None)]),
            },
            columns: vec!["id".to_string(), "name".to_string()],
            source_columns: None,
            rows: vec![vec![json!(2), json!("Ada")]],
            dirty_rows: vec![(0, vec![(1, json!("111\n222"))])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["UPDATE `employees` SET `name` = '111\n222' WHERE `id` = 2;"]);
        assert_eq!(
            result.rollback_statements,
            vec!["UPDATE `employees` SET `name` = 'Ada' WHERE `id` = 2 AND BINARY `name` = '111\n222';"]
        );
    }

    #[test]
    fn formats_mysql_temporal_columns_by_target_type() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "policies".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![
                    column("id", "int", false, None),
                    column("insurance_start_time", "datetime", true, None),
                    column("raw_text", "varchar(64)", true, None),
                    column("coverage_day", "date", true, None),
                    column("start_clock", "time", true, None),
                ]),
            },
            columns: vec![
                "id".to_string(),
                "insurance_start_time".to_string(),
                "raw_text".to_string(),
                "coverage_day".to_string(),
                "start_clock".to_string(),
            ],
            source_columns: None,
            rows: vec![vec![
                json!(1),
                json!("2026-05-12T00:00:00+00:00"),
                json!("old"),
                json!("2026-05-12T00:00:00+00:00"),
                json!("2026-05-12T09:30:45+00:00"),
            ]],
            dirty_rows: vec![(
                0,
                vec![
                    (1, json!("2026-05-12T00:00:00+00:00")),
                    (2, json!("2026-05-12T00:00:00+00:00")),
                    (3, json!("2026-05-12T00:00:00+00:00")),
                    (4, json!("2026-05-12T09:30:45+00:00")),
                ],
            )],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(
            result.statements,
            vec!["UPDATE `policies` SET `insurance_start_time` = '2026-05-12 00:00:00', `raw_text` = '2026-05-12T00:00:00+00:00', `coverage_day` = '2026-05-12', `start_clock` = '09:30:45' WHERE `id` = 1;"]
        );
    }

    #[test]
    fn mysql_primary_key_text_predicates_do_not_use_binary_comparison() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "school".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![column("id", "varchar(32)", true, None), column("age", "varchar(8)", false, None)]),
            },
            columns: vec!["id".to_string(), "age".to_string()],
            source_columns: None,
            rows: vec![vec![json!("0001492305e412e88086bd582d2678e0"), json!("17")]],
            dirty_rows: vec![(0, vec![(1, json!("18"))])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(
            result.statements,
            vec!["UPDATE `school` SET `age` = '18' WHERE `id` = '0001492305e412e88086bd582d2678e0';"]
        );
    }

    #[test]
    fn mysql_row_text_predicates_use_binary_comparison_for_width_sensitive_edits() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: None,
                table_name: "parts".to_string(),
                primary_keys: vec![],
                columns: Some(vec![column("code", "varchar(32)", true, None)]),
            },
            columns: vec!["code".to_string()],
            source_columns: None,
            rows: vec![vec![json!("S471355(0)")]],
            dirty_rows: vec![(0, vec![(0, json!("S471355（0）"))])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(
            result.statements,
            vec!["UPDATE `parts` SET `code` = 'S471355（0）' WHERE BINARY `code` = 'S471355(0)';"]
        );
        assert_eq!(
            result.rollback_statements,
            vec![
                "UPDATE `parts` SET `code` = 'S471355(0)' WHERE BINARY `code` = 'S471355（0）' AND BINARY `code` = 'S471355（0）';"
            ]
        );
    }

    fn pk_column(name: &str, data_type: &str, nullable: bool, extra: Option<&str>) -> DataGridColumnInfo {
        DataGridColumnInfo {
            name: name.to_string(),
            data_type: data_type.to_string(),
            is_nullable: nullable,
            is_primary_key: true,
            column_default: None,
            extra: extra.map(ToString::to_string),
        }
    }

    #[test]
    fn prepare_data_grid_save_omits_empty_mysql_auto_increment_value() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "users".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![
                    pk_column("id", "BIGINT", false, Some("auto_increment")),
                    column("name", "VARCHAR", false, None),
                ]),
            },
            columns: vec!["id".to_string(), "name".to_string()],
            source_columns: None,
            rows: vec![],
            dirty_rows: vec![],
            deleted_rows: vec![],
            new_rows: vec![vec![json!(""), json!("Ada")]],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["INSERT INTO `app`.`users` (`name`) VALUES ('Ada');"]);
    }

    #[test]
    fn prepare_data_grid_save_omits_mysql_not_null_column_for_before_insert_trigger() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "events".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![
                    pk_column("id", "BIGINT", false, Some("auto_increment")),
                    column("trigger_value", "VARCHAR(64)", false, None),
                    column("payload", "VARCHAR(64)", false, None),
                ]),
            },
            columns: vec!["id".to_string(), "trigger_value".to_string(), "payload".to_string()],
            source_columns: None,
            rows: vec![],
            dirty_rows: vec![],
            deleted_rows: vec![],
            new_rows: vec![vec![Value::Null, Value::Null, json!("created")]],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["INSERT INTO `app`.`events` (`payload`) VALUES ('created');"]);
        assert!(result.rollback_statements.is_empty());
    }

    #[test]
    fn prepare_data_grid_save_uses_mysql_default_row_insert_for_trigger_only_rows() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "trigger_only".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![
                    pk_column("id", "BIGINT", false, Some("auto_increment")),
                    column("required_value", "VARCHAR(64)", false, None),
                ]),
            },
            columns: vec!["id".to_string(), "required_value".to_string()],
            source_columns: None,
            rows: vec![],
            dirty_rows: vec![],
            deleted_rows: vec![],
            new_rows: vec![vec![Value::Null, Value::Null]],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["INSERT INTO `app`.`trigger_only` () VALUES ();"]);
        assert!(result.rollback_statements.is_empty());
    }

    #[test]
    fn prepare_data_grid_save_uses_known_mysql_primary_key_for_trigger_rollback() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "events".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![
                    pk_column("id", "BIGINT", false, None),
                    column("trigger_value", "VARCHAR(64)", false, None),
                    column("payload", "VARCHAR(64)", false, None),
                ]),
            },
            columns: vec!["id".to_string(), "trigger_value".to_string(), "payload".to_string()],
            source_columns: None,
            rows: vec![],
            dirty_rows: vec![],
            deleted_rows: vec![],
            new_rows: vec![vec![json!(7), Value::Null, json!("created")]],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, None);
        assert_eq!(result.statements, vec!["INSERT INTO `app`.`events` (`id`, `payload`) VALUES (7, 'created');"]);
        assert_eq!(result.rollback_statements, vec!["DELETE FROM `app`.`events` WHERE `id` = 7;"]);
    }

    #[test]
    fn prepare_data_grid_save_still_rejects_mysql_null_update() {
        let result = prepare_data_grid_save(DataGridSaveStatementOptions {
            database_type: Some(DatabaseType::Mysql),
            identifier_quote: None,
            server_version: None,
            table_meta: DataGridTableMeta {
                catalog: None,
                database: None,
                schema: Some("app".to_string()),
                table_name: "events".to_string(),
                primary_keys: vec!["id".to_string()],
                columns: Some(vec![
                    pk_column("id", "BIGINT", false, Some("auto_increment")),
                    column("trigger_value", "VARCHAR(64)", false, None),
                ]),
            },
            columns: vec!["id".to_string(), "trigger_value".to_string()],
            source_columns: None,
            rows: vec![vec![json!(1), json!("existing")]],
            dirty_rows: vec![(0, vec![(1, Value::Null)])],
            deleted_rows: vec![],
            new_rows: vec![],
            include_database_name: false,
        });

        assert_eq!(result.validation_error, Some(r#"Column "trigger_value" does not allow NULL."#.to_string()));
        assert!(result.statements.is_empty());
    }
}
