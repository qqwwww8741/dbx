use std::{collections::HashMap, sync::Arc};

use axum::http::HeaderMap;
use dbx_core::mcp_policy::McpConnectionGroupPath;
use dbx_core::models::connection::ConnectionConfig;
use dbx_core::storage::{McpDatabaseScope, McpGlobalPolicy};

use crate::error::AppError;
use crate::state::WebState;

const MCP_REQUEST_HEADER: &str = "x-dbx-mcp-request";

pub fn is_mcp_request(headers: &HeaderMap) -> bool {
    headers.get(MCP_REQUEST_HEADER).and_then(|value| value.to_str().ok()) == Some("1")
}

async fn load_policy(state: &Arc<WebState>) -> Result<McpGlobalPolicy, AppError> {
    state.app.storage.load_mcp_global_policy().await.map(|state| state.policy()).map_err(AppError::from)
}

async fn load_policy_context(
    state: &Arc<WebState>,
) -> Result<(McpGlobalPolicy, HashMap<String, McpConnectionGroupPath>), AppError> {
    let policy = load_policy(state).await?;
    let group_paths = match state.app.storage.load_sidebar_layout().await {
        Ok(Some(layout)) => dbx_core::mcp_policy::connection_group_paths(&layout),
        Ok(None) => Ok(HashMap::new()),
        Err(error) => Err(error),
    };
    match group_paths {
        Ok(paths) => Ok((policy, paths)),
        Err(error) if dbx_core::mcp_policy::policy_uses_connection_groups(&policy) => {
            Err(AppError::from(format!("MCP_POLICY_UNAVAILABLE: {error}")))
        }
        Err(_) => Ok((policy, HashMap::new())),
    }
}

fn ensure_allowed(
    policy: &McpGlobalPolicy,
    group_path: Option<&McpConnectionGroupPath>,
    connection_id: &str,
) -> Result<(), AppError> {
    if !dbx_core::mcp_policy::policy_allows_connection(policy, group_path, connection_id) {
        return Err(AppError::from(format!(
            "CONNECTION_OUT_OF_SCOPE: connection '{connection_id}' is not allowed by DBX MCP settings"
        )));
    }
    Ok(())
}

fn ensure_database_in_scope(policy: &McpGlobalPolicy, connection_id: &str, database: &str) -> Result<(), AppError> {
    let Some(rule) = policy.connection_policies.iter().find(|rule| rule.connection_id == connection_id) else {
        return Ok(());
    };
    match rule.database_scope {
        McpDatabaseScope::All => Ok(()),
        McpDatabaseScope::None => Err(AppError::from(format!(
            "DATABASE_OUT_OF_SCOPE: database '{database}' is not allowed by DBX MCP settings for connection '{connection_id}'"
        ))),
        McpDatabaseScope::Selected if rule.allowed_databases.iter().any(|allowed| allowed == database) => Ok(()),
        McpDatabaseScope::Selected => Err(AppError::from(format!(
            "DATABASE_OUT_OF_SCOPE: database '{database}' is not allowed by DBX MCP settings for connection '{connection_id}'"
        ))),
    }
}

/// Execution modes are scoped defaults: a database setting overrides a
/// configured connection default, then the nearest configured group and the
/// global default.
/// Connection read-only and production protections are checked separately.
#[cfg(test)]
fn effective_database_execution_policy(policy: &McpGlobalPolicy, connection_id: &str, database: &str) -> (bool, bool) {
    effective_database_execution_policy_with_groups(policy, &[], connection_id, database)
}

fn effective_database_execution_policy_with_groups(
    policy: &McpGlobalPolicy,
    group_ids: &[String],
    connection_id: &str,
    database: &str,
) -> (bool, bool) {
    dbx_core::mcp_policy::effective_database_execution_policy_with_groups(policy, group_ids, connection_id, database)
}

fn ensure_sql_database_execution_scope(
    policy: &McpGlobalPolicy,
    connection: &ConnectionConfig,
    active_database: &str,
    sql: &str,
) -> Result<(), AppError> {
    dbx_core::mcp_policy::ensure_sql_database_execution_scope(policy, connection, active_database, sql)
        .map_err(AppError::from)
}

fn connection_read_only_error(message: impl Into<String>) -> AppError {
    AppError::from(format!("CONNECTION_READ_ONLY: {}", message.into()))
}

async fn load_connection(state: &Arc<WebState>, connection_id: &str) -> Result<ConnectionConfig, AppError> {
    state
        .app
        .storage
        .load_connections()
        .await
        .map_err(AppError::from)?
        .into_iter()
        .find(|config| config.id == connection_id)
        .ok_or_else(|| AppError::from(format!("Connection with id '{connection_id}' not found")))
}

pub async fn resolve_database(
    state: &Arc<WebState>,
    headers: &HeaderMap,
    connection_id: &str,
    database: &str,
) -> Result<String, AppError> {
    if !is_mcp_request(headers) {
        return Ok(database.to_string());
    }
    let config = load_connection(state, connection_id).await?;
    Ok(dbx_core::mcp_policy::resolve_database(database, config.database.as_deref()))
}

pub async fn ensure_scope(state: &Arc<WebState>, headers: &HeaderMap, connection_id: &str) -> Result<(), AppError> {
    if !is_mcp_request(headers) {
        return Ok(());
    }
    let (policy, group_paths) = load_policy_context(state).await?;
    ensure_allowed(&policy, group_paths.get(connection_id), connection_id)
}

pub async fn ensure_write(
    state: &Arc<WebState>,
    headers: &HeaderMap,
    connection_id: &str,
    database: &str,
    action: &str,
) -> Result<(), AppError> {
    ensure_write_with_risk(state, headers, connection_id, database, action, false).await
}

pub async fn ensure_dangerous_write(
    state: &Arc<WebState>,
    headers: &HeaderMap,
    connection_id: &str,
    database: &str,
    action: &str,
) -> Result<(), AppError> {
    ensure_write_with_risk(state, headers, connection_id, database, action, true).await
}

async fn ensure_write_with_risk(
    state: &Arc<WebState>,
    headers: &HeaderMap,
    connection_id: &str,
    database: &str,
    action: &str,
    dangerous: bool,
) -> Result<(), AppError> {
    if !is_mcp_request(headers) {
        return Ok(());
    }
    let (policy, group_paths) = load_policy_context(state).await?;
    let group_path = group_paths.get(connection_id);
    ensure_allowed(&policy, group_path, connection_id)?;
    let config = load_connection(state, connection_id).await?;
    let database = dbx_core::mcp_policy::resolve_database(database, config.database.as_deref());
    ensure_database_in_scope(&policy, connection_id, &database)?;
    let group_ids = group_path.map(|path| path.ids.as_slice()).unwrap_or_default();
    let (read_only, allow_dangerous_sql) =
        effective_database_execution_policy_with_groups(&policy, group_ids, connection_id, &database);
    if read_only {
        return Err(AppError::from(format!(
            "MCP_READ_ONLY: MCP execution permission for database '{database}' is read-only. {action} blocked."
        )));
    }
    if dangerous && !allow_dangerous_sql {
        return Err(AppError::from(format!(
            "SQL_BLOCKED: High-risk operation '{action}' is disabled in DBX MCP settings."
        )));
    }
    if config.read_only {
        return Err(connection_read_only_error(format!(
            "Connection '{}' has read-only protection enabled. {action} blocked.",
            config.name
        )));
    }
    if dbx_core::production_safety::is_production_database(&config, &database) {
        return Err(AppError::from(format!(
            "PRODUCTION_DATABASE_READ_ONLY: {action} blocked for production database '{database}'."
        )));
    }
    Ok(())
}

pub async fn ensure_sql(
    state: &Arc<WebState>,
    headers: &HeaderMap,
    connection_id: &str,
    database: &str,
    sql: &str,
    allow_database_switch: bool,
) -> Result<String, AppError> {
    if !is_mcp_request(headers) {
        return Ok(database.to_string());
    }
    let (policy, group_paths) = load_policy_context(state).await?;
    let group_path = group_paths.get(connection_id);
    ensure_allowed(&policy, group_path, connection_id)?;
    let config = load_connection(state, connection_id).await?;
    let database = dbx_core::mcp_policy::resolve_database(database, config.database.as_deref());
    ensure_database_in_scope(&policy, connection_id, &database)?;
    if !allow_database_switch && dbx_core::sql_risk::mcp_sql_has_forbidden_database_switch(sql, config.db_type) {
        return Err(AppError::from(
            "SQL_BLOCKED: MCP does not allow USE or persistent database switching.".to_string(),
        ));
    }
    let is_write = dbx_core::query_execution_sql::is_write_sql_for_database(sql, config.db_type);
    ensure_sql_database_execution_scope(&policy, &config, &database, sql)?;
    let group_ids = group_path.map(|path| path.ids.as_slice()).unwrap_or_default();
    let (read_only, allow_dangerous_sql) =
        effective_database_execution_policy_with_groups(&policy, group_ids, connection_id, &database);
    if read_only && is_write {
        return Err(AppError::from(format!(
            "MCP_READ_ONLY: MCP execution permission for database '{database}' is read-only. SQL write blocked."
        )));
    }
    if !allow_dangerous_sql && dbx_core::sql_risk::is_dangerous_sql_for_database(sql, config.db_type) {
        return Err(AppError::from("SQL_BLOCKED: High-risk SQL is disabled in DBX MCP settings.".to_string()));
    }
    if config.read_only {
        dbx_core::query_execution_sql::check_read_only(sql, &config.name, config.db_type)
            .map_err(connection_read_only_error)?;
    }
    if is_write && dbx_core::production_safety::targets_production_database(&config, &database, sql) {
        return Err(AppError::from(
            "PRODUCTION_DATABASE_READ_ONLY: SQL write targeting production scope is blocked.".to_string(),
        ));
    }
    Ok(database)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::storage::{McpConnectionPolicy, McpDatabasePolicy, McpDatabaseScope, McpGlobalPolicy};

    fn database_policy() -> McpGlobalPolicy {
        McpGlobalPolicy {
            read_only: true,
            allow_dangerous_sql: false,
            connection_policies: vec![McpConnectionPolicy {
                connection_id: "conn-1".to_string(),
                read_only: true,
                allow_dangerous_sql: false,
                execution_mode_configured: true,
                execution_mode_policy_version: Some(dbx_core::mcp_policy::MCP_EXECUTION_POLICY_VERSION),
                database_scope: McpDatabaseScope::Selected,
                allowed_databases: vec!["operations".to_string(), "reporting".to_string()],
                database_policies: vec![McpDatabasePolicy {
                    database_name: "operations".to_string(),
                    read_only: false,
                    allow_dangerous_sql: true,
                }],
                allow_salesforce_dml: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn database_execution_policy_overrides_global_and_connection_defaults() {
        let policy = database_policy();

        assert_eq!(effective_database_execution_policy(&policy, "conn-1", "operations"), (false, true));
        assert_eq!(effective_database_execution_policy(&policy, "conn-1", "reporting"), (true, false));
    }

    #[test]
    fn connection_read_only_errors_use_the_stable_mcp_code() {
        assert_eq!(connection_read_only_error("write blocked").message, "CONNECTION_READ_ONLY: write blocked");
    }
}
