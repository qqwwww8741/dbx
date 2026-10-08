//! Read-only data access for plugins (`host.data:read`, Host API 1.4).
//!
//! A plugin UI can ask the host to run one read-only SQL statement on a DBX
//! connection. The plugin never receives a driver, a pool, a credential, or a
//! connection string, and the host owns every safety decision:
//!
//! * the plugin must be installed, compatible, and declare `host.data:read`;
//! * the user must have granted this plugin access to this connection — the
//!   grant is persisted per (plugin, connection) and revocable in the Plugin
//!   Center, and a plugin cannot grant itself anything;
//! * the connection must already be open: like the plan and schema metadata
//!   APIs, DBX never connects on a plugin's behalf;
//! * the SQL must be a single statement the shared risk classifier rates
//!   read-only (the same gate as MCP read-only access and the AI agent), and
//!   may not switch the session's database, because the statement runs on the
//!   connection's shared pool;
//! * execution uses the host's own connection, timeout ceiling, and row and
//!   byte caps.

use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::plugin_plan::plugin_plan_timeout_secs;
use super::{execute_sql_statement_with_options, QueryExecutionOptions};
use crate::connection::AppState;
use crate::db::QueryResult;
use crate::models::connection::{ConnectionConfig, DatabaseType};
use crate::sql_risk::SqlRisk;

pub const PLUGIN_DATA_READ_PERMISSION: &str = "host.data:read";
/// Error code prefix for a missing (or revoked) user grant, so the host UI can
/// ask for consent instead of showing a failure.
pub const PLUGIN_DATA_ACCESS_NOT_GRANTED: &str = "PLUGIN_DATA_ACCESS_NOT_GRANTED";
pub const DEFAULT_PLUGIN_DATA_MAX_ROWS: usize = 500;
pub const MAX_PLUGIN_DATA_MAX_ROWS: usize = 5_000;
/// Serialized row payload cap; well inside what the plugin bridge carries.
pub const MAX_PLUGIN_DATA_RESULT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_PLUGIN_DATA_SQL_CHARS: usize = 100_000;
const MAX_PLUGIN_DATA_NAME_CHARS: usize = 256;

/// One read request. There is deliberately no way to pass a session id, an
/// execution mode, or several statements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDataQueryRequest {
    pub connection_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    pub sql: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rows: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDataColumn {
    pub name: String,
    /// Database type name when the driver reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDataQueryResult {
    pub db_type: String,
    pub columns: Vec<PluginDataColumn>,
    pub rows: Vec<Vec<Value>>,
    /// True when rows were cut by the row cap, the byte cap, or the driver.
    pub truncated: bool,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDataGrant {
    pub connection_id: String,
    /// Absent when the granted connection no longer exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_name: Option<String>,
}

/// Runs one read-only statement for `plugin_id`. `plugin_id` must come from
/// the host (the plugin bridge binds it), never from plugin-supplied data.
pub async fn query_plugin_data(
    state: &AppState,
    plugin_id: &str,
    request: PluginDataQueryRequest,
) -> Result<PluginDataQueryResult, String> {
    let request = validate_request(request)?;
    require_plugin_permission(state, plugin_id)?;
    // The grant is checked before the connection is resolved, so an unknown id
    // and an ungranted one look the same to the plugin.
    require_grant(state, plugin_id, &request.connection_id).await?;
    let config = connection_config(state, &request.connection_id).await?;
    if !state.is_connection_open(&request.connection_id).await {
        return Err("Connection is not open".to_string());
    }
    require_sql_connection(&config)?;
    require_single_read_only_statement(&request.sql, config.db_type)?;

    let max_rows = request.max_rows.unwrap_or(DEFAULT_PLUGIN_DATA_MAX_ROWS).clamp(1, MAX_PLUGIN_DATA_MAX_ROWS);
    let started = Instant::now();
    let result = execute_sql_statement_with_options(
        state,
        &request.connection_id,
        request.database.as_deref().unwrap_or_default(),
        &request.sql,
        request.schema.as_deref(),
        None,
        QueryExecutionOptions {
            max_rows: Some(max_rows),
            max_result_bytes: Some(MAX_PLUGIN_DATA_RESULT_BYTES),
            timeout_secs: Some(plugin_plan_timeout_secs(request.timeout_ms, &config)),
            ..Default::default()
        },
    )
    .await?;
    let shaped = shape_result(config.db_type, result, max_rows, started.elapsed().as_millis() as u64);
    // Audit trail without SQL text or values: literals can carry user data.
    log::info!(
        "[plugin-data] plugin={plugin_id} connection={} rows={} truncated={}",
        request.connection_id,
        shaped.rows.len(),
        shaped.truncated
    );
    Ok(shaped)
}

/// The connections the user granted to `plugin_id`, with current names.
pub async fn list_plugin_data_grants(state: &AppState, plugin_id: &str) -> Result<Vec<PluginDataGrant>, String> {
    let connection_ids = state.storage.load_plugin_data_grants(plugin_id).await?;
    let configs = state.configs.read().await;
    Ok(connection_ids
        .into_iter()
        .map(|connection_id| PluginDataGrant {
            connection_name: configs.get(&connection_id).map(|config| config.name.clone()),
            connection_id,
        })
        .collect())
}

/// Records the user's answer to a consent prompt, or a revocation. Granting
/// requires a plugin that declared the permission and an existing SQL
/// connection, so a grant can never name something the API would refuse.
pub async fn set_plugin_data_grant(
    state: &AppState,
    plugin_id: &str,
    connection_id: &str,
    granted: bool,
) -> Result<Vec<PluginDataGrant>, String> {
    if granted {
        require_plugin_permission(state, plugin_id)?;
        let config = connection_config(state, connection_id.trim()).await?;
        require_sql_connection(&config)?;
    }
    state.storage.set_plugin_data_grant(plugin_id, connection_id, granted).await?;
    list_plugin_data_grants(state, plugin_id).await
}

fn validate_request(mut request: PluginDataQueryRequest) -> Result<PluginDataQueryRequest, String> {
    request.connection_id = required_name(&request.connection_id, "connectionId")?;
    let sql = request.sql.trim();
    if sql.is_empty() {
        return Err("Plugin data queries need a non-empty sql".to_string());
    }
    if sql.chars().count() > MAX_PLUGIN_DATA_SQL_CHARS {
        return Err(format!("Plugin data query sql exceeds {MAX_PLUGIN_DATA_SQL_CHARS} characters"));
    }
    request.sql = sql.to_string();
    request.database = optional_name(request.database, "database")?;
    request.schema = optional_name(request.schema, "schema")?;
    Ok(request)
}

fn required_name(value: &str, label: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_PLUGIN_DATA_NAME_CHARS {
        return Err(format!("Plugin data queries need a valid {label}"));
    }
    Ok(trimmed.to_string())
}

fn optional_name(value: Option<String>, label: &str) -> Result<Option<String>, String> {
    match value.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(trimmed) if trimmed.chars().count() > MAX_PLUGIN_DATA_NAME_CHARS => {
            Err(format!("Plugin data query {label} is too long"))
        }
        Some(trimmed) => Ok(Some(trimmed.to_string())),
    }
}

fn require_plugin_permission(state: &AppState, plugin_id: &str) -> Result<(), String> {
    let plugin =
        state.plugins.find_plugin(plugin_id)?.ok_or_else(|| format!("Plugin '{plugin_id}' is not installed"))?;
    if !plugin.compatibility.compatible {
        return Err(format!("Plugin '{plugin_id}' is not compatible with this DBX version"));
    }
    if !plugin.manifest.permissions.iter().any(|permission| permission == PLUGIN_DATA_READ_PERMISSION) {
        return Err(format!("Plugin has not declared permission '{PLUGIN_DATA_READ_PERMISSION}'"));
    }
    Ok(())
}

async fn require_grant(state: &AppState, plugin_id: &str, connection_id: &str) -> Result<(), String> {
    let grants = state.storage.load_plugin_data_grants(plugin_id).await?;
    if grants.iter().any(|granted| granted == connection_id) {
        return Ok(());
    }
    Err(format!("{PLUGIN_DATA_ACCESS_NOT_GRANTED}: the user has not granted this plugin access to the connection"))
}

async fn connection_config(state: &AppState, connection_id: &str) -> Result<ConnectionConfig, String> {
    state.configs.read().await.get(connection_id).cloned().ok_or_else(|| "Connection config not found".to_string())
}

fn require_sql_connection(config: &ConnectionConfig) -> Result<(), String> {
    let sql_capable = crate::query_execution_sql::supports_sql_query(config.db_type) && !false;
    if sql_capable {
        return Ok(());
    }
    Err(format!("Plugin data queries are not available for '{}' connections", config.db_type.as_str()))
}

fn require_single_read_only_statement(sql: &str, db_type: DatabaseType) -> Result<(), String> {
    if crate::sql::split_sql_statements_for_database(sql, db_type).len() != 1 {
        return Err("Plugin data queries accept exactly one SQL statement".to_string());
    }
    // Messages stay generic so a plugin cannot use them to probe the gate.
    match crate::sql_risk::classify_sql_risk_for_database(sql, db_type) {
        Ok(SqlRisk::ReadOnly) => {}
        Ok(_) => return Err("Plugin data queries are read-only; the statement was rejected".to_string()),
        Err(_) => return Err("The statement could not be verified as read-only".to_string()),
    }
    if crate::sql_risk::mcp_sql_has_forbidden_database_switch(sql, db_type) {
        return Err("Plugin data queries cannot switch the session database; pass `database` instead".to_string());
    }
    Ok(())
}

fn shape_result(db_type: DatabaseType, result: QueryResult, max_rows: usize, elapsed_ms: u64) -> PluginDataQueryResult {
    let columns = result
        .columns
        .iter()
        .enumerate()
        .map(|(index, name)| PluginDataColumn {
            name: name.clone(),
            data_type: result.column_types.get(index).filter(|data_type| !data_type.is_empty()).cloned(),
        })
        .collect();
    let mut truncated = result.truncated || result.has_more || result.rows.len() > max_rows;
    let mut rows = Vec::new();
    let mut bytes = 0usize;
    for row in result.rows.into_iter().take(max_rows) {
        let row_bytes = serde_json::to_vec(&row).map_or(0, |encoded| encoded.len());
        if bytes + row_bytes > MAX_PLUGIN_DATA_RESULT_BYTES {
            truncated = true;
            break;
        }
        bytes += row_bytes;
        rows.push(row);
    }
    PluginDataQueryResult { db_type: db_type.as_str().to_string(), columns, rows, truncated, elapsed_ms }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(sql: &str) -> PluginDataQueryRequest {
        PluginDataQueryRequest {
            connection_id: " conn-1 ".to_string(),
            database: Some("  ".to_string()),
            schema: None,
            sql: sql.to_string(),
            max_rows: None,
            timeout_ms: None,
        }
    }

    #[test]
    fn requests_are_trimmed_and_bounded() {
        let validated = validate_request(request("  SELECT 1  ")).unwrap();
        assert_eq!(validated.connection_id, "conn-1");
        assert_eq!(validated.database, None);
        assert_eq!(validated.sql, "SELECT 1");
        assert!(validate_request(request("   ")).is_err());
        assert!(
            validate_request(PluginDataQueryRequest { connection_id: " ".to_string(), ..request("SELECT 1") }).is_err()
        );
        assert!(validate_request(request(&"x".repeat(MAX_PLUGIN_DATA_SQL_CHARS + 1))).is_err());
    }
}
