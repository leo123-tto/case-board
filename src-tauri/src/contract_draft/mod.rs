//! 合同起草模块(2026-06-18 · 非诉 tab「合同起草」B1)。
//!
//! 数据流(对应三观四步法):
//!   步骤 1-3 `plan_contract_draft`:口语化需求 → 三观分析判类型 → 结构大纲 + 引导式采集清单 + 关键追问。
//!   步骤 4   `generate_contract_draft`:据已采集信息按三点一线法生成完整合同草案(markdown)+ 关键条款 + 风险。
//!   导出     `export_contract_draft_docx`:复用 `docx_filing` 原生 OOXML 引擎出 Word(黑体标题 / 仿宋正文)。
//!
//! **Clean-room + 致谢**:方法论借鉴 `pa1nrui1/legal-skills`(MIT,作者「小潘律师」),prompt/schema/引擎自建,
//! 零照搬其参考 md。致谢落在非诉「合同起草」前端 UI。
//!
//! 纯工具形态:不落库、不依赖 pool(合同未必属于某个案件)。失败透传真错(坑 #8)。
//! 多轮修订 / 版本管理 / 偏好学习见后续 B2 / B3(本文件只管 B1 起草核心)。

pub mod generate;

use generate::{ContractDraftPlan, ContractDraftResult, ContractReviseResult, DraftStance};

use crate::db::contract_drafts as drafts_db;
use crate::db::contract_drafts::{ContractDraft, ContractDraftVersion};
use crate::db::contract_preferences as prefs_db;
use crate::db::contract_preferences::ContractPreference;
use serde::Serialize;
use sqlx::SqlitePool;
use std::path::Path;

const CONTEXT_FILE_MAX_BYTES: u64 = 50 * 1024 * 1024;
const CONTEXT_FILE_MAX_CHARS: usize = 80_000;

#[derive(Debug, Serialize)]
pub struct ContractDraftContextFile {
    pub filename: String,
    pub path: String,
    pub text: String,
    pub char_count: usize,
    pub truncated: bool,
}

fn is_supported_context_file(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    [
        ".pdf",
        ".md",
        ".markdown",
        ".txt",
        ".docx",
        ".doc",
        ".rtf",
        ".odt",
    ]
    .iter()
    .any(|ext| lower.ends_with(ext))
}

/// 读取合同起草附件上下文。只做本地低成本文本抽取;扫描 PDF 不触发云端 OCR。
#[tauri::command]
pub async fn extract_contract_draft_context_file(
    path: String,
) -> Result<ContractDraftContextFile, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("文件不存在:{}", path));
    }
    if !p.is_file() {
        return Err(format!("不是文件:{}", path));
    }
    let filename = p
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("附件")
        .to_string();
    if !is_supported_context_file(&filename) {
        return Err("仅支持 PDF、Markdown、TXT、Word(.docx/.doc/.rtf/.odt) 文件".to_string());
    }
    let size = std::fs::metadata(p)
        .map(|m| m.len())
        .map_err(|e| format!("读不到文件元信息:{}", e))?;
    if size > CONTEXT_FILE_MAX_BYTES {
        return Err(format!(
            "文件太大({:.1} MB),超过 {} MB 上限",
            size as f64 / 1024.0 / 1024.0,
            CONTEXT_FILE_MAX_BYTES / 1024 / 1024
        ));
    }

    let text = crate::ingest::extractor::extract_text_only_cheap(p, &filename)
        .await?
        .ok_or_else(|| {
            "这份文件没有可直接读取的文字。若是扫描版 PDF,请先 OCR/另存为可复制文字后再导入。"
                .to_string()
        })?;
    let cleaned = text.trim();
    if cleaned.chars().count() < 10 {
        return Err("文件可读取文字太少,无法作为起草上下文。".to_string());
    }
    let char_count = cleaned.chars().count();
    let truncated = char_count > CONTEXT_FILE_MAX_CHARS;
    let text = if truncated {
        cleaned.chars().take(CONTEXT_FILE_MAX_CHARS).collect()
    } else {
        cleaned.to_string()
    };

    Ok(ContractDraftContextFile {
        filename,
        path,
        text,
        char_count,
        truncated,
    })
}

/// 步骤 1-3:起草前规划(类型判定 + 结构大纲 + 引导式信息采集清单)。
#[tauri::command]
pub async fn plan_contract_draft(
    requirement: String,
    stance: String,
    contract_type_hint: String,
) -> Result<ContractDraftPlan, String> {
    let settings = crate::settings::read_settings().unwrap_or_default();
    let config = crate::llm::LlmConfig::from_settings(&settings);
    let st = DraftStance::from_label(&stance);
    generate::plan_contract(&config, &requirement, st, &contract_type_hint)
        .await
        .map_err(|e| format!("起草规划失败:{}", e))
}

/// 步骤 4:据已采集信息生成完整合同草案。`collected_info` 为用户对采集清单/追问的回答汇总(可空)。
#[tauri::command]
pub async fn generate_contract_draft(
    requirement: String,
    stance: String,
    contract_type_hint: String,
    collected_info: String,
) -> Result<ContractDraftResult, String> {
    let settings = crate::settings::read_settings().unwrap_or_default();
    let config = crate::llm::LlmConfig::from_settings(&settings);
    let st = DraftStance::from_label(&stance);
    generate::generate_contract(
        &config,
        &requirement,
        st,
        &contract_type_hint,
        &collected_info,
    )
    .await
    .map_err(|e| format!("合同起草失败:{}", e))
}

/// 导出合同草案为 Word。前端把 `generate_contract_draft` 拿到的 `draft_md` + `contract_name` 回传。
#[tauri::command]
pub async fn export_contract_draft_docx(
    draft_md: String,
    contract_name: String,
    save_path: String,
) -> Result<String, String> {
    let (title, body) = split_title_body(&contract_name, &draft_md);
    let bytes = crate::docx_filing::build_filing_docx_bytes(&title, &body)?;
    std::fs::write(&save_path, &bytes).map_err(|e| format!("写合同草案 docx 失败:{}", e))?;
    Ok(save_path)
}

/// 多轮修订(B2):据修订要求把现有合同改成新版(case-less LLM,不落库;落库走 add_contract_draft_version)。
#[tauri::command]
pub async fn revise_contract_draft(
    current_md: String,
    feedback: String,
    stance: String,
) -> Result<ContractReviseResult, String> {
    let settings = crate::settings::read_settings().unwrap_or_default();
    let config = crate::llm::LlmConfig::from_settings(&settings);
    let st = DraftStance::from_label(&stance);
    generate::revise_contract(&config, &current_md, &feedback, st)
        .await
        .map_err(|e| format!("合同修订失败:{}", e))
}

// ── B2 版本管理:落库 CRUD(挂 draft_id 扛刷新)──────────────────────────────

/// 保存一份合同草案(同时落第 1 版)。返回草案行。
#[tauri::command]
pub async fn save_contract_draft(
    pool: tauri::State<'_, SqlitePool>,
    contract_name: String,
    contract_type: String,
    stance: String,
    requirement: String,
    draft_md: String,
) -> Result<ContractDraft, String> {
    drafts_db::create_draft(
        pool.inner(),
        &contract_name,
        &contract_type,
        &stance,
        &requirement,
        &draft_md,
    )
    .await
    .map_err(|e| format!("保存合同草案失败:{}", e))
}

/// 全部草案列表(最近更新在前)。
#[tauri::command]
pub async fn list_contract_drafts(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<Vec<ContractDraft>, String> {
    drafts_db::list_drafts(pool.inner())
        .await
        .map_err(|e| format!("读取合同草案列表失败:{}", e))
}

/// 某草案的全部版本(版本号升序)。
#[tauri::command]
pub async fn list_contract_draft_versions(
    pool: tauri::State<'_, SqlitePool>,
    draft_id: String,
) -> Result<Vec<ContractDraftVersion>, String> {
    drafts_db::list_versions(pool.inner(), &draft_id)
        .await
        .map_err(|e| format!("读取版本列表失败:{}", e))
}

/// 追加一版(多轮修订落库)。返回新版本行。
#[tauri::command]
pub async fn add_contract_draft_version(
    pool: tauri::State<'_, SqlitePool>,
    draft_id: String,
    source: String,
    based_on_version: Option<i64>,
    purpose: String,
    draft_md: String,
    change_summary: String,
) -> Result<ContractDraftVersion, String> {
    drafts_db::add_version(
        pool.inner(),
        &draft_id,
        &source,
        based_on_version,
        &purpose,
        &draft_md,
        &change_summary,
    )
    .await
    .map_err(|e| format!("保存新版本失败:{}", e))
}

/// 标记最终版(用户明确确认才调)。
#[tauri::command]
pub async fn mark_contract_draft_final(
    pool: tauri::State<'_, SqlitePool>,
    draft_id: String,
    version_id: String,
) -> Result<(), String> {
    drafts_db::mark_final(pool.inner(), &draft_id, &version_id)
        .await
        .map_err(|e| format!("标记最终版失败:{}", e))
}

/// 删除草案及其全部版本。
#[tauri::command]
pub async fn delete_contract_draft(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
) -> Result<u64, String> {
    drafts_db::delete_draft(pool.inner(), &id)
        .await
        .map_err(|e| format!("删除合同草案失败:{}", e))
}

// ── B3 起草偏好库:用户确认的条款偏好,起草/修订时按类型注入(仅辅助取舍,不降强制性规范风险)──

/// 新增一条起草偏好。`contract_type` 空 = 通用。
#[tauri::command]
pub async fn add_contract_preference(
    pool: tauri::State<'_, SqlitePool>,
    contract_type: String,
    topic: String,
    preference: String,
) -> Result<ContractPreference, String> {
    if preference.trim().is_empty() {
        return Err("偏好内容不能为空".to_string());
    }
    prefs_db::add(pool.inner(), &contract_type, &topic, &preference, "user")
        .await
        .map_err(|e| format!("保存起草偏好失败:{}", e))
}

/// 全部起草偏好(前端按合同类型筛选展示/注入)。
#[tauri::command]
pub async fn list_contract_preferences(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<Vec<ContractPreference>, String> {
    prefs_db::list(pool.inner())
        .await
        .map_err(|e| format!("读取起草偏好失败:{}", e))
}

/// 删除一条起草偏好。
#[tauri::command]
pub async fn delete_contract_preference(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
) -> Result<u64, String> {
    prefs_db::delete(pool.inner(), &id)
        .await
        .map_err(|e| format!("删除起草偏好失败:{}", e))
}

/// 把 draft_md 顶部的 H1 标题剥出来,避免和 `build_filing_docx_bytes` 的独立标题重复。
/// `contract_name` 非空时优先用它当标题;否则用剥出来的 H1;再兜底「合同」。
fn split_title_body(contract_name: &str, draft_md: &str) -> (String, String) {
    let trimmed = draft_md.trim_start();
    let cn = contract_name.trim();
    if let Some(rest) = trimmed.strip_prefix("# ") {
        let (first_line, remaining) = match rest.find('\n') {
            Some(nl) => (rest[..nl].trim(), rest[nl + 1..].trim_start()),
            None => (rest.trim(), ""),
        };
        let h1 = first_line.trim_matches(|c| c == '《' || c == '》').trim();
        let title = if cn.is_empty() { h1 } else { cn };
        let title = if title.is_empty() { "合同" } else { title };
        return (title.to_string(), remaining.to_string());
    }
    let title = if cn.is_empty() { "合同" } else { cn };
    (title.to_string(), draft_md.to_string())
}
