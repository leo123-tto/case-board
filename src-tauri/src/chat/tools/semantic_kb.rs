//! 本地知识库**语义检索** tool。治元典本地命中率低 —— 整部法律按法条切向量索引,
//! 语义命中对的条文,命中后 `read_kb_file` 拿全文,0 元典积分。
//!
//! 跟 `kb::SearchLocalKb`(关键词)互补:语义模糊查 vs 关键词精确查(法条号/案号)。
//! 走 `local_kb::semantic`，消耗用户所配 embedding 服务额度，**不消耗元典积分**。
//! 没配 embedding key → 优雅提示改用关键词工具(不报错,AI 无感)。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{opt_u32, require_str, Tool, ToolContext, ToolError, ToolResult};

const DEFAULT_TOP_N: usize = 6;
const MAX_TOP_N: usize = 12;
const EXCERPT_CHAR_LIMIT: usize = 600;

pub struct SemanticSearchLocalKb;

#[async_trait]
impl Tool for SemanticSearchLocalKb {
    fn name(&self) -> &str {
        "semantic_search_local_kb"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/semantic_search_local_kb.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "检索语义,用自然语言完整描述想找的内容(如「合同解除的法定情形」「债权人代位权的行使条件」),别只写一个词"},
                "top_n": {"type": "integer", "description": "返回最相关的几个片段,默认 6,最大 12"}
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let query = require_str(args, "query")?;
        let top_n = opt_u32(args, "top_n")
            .map(|n| (n as usize).clamp(1, MAX_TOP_N))
            .unwrap_or(DEFAULT_TOP_N);

        // KB 未启用 → 静默降级返回空(跟 search_local_kb 一致)
        let Some(kb) = ctx.local_kb else {
            return Ok(ToolResult {
                content: "[]".into(),
                yuandian_credits_used: 0,
                kb_hit: false,
            });
        };

        // 没配 embedding key → 优雅提示改用关键词工具(不报错)
        let credential = crate::embedding::credential_source();
        if !credential.is_ready().await.map_err(ToolError::Runtime)? {
            return Ok(ToolResult::plain(
                "本地知识库未配置语义检索(embedding 未设置)。请改用 `search_local_kb`(关键词检索),\
                 或提示用户在设置页配置 embedding 服务。不要反复调用本工具。",
            ));
        }
        let endpoint = ctx.settings.embedding_endpoint.as_deref().unwrap_or("");
        let model = ctx.settings.embedding_model.as_deref().unwrap_or("");

        // embed 报错透传(坑#8),让 LLM 看到真错自行回退关键词工具。
        let hits = crate::local_kb::semantic::semantic_search(
            &kb.root,
            query,
            top_n,
            endpoint,
            model,
            &credential,
        )
        .await
        .map_err(ToolError::Runtime)?;

        if hits.is_empty() {
            return Ok(ToolResult {
                content: "语义检索无结果(本地向量索引可能未建/为空,或确无相关条文)。\
                          可改用 `search_local_kb` 关键词检索;若需启用语义检索,\
                          提示用户在「设置/独立」点「重建向量索引」。"
                    .into(),
                yuandian_credits_used: 0,
                kb_hit: false,
            });
        }

        let arr: Vec<Value> = hits
            .iter()
            .map(|h| {
                let excerpt: String = h.text.chars().take(EXCERPT_CHAR_LIMIT).collect();
                json!({
                    "relative_path": h.rel_path,
                    "score": (h.score * 1000.0).round() / 1000.0,
                    "excerpt": excerpt,
                })
            })
            .collect();
        let content = serde_json::to_string_pretty(&arr).unwrap_or_else(|_| "[]".into());
        Ok(ToolResult {
            content,
            yuandian_credits_used: 0,
            // 本地命中 = 替元典省了一次外查,计入命中率(跟 search_local_kb 一致)
            kb_hit: true,
        })
    }
}
