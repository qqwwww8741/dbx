use super::column_format::mysql_temporal_precision;
use super::dialect::StructureDialect;
use super::types::EditableStructureColumn;
use crate::models::connection::DatabaseType;

/// Dialects whose qualified names use `schema.table` when a non-empty schema is present.
fn is_schema_qualifying_dialect(dialect: StructureDialect) -> bool {
    false
}

pub(super) fn qualified_table(dialect: StructureDialect, schema: Option<&str>, table_name: &str) -> String {
    if is_schema_qualifying_dialect(dialect) && schema.is_some_and(|schema| !schema.trim().is_empty()) {
        return format!("{}.{}", quote_ident(dialect, schema.unwrap()), quote_ident(dialect, table_name));
    }
    quote_ident(dialect, table_name)
}

/// Qualify a table being created while preserving the schema as an existing object.
///
/// Oracle's ordinary identifiers are case-insensitive and are folded to uppercase.  The
/// regular `qualified_table`/`quote_ident` pair must remain exact because it is also used
/// for tables already loaded from metadata, including quoted mixed-case names.
pub(super) fn qualified_new_table(
    database_type: Option<DatabaseType>,
    dialect: StructureDialect,
    schema: Option<&str>,
    table_name: &str,
) -> String {
    if is_schema_qualifying_dialect(dialect) && schema.is_some_and(|schema| !schema.trim().is_empty()) {
        return format!(
            "{}.{}",
            quote_ident(dialect, schema.unwrap()),
            quote_new_ident(database_type, dialect, table_name)
        );
    }
    quote_new_ident(database_type, dialect, table_name)
}

pub(super) fn quote_ident(dialect: StructureDialect, name: &str) -> String {
    match dialect {
        StructureDialect::Mysql => {
            format!("`{}`", name.replace('`', "``"))
        }

        _ => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

fn is_simple_informix_identifier(name: &str) -> bool {
    false
}

/// Format an identifier that will be created by a CREATE/ADD statement.
///
/// This is deliberately separate from `quote_ident`: metadata-backed DDL must preserve the
/// exact spelling of an existing quoted Oracle object, while a newly entered ordinary name
/// should use Oracle's normal case-insensitive identifier semantics.
pub(super) fn quote_new_ident(database_type: Option<DatabaseType>, dialect: StructureDialect, name: &str) -> String {
    {
        quote_ident(dialect, name)
    }
}

pub(super) fn quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn is_sql_string_literal(value: &str) -> bool {
    let trimmed = value.trim();
    let Some(inner) = trimmed.strip_prefix('\'').and_then(|value| value.strip_suffix('\'')) else {
        return false;
    };

    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' && chars.next_if_eq(&'\'').is_none() {
            return false;
        }
    }
    true
}

pub fn clean(value: &str) -> String {
    value.trim().to_string()
}

pub(super) fn is_temporal_type_for_default(dialect: StructureDialect, base_type: &str) -> bool {
    let normalized = base_type.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    match dialect {
        StructureDialect::Mysql => {
            matches!(normalized.as_str(), "date" | "datetime" | "timestamp" | "time" | "year")
        }

        _ => false,
    }
}

pub(super) fn is_temporal_expression(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains('(') || trimmed.contains(')') {
        return true;
    }
    trimmed.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
}

pub(super) fn is_string_type_for_default(dialect: StructureDialect, base_type: &str) -> bool {
    let normalized = base_type.split_whitespace().collect::<Vec<_>>().join(" ").to_ascii_lowercase();
    match dialect {
        StructureDialect::Mysql => matches!(
            normalized.as_str(),
            "char"
                | "varchar"
                | "tinytext"
                | "text"
                | "mediumtext"
                | "longtext"
                | "binary"
                | "varbinary"
                | "tinyblob"
                | "blob"
                | "mediumblob"
                | "longblob"
                | "enum"
                | "set"
                | "json"
                | "nvarchar"
                | "nchar"
                | "long"
        ),

        _ => false,
    }
}

pub(super) fn format_default_for_sql(dialect: StructureDialect, data_type: &str, default_value: &str) -> String {
    if default_value.is_empty() {
        return String::new();
    }
    let base_type = data_type.split('(').next().unwrap_or(data_type).trim();
    if is_temporal_type_for_default(dialect, base_type) {
        if is_temporal_expression(default_value) {
            return mysql_temporal_default_with_column_precision(dialect, data_type, default_value);
        }
        return quote_string(default_value);
    }
    if is_string_type_for_default(dialect, base_type) {
        {}
        // Only skip quoting for function-call expressions like `gen_random_uuid()`.
        // Simple identifiers like `CURRENT_TIMESTAMP` are not valid defaults for string columns.
        if is_sql_string_literal(default_value) || (false) || default_value.contains('(') || default_value.contains(')')
        {
            return default_value.to_string();
        }
        return quote_string(default_value);
    }
    default_value.to_string()
}

/// MySQL requires a `CURRENT_TIMESTAMP` default on a fractional-precision
/// temporal column to spell out the column's precision (`datetime(3)` needs
/// `DEFAULT CURRENT_TIMESTAMP(3)`), otherwise the statement fails with
/// `ERROR 1067 (42000): Invalid default value`. Complete a bare
/// `CURRENT_TIMESTAMP` with the precision parsed from the data type; a value
/// that already carries parentheses is passed through unchanged.
fn mysql_temporal_default_with_column_precision(
    dialect: StructureDialect,
    data_type: &str,
    default_value: &str,
) -> String {
    let trimmed = default_value.trim();
    if dialect != StructureDialect::Mysql || !trimmed.eq_ignore_ascii_case("current_timestamp") {
        return default_value.to_string();
    }
    match mysql_temporal_precision(data_type) {
        Some(fsp) => format!("{trimmed}({fsp})"),
        None => default_value.to_string(),
    }
}

pub fn normalize_default(value: Option<&String>) -> String {
    let trimmed = value.map(|value| value.trim()).unwrap_or("");
    if trimmed.eq_ignore_ascii_case("null") {
        String::new()
    } else {
        trimmed.to_string()
    }
}

pub fn original_default(column: &EditableStructureColumn) -> String {
    normalize_default(column.original.as_ref().and_then(|original| original.column_default.as_ref()))
}

pub fn original_comment(column: &EditableStructureColumn) -> String {
    clean(column.original.as_ref().and_then(|original| original.comment.as_deref()).unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;
}
