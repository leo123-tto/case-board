//! 整库关键词检索 + 文件读取(带路径穿越防护)。
//!
//! 默认搜索范围:整个 `local_kb_root` 下的 `.md` / `.txt` 文件。
//! **排除**:`raw/yuandian-cache/`(那是元典缓存,LLM 走专用元典工具命中;确需搜可显式 include)
//!
//! `read_kb_file` 的安全约束:
//!   1. `canonicalize` + `starts_with` 防穿越(LLM 给 `../../etc/passwd` 直接拒)
//!   2. 文件大小上限 5MB
//!   3. 二进制检测:open 后读头 512 字节,含 NUL 拒绝

use std::path::Path;

use serde::Serialize;
use walkdir::WalkDir;

use super::KbError;

const MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;
const BINARY_PEEK_BYTES: usize = 512;

#[derive(Debug, Clone, Copy)]
pub enum KbScope {
    Root,            // 整个 local_kb_root(默认)
    Notes,           // raw/notes/
    Companies,       // raw/companies/(企业档案 / 调查报告)
    CasesExperience, // raw/cases-experience/(CaseBoard 结案沉淀的办案经验卡片)
    Sources,         // wiki/sources/
    Topics,          // wiki/topics/
    GapLog,          // gap-log.md(单文件)
    YuandianCache,   // raw/yuandian-cache/(默认**不**搜)
}

impl KbScope {
    fn rel_path(&self) -> &'static str {
        match self {
            KbScope::Root => "",
            KbScope::Notes => "raw/notes",
            KbScope::Companies => "raw/companies",
            KbScope::CasesExperience => "raw/cases-experience",
            KbScope::Sources => "wiki/sources",
            KbScope::Topics => "wiki/topics",
            KbScope::GapLog => "gap-log.md",
            KbScope::YuandianCache => "raw/yuandian-cache",
        }
    }
    fn is_file(&self) -> bool {
        matches!(self, KbScope::GapLog)
    }
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    /// None = 默认 [Notes, Sources, Topics, GapLog](排除 yuandian-cache)
    pub scopes: Option<Vec<KbScope>>,
    pub max_results: usize,
    /// 每条命中里抽多少 char 作为预览片段
    pub snippet_chars: usize,
    /// 大小写敏感(false 用 `(?i)` flag)
    pub case_sensitive: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            scopes: None,
            max_results: 30,
            snippet_chars: 200,
            case_sensitive: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct KbSearchHit {
    pub relative_path: String,
    pub scope: String,
    pub match_count: u32,
    /// 第一个命中位置周围 [-snippet_chars/2, +snippet_chars/2] 文本片段
    pub snippet: String,
    /// 文件修改时间(秒级 Unix epoch)
    pub modified_at: i64,
}

fn default_scopes() -> Vec<KbScope> {
    vec![KbScope::Root]
}

/// 在 KB 下做整库关键词检索。
pub fn search_kb_files(
    kb_root: &Path,
    keyword: &str,
    opts: SearchOptions,
) -> Result<Vec<KbSearchHit>, KbError> {
    if keyword.trim().is_empty() {
        return Ok(Vec::new());
    }
    let root_canonical = kb_root
        .canonicalize()
        .map_err(|_| KbError::NoPath(kb_root.to_path_buf()))?;

    let pattern = if opts.case_sensitive {
        regex::escape(keyword)
    } else {
        format!("(?i){}", regex::escape(keyword))
    };
    let re = regex::Regex::new(&pattern).map_err(|e| {
        KbError::Json(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            e.to_string(),
        )))
    })?;

    let scopes = opts.scopes.clone().unwrap_or_else(default_scopes);
    let mut hits: Vec<KbSearchHit> = Vec::new();

    for scope in scopes {
        let target = if matches!(scope, KbScope::Root) {
            root_canonical.clone()
        } else {
            root_canonical.join(scope.rel_path())
        };
        if !target.exists() {
            continue;
        }
        if scope.is_file() {
            if let Some(hit) = try_match_file(&root_canonical, &target, &re, &opts, scope)? {
                hits.push(hit);
            }
            continue;
        }
        for entry in WalkDir::new(&target)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            if matches!(scope, KbScope::Root) && should_skip_root_search_path(&root_canonical, p) {
                continue;
            }
            // 只搜 .md / .txt(避免误读 .docx 等大二进制)
            let ext_ok = p
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| matches!(e.to_lowercase().as_str(), "md" | "txt"))
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            if let Some(hit) = try_match_file(&root_canonical, p, &re, &opts, scope)? {
                hits.push(hit);
            }
        }
    }

    // 排序:命中次数高 → 修改时间新
    hits.sort_by(|a, b| {
        b.match_count
            .cmp(&a.match_count)
            .then(b.modified_at.cmp(&a.modified_at))
    });
    hits.truncate(opts.max_results);
    Ok(hits)
}

fn should_skip_root_search_path(root_canonical: &Path, path: &Path) -> bool {
    let rel = path
        .strip_prefix(root_canonical)
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"));
    if rel == "raw/yuandian-cache" || rel.starts_with("raw/yuandian-cache/") {
        return true;
    }
    rel.split('/').any(|seg| {
        matches!(
            seg,
            ".git" | "node_modules" | "target" | "dist" | "__MACOSX" | ".DS_Store"
        )
    })
}

fn try_match_file(
    root_canonical: &Path,
    path: &Path,
    re: &regex::Regex,
    opts: &SearchOptions,
    scope: KbScope,
) -> Result<Option<KbSearchHit>, KbError> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_FILE_SIZE {
        return Ok(None);
    }
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Ok(None), // 二进制或编码问题:跳过,不致命
    };
    let mc = re.find_iter(&content).count();
    if mc == 0 {
        return Ok(None);
    }
    let first = re.find(&content).unwrap();
    let half = opts.snippet_chars / 2;
    let start = first.start().saturating_sub(half);
    let end = (first.end() + half).min(content.len());
    let snippet = safe_char_slice(&content, start, end);
    let relative = path
        .strip_prefix(root_canonical)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(Some(KbSearchHit {
        relative_path: relative,
        scope: format!("{:?}", scope),
        match_count: mc as u32,
        snippet,
        modified_at,
    }))
}

/// 字节 offset → 安全的 char 边界 slice。content 是 UTF-8,任意 [start,end) 可能
/// 落在多字节字符中间,会 panic — 这里向外扩到最近的 char boundary。
fn safe_char_slice(s: &str, start: usize, end: usize) -> String {
    let start = floor_char_boundary(s, start);
    let end = ceil_char_boundary(s, end);
    s[start..end].to_string()
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}
fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    let len = s.len();
    while i < len && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// 读 KB 内某个文件。路径必须**相对于 kb_root**,且 canonicalize 后仍在 kb_root 内。
pub fn read_kb_file(
    kb_root: &Path,
    relative_path: &str,
    offset: Option<usize>,
    length: Option<usize>,
) -> Result<String, KbError> {
    let root_canonical = kb_root
        .canonicalize()
        .map_err(|_| KbError::NoPath(kb_root.to_path_buf()))?;
    // 拒绝绝对路径 — LLM 给的路径必须是相对路径
    if Path::new(relative_path).is_absolute() {
        return Err(KbError::PathEscape(relative_path.to_string()));
    }
    let candidate = root_canonical.join(relative_path);
    // canonicalize 必须成功(意味着文件确实存在 + 路径合法)
    let target = candidate
        .canonicalize()
        .map_err(|_| KbError::PathEscape(relative_path.to_string()))?;
    if !target.starts_with(&root_canonical) {
        return Err(KbError::PathEscape(relative_path.to_string()));
    }
    let meta = std::fs::metadata(&target)?;
    if meta.len() > MAX_FILE_SIZE {
        return Err(KbError::FileTooBig {
            path: target.clone(),
            size: meta.len(),
        });
    }
    // 二进制检测:读头 N 字节,看有没有 NUL
    {
        use std::io::Read;
        let mut f = std::fs::File::open(&target)?;
        let mut buf = vec![0u8; BINARY_PEEK_BYTES.min(meta.len() as usize)];
        let _ = f.read(&mut buf)?;
        if buf.contains(&0u8) {
            return Err(KbError::BinaryFile(target));
        }
    }
    let content = std::fs::read_to_string(&target)?;
    let chars: Vec<char> = content.chars().collect();
    let start = offset.unwrap_or(0).min(chars.len());
    let take = length.unwrap_or(10_000).min(chars.len() - start);
    Ok(chars[start..start + take].iter().collect())
}
