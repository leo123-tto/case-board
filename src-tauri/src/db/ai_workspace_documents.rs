use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DocumentStoreError {
    #[error("数据库错误:{0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("工作区文档不存在")]
    NotFound,
    #[error("文稿已被其他保存更新，请刷新后重试")]
    RevisionConflict,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiWorkspaceDocument {
    pub id: String,
    pub workspace_id: String,
    pub kind: String,
    pub title: String,
    pub filename: String,
    pub mime_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub source_path: Option<String>,
    pub normalized_source_path: Option<String>,
    pub content_path: Option<String>,
    pub extracted_text_path: Option<String>,
    pub extraction_status: String,
    pub last_error: Option<String>,
    pub missing: i64,
    pub quality_status: Option<String>,
    pub working_copy_revision: i64,
    pub working_copy_hash: Option<String>,
    pub latest_version_no: i64,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiWorkspaceDocumentVersion {
    pub id: String,
    pub document_id: String,
    pub version_no: i64,
    pub content_md: String,
    pub created_by: String,
    pub trigger: String,
    pub change_summary: String,
    pub source_snapshot_json: String,
    pub message_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AiWorkspaceDocumentChunk {
    pub id: String,
    pub document_id: String,
    pub ordinal: i64,
    pub page_no: Option<i64>,
    pub section_label: Option<String>,
    pub content: String,
    pub content_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct DocumentChunkInput {
    pub ordinal: i64,
    pub page_no: Option<i64>,
    pub section_label: Option<String>,
    pub content: String,
    pub content_hash: String,
}

#[allow(clippy::too_many_arguments)]
pub async fn create_source_document(
    pool: &SqlitePool,
    workspace_id: &str,
    filename: &str,
    source_path: &str,
    normalized_source_path: &str,
    mime_type: Option<&str>,
    size_bytes: Option<i64>,
) -> Result<AiWorkspaceDocument, DocumentStoreError> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO ai_workspace_documents \
         (id, workspace_id, kind, title, filename, mime_type, size_bytes, source_path, \
          normalized_source_path, extraction_status) \
         VALUES (?, ?, 'source', ?, ?, ?, ?, ?, ?, 'queued')",
    )
    .bind(&id)
    .bind(workspace_id)
    .bind(filename)
    .bind(filename)
    .bind(mime_type)
    .bind(size_bytes)
    .bind(source_path)
    .bind(normalized_source_path)
    .execute(pool)
    .await?;
    get_document(pool, workspace_id, &id)
        .await?
        .ok_or(DocumentStoreError::NotFound)
}

pub async fn create_artifact_document(
    pool: &SqlitePool,
    workspace_id: &str,
    title: &str,
    filename: &str,
    content_path: &str,
    working_copy_hash: &str,
) -> Result<AiWorkspaceDocument, DocumentStoreError> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO ai_workspace_documents \
         (id, workspace_id, kind, title, filename, content_path, extraction_status, working_copy_hash) \
         VALUES (?, ?, 'artifact', ?, ?, ?, 'not_required', ?)",
    )
    .bind(&id)
    .bind(workspace_id)
    .bind(title.trim())
    .bind(filename)
    .bind(content_path)
    .bind(working_copy_hash)
    .execute(pool)
    .await?;
    get_document(pool, workspace_id, &id)
        .await?
        .ok_or(DocumentStoreError::NotFound)
}

#[allow(clippy::too_many_arguments)]
pub async fn create_artifact_with_initial_version(
    pool: &SqlitePool,
    id: &str,
    workspace_id: &str,
    title: &str,
    filename: &str,
    content_path: &str,
    working_copy_hash: &str,
    content_md: &str,
) -> Result<AiWorkspaceDocument, DocumentStoreError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO ai_workspace_documents \
         (id, workspace_id, kind, title, filename, mime_type, content_path, extraction_status, \
          working_copy_hash, latest_version_no) \
         VALUES (?, ?, 'artifact', ?, ?, 'text/markdown', ?, 'not_required', ?, 1)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(title.trim())
    .bind(filename)
    .bind(content_path)
    .bind(working_copy_hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO ai_workspace_document_versions \
         (id, document_id, version_no, content_md, created_by, trigger, change_summary, source_snapshot_json) \
         VALUES (?, ?, 1, ?, 'user', 'created', '创建文稿', '[]')",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(id)
    .bind(content_md)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE ai_workspaces SET last_document_id = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(id)
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get_document(pool, workspace_id, id)
        .await?
        .ok_or(DocumentStoreError::NotFound)
}

#[allow(clippy::too_many_arguments)]
pub async fn create_artifact_from_assistant_message(
    pool: &SqlitePool,
    id: &str,
    workspace_id: &str,
    title: &str,
    filename: &str,
    content_path: &str,
    working_copy_hash: &str,
    content_md: &str,
    message_id: &str,
    source_snapshot_json: &str,
) -> Result<AiWorkspaceDocument, DocumentStoreError> {
    let mut tx = pool.begin().await?;
    let message_scope: Option<(i64,)> = sqlx::query_as(
        "SELECT 1 FROM ai_workspace_messages m \
         JOIN ai_workspace_conversations c ON c.id = m.conversation_id \
         WHERE m.id = ? AND c.workspace_id = ? AND m.role = 'assistant' \
           AND m.status IN ('completed', 'incomplete') AND length(trim(m.content)) > 0",
    )
    .bind(message_id)
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?;
    if message_scope.is_none() {
        return Err(DocumentStoreError::NotFound);
    }
    sqlx::query(
        "INSERT INTO ai_workspace_documents \
         (id, workspace_id, kind, title, filename, mime_type, content_path, extraction_status, \
          working_copy_hash, latest_version_no) \
         VALUES (?, ?, 'artifact', ?, ?, 'text/markdown', ?, 'not_required', ?, 1)",
    )
    .bind(id)
    .bind(workspace_id)
    .bind(title.trim())
    .bind(filename)
    .bind(content_path)
    .bind(working_copy_hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO ai_workspace_document_versions \
         (id, document_id, version_no, content_md, created_by, trigger, change_summary, \
          source_snapshot_json, message_id) \
         VALUES (?, ?, 1, ?, 'ai', 'ai_created', '由 AI 对话保存为新文稿', ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(id)
    .bind(content_md)
    .bind(source_snapshot_json)
    .bind(message_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE ai_workspace_messages SET artifact_document_id = ?, updated_at = datetime('now') \
         WHERE id = ?",
    )
    .bind(id)
    .bind(message_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE ai_workspaces SET last_document_id = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(id)
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get_document(pool, workspace_id, id)
        .await?
        .ok_or(DocumentStoreError::NotFound)
}

pub async fn get_document(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<Option<AiWorkspaceDocument>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceDocument>(
        "SELECT * FROM ai_workspace_documents \
         WHERE id = ? AND workspace_id = ? AND archived_at IS NULL",
    )
    .bind(document_id)
    .bind(workspace_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_documents(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Vec<AiWorkspaceDocument>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceDocument>(
        "SELECT * FROM ai_workspace_documents \
         WHERE workspace_id = ? AND archived_at IS NULL \
         ORDER BY CASE kind WHEN 'source' THEN 0 ELSE 1 END, updated_at DESC, id",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

pub async fn mark_extraction_started(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<(), DocumentStoreError> {
    let result = sqlx::query(
        "UPDATE ai_workspace_documents SET extraction_status = 'processing', last_error = NULL, \
         missing = 0, updated_at = datetime('now') \
         WHERE id = ? AND workspace_id = ? AND kind = 'source' AND archived_at IS NULL",
    )
    .bind(document_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DocumentStoreError::NotFound);
    }
    Ok(())
}

pub async fn finish_extraction(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    extracted_text_path: &str,
    quality_status: &str,
) -> Result<(), DocumentStoreError> {
    let result = sqlx::query(
        "UPDATE ai_workspace_documents SET extraction_status = ?, extracted_text_path = ?, \
         quality_status = ?, last_error = NULL, missing = 0, updated_at = datetime('now') \
         WHERE id = ? AND workspace_id = ? AND kind = 'source' AND archived_at IS NULL",
    )
    .bind(quality_status)
    .bind(extracted_text_path)
    .bind(quality_status)
    .bind(document_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DocumentStoreError::NotFound);
    }
    Ok(())
}

pub async fn fail_extraction(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    error: &str,
    missing: bool,
) -> Result<(), DocumentStoreError> {
    let status = if missing { "missing" } else { "failed" };
    let result = sqlx::query(
        "UPDATE ai_workspace_documents SET extraction_status = ?, last_error = ?, missing = ?, \
         updated_at = datetime('now') \
         WHERE id = ? AND workspace_id = ? AND kind = 'source' AND archived_at IS NULL",
    )
    .bind(status)
    .bind(error)
    .bind(i64::from(missing))
    .bind(document_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DocumentStoreError::NotFound);
    }
    Ok(())
}

pub async fn reset_source_extraction(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<AiWorkspaceDocument, DocumentStoreError> {
    let result = sqlx::query(
        "UPDATE ai_workspace_documents SET extraction_status = 'queued', last_error = NULL, \
         missing = 0, updated_at = datetime('now') \
         WHERE id = ? AND workspace_id = ? AND kind = 'source' AND archived_at IS NULL",
    )
    .bind(document_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DocumentStoreError::NotFound);
    }
    get_document(pool, workspace_id, document_id)
        .await?
        .ok_or(DocumentStoreError::NotFound)
}

pub async fn relink_source_document(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    filename: &str,
    source_path: &str,
    normalized_source_path: &str,
    size_bytes: i64,
) -> Result<AiWorkspaceDocument, DocumentStoreError> {
    let result = sqlx::query(
        "UPDATE ai_workspace_documents SET filename = ?, title = ?, source_path = ?, \
         normalized_source_path = ?, size_bytes = ?, extraction_status = 'queued', \
         last_error = NULL, missing = 0, updated_at = datetime('now') \
         WHERE id = ? AND workspace_id = ? AND kind = 'source' AND archived_at IS NULL",
    )
    .bind(filename)
    .bind(filename)
    .bind(source_path)
    .bind(normalized_source_path)
    .bind(size_bytes)
    .bind(document_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DocumentStoreError::NotFound);
    }
    get_document(pool, workspace_id, document_id)
        .await?
        .ok_or(DocumentStoreError::NotFound)
}

pub async fn archive_document(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<(), DocumentStoreError> {
    let result = sqlx::query(
        "UPDATE ai_workspace_documents SET archived_at = datetime('now'), updated_at = datetime('now') \
         WHERE id = ? AND workspace_id = ? AND archived_at IS NULL",
    )
    .bind(document_id)
    .bind(workspace_id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(DocumentStoreError::NotFound);
    }
    Ok(())
}

pub async fn replace_document_chunks(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    chunks: &[DocumentChunkInput],
) -> Result<(), DocumentStoreError> {
    if get_document(pool, workspace_id, document_id)
        .await?
        .is_none()
    {
        return Err(DocumentStoreError::NotFound);
    }
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM ai_workspace_document_chunks WHERE document_id = ?")
        .bind(document_id)
        .execute(&mut *tx)
        .await?;
    for chunk in chunks {
        sqlx::query(
            "INSERT INTO ai_workspace_document_chunks \
             (id, document_id, ordinal, page_no, section_label, content, content_hash) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(document_id)
        .bind(chunk.ordinal)
        .bind(chunk.page_no)
        .bind(&chunk.section_label)
        .bind(&chunk.content)
        .bind(&chunk.content_hash)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn list_document_chunks(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<Vec<AiWorkspaceDocumentChunk>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceDocumentChunk>(
        "SELECT c.* FROM ai_workspace_document_chunks c \
         JOIN ai_workspace_documents d ON d.id = c.document_id \
         WHERE c.document_id = ? AND d.workspace_id = ? AND d.archived_at IS NULL \
         ORDER BY c.ordinal ASC",
    )
    .bind(document_id)
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

pub async fn update_working_copy_revision(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    expected_revision: i64,
    working_copy_hash: &str,
) -> Result<AiWorkspaceDocument, DocumentStoreError> {
    let result = sqlx::query(
        "UPDATE ai_workspace_documents \
         SET working_copy_revision = working_copy_revision + 1, working_copy_hash = ?, \
             updated_at = datetime('now') \
         WHERE id = ? AND workspace_id = ? AND kind = 'artifact' AND archived_at IS NULL \
           AND working_copy_revision = ?",
    )
    .bind(working_copy_hash)
    .bind(document_id)
    .bind(workspace_id)
    .bind(expected_revision)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return if get_document(pool, workspace_id, document_id)
            .await?
            .is_some()
        {
            Err(DocumentStoreError::RevisionConflict)
        } else {
            Err(DocumentStoreError::NotFound)
        };
    }
    get_document(pool, workspace_id, document_id)
        .await?
        .ok_or(DocumentStoreError::NotFound)
}

pub async fn save_artifact_revision(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    title: &str,
    expected_revision: i64,
    working_copy_hash: &str,
    size_bytes: i64,
) -> Result<AiWorkspaceDocument, DocumentStoreError> {
    let filename = format!("{}.md", title.replace(['/', '\\'], "_"));
    let result = sqlx::query(
        "UPDATE ai_workspace_documents SET title = ?, filename = ?, size_bytes = ?, \
         working_copy_revision = working_copy_revision + 1, working_copy_hash = ?, \
         updated_at = datetime('now') \
         WHERE id = ? AND workspace_id = ? AND kind = 'artifact' AND archived_at IS NULL \
           AND working_copy_revision = ?",
    )
    .bind(title.trim())
    .bind(filename)
    .bind(size_bytes)
    .bind(working_copy_hash)
    .bind(document_id)
    .bind(workspace_id)
    .bind(expected_revision)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return if get_document(pool, workspace_id, document_id)
            .await?
            .is_some()
        {
            Err(DocumentStoreError::RevisionConflict)
        } else {
            Err(DocumentStoreError::NotFound)
        };
    }
    get_document(pool, workspace_id, document_id)
        .await?
        .ok_or(DocumentStoreError::NotFound)
}

#[allow(clippy::too_many_arguments)]
pub async fn add_document_version(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    content_md: &str,
    created_by: &str,
    trigger: &str,
    message_id: Option<&str>,
    source_snapshot_json: &str,
) -> Result<AiWorkspaceDocumentVersion, DocumentStoreError> {
    add_document_version_with_summary(
        pool,
        workspace_id,
        document_id,
        content_md,
        created_by,
        trigger,
        "",
        message_id,
        source_snapshot_json,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn add_document_version_with_summary(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    content_md: &str,
    created_by: &str,
    trigger: &str,
    change_summary: &str,
    message_id: Option<&str>,
    source_snapshot_json: &str,
) -> Result<AiWorkspaceDocumentVersion, DocumentStoreError> {
    let mut tx = pool.begin().await?;
    let latest: Option<(i64,)> = sqlx::query_as(
        "SELECT latest_version_no FROM ai_workspace_documents \
         WHERE id = ? AND workspace_id = ? AND kind = 'artifact' AND archived_at IS NULL",
    )
    .bind(document_id)
    .bind(workspace_id)
    .fetch_optional(&mut *tx)
    .await?;
    let next = latest.ok_or(DocumentStoreError::NotFound)?.0 + 1;
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO ai_workspace_document_versions \
         (id, document_id, version_no, content_md, created_by, trigger, change_summary, source_snapshot_json, message_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(document_id)
    .bind(next)
    .bind(content_md)
    .bind(created_by)
    .bind(trigger)
    .bind(change_summary)
    .bind(source_snapshot_json)
    .bind(message_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE ai_workspace_documents SET latest_version_no = ?, updated_at = datetime('now') \
         WHERE id = ? AND workspace_id = ?",
    )
    .bind(next)
    .bind(document_id)
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get_document_version(pool, workspace_id, document_id, &id)
        .await?
        .ok_or(DocumentStoreError::NotFound)
}

pub async fn get_document_version(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    version_id: &str,
) -> Result<Option<AiWorkspaceDocumentVersion>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceDocumentVersion>(
        "SELECT v.* FROM ai_workspace_document_versions v \
         JOIN ai_workspace_documents d ON d.id = v.document_id \
         WHERE v.id = ? AND v.document_id = ? AND d.workspace_id = ?",
    )
    .bind(version_id)
    .bind(document_id)
    .bind(workspace_id)
    .fetch_optional(pool)
    .await
}

pub async fn list_document_versions(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<Vec<AiWorkspaceDocumentVersion>, sqlx::Error> {
    sqlx::query_as::<_, AiWorkspaceDocumentVersion>(
        "SELECT v.* FROM ai_workspace_document_versions v \
         JOIN ai_workspace_documents d ON d.id = v.document_id \
         WHERE v.document_id = ? AND d.workspace_id = ? ORDER BY v.version_no ASC",
    )
    .bind(document_id)
    .bind(workspace_id)
    .fetch_all(pool)
    .await
}

pub async fn restore_document_version(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    version_id: &str,
    created_by: &str,
) -> Result<AiWorkspaceDocumentVersion, DocumentStoreError> {
    let source = get_document_version(pool, workspace_id, document_id, version_id)
        .await?
        .ok_or(DocumentStoreError::NotFound)?;
    add_document_version(
        pool,
        workspace_id,
        document_id,
        &source.content_md,
        created_by,
        "restore",
        source.message_id.as_deref(),
        &source.source_snapshot_json,
    )
    .await
}
