//! 案件 AI 助手(case-aware chat)模块。
//!
//! 2026-05-27 V0.1.13+ 起的新模块,为案件详情页右侧聊天面板提供:
//!   - context:案件 snapshot + 文档轻量摘要(`case_snapshot_md` / `lightweight_docs_md`,
//!     被 constitution 复用)+ TaskType 路由枚举
//!   - constitution:完整宪法 system prompt(所有 chat 走它)
//!   - prompts:4 个工具/分析型任务的 user prompt 模板
//!   - stream:流式 chat 的共享类型(ChatStreamEvent / ChatUsage)
//!   - agent_loop:function calling 工具链路(V0.3.3 起所有 chat 的唯一执行路径)
//!
//! 跟现有 `llm` 模块的关系:
//!   - `llm::LlmConfig` 复用(读 Settings,决定本机 / 云端 endpoint + model)
//!   - `llm::extract_case_fields_*` 是**非流式 + JSON 提取**的抽取链路,跟本模块解耦
//!   - 本模块只负责 chat 的流式输出,不做结构化抽取
//!
//! 数据持久化由上层 Tauri 命令负责(写 chat_messages 表),本模块只做生成。

pub mod agent_loop;
pub mod arg_repair;
pub mod artifact_intent;
pub mod citations;
pub mod commands;
pub mod constitution;
pub mod context;
pub mod diagnostics;
pub mod hooks;
pub mod loop_guard;
pub mod mcp_bridge;
pub mod mcp_paste;
pub mod memory_extract;
pub mod model_router;
pub mod parallel;
pub mod policy;
pub mod prefix_cache;
pub mod prompts;
pub mod quality_gate;
pub mod retrieval_policy;
pub mod runtime;
pub mod skills;
pub mod stream;
pub mod task_contract;
pub mod tools;

pub use commands::{
    cancel_chat_impl, case_chat_impl, clear_chat_conversation_impl, clear_chat_history_impl,
    list_chat_history_impl, steer_case_chat_impl, CaseChatInput, CaseChatResult,
    ChatCancelRegistry, ChatRunScope,
};
pub use context::TaskType;
pub use prompts::task_user_prompt;
pub use stream::{ChatStreamEvent, ChatUsage};
