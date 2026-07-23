use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};

use crate::ai_workspace::material_processor::clean_workspace_extracted_text;
use crate::db::ai_workspace_documents;
use crate::embedding::index::chunk_text;

const MAX_RESULTS: usize = 6;
const MAX_EXCERPT_CHARS: usize = 1_200;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct WorkspaceReference {
    pub document_id: String,
    #[serde(default)]
    pub page_no: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedWorkspaceSource {
    pub document_id: String,
    pub title: String,
    pub page_no: Option<i64>,
    pub excerpt: String,
    pub content_hash: String,
    pub score: f32,
    pub selection: String,
    pub source_missing: bool,
}

#[derive(Debug, Clone, FromRow)]
struct CandidateRow {
    document_id: String,
    title: String,
    page_no: Option<i64>,
    content: String,
    content_hash: String,
    missing: i64,
}

fn truncate_chars(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn tokens(text: &str) -> HashSet<String> {
    let mut result = HashSet::new();
    let normalized: Vec<char> = text
        .to_lowercase()
        .chars()
        .filter(|character| !character.is_whitespace() && !character.is_ascii_punctuation())
        .collect();
    for window in normalized.windows(2) {
        result.insert(window.iter().collect());
    }
    for word in text
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| word.chars().count() >= 2)
    {
        result.insert(word.to_lowercase());
    }
    result
}

fn keyword_score(query: &HashSet<String>, content: &str) -> f32 {
    if query.is_empty() {
        return 0.0;
    }
    let content_tokens = tokens(content);
    query.intersection(&content_tokens).count() as f32 / query.len() as f32
}

async fn source_candidates(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Vec<CandidateRow>, String> {
    let mut rows = sqlx::query_as::<_, CandidateRow>(
        "SELECT d.id AS document_id, d.title, c.page_no, c.content, c.content_hash, d.missing \
         FROM ai_workspace_documents d \
         JOIN ai_workspace_document_chunks c ON c.document_id = d.id \
         WHERE d.workspace_id = ? AND d.kind = 'source' AND d.archived_at IS NULL \
           AND d.extraction_status IN ('ready', 'review') \
         ORDER BY d.updated_at DESC, c.ordinal ASC",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    for row in &mut rows {
        row.content = clean_workspace_extracted_text(&row.content);
        row.content_hash = format!("{:x}", Sha256::digest(row.content.as_bytes()));
    }
    Ok(rows)
}

async fn artifact_candidates(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Vec<CandidateRow>, String> {
    let documents = ai_workspace_documents::list_documents(pool, workspace_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut result = Vec::new();
    for document in documents
        .into_iter()
        .filter(|document| document.kind == "artifact")
    {
        let Some(path) = document.content_path.as_deref() else {
            continue;
        };
        let Ok(markdown) = tokio::fs::read_to_string(path).await else {
            continue;
        };
        let mut chunks = chunk_text(&markdown, MAX_EXCERPT_CHARS);
        if chunks.is_empty() {
            chunks.push("（当前文稿为空，可按用户要求从头起草。）".to_string());
        }
        for content in chunks {
            result.push(CandidateRow {
                document_id: document.id.clone(),
                title: document.title.clone(),
                page_no: None,
                content_hash: format!("{:x}", Sha256::digest(content.as_bytes())),
                content,
                missing: 0,
            });
        }
    }
    Ok(result)
}

pub async fn retrieve_workspace_context(
    pool: &SqlitePool,
    workspace_id: &str,
    query: &str,
    manual: &[WorkspaceReference],
) -> Result<Vec<RetrievedWorkspaceSource>, String> {
    let mut candidates = source_candidates(pool, workspace_id).await?;
    candidates.extend(artifact_candidates(pool, workspace_id).await?);
    let mut by_document: HashMap<&str, Vec<&CandidateRow>> = HashMap::new();
    for candidate in &candidates {
        by_document
            .entry(candidate.document_id.as_str())
            .or_default()
            .push(candidate);
    }

    let mut results = Vec::new();
    let mut seen = HashSet::new();
    for reference in manual {
        let scoped =
            ai_workspace_documents::get_document(pool, workspace_id, &reference.document_id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "引用的工作区文档不存在或不属于当前工作区".to_string())?;
        let matching = by_document
            .get(reference.document_id.as_str())
            .into_iter()
            .flatten()
            .filter(|candidate| {
                reference.page_no.is_none() || candidate.page_no == reference.page_no
            });
        let mut found = false;
        for candidate in matching {
            found = true;
            let key = (
                candidate.document_id.clone(),
                candidate.page_no,
                candidate.content_hash.clone(),
            );
            if seen.insert(key) {
                results.push(RetrievedWorkspaceSource {
                    document_id: candidate.document_id.clone(),
                    title: scoped.title.clone(),
                    page_no: candidate.page_no,
                    excerpt: truncate_chars(&candidate.content, MAX_EXCERPT_CHARS),
                    content_hash: candidate.content_hash.clone(),
                    score: 1.0,
                    selection: "manual".to_string(),
                    source_missing: candidate.missing == 1,
                });
            }
            if results.len() >= MAX_RESULTS {
                return Ok(results);
            }
        }
        if !found {
            return Err("引用的文档或指定页没有可用文本".to_string());
        }
    }

    let query_tokens = tokens(query);
    let mut automatic: Vec<(f32, &CandidateRow)> = candidates
        .iter()
        .map(|candidate| (keyword_score(&query_tokens, &candidate.content), candidate))
        .filter(|(score, _)| *score > 0.0)
        .collect();
    automatic.sort_by(|(left, _), (right, _)| {
        right.partial_cmp(left).unwrap_or(std::cmp::Ordering::Equal)
    });
    for (score, candidate) in automatic {
        let key = (
            candidate.document_id.clone(),
            candidate.page_no,
            candidate.content_hash.clone(),
        );
        if !seen.insert(key) {
            continue;
        }
        results.push(RetrievedWorkspaceSource {
            document_id: candidate.document_id.clone(),
            title: candidate.title.clone(),
            page_no: candidate.page_no,
            excerpt: truncate_chars(&candidate.content, MAX_EXCERPT_CHARS),
            content_hash: candidate.content_hash.clone(),
            score,
            selection: "auto".to_string(),
            source_missing: candidate.missing == 1,
        });
        if results.len() >= MAX_RESULTS {
            break;
        }
    }
    Ok(results)
}
