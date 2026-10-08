use super::column_format::{
    column_data_type, column_definition, has_dameng_identity, is_dameng_identity_compatible_type,
    is_mysql_character_data_type, original_is_mysql_generated_column, original_mysql_generated_clause,
};
use super::columns::{build_add_column_sql, build_drop_column_sql};

use super::dialect::{capabilities_for, database_label, StructureDialect};
use super::types::{EditableStructureColumn, SingleColumnAlterSqlOptions, TableStructureSqlResult};
use super::util::{
    clean, format_default_for_sql, normalize_default, original_comment, original_default, qualified_table, quote_ident,
    quote_string,
};
use crate::table_structure_sql::ColumnExtra;

const SINGLE_COLUMN_ADD_PREVIEW_ID_PREFIX: &str = "ddl-preview:";

pub fn build_single_column_alter_sql(options: SingleColumnAlterSqlOptions) -> TableStructureSqlResult {
    let capabilities = capabilities_for(options.database_type, options.driver_profile.as_deref());
    let dialect = capabilities.dialect;
    let table = qualified_table(dialect, options.schema.as_deref(), &options.table_name);
    let database_label = database_label(options.database_type);
    let mut warnings = Vec::new();
    let mut statements = Vec::new();

    if options.column.marked_for_drop {
        let Some(original) = &options.column.original else {
            warnings.push("No original column info available.".to_string());
            return TableStructureSqlResult { statements, warnings };
        };
        if !capabilities.drop_column {
            warnings.push(format!("Dropping columns is not supported for {database_label} from this editor."));
            return TableStructureSqlResult { statements, warnings };
        }
        if original.is_primary_key {
            warnings.push(format!("Primary key column \"{}\" cannot be dropped from this editor.", original.name));
            return TableStructureSqlResult { statements, warnings };
        }
        {}
        statements.push(build_drop_column_sql(dialect, &table, &original.name));
        return TableStructureSqlResult { statements, warnings };
    }

    // The sidebar marks field-DDL previews explicitly in the draft id. Keep
    // `original` in that request so generated-column expressions can be copied
    // into the ADD definition without changing the meaning of position data
    // for existing API callers.
    if options.column.id.starts_with(SINGLE_COLUMN_ADD_PREVIEW_ID_PREFIX) {
        if !capabilities.add_column {
            warnings.push(format!("Adding columns is not supported for {database_label} from this editor."));
            return TableStructureSqlResult { statements, warnings };
        }
        if options.column.name.trim().is_empty() {
            warnings.push("Column name cannot be empty.".to_string());
            return TableStructureSqlResult { statements, warnings };
        }
        if options.column.data_type.trim().is_empty() {
            warnings.push("Column type cannot be empty.".to_string());
            return TableStructureSqlResult { statements, warnings };
        }
        if !capabilities.comment && !clean(&options.column.comment).is_empty() {
            warnings.push(format!(
                "Column comments are not supported for {database_label} from this editor; the comment for \"{}\" was ignored.",
                options.column.name
            ));
        }
        statements.extend(build_add_column_sql(
            dialect,
            options.database_type,
            capabilities.comment,
            &table,
            &options.column,
            "",
            options.schema.as_deref(),
            &options.table_name,
            options.driver_profile.as_deref(),
        ));
        return TableStructureSqlResult { statements, warnings };
    }

    let Some(original) = &options.column.original else {
        warnings.push(
            "This column has no original state — ALTER statements are only available for existing columns.".to_string(),
        );
        return TableStructureSqlResult { statements, warnings };
    };

    if !has_existing_column_attribute_change(&options.column) && !has_column_extra_change(&options.column) {
        warnings.push("No changes detected for this column.".to_string());
        return TableStructureSqlResult { statements, warnings };
    }

    let has_rename = options.column.name != original.name;
    let has_comment_change = clean(&options.column.comment) != original_comment(&options.column);
    let has_attribute_change = options.column.data_type.trim() != original.data_type.trim()
        || options.column.is_nullable != original.is_nullable
        || normalize_default(Some(&options.column.default_value)) != original_default(&options.column)
        || (has_comment_change && capabilities.comment)
        || (is_mysql_character_data_type(&options.column.data_type)
            && (options.column.character_set.trim() != original.character_set.as_deref().unwrap_or("")
                || options.column.collation.trim() != original.collation.as_deref().unwrap_or("")));

    if has_comment_change && !capabilities.comment {
        warnings.push(format!(
            "Column comments are not supported for {database_label} from this editor; the comment change for \"{}\" was ignored.",
            original.name
        ));
    }

    if has_rename && !capabilities.rename_column {
        warnings.push(format!("Renaming columns is not supported for {database_label} from this editor."));
    }
    if has_attribute_change && !capabilities.alter_existing_column && true {
        warnings.push(format!("Editing existing columns is not supported for {database_label} yet."));
    }

    if (has_rename && !capabilities.rename_column)
        || (has_attribute_change && !capabilities.alter_existing_column && true)
    {
        return TableStructureSqlResult { statements, warnings };
    }
    if dialect == StructureDialect::Mysql
        && original_is_mysql_generated_column(&options.column)
        && original_mysql_generated_clause(&options.column).is_none()
    {
        warnings.push(format!(
            "Column \"{}\" is generated, but its generation expression could not be loaded; no ALTER statement was generated to avoid removing the generated-column definition.",
            original.name
        ));
        return TableStructureSqlResult { statements, warnings };
    }
    if !has_rename && !has_attribute_change && !has_column_extra_change(&options.column) {
        return TableStructureSqlResult { statements, warnings };
    }

    match dialect {
        StructureDialect::Mysql => statements.extend(build_mysql_existing_column_sql(&table, &options.column, "")),

        _ => warnings.push(format!("Editing existing columns is not supported for {database_label} yet.")),
    }

    TableStructureSqlResult { statements, warnings }
}

fn original_has_auto_increment(extra: &str) -> bool {
    let lower = extra.to_lowercase();
    lower.contains("auto_increment") || lower.contains("autoincrement")
}

pub(super) fn dameng_drops_identity(column: &EditableStructureColumn) -> bool {
    false
}

pub(super) fn validate_dameng_existing_identity_change(
    column: &EditableStructureColumn,
    warnings: &mut Vec<String>,
) -> bool {
    false
}

pub(super) fn has_column_extra_change(column: &EditableStructureColumn) -> bool {
    let Some(original) = &column.original else { return false };
    let old = original.extra.as_deref().unwrap_or("").to_lowercase();
    let ai = column.extra.as_ref().and_then(|e| e.auto_increment).unwrap_or(false);
    let on_update = column.extra.as_ref().and_then(|e| e.on_update_current_timestamp).unwrap_or(false);
    ai != original_has_auto_increment(&old) || on_update != old.contains("on update")
}

pub(super) fn build_mysql_existing_column_sql(
    table: &str,
    column: &EditableStructureColumn,
    position_clause: &str,
) -> Vec<String> {
    let operation = build_mysql_existing_column_clause(column, position_clause);
    vec![format!("ALTER TABLE {table} {operation};")]
}

pub(super) fn build_mysql_existing_column_clause(column: &EditableStructureColumn, position_clause: &str) -> String {
    let original_name = column.original.as_ref().map(|original| original.name.as_str()).unwrap_or(&column.name);
    let operation = if column.name == original_name {
        format!("MODIFY COLUMN {}", column_definition(StructureDialect::Mysql, column))
    } else {
        format!(
            "CHANGE COLUMN {} {}",
            quote_ident(StructureDialect::Mysql, original_name),
            column_definition(StructureDialect::Mysql, column)
        )
    };
    format!("{operation}{position_clause}")
}

pub(super) fn has_existing_column_attribute_change(column: &EditableStructureColumn) -> bool {
    let Some(original) = &column.original else {
        return false;
    };
    column.name != original.name
        || column.data_type.trim() != original.data_type.trim()
        || column.is_nullable != original.is_nullable
        || normalize_default(Some(&column.default_value)) != original_default(column)
        || clean(&column.comment) != original_comment(column)
        || (is_mysql_character_data_type(&column.data_type)
            && (column.character_set.trim() != original.character_set.as_deref().unwrap_or("")
                || column.collation.trim() != original.collation.as_deref().unwrap_or("")))
}
