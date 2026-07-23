use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiWorkspaceConversation {
    pub id: String,
    pub workspace_id: String,
    pub title: String,
    pub title_is_manual: i64,
    pub last_message_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiWorkspaceMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub status: String,
    pub attached_document_ids_json: String,
    pub citations_json: String,
    pub artifact_document_id: Option<String>,
    pub model: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub latency_ms: Option<i64>,
    pub error_short: Option<String>,
    pub task_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiWorkspaceTask {
    pub id: String,
    pub workspace_id: String,
    pub conversation_id: String,
    pub assistant_message_id: String,
    pub status: String,
    pub input_json: String,
    pub tool_calls_json: String,
    pub error_short: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

pub struct NewWorkspaceMessage<'a> {
    pub id: &'a str,
    pub conversation_id: &'a str,
    pub role: &'a str,
    pub content: &'a str,
    pub status: &'a str,
    pub attached_document_ids_json: &'a str,
}

pub async fn create_conversation(
    pool: &SqlitePool,
    workspace_id: &str,
    title: &str,
    title_is_manual: bool,
) -> Result<AiWorkspaceConversation, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO ai_workspace_conversations (id, workspace_id, title, title_is_manual) \
         VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(workspace_id)
    .bind(title.trim())
    .bind(i64::from(title_is_manual))
    .execute(pool)
    .await?;
    get_conversation(pool, workspace_id, &id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn get_conversation(
    pool: &SqlitePool,
    workspace_id: &str,
    conversation_id: &str,
) -> Result<Option<AiWorkspaceConversation>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceConversation>(
        "SELECT * FROM ai_workspace_conversations \
         WHERE id = ? AND workspace_id = ? AND archived_at IS NULL",
    )
    .bind(conversation_id)
    .bind(workspace_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_conversations(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Vec<AiWorkspaceConversation>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceConversation>(
        "SELECT * FROM ai_workspace_conversations \
         WHERE workspace_id = ? AND archived_at IS NULL \
         ORDER BY COALESCE(last_message_at, updated_at) DESC, created_at DESC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

pub async fn rename_conversation(
    pool: &SqlitePool,
    workspace_id: &str,
    conversation_id: &str,
    title: &str,
) -> Result<AiWorkspaceConversation, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE ai_workspace_conversations SET title = ?, title_is_manual = 1, \
         updated_at = datetime('now') WHERE id = ? AND workspace_id = ? AND archived_at IS NULL",
    )
    .bind(title.trim())
    .bind(conversation_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    get_conversation(pool, workspace_id, conversation_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn archive_conversation(
    pool: &SqlitePool,
    workspace_id: &str,
    conversation_id: &str,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        "UPDATE ai_workspace_conversations SET archived_at = datetime('now'), \
         updated_at = datetime('now') WHERE id = ? AND workspace_id = ? AND archived_at IS NULL",
    )
    .bind(conversation_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn set_last_conversation(
    pool: &SqlitePool,
    workspace_id: &str,
    conversation_id: &str,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        "UPDATE ai_workspaces SET last_conversation_id = ?, last_opened_at = datetime('now'), \
         updated_at = datetime('now') WHERE id = ? AND archived_at IS NULL \
           AND EXISTS (SELECT 1 FROM ai_workspace_conversations c \
             WHERE c.id = ? AND c.workspace_id = ai_workspaces.id AND c.archived_at IS NULL)",
    )
    .bind(conversation_id)
    .bind(workspace_id)
    .bind(conversation_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn insert_message(
    pool: &SqlitePool,
    message: NewWorkspaceMessage<'_>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO ai_workspace_messages \
         (id, conversation_id, role, content, status, attached_document_ids_json) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(message.id)
    .bind(message.conversation_id)
    .bind(message.role)
    .bind(message.content)
    .bind(message.status)
    .bind(message.attached_document_ids_json)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE ai_workspace_conversations SET last_message_at = datetime('now'), \
         updated_at = datetime('now') WHERE id = ?",
    )
    .bind(message.conversation_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

pub async fn list_messages(
    pool: &SqlitePool,
    workspace_id: &str,
    conversation_id: &str,
    limit: Option<i64>,
) -> Result<Vec<AiWorkspaceMessage>, sqlx::Error> {
    match limit {
        Some(limit) => {
            sqlx::query_as::<_, AiWorkspaceMessage>(
                "SELECT m.* FROM ai_workspace_messages m \
                 JOIN ai_workspace_conversations c ON c.id = m.conversation_id \
                 WHERE m.conversation_id = ? AND c.workspace_id = ? \
                   AND m.rowid IN ( \
                     SELECT recent.rowid FROM ai_workspace_messages recent \
                     WHERE recent.conversation_id = ? \
                     ORDER BY recent.rowid DESC LIMIT ? \
                   ) \
                 ORDER BY m.rowid ASC",
            )
            .bind(conversation_id)
            .bind(workspace_id)
            .bind(conversation_id)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, AiWorkspaceMessage>(
                "SELECT m.* FROM ai_workspace_messages m \
                 JOIN ai_workspace_conversations c ON c.id = m.conversation_id \
                 WHERE m.conversation_id = ? AND c.workspace_id = ? \
                 ORDER BY m.rowid ASC",
            )
            .bind(conversation_id)
            .bind(workspace_id)
            .fetch_all(pool)
            .await
        }
    }
}

pub async fn get_message(
    pool: &SqlitePool,
    workspace_id: &str,
    message_id: &str,
) -> Result<Option<AiWorkspaceMessage>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceMessage>(
        "SELECT m.* FROM ai_workspace_messages m \
         JOIN ai_workspace_conversations c ON c.id = m.conversation_id \
         WHERE m.id = ? AND c.workspace_id = ?",
    )
    .bind(message_id)
    .bind(workspace_id)
    .fetch_optional(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn update_message_run(
    pool: &SqlitePool,
    workspace_id: &str,
    message_id: &str,
    content: &str,
    status: &str,
    error_short: Option<&str>,
    model: Option<&str>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    latency_ms: Option<i64>,
    citations_json: &str,
    task_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        "UPDATE ai_workspace_messages SET content = ?, status = ?, error_short = ?, model = ?, \
         prompt_tokens = ?, completion_tokens = ?, latency_ms = ?, citations_json = ?, task_id = ?, \
         updated_at = datetime('now') \
         WHERE id = ? AND conversation_id IN \
           (SELECT id FROM ai_workspace_conversations WHERE workspace_id = ?)",
    )
    .bind(content)
    .bind(status)
    .bind(error_short)
    .bind(model)
    .bind(prompt_tokens)
    .bind(completion_tokens)
    .bind(latency_ms)
    .bind(citations_json)
    .bind(task_id)
    .bind(message_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn begin_chat_run(
    pool: &SqlitePool,
    workspace_id: &str,
    conversation_id: &str,
    user_message_id: &str,
    assistant_message_id: &str,
    task_id: &str,
    user_content: &str,
    attached_document_ids_json: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    let conversation: Option<(String, i64)> = sqlx::query_as(
        "SELECT title, title_is_manual FROM ai_workspace_conversations \
         WHERE id = ? AND workspace_id = ? AND archived_at IS NULL",
    )
    .bind(conversation_id)
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (_, title_is_manual) = conversation.ok_or(sqlx::Error::RowNotFound)?;
    let prior_user_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ai_workspace_messages WHERE conversation_id = ? AND role = 'user'",
    )
    .bind(conversation_id)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO ai_workspace_messages \
         (id, conversation_id, role, content, status, attached_document_ids_json) \
         VALUES (?, ?, 'user', ?, 'completed', ?)",
    )
    .bind(user_message_id)
    .bind(conversation_id)
    .bind(user_content)
    .bind(attached_document_ids_json)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO ai_workspace_messages \
         (id, conversation_id, role, content, status, attached_document_ids_json, task_id) \
         VALUES (?, ?, 'assistant', '', 'streaming', '[]', ?)",
    )
    .bind(assistant_message_id)
    .bind(conversation_id)
    .bind(task_id)
    .execute(&mut *tx)
    .await?;
    let input_json = serde_json::json!({
        "user_message_id": user_message_id,
        "attached_document_ids": serde_json::from_str::<serde_json::Value>(attached_document_ids_json)
            .unwrap_or_else(|_| serde_json::json!([])),
    })
    .to_string();
    sqlx::query(
        "INSERT INTO ai_workspace_tasks \
         (id, workspace_id, conversation_id, assistant_message_id, status, input_json) \
         VALUES (?, ?, ?, ?, 'streaming', ?)",
    )
    .bind(task_id)
    .bind(workspace_id)
    .bind(conversation_id)
    .bind(assistant_message_id)
    .bind(input_json)
    .execute(&mut *tx)
    .await?;

    if title_is_manual == 0 && prior_user_count.0 == 0 {
        let auto_title: String = user_content.trim().chars().take(20).collect();
        if !auto_title.is_empty() {
            sqlx::query(
                "UPDATE ai_workspace_conversations SET title = ? WHERE id = ? AND workspace_id = ?",
            )
            .bind(auto_title)
            .bind(conversation_id)
            .bind(workspace_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    sqlx::query(
        "UPDATE ai_workspace_conversations SET last_message_at = datetime('now'), \
         updated_at = datetime('now') WHERE id = ?",
    )
    .bind(conversation_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE ai_workspaces SET last_conversation_id = ?, last_opened_at = datetime('now'), \
         updated_at = datetime('now') WHERE id = ?",
    )
    .bind(conversation_id)
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await
}

pub async fn update_task_run(
    pool: &SqlitePool,
    workspace_id: &str,
    task_id: &str,
    status: &str,
    tool_calls_json: &str,
    error_short: Option<&str>,
) -> Result<(), sqlx::Error> {
    let terminal = matches!(status, "completed" | "incomplete" | "failed" | "cancelled");
    let result = sqlx::query(
        "UPDATE ai_workspace_tasks SET status = ?, tool_calls_json = ?, error_short = ?, \
         updated_at = datetime('now'), finished_at = CASE WHEN ? THEN datetime('now') ELSE NULL END \
         WHERE id = ? AND workspace_id = ?",
    )
    .bind(status)
    .bind(tool_calls_json)
    .bind(error_short)
    .bind(terminal)
    .bind(task_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn checkpoint_chat_run(
    pool: &SqlitePool,
    workspace_id: &str,
    message_id: &str,
    task_id: &str,
    partial_content: &str,
    tool_calls_json: &str,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let task = sqlx::query(
        "UPDATE ai_workspace_tasks SET tool_calls_json = ?, updated_at = datetime('now') \
         WHERE id = ? AND workspace_id = ? AND status IN ('queued', 'streaming')",
    )
    .bind(tool_calls_json)
    .bind(task_id)
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?;
    if task.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    let message = sqlx::query(
        "UPDATE ai_workspace_messages SET content = ?, updated_at = datetime('now') \
         WHERE id = ? AND status IN ('queued', 'streaming') AND conversation_id IN \
           (SELECT id FROM ai_workspace_conversations WHERE workspace_id = ?)",
    )
    .bind(partial_content)
    .bind(message_id)
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?;
    if message.rows_affected() == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    tx.commit().await?;
    Ok(true)
}

pub async fn list_tasks(
    pool: &SqlitePool,
    workspace_id: &str,
    conversation_id: &str,
) -> Result<Vec<AiWorkspaceTask>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceTask>(
        "SELECT * FROM ai_workspace_tasks \
         WHERE workspace_id = ? AND conversation_id = ? \
         ORDER BY created_at ASC",
    )
    .bind(workspace_id)
    .bind(conversation_id)
    .fetch_all(pool)
    .await
}

pub async fn recover_interrupted_runs(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let tasks = sqlx::query(
        "UPDATE ai_workspace_tasks SET status = 'incomplete', \
         error_short = '上次运行被中断，可继续在本对话中重试', updated_at = datetime('now'), \
         finished_at = datetime('now') WHERE status IN ('queued', 'streaming')",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE ai_workspace_messages SET status = 'incomplete', \
         error_short = COALESCE(error_short, '上次运行被中断，可继续重试'), updated_at = datetime('now') \
         WHERE status IN ('queued', 'streaming') AND id IN \
           (SELECT assistant_message_id FROM ai_workspace_tasks WHERE status = 'incomplete')",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(tasks.rows_affected())
}
