//! 工作记录时间轴 CRUD(work_logs 表,migration 0028)。
//!
//! 记录律师为案件付出的具体劳动（打电话/写文书/会见/阅卷等）。
//! 录入时自动获取系统当前时间写入 log_time 字段。
//! 查询按 log_time DESC, id DESC 排序（倒序，最新在上）。

use serde::Serialize;
use sqlx::SqlitePool;

// ============================================================================
// 公共结构体
// ============================================================================

/// 工作记录行（前端消费的唯一格式）。
#[derive(Debug, Clone, Serialize)]
pub struct WorkLog {
    pub id: String,
    pub case_id: String,
    /// ISO 8601 完整时间戳（精确到毫秒，例如 "2026-06-13T20:07:12.345+08:00"）
    pub log_time: String,
    /// 工作内容描述
    pub content: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 新增工作记录入参（前端 / 后端共用）。
pub struct NewWorkLog {
    pub case_id: String,
    /// ISO 8601 时间戳；如果为 None 则取系统当前时间
    pub log_time: Option<String>,
    pub content: String,
}

// ============================================================================
// 查询（倒序：最新在上）
// ============================================================================

/// 取一个案件下所有工作记录，按 log_time 降序、id 降序排列。
/// 同一分钟录入多条时，后写入的 id（UUID v7 含时间戳前缀或自增特性）会在上面。
pub async fn get_work_logs(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Vec<WorkLog>, sqlx::Error> {
    let rows: Vec<(String, String, String, String, String, String)> =
        sqlx::query_as(
            "SELECT id, case_id, log_time, content, created_at, updated_at \
             FROM work_logs WHERE case_id = ? \
             ORDER BY log_time DESC, id DESC",
        )
        .bind(case_id)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .into_iter()
        .map(|(id, case_id, log_time, content, created_at, updated_at)| WorkLog {
            id,
            case_id,
            log_time,
            content,
            created_at,
            updated_at,
        })
        .collect())
}

// ============================================================================
// 新增
// ============================================================================

/// 新增一条工作记录（默认取系统当前时间）。
pub async fn add_work_log(
    pool: &SqlitePool,
    input: &NewWorkLog,
) -> Result<WorkLog, sqlx::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();

    // log_time：如果前端传入则用传入值，否则取系统当前完整 ISO 时间戳
    let log_time = input.log_time.clone().unwrap_or_else(|| {
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()
    });

    sqlx::query(
        "INSERT INTO work_logs (id, case_id, log_time, content, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.case_id)
    .bind(&log_time)
    .bind(&input.content)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;

    Ok(WorkLog {
        id,
        case_id: input.case_id.clone(),
        log_time,
        content: input.content.clone(),
        created_at: now.clone(),
        updated_at: now,
    })
}

// ============================================================================
// 更新
// ============================================================================

/// 更新一条工作记录的内容或时间。返回受影响行数。
pub async fn update_work_log(
    pool: &SqlitePool,
    id: &str,
    log_time: &str,
    content: &str,
) -> Result<u64, sqlx::Error> {
    let now = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();

    let result = sqlx::query(
        "UPDATE work_logs SET log_time = ?, content = ?, updated_at = ? WHERE id = ?",
    )
    .bind(log_time)
    .bind(content)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

// ============================================================================
// 删除
// ============================================================================

/// 删除一条工作记录。返回 true 表示成功删除。
pub async fn delete_work_log(
    pool: &SqlitePool,
    id: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM work_logs WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
