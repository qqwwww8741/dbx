use std::time::Duration;

use serde_json::Value;

use crate::connection::{AppState, PoolKind};
use crate::models::connection::{ConnectionConfig, DatabaseType};
use crate::query_execution_sql::{is_safe_dameng_autotrace_sql, is_safe_explain_sql_for_database};

/// Acquires an estimated plan for an agent-backed connection.
///
/// `timeout_secs` overrides the connection's own query timeout for this call
/// (`None` keeps the connection default). Overrides are host-owned: the plugin
/// Host plan API clamps the value before it gets here.
pub async fn get_agent_explain_info_core(
    state: &AppState,
    connection_id: &str,
    database: Option<&str>,
    schema: Option<&str>,
    sql: &str,
    mode: Option<&str>,
    timeout_secs: Option<u64>,
) -> Result<String, String> {
    let mode = mode.unwrap_or("explain");
    let (database_type, timeout_secs) = {
        let configs = state.configs.read().await;
        let config = configs.get(connection_id).ok_or_else(|| "Connection config not found".to_string())?.clone();
        let database_type = explain_database_type(&config);
        let timeout_secs = timeout_secs.unwrap_or_else(|| config.effective_query_timeout_secs());
        (database_type, timeout_secs)
    };
    if !is_safe_agent_explain_sql(sql, mode, database_type) {
        return Err("unsafe".to_string());
    }

    let database_for_pool = database.filter(|value| !value.trim().is_empty());
    let pool_key = state.get_or_create_pool(connection_id, database_for_pool).await?;

    enum ExplainTarget {
        Agent(std::sync::Arc<crate::driver_error::PooledAgentClient>),
        External {
            config: std::sync::Arc<ConnectionConfig>,
            session: std::sync::Arc<crate::plugins::PluginDriverSession>,
        },
    }

    let target = {
        let pool_handle = state.pool_handle(&pool_key).await;
        let pool = pool_handle.as_ref().ok_or_else(|| "Connection not found".to_string())?;
        match pool {
            _ => return Err("Connection is not an agent-based connection".to_string()),
        }
    };

    let params = serde_json::json!({
        "sql": sql,
        "database": database.unwrap_or_default(),
        "schema": schema.unwrap_or_default(),
        "timeoutSecs": timeout_secs as i64,
        "mode": mode,
    });
    let result: Value = match target {
        ExplainTarget::Agent(client) => client.lock().await.get_explain_info(params).await?,
        ExplainTarget::External { config, session } => {
            let mut params = params;
            params["connection"] = serde_json::to_value(config.as_ref()).map_err(|error| error.to_string())?;
            let timeout = (timeout_secs > 0).then(|| Duration::from_secs(timeout_secs));
            session.invoke_with_timeout("getExplainInfo", params, timeout).await?
        }
    };
    decode_agent_explain_result(result)
}

/// Resolves the dialect an EXPLAIN must be generated for. Doris' MySQL-based
/// connection profiles are Doris dialects, not MySQL. A custom JDBC connection
/// is only known to be Oracle when its configuration says so; every other
/// custom JDBC connection stays `Jdbc` and therefore gets no plan acquisition.
/// Shared with the plugin Host plan API so both paths agree on which dialects
/// are treatable.
pub(crate) fn explain_database_type(config: &ConnectionConfig) -> DatabaseType {
    {}
    {
        return config.db_type;
    }
}

fn is_safe_agent_explain_sql(sql: &str, mode: &str, database_type: DatabaseType) -> bool {
    if mode.eq_ignore_ascii_case("autotrace") {
        false
    } else {
        is_safe_explain_sql_for_database(sql, Some(database_type))
    }
}

fn decode_agent_explain_result(result: Value) -> Result<String, String> {
    match result {
        Value::String(plan) => Ok(plan),
        Value::Object(object) => Ok(object.get("plan").and_then(Value::as_str).unwrap_or_default().to_string()),
        value => Err(format!("Unexpected result type from getExplainInfo: {value:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::connection::PoolKind;
    use crate::models::connection::DatabaseType;
    #[cfg(unix)]
    use crate::plugins::{
        InstalledPlugin, PluginDriverManifest, PluginDriverSession, PluginManifest, PluginRuntimeEnv,
    };
    use crate::query_execution_sql::{build_explain_sql, ExplainSqlOptions};
    #[cfg(unix)]
    use std::sync::Arc;

    #[test]
    fn decodes_string_and_object_agent_explain_results() {
        assert_eq!(decode_agent_explain_result(Value::String("plan text".to_string())).unwrap(), "plan text");
        assert_eq!(
            decode_agent_explain_result(serde_json::json!({ "plan": "object plan", "has_actual_stats": false }))
                .unwrap(),
            "object plan"
        );
        assert!(decode_agent_explain_result(serde_json::json!(["unexpected"])).is_err());
    }
}
