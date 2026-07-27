use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::chat::agent_loop::AgentLoopRequest;
use crate::chat::tools::ToolRegistry;
use crate::llm::capability::{LlmProviderKind, OutputTokenParam, ProviderCapability};
use crate::llm::LlmConfig;

pub const PI_PROTOCOL_VERSION: u32 = 3;
const LOCAL_RUNTIME_KEY: &str = "caseboard-local-runtime";

#[derive(Clone, Serialize)]
pub struct PiStartRequest {
    #[serde(rename = "type")]
    message_type: &'static str,
    pub protocol_version: u32,
    pub request_id: String,
    pub system_prompt: String,
    pub history: Vec<PiHistoryMessage>,
    pub user_message: String,
    pub model: PiModelConfig,
    pub tools: Vec<PiToolDefinition>,
    pub skills: Vec<PiSkillDefinition>,
}

#[derive(Clone, Serialize)]
pub struct PiModelConfig {
    pub provider_id: String,
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<PiCredential>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caseboard_custom: Option<PiCustomModelConfig>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PiCredential {
    ApiKey {
        #[serde(skip_serializing_if = "Option::is_none")]
        key: Option<String>,
        #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
        env: std::collections::BTreeMap<String, String>,
    },
    #[serde(rename = "oauth")]
    OAuth {
        access: String,
        refresh: String,
        expires: u64,
        #[serde(flatten)]
        extra: std::collections::BTreeMap<String, Value>,
    },
}

impl fmt::Debug for PiCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey { .. } => f.write_str("PiCredential::ApiKey([REDACTED])"),
            Self::OAuth { .. } => f.write_str("PiCredential::OAuth([REDACTED])"),
        }
    }
}

#[derive(Clone, Serialize)]
pub struct PiCustomModelConfig {
    pub base_url: String,
    pub auth_header: bool,
    pub reasoning: bool,
    pub context_window: u32,
    pub max_tokens: u32,
    pub temperature: f64,
    pub headers: std::collections::BTreeMap<String, String>,
    pub compat: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct PiHistoryMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PiToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    pub mutating: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PiSkillDefinition {
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub base_dir: String,
    pub source: String,
    pub version: String,
    pub sha256: String,
}

impl fmt::Debug for PiStartRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PiStartRequest")
            .field("protocol_version", &self.protocol_version)
            .field("request_id", &self.request_id)
            .field("system_prompt_chars", &self.system_prompt.chars().count())
            .field("history_messages", &self.history.len())
            .field(
                "history_chars",
                &self
                    .history
                    .iter()
                    .map(|message| message.content.chars().count())
                    .sum::<usize>(),
            )
            .field("user_message_chars", &self.user_message.chars().count())
            .field("provider", &self.model.provider_id)
            .field("model", &self.model.model_id)
            .field("credential", &"[REDACTED]")
            .field("tools", &self.tools.len())
            .field("skills", &self.skills.len())
            .finish()
    }
}

impl PiStartRequest {
    pub fn from_caseboard(
        request_id: impl Into<String>,
        config: &LlmConfig,
        request: &AgentLoopRequest,
        registry: &ToolRegistry,
        credential: Option<PiCredential>,
    ) -> Result<Self, String> {
        let capability = ProviderCapability::from_backend("", &config.endpoint, &config.model);
        let (credential, auth_header) = match credential {
            Some(credential) => (credential, true),
            None => (
                PiCredential::ApiKey {
                    key: Some(LOCAL_RUNTIME_KEY.to_string()),
                    env: std::collections::BTreeMap::new(),
                },
                false,
            ),
        };
        let reasoning_model = {
            let model = config.model.to_ascii_lowercase();
            model.contains("thinking")
                || model.contains("reason")
                || model.contains("-pro")
                || model == "deepseek-v4-flash"
        };
        let history = request
            .history
            .iter()
            .filter(|(role, _)| role == "user" || role == "assistant")
            .map(|(role, content)| PiHistoryMessage {
                role: role.clone(),
                content: content.clone(),
            })
            .collect();
        let tools = registry
            .iter()
            .map(|tool| PiToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters_schema(),
                mutating: tool.is_mutating(),
            })
            .collect();
        let skills = crate::chat::skills::load_all()?
            .into_iter()
            .map(|skill| PiSkillDefinition {
                name: skill.summary.name,
                description: skill.summary.description,
                base_dir: skill
                    .file_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(""))
                    .to_string_lossy()
                    .into_owned(),
                file_path: skill.file_path.to_string_lossy().into_owned(),
                source: skill.summary.source,
                version: skill.summary.version,
                sha256: skill.summary.sha256,
            })
            .collect();
        let mut base_url = completions_base_url(&config.endpoint);
        if capability.kind == LlmProviderKind::DeepSeek && base_url.ends_with("/v1") {
            base_url.truncate(base_url.len() - 3);
            base_url.push_str("/beta");
        }
        if base_url.is_empty() {
            return Err("Pi Runtime 的模型 endpoint 为空".into());
        }
        let max_tokens_field = match capability.output_token_param {
            OutputTokenParam::MaxTokens => "max_tokens",
            OutputTokenParam::MaxCompletionTokens => "max_completion_tokens",
        };

        Ok(Self {
            message_type: "start",
            protocol_version: PI_PROTOCOL_VERSION,
            request_id: request_id.into(),
            system_prompt: request.system_prompt.clone(),
            history,
            user_message: request.user_message.clone(),
            model: PiModelConfig {
                provider_id: "caseboard-custom".into(),
                model_id: config.model.clone(),
                thinking_level: None,
                credential: Some(credential),
                caseboard_custom: Some(PiCustomModelConfig {
                    base_url,
                    auth_header,
                    reasoning: reasoning_model,
                    context_window: 160_000,
                    max_tokens: request.max_tokens,
                    temperature: capability.normalize_temperature(request.temperature),
                    headers: std::collections::BTreeMap::new(),
                    compat: serde_json::json!({
                        "maxTokensField": max_tokens_field,
                        "supportsUsageInStreaming": capability.supports_stream_usage,
                        "requiresReasoningContentOnAssistantMessages": capability.requires_reasoning_replay_for_tool_calls,
                    }),
                }),
            },
            tools,
            skills,
        })
    }
}

fn completions_base_url(endpoint: &str) -> String {
    let endpoint = endpoint.trim().trim_end_matches('/');
    for suffix in ["/chat/completions", "/beta/chat/completions"] {
        if let Some(base) = endpoint.strip_suffix(suffix) {
            return base.to_string();
        }
    }
    endpoint.to_string()
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PiHostMessage {
    HealthCheck {
        protocol_version: u32,
    },
    CatalogRequest {
        protocol_version: u32,
    },
    AuthStart {
        protocol_version: u32,
        request_id: String,
        provider_id: String,
        auth_type: String,
    },
    AuthPromptResponse {
        protocol_version: u32,
        request_id: String,
        prompt_id: String,
        value: String,
    },
    AuthCancel {
        protocol_version: u32,
        request_id: String,
    },
    ToolResult {
        protocol_version: u32,
        request_id: String,
        tool_call_id: String,
        content: String,
        is_error: bool,
        kb_hit: bool,
        credits_used: u32,
    },
    Cancel {
        protocol_version: u32,
        request_id: String,
    },
    Steer {
        protocol_version: u32,
        request_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PiSidecarMessage {
    Ready {
        protocol_version: u32,
        request_id: String,
        #[serde(default)]
        sidecar_version: Option<String>,
        #[serde(default)]
        pi_sdk_version: Option<String>,
    },
    TurnStart {
        protocol_version: u32,
        request_id: String,
    },
    TurnEnd {
        protocol_version: u32,
        request_id: String,
        elapsed_ms: u64,
    },
    Delta {
        protocol_version: u32,
        request_id: String,
        content: String,
    },
    Reasoning {
        protocol_version: u32,
        request_id: String,
        content: String,
    },
    RetryStarted {
        protocol_version: u32,
        request_id: String,
        attempt: u32,
        max_attempts: u32,
        delay_ms: u64,
        error_message: String,
    },
    RetryFinished {
        protocol_version: u32,
        request_id: String,
        attempt: u32,
        success: bool,
        error_message: Option<String>,
    },
    ToolRequest {
        protocol_version: u32,
        request_id: String,
        tool_call_id: String,
        tool: String,
        args: Value,
    },
    ToolComplete {
        protocol_version: u32,
        request_id: String,
        tool_call_id: String,
        tool: String,
        is_error: bool,
    },
    AskUser {
        protocol_version: u32,
        request_id: String,
        tool_call_id: String,
        args: Value,
    },
    Done {
        protocol_version: u32,
        request_id: String,
        content: String,
        stop_reason: String,
        usage: PiUsage,
    },
    Error {
        protocol_version: u32,
        request_id: Option<String>,
        code: String,
        message: String,
    },
    Health {
        protocol_version: u32,
        sidecar_version: String,
        pi_sdk_version: String,
        platform: String,
        arch: String,
        #[serde(default)]
        capabilities: PiRuntimeCapabilities,
    },
    Catalog {
        protocol_version: u32,
        providers: Vec<PiProviderSummary>,
    },
    CredentialUpdate {
        protocol_version: u32,
        request_id: String,
        provider_id: String,
        credential: PiCredential,
    },
    AuthPrompt {
        protocol_version: u32,
        request_id: String,
        prompt_id: String,
        prompt_type: String,
        message: String,
        placeholder: Option<String>,
        options: Option<Vec<PiAuthOption>>,
    },
    AuthInfo {
        protocol_version: u32,
        request_id: String,
        message: String,
        links: Option<Vec<PiAuthLink>>,
    },
    AuthUrl {
        protocol_version: u32,
        request_id: String,
        url: String,
        instructions: Option<String>,
    },
    AuthDeviceCode {
        protocol_version: u32,
        request_id: String,
        user_code: String,
        verification_uri: String,
        interval_seconds: Option<u64>,
        expires_in_seconds: Option<u64>,
    },
    AuthProgress {
        protocol_version: u32,
        request_id: String,
        message: String,
    },
    AuthSuccess {
        protocol_version: u32,
        request_id: String,
        provider_id: String,
        credential: PiCredential,
    },
    AuthError {
        protocol_version: u32,
        request_id: String,
        message: String,
    },
    AuthCancelled {
        protocol_version: u32,
        request_id: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PiRuntimeCapabilities {
    pub subagents: Option<PiSubagentsCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PiSubagentsCapability {
    pub package: String,
    pub version: String,
    pub child_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiAuthOption {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiAuthLink {
    pub url: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiProviderCatalog {
    pub providers: Vec<PiProviderSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiProviderSummary {
    pub id: String,
    pub name: String,
    pub auth_types: Vec<String>,
    pub models: Vec<PiModelSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiModelSummary {
    pub id: String,
    pub name: String,
    pub api: String,
    pub reasoning: bool,
    #[serde(default)]
    pub thinking_levels: Vec<String>,
    pub context_window: u32,
    pub max_tokens: u32,
    pub input: Vec<String>,
}

impl PiSidecarMessage {
    pub fn validate_for_request(&self, expected: &str) -> Result<(), String> {
        let (version, request_id) = match self {
            Self::Ready {
                protocol_version,
                request_id,
                ..
            }
            | Self::TurnStart {
                protocol_version,
                request_id,
            }
            | Self::TurnEnd {
                protocol_version,
                request_id,
                ..
            }
            | Self::Delta {
                protocol_version,
                request_id,
                ..
            }
            | Self::Reasoning {
                protocol_version,
                request_id,
                ..
            }
            | Self::RetryStarted {
                protocol_version,
                request_id,
                ..
            }
            | Self::RetryFinished {
                protocol_version,
                request_id,
                ..
            }
            | Self::ToolRequest {
                protocol_version,
                request_id,
                ..
            }
            | Self::ToolComplete {
                protocol_version,
                request_id,
                ..
            }
            | Self::AskUser {
                protocol_version,
                request_id,
                ..
            }
            | Self::Done {
                protocol_version,
                request_id,
                ..
            }
            | Self::CredentialUpdate {
                protocol_version,
                request_id,
                ..
            }
            | Self::AuthPrompt {
                protocol_version,
                request_id,
                ..
            }
            | Self::AuthInfo {
                protocol_version,
                request_id,
                ..
            }
            | Self::AuthUrl {
                protocol_version,
                request_id,
                ..
            }
            | Self::AuthDeviceCode {
                protocol_version,
                request_id,
                ..
            }
            | Self::AuthProgress {
                protocol_version,
                request_id,
                ..
            }
            | Self::AuthSuccess {
                protocol_version,
                request_id,
                ..
            }
            | Self::AuthError {
                protocol_version,
                request_id,
                ..
            }
            | Self::AuthCancelled {
                protocol_version,
                request_id,
            } => (*protocol_version, Some(request_id.as_str())),
            Self::Error {
                protocol_version,
                request_id,
                ..
            } => (*protocol_version, request_id.as_deref()),
            Self::Health {
                protocol_version, ..
            }
            | Self::Catalog {
                protocol_version, ..
            } => (*protocol_version, None),
        };
        if version != PI_PROTOCOL_VERSION {
            return Err("Pi Sidecar 协议版本不匹配".into());
        }
        if request_id.is_some_and(|id| id != expected) {
            return Err("Pi Sidecar request_id 不匹配".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PiUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
    pub total_tokens: u64,
}
