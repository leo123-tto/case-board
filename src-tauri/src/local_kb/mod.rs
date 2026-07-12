//! 本地法律知识库(V0.2 起逐步实施 · 详 docs/V0.2-法律AI工作台-实施计划.md § 6.6 ~ § 6.8)。
//!
//! 子模块:
//! - `hash`(D1.5)— `query_hash` 跟 Python `_query_hash` 100% 对齐
//! - `cache`(D2)— `LocalKb` 主入口 + `check_cache` + `save_search` + `save_detail`,
//!   MD 模板跟 Python `cache.py` 严格对齐(双写互通)
//! - `init`(D2)— `create_empty_kb` + `reconcile_existing`(只补,不覆盖)
//! - `search`(D2)— 整库关键词检索 + `read_kb_file` 防路径穿越
//!
//! 跟 Python `~/.claude/skills/yuandian-legal-search/` skill 共写同一个目录,
//! 任何文件格式变化必须**两边同时改**(详 § 21 提醒)。

pub mod cache;
pub mod experience;
pub mod hash;
pub mod ingest;
pub mod init;
pub mod search;
pub mod semantic;
pub mod share;
pub mod status;

use std::path::PathBuf;
use thiserror::Error;

/// LocalKb 内部共享错误类型。`anyhow::Error` 不够语义,前端要区分"目录不存在 / 没权限 / 路径越界"等。
#[derive(Debug, Error)]
pub enum KbError {
    #[error("KB 路径不存在:{0}")]
    NoPath(PathBuf),
    #[error("KB 路径不是目录:{0}")]
    NotADir(PathBuf),
    #[error("KB 路径无权访问(macOS Documents 权限?):{0}")]
    PermissionDenied(PathBuf),
    #[error("路径越界(疑似攻击):{0}")]
    PathEscape(String),
    #[error("文件过大(上限 5MB):{path} = {size} bytes")]
    FileTooBig { path: PathBuf, size: u64 },
    #[error("文件疑似二进制(含 NUL):{0}")]
    BinaryFile(PathBuf),
    #[error("IO 错误:{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 错误:{0}")]
    Json(#[from] serde_json::Error),
    #[error("知识库迁移目标无效:{0}")]
    InvalidMigrationTarget(String),
    #[error(
        "知识库迁移校验失败:文件 {actual_files}/{expected_files},字节 {actual_bytes}/{expected_bytes}"
    )]
    MigrationVerificationFailed {
        expected_files: u64,
        actual_files: u64,
        expected_bytes: u64,
        actual_bytes: u64,
    },
}

impl serde::Serialize for KbError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}
