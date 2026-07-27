use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager, State};

use super::models::{
    AddAiWorkspaceSourcesResult, AiWorkspaceArtifactContent, AiWorkspaceDocumentProgress,
    AiWorkspaceExportRefreshResult, AiWorkspaceExportWrite, AiWorkspaceExportWriteError,
    CreateAiWorkspaceArtifactInput, CreateAiWorkspaceArtifactVersionInput, CreateAiWorkspaceInput,
    ListAiWorkspacesInput, SaveAiWorkspaceArtifactInput, SourceAddError, UpdateAiWorkspaceInput,
    WorkspaceListView,
};
use crate::ai_workspace::material_processor::clean_workspace_extracted_text;
use crate::db::ai_workspace_chat::{
    self, AiWorkspaceConversation, AiWorkspaceMessage, AiWorkspaceTask,
};
use crate::db::ai_workspace_documents::{self, AiWorkspaceDocument};
use crate::db::ai_workspace_local_exports::{self, AiWorkspaceExportPaths};
use crate::db::ai_workspace_proposals::{self, AiWorkspaceDocumentProposal};
use crate::db::ai_workspaces::{self, AiWorkspace, AiWorkspaceSummary};
use crate::ingest::ocr::OcrContext;

const MAX_TITLE_CHARS: usize = 120;
const MAX_DESCRIPTION_CHARS: usize = 1_000;
const MAX_CONVERSATION_TITLE_CHARS: usize = 80;
const WORKSPACE_TEXT_MAX_BYTES: u64 = 100 * 1024 * 1024;
const WORKSPACE_ARTIFACT_MAX_BYTES: usize = 20 * 1024 * 1024;

fn validate_title(title: &str) -> Result<&str, String> {
    let title = title.trim();
    let chars = title.chars().count();
    if chars == 0 {
        return Err("工作区名称不能为空".into());
    }
    if chars > MAX_TITLE_CHARS {
        return Err(format!("工作区名称不能超过 {MAX_TITLE_CHARS} 个字符"));
    }
    Ok(title)
}

fn validate_description(description: Option<&str>) -> Result<Option<&str>, String> {
    let description = description.map(str::trim).filter(|value| !value.is_empty());
    if description
        .as_ref()
        .is_some_and(|value| value.chars().count() > MAX_DESCRIPTION_CHARS)
    {
        return Err(format!("工作区说明不能超过 {MAX_DESCRIPTION_CHARS} 个字符"));
    }
    Ok(description)
}

fn validate_conversation_title(title: &str) -> Result<&str, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("对话名称不能为空".into());
    }
    if title.chars().count() > MAX_CONVERSATION_TITLE_CHARS {
        return Err(format!(
            "对话名称不能超过 {MAX_CONVERSATION_TITLE_CHARS} 个字符"
        ));
    }
    Ok(title)
}

pub(crate) async fn list_ai_workspaces_impl(
    pool: &SqlitePool,
    input: ListAiWorkspacesInput,
) -> Result<Vec<AiWorkspaceSummary>, String> {
    let query = input
        .query
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    ai_workspaces::list_workspaces(
        pool,
        query,
        matches!(input.view, WorkspaceListView::Recent),
        input.include_archived,
    )
    .await
    .map_err(|error| error.to_string())
}

pub(crate) async fn create_ai_workspace_impl(
    pool: &SqlitePool,
    input: CreateAiWorkspaceInput,
) -> Result<AiWorkspace, String> {
    let title = validate_title(&input.title)?;
    let description = validate_description(input.description.as_deref())?;
    ai_workspaces::create_workspace(pool, title, description)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn open_ai_workspace_impl(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<AiWorkspace, String> {
    ai_workspaces::touch_workspace_opened(pool, workspace_id)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn update_ai_workspace_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    input: UpdateAiWorkspaceInput,
) -> Result<AiWorkspace, String> {
    let title = input.title.as_deref().map(validate_title).transpose()?;
    let description = input
        .description
        .as_deref()
        .map(|value| validate_description(Some(value)))
        .transpose()?
        .flatten();
    ai_workspaces::update_workspace(
        pool,
        workspace_id,
        title,
        description.map(Some),
        input.is_favorite,
    )
    .await
    .map_err(|error| error.to_string())
}

pub(crate) async fn archive_ai_workspace_impl(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<(), String> {
    ai_workspaces::archive_workspace(pool, workspace_id)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn create_ai_workspace_conversation_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    title: Option<String>,
) -> Result<AiWorkspaceConversation, String> {
    ensure_workspace_exists(pool, workspace_id).await?;
    let (title, title_is_manual) = match title.as_deref() {
        Some(title) => (validate_conversation_title(title)?, true),
        None => ("新对话", false),
    };
    let conversation =
        ai_workspace_chat::create_conversation(pool, workspace_id, title, title_is_manual)
            .await
            .map_err(|error| error.to_string())?;
    ai_workspace_chat::set_last_conversation(pool, workspace_id, &conversation.id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(conversation)
}

pub(crate) async fn ensure_ai_workspace_conversation_impl(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<AiWorkspaceConversation, String> {
    ensure_workspace_exists(pool, workspace_id).await?;
    let workspace = ai_workspaces::get_workspace(pool, workspace_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "工作区不存在".to_string())?;
    if let Some(conversation_id) = workspace.last_conversation_id.as_deref() {
        if let Some(conversation) =
            ai_workspace_chat::get_conversation(pool, workspace_id, conversation_id)
                .await
                .map_err(|error| error.to_string())?
        {
            return Ok(conversation);
        }
    }
    if let Some(conversation) = ai_workspace_chat::list_conversations(pool, workspace_id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .next()
    {
        ai_workspace_chat::set_last_conversation(pool, workspace_id, &conversation.id)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(conversation);
    }
    create_ai_workspace_conversation_impl(pool, workspace_id, None).await
}

pub(crate) async fn rename_ai_workspace_conversation_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    conversation_id: &str,
    title: &str,
) -> Result<AiWorkspaceConversation, String> {
    let title = validate_conversation_title(title)?;
    ai_workspace_chat::rename_conversation(pool, workspace_id, conversation_id, title)
        .await
        .map_err(|error| error.to_string())
}

fn workspace_ocr_context() -> OcrContext {
    let settings = crate::settings::read_settings().unwrap_or_default();
    OcrContext::from_settings(&settings)
}

fn emit_document_progress(
    app: &AppHandle,
    document: &AiWorkspaceDocument,
    status: &str,
    error: Option<String>,
) {
    let _ = app.emit(
        "ai-workspace-document-progress",
        AiWorkspaceDocumentProgress {
            workspace_id: document.workspace_id.clone(),
            document_id: document.id.clone(),
            filename: document.filename.clone(),
            status: status.to_string(),
            error,
        },
    );
}

fn spawn_document_processing(
    app: AppHandle,
    pool: SqlitePool,
    app_data_root: PathBuf,
    document: AiWorkspaceDocument,
) {
    emit_document_progress(&app, &document, "queued", None);
    tokio::spawn(async move {
        emit_document_progress(&app, &document, "processing", None);
        let result = super::material_processor::process_workspace_document(
            &pool,
            &app_data_root,
            &document,
            &workspace_ocr_context(),
        )
        .await;
        match result {
            Ok(stored) => emit_document_progress(
                &app,
                &stored,
                &stored.extraction_status,
                stored.last_error.clone(),
            ),
            Err(error) => {
                let stored = ai_workspace_documents::get_document(
                    &pool,
                    &document.workspace_id,
                    &document.id,
                )
                .await
                .ok()
                .flatten();
                let status = stored
                    .as_ref()
                    .map(|item| item.extraction_status.as_str())
                    .unwrap_or("failed");
                emit_document_progress(&app, &document, status, Some(error));
            }
        }
    });
}

fn canonical_source(path: &str) -> Result<(PathBuf, std::fs::Metadata), String> {
    let path = Path::new(path);
    if !path.is_file() {
        return Err("所选路径不是可读取文件".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("无法解析材料路径: {error}"))?;
    let metadata =
        std::fs::metadata(&canonical).map_err(|error| format!("无法读取材料信息: {error}"))?;
    Ok((canonical, metadata))
}

#[derive(Debug, Default)]
struct CollectedWorkspaceSources {
    files: Vec<PathBuf>,
    errors: Vec<SourceAddError>,
    preferred_export_dir: Option<PathBuf>,
}

fn workspace_source_extension_supported(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some(
            "md" | "markdown"
                | "txt"
                | "html"
                | "htm"
                | "docx"
                | "doc"
                | "rtf"
                | "odt"
                | "ppt"
                | "pptx"
                | "xls"
                | "xlsx"
                | "pdf"
                | "png"
                | "jpg"
                | "jpeg"
                | "webp"
                | "bmp"
                | "tiff"
                | "tif"
        )
    )
}

fn hidden_path_entry(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

fn collect_directory_files(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    errors: &mut Vec<SourceAddError>,
) {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(SourceAddError {
                path: directory.to_string_lossy().into_owned(),
                error: format!("无法读取材料文件夹: {error}"),
            });
            return;
        }
    };
    let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        if hidden_path_entry(&path) {
            continue;
        }
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                errors.push(SourceAddError {
                    path: path.to_string_lossy().into_owned(),
                    error: format!("无法读取材料类型: {error}"),
                });
                continue;
            }
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_directory_files(&path, files, errors);
        } else if file_type.is_file() && workspace_source_extension_supported(&path) {
            match path.canonicalize() {
                Ok(canonical) => files.push(canonical),
                Err(error) => errors.push(SourceAddError {
                    path: path.to_string_lossy().into_owned(),
                    error: format!("无法解析材料路径: {error}"),
                }),
            }
        }
    }
}

fn collect_workspace_source_paths(paths: Vec<String>) -> CollectedWorkspaceSources {
    let mut collected = CollectedWorkspaceSources::default();
    for source_path in paths {
        let path = Path::new(&source_path);
        let canonical = match path.canonicalize() {
            Ok(canonical) => canonical,
            Err(_) => {
                collected.errors.push(SourceAddError {
                    path: source_path,
                    error: "所选路径不是可读取文件".to_string(),
                });
                continue;
            }
        };
        if canonical.is_file() {
            collected.files.push(canonical);
            continue;
        }
        if !canonical.is_dir() {
            collected.errors.push(SourceAddError {
                path: source_path,
                error: "所选路径不是可读取文件或文件夹".to_string(),
            });
            continue;
        }
        let before = collected.files.len();
        collect_directory_files(&canonical, &mut collected.files, &mut collected.errors);
        if collected.files.len() > before {
            collected.preferred_export_dir = Some(canonical);
        } else {
            collected.errors.push(SourceAddError {
                path: source_path,
                error: "文件夹中没有支持的材料文件".to_string(),
            });
        }
    }
    collected.files.sort();
    collected.files.dedup();
    collected
}

fn mime_type_for(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("pdf") => Some("application/pdf"),
        Some("txt" | "md" | "markdown") => Some("text/plain"),
        Some("html" | "htm") => Some("text/html"),
        Some("docx") => {
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        }
        Some("doc") => Some("application/msword"),
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        _ => None,
    }
}

async fn ensure_workspace_exists(pool: &SqlitePool, workspace_id: &str) -> Result<(), String> {
    match ai_workspaces::get_workspace(pool, workspace_id)
        .await
        .map_err(|error| error.to_string())?
    {
        Some(workspace) if workspace.archived_at.is_none() => Ok(()),
        _ => Err("工作区不存在或已归档".to_string()),
    }
}

pub(crate) async fn add_ai_workspace_sources_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    paths: Vec<String>,
) -> Result<AddAiWorkspaceSourcesResult, String> {
    ensure_workspace_exists(pool, workspace_id).await?;
    let collected = collect_workspace_source_paths(paths);
    let mut added = Vec::new();
    let mut errors = collected.errors;
    for canonical in collected.files {
        let source_path = canonical.to_string_lossy().into_owned();
        let result = async {
            let metadata = std::fs::metadata(&canonical)
                .map_err(|error| format!("无法读取材料信息: {error}"))?;
            let filename = canonical
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| "材料文件名不是有效 UTF-8".to_string())?;
            let normalized = canonical.to_string_lossy().to_string();
            ai_workspace_documents::create_source_document(
                pool,
                workspace_id,
                filename,
                &normalized,
                &normalized,
                mime_type_for(&canonical),
                Some(metadata.len() as i64),
            )
            .await
            .map_err(|error| {
                if matches!(error, ai_workspace_documents::DocumentStoreError::Sqlx(ref sqlx_error) if sqlx_error.to_string().contains("UNIQUE constraint")) {
                    "该材料已经在当前工作区中".to_string()
                } else {
                    error.to_string()
                }
            })
        }
        .await;
        match result {
            Ok(document) => added.push(document),
            Err(error) => errors.push(SourceAddError {
                path: source_path,
                error,
            }),
        }
    }
    let preferred_export_dir = collected
        .preferred_export_dir
        .filter(|directory| {
            added.iter().any(|document| {
                document
                    .source_path
                    .as_deref()
                    .is_some_and(|path| Path::new(path).starts_with(directory))
            })
        })
        .map(|directory| directory.to_string_lossy().into_owned());
    if let Some(directory) = preferred_export_dir.as_deref() {
        if let Err(error) =
            ai_workspace_local_exports::set_preferred_export_dir(pool, workspace_id, directory)
                .await
        {
            errors.push(SourceAddError {
                path: directory.to_string(),
                error: format!("记录默认导出文件夹失败: {error}"),
            });
        }
    }
    Ok(AddAiWorkspaceSourcesResult {
        added,
        errors,
        preferred_export_dir,
    })
}

pub(crate) async fn get_ai_workspace_export_paths_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<AiWorkspaceExportPaths, String> {
    ensure_workspace_exists(pool, workspace_id).await?;
    let mut paths = ai_workspace_local_exports::get_export_paths(pool, workspace_id, document_id)
        .await
        .map_err(|error| error.to_string())?;
    if paths
        .preferred_export_dir
        .as_deref()
        .is_some_and(|path| !Path::new(path).is_dir())
    {
        paths.preferred_export_dir = None;
    }
    Ok(paths)
}

pub(crate) async fn record_ai_workspace_export_path_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    format: &str,
    path: &str,
) -> Result<(), String> {
    record_ai_workspace_export_path_with_template_impl(
        pool,
        workspace_id,
        document_id,
        format,
        path,
        None,
    )
    .await
}

pub(crate) async fn record_ai_workspace_export_path_with_template_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    format: &str,
    path: &str,
    word_template: Option<&str>,
) -> Result<(), String> {
    ensure_workspace_exists(pool, workspace_id).await?;
    let canonical = Path::new(path)
        .canonicalize()
        .map_err(|error| format!("无法确认导出文件路径: {error}"))?;
    if !canonical.is_file() {
        return Err("导出路径不是可读取文件".to_string());
    }
    ai_workspace_local_exports::record_export_path_with_template(
        pool,
        workspace_id,
        document_id,
        format,
        canonical.to_string_lossy().as_ref(),
        word_template,
    )
    .await
    .map_err(|error| error.to_string())
}

pub(crate) async fn refresh_ai_workspace_exports_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<AiWorkspaceExportRefreshResult, String> {
    ensure_workspace_exists(pool, workspace_id).await?;
    let document = scoped_document(pool, workspace_id, document_id).await?;
    if document.kind != "artifact" {
        return Err("只有 AI 文稿可以更新导出文件".to_string());
    }
    let content_path = document
        .content_path
        .as_deref()
        .ok_or_else(|| "文稿没有内部 Markdown 路径".to_string())?;
    let records = ai_workspace_local_exports::list_export_records(pool, workspace_id, document_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut written = Vec::new();
    let mut errors = Vec::new();
    for record in records {
        let header_inputs =
            if record.format == "docx" && record.word_template.as_deref() == Some("legal_filing") {
                crate::export::load_editor_header_inputs(pool, &document.title, None).await?
            } else {
                crate::docx_filing::HeaderInputs::default()
            };
        match crate::export::export_editor_document_with_template_to(
            Path::new(content_path),
            &document.title,
            &record.format,
            Path::new(&record.export_path),
            record.word_template.as_deref(),
            &header_inputs,
        )
        .await
        {
            Ok(output) => written.push(AiWorkspaceExportWrite {
                format: record.format,
                path: output.to_string_lossy().into_owned(),
            }),
            Err(error) => errors.push(AiWorkspaceExportWriteError {
                format: record.format,
                path: record.export_path,
                error,
            }),
        }
    }
    Ok(AiWorkspaceExportRefreshResult { written, errors })
}

async fn scoped_document(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<AiWorkspaceDocument, String> {
    ai_workspace_documents::get_document(pool, workspace_id, document_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "工作区文档不存在".to_string())
}

fn readable_document_path(
    document: &AiWorkspaceDocument,
    source_bytes: bool,
) -> Result<PathBuf, String> {
    let selected = if document.kind == "artifact" {
        document.content_path.as_deref()
    } else if source_bytes {
        document.source_path.as_deref()
    } else {
        document.extracted_text_path.as_deref()
    };
    let path = selected.ok_or_else(|| {
        if document.kind == "source" && !source_bytes {
            "材料尚未完成文字抽取".to_string()
        } else {
            "文档没有可读取文件".to_string()
        }
    })?;
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err(format!("文档文件不存在: {}", path.display()));
    }
    path.canonicalize()
        .map_err(|error| format!("无法解析文档路径: {error}"))
}

fn validate_artifact_markdown(markdown: &str) -> Result<(), String> {
    if markdown.len() > WORKSPACE_ARTIFACT_MAX_BYTES {
        return Err("文稿正文超过 20MB，无法保存".to_string());
    }
    Ok(())
}

/// Milkdown 把原始 HTML 块视为不可直接编辑的原子节点。AI 偶尔会为落款右对齐输出
/// `<p align="right">…</p>`；在工作区文稿边界把整行 HTML 段落降级为普通 Markdown
/// 段落，保留正文与段落间距。代码围栏里的 HTML 示例保持原样。
fn normalize_editable_artifact_markdown(markdown: &str) -> String {
    static HTML_PARAGRAPH: OnceLock<Regex> = OnceLock::new();
    let html_paragraph = HTML_PARAGRAPH.get_or_init(|| {
        Regex::new(r"(?i)^[ \t]*<p\b[^>\r\n]*>[ \t]*(.*?)[ \t]*</p>[ \t]*$")
            .expect("valid artifact HTML paragraph regex")
    });

    let lines = markdown.lines().collect::<Vec<_>>();
    let mut normalized = Vec::with_capacity(lines.len());
    let mut fence: Option<char> = None;

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let fence_marker = if trimmed.starts_with("```") {
            Some('`')
        } else if trimmed.starts_with("~~~") {
            Some('~')
        } else {
            None
        };
        if let Some(marker) = fence_marker {
            match fence {
                Some(open) if open == marker => fence = None,
                None => fence = Some(marker),
                _ => {}
            }
            normalized.push((*line).to_string());
            continue;
        }

        if fence.is_none() {
            if let Some(captures) = html_paragraph.captures(line) {
                normalized.push(captures[1].trim().to_string());
                if index + 1 < lines.len() && !lines[index + 1].trim().is_empty() {
                    normalized.push(String::new());
                }
                continue;
            }
        }
        normalized.push((*line).to_string());
    }

    let mut result = normalized.join("\n");
    if markdown.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn artifact_filename(title: &str) -> String {
    format!("{}.md", title.replace(['/', '\\'], "_"))
}

fn write_artifact_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "文稿路径缺少父目录".to_string())?;
    std::fs::create_dir_all(parent).map_err(|error| format!("创建文稿目录失败: {error}"))?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| format!("创建文稿临时文件失败: {error}"))?;
        file.write_all(bytes)
            .map_err(|error| format!("写入文稿失败: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("刷新文稿到磁盘失败: {error}"))?;
        std::fs::rename(&temporary, path).map_err(|error| format!("原子替换文稿失败: {error}"))?;
        Ok::<(), String>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub(crate) async fn create_ai_workspace_artifact_impl(
    pool: &SqlitePool,
    app_data_root: &Path,
    workspace_id: &str,
    input: CreateAiWorkspaceArtifactInput,
) -> Result<AiWorkspaceArtifactContent, String> {
    ensure_workspace_exists(pool, workspace_id).await?;
    let title = validate_title(&input.title)?.to_string();
    let markdown = input
        .initial_markdown
        .unwrap_or_else(|| format!("# {title}\n"));
    let markdown = normalize_editable_artifact_markdown(&markdown);
    validate_artifact_markdown(&markdown)?;
    let document_id = uuid::Uuid::new_v4().to_string();
    let path = app_data_root
        .join("ai-workspaces")
        .join(workspace_id)
        .join("artifacts")
        .join(format!("{document_id}.md"));
    write_artifact_atomic(&path, markdown.as_bytes())?;
    let hash = format!("{:x}", Sha256::digest(markdown.as_bytes()));
    let path_string = path.to_string_lossy().to_string();
    let created = ai_workspace_documents::create_artifact_with_initial_version(
        pool,
        &document_id,
        workspace_id,
        &title,
        &artifact_filename(&title),
        &path_string,
        &hash,
        &markdown,
    )
    .await;
    match created {
        Ok(document) => Ok(AiWorkspaceArtifactContent { document, markdown }),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            Err(error.to_string())
        }
    }
}

pub(crate) async fn create_ai_workspace_artifact_from_message_impl(
    pool: &SqlitePool,
    app_data_root: &Path,
    workspace_id: &str,
    message_id: &str,
    title: &str,
) -> Result<AiWorkspaceArtifactContent, String> {
    ensure_workspace_exists(pool, workspace_id).await?;
    let title = validate_title(title)?.to_string();
    let message = ai_workspace_chat::get_message(pool, workspace_id, message_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "AI 消息不存在或不属于当前工作区".to_string())?;
    if message.role != "assistant" || message.content.trim().is_empty() {
        return Err("所选消息没有可保存的 AI 正文".into());
    }
    if let Some(document_id) = message.artifact_document_id.as_deref() {
        return read_ai_workspace_artifact_impl(pool, workspace_id, document_id).await;
    }
    let markdown = normalize_editable_artifact_markdown(&message.content);
    validate_artifact_markdown(&markdown)?;
    let document_id = uuid::Uuid::new_v4().to_string();
    let path = app_data_root
        .join("ai-workspaces")
        .join(workspace_id)
        .join("artifacts")
        .join(format!("{document_id}.md"));
    write_artifact_atomic(&path, markdown.as_bytes())?;
    let hash = format!("{:x}", Sha256::digest(markdown.as_bytes()));
    let source_snapshot_json = if message.citations_json.trim().is_empty() {
        "[]"
    } else {
        message.citations_json.as_str()
    };
    let created = ai_workspace_documents::create_artifact_from_assistant_message(
        pool,
        &document_id,
        workspace_id,
        &title,
        &artifact_filename(&title),
        &path.to_string_lossy(),
        &hash,
        &markdown,
        message_id,
        source_snapshot_json,
    )
    .await;
    match created {
        Ok(document) => Ok(AiWorkspaceArtifactContent { document, markdown }),
        Err(error) => {
            let _ = std::fs::remove_file(&path);
            Err(error.to_string())
        }
    }
}

pub(crate) async fn read_ai_workspace_artifact_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
) -> Result<AiWorkspaceArtifactContent, String> {
    let document = scoped_document(pool, workspace_id, document_id).await?;
    if document.kind != "artifact" {
        return Err("所选文档不是工作区文稿".to_string());
    }
    let path = readable_document_path(&document, false)?;
    let markdown = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| format!("读取文稿失败: {error}"))?;
    let markdown = normalize_editable_artifact_markdown(&markdown);
    Ok(AiWorkspaceArtifactContent { document, markdown })
}

pub(crate) async fn save_ai_workspace_artifact_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    input: SaveAiWorkspaceArtifactInput,
) -> Result<AiWorkspaceArtifactContent, String> {
    let title = validate_title(&input.title)?.to_string();
    let markdown = normalize_editable_artifact_markdown(&input.markdown);
    validate_artifact_markdown(&markdown)?;
    let current = scoped_document(pool, workspace_id, document_id).await?;
    if current.kind != "artifact" {
        return Err("所选文档不是工作区文稿".to_string());
    }
    if current.working_copy_revision != input.expected_revision {
        return Err("文稿已被其他保存更新，请刷新后重试".to_string());
    }
    let path = readable_document_path(&current, false)?;
    let old_bytes = std::fs::read(&path).map_err(|error| format!("读取保存前文稿失败: {error}"))?;
    write_artifact_atomic(&path, markdown.as_bytes())?;
    let hash = format!("{:x}", Sha256::digest(markdown.as_bytes()));
    let saved = ai_workspace_documents::save_artifact_revision(
        pool,
        workspace_id,
        document_id,
        &title,
        input.expected_revision,
        &hash,
        markdown.len() as i64,
    )
    .await;
    match saved {
        Ok(document) => Ok(AiWorkspaceArtifactContent { document, markdown }),
        Err(error) => {
            if let Err(rollback_error) = write_artifact_atomic(&path, &old_bytes) {
                return Err(format!(
                    "{}；且恢复保存前正文失败: {}",
                    error, rollback_error
                ));
            }
            Err(error.to_string())
        }
    }
}

pub(crate) async fn create_ai_workspace_artifact_version_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    input: CreateAiWorkspaceArtifactVersionInput,
) -> Result<ai_workspace_documents::AiWorkspaceDocumentVersion, String> {
    let allowed = [
        "manual",
        "leave",
        "export",
        "ai",
        "before_ai",
        "after_ai",
        "restore",
    ];
    if !allowed.contains(&input.trigger.as_str()) {
        return Err("不支持的版本触发类型".to_string());
    }
    let content = read_ai_workspace_artifact_impl(pool, workspace_id, document_id).await?;
    ai_workspace_documents::add_document_version_with_summary(
        pool,
        workspace_id,
        document_id,
        &content.markdown,
        "user",
        &input.trigger,
        input.summary.as_deref().unwrap_or(""),
        None,
        "[]",
    )
    .await
    .map_err(|error| error.to_string())
}

pub(crate) async fn restore_ai_workspace_artifact_version_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    version_id: &str,
    expected_revision: i64,
) -> Result<AiWorkspaceArtifactContent, String> {
    let version =
        ai_workspace_documents::get_document_version(pool, workspace_id, document_id, version_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "文稿版本不存在".to_string())?;
    let current = scoped_document(pool, workspace_id, document_id).await?;
    let saved = save_ai_workspace_artifact_impl(
        pool,
        workspace_id,
        document_id,
        SaveAiWorkspaceArtifactInput {
            title: current.title,
            markdown: version.content_md,
            expected_revision,
        },
    )
    .await?;
    ai_workspace_documents::add_document_version_with_summary(
        pool,
        workspace_id,
        document_id,
        &saved.markdown,
        "user",
        "restore",
        &format!("恢复版本 v{}", version.version_no),
        None,
        "[]",
    )
    .await
    .map_err(|error| error.to_string())?;
    read_ai_workspace_artifact_impl(pool, workspace_id, document_id).await
}

pub(crate) async fn create_ai_workspace_document_proposal_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    document_id: &str,
    conversation_id: &str,
    message_id: &str,
) -> Result<AiWorkspaceDocumentProposal, String> {
    let document = scoped_document(pool, workspace_id, document_id).await?;
    if document.kind != "artifact" {
        return Err("只能对工作区文稿提出修改建议".into());
    }
    ai_workspace_proposals::create_proposal_from_message(
        pool,
        workspace_id,
        document_id,
        conversation_id,
        message_id,
    )
    .await
    .map_err(|error| error.to_string())
}

pub(crate) async fn resolve_ai_workspace_document_proposal_impl(
    pool: &SqlitePool,
    workspace_id: &str,
    proposal_id: &str,
    action: &str,
    resolved_markdown: Option<String>,
) -> Result<Option<AiWorkspaceArtifactContent>, String> {
    let proposal = ai_workspace_proposals::get_proposal(pool, workspace_id, proposal_id)
        .await
        .map_err(|error| error.to_string())?
        .filter(|proposal| proposal.status == "pending")
        .ok_or_else(|| "修改建议不存在或已经处理".to_string())?;
    if action == "rejected" {
        ai_workspace_proposals::resolve_proposal(pool, workspace_id, proposal_id, "rejected", None)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(None);
    }
    if action != "accepted" {
        return Err("修改建议只能接受或拒绝".into());
    }
    let final_markdown = resolved_markdown.ok_or_else(|| "缺少审阅后的最终正文".to_string())?;
    validate_artifact_markdown(&final_markdown)?;
    let current = scoped_document(pool, workspace_id, &proposal.document_id).await?;
    if current.kind != "artifact"
        || current.working_copy_revision != proposal.base_revision
        || current.working_copy_hash.as_deref().unwrap_or("") != proposal.base_content_hash
    {
        return Err("文稿已在 AI 建议后发生变化，请重新生成修改建议".into());
    }
    let current_content = read_ai_workspace_artifact_impl(pool, workspace_id, &current.id).await?;
    ai_workspace_documents::add_document_version_with_summary(
        pool,
        workspace_id,
        &current.id,
        &current_content.markdown,
        "user",
        "before_ai",
        "应用 AI 修改前",
        proposal.message_id.as_deref(),
        &proposal.source_snapshot_json,
    )
    .await
    .map_err(|error| error.to_string())?;
    let saved = save_ai_workspace_artifact_impl(
        pool,
        workspace_id,
        &current.id,
        SaveAiWorkspaceArtifactInput {
            title: current.title,
            markdown: final_markdown.clone(),
            expected_revision: current.working_copy_revision,
        },
    )
    .await?;
    ai_workspace_documents::add_document_version_with_summary(
        pool,
        workspace_id,
        &saved.document.id,
        &saved.markdown,
        "ai",
        "after_ai",
        &proposal.summary,
        proposal.message_id.as_deref(),
        &proposal.source_snapshot_json,
    )
    .await
    .map_err(|error| error.to_string())?;
    ai_workspace_proposals::resolve_proposal(
        pool,
        workspace_id,
        proposal_id,
        "accepted",
        Some(&final_markdown),
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(Some(saved))
}

#[tauri::command]
pub async fn list_ai_workspaces(
    pool: State<'_, SqlitePool>,
    input: ListAiWorkspacesInput,
) -> Result<Vec<AiWorkspaceSummary>, String> {
    list_ai_workspaces_impl(pool.inner(), input).await
}

#[tauri::command]
pub async fn create_ai_workspace(
    pool: State<'_, SqlitePool>,
    input: CreateAiWorkspaceInput,
) -> Result<AiWorkspace, String> {
    create_ai_workspace_impl(pool.inner(), input).await
}

#[tauri::command]
pub async fn open_ai_workspace(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
) -> Result<AiWorkspace, String> {
    open_ai_workspace_impl(pool.inner(), &workspace_id).await
}

#[tauri::command]
pub async fn update_ai_workspace(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    input: UpdateAiWorkspaceInput,
) -> Result<AiWorkspace, String> {
    update_ai_workspace_impl(pool.inner(), &workspace_id, input).await
}

#[tauri::command]
pub async fn archive_ai_workspace(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
) -> Result<(), String> {
    archive_ai_workspace_impl(pool.inner(), &workspace_id).await
}

#[tauri::command]
pub async fn list_ai_workspace_conversations(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
) -> Result<Vec<AiWorkspaceConversation>, String> {
    ensure_workspace_exists(pool.inner(), &workspace_id).await?;
    ai_workspace_chat::list_conversations(pool.inner(), &workspace_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn ensure_ai_workspace_conversation(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
) -> Result<AiWorkspaceConversation, String> {
    ensure_ai_workspace_conversation_impl(pool.inner(), &workspace_id).await
}

#[tauri::command]
pub async fn create_ai_workspace_conversation(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    title: Option<String>,
) -> Result<AiWorkspaceConversation, String> {
    create_ai_workspace_conversation_impl(pool.inner(), &workspace_id, title).await
}

#[tauri::command]
pub async fn rename_ai_workspace_conversation(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    conversation_id: String,
    title: String,
) -> Result<AiWorkspaceConversation, String> {
    rename_ai_workspace_conversation_impl(pool.inner(), &workspace_id, &conversation_id, &title)
        .await
}

#[tauri::command]
pub async fn select_ai_workspace_conversation(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    conversation_id: String,
) -> Result<(), String> {
    ai_workspace_chat::set_last_conversation(pool.inner(), &workspace_id, &conversation_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn archive_ai_workspace_conversation(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    conversation_id: String,
) -> Result<(), String> {
    ai_workspace_chat::archive_conversation(pool.inner(), &workspace_id, &conversation_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_ai_workspace_messages(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    conversation_id: String,
) -> Result<Vec<AiWorkspaceMessage>, String> {
    ai_workspace_chat::list_messages(pool.inner(), &workspace_id, &conversation_id, None)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn list_ai_workspace_tasks(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    conversation_id: String,
) -> Result<Vec<AiWorkspaceTask>, String> {
    ensure_workspace_exists(pool.inner(), &workspace_id).await?;
    ai_workspace_chat::list_tasks(pool.inner(), &workspace_id, &conversation_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn add_ai_workspace_sources(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    paths: Vec<String>,
) -> Result<AddAiWorkspaceSourcesResult, String> {
    let result = add_ai_workspace_sources_impl(pool.inner(), &workspace_id, paths).await?;
    let app_data_root = crate::db::app_data_dir().map_err(|error| error.to_string())?;
    for document in &result.added {
        spawn_document_processing(
            app.clone(),
            pool.inner().clone(),
            app_data_root.clone(),
            document.clone(),
        );
    }
    Ok(result)
}

#[tauri::command]
pub async fn list_ai_workspace_documents(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
) -> Result<Vec<AiWorkspaceDocument>, String> {
    ensure_workspace_exists(pool.inner(), &workspace_id).await?;
    ai_workspace_documents::list_documents(pool.inner(), &workspace_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_ai_workspace_export_paths(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
) -> Result<AiWorkspaceExportPaths, String> {
    get_ai_workspace_export_paths_impl(pool.inner(), &workspace_id, &document_id).await
}

#[tauri::command]
pub async fn record_ai_workspace_export_path(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
    format: String,
    path: String,
    word_template: Option<String>,
) -> Result<(), String> {
    match word_template.as_deref() {
        Some(template) => {
            record_ai_workspace_export_path_with_template_impl(
                pool.inner(),
                &workspace_id,
                &document_id,
                &format,
                &path,
                Some(template),
            )
            .await
        }
        None => {
            record_ai_workspace_export_path_impl(
                pool.inner(),
                &workspace_id,
                &document_id,
                &format,
                &path,
            )
            .await
        }
    }
}

#[tauri::command]
pub async fn refresh_ai_workspace_exports(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
) -> Result<AiWorkspaceExportRefreshResult, String> {
    refresh_ai_workspace_exports_impl(pool.inner(), &workspace_id, &document_id).await
}

#[tauri::command]
pub async fn retry_ai_workspace_source(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
) -> Result<(), String> {
    let document =
        ai_workspace_documents::reset_source_extraction(pool.inner(), &workspace_id, &document_id)
            .await
            .map_err(|error| error.to_string())?;
    spawn_document_processing(
        app,
        pool.inner().clone(),
        crate::db::app_data_dir().map_err(|error| error.to_string())?,
        document,
    );
    Ok(())
}

#[tauri::command]
pub async fn relink_ai_workspace_source(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
    path: String,
) -> Result<AiWorkspaceDocument, String> {
    let (canonical, metadata) = canonical_source(&path)?;
    let filename = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "材料文件名不是有效 UTF-8".to_string())?;
    let normalized = canonical.to_string_lossy().to_string();
    let document = ai_workspace_documents::relink_source_document(
        pool.inner(),
        &workspace_id,
        &document_id,
        filename,
        &normalized,
        &normalized,
        metadata.len() as i64,
    )
    .await
    .map_err(|error| error.to_string())?;
    spawn_document_processing(
        app,
        pool.inner().clone(),
        crate::db::app_data_dir().map_err(|error| error.to_string())?,
        document.clone(),
    );
    Ok(document)
}

#[tauri::command]
pub async fn archive_ai_workspace_document(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
) -> Result<(), String> {
    ai_workspace_documents::archive_document(pool.inner(), &workspace_id, &document_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn read_ai_workspace_text(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
) -> Result<String, String> {
    let document = scoped_document(pool.inner(), &workspace_id, &document_id).await?;
    let path = readable_document_path(&document, false)?;
    let size = std::fs::metadata(&path)
        .map_err(|error| format!("无法读取文档信息: {error}"))?
        .len();
    if size > WORKSPACE_TEXT_MAX_BYTES {
        return Err("工作区文本超过 100MB，无法直接打开".to_string());
    }
    let text = tokio::fs::read_to_string(path)
        .await
        .map_err(|error| format!("读取工作区文本失败: {error}"))?;
    Ok(clean_workspace_extracted_text(&text))
}

#[tauri::command]
pub async fn allow_ai_workspace_assets(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
) -> Result<(), String> {
    let document = scoped_document(pool.inner(), &workspace_id, &document_id).await?;
    let path = readable_document_path(&document, true)?;
    app.asset_protocol_scope()
        .allow_file(&path)
        .map_err(|error| format!("无法授权工作区文件预览: {error}"))
}

#[tauri::command]
pub async fn read_ai_workspace_file_bytes(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
) -> Result<Vec<u8>, String> {
    let document = scoped_document(pool.inner(), &workspace_id, &document_id).await?;
    let path = readable_document_path(&document, true)?;
    tokio::fs::read(path)
        .await
        .map_err(|error| format!("读取工作区文件失败: {error}"))
}

#[tauri::command]
pub async fn create_ai_workspace_artifact(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    input: CreateAiWorkspaceArtifactInput,
) -> Result<AiWorkspaceArtifactContent, String> {
    create_ai_workspace_artifact_impl(
        pool.inner(),
        &crate::db::app_data_dir().map_err(|error| error.to_string())?,
        &workspace_id,
        input,
    )
    .await
}

#[tauri::command]
pub async fn create_ai_workspace_artifact_from_message(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    message_id: String,
    title: String,
) -> Result<AiWorkspaceArtifactContent, String> {
    create_ai_workspace_artifact_from_message_impl(
        pool.inner(),
        &crate::db::app_data_dir().map_err(|error| error.to_string())?,
        &workspace_id,
        &message_id,
        &title,
    )
    .await
}

#[tauri::command]
pub async fn read_ai_workspace_artifact(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
) -> Result<AiWorkspaceArtifactContent, String> {
    read_ai_workspace_artifact_impl(pool.inner(), &workspace_id, &document_id).await
}

#[tauri::command]
pub async fn save_ai_workspace_artifact(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
    input: SaveAiWorkspaceArtifactInput,
) -> Result<AiWorkspaceArtifactContent, String> {
    save_ai_workspace_artifact_impl(pool.inner(), &workspace_id, &document_id, input).await
}

#[tauri::command]
pub async fn create_ai_workspace_artifact_version(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
    input: CreateAiWorkspaceArtifactVersionInput,
) -> Result<ai_workspace_documents::AiWorkspaceDocumentVersion, String> {
    create_ai_workspace_artifact_version_impl(pool.inner(), &workspace_id, &document_id, input)
        .await
}

#[tauri::command]
pub async fn list_ai_workspace_artifact_versions(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
) -> Result<Vec<ai_workspace_documents::AiWorkspaceDocumentVersion>, String> {
    let document = scoped_document(pool.inner(), &workspace_id, &document_id).await?;
    if document.kind != "artifact" {
        return Err("所选文档不是工作区文稿".to_string());
    }
    ai_workspace_documents::list_document_versions(pool.inner(), &workspace_id, &document_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn restore_ai_workspace_artifact_version(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
    version_id: String,
    expected_revision: i64,
) -> Result<AiWorkspaceArtifactContent, String> {
    restore_ai_workspace_artifact_version_impl(
        pool.inner(),
        &workspace_id,
        &document_id,
        &version_id,
        expected_revision,
    )
    .await
}

#[tauri::command]
pub async fn create_ai_workspace_document_proposal(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
    conversation_id: String,
    message_id: String,
) -> Result<AiWorkspaceDocumentProposal, String> {
    create_ai_workspace_document_proposal_impl(
        pool.inner(),
        &workspace_id,
        &document_id,
        &conversation_id,
        &message_id,
    )
    .await
}

#[tauri::command]
pub async fn list_ai_workspace_document_proposals(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    document_id: String,
) -> Result<Vec<AiWorkspaceDocumentProposal>, String> {
    ai_workspace_proposals::list_pending_proposals(pool.inner(), &workspace_id, &document_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn resolve_ai_workspace_document_proposal(
    pool: State<'_, SqlitePool>,
    workspace_id: String,
    proposal_id: String,
    action: String,
    resolved_markdown: Option<String>,
) -> Result<Option<AiWorkspaceArtifactContent>, String> {
    resolve_ai_workspace_document_proposal_impl(
        pool.inner(),
        &workspace_id,
        &proposal_id,
        &action,
        resolved_markdown,
    )
    .await
}
