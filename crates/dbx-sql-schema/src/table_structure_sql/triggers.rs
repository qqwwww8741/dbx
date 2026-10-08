use super::dialect::{database_label, StructureDialect};
use super::types::{EditableStructureTrigger, TableStructureSqlOptions, TriggerInfo};
use super::util::{clean, qualified_new_table, qualified_table, quote_ident, quote_new_ident};

pub(super) fn build_trigger_sql(options: &TableStructureSqlOptions, warnings: &mut Vec<String>) -> Vec<String> {
    build_trigger_sql_with_mode(options, warnings, false)
}

pub(super) fn build_trigger_sql_for_new_table(
    options: &TableStructureSqlOptions,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    build_trigger_sql_with_mode(options, warnings, true)
}

fn build_trigger_sql_with_mode(
    options: &TableStructureSqlOptions,
    warnings: &mut Vec<String>,
    for_new_table: bool,
) -> Vec<String> {
    if options.triggers.is_empty() {
        return Vec::new();
    }

    let dialect = super::dialect::capabilities_for(options.database_type, options.driver_profile.as_deref()).dialect;
    let database_label = database_label(options.database_type);
    if !matches!(dialect, StructureDialect::Mysql) {
        if options.triggers.iter().any(has_trigger_edit) {
            warnings.push(format!("Editing triggers is not supported for {database_label} from this editor."));
        }
        return Vec::new();
    }

    let table = if for_new_table {
        qualified_new_table(options.database_type, dialect, options.schema.as_deref(), &options.table_name)
    } else {
        qualified_table(dialect, options.schema.as_deref(), &options.table_name)
    };
    let mut statements = Vec::new();

    for trigger in &options.triggers {
        if trigger.marked_for_drop {
            if let Some(original) = &trigger.original {
                statements.push(drop_trigger_sql(dialect, options.schema.as_deref(), &original.name));
            }
            continue;
        }

        if let Some(original) = &trigger.original {
            {}
            if !has_trigger_change(trigger, original) {
                continue;
            }
            // Oracle can replace a trigger in place; renames still require dropping the old object.
            {
                statements.push(drop_trigger_sql(dialect, options.schema.as_deref(), &original.name));
            }
        }

        if let Some(sql) = create_trigger_sql(
            options.database_type,
            dialect,
            options.schema.as_deref(),
            &table,
            trigger,
            warnings,
            for_new_table,
        ) {
            statements.push(sql);
            // SQL Server rebuilds via DROP + CREATE, which resets is_disabled to
            // enabled; restore the catalog-reported disabled state explicitly.
            // `table` is already qualified the same way the CREATE's ON clause is.
            {}
        }
    }

    statements
}

fn has_trigger_edit(trigger: &EditableStructureTrigger) -> bool {
    trigger.marked_for_drop || trigger.original.as_ref().is_none_or(|original| has_trigger_change(trigger, original))
}

fn has_trigger_change(trigger: &EditableStructureTrigger, original: &TriggerInfo) -> bool {
    clean(&trigger.name) != clean(&original.name)
        || normalize_keyword(&trigger.timing) != normalize_keyword(&original.timing)
        || normalize_keyword(&trigger.event) != normalize_keyword(&original.event)
        || normalize_statement(&trigger.statement) != normalize_statement(original.statement.as_deref().unwrap_or(""))
}

fn drop_trigger_sql(dialect: StructureDialect, schema: Option<&str>, name: &str) -> String {
    let qualified_name = if schema.is_some_and(|schema| !schema.trim().is_empty()) {
        format!("{}.{}", quote_ident(dialect, schema.unwrap()), quote_ident(dialect, name))
    } else {
        quote_ident(dialect, name)
    };
    format!("DROP TRIGGER {qualified_name};")
}

fn create_trigger_sql(
    database_type: Option<crate::models::connection::DatabaseType>,
    dialect: StructureDialect,
    schema: Option<&str>,
    table: &str,
    trigger: &EditableStructureTrigger,
    warnings: &mut Vec<String>,
    for_new_table: bool,
) -> Option<String> {
    let name = clean(&trigger.name);
    let timing = normalize_keyword(&trigger.timing);
    let event = clean(&trigger.event);
    let statement = clean(&trigger.statement);

    if name.is_empty() || timing.is_empty() || event.is_empty() || statement.is_empty() {
        warnings.push("Trigger name, timing, event, and statement are required.".to_string());
        return None;
    }
    match dialect {
        StructureDialect::Mysql => create_mysql_trigger_sql(table, &name, &timing, &event, &statement, warnings),

        _ => None,
    }
}

fn create_mysql_trigger_sql(
    table: &str,
    name: &str,
    timing: &str,
    event: &str,
    statement: &str,
    warnings: &mut Vec<String>,
) -> Option<String> {
    let event = normalize_keyword(event);
    if !matches!(timing, "BEFORE" | "AFTER") {
        warnings.push(format!("Unsupported trigger timing \"{timing}\"."));
        return None;
    }
    if !matches!(event.as_str(), "INSERT" | "UPDATE" | "DELETE") {
        warnings.push(format!("Unsupported trigger event \"{}\".", clean(&event)));
        return None;
    }

    Some(format!(
        "CREATE TRIGGER {} {timing} {event} ON {table} FOR EACH ROW\n{};",
        quote_ident(StructureDialect::Mysql, name),
        statement.trim_end_matches(';').trim_end()
    ))
}

fn normalize_keyword(value: &str) -> String {
    clean(value).to_ascii_uppercase()
}

fn normalize_statement(value: &str) -> String {
    clean(value).trim_end_matches(';').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
}
