pub mod connection_secrets;

pub mod runtime_config;
pub mod session_credentials;
pub mod task_supervisor;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{watch, Mutex, OwnedSemaphorePermit, RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

use mysql_async::prelude::Queryable;
use mysql_async::Row as MysqlRow;

use crate::database_capabilities;
use crate::db;

use crate::db::http_tunnel::HttpTunnelManager;
use crate::db::proxy_tunnel::ProxyTunnelManager;
use crate::db::ssh_tunnel::TunnelManager;
use crate::models::connection::{
    database_info_from_protocol_value, parse_mongo_first_host, ConnectionConfig, ConnectionLivenessFailureKind,
    ConnectionLivenessMessage, ConnectionTestResult, DatabaseConnectionInfo, DatabaseType, TransportLayerConfig,
};

use crate::path_utils::expand_tilde;
use crate::plugins::{
    PluginConnectionActionResult, PluginConnectionHandle, PluginDriverSession, PluginHost, PluginRegistry,
    PluginRuntimeEnv, PluginRuntimeProxy,
};
use crate::query_cancel::RunningQueries;

use crate::session_credentials::SessionCredentialStore;
use crate::storage::{Storage, DUCKDB_WORKER_MAX_PROCESSES_DEFAULT};
use crate::task_supervisor::TaskSupervisor;

pub const JDBC_PLUGIN_NOT_INSTALLED: &str =
    "JDBC plugin is not installed. Install the optional JDBC plugin to use this connection.";

const DEFAULT_AGENT_CONNECT_TIMEOUT_SECS: u64 = 30;
const ACCESS_AGENT_CONNECT_TIMEOUT_SECS: u64 = 30;
const POOL_CLOSE_TIMEOUT_SECS: u64 = 3;
const HEALTH_CHECK_POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_millis(500);
/// Upper bound for the "is this checked-out connection still alive" query that
/// follows a successful health checkout. Windows keeps retransmitting on a
/// half-open TCP connection for ~21s before the read fails, so a probe without
/// its own budget would make `check_connection_health` (and therefore
/// `ensureConnected`) hang for the whole OS retry window.
const HEALTH_CHECK_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const METADATA_POOL_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const METADATA_POOL_DEFAULT_LIMIT: usize = 6;

mod duckdb_types {

    pub type DuckDbWorkerHandle = ();
}

use duckdb_types::DuckDbWorkerHandle;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MysqlMode {
    Normal,
}

fn mysql_pool_setup_queries(_config: &ConnectionConfig, _url: &str) -> Vec<String> {
    Vec::new()
}

#[derive(Clone)]
pub enum PoolKind {
    Mysql(db::mysql::MySqlPool, MysqlMode),
}
impl PoolKind {
    fn is_available_for_routing(&self) -> bool {
        true
    }
}

#[derive(Clone)]
struct PoolPublication(Arc<()>);

impl PoolPublication {
    fn new() -> Self {
        Self(Arc::new(()))
    }

    fn is_same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone)]
struct PoolPublicationSnapshot {
    pool: PoolKind,
    publication: PoolPublication,
}

#[cfg(test)]
#[derive(Default)]
struct StalePoolCleanupBarriers {
    before_removal: Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>,
    after_removal: Option<(Arc<tokio::sync::Barrier>, Arc<tokio::sync::Barrier>)>,
}

/// Internal pool registry that assigns an opaque identity to every published
/// entry. The identity belongs to the registry publication rather than to the
/// driver handle, so replacing an entry always creates a new generation even
/// when the replacement reuses a cloned handle.
#[doc(hidden)]
pub struct ConnectionPoolRegistry {
    pools: HashMap<String, PoolKind>,
    publications: HashMap<String, PoolPublication>,
}

impl ConnectionPoolRegistry {
    fn new() -> Self {
        Self { pools: HashMap::new(), publications: HashMap::new() }
    }

    pub fn insert(&mut self, pool_key: String, pool: PoolKind) -> Option<PoolKind> {
        self.publications.insert(pool_key.clone(), PoolPublication::new());
        self.pools.insert(pool_key, pool)
    }

    pub fn remove(&mut self, pool_key: &str) -> Option<PoolKind> {
        self.publications.remove(pool_key);
        self.pools.remove(pool_key)
    }

    #[cfg(test)]
    fn clear(&mut self) {
        self.publications.clear();
        self.pools.clear();
    }

    fn drain(&mut self) -> std::collections::hash_map::Drain<'_, String, PoolKind> {
        self.publications.clear();
        self.pools.drain()
    }

    fn snapshot(&self, pool_key: &str) -> Option<PoolPublicationSnapshot> {
        Some(PoolPublicationSnapshot {
            pool: self.pools.get(pool_key)?.clone(),
            publication: self.publications.get(pool_key)?.clone(),
        })
    }

    fn remove_if_publication(&mut self, pool_key: &str, expected: &PoolPublication) -> Option<PoolKind> {
        let is_current = self.publications.get(pool_key).is_some_and(|current| current.is_same(expected));
        is_current.then(|| self.remove(pool_key)).flatten()
    }
}

impl std::ops::Deref for ConnectionPoolRegistry {
    type Target = HashMap<String, PoolKind>;

    fn deref(&self) -> &Self::Target {
        &self.pools
    }
}

enum ConnectionDatabaseInfoSource {
    NativeMysql(db::mysql::MySqlPool),
}

/// Held connection for a manual transaction session
pub enum TxnConnection {
    /// A dedicated MySQL connection. Cancellation may consume and discard this
    /// connection instead of trying to reuse it after an interrupted result set.
    Mysql(Option<mysql_async::Conn>),
}

pub struct TransactionSession {
    pub connection: Arc<Mutex<TxnConnection>>,
    pub pool_key: String,
    pub last_activity: std::time::Instant,
    pub busy: bool,
    pub snapshot_rotation_safe: bool,
    pub connection_id: String,
    pub database: String,
    pub schema: Option<String>,
}

impl TransactionSession {
    pub fn can_rotate_read_only_snapshot(&self, conn: &TxnConnection) -> bool {
        self.snapshot_rotation_safe && matches!(conn, TxnConnection::Mysql(Some(_)))
    }
}

#[derive(Clone)]
pub struct ConnectionLifecycleSnapshot {
    generation: u64,
    cancellation: CancellationToken,
}

impl ConnectionLifecycleSnapshot {
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

struct ConnectionLifecycle {
    generation: u64,
    cancellation: CancellationToken,
}

struct SharedResourceBudget {
    capacity: usize,
    semaphore: Arc<Semaphore>,
}

/// Cached Salesforce connected-user identity + org display name, serialized
/// with camelCase field names for the frontend. `Deserialize` is derived too so
/// the Web-mode MCP backend can decode the same JSON the desktop route emits.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SalesforceCurrentUser {
    pub user_id: String,
    pub name: String,
    pub email: String,
    pub organization_id: String,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_admin: Option<bool>,
    pub org_name: String,
}

pub struct AppState {
    connections: Arc<RwLock<ConnectionPoolRegistry>>,
    task_supervisor: TaskSupervisor,
    pool_activity: Arc<RwLock<HashMap<String, PoolActivity>>>,
    draining_pools: Arc<std::sync::Mutex<HashMap<String, watch::Sender<bool>>>>,
    connection_attempts: RwLock<HashMap<String, ConnectionAttemptState>>,
    connection_lifecycles: std::sync::Mutex<HashMap<String, ConnectionLifecycle>>,
    shared_resource_budgets: std::sync::Mutex<HashMap<String, SharedResourceBudget>>,
    pub configs: RwLock<HashMap<String, ConnectionConfig>>,
    pub running_queries: RunningQueries,
    pub tunnels: TunnelManager,
    pub proxy_tunnels: ProxyTunnelManager,
    pub http_tunnels: HttpTunnelManager,
    pub storage: Storage,
    pub plugins: PluginRegistry,
    pub plugin_host: PluginHost,

    /// Pool keys whose tab-scoped MySQL connection holds a transaction the user
    /// opened explicitly and DBX deliberately kept open
    /// (`preserve_explicit_transaction`). Keeping it here — not on the driver
    /// connection — makes the state die with the pool: a reconnect, a rebuilt
    /// pool, or a closed tab can never inherit a transaction that no longer
    /// exists.
    mysql_preserved_transactions: Arc<RwLock<HashSet<String>>>,
    pub transaction_sessions: Arc<RwLock<HashMap<String, TransactionSession>>>,

    /// `save_password=false` 连接本次运行期的临时密码（内存，进程退出即丢，
    /// 绝不落盘）。键为 `(owner_scope, connection_id)`：桌面端 owner 为空串，
    /// Web 端 owner 为已认证会话 token，不同登录会话互不可见。建池/池重建/
    /// AI/元数据从它读取，前端通过状态接口查询。
    pub session_credentials: SessionCredentialStore,
    /// In-memory, never-persisted 1/5 minute write overrides for read-only connections.
    pub write_unlock_windows: crate::write_unlock::WriteUnlockWindows,
    metadata_gates: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
    /// Backend-confirmed liveness losses plus resync requests (#4339). The keepalive task
    /// publishes here once a connection has no pools left; each shell relays it to its own
    /// frontend, so DBX core never needs a UI handle of its own.
    connection_liveness: tokio::sync::broadcast::Sender<ConnectionLivenessMessage>,
}

fn transport_layers_through_last_ssh(layers: &[TransportLayerConfig]) -> Result<&[TransportLayerConfig], String> {
    let Some(last_ssh_index) = layers.iter().rposition(|layer| matches!(layer, TransportLayerConfig::Ssh(_))) else {
        return Err("Connection has no enabled SSH tunnel layer".to_string());
    };
    Ok(&layers[..=last_ssh_index])
}

/// 活跃时间以进程内单调时钟的相对毫秒存储（AtomicU64）：热路径每条查询都要
/// 更新它，读锁 + 原子写让并发查询不再在全局写锁上串行化。
/// 基准偏移让测试能构造"过去"的时间点（否则进程刚启动时相对毫秒接近 0）。
const POOL_ACTIVITY_BASE_MS: u64 = 86_400_000;

fn pool_activity_epoch() -> Instant {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

fn pool_activity_now_ms() -> u64 {
    POOL_ACTIVITY_BASE_MS + pool_activity_epoch().elapsed().as_millis() as u64
}

#[cfg_attr(not(test), allow(dead_code))]
struct PoolActivity {
    last_used_at_ms: std::sync::atomic::AtomicU64,
}

#[derive(Clone, Copy)]
struct ConnectionAttemptState {
    server_attempt: u64,
    client_attempt: Option<u64>,
}

pub(crate) fn metadata_concurrency_limit(db_type: DatabaseType, max_connections: usize) -> usize {
    {
        max_connections.saturating_sub(2).clamp(1, METADATA_POOL_DEFAULT_LIMIT)
    }
}

pub(crate) fn uses_metadata_gate(db_type: DatabaseType) -> bool {
    matches!(db_type, DatabaseType::Mysql)
}

/// Session-scoped metadata gate allowance for a database type.
///
/// A non-empty `client_session_id` normally means the caller opened a
/// dedicated single-connection session pool (for example MySQL tab sessions),
/// so the gate allows only one in-flight metadata operation. PostgreSQL is the
/// exception: `get_or_create_pool_for_session` builds a fresh full-size
/// (10-connection) pool for each export session, so an allowance of 1 would
/// serialize the database-export metadata prefetch (4 concurrent DDL/column
/// lookups) and make three of them time out after METADATA_POOL_ACQUIRE_TIMEOUT
/// with "DBX metadata pool is busy". Returning the pool's real capacity here
/// flows into metadata_concurrency_limit, which caps the effective gate at
/// METADATA_POOL_DEFAULT_LIMIT (6) -- still enough for the 4-wide prefetch and
/// still isolated from the UI/base pool.
fn metadata_gate_session_allowance(db_type: DatabaseType) -> usize {
    {
        // MySQL and other session-scoped pools are single-connection.
        1
    }
}

fn metadata_gate_key(
    connection_id: &str,
    database: Option<&str>,
    db_type: DatabaseType,
    client_session_id: Option<&str>,
) -> String {
    let session = { client_session_id.map(str::trim).filter(|session| !session.is_empty()).unwrap_or_default() };
    format!("{connection_id}\0{}\0{}", database.unwrap_or_default(), session)
}

struct PoolDrainGuard {
    pool_key: String,
    draining_pools: Arc<std::sync::Mutex<HashMap<String, watch::Sender<bool>>>>,
    signal: watch::Sender<bool>,
}

impl Drop for PoolDrainGuard {
    fn drop(&mut self) {
        self.draining_pools.lock().unwrap_or_else(|error| error.into_inner()).remove(&self.pool_key);
        let _ = self.signal.send(false);
    }
}

impl PoolActivity {
    fn now() -> Self {
        Self { last_used_at_ms: std::sync::atomic::AtomicU64::new(pool_activity_now_ms()) }
    }

    fn touch(&self) {
        // fetch_max 防止"先算后写"交错导致时间戳倒退（A 算得 100 被挂起，
        // B 写入 200 后 A 再写 100）
        self.last_used_at_ms.fetch_max(pool_activity_now_ms(), std::sync::atomic::Ordering::Relaxed);
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn elapsed(&self) -> Duration {
        Duration::from_millis(
            pool_activity_now_ms().saturating_sub(self.last_used_at_ms.load(std::sync::atomic::Ordering::Relaxed)),
        )
    }

    #[cfg(test)]
    fn idle_for(idle: Duration) -> Self {
        Self {
            last_used_at_ms: std::sync::atomic::AtomicU64::new(
                pool_activity_now_ms().saturating_sub(idle.as_millis() as u64),
            ),
        }
    }
}

pub struct PoolActivityTouch {
    pool_key: String,
    connections: Arc<RwLock<ConnectionPoolRegistry>>,
    pool_activity: Arc<RwLock<HashMap<String, PoolActivity>>>,
    task_supervisor: TaskSupervisor,
}

#[derive(Clone)]
struct PoolRoutingControl {
    connections: Arc<RwLock<ConnectionPoolRegistry>>,
    pool_activity: Arc<RwLock<HashMap<String, PoolActivity>>>,

    /// The same set as [`AppState::mysql_preserved_transactions`]. Every detach
    /// path (including `ClientSessionPoolCleanupGuard`'s `Drop`, which never
    /// reaches `AppState`) has to clear the marker together with the pool:
    /// otherwise a pool rebuilt under the same key would read a stale
    /// `already_preserved` and keep a leftover transaction the way #9479
    /// described, even with the opt-in turned off.
    mysql_preserved_transactions: Arc<RwLock<HashSet<String>>>,
    task_supervisor: TaskSupervisor,
}

pub struct ClientSessionPoolCleanupGuard {
    pool_key: String,
    routing: PoolRoutingControl,
    armed: bool,
}

impl Drop for PoolActivityTouch {
    fn drop(&mut self) {
        let pool_key = self.pool_key.clone();
        let connections = self.connections.clone();
        let pool_activity = self.pool_activity.clone();
        self.task_supervisor.spawn_replace(format!("pool-activity:{pool_key}"), move |_| async move {
            if !connections.read().await.contains_key(&pool_key) {
                return;
            }
            if let Some(activity) = pool_activity.read().await.get(&pool_key) {
                activity.touch();
                return;
            }
            pool_activity.write().await.insert(pool_key, PoolActivity::now());
        });
    }
}

impl ClientSessionPoolCleanupGuard {
    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for ClientSessionPoolCleanupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let pool_key = self.pool_key.clone();
        let routing = self.routing.clone();
        routing.stop_keepalive(&pool_key);
        let supervisor = routing.task_supervisor.clone();
        supervisor.spawn_once(format!("client-session-cleanup:{pool_key}"), move |_| async move {
            routing.detach_pool_by_key(&pool_key, false).await;
        });
    }
}

impl PoolRoutingControl {
    fn stop_keepalive(&self, pool_key: &str) {
        self.task_supervisor.stop(&format!("keepalive:{pool_key}"));
    }

    async fn detach_pool_by_key(&self, pool_key: &str, replace_agent_runtime: bool) -> bool {
        let removed = {
            let mut connections = self.connections.write().await;
            let Some(pool) = connections.remove(pool_key) else {
                return false;
            };
            let mut removed = vec![(pool_key.to_string(), pool)];
            if replace_agent_runtime {
                let sibling_keys = shared_runtime_sibling_keys(&connections, &removed[0].1);
                let protects_manual_txn = sibling_keys.iter().any(|key| is_manual_transaction_pool_key(key))
                    || is_manual_transaction_pool_key(pool_key);
                if protects_manual_txn {
                    log::warn!(
                        "Skipping shared Agent runtime kill for '{pool_key}' because a manual-transaction session is active on the same runtime"
                    );
                } else {
                    for key in sibling_keys {
                        if let Some(pool) = connections.remove(&key) {
                            removed.push((key, pool));
                        }
                    }
                }
            }
            removed
        };

        self.finish_detach(removed).await;
        true
    }

    async fn finish_detach(&self, removed: Vec<(String, PoolKind)>) {
        for (key, _) in &removed {
            self.stop_keepalive(key);
        }

        self.close_removed_in_background(removed);
    }

    async fn close_pool_with_timeout(&self, pool_key: String, pool: PoolKind) {
        match tokio::time::timeout(Duration::from_secs(POOL_CLOSE_TIMEOUT_SECS), close_pool_kind(pool)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => log::warn!("Failed to close MySQL pool '{}': {}", pool_key, error),
            Err(_) => log::warn!("Timed out closing MySQL pool '{}'", pool_key),
        }
    }

    async fn close_removed(&self, removed: Vec<(String, PoolKind)>) {
        for (pool_key, pool) in removed {
            self.close_pool_with_timeout(pool_key, pool).await;
        }
    }

    fn close_removed_in_background(&self, removed: Vec<(String, PoolKind)>) {
        if removed.is_empty() {
            return;
        }
        let pool_count = removed.len();
        let routing = self.clone();
        let task_key = format!("pool-close:{}", uuid::Uuid::new_v4());
        if !self.task_supervisor.spawn_once(task_key, move |_| async move {
            for (pool_key, pool) in removed {
                routing.close_pool_with_timeout(pool_key, pool).await;
            }
        }) {
            log::debug!("Dropped {pool_count} detached pool handle(s) during application shutdown");
        }
    }
}

fn shared_runtime_sibling_keys(connections: &HashMap<String, PoolKind>, source_pool: &PoolKind) -> Vec<String> {
    {
        return Vec::new();
    }
}

pub fn metadata_connection_config(config: &ConnectionConfig) -> ConnectionConfig {
    let mut db_config = config.canonicalized();
    if database_capabilities::is_metadata_connection_scoped(&db_config.db_type) {
        db_config.database = None;
    }
    db_config
}

pub fn database_connection_config(config: &ConnectionConfig, database: Option<&str>) -> ConnectionConfig {
    database_connection_config_with_catalog(config, database, None)
}

/// Like [`database_connection_config`], but optionally injects a Doris/StarRocks
/// `catalog=<name>` URL parameter so mysql_async emits `SET catalog` during
/// connection setup (before any `USE <database>`).
pub fn database_connection_config_with_catalog(
    config: &ConnectionConfig,
    database: Option<&str>,
    catalog: Option<&str>,
) -> ConnectionConfig {
    let mut db_config = if database.is_some() { config.clone() } else { metadata_connection_config(config) };
    if let Some(db) = database {
        {
            db_config.database = Some(db.to_string());
        }
    }
    if let Some(catalog) = catalog.map(str::trim).filter(|catalog| !catalog.is_empty()) {
        db_config.url_params = Some(upsert_connection_url_param(db_config.url_params.as_deref(), "catalog", catalog));
    }
    db_config
}

/// Insert or replace a single `key=value` entry in a connection URL-params string.
pub fn upsert_connection_url_param(params: Option<&str>, key: &str, value: &str) -> String {
    let key = key.trim();
    let value = value.trim();
    let key_lower = key.to_ascii_lowercase();
    let encoded_value = percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string();
    let mut parts: Vec<String> = params
        .unwrap_or("")
        .trim()
        .trim_start_matches('?')
        .split('&')
        .filter(|part| !part.trim().is_empty())
        .filter(|part| {
            part.split_once('=')
                .map(|(existing_key, _)| existing_key.trim().to_ascii_lowercase() != key_lower)
                .unwrap_or(true)
        })
        .map(str::to_string)
        .collect();
    parts.push(format!("{key}={encoded_value}"));
    parts.join("&")
}

fn metadata_pool_database<'a>(config: Option<&ConnectionConfig>, database: Option<&'a str>) -> Option<&'a str> {
    database
}

pub async fn connect_mysql_metadata_pool(
    config: &ConnectionConfig,
    db_config: &ConnectionConfig,
    host: &str,
    port: u16,
    connect_timeout: std::time::Duration,
    max_connections: usize,
) -> Result<(db::mysql::MySqlPool, MysqlMode), String> {
    let url = connection_url_for_endpoint(db_config, host, port);
    let idle_timeout_secs = Some(db_config.idle_timeout_secs);
    let extra_setup_queries = mysql_pool_setup_queries(db_config, &url);
    {}

    match db::mysql::connect_with_ca_cert_pool_limit_idle_and_setup(
        &url,
        Some(&db_config.ca_cert_path),
        connect_timeout,
        max_connections,
        idle_timeout_secs,
        &extra_setup_queries,
    )
    .await
    {
        Ok(pool) => {
            let mode = MysqlMode::Normal;
            Ok((pool, mode))
        }
        Err(err) => {
            let fallback_url = mysql_metadata_fallback_url(config, db_config, host, port);
            if let Some(fallback_url) = fallback_url {
                log::info!(
                    "MySQL metadata connection without a default database failed ({err}); retrying with configured default database."
                );
                let pool = db::mysql::connect_with_ca_cert_pool_limit_idle_and_setup(
                    &fallback_url,
                    Some(&config.ca_cert_path),
                    connect_timeout,
                    max_connections,
                    idle_timeout_secs,
                    &extra_setup_queries,
                )
                .await?;
                let mode = MysqlMode::Normal;
                Ok((pool, mode))
            } else if let Some(db) = db_config.effective_database() {
                let mut unscoped_config = db_config.clone();
                unscoped_config.database = None;
                let unscoped_url = connection_url_for_endpoint(&unscoped_config, host, port);
                log::info!("MySQL connection with database in URL failed ({err}); retrying without database in URL and using USE statement.");
                let pool = db::mysql::connect_with_ca_cert_pool_limit_idle_and_setup_database(
                    &unscoped_url,
                    Some(&config.ca_cert_path),
                    connect_timeout,
                    max_connections,
                    idle_timeout_secs,
                    Some(db),
                    &extra_setup_queries,
                )
                .await?;
                let mode = MysqlMode::Normal;
                Ok((pool, mode))
            } else {
                Err(err)
            }
        }
    }
}

pub async fn connect_bare_metadata_pool(
    db_config: &ConnectionConfig,
    host: &str,
    port: u16,
    connect_timeout: std::time::Duration,
    max_connections: usize,
) -> Result<db::mysql::MySqlPool, String> {
    let url = connection_url_for_endpoint(db_config, host, port);
    let extra_setup_queries = mysql_pool_setup_queries(db_config, &url);
    if db_config.effective_database().is_none() {
        return connect_bare_mysql_pool_with_setup(
            db_config,
            &url,
            connect_timeout,
            max_connections,
            &extra_setup_queries,
        )
        .await;
    }

    let mut unscoped_config = db_config.clone();
    unscoped_config.database = None;
    let unscoped_url = connection_url_for_endpoint(&unscoped_config, host, port);
    if unscoped_url == url {
        return connect_bare_mysql_pool_with_setup(
            db_config,
            &url,
            connect_timeout,
            max_connections,
            &extra_setup_queries,
        )
        .await;
    }

    let preferred =
        connect_bare_mysql_pool_with_setup(db_config, &url, connect_timeout, max_connections, &extra_setup_queries);
    let unscoped = connect_bare_mysql_pool_with_setup(
        db_config,
        &unscoped_url,
        connect_timeout,
        max_connections,
        &extra_setup_queries,
    );
    tokio::pin!(preferred);
    tokio::pin!(unscoped);

    tokio::select! {
        result = &mut preferred => match result {
            Ok(pool) => Ok(pool),
            Err(preferred_err) => match (&mut unscoped).await {
                Ok(pool) => Ok(pool),
                Err(unscoped_err) => Err(format!(
                    "Connection with the configured database failed: {preferred_err}\n\nConnection without a default database also failed: {unscoped_err}"
                )),
            },
        },
        result = &mut unscoped => match result {
            Ok(pool) => Ok(pool),
            Err(unscoped_err) => match (&mut preferred).await {
                Ok(pool) => Ok(pool),
                Err(preferred_err) => Err(format!(
                    "Connection with the configured database failed: {preferred_err}\n\nConnection without a default database also failed: {unscoped_err}"
                )),
            },
        },
    }
}

async fn connect_bare_mysql_pool_with_setup(
    db_config: &ConnectionConfig,
    url: &str,
    connect_timeout: std::time::Duration,
    max_connections: usize,
    extra_setup_queries: &[String],
) -> Result<db::mysql::MySqlPool, String> {
    {
        db::mysql::connect_bare_with_pool_limit_and_setup(url, connect_timeout, max_connections, extra_setup_queries)
            .await
    }
}

fn mysql_metadata_fallback_url(
    config: &ConnectionConfig,
    db_config: &ConnectionConfig,
    host: &str,
    port: u16,
) -> Option<String> {
    if false || db_config.effective_database().is_some() {
        return None;
    }
    config.effective_database()?;
    Some(connection_url_for_endpoint(config, host, port))
}

impl AppState {
    pub fn shared_resource_budget(&self, name: &str, capacity: usize) -> Result<Arc<Semaphore>, String> {
        if capacity == 0 {
            return Err("Shared resource budget capacity must be greater than zero".to_string());
        }
        let mut budgets = self.shared_resource_budgets.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(budget) = budgets.get(name) {
            if budget.capacity != capacity {
                return Err(format!(
                    "Shared resource budget {name:?} already has capacity {}, not {capacity}",
                    budget.capacity
                ));
            }
            return Ok(budget.semaphore.clone());
        }
        let semaphore = Arc::new(Semaphore::new(capacity));
        budgets.insert(name.to_string(), SharedResourceBudget { capacity, semaphore: semaphore.clone() });
        Ok(semaphore)
    }

    pub fn connection_lifecycle_snapshot(&self, connection_id: &str) -> ConnectionLifecycleSnapshot {
        let mut lifecycles = self.connection_lifecycles.lock().unwrap_or_else(|error| error.into_inner());
        let lifecycle = lifecycles
            .entry(connection_id.to_string())
            .or_insert_with(|| ConnectionLifecycle { generation: 0, cancellation: CancellationToken::new() });
        ConnectionLifecycleSnapshot { generation: lifecycle.generation, cancellation: lifecycle.cancellation.clone() }
    }

    pub fn connection_lifecycle_is_current(&self, connection_id: &str, snapshot: &ConnectionLifecycleSnapshot) -> bool {
        self.connection_lifecycles.lock().unwrap_or_else(|error| error.into_inner()).get(connection_id).is_some_and(
            |lifecycle| lifecycle.generation == snapshot.generation && !snapshot.cancellation.is_cancelled(),
        )
    }

    pub fn invalidate_connection_lifecycle(&self, connection_id: &str) {
        let previous = {
            let mut lifecycles = self.connection_lifecycles.lock().unwrap_or_else(|error| error.into_inner());
            let lifecycle = lifecycles
                .entry(connection_id.to_string())
                .or_insert_with(|| ConnectionLifecycle { generation: 0, cancellation: CancellationToken::new() });
            let previous = std::mem::replace(&mut lifecycle.cancellation, CancellationToken::new());
            lifecycle.generation = lifecycle.generation.wrapping_add(1);
            previous
        };
        previous.cancel();
    }

    fn invalidate_all_connection_lifecycles(&self) {
        let previous = {
            let mut lifecycles = self.connection_lifecycles.lock().unwrap_or_else(|error| error.into_inner());
            lifecycles
                .values_mut()
                .map(|lifecycle| {
                    lifecycle.generation = lifecycle.generation.wrapping_add(1);
                    std::mem::replace(&mut lifecycle.cancellation, CancellationToken::new())
                })
                .collect::<Vec<_>>()
        };
        for cancellation in previous {
            cancellation.cancel();
        }
    }

    /// Return an owned pool handle. The registry read lock is released before
    /// the caller can perform any asynchronous database operation.
    pub async fn pool_handle(&self, pool_key: &str) -> Option<PoolKind> {
        self.connections.read().await.get(pool_key).cloned()
    }

    async fn pool_publication_snapshot(&self, pool_key: &str) -> Option<PoolPublicationSnapshot> {
        self.connections.read().await.snapshot(pool_key)
    }

    async fn connection_pool_publication_snapshots(&self) -> Vec<(String, PoolPublicationSnapshot)> {
        let connections = self.connections.read().await;
        connections
            .pools
            .keys()
            .filter_map(|pool_key| connections.snapshot(pool_key).map(|snapshot| (pool_key.clone(), snapshot)))
            .collect()
    }

    /// Return an owned snapshot for operations that need to inspect multiple
    /// entries. Cloning handles is cheap and prevents registry guards from
    /// leaking into asynchronous database work.
    pub async fn connection_pools_snapshot(&self) -> HashMap<String, PoolKind> {
        self.connections.read().await.pools.clone()
    }

    /// Inspect the registry while holding its read lock. The callback is
    /// deliberately synchronous so no database I/O can run under the lock.
    pub async fn with_connection_pools<R>(&self, inspect: impl FnOnce(&HashMap<String, PoolKind>) -> R) -> R {
        let connections = self.connections.read().await;
        inspect(&connections.pools)
    }

    /// Whether DBX currently holds a pool for `connection_id`, i.e. the
    /// connection is open right now.
    ///
    /// A saved config proves nothing on its own: a disconnected connection keeps
    /// its config while every one of its pools has been drained. The registry is
    /// the only state that answers "is this connection open", so callers that
    /// must not connect on a user's behalf gate on this instead of on
    /// [`Self::configs`].
    ///
    /// Deliberately a pure registry read: it never calls
    /// `get_or_create_pool`, so checking the state cannot itself open the
    /// connection. Ownership uses the same key convention as
    /// `drain_connection_pools` — the connection id, optionally followed by `:`
    /// and the database/catalog/role/session suffix that `base_pool_key_for` and
    /// its session-scoped variant build.
    pub async fn is_connection_open(&self, connection_id: &str) -> bool {
        self.connections.read().await.keys().any(|key| pool_key_belongs_to_connection(key, connection_id))
    }

    /// Subscribe to backend-confirmed connection liveness messages (#4339).
    ///
    /// Each shell relays these to its own frontend. A fresh receiver only sees messages
    /// published after it subscribed, so shells subscribe once at startup and keep the
    /// receiver alive rather than re-subscribing per request.
    pub fn subscribe_connection_liveness(&self) -> tokio::sync::broadcast::Receiver<ConnectionLivenessMessage> {
        self.connection_liveness.subscribe()
    }

    /// Find an already-registered metadata/workload pool for a metadata read.
    /// Unlike `get_or_create_metadata_pool_for_session`, this is a pure lookup:
    /// it never validates credentials, starts an agent, or opens a transport.
    pub(crate) async fn existing_metadata_pool_key_for_session(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: Option<&str>,
    ) -> Option<String> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id).cloned()
        }?;
        let pool_database = metadata_pool_database(Some(&config), database);
        let mut base_pool_keys =
            vec![base_pool_key_for_config(Some(&config), connection_id, pool_database, None, false)];
        // MongoDB document operations use a connection-level pool and send the
        // requested database in the command. A host connection can therefore
        // legitimately be registered either under the selected database or
        // under the connection-level key; probe both without creating either.
        {}
        let connections = self.connections.read().await;
        base_pool_keys
            .into_iter()
            .flat_map(|base_pool_key| {
                [
                    pool_key_for_session_role(
                        Some(&config),
                        base_pool_key.clone(),
                        client_session_id,
                        PoolSessionRole::Metadata,
                    ),
                    pool_key_for_session_role(
                        Some(&config),
                        base_pool_key,
                        client_session_id,
                        PoolSessionRole::Workload,
                    ),
                ]
            })
            .find(|pool_key| connections.pools.contains_key(pool_key))
    }

    /// Mutate the registry atomically. The callback is deliberately
    /// synchronous; asynchronous cleanup must use values returned from it.
    pub async fn update_connection_pools<R>(&self, update: impl FnOnce(&mut ConnectionPoolRegistry) -> R) -> R {
        let mut connections = self.connections.write().await;
        update(&mut connections)
    }

    fn pool_routing_control(&self) -> PoolRoutingControl {
        PoolRoutingControl {
            connections: self.connections.clone(),
            pool_activity: self.pool_activity.clone(),

            mysql_preserved_transactions: self.mysql_preserved_transactions.clone(),
            task_supervisor: self.task_supervisor.clone(),
        }
    }

    /// save_password=false 连接：持久化/运行态 config 的 password 恒为空，从本次
    /// 运行期会话凭据仓库补主密码，使手动/编辑器/AI/元数据/池重建复用首次输入，
    /// 不再反复弹窗。会话凭据只在内存中，进程退出即丢；"断开并忘记"后仓库为空，
    /// 此处自然回到空密码（需重新输入）。
    ///
    /// owner 作用域取自 [`crate::session_credentials::current_credential_owner`]：
    /// Web 请求由鉴权中间件注入会话 token，桌面端/后台任务为空串。这样不同登录
    /// 会话只会读取各自输入的密码，不会跨会话复用。
    pub fn apply_session_credential(
        &self,
        config: &ConnectionConfig,
        db_config: &mut ConnectionConfig,
        connection_id: &str,
    ) {
        if !config.save_password && db_config.password.is_empty() {
            let owner = crate::session_credentials::current_credential_owner().unwrap_or_default();
            if let Some(session_password) = self.session_credentials.get(&owner, connection_id) {
                db_config.password = session_password;
            }
        }
    }

    /// no-save 连接复用已有池时，若该池由"另一个 owner 会话"创建，则不应复用
    /// （否则会话 B 会直接使用会话 A 输入的临时密码建立的连接）。返回 `true`
    /// 表示需要销毁旧池、以当前 owner 的凭据重建。
    async fn pool_credential_owner_mismatch(&self, config: &ConnectionConfig, pool_key: &str) -> bool {
        if config.save_password {
            return false;
        }
        let owner = crate::session_credentials::current_credential_owner().unwrap_or_default();
        if owner.is_empty() {
            // 桌面端/未注入 owner：单用户场景，按既有逻辑复用。
            return false;
        }
        self.session_credentials.pool_owner_mismatch(pool_key, &owner)
    }

    pub fn new(storage: Storage) -> Self {
        Self::new_with_plugin_dir(storage, default_plugin_dir())
    }

    pub fn new_with_plugin_dir(storage: Storage, plugin_dir: PathBuf) -> Self {
        Self::new_with_plugin_dir_and_app_version(storage, plugin_dir, env!("CARGO_PKG_VERSION"))
    }

    pub fn new_with_plugin_dir_and_app_version(
        storage: Storage,
        plugin_dir: PathBuf,
        app_version: impl Into<String>,
    ) -> Self {
        let app_version = app_version.into();
        let data_dir = storage.data_dir().to_path_buf();
        let plugins = PluginRegistry::new_with_app_version(plugin_dir, app_version.clone());
        let plugin_host = PluginHost::new(plugins.clone());
        Self {
            connections: Arc::new(RwLock::new(ConnectionPoolRegistry::new())),
            task_supervisor: TaskSupervisor::new(),
            pool_activity: Arc::new(RwLock::new(HashMap::new())),
            draining_pools: Arc::new(std::sync::Mutex::new(HashMap::new())),
            connection_attempts: RwLock::new(HashMap::new()),
            connection_lifecycles: std::sync::Mutex::new(HashMap::new()),
            shared_resource_budgets: std::sync::Mutex::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
            running_queries: RunningQueries::default(),
            tunnels: TunnelManager::new(data_dir),
            proxy_tunnels: ProxyTunnelManager::new(),
            http_tunnels: HttpTunnelManager::new(),
            storage,
            plugins,
            plugin_host,

            mysql_preserved_transactions: Arc::new(RwLock::new(HashSet::new())),
            transaction_sessions: Arc::new(RwLock::new(HashMap::new())),

            session_credentials: SessionCredentialStore::new(),
            write_unlock_windows: crate::write_unlock::WriteUnlockWindows::default(),
            metadata_gates: Arc::new(Mutex::new(HashMap::new())),
            // Bounded so a shell that stops draining cannot grow memory without bound. A
            // lagged receiver re-syncs from the read-only status check on its next use, so
            // dropping events here is safe.
            connection_liveness: tokio::sync::broadcast::Sender::new(64),
        }
    }

    pub(crate) async fn acquire_metadata_permit(
        &self,
        connection_id: &str,
        database: Option<&str>,
        db_type: DatabaseType,
        client_session_id: Option<&str>,
    ) -> Result<OwnedSemaphorePermit, String> {
        let key = metadata_gate_key(connection_id, database, db_type, client_session_id);
        let has_session = client_session_id.map(str::trim).is_some_and(|session| !session.is_empty());
        let max_connections = if has_session { metadata_gate_session_allowance(db_type) } else { 10 };
        let limit = metadata_concurrency_limit(db_type, max_connections);
        let gate = {
            let mut gates = self.metadata_gates.lock().await;
            gates.entry(key.clone()).or_insert_with(|| Arc::new(Semaphore::new(limit))).clone()
        };
        let started = Instant::now();
        let queued = gate.available_permits() == 0;
        let permit = match tokio::time::timeout(METADATA_POOL_ACQUIRE_TIMEOUT, gate.acquire_owned()).await {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => return Err(crate::query::METADATA_POOL_BUSY_ERROR.to_string()),
            Err(_) => {
                log::warn!(
                    "[metadata:pool:busy] connection_id={} database={} wait_ms={} limit={}",
                    connection_id,
                    database.unwrap_or_default(),
                    started.elapsed().as_millis(),
                    limit
                );
                return Err(crate::query::METADATA_POOL_BUSY_ERROR.to_string());
            }
        };
        if queued {
            log::debug!(
                "[metadata:pool:acquired] connection_id={} database={} wait_ms={} limit={}",
                connection_id,
                database.unwrap_or_default(),
                started.elapsed().as_millis(),
                limit
            );
        }
        Ok(permit)
    }

    async fn clear_metadata_gates_for_connection(&self, connection_id: &str) {
        let prefix = format!("{connection_id}\0");
        self.metadata_gates.lock().await.retain(|key, _| !key.starts_with(&prefix));
    }

    async fn clear_metadata_gate_for_database(&self, connection_id: &str, database: Option<&str>) {
        let prefix = format!("{connection_id}\0{}\0", database.unwrap_or_default());
        self.metadata_gates.lock().await.retain(|key, _| !key.starts_with(&prefix));
    }

    fn begin_pool_drain(&self, pool_key: &str) -> Option<PoolDrainGuard> {
        let mut draining = self.draining_pools.lock().unwrap_or_else(|error| error.into_inner());
        if draining.contains_key(pool_key) {
            return None;
        }
        let (signal, _) = watch::channel(true);
        draining.insert(pool_key.to_string(), signal.clone());
        Some(PoolDrainGuard { pool_key: pool_key.to_string(), draining_pools: self.draining_pools.clone(), signal })
    }

    async fn wait_for_pool_drain(&self, pool_key: &str) {
        loop {
            let receiver = self
                .draining_pools
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(pool_key)
                .map(watch::Sender::subscribe);
            let Some(mut receiver) = receiver else {
                return;
            };
            if *receiver.borrow_and_update() && receiver.changed().await.is_err() {
                return;
            }
        }
    }

    async fn insert_connection_pool_inner(
        &self,
        pool_key: String,
        pool: PoolKind,
        config: &ConnectionConfig,
        wait_for_drain: bool,
    ) -> Result<(), String> {
        if wait_for_drain {
            self.wait_for_pool_drain(&pool_key).await;
        }

        let routing = self.pool_routing_control();
        let previous = loop {
            let mut connections = self.connections.write().await;
            if !pool.is_available_for_routing() {
                break Err(pool);
            }
            let Ok(mut activity) = self.pool_activity.try_write() else {
                // Idle reclamation reads activity before routing. Never await that lock while
                // holding the routing lock; release and retry to preserve a single lock order.
                drop(connections);
                tokio::task::yield_now().await;
                continue;
            };
            // Abort the old probe while the route cannot change underneath us. A failed
            // candidate never reaches this point, so the existing route keeps its state.
            routing.stop_keepalive(&pool_key);
            activity.insert(pool_key.clone(), PoolActivity::now());
            // Manual-transaction sessions pin a sticky connection/TX. Keepalive
            // detach on these pools was able to tear down the shared agent runtime
            // (and any open TX) when close timed out — skip probes for them.
            let skip_keepalive = pool_key.contains(":session:manual-txn-");
            if !skip_keepalive {
                self.start_keepalive_task(&pool_key, &pool, config);
            }
            break Ok(connections.insert(pool_key.clone(), pool));
        };
        let previous = match previous {
            Ok(previous) => previous,
            Err(pool) => {
                {}
                routing.close_pool_with_timeout(pool_key, pool).await;
                return Err("Agent runtime is unavailable while publishing the connection pool".to_string());
            }
        };
        if let Some(pool) = previous {
            routing.close_pool_with_timeout(pool_key.clone(), pool).await;
        }
        let route_is_available = self
            .with_connection_pools(|connections| {
                connections.get(&pool_key).is_some_and(PoolKind::is_available_for_routing)
            })
            .await;
        if !route_is_available {
            routing.detach_pool_by_key(&pool_key, true).await;
            return Err("Agent runtime is unavailable while publishing the connection pool".to_string());
        }
        // 记录 no-save 池由哪个 owner 会话创建，供多会话复用判定使用
        // （见 pool_credential_owner_mismatch）。已保存密码连接始终共享池，不记录。
        if !config.save_password {
            let owner = crate::session_credentials::current_credential_owner().unwrap_or_default();
            if !owner.is_empty() {
                self.session_credentials.record_pool_owner(&pool_key, &owner);
            }
        }
        Ok(())
    }

    pub async fn insert_connection_pool(
        &self,
        pool_key: String,
        pool: PoolKind,
        config: &ConnectionConfig,
    ) -> Result<(), String> {
        self.insert_connection_pool_inner(pool_key, pool, config, true).await
    }

    pub async fn begin_connection_attempt(&self, connection_id: &str) -> u64 {
        self.begin_connection_attempt_with_client_attempt(connection_id, None).await
    }

    pub async fn begin_connection_attempt_with_client_attempt(
        &self,
        connection_id: &str,
        client_attempt: Option<u64>,
    ) -> u64 {
        let mut attempts = self.connection_attempts.write().await;
        let next = attempts.get(connection_id).map(|state| state.server_attempt).unwrap_or(0).wrapping_add(1);
        attempts.insert(connection_id.to_string(), ConnectionAttemptState { server_attempt: next, client_attempt });
        next
    }

    pub async fn supersede_connection_attempt(&self, connection_id: &str) {
        self.begin_connection_attempt(connection_id).await;
    }

    pub async fn supersede_connection_attempt_if_client_attempt(
        &self,
        connection_id: &str,
        client_attempt: u64,
    ) -> bool {
        let mut attempts = self.connection_attempts.write().await;
        let Some(current) = attempts.get(connection_id).copied() else {
            return false;
        };
        if current.client_attempt != Some(client_attempt) {
            return false;
        }
        attempts.insert(
            connection_id.to_string(),
            ConnectionAttemptState { server_attempt: current.server_attempt.wrapping_add(1), client_attempt: None },
        );
        true
    }

    async fn connection_attempt_is_current(&self, connection_id: &str, attempt: u64) -> bool {
        self.connection_attempts.read().await.get(connection_id).map(|state| state.server_attempt) == Some(attempt)
    }

    pub async fn ensure_current_connection_attempt(
        &self,
        connection_id: &str,
        attempt: Option<u64>,
    ) -> Result<(), String> {
        let Some(attempt) = attempt else {
            return Ok(());
        };
        if self.connection_attempt_is_current(connection_id, attempt).await {
            Ok(())
        } else {
            Err("Connection attempt was superseded by a newer attempt".to_string())
        }
    }

    pub async fn insert_connection_pool_for_attempt(
        &self,
        connection_id: &str,
        attempt: u64,
        pool_key: String,
        pool: PoolKind,
        config: &ConnectionConfig,
    ) -> Result<(), String> {
        if let Err(err) = self.ensure_current_connection_attempt(connection_id, Some(attempt)).await {
            self.pool_routing_control().close_pool_with_timeout(pool_key, pool).await;
            return Err(err);
        }
        self.insert_connection_pool(pool_key, pool, config).await
    }

    async fn discard_stale_connection_attempt_pool(
        &self,
        connection_id: &str,
        pool_key: String,
        pool: PoolKind,
        config: &ConnectionConfig,
    ) {
        self.reset_connection_transport_for_config(connection_id, config).await;
        self.pool_routing_control().close_pool_with_timeout(pool_key, pool).await;
    }

    fn start_keepalive_task(&self, pool_key: &str, pool: &PoolKind, config: &ConnectionConfig) {
        let interval_secs = config.keepalive_interval_secs;
        let mut target = keepalive_target_from_pool(pool, config);

        if interval_secs == 0 {
            return;
        }
        if interval_secs > 0 && target.is_none() {
            log::debug!(
                "Connection keepalive requested for '{pool_key}', but this database driver does not keep a pingable client handle."
            );
            return;
        };

        let key = pool_key.to_string();
        let interval = Duration::from_secs(interval_secs.max(1));
        // MQ keepalive runs a full adapter test_connection (agent RPC); use query timeout
        // so slow clusters are not spuriously dropped by the shorter connect timeout.

        let timeout = Duration::from_secs(config.effective_connect_timeout_secs().max(1));
        let routing = self.pool_routing_control();
        let connections = self.connections.clone();
        let running_queries = self.running_queries.clone();
        let liveness = self.connection_liveness.clone();
        // Liveness is keyed by connection, not by pool: pool keys are per session/role/
        // database derived, so only the connection id identifies "this connection is gone".
        let connection_id = config.id.clone();
        // MQ pool markers are empty; close_pool_kind is a no-op, so keepalive must
        // drop the registry adapter or reconnect would reuse a dead agent.

        self.task_supervisor.spawn_replace(format!("keepalive:{pool_key}"), move |shutdown| async move {
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = tokio::time::sleep(interval) => {}
                }

                if running_queries.is_pool_active(&key) {
                    continue;
                }

                if let Some(target) = target.as_mut() {
                    let result = tokio::time::timeout(timeout, ping_keepalive_target(target, timeout)).await;
                    match result {
                        Ok(Ok(())) => {}
                        Ok(Err(err)) => {
                            if !keepalive_failure_proves_pool_dead(&err.to_string()) {
                                log::debug!(
                                    "Connection keepalive for '{key}' could not check out a connection; keeping the busy pool: {err}"
                                );
                                continue;
                            }
                            log::warn!("Connection keepalive failed for '{key}': {err}; invalidating pool");
                            let replace_runtime =
                                false;
                            // PoolKind::MessageQueue is a unit marker, so matches_pool cannot
                            // Arc::ptr_eq. Identity-check the registry adapter before teardown.


                            // Only a pool we actually detached may report liveness loss; a
                            // probe failure on an already-replaced pool must not.
                            let detached = detach_and_report_keepalive_loss(
                                &routing,
                                &connections,
                                &liveness,
                                &key,
                                target,
                                &connection_id,
                                ConnectionLivenessFailureKind::ProbeFailed,
                                false,
                            )
                            .await;


                            let _ = detached;
                            break;
                        }
                        Err(_) => {
                            log::warn!(
                                "Connection keepalive timed out for '{key}' after {}s; invalidating pool",
                                timeout.as_secs()
                            );


                            // The pool is torn down either way, so a timed-out probe is reported
                            // too: staying silent would keep the sidebar claiming the connection
                            // is usable.
                            let detached = detach_and_report_keepalive_loss(
                                &routing,
                                &connections,
                                &liveness,
                                &key,
                                target,
                                &connection_id,
                                ConnectionLivenessFailureKind::TimedOut,
                                false,
                            )
                            .await;


                            let _ = detached;
                            break;
                        }
                    }
                }
            }
        });
    }

    async fn stop_keepalive_task(&self, pool_key: &str) {
        self.task_supervisor.stop(&format!("keepalive:{pool_key}"));
    }

    async fn stop_keepalive_tasks(&self, pool_keys: &[String]) {
        let keys: Vec<String> = pool_keys.iter().map(|pool_key| format!("keepalive:{pool_key}")).collect();
        self.task_supervisor.stop_many(keys.iter().map(String::as_str));
    }

    pub async fn touch_pool_activity(&self, pool_key: &str) {
        // 热路径：读锁下原子更新；仅条目缺失（首次/已被清理）才退化为写锁插入
        if let Some(activity) = self.pool_activity.read().await.get(pool_key) {
            activity.touch();
            return;
        }
        self.pool_activity.write().await.insert(pool_key.to_string(), PoolActivity::now());
    }

    pub fn pool_activity_touch(&self, pool_key: &str) -> PoolActivityTouch {
        PoolActivityTouch {
            pool_key: pool_key.to_string(),
            connections: self.connections.clone(),
            pool_activity: self.pool_activity.clone(),
            task_supervisor: self.task_supervisor.clone(),
        }
    }

    pub async fn shutdown(&self, deadline: Duration) {
        self.invalidate_all_connection_lifecycles();
        self.running_queries.cancel_all();
        let removed_pools = self.drain_all_connection_pools().await;
        self.transaction_sessions.write().await.clear();

        let shutdown = async {
            let routing = self.pool_routing_control();
            let (_, _, _, _, _, plugin_shutdown) = tokio::join!(
                self.task_supervisor.shutdown(deadline),
                routing.close_removed(removed_pools),
                self.tunnels.stop_all_tunnels(),
                self.proxy_tunnels.stop_all_tunnels(),
                self.http_tunnels.stop_all_tunnels(),
                self.plugin_host.stop_all(),
            );
            if let Err(error) = plugin_shutdown {
                log::warn!("Failed to stop plugin runtimes during shutdown: {error}");
            }
        };
        if tokio::time::timeout(deadline, shutdown).await.is_err() {
            log::warn!("Timed out shutting down DBX runtime resources after {}ms", deadline.as_millis());
        }
    }

    /// Cancels the tasks whose frontend consumer lives in the webview renderer
    /// session that is about to be reloaded after a WebView2 renderer process
    /// failure.
    ///
    /// A renderer reload keeps the application process alive, so only the tasks
    /// bound to the (now-dying) renderer session must be torn down. All of those
    /// (SQL execution, counts/explains and exports) are registered in
    /// [`Self::running_queries`], so this narrow boundary is exactly
    /// [`RunningQueries::cancel_all`]: it signals their cancellation tokens and
    /// fires any registered interrupts, which is what the underlying drivers
    /// (and `query_result_export`) poll to stop work on the database.
    ///
    /// Connection pools, tunnels, transaction sessions and daemons are
    /// *application-scoped* and are reused by the reloaded frontend, so they are
    /// deliberately NOT closed here — closing them is the job of
    /// [`Self::shutdown`] on a full application restart. This keeps a renderer
    /// reload from evicting a pool and tearing down tunnels that the reloaded
    /// page still needs.
    ///
    /// Returns the number of tasks signalled for cancellation (0 when there was
    /// nothing running).
    pub fn cancel_webview_reload_session_tasks(&self) -> usize {
        let cancelled = self.running_queries.cancel_all();
        if cancelled > 0 {
            log::info!("cancelled {cancelled} webview-session query/export tasks before renderer reload");
        }
        cancelled
    }

    #[cfg(test)]
    pub fn supervised_task_count(&self) -> usize {
        self.task_supervisor.active_count()
    }

    pub async fn get_or_create_pool(&self, connection_id: &str, database: Option<&str>) -> Result<String, String> {
        self.get_or_create_pool_for_session(connection_id, database, None).await
    }

    pub async fn get_or_create_pool_with_catalog(
        &self,
        connection_id: &str,
        database: Option<&str>,
        catalog: Option<&str>,
    ) -> Result<String, String> {
        self.get_or_create_pool_for_session_with_catalog(connection_id, database, catalog, None).await
    }

    pub async fn get_or_create_pool_for_connection_attempt(
        &self,
        connection_id: &str,
        database: Option<&str>,
        attempt: u64,
    ) -> Result<String, String> {
        self.get_or_create_pool_for_session_inner(
            connection_id,
            database,
            None,
            None,
            PoolSessionRole::Workload,
            Some(attempt),
        )
        .await
    }

    pub async fn get_or_create_pool_for_session(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: Option<&str>,
    ) -> Result<String, String> {
        self.get_or_create_pool_for_session_with_catalog(connection_id, database, None, client_session_id).await
    }

    pub async fn get_or_create_pool_for_session_with_catalog(
        &self,
        connection_id: &str,
        database: Option<&str>,
        catalog: Option<&str>,
        client_session_id: Option<&str>,
    ) -> Result<String, String> {
        self.get_or_create_pool_for_session_inner(
            connection_id,
            database,
            catalog,
            client_session_id,
            PoolSessionRole::Workload,
            None,
        )
        .await
    }

    pub(crate) async fn get_or_create_metadata_pool_for_session(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: Option<&str>,
    ) -> Result<String, String> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id).cloned()
        };
        let pool_database = metadata_pool_database(config.as_ref(), database);
        self.get_or_create_pool_for_session_inner(
            connection_id,
            pool_database,
            None,
            client_session_id,
            PoolSessionRole::Metadata,
            None,
        )
        .await
    }

    async fn get_or_create_pool_for_session_inner(
        &self,
        connection_id: &str,
        database: Option<&str>,
        catalog: Option<&str>,
        client_session_id: Option<&str>,
        session_role: PoolSessionRole,
        connection_attempt: Option<u64>,
    ) -> Result<String, String> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id).ok_or("Connection config not found")?.clone()
        };
        validate_connection_url_params(&config)?;
        let validate_existing_pool = should_validate_existing_pool_before_reuse(config.db_type);
        let catalog = catalog.map(str::trim).filter(|value| !value.is_empty());

        let base_pool_key = base_pool_key_for_config(Some(&config), connection_id, database, catalog, false);
        let pool_key = pool_key_for_session_role(Some(&config), base_pool_key.clone(), client_session_id, session_role);

        loop {
            self.wait_for_pool_drain(&pool_key).await;
            if self.pool_handle(&pool_key).await.is_some() {
                if self.pool_credential_owner_mismatch(&config, &pool_key).await {
                    // 创建该 no-save 池的是另一个登录会话：销毁旧池，用当前会话的
                    // 凭据重建，避免跨会话复用他人输入的临时密码。
                    self.remove_stale_connection_pool(&pool_key).await;
                    break;
                } else if !validate_existing_pool || !self.remove_stale_connection_pool(&pool_key).await {
                    self.touch_pool_activity(&pool_key).await;
                    return Ok(pool_key);
                }
                break;
            }
            // A reclaim may have removed the pool after the first drain check. Wait
            // for its confirmed close or rollback before deciding to create a new one.
            self.wait_for_pool_drain(&pool_key).await;
            if self.pool_handle(&pool_key).await.is_some() {
                continue;
            }
            break;
        }

        let mut db_config = database_connection_config_with_catalog(&config, database, catalog);
        self.apply_session_credential(&config, &mut db_config, connection_id);

        self.ensure_current_connection_attempt(connection_id, connection_attempt).await?;
        let endpoint = self.connection_endpoint(connection_id, &db_config).await?;
        let (host, port) = (endpoint.host, endpoint.port);
        let runtime_proxy = endpoint.proxy;
        if let Err(err) = self.ensure_current_connection_attempt(connection_id, connection_attempt).await {
            self.reset_connection_transport_for_config(connection_id, &db_config).await;
            return Err(err);
        }

        if true && runtime_proxy.is_none() {
            probe_connection_endpoint(&db_config, &host, port).await?;
        }
        if let Err(err) = self.ensure_current_connection_attempt(connection_id, connection_attempt).await {
            self.reset_connection_transport_for_config(connection_id, &db_config).await;
            return Err(err);
        }
        let url = connection_url_for_endpoint(&db_config, &host, port);
        let connect_timeout = std::time::Duration::from_secs(db_config.effective_connect_timeout_secs());
        let idle_timeout = std::time::Duration::from_secs(db_config.idle_timeout_secs);
        let mysql_pool_max_connections = mysql_pool_max_connections_for_session(client_session_id);
        let pool = match db_config.db_type {
            DatabaseType::Mysql => {
                let (pool, mode) = connect_mysql_metadata_pool(
                    &config,
                    &db_config,
                    &host,
                    port,
                    connect_timeout,
                    mysql_pool_max_connections,
                )
                .await?;
                PoolKind::Mysql(pool, mode)
            }
        };

        if let Err(err) = self.ensure_current_connection_attempt(connection_id, connection_attempt).await {
            self.discard_stale_connection_attempt_pool(connection_id, pool_key.clone(), pool, &db_config).await;
            return Err(err);
        }
        self.insert_connection_pool(pool_key.clone(), pool, &db_config).await?;
        Ok(pool_key)
    }

    /// Returns the enabled transport layers for a connection with tunnel
    /// profile references resolved: a layer carrying a `profile_id` is
    /// replaced by the shared profile from storage (Settings > Tunnels), so
    /// edits to a profile take effect for every connection referencing it.
    /// Fails when a referenced profile no longer exists — connecting without
    /// the intended tunnel would silently bypass it.
    pub async fn resolved_transport_layers(
        &self,
        config: &ConnectionConfig,
    ) -> Result<Vec<TransportLayerConfig>, String> {
        let layers = config.effective_transport_layers();
        if layers.iter().all(|layer| layer.profile_id().is_empty()) {
            return Ok(layers);
        }

        let profiles: HashMap<String, TransportLayerConfig> = self
            .storage
            .load_tunnel_profiles()
            .await?
            .into_iter()
            .map(|profile| (profile.id().to_string(), profile))
            .collect();

        layers
            .into_iter()
            .map(|layer| {
                let profile_id = layer.profile_id();
                if profile_id.is_empty() {
                    return Ok(layer);
                }
                let Some(profile) = profiles.get(profile_id) else {
                    let label = if layer.name().is_empty() { profile_id } else { layer.name() };
                    return Err(format!(
                        "Tunnel profile '{label}' referenced by this connection no longer exists. Re-create it in Settings > Tunnels or edit the connection's tunnel settings."
                    ));
                };
                // Validate the stored reference again at connect time because synced or
                // externally supplied configs may bypass the editor's type constraints.
                if !layer.same_type_as(profile) {
                    return Err(format!(
                        "Tunnel profile '{}' has a different type than the referencing transport layer.",
                        if layer.name().is_empty() { profile_id } else { layer.name() }
                    ));
                }
                Ok(layer.resolved_from_profile(profile))
            })
            .collect()
    }

    /// Tests a shared tunnel profile in isolation (no downstream database), for
    /// the Test button in Settings > Tunnels.
    ///
    /// - SSH: starting an SSH tunnel connects and authenticates eagerly, so a
    ///   successful start verifies host reachability and credentials.
    /// - Proxy (HTTP CONNECT / SOCKS5): performs a standalone handshake test
    ///   against the proxy endpoint to verify reachability and credentials.
    /// - HTTP tunnel: connects lazily (nothing happens until traffic flows), so
    ///   there is nothing to verify here without a target to probe.
    pub async fn test_tunnel_profile(&self, profile: &TransportLayerConfig) -> Result<String, String> {
        match profile {
            TransportLayerConfig::Ssh(ssh) => {
                // A `~/.ssh/config` alias declaring `ProxyJump` resolves to more
                // than one hop; every other alias (the common case) resolves to
                // exactly one, matching `resolve_ssh_tunnel_config`.
                let chain = crate::ssh_config::resolve_ssh_tunnel_chain(ssh);
                let leaf = chain.last().expect("resolve_ssh_tunnel_chain always returns at least one hop");
                if leaf.host.trim().is_empty() {
                    return Err("SSH host is required.".to_string());
                }
                // A throwaway id so the probe never reuses or evicts a live tunnel, and
                // a sentinel forward target: SSH auth completes on connect, before any
                // channel to this target is opened, so it need not be reachable.
                let probe_id = format!("__tunnel_profile_test__:{}", uuid::Uuid::new_v4());
                let result = if chain.len() == 1 {
                    let timeout = if leaf.connect_timeout_secs == 0 {
                        crate::models::connection::default_ssh_connect_timeout_secs()
                    } else {
                        leaf.connect_timeout_secs
                    };
                    self.tunnels
                        .start_tunnel(
                            &probe_id,
                            &leaf.host,
                            leaf.port,
                            &leaf.host,
                            leaf.port,
                            &leaf.user,
                            &leaf.password,
                            &leaf.key_path,
                            &leaf.key_passphrase,
                            leaf.use_ssh_agent,
                            &leaf.ssh_agent_sock_path,
                            &leaf.auth_method,
                            timeout,
                            "127.0.0.1",
                            1,
                            false,
                            leaf.allow_exec_channel_proxy,
                            &leaf.proxy_command,
                        )
                        .await
                        .map(|_| ())
                } else {
                    self.tunnels.start_chain(&probe_id, &chain, &leaf.host, leaf.port).await.map(|_| ())
                };
                self.tunnels.stop_tunnel(&probe_id).await;
                result.map(|_| "SSH tunnel connection successful".to_string())
            }
            TransportLayerConfig::Proxy(proxy) => {
                if proxy.host.trim().is_empty() {
                    return Err("Proxy host is required.".to_string());
                }
                if proxy.port == 0 {
                    return Err("Proxy port is required.".to_string());
                }
                crate::db::proxy_tunnel::test_proxy_endpoint(
                    proxy.proxy_type,
                    &proxy.host,
                    proxy.port,
                    &proxy.username,
                    &proxy.password,
                    proxy.test_target.as_deref(),
                )
                .await
            }
            TransportLayerConfig::HttpTunnel(_) => {
                Err("Tunnel test is not supported for HTTP tunnel profiles.".to_string())
            }
        }
    }

    /// Tests the enabled SSH chain without opening a database connection.
    /// Layers before the final SSH hop are included because they may be
    /// required to reach that hop; layers after it are unrelated to SSH auth.
    pub async fn test_connection_ssh_tunnel(&self, config: &ConnectionConfig) -> Result<String, String> {
        let resolved_layers = self.resolved_transport_layers(config).await?;
        let test_layers = transport_layers_through_last_ssh(&resolved_layers)?;
        let probe_id = format!("__connection_ssh_test__:{}", uuid::Uuid::new_v4());
        let result = db::transport_layer_tunnel::start_transport_layers(
            &probe_id,
            test_layers,
            "127.0.0.1",
            1,
            &self.tunnels,
            &self.proxy_tunnels,
            &self.http_tunnels,
        )
        .await;
        db::transport_layer_tunnel::stop_transport_layers(
            &probe_id,
            test_layers.len(),
            &self.tunnels,
            &self.proxy_tunnels,
            &self.http_tunnels,
        )
        .await;
        result.map(|_| "SSH tunnel connection successful".to_string())
    }

    pub async fn connection_host_port(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> Result<(String, u16), String> {
        let endpoint = self.connection_endpoint(connection_id, config).await?;
        Ok((endpoint.host, endpoint.port))
    }

    /// Resolves the runtime dial endpoint for a plugin connection, including
    /// the host-managed SOCKS5 route when the provider declares
    /// `proxy_route` and transport layers are configured.
    pub async fn plugin_connection_endpoint(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> Result<ConnectionEndpoint, String> {
        self.connection_endpoint(connection_id, config).await
    }

    async fn connection_endpoint(
        &self,
        connection_id: &str,
        config: &ConnectionConfig,
    ) -> Result<ConnectionEndpoint, String> {
        let transport_layers = self.resolved_transport_layers(config).await?;
        if transport_layers.is_empty() {
            return Ok(ConnectionEndpoint::direct(config.host.clone(), config.port));
        }
        {}
        {}
        {}

        // Multi-endpoint plugin providers (Kafka bootstrap + advertised
        // listeners) route every endpoint through a host-managed SOCKS5
        // dialer instead of a static tunnel, which can only reach a single
        // broker. The payload keeps the logical endpoint so the plugin can
        // still resolve its own seed list and metadata names.
        {}

        {}

        let (remote_host, remote_port) = connection_remote_endpoint(config);
        // Plugin providers commonly declare no host/port binding (Kafka keeps
        // its endpoints in provider fields instead), so a static tunnel would
        // silently forward to an empty target and every downstream dial would
        // time out with no actionable hint. Fail here instead.
        {}
        let local_port = db::transport_layer_tunnel::start_transport_layers(
            connection_id,
            &transport_layers,
            &remote_host,
            remote_port,
            &self.tunnels,
            &self.proxy_tunnels,
            &self.http_tunnels,
        )
        .await?;

        Ok(ConnectionEndpoint { host: "127.0.0.1".to_string(), port: local_port, proxy: None })
    }

    pub async fn invoke_plugin_connection_action(
        &self,
        config: ConnectionConfig,
        action_id: &str,
    ) -> Result<PluginConnectionActionResult, String> {
        {
            return Err("Connection is not owned by a plugin".to_string());
        }
    }

    async fn remove_stale_connection_pool(&self, pool_key: &str) -> bool {
        if self.running_queries.is_pool_active(pool_key) {
            return false;
        }

        let Some(checked) = self.pool_publication_snapshot(pool_key).await else {
            return false;
        };
        let stale = {
            match &checked.pool {
                PoolKind::Mysql(pool, _) => {
                    let pool = pool.clone();
                    match db::mysql::checkout_mysql_conn(&pool, HEALTH_CHECK_POOL_ACQUIRE_TIMEOUT).await {
                        // The 500 ms probe budget is intentionally shorter than a foreground checkout. A timeout
                        // while waiting, creating, or recycling is inconclusive: slow remote handshakes and active
                        // metadata exports can legitimately exceed it. Removing the pool here would start competing
                        // reconnects while useful work is still running.
                        Err(err @ db::PoolCheckoutError::Timeout { .. }) => {
                            log::debug!(
                                "MySQL connection pool '{pool_key}' did not finish a health checkout; keeping pool: {err}"
                            );
                            false
                        }
                        Err(err) => {
                            log::warn!("MySQL connection pool '{pool_key}' is stale: {err}");
                            true
                        }
                        Ok(mut conn) => {
                            // The probe runs no statement, so a shared pool can take
                            // the connection back without COM_RESET_CONNECTION and the
                            // setup replay, and a connection verified moments ago (by
                            // this probe or the checkout that follows it) is not
                            // pinged again.
                            conn.reset_connection(false);
                            let timeout = crate::db::connection_timeout();
                            match tokio::time::timeout(timeout, db::mysql::verify_pooled_conn(&pool, &mut conn)).await {
                                Ok(Ok(())) => false,
                                Ok(Err(err)) => {
                                    log::warn!("MySQL connection pool '{pool_key}' is stale: {err}");
                                    true
                                }
                                Err(_) => {
                                    log::warn!("MySQL connection pool '{pool_key}' is stale: health check timed out");
                                    true
                                }
                            }
                        }
                    }
                }
            }
        };

        if !stale {
            return false;
        }

        self.remove_stale_pool_if_current(pool_key, &checked.publication).await
    }
    async fn remove_stale_pool_if_current(&self, pool_key: &str, checked_publication: &PoolPublication) -> bool {
        self.remove_stale_pool_if_current_inner(
            pool_key,
            checked_publication,
            #[cfg(test)]
            None,
        )
        .await
    }

    async fn remove_stale_pool_if_current_inner(
        &self,
        pool_key: &str,
        checked_publication: &PoolPublication,
        #[cfg(test)] cleanup_barriers: Option<StalePoolCleanupBarriers>,
    ) -> bool {
        #[cfg(test)]
        if let Some((cleanup_ready, continue_cleanup)) =
            cleanup_barriers.as_ref().and_then(|barriers| barriers.before_removal.as_ref())
        {
            cleanup_ready.wait().await;
            continue_cleanup.wait().await;
        }

        let routing = self.pool_routing_control();
        let removed = loop {
            let mut connections = self.connections.write().await;
            let is_current =
                connections.publications.get(pool_key).is_some_and(|current| current.is_same(checked_publication));
            if !is_current {
                log::debug!(
                    "Connection pool '{pool_key}' was replaced while its health check was running; keeping the current route"
                );
                return false;
            }
            let Ok(mut activity) = self.pool_activity.try_write() else {
                drop(connections);
                tokio::task::yield_now().await;
                continue;
            };
            let removed = connections
                .remove_if_publication(pool_key, checked_publication)
                .expect("checked pool publication must remain current while routing is locked");
            #[cfg(test)]
            if let Some((route_removed, continue_cleanup)) =
                cleanup_barriers.as_ref().and_then(|barriers| barriers.after_removal.as_ref())
            {
                route_removed.wait().await;
                continue_cleanup.wait().await;
            }
            routing.stop_keepalive(pool_key);
            activity.remove(pool_key);
            break removed;
        };

        routing.close_pool_with_timeout(pool_key.to_string(), removed).await;
        true
    }

    pub async fn reconnect_pool(&self, connection_id: &str, database: Option<&str>) -> Result<String, String> {
        self.reconnect_pool_for_session(connection_id, database, None).await
    }

    pub async fn reconnect_pool_for_session(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: Option<&str>,
    ) -> Result<String, String> {
        self.reconnect_pool_for_session_with_catalog(connection_id, database, None, client_session_id).await
    }

    pub async fn reconnect_pool_for_session_with_catalog(
        &self,
        connection_id: &str,
        database: Option<&str>,
        catalog: Option<&str>,
        client_session_id: Option<&str>,
    ) -> Result<String, String> {
        self.reconnect_pool_for_session_with_catalog_and_role(
            connection_id,
            database,
            catalog,
            client_session_id,
            PoolSessionRole::Workload,
        )
        .await
    }

    pub(crate) async fn reconnect_metadata_pool_for_session(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: Option<&str>,
    ) -> Result<String, String> {
        self.reconnect_pool_for_session_with_catalog_and_role(
            connection_id,
            database,
            None,
            client_session_id,
            PoolSessionRole::Metadata,
        )
        .await
    }

    async fn reconnect_pool_for_session_with_catalog_and_role(
        &self,
        connection_id: &str,
        database: Option<&str>,
        catalog: Option<&str>,
        client_session_id: Option<&str>,
        session_role: PoolSessionRole,
    ) -> Result<String, String> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id).cloned()
        };
        let catalog = catalog.map(str::trim).filter(|value| !value.is_empty());
        let pool_database = if session_role == PoolSessionRole::Metadata {
            metadata_pool_database(config.as_ref(), database)
        } else {
            database
        };
        let base_pool_key = base_pool_key_for_config(config.as_ref(), connection_id, pool_database, catalog, true);
        let pool_key = pool_key_for_session_role(config.as_ref(), base_pool_key, client_session_id, session_role);

        self.get_or_create_pool_for_session_inner(
            connection_id,
            pool_database,
            catalog,
            client_session_id,
            session_role,
            None,
        )
        .await
    }

    pub async fn close_client_session_pool(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: &str,
    ) -> Result<bool, String> {
        let Some((pool_key, pool)) = self
            .take_client_session_pool(connection_id, database, client_session_id, PoolSessionRole::Workload)
            .await?
        else {
            return Ok(false);
        };
        self.pool_routing_control().close_pool_with_timeout(pool_key, pool).await;
        Ok(true)
    }

    pub(crate) async fn close_metadata_session_pool(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: &str,
    ) -> Result<bool, String> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id).cloned()
        };
        let pool_database = metadata_pool_database(config.as_ref(), database);
        let Some((pool_key, pool)) = self
            .take_client_session_pool(connection_id, pool_database, client_session_id, PoolSessionRole::Metadata)
            .await?
        else {
            return Ok(false);
        };
        self.pool_routing_control().close_pool_with_timeout(pool_key, pool).await;
        Ok(true)
    }

    pub(crate) async fn metadata_session_pool_cleanup_guard(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: &str,
    ) -> Option<ClientSessionPoolCleanupGuard> {
        self.client_session_pool_cleanup_guard_for_role(
            connection_id,
            database,
            client_session_id,
            PoolSessionRole::Metadata,
        )
        .await
    }

    pub(crate) async fn workload_session_pool_cleanup_guard(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: &str,
    ) -> Option<ClientSessionPoolCleanupGuard> {
        self.client_session_pool_cleanup_guard_for_role(
            connection_id,
            database,
            client_session_id,
            PoolSessionRole::Workload,
        )
        .await
    }

    async fn client_session_pool_cleanup_guard_for_role(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: &str,
        session_role: PoolSessionRole,
    ) -> Option<ClientSessionPoolCleanupGuard> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id).cloned()
        };
        let pool_database = if session_role == PoolSessionRole::Metadata {
            metadata_pool_database(config.as_ref(), database)
        } else {
            database
        };
        let base_pool_key = base_pool_key_for_config(config.as_ref(), connection_id, pool_database, None, false);
        let pool_key =
            pool_key_for_session_role(config.as_ref(), base_pool_key.clone(), Some(client_session_id), session_role);
        if pool_key == base_pool_key {
            return None;
        }
        Some(ClientSessionPoolCleanupGuard { pool_key, routing: self.pool_routing_control(), armed: true })
    }

    /// Removes a session-scoped pool immediately and schedules the potentially slow driver
    /// shutdown on the supervised background task set.
    pub async fn detach_client_session_pool(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: &str,
    ) -> Result<bool, String> {
        let Some(removed) = self
            .take_client_session_pool(connection_id, database, client_session_id, PoolSessionRole::Workload)
            .await?
        else {
            return Ok(false);
        };
        self.pool_routing_control().close_removed_in_background(vec![removed]);
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) async fn replace_runtime_for_metadata_pool(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: Option<&str>,
    ) -> bool {
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id).cloned()
        };
        let pool_database = metadata_pool_database(config.as_ref(), database);
        let base_pool_key = base_pool_key_for_config(config.as_ref(), connection_id, pool_database, None, false);
        let pool_key =
            pool_key_for_session_role(config.as_ref(), base_pool_key, client_session_id, PoolSessionRole::Metadata);
        self.detach_pool_by_key(&pool_key, true).await
    }

    pub(crate) async fn detach_metadata_pool_after_recovery(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: Option<&str>,
        agent_session_id: Option<&str>,
        replace_agent_runtime: bool,
    ) -> bool {
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id).cloned()
        };
        let pool_database = metadata_pool_database(config.as_ref(), database);
        let base_pool_key = base_pool_key_for_config(config.as_ref(), connection_id, pool_database, None, false);
        let pool_key =
            pool_key_for_session_role(config.as_ref(), base_pool_key, client_session_id, PoolSessionRole::Metadata);
        if let Some(session_id) = agent_session_id {
            {}
        }
        self.detach_pool_by_key(&pool_key, replace_agent_runtime).await
    }

    /// Detaches a pool before cleanup so a stuck Agent close cannot delay replacement.
    pub async fn detach_pool_by_key(&self, pool_key: &str, replace_agent_runtime: bool) -> bool {
        self.pool_routing_control().detach_pool_by_key(pool_key, replace_agent_runtime).await
    }

    async fn take_client_session_pool(
        &self,
        connection_id: &str,
        database: Option<&str>,
        client_session_id: &str,
        session_role: PoolSessionRole,
    ) -> Result<Option<(String, PoolKind)>, String> {
        let session = normalize_client_session_id(Some(client_session_id));
        let Some(session) = session else {
            return Ok(None);
        };
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id).cloned()
        };
        let base_pool_key = base_pool_key_for_config(config.as_ref(), connection_id, database, None, false);
        let pool_key = pool_key_for_session_role(config.as_ref(), base_pool_key.clone(), Some(&session), session_role);
        if pool_key == base_pool_key {
            return Ok(None);
        }
        self.stop_keepalive_task(&pool_key).await;
        self.pool_activity.write().await.remove(&pool_key);

        self.mysql_preserved_transactions.write().await.remove(&pool_key);
        let removed = self.update_connection_pools(|connections| connections.remove(&pool_key)).await;
        Ok(removed.map(|pool| (pool_key, pool)))
    }

    /// Whether `pool_key` keeps a transaction the user opened explicitly open
    /// on purpose (MySQL auto-commit tabs with `preserve_explicit_transaction`).
    pub(crate) async fn has_preserved_explicit_transaction(&self, pool_key: &str) -> bool {
        self.mysql_preserved_transactions.read().await.contains(pool_key)
    }

    pub(crate) async fn mark_preserved_explicit_transaction(&self, pool_key: &str) {
        self.mysql_preserved_transactions.write().await.insert(pool_key.to_string());
    }

    pub(crate) async fn clear_preserved_explicit_transaction(&self, pool_key: &str) {
        self.mysql_preserved_transactions.write().await.remove(pool_key);
    }

    pub async fn remove_pool_by_key(&self, pool_key: &str) -> bool {
        self.stop_keepalive_task(pool_key).await;
        self.pool_activity.write().await.remove(pool_key);

        self.mysql_preserved_transactions.write().await.remove(pool_key);
        let removed = self.connections.write().await.remove(pool_key);
        if let Some(pool) = removed {
            self.pool_routing_control().close_pool_with_timeout(pool_key.to_string(), pool).await;
            true
        } else {
            false
        }
    }

    async fn reclaim_idle_base_pool_for_session(&self, connection_id: &str, preferred_base_pool_key: &str) -> bool {
        let pool_prefix = format!("{connection_id}:");
        let activity = self.pool_activity.read().await;
        let connections = self.connection_pools_snapshot().await;
        let mut candidates: Vec<(String, (usize, u64))> = connections
            .iter()
            .filter_map(|(key, pool)| {
                return None;
            })
            .collect();
        drop(connections);
        drop(activity);
        candidates.sort_by_key(|(_, rank)| *rank);

        for (pool_key, _) in candidates {
            let Some(_drain) = self.begin_pool_drain(&pool_key) else {
                continue;
            };
            {}
        }
        false
    }

    pub async fn close_database_pool(&self, connection_id: &str, database: Option<&str>) -> Result<bool, String> {
        let config = {
            let configs = self.configs.read().await;
            configs.get(connection_id).cloned()
        };
        let db_type = config.as_ref().map(|config| config.db_type);
        let default_database = config.as_ref().and_then(|config| config.effective_database());
        if database.is_some() && db_type.is_some_and(|db_type| shares_database_pool_with_connection(&db_type)) {
            return Ok(false);
        }
        let target_database = { database.map(str::trim).filter(|database| !database.is_empty()) };
        let mut base_pool_keys =
            vec![base_pool_key_for_config(config.as_ref(), connection_id, target_database, None, false)];
        if target_database.is_some() && target_database == default_database {
            let connection_pool_key = base_pool_key_for_config(config.as_ref(), connection_id, None, None, false);
            if !base_pool_keys.contains(&connection_pool_key) {
                base_pool_keys.push(connection_pool_key);
            }
        }
        let session_prefixes: Vec<String> = base_pool_keys.iter().map(|key| format!("{key}:session:")).collect();
        let metadata_role_keys: Vec<String> = base_pool_keys.iter().map(|key| format!("{key}:role:metadata")).collect();
        let keys_to_remove: Vec<String> = self
            .connections
            .read()
            .await
            .keys()
            .filter(|key| {
                base_pool_keys.iter().any(|base_key| *key == base_key)
                    || metadata_role_keys.iter().any(|metadata_key| *key == metadata_key)
                    || session_prefixes.iter().any(|prefix| key.starts_with(prefix))
            })
            .cloned()
            .collect();
        self.stop_keepalive_tasks(&keys_to_remove).await;

        let mut conns = self.connections.write().await;
        let mut removed = Vec::with_capacity(keys_to_remove.len());
        for key in keys_to_remove {
            if let Some(pool) = conns.remove(&key) {
                removed.push((key, pool));
            }
        }
        drop(conns);
        let closed = !removed.is_empty();
        self.clear_metadata_gate_for_database(connection_id, database).await;
        for (key, pool) in removed {
            self.pool_routing_control().close_pool_with_timeout(key, pool).await;
        }
        Ok(closed)
    }

    pub async fn connection_identifier_quote(
        &self,
        connection_id: &str,
        database: Option<&str>,
    ) -> Result<Option<String>, String> {
        let _ = (connection_id, database);
        Ok(Some("`".to_string()))
    }

    pub async fn connection_database_info(
        &self,
        connection_id: &str,
        database: Option<&str>,
    ) -> Result<Option<DatabaseConnectionInfo>, String> {
        let config = self
            .configs
            .read()
            .await
            .get(connection_id)
            .cloned()
            .ok_or_else(|| format!("Connection config not found: {connection_id}"))?;
        let pool_key = self.get_or_create_pool(connection_id, database).await?;
        let source = {
            match self.pool_handle(&pool_key).await.as_ref() {
                Some(PoolKind::Mysql(pool, _)) => Some(ConnectionDatabaseInfoSource::NativeMysql(pool.clone())),

                _ => None,
            }
        };

        match source {
            Some(ConnectionDatabaseInfoSource::NativeMysql(pool)) => {
                db::mysql::database_connection_info(&pool, db::mysql::protocol_product_name(&config)).await.map(Some)
            }

            None => Ok(None),
        }
    }

    /// Persist the database resolved for a legacy empty-database connection and keep the
    /// runtime config in sync so peer pool creations reuse the discovered database.
    pub async fn save_connection_database(&self, connection_id: &str, database: &str) -> Result<(), String> {
        self.storage.save_connection_database(connection_id, database).await?;
        if let Some(config) = self.configs.write().await.get_mut(connection_id) {
            config.database = Some(database.to_string());
        }
        Ok(())
    }

    pub async fn save_connection_database_info(
        &self,
        connection_id: &str,
        database_info: Option<DatabaseConnectionInfo>,
    ) -> Result<(), String> {
        self.storage.save_connection_database_info(connection_id, database_info.clone()).await?;
        if let Some(config) = self.configs.write().await.get_mut(connection_id) {
            config.database_info = database_info;
        }
        Ok(())
    }

    pub async fn reset_connection_transport(&self, connection_id: &str) {
        let layer_count = {
            let configs = self.configs.read().await;
            configs.get(connection_id).map(|config| config.effective_transport_layers().len()).unwrap_or(0)
        };
        self.reset_connection_transport_layers(connection_id, layer_count).await;
    }

    pub async fn reset_connection_transport_for_config(&self, connection_id: &str, config: &ConnectionConfig) {
        let existing_layer_count = {
            let configs = self.configs.read().await;
            configs.get(connection_id).map(|config| config.effective_transport_layers().len()).unwrap_or(0)
        };
        let layer_count = existing_layer_count.max(config.effective_transport_layers().len());
        self.reset_connection_transport_layers(connection_id, layer_count).await;
    }

    async fn reset_connection_transport_layers(&self, connection_id: &str, layer_count: usize) {
        db::transport_layer_tunnel::stop_transport_layers(
            connection_id,
            layer_count,
            &self.tunnels,
            &self.proxy_tunnels,
            &self.http_tunnels,
        )
        .await;
        self.tunnels.stop_tunnel(connection_id).await;
        self.proxy_tunnels.stop_tunnel(connection_id).await;
        self.http_tunnels.stop_tunnel(connection_id).await;
    }

    /// Health-check the base connection pool for a given connection_id.
    /// Returns `Ok(())` if the pool exists and is healthy, `Err` otherwise.
    /// If the pool is unhealthy it is removed from the map so subsequent
    /// `get_or_create_pool` calls will transparently recreate it.
    pub async fn check_connection_health(&self, connection_id: &str) -> Result<(), String> {
        let db_type = {
            let configs = self.configs.read().await;
            configs.get(connection_id).map(|c| c.db_type)
        };
        let pool_key = base_pool_key_for(db_type, connection_id, None, false);

        // Check if pool exists first
        if self.pool_handle(&pool_key).await.is_none() {
            return Err("No active connection pool found".to_string());
        }

        // `remove_stale_connection_pool` returns true if the pool was stale (and removed)
        if self.remove_stale_connection_pool(&pool_key).await {
            return Err("Connection pool is unhealthy".to_string());
        }
        Ok(())
    }

    /// Warm the driver/pool a tab is about to use, off the user's critical path.
    ///
    /// The first statement of a session pays costs that the user perceives as
    /// "the query is still loading" but that never appear in the reported
    /// statement duration: creating the pool, spawning a JDBC/agent driver
    /// session (JVM startup for external drivers such as Oracle), opening
    /// tunnels, and completing TLS/startup handshakes. `get_or_create_pool_*`
    /// performs exactly that work and verifies connectivity before returning, so
    /// calling it while the editor is being opened moves those seconds from the
    /// first Run to a moment where nobody is waiting on the result.
    ///
    /// This is deliberately *not* a health probe: it never tears an existing
    /// pool down. Use `check_connection_health` when the caller needs a verdict.
    pub async fn prewarm_connection_pool(
        &self,
        connection_id: &str,
        database: Option<&str>,
        catalog: Option<&str>,
        client_session_id: Option<&str>,
    ) -> Result<(), String> {
        let pool_key = self
            .get_or_create_pool_for_session_with_catalog(connection_id, database, catalog, client_session_id)
            .await?;
        self.touch_pool_activity(&pool_key).await;
        Ok(())
    }

    pub async fn refresh_connections(&self) {
        // Clone pool handles under a short-lived read lock, then release it
        // before performing I/O-heavy health checks to avoid blocking writers.
        let checks = self.connection_pool_publication_snapshots().await;

        let mut dead_pools = Vec::new();
        let timeout = crate::db::connection_timeout();

        // Check cloned pools (async I/O, no lock held)
        for (key, checked) in &checks {
            let pool = &checked.pool;
            let healthy = match pool {
                PoolKind::Mysql(p, _) => match db::mysql::get_conn_with_health_check(p).await {
                    Ok(_) => true,
                    Err(e) if crate::query::is_pool_saturation_error(&e) => {
                        log::debug!("MySQL connection pool '{key}' is busy; skipping health probe: {e}");
                        true
                    }
                    Err(e) => {
                        log::warn!("MySQL connection pool '{key}' is unhealthy: {e}");
                        false
                    }
                },
            };
            if !healthy && !false {
                dead_pools.push((key.clone(), checked.publication.clone()));
            }
        }

        let mut detached_pool_keys = Vec::new();
        {}

        // Remove dead pools
        if !dead_pools.is_empty() {
            let mut conns = self.connections.write().await;
            let mut removed = Vec::with_capacity(dead_pools.len());
            for (key, publication) in &dead_pools {
                if let Some(pool) = conns.remove_if_publication(key, publication) {
                    removed.push((key.clone(), pool));
                } else {
                    log::debug!("Skipping stale refresh health result for replaced pool '{key}'");
                }
            }
            drop(conns);
            detached_pool_keys.extend(removed.iter().map(|(key, _)| key.clone()));
            self.pool_routing_control().finish_detach(removed).await;
        }

        // Only failed pools may require a fresh transport. Healthy tunnel listeners must keep
        // their local ports stable across app resume and visibility-triggered health checks.
        let tunnel_connection_ids: HashSet<String> = {
            let configs = self.configs.read().await;
            detached_pool_keys
                .iter()
                .filter_map(|pool_key| config_for_pool_key(pool_key, &configs))
                .filter(|config| config.has_effective_transport_layers())
                .map(|config| config.id.clone())
                .collect()
        };
        for connection_id in tunnel_connection_ids {
            self.reset_connection_transport(&connection_id).await;
            // Tunnels will be re-created on next pool access via connection_host_port
        }
    }

    pub async fn remove_connection_pools(&self, connection_id: &str) {
        self.invalidate_connection_lifecycle(connection_id);
        self.rollback_manual_transaction_sessions(connection_id).await;
        let removed = self.drain_connection_pools(connection_id).await;
        self.clear_metadata_gates_for_connection(connection_id).await;
        self.pool_routing_control().close_removed(removed).await;
    }

    /// Drop a connection's pools WITHOUT closing them. Only for plugin
    /// connections on the connect path: the plugin's connection/connect is an
    /// idempotent upsert, so a re-push/reconnect just replaces the pool entry.
    /// Closing would send connection/disconnect to the sidecar, whose
    /// semantics are "drop the registry entry AND kill every session of this
    /// connection" — murdering the live terminals of sibling tabs. Explicit
    /// user disconnect still goes through remove_connection_pools* and does
    /// send connection/disconnect.
    pub async fn drop_connection_pools_without_close(&self, connection_id: &str) {
        self.invalidate_connection_lifecycle(connection_id);
        self.rollback_manual_transaction_sessions(connection_id).await;
        let removed = self.drain_connection_pools(connection_id).await;
        self.clear_metadata_gates_for_connection(connection_id).await;
        drop(removed);
    }

    pub async fn remove_connection_pools_detached(&self, connection_id: &str) {
        self.invalidate_connection_lifecycle(connection_id);
        self.rollback_manual_transaction_sessions(connection_id).await;
        let removed = self.drain_connection_pools(connection_id).await;
        self.clear_metadata_gates_for_connection(connection_id).await;
        self.pool_routing_control().close_removed_in_background(removed);
    }

    /// Close and roll back every manual-transaction session of a connection
    /// before its pools are drained. The transaction sessions hold dedicated
    /// connections outside the pools being removed; without this the session
    /// map survives a user disconnect with open transactions, and a later
    /// reconnect/execution could observe or reuse the stale snapshot state.
    /// Errors are logged and the session is dropped regardless: pool removal
    /// is already the caller's decision.
    async fn rollback_manual_transaction_sessions(&self, connection_id: &str) {
        let sessions: Vec<(String, TransactionSession)> = {
            let mut map = self.transaction_sessions.write().await;
            let keys: Vec<String> = map
                .iter()
                .filter(|(_, session)| session.connection_id == connection_id)
                .map(|(id, _)| id.clone())
                .collect();
            keys.into_iter().filter_map(|id| map.remove(&id).map(|session| (id, session))).collect()
        };
        for (session_id, session) in sessions {
            let mut conn = session.connection.lock().await;
            let outcome = crate::query::rollback_manual_txn_connection(&mut conn).await;
            if let Err(error) = outcome {
                log::warn!("[connection:manual-txn:rollback-on-disconnect] session={} error={}", session_id, error);
            }
        }
    }

    async fn drain_all_connection_pools(&self) -> Vec<(String, PoolKind)> {
        let pool_keys = self.connection_pools_snapshot().await.keys().cloned().collect::<Vec<_>>();
        self.stop_keepalive_tasks(&pool_keys).await;
        self.pool_activity.write().await.clear();
        self.session_credentials.clear_pool_owners();

        self.draining_pools.lock().unwrap_or_else(|error| error.into_inner()).clear();
        self.metadata_gates.lock().await.clear();
        self.connections.write().await.drain().collect()
    }

    pub async fn remove_plugin_connection_pools(&self, plugin_id: &str) {
        let removed = self.drain_plugin_connection_pools(plugin_id).await;
        self.pool_routing_control().close_removed(removed).await;
    }

    async fn drain_connection_pools(&self, connection_id: &str) -> Vec<(String, PoolKind)> {
        let pool_prefix = format!("{connection_id}:");
        let keys_to_remove: Vec<String> = self
            .connections
            .read()
            .await
            .keys()
            .filter(|k| *k == connection_id || k.starts_with(&pool_prefix))
            .cloned()
            .collect();
        self.stop_keepalive_tasks(&keys_to_remove).await;

        self.session_credentials.remove_pool_owners(&keys_to_remove);
        let mut conns = self.connections.write().await;
        let mut removed = Vec::with_capacity(keys_to_remove.len());
        for key in keys_to_remove {
            if let Some(pool) = conns.remove(&key) {
                removed.push((key, pool));
            }
        }
        drop(conns);
        removed
    }

    async fn drain_plugin_connection_pools(&self, plugin_id: &str) -> Vec<(String, PoolKind)> {
        let keys_to_remove: Vec<String> = self
            .connections
            .read()
            .await
            .iter()
            .filter_map(|(key, pool)| match pool {
                _ => None,
            })
            .collect();
        self.stop_keepalive_tasks(&keys_to_remove).await;
        {
            let mut activity = self.pool_activity.write().await;
            for key in &keys_to_remove {
                activity.remove(key);
            }
        }
        let mut conns = self.connections.write().await;
        let mut removed = Vec::with_capacity(keys_to_remove.len());
        for key in keys_to_remove {
            if let Some(pool) = conns.remove(&key) {
                removed.push((key, pool));
            }
        }
        removed
    }
}

enum KeepaliveTarget {
    Mysql(db::mysql::MySqlPool),
}

#[derive(Debug)]
enum KeepaliveError {
    Legacy(String),
}

impl KeepaliveError {}

impl std::fmt::Display for KeepaliveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Legacy(error) => formatter.write_str(error),
        }
    }
}

impl From<String> for KeepaliveError {
    fn from(error: String) -> Self {
        Self::Legacy(error)
    }
}

/// Whether a failed keepalive probe is evidence that the pool is dead.
///
/// A probe that ran out of its checkout budget because every connection is in use reports
/// pool saturation, which says nothing about the health of the pooled connections. That is
/// the normal state of a session-scoped pool (a single connection) while a batch import or
/// transfer holds a long transaction on it. Invalidating the pool there used to abort the
/// running operation with "Connection not found for transaction" instead of letting it
/// finish. Every other probe failure keeps the invalidate-and-reconnect behaviour.
fn keepalive_failure_proves_pool_dead(error: &str) -> bool {
    !crate::query::is_pool_saturation_error(error)
}

/// Tells the shells that `connection_id` has no pools left, so the frontend can stop
/// claiming the connection is connected (#4339).
///
/// Pool keys are per session/role/database derived, so one dying derived pool says nothing
/// about the connection root: only the last pool to go makes the connection really unusable.
/// Gating on "no pools left" is also what collapses N per-pool keepalive failures into one
/// logical disconnect. Two pools of the same connection can both finish detaching before
/// either evaluates this, publishing twice; the frontend path is idempotent.
async fn report_connection_liveness_lost(
    connections: &Arc<RwLock<ConnectionPoolRegistry>>,
    liveness: &tokio::sync::broadcast::Sender<ConnectionLivenessMessage>,
    connection_id: &str,
    failure_kind: ConnectionLivenessFailureKind,
) {
    if connections.read().await.keys().any(|key| pool_key_belongs_to_connection(key, connection_id)) {
        return;
    }
    // A missing subscriber (headless shell) is not an error worth reporting.
    let _ = liveness.send(ConnectionLivenessMessage::Lost { connection_id: connection_id.to_string(), failure_kind });
}

/// Detaches a probe's pool when it is still the current one and reports the loss if the
/// connection has nothing left (#4339).
///
/// Returns whether this probe's pool was still current and was detached. A stale probe (its
/// pool was already replaced) reports nothing: its result says nothing about the connection.
async fn detach_and_report_keepalive_loss(
    routing: &PoolRoutingControl,
    connections: &Arc<RwLock<ConnectionPoolRegistry>>,
    liveness: &tokio::sync::broadcast::Sender<ConnectionLivenessMessage>,
    pool_key: &str,
    target: &KeepaliveTarget,
    connection_id: &str,
    failure_kind: ConnectionLivenessFailureKind,
    replace_agent_runtime: bool,
) -> bool {
    if !detach_keepalive_target_if_current(routing, connections, pool_key, target, replace_agent_runtime).await {
        log::debug!("Skipping stale keepalive {failure_kind:?} result for replaced pool '{pool_key}'");
        return false;
    }
    report_connection_liveness_lost(connections, liveness, connection_id, failure_kind).await;
    true
}

impl KeepaliveTarget {
    fn matches_pool(&self, pool: &PoolKind) -> bool {
        match (self, pool) {
            _ => true,
        }
    }
}

async fn remove_keepalive_pool_if_current(
    connections: &Arc<RwLock<ConnectionPoolRegistry>>,
    pool_key: &str,
    target: &KeepaliveTarget,
) -> Option<PoolKind> {
    let mut pools = connections.write().await;
    if pools.get(pool_key).is_some_and(|pool| target.matches_pool(pool)) {
        pools.remove(pool_key)
    } else {
        None
    }
}

async fn detach_keepalive_target_if_current(
    routing: &PoolRoutingControl,
    connections: &Arc<RwLock<ConnectionPoolRegistry>>,
    pool_key: &str,
    target: &KeepaliveTarget,
    replace_agent_runtime: bool,
) -> bool {
    {}
    let Some(pool) = remove_keepalive_pool_if_current(connections, pool_key, target).await else {
        return false;
    };
    routing.finish_detach(vec![(pool_key.to_string(), pool)]).await;
    true
}

fn keepalive_target_from_pool(pool: &PoolKind, config: &ConnectionConfig) -> Option<KeepaliveTarget> {
    match pool {
        PoolKind::Mysql(pool, _) => Some(KeepaliveTarget::Mysql(pool.clone())),

        _ => None,
    }
}

async fn ping_keepalive_target(target: &mut KeepaliveTarget, timeout: Duration) -> Result<(), KeepaliveError> {
    match target {
        KeepaliveTarget::Mysql(pool) => {
            // The checkout health check is the keepalive round trip: it pings
            // the idle connection (unless it was verified moments ago) and
            // replaces it when it died. A ping leaves no session state behind,
            // so return the connection without the COM_RESET_CONNECTION and
            // setup replay a shared pool would run.
            let mut conn = db::mysql::get_conn_with_health_check(pool).await?;
            conn.reset_connection(false);
            Ok(())
        }
    }
}

fn is_agent_validate_connection_unsupported(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("validate_connection") && (lower.contains("unknown method") || lower.contains("method not found"))
}

/// Runtime dial endpoint handed to a plugin lifecycle call: the logical
/// `host:port` plus an optional host-managed SOCKS5 route for providers
/// declaring `proxy_route`. When `proxy` is set the plugin is
/// expected to dial every endpoint (its seed list and metadata names) through
/// the route, keeping the logical endpoint only for metadata discovery.
#[derive(Debug, Clone)]
pub struct ConnectionEndpoint {
    pub host: String,
    pub port: u16,
    pub proxy: Option<PluginRuntimeProxy>,
}

impl ConnectionEndpoint {
    fn direct(host: String, port: u16) -> Self {
        Self { host, port, proxy: None }
    }
}

fn connection_remote_endpoint(config: &ConnectionConfig) -> (String, u16) {
    {
        (config.host.clone(), config.port)
    }
}

fn normalize_client_session_id(client_session_id: Option<&str>) -> Option<String> {
    client_session_id.map(str::trim).filter(|session| !session.is_empty()).map(|session| session.replace(':', "_"))
}

pub fn task_client_session_id(task_kind: &str, task_id: &str) -> String {
    format!("{task_kind}:{task_id}")
}

fn mysql_pool_max_connections_for_session(client_session_id: Option<&str>) -> usize {
    if normalize_client_session_id(client_session_id).is_some() {
        1
    } else {
        10
    }
}

fn session_scoped_pool_key(base_pool_key: String, client_session_id: Option<&str>) -> String {
    normalize_client_session_id(client_session_id)
        .map(|session| format!("{base_pool_key}:session:{session}"))
        .unwrap_or(base_pool_key)
}

fn is_manual_transaction_pool_key(pool_key: &str) -> bool {
    pool_key.contains(":session:manual-txn-")
}

/// Whether `pool_key` names a pool owned by `connection_id`.
///
/// Every pool key starts with its connection id and appends `:` plus the
/// database, catalog, role, or session suffix, so the separator is what keeps
/// `conn` from matching a `conn-2` pool. Used by
/// [`AppState::is_connection_open`] and [`config_for_pool_key`];
/// `drain_connection_pools` filters on the same convention.
fn pool_key_belongs_to_connection(pool_key: &str, connection_id: &str) -> bool {
    pool_key.strip_prefix(connection_id).is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
}

pub(crate) fn config_for_pool_key<'a>(
    pool_key: &str,
    configs: &'a HashMap<String, ConnectionConfig>,
) -> Option<&'a ConnectionConfig> {
    configs
        .iter()
        .filter(|(connection_id, _)| pool_key_belongs_to_connection(pool_key, connection_id))
        .max_by_key(|(connection_id, _)| connection_id.len())
        .map(|(_, config)| config)
}

fn session_scoped_pool_key_for(
    config: Option<&ConnectionConfig>,
    base_pool_key: String,
    client_session_id: Option<&str>,
) -> String {
    let shares_base_pool = false;
    {}
    session_scoped_pool_key(base_pool_key, client_session_id)
}

/// 两个运行态连接配置是否视为同一连接（用于决定是否销毁连接池）。
///
/// 始终忽略只影响前端导航的表加载策略。仅当双方都是 `save_password=false` 时才忽略
/// `password` 字段的差异：这类连接在 connect 时运行态配置可能携带会话密码，持久化
/// 同步后为空，这种空值差异不应触发池重建。若任一方 `save_password=true`，密码是
/// 真实的连接参数，任何密码变更（包括用户保存了新密码）都必须销毁旧池，否则旧池会
/// 继续用旧密码认证。
pub fn connection_configs_pool_equivalent(a: &ConnectionConfig, b: &ConnectionConfig) -> bool {
    if a == b {
        return true;
    }
    let mut a = a.clone();
    let mut b = b.clone();
    // Sidebar paging is a presentation preference and cannot change an
    // established database session.
    a.sidebar_auto_load_all_tables = false;
    b.sidebar_auto_load_all_tables = false;
    // A default browser filter does not change the established Redis session.
    a.redis_key_filter = None;
    b.redis_key_filter = None;
    if !a.save_password && !b.save_password {
        a.password.clear();
        b.password.clear();
    }
    a == b
}

/// Whether transient credentials can safely survive a persisted config update.
///
/// This comparison is intentionally conservative: only presentation, local
/// visibility, and runtime-policy fields are ignored. Endpoint, account,
/// transport, TLS, driver, and unclassified external-config changes still
/// invalidate credentials. Nacos managed namespaces are a local discovery
/// scope and therefore do not change which account a transient password
/// belongs to.
pub fn connection_configs_session_credentials_compatible(a: &ConnectionConfig, b: &ConnectionConfig) -> bool {
    fn normalize(mut config: ConnectionConfig) -> ConnectionConfig {
        config.name.clear();
        config.note.clear();
        config.driver_label = None;
        config.default_schema = None;
        config.visible_databases = None;
        config.visible_schemas = None;
        config.show_system_schemas = false;
        config.sidebar_auto_load_all_tables = false;
        config.color = None;
        config.docs_notes_path = None;
        config.connect_timeout_secs = 0;
        config.query_timeout_secs = 0;
        config.idle_timeout_secs = 0;
        config.keepalive_interval_secs = 0;
        config.redis_key_filter = None;
        config.redis_key_separator.clear();
        config.redis_scan_page_size = None;
        config.redis_database_aliases.clear();
        config.one_time = false;
        config.read_only = false;
        config.is_production = false;
        config.production_databases.clear();
        config.database_info = None;

        {}
        if !config.save_password {
            config.password.clear();
        }
        config
    }

    normalize(a.clone()) == normalize(b.clone())
}

fn pool_key_for_session_role(
    config: Option<&ConnectionConfig>,
    base_pool_key: String,
    client_session_id: Option<&str>,
    session_role: PoolSessionRole,
) -> String {
    let pool_key = session_scoped_pool_key_for(config, base_pool_key, client_session_id);
    {
        pool_key
    }
}

async fn close_pool_kind(pool: PoolKind) -> Result<(), String> {
    match pool {
        PoolKind::Mysql(p, _) => {
            let _ = p.disconnect().await;
        }
    }
    Ok(())
}

fn base_pool_key_for_config(
    config: Option<&ConnectionConfig>,
    connection_id: &str,
    database: Option<&str>,
    catalog: Option<&str>,
    include_elasticsearch_single_pool: bool,
) -> String {
    {}
    base_pool_key_for_with_catalog(
        config.map(|config| config.db_type),
        connection_id,
        database,
        catalog,
        include_elasticsearch_single_pool,
    )
}

fn base_pool_key_for(
    db_type: Option<DatabaseType>,
    connection_id: &str,
    database: Option<&str>,
    include_elasticsearch_single_pool: bool,
) -> String {
    base_pool_key_for_with_catalog(db_type, connection_id, database, None, include_elasticsearch_single_pool)
}

fn base_pool_key_for_with_catalog(
    db_type: Option<DatabaseType>,
    connection_id: &str,
    database: Option<&str>,
    catalog: Option<&str>,
    include_elasticsearch_single_pool: bool,
) -> String {
    let is_single_connection_pool = db_type.as_ref().is_some_and(|db_type| {
        let is_single = database_capabilities::is_single_connection_pool(db_type) || (false);
        is_single && (true)
    });

    let key = if is_single_connection_pool {
        connection_id.to_string()
    } else {
        match database.filter(|db| !db.trim().is_empty()) {
            Some(db) => format!("{connection_id}:{db}"),
            None => connection_id.to_string(),
        }
    };
    match catalog.map(str::trim).filter(|value| !value.is_empty()) {
        Some(catalog) => format!("{key}:catalog:{catalog}"),
        None => key,
    }
}

fn is_connection_slot_exhausted_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("remaining connection slots are reserved")
        || lower.contains("too many connections")
        || lower.contains("maximum number of connections exceeded")
        || lower.contains("max client connections reached")
        || lower.contains("ora-00018")
}

fn shares_database_pool_with_connection(db_type: &DatabaseType) -> bool {
    false
}

fn should_validate_existing_pool_before_reuse(db_type: DatabaseType) -> bool {
    // PostgreSQL and Agent-backed databases validate connections when they are
    // checked out for actual work. An eager probe here would add a database
    // round-trip before every request and can compete with active Agent leases.
    true
}

#[cfg(test)]
fn uses_bare_mysql_pool(db_type: &DatabaseType) -> bool {
    false
}

fn default_plugin_dir() -> PathBuf {
    default_dbx_dir().join("plugins")
}

fn default_dbx_dir() -> PathBuf {
    let home = std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".dbx")
}

pub fn connection_url_for_endpoint(config: &ConnectionConfig, host: &str, port: u16) -> String {
    if host == config.host && port == config.port {
        config.connection_url()
    } else {
        config.connection_url_with_host(host, port)
    }
}

pub fn redacted_connection_url_for_endpoint(config: &ConnectionConfig, host: &str, port: u16) -> String {
    if host == config.host && port == config.port {
        config.redacted_connection_url()
    } else {
        config.redacted_connection_url_with_host(host, port)
    }
}

fn validate_connection_url_params(config: &ConnectionConfig) -> Result<(), String> {
    config.validate_native_url_params()
}

pub async fn probe_connection_endpoint(config: &ConnectionConfig, host: &str, port: u16) -> Result<(), String> {
    if !uses_tcp_probe(config, host, port) {
        return Ok(());
    }
    let timeout = std::time::Duration::from_secs(config.effective_connect_timeout_secs());

    let entries = connection_probe_endpoints(host, port);

    if entries.is_empty() {
        return Err("no host entries to probe".to_string());
    }

    // Probe each node sequentially; return success on the first reachable node.
    // This matches the failover semantics of the real connection path.
    let mut last_error = String::new();
    for (entry_host, entry_port) in &entries {
        match db::probe_tcp_endpoint(&format!("{:?}", config.db_type), entry_host, *entry_port, timeout).await {
            Ok(()) => return Ok(()),
            Err(e) => last_error = e,
        }
    }
    Err(last_error)
}

fn connection_probe_endpoints(host: &str, default_port: u16) -> Vec<(String, u16)> {
    host.split(',').filter_map(|part| parse_connection_probe_endpoint(part.trim(), default_port)).collect()
}

fn parse_connection_probe_endpoint(endpoint: &str, default_port: u16) -> Option<(String, u16)> {
    if endpoint.is_empty() {
        return None;
    }
    if let Some(rest) = endpoint.strip_prefix('[') {
        let close = rest.find(']')?;
        let host = rest[..close].to_string();
        let port = rest
            .get(close + 1..)
            .and_then(|suffix| suffix.strip_prefix(':'))
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(default_port);
        return Some((host, port));
    }
    if endpoint.matches(':').count() == 1 {
        if let Some((host, raw_port)) = endpoint.rsplit_once(':') {
            if let Ok(port) = raw_port.parse::<u16>() {
                return Some((host.to_string(), port));
            }
        }
    }
    Some((endpoint.to_string(), default_port))
}

fn uses_tcp_probe(config: &ConnectionConfig, host: &str, port: u16) -> bool {
    {}
    if database_capabilities::skips_tcp_probe(&config.db_type) {
        return false;
    }
    if is_original_endpoint(config, host, port) {
        return false;
    }
    true
}

fn is_original_endpoint(config: &ConnectionConfig, host: &str, port: u16) -> bool {
    host == config.host && port == config.port
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::database_capabilities;
    use crate::db;
    use crate::models::connection::{
        default_connect_timeout_secs, default_redis_key_separator, AttachedDatabaseConfig, ConnectionConfig,
        ConnectionLivenessFailureKind, ConnectionLivenessMessage, DatabaseType, HttpTunnelConfig, ProxyTunnelConfig,
        ProxyType, SshTunnelConfig, TransportLayerConfig,
    };
    use crate::plugins::PluginRuntimeProxy;
    use crate::query;
    use crate::schema;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    pub(super) fn mysql_config(database: Option<&str>) -> ConnectionConfig {
        ConnectionConfig {
            oracle_oci_nls_lang: None,
            oracle_oci_tns_admin: None,
            docs_notes_path: None,
            id: "conn".to_string(),
            name: "MySQL".to_string(),
            note: String::new(),
            db_type: DatabaseType::Mysql,
            driver_profile: None,
            driver_label: None,
            url_params: None,
            agent_java_options: Vec::new(),
            host: "127.0.0.1".to_string(),
            port: 3306,
            username: "root".to_string(),
            password: "secret".to_string(),
            database: database.map(str::to_string),
            default_schema: None,
            visible_databases: None,
            visible_database_patterns: None,
            visible_schemas: None,
            show_system_schemas: false,
            sidebar_auto_load_all_tables: false,
            attached_databases: Vec::new(),
            init_script: None,
            color: None,
            transport_layers: Vec::new(),
            connect_timeout_secs: default_connect_timeout_secs(),
            query_timeout_secs: crate::models::connection::default_query_timeout_secs(),
            idle_timeout_secs: crate::models::connection::default_idle_timeout_secs(),
            keepalive_interval_secs: crate::models::connection::default_keepalive_interval_secs(),
            ssl: false,
            ca_cert_path: String::new(),
            client_cert_path: String::new(),
            client_key_path: String::new(),
            sysdba: false,
            oracle_connection_type: None,
            connection_string: None,
            redis_connection_mode: None,
            redis_sentinel_master: String::new(),
            redis_sentinel_nodes: String::new(),
            redis_sentinel_username: String::new(),
            redis_sentinel_password: String::new(),
            redis_sentinel_tls: false,
            redis_cluster_nodes: String::new(),
            redis_key_separator: default_redis_key_separator(),
            redis_scan_page_size: None,
            redis_database_aliases: Default::default(),
            redis_key_templates: Vec::new(),
            redis_key_filter: None,
            redis_key_grouping: None,
            etcd_endpoints: String::new(),
            gbase_server: String::new(),
            informix_server: String::new(),
            external_config: None,
            plugin_id: None,
            plugin_connection_provider: None,
            plugin_connection_type: None,
            connection_secrets: Default::default(),
            jdbc_driver_class: None,
            jdbc_driver_paths: Vec::new(),
            one_time: false,
            save_password: true,
            read_only: false,
            is_production: false,
            production_databases: vec![],
            database_info: None,
        }
    }

    #[test]
    fn upsert_connection_url_param_inserts_and_replaces_catalog() {
        assert_eq!(upsert_connection_url_param(None, "catalog", "paimon"), "catalog=paimon");
        assert_eq!(
            upsert_connection_url_param(Some("charset=utf8mb4"), "catalog", "hive_catalog"),
            "charset=utf8mb4&catalog=hive%5Fcatalog"
        );
        assert_eq!(
            upsert_connection_url_param(Some("catalog=old&charset=utf8mb4"), "catalog", "new_cat"),
            "charset=utf8mb4&catalog=new%5Fcat"
        );
    }

    #[test]
    fn connection_configs_pool_equivalent_ignores_password_differences() {
        let mut a = mysql_config(None);
        a.save_password = false;
        a.password = "session-secret".to_string();
        // 持久化同步后 password 为空（save_password=false）：视为同一连接，不销毁池。
        let mut b = a.clone();
        b.password.clear();
        assert!(connection_configs_pool_equivalent(&a, &b));
        assert!(connection_configs_pool_equivalent(&b, &a));

        // 两个非空但不同的密码：同样忽略（密码不应触发池重建）。
        let mut c = a.clone();
        c.password = "changed-secret".to_string();
        assert!(connection_configs_pool_equivalent(&a, &c));
    }

    #[test]
    fn connection_configs_pool_equivalent_ignores_sidebar_table_loading_preference() {
        let a = mysql_config(None);
        let mut b = a.clone();
        b.sidebar_auto_load_all_tables = true;

        assert!(connection_configs_pool_equivalent(&a, &b));
        assert!(connection_configs_pool_equivalent(&b, &a));
    }

    #[test]
    fn connection_configs_pool_equivalent_detects_saved_password_change() {
        let mut a = mysql_config(None);
        a.save_password = true;
        a.password = "old-secret".to_string();
        let mut b = a.clone();
        b.password = "new-secret".to_string();
        // save_password=true：密码是真实连接参数，保存新密码后旧池不得继续用旧密码认证。
        assert!(!connection_configs_pool_equivalent(&a, &b));
        assert!(!connection_configs_pool_equivalent(&b, &a));
    }

    #[test]
    fn connection_configs_pool_equivalent_detects_real_parameter_changes() {
        let mut a = mysql_config(None);
        a.password = "secret".to_string();
        // host / port / username / database 等真实连接参数变化应视为不同连接（销毁池）。
        let mut host = a.clone();
        host.host = "other-host".to_string();
        assert!(!connection_configs_pool_equivalent(&a, &host));

        let mut port = a.clone();
        port.port = 5433;
        assert!(!connection_configs_pool_equivalent(&a, &port));

        let mut user = a.clone();
        user.username = "other-user".to_string();
        assert!(!connection_configs_pool_equivalent(&a, &user));

        let mut ssl = a.clone();
        ssl.ssl = true;
        assert!(!connection_configs_pool_equivalent(&a, &ssl));

        // 完全相等（含相同密码）→ true。
        assert!(connection_configs_pool_equivalent(&a, &a.clone()));
    }

    #[tokio::test]
    async fn apply_session_credential_injects_saved_password_only_for_no_save_connections() {
        let dir = std::env::temp_dir().join(format!("dbx-core-session-cred-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = crate::persistence::test_storage::open(&dir.join("storage.db")).await.unwrap();
        let state = AppState::new_with_plugin_dir(storage, dir.join("plugins"));
        let _ = state.session_credentials.set("", "conn-a", "s3cret");

        let mut config = mysql_config(None);
        config.id = "conn-a".to_string();
        config.save_password = false;
        config.password.clear();

        // save_password=false + db_config 密码为空 → 从会话凭据仓库注入。
        let mut db_config = metadata_connection_config(&config);
        state.apply_session_credential(&config, &mut db_config, &config.id);
        assert_eq!(db_config.password, "s3cret");

        // save_password=true → 不注入（走持久化水合的密码，若为空则保持空）。
        config.save_password = true;
        let mut db_config = metadata_connection_config(&config);
        state.apply_session_credential(&config, &mut db_config, &config.id);
        assert_eq!(db_config.password, "");

        // 无会话凭据 → 保持空密码（"断开并忘记"后重新输入）。
        state.session_credentials.remove("", "conn-a");
        config.save_password = false;
        let mut db_config = metadata_connection_config(&config);
        state.apply_session_credential(&config, &mut db_config, &config.id);
        assert_eq!(db_config.password, "");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn apply_session_credential_reads_owner_scoped_credentials_only() {
        let dir = std::env::temp_dir().join(format!("dbx-core-session-cred-owner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = crate::persistence::test_storage::open(&dir.join("storage.db")).await.unwrap();
        let state = AppState::new_with_plugin_dir(storage, dir.join("plugins"));

        let mut config = mysql_config(None);
        config.id = "conn-a".to_string();
        config.save_password = false;
        config.password.clear();

        // 会话 X 输入了密码，会话 Y 未输入：Y 的请求（owner=token-y）不得注入 X 的密码。
        let _ = state.session_credentials.set("token-x", "conn-a", "x-secret");
        crate::session_credentials::with_credential_owner(Some("token-y".to_string()), async {
            let mut db_config = metadata_connection_config(&config);
            state.apply_session_credential(&config, &mut db_config, &config.id);
            assert_eq!(db_config.password, "");
        })
        .await;

        // 会话 X 自身能读到自己的密码。
        crate::session_credentials::with_credential_owner(Some("token-x".to_string()), async {
            let mut db_config = metadata_connection_config(&config);
            state.apply_session_credential(&config, &mut db_config, &config.id);
            assert_eq!(db_config.password, "x-secret");
        })
        .await;

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn pool_credential_owner_mismatch_prevents_cross_session_pool_reuse() {
        let dir = std::env::temp_dir().join(format!("dbx-core-pool-owner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = crate::persistence::test_storage::open(&dir.join("storage.db")).await.unwrap();
        let state = AppState::new_with_plugin_dir(storage, dir.join("plugins"));

        let mut config = mysql_config(None);
        config.id = "conn-a".to_string();
        config.save_password = false;

        // 模拟会话 X 创建的 no-save 池。
        state.session_credentials.record_pool_owner("conn-a", "token-x");

        // 同一会话 X 复用 → 无冲突。
        crate::session_credentials::with_credential_owner(Some("token-x".to_string()), async {
            assert!(!state.pool_credential_owner_mismatch(&config, "conn-a").await);
        })
        .await;

        // 另一会话 Y 请求同一 no-save 池 → 冲突，需以 Y 的凭据重建，避免复用 X 的密码。
        crate::session_credentials::with_credential_owner(Some("token-y".to_string()), async {
            assert!(state.pool_credential_owner_mismatch(&config, "conn-a").await);
        })
        .await;

        // owner 标记缺失时按不可信处理，避免全局配置失效与异步移除旧池之间复用旧池。
        state.session_credentials.clear_connection("conn-a");
        crate::session_credentials::with_credential_owner(Some("token-x".to_string()), async {
            assert!(state.pool_credential_owner_mismatch(&config, "conn-a").await);
        })
        .await;

        // 桌面端（无 owner）单用户 → 不冲突，按既有逻辑复用。
        assert!(!state.pool_credential_owner_mismatch(&config, "conn-a").await);

        // 已保存密码连接始终共享池 → 不冲突。
        config.save_password = true;
        crate::session_credentials::with_credential_owner(Some("token-y".to_string()), async {
            assert!(!state.pool_credential_owner_mismatch(&config, "conn-a").await);
        })
        .await;

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn task_client_session_ids_are_stable_and_isolated() {
        assert_eq!(task_client_session_id("table-export", "job-1"), "table-export:job-1");
        assert_ne!(task_client_session_id("table-export", "job-1"), task_client_session_id("database-export", "job-1"));
        assert_ne!(task_client_session_id("table-export", "job-1"), task_client_session_id("table-export", "job-2"));
    }

    pub(super) async fn test_app_state() -> (AppState, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("dbx-core-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage = crate::persistence::test_storage::open(&dir.join("storage.db")).await.unwrap();
        (AppState::new(storage), dir)
    }

    #[tokio::test]
    async fn connection_lifecycle_invalidation_rotates_generation_and_cancels_previous_snapshot() {
        let (state, dir) = test_app_state().await;
        let first = state.connection_lifecycle_snapshot("conn");
        assert!(state.connection_lifecycle_is_current("conn", &first));

        state.invalidate_connection_lifecycle("conn");

        tokio::time::timeout(Duration::from_millis(100), first.cancellation().cancelled())
            .await
            .expect("previous lifecycle must be cancelled");
        assert!(!state.connection_lifecycle_is_current("conn", &first));
        let second = state.connection_lifecycle_snapshot("conn");
        assert!(state.connection_lifecycle_is_current("conn", &second));
        assert!(!second.cancellation().is_cancelled());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn every_connection_pool_removal_boundary_invalidates_lifecycle_snapshots() {
        let (state, dir) = test_app_state().await;

        let removed = state.connection_lifecycle_snapshot("removed");
        state.remove_connection_pools("removed").await;
        assert!(removed.cancellation().is_cancelled());

        let dropped = state.connection_lifecycle_snapshot("dropped");
        state.drop_connection_pools_without_close("dropped").await;
        assert!(dropped.cancellation().is_cancelled());

        let detached = state.connection_lifecycle_snapshot("detached");
        state.remove_connection_pools_detached("detached").await;
        assert!(detached.cancellation().is_cancelled());

        let shutdown = state.connection_lifecycle_snapshot("shutdown");
        state.shutdown(Duration::from_millis(100)).await;
        assert!(shutdown.cancellation().is_cancelled());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn named_resource_budget_is_shared_across_app_state_views() {
        let (state, dir) = test_app_state().await;
        let first = state.shared_resource_budget("fixed-owner", 2).unwrap();
        let second = state.shared_resource_budget("fixed-owner", 2).unwrap();
        assert!(Arc::ptr_eq(&first, &second));

        let first_permit = first.clone().try_acquire_owned().unwrap();
        let second_permit = second.clone().try_acquire_owned().unwrap();
        assert!(first.clone().try_acquire_owned().is_err());
        drop(first_permit);
        assert!(second.clone().try_acquire_owned().is_ok());
        drop(second_permit);

        assert!(state.shared_resource_budget("fixed-owner", 3).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn webview_reload_session_cancellation_marks_running_tasks_cancelled() {
        let (state, dir) = test_app_state().await;
        let registered = state.running_queries.register_task(
            "exec-1".to_string(),
            crate::query_cancel::RunningTaskMetadata::query("conn-1", "main", Some("tab-1".to_string())),
        );
        assert!(!registered.token().is_cancelled());

        // The narrow renderer-reload boundary signals all running-query tasks
        // (SQL execution and exports register here) without touching pools.
        assert_eq!(state.cancel_webview_reload_session_tasks(), 1);
        assert!(registered.token().is_cancelled());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn ssh_tunnel_test_includes_prerequisites_through_final_ssh_hop() {
        let before = TransportLayerConfig::Proxy(ProxyTunnelConfig {
            id: "before".to_string(),
            name: String::new(),
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: "proxy.internal".to_string(),
            port: 1080,
            username: String::new(),
            password: String::new(),
            test_target: None,
            profile_id: String::new(),
        });
        let ssh = TransportLayerConfig::Ssh(ssh_layer("ssh", ""));
        let after = TransportLayerConfig::Proxy(ProxyTunnelConfig {
            id: "after".to_string(),
            name: String::new(),
            enabled: true,
            proxy_type: ProxyType::Http,
            host: "downstream.internal".to_string(),
            port: 8080,
            username: String::new(),
            password: String::new(),
            test_target: None,
            profile_id: String::new(),
        });
        let layers = vec![before.clone(), ssh.clone(), after];

        assert_eq!(transport_layers_through_last_ssh(&layers).unwrap(), &[before, ssh]);
    }

    #[test]
    fn ssh_tunnel_test_rejects_chain_without_ssh() {
        let layers = vec![TransportLayerConfig::Proxy(ProxyTunnelConfig {
            id: "proxy".to_string(),
            name: String::new(),
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: "proxy.internal".to_string(),
            port: 1080,
            username: String::new(),
            password: String::new(),
            test_target: None,
            profile_id: String::new(),
        })];

        assert_eq!(
            transport_layers_through_last_ssh(&layers).unwrap_err(),
            "Connection has no enabled SSH tunnel layer"
        );
    }

    #[tokio::test]
    async fn test_tunnel_profile_rejects_non_ssh_and_missing_host() {
        let (state, dir) = test_app_state().await;

        // Proxy profiles now attempt a connection; with no proxy running at the
        // test address the result is a connection error, not an SSH-only guard.
        let test_port = portpicker::pick_unused_port().expect("no port available");
        let proxy = TransportLayerConfig::Proxy(ProxyTunnelConfig {
            id: "p1".to_string(),
            name: String::new(),
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: "127.0.0.1".to_string(),
            port: test_port,
            username: String::new(),
            password: String::new(),
            test_target: None,
            profile_id: String::new(),
        });
        let err = state.test_tunnel_profile(&proxy).await.unwrap_err();
        assert!(!err.contains("SSH"), "proxy test should not return SSH error, got: {err}");

        // An SSH profile with no host fails fast rather than dialing an empty host.
        let ssh = TransportLayerConfig::Ssh(SshTunnelConfig {
            id: "s1".to_string(),
            name: String::new(),
            enabled: true,
            host: String::new(),
            port: 22,
            user: "root".to_string(),
            password: String::new(),
            key_path: String::new(),
            key_passphrase: String::new(),
            connect_timeout_secs: 5,
            expose_lan: false,
            use_ssh_agent: false,
            ssh_agent_sock_path: String::new(),
            auth_method: "password".to_string(),
            allow_exec_channel_proxy: false,
            proxy_command: String::new(),
            profile_id: String::new(),
        });
        assert!(state.test_tunnel_profile(&ssh).await.is_err());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    #[ignore = "requires DBX_TEST_MYSQL_URL"]
    async fn live_mysql_health_check_keeps_saturated_pool() {
        let url = std::env::var("DBX_TEST_MYSQL_URL").expect("DBX_TEST_MYSQL_URL is required");
        let (state, dir) = test_app_state().await;
        let config = mysql_config(Some("testdb"));
        let pool = db::mysql::connect_bare_with_pool_limit(&url, Duration::from_secs(5), 1).await.unwrap();
        state
            .insert_connection_pool("conn".to_string(), PoolKind::Mysql(pool.clone(), MysqlMode::Normal), &config)
            .await
            .unwrap();
        let held_connection = pool.get_conn().await.unwrap();

        let started = Instant::now();
        state.check_connection_health("conn").await.unwrap();

        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(state.connections.read().await.contains_key("conn"));
        drop(held_connection);
        state.remove_connection_pools_detached("conn").await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn mysql_health_check_keeps_pool_when_connection_creation_exceeds_probe_budget() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let pool_options =
            mysql_async::PoolOpts::new().with_constraints(mysql_async::PoolConstraints::new(1, 2).unwrap());
        let options = mysql_async::OptsBuilder::default()
            .ip_or_hostname(address.ip().to_string())
            .tcp_port(address.port())
            .user(Some("fault-injection"))
            .pass(Some("fault-injection"))
            .pool_opts(Some(pool_options));
        let pool = db::mysql::MySqlPool::new(options, 2);
        let (state, dir) = test_app_state().await;
        state.connections.write().await.insert("conn".to_string(), PoolKind::Mysql(pool.clone(), MysqlMode::Normal));

        let started = Instant::now();
        assert!(!state.remove_stale_connection_pool("conn").await);

        assert!(started.elapsed() >= super::HEALTH_CHECK_POOL_ACQUIRE_TIMEOUT);
        assert!(matches!(
            state.connections.read().await.get("conn"),
            Some(PoolKind::Mysql(current, _)) if pool.is_same_pool(current)
        ));
        state.connections.write().await.remove("conn");
        server.abort();
        let _ = tokio::time::timeout(Duration::from_secs(1), pool.disconnect()).await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keepalive_probe_keeps_a_busy_pool_but_invalidates_a_dead_one() {
        // Every connection is checked out: the probe learned nothing about pool health, so the
        // pool must survive. Invalidating it here aborts whatever holds the connection — a
        // truncate import reports "Connection not found for transaction" on its next chunk.
        assert!(!keepalive_failure_proves_pool_dead(
            "MySQL connection pool checkout timed out [stage=wait, timeout_ms=10000]"
        ));
        // A checkout failure while creating a connection is still evidence the pool is dead.
        assert!(keepalive_failure_proves_pool_dead(
            "MySQL connection pool checkout failed [stage=create]: connection refused"
        ));
        assert!(keepalive_failure_proves_pool_dead(
            "MySQL connection pool checkout timed out [stage=create, timeout_ms=10000]"
        ));
    }

    #[test]
    fn mysql_metadata_connection_ignores_saved_default_database() {
        let config = mysql_config(Some("app"));

        let metadata = metadata_connection_config(&config);

        assert_eq!(metadata.database, None);
        assert_eq!(metadata.db_type, DatabaseType::Mysql);
    }

    #[test]
    fn mysql_metadata_fallback_uses_saved_default_database() {
        let config = mysql_config(Some("app"));
        let metadata = metadata_connection_config(&config);

        assert_eq!(
            mysql_metadata_fallback_url(&config, &metadata, &config.host, config.port),
            Some("mysql://root:secret@127.0.0.1:3306/app?ssl-mode=disabled&charset=utf8mb4".to_string())
        );
    }

    #[test]
    fn mysql_metadata_fallback_is_unavailable_without_default_database() {
        let config = mysql_config(None);
        let metadata = metadata_connection_config(&config);

        assert_eq!(mysql_metadata_fallback_url(&config, &metadata, &config.host, config.port), None);
    }

    #[test]
    fn mysql_database_connection_keeps_requested_database() {
        let config = mysql_config(Some("app"));

        let scoped = database_connection_config(&config, Some("analytics"));

        assert_eq!(scoped.database.as_deref(), Some("analytics"));
    }

    #[test]
    fn mysql_pool_size_keeps_session_pools_single_connection() {
        assert_eq!(super::mysql_pool_max_connections_for_session(None), 10);
        assert_eq!(super::mysql_pool_max_connections_for_session(Some("")), 10);
        assert_eq!(super::mysql_pool_max_connections_for_session(Some("tab-1")), 1);
    }

    #[test]
    fn stale_mysql_pool_observation_does_not_remove_replacement_generation() {
        let checked = crate::db::mysql::MySqlPool::new("mysql://root@127.0.0.1:3306/app", 10);
        let checked_clone = checked.clone();
        let replacement = crate::db::mysql::MySqlPool::new("mysql://root@127.0.0.1:3306/app", 10);
        assert!(checked.is_same_pool(&checked_clone));
        assert!(!checked.is_same_pool(&replacement));
    }

    #[tokio::test]
    async fn stale_mysql_cleanup_preserves_replacement_support_state() {
        let (state, dir) = test_app_state().await;
        let state = std::sync::Arc::new(state);
        let pool_key = "conn:app";
        let checked = crate::db::mysql::MySqlPool::new("mysql://root@127.0.0.1:3306/app", 10);
        let replacement = crate::db::mysql::MySqlPool::new("mysql://root@127.0.0.1:3306/app", 10);
        let mut config = mysql_config(Some("app"));
        config.keepalive_interval_secs = 60;
        state
            .connections
            .write()
            .await
            .insert(pool_key.to_string(), PoolKind::Mysql(checked.clone(), MysqlMode::Normal));
        state.pool_activity.write().await.insert(pool_key.to_string(), super::PoolActivity::now());
        let checked_publication = state.pool_publication_snapshot(pool_key).await.unwrap().publication;
        state.start_keepalive_task(pool_key, &PoolKind::Mysql(checked, MysqlMode::Normal), &config);
        assert_eq!(state.supervised_task_count(), 1);

        let route_removed = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let continue_cleanup = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let cleanup_state = state.clone();
        let cleanup_publication = checked_publication.clone();
        let cleanup_route_removed = route_removed.clone();
        let cleanup_continue = continue_cleanup.clone();
        let cleanup = tokio::spawn(async move {
            cleanup_state
                .remove_stale_pool_if_current_inner(
                    pool_key,
                    &cleanup_publication,
                    Some(super::StalePoolCleanupBarriers {
                        before_removal: None,
                        after_removal: Some((cleanup_route_removed, cleanup_continue)),
                    }),
                )
                .await
        });
        route_removed.wait().await;

        let publish_state = state.clone();
        let publish_config = config.clone();
        let publish_replacement = replacement.clone();
        let mut publish = tokio::spawn(async move {
            publish_state
                .insert_connection_pool(
                    pool_key.to_string(),
                    PoolKind::Mysql(publish_replacement, MysqlMode::Normal),
                    &publish_config,
                )
                .await
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut publish).await.is_err(),
            "replacement publication must wait while stale support state is cleaned"
        );
        continue_cleanup.wait().await;
        assert!(cleanup.await.unwrap());
        publish.await.unwrap().unwrap();

        let current = state.pool_publication_snapshot(pool_key).await.expect("replacement must remain routable");
        assert!(matches!(current.pool, PoolKind::Mysql(ref pool, _) if replacement.is_same_pool(pool)));
        assert!(!current.publication.is_same(&checked_publication));
        assert!(state.pool_activity.read().await.contains_key(pool_key));
        assert_eq!(state.supervised_task_count(), 1);

        state.shutdown(Duration::from_secs(1)).await;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mysql_hostname_connections_skip_tcp_probe() {
        let mut config = mysql_config(Some("app"));
        config.host = "mysql.example.com".to_string();

        assert!(!uses_tcp_probe(&config, "mysql.example.com", 3306));
        config.host = "192.0.2.10".to_string();
        assert!(!uses_tcp_probe(&config, "192.0.2.10", 3306));
        assert!(uses_tcp_probe(&config, "127.0.0.1", 53306));
    }

    /// Fake agent whose `validate_connection` reports a real failure.
    ///
    /// It must NOT answer with an unsupported-method error: `ping_keepalive_target` treats that
    /// as a healthy connection (`is_agent_validate_connection_unsupported`), which would make
    /// this fixture silently green.
    const KEEPALIVE_FAILING_AGENT: &str = r#"import json, sys
print(json.dumps({'ready': True}), flush=True)
for line in sys.stdin:
    req = json.loads(line)
    if req['method'] == 'handshake':
        result = {'protocolVersion': 2, 'agentProtocolVersion': 2, 'capabilities': ['multi_session']}
        print(json.dumps({'jsonrpc': '2.0', 'id': req['id'], 'result': result}), flush=True)
    elif 'validate_connection' in req['method']:
        print(json.dumps({'jsonrpc': '2.0', 'id': req['id'], 'error': {'code': -32000, 'message': 'connection refused by keepalive fixture'}}), flush=True)
    else:
        print(json.dumps({'jsonrpc': '2.0', 'id': req['id'], 'result': {}}), flush=True)
"#;

    #[test]
    fn detects_database_connection_slot_exhaustion_errors() {
        assert!(super::is_connection_slot_exhausted_error(
            "Agent RPC error (-1): FATAL: remaining connection slots are reserved for superuser manager connections"
        ));
        assert!(super::is_connection_slot_exhausted_error("ORA-00018: maximum number of sessions exceeded"));
        assert!(!super::is_connection_slot_exhausted_error("password authentication failed"));
    }

    #[test]
    fn pool_activity_touch_never_moves_backwards() {
        let activity = super::PoolActivity::idle_for(std::time::Duration::from_secs(0));
        let newer = u64::MAX;
        activity.last_used_at_ms.store(newer, std::sync::atomic::Ordering::Relaxed);
        // touch 的当前时间早于已存值：不得倒退
        activity.touch();
        assert_eq!(activity.last_used_at_ms.load(std::sync::atomic::Ordering::Relaxed), newer);
    }

    #[tokio::test]
    async fn global_health_preserves_transport_for_connections_without_failed_pools() {
        let (state, dir) = test_app_state().await;
        let mut config = mysql_config(None);
        config.id = "proxied-connection".to_string();
        config.transport_layers = vec![TransportLayerConfig::Proxy(ProxyTunnelConfig {
            profile_id: String::new(),
            id: "proxy".to_string(),
            name: String::new(),
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: "127.0.0.1".to_string(),
            port: 65000,
            username: String::new(),
            password: String::new(),
            test_target: None,
        })];
        state.configs.write().await.insert(config.id.clone(), config.clone());
        let (_, local_port) = state.connection_host_port(&config.id, &config).await.unwrap();

        state.refresh_connections().await;

        assert_eq!(state.proxy_tunnels.local_port("proxied-connection:transport:0").await, Some(local_port));
        state.reset_connection_transport(&config.id).await;
        let _ = std::fs::remove_dir_all(dir);
    }

    fn ssh_layer(id: &str, profile_id: &str) -> SshTunnelConfig {
        SshTunnelConfig {
            id: id.to_string(),
            name: String::new(),
            enabled: true,
            host: String::new(),
            port: 22,
            user: String::new(),
            password: String::new(),
            key_path: String::new(),
            key_passphrase: String::new(),
            connect_timeout_secs: 5,
            expose_lan: false,
            use_ssh_agent: false,
            ssh_agent_sock_path: String::new(),
            auth_method: String::new(),
            allow_exec_channel_proxy: false,
            proxy_command: String::new(),
            profile_id: profile_id.to_string(),
        }
    }

    fn proxy_layer(id: &str, profile_id: &str) -> ProxyTunnelConfig {
        ProxyTunnelConfig {
            id: id.to_string(),
            name: String::new(),
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: String::new(),
            port: 1080,
            username: String::new(),
            password: String::new(),
            test_target: None,
            profile_id: profile_id.to_string(),
        }
    }

    fn http_tunnel_layer(id: &str, profile_id: &str) -> HttpTunnelConfig {
        HttpTunnelConfig {
            id: id.to_string(),
            name: String::new(),
            enabled: true,
            url: String::new(),
            token: String::new(),
            connect_timeout_secs: 10,
            profile_id: profile_id.to_string(),
        }
    }

    #[tokio::test]
    async fn resolved_transport_layers_substitutes_shared_profiles() {
        let (state, dir) = test_app_state().await;

        let mut profile = ssh_layer("shared-bastion", "");
        profile.name = "Bastion".to_string();
        profile.host = "bastion.example.com".to_string();
        profile.user = "deploy".to_string();
        profile.password = "s3cret".to_string();
        profile.auth_method = "password".to_string();
        state.storage.save_tunnel_profiles(&[TransportLayerConfig::Ssh(profile)]).await.unwrap();

        let mut config = mysql_config(Some("app"));
        config.transport_layers = vec![TransportLayerConfig::Ssh(ssh_layer("layer-1", "shared-bastion"))];

        let resolved = state.resolved_transport_layers(&config).await.unwrap();
        assert_eq!(resolved.len(), 1);
        match &resolved[0] {
            TransportLayerConfig::Ssh(ssh) => {
                // Profile supplies the configuration; the layer keeps its identity.
                assert_eq!(ssh.id, "layer-1");
                assert_eq!(ssh.profile_id, "shared-bastion");
                assert_eq!(ssh.host, "bastion.example.com");
                assert_eq!(ssh.user, "deploy");
                assert_eq!(ssh.password, "s3cret");
            }
            other => panic!("expected ssh layer, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn resolved_transport_layers_fails_closed_on_missing_profile() {
        let (state, dir) = test_app_state().await;

        let mut config = mysql_config(Some("app"));
        config.transport_layers = vec![TransportLayerConfig::Ssh(ssh_layer("layer-1", "deleted-profile"))];

        let err = state.resolved_transport_layers(&config).await.unwrap_err();
        assert!(err.contains("no longer exists"), "unexpected error: {err}");

        // Disabled reference layers are filtered out before resolution, so a
        // dangling reference on a disabled layer must not block connecting.
        let mut disabled = ssh_layer("layer-1", "deleted-profile");
        disabled.enabled = false;
        config.transport_layers = vec![TransportLayerConfig::Ssh(disabled)];
        assert!(state.resolved_transport_layers(&config).await.unwrap().is_empty());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn resolved_transport_layers_rejects_mismatched_profile_types() {
        let (state, dir) = test_app_state().await;

        let mismatches = [
            (
                TransportLayerConfig::Ssh(ssh_layer("layer", "shared")),
                TransportLayerConfig::Proxy(proxy_layer("shared", "")),
            ),
            (
                TransportLayerConfig::Proxy(proxy_layer("layer", "shared")),
                TransportLayerConfig::HttpTunnel(http_tunnel_layer("shared", "")),
            ),
            (
                TransportLayerConfig::HttpTunnel(http_tunnel_layer("layer", "shared")),
                TransportLayerConfig::Ssh(ssh_layer("shared", "")),
            ),
        ];

        for (layer, profile) in mismatches {
            state.storage.save_tunnel_profiles(&[profile]).await.unwrap();
            let mut config = mysql_config(Some("app"));
            config.transport_layers = vec![layer];

            let error = state.resolved_transport_layers(&config).await.unwrap_err();
            assert!(error.contains("different type"));
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn proxy_connection_uses_local_forward_endpoint() {
        let (state, dir) = test_app_state().await;
        let mut config = mysql_config(Some("app"));
        config.transport_layers = vec![TransportLayerConfig::Proxy(ProxyTunnelConfig {
            profile_id: String::new(),
            id: "proxy".to_string(),
            name: String::new(),
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: "127.0.0.1".to_string(),
            port: 65000,
            username: String::new(),
            password: String::new(),
            test_target: None,
        })];

        let (host, _port) = state.connection_host_port("proxied", &config).await.unwrap();

        assert_eq!(host, "127.0.0.1");
        state.proxy_tunnels.stop_tunnel("proxied:transport:0").await;
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoolSessionRole {
    Metadata,
    Workload,
}
