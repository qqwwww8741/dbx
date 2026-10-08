use super::dialect::{capabilities_for, database_label, dialect_label, StructureDialect};
use super::types::TableStructureSqlOptions;
use super::util::{clean, qualified_table, quote_string};
use crate::models::connection::DatabaseType;

pub(super) fn build_table_comment_sql(options: &TableStructureSqlOptions, warnings: &mut Vec<String>) -> Vec<String> {
    let capabilities = capabilities_for(options.database_type, options.driver_profile.as_deref());
    let new_comment = options.table_comment.as_deref().unwrap_or("");
    let original_comment = options.original_table_comment.as_deref().unwrap_or("");
    if clean(new_comment) == clean(original_comment) {
        return Vec::new();
    }
    if !capabilities.comment {
        warnings.push(format!(
            "Table comments are not supported for {} from this editor; the comment change was ignored.",
            database_label(options.database_type)
        ));
        return Vec::new();
    }
    let dialect = capabilities.dialect;
    let table = qualified_table(dialect, options.schema.as_deref(), &options.table_name);
    let quoted = quote_string(&clean(new_comment));
    {}
    match dialect {
        StructureDialect::Mysql => {
            vec![format!("ALTER TABLE {table} COMMENT = {quoted};")]
        }

        _ => {
            if !clean(new_comment).is_empty() {
                warnings
                    .push(format!("Table comments are not supported for {} from this editor.", dialect_label(dialect)));
            }
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
}
