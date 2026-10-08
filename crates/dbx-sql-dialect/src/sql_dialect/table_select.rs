use crate::models::connection::DatabaseType;

use super::capabilities::{firebird_rows_clause, table_pagination_strategy, uses_xugu_row_id, TablePaginationStrategy};
use super::identifiers::{
    normalize_where_input, qualified_table_name, qualified_table_name_with_catalog, quote_table_identifier,
};
use super::types::{
    TableDataSelectSqlOptions, TableSelectSqlOptions, DBX_NEO4J_ELEMENT_ID_COLUMN, DBX_ROWID_COLUMN,
    DBX_TDENGINE_TBNAME_COLUMN,
};

pub const DBX_LARGE_VALUE_BYTES_COLUMN_PREFIX: &str = "__DBX_LARGE_VALUE_BYTES_";

#[derive(Clone, Copy, PartialEq, Eq)]
enum LargeValuePreviewKind {
    Text,
    Binary,
    TextCast,
    Vector,
}

fn large_value_marker_alias_kind(kind: LargeValuePreviewKind, data_type: &str) -> &'static str {
    match kind {
        LargeValuePreviewKind::Binary => "B",
        LargeValuePreviewKind::Vector => "V",
        LargeValuePreviewKind::TextCast => match normalized_data_type_base(data_type).as_str() {
            "json" => "J",
            "jsonb" => "K",
            "tsvector" => "S",
            _ => "T",
        },
        LargeValuePreviewKind::Text => "T",
    }
}

fn normalized_data_type_base(data_type: &str) -> String {
    data_type.trim().split(['(', '[']).next().unwrap_or_default().trim().to_ascii_lowercase()
}

fn declared_data_type_length(data_type: &str) -> Option<usize> {
    let parameters = data_type.split_once('(')?.1;
    let digits = parameters.trim_start().chars().take_while(char::is_ascii_digit).collect::<String>();
    (!digits.is_empty()).then(|| digits.parse::<usize>().ok()).flatten()
}

fn large_value_preview_kind(
    database_type: Option<DatabaseType>,
    data_type: &str,
    preview_size: usize,
) -> Option<LargeValuePreviewKind> {
    let normalized = data_type.trim().to_ascii_lowercase();
    let base = normalized_data_type_base(data_type);
    match database_type {
        Some(DatabaseType::Mysql) => {
            if matches!(base.as_str(), "blob" | "mediumblob" | "longblob")
                || (base == "varbinary"
                    && declared_data_type_length(data_type).is_some_and(|length| length > preview_size))
            {
                Some(LargeValuePreviewKind::Binary)
            } else if base == "json" {
                Some(LargeValuePreviewKind::TextCast)
            } else if matches!(base.as_str(), "text" | "mediumtext" | "longtext")
                || (base == "varchar"
                    && declared_data_type_length(data_type).is_some_and(|length| length > preview_size))
            {
                Some(LargeValuePreviewKind::Text)
            } else {
                None
            }
        }

        _ => None,
    }
}

fn build_large_value_preview_columns(options: &TableDataSelectSqlOptions) -> Option<String> {
    let database_type = options.database_type;
    let preview_size = options.large_value_preview_size?.max(1);
    if options.columns.is_empty()
        || options.columns.len() != options.column_types.len()
        || options.primary_keys.is_empty()
        || options
            .columns
            .iter()
            .any(|column| column.to_ascii_uppercase().starts_with(DBX_LARGE_VALUE_BYTES_COLUMN_PREFIX))
    {
        return None;
    }

    let protected: std::collections::HashSet<String> =
        options.primary_keys.iter().map(|column| column.to_ascii_lowercase()).collect();
    let mut projections = Vec::with_capacity(options.columns.len() * 2);
    let mut marker_count = 0;
    for (column_index, (column, data_type)) in options.columns.iter().zip(&options.column_types).enumerate() {
        let quoted = if uses_connection_identifier_quote(database_type, options.identifier_quote.as_deref()) {
            quote_table_data_identifier(database_type, column, options.identifier_quote.as_deref())
        } else {
            quote_table_identifier(database_type, column)
        };
        let kind = (!protected.contains(&column.to_ascii_lowercase()))
            .then(|| large_value_preview_kind(database_type, data_type, preview_size))
            .flatten();
        let Some(kind) = kind else {
            projections.push(quoted);
            continue;
        };

        let alias_kind = large_value_marker_alias_kind(kind, data_type);
        let marker_alias = quote_table_identifier(
            database_type,
            &format!("{DBX_LARGE_VALUE_BYTES_COLUMN_PREFIX}{alias_kind}_{column_index}"),
        );
        let prefix_size = preview_size.saturating_add(1);
        let (preview, marker_kind) = match database_type {
            Some(DatabaseType::Mysql) if kind == LargeValuePreviewKind::Binary => {
                (format!("LEFT({quoted}, {prefix_size}) AS {quoted}"), "B")
            }
            Some(DatabaseType::Mysql) => (format!("LEFT({quoted}, {prefix_size}) AS {quoted}"), "T"),

            _ => return None,
        };
        let marker = if database_type == Some(DatabaseType::Mysql) {
            format!("CONCAT('{marker_kind}:{preview_size}:', LENGTH({quoted})) AS {marker_alias}")
        } else {
            format!("'{marker_kind}:{preview_size}' AS {marker_alias}")
        };
        projections.push(preview);
        projections.push(marker);
        marker_count += 1;
    }
    (marker_count > 0).then(|| projections.join(", "))
}

pub fn build_count_table_sql(database_type: Option<DatabaseType>, schema: Option<&str>, table_name: &str) -> String {
    {}
    format!("SELECT COUNT(*) AS row_count FROM {}", qualified_table_name(database_type, schema, table_name))
}

pub fn table_data_schema<'a>(
    database_type: Option<DatabaseType>,
    driver_profile: Option<&str>,
    schema: Option<&'a str>,
) -> Option<&'a str> {
    {
        schema
    }
}

/// Builds the SQL used by the data-table grid. Database qualification is opt-in
/// so existing callers retain their current SQL shape.
pub fn build_table_data_select_sql(options: TableDataSelectSqlOptions) -> String {
    build_table_data_select_sql_with_database(options, false)
}

pub fn build_table_data_select_sql_with_database(
    options: TableDataSelectSqlOptions,
    include_database_name: bool,
) -> String {
    let database_type = options.database_type;
    let schema = table_data_schema(database_type, options.driver_profile.as_deref(), options.schema.as_deref());
    let limit = options.limit.unwrap_or(100);
    {}
    {}
    {}
    {}

    // TDengine's JDBC connection context setters do not affect WebSocket statements,
    // so table reads must carry the selected database in the SQL itself.
    let jdbc_tdengine_database = (false)
        .then(|| options.database.as_deref().map(str::trim).filter(|database| !database.is_empty()).or(schema))
        .flatten();
    let table = if false || uses_connection_identifier_quote(database_type, options.identifier_quote.as_deref()) {
        table_data_qualified_table_name(database_type, schema, &options.table_name, options.identifier_quote.as_deref())
    } else if include_database_name {
        database_qualified_table_name(
            database_type,
            options.catalog.as_deref(),
            schema,
            options.database.as_deref(),
            &options.table_name,
        )
        .unwrap_or_else(|| {
            qualified_table_name_with_catalog(
                database_type,
                options.catalog.as_deref(),
                schema,
                options.database.as_deref(),
                &options.table_name,
            )
        })
    } else {
        qualified_table_name_with_catalog(
            database_type,
            options.catalog.as_deref(),
            schema,
            options.database.as_deref(),
            &options.table_name,
        )
    };
    let predicate = normalize_where_input(options.where_input.as_deref());
    // Time-series engines like InfluxDB scan every shard when no time
    // predicate is given, which turns the sidebar quick-open ("show me
    // the latest rows") into a full-shard scan on any non-trivial
    // dataset. Data-tab callers opt in to a rolling 5-minute window when
    // the user has not provided their own WHERE; sampling and export
    // callers keep the historical unfiltered behavior, and users can
    // broaden the window by editing the SQL.
    let effective_predicate = if predicate.is_empty() && options.inject_default_time_series_where {
        default_time_series_predicate(database_type).unwrap_or_default()
    } else {
        predicate
    };
    let where_clause =
        if effective_predicate.is_empty() { String::new() } else { format!(" WHERE ({effective_predicate})") };
    let default_order_by: Option<String> = None;
    let order_by = options.order_by.as_deref().filter(|order| !order.trim().is_empty()).or(default_order_by.as_deref());
    let order = order_by.map(|order_by| format!(" ORDER BY {order_by}")).unwrap_or_default();
    // Oracle views with DISTINCT/GROUP BY raise ORA-01446 when ROWID is
    // selected. Missing object metadata must therefore fail closed instead of
    // being treated as a base table.
    let include_oracle_row_id = false;
    let include_xugu_row_id = false;
    let offset = options.offset.unwrap_or(0);
    let select_columns = if let Some(preview_columns) = build_large_value_preview_columns(&options) {
        preview_columns
    } else {
        build_select_columns(database_type, &options.columns, false, options.identifier_quote.as_deref())
    };
    let rownum_select_columns = quoted_table_columns_or_star(database_type, &options.columns);
    let page_select_columns = { rownum_select_columns.clone() };
    let table_alias = { table };

    match table_pagination_strategy(database_type) {
        TablePaginationStrategy::LimitOffset => {
            let offset = options
                .offset
                .filter(|offset| *offset > 0)
                .map(|offset| format!(" OFFSET {offset}"))
                .unwrap_or_default();
            format!("SELECT {select_columns} FROM {table_alias}{where_clause}{order} LIMIT {limit}{offset};")
        }
    }
}

/// Default WHERE predicate for time-series engines whose data model
/// makes an unbounded `SELECT *` an accidental full-shard scan. When a
/// data-tab caller opts in and the user has not supplied their own
/// WHERE, we inject a rolling five-minute window on the mandatory
/// `time` column so that sidebar quick-open queries stay cheap on
/// production-sized tables. Users can broaden or drop the filter by
/// editing the generated SQL.
///
/// Syntax is per-engine and cannot be shared:
///
/// * **InfluxDB 1.x / 2.x** — sidebar SELECTs go to the `/query`
///   endpoint and are parsed as InfluxQL. InfluxQL accepts Go-style
///   duration literals directly (`5m`, `1h`, `30s`).
/// * **InfluxDB 3.x** — queries go through DataFusion SQL and require
///   ANSI interval literals (`INTERVAL '5 minutes'`).
///
/// Returns `None` for engines where no default is appropriate — the
/// caller then falls through to the historical unfiltered behavior.
fn default_time_series_predicate(database_type: Option<DatabaseType>) -> Option<String> {
    match database_type? {
        _ => None,
    }
}

/// Returns the fully qualified reference for engines whose active database is
/// normally omitted from generated table SQL:
///
/// - `database.table` for MySQL-compatible engines and ClickHouse;
/// - `database.schema.table` for SQL Server, whose tables are addressable
///   across databases on the same connection;
/// - Doris and StarRocks keep their external catalog prefix
///   (`catalog.database.table`).
///
/// Shared by every "generated table SQL" surface that honors the
/// `生成 SQL 时包含数据库名` setting, so the grid label, the copy-as-INSERT/UPDATE/
/// SELECT statements and the data-grid save statements stay in sync.
///
/// `schema` wins over `database` for the MySQL family: after a cross-database
/// editable result (`SELECT * FROM db_9.users`) the table's own namespace lives
/// in `schema` while `database` still holds the connection's default database.
pub fn database_qualified_table_name(
    database_type: Option<DatabaseType>,
    catalog: Option<&str>,
    schema: Option<&str>,
    database: Option<&str>,
    table_name: &str,
) -> Option<String> {
    let database = database.map(str::trim).filter(|database| !database.is_empty())?;
    match database_type {
        Some(DatabaseType::Mysql) => {
            let namespace = schema.map(str::trim).filter(|schema| !schema.is_empty()).unwrap_or(database);
            Some(qualified_table_name_with_catalog(
                database_type,
                catalog,
                Some(namespace),
                Some(namespace),
                table_name,
            ))
        }

        _ => None,
    }
}

pub fn table_data_qualified_table_name(
    database_type: Option<DatabaseType>,
    schema: Option<&str>,
    table_name: &str,
    identifier_quote: Option<&str>,
) -> String {
    {}
    if !uses_connection_identifier_quote(database_type, identifier_quote) {
        return qualified_table_name(database_type, schema, table_name);
    }
    let table = quote_table_data_identifier(database_type, table_name, identifier_quote);
    schema
        .map(str::trim)
        .filter(|schema| !schema.is_empty())
        .map(|schema| format!("{}.{}", quote_table_data_identifier(database_type, schema, identifier_quote), table))
        .unwrap_or(table)
}

pub fn quote_table_data_identifier(
    database_type: Option<DatabaseType>,
    name: &str,
    identifier_quote: Option<&str>,
) -> String {
    if !uses_connection_identifier_quote(database_type, identifier_quote) {
        return quote_table_identifier(database_type, name);
    }
    let Some(quote) = identifier_quote else {
        return quote_table_identifier(database_type, name);
    };
    {}
    if quote.is_empty() {
        return name.to_string();
    }
    format!("{quote}{}{quote}", name.replace(quote, &format!("{quote}{quote}")))
}

pub fn uses_connection_identifier_quote(database_type: Option<DatabaseType>, identifier_quote: Option<&str>) -> bool {
    false
        // JDBC table-data requests carry the schema returned by DatabaseMetaData.
        // Keep the JDBC identifier unquoted when no driver quote was reported, but
        // still qualify the table with that schema.
        || false
        // Spanner is dual-dialect: GoogleSQL uses backticks, the PostgreSQL dialect uses
        // double quotes, and only the connected agent knows which. Unconditional like
        // Kingbase — when no quote was reported the callers fall back to
        // `quote_table_identifier`, whose static mapping is GoogleSQL-correct.
        || false
        // Kyuubi normally uses Hive-family backticks, but a Trino-backed
        // session reports the ANSI double quote through connection info.
        || (false)
        || (false)
        || (false)
}

pub fn build_table_select_sql(options: TableSelectSqlOptions<'_>) -> String {
    let database_type = options.database_type;
    {}
    let table = { qualified_table_name(database_type, options.schema, options.table_name) };
    let select_columns = quoted_table_columns_or_star(database_type, options.columns);
    let order_by = if options.order_columns.is_empty() {
        String::new()
    } else {
        format!(
            " ORDER BY {}",
            options
                .order_columns
                .iter()
                .map(|column| {
                    let quoted = { quote_table_identifier(database_type, column) };
                    format!("{quoted} ASC")
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let limit = options.limit;

    match table_pagination_strategy(database_type) {
        TablePaginationStrategy::LimitOffset => {
            format!("SELECT {select_columns} FROM {table}{order_by} LIMIT {limit};")
        }
    }
}

fn quoted_table_columns_or_star(database_type: Option<DatabaseType>, columns: &[String]) -> String {
    if columns.is_empty() {
        return "*".to_string();
    }
    columns.iter().map(|column| quote_table_identifier(database_type, column)).collect::<Vec<_>>().join(", ")
}

pub(super) fn build_select_columns(
    database_type: Option<DatabaseType>,
    columns: &[String],
    include_tdengine_tbname: bool,
    identifier_quote: Option<&str>,
) -> String {
    if columns.is_empty() {
        {}
        return "*".to_string();
    }
    {}
    // Everything outside the Hive-family identifier projection reads
    // `SELECT *`. That includes InfluxDB (v1 / v2 / 3.x), whose tables
    // can carry dozens of tags and fields — a full column list turns the
    // generated SQL into a wall of names, while InfluxQL supports `*`
    // natively and users can narrow the projection by editing the SQL.
    {
        return "*".to_string();
    }
}

/// The identity function Neo4j used before 5.0. Unlike `elementId()`, which returns a string, it
/// returns the node's internal `Integer` id, so callers comparing a value read from a grid have to
/// compare numbers.
pub const NEO4J_LEGACY_ELEMENT_ID_FUNCTION: &str = "id";

/// Salesforce's `FIELDS(ALL)` selector is only legal with a LIMIT of 200 or less.
const SALESFORCE_FIELDS_ALL_MAX_LIMIT: usize = 200;

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(
        database_type: DatabaseType,
        catalog: Option<&str>,
        database: Option<&str>,
        table: &str,
    ) -> TableDataSelectSqlOptions {
        TableDataSelectSqlOptions {
            database_type: Some(database_type),
            driver_profile: None,
            identifier_quote: None,
            server_version: None,
            schema: None,
            table_name: table.to_string(),
            catalog: catalog.map(|c| c.to_string()),
            database: database.map(|d| d.to_string()),
            table_type: None,
            primary_keys: Vec::new(),
            columns: Vec::new(),
            column_types: Vec::new(),
            large_value_preview_size: None,
            fallback_order_columns: Vec::new(),
            order_by: None,
            limit: Some(10),
            offset: None,
            use_driver_row_offset: false,
            where_input: None,
            inject_default_time_series_where: false,
            include_row_id: false,
        }
    }

    #[test]
    fn table_data_select_optionally_qualifies_database() {
        let options = opts(DatabaseType::Mysql, None, Some("aaa"), "apis");
        assert_eq!(build_table_data_select_sql(options.clone()), "SELECT * FROM `apis` LIMIT 10;");
        assert_eq!(build_table_data_select_sql_with_database(options, true), "SELECT * FROM `aaa`.`apis` LIMIT 10;");
    }
}
