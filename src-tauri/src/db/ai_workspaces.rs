use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiWorkspace {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub is_favorite: i64,
    pub last_opened_at: Option<String>,
    pub last_document_id: Option<String>,
    pub last_conversation_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiWorkspaceSummary {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub is_favorite: i64,
    pub last_opened_at: Option<String>,
    pub last_document_id: Option<String>,
    pub last_conversation_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
    pub source_count: i64,
    pub artifact_count: i64,
    pub conversation_count: i64,
}

pub async fn create_workspace(
    pool: &SqlitePool,
    title: &str,
    description: Option<&str>,
) -> Result<AiWorkspace, sqlx::Error> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO ai_workspaces (id, title, description) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(title.trim())
        .bind(description.map(str::trim).filter(|value| !value.is_empty()))
        .execute(pool)
        .await?;
    get_workspace(pool, &id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn get_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Option<AiWorkspace>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspace>("SELECT * FROM ai_workspaces WHERE id = ?")
        .bind(workspace_id)
        .fetch_optional(pool)
        .await
}

pub async fn list_workspaces(
    pool: &SqlitePool,
    query: Option<&str>,
    recent_only: bool,
    include_archived: bool,
) -> Result<Vec<AiWorkspaceSummary>, sqlx::Error> {
    let query = query.map(str::trim).filter(|value| !value.is_empty());
    sqlx::query_as::<_, AiWorkspaceSummary>(
        "SELECT w.*, \
           (SELECT COUNT(*) FROM ai_workspace_documents d \
             WHERE d.workspace_id = w.id AND d.kind = 'source' AND d.archived_at IS NULL) AS source_count, \
           (SELECT COUNT(*) FROM ai_workspace_documents d \
             WHERE d.workspace_id = w.id AND d.kind = 'artifact' AND d.archived_at IS NULL) AS artifact_count, \
           (SELECT COUNT(*) FROM ai_workspace_conversations c \
             WHERE c.workspace_id = w.id AND c.archived_at IS NULL) AS conversation_count \
         FROM ai_workspaces w \
         WHERE (?1 = 1 OR w.archived_at IS NULL) \
           AND (?2 IS NULL OR w.title LIKE '%' || ?2 || '%' \
                OR COALESCE(w.description, '') LIKE '%' || ?2 || '%') \
           AND (?3 = 0 OR w.last_opened_at IS NOT NULL) \
         ORDER BY w.is_favorite DESC, \
           CASE WHEN ?3 = 1 THEN w.last_opened_at ELSE w.updated_at END DESC, \
           w.created_at DESC",
    )
    .bind(i64::from(include_archived))
    .bind(query)
    .bind(i64::from(recent_only))
    .fetch_all(pool)
    .await
}

pub async fn touch_workspace_opened(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<AiWorkspace, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE ai_workspaces SET last_opened_at = datetime('now'), \
         updated_at = datetime('now') WHERE id = ?",
    )
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    get_workspace(pool, workspace_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn update_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
    title: Option<&str>,
    description: Option<Option<&str>>,
    is_favorite: Option<bool>,
) -> Result<AiWorkspace, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM ai_workspaces WHERE id = ?")
        .bind(workspace_id)
        .fetch_optional(&mut *tx)
        .await?;
    if exists.is_none() {
        return Err(sqlx::Error::RowNotFound);
    }
    if let Some(title) = title {
        sqlx::query("UPDATE ai_workspaces SET title = ? WHERE id = ?")
            .bind(title.trim())
            .bind(workspace_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(description) = description {
        sqlx::query("UPDATE ai_workspaces SET description = ? WHERE id = ?")
            .bind(description.map(str::trim).filter(|value| !value.is_empty()))
            .bind(workspace_id)
            .execute(&mut *tx)
            .await?;
    }
    if let Some(is_favorite) = is_favorite {
        sqlx::query("UPDATE ai_workspaces SET is_favorite = ? WHERE id = ?")
            .bind(i64::from(is_favorite))
            .bind(workspace_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("UPDATE ai_workspaces SET updated_at = datetime('now') WHERE id = ?")
        .bind(workspace_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    get_workspace(pool, workspace_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn set_workspace_last_selection(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: Option<&str>,
    conversation_id: Option<&str>,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        "UPDATE ai_workspaces SET last_document_id = ?, last_conversation_id = ?, \
         updated_at = datetime('now') WHERE id = ?",
    )
    .bind(document_id)
    .bind(conversation_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn archive_workspace(pool: &SqlitePool, workspace_id: &str) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        "UPDATE ai_workspaces SET archived_at = datetime('now'), \
         updated_at = datetime('now') WHERE id = ? AND archived_at IS NULL",
    )
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}
