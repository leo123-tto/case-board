//! 新建空 KB 目录结构 + 已存在 KB 的子目录补齐(只补不覆盖)。
//!
//! 跟 `legal-kb` skill 的主库结构对齐(详 § 6.7-bis):
//!   `raw/` + `raw/notes/` + `raw/companies/` + `raw/yuandian-cache/`
//!   + `wiki/` + `wiki/sources/` + `wiki/topics/` + `wiki/index.md` + `gap-log.md`

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::Serialize;
use uuid::Uuid;

use super::KbError;

// SAFETY: PathBuf 在 KbInitResult struct 字段里用了,clippy 误判为 unused 可以忽略。
// (这条 use 也确实保留 PathBuf 让 struct 字段语义清楚)

/// 创建新空 KB 时给前端的回执。
#[derive(Debug, Clone, Serialize)]
pub struct KbInitResult {
    pub created_at: DateTime<Local>,
    pub path: PathBuf,
    pub files_created: u32,
    pub dirs_created: u32,
    /// `true` = 已存在,本次只补缺失子目录;`false` = 全新创建
    pub reused_existing: bool,
}

/// 安全迁移回执。迁移采用“复制 → 核对 → 同卷原子改名”，源目录始终保留。
#[derive(Debug, Clone, Serialize)]
pub struct KbMigrationResult {
    pub source: PathBuf,
    pub target: PathBuf,
    pub files_copied: u64,
    pub dirs_copied: u64,
    pub bytes_copied: u64,
    pub source_preserved: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TreeStats {
    files: u64,
    dirs: u64,
    bytes: u64,
}

const SUBDIRS: &[&str] = &[
    "raw",
    "raw/notes",
    "raw/companies",
    "raw/yuandian-cache",
    "raw/cases-experience",
    "wiki",
    "wiki/sources",
    "wiki/topics",
];

const WELCOME_MD: &str = "# 法律知识库\n\n\
这是 CaseBoard 为你创建的空知识库。\n\n\
## 目录说明\n\
- `raw/notes/` — 你手动整理的原始笔记\n\
- `raw/companies/` — 企业档案\n\
- `raw/yuandian-cache/` — **CaseBoard / Codex 自动写入的元典 API 缓存**(不建议手动改)\n\
- `raw/cases-experience/` — **CaseBoard 结案案件沉淀的办案经验卡片**(可被 search_local_kb 检索复用)\n\
- `wiki/sources/` — 你整理过的来源页(由 Claude Code + legal-kb skill 治理)\n\
- `wiki/topics/` — 专题页\n\n\
## 长期使用建议\n\
- 用 CaseBoard 跑案件 chat,元典调用先写缓存；整部法规会清洗进入 `raw/notes/` L1 原文区并标记待复核，不自动冒充已治理的 `wiki/sources/` 来源页\n\
- 本地 BM25/语义检索强命中就直接复用；仅本地不足时再调用元典\n\
- 同事可以通过 CaseBoard 导出/导入资料包共享 `yuandian-cache/`\n";

const GAP_LOG_MD: &str = "# 缺口清单\n\n(暂无)\n";

/// 在指定路径创建空 KB 目录结构。已存在则走 [`reconcile_existing`](见同文件)。
pub fn create_empty_kb(target: &Path) -> Result<KbInitResult, KbError> {
    if target.exists() {
        return reconcile_existing(target);
    }
    std::fs::create_dir_all(target)?;
    let mut dirs_created = 1u32;
    for sub in SUBDIRS {
        let p = target.join(sub);
        if !p.exists() {
            std::fs::create_dir_all(&p)?;
            dirs_created += 1;
        }
    }
    let mut files_created = 0u32;
    let wiki_index = target.join("wiki").join("index.md");
    if !wiki_index.exists() {
        std::fs::write(&wiki_index, WELCOME_MD)?;
        files_created += 1;
    }
    let gap_log = target.join("gap-log.md");
    if !gap_log.exists() {
        std::fs::write(&gap_log, GAP_LOG_MD)?;
        files_created += 1;
    }
    Ok(KbInitResult {
        created_at: Local::now(),
        path: target.to_path_buf(),
        files_created,
        dirs_created,
        reused_existing: false,
    })
}

/// 已存在路径:**只补缺失的子目录,绝不覆盖任何已有文件**。
/// 若用户选了一个已有 KB(或一个完全无关的目录),都走这条 — 补全到结构齐备即可。
pub fn reconcile_existing(target: &Path) -> Result<KbInitResult, KbError> {
    if !target.is_dir() {
        return Err(KbError::NotADir(target.to_path_buf()));
    }
    let mut dirs_created = 0u32;
    for sub in SUBDIRS {
        let p = target.join(sub);
        if !p.exists() {
            std::fs::create_dir_all(&p)?;
            dirs_created += 1;
        }
    }
    // 文件**只补不覆盖** — 老板可能已经在 wiki/index.md 写了内容
    let mut files_created = 0u32;
    let wiki_index = target.join("wiki").join("index.md");
    if !wiki_index.exists() {
        std::fs::write(&wiki_index, WELCOME_MD)?;
        files_created += 1;
    }
    let gap_log = target.join("gap-log.md");
    if !gap_log.exists() {
        std::fs::write(&gap_log, GAP_LOG_MD)?;
        files_created += 1;
    }
    Ok(KbInitResult {
        created_at: Local::now(),
        path: target.to_path_buf(),
        files_created,
        dirs_created,
        reused_existing: true,
    })
}

/// 把现有 KB 安全复制到新目录。不会删除或改写源目录，也不会合并进非空目标目录。
pub fn migrate_kb(source: &Path, target: &Path) -> Result<KbMigrationResult, KbError> {
    if !source.is_dir() {
        return Err(KbError::NotADir(source.to_path_buf()));
    }
    let source = source.canonicalize()?;
    let target_parent = target.parent().ok_or_else(|| {
        KbError::InvalidMigrationTarget("目标目录必须有可写的上级目录".to_string())
    })?;
    if !target_parent.is_dir() {
        return Err(KbError::NoPath(target_parent.to_path_buf()));
    }
    let target_parent = target_parent.canonicalize()?;
    let target_name = target
        .file_name()
        .ok_or_else(|| KbError::InvalidMigrationTarget("请选择具体的目标目录".to_string()))?;
    let normalized_target = target_parent.join(target_name);

    if normalized_target == source
        || normalized_target.starts_with(&source)
        || source.starts_with(&normalized_target)
    {
        return Err(KbError::InvalidMigrationTarget(
            "源目录与目标目录不能相同，也不能互相嵌套".to_string(),
        ));
    }
    if normalized_target.exists() {
        if std::fs::symlink_metadata(&normalized_target)?
            .file_type()
            .is_symlink()
        {
            return Err(KbError::InvalidMigrationTarget(
                "目标目录不能是符号链接".to_string(),
            ));
        }
        if !normalized_target.is_dir() {
            return Err(KbError::NotADir(normalized_target));
        }
        if std::fs::read_dir(&normalized_target)?.next().is_some() {
            return Err(KbError::InvalidMigrationTarget(
                "目标目录不是空目录；为避免覆盖或混合资料，请选择一个空目录".to_string(),
            ));
        }
    }

    let staging = target_parent.join(format!(
        ".caseboard-kb-migrating-{}",
        Uuid::new_v4().simple()
    ));
    std::fs::create_dir(&staging)?;
    let copy_result = (|| {
        let source_stats = copy_tree_checked(&source, &staging)?;
        let target_stats = inspect_tree_checked(&staging)?;
        if source_stats != target_stats {
            return Err(KbError::MigrationVerificationFailed {
                expected_files: source_stats.files,
                actual_files: target_stats.files,
                expected_bytes: source_stats.bytes,
                actual_bytes: target_stats.bytes,
            });
        }
        if normalized_target.exists() {
            std::fs::remove_dir(&normalized_target)?;
        }
        std::fs::rename(&staging, &normalized_target)?;
        Ok(KbMigrationResult {
            source,
            target: normalized_target,
            files_copied: source_stats.files,
            dirs_copied: source_stats.dirs,
            bytes_copied: source_stats.bytes,
            source_preserved: true,
        })
    })();

    if copy_result.is_err() && staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    copy_result
}

fn copy_tree_checked(source: &Path, target: &Path) -> Result<TreeStats, KbError> {
    let mut stats = TreeStats::default();
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            return Err(KbError::InvalidMigrationTarget(format!(
                "知识库含符号链接，无法安全迁移：{}",
                source_path.display()
            )));
        }
        if metadata.is_dir() {
            std::fs::create_dir(&target_path)?;
            stats.dirs += 1;
            let child = copy_tree_checked(&source_path, &target_path)?;
            stats.files += child.files;
            stats.dirs += child.dirs;
            stats.bytes += child.bytes;
        } else if metadata.is_file() {
            let copied = std::fs::copy(&source_path, &target_path)?;
            if copied != metadata.len() {
                return Err(KbError::MigrationVerificationFailed {
                    expected_files: 1,
                    actual_files: 1,
                    expected_bytes: metadata.len(),
                    actual_bytes: copied,
                });
            }
            stats.files += 1;
            stats.bytes += copied;
        } else {
            return Err(KbError::InvalidMigrationTarget(format!(
                "知识库含不支持的特殊文件：{}",
                source_path.display()
            )));
        }
    }
    Ok(stats)
}

fn inspect_tree_checked(root: &Path) -> Result<TreeStats, KbError> {
    let mut stats = TreeStats::default();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let metadata = std::fs::symlink_metadata(entry.path())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            stats.dirs += 1;
            let child = inspect_tree_checked(&entry.path())?;
            stats.files += child.files;
            stats.dirs += child.dirs;
            stats.bytes += child.bytes;
        } else if metadata.is_file() {
            stats.files += 1;
            stats.bytes += metadata.len();
        } else {
            return Err(KbError::InvalidMigrationTarget(format!(
                "迁移暂存目录含不支持的文件：{}",
                entry.path().display()
            )));
        }
    }
    Ok(stats)
}
