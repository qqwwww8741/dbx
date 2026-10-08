use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};

use serde_json::json;
use serde_json::Value;

use crate::agent_events::{ToolCall, ToolDefinition, ToolResult};
use crate::connection::AppState;

use crate::models::connection::DatabaseType;
use crate::models::connection::{ConnectionConfig, SPANNER_MIN_QUERY_TIMEOUT_SECS};
use crate::query::QueryExecutionOptions;
use crate::query_execution_sql::{build_explain_sql, supports_explain_plan, supports_sql_query, ExplainSqlOptions};
use crate::sql_dialect::{build_table_data_select_sql, TableDataSelectSqlOptions};
use crate::sql_risk::SqlRisk;
use crate::types::{QueryMessage, QueryResult};

/// Maximum number of tables returned by list_tables tool.
const LIST_TABLES_LIMIT: usize = 200;

/// Maximum number of rows returned by execute_query tool.
const EXECUTE_QUERY_LIMIT: usize = 50;

/// Maximum number of rows returned by get_sample_data tool.
const SAMPLE_DATA_LIMIT: usize = 20;

/// Maximum number of rows returned by browse_collection tool.
const BROWSE_COLLECTION_LIMIT: usize = 20;

/// Absolute maximum rows requested by the sampling tools (get_sample_data,
/// browse_collection) and by execute_query on MongoDB shell commands.
const MAX_ALLOWED_ROWS: usize = 100;

/// Absolute maximum rows `execute_query` may request on SQL connections.
/// MCP publishes this as the `max_rows` tool parameter; kept below the driver
/// fetch ceiling (`dbx_drivers::execution::MAX_ROWS = 10000`).
pub const MAX_EXECUTE_QUERY_ROWS: usize = 1_000;

/// The published "up to `MAX_EXECUTE_QUERY_ROWS` rows" contract only holds while this
/// ceiling stays below what the driver will actually fetch. If the agent ceiling ever met
/// or exceeded the driver ceiling, a large `max_rows` request would come back truncated by
/// the driver instead of by our own cap, and the tool description would be a lie.
///
/// Asserted at compile time so it cannot be silently dropped.
const _: () = assert!(MAX_EXECUTE_QUERY_ROWS < dbx_drivers::execution::MAX_ROWS);

/// Default string-cell character budget for AI and local MCP query results.
const DEFAULT_QUERY_CELL_CHAR_LIMIT: usize = 200;

/// Explicit string-cell windows stay bounded so one tool call cannot flood the model context.
const MAX_QUERY_CELL_CHAR_LIMIT: usize = 4_000;

/// Bounds explicit sliding-window scans without changing the underlying database request.
const MAX_QUERY_CELL_CHAR_OFFSET: usize = 1_000_000;

fn connection_tool_lock(connection_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<StdMutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut registry = locks.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    registry.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = registry.get(connection_id).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    registry.insert(connection_id.to_string(), Arc::downgrade(&lock));
    lock
}

fn tool_uses_database(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "list_databases"
            | "list_tables"
            | "get_columns"
            | "execute_query"
            | "get_sample_data"
            | "list_collections"
            | "browse_collection"
            | "explain_query"
            | "execute_redis_command"
    )
}

/// Resolve the query timeout (seconds) for an agent/MCP query.
///
/// Precedence: an explicit per-call `requested` value wins; otherwise inherit
/// the connection's `effective_query_timeout_secs()`; the legacy 30s survives
/// only when no connection config is available to resolve against. `Some(0)`
/// from either source means "no limit" (unlimited, and it bypasses the Spanner
/// floor — see `ConnectionConfig::effective_query_timeout_secs`). A finite
/// value on a Spanner connection gets the 120s floor.
pub fn agent_query_timeout_secs(requested: Option<u64>, connection: Option<&ConnectionConfig>) -> u64 {
    match requested {
        Some(0) => 0,
        Some(n) => n,
        None => connection.map_or(QUERY_TIMEOUT_SECS, ConnectionConfig::effective_query_timeout_secs),
    }
}

/// Legacy fallback query timeout for agent queries. Matches the current fixed
/// `Some(30)` behavior and remains the last resort when no connection config
/// can be resolved (that path fails the query at pool lookup anyway).
const QUERY_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueryCellWindow {
    offset: usize,
    limit: usize,
}

impl Default for QueryCellWindow {
    fn default() -> Self {
        Self { offset: 0, limit: DEFAULT_QUERY_CELL_CHAR_LIMIT }
    }
}

impl QueryCellWindow {
    pub fn from_options(offset: Option<u64>, limit: Option<u64>) -> Self {
        Self {
            offset: bounded_query_cell_option(offset, 0, MAX_QUERY_CELL_CHAR_OFFSET),
            limit: bounded_query_cell_option(limit, DEFAULT_QUERY_CELL_CHAR_LIMIT, MAX_QUERY_CELL_CHAR_LIMIT).max(1),
        }
    }

    pub fn from_arguments(arguments: &serde_json::Value) -> Self {
        Self::from_options(
            arguments.get("cell_char_offset").and_then(serde_json::Value::as_u64),
            arguments.get("cell_char_limit").and_then(serde_json::Value::as_u64),
        )
    }

    pub fn explicit_from_arguments(arguments: &serde_json::Value) -> Option<Self> {
        let offset = arguments.get("cell_char_offset").and_then(serde_json::Value::as_u64);
        let limit = arguments.get("cell_char_limit").and_then(serde_json::Value::as_u64);
        (offset.is_some() || limit.is_some()).then(|| Self::from_options(offset, limit))
    }
}

fn bounded_query_cell_option(value: Option<u64>, default: usize, maximum: usize) -> usize {
    value.map(|value| value.min(maximum as u64) as usize).unwrap_or(default)
}

/// Resolve the row cap for execute_query. Splitting by db_type is deliberate:
/// SQL connections may go up to MAX_EXECUTE_QUERY_ROWS, while the MongoDB shell
/// path keeps MAX_ALLOWED_ROWS, so widening the SQL ceiling does not also
/// widen what a Mongo command can return.
fn execute_query_row_limit(requested: Option<usize>, db_type: &DatabaseType) -> usize {
    let ceiling = { MAX_EXECUTE_QUERY_ROWS };
    requested.map(|rows| rows.min(ceiling)).unwrap_or(EXECUTE_QUERY_LIMIT.min(ceiling))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentSqlPermissions {
    pub allow_writes: bool,
    pub allow_dangerous: bool,
    /// When present, write/DDL execute_query calls must match this SQL
    /// (after trimming only surrounding whitespace). Set by the frontend
    /// when the user confirms a specific write-SQL proposal.
    pub confirmed_write_sql: Option<String>,
}

/// Build the write permissions for one AI-agent run from an explicit user
/// confirmation. Both Desktop and Web use this fail-closed boundary so an
/// empty confirmation or a production target cannot enable writes.
pub fn confirmed_write_sql_permissions(
    production_database: bool,
    allow_write_sql: bool,
    confirmed_write_sql: Option<String>,
) -> AgentSqlPermissions {
    let confirmed_write_sql = confirmed_write_sql.filter(|sql| !sql.trim().is_empty());
    let write_sql_confirmed = !production_database && allow_write_sql && confirmed_write_sql.is_some();

    AgentSqlPermissions {
        allow_writes: write_sql_confirmed,
        allow_dangerous: write_sql_confirmed,
        confirmed_write_sql: write_sql_confirmed.then_some(confirmed_write_sql).flatten(),
    }
}

/// Verify that the confirmed connection/database snapshot matches the actual
/// target. Returns `(allow_write_sql, confirmed_write_sql)` — when the target
/// does not match, the grant is voided (allow=false, confirmed=None).
///
/// This is defense-in-depth: the frontend also verifies synchronously, but
/// this backend check protects CLI-provider and API-driven paths.
pub fn verify_confirmed_target(
    allow_write_sql: Option<bool>,
    confirmed_write_sql: Option<String>,
    confirmed_connection_id: Option<String>,
    confirmed_database: Option<String>,
    confirmed_schema: Option<String>,
    actual_connection_id: &str,
    actual_database: &str,
    actual_schema: Option<&str>,
) -> (Option<bool>, Option<String>) {
    let Some(ref confirmed_sql) = confirmed_write_sql else {
        return (allow_write_sql, confirmed_write_sql);
    };
    // Only verify when a write SQL was actually confirmed.
    let target_mismatch = confirmed_connection_id.as_deref() != Some(actual_connection_id)
        || confirmed_database.as_deref() != Some(actual_database)
        || confirmed_schema.as_deref() != actual_schema;
    if target_mismatch {
        log::warn!(
            "Write-SQL grant voided: confirmed target (conn={:?}, db={:?}, schema={:?}) does not match actual (conn={}, db={}, schema={:?}).",
            confirmed_connection_id,
            confirmed_database,
            confirmed_schema,
            actual_connection_id,
            actual_database,
            actual_schema,
        );
        // SQL can contain literals or credentials. Keep diagnostic visibility
        // behind the shared debug-only redaction boundary.
        crate::sql_diagnostics::debug_sql("write_sql_grant_voided", confirmed_sql);
        return (Some(false), None);
    }
    (allow_write_sql, confirmed_write_sql)
}

fn sql_risk_allowed(risk: SqlRisk, permissions: AgentSqlPermissions) -> bool {
    match risk {
        SqlRisk::ReadOnly => true,
        SqlRisk::Write => permissions.allow_writes,
        SqlRisk::Ddl => permissions.allow_dangerous,
        SqlRisk::Transaction => false,
    }
}

/// Returns true when an Agent attempted a write or DDL call before DBX has a
/// user-confirmed SQL binding for the current run. The caller must turn that
/// attempt into a confirmation proposal instead of sending it to the database.
pub fn write_requires_confirmation(
    sql: &str,
    db_type: DatabaseType,
    permissions: &AgentSqlPermissions,
) -> Result<bool, String> {
    if permissions.allow_writes || permissions.allow_dangerous || permissions.confirmed_write_sql.is_some() {
        return Ok(false);
    }
    let risk = crate::sql_risk::classify_sql_risk_for_database(sql, db_type)?;
    Ok(matches!(risk, SqlRisk::Write | SqlRisk::Ddl))
}

/// Snapshot the permissions for one execute_query call and consume an exact
/// confirmed write/DDL grant before dispatch. Later calls in the same agent run
/// receive the cleared permissions and must request a new confirmation.
pub(crate) fn take_sql_permissions_for_execution(
    sql: &str,
    db_type: DatabaseType,
    permissions: &mut AgentSqlPermissions,
) -> AgentSqlPermissions {
    let execution_permissions = permissions.clone();
    let consumes_confirmation = match crate::sql_risk::classify_sql_risk_for_database(sql, db_type) {
        Ok(risk @ (SqlRisk::Write | SqlRisk::Ddl)) => {
            sql_risk_allowed(risk, execution_permissions.clone())
                && sql_matches_confirmed_write(sql, &execution_permissions.confirmed_write_sql)
                && execution_permissions.confirmed_write_sql.is_some()
        }
        _ => false,
    };
    if consumes_confirmation {
        *permissions = AgentSqlPermissions::default();
    }
    execution_permissions
}

/// Returns true for vector database types (Qdrant, Milvus, Weaviate, ChromaDb).
/// If modifying this, also update VECTOR_DB_TYPES in apps/desktop/src/lib/ai.ts.
pub fn is_vector_db(db_type: DatabaseType) -> bool {
    false
}

/// `get_current_time` tool definition — DB-independent utility that returns
/// the current UTC time plus a caller-provided local offset.
fn get_current_time_tool() -> ToolDefinition {
    ToolDefinition {
        name: "get_current_time".into(),
        description: "Get the current date and time with timezone information. \
                      Pass the client UTC offset from the system prompt; when \
                      omitted, local time safely falls back to UTC. Use this to resolve \
                      relative time expressions like \"last 7 days\", \
                      \"yesterday\", \"this month\" into concrete dates \
                      for constructing SQL queries."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "utc_offset_minutes": {
                    "type": "integer",
                    "minimum": -1439,
                    "maximum": 1439,
                    "description": "Client UTC offset in minutes from the system prompt"
                },
                "timezone": {
                    "type": "string",
                    "description": "Client IANA timezone name from the system prompt"
                }
            },
            "required": []
        }),
        read_only: true,
        parallel_ok: true,
    }
}

/// Execute `get_current_time` — returns a JSON payload with utc, local,
/// utc_offset_minutes, timezone, and readable fields.
fn execute_get_current_time(tool_call: &ToolCall) -> Result<String, String> {
    let utc = chrono::Utc::now();
    let offset_minutes = tool_call.arguments.get("utc_offset_minutes").and_then(serde_json::Value::as_i64).unwrap_or(0);
    let offset_minutes = i32::try_from(offset_minutes).map_err(|_| "utc_offset_minutes is out of range".to_string())?;
    let offset_seconds =
        offset_minutes.checked_mul(60).ok_or_else(|| "utc_offset_minutes is out of range".to_string())?;
    let offset = chrono::FixedOffset::east_opt(offset_seconds)
        .ok_or_else(|| "utc_offset_minutes must be between -1439 and 1439".to_string())?;
    let local = utc.with_timezone(&offset);
    let timezone = tool_call
        .arguments
        .get("timezone")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| if offset_minutes == 0 { "UTC".to_string() } else { format!("UTC{}", local.format("%:z")) });
    let readable = format!("{} ({timezone}, UTC{})", local.format("%Y-%m-%d %H:%M:%S"), local.format("%:z"));
    Ok(serde_json::json!({
        "utc": utc.to_rfc3339(),
        "local": local.to_rfc3339(),
        "utc_offset_minutes": offset_minutes,
        "timezone": timezone,
        "readable": readable,
    })
    .to_string())
}

/// Get read-only tool definitions for the given database type.
/// Returns vector tools for vector DBs, SQL tools otherwise.
pub fn read_only_tools(db_type: DatabaseType) -> Vec<ToolDefinition> {
    if is_vector_db(db_type) {
        vec![list_collections_tool(), get_current_time_tool()]
    } else {
        vec![list_databases_tool(), list_tables_tool(), get_columns_tool(db_type), get_current_time_tool()]
    }
}

/// Get all available tool definitions for the given database type.
/// Includes read-only tools plus execute_query, get_sample_data, and
/// explain_query for database types that support them.
pub fn all_tools(db_type: DatabaseType, sql_permissions: AgentSqlPermissions) -> Vec<ToolDefinition> {
    {}
    if is_vector_db(db_type) {
        return vec![list_collections_tool(), browse_collection_tool(), get_current_time_tool()];
    }
    let mut tools = vec![list_databases_tool(), list_tables_tool(), get_columns_tool(db_type), get_current_time_tool()];
    if supports_sql_query(db_type) {
        tools.push(execute_query_tool(sql_permissions));
        tools.push(get_sample_data_tool());
    }
    if supports_explain_plan(Some(db_type)) {
        tools.push(explain_query_tool());
    }
    tools
}

fn list_databases_tool() -> ToolDefinition {
    ToolDefinition {
        name: "list_databases".into(),
        description: "List databases available through the current connection. If more than one database is returned, cross-database read queries can use fully qualified names such as database.table (or database.schema.table for SQL Server).".into(),
        parameters: json!({"type": "object", "properties": {}, "required": []}),
        read_only: true,
        parallel_ok: true,
    }
}

/// list_tables tool definition.
fn list_tables_tool() -> ToolDefinition {
    ToolDefinition {
        name: "list_tables".into(),
        description: "List all tables and views in the current database. Returns table names, types, and comments."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "schema": {
                    "type": "string",
                    "description": "Schema name to list tables from (optional, defaults to current database)"
                },
                "database": {
                    "type": "string",
                    "description": "Database to inspect (optional, defaults to the current database)"
                }
            },
            "required": []
        }),
        read_only: true,
        parallel_ok: true,
    }
}

/// get_columns tool definition.
fn get_columns_tool(db_type: DatabaseType) -> ToolDefinition {
    ToolDefinition {
        name: "get_columns".into(),
        description: {
            "Get column definitions for a table: names, types, primary keys, nullable, defaults, and comments. \
             Use this when the user asks about table structure, column details, or field information — \
             even if some schema context was provided, this tool returns the authoritative and complete column list."
                .into()
        },
        parameters: json!({
            "type": "object",
            "properties": {
                "table": {
                    "type": "string",
                    "description": "Table name to get columns for"
                },
                "schema": {
                    "type": "string",
                    "description": "Schema name (optional, defaults to current database)"
                },
                "database": {
                    "type": "string",
                    "description": "Database containing the table (optional, defaults to the current database)"
                }
            },
            "required": ["table"]
        }),
        read_only: true,
        // get_columns runs sequentially: concurrent metadata queries can exhaust
        // single-connection drivers (e.g. DuckDB), causing cascading tool errors.
        parallel_ok: false,
    }
}
/// execute_query tool definition.
fn execute_query_tool(sql_permissions: AgentSqlPermissions) -> ToolDefinition {
    let description = if sql_permissions.allow_dangerous {
        "Execute SQL after the user explicitly confirmed this operation. Read queries, writes, and DDL are allowed for this run."
    } else if sql_permissions.allow_writes {
        "Execute SQL after the user explicitly confirmed this operation. Read queries and non-DDL writes are allowed for this run."
    } else {
        "Execute a read-only SQL query and return results (default 50 rows, up to 1000 with the limit argument). Cross-database reads may use fully qualified names such as database.table (or database.schema.table for SQL Server) without switching the current database. This run cannot execute writes or DDL because no specific SQL has been confirmed yet; this does not mean the database itself is read-only. When the user requests a write, first propose the exact SQL in one ```sql code block and ask for confirmation. After confirmation, DBX starts a new run that can execute only that exact SQL. Only SELECT, WITH, SHOW, DESCRIBE, EXPLAIN statements may be executed in this run."
    };
    ToolDefinition {
        name: "execute_query".into(),
        description: description.into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "sql": {
                    "type": "string",
                    "description": "The SQL query to execute"
                },
                "limit": {
                    "type": "number",
                    "minimum": 1,
                    "maximum": MAX_EXECUTE_QUERY_ROWS,
                    "description": format!("Max rows to return (default {EXECUTE_QUERY_LIMIT}, max {MAX_EXECUTE_QUERY_ROWS})")
                },
                "cell_char_offset": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 1000000,
                    "description": "Start character offset for every string cell (default 0). Use the next offset reported by a truncated result to slide through long values. Narrow the query to the target row and column before expanding."
                },
                "cell_char_limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 4000,
                    "description": "Maximum characters returned per string cell (default 200, max 4000). Increase only for an explicit long-value expansion."
                },
                "client_session_id": {
                    "type": "string",
                    "description": "Opaque DBX session handle that pins this query to the same backend connection as earlier queries in the session (preserves USE/SET/session state). Managed by DBX; agents should not invent values."
                }
            },
            "required": ["sql"]
        }),
        read_only: true,
        parallel_ok: false,
    }
}

/// get_sample_data tool definition.
fn get_sample_data_tool() -> ToolDefinition {
    ToolDefinition {
        name: "get_sample_data".into(),
        description: "Get sample rows from a table to understand its data. Returns up to 20 rows.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "table": {
                    "type": "string",
                    "description": "Table name"
                },
                "schema": {
                    "type": "string",
                    "description": "Schema name (optional)"
                },
                "database": {
                    "type": "string",
                    "description": "Database containing the table (optional, defaults to the current database)"
                },
                "limit": {
                    "type": "number",
                    "description": "Max rows (default 20)"
                }
            },
            "required": ["table"]
        }),
        read_only: true,
        parallel_ok: true,
    }
}

/// explain_query tool definition (Phase 3).
fn explain_query_tool() -> ToolDefinition {
    ToolDefinition {
        name: "explain_query".into(),
        description: "Get the execution plan for a SQL query using EXPLAIN. \
                      Shows how the database will execute the query (scan type, indexes, cost). \
                      Only read-only queries (SELECT, WITH, SHOW, DESCRIBE, EXPLAIN) are allowed. \
                      Use this to analyze query performance and suggest index optimizations."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "sql": {
                    "type": "string",
                    "description": "The SQL query to explain (must be read-only)"
                }
            },
            "required": ["sql"]
        }),
        read_only: true,
        parallel_ok: true,
    }
}

/// list_collections tool definition (vector databases).
fn list_collections_tool() -> ToolDefinition {
    ToolDefinition {
        name: "list_collections".into(),
        description: "List all collections in the current vector database. Returns collection names and dimensions."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {},
            "required": []
        }),
        read_only: true,
        parallel_ok: true,
    }
}

/// browse_collection tool definition (vector databases).
fn browse_collection_tool() -> ToolDefinition {
    ToolDefinition {
        name: "browse_collection".into(),
        description: "Browse documents in a collection. Returns up to 20 items with payload/metadata (vectors excluded for compactness). For ChromaDB, use the collection id (UUID from list_collections) instead of the collection name.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "collection": {
                    "type": "string",
                    "description": "Collection name"
                },
                "limit": {
                    "type": "number",
                    "description": "Max items to return (default 20, max 100)"
                }
            },
            "required": ["collection"]
        }),
        read_only: true,
        parallel_ok: true,
    }
}

/// Execute a tool call and return the result.
///
/// Callers that know the run's database scope should use
/// [`execute_tool_scoped`] instead; this wrapper exposes the unscoped behaviour
/// that every existing caller relied on.
pub async fn execute_tool(
    tool_call: &ToolCall,
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
    default_schema: Option<&str>,
    db_type: &DatabaseType,
    sql_permissions: AgentSqlPermissions,
) -> ToolResult {
    execute_tool_scoped(tool_call, state, connection_id, database, &[], default_schema, db_type, sql_permissions).await
}

/// Execute a tool call with the run's database scope.
///
/// `database_scope` lists the databases this run is allowed to touch: the
/// conversation's bound database plus any the user explicitly selected. Only
/// `execute_redis_command` consumes it today — a Redis logical database is a
/// real namespace, so a `db` argument outside the scope would read a different
/// database than the conversation is bound to, which the MCP server already
/// refuses for its own clients. An empty scope keeps the previous behaviour.
pub async fn execute_tool_scoped(
    tool_call: &ToolCall,
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
    database_scope: &[String],
    default_schema: Option<&str>,
    db_type: &DatabaseType,
    sql_permissions: AgentSqlPermissions,
) -> ToolResult {
    // Agent loops for different conversations may share a physical connection.
    // Lock only the database tool future; model generation and non-DB tools stay
    // concurrent. Dropping a cancelled future releases either the waiter or the
    // acquired guard automatically.
    let _connection_guard = if tool_uses_database(&tool_call.name) {
        Some(connection_tool_lock(connection_id).lock_owned().await)
    } else {
        None
    };
    let result = match tool_call.name.as_str() {
        "list_databases" => execute_list_databases(tool_call, state, connection_id).await,
        "list_tables" => execute_list_tables(tool_call, state, connection_id, database, default_schema, db_type).await,
        "get_columns" => execute_get_columns(tool_call, state, connection_id, database, default_schema, db_type).await,
        "execute_query" => {
            execute_execute_query(tool_call, state, connection_id, database, default_schema, db_type, sql_permissions)
                .await
        }
        "get_sample_data" => {
            execute_get_sample_data(tool_call, state, connection_id, database, default_schema, db_type).await
        }

        "explain_query" => {
            let (text_result, explain_data) =
                execute_explain_query(tool_call, state, connection_id, database, default_schema, db_type).await;
            match text_result {
                Ok(content) => {
                    return ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        tool_name: tool_call.name.clone(),
                        content,
                        is_error: false,
                        explain_data,
                    };
                }
                Err(err) => {
                    return ToolResult {
                        tool_call_id: tool_call.id.clone(),
                        tool_name: tool_call.name.clone(),
                        content: format!("Error: {err}"),
                        is_error: true,
                        explain_data: None,
                    };
                }
            }
        }

        "get_current_time" => execute_get_current_time(tool_call),
        _ => Err(format!("Unknown tool: {}", tool_call.name)),
    };

    match result {
        Ok(content) => ToolResult {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            content,
            is_error: false,
            explain_data: None,
        },
        Err(err) => ToolResult {
            tool_call_id: tool_call.id.clone(),
            tool_name: tool_call.name.clone(),
            content: format!("Error: {err}"),
            is_error: true,
            explain_data: None,
        },
    }
}

async fn execute_list_databases(
    _tool_call: &ToolCall,
    state: &Arc<AppState>,
    connection_id: &str,
) -> Result<String, String> {
    let databases = crate::schema::list_databases_core(state, connection_id)
        .await
        .map_err(|e| format!("Failed to list databases: {e}"))?;
    if databases.is_empty() {
        return Ok("No databases were returned for this connection.".to_string());
    }
    Ok(databases.into_iter().map(|database| format!("- {}", database.name)).collect::<Vec<_>>().join("\n"))
}

async fn execute_list_tables(
    tool_call: &ToolCall,
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
    default_schema: Option<&str>,
    _db_type: &DatabaseType,
) -> Result<String, String> {
    let database = effective_database(tool_call, database);
    let schema = effective_schema(tool_call, default_schema).unwrap_or_default();

    // Request one extra to detect whether more tables exist beyond the limit.
    let tables = crate::schema::list_tables_core(
        state,
        connection_id,
        &database,
        &schema,
        None,
        Some(LIST_TABLES_LIMIT + 1),
        None,
        None,
        None,
    )
    .await
    .map_err(|e| format!("Failed to list tables: {e}"))?;

    let total = tables.len();
    let truncated = total > LIST_TABLES_LIMIT;

    let mut lines = Vec::new();
    let display_count = if truncated { LIST_TABLES_LIMIT } else { total };
    for table in tables.iter().take(display_count) {
        let mut line = format!("- {} ({})", table.name, table.table_type);
        if let Some(comment) = &table.comment {
            let trimmed = comment.trim();
            if !trimmed.is_empty() {
                line.push_str(&format!(" -- {}", trimmed));
            }
        }
        lines.push(line);
    }

    if truncated {
        lines.push(format!("... (showing {LIST_TABLES_LIMIT} of {total} tables)"));
    }

    if lines.is_empty() {
        return Ok("No tables found in this database/schema.".to_string());
    }

    Ok(lines.join("\n"))
}

async fn execute_get_columns(
    tool_call: &ToolCall,
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
    default_schema: Option<&str>,
    db_type: &DatabaseType,
) -> Result<String, String> {
    let database = effective_database(tool_call, database);
    let table = tool_call
        .arguments
        .get("table")
        .and_then(|v| v.as_str())
        .ok_or("Missing required parameter: table")?
        .trim()
        .to_string();

    if table.is_empty() {
        return Err("Table name cannot be empty".to_string());
    }
    if table.len() > 256 {
        return Err(format!("Table name too long: {} characters (max 256)", table.len()));
    }
    // Reject names with characters that are unlikely to be valid identifiers
    if table.contains(';') || table.contains('\'') || table.contains('"') || table.contains('\\') {
        return Err(format!("Table name contains invalid characters: '{}'", table));
    }

    let schema = effective_schema(tool_call, default_schema).unwrap_or_default();

    let columns = crate::schema::get_columns_core(state, connection_id, &database, &schema, &table)
        .await
        .map_err(|e| format!("Failed to get columns for {table}: {e}"))?;

    if columns.is_empty() {
        {}
        return Ok(format!("No columns found for table '{table}'."));
    }

    let mut lines = Vec::new();
    lines.push({ format!("Columns of {table}:") });
    for col in &columns {
        let mut flags: Vec<String> = Vec::new();
        if col.is_primary_key {
            flags.push("PK".to_string());
        }
        if col.is_nullable {
            flags.push("nullable".to_string());
        } else {
            flags.push("NOT NULL".to_string());
        }
        if let Some(default) = &col.column_default {
            if !default.is_empty() {
                flags.push(format!("default {default}"));
            }
        }
        if let Some(extra) = &col.extra {
            if !extra.is_empty() {
                flags.push(extra.clone());
            }
        }

        let flags_str = if flags.is_empty() { String::new() } else { format!(" ({})", flags.join(", ")) };

        let comment_str = col
            .comment
            .as_ref()
            .filter(|c| !c.trim().is_empty())
            .map(|c| format!(" -- {}", c.trim()))
            .unwrap_or_default();

        lines.push(format!("  - {}: {}{}{}", col.name, col.data_type, flags_str, comment_str));
    }

    Ok(lines.join("\n"))
}

/// Normalize a SQL string for confirmation comparison.
///
/// Confirmation is intentionally fail-closed: only surrounding whitespace is
/// ignored. SQL case, internal whitespace, comments, literals, and quoted
/// identifiers can all affect execution semantics across supported dialects.
pub fn normalize_sql_for_confirmation(sql: &str) -> String {
    sql.trim().to_string()
}

fn truncate_sql_for_error(sql: &str) -> String {
    let s = sql.trim();
    let char_count = s.chars().count();
    if char_count <= 120 {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(117).collect::<String>())
    }
}

/// Check whether `executed_sql` matches the user-confirmed write SQL. Returns
/// `true` when no confirmation is required (confirmed is `None`) or when the
/// trimmed forms match.
fn sql_matches_confirmed_write(executed_sql: &str, confirmed: &Option<String>) -> bool {
    match confirmed {
        None => true,
        Some(confirmed) => normalize_sql_for_confirmation(executed_sql) == normalize_sql_for_confirmation(confirmed),
    }
}

async fn execute_execute_query(
    tool_call: &ToolCall,
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
    default_schema: Option<&str>,
    db_type: &DatabaseType,
    sql_permissions: AgentSqlPermissions,
) -> Result<String, String> {
    let sql = tool_call.arguments.get("sql").and_then(|v| v.as_str()).ok_or("Missing required parameter: sql")?.trim();

    if sql.is_empty() {
        return Err("SQL query cannot be empty".to_string());
    }

    let limit =
        execute_query_row_limit(tool_call.arguments.get("limit").and_then(|v| v.as_u64()).map(|l| l as usize), db_type);
    let cell_window = QueryCellWindow::from_arguments(&tool_call.arguments);

    {}

    // Classify SQL risk using the concrete database dialect.
    let risk = crate::sql_risk::classify_sql_risk_for_database(sql, *db_type)?;
    let connection_config = state.configs.read().await.get(connection_id).cloned();
    let targets_production = connection_config.as_ref().is_some_and(|config| {
        risk != SqlRisk::ReadOnly && crate::production_safety::targets_production_database(config, database, sql)
    });
    let risk_allowed = sql_risk_allowed(risk, sql_permissions.clone());
    let confirmed_sql_matches =
        risk == SqlRisk::ReadOnly || sql_matches_confirmed_write(sql, &sql_permissions.confirmed_write_sql);

    // Keep permission diagnostics useful without logging SQL, connection details, or result data.
    log::debug!(
        "[agent:execute_query:permission] risk={risk:?} targets_production={targets_production} allow_writes={} allow_dangerous={} has_confirmed_sql={} confirmed_sql_matches={confirmed_sql_matches} will_execute={}",
        sql_permissions.allow_writes,
        sql_permissions.allow_dangerous,
        sql_permissions.confirmed_write_sql.is_some(),
        !targets_production && risk_allowed && confirmed_sql_matches,
    );

    if targets_production {
        return Err("Blocked: AI agents cannot execute writes or DDL on a production database. Return the SQL for the user to review and execute manually in DBX.".to_string());
    }
    if !risk_allowed {
        if risk == SqlRisk::Transaction {
            return Err("Blocked: transaction control statements are not available to the AI agent.".to_string());
        }
        return Err(format!(
            "Blocked: {} statement detected. Ask the user to confirm the proposed database change before executing it.",
            risk
        ));
    }

    // When the user confirmed a specific write SQL, the agent must
    // execute only that SQL — not an arbitrary different statement.
    if !confirmed_sql_matches {
        let confirmed = sql_permissions.confirmed_write_sql.as_deref().unwrap_or("");
        return Err(format!(
            "Blocked: the executed SQL does not match the user-confirmed SQL.\n\
             Confirmed: {}\n\
             Attempted: {}",
            truncate_sql_for_error(confirmed),
            truncate_sql_for_error(sql),
        ));
    }

    // Stateful callers (e.g. MCP sessions) pin the query to their dedicated
    // connection pool so USE/SET and other session state is preserved.
    let client_session_id =
        tool_call.arguments.get("client_session_id").and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty());

    // Execute query using existing infrastructure. Timeout resolves as
    // per-call `timeout_secs` > connection effective timeout (see
    // `agent_query_timeout_secs`); MCP injects the policy value as
    // `timeout_secs` in the arguments.
    let options = QueryExecutionOptions {
        max_rows: Some(limit),
        timeout_secs: Some(agent_query_timeout_secs(
            tool_call.arguments.get("timeout_secs").and_then(Value::as_u64),
            connection_config.as_ref(),
        )),
        client_session_id: client_session_id.map(str::to_string),
        ..Default::default()
    };
    let result = crate::query::execute_sql_statement_with_options(
        state,
        connection_id,
        database,
        sql,
        default_schema,
        None,
        options,
    )
    .await?;

    format_query_result_as_text(&result, limit, cell_window)
}

/// Format a QueryResult as a Markdown table for LLM consumption.
pub fn format_query_result_as_text(
    result: &QueryResult,
    limit: usize,
    cell_window: QueryCellWindow,
) -> Result<String, String> {
    // A result without columns is a command result, not an empty result set.
    // This is how drivers represent DML that does not use RETURNING.
    if result.columns.is_empty() {
        return Ok(append_server_messages(
            format!("Query executed. {} row(s) affected.", result.affected_rows),
            &result.messages,
        ));
    }

    let mut lines = Vec::new();

    // Header row
    lines.push(format!("| {} |", result.columns.join(" | ")));
    // Separator row
    lines.push(format!("|{}|", result.columns.iter().map(|_| "---").collect::<Vec<_>>().join("|")));

    // Data rows
    for row in &result.rows {
        let cells: Vec<String> = row
            .iter()
            .map(|v| match v {
                serde_json::Value::Null => "NULL".to_string(),
                serde_json::Value::String(value) => format_query_string_cell(value, cell_window),
                other => other.to_string(),
            })
            .collect();
        lines.push(format!("| {} |", cells.join(" | ")));
    }

    // Truncation notice. `truncated` is not only a row-cap signal: a driver-side
    // size budget (e.g. the SQLite worker's blob/response limits) raises it
    // before `limit` rows come back, so only claim the row cap when the returned
    // rows actually reach it. The remedy differs — raising the limit helps in one
    // case and does nothing in the other — so the notice never states one.
    if result.truncated || result.rows.len() >= limit {
        let detail = if result.rows.len() >= limit {
            format!("the {limit}-row cap was reached — the result may be truncated")
        } else {
            format!("the result was truncated by a driver size limit, not the {limit}-row cap")
        };
        lines.push(format!("... (showing {} rows; {detail})", result.rows.len()));
    }

    // Stats line
    lines.push(format!("({} rows, {}ms)", result.rows.len(), result.execution_time_ms));

    Ok(append_server_messages(lines.join("\n"), &result.messages))
}

fn format_query_string_cell(value: &str, window: QueryCellWindow) -> String {
    let mut characters = value.chars().skip(window.offset);
    let mut returned = 0usize;
    let content = characters.by_ref().take(window.limit).inspect(|_| returned += 1).collect::<String>();
    let has_more = characters.next().is_some();
    if window.offset == 0 && !has_more {
        return content;
    }

    let end = window.offset.saturating_add(returned);
    let prefix = if window.offset > 0 { "..." } else { "" };
    if has_more {
        format!("{prefix}{content}... [chars {}..{end}; next cell_char_offset={end}]", window.offset)
    } else {
        format!("{prefix}{content} [chars {}..{end}; end of value]", window.offset)
    }
}

/// Append server messages in the same style as the MCP `format_query_result`
/// renderer: a `Server messages:` section with `- SEVERITY: message` lines.
fn append_server_messages(mut output: String, messages: &[QueryMessage]) -> String {
    if messages.is_empty() {
        return output;
    }
    output.push_str("\n\nServer messages:");
    for message in messages {
        output.push_str(&format!("\n- {}", message.format_line()));
    }
    output
}

/// Get sample data from a table via the get_sample_data tool.
async fn execute_get_sample_data(
    tool_call: &ToolCall,
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
    default_schema: Option<&str>,
    db_type: &DatabaseType,
) -> Result<String, String> {
    let database = effective_database(tool_call, database);
    let table =
        tool_call.arguments.get("table").and_then(|v| v.as_str()).ok_or("Missing required parameter: table")?.trim();

    if table.is_empty() {
        return Err("Table name cannot be empty".to_string());
    }
    if table.contains(';') || table.contains('\'') || table.contains('"') || table.contains('\\') {
        return Err(format!("Table name contains invalid characters: '{}'", table));
    }

    let schema = effective_schema(tool_call, default_schema);
    let limit = tool_call
        .arguments
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|l| (l as usize).min(MAX_ALLOWED_ROWS))
        .unwrap_or(SAMPLE_DATA_LIMIT);

    // Reuse the table-data builder so identifier quoting and row limiting follow
    // the active database instead of assuming PostgreSQL syntax.
    let server_version = connection_server_version(state, connection_id).await;
    let sql = build_sample_data_sql(db_type, server_version.as_deref(), schema.as_deref(), table, limit);

    // Delegate to execute_execute_query with a synthetic tool call
    let synthetic_call = ToolCall {
        id: tool_call.id.clone(),
        name: "execute_query".to_string(),
        arguments: serde_json::json!({ "sql": sql, "limit": limit }),
        provider_payload: None,
    };
    execute_execute_query(
        &synthetic_call,
        state,
        connection_id,
        &database,
        schema.as_deref(),
        db_type,
        AgentSqlPermissions::default(),
    )
    .await
}

/// The sample-data statement is produced by the shared dialect code, so it has to match the
/// connected server too: Neo4j below 5 only accepts `id()` where 5+ uses `elementId()`.
async fn connection_server_version(state: &Arc<AppState>, connection_id: &str) -> Option<String> {
    state
        .configs
        .read()
        .await
        .get(connection_id)
        .and_then(|config| config.database_info.as_ref())
        .and_then(|info| info.product_version.clone())
}

fn build_sample_data_sql(
    db_type: &DatabaseType,
    server_version: Option<&str>,
    schema: Option<&str>,
    table: &str,
    limit: usize,
) -> String {
    build_table_data_select_sql(TableDataSelectSqlOptions {
        database_type: Some(*db_type),
        server_version: server_version.map(str::to_owned),
        schema: schema.map(str::to_owned),
        table_name: table.to_string(),
        limit: Some(limit),
        ..Default::default()
    })
}

/// Execute an EXPLAIN query via the explain_query tool.
/// Returns (text_for_llm, optional_explain_data_for_frontend).
async fn execute_explain_query(
    tool_call: &ToolCall,
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
    default_schema: Option<&str>,
    db_type: &DatabaseType,
) -> (Result<String, String>, Option<serde_json::Value>) {
    let sql = match tool_call.arguments.get("sql").and_then(|v| v.as_str()) {
        Some(s) => s.trim(),
        None => return (Err("Missing required parameter: sql".to_string()), None),
    };

    if sql.is_empty() {
        return (Err("SQL query cannot be empty".to_string()), None);
    }

    // Classify SQL risk – only ReadOnly queries can be explained
    let risk = match crate::sql_risk::classify_sql_risk_for_database(sql, *db_type) {
        Ok(r) => r,
        Err(e) => return (Err(e), None),
    };
    match risk {
        SqlRisk::ReadOnly => { /* proceed */ }
        _ => {
            return (
                Err(format!(
                    "Blocked: {} statement detected. Only read-only queries (SELECT, SHOW, DESCRIBE, EXPLAIN) can be analyzed.",
                    risk
                )),
                None,
            );
        }
    }

    {}

    // Build the database-specific EXPLAIN SQL
    let explain_result = build_explain_sql(ExplainSqlOptions {
        database_type: Some(*db_type),
        format: None,
        analyze: None,
        sql: sql.to_string(),
    });

    let explain_sql = match (explain_result.ok, explain_result.sql) {
        (true, Some(sql)) => sql,
        (true, None) => return (Err("EXPLAIN SQL is empty".to_string()), None),
        (false, _) => {
            let reason = explain_result.reason.unwrap_or_else(|| "unknown".to_string());
            return (Err(format!("Cannot explain this query: {}. The database type may not support EXPLAIN, or the query may be unsafe.", reason)), None);
        }
    };

    // Execute the EXPLAIN query. Timeout resolves like the execute path
    // (per-call `timeout_secs` > connection effective timeout) so the global
    // MCP timeout override covers EXPLAIN too.
    let connection_config = state.configs.read().await.get(connection_id).cloned();
    let options = QueryExecutionOptions {
        max_rows: Some(100),
        timeout_secs: Some(agent_query_timeout_secs(
            tool_call.arguments.get("timeout_secs").and_then(Value::as_u64),
            connection_config.as_ref(),
        )),
        ..Default::default()
    };
    let result = match crate::query::execute_sql_statement_with_options(
        state,
        connection_id,
        database,
        &explain_sql,
        default_schema,
        None,
        options,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return (Err(e), None),
    };

    // Serialize the raw QueryResult for the frontend ExplainPlanViewer
    let explain_data = serde_json::to_value(&result).ok();
    let text = match format_query_result_as_text(&result, 100, QueryCellWindow::default()) {
        Ok(t) => t,
        Err(e) => return (Err(e), None),
    };

    (Ok(text), explain_data)
}

/// A selected context schema is authoritative for an Agent run. If none was
/// selected, keep the existing per-tool schema parameter behavior.
fn effective_schema(tool_call: &ToolCall, default_schema: Option<&str>) -> Option<String> {
    default_schema.map(str::trim).filter(|schema| !schema.is_empty()).map(ToOwned::to_owned).or_else(|| {
        tool_call
            .arguments
            .get("schema")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|schema| !schema.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn effective_database(tool_call: &ToolCall, default_database: &str) -> String {
    tool_call
        .arguments
        .get("database")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|database| !database.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_database.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_connection() -> ConnectionConfig {
        serde_json::from_value(json!({
            "id": "agent-timeout-test",
            "name": "timeout-test",
            "db_type": "mysql",
            "host": "127.0.0.1",
            "port": 5432,
            "username": "",
            "password": "",
            "ssl": false,
        }))
        .expect("test connection config")
    }

    #[tokio::test]
    async fn database_tool_locks_are_connection_scoped_and_cancel_safe() {
        let first = connection_tool_lock("agent-tool-lock-a").lock_owned().await;
        let same_connection = connection_tool_lock("agent-tool-lock-a");
        let other_connection = connection_tool_lock("agent-tool-lock-b");

        assert!(same_connection.try_lock().is_err(), "same connection must serialize tool calls");
        assert!(other_connection.try_lock().is_ok(), "different connections must remain concurrent");

        let waiter = tokio::spawn(async move { same_connection.lock_owned().await });
        tokio::task::yield_now().await;
        waiter.abort();
        let _ = waiter.await;
        drop(first);

        assert!(connection_tool_lock("agent-tool-lock-a").try_lock().is_ok());
    }
    #[cfg(unix)]
    use crate::connection::PoolKind;

    #[cfg(unix)]
    use crate::models::connection::{default_redis_key_separator, ConnectionConfig};

    #[test]
    fn confirmed_sql_permissions_update_execute_query_contract() {
        let tools = all_tools(
            DatabaseType::Mysql,
            AgentSqlPermissions { allow_writes: true, allow_dangerous: true, confirmed_write_sql: None },
        );
        let execute_query = tools.iter().find(|tool| tool.name == "execute_query").unwrap();

        assert!(execute_query.description.contains("explicitly confirmed"));
        assert!(execute_query.description.contains("DDL"));
    }

    #[test]
    fn sql_permissions_keep_writes_blocked_until_confirmation() {
        assert!(!sql_risk_allowed(SqlRisk::Write, AgentSqlPermissions::default()));
        assert!(!sql_risk_allowed(SqlRisk::Ddl, AgentSqlPermissions::default()));
        assert!(sql_risk_allowed(
            SqlRisk::Ddl,
            AgentSqlPermissions { allow_writes: true, allow_dangerous: true, confirmed_write_sql: None }
        ));
        assert!(!sql_risk_allowed(
            SqlRisk::Transaction,
            AgentSqlPermissions { allow_writes: true, allow_dangerous: true, confirmed_write_sql: None }
        ));
    }

    fn query_result(columns: Vec<&str>, rows: Vec<Vec<serde_json::Value>>, affected_rows: u64) -> QueryResult {
        QueryResult {
            columns: columns.into_iter().map(str::to_string).collect(),
            column_types: Vec::new(),
            column_sortables: Vec::new(),
            spatial_columns: Vec::new(),
            spatial_values: Vec::new(),
            rows,
            affected_rows,
            execution_time_ms: 1,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        }
    }

    #[test]
    fn query_result_formatter_reports_dml_affected_rows() {
        let result = query_result(vec![], vec![], 2);

        assert_eq!(
            format_query_result_as_text(&result, 50, QueryCellWindow::default()).unwrap(),
            "Query executed. 2 row(s) affected."
        );
    }

    #[test]
    fn query_result_formatter_distinguishes_zero_row_dml_from_an_empty_result_set() {
        let dml = query_result(vec![], vec![], 0);
        let returning = query_result(vec!["id", "name"], vec![], 0);

        assert_eq!(
            format_query_result_as_text(&dml, 50, QueryCellWindow::default()).unwrap(),
            "Query executed. 0 row(s) affected."
        );
        assert_eq!(
            format_query_result_as_text(&returning, 50, QueryCellWindow::default()).unwrap(),
            "| id | name |\n|---|---|\n(0 rows, 1ms)"
        );
    }

    #[test]
    fn query_result_formatter_renders_returning_rows() {
        let result =
            query_result(vec!["id", "name"], vec![vec![serde_json::json!(5), serde_json::json!("returning")]], 0);

        assert_eq!(
            format_query_result_as_text(&result, 50, QueryCellWindow::default()).unwrap(),
            "| id | name |\n|---|---|\n| 5 | returning |\n(1 rows, 1ms)"
        );
    }

    #[test]
    fn query_result_formatter_marks_the_default_character_window() {
        let value = format!("{}DBX_ISSUE_5620_SENTINEL", "A".repeat(200));
        let result = query_result(vec!["message"], vec![vec![serde_json::json!(value)]], 0);

        let output = format_query_result_as_text(&result, 50, QueryCellWindow::from_options(None, None)).unwrap();

        assert!(output.contains(&format!("{}... [chars 0..200; next cell_char_offset=200]", "A".repeat(200))));
        assert!(!output.contains("DBX_ISSUE_5620_SENTINEL"));
    }

    #[test]
    fn query_result_formatter_supports_expanded_and_sliding_character_windows() {
        let value = format!("{}DBX_ISSUE_5620_SENTINEL{}", "A".repeat(200), "Z".repeat(20));
        let result = query_result(vec!["message"], vec![vec![serde_json::json!(value)]], 0);

        let expanded =
            format_query_result_as_text(&result, 50, QueryCellWindow::from_options(None, Some(400))).unwrap();
        assert!(expanded.contains("DBX_ISSUE_5620_SENTINEL"));
        assert!(!expanded.contains("next cell_char_offset"));

        let sliding =
            format_query_result_as_text(&result, 50, QueryCellWindow::from_options(Some(200), Some(23))).unwrap();
        assert!(sliding.contains("...DBX_ISSUE_5620_SENTINEL... [chars 200..223; next cell_char_offset=223]"));
    }

    #[test]
    fn query_result_formatter_counts_unicode_characters_in_windows() {
        let result = query_result(vec!["message"], vec![vec![serde_json::json!("甲乙丙丁戊己庚辛")]], 0);

        let output = format_query_result_as_text(&result, 50, QueryCellWindow::from_options(Some(2), Some(3))).unwrap();

        assert!(output.contains("...丙丁戊... [chars 2..5; next cell_char_offset=5]"));
    }

    #[test]
    fn query_cell_window_clamps_explicit_bounds() {
        assert_eq!(QueryCellWindow::from_options(None, None), QueryCellWindow::default());
        assert_eq!(
            QueryCellWindow::from_options(Some(u64::MAX), Some(0)),
            QueryCellWindow { offset: 1_000_000, limit: 1 }
        );
        assert_eq!(
            QueryCellWindow::from_options(Some(1_000_001), Some(u64::MAX)),
            QueryCellWindow { offset: 1_000_000, limit: 4_000 }
        );
    }

    #[test]
    fn query_result_formatter_only_claims_the_row_cap_when_it_was_reached() {
        let row = vec![vec![serde_json::json!(1)]];

        // Returned rows reach the cap: the notice names the effective cap so the
        // model can connect it to the tool's limit argument.
        let at_cap = query_result(vec!["id"], row.clone(), 0);
        let output = format_query_result_as_text(&at_cap, 1, QueryCellWindow::default()).unwrap();
        assert!(
            output.contains("... (showing 1 rows; the 1-row cap was reached — the result may be truncated)"),
            "unexpected notice: {output}"
        );

        // `truncated` without reaching the cap comes from a driver size budget
        // (SQLite worker blobs / response size). It must not claim the row cap,
        // because raising the limit would not return the missing rows.
        let mut size_limited = query_result(vec!["id"], row, 0);
        size_limited.truncated = true;
        let output = format_query_result_as_text(&size_limited, 100, QueryCellWindow::default()).unwrap();
        assert!(
            output.contains("the result was truncated by a driver size limit, not the 100-row cap"),
            "unexpected notice: {output}"
        );
        assert!(!output.contains("cap was reached"), "a size truncation must not claim the row cap: {output}");
    }

    #[test]
    fn query_result_formatter_appends_server_messages() {
        let mut dml = query_result(vec![], vec![], 2);
        dml.messages = vec![
            QueryMessage {
                severity: "notice".to_string(),
                message: "hello world".to_string(),
                code: Some("00000".to_string()),
                detail: None,
                hint: Some("use a table".to_string()),
            },
            QueryMessage {
                severity: "WARNING".to_string(),
                message: "careful".to_string(),
                code: None,
                detail: None,
                hint: None,
            },
        ];
        assert_eq!(
            format_query_result_as_text(&dml, 50, QueryCellWindow::default()).unwrap(),
            "Query executed. 2 row(s) affected.\n\nServer messages:\n- NOTICE: hello world (code: 00000, hint: use a table)\n- WARNING: careful"
        );

        let mut result = query_result(vec!["id"], vec![vec![serde_json::json!(1)]], 0);
        result.messages = vec![QueryMessage {
            severity: "INFO".to_string(),
            message: "print output".to_string(),
            code: None,
            detail: None,
            hint: None,
        }];
        assert_eq!(
            format_query_result_as_text(&result, 50, QueryCellWindow::default()).unwrap(),
            "| id |\n|---|\n| 1 |\n(1 rows, 1ms)\n\nServer messages:\n- INFO: print output"
        );
    }

    #[test]
    fn selected_schema_overrides_the_tool_schema_argument() {
        let call = ToolCall {
            id: "call-1".to_string(),
            name: "list_tables".to_string(),
            arguments: serde_json::json!({ "schema": "OTHER" }),
            provider_payload: None,
        };

        assert_eq!(effective_schema(&call, Some("REPORTING")).as_deref(), Some("REPORTING"));
        assert_eq!(effective_schema(&call, None).as_deref(), Some("OTHER"));
    }

    // ── SQL confirmation binding tests ──────────────────────────────────────

    #[test]
    fn normalize_sql_only_trims_outer_whitespace() {
        assert_eq!(normalize_sql_for_confirmation("  CREATE TABLE users (id INT);\n"), "CREATE TABLE users (id INT);");
    }

    #[test]
    fn confirmed_sql_binding_rejects_quoted_identifier_case_change() {
        let confirmed = Some("DROP TABLE \"Users\"".to_string());
        assert!(!sql_matches_confirmed_write("DROP TABLE \"users\"", &confirmed));
    }

    #[test]
    fn confirmed_sql_binding_rejects_keyword_case_or_reformatting() {
        let confirmed = Some("DELETE FROM AuditLog WHERE id = 1".to_string());
        assert!(!sql_matches_confirmed_write("delete from AuditLog where id = 1", &confirmed));
        assert!(!sql_matches_confirmed_write("DELETE FROM AuditLog\nWHERE id = 1", &confirmed));
    }

    #[test]
    fn confirmed_sql_binding_rejects_line_comment_newline_change() {
        let confirmed = Some("DELETE FROM users -- only one record\nWHERE id = 1".to_string());
        assert!(!sql_matches_confirmed_write("DELETE FROM users -- only one record WHERE id = 1", &confirmed));
    }

    #[test]
    fn sql_matches_when_no_confirmation_required() {
        assert!(sql_matches_confirmed_write("DELETE FROM users WHERE id = 1", &None));
    }

    #[test]
    fn sql_matches_when_only_outer_whitespace_differs() {
        // The confirmed statement itself must be unchanged; only surrounding
        // whitespace is ignored before comparison.
        let confirmed = Some("CREATE TABLE users (id INT)".to_string());
        assert!(sql_matches_confirmed_write("  CREATE TABLE users (id INT)  ", &confirmed,));
    }

    #[test]
    fn sql_mismatch_rejected_when_executed_differs_from_confirmed() {
        let confirmed = Some("CREATE TABLE users (id INT)".to_string());
        assert!(!sql_matches_confirmed_write("DROP TABLE users", &confirmed,));
    }

    #[test]
    fn confirmed_sql_binding_rejects_same_table_different_statement() {
        let confirmed = Some("INSERT INTO users (id, name) VALUES (1, 'test')".to_string());
        assert!(!sql_matches_confirmed_write("DELETE FROM users WHERE id = 1", &confirmed,));
    }

    #[test]
    fn confirmed_sql_binding_rejects_different_case_in_data_values() {
        // Confirmed VALUES ('Alice') must NOT match executed VALUES ('alice').
        // String-literal data values are preserved verbatim.
        let confirmed = Some("INSERT INTO users (name) VALUES ('Alice')".to_string());
        assert!(!sql_matches_confirmed_write("INSERT INTO users (name) VALUES ('alice')", &confirmed,));
    }

    #[test]
    fn confirmed_sql_binding_rejects_different_whitespace_in_data_values() {
        // Confirmed VALUES ('a b') must NOT match executed VALUES ('a  b').
        let confirmed = Some("INSERT INTO t (c) VALUES ('a b')".to_string());
        assert!(!sql_matches_confirmed_write("INSERT INTO t (c) VALUES ('a  b')", &confirmed,));
    }

    #[test]
    fn confirmed_sql_default_is_none() {
        let perms = AgentSqlPermissions::default();
        assert_eq!(perms.confirmed_write_sql, None);
    }

    #[test]
    fn confirmed_write_sql_diagnostics_redact_sensitive_literals() {
        let confirmed_sql = "CREATE USER app_user WITH PASSWORD 'secret-123'";
        let diagnostic = crate::sql_diagnostics::redact_sql_for_diagnostics(confirmed_sql);

        assert!(!diagnostic.contains("secret-123"));
        assert!(diagnostic.contains("'[REDACTED]'"));
    }

    #[test]
    fn confirmed_write_permissions_bind_only_a_nonproduction_nonempty_confirmation() {
        let confirmed_sql = Some("DELETE FROM sessions WHERE id = 7".to_string());
        let permissions = confirmed_write_sql_permissions(false, true, confirmed_sql.clone());

        assert!(permissions.allow_writes);
        assert!(permissions.allow_dangerous);
        assert_eq!(permissions.confirmed_write_sql, confirmed_sql);
        assert!(!sql_matches_confirmed_write("DROP TABLE sessions", &permissions.confirmed_write_sql));
    }

    #[test]
    fn confirmed_write_permissions_fail_closed_for_production_or_empty_confirmation() {
        for (production_database, confirmed_write_sql) in
            [(true, Some("DELETE FROM sessions".to_string())), (false, None), (false, Some("  \n".to_string()))]
        {
            let permissions = confirmed_write_sql_permissions(production_database, true, confirmed_write_sql);
            assert!(!permissions.allow_writes);
            assert!(!permissions.allow_dangerous);
            assert_eq!(permissions.confirmed_write_sql, None);
        }
    }

    #[test]
    fn confirmed_write_target_binding_rejects_replay_to_another_scope() {
        let confirmed_sql = Some("DELETE FROM sessions WHERE id = 7".to_string());
        let matching = verify_confirmed_target(
            Some(true),
            confirmed_sql.clone(),
            Some("connection-1".to_string()),
            Some("app".to_string()),
            Some("public".to_string()),
            "connection-1",
            "app",
            Some("public"),
        );
        assert_eq!(matching, (Some(true), confirmed_sql.clone()));

        for (connection_id, database, schema) in [
            ("connection-2", "app", Some("public")),
            ("connection-1", "audit", Some("public")),
            ("connection-1", "app", Some("private")),
        ] {
            let rejected = verify_confirmed_target(
                Some(true),
                confirmed_sql.clone(),
                Some("connection-1".to_string()),
                Some("app".to_string()),
                Some("public".to_string()),
                connection_id,
                database,
                schema,
            );
            assert_eq!(rejected, (Some(false), None));
        }
    }

    #[test]
    fn confirmed_sql_preserved_through_permission_construction() {
        let perms = AgentSqlPermissions {
            allow_writes: true,
            allow_dangerous: true,
            confirmed_write_sql: Some("CREATE TABLE t (c INT)".to_string()),
        };
        assert_eq!(perms.confirmed_write_sql.as_deref(), Some("CREATE TABLE t (c INT)"));
    }

    #[test]
    fn write_allowed_when_confirmed_sql_is_none_and_writes_enabled() {
        // confirmed_write_sql=None + allow_writes=true: write SQL is allowed
        // (the check passes because sql_matches_confirmed_write returns true
        // when no confirmation is required). This documents the current
        // contract — the frontend is responsible for only sending
        // allow_write_sql=true when a specific SQL was confirmed.
        assert!(sql_matches_confirmed_write("INSERT INTO t VALUES (1)", &None));
    }

    #[test]
    fn get_current_time_tool_is_read_only_and_parallel_ok() {
        let tool = get_current_time_tool();
        assert!(tool.read_only, "get_current_time must be read_only");
        assert!(tool.parallel_ok, "get_current_time must be parallel_ok");
    }

    #[test]
    fn execute_get_current_time_returns_valid_timestamps() {
        let before_secs = chrono::Utc::now().timestamp();
        let tool_call = ToolCall {
            id: "call-gct".to_string(),
            name: "get_current_time".to_string(),
            arguments: serde_json::json!({ "utc_offset_minutes": 480, "timezone": "Asia/Shanghai" }),
            provider_payload: None,
        };
        let result = execute_get_current_time(&tool_call).expect("get_current_time should succeed");
        let after_secs = chrono::Utc::now().timestamp();

        let parsed: serde_json::Value =
            serde_json::from_str(&result).expect("get_current_time result should be valid JSON");

        let utc_str = parsed["utc"].as_str().expect("utc field should be a string");
        let local_str = parsed["local"].as_str().expect("local field should be a string");

        // Parse as RFC3339 timestamps.
        let _utc_dt = chrono::DateTime::parse_from_rfc3339(utc_str).expect("utc should be valid RFC3339");
        let local_dt = chrono::DateTime::parse_from_rfc3339(local_str).expect("local should be valid RFC3339");

        // Verify UTC is within tolerance.
        let utc_dt_utc = chrono::DateTime::parse_from_rfc3339(utc_str)
            .expect("utc should parse as rfc3339")
            .with_timezone(&chrono::Utc);
        let utc_ts = utc_dt_utc.timestamp();
        assert!(
            utc_ts >= before_secs && utc_ts <= after_secs + 1,
            "UTC timestamp {utc_ts} should be within [{before_secs}, {after_secs}+1]"
        );

        // Verify local is within tolerance (converted to UTC).
        let local_ts = local_dt.with_timezone(&chrono::Utc).timestamp();
        assert!(
            local_ts >= before_secs && local_ts <= after_secs + 1,
            "Local timestamp {local_ts} (UTC) should be within [{before_secs}, {after_secs}+1]"
        );

        // utc_offset_minutes should match the offset in local.
        let offset_minutes =
            parsed["utc_offset_minutes"].as_i64().expect("utc_offset_minutes should be an integer") as i32;
        let local_offset_secs = local_dt.offset().local_minus_utc();
        // The offset from the RFC3339 timestamp should match utc_offset_minutes * 60.
        assert_eq!(
            local_offset_secs / 60,
            offset_minutes,
            "utc_offset_minutes {offset_minutes} does not match local offset {}",
            local_offset_secs / 60
        );
        assert_eq!(offset_minutes, 480);
        assert_eq!(parsed["timezone"], "Asia/Shanghai");

        // readable should be a non-empty string.
        let readable = parsed["readable"].as_str().expect("readable field should be a string");
        assert!(!readable.is_empty(), "readable should not be empty");
    }

    #[test]
    fn execute_get_current_time_defaults_to_utc_without_client_context() {
        let tool_call = ToolCall {
            id: "call-gct".to_string(),
            name: "get_current_time".to_string(),
            arguments: serde_json::json!({}),
            provider_payload: None,
        };
        let result = execute_get_current_time(&tool_call).expect("get_current_time should succeed");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["utc_offset_minutes"], 0);
        assert_eq!(parsed["timezone"], "UTC");
        assert!(parsed["local"].as_str().unwrap().ends_with("+00:00"));
    }

    #[test]
    fn execute_get_current_time_labels_offset_when_timezone_name_is_missing() {
        let tool_call = ToolCall {
            id: "call-gct".to_string(),
            name: "get_current_time".to_string(),
            arguments: serde_json::json!({ "utc_offset_minutes": -300 }),
            provider_payload: None,
        };
        let result = execute_get_current_time(&tool_call).expect("get_current_time should succeed");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();

        assert_eq!(parsed["timezone"], "UTC-05:00");
        assert!(parsed["local"].as_str().unwrap().ends_with("-05:00"));
    }
}
