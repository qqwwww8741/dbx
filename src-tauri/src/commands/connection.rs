use std::collections::HashSet;
use std::sync::Arc;
use tauri::{Emitter, State};

pub use dbx_core::connection::{
    connect_bare_metadata_pool, connect_mysql_metadata_pool, connection_configs_pool_equivalent,
    connection_configs_session_credentials_compatible, connection_url_for_endpoint, metadata_connection_config,
    probe_connection_endpoint, redacted_connection_url_for_endpoint, AppState, MysqlMode, PoolKind,
};
use dbx_core::database_capabilities;
use dbx_core::db;

use dbx_core::models::connection::{
    database_info_from_protocol_value, ConnectionConfig, ConnectionLivenessMessage, ConnectionTestResult,
    DatabaseConnectionInfo, DatabaseType,
};
pub use dbx_core::path_utils::expand_tilde;
use dbx_core::runtime_config::{release_runtime_config_on_disconnect, should_retain_runtime_config};

async fn optional_mysql_database_info(
    pool: &db::mysql::MySqlPool,
    config: &ConnectionConfig,
) -> Option<DatabaseConnectionInfo> {
    match db::mysql::database_connection_info(pool, db::mysql::protocol_product_name(config)).await {
        Ok(info) => Some(info),
        Err(error) => {
            log::warn!("Failed to read optional MySQL database information: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use dbx_core::connection::{AppState, PoolKind};
    use dbx_core::models::connection::{
        AttachedDatabaseConfig, ConnectionConfig, DatabaseType, ProxyTunnelConfig, ProxyType, TransportLayerConfig,
    };
    use std::time::Duration;
    use tauri::Manager;

    #[tokio::test]
    async fn sync_connection_configs_ignores_password_only_changes() {
        let dir = std::env::temp_dir().join(format!("dbx-tauri-conn-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = dbx_core::persistence::test_storage::open(&dir.join("storage.db")).await.unwrap();
        let state = AppState::new_with_plugin_dir(storage, dir.join("plugins"));

        let mut initial: ConnectionConfig = serde_json::from_value(serde_json::json!({"id":"conn-a","name":"MySQL","db_type":"mysql","host":"localhost","port":3306,"username":"root","password":""})).unwrap();
        initial.id = "conn-a".to_string();
        initial.save_password = false;
        initial.password = "session-secret".to_string();
        let _ = state.session_credentials.set("", "conn-a", "session-secret");
        state.configs.write().await.insert(initial.id.clone(), initial.clone());

        // 持久化同步的空密码 config 覆盖运行态：save_password=false 连接仅密码
        // 差异不应销毁池（会话密码由内存仓库提供，与运行态 config 无关）。
        let mut updated = initial.clone();
        updated.password.clear();
        let sync = sync_connection_configs(&state, std::slice::from_ref(&updated)).await;
        assert!(sync.connection_pool_ids_to_drop.is_empty());
        assert_eq!(state.configs.read().await.get("conn-a").map(|c| c.password.as_str()), Some(""));
        assert!(state.session_credentials.has("", "conn-a"));

        // 真实连接参数（host）变化应销毁池，并清除旧会话凭据以便重新输入。
        let mut host_changed = updated.clone();
        host_changed.host = "other-host".to_string();
        let sync2 = sync_connection_configs(&state, std::slice::from_ref(&host_changed)).await;
        assert_eq!(sync2.connection_pool_ids_to_drop.as_slice(), &["conn-a".to_string()]);
        assert!(!state.session_credentials.has("", "conn-a"));

        let _ = std::fs::remove_dir_all(dir);
    }
}

#[tauri::command]
pub async fn save_connections(
    state: State<'_, Arc<AppState>>,
    configs: Vec<ConnectionConfig>,
    removed_ids: Option<Vec<String>>,
) -> Result<(), String> {
    let configs: Vec<ConnectionConfig> = configs.into_iter().map(|config| config.canonicalized()).collect();
    save_connection_configs(state.inner(), &configs, removed_ids.unwrap_or_default()).await
}

async fn save_connection_configs(
    state: &AppState,
    configs: &[ConnectionConfig],
    removed_ids: Vec<String>,
) -> Result<(), String> {
    for config in configs {
        {}
    }
    if !removed_ids.is_empty() {
        state.storage.delete_connections(&removed_ids).await?;
    }
    state.storage.save_connections(configs).await?;
    // Saving upserts, so the request only covers this window's connections. Sync
    // against the whole persisted list to keep connections saved by another
    // window/process alive in the runtime cache as well.
    let persisted = state.storage.load_connections().await?;
    let sync = sync_connection_configs(state, &persisted).await;
    remove_connection_pools_for_connection_ids(state, &sync.connection_pool_ids_to_drop).await;

    Ok(())
}

struct ConnectionConfigSync {
    connection_pool_ids_to_drop: Vec<String>,
}

async fn sync_connection_configs(state: &AppState, configs: &[ConnectionConfig]) -> ConnectionConfigSync {
    let saved_ids: HashSet<&str> = configs.iter().map(|config| config.id.as_str()).collect();

    let mut connection_pool_ids_to_drop = HashSet::new();
    let mut runtime_configs = state.configs.write().await;
    runtime_configs.retain(|id, existing| {
        if saved_ids.contains(id.as_str()) || should_retain_runtime_config(id, existing) {
            true
        } else {
            connection_pool_ids_to_drop.insert(id.clone());
            // 连接已被删除：同步清理本次运行期会话凭据。
            state.session_credentials.clear_connection(id);
            {}
            {}
            false
        }
    });
    for config in configs {
        {}
        {}
        if let Some(previous) = runtime_configs.insert(config.id.clone(), config.clone()) {
            {}
            {}
            if !connection_configs_session_credentials_compatible(&previous, config) {
                // 端点或认证身份变化后，旧密码不能安全复用。显示范围等本地设置
                // 不影响凭据归属，因此必须保留 no-save 连接的新会话密码。
                state.session_credentials.clear_connection(&config.id);
            }
            // 仅在真实连接参数变化时销毁池；save_password=false 连接因持久化
            // 空密码与运行态密码产生的差异被忽略（见 connection_configs_pool_equivalent）。
            if !connection_configs_pool_equivalent(&previous, config) {
                connection_pool_ids_to_drop.insert(config.id.clone());
            }
        }
    }
    ConnectionConfigSync { connection_pool_ids_to_drop: connection_pool_ids_to_drop.into_iter().collect() }
}

async fn remove_connection_pools_for_connection_ids(state: &AppState, connection_ids: &[String]) {
    for connection_id in connection_ids {
        state.remove_connection_pools_detached(connection_id).await;
    }
}

#[tauri::command]
pub async fn load_connections(state: State<'_, Arc<AppState>>) -> Result<Vec<ConnectionConfig>, String> {
    load_connection_configs(state.inner()).await
}

async fn load_connection_configs(state: &AppState) -> Result<Vec<ConnectionConfig>, String> {
    let configs: Vec<ConnectionConfig> =
        state.storage.load_connections().await?.into_iter().map(|config| config.canonicalized()).collect();
    let sync = sync_connection_configs(state, &configs).await;
    remove_connection_pools_for_connection_ids(state, &sync.connection_pool_ids_to_drop).await;

    Ok(configs)
}

#[tauri::command]
pub async fn save_sidebar_layout(state: State<'_, Arc<AppState>>, layout: serde_json::Value) -> Result<(), String> {
    state.storage.save_sidebar_layout(&layout).await
}

#[tauri::command]
pub async fn load_sidebar_layout(state: State<'_, Arc<AppState>>) -> Result<Option<serde_json::Value>, String> {
    state.storage.load_sidebar_layout().await
}

#[tauri::command]
pub async fn save_table_vgroups(
    state: State<'_, Arc<AppState>>,
    scope_key: String,
    layout: serde_json::Value,
) -> Result<(), String> {
    state.storage.save_table_vgroups(&scope_key, &layout).await
}

#[tauri::command]
pub async fn load_table_vgroups(state: State<'_, Arc<AppState>>) -> Result<serde_json::Value, String> {
    state.storage.load_table_vgroups().await
}

#[tauri::command]
pub async fn delete_table_vgroups_for_connection(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<(), String> {
    state.storage.delete_table_vgroups_for_connection(&connection_id).await
}

#[tauri::command]
pub async fn test_connection(state: State<'_, Arc<AppState>>, config: ConnectionConfig) -> Result<String, String> {
    test_connection_with_info_inner(state.inner(), config).await.map(|result| result.message)
}

#[tauri::command]
pub async fn test_connection_with_info(
    state: State<'_, Arc<AppState>>,
    config: ConnectionConfig,
) -> Result<ConnectionTestResult, String> {
    test_connection_with_info_inner(state.inner(), config).await
}

#[tauri::command]
pub async fn test_ssh_tunnel(state: State<'_, Arc<AppState>>, config: ConnectionConfig) -> Result<String, String> {
    state.test_connection_ssh_tunnel(&config.canonicalized()).await
}

async fn test_connection_with_info_inner(
    state: &Arc<AppState>,
    config: ConnectionConfig,
) -> Result<ConnectionTestResult, String> {
    let config = { config };
    let tunnel_id = format!("{}:test", config.id);
    let has_transport_layers = config.has_effective_transport_layers();
    let connection_id = if has_transport_layers { tunnel_id.as_str() } else { config.id.as_str() };
    let endpoint = state.plugin_connection_endpoint(connection_id, &config).await?;
    let (host, port) = (endpoint.host.clone(), endpoint.port);
    let runtime_proxy = endpoint.proxy;
    // SOCKS-routed plugin tests dial through the route themselves; probing the
    // logical endpoint (possibly empty host/port) would mislead.
    let probe_result =
        if runtime_proxy.is_some() { Ok(()) } else { probe_connection_endpoint(&config, &host, port).await };
    let url = connection_url_for_endpoint(&config, &host, port);
    let target = redacted_connection_url_for_endpoint(&config, &host, port);
    let connect_timeout = std::time::Duration::from_secs(config.effective_connect_timeout_secs());
    let idle_timeout = std::time::Duration::from_secs(config.idle_timeout_secs);

    log::info!("[test_connection] db_type={:?} target={}", config.db_type, target);
    {}
    let mut database_info = None;
    let result = match probe_result {
        Err(e) => Err(e),
        Ok(()) => match config.db_type {
            DatabaseType::Mysql => {
                match db::mysql::connect_with_ca_cert(&url, Some(&config.ca_cert_path), connect_timeout).await {
                    Ok(pool) => {
                        database_info = optional_mysql_database_info(&pool, &config).await;
                        let _ = pool.disconnect().await;
                        Ok("Connection successful".to_string())
                    }
                    Err(e) => Err(e),
                }
            }

            db_type => Err(format!("Unsupported database type: {db_type:?}")),
        },
    };

    if has_transport_layers {
        state.reset_connection_transport_for_config(&tunnel_id, &config).await;
    }

    result.map(|message| ConnectionTestResult::success(message).with_database_info(database_info))
}

/// 连接成功且 `save_password=false` 时，把本次输入的密码记入内存会话凭据仓库，
/// 供本次运行内 AI / 元数据 / 池重建复用（进程退出即丢，绝不落盘）。
fn record_session_credential(state: &AppState, config: &ConnectionConfig, connection_id: &str) {
    if !config.save_password && !config.password.is_empty() {
        let _ = state.session_credentials.set("", connection_id, &config.password);
    }
}

#[tauri::command]
pub async fn connect_db(
    state: State<'_, Arc<AppState>>,
    config: ConnectionConfig,
    client_attempt: Option<u64>,
) -> Result<String, String> {
    let config = config.canonicalized();
    {}
    let id = config.id.clone();
    let mut db_config = metadata_connection_config(&config);
    // save_password=false 连接：前端在会话凭据存在时跳过弹窗并以空密码请求，
    // 此处从运行期会话凭据仓库补主密码，使重连/AI 新建池不再 ORA-01005。
    state.apply_session_credential(&config, &mut db_config, &id);
    let attempt = state.begin_connection_attempt_with_client_attempt(&id, client_attempt).await;
    let mut connected_config = config.clone();
    let mut connected_db_config = db_config.clone();

    // Plugin 连接的 connection/connect 是幂等 upsert：连接路径（首连、重推、
    // 重连）只替换池条目，完全跳过旧池拆除——close 会向 sidecar 发
    // connection/disconnect，其语义是"删注册表 + 杀掉该连接全部会话"，会把
    // 同连接其他 tab 的活跃终端一起杀死。用户显式断开仍走 disconnect_db 的
    // 完整关闭路径（会发 disconnect）。其他类型维持 detached 关闭不变。
    {
        state.remove_connection_pools_detached(&id).await;
    }

    state.reset_connection_transport_for_config(&id, &db_config).await;

    let endpoint = state.plugin_connection_endpoint(&id, &db_config).await?;
    let (host, port) = (endpoint.host, endpoint.port);
    let runtime_proxy = endpoint.proxy;
    if let Err(err) = state.ensure_current_connection_attempt(&id, Some(attempt)).await {
        state.reset_connection_transport_for_config(&id, &db_config).await;
        return Err(err);
    }
    {
        probe_connection_endpoint(&db_config, &host, port).await?;
    }
    if let Err(err) = state.ensure_current_connection_attempt(&id, Some(attempt)).await {
        state.reset_connection_transport_for_config(&id, &db_config).await;
        return Err(err);
    }
    let url = connection_url_for_endpoint(&db_config, &host, port);
    let connect_timeout = std::time::Duration::from_secs(db_config.effective_connect_timeout_secs());
    let idle_timeout = std::time::Duration::from_secs(db_config.idle_timeout_secs);

    let pool = match db_config.db_type {
        DatabaseType::Mysql => {
            let (pool, mode) =
                connect_mysql_metadata_pool(&config, &db_config, &host, port, connect_timeout, 3).await?;
            PoolKind::Mysql(pool, mode)
        }

        db_type => return Err(format!("Unsupported database type: {db_type:?}")),
    };

    if let Err(err) =
        state.insert_connection_pool_for_attempt(&id, attempt, id.clone(), pool, &connected_db_config).await
    {
        state.reset_connection_transport_for_config(&id, &connected_db_config).await;
        return Err(err);
    }
    record_session_credential(state.inner(), &connected_config, &id);
    // 存入全局运行态 configs 的配置脱敏（no-save 密码恒为空），明文只存在于会话凭据仓库。
    let mut stored = connected_config;
    if !stored.save_password {
        stored.password.clear();
    }
    state.configs.write().await.insert(id.clone(), stored);

    Ok(id)
}

#[tauri::command]
pub async fn connection_final_proxy_port(
    state: State<'_, Arc<AppState>>,
    config: ConnectionConfig,
) -> Result<u16, String> {
    let runtime_config = config.canonicalized();
    if !runtime_config.has_effective_transport_layers() {
        return Err("Connection has no configured transport layers".to_string());
    }
    {}

    let connection_id = runtime_config.id.clone();
    let db_config = metadata_connection_config(&runtime_config);
    // This pre-connect path caches the configuration for tunnel resolution. Keep
    // no-save passwords out of that shared runtime cache just like connect_db.
    let mut stored_config = runtime_config.clone();
    if !stored_config.save_password {
        stored_config.password.clear();
    }
    state.configs.write().await.insert(connection_id.clone(), stored_config);

    let (_, port) = state.connection_host_port(&connection_id, &db_config).await?;
    record_session_credential(state.inner(), &runtime_config, &connection_id);
    Ok(port)
}

#[tauri::command]
pub async fn disconnect_db(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    client_attempt: Option<u64>,
) -> Result<(), String> {
    let should_disconnect = if let Some(client_attempt) = client_attempt {
        state.supersede_connection_attempt_if_client_attempt(&connection_id, client_attempt).await
    } else {
        state.supersede_connection_attempt(&connection_id).await;
        true
    };
    if !should_disconnect {
        return Ok(());
    }
    state.running_queries.cancel_connection(&connection_id);
    state.remove_connection_pools_detached(&connection_id).await;

    state.reset_connection_transport(&connection_id).await;
    release_runtime_config_on_disconnect(state.inner(), &connection_id).await;
    Ok(())
}

#[tauri::command]
pub async fn close_database_connection(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: String,
) -> Result<bool, String> {
    let database = database.trim();
    let database = if database.is_empty() { None } else { Some(database) };
    state.close_database_pool(&connection_id, database).await
}

/// 查询连接在本次运行期是否已输入并暂存密码（`save_password=false`）。
/// 供前端决定是否需要弹密码框；仅返回布尔状态，不泄露密码本身。
#[tauri::command]
pub async fn session_credential_status(state: State<'_, Arc<AppState>>, connection_id: String) -> Result<bool, String> {
    Ok(state.session_credentials.has("", &connection_id))
}

/// "断开并忘记本次密码"：清除连接本次运行期的临时密码，下次连接需重新输入。
/// 只清内存会话凭据，不影响持久化配置与已保存密码。
#[tauri::command]
pub async fn forget_session_credential(state: State<'_, Arc<AppState>>, connection_id: String) -> Result<(), String> {
    if !state.session_credentials.has("", &connection_id) {
        return Err(format!("Connection has no transient session credential to forget: {connection_id}"));
    }
    state.session_credentials.remove("", &connection_id);
    Ok(())
}

/// 清空全部运行期会话凭据（桌面端退出前调用；Web 端登出时走 `auth.rs logout`）。
/// 密码只存在于本次进程内存，进程退出本就会丢失；显式清除用于退出前兜底。
#[tauri::command]
pub async fn clear_all_session_credentials(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.session_credentials.clear();
    Ok(())
}

#[tauri::command]
pub async fn refresh_connections(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.refresh_connections().await;
    Ok(())
}

#[tauri::command]
pub async fn check_connection_health(state: State<'_, Arc<AppState>>, connection_id: String) -> Result<(), String> {
    state.check_connection_health(&connection_id).await
}

/// Read-only counterpart of `check_connection_health`: reports whether the connection still
/// has a pool, without probing, mutating, or triggering a reconnect.
///
/// The frontend uses it to confirm a keepalive liveness event before greying the sidebar
/// (#4339). `check_connection_health` must never be used for that confirmation: it removes
/// unhealthy pools and is the path `ensureConnected` uses to reconnect.
#[tauri::command]
pub async fn connection_is_open(state: State<'_, Arc<AppState>>, connection_id: String) -> Result<bool, String> {
    Ok(state.is_connection_open(&connection_id).await)
}

/// Relay backend-confirmed liveness losses to the frontend (#4339).
///
/// Mirrors `install_plugin_event_bridge`: the core publishes to a broadcast channel and each
/// shell owns its transport, so core never needs a UI handle.
pub fn install_connection_liveness_bridge(app: &tauri::AppHandle, state: Arc<AppState>) {
    let app_handle = app.clone();
    let mut events = state.subscribe_connection_liveness();
    tauri::async_runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(message) => {
                    let _ = app_handle.emit("dbx-connection-liveness", message);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // The skipped messages are gone for good, so ask the frontend to re-check
                    // every connection it still shows as connected. Dropping the transition
                    // silently would leave a sidebar green indefinitely — exactly the state
                    // this bridge exists to prevent.
                    log::warn!("Desktop connection liveness bridge skipped {skipped} messages; requesting a resync");
                    let _ = app_handle.emit("dbx-connection-liveness", ConnectionLivenessMessage::Resync);
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// Warm the driver and connection pool for a connection a tab is opening, so the
/// first Run does not pay pool creation, tunnel setup, or external-driver (JDBC
/// agent) startup while the user waits. Never removes an existing pool.
#[tauri::command]
pub async fn prewarm_connection(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: Option<String>,
    catalog: Option<String>,
    client_session_id: Option<String>,
) -> Result<(), String> {
    state
        .prewarm_connection_pool(
            &connection_id,
            database.as_deref().filter(|value| !value.is_empty()),
            catalog.as_deref().filter(|value| !value.is_empty()),
            client_session_id.as_deref().filter(|value| !value.is_empty()),
        )
        .await
}

#[tauri::command]
pub async fn connection_identifier_quote(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: Option<String>,
) -> Result<Option<String>, String> {
    state.connection_identifier_quote(&connection_id, database.as_deref()).await
}

#[tauri::command]
pub async fn connection_database_info(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database: Option<String>,
) -> Result<Option<DatabaseConnectionInfo>, String> {
    state.connection_database_info(&connection_id, database.as_deref()).await
}

#[tauri::command]
pub async fn save_connection_database_info(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    database_info: Option<DatabaseConnectionInfo>,
) -> Result<(), String> {
    state.save_connection_database_info(&connection_id, database_info).await
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteUnlockState {
    pub remaining_ms: u64,
}

#[tauri::command]
pub async fn unlock_connection_writes(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
    duration_secs: u64,
) -> Result<WriteUnlockState, String> {
    if !state.configs.read().await.contains_key(&connection_id) {
        return Err("Connection not found".to_string());
    }
    let remaining_ms = state.write_unlock_windows.unlock(&connection_id, duration_secs).await?;
    Ok(WriteUnlockState { remaining_ms })
}

#[tauri::command]
pub async fn lock_connection_writes(state: State<'_, Arc<AppState>>, connection_id: String) -> Result<(), String> {
    state.write_unlock_windows.lock(&connection_id).await;
    Ok(())
}

#[tauri::command]
pub async fn connection_write_unlock_state(
    state: State<'_, Arc<AppState>>,
    connection_id: String,
) -> Result<WriteUnlockState, String> {
    Ok(WriteUnlockState { remaining_ms: state.write_unlock_windows.remaining_ms(&connection_id).await })
}

/// Check whether a connection has read-only protection enabled.
/// Returns an error if the connection is read-only, preventing write operations.
pub async fn ensure_connection_writable(
    state: &Arc<AppState>,
    connection_id: &str,
    action: &str,
) -> Result<(), String> {
    if let Some(name) = dbx_core::query::connection_readonly_name(state, connection_id).await {
        return Err(format!(
            "Read-only mode: connection '{}' has read-only protection enabled. {} blocked.",
            name, action
        ));
    }
    Ok(())
}
