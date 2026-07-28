use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::UnboundedSender;

use zeroize::Zeroizing;

use super::pi_credentials::{resolve_pi_credential, OsPiCredentialVault, PiCredentialVault};
use super::pi_locator::resolve_pi_runtime_binary;
use super::pi_protocol::{
    PiHostMessage, PiModelConfig, PiRuntimeCapabilities, PiSidecarMessage, PiStartRequest,
    PI_PROTOCOL_VERSION,
};
use super::pi_safety::PiSafetyGuard;
use super::ChatRunControl;
use super::PiRuntimeSelection;
use crate::chat::agent_loop::{
    parse_ask_user_args, AgentLoopError, AgentLoopOutput, AgentLoopRequest, CostMetrics,
    ToolCallRecord,
};
use crate::chat::diagnostics::{
    append_runtime_trace, classify_runtime_error, RuntimeErrorCategory, RuntimeRequestFinished,
    RuntimeRequestStarted, RuntimeRetry, RuntimeRetryStatus, RuntimeTerminalStatus,
    RuntimeTraceCommon, RuntimeTraceEvent,
};
use crate::chat::hooks::{HookChain, HookContext, HookOutcome, SessionStats};
use crate::chat::parallel::{execute_registered_tool, Subtask, SubtaskResult};
use crate::chat::prefix_cache::PrefixFingerprint;
use crate::chat::stream::{
    ChatActivity, ChatActivityPhase, ChatActivityStatus, ChatStreamEvent, ChatUsage,
};
use crate::chat::tools::{ToolContext, ToolRegistry, ToolResult};
use crate::llm::LlmConfig;

const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const EXIT_GRACE: Duration = Duration::from_secs(2);
const PI_MINIMUM_MACOS_MAJOR: u32 = 13;

#[derive(Clone)]
enum PiTraceSink {
    AppData,
}

impl PiTraceSink {
    fn emit(&self, event: RuntimeTraceEvent) {
        match self {
            Self::AppData => append_runtime_trace(&event),
        }
    }
}

struct PiTraceSession {
    sink: PiTraceSink,
    started: RuntimeRequestStarted,
    started_emitted: bool,
    finished: bool,
    started_at: std::time::Instant,
    turns: u32,
    tool_calls: u32,
    tool_failures: u32,
    retry_count: u32,
    text_chars: u64,
    reasoning_chars: u64,
    secrets: Zeroizing<Vec<String>>,
}

impl PiTraceSession {
    fn new(
        start: &PiStartRequest,
        max_tokens: u64,
        credential_type: Option<String>,
        credential_source: Option<String>,
        sink: PiTraceSink,
    ) -> Self {
        let secrets = pi_model_secret_values(&start.model);
        let reasoning_enabled = start
            .model
            .caseboard_custom
            .as_ref()
            .is_some_and(|model| model.reasoning)
            || start.model.model_id.to_ascii_lowercase().contains("reason");
        Self {
            sink,
            started: RuntimeRequestStarted {
                common: RuntimeTraceCommon::new("case", &start.request_id, "pi"),
                provider_id: start.model.provider_id.clone(),
                model_id: start.model.model_id.clone(),
                model_api: None,
                credential_type,
                credential_source,
                sidecar_version: None,
                pi_sdk_version: None,
                protocol_version: start.protocol_version,
                platform: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
                system_prompt_chars: start.system_prompt.chars().count() as u64,
                history_messages: start.history.len() as u64,
                history_chars: start
                    .history
                    .iter()
                    .map(|message| message.content.chars().count() as u64)
                    .sum(),
                user_message_chars: start.user_message.chars().count() as u64,
                tool_count: start.tools.len() as u64,
                skill_count: start.skills.len() as u64,
                max_tokens,
                reasoning_enabled,
            },
            started_emitted: false,
            finished: false,
            started_at: std::time::Instant::now(),
            turns: 0,
            tool_calls: 0,
            tool_failures: 0,
            retry_count: 0,
            text_chars: 0,
            reasoning_chars: 0,
            secrets,
        }
    }

    fn mark_ready(&mut self, sidecar_version: Option<String>, pi_sdk_version: Option<String>) {
        self.started.sidecar_version = sidecar_version;
        self.started.pi_sdk_version = pi_sdk_version;
        self.ensure_started();
    }

    fn ensure_started(&mut self) {
        if self.started_emitted {
            return;
        }
        self.started_emitted = true;
        self.sink
            .emit(RuntimeTraceEvent::RequestStarted(self.started.clone()));
    }

    fn note_retry_started(&mut self, attempt: u32, max_attempts: u32, delay_ms: u64, error: &str) {
        self.ensure_started();
        self.retry_count = self.retry_count.max(attempt);
        let safe = sanitize_pi_diagnostic_line(error, &self.secrets);
        let (origin, category) = classify_runtime_error(None, &safe);
        self.sink.emit(RuntimeTraceEvent::Retry(RuntimeRetry {
            common: RuntimeTraceCommon::new(
                &self.started.common.surface,
                &self.started.common.request_id,
                "pi",
            ),
            attempt,
            max_attempts,
            delay_ms: Some(delay_ms),
            status: RuntimeRetryStatus::Started,
            failure_origin: Some(origin),
            error_category: Some(category),
            error_message: Some(safe),
        }));
    }

    fn note_retry_finished(&mut self, attempt: u32, success: bool, error: Option<&str>) {
        self.ensure_started();
        self.retry_count = self.retry_count.max(attempt);
        let safe = error.map(|message| sanitize_pi_diagnostic_line(message, &self.secrets));
        let classified = safe
            .as_deref()
            .map(|message| classify_runtime_error(None, message));
        self.sink.emit(RuntimeTraceEvent::Retry(RuntimeRetry {
            common: RuntimeTraceCommon::new(
                &self.started.common.surface,
                &self.started.common.request_id,
                "pi",
            ),
            attempt,
            max_attempts: 0,
            delay_ms: None,
            status: if success {
                RuntimeRetryStatus::Succeeded
            } else {
                RuntimeRetryStatus::Failed
            },
            failure_origin: classified.map(|value| value.0),
            error_category: classified.map(|value| value.1),
            error_message: safe,
        }));
    }

    fn finish_error(&mut self, code: &str, message: &str, stop_reason: &str, stderr: Vec<String>) {
        if self.finished {
            return;
        }
        self.ensure_started();
        self.finished = true;
        let safe = sanitize_pi_diagnostic_line(message, &self.secrets);
        let (origin, category) = classify_runtime_error(Some(code), &safe);
        self.sink
            .emit(RuntimeTraceEvent::RequestFinished(RuntimeRequestFinished {
                common: RuntimeTraceCommon::new(
                    &self.started.common.surface,
                    &self.started.common.request_id,
                    "pi",
                ),
                status: if category == RuntimeErrorCategory::Cancelled {
                    RuntimeTerminalStatus::Cancelled
                } else {
                    RuntimeTerminalStatus::Failed
                },
                provider_id: Some(self.started.provider_id.clone()),
                model_id: Some(self.started.model_id.clone()),
                stop_reason: Some(stop_reason.to_string()),
                failure_origin: Some(origin),
                error_category: Some(category),
                error_code: Some(code.to_string()),
                http_status: extract_http_status(code).or_else(|| extract_http_status(&safe)),
                error_message: Some(safe),
                elapsed_ms: self.started_at.elapsed().as_millis() as u64,
                turns: self.turns,
                tool_calls: self.tool_calls,
                tool_failures: self.tool_failures,
                retry_count: self.retry_count,
                text_chars: self.text_chars,
                reasoning_chars: self.reasoning_chars,
                stderr_tail: stderr,
                ..RuntimeRequestFinished::default()
            }));
    }

    fn finish_success(&mut self, stop_reason: &str, usage: &super::pi_protocol::PiUsage) {
        if self.finished {
            return;
        }
        self.ensure_started();
        self.finished = true;
        self.sink
            .emit(RuntimeTraceEvent::RequestFinished(RuntimeRequestFinished {
                common: RuntimeTraceCommon::new(
                    &self.started.common.surface,
                    &self.started.common.request_id,
                    "pi",
                ),
                status: RuntimeTerminalStatus::Completed,
                provider_id: Some(self.started.provider_id.clone()),
                model_id: Some(self.started.model_id.clone()),
                stop_reason: Some(stop_reason.to_string()),
                elapsed_ms: self.started_at.elapsed().as_millis() as u64,
                turns: self.turns,
                tool_calls: self.tool_calls,
                tool_failures: self.tool_failures,
                retry_count: self.retry_count,
                text_chars: self.text_chars,
                reasoning_chars: self.reasoning_chars,
                prompt_tokens: Some(usage.input.saturating_add(usage.cache_read)),
                completion_tokens: Some(usage.output),
                cache_read_tokens: Some(usage.cache_read),
                cache_write_tokens: Some(usage.cache_write),
                total_tokens: Some(usage.total_tokens),
                ..RuntimeRequestFinished::default()
            }));
    }
}

impl Drop for PiTraceSession {
    fn drop(&mut self) {
        if !self.finished {
            self.finish_error(
                "unclassified_early_return",
                "Pi Runtime 请求在写入结构化终态前结束",
                "error",
                Vec::new(),
            );
        }
    }
}

fn extract_http_status(value: &str) -> Option<u16> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .find_map(|part| {
            let status = part.parse::<u16>().ok()?;
            (400..=599).contains(&status).then_some(status)
        })
}

fn pi_model_secret_values(model: &PiModelConfig) -> Zeroizing<Vec<String>> {
    Zeroizing::new(match model.credential.as_ref() {
        Some(super::pi_protocol::PiCredential::ApiKey { key, env }) => key
            .iter()
            .chain(env.values())
            .filter(|value| !value.is_empty())
            .cloned()
            .collect(),
        Some(super::pi_protocol::PiCredential::OAuth {
            access, refresh, ..
        }) => vec![access.clone(), refresh.clone()],
        None => Vec::new(),
    })
}

fn sanitize_pi_diagnostic_line(line: &str, secrets: &[String]) -> String {
    let mut safe = line.replace(['\r', '\n', '\t'], " ");
    for secret in secrets.iter().filter(|secret| !secret.is_empty()) {
        safe = safe.replace(secret, "[REDACTED]");
    }
    static BEARER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static SK_TOKEN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static NAMED_SECRET: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    safe = BEARER
        .get_or_init(|| regex::Regex::new(r"(?i)bearer\s+[^\s,;]+").unwrap())
        .replace_all(&safe, "Bearer [REDACTED]")
        .into_owned();
    safe = SK_TOKEN
        .get_or_init(|| regex::Regex::new(r"(?i)\bsk-[a-z0-9._-]+").unwrap())
        .replace_all(&safe, "[REDACTED]")
        .into_owned();
    safe = NAMED_SECRET
        .get_or_init(|| {
            regex::Regex::new(r"(?i)(authorization|cookie|api[_ -]?key|token)\s*[:=]\s*[^\s,;]+")
                .unwrap()
        })
        .replace_all(&safe, "$1: [REDACTED]")
        .into_owned();
    crate::feedback::sanitize_paths(&safe.chars().take(2_000).collect::<String>())
}

fn actionable_pi_runtime_error(code: &str, safe_message: &str) -> String {
    let (_, category) = classify_runtime_error(Some(code), safe_message);
    match category {
        RuntimeErrorCategory::Authentication => format!(
            "Pi Provider 认证失败：{safe_message}。请前往「设置 → 大脑 → Pi Provider」重新配置凭据后重试。"
        ),
        RuntimeErrorCategory::ModelNotFound => format!(
            "Pi Provider 当前模型不可用：{safe_message}。请前往「设置 → 大脑 → Pi Provider」重新选择模型后重试。"
        ),
        _ => format!("Pi Sidecar {code}:{safe_message}"),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PiRuntimeHealth {
    pub sidecar_version: String,
    pub pi_sdk_version: String,
    pub protocol_version: u32,
    pub platform: String,
    pub arch: String,
    pub capabilities: PiRuntimeCapabilities,
}

#[derive(Debug, thiserror::Error)]
pub enum PiHealthError {
    #[error("Pi Runtime 不兼容:{0}")]
    Incompatible(String),
    #[error("Pi Runtime 健康检查失败:{0}")]
    Unhealthy(String),
}

#[derive(Debug, Clone)]
pub struct PiProcessCommand {
    program: PathBuf,
    args: Vec<OsString>,
}

impl PiProcessCommand {
    pub fn new(program: impl AsRef<Path>) -> Self {
        Self {
            program: program.as_ref().to_path_buf(),
            args: Vec::new(),
        }
    }
}

fn macos_version_supports_pi(version: &str) -> bool {
    version
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major >= PI_MINIMUM_MACOS_MAJOR)
}

pub(crate) fn ensure_pi_host_compatible() -> Result<(), PiHealthError> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .map_err(|error| PiHealthError::Unhealthy(error.to_string()))?;
        let version = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || !macos_version_supports_pi(version.trim()) {
            return Err(PiHealthError::Incompatible(
                "Pi Runtime 需要 macOS 13 或更高版本；当前系统过旧，请继续使用原生 Runtime".into(),
            ));
        }
    }
    Ok(())
}

pub async fn check_pi_runtime_health(
    binary: impl AsRef<Path>,
) -> Result<PiRuntimeHealth, PiHealthError> {
    ensure_pi_host_compatible()?;
    check_pi_runtime_health_with_command(PiProcessCommand::new(binary)).await
}

fn explain_sidecar_spawn_failure(program: &Path, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::NotFound {
        let state = if program.is_file() {
            "文件仍存在但 Windows 无法加载"
        } else {
            "文件在启动时已不存在"
        };
        return format!(
            "Pi Runtime 可执行文件启动失败（{state}）。可能被系统安全软件隔离或移除；请先继续使用原生 Runtime，检查安全软件的隔离记录后重新安装 CaseBoard"
        );
    }
    crate::feedback::sanitize_paths(&format!("Pi Runtime 启动失败:{error}"))
}

async fn check_pi_runtime_health_with_command(
    process: PiProcessCommand,
) -> Result<PiRuntimeHealth, PiHealthError> {
    let mut command = Command::new(&process.program);
    command
        .args(&process.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::proc_util::hide_console_window(&mut command);
    let mut child = command.spawn().map_err(|error| {
        PiHealthError::Unhealthy(explain_sidecar_spawn_failure(&process.program, &error))
    })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| PiHealthError::Unhealthy("stdin 不可用".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PiHealthError::Unhealthy("stdout 不可用".into()))?;
    write_json_line(
        &mut stdin,
        &PiHostMessage::HealthCheck {
            protocol_version: PI_PROTOCOL_VERSION,
        },
    )
    .await
    .map_err(|error| PiHealthError::Unhealthy(error.to_string()))?;
    drop(stdin);
    let mut stdout = BufReader::new(stdout);
    let line = tokio::time::timeout(Duration::from_secs(5), read_bounded_line(&mut stdout))
        .await
        .map_err(|_| PiHealthError::Unhealthy("握手超时".into()))?
        .map_err(|_| PiHealthError::Unhealthy("输出读取失败".into()))?
        .ok_or_else(|| PiHealthError::Unhealthy("没有 health 响应".into()))?;
    let message: PiSidecarMessage = serde_json::from_str(&line)
        .map_err(|_| PiHealthError::Unhealthy("health 响应不是有效 JSON".into()))?;
    let PiSidecarMessage::Health {
        protocol_version,
        sidecar_version,
        pi_sdk_version,
        platform,
        arch,
        capabilities,
    } = message
    else {
        return Err(PiHealthError::Unhealthy("响应类型不是 health".into()));
    };
    if protocol_version != PI_PROTOCOL_VERSION {
        return Err(PiHealthError::Incompatible(format!(
            "协议版本 {protocol_version}，当前需要 {PI_PROTOCOL_VERSION}"
        )));
    }
    let expected_platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let expected_arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    if platform != expected_platform || arch != expected_arch {
        return Err(PiHealthError::Incompatible(format!(
            "Runtime 平台为 {platform}/{arch}，当前设备为 {expected_platform}/{expected_arch}"
        )));
    }
    let _ = tokio::time::timeout(EXIT_GRACE, child.wait()).await;
    Ok(PiRuntimeHealth {
        sidecar_version,
        pi_sdk_version,
        protocol_version,
        platform,
        arch,
        capabilities,
    })
}

pub async fn run_pi_chat(
    config: &LlmConfig,
    request: AgentLoopRequest,
    registry: &ToolRegistry,
    ctx: ToolContext<'_>,
    tx: UnboundedSender<ChatStreamEvent>,
    control: ChatRunControl,
) -> Result<AgentLoopOutput, AgentLoopError> {
    ensure_pi_host_compatible()
        .map_err(|error| AgentLoopError::RuntimeUnavailable(error.to_string()))?;
    let resolved = resolve_pi_runtime_binary(ctx.app.as_ref()).map_err(|error| {
        AgentLoopError::RuntimeUnavailable(crate::feedback::sanitize_paths(&error.to_string()))
    })?;
    let tracked_version = (resolved.source == super::pi_locator::PiRuntimeSource::AppData)
        .then(|| resolved.version.clone())
        .flatten();
    let catalog = super::pi_catalog::load_pi_catalog(ctx.app.as_ref())
        .await
        .map_err(|error| AgentLoopError::RuntimeUnavailable(error.to_string()))?;
    let selection = PiRuntimeSelection::from_settings(ctx.settings, &catalog, &config.model)
        .map_err(|error| {
            AgentLoopError::RuntimeUnavailable(format!(
                "{error}。请前往「设置 → 大脑 → Pi Provider」重新选择并完成配置。"
            ))
        })?;
    let (selected_model, caseboard_custom_material, credential_type, credential_source) =
        if selection.provider_id == "caseboard-custom" {
            let material = config
                .issue_pi_credential_material()
                .await
                .map_err(AgentLoopError::RuntimeUnavailable)?;
            let credential_type = material
                .as_ref()
                .map(|_| "api_key".to_string())
                .or_else(|| Some("local_runtime".to_string()));
            (
                None,
                material,
                credential_type,
                Some("credential_bridge".to_string()),
            )
        } else {
            let resolved_credential =
            resolve_pi_credential(ctx.settings, &OsPiCredentialVault, &selection.provider_id)
                .map_err(AgentLoopError::RuntimeUnavailable)?
                .ok_or_else(|| {
                    AgentLoopError::RuntimeUnavailable(format!(
                        "Pi Provider「{}」尚未配置可用凭据。请前往「设置 → 大脑 → Pi Provider」完成配置后重试。",
                        selection.provider_id
                    ))
                })?;
            let credential_type = resolved_credential.credential_type().to_string();
            let credential_source = resolved_credential.source.as_str().to_string();
            (
                Some(PiModelConfig {
                    provider_id: selection.provider_id,
                    model_id: selection.model_id,
                    thinking_level: selection.thinking_level,
                    credential: Some(resolved_credential.credential),
                    caseboard_custom: None,
                }),
                None,
                Some(credential_type),
                Some(credential_source),
            )
        };
    let result = run_pi_chat_with_command_selected(
        PiChatProcess {
            process: PiProcessCommand::new(resolved.binary),
            selected_model,
            caseboard_custom_material,
            credential_type,
            credential_source,
            trace_sink: PiTraceSink::AppData,
        },
        config,
        request,
        registry,
        ctx,
        tx,
        control,
    )
    .await;
    if let Some(version) = tracked_version.as_deref() {
        match &result {
            Ok(_) => crate::pi_runtime_update::record_runtime_health_success(version),
            Err(AgentLoopError::RuntimeUnavailable(_)) => {
                crate::pi_runtime_update::record_runtime_handshake_failure(version)
            }
            _ => {}
        }
    }
    result
}

struct PiChatProcess {
    process: PiProcessCommand,
    selected_model: Option<PiModelConfig>,
    caseboard_custom_material: Option<crate::llm::LlmCredentialMaterial>,
    credential_type: Option<String>,
    credential_source: Option<String>,
    trace_sink: PiTraceSink,
}

async fn run_pi_chat_with_command_selected(
    mut invocation: PiChatProcess,
    config: &LlmConfig,
    request: AgentLoopRequest,
    registry: &ToolRegistry,
    ctx: ToolContext<'_>,
    tx: UnboundedSender<ChatStreamEvent>,
    control: ChatRunControl,
) -> Result<AgentLoopOutput, AgentLoopError> {
    let ChatRunControl {
        mut cancel,
        mut steering,
    } = control;
    let run_started = std::time::Instant::now();
    let mut activity_sequence = 1_u32;
    let _ = tx.send(ChatStreamEvent::Activity {
        activity: ChatActivity {
            runtime: "pi".into(),
            phase: ChatActivityPhase::Run,
            status: ChatActivityStatus::Started,
            sequence: activity_sequence,
            turn: None,
            tool: None,
            elapsed_ms: Some(0),
            error_category: None,
        },
    });
    let request_id = uuid::Uuid::new_v4().to_string();
    let caseboard_custom_credential = invocation
        .caseboard_custom_material
        .as_ref()
        .map(|material| material.with_secret(super::pi_protocol::PiCredential::api_key_material));
    let mut start = PiStartRequest::from_caseboard_with_model(
        &request_id,
        config,
        &request,
        registry,
        caseboard_custom_credential,
        invocation.selected_model.take(),
    )
    .map_err(AgentLoopError::RuntimeUnavailable)?;
    let expected_provider_id = start.model.provider_id.clone();
    let expected_model_id = start.model.model_id.clone();
    let mut trace = PiTraceSession::new(
        &start,
        request.max_tokens as u64,
        invocation.credential_type.clone(),
        invocation.credential_source.clone(),
        invocation.trace_sink.clone(),
    );
    let tool_schemas = registry.to_function_schemas();
    let prefix = PrefixFingerprint::compute(&request.system_prompt, &tool_schemas);
    // Pi SDK owns the ordinary agent loop. CaseBoard does not impose native iteration,
    // reasoning or duplicate-call caps. The idle fuse only catches a truly silent process:
    // reasoning deltas and SDK retries also reset the window — 模拟对抗等深推理任务可能
    // 连续数分钟只吐 reasoning,字节仍在流动就不是挂死(2026-07-27 真机误杀反馈)。
    // 失控的无限思考由 PI_EXTREME_DURATION_SECS 极端总时长兜底。
    let mut guard = PiSafetyGuard::with_idle_timeout(
        request.loop_guard_config.map(|config| config.idle_timeout),
    );
    let session = Arc::new(RwLock::new(SessionStats::default()));
    let hooks = HookChain::default_v0_2();
    let hook_context = HookContext::new(ctx.pool, ctx.settings, ctx.case_id, None, session.clone());

    let mut command = Command::new(&invocation.process.program);
    command
        .args(&invocation.process.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::proc_util::hide_console_window(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = explain_sidecar_spawn_failure(&invocation.process.program, &error);
            trace.finish_error("spawn_error", &message, "error", Vec::new());
            return Err(AgentLoopError::RuntimeUnavailable(message));
        }
    };
    let mut stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let message = "Pi Sidecar stdin 不可用";
            trace.finish_error("process_io", message, "error", Vec::new());
            return Err(AgentLoopError::RuntimeUnavailable(message.into()));
        }
    };
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let message = "Pi Sidecar stdout 不可用";
            trace.finish_error("process_io", message, "error", Vec::new());
            return Err(AgentLoopError::RuntimeUnavailable(message.into()));
        }
    };
    let stderr_secrets = trace.secrets.clone();
    let mut stderr_task = child.stderr.take().map(|stderr| {
        tokio::spawn(async move { collect_pi_stderr_tail(stderr, stderr_secrets).await })
    });
    let start_write = write_json_line(&mut stdin, &start).await;
    start.clear_runtime_credential();
    drop(invocation.caseboard_custom_material.take());
    if let Err(error) = start_write {
        trace.finish_error("process_io", &error.to_string(), "error", Vec::new());
        return Err(error);
    }

    let mut stdout = BufReader::new(stdout);
    let mut ready = false;
    let mut streamed_any = false;
    let mut tool_trace = Vec::new();
    let mut hall_detect_attempted = false;
    let mut ask_user = None;
    let mut steering_open = true;
    let completed = loop {
        if let Err(error) = guard.check() {
            let message = error.to_string();
            trace.finish_error("runtime_guard", &message, "error", Vec::new());
            return Err(AgentLoopError::Incomplete(message));
        }
        let wait = if ready {
            guard.wait_remaining()
        } else {
            READY_TIMEOUT
        };
        let line = tokio::select! {
            biased;
            _ = &mut cancel => {
                cancel_child(&mut child, &mut stdin, &request_id).await;
                let stderr = take_pi_stderr_tail(&mut stderr_task).await;
                trace.finish_error("cancelled", "用户取消了 Pi Runtime 请求", "cancelled", stderr);
                return Err(AgentLoopError::Cancelled);
            }
            guidance = steering.recv(), if steering_open => {
                match guidance {
                    Some(content) => {
                        write_json_line(&mut stdin, &PiHostMessage::Steer {
                            protocol_version: PI_PROTOCOL_VERSION,
                            request_id: request_id.clone(),
                            content,
                        })
                        .await
                        .map_err(|error| AgentLoopError::Parse(format!("发送 Pi 引导失败:{error}")))?;
                        guard.note_progress();
                    }
                    None => steering_open = false,
                }
                continue;
            }
            result = tokio::time::timeout(wait, read_bounded_line(&mut stdout)) => {
                match result {
                    Ok(Ok(line)) => line,
                    Ok(Err(error)) => {
                        let message = format!("Pi Sidecar 输出读取失败:{error}");
                        trace.finish_error("process_io", &message, "error", Vec::new());
                        return Err(AgentLoopError::Parse(message));
                    }
                    Err(_) if ready => {
                        let message = guard
                            .check()
                            .map_or_else(|error| error.to_string(), |_| "Pi Runtime 长时间没有任何运行事件".into());
                        trace.finish_error("timeout", &message, "error", Vec::new());
                        return Err(AgentLoopError::Incomplete(message));
                    }
                    Err(_) => {
                        let message = "Pi Sidecar 启动握手超时";
                        trace.finish_error("handshake_timeout", message, "error", Vec::new());
                        return Err(AgentLoopError::RuntimeUnavailable(message.into()));
                    }
                }
            }
        };
        let Some(line) = line else {
            let message = if streamed_any {
                "Pi Sidecar 在已有部分输出后意外退出"
            } else {
                "Pi Sidecar 在完成前意外退出"
            };
            let stderr = take_pi_stderr_tail(&mut stderr_task).await;
            trace.finish_error("process_exit", message, "error", stderr);
            return Err(AgentLoopError::Incomplete(message.into()));
        };
        let message: PiSidecarMessage = match serde_json::from_str(&line) {
            Ok(message) => message,
            Err(_) => {
                let error = "Pi Sidecar 输出不是有效协议 JSON";
                let stderr = take_pi_stderr_tail(&mut stderr_task).await;
                trace.finish_error("protocol_error", error, "error", stderr);
                return Err(AgentLoopError::Parse(error.into()));
            }
        };
        if let Err(error) = message.validate_for_request(&request_id) {
            trace.finish_error("protocol_error", &error, "error", Vec::new());
            return Err(AgentLoopError::Parse(error));
        }

        if !ready
            && !matches!(
                message,
                PiSidecarMessage::Ready { .. } | PiSidecarMessage::Error { .. }
            )
        {
            return Err(AgentLoopError::Parse(
                "Pi Sidecar 在 ready 前发送了业务事件".into(),
            ));
        }

        match message {
            PiSidecarMessage::Ready {
                sidecar_version,
                pi_sdk_version,
                ..
            } => {
                if ready {
                    trace.finish_error(
                        "protocol_error",
                        "Pi Sidecar 重复 ready",
                        "error",
                        Vec::new(),
                    );
                    return Err(AgentLoopError::Parse("Pi Sidecar 重复 ready".into()));
                }
                ready = true;
                trace.mark_ready(sidecar_version, pi_sdk_version);
                guard.note_progress();
            }
            PiSidecarMessage::TurnStart { .. } => {
                guard.note_turn();
                trace.turns = guard.turns();
                activity_sequence = activity_sequence.saturating_add(1);
                let _ = tx.send(ChatStreamEvent::Activity {
                    activity: ChatActivity {
                        runtime: "pi".into(),
                        phase: ChatActivityPhase::Turn,
                        status: ChatActivityStatus::Started,
                        sequence: activity_sequence,
                        turn: Some(guard.turns()),
                        tool: None,
                        elapsed_ms: Some(0),
                        error_category: None,
                    },
                });
            }
            PiSidecarMessage::TurnEnd { elapsed_ms, .. } => {
                guard.note_progress();
                activity_sequence = activity_sequence.saturating_add(1);
                let _ = tx.send(ChatStreamEvent::Activity {
                    activity: ChatActivity {
                        runtime: "pi".into(),
                        phase: ChatActivityPhase::Turn,
                        status: ChatActivityStatus::Completed,
                        sequence: activity_sequence,
                        turn: Some(guard.turns()),
                        tool: None,
                        elapsed_ms: Some(elapsed_ms),
                        error_category: None,
                    },
                });
            }
            PiSidecarMessage::Delta { content, .. } => {
                streamed_any |= !content.is_empty();
                if !content.is_empty() {
                    trace.text_chars = trace
                        .text_chars
                        .saturating_add(content.chars().count() as u64);
                    guard.note_progress();
                    let _ = tx.send(ChatStreamEvent::Delta { text: content });
                }
            }
            PiSidecarMessage::Reasoning { content, .. } => {
                guard.note_progress();
                trace.reasoning_chars = trace
                    .reasoning_chars
                    .saturating_add(content.chars().count() as u64);
                let _ = tx.send(ChatStreamEvent::Reasoning { text: content });
            }
            PiSidecarMessage::RetryStarted {
                attempt,
                max_attempts,
                delay_ms,
                error_message,
                ..
            } => {
                guard.note_progress();
                trace.note_retry_started(attempt, max_attempts, delay_ms, &error_message);
            }
            PiSidecarMessage::RetryFinished {
                attempt,
                success,
                error_message,
                ..
            } => {
                guard.note_progress();
                trace.note_retry_finished(attempt, success, error_message.as_deref());
            }
            PiSidecarMessage::ToolRequest {
                tool_call_id,
                tool,
                args,
                ..
            } => {
                guard.note_progress();
                trace.tool_calls = trace.tool_calls.saturating_add(1);
                activity_sequence = activity_sequence.saturating_add(1);
                let _ = tx.send(ChatStreamEvent::Activity {
                    activity: ChatActivity {
                        runtime: "pi".into(),
                        phase: ChatActivityPhase::Tool,
                        status: ChatActivityStatus::Started,
                        sequence: activity_sequence,
                        turn: Some(guard.turns()),
                        tool: Some(tool.clone()),
                        elapsed_ms: Some(0),
                        error_category: None,
                    },
                });
                let duplicate_hall_detect =
                    tool == "verify_legal_citations" && hall_detect_attempted;
                if tool == "verify_legal_citations" {
                    hall_detect_attempted = true;
                }
                let (result, executed) = if duplicate_hall_detect {
                    (
                        denied_result(
                            &tool_call_id,
                            &tool,
                            args,
                            "元典幻觉核验每轮最多调用一次；首次结果无论成功、失败或超时都不得自动重复扣费。".into(),
                        ),
                        false,
                    )
                } else {
                    match hooks
                        .run_before_tool_call(&tool, &args, &hook_context)
                        .await
                    {
                        HookOutcome::Deny(reason) => {
                            (denied_result(&tool_call_id, &tool, args, reason), false)
                        }
                        HookOutcome::Continue => {
                            let subtask = Subtask {
                                tool_call_id: tool_call_id.clone(),
                                tool: tool.clone(),
                                args,
                            };
                            let result = tokio::select! {
                                biased;
                                _ = &mut cancel => {
                                    cancel_child(&mut child, &mut stdin, &request_id).await;
                                    let stderr = take_pi_stderr_tail(&mut stderr_task).await;
                                    trace.finish_error("cancelled", "用户取消了 Pi Runtime 请求", "cancelled", stderr);
                                    return Err(AgentLoopError::Cancelled);
                                }
                                result = execute_registered_tool(subtask, registry, &ctx) => result,
                            };
                            (result, true)
                        }
                    }
                };
                if executed {
                    hooks
                        .run_after_tool_call(
                            &result.tool,
                            &ToolResult {
                                content: result.content.clone(),
                                yuandian_credits_used: result.credits_used,
                                kb_hit: result.kb_hit,
                            },
                            result.success,
                            &hook_context,
                        )
                        .await;
                }
                let record = subtask_record(&result);
                if !record.success {
                    trace.tool_failures = trace.tool_failures.saturating_add(1);
                }
                let _ = tx.send(ChatStreamEvent::ToolCall {
                    record: record.clone(),
                });
                activity_sequence = activity_sequence.saturating_add(1);
                let _ = tx.send(ChatStreamEvent::Activity {
                    activity: ChatActivity {
                        runtime: "pi".into(),
                        phase: ChatActivityPhase::Tool,
                        status: if record.success {
                            ChatActivityStatus::Completed
                        } else {
                            ChatActivityStatus::Failed
                        },
                        sequence: activity_sequence,
                        turn: Some(guard.turns()),
                        tool: Some(record.tool.clone()),
                        elapsed_ms: Some(
                            record.finished_at_ms.saturating_sub(record.started_at_ms) as u64,
                        ),
                        error_category: (!record.success).then(|| "tool".into()),
                    },
                });
                tool_trace.push(record);
                guard.note_progress();
                write_json_line(
                    &mut stdin,
                    &PiHostMessage::ToolResult {
                        protocol_version: PI_PROTOCOL_VERSION,
                        request_id: request_id.clone(),
                        tool_call_id: result.tool_call_id,
                        content: result.content,
                        is_error: !result.success,
                        kb_hit: result.kb_hit,
                        credits_used: result.credits_used,
                    },
                )
                .await?;
            }
            PiSidecarMessage::ToolComplete {
                tool_call_id,
                tool,
                is_error,
                ..
            } => {
                let _ = (tool_call_id, tool, is_error);
                guard.note_progress();
            }
            PiSidecarMessage::AskUser {
                tool_call_id, args, ..
            } => {
                let _ = tool_call_id;
                let questions = parse_ask_user_args(&args);
                if !questions.is_empty() {
                    guard.note_progress();
                    let _ = tx.send(ChatStreamEvent::AskUser {
                        questions: questions.clone(),
                    });
                    ask_user = Some(questions);
                }
            }
            PiSidecarMessage::Done {
                content,
                stop_reason,
                usage,
                ..
            } => {
                if stop_reason == "length" {
                    trace.finish_error(
                        "output_length",
                        "Pi Runtime 达到模型输出长度上限",
                        &stop_reason,
                        Vec::new(),
                    );
                    return Err(AgentLoopError::Incomplete(
                        "Pi Runtime 达到模型输出长度上限".into(),
                    ));
                }
                if content.trim().is_empty() && ask_user.is_none() {
                    trace.finish_error(
                        "empty_response",
                        "Pi Runtime 没有返回可用正文",
                        &stop_reason,
                        Vec::new(),
                    );
                    return Err(AgentLoopError::Incomplete(
                        "Pi Runtime 没有返回可用正文".into(),
                    ));
                }
                let _ = (usage.reasoning, usage.total_tokens);
                trace.finish_success(&stop_reason, &usage);
                guard.note_progress();
                activity_sequence = activity_sequence.saturating_add(1);
                let _ = tx.send(ChatStreamEvent::Activity {
                    activity: ChatActivity {
                        runtime: "pi".into(),
                        phase: ChatActivityPhase::Run,
                        status: ChatActivityStatus::Completed,
                        sequence: activity_sequence,
                        turn: Some(guard.turns()),
                        tool: None,
                        elapsed_ms: Some(run_started.elapsed().as_millis() as u64),
                        error_category: None,
                    },
                });
                break (content, usage);
            }
            PiSidecarMessage::Error { code, message, .. } => {
                activity_sequence = activity_sequence.saturating_add(1);
                let _ = tx.send(ChatStreamEvent::Activity {
                    activity: ChatActivity {
                        runtime: "pi".into(),
                        phase: ChatActivityPhase::Run,
                        status: ChatActivityStatus::Failed,
                        sequence: activity_sequence,
                        turn: Some(guard.turns()),
                        tool: None,
                        elapsed_ms: Some(run_started.elapsed().as_millis() as u64),
                        error_category: Some(code.clone()),
                    },
                });
                trace.finish_error(&code, &message, "error", Vec::new());
                let safe = sanitize_pi_diagnostic_line(&message, &trace.secrets);
                return Err(AgentLoopError::RuntimeUnavailable(
                    actionable_pi_runtime_error(&code, &safe),
                ));
            }
            PiSidecarMessage::Health {
                sidecar_version,
                pi_sdk_version,
                platform,
                arch,
                ..
            } => {
                let _ = (sidecar_version, pi_sdk_version, platform, arch);
                return Err(AgentLoopError::Parse("聊天请求收到了 health 响应".into()));
            }
            PiSidecarMessage::Catalog { .. } => {
                return Err(AgentLoopError::Parse("聊天请求收到了 catalog 响应".into()));
            }
            PiSidecarMessage::CredentialUpdate {
                provider_id,
                credential,
                ..
            } => {
                if provider_id != expected_provider_id {
                    return Err(AgentLoopError::Parse(
                        "Pi credential_update provider_id 不匹配".into(),
                    ));
                }
                OsPiCredentialVault
                    .write(&provider_id, &credential)
                    .map_err(AgentLoopError::RuntimeUnavailable)?;
                guard.note_progress();
            }
            PiSidecarMessage::AuthPrompt { .. }
            | PiSidecarMessage::AuthInfo { .. }
            | PiSidecarMessage::AuthUrl { .. }
            | PiSidecarMessage::AuthDeviceCode { .. }
            | PiSidecarMessage::AuthProgress { .. }
            | PiSidecarMessage::AuthSuccess { .. }
            | PiSidecarMessage::AuthError { .. }
            | PiSidecarMessage::AuthCancelled { .. } => {
                return Err(AgentLoopError::Parse("聊天请求收到了认证响应".into()));
            }
        }
    };

    drop(stdin);
    if tokio::time::timeout(EXIT_GRACE, child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    let _ = take_pi_stderr_tail(&mut stderr_task).await;

    let (final_content, pi_usage) = completed;
    let prompt_tokens = pi_usage
        .input
        .saturating_add(pi_usage.cache_read)
        .saturating_add(pi_usage.cache_write);
    let usage = ChatUsage {
        prompt_tokens: Some(prompt_tokens),
        completion_tokens: Some(pi_usage.output),
        model: expected_model_id,
    };
    hooks.run_after_llm_call(&usage, &hook_context).await;
    let _ = tx.send(ChatStreamEvent::Done {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        model: usage.model.clone(),
    });
    let parsed = crate::chat::citations::parse_with_doc_paths(
        &final_content,
        &request.case_doc_paths_for_citation_check,
    );
    let session_stats = session
        .read()
        .map(|stats| stats.clone())
        .unwrap_or_default();

    Ok(AgentLoopOutput {
        final_content,
        content_cleaned: parsed.content_cleaned,
        citations: parsed.citations,
        usage,
        tool_trace,
        iterations: guard.turns(),
        session_stats,
        metrics: CostMetrics {
            turns: guard.turns(),
            prompt_tokens,
            completion_tokens: pi_usage.output,
            cache_hit_tokens: pi_usage.cache_read,
            cache_miss_tokens: pi_usage.input,
            prefix_fp: prefix.short().to_string(),
            prefix_sys: prefix.system_short().to_string(),
            prefix_tools: prefix.tools_short().to_string(),
        },
        ask_user,
    })
}

fn denied_result(
    tool_call_id: &str,
    tool: &str,
    args: serde_json::Value,
    reason: String,
) -> SubtaskResult {
    let safe = crate::feedback::sanitize_paths(&reason);
    let now = chrono::Local::now().timestamp_millis();
    SubtaskResult {
        tool_call_id: tool_call_id.to_string(),
        tool: tool.to_string(),
        args,
        success: false,
        content: serde_json::json!({ "error": safe }).to_string(),
        kb_hit: false,
        credits_used: 0,
        error_short: Some(safe),
        result_preview: None,
        started_at_ms: now,
        finished_at_ms: now,
    }
}

fn subtask_record(result: &SubtaskResult) -> ToolCallRecord {
    ToolCallRecord {
        tool: result.tool.clone(),
        args: result.args.clone(),
        kb_hit: result.kb_hit,
        credits_used: result.credits_used,
        success: result.success,
        error_short: result.error_short.clone(),
        result_preview: result.result_preview.clone(),
        started_at_ms: result.started_at_ms,
        finished_at_ms: result.finished_at_ms,
    }
}

async fn write_json_line(
    stdin: &mut ChildStdin,
    message: &impl Serialize,
) -> Result<(), AgentLoopError> {
    let mut encoded = Zeroizing::new(
        serde_json::to_vec(message)
            .map_err(|_| AgentLoopError::Parse("Pi Sidecar 请求序列化失败".into()))?,
    );
    encoded.push(b'\n');
    stdin
        .write_all(&encoded)
        .await
        .map_err(|error| AgentLoopError::Network(format!("Pi Sidecar stdin 写入失败:{error}")))?;
    stdin
        .flush()
        .await
        .map_err(|error| AgentLoopError::Network(format!("Pi Sidecar stdin 刷新失败:{error}")))
}

async fn collect_pi_stderr_tail(
    stderr: tokio::process::ChildStderr,
    secrets: Zeroizing<Vec<String>>,
) -> Vec<String> {
    const MAX_LINES: usize = 32;
    const MAX_LINE_CHARS: usize = 1_000;
    const MAX_TOTAL_CHARS: usize = 16 * 1024;

    let mut lines = BufReader::new(stderr).lines();
    let mut tail = std::collections::VecDeque::new();
    let mut total_chars = 0_usize;
    while let Ok(Some(line)) = lines.next_line().await {
        let safe = sanitize_pi_diagnostic_line(&line, &secrets)
            .chars()
            .take(MAX_LINE_CHARS)
            .collect::<String>();
        total_chars = total_chars.saturating_add(safe.chars().count());
        tail.push_back(safe);
        while tail.len() > MAX_LINES || total_chars > MAX_TOTAL_CHARS {
            let Some(removed) = tail.pop_front() else {
                break;
            };
            total_chars = total_chars.saturating_sub(removed.chars().count());
        }
    }
    tail.into_iter().collect()
}

async fn take_pi_stderr_tail(
    task: &mut Option<tokio::task::JoinHandle<Vec<String>>>,
) -> Vec<String> {
    let Some(mut task) = task.take() else {
        return Vec::new();
    };
    match tokio::time::timeout(Duration::from_millis(200), &mut task).await {
        Ok(Ok(lines)) => lines,
        _ => {
            task.abort();
            Vec::new()
        }
    }
}

async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
            if line.len().saturating_add(position) > MAX_LINE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Pi Sidecar 单行消息超过 16 MiB",
                ));
            }
            line.extend_from_slice(&available[..position]);
            reader.consume(position + 1);
            break;
        }
        if line.len().saturating_add(available.len()) > MAX_LINE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Pi Sidecar 单行消息超过 16 MiB",
            ));
        }
        let length = available.len();
        line.extend_from_slice(available);
        reader.consume(length);
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line).map(Some).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "Pi Sidecar 输出不是 UTF-8")
    })
}

async fn cancel_child(child: &mut Child, stdin: &mut ChildStdin, request_id: &str) {
    let _ = write_json_line(
        stdin,
        &PiHostMessage::Cancel {
            protocol_version: PI_PROTOCOL_VERSION,
            request_id: request_id.to_string(),
        },
    )
    .await;
    if tokio::time::timeout(EXIT_GRACE, child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}
