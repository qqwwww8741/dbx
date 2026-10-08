use std::collections::HashMap;

use crate::models::connection::ConnectionConfig;
use crate::production_safety::sql_references_disallowed_database;
use crate::storage::McpGlobalPolicy;
use serde::Deserialize;
use serde_json::Value;

/// Version used by rules that opt into scoped override semantics.
pub const MCP_EXECUTION_POLICY_VERSION: u8 = 1;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpConnectionGroupPath {
    pub ids: Vec<String>,
    pub names: Vec<String>,
}

#[derive(Deserialize)]
struct SidebarLayout {
    #[serde(default)]
    groups: Vec<SidebarGroup>,
    #[serde(default)]
    order: Vec<SidebarOrderEntry>,
}

#[derive(Deserialize)]
struct SidebarGroup {
    id: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum SidebarOrderEntry {
    #[serde(rename = "group")]
    Group {
        id: String,
        children: Option<Vec<SidebarOrderEntry>>,
        #[serde(rename = "connectionIds")]
        connection_ids: Option<Vec<String>>,
    },
    #[serde(rename = "connection")]
    Connection { id: String },
}

/// Resolve each connection's stable group ids and display names from the
/// persisted sidebar tree. Legacy `connectionIds` group entries remain valid.
pub fn connection_group_paths(layout: &Value) -> Result<HashMap<String, McpConnectionGroupPath>, String> {
    let layout = serde_json::from_value::<SidebarLayout>(layout.clone())
        .map_err(|error| format!("INVALID_SIDEBAR_LAYOUT: {error}"))?;
    let groups = layout.groups.into_iter().map(|group| (group.id, group.name)).collect::<HashMap<_, _>>();
    let mut paths = HashMap::new();
    collect_connection_group_paths(&layout.order, &groups, &mut McpConnectionGroupPath::default(), &mut paths)?;
    Ok(paths)
}

fn collect_connection_group_paths(
    entries: &[SidebarOrderEntry],
    groups: &HashMap<String, String>,
    path: &mut McpConnectionGroupPath,
    paths: &mut HashMap<String, McpConnectionGroupPath>,
) -> Result<(), String> {
    for entry in entries {
        match entry {
            SidebarOrderEntry::Connection { id } => {
                paths.insert(id.clone(), path.clone());
            }
            SidebarOrderEntry::Group { id, children, connection_ids } => {
                let name =
                    groups.get(id).ok_or_else(|| format!("INVALID_SIDEBAR_LAYOUT: group '{id}' has no metadata"))?;
                path.ids.push(id.clone());
                path.names.push(name.clone());
                if let Some(children) = children {
                    collect_connection_group_paths(children, groups, path, paths)?;
                } else if let Some(connection_ids) = connection_ids {
                    for connection_id in connection_ids {
                        paths.insert(connection_id.clone(), path.clone());
                    }
                }
                path.ids.pop();
                path.names.pop();
            }
        }
    }
    Ok(())
}

pub fn policy_uses_connection_groups(policy: &McpGlobalPolicy) -> bool {
    !policy.allowed_group_ids.is_empty() || !policy.group_policies.is_empty()
}

pub fn policy_allows_connection(
    policy: &McpGlobalPolicy,
    group_path: Option<&McpConnectionGroupPath>,
    connection_id: &str,
) -> bool {
    policy.allowed_connection_ids.as_ref().is_none_or(|allowed| {
        allowed.iter().any(|id| id == connection_id)
            || group_path.is_some_and(|path| {
                path.ids.iter().any(|id| policy.allowed_group_ids.iter().any(|allowed| allowed == id))
            })
    })
}

/// Resolve an omitted or blank request database to the connection default.
pub fn resolve_database(requested: &str, configured: Option<&str>) -> String {
    let requested = requested.trim();
    if requested.is_empty() {
        configured.unwrap_or_default().trim().to_string()
    } else {
        requested.to_string()
    }
}

/// Compute the effective execution mode while preserving legacy rule ceilings.
/// Rules without the current version marker are never allowed to widen the
/// global policy, including when they contain fields added by a newer client.
pub fn effective_database_execution_policy(
    policy: &McpGlobalPolicy,
    connection_id: &str,
    database: &str,
) -> (bool, bool) {
    effective_database_execution_policy_with_groups(policy, &[], connection_id, database)
}

pub fn effective_database_execution_policy_with_groups(
    policy: &McpGlobalPolicy,
    group_ids: &[String],
    connection_id: &str,
    database: &str,
) -> (bool, bool) {
    let mut effective = (policy.read_only, policy.allow_dangerous_sql);
    for group_id in group_ids {
        if let Some(rule) = policy.group_policies.iter().find(|rule| rule.group_id == *group_id) {
            effective = (rule.read_only, !rule.read_only && rule.allow_dangerous_sql);
        }
    }
    let Some(rule) = policy.connection_policies.iter().find(|rule| rule.connection_id == connection_id) else {
        return effective;
    };

    if rule.execution_mode_policy_version == Some(MCP_EXECUTION_POLICY_VERSION) {
        if rule.execution_mode_configured {
            effective = (rule.read_only, !rule.read_only && rule.allow_dangerous_sql);
        }
        if let Some(database_policy) = rule.database_policies.iter().find(|rule| rule.database_name == database) {
            effective = (database_policy.read_only, !database_policy.read_only && database_policy.allow_dangerous_sql);
        }
    } else {
        // Legacy rules only carried a ceiling when the old UI had configured
        // an execution mode; scope-only rules inherited the global mode
        // untouched and must keep doing so instead of being narrowed by the
        // implicit (false, false) ceiling.
        if rule.execution_mode_configured {
            effective = apply_ceiling(effective, (rule.read_only, rule.allow_dangerous_sql));
        }
        if let Some(database_policy) = rule.database_policies.iter().find(|rule| rule.database_name == database) {
            effective = apply_ceiling(effective, (database_policy.read_only, database_policy.allow_dangerous_sql));
        }
    }
    effective
}

fn apply_ceiling(current: (bool, bool), ceiling: (bool, bool)) -> (bool, bool) {
    let read_only = current.0 || ceiling.0;
    (read_only, !read_only && current.1 && ceiling.1)
}

/// Whether an AI agent may write to this Salesforce connection through MCP.
///
/// A pure per-connection opt-in: no global or group setting can turn it on, and
/// a connection without a saved rule stays off. Callers still run every write
/// through the ordinary execution policy, so the global/connection read-only
/// modes, connection read-only protection and production protection all remain
/// upper bounds on top of this switch.
pub fn connection_allows_salesforce_dml(policy: &McpGlobalPolicy, connection_id: &str) -> bool {
    false
}

/// Reject qualified SQL references when database-specific execution rules are
/// present and individual referenced databases have not been evaluated.
pub fn ensure_sql_database_execution_scope(
    policy: &McpGlobalPolicy,
    connection: &ConnectionConfig,
    active_database: &str,
    sql: &str,
) -> Result<(), String> {
    let Some(rule) = policy.connection_policies.iter().find(|rule| rule.connection_id == connection.id) else {
        return Ok(());
    };
    if rule.database_policies.is_empty()
        || !sql_references_disallowed_database(
            sql,
            &connection.db_type,
            active_database,
            &[active_database.to_string()],
        )
    {
        return Ok(());
    }
    Err(
        "DATABASE_EXECUTION_POLICY_OUT_OF_SCOPE: SQL cannot reference another database while database-specific MCP execution permissions are configured."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        connection_allows_salesforce_dml, effective_database_execution_policy, resolve_database,
        MCP_EXECUTION_POLICY_VERSION,
    };
    use crate::storage::{McpConnectionPolicy, McpDatabasePolicy, McpGlobalPolicy};

    fn policy(version: Option<u8>) -> McpGlobalPolicy {
        McpGlobalPolicy {
            read_only: true,
            connection_policies: vec![McpConnectionPolicy {
                connection_id: "conn".to_string(),
                read_only: false,
                allow_dangerous_sql: false,
                execution_mode_configured: true,
                execution_mode_policy_version: version,
                database_scope: Default::default(),
                allowed_databases: Vec::new(),
                database_policies: vec![McpDatabasePolicy {
                    database_name: "db".to_string(),
                    read_only: false,
                    allow_dangerous_sql: true,
                }],
                allow_salesforce_dml: false,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn legacy_rules_cannot_widen_global_read_only() {
        assert_eq!(effective_database_execution_policy(&policy(None), "conn", "db"), (true, false));
    }

    #[test]
    fn current_rules_use_database_override() {
        assert_eq!(
            effective_database_execution_policy(&policy(Some(MCP_EXECUTION_POLICY_VERSION)), "conn", "db"),
            (false, true)
        );
    }

    #[test]
    fn legacy_connection_ceiling_is_preserved_for_safe_write_rule() {
        let mut policy = policy(None);
        policy.read_only = false;
        policy.allow_dangerous_sql = true;
        policy.connection_policies[0].allow_dangerous_sql = false;
        policy.connection_policies[0].database_policies.clear();
        assert_eq!(effective_database_execution_policy(&policy, "conn", "db"), (false, false));
    }

    #[test]
    fn legacy_scope_only_rules_inherit_global_mode() {
        let mut policy = policy(None);
        policy.read_only = false;
        policy.allow_dangerous_sql = true;
        policy.connection_policies[0].execution_mode_configured = false;
        policy.connection_policies[0].database_policies.clear();
        // Old-UI scope-only rules (configured=false, no mode saved) inherited
        // the global mode verbatim; the implicit (false, false) values must
        // not narrow allow_dangerous_sql on upgrade.
        assert_eq!(effective_database_execution_policy(&policy, "conn", "db"), (false, true));
    }

    #[test]
    fn blank_database_uses_connection_default() {
        assert_eq!(resolve_database("  ", Some("sample")), "sample");
        assert_eq!(resolve_database("analytics", Some("sample")), "analytics");
    }
}
