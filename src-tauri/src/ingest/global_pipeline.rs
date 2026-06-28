//! 案件全局抽取的编排层(2026-05-24 h)。
//!
//! 输入:case_id
//! 流程:
//!   1. 拉所有 done 文档 + 各自 extracted_text_path
//!   2. 读 MD 文件内容
//!   3. 拼 corpus + 两次并发 LLM 调用(call A 表格 / call B 报告)
//!   4. 写 cases.agg_* 全套 + case_summary + case_report_path + case_report_generated_at
//!   5. 报告 MD 落盘到 ~/Library/.../reports/<case_id>.md
//!
//! 替代了 `db/aggregator.rs::aggregate_case_facts`,**不再做规则去污**,全部交给 LLM。

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::db::case_instances::NewInstance;
use crate::llm::global_extract::{
    build_corpus, extract_combined, report_path_for_case, DocInput, GlobalExtractTable,
    InstanceExtract, RepaymentExtract,
};
use crate::llm::LlmConfig;

#[derive(Debug, Clone)]
struct PaymentSourceDoc {
    id: String,
    filename: String,
    source_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalExtractReport {
    pub case_id: String,
    pub docs_included: usize,
    pub table_ok: bool,
    pub report_ok: bool,
    pub report_path: Option<String>,
    pub elapsed_ms: u128,
    pub error: Option<String>,
}

/// 批量重抽所有案件后的汇报(给前端 Toast 用)。
///
/// 2026-05-24 h:从 `db::aggregator::ReaggregateReport` 搬过来,接口保持兼容
/// (前端 `reaggregateAllCases` 仍能用),但底层从规则聚合换成 LLM 全局抽。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReaggregateReport {
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    /// (case_id, 错误消息) 列表
    pub failures: Vec<(String, String)>,
}

/// 跑一次案件全局抽。两次 LLM call **并发跑**(call A 表格 + call B 报告)。
pub async fn run_global_extract(
    pool: &SqlitePool,
    case_id: &str,
    llm_config: &LlmConfig,
) -> GlobalExtractReport {
    let start = std::time::Instant::now();

    // 1. 拿 done 文档清单 + extracted_text_path
    type DocRow = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    );
    let rows: Vec<DocRow> = match sqlx::query_as(
        "SELECT id, filename, category, stage, extracted_text_path, source_path \
         FROM documents \
         WHERE case_id = ? AND deleted_at IS NULL AND extraction_status = 'done' \
         ORDER BY filename",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(e) => {
            return GlobalExtractReport {
                case_id: case_id.into(),
                docs_included: 0,
                table_ok: false,
                report_ok: false,
                report_path: None,
                elapsed_ms: start.elapsed().as_millis(),
                error: Some(format!("查文档列表失败:{}", e)),
            }
        }
    };

    if rows.is_empty() {
        return GlobalExtractReport {
            case_id: case_id.into(),
            docs_included: 0,
            table_ok: false,
            report_ok: false,
            report_path: None,
            elapsed_ms: start.elapsed().as_millis(),
            error: Some("无已 done 文档,无法全局抽取".into()),
        };
    }

    // D3-1:检测语料是否为完整集的子集 —— 有未 done 的文档说明本次基于**不完整语料**抽取。
    // 数组字段(当事人/日期/费用)可能比完整抽取更短;COALESCE 只防"整列被空值抹除",
    // **防不了"变短覆盖"**(P1 残留:完整性 gate 待定)。这里落 dlog 让 partial-shrink 可观测,不再静默。
    if let Ok(not_done) = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM documents \
         WHERE case_id = ? AND deleted_at IS NULL AND extraction_status != 'done'",
    )
    .bind(case_id)
    .fetch_one(pool)
    .await
    {
        if not_done > 0 {
            crate::dlog!(
                "[global_extract] case={} 有 {} 份文档未 done → 基于不完整语料抽取,\
                 数组字段可能比完整抽取更短(D3-1 残留:仅防空覆盖,未防变短)",
                case_id,
                not_done
            );
        }
    }

    // 2. 读 MD 文件内容(本地 IO,blocking,但量小可接受)
    let mut docs: Vec<DocInput> = Vec::with_capacity(rows.len());
    let mut payment_sources: Vec<PaymentSourceDoc> = Vec::with_capacity(rows.len());
    for (id, filename, category, stage, text_path, source_path) in &rows {
        if crate::ingest::pipeline::is_archival_category(category.as_deref()) {
            continue;
        }
        let Some(p) = text_path else {
            crate::dlog!("[global_extract] {} 无 extracted_text_path,跳过", filename);
            continue;
        };
        match std::fs::read_to_string(p) {
            Ok(content) => {
                docs.push(DocInput {
                    filename: filename.clone(),
                    category: category.clone(),
                    stage: stage.clone(),
                    text_md: content,
                });
                payment_sources.push(PaymentSourceDoc {
                    id: id.clone(),
                    filename: filename.clone(),
                    source_path: source_path.clone(),
                });
            }
            Err(e) => crate::dlog!("[global_extract] 读 {} 失败:{}", p, e),
        }
    }

    if docs.is_empty() {
        return GlobalExtractReport {
            case_id: case_id.into(),
            docs_included: 0,
            table_ok: false,
            report_ok: false,
            report_path: None,
            elapsed_ms: start.elapsed().as_millis(),
            error: Some("MD 文件都读不到,无法全局抽取".into()),
        };
    }

    let docs_count = docs.len();
    let corpus = build_corpus(&docs);
    crate::dlog!(
        "[global_extract] case={} 拼了 {} 份 MD,{} chars(~{} tokens)",
        case_id,
        docs_count,
        corpus.len(),
        corpus.len() / 4
    );

    // 2b. 读律师已确认的「我方代理立场」(详情页改的走 user_overrides_json.fields.agg_our_side)。
    // 有则回喂当 LLM 输入,保证用户纠正立场后报告/画像按正确立场重写(advisor 命门②:立场双向)。
    let confirmed_our_side = read_confirmed_our_side(pool, case_id).await;

    // 3. 单次 LLM call 同时拿表格 + 报告(2026-05-24 i 合并)
    let combined = extract_combined(llm_config, &corpus, confirmed_our_side.as_deref()).await;

    let (table_ok, report_ok, report_path_str, err) = match combined {
        Ok(r) => {
            // 报告 MD 落盘
            let report_path = match report_path_for_case(case_id) {
                Ok(p) => match std::fs::write(&p, &r.report_md) {
                    Ok(_) => Some(p.to_string_lossy().to_string()),
                    Err(e) => {
                        crate::dlog!("[global_extract] 写报告 MD 失败:{}", e);
                        None
                    }
                },
                Err(e) => {
                    crate::dlog!("[global_extract] 算报告路径失败:{}", e);
                    None
                }
            };
            // 写 cases 表
            if let Err(e) =
                write_table_to_cases(pool, case_id, &r.table, report_path.as_deref()).await
            {
                crate::dlog!("[global_extract] 写 cases 失败:{}", e);
            }
            // 2026-06-11 审级模型:instances 落库 + 当前审级快照回写 agg_*
            if let Err(e) = write_instances(pool, case_id, &r.table.instances).await {
                crate::dlog!("[global_extract] 写 case_instances 失败:{}", e);
            }
            // 还款自动入账(幂等,标 [AI识别])
            if let Err(e) =
                write_repayments(pool, case_id, &r.table.repayments, &payment_sources).await
            {
                crate::dlog!("[global_extract] 写还款记录失败:{}", e);
            }
            (true, report_path.is_some(), report_path, None)
        }
        Err(e) => {
            crate::dlog!("[global_extract] LLM 调用失败:{}", e);
            (false, false, None, Some(e.to_string()))
        }
    };

    GlobalExtractReport {
        case_id: case_id.into(),
        docs_included: docs_count,
        table_ok,
        report_ok,
        report_path: report_path_str,
        elapsed_ms: start.elapsed().as_millis(),
        error: err,
    }
}

/// 对所有案件依次跑一遍全局抽。**串行**(每个案件单 LLM call 已经够慢),
/// 失败不阻断后续案件,失败列表通过 ReaggregateReport.failures 返回。
pub async fn rerun_all_cases(
    pool: &SqlitePool,
    llm_config: &LlmConfig,
) -> Result<ReaggregateReport, sqlx::Error> {
    let ids: Vec<(String,)> = sqlx::query_as("SELECT id FROM cases")
        .fetch_all(pool)
        .await?;
    let total = ids.len();
    let mut succeeded = 0usize;
    let mut failures: Vec<(String, String)> = Vec::new();
    for (id,) in ids {
        let r = run_global_extract(pool, &id, llm_config).await;
        if r.table_ok {
            succeeded += 1;
        } else {
            failures.push((id, r.error.unwrap_or_else(|| "table 抽取失败".into())));
        }
    }
    Ok(ReaggregateReport {
        total,
        succeeded,
        failed: failures.len(),
        failures,
    })
}

/// 2026-06-11 审级模型:LLM instances → case_instances 表 + 当前审级快照回写 cases.agg_*。
/// 空列表 = LLM 没识别出审级 → 不动现有行(与 D3-1 防空覆盖同哲学,user 行永远保留)。
async fn write_instances(
    pool: &SqlitePool,
    case_id: &str,
    items: &[InstanceExtract],
) -> Result<(), sqlx::Error> {
    let rows: Vec<NewInstance> = items
        .iter()
        .filter_map(|it| {
            let level = it.level.as_deref()?.trim().to_string();
            let seq = level_seq(&level)?;
            Some(NewInstance {
                level,
                seq,
                case_no: it.case_no.clone(),
                authority: it.authority.clone(),
                authority_type: it.authority_type.clone(),
                handlers: non_empty_json(&it.handlers),
                party_roles: non_empty_json(&it.party_roles),
                filed_at: it.filed_at.clone(),
                result: it.result.clone(),
                note: it.note.clone(),
            })
        })
        .collect();
    if rows.is_empty() {
        return Ok(());
    }
    let list = crate::db::case_instances::replace_llm_instances(pool, case_id, &rows).await?;
    // 当前审级(seq 最大)快照回写首页卡读的 agg_* —— 识别到二审,首页就显二审
    if let Some(cur) = list.first() {
        sqlx::query(
            "UPDATE cases SET \
                agg_case_no = COALESCE(?, agg_case_no), \
                agg_court = COALESCE(?, agg_court), \
                agg_court_type = COALESCE(?, agg_court_type) \
             WHERE id = ?",
        )
        .bind(&cur.case_no)
        .bind(&cur.authority)
        .bind(&cur.authority_type)
        .bind(case_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// level → 约定排序号(仲裁1 / 一审2 / 二审3 / 再审4);未知 level 不入库。
fn level_seq(level: &str) -> Option<i64> {
    match level {
        "仲裁" => Some(1),
        "一审" => Some(2),
        "二审" => Some(3),
        "再审" => Some(4),
        _ => None,
    }
}

/// 2026-06-11:LLM 识别的还款幂等落 case_payments(标 [AI识别],识别错用户可删)。
/// (case_id, amount, paid_at) 已存在则跳过 —— 防重抽重复入账;无金额或无日期跳过
/// (法律数据不编造日期,摘要文本里仍可见,律师手补)。
async fn write_repayments(
    pool: &SqlitePool,
    case_id: &str,
    items: &[RepaymentExtract],
    sources: &[PaymentSourceDoc],
) -> Result<(), sqlx::Error> {
    for it in items {
        let Some(amount) = it.amount else { continue };
        if amount <= 0.0 {
            continue;
        }
        let Some(paid_at) = it.paid_at.as_deref().filter(|s| !s.trim().is_empty()) else {
            crate::dlog!(
                "[global_extract] 还款 {} 元无日期,跳过自动入账(摘要里仍可见)",
                amount
            );
            continue;
        };
        let source = resolve_payment_source(it.source_filename.as_deref(), sources);
        let existing: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT id, source_document_id FROM case_payments \
             WHERE case_id = ? AND amount = ? AND paid_at = ? \
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(case_id)
        .bind(amount)
        .bind(paid_at)
        .fetch_optional(pool)
        .await?;
        if let Some((id, existing_source_document_id)) = existing {
            if existing_source_document_id.is_none() {
                if let Some(src) = source {
                    sqlx::query(
                        "UPDATE case_payments \
                         SET source_document_id = ?, source_path = ?, source_filename = ? \
                         WHERE id = ?",
                    )
                    .bind(&src.id)
                    .bind(&src.source_path)
                    .bind(&src.filename)
                    .bind(&id)
                    .execute(pool)
                    .await?;
                }
            }
            continue;
        }
        let mut note = String::from("[AI识别]");
        if let Some(p) = it.payer.as_deref().filter(|s| !s.trim().is_empty()) {
            note.push(' ');
            note.push_str(p);
        }
        if let Some(n) = it.note.as_deref().filter(|s| !s.trim().is_empty()) {
            note.push_str(" · ");
            note.push_str(n);
        }
        crate::db::payments::add(
            pool,
            crate::db::payments::NewPayment {
                case_id: case_id.to_string(),
                amount,
                paid_at: paid_at.to_string(),
                note: Some(note),
                source_document_id: source.map(|s| s.id.clone()),
                source_path: source.map(|s| s.source_path.clone()),
                source_filename: source.map(|s| s.filename.clone()),
            },
        )
        .await?;
    }
    Ok(())
}

fn resolve_payment_source<'a>(
    source_filename: Option<&str>,
    sources: &'a [PaymentSourceDoc],
) -> Option<&'a PaymentSourceDoc> {
    let filename = source_filename?.trim();
    if filename.is_empty() {
        return None;
    }
    if let Some(exact) = sources.iter().find(|s| s.filename == filename) {
        return Some(exact);
    }
    let matches: Vec<&PaymentSourceDoc> = sources
        .iter()
        .filter(|s| s.filename.contains(filename) || filename.contains(&s.filename))
        .collect();
    if matches.len() == 1 {
        matches.into_iter().next()
    } else {
        None
    }
}

/// D3-1:空集合 → None(配合 SQL COALESCE 跳过覆盖),非空才序列化为 JSON。
fn non_empty_json<T: serde::Serialize>(v: &[T]) -> Option<String> {
    if v.is_empty() {
        None
    } else {
        Some(serde_json::to_string(v).unwrap_or_else(|_| "[]".into()))
    }
}

/// D9-1:`cases.workflow_status` 单一英文口径。LLM 输出的中文 9 档 → 前端 `StatusId`(英文)。
/// 不在表内 → None(写库时 COALESCE 保留 DB 现值)。**与前端 `inferStatus.ts::StatusId` 严格对齐**。
pub fn workflow_status_zh_to_en(zh: &str) -> Option<&'static str> {
    match zh.trim() {
        "接案" => Some("intake"),
        "立案中" => Some("filing"),
        "仲裁中" => Some("arbitration"),
        "待开庭" => Some("awaiting_hearing"),
        "审理中" => Some("trial"),
        "已调解" => Some("mediated"),
        "上诉期" => Some("appeal_window"),
        "二审中" => Some("appeal"),
        "再审中" => Some("retrial"),
        "执行中" => Some("execution"),
        "已结案" => Some("closed"),
        _ => None,
    }
}

/// D9-1 反向:英文 `StatusId` → 中文 label。给 chat context 喂 LLM 时还原可读中文用。
/// 未知值原样返回(兼容历史脏数据)。
pub fn workflow_status_en_to_zh(en: &str) -> &str {
    match en.trim() {
        "intake" => "接案",
        "filing" => "立案中",
        "arbitration" => "仲裁中",
        "awaiting_hearing" => "待开庭",
        "trial" => "审理中",
        "mediated" => "已调解",
        "appeal_window" => "上诉期",
        "appeal" => "二审中",
        "retrial" => "再审中",
        "execution" => "执行中",
        "closed" => "已结案",
        other => other,
    }
}

/// 读律师在详情页确认/纠正过的「我方代理立场」(user_overrides_json.fields.agg_our_side)。
/// 返回 None = 用户没改过 → 让 LLM 自行推断;Some = 以用户值为准回喂 LLM。
async fn read_confirmed_our_side(pool: &SqlitePool, case_id: &str) -> Option<String> {
    // 列可空 → query_scalar 的列类型是 Option<String>,fetch_optional 再裹一层 → 两次 flatten。
    let json: String = sqlx::query_scalar::<_, Option<String>>(
        "SELECT user_overrides_json FROM cases WHERE id = ?",
    )
    .bind(case_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .flatten()?;
    let parsed: serde_json::Value = serde_json::from_str(&json).ok()?;
    let v = parsed
        .get("fields")
        .and_then(|f| f.get("agg_our_side"))
        .and_then(|x| x.as_str())?;
    let v = v.trim();
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

/// 把 LLM 抽出来的 GlobalExtractTable 写到 cases 表里。
async fn write_table_to_cases(
    pool: &SqlitePool,
    case_id: &str,
    t: &GlobalExtractTable,
    report_path: Option<&str>,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();

    // D3-1:数组/文本 agg_* 字段空值时返回 None → 配合下方 SQL 的 COALESCE 跳过覆盖,
    // 防"重抽期间个别文档失败、语料变子集"用更小结果把已抽到的当事人/日期/费用静默抹掉。
    let plaintiffs_json = non_empty_json(&t.plaintiffs);
    let defendants_json = non_empty_json(&t.defendants);
    let third_json = non_empty_json(&t.third_parties);
    let judges_json = non_empty_json(&t.judges);
    let party_contacts_json = non_empty_json(&t.party_contacts);
    let court_contacts_json = non_empty_json(&t.court_contacts);
    let key_dates_json = non_empty_json(&t.key_dates);
    let fees_json = non_empty_json(&t.fees);
    let resolution_opt = t.resolution.as_deref().filter(|s| !s.trim().is_empty());
    let status_text_opt = t.status_text.as_deref().filter(|s| !s.trim().is_empty());
    let summary_opt = t.summary.as_deref().filter(|s| !s.trim().is_empty());
    let our_side_opt = t.our_side.as_deref().filter(|s| !s.trim().is_empty());

    // D9-1:LLM 输出中文状态 → 前端/DB 统一英文 StatusId(单一口径);不在表内则 None(保留 DB 现值,
    // 用户可能手工标过)。修复"LLM 写中文、前端只认英文 → 推断状态在看板/执行 tab 落不了地"。
    let workflow_status_to_set = t
        .workflow_status
        .as_deref()
        .and_then(workflow_status_zh_to_en);

    sqlx::query(
        "UPDATE cases SET \
            agg_case_no = COALESCE(?, agg_case_no), \
            agg_court = COALESCE(?, agg_court), \
            agg_cause = COALESCE(?, agg_cause), \
            agg_filed_at = COALESCE(?, agg_filed_at), \
            agg_claim_amount = COALESCE(?, agg_claim_amount), \
            agg_plaintiffs = COALESCE(?, agg_plaintiffs), \
            agg_defendants = COALESCE(?, agg_defendants), \
            agg_third_parties = COALESCE(?, agg_third_parties), \
            agg_judges = COALESCE(?, agg_judges), \
            agg_party_contacts = COALESCE(?, agg_party_contacts), \
            agg_court_contacts = COALESCE(?, agg_court_contacts), \
            agg_key_dates = COALESCE(?, agg_key_dates), \
            agg_fees = COALESCE(?, agg_fees), \
            agg_resolution = COALESCE(?, agg_resolution), \
            agg_status_text = COALESCE(?, agg_status_text), \
            agg_our_side = COALESCE(?, agg_our_side), \
            case_summary = COALESCE(?, case_summary), \
            case_report_path = COALESCE(?, case_report_path), \
            case_report_generated_at = ?, \
            workflow_status = CASE WHEN workflow_status_locked = 1 \
                THEN workflow_status ELSE COALESCE(?, workflow_status) END, \
            agg_computed_at = ? \
         WHERE id = ?",
    )
    .bind(&t.case_no)
    .bind(&t.court)
    .bind(&t.cause)
    .bind(&t.filed_at)
    .bind(t.claim_amount)
    .bind(&plaintiffs_json)
    .bind(&defendants_json)
    .bind(&third_json)
    .bind(&judges_json)
    .bind(&party_contacts_json)
    .bind(&court_contacts_json)
    .bind(&key_dates_json)
    .bind(&fees_json)
    .bind(resolution_opt)
    .bind(status_text_opt)
    .bind(our_side_opt)
    .bind(summary_opt)
    .bind(report_path)
    .bind(if report_path.is_some() {
        Some(now.clone())
    } else {
        None
    })
    .bind(workflow_status_to_set)
    .bind(&now)
    .bind(case_id)
    .execute(pool)
    .await?;

    Ok(())
}
