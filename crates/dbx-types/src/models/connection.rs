use percent_encoding::{percent_decode_str, utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum IdentifierCase {
    Lower,
    Upper,
    Mixed,
}

/// Why the backend declared a connection's pools dead while nobody was using it.
///
/// A stable enum on purpose: the frontend only needs to tell the two cases apart, and raw
/// driver/network error text must not leak into a background UI notification (#4339).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionLivenessFailureKind {
    /// The keepalive probe ran and reported a dead connection.
    ProbeFailed,
    /// The keepalive probe exceeded its budget.
    TimedOut,
}

/// A message on the connection-liveness channel (#4339).
///
/// One shape for both shells and both transports, discriminated by `kind`. The `Resync`
/// variant exists because a broadcast subscriber can fall behind: without a way to say
/// "you skipped messages", a dropped `Lost` would leave a sidebar claiming a dead
/// connection is connected until the user happened to act on it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ConnectionLivenessMessage {
    /// The connection has no pools left, so the sidebar must stop showing it as connected.
    Lost { connection_id: String, failure_kind: ConnectionLivenessFailureKind },
    /// The transport skipped messages and must re-check every connection it currently
    /// shows as connected.
    Resync,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseConnectionInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_charset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_collation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unquoted_identifier_case: Option<IdentifierCase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quoted_identifier_case: Option<IdentifierCase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jdbc_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestResult {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_info: Option<DatabaseConnectionInfo>,
}

impl ConnectionTestResult {
    pub fn success(message: impl Into<String>) -> Self {
        Self { message: message.into(), database_info: None }
    }

    pub fn with_database_info(mut self, database_info: Option<DatabaseConnectionInfo>) -> Self {
        self.database_info = database_info;
        self
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DatabaseInfoEnvelope {
    #[serde(default)]
    database_info: Option<DatabaseConnectionInfo>,
}

pub fn database_info_from_protocol_value(value: &Value) -> Option<DatabaseConnectionInfo> {
    serde_json::from_value::<DatabaseInfoEnvelope>(value.clone()).ok()?.database_info
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RedisKeyGroupRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub includes: Vec<String>,
    pub excludes: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RedisKeyGroupView {
    List,
    Tree,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RedisKeyGrouping {
    pub version: u8,
    pub enabled: bool,
    pub inner_view: RedisKeyGroupView,
    pub rules: Vec<RedisKeyGroupRule>,
}

#[derive(Clone, Serialize, PartialEq)]
pub struct ConnectionConfig {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub db_type: DatabaseType,
    #[serde(default)]
    pub driver_profile: Option<String>,
    #[serde(default)]
    pub driver_label: Option<String>,
    #[serde(default)]
    pub url_params: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_java_options: Vec<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_databases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_database_patterns: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_schemas: Option<HashMap<String, Vec<String>>>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub show_system_schemas: bool,
    /// Frontend navigation preference: exhaust the paginated Tables group when opened.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sidebar_auto_load_all_tables: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attached_databases: Vec<AttachedDatabaseConfig>,
    /// SQL statements executed right after the connection is established
    /// (DuckDB only for now): INSTALL/LOAD, SET, CREATE SECRET, ATTACH, ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub init_script: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    /// Path to this connection's documentation notes file. Set by the
    /// desktop app; the CLI takes an explicit `--notes` path instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs_notes_path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transport_layers: Vec<TransportLayerConfig>,
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_query_timeout_secs")]
    pub query_timeout_secs: u64,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    #[serde(default = "default_keepalive_interval_secs")]
    pub keepalive_interval_secs: u64,
    #[serde(default)]
    pub ssl: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ca_cert_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_cert_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub client_key_path: String,
    #[serde(default)]
    pub sysdba: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oracle_connection_type: Option<String>,
    /// Connection-level NLS_LANG override for OCI connections. Empty keeps the
    /// global default; a value makes this connection own a dedicated agent
    /// process so its client charset cannot bleed into other sessions (the
    /// variable is process-scoped for OCI).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oracle_oci_nls_lang: Option<String>,
    /// Connection-level `TNS_ADMIN` override for OCI (thick) connections.
    ///
    /// Points at the directory holding `tnsnames.ora` / `sqlnet.ora` / the
    /// wallet, so any OCI connection can use an ADB wallet or network options
    /// without switching to the TNS connection form. Resolved against the
    /// global default and the TNS connection string in
    /// `dbx_core::connection::AppState::agent_launch_env`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oracle_oci_tns_admin: Option<String>,
    #[serde(default)]
    pub connection_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_connection_mode: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub redis_sentinel_master: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub redis_sentinel_nodes: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub redis_sentinel_username: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub redis_sentinel_password: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub redis_sentinel_tls: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub redis_cluster_nodes: String,
    #[serde(default = "default_redis_key_separator", skip_serializing_if = "is_default_redis_separator")]
    pub redis_key_separator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_scan_page_size: Option<u64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub redis_database_aliases: HashMap<String, String>,
    /// Optional key-search templates for the Redis key browser (one pattern per entry).
    /// Empty means inherit the global editor setting.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redis_key_templates: Vec<String>,
    /// Default Redis MATCH pattern for new key-browser tabs; empty means all keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_key_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redis_key_grouping: Option<RedisKeyGrouping>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub etcd_endpoints: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gbase_server: String,
    /// Informix server name (INFORMIXSERVER). When empty, the agent
    /// derives it from the hostname.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub informix_server: String,
    /// Typed configuration for external tabular sources.
    #[serde(default)]
    pub external_config: Option<serde_json::Value>,
    /// Owning plugin for a first-class plugin connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    /// `connection-provider` contribution id inside `plugin_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_connection_provider: Option<String>,
    /// Provider-defined connection kind, such as `ssh` or `s3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_connection_type: Option<String>,
    /// Provider-defined sensitive values. Persistence moves these into the
    /// connection secret store rather than keeping them in `config_json`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub connection_secrets: HashMap<String, String>,
    #[serde(default)]
    pub jdbc_driver_class: Option<String>,
    #[serde(default)]
    pub jdbc_driver_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub one_time: bool,
    /// Whether the database password may be persisted locally (SQLite
    /// `connection_secrets`). When false, the `"password"` secret is never
    /// written (or is deleted) and the user must type it on every connect.
    /// Defaults to `true` so pre-existing saved connections keep current
    /// behavior; a bare `#[serde(default)]` would upgrade them to "don't save"
    /// and delete every stored password on the next save.
    #[serde(default = "default_true")]
    pub save_password: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only: bool,
    /// Explicitly marks every database reachable through this connection as production.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_production: bool,
    /// Database-level production markers for multi-database connections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub production_databases: Vec<String>,
    /// Metadata captured from the latest successful connection test for this saved config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database_info: Option<DatabaseConnectionInfo>,
}

impl fmt::Debug for ConnectionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut value = serde_json::to_value(self).map_err(|_| fmt::Error)?;
        redact_connection_debug_value(&mut value);
        formatter.debug_tuple("ConnectionConfig").field(&value).finish()
    }
}

fn redact_connection_debug_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                let key = key.to_ascii_lowercase().replace(['_', '-'], "");
                if key.contains("password")
                    || key.contains("passphrase")
                    || key.contains("token")
                    || key.contains("secret")
                    || key.contains("apikey")
                    || key == "connectionstring"
                    || key == "initscript"
                {
                    *value = serde_json::Value::String("[REDACTED]".to_string());
                } else {
                    redact_connection_debug_value(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_connection_debug_value(value);
            }
        }
        _ => {}
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum TransportLayerConfig {
    Ssh(SshTunnelConfig),
    Proxy(ProxyTunnelConfig),
    #[serde(rename = "http_tunnel")]
    HttpTunnel(HttpTunnelConfig),
}

impl TransportLayerConfig {
    pub fn same_type_as(&self, other: &TransportLayerConfig) -> bool {
        matches!(
            (self, other),
            (TransportLayerConfig::Ssh(_), TransportLayerConfig::Ssh(_))
                | (TransportLayerConfig::Proxy(_), TransportLayerConfig::Proxy(_))
                | (TransportLayerConfig::HttpTunnel(_), TransportLayerConfig::HttpTunnel(_))
        )
    }

    pub fn id(&self) -> &str {
        match self {
            TransportLayerConfig::Ssh(layer) => &layer.id,
            TransportLayerConfig::Proxy(layer) => &layer.id,
            TransportLayerConfig::HttpTunnel(layer) => &layer.id,
        }
    }

    pub fn profile_id(&self) -> &str {
        match self {
            TransportLayerConfig::Ssh(layer) => &layer.profile_id,
            TransportLayerConfig::Proxy(layer) => &layer.profile_id,
            TransportLayerConfig::HttpTunnel(layer) => &layer.profile_id,
        }
    }

    /// Builds the concrete layer used at connect time for a layer that
    /// references a shared tunnel profile: the profile supplies the whole
    /// configuration while the referencing layer keeps its own identity,
    /// enabled flag, and profile reference.
    pub fn resolved_from_profile(&self, profile: &TransportLayerConfig) -> TransportLayerConfig {
        let mut resolved = profile.clone();
        let (id, enabled, profile_id) = (self.id().to_string(), self.enabled(), self.profile_id().to_string());
        match &mut resolved {
            TransportLayerConfig::Ssh(layer) => {
                layer.id = id;
                layer.enabled = enabled;
                layer.profile_id = profile_id;
            }
            TransportLayerConfig::Proxy(layer) => {
                layer.id = id;
                layer.enabled = enabled;
                layer.profile_id = profile_id;
            }
            TransportLayerConfig::HttpTunnel(layer) => {
                layer.id = id;
                layer.enabled = enabled;
                layer.profile_id = profile_id;
            }
        }
        resolved
    }

    pub fn scrub_secrets(&mut self) {
        match self {
            TransportLayerConfig::Ssh(layer) => {
                layer.password = String::new();
                layer.key_passphrase = String::new();
            }
            TransportLayerConfig::Proxy(layer) => {
                layer.password = String::new();
            }
            TransportLayerConfig::HttpTunnel(layer) => {
                layer.token = String::new();
            }
        }
    }

    pub fn name(&self) -> &str {
        match self {
            TransportLayerConfig::Ssh(layer) => &layer.name,
            TransportLayerConfig::Proxy(layer) => &layer.name,
            TransportLayerConfig::HttpTunnel(layer) => &layer.name,
        }
    }

    pub fn enabled(&self) -> bool {
        match self {
            TransportLayerConfig::Ssh(layer) => layer.enabled,
            TransportLayerConfig::Proxy(layer) => layer.enabled,
            TransportLayerConfig::HttpTunnel(layer) => layer.enabled,
        }
    }

    pub fn endpoint(&self) -> (&str, u16) {
        match self {
            TransportLayerConfig::Ssh(layer) => (&layer.host, layer.port),
            TransportLayerConfig::Proxy(layer) => (&layer.host, layer.port),
            // HTTP script tunnel layers dial a PHP script URL instead of a host:port
            // endpoint, and are validated as the outermost transport layer.
            TransportLayerConfig::HttpTunnel(_) => ("", 0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SshTunnelConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub key_path: String,
    #[serde(default)]
    pub key_passphrase: String,
    #[serde(default = "default_ssh_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    #[serde(default)]
    pub expose_lan: bool,
    #[serde(default)]
    pub use_ssh_agent: bool,
    /// Custom SSH agent socket path (e.g. `~/.ssh/agent.sock`).
    /// When set and `use_ssh_agent` is true, this path is used instead of
    /// the `SSH_AUTH_SOCK` environment variable.
    #[serde(default)]
    pub ssh_agent_sock_path: String,
    /// Login method: `"password"`, `"key"`, `"key+password"`, `"agent"`, or `"none"`.
    /// Empty string means an older saved connection predating this field —
    /// the backend falls back to probing key > password > agent based on
    /// which fields are non-empty. When set to a specific method the backend
    /// only tries that method (after the standard `none` probe).
    /// `"key+password"` tries private key auth first and falls back to
    /// password auth if the key is rejected.
    #[serde(default)]
    pub auth_method: String,
    /// Allow an SSH session exec channel to run `nc` when the server rejects
    /// `direct-tcpip`. Disabled by default because this can bypass a server's
    /// TCP-forwarding policy; enable only for trusted JumpServer/Koko setups.
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_exec_channel_proxy: bool,
    /// OpenSSH-style `ProxyCommand` used to reach this SSH host instead of a
    /// direct TCP connection (e.g. `nc %h %p` or
    /// `cloudflared access ssh --hostname %h`). Empty means a direct
    /// connection. Resolved from `~/.ssh/config` when the host is an alias and
    /// otherwise taken from the connection form. `%h`/`%p`/`%r`/`%%` are
    /// expanded from the effective host, port, and user before the command
    /// starts; the executable must be one of
    /// `dbx_drivers::db::ssh_proxy_command::ALLOWED_PROXY_COMMAND_BINARIES`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub proxy_command: String,
    /// When non-empty, this layer references a shared tunnel profile
    /// (Settings > Tunnels). The profile's configuration replaces this
    /// layer's own fields at connect time; only `id` and `enabled` are
    /// kept from the referencing layer.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyTunnelConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub proxy_type: ProxyType,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_proxy_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// Optional target for tunnel profile testing. When set, the test connects
    /// to this `host:port`; when empty, the test performs an endpoint-only
    /// liveness probe that requires no external destination.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_target: Option<String>,
    /// See [`SshTunnelConfig::profile_id`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpTunnelConfig {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default = "default_http_tunnel_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    /// See [`SshTunnelConfig::profile_id`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttachedDatabaseConfig {
    pub name: String,
    pub path: String,
}

fn default_true() -> bool {
    true
}

fn default_ssh_port() -> u16 {
    22
}

pub fn default_ssh_connect_timeout_secs() -> u64 {
    5
}

pub fn default_http_tunnel_connect_timeout_secs() -> u64 {
    10
}

pub fn default_connect_timeout_secs() -> u64 {
    10
}

/// Cloud Spanner schema changes are long-running operations rather than plain
/// statements. Measured against the real service, `CREATE INDEX` on an *empty*
/// table took 22.9-33.6s over seven runs, so the generic default fails
/// intermittently — and the failure is misleading, because the operation keeps
/// running server-side and completes, leaving a retry to report
/// `Duplicate name in schema`. Raising a floor mirrors what
/// `connection::agent_connect_timeout` already does for Access.
pub const SPANNER_MIN_QUERY_TIMEOUT_SECS: u64 = 120;

pub fn default_query_timeout_secs() -> u64 {
    60
}

pub fn default_idle_timeout_secs() -> u64 {
    60
}

pub fn default_keepalive_interval_secs() -> u64 {
    30
}

fn default_proxy_port() -> u16 {
    1080
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub fn default_redis_key_separator() -> String {
    ":".to_string()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ProxyType {
    #[default]
    Socks5,
    Http,
}

include!(concat!(env!("OUT_DIR"), "/database_type.rs"));

#[derive(Deserialize)]
struct ConnectionConfigData {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub note: String,
    pub db_type: DatabaseType,
    #[serde(default)]
    pub driver_profile: Option<String>,
    #[serde(default)]
    pub driver_label: Option<String>,
    #[serde(default)]
    pub url_params: Option<String>,
    #[serde(default)]
    pub agent_java_options: Vec<String>,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    pub database: Option<String>,
    #[serde(default)]
    pub default_schema: Option<String>,
    #[serde(default)]
    pub visible_databases: Option<Vec<String>>,
    #[serde(default)]
    pub visible_database_patterns: Option<Vec<String>>,
    #[serde(default)]
    pub visible_schemas: Option<HashMap<String, Vec<String>>>,
    #[serde(default)]
    pub show_system_schemas: bool,
    #[serde(default)]
    pub sidebar_auto_load_all_tables: bool,
    #[serde(default)]
    pub attached_databases: Vec<AttachedDatabaseConfig>,
    #[serde(default)]
    pub init_script: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub docs_notes_path: Option<String>,
    #[serde(default)]
    pub transport_layers: Vec<TransportLayerConfig>,
    #[serde(default = "default_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_query_timeout_secs")]
    pub query_timeout_secs: u64,
    #[serde(default = "default_idle_timeout_secs")]
    pub idle_timeout_secs: u64,
    #[serde(default = "default_keepalive_interval_secs")]
    pub keepalive_interval_secs: u64,
    #[serde(default)]
    pub ssl: bool,
    #[serde(default)]
    pub ca_cert_path: String,
    #[serde(default)]
    pub client_cert_path: String,
    #[serde(default)]
    pub client_key_path: String,
    #[serde(default)]
    pub sysdba: bool,
    #[serde(default)]
    pub oracle_connection_type: Option<String>,
    #[serde(default)]
    pub oracle_oci_nls_lang: Option<String>,
    #[serde(default)]
    pub oracle_oci_tns_admin: Option<String>,
    #[serde(default)]
    pub connection_string: Option<String>,
    #[serde(default)]
    pub redis_connection_mode: Option<String>,
    #[serde(default)]
    pub redis_sentinel_master: String,
    #[serde(default)]
    pub redis_sentinel_nodes: String,
    #[serde(default)]
    pub redis_sentinel_username: String,
    #[serde(default)]
    pub redis_sentinel_password: String,
    #[serde(default)]
    pub redis_sentinel_tls: bool,
    #[serde(default)]
    pub redis_cluster_nodes: String,
    #[serde(default = "default_redis_key_separator")]
    pub redis_key_separator: String,
    #[serde(default)]
    pub redis_scan_page_size: Option<u64>,
    #[serde(default)]
    pub redis_database_aliases: HashMap<String, String>,
    #[serde(default)]
    pub redis_key_templates: Vec<String>,
    #[serde(default)]
    pub redis_key_filter: Option<String>,
    #[serde(default)]
    pub redis_key_grouping: Option<RedisKeyGrouping>,
    #[serde(default)]
    pub etcd_endpoints: String,
    #[serde(default)]
    pub gbase_server: String,
    #[serde(default)]
    pub informix_server: String,
    #[serde(default)]
    pub external_config: Option<serde_json::Value>,
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub plugin_connection_provider: Option<String>,
    #[serde(default)]
    pub plugin_connection_type: Option<String>,
    #[serde(default)]
    pub connection_secrets: HashMap<String, String>,
    #[serde(default)]
    pub jdbc_driver_class: Option<String>,
    #[serde(default)]
    pub jdbc_driver_paths: Vec<String>,
    #[serde(default)]
    pub one_time: bool,
    #[serde(default = "default_true")]
    pub save_password: bool,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub is_production: bool,
    #[serde(default)]
    pub production_databases: Vec<String>,
    #[serde(default)]
    pub database_info: Option<DatabaseConnectionInfo>,
}

impl From<ConnectionConfigData> for ConnectionConfig {
    fn from(data: ConnectionConfigData) -> Self {
        Self {
            id: data.id,
            name: data.name,
            note: data.note,
            db_type: data.db_type,
            driver_profile: data.driver_profile,
            driver_label: data.driver_label,
            url_params: data.url_params,
            agent_java_options: data.agent_java_options,
            host: data.host,
            port: data.port,
            username: data.username,
            password: data.password,
            database: data.database,
            default_schema: data.default_schema,
            visible_databases: data.visible_databases,
            visible_database_patterns: data.visible_database_patterns,
            visible_schemas: data.visible_schemas,
            show_system_schemas: data.show_system_schemas,
            sidebar_auto_load_all_tables: data.sidebar_auto_load_all_tables,
            attached_databases: data.attached_databases,
            init_script: data.init_script,
            color: data.color,
            docs_notes_path: data.docs_notes_path,
            transport_layers: data.transport_layers,
            connect_timeout_secs: data.connect_timeout_secs,
            query_timeout_secs: data.query_timeout_secs,
            idle_timeout_secs: data.idle_timeout_secs,
            keepalive_interval_secs: data.keepalive_interval_secs,
            ssl: data.ssl,
            ca_cert_path: data.ca_cert_path,
            client_cert_path: data.client_cert_path,
            client_key_path: data.client_key_path,
            sysdba: data.sysdba,
            oracle_connection_type: data.oracle_connection_type,
            oracle_oci_nls_lang: data.oracle_oci_nls_lang,
            oracle_oci_tns_admin: data.oracle_oci_tns_admin,
            connection_string: data.connection_string,
            redis_connection_mode: data.redis_connection_mode,
            redis_sentinel_master: data.redis_sentinel_master,
            redis_sentinel_nodes: data.redis_sentinel_nodes,
            redis_sentinel_username: data.redis_sentinel_username,
            redis_sentinel_password: data.redis_sentinel_password,
            redis_sentinel_tls: data.redis_sentinel_tls,
            redis_cluster_nodes: data.redis_cluster_nodes,
            redis_key_separator: data.redis_key_separator,
            redis_scan_page_size: data.redis_scan_page_size,
            redis_database_aliases: data.redis_database_aliases,
            redis_key_templates: data.redis_key_templates,
            redis_key_filter: data.redis_key_filter,
            redis_key_grouping: data.redis_key_grouping,
            etcd_endpoints: data.etcd_endpoints,
            gbase_server: data.gbase_server,
            informix_server: data.informix_server,
            external_config: data.external_config,
            plugin_id: data.plugin_id,
            plugin_connection_provider: data.plugin_connection_provider,
            plugin_connection_type: data.plugin_connection_type,
            connection_secrets: data.connection_secrets,
            jdbc_driver_class: data.jdbc_driver_class,
            jdbc_driver_paths: data.jdbc_driver_paths,
            one_time: data.one_time,
            save_password: data.save_password,
            read_only: data.read_only,
            is_production: data.is_production,
            production_databases: data.production_databases,
            database_info: data.database_info,
        }
    }
}

impl<'de> Deserialize<'de> for ConnectionConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut value = Value::deserialize(deserializer)?;
        migrate_legacy_transport_layers(&mut value);
        let data = ConnectionConfigData::deserialize(value).map_err(serde::de::Error::custom)?;
        if let Some(grouping) = &data.redis_key_grouping {
            let mut ids = std::collections::HashSet::new();
            if grouping.version != 1
                || grouping.rules.len() > 64
                || grouping.rules.iter().any(|rule| {
                    rule.id.is_empty()
                        || !ids.insert(&rule.id)
                        || rule.name.trim().is_empty()
                        || (rule.includes.is_empty() && rule.excludes.is_empty())
                        || rule.includes.len() > 64
                        || rule.excludes.len() > 64
                        || rule.includes.iter().chain(&rule.excludes).any(|pattern| pattern.len() > 1024)
                })
            {
                return Err(serde::de::Error::custom("Invalid Redis grouping configuration"));
            }
        }
        Ok(data.into())
    }
}

fn migrate_legacy_transport_layers(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    if object.get("transport_layers").and_then(Value::as_array).is_some_and(|layers| !layers.is_empty()) {
        return;
    }

    let mut layers = Vec::new();
    let ssh_enabled = object.get("ssh_enabled").and_then(Value::as_bool).unwrap_or(false);
    if ssh_enabled {
        if let Some(ssh_tunnels) = object.get("ssh_tunnels").and_then(Value::as_array) {
            for hop in ssh_tunnels {
                let mut layer = hop.clone();
                if let Some(layer_object) = layer.as_object_mut() {
                    layer_object.insert("type".to_string(), Value::String("ssh".to_string()));
                }
                layers.push(layer);
            }
        }

        if layers.is_empty() && string_field(object, "ssh_host").is_some() {
            let mut layer = serde_json::Map::new();
            layer.insert("type".to_string(), Value::String("ssh".to_string()));
            layer.insert("id".to_string(), Value::String("legacy".to_string()));
            layer.insert("enabled".to_string(), Value::Bool(true));
            copy_string(object, &mut layer, "ssh_host", "host");
            copy_u64(object, &mut layer, "ssh_port", "port", default_ssh_port() as u64);
            copy_string(object, &mut layer, "ssh_user", "user");
            copy_string(object, &mut layer, "ssh_password", "password");
            copy_string(object, &mut layer, "ssh_key_path", "key_path");
            copy_string(object, &mut layer, "ssh_key_passphrase", "key_passphrase");
            copy_u64(
                object,
                &mut layer,
                "ssh_connect_timeout_secs",
                "connect_timeout_secs",
                default_ssh_connect_timeout_secs(),
            );
            copy_bool(object, &mut layer, "ssh_expose_lan", "expose_lan");
            layers.push(Value::Object(layer));
        }
    }

    let proxy_enabled = object.get("proxy_enabled").and_then(Value::as_bool).unwrap_or(false);
    if proxy_enabled && string_field(object, "proxy_host").is_some() {
        let mut layer = serde_json::Map::new();
        layer.insert("type".to_string(), Value::String("proxy".to_string()));
        layer.insert("id".to_string(), Value::String("legacy-proxy".to_string()));
        layer.insert("enabled".to_string(), Value::Bool(true));
        copy_string(object, &mut layer, "proxy_type", "proxy_type");
        copy_string(object, &mut layer, "proxy_host", "host");
        copy_u64(object, &mut layer, "proxy_port", "port", default_proxy_port() as u64);
        copy_string(object, &mut layer, "proxy_username", "username");
        copy_string(object, &mut layer, "proxy_password", "password");
        layers.push(Value::Object(layer));
    }

    if !layers.is_empty() {
        object.insert("transport_layers".to_string(), Value::Array(layers));
    }
}

fn string_field(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::trim).filter(|value| !value.is_empty()).map(str::to_string)
}

fn copy_string(
    object: &serde_json::Map<String, Value>,
    target: &mut serde_json::Map<String, Value>,
    from: &str,
    to: &str,
) {
    if let Some(value) = object.get(from).and_then(Value::as_str) {
        target.insert(to.to_string(), Value::String(value.to_string()));
    }
}

fn copy_bool(
    object: &serde_json::Map<String, Value>,
    target: &mut serde_json::Map<String, Value>,
    from: &str,
    to: &str,
) {
    if let Some(value) = object.get(from).and_then(Value::as_bool) {
        target.insert(to.to_string(), Value::Bool(value));
    }
}

fn copy_u64(
    object: &serde_json::Map<String, Value>,
    target: &mut serde_json::Map<String, Value>,
    from: &str,
    to: &str,
    default: u64,
) {
    let value = object.get(from).and_then(Value::as_u64).filter(|value| *value > 0).unwrap_or(default);
    target.insert(to.to_string(), Value::Number(value.into()));
}

impl ConnectionConfig {
    pub fn effective_transport_layers(&self) -> Vec<TransportLayerConfig> {
        self.transport_layers.iter().filter(|layer| layer.enabled()).cloned().collect()
    }

    pub fn effective_ssh_tunnels(&self) -> Vec<SshTunnelConfig> {
        self.effective_transport_layers()
            .into_iter()
            .filter_map(|layer| match layer {
                TransportLayerConfig::Ssh(ssh) => Some(ssh),
                TransportLayerConfig::Proxy(_) | TransportLayerConfig::HttpTunnel(_) => None,
            })
            .collect()
    }

    pub fn has_effective_transport_layers(&self) -> bool {
        !self.effective_transport_layers().is_empty()
    }

    pub fn has_effective_ssh_tunnels(&self) -> bool {
        self.effective_transport_layers().iter().any(|layer| matches!(layer, TransportLayerConfig::Ssh(_)))
    }

    pub fn effective_connect_timeout_secs(&self) -> u64 {
        if self.connect_timeout_secs == 0 {
            default_connect_timeout_secs()
        } else {
            self.connect_timeout_secs.clamp(1, 300)
        }
    }

    pub fn effective_query_timeout_secs(&self) -> u64 {
        if self.query_timeout_secs == 0 {
            // An explicit 0 is the UI's "no limit"; a floor must not impose one.
            return 0;
        }
        let floor = { 1 };
        self.query_timeout_secs.max(floor)
    }

    pub fn effective_database(&self) -> Option<&str> {
        self.database.as_deref().filter(|database| !database.trim().is_empty()).or_else(|| self.default_database())
    }

    fn default_database(&self) -> Option<&'static str> {
        match self.db_type {
            _ => None,
        }
    }

    pub fn needs_bare_mysql(&self) -> bool {
        false
    }

    pub fn bare_mysql_supports_tls(&self) -> bool {
        false
    }

    pub fn bare_mysql_uses_tls(&self) -> bool {
        false
    }

    pub fn canonicalized(&self) -> Self {
        self.clone()
    }

    pub fn connection_url(&self) -> String {
        self.connection_url_with_host(&self.host, self.port)
    }

    pub fn redacted_connection_url(&self) -> String {
        self.redacted_connection_url_with_host(&self.host, self.port)
    }

    pub fn redacted_connection_url_with_host(&self, host: &str, port: u16) -> String {
        let raw_host = host;
        let host = bracket_ipv6(host);
        let db_part = self.effective_database().map(|d| format!("/{}", encode_url_part(d))).unwrap_or_default();
        let params = self.redacted_url_params();

        match self.db_type {
            DatabaseType::Mysql => {
                let suffix = if params.is_empty() { String::new() } else { format!("?{params}") };
                format!("mysql://{host}:{port}{db_part}{suffix}")
            }
        }
    }

    pub fn connection_url_with_host(&self, host: &str, port: u16) -> String {
        let raw_host = host;
        let host = bracket_ipv6(host);
        let db_part = self.effective_database().map(|d| format!("/{}", encode_url_part(d))).unwrap_or_default();
        let username = encode_url_part(&self.username);
        let password = encode_url_part(&self.password);
        let params = self.normalized_url_params();

        match self.db_type {
            DatabaseType::Mysql => {
                let suffix = if params.is_empty() { String::new() } else { format!("?{params}") };
                format!("mysql://{}:{}@{host}:{port}{db_part}{suffix}", username, password)
            }
        }
    }

    fn normalized_url_params(&self) -> String {
        let value = self.url_params.as_deref().unwrap_or("").trim();
        match self.db_type {
            DatabaseType::Mysql => {
                normalize_mysql_url_params(value, self.mysql_uses_tls(), self.ca_cert_path.trim().is_empty())
            }

            _ => value.trim_start_matches('?').to_string(),
        }
    }

    fn redacted_url_params(&self) -> String {
        let params = self.normalized_url_params();
        {
            params
        }
    }

    pub fn validate_native_url_params(&self) -> Result<(), String> {
        {
            Ok(())
        }
    }

    pub fn mysql_uses_tls(&self) -> bool {
        self.ssl
            || self.host.to_ascii_lowercase().ends_with(".tidbcloud.com")
            || mysql_url_params_require_tls(self.url_params.as_deref())
    }
}

fn sqlserver_legacy_compatibility_param(params: Option<&str>) -> bool {
    false
}

fn mysql_tls_file_param_is(key: &str, target: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    normalized == format!("ssl{target}")
}

fn mysql_url_params_require_tls(params: Option<&str>) -> bool {
    let params = params.unwrap_or("").trim().trim_start_matches('?');
    let has_native_tls_param = params.split('&').any(|part| {
        url_param_key_is(part, "ssl-mode") || url_param_key_is(part, "sslmode") || url_param_key_is(part, "require_ssl")
    });
    let native_requires_tls = params.split('&').any(|part| {
        let part = part.trim();
        if part.is_empty() {
            return false;
        }
        let Some((key, value)) = part.split_once('=') else {
            return mysql_tls_file_param_is(part, "cert") || mysql_tls_file_param_is(part, "key");
        };
        let key = key.trim();
        let value = value.trim();
        (key.eq_ignore_ascii_case("require_ssl") && value.eq_ignore_ascii_case("true"))
            || mysql_tls_file_param_is(key, "cert")
            || mysql_tls_file_param_is(key, "key")
            || ((key.eq_ignore_ascii_case("ssl-mode") || key.eq_ignore_ascii_case("sslmode"))
                && matches!(
                    value.to_ascii_lowercase().replace('-', "_").as_str(),
                    "required" | "require" | "verify_ca" | "verify_identity"
                ))
    });
    native_requires_tls
        || (!has_native_tls_param
            && matches!(
                mysql_jdbc_tls_mode(Some(params)),
                Some(MysqlJdbcTlsMode::Required | MysqlJdbcTlsMode::VerifyCa)
            ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MysqlJdbcTlsMode {
    Disabled,
    Preferred,
    Required,
    VerifyCa,
}

pub fn is_mysql_jdbc_tls_param(key: &str) -> bool {
    matches!(
        percent_decode_str(key).decode_utf8_lossy().trim().to_ascii_lowercase().as_str(),
        "usessl" | "requiressl" | "verifyservercertificate"
    )
}

pub fn mysql_jdbc_tls_mode(params: Option<&str>) -> Option<MysqlJdbcTlsMode> {
    let mut has_jdbc_tls_param = false;
    let mut use_ssl = None;
    let mut require_ssl = None;
    let mut verify_server_certificate = None;

    for part in params.unwrap_or("").trim().trim_start_matches('?').split('&') {
        let Some((raw_key, raw_value)) = part.split_once('=') else {
            continue;
        };
        let key = percent_decode_str(raw_key).decode_utf8_lossy().trim().to_ascii_lowercase();
        let value = percent_decode_str(raw_value).decode_utf8_lossy();
        let parsed_value = mysql_url_param_value_is_true(&value);
        match key.as_str() {
            "usessl" => {
                has_jdbc_tls_param = true;
                use_ssl = Some(parsed_value);
            }
            "requiressl" => {
                has_jdbc_tls_param = true;
                require_ssl = Some(parsed_value);
            }
            "verifyservercertificate" => {
                has_jdbc_tls_param = true;
                verify_server_certificate = Some(parsed_value);
            }
            _ => {}
        }
    }

    if !has_jdbc_tls_param {
        None
    } else if use_ssl == Some(false) {
        Some(MysqlJdbcTlsMode::Disabled)
    } else if verify_server_certificate == Some(true) {
        Some(MysqlJdbcTlsMode::VerifyCa)
    } else if require_ssl == Some(true) {
        Some(MysqlJdbcTlsMode::Required)
    } else {
        Some(MysqlJdbcTlsMode::Preferred)
    }
}

fn is_mysql_cleartext_password_param(key: &str) -> bool {
    matches!(key.to_ascii_lowercase().as_str(), "allowcleartextpasswords" | "enable_cleartext_plugin")
}

fn mysql_url_param_value_is_true(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
}

fn normalize_mysql_url_params(value: &str, force_tls: bool, accept_invalid_certs: bool) -> String {
    let value = value.trim_start_matches('?');
    let mut parts: Vec<String> = value.split('&').filter(|part| !part.is_empty()).map(str::to_string).collect();
    let jdbc_tls_mode = mysql_jdbc_tls_mode(Some(value));
    let has_native_tls_param = parts.iter().any(|part| {
        url_param_key_is(part, "ssl-mode") || url_param_key_is(part, "sslmode") || url_param_key_is(part, "require_ssl")
    });
    let enable_cleartext_plugin = parts.iter().any(|part| {
        let Some((key, value)) = part.split_once('=') else {
            return false;
        };
        is_mysql_cleartext_password_param(key.trim()) && mysql_url_param_value_is_true(value)
    });

    parts.retain(|part| {
        let Some((key, _)) = part.split_once('=') else {
            return true;
        };
        !is_mysql_cleartext_password_param(key.trim()) && !is_mysql_jdbc_tls_param(key.trim())
    });

    if !has_native_tls_param {
        match jdbc_tls_mode {
            Some(MysqlJdbcTlsMode::Disabled) => parts.push("ssl-mode=disabled".to_string()),
            Some(MysqlJdbcTlsMode::Preferred) => parts.push("ssl-mode=preferred".to_string()),
            Some(MysqlJdbcTlsMode::Required) => {
                parts.push("require_ssl=true".to_string());
                parts.push("verify_ca=false".to_string());
                parts.push("verify_identity=false".to_string());
            }
            Some(MysqlJdbcTlsMode::VerifyCa) => {
                parts.push("require_ssl=true".to_string());
                parts.push("verify_ca=true".to_string());
                parts.push("verify_identity=false".to_string());
            }
            None => {}
        }
    }

    if force_tls {
        parts.retain(|part| {
            !url_param_key_is(part, "ssl-mode")
                && !url_param_key_is(part, "sslmode")
                && !url_param_key_is(part, "require_ssl")
        });
        parts.insert(0, "require_ssl=true".to_string());
        if accept_invalid_certs && !parts.iter().any(|part| url_param_key_is(part, "verify_ca")) {
            parts.push("verify_ca=false".to_string());
        }
        if !parts.iter().any(|part| url_param_key_is(part, "verify_identity")) {
            parts.push("verify_identity=false".to_string());
        }
    } else if !parts.iter().any(|part| {
        url_param_key_is(part, "ssl-mode") || url_param_key_is(part, "sslmode") || url_param_key_is(part, "require_ssl")
    }) {
        // Default MySQL connections keep TLS off unless the user explicitly
        // enables a TLS mode.
        parts.insert(0, "ssl-mode=disabled".to_string());
    }

    if !parts.iter().any(|part| url_param_key_is(part, "charset")) {
        parts.push("charset=utf8mb4".to_string());
    }
    if enable_cleartext_plugin {
        parts.push("enable_cleartext_plugin=true".to_string());
    }

    parts.join("&")
}

fn url_param_key_is(part: &str, expected: &str) -> bool {
    let key = part.split_once('=').map(|(key, _)| key).unwrap_or(part);
    percent_decode_str(key).decode_utf8_lossy().eq_ignore_ascii_case(expected)
}

pub fn parse_mongo_first_host(uri: &str) -> Option<(String, u16)> {
    let rest = uri.strip_prefix("mongodb://").or_else(|| uri.strip_prefix("mongodb+srv://"))?;
    let authority = rest.split('/').next()?;
    let host_section = match authority.rfind('@') {
        Some(idx) => &authority[idx + 1..],
        None => authority,
    };
    let first = host_section.split(',').next()?;
    match first.rsplit_once(':') {
        Some((h, p)) => Some((h.to_string(), p.parse().unwrap_or(27017))),
        None => Some((first.to_string(), 27017)),
    }
}

#[derive(Debug)]
enum JdbcTransportEndpoint {
    Standard { host: String, port: u16 },
    As400(As400JdbcEndpoint),
}

#[derive(Debug)]
struct As400JdbcEndpoint {
    host: String,
    port: u16,
    host_range: std::ops::Range<usize>,
    port_range: std::ops::Range<usize>,
}

#[derive(Debug)]
struct As400JdbcProperty<'a> {
    key: &'a str,
    value: &'a str,
    value_range: std::ops::Range<usize>,
}

fn encode_url_part(value: &str) -> String {
    utf8_percent_encode(value, NON_ALPHANUMERIC).to_string()
}

/// Percent-encode set for Spanner resource paths: everything [`NON_ALPHANUMERIC`]
/// escapes, minus the RFC 3986 *unreserved* characters, which must stay literal.
///
/// `-` matters most in practice: GCP project and instance IDs allow only lowercase
/// letters, digits and hyphens, so without this a hyphenated project would render as
/// `my%2Dproject` in every Spanner connection.
const SPANNER_PATH_ENCODE_SET: &percent_encoding::AsciiSet =
    &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.').remove(b'~');

fn bracket_ipv6(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

#[cfg(test)]
mod tests {

    use super::{
        database_info_from_protocol_value, default_query_timeout_secs, default_redis_key_separator,
        default_ssh_connect_timeout_secs, sqlserver_legacy_compatibility_param, ConnectionConfig, ConnectionTestResult,
        DatabaseConnectionInfo, DatabaseType, IdentifierCase, ProxyTunnelConfig, ProxyType, TransportLayerConfig,
    };

    #[test]
    fn default_query_timeout_is_sixty_seconds() {
        assert_eq!(default_query_timeout_secs(), 60);
    }

    #[test]
    fn connection_test_result_uses_camel_case_and_omits_missing_details() {
        let result =
            ConnectionTestResult::success("Connection successful").with_database_info(Some(DatabaseConnectionInfo {
                product_name: Some("H2".to_string()),
                unquoted_identifier_case: Some(IdentifierCase::Upper),
                ..DatabaseConnectionInfo::default()
            }));

        let value = serde_json::to_value(result).unwrap();
        assert_eq!(value["message"], "Connection successful");
        assert_eq!(value["databaseInfo"]["productName"], "H2");
        assert_eq!(value["databaseInfo"]["unquotedIdentifierCase"], "upper");
        assert!(value["databaseInfo"].get("driverName").is_none());
    }

    #[test]
    fn protocol_database_info_parser_accepts_details_and_legacy_responses() {
        let parsed = database_info_from_protocol_value(&serde_json::json!({
            "ok": true,
            "databaseInfo": {
                "productName": "H2",
                "currentDatabase": "app",
                "serverCharset": "utf8mb4",
                "jdbcVersion": "4.2"
            }
        }))
        .unwrap();
        assert_eq!(parsed.product_name.as_deref(), Some("H2"));
        assert_eq!(parsed.current_database.as_deref(), Some("app"));
        assert_eq!(parsed.server_charset.as_deref(), Some("utf8mb4"));
        assert_eq!(parsed.jdbc_version.as_deref(), Some("4.2"));
        assert_eq!(database_info_from_protocol_value(&serde_json::json!({ "ok": true })), None);
    }

    #[test]
    fn default_schema_is_optional_and_round_trips() {
        let base = serde_json::json!({
            "id": "id",
            "name": "PostgreSQL",
            "db_type": "mysql",
            "host": "localhost",
            "port": 5432,
            "username": "postgres",
            "password": "",
            "database": "app"
        });
        let legacy: ConnectionConfig = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(legacy.default_schema, None);

        let mut configured = base;
        configured["default_schema"] = serde_json::json!("archive");
        let parsed: ConnectionConfig = serde_json::from_value(configured).unwrap();
        assert_eq!(parsed.default_schema.as_deref(), Some("archive"));
        assert_eq!(serde_json::to_value(parsed).unwrap()["default_schema"], "archive");
    }

    #[test]
    fn sidebar_auto_load_all_tables_defaults_off_and_round_trips() {
        let mut value = serde_json::json!({
            "id": "id",
            "name": "MariaDB",
            "db_type": "mysql",
            "host": "localhost",
            "port": 3306,
            "username": "root",
            "password": "",
            "database": "app"
        });
        let legacy: ConnectionConfig = serde_json::from_value(value.clone()).unwrap();
        assert!(!legacy.sidebar_auto_load_all_tables);
        let serialized_legacy = serde_json::to_value(legacy).unwrap();
        assert!(serialized_legacy.get("sidebar_auto_load_all_tables").is_none());

        value["sidebar_auto_load_all_tables"] = serde_json::json!(true);
        let configured: ConnectionConfig = serde_json::from_value(value).unwrap();
        assert!(configured.sidebar_auto_load_all_tables);
        assert!(serde_json::to_value(configured).unwrap()["sidebar_auto_load_all_tables"].as_bool().unwrap());
    }

    fn mysql_config(username: &str, password: &str, database: Option<&str>) -> ConnectionConfig {
        ConnectionConfig {
            oracle_oci_nls_lang: None,
            oracle_oci_tns_admin: None,
            docs_notes_path: None,
            id: "id".to_string(),
            name: "name".to_string(),
            note: String::new(),
            db_type: DatabaseType::Mysql,
            driver_profile: None,
            driver_label: None,
            url_params: None,
            agent_java_options: Vec::new(),
            host: "10.1.2.3".to_string(),
            port: 2883,
            username: username.to_string(),
            password: password.to_string(),
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
            connect_timeout_secs: super::default_connect_timeout_secs(),
            query_timeout_secs: default_query_timeout_secs(),
            idle_timeout_secs: super::default_idle_timeout_secs(),
            keepalive_interval_secs: super::default_keepalive_interval_secs(),
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
    fn connection_debug_redacts_passwords_tokens_and_nested_secrets() {
        let mut config = mysql_config("root", "database-password", Some("app"));
        config.connection_string = Some("server=db;password=inline-secret".into());
        config.init_script = Some("CREATE SECRET leaked".into());
        config.external_config = Some(serde_json::json!({
            "consulToken": "consul-secret",
            "nested": { "OIDCClientSecret": "oidc-secret", "visible": "safe" }
        }));

        let output = format!("{config:?}");
        for secret in ["database-password", "inline-secret", "CREATE SECRET leaked", "consul-secret", "oidc-secret"] {
            assert!(!output.contains(secret));
        }
        assert!(output.contains("[REDACTED]"));
        assert!(output.contains("safe"));
    }

    #[test]
    fn docs_notes_path_defaults_to_none_and_round_trips() {
        // A connection stored before this field existed must still load.
        let legacy: ConnectionConfig = serde_json::from_value(serde_json::json!({
            "id": "c1",
            "name": "local",
            "db_type": "mysql",
            "host": "127.0.0.1",
            "port": 5432,
            "username": "postgres",
            "password": "",
            "database": null
        }))
        .expect("legacy config must still parse");
        assert_eq!(legacy.docs_notes_path, None);

        // And a stored path must actually survive a load — this is the half
        // that fails if the field is added to ConnectionConfig only, without
        // ConnectionConfigData and the From impl.
        let with_path: ConnectionConfig = serde_json::from_value(serde_json::json!({
            "id": "c1",
            "name": "local",
            "db_type": "mysql",
            "host": "127.0.0.1",
            "port": 5432,
            "username": "postgres",
            "password": "",
            "database": null,
            "docs_notes_path": "docs/dbx-docs.json"
        }))
        .expect("config with a notes path must parse");
        assert_eq!(with_path.docs_notes_path.as_deref(), Some("docs/dbx-docs.json"));
    }

    #[test]
    fn connection_note_is_optional_and_round_trips_when_present() {
        let config = mysql_config("root", "secret", Some("app"));
        let value = serde_json::to_value(&config).unwrap();
        assert!(value.get("note").is_none());
        assert!(serde_json::from_value::<ConnectionConfig>(value).unwrap().note.is_empty());

        let mut config = config;
        config.note = "Production reporting".to_string();
        let value = serde_json::to_value(&config).unwrap();
        assert_eq!(value["note"], "Production reporting");
        assert_eq!(serde_json::from_value::<ConnectionConfig>(value).unwrap().note, config.note);
    }

    #[test]
    fn connection_config_database_info_survives_json_round_trip() {
        let mut config = mysql_config("root", "secret", Some("app"));
        config.database_info = Some(DatabaseConnectionInfo {
            product_name: Some("MySQL".to_string()),
            product_version: Some("8.4.0".to_string()),
            current_database: Some("app".to_string()),
            server_charset: Some("utf8mb4".to_string()),
            ..DatabaseConnectionInfo::default()
        });

        let value = serde_json::to_value(&config).unwrap();
        assert_eq!(value["database_info"]["productVersion"], "8.4.0");

        let restored: ConnectionConfig = serde_json::from_value(value).unwrap();
        assert_eq!(restored.database_info, config.database_info);
    }

    #[test]
    fn legacy_single_ssh_config_migrates_to_transport_layer() {
        let config: ConnectionConfig = serde_json::from_value(serde_json::json!({
            "id": "id",
            "name": "name",
            "db_type": "mysql",
            "host": "10.1.2.3",
            "port": 3306,
            "username": "root",
            "password": "",
            "database": null,
            "ssh_enabled": true,
            "ssh_host": "bastion.example.com",
            "ssh_port": 2200,
            "ssh_user": "deploy",
            "ssh_password": "secret",
            "ssh_connect_timeout_secs": 0,
            "ssh_expose_lan": true
        }))
        .unwrap();

        let hops = config.effective_ssh_tunnels();
        assert_eq!(hops.len(), 1);
        assert_eq!(hops[0].id, "legacy");
        assert_eq!(hops[0].host, "bastion.example.com");
        assert_eq!(hops[0].port, 2200);
        assert_eq!(hops[0].user, "deploy");
        assert_eq!(hops[0].password, "secret");
        assert_eq!(hops[0].connect_timeout_secs, default_ssh_connect_timeout_secs());
        assert!(hops[0].expose_lan);
    }

    #[test]
    fn missing_connection_timeout_defaults_to_ten_seconds() {
        let config: ConnectionConfig = serde_json::from_value(serde_json::json!({
            "id": "id",
            "name": "name",
            "db_type": "mysql",
            "host": "10.1.2.3",
            "port": 3306,
            "username": "root",
            "password": "",
            "database": null
        }))
        .unwrap();

        assert_eq!(config.connect_timeout_secs, 10);
        assert_eq!(config.effective_connect_timeout_secs(), 10);
    }

    #[test]
    fn legacy_ssh_tunnels_migrate_to_ordered_transport_layers() {
        let config: ConnectionConfig = serde_json::from_value(serde_json::json!({
            "id": "id",
            "name": "name",
            "db_type": "mysql",
            "host": "10.1.2.3",
            "port": 3306,
            "username": "root",
            "password": "",
            "database": null,
            "ssh_enabled": true,
            "ssh_tunnels": [
                { "id": "first", "host": "a", "port": 22, "user": "u" },
                { "id": "second", "host": "b", "port": 2200, "user": "u" }
            ]
        }))
        .unwrap();

        let hops = config.effective_ssh_tunnels();
        assert_eq!(hops.iter().map(|hop| hop.id.as_str()).collect::<Vec<_>>(), vec!["first", "second"]);
    }

    #[test]
    fn legacy_proxy_config_migrates_to_transport_layer() {
        let config: ConnectionConfig = serde_json::from_value(serde_json::json!({
            "id": "id",
            "name": "name",
            "db_type": "mysql",
            "host": "10.1.2.3",
            "port": 3306,
            "username": "root",
            "password": "",
            "database": null,
            "proxy_enabled": true,
            "proxy_type": "http",
            "proxy_host": "proxy.example.com",
            "proxy_port": 8080,
            "proxy_username": "alice",
            "proxy_password": "secret"
        }))
        .unwrap();

        assert_eq!(config.transport_layers.len(), 1);
        match &config.transport_layers[0] {
            TransportLayerConfig::Proxy(proxy) => {
                assert_eq!(proxy.id, "legacy-proxy");
                assert_eq!(proxy.proxy_type, ProxyType::Http);
                assert_eq!(proxy.host, "proxy.example.com");
                assert_eq!(proxy.port, 8080);
                assert_eq!(proxy.username, "alice");
                assert_eq!(proxy.password, "secret");
            }
            _ => panic!("expected proxy layer"),
        }
    }

    #[test]
    fn existing_transport_layers_take_precedence_over_legacy_fields() {
        let config: ConnectionConfig = serde_json::from_value(serde_json::json!({
            "id": "id",
            "name": "name",
            "db_type": "mysql",
            "host": "10.1.2.3",
            "port": 3306,
            "username": "root",
            "password": "",
            "database": null,
            "ssh_enabled": true,
            "ssh_host": "legacy.example.com",
            "transport_layers": [{ "type": "proxy", "id": "proxy", "host": "proxy", "port": 1080 }]
        }))
        .unwrap();

        assert_eq!(config.transport_layers.len(), 1);
        assert!(matches!(&config.transport_layers[0], TransportLayerConfig::Proxy(proxy) if proxy.id == "proxy"));
    }

    #[test]
    fn serialized_connection_config_omits_legacy_transport_fields() {
        let mut config = mysql_config("root", "", None);
        config.transport_layers = vec![TransportLayerConfig::Proxy(ProxyTunnelConfig {
            profile_id: String::new(),
            id: "proxy".to_string(),
            name: String::new(),
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: "proxy".to_string(),
            port: 1080,
            username: String::new(),
            password: String::new(),
            test_target: None,
        })];

        let saved = serde_json::to_value(config).unwrap();

        assert!(saved.get("transport_layers").is_some());
        for key in ["ssh_tunnels", "ssh_host", "ssh_password", "proxy_host", "proxy_password", "proxy_enabled"] {
            assert!(saved.get(key).is_none(), "legacy key {key} should not serialize");
        }
    }

    #[test]
    fn query_timeout_zero_disables_timeout() {
        let mut config = mysql_config("root", "", None);
        config.query_timeout_secs = 0;

        assert_eq!(config.effective_query_timeout_secs(), 0);
    }

    #[test]
    fn query_timeout_preserves_long_running_exports() {
        let mut config = mysql_config("root", "", None);
        config.query_timeout_secs = 3600;

        assert_eq!(config.effective_query_timeout_secs(), 3600);
    }

    #[test]
    fn mysql_url_encodes_proxy_style_username_with_colons_and_at() {
        let config = mysql_config("1234:xxx@xx.xx:abc123", "p@ss:word", None);

        assert_eq!(
            config.connection_url(),
            "mysql://1234%3Axxx%40xx%2Exx%3Aabc123:p%40ss%3Aword@10.1.2.3:2883?ssl-mode=disabled&charset=utf8mb4"
        );
    }

    #[test]
    fn mysql_url_encodes_password_and_database() {
        let config = mysql_config("root", "p@ss:word#1", Some("db/name"));

        assert_eq!(
            config.connection_url(),
            "mysql://root:p%40ss%3Aword%231@10.1.2.3:2883/db%2Fname?ssl-mode=disabled&charset=utf8mb4"
        );
    }

    #[test]
    fn mysql_url_appends_custom_params() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params = Some("charset=utf8mb4".to_string());

        assert_eq!(config.connection_url(), "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=disabled&charset=utf8mb4");
    }

    #[test]
    fn mysql_cleartext_password_auth_alias_normalizes_to_driver_param() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params = Some("allowCleartextPasswords=true".to_string());

        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=disabled&charset=utf8mb4&enable_cleartext_plugin=true"
        );
    }

    #[test]
    fn mysql_cleartext_password_auth_keeps_canonical_driver_param() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params = Some("enable_cleartext_plugin=true".to_string());

        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=disabled&charset=utf8mb4&enable_cleartext_plugin=true"
        );
    }

    #[test]
    fn mysql_cleartext_password_auth_deduplicates_aliases() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params =
            Some("allowCleartextPasswords=true&enable_cleartext_plugin=true&charset=utf8mb4".to_string());

        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=disabled&charset=utf8mb4&enable_cleartext_plugin=true"
        );
    }

    #[test]
    fn mysql_cleartext_password_auth_omits_disabled_values() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params = Some("allowCleartextPasswords=false&enable_cleartext_plugin=&charset=utf8mb4".to_string());

        assert_eq!(config.connection_url(), "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=disabled&charset=utf8mb4");
    }

    #[test]
    fn mysql_tls_switch_requires_ssl_without_strict_certificate_checks_by_default() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.ssl = true;

        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?require_ssl=true&verify_ca=false&verify_identity=false&charset=utf8mb4"
        );
    }

    #[test]
    fn mysql_explicit_preferred_tls_mode_is_preserved() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params = Some("ssl-mode=preferred".to_string());

        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=preferred&charset=utf8mb4"
        );
    }

    #[test]
    fn mysql_connector_j_tls_params_are_normalized() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params = Some("useSSL=true&requireSSL=true&verifyServerCertificate=true".to_string());

        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?require_ssl=true&verify_ca=true&verify_identity=false&charset=utf8mb4"
        );
    }

    #[test]
    fn mysql_connector_j_preferred_and_disabled_tls_modes_are_preserved() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params = Some("useSSL=true".to_string());
        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=preferred&charset=utf8mb4"
        );

        config.url_params = Some("useSSL=false".to_string());
        assert_eq!(config.connection_url(), "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=disabled&charset=utf8mb4");
    }

    #[test]
    fn mysql_connector_j_tls_truth_table_preserves_legacy_precedence() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params = Some("verifyServerCertificate=true".to_string());
        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?require_ssl=true&verify_ca=true&verify_identity=false&charset=utf8mb4"
        );

        config.url_params = Some("useSSL=false&requireSSL=true&verifyServerCertificate=true".to_string());
        assert_eq!(config.connection_url(), "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=disabled&charset=utf8mb4");

        config.url_params = Some("requireSSL=false".to_string());
        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=preferred&charset=utf8mb4"
        );
    }

    #[test]
    fn mysql_connector_j_duplicate_tls_params_use_the_last_value() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params = Some("useSSL=false&useSSL=true".to_string());
        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=preferred&charset=utf8mb4"
        );

        config.url_params = Some("useSSL=true&useSSL=false&verifyServerCertificate=true".to_string());
        assert_eq!(config.connection_url(), "mysql://root:secret@10.1.2.3:2883/test?ssl-mode=disabled&charset=utf8mb4");
    }

    #[test]
    fn mysql_native_tls_mode_takes_precedence_over_connector_j_aliases() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.url_params = Some("sslMode=REQUIRED&useSSL=false".to_string());

        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?require_ssl=true&verify_ca=false&verify_identity=false&charset=utf8mb4"
        );
    }

    #[test]
    fn mysql_tls_switch_uses_ca_cert_for_ca_validation() {
        let mut config = mysql_config("root", "secret", Some("test"));
        config.ssl = true;
        config.ca_cert_path = "/tmp/tidb-ca.pem".to_string();

        assert_eq!(
            config.connection_url(),
            "mysql://root:secret@10.1.2.3:2883/test?require_ssl=true&verify_identity=false&charset=utf8mb4"
        );
    }

    #[test]
    fn redacted_mysql_url_omits_credentials() {
        let config = mysql_config("user@tenant#cluster", "p@ss:word#1", Some("db/name"));

        let url = config.redacted_connection_url();

        assert_eq!(url, "mysql://10.1.2.3:2883/db%2Fname?ssl-mode=disabled&charset=utf8mb4");
        assert!(!url.contains("user"));
        assert!(!url.contains("p%40ss"));
        assert!(!url.contains("p@ss"));
    }
}

fn is_default_redis_separator(value: &str) -> bool {
    value == ":"
}
