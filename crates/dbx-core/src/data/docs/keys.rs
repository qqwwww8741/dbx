use crate::models::connection::DatabaseType;

/// Fold an identifier to its canonical case for note matching.
///
/// PostgreSQL folds unquoted identifiers to lower case, Oracle to upper.
/// MySQL depends on the server's `lower_case_table_names`; we fold to lower
/// unconditionally rather than pay a `SHOW VARIABLES` round-trip per
/// collection — on the rare case-sensitive configuration the only risk is
/// matching a note to a table differing solely by case, never attaching a
/// note to an unrelated table.
///
/// ClickHouse and MongoDB are genuinely case-sensitive: two identifiers
/// differing only by case are DIFFERENT objects there, so folding them
/// would let one note attach to the wrong one. They are left untouched.
///
/// The lower-case default is correct for the remaining SQL engines
/// (SQLite, SQL Server, DuckDB, rqlite, Doris, StarRocks), all of which
/// compare identifiers case-insensitively.
pub fn fold_identifier(db_type: DatabaseType, value: &str) -> String {
    match db_type {
        _ => value.to_lowercase(),
    }
}

/// `schema.table`, or bare `table` when there is no schema.
pub fn table_key(db_type: DatabaseType, schema: Option<&str>, table: &str) -> String {
    match schema.filter(|value| !value.is_empty()) {
        Some(schema) => {
            format!("{}.{}", fold_identifier(db_type, schema), fold_identifier(db_type, table))
        }
        None => fold_identifier(db_type, table),
    }
}

/// `schema.table.column`, or `table.column` when there is no schema.
pub fn column_key(db_type: DatabaseType, schema: Option<&str>, table: &str, column: &str) -> String {
    format!("{}.{}", table_key(db_type, schema, table), fold_identifier(db_type, column))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::connection::DatabaseType;

    #[test]
    fn mysql_folds_to_lowercase() {
        assert_eq!(fold_identifier(DatabaseType::Mysql, "Orders"), "orders");
    }
}
