//! V0.2 D7 · 本地 KB 三态检测 + 统计,给 Settings 卡片渲染。
//!
//! 三态(详 § 7.5):
//!   - `Bound`:路径存在 + 可读写 + 已挂上 LocalKb,这是常态
//!   - `Unbound`:settings 没填 / 路径不存在(用户没建过)
//!   - `PermissionDenied`:路径**实际存在**但 std::fs::read_dir 失败,
//!     极大概率是 macOS Documents/Desktop 访问权限被拒
//!
//! 统计字段(只在 `Bound` 态有意义):
//!   - `cache_count`:`yuandian-cache/index.json` 条目数
//!   - `cache_breakdown`:按 query_type 前缀分(rh_ft_* / rh_pt* / rh_enterprise*)
//!   - `total_size_bytes`:整个 KB root 的 du 求和(只扫顶 3 层,避免 wiki/sources 巨大子树拖慢)
//!   - `last_write_at`:`yuandian-cache/index.json` 的 mtime
//!
//! 设计原则:**全部静默**,任何 IO 错都降级返回部分数据,前端拿到 `null` 字段
//! 就 fallback 显示「—」而不是报错。

use std::path::PathBuf;

use chrono::{DateTime, Local};
use serde::Serialize;

use super::cache::{IndexEntry, LocalKb};
use crate::settings::Settings;

/// 三态枚举。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum KbStatus {
    Bound {
        root: PathBuf,
        cache_dir: PathBuf,
        /// `yuandian-cache/index.json` 条目数
        cache_count: u64,
        /// 按前缀粗分类:`{"法规": 156, "案例": 89, "企业": 242, "其他": 0}`
        cache_breakdown: serde_json::Value,
        /// 可检索内容篇数 —— 跟 search 实际覆盖范围一致(raw/notes + wiki/sources +
        /// wiki/topics + gap-log,不含 yuandian-cache)。跟 `cache_count` 是两回事:
        /// 前者是用户整理的资料,后者是元典查询缓存。
        content_count: u64,
        /// 整个 KB root 总占用(bytes),null = 求和失败
        total_size_bytes: Option<u64>,
        /// `yuandian-cache/index.json` 最近 mtime(ISO8601)
        last_write_at: Option<String>,
    },
    Unbound {
        /// 用户配置里的路径(若有),前端展示「默认路径 …(不存在)」
        configured_root: Option<String>,
    },
    PermissionDenied {
        /// 路径存在但无权访问,提示用户去系统设置授权
        root: String,
    },
}

/// 检测当前 KB 状态。失败兜底返回 `Unbound { configured_root: None }`。
pub fn detect_kb_status(settings: &Settings) -> KbStatus {
    let configured = settings.local_kb_root.clone();
    if settings.local_kb_enabled == Some(false) {
        return KbStatus::Unbound {
            configured_root: configured,
        };
    }
    let Some(raw) = configured.as_deref().filter(|s| !s.trim().is_empty()) else {
        return KbStatus::Unbound {
            configured_root: None,
        };
    };
    let expanded = shellexpand::tilde(raw).into_owned();
    let root = PathBuf::from(&expanded);

    if !root.exists() {
        return KbStatus::Unbound {
            configured_root: Some(raw.to_string()),
        };
    }
    if !root.is_dir() {
        // 配的不是目录(用户填了一个文件路径)— 视同 Unbound,前端会提示
        return KbStatus::Unbound {
            configured_root: Some(raw.to_string()),
        };
    }
    // 试探读权限:read_dir 失败十有八九是 macOS 权限拒
    if std::fs::read_dir(&root).is_err() {
        return KbStatus::PermissionDenied {
            root: raw.to_string(),
        };
    }

    // 走 auto_detect 拿一个 LocalKb(里面会确保 yuandian-cache/ 目录创出来)
    let Some(kb) = LocalKb::auto_detect(settings) else {
        // 上面 read_dir 已经通过,但 auto_detect 失败 — 大概率 create_dir_all 失败
        // 这种情况罕见,降级为 PermissionDenied
        return KbStatus::PermissionDenied {
            root: raw.to_string(),
        };
    };

    let (cache_count, cache_breakdown) = read_index_stats(&kb);
    let content_count = count_content_files(&kb.root);
    let total_size_bytes = compute_size_capped(&kb.root, 3);
    let last_write_at = kb
        .index_path
        .metadata()
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| DateTime::<Local>::from(t).to_rfc3339().into());

    KbStatus::Bound {
        root: kb.root.clone(),
        cache_dir: kb.yuandian_cache_dir.clone(),
        cache_count,
        cache_breakdown,
        content_count,
        total_size_bytes,
        last_write_at,
    }
}

/// 读 `index.json` 算条目数 + 按 query_type 粗分类。IO/parse 失败返回 (0, 空 map)。
fn read_index_stats(kb: &LocalKb) -> (u64, serde_json::Value) {
    use std::collections::HashMap;
    let Ok(raw) = std::fs::read_to_string(&kb.index_path) else {
        return (0, serde_json::json!({}));
    };
    if raw.trim().is_empty() {
        return (0, serde_json::json!({}));
    }
    let Ok(map) = serde_json::from_str::<HashMap<String, IndexEntry>>(&raw) else {
        return (0, serde_json::json!({}));
    };
    let count = map.len() as u64;
    let mut breakdown: HashMap<&'static str, u64> = HashMap::new();
    for v in map.values() {
        let bucket = bucket_of(&v.query_type);
        *breakdown.entry(bucket).or_insert(0) += 1;
    }
    (count, serde_json::to_value(breakdown).unwrap_or_default())
}

/// 数可检索内容篇数。范围跟 `search::default_scopes` 一致(raw/notes + raw/companies +
/// wiki/sources + wiki/topics + gap-log,**不含** yuandian-cache),只数 `.md` / `.txt`。
/// 给 Settings 卡片区分"已检索内容"和"元典缓存",避免只显缓存数误导用户。
fn count_content_files(root: &std::path::Path) -> u64 {
    use walkdir::WalkDir;
    let mut n: u64 = 0;
    for dir in ["raw/notes", "raw/companies", "wiki/sources", "wiki/topics"] {
        let target = root.join(dir);
        if !target.exists() {
            continue;
        }
        for entry in WalkDir::new(&target)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            let ext_ok = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e.to_lowercase().as_str(), "md" | "txt"))
                .unwrap_or(false);
            if p.is_file() && ext_ok {
                n += 1;
            }
        }
    }
    if root.join("gap-log.md").is_file() {
        n += 1;
    }
    n
}

/// query_type 前缀 → 中文分类标签。
fn bucket_of(query_type: &str) -> &'static str {
    if query_type.starts_with("rh_ft_")
        || query_type.starts_with("rh_fg_")
        || query_type == "law_vector_search"
    {
        "法规"
    } else if query_type.starts_with("rh_pt")
        || query_type.starts_with("rh_qw")
        || query_type == "case_vector_search"
    {
        "案例"
    } else if query_type.starts_with("rh_enterprise") {
        "企业"
    } else {
        "其他"
    }
}

/// 递归求和文件大小,限制最大深度避免扫穿巨型子树。失败返回 None。
fn compute_size_capped(root: &std::path::Path, max_depth: usize) -> Option<u64> {
    fn walk(p: &std::path::Path, depth: usize, max: usize, acc: &mut u64) -> std::io::Result<()> {
        if depth > max {
            return Ok(());
        }
        let md = std::fs::metadata(p)?;
        if md.is_file() {
            *acc += md.len();
            return Ok(());
        }
        if !md.is_dir() {
            return Ok(());
        }
        for entry in std::fs::read_dir(p)? {
            let entry = entry?;
            walk(&entry.path(), depth + 1, max, acc)?;
        }
        Ok(())
    }
    let mut total = 0u64;
    walk(root, 0, max_depth, &mut total).ok()?;
    Some(total)
}
