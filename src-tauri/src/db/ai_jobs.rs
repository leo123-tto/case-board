//! AI job 状态与案件分析陈旧标记。
//!
//! 这是统一队列的基础设施层:先把状态、输入签名和陈旧标记落库,
//! 暂不改变现有的后台执行方式。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiJob {
    pub id: String,
    pub case_id: Option<String>,
    pub kind: String,
    pub status: String,
    pub phase: Option<String>,
    pub progress: f64,
    pub input_signature: Option<String>,
    pub output_refs_json: Option<String>,
    pub error_sanitized: Option<String>,
    pub provider: Option<String>,
    pub cost_json: Option<String>,
    pub cancellable: bool,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

pub async fn create_job(
    pool: &SqlitePool,
    case_id: Option<&str>,
    kind: &str,
    input_signature: Option<&str>,
    provider: Option<&str>,
) -> Result<AiJob, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO ai_jobs (id, case_id, kind, input_signature, provider) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(case_id)
    .bind(kind)
    .bind(input_signature)
    .bind(provider)
    .execute(pool)
    .await?;
    get_job(pool, &id).await?.ok_or(sqlx::Error::RowNotFound)
}

pub async fn get_job(pool: &SqlitePool, id: &str) -> Result<Option<AiJob>, sqlx::Error> {
    sqlx::query_as::<_, AiJob>("SELECT * FROM ai_jobs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn update_job_status(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    phase: Option<&str>,
    progress: f64,
    error_sanitized: Option<&str>,
) -> Result<u64, sqlx::Error> {
    let finished_status = matches!(status, "succeeded" | "failed" | "cancelled");
    let res = sqlx::query(
        "UPDATE ai_jobs SET status = ?, phase = ?, progress = ?, error_sanitized = ?, \
            updated_at = datetime('now'), \
            finished_at = CASE WHEN ? THEN datetime('now') ELSE finished_at END \
         WHERE id = ?",
    )
    .bind(status)
    .bind(phase)
    .bind(progress.clamp(0.0, 1.0))
    .bind(error_sanitized)
    .bind(finished_status)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn mark_case_analysis_stale(
    pool: &SqlitePool,
    case_id: &str,
    reason: &str,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE cases SET analysis_stale = 1, analysis_stale_reason = ?, \
            updated_at = datetime('now') \
         WHERE id = ?",
    )
    .bind(reason)
    .bind(case_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn mark_case_analysis_current(
    pool: &SqlitePool,
    case_id: &str,
    input_signature: &str,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE cases SET analysis_input_signature = ?, analysis_stale = 0, \
            analysis_stale_reason = NULL, updated_at = datetime('now') \
         WHERE id = ?",
    )
    .bind(input_signature)
    .bind(case_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}
