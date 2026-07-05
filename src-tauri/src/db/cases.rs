//! 案件(`cases`)表的 CRUD。
//!
//! 单一职责:把案件元数据落库 / 读出来。文档相关的操作在 [`super::documents`]。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

/// 案件主表的行结构。
///
/// 字段命名跟 SQL schema 一致(snake_case)。前端拿到的 JSON 也用 snake_case
/// (跟 ScannedDoc 一致),保持整个 IPC 数据风格统一。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Case {
    pub id: String,
    pub name: String,
    pub case_type: String,
    pub cause: Option<String>,
    pub case_no: Option<String>,
    pub court: Option<String>,
    pub judge_id: Option<String>,
    pub stage: Option<String>,
    pub source_folder: String,
    pub ai_summary_md: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_scanned_at: Option<String>,

    // ====== 2026-05-23 加(migration 0002) ======
    /// 案件级聚合字段(由 aggregator 从 documents.extracted_fields 算出)
    pub agg_case_no: Option<String>,
    pub agg_court: Option<String>,
    pub agg_cause: Option<String>,
    pub agg_plaintiffs: Option<String>,    // JSON array
    pub agg_defendants: Option<String>,    // JSON array
    pub agg_third_parties: Option<String>, // JSON array
    pub agg_judges: Option<String>,        // JSON array
    pub agg_claim_amount: Option<f64>,
    pub agg_filed_at: Option<String>,
    pub agg_computed_at: Option<String>,

    /// 下一关键节点(驱动首页 30 天 widget)
    pub next_milestone_type: Option<String>,
    pub next_milestone_at: Option<String>,
    pub next_milestone_status: Option<String>,
    pub next_milestone_note: Option<String>,

    /// 案件总状态(进行中/已结案/已归档)
    pub case_status: String,

    /// 执行款追踪聚合
    pub execution_total: Option<f64>,
    pub execution_total_breakdown: Option<String>, // JSON
    pub execution_started_at: Option<String>,
    pub execution_received: Option<f64>,
    pub execution_remaining: Option<f64>,

    /// ====== 2026-05-24 加(migration 0006)======
    /// 案件工作流状态(看板卡片右上角的"接案/立案中/待开庭/审理中/上诉期/二审中/执行中/已结案")
    /// NULL = 走前端自动推断;非 NULL = 用户手工选过,优先取用户值
    pub workflow_status: Option<String>,

    /// ====== 2026-05-24 h 加(migration 0008 · LLM 全局抽方案)======
    /// LLM 全局抽出的扩展字段(替代旧 aggregator 规则)
    /// 一句话案件概括(50 字内)
    pub case_summary: Option<String>,
    /// 完整案件分析报告 MD 路径(详情页「📖 案件报告」按钮渲染)
    pub case_report_path: Option<String>,
    pub case_report_generated_at: Option<String>,
    /// 调解 / 判决 / 执行结果(自由文本,200 字内)
    pub agg_resolution: Option<String>,
    /// LLM 推断的状态文字(跟 workflow_status 11 档不同,自由描述)
    pub agg_status_text: Option<String>,
    /// JSON: [{name,role,id_no,address,phone,is_our_side}]
    pub agg_party_contacts: Option<String>,
    /// JSON: [{name,role,phone}]
    pub agg_court_contacts: Option<String>,
    /// JSON: [{date,event,note}]
    pub agg_key_dates: Option<String>,
    /// JSON: [{item,amount,note}]
    pub agg_fees: Option<String>,

    /// 2026-05-24 k 加(migration 0010 · 元典查被执行人 P1)
    /// 风险提示报告 MD 路径(详情页「🔍 查被执行人」按钮触发,跑完落盘)
    pub risk_assessment_path: Option<String>,
    pub risk_assessment_at: Option<String>,

    /// 2026-05-24 k-9 加(migration 0011 · P2 深挖)
    /// 深查报告 MD 路径(详情页「🔬 深挖」按钮触发)
    pub deep_dive_report_path: Option<String>,
    pub deep_dive_at: Option<String>,

    /// 2026-05-25 V0.1.7 加(migration 0013 · 完整报告)
    /// 合并风险报告 + 深挖报告 → DeepSeek 出第三份完整报告
    pub full_report_path: Option<String>,
    pub full_report_at: Option<String>,

    /// 2026-05-26 V0.1.13 加(migration 0016 · 编辑模式 user overrides)
    /// 用户手改的 overlay(JSON),前端定义结构,后端透传。LLM 全局抽永不覆盖此列。
    /// 渲染时叠加在 agg_* 之上,使用户改动优先级高于 LLM 抽取。
    pub user_overrides_json: Option<String>,

    /// 2026-06-11 加(migration 0022 · 审级模型)
    /// 当前承办机关类型('法院'/'仲裁委'/'其他'),驱动前端 label。
    /// agg_court/agg_case_no 自此语义=「当前审级」快照,全部审级明细在 case_instances 表。
    pub agg_court_type: Option<String>,

    /// 2026-06-13 加(migration 0023 · 我方代理立场)
    /// 我方代理地位:'原告方'/'被告方'/'第三人'/'反诉混合'/NULL(未知)。
    /// LLM 从 is_our_side=true 当事人推断;用户改值走 user_overrides_json(fields.agg_our_side)。
    /// 驱动:报告侧重、AI 助手立场、各 chip 不再"猜我方"。
    pub agg_our_side: Option<String>,

    /// 2026-06-13 加(migration 0025 · 工作流状态锁)
    /// 1 = 用户在卡片右上角手动选过 workflow_status → 全局抽不再用 LLM 值覆盖;
    /// 0 = 走自动推断。修「结案/手设状态被重新分析刷新掉」的 bug。
    pub workflow_status_locked: i64,

    /// 2026-07-04 加(migration 0039):最近一次全案分析使用的材料输入签名。
    pub analysis_input_signature: Option<String>,
    /// 1 = 当前材料集变更后尚未重新跑全案分析。
    pub analysis_stale: i64,
    /// 分析过期原因,如 source_files_changed / document_reextract_requested。
    pub analysis_stale_reason: Option<String>,
}

/// 仅取用户在详情页确认/纠正的我方立场(user_overrides_json.fields.agg_our_side)。空返回 None。
/// 单一来源:chat 快照 + 执行模块立场判断共用,避免两处各写一份 JSON 解析漂移。
pub fn user_override_our_side(user_overrides_json: Option<&str>) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(user_overrides_json?).ok()?;
    let s = v
        .get("fields")
        .and_then(|f| f.get("agg_our_side"))
        .and_then(|x| x.as_str())?
        .trim();
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// 我方代理立场:用户 override 优先,否则用 LLM 抽的 agg_our_side。空/未识别返回 None。
pub fn effective_our_side(
    agg_our_side: Option<&str>,
    user_overrides_json: Option<&str>,
) -> Option<String> {
    user_override_our_side(user_overrides_json).or_else(|| {
        agg_our_side
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    })
}

/// 创建新案件的最小参数。
#[derive(Debug, Clone)]
pub struct NewCase {
    pub name: String,
    pub case_type: String, // "诉讼" / "非诉"
    pub source_folder: String,
}

/// 插入新案件,返回新建的 Case。
///
/// 不做 upsert——如果 `source_folder` 已存在,会因为 UNIQUE 索引报错。
/// 想要 upsert 行为请用 [`upsert_case_for_folder`]。
pub async fn create_case(pool: &SqlitePool, new: NewCase) -> Result<Case, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO cases (id, name, case_type, source_folder) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(&new.name)
        .bind(&new.case_type)
        .bind(&new.source_folder)
        .execute(pool)
        .await?;

    get_case(pool, &id)
        .await?
        .ok_or_else(|| sqlx::Error::RowNotFound)
}

/// 如果 `source_folder` 已经入库过,返回现有 Case 并刷新 `updated_at` + `last_scanned_at`;
/// 否则按 `default_name` / `default_case_type` 新建一条。
///
/// 这是导入流程的标准入口:用户选个文件夹,不管是不是第一次都能正确处理。
pub async fn upsert_case_for_folder(
    pool: &SqlitePool,
    source_folder: &str,
    default_name: &str,
    default_case_type: &str,
) -> Result<Case, sqlx::Error> {
    if let Some(existing) = find_case_by_folder(pool, source_folder).await? {
        // 已存在 → 只刷新扫描时间
        sqlx::query(
            "UPDATE cases SET last_scanned_at = datetime('now'), updated_at = datetime('now') WHERE id = ?",
        )
        .bind(&existing.id)
        .execute(pool)
        .await?;
        return get_case(pool, &existing.id)
            .await?
            .ok_or(sqlx::Error::RowNotFound);
    }

    // 不存在 → 新建
    let case = create_case(
        pool,
        NewCase {
            name: default_name.to_string(),
            case_type: default_case_type.to_string(),
            source_folder: source_folder.to_string(),
        },
    )
    .await?;

    // 再设一下 last_scanned_at
    sqlx::query("UPDATE cases SET last_scanned_at = datetime('now') WHERE id = ?")
        .bind(&case.id)
        .execute(pool)
        .await?;

    get_case(pool, &case.id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

/// 按 id 取案件。
pub async fn get_case(pool: &SqlitePool, id: &str) -> Result<Option<Case>, sqlx::Error> {
    sqlx::query_as::<_, Case>("SELECT * FROM cases WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// 按 source_folder 取案件(用于"这个文件夹是否已经入库过")。
pub async fn find_case_by_folder(
    pool: &SqlitePool,
    source_folder: &str,
) -> Result<Option<Case>, sqlx::Error> {
    sqlx::query_as::<_, Case>("SELECT * FROM cases WHERE source_folder = ?")
        .bind(source_folder)
        .fetch_optional(pool)
        .await
}

/// 列出所有案件,按 `updated_at` 倒序(最近的在前)。
pub async fn list_cases(pool: &SqlitePool) -> Result<Vec<Case>, sqlx::Error> {
    sqlx::query_as::<_, Case>("SELECT * FROM cases ORDER BY updated_at DESC")
        .fetch_all(pool)
        .await
}

/// 仅当 `case_no` 当前为空/NULL 时才写入(案件资料包合并:只补空白、不覆盖目标方已有值)。
/// 返回受影响行数(0 = 目标已有非空案号,未动)。
pub async fn set_case_no_if_empty(
    pool: &SqlitePool,
    id: &str,
    case_no: &str,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE cases SET case_no = ?, updated_at = datetime('now') \
         WHERE id = ? AND (case_no IS NULL OR trim(case_no) = '')",
    )
    .bind(case_no)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 仅当 `case_summary` 当前为空/NULL 时才写入(同上,只补空白)。返回受影响行数。
pub async fn set_summary_if_empty(
    pool: &SqlitePool,
    id: &str,
    summary: &str,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "UPDATE cases SET case_summary = ?, updated_at = datetime('now') \
         WHERE id = ? AND (case_summary IS NULL OR trim(case_summary) = '')",
    )
    .bind(summary)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

/// 删除一个案件(级联删除所有关联表:documents/events/contacts/...)。
pub async fn delete_case(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    // migration 0029 的 court_filing_jobs 是当前唯一没有 ON DELETE 动作的案件子表。
    // 其余子表继续交给各自的 CASCADE / SET NULL 约束，避免手写表清单改变既有语义。
    sqlx::query("DELETE FROM court_filing_jobs WHERE case_id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM cases WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// 2026-05-24 e:更新案件的工作流状态(右上角状态 chip 的手工覆盖)。
///
/// `status = None` → 清空,前端走自动推断;
/// `status = Some("closed"|"intake"|"filing"|"awaiting_hearing"|"trial"|
///                 "appeal_window"|"appeal"|"execution")` → 用户手工覆盖,优先级最高
///
/// 不校验 status 字面值(由前端的枚举类型约束),DB 层只做透传。
///
/// 2026-06-13:同时维护 `workflow_status_locked` —— 用户手设(status=Some)→ 锁=1,
/// 全局抽不再用 LLM 值覆盖;设回自动(status=None)→ 锁=0,恢复自动推断。
/// 修「结案/手设状态被重新分析刷新掉」(胡彬律师反馈)。
pub async fn update_workflow_status(
    pool: &SqlitePool,
    id: &str,
    status: Option<&str>,
) -> Result<(), sqlx::Error> {
    let locked: i64 = if status.is_some() { 1 } else { 0 };
    sqlx::query(
        "UPDATE cases SET workflow_status = ?, workflow_status_locked = ?, \
         updated_at = datetime('now') WHERE id = ?",
    )
    .bind(status)
    .bind(locked)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// 2026-05-26 V0.1.13 · 写入案件 user_overrides JSON。
///
/// `json = None` → 清空所有用户改动(回到纯 LLM 抽取的视图);
/// `json = Some(...)` → 整段覆盖(前端 debounce 后整包提交)。
///
/// 后端不解析 / 不校验 JSON 结构,完全透传。结构定义见 migration 0016 注释。
pub async fn update_user_overrides(
    pool: &SqlitePool,
    id: &str,
    json: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE cases SET user_overrides_json = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(json)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

// ============================================================================
// 测试
// ============================================================================
