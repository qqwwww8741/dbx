use std::sync::OnceLock;

use serde::Deserialize;

use crate::models::connection::DatabaseType;

const DATABASE_MANIFEST_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/database_manifest.json"));

#[derive(Debug, Deserialize)]
struct DatabaseManifestFile {
    drivers: Vec<DatabaseManifestEntry>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseRuntimeMode {
    Native,
    File,
    Agent,
    External,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseDriverProfile {
    pub profile: String,
    pub agent_key: String,
    #[serde(default)]
    pub package_key: Option<String>,
    pub label: String,
    #[serde(default)]
    pub store_visible: bool,
    #[serde(default)]
    pub store_order: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedDriverEntry {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub store_visible: bool,
    #[serde(default)]
    pub store_order: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseManifestEntry {
    pub db_type: DatabaseType,
    pub label: String,
    #[serde(default)]
    pub dialect: Option<String>,
    pub runtime_mode: DatabaseRuntimeMode,
    pub mcp_mode: String,
    #[serde(default)]
    pub agent_key: Option<String>,
    #[serde(default)]
    pub driver_store_visible: bool,
    #[serde(default)]
    pub driver_store_order: Option<u32>,
    #[serde(default)]
    pub driver_profiles: Vec<DatabaseDriverProfile>,
    #[serde(default)]
    pub managed_drivers: Vec<ManagedDriverEntry>,
    #[serde(default)]
    pub single_connection_pool: bool,
    #[serde(default)]
    pub metadata_connection_scoped: bool,
    #[serde(default)]
    pub skip_tcp_probe: bool,
    #[serde(default)]
    pub default_port: Option<u16>,
    #[serde(default)]
    pub local_file: bool,
    #[serde(default)]
    pub specialized_surface: bool,
}

static DATABASE_MANIFEST: OnceLock<DatabaseManifestFile> = OnceLock::new();

fn manifest() -> &'static DatabaseManifestFile {
    DATABASE_MANIFEST.get_or_init(|| {
        serde_json::from_str(DATABASE_MANIFEST_JSON).expect("database-drivers.manifest.json must be valid")
    })
}

pub fn entries() -> &'static [DatabaseManifestEntry] {
    &manifest().drivers
}

pub fn entry(db_type: &DatabaseType) -> Option<&'static DatabaseManifestEntry> {
    entries().iter().find(|candidate| candidate.db_type == *db_type)
}

pub fn default_port(db_type: &DatabaseType) -> Option<u16> {
    entry(db_type).and_then(|candidate| candidate.default_port.or(candidate.local_file.then_some(0)))
}

pub fn agent_key(db_type: &DatabaseType) -> Option<&'static str> {
    entry(db_type).and_then(|candidate| candidate.agent_key.as_deref())
}

pub fn dialect_name(db_type: &DatabaseType) -> Option<&'static str> {
    entry(db_type).and_then(|candidate| candidate.dialect.as_deref())
}

pub fn is_agent_runtime(db_type: &DatabaseType) -> bool {
    entry(db_type).is_some_and(|candidate| candidate.runtime_mode == DatabaseRuntimeMode::Agent)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
}
