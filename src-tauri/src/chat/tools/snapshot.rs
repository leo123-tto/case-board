//! 案件快照(user_overrides)写操作工具。
//!
//! 让 AI 助手能按用户指令直接纠正案件画像中的字段(案号、当事人、金额等),
//! 修改结果写入 `cases.user_overrides_json`,与前端编辑模式共享同一套 overlay 机制,
//! LLM 后续重抽不会覆盖这些人工确认值。

use async_trait::async_trait;
use serde_json::Value;
use tauri::Emitter;

use super::{opt_str, require_str, Tool, ToolContext, ToolError, ToolResult};

/// 修改案件画像(user_overrides_json)里的一个字段。
pub struct UpdateCaseSnapshotField;

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
                    "description": "要修改的字段路径。支持: agg_case_no / agg_court / agg_cause / agg_filed_at / agg_claim_amount / agg_status_text / agg_resolution / agg_our_side / case_summary / case_stage / case_status / case_type / case_note / expected_close_at; 当事人/法官列表项 agg_plaintiffs.0 / agg_defendants.1 / agg_third_parties.0 / agg_judges.0; 子表行内字段 agg_party_contacts.{张三|原告}.phone / agg_court_contacts.{张法官|审判长}.phone / agg_key_dates.{开庭|2024-09-15}.note / agg_fees.{律师代理费|5000}.note"
                },
                "value": {
                    "type": "string",
                    "description": "新值。传空字符串表示清空该字段(写入 null)"
                },
                "reason": {
                    "type": "string",
                    "description": "修改原因,会返回给用户确认"
                }
            },
            "required": ["field_path", "value"]
        })
    }

    fn is_mutating(&self) -> bool {
        true
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let case_id = ctx.case_id.ok_or(ToolError::NoCaseBound)?;
        let path = require_str(args, "field_path")?;
        let value = opt_str(args, "value");
        let reason = opt_str(args, "reason").unwrap_or("未说明");

        validate_field_path(path)?;

        crate::db::cases::patch_user_override_field(ctx.pool, case_id, path, value)
            .await
            .map_err(ToolError::Sqlx)?;

        // 通知前端刷新案件数据,让 AI 修改后的覆盖立即生效。
        if let Some(app) = &ctx.app {
            let _ = app.emit(
                "case-snapshot-changed",
                serde_json::json!({ "case_id": case_id, "field_path": path }),
            );
        }

        let op = if value.map(|s| s.trim().is_empty()).unwrap_or(true) {
            "已清空"
        } else {
            "已更新"
        };
        Ok(ToolResult::plain(format!(
            "{}字段 `{}`。原因:{}。前端已收到刷新通知。",
            op, path, reason
        )))
    }
}

/// 校验 path 是否落在已支持的覆盖格式里,避免 LLM 编造无效路径。
fn validate_field_path(path: &str) -> Result<(), ToolError> {
    if path.is_empty() {
        return Err(ToolError::InvalidArgs("field_path 不能为空".into()));
    }

    // 1. 顶层字段(无 dot)
    if !path.contains('.') {
        if is_top_level_path(path) {
            return Ok(());
        }
        return Err(ToolError::InvalidArgs(format!(
            "不支持的顶层字段路径: {}",
            path
        )));
    }

    let parts: Vec<&str> = path.split('.').collect();

    // 2. 数组型当事人/法官列表: agg_plaintiffs.0
    if parts.len() == 2
        && matches!(
            parts[0],
            "agg_plaintiffs" | "agg_defendants" | "agg_third_parties" | "agg_judges"
        )
        && parts[1].parse::<usize>().is_ok()
    {
        return Ok(());
    }

    // 3. 子表行内字段: agg_party_contacts.{张三|原告}.phone
    if parts.len() == 3
        && matches!(
            parts[0],
            "agg_party_contacts" | "agg_court_contacts" | "agg_key_dates" | "agg_fees"
        )
        && parts[1].starts_with('{')
        && parts[1].ends_with('}')
        && is_identifier(parts[2])
    {
        return Ok(());
    }

    Err(ToolError::InvalidArgs(format!(
        "field_path 格式不被支持: {}。请使用顶层 agg_*、当事人列表索引 agg_plaintiffs.N 或子表 agg_party_contacts.{{name|role}}.inner",
        path
    )))
}

fn is_top_level_path(path: &str) -> bool {
    const TOP_LEVEL: &[&str] = &[
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
        "case_note",
        "expected_close_at",
    ];
    TOP_LEVEL.contains(&path)
}

fn is_identifier(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c == '_' || c.is_ascii_lowercase() || c.is_ascii_digit())
}
