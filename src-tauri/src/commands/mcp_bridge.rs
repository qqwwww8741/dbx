use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::connection::AppState;

use dbx_core::mcp_policy::McpConnectionGroupPath;
use dbx_core::storage::{McpDatabaseScope, McpGlobalPolicy};

const BIND_ADDR: &str = "127.0.0.1:0";
const MCP_BRIDGE_PORT_FILE: &str = "mcp-bridge-port";
const MCP_EXECUTE_AND_SHOW_SQL_ONLY: &str =
    "UNSUPPORTED_OPERATION: MCP execute-and-show only supports SQL connections.";

#[derive(Deserialize)]
struct OpenTableRequest {
    connection_name: String,
    connection_id: Option<String>,
    database: Option<String>,
    schema: Option<String>,
    table: String,
}

#[derive(Deserialize)]
struct ExecuteQueryRequest {
    connection_name: String,
    connection_id: Option<String>,
    database: Option<String>,
    sql: String,
    schema: Option<String>,
}

#[derive(Deserialize)]
struct ListTablesRequest {
    connection_name: String,
    connection_id: Option<String>,
    database: Option<String>,
    schema: Option<String>,
}

#[derive(Deserialize)]
struct DescribeTableRequest {
    connection_name: String,
    connection_id: Option<String>,
    database: Option<String>,
    schema: Option<String>,
    table: String,
}

#[derive(Clone, Serialize)]
pub struct McpOpenTableEvent {
    pub connection_id: String,
    pub database: String,
    pub schema: Option<String>,
    pub table: String,
}

#[derive(Clone, Serialize)]
pub struct McpExecuteQueryEvent {
    pub connection_id: String,
    pub database: String,
    pub sql: String,
    pub results: Vec<dbx_core::db::QueryResult>,
}

pub fn start(app_handle: AppHandle, state: Arc<AppState>, data_dir: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::bind(BIND_ADDR).await {
            Ok(l) => l,
            Err(e) => {
                log::warn!("MCP bridge failed to bind {BIND_ADDR}: {e}");
                return;
            }
        };
        log::info!("MCP bridge listening on {BIND_ADDR}");
        let actual_port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        log::info!("MCP bridge assigned port {actual_port}");
        // Publish into DBX's resolved data dir so DBX_DATA_DIR and portable mode share the same discovery file.
        if let Err(err) = write_port_file(&data_dir, actual_port) {
            log::warn!("MCP bridge failed to write port file in {}: {err}", data_dir.display());
        }
        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let app = app_handle.clone();
            let st = state.clone();
            tokio::spawn(async move {
                let request = match read_bridge_request(&mut stream).await {
                    Some(request) => request,
                    None => return,
                };
                let mut parts = request.splitn(2, "\r\n\r\n");
                let head = parts.next().unwrap_or("");
                let body = parts.next().unwrap_or("");
                let first_line = head.lines().next().unwrap_or("");
                if !bridge_request_is_local_client(first_line, head) {
                    let _ = stream
                        .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                }

                if first_line.starts_with("POST /open-table") {
                    handle_open_table(&app, &st, body, &mut stream).await;
                } else if first_line.starts_with("POST /call-plugin-tool") {
                    handle_call_plugin_tool(&app, &st, body, &mut stream).await;
                } else if first_line.starts_with("POST /list-plugin-connections") {
                    handle_list_plugin_connections(&st, body, &mut stream).await;
                } else if first_line.starts_with("POST /data/list-tables") {
                    handle_list_tables_data(&st, body, &mut stream).await;
                } else if first_line.starts_with("POST /data/describe-table") {
                    handle_describe_table_data(&st, body, &mut stream).await;
                } else if first_line.starts_with("POST /data/execute-query") {
                    handle_execute_query_data(&st, body, &mut stream).await;
                } else if first_line.starts_with("POST /execute-query") {
                    handle_execute_query(&app, &st, body, &mut stream).await;
                } else if first_line.starts_with("POST /reload-connections") {
                    let _ = app.emit("mcp-reload-connections", ());
                    respond(&mut stream, "200 OK", "ok").await;
                } else {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
                }
            });
        }
    });
}

fn write_port_file(data_dir: &Path, actual_port: u16) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(MCP_BRIDGE_PORT_FILE);
    // 0600: the published port is the discovery half of a local control
    // plane; other local users must not be able to read (and talk to) it.
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    std::io::Write::write_all(&mut file, actual_port.to_string().as_bytes())?;
    Ok(path)
}

/// Upper bound for one bridge request (headers + body). Generous for large
/// SQL batches; the previous fixed 64 KiB single read silently truncated
/// bigger requests — and a truncation that landed on a JSON boundary could
/// execute cut-down SQL.
const MAX_BRIDGE_REQUEST_BYTES: usize = 10 * 1024 * 1024;
const BRIDGE_HEADER_TERMINATOR: &[u8] = b"\r\n\r\n";

/// Reads one HTTP/1.1 request off the bridge connection: the header block
/// first, then exactly `Content-Length` body bytes. TCP delivers large
/// bodies across multiple segments, so a single read truncates them; a
/// closed or short read still returns whatever arrived so routing can
/// answer with a precise 400 instead of dropping the connection.
async fn read_bridge_request(stream: &mut tokio::net::TcpStream) -> Option<String> {
    let mut buf: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 16384];
    let mut header_end: Option<usize> = None;
    let mut content_length: usize = 0;
    let mut scanned = 0usize;
    loop {
        if let Some(end) = header_end {
            if buf.len() >= end + content_length {
                return Some(String::from_utf8_lossy(&buf).into_owned());
            }
        } else if let Some(pos) =
            buf[scanned..].windows(BRIDGE_HEADER_TERMINATOR.len()).position(|window| window == BRIDGE_HEADER_TERMINATOR)
        {
            let end = scanned + pos + BRIDGE_HEADER_TERMINATOR.len();
            header_end = Some(end);
            content_length = parse_content_length(&String::from_utf8_lossy(&buf[..end]));
            // The same read may already contain the complete body (or no body).
            continue;
        } else {
            scanned = buf.len().saturating_sub(BRIDGE_HEADER_TERMINATOR.len() - 1);
        }
        let n = match stream.read(&mut chunk).await {
            Ok(n) if n > 0 => n,
            _ => {
                return if buf.is_empty() { None } else { Some(String::from_utf8_lossy(&buf).into_owned()) };
            }
        };
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_BRIDGE_REQUEST_BYTES {
            log::warn!("MCP bridge dropped an oversized request ({} bytes)", buf.len());
            return None;
        }
    }
}

fn parse_content_length(header_block: &str) -> usize {
    header_block
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim().eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0)
}

/// The bridge is a loopback-only control plane for local, non-browser
/// clients. Browsers attach an `Origin` header to every cross-origin
/// request, and their non-preflighted cross-site POSTs cannot carry
/// `Content-Type: application/json` — either one marks a fire-and-forget
/// CSRF attempt. A `Host` outside the loopback forms marks DNS rebinding.
/// All are refused before routing; ordinary local clients (the plugin
/// sidecars, dbx-mcp, dbx-cli) send Host + application/json and no Origin.
fn bridge_request_is_local_client(first_line: &str, head: &str) -> bool {
    let header_value = |name: &str| -> Option<String> {
        head.lines().skip(1).find_map(|line| {
            let (line_name, value) = line.split_once(':')?;
            line_name.trim().eq_ignore_ascii_case(name).then(|| value.trim().to_string())
        })
    };
    if first_line.starts_with("POST") {
        let is_json = header_value("content-type")
            .map(|value| value.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json"))
            .unwrap_or(false);
        if !is_json {
            return false;
        }
    }
    if header_value("origin").is_some() {
        return false;
    }
    match header_value("host") {
        Some(host) => {
            host.starts_with("127.0.0.1:")
                || host.starts_with("localhost:")
                || host == "127.0.0.1"
                || host == "localhost"
        }
        None => false,
    }
}

fn find_config_by_name<'a>(
    configs: &'a [crate::models::connection::ConnectionConfig],
    name: &str,
) -> Option<&'a crate::models::connection::ConnectionConfig> {
    configs.iter().find(|c| c.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use dbx_core::models::connection::{ConnectionConfig, DatabaseType};
    use dbx_core::storage::{McpConnectionPolicy, McpDatabasePolicy, McpDatabaseScope, McpGlobalPolicy};
    use std::sync::Arc;

    fn mysql_config(read_only: bool) -> ConnectionConfig {
        serde_json::from_value(serde_json::json!({
            "id": "readonly-connection",
            "name": "Read-only connection",
            "db_type": "mysql",
            "host": "localhost",
            "port": 3306,
            "username": "tester",
            "password": "",
            "database": "test",
            "read_only": read_only
        }))
        .unwrap()
    }

    #[test]
    fn writes_bridge_port_file_to_resolved_data_dir() {
        let root = std::env::temp_dir().join(format!(
            "dbx-mcp-bridge-port-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let default_data_dir = root.join("default-app-data");
        let resolved_data_dir = root.join("resolved-data");
        std::fs::create_dir_all(&default_data_dir).unwrap();

        let port_file = write_port_file(&resolved_data_dir, 49152).unwrap();

        assert_eq!(port_file, resolved_data_dir.join("mcp-bridge-port"));
        assert_eq!(std::fs::read_to_string(&port_file).unwrap(), "49152");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&port_file).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the port file must not be world-readable");
        }
        assert!(!default_data_dir.join("mcp-bridge-port").exists());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn call_plugin_target_accepts_the_bound_plugin_with_or_without_an_echo() {
        assert_eq!(resolve_call_plugin_target(Some("io.dbx.ssh"), None).unwrap(), "io.dbx.ssh");
        assert_eq!(resolve_call_plugin_target(Some("io.dbx.ssh"), Some("io.dbx.ssh")).unwrap(), "io.dbx.ssh");
    }

    #[test]
    fn call_plugin_target_rejects_override_and_unbound_connections() {
        assert!(resolve_call_plugin_target(Some("io.dbx.ssh"), Some("io.dbx.files")).is_err());
        assert!(resolve_call_plugin_target(None, Some("io.dbx.ssh")).is_err());
        assert!(resolve_call_plugin_target(Some(""), Some("io.dbx.ssh")).is_err());
        assert_eq!(resolve_call_plugin_target(None, None).unwrap_err(), "connection is not bound to a plugin");
    }

    #[test]
    fn content_length_parsing_tolerates_case_and_whitespace() {
        assert_eq!(parse_content_length("POST /x HTTP/1.1\r\nContent-Length: 12\r\n\r\n"), 12);
        assert_eq!(parse_content_length("POST /x HTTP/1.1\r\ncontent-length:  7\r\n\r\n"), 7);
        assert_eq!(parse_content_length("POST /x HTTP/1.1\r\nContent-Length: bogus\r\n\r\n"), 0);
        assert_eq!(parse_content_length("POST /x HTTP/1.1\r\n\r\n"), 0);
    }

    #[tokio::test]
    async fn bridge_reads_bodies_arriving_in_multiple_segments() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let reader = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_bridge_request(&mut stream).await.unwrap()
        });
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        use tokio::io::AsyncWriteExt;
        let body = "x".repeat(100_000);
        let request = format!(
            "POST /execute-query HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let bytes = request.into_bytes();
        // Split the request well past the header so the body cannot arrive
        // in the same TCP segment the header rides in.
        client.write_all(&bytes[..4096]).await.unwrap();
        client.flush().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        client.write_all(&bytes[4096..]).await.unwrap();
        let parsed = reader.await.unwrap();
        assert!(parsed.ends_with(&body), "body must arrive complete, not truncated");
    }

    async fn wait_for_queued_request(stream: &tokio::net::TcpStream, expected_len: usize) {
        let mut queued = vec![0u8; expected_len];
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if stream.peek(&mut queued).await.unwrap() >= expected_len {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("request bytes must be queued before the reader starts");
    }

    async fn read_complete_request_while_writer_stays_open(request: &[u8]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        use tokio::io::AsyncWriteExt;
        client.write_all(request).await.unwrap();
        wait_for_queued_request(&server, request.len()).await;

        tokio::time::timeout(std::time::Duration::from_secs(1), read_bridge_request(&mut server))
            .await
            .expect("complete request must not wait for another read")
            .unwrap()
    }

    #[tokio::test]
    async fn bridge_reads_complete_small_request_while_writer_stays_open() {
        let request = b"POST /x HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\n\r\n{}";

        assert_eq!(read_complete_request_while_writer_stays_open(request).await.as_bytes(), request);
    }

    #[tokio::test]
    async fn bridge_reads_zero_length_body_while_writer_stays_open() {
        let request = b"POST /x HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\n\r\n";

        assert_eq!(read_complete_request_while_writer_stays_open(request).await.as_bytes(), request);
    }

    #[tokio::test]
    async fn bridge_reads_complete_body_in_fragment_that_finishes_header_delimiter() {
        let first = b"POST /x HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\n\r";
        let last = b"\n{}";
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut client = tokio::net::TcpStream::connect(addr).await.unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        use tokio::io::AsyncWriteExt;
        client.write_all(first).await.unwrap();
        wait_for_queued_request(&server, first.len()).await;
        let mut reader = Box::pin(read_bridge_request(&mut server));
        // Consume the queued prefix before providing the delimiter's final bytes.
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(reader.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        client.write_all(last).await.unwrap();

        let parsed = tokio::time::timeout(std::time::Duration::from_secs(1), reader)
            .await
            .expect("complete final fragment must not wait for another read")
            .unwrap();
        let expected = [first.as_slice(), last.as_slice()].concat();
        assert_eq!(parsed.as_bytes(), expected);
    }

    #[test]
    fn bridge_rejects_browser_shaped_requests_and_keeps_local_clients() {
        assert!(bridge_request_is_local_client(
            "POST /call-plugin-tool HTTP/1.1",
            "POST /call-plugin-tool HTTP/1.1\r\nHost: 127.0.0.1:7777\r\nContent-Type: application/json"
        ));
        assert!(bridge_request_is_local_client("GET / HTTP/1.1", "GET / HTTP/1.1\r\nHost: localhost"));
        assert!(bridge_request_is_local_client(
            "POST /execute-query HTTP/1.1",
            "POST /execute-query HTTP/1.1\r\nHost: 127.0.0.1:7777\r\nContent-Type: application/json; charset=utf-8"
        ));
        // Browsers always attach Origin to cross-origin requests.
        assert!(!bridge_request_is_local_client(
            "POST /call-plugin-tool HTTP/1.1",
            "POST /call-plugin-tool HTTP/1.1\r\nHost: 127.0.0.1:7777\r\nContent-Type: application/json\r\nOrigin: http://evil.example"
        ));
        // Non-preflightable content types are the classic fire-and-forget form POST.
        assert!(!bridge_request_is_local_client(
            "POST /call-plugin-tool HTTP/1.1",
            "POST /call-plugin-tool HTTP/1.1\r\nHost: 127.0.0.1:7777\r\nContent-Type: text/plain"
        ));
        assert!(!bridge_request_is_local_client(
            "POST /call-plugin-tool HTTP/1.1",
            "POST /call-plugin-tool HTTP/1.1\r\nHost: 127.0.0.1:7777"
        ));
        // DNS rebinding presents a foreign Host.
        assert!(!bridge_request_is_local_client(
            "POST /call-plugin-tool HTTP/1.1",
            "POST /call-plugin-tool HTTP/1.1\r\nHost: evil.example\r\nContent-Type: application/json"
        ));
    }

    #[test]
    fn only_ssh_exec_tools_may_open_the_workbench_terminal() {
        // The workbench open (and its tab churn) exists solely so a
        // terminal-routed exec lands in a real PTY; forwarded sftp/metrics
        // calls are hidden-channel by design and must never trigger it,
        // whatever runInTerminal says.
        assert!(is_terminal_routed_exec("ssh_exec"));
        assert!(is_terminal_routed_exec("ssh_exec_sudo"));
        assert!(!is_terminal_routed_exec("sftp_list_dir"));
        assert!(!is_terminal_routed_exec("ssh_metrics"));
        assert!(!is_terminal_routed_exec("mcp_exec"));
    }

    #[test]
    fn mcp_sql_checks_supplied_connection_read_only_flag() {
        let mut config = mysql_config(true);

        assert!(ensure_mcp_connection_sql_write_allowed(&config, false).is_ok());
        let error = ensure_mcp_connection_sql_write_allowed(&config, true).unwrap_err();
        assert!(error.starts_with("CONNECTION_READ_ONLY:"));

        config.read_only = false;
        assert!(ensure_mcp_connection_sql_write_allowed(&config, true).is_ok());
    }

    #[tokio::test]
    async fn resolve_connection_refreshes_read_only_from_storage() {
        let root = std::env::temp_dir().join(format!(
            "dbx-mcp-bridge-connection-refresh-test-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let storage = dbx_core::persistence::test_storage::open(&root.join("storage.db")).await.unwrap();
        let mut config = mysql_config(false);
        storage.save_connections(&[config.clone()]).await.unwrap();
        let state = Arc::new(AppState::new_with_plugin_dir(storage, root.join("plugins")));

        let initial = resolve_connection(&state, Some(&config.id), &config.name).await.unwrap();
        assert!(!initial.read_only);

        config.read_only = true;
        state.storage.save_connections(&[config.clone()]).await.unwrap();
        let refreshed = resolve_connection(&state, Some(&config.id), &config.name).await.unwrap();
        assert!(refreshed.read_only);

        drop(state);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn mcp_sql_rejects_persistent_database_switches() {
        assert!(ensure_mcp_sql_database_switch_allowed(DatabaseType::Mysql, "SELECT 1").is_ok());
        assert_eq!(
            ensure_mcp_sql_database_switch_allowed(DatabaseType::Mysql, "USE production").unwrap_err(),
            "SQL_BLOCKED: MCP does not allow USE or persistent database switching."
        );
    }

    #[test]
    fn mcp_allowlist_distinguishes_all_subset_and_none() {
        let all = McpGlobalPolicy {
            read_only: false,
            allow_dangerous_sql: false,
            allowed_connection_ids: None,
            ..Default::default()
        };
        assert!(ensure_connection_in_mcp_scope(&all, "conn-1").is_ok());

        let subset = McpGlobalPolicy {
            read_only: false,
            allow_dangerous_sql: false,
            allowed_connection_ids: Some(vec!["conn-1".to_string()]),
            ..Default::default()
        };
        assert!(ensure_connection_in_mcp_scope(&subset, "conn-1").is_ok());
        assert!(ensure_connection_in_mcp_scope(&subset, "conn-2").unwrap_err().starts_with("CONNECTION_OUT_OF_SCOPE:"));

        let none = McpGlobalPolicy {
            read_only: false,
            allow_dangerous_sql: false,
            allowed_connection_ids: Some(Vec::new()),
            ..Default::default()
        };
        assert!(ensure_connection_in_mcp_scope(&none, "conn-1").is_err());
    }

    #[test]
    fn database_execution_policy_overrides_connection_and_global_defaults() {
        let policy = McpGlobalPolicy {
            read_only: false,
            allow_dangerous_sql: false,
            connection_policies: vec![McpConnectionPolicy {
                connection_id: "conn-1".to_string(),
                read_only: false,
                allow_dangerous_sql: false,
                execution_mode_configured: true,
                execution_mode_policy_version: Some(dbx_core::mcp_policy::MCP_EXECUTION_POLICY_VERSION),
                database_scope: McpDatabaseScope::Selected,
                allowed_databases: vec!["aa".to_string(), "aaa".to_string()],
                database_policies: vec![
                    McpDatabasePolicy { database_name: "aa".to_string(), read_only: false, allow_dangerous_sql: true },
                    McpDatabasePolicy { database_name: "aaa".to_string(), read_only: true, allow_dangerous_sql: false },
                ],
                allow_salesforce_dml: false,
            }],
            ..Default::default()
        };

        assert_eq!(effective_database_execution_policy(&policy, "conn-1", "aa"), (false, true));
        assert_eq!(effective_database_execution_policy(&policy, "conn-1", "aaa"), (true, false));
        assert_eq!(effective_database_execution_policy(&policy, "conn-1", "unconfigured"), (false, false));
    }

    fn plugin_connection_config(id: &str, name: &str, external_config: serde_json::Value) -> ConnectionConfig {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "name": name,
            "db_type": "plugin",
            "plugin_id": "io.dbx.ssh",
            "plugin_connection_type": "ssh",
            "host": "server.example.com",
            "port": 2222,
            "username": "deploy",
            "password": "",
            "connection_secrets": { "password": "hunter2", "totp_secret": "otpauth://secret" },
            "external_config": external_config
        }))
        .unwrap()
    }

    #[test]
    fn plugin_connection_summary_is_metadata_only() {
        let config = plugin_connection_config(
            "conn-ssh-1",
            "prod-bastion",
            serde_json::json!({ "authentication": "private-key", "read_only": true, "agent_socket": "/tmp/agent" }),
        );
        let value = serde_json::to_value(plugin_connection_summary(&config)).unwrap();
        let object = value.as_object().unwrap();
        let keys: std::collections::HashSet<_> = object.keys().cloned().collect();
        assert_eq!(
            keys,
            ["id", "name", "host", "port", "username", "authentication", "readOnly"]
                .into_iter()
                .map(String::from)
                .collect()
        );
        assert_eq!(value["id"], "conn-ssh-1");
        assert_eq!(value["host"], "server.example.com");
        assert_eq!(value["port"], 2222);
        assert_eq!(value["username"], "deploy");
        assert_eq!(value["authentication"], "private-key");
        assert_eq!(value["readOnly"], true);
        let rendered = value.to_string();
        for secret in ["hunter2", "otpauth", "password", "connection_secrets", "external_config", "agent_socket"] {
            assert!(!rendered.contains(secret), "summary must not leak {secret}");
        }
    }

    #[test]
    fn plugin_connection_summary_falls_back_to_safe_defaults() {
        let mut config = plugin_connection_config("conn-ssh-2", "fallback", serde_json::json!({}));
        config.read_only = true;
        let value = serde_json::to_value(plugin_connection_summary(&config)).unwrap();
        assert_eq!(value["authentication"], "password");
        assert_eq!(value["readOnly"], true);
    }

    #[test]
    fn plugin_connection_summaries_filter_by_plugin_and_sort_by_name() {
        let ssh_b = plugin_connection_config("b", "Beta", serde_json::json!({}));
        let ssh_a = plugin_connection_config("a", "alpha", serde_json::json!({}));
        let mut ldap = plugin_connection_config("l", "zeta", serde_json::json!({}));
        ldap.plugin_id = Some("io.dbx.ldap".to_string());
        let mysql = mysql_config(false);

        let summaries = plugin_connection_summaries(&[ssh_b, ldap, mysql, ssh_a], "io.dbx.ssh");
        let names: Vec<_> = summaries.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "Beta"]);

        assert!(plugin_connection_summaries(
            &[plugin_connection_config("a", "alpha", serde_json::json!({}))],
            "io.dbx.files"
        )
        .is_empty());
    }
}

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let resp = format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    let _ = stream.write_all(resp.as_bytes()).await;
}

async fn respond_json<T: Serialize>(stream: &mut tokio::net::TcpStream, data: &T) {
    let body = serde_json::to_string(data).unwrap_or_else(|_| "null".to_string());
    let resp =
        format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    let _ = stream.write_all(resp.as_bytes()).await;
}

async fn respond_error(stream: &mut tokio::net::TcpStream, status: &str, message: &str) {
    let body = serde_json::json!({ "error": message }).to_string();
    let resp =
        format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    let _ = stream.write_all(resp.as_bytes()).await;
}

async fn resolve_connection(
    state: &Arc<AppState>,
    connection_id: Option<&str>,
    connection_name: &str,
) -> Result<crate::models::connection::ConnectionConfig, String> {
    let (policy, group_paths) = load_mcp_policy_context(state).await?;
    let configs = state.storage.load_connections().await.map_err(|e| mcp_policy_unavailable(e.to_string()))?;
    let config = if let Some(id) = connection_id.filter(|s| !s.is_empty()) {
        configs.iter().find(|c| c.id == id).ok_or_else(|| format!("Connection with id '{}' not found", id))?
    } else {
        find_config_by_name(&configs, connection_name).ok_or_else(|| "Connection not found".to_string())?
    };
    ensure_connection_in_mcp_scope_with_groups(&policy, group_paths.get(&config.id), &config.id)?;
    let mut state_configs = state.configs.write().await;
    if !state_configs.contains_key(&config.id) {
        state_configs.insert(config.id.clone(), config.clone());
    }
    drop(state_configs);
    Ok(config.clone())
}

async fn load_mcp_policy(state: &Arc<AppState>) -> Result<McpGlobalPolicy, String> {
    state
        .storage
        .load_mcp_global_policy()
        .await
        .map(|state| state.policy())
        .map_err(|error| mcp_policy_unavailable(error.to_string()))
}

async fn load_mcp_policy_context(
    state: &Arc<AppState>,
) -> Result<(McpGlobalPolicy, HashMap<String, McpConnectionGroupPath>), String> {
    let policy = load_mcp_policy(state).await?;
    let group_paths = match state.storage.load_sidebar_layout().await {
        Ok(Some(layout)) => dbx_core::mcp_policy::connection_group_paths(&layout),
        Ok(None) => Ok(HashMap::new()),
        Err(error) => Err(error),
    };
    match group_paths {
        Ok(paths) => Ok((policy, paths)),
        Err(error) if dbx_core::mcp_policy::policy_uses_connection_groups(&policy) => {
            Err(mcp_policy_unavailable(error))
        }
        Err(_) => Ok((policy, HashMap::new())),
    }
}

fn mcp_policy_unavailable(error: String) -> String {
    if error.starts_with("MCP_POLICY_UNAVAILABLE:") {
        error
    } else {
        format!("MCP_POLICY_UNAVAILABLE: {error}")
    }
}

#[cfg(test)]
fn ensure_connection_in_mcp_scope(policy: &McpGlobalPolicy, connection_id: &str) -> Result<(), String> {
    ensure_connection_in_mcp_scope_with_groups(policy, None, connection_id)
}

fn ensure_connection_in_mcp_scope_with_groups(
    policy: &McpGlobalPolicy,
    group_path: Option<&McpConnectionGroupPath>,
    connection_id: &str,
) -> Result<(), String> {
    if !dbx_core::mcp_policy::policy_allows_connection(policy, group_path, connection_id) {
        return Err(format!(
            "CONNECTION_OUT_OF_SCOPE: connection '{connection_id}' is not allowed by DBX MCP settings"
        ));
    }
    Ok(())
}

fn ensure_database_in_mcp_scope(policy: &McpGlobalPolicy, connection_id: &str, database: &str) -> Result<(), String> {
    let Some(rule) = policy.connection_policies.iter().find(|rule| rule.connection_id == connection_id) else {
        return Ok(());
    };
    let allowed = match rule.database_scope {
        McpDatabaseScope::All => true,
        McpDatabaseScope::Selected => rule.allowed_databases.iter().any(|allowed| allowed == database),
        McpDatabaseScope::None => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(format!(
            "DATABASE_OUT_OF_SCOPE: database '{database}' is not allowed by DBX MCP settings for connection '{connection_id}'"
        ))
    }
}

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

async fn ensure_mcp_write_allowed(
    state: &Arc<AppState>,
    config: &crate::models::connection::ConnectionConfig,
    database: &str,
    action: &str,
) -> Result<(), String> {
    ensure_mcp_write_allowed_with_risk(state, config, database, action, false).await
}

async fn ensure_mcp_write_allowed_with_risk(
    state: &Arc<AppState>,
    config: &crate::models::connection::ConnectionConfig,
    database: &str,
    action: &str,
    dangerous: bool,
) -> Result<(), String> {
    let (policy, group_paths) = load_mcp_policy_context(state).await?;
    let group_path = group_paths.get(&config.id);
    ensure_connection_in_mcp_scope_with_groups(&policy, group_path, &config.id)?;
    ensure_database_in_mcp_scope(&policy, &config.id, database)?;
    let group_ids = group_path.map(|path| path.ids.as_slice()).unwrap_or_default();
    let (read_only, allow_dangerous_sql) =
        effective_database_execution_policy_with_groups(&policy, group_ids, &config.id, database);
    if read_only {
        return Err(format!(
            "MCP_READ_ONLY: MCP execution permission for database '{database}' is read-only. {action} blocked."
        ));
    }
    if dangerous && !allow_dangerous_sql {
        return Err(format!("SQL_BLOCKED: High-risk operation '{action}' is disabled in DBX MCP settings."));
    }
    if config.read_only {
        return Err(format!(
            "CONNECTION_READ_ONLY: connection '{}' has read-only protection enabled. {action} blocked.",
            config.name
        ));
    }
    if dbx_core::production_safety::is_production_database(config, database) {
        return Err(format!("PRODUCTION_DATABASE_READ_ONLY: {action} blocked for production database '{database}'."));
    }
    Ok(())
}

pub(crate) async fn ensure_mcp_read_allowed_by_id(
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
) -> Result<(), String> {
    let (policy, group_paths) = load_mcp_policy_context(state).await?;
    ensure_connection_in_mcp_scope_with_groups(&policy, group_paths.get(connection_id), connection_id)?;
    ensure_database_in_mcp_scope(&policy, connection_id, database)?;
    let configs = state.storage.load_connections().await.map_err(|e| format!("MCP_POLICY_UNAVAILABLE: {e}"))?;
    let config = configs
        .iter()
        .find(|config| config.id == connection_id)
        .ok_or_else(|| format!("Connection with id '{connection_id}' not found"))?;
    check_visible_database(config, database)
}

pub(crate) async fn ensure_mcp_write_allowed_by_id(
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
    action: &str,
) -> Result<(), String> {
    ensure_mcp_write_allowed_by_id_with_risk(state, connection_id, database, action, false).await
}

pub(crate) async fn ensure_mcp_dangerous_write_allowed_by_id(
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
    action: &str,
) -> Result<(), String> {
    ensure_mcp_write_allowed_by_id_with_risk(state, connection_id, database, action, true).await
}

async fn ensure_mcp_write_allowed_by_id_with_risk(
    state: &Arc<AppState>,
    connection_id: &str,
    database: &str,
    action: &str,
    dangerous: bool,
) -> Result<(), String> {
    let configs = state.storage.load_connections().await.map_err(|e| format!("MCP_POLICY_UNAVAILABLE: {e}"))?;
    let config = configs
        .iter()
        .find(|config| config.id == connection_id)
        .ok_or_else(|| format!("Connection with id '{connection_id}' not found"))?;
    ensure_mcp_write_allowed_with_risk(state, config, database, action, dangerous).await
}

async fn ensure_mcp_sql_allowed(
    state: &Arc<AppState>,
    config: &crate::models::connection::ConnectionConfig,
    database: &str,
    sql: &str,
) -> Result<(), String> {
    let (policy, group_paths) = load_mcp_policy_context(state).await?;
    let group_path = group_paths.get(&config.id);
    ensure_connection_in_mcp_scope_with_groups(&policy, group_path, &config.id)?;
    ensure_database_in_mcp_scope(&policy, &config.id, database)?;
    if let Some(rule) = policy.connection_policies.iter().find(|rule| rule.connection_id == config.id) {
        if rule.database_scope == McpDatabaseScope::Selected
            && dbx_core::production_safety::sql_references_disallowed_database(
                sql,
                &config.db_type,
                database,
                &rule.allowed_databases,
            )
        {
            return Err(
                "DATABASE_OUT_OF_SCOPE: SQL references a database that is not allowed by DBX MCP settings for this connection."
                    .to_string(),
            );
        }
    }
    ensure_mcp_sql_database_switch_allowed(config.db_type, sql)?;
    let is_write = dbx_core::query_execution_sql::is_write_sql_for_database(sql, config.db_type);
    dbx_core::mcp_policy::ensure_sql_database_execution_scope(&policy, config, database, sql)?;
    let group_ids = group_path.map(|path| path.ids.as_slice()).unwrap_or_default();
    let (read_only, allow_dangerous_sql) =
        effective_database_execution_policy_with_groups(&policy, group_ids, &config.id, database);
    if read_only && is_write {
        return Err(format!(
            "MCP_READ_ONLY: MCP execution permission for database '{database}' is read-only. SQL write blocked."
        ));
    }
    if !allow_dangerous_sql && dbx_core::sql_risk::is_dangerous_sql_for_database(sql, config.db_type) {
        return Err("SQL_BLOCKED: High-risk SQL is disabled in DBX MCP settings.".to_string());
    }
    ensure_mcp_connection_sql_write_allowed(config, is_write)?;
    if is_write && dbx_core::production_safety::targets_production_database(config, database, sql) {
        return Err("PRODUCTION_DATABASE_READ_ONLY: SQL write targeting production scope is blocked.".to_string());
    }
    Ok(())
}

fn ensure_mcp_sql_database_switch_allowed(
    database_type: crate::models::connection::DatabaseType,
    sql: &str,
) -> Result<(), String> {
    if dbx_core::sql_risk::mcp_sql_has_forbidden_database_switch(sql, database_type) {
        return Err("SQL_BLOCKED: MCP does not allow USE or persistent database switching.".to_string());
    }
    Ok(())
}

fn ensure_mcp_connection_sql_write_allowed(
    config: &crate::models::connection::ConnectionConfig,
    is_write: bool,
) -> Result<(), String> {
    if is_write && config.read_only {
        return Err(format!(
            "CONNECTION_READ_ONLY: connection '{}' has read-only protection enabled. SQL write blocked.",
            config.name
        ));
    }
    Ok(())
}

fn check_visible_database(config: &crate::models::connection::ConnectionConfig, database: &str) -> Result<(), String> {
    if let Some(ref visible) = config.visible_databases {
        if !visible.is_empty() && !visible.iter().any(|v| v == database) {
            return Err(format!("Database '{}' is not in the visible databases list for this connection", database));
        }
    }
    Ok(())
}

async fn handle_open_table(app: &AppHandle, state: &Arc<AppState>, body: &str, stream: &mut tokio::net::TcpStream) {
    let req: OpenTableRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => {
            respond(stream, "400 Bad Request", "").await;
            return;
        }
    };
    let config = match resolve_connection(state, req.connection_id.as_deref(), &req.connection_name).await {
        Ok(c) => c,
        Err(e) => {
            respond(stream, "404 Not Found", &e).await;
            return;
        }
    };
    let event = McpOpenTableEvent {
        connection_id: config.id.clone(),
        database: req.database.unwrap_or_else(|| config.database.clone().unwrap_or_default()),
        schema: req.schema,
        table: req.table,
    };
    let _ = app.emit("mcp-open-table", &event);
    respond(stream, "200 OK", "ok").await;
}

#[derive(Deserialize)]
struct CallPluginToolRequest {
    connection_id: String,
    tool: String,
    #[serde(default)]
    arguments: serde_json::Value,
    plugin_id: Option<String>,
    timeout_ms: Option<u64>,
}

/// Tools whose terminal routing (`runInTerminal` / terminal MCP mode) can
/// require the workbench terminal to exist. Every other forwarded tool runs
/// on the hidden channel by design and must never open the workbench.
fn is_terminal_routed_exec(tool: &str) -> bool {
    tool == "ssh_exec" || tool == "ssh_exec_sudo"
}

/// Asks the plugin for a connection's terminal MCP mode (the toggle in the
/// SSH terminal toolbar). Any failure — an older plugin without the probe
/// route, a busy sidecar — degrades to off, so the silent path can never
/// regress into opening the workbench.
async fn plugin_agent_mode_on(state: &Arc<AppState>, plugin_id: &str, connection_id: &str) -> bool {
    let probe: Result<serde_json::Value, String> = state
        .plugin_host
        .invoke(
            plugin_id,
            "ssh/agent/mode/get",
            serde_json::json!({ "connectionId": connection_id }),
            None,
            Some(std::time::Duration::from_secs(5)),
        )
        .await;
    probe
        .ok()
        .and_then(|value| value.get("agentTerminalMode").and_then(serde_json::Value::as_str).map(str::to_string))
        .is_some_and(|mode| mode == "auto" || mode == "strict")
}

/// `/call-plugin-tool` may only talk to the saved connection's own plugin:
/// the lifecycle payload (which carries the connection's credentials) is
/// built from the config, so honoring a request-named `plugin_id` would hand
/// connection A's secrets to plugin B's sidecar. Callers may repeat the
/// binding, never override it.
fn resolve_call_plugin_target(
    config_plugin_id: Option<&str>,
    request_plugin_id: Option<&str>,
) -> Result<String, String> {
    let bound = config_plugin_id
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "connection is not bound to a plugin".to_string())?;
    if let Some(requested) = request_plugin_id {
        if requested != bound {
            return Err(format!("plugin_id {requested:?} does not match the connection's plugin {bound:?}"));
        }
    }
    Ok(bound.to_string())
}

/// POST /call-plugin-tool: runs a plugin MCP tool on the desktop app's own
/// plugin session — the same sidecar process the workbench talks to. The
/// connection's workbench tab is opened only when the call will route into
/// the visible terminal (ssh runInTerminal / terminal MCP mode), so
/// agent-terminal commands execute in a real PTY while silent hidden-channel
/// calls never open or focus the app. The lifecycle payload comes from the
/// saved connection config, so callers never pass credentials.
async fn handle_call_plugin_tool(
    app: &AppHandle,
    state: &Arc<AppState>,
    body: &str,
    stream: &mut tokio::net::TcpStream,
) {
    let req: CallPluginToolRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            respond_error(stream, "400 Bad Request", &format!("invalid body: {e}")).await;
            return;
        }
    };
    let config = match resolve_connection(state, Some(&req.connection_id), "").await {
        Ok(c) => c,
        Err(e) => {
            respond_error(stream, "404 Not Found", &e).await;
            return;
        }
    };
    let plugin_id = match resolve_call_plugin_target(config.plugin_id.as_deref(), req.plugin_id.as_deref()) {
        Ok(plugin_id) => plugin_id,
        Err(message) => {
            respond_error(stream, "400 Bad Request", &message).await;
            return;
        }
    };
    // The workbench tab only needs to exist when the forwarded call will
    // actually route into the visible terminal: an explicit runInTerminal,
    // or no flag plus the connection's terminal MCP mode being on (asked
    // from the plugin, so the toggle in the terminal UI owns the decision).
    // Everything else — sftp listings, silent execs on the hidden channel —
    // must not open (let alone focus) the app while the agent works.
    let route_terminal = is_terminal_routed_exec(&req.tool)
        && match req.arguments.get("runInTerminal").and_then(serde_json::Value::as_bool) {
            Some(explicit) => explicit,
            None => plugin_agent_mode_on(state, &plugin_id, &config.id).await,
        };
    if route_terminal {
        let _ = app.emit("mcp-open-connection-workbench", serde_json::json!({ "connection_id": config.id }));
    }
    let lifecycle = match state.plugin_host.connection_params_standalone(&config) {
        Ok(l) => l,
        Err(e) => {
            respond_error(stream, "500 Internal Server Error", &e).await;
            return;
        }
    };
    let params = serde_json::json!({
        "tool": req.tool,
        "arguments": req.arguments,
        "lifecycle": lifecycle,
    });
    // Long ceiling: agent-terminal calls may sit in a workbench approval
    // prompt (default 120s) before the command even starts.
    let timeout = std::time::Duration::from_millis(req.timeout_ms.unwrap_or(300_000).clamp(1_000, 600_000));
    let result: Result<serde_json::Value, String> =
        state.plugin_host.invoke(&plugin_id, "mcp/call", params, None, Some(timeout)).await;
    match result {
        Ok(value) => respond_json(stream, &value).await,
        Err(e) => respond_error(stream, "502 Bad Gateway", &e).await,
    }
}

#[derive(Deserialize)]
struct ListPluginConnectionsRequest {
    plugin_id: String,
}

/// Metadata-only view of a saved plugin connection. This is a deliberate
/// field whitelist: credentials (`password`, `connection_secrets`, private
/// keys, `sudo_password`, `totp_secret`, ...) can never reach the response
/// because they are not part of this struct and `external_config` is only
/// probed for the two safe keys below.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginConnectionSummary {
    id: String,
    name: String,
    host: String,
    port: u16,
    username: String,
    authentication: String,
    read_only: bool,
}

fn plugin_connection_summary(config: &crate::models::connection::ConnectionConfig) -> PluginConnectionSummary {
    let external = config.external_config.as_ref();
    // The dialog persists every declared config-bound field (including
    // defaults), so `authentication` is normally present; fall back to the
    // provider's historical default ("password") for pre-existing rows.
    let authentication = external
        .and_then(|v| v.get("authentication"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("password")
        .to_string();
    let read_only =
        external.and_then(|v| v.get("read_only")).and_then(serde_json::Value::as_bool).unwrap_or(config.read_only);
    PluginConnectionSummary {
        id: config.id.clone(),
        name: config.name.clone(),
        host: config.host.clone(),
        port: config.port,
        username: config.username.clone(),
        authentication,
        read_only,
    }
}

fn plugin_connection_summaries(
    configs: &[crate::models::connection::ConnectionConfig],
    plugin_id: &str,
) -> Vec<PluginConnectionSummary> {
    let mut summaries: Vec<PluginConnectionSummary> =
        configs.iter().filter(|c| false).map(plugin_connection_summary).collect();
    summaries.sort_by_key(|summary| summary.name.to_lowercase());
    summaries
}

/// POST /list-plugin-connections: returns saved connection metadata for one
/// plugin so standalone stdio MCP agents can pick a connection without
/// reading the SQLite store. Metadata only — this route never returns
/// credentials; connecting still goes through /call-plugin-tool, whose
/// lifecycle payload comes from the host-side saved config.
async fn handle_list_plugin_connections(state: &Arc<AppState>, body: &str, stream: &mut tokio::net::TcpStream) {
    let req: ListPluginConnectionsRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(e) => {
            respond_error(stream, "400 Bad Request", &format!("invalid body: {e}")).await;
            return;
        }
    };
    if req.plugin_id.trim().is_empty() {
        respond_error(stream, "400 Bad Request", "plugin_id is required").await;
        return;
    }
    let configs = match state.storage.load_connections().await {
        Ok(c) => c,
        Err(e) => {
            respond_error(stream, "500 Internal Server Error", &e).await;
            return;
        }
    };
    let connections = plugin_connection_summaries(&configs, req.plugin_id.trim());
    respond_json(stream, &serde_json::json!({ "connections": connections })).await;
}

async fn handle_execute_query(app: &AppHandle, state: &Arc<AppState>, body: &str, stream: &mut tokio::net::TcpStream) {
    let req: ExecuteQueryRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => {
            respond(stream, "400 Bad Request", "").await;
            return;
        }
    };
    let config = match resolve_connection(state, req.connection_id.as_deref(), &req.connection_name).await {
        Ok(c) => c,
        Err(e) => {
            respond(stream, "404 Not Found", &e).await;
            return;
        }
    };
    let database = req.database.unwrap_or_else(|| config.database.clone().unwrap_or_default());
    if let Err(error) = ensure_mcp_execute_and_show_supported(&config.db_type) {
        respond(stream, "400 Bad Request", error).await;
        return;
    }
    // Check the complete batch first so a session-changing statement cannot
    // hide a later production write, then recheck every statement immediately
    // before it reaches the database.
    if let Err(e) = ensure_mcp_sql_allowed(state, &config, &database, &req.sql).await {
        respond(stream, "403 Forbidden", &e).await;
        return;
    }
    let statements = { dbx_core::sql::split_sql_statements_for_database(&req.sql, config.db_type) };
    let statements = if statements.is_empty() { vec![req.sql.clone()] } else { statements };
    let mut results = Vec::with_capacity(statements.len());
    for statement in statements {
        let current = match resolve_connection(state, Some(&config.id), &config.name).await {
            Ok(config) => config,
            Err(e) => {
                respond(stream, "403 Forbidden", &e).await;
                return;
            }
        };
        if let Err(e) = ensure_mcp_sql_allowed(state, &current, &database, &statement).await {
            respond(stream, "403 Forbidden", &e).await;
            return;
        }
        match dbx_core::query::execute_sql_statement(state, &current.id, &database, &statement, None, None).await {
            Ok(result) => results.push(result),
            Err(e) => {
                respond(stream, "500 Internal Server Error", &format!("QUERY_ERROR: {e}")).await;
                return;
            }
        }
    }
    let event = McpExecuteQueryEvent { connection_id: config.id.clone(), database, sql: req.sql, results };
    let _ = app.emit("mcp-execute-query", &event);
    respond(stream, "200 OK", "ok").await;
}

fn ensure_mcp_execute_and_show_supported(
    database_type: &crate::models::connection::DatabaseType,
) -> Result<(), &'static str> {
    if dbx_core::query_execution_sql::supports_sql_query(*database_type) {
        Ok(())
    } else {
        Err(MCP_EXECUTE_AND_SHOW_SQL_ONLY)
    }
}

async fn handle_list_tables_data(state: &Arc<AppState>, body: &str, stream: &mut tokio::net::TcpStream) {
    let req: ListTablesRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => {
            respond_error(stream, "400 Bad Request", "Invalid JSON").await;
            return;
        }
    };
    let config = match resolve_connection(state, req.connection_id.as_deref(), &req.connection_name).await {
        Ok(c) => c,
        Err(e) => {
            respond_error(stream, "404 Not Found", &e).await;
            return;
        }
    };
    let database = req.database.unwrap_or_else(|| config.database.clone().unwrap_or_default());
    let schema = req.schema.unwrap_or_default();
    if let Err(e) = check_visible_database(&config, &database) {
        respond_error(stream, "403 Forbidden", &e).await;
        return;
    }
    match dbx_core::schema::list_tables_core(state, &config.id, &database, &schema, None, None, None, None, None).await
    {
        Ok(tables) => respond_json(stream, &tables).await,
        Err(e) => respond_error(stream, "500 Internal Server Error", &e).await,
    }
}

async fn handle_describe_table_data(state: &Arc<AppState>, body: &str, stream: &mut tokio::net::TcpStream) {
    let req: DescribeTableRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => {
            respond_error(stream, "400 Bad Request", "Invalid JSON").await;
            return;
        }
    };
    let config = match resolve_connection(state, req.connection_id.as_deref(), &req.connection_name).await {
        Ok(c) => c,
        Err(e) => {
            respond_error(stream, "404 Not Found", &e).await;
            return;
        }
    };
    let database = req.database.unwrap_or_else(|| config.database.clone().unwrap_or_default());
    let schema = req.schema.unwrap_or_default();
    if let Err(e) = check_visible_database(&config, &database) {
        respond_error(stream, "403 Forbidden", &e).await;
        return;
    }
    match dbx_core::schema::get_columns_core(state, &config.id, &database, &schema, &req.table).await {
        Ok(columns) => respond_json(stream, &columns).await,
        Err(e) => respond_error(stream, "500 Internal Server Error", &e).await,
    }
}

async fn handle_execute_query_data(state: &Arc<AppState>, body: &str, stream: &mut tokio::net::TcpStream) {
    let req: ExecuteQueryRequest = match serde_json::from_str(body) {
        Ok(r) => r,
        Err(_) => {
            respond_error(stream, "400 Bad Request", "Invalid JSON").await;
            return;
        }
    };
    let config = match resolve_connection(state, req.connection_id.as_deref(), &req.connection_name).await {
        Ok(c) => c,
        Err(e) => {
            respond_error(stream, "404 Not Found", &e).await;
            return;
        }
    };
    let database = req.database.unwrap_or_else(|| config.database.clone().unwrap_or_default());
    if let Err(e) = check_visible_database(&config, &database) {
        respond_error(stream, "403 Forbidden", &e).await;
        return;
    }
    if let Err(e) = ensure_mcp_sql_allowed(state, &config, &database, &req.sql).await {
        respond_error(stream, "403 Forbidden", &e).await;
        return;
    }
    let statements = { dbx_core::sql::split_sql_statements_for_database(&req.sql, config.db_type) };
    let statements = if statements.is_empty() { vec![req.sql] } else { statements };
    let mut last_result = None;
    for statement in statements {
        // Re-read both policy and connection settings immediately before every
        // statement so a settings change can stop the remainder of the batch.
        let current = match resolve_connection(state, Some(&config.id), &config.name).await {
            Ok(config) => config,
            Err(e) => {
                respond_error(stream, "403 Forbidden", &e).await;
                return;
            }
        };
        if let Err(e) = ensure_mcp_sql_allowed(state, &current, &database, &statement).await {
            respond_error(stream, "403 Forbidden", &e).await;
            return;
        }
        match dbx_core::query::execute_sql_statement(
            state,
            &current.id,
            &database,
            &statement,
            req.schema.as_deref(),
            None,
        )
        .await
        {
            Ok(result) => last_result = Some(result),
            Err(e) => {
                respond_error(stream, "500 Internal Server Error", &e).await;
                return;
            }
        }
    }
    if let Some(result) = last_result {
        respond_json(stream, &result).await;
    } else {
        respond_error(stream, "500 Internal Server Error", "No SQL statement to execute").await;
    }
}
