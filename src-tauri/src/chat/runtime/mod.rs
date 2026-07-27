use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::{
    mpsc::{UnboundedReceiver, UnboundedSender},
    oneshot,
};

use super::agent_loop::{run_chat_with_tools, AgentLoopError, AgentLoopOutput, AgentLoopRequest};
use super::stream::ChatStreamEvent;
use super::tools::{ToolContext, ToolRegistry};
use crate::llm::LlmConfig;
use crate::settings::Settings;

pub(crate) mod pi_auth;
pub(crate) mod pi_catalog;
pub(crate) mod pi_credentials;
pub(crate) mod pi_locator;
pub(crate) mod pi_protocol;
pub(crate) mod pi_safety;
pub(crate) mod pi_sidecar;

pub struct ChatRunControl {
    pub cancel: oneshot::Receiver<()>,
    pub steering: UnboundedReceiver<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PiRuntimeStatus {
    pub state: &'static str,
    pub available: bool,
    pub source: Option<pi_locator::PiRuntimeSource>,
    pub installed_version: Option<String>,
    pub sidecar_version: Option<String>,
    pub pi_sdk_version: Option<String>,
    pub protocol_version: u32,
    pub platform: Option<String>,
    pub arch: Option<String>,
    pub error_category: Option<&'static str>,
    pub message: Option<String>,
}

pub async fn get_pi_runtime_status(app: &tauri::AppHandle) -> PiRuntimeStatus {
    let resolved = match pi_locator::resolve_pi_runtime_binary(Some(app)) {
        Ok(resolved) => resolved,
        Err(pi_locator::PiRuntimeError::Missing) => {
            return PiRuntimeStatus {
                state: "missing",
                available: false,
                source: None,
                installed_version: None,
                sidecar_version: None,
                pi_sdk_version: None,
                protocol_version: pi_protocol::PI_PROTOCOL_VERSION,
                platform: None,
                arch: None,
                error_category: Some("missing"),
                message: Some("当前安装包未包含 Pi Runtime，也没有已安装的独立 Runtime".into()),
            };
        }
        Err(error) => {
            return unavailable_status("unhealthy", "resolve", error.to_string());
        }
    };
    match pi_sidecar::check_pi_runtime_health(&resolved.binary).await {
        Ok(health) => {
            if let (pi_locator::PiRuntimeSource::AppData, Some(version)) =
                (resolved.source, resolved.version.as_deref())
            {
                crate::pi_runtime_update::record_runtime_health_success(version);
            }
            PiRuntimeStatus {
                state: "available",
                available: true,
                source: Some(resolved.source),
                installed_version: resolved.version,
                sidecar_version: Some(health.sidecar_version),
                pi_sdk_version: Some(health.pi_sdk_version),
                protocol_version: health.protocol_version,
                platform: Some(health.platform),
                arch: Some(health.arch),
                error_category: None,
                message: None,
            }
        }
        Err(pi_sidecar::PiHealthError::Incompatible(message)) => {
            if let (pi_locator::PiRuntimeSource::AppData, Some(version)) =
                (resolved.source, resolved.version.as_deref())
            {
                crate::pi_runtime_update::record_runtime_handshake_failure(version);
            }
            let mut status = unavailable_status("incompatible", "incompatible", message);
            status.source = Some(resolved.source);
            status.installed_version = resolved.version;
            status
        }
        Err(pi_sidecar::PiHealthError::Unhealthy(message)) => {
            if let (pi_locator::PiRuntimeSource::AppData, Some(version)) =
                (resolved.source, resolved.version.as_deref())
            {
                crate::pi_runtime_update::record_runtime_handshake_failure(version);
            }
            let mut status = unavailable_status("unhealthy", "health", message);
            status.source = Some(resolved.source);
            status.installed_version = resolved.version;
            status
        }
    }
}

pub async fn get_pi_provider_catalog(
    app: &tauri::AppHandle,
) -> Result<pi_protocol::PiProviderCatalog, String> {
    pi_catalog::load_pi_catalog(Some(app))
        .await
        .map_err(|error| error.to_string())
}

pub fn get_pi_credential_status(
    provider_id: &str,
) -> Result<pi_credentials::PiCredentialStatus, String> {
    use pi_credentials::PiCredentialVault;
    pi_credentials::OsPiCredentialVault.status(provider_id)
}

pub async fn begin_pi_provider_auth(
    app: &tauri::AppHandle,
    provider_id: String,
    auth_type: String,
    login_method: Option<String>,
) -> Result<String, String> {
    pi_auth::begin_pi_provider_auth(app, provider_id, auth_type, login_method).await
}

pub fn respond_pi_provider_auth(
    auth_session_id: &str,
    prompt_id: &str,
    value: String,
) -> Result<(), String> {
    pi_auth::respond_pi_provider_auth(auth_session_id, prompt_id, value)
}

pub fn cancel_pi_provider_auth(auth_session_id: &str) -> Result<(), String> {
    pi_auth::cancel_pi_provider_auth(auth_session_id)
}

pub fn remove_pi_provider_credential(provider_id: &str) -> Result<(), String> {
    use pi_credentials::PiCredentialVault;
    pi_credentials::OsPiCredentialVault.delete(provider_id)
}

#[derive(Debug, Clone, Serialize)]
pub struct PiProviderVerificationResult {
    pub ok: bool,
    pub message: String,
    pub provider_id: String,
    pub model_id: String,
    pub credential_type: Option<String>,
    pub latency_ms: u64,
}

pub async fn verify_pi_provider(
    app: &tauri::AppHandle,
    pool: &SqlitePool,
    requested_provider_id: &str,
    requested_model_id: &str,
    requested_thinking_level: Option<&str>,
) -> Result<PiProviderVerificationResult, String> {
    use crate::chat::context::TaskType;

    let mut settings = crate::settings::read_settings().unwrap_or_default();
    let provider_id = requested_provider_id.trim();
    if provider_id.is_empty() {
        return Err("请先选择 Pi Provider".to_string());
    }
    let provider_id = provider_id.to_string();
    let model_id = requested_model_id.trim();
    if model_id.is_empty() {
        return Err("请先选择 Pi 模型".to_string());
    }
    let model_id = model_id.to_string();
    settings.pi_provider_id = Some(provider_id.clone());
    settings.pi_model_id = Some(model_id.clone());
    if let Some(thinking_level) = requested_thinking_level
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        settings
            .pi_model_thinking_levels
            .entry(provider_id.clone())
            .or_default()
            .insert(model_id.clone(), thinking_level.to_string());
    }
    let credential_type = get_pi_credential_status(&provider_id)?.credential_type;
    let config = LlmConfig::from_settings(&settings);
    let request = AgentLoopRequest {
        task_type: TaskType::FreeChat,
        system_prompt: "这是一次 CaseBoard Provider 连通性验证。不要调用工具，只回复 OK。".into(),
        history: Vec::new(),
        user_message: "只回复 OK".into(),
        temperature: 0.0,
        max_tokens: 16,
        tool_choice: "none".into(),
        case_doc_paths_for_citation_check: Vec::new(),
        loop_guard_config: None,
        emit_turn_progress: false,
        tool_call_budget_config: None,
    };
    let registry = ToolRegistry::empty();
    let context = ToolContext {
        pool,
        settings: &settings,
        case_id: None,
        local_kb: None,
        app: Some(app.clone()),
        message_id: None,
        visualization_consent: false,
    };
    let (stream_tx, _stream_rx) = tokio::sync::mpsc::unbounded_channel();
    let (_cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let (_steering_tx, steering_rx) = tokio::sync::mpsc::unbounded_channel();
    let started = std::time::Instant::now();
    match pi_sidecar::run_pi_chat(
        &config,
        request,
        &registry,
        context,
        stream_tx,
        ChatRunControl {
            cancel: cancel_rx,
            steering: steering_rx,
        },
    )
    .await
    {
        Ok(output) => Ok(PiProviderVerificationResult {
            ok: !output.content_cleaned.trim().is_empty(),
            message: "真实模型请求已成功返回".into(),
            provider_id,
            model_id,
            credential_type,
            latency_ms: started.elapsed().as_millis() as u64,
        }),
        Err(error) => Ok(PiProviderVerificationResult {
            ok: false,
            message: error.to_string(),
            provider_id,
            model_id,
            credential_type,
            latency_ms: started.elapsed().as_millis() as u64,
        }),
    }
}

fn unavailable_status(
    state: &'static str,
    category: &'static str,
    message: String,
) -> PiRuntimeStatus {
    PiRuntimeStatus {
        state,
        available: false,
        source: None,
        installed_version: None,
        sidecar_version: None,
        pi_sdk_version: None,
        protocol_version: pi_protocol::PI_PROTOCOL_VERSION,
        platform: None,
        arch: None,
        error_category: Some(category),
        message: Some(crate::feedback::sanitize_paths(&message)),
    }
}

/// 用户可选的 Agent Runtime。缺失、原生和未知值都保持既有原生行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRuntimeKind {
    Native,
    Pi,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiRuntimeSelection {
    pub provider_id: String,
    pub model_id: String,
    pub thinking_level: Option<String>,
}

const PI_THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

fn clamp_pi_thinking_level(model: &pi_protocol::PiModelSummary, requested: &str) -> String {
    let supported = if model.thinking_levels.is_empty() {
        if model.reasoning {
            vec!["off", "minimal", "low", "medium", "high"]
        } else {
            vec!["off"]
        }
    } else {
        model
            .thinking_levels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    };
    if supported.contains(&requested) {
        return requested.to_string();
    }
    let requested_index = PI_THINKING_LEVELS
        .iter()
        .position(|candidate| *candidate == requested)
        .unwrap_or(3);
    for candidate in PI_THINKING_LEVELS.iter().skip(requested_index) {
        if supported.contains(candidate) {
            return (*candidate).to_string();
        }
    }
    for candidate in PI_THINKING_LEVELS[..requested_index].iter().rev() {
        if supported.contains(candidate) {
            return (*candidate).to_string();
        }
    }
    supported.first().copied().unwrap_or("off").to_string()
}

fn selected_pi_thinking_level(
    settings: &Settings,
    provider_id: &str,
    model: &pi_protocol::PiModelSummary,
) -> String {
    let requested = settings
        .pi_model_thinking_levels
        .get(provider_id)
        .and_then(|models| models.get(&model.id))
        .map(String::as_str)
        .unwrap_or(if model.reasoning { "medium" } else { "off" });
    clamp_pi_thinking_level(model, requested)
}

impl PiRuntimeSelection {
    pub fn from_settings(
        settings: &Settings,
        catalog: &pi_protocol::PiProviderCatalog,
        fallback_model: &str,
    ) -> Result<Self, String> {
        let explicit_provider = settings
            .pi_provider_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let explicit_model = settings
            .pi_model_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if explicit_provider.is_some() != explicit_model.is_some() {
            return Err("Pi provider 与模型必须同时选择".into());
        }
        if let (Some(provider_id), Some(model_id)) = (explicit_provider, explicit_model) {
            let thinking_level = if provider_id == "caseboard-custom" {
                None
            } else {
                let model = pi_catalog::validate_selection(catalog, provider_id, model_id)?;
                Some(selected_pi_thinking_level(settings, provider_id, model))
            };
            return Ok(Self {
                provider_id: provider_id.into(),
                model_id: model_id.into(),
                thinking_level,
            });
        }

        // “跟随现有云端配置”是协议兼容模式：Agent Loop 由 Pi 执行，但模型 endpoint、
        // model 与 API Key 全部沿用 CaseBoard 右侧云端配置。不要把它暗中改写成 Pi 目录
        // provider，否则会错误要求第二份 Pi 凭据，并让界面文案与真实路由不一致。
        Ok(Self {
            provider_id: "caseboard-custom".into(),
            model_id: fallback_model.into(),
            thinking_level: None,
        })
    }
}

/// 当前 AI 助手请求是否需要右侧云端模型配置。
///
/// Native 始终需要；Pi 只有在未显式选择 provider/model 的兼容模式下需要。
pub fn uses_cloud_model_config(settings: &Settings) -> bool {
    if AgentRuntimeKind::from_settings(settings) == AgentRuntimeKind::Native {
        return true;
    }
    let provider = settings
        .pi_provider_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    let model = settings
        .pi_model_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty());
    !provider && !model
}

/// 在创建用户消息、启动 Agent Loop 之前校验 AI 助手真正会走的模型配置。
/// Native / Pi 兼容模式检查“材料处理模型”；Pi 独立 Provider 检查 provider+model 和凭据库。
pub async fn validate_ai_assistant_ready(settings: &Settings) -> Result<(), String> {
    if uses_cloud_model_config(settings) {
        settings
            .validate_material_llm_non_secret_config()
            .map_err(|error| format!("AI 助手当前使用材料处理模型，但{error}"))?;
        return LlmConfig::from_settings(settings)
            .ensure_material_ready()
            .await
            .map_err(|error| format!("AI 助手当前使用材料处理模型，但{error}"));
    }
    validate_ai_assistant_ready_with_vault(settings, &pi_credentials::OsPiCredentialVault)
}

fn validate_ai_assistant_ready_with_vault(
    settings: &Settings,
    vault: &dyn pi_credentials::PiCredentialVault,
) -> Result<(), String> {
    const SETTINGS_HINT: &str = "请前往「设置 → 大脑 → Pi Provider」完成配置后重试。";
    let provider_id = settings
        .pi_provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let model_id = settings
        .pi_model_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (provider_id, _model_id) = match (provider_id, model_id) {
        (Some(provider_id), Some(model_id)) => (provider_id, model_id),
        _ => {
            return Err(format!(
                "AI 助手的 Pi Provider 和模型必须同时选择。{SETTINGS_HINT}"
            ));
        }
    };
    let credential = pi_credentials::resolve_pi_credential(settings, vault, provider_id)
        .map_err(|error| format!("无法读取 Pi Provider 凭据：{error}。{SETTINGS_HINT}"))?;
    if credential.is_none() {
        return Err(format!(
            "Pi Provider「{provider_id}」尚未配置可用凭据，或现有 API Key 尚未验证成功。{SETTINGS_HINT}"
        ));
    }
    Ok(())
}

impl AgentRuntimeKind {
    pub fn from_settings(settings: &Settings) -> Self {
        match settings.agent_runtime.as_deref().map(str::trim) {
            Some("pi") => Self::Pi,
            _ => Self::Native,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Pi => "pi",
        }
    }
}

/// 所有用户聊天共用的 Runtime 分发边界。
///
/// Pi 尚未安装/接通时明确报错，不静默切回原生；这样设置里的 A/B 对比才可信。
pub async fn run_chat_with_runtime(
    runtime: AgentRuntimeKind,
    config: &LlmConfig,
    req: AgentLoopRequest,
    registry: &ToolRegistry,
    ctx: ToolContext<'_>,
    tx: UnboundedSender<ChatStreamEvent>,
    control: ChatRunControl,
) -> Result<AgentLoopOutput, AgentLoopError> {
    match runtime {
        AgentRuntimeKind::Native => {
            run_chat_with_tools(
                config,
                req,
                registry,
                ctx,
                tx,
                control.cancel,
                control.steering,
            )
            .await
        }
        AgentRuntimeKind::Pi => {
            pi_sidecar::run_pi_chat(config, req, registry, ctx, tx, control).await
        }
    }
}
