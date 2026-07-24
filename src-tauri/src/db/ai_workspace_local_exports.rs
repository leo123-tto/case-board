use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AiWorkspaceExportPaths {
    pub preferred_export_dir: Option<String>,
    pub docx_path: Option<String>,
    pub docx_word_template: Option<String>,
    pub html_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AiWorkspaceExportRecord {
    pub format: String,
    pub export_path: String,
    pub word_template: Option<String>,
}

fn normalize_word_template(
    format: &str,
    word_template: Option<&str>,
) -> Result<Option<&'static str>, sqlx::Error> {
    match format {
        "docx" => {
            let template = match word_template {
                Some(value) => {
                    crate::docx_filing::WordTemplate::parse(value).map_err(sqlx::Error::Protocol)?
                }
                None => crate::docx_filing::WordTemplate::default(),
            };
            Ok(Some(template.as_str()))
        }
        "html" if word_template.is_some() => Err(sqlx::Error::Protocol(
            "HTML 导出不得记录 Word 模板".to_string(),
        )),
        "html" => Ok(None),
        _ => Err(sqlx::Error::Protocol(
            "导出格式仅支持 docx 或 html".to_string(),
        )),
    }
}

async fn ensure_document_belongs_to_workspace(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<(), sqlx::Error> {
    let exists: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM ai_workspace_documents \
         WHERE id = ? AND workspace_id = ? AND kind = 'artifact' AND archived_at IS NULL",
    )
    .bind(document_id)
    .bind(workspace_id)
    .fetch_optional(pool)
    .await?;
    exists.map(|_| ()).ok_or(sqlx::Error::RowNotFound)
}

pub async fn set_preferred_export_dir(
    pool: &SqlitePool,
    workspace_id: &str,
    path: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO ai_workspace_local_preferences \
         (workspace_id, preferred_export_dir) VALUES (?, ?) \
         ON CONFLICT(workspace_id) DO UPDATE SET \
           preferred_export_dir = excluded.preferred_export_dir, \
           updated_at = datetime('now')",
    )
    .bind(workspace_id)
    .bind(path)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_preferred_export_dir(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT preferred_export_dir FROM ai_workspace_local_preferences WHERE workspace_id = ?",
    )
    .bind(workspace_id)
    .fetch_optional(pool)
    .await
}

pub async fn record_export_path(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    format: &str,
    path: &str,
) -> Result<(), sqlx::Error> {
    record_export_path_with_template(pool, workspace_id, document_id, format, path, None).await
}

pub async fn record_export_path_with_template(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    format: &str,
    path: &str,
    word_template: Option<&str>,
) -> Result<(), sqlx::Error> {
    let word_template = normalize_word_template(format, word_template)?;
    ensure_document_belongs_to_workspace(pool, workspace_id, document_id).await?;
    sqlx::query(
        "INSERT INTO ai_workspace_document_exports \
         (document_id, format, export_path, word_template) VALUES (?, ?, ?, ?) \
         ON CONFLICT(document_id, format) DO UPDATE SET \
           export_path = excluded.export_path, \
           word_template = excluded.word_template, \
           updated_at = datetime('now')",
    )
    .bind(document_id)
    .bind(format)
    .bind(path)
    .bind(word_template)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_export_paths(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    Ok(list_export_records(pool, workspace_id, document_id)
        .await?
        .into_iter()
        .map(|record| (record.format, record.export_path))
        .collect())
}

pub async fn list_export_records(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<Vec<AiWorkspaceExportRecord>, sqlx::Error> {
    ensure_document_belongs_to_workspace(pool, workspace_id, document_id).await?;
    sqlx::query_as(
        "SELECT format, export_path, word_template FROM ai_workspace_document_exports \
         WHERE document_id = ? ORDER BY format",
    )
    .bind(document_id)
    .fetch_all(pool)
    .await
}

pub async fn get_export_paths(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<AiWorkspaceExportPaths, sqlx::Error> {
    let mut state = AiWorkspaceExportPaths {
        preferred_export_dir: get_preferred_export_dir(pool, workspace_id).await?,
        ..AiWorkspaceExportPaths::default()
    };
    for record in list_export_records(pool, workspace_id, document_id).await? {
        match record.format.as_str() {
            "docx" => {
                state.docx_path = Some(record.export_path);
                state.docx_word_template = record.word_template;
            }
            "html" => state.html_path = Some(record.export_path),
            _ => {}
        }
    }
    Ok(state)
}
