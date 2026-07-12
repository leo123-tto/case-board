//! V0.2 D5.C · 4 个 ChatHook(详 § 6.4)。
//!
//! 设计:
//!   - `ChatHook` trait 4 个生命周期方法,默认 no-op,实现按需 override
//!   - `HookChain` 是组合多个 Hook 的便捷封装,agent_loop 一次调,内部 fan-out
//!   - `before_tool_call` 返回 `HookOutcome::Deny(msg)` 时,agent_loop 不派发该工具,
//!     而是直接把 deny msg 作为该 tool_call 的 result 塞回 messages,**LLM 仍能看到失败原因**继续答
//!
//! 4 个 hook 的职责分工:
//!   - **CreditsGuardHook**:熔断 + 记账(D5 acceptance 必过的核心)
//!   - **KbHitRatioHook**:统计本会话 KB 命中率(给反馈 MD 看)
//!   - **CostEstimateHook**:LLM token 累计估元(展示给用户「本次约 ¥X」)
//!   - **CacheStatsHook**:DeepSeek prompt cache 命中率(优化稳态段排序的依据)
//!
//! HookContext 字段:pool / settings / case_id / task_id / year_month / 共享统计 RwLock。

use std::sync::Arc;
use std::sync::RwLock;

use async_trait::async_trait;
use serde::Serialize;
use sqlx::SqlitePool;

use super::stream::ChatUsage;
use super::tools::ToolResult;
use crate::db::credits;
use crate::settings::Settings;

/// before_tool_call / before_llm_call 的回执。
#[derive(Debug, Clone)]
pub enum HookOutcome {
    /// 允许继续
    Continue,
    /// 拒绝,带上给 LLM(和用户)看的中文原因
    Deny(String),
}

/// 跑 hook 时所需的运行时上下文。借用 pool / settings;**task_id 可选**(不进 chat_tasks 表
/// 的简单调用传 None)。
pub struct HookContext<'a> {
    pub pool: &'a SqlitePool,
    pub settings: &'a Settings,
    pub case_id: Option<&'a str>,
    pub task_id: Option<&'a str>,
    /// `YYYY-MM`,credits monthly 表 PK
    pub year_month: String,
    /// 共享会话统计(KbHitRatio / CostEstimate / CacheStats 累加进去)
    pub session: Arc<RwLock<SessionStats>>,
}

impl<'a> HookContext<'a> {
    pub fn new(
        pool: &'a SqlitePool,
        settings: &'a Settings,
        case_id: Option<&'a str>,
        task_id: Option<&'a str>,
        session: Arc<RwLock<SessionStats>>,
    ) -> Self {
        Self {
            pool,
            settings,
            case_id,
            task_id,
            year_month: credits::current_year_month(),
            session,
        }
    }
}

/// 单个会话累计的运行时统计(给反馈 MD + 前端展示用)。
#[derive(Debug, Clone, Default, Serialize)]
pub struct SessionStats {
    /// 工具调用总数
    pub tool_calls: u32,
    /// 其中 KB 命中数
    pub kb_hits: u32,
    /// 在线元典调用数(KB miss + API 成功)
    pub yuandian_calls: u32,
    /// 本会话累计元典积分
    pub yuandian_credits_used: u32,
    /// LLM prompt_tokens 累计
    pub prompt_tokens: u64,
    /// LLM completion_tokens 累计
    pub completion_tokens: u64,
    /// DeepSeek prompt cache hit tokens 累计
    pub cache_hit_tokens: u64,
    /// 估算本次会话总成本(元)
    pub est_cost_yuan: f64,
}

impl SessionStats {
    /// KB 命中率(0.0 ~ 1.0)。tool_calls==0 时 0.0。
    pub fn kb_hit_ratio(&self) -> f64 {
        if self.tool_calls == 0 {
            0.0
        } else {
            self.kb_hits as f64 / self.tool_calls as f64
        }
    }

    /// DeepSeek prompt cache 命中率(0.0 ~ 1.0)。目标 ≥ 0.7(§ 4.2)。
    pub fn prompt_cache_ratio(&self) -> f64 {
        if self.prompt_tokens == 0 {
            0.0
        } else {
            self.cache_hit_tokens as f64 / self.prompt_tokens as f64
        }
    }
}

/// hook trait — agent_loop / commands 持有 `Vec<Box<dyn ChatHook>>`,生命周期 4 个回调。
#[async_trait]
pub trait ChatHook: Send + Sync {
    fn name(&self) -> &'static str;

    /// 工具派发前。返回 Deny 会让 agent_loop 跳过本次调用,把 deny msg 当结果回填。
    async fn before_tool_call(
        &self,
        _tool: &str,
        _args: &serde_json::Value,
        _ctx: &HookContext<'_>,
    ) -> HookOutcome {
        HookOutcome::Continue
    }

    /// 工具调用后(成功或失败都调)。
    async fn after_tool_call(
        &self,
        _tool: &str,
        _result: &ToolResult,
        _success: bool,
        _ctx: &HookContext<'_>,
    ) {
    }

    /// LLM 调用后 / 流式结束。
    async fn after_llm_call(&self, _usage: &ChatUsage, _ctx: &HookContext<'_>) {}
}

/// 多个 Hook 的执行链。fan-out 调每个 hook,第一个 Deny 就短路。
pub struct HookChain {
    pub hooks: Vec<Box<dyn ChatHook>>,
}

impl HookChain {
    pub fn empty() -> Self {
        Self { hooks: Vec::new() }
    }

    /// V0.2 默认链:4 个 hook 全开。
    pub fn default_v0_2() -> Self {
        Self {
            hooks: vec![
                Box::new(CreditsGuardHook),
                Box::new(KbHitRatioHook),
                Box::new(CostEstimateHook),
                Box::new(CacheStatsHook),
            ],
        }
    }

    pub async fn run_before_tool_call(
        &self,
        tool: &str,
        args: &serde_json::Value,
        ctx: &HookContext<'_>,
    ) -> HookOutcome {
        for h in &self.hooks {
            match h.before_tool_call(tool, args, ctx).await {
                HookOutcome::Continue => {}
                deny => return deny,
            }
        }
        HookOutcome::Continue
    }

    pub async fn run_after_tool_call(
        &self,
        tool: &str,
        result: &ToolResult,
        success: bool,
        ctx: &HookContext<'_>,
    ) {
        for h in &self.hooks {
            h.after_tool_call(tool, result, success, ctx).await;
        }
    }

    pub async fn run_after_llm_call(&self, usage: &ChatUsage, ctx: &HookContext<'_>) {
        for h in &self.hooks {
            h.after_llm_call(usage, ctx).await;
        }
    }
}

// ============================================================================
// 4 个 hook 实现
// ============================================================================

/// **熔断 + 记账**:在线元典调用前查月度配额,超限拒;调用后写月度账单。
pub struct CreditsGuardHook;

#[async_trait]
impl ChatHook for CreditsGuardHook {
    fn name(&self) -> &'static str {
        "credits_guard"
    }

    async fn before_tool_call(
        &self,
        tool: &str,
        _args: &serde_json::Value,
        ctx: &HookContext<'_>,
    ) -> HookOutcome {
        // 只对**会消耗元典积分的工具**做熔断 — 本地工具(case_doc/kb)0 积分不查
        let expected = credits::estimate_credits_for(tool);
        if expected == 0 {
            return HookOutcome::Continue;
        }
        // get_law_article 内部先走主库整部法规抽条。必须允许它执行到本地命中分支；
        // 真正准备联网前由工具按实际 5/1 分再次检查额度，避免“额度为 0 时连本地也不让读”。
        if tool == "get_law_article" {
            return HookOutcome::Continue;
        }
        let Some(limit) = ctx.settings.yuandian_monthly_credit_limit else {
            return HookOutcome::Continue; // 用户没设上限 = 不限制
        };
        let remaining =
            credits::get_monthly_remaining(ctx.pool, &ctx.year_month, Some(limit)).await;
        if remaining < expected as i64 {
            return HookOutcome::Deny(format!(
                "本月元典积分余额不足以执行本工具(剩余 {},预计最多 {} 分,月度上限 {})。请先用本地 KB 工具(search_local_kb / read_kb_file),或到设置调整上限。",
                remaining.max(0), expected, limit
            ));
        }
        HookOutcome::Continue
    }

    async fn after_tool_call(
        &self,
        _tool: &str,
        result: &ToolResult,
        success: bool,
        ctx: &HookContext<'_>,
    ) {
        // 失败的工具调用不记账(避免坑 #12 误算)
        if !success {
            return;
        }
        if result.kb_hit {
            // 本地命中,只记 kb_hits
            let _ = credits::record_kb_hit(ctx.pool, &ctx.year_month).await;
            return;
        }
        let actual = result.yuandian_credits_used;
        if actual == 0 {
            return; // 本地工具,不进月度账
        }
        let _ = credits::record_yuandian_call(ctx.pool, &ctx.year_month, actual).await;
    }
}

/// **统计 KB 命中率** — 累加 session.tool_calls / kb_hits。
pub struct KbHitRatioHook;

#[async_trait]
impl ChatHook for KbHitRatioHook {
    fn name(&self) -> &'static str {
        "kb_hit_ratio"
    }

    async fn after_tool_call(
        &self,
        _tool: &str,
        result: &ToolResult,
        success: bool,
        ctx: &HookContext<'_>,
    ) {
        if !success {
            return;
        }
        if let Ok(mut s) = ctx.session.write() {
            s.tool_calls = s.tool_calls.saturating_add(1);
            if result.kb_hit {
                s.kb_hits = s.kb_hits.saturating_add(1);
            }
            // yuandian_calls / credits_used 也累加
            if !result.kb_hit && result.yuandian_credits_used > 0 {
                s.yuandian_calls = s.yuandian_calls.saturating_add(1);
                s.yuandian_credits_used = s
                    .yuandian_credits_used
                    .saturating_add(result.yuandian_credits_used);
            }
        }
    }
}

/// **估算成本** — LLM 累计 token × 单价,放进 session.est_cost_yuan。
pub struct CostEstimateHook;

#[async_trait]
impl ChatHook for CostEstimateHook {
    fn name(&self) -> &'static str {
        "cost_estimate"
    }

    async fn after_llm_call(&self, usage: &ChatUsage, ctx: &HookContext<'_>) {
        let pt = usage.prompt_tokens.unwrap_or(0);
        let ct = usage.completion_tokens.unwrap_or(0);
        // DeepSeek V4 大致单价(2026-05,**仅估算**,实际以官方为准):
        //   - input 1M tokens ≈ ¥1.0(cache miss);cache hit ¥0.1
        //   - output 1M tokens ≈ ¥4.0
        // 不区分 cache hit/miss 简化:input = ¥1 / 1M token,output = ¥4 / 1M token
        let cost = (pt as f64) * 1.0e-6 + (ct as f64) * 4.0e-6;
        if let Ok(mut s) = ctx.session.write() {
            s.prompt_tokens = s.prompt_tokens.saturating_add(pt);
            s.completion_tokens = s.completion_tokens.saturating_add(ct);
            s.est_cost_yuan += cost;
        }
    }
}

/// **prompt cache 命中** — 累加 cache_hit_tokens,反馈 MD 展示缓存命中率(目标 ≥70%)。
pub struct CacheStatsHook;

#[async_trait]
impl ChatHook for CacheStatsHook {
    fn name(&self) -> &'static str {
        "cache_stats"
    }

    async fn after_llm_call(&self, _usage: &ChatUsage, _ctx: &HookContext<'_>) {
        // ChatUsage struct 目前没暴露 cache_hit_tokens(在 agent_loop ChunkUsage 内部);
        // V0.2 D5 先 stub — D5 后续把字段穿到 ChatUsage 时把代码补上(在
        // `chat::stream::ChatUsage` 加 `cache_hit_tokens: Option<u64>`,这里累加进 session)。
        // 暂时空 impl,本会话 prompt_cache_ratio() 会返回 0.0,
        // 反馈 MD 会显示「prompt cache 命中率:暂未上报」。
    }
}
