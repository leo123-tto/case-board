//! 案件 AI 助手聊天记录(`chat_messages`)表的 CRUD。
//!
//! V0.1.13+ 案件详情页右侧聊天面板的持久化层。
//!
//! 设计要点:
//!   - 流式输出**完成后**才写一条 assistant 消息(中途不写,避免半句留库)
//!   - prompt_tokens / completion_tokens / latency_ms 由 chat 模块读 DeepSeek
//!     usage 段写入,用于反馈 MD 性能埋点(content **不**进反馈)
//!   - based_on 是 JSON 数组,记录这次回答引用的 document.id;前端 markdown
//!     可以渲染成"来源:民事诉状.docx"之类的引用
//!   - artifact_doc_id 指向 documents 表里的 chat artifact(若本次输出落了 MD)
//!   - error_short:assistant 出错时填脱敏错误(真错透传,见 CLAUDE.md 坑 #9)
//!
//! 隐私铁律 #3:chat_messages.content **永远不进反馈 MD**,
//! feedback::tests 加 regression 测试。

use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

/// 聊天消息表行。
///
/// 一条 user 消息 + 一条 assistant 消息 = 一对 turn。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ChatMessage {
    pub id: String,
    pub case_id: String,
    pub conversation_id: Option<String>,
    /// 'user' / 'assistant'
    pub role: String,
    pub content: String,
    /// NULL=自由问;否则枚举 generate_case_overview / generate_evidence_list /
    /// generate_timeline / generate_client_update / find_payment / list_missing
    pub task_type: Option<String>,
    pub model: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
    /// JSON 数组,例如 `["doc-uuid-1","doc-uuid-2"]`
    pub based_on: Option<String>,
    /// 若 assistant 输出落了 artifact,指向 documents.id
    pub artifact_doc_id: Option<String>,
    /// assistant 出错时填脱敏错误
    pub error_short: Option<String>,
    pub created_at: String,
    /// V0.2 D6.5 · 本轮用户在 AttachmentPicker 引用的 doc.id 列表(JSON 数组字符串)。
    /// user 消息上写;assistant 消息可冗余写一份方便回放(也可保持 null,按 task_id link)。
    pub attached_doc_ids: Option<String>,
    /// V0.2 D6.5 · `<CITATIONS>` 协议解析后落库的 JSON 数组(`Vec<Citation>` 序列化)。
    /// 前端 history 重放时 deserialize 给 CitationsCard 渲染。仅 assistant 消息有值。
    pub citations_json: Option<String>,
    /// V0.2 D6.5 · 关联到 chat_tasks.id(若本条消息走了 agent_loop 路径)。
    /// chat_tasks 行落业务级数据(plan/subtask/tool_calls/credits 等);本字段只是 link。
    /// 前端 history 重放 ToolCallTrace 时按 task_id 反查 chat_tasks.tool_calls_json。
    pub task_id: Option<String>,
}

/// 新建一条聊天消息的入参。
#[derive(Debug, Clone)]
pub struct NewChatMessage<'a> {
    pub id: &'a str,
    pub case_id: &'a str,
    pub conversation_id: Option<&'a str>,
    pub role: &'a str,
    pub content: &'a str,
    pub task_type: Option<&'a str>,
    pub model: Option<&'a str>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
    pub based_on: Option<&'a str>,
    pub artifact_doc_id: Option<&'a str>,
    pub error_short: Option<&'a str>,
    /// V0.2 D6.5 · 引用的 doc.id JSON 数组(`["uuid1","uuid2"]`)
    pub attached_doc_ids: Option<&'a str>,
    /// V0.2 D6.5 · `<CITATIONS>` 解析结果 JSON
    pub citations_json: Option<&'a str>,
    /// V0.2 D6.5 · chat_tasks.id;tool_calls 通过本 link 反查 chat_tasks 表
    pub task_id: Option<&'a str>,
}

/// 插入一条 chat 消息。流式完成后一次性写,中途不写半句。
pub async fn insert_chat_message(
    pool: &SqlitePool,
    msg: NewChatMessage<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO chat_messages \
         (id, case_id, conversation_id, role, content, task_type, model, \
          prompt_tokens, completion_tokens, latency_ms, \
          based_on, artifact_doc_id, error_short, \
          attached_doc_ids, citations_json, task_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(msg.id)
    .bind(msg.case_id)
    .bind(msg.conversation_id)
    .bind(msg.role)
    .bind(msg.content)
    .bind(msg.task_type)
    .bind(msg.model)
    .bind(msg.prompt_tokens)
    .bind(msg.completion_tokens)
    .bind(msg.latency_ms)
    .bind(msg.based_on)
    .bind(msg.artifact_doc_id)
    .bind(msg.error_short)
    .bind(msg.attached_doc_ids)
    .bind(msg.citations_json)
    .bind(msg.task_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_chat_messages_in_conversation(
    pool: &SqlitePool,
    case_id: &str,
    conversation_id: &str,
    limit: Option<i64>,
) -> Result<Vec<ChatMessage>, sqlx::Error> {
    match limit {
        Some(limit) => {
            sqlx::query_as(
                "SELECT * FROM ( \
                   SELECT * FROM chat_messages \
                   WHERE case_id = ? AND conversation_id = ? \
                   ORDER BY created_at DESC, id DESC LIMIT ? \
                 ) ORDER BY created_at ASC, id ASC",
            )
            .bind(case_id)
            .bind(conversation_id)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as(
                "SELECT * FROM chat_messages \
                 WHERE case_id = ? AND conversation_id = ? \
                 ORDER BY created_at ASC, id ASC",
            )
            .bind(case_id)
            .bind(conversation_id)
            .fetch_all(pool)
            .await
        }
    }
}

pub async fn delete_chat_history_for_conversation(
    pool: &SqlitePool,
    case_id: &str,
    conversation_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM chat_messages WHERE case_id = ? AND conversation_id = ?")
        .bind(case_id)
        .bind(conversation_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

/// 列出某案件下所有聊天记录,按 created_at 升序(老的在前)。
///
/// 前端拿到后直接按顺序渲染。limit=None 表示不限,常见 limit=200 防意外暴涨。
pub async fn list_chat_messages(
    pool: &SqlitePool,
    case_id: &str,
    limit: Option<i64>,
) -> Result<Vec<ChatMessage>, sqlx::Error> {
    match limit {
        Some(n) => {
            // 取最近 n 条然后正序返回(用子查询)
            sqlx::query_as::<_, ChatMessage>(
                "SELECT * FROM ( \
                   SELECT * FROM chat_messages \
                   WHERE case_id = ? \
                   ORDER BY created_at DESC \
                   LIMIT ? \
                 ) ORDER BY created_at ASC",
            )
            .bind(case_id)
            .bind(n)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, ChatMessage>(
                "SELECT * FROM chat_messages WHERE case_id = ? ORDER BY created_at ASC",
            )
            .bind(case_id)
            .fetch_all(pool)
            .await
        }
    }
}

/// 删除某案件下全部聊天记录(用户主动清空 / 案件删除级联)。
///
/// 案件删除时 ON DELETE CASCADE 已经自动清理,这个函数留给用户手动清空。
pub async fn delete_chat_history_for_case(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query("DELETE FROM chat_messages WHERE case_id = ?")
        .bind(case_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected())
}
