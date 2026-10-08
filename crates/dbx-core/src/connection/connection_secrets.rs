use crate::models::connection::{ConnectionConfig, DatabaseType, TransportLayerConfig};
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const MAIN_PASSWORD_KEY: &str = "password";
pub const SSH_PASSWORD_KEY: &str = "ssh_password";
pub const SSH_KEY_PASSPHRASE_KEY: &str = "ssh_key_passphrase";
pub const SSH_TUNNEL_SECRET_PREFIX: &str = "ssh_tunnels.";
pub const TRANSPORT_LAYER_SECRET_PREFIX: &str = "transport_layers.";
pub const PROXY_PASSWORD_KEY: &str = "proxy_password";
pub const REDIS_SENTINEL_PASSWORD_KEY: &str = "redis_sentinel_password";
pub const CONNECTION_STRING_KEY: &str = "connection_string";
pub const INIT_SCRIPT_KEY: &str = "init_script";
pub const MQ_AUTH_SECRET_PREFIX: &str = "mq.auth.";
pub const MQ_AUTH_TOKEN_KEY: &str = "mq.auth.token";
pub const MQ_AUTH_PASSWORD_KEY: &str = "mq.auth.password";
pub const MQ_AUTH_API_KEY_VALUE_KEY: &str = "mq.auth.api_key_value";
pub const MQ_AUTH_CLIENT_SECRET_KEY: &str = "mq.auth.client_secret";
pub const MQ_TOKEN_SIGNING_SECRET_PREFIX: &str = "mq.token_signing.";
pub const MQ_TOKEN_SIGNING_KEY: &str = "mq.token_signing.key";
pub const NACOS_AUTH_SECRET_PREFIX: &str = "nacos.auth.";
pub const NACOS_AUTH_PASSWORD_KEY: &str = "nacos.auth.password";
pub const NACOS_RNACOS_CONSOLE_PASSWORD_KEY: &str = "nacos.auth.rnacos_console_password";
pub const MQTT_AUTH_SECRET_PREFIX: &str = "mqtt.auth.";
pub const MQTT_AUTH_PASSWORD_KEY: &str = "mqtt.auth.password";
pub const CASSANDRA_TLS_SECRET_PREFIX: &str = "cassandra.tls.";
pub const CASSANDRA_TRUSTSTORE_PASSWORD_KEY: &str = "cassandra.tls.truststore_password";
pub const CASSANDRA_KEYSTORE_PASSWORD_KEY: &str = "cassandra.tls.keystore_password";
pub const SALESFORCE_AUTH_SECRET_PREFIX: &str = "salesforce.auth.";
pub const SALESFORCE_AUTH_CLIENT_SECRET_KEY: &str = "salesforce.auth.client_secret";
pub const SALESFORCE_AUTH_REFRESH_TOKEN_KEY: &str = "salesforce.auth.refresh_token";
pub const SALESFORCE_AUTH_PASSWORD_KEY: &str = "salesforce.auth.password";
pub const PLUGIN_CONNECTION_SECRET_PREFIX: &str = "plugin_connection.";

/// Storage-level secret key for one plugin connection field: namespaced under
/// [`PLUGIN_CONNECTION_SECRET_PREFIX`] so plugin keys never collide with the
/// fixed driver keys and can be scrubbed with a single prefix delete.
pub fn plugin_connection_secret_key(key: &str) -> Result<String, String> {
    if key.is_empty() {
        return Err("Plugin connection secret key must not be empty".to_string());
    }
    Ok(format!("{PLUGIN_CONNECTION_SECRET_PREFIX}{key}"))
}

pub trait ConnectionSecretStore {
    fn set_secret(&self, connection_id: &str, key: &str, secret: &str) -> Result<(), String>;
    fn get_secret(&self, connection_id: &str, key: &str) -> Result<Option<String>, String>;
    fn delete_secret(&self, connection_id: &str, key: &str) -> Result<(), String>;
    fn delete_secret_prefix(&self, _connection_id: &str, _key_prefix: &str) -> Result<(), String> {
        Ok(())
    }
}

pub struct FileSecretStore {
    path: PathBuf,
}

impl FileSecretStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn read_store(&self) -> HashMap<String, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(map) => map,
                Err(e) => {
                    log::warn!(
                        "Failed to parse secret store at {:?}: {}. Returning empty store. This may indicate file corruption.",
                        self.path, e
                    );
                    HashMap::default()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::default(),
            Err(e) => {
                log::warn!("Failed to read secret store at {:?}: {}. Returning empty store.", self.path, e);
                HashMap::default()
            }
        }
    }

    fn write_store(&self, map: &HashMap<String, String>) -> Result<(), String> {
        let json = serde_json::to_string_pretty(map).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())
    }
}

impl ConnectionSecretStore for FileSecretStore {
    fn set_secret(&self, connection_id: &str, key: &str, secret: &str) -> Result<(), String> {
        let mut map = self.read_store();
        map.insert(secret_account(connection_id, key), secret.to_string());
        self.write_store(&map)
    }

    fn get_secret(&self, connection_id: &str, key: &str) -> Result<Option<String>, String> {
        Ok(self.read_store().get(&secret_account(connection_id, key)).cloned())
    }

    fn delete_secret(&self, connection_id: &str, key: &str) -> Result<(), String> {
        let mut map = self.read_store();
        map.remove(&secret_account(connection_id, key));
        self.write_store(&map)
    }

    fn delete_secret_prefix(&self, connection_id: &str, key_prefix: &str) -> Result<(), String> {
        let mut map = self.read_store();
        let account_prefix = secret_account(connection_id, key_prefix);
        map.retain(|key, _| !key.starts_with(&account_prefix));
        self.write_store(&map)
    }
}

pub fn save_connections_to_file(
    path: &Path,
    configs: &[ConnectionConfig],
    store: &dyn ConnectionSecretStore,
) -> Result<(), String> {
    delete_removed_connection_secrets(path, configs, store)?;
    for config in configs {
        persist_secret(store, &config.id, MAIN_PASSWORD_KEY, &config.password)?;
        delete_secret_prefix(store, &config.id, TRANSPORT_LAYER_SECRET_PREFIX)?;
        for (index, layer) in config.transport_layers.iter().enumerate() {
            persist_transport_layer_secrets(store, &config.id, index, layer)?;
        }
        persist_secret(store, &config.id, REDIS_SENTINEL_PASSWORD_KEY, &config.redis_sentinel_password)?;
        persist_optional_secret(store, &config.id, CONNECTION_STRING_KEY, config.connection_string.as_deref())?;
        persist_optional_secret(store, &config.id, INIT_SCRIPT_KEY, config.init_script.as_deref())?;
        persist_mq_auth_secrets(store, config)?;
        persist_mq_token_signing_secret(store, config)?;
        persist_mqtt_auth_secrets(store, config)?;
        persist_cassandra_tls_secrets(store, config)?;

        delete_secret_prefix(store, &config.id, PLUGIN_CONNECTION_SECRET_PREFIX)?;
        for (key, secret) in &config.connection_secrets {
            persist_secret(store, &config.id, &plugin_connection_secret_key(key)?, secret)?;
        }

        // New configs persist transport-layer secrets only. Remove legacy transport secret slots after the
        // migrated layer values have been written so old configs do not keep two sources of truth.
        store.delete_secret(&config.id, SSH_PASSWORD_KEY)?;
        store.delete_secret(&config.id, SSH_KEY_PASSPHRASE_KEY)?;
        store.delete_secret(&config.id, PROXY_PASSWORD_KEY)?;
        delete_secret_prefix(store, &config.id, SSH_TUNNEL_SECRET_PREFIX)?;
    }

    write_sanitized_connections(path, configs)
}

pub fn load_connections_from_file(
    path: &Path,
    store: &dyn ConnectionSecretStore,
) -> Result<Vec<ConnectionConfig>, String> {
    if !path.exists() {
        return Ok(vec![]);
    }

    let mut configs = read_connections(path)?;
    let mut needs_rewrite = false;
    for config in &mut configs {
        if config.password.is_empty() {
            if let Some(secret) = store.get_secret(&config.id, MAIN_PASSWORD_KEY)? {
                config.password = secret;
            }
        } else {
            store.set_secret(&config.id, MAIN_PASSWORD_KEY, &config.password)?;
            needs_rewrite = true;
        }

        hydrate_transport_layer_secrets(store, config, &mut needs_rewrite)?;

        if config.redis_sentinel_password.is_empty() {
            if let Some(secret) = store.get_secret(&config.id, REDIS_SENTINEL_PASSWORD_KEY)? {
                config.redis_sentinel_password = secret;
            }
        } else {
            store.set_secret(&config.id, REDIS_SENTINEL_PASSWORD_KEY, &config.redis_sentinel_password)?;
            needs_rewrite = true;
        }

        match config.connection_string.as_deref().filter(|secret| !secret.is_empty()) {
            Some(secret) => {
                store.set_secret(&config.id, CONNECTION_STRING_KEY, secret)?;
                needs_rewrite = true;
            }
            None => {
                if let Some(secret) = store.get_secret(&config.id, CONNECTION_STRING_KEY)? {
                    config.connection_string = Some(secret);
                }
            }
        }

        match config.init_script.as_deref().filter(|secret| !secret.is_empty()) {
            Some(secret) => {
                store.set_secret(&config.id, INIT_SCRIPT_KEY, secret)?;
                needs_rewrite = true;
            }
            None => {
                if let Some(secret) = store.get_secret(&config.id, INIT_SCRIPT_KEY)? {
                    config.init_script = Some(secret);
                }
            }
        }
        hydrate_mq_auth_secrets(store, config, &mut needs_rewrite)?;
        hydrate_mq_token_signing_secret(store, config, &mut needs_rewrite)?;
        hydrate_mqtt_auth_secrets(store, config, &mut needs_rewrite)?;
        hydrate_cassandra_tls_secrets(store, config, &mut needs_rewrite)?;

        let plugin_secret_keys = config.connection_secrets.keys().cloned().collect::<Vec<_>>();
        for key in plugin_secret_keys {
            let storage_key = plugin_connection_secret_key(&key)?;
            let current = config.connection_secrets.get(&key).cloned().unwrap_or_default();
            if current.is_empty() {
                if let Some(secret) = store.get_secret(&config.id, &storage_key)? {
                    config.connection_secrets.insert(key, secret);
                }
            } else {
                store.set_secret(&config.id, &storage_key, &current)?;
                needs_rewrite = true;
            }
        }
    }

    if needs_rewrite {
        write_sanitized_connections(path, &configs)?;
    }

    Ok(configs)
}

fn persist_transport_layer_secrets(
    store: &dyn ConnectionSecretStore,
    connection_id: &str,
    index: usize,
    layer: &TransportLayerConfig,
) -> Result<(), String> {
    match layer {
        TransportLayerConfig::Ssh(ssh) => {
            persist_secret(store, connection_id, &transport_layer_ssh_password_key(index, layer), &ssh.password)?;
            persist_secret(
                store,
                connection_id,
                &transport_layer_ssh_key_passphrase_key(index, layer),
                &ssh.key_passphrase,
            )?;
        }
        TransportLayerConfig::Proxy(proxy) => {
            persist_secret(store, connection_id, &transport_layer_proxy_password_key(index, layer), &proxy.password)?;
        }
        TransportLayerConfig::HttpTunnel(http) => {
            persist_secret(store, connection_id, &transport_layer_http_tunnel_token_key(index, layer), &http.token)?;
        }
    }
    Ok(())
}

fn hydrate_transport_layer_secrets(
    store: &dyn ConnectionSecretStore,
    config: &mut ConnectionConfig,
    needs_rewrite: &mut bool,
) -> Result<(), String> {
    for index in 0..config.transport_layers.len() {
        let layer_for_key = config.transport_layers[index].clone();
        match &mut config.transport_layers[index] {
            TransportLayerConfig::Ssh(ssh) => {
                let password_key = transport_layer_ssh_password_key(index, &layer_for_key);
                if ssh.password.is_empty() {
                    if let Some(secret) = store.get_secret(&config.id, &password_key)?.or(legacy_ssh_password_secret(
                        store,
                        &config.id,
                        index,
                        &layer_for_key,
                    )?) {
                        ssh.password = secret;
                    }
                } else {
                    store.set_secret(&config.id, &password_key, &ssh.password)?;
                    *needs_rewrite = true;
                }

                let passphrase_key = transport_layer_ssh_key_passphrase_key(index, &layer_for_key);
                if ssh.key_passphrase.is_empty() {
                    if let Some(secret) = store
                        .get_secret(&config.id, &passphrase_key)?
                        .or(legacy_ssh_key_passphrase_secret(store, &config.id, index, &layer_for_key)?)
                    {
                        ssh.key_passphrase = secret;
                    }
                } else {
                    store.set_secret(&config.id, &passphrase_key, &ssh.key_passphrase)?;
                    *needs_rewrite = true;
                }
            }
            TransportLayerConfig::Proxy(proxy) => {
                let password_key = transport_layer_proxy_password_key(index, &layer_for_key);
                if proxy.password.is_empty() {
                    if let Some(secret) = store.get_secret(&config.id, &password_key)?.or(legacy_proxy_password_secret(
                        store,
                        &config.id,
                        &layer_for_key,
                    )?) {
                        proxy.password = secret;
                    }
                } else {
                    store.set_secret(&config.id, &password_key, &proxy.password)?;
                    *needs_rewrite = true;
                }
            }
            TransportLayerConfig::HttpTunnel(http) => {
                let token_key = transport_layer_http_tunnel_token_key(index, &layer_for_key);
                if http.token.is_empty() {
                    if let Some(secret) = store.get_secret(&config.id, &token_key)? {
                        http.token = secret;
                    }
                } else {
                    store.set_secret(&config.id, &token_key, &http.token)?;
                    *needs_rewrite = true;
                }
            }
        }
    }
    Ok(())
}

fn legacy_ssh_password_secret(
    store: &dyn ConnectionSecretStore,
    connection_id: &str,
    index: usize,
    layer: &TransportLayerConfig,
) -> Result<Option<String>, String> {
    if let TransportLayerConfig::Ssh(ssh) = layer {
        if ssh.id == "legacy" {
            if let Some(secret) = store.get_secret(connection_id, SSH_PASSWORD_KEY)? {
                return Ok(Some(secret));
            }
        }
        store.get_secret(connection_id, &ssh_tunnel_password_key(index, ssh))
    } else {
        Ok(None)
    }
}

fn legacy_ssh_key_passphrase_secret(
    store: &dyn ConnectionSecretStore,
    connection_id: &str,
    index: usize,
    layer: &TransportLayerConfig,
) -> Result<Option<String>, String> {
    if let TransportLayerConfig::Ssh(ssh) = layer {
        if ssh.id == "legacy" {
            if let Some(secret) = store.get_secret(connection_id, SSH_KEY_PASSPHRASE_KEY)? {
                return Ok(Some(secret));
            }
        }
        store.get_secret(connection_id, &ssh_tunnel_key_passphrase_key(index, ssh))
    } else {
        Ok(None)
    }
}

fn legacy_proxy_password_secret(
    store: &dyn ConnectionSecretStore,
    connection_id: &str,
    layer: &TransportLayerConfig,
) -> Result<Option<String>, String> {
    if matches!(layer, TransportLayerConfig::Proxy(proxy) if proxy.id == "legacy-proxy") {
        store.get_secret(connection_id, PROXY_PASSWORD_KEY)
    } else {
        Ok(None)
    }
}

fn delete_removed_connection_secrets(
    path: &Path,
    configs: &[ConnectionConfig],
    store: &dyn ConnectionSecretStore,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    let previous = match read_connections(path) {
        Ok(configs) => configs,
        Err(_) => return Ok(()),
    };
    let current_ids: HashSet<&str> = configs.iter().map(|config| config.id.as_str()).collect();
    for config in previous {
        if current_ids.contains(config.id.as_str()) {
            continue;
        }
        store.delete_secret(&config.id, MAIN_PASSWORD_KEY)?;
        store.delete_secret(&config.id, SSH_PASSWORD_KEY)?;
        store.delete_secret(&config.id, SSH_KEY_PASSPHRASE_KEY)?;
        delete_secret_prefix(store, &config.id, SSH_TUNNEL_SECRET_PREFIX)?;
        delete_secret_prefix(store, &config.id, TRANSPORT_LAYER_SECRET_PREFIX)?;
        store.delete_secret(&config.id, CONNECTION_STRING_KEY)?;
        store.delete_secret(&config.id, INIT_SCRIPT_KEY)?;
        delete_secret_prefix(store, &config.id, MQ_AUTH_SECRET_PREFIX)?;
        delete_secret_prefix(store, &config.id, MQ_TOKEN_SIGNING_SECRET_PREFIX)?;
        delete_secret_prefix(store, &config.id, MQTT_AUTH_SECRET_PREFIX)?;
        delete_secret_prefix(store, &config.id, CASSANDRA_TLS_SECRET_PREFIX)?;
        delete_secret_prefix(store, &config.id, SALESFORCE_AUTH_SECRET_PREFIX)?;
    }
    Ok(())
}

fn persist_secret(
    store: &dyn ConnectionSecretStore,
    connection_id: &str,
    key: &str,
    secret: &str,
) -> Result<(), String> {
    if secret.is_empty() {
        store.delete_secret(connection_id, key)
    } else {
        store.set_secret(connection_id, key, secret)
    }
}

fn persist_optional_secret(
    store: &dyn ConnectionSecretStore,
    connection_id: &str,
    key: &str,
    secret: Option<&str>,
) -> Result<(), String> {
    match secret.filter(|secret| !secret.is_empty()) {
        Some(secret) => store.set_secret(connection_id, key, secret),
        None => store.delete_secret(connection_id, key),
    }
}

fn delete_secret_prefix(
    store: &dyn ConnectionSecretStore,
    connection_id: &str,
    key_prefix: &str,
) -> Result<(), String> {
    store.delete_secret_prefix(connection_id, key_prefix)
}

fn persist_mq_auth_secrets(store: &dyn ConnectionSecretStore, config: &ConnectionConfig) -> Result<(), String> {
    {
        delete_secret_prefix(store, &config.id, MQ_AUTH_SECRET_PREFIX)?;
        return Ok(());
    }
}

fn persist_mq_token_signing_secret(store: &dyn ConnectionSecretStore, config: &ConnectionConfig) -> Result<(), String> {
    {
        delete_secret_prefix(store, &config.id, MQ_TOKEN_SIGNING_SECRET_PREFIX)?;
        return Ok(());
    }
}

fn hydrate_mq_auth_secrets(
    store: &dyn ConnectionSecretStore,
    config: &mut ConnectionConfig,
    needs_rewrite: &mut bool,
) -> Result<(), String> {
    {
        return Ok(());
    }
}

fn hydrate_mq_token_signing_secret(
    store: &dyn ConnectionSecretStore,
    config: &mut ConnectionConfig,
    needs_rewrite: &mut bool,
) -> Result<(), String> {
    {
        return Ok(());
    }
}

fn scrub_mq_auth_secrets(config: &mut ConnectionConfig) {
    let Some(auth) = mq_auth_object_mut(config.external_config.as_mut()) else {
        return;
    };
    match mq_auth_kind(auth).as_deref() {
        Some("token") => scrub_json_secret(auth, "token"),
        Some("basic") => scrub_json_secret(auth, "password"),
        Some("apiKey") | Some("api_key") | Some("apikey") => scrub_json_secret(auth, "value"),
        Some("oauth2") => scrub_json_secret(auth, "clientSecret"),
        _ => {}
    }
}

fn scrub_mq_token_signing_secret(config: &mut ConnectionConfig) {
    let Some(signing) = mq_token_signing_object_mut(config.external_config.as_mut()) else {
        return;
    };
    scrub_json_secret(signing, "key");
}

fn persist_cassandra_tls_secrets(store: &dyn ConnectionSecretStore, config: &ConnectionConfig) -> Result<(), String> {
    {
        return delete_secret_prefix(store, &config.id, CASSANDRA_TLS_SECRET_PREFIX);
    }
}

fn hydrate_cassandra_tls_secrets(
    store: &dyn ConnectionSecretStore,
    config: &mut ConnectionConfig,
    needs_rewrite: &mut bool,
) -> Result<(), String> {
    {
        return Ok(());
    }
}

fn scrub_cassandra_tls_secrets(config: &mut ConnectionConfig) {
    {
        return;
    }
}

fn cassandra_tls_object(value: Option<&serde_json::Value>) -> Option<&serde_json::Map<String, serde_json::Value>> {
    value?.get("tls")?.as_object()
}

fn cassandra_tls_object_mut(
    value: Option<&mut serde_json::Value>,
) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    value?.get_mut("tls")?.as_object_mut()
}

// ── MQTT 密钥持久化 ──────────────────────────────────────────────

fn persist_mqtt_auth_secrets(store: &dyn ConnectionSecretStore, config: &ConnectionConfig) -> Result<(), String> {
    {
        delete_secret_prefix(store, &config.id, MQTT_AUTH_SECRET_PREFIX)?;
        return Ok(());
    }
}

fn hydrate_mqtt_auth_secrets(
    store: &dyn ConnectionSecretStore,
    config: &mut ConnectionConfig,
    needs_rewrite: &mut bool,
) -> Result<(), String> {
    {
        return Ok(());
    }
}

fn scrub_mqtt_auth_secrets(config: &mut ConnectionConfig) {
    let Some(auth) = mqtt_auth_object_mut(config.external_config.as_mut()) else {
        return;
    };
    if let Some("password") = mqtt_auth_kind(auth).as_deref() {
        scrub_json_secret(auth, "password")
    }
}

fn mqtt_auth_kind(auth: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    auth.get("kind").and_then(serde_json::Value::as_str).map(ToString::to_string)
}

fn mqtt_auth_object_mut(
    value: Option<&mut serde_json::Value>,
) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    value?.get_mut("auth")?.as_object_mut()
}

fn scrub_json_secret(auth: &mut serde_json::Map<String, serde_json::Value>, field: &str) {
    if auth.contains_key(field) {
        auth.insert(field.to_string(), serde_json::Value::String(String::new()));
    }
}

fn mq_auth_kind(auth: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    auth.get("kind").and_then(serde_json::Value::as_str).map(ToString::to_string)
}

fn mq_auth_object(value: Option<&serde_json::Value>) -> Option<&serde_json::Map<String, serde_json::Value>> {
    value?.get("auth")?.as_object()
}

fn mq_auth_object_mut(
    value: Option<&mut serde_json::Value>,
) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    value?.get_mut("auth")?.as_object_mut()
}

fn mq_token_signing_object(value: Option<&serde_json::Value>) -> Option<&serde_json::Map<String, serde_json::Value>> {
    value?.get("tokenSigning")?.as_object()
}

fn mq_token_signing_object_mut(
    value: Option<&mut serde_json::Value>,
) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    value?.get_mut("tokenSigning")?.as_object_mut()
}

fn ssh_tunnel_secret_segment(index: usize, hop: &crate::models::connection::SshTunnelConfig) -> String {
    if hop.id.trim().is_empty() {
        index.to_string()
    } else {
        hop.id.clone()
    }
}

fn ssh_tunnel_password_key(index: usize, hop: &crate::models::connection::SshTunnelConfig) -> String {
    format!("{}{}.password", SSH_TUNNEL_SECRET_PREFIX, ssh_tunnel_secret_segment(index, hop))
}

fn ssh_tunnel_key_passphrase_key(index: usize, hop: &crate::models::connection::SshTunnelConfig) -> String {
    format!("{}{}.key_passphrase", SSH_TUNNEL_SECRET_PREFIX, ssh_tunnel_secret_segment(index, hop))
}

fn transport_layer_secret_segment(index: usize, layer: &TransportLayerConfig) -> String {
    let id = layer.id().trim();
    if id.is_empty() {
        index.to_string()
    } else {
        id.to_string()
    }
}

fn transport_layer_ssh_password_key(index: usize, layer: &TransportLayerConfig) -> String {
    format!("{}{}.ssh_password", TRANSPORT_LAYER_SECRET_PREFIX, transport_layer_secret_segment(index, layer))
}

fn transport_layer_ssh_key_passphrase_key(index: usize, layer: &TransportLayerConfig) -> String {
    format!("{}{}.ssh_key_passphrase", TRANSPORT_LAYER_SECRET_PREFIX, transport_layer_secret_segment(index, layer))
}

fn transport_layer_proxy_password_key(index: usize, layer: &TransportLayerConfig) -> String {
    format!("{}{}.proxy_password", TRANSPORT_LAYER_SECRET_PREFIX, transport_layer_secret_segment(index, layer))
}

fn transport_layer_http_tunnel_token_key(index: usize, layer: &TransportLayerConfig) -> String {
    format!("{}{}.http_tunnel_token", TRANSPORT_LAYER_SECRET_PREFIX, transport_layer_secret_segment(index, layer))
}

fn read_connections(path: &Path) -> Result<Vec<ConnectionConfig>, String> {
    let json = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(&json).map_err(|e| e.to_string())
}

fn write_sanitized_connections(path: &Path, configs: &[ConnectionConfig]) -> Result<(), String> {
    let sanitized = sanitize_connections(configs);
    let json = serde_json::to_string_pretty(&sanitized).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| e.to_string())
}

fn sanitize_connections(configs: &[ConnectionConfig]) -> Vec<ConnectionConfig> {
    configs
        .iter()
        .cloned()
        .map(|mut config| {
            config.password.clear();
            for layer in &mut config.transport_layers {
                match layer {
                    TransportLayerConfig::Ssh(ssh) => {
                        ssh.password.clear();
                        ssh.key_passphrase.clear();
                    }
                    TransportLayerConfig::Proxy(proxy) => {
                        proxy.password.clear();
                    }
                    TransportLayerConfig::HttpTunnel(http) => {
                        http.token.clear();
                    }
                }
            }
            config.redis_sentinel_password.clear();
            config.connection_string = None;
            config.init_script = None;
            scrub_mq_auth_secrets(&mut config);
            scrub_mq_token_signing_secret(&mut config);
            scrub_mqtt_auth_secrets(&mut config);
            scrub_cassandra_tls_secrets(&mut config);

            scrub_plugin_connection_secrets(&mut config);
            config
        })
        .collect()
}

fn scrub_plugin_connection_secrets(config: &mut ConnectionConfig) {
    for secret in config.connection_secrets.values_mut() {
        secret.clear();
    }
}

pub fn secret_account(connection_id: &str, key: &str) -> String {
    format!("connection:{connection_id}:{key}")
}

#[cfg(test)]
mod tests {
    fn connection(id: &str, password: &str, _ssh_password: &str) -> ConnectionConfig {
        ConnectionConfig {
            oracle_oci_nls_lang: None,
            oracle_oci_tns_admin: None,
            docs_notes_path: None,
            id: id.to_string(),
            name: format!("{id} connection"),
            note: String::new(),
            db_type: DatabaseType::Mysql,
            driver_profile: None,
            driver_label: None,
            url_params: None,
            agent_java_options: Vec::new(),
            host: "localhost".to_string(),
            port: 3306,
            username: "mysql".to_string(),
            password: password.to_string(),
            database: Some("mysql".to_string()),
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
            connect_timeout_secs: crate::models::connection::default_connect_timeout_secs(),
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
            redis_key_separator: crate::models::connection::default_redis_key_separator(),
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

    use super::{
        load_connections_from_file, save_connections_to_file, ConnectionSecretStore, CASSANDRA_KEYSTORE_PASSWORD_KEY,
        CASSANDRA_TRUSTSTORE_PASSWORD_KEY, CONNECTION_STRING_KEY, INIT_SCRIPT_KEY, MAIN_PASSWORD_KEY,
        MQTT_AUTH_PASSWORD_KEY, MQ_AUTH_PASSWORD_KEY, MQ_AUTH_TOKEN_KEY, MQ_TOKEN_SIGNING_KEY,
        PLUGIN_CONNECTION_SECRET_PREFIX, REDIS_SENTINEL_PASSWORD_KEY, SSH_PASSWORD_KEY,
    };
    use crate::models::connection::{
        ConnectionConfig, DatabaseType, HttpTunnelConfig, SshTunnelConfig, TransportLayerConfig,
    };
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::Path;

    #[derive(Default)]
    struct MemorySecretStore {
        values: RefCell<HashMap<String, String>>,
        deleted: RefCell<Vec<String>>,
    }

    impl MemorySecretStore {
        fn set_existing(&self, connection_id: &str, key: &str, value: &str) {
            self.values.borrow_mut().insert(secret_key(connection_id, key), value.to_string());
        }

        fn get_existing(&self, connection_id: &str, key: &str) -> Option<String> {
            self.values.borrow().get(&secret_key(connection_id, key)).cloned()
        }

        fn was_deleted(&self, connection_id: &str, key: &str) -> bool {
            self.deleted.borrow().contains(&secret_key(connection_id, key))
        }
    }

    impl ConnectionSecretStore for MemorySecretStore {
        fn set_secret(&self, connection_id: &str, key: &str, secret: &str) -> Result<(), String> {
            self.values.borrow_mut().insert(secret_key(connection_id, key), secret.to_string());
            Ok(())
        }

        fn get_secret(&self, connection_id: &str, key: &str) -> Result<Option<String>, String> {
            Ok(self.values.borrow().get(&secret_key(connection_id, key)).cloned())
        }

        fn delete_secret(&self, connection_id: &str, key: &str) -> Result<(), String> {
            self.values.borrow_mut().remove(&secret_key(connection_id, key));
            self.deleted.borrow_mut().push(secret_key(connection_id, key));
            Ok(())
        }

        fn delete_secret_prefix(&self, connection_id: &str, key_prefix: &str) -> Result<(), String> {
            let prefix = secret_key(connection_id, key_prefix);
            self.values.borrow_mut().retain(|key, _| !key.starts_with(&prefix));
            self.deleted.borrow_mut().push(prefix);
            Ok(())
        }
    }

    fn secret_key(connection_id: &str, key: &str) -> String {
        format!("{connection_id}:{key}")
    }

    fn temp_connections_file(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dbx-connection-secrets-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("connections.json")
    }

    fn ssh_hop(id: &str, password: &str, passphrase: &str) -> SshTunnelConfig {
        SshTunnelConfig {
            profile_id: String::new(),
            id: id.to_string(),
            name: String::new(),
            enabled: true,
            host: "bastion".to_string(),
            port: 22,
            user: "user".to_string(),
            password: password.to_string(),
            key_path: "~/.ssh/id_ed25519".to_string(),
            key_passphrase: passphrase.to_string(),
            connect_timeout_secs: 5,
            expose_lan: false,
            use_ssh_agent: false,
            ssh_agent_sock_path: String::new(),
            auth_method: "key".to_string(),
            allow_exec_channel_proxy: false,
            proxy_command: String::new(),
        }
    }

    fn http_tunnel(id: &str, token: &str) -> TransportLayerConfig {
        TransportLayerConfig::HttpTunnel(HttpTunnelConfig {
            profile_id: String::new(),
            id: id.to_string(),
            name: String::new(),
            enabled: true,
            url: "https://dbx.example.com/dbx_tunnel.php".to_string(),
            token: token.to_string(),
            connect_timeout_secs: 10,
        })
    }

    fn read_configs(path: &Path) -> Vec<ConnectionConfig> {
        let json = std::fs::read_to_string(path).unwrap();
        serde_json::from_str(&json).unwrap()
    }

    #[test]
    fn save_connections_moves_passwords_to_secret_store_and_redacts_file() {
        let path = temp_connections_file("save-redacts");
        let store = MemorySecretStore::default();
        let mut config = connection("main", "db-secret", "ssh-secret");
        config.transport_layers = vec![TransportLayerConfig::Ssh(ssh_hop("hop-1", "hop-secret", "hop-key"))];
        config.redis_sentinel_password = "sentinel-secret".to_string();
        let configs = vec![config];

        save_connections_to_file(&path, &configs, &store).unwrap();

        assert_eq!(store.get_existing("main", MAIN_PASSWORD_KEY).as_deref(), Some("db-secret"));
        assert_eq!(store.get_existing("main", "transport_layers.hop-1.ssh_password").as_deref(), Some("hop-secret"));
        assert_eq!(store.get_existing("main", "transport_layers.hop-1.ssh_key_passphrase").as_deref(), Some("hop-key"));
        assert_eq!(store.get_existing("main", REDIS_SENTINEL_PASSWORD_KEY).as_deref(), Some("sentinel-secret"));
        let persisted = read_configs(&path);
        assert_eq!(persisted[0].password, "");
        match &persisted[0].transport_layers[0] {
            TransportLayerConfig::Ssh(ssh) => {
                assert_eq!(ssh.password, "");
                assert_eq!(ssh.key_passphrase, "");
            }
            _ => panic!("expected ssh layer"),
        }
        assert_eq!(persisted[0].redis_sentinel_password, "");
    }

    #[test]
    fn load_connections_restores_passwords_from_secret_store() {
        let path = temp_connections_file("load-restores");
        let store = MemorySecretStore::default();
        store.set_existing("main", MAIN_PASSWORD_KEY, "db-secret");
        store.set_existing("main", SSH_PASSWORD_KEY, "ssh-secret");
        store.set_existing("main", "ssh_tunnels.hop-1.password", "hop-secret");
        store.set_existing("main", "ssh_tunnels.hop-1.key_passphrase", "hop-key");
        store.set_existing("main", REDIS_SENTINEL_PASSWORD_KEY, "sentinel-secret");
        let mut sanitized_config = connection("main", "", "");
        sanitized_config.transport_layers = vec![TransportLayerConfig::Ssh(ssh_hop("hop-1", "", ""))];
        let sanitized = vec![sanitized_config];
        std::fs::write(&path, serde_json::to_string_pretty(&sanitized).unwrap()).unwrap();

        let loaded = load_connections_from_file(&path, &store).unwrap();

        assert_eq!(loaded[0].password, "db-secret");
        match &loaded[0].transport_layers[0] {
            TransportLayerConfig::Ssh(ssh) => {
                assert_eq!(ssh.password, "hop-secret");
                assert_eq!(ssh.key_passphrase, "hop-key");
            }
            _ => panic!("expected ssh layer"),
        }
        assert_eq!(loaded[0].redis_sentinel_password, "sentinel-secret");
    }

    #[test]
    fn save_and_load_connections_move_http_tunnel_token_to_secret_store() {
        let path = temp_connections_file("http-tunnel-token");
        let store = MemorySecretStore::default();
        let mut config = connection("main", "", "");
        config.transport_layers = vec![http_tunnel("http", "tunnel-secret")];

        save_connections_to_file(&path, &[config], &store).unwrap();

        assert_eq!(
            store.get_existing("main", "transport_layers.http.http_tunnel_token").as_deref(),
            Some("tunnel-secret")
        );
        let persisted = read_configs(&path);
        match &persisted[0].transport_layers[0] {
            TransportLayerConfig::HttpTunnel(http) => assert_eq!(http.token, ""),
            _ => panic!("expected http tunnel layer"),
        }

        let loaded = load_connections_from_file(&path, &store).unwrap();
        match &loaded[0].transport_layers[0] {
            TransportLayerConfig::HttpTunnel(http) => assert_eq!(http.token, "tunnel-secret"),
            _ => panic!("expected http tunnel layer"),
        }
    }

    #[test]
    fn load_connections_migrates_plaintext_passwords_and_rewrites_sanitized_file() {
        let path = temp_connections_file("migrates-plaintext");
        let store = MemorySecretStore::default();
        let legacy = serde_json::json!([{
            "id": "legacy",
            "name": "legacy connection",
            "db_type": "mysql",
            "host": "localhost",
            "port": 5432,
            "username": "postgres",
            "password": "plain-db",
            "database": "postgres",
            "ssh_enabled": true,
            "ssh_host": "bastion",
            "ssh_port": 22,
            "ssh_user": "user",
            "ssh_password": "plain-ssh"
        }]);
        std::fs::write(&path, serde_json::to_string_pretty(&legacy).unwrap()).unwrap();

        let loaded = load_connections_from_file(&path, &store).unwrap();

        assert_eq!(loaded[0].password, "plain-db");
        match &loaded[0].transport_layers[0] {
            TransportLayerConfig::Ssh(ssh) => assert_eq!(ssh.password, "plain-ssh"),
            _ => panic!("expected ssh layer"),
        }
        assert_eq!(store.get_existing("legacy", MAIN_PASSWORD_KEY).as_deref(), Some("plain-db"));
        assert_eq!(store.get_existing("legacy", "transport_layers.legacy.ssh_password").as_deref(), Some("plain-ssh"));
        let persisted = read_configs(&path);
        assert_eq!(persisted[0].password, "");
        match &persisted[0].transport_layers[0] {
            TransportLayerConfig::Ssh(ssh) => assert_eq!(ssh.password, ""),
            _ => panic!("expected ssh layer"),
        }
    }

    #[test]
    fn save_connections_deletes_secrets_for_removed_connections() {
        let path = temp_connections_file("deletes-removed");
        let store = MemorySecretStore::default();
        let previous = vec![connection("old", "", ""), connection("kept", "", "")];
        std::fs::write(&path, serde_json::to_string_pretty(&previous).unwrap()).unwrap();
        store.set_existing("old", MAIN_PASSWORD_KEY, "old-db");
        store.set_existing("old", SSH_PASSWORD_KEY, "old-ssh");
        store.set_existing("kept", MAIN_PASSWORD_KEY, "kept-db");

        save_connections_to_file(&path, &[connection("kept", "new-db", "")], &store).unwrap();

        assert!(store.was_deleted("old", MAIN_PASSWORD_KEY));
        assert!(store.was_deleted("old", SSH_PASSWORD_KEY));
        assert_eq!(store.get_existing("kept", MAIN_PASSWORD_KEY).as_deref(), Some("new-db"));
    }
}
