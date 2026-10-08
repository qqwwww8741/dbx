use super::column_alter::{
    build_mysql_existing_column_clause, dameng_drops_identity, has_column_extra_change,
    has_existing_column_attribute_change, validate_dameng_existing_identity_change,
};
use super::column_format::{
    column_definition, has_dameng_identity, is_dameng_identity_compatible_type, is_mysql_character_data_type,
    original_is_mysql_generated_column, original_mysql_generated_clause,
};

use super::dialect::{capabilities_for, database_label, StructureDialect};
use super::indexes::has_existing_index_change;
use super::types::{EditableStructureColumn, TableStructureSqlOptions};
use super::util::{
    clean, normalize_default, original_comment, original_default, qualified_table, quote_ident, quote_string,
};
use crate::models::connection::DatabaseType;
use std::collections::HashSet;

pub(super) fn build_column_sql(options: &TableStructureSqlOptions, warnings: &mut Vec<String>) -> Vec<String> {
    let capabilities = capabilities_for(options.database_type, options.driver_profile.as_deref());
    let dialect = capabilities.dialect;
    let table = qualified_table(dialect, options.schema.as_deref(), &options.table_name);
    let database_label = database_label(options.database_type);
    let active_columns: Vec<_> = options.columns.iter().filter(|column| !column.marked_for_drop).collect();
    {}
    {}
    let has_original_column_positions = active_columns.iter().any(|column| column.original_position.is_some());
    let mut simulated_column_order =
        if has_original_column_positions { original_active_column_order(&active_columns) } else { Vec::new() };
    // Pre-compute the minimal set of existing columns that really need an explicit move.
    // For MySQL/ClickHouse we keep the largest already-ordered subset in place and only
    // emit FIRST/AFTER SQL for columns outside that subset.
    let reordered_existing_column_ids = if has_original_column_positions && matches!(dialect, StructureDialect::Mysql) {
        planned_existing_column_move_ids(&active_columns)
    } else {
        HashSet::new()
    };
    let mysql_primary_key_change = if options.database_type == Some(DatabaseType::Mysql) && !options.is_gaussdb_m_mode {
        primary_key_change(options)
    } else {
        None
    };
    // MySQL validates AUTO_INCREMENT against the statement's final index layout. Keep the
    // dependent column and key clauses together so no implicit commit exposes an invalid middle state.
    let mysql_coalesced_primary_key_change = mysql_primary_key_change
        .as_ref()
        .filter(|change| options.columns.iter().any(|column| mysql_auto_increment_touches_primary_key(column, change)));
    let mut mysql_primary_key_column_clauses = Vec::new();
    let mut statements = Vec::new();
    // DM8 owns identity at table level, so remove the old identity before any per-column ADD,
    // even when the target column appears earlier in the submitted draft.
    {}

    for column in &options.columns {
        if column.marked_for_drop {
            let Some(original) = &column.original else {
                continue;
            };
            if !capabilities.drop_column {
                warnings.push(format!("Dropping columns is not supported for {database_label} from this editor."));
                continue;
            }
            if original.is_primary_key {
                warnings.push(format!("Primary key column \"{}\" cannot be dropped from this editor.", original.name));
                continue;
            }
            {}
            statements.push(build_drop_column_sql(dialect, &table, &original.name));
            continue;
        }

        let active_index = active_columns.iter().position(|active| active.id == column.id).unwrap_or(0);
        let position_clause = if has_original_column_positions && true {
            column_position_clause(dialect, &active_columns, active_index)
        } else {
            String::new()
        };
        let desired_previous_column_id = active_previous_column_id(&active_columns, active_index);
        // A position change only matters when this column is part of the planned move set
        // and its predecessor still differs in the simulated order.
        let has_position_change = has_original_column_positions
            && matches!(dialect, StructureDialect::Mysql)
            && reordered_existing_column_ids.contains(&column.id)
            && column.original.is_some()
            && column.original_position.is_some()
            && simulated_column_position_changed(&simulated_column_order, &column.id, desired_previous_column_id);

        if column.original.is_none() {
            if !capabilities.add_column {
                warnings.push(format!("Adding columns is not supported for {database_label} from this editor."));
                continue;
            }
            {}
            if !capabilities.comment && !clean(&column.comment).is_empty() {
                warnings.push(format!(
                    "Column comments are not supported for {database_label} from this editor; the comment for \"{}\" was ignored.",
                    column.name
                ));
            }
            if mysql_coalesced_primary_key_change
                .is_some_and(|change| mysql_auto_increment_touches_primary_key(column, change))
            {
                mysql_primary_key_column_clauses.push(format!(
                    "ADD COLUMN {}{position_clause}",
                    column_definition(StructureDialect::Mysql, column)
                ));
            } else {
                statements.extend(build_add_column_sql(
                    dialect,
                    options.database_type,
                    capabilities.comment,
                    &table,
                    column,
                    &position_clause,
                    options.schema.as_deref(),
                    &options.table_name,
                    options.driver_profile.as_deref(),
                ));
            }
            if has_original_column_positions && matches!(dialect, StructureDialect::Mysql) {
                apply_simulated_column_position(&mut simulated_column_order, &column.id, desired_previous_column_id);
            }
            continue;
        }

        // A column handing its primary key membership over to another column keeps its
        // AUTO_INCREMENT flag in the submitted draft, which MySQL rejects (ERROR 1075) once
        // the column is no longer keyed. Rewrite it inside the same coalesced ALTER even
        // though the draft reports no change on the column itself.
        let clears_orphaned_auto_increment = mysql_coalesced_primary_key_change
            .is_some_and(|change| mysql_orphans_auto_increment_column(options, column, change));

        if !has_existing_column_attribute_change(column)
            && !has_column_extra_change(column)
            && !has_position_change
            && !clears_orphaned_auto_increment
        {
            continue;
        }
        let original = column.original.as_ref().unwrap();
        let has_rename = column.name != original.name;
        let has_comment_change = clean(&column.comment) != original_comment(column);
        let has_attribute_change = column.data_type.trim() != original.data_type.trim()
            || column.is_nullable != original.is_nullable
            || normalize_default(Some(&column.default_value)) != original_default(column)
            || (has_comment_change && capabilities.comment)
            || (is_mysql_character_data_type(&column.data_type)
                && (column.character_set.trim() != original.character_set.as_deref().unwrap_or("")
                    || column.collation.trim() != original.collation.as_deref().unwrap_or("")))
            || has_column_extra_change(column);
        if has_comment_change && !capabilities.comment {
            warnings.push(format!(
                "Column comments are not supported for {database_label} from this editor; the comment change for \"{}\" was ignored.",
                original.name
            ));
        }
        if has_position_change && !capabilities.reorder_column {
            warnings.push(format!("Reordering columns is not supported for {database_label} from this editor."));
        }
        if has_rename && !capabilities.rename_column {
            warnings.push(format!("Renaming columns is not supported for {database_label} from this editor."));
        }
        if has_attribute_change && !capabilities.alter_existing_column && true {
            warnings.push(format!("Editing existing columns is not supported for {database_label} yet."));
        }
        if (has_position_change && !capabilities.reorder_column)
            || (has_rename && !capabilities.rename_column)
            || (has_attribute_change && !capabilities.alter_existing_column && true)
        {
            continue;
        }
        if dialect == StructureDialect::Mysql
            && original_is_mysql_generated_column(column)
            && original_mysql_generated_clause(column).is_none()
        {
            warnings.push(format!(
                "Column \"{}\" is generated, but its generation expression could not be loaded; no ALTER statement was generated to avoid removing the generated-column definition.",
                original.name
            ));
            continue;
        }
        if !has_rename && !has_attribute_change && !has_position_change && !clears_orphaned_auto_increment {
            continue;
        }

        match dialect {
            StructureDialect::Mysql => {
                let rewritten;
                let effective_column = if clears_orphaned_auto_increment {
                    rewritten = without_auto_increment(column);
                    &rewritten
                } else {
                    column
                };
                let position = if has_position_change { &position_clause } else { "" };
                let clause = { build_mysql_existing_column_clause(effective_column, position) };
                if mysql_coalesced_primary_key_change
                    .is_some_and(|change| mysql_auto_increment_touches_primary_key(column, change))
                {
                    mysql_primary_key_column_clauses.push(clause);
                } else {
                    statements.push(format!("ALTER TABLE {table} {clause};"));
                }
            }

            _ => warnings.push(format!("Editing existing columns is not supported for {database_label} yet.")),
        }
        if has_position_change {
            apply_simulated_column_position(&mut simulated_column_order, &column.id, desired_previous_column_id);
        }
    }

    if let Some(change) = mysql_coalesced_primary_key_change {
        let mut clauses = Vec::new();
        if !change.old_ids.is_empty() {
            clauses.push("DROP PRIMARY KEY".to_string());
        }
        clauses.append(&mut mysql_primary_key_column_clauses);
        if !change.new_names.is_empty() {
            let pk_list = change
                .new_names
                .iter()
                .map(|name| quote_ident(StructureDialect::Mysql, name))
                .collect::<Vec<_>>()
                .join(", ");
            clauses.push(format!("ADD PRIMARY KEY ({pk_list})"));
        }
        statements.push(format!("ALTER TABLE {table} {};", clauses.join(", ")));
    } else {
        // Keep the existing key while column DDL validates. This avoids leaving a table
        // without a key when an incoming key column cannot be made valid.
        statements.extend(build_primary_key_sql(options, dialect, &table, warnings));
    }

    statements
}

fn was_primary_key_column(column: &EditableStructureColumn) -> bool {
    column.original.as_ref().is_some_and(|original| original.is_primary_key)
}

/// Columns that should appear in `ADD PRIMARY KEY (...)` (must remain on the table).
fn appears_in_add_primary_key(column: &EditableStructureColumn) -> bool {
    column.is_primary_key && !column.marked_for_drop
}

struct PrimaryKeyChange<'a> {
    old_ids: HashSet<&'a str>,
    new_ids: HashSet<&'a str>,
    new_names: Vec<&'a str>,
}

fn primary_key_change(options: &TableStructureSqlOptions) -> Option<PrimaryKeyChange<'_>> {
    if options.columns.iter().any(|column| column.marked_for_drop && was_primary_key_column(column)) {
        return None;
    }

    let old_ids: HashSet<&str> = options
        .columns
        .iter()
        .filter(|column| was_primary_key_column(column))
        .map(|column| column.id.as_str())
        .collect();
    let new_ids: HashSet<&str> = options
        .columns
        .iter()
        .filter(|column| appears_in_add_primary_key(column))
        .map(|column| column.id.as_str())
        .collect();
    if old_ids == new_ids {
        return None;
    }

    let new_names = options
        .columns
        .iter()
        .filter(|column| appears_in_add_primary_key(column))
        .map(|column| column.name.as_str())
        .collect();
    Some(PrimaryKeyChange { old_ids, new_ids, new_names })
}

fn unsupported_primary_key_change_warning(options: &TableStructureSqlOptions, change: &PrimaryKeyChange<'_>) -> String {
    let action = if change.old_ids.is_empty() { "Adding" } else { "Changing" };
    format!("{action} primary keys is not supported for {} from this editor.", database_label(options.database_type))
}

pub(super) fn validate_primary_key_change_scope(options: &TableStructureSqlOptions) -> Vec<String> {
    if let Some(original) = options
        .columns
        .iter()
        .find(|column| column.marked_for_drop && was_primary_key_column(column))
        .and_then(|column| column.original.as_ref())
    {
        return vec![format!("Primary key column \"{}\" cannot be dropped from this editor.", original.name)];
    }

    let Some(change) = primary_key_change(options) else { return Vec::new() };
    let capabilities = capabilities_for(options.database_type, options.driver_profile.as_deref());
    let supported =
        if change.old_ids.is_empty() { capabilities.add_primary_key } else { capabilities.alter_primary_key };
    if supported {
        Vec::new()
    } else {
        vec![unsupported_primary_key_change_warning(options, &change)]
    }
}

fn mysql_auto_increment_touches_primary_key(column: &EditableStructureColumn, change: &PrimaryKeyChange<'_>) -> bool {
    column.extra.as_ref().is_some_and(|extra| extra.auto_increment.unwrap_or(false))
        && (change.old_ids.contains(column.id.as_str()) || change.new_ids.contains(column.id.as_str()))
}

/// MySQL allows AUTO_INCREMENT only on a column that leads some key, so a column handing the
/// primary key over to another column must lose the flag in the same statement — unless an
/// already existing index that survives this change still keys it.
fn mysql_orphans_auto_increment_column(
    options: &TableStructureSqlOptions,
    column: &EditableStructureColumn,
    change: &PrimaryKeyChange<'_>,
) -> bool {
    column.original.is_some()
        && column.extra.as_ref().is_some_and(|extra| extra.auto_increment.unwrap_or(false))
        && change.old_ids.contains(column.id.as_str())
        && !change.new_ids.contains(column.id.as_str())
        && !mysql_column_leads_kept_index(options, column)
}

/// Whether a persisted, non-primary index keys this column for the whole change.
///
/// Only an index the draft leaves completely untouched counts. One created by this draft is
/// added after the column DDL, and an edited one is rebuilt as DROP + CREATE after it, so
/// neither covers the column while the coalesced ALTER runs — and the DROP would hit the very
/// same ERROR 1075. Matching is against the persisted names on both sides, so a draft that
/// swaps two column names cannot credit one column's index to the other.
///
/// This applies the InnoDB rule that the auto column must *lead* an index. MyISAM also accepts
/// it in a later position of a multi-column index; DBX has no engine information here, so such
/// a table loses AUTO_INCREMENT rather than every InnoDB table keeping an invalid one.
fn mysql_column_leads_kept_index(options: &TableStructureSqlOptions, column: &EditableStructureColumn) -> bool {
    let Some(original_name) = column.original.as_ref().map(|original| original.name.as_str()) else {
        return false;
    };
    options.indexes.iter().any(|index| {
        !index.marked_for_drop
            && !has_existing_index_change(index)
            && index.original.as_ref().is_some_and(|original| {
                !original.is_primary
                    && original.columns.first().is_some_and(|first| clean(first).eq_ignore_ascii_case(original_name))
            })
    })
}

fn without_auto_increment(column: &EditableStructureColumn) -> EditableStructureColumn {
    let mut cleared = column.clone();
    if let Some(extra) = cleared.extra.as_mut() {
        extra.auto_increment = Some(false);
    }
    cleared
}

pub(super) fn build_primary_key_sql(
    options: &TableStructureSqlOptions,
    dialect: StructureDialect,
    table: &str,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let capabilities = capabilities_for(options.database_type, options.driver_profile.as_deref());

    // Membership by draft id (set equality): pure rename / local reorder of the same key
    // columns is not a PK change. A draft that drops a PK column is also rejected here.
    let Some(change) = primary_key_change(options) else { return Vec::new() };

    let supported =
        if change.old_ids.is_empty() { capabilities.add_primary_key } else { capabilities.alter_primary_key };
    if !supported {
        warnings.push(unsupported_primary_key_change_warning(options, &change));
        return Vec::new();
    }

    // PostgreSQL and SQL Server replace the persisted primary key by constraint name (neither
    // engine has a dependable default naming rule), so the name must come from index metadata.
    let persisted_primary_key_name = match options.database_type {
        _ => None,
    };

    let mut statements = Vec::new();
    if !change.old_ids.is_empty() {
        let Some(drop_sql) = drop_primary_key_statement(dialect, table, options, persisted_primary_key_name) else {
            warnings.push(format!(
                "Changing primary keys is not supported for {} from this editor.",
                database_label(options.database_type)
            ));
            return Vec::new();
        };
        statements.push(drop_sql);
    }

    if !change.new_names.is_empty() {
        let pk_list = change.new_names.iter().map(|name| quote_ident(dialect, name)).collect::<Vec<_>>().join(", ");
        // DM8: ADD [CONSTRAINT name] PRIMARY KEY; anonymous form matches Navicat/DBeaver/MySQL editors.
        let constraint = persisted_primary_key_name
            .map(|name| format!("CONSTRAINT {} ", quote_ident(dialect, name)))
            .unwrap_or_default();
        statements.push(format!("ALTER TABLE {table} ADD {constraint}PRIMARY KEY ({pk_list});"));
    }

    statements
}

/// Dialect-specific DROP for an existing primary key.
///
/// - MySQL: `DROP PRIMARY KEY`
/// - Dameng (DM8): official `DROP PRIMARY KEY [RESTRICT|CASCADE]`; default RESTRICT
///   (no CASCADE — dependent FKs should not be silently removed).
///   System names (`CONS…`) are not stable; name-based DROP is avoided.
///   Cluster primary keys cannot use this path (DM8 restriction) — left to the server.
/// - Postgres: `DROP CONSTRAINT <persisted primary index name>` for PostgreSQL;
///   other Postgres-compatible engines retain the existing default-name behavior.
/// - SQL Server: `DROP CONSTRAINT <persisted primary index name>`; the caller resolves the name
///   from the index metadata (server-generated names like `PK__orders__3213E83F` have no rule).
fn drop_primary_key_statement(
    dialect: StructureDialect,
    table: &str,
    options: &TableStructureSqlOptions,
    persisted_primary_key_name: Option<&str>,
) -> Option<String> {
    match dialect {
        // 神通 Oscar 实测支持 `ALTER TABLE ... DROP PRIMARY KEY`（与 Dameng/MySQL 一致）。
        StructureDialect::Mysql => Some(format!("ALTER TABLE {table} DROP PRIMARY KEY;")),
        _ => None,
    }
}

pub(super) fn build_add_column_sql(
    dialect: StructureDialect,
    database_type: Option<crate::models::connection::DatabaseType>,
    supports_comments: bool,
    table: &str,
    column: &EditableStructureColumn,
    position_clause: &str,
    schema: Option<&str>,
    table_name: &str,
    driver_profile: Option<&str>,
) -> Vec<String> {
    let definition = column_definition(dialect, column);
    let mut statements = {
        let add_keyword = { "ADD COLUMN" };
        vec![format!("ALTER TABLE {table} {add_keyword} {definition}{position_clause};")]
    };
    {}
    {}
    {}
    statements
}

pub(super) fn build_drop_column_sql(dialect: StructureDialect, table: &str, column_name: &str) -> String {
    {}
    format!("ALTER TABLE {table} DROP COLUMN {};", quote_ident(dialect, column_name))
}

pub(super) fn column_position_clause(
    dialect: StructureDialect,
    columns: &[&EditableStructureColumn],
    index: usize,
) -> String {
    if !matches!(dialect, StructureDialect::Mysql) {
        return String::new();
    }
    if index == 0 {
        return " FIRST".to_string();
    }
    format!(" AFTER {}", quote_ident(dialect, columns.get(index - 1).map(|column| column.name.as_str()).unwrap_or("")))
}

pub(super) fn original_active_column_order(columns: &[&EditableStructureColumn]) -> Vec<String> {
    let mut original_columns: Vec<_> = columns
        .iter()
        .filter(|column| column.original.is_some() && column.original_position.is_some())
        .copied()
        .collect();
    original_columns.sort_by_key(|column| column.original_position.unwrap_or(0));
    original_columns.into_iter().map(|column| column.id.clone()).collect()
}

/// Returns the ids of existing columns that must be explicitly moved to reach the target order.
///
/// The function keeps the longest subsequence of existing columns whose relative order is already
/// correct, and marks only the remaining columns for FIRST/AFTER reordering SQL.
pub(super) fn planned_existing_column_move_ids(columns: &[&EditableStructureColumn]) -> HashSet<String> {
    // Only existing columns with an original position participate in move planning.
    // Newly added columns are positioned directly from the target order.
    let reorderable_columns: Vec<_> = columns
        .iter()
        .filter_map(|column| {
            column
                .original
                .as_ref()
                .zip(column.original_position)
                .map(|_| (column.id.as_str(), column.original_position.unwrap_or(0)))
        })
        .collect();
    if reorderable_columns.len() < 2 {
        return HashSet::new();
    }

    // Map the target order back to original positions, then keep the largest increasing subsequence.
    let original_positions: Vec<_> = reorderable_columns.iter().map(|(_, position)| *position).collect();
    // Columns inside the LIS can stay where they are; everything else needs an explicit move.
    let kept_indices: HashSet<_> = longest_increasing_subsequence_indices(&original_positions).into_iter().collect();

    reorderable_columns
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !kept_indices.contains(index))
        .map(|(_, (column_id, _))| column_id.to_string())
        .collect()
}

/// Returns the indices of one longest increasing subsequence within `values`.
///
/// In the reorder planner, an increasing subsequence represents existing columns whose relative
/// order still matches the original table layout, so they can remain untouched.
fn longest_increasing_subsequence_indices(values: &[usize]) -> Vec<usize> {
    if values.is_empty() {
        return Vec::new();
    }

    // O(n^2) is sufficient here because table editors deal with relatively small column counts
    // and the simpler implementation is easier to maintain.
    let mut lengths = vec![1; values.len()];
    let mut previous = vec![None; values.len()];
    let mut best_end_index = 0;

    for current_index in 0..values.len() {
        for previous_index in 0..current_index {
            if values[previous_index] < values[current_index] && lengths[previous_index] + 1 > lengths[current_index] {
                lengths[current_index] = lengths[previous_index] + 1;
                previous[current_index] = Some(previous_index);
            }
        }

        if lengths[current_index] > lengths[best_end_index] {
            best_end_index = current_index;
        }
    }

    // Reconstruct the subsequence by following the predecessor chain backwards.
    let mut indices = Vec::new();
    let mut cursor = Some(best_end_index);
    while let Some(index) = cursor {
        indices.push(index);
        cursor = previous[index];
    }
    indices.reverse();
    indices
}

pub(super) fn active_previous_column_id<'a>(columns: &[&'a EditableStructureColumn], index: usize) -> Option<&'a str> {
    if index == 0 {
        None
    } else {
        columns.get(index - 1).map(|column| column.id.as_str())
    }
}

pub(super) fn simulated_column_position_changed(
    simulated_column_order: &[String],
    column_id: &str,
    desired_previous_column_id: Option<&str>,
) -> bool {
    let Some(index) = simulated_column_order.iter().position(|id| id == column_id) else {
        return false;
    };
    let current_previous_column_id = if index == 0 { None } else { Some(simulated_column_order[index - 1].as_str()) };
    current_previous_column_id != desired_previous_column_id
}

pub(super) fn apply_simulated_column_position(
    simulated_column_order: &mut Vec<String>,
    column_id: &str,
    desired_previous_column_id: Option<&str>,
) {
    if let Some(index) = simulated_column_order.iter().position(|id| id == column_id) {
        simulated_column_order.remove(index);
    }
    let index = desired_previous_column_id
        .and_then(|previous_id| simulated_column_order.iter().position(|id| id == previous_id).map(|index| index + 1))
        .unwrap_or(0);
    simulated_column_order.insert(index, column_id.to_string());
}
