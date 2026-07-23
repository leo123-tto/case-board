//! 派生工作区文件工具。
//!
//! 对 Agent 暴露 document id，不暴露任意路径。案件源材料和独立工作区 source 只能读取
//! 已抽取文本；创建、覆盖、改名、复制、归档只作用于 AppData 内派生文稿或数据库软归档。

use std::io::Write;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::ai_workspace::models::{CreateAiWorkspaceArtifactInput, SaveAiWorkspaceArtifactInput};

use super::{require_str, Tool, ToolContext, ToolError, ToolResult};

const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
const MAX_WRITE_BYTES: usize = 20 * 1024 * 1024;

macro_rules! workspace_tool_description {
    ($purpose:literal) => {
        concat!(
            $purpose,
            "\n\n【权限边界】本工具只在当前聊天已经绑定的案件工作区或独立事务工作区内生效。模型只能使用 list_workspace_files 返回的 document_id，不能传入、拼接或猜测磁盘路径。案件原始文件以及独立工作区导入的 source 文件始终只读：读取时只返回 CaseBoard 在 AppData 中生成的 Markdown/抽取文本，绝不把写入、改名、移动或删除操作落到用户原文件。\n\n【派生文稿权限】kind=artifact 且 writable=true 的文稿属于可恢复的派生工作区，允许按用户任务创建、编辑、改名、复制和软归档。所有写入都由 Rust 宿主校验 document_id、所属工作区、文档类型和 AppData 路径；Sidecar 与模型都没有任意文件系统权限。归档只改变 CaseBoard 中的可见状态，不删除原文件。\n\n【使用方法】先调用 list_workspace_files 取得真实清单，再按 document_id 操作；不要根据标题虚构 id。读取原始材料后需要改写时，应复制或新建一份 artifact，而不是尝试覆盖 source。用户明确要求“保存下来、形成报告、输出文件”时，必须真实调用 create_workspace_file 或适当的写作工具，不得只在回答里声称已保存。工具失败时应如实说明原因，不要绕过边界或反复猜路径。"
        )
    };
}

enum WorkspaceScope<'a> {
    Case(&'a str),
    Independent(String),
}

#[derive(Default)]
struct WorkspaceEditPolicy {
    editing_document_id: Option<String>,
    allow_new_workspace_file: bool,
}

async fn workspace_edit_policy(ctx: &ToolContext<'_>) -> Result<WorkspaceEditPolicy, ToolError> {
    if ctx.case_id.is_some() {
        return Ok(WorkspaceEditPolicy::default());
    }
    let Some(message_id) = ctx.message_id else {
        return Ok(WorkspaceEditPolicy::default());
    };
    let input_json = sqlx::query_scalar::<_, String>(
        "SELECT input_json FROM ai_workspace_tasks \
         WHERE assistant_message_id = ? ORDER BY created_at DESC LIMIT 1",
    )
    .bind(message_id)
    .fetch_optional(ctx.pool)
    .await?;
    let Some(input_json) = input_json else {
        return Ok(WorkspaceEditPolicy::default());
    };
    let value: Value = serde_json::from_str(&input_json)
        .map_err(|error| ToolError::Runtime(format!("读取文稿编辑边界失败:{error}")))?;
    Ok(WorkspaceEditPolicy {
        editing_document_id: value
            .get("editing_document_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        allow_new_workspace_file: value
            .get("allow_new_workspace_file")
            .and_then(|value| value.as_bool().or_else(|| value.as_i64().map(|n| n != 0)))
            .unwrap_or(false),
    })
}

async fn require_new_workspace_file_permission(ctx: &ToolContext<'_>) -> Result<(), ToolError> {
    let policy = workspace_edit_policy(ctx).await?;
    if let Some(document_id) = policy
        .editing_document_id
        .filter(|_| !policy.allow_new_workspace_file)
    {
        return Err(ToolError::InvalidArgs(format!(
            "当前编辑目标是 document_id={document_id}；用户没有要求另存或新建副本，请读取后用 write_workspace_file 原位更新该文稿"
        )));
    }
    Ok(())
}

async fn require_editing_target(ctx: &ToolContext<'_>, document_id: &str) -> Result<(), ToolError> {
    let policy = workspace_edit_policy(ctx).await?;
    if let Some(target) = policy.editing_document_id {
        if !policy.allow_new_workspace_file && target != document_id {
            return Err(ToolError::InvalidArgs(format!(
                "当前编辑目标是 document_id={target}，不得把本轮修改写到其它文稿"
            )));
        }
    }
    Ok(())
}

async fn resolve_scope<'a>(ctx: &'a ToolContext<'_>) -> Result<WorkspaceScope<'a>, ToolError> {
    if let Some(case_id) = ctx.case_id {
        return Ok(WorkspaceScope::Case(case_id));
    }
    let message_id = ctx
        .message_id
        .ok_or_else(|| ToolError::Runtime("当前对话没有派生工作区边界".into()))?;
    let workspace_id = sqlx::query_scalar::<_, String>(
        "SELECT c.workspace_id FROM ai_workspace_messages m \
         JOIN ai_workspace_conversations c ON c.id = m.conversation_id \
         WHERE m.id = ? LIMIT 1",
    )
    .bind(message_id)
    .fetch_optional(ctx.pool)
    .await?
    .ok_or_else(|| ToolError::Runtime("当前消息不属于可管理的独立工作区".into()))?;
    Ok(WorkspaceScope::Independent(workspace_id))
}

fn safe_title(value: &str) -> Result<String, ToolError> {
    let title: String = value
        .trim()
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' | '\t' => '_',
            other => other,
        })
        .take(120)
        .collect();
    if title.is_empty() {
        Err(ToolError::InvalidArgs("title 不能为空".into()))
    } else {
        Ok(title)
    }
}

fn read_bounded(path: &Path) -> Result<String, ToolError> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(ToolError::Runtime("工作区文本不存在".into()));
    }
    if metadata.len() > MAX_READ_BYTES {
        return Err(ToolError::Runtime(
            "工作区文本超过 10MB，请缩小读取范围".into(),
        ));
    }
    std::fs::read_to_string(path).map_err(ToolError::Io)
}

fn assert_appdata_file(path: &Path) -> Result<PathBuf, ToolError> {
    let root = crate::db::app_data_dir().map_err(|error| ToolError::Runtime(error.to_string()))?;
    let root = root.canonicalize().map_err(ToolError::Io)?;
    let path = path.canonicalize().map_err(ToolError::Io)?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(ToolError::Runtime(
            "拒绝写入：目标不是 CaseBoard AppData 内的派生文稿".into(),
        ));
    }
    Ok(path)
}

fn write_atomic(path: &Path, content: &str) -> Result<(), ToolError> {
    if content.len() > MAX_WRITE_BYTES {
        return Err(ToolError::InvalidArgs("正文超过 20MB".into()));
    }
    let path = assert_appdata_file(path)?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, &path)?;
        Ok::<(), std::io::Error>(())
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(ToolError::Io(error));
    }
    Ok(())
}

async fn list_files(ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
    let files = match resolve_scope(ctx).await? {
        WorkspaceScope::Case(case_id) => sqlx::query_as::<_, (String, String, bool, String)>(
            "SELECT id, filename, is_ai_artifact, COALESCE(category, '') FROM documents \
             WHERE case_id = ? AND deleted_at IS NULL ORDER BY is_ai_artifact DESC, filename",
        )
        .bind(case_id)
        .fetch_all(ctx.pool)
        .await?
        .into_iter()
        .map(|(id, name, derived, category)| {
            json!({"document_id": id, "name": name, "kind": if derived {"artifact"} else {"source"}, "category": category, "writable": derived})
        })
        .collect::<Vec<_>>(),
        WorkspaceScope::Independent(workspace_id) => {
            crate::db::ai_workspace_documents::list_documents(ctx.pool, &workspace_id)
                .await?
                .into_iter()
                .map(|document| {
                    json!({"document_id": document.id, "name": document.title, "kind": document.kind, "status": document.extraction_status, "writable": document.kind == "artifact"})
                })
                .collect::<Vec<_>>()
        }
    };
    Ok(ToolResult::plain(
        serde_json::to_string_pretty(&files)
            .map_err(|error| ToolError::Runtime(error.to_string()))?,
    ))
}

async fn read_file(ctx: &ToolContext<'_>, document_id: &str) -> Result<String, ToolError> {
    match resolve_scope(ctx).await? {
        WorkspaceScope::Case(case_id) => {
            let row = sqlx::query_as::<_, (String, bool, String, Option<String>)>(
                "SELECT filename, is_ai_artifact, source_path, extracted_text_path FROM documents \
                 WHERE id = ? AND case_id = ? AND deleted_at IS NULL",
            )
            .bind(document_id)
            .bind(case_id)
            .fetch_optional(ctx.pool)
            .await?
            .ok_or_else(|| ToolError::InvalidArgs("文档不存在或不属于当前案件".into()))?;
            let path = if row.1 { Some(row.2) } else { row.3 };
            let path =
                path.ok_or_else(|| ToolError::Runtime(format!("《{}》尚无可读的派生文本", row.0)))?;
            read_bounded(Path::new(&path))
        }
        WorkspaceScope::Independent(workspace_id) => {
            let document = crate::db::ai_workspace_documents::get_document(
                ctx.pool,
                &workspace_id,
                document_id,
            )
            .await?
            .ok_or_else(|| ToolError::InvalidArgs("文档不存在或不属于当前工作区".into()))?;
            let path = if document.kind == "artifact" {
                document.content_path
            } else {
                document.extracted_text_path
            }
            .ok_or_else(|| ToolError::Runtime("文档尚无可读的派生文本".into()))?;
            read_bounded(Path::new(&path))
        }
    }
}

async fn create_file(
    ctx: &ToolContext<'_>,
    title: &str,
    markdown: &str,
) -> Result<String, ToolError> {
    require_new_workspace_file_permission(ctx).await?;
    let title = safe_title(title)?;
    if markdown.len() > MAX_WRITE_BYTES {
        return Err(ToolError::InvalidArgs("正文超过 20MB".into()));
    }
    match resolve_scope(ctx).await? {
        WorkspaceScope::Case(case_id) => crate::chat::tools::artifact::persist_filing(
            ctx.pool,
            case_id,
            "工作区文稿",
            &title,
            markdown,
        )
        .await
        .map_err(ToolError::Runtime),
        WorkspaceScope::Independent(workspace_id) => {
            let root =
                crate::db::app_data_dir().map_err(|error| ToolError::Runtime(error.to_string()))?;
            crate::ai_workspace::commands::create_ai_workspace_artifact_impl(
                ctx.pool,
                &root,
                &workspace_id,
                CreateAiWorkspaceArtifactInput {
                    title,
                    initial_markdown: Some(markdown.to_string()),
                },
            )
            .await
            .map(|created| created.document.id)
            .map_err(ToolError::Runtime)
        }
    }
}

async fn write_file(
    ctx: &ToolContext<'_>,
    document_id: &str,
    markdown: &str,
    title_override: Option<&str>,
) -> Result<(), ToolError> {
    require_editing_target(ctx, document_id).await?;
    match resolve_scope(ctx).await? {
        WorkspaceScope::Case(case_id) => {
            let row = sqlx::query_as::<_, (String, String)>(
                "SELECT source_path, filename FROM documents WHERE id = ? AND case_id = ? \
                 AND is_ai_artifact = 1 AND source IN ('chat', 'chat_artifact') AND deleted_at IS NULL",
            )
            .bind(document_id)
            .bind(case_id)
            .fetch_optional(ctx.pool)
            .await?
            .ok_or_else(|| ToolError::Runtime("只能覆盖当前案件的派生文稿，原始材料始终只读".into()))?;
            let path = PathBuf::from(&row.0);
            let old = read_bounded(&path)?;
            write_atomic(&path, markdown)?;
            let filename = title_override
                .map(safe_title)
                .transpose()?
                .map(|title| format!("{title}.md"));
            let update = sqlx::query(
                "UPDATE documents SET filename = COALESCE(?, filename), size_bytes = ?, \
                 modified_at = ? WHERE id = ? AND case_id = ? AND is_ai_artifact = 1",
            )
            .bind(filename)
            .bind(markdown.len() as i64)
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(document_id)
            .bind(case_id)
            .execute(ctx.pool)
            .await;
            if let Err(error) = update {
                let _ = write_atomic(&path, &old);
                return Err(ToolError::Sqlx(error));
            }
            Ok(())
        }
        WorkspaceScope::Independent(workspace_id) => {
            let current = crate::db::ai_workspace_documents::get_document(
                ctx.pool,
                &workspace_id,
                document_id,
            )
            .await?
            .ok_or_else(|| ToolError::InvalidArgs("文稿不存在".into()))?;
            if current.kind != "artifact" {
                return Err(ToolError::Runtime(
                    "原始材料始终只读，只能覆盖派生文稿".into(),
                ));
            }
            let title = title_override
                .map(safe_title)
                .transpose()?
                .unwrap_or(current.title);
            let current_content = crate::ai_workspace::commands::read_ai_workspace_artifact_impl(
                ctx.pool,
                &workspace_id,
                document_id,
            )
            .await
            .map_err(ToolError::Runtime)?;
            crate::db::ai_workspace_documents::add_document_version_with_summary(
                ctx.pool,
                &workspace_id,
                document_id,
                &current_content.markdown,
                "user",
                "before_ai",
                "AI 原位修改前",
                ctx.message_id,
                "[]",
            )
            .await
            .map_err(|error| ToolError::Runtime(error.to_string()))?;
            let saved = crate::ai_workspace::commands::save_ai_workspace_artifact_impl(
                ctx.pool,
                &workspace_id,
                document_id,
                SaveAiWorkspaceArtifactInput {
                    title,
                    markdown: markdown.to_string(),
                    expected_revision: current.working_copy_revision,
                },
            )
            .await
            .map_err(ToolError::Runtime)?;
            crate::db::ai_workspace_documents::add_document_version_with_summary(
                ctx.pool,
                &workspace_id,
                document_id,
                &saved.markdown,
                "ai",
                "after_ai",
                "AI 原位更新当前文稿",
                ctx.message_id,
                "[]",
            )
            .await
            .map_err(|error| ToolError::Runtime(error.to_string()))?;
            Ok(())
        }
    }
}

async fn rename_file(
    ctx: &ToolContext<'_>,
    document_id: &str,
    title: &str,
) -> Result<(), ToolError> {
    let content = read_file(ctx, document_id).await?;
    write_file(ctx, document_id, &content, Some(title)).await
}

async fn archive_file(ctx: &ToolContext<'_>, document_id: &str) -> Result<(), ToolError> {
    match resolve_scope(ctx).await? {
        WorkspaceScope::Case(case_id) => {
            let result = sqlx::query(
                "UPDATE documents SET deleted_at = ? WHERE id = ? AND case_id = ? \
                 AND is_ai_artifact = 1 AND source IN ('chat', 'chat_artifact') AND deleted_at IS NULL",
            )
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(document_id)
            .bind(case_id)
            .execute(ctx.pool)
            .await?;
            if result.rows_affected() == 0 {
                return Err(ToolError::Runtime(
                    "只能归档当前案件的派生文稿；原始材料不能删除".into(),
                ));
            }
            Ok(())
        }
        WorkspaceScope::Independent(workspace_id) => {
            crate::db::ai_workspace_documents::archive_document(
                ctx.pool,
                &workspace_id,
                document_id,
            )
            .await
            .map_err(|error| ToolError::Runtime(error.to_string()))
        }
    }
}

fn document_schema(extra: Value, required: &[&str]) -> Value {
    let mut properties = serde_json::Map::new();
    properties.insert(
        "document_id".into(),
        json!({"type": "string", "description": "list_workspace_files 返回的 document_id"}),
    );
    if let Some(extra) = extra.as_object() {
        properties.extend(extra.clone());
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

pub struct ListWorkspaceFiles;
pub struct ReadWorkspaceFile;
pub struct CreateWorkspaceFile;
pub struct WriteWorkspaceFile;
pub struct RenameWorkspaceFile;
pub struct CopyWorkspaceFile;
pub struct ArchiveWorkspaceFile;

#[async_trait]
impl Tool for ListWorkspaceFiles {
    fn name(&self) -> &str {
        "list_workspace_files"
    }
    fn description(&self) -> &str {
        workspace_tool_description!("列出当前案件或独立事务工作区的真实材料和派生文稿，返回 document_id、名称、类型、处理状态和 writable 标记。开始涉及文件的任务时优先调用本工具建立清单。")
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{},"additionalProperties":false})
    }
    async fn execute(&self, _args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        list_files(ctx).await
    }
}

#[async_trait]
impl Tool for ReadWorkspaceFile {
    fn name(&self) -> &str {
        "read_workspace_file"
    }
    fn description(&self) -> &str {
        workspace_tool_description!("按 document_id 读取一份工作区文件的 Markdown/纯文本。source 只读取 CaseBoard 生成的派生文本；artifact 读取当前工作副本。")
    }
    fn parameters_schema(&self) -> Value {
        document_schema(json!({}), &["document_id"])
    }
    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        Ok(ToolResult::plain(
            read_file(ctx, require_str(args, "document_id")?).await?,
        ))
    }
}

#[async_trait]
impl Tool for CreateWorkspaceFile {
    fn name(&self) -> &str {
        "create_workspace_file"
    }
    fn description(&self) -> &str {
        workspace_tool_description!("在当前派生工作区新建一份可编辑 Markdown 文稿并返回 document_id。用户要求保存、形成报告或输出文件时，完成全文后使用。")
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"title":{"type":"string"},"markdown":{"type":"string"}},"required":["title","markdown"],"additionalProperties":false})
    }
    fn is_mutating(&self) -> bool {
        true
    }
    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let id = create_file(
            ctx,
            require_str(args, "title")?,
            require_str(args, "markdown")?,
        )
        .await?;
        Ok(ToolResult::plain(format!(
            "✅ 已创建派生文稿(document_id={id})"
        )))
    }
}

#[async_trait]
impl Tool for WriteWorkspaceFile {
    fn name(&self) -> &str {
        "write_workspace_file"
    }
    fn description(&self) -> &str {
        workspace_tool_description!("更新当前工作区的一份派生 Markdown 文稿。调用前必须读取完整正文，传回修改后的完整 Markdown；只改变用户要求的内容并保留其它段落。适用于补充内容、调整标题层级/粗体/列表/表格以及整体重写；原始材料永远只读。")
    }
    fn parameters_schema(&self) -> Value {
        document_schema(
            json!({"markdown":{"type":"string"}}),
            &["document_id", "markdown"],
        )
    }
    fn is_mutating(&self) -> bool {
        true
    }
    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        write_file(
            ctx,
            require_str(args, "document_id")?,
            require_str(args, "markdown")?,
            None,
        )
        .await?;
        Ok(ToolResult::plain("✅ 已更新派生文稿"))
    }
}

#[async_trait]
impl Tool for RenameWorkspaceFile {
    fn name(&self) -> &str {
        "rename_workspace_file"
    }
    fn description(&self) -> &str {
        workspace_tool_description!("修改派生文稿在 CaseBoard 中显示的标题和文件名。它只调整工作区逻辑名称，不移动或重命名任何导入的原始材料。")
    }
    fn parameters_schema(&self) -> Value {
        document_schema(
            json!({"title":{"type":"string"}}),
            &["document_id", "title"],
        )
    }
    fn is_mutating(&self) -> bool {
        true
    }
    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        rename_file(
            ctx,
            require_str(args, "document_id")?,
            require_str(args, "title")?,
        )
        .await?;
        Ok(ToolResult::plain("✅ 已重命名派生文稿"))
    }
}

#[async_trait]
impl Tool for CopyWorkspaceFile {
    fn name(&self) -> &str {
        "copy_workspace_file"
    }
    fn description(&self) -> &str {
        workspace_tool_description!("把工作区 source 材料的派生文本或现有 artifact 复制为一份新的可编辑 Markdown 文稿，返回新 document_id，原文件保持不变。仅在用户明确要求另存、新建副本、复制一份或保留旧版时使用；当前已有可编辑文稿且用户只要求修改时禁止使用。")
    }
    fn parameters_schema(&self) -> Value {
        document_schema(
            json!({"title":{"type":"string"}}),
            &["document_id", "title"],
        )
    }
    fn is_mutating(&self) -> bool {
        true
    }
    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let content = read_file(ctx, require_str(args, "document_id")?).await?;
        let id = create_file(ctx, require_str(args, "title")?, &content).await?;
        Ok(ToolResult::plain(format!(
            "✅ 已复制为派生文稿(document_id={id})"
        )))
    }
}

#[async_trait]
impl Tool for ArchiveWorkspaceFile {
    fn name(&self) -> &str {
        "archive_workspace_file"
    }
    fn description(&self) -> &str {
        workspace_tool_description!("从当前工作区软归档指定文件。案件原始材料不可归档；独立工作区 source 仅隐藏工作区记录，磁盘原文件和 CaseBoard 派生文本不会被物理删除。")
    }
    fn parameters_schema(&self) -> Value {
        document_schema(json!({}), &["document_id"])
    }
    fn is_mutating(&self) -> bool {
        true
    }
    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        archive_file(ctx, require_str(args, "document_id")?).await?;
        Ok(ToolResult::plain("✅ 已从工作区归档；原始磁盘文件未改动"))
    }
}
