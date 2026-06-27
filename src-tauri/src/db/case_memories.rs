//! 案件 AI 助手记忆(case_memories 表)。
//!
//! 记忆是律师确认过的长期上下文,用于补充 prompt。它的优先级低于用户本轮消息、
//! 引用文件、工具返回和案件快照,不能替代原始材料。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use crate::chat::memory_extract::{MemoryCandidateDraft, MemoryScope};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CaseMemory {
    pub id: String,
    pub case_id: String,
    pub content: String,
    pub source: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub disabled_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MemoryEvent {
    pub id: String,
    pub case_id: Option<String>,
    pub event_type: String,
    pub user_message_id: Option<String>,
    pub assistant_message_id: Option<String>,
    pub user_text: String,
    pub assistant_text: String,
    pub source: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct MemoryCandidate {
    pub id: String,
    pub event_id: String,
    pub case_id: Option<String>,
    pub scope: String,
    pub content: String,
    pub trigger: String,
    pub confidence: f64,
    pub status: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub decided_at: Option<String>,
    pub decision_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GlobalMemory {
    pub id: String,
    pub content: String,
    pub source: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub disabled_at: Option<String>,
}

pub async fn list(
    pool: &SqlitePool,
    case_id: &str,
    include_disabled: bool,
) -> Result<Vec<CaseMemory>, sqlx::Error> {
    let sql = if include_disabled {
        "SELECT * FROM case_memories WHERE case_id = ? ORDER BY status ASC, updated_at DESC"
    } else {
        "SELECT * FROM case_memories WHERE case_id = ? AND status != 'disabled' \
         AND disabled_at IS NULL ORDER BY updated_at DESC"
    };
    sqlx::query_as::<_, CaseMemory>(sql)
        .bind(case_id)
        .fetch_all(pool)
        .await
}

pub async fn list_active(pool: &SqlitePool, case_id: &str) -> Result<Vec<CaseMemory>, sqlx::Error> {
    sqlx::query_as::<_, CaseMemory>(
        "SELECT * FROM case_memories WHERE case_id = ? AND status = 'active' \
         AND disabled_at IS NULL ORDER BY updated_at DESC LIMIT 20",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
}

pub async fn list_active_global_memories(
    pool: &SqlitePool,
) -> Result<Vec<GlobalMemory>, sqlx::Error> {
    sqlx::query_as::<_, GlobalMemory>(
        "SELECT * FROM global_memories WHERE status = 'active' \
         AND disabled_at IS NULL ORDER BY updated_at DESC LIMIT 20",
    )
    .fetch_all(pool)
    .await
}

pub async fn create(
    pool: &SqlitePool,
    case_id: &str,
    content: &str,
    source: &str,
    status: &str,
) -> Result<CaseMemory, String> {
    let content = normalize_content(content)?;
    let source = normalize_source(source);
    let status = normalize_status(status);
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO case_memories (id, case_id, content, source, status) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(case_id)
    .bind(&content)
    .bind(source)
    .bind(status)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    get(pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "案件记忆写入后未找到".to_string())
}

pub async fn create_global_memory(
    pool: &SqlitePool,
    content: &str,
    source: &str,
    status: &str,
) -> Result<GlobalMemory, String> {
    let content = normalize_content(content)?;
    let source = normalize_source(source);
    let status = normalize_status(status);
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO global_memories (id, content, source, status) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(&content)
        .bind(source)
        .bind(status)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;

    get_global_memory(pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "全局记忆写入后未找到".to_string())
}

pub async fn record_turn_event(
    pool: &SqlitePool,
    case_id: Option<&str>,
    user_message_id: &str,
    assistant_message_id: &str,
    user_text: &str,
    assistant_text: &str,
) -> Result<MemoryEvent, String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO memory_events \
         (id, case_id, event_type, user_message_id, assistant_message_id, user_text, assistant_text, source) \
         VALUES (?, ?, 'chat_turn', ?, ?, ?, ?, 'case_chat')",
    )
    .bind(&id)
    .bind(case_id)
    .bind(user_message_id)
    .bind(assistant_message_id)
    .bind(user_text)
    .bind(assistant_text)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    sqlx::query_as::<_, MemoryEvent>("SELECT * FROM memory_events WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}

pub async fn create_candidate_from_draft(
    pool: &SqlitePool,
    event_id: &str,
    draft: &MemoryCandidateDraft,
) -> Result<MemoryCandidate, String> {
    let content = normalize_content(&draft.content)?;
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO memory_candidates \
         (id, event_id, case_id, scope, content, trigger, confidence, status, source) \
         VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', 'heuristic')",
    )
    .bind(&id)
    .bind(event_id)
    .bind(draft.case_id.as_deref())
    .bind(draft.scope.as_str())
    .bind(&content)
    .bind(draft.trigger.as_str())
    .bind(draft.confidence)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    add_candidate_evidence(pool, &id, "chat_user", None, &content).await?;

    get_candidate(pool, &id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "候选记忆写入后未找到".to_string())
}

pub async fn list_pending_candidates(
    pool: &SqlitePool,
    case_id: Option<&str>,
) -> Result<Vec<MemoryCandidate>, sqlx::Error> {
    if let Some(case_id) = case_id {
        sqlx::query_as::<_, MemoryCandidate>(
            "SELECT * FROM memory_candidates \
             WHERE status = 'pending' AND (case_id = ? OR scope = 'global') \
             ORDER BY confidence DESC, updated_at DESC LIMIT 50",
        )
        .bind(case_id)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, MemoryCandidate>(
            "SELECT * FROM memory_candidates WHERE status = 'pending' \
             ORDER BY confidence DESC, updated_at DESC LIMIT 50",
        )
        .fetch_all(pool)
        .await
    }
}

pub async fn accept_candidate(pool: &SqlitePool, id: &str) -> Result<CaseMemory, String> {
    let candidate = get_candidate(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "候选记忆不存在".to_string())?;
    if candidate.status != "pending" {
        return Err("候选记忆不是待确认状态".into());
    }
    if MemoryScope::parse_db(&candidate.scope) == MemoryScope::Global {
        return Err("全局候选记忆暂未接入 active 存储".into());
    }
    let case_id = candidate
        .case_id
        .as_deref()
        .ok_or_else(|| "案件候选记忆缺少 case_id".to_string())?;

    let memory = create(
        pool,
        case_id,
        &candidate.content,
        "assistant_candidate",
        "active",
    )
    .await?;
    mark_candidate_decided(pool, id, "accepted", Some("user_accepted"))
        .await
        .map_err(|e| e.to_string())?;
    Ok(memory)
}

pub async fn accept_candidate_as_global(
    pool: &SqlitePool,
    id: &str,
) -> Result<GlobalMemory, String> {
    let candidate = get_candidate(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "候选记忆不存在".to_string())?;
    if candidate.status != "pending" {
        return Err("候选记忆不是待确认状态".into());
    }
    if MemoryScope::parse_db(&candidate.scope) != MemoryScope::Global {
        return Err("候选记忆不是全局记忆".into());
    }
    let memory =
        create_global_memory(pool, &candidate.content, "assistant_candidate", "active").await?;
    mark_candidate_decided(pool, id, "accepted", Some("user_accepted"))
        .await
        .map_err(|e| e.to_string())?;
    Ok(memory)
}

pub async fn ignore_candidate(pool: &SqlitePool, id: &str) -> Result<u64, sqlx::Error> {
    mark_candidate_decided(pool, id, "ignored", Some("user_ignored")).await
}

pub async fn update(
    pool: &SqlitePool,
    id: &str,
    content: &str,
    status: &str,
) -> Result<CaseMemory, String> {
    let content = normalize_content(content)?;
    let status = normalize_status(status);
    sqlx::query(
        "UPDATE case_memories SET content = ?, status = ?, \
         disabled_at = CASE WHEN ? = 'disabled' THEN COALESCE(disabled_at, datetime('now')) ELSE NULL END, \
         updated_at = datetime('now') WHERE id = ?",
    )
    .bind(&content)
    .bind(status)
    .bind(status)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    get(pool, id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "案件记忆不存在".to_string())
}

pub async fn disable(pool: &SqlitePool, id: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE case_memories SET status = 'disabled', disabled_at = COALESCE(disabled_at, datetime('now')), \
         updated_at = datetime('now') WHERE id = ? AND disabled_at IS NULL",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

async fn get(pool: &SqlitePool, id: &str) -> Result<Option<CaseMemory>, sqlx::Error> {
    sqlx::query_as::<_, CaseMemory>("SELECT * FROM case_memories WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

async fn get_global_memory(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<GlobalMemory>, sqlx::Error> {
    sqlx::query_as::<_, GlobalMemory>("SELECT * FROM global_memories WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

async fn get_candidate(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<MemoryCandidate>, sqlx::Error> {
    sqlx::query_as::<_, MemoryCandidate>("SELECT * FROM memory_candidates WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

async fn add_candidate_evidence(
    pool: &SqlitePool,
    candidate_id: &str,
    evidence_type: &str,
    ref_id: Option<&str>,
    quote: &str,
) -> Result<(), String> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO memory_evidence (id, candidate_id, evidence_type, ref_id, quote) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(candidate_id)
    .bind(evidence_type)
    .bind(ref_id)
    .bind(quote)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn mark_candidate_decided(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    reason: Option<&str>,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE memory_candidates SET status = ?, decision_reason = ?, decided_at = datetime('now'), \
         updated_at = datetime('now') WHERE id = ? AND status = 'pending'",
    )
    .bind(status)
    .bind(reason)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

fn normalize_content(content: &str) -> Result<String, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("案件记忆不能为空".into());
    }
    if content.chars().count() > 2_000 {
        return Err("案件记忆最多 2000 字".into());
    }
    Ok(content.to_string())
}

fn normalize_source(source: &str) -> &str {
    match source {
        "assistant_candidate" => "assistant_candidate",
        _ => "manual",
    }
}

fn normalize_status(status: &str) -> &str {
    match status {
        "candidate" => "candidate",
        "disabled" => "disabled",
        _ => "active",
    }
}
