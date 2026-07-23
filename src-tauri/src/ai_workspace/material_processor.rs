use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::db::ai_workspace_documents::{self, AiWorkspaceDocument, DocumentChunkInput};
use crate::embedding::index::chunk_text;
use crate::ingest::extractor::extract_text_with_ocr;
use crate::ingest::ocr::OcrContext;
use crate::ingest::ocr_throttle::{global_submit_throttle, might_hit_mineru};

const WORKSPACE_CHUNK_CHARS: usize = 1_200;
static WORKSPACE_PROCESSING_SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// 清理 OCR 服务夹带的图片定位标记，只保留可供阅读和检索的正文。
///
/// MinerU 等文档解析后端会在 Markdown 中返回 `<div><img ...></div>` 或
/// `![Image](imgs/...)`。这些引用指向 OCR 临时压缩包里的图片，在 CaseBoard 的派生文本中
/// 既无法访问，也会以代码形式污染阅读和 AI 检索。这里只移除图片及其布局容器，不做通用
/// HTML 清洗，避免误伤正文里的比较符号、法条内容或用户自己的代码片段。
pub(crate) fn clean_workspace_extracted_text(text: &str) -> String {
    static HTML_IMAGE: OnceLock<Regex> = OnceLock::new();
    static HTML_IMAGE_LAYOUT: OnceLock<Regex> = OnceLock::new();
    static MARKDOWN_IMAGE_LINE: OnceLock<Regex> = OnceLock::new();

    let without_html_images = HTML_IMAGE
        .get_or_init(|| Regex::new(r"(?is)<img\b[^>]*>").expect("valid OCR image regex"))
        .replace_all(text, "");
    let without_layout = HTML_IMAGE_LAYOUT
        .get_or_init(|| {
            Regex::new(r"(?is)</?(?:div|figure|picture)\b[^>]*>")
                .expect("valid OCR image layout regex")
        })
        .replace_all(&without_html_images, "\n");
    let without_markdown_images = MARKDOWN_IMAGE_LINE
        .get_or_init(|| {
            Regex::new(r"(?im)^[ \t]*!\[[^\]\r\n]*\]\([^\r\n)]*\)[ \t]*$")
                .expect("valid OCR markdown image regex")
        })
        .replace_all(&without_layout, "");

    let mut cleaned = String::with_capacity(without_markdown_images.len());
    let mut blank_lines = 0usize;
    for line in without_markdown_images.lines() {
        if line.trim().is_empty() {
            blank_lines += 1;
            if blank_lines > 2 {
                continue;
            }
        } else {
            blank_lines = 0;
        }
        cleaned.push_str(line.trim_end());
        cleaned.push('\n');
    }
    cleaned.trim().to_string()
}

fn derived_text_path(app_data_root: &Path, document: &AiWorkspaceDocument) -> PathBuf {
    app_data_root
        .join("ai-workspaces")
        .join(&document.workspace_id)
        .join("materials")
        .join(format!("{}.md", document.id))
}

async fn write_derived_text(path: &Path, text: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "派生文本路径无父目录".to_string())?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("创建派生文本目录失败: {error}"))?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let cleaned = clean_workspace_extracted_text(text);
    tokio::fs::write(&temporary, cleaned)
        .await
        .map_err(|error| format!("写入派生文本失败: {error}"))?;
    if let Err(error) = tokio::fs::rename(&temporary, path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(format!("保存派生文本失败: {error}"));
    }
    Ok(())
}

fn build_chunks(text: &str) -> Vec<DocumentChunkInput> {
    let cleaned = clean_workspace_extracted_text(text);
    chunk_text(&cleaned, WORKSPACE_CHUNK_CHARS)
        .into_iter()
        .enumerate()
        .map(|(ordinal, content)| DocumentChunkInput {
            ordinal: ordinal as i64,
            page_no: None,
            section_label: None,
            content_hash: format!("{:x}", Sha256::digest(content.as_bytes())),
            content,
        })
        .collect()
}

pub async fn process_workspace_document(
    pool: &SqlitePool,
    app_data_root: &Path,
    document: &AiWorkspaceDocument,
    ocr_ctx: &OcrContext,
) -> Result<AiWorkspaceDocument, String> {
    let _permit = WORKSPACE_PROCESSING_SLOT
        .acquire()
        .await
        .map_err(|_| "工作区材料处理队列已关闭".to_string())?;
    ai_workspace_documents::mark_extraction_started(pool, &document.workspace_id, &document.id)
        .await
        .map_err(|error| error.to_string())?;

    let source = document
        .source_path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| "材料没有源文件路径".to_string())?;
    if !source.is_file() {
        let error = format!("材料源文件不存在: {}", source.display());
        ai_workspace_documents::fail_extraction(
            pool,
            &document.workspace_id,
            &document.id,
            &error,
            true,
        )
        .await
        .map_err(|persist_error| format!("{error}；记录状态失败: {persist_error}"))?;
        return Err(error);
    }

    if ocr_ctx.cloud_enabled && might_hit_mineru(&document.filename) {
        global_submit_throttle().acquire().await;
    }

    let processing = async {
        let extracted = extract_text_with_ocr(&source, &document.filename, ocr_ctx).await?;
        let output = derived_text_path(app_data_root, document);
        write_derived_text(&output, &extracted.text_md).await?;
        let chunks = build_chunks(&extracted.text_md);
        ai_workspace_documents::replace_document_chunks(
            pool,
            &document.workspace_id,
            &document.id,
            &chunks,
        )
        .await
        .map_err(|error| error.to_string())?;
        let output_str = output.to_string_lossy().to_string();
        ai_workspace_documents::finish_extraction(
            pool,
            &document.workspace_id,
            &document.id,
            &output_str,
            &extracted.quality_status,
        )
        .await
        .map_err(|error| error.to_string())?;
        Ok::<(), String>(())
    }
    .await;

    if let Err(error) = processing {
        let short = crate::feedback::sanitize_paths(&error)
            .chars()
            .take(500)
            .collect::<String>();
        ai_workspace_documents::fail_extraction(
            pool,
            &document.workspace_id,
            &document.id,
            &short,
            false,
        )
        .await
        .map_err(|persist_error| format!("{error}；记录状态失败: {persist_error}"))?;
        return Err(error);
    }

    ai_workspace_documents::get_document(pool, &document.workspace_id, &document.id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "工作区材料不存在".to_string())
}
