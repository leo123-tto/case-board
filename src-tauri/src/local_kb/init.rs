//! 新建空 KB 目录结构 + 已存在 KB 的子目录补齐(只补不覆盖)。
//!
//! 跟 `legal-kb` skill 的主库结构对齐(详 § 6.7-bis):
//!   `raw/` + `raw/notes/` + `raw/companies/` + `raw/yuandian-cache/`
//!   + `wiki/` + `wiki/sources/` + `wiki/topics/` + `wiki/index.md` + `gap-log.md`

use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use serde::Serialize;

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
- `raw/yuandian-cache/` — **CaseBoard / Claude Code 自动写入的元典缓存**(不建议手动改)\n\
- `raw/cases-experience/` — **CaseBoard 结案案件沉淀的办案经验卡片**(可被 search_local_kb 检索复用)\n\
- `wiki/sources/` — 你整理过的来源页(由 Claude Code + legal-kb skill 治理)\n\
- `wiki/topics/` — 专题页\n\n\
## 长期使用建议\n\
- 用 CaseBoard 跑案件 chat,法规/案例自动写入 `raw/yuandian-cache/`\n\
- 用 Claude Code + legal-kb skill 把重要内容升级到 `wiki/sources/`\n\
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

/// 知识库迁移回执。
#[derive(Debug, Clone, Serialize)]
pub struct KbRelocateResult {
    pub old_root: PathBuf,
    pub new_root: PathBuf,
    pub moved_files: u64,
    pub moved_bytes: u64,
}

/// 忽略 macOS 在空目录里自动生成的占位文件,避免"看起来空"的目录被误判为非空。
fn is_effectively_empty(dir: &Path) -> Result<bool, KbError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == ".DS_Store" || name == ".localized" {
            continue;
        }
        return Ok(false);
    }
    Ok(true)
}

/// 将旧知识库目录下的内容迁移到新目录。
///
/// 行为:
/// - 新目录不存在:自动创建。
/// - 新目录存在但为空(忽略 `.DS_Store`/`.localized`):把旧内容移入。
/// - 新目录存在且非空:返回错误,避免覆盖或混淆。
/// - 旧目录迁移完成后若为空则删除。
/// - 跨卷/跨设备时自动回退为"复制后删除源文件"。
pub fn relocate_kb(old_root: &Path, new_root: &Path) -> Result<KbRelocateResult, KbError> {
    let old = old_root
        .canonicalize()
        .unwrap_or_else(|_| old_root.to_path_buf());
    let new = new_root
        .canonicalize()
        .unwrap_or_else(|_| new_root.to_path_buf());

    if old == new {
        return Err(KbError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "新路径与当前路径相同",
        )));
    }
    if new.starts_with(&old) || old.starts_with(&new) {
        return Err(KbError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "新旧目录不能互相嵌套",
        )));
    }

    if !old.exists() || !old.is_dir() {
        return Err(KbError::NoPath(old));
    }
    if new.exists() && new.is_file() {
        return Err(KbError::NotADir(new));
    }
    if !new.exists() {
        std::fs::create_dir_all(&new)?;
    } else if !is_effectively_empty(&new)? {
        return Err(KbError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "目标目录已存在且非空,请选择一个空目录",
        )));
    } else {
        // 清理 macOS 占位文件,避免后续同名文件冲突。
        for name in [".DS_Store", ".localized"] {
            let _ = std::fs::remove_file(new.join(name));
        }
    }

    let mut moved_files = 0u64;
    let mut moved_bytes = 0u64;
    let mut failed_removals: Vec<String> = Vec::new();

    for entry in walkdir::WalkDir::new(&old).contents_first(true) {
        let entry = entry.map_err(|e| KbError::Io(std::io::Error::other(e)))?;
        let src = entry.path();
        if src == old.as_path() {
            continue;
        }
        let rel = src.strip_prefix(&old).map_err(|e| {
            KbError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                e.to_string(),
            ))
        })?;
        let dst = new.join(rel);

        if entry.file_type().is_dir() {
            if !dst.exists() {
                std::fs::create_dir_all(&dst)?;
            }
            if std::fs::read_dir(src)?.next().is_none() {
                if let Err(e) = std::fs::remove_dir(src) {
                    failed_removals.push(format!("{}: {}", src.display(), e));
                }
            }
        } else if entry.file_type().is_file() {
            if let Some(parent) = dst.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                }
            }
            let size = entry
                .metadata()
                .map_err(|e| KbError::Io(std::io::Error::other(e)))?
                .len();
            if std::fs::rename(src, &dst).is_ok() {
                moved_files += 1;
                moved_bytes += size;
            } else {
                // 跨卷等场景:复制后删除源文件。
                std::fs::copy(src, &dst)?;
                moved_files += 1;
                moved_bytes += size;
                if let Err(e) = std::fs::remove_file(src) {
                    failed_removals.push(format!("{}: {}", src.display(), e));
                }
            }
        }
        // 其他类型(如符号链接)跳过,不迁移也不报错。
    }

    if std::fs::read_dir(&old)?.next().is_none() {
        let _ = std::fs::remove_dir(&old);
    }

    if !failed_removals.is_empty() {
        return Err(KbError::Io(std::io::Error::other(format!(
            "迁移完成,但未能清理部分源文件: {}",
            failed_removals.join("; ")
        ))));
    }

    Ok(KbRelocateResult {
        old_root: old,
        new_root: new,
        moved_files,
        moved_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relocate_kb_moves_content_and_leaves_empty_old() {
        let tmp =
            std::env::temp_dir().join(format!("caseboard-relocate-test-{}", std::process::id()));
        let old = tmp.join("old-kb");
        let new = tmp.join("new-kb");

        // 清理残留
        let _ = std::fs::remove_dir_all(&tmp);

        create_empty_kb(&old).unwrap();
        std::fs::write(
            old.join("raw").join("notes").join("note.md"),
            "# 测试笔记\n",
        )
        .unwrap();
        std::fs::write(old.join("wiki").join("index.md"), "# 已有 Wiki\n").unwrap();

        let r = relocate_kb(&old, &new).unwrap();

        assert!(r.moved_files >= 2);
        assert!(new.join("raw").join("notes").join("note.md").exists());
        assert!(new.join("wiki").join("index.md").exists());
        // 旧目录应已被删除(因为迁移后为空)
        assert!(!old.exists());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn relocate_kb_rejects_nonempty_target() {
        let tmp =
            std::env::temp_dir().join(format!("caseboard-relocate-nontest-{}", std::process::id()));
        let old = tmp.join("old-kb");
        let new = tmp.join("new-kb");
        let _ = std::fs::remove_dir_all(&tmp);

        create_empty_kb(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(new.join("existing.txt"), "占坑").unwrap();

        let err = relocate_kb(&old, &new).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("非空") || msg.contains("目标目录"),
            "错误提示应说明目标目录非空: {}",
            msg
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
