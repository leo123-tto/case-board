use serde::{Deserialize, Serialize};

use crate::db::ai_workspace_documents::{AiWorkspaceDocument, AiWorkspaceDocumentVersion};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceListView {
    #[default]
    All,
    Recent,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListAiWorkspacesInput {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub view: WorkspaceListView,
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAiWorkspaceInput {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateAiWorkspaceInput {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_favorite: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceAddError {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddAiWorkspaceSourcesResult {
    pub added: Vec<AiWorkspaceDocument>,
    pub errors: Vec<SourceAddError>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiWorkspaceDocumentProgress {
    pub workspace_id: String,
    pub document_id: String,
    pub filename: String,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAiWorkspaceArtifactInput {
    pub title: String,
    #[serde(default)]
    pub initial_markdown: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveAiWorkspaceArtifactInput {
    pub title: String,
    pub markdown: String,
    pub expected_revision: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateAiWorkspaceArtifactVersionInput {
    pub trigger: String,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiWorkspaceArtifactContent {
    pub document: AiWorkspaceDocument,
    pub markdown: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiWorkspaceArtifactVersionList {
    pub versions: Vec<AiWorkspaceDocumentVersion>,
}
