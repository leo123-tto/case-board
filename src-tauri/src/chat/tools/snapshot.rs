//! 案件画像字段级更新工具。只开放已验证的顶层覆盖路径，暂不接收易错的数组索引或子表路径。

use async_trait::async_trait;
use serde_json::Value;
use tauri::Emitter;

use super::{opt_str, require_str, Tool, ToolContext, ToolError, ToolResult};

pub struct UpdateCaseSnapshotField;

const SUPPORTED_FIELDS: &[&str] = &[
    "agg_case_no",
    "agg_court",
    "agg_cause",
    "agg_filed_at",
    "agg_claim_amount",
    "agg_status_text",
    "agg_resolution",
    "agg_our_side",
    "case_summary",
    "case_stage",
    "case_status",
    "case_type",
];

#[async_trait]
impl Tool for UpdateCaseSnapshotField {
    fn name(&self) -> &str {
        "update_case_snapshot_field"
    }

    fn description(&self) -> &str {
        include_str!("descriptions/update_case_snapshot_field.md")
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "field_path": {
                    "type": "string",
                    "enum": SUPPORTED_FIELDS,
                    "description": "要修改的案件画像字段"
                },
                "value": {
                    "type": "string",
                    "description": "新值；空字符串表示用户明确要求清空"
                },
                "reason": {
                    "type": "string",
                    "description": "本次修改依据"
                }
            },
            "required": ["field_path", "value", "reason"],
            "additionalProperties": false
        })
    }

    fn is_mutating(&self) -> bool {
        true
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let case_id = ctx.case_id.ok_or(ToolError::NoCaseBound)?;
        let path = require_str(args, "field_path")?;
        if !SUPPORTED_FIELDS.contains(&path) {
            return Err(ToolError::InvalidArgs(format!(
                "不支持的案件画像字段: {}",
                path
            )));
        }
        let value = args
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("value 必须是字符串；清空请传空字符串".into()))?;
        let reason = opt_str(args, "reason")
            .ok_or_else(|| ToolError::InvalidArgs("修改案件画像必须说明用户确认依据".into()))?;

        crate::db::cases::patch_user_override_field(ctx.pool, case_id, path, Some(value)).await?;
        if let Some(app) = &ctx.app {
            let _ = app.emit(
                "case-snapshot-changed",
                serde_json::json!({ "case_id": case_id, "field_path": path }),
            );
        }
        let action = if value.trim().is_empty() {
            "清空"
        } else {
            "更新"
        };
        Ok(ToolResult::plain(format!(
            "已{}案件画像字段 `{}`。依据：{}。",
            action, path, reason
        )))
    }
}
