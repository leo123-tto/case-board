//! 律师档案(lawyer_profiles 表)的 CRUD。
//!
//! 存储代理律师信息，供立案时选择（支持多律师档案）。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

/// 律师档案行结构。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct LawyerProfile {
    pub id: String,
    pub name: String,
    pub bar_number: Option<String>,
    pub law_firm: Option<String>,
    pub id_number: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub is_default: i32,
    pub created_at: String,
    pub updated_at: String,
}

/// 新建/更新律师档案入参。
#[derive(Debug, Clone, Deserialize)]
pub struct SaveLawyerProfile {
    pub name: String,
    pub bar_number: Option<String>,
    pub law_firm: Option<String>,
    pub id_number: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub is_default: Option<bool>,
}

/// 插入一条律师档案，返回完整行。
pub async fn insert(
    pool: &SqlitePool,
    p: &SaveLawyerProfile,
) -> Result<LawyerProfile, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    let is_default = if p.is_default.unwrap_or(false) { 1 } else { 0 };

    // 如果设为默认，先清除其他默认
    if is_default == 1 {
        sqlx::query("UPDATE lawyer_profiles SET is_default = 0")
            .execute(pool)
            .await?;
    }

    sqlx::query(
        "INSERT INTO lawyer_profiles (id, name, bar_number, law_firm, id_number, phone, address, is_default) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&p.name)
    .bind(&p.bar_number)
    .bind(&p.law_firm)
    .bind(&p.id_number)
    .bind(&p.phone)
    .bind(&p.address)
    .bind(is_default)
    .execute(pool)
    .await?;

    sqlx::query_as::<_, LawyerProfile>("SELECT * FROM lawyer_profiles WHERE id = ?")
        .bind(&id)
        .fetch_one(pool)
        .await
}

/// 查全部律师档案，按 created_at 倒序。
pub async fn list(pool: &SqlitePool) -> Result<Vec<LawyerProfile>, sqlx::Error> {
    sqlx::query_as::<_, LawyerProfile>(
        "SELECT * FROM lawyer_profiles ORDER BY is_default DESC, created_at DESC",
    )
    .fetch_all(pool)
    .await
}

/// 查单条记录。
pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<LawyerProfile>, sqlx::Error> {
    sqlx::query_as::<_, LawyerProfile>("SELECT * FROM lawyer_profiles WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// 更新律师档案。
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    p: &SaveLawyerProfile,
) -> Result<LawyerProfile, sqlx::Error> {
    let is_default = if p.is_default.unwrap_or(false) { 1 } else { 0 };

    if is_default == 1 {
        sqlx::query("UPDATE lawyer_profiles SET is_default = 0")
            .execute(pool)
            .await?;
    }

    sqlx::query(
        "UPDATE lawyer_profiles SET name = ?, bar_number = ?, law_firm = ?, id_number = ?, \
         phone = ?, address = ?, is_default = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(&p.name)
    .bind(&p.bar_number)
    .bind(&p.law_firm)
    .bind(&p.id_number)
    .bind(&p.phone)
    .bind(&p.address)
    .bind(is_default)
    .bind(id)
    .execute(pool)
    .await?;

    sqlx::query_as::<_, LawyerProfile>("SELECT * FROM lawyer_profiles WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
}

/// 删除律师档案。
pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM lawyer_profiles WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// 设为默认律师（清除其他默认）。
pub async fn set_default(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE lawyer_profiles SET is_default = 0")
        .execute(pool)
        .await?;
    sqlx::query(
        "UPDATE lawyer_profiles SET is_default = 1, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
