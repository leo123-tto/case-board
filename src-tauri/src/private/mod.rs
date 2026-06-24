//! Private feature stubs for the public build.
//!
//! The private repository registers these commands from shared `lib.rs`, so the
//! public repository keeps matching symbols for compilation. Implementations
//! intentionally return an error and no private feature code is shipped here.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::BTreeMap;

fn private_unavailable() -> String {
    "该功能不包含在公开版中".to_string()
}

#[tauri::command]
pub async fn telemetry_get(
    _base: String,
    _key: String,
    _path: String,
    _range_start: u32,
    _range_end: u32,
) -> Result<String, String> {
    Err(private_unavailable())
}

#[tauri::command]
pub async fn reset_yuandian_credits(_pool: tauri::State<'_, SqlitePool>) -> Result<u64, String> {
    Err(private_unavailable())
}

#[derive(Debug, Serialize)]
pub struct DiligenceScanFile {
    path: String,
    relative_path: String,
    extension: String,
    size_bytes: u64,
    modified_at: Option<String>,
    source_archive: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DiligenceScanResult {
    folder_path: String,
    workspace_path: String,
    total_files: usize,
    returned_files: usize,
    truncated: bool,
    by_extension: BTreeMap<String, usize>,
    extracted_archives: usize,
    archive_warnings: Vec<String>,
    files: Vec<DiligenceScanFile>,
}

#[derive(Debug, Serialize)]
pub struct DiligenceArtifactResult {
    path: String,
    workspace_path: String,
}

#[derive(Debug, Deserialize)]
pub struct DiligenceAuditItemInput {
    id: String,
    title: String,
    requirement: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DiligenceAuditMatch {
    item_id: String,
    matched_files: Vec<DiligenceMatchedFile>,
    note: String,
}

#[derive(Debug, Serialize)]
pub struct DiligenceMatchedFile {
    path: String,
    relative_path: String,
    extension: String,
    size_bytes: u64,
    modified_at: Option<String>,
    source_archive: Option<String>,
    score: i32,
    reason: String,
}

#[derive(Debug, Deserialize)]
pub struct DiligenceInferFileHint {
    relative_path: String,
    extension: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiligenceSubjectHint {
    name: String,
    role: String,
    confidence: f32,
    reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiligenceConfirmationQuestion {
    id: String,
    question: String,
    options: Vec<String>,
    reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DiligenceProjectInference {
    target_name: Option<String>,
    target_confidence: f32,
    deal_type: Option<String>,
    base_date: Option<String>,
    project_summary: String,
    subjects: Vec<DiligenceSubjectHint>,
    confirmation_questions: Vec<DiligenceConfirmationQuestion>,
    suggested_next_actions: Vec<String>,
}

#[tauri::command]
pub async fn diligence_scan_folder(_folder_path: String) -> Result<DiligenceScanResult, String> {
    Err(private_unavailable())
}

#[tauri::command]
pub async fn diligence_infer_project_context(
    _background: String,
    _file_hints: Vec<DiligenceInferFileHint>,
) -> Result<DiligenceProjectInference, String> {
    Err(private_unavailable())
}

#[tauri::command]
pub async fn diligence_write_markdown_artifact(
    _project_path: String,
    _kind: String,
    _title: String,
    _content: String,
) -> Result<DiligenceArtifactResult, String> {
    Err(private_unavailable())
}

#[tauri::command]
pub async fn diligence_write_docx_artifact(
    _project_path: String,
    _kind: String,
    _title: String,
    _content: String,
) -> Result<DiligenceArtifactResult, String> {
    Err(private_unavailable())
}

#[tauri::command]
pub async fn diligence_deep_audit(
    _project_path: String,
    _items: Vec<DiligenceAuditItemInput>,
) -> Result<Vec<DiligenceAuditMatch>, String> {
    Err(private_unavailable())
}
