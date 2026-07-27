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
        "temperature": capability.normalize_temperature(temperature),
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

    const MAX_ATTEMPTS: u32 = 3;
    let mut last_error = None;
    for attempt in 0..MAX_ATTEMPTS {
        let (req, credential) = config
            .authorize_request(client.post(&config.endpoint).json(&body))
            .await
            .map_err(|error| LlmGatewayError::new(LlmGatewayErrorKind::Auth, error))?;

        let response = match req.send().await {
            Ok(response) => response,
            Err(error) => {
                let safe_error = credential.as_ref().map_or_else(
                    || error.to_string(),
                    |value| value.redact(&error.to_string()),
                );
                let mapped = if error.is_timeout() {
                    LlmGatewayError::new(LlmGatewayErrorKind::Timeout, safe_error)
                } else {
                    LlmGatewayError::new(LlmGatewayErrorKind::Network, safe_error)
                };
                if attempt + 1 >= MAX_ATTEMPTS {
                    return Err(mapped);
                }
                last_error = Some(mapped);
                sleep_before_retry(attempt, None).await;
                continue;
            }
        };
        let status = response.status();
        if !status.is_success() {
            let retry_after = retry_after_seconds(response.headers());
            let error_body = response.text().await.unwrap_or_default();
            let safe_error_body = credential
                .as_ref()
                .map_or_else(|| error_body.clone(), |value| value.redact(&error_body));
            let error = LlmGatewayError::http(status.as_u16(), safe_error_body);
            let retryable = status.as_u16() == 429 || status.is_server_error();
            // 余额/总额度耗尽不是瞬时限流，等待只会拖慢并重复失败。
            if !retryable || is_quota_exhausted(&error_body) || attempt + 1 >= MAX_ATTEMPTS {
                return Err(error);
            }
            last_error = Some(error);
            sleep_before_retry(attempt, retry_after).await;
            continue;
        }

        let value = response.json::<Value>().await.map_err(|e| {
            LlmGatewayError::new(LlmGatewayErrorKind::ResponseFormat, e.to_string())
        })?;
        let mut output = parse_non_stream_chat_response(value).map_err(|mut error| {
            if let Some(credential) = credential.as_ref() {
                error.message = credential.redact(&error.message);
            }
            error
        })?;
        if let Some(credential) = credential.as_ref() {
            output.content = credential.redact(&output.content);
            output.reasoning_content = output
                .reasoning_content
                .as_deref()
                .map(|value| credential.redact(value));
            output.finish_reason = output
                .finish_reason
                .as_deref()
                .map(|value| credential.redact(value));
            output.model = output
                .model
                .as_deref()
                .map(|value| credential.redact(value));
        }
        return Ok(output);
    }
    Err(last_error.unwrap_or_else(|| {
        LlmGatewayError::new(
            LlmGatewayErrorKind::ProviderUnavailable,
            "LLM 重试次数已用尽",
        )
    }))
}

fn retry_after_seconds(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|seconds| seconds.min(30))
}

fn is_quota_exhausted(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    [
        "insufficient balance",
        "balance insufficient",
        "insufficient quota",
        "quota exceeded",
        "usage limit",
        "credit balance",
        "余额不足",
        "额度不足",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

async fn sleep_before_retry(attempt: u32, retry_after_secs: Option<u64>) {
    let delay = retry_after_secs
        .map(std::time::Duration::from_secs)
        .unwrap_or_else(|| {
            std::time::Duration::from_millis((500u64.saturating_mul(1 << attempt)).min(4_000))
        });
    crate::dlog!(
        "[llm gateway] 瞬时失败，第 {}/3 次请求后等待 {}ms 重试",
        attempt + 1,
        delay.as_millis()
    );
    tokio::time::sleep(delay).await;
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
