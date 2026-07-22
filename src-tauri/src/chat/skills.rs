//! CaseBoard 全局法律 Skill 注册表。
//!
//! 这里只接收人工选择的纯 Markdown 指令包；不生成 Skill、不执行 Skill 内脚本，
//! 也不把用户的其他 Agent 目录暴露给 Runtime。

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

const BUILTIN_VERSION: &str = "1.0.0";
static REGISTRY_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

fn registry_lock() -> &'static std::sync::Mutex<()> {
    REGISTRY_LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

const BUILTINS: &[(&str, &str)] = &[
    (
        "compile-legal-basis",
        include_str!("../../resources/legal-skills/compile-legal-basis/SKILL.md"),
    ),
    (
        "simulate-opposition",
        include_str!("../../resources/legal-skills/simulate-opposition/SKILL.md"),
    ),
    (
        "find-similar-cases",
        include_str!("../../resources/legal-skills/find-similar-cases/SKILL.md"),
    ),
    (
        "deep-case-analysis",
        include_str!("../../resources/legal-skills/deep-case-analysis/SKILL.md"),
    ),
    (
        "criminal-case-analysis",
        include_str!("../../resources/legal-skills/criminal-case-analysis/SKILL.md"),
    ),
    (
        "legal-document-writing",
        include_str!("../../resources/legal-skills/legal-document-writing/SKILL.md"),
    ),
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LegalSkillSummary {
    pub name: String,
    pub description: String,
    pub source: String,
    pub version: String,
    pub sha256: String,
    pub removable: bool,
}

#[derive(Debug, Clone)]
pub struct LegalSkill {
    pub summary: LegalSkillSummary,
    pub body: String,
    pub file_path: PathBuf,
}

fn registry_root() -> Result<PathBuf, String> {
    {
        Ok(crate::db::app_data_dir()
            .map_err(|error| error.to_string())?
            .join("agent-skills"))
    }
}

fn sha256(body: &str) -> String {
    format!("{:x}", Sha256::digest(body.as_bytes()))
}

fn clean_yaml_value(value: &str) -> String {
    value.trim().trim_matches(['\'', '"']).trim().to_string()
}

fn parse_skill(body: &str) -> Result<(String, String, String), String> {
    if body.len() > 128 * 1024 {
        return Err("SKILL.md 超过 128KB，已拒绝导入".into());
    }
    let mut lines = body.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err("SKILL.md 缺少 YAML frontmatter".into());
    }
    let mut name = None;
    let mut description = None;
    let mut version = None;
    let mut closed = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            closed = true;
            break;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            name = Some(clean_yaml_value(value));
        } else if let Some(value) = trimmed.strip_prefix("description:") {
            description = Some(clean_yaml_value(value));
        } else if let Some(value) = trimmed.strip_prefix("version:") {
            version = Some(clean_yaml_value(value));
        }
    }
    if !closed {
        return Err("SKILL.md frontmatter 未闭合".into());
    }
    let name = name
        .filter(|value| !value.is_empty())
        .ok_or("Skill 缺少 name")?;
    let valid_name = name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid_name {
        return Err("Skill name 只允许 1-64 位小写字母、数字和单连字符".into());
    }
    let description = description
        .filter(|value| !value.is_empty() && value.chars().count() <= 1024)
        .ok_or("Skill 缺少有效 description")?;
    if body.split_once("---").is_none() || body.lines().count() < 6 {
        return Err("Skill 没有可执行的文字指令".into());
    }
    Ok((name, description, version.unwrap_or_else(|| "1.0.0".into())))
}

fn write_readonly(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    if path.exists() {
        make_owner_writable(path)?;
    }
    std::fs::write(path, content).map_err(|error| error.to_string())?;
    make_readonly(path)
}

#[cfg(unix)]
fn make_owner_writable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn make_owner_writable(path: &Path) -> Result<(), String> {
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_readonly(false);
    std::fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

#[cfg(unix)]
fn make_readonly(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o400))
        .map_err(|error| error.to_string())
}

#[cfg(not(unix))]
fn make_readonly(path: &Path) -> Result<(), String> {
    let mut permissions = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(path, permissions).map_err(|error| error.to_string())
}

fn ensure_builtins(root: &Path) -> Result<Vec<LegalSkill>, String> {
    BUILTINS
        .iter()
        .map(|(expected_name, body)| {
            let (name, description, version) = parse_skill(body)?;
            if name != *expected_name {
                return Err(format!("内置 Skill 名称不一致: {expected_name}/{name}"));
            }
            let file_path = root.join("builtin").join(&name).join("SKILL.md");
            let needs_write = std::fs::read_to_string(&file_path)
                .map(|existing| existing != *body)
                .unwrap_or(true);
            if needs_write {
                write_readonly(&file_path, body)?;
            }
            Ok(LegalSkill {
                summary: LegalSkillSummary {
                    name,
                    description,
                    source: "builtin".into(),
                    version: if version.is_empty() {
                        BUILTIN_VERSION.into()
                    } else {
                        version
                    },
                    sha256: sha256(body),
                    removable: false,
                },
                body: (*body).to_string(),
                file_path,
            })
        })
        .collect()
}

fn load_imported(root: &Path) -> Result<Vec<LegalSkill>, String> {
    let imported_root = root.join("imported");
    if !imported_root.exists() {
        return Ok(Vec::new());
    }
    let mut skills = Vec::new();
    for entry in std::fs::read_dir(&imported_root).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path().join("SKILL.md");
        if !path.is_file() {
            continue;
        }
        let body = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
        let (name, description, version) = parse_skill(&body)?;
        skills.push(LegalSkill {
            summary: LegalSkillSummary {
                name,
                description,
                source: "imported".into(),
                version,
                sha256: sha256(&body),
                removable: true,
            },
            body,
            file_path: path,
        });
    }
    skills.sort_by(|left, right| left.summary.name.cmp(&right.summary.name));
    Ok(skills)
}

pub fn load_all() -> Result<Vec<LegalSkill>, String> {
    let _guard = registry_lock()
        .lock()
        .map_err(|_| "Skill 注册表锁已损坏".to_string())?;
    load_all_unlocked()
}

fn load_all_unlocked() -> Result<Vec<LegalSkill>, String> {
    let root = registry_root()?;
    let mut skills = ensure_builtins(&root)?;
    let builtin_names = skills
        .iter()
        .map(|skill| skill.summary.name.clone())
        .collect::<std::collections::HashSet<_>>();
    for skill in load_imported(&root)? {
        if !builtin_names.contains(&skill.summary.name) {
            skills.push(skill);
        }
    }
    Ok(skills)
}

pub fn list() -> Result<Vec<LegalSkillSummary>, String> {
    Ok(load_all()?.into_iter().map(|skill| skill.summary).collect())
}

pub fn resolve(name: &str) -> Result<LegalSkill, String> {
    load_all()?
        .into_iter()
        .find(|skill| skill.summary.name == name)
        .ok_or_else(|| format!("未找到法律 Skill: {name}"))
}

pub fn native_prompt(selected_name: Option<&str>) -> Result<String, String> {
    let skills = load_all()?;
    let mut prompt = String::from("\n\n【全局法律 Skills】\n以下 Skills 可在需要时调用；先根据名称和说明判断，需要完整步骤时调用 read_legal_skill(name)。不得声称使用了不存在的 Skill，也不得创建、修改或安装 Skill：\n");
    for skill in &skills {
        prompt.push_str(&format!(
            "- {}：{}\n",
            skill.summary.name, skill.summary.description
        ));
    }
    if let Some(name) = selected_name {
        let selected = skills
            .iter()
            .find(|skill| skill.summary.name == name)
            .ok_or_else(|| format!("未找到法律 Skill: {name}"))?;
        prompt.push_str(&format!(
            "\n【本轮明确指定 Skill：{}】\n{}\n",
            selected.summary.name, selected.body
        ));
    }
    Ok(prompt)
}

pub fn import(path: &Path) -> Result<LegalSkillSummary, String> {
    let _guard = registry_lock()
        .lock()
        .map_err(|_| "Skill 注册表锁已损坏".to_string())?;
    let source_file = if path.is_dir() {
        let entries = std::fs::read_dir(path).map_err(|error| error.to_string())?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let candidate = entry.path();
            if candidate.file_name().and_then(|name| name.to_str()) == Some("SKILL.md") {
                continue;
            }
            return Err("为保证安全，目前只允许导入仅含 SKILL.md 的纯文字 Skill 目录".into());
        }
        path.join("SKILL.md")
    } else {
        path.to_path_buf()
    };
    if !source_file.is_file() {
        return Err("没有找到可读取的 SKILL.md".into());
    }
    let body = std::fs::read_to_string(&source_file).map_err(|error| error.to_string())?;
    let (name, description, version) = parse_skill(&body)?;
    if BUILTINS.iter().any(|(builtin, _)| *builtin == name) {
        return Err("不能覆盖 CaseBoard 内置法律 Skill".into());
    }
    let root = registry_root()?;
    let imported_root = root.join("imported");
    let imported_count = std::fs::read_dir(&imported_root)
        .map(|entries| entries.filter_map(Result::ok).count())
        .unwrap_or(0);
    if imported_count >= 32 {
        return Err("最多导入 32 个全局 Skill，请先移除不再使用的 Skill".into());
    }
    let destination = imported_root.join(&name).join("SKILL.md");
    if destination.exists() {
        return Err(format!("已存在同名 Skill: {name}"));
    }
    write_readonly(&destination, &body)?;
    Ok(LegalSkillSummary {
        name,
        description,
        source: "imported".into(),
        version,
        sha256: sha256(&body),
        removable: true,
    })
}

pub fn remove_imported(name: &str) -> Result<(), String> {
    let _guard = registry_lock()
        .lock()
        .map_err(|_| "Skill 注册表锁已损坏".to_string())?;
    if BUILTINS.iter().any(|(builtin, _)| *builtin == name) {
        return Err("内置法律 Skill 只读，不能移除".into());
    }
    let validated = parse_skill(&format!(
        "---\nname: {name}\ndescription: validation\n---\n\n# validation"
    ))
    .map(|(name, _, _)| name)?;
    let directory = registry_root()?.join("imported").join(validated);
    if !directory.is_dir() {
        return Err("没有找到这个已导入 Skill".into());
    }
    let file = directory.join("SKILL.md");
    if file.exists() {
        make_owner_writable(&file)?;
    }
    std::fs::remove_dir_all(directory).map_err(|error| error.to_string())
}
