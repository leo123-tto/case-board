//! Minimal non-streaming LLM gateway.
//!
//! P0 先承接首页问候、后续再逐步迁移抽取 / 整理 / agent_loop。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use super::capability::{OutputTokenParam, ProviderCapability};
use super::LlmConfig;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmChatMessage {
    pub role: String,
    pub content: String,
}

impl LlmChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NonStreamChatRequest {
    pub messages: Vec<LlmChatMessage>,
    pub max_output_tokens: u32,
    pub temperature: f32,
    pub timeout_secs: Option<u64>,
    pub response_format_json_object: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmGatewayUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub cache_hit_tokens: Option<u64>,
    pub cache_miss_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonStreamChatOutput {
    pub content: String,
    pub reasoning_content: Option<String>,
    pub finish_reason: Option<String>,
    pub model: Option<String>,
    pub usage: LlmGatewayUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LlmGatewayErrorKind {
    Auth,
    RateLimit,
    Timeout,
    ProviderSchema,
    ProviderUnavailable,
    Network,
    ResponseFormat,
}

#[derive(Debug, Error, Clone)]
#[error("{kind:?}: {message}")]
pub struct LlmGatewayError {
    pub kind: LlmGatewayErrorKind,
    pub message: String,
    pub status: Option<u16>,
}

impl LlmGatewayError {
    fn new(kind: LlmGatewayErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            status: None,
        }
    }

    fn http(status: u16, body: String) -> Self {
        let kind = match status {
            401 | 403 => LlmGatewayErrorKind::Auth,
            429 => LlmGatewayErrorKind::RateLimit,
            400 | 404 | 422 => LlmGatewayErrorKind::ProviderSchema,
            500..=599 => LlmGatewayErrorKind::ProviderUnavailable,
            _ => LlmGatewayErrorKind::ProviderSchema,
        };
        Self {
            kind,
            message: safe_error_snippet(&body),
            status: Some(status),
        }
    }
}

pub fn build_non_stream_chat_body(
    model: &str,
    messages: &[LlmChatMessage],
    max_output_tokens: u32,
    temperature: f32,
    capability: &ProviderCapability,
) -> Value {
    build_non_stream_chat_body_with_response_format(
        model,
        messages,
        max_output_tokens,
        temperature,
        false,
        capability,
    )
}

pub fn build_non_stream_chat_body_with_response_format(
    model: &str,
    messages: &[LlmChatMessage],
    max_output_tokens: u32,
    temperature: f32,
    response_format_json_object: bool,
    capability: &ProviderCapability,
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": messages,
        "temperature": temperature,
        "stream": false,
    });
    match capability.output_token_param {
        OutputTokenParam::MaxTokens => body["max_tokens"] = json!(max_output_tokens),
        OutputTokenParam::MaxCompletionTokens => {
            body["max_completion_tokens"] = json!(max_output_tokens)
        }
    }
    if response_format_json_object && capability.supports_json_object_response_format {
        body["response_format"] = json!({"type": "json_object"});
    }
    body
}

/// 429 退避参数:最多重试 5 次,基础间隔 800ms,指数增长并带轻微抖动避免羊群效应。
const MAX_RATE_LIMIT_RETRIES: u32 = 5;
const BASE_BACKOFF_MS: u64 = 800;

pub async fn complete_non_stream_chat(
    config: &LlmConfig,
    capability: &ProviderCapability,
    request: NonStreamChatRequest,
) -> Result<NonStreamChatOutput, LlmGatewayError> {
    if config.endpoint.trim().is_empty() {
        return Err(LlmGatewayError::new(
            LlmGatewayErrorKind::ProviderSchema,
            "LLM endpoint 未配置",
        ));
    }

    let timeout_secs = request.timeout_secs.unwrap_or(config.timeout_secs).max(1);
    let body = build_non_stream_chat_body_with_response_format(
        &config.model,
        &request.messages,
        request.max_output_tokens,
        request.temperature,
        request.response_format_json_object,
        capability,
    );

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| LlmGatewayError::new(LlmGatewayErrorKind::Network, e.to_string()))?;

    let mut last_error: Option<LlmGatewayError> = None;
    for attempt in 0..=MAX_RATE_LIMIT_RETRIES {
        if attempt > 0 {
            // 指数退避:800ms -> 1.6s -> 3.2s -> 6.4s -> 12.8s,加上基于 attempt 的抖动 0~300ms
            let jitter = (attempt as u64 * 73) % 300;
            let delay_ms = BASE_BACKOFF_MS * (1_u64 << (attempt - 1)) + jitter;
            crate::dlog!(
                "[llm gateway] 遇到 429 速率限制,第 {} 次退避 {} ms",
                attempt,
                delay_ms
            );
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }

        let mut req = client.post(&config.endpoint).json(&body);
        if let Some(key) = config
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            req = req.bearer_auth(key);
        }

        let response = req.send().await.map_err(|e| {
            if e.is_timeout() {
                LlmGatewayError::new(LlmGatewayErrorKind::Timeout, e.to_string())
            } else {
                LlmGatewayError::new(LlmGatewayErrorKind::Network, e.to_string())
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            let err_body = response.text().await.unwrap_or_default();
            let err = LlmGatewayError::http(status.as_u16(), err_body);
            if err.kind == LlmGatewayErrorKind::RateLimit && attempt < MAX_RATE_LIMIT_RETRIES {
                last_error = Some(err);
                continue;
            }
            return Err(err);
        }

        let value = response.json::<Value>().await.map_err(|e| {
            LlmGatewayError::new(LlmGatewayErrorKind::ResponseFormat, e.to_string())
        })?;
        return parse_non_stream_chat_response(value);
    }

    Err(last_error.unwrap_or_else(|| {
        LlmGatewayError::new(
            LlmGatewayErrorKind::RateLimit,
            "LLM 服务持续返回 429,已超过最大重试次数",
        )
    }))
}

fn parse_non_stream_chat_response(value: Value) -> Result<NonStreamChatOutput, LlmGatewayError> {
    let first_choice = value.get("choices").and_then(|v| v.get(0));
    let first_message = first_choice.and_then(|v| v.get("message"));
    let content = first_message
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            first_message
                .and_then(|m| m.get("reasoning_content"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
        })
        .or_else(|| value.get("text").and_then(|v| v.as_str()))
        .ok_or_else(|| {
            let finish_reason = first_choice
                .and_then(|c| c.get("finish_reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let usage = parse_usage(value.get("usage"));
            LlmGatewayError::new(
                LlmGatewayErrorKind::ResponseFormat,
                format!(
                    "响应无 content (finish_reason={}, completion_tokens={}, reasoning_tokens={})",
                    finish_reason,
                    usage.completion_tokens.unwrap_or(0),
                    usage.reasoning_tokens.unwrap_or(0)
                ),
            )
        })?;
    let reasoning_content = first_message
        .and_then(|m| m.get("reasoning_content"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let finish_reason = first_choice
        .and_then(|c| c.get("finish_reason"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let model = value
        .get("model")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    Ok(NonStreamChatOutput {
        content: content.to_string(),
        reasoning_content,
        finish_reason,
        model,
        usage: parse_usage(value.get("usage")),
    })
}

fn parse_usage(usage: Option<&Value>) -> LlmGatewayUsage {
    let Some(usage) = usage else {
        return LlmGatewayUsage::default();
    };
    let cached_tokens = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(|v| v.as_u64());
    let reasoning_tokens = usage
        .pointer("/completion_tokens_details/reasoning_tokens")
        .and_then(|v| v.as_u64());
    LlmGatewayUsage {
        prompt_tokens: usage.get("prompt_tokens").and_then(|v| v.as_u64()),
        completion_tokens: usage.get("completion_tokens").and_then(|v| v.as_u64()),
        cache_hit_tokens: usage
            .get("prompt_cache_hit_tokens")
            .and_then(|v| v.as_u64())
            .or(cached_tokens),
        cache_miss_tokens: usage
            .get("prompt_cache_miss_tokens")
            .and_then(|v| v.as_u64()),
        reasoning_tokens,
    }
}

fn safe_error_snippet(raw: &str) -> String {
    raw.replace(['\r', '\n', '\t'], " ")
        .chars()
        .take(800)
        .collect::<String>()
}
