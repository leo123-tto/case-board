use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiWorkspaceDocumentProposal {
    pub id: String,
    pub workspace_id: String,
    pub document_id: String,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub base_revision: i64,
    pub base_content_hash: String,
    pub proposed_markdown: String,
    pub summary: String,
    pub source_snapshot_json: String,
    pub status: String,
    pub resolved_markdown: Option<String>,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

pub async fn create_proposal_from_message(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    conversation_id: &str,
    message_id: &str,
) -> Result<AiWorkspaceDocumentProposal, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let document: Option<(i64, Option<String>)> = sqlx::query_as(
        "SELECT working_copy_revision, working_copy_hash FROM ai_workspace_documents \
         WHERE id = ? AND workspace_id = ? AND kind = 'artifact' AND archived_at IS NULL",
    )
    .bind(document_id)
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (base_revision, base_hash) = document.ok_or(sqlx::Error::RowNotFound)?;
    let message: Option<(String, String)> = sqlx::query_as(
        "SELECT m.content, m.citations_json FROM ai_workspace_messages m \
         JOIN ai_workspace_conversations c ON c.id = m.conversation_id \
         WHERE m.id = ? AND m.conversation_id = ? AND c.workspace_id = ? \
           AND m.role = 'assistant' AND m.status IN ('completed', 'incomplete') \
           AND length(trim(m.content)) > 0",
    )
    .bind(message_id)
    .bind(conversation_id)
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?;
    let (proposed_markdown, source_snapshot_json) = message.ok_or(sqlx::Error::RowNotFound)?;

    sqlx::query(
        "UPDATE ai_workspace_document_proposals SET status = 'superseded', \
         resolved_at = datetime('now') WHERE workspace_id = ? AND document_id = ? \
         AND status = 'pending'",
    )
    .bind(workspace_id)
    .bind(document_id)
    .execute(&mut *tx)
    .await?;
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO ai_workspace_document_proposals \
         (id, workspace_id, document_id, conversation_id, message_id, base_revision, \
          base_content_hash, proposed_markdown, summary, source_snapshot_json) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'AI 建议修改当前文稿', ?)",
    )
    .bind(&id)
    .bind(workspace_id)
    .bind(document_id)
    .bind(conversation_id)
    .bind(message_id)
    .bind(base_revision)
    .bind(base_hash.unwrap_or_default())
    .bind(proposed_markdown)
    .bind(source_snapshot_json)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get_proposal(pool, workspace_id, &id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn get_proposal(
    pool: &SqlitePool,
    workspace_id: &str,
    proposal_id: &str,
) -> Result<Option<AiWorkspaceDocumentProposal>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceDocumentProposal>(
        "SELECT * FROM ai_workspace_document_proposals WHERE id = ? AND workspace_id = ?",
    )
    .bind(proposal_id)
    .bind(workspace_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_pending_proposals(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<Vec<AiWorkspaceDocumentProposal>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceDocumentProposal>(
        "SELECT * FROM ai_workspace_document_proposals \
         WHERE workspace_id = ? AND document_id = ? AND status = 'pending' \
         ORDER BY created_at DESC",
    )
    .bind(workspace_id)
    .bind(document_id)
    .fetch_all(pool)
    .await
}

pub async fn resolve_proposal(
    pool: &SqlitePool,
    workspace_id: &str,
    proposal_id: &str,
    status: &str,
    resolved_markdown: Option<&str>,
) -> Result<AiWorkspaceDocumentProposal, sqlx::Error> {
    if !matches!(status, "accepted" | "rejected") {
        return Err(sqlx::Error::Protocol("invalid proposal status".into()));
    }
    let result = sqlx::query(
        "UPDATE ai_workspace_document_proposals SET status = ?, resolved_markdown = ?, \
         resolved_at = datetime('now') WHERE id = ? AND workspace_id = ? AND status = 'pending'",
    )
    .bind(status)
    .bind(resolved_markdown)
    .bind(proposal_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    get_proposal(pool, workspace_id, proposal_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}
