use std::collections::HashSet;
use std::convert::Infallible;
use std::sync::Arc;

use async_stream::stream;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use dbx_core::connection::{
    connection_configs_pool_equivalent, connection_configs_session_credentials_compatible, AppState, PoolKind,
};
use dbx_core::models::connection::{
    ConnectionConfig, ConnectionLivenessMessage, ConnectionTestResult, DatabaseConnectionInfo, DatabaseType,
};

use dbx_core::runtime_config::{
    release_runtime_config_on_disconnect, should_retain_runtime_config, TEST_PROBE_ID_PREFIX,
};

use dbx_core::session_credentials::{PurposeSessionCredentialWriteToken, SessionCredentialWriteToken};
use serde::{Deserialize, Serialize};

use crate::auth::session_token_from_headers;
use crate::error::AppError;
use crate::state::WebState;

#[derive(Default)]
struct NoSaveRuntimeSecrets {
    primary: Option<String>,
    console: Option<String>,
}

#[derive(Default)]
struct SessionCredentialWrites {
    primary: Option<SessionCredentialWriteToken>,
    purposes: Vec<PurposeSessionCredentialWriteToken>,
}

fn prepare_runtime_config(mut config: ConnectionConfig) -> (ConnectionConfig, NoSaveRuntimeSecrets) {
    if config.save_password {
        return (config, NoSaveRuntimeSecrets::default());
    }
    {}
    let primary = std::mem::take(&mut config.password);
    (config, NoSaveRuntimeSecrets { primary: (!primary.is_empty()).then_some(primary), console: None })
}

fn record_session_credentials(
    app: &AppState,
    owner: &str,
    connection_id: &str,
    secrets: &NoSaveRuntimeSecrets,
    nacos: bool,
) -> SessionCredentialWrites {
    let primary =
        secrets.primary.as_deref().and_then(|password| app.session_credentials.set(owner, connection_id, password));
    let mut purposes = Vec::new();
    {}
    SessionCredentialWrites { primary, purposes }
}

fn rollback_session_credential_writes(app: &AppState, writes: &SessionCredentialWrites) {
    for token in &writes.purposes {
        app.session_credentials.remove_purpose_if_current(token);
    }
    if let Some(token) = writes.primary.as_ref() {
        app.session_credentials.remove_if_current(token);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub config: ConnectionConfig,
    pub client_attempt: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisconnectRequest {
    pub connection_id: String,
    pub client_attempt: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrewarmConnectionRequest {
    pub connection_id: String,
    pub database: Option<String>,
    pub catalog: Option<String>,
    pub client_session_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseDatabaseConnectionRequest {
    pub connection_id: String,
    pub database: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCredentialStatusRequest {
    pub connection_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionIdentifierQuoteRequest {
    pub connection_id: String,
    pub database: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConnectionDatabaseInfoRequest {
    pub connection_id: String,
    pub database_info: Option<DatabaseConnectionInfo>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockConnectionWritesRequest {
    pub connection_id: String,
    pub duration_secs: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteUnlockStateResponse {
    pub remaining_ms: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveConnectionsRequest {
    pub configs: Vec<ConnectionConfig>,
    /// Ids the client deleted locally. Connections are saved by upsert, so a
    /// client that no longer lists a connection must say so explicitly instead
    /// of wiping every connection another client may have created meanwhile.
    #[serde(default)]
    pub removed_ids: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAddConnectionRequest {
    pub config: ConnectionConfig,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpDuplicateConnectionRequest {
    pub source_id: String,
    pub copy_id: String,
    pub copy_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRemoveConnectionRequest {
    pub connection_id: String,
}

fn is_connection_info_capability_unsupported(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("connectioninfo")
        && (error.contains("unsupported") || error.contains("unknown method") || error.contains("method not found"))
}

async fn run_temporary_connection_test(
    app: &Arc<AppState>,
    config: ConnectionConfig,
    include_database_info: bool,
) -> Result<ConnectionTestResult, String> {
    let temp_id = format!("{TEST_PROBE_ID_PREFIX}{}", uuid::Uuid::new_v4());
    app.configs.write().await.insert(temp_id.clone(), config.clone());

    {}

    let pool_result = { app.get_or_create_pool(&temp_id, config.database.as_deref()).await };
    let database_info = if include_database_info {
        match &pool_result {
            Ok(_) => match app.connection_database_info(&temp_id, config.database.as_deref()).await {
                Ok(info) => info,
                Err(error) if is_connection_info_capability_unsupported(&error) => {
                    log::debug!("Connection information capability is unavailable: {error}");
                    None
                }
                Err(error) => {
                    log::warn!("Failed to read optional connection information: {error}");
                    None
                }
            },
            Err(_) => None,
        }
    } else {
        None
    };

    // Keep all fallible post-connect checks inside this block so cleanup below
    // runs before either a successful result or an error is returned.
    let result: Result<ConnectionTestResult, String> = async {
        let success_message = { "Connection successful".to_string() };

        pool_result.map(|_| ConnectionTestResult::success(success_message).with_database_info(database_info))
    }
    .await;

    app.remove_connection_pools(&temp_id).await;
    // Pool drain intentionally keeps durable MQ adapters for reconnect reuse; temporary
    // probes must still release any registry entry if a cached path was used.

    app.reset_connection_transport_for_config(&temp_id, &config).await;
    app.configs.write().await.remove(&temp_id);

    result
}

pub async fn test_connection(
    State(state): State<Arc<WebState>>,
    Json(body): Json<ConnectRequest>,
) -> Result<Json<String>, AppError> {
    run_temporary_connection_test(&state.app, body.config, false)
        .await
        .map(|result| Json(result.message))
        .map_err(AppError::from)
}

pub async fn test_connection_with_info(
    State(state): State<Arc<WebState>>,
    Json(body): Json<ConnectRequest>,
) -> Result<Json<ConnectionTestResult>, AppError> {
    run_temporary_connection_test(&state.app, body.config, true).await.map(Json).map_err(AppError::from)
}

pub async fn test_ssh_tunnel(
    State(state): State<Arc<WebState>>,
    Json(body): Json<ConnectRequest>,
) -> Result<Json<String>, AppError> {
    state.app.test_connection_ssh_tunnel(&body.config.canonicalized()).await.map(Json).map_err(AppError::from)
}

pub async fn connect_db(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(body): Json<ConnectRequest>,
) -> Result<Json<String>, AppError> {
    let config = body.config;
    // 演示模式：只允许连接已保存的连接，端点身份以存储为准，防止伪造 body
    // 配置把服务器拨向任意主机（见 demo 模块）。
    if state.demo_mode {
        crate::demo::ensure_demo_connect_allowed(&state.app, &config).await.map_err(AppError::forbidden)?;
    }
    {}
    let app = &state.app;
    let connection_id = config.id.clone();
    let owner = session_token_from_headers(&headers).unwrap_or_default();
    let attempt = app.begin_connection_attempt_with_client_attempt(&connection_id, body.client_attempt).await;

    // save_password=false 连接：
    // 1) 先把本次输入的密码按 owner（登录会话）记入内存会话凭据仓库，供池创建/
    //    池重建按 owner 读取（见 apply_session_credential）；
    // 2) 存入全局运行态 configs 的配置必须脱敏（password 恒为空），禁止明文驻留
    //    AppState.configs——否则其他登录会话借池重建即可读到它，绕过 owner 隔离。
    // 只有本次请求确实写入/替换了会话凭据（输入了非空密码）时，才需要在连接
    // 失败时回滚它；无密码重连复用的是既有有效凭据，失败不应清掉它，否则瞬时的
    // 数据库/网络抖动就会让"本次会话内记住密码"失效、下次又弹窗。
    let (runtime_config, runtime_secrets) = prepare_runtime_config(config.clone());
    let session_credential_writes = record_session_credentials(app, &owner, &connection_id, &runtime_secrets, false);

    app.remove_connection_pools_detached(&connection_id).await;

    app.reset_connection_transport_for_config(&connection_id, &runtime_config).await;
    app.configs.write().await.insert(connection_id.clone(), runtime_config);

    {}

    if let Err(error) = app.get_or_create_pool_for_connection_attempt(&connection_id, None, attempt).await {
        // 连接失败：仅回滚本次请求刚写入的会话凭据，避免前端误判"已记住密码"而用
        // 失败密码免弹窗重试；无密码重连复用的既有凭据不受瞬时失败影响。
        rollback_session_credential_writes(app, &session_credential_writes);
        return Err(AppError::from(error));
    }
    {}

    Ok(Json(connection_id))
}

pub async fn connected_database_info(
    State(state): State<Arc<WebState>>,
    Json(body): Json<ConnectionIdentifierQuoteRequest>,
) -> Result<Json<Option<DatabaseConnectionInfo>>, AppError> {
    state
        .app
        .connection_database_info(&body.connection_id, body.database.as_deref())
        .await
        .map(Json)
        .map_err(AppError::from)
}

pub async fn save_connection_database_info(
    State(state): State<Arc<WebState>>,
    Json(body): Json<SaveConnectionDatabaseInfoRequest>,
) -> Result<Json<()>, AppError> {
    state
        .app
        .save_connection_database_info(&body.connection_id, body.database_info)
        .await
        .map(|_| Json(()))
        .map_err(AppError::from)
}

pub async fn unlock_connection_writes(
    State(state): State<Arc<WebState>>,
    Json(body): Json<UnlockConnectionWritesRequest>,
) -> Result<Json<WriteUnlockStateResponse>, AppError> {
    if !state.app.configs.read().await.contains_key(&body.connection_id) {
        return Err(AppError::from("Connection not found".to_string()));
    }
    let remaining_ms =
        state.app.write_unlock_windows.unlock(&body.connection_id, body.duration_secs).await.map_err(AppError::from)?;
    Ok(Json(WriteUnlockStateResponse { remaining_ms }))
}

pub async fn lock_connection_writes(
    State(state): State<Arc<WebState>>,
    Json(body): Json<ConnectionIdentifierQuoteRequest>,
) -> Result<Json<()>, AppError> {
    state.app.write_unlock_windows.lock(&body.connection_id).await;
    Ok(Json(()))
}

pub async fn connection_write_unlock_state(
    State(state): State<Arc<WebState>>,
    Json(body): Json<ConnectionIdentifierQuoteRequest>,
) -> Result<Json<WriteUnlockStateResponse>, AppError> {
    Ok(Json(WriteUnlockStateResponse {
        remaining_ms: state.app.write_unlock_windows.remaining_ms(&body.connection_id).await,
    }))
}

pub async fn connection_final_proxy_port(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(body): Json<ConnectRequest>,
) -> Result<Json<u16>, AppError> {
    let runtime_config = body.config.canonicalized();
    if !runtime_config.has_effective_transport_layers() {
        return Err(AppError::from("Connection has no configured transport layers".to_string()));
    }
    {}

    let app = &state.app;
    let connection_id = runtime_config.id.clone();
    let db_config = dbx_core::connection::metadata_connection_config(&runtime_config);
    // Tunnel resolution needs a runtime config lookup, but a no-save password
    // must never enter the shared Web config cache and bypass owner isolation.
    let (stored_config, runtime_secrets) = prepare_runtime_config(runtime_config.clone());
    app.configs.write().await.insert(connection_id.clone(), stored_config);

    let (_, port) = app.connection_host_port(&connection_id, &db_config).await.map_err(AppError::from)?;
    let owner = session_token_from_headers(&headers).unwrap_or_default();
    record_session_credentials(app, &owner, &connection_id, &runtime_secrets, false);
    Ok(Json(port))
}

pub async fn disconnect_db(
    State(state): State<Arc<WebState>>,
    Json(body): Json<DisconnectRequest>,
) -> Result<Json<()>, AppError> {
    let app = &state.app;

    let should_disconnect = if let Some(client_attempt) = body.client_attempt {
        app.supersede_connection_attempt_if_client_attempt(&body.connection_id, client_attempt).await
    } else {
        app.supersede_connection_attempt(&body.connection_id).await;
        true
    };
    if !should_disconnect {
        return Ok(Json(()));
    }
    app.running_queries.cancel_connection(&body.connection_id);
    app.remove_connection_pools_detached(&body.connection_id).await;

    app.reset_connection_transport(&body.connection_id).await;
    release_runtime_config_on_disconnect(app, &body.connection_id).await;

    Ok(Json(()))
}

pub async fn check_connection_health(
    State(state): State<Arc<WebState>>,
    Json(body): Json<DisconnectRequest>,
) -> Result<Json<()>, AppError> {
    state.app.check_connection_health(&body.connection_id).await.map_err(AppError::from)?;
    Ok(Json(()))
}

/// Read-only counterpart of `check_connection_health`: reports whether the connection still
/// has a pool, without probing, mutating, or triggering a reconnect (#4339).
pub async fn connection_is_open(
    State(state): State<Arc<WebState>>,
    Json(body): Json<DisconnectRequest>,
) -> Result<Json<bool>, AppError> {
    Ok(Json(state.app.is_connection_open(&body.connection_id).await))
}

/// Serialise one liveness message for the SSE stream, or `None` when serialisation fails.
fn liveness_sse_event(message: &ConnectionLivenessMessage) -> Option<Event> {
    match serde_json::to_string(message) {
        Ok(payload) => Some(Event::default().data(payload)),
        // The payload is a tagged enum of plain fields, so this is unreachable in practice.
        Err(error) => {
            log::warn!("Connection liveness message could not be serialised: {error}");
            None
        }
    }
}

/// Stream backend-confirmed liveness messages to the browser (#4339).
///
/// The payload is the message itself, byte-identical to what the desktop shell emits as
/// `dbx-connection-liveness`, so the frontend parses one shape on both transports.
pub async fn connection_liveness_events(
    State(state): State<Arc<WebState>>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let mut events = state.app.subscribe_connection_liveness();
    let stream = stream! {
        loop {
            match events.recv().await {
                Ok(message) => {
                    if let Some(event) = liveness_sse_event(&message) {
                        yield Ok(event);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // The skipped messages are gone for good, so ask the client to re-check
                    // every connection it still shows as connected. Dropping the transition
                    // silently would leave a sidebar green indefinitely — exactly the state
                    // this stream exists to prevent.
                    log::warn!("Web connection liveness stream skipped {skipped} messages; requesting a resync");
                    if let Some(event) = liveness_sse_event(&ConnectionLivenessMessage::Resync) {
                        yield Ok(event);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

pub async fn prewarm_connection(
    State(state): State<Arc<WebState>>,
    Json(body): Json<PrewarmConnectionRequest>,
) -> Result<Json<()>, AppError> {
    let database = body.database.as_deref().filter(|value| !value.is_empty());
    let catalog = body.catalog.as_deref().filter(|value| !value.is_empty());
    let client_session_id = body.client_session_id.as_deref().filter(|value| !value.is_empty());
    state
        .app
        .prewarm_connection_pool(&body.connection_id, database, catalog, client_session_id)
        .await
        .map_err(AppError::from)?;
    Ok(Json(()))
}

/// 查询连接在本次运行期是否已输入并暂存密码（`save_password=false`）。
/// 供前端决定是否需要弹密码框；仅返回布尔状态，不泄露密码本身。
/// 按当前登录会话（owner）查询，不会暴露其他会话的凭据状态。
pub async fn session_credential_status(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(body): Json<SessionCredentialStatusRequest>,
) -> Result<Json<bool>, AppError> {
    let owner = session_token_from_headers(&headers).unwrap_or_default();
    Ok(Json(state.app.session_credentials.has(&owner, &body.connection_id)))
}

/// "断开并忘记本次密码"：清除连接本次运行期的临时密码，下次连接需重新输入。
/// 只清当前登录会话的内存凭据，不影响其他会话与持久化配置。
pub async fn forget_session_credential(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(body): Json<SessionCredentialStatusRequest>,
) -> Result<Json<()>, AppError> {
    let owner = session_token_from_headers(&headers).unwrap_or_default();
    if !state.app.session_credentials.has(&owner, &body.connection_id) {
        return Err(AppError::from(format!(
            "Connection has no transient session credential to forget: {}",
            body.connection_id
        )));
    }
    state.app.session_credentials.remove(&owner, &body.connection_id);
    Ok(Json(()))
}

pub async fn connection_identifier_quote(
    State(state): State<Arc<WebState>>,
    Json(body): Json<ConnectionIdentifierQuoteRequest>,
) -> Result<Json<Option<String>>, AppError> {
    state
        .app
        .connection_identifier_quote(&body.connection_id, body.database.as_deref())
        .await
        .map(Json)
        .map_err(AppError::from)
}

pub async fn close_database_connection(
    State(state): State<Arc<WebState>>,
    Json(body): Json<CloseDatabaseConnectionRequest>,
) -> Result<Json<bool>, AppError> {
    let database = body.database.trim();
    let database = if database.is_empty() { None } else { Some(database) };
    state.app.close_database_pool(&body.connection_id, database).await.map(Json).map_err(AppError::from)
}

pub async fn save_connections(
    State(state): State<Arc<WebState>>,
    headers: HeaderMap,
    Json(body): Json<SaveConnectionsRequest>,
) -> Result<Json<()>, AppError> {
    for config in &body.configs {
        {}
    }
    if !body.removed_ids.is_empty() {
        state.app.storage.delete_connections(&body.removed_ids).await.map_err(AppError::from)?;
    }
    state.app.storage.save_connections(&body.configs).await.map_err(AppError::from)?;
    let owner = session_token_from_headers(&headers).unwrap_or_default();
    let runtime_configs = body.configs.iter().cloned().map(prepare_runtime_config).collect::<Vec<_>>();
    // Saving is an upsert, so the request only describes the connections of this
    // client. Sync the runtime cache against the whole persisted list instead of
    // the request payload, otherwise a concurrent save from another client would
    // drop its runtime config, pool and session credentials.
    let persisted = state.app.storage.load_connections().await.map_err(AppError::from)?;
    let sync = sync_connection_configs(&state, &persisted).await;
    for (config, secrets) in &runtime_configs {
        record_session_credentials(&state.app, &owner, &config.id, secrets, false);
    }
    remove_connection_pools_for_connection_ids(&state, &sync.connection_pool_ids_to_drop).await;

    drop_mq_adapters_for_connection_ids(&state, &sync.mq_adapter_ids_to_drop).await;
    Ok(Json(()))
}

pub async fn mcp_add_connection(
    State(state): State<Arc<WebState>>,
    Json(body): Json<McpAddConnectionRequest>,
) -> Result<Json<ConnectionConfig>, AppError> {
    let saved = state.app.storage.add_connection_for_mcp(body.config).await.map_err(AppError::from)?;
    state.app.session_credentials.clear_connection(&saved.id);
    state.app.remove_connection_pools_detached(&saved.id).await;
    state.app.configs.write().await.insert(saved.id.clone(), saved.clone());
    Ok(Json(saved))
}

pub async fn mcp_duplicate_connection(
    State(state): State<Arc<WebState>>,
    Json(body): Json<McpDuplicateConnectionRequest>,
) -> Result<Json<ConnectionConfig>, AppError> {
    let saved = state
        .app
        .storage
        .duplicate_connection_for_mcp(&body.source_id, &body.copy_id, &body.copy_name)
        .await
        .map_err(AppError::from)?;
    state.app.session_credentials.clear_connection(&saved.id);
    state.app.remove_connection_pools_detached(&saved.id).await;
    state.app.configs.write().await.insert(saved.id.clone(), saved.clone());
    Ok(Json(saved))
}

pub async fn mcp_remove_connection(
    State(state): State<Arc<WebState>>,
    _headers: HeaderMap,
    Json(body): Json<McpRemoveConnectionRequest>,
) -> Result<Json<bool>, AppError> {
    let connection_id = body.connection_id;
    let removed = state.app.storage.remove_connection_for_mcp(&connection_id).await.map_err(AppError::from)?;

    Ok(Json(removed))
}

pub async fn load_connections(
    State(state): State<Arc<WebState>>,
    _headers: HeaderMap,
) -> Result<Json<Vec<ConnectionConfig>>, AppError> {
    let configs = state.app.storage.load_connections().await.map_err(AppError::from)?;
    let sync = sync_connection_configs(&state, &configs).await;
    remove_connection_pools_for_connection_ids(&state, &sync.connection_pool_ids_to_drop).await;

    drop_mq_adapters_for_connection_ids(&state, &sync.mq_adapter_ids_to_drop).await;
    Ok(Json(configs))
}

struct ConnectionConfigSync {
    nacos_adapter_ids_to_drop: Vec<String>,
    mq_adapter_ids_to_drop: Vec<String>,
    connection_pool_ids_to_drop: Vec<String>,
}

async fn sync_connection_configs(state: &WebState, configs: &[ConnectionConfig]) -> ConnectionConfigSync {
    let saved_ids: HashSet<&str> = configs.iter().map(|config| config.id.as_str()).collect();
    let mut nacos_adapter_ids_to_drop = HashSet::new();
    let mut mq_adapter_ids_to_drop = HashSet::new();
    let mut connection_pool_ids_to_drop = HashSet::new();
    let mut runtime_configs = state.app.configs.write().await;
    runtime_configs.retain(|id, existing| {
        if saved_ids.contains(id.as_str()) || should_retain_runtime_config(id, existing) {
            true
        } else {
            connection_pool_ids_to_drop.insert(id.clone());
            // 连接已被全局删除：清除所有 Web owner 的临时凭据与池 owner。
            state.app.session_credentials.clear_connection(id);
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
                // 全局端点或认证身份变化时清除所有 owner 的旧凭据；显示范围等
                // 本地设置不改变凭据归属，必须保留各 owner 的 no-save 密码。
                state.app.session_credentials.clear_connection(&config.id);
            }
            // 仅在真实连接参数变化时销毁池；save_password=false 连接因持久化
            // 空密码与运行态密码产生的差异被忽略（见 connection_configs_pool_equivalent）。
            if !connection_configs_pool_equivalent(&previous, config) {
                connection_pool_ids_to_drop.insert(config.id.clone());
            }
        }
    }
    ConnectionConfigSync {
        nacos_adapter_ids_to_drop: nacos_adapter_ids_to_drop.into_iter().collect(),
        mq_adapter_ids_to_drop: mq_adapter_ids_to_drop.into_iter().collect(),
        connection_pool_ids_to_drop: connection_pool_ids_to_drop.into_iter().collect(),
    }
}

async fn drop_mq_adapters_for_connection_ids(_state: &WebState, _connection_ids: &[String]) {}

async fn remove_connection_pools_for_connection_ids(state: &WebState, connection_ids: &[String]) {
    for connection_id in connection_ids {
        state.app.remove_connection_pools_detached(connection_id).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::WebState;
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::Json;
    use dbx_core::connection::{AppState, PoolKind};
    use dbx_core::models::connection::{
        AttachedDatabaseConfig, ConnectionConfig, DatabaseConnectionInfo, DatabaseType, ProxyTunnelConfig, ProxyType,
        TransportLayerConfig,
    };

    use dbx_core::storage::McpGlobalPolicy;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn test_web_state() -> (Arc<WebState>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("dbx-web-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = dbx_core::persistence::test_storage::open(&dir.join("storage.db")).await.unwrap();
        let app = Arc::new(AppState::new_with_plugin_dir(storage, dir.join("plugins")));
        let state = Arc::new(WebState::for_tests(app, dir.clone()));
        (state, dir)
    }

    fn cookie_headers(token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("cookie", format!("dbx_session={token}").parse().unwrap());
        headers
    }
}
