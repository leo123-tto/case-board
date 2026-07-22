//! V0.2 D5.5 · `chat_tasks` 表 CRUD(长任务持久化 + 重启恢复)。
//!
//! 表 schema 在 migration 0018 已落(详 docs/V0.2-法律AI工作台-实施计划.md § 3.1)。
//! 本模块只提供 Rust 端 CRUD。设计要点:
//!
//! - **状态机**:`planning` → `executing` → `synthesizing` → `verifying`
//!   → `done` / `failed` / `cancelled`。CRUD 层不强制校验流转方向,
//!   由 agent_loop 调用方负责;校验只在 `is_valid_status()` 做存量保护。
//! - **patch 风格 update**:`UpdateChatTask` 全字段 Option,只写 Some 的,
//!   避免每加一个新字段都写一个新函数,也避免错把 NULL 覆盖已有值。
//! - **orphan 检测**:启动时调 `resume_orphaned_chat_tasks`,把任何活跃状态
//!   且 `started_at` 早于阈值的任务标 failed。阈值由调用方传,通常 5 分钟。
//! - **隐私**:`error_short` 入库前由调用方负责脱敏(`feedback::sanitize_paths`),
//!   本模块不做(职责分离)。
//!
//! 涉坑:CLAUDE.md 坑 #3(0018 已发布,schema 一旦改要走 0019+)、
//!       坑 #14(LLM Schema 嵌套字段默认 Option,跟本模块 patch 一致)。

use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

/// `chat_tasks` 表行。字段与 0018 migration 一一对应。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ChatTask {
    pub id: String,
    pub case_id: String,
    pub conversation_id: Option<String>,
    pub message_id: String,
    pub task_type: String,
    pub status: String,
    pub attached_doc_ids: Option<String>,
    pub plan_json: Option<String>,
    pub subtask_results_json: Option<String>,
    pub tool_calls_json: Option<String>,
    pub citations_json: Option<String>,
    pub verification_passes: Option<i64>,
    pub yuandian_credits_used: i64,
    pub kb_hits: i64,
    pub yuandian_calls: i64,
    pub model_used: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cache_hit_tokens: Option<i64>,
    pub artifact_doc_id: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub error_short: Option<String>,
}

/// 新建 chat_task 入参。仅必填字段,可选字段走 `UpdateChatTask`。
#[derive(Debug, Clone)]
pub struct NewChatTask<'a> {
    pub id: &'a str,
    pub case_id: &'a str,
    pub conversation_id: Option<&'a str>,
    pub message_id: &'a str,
    pub task_type: &'a str,
    /// 通常 `"planning"`(主 agent 还在拆任务),也可以直接 `"executing"`。
    pub status: &'a str,
    /// JSON 数组字符串(`["doc-uuid-1", ...]`);None 表示这次任务不引用文档。
    pub attached_doc_ids: Option<&'a str>,
}

/// patch 风格更新结构。全字段 Option:`None` 表示**不动**,`Some` 表示写入。
///
/// 不区分"写 NULL"和"不动" — 一旦 chat_task 走到下一阶段,旧字段不应该被清掉。
/// 如果未来真有"清空字段"语义,再加一个 `clear_*` 列。
#[derive(Debug, Clone, Default)]
pub struct UpdateChatTask<'a> {
    pub status: Option<&'a str>,
    pub plan_json: Option<&'a str>,
    pub subtask_results_json: Option<&'a str>,
    pub tool_calls_json: Option<&'a str>,
    pub citations_json: Option<&'a str>,
    pub verification_passes: Option<i64>,
    pub yuandian_credits_used: Option<i64>,
    pub kb_hits: Option<i64>,
    pub yuandian_calls: Option<i64>,
    pub model_used: Option<&'a str>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cache_hit_tokens: Option<i64>,
    pub artifact_doc_id: Option<&'a str>,
    pub finished_at: Option<&'a str>,
    pub error_short: Option<&'a str>,
}

/// 合法状态白名单(`status` 列只接受这 7 个值)。
pub const VALID_STATUSES: &[&str] = &[
    "planning",
    "executing",
    "synthesizing",
    "verifying",
    "done",
    "failed",
    "cancelled",
];

/// 状态属于"任务仍在跑"(orphan 检测的目标集合)。
pub fn is_active_status(status: &str) -> bool {
    matches!(
        status,
        "planning" | "executing" | "synthesizing" | "verifying"
    )
}

/// 状态属于"任务已收尾"。`done` / `failed` / `cancelled` 算终态。
pub fn is_terminal_status(status: &str) -> bool {
    matches!(status, "done" | "failed" | "cancelled")
}

/// 校验状态字符串是否合法。CRUD 层防御性校验,避免脏数据入库。
pub fn is_valid_status(status: &str) -> bool {
    VALID_STATUSES.contains(&status)
}

// ============================================================================
// CRUD
// ============================================================================

/// 插入一条 chat_task。`started_at` 自动取当前 UTC 时间(RFC3339)。
pub async fn create_chat_task(pool: &SqlitePool, task: NewChatTask<'_>) -> Result<(), sqlx::Error> {
    if !is_valid_status(task.status) {
        return Err(sqlx::Error::Protocol(format!(
            "invalid chat_task status: {}",
            task.status
        )));
    }
    let started_at = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO chat_tasks \
         (id, case_id, conversation_id, message_id, task_type, status, attached_doc_ids, started_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(task.id)
    .bind(task.case_id)
    .bind(task.conversation_id)
    .bind(task.message_id)
    .bind(task.task_type)
    .bind(task.status)
    .bind(task.attached_doc_ids)
    .bind(&started_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// 取一条 chat_task;不存在返回 `Ok(None)`。
pub async fn get_chat_task(pool: &SqlitePool, id: &str) -> Result<Option<ChatTask>, sqlx::Error> {
    sqlx::query_as::<_, ChatTask>("SELECT * FROM chat_tasks WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// 按案件列出 chat_tasks,按 `started_at` 倒序(新的在前)。
#[allow(dead_code)]
pub async fn list_chat_tasks_by_case(
    pool: &SqlitePool,
    case_id: &str,
    limit: Option<i64>,
) -> Result<Vec<ChatTask>, sqlx::Error> {
    match limit {
        Some(n) => {
            sqlx::query_as::<_, ChatTask>(
                "SELECT * FROM chat_tasks WHERE case_id = ? ORDER BY started_at DESC LIMIT ?",
            )
            .bind(case_id)
            .bind(n)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, ChatTask>(
                "SELECT * FROM chat_tasks WHERE case_id = ? ORDER BY started_at DESC",
            )
            .bind(case_id)
            .fetch_all(pool)
            .await
        }
    }
}

/// patch 风格 update。只更新 `patch` 里 `Some` 的字段,其他列原样保留。
///
/// 用动态 SQL 拼接而不是 17 个 if let 各发一条 update — 性能 / 原子性都好。
pub async fn update_chat_task(
    pool: &SqlitePool,
    id: &str,
    patch: UpdateChatTask<'_>,
) -> Result<(), sqlx::Error> {
    if let Some(s) = patch.status {
        if !is_valid_status(s) {
            return Err(sqlx::Error::Protocol(format!(
                "invalid chat_task status: {}",
                s
            )));
        }
    }

    let mut sets: Vec<&'static str> = Vec::new();
    let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new("UPDATE chat_tasks SET ");

    macro_rules! push_field {
        ($field:expr, $col:literal) => {
            if let Some(v) = $field {
                if !sets.is_empty() {
                    q.push(", ");
                }
                q.push(concat!($col, " = "));
                q.push_bind(v);
                sets.push($col);
            }
        };
    }

    push_field!(patch.status, "status");
    push_field!(patch.plan_json, "plan_json");
    push_field!(patch.subtask_results_json, "subtask_results_json");
    push_field!(patch.tool_calls_json, "tool_calls_json");
    push_field!(patch.citations_json, "citations_json");
    push_field!(patch.verification_passes, "verification_passes");
    push_field!(patch.yuandian_credits_used, "yuandian_credits_used");
    push_field!(patch.kb_hits, "kb_hits");
    push_field!(patch.yuandian_calls, "yuandian_calls");
    push_field!(patch.model_used, "model_used");
    push_field!(patch.prompt_tokens, "prompt_tokens");
    push_field!(patch.completion_tokens, "completion_tokens");
    push_field!(patch.cache_hit_tokens, "cache_hit_tokens");
    push_field!(patch.artifact_doc_id, "artifact_doc_id");
    push_field!(patch.finished_at, "finished_at");
    push_field!(patch.error_short, "error_short");

    if sets.is_empty() {
        // patch 全 None,什么都不更新;省一次空 SQL。
        return Ok(());
    }

    q.push(" WHERE id = ");
    q.push_bind(id);
    q.build().execute(pool).await?;
    Ok(())
}

/// 收尾任务的便捷方法:写 status + finished_at + 可选 error_short。
///
/// `status` 必须是 terminal(`done` / `failed` / `cancelled`),否则报错。
pub async fn finish_chat_task(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    error_short: Option<&str>,
) -> Result<(), sqlx::Error> {
    if !is_terminal_status(status) {
        return Err(sqlx::Error::Protocol(format!(
            "finish_chat_task 只能用 terminal status,收到 {}",
            status
        )));
    }
    let finished_at = Utc::now().to_rfc3339();
    update_chat_task(
        pool,
        id,
        UpdateChatTask {
            status: Some(status),
            finished_at: Some(&finished_at),
            error_short,
            ..Default::default()
        },
    )
    .await
}

/// 找"孤儿"任务:活跃状态(planning/executing/synthesizing/verifying)
/// 且 `started_at` 早于 (now - older_than) 的全部任务。
///
/// 启动时调一次,假定上次进程崩溃前没机会写 terminal 状态。
pub async fn find_orphans(
    pool: &SqlitePool,
    older_than: Duration,
) -> Result<Vec<ChatTask>, sqlx::Error> {
    let cutoff = (Utc::now() - older_than).to_rfc3339();
    sqlx::query_as::<_, ChatTask>(
        "SELECT * FROM chat_tasks \
         WHERE status IN ('planning','executing','synthesizing','verifying') \
           AND started_at < ? \
         ORDER BY started_at ASC",
    )
    .bind(&cutoff)
    .fetch_all(pool)
    .await
}

/// 标记 orphan 失败。`error_short` 入库前应已脱敏。
pub async fn mark_failed(
    pool: &SqlitePool,
    id: &str,
    error_short: &str,
) -> Result<(), sqlx::Error> {
    finish_chat_task(pool, id, "failed", Some(error_short)).await
}

/// 启动钩子:把所有活跃且超过 `older_than_secs` 秒的任务标失败,留给前端展示「重试」。
///
/// **不返回 Err**:启动恢复失败不应该阻止 app 启动,失败时 eprintln 走 dlog ring。
/// 调用方(`lib.rs setup`)拿到的是 `mark_failed_count`。
pub async fn resume_orphaned_chat_tasks(pool: &SqlitePool, older_than_secs: i64) -> usize {
    let older_than = Duration::seconds(older_than_secs);
    let orphans = match find_orphans(pool, older_than).await {
        Ok(v) => v,
        Err(e) => {
            crate::dlog!("resume_orphaned_chat_tasks find_orphans failed: {}", e);
            return 0;
        }
    };
    let mut ok = 0usize;
    for t in &orphans {
        if let Err(e) = mark_failed(pool, &t.id, "上次任务异常中断,可点击重试").await {
            crate::dlog!(
                "resume_orphaned_chat_tasks mark_failed({}) failed: {}",
                t.id,
                e
            );
            continue;
        }
        ok += 1;
    }
    ok
}

// ============================================================================
// 测试
// ============================================================================
