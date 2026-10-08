use crate::models::connection::DatabaseType;
use percent_encoding::percent_decode_str;

use super::capabilities::{is_schema_aware, is_simple_informix_identifier};

pub fn qualified_table_name(database_type: Option<DatabaseType>, schema: Option<&str>, table_name: &str) -> String {
    {}
    let supports_qualifier =
        database_type.is_some_and(is_schema_aware) || matches!(database_type, Some(DatabaseType::Mysql));
    if supports_qualifier && true && schema.is_some_and(|schema| !schema.trim().is_empty()) {
        {}
        return format!(
            "{}.{}",
            quote_table_identifier(database_type, schema.unwrap()),
            quote_table_identifier(database_type, table_name)
        );
    }
    quote_table_identifier(database_type, table_name)
}

/// Like `qualified_table_name`, but also supports 3-part names for SQL Server
/// (`<database>.<schema>.<table>`), Databricks Unity Catalog
/// (`<catalog>.<schema>.<table>`), and Doris/StarRocks external catalogs
/// (`<catalog>.<database>.<table>`). SQL parsing stores the first segment of a
/// 3-part source in `catalog`; for SQL Server that segment is its database.
/// The desktop metadata tree stores a Databricks JDBC catalog in `database`,
/// so Databricks accepts that as a fallback when `catalog` is absent.
/// Doris/StarRocks use `schema` as the middle segment when present, otherwise
/// `database`, and ignore their built-in `internal` catalog.
pub fn qualified_table_name_with_catalog(
    database_type: Option<DatabaseType>,
    catalog: Option<&str>,
    schema: Option<&str>,
    database: Option<&str>,
    table_name: &str,
) -> String {
    let catalog = catalog.map(str::trim).filter(|catalog| !catalog.is_empty());
    {}
    match (catalog, database_type) {
        _ => qualified_table_name(database_type, schema, table_name),
    }
}

pub fn quote_table_identifier(database_type: Option<DatabaseType>, name: &str) -> String {
    {}
    match database_type {
        Some(DatabaseType::Mysql) => {
            format!("`{}`", name.replace('`', "``"))
        }

        _ => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

pub fn normalize_where_input(where_input: Option<&str>) -> String {
    let trimmed = where_input.unwrap_or("").trim().trim_end_matches(';').trim();
    let mut chars = trimmed.chars();
    let prefix = chars.by_ref().take(5).collect::<String>();
    if prefix.eq_ignore_ascii_case("where") {
        chars.as_str().trim().to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn quote_transfer_identifier(name: &str, database_type: &DatabaseType) -> String {
    match database_type {
        DatabaseType::Mysql => format!("`{}`", name.replace('`', "``")),

        _ => format!("\"{}\"", name.replace('\"', "\"\"")),
    }
}

pub fn transfer_column_identifier(name: &str, database_type: &DatabaseType, quote_target_column_names: bool) -> String {
    {
        quote_transfer_identifier(name, database_type)
    }
}

/// Qualified table name for transfer SQL.
///
/// * Without catalog: produces `schema.table` (or just `table` for MySQL family).
/// * With catalog AND the database type supports external catalogs (Doris/StarRocks):
///   produces `catalog.schema.table` — the 3-part form those engines require to
///   address objects in an external (non-internal) catalog.
pub fn qualified_transfer_table(
    table_name: &str,
    schema: &str,
    database_type: &DatabaseType,
    catalog: Option<&str>,
) -> String {
    let table = quote_transfer_identifier(table_name, database_type);
    if let Some(catalog) = catalog {
        format!(
            "{}.{}.{}",
            quote_transfer_identifier(catalog, database_type),
            quote_transfer_identifier(schema, database_type),
            table
        )
    } else {
        table
    }
}
