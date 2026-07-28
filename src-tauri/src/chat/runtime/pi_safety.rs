//! Pi Runtime 的宿主级保险丝。
//!
//! Pi 自己负责 Agent turn loop；这里刻意没有轮数、推理 token、重复工具和普通任务
//! 时长上限，只保留“完全无运行事件”和“极端总时长”两种进程安全边界。
//! 推理/重试等流式事件都算活性——模拟对抗等深推理任务可能连续数分钟只吐 reasoning,
//! 只要字节还在流动就不是挂死,不得误杀(2026-07-27 真机反馈)。

use std::time::{Duration, Instant};

use thiserror::Error;

pub const PI_IDLE_TIMEOUT_SECS: u64 = 900;
pub const PI_EXTREME_DURATION_SECS: u64 = 7_200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PiSafetyPolicy {
    idle_timeout: Duration,
    extreme_duration: Duration,
}

impl Default for PiSafetyPolicy {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(PI_IDLE_TIMEOUT_SECS),
            extreme_duration: Duration::from_secs(PI_EXTREME_DURATION_SECS),
        }
    }
}

impl PiSafetyPolicy {
    pub fn check_elapsed(
        self,
        total_elapsed: Duration,
        idle_elapsed: Duration,
    ) -> Result<(), PiSafetyViolation> {
        if total_elapsed >= self.extreme_duration {
            return Err(PiSafetyViolation::ExtremeDuration {
                limit_secs: self.extreme_duration.as_secs(),
            });
        }
        if idle_elapsed >= self.idle_timeout {
            return Err(PiSafetyViolation::Idle {
                idle_secs: self.idle_timeout.as_secs(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PiSafetyViolation {
    #[error("Pi Runtime 连续 {idle_secs}s 没有任何运行事件（正文、推理、工具调用与重试均无），进程疑似挂死，已停止本轮")]
    Idle { idle_secs: u64 },
    #[error("Pi Runtime 已达到宿主极端运行时长 {limit_secs}s")]
    ExtremeDuration { limit_secs: u64 },
}

pub struct PiSafetyGuard {
    policy: PiSafetyPolicy,
    started_at: Instant,
    last_activity_at: Instant,
    turns: u32,
}

impl Default for PiSafetyGuard {
    fn default() -> Self {
        let now = Instant::now();
        Self {
            policy: PiSafetyPolicy::default(),
            started_at: now,
            last_activity_at: now,
            turns: 0,
        }
    }
}

impl PiSafetyGuard {
    pub fn with_idle_timeout(idle_timeout: Option<Duration>) -> Self {
        let now = Instant::now();
        let mut policy = PiSafetyPolicy::default();
        if let Some(idle_timeout) = idle_timeout {
            policy.idle_timeout = idle_timeout;
        }
        Self {
            policy,
            started_at: now,
            last_activity_at: now,
            turns: 0,
        }
    }

    pub fn check(&self) -> Result<(), PiSafetyViolation> {
        self.policy
            .check_elapsed(self.started_at.elapsed(), self.last_activity_at.elapsed())
    }

    pub fn wait_remaining(&self) -> Duration {
        let idle_remaining = self
            .policy
            .idle_timeout
            .saturating_sub(self.last_activity_at.elapsed());
        let total_remaining = self
            .policy
            .extreme_duration
            .saturating_sub(self.started_at.elapsed());
        idle_remaining.min(total_remaining)
    }

    pub fn note_progress(&mut self) {
        self.last_activity_at = Instant::now();
    }

    pub fn note_turn(&mut self) {
        self.turns = self.turns.saturating_add(1);
        self.note_progress();
    }

    pub const fn turns(&self) -> u32 {
        self.turns
    }
}
