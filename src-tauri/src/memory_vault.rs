//! 应用级 AI 记忆文件仓库。
//!
//! 这里管理的是用户可直接查看/编辑的 Markdown 记忆文件,默认落在本地知识库目录内:
//! `<local_kb_root>/记忆/`。SQLite 里的 `case_memories/global_memories` 仍负责聊天候选;
//! 本模块负责把“冷启动、全局、功能、写作”等长期记忆做成可管理文件,并按预算生成 prompt pack。

use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use directories::UserDirs;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::settings::Settings;

const MEMORY_DIR_NAME: &str = "记忆";
const DEFAULT_KB_DIR_NAME: &str = "知识库";
const README_FILE: &str = "README.md";
const NOTE_MAX_CHARS: usize = 20_000;

const CATEGORY_DIRS: &[&str] = &[
    "cold_start",
    "global",
    "case",
    "function",
    "writing",
    "workflow",
    "other",
];

const DEFAULT_PROMPT_CHAR_BUDGET: usize = 6_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryVaultStatus {
    pub root_path: String,
    pub notes: Vec<MemoryNote>,
    pub prompt_pack: MemoryPromptPack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryNote {
    pub id: String,
    pub title: String,
    pub category: String,
    pub content: String,
    pub source: String,
    pub inject_mode: String,
    pub path: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveMemoryNoteInput {
    pub id: Option<String>,
    pub title: String,
    pub category: String,
    pub content: String,
    pub source: Option<String>,
    pub inject_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPromptPack {
    pub items: Vec<String>,
    pub source_count: usize,
    pub omitted_count: usize,
    pub char_budget: usize,
    pub used_chars: usize,
    pub compressed: bool,
}

pub fn load_vault_status(settings: &Settings) -> Result<MemoryVaultStatus, String> {
    let root = ensure_memory_root(settings)?;
    let notes = list_notes_in_root(&root)?;
    let prompt_pack = build_prompt_pack_from_notes(&notes, DEFAULT_PROMPT_CHAR_BUDGET);
    Ok(MemoryVaultStatus {
        root_path: root.to_string_lossy().to_string(),
        notes,
        prompt_pack,
    })
}

pub fn save_note(settings: &Settings, input: SaveMemoryNoteInput) -> Result<MemoryNote, String> {
    let root = ensure_memory_root(settings)?;
    save_note_in_root(&root, input)
}

pub fn build_prompt_pack(settings: &Settings) -> Result<MemoryPromptPack, String> {
    let root = ensure_memory_root(settings)?;
    let notes = list_notes_in_root(&root)?;
    Ok(build_prompt_pack_from_notes(
        &notes,
        DEFAULT_PROMPT_CHAR_BUDGET,
    ))
}

pub fn build_prompt_pack_for_modes(
    settings: &Settings,
    allowed_modes: &[&str],
) -> Result<MemoryPromptPack, String> {
    let root = ensure_memory_root(settings)?;
    let notes = list_notes_in_root(&root)?;
    Ok(build_prompt_pack_from_notes_for_modes(
        &notes,
        DEFAULT_PROMPT_CHAR_BUDGET,
        allowed_modes,
    ))
}

pub(crate) fn resolve_memory_root_from_settings(settings: &Settings) -> Result<PathBuf, String> {
    if let Some(root) = settings
        .local_kb_root
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(PathBuf::from(root).join(MEMORY_DIR_NAME));
    }

    let user_dirs = UserDirs::new().ok_or_else(|| "无法定位用户目录".to_string())?;
    let documents = user_dirs
        .document_dir()
        .ok_or_else(|| "无法定位 Documents 目录".to_string())?;
    Ok(documents.join(DEFAULT_KB_DIR_NAME).join(MEMORY_DIR_NAME))
}

fn ensure_memory_root(settings: &Settings) -> Result<PathBuf, String> {
    let root = resolve_memory_root_from_settings(settings)?;
    fs::create_dir_all(&root).map_err(|e| format!("创建记忆目录失败: {e}"))?;
    for category in CATEGORY_DIRS {
        fs::create_dir_all(root.join(category))
            .map_err(|e| format!("创建记忆分类目录失败({category}): {e}"))?;
    }
    let readme = root.join(README_FILE);
    if !readme.exists() {
        fs::write(
            &readme,
            "# CaseBoard 记忆\n\n这里存放 CaseBoard 的 AI 长期记忆。每条记忆是一个 Markdown 文件,可在 App 的「记忆」页查看和编辑。\n",
        )
        .map_err(|e| format!("写记忆 README 失败: {e}"))?;
    }
    Ok(root)
}

pub(crate) fn list_notes_in_root(root: &Path) -> Result<Vec<MemoryNote>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut notes = Vec::new();
    collect_notes(root, root, &mut notes)?;
    notes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(notes)
}

pub(crate) fn save_note_in_root(
    root: &Path,
    input: SaveMemoryNoteInput,
) -> Result<MemoryNote, String> {
    fs::create_dir_all(root).map_err(|e| format!("创建记忆目录失败: {e}"))?;

    let id = input
        .id
        .as_deref()
        .map(normalize_id)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let title = normalize_title(&input.title)?;
    let category = normalize_category(&input.category);
    let content = normalize_content(&input.content)?;
    let source = normalize_meta(input.source.as_deref(), "manual");
    let inject_mode = normalize_meta(input.inject_mode.as_deref(), "manual_select");
    let now = Utc::now().to_rfc3339();

    let existing = find_note_by_id(root, &id)?;
    let created_at = existing
        .as_ref()
        .and_then(|(_, note)| non_empty(&note.created_at))
        .unwrap_or_else(|| now.clone());

    let dir = root.join(&category);
    fs::create_dir_all(&dir).map_err(|e| format!("创建记忆分类目录失败: {e}"))?;
    let target = dir.join(format!("{id}.md"));
    let note = MemoryNote {
        id,
        title,
        category,
        content,
        source,
        inject_mode,
        path: String::new(),
        created_at,
        updated_at: now,
    };
    fs::write(&target, render_note_md(&note)).map_err(|e| format!("写记忆文件失败: {e}"))?;

    if let Some((old_path, _)) = existing {
        if old_path != target && old_path.exists() {
            fs::remove_file(&old_path).map_err(|e| format!("清理旧记忆文件失败: {e}"))?;
        }
    }

    let mut saved = note;
    saved.path = rel_path(root, &target);
    Ok(saved)
}

pub(crate) fn build_prompt_pack_from_notes(
    notes: &[MemoryNote],
    char_budget: usize,
) -> MemoryPromptPack {
    build_prompt_pack_from_notes_for_modes(notes, char_budget, &[])
}

pub(crate) fn build_prompt_pack_from_notes_for_modes(
    notes: &[MemoryNote],
    char_budget: usize,
    allowed_modes: &[&str],
) -> MemoryPromptPack {
    let mut injectable: Vec<&MemoryNote> = notes
        .iter()
        .filter(|note| is_prompt_injectable_for_modes(&note.inject_mode, allowed_modes))
        .collect();
    injectable.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

    let source_count = injectable.len();
    let mut items = Vec::new();
    let mut used_chars = 0usize;
    let mut omitted = 0usize;
    let budget = char_budget.max(500);

    for note in injectable {
        let item = format!(
            "[{} / {}] {}: {}",
            note.category,
            note.inject_mode,
            note.title,
            note.content.trim()
        );
        let len = item.chars().count() + 1;
        if used_chars + len <= budget {
            used_chars += len;
            items.push(item);
        } else {
            omitted += 1;
        }
    }

    let compressed = omitted > 0;
    if compressed {
        while !items.is_empty() && budget.saturating_sub(used_chars) < 160 {
            if let Some(removed) = items.pop() {
                used_chars = used_chars.saturating_sub(removed.chars().count() + 1);
                omitted += 1;
            }
        }
        let summary = compact_omitted_summary(notes, omitted, budget.saturating_sub(used_chars));
        if !summary.is_empty() {
            used_chars += summary.chars().count() + 1;
            items.push(summary);
        }
    }

    MemoryPromptPack {
        items,
        source_count,
        omitted_count: omitted,
        char_budget: budget,
        used_chars,
        compressed,
    }
}

fn collect_notes(root: &Path, dir: &Path, out: &mut Vec<MemoryNote>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("读取记忆目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取记忆目录项失败: {e}"))?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        if file_name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect_notes(root, &path, out)?;
            continue;
        }
        if path.extension().and_then(|s| s.to_str()) != Some("md") || file_name == README_FILE {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(|e| format!("读取记忆文件失败: {e}"))?;
        let mut note = parse_note_md(&text)?;
        note.path = rel_path(root, &path);
        out.push(note);
    }
    Ok(())
}

fn find_note_by_id(root: &Path, id: &str) -> Result<Option<(PathBuf, MemoryNote)>, String> {
    for note in list_notes_in_root(root)? {
        if note.id == id {
            return Ok(Some((root.join(&note.path), note)));
        }
    }
    Ok(None)
}

fn render_note_md(note: &MemoryNote) -> String {
    format!(
        "---\n\
         id: {}\n\
         title: {}\n\
         category: {}\n\
         source: {}\n\
         inject_mode: {}\n\
         created_at: {}\n\
         updated_at: {}\n\
         ---\n\n\
         {}\n",
        note.id,
        one_line(&note.title),
        note.category,
        note.source,
        note.inject_mode,
        note.created_at,
        note.updated_at,
        note.content.trim()
    )
}

fn parse_note_md(text: &str) -> Result<MemoryNote, String> {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---\n") {
        return Err("记忆文件缺少 front matter".into());
    }
    let rest = &trimmed[4..];
    let Some(end) = rest.find("\n---") else {
        return Err("记忆文件 front matter 未闭合".into());
    };
    let meta = &rest[..end];
    let content = rest[end + 4..]
        .trim_start_matches(['\n', '\r'])
        .trim()
        .to_string();

    let get = |key: &str| -> String {
        meta.lines()
            .find_map(|line| {
                let (k, v) = line.split_once(':')?;
                (k.trim() == key).then(|| v.trim().to_string())
            })
            .unwrap_or_default()
    };

    let id = non_empty(&get("id")).unwrap_or_else(|| Uuid::new_v4().to_string());
    let title = non_empty(&get("title")).unwrap_or_else(|| "未命名记忆".to_string());
    let category = normalize_category(&get("category"));
    let source = normalize_meta(Some(&get("source")), "manual");
    let inject_mode = normalize_meta(Some(&get("inject_mode")), "manual_select");
    let created_at = non_empty(&get("created_at")).unwrap_or_default();
    let updated_at = non_empty(&get("updated_at")).unwrap_or_else(|| created_at.clone());

    Ok(MemoryNote {
        id,
        title,
        category,
        content,
        source,
        inject_mode,
        path: String::new(),
        created_at,
        updated_at,
    })
}

fn normalize_title(title: &str) -> Result<String, String> {
    let title = one_line(title);
    if title.is_empty() {
        return Err("记忆标题不能为空".into());
    }
    Ok(title.chars().take(120).collect())
}

fn normalize_content(content: &str) -> Result<String, String> {
    let content = content.trim();
    if content.is_empty() {
        return Err("记忆内容不能为空".into());
    }
    if content.chars().count() > NOTE_MAX_CHARS {
        return Err(format!("记忆内容最多 {NOTE_MAX_CHARS} 字"));
    }
    Ok(content.to_string())
}

fn normalize_category(category: &str) -> String {
    let cleaned = category
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let cleaned = cleaned.trim_matches('_');
    if cleaned.is_empty() {
        "other".into()
    } else {
        cleaned.chars().take(40).collect()
    }
}

fn normalize_id(id: &str) -> String {
    id.trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .take(80)
        .collect()
}

fn normalize_meta(value: Option<&str>, fallback: &str) -> String {
    let value = value.map(one_line).unwrap_or_default();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.chars().take(80).collect()
    }
}

fn is_prompt_injectable(inject_mode: &str) -> bool {
    !matches!(
        inject_mode.trim(),
        "" | "manual_select" | "never" | "disabled" | "archive"
    )
}

fn is_prompt_injectable_for_modes(inject_mode: &str, allowed_modes: &[&str]) -> bool {
    if !is_prompt_injectable(inject_mode) {
        return false;
    }
    allowed_modes.is_empty() || allowed_modes.iter().any(|mode| *mode == inject_mode.trim())
}

fn compact_omitted_summary(
    notes: &[MemoryNote],
    omitted_count: usize,
    remaining_budget: usize,
) -> String {
    if omitted_count == 0 || remaining_budget < 80 {
        return String::new();
    }
    let mut categories = std::collections::BTreeMap::<String, usize>::new();
    for note in notes
        .iter()
        .filter(|note| is_prompt_injectable(&note.inject_mode))
    {
        *categories.entry(note.category.clone()).or_default() += 1;
    }
    let category_text = categories
        .into_iter()
        .map(|(category, count)| format!("{category} {count}条"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut summary = format!(
        "[压缩记忆] 另有 {omitted_count} 条可注入记忆因预算限制未展开;分类分布: {category_text}。如本轮任务需要,优先遵守用户本轮指令、案件材料和工具结果。"
    );
    if summary.chars().count() > remaining_budget {
        summary = summary.chars().take(remaining_budget).collect();
    }
    summary
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ").trim().to_string()
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}
