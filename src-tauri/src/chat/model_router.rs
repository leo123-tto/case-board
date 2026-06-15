//! V0.2 D4-D5.B · 模型路由(V0.3 重构:统一到 `settings.cloud_llm_model` 单一字段)。
//!
//! 把 `(TaskType, user_message, Settings)` 映射到具体 DeepSeek 模型 + 温度 + max_tokens。
//!
//! **用户在设置里只有一个选择 `cloud_llm_model`(= 三档「模型档位」)**:
//!   - `"deepseek-v4-flash"`(默认)= **全局 Flash**:所有任务都走 flash(便宜,约 pro 的 1/3 价)。
//!   - `"deepseek-v4-pro"` = **全局 Pro**:所有任务都走 pro(更准更贵;实测 v4-pro 本身即思考模型,无独立 -thinking 变体)。
//!   - `"auto"` = **自动挡**:简单任务走 flash、复杂任务走 pro(下面的 task 路由表)。
//!
//! 关键:**非 auto 档绝不"偷偷"把某些任务升到 pro**(老逻辑工具型 chip 强制 pro 烧钱,已废)。
//! 自动挡(auto)下才按任务复杂度分流(V0.3.3 起 6 个生成型 chip 已删):
//!   - 4 个工具/分析型(法律依据/类案/校验/模拟对抗) → pro
//!   - FreeChat → 启发式:短问/无 reasoning 关键词 = flash,否则 pro

use serde::Serialize;

use super::context::TaskType;
use crate::settings::Settings;

/// 路由结果。给 agent_loop / commands 用,代替原来硬编码的 temperature / max_tokens。
#[derive(Debug, Clone, Serialize)]
pub struct ModelChoice {
    pub model: String,
    pub temperature: f32,
    pub max_tokens: u32,
}

/// DeepSeek V4 输出长度上限(官方文档:context 1M / output 最大 384K)。
/// flash / pro 用同一上限——旧的 4096/8192 低值会把长文书拦腰截断(`finish_reason=length`,
/// 体感像「写一半就傻了」)。这是「天花板」不是「目标」:只在模型真写那么长时才计费,
/// 短问答仍会自然停(`finish_reason=stop`)。模型档位(flash/pro)由作者在 Settings 手切,本值不区分。
pub const MAX_OUTPUT_TOKENS: u32 = 384_000;

/// MiniMax M 系列输出上限保守值(M2.7≈8K / M3≈32K,且思考占 output)。
/// 不用 DeepSeek 的 384K —— 远超 MiniMax 模型上限,可能触发 2013 参数错。
pub const MINIMAX_MAX_OUTPUT_TOKENS: u32 = 32_768;

impl ModelChoice {
    /// DeepSeek V4 Flash:快速 + 便宜,适合摘要/列表/简单问答。
    pub fn flash() -> Self {
        Self {
            model: "deepseek-v4-flash".into(),
            temperature: 0.3,
            max_tokens: MAX_OUTPUT_TOKENS,
        }
    }

    /// DeepSeek V4 Pro:推理 + 工具调用,适合法律论证/工具任务。
    ///
    /// **实测 2026-06-13**:`deepseek-v4-pro` 本身即思考模型 —— 不带任何 thinking 参数也默认返回
    /// `reasoning_content`;且思考 + strict 工具调用同时正常(工具轮回传 reasoning_content 已在
    /// agent_loop 处理)。DeepSeek V4 **全系(flash/pro)都是思考模式,没有独立的 `-thinking` 变体**
    /// (`/models` 只列 flash/pro;调 `deepseek-v4-pro-thinking` 直接 400)。故 `with_reasoning` 入参
    /// 已无实际意义,保留仅为兼容签名,一律返回 `deepseek-v4-pro`。
    pub fn pro(_with_reasoning: bool) -> Self {
        Self {
            model: "deepseek-v4-pro".into(),
            temperature: 0.15,
            max_tokens: MAX_OUTPUT_TOKENS,
        }
    }

    /// 把用户在 Settings 强制选定的 model 字符串包装成 ModelChoice。
    /// 不识别的 model 名透传(让 DeepSeek 自己报 400)。
    pub fn from_forced(model: &str) -> Self {
        let is_pro = model.contains("pro");
        Self {
            model: model.to_string(),
            temperature: if is_pro { 0.15 } else { 0.3 },
            max_tokens: MAX_OUTPUT_TOKENS,
        }
    }

    /// MiniMax M 系列(2026-06-15)。无 flash/pro 档位之分,用户在设置里直接填模型名。
    /// 温度:M3 官方建议 1.0,其余 M 系列 0.3(M 系列禁 0.0);max_tokens 用 MiniMax 保守上限。
    pub fn from_minimax(model: &str) -> Self {
        let temperature = if model.to_ascii_lowercase().contains("m3") {
            1.0
        } else {
            0.3
        };
        Self {
            model: model.to_string(),
            temperature,
            max_tokens: MINIMAX_MAX_OUTPUT_TOKENS,
        }
    }
}

/// 路由主入口。统一读 `settings.cloud_llm_model` 这一个「模型档位」字段。
pub fn route_model(task: TaskType, user_message: &str, settings: &Settings) -> ModelChoice {
    // 2026-06-15:MiniMax 后端不套用 DeepSeek 的 flash/pro/auto 档位 —— 用户直接填模型名。
    if settings.effective_cloud_llm_backend() == "minimax" {
        let provider_minimax = matches!(
            settings.cloud_llm_provider.as_deref().map(str::trim),
            Some("minimax")
        );
        let model = settings
            .cloud_llm_model
            .as_deref()
            .filter(|_| provider_minimax)
            .or(settings.minimax_model.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "auto")
            .unwrap_or("MiniMax-M2");
        return ModelChoice::from_minimax(model);
    }

    // 档位:默认 flash(便宜)。空字符串也当默认。
    let mode = settings
        .cloud_llm_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("deepseek-v4-flash");

    // 全局档(非 auto):所有任务都用这个模型,不再按任务强制 pro。
    if mode != "auto" {
        return ModelChoice::from_forced(mode);
    }

    // 自动挡(auto):按任务复杂度分流。
    match task {
        // 工具/分析型 + 深度分析 → pro。deepseek-v4-pro 本身即思考模型(实测默认开推理),
        // 思考 + strict 工具调用实测正常,深度分析走 pro 就已是「推理 + 工具」模式。
        TaskType::CompileLegalBasis
        | TaskType::FindSimilarCases
        | TaskType::VerifyMyDraft
        | TaskType::SimulateOpposition
        | TaskType::DeepAnalysis => ModelChoice::pro(false),
        // 自由问 → 启发式
        TaskType::FreeChat => route_free_chat(user_message),
    }
}

/// 启发式:短问(<30 字)或不带"推理类"关键词 → flash;否则 pro。
fn route_free_chat(msg: &str) -> ModelChoice {
    let chars = msg.chars().count();
    if chars < 30 {
        return ModelChoice::flash();
    }
    const REASONING_KEYWORDS: &[&str] = &[
        "建议",
        "分析",
        "为什么",
        "怎么办",
        "如何",
        "拒执",
        "风险",
        "怎么处理",
        "策略",
        "对比",
        "评估",
        "推理",
    ];
    if REASONING_KEYWORDS.iter().any(|k| msg.contains(k)) {
        ModelChoice::pro(false)
    } else {
        ModelChoice::flash()
    }
}
