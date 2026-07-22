//! 流式 chat 的**共享类型**:`ChatStreamEvent`(发给前端的 SSE 事件)+ `ChatUsage`。
//!
//! V0.3.3 起删除了老的无工具流式实现(`run_chat`、`ChatStreamRequest`、`ChatStreamError`
//! 及私有 SSE 解析):所有 chat 统一走 `agent_loop`(它有自己的 SSE 解析与请求体类型)。
//! 本文件现在只留两条被 `agent_loop` / `commands` / `hooks` / `feedback` 共用的类型。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatActivityPhase {
    Run,
    Turn,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChatActivityStatus {
    Started,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatActivity {
    pub runtime: String,
    pub phase: ChatActivityPhase,
    pub status: ChatActivityStatus,
    pub sequence: u32,
    pub turn: Option<u32>,
    pub tool: Option<String>,
    pub elapsed_ms: Option<u64>,
    pub error_category: Option<String>,
}

/// 流式输出事件,通过 tx 发给上层(Tauri 命令 → window.emit)。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChatStreamEvent {
    /// Runtime 的脱敏过程元数据。绝不携带 prompt、正文、推理、工具参数或结果。
    Activity { activity: ChatActivity },
    /// 增量 token(已 utf-8 安全)
    Delta { text: String },
    /// V0.3 · thinking 模型(deepseek-v4-pro)推理阶段的 `reasoning_content` 增量。
    /// 不进正文(reasoning 仍按原逻辑累积回传 DeepSeek),只给前端做「深度推理中…(N 字)」
    /// 的进度反馈 —— 否则大上下文单轮推理可长达几分钟,UI 零反馈像卡死。
    Reasoning { text: String },
    /// V0.2 D6.5 · 单次工具调用完成,前端 ToolCallTrace 实时追加一行。
    /// agent_loop 在 tool_trace.push 后立即 emit。
    ToolCall {
        record: crate::chat::agent_loop::ToolCallRecord,
    },
    /// V0.3 · 模型调 `ask_user` 发起选项式追问。前端收到后渲染选项卡片(A/B/C + 自由输入)。
    /// 紧随其后会有 Done(本轮 agent_loop 已 break)。
    AskUser {
        questions: Vec<crate::chat::agent_loop::AskQuestion>,
    },
    /// 流式结束。携带 usage(可能缺失,某些 endpoint 不返回 stream_options)。
    Done {
        prompt_tokens: Option<u64>,
        completion_tokens: Option<u64>,
        model: String,
    },
    /// 真错(网络 / HTTP / 解析)。透传给前端弹红条。
    Error { message: String },
}

#[derive(Debug, Clone, Default)]
pub struct ChatUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub model: String,
}
