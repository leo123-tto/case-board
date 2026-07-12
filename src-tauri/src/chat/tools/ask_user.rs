//! 交互工具:`ask_user`(V0.3 · 选项式追问 / Claude Code 风格提问框)。
//!
//! 当模型缺关键信息无法写出有用结果时,调本工具把问题(带预设选项)抛回前端,
//! 前端渲染成可点击的选项卡片;用户点选 / 自由输入后,答案当作**下一条普通 user
//! 消息**回灌,模型下一轮基于答案续写。
//!
//! **本工具不进 parallel 派发** —— `agent_loop` 在派发前就拦截 `ask_user`:emit
//! `ChatStreamEvent::AskUser` + 把问题塞进 `AgentLoopOutput.ask_user` + break,
//! 永远不会真正走到这里的 `execute`。`execute` 仅作防御兜底(万一漏拦,给模型一个
//! 明确回执而不是静默错乱),正常路径不可达。
//!
//! 因此本工具:无副作用、无 case 依赖、不耗元典积分、不走 KB cache。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{Tool, ToolContext, ToolError, ToolResult};

pub struct AskUser;

#[async_trait]
impl Tool for AskUser {
    fn name(&self) -> &str {
        "ask_user"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/ask_user.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "要问用户的问题列表,一般 1-3 个阻塞性关键问题,别一次堆太多",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "description": "问题文本,简洁明确"
                            },
                            "options": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "预设选项(2-4 个常见答案,如 [\"个人\",\"公司\"]);没有合适预设就留空数组"
                            },
                            "allow_input": {
                                "type": "boolean",
                                "description": "是否允许用户自己输入文字(选项穷尽不了、要填具体姓名/金额/日期时设 true)"
                            },
                            "multiple": {
                                "type": "boolean",
                                "description": "是否允许多选。仅当多个选项可以同时成立时设 true；普通追问保持 false"
                            },
                            "min_selections": {
                                "type": "integer",
                                "minimum": 0,
                                "description": "多选时至少选择几项"
                            },
                            "max_selections": {
                                "type": "integer",
                                "minimum": 1,
                                "description": "多选时最多选择几项，不得超过 options 数量"
                            }
                        },
                        "required": ["question"]
                    }
                }
            },
            "required": ["questions"]
        })
    }

    async fn execute(
        &self,
        _args: &Value,
        _ctx: &ToolContext<'_>,
    ) -> Result<ToolResult, ToolError> {
        // 正常路径不可达(agent_loop 在派发前拦截)。兜底回执,避免漏拦时静默错乱。
        Ok(ToolResult::plain(
            "(已向用户发起提问,正在等待用户从选项卡片中选择或输入答案。请勿继续凭空假设,\
             等用户回答后再继续。)",
        ))
    }
}
