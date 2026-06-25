//! 案件文档 3 个 tool(V0.2 D2-D3.E)。
//!
//! 全部本地 sqlite + 文件读,**不消耗元典积分**。
//! 三个工具都要求 `ctx.case_id` 非 None,否则报 `NoCaseBound`。

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{opt_bool, opt_u32, require_str, Tool, ToolContext, ToolError, ToolResult};
use crate::db::document_tags;
use crate::db::documents::{list_documents_by_case, Document};

const READ_DEFAULT_CHARS: usize = 8_000;
const READ_MAX_CHARS: usize = 30_000;
const FIND_DEFAULT_MAX_HITS: usize = 10;
const FIND_SNIPPET_CHARS: usize = 200;

/// 在案件文档里定位一份文档 —— **id 或 filename 都能匹配**。
///
/// 实测踩坑(2026-05-31):LLM 经常把 `doc_id` 填成**文件名**(如「5、民事起诉状.docx」)
/// 而不是 UUID,因为 case_snapshot / 用户口语里都用文件名指代文档。原实现只比 `d.id`,
/// 导致大量「找不到 doc_id=xxx.docx」报错。这里放宽:
///   1. 先精确匹配 id(UUID 主路径,零歧义)
///   2. 再精确匹配 filename(LLM 传文件名的常见情况)
///   3. 都没中 → 报错时**带上可用文档清单**(id + filename),让 LLM 自纠重试,
///      而不是只回一句「找不到」让它瞎猜。
pub(crate) fn resolve_doc(docs: Vec<Document>, key: &str) -> Result<Document, ToolError> {
    if let Some(d) = docs.iter().find(|d| d.id == key) {
        return Ok(d.clone());
    }
    if let Some(d) = docs.iter().find(|d| d.filename == key) {
        return Ok(d.clone());
    }
    // 容错:去掉可能的路径前缀再比一次 filename(LLM 偶尔带相对路径)
    if let Some(stripped) = key.rsplit('/').next() {
        if stripped != key {
            if let Some(d) = docs.iter().find(|d| d.filename == stripped) {
                return Ok(d.clone());
            }
        }
    }
    let available: Vec<String> = docs
        .iter()
        .map(|d| format!("{} (id={})", d.filename, d.id))
        .collect();
    Err(ToolError::InvalidArgs(format!(
        "找不到文档「{}」。请用下列 filename 或 id 之一重试:\n{}",
        key,
        available.join("\n")
    )))
}

pub struct ListCaseDocs;

#[async_trait]
impl Tool for ListCaseDocs {
    fn name(&self) -> &str {
        "list_case_docs"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/list_case_docs.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn execute(&self, _args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let case_id = ctx.case_id.ok_or(ToolError::NoCaseBound)?;
        let docs = list_documents_by_case(ctx.pool, case_id).await?;
        let tags = document_tags::list_by_case(ctx.pool, case_id).await?;
        let mut tags_by_doc: std::collections::HashMap<&str, Vec<&document_tags::DocumentTag>> =
            std::collections::HashMap::new();
        for tag in &tags {
            tags_by_doc
                .entry(tag.document_id.as_str())
                .or_default()
                .push(tag);
        }
        let arr: Vec<Value> = docs
            .iter()
            .map(|d| {
                let doc_tags = tags_by_doc.get(d.id.as_str()).cloned().unwrap_or_default();
                let tag_json: Vec<Value> = doc_tags
                    .iter()
                    .map(|t| {
                        json!({
                            "namespace": t.namespace,
                            "value": t.value,
                            "source": t.source,
                        })
                    })
                    .collect();
                let values = |namespace: &str| -> Vec<String> {
                    doc_tags
                        .iter()
                        .filter(|t| t.namespace == namespace)
                        .map(|t| t.value.clone())
                        .collect()
                };
                let first_value = |namespace: &str| -> Option<String> {
                    doc_tags
                        .iter()
                        .find(|t| t.namespace == namespace)
                        .map(|t| t.value.clone())
                };
                json!({
                    "id": d.id,
                    "filename": d.filename,
                    "display_name": d.display_name,
                    "category": d.category,
                    "is_ai_artifact": d.is_ai_artifact,
                    "source": d.source,
                    "has_extracted_text": d.extracted_text_path.is_some(),
                    "pinned_at": d.pinned_at,
                    "size_bytes": d.size_bytes,
                    "organize_tags": tag_json,
                    "importance": first_value(document_tags::NS_IMPORTANCE),
                    "organized_category": first_value(document_tags::NS_CATEGORY),
                    "party_side": values(document_tags::NS_PARTY_SIDE),
                    "evidence_attitude": first_value(document_tags::NS_EVIDENCE_ATTITUDE),
                    "submission_stage": first_value(document_tags::NS_SUBMISSION_STAGE),
                })
            })
            .collect();
        let content = serde_json::to_string_pretty(&arr).unwrap_or_else(|_| "[]".into());
        Ok(ToolResult {
            content,
            yuandian_credits_used: 0,
            kb_hit: false,
        })
    }
}

pub struct ReadCaseDoc;

#[async_trait]
impl Tool for ReadCaseDoc {
    fn name(&self) -> &str {
        "read_case_doc"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/read_case_doc.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "doc_id": {"type": "string", "description": "文档标识:可填 list_case_docs 拿到的 id(UUID,最稳),也可直接填文件名(如「5、民事起诉状.docx」)"},
                "offset": {"type": "integer", "description": "从第几个 char 开始读,默认 0"},
                "length": {"type": "integer", "description": "读多少 char,默认 8000,最大 30000"}
            },
            "required": ["doc_id"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let case_id = ctx.case_id.ok_or(ToolError::NoCaseBound)?;
        let doc_id = require_str(args, "doc_id")?;
        let offset = opt_u32(args, "offset").unwrap_or(0) as usize;
        let length = opt_u32(args, "length")
            .map(|n| (n as usize).min(READ_MAX_CHARS))
            .unwrap_or(READ_DEFAULT_CHARS);

        let docs = list_documents_by_case(ctx.pool, case_id).await?;
        let doc = resolve_doc(docs, doc_id)?;
        let txt_path = doc.extracted_text_path.as_deref().ok_or_else(|| {
            ToolError::Runtime(format!(
                "文档「{}」还没抽取过文字(可能是证据/合同等被跳过抽取的材料)。\
                 它的原文未进抽取库,无法 read_case_doc;如确需内容,请提示用户在详情页对该文件单独重抽。",
                doc.filename
            ))
        })?;
        let content_raw = std::fs::read_to_string(PathBuf::from(txt_path))?;
        let chars: Vec<char> = content_raw.chars().collect();
        let total_chars = chars.len();
        let start = offset.min(total_chars);
        let end = (start + length).min(total_chars);
        let slice: String = chars[start..end].iter().collect();
        let has_more = end < total_chars;

        let result = json!({
            "filename": doc.filename,
            "category": doc.category,
            "total_chars": total_chars,
            "has_more": has_more,
            "content": slice,
        });
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".into()),
            yuandian_credits_used: 0,
            kb_hit: false,
        })
    }
}

pub struct FindInDocument;

#[async_trait]
impl Tool for FindInDocument {
    fn name(&self) -> &str {
        "find_in_document"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/find_in_document.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "doc_id": {"type": "string", "description": "文档标识:id(UUID)或文件名都可"},
                "pattern": {"type": "string", "description": "搜索关键词,自动 escape 特殊字符"},
                "case_sensitive": {"type": "boolean", "description": "默认 false"},
                "max_hits": {"type": "integer", "description": "默认 10"}
            },
            "required": ["doc_id", "pattern"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let case_id = ctx.case_id.ok_or(ToolError::NoCaseBound)?;
        let doc_id = require_str(args, "doc_id")?;
        let pattern_raw = require_str(args, "pattern")?;
        let case_sensitive = opt_bool(args, "case_sensitive").unwrap_or(false);
        let max_hits = opt_u32(args, "max_hits").unwrap_or(FIND_DEFAULT_MAX_HITS as u32) as usize;

        let docs = list_documents_by_case(ctx.pool, case_id).await?;
        let doc = resolve_doc(docs, doc_id)?;
        let txt_path = doc.extracted_text_path.as_deref().ok_or_else(|| {
            ToolError::Runtime(format!(
                "文档「{}」还没抽取过文字(可能是被跳过抽取的证据/合同材料),无法在其中搜索。",
                doc.filename
            ))
        })?;
        let content_raw = std::fs::read_to_string(PathBuf::from(txt_path))?;

        let pattern = if case_sensitive {
            regex::escape(pattern_raw)
        } else {
            format!("(?i){}", regex::escape(pattern_raw))
        };
        let re = regex::Regex::new(&pattern)
            .map_err(|e| ToolError::InvalidArgs(format!("regex 编译失败:{}", e)))?;

        let mut hits: Vec<Value> = Vec::new();
        for (line_no, line) in content_raw.lines().enumerate() {
            if hits.len() >= max_hits {
                break;
            }
            if let Some(m) = re.find(line) {
                let half = FIND_SNIPPET_CHARS / 2;
                let s = m.start().saturating_sub(half);
                let e = (m.end() + half).min(line.len());
                let snippet = safe_char_slice(line, s, e);
                hits.push(json!({
                    "line_no": line_no + 1,
                    "snippet": snippet,
                    "match_start": m.start() - s,
                    "match_end": m.end() - s,
                }));
            }
        }

        Ok(ToolResult {
            content: serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".into()),
            yuandian_credits_used: 0,
            kb_hit: false,
        })
    }
}

fn safe_char_slice(s: &str, mut start: usize, mut end: usize) -> String {
    while start > 0 && !s.is_char_boundary(start) {
        start -= 1;
    }
    while end < s.len() && !s.is_char_boundary(end) {
        end += 1;
    }
    s[start..end].to_string()
}
