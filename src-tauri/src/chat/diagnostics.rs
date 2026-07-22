//! Agent Runtime 的隐私安全诊断日志。
//!
//! 允许字段固定为运行元数据；接口不接受 prompt、回答、推理、工具参数或工具结果，
//! 从类型边界上减少误记案件内容的可能。

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};

use super::stream::{ChatActivity, ChatActivityPhase, ChatActivityStatus};

const LOG_FILE: &str = "agent-runtime-events.jsonl";
const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const TRACE_SCHEMA_VERSION: u8 = 1;
const TRACE_WINDOW_HOURS: i64 = 24;
const MAX_TRACE_EVENTS: usize = 200;
const MAX_TRACE_REQUESTS: usize = 10;

static TRACE_WRITER_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RuntimeTraceCommon {
    pub schema_version: u8,
    pub ts: String,
    pub surface: String,
    pub request_id: String,
    pub runtime: String,
}

impl RuntimeTraceCommon {
    pub fn new(surface: &str, request_id: &str, runtime: &str) -> Self {
        Self::new_at(surface, request_id, runtime, Utc::now())
    }

    pub fn new_at(surface: &str, request_id: &str, runtime: &str, at: DateTime<Utc>) -> Self {
        Self {
            schema_version: TRACE_SCHEMA_VERSION,
            ts: at.to_rfc3339(),
            surface: surface.to_string(),
            request_id: request_id.to_string(),
            runtime: runtime.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureOrigin {
    Provider,
    Network,
    PiRuntime,
    Protocol,
    HostTool,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeErrorCategory {
    Authentication,
    Quota,
    RateLimit,
    ModelNotFound,
    ContextLimit,
    Provider4xx,
    Provider5xx,
    Dns,
    Tls,
    Timeout,
    ConnectionReset,
    RuntimeUnavailable,
    ProcessExit,
    Protocol,
    Tool,
    Cancelled,
    Execution,
}

impl RuntimeErrorCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Authentication => "authentication",
            Self::Quota => "quota",
            Self::RateLimit => "rate_limit",
            Self::ModelNotFound => "model_not_found",
            Self::ContextLimit => "context_limit",
            Self::Provider4xx => "provider_4xx",
            Self::Provider5xx => "provider_5xx",
            Self::Dns => "dns",
            Self::Tls => "tls",
            Self::Timeout => "timeout",
            Self::ConnectionReset => "connection_reset",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::ProcessExit => "process_exit",
            Self::Protocol => "protocol",
            Self::Tool => "tool",
            Self::Cancelled => "cancelled",
            Self::Execution => "execution",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeRetryStatus {
    #[default]
    Started,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTerminalStatus {
    Completed,
    #[default]
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RuntimeRequestStarted {
    #[serde(flatten)]
    pub common: RuntimeTraceCommon,
    pub provider_id: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_api: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidecar_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pi_sdk_version: Option<String>,
    pub protocol_version: u32,
    pub platform: String,
    pub arch: String,
    pub system_prompt_chars: u64,
    pub history_messages: u64,
    pub history_chars: u64,
    pub user_message_chars: u64,
    pub tool_count: u64,
    pub skill_count: u64,
    pub max_tokens: u64,
    pub reasoning_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RuntimeRetry {
    #[serde(flatten)]
    pub common: RuntimeTraceCommon,
    pub attempt: u32,
    pub max_attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay_ms: Option<u64>,
    pub status: RuntimeRetryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_origin: Option<FailureOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_category: Option<RuntimeErrorCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RuntimeRequestFinished {
    #[serde(flatten)]
    pub common: RuntimeTraceCommon,
    pub status: RuntimeTerminalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_origin: Option<FailureOrigin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_category: Option<RuntimeErrorCategory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    pub elapsed_ms: u64,
    pub turns: u32,
    pub tool_calls: u32,
    pub tool_failures: u32,
    pub retry_count: u32,
    pub text_chars: u64,
    pub reasoning_chars: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stderr_tail: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RuntimeTraceEvent {
    RequestStarted(RuntimeRequestStarted),
    Retry(RuntimeRetry),
    RequestFinished(RuntimeRequestFinished),
}

impl RuntimeTraceEvent {
    fn common(&self) -> &RuntimeTraceCommon {
        match self {
            Self::RequestStarted(event) => &event.common,
            Self::Retry(event) => &event.common,
            Self::RequestFinished(event) => &event.common,
        }
    }

    pub fn request_id(&self) -> &str {
        &self.common().request_id
    }

    pub fn timestamp(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.common().ts)
            .ok()
            .map(|value| value.with_timezone(&Utc))
    }

    pub fn provider_id(&self) -> Option<&str> {
        match self {
            Self::RequestStarted(event) => Some(&event.provider_id),
            Self::RequestFinished(event) => event.provider_id.as_deref(),
            Self::Retry(_) => None,
        }
    }

    pub fn is_failed_terminal(&self) -> bool {
        matches!(
            self,
            Self::RequestFinished(RuntimeRequestFinished {
                status: RuntimeTerminalStatus::Failed | RuntimeTerminalStatus::Cancelled,
                ..
            })
        )
    }
}

#[derive(Serialize)]
struct RuntimeDiagnosticRow<'a> {
    ts: String,
    surface: &'a str,
    request_id: &'a str,
    #[serde(flatten)]
    activity: &'a ChatActivity,
}

pub fn append_runtime_activity(surface: &str, request_id: &str, activity: &ChatActivity) {
    let Ok(root) = crate::db::app_data_dir() else {
        return;
    };
    if let Err(error) = append_runtime_activity_at(
        &root.join("logs"),
        surface,
        request_id,
        activity,
        MAX_LOG_BYTES,
    ) {
        crate::dlog!("agent diagnostics metadata write failed: {}", error);
    }
}

pub fn append_runtime_terminal(
    surface: &str,
    request_id: &str,
    runtime: &str,
    cancelled: bool,
    elapsed_ms: u64,
    error_category: &'static str,
) {
    append_runtime_activity(
        surface,
        request_id,
        &ChatActivity {
            runtime: runtime.to_string(),
            phase: ChatActivityPhase::Run,
            status: if cancelled {
                ChatActivityStatus::Cancelled
            } else {
                ChatActivityStatus::Failed
            },
            sequence: u32::MAX,
            turn: None,
            tool: None,
            elapsed_ms: Some(elapsed_ms),
            error_category: Some(error_category.to_string()),
        },
    );
}

pub fn runtime_error_category(error: &str) -> &'static str {
    classify_runtime_error(None, error).1.as_str()
}

pub fn classify_runtime_error(
    code: Option<&str>,
    message: &str,
) -> (FailureOrigin, RuntimeErrorCategory) {
    let normalized_code = code.unwrap_or_default().to_lowercase();
    let combined = format!("{normalized_code} {message}").to_lowercase();
    if normalized_code.contains("process_exit") {
        return (FailureOrigin::PiRuntime, RuntimeErrorCategory::ProcessExit);
    }
    if normalized_code.contains("protocol") || normalized_code.contains("parse") {
        return (FailureOrigin::Protocol, RuntimeErrorCategory::Protocol);
    }
    if normalized_code.contains("tool") {
        return (FailureOrigin::HostTool, RuntimeErrorCategory::Tool);
    }
    if normalized_code.contains("spawn")
        || normalized_code.contains("process_io")
        || normalized_code.contains("handshake")
        || normalized_code.contains("runtime_guard")
    {
        return (
            FailureOrigin::PiRuntime,
            RuntimeErrorCategory::RuntimeUnavailable,
        );
    }
    if message.contains("用户取消") || combined.contains("cancel") {
        return (FailureOrigin::Cancelled, RuntimeErrorCategory::Cancelled);
    }
    if combined.contains("401")
        || combined.contains("403")
        || combined.contains("api key")
        || combined.contains("unauthorized")
        || combined.contains("forbidden")
    {
        return (
            FailureOrigin::Provider,
            RuntimeErrorCategory::Authentication,
        );
    }
    if combined.contains("402")
        || combined.contains("quota")
        || combined.contains("balance")
        || combined.contains("insufficient credit")
    {
        return (FailureOrigin::Provider, RuntimeErrorCategory::Quota);
    }
    if combined.contains("429") || combined.contains("rate limit") {
        return (FailureOrigin::Provider, RuntimeErrorCategory::RateLimit);
    }
    if combined.contains("model_not_found") || combined.contains("model not found") {
        return (FailureOrigin::Provider, RuntimeErrorCategory::ModelNotFound);
    }
    if combined.contains("context length")
        || combined.contains("context window")
        || combined.contains("too many tokens")
    {
        return (FailureOrigin::Provider, RuntimeErrorCategory::ContextLimit);
    }
    if ["500", "502", "503", "504"]
        .iter()
        .any(|status| combined.contains(status))
    {
        return (FailureOrigin::Provider, RuntimeErrorCategory::Provider5xx);
    }
    if ["400", "404", "405", "409", "422"]
        .iter()
        .any(|status| combined.contains(status))
    {
        return (FailureOrigin::Provider, RuntimeErrorCategory::Provider4xx);
    }
    if combined.contains("dns")
        || combined.contains("name resolution")
        || combined.contains("failed to lookup address")
    {
        return (FailureOrigin::Network, RuntimeErrorCategory::Dns);
    }
    if combined.contains("tls") || combined.contains("ssl") || combined.contains("certificate") {
        return (FailureOrigin::Network, RuntimeErrorCategory::Tls);
    }
    if message.contains("超时") || combined.contains("timeout") || combined.contains("timed out")
    {
        return (FailureOrigin::Network, RuntimeErrorCategory::Timeout);
    }
    if combined.contains("connection reset")
        || combined.contains("connection closed")
        || combined.contains("unexpected eof")
        || combined.contains("socket")
    {
        return (
            FailureOrigin::Network,
            RuntimeErrorCategory::ConnectionReset,
        );
    }
    if combined.contains("json") || combined.contains("协议") || combined.contains("parse") {
        return (FailureOrigin::Protocol, RuntimeErrorCategory::Protocol);
    }
    if combined.contains("tool") || combined.contains("工具") {
        return (FailureOrigin::HostTool, RuntimeErrorCategory::Tool);
    }
    if combined.contains("sidecar") || combined.contains("runtime") {
        return (
            FailureOrigin::PiRuntime,
            RuntimeErrorCategory::RuntimeUnavailable,
        );
    }
    (FailureOrigin::Unknown, RuntimeErrorCategory::Execution)
}

pub fn append_runtime_trace(event: &RuntimeTraceEvent) {
    let Ok(root) = crate::db::app_data_dir() else {
        return;
    };
    if let Err(error) = append_runtime_trace_at(&root.join("logs"), event, MAX_LOG_BYTES) {
        crate::dlog!("agent trace metadata write failed: {}", error);
    }
}

fn append_runtime_trace_at(
    log_dir: &Path,
    event: &RuntimeTraceEvent,
    max_bytes: u64,
) -> Result<(), String> {
    let encoded = serde_json::to_string(event).map_err(|error| error.to_string())?;
    let _guard = TRACE_WRITER_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "agent trace writer lock poisoned".to_string())?;
    std::fs::create_dir_all(log_dir).map_err(|error| error.to_string())?;
    let path = log_dir.join(LOG_FILE);
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= max_bytes.max(1)) {
        let rotated = log_dir.join(format!("{LOG_FILE}.1"));
        if rotated.exists() {
            std::fs::remove_file(&rotated).map_err(|error| error.to_string())?;
        }
        std::fs::rename(&path, &rotated).map_err(|error| error.to_string())?;
        set_private_file_permissions(&rotated).map_err(|error| error.to_string())?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| error.to_string())?;
    set_private_file_permissions(&path).map_err(|error| error.to_string())?;
    writeln!(file, "{encoded}").map_err(|error| error.to_string())
}

#[cfg(unix)]
fn set_private_file_permissions(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_file_permissions(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

pub fn read_recent_runtime_traces() -> Result<Vec<RuntimeTraceEvent>, String> {
    let root = crate::db::app_data_dir().map_err(|error| error.to_string())?;
    read_recent_runtime_traces_at(&root.join("logs"), Utc::now())
}

fn read_recent_runtime_traces_at(
    log_dir: &Path,
    now: DateTime<Utc>,
) -> Result<Vec<RuntimeTraceEvent>, String> {
    let cutoff = now - ChronoDuration::hours(TRACE_WINDOW_HOURS);
    let mut events = Vec::new();
    for path in [
        log_dir.join(format!("{LOG_FILE}.1")),
        log_dir.join(LOG_FILE),
    ] {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(event) = serde_json::from_str::<RuntimeTraceEvent>(&line) else {
                continue;
            };
            if event
                .timestamp()
                .is_some_and(|timestamp| timestamp >= cutoff)
            {
                events.push(event);
            }
        }
    }

    let mut request_last_seen: HashMap<String, DateTime<Utc>> = HashMap::new();
    let mut failed_requests = HashSet::new();
    for event in &events {
        if let Some(timestamp) = event.timestamp() {
            request_last_seen
                .entry(event.request_id().to_string())
                .and_modify(|current| *current = (*current).max(timestamp))
                .or_insert(timestamp);
        }
        if event.is_failed_terminal() {
            failed_requests.insert(event.request_id().to_string());
        }
    }
    let mut request_ids: Vec<String> = request_last_seen.keys().cloned().collect();
    request_ids.sort_by(|a, b| {
        failed_requests
            .contains(b)
            .cmp(&failed_requests.contains(a))
            .then_with(|| request_last_seen[b].cmp(&request_last_seen[a]))
    });
    request_ids.truncate(MAX_TRACE_REQUESTS);
    let rank: HashMap<String, usize> = request_ids
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, request_id)| (request_id, index))
        .collect();
    events.retain(|event| rank.contains_key(event.request_id()));
    events.sort_by(|a, b| {
        rank[a.request_id()]
            .cmp(&rank[b.request_id()])
            .then_with(|| a.timestamp().cmp(&b.timestamp()))
    });
    events.truncate(MAX_TRACE_EVENTS);
    Ok(events)
}

fn append_runtime_activity_at(
    log_dir: &Path,
    surface: &str,
    request_id: &str,
    activity: &ChatActivity,
    max_bytes: u64,
) -> Result<(), String> {
    let _guard = TRACE_WRITER_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "agent trace writer lock poisoned".to_string())?;
    std::fs::create_dir_all(log_dir).map_err(|error| error.to_string())?;
    let path = log_dir.join(LOG_FILE);
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() >= max_bytes.max(1)) {
        let rotated = log_dir.join(format!("{LOG_FILE}.1"));
        if rotated.exists() {
            std::fs::remove_file(&rotated).map_err(|error| error.to_string())?;
        }
        std::fs::rename(&path, &rotated).map_err(|error| error.to_string())?;
        set_private_file_permissions(&rotated).map_err(|error| error.to_string())?;
    }
    let row = RuntimeDiagnosticRow {
        ts: chrono::Utc::now().to_rfc3339(),
        surface,
        request_id,
        activity,
    };
    let encoded = serde_json::to_string(&row).map_err(|error| error.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    set_private_file_permissions(&log_dir.join(LOG_FILE)).map_err(|error| error.to_string())?;
    writeln!(file, "{encoded}").map_err(|error| error.to_string())
}

pub fn open_runtime_log_directory() -> Result<(), String> {
    let directory = crate::db::app_data_dir()
        .map_err(|error| error.to_string())?
        .join("logs");
    std::fs::create_dir_all(&directory).map_err(|error| format!("创建日志目录失败: {error}"))?;
    tauri_plugin_opener::open_path(&directory, None::<&str>)
        .map_err(|error| format!("打开日志目录失败: {error}"))
}
