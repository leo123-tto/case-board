//! 文档标记层(2026-06-19 · 源文件看板重构 Phase 3 · `document_tags` 表)。
//!
//! 两条独立标记轴:
//! - `importance`:`重要` / `忽略`(普通=不打标)。**单值**(set 前先删同 namespace 旧值)。
//! - `party_side`:`原告` / `被告` / `第三人`。**可多值**(toggle 单个值)。
//! - `evidence_attitude`:`有利` / `不利` / `中性`。**单值**,用于证据目录筛材。
//! - `submission_stage`:材料提交阶段。**单值**,用于判断是否新证据 / 补充证据。
//!
//! `source`:`user`(人工)/ `ai_suggest`(Phase 3b AI 建议)。标记挂 `documents.id`,
//! 软删/重扫保留同行 id → 刷新不丢(见 0033 注释)。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

pub const NS_IMPORTANCE: &str = "importance";
pub const NS_PARTY_SIDE: &str = "party_side";
pub const NS_CATEGORY: &str = "category";
pub const NS_EVIDENCE_ATTITUDE: &str = "evidence_attitude";
pub const NS_SUBMISSION_STAGE: &str = "submission_stage";

/// 自动归类的固定分类(AI 与人工都从这套里选,单值)。
pub const CATEGORIES: &[&str] = &[
    "起诉材料",
    "证据",
    "法院文书",
    "对方材料",
    "程序文书",
    "参考材料",
    "其他",
];

pub const EVIDENCE_ATTITUDES: &[&str] = &["有利", "不利", "中性"];

pub const SUBMISSION_STAGES: &[&str] = &[
    "起诉/答辩随附",
    "举证期限内",
    "补充提交",
    "二审新证据",
    "未提交或待确认",
];

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct DocumentTag {
    pub id: String,
    pub document_id: String,
    pub namespace: String,
    pub value: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 校验 (namespace, value) 合法,挡住脏数据。
fn valid(namespace: &str, value: &str) -> bool {
    match namespace {
        NS_IMPORTANCE => matches!(value, "重要" | "忽略"),
        NS_PARTY_SIDE => matches!(value, "原告" | "被告" | "第三人"),
        NS_CATEGORY => CATEGORIES.contains(&value),
        NS_EVIDENCE_ATTITUDE => EVIDENCE_ATTITUDES.contains(&value),
        NS_SUBMISSION_STAGE => SUBMISSION_STAGES.contains(&value),
        _ => false,
    }
}

/// 列出某案件下全部文档的标记(join documents 按 case 过滤;含已软删的文档行,前端自行决定显隐)。
pub async fn list_by_case(pool: &SqlitePool, case_id: &str) -> sqlx::Result<Vec<DocumentTag>> {
    sqlx::query_as::<_, DocumentTag>(
        "SELECT t.id, t.document_id, t.namespace, t.value, t.source, t.created_at, t.updated_at \
         FROM document_tags t JOIN documents d ON d.id = t.document_id \
         WHERE d.case_id = ?1",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
}

/// 设置某文档某单值 namespace(importance / category):先清该 doc 同 ns 标记,再可选写入新值。
/// `value = None` → 清空。`source` 区分人工 / AI 建议。
async fn set_single(
    pool: &SqlitePool,
    document_id: &str,
    namespace: &str,
    value: Option<&str>,
    source: &str,
) -> Result<(), String> {
    if let Some(v) = value {
        if !valid(namespace, v) {
            return Err(format!("非法 {namespace} 值: {v}"));
        }
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM document_tags WHERE document_id = ?1 AND namespace = ?2")
        .bind(document_id)
        .bind(namespace)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(v) = value {
        sqlx::query(
            "INSERT INTO document_tags (id, document_id, namespace, value, source) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(document_id)
        .bind(namespace)
        .bind(v)
        .bind(source)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())
}

/// 人工设 importance(单值,`None`=清空)。
pub async fn set_importance(
    pool: &SqlitePool,
    document_id: &str,
    value: Option<&str>,
) -> Result<(), String> {
    set_single(pool, document_id, NS_IMPORTANCE, value, "user").await
}

/// 人工设 category(单值,`None`=清空)。
pub async fn set_category(
    pool: &SqlitePool,
    document_id: &str,
    value: Option<&str>,
) -> Result<(), String> {
    set_single(pool, document_id, NS_CATEGORY, value, "user").await
}

/// 人工设证据倾向(单值,`None`=清空)。
pub async fn set_evidence_attitude(
    pool: &SqlitePool,
    document_id: &str,
    value: Option<&str>,
) -> Result<(), String> {
    set_single(pool, document_id, NS_EVIDENCE_ATTITUDE, value, "user").await
}

/// 人工设提交阶段(单值,`None`=清空)。
pub async fn set_submission_stage(
    pool: &SqlitePool,
    document_id: &str,
    value: Option<&str>,
) -> Result<(), String> {
    set_single(pool, document_id, NS_SUBMISSION_STAGE, value, "user").await
}

/// AI 写建议(单值 namespace):**若该 doc 此 ns 已有人工标记则跳过**(人工永远优先),
/// 否则覆盖旧的 ai_suggest。给「AI 自动整理」用。
pub async fn set_ai_suggestion(
    pool: &SqlitePool,
    document_id: &str,
    namespace: &str,
    value: &str,
) -> Result<(), String> {
    if !valid(namespace, value) {
        return Err(format!("非法 {namespace} 值: {value}"));
    }
    let has_user: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM document_tags \
         WHERE document_id = ?1 AND namespace = ?2 AND source = 'user' LIMIT 1",
    )
    .bind(document_id)
    .bind(namespace)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    if has_user.is_some() {
        return Ok(()); // 不覆盖人工标记
    }
    set_single(pool, document_id, namespace, Some(value), "ai_suggest").await
}

/// AI 写 party_side 建议(多值):若已有任一人工 party_side,则完全跳过。
pub async fn set_ai_party_suggestions(
    pool: &SqlitePool,
    document_id: &str,
    values: &[String],
) -> Result<(), String> {
    let cleaned: Vec<&str> = values
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .collect();
    for v in &cleaned {
        if !valid(NS_PARTY_SIDE, v) {
            return Err(format!("非法 party_side 值: {v}"));
        }
    }
    let has_user: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM document_tags \
         WHERE document_id = ?1 AND namespace = ?2 AND source = 'user' LIMIT 1",
    )
    .bind(document_id)
    .bind(NS_PARTY_SIDE)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    if has_user.is_some() {
        return Ok(());
    }
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    sqlx::query(
        "DELETE FROM document_tags \
         WHERE document_id = ?1 AND namespace = ?2 AND source = 'ai_suggest'",
    )
    .bind(document_id)
    .bind(NS_PARTY_SIDE)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    for v in cleaned {
        sqlx::query(
            "INSERT OR IGNORE INTO document_tags (id, document_id, namespace, value, source) \
             VALUES (?1, ?2, ?3, ?4, 'ai_suggest')",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(document_id)
        .bind(NS_PARTY_SIDE)
        .bind(v)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    tx.commit().await.map_err(|e| e.to_string())
}

/// 切换某文档的一个 party_side 值(可多值):`enabled=true` 加,`false` 删。
pub async fn set_party_side(
    pool: &SqlitePool,
    document_id: &str,
    value: &str,
    enabled: bool,
) -> Result<(), String> {
    if !valid(NS_PARTY_SIDE, value) {
        return Err(format!("非法 party_side 值: {value}"));
    }
    if enabled {
        sqlx::query(
            "DELETE FROM document_tags \
             WHERE document_id = ?1 AND namespace = ?2 AND value = ?3 AND source = 'ai_suggest'",
        )
        .bind(document_id)
        .bind(NS_PARTY_SIDE)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        sqlx::query(
            "INSERT OR IGNORE INTO document_tags (id, document_id, namespace, value, source) \
             VALUES (?1, ?2, ?3, ?4, 'user')",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(document_id)
        .bind(NS_PARTY_SIDE)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    } else {
        sqlx::query(
            "DELETE FROM document_tags WHERE document_id = ?1 AND namespace = ?2 AND value = ?3",
        )
        .bind(document_id)
        .bind(NS_PARTY_SIDE)
        .bind(value)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 批量设 importance(文件夹级整批标记)。空列表直接返回。
pub async fn set_importance_batch(
    pool: &SqlitePool,
    document_ids: &[String],
    value: Option<&str>,
) -> Result<(), String> {
    for id in document_ids {
        set_importance(pool, id, value).await?;
    }
    Ok(())
}

pub async fn set_category_batch(
    pool: &SqlitePool,
    document_ids: &[String],
    value: Option<&str>,
) -> Result<(), String> {
    for id in document_ids {
        set_category(pool, id, value).await?;
    }
    Ok(())
}

pub async fn set_evidence_attitude_batch(
    pool: &SqlitePool,
    document_ids: &[String],
    value: Option<&str>,
) -> Result<(), String> {
    for id in document_ids {
        set_evidence_attitude(pool, id, value).await?;
    }
    Ok(())
}

pub async fn set_submission_stage_batch(
    pool: &SqlitePool,
    document_ids: &[String],
    value: Option<&str>,
) -> Result<(), String> {
    for id in document_ids {
        set_submission_stage(pool, id, value).await?;
    }
    Ok(())
}

/// 批量切换 party_side(文件夹级整批标记)。
pub async fn set_party_side_batch(
    pool: &SqlitePool,
    document_ids: &[String],
    value: &str,
    enabled: bool,
) -> Result<(), String> {
    for id in document_ids {
        set_party_side(pool, id, value, enabled).await?;
    }
    Ok(())
}
