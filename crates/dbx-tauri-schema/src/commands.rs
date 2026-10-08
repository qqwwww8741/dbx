use std::future::Future;
use std::sync::Arc;
use tauri::State;

use dbx_core::connection::AppState;
use dbx_core::db;

async fn run_cancellable<T, F>(state: &Arc<AppState>, execution_id: Option<String>, future: F) -> Result<T, String>
where
    F: Future<Output = Result<T, String>>,
{
    let registered =
        execution_id.as_ref().filter(|id| !id.trim().is_empty()).map(|id| state.running_queries.register(id.clone()));
    if let Some(query) = registered.as_ref() {
        let token = query.token();
        tokio::select! {
            biased;
            _ = token.cancelled() => Err(dbx_core::query::canceled_error()),
            result = future => result,
        }
    } else {
        future.await
    }
}

#[tauri::command]
pub async fn list_databases(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<Vec<db::DatabaseInfo>, String> {
    dbx_core::schema::list_databases_core(&state, &connection_id).await
}

#[tauri::command]
pub async fn list_database_metadata(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<Vec<db::DatabaseInfo>, String> {
    dbx_core::schema::list_database_metadata_core(&state, &connection_id).await
}

#[tauri::command]
pub async fn list_database_storage(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    databases: Vec<String>,
) -> Result<Vec<db::DatabaseStorageInfo>, String> {
    dbx_core::schema::list_database_storage_core(&state, &connection_id, &databases).await
}

#[tauri::command]
pub async fn list_schemas(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    apply_visible_filter: Option<bool>,
) -> Result<Vec<String>, String> {
    dbx_core::schema::list_schemas_core_with_visible_filter(
        &state,
        &connection_id,
        &database,
        apply_visible_filter.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn list_schema_infos(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
) -> Result<Vec<db::SchemaInfo>, String> {
    dbx_core::schema::list_schema_infos_core(&state, &connection_id, &database).await
}

#[tauri::command]
pub async fn list_data_types(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
) -> Result<Vec<String>, String> {
    dbx_core::schema::list_data_types_core(&state, &connection_id, &database).await
}

#[tauri::command]
pub async fn list_tables(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    filter: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    object_types: Option<Vec<String>>,
    catalog: Option<String>,
    table_name_filter: Option<dbx_core::schema::TableNameFilter>,
) -> Result<Vec<db::TableInfo>, String> {
    {}
    dbx_core::schema::list_tables_core(
        &state,
        &connection_id,
        &database,
        &schema,
        filter.as_deref(),
        limit,
        offset,
        object_types.as_deref(),
        table_name_filter.as_ref(),
    )
    .await
}

#[tauri::command]
pub async fn get_table_comment(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
    catalog: Option<String>,
) -> Result<Option<String>, String> {
    {}
    dbx_core::schema::get_table_comment_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn get_mysql_table_auto_increment(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    table: String,
) -> Result<Option<String>, String> {
    dbx_core::schema::get_mysql_table_auto_increment_core(&state, &connection_id, &database, &table).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn list_objects(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    filter: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    object_types: Option<Vec<String>>,
    catalog: Option<String>,
    table_name_filter: Option<dbx_core::schema::TableNameFilter>,
    execution_id: Option<String>,
) -> Result<Vec<db::ObjectInfo>, String> {
    let app = Arc::clone(state.inner());
    let operation_app = Arc::clone(&app);
    run_cancellable(&app, execution_id, async move {
        {}
        dbx_core::schema::list_objects_core(
            &operation_app,
            &connection_id,
            &database,
            &schema,
            filter.as_deref(),
            limit,
            offset,
            object_types.as_deref(),
            table_name_filter.as_ref(),
        )
        .await
    })
    .await
}

#[tauri::command]
pub async fn list_object_statistics(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
) -> Result<Vec<db::ObjectStatistics>, String> {
    dbx_core::schema::list_object_statistics_core(&state, &connection_id, &database, &schema).await
}

#[tauri::command]
pub async fn list_completion_objects(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
) -> Result<Vec<db::ObjectInfo>, String> {
    dbx_core::schema::list_completion_objects_core(&state, &connection_id, &database, &schema).await
}

#[tauri::command]
pub async fn completion_assistant_search(
    state: State<'_, Arc<AppState>>,
    request: db::CompletionAssistantRequest,
) -> Result<db::CompletionAssistantResponse, String> {
    dbx_core::schema::completion_assistant_search_core(&state, request).await
}

#[tauri::command]
pub async fn get_object_source(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    name: String,
    object_type: db::ObjectSourceKind,
    signature: Option<String>,
    relation_name: Option<String>,
) -> Result<db::ObjectSource, String> {
    dbx_core::schema::get_object_source_core(
        &state,
        &connection_id,
        &database,
        &schema,
        &name,
        object_type,
        signature.as_deref(),
        relation_name.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn get_event_info(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    name: String,
) -> Result<db::MysqlEventInfo, String> {
    dbx_core::schema::get_event_info_core(&state, &connection_id, &database, &schema, &name).await
}

#[tauri::command]
pub async fn get_columns(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
    catalog: Option<String>,
    client_session_id: Option<String>,
) -> Result<Vec<db::ColumnInfo>, String> {
    {}
    dbx_core::schema::get_columns_core_for_session(
        &state,
        &connection_id,
        &database,
        &schema,
        &table,
        client_session_id.as_deref(),
    )
    .await
}

/// Read-only Plugin Host API over the existing connection. The core function
/// performs the open-connection gate before entering the ordinary metadata path.
#[tauri::command]
pub async fn get_plugin_table_metadata(
    state: State<'_, Arc<AppState>>,
    request: dbx_core::schema::plugin_metadata::PluginTableContext,
) -> Result<dbx_core::schema::plugin_metadata::PluginTableMetadata, String> {
    dbx_core::schema::plugin_metadata::get_table_metadata(&state, request).await
}

#[tauri::command]
pub async fn get_all_columns(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
) -> Result<Vec<db::TableColumnsResult>, String> {
    dbx_core::schema::get_all_columns_core(&state, &connection_id, &database, &schema).await
}

#[tauri::command]
pub async fn list_indexes(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
    catalog: Option<String>,
) -> Result<Vec<db::IndexInfo>, String> {
    {}
    dbx_core::schema::list_indexes_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn list_reference_key_columns(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
    catalog: Option<String>,
) -> Result<Vec<String>, String> {
    {}
    dbx_core::schema::list_reference_key_columns_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn list_reference_keys(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
    catalog: Option<String>,
) -> Result<Vec<dbx_core::schema::ReferenceKeyInfo>, String> {
    {}
    dbx_core::schema::list_reference_keys_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn list_foreign_keys(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
    catalog: Option<String>,
) -> Result<Vec<db::ForeignKeyInfo>, String> {
    {}
    dbx_core::schema::list_foreign_keys_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn list_foreign_keys_for_database(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    catalog: Option<String>,
    execution_id: Option<String>,
) -> Result<std::collections::HashMap<String, Vec<db::ForeignKeyInfo>>, String> {
    let app = Arc::clone(state.inner());
    let operation_app = Arc::clone(&app);
    run_cancellable(&app, execution_id, async move {
        {}
        dbx_core::schema::list_foreign_keys_for_database_core(&operation_app, &connection_id, &database, &schema).await
    })
    .await
}

#[tauri::command]
pub async fn list_triggers(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
    catalog: Option<String>,
) -> Result<Vec<db::TriggerInfo>, String> {
    {}
    dbx_core::schema::list_triggers_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn list_constraints(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
    catalog: Option<String>,
) -> Result<Vec<dbx_core::db::ConstraintInfo>, String> {
    let _ = catalog;
    dbx_core::schema::list_constraints_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn list_partitions(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
) -> Result<Vec<dbx_core::db::PartitionInfo>, String> {
    dbx_core::schema::list_partitions_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn get_table_partition_status(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
) -> Result<dbx_core::schema::TablePartitionStatus, String> {
    dbx_core::schema::table_partition_status_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn list_invalid_indexes(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
) -> Result<Vec<String>, String> {
    dbx_core::schema::list_invalid_indexes_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn get_table_partitioning(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
) -> Result<dbx_core::db::PgTablePartitioning, String> {
    dbx_core::schema::get_table_partitioning_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn list_subpartitions(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
) -> Result<Vec<dbx_core::db::SubpartitionInfo>, String> {
    dbx_core::schema::list_subpartitions_core(&state, &connection_id, &database, &schema, &table).await
}

#[tauri::command]
pub async fn get_table_ddl(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
    object_type: Option<db::ObjectSourceKind>,
    catalog: Option<String>,
    include_postgres_access: Option<bool>,
    portable: Option<bool>,
) -> Result<String, String> {
    {}
    if portable.unwrap_or(false) {
        dbx_core::schema::get_table_export_ddl_core(&state, &connection_id, &database, &schema, &table, object_type)
            .await
    } else if include_postgres_access.unwrap_or(false) {
        dbx_core::schema::get_table_display_ddl_core(&state, &connection_id, &database, &schema, &table, object_type)
            .await
    } else {
        dbx_core::schema::get_table_ddl_core(&state, &connection_id, &database, &schema, &table, object_type).await
    }
}

#[tauri::command]
pub async fn list_functions(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
) -> Result<Vec<db::FunctionInfo>, String> {
    dbx_core::schema::list_functions_core(&state, &connection_id, &database, &schema).await
}

#[tauri::command]
pub async fn list_sequences(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    with_last_values: bool,
) -> Result<Vec<db::SequenceInfo>, String> {
    dbx_core::schema::list_sequences_core(&state, &connection_id, &database, &schema, with_last_values).await
}

#[tauri::command]
pub async fn list_rules(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
) -> Result<Vec<db::RuleInfo>, String> {
    dbx_core::schema::list_rules_core(&state, &connection_id, &database, &schema).await
}

#[tauri::command]
pub async fn list_owners(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
) -> Result<Vec<db::OwnerInfo>, String> {
    dbx_core::schema::list_owners_core(&state, &connection_id, &database, &schema).await
}

#[tauri::command]
pub async fn get_table_owner(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
    schema: String,
    table: String,
) -> Result<Option<String>, String> {
    dbx_core::schema::get_table_owner_core(&state, &connection_id, &database, &schema, &table).await
}
