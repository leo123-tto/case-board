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

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::case_instances::NewInstance;
use crate::llm::global_extract::{
    build_corpus_with_char_budget, extract_combined, global_extract_input_char_budget,
    report_path_for_case, BudgetedCorpus, ConfirmedRepresentation, DocInput, GlobalExtractTable,
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
    pub warning: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct AnalysisInputPart {
    doc_id: String,
    filename: String,
    category: Option<String>,
    stage: Option<String>,
    text_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepresentationSnapshot {
    representation: Option<ConfirmedRepresentation>,
    user_overrides_json: Option<String>,
    agg_our_side: Option<String>,
}

#[derive(Debug, Clone)]
struct ReportGeneration {
    path: std::path::PathBuf,
    temp_path: std::path::PathBuf,
}

fn analysis_input_signature(parts: &[AnalysisInputPart]) -> String {
    let mut lines: Vec<String> = parts
        .iter()
        .map(|p| {
            format!(
                "{}\t{}\t{}\t{}\t{}",
                p.doc_id,
                p.filename,
                p.category.as_deref().unwrap_or(""),
                p.stage.as_deref().unwrap_or(""),
                p.text_hash
            )
        })
        .collect();
    lines.sort();
    crate::db::documents::stable_text_hash(&lines.join("\n"))
}

/// 跑一次案件全局抽。两次 LLM call **并发跑**(call A 表格 + call B 报告)。
pub async fn run_global_extract(
    pool: &SqlitePool,
    case_id: &str,
    llm_config: &LlmConfig,
) -> GlobalExtractReport {
    let start = std::time::Instant::now();

    // 1. 拿可读正文清单:done 文档 + LLM 字段失败但 OCR/文本已落盘的文档。
    type DocRow = (
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
    );
    let rows: Vec<DocRow> = match sqlx::query_as(
        "SELECT id, filename, category, stage, extracted_text_path, source_path, extracted_text_hash \
         FROM documents \
         WHERE case_id = ? AND deleted_at IS NULL AND extracted_text_path IS NOT NULL \
           AND (extraction_status = 'done' OR extraction_status = 'failed') \
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
                warning: None,
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
            warning: None,
            error: Some("无可分析正文,无法全局抽取".into()),
        };
    }

    // D3-1:检测语料是否为完整集的子集 —— 有未纳入正文的文档说明本次基于**不完整语料**抽取。
    // 数组字段(当事人/日期/费用)可能比完整抽取更短;COALESCE 只防"整列被空值抹除",
    // **防不了"变短覆盖"**(P1 残留:完整性 gate 待定)。这里落 dlog 让 partial-shrink 可观测,不再静默。
    let mut warning = None;
    if let Ok(not_done) = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM documents \
         WHERE case_id = ? AND deleted_at IS NULL \
           AND NOT (extracted_text_path IS NOT NULL \
             AND (extraction_status = 'done' OR extraction_status = 'failed'))",
    )
    .bind(case_id)
    .fetch_one(pool)
    .await
    {
        warning = corpus_incomplete_warning(not_done);
        if let Some(w) = warning.as_deref() {
            crate::dlog!(
                "[global_extract] case={} {} \
                 数组字段可能比完整抽取更短(D3-1 残留:仅防空覆盖,未防变短)",
                case_id,
                w
            );
        }
    }

    // 2. 读 MD 文件内容(本地 IO,blocking,但量小可接受)
    let mut docs: Vec<DocInput> = Vec::with_capacity(rows.len());
    let mut payment_sources: Vec<PaymentSourceDoc> = Vec::with_capacity(rows.len());
    let mut signature_parts: Vec<AnalysisInputPart> = Vec::with_capacity(rows.len());
    for (id, filename, category, stage, text_path, source_path, extracted_text_hash) in &rows {
        if crate::ingest::pipeline::is_archival_category(category.as_deref()) {
            continue;
        }
        let Some(p) = text_path else {
            crate::dlog!("[global_extract] {} 无 extracted_text_path,跳过", filename);
            continue;
        };
        match std::fs::read_to_string(p) {
            Ok(content) => {
                let text_hash = extracted_text_hash
                    .clone()
                    .unwrap_or_else(|| crate::db::documents::stable_text_hash(&content));
                docs.push(DocInput {
                    filename: filename.clone(),
                    category: category.clone(),
                    stage: stage.clone(),
                    text_md: content,
                });
                signature_parts.push(AnalysisInputPart {
                    doc_id: id.clone(),
                    filename: filename.clone(),
                    category: category.clone(),
                    stage: stage.clone(),
                    text_hash,
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
            warning,
            error: Some("MD 文件都读不到,无法全局抽取".into()),
        };
    }

    let docs_count = docs.len();
    let input_signature = analysis_input_signature(&signature_parts);
    let corpus_budget = global_extract_input_char_budget(llm_config);
    let budgeted_corpus = build_corpus_with_char_budget(&docs, corpus_budget);
    if let Some(w) = corpus_budget_warning(&budgeted_corpus, corpus_budget) {
        warning = append_warning(warning, w);
    }
    let corpus = budgeted_corpus.corpus;
    crate::dlog!(
        "[global_extract] case={} 拼了 {} 份 MD,{} chars(~{} tokens), 原始正文 {} chars, 纳入正文 {} chars, 表格摘要 {} 份, 裁剪文档 {} 份, 预算 {} chars",
        case_id,
        docs_count,
        corpus.len(),
        corpus.len() / 4,
        budgeted_corpus.original_text_chars,
        budgeted_corpus.included_text_chars,
        budgeted_corpus.spreadsheet_digest_docs,
        budgeted_corpus.truncated_docs,
        corpus_budget
    );

    // 2b. 读已锁定的代理范围。精确人工委托人优先；旧案件仅有粗立场时保持兼容。
    // 损坏的 representation 必须中断本次分析，绝不能静默扩大成整方代理。
    let representation_snapshot = match capture_representation_snapshot(pool, case_id).await {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return GlobalExtractReport {
                case_id: case_id.into(),
                docs_included: docs_count,
                table_ok: false,
                report_ok: false,
                report_path: None,
                elapsed_ms: start.elapsed().as_millis(),
                warning,
                error: Some(format!("案件精确委托人状态无效: {error}")),
            }
        }
    };

    // 3. 单次 LLM call 同时拿表格 + 报告(2026-05-24 i 合并)
    let combined = extract_combined(
        llm_config,
        &corpus,
        representation_snapshot.representation.as_ref(),
    )
    .await;

    let (table_ok, report_ok, report_path_str, err) = match combined {
        Ok(mut r) => {
            if let Some(confirmed) = representation_snapshot.representation.as_ref() {
                if let Err(error) = apply_confirmed_representation(&mut r.table, confirmed) {
                    return GlobalExtractReport {
                        case_id: case_id.into(),
                        docs_included: docs_count,
                        table_ok: false,
                        report_ok: false,
                        report_path: None,
                        elapsed_ms: start.elapsed().as_millis(),
                        warning,
                        error: Some(error),
                    };
                }
            }
            for validation_warning in &r.validation_warnings {
                warning = append_warning(warning, validation_warning.clone());
            }
            let mut persistence_errors = Vec::new();
            // 主画像先以开始时的代理范围做 CAS 写入；失败时报告和下游表均不得落盘。
            if let Err(error) = write_table_to_cases_guarded(
                pool,
                case_id,
                &r.table,
                None,
                &representation_snapshot,
            )
            .await
            {
                return GlobalExtractReport {
                    case_id: case_id.into(),
                    docs_included: docs_count,
                    table_ok: false,
                    report_ok: false,
                    report_path: None,
                    elapsed_ms: start.elapsed().as_millis(),
                    warning,
                    error: Some(error.to_string()),
                };
            }

            // 审级与还款是画像的下游表；代理范围变更时绝不能继续写入旧分析。
            let mut downstream_ok = true;
            if !representation_snapshot_is_current(pool, case_id, &representation_snapshot)
                .await
                .unwrap_or(false)
            {
                return representation_changed_report(case_id, docs_count, start, warning);
            }
            if let Err(e) = write_instances(pool, case_id, &r.table.instances).await {
                downstream_ok = false;
                crate::dlog!("[global_extract] 写 case_instances 失败:{}", e);
                warning = append_warning(
                    warning,
                    format!("案件画像已更新，但审级明细保存失败: {}", e),
                );
            }
            if let Err(e) =
                write_repayments(pool, case_id, &r.table.repayments, &payment_sources).await
            {
                downstream_ok = false;
                crate::dlog!("[global_extract] 写还款记录失败:{}", e);
                warning = append_warning(
                    warning,
                    format!("案件画像已更新，但 AI 识别还款记录保存失败: {}", e),
                );
            }

            let report_path = if downstream_ok {
                let generation = report_path_for_case(case_id)
                    .map(|path| path.parent().map(std::path::Path::to_path_buf))
                    .map_err(|error| error.to_string())
                    .and_then(|dir| dir.ok_or_else(|| "报告目录无效".to_string()))
                    .and_then(|dir| write_report_generation(&dir, case_id, &r.report_md));
                match generation {
                    Ok(generation) => match publish_report_generation_if_snapshot_current(
                        pool,
                        case_id,
                        &input_signature,
                        &representation_snapshot,
                        generation,
                    )
                    .await
                    {
                        Ok(path) => Some(path),
                        Err(error) => {
                            if error == REPRESENTATION_CHANGED_ERROR {
                                return representation_changed_report(
                                    case_id, docs_count, start, warning,
                                );
                            }
                            persistence_errors.push(format!("发布案件分析报告失败: {error}"));
                            None
                        }
                    },
                    Err(error) => {
                        persistence_errors.push(format!("写案件分析报告失败: {error}"));
                        None
                    }
                }
            } else {
                None
            };

            if !(downstream_ok && report_path.is_some())
                && representation_snapshot_is_current(pool, case_id, &representation_snapshot)
                    .await
                    .unwrap_or(false)
            {
                if let Err(e) = crate::db::ai_jobs::mark_case_analysis_stale(
                    pool,
                    case_id,
                    "analysis_persistence_partial",
                )
                .await
                {
                    persistence_errors.push(format!("写分析新鲜度标记失败: {e}"));
                }
            }

            let table_ok = true;
            let report_ok = report_path.is_some();
            let error = (!persistence_errors.is_empty()).then(|| persistence_errors.join("；"));
            (table_ok, report_ok, report_path, error)
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
        warning,
        error: err,
    }
}

fn corpus_incomplete_warning(not_done: i64) -> Option<String> {
    if not_done <= 0 {
        return None;
    }
    Some(format!(
        "有 {} 份文档未纳入本次语料,报告基于不完整材料生成,建议先处理失败/未完成材料后重新分析。",
        not_done
    ))
}

fn corpus_budget_warning(corpus: &BudgetedCorpus, budget: usize) -> Option<String> {
    if corpus.truncated_docs == 0 && corpus.spreadsheet_digest_docs == 0 {
        return None;
    }
    let mut details = Vec::new();
    if corpus.spreadsheet_digest_docs > 0 {
        details.push(format!(
            "已将 {} 份表格材料转为 Markdown 摘要",
            corpus.spreadsheet_digest_docs
        ));
    }
    if corpus.truncated_docs > 0 {
        details.push(format!(
            "已按 {} 字上下文预算裁剪 {} 份超长文档",
            budget, corpus.truncated_docs
        ));
    }
    Some(format!(
        "本案材料过长(原始正文约 {} 字),{}后分析;超长台账/扫描件可能只保留摘要、开头和结尾。",
        corpus.original_text_chars,
        details.join(";")
    ))
}

fn append_warning(existing: Option<String>, next: String) -> Option<String> {
    let next = next.trim();
    if next.is_empty() {
        return existing;
    }
    match existing {
        Some(prev) if !prev.trim().is_empty() => Some(format!("{} {}", prev.trim(), next)),
        _ => Some(next.to_string()),
    }
}

const REPRESENTATION_CHANGED_ERROR: &str = "律师已修改委托人，已丢弃本次分析，请重新分析";

fn representation_changed_report(
    case_id: &str,
    docs_included: usize,
    start: std::time::Instant,
    warning: Option<String>,
) -> GlobalExtractReport {
    GlobalExtractReport {
        case_id: case_id.into(),
        docs_included,
        table_ok: false,
        report_ok: false,
        report_path: None,
        elapsed_ms: start.elapsed().as_millis(),
        warning,
        error: Some(REPRESENTATION_CHANGED_ERROR.into()),
    }
}

/// 生成一份独立候选报告。它绝不触碰当前 `case_report_path`，是否发布由最终 CAS 决定。
fn write_report_generation(
    report_dir: &std::path::Path,
    case_id: &str,
    report_md: &str,
) -> Result<ReportGeneration, String> {
    let generation_id = Uuid::new_v4();
    let path = report_dir.join(format!("{case_id}-{generation_id}.md"));
    let temp_path = report_dir.join(format!(".{case_id}-{generation_id}.tmp"));
    write_report_generation_to_paths(&path, &temp_path, report_md)
}

/// 低层写入便于可靠覆盖 write/rename 失败时的清理路径测试。
fn write_report_generation_to_paths(
    path: &std::path::Path,
    temp_path: &std::path::Path,
    report_md: &str,
) -> Result<ReportGeneration, String> {
    if let Err(error) = std::fs::write(temp_path, report_md) {
        let _ = std::fs::remove_file(temp_path);
        return Err(error.to_string());
    }
    if let Err(error) = std::fs::rename(temp_path, path) {
        let _ = std::fs::remove_file(temp_path);
        return Err(error.to_string());
    }
    Ok(ReportGeneration {
        path: path.to_path_buf(),
        temp_path: temp_path.to_path_buf(),
    })
}

fn discard_report_generation(generation: &ReportGeneration) {
    let _ = std::fs::remove_file(&generation.temp_path);
    let _ = std::fs::remove_file(&generation.path);
}

/// 候选 generation 只有在最终 representation snapshot CAS 成功时才发布到数据库。
async fn publish_report_generation_if_snapshot_current(
    pool: &SqlitePool,
    case_id: &str,
    input_signature: &str,
    expected: &RepresentationSnapshot,
    generation: ReportGeneration,
) -> Result<String, String> {
    let report_path = generation.path.to_string_lossy().to_string();
    match mark_case_analysis_current_if_representation_current(
        pool,
        case_id,
        input_signature,
        Some(&report_path),
        expected,
    )
    .await
    {
        // 旧 path 可能来自历史固定文件或其他功能，不能据 filename 猜测归属后删除；受控的
        // generation 清理留待后续单独治理。本轮的核心是不覆盖旧路径，且失败必删新候选。
        Ok(true) => Ok(report_path),
        Ok(false) => {
            discard_report_generation(&generation);
            Err(REPRESENTATION_CHANGED_ERROR.into())
        }
        Err(error) => {
            discard_report_generation(&generation);
            Err(error.to_string())
        }
    }
}

/// 只有代理范围仍是本轮输入时才把分析标为 current，并同步报告路径。
async fn mark_case_analysis_current_if_representation_current(
    pool: &SqlitePool,
    case_id: &str,
    input_signature: &str,
    report_path: Option<&str>,
    expected: &RepresentationSnapshot,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE cases SET analysis_input_signature = ?, analysis_stale = 0, \
            analysis_stale_reason = NULL, case_report_path = COALESCE(?, case_report_path), \
            case_report_generated_at = CASE WHEN ? IS NULL THEN case_report_generated_at ELSE ? END, \
            updated_at = datetime('now') \
         WHERE id = ? AND user_overrides_json IS ? AND agg_our_side IS ?",
    )
    .bind(input_signature)
    .bind(report_path)
    .bind(report_path)
    .bind(chrono::Utc::now().to_rfc3339())
    .bind(case_id)
    .bind(&expected.user_overrides_json)
    .bind(&expected.agg_our_side)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
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

/// D9-1:`cases.workflow_status` 单一英文口径。LLM 输出的中文 11 档 → 前端 `StatusId`(英文)。
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

/// 捕获本轮分析所依据的代理范围及原始权威状态；后续写入必须以这份快照做 CAS。
async fn capture_representation_snapshot(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<RepresentationSnapshot, String> {
    let row: (Option<String>, Option<String>) =
        sqlx::query_as("SELECT user_overrides_json, agg_our_side FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| format!("读取案件代理范围失败: {error}"))?
            .ok_or_else(|| "案件不存在".to_string())?;
    crate::db::case_representation::effective_representation(row.0.as_deref(), row.1.as_deref())
        .map(|representation| RepresentationSnapshot {
            representation: representation.map(|representation| ConfirmedRepresentation {
                side: representation.side,
                parties: representation.parties,
            }),
            user_overrides_json: row.0,
            agg_our_side: row.1,
        })
        .map_err(|error| error.to_string())
}

async fn representation_snapshot_is_current(
    pool: &SqlitePool,
    case_id: &str,
    expected: &RepresentationSnapshot,
) -> Result<bool, String> {
    let current: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT user_overrides_json, agg_our_side FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(pool)
            .await
            .map_err(|error| format!("复核案件代理范围失败: {error}"))?;
    Ok(current.is_some_and(|current| {
        current.0 == expected.user_overrides_json && current.1 == expected.agg_our_side
    }))
}

/// 精确人工名单是事实上的利益冲突边界，不能信任重分析模型把同阵营全标成我方。
fn apply_confirmed_representation(
    table: &mut GlobalExtractTable,
    confirmed: &ConfirmedRepresentation,
) -> Result<(), String> {
    if confirmed.parties.is_empty() {
        table.our_side = Some(confirmed.side.clone());
        return Ok(());
    }

    let exact_parties = confirmed
        .parties
        .iter()
        .filter(|party| !party.name.trim().is_empty())
        .collect::<Vec<_>>();
    for exact in &exact_parties {
        let matched = table
            .party_contacts
            .iter()
            .filter(|party| {
                party
                    .name
                    .as_deref()
                    .zip(party.role.as_deref())
                    .is_some_and(|(name, role)| {
                        name.trim() == exact.name.trim() && role.trim() == exact.role.trim()
                    })
            })
            .count();
        if matched != 1 {
            return Err(format!(
                "精确委托人“{}”（{}）在本轮当事人联系人中匹配{}条，当前版本无法自动区分；已停止发布本轮案件画像和报告",
                exact.name, exact.role, matched
            ));
        }
    }
    table.our_side = Some(confirmed.side.clone());
    for party in &mut table.party_contacts {
        party.is_our_side = Some(
            party
                .name
                .as_deref()
                .zip(party.role.as_deref())
                .is_some_and(|(name, role)| {
                    exact_parties.iter().any(|exact| {
                        exact.name.trim() == name.trim() && exact.role.trim() == role.trim()
                    })
                }),
        );
    }

    let main_roles_by_name = main_party_roles_by_name(&table.party_contacts);
    for instance in &mut table.instances {
        for party in &mut instance.party_roles {
            party.is_our_side = Some(
                party
                    .name
                    .as_deref()
                    .and_then(|name| main_roles_by_name.get(name.trim()))
                    .filter(|roles| roles.len() == 1)
                    .is_some_and(|roles| {
                        exact_parties.iter().any(|exact| {
                            exact.name.trim() == party.name.as_deref().unwrap_or_default().trim()
                                && roles.contains(exact.role.trim())
                        })
                    }),
            );
        }
    }
    Ok(())
}

fn main_party_roles_by_name(
    contacts: &[crate::llm::global_extract::PartyContact],
) -> std::collections::HashMap<String, HashSet<String>> {
    let mut roles_by_name = std::collections::HashMap::new();
    for contact in contacts {
        let Some(name) = contact
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let Some(role) = contact
            .role
            .as_deref()
            .map(str::trim)
            .filter(|role| matches!(*role, "原告" | "被告" | "第三人"))
        else {
            continue;
        };
        roles_by_name
            .entry(name.to_string())
            .or_insert_with(HashSet::new)
            .insert(role.to_string());
    }
    roles_by_name
}

/// 用全案分析开始前捕获的代理范围写入 cases；快照不一致时 CAS 拒绝写入。
async fn write_table_to_cases_guarded(
    pool: &SqlitePool,
    case_id: &str,
    t: &GlobalExtractTable,
    report_path: Option<&str>,
    expected: &RepresentationSnapshot,
) -> Result<(), sqlx::Error> {
    let now = chrono::Utc::now().to_rfc3339();

    let (overrides_json, existing_our_side): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT user_overrides_json, agg_our_side FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(pool)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
    if overrides_json != expected.user_overrides_json || existing_our_side != expected.agg_our_side
    {
        return Err(sqlx::Error::Protocol(
            "律师已修改委托人，已丢弃本次分析，请重新分析".into(),
        ));
    }
    let mut corrected_table = t.clone();
    if let Some(confirmed) = expected.representation.as_ref() {
        apply_confirmed_representation(&mut corrected_table, confirmed)
            .map_err(sqlx::Error::Protocol)?;
    }
    let t = &corrected_table;

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
    let manual_our_side = crate::db::cases::user_override_our_side(overrides_json.as_deref());
    let has_manual_our_side = manual_our_side.is_some();
    let manual_or_llm_our_side = manual_our_side.as_deref().or(our_side_opt);

    // D9-1:LLM 输出中文状态 → 前端/DB 统一英文 StatusId(单一口径);不在表内则 None(保留 DB 现值,
    // 用户可能手工标过)。修复"LLM 写中文、前端只认英文 → 推断状态在看板/执行 tab 落不了地"。
    let workflow_status_to_set = t
        .workflow_status
        .as_deref()
        .and_then(workflow_status_zh_to_en);

    let result = sqlx::query(
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
            agg_our_side = CASE WHEN ? \
                THEN COALESCE(?, agg_our_side) \
                ELSE COALESCE(agg_our_side, ?) END, \
            case_summary = COALESCE(?, case_summary), \
            case_report_path = COALESCE(?, case_report_path), \
            case_report_generated_at = ?, \
            workflow_status = CASE WHEN workflow_status_locked = 1 \
                THEN workflow_status ELSE COALESCE(?, workflow_status) END, \
            agg_computed_at = ? \
         WHERE id = ? AND user_overrides_json IS ? AND agg_our_side IS ?",
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
    .bind(has_manual_our_side)
    .bind(manual_or_llm_our_side)
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
    .bind(&expected.user_overrides_json)
    .bind(&expected.agg_our_side)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Err(sqlx::Error::Protocol(
            "律师已修改委托人，已丢弃本次分析，请重新分析".into(),
        ));
    }

    Ok(())
}
