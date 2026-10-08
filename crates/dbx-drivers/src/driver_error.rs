use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::time::Duration;
const AGENT_RPC_ERROR_DATA_MARKER: &str = "\nDBX_AGENT_ERROR_DATA:";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionDisposition {
    Keep,
    Quarantine,
    ReplaceRuntime,
}

impl AgentSessionDisposition {
    fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::Quarantine => "quarantine",
            Self::ReplaceRuntime => "replace_runtime",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentErrorCategory {
    Connection,
    Sql,
    Resource,
    Protocol,
    Timeout,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentErrorStage {
    Request,
    Checkout,
    Connect,
    Validate,
    Execute,
    Fetch,
    Cancel,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOperationOutcome {
    NotStarted,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentErrorContext {
    pub contract_version: u8,
    pub category: AgentErrorCategory,
    pub retryable: bool,
    pub session_disposition: AgentSessionDisposition,
    pub stage: AgentErrorStage,
    pub operation_outcome: AgentOperationOutcome,
    #[serde(default)]
    pub agent_session_id: Option<String>,
    #[serde(default)]
    pub sql_state: Option<String>,
    #[serde(default)]
    pub vendor_code: Option<i32>,
    #[serde(default)]
    pub exception_class: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyAgentHints {
    pub category: Option<AgentErrorCategory>,
    pub retryable: Option<bool>,
    pub session_disposition: Option<AgentSessionDisposition>,
    pub stage: Option<AgentErrorStage>,
    pub operation_outcome: Option<AgentOperationOutcome>,
    pub agent_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractViolationReason {
    MissingContractVersion,
    UnsupportedContractVersion,
    InvalidDataShape,
    InvalidSessionId,
    InvalidCombination,
    MissingResultOrError,
    InvalidResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentCallError {
    Structured { rpc_code: i64, message: String, context: AgentErrorContext },
    Legacy { rpc_code: Option<i64>, message: String, hints: LegacyAgentHints },
    ContractViolation { rpc_code: Option<i64>, message: String, reason: ContractViolationReason },
    Transport { message: String },
    Timeout { stage: AgentErrorStage, operation_outcome: AgentOperationOutcome },
    Canceled { stage: AgentErrorStage, operation_outcome: AgentOperationOutcome },
}

/// Decode an already-stringified Agent failure at the compatibility boundary.
///
/// Business modules must use this adapter instead of inspecting the legacy
/// `DBX_AGENT_ERROR_DATA` marker themselves. Callers must only use it after the
/// owning pool has been identified as an Agent pool.
pub fn agent_error_from_legacy(error: &str, expected_session_id: Option<&str>) -> AgentCallError {
    if error == "Query canceled" {
        return AgentCallError::Canceled {
            stage: AgentErrorStage::Cancel,
            operation_outcome: AgentOperationOutcome::Unknown,
        };
    }
    let lower = error.to_ascii_lowercase();
    if lower.starts_with("agent rpc call timed out") {
        return AgentCallError::Timeout {
            stage: AgentErrorStage::Execute,
            operation_outcome: AgentOperationOutcome::Unknown,
        };
    }
    legacy_agent_call_error(error.to_string(), expected_session_id)
}

/// Returns a typed error only when a mixed legacy path can prove the string
/// originated from the Agent call corridor.
pub fn try_agent_error_from_legacy(error: &str) -> Option<AgentCallError> {
    let lower = error.to_ascii_lowercase();
    let is_local_agent_failure = error == "Query canceled"
        || lower.starts_with("agent rpc call timed out")
        || lower.contains("agent stdin not available")
        || lower.contains("agent stdout not available")
        || lower.contains("agent runtime terminated")
        || lower.contains("agent runtime is unavailable")
        || lower.contains("agent runtime unavailable")
        || lower.contains("failed to write to agent stdin")
        || lower.contains("failed to flush agent stdin")
        || lower.contains("agent rpc task failed");
    (is_agent_rpc_response_error(error) || is_local_agent_failure).then(|| agent_error_from_legacy(error, None))
}

impl AgentCallError {
    pub fn into_legacy_string(self) -> String {
        match self {
            Self::Structured { rpc_code, message, context } => {
                let data = serde_json::json!({
                    "contractVersion": context.contract_version,
                    "category": category_name(context.category),
                    "retryable": context.retryable,
                    "sessionDisposition": context.session_disposition.as_str(),
                    "stage": stage_name(context.stage),
                    "operationOutcome": outcome_name(context.operation_outcome),
                    "agentSessionId": context.agent_session_id,
                    "sqlState": context.sql_state,
                    "vendorCode": context.vendor_code,
                    "exceptionClass": context.exception_class,
                });
                format_agent_rpc_error_parts(rpc_code, &message, Some(&data))
            }
            Self::Legacy { rpc_code, message, hints } => {
                let data = serde_json::json!({
                    "category": hints.category.map(category_name),
                    "retryable": hints.retryable,
                    "sessionDisposition": hints.session_disposition.map(|value| value.as_str()),
                    "stage": hints.stage.map(stage_name),
                    "operationOutcome": hints.operation_outcome.map(outcome_name),
                    "agentSessionId": hints.agent_session_id,
                });
                format_agent_rpc_error_parts(rpc_code.unwrap_or(-1), &message, Some(&data))
            }
            Self::ContractViolation { rpc_code, message, .. } => format_agent_rpc_error_parts(
                rpc_code.unwrap_or(-1),
                &message,
                Some(&serde_json::json!({ "contractVersion": 1 })),
            ),
            Self::Transport { message } => message,
            Self::Timeout { stage, .. } => format!("Agent RPC call timed out at {}", stage_name(stage)),
            Self::Canceled { .. } => "Query canceled".to_string(),
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Structured { context, .. } => context.agent_session_id.as_deref(),
            Self::Legacy { hints, .. } => hints.agent_session_id.as_deref(),
            _ => None,
        }
    }

    pub fn session_disposition(&self) -> Option<AgentSessionDisposition> {
        match self {
            Self::Structured { context, .. } => Some(context.session_disposition),
            Self::Legacy { hints, .. } => hints.session_disposition,
            _ => None,
        }
    }

    pub fn category(&self) -> Option<AgentErrorCategory> {
        match self {
            Self::Structured { context, .. } => Some(context.category),
            Self::Legacy { hints, .. } => hints.category,
            Self::Timeout { .. } => Some(AgentErrorCategory::Timeout),
            Self::Canceled { .. } => Some(AgentErrorCategory::Canceled),
            Self::ContractViolation { .. } | Self::Transport { .. } => None,
        }
    }

    pub fn operation_outcome(&self) -> AgentOperationOutcome {
        match self {
            Self::Structured { context, .. } => context.operation_outcome,
            Self::Legacy { hints, .. } => hints.operation_outcome.unwrap_or(AgentOperationOutcome::Unknown),
            Self::Timeout { operation_outcome, .. } | Self::Canceled { operation_outcome, .. } => *operation_outcome,
            Self::ContractViolation { .. } | Self::Transport { .. } => AgentOperationOutcome::Unknown,
        }
    }

    fn with_legacy_session_id(mut self, agent_session_id: Option<&str>) -> Self {
        if let (Self::Legacy { hints, .. }, Some(agent_session_id)) = (&mut self, agent_session_id) {
            hints.agent_session_id.get_or_insert_with(|| agent_session_id.to_string());
        }
        self
    }
}

impl fmt::Display for AgentCallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Structured { message, .. } | Self::Legacy { message, .. } => f.write_str(message),
            Self::ContractViolation { message, .. } => write!(f, "Agent error contract violation: {message}"),
            Self::Transport { message } => f.write_str(message),
            Self::Timeout { stage, .. } => write!(f, "Agent RPC call timed out at {}", stage_name(*stage)),
            Self::Canceled { .. } => f.write_str("Query canceled"),
        }
    }
}

impl From<AgentCallError> for String {
    fn from(error: AgentCallError) -> Self {
        error.into_legacy_string()
    }
}

impl From<String> for AgentCallError {
    fn from(message: String) -> Self {
        Self::Transport { message }
    }
}

impl From<&str> for AgentCallError {
    fn from(message: &str) -> Self {
        Self::Transport { message: message.to_string() }
    }
}

fn parse_structured_error_context(
    data: &Value,
    expected_session_id: Option<&str>,
) -> Result<AgentErrorContext, ContractViolationReason> {
    let Some(contract_version_value) = data.get("contractVersion") else {
        return Err(ContractViolationReason::MissingContractVersion);
    };
    let Some(contract_version) = contract_version_value.as_u64() else {
        return Err(ContractViolationReason::InvalidDataShape);
    };
    if contract_version != 1 {
        return Err(ContractViolationReason::UnsupportedContractVersion);
    }
    let context = serde_json::from_value::<AgentErrorContext>(data.clone())
        .map_err(|_| ContractViolationReason::InvalidDataShape)?;
    if context.contract_version != 1 {
        return Err(ContractViolationReason::UnsupportedContractVersion);
    }
    if let Some(expected_session_id) = expected_session_id {
        if context.agent_session_id.as_deref() != Some(expected_session_id) {
            return Err(ContractViolationReason::InvalidSessionId);
        }
    }
    if !valid_agent_error_combination(&context) {
        return Err(ContractViolationReason::InvalidCombination);
    }
    Ok(context)
}

fn legacy_agent_hints(data: &Value) -> LegacyAgentHints {
    LegacyAgentHints {
        category: data.get("category").and_then(|value| serde_json::from_value(value.clone()).ok()),
        retryable: data.get("retryable").and_then(Value::as_bool),
        session_disposition: match data.get("sessionDisposition").and_then(Value::as_str) {
            Some("keep") => Some(AgentSessionDisposition::Keep),
            Some("quarantine") => Some(AgentSessionDisposition::Quarantine),
            Some("replace_runtime") => Some(AgentSessionDisposition::ReplaceRuntime),
            _ => None,
        },
        stage: data.get("stage").and_then(|value| serde_json::from_value(value.clone()).ok()),
        operation_outcome: data.get("operationOutcome").and_then(|value| serde_json::from_value(value.clone()).ok()),
        agent_session_id: data.get("agentSessionId").and_then(Value::as_str).map(str::to_string),
    }
}

pub fn valid_agent_error_combination(context: &AgentErrorContext) -> bool {
    let expected_outcome = match context.stage {
        AgentErrorStage::Request | AgentErrorStage::Checkout | AgentErrorStage::Connect | AgentErrorStage::Validate => {
            AgentOperationOutcome::NotStarted
        }
        AgentErrorStage::Execute | AgentErrorStage::Fetch | AgentErrorStage::Cancel | AgentErrorStage::Close => {
            AgentOperationOutcome::Unknown
        }
    };
    if context.operation_outcome != expected_outcome {
        return false;
    }
    let valid_disposition = match context.category {
        AgentErrorCategory::Connection => context.session_disposition != AgentSessionDisposition::ReplaceRuntime,
        AgentErrorCategory::Sql => {
            matches!(
                context.stage,
                AgentErrorStage::Execute | AgentErrorStage::Fetch | AgentErrorStage::Cancel | AgentErrorStage::Close
            ) && context.session_disposition != AgentSessionDisposition::ReplaceRuntime
        }
        AgentErrorCategory::Resource => {
            context.session_disposition == AgentSessionDisposition::ReplaceRuntime
                || context.operation_outcome == AgentOperationOutcome::NotStarted
        }
        AgentErrorCategory::Protocol => {
            context.session_disposition != AgentSessionDisposition::ReplaceRuntime
                || context.operation_outcome == AgentOperationOutcome::Unknown
        }
        AgentErrorCategory::Timeout | AgentErrorCategory::Canceled => {
            context.session_disposition == AgentSessionDisposition::Quarantine
        }
    };
    if !valid_disposition {
        return false;
    }
    valid_ascii_diagnostic(context.sql_state.as_deref(), 16)
        && valid_ascii_diagnostic(context.exception_class.as_deref(), 160)
        && context.agent_session_id.as_deref().is_none_or(valid_ascii_identifier)
}

fn valid_ascii_diagnostic(value: Option<&str>, max_length: usize) -> bool {
    value.is_none_or(|value| {
        !value.is_empty() && value.len() <= max_length && value.bytes().all(|byte| byte.is_ascii_graphic())
    })
}

fn valid_ascii_identifier(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_graphic())
}

pub fn category_name(category: AgentErrorCategory) -> &'static str {
    match category {
        AgentErrorCategory::Connection => "connection",
        AgentErrorCategory::Sql => "sql",
        AgentErrorCategory::Resource => "resource",
        AgentErrorCategory::Protocol => "protocol",
        AgentErrorCategory::Timeout => "timeout",
        AgentErrorCategory::Canceled => "canceled",
    }
}

pub fn stage_name(stage: AgentErrorStage) -> &'static str {
    match stage {
        AgentErrorStage::Request => "request",
        AgentErrorStage::Checkout => "checkout",
        AgentErrorStage::Connect => "connect",
        AgentErrorStage::Validate => "validate",
        AgentErrorStage::Execute => "execute",
        AgentErrorStage::Fetch => "fetch",
        AgentErrorStage::Cancel => "cancel",
        AgentErrorStage::Close => "close",
    }
}

pub fn outcome_name(outcome: AgentOperationOutcome) -> &'static str {
    match outcome {
        AgentOperationOutcome::NotStarted => "not_started",
        AgentOperationOutcome::Unknown => "unknown",
    }
}

fn format_agent_rpc_error_parts(code: i64, message: &str, data: Option<&Value>) -> String {
    let mut formatted = format!("Agent RPC error ({code}): {message}");
    if let Some(data) = data {
        formatted.push_str(AGENT_RPC_ERROR_DATA_MARKER);
        formatted.push_str(&data.to_string());
    }
    formatted
}

pub fn legacy_agent_call_error(error: String, agent_session_id: Option<&str>) -> AgentCallError {
    if !is_agent_rpc_response_error(&error) {
        return AgentCallError::Transport { message: error };
    }
    let (header, data, suffix) =
        error.rsplit_once(AGENT_RPC_ERROR_DATA_MARKER).map_or((error.as_str(), None, ""), |(header, data)| {
            match parse_legacy_agent_error_data(data) {
                Some((data, suffix)) => (header, Some(data), suffix),
                None => (header, None, ""),
            }
        });
    let header_with_suffix = (!suffix.is_empty()).then(|| format!("{header}{suffix}"));
    let header = header_with_suffix.as_deref().unwrap_or(header);
    let (rpc_code, message) = parse_agent_rpc_error_header(header);
    if let Some(data) = data.as_ref().filter(|data| data.get("contractVersion").is_some()) {
        return match parse_structured_error_context(data, agent_session_id) {
            Ok(context) => AgentCallError::Structured { rpc_code: rpc_code.unwrap_or(-1), message, context },
            Err(reason) => AgentCallError::ContractViolation { rpc_code, message, reason },
        };
    }
    let mut hints = data.as_ref().map(legacy_agent_hints).unwrap_or_default();
    if hints.category.is_none() && is_legacy_connection_message(&message) {
        hints.category = Some(AgentErrorCategory::Connection);
        hints.session_disposition.get_or_insert(AgentSessionDisposition::Quarantine);
    }
    AgentCallError::Legacy { rpc_code, message, hints }.with_legacy_session_id(agent_session_id)
}

fn parse_legacy_agent_error_data(data: &str) -> Option<(Value, &str)> {
    let mut values = serde_json::Deserializer::from_str(data).into_iter::<Value>();
    let value = values.next()?.ok()?;
    Some((value, &data[values.byte_offset()..]))
}

pub fn append_legacy_error_context(error: &str, context: &str) -> String {
    if error.contains(context) {
        return error.to_string();
    }
    if let Some((header, data)) = error.rsplit_once(AGENT_RPC_ERROR_DATA_MARKER) {
        return format!("{header}\n{context}{AGENT_RPC_ERROR_DATA_MARKER}{data}");
    }
    format!("{error}\n{context}")
}

fn is_legacy_connection_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "connection lost",
        "connection reset",
        "broken pipe",
        "communications link failure",
        "sqlrecoverableexception",
        "sqlnontransientconnectionexception",
        "sqltransientconnectionexception",
        "network communication",
        "网络通信异常",
        "通信异常",
        "关闭的连接",
        "连接已关闭",
        "not connected",
        "end of stream",
        "end-of-file",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn parse_agent_rpc_error_header(header: &str) -> (Option<i64>, String) {
    let Some(rest) = header.strip_prefix("Agent RPC error (") else {
        return (None, header.to_string());
    };
    let Some((code, message)) = rest.split_once("): ") else {
        return (None, header.to_string());
    };
    (code.parse().ok(), message.to_string())
}

fn is_agent_rpc_response_error(message: &str) -> bool {
    message.trim_start().starts_with("Agent RPC error (")
}
