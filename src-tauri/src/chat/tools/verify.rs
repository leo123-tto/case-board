//! 法律幻觉校验 1 个 tool(V0.2 D2-D3.D)。
//!
//! `verify_legal_citations` 调元典 `hall_detect`,**不缓存**(实时校验)。
//! 付费接口(贵),**默认不自动调** —— 仅当用户明确要求核验引用时才走
//! (`TaskType::VerifyMyDraft` 或用户直接要求);防幻觉平时靠"必查现行版本"+ `<CITATIONS>`。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{require_str, yuandian_key, Tool, ToolContext, ToolError, ToolResult};
use crate::yuandian;

pub struct VerifyLegalCitations;

#[async_trait]
impl Tool for VerifyLegalCitations {
    fn name(&self) -> &str {
        "verify_legal_citations"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/verify_legal_citations.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": {
                    "type": "string",
                    "description": "要校验的文本,含法规/案号引用。建议 <4000 字,长文本拆段分多次调"
                }
            },
            "required": ["text"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let text = require_str(args, "text")?;
        let api_key = yuandian_key(ctx)?;
        // **不缓存** — 校验需要实时数据
        let resp = yuandian::hall_detect(api_key, text).await?;
        Ok(ToolResult {
            content: serde_json::to_string_pretty(&resp).unwrap_or_else(|_| "{}".into()),
            yuandian_credits_used: crate::db::credits::credits_for_query_type("hall_detect"),
            kb_hit: false,
        })
    }
}
