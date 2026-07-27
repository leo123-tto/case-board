//! 案件文档语义检索 tool(V0.3.3 阶段3)。
//!
//! `semantic_search_case_docs`:用 embedding 向量 + 余弦相似度,按**语义**(而非关键词)
//! 在本案材料全文里检索 top-N 相关片段。跟 `find_in_document`(精确关键词)互补:
//! 不确定关键词 / 想按主题找时用本工具。
//!
//! 本地向量索引 + 文件 + embedding API，**不消耗元典积分**，但会使用 embedding 服务额度。
//! 需 `ctx.case_id` 非 None;需用户在设置配置 embedding —— 没配则**优雅提示改用关键词工具**
//! (不报错、AI 无感,守「填了才启用,没填回退」原则)。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{opt_u32, require_str, Tool, ToolContext, ToolError, ToolResult};
use crate::db::documents::list_documents_by_case;

const DEFAULT_TOP_N: usize = 6;
const MAX_TOP_N: usize = 12;
/// 单片段回填上限,防超长片段把 tool 结果撑爆。
const EXCERPT_CHAR_LIMIT: usize = 600;

pub struct SemanticSearchCaseDocs;

#[async_trait]
impl Tool for SemanticSearchCaseDocs {
    fn name(&self) -> &str {
        "semantic_search_case_docs"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/semantic_search_case_docs.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "检索语义,用自然语言完整描述想找的内容(如「被告承认欠款的陈述」「关于交付时间的约定」),别只写一个词"},
                "top_n": {"type": "integer", "description": "返回最相关的几个片段,默认 6,最大 12"}
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let case_id = ctx.case_id.ok_or(ToolError::NoCaseBound)?;
        let query = require_str(args, "query")?;
        let top_n = opt_u32(args, "top_n")
            .map(|n| (n as usize).clamp(1, MAX_TOP_N))
            .unwrap_or(DEFAULT_TOP_N);

        // 配置门禁:没配 embedding key → 优雅回退提示(不报错,让模型改用关键词工具,AI 无感)
        let credential = crate::embedding::credential_source();
        if !credential.is_ready().await.map_err(ToolError::Runtime)? {
            return Ok(ToolResult::plain(
                "本案未配置语义检索(embedding 未设置)。请改用 `find_in_document`(关键词精确查)\
                 或 `read_case_doc`(读全文)来查材料;如需启用语义检索,请提示用户在设置页配置 embedding。\
                 不要反复调用本工具。",
            ));
        }
        let endpoint = ctx.settings.embedding_endpoint.as_deref().unwrap_or("");
        let model = ctx.settings.embedding_model.as_deref().unwrap_or("");

        let docs = list_documents_by_case(ctx.pool, case_id).await?;
        // embed 报错透传(坑#8):网络/额度问题让 LLM 看到真错,自行回退关键词工具。
        let hits = crate::embedding::index::semantic_search(
            case_id,
            &docs,
            query,
            top_n,
            endpoint,
            model,
            &credential,
        )
        .await
        .map_err(ToolError::Runtime)?;

        if hits.is_empty() {
            return Ok(ToolResult::plain(
                "语义检索没有命中任何片段(可能本案暂无可索引的材料全文)。\
                 可用 `list_case_docs` 看有哪些材料,或用 `find_in_document` 精确查。",
            ));
        }

        let arr: Vec<Value> = hits
            .iter()
            .map(|h| {
                let excerpt: String = h.text.chars().take(EXCERPT_CHAR_LIMIT).collect();
                json!({
                    "doc_id": h.doc_id,
                    "filename": h.filename,
                    "category": h.category,
                    // 余弦相似度留 3 位,给 LLM 判相关度
                    "score": (h.score * 1000.0).round() / 1000.0,
                    "excerpt": excerpt,
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
