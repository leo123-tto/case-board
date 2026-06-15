//! 办案日志(`case_logs`)表读写。
//!
//! `case_logs` 承载律师手工工作日志和后续自动事件留痕，区别于 LLM
//! 从案卷里抽出的 `case_note`/`next_milestone_note`。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct CaseLog {
    pub id: String,
    pub case_id: String,
    pub occurred_at: String,
    pub content: String,
    pub source: Option<String>,
    pub source_doc_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewCaseLog {
    pub case_id: String,
    pub content: String,
    pub occurred_at: Option<String>,
    pub source: Option<String>,
}

pub async fn list_by_case(pool: &SqlitePool, case_id: &str) -> Result<Vec<CaseLog>, sqlx::Error> {
    sqlx::query_as::<_, CaseLog>(
        "SELECT * FROM case_logs WHERE case_id = ? ORDER BY occurred_at DESC, created_at DESC",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
}

pub async fn add(pool: &SqlitePool, input: NewCaseLog) -> Result<CaseLog, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let occurred_at = input
        .occurred_at
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let source = input
        .source
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "manual".to_string());

    sqlx::query(
        "INSERT INTO case_logs (id, case_id, occurred_at, content, source) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.case_id)
    .bind(&occurred_at)
    .bind(input.content.trim())
    .bind(&source)
    .execute(pool)
    .await?;

    get(pool, &id).await?.ok_or(sqlx::Error::RowNotFound)
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<u64, sqlx::Error> {
    let res = sqlx::query("DELETE FROM case_logs WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}

async fn get(pool: &SqlitePool, id: &str) -> Result<Option<CaseLog>, sqlx::Error> {
    sqlx::query_as::<_, CaseLog>("SELECT * FROM case_logs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::cases::{create_case, NewCase};
    use crate::db::init_pool;

    #[tokio::test]
    async fn case_log_round_trip() {
        let pool = init_pool(":memory:").await.unwrap();
        let case = create_case(
            &pool,
            NewCase {
                name: "日志测试".into(),
                case_type: "诉讼".into(),
                source_folder: "/tmp/case-log-test".into(),
            },
        )
        .await
        .unwrap();

        let saved = add(
            &pool,
            NewCaseLog {
                case_id: case.id.clone(),
                content: "今天联系法院确认送达地址".into(),
                occurred_at: Some("2026-06-10T09:00:00Z".into()),
                source: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(saved.source.as_deref(), Some("manual"));

        let logs = list_by_case(&pool, &case.id).await.unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].content, "今天联系法院确认送达地址");

        assert_eq!(delete(&pool, &saved.id).await.unwrap(), 1);
        assert!(list_by_case(&pool, &case.id).await.unwrap().is_empty());
    }
}
