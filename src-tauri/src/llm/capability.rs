//! LLM provider capability matrix.
//!
//! 目的不是替代模型路由,而是把 DeepSeek / MiniMax / OpenAI-compatible
//! 在 wire contract 上的差异集中到一处,避免每个调用点各写一套 if/else。

use serde::{Deserialize, Serialize};

use super::LlmConfig;
use crate::settings::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LlmProviderKind {
    DeepSeek,
    MiniMaxNative,
    GlmCompat,
    MimoCompat,
    KimiCompat,
    CustomCompat,
    LocalOpenAiCompat,
    UnknownOpenAiCompat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputTokenParam {
    MaxTokens,
    MaxCompletionTokens,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapability {
    pub kind: LlmProviderKind,
    pub supports_tools: bool,
    pub supports_strict_tools: bool,
    pub requires_reasoning_replay_for_tool_calls: bool,
    pub supports_prompt_cache_usage: bool,
    pub supports_json_object_response_format: bool,
    pub supports_stream_usage: bool,
    pub supports_tool_choice_required: bool,
    pub output_token_param: OutputTokenParam,
}

impl ProviderCapability {
    pub fn from_settings(settings: &Settings, config: &LlmConfig) -> Self {
        Self::from_backend(
            settings.effective_cloud_llm_backend(),
            &config.endpoint,
            &config.model,
        )
    }

    pub fn from_backend(backend: &str, endpoint: &str, model: &str) -> Self {
        let kind = infer_provider_kind(backend, endpoint, model);
        Self::for_kind(kind)
    }

    pub fn for_kind(kind: LlmProviderKind) -> Self {
        match kind {
            LlmProviderKind::DeepSeek => Self {
                kind,
                supports_tools: true,
                supports_strict_tools: true,
                requires_reasoning_replay_for_tool_calls: true,
                supports_prompt_cache_usage: true,
                supports_json_object_response_format: true,
                supports_stream_usage: true,
                // DeepSeek V4 thinking mode rejects "required"; use auto.
                supports_tool_choice_required: false,
                output_token_param: OutputTokenParam::MaxTokens,
            },
            LlmProviderKind::MiniMaxNative => Self {
                kind,
                supports_tools: true,
                supports_strict_tools: false,
                requires_reasoning_replay_for_tool_calls: true,
                supports_prompt_cache_usage: true,
                supports_json_object_response_format: false,
                supports_stream_usage: true,
                supports_tool_choice_required: false,
                output_token_param: OutputTokenParam::MaxCompletionTokens,
            },
            LlmProviderKind::GlmCompat | LlmProviderKind::MimoCompat => Self {
                kind,
                supports_tools: true,
                supports_strict_tools: false,
                requires_reasoning_replay_for_tool_calls: false,
                supports_prompt_cache_usage: false,
                supports_json_object_response_format: true,
                supports_stream_usage: false,
                supports_tool_choice_required: false,
                output_token_param: OutputTokenParam::MaxTokens,
            },
            LlmProviderKind::KimiCompat => Self {
                kind,
                supports_tools: true,
                supports_strict_tools: false,
                requires_reasoning_replay_for_tool_calls: true,
                supports_prompt_cache_usage: false,
                supports_json_object_response_format: true,
                supports_stream_usage: false,
                supports_tool_choice_required: false,
                output_token_param: OutputTokenParam::MaxTokens,
            },
            LlmProviderKind::CustomCompat | LlmProviderKind::UnknownOpenAiCompat => Self {
                kind,
                supports_tools: false,
                supports_strict_tools: false,
                requires_reasoning_replay_for_tool_calls: false,
                supports_prompt_cache_usage: false,
                supports_json_object_response_format: false,
                supports_stream_usage: false,
                supports_tool_choice_required: false,
                output_token_param: OutputTokenParam::MaxTokens,
            },
            LlmProviderKind::LocalOpenAiCompat => Self {
                kind,
                supports_tools: false,
                supports_strict_tools: false,
                requires_reasoning_replay_for_tool_calls: false,
                supports_prompt_cache_usage: false,
                supports_json_object_response_format: false,
                supports_stream_usage: false,
                supports_tool_choice_required: false,
                output_token_param: OutputTokenParam::MaxTokens,
            },
        }
    }

    /// Kimi Coding Plan 只接受 temperature=1；其他 provider 保留调用方意图，但统一在
    /// 协议边界收敛到两位小数。不要直接把 `f32` 塞进 `serde_json::Value`：例如 0.3 会
    /// 被扩成 0.30000001192092896，MiniMax 会以 1210 拒绝该参数。
    pub fn normalize_temperature(&self, requested: f32) -> f64 {
        let normalized = if self.kind == LlmProviderKind::KimiCompat {
            1.0
        } else {
            requested as f64
        };
        (normalized * 100.0).round() / 100.0
    }
}

pub fn infer_provider_kind(backend: &str, endpoint: &str, model: &str) -> LlmProviderKind {
    let backend = backend.trim().to_ascii_lowercase();
    let endpoint_l = endpoint.to_ascii_lowercase();
    let model_l = model.to_ascii_lowercase();

    match backend.as_str() {
        "deepseek" => return LlmProviderKind::DeepSeek,
        "minimax" => return LlmProviderKind::MiniMaxNative,
        "glm" => return LlmProviderKind::GlmCompat,
        "mimo" => return LlmProviderKind::MimoCompat,
        "kimi" => return LlmProviderKind::KimiCompat,
        "custom" => return LlmProviderKind::CustomCompat,
        _ => {}
    }

    if endpoint_l.contains("deepseek") || model_l.contains("deepseek") {
        LlmProviderKind::DeepSeek
    } else if endpoint_l.contains("chatcompletion_v2") || endpoint_l.contains("minimax") {
        LlmProviderKind::MiniMaxNative
    } else if endpoint_l.contains("bigmodel.cn") || model_l.contains("glm") {
        LlmProviderKind::GlmCompat
    } else if endpoint_l.contains("xiaomimimo") || model_l.contains("mimo") {
        LlmProviderKind::MimoCompat
    } else if endpoint_l.contains("api.kimi.com/coding") || model_l.contains("kimi-for-coding") {
        LlmProviderKind::KimiCompat
    } else if endpoint_l.contains("127.0.0.1") || endpoint_l.contains("localhost") {
        LlmProviderKind::LocalOpenAiCompat
    } else {
        LlmProviderKind::UnknownOpenAiCompat
    }
}
