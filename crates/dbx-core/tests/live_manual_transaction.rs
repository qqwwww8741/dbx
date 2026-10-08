use dbx_core::connection::AppState;
use dbx_core::database_export::{
    begin_database_backup_snapshot_core, clear_export_cancelled, export_database_sql_core, set_export_cancelled,
    DatabaseExportRequest, ExportStatus,
};
use dbx_core::models::connection::{ConnectionConfig, DatabaseType};
use dbx_core::query::{
    begin_manual_transaction, commit_manual_transaction, execute_in_manual_transaction, execute_sql_statement,
    rollback_manual_transaction, stream_rows_in_manual_transaction,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

fn live_config(prefix: &str, db_type: DatabaseType, default_port: u16) -> ConnectionConfig {
    let host = std::env::var(format!("{prefix}_HOST")).expect("live DB host env var");
    let port = std::env::var(format!("{prefix}_PORT"))
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(default_port);
    let username = std::env::var(format!("{prefix}_USER")).expect("live DB user env var");
    let password = std::env::var(format!("{prefix}_PASSWORD")).expect("live DB password env var");
    let database = std::env::var(format!("{prefix}_DATABASE")).expect("live DB database env var");
    let url_params = std::env::var(format!("{prefix}_URL_PARAMS")).ok();

    serde_json::from_value(serde_json::json!({
        "id": format!("manual-txn-{prefix}"),
        "name": format!("Manual transaction {prefix}"),
        "db_type": db_type,
        "host": host,
        "port": port,
        "username": username,
        "password": password,
        "database": database,
        "connect_timeout_secs": 5,
        "query_timeout_secs": 30,
        "idle_timeout_secs": 60,
        "keepalive_interval_secs": 0,
        "url_params": url_params
    }))
    .expect("live connection config should deserialize")
}

async fn app_state_with_config(config: ConnectionConfig) -> (Arc<AppState>, std::path::PathBuf) {
    let db_path = std::env::temp_dir().join(format!("dbx-live-manual-txn-{}.db", uuid::Uuid::new_v4().simple()));
    let storage = dbx_core::persistence::test_storage::open(&db_path).await.expect("open temp storage");
    let state = Arc::new(AppState::new(storage));
    state.configs.write().await.insert(config.id.clone(), config);
    (state, db_path)
}

#[tokio::test]
#[ignore = "requires DBX_LIVE_MANUAL_TXN_MYSQL_* env vars pointing at writable MySQL"]
async fn live_manual_transaction_mysql_streams_with_row_limit() {
    let config = live_config("DBX_LIVE_MANUAL_TXN_MYSQL", DatabaseType::Mysql, 3306);
    let database = config.database.clone().expect("database");
    let (state, db_path) = app_state_with_config(config.clone()).await;

    let txn = begin_manual_transaction(&state, &config.id, &database, None, None).await.expect("begin");
    let limited = execute_in_manual_transaction(
        &state,
        &txn,
        "SELECT 1 AS id UNION ALL SELECT 2 UNION ALL SELECT 3",
        &database,
        None,
        Some(2),
    )
    .await
    .expect("limited select");
    assert_eq!(limited[0].columns, vec!["id"]);
    assert_eq!(limited[0].rows.len(), 2);
    assert!(limited[0].truncated);

    rollback_manual_transaction(&state, &txn).await.expect("rollback");
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
#[ignore = "requires DBX_LIVE_MANUAL_TXN_MYSQL_* env vars pointing at readable MySQL with table t_0001"]
async fn live_mysql_database_backup_refreshes_an_idle_snapshot_before_export() {
    let config = live_config("DBX_LIVE_MANUAL_TXN_MYSQL", DatabaseType::Mysql, 3306);
    let database = config.database.clone().expect("database");
    let (state, db_path) = app_state_with_config(config.clone()).await;
    let export_path = std::env::temp_dir().join(format!("dbx-live-backup-{}.sql", uuid::Uuid::new_v4().simple()));

    let snapshot = begin_database_backup_snapshot_core(&state, &config.id, &database).await.expect("begin snapshot");
    {
        let mut sessions = state.transaction_sessions.write().await;
        sessions.get_mut(&snapshot.session_id).expect("snapshot session").last_activity =
            Instant::now() - Duration::from_secs(301);
    }

    let request = DatabaseExportRequest {
        export_id: format!("live-mysql-backup-{}", uuid::Uuid::new_v4().simple()),
        connection_id: config.id.clone(),
        database: database.clone(),
        schema: database.clone(),
        file_path: export_path.to_string_lossy().to_string(),
        selected_tables: vec!["t_0001".to_string()],
        excluded_tables: Vec::new(),
        include_structure: true,
        include_data: true,
        include_objects: false,
        include_create_database: false,
        drop_table_if_exists: false,
        omit_auto_increment: false,
        preserve_original_language: false,
        fail_on_error: true,
        prevent_overwrite: false,
        output_compression: Default::default(),
        insert_dialect: Default::default(),
        insert_mode: Default::default(),
        snapshot_session_id: Some(snapshot.session_id.clone()),
        batch_size: 100,
        split_max_mb: None,
    };
    let export_result = export_database_sql_core(&state, &request, |_| {}).await;
    let rollback_result = rollback_manual_transaction(&state, &snapshot.session_id).await;

    export_result.expect("export through refreshed snapshot");
    rollback_result.expect("rollback snapshot");
    let sql = std::fs::read_to_string(&export_path).expect("read exported SQL");
    assert!(sql.contains("t_0001"));

    let _ = std::fs::remove_file(export_path);
    let _ = std::fs::remove_file(db_path);
}
