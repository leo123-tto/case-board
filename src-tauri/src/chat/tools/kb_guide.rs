//! 人与 AI 共用的本地知识库检索/维护说明。正文只维护在 `local_kb/guide.md`。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolContext, ToolError, ToolResult};

pub struct GetLocalKbGuide;

#[async_trait]
impl Tool for GetLocalKbGuide {
    fn name(&self) -> &str {
        "get_local_kb_guide"
    }

    fn description(&self) -> &str {
        include_str!("descriptions/get_local_kb_guide.md")
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    async fn execute(&self, _args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let root = ctx.local_kb.map(|kb| kb.root.as_path());
        Ok(ToolResult {
            content: crate::local_kb::guide::render(root),
            yuandian_credits_used: 0,
            kb_hit: false,
        })
    }
}
