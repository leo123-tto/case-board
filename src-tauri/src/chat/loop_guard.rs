//! agent_loop 的 5 条 cap(V0.2 D3-D4.B,详 § 6.5)。
//!
//! 在 chat agent 多轮工具调用循环里,防止"无限调"、"反复调同一个工具"、"调用堆积太久"、
//! "长时间无进展卡住"、"thinking 模型 reasoning token 爆炸"五种失控情况。
//!
//! 每轮 LLM 请求前调 `check_iter_cap` + `check_duration_cap`;每次准备发起 tool 调用前调
//! `check_duplicate_tool_call`;流式 token/reasoning/tool 结果到来时调 `note_progress`;
//! LLM 返回 usage 后调 `add_reasoning_tokens`。
//!
//! 任何一个 cap 触发 → 返回 `LoopGuardViolation`,agent_loop 终止本轮并把信息塞回 LLM 让它
//! 收尾(或者直接 abort,看上层策略)。
//!
//! **2026-07-14 idle 缩放**:`idle_timeout` 已不再固定 180s,改为跟随任务
//! `max_duration` 自动缩放(max_duration / 3,下限 180s,上限 max_duration 自身)。
//! 之前固定 180s 在思考模型(MiniMax M3 / deepseek-v4-pro)上会被误判「读流卡死」:
//! 思考 + 生成大 CaseGraph JSON 阶段常连续 5-10 分钟不吐 token,reqwest.read_timeout
//! 在 180s 时强行断流,返回 `error decoding response body` 把好流截掉。VisualizeCase
//! 的 max_duration 是 1800s,缩放后 idle = 600s(10 分钟)够用且不会让真卡死的请求
//! 拖太久。

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde::Serialize;
use thiserror::Error;

use super::context::TaskType;

pub const DEFAULT_CHAT_LOOP_TIMEOUT_DEFAULT_SECS: u64 = 300;
pub const DEFAULT_CHAT_LOOP_TIMEOUT_COMPLEX_SECS: u64 = 1_800;
pub const DEFAULT_CHAT_LOOP_TIMEOUT_DEEP_ANALYSIS_SECS: u64 = 2_700;
pub const DEFAULT_CHAT_LOOP_IDLE_TIMEOUT_SECS: u64 = 180;
pub const DEFAULT_CHAT_LOOP_MAX_ITERS_DEFAULT: u32 = 16;
pub const DEFAULT_CHAT_LOOP_MAX_ITERS_COMPLEX: u32 = 48;
pub const DEFAULT_CHAT_LOOP_MAX_ITERS_DEEP_ANALYSIS: u32 = 64;
pub const DEFAULT_REASONING_TOKENS_DEFAULT: u64 = 64_000;
pub const DEFAULT_REASONING_TOKENS_COMPLEX: u64 = 192_000;
pub const DEFAULT_REASONING_TOKENS_DEEP_ANALYSIS: u64 = 256_000;
const MIN_CHAT_LOOP_TIMEOUT_SECS: u64 = 60;

/// 5 条 cap 中触发哪一条。
#[derive(Debug, Clone, Serialize, Error)]
pub enum LoopGuardViolation {
    #[error("超过本会话最大轮数(max={max})")]
    IterCapExceeded { max: u32 },
    #[error("LLM 反复调同一工具 + 同参数:tool={tool},循环模式拦下")]
    DuplicateToolCall { tool: String },
    #[error("本会话总耗时超 {limit_secs}s,可能后端慢或卡死,提前 abort")]
    DurationCapExceeded { limit_secs: u64 },
    #[error("连续 {idle_secs}s 没有新进展(token / reasoning / 工具结果),疑似卡住,提前 abort")]
    IdleCapExceeded { idle_secs: u64 },
    #[error("reasoning token 累计超 {limit},thinking 模型可能跑飞")]
    ReasoningTokenCapExceeded { limit: u64 },
}

#[derive(Debug, Clone, Copy)]
pub struct LoopGuardConfig {
    pub max_iters: u32,
    pub max_duration: Duration,
    pub idle_timeout: Duration,
    pub max_reasoning_tokens: u64,
}

impl LoopGuardConfig {
    pub fn from_settings_for_task(s: &crate::settings::Settings, task: TaskType) -> Self {
        let (task_default_iters, max_duration_secs, max_reasoning_tokens) = match task {
            TaskType::DeepAnalysis | TaskType::CriminalDeepAnalysis => (
                DEFAULT_CHAT_LOOP_MAX_ITERS_DEEP_ANALYSIS,
                DEFAULT_CHAT_LOOP_TIMEOUT_DEEP_ANALYSIS_SECS,
                DEFAULT_REASONING_TOKENS_DEEP_ANALYSIS,
            ),
            TaskType::CompileLegalBasis
            | TaskType::FindSimilarCases
            | TaskType::VerifyMyDraft
            | TaskType::SimulateOpposition
            | TaskType::VisualizeCase => (
                DEFAULT_CHAT_LOOP_MAX_ITERS_COMPLEX,
                DEFAULT_CHAT_LOOP_TIMEOUT_COMPLEX_SECS,
                DEFAULT_REASONING_TOKENS_COMPLEX,
            ),
            TaskType::FreeChat => (
                DEFAULT_CHAT_LOOP_MAX_ITERS_DEFAULT,
                DEFAULT_CHAT_LOOP_TIMEOUT_DEFAULT_SECS,
                DEFAULT_REASONING_TOKENS_DEFAULT,
            ),
        };
        let max_duration_secs = max_duration_secs.max(MIN_CHAT_LOOP_TIMEOUT_SECS);
        // 16 是旧版写进 settings.json 的显示默认值，不代表用户主动限制复杂任务。
        // 其他值视为明确自定义并继续尊重。
        let configured_iters = s
            .chat_loop_max_iters
            .unwrap_or(DEFAULT_CHAT_LOOP_MAX_ITERS_DEFAULT);
        let max_iters = if configured_iters == DEFAULT_CHAT_LOOP_MAX_ITERS_DEFAULT {
            task_default_iters
        } else {
            configured_iters
        };
        let idle_secs = max_duration_secs
            .saturating_div(3)
            .max(DEFAULT_CHAT_LOOP_IDLE_TIMEOUT_SECS)
            .min(max_duration_secs);
        Self {
            max_iters,
            max_duration: Duration::from_secs(max_duration_secs),
            idle_timeout: Duration::from_secs(idle_secs),
            max_reasoning_tokens,
        }
    }
}

pub struct LoopGuard {
    iter_count: u32,
    max_iters: u32,
    seen_tool_args: HashSet<(String, String)>,
    started_at: Instant,
    max_duration: Duration,
    last_progress_at: Instant,
    idle_timeout: Duration,
    reasoning_tokens: u64,
    max_reasoning_tokens: u64,
}

impl LoopGuard {
    pub fn from_config(cfg: LoopGuardConfig) -> Self {
        Self {
            iter_count: 0,
            max_iters: cfg.max_iters,
            seen_tool_args: HashSet::new(),
            started_at: Instant::now(),
            max_duration: cfg.max_duration,
            last_progress_at: Instant::now(),
            idle_timeout: cfg.idle_timeout,
            reasoning_tokens: 0,
            max_reasoning_tokens: cfg.max_reasoning_tokens,
        }
    }

    /// 用 settings + 任务类型配置 5 条 cap。settings 字段为 None 时用默认值。
    pub fn from_settings_for_task(s: &crate::settings::Settings, task: TaskType) -> Self {
        Self::from_config(LoopGuardConfig::from_settings_for_task(s, task))
    }

    pub fn iter_count(&self) -> u32 {
        self.iter_count
    }

    pub fn idle_timeout_secs(&self) -> u64 {
        self.idle_timeout.as_secs()
    }

    /// 收到新的 token / reasoning / 工具结果时更新最近进展时间。
    pub fn note_progress(&mut self) {
        self.last_progress_at = Instant::now();
    }

    /// 进入新一轮(发请求前调)。失败返回 `IterCapExceeded`。
    pub fn check_iter_cap(&mut self) -> Result<(), LoopGuardViolation> {
        if self.iter_count >= self.max_iters {
            return Err(LoopGuardViolation::IterCapExceeded {
                max: self.max_iters,
            });
        }
        self.iter_count += 1;
        Ok(())
    }

    /// 派发工具前调:同一 tool + 同参数 hash 之前调过就拒绝(防 LLM 死循环)。
    /// `args` 用 canonical JSON(local_kb::hash::query_hash 不带 prefix)做 dedupe key。
    pub fn check_duplicate_tool_call(
        &mut self,
        tool: &str,
        args: &serde_json::Value,
    ) -> Result<(), LoopGuardViolation> {
        // 用同种 canonical 算法跟 KB cache 对齐,sort_keys + ensure_ascii=False
        let canonical = crate::local_kb::hash::query_hash("", args);
        let key = (tool.to_string(), canonical);
        if !self.seen_tool_args.insert(key) {
            return Err(LoopGuardViolation::DuplicateToolCall {
                tool: tool.to_string(),
            });
        }
        Ok(())
    }

    /// 检查整轮会话的总墙钟时长是否超出当前任务的内置上限。
    pub fn check_duration_cap(&self) -> Result<(), LoopGuardViolation> {
        if self.started_at.elapsed() > self.max_duration {
            return Err(LoopGuardViolation::DurationCapExceeded {
                limit_secs: self.max_duration.as_secs(),
            });
        }
        Ok(())
    }

    /// 连续太久没有任何新进展(token / reasoning / 工具结果)就判定为卡住。
    pub fn check_idle_cap(&self) -> Result<(), LoopGuardViolation> {
        if self.last_progress_at.elapsed() > self.idle_timeout {
            return Err(LoopGuardViolation::IdleCapExceeded {
                idle_secs: self.idle_timeout.as_secs(),
            });
        }
        Ok(())
    }

    /// LLM 返回 usage 时累计 reasoning_tokens(thinking 模型 usage.reasoning_tokens)。
    pub fn add_reasoning_tokens(&mut self, n: u64) -> Result<(), LoopGuardViolation> {
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(n);
        if self.reasoning_tokens > self.max_reasoning_tokens {
            return Err(LoopGuardViolation::ReasoningTokenCapExceeded {
                limit: self.max_reasoning_tokens,
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    /// 2026-07-14:idle 缩放回归测试。固定 180s 在思考模型上被 5-10 分钟思考 + 大 JSON
    /// 生成阶段吃光,reqwest.read_timeout 误判读流卡死(error decoding response body)。
    /// 新公式 `max(max_duration/3, 180)`,min(max_duration) 让 idle 跟任务时长缩放。
    #[test]
    fn idle_scales_with_max_duration() {
        // 复杂任务(max_duration=1800)→ max(1800/3=600, 180)=600 → min(600, 1800)=600
        let s = Settings::default();
        let guard = LoopGuard::from_settings_for_task(&s, TaskType::VisualizeCase);
        assert_eq!(
            guard.idle_timeout_secs(),
            600,
            "VisualizeCase max_duration=1800s 缩放后 idle 应该是 600s;实际 {}s",
            guard.idle_timeout_secs()
        );
    }

    #[test]
    fn freechat_idle_keeps_old_floor() {
        // FreeChat max_duration=300 → 300/3=100 → max(100, 180)=180 → min(180, 300)=180
        // 不应缩短老用户已习惯的 180s idle
        let s = Settings::default();
        let guard = LoopGuard::from_settings_for_task(&s, TaskType::FreeChat);
        assert_eq!(guard.idle_timeout_secs(), 180);
    }

    #[test]
    fn deep_analysis_idle_gets_max_window() {
        // DeepAnalysis max_duration=2700 → 2700/3=900 → max(900, 180)=900 → min(900, 2700)=900
        let s = Settings::default();
        let guard = LoopGuard::from_settings_for_task(&s, TaskType::DeepAnalysis);
        assert_eq!(guard.idle_timeout_secs(), 900);
    }
}
