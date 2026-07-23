//! 受控知识库写回：只允许 create-only 新增 L1 raw 材料。

use std::io::Write;

use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::{opt_str, require_str, Tool, ToolContext, ToolError, ToolResult};

const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;
const ALLOWED_TYPES: &[&str] = &["regulation", "case", "article", "note"];

fn safe_title_stem(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' | '\t' => '_',
            value => value,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    let stem: String = trimmed.chars().take(72).collect();
    if stem.is_empty() {
        "未命名材料".into()
    } else {
        stem
    }
}

fn yaml_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".into())
}

fn write_l1_material(
    kb_root: &std::path::Path,
    material_type: &str,
    title: &str,
    content: &str,
    source: Option<&str>,
    created_at: &str,
) -> std::io::Result<(String, bool)> {
    let digest = format!("{:x}", Sha256::digest(content.as_bytes()));
    let short_hash = &digest[..12];
    let stem = safe_title_stem(title);
    let relative = format!("raw/notes/{stem}_{short_hash}.md");
    let path = kb_root.join(&relative);
    if path.exists() {
        return Ok((relative, false));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let source = source.unwrap_or("用户提供或本轮外部检索");
    let validity_scope = if material_type == "regulation"
        && crate::local_kb::validity::is_inactive_regulation_text(content)
    {
        "historical_research_only"
    } else {
        "current_or_unspecified"
    };
    let body = format!(
        "---\nkb_level: L1\nwiki_status: pending_review\nmaterial_type: {}\nvalidity_scope: {}\ntitle: {}\nsource: {}\ncreated_at: {}\ncontent_sha256: {}\n---\n\n# {}\n\n{}\n",
        yaml_string(material_type),
        validity_scope,
        yaml_string(title),
        yaml_string(source),
        yaml_string(created_at),
        yaml_string(&digest),
        title.trim(),
        content.trim(),
    );
    file.write_all(body.as_bytes())?;
    file.sync_all()?;
    Ok((relative, true))
}

pub struct SaveLocalKbMaterial;

#[async_trait]
impl Tool for SaveLocalKbMaterial {
    fn name(&self) -> &str {
        "save_local_kb_material"
    }

    fn description(&self) -> &str {
        include_str!("descriptions/save_local_kb_material.md")
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "material_type": {"type": "string", "enum": ALLOWED_TYPES, "description": "regulation=法规全文，case=案例全文，article=实务文章，note=其他完整笔记"},
                "title": {"type": "string", "description": "材料的真实完整标题"},
                "content_md": {"type": "string", "description": "完整或近完整 Markdown 正文；不得传搜索列表、空壳摘要、日志或报错页"},
                "source": {"type": "string", "description": "来源名称、URL、案号、法规发布机关或用户提供说明"}
            },
            "required": ["material_type", "title", "content_md"]
        })
    }

    fn is_mutating(&self) -> bool {
        true
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        if !ctx.settings.ai_kb_maintenance_enabled {
            return Err(ToolError::Runtime(
                "内置 AI 知识库写回未开启；请由用户在设置 → 知识库中明确开启".into(),
            ));
        }
        let kb = ctx
            .local_kb
            .ok_or_else(|| ToolError::Runtime("本地知识库未启用，无法新增 L1 raw 材料".into()))?;
        let material_type = require_str(args, "material_type")?.trim();
        if !ALLOWED_TYPES.contains(&material_type) {
            return Err(ToolError::InvalidArgs("material_type 不在允许清单".into()));
        }
        let title = require_str(args, "title")?.trim();
        let content = require_str(args, "content_md")?.trim();
        if content.len() > MAX_CONTENT_BYTES {
            return Err(ToolError::InvalidArgs("content_md 超过 2MB 上限".into()));
        }
        if content.as_bytes().contains(&0) {
            return Err(ToolError::InvalidArgs("content_md 疑似二进制内容".into()));
        }
        let historical_only = material_type == "regulation"
            && crate::local_kb::validity::is_inactive_regulation_text(content);
        let created_at = chrono::Local::now().to_rfc3339();
        let (relative, created) = write_l1_material(
            &kb.root,
            material_type,
            title,
            content,
            opt_str(args, "source"),
            &created_at,
        )?;
        if created {
            if let Some(app) = ctx.app.clone() {
                crate::spawn_kb_auto_index(app);
            }
            let scope_note = if historical_only {
                "；validity_scope=historical_research_only，仅供历史适用法研究，默认现行法检索与向量索引会排除"
            } else {
                ""
            };
            Ok(ToolResult::plain(format!(
                "✅ 已新增 L1 raw 材料：{relative}\n状态：wiki_status=pending_review{scope_note}。未经人工复核不会自动提升为 Wiki 导航页。"
            )))
        } else {
            Ok(ToolResult::plain(format!(
                "相同正文已存在：{relative}。未覆盖、未重复写入。"
            )))
        }
    }
}
