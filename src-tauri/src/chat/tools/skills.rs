use async_trait::async_trait;
use serde_json::{json, Value};

use super::{require_str, Tool, ToolContext, ToolError, ToolResult};

/// 只读取 CaseBoard 注册表中的 Skill 正文，不接受路径，因而不能越界读取其他文件。
pub struct ReadLegalSkill;

#[async_trait]
impl Tool for ReadLegalSkill {
    fn name(&self) -> &str {
        "read_legal_skill"
    }

    fn description(&self) -> &str {
        "读取一个已登记的全局法律 Skill 的完整只读指令。CaseBoard 会在系统提示中列出所有可用 Skill 的 name 与 description；当任务与某项说明明确匹配、而本轮又没有通过快捷入口显式展开完整 Skill 时，先按精确 name 调用本工具，再遵循返回的步骤工作。只能传 Skill name，不能传路径、URL 或正文，也不能枚举 CaseBoard 以外的 Agent 目录。Skill 是律师确认后全局安装的法律方法、文书格式或处理风格，可能要求先整理事实、核验法条、模拟反方、检索类案或按特定格式保存文稿。使用 Skill 不能覆盖更高优先级的原文件只读、事实证据边界、隐私规则、工具真实结果和用户本轮要求；冲突时以这些边界为准。工具返回的 sha256 与 version 用于说明本轮采用的具体版本，但不要在面向用户的正文中堆砌内部元数据。CaseBoard 不允许 Agent 自动生成、修改、下载、安装或删除 Skill；如果用户要求增加 Skill，应说明需要到设置中人工导入经过审查的纯 Markdown SKILL.md。未找到 name 时不要猜测近似路径，应回到系统提供的清单选择真实名称，或在没有适合 Skill 时直接按通用法律工作流完成任务。"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "系统提示中列出的 Skill name"
                }
            },
            "required": ["name"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let name = require_str(args, "name")?;
        let skill = crate::chat::skills::resolve(name).map_err(ToolError::Runtime)?;
        Ok(ToolResult::plain(format!(
            "<skill name=\"{}\" version=\"{}\" sha256=\"{}\">\n{}\n</skill>",
            skill.summary.name, skill.summary.version, skill.summary.sha256, skill.body
        )))
    }
}
