//! 要素式审判智能辅助模块(2026-06-22)
//!
//! 核心功能:
//! 1. 庭审前 — 案由识别→要素匹配→AI 事实提取→要素式起诉状/答辩状自动填充
//! 2. 庭审中 — 证据→要件→争点三重归依,法院认定预测
//! 3. 攻防策略 — 三级递进(主张责任→证明责任→举证行为)+ 抗辩预判

pub mod prompts;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

// ───── 数据模型 ─────

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ElementTemplate {
    pub id: String,
    pub cause: String,
    pub direction: String,
    pub element_name: String,
    pub element_desc: String,
    pub is_required: bool,
    pub evidence_type: Option<String>,
    pub evidence_hint: Option<String>,
    pub burden_party: Option<String>,
    pub sort_order: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ElementFact {
    pub id: String,
    pub case_id: String,
    pub stage: Option<String>,
    pub template_id: Option<String>,
    pub fact_name: String,
    pub fact_desc: Option<String>,
    pub claim_party: Option<String>,
    pub evidence_ids: Option<String>,
    pub proof_status: String,
    pub opponent_rebuttal: Option<String>,
    pub court_finding: Option<String>,
    pub is_established: Option<bool>,
    pub is_disputed: bool,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct TrialStrategy {
    pub id: String,
    pub case_id: String,
    pub stage: Option<String>,
    pub strategy_layer: String,
    pub strategy_content: String,
    pub target_fact_ids: Option<String>,
    pub predicted_opponent_strategy: Option<String>,
    pub evidence_gap_analysis: Option<String>,
    pub recommended_actions: Option<String>,
    pub risk_level: Option<String>,
    pub is_adopted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ElementComplaint {
    pub id: String,
    pub case_id: String,
    pub doc_type: String,
    pub direction: String,
    pub content_md: String,
    pub filled_elements: Option<String>,
    pub version: i64,
    pub is_final: bool,
}

// ───── 案件摘要(传给 AI 用) ─────

#[derive(Debug, Clone, Serialize)]
pub struct CaseSummaryForAI {
    pub case_name: String,
    pub cause: Option<String>,
    pub case_no: Option<String>,
    pub court: Option<String>,
    pub stage: Option<String>,
    pub our_side: Option<String>,
    pub parties: Vec<PartyBrief>,
    pub key_dates: Vec<EventBrief>,
    pub doc_summaries: Vec<DocBrief>,
    pub element_facts: Vec<ElementFact>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PartyBrief {
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventBrief {
    pub occurred_at: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocBrief {
    pub filename: String,
    pub category: Option<String>,
    pub stage: Option<String>,
    pub extraction_summary: Option<String>,
}

// ───── DB 查询 ─────

/// 获取案由下所有要素模板
pub async fn get_templates(
    pool: &SqlitePool,
    cause: &str,
    direction: Option<&str>,
) -> Result<Vec<ElementTemplate>, String> {
    let rows = if let Some(dir) = direction {
        sqlx::query_as::<_, ElementTemplate>(
            "SELECT * FROM element_templates WHERE cause = ? AND direction = ? ORDER BY sort_order",
        )
        .bind(cause)
        .bind(dir)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, ElementTemplate>(
            "SELECT * FROM element_templates WHERE cause = ? ORDER BY direction, sort_order",
        )
        .bind(cause)
        .fetch_all(pool)
        .await
    }
    .map_err(|e| format!("查询要素模板失败: {e}"))?;
    Ok(rows)
}

/// 列出所有已预置模板的案由
pub async fn list_template_causes(pool: &SqlitePool) -> Result<Vec<String>, String> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT DISTINCT cause FROM element_templates ORDER BY cause",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询案由列表失败: {e}"))?;
    Ok(rows.into_iter().map(|(c,)| c).collect())
}

/// 获取案件的要件事实列表
pub async fn get_element_facts(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Vec<ElementFact>, String> {
    sqlx::query_as::<_, ElementFact>(
        "SELECT * FROM element_facts WHERE case_id = ? ORDER BY created_at",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询要件事实失败: {e}"))
}

/// 获取争点列表
pub async fn get_disputed_facts(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Vec<ElementFact>, String> {
    sqlx::query_as::<_, ElementFact>(
        "SELECT * FROM element_facts WHERE case_id = ? AND is_disputed = 1",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询争点失败: {e}"))
}

/// 获取案件的攻防策略
pub async fn get_strategies(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Vec<TrialStrategy>, String> {
    sqlx::query_as::<_, TrialStrategy>(
        "SELECT * FROM trial_strategies WHERE case_id = ? ORDER BY created_at",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询攻防策略失败: {e}"))
}

/// 获取已生成的要素式文书
pub async fn get_complaints(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Vec<ElementComplaint>, String> {
    sqlx::query_as::<_, ElementComplaint>(
        "SELECT * FROM element_complaints WHERE case_id = ? ORDER BY created_at DESC",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询文书失败: {e}"))
}

/// 保存/更新要件事实(批量 upsert)
pub async fn upsert_element_facts(
    pool: &SqlitePool,
    facts: &[ElementFact],
) -> Result<(), String> {
    for fact in facts {
        sqlx::query(
            "INSERT INTO element_facts (id, case_id, stage, template_id, fact_name, fact_desc, claim_party, evidence_ids, proof_status, opponent_rebuttal, court_finding, is_established, is_disputed, notes, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
             ON CONFLICT(id) DO UPDATE SET
               stage=excluded.stage, fact_desc=excluded.fact_desc, claim_party=excluded.claim_party,
               evidence_ids=excluded.evidence_ids, proof_status=excluded.proof_status,
               opponent_rebuttal=excluded.opponent_rebuttal, court_finding=excluded.court_finding,
               is_established=excluded.is_established, is_disputed=excluded.is_disputed,
               notes=excluded.notes, updated_at=datetime('now')",
        )
        .bind(&fact.id)
        .bind(&fact.case_id)
        .bind(&fact.stage)
        .bind(&fact.template_id)
        .bind(&fact.fact_name)
        .bind(&fact.fact_desc)
        .bind(&fact.claim_party)
        .bind(&fact.evidence_ids)
        .bind(&fact.proof_status)
        .bind(&fact.opponent_rebuttal)
        .bind(&fact.court_finding)
        .bind(&fact.is_established)
        .bind(fact.is_disputed)
        .bind(&fact.notes)
        .execute(pool)
        .await
        .map_err(|e| format!("保存要件事实失败: {e}"))?;
    }
    Ok(())
}

/// 保存攻防策略
pub async fn save_strategy(
    pool: &SqlitePool,
    strategy: &TrialStrategy,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO trial_strategies (id, case_id, stage, strategy_layer, strategy_content, target_fact_ids, predicted_opponent_strategy, evidence_gap_analysis, recommended_actions, risk_level)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&strategy.id)
    .bind(&strategy.case_id)
    .bind(&strategy.stage)
    .bind(&strategy.strategy_layer)
    .bind(&strategy.strategy_content)
    .bind(&strategy.target_fact_ids)
    .bind(&strategy.predicted_opponent_strategy)
    .bind(&strategy.evidence_gap_analysis)
    .bind(&strategy.recommended_actions)
    .bind(&strategy.risk_level)
    .execute(pool)
    .await
    .map_err(|e| format!("保存策略失败: {e}"))?;
    Ok(())
}

/// 保存要素式文书
pub async fn save_complaint(
    pool: &SqlitePool,
    complaint: &ElementComplaint,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO element_complaints (id, case_id, doc_type, direction, content_md, filled_elements, version, is_final)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&complaint.id)
    .bind(&complaint.case_id)
    .bind(&complaint.doc_type)
    .bind(&complaint.direction)
    .bind(&complaint.content_md)
    .bind(&complaint.filled_elements)
    .bind(complaint.version)
    .bind(complaint.is_final)
    .execute(pool)
    .await
    .map_err(|e| format!("保存文书失败: {e}"))?;
    Ok(())
}
