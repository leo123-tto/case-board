//! 法院一张网在线立案任务(court_filing_jobs 表)的 CRUD。
//!
//! 保存每次立案任务的状态、进度、输出目录和验证码等待状态。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

/// 立案任务行结构(对应 court_filing_jobs 表)。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CourtFilingJob {
    pub id: String,
    pub case_id: String,
    pub filing_type: String,
    pub court_name: String,
    pub cookie_account: Option<String>,
    pub status: String,
    pub output_dir: Option<String>,
    pub preview_url: Option<String>,
    pub progress_json: Option<String>,
    pub captcha_active: i32,
    pub error: Option<String>,
    pub timing_json: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// 新建任务入参。
#[derive(Debug, Clone, Deserialize)]
pub struct NewCourtFilingJob {
    pub case_id: String,
    pub filing_type: String,
    pub court_name: String,
    pub cookie_account: Option<String>,
    pub output_dir: Option<String>,
}

/// 插入一条 pending 记录，返回完整行。
pub async fn insert(
    pool: &SqlitePool,
    j: &NewCourtFilingJob,
) -> Result<CourtFilingJob, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO court_filing_jobs (id, case_id, filing_type, court_name, cookie_account, output_dir) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&j.case_id)
    .bind(&j.filing_type)
    .bind(&j.court_name)
    .bind(&j.cookie_account)
    .bind(&j.output_dir)
    .execute(pool)
    .await?;

    sqlx::query_as::<_, CourtFilingJob>("SELECT * FROM court_filing_jobs WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await
}

/// 查某案件的全部立案记录，按 created_at 倒序。
pub async fn list_by_case(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Vec<CourtFilingJob>, sqlx::Error> {
    sqlx::query_as::<_, CourtFilingJob>(
        "SELECT * FROM court_filing_jobs WHERE case_id = ? ORDER BY created_at DESC",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
}

/// 查单条记录。
pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<CourtFilingJob>, sqlx::Error> {
    sqlx::query_as::<_, CourtFilingJob>("SELECT * FROM court_filing_jobs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// 检查某 case 是否有 running/waiting_captcha 的任务（防重复草稿）。
pub async fn has_active_job(pool: &SqlitePool, case_id: &str) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM court_filing_jobs WHERE case_id = ? AND status IN ('running', 'waiting_captcha', 'pending')",
    )
    .bind(case_id)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

/// 更新任务状态（基础）。
pub async fn update_status(
    pool: &SqlitePool,
    id: &str,
    status: &str,
    error: Option<&str>,
    preview_url: Option<&str>,
    timing_json: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE court_filing_jobs SET status = ?, error = ?, preview_url = ?, timing_json = ?, \
         updated_at = datetime('now') WHERE id = ?",
    )
    .bind(status)
    .bind(error)
    .bind(preview_url)
    .bind(timing_json)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 更新 progress_json（前端实时展示进度）。
pub async fn update_progress(
    pool: &SqlitePool,
    id: &str,
    progress_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE court_filing_jobs SET progress_json = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(progress_json)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 设置 captcha_active 状态。
pub async fn set_captcha_active(
    pool: &SqlitePool,
    id: &str,
    active: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE court_filing_jobs SET captcha_active = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(if active { 1 } else { 0 })
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 更新错误信息。
pub async fn set_error(pool: &SqlitePool, id: &str, error: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE court_filing_jobs SET error = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(error)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
