//! Read-only planning for the structure-only transfer SQL preview.
//!
//! A structure-only transfer used to start with no idea what it would run. This module
//! renders the statements the create pass is about to execute so the confirmation dialog can
//! show them first. It is strictly planning: target names, source metadata and the SQL are
//! read from the same helpers the execution pass uses (`ddl_plan::prepare_table_ddl`,
//! `plan_postgres_owned_sequences_for_transfer`, `generate_comment_ddl_with_column_quoting`,
//! `generate_postgres_index_ddl`, `generate_postgres_foreign_key_ddl`,
//! `generate_mysql_foreign_key_alter_statements`), and no statement is ever executed here.
//!
//! The result is a plan, not a frozen script: `start_transfer` re-reads source and target
//! metadata before it executes, and the existing execution-time fail-closed checks stay in
//! force. Callers must therefore never present the preview as "these exact statements will
//! run".
//!
//! Where the create pass normalizes before executing (splitting a reused PostgreSQL table
//! script, stripping inline foreign keys, dropping the post-table index/foreign-key statements
//! it re-applies from metadata), the preview runs the same normalization — a statement the
//! executor filters out must not be shown. Re-applied statements are shown once per step,
//! including the idempotent `COMMENT` the reused script may already contain.

use super::*;

/// Emitted when the request asks for no structure DDL at all (`createTable = false`).
const CREATE_TABLE_DISABLED_NOTE: &str =
    "-- This transfer has structure DDL disabled (createTable = false): no structure statements will run.";

/// Emitted when the transfer also moves non-table schema objects whose DDL this preview
/// deliberately does not expand (views, routines, triggers, standalone sequences, ...).
const UNEXPANDED_OBJECTS_NOTE: &str = "\
-- Additional schema objects selected by this request (or the legacy PostgreSQL default) are transferred.
-- Their generated DDL is not expanded in this preview.";

/// Plan the structure DDL a structure-only transfer is about to run.
///
/// Everything inside is read-only: metadata reads plus SQL rendering. The only statements
/// handed to `execute_on_pool` are catalog queries (`SELECT`), never DDL.
pub(super) async fn build_structure_preview(
    state: &Arc<AppState>,
    request: &TransferRequest,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    source_pool_key: &str,
    target_pool_key: &str,
) -> Result<TransferStructurePreview, String> {
    if !request.create_table {
        return Ok(TransferStructurePreview {
            sql: CREATE_TABLE_DISABLED_NOTE.to_string(),
            tables: Vec::new(),
            operations: Vec::new(),
        });
    }

    let pg_compat_transfer = false;
    let index_if_not_exists = false;
    // Two execution paths create the target schema before any table does: the inline ensure in
    // `create_transfer_target_table` (PostgreSQL-dialect target from another family) and
    // `transfer_postgres_schema_dependencies` (PostgreSQL→PostgreSQL). Plan the same statement
    // for either, deciding it with a read-only existence check instead of executing anything.
    let create_schema_sql = { None };

    // Same order and same foreign key metadata the create pass uses.
    let (tables, known_foreign_keys) = sorted_source_tables_with_foreign_keys(state, request).await;

    let mut sections: Vec<String> = Vec::new();
    let mut operations = Vec::new();
    let mut deferred_fk_operations = Vec::new();
    if let Some(create_schema_sql) = &create_schema_sql {
        operations.push(TransferStructureOperation::schema(&request.target_schema));
        sections.push(format!(
            "-- Ensure the target schema exists\n{}",
            statement_block(std::slice::from_ref(create_schema_sql))
        ));
    }

    let mut planned_tables: Vec<TransferStructurePreviewTable> = Vec::with_capacity(tables.len());
    let mut table_sections: Vec<String> = Vec::new();
    let mut deferred_fk_alters: Vec<String> = Vec::new();

    for table in &tables {
        let ResolvedTransferTargetTable { name: target_table, preexisting } = resolve_transfer_target_table_name(
            state,
            request,
            table,
            target_pool_key,
            target_db_type,
            request.source_catalog.as_deref(),
            request.target_catalog.as_deref(),
        )
        .await;
        // A rebuild renames every preexisting target aside before the create pass runs, so
        // those tables are created fresh — the same reset the create pass performs.
        if preexisting && !request.drop_target_before_create {
            operations.push(TransferStructureOperation::table(
                TransferStructureOperationKind::SkipExistingTable,
                table,
                &target_table,
            ));
            let note = format!(
                "-- {table} -> {target_table}: the target table already exists, so this transfer plans no structure \
                 DDL for it"
            );
            planned_tables.push(TransferStructurePreviewTable {
                source_table: table.clone(),
                target_table,
                preexisting: true,
                sql: note.clone(),
            });
            table_sections.push(note);
            continue;
        }

        let planned = plan_table_structure(
            state,
            request,
            table,
            &target_table,
            preexisting && request.drop_target_before_create,
            source_db_type,
            target_db_type,
            source_pool_key,
            target_pool_key,
            &known_foreign_keys,
            false,
            false,
        )
        .await?;
        deferred_fk_alters.extend(planned.deferred_fk_alters);
        operations.extend(planned.operations);
        deferred_fk_operations.extend(planned.deferred_fk_operations);

        let section = format!("-- {} -> {}\n{}", table, target_table, statement_block(&planned.statements));
        planned_tables.push(TransferStructurePreviewTable {
            source_table: table.clone(),
            target_table,
            preexisting: false,
            sql: section.clone(),
        });
        table_sections.push(section);
    }

    sections.extend(table_sections);
    {}
    if !deferred_fk_alters.is_empty() {
        // MySQL-family targets defer foreign keys so creation order never has to satisfy
        // them; the create pass flushes these after every table exists.
        sections.push(format!(
            "-- Foreign keys added after every table exists, so creation order never has to satisfy them\n{}",
            statement_block(&deferred_fk_alters)
        ));
    }
    operations.extend(deferred_fk_operations);
    sections.extend(unexpanded_schema_object_notes(source_db_type, target_db_type, request));

    Ok(TransferStructurePreview { sql: sections.join("\n\n"), tables: planned_tables, operations })
}

struct PlannedTableStructure {
    statements: Vec<String>,
    operations: Vec<TransferStructureOperation>,
    deferred_fk_alters: Vec<String>,
    deferred_fk_operations: Vec<TransferStructureOperation>,
}

fn foreign_key_operations(names: &[String], source_table: &str, target_table: &str) -> Vec<TransferStructureOperation> {
    names
        .iter()
        .map(|name| {
            TransferStructureOperation::object(
                TransferStructureOperationKind::AddForeignKey,
                name,
                source_table,
                target_table,
            )
        })
        .collect()
}

fn comment_operations(
    columns: &[db::ColumnInfo],
    source_table: &str,
    target_table: &str,
    target_db_type: &DatabaseType,
    table_comment: Option<&str>,
) -> Vec<TransferStructureOperation> {
    let table_comments_supported = supports_comment_on_transfer_ddl(target_db_type);
    let column_comments_supported = table_comments_supported || false;
    let mut operations = Vec::new();

    if table_comments_supported && table_comment.is_some_and(|comment| !comment.trim().is_empty()) {
        operations.push(TransferStructureOperation::table(
            TransferStructureOperationKind::AddComment,
            source_table,
            target_table,
        ));
    }
    if column_comments_supported {
        operations.extend(columns.iter().filter_map(|column| {
            column.comment.as_deref().filter(|comment| !comment.trim().is_empty()).map(|_| {
                TransferStructureOperation::object(
                    TransferStructureOperationKind::AddComment,
                    &column.name,
                    source_table,
                    target_table,
                )
            })
        }));
    }

    operations
}

/// Render the statements the create pass runs for one table, in execution order.
#[allow(clippy::too_many_arguments)]
async fn plan_table_structure(
    state: &Arc<AppState>,
    request: &TransferRequest,
    table: &str,
    target_table: &str,
    rebuild_existing_target: bool,
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    source_pool_key: &str,
    target_pool_key: &str,
    known_foreign_keys: &HashMap<String, Vec<db::ForeignKeyInfo>>,
    pg_compat_transfer: bool,
    index_if_not_exists: bool,
) -> Result<PlannedTableStructure, String> {
    let preserves_target_table_name = target_table == table;
    let columns = source_transfer_columns(state, request, table, source_pool_key).await?;
    let table_comment = source_table_comment(state, request, table, source_db_type).await;

    // Planned here, created by the execution pass.

    // Shared with `create_transfer_target_table`: the same helper renders the DDL that runs.
    let prepared = ddl_plan::prepare_table_ddl(
        state,
        request,
        table,
        target_table,
        source_db_type,
        target_db_type,
        source_pool_key,
        &columns,
        table_comment.as_deref(),
        known_foreign_keys,
    )
    .await?;
    let ddl = { prepared.ddl };

    let mut statements = Vec::new();
    let mut operations = Vec::new();
    {}
    let table_kind = if rebuild_existing_target {
        TransferStructureOperationKind::RebuildTable
    } else {
        TransferStructureOperationKind::CreateTable
    };
    operations.push(TransferStructureOperation::table(table_kind, table, target_table));

    // Normalize the DDL exactly like the executor does before running it: the reused
    // PostgreSQL script spans several statements, has its inline foreign keys stripped, and
    // drops the post-table index/foreign-key statements that are re-applied from structured
    // metadata below. A statement the create pass filters out must never appear here.
    statements.extend(transfer_ddl_statements(&ddl, target_db_type));

    let comment_statements = generate_comment_ddl_with_column_quoting(
        &columns,
        target_table,
        &request.target_schema,
        target_db_type,
        table_comment.as_deref(),
        request.quote_target_column_names,
    );
    operations.extend(comment_operations(&columns, table, target_table, target_db_type, table_comment.as_deref()));
    statements.extend(comment_statements);

    {}

    let mut deferred_fk_operations = Vec::new();
    if supports_deferred_mysql_foreign_keys(target_db_type) {
        deferred_fk_operations = foreign_key_operations(&prepared.deferred_fk_names, table, target_table);
    }

    {}

    Ok(PlannedTableStructure {
        statements,
        operations,
        deferred_fk_alters: prepared.deferred_fk_alters,
        deferred_fk_operations,
    })
}

/// Read and de-duplicate the source columns the create pass uses, failing the same way the
/// pass does when the source returns nothing.
async fn source_transfer_columns(
    state: &AppState,
    request: &TransferRequest,
    table: &str,
    source_pool_key: &str,
) -> Result<Vec<db::ColumnInfo>, String> {
    let raw = get_columns_for_transfer(
        state,
        source_pool_key,
        &request.source_connection_id,
        &request.source_database,
        &request.source_schema,
        table,
        request.source_catalog.as_deref(),
    )
    .await?;
    let mut seen = HashSet::new();
    let columns = raw.into_iter().filter(|column| seen.insert(column.name.clone())).collect::<Vec<_>>();
    if columns.is_empty() {
        return Err(format!("No columns found for table {table}"));
    }
    Ok(columns)
}

/// Source table comment, via the same fallback chain the create pass uses. A metadata read
/// failure only drops the comment, exactly like execution.
async fn source_table_comment(
    state: &Arc<AppState>,
    request: &TransferRequest,
    table: &str,
    source_db_type: &DatabaseType,
) -> Option<String> {
    {
        list_transfer_tables_isolated(
            state.clone(),
            request.source_connection_id.clone(),
            request.source_database.clone(),
            request.source_schema.clone(),
            request.source_catalog.clone(),
            *source_db_type,
            table.to_string(),
            1,
        )
        .await
        .unwrap_or_default()
        .into_iter()
        .next()
        .and_then(|info| info.comment)
    }
}

/// Table creation order plus the foreign key metadata behind the deferred MySQL alters, using
/// the same helper (and the same external-catalog skip) the transfer pass uses.
async fn sorted_source_tables_with_foreign_keys(
    state: &Arc<AppState>,
    request: &TransferRequest,
) -> (Vec<String>, HashMap<String, Vec<db::ForeignKeyInfo>>) {
    if request.tables.len() <= 1 {
        return (request.tables.clone(), HashMap::new());
    }
    let skip_fk_sort = false;
    {}
    sort_tables_by_fk_dependency_with_foreign_keys(
        state,
        &request.source_connection_id,
        &request.source_database,
        &request.source_schema,
        &request.tables,
        true,
    )
    .await
    .unwrap_or_else(|error| {
        log::warn!("[transfer] structure preview could not sort tables by FK dependency: {error}");
        (request.tables.clone(), HashMap::new())
    })
}

/// The statement block the preview never expands: selected non-table schema objects.
fn unexpanded_schema_object_notes(
    source_db_type: &DatabaseType,
    target_db_type: &DatabaseType,
    request: &TransferRequest,
) -> Vec<String> {
    if !should_transfer_schema_objects(source_db_type, target_db_type, request) {
        return Vec::new();
    }
    let selections = match request.object_selection_mode() {
        TransferObjectSelectionMode::LegacyUnspecified => return vec![UNEXPANDED_OBJECTS_NOTE.to_string()],
        TransferObjectSelectionMode::Explicit(selections) => selections,
    };
    let mut notes = vec![UNEXPANDED_OBJECTS_NOTE.to_string()];
    for selection in selections {
        if selection.object_type == TransferObjectKind::Table || selection.names.is_empty() {
            continue;
        }
        notes.push(format!("-- {}: {}", preview_object_kind_label(&selection.object_type), selection.names.join(", ")));
    }
    notes
}

fn preview_object_kind_label(kind: &TransferObjectKind) -> &'static str {
    match kind {
        TransferObjectKind::Table => "Tables",
        TransferObjectKind::View => "Views",
        TransferObjectKind::MaterializedView => "Materialized views",
        TransferObjectKind::Procedure => "Procedures",
        TransferObjectKind::Function => "Functions",
        TransferObjectKind::Trigger => "Triggers",
        TransferObjectKind::Sequence => "Sequences",
        TransferObjectKind::Event => "Events",
    }
}

fn statement_block(statements: &[String]) -> String {
    statements
        .iter()
        .map(|statement| ensure_sql_statement_terminated(statement))
        .filter(|statement| !statement.is_empty() && statement != ";")
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn structure_request(overrides: serde_json::Value) -> TransferRequest {
        let mut value = json!({
            "transferId": uuid::Uuid::new_v4().to_string(),
            "sourceConnectionId": "source", "sourceDatabase": "main", "sourceSchema": "main",
            "targetConnectionId": "target", "targetDatabase": "main", "targetSchema": "main",
            "tables": ["orders", "users"], "createTable": true, "content": "structureOnly",
            "mode": "append", "batchSize": 10
        });
        let map = value.as_object_mut().unwrap();
        for (key, override_value) in overrides.as_object().unwrap() {
            map.insert(key.clone(), override_value.clone());
        }
        serde_json::from_value(value).unwrap()
    }

    async fn target_table_names(state: &AppState, target_pool: &str) -> Vec<String> {
        let result = execute_read_on_pool(
            state,
            target_pool,
            "SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name",
        )
        .await
        .unwrap();
        result.rows.iter().filter_map(|row| json_string_cell(row, 0)).collect()
    }
}
