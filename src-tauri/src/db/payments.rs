//! 还款记录(2026-05-25 · case_payments 表)
//!
//! 律师在执行案件里手工录入对方实际还款,App 自动计算剩余执行款。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Payment {
    pub id: String,
    pub case_id: String,
    pub amount: f64,
    pub paid_at: String, // YYYY-MM-DD
    pub note: Option<String>,
    pub source_document_id: Option<String>,
    pub source_path: Option<String>,
    pub source_filename: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewPayment {
    pub case_id: String,
    pub amount: f64,
    pub paid_at: String,
    pub note: Option<String>,
    pub source_document_id: Option<String>,
    pub source_path: Option<String>,
    pub source_filename: Option<String>,
}

pub async fn add(pool: &SqlitePool, p: NewPayment) -> Result<Payment, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO case_payments \
         (id, case_id, amount, paid_at, note, source_document_id, source_path, source_filename) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&p.case_id)
    .bind(p.amount)
    .bind(&p.paid_at)
    .bind(&p.note)
    .bind(&p.source_document_id)
    .bind(&p.source_path)
    .bind(&p.source_filename)
    .execute(pool)
    .await?;

    sqlx::query_as::<_, Payment>("SELECT * FROM case_payments WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await
}

pub async fn list_by_case(pool: &SqlitePool, case_id: &str) -> Result<Vec<Payment>, sqlx::Error> {
    sqlx::query_as::<_, Payment>(
        "SELECT * FROM case_payments WHERE case_id = ? ORDER BY paid_at DESC, created_at DESC",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<u64, sqlx::Error> {
    let r = sqlx::query("DELETE FROM case_payments WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(r.rows_affected())
}
