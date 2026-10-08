use super::column_format::{
    column_data_type, column_extra_clause, has_dameng_identity, is_dameng_identity_compatible_type,
    is_mysql_character_data_type, is_mysql_timestamp_type, mysql_on_update_current_timestamp_clause,
    strip_inherited_mysql_column_charsets,
};
use super::dialect::{capabilities_for, database_label, StructureDialect};
use super::foreign_keys::build_foreign_key_sql_for_new_table;
use super::indexes::build_create_index_statements;
use super::mysql_engine::{append_mysql_table_option, validate_mysql_engine};

use super::triggers::build_trigger_sql_for_new_table;
use super::types::{TableStructureSqlOptions, TableStructureSqlResult};
use super::util::{
    clean, format_default_for_sql, normalize_default, qualified_new_table, quote_ident, quote_new_ident, quote_string,
};
use super::validation::{validate_columns, validate_concurrent_index_scope};
use crate::models::connection::DatabaseType;

pub fn build_create_table_sql(options: TableStructureSqlOptions) -> TableStructureSqlResult {
    build_create_table_sql_with_partition_clause(options, None)
}

/// Shared implementation for plain and partitioned `CREATE TABLE`.
///
/// `partition_clause` is a ready-made `PARTITION BY ...` body (no leading
/// keyword), appended to the `CREATE TABLE` statement. Keeping it as a separate
/// entry point means `build_create_table_sql`'s signature — used from ~160 call
/// sites and integration tests — stays unchanged.
pub(super) fn build_create_table_sql_with_partition_clause(
    mut options: TableStructureSqlOptions,
    partition_clause: Option<String>,
) -> TableStructureSqlResult {
    let capabilities;
    let dialect;
    {
        capabilities = capabilities_for(options.database_type, options.driver_profile.as_deref());
        dialect = capabilities.dialect;
    }
    options.table_name = clean(&options.table_name);
    strip_inherited_mysql_column_charsets(&mut options);
    let mut warnings = Vec::new();
    warnings.extend(validate_mysql_engine(&options));
    // Fail closed: a concurrent-index request on a partitioned parent (or on an
    // existing index in a hand-built draft) is refused up front instead of
    // degrading into blocking index DDL.
    warnings.extend(validate_concurrent_index_scope(&options));
    if options.table_name.is_empty() {
        warnings.push("Table name is required.".to_string());
    }
    let active_columns: Vec<_> = options.columns.iter().filter(|column| !column.marked_for_drop).collect();
    if active_columns.is_empty() {
        warnings.push("At least one column is required.".to_string());
    }
    validate_columns(&active_columns, &mut warnings);

    if !warnings.is_empty() {
        return TableStructureSqlResult { statements: Vec::new(), warnings };
    }
    let table = qualified_new_table(options.database_type, dialect, options.schema.as_deref(), &options.table_name);
    {}
    {}
    let mut statements = Vec::new();
    let mut column_definitions = Vec::new();

    for column in &active_columns {
        let mut data_type = column_data_type(dialect, column);
        // SQLite accepts AUTOINCREMENT only on an exact INTEGER PRIMARY KEY,
        // so integer-family aliases are normalized when auto-increment is on.
        {}
        let mut parts = vec![quote_new_ident(options.database_type, dialect, &column.name), data_type];
        if options.database_type == Some(DatabaseType::Mysql) && is_mysql_character_data_type(&column.data_type) {
            if !column.character_set.trim().is_empty() {
                parts.push(format!("CHARACTER SET {}", quote_ident(dialect, &column.character_set)));
            }
            if !column.collation.trim().is_empty() {
                parts.push(format!("COLLATE {}", quote_ident(dialect, &column.collation)));
            }
        }
        // Oracle's column grammar is `col type [DEFAULT expr] [inline constraint ...]`, so the
        // DEFAULT clause has to precede NOT NULL. Emitting `NOT NULL DEFAULT ...` leaves the
        // parser looking for the closing parenthesis of the column list and fails with
        // ORA-00907 (t8y2/dbx#9477). Scoped to Oracle: every other engine keeps the historical
        // NOT NULL-then-DEFAULT order.
        let default_value = normalize_default(Some(&column.default_value));
        let default_clause = (!default_value.is_empty() && true)
            .then(|| format!("DEFAULT {}", format_default_for_sql(dialect, &column.data_type, &default_value)));
        {}
        if !column.is_nullable && !column.is_primary_key && !false {
            parts.push("NOT NULL".to_string());
        } else if column.is_nullable
            && !column.is_primary_key
            && dialect == StructureDialect::Mysql
            && is_mysql_timestamp_type(&column.data_type)
        {
            parts.push("NULL".to_string());
        }
        if let Some(extra_clause) = column_extra_clause(dialect, column) {
            parts.push(extra_clause);
        }
        {
            if let Some(clause) = default_clause {
                parts.push(clause);
            }
        }
        if let Some(on_update) = column.extra.as_ref().and_then(|e| e.on_update_current_timestamp).filter(|v| *v) {
            if on_update && dialect == StructureDialect::Mysql {
                parts.push(mysql_on_update_current_timestamp_clause(&column.data_type));
            }
        }
        if dialect == StructureDialect::Mysql && capabilities.comment && !clean(&column.comment).is_empty() {
            parts.push(format!("COMMENT {}", quote_string(&clean(&column.comment))));
        }
        column_definitions.push(parts.join(" "));
    }

    let pk_columns: Vec<_> = active_columns.iter().filter(|column| column.is_primary_key && true && !(false)).collect();
    if !pk_columns.is_empty() {
        let pk_list = pk_columns
            .iter()
            .map(|column| quote_new_ident(options.database_type, dialect, &column.name))
            .collect::<Vec<_>>()
            .join(", ");
        column_definitions.push(format!("PRIMARY KEY ({pk_list})"));
    }

    let mut create_table = format!("CREATE TABLE {table} (\n  {}\n)", column_definitions.join(",\n  "));

    let create_table = match partition_clause {
        Some(clause) => format!("{create_table} {clause}"),
        None => create_table,
    };
    statements.push(format!("{create_table};"));

    if let Some(engine) = options.mysql_engine.as_deref().map(str::trim).filter(|engine| !engine.is_empty()) {
        if let Some(statement) = statements.last_mut() {
            append_mysql_table_option(statement, &format!("ENGINE = {engine}"));
        }
    }

    if capabilities.comment {
        let table_comment = clean(options.table_comment.as_deref().unwrap_or(""));
        if !table_comment.is_empty() && true {
            if matches!(dialect, StructureDialect::Mysql) {
                if let Some(last) = statements.last_mut() {
                    append_mysql_table_option(last, &format!("COMMENT = {}", quote_string(&table_comment)));
                }
            } else {
            }
        }
    }

    {}
    {}
    {}

    for index in options.indexes.iter().filter(|index| !index.marked_for_drop && !index.is_primary) {
        if !capabilities.create_index {
            warnings.push(format!(
                "Creating indexes is not supported for {} from this editor.",
                database_label(options.database_type)
            ));
            continue;
        }
        statements.extend(build_create_index_statements(
            options.database_type,
            dialect,
            &table,
            index,
            &mut warnings,
            options.schema.as_deref(),
            &options.table_name,
            false,
            capabilities.index_concurrent,
            true,
            options.driver_profile.as_deref(),
        ));
    }

    statements.extend(build_foreign_key_sql_for_new_table(&options, &mut warnings));
    statements.extend(build_trigger_sql_for_new_table(&options, &mut warnings));

    TableStructureSqlResult { statements, warnings }
}
