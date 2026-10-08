pub mod batch_progress;

pub mod object_cache;
pub mod plugin_data;
pub mod plugin_plan;
pub mod query_cancel;

pub mod two_phase_commit;

pub use dbx_drivers::execution::{
    await_stream_with_progress_timeout, canceled_error, is_canceled, lock_shared_client_with_wait,
    query_timeout_duration, resolve_query_timeout, timeout_error, timeout_error_for, wait_for_query,
    wait_for_query_opt, wait_for_query_with_timeout, wait_for_result_opt, wait_for_result_with_timeout,
    wait_for_value_opt, DbOperationBudget, StreamProgressClock, MAX_ROWS, QUERY_CANCELED, QUERY_TIMEOUT,
};

use futures::StreamExt;
use mysql_async::prelude::Queryable;
use serde::{Deserialize, Serialize};
use sqlparser::ast::{
    visit_relations_mut, Ident, ObjectName, ObjectNamePart, ObjectType, Statement, TableFactor, VisitMut, VisitorMut,
};
use sqlparser::dialect::{GenericDialect, MsSqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::connection::{AppState, PoolKind, TransactionSession, TxnConnection};
use crate::database_capabilities;
use crate::db;
use crate::driver_error::{AgentCallError, AgentErrorStage, AgentOperationOutcome};
use crate::models::connection::{ConnectionConfig, DatabaseType};
use crate::query_execution_sql::{is_write_sql, strip_sql_comments_and_literals};
use crate::sql::{split_sql_batches, split_sql_statements, starts_with_executable_sql_keyword_for_database};
use crate::sql_dialect::{resolve_for_db, CAP_TRANSACTIONAL_DDL};
use crate::sql_risk::{classify_sql_risk_for_database, SqlRisk};

pub const AGENT_PROTOCOL_MAX_ROWS: usize = i32::MAX as usize;
pub const METADATA_POOL_BUSY_ERROR: &str = "DBX metadata pool is busy; please retry";
pub const MANUAL_TRANSACTION_IDLE_TIMEOUT_SECS: u64 = 300;
pub const MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR: &str =
    "Transaction session not found or expired; it may have been auto-rolled back due to inactivity";
pub const MANUAL_TRANSACTION_IDLE_ROLLBACK_ERROR: &str =
    "Transaction was auto-rolled back due to 5 minutes of inactivity";

pub fn is_manual_transaction_session_expired_error(error: &str) -> bool {
    error.starts_with(MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR) || error == MANUAL_TRANSACTION_IDLE_ROLLBACK_ERROR
}

/// Returns true when a metadata request failed because all pool/client slots
/// were temporarily occupied. This is deliberately separate from
/// `is_connection_error`: a saturated pool is healthy and must not trigger a
/// shared-pool reconnect.
pub fn is_pool_saturation_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("connection pool checkout timed out [stage=wait") || lower.contains("dbx metadata pool is busy")
}
/// Fallback when a Mongo connection hits the generic SQL executor instead of the shell path.
/// Wording must match packages/mongo-shell `MONGO_SHELL_COMMAND_HINT`
/// (desktop/CLI diagnose first; this is only the Rust SQL-executor backstop).
const MONGO_SHELL_COMMAND_HINT: &str = "Use MongoDB shell-style commands, for example: db.collection.find({}).limit(100), db.collection.aggregate([]), db.collection.aggregate([], { explain: true }), db.version(), db.collection.countDocuments({}), db.collection.distinct(\"field\"), db.collection.getIndexes(), db.collection.createIndex({...}), db.createUser({...}), or db.collection.insertOne({...}).";
const SQL_OMITTED_ERROR_CONTEXT: &str =
    "SQL text omitted from user-facing error; enable debug SQL diagnostics to inspect the original statement.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolErrorAction {
    Keep,
    Discard,
    ReconnectAndRetry,
}

#[derive(Debug, Clone)]
pub enum QueryExecutionError {
    Canceled {
        stage: AgentErrorStage,
        operation_outcome: AgentOperationOutcome,
    },
    Timeout(String),
    Sql(String),
    /// Native PostgreSQL SQL failure whose driver-reported cursor position was
    /// resolved against the executed statement text. Kept distinct from
    /// [`Self::Sql`] so the position survives `classify_query_error` and reaches
    /// [`Self::into_backend_error`] as a typed field instead of being parsed back
    /// out of a string.
    SqlWithPosition {
        message: String,
        position: crate::sql_error_position::SqlErrorPosition,
    },
    Legacy(String),
}

impl QueryExecutionError {
    pub fn into_legacy_string(self) -> String {
        let mut message = match self {
            Self::Canceled { .. } => canceled_error(),
            Self::Timeout(error) => error,
            Self::Sql(error) => error,
            Self::SqlWithPosition { message, .. } => message,
            Self::Legacy(error) => error,
        };
        // Defensive: any remaining transport marker (e.g. from a driver error
        // that never passed through the resolve step) must not reach clients.
        while crate::sql_error_position::take_marker(&mut message).is_some() {}
        message
    }

    pub fn into_backend_error(self) -> crate::backend_error::BackendError {
        match self {
            Self::Canceled { stage, operation_outcome } => {
                crate::backend_error::BackendError::from_canceled(stage, operation_outcome)
            }
            Self::Timeout(error) => crate::backend_error::BackendError::from_timeout_detail(&error),
            Self::Sql(error) => crate::backend_error::BackendError::from_sql_detail(&error),
            Self::SqlWithPosition { message, position } => {
                crate::backend_error::BackendError::from_sql_detail_with_position(&message, position)
            }
            Self::Legacy(error) => crate::backend_error::BackendError::from_legacy_string(&error),
        }
    }

    fn with_omitted_sql_context(self, sql: &str) -> Self {
        match self {
            canceled @ Self::Canceled { .. } => canceled,
            Self::Timeout(error) => Self::Timeout(query_error_with_omitted_sql_context(&error, sql)),
            Self::Sql(error) => Self::Sql(append_typed_sql_error_context(&error, sql)),
            Self::SqlWithPosition { message, position } => {
                Self::SqlWithPosition { message: append_typed_sql_error_context(&message, sql), position }
            }
            Self::Legacy(error) => Self::Legacy(query_error_with_omitted_sql_context(&error, sql)),
        }
    }
}

impl std::fmt::Display for QueryExecutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Canceled { .. } => formatter.write_str(QUERY_CANCELED),
            Self::Timeout(error) => formatter.write_str(error),
            Self::Sql(error) => formatter.write_str(error),
            Self::SqlWithPosition { message, .. } => formatter.write_str(message),
            Self::Legacy(error) => formatter.write_str(error),
        }
    }
}

impl From<String> for QueryExecutionError {
    fn from(error: String) -> Self {
        Self::Legacy(error)
    }
}

impl From<&str> for QueryExecutionError {
    fn from(error: &str) -> Self {
        Self::Legacy(error.to_string())
    }
}

fn query_error_with_omitted_sql_context(error: &str, _sql: &str) -> String {
    crate::driver_error::append_legacy_error_context(error, SQL_OMITTED_ERROR_CONTEXT)
}

fn append_typed_sql_error_context(error: &str, _sql: &str) -> String {
    if error.contains(SQL_OMITTED_ERROR_CONTEXT) {
        return error.to_string();
    }
    let separator = if error.trim_start().starts_with("Server error:") { " " } else { "\n" };
    format!("{error}{separator}{SQL_OMITTED_ERROR_CONTEXT}")
}

/// A multi-statement result with metadata intended for query clients.
///
/// `execution_error` is emitted for synthesized per-statement errors so clients
/// can distinguish them from a successful result column named `Error`.
/// `statement_index` is emitted only after a concrete statement starts running.
/// `server_message` is emitted only for SQL Server TDS informational messages.
#[derive(Debug, Clone, Serialize)]
pub struct ExecuteMultiResult {
    #[serde(flatten)]
    pub result: db::QueryResult,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub large_value_cells: Vec<db::LargeValueCell>,
    #[serde(skip_serializing_if = "is_false")]
    pub execution_error: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub statement_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<crate::backend_error::BackendError>,
    #[serde(skip_serializing_if = "is_false")]
    pub server_message: bool,
    /// Manual-transaction UX metadata for sticky proven-read-only dialects
    /// (Oracle, OceanBase-Oracle, MySQL, PostgreSQL): true only for a statement
    /// proven to be an ordinary read by that dialect's strict heuristic.
    /// Absent/false for unproven statements and non-participating dialects.
    /// Not part of the reusable database-result model (`db::QueryResult`).
    #[serde(skip_serializing_if = "is_false")]
    pub manual_transaction_proven_read_only: bool,
    /// Manual-transaction UX metadata for the same dialects: true on the
    /// synthetic successful result when the manual-execution splitter found zero
    /// statements (empty/whitespace/comments-only script). Lets the frontend
    /// treat it as a no-op rather than an unproven statement.
    #[serde(skip_serializing_if = "is_false")]
    pub manual_transaction_no_statement: bool,
    /// MySQL auto-commit tab: set on every result of the batch when the tab's
    /// connection was settled. `Some(true)` means the connection still holds a
    /// transaction the user opened explicitly and DBX kept it open
    /// (`preserve_explicit_transaction`); `Some(false)` means the settlement ran
    /// and no such transaction is open. `None` means this execution never
    /// observed a tab-scoped MySQL connection, so it says nothing about the
    /// tab's state. The frontend mirrors `Some(..)` into the tab's transaction
    /// badge/actions so the state is never silent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_commit_open_transaction: Option<bool>,
    /// MySQL auto-commit tab: set on every result of the batch when DBX rolled
    /// back a transaction the user opened explicitly (`BEGIN` /
    /// `START TRANSACTION`) and left open. Lets the UI report the cleanup
    /// instead of discarding the transaction silently.
    #[serde(skip_serializing_if = "is_false")]
    pub auto_commit_explicit_transaction_rolled_back: bool,
    /// MySQL auto-commit tab: set on every result of the batch when DBX rolled
    /// back a transaction nobody opened explicitly — the session turned
    /// auto-commit off (`SET autocommit = 0`), so its transactions are implicit.
    /// Reported separately from the explicit case: the user never asked for a
    /// transaction, so the tab shows a distinct notice that is raised once per
    /// connection instead of repeating "your explicit transaction was rolled
    /// back" after every execution.
    #[serde(skip_serializing_if = "is_false")]
    pub auto_commit_session_autocommit_rolled_back: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExecuteMultiProgress {
    pub statement_index: usize,
    pub completed: usize,
    pub total: usize,
    pub success: bool,
    pub execution_time_ms: u128,
    pub affected_rows: u64,
    pub error: Option<crate::backend_error::BackendError>,
}

pub type ExecuteMultiProgressCallback = Arc<dyn Fn(ExecuteMultiProgress) + Send + Sync>;

fn report_execute_multi_progress(
    progress: Option<&ExecuteMultiProgressCallback>,
    statement_index: usize,
    total: usize,
    result: &db::QueryResult,
    success: bool,
    error: Option<crate::backend_error::BackendError>,
) {
    if let Some(progress) = progress {
        progress(ExecuteMultiProgress {
            statement_index,
            completed: statement_index + 1,
            total,
            success,
            execution_time_ms: result.execution_time_ms,
            affected_rows: result.affected_rows,
            error,
        });
    }
}

impl ExecuteMultiResult {
    fn execution_error(result: db::QueryResult) -> Self {
        let error = error_from_query_result(&result);
        Self {
            result,
            large_value_cells: Vec::new(),
            execution_error: true,
            statement_index: None,
            error,
            server_message: false,
            manual_transaction_proven_read_only: false,
            manual_transaction_no_statement: false,
            auto_commit_open_transaction: None,
            auto_commit_explicit_transaction_rolled_back: false,
            auto_commit_session_autocommit_rolled_back: false,
        }
    }

    #[cfg(test)]
    fn execution_error_with_index(result: db::QueryResult, statement_index: usize) -> Self {
        let error = error_from_query_result(&result);
        Self {
            result,
            large_value_cells: Vec::new(),
            execution_error: true,
            statement_index: Some(statement_index),
            error,
            server_message: false,
            manual_transaction_proven_read_only: false,
            manual_transaction_no_statement: false,
            auto_commit_open_transaction: None,
            auto_commit_explicit_transaction_rolled_back: false,
            auto_commit_session_autocommit_rolled_back: false,
        }
    }

    fn execution_error_with_backend(
        result: db::QueryResult,
        statement_index: Option<usize>,
        error: crate::backend_error::BackendError,
    ) -> Self {
        Self {
            result,
            large_value_cells: Vec::new(),
            execution_error: true,
            statement_index,
            error: Some(error),
            server_message: false,
            manual_transaction_proven_read_only: false,
            manual_transaction_no_statement: false,
            auto_commit_open_transaction: None,
            auto_commit_explicit_transaction_rolled_back: false,
            auto_commit_session_autocommit_rolled_back: false,
        }
    }

    fn success_with_index(result: db::QueryResult, statement_index: usize) -> Self {
        Self {
            result,
            large_value_cells: Vec::new(),
            execution_error: false,
            statement_index: Some(statement_index),
            error: None,
            server_message: false,
            manual_transaction_proven_read_only: false,
            manual_transaction_no_statement: false,
            auto_commit_open_transaction: None,
            auto_commit_explicit_transaction_rolled_back: false,
            auto_commit_session_autocommit_rolled_back: false,
        }
    }

    fn success_with_index_and_large_values(
        mut result: db::QueryResult,
        statement_index: usize,
        mut large_value_cells: Vec<db::LargeValueCell>,
        table_data_preview: bool,
    ) -> Self {
        let server_cells = if table_data_preview {
            remap_large_value_cells_around_server_markers(&result, &mut large_value_cells);
            extract_server_large_value_markers(&mut result)
        } else {
            Vec::new()
        };
        Self {
            result,
            large_value_cells: merge_large_value_cells(large_value_cells, server_cells),
            execution_error: false,
            statement_index: Some(statement_index),
            error: None,
            server_message: false,
            manual_transaction_proven_read_only: false,
            manual_transaction_no_statement: false,
            auto_commit_open_transaction: None,
            auto_commit_explicit_transaction_rolled_back: false,
            auto_commit_session_autocommit_rolled_back: false,
        }
    }

    fn success_with_index_and_optional_server_large_values(
        result: db::QueryResult,
        statement_index: usize,
        table_data_preview: bool,
    ) -> Self {
        Self::success_with_index_and_large_values(result, statement_index, Vec::new(), table_data_preview)
    }

    fn success_with_optional_server_large_values(mut result: db::QueryResult, table_data_preview: bool) -> Self {
        let large_value_cells =
            if table_data_preview { extract_server_large_value_markers(&mut result) } else { Vec::new() };
        Self {
            result,
            large_value_cells,
            execution_error: false,
            statement_index: None,
            error: None,
            server_message: false,
            manual_transaction_proven_read_only: false,
            manual_transaction_no_statement: false,
            auto_commit_open_transaction: None,
            auto_commit_explicit_transaction_rolled_back: false,
            auto_commit_session_autocommit_rolled_back: false,
        }
    }

    fn with_manual_transaction_proven_read_only(mut self) -> Self {
        self.manual_transaction_proven_read_only = true;
        self
    }

    fn with_manual_transaction_no_statement(mut self) -> Self {
        self.manual_transaction_no_statement = true;
        self
    }

    pub fn without_error_detail(mut self) -> Self {
        self.error = self.error.map(crate::backend_error::BackendError::without_detail);
        self
    }

    fn into_query_result(self) -> db::QueryResult {
        self.result
    }
}

const SERVER_LARGE_VALUE_UNKNOWN_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
enum ServerLargeValuePreviewKind {
    Text,
    Binary,
    Vector,
    Deferred,
}

#[derive(Clone, Copy)]
struct ServerLargeValueMarker {
    result_index: usize,
    source_index: usize,
    preview_kind: Option<ServerLargeValuePreviewKind>,
    source_type: Option<&'static str>,
}

#[derive(Clone, Copy)]
struct ServerLargeValueMarkerValue {
    kind: ServerLargeValuePreviewKind,
    preview_size: usize,
    original_bytes: Option<usize>,
}

/// Some PostgreSQL-compatible servers (for example KingbaseES instances with
/// case-insensitive identifiers) fold even quoted column aliases, so the
/// internal preview marker prefix must be matched ASCII case-insensitively.
fn strip_large_value_marker_prefix(column: &str) -> Option<&str> {
    let prefix = crate::sql_dialect::DBX_LARGE_VALUE_BYTES_COLUMN_PREFIX;
    if column.len() < prefix.len() {
        return None;
    }
    let head = column.get(..prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    column.get(prefix.len()..)
}

fn server_large_value_alias(
    suffix: &str,
) -> Option<(usize, Option<ServerLargeValuePreviewKind>, Option<&'static str>)> {
    if let Ok(source_index) = suffix.parse::<usize>() {
        return Some((source_index, None, None));
    }
    let (kind, source_index) = suffix.split_once('_')?;
    let source_index = source_index.parse::<usize>().ok()?;
    let (preview_kind, source_type) = match kind {
        "T" | "t" => (ServerLargeValuePreviewKind::Text, None),
        "B" | "b" => (ServerLargeValuePreviewKind::Binary, None),
        "V" | "v" => (ServerLargeValuePreviewKind::Vector, Some("vector")),
        "J" | "j" => (ServerLargeValuePreviewKind::Text, Some("json")),
        "K" | "k" => (ServerLargeValuePreviewKind::Text, Some("jsonb")),
        "S" | "s" => (ServerLargeValuePreviewKind::Text, Some("tsvector")),
        "C" | "c" => (ServerLargeValuePreviewKind::Deferred, Some("clob")),
        "N" | "n" => (ServerLargeValuePreviewKind::Deferred, Some("nclob")),
        "L" | "l" => (ServerLargeValuePreviewKind::Deferred, Some("blob")),
        "F" | "f" => (ServerLargeValuePreviewKind::Deferred, Some("bfile")),
        _ => return None,
    };
    Some((source_index, Some(preview_kind), source_type))
}

fn server_large_value_marker(value: &serde_json::Value) -> Option<ServerLargeValueMarkerValue> {
    let mut parts = value.as_str()?.split(':');
    let kind = parts.next()?;
    let preview_size = parts.next()?;
    let kind = match kind {
        "T" => ServerLargeValuePreviewKind::Text,
        "B" => ServerLargeValuePreviewKind::Binary,
        "V" => ServerLargeValuePreviewKind::Vector,
        "D" => ServerLargeValuePreviewKind::Deferred,
        _ => return None,
    };
    let original_bytes = parts.next().and_then(|value| value.parse::<usize>().ok());
    Some(ServerLargeValueMarkerValue { kind, preview_size: preview_size.parse::<usize>().ok()?.max(1), original_bytes })
}

fn truncate_server_large_value_preview(
    value: &mut serde_json::Value,
    kind: ServerLargeValuePreviewKind,
    preview_size: usize,
) -> bool {
    let serde_json::Value::String(text) = value else {
        return false;
    };
    if matches!(kind, ServerLargeValuePreviewKind::Deferred) {
        return true;
    }
    if matches!(kind, ServerLargeValuePreviewKind::Vector) {
        let truncated = text.chars().count() > preview_size;
        let vector_text = if truncated {
            let prefix: String = text.chars().take(preview_size).collect();
            let Some(last_separator) = prefix.rfind(',') else {
                return false;
            };
            format!("{}]", &prefix[..last_separator])
        } else {
            text.clone()
        };
        let Ok(vector) = serde_json::from_str::<Vec<serde_json::Value>>(&vector_text) else {
            return false;
        };
        *value = serde_json::Value::Array(vector);
        return truncated;
    }
    let truncate_at = match kind {
        ServerLargeValuePreviewKind::Text => text.char_indices().nth(preview_size).map(|(index, _)| index),
        ServerLargeValuePreviewKind::Binary => text
            .strip_prefix("0x")
            .filter(|hex| hex.len() > preview_size.saturating_mul(2))
            .map(|_| 2usize.saturating_add(preview_size.saturating_mul(2))),
        ServerLargeValuePreviewKind::Vector | ServerLargeValuePreviewKind::Deferred => unreachable!(),
    };
    let Some(truncate_at) = truncate_at else {
        return false;
    };
    text.truncate(truncate_at);
    text.push_str("...");
    true
}

fn server_large_value_markers(result: &db::QueryResult) -> Vec<ServerLargeValueMarker> {
    let mut markers = Vec::new();
    for (result_index, column) in result.columns.iter().enumerate() {
        let Some((source_index, preview_kind, source_type)) =
            strip_large_value_marker_prefix(column).and_then(server_large_value_alias)
        else {
            continue;
        };
        let expected_source_index = result_index.checked_sub(markers.len() + 1);
        if expected_source_index == Some(source_index) {
            markers.push(ServerLargeValueMarker { result_index, source_index, preview_kind, source_type });
        }
    }
    markers
}

fn extract_server_large_value_markers(result: &mut db::QueryResult) -> Vec<db::LargeValueCell> {
    let markers = server_large_value_markers(result);
    if markers.is_empty() {
        return Vec::new();
    }

    let mut large_value_cells = Vec::new();
    for marker in &markers {
        let source_result_index = marker.result_index.saturating_sub(1);
        let row_kind = result
            .rows
            .iter()
            .find_map(|row| row.get(marker.result_index).and_then(server_large_value_marker).map(|value| value.kind));
        let source_type = marker.source_type.or_else(|| {
            (marker.preview_kind.or(row_kind) == Some(ServerLargeValuePreviewKind::Vector)).then_some("vector")
        });
        if let (Some(source_type), Some(column_type)) = (source_type, result.column_types.get_mut(source_result_index))
        {
            *column_type = source_type.to_string();
        }
    }
    for (row_index, row) in result.rows.iter_mut().enumerate() {
        for marker in &markers {
            let marker_value = row.get(marker.result_index).and_then(server_large_value_marker);
            let source_result_index = marker.result_index.saturating_sub(1);
            if marker_value.is_some_and(|value| {
                row.get_mut(source_result_index)
                    .is_some_and(|source| truncate_server_large_value_preview(source, value.kind, value.preview_size))
            }) {
                large_value_cells.push(db::LargeValueCell {
                    row_index,
                    column_index: marker.source_index,
                    original_bytes: marker_value
                        .and_then(|value| value.original_bytes)
                        .unwrap_or(SERVER_LARGE_VALUE_UNKNOWN_BYTES),
                });
            }
        }
    }

    let removed: std::collections::HashSet<usize> = markers.iter().map(|marker| marker.result_index).collect();
    let retained_index = |index: usize| index - removed.iter().filter(|removed_index| **removed_index < index).count();
    result.spatial_columns.retain_mut(|column| {
        if removed.contains(&column.column_index) {
            return false;
        }
        column.column_index = retained_index(column.column_index);
        true
    });
    for row in &mut result.rows {
        let mut index = 0;
        row.retain(|_| {
            let keep = !removed.contains(&index);
            index += 1;
            keep
        });
    }
    for row in &mut result.spatial_values {
        let mut index = 0;
        row.retain(|_| {
            let keep = !removed.contains(&index);
            index += 1;
            keep
        });
    }
    let mut index = 0;
    result.columns.retain(|_| {
        let keep = !removed.contains(&index);
        index += 1;
        keep
    });
    let mut index = 0;
    result.column_types.retain(|_| {
        let keep = !removed.contains(&index);
        index += 1;
        keep
    });
    let mut index = 0;
    result.column_sortables.retain(|_| {
        let keep = !removed.contains(&index);
        index += 1;
        keep
    });
    large_value_cells
}

fn remap_large_value_cells_around_server_markers(result: &db::QueryResult, cells: &mut Vec<db::LargeValueCell>) {
    let removed: std::collections::HashSet<usize> =
        server_large_value_markers(result).into_iter().map(|marker| marker.result_index).collect();
    if removed.is_empty() {
        return;
    }
    cells.retain_mut(|cell| {
        if removed.contains(&cell.column_index) {
            return false;
        }
        cell.column_index -= removed.iter().filter(|index| **index < cell.column_index).count();
        true
    });
}

fn merge_large_value_cells(
    mut driver_cells: Vec<db::LargeValueCell>,
    server_cells: Vec<db::LargeValueCell>,
) -> Vec<db::LargeValueCell> {
    if driver_cells.is_empty() {
        return server_cells;
    }
    if server_cells.is_empty() {
        return driver_cells;
    }
    let mut driver_indexes = driver_cells
        .iter()
        .enumerate()
        .map(|(index, cell)| ((cell.row_index, cell.column_index), index))
        .collect::<HashMap<_, _>>();
    for server_cell in server_cells {
        let key = (server_cell.row_index, server_cell.column_index);
        if let Some(index) = driver_indexes.get(&key).copied() {
            driver_cells[index] = server_cell;
        } else {
            driver_indexes.insert(key, driver_cells.len());
            driver_cells.push(server_cell);
        }
    }
    driver_cells
}

impl From<db::QueryResult> for ExecuteMultiResult {
    fn from(result: db::QueryResult) -> Self {
        Self {
            result,
            large_value_cells: Vec::new(),
            execution_error: false,
            statement_index: None,
            error: None,
            server_message: false,
            manual_transaction_proven_read_only: false,
            manual_transaction_no_statement: false,
            auto_commit_open_transaction: None,
            auto_commit_explicit_transaction_rolled_back: false,
            auto_commit_session_autocommit_rolled_back: false,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn error_from_query_result(result: &db::QueryResult) -> Option<crate::backend_error::BackendError> {
    result.rows.first()?.first()?.as_str().map(crate::backend_error::BackendError::from_legacy_string)
}

/// Check read-only protection for a connection, blocking write SQL statements.
/// Only clones the connection name when read-only mode is active, avoiding
/// unnecessary allocations otherwise.
/// Uses config_for_pool_key to correctly resolve configs when pool_key includes
/// a database suffix (e.g., "prod:app" → config stored under "prod").
pub async fn check_read_only_for_connection(state: &AppState, pool_key: &str, sql: &str) -> Result<(), String> {
    let connection = {
        let configs = state.configs.read().await;
        crate::connection::config_for_pool_key(pool_key, &configs)
            .filter(|config| config.read_only)
            .map(|config| (config.id.clone(), config.name.clone(), config.db_type))
    };
    if let Some((connection_id, name, database_type)) = connection {
        if !state.write_unlock_windows.is_active(&connection_id).await {
            crate::query_execution_sql::check_read_only(sql, &name, database_type)?;
        }
    }
    Ok(())
}

/// Check read-only protection for a connection across multiple SQL statements.
pub async fn check_read_only_for_connection_multi(
    state: &AppState,
    pool_key: &str,
    statements: &[impl AsRef<str>],
) -> Result<(), String> {
    let connection = {
        let configs = state.configs.read().await;
        crate::connection::config_for_pool_key(pool_key, &configs)
            .filter(|config| config.read_only)
            .map(|config| (config.id.clone(), config.name.clone(), config.db_type))
    };
    if let Some((connection_id, name, database_type)) = connection {
        if !state.write_unlock_windows.is_active(&connection_id).await {
            for sql in statements {
                crate::query_execution_sql::check_read_only(sql.as_ref(), &name, database_type)?;
            }
        }
    }
    Ok(())
}

/// Check whether a connection has read-only mode enabled, returning the connection name if so.
/// This uses connection_id directly (not pool_key), so it is safe to call at command entry points
/// before any pool key is constructed.
pub async fn connection_readonly_name(state: &AppState, connection_id: &str) -> Option<String> {
    let name = {
        let configs = state.configs.read().await;
        configs.get(connection_id).filter(|c| c.read_only).map(|c| c.name.clone())?
    };
    if state.write_unlock_windows.is_active(connection_id).await {
        return None;
    }
    Some(name)
}

async fn connection_is_mongodb(state: &AppState, connection_id: &str) -> bool {
    let configs = state.configs.read().await;
    false
}

async fn connection_database_type(state: &AppState, connection_id: &str) -> Option<DatabaseType> {
    let configs = state.configs.read().await;
    configs.get(connection_id).map(|config| config.db_type)
}

async fn connection_mysql_query_dialect(state: &AppState, connection_id: &str) -> db::mysql::MySqlQueryDialect {
    let configs = state.configs.read().await;
    configs
        .get(connection_id)
        .map(|config| db::mysql::MySqlQueryDialect::for_connection(config.db_type, config.driver_profile.as_deref()))
        .unwrap_or_default()
}

async fn connection_mysql_catalog_dialect(
    state: &AppState,
    connection_id: &str,
) -> Option<db::mysql::MySqlCatalogDialect> {
    let configs = state.configs.read().await;
    configs
        .get(connection_id)
        .and_then(|config| db::mysql::mysql_catalog_dialect(config.db_type, config.driver_profile.as_deref()))
}

async fn connection_mysql_catalog_dialect_for_pool_key(
    state: &AppState,
    pool_key: &str,
) -> Option<db::mysql::MySqlCatalogDialect> {
    let configs = state.configs.read().await;
    crate::connection::config_for_pool_key(pool_key, &configs)
        .and_then(|config| db::mysql::mysql_catalog_dialect(config.db_type, config.driver_profile.as_deref()))
}

async fn connection_database_type_for_pool_key(state: &AppState, pool_key: &str) -> Option<DatabaseType> {
    let configs = state.configs.read().await;
    configs
        .iter()
        .filter(|(connection_id, _)| {
            pool_key.strip_prefix(connection_id.as_str()).is_some_and(|rest| rest.is_empty() || rest.starts_with(':'))
        })
        .max_by_key(|(connection_id, _)| connection_id.len())
        .map(|(_, config)| config.db_type)
}

struct SchemaRelationQualifier<'a> {
    schema_identifier: Ident,
    cte_names: &'a HashSet<String>,
    table_aliases: &'a HashSet<String>,
    parameterized_table_depth: usize,
    changed: bool,
}

impl VisitorMut for SchemaRelationQualifier<'_> {
    type Break = ();

    fn pre_visit_table_factor(&mut self, table_factor: &mut TableFactor) -> ControlFlow<Self::Break> {
        if matches!(table_factor, TableFactor::Table { args: Some(_), .. }) {
            self.parameterized_table_depth += 1;
        }
        ControlFlow::Continue(())
    }

    fn post_visit_table_factor(&mut self, table_factor: &mut TableFactor) -> ControlFlow<Self::Break> {
        if matches!(table_factor, TableFactor::Table { args: Some(_), .. }) {
            self.parameterized_table_depth = self.parameterized_table_depth.saturating_sub(1);
        }
        ControlFlow::Continue(())
    }

    fn post_visit_relation(&mut self, relation: &mut ObjectName) -> ControlFlow<Self::Break> {
        if self.parameterized_table_depth == 0
            && qualify_unqualified_relation_name(relation, &self.schema_identifier, self.cte_names, self.table_aliases)
        {
            self.changed = true;
        }
        ControlFlow::Continue(())
    }
}

/// Qualify a single-part relation with `schema_identifier`, which is already
/// rendered in the dialect's own spelling (quoted where the dialect needs it,
/// unquoted where quoting would break parsing).
fn qualify_unqualified_relation_name(
    name: &mut ObjectName,
    schema_identifier: &Ident,
    cte_names: &HashSet<String>,
    table_aliases: &HashSet<String>,
) -> bool {
    let [ObjectNamePart::Identifier(table)] = name.0.as_slice() else {
        return false;
    };
    let upper_name = table.value.to_ascii_uppercase();
    if cte_names.contains(&upper_name) || table_aliases.contains(&upper_name) {
        return false;
    }
    if table.value.starts_with('@') || table.value.starts_with('#') {
        return false;
    }

    let table = table.clone();
    name.0 = vec![ObjectNamePart::Identifier(schema_identifier.clone()), ObjectNamePart::Identifier(table)];
    true
}

struct TableAliasCollector<'a> {
    names: &'a mut HashSet<String>,
}

impl VisitorMut for TableAliasCollector<'_> {
    type Break = ();

    fn post_visit_table_factor(&mut self, table_factor: &mut TableFactor) -> ControlFlow<Self::Break> {
        if let TableFactor::Table { alias: Some(alias), .. } = table_factor {
            self.names.insert(alias.name.value.to_ascii_uppercase());
        }
        ControlFlow::Continue(())
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueryExecutionMode {
    #[default]
    Standard,
    Simple,
    PostgresReadOnlyTransaction,
}

#[derive(Clone, Debug, Default)]
pub struct QueryExecutionOptions {
    pub max_rows: Option<usize>,
    pub fetch_size: Option<usize>,
    pub page_size: Option<usize>,
    pub row_offset: Option<usize>,
    pub max_result_bytes: Option<usize>,
    /// Result columns that must stay exact because clients use them as stable
    /// row identifiers when fetching full large-cell values on demand.
    pub result_key_columns: Vec<String>,
    /// Enables extraction of hidden server-side preview metadata. This must be
    /// set only for generated table-data preview SQL, never arbitrary queries.
    pub table_data_preview: bool,
    /// Doris / StarRocks catalog selected for this query tab.
    pub catalog: Option<String>,
    pub result_session_id: Option<String>,
    pub client_session_id: Option<String>,
    /// Query timeout in seconds. `None` uses the default (30s).
    /// `Some(0)` disables the timeout entirely.
    pub timeout_secs: Option<u64>,
    /// Keep awaiting the database response after cancellation so callers can
    /// distinguish an interrupt request from a confirmed terminal state.
    pub await_cancel_completion: bool,
    pub execution_id: Option<String>,
    /// When `Some(true)`, multiple statements are executed within a single transaction
    /// (BEGIN … COMMIT) instead of auto-commit mode. `None` and `Some(false)` behave
    /// identically — auto-commit for each statement.
    pub use_transaction: Option<bool>,
    /// When `true`, multi-statement execution continues after a statement error instead
    /// of stopping at the first failure. Connection-level failures always stop the batch.
    pub continue_on_error: bool,
    /// Explicit low-level execution path. `Simple` is currently used by SQL Server
    /// SHOWPLAN so the source SQL bypasses result-set probing and query rewriting.
    /// `PostgresReadOnlyTransaction` executes on an isolated client session and
    /// always rolls the transaction back after the result is collected.
    pub execution_mode: QueryExecutionMode,
    /// MySQL auto-commit tabs only: keep a transaction the user opened
    /// explicitly (`BEGIN` / `START TRANSACTION`, or a batch that disabled
    /// auto-commit) open across executions instead of rolling it back when the
    /// batch finishes. Opt-in per execution (driven by the editor setting);
    /// `false` keeps the historical cleanup that stops a leftover transaction
    /// from pinning the tab's read view (#9479).
    pub preserve_explicit_transaction: bool,
}

fn validate_query_execution_mode(
    db_type: Option<DatabaseType>,
    sql: &str,
    options: &QueryExecutionOptions,
) -> Result<(), String> {
    if options.execution_mode != QueryExecutionMode::PostgresReadOnlyTransaction {
        return Ok(());
    }
    {
        return Err("PostgreSQL read-only transaction mode requires a PostgreSQL connection".to_string());
    }
}

fn query_result_row_limit(max_rows: Option<usize>) -> usize {
    max_rows.unwrap_or(MAX_ROWS).max(1)
}

pub fn truncate_result(result: db::QueryResult) -> db::QueryResult {
    truncate_result_with_max_rows(result, None)
}

pub fn truncate_result_with_max_rows(mut result: db::QueryResult, max_rows: Option<usize>) -> db::QueryResult {
    let row_limit = query_result_row_limit(max_rows);
    if result.rows.len() > row_limit {
        result.rows.truncate(row_limit);
        result.truncated = true;
    }
    result
}

pub fn is_connection_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    if is_dbx_query_timeout_error(&lower) || is_agent_rpc_timeout_error(&lower) || is_pool_saturation_error(err) {
        return false;
    }
    lower.contains("connection")
        || lower.contains("broken pipe")
        || lower.contains("reset by peer")
        || lower.contains("timed out")
        || (lower.contains("pool") && lower.contains("timeout"))
        || lower.contains("closed")
        || lower.contains("关闭的连接")
        || lower.contains("连接已关闭")
        || lower.contains("网络通信异常")
        || lower.contains("通信异常")
        || lower.contains("communications link failure")
        || lower.contains("sqlrecoverableexception")
        || lower.contains("sqlnontransientconnectionexception")
        || lower.contains("sqltransientconnectionexception")
        || lower.contains("eof")
        || lower.contains("i/o error")
        || lower.contains("input/output error")
        || lower.contains("not connected")
        || lower.contains("end-of-file")
        || lower.contains("idle")
        || lower.contains("agent stdin not available")
        || lower.contains("agent stdout not available")
        || lower.contains("agent runtime terminated")
        || lower.contains("agent runtime is unavailable")
        || lower.contains("agent runtime unavailable")
        || lower.contains("failed to write to agent stdin")
        || lower.contains("failed to flush agent stdin")
        || lower.contains("communicating with the server")
        || is_os_connection_error(&lower)
}

pub(crate) fn is_dbx_query_timeout_error(lower: &str) -> bool {
    lower.starts_with("query timed out after ")
}

fn is_agent_rpc_timeout_error(lower: &str) -> bool {
    lower.starts_with("agent rpc call timed out ")
}

fn is_schema_reset_cleanup_error(lower: &str) -> bool {
    lower.contains("schema.reset cleanup failed")
}

pub fn pool_error_action(db_type: Option<DatabaseType>, err: &str) -> PoolErrorAction {
    {}
    let lower = err.to_lowercase();
    if false
        || (is_dbx_query_timeout_error(&lower) && should_discard_pool_after_query_timeout(db_type))
        || is_schema_reset_cleanup_error(&lower)
        || false
    {
        return PoolErrorAction::Discard;
    }

    if is_connection_error(err) {
        PoolErrorAction::ReconnectAndRetry
    } else {
        PoolErrorAction::Keep
    }
}

fn should_continue_batch_after_error(continue_on_error: bool, action: PoolErrorAction) -> bool {
    // A broken connection cannot safely execute the remaining statements even when
    // the user explicitly enabled continue-on-error.
    continue_on_error && action == PoolErrorAction::Keep
}

fn options_for_sequential_statements(
    options: &QueryExecutionOptions,
    statement_count: usize,
    db_type: Option<DatabaseType>,
) -> QueryExecutionOptions {
    let mut statement_options = options.clone();
    {
        return statement_options;
    }
}

pub(crate) fn should_discard_pool_after_query_timeout(db_type: Option<DatabaseType>) -> bool {
    let Some(db_type) = db_type else {
        return false;
    };
    true
}

pub fn should_discard_pool_after_error(db_type: Option<DatabaseType>, err: &str) -> bool {
    matches!(pool_error_action(db_type, err), PoolErrorAction::Discard | PoolErrorAction::ReconnectAndRetry)
}

async fn discard_pool_after_error(state: &AppState, pool_key: &str, db_type: Option<DatabaseType>, error: &str) {
    let action = pool_error_action(db_type, error);
    if !matches!(action, PoolErrorAction::Discard | PoolErrorAction::ReconnectAndRetry) {
        return;
    }

    let replace_agent_runtime = false;
    {
        state.remove_pool_by_key(pool_key).await;
    }
}

fn query_pool_error_action(db_type: Option<DatabaseType>, sql: &str, err: &str) -> PoolErrorAction {
    match pool_error_action(db_type, err) {
        // A connection error does not prove that the database did not receive
        // a write. Only replay statements already accepted by the read-only
        // protection classifier; writes discard the stale pool without retry.
        PoolErrorAction::ReconnectAndRetry if is_write_sql(sql) => PoolErrorAction::Discard,
        action => action,
    }
}

fn query_execution_error_action(
    db_type: Option<DatabaseType>,
    sql: &str,
    error: &QueryExecutionError,
) -> PoolErrorAction {
    {}
    match error {
        QueryExecutionError::Canceled { .. } => PoolErrorAction::Keep,

        QueryExecutionError::Timeout(message)
        | QueryExecutionError::Sql(message)
        | QueryExecutionError::SqlWithPosition { message, .. }
        | QueryExecutionError::Legacy(message) => query_pool_error_action(db_type, sql, message),
    }
}

fn is_os_connection_error(lower: &str) -> bool {
    let os_error_codes = ["10053", "10054", "10057", "10058", "10060", "10061"];
    if let Some(pos) = lower.find("os error ") {
        let after = &lower[pos + 9..];
        let code: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        return os_error_codes.contains(&code.as_str());
    }
    false
}

fn canceled_query_execution_error() -> QueryExecutionError {
    QueryExecutionError::Canceled { stage: AgentErrorStage::Cancel, operation_outcome: AgentOperationOutcome::Unknown }
}

fn pre_dispatch_canceled_query_execution_error() -> QueryExecutionError {
    QueryExecutionError::Canceled {
        stage: AgentErrorStage::Request,
        operation_outcome: AgentOperationOutcome::NotStarted,
    }
}

fn query_pool_database<'a>(database: &'a str, catalog: Option<&str>) -> Option<&'a str> {
    if database.is_empty() || catalog.is_some() {
        None
    } else {
        Some(database)
    }
}

pub async fn operation_budget_for_pool_key(
    state: &AppState,
    pool_key: &str,
    query_timeout: Option<Duration>,
) -> DbOperationBudget {
    let mut budget = configured_operation_budget_for_pool_key(state, pool_key).await;
    budget.query_timeout = query_timeout;
    budget
}

async fn configured_operation_budget_for_pool_key(state: &AppState, pool_key: &str) -> DbOperationBudget {
    let configs = state.configs.read().await;
    crate::connection::config_for_pool_key(pool_key, &configs)
        .map(DbOperationBudget::from_connection_config)
        .unwrap_or_else(DbOperationBudget::with_defaults)
}

/// Override a transaction budget's query timeout from a per-call override (e.g. the MCP
/// global query-timeout policy). `None` leaves the budget unchanged; `Some(secs)` follows
/// `resolve_query_timeout` semantics (`Some(0)` clears the limit, meaning unlimited).
fn apply_query_timeout_override(budget: &mut DbOperationBudget, timeout_secs: Option<u64>) {
    if let Some(secs) = timeout_secs {
        budget.query_timeout = resolve_query_timeout(Some(secs));
    }
}

async fn apply_oceanbase_mysql_session_timeout(
    state: &AppState,
    pool_key: &str,
    conn: &mut mysql_async::Conn,
    timeout_secs: Option<u64>,
) -> Result<(), String> {
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn do_execute_typed(
    state: &AppState,
    pool_key: &str,
    mysql_dialect: db::mysql::MySqlQueryDialect,
    database: Option<&str>,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
    transaction_outcome: &mut Option<MysqlAutoCommitTransaction>,
) -> Result<db::QueryResult, QueryExecutionError> {
    crate::sql_diagnostics::debug_sql("do_execute", sql);
    if let Some(execution_id) = options.execution_id.as_deref() {
        state.running_queries.set_pool_key(execution_id, pool_key.to_string());
    }
    state.touch_pool_activity(pool_key).await;
    let _activity_touch = state.pool_activity_touch(pool_key);

    let query_timeout = resolve_query_timeout(options.timeout_secs);
    // Re-check at statement start so a write that becomes ready after the
    // temporary unlock window expires is still blocked, even if an earlier
    // statement in the same batch is still running.
    check_read_only_for_connection(state, pool_key, sql).await?;
    let operation_budget = operation_budget_for_pool_key(state, pool_key, query_timeout).await;
    let pool_db_type = connection_database_type_for_pool_key(state, pool_key).await;
    let mysql_catalog_dialect = connection_mysql_catalog_dialect_for_pool_key(state, pool_key).await;
    let pool = state.pool_handle(pool_key).await.ok_or("Connection not found")?;
    // Everything the driver dispatch does for this statement — pool checkout,
    // schema/search_path setup, the statement, and the post-statement cleanup —
    // is what the user actually waited for. Report that span instead of the
    // driver-internal timer, which starts only after a client is in hand and
    // therefore hides connection-pool stalls from the summary (#6097 fixed the
    // same mismatch for SQL Server's shared-connection lock).
    let dispatch_start = std::time::Instant::now();

    let result: Result<db::QueryResult, String> = match &pool {
        PoolKind::Mysql(p, mode) => {
            let p = p.clone();
            let bare = false;
            let max_rows = options.max_rows;
            let max_result_bytes = options.max_result_bytes.filter(|value| *value > 0);
            let mut conn = match db::mysql::get_conn_with_health_check_with_cancel(
                &p,
                operation_budget.checkout_timeout,
                operation_budget.cleanup_timeout,
                cancel_token.as_ref(),
            )
            .await
            {
                Ok(conn) => conn,
                Err(err) if err == QUERY_CANCELED => {
                    state.remove_pool_by_key(pool_key).await;
                    return Err(err.into());
                }
                Err(err) => return Err(err.into()),
            };
            let connection_id = conn.id();
            if let Some(ref execution_id) = options.execution_id {
                let kill_opts = conn.opts().clone();
                state.running_queries.register_interrupt(execution_id, move || {
                    let kill_opts = kill_opts.clone();
                    tokio::spawn(async move {
                        if let Err(error) = db::mysql::kill_query_with_opts(kill_opts, connection_id).await {
                            log::warn!("Failed to cancel MySQL query {connection_id}: {error}");
                        }
                    });
                });
            }
            apply_oceanbase_mysql_session_timeout(state, pool_key, &mut conn, options.timeout_secs).await?;
            wait_for_result_opt(
                cancel_token.clone(),
                query_timeout,
                db::mysql::apply_catalog_database_context(
                    &mut conn,
                    mysql_catalog_dialect,
                    options.catalog.as_deref(),
                    database.unwrap_or_default(),
                ),
            )
            .await?;
            let execution_cancel_token = if options.await_cancel_completion { None } else { cancel_token };
            let statement_result = wait_for_result_opt(
                execution_cancel_token,
                query_timeout,
                db::mysql::execute_query_on_conn_with_limits(
                    &mut conn,
                    sql,
                    false,
                    max_rows,
                    max_result_bytes,
                    &options.result_key_columns,
                    mysql_dialect,
                    options.execution_id.as_deref(),
                ),
            )
            .await
            .map(|result| result.result);

            // Client-session pools hold one connection for the whole tab and
            // skip COM_RESET_CONNECTION on return so session state survives
            // across executions. A single-statement BEGIN / START TRANSACTION
            // would therefore leave its transaction open and pin the
            // connection's REPEATABLE READ read view, so every later
            // auto-commit query in the tab would keep reading the same stale
            // snapshot until the connection was closed. Settle it here, exactly
            // like the multi-statement MySQL path does: roll the transaction
            // back, unless the tab keeps explicit user transactions open
            // (`preserve_explicit_transaction`) or a later execution already
            // decided to keep this one.
            if p.is_client_session_pool() {
                // A truncated or failed result, or result sets still pending
                // behind the one that was read, may leave the last status packet
                // short of the end, so only a complete response can prove that
                // the cleanup `ROLLBACK` is unnecessary.
                let status_is_final = *mode == crate::connection::MysqlMode::Normal
                    && statement_result.as_ref().is_ok_and(|result| !result.truncated)
                    && db::mysql::last_ok_ends_response(&conn);
                let transaction = settle_mysql_auto_commit_transaction_boxed(
                    state,
                    pool_key,
                    &mut conn,
                    options.preserve_explicit_transaction,
                    crate::query_execution_sql::mysql_statement_opens_explicit_transaction(sql),
                    status_is_final,
                )
                .await;
                match transaction {
                    Ok(transaction) => *transaction_outcome = Some(transaction),
                    Err(error) => {
                        log::warn!(
                            "[query][mysql] trace_id={} open_txn_rollback_failed error={}",
                            options.execution_id.as_deref().unwrap_or_default(),
                            error
                        );
                        let _ = tokio::time::timeout(Duration::from_secs(5), conn.disconnect()).await;
                        state.remove_pool_by_key(pool_key).await;
                    }
                }
            }
            statement_result
        }
    };
    result
        .map(|mut result| {
            result.execution_time_ms = result.execution_time_ms.max(dispatch_start.elapsed().as_millis());
            result
        })
        .map_err(|error| {
            {}
            // PostgreSQL reports a cursor position as a marker suffix on the
            // driver message. Resolve it here, while the executed statement text
            // is still available, into a typed field. The marker is stripped even
            // when it cannot be resolved, so it can never reach a user-facing
            // message. The PostgreSQL driver also backs Redshift/GaussDB/Kwdb/
            // QuestDB/openGauss, so this is not gated on `DatabaseType::Postgres`.
            if error.contains(crate::sql_error_position::SQL_ERROR_POSITION_MARKER) {
                let (message, position) = crate::sql_error_position::take_message_position(&error, sql);
                return match position {
                    Some(position) => QueryExecutionError::SqlWithPosition { message, position },
                    None => QueryExecutionError::Legacy(message),
                };
            }
            QueryExecutionError::Legacy(error)
        })
        .map_err(|error| classify_query_error(pool_db_type, error))
}

fn classify_query_error(db_type: Option<DatabaseType>, error: QueryExecutionError) -> QueryExecutionError {
    match error {
        QueryExecutionError::Legacy(message) if message == QUERY_CANCELED => canceled_query_execution_error(),
        QueryExecutionError::Legacy(message) if is_dbx_query_timeout_error(&message.to_ascii_lowercase()) => {
            QueryExecutionError::Timeout(message)
        }

        other => other,
    }
}

pub async fn do_execute(
    state: &AppState,
    pool_key: &str,
    mysql_dialect: db::mysql::MySqlQueryDialect,
    database: Option<&str>,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
) -> Result<db::QueryResult, String> {
    do_execute_typed(state, pool_key, mysql_dialect, database, sql, schema, cancel_token, options, &mut None)
        .await
        .map_err(QueryExecutionError::into_legacy_string)
}

fn is_sql_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

fn sql_keyword_at(bytes: &[u8], index: usize, keyword: &[u8]) -> bool {
    let end = index.saturating_add(keyword.len());
    end <= bytes.len()
        && bytes[index..end].eq_ignore_ascii_case(keyword)
        && (index == 0 || !is_sql_word_byte(bytes[index - 1]))
        && (end == bytes.len() || !is_sql_word_byte(bytes[end]))
}

pub async fn execute_sql_statement(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
) -> Result<db::QueryResult, String> {
    execute_sql_statement_with_options(
        state,
        connection_id,
        database,
        sql,
        schema,
        cancel_token,
        QueryExecutionOptions::default(),
    )
    .await
}

pub async fn execute_sql_statement_with_options_typed(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
) -> Result<db::QueryResult, QueryExecutionError> {
    // `_with_outcome` is a second async layer in front of
    // `execute_sql_statement_with_options_typed_inner`. Awaiting it inline nests
    // one more concrete future type into every caller's generator, which pushed
    // Rust targets of this workspace past rustc's query depth limit on Linux
    // (`error: queries overflow the depth limit!`). Erasing the type keeps this
    // wrapper as shallow as it was before the outcome was added.
    let executed: ExecutedSqlStatement<'_> = Box::pin(execute_sql_statement_with_options_typed_with_outcome(
        state,
        connection_id,
        database,
        sql,
        schema,
        cancel_token,
        options,
    ));
    executed.await.map(|(result, _)| result)
}

/// Boxed, type-erased run of one SQL statement, awaiting it without nesting
/// another concrete future type into the caller's generator.
type ExecutedSqlStatement<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = ExecutedSqlStatementOutcome> + Send + 'a>>;

/// Result of [`ExecutedSqlStatement`].
type ExecutedSqlStatementOutcome = Result<(db::QueryResult, Option<MysqlAutoCommitTransaction>), QueryExecutionError>;

/// Same as [`execute_sql_statement_with_options_typed`], but also returns how
/// the MySQL auto-commit settlement left the tab-scoped connection (see
/// [`MysqlAutoCommitTransaction`]). `None` means the statement did not run on a
/// tab-scoped MySQL connection, so there is no such state to report.
pub(crate) async fn execute_sql_statement_with_options_typed_with_outcome(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
) -> Result<(db::QueryResult, Option<MysqlAutoCommitTransaction>), QueryExecutionError> {
    let db_type = connection_database_type(state, connection_id).await;
    let invalidate = crate::object_cache::sql_may_change_object_metadata(sql, db_type);
    let mut transaction = None;
    // Same contract as the multi-statement entry point: a single statement is
    // reported with the duration of the whole request, including creating or
    // reconnecting the pool, so the summary matches the loading indicator.
    let request_start = std::time::Instant::now();
    let result = execute_sql_statement_with_options_typed_inner(
        state,
        connection_id,
        database,
        sql,
        schema,
        cancel_token,
        options,
        &mut transaction,
    )
    .await;
    if invalidate {
        crate::object_cache::invalidate_connection_object_cache(&state.storage, connection_id).await;
    }
    result.map(|mut result| {
        result.execution_time_ms = result.execution_time_ms.max(request_start.elapsed().as_millis());
        (result, transaction)
    })
}

async fn execute_sql_statement_with_options_typed_inner(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
    transaction_outcome: &mut Option<MysqlAutoCommitTransaction>,
) -> Result<db::QueryResult, QueryExecutionError> {
    // MongoDB connections use shell-style commands dispatched through the
    // frontend parser. Queries that fall through to the generic SQL executor
    // (e.g. typos) must be rejected before any pool/key creation so that
    // session-scoped pools do not leak MongoDB Clients and SSH tunnels.
    if connection_is_mongodb(state, connection_id).await {
        return Err(MONGO_SHELL_COMMAND_HINT.into());
    }

    let db_type = connection_database_type(state, connection_id).await;
    validate_query_execution_mode(db_type, sql, &options)?;
    let has_executable_sql = db_type.map_or_else(
        || crate::sql::has_executable_sql(sql),
        |db_type| crate::sql::has_executable_sql_for_database(sql, db_type),
    );
    if !has_executable_sql && options.result_session_id.is_none() {
        return Ok(empty_query_result(0));
    }

    {}

    // When a query tab has a client session, keep even database-less execution
    // on that tab-scoped pool so connection-level state (for example MySQL @vars)
    // survives across runs.
    let pool_database = query_pool_database(database, options.catalog.as_deref());
    let pool_key = state
        .get_or_create_pool_for_session(connection_id, pool_database, options.client_session_id.as_deref())
        .await
        .map_err(|e| query_error_with_omitted_sql_context(&e, sql))?;

    if is_canceled(&cancel_token) {
        return Err(pre_dispatch_canceled_query_execution_error());
    }

    let mysql_dialect = connection_mysql_query_dialect(state, connection_id).await;
    let result = do_execute_typed(
        state,
        &pool_key,
        mysql_dialect,
        Some(database),
        sql,
        schema,
        cancel_token.clone(),
        options.clone(),
        transaction_outcome,
    )
    .await;

    let with_sql_context = |result: Result<db::QueryResult, QueryExecutionError>| {
        result.map_err(|error| error.with_omitted_sql_context(sql))
    };

    let action = result.as_ref().err().map(|error| query_execution_error_action(db_type, sql, error));
    if let Some(initial_error) = result.as_ref().err() {
        {}
    }
    match action {
        Some(PoolErrorAction::ReconnectAndRetry) if !is_canceled(&cancel_token) => {
            let pool_database = query_pool_database(database, options.catalog.as_deref());
            let new_key = state
                .reconnect_pool_for_session(connection_id, pool_database, options.client_session_id.as_deref())
                .await
                .map_err(|e| query_error_with_omitted_sql_context(&e, sql))?;
            with_sql_context(
                do_execute_typed(
                    state,
                    &new_key,
                    mysql_dialect,
                    Some(database),
                    sql,
                    schema,
                    cancel_token,
                    options,
                    &mut None,
                )
                .await,
            )
        }
        Some(PoolErrorAction::Discard) => {
            // Agent execution owns structured quarantine/runtime replacement before
            // returning. Native drivers retain the existing caller-side cleanup.
            {
                state.remove_pool_by_key(&pool_key).await;
            }
            with_sql_context(result)
        }
        _ => with_sql_context(result),
    }
}

pub async fn execute_sql_statement_with_options(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
) -> Result<db::QueryResult, String> {
    execute_sql_statement_with_options_typed(state, connection_id, database, sql, schema, cancel_token, options)
        .await
        .map_err(QueryExecutionError::into_legacy_string)
}

fn parse_drop_database_target(sql: &str) -> Option<String> {
    let dialect = PostgreSqlDialect {};
    let statements = Parser::parse_sql(&dialect, sql).ok()?;
    let [Statement::Drop { object_type, names, .. }] = statements.as_slice() else {
        return None;
    };
    if *object_type != ObjectType::Database || names.len() != 1 {
        return None;
    }

    let parts = &names[0].0;
    if parts.len() != 1 {
        return None;
    }
    parts[0].as_ident().map(|ident| ident.value.clone())
}

pub async fn close_query_session(
    state: &AppState,
    connection_id: &str,
    database: &str,
    session_id: &str,
    client_session_id: Option<&str>,
    catalog: Option<&str>,
) -> Result<bool, String> {
    let pool_database = query_pool_database(database, catalog);
    let pool_key = state.get_or_create_pool_for_session(connection_id, pool_database, client_session_id).await?;

    let pool_handle = state.pool_handle(&pool_key).await;
    let pool = pool_handle.as_ref().ok_or("Connection not found")?;
    match pool {
        _ => Ok(false),
    }
}

pub async fn execute_multi_core(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
) -> Result<Vec<db::QueryResult>, String> {
    execute_multi_core_with_options(
        state,
        connection_id,
        database,
        sql,
        schema,
        cancel_token,
        QueryExecutionOptions::default(),
    )
    .await
}

pub async fn execute_multi_core_with_options(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
) -> Result<Vec<db::QueryResult>, String> {
    execute_multi_core_with_options_for_client(state, connection_id, database, sql, schema, cancel_token, options)
        .await
        .map(|results| results.into_iter().map(ExecuteMultiResult::into_query_result).collect())
}

/// Execute a SQL batch and retain client-facing metadata for synthesized errors.
pub async fn execute_multi_core_with_options_for_client(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
) -> Result<Vec<ExecuteMultiResult>, String> {
    execute_multi_core_with_options_for_client_typed(state, connection_id, database, sql, schema, cancel_token, options)
        .await
        .map_err(QueryExecutionError::into_legacy_string)
}

/// Execute a SQL batch for a client while preserving typed failures.
pub async fn execute_multi_core_with_options_for_client_typed(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
) -> Result<Vec<ExecuteMultiResult>, QueryExecutionError> {
    execute_multi_core_with_options_for_client_and_progress_typed(
        state,
        connection_id,
        database,
        sql,
        schema,
        cancel_token,
        options,
        None,
    )
    .await
}

/// Executes a SQL batch and reports each completed statement to the optional callback.
pub async fn execute_multi_core_with_options_for_client_and_progress(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
    progress: Option<ExecuteMultiProgressCallback>,
) -> Result<Vec<ExecuteMultiResult>, String> {
    execute_multi_core_with_options_for_client_and_progress_typed(
        state,
        connection_id,
        database,
        sql,
        schema,
        cancel_token,
        options,
        progress,
    )
    .await
    .map_err(QueryExecutionError::into_legacy_string)
}

/// Executes a SQL batch without erasing structured query errors at transport boundaries.
pub async fn execute_multi_core_with_options_for_client_and_progress_typed(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
    progress: Option<ExecuteMultiProgressCallback>,
) -> Result<Vec<ExecuteMultiResult>, QueryExecutionError> {
    let db_type = connection_database_type(state, connection_id).await;
    let invalidate = crate::object_cache::sql_may_change_object_metadata(sql, db_type);
    // A run that produced a single statement result is reported back as one
    // duration, so make it the duration of the whole request: pool creation or
    // reconnection for a cold pool happens outside the statement dispatcher and
    // would otherwise be invisible while the loading indicator counts it.
    let request_start = std::time::Instant::now();
    let result = execute_multi_core_with_options_for_client_and_progress_typed_inner(
        state,
        connection_id,
        database,
        sql,
        schema,
        cancel_token,
        options,
        progress,
    )
    .await;
    if invalidate {
        crate::object_cache::invalidate_connection_object_cache(&state.storage, connection_id).await;
    }
    result.map(|mut results| {
        if results.len() == 1 {
            if let Some(item) = results.first_mut() {
                item.result.execution_time_ms = item.result.execution_time_ms.max(request_start.elapsed().as_millis());
            }
        }
        results
    })
}

async fn execute_multi_core_with_options_for_client_and_progress_typed_inner(
    state: &AppState,
    connection_id: &str,
    database: &str,
    sql: &str,
    schema: Option<&str>,
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
    progress: Option<ExecuteMultiProgressCallback>,
) -> Result<Vec<ExecuteMultiResult>, QueryExecutionError> {
    let pool_database = query_pool_database(database, options.catalog.as_deref());
    // Reject MongoDB queries that fall through to the generic executor.
    if connection_is_mongodb(state, connection_id).await {
        return Err(MONGO_SHELL_COMMAND_HINT.into());
    }

    let db_type = connection_database_type(state, connection_id).await;
    validate_query_execution_mode(db_type, sql, &options)?;
    if options.execution_mode == QueryExecutionMode::PostgresReadOnlyTransaction {
        let result =
            execute_sql_statement_with_options(state, connection_id, database, sql, schema, cancel_token, options)
                .await?;
        return Ok(vec![result.into()]);
    }

    let pool_key = state
        .get_or_create_pool_for_session(connection_id, pool_database, options.client_session_id.as_deref())
        .await
        .map_err(|e| query_error_with_omitted_sql_context(&e, sql))?;
    if let Some(execution_id) = options.execution_id.as_deref() {
        state.running_queries.set_pool_key(execution_id, pool_key.clone());
    }
    state.touch_pool_activity(&pool_key).await;
    let _activity_touch = state.pool_activity_touch(pool_key.as_str());

    let execution_plan = query_execution_plan_with_compatibility(sql, db_type, false, None);
    let continue_on_error = options.continue_on_error && !execution_plan.stop_on_error;
    let statements = execution_plan.statements;
    if statements.is_empty() {
        return Ok(vec![empty_query_result(0).into()]);
    }

    // Check the transaction request before database-specific fast paths. Otherwise
    // a backend such as SQL Server or HTTP SQLite can return successful
    // auto-commit results while the API has promised a rollbackable batch.
    // The DDL cap applies only to this opt-in use_transaction contract: a script
    // that the user asked to run atomically must not silently produce partial
    // effects. Other callers that share the transaction kernel (schema-diff
    // deploy, imports) document a mixed-outcome-on-failure behaviour instead, so
    // they are deliberately not capped here.
    if options.use_transaction == Some(true) && statements.len() > 1 {
        if batch_transaction_ddl_is_unrollbackable(db_type, &statements) {
            return Err(
                "use_transaction cannot be used with a batch whose DDL cannot be rolled back: DDL statements implicitly commit and cannot be undone. Run the batch without use_transaction (auto-commit, one result per statement) or split the DDL and DML into separate calls."
                    .to_string()
                    .into(),
            );
        }
        let result = execute_statements_in_transaction_typed(
            state,
            connection_id,
            database,
            &statements,
            schema,
            options.catalog.as_deref(),
            options.timeout_secs,
        )
        .await?;
        return Ok(vec![result.into()]);
    }

    {}

    let is_http_sqlite = {
        let configs = state.configs.read().await;
        false
    };

    // HTTP SQLite providers send all statements in one request so the provider
    // can preserve batch ordering and atomicity in the default batch mode.
    {}

    let mysql_pool = {
        let pool_handle = state.pool_handle(&pool_key).await;
        match pool_handle.as_ref() {
            Some(PoolKind::Mysql(pool, mode)) => Some((pool.clone(), *mode)),
            _ => None,
        }
    };

    if statements.len() == 1
        && !mysql_single_statement_uses_batch_route(
            db_type,
            mysql_pool.is_some(),
            &statements[0],
            options.max_result_bytes,
        )
    {
        let single_sql = statements.into_iter().next().unwrap_or_default();
        let table_data_preview = options.table_data_preview;
        let executed = execute_sql_statement_with_options_typed_with_outcome(
            state,
            connection_id,
            database,
            &single_sql,
            schema,
            cancel_token,
            options,
        )
        .await;
        let (result, transaction) = match executed {
            Ok((result, transaction)) => (Ok(result), transaction),
            Err(error) => (Err(error), None),
        };
        return single_statement_multi_result(result, table_data_preview, transaction);
    }

    if let Some((pool, mode)) = mysql_pool {
        // Read-only check for MySQL batch path
        check_read_only_for_connection_multi(state, &pool_key, &statements).await?;
        let mysql_dialect = connection_mysql_query_dialect(state, connection_id).await;
        let mysql_catalog_dialect = connection_mysql_catalog_dialect(state, connection_id).await;
        return execute_multi_mysql(
            state,
            &pool_key,
            db_type,
            &pool,
            mode,
            mysql_dialect,
            mysql_catalog_dialect,
            database,
            &statements,
            cancel_token,
            options,
            progress.as_ref(),
        )
        .await
        .map_err(QueryExecutionError::from);
    }

    // Some Agent drivers cannot execute another statement while a paged result
    // cursor remains open on the same physical connection. Multi-result execution
    // therefore reads a bounded first page without retaining those cursors.
    let statement_options = options_for_sequential_statements(&options, statements.len(), db_type);
    let mut results = Vec::with_capacity(statements.len());
    for (statement_index, stmt) in statements.iter().enumerate() {
        if is_canceled(&cancel_token) {
            let error = canceled_query_execution_error();
            results.push(ExecuteMultiResult::execution_error_with_backend(
                error_query_result(error.clone().into_legacy_string()),
                Some(statement_index),
                error.into_backend_error(),
            ));
            break;
        }
        match execute_sql_statement_with_options_typed(
            state,
            connection_id,
            database,
            stmt,
            schema,
            cancel_token.clone(),
            statement_options.clone(),
        )
        .await
        {
            Ok(r) => {
                report_execute_multi_progress(progress.as_ref(), statement_index, statements.len(), &r, true, None);
                results.push(ExecuteMultiResult::success_with_index_and_optional_server_large_values(
                    r,
                    statement_index,
                    options.table_data_preview,
                ));
            }
            Err(error) => {
                let action = query_execution_error_action(db_type, stmt, &error);
                let result = error_query_result(error.clone().into_legacy_string());
                let backend_error = error.into_backend_error();
                report_execute_multi_progress(
                    progress.as_ref(),
                    statement_index,
                    statements.len(),
                    &result,
                    false,
                    Some(backend_error.clone()),
                );
                results.push(ExecuteMultiResult::execution_error_with_backend(
                    result,
                    Some(statement_index),
                    backend_error,
                ));
                if !should_continue_batch_after_error(continue_on_error, action) {
                    break;
                }
            }
        }
    }

    Ok(results)
}

/// Split a batch script into the statements the core will execute, using the
/// same dialect-aware splitter for every caller. SQL Server agent pools split on
/// `GO` batch separators (`split_sql_batches`); everything else uses the
/// database-dialect statement splitter. Public so the MCP pre-check can align its
/// transaction-entry decision with the core's actual script split.
pub fn query_execution_plan(
    sql: &str,
    db_type: Option<DatabaseType>,
    preserve_sqlserver_batches: bool,
) -> crate::sql::SqlExecutionPlan {
    query_execution_plan_with_compatibility(sql, db_type, preserve_sqlserver_batches, None)
}

/// Same as [`query_execution_plan`], but lets openGauss callers pass the
/// database compatibility mode so A-mode PL/SQL (package) bodies are never split
/// on inner semicolons. `None` keeps the conservative PL/SQL-capable openGauss
/// profile while the probe is cold or unavailable.
pub fn query_execution_plan_with_compatibility(
    sql: &str,
    db_type: Option<DatabaseType>,
    preserve_sqlserver_batches: bool,
    compatibility_mode: Option<&str>,
) -> crate::sql::SqlExecutionPlan {
    {}

    db_type.map_or_else(
        || crate::sql::SqlExecutionPlan { statements: split_sql_statements(sql), stop_on_error: false },
        |db_type| crate::sql::sql_execution_plan_for_database_with_compatibility(sql, db_type, compatibility_mode),
    )
}

fn single_statement_multi_result(
    result: Result<db::QueryResult, QueryExecutionError>,
    table_data_preview: bool,
    transaction: Option<MysqlAutoCommitTransaction>,
) -> Result<Vec<ExecuteMultiResult>, QueryExecutionError> {
    result.map(|result| {
        let mut result = ExecuteMultiResult::success_with_optional_server_large_values(result, table_data_preview);
        if let Some(transaction) = transaction {
            transaction.mark(&mut result);
        }
        vec![result]
    })
}

fn mysql_single_statement_uses_batch_route(
    db_type: Option<DatabaseType>,
    has_mysql_pool: bool,
    sql: &str,
    max_result_bytes: Option<usize>,
) -> bool {
    has_mysql_pool
        && (max_result_bytes.is_some_and(|value| value > 0)
            || (db_type == Some(DatabaseType::Mysql)
                && starts_with_executable_sql_keyword_for_database(sql, &["CALL"], DatabaseType::Mysql)))
}

trait MysqlBatchStatementExecutor {
    fn table_data_preview(&self) -> bool {
        false
    }

    async fn execute_statement(&mut self, statement: &str) -> Result<Vec<db::mysql::MySqlQueryResult>, String>;

    async fn execute_non_result_batch(
        &mut self,
        statements: &[String],
        on_result: &mut (dyn FnMut(usize, &db::QueryResult) + Send),
    ) -> db::mysql::MySqlNonResultBatchOutcome {
        let mut results = Vec::with_capacity(statements.len());
        for (statement_index, statement) in statements.iter().enumerate() {
            match self.execute_statement(statement).await {
                Ok(statement_results) if statement_results.len() == 1 => {
                    let result = statement_results.into_iter().next().expect("single MySQL batch result").result;
                    on_result(statement_index, &result);
                    results.push(result);
                }
                Ok(_) => {
                    return db::mysql::MySqlNonResultBatchOutcome {
                        results,
                        error: Some("A non-result MySQL batch statement returned multiple results.".to_string()),
                    };
                }
                Err(error) => return db::mysql::MySqlNonResultBatchOutcome { results, error: Some(error) },
            }
        }
        db::mysql::MySqlNonResultBatchOutcome { results, error: None }
    }
}

struct MysqlBatchConnection<'a> {
    conn: &'a mut mysql_async::Conn,
    cancel_token: Option<CancellationToken>,
    query_timeout: Option<Duration>,
    bare: bool,
    max_rows: Option<usize>,
    max_result_bytes: Option<usize>,
    result_key_columns: &'a [String],
    table_data_preview: bool,
    dialect: db::mysql::MySqlQueryDialect,
    diagnostic_trace_id: Option<&'a str>,
}

impl MysqlBatchStatementExecutor for MysqlBatchConnection<'_> {
    fn table_data_preview(&self) -> bool {
        self.table_data_preview
    }

    async fn execute_statement(&mut self, statement: &str) -> Result<Vec<db::mysql::MySqlQueryResult>, String> {
        wait_for_result_opt(
            self.cancel_token.clone(),
            self.query_timeout,
            db::mysql::execute_query_results_on_conn_with_limits(
                &mut *self.conn,
                statement,
                self.bare,
                self.max_rows,
                self.max_result_bytes,
                self.result_key_columns,
                self.dialect,
                self.diagnostic_trace_id,
            ),
        )
        .await
    }

    async fn execute_non_result_batch(
        &mut self,
        statements: &[String],
        on_result: &mut (dyn FnMut(usize, &db::QueryResult) + Send),
    ) -> db::mysql::MySqlNonResultBatchOutcome {
        let sql = statements.join(";\n");
        let mut completed_results = Vec::with_capacity(statements.len());
        let mut record_result = |statement_index: usize, result: &db::QueryResult| {
            completed_results.push(result.clone());
            on_result(statement_index, result);
        };
        match wait_for_result_opt(
            self.cancel_token.clone(),
            self.query_timeout,
            db::mysql::execute_non_result_batch_on_conn(&mut *self.conn, &sql, statements.len(), &mut record_result),
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => db::mysql::MySqlNonResultBatchOutcome { results: completed_results, error: Some(error) },
        }
    }
}

const MYSQL_MULTI_STATEMENT_BATCH_MAX_STATEMENTS: usize = 50;
const MYSQL_MULTI_STATEMENT_BATCH_MAX_BYTES: usize = 4 * 1024 * 1024;

fn mysql_batch_pool_error_action(db_type: Option<DatabaseType>, error: &str) -> PoolErrorAction {
    if error == QUERY_CANCELED {
        // Dropping an in-flight COM_QUERY future can leave unread packets on the
        // connection. Do not return that connection to the pool.
        PoolErrorAction::Discard
    } else {
        pool_error_action(db_type, error)
    }
}

fn mysql_batch_backend_error(action: PoolErrorAction, error: &str) -> crate::backend_error::BackendError {
    if action == PoolErrorAction::Keep {
        crate::backend_error::BackendError::from_sql_detail(error)
    } else {
        crate::backend_error::BackendError::from_legacy_backend(error)
    }
}

fn mysql_non_result_batch_end(
    statements: &[String],
    start: usize,
    dialect: db::mysql::MySqlQueryDialect,
    max_bytes: usize,
) -> usize {
    let Some(first) = statements.get(start) else {
        return start;
    };
    if !db::mysql::is_batchable_non_result_query(first, dialect) {
        return start + 1;
    }

    let mut end = start;
    let mut byte_len = 0usize;
    while let Some(statement) = statements.get(end) {
        if end - start >= MYSQL_MULTI_STATEMENT_BATCH_MAX_STATEMENTS
            || !db::mysql::is_batchable_non_result_query(statement, dialect)
        {
            break;
        }
        let next_len = byte_len.saturating_add(statement.len()).saturating_add(2);
        if end > start && next_len > max_bytes {
            break;
        }
        byte_len = next_len;
        end += 1;
    }
    end.max(start + 1)
}

fn mysql_non_result_pipeline_enabled(
    statement_count: usize,
    continue_on_error: bool,
    mode: crate::connection::MysqlMode,
) -> bool {
    statement_count > 1 && !continue_on_error && mode == crate::connection::MysqlMode::Normal
}

async fn execute_mysql_batch_statements<E>(
    executor: &mut E,
    statements: &[String],
    db_type: Option<DatabaseType>,
    mysql_dialect: db::mysql::MySqlQueryDialect,
    cancel_token: Option<CancellationToken>,
    continue_on_error: bool,
    pipeline_non_result_max_bytes: Option<usize>,
    progress: Option<&ExecuteMultiProgressCallback>,
) -> (Vec<ExecuteMultiResult>, Option<PoolErrorAction>)
where
    E: MysqlBatchStatementExecutor,
{
    let mut results = Vec::with_capacity(statements.len());
    let mut statement_index = 0usize;
    let table_data_preview = executor.table_data_preview();
    while statement_index < statements.len() {
        if is_canceled(&cancel_token) {
            results.push(ExecuteMultiResult::execution_error(error_query_result(canceled_error())));
            return (results, None);
        }

        let batch_end = if let Some(max_bytes) = pipeline_non_result_max_bytes {
            mysql_non_result_batch_end(statements, statement_index, mysql_dialect, max_bytes)
        } else {
            statement_index + 1
        };
        if batch_end > statement_index + 1 {
            let mut report_result = |offset: usize, result: &db::QueryResult| {
                report_execute_multi_progress(progress, statement_index + offset, statements.len(), result, true, None);
            };
            let outcome =
                executor.execute_non_result_batch(&statements[statement_index..batch_end], &mut report_result).await;
            let completed = outcome.results.len();
            for (offset, result) in outcome.results.into_iter().enumerate() {
                results.push(ExecuteMultiResult::success_with_index(result, statement_index + offset));
            }
            if let Some(error) = outcome.error {
                let action = mysql_batch_pool_error_action(db_type, &error);
                if completed >= batch_end - statement_index {
                    return (results, Some(action));
                }
                let failed_index = statement_index + completed;
                let result = error_query_result(error.clone());
                let backend_error = mysql_batch_backend_error(action, &error);
                report_execute_multi_progress(
                    progress,
                    failed_index,
                    statements.len(),
                    &result,
                    false,
                    Some(backend_error.clone()),
                );
                results.push(ExecuteMultiResult::execution_error_with_backend(
                    result,
                    Some(failed_index),
                    backend_error,
                ));
                return (results, Some(action));
            }
            statement_index = batch_end;
            continue;
        }

        let statement = &statements[statement_index];

        match executor.execute_statement(statement).await {
            Ok(statement_results) => {
                if let Some(result) = statement_results.last() {
                    report_execute_multi_progress(
                        progress,
                        statement_index,
                        statements.len(),
                        &result.result,
                        true,
                        None,
                    );
                }
                results.extend(statement_results.into_iter().map(|result| {
                    ExecuteMultiResult::success_with_index_and_large_values(
                        result.result,
                        statement_index,
                        result.large_value_cells,
                        table_data_preview,
                    )
                }));
            }
            Err(err) => {
                let action = mysql_batch_pool_error_action(db_type, &err);
                let result = error_query_result(err.clone());
                let backend_error = mysql_batch_backend_error(action, &err);
                report_execute_multi_progress(
                    progress,
                    statement_index,
                    statements.len(),
                    &result,
                    false,
                    Some(backend_error.clone()),
                );
                results.push(ExecuteMultiResult::execution_error_with_backend(
                    result,
                    Some(statement_index),
                    backend_error,
                ));
                // Statement errors are safe to collect, but connection-level failures leave
                // the protocol state unusable and must still trigger pool cleanup.
                if !should_continue_batch_after_error(continue_on_error, action) {
                    return (results, Some(action));
                }
            }
        }
        statement_index += 1;
    }

    (results, None)
}

/// Settled transaction state of a tab-scoped MySQL connection after one
/// execution finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MysqlAutoCommitTransaction {
    /// No transaction is open on the connection.
    None,
    /// A transaction the user opened explicitly is still open and was kept.
    Preserved,
    /// DBX rolled back a transaction the user opened explicitly.
    RolledBackExplicit,
    /// DBX rolled back a transaction the session opened implicitly because
    /// auto-commit was turned off (`SET autocommit = 0`).
    RolledBackSessionAutocommit,
}

impl MysqlAutoCommitTransaction {
    fn mark(self, result: &mut ExecuteMultiResult) {
        result.auto_commit_open_transaction = Some(self == Self::Preserved);
        result.auto_commit_explicit_transaction_rolled_back = self == Self::RolledBackExplicit;
        result.auto_commit_session_autocommit_rolled_back = self == Self::RolledBackSessionAutocommit;
    }
}

/// What the settlement did with the transaction that was still open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MysqlAutoCommitRollback {
    /// Nothing was rolled back.
    None,
    /// A transaction the batch opened explicitly (`BEGIN` / `START TRANSACTION`).
    Explicit,
    /// A transaction the session opened implicitly: auto-commit was off
    /// (`SET autocommit = 0`), so every statement starts one.
    SessionAutocommit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MysqlAutoCommitDecision {
    preserve: bool,
    rollback: MysqlAutoCommitRollback,
}

/// Decides what to do with a transaction that is still open when an
/// auto-commit tab finished executing.
///
/// A `BEGIN` / `START TRANSACTION` the user typed (or a batch that turned
/// auto-commit off) is an explicit request to hold the changes until `COMMIT` /
/// `ROLLBACK`, so `preserve_explicit_transaction` keeps it — including the
/// later executions that never mention a transaction, which is what
/// `already_preserved` carries. Everything else keeps the historical cleanup
/// that stops a leftover transaction from pinning the tab's read view (#9479),
/// and is reported so the cleanup is never silent.
fn decide_mysql_auto_commit_transaction(
    allow_preserve: bool,
    already_preserved: bool,
    explicit_start_in_batch: bool,
    status: Option<db::mysql::MySqlSessionStatus>,
) -> MysqlAutoCommitDecision {
    // A session or batch that turned auto-commit off opens its transactions
    // implicitly, so the server status is the only signal for those.
    let explicit_transaction = explicit_start_in_batch || status.is_some_and(|status| !status.autocommit);
    let transaction_open = match status {
        Some(status) => status.in_transaction,
        // No usable status packet (the last statement ended with an ERR packet
        // and the refresh failed): an explicit opener is the only evidence left.
        None => explicit_start_in_batch || already_preserved,
    };
    if !transaction_open {
        return MysqlAutoCommitDecision { preserve: false, rollback: MysqlAutoCommitRollback::None };
    }
    if already_preserved || (allow_preserve && explicit_transaction) {
        return MysqlAutoCommitDecision { preserve: true, rollback: MysqlAutoCommitRollback::None };
    }
    // Distinguish who opened it: only a batch with its own `BEGIN` /
    // `START TRANSACTION` is a user transaction; `autocommit = 0` opens
    // transactions implicitly for every statement.
    let rollback = if explicit_start_in_batch {
        MysqlAutoCommitRollback::Explicit
    } else if status.is_some_and(|status| !status.autocommit) {
        MysqlAutoCommitRollback::SessionAutocommit
    } else {
        MysqlAutoCommitRollback::None
    };
    MysqlAutoCommitDecision { preserve: false, rollback }
}

/// Whether the cleanup `ROLLBACK` after an execution would be a server no-op.
///
/// Every final status packet carries `SERVER_STATUS_IN_TRANS`, and MariaDB
/// Connector/J skips `ROLLBACK`/`COMMIT` on the same flag. DBX only relies on it
/// when the packet provably closes the execution (`status_is_final`: a native
/// MySQL-mode connection whose response was read to the end without an error,
/// so neither an earlier command's packet nor a pending result set is left
/// behind) and the batch did not open a transaction itself. A batch with
/// `BEGIN` keeps the historical cleanup, so a server or proxy that never
/// reports the flag cannot leave the user's transaction open.
fn mysql_cleanup_rollback_is_noop(
    status_is_final: bool,
    explicit_start_in_batch: bool,
    status: Option<db::mysql::MySqlSessionStatus>,
) -> bool {
    status_is_final
        && !explicit_start_in_batch
        && status.is_some_and(|status| !status.in_transaction && status.autocommit)
}

/// Type-erased entry point for [`settle_mysql_auto_commit_transaction`].
///
/// `do_execute_typed` is one of the largest async fns in the crate and is
/// reached transitively by the workspace's live tests and examples. Awaiting the
/// settle future inline nests its concrete future type inside that generator,
/// which pushed `dbx-core` targets past rustc's query depth limit on Linux
/// (`error: queries overflow the depth limit!`). Erasing the type keeps the
/// enclosing generators as shallow as they were before the settlement moved in.
fn settle_mysql_auto_commit_transaction_boxed<'a>(
    state: &'a AppState,
    pool_key: &'a str,
    conn: &'a mut mysql_async::Conn,
    allow_preserve: bool,
    explicit_start_in_batch: bool,
    status_is_final: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<MysqlAutoCommitTransaction, String>> + Send + 'a>> {
    Box::pin(settle_mysql_auto_commit_transaction(
        state,
        pool_key,
        conn,
        allow_preserve,
        explicit_start_in_batch,
        status_is_final,
    ))
}

/// Settles the open transaction of a tab-scoped MySQL connection after an
/// execution: keeps an explicit user transaction when the caller opted in,
/// otherwise rolls it back exactly as before.
///
/// `Err` means the transaction state could not be settled, so the caller must
/// discard the connection instead of returning it to the pool.
async fn settle_mysql_auto_commit_transaction(
    state: &AppState,
    pool_key: &str,
    conn: &mut mysql_async::Conn,
    allow_preserve: bool,
    explicit_start_in_batch: bool,
    status_is_final: bool,
) -> Result<MysqlAutoCommitTransaction, String> {
    let already_preserved = state.has_preserved_explicit_transaction(pool_key).await;
    let mut status = db::mysql::session_status_from_last_ok(conn);
    if status.is_none() && (allow_preserve || already_preserved) {
        // Only pay for the extra round trip when the answer can change the
        // outcome: without the opt-in the historical cleanup runs regardless.
        status = db::mysql::ping_session_status_on_conn(conn).await.ok();
    }
    let decision =
        decide_mysql_auto_commit_transaction(allow_preserve, already_preserved, explicit_start_in_batch, status);
    if decision.preserve {
        state.mark_preserved_explicit_transaction(pool_key).await;
        return Ok(MysqlAutoCommitTransaction::Preserved);
    }
    state.clear_preserved_explicit_transaction(pool_key).await;
    // Historical cleanup: `ROLLBACK` releases the read view of a transaction
    // that is still open. Skip the round trip only when the final status
    // packet proves nothing is open.
    if !mysql_cleanup_rollback_is_noop(status_is_final, explicit_start_in_batch, status) {
        db::mysql::rollback_open_transaction(conn).await?;
    }
    Ok(match decision.rollback {
        MysqlAutoCommitRollback::Explicit => MysqlAutoCommitTransaction::RolledBackExplicit,
        MysqlAutoCommitRollback::SessionAutocommit => MysqlAutoCommitTransaction::RolledBackSessionAutocommit,
        MysqlAutoCommitRollback::None => MysqlAutoCommitTransaction::None,
    })
}

#[allow(clippy::too_many_arguments)]
async fn execute_multi_mysql(
    state: &AppState,
    pool_key: &str,
    db_type: Option<DatabaseType>,
    pool: &db::mysql::MySqlPool,
    mode: crate::connection::MysqlMode,
    dialect: db::mysql::MySqlQueryDialect,
    catalog_dialect: Option<db::mysql::MySqlCatalogDialect>,
    database: &str,
    statements: &[String],
    cancel_token: Option<CancellationToken>,
    options: QueryExecutionOptions,
    progress: Option<&ExecuteMultiProgressCallback>,
) -> Result<Vec<ExecuteMultiResult>, String> {
    let trace_id = options.execution_id.as_deref().unwrap_or("none");
    let total_started_at = std::time::Instant::now();
    let query_timeout = resolve_query_timeout(options.timeout_secs);
    let operation_budget = operation_budget_for_pool_key(state, pool_key, query_timeout).await;
    let bare = false;
    let max_rows = options.max_rows;
    let max_result_bytes = options.max_result_bytes.filter(|value| *value > 0);
    let pipeline_non_result_statements =
        mysql_non_result_pipeline_enabled(statements.len(), options.continue_on_error, mode);
    let checkout_started_at = std::time::Instant::now();
    let mut conn = match db::mysql::get_conn_with_health_check_with_cancel(
        pool,
        operation_budget.checkout_timeout,
        operation_budget.cleanup_timeout,
        cancel_token.as_ref(),
    )
    .await
    {
        Ok(conn) => conn,
        Err(err) => {
            if matches!(pool_error_action(db_type, &err), PoolErrorAction::Discard | PoolErrorAction::ReconnectAndRetry)
                || err == QUERY_CANCELED
            {
                state.remove_pool_by_key(pool_key).await;
            }
            return Ok(vec![ExecuteMultiResult::execution_error(error_query_result(err))]);
        }
    };
    let checkout_ms = checkout_started_at.elapsed().as_millis();
    apply_oceanbase_mysql_session_timeout(state, pool_key, &mut conn, options.timeout_secs).await?;
    let catalog_started_at = std::time::Instant::now();
    wait_for_result_opt(
        cancel_token.clone(),
        query_timeout,
        db::mysql::apply_catalog_database_context(&mut conn, catalog_dialect, options.catalog.as_deref(), database),
    )
    .await?;
    let catalog_ms = catalog_started_at.elapsed().as_millis();
    let pipeline_non_result_max_bytes = if pipeline_non_result_statements {
        db::mysql::max_allowed_packet_on_conn(&mut conn)
            .await
            .ok()
            .and_then(db::mysql::mysql_sql_statement_hard_limit)
            .map(|limit| limit.min(MYSQL_MULTI_STATEMENT_BATCH_MAX_BYTES))
    } else {
        None
    };

    let mut executor = MysqlBatchConnection {
        conn: &mut conn,
        cancel_token: cancel_token.clone(),
        query_timeout,
        bare,
        max_rows,
        max_result_bytes,
        result_key_columns: &options.result_key_columns,
        table_data_preview: options.table_data_preview,
        dialect,
        diagnostic_trace_id: options.execution_id.as_deref(),
    };
    let statements_started_at = std::time::Instant::now();
    let (mut results, error_action) = execute_mysql_batch_statements(
        &mut executor,
        statements,
        db_type,
        dialect,
        cancel_token,
        options.continue_on_error,
        pipeline_non_result_max_bytes,
        progress,
    )
    .await;
    let statements_ms = statements_started_at.elapsed().as_millis();
    drop(executor);

    // Tab-scoped single-connection pools disable COM_RESET_CONNECTION on return
    // (to preserve session state like temporary tables), so an open transaction
    // left on the connection — a user-typed BEGIN/START TRANSACTION without
    // COMMIT, or a canceled/aborted batch that skipped its cleanup — would pin
    // the REPEATABLE READ snapshot for every later auto-commit query on that
    // tab, making the tab read stale rows until disconnect. Closing any open
    // transaction before returning the connection restores the auto-commit
    // contract, unless the tab keeps explicit user transactions open
    // (`preserve_explicit_transaction`). The ROLLBACK round trip is skipped when
    // the final status packet proves nothing is open, and a failure here only
    // discards this connection.
    {
        let tab_scoped = pool.is_client_session_pool();
        let explicit_start_in_batch = statements
            .iter()
            .any(|statement| crate::query_execution_sql::mysql_statement_opens_explicit_transaction(statement));
        // An error, a truncated result, or result sets still pending may leave
        // the last status packet short of the end, so only a batch whose
        // response was read completely can prove that the cleanup `ROLLBACK`
        // is unnecessary.
        let status_is_final = mode == crate::connection::MysqlMode::Normal
            && error_action.is_none()
            && results.iter().all(|result| !result.execution_error && !result.result.truncated)
            && db::mysql::last_ok_ends_response(&conn);
        let rollback_started_at = std::time::Instant::now();
        let transaction = if tab_scoped {
            settle_mysql_auto_commit_transaction(
                state,
                pool_key,
                &mut conn,
                options.preserve_explicit_transaction,
                explicit_start_in_batch,
                status_is_final,
            )
            .await
        } else if mysql_cleanup_rollback_is_noop(
            status_is_final,
            explicit_start_in_batch,
            db::mysql::session_status_from_last_ok(&conn),
        ) {
            Ok(MysqlAutoCommitTransaction::None)
        } else {
            db::mysql::rollback_open_transaction(&mut conn).await.map(|()| MysqlAutoCommitTransaction::None)
        };
        match transaction {
            Ok(transaction) => {
                if rollback_started_at.elapsed() > std::time::Duration::from_millis(5) {
                    log::info!(
                        "[query][mysql-batch] trace_id={} open_txn_rollback_ms={} preserved={}",
                        trace_id,
                        rollback_started_at.elapsed().as_millis(),
                        transaction == MysqlAutoCommitTransaction::Preserved
                    );
                }
                for result in &mut results {
                    transaction.mark(result);
                }
            }
            Err(error) => {
                // A failed ROLLBACK leaves the transaction state unknown: drop
                // the connection instead of returning it to the session pool.
                log::warn!("[query][mysql-batch] trace_id={} open_txn_rollback_failed error={}", trace_id, error);
                let _ = tokio::time::timeout(Duration::from_secs(5), conn.disconnect()).await;
                state.remove_pool_by_key(pool_key).await;
                return Ok(results);
            }
        }
    }

    log::info!(
        "[query][mysql-batch] trace_id={} checkout_ms={} catalog_ms={} statements_ms={} total_ms={} result_count={} row_counts={:?}",
        trace_id,
        checkout_ms,
        catalog_ms,
        statements_ms,
        total_started_at.elapsed().as_millis(),
        results.len(),
        results.iter().map(|result| result.result.rows.len()).collect::<Vec<_>>()
    );

    if matches!(error_action, Some(PoolErrorAction::Discard | PoolErrorAction::ReconnectAndRetry)) {
        state.remove_pool_by_key(pool_key).await;
    }

    Ok(results)
}

fn error_query_result(message: String) -> db::QueryResult {
    db::QueryResult {
        columns: vec!["Error".to_string()],
        column_types: Vec::new(),
        column_sortables: vec![],
        spatial_columns: vec![],
        spatial_values: vec![],
        rows: vec![vec![serde_json::Value::String(message)]],
        affected_rows: 0,
        execution_time_ms: 0,
        server_execute_time_us: None,
        query_timings_ms: None,
        truncated: false,
        session_id: None,
        has_more: false,
        elasticsearch_raw_body: None,
        messages: Vec::new(),
    }
}

fn empty_query_result(execution_time_ms: u128) -> db::QueryResult {
    db::QueryResult {
        columns: vec![],
        column_types: Vec::new(),
        column_sortables: vec![],
        spatial_columns: vec![],
        spatial_values: vec![],
        rows: vec![],
        affected_rows: 0,
        execution_time_ms,
        server_execute_time_us: None,
        query_timings_ms: None,
        truncated: false,
        session_id: None,
        has_more: false,
        elasticsearch_raw_body: None,
        messages: Vec::new(),
    }
}

pub async fn execute_statements(
    state: &AppState,
    connection_id: &str,
    database: &str,
    statements: &[String],
    schema: Option<&str>,
    timeout_secs: Option<u64>,
) -> Result<db::QueryResult, String> {
    let db_type = connection_database_type(state, connection_id).await;
    let invalidate = statements.iter().any(|sql| crate::object_cache::sql_may_change_object_metadata(sql, db_type));
    let result = execute_statements_inner(state, connection_id, database, statements, schema, timeout_secs).await;
    if invalidate {
        crate::object_cache::invalidate_connection_object_cache(&state.storage, connection_id).await;
    }
    result
}

/// Execute a batch, optionally on a single transaction.
///
/// `use_transaction` is opt-in and only the structure editor sets it, for
/// batches that change a partition hierarchy: a mid-batch failure there would
/// otherwise leave a half-created hierarchy behind. The shared transaction
/// kernel rejects backends whose DDL cannot roll back, and statements that
/// cannot run inside a transaction block (`... CONCURRENTLY`) are refused up
/// front rather than half-applied.
pub async fn execute_statements_with_transaction_option(
    state: &AppState,
    connection_id: &str,
    database: &str,
    statements: &[String],
    schema: Option<&str>,
    use_transaction: bool,
    timeout_secs: Option<u64>,
) -> Result<db::QueryResult, String> {
    if use_transaction && statements.len() > 1 {
        if batch_has_concurrently_statement(statements) {
            return Err(
                "use_transaction cannot wrap CONCURRENTLY statements: they cannot run inside a transaction block. Run the batch without use_transaction."
                    .to_string(),
            );
        }
        let db_type = connection_database_type(state, connection_id).await;
        if batch_transaction_ddl_is_unrollbackable(db_type, statements) {
            return Err(
                "use_transaction cannot be used with a batch whose DDL cannot be rolled back: DDL statements implicitly commit and cannot be undone. Run the batch without use_transaction (auto-commit, one result per statement) or split the DDL and DML into separate calls."
                    .to_string(),
            );
        }
        let result = execute_statements_in_transaction_typed(
            state,
            connection_id,
            database,
            statements,
            schema,
            None,
            timeout_secs,
        )
        .await
        .map_err(|error| error.into_legacy_string())?;
        let invalidate = statements.iter().any(|sql| crate::object_cache::sql_may_change_object_metadata(sql, db_type));
        if invalidate {
            crate::object_cache::invalidate_connection_object_cache(&state.storage, connection_id).await;
        }
        return Ok(result);
    }
    execute_statements(state, connection_id, database, statements, schema, timeout_secs).await
}

/// Whether a batch contains a statement PostgreSQL-family servers refuse to run
/// inside a transaction block (`CREATE/DROP INDEX CONCURRENTLY`,
/// `ALTER TABLE ... DETACH PARTITION CONCURRENTLY`, ...).
fn batch_has_concurrently_statement(statements: &[String]) -> bool {
    statements.iter().any(|statement| {
        let upper = statement.to_ascii_uppercase();
        // Real CONCURRENTLY statements always start with one of these verbs.
        // Requiring the verb plus a standalone keyword keeps the word inside
        // string literals, comments or plain identifiers (e.g. a table named
        // `concurrently`) from silently demoting the batch to auto-commit; the
        // direction stays fail-safe because no CONCURRENTLY statement form
        // starts with another verb.
        let trimmed = upper.trim_start();
        let starts_with_ddl_verb =
            ["CREATE ", "DROP ", "REINDEX", "REFRESH ", "ALTER "].iter().any(|verb| trimmed.starts_with(verb));
        starts_with_ddl_verb && contains_standalone_concurrently_keyword(&upper)
    })
}

fn contains_standalone_concurrently_keyword(upper: &str) -> bool {
    let keyword = "CONCURRENTLY";
    let mut search_from = 0;
    while let Some(pos) = upper[search_from..].find(keyword) {
        let pos = search_from + pos;
        let boundary =
            |ch: Option<char>| ch.map(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '"').unwrap_or(false);
        let before = boundary(upper[..pos].chars().next_back());
        let after = boundary(upper[pos + keyword.len()..].chars().next());
        if !before && !after {
            return true;
        }
        search_from = pos + 1;
    }
    false
}

async fn execute_statements_inner(
    state: &AppState,
    connection_id: &str,
    database: &str,
    statements: &[String],
    schema: Option<&str>,
    timeout_secs: Option<u64>,
) -> Result<db::QueryResult, String> {
    let sql_ctx = statements.first().map(|s| s.as_str()).unwrap_or("");
    let pool_key = if database.is_empty() {
        connection_id.to_string()
    } else {
        state
            .get_or_create_pool(connection_id, Some(database))
            .await
            .map_err(|e| query_error_with_omitted_sql_context(&e, sql_ctx))?
    };

    let mut total_affected: u64 = 0;
    let start = std::time::Instant::now();
    let mysql_dialect = connection_mysql_query_dialect(state, connection_id).await;

    {}

    for (i, sql) in statements.iter().enumerate() {
        match do_execute(
            state,
            &pool_key,
            mysql_dialect,
            Some(database),
            sql,
            schema,
            None,
            QueryExecutionOptions { timeout_secs, ..Default::default() },
        )
        .await
        {
            Ok(result) => {
                total_affected += result.affected_rows;
            }
            Err(e) => {
                let db_type = connection_database_type(state, connection_id).await;
                match pool_error_action(db_type, &e) {
                    PoolErrorAction::ReconnectAndRetry => {
                        let db_opt = if database.is_empty() { None } else { Some(database) };
                        let _ = state.reconnect_pool(connection_id, db_opt).await;
                    }
                    PoolErrorAction::Discard if !false => {
                        let _ = state.remove_pool_by_key(&pool_key).await;
                    }
                    PoolErrorAction::Discard | PoolErrorAction::Keep => {}
                }
                let error = crate::driver_error::append_legacy_error_context(
                    &e,
                    &format!("Statement {} failed; previous {} statement(s) may have been committed.", i + 1, i),
                );
                return Err(query_error_with_omitted_sql_context(&error, sql));
            }
        }
    }

    Ok(db::QueryResult {
        columns: vec![],
        column_types: Vec::new(),
        column_sortables: vec![],
        spatial_columns: vec![],
        spatial_values: vec![],
        rows: vec![],
        affected_rows: total_affected,
        execution_time_ms: start.elapsed().as_millis(),
        server_execute_time_us: None,
        query_timings_ms: None,
        truncated: false,
        session_id: None,
        has_more: false,
        elasticsearch_raw_body: None,
        messages: Vec::new(),
    })
}

/// Deploy result for Schema Diff: single-connection transactional execution.
/// On statement failure the transaction is rolled back by the underlying path.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaDiffDeployResult {
    pub transaction_id: String,
    pub status: String,
    pub participants: Vec<crate::two_phase_commit::ParticipantInfo>,
    pub created_at: String,
    pub updated_at: String,
    pub executed_count: usize,
    pub statement_count: usize,
    pub error: Option<String>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaDiffAtomicity {
    GuaranteedRollback,
    PartialEffectsPossible,
}

impl SchemaDiffAtomicity {
    fn ddl_atomic(self) -> bool {
        matches!(self, Self::GuaranteedRollback)
    }
}

fn database_supports_transactional_ddl(db_type: DatabaseType) -> bool {
    if resolve_for_db(db_type).has_capability(CAP_TRANSACTIONAL_DDL) {
        return true;
    }
    // SQLite-family DDL is transactional even when dialect registry omits the flag.
    false
}

fn classify_schema_diff_atomicity(
    db_type: Option<DatabaseType>,
    statements: &[String],
    has_transactional_path: bool,
) -> SchemaDiffAtomicity {
    if !has_transactional_path {
        return SchemaDiffAtomicity::PartialEffectsPossible;
    }

    let mut contains_ddl = false;
    for statement in statements {
        let risk = match db_type {
            Some(db_type) => classify_sql_risk_for_database(statement, db_type).ok(),
            None => None,
        };

        match risk {
            Some(SqlRisk::Ddl) => {
                contains_ddl = true;
            }
            Some(SqlRisk::Write | SqlRisk::ReadOnly) => {}
            Some(SqlRisk::Transaction) | None => {
                return SchemaDiffAtomicity::PartialEffectsPossible;
            }
        }
    }

    if !contains_ddl {
        return SchemaDiffAtomicity::GuaranteedRollback;
    }

    match db_type {
        Some(db_type) if database_supports_transactional_ddl(db_type) => SchemaDiffAtomicity::GuaranteedRollback,
        _ => SchemaDiffAtomicity::PartialEffectsPossible,
    }
}

fn executed_count_before_error(error: &str, statement_count: usize) -> usize {
    let Some(rest) = error.strip_prefix("Statement ") else {
        return statement_count;
    };
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    let Ok(statement_number) = digits.parse::<usize>() else {
        return statement_count;
    };
    statement_number.saturating_sub(1).min(statement_count)
}

fn is_destructive_schema_diff_statement(statement: &str) -> bool {
    let normalized = strip_sql_comments_and_literals(statement).to_ascii_uppercase();
    let tokens = normalized
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    match tokens.first().copied() {
        Some("DROP" | "TRUNCATE") => true,
        Some("ALTER") => tokens.iter().skip(1).any(|token| *token == "DROP"),
        _ => false,
    }
}

/// Pure failure mapping used by deploy and unit tests.
fn schema_diff_failure_outcome(
    atomicity: SchemaDiffAtomicity,
    error: &str,
    statement_count: usize,
) -> (crate::two_phase_commit::TransactionStatus, usize) {
    if atomicity.ddl_atomic() {
        (crate::two_phase_commit::TransactionStatus::RolledBack, 0)
    } else {
        (crate::two_phase_commit::TransactionStatus::Mixed, executed_count_before_error(error, statement_count))
    }
}

fn pool_kind_has_transactional_path(pool: &PoolKind) -> bool {
    match pool {
        PoolKind::Mysql(_, _) => true,
    }
}

/// Execute Schema Diff deploy SQL as one real single-connection transaction.
///
/// - Uses [`execute_statements_in_transaction`] so partial success rolls back.
/// - Returns a structured result (never re-executes statements to probe status).
/// - Comment-only / empty scripts succeed as `committed` with zero statements.
/// - When the target path cannot guarantee DDL atomicity (MySQL/Oracle DDL
///   auto-commit, unsupported batch transaction paths, etc.), a failure reports `mixed` and
///   `executed_count` reflects the statements that were issued before the
///   error, so the caller can warn the user that partial effects may persist.
pub async fn execute_schema_diff_deploy(
    state: &AppState,
    connection_id: &str,
    database: &str,
    statements: &[String],
    schema: Option<&str>,
    destructive_confirmed: bool,
) -> SchemaDiffDeployResult {
    let tx_id = format!("deploy_{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();
    let db_type = connection_database_type(state, connection_id).await;

    // openGauss A-mode deploy scripts may contain CREATE PACKAGE … / blocks
    // that must stay intact. Probe the database compatibility mode when a pool
    // is available; on failure fall back to the plain openGauss splitter.

    let parsed: Vec<String> = statements
        .iter()
        .flat_map(|s| {
            db_type.map_or_else(
                || split_sql_statements(s),
                |dt| crate::sql::split_sql_statements_for_database_with_compatibility(s, dt, None),
            )
        })
        .map(|s| s.trim().to_string())
        .filter(|s| {
            !s.is_empty()
                && !s.lines().all(|line| {
                    let t = line.trim();
                    t.is_empty() || t.starts_with("--")
                })
        })
        .collect();

    let participant = crate::two_phase_commit::ParticipantInfo {
        id: "connection".to_string(),
        name: format!("{connection_id}/{database}"),
        role: "database".to_string(),
    };

    let destructive_statement_count =
        parsed.iter().filter(|statement| is_destructive_schema_diff_statement(statement)).count();
    if destructive_statement_count > 0 && !destructive_confirmed {
        return SchemaDiffDeployResult {
            transaction_id: tx_id,
            status: crate::two_phase_commit::TransactionStatus::RolledBack.as_str().to_string(),
            participants: vec![participant],
            created_at: now.clone(),
            updated_at: now,
            executed_count: 0,
            statement_count: parsed.len(),
            error: Some("Destructive schema diff SQL requires explicit confirmation".to_string()),
            metadata: serde_json::json!({
                "source": "schema_diff_deploy",
                "mode": "single_connection_tx",
                "blocked": "destructive_confirmation_required",
                "destructive_statement_count": destructive_statement_count,
            }),
        };
    }

    if parsed.is_empty() {
        return SchemaDiffDeployResult {
            transaction_id: tx_id,
            status: crate::two_phase_commit::TransactionStatus::Committed.as_str().to_string(),
            participants: vec![participant],
            created_at: now.clone(),
            updated_at: now,
            executed_count: 0,
            statement_count: 0,
            error: None,
            metadata: serde_json::json!({"source": "schema_diff_deploy", "mode": "single_connection_tx"}),
        };
    }

    let pool_key = if database.is_empty() {
        connection_id.to_string()
    } else {
        match state.get_or_create_pool(connection_id, Some(database)).await {
            Ok(key) => key,
            Err(_) => {
                return SchemaDiffDeployResult {
                    transaction_id: tx_id.clone(),
                    status: crate::two_phase_commit::TransactionStatus::RolledBack.as_str().to_string(),
                    participants: vec![participant],
                    created_at: now.clone(),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    executed_count: 0,
                    statement_count: parsed.len(),
                    error: Some("Connection not available for deploy".to_string()),
                    metadata: serde_json::json!({"source": "schema_diff_deploy", "mode": "single_connection_tx"}),
                };
            }
        }
    };
    let has_transactional_path =
        { state.pool_handle(&pool_key).await.as_ref().is_some_and(pool_kind_has_transactional_path) };
    let atomicity = classify_schema_diff_atomicity(db_type, &parsed, has_transactional_path);

    match execute_statements_in_transaction_on_pool(state, &pool_key, connection_id, database, &parsed, schema, None)
        .await
    {
        Ok(result) => SchemaDiffDeployResult {
            transaction_id: tx_id,
            status: crate::two_phase_commit::TransactionStatus::Committed.as_str().to_string(),
            participants: vec![participant],
            created_at: now.clone(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            executed_count: parsed.len(),
            statement_count: parsed.len(),
            error: None,
            metadata: serde_json::json!({
                "source": "schema_diff_deploy",
                "mode": "single_connection_tx",
                "affected_rows": result.affected_rows,
                "execution_time_ms": result.execution_time_ms,
            }),
        },
        Err(e) => {
            let (status, executed_count) = schema_diff_failure_outcome(atomicity, &e, parsed.len());
            SchemaDiffDeployResult {
                transaction_id: tx_id,
                status: status.as_str().to_string(),
                participants: vec![participant],
                created_at: now.clone(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                executed_count,
                statement_count: parsed.len(),
                error: Some(e.clone()),
                metadata: serde_json::json!({
                    "source": "schema_diff_deploy",
                    "mode": "single_connection_tx",
                    "ddl_atomic": atomicity.ddl_atomic(),
                    "atomicity": match atomicity {
                        SchemaDiffAtomicity::GuaranteedRollback => "guaranteed_rollback",
                        SchemaDiffAtomicity::PartialEffectsPossible => "partial_effects_possible",
                    },
                    "error": e,
                }),
            }
        }
    }
}

/// Execute multiple SQL statements within a single transaction.
/// For pooled drivers (Postgres/MySQL), uses the driver transaction API.
/// For SQLite and SQL Server, uses a transaction on the driver's shared connection.
/// Agent drivers must provide the same rollbackable transaction contract.
/// Backends without a verified rollbackable path are rejected instead of being
/// silently executed one statement at a time in auto-commit mode.
pub async fn execute_statements_in_transaction(
    state: &AppState,
    connection_id: &str,
    database: &str,
    statements: &[String],
    schema: Option<&str>,
    catalog: Option<&str>,
) -> Result<db::QueryResult, String> {
    execute_statements_in_transaction_typed(state, connection_id, database, statements, schema, catalog, None)
        .await
        .map_err(QueryExecutionError::into_legacy_string)
}

/// Execute multiple SQL statements transactionally while retaining typed failures.
pub async fn execute_statements_in_transaction_typed(
    state: &AppState,
    connection_id: &str,
    database: &str,
    statements: &[String],
    schema: Option<&str>,
    catalog: Option<&str>,
    timeout_secs: Option<u64>,
) -> Result<db::QueryResult, QueryExecutionError> {
    let sql_ctx = statements.first().map(|s| s.as_str()).unwrap_or("");
    let pool_database = query_pool_database(database, catalog);
    let pool_key = state
        .get_or_create_pool(connection_id, pool_database)
        .await
        .map_err(|e| query_error_with_omitted_sql_context(&e, sql_ctx))?;

    execute_statements_in_transaction_on_pool_typed(
        state,
        &pool_key,
        connection_id,
        database,
        statements,
        schema,
        catalog,
        timeout_secs,
    )
    .await
}

/// Execute multiple SQL statements transactionally on an already-resolved pool.
/// This preserves session-scoped pools used by long-running imports.
pub async fn execute_statements_in_transaction_on_pool(
    state: &AppState,
    pool_key: &str,
    connection_id: &str,
    database: &str,
    statements: &[String],
    schema: Option<&str>,
    catalog: Option<&str>,
) -> Result<db::QueryResult, String> {
    execute_statements_in_transaction_on_pool_typed(
        state,
        pool_key,
        connection_id,
        database,
        statements,
        schema,
        catalog,
        None,
    )
    .await
    .map_err(QueryExecutionError::into_legacy_string)
}

/// Execute a transaction on an already-resolved pool without erasing typed failures.
pub async fn execute_statements_in_transaction_on_pool_typed(
    state: &AppState,
    pool_key: &str,
    connection_id: &str,
    database: &str,
    statements: &[String],
    schema: Option<&str>,
    catalog: Option<&str>,
    timeout_secs: Option<u64>,
) -> Result<db::QueryResult, QueryExecutionError> {
    let db_type = connection_database_type(state, connection_id).await;

    // Read-only check: intercept all transaction paths before dispatching
    check_read_only_for_connection_multi(state, pool_key, statements).await?;

    let start = std::time::Instant::now();
    let mysql_catalog_dialect = connection_mysql_catalog_dialect(state, connection_id).await;
    let mut operation_budget = configured_operation_budget_for_pool_key(state, pool_key).await;
    apply_query_timeout_override(&mut operation_budget, timeout_secs);

    // This is the single capability model for explicit batch transactions.
    // Do not add a backend here unless its execution path keeps every statement
    // on one transaction-capable connection and can roll back on failure.
    // Agent drivers are delegated because their transaction RPC has the same
    // contract and rejects drivers that cannot provide it.
    // Clone the pool handle within the lock, then drop it before any async work.
    let path = { state.pool_handle(pool_key).await.as_ref().map(batch_transaction_path) };

    let result = match path {
        Some(BatchTransactionPath::Mysql(pool)) => exec_tx_mysql_inner(
            state,
            pool_key,
            pool,
            statements,
            start,
            operation_budget.clone(),
            mysql_catalog_dialect,
            catalog,
            database,
        )
        .await
        .map_err(QueryExecutionError::from),

        Some(BatchTransactionPath::Explicit) => {
            let mysql_dialect = connection_mysql_query_dialect(state, connection_id).await;
            exec_tx_explicit_inner(state, pool_key, mysql_dialect, Some(database), statements, schema, start)
                .await
                .map_err(QueryExecutionError::from)
        }
        Some(BatchTransactionPath::Unsupported) => Err(
            "The active backend cannot provide a rollbackable transaction for a batch; run without use_transaction."
                .to_string()
                .into(),
        ),
        None => Err("Connection not found for transaction".to_string().into()),
    };

    if let Err(err) = result.as_ref() {
        discard_pool_after_error(state, pool_key, db_type, &err.to_string()).await;
    }

    result
}

/// Whether an opt-in explicit batch transaction (`use_transaction`) must be
/// rejected because the backend's DDL statements implicitly commit and cannot be
/// rolled back. Used by the opt-in batch-transaction entry point (the
/// `use_transaction` branch of [`execute_multi_core_with_options_for_client_and_progress_typed`])
/// and by the MCP layer's identical pre-check. It covers every backend whose DDL
/// is not rollbackable (MySQL-family and Oracle) via the single capability
/// predicate (`database_supports_transactional_ddl`) without re-listing engines
/// here. Paths that document a mixed-outcome-on-failure behaviour (schema-diff
/// deploy, imports) do not call this and keep running-and-reporting.
pub fn batch_transaction_ddl_is_unrollbackable(db_type: Option<DatabaseType>, statements: &[String]) -> bool {
    let Some(db_type) = db_type else {
        return false;
    };
    if database_supports_transactional_ddl(db_type) {
        return false;
    }
    statements.iter().any(|statement| matches!(classify_sql_risk_for_database(statement, db_type), Ok(SqlRisk::Ddl)))
}

/// Owned transaction-capable pool variants for safe dispatch across async boundaries.
enum BatchTransactionPath {
    Mysql(db::mysql::MySqlPool),

    Explicit,
    Unsupported,
}

fn batch_transaction_path(pool: &PoolKind) -> BatchTransactionPath {
    match pool {
        PoolKind::Mysql(pool, _mode) => BatchTransactionPath::Mysql(pool.clone()),
    }
}

async fn exec_tx_mysql_inner(
    state: &AppState,
    pool_key: &str,
    pool: db::mysql::MySqlPool,
    statements: &[String],
    start: std::time::Instant,
    budget: DbOperationBudget,
    catalog_dialect: Option<db::mysql::MySqlCatalogDialect>,
    catalog: Option<&str>,
    database: &str,
) -> Result<db::QueryResult, String> {
    let mut conn = db::mysql::get_conn_with_health_check_with_timeout(&pool, budget.checkout_timeout).await?;
    apply_oceanbase_mysql_session_timeout(state, pool_key, &mut conn, None).await?;
    db::mysql::apply_catalog_database_context(&mut conn, catalog_dialect, catalog, database).await?;
    mysql_query_drop_with_timeout(
        &mut conn,
        "START TRANSACTION",
        budget.recycle_timeout,
        "Failed to begin transaction",
    )
    .await?;
    let mut total_affected: u64 = 0;
    for (i, sql) in statements.iter().enumerate() {
        match mysql_query_iter_with_timeout(&mut conn, sql, budget.query_timeout).await {
            Ok(affected) => total_affected += affected,
            Err(e) => {
                let _ = mysql_query_drop_with_timeout(&mut conn, "ROLLBACK", budget.cleanup_timeout, "ROLLBACK failed")
                    .await;
                return Err(query_error_with_omitted_sql_context(&format!("Statement {} failed: {}", i + 1, e), sql));
            }
        }
    }
    mysql_query_drop_with_timeout(&mut conn, "COMMIT", budget.cleanup_timeout, "COMMIT failed").await?;
    Ok(db::QueryResult {
        columns: vec![],
        column_types: Vec::new(),
        column_sortables: vec![],
        spatial_columns: vec![],
        spatial_values: vec![],
        rows: vec![],
        affected_rows: total_affected,
        execution_time_ms: start.elapsed().as_millis(),
        server_execute_time_us: None,
        query_timings_ms: None,
        truncated: false,
        session_id: None,
        has_more: false,
        elasticsearch_raw_body: None,
        messages: Vec::new(),
    })
}

async fn mysql_query_drop_with_timeout(
    conn: &mut mysql_async::Conn,
    sql: &str,
    timeout_duration: Duration,
    context: &str,
) -> Result<(), String> {
    tokio::time::timeout(timeout_duration, conn.query_drop(sql))
        .await
        .map_err(|_| format!("{context}: timed out after {} seconds", timeout_duration.as_secs()))?
        .map_err(|e| format!("{context}: {e}"))
}

async fn mysql_query_iter_with_timeout(
    conn: &mut mysql_async::Conn,
    sql: &str,
    timeout_duration: Option<Duration>,
) -> Result<u64, String> {
    match timeout_duration {
        Some(timeout_duration) => tokio::time::timeout(timeout_duration, conn.query_iter(sql))
            .await
            .map_err(|_| format!("Query timed out after {} seconds", timeout_duration.as_secs()))?
            .map(|result| result.affected_rows())
            .map_err(|e| e.to_string()),
        None => conn.query_iter(sql).await.map(|result| result.affected_rows()).map_err(|e| e.to_string()),
    }
}

async fn exec_tx_explicit_inner(
    state: &AppState,
    pool_key: &str,
    mysql_dialect: db::mysql::MySqlQueryDialect,
    database: Option<&str>,
    statements: &[String],
    schema: Option<&str>,
    start: std::time::Instant,
) -> Result<db::QueryResult, String> {
    do_execute(
        state,
        pool_key,
        mysql_dialect,
        database,
        "BEGIN TRANSACTION",
        schema,
        None,
        QueryExecutionOptions::default(),
    )
    .await
    .map_err(|e| format!("Failed to begin transaction: {}", e))?;

    let mut total_affected: u64 = 0;
    for (i, sql) in statements.iter().enumerate() {
        match do_execute(state, pool_key, mysql_dialect, database, sql, schema, None, QueryExecutionOptions::default())
            .await
        {
            Ok(result) => {
                total_affected += result.affected_rows;
            }
            Err(e) => {
                if let Err(rb_err) = do_execute(
                    state,
                    pool_key,
                    mysql_dialect,
                    database,
                    "ROLLBACK",
                    schema,
                    None,
                    QueryExecutionOptions::default(),
                )
                .await
                {
                    log::error!("ROLLBACK failed after statement {} error: {}", i + 1, rb_err);
                }
                return Err(query_error_with_omitted_sql_context(&format!("Statement {} failed: {}", i + 1, e), sql));
            }
        }
    }

    do_execute(state, pool_key, mysql_dialect, database, "COMMIT", schema, None, QueryExecutionOptions::default())
        .await
        .map_err(|e| format!("COMMIT failed: {}", e))?;

    Ok(db::QueryResult {
        columns: vec![],
        column_types: Vec::new(),
        column_sortables: vec![],
        spatial_columns: vec![],
        spatial_values: vec![],
        rows: vec![],
        affected_rows: total_affected,
        execution_time_ms: start.elapsed().as_millis(),
        server_execute_time_us: None,
        query_timings_ms: None,
        truncated: false,
        session_id: None,
        has_more: false,
        elasticsearch_raw_body: None,
        messages: Vec::new(),
    })
}

/// Start a manual transaction session, holding a connection from the pool.
/// Returns a transaction session ID that must be passed to subsequent calls.
pub async fn begin_manual_transaction(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: Option<&str>,
    catalog: Option<&str>,
) -> Result<String, String> {
    begin_transaction_session(state, connection_id, database, schema, catalog, false).await
}

/// Start a read-only, repeatable snapshot for a database backup.
pub async fn begin_database_backup_snapshot(
    state: &AppState,
    connection_id: &str,
    database: &str,
) -> Result<String, String> {
    begin_transaction_session(state, connection_id, database, None, None, true).await
}

fn mysql_transaction_begin_sql_candidates(consistent_snapshot: bool) -> &'static [&'static str] {
    if consistent_snapshot {
        &[
            "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY",
            "START TRANSACTION WITH CONSISTENT SNAPSHOT",
            "START TRANSACTION",
        ]
    } else {
        &["START TRANSACTION"]
    }
}

fn mysql_transaction_isolation_sql(consistent_snapshot: bool) -> Option<&'static str> {
    consistent_snapshot.then_some("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
}

fn mysql_error_is_syntax_error(error: &mysql_async::Error) -> bool {
    match error {
        mysql_async::Error::Server(server_error) if server_error.code == 1064 => true,
        // Doris (and some other MySQL-compatible servers) report unsupported
        // transaction clauses as ER_UNKNOWN_ERROR (1105) with a parser
        // diagnostic instead of MySQL's ER_PARSE_ERROR (1064).  Only treat
        // diagnostics that clearly identify a syntax/parser failure as
        // retryable; generic 1105 errors may indicate a real server failure.
        mysql_async::Error::Server(server_error) if server_error.code == 1105 => {
            let message = server_error.message.to_ascii_lowercase();
            message.contains("syntax error") && (message.contains("encountered:") || message.contains("expected"))
        }
        // PolarDB-X exposes parser failures such as PXC-4500 / ERR_PARSER
        // through the generic MySQL protocol ERR_HANDLE_DATA code (3009).
        mysql_async::Error::Server(server_error) if server_error.code == 3009 => {
            let message = server_error.message.to_ascii_lowercase();
            message.contains("[err_parser]") || message.contains("syntax error") || message.contains("语法错误")
        }
        _ => false,
    }
}

/// Compute per-execution-statement proven-read-only markers for the sticky
/// manual-transaction UX (#7122 Oracle, #9018 MySQL/PostgreSQL). The user-facing
/// classification SQL is split with the same dialect-aware splitter as the
/// execution SQL and paired by count/position; any mismatch is fail-closed (no
/// markers). Oracle/OceanBase-Oracle keep the lexical classifier, MySQL and
/// PostgreSQL use the strict `sql_risk` proof; every other dialect is unproven.
fn classify_manual_transaction_statements(
    database_type: Option<DatabaseType>,
    execution_statement_count: usize,
    classification_sql: Option<&str>,
) -> Vec<bool> {
    let Some(database_type) = database_type.filter(|database_type| matches!(database_type, DatabaseType::Mysql)) else {
        return Vec::new();
    };
    let Some(classification_sql) = classification_sql else {
        return Vec::new();
    };
    let user_statements = crate::sql::split_sql_statements_for_database(classification_sql, database_type);
    let paired = user_statements.len() == execution_statement_count && !user_statements.is_empty();
    if !paired {
        return Vec::new();
    }
    user_statements
        .iter()
        .map(|statement| match database_type {
            DatabaseType::Mysql => {
                crate::sql_risk::prove_read_only_for_database(statement, database_type)
                    == crate::sql_risk::ReadProof::ProvenReadOnly
            }
            _ => false,
        })
        .collect()
}

async fn begin_transaction_session(
    state: &AppState,
    connection_id: &str,
    database: &str,
    schema: Option<&str>,
    catalog: Option<&str>,
    consistent_snapshot: bool,
) -> Result<String, String> {
    {}
    let mysql_catalog_dialect = connection_mysql_catalog_dialect(state, connection_id).await;
    let pool_database = query_pool_database(database, catalog);
    // Probe the primary pool to learn the backend kind. Agent manual TX opens a
    // dedicated multi_session workload pool so sticky TX state is isolated.
    let probe_pool_key = state.get_or_create_pool(connection_id, pool_database).await?;

    // Clone the pool handle under a brief read lock, then drop the lock before
    // any async I/O — same pattern as do_execute throughout this file.
    enum TxnPoolHandle {
        Mysql(db::mysql::MySqlPool),
    }
    let pool_handle = {
        let pool = state.pool_handle(&probe_pool_key).await.ok_or("Connection not found")?;
        match &pool {
            PoolKind::Mysql(mp, _) => TxnPoolHandle::Mysql(mp.clone()),

            _ => return Err("Manual transaction is not supported for this database type".to_string()),
        }
    }; // connections lock released here

    let (txn_conn, pool_key) = match pool_handle {
        TxnPoolHandle::Mysql(mysql_pool) => {
            let mut conn = mysql_pool.get_conn().await.map_err(|e| format!("Failed to get MySQL connection: {e}"))?;
            db::mysql::apply_catalog_database_context(&mut conn, mysql_catalog_dialect, catalog, database).await?;
            if let Some(isolation_sql) = mysql_transaction_isolation_sql(consistent_snapshot) {
                conn.query_drop(isolation_sql).await.map_err(|e| format!("SET TRANSACTION failed: {e}"))?;
            }
            let mut syntax_errors = Vec::new();
            for begin_sql in mysql_transaction_begin_sql_candidates(consistent_snapshot) {
                match conn.query_drop(*begin_sql).await {
                    Ok(()) => {
                        syntax_errors.clear();
                        break;
                    }
                    Err(error) if mysql_error_is_syntax_error(&error) => {
                        syntax_errors.push(format!("{begin_sql}: {error}"));
                    }
                    Err(error) => return Err(format!("START TRANSACTION failed: {error}")),
                }
            }
            if !syntax_errors.is_empty() {
                return Err(format!("START TRANSACTION failed for all compatible forms: {}", syntax_errors.join("; ")));
            }
            (TxnConnection::Mysql(Some(conn)), probe_pool_key.clone())
        }
    };

    let txn_session_id = uuid::Uuid::new_v4().to_string();
    let session = TransactionSession {
        connection: Arc::new(tokio::sync::Mutex::new(txn_conn)),
        pool_key: pool_key.clone(),
        last_activity: std::time::Instant::now(),
        busy: false,
        snapshot_rotation_safe: !consistent_snapshot,
        connection_id: connection_id.to_string(),
        database: database.to_string(),
        schema: schema.map(|s| s.to_string()),
    };

    {
        let mut sessions = state.transaction_sessions.write().await;
        sessions.insert(txn_session_id.clone(), session);
    }

    // Schedule idle timeout watcher
    spawn_txn_idle_watcher(state, txn_session_id.clone());

    log::info!("[query][manual_txn:begin] session_id={}", txn_session_id);
    Ok(txn_session_id)
}

pub struct ManualTransactionKeepAlive {
    task: tokio::task::JoinHandle<()>,
    sessions: Arc<tokio::sync::RwLock<std::collections::HashMap<String, TransactionSession>>>,
    txn_session_id: String,
}

impl Drop for ManualTransactionKeepAlive {
    fn drop(&mut self) {
        self.task.abort();
        spawn_txn_idle_watcher_for_sessions(Arc::clone(&self.sessions), self.txn_session_id.clone());
    }
}

/// Keep an existing transaction session alive while a caller prepares work for
/// that session. The caller must retain the returned guard for the full period;
/// dropping it restores the normal five-minute idle rollback behavior.
pub async fn keep_manual_transaction_alive(
    state: &AppState,
    txn_session_id: &str,
) -> Result<ManualTransactionKeepAlive, String> {
    {
        let mut sessions = state.transaction_sessions.write().await;
        let session =
            sessions.get_mut(txn_session_id).ok_or_else(|| MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR.to_string())?;
        if !session.busy {
            session.last_activity = std::time::Instant::now();
        }
    }

    let sessions = Arc::clone(&state.transaction_sessions);
    let keep_alive_sessions = Arc::clone(&sessions);
    let txn_session_id = txn_session_id.to_string();
    let keep_alive_txn_session_id = txn_session_id.clone();
    let task = tokio::spawn(async move {
        const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
        loop {
            tokio::time::sleep(KEEPALIVE_INTERVAL).await;
            let should_continue = {
                let mut guard = sessions.write().await;
                if let Some(session) = guard.get_mut(&keep_alive_txn_session_id) {
                    if !session.busy {
                        session.last_activity = std::time::Instant::now();
                    }
                    true
                } else {
                    false
                }
            };
            if !should_continue {
                break;
            }
        }
    });
    Ok(ManualTransactionKeepAlive { task, sessions: keep_alive_sessions, txn_session_id })
}

/// Execute SQL within an existing manual transaction session.
pub async fn execute_in_manual_transaction(
    state: &AppState,
    txn_session_id: &str,
    sql: &str,
    database: &str,
    schema: Option<&str>,
    max_rows: Option<usize>,
) -> Result<Vec<db::QueryResult>, String> {
    execute_in_manual_transaction_with_options(
        state,
        txn_session_id,
        sql,
        database,
        schema,
        ManualTransactionExecutionOptions { max_rows, ..Default::default() },
    )
    .await
    .map(|results| results.into_iter().map(ExecuteMultiResult::into_query_result).collect())
}

#[derive(Clone, Debug, Default)]
pub struct ManualTransactionExecutionOptions {
    pub max_rows: Option<usize>,
    pub table_data_preview: bool,
    pub execution_id: Option<String>,
    pub timeout_secs: Option<u64>,
    pub page_size: Option<usize>,
    pub result_session_id: Option<String>,
    /// User-facing SQL to classify (Oracle-only). When present, the core
    /// classifies each split statement of this SQL rather than the rewritten
    /// execution SQL, pairing them by count and position so DBX-owned
    /// read-preserving rewrites (hidden primary keys, sort wrappers,
    /// pagination) do not change toolbar semantics. Fail-closed on mismatch.
    pub classification_sql: Option<String>,
}

pub async fn execute_in_manual_transaction_with_options(
    state: &AppState,
    txn_session_id: &str,
    sql: &str,
    database: &str,
    schema: Option<&str>,
    options: ManualTransactionExecutionOptions,
) -> Result<Vec<ExecuteMultiResult>, String> {
    {}
    const TXN_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(MANUAL_TRANSACTION_IDLE_TIMEOUT_SECS);

    // Resolve statements and validate before taking the per-session connection
    // lock. The session stays visible in the map so close/disconnect cleanup can
    // remove it and roll back once the current DB operation releases the lock.
    let (pool_key, connection_id) = {
        let sessions = state.transaction_sessions.read().await;
        let session = sessions.get(txn_session_id).ok_or(MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR)?;
        (session.pool_key.clone(), session.connection_id.clone())
    };

    let db_type = connection_database_type(state, &connection_id).await;

    let statements = db_type.map_or_else(
        || split_sql_statements(sql),
        |db_type| crate::sql::split_sql_statements_for_database_with_compatibility(sql, db_type, None),
    );
    if statements.is_empty() {
        // Sticky-dialect UX marker: the no-op is Core's decision that the script
        // (empty/whitespace/comments-only) has no statements, so the frontend
        // must not treat it as an unproven statement. Every other database
        // receives the plain empty result.
        let mut result = ExecuteMultiResult::success_with_optional_server_large_values(
            empty_query_result(0),
            options.table_data_preview,
        );
        if matches!(db_type, Some(DatabaseType::Mysql)) {
            result = result.with_manual_transaction_no_statement();
        }
        return Ok(vec![result]);
    }
    // A result-session cursor can only track one result set, so pagination is
    // single-statement only. Multi-statement scripts predate pagination: keep
    // the legacy sequential execution and ignore the pagination options rather
    // than failing the whole script.
    let options = if statements.len() != 1 && (options.page_size.is_some() || options.result_session_id.is_some()) {
        ManualTransactionExecutionOptions { page_size: None, result_session_id: None, ..options }
    } else {
        options
    };
    if options.result_session_id.is_some() && options.page_size.is_none() {
        return Err("Manual transaction result pagination requires a page size".to_string());
    }

    // Read-only check while the session is still in the map. If this fails the
    // session remains intact.
    check_read_only_for_connection_multi(state, &pool_key, &statements).await?;

    let classification: Vec<bool> =
        classify_manual_transaction_statements(db_type, statements.len(), options.classification_sql.as_deref());

    let connection = {
        let mut sessions = state.transaction_sessions.write().await;
        let Some(session) = sessions.get_mut(txn_session_id) else {
            return Err(MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR.to_string());
        };
        if session.busy {
            return Err("Transaction session is already executing".to_string());
        }
        if session.last_activity.elapsed() > TXN_IDLE_TIMEOUT {
            let session = sessions.remove(txn_session_id).expect("session exists");
            Some(session)
        } else {
            session.busy = true;
            session.snapshot_rotation_safe &=
                classification.len() == statements.len() && classification.iter().all(|proven| *proven);
            session.last_activity = std::time::Instant::now();
            None
        }
    };
    if let Some(session) = connection {
        let mut conn = session.connection.lock().await;
        let _ = rollback_manual_txn_connection(&mut conn).await;
        release_manual_txn_session_pool(state, &session.connection_id, &mut conn).await;
        return Err(MANUAL_TRANSACTION_IDLE_ROLLBACK_ERROR.to_string());
    }

    let connection = {
        let sessions = state.transaction_sessions.read().await;
        sessions
            .get(txn_session_id)
            .map(|session| Arc::clone(&session.connection))
            .ok_or(MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR)?
    };
    let row_limit = options.max_rows.unwrap_or(MAX_ROWS).max(1);
    let mut results = Vec::with_capacity(statements.len());

    let mut conn = connection.lock().await;
    for (i, statement) in statements.iter().enumerate() {
        let result = match &mut *conn {
            TxnConnection::Mysql(conn) => {
                execute_manual_txn_mysql_statement(
                    conn.as_mut().ok_or_else(|| MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR.to_string())?,
                    statement,
                    row_limit,
                )
                .await
            }
        };
        match result {
            Ok(query_result) => {
                let mut executed = ExecuteMultiResult::success_with_optional_server_large_values(
                    query_result,
                    options.table_data_preview,
                );
                if classification.get(i).copied().unwrap_or(false) {
                    executed = executed.with_manual_transaction_proven_read_only();
                }
                results.push(executed);
            }
            Err(e) => {
                // Statement failure ends the transaction. If another cleanup path
                // already removed the session, it owns the final rollback.
                let should_rollback = {
                    let mut sessions = state.transaction_sessions.write().await;
                    sessions.remove(txn_session_id).is_some()
                };
                if should_rollback {
                    let _ = rollback_manual_txn_connection(&mut conn).await;
                    release_manual_txn_session_pool(state, &connection_id, &mut conn).await;
                }
                return Err(format!("Statement {} failed: {}. The manual transaction was rolled back.", i + 1, e));
            }
        }
    }

    // Snapshot rotation for fully proven read-only batches (native MySQL/PG
    // connections only): see rotate_clean_read_only_snapshot. Runs while the
    // session is still marked busy so no concurrent execution can observe the
    // half-rotated transaction. A rotation failure tears the session down and
    // leans on the frontend rolled-back-session recovery (next execution opens
    // a fresh session) instead of failing the already-successful batch.
    if manual_txn_batch_fully_proven_read_only(&results) && {
        let sessions = state.transaction_sessions.read().await;
        sessions.get(txn_session_id).map(|session| session.can_rotate_read_only_snapshot(&conn)).unwrap_or(false)
    } {
        if let Err(rotation_error) = rotate_clean_read_only_snapshot(&mut conn, schema).await {
            let removed = {
                let mut sessions = state.transaction_sessions.write().await;
                sessions.remove(txn_session_id).is_some()
            };
            if removed {
                let _ = rollback_manual_txn_connection(&mut conn).await;
                release_manual_txn_session_pool(state, &connection_id, &mut conn).await;
            }
            let _ = rotation_error;
        }
    }
    drop(conn);

    let should_watch = {
        let mut sessions = state.transaction_sessions.write().await;
        if let Some(session) = sessions.get_mut(txn_session_id) {
            session.busy = false;
            session.last_activity = std::time::Instant::now();
            true
        } else {
            false
        }
    };
    if should_watch {
        spawn_txn_idle_watcher(state, txn_session_id.to_string());
    }

    Ok(results)
}

/// Stream a read query through an existing transaction without materializing
/// the whole result set. The callback runs once per batch while the same held
/// connection and transaction snapshot remain active.
pub async fn stream_rows_in_manual_transaction<F>(
    state: &AppState,
    txn_session_id: &str,
    sql: &str,
    batch_size: usize,
    on_batch: F,
) -> Result<u64, String>
where
    F: FnMut(Vec<Vec<serde_json::Value>>) -> Result<(), String> + Send,
{
    stream_rows_in_manual_transaction_with_cancel(state, txn_session_id, sql, batch_size, None, on_batch).await
}

pub(crate) async fn stream_rows_in_manual_transaction_with_cancel<F>(
    state: &AppState,
    txn_session_id: &str,
    sql: &str,
    batch_size: usize,
    cancel_token: Option<CancellationToken>,
    mut on_batch: F,
) -> Result<u64, String>
where
    F: FnMut(Vec<Vec<serde_json::Value>>) -> Result<(), String> + Send,
{
    const TXN_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(MANUAL_TRANSACTION_IDLE_TIMEOUT_SECS);

    let expired_connection = {
        let mut sessions = state.transaction_sessions.write().await;
        let Some(session) = sessions.get_mut(txn_session_id) else {
            return Err(MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR.to_string());
        };
        if session.busy {
            return Err("Transaction session is already executing".to_string());
        }
        if session.last_activity.elapsed() > TXN_IDLE_TIMEOUT {
            Some(sessions.remove(txn_session_id).expect("session exists").connection)
        } else {
            session.busy = true;
            session.snapshot_rotation_safe = false;
            session.last_activity = std::time::Instant::now();
            None
        }
    };
    if let Some(connection) = expired_connection {
        let mut conn = connection.lock().await;
        let _ = rollback_manual_txn_connection(&mut conn).await;
        return Err(MANUAL_TRANSACTION_IDLE_ROLLBACK_ERROR.to_string());
    }

    let (connection, pool_key, connection_id) = {
        let sessions = state.transaction_sessions.read().await;
        sessions
            .get(txn_session_id)
            .map(|session| (Arc::clone(&session.connection), session.pool_key.clone(), session.connection_id.clone()))
            .ok_or(MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR)?
    };
    let operation_budget = {
        let configs = state.configs.read().await;
        configs
            .get(&connection_id)
            .map(DbOperationBudget::from_connection_config)
            .unwrap_or_else(DbOperationBudget::with_defaults)
    };

    let batch_size = batch_size.max(1);
    let mut conn = connection.lock().await;
    let stream_result = match &mut *conn {
        TxnConnection::Mysql(Some(conn)) => {
            // The query timeout is an inactivity budget reset by every received row,
            // not a cap on the total duration of a long backup/export stream.
            let progress_clock = Arc::new(StreamProgressClock::new());
            let progress_clock_for_rows = progress_clock.clone();
            let timeout_error = format!(
                "Query timed out after {} seconds",
                operation_budget.query_timeout.map_or(0, |timeout| timeout.as_secs())
            );
            let stream_future = async {
                let mut result = conn.query_iter(sql).await.map_err(|error| format!("Query failed: {error}"))?;
                let Some(mut stream) =
                    result.stream::<mysql_async::Row>().await.map_err(|error| format!("Query failed: {error}"))?
                else {
                    return Err("Empty result set stream".to_string());
                };

                let mut batch = Vec::with_capacity(batch_size);
                let mut total_rows = 0_u64;
                while let Some(row_result) = stream.next().await {
                    match row_result {
                        Ok(row) => {
                            batch.push(
                                (0..row.len()).map(|index| db::mysql::mysql_value_to_json(&row, index)).collect(),
                            );
                            total_rows += 1;
                            if batch.len() >= batch_size {
                                on_batch(std::mem::take(&mut batch))?;
                                batch = Vec::with_capacity(batch_size);
                            }
                        }
                        Err(err) => return Err(format!("Query failed: {err}")),
                    }
                    progress_clock_for_rows.mark();
                }
                if !batch.is_empty() {
                    on_batch(batch)?;
                }
                Ok(total_rows)
            };
            await_stream_with_progress_timeout(
                stream_future,
                operation_budget.query_timeout,
                progress_clock,
                cancel_token.as_ref(),
                timeout_error,
            )
            .await
        }
        TxnConnection::Mysql(None) => Err(MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR.to_string()),
    };

    if let Err(err) = &stream_result {
        let removed = {
            let mut sessions = state.transaction_sessions.write().await;
            sessions.remove(txn_session_id)
        };
        if let Some(session) = removed {
            let rollback_result = if err == QUERY_CANCELED {
                discard_mysql_manual_txn_connection(&mut conn, operation_budget.cleanup_timeout).await
            } else {
                rollback_manual_txn_connection_with_timeout(&mut conn, Some(operation_budget.cleanup_timeout)).await
            };
            release_manual_txn_session_pool(state, &session.connection_id, &mut conn).await;
            if let Err(rollback_error) = rollback_result {
                return Err(format!("{err}. Transaction cleanup failed: {rollback_error}"));
            }
        }
        return Err(format!("{err}. Transaction was auto-rolled back."));
    }
    drop(conn);

    let should_watch = {
        let mut sessions = state.transaction_sessions.write().await;
        if let Some(session) = sessions.get_mut(txn_session_id) {
            session.busy = false;
            session.last_activity = std::time::Instant::now();
            true
        } else {
            false
        }
    };
    if should_watch {
        spawn_txn_idle_watcher(state, txn_session_id.to_string());
    }
    stream_result
}

/// Whether a finished batch was fully proven read-only. Session history is
/// checked separately before rotating its snapshot.
fn manual_txn_batch_fully_proven_read_only(results: &[ExecuteMultiResult]) -> bool {
    !results.is_empty() && results.iter().all(|result| result.manual_transaction_proven_read_only)
}

/// Rotate the transaction on a natively-connected MySQL/PostgreSQL session after
/// a fully proven read-only batch. MySQL REPEATABLE READ (and PostgreSQL when
/// the server default was changed to a snapshot isolation) pins the read view of
/// the first SELECT for the whole transaction: a user who keeps polling a clean
/// read-only session would otherwise never see rows committed by others, and the
/// clean-state toolbar hides Commit/Rollback, leaving disconnect/reconnect as
/// the only visible way out. A rollback of a read-only transaction and a fresh
/// BEGIN are both cheap metadata operations. The session-history gate excludes
/// any prior write or uncertain batch. Failures never fail the successful batch:
/// the session is torn down instead and the frontend's existing
/// rolled-back-session recovery transparently opens a new one on the next run.
async fn rotate_clean_read_only_snapshot(conn: &mut TxnConnection, schema: Option<&str>) -> Result<(), String> {
    match conn {
        TxnConnection::Mysql(Some(conn)) => {
            conn.query_drop("ROLLBACK").await.map_err(|e| format!("ROLLBACK failed: {e}"))?;
            conn.query_drop("START TRANSACTION").await.map_err(|e| format!("START TRANSACTION failed: {e}"))?;
            Ok(())
        }

        // Agent and external-driver sessions cannot reopen in place (their
        // rollback path closes the dedicated session), so they keep the
        // snapshot semantics; proven-read-only markers for those dialects do
        // not reach this helper's native variants in practice.
        _ => Ok(()),
    }
}

pub(crate) async fn rollback_manual_txn_connection(conn: &mut TxnConnection) -> Result<(), String> {
    rollback_manual_txn_connection_with_timeout(conn, None).await
}

/// A cancelled MySQL row stream may still have unread result packets.  Do not
/// send ROLLBACK on that connection: mysql_async only consumes a dropped stream
/// when the next result set is requested.  Discarding the dedicated connection
/// makes MySQL roll the transaction back and prevents protocol desynchronization.
async fn discard_mysql_manual_txn_connection(conn: &mut TxnConnection, timeout: Duration) -> Result<(), String> {
    let TxnConnection::Mysql(conn) = conn else {
        return rollback_manual_txn_connection_with_timeout(conn, Some(timeout)).await;
    };
    let Some(conn) = conn.take() else {
        return Ok(());
    };
    match tokio::time::timeout(timeout, conn.disconnect()).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(format!("Failed to discard cancelled MySQL connection: {error}")),
        Err(_) => Err(format!("Discarding cancelled MySQL connection timed out after {} seconds", timeout.as_secs())),
    }
}

async fn release_manual_txn_session_pool(state: &AppState, connection_id: &str, conn: &mut TxnConnection) {}

#[derive(Debug)]
enum ManualTxnAgentQueryRequest {
    Execute(serde_json::Value),
    ExecutePage(serde_json::Value),
    FetchPage(serde_json::Value),
}

/// Spawn a background task that removes and rolls back a transaction session
/// after 5 minutes of inactivity. The task does not hold the global lock across
/// I/O: it briefly checks the map, and if the session exists and is expired,
/// removes it, drops the lock, then rolls back the held connection.
///
/// Safety: if multiple watchers exist for the same session ID (e.g. due to
/// a race), only the one that actually finds the session in the map and
/// observes an elapsed time >= timeout will remove and roll back. Others
/// will see a missing session or a non-expired one and exit harmlessly.
fn spawn_txn_idle_watcher(state: &AppState, txn_session_id: String) {
    let sessions = Arc::clone(&state.transaction_sessions);
    spawn_txn_idle_watcher_for_sessions(sessions, txn_session_id);
}

fn spawn_txn_idle_watcher_for_sessions(
    sessions: Arc<tokio::sync::RwLock<std::collections::HashMap<String, TransactionSession>>>,
    txn_session_id: String,
) {
    tokio::spawn(async move {
        const TXN_IDLE_TIMEOUT: std::time::Duration =
            std::time::Duration::from_secs(MANUAL_TRANSACTION_IDLE_TIMEOUT_SECS);
        tokio::time::sleep(TXN_IDLE_TIMEOUT).await;

        let removed: Option<TransactionSession> = {
            let mut guard = sessions.write().await;
            match guard.get(&txn_session_id) {
                Some(session) if !session.busy && session.last_activity.elapsed() >= TXN_IDLE_TIMEOUT => {
                    guard.remove(&txn_session_id)
                }
                _ => None,
            }
        };

        if let Some(session) = removed {
            let mut conn = session.connection.lock().await;
            let _ = rollback_manual_txn_connection(&mut conn).await;
            log::info!(
                "[query][manual_txn:idle_timeout] session_id={} auto-rolled back after 5 minutes of inactivity",
                txn_session_id
            );
        }
    });
}

async fn execute_manual_txn_mysql_statement(
    conn: &mut mysql_async::Conn,
    sql: &str,
    row_limit: usize,
) -> Result<db::QueryResult, String> {
    if db::mysql::is_result_set_query(sql, db::mysql::MySqlQueryDialect::default()) {
        let start = std::time::Instant::now();
        let mut result = conn.query_iter(sql).await.map_err(|e| format!("Query failed: {e}"))?;
        let columns: Vec<String> = result.columns_ref().iter().map(|c| c.name_str().to_string()).collect();
        let column_types: Vec<String> = result.columns_ref().iter().map(db::mysql::mysql_column_type_name).collect();
        // Some statements only *may* return rows — `EXECUTE` of a prepared DML statement
        // finishes with a plain OK packet instead — so report the affected rows rather than
        // failing on the missing result set.
        if columns.is_empty() {
            let affected_rows = result.affected_rows();
            result.drop_result().await.map_err(|e| format!("Query failed: {e}"))?;
            return Ok(db::QueryResult {
                columns: Vec::new(),
                column_types: Vec::new(),
                column_sortables: vec![],
                spatial_columns: vec![],
                spatial_values: vec![],
                rows: vec![],
                affected_rows,
                execution_time_ms: start.elapsed().as_millis(),
                server_execute_time_us: None,
                query_timings_ms: None,
                truncated: false,
                session_id: None,
                has_more: false,
                elasticsearch_raw_body: None,
                messages: Vec::new(),
            });
        }
        let mut data: Vec<Vec<serde_json::Value>> = Vec::with_capacity(row_limit.min(1024));
        let mut stream = result
            .stream::<mysql_async::Row>()
            .await
            .map_err(|e| format!("Query failed: {e}"))?
            .ok_or_else(|| "Empty result set stream".to_string())?;
        let mut truncated = false;
        while let Some(row) = stream.next().await {
            if data.len() >= row_limit {
                truncated = true;
                break;
            }
            let row = row.map_err(|e| format!("Query failed: {e}"))?;
            data.push((0..row.len()).map(|i| db::mysql::mysql_value_to_json(&row, i)).collect());
        }
        Ok(db::QueryResult {
            columns,
            column_types,
            column_sortables: vec![],
            spatial_columns: vec![],
            spatial_values: vec![],
            rows: data,
            affected_rows: 0,
            execution_time_ms: start.elapsed().as_millis(),
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        })
    } else {
        let result = conn.query_iter(sql).await.map_err(|e| format!("Query failed: {e}"))?;
        let affected_rows = result.affected_rows();
        result.drop_result().await.map_err(|e| format!("Query failed: {e}"))?;
        Ok(db::QueryResult {
            columns: vec![],
            column_types: Vec::new(),
            column_sortables: vec![],
            spatial_columns: vec![],
            spatial_values: vec![],
            rows: vec![],
            affected_rows,
            execution_time_ms: 0,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        })
    }
}

/// Commit an existing manual transaction session.
pub async fn commit_manual_transaction(state: &AppState, txn_session_id: &str) -> Result<db::QueryResult, String> {
    {}
    let session = {
        let mut sessions = state.transaction_sessions.write().await;
        sessions.remove(txn_session_id).ok_or("Transaction session not found")?
    };

    let mut conn = session.connection.lock().await;
    match &mut *conn {
        TxnConnection::Mysql(Some(conn)) => {
            conn.query_drop("COMMIT").await.map_err(|e| format!("COMMIT failed: {e}"))?;
        }
        TxnConnection::Mysql(None) => return Err(MANUAL_TRANSACTION_SESSION_NOT_FOUND_ERROR.to_string()),
    }
    release_manual_txn_session_pool(state, &session.connection_id, &mut conn).await;

    log::info!("[query][manual_txn:commit] session_id={}", txn_session_id);
    Ok(db::QueryResult {
        columns: vec![],
        column_types: Vec::new(),
        column_sortables: vec![],
        spatial_columns: vec![],
        spatial_values: vec![],
        rows: vec![],
        affected_rows: 0,
        execution_time_ms: 0,
        server_execute_time_us: None,
        query_timings_ms: None,
        truncated: false,
        session_id: None,
        has_more: false,
        elasticsearch_raw_body: None,
        messages: Vec::new(),
    })
}

/// Rollback an existing manual transaction session.
pub async fn rollback_manual_transaction(state: &AppState, txn_session_id: &str) -> Result<db::QueryResult, String> {
    {}
    let session = {
        let mut sessions = state.transaction_sessions.write().await;
        sessions.remove(txn_session_id).ok_or("Transaction session not found")?
    };

    let mut conn = session.connection.lock().await;
    rollback_manual_txn_connection(&mut conn).await?;
    release_manual_txn_session_pool(state, &session.connection_id, &mut conn).await;

    log::info!("[query][manual_txn:rollback] session_id={}", txn_session_id);
    Ok(db::QueryResult {
        columns: vec![],
        column_types: Vec::new(),
        column_sortables: vec![],
        spatial_columns: vec![],
        spatial_values: vec![],
        rows: vec![],
        affected_rows: 0,
        execution_time_ms: 0,
        server_execute_time_us: None,
        query_timings_ms: None,
        truncated: false,
        session_id: None,
        has_more: false,
        elasticsearch_raw_body: None,
        messages: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_cancel::RunningTaskMetadata;
    use std::sync::atomic::Ordering;
    use tokio::time::timeout;

    #[test]
    fn manual_txn_batch_proven_read_only_requires_non_empty_all_proven_results() {
        // Empty batch (e.g. comments-only script) must not rotate: there is no
        // snapshot to renew and rotating would churn a BEGIN for nothing.
        assert!(!manual_txn_batch_fully_proven_read_only(&[]));

        let unproven =
            vec![ExecuteMultiResult::success_with_optional_server_large_values(empty_query_result(1), false)];
        assert!(!manual_txn_batch_fully_proven_read_only(&unproven));

        let proven = unproven
            .clone()
            .into_iter()
            .map(|result| result.with_manual_transaction_proven_read_only())
            .collect::<Vec<_>>();
        assert!(manual_txn_batch_fully_proven_read_only(&proven));

        // One write statement anywhere in the batch keeps the transaction.
        let mixed = vec![
            ExecuteMultiResult::success_with_optional_server_large_values(empty_query_result(1), false)
                .with_manual_transaction_proven_read_only(),
            ExecuteMultiResult::success_with_optional_server_large_values(empty_query_result(1), false),
        ];
        assert!(!manual_txn_batch_fully_proven_read_only(&mixed));
    }

    #[test]
    fn apply_query_timeout_override_respects_resolve_semantics() {
        // None leaves the budget unchanged.
        let mut budget = DbOperationBudget::with_defaults();
        let original = budget.query_timeout;
        apply_query_timeout_override(&mut budget, None);
        assert_eq!(budget.query_timeout, original);

        // Some(5) sets query_timeout to 5s.
        let mut budget = DbOperationBudget::with_defaults();
        apply_query_timeout_override(&mut budget, Some(5));
        assert_eq!(budget.query_timeout, Some(Duration::from_secs(5)));

        // Some(0) clears the limit (unlimited), matching resolve_query_timeout.
        let mut budget = DbOperationBudget::with_defaults();
        apply_query_timeout_override(&mut budget, Some(0));
        assert_eq!(budget.query_timeout, None);
    }

    #[tokio::test]
    async fn stream_progress_timeout_survives_steady_progress_past_the_budget() {
        // The timeout is an inactivity window, not a wall clock: a stream that keeps
        // marking progress survives well past the budget, as long as each gap between
        // marks is shorter than the timeout.
        let clock = Arc::new(StreamProgressClock::new());
        let clock_for_rows = clock.clone();
        let result = await_stream_with_progress_timeout(
            async move {
                for _ in 0..20 {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    clock_for_rows.mark();
                }
                Ok::<_, String>(42)
            },
            Some(Duration::from_millis(200)),
            clock,
            None,
            "timed out".to_string(),
        )
        .await;
        assert_eq!(result, Ok(42));
    }

    #[tokio::test]
    async fn stream_progress_timeout_fires_when_no_progress_arrives() {
        // A genuine stall — no progress for the whole budget — must still time out.
        let clock = Arc::new(StreamProgressClock::new());
        let result = await_stream_with_progress_timeout(
            async {
                tokio::time::sleep(Duration::from_millis(600)).await;
                Ok::<_, String>(42)
            },
            Some(Duration::from_millis(200)),
            clock,
            None,
            "timed out".to_string(),
        )
        .await;
        assert_eq!(result, Err("timed out".to_string()));
    }

    #[test]
    fn execute_multi_result_manual_transaction_markers_serialize_conditionally() {
        let plain = ExecuteMultiResult::success_with_optional_server_large_values(empty_query_result(0), false);
        let plain_value = serde_json::to_value(&plain).unwrap();
        assert!(plain_value.get("manual_transaction_proven_read_only").is_none());
        assert!(plain_value.get("manual_transaction_no_statement").is_none());

        let proven = plain.clone().with_manual_transaction_proven_read_only();
        let proven_value = serde_json::to_value(&proven).unwrap();
        assert_eq!(proven_value.get("manual_transaction_proven_read_only"), Some(&serde_json::Value::Bool(true)));
        assert!(proven_value.get("manual_transaction_no_statement").is_none());

        let no_statement = plain.with_manual_transaction_no_statement();
        let no_statement_value = serde_json::to_value(&no_statement).unwrap();
        assert_eq!(no_statement_value.get("manual_transaction_no_statement"), Some(&serde_json::Value::Bool(true)));
        assert!(no_statement_value.get("manual_transaction_proven_read_only").is_none());
    }

    #[test]
    fn schema_diff_destructive_detection_covers_drop_and_alter_drop() {
        assert!(is_destructive_schema_diff_statement("DROP INDEX idx_users_email ON users"));
        assert!(is_destructive_schema_diff_statement("TRUNCATE TABLE audit_log"));
        assert!(is_destructive_schema_diff_statement(
            "ALTER TABLE users DROP COLUMN legacy_code, DROP INDEX idx_legacy"
        ));
    }

    #[test]
    fn schema_diff_destructive_detection_ignores_comments_literals_and_identifiers() {
        assert!(!is_destructive_schema_diff_statement("-- DROP INDEX idx_fake\nSELECT 1"));
        assert!(!is_destructive_schema_diff_statement("SELECT 'DROP TABLE users'"));
        assert!(!is_destructive_schema_diff_statement("ALTER TABLE \"DROP INDEX audit\" ADD COLUMN note TEXT"));
    }

    use crate::models::connection::{default_redis_key_separator, ConnectionConfig, DatabaseType};
    #[cfg(unix)]
    use crate::plugins::{
        InstalledPlugin, PluginCompatibility, PluginDriverManifest, PluginDriverSession, PluginManifest,
        PluginRuntimeEnv,
    };

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_multi_agent_zero_timeout_waits_for_response() {
        let (mut client, _script) = spawn_agent_batch_timeout_test_client().await;

        let result = execute_multi_agent(&mut client, None, &["fast".to_string()], None, Some(0)).await.unwrap();

        assert_eq!(result.affected_rows, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_multi_agent_positive_timeout_still_expires() {
        let (mut client, _script) = spawn_agent_batch_timeout_test_client().await;

        let error = execute_multi_agent(&mut client, None, &["slow".to_string()], None, Some(1)).await.unwrap_err();

        assert!(matches!(
            error,
            AgentCallError::Timeout {
                stage: AgentErrorStage::Execute,
                operation_outcome: AgentOperationOutcome::Unknown,
            }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_multi_agent_default_timeout_keeps_normal_execution() {
        let (mut client, _script) = spawn_agent_batch_timeout_test_client().await;

        let result = execute_multi_agent(&mut client, None, &["fast".to_string()], None, None).await.unwrap();

        assert_eq!(result.affected_rows, 1);
        assert_eq!(resolve_query_timeout(None), Some(QUERY_TIMEOUT));
    }

    #[test]
    fn external_catalog_queries_do_not_bind_database_during_pool_creation() {
        assert_eq!(query_pool_database("bi", Some("paimon_catalog")), None);
        assert_eq!(query_pool_database("bi", None), Some("bi"));
        assert_eq!(query_pool_database("", None), None);
    }

    #[test]
    fn schema_diff_atomicity_marks_mysql_ddl_as_partial() {
        let atomicity = classify_schema_diff_atomicity(
            Some(DatabaseType::Mysql),
            &["CREATE TABLE users (id INT)".to_string(), "ALTER TABLE users ADD COLUMN name VARCHAR(32)".to_string()],
            true,
        );

        assert_eq!(atomicity, SchemaDiffAtomicity::PartialEffectsPossible);
    }

    #[test]
    fn schema_diff_atomicity_keeps_dml_atomic_when_tx_path_exists() {
        let atomicity = classify_schema_diff_atomicity(
            Some(DatabaseType::Mysql),
            &["INSERT INTO users VALUES (1)".to_string(), "UPDATE users SET active = 1 WHERE id = 1".to_string()],
            true,
        );

        assert_eq!(atomicity, SchemaDiffAtomicity::GuaranteedRollback);
    }

    #[test]
    fn executed_count_before_error_uses_failing_statement_index() {
        assert_eq!(executed_count_before_error("Statement 1 failed: syntax error", 3), 0);
        assert_eq!(executed_count_before_error("Statement 2 failed: syntax error", 3), 1);
        assert_eq!(executed_count_before_error("Statement 3 failed: syntax error", 3), 2);
    }

    #[test]
    fn schema_diff_failure_outcome_rolls_back_when_atomic() {
        let (status, executed) =
            schema_diff_failure_outcome(SchemaDiffAtomicity::GuaranteedRollback, "Statement 2 failed: syntax error", 3);
        assert_eq!(status, crate::two_phase_commit::TransactionStatus::RolledBack);
        assert_eq!(executed, 0);
    }

    #[test]
    fn schema_diff_failure_outcome_reports_mixed_with_partial_count() {
        let (status, executed) = schema_diff_failure_outcome(
            SchemaDiffAtomicity::PartialEffectsPossible,
            "Statement 2 failed: syntax error",
            3,
        );
        assert_eq!(status, crate::two_phase_commit::TransactionStatus::Mixed);
        assert_eq!(executed, 1);
    }

    /// MySQL: first DDL may already commit; second fails → mixed + executed_count = 1.
    #[test]
    fn mysql_second_ddl_failure_maps_to_mixed_with_partial_executed_count() {
        let stmts = ["CREATE TABLE t1 (id INT)".to_string(), "CREATE TABLE t2 (id INT)".to_string()];
        let atomicity = classify_schema_diff_atomicity(Some(DatabaseType::Mysql), &stmts, true);
        assert_eq!(atomicity, SchemaDiffAtomicity::PartialEffectsPossible);
        let (status, executed) =
            schema_diff_failure_outcome(atomicity, "Statement 2 failed: table already exists", stmts.len());
        assert_eq!(status, crate::two_phase_commit::TransactionStatus::Mixed);
        assert_eq!(executed, 1);
    }

    #[test]
    fn query_execution_mode_deserializes_simple_client_value() {
        let mode: QueryExecutionMode = serde_json::from_str("\"simple\"").unwrap();

        assert_eq!(mode, QueryExecutionMode::Simple);
        assert_eq!(QueryExecutionMode::default(), QueryExecutionMode::Standard);
    }

    fn test_connection_config(db_type: DatabaseType) -> ConnectionConfig {
        ConnectionConfig {
            oracle_oci_nls_lang: None,
            oracle_oci_tns_admin: None,
            docs_notes_path: None,
            id: "conn-1".to_string(),
            name: "Connection".to_string(),
            note: String::new(),
            db_type,
            driver_profile: None,
            driver_label: None,
            url_params: None,
            agent_java_options: Vec::new(),
            host: "localhost".to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
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
            connect_timeout_secs: 10,
            query_timeout_secs: 30,
            idle_timeout_secs: 60,
            keepalive_interval_secs: 30,
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

    struct FakeMysqlBatchExecutor {
        outcomes: std::collections::VecDeque<Result<Vec<db::mysql::MySqlQueryResult>, String>>,
        executed: Vec<String>,
    }

    impl MysqlBatchStatementExecutor for FakeMysqlBatchExecutor {
        async fn execute_statement(&mut self, statement: &str) -> Result<Vec<db::mysql::MySqlQueryResult>, String> {
            self.executed.push(statement.to_string());
            self.outcomes.pop_front().expect("test outcome for statement")
        }
    }

    fn mysql_query_result(result: db::QueryResult) -> db::mysql::MySqlQueryResult {
        db::mysql::MySqlQueryResult { result, large_value_cells: Vec::new() }
    }

    fn mysql_batch_result(result: db::QueryResult) -> Result<Vec<db::mysql::MySqlQueryResult>, String> {
        Ok(vec![mysql_query_result(result)])
    }

    struct FakePipelinedMysqlBatchExecutor {
        batch_outcomes: std::collections::VecDeque<db::mysql::MySqlNonResultBatchOutcome>,
        statement_outcomes: std::collections::VecDeque<Result<Vec<db::mysql::MySqlQueryResult>, String>>,
        batches: Vec<Vec<String>>,
        statements: Vec<String>,
    }

    impl MysqlBatchStatementExecutor for FakePipelinedMysqlBatchExecutor {
        async fn execute_statement(&mut self, statement: &str) -> Result<Vec<db::mysql::MySqlQueryResult>, String> {
            self.statements.push(statement.to_string());
            self.statement_outcomes.pop_front().expect("test outcome for single statement")
        }

        async fn execute_non_result_batch(
            &mut self,
            statements: &[String],
            on_result: &mut (dyn FnMut(usize, &db::QueryResult) + Send),
        ) -> db::mysql::MySqlNonResultBatchOutcome {
            self.batches.push(statements.to_vec());
            let outcome = self.batch_outcomes.pop_front().expect("test outcome for pipelined statements");
            for (statement_index, result) in outcome.results.iter().enumerate() {
                on_result(statement_index, result);
            }
            outcome
        }
    }

    #[test]
    fn batch_has_concurrently_statement_detects_transaction_block_escapees() {
        assert!(batch_has_concurrently_statement(&["CREATE INDEX CONCURRENTLY idx ON t (id)".to_string(),]));
        assert!(batch_has_concurrently_statement(&[
            "ALTER TABLE parent DETACH PARTITION child CONCURRENTLY".to_string(),
        ]));
        // Case-insensitive, and a single CONCURRENTLY anywhere in the batch is enough.
        assert!(batch_has_concurrently_statement(&[
            "ALTER TABLE t ADD COLUMN c int".to_string(),
            "drop index concurrently idx".to_string(),
        ]));
        assert!(!batch_has_concurrently_statement(&[
            "CREATE TABLE child PARTITION OF parent FOR VALUES FROM (0) TO (1)".to_string(),
        ]));
        // The word inside literals or plain identifiers must not demote a batch
        // that would otherwise run in one transaction.
        assert!(!batch_has_concurrently_statement(&[
            "INSERT INTO notes (body) VALUES ('run CREATE INDEX CONCURRENTLY later')".to_string(),
        ]));
        assert!(!batch_has_concurrently_statement(&[
            "SELECT id FROM concurrently WHERE label = 'drop index concurrently idx'".to_string(),
        ]));
        assert!(!batch_has_concurrently_statement(&[
            "-- refresh materialized view concurrently next".to_string(),
            "SELECT 1".to_string(),
        ]));
        // REINDEX/REFRESH forms still detect.
        assert!(batch_has_concurrently_statement(&["REINDEX TABLE CONCURRENTLY t".to_string()]));
        assert!(batch_has_concurrently_statement(&["REFRESH MATERIALIZED VIEW CONCURRENTLY mv".to_string()]));
    }

    #[tokio::test]
    async fn mysql_batch_stops_after_the_first_statement_error() {
        let statements = vec!["first".to_string(), "fails".to_string(), "must-not-run".to_string()];
        let mut executor = FakeMysqlBatchExecutor {
            outcomes: std::collections::VecDeque::from([
                mysql_batch_result(empty_query_result(0)),
                Err("Duplicate entry".to_string()),
                mysql_batch_result(empty_query_result(0)),
            ]),
            executed: Vec::new(),
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            false,
            None,
            None,
        )
        .await;

        assert_eq!(executor.executed, vec!["first", "fails"]);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].statement_index, Some(0));
        assert_eq!(results[1].statement_index, Some(1));
        assert!(results[1].execution_error);
        assert_eq!(error_action, Some(PoolErrorAction::Keep));
    }

    #[tokio::test]
    async fn mysql_batch_reports_progress_for_each_completed_statement() {
        let statements = vec!["first".to_string(), "fails".to_string(), "must-not-run".to_string()];
        let mut executor = FakeMysqlBatchExecutor {
            outcomes: std::collections::VecDeque::from([
                mysql_batch_result(empty_query_result(0)),
                Err("Duplicate entry".to_string()),
            ]),
            executed: Vec::new(),
        };
        let progress_events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress: ExecuteMultiProgressCallback = {
            let progress_events = Arc::clone(&progress_events);
            Arc::new(move |event| progress_events.lock().unwrap().push(event))
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            false,
            None,
            Some(&progress),
        )
        .await;

        assert_eq!(executor.executed, vec!["first", "fails"]);
        assert_eq!(results.len(), 2);
        assert_eq!(
            *progress_events.lock().unwrap(),
            vec![
                ExecuteMultiProgress {
                    statement_index: 0,
                    completed: 1,
                    total: 3,
                    success: true,
                    execution_time_ms: 0,
                    affected_rows: 0,
                    error: None,
                },
                ExecuteMultiProgress {
                    statement_index: 1,
                    completed: 2,
                    total: 3,
                    success: false,
                    execution_time_ms: 0,
                    affected_rows: 0,
                    error: Some(crate::backend_error::BackendError::from_sql_detail("Duplicate entry")),
                },
            ]
        );
        assert_eq!(error_action, Some(PoolErrorAction::Keep));
    }

    #[tokio::test]
    async fn mysql_batch_pipelines_adjacent_non_result_statements() {
        let statements = vec![
            "SET @batch = 1".to_string(),
            "INSERT INTO users(id) VALUES (1)".to_string(),
            "INSERT INTO users(id) VALUES (2)".to_string(),
            "SELECT COUNT(*) FROM users".to_string(),
        ];
        let mut executor = FakePipelinedMysqlBatchExecutor {
            batch_outcomes: std::collections::VecDeque::from([db::mysql::MySqlNonResultBatchOutcome {
                results: vec![empty_query_result(2), empty_query_result(3)],
                error: None,
            }]),
            statement_outcomes: std::collections::VecDeque::from([
                mysql_batch_result(empty_query_result(1)),
                mysql_batch_result(db::QueryResult {
                    columns: vec!["COUNT(*)".to_string()],
                    column_types: vec!["BIGINT".to_string()],
                    column_sortables: vec![],
                    spatial_columns: vec![],
                    spatial_values: vec![],
                    rows: vec![vec![serde_json::json!(2)]],
                    affected_rows: 0,
                    execution_time_ms: 4,
                    server_execute_time_us: None,
                    query_timings_ms: None,
                    truncated: false,
                    session_id: None,
                    has_more: false,
                    elasticsearch_raw_body: None,
                    messages: Vec::new(),
                }),
            ]),
            batches: Vec::new(),
            statements: Vec::new(),
        };
        let progress_events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress: ExecuteMultiProgressCallback = {
            let progress_events = Arc::clone(&progress_events);
            Arc::new(move |event| progress_events.lock().unwrap().push(event))
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            false,
            Some(MYSQL_MULTI_STATEMENT_BATCH_MAX_BYTES),
            Some(&progress),
        )
        .await;

        assert_eq!(executor.batches, vec![statements[1..3].to_vec()]);
        assert_eq!(executor.statements, vec![statements[0].clone(), statements[3].clone()]);
        assert_eq!(results.len(), 4);
        assert_eq!(
            results.iter().map(|result| result.statement_index).collect::<Vec<_>>(),
            vec![Some(0), Some(1), Some(2), Some(3)]
        );
        assert_eq!(
            progress_events.lock().unwrap().iter().map(|event| event.completed).collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(error_action, None);
    }

    #[tokio::test]
    async fn mysql_pipelined_batch_reports_the_first_failed_statement() {
        let statements = vec![
            "INSERT INTO users(id) VALUES (1)".to_string(),
            "INSERT INTO users(id) VALUES (1)".to_string(),
            "INSERT INTO users(id) VALUES (2)".to_string(),
        ];
        let mut executor = FakePipelinedMysqlBatchExecutor {
            batch_outcomes: std::collections::VecDeque::from([db::mysql::MySqlNonResultBatchOutcome {
                results: vec![empty_query_result(1)],
                error: Some("Duplicate entry".to_string()),
            }]),
            statement_outcomes: std::collections::VecDeque::new(),
            batches: Vec::new(),
            statements: Vec::new(),
        };
        let progress_events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress: ExecuteMultiProgressCallback = {
            let progress_events = Arc::clone(&progress_events);
            Arc::new(move |event| progress_events.lock().unwrap().push(event))
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            false,
            Some(MYSQL_MULTI_STATEMENT_BATCH_MAX_BYTES),
            Some(&progress),
        )
        .await;

        assert_eq!(executor.batches, vec![statements]);
        assert!(executor.statements.is_empty());
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].statement_index, Some(0));
        assert_eq!(results[1].statement_index, Some(1));
        assert!(results[1].execution_error);
        assert_eq!(results[1].error.as_ref().map(|error| error.code()), Some("DBX-JDBC-4001"));
        assert_eq!(
            progress_events
                .lock()
                .unwrap()
                .iter()
                .map(|event| (event.statement_index, event.success))
                .collect::<Vec<_>>(),
            vec![(0, true), (1, false)]
        );
        assert_eq!(error_action, Some(PoolErrorAction::Keep));
    }

    #[tokio::test]
    async fn mysql_pipelined_batch_discards_a_cancelled_connection() {
        let statements =
            vec!["INSERT INTO users(id) VALUES (1)".to_string(), "INSERT INTO users(id) VALUES (2)".to_string()];
        let mut executor = FakePipelinedMysqlBatchExecutor {
            batch_outcomes: std::collections::VecDeque::from([db::mysql::MySqlNonResultBatchOutcome {
                results: Vec::new(),
                error: Some(QUERY_CANCELED.to_string()),
            }]),
            statement_outcomes: std::collections::VecDeque::new(),
            batches: Vec::new(),
            statements: Vec::new(),
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            false,
            Some(MYSQL_MULTI_STATEMENT_BATCH_MAX_BYTES),
            None,
        )
        .await;

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].statement_index, Some(0));
        assert!(results[0].execution_error);
        assert_eq!(error_action, Some(PoolErrorAction::Discard));
    }

    #[tokio::test]
    async fn mysql_pipelined_batch_does_not_add_a_failure_after_every_statement_completed() {
        let statements =
            vec!["INSERT INTO users(id) VALUES (1)".to_string(), "INSERT INTO users(id) VALUES (2)".to_string()];
        let mut executor = FakePipelinedMysqlBatchExecutor {
            batch_outcomes: std::collections::VecDeque::from([db::mysql::MySqlNonResultBatchOutcome {
                results: vec![empty_query_result(1), empty_query_result(1)],
                error: Some(QUERY_CANCELED.to_string()),
            }]),
            statement_outcomes: std::collections::VecDeque::new(),
            batches: Vec::new(),
            statements: Vec::new(),
        };
        let progress_events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let progress: ExecuteMultiProgressCallback = {
            let progress_events = Arc::clone(&progress_events);
            Arc::new(move |event| progress_events.lock().unwrap().push(event))
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            false,
            Some(MYSQL_MULTI_STATEMENT_BATCH_MAX_BYTES),
            Some(&progress),
        )
        .await;

        assert_eq!(results.len(), statements.len());
        assert!(results.iter().all(|result| !result.execution_error));
        assert_eq!(results.iter().map(|result| result.statement_index).collect::<Vec<_>>(), vec![Some(0), Some(1)]);
        assert_eq!(
            progress_events
                .lock()
                .unwrap()
                .iter()
                .map(|event| (event.statement_index, event.completed, event.total, event.success))
                .collect::<Vec<_>>(),
            vec![(0, 1, 2, true), (1, 2, 2, true)]
        );
        assert_eq!(error_action, Some(PoolErrorAction::Discard));
    }

    #[test]
    fn mysql_non_result_batches_respect_the_statement_limit() {
        let statements = (0..51).map(|index| format!("INSERT INTO users(id) VALUES ({index})")).collect::<Vec<_>>();

        assert_eq!(
            mysql_non_result_batch_end(
                &statements,
                0,
                db::mysql::MySqlQueryDialect::default(),
                MYSQL_MULTI_STATEMENT_BATCH_MAX_BYTES,
            ),
            MYSQL_MULTI_STATEMENT_BATCH_MAX_STATEMENTS
        );
        assert_eq!(
            mysql_non_result_batch_end(
                &statements,
                MYSQL_MULTI_STATEMENT_BATCH_MAX_STATEMENTS,
                db::mysql::MySqlQueryDialect::default(),
                MYSQL_MULTI_STATEMENT_BATCH_MAX_BYTES,
            ),
            51
        );
    }

    #[test]
    fn mysql_non_result_batches_respect_the_byte_limit() {
        let payload = "x".repeat(1_500_000);
        let statements = (0..3)
            .map(|index| format!("INSERT INTO users(id, payload) VALUES ({index}, '{payload}')"))
            .collect::<Vec<_>>();

        assert_eq!(
            mysql_non_result_batch_end(
                &statements,
                0,
                db::mysql::MySqlQueryDialect::default(),
                MYSQL_MULTI_STATEMENT_BATCH_MAX_BYTES,
            ),
            2
        );
    }

    #[test]
    fn mysql_single_call_uses_multi_result_route() {
        assert!(mysql_single_statement_uses_batch_route(
            Some(DatabaseType::Mysql),
            true,
            "# generated call\nCALL testA()",
            None,
        ));
    }

    #[test]
    fn ordinary_mysql_single_statements_keep_singular_route() {
        for sql in ["SELECT 1", "SHOW TABLES", "UPDATE users SET active = 1"] {
            assert!(!mysql_single_statement_uses_batch_route(Some(DatabaseType::Mysql), true, sql, None));
        }
    }

    #[test]
    fn mysql_result_byte_limit_keeps_existing_batch_route() {
        assert!(mysql_single_statement_uses_batch_route(
            Some(DatabaseType::Mysql),
            true,
            "SELECT * FROM users",
            Some(1024),
        ));
        assert!(!mysql_single_statement_uses_batch_route(
            Some(DatabaseType::Mysql),
            true,
            "SELECT * FROM users",
            Some(0),
        ));
    }

    #[tokio::test]
    async fn mysql_batch_preserves_multiple_result_sets_from_one_statement() {
        let statements = vec!["CALL testA()".to_string(), "UPDATE users SET active = 1".to_string()];
        let result_set = |value| db::QueryResult {
            columns: vec!["value".to_string()],
            column_types: vec!["INT".to_string()],
            column_sortables: vec![],
            spatial_columns: vec![],
            spatial_values: vec![],
            rows: vec![vec![serde_json::json!(value)]],
            affected_rows: 0,
            execution_time_ms: 1,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        };
        let mut executor = FakeMysqlBatchExecutor {
            outcomes: std::collections::VecDeque::from([
                Ok(vec![
                    mysql_query_result(result_set(1)),
                    mysql_query_result(result_set(2)),
                    mysql_query_result(result_set(3)),
                ]),
                mysql_batch_result(empty_query_result(1)),
            ]),
            executed: Vec::new(),
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            false,
            None,
            None,
        )
        .await;

        assert_eq!(executor.executed, statements);
        assert_eq!(results.len(), 4);
        assert_eq!(
            results.iter().map(|result| result.statement_index).collect::<Vec<_>>(),
            vec![Some(0), Some(0), Some(0), Some(1)]
        );
        assert_eq!(
            results[..3].iter().map(|result| result.result.rows[0][0].clone()).collect::<Vec<_>>(),
            vec![serde_json::json!(1), serde_json::json!(2), serde_json::json!(3)]
        );
        assert_eq!(error_action, None);
    }

    #[tokio::test]
    async fn mysql_batch_stops_when_the_first_statement_fails() {
        let statements = vec!["fails".to_string(), "must-not-run".to_string()];
        let mut executor = FakeMysqlBatchExecutor {
            outcomes: std::collections::VecDeque::from([
                Err("Duplicate entry".to_string()),
                mysql_batch_result(empty_query_result(0)),
            ]),
            executed: Vec::new(),
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            false,
            None,
            None,
        )
        .await;

        assert_eq!(executor.executed, vec!["fails"]);
        assert_eq!(results.len(), 1);
        assert!(results[0].execution_error);
        assert_eq!(error_action, Some(PoolErrorAction::Keep));
    }

    #[tokio::test]
    async fn mysql_batch_continues_after_statement_errors_when_enabled() {
        let statements = vec!["first".to_string(), "fails".to_string(), "third".to_string()];
        let mut executor = FakeMysqlBatchExecutor {
            outcomes: std::collections::VecDeque::from([
                mysql_batch_result(empty_query_result(0)),
                Err("Duplicate entry".to_string()),
                mysql_batch_result(empty_query_result(0)),
            ]),
            executed: Vec::new(),
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            true,
            None,
            None,
        )
        .await;

        assert_eq!(executor.executed, statements);
        assert_eq!(results.len(), 3);
        assert_eq!(
            results.iter().map(|result| result.statement_index).collect::<Vec<_>>(),
            vec![Some(0), Some(1), Some(2)]
        );
        assert!(results[1].execution_error);
        assert_eq!(error_action, None);
    }

    #[tokio::test]
    async fn mysql_batch_continues_when_the_first_statement_fails_and_enabled() {
        let statements = vec!["fails".to_string(), "second".to_string()];
        let mut executor = FakeMysqlBatchExecutor {
            outcomes: std::collections::VecDeque::from([
                Err("Duplicate entry".to_string()),
                mysql_batch_result(empty_query_result(0)),
            ]),
            executed: Vec::new(),
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            true,
            None,
            None,
        )
        .await;

        assert_eq!(executor.executed, statements);
        assert_eq!(results.len(), 2);
        assert!(results[0].execution_error);
        assert_eq!(error_action, None);
    }

    #[tokio::test]
    async fn mysql_batch_stops_on_connection_errors_when_continue_is_enabled() {
        let statements = vec!["first".to_string(), "disconnects".to_string(), "must-not-run".to_string()];
        let mut executor = FakeMysqlBatchExecutor {
            outcomes: std::collections::VecDeque::from([
                mysql_batch_result(empty_query_result(0)),
                Err("connection reset by peer".to_string()),
                mysql_batch_result(empty_query_result(0)),
            ]),
            executed: Vec::new(),
        };

        let (results, error_action) = execute_mysql_batch_statements(
            &mut executor,
            &statements,
            Some(DatabaseType::Mysql),
            db::mysql::MySqlQueryDialect::default(),
            None,
            true,
            None,
            None,
        )
        .await;

        assert_eq!(executor.executed, vec!["first", "disconnects"]);
        assert_eq!(results.len(), 2);
        assert_eq!(results.iter().map(|result| result.statement_index).collect::<Vec<_>>(), vec![Some(0), Some(1)]);
        assert!(results[1].execution_error);
        assert_eq!(results[1].error.as_ref().map(|error| error.code()), Some("DBX-LEGACY-0001"));
        assert_eq!(error_action, Some(PoolErrorAction::ReconnectAndRetry));
    }

    #[test]
    fn execute_multi_result_serializes_client_metadata_only_when_present() {
        let success = serde_json::to_value(ExecuteMultiResult::from(empty_query_result(0))).unwrap();
        assert!(success.get("execution_error").is_none());
        assert!(success.get("statement_index").is_none());
        assert!(success.get("server_message").is_none());
        assert!(success.get("large_value_cells").is_none());

        let mut error_column = empty_query_result(0);
        error_column.columns = vec!["Error".to_string()];
        error_column.rows = vec![vec![serde_json::json!("valid query value")]];
        let error_column = serde_json::to_value(ExecuteMultiResult::success_with_index(error_column, 0)).unwrap();
        assert!(error_column.get("execution_error").is_none());
        assert!(error_column.get("error").is_none());

        let failure = serde_json::to_value(ExecuteMultiResult::execution_error_with_index(
            error_query_result("failed".to_string()),
            2,
        ))
        .unwrap();
        assert_eq!(failure.get("execution_error"), Some(&serde_json::Value::Bool(true)));
        assert_eq!(failure.get("statement_index"), Some(&serde_json::json!(2)));
        assert_eq!(failure.get("columns"), Some(&serde_json::json!(["Error"])));
        assert_eq!(
            failure.get("error").and_then(|value| value.get("code")),
            Some(&serde_json::json!("DBX-LEGACY-0001"))
        );

        let redacted = serde_json::to_value(
            ExecuteMultiResult::execution_error_with_index(error_query_result("safe failure".to_string()), 2)
                .without_error_detail(),
        )
        .unwrap();
        assert!(redacted.get("error").and_then(|value| value.get("detail")).is_none());
    }

    #[test]
    fn execute_multi_result_serializes_large_value_metadata_only_when_present() {
        let result = ExecuteMultiResult::success_with_index_and_large_values(
            empty_query_result(1),
            0,
            vec![db::LargeValueCell { row_index: 2, column_index: 3, original_bytes: 65_536 }],
            false,
        );

        let serialized = serde_json::to_value(result).unwrap();
        assert_eq!(
            serialized.get("large_value_cells"),
            Some(&serde_json::json!([{"row_index": 2, "column_index": 3, "original_bytes": 65_536}]))
        );
    }

    #[test]
    fn large_value_cell_merge_replaces_driver_entries_and_appends_server_entries() {
        let driver_cells = vec![
            db::LargeValueCell { row_index: 1, column_index: 2, original_bytes: 10 },
            db::LargeValueCell { row_index: 3, column_index: 4, original_bytes: 20 },
        ];
        let server_cells = vec![
            db::LargeValueCell { row_index: 1, column_index: 2, original_bytes: 100 },
            db::LargeValueCell { row_index: 5, column_index: 6, original_bytes: 200 },
        ];

        assert_eq!(
            merge_large_value_cells(driver_cells, server_cells),
            vec![
                db::LargeValueCell { row_index: 1, column_index: 2, original_bytes: 100 },
                db::LargeValueCell { row_index: 3, column_index: 4, original_bytes: 20 },
                db::LargeValueCell { row_index: 5, column_index: 6, original_bytes: 200 },
            ]
        );
    }

    #[test]
    fn large_value_cell_merge_preserves_single_source_inputs() {
        let driver_cell = db::LargeValueCell { row_index: 1, column_index: 2, original_bytes: 10 };
        let server_cell = db::LargeValueCell { row_index: 3, column_index: 4, original_bytes: 20 };

        assert_eq!(merge_large_value_cells(vec![driver_cell.clone()], Vec::new()), vec![driver_cell]);
        assert_eq!(merge_large_value_cells(Vec::new(), vec![server_cell.clone()]), vec![server_cell]);
    }

    #[test]
    fn large_value_cell_merge_handles_large_disjoint_inputs() {
        const CELL_COUNT: usize = 100_000;
        let driver_cells = (0..CELL_COUNT)
            .map(|row_index| db::LargeValueCell { row_index, column_index: 0, original_bytes: 10 })
            .collect();
        let server_cells = (0..CELL_COUNT)
            .map(|row_index| db::LargeValueCell { row_index, column_index: 1, original_bytes: 20 })
            .collect();

        let merged = merge_large_value_cells(driver_cells, server_cells);

        assert_eq!(merged.len(), CELL_COUNT * 2);
        assert_eq!(merged[CELL_COUNT].row_index, 0);
        assert_eq!(merged[CELL_COUNT * 2 - 1].row_index, CELL_COUNT - 1);
    }

    #[tokio::test]
    async fn lock_shared_client_with_wait_is_near_zero_when_uncontended() {
        let client = Arc::new(tokio::sync::Mutex::new(0u8));

        let (_guard, wait_ms) = lock_shared_client_with_wait(&client, None, None).await.unwrap();

        assert!(wait_ms < 50, "expected an uncontended lock to report negligible wait, got {wait_ms}ms");
    }

    #[test]
    fn legacy_rendering_strips_a_leftover_transport_marker() {
        // Guards the driver-error path that never went through the resolve step
        // (e.g. a PostgreSQL-family driver error shown as a legacy string).
        let error = QueryExecutionError::Legacy(format!(
            "ERROR: boom{}{}",
            crate::sql_error_position::encode_marker(9),
            crate::sql_error_position::encode_marker(2)
        ));
        let rendered = error.into_legacy_string();
        assert_eq!(rendered, "ERROR: boom");
        assert!(!rendered.contains(crate::sql_error_position::SQL_ERROR_POSITION_MARKER));
    }

    #[test]
    fn sequential_multi_cancellation_uses_canceled_catalog() {
        let backend_error = canceled_query_execution_error().into_backend_error();

        assert_eq!(backend_error.code(), "DBX-JDBC-2003");
        assert_eq!(backend_error.message_key(), "backendErrors.jdbc.operationCanceled");
    }

    #[tokio::test]
    async fn wait_for_query_returns_cancelled_when_token_is_cancelled() {
        let token = CancellationToken::new();
        token.cancel();

        let result = wait_for_query(Some(token), async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(db::QueryResult {
                columns: vec![],
                column_types: Vec::new(),
                column_sortables: vec![],
                spatial_columns: vec![],
                spatial_values: vec![],
                rows: vec![],
                affected_rows: 0,
                execution_time_ms: 0,
                server_execute_time_us: None,
                query_timings_ms: None,
                truncated: false,
                session_id: None,
                has_more: false,
                elasticsearch_raw_body: None,
                messages: Vec::new(),
            })
        })
        .await;

        assert_eq!(result.unwrap_err(), QUERY_CANCELED);
    }

    #[tokio::test]
    async fn wait_for_query_without_token_still_times_out() {
        let result = wait_for_query_with_timeout(None, Duration::from_millis(10), async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(db::QueryResult {
                columns: vec![],
                column_types: Vec::new(),
                column_sortables: vec![],
                spatial_columns: vec![],
                spatial_values: vec![],
                rows: vec![],
                affected_rows: 0,
                execution_time_ms: 0,
                server_execute_time_us: None,
                query_timings_ms: None,
                truncated: false,
                session_id: None,
                has_more: false,
                elasticsearch_raw_body: None,
                messages: Vec::new(),
            })
        })
        .await;

        assert_eq!(result.unwrap_err(), timeout_error_for(Duration::from_millis(10)));
    }

    #[tokio::test]
    async fn wait_for_value_opt_times_out_while_waiting_for_lock() {
        let lock = tokio::sync::Mutex::new(());
        let _guard = lock.lock().await;

        let result = wait_for_value_opt(None, Some(Duration::from_millis(10)), lock.lock()).await;

        assert_eq!(result.unwrap_err(), timeout_error_for(Duration::from_millis(10)));
    }

    #[tokio::test]
    async fn wait_for_value_opt_can_cancel_while_waiting_for_lock() {
        let lock = tokio::sync::Mutex::new(());
        let _guard = lock.lock().await;
        let token = CancellationToken::new();
        token.cancel();

        let result = wait_for_value_opt(Some(token), Some(Duration::from_secs(30)), lock.lock()).await;

        assert_eq!(result.unwrap_err(), QUERY_CANCELED);
    }

    #[test]
    fn db_operation_budget_from_config() {
        let budget = DbOperationBudget::from_config(10, Some(30));
        assert_eq!(budget.checkout_timeout, Duration::from_secs(10));
        assert_eq!(budget.connect_timeout, Duration::from_secs(10));
        assert_eq!(budget.recycle_timeout, Duration::from_secs(10));
        assert_eq!(budget.query_timeout, Some(Duration::from_secs(30)));
        assert_eq!(budget.cancel_timeout, Duration::from_secs(5));
        assert_eq!(budget.cleanup_timeout, Duration::from_secs(3));
    }

    #[test]
    fn db_operation_budget_query_timeout_zero_means_no_limit() {
        let budget = DbOperationBudget::from_config(10, Some(0));
        assert_eq!(budget.query_timeout, None);
        // Infrastructure timeouts still have hard limits
        assert_eq!(budget.checkout_timeout, Duration::from_secs(10));
        assert_eq!(budget.cancel_timeout, Duration::from_secs(5));
    }

    #[test]
    fn db_operation_budget_query_timeout_zero_keeps_transaction_infra_limits() {
        let mut config = test_connection_config(DatabaseType::Mysql);
        config.connect_timeout_secs = 7;
        config.query_timeout_secs = 0;

        let budget = DbOperationBudget::from_connection_config(&config);

        assert_eq!(budget.query_timeout, None);
        assert_eq!(budget.checkout_timeout, Duration::from_secs(7));
        assert_eq!(budget.recycle_timeout, Duration::from_secs(7));
        assert_eq!(budget.cleanup_timeout, Duration::from_secs(3));
    }

    #[test]
    fn db_operation_budget_clamps_infra_timeout() {
        let budget = DbOperationBudget::from_config(0, Some(30));
        assert_eq!(budget.checkout_timeout, Duration::from_secs(1)); // clamped to min 1s
        let budget = DbOperationBudget::from_config(600, Some(30));
        assert_eq!(budget.checkout_timeout, Duration::from_secs(300)); // clamped to max 300s
    }

    #[test]
    fn db_operation_budget_with_defaults() {
        let budget = DbOperationBudget::with_defaults();
        assert_eq!(budget.checkout_timeout, db::connection_timeout());
        assert_eq!(budget.query_timeout, Some(QUERY_TIMEOUT));
    }

    #[test]
    fn is_connection_error_detects_english_messages() {
        assert!(is_connection_error("connection reset"));
        assert!(is_connection_error("broken pipe"));
        assert!(is_connection_error("reset by peer"));
        assert!(is_connection_error("Connection timed out"));
        assert!(is_connection_error("socket closed"));
        assert!(is_connection_error("unexpected eof"));
        assert!(is_connection_error("Error occurred while creating a new object: error communicating with the server"));
    }

    #[test]
    fn query_error_context_omits_raw_sql_and_is_not_duplicated() {
        let sql = "select 'secret-123' as token";
        let error = query_error_with_omitted_sql_context("driver rejected statement", sql);

        assert!(error.contains("driver rejected statement"));
        assert!(error.contains(SQL_OMITTED_ERROR_CONTEXT));
        assert!(!error.contains("secret-123"));
        assert!(!error.contains("SQL:"));

        let repeated = query_error_with_omitted_sql_context(&error, sql);
        assert_eq!(repeated.matches(SQL_OMITTED_ERROR_CONTEXT).count(), 1);
    }

    #[test]
    fn typed_sql_error_context_keeps_driver_text_on_one_line() {
        let error = QueryExecutionError::Sql("Server error: `ERROR 1064 (42000): syntax error`".to_string())
            .with_omitted_sql_context("SELECT * FROM users")
            .into_backend_error();

        assert_eq!(
            error.detail(),
            Some(
                "Server error: `ERROR 1064 (42000): syntax error` SQL text omitted from user-facing error; enable debug SQL diagnostics to inspect the original statement."
            )
        );
    }

    #[test]
    fn reconnect_retry_error_context_omits_raw_sql() {
        let sql = "select 'secret-123' as token";
        let reconnect_error = query_error_with_omitted_sql_context("connection reset after reconnect", sql);

        assert!(reconnect_error.contains("connection reset after reconnect"));
        assert!(reconnect_error.contains(SQL_OMITTED_ERROR_CONTEXT));
        assert!(!reconnect_error.contains("secret-123"));
    }

    #[test]
    fn execute_statements_error_omits_raw_sql() {
        let sql = "select 'secret-token' as t";
        let err = query_error_with_omitted_sql_context(
            &format!(
                "Statement {} failed: {}. Previous {} statement(s) may have been committed.",
                2, "driver error", 1
            ),
            sql,
        );

        assert!(err.contains("driver error"));
        assert!(err.contains(SQL_OMITTED_ERROR_CONTEXT));
        assert!(!err.contains("secret-token"));
        assert!(!err.contains("SQL:"));
        assert!(err.contains("Statement 2 failed:"));
    }

    #[test]
    fn batch_transaction_error_omits_raw_sql() {
        let sql = "delete from users where id = 'secret-id'";
        let err = query_error_with_omitted_sql_context(
            &format!("Statement {} failed: {}. No transaction support for this database type.", 3, "batch error"),
            sql,
        );

        assert!(err.contains("batch error"));
        assert!(err.contains(SQL_OMITTED_ERROR_CONTEXT));
        assert!(!err.contains("secret-id"));
        assert!(err.contains("Statement 3 failed:"));
    }

    #[test]
    fn is_connection_error_detects_localized_io_errors() {
        assert!(is_connection_error("I/O error: 远程主机强迫关闭了一个现有的连接。 (os error 10054)"));
        assert!(is_connection_error(
            "I/O error: 由于连接方在一段时间后没有正确答复或连接的主机没有反应，连接尝试失败。 (os error 10060)"
        ));
        assert!(is_connection_error("Agent RPC error (-1): dm.jdbc.driver.DMException: 网络通信异常"));
        assert!(is_connection_error(
            "Agent RPC error (-1): java.sql.SQLRecoverableException: IO 错误: Got minus one from a read call"
        ));
        assert!(is_connection_error(
            "Agent RPC error (-1): com.mysql.cj.jdbc.exceptions.CommunicationsException: Communications link failure"
        ));
    }

    #[test]
    fn is_connection_error_detects_os_error_codes() {
        assert!(is_connection_error("os error 10053"));
        assert!(is_connection_error("os error 10054"));
        assert!(is_connection_error("os error 10060"));
        assert!(is_connection_error("os error 10061"));
    }

    #[test]
    fn is_connection_error_rejects_non_connection_errors() {
        assert!(!is_connection_error("Query timed out after 30 seconds"));
        assert!(!is_connection_error("ORA-00942: table or view does not exist"));
        assert!(!is_connection_error("syntax error at position 5"));
        assert!(!is_connection_error("os error 13"));
    }

    #[test]
    fn ignores_non_single_drop_database_statements() {
        assert_eq!(parse_drop_database_target("DROP TABLE vaultwarden;"), None);
        assert_eq!(parse_drop_database_target("DROP DATABASE vaultwarden; SELECT 1;"), None);
        assert_eq!(parse_drop_database_target("DROP DATABASE 123bad;"), None);
    }

    #[test]
    fn query_results_convert_unsafe_json_integers_to_strings_for_js() {
        let result = db::QueryResult {
            columns: vec!["id".to_string(), "nested".to_string()],
            column_types: Vec::new(),
            column_sortables: vec![],
            spatial_columns: vec![],
            spatial_values: vec![],
            rows: vec![vec![
                serde_json::json!(2_041_797_190_226_354_178_i64),
                serde_json::json!([1, 2_041_797_190_226_354_178_i64]),
            ]],
            affected_rows: 0,
            execution_time_ms: 0,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        };

        let serialized = serde_json::to_value(result).unwrap();

        assert_eq!(serialized["rows"][0][0], serde_json::json!("2041797190226354178"));
        assert_eq!(serialized["rows"][0][1], serde_json::json!([1, "2041797190226354178"]));
    }

    #[test]
    fn extracts_server_large_value_markers_and_restores_source_columns() {
        let mut result = db::QueryResult {
            columns: vec![
                "id".to_string(),
                "payload".to_string(),
                format!("{}T_1", crate::sql_dialect::DBX_LARGE_VALUE_BYTES_COLUMN_PREFIX),
                "note".to_string(),
            ],
            column_types: vec!["integer".to_string(), "text".to_string(), "bigint".to_string(), "text".to_string()],
            column_sortables: vec![true, true, true, true],
            spatial_columns: Vec::new(),
            spatial_values: Vec::new(),
            rows: vec![
                vec![
                    serde_json::json!(1),
                    serde_json::json!("预览文本多"),
                    serde_json::json!("T:4"),
                    serde_json::json!("a"),
                ],
                vec![serde_json::json!(2), serde_json::json!("短值"), serde_json::json!("T:4"), serde_json::json!("b")],
            ],
            affected_rows: 0,
            execution_time_ms: 0,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        };

        let cells = extract_server_large_value_markers(&mut result);

        assert_eq!(result.columns, vec!["id", "payload", "note"]);
        assert_eq!(result.column_types, vec!["integer", "text", "text"]);
        assert_eq!(
            result.rows[0],
            vec![serde_json::json!(1), serde_json::json!("预览文本..."), serde_json::json!("a")]
        );
        assert_eq!(result.rows[1], vec![serde_json::json!(2), serde_json::json!("短值"), serde_json::json!("b")]);
        assert_eq!(
            cells,
            vec![db::LargeValueCell {
                row_index: 0,
                column_index: 1,
                original_bytes: SERVER_LARGE_VALUE_UNKNOWN_BYTES,
            }]
        );
    }

    #[test]
    fn extracts_lowercased_server_large_value_markers_from_case_folding_servers() {
        // KingbaseES instances with case-insensitive identifiers return even
        // quoted marker aliases folded to lowercase; the markers must still be
        // consumed and stripped instead of leaking into the data grid.
        let mut result = db::QueryResult {
            columns: vec![
                "id".to_string(),
                "payload".to_string(),
                "__dbx_large_value_bytes_t_1".to_string(),
                "note".to_string(),
                "__dbx_large_value_bytes_t_2".to_string(),
            ],
            column_types: vec![
                "integer".to_string(),
                "text".to_string(),
                "text".to_string(),
                "text".to_string(),
                "text".to_string(),
            ],
            column_sortables: vec![true; 5],
            spatial_columns: Vec::new(),
            spatial_values: Vec::new(),
            rows: vec![
                vec![
                    serde_json::json!(1),
                    serde_json::json!("长文本预览内容"),
                    serde_json::json!("T:4"),
                    serde_json::json!("abcdefgh"),
                    serde_json::json!("T:4"),
                ],
                vec![
                    serde_json::json!(2),
                    serde_json::json!("短值"),
                    serde_json::json!("T:4"),
                    serde_json::json!("b"),
                    serde_json::json!("T:4"),
                ],
            ],
            affected_rows: 0,
            execution_time_ms: 0,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        };

        let cells = extract_server_large_value_markers(&mut result);

        assert_eq!(result.columns, vec!["id", "payload", "note"]);
        assert_eq!(result.column_types, vec!["integer", "text", "text"]);
        assert_eq!(
            result.rows[0],
            vec![serde_json::json!(1), serde_json::json!("长文本预..."), serde_json::json!("abcd...")]
        );
        assert_eq!(result.rows[1], vec![serde_json::json!(2), serde_json::json!("短值"), serde_json::json!("b")]);
        assert_eq!(
            cells,
            vec![
                db::LargeValueCell { row_index: 0, column_index: 1, original_bytes: SERVER_LARGE_VALUE_UNKNOWN_BYTES },
                db::LargeValueCell { row_index: 0, column_index: 2, original_bytes: SERVER_LARGE_VALUE_UNKNOWN_BYTES },
            ]
        );
    }

    #[test]
    fn truncates_server_binary_preview_after_the_configured_byte_count() {
        let mut result = db::QueryResult {
            columns: vec![
                "raw_value".to_string(),
                format!("{}B_0", crate::sql_dialect::DBX_LARGE_VALUE_BYTES_COLUMN_PREFIX),
            ],
            column_types: vec!["bytea".to_string(), "text".to_string()],
            column_sortables: vec![true, true],
            spatial_columns: Vec::new(),
            spatial_values: Vec::new(),
            rows: vec![vec![serde_json::json!("0x0102030405"), serde_json::json!("B:4:5")]],
            affected_rows: 0,
            execution_time_ms: 0,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        };

        let cells = extract_server_large_value_markers(&mut result);

        assert_eq!(result.rows, vec![vec![serde_json::json!("0x01020304...")]]);
        assert_eq!(cells, vec![db::LargeValueCell { row_index: 0, column_index: 0, original_bytes: 5 }]);
    }

    #[test]
    fn restores_pgvector_previews_as_arrays_and_marks_only_truncated_values() {
        let marker = format!("{}V_0", crate::sql_dialect::DBX_LARGE_VALUE_BYTES_COLUMN_PREFIX);
        let mut result = db::QueryResult {
            columns: vec!["embedding".to_string(), marker],
            column_types: vec!["text".to_string(), "text".to_string()],
            column_sortables: vec![true, true],
            spatial_columns: Vec::new(),
            spatial_values: Vec::new(),
            rows: vec![
                vec![serde_json::json!("[0.1,0.2,0.3]"), serde_json::json!("V:9")],
                vec![serde_json::json!("[0.1,0.2]"), serde_json::json!("V:20")],
            ],
            affected_rows: 0,
            execution_time_ms: 0,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        };

        let cells = extract_server_large_value_markers(&mut result);

        assert_eq!(result.column_types, vec!["vector"]);
        assert_eq!(result.rows[0][0], serde_json::json!([0.1, 0.2]));
        assert_eq!(result.rows[1][0], serde_json::json!([0.1, 0.2]));
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].row_index, 0);
    }

    #[test]
    fn restores_server_preview_types_without_rows() {
        let mut result = db::QueryResult {
            columns: vec![
                "embedding".to_string(),
                format!("{}V_0", crate::sql_dialect::DBX_LARGE_VALUE_BYTES_COLUMN_PREFIX),
                "metadata".to_string(),
                format!("{}K_1", crate::sql_dialect::DBX_LARGE_VALUE_BYTES_COLUMN_PREFIX),
            ],
            column_types: vec!["text".to_string(), "text".to_string(), "text".to_string(), "text".to_string()],
            column_sortables: vec![true; 4],
            spatial_columns: Vec::new(),
            spatial_values: Vec::new(),
            rows: Vec::new(),
            affected_rows: 0,
            execution_time_ms: 0,
            server_execute_time_us: None,
            query_timings_ms: None,
            truncated: false,
            session_id: None,
            has_more: false,
            elasticsearch_raw_body: None,
            messages: Vec::new(),
        };

        let cells = extract_server_large_value_markers(&mut result);

        assert_eq!(result.columns, vec!["embedding", "metadata"]);
        assert_eq!(result.column_types, vec!["vector", "jsonb"]);
        assert!(cells.is_empty());
    }

    #[test]
    fn ordinary_queries_preserve_columns_that_resemble_preview_markers() {
        let marker = format!("{}0", crate::sql_dialect::DBX_LARGE_VALUE_BYTES_COLUMN_PREFIX);
        let mut result = empty_query_result(1);
        result.columns = vec!["payload".to_string(), marker.clone()];
        result.column_types = vec!["text".to_string(), "bigint".to_string()];
        result.column_sortables = vec![true, true];
        result.rows = vec![vec![serde_json::json!("value"), serde_json::json!(123)]];

        let ordinary = ExecuteMultiResult::success_with_index_and_optional_server_large_values(result, 0, false);

        assert_eq!(ordinary.result.columns, vec!["payload".to_string(), marker]);
        assert_eq!(ordinary.result.rows[0], vec![serde_json::json!("value"), serde_json::json!(123)]);
        assert!(ordinary.large_value_cells.is_empty());
    }

    #[test]
    fn single_statement_preview_preserves_absent_statement_index() {
        let result = single_statement_multi_result(Ok(empty_query_result(1)), true, None).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].statement_index, None);
    }

    #[test]
    fn query_execution_options_default_use_transaction_is_none() {
        let opts = QueryExecutionOptions::default();
        assert_eq!(opts.use_transaction, None);
    }

    #[test]
    fn query_execution_options_use_transaction_some_true_is_preserved() {
        let opts = QueryExecutionOptions { use_transaction: Some(true), ..Default::default() };
        assert_eq!(opts.use_transaction, Some(true));
    }

    #[test]
    fn query_execution_options_use_transaction_some_false_is_preserved() {
        let opts = QueryExecutionOptions { use_transaction: Some(false), ..Default::default() };
        assert_eq!(opts.use_transaction, Some(false));
    }

    fn mysql_status(in_transaction: bool, autocommit: bool) -> db::mysql::MySqlSessionStatus {
        db::mysql::MySqlSessionStatus { in_transaction, autocommit }
    }

    #[test]
    fn mysql_auto_commit_default_rolls_back_an_explicitly_opened_transaction() {
        // The historical behavior for the default setting: a `BEGIN` a user
        // left open is rolled back, but the cleanup is now reported so the tab
        // can tell the user instead of discarding the transaction silently.
        let decision = decide_mysql_auto_commit_transaction(false, false, true, Some(mysql_status(true, true)));
        assert!(!decision.preserve);
        assert_eq!(decision.rollback, MysqlAutoCommitRollback::Explicit);
    }

    #[test]
    fn mysql_auto_commit_opt_in_keeps_an_explicitly_opened_transaction() {
        // The opt-in decision is made from the *submitted* statement list, not
        // from how much of the batch actually ran. A batch that opens a
        // transaction and is then cancelled (or aborted by an error) has only
        // executed a prefix of its statements, yet the opener is still part of
        // the batch, so the connection keeps whatever the prefix did and stays
        // in the transaction. Preserving is what makes the abandoned work
        // visible — the tab shows the open-transaction badge and the manual
        // rollback action — instead of silently rolling back a transaction the
        // user asked for. `decide(true, false, true, ..)` below is exactly that
        // cancelled-batch case: the caller still passes the opener it was
        // about to run, so the decision must be preserve, not roll back.
        let decision = decide_mysql_auto_commit_transaction(true, false, true, Some(mysql_status(true, true)));
        assert!(decision.preserve);
        assert_eq!(decision.rollback, MysqlAutoCommitRollback::None);
    }

    #[test]
    fn mysql_auto_commit_opt_in_keeps_the_transaction_of_a_later_execution() {
        // `START TRANSACTION` ran first, this execution only ran an UPDATE, so
        // the batch itself carries no opener: the kept state is the only signal.
        let decision = decide_mysql_auto_commit_transaction(true, true, false, Some(mysql_status(true, true)));
        assert!(decision.preserve);
        assert_eq!(decision.rollback, MysqlAutoCommitRollback::None);

        // The kept state survives even when the caller does not pass the option
        // (auxiliary queries, result paging): an open user transaction is never
        // destroyed from another code path.
        let no_option = decide_mysql_auto_commit_transaction(false, true, false, Some(mysql_status(true, true)));
        assert!(no_option.preserve);
        assert_eq!(no_option.rollback, MysqlAutoCommitRollback::None);
    }

    #[test]
    fn mysql_auto_commit_opt_in_keeps_a_transaction_opened_by_auto_commit_off() {
        let decision = decide_mysql_auto_commit_transaction(true, false, false, Some(mysql_status(true, false)));
        assert!(decision.preserve);

        // Without the opt-in the same connection is cleaned up as before, but
        // reported as a session-level implicit transaction: nobody typed
        // `BEGIN`, the connection simply runs with auto-commit off.
        let default = decide_mysql_auto_commit_transaction(false, false, false, Some(mysql_status(true, false)));
        assert!(!default.preserve);
        assert_eq!(default.rollback, MysqlAutoCommitRollback::SessionAutocommit);
    }

    #[test]
    fn mysql_auto_commit_reports_a_batch_opener_ahead_of_autocommit_off() {
        // Both signals at once: a `BEGIN` in the batch wins, so the notice is
        // the explicit-transaction one.
        let decision = decide_mysql_auto_commit_transaction(false, false, true, Some(mysql_status(true, false)));
        assert!(!decision.preserve);
        assert_eq!(decision.rollback, MysqlAutoCommitRollback::Explicit);
    }

    #[test]
    fn mysql_auto_commit_rolls_back_a_leftover_transaction_without_reporting_it() {
        // No opener in the batch and auto-commit still on: this is a leftover
        // transaction (cancelled or aborted batch), not a user transaction.
        let decision = decide_mysql_auto_commit_transaction(true, false, false, Some(mysql_status(true, true)));
        assert!(!decision.preserve);
        assert_eq!(decision.rollback, MysqlAutoCommitRollback::None);
    }

    #[test]
    fn mysql_auto_commit_leaves_a_clean_connection_alone() {
        // `BEGIN; ...; COMMIT;` in one batch ends with nothing open.
        let decision = decide_mysql_auto_commit_transaction(true, false, true, Some(mysql_status(false, true)));
        assert!(!decision.preserve);
        assert_eq!(decision.rollback, MysqlAutoCommitRollback::None);

        // A kept transaction that the user committed in this batch must be
        // forgotten, so later executions are auto-commit again.
        let after_commit = decide_mysql_auto_commit_transaction(true, true, false, Some(mysql_status(false, true)));
        assert!(!after_commit.preserve);
        assert_eq!(after_commit.rollback, MysqlAutoCommitRollback::None);
    }

    #[test]
    fn mysql_auto_commit_uses_the_batch_opener_when_the_status_packet_is_missing() {
        // The last statement ended with an ERR packet, so no status is cached
        // and the COM_PING refresh failed.
        let kept = decide_mysql_auto_commit_transaction(true, false, true, None);
        assert!(kept.preserve);
        assert_eq!(kept.rollback, MysqlAutoCommitRollback::None);

        let cleaned = decide_mysql_auto_commit_transaction(false, false, true, None);
        assert!(!cleaned.preserve);
        assert_eq!(cleaned.rollback, MysqlAutoCommitRollback::Explicit);

        let unknown_without_opener = decide_mysql_auto_commit_transaction(true, false, false, None);
        assert!(!unknown_without_opener.preserve);
        assert_eq!(unknown_without_opener.rollback, MysqlAutoCommitRollback::None);
    }

    #[test]
    fn mysql_cleanup_rollback_is_skipped_only_when_the_final_status_proves_nothing_is_open() {
        assert!(mysql_cleanup_rollback_is_noop(true, false, Some(mysql_status(false, true))));

        // A transaction is open, or auto-commit is off: the cleanup must run.
        assert!(!mysql_cleanup_rollback_is_noop(true, false, Some(mysql_status(true, true))));
        assert!(!mysql_cleanup_rollback_is_noop(true, false, Some(mysql_status(false, false))));
        // The last statement failed, so no status packet is cached.
        assert!(!mysql_cleanup_rollback_is_noop(true, false, None));
        // A truncated or failed result may leave an earlier command's packet.
        assert!(!mysql_cleanup_rollback_is_noop(false, false, Some(mysql_status(false, true))));
        // A batch that opened a transaction keeps the cleanup even when the
        // server reports nothing open, in case it never sets the flag.
        assert!(!mysql_cleanup_rollback_is_noop(true, true, Some(mysql_status(false, true))));
    }

    #[test]
    fn mysql_backup_transaction_only_falls_back_for_syntax_errors() {
        let syntax_error = mysql_async::Error::Server(mysql_async::ServerError {
            code: 1064,
            message: "unsupported transaction characteristic".to_string(),
            state: "42000".to_string(),
        });
        let permission_error = mysql_async::Error::Server(mysql_async::ServerError {
            code: 1044,
            message: "access denied".to_string(),
            state: "42000".to_string(),
        });

        assert!(mysql_error_is_syntax_error(&syntax_error));
        assert!(!mysql_error_is_syntax_error(&permission_error));
    }

    #[test]
    fn mysql_backup_transaction_falls_back_for_polardbx_parser_errors() {
        for message in [
            "[PXC-4500][ERR_PARSER] syntax error, expect EOF, actual COMMA after WITH CONSISTENT SNAPSHOT",
            "START TRANSACTION WITH CONSISTENT SNAPSHOT, READ ONLY 语法错误",
        ] {
            let polardbx_parser_error = mysql_async::Error::Server(mysql_async::ServerError {
                code: 3009,
                message: message.to_string(),
                state: "HY000".to_string(),
            });
            assert!(mysql_error_is_syntax_error(&polardbx_parser_error));
        }
        let polardbx_non_parser_error = mysql_async::Error::Server(mysql_async::ServerError {
            code: 3009,
            message: "Failed to handle data".to_string(),
            state: "HY000".to_string(),
        });

        assert!(!mysql_error_is_syntax_error(&polardbx_non_parser_error));
    }
}

async fn rollback_manual_txn_connection_with_timeout(
    conn: &mut TxnConnection,
    postgres_timeout: Option<Duration>,
) -> Result<(), String> {
    match conn {
        TxnConnection::Mysql(Some(conn)) => {
            conn.query_drop("ROLLBACK").await.map_err(|e| format!("ROLLBACK failed: {e}"))?;
        }
        TxnConnection::Mysql(None) => return Ok(()),
    }
    Ok(())
}
