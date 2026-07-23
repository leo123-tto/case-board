//! 整库关键词检索 + 文件读取(带路径穿越防护)。
//!
//! 通用工具默认可在 `local_kb_root` 下做宽口径 `.md` / `.txt` 搜索；法规、案例、
//! 企业等外部调用门禁会改用明确 scope 分层检索，不把自建目录当作同等可信原文。
//! **排除**：元典缓存、迁移收件箱、归档和技术目录；详情缓存确需搜索时显式 include。
//!
//! `read_kb_file` 的安全约束:
//!   1. `canonicalize` + `starts_with` 防穿越(LLM 给 `../../etc/passwd` 直接拒)
//!   2. 文件大小上限 5MB
//!   3. 二进制检测:open 后读头 512 字节,含 NUL 拒绝

use std::collections::{HashMap, HashSet};
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
    /// None = 默认宽口径 Root（仍排除元典缓存、收件箱、归档和技术目录）。
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
    pub title: String,
    pub doc_type: String,
    /// 轻量 BM25 分数（越高越相关）；叠加标题、法规名、精确条号结构加权。
    pub score: f64,
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

    let query = LexicalQuery::new(keyword, opts.case_sensitive);
    if query.terms.is_empty() {
        return Ok(Vec::new());
    }

    let scopes = opts.scopes.clone().unwrap_or_else(default_scopes);
    let mut docs: Vec<SearchDoc> = Vec::new();
    let mut seen_paths = HashSet::new();

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
            if seen_paths.insert(target.clone()) {
                if let Some(doc) = load_search_doc(&root_canonical, &target, &query, scope)? {
                    docs.push(doc);
                }
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
            let path_buf = p.to_path_buf();
            if seen_paths.insert(path_buf) {
                if let Some(doc) = load_search_doc(&root_canonical, p, &query, scope)? {
                    docs.push(doc);
                }
            }
        }
    }

    let doc_count = docs.len().max(1) as f64;
    let avg_doc_len = docs.iter().map(|d| d.doc_len as f64).sum::<f64>() / doc_count;
    let mut doc_freq: HashMap<&str, usize> = HashMap::new();
    for term in &query.terms {
        let count = docs
            .iter()
            .filter(|d| d.search_text.contains(term.as_str()))
            .count();
        doc_freq.insert(term, count);
    }

    let mut hits: Vec<KbSearchHit> = docs
        .into_iter()
        .map(|doc| score_doc(doc, &query, &doc_freq, doc_count, avg_doc_len, &opts))
        .filter(|hit| hit.score > 0.0)
        .collect();

    // 排序:BM25/结构分 → 修改时间新。标题/来源页/精确条号已进入 score，不再按词频蛮排。
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
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
            "_inbox"
                | "_deprecated"
                | "00_ARCHIVE"
                | "archive"
                | ".git"
                | "node_modules"
                | "target"
                | "dist"
                | "__MACOSX"
                | ".DS_Store"
        )
    })
}

struct SearchDoc {
    relative_path: String,
    scope: KbScope,
    title: String,
    doc_type: String,
    content: String,
    search_text: String,
    doc_len: usize,
    modified_at: i64,
}

fn load_search_doc(
    root_canonical: &Path,
    path: &Path,
    query: &LexicalQuery,
    scope: KbScope,
) -> Result<Option<SearchDoc>, KbError> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_FILE_SIZE {
        return Ok(None);
    }
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return Ok(None), // 二进制或编码问题:跳过,不致命
    };
    if super::validity::is_inactive_regulation_text(&content) {
        return Ok(None);
    }
    let relative = path
        .strip_prefix(root_canonical)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string_lossy().into_owned());
    if relative.starts_with("raw/yuandian-cache/")
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("SEARCH-"))
    {
        return Ok(None);
    }
    let title = extract_title(path, &content);
    let search_text = if query.case_sensitive {
        format!("{relative}\n{title}\n{content}")
    } else {
        format!("{relative}\n{title}\n{content}").to_lowercase()
    };
    if !query.terms.iter().any(|term| search_text.contains(term)) {
        return Ok(None);
    }
    let modified_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let doc_type = doc_type_for(&relative);
    Ok(Some(SearchDoc {
        relative_path: relative,
        scope,
        title,
        doc_type,
        doc_len: content.chars().count().max(1),
        content,
        search_text,
        modified_at,
    }))
}

fn score_doc(
    doc: SearchDoc,
    query: &LexicalQuery,
    doc_freq: &HashMap<&str, usize>,
    doc_count: f64,
    avg_doc_len: f64,
    opts: &SearchOptions,
) -> KbSearchHit {
    const K1: f64 = 1.2;
    const B: f64 = 0.75;
    let title_cmp = if query.case_sensitive {
        doc.title.clone()
    } else {
        doc.title.to_lowercase()
    };
    let path_cmp = if query.case_sensitive {
        doc.relative_path.clone()
    } else {
        doc.relative_path.to_lowercase()
    };
    let content_cmp = if query.case_sensitive {
        doc.content.clone()
    } else {
        doc.content.to_lowercase()
    };
    let mut score = 0.0;
    let mut match_count = 0u32;
    for term in &query.terms {
        let body_tf = count_occurrences(&content_cmp, term);
        let title_tf = count_occurrences(&title_cmp, term);
        let path_tf = count_occurrences(&path_cmp, term);
        let tf = body_tf + title_tf * 5 + path_tf * 3;
        // 对外 match_count 保持“正文命中次数”语义；标题/路径只参与 score 加权。
        match_count = match_count.saturating_add(body_tf as u32);
        if tf == 0 {
            continue;
        }
        let df = *doc_freq.get(term.as_str()).unwrap_or(&0) as f64;
        let idf = (1.0 + (doc_count - df + 0.5) / (df + 0.5)).ln();
        let len_norm = 1.0 - B + B * (doc.doc_len as f64 / avg_doc_len.max(1.0));
        score += idf * ((tf as f64 * (K1 + 1.0)) / (tf as f64 + K1 * len_norm));
    }

    if !query.exact.is_empty() {
        if title_cmp.contains(&query.exact) {
            score += 24.0;
        } else if content_cmp.contains(&query.exact) {
            score += 8.0;
        }
    }
    if let Some(law_hint) = query.law_hint.as_deref() {
        if title_cmp.contains(law_hint) || path_cmp.contains(law_hint) {
            score += 24.0;
            if query
                .article_markers
                .iter()
                .any(|marker| content_cmp.contains(marker))
            {
                score += 60.0;
            }
        }
    }
    score += match doc.doc_type.as_str() {
        "topic" => 8.0,
        "source" => 6.0,
        "custom" => 2.0,
        "raw" => 0.0,
        "yuandian" => -1.0,
        _ => 0.0,
    };

    let snippet = best_snippet(&doc.content, query, opts.snippet_chars);
    KbSearchHit {
        relative_path: doc.relative_path,
        scope: format!("{:?}", doc.scope),
        title: doc.title,
        doc_type: doc.doc_type,
        score: (score * 1000.0).round() / 1000.0,
        match_count,
        snippet,
        modified_at: doc.modified_at,
    }
}

#[derive(Debug)]
struct LexicalQuery {
    exact: String,
    terms: Vec<String>,
    law_hint: Option<String>,
    article_markers: Vec<String>,
    case_sensitive: bool,
}

impl LexicalQuery {
    fn new(raw: &str, case_sensitive: bool) -> Self {
        let normalized = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        let exact = if case_sensitive {
            normalized.clone()
        } else {
            normalized.to_lowercase()
        };
        let mut terms = tokenize(&exact);
        let article_re = regex::Regex::new(r"第?\s*(\d{1,4})\s*条").expect("static regex");
        let chinese_article_re =
            regex::Regex::new(r"第[零〇一二三四五六七八九十百千万两]+条").expect("static regex");
        let mut law_hint = None;
        let mut article_markers = Vec::new();
        if let Some(caps) = article_re.captures(&exact) {
            if let Some(n) = caps.get(1).and_then(|m| m.as_str().parse::<u32>().ok()) {
                article_markers.push(format!("第{n}条"));
                if let Some(chinese) = arabic_to_chinese(n) {
                    article_markers.push(format!("第{chinese}条"));
                }
                terms.push(n.to_string());
            }
            if let Some(m) = caps.get(0) {
                let hint = exact[..m.start()]
                    .trim_matches(|c: char| c.is_whitespace() || "《》“”\"'，,：:".contains(c))
                    .to_string();
                if !hint.is_empty() {
                    terms.extend(tokenize(&hint));
                    law_hint = Some(hint);
                }
            }
        } else if let Some(m) = chinese_article_re.find(&exact) {
            let marker = m.as_str().to_string();
            article_markers.push(marker.clone());
            terms.push(marker);
            let hint = exact[..m.start()]
                .trim_matches(|c: char| c.is_whitespace() || "《》“”\"'，,：:".contains(c))
                .to_string();
            if !hint.is_empty() {
                terms.extend(tokenize(&hint));
                law_hint = Some(hint);
            }
        }
        terms.extend(article_markers.iter().cloned());
        terms.sort();
        terms.dedup();
        Self {
            exact,
            terms,
            law_hint,
            article_markers,
            case_sensitive,
        }
    }
}

fn tokenize(text: &str) -> Vec<String> {
    let mut groups = Vec::new();
    let mut buf = String::new();
    let mut cjk_mode = None;
    let flush = |buf: &mut String, cjk: Option<bool>, out: &mut Vec<String>| {
        if buf.is_empty() {
            return;
        }
        if cjk == Some(true) {
            let chars: Vec<char> = buf.chars().collect();
            if chars.len() <= 8 {
                out.push(buf.clone());
            }
            if chars.len() == 1 {
                out.push(buf.clone());
            } else {
                for pair in chars.windows(2) {
                    out.push(pair.iter().collect());
                }
            }
        } else {
            out.push(buf.to_lowercase());
        }
        buf.clear();
    };
    for c in text.chars() {
        let is_cjk = ('\u{3400}'..='\u{9fff}').contains(&c);
        let is_word = is_cjk || c.is_ascii_alphanumeric() || c == '_';
        if !is_word {
            flush(&mut buf, cjk_mode, &mut groups);
            cjk_mode = None;
            continue;
        }
        if cjk_mode.is_some_and(|mode| mode != is_cjk) {
            flush(&mut buf, cjk_mode, &mut groups);
        }
        cjk_mode = Some(is_cjk);
        buf.push(c);
    }
    flush(&mut buf, cjk_mode, &mut groups);
    groups.retain(|t| !t.trim().is_empty() && !matches!(t.as_str(), "的" | "了" | "和" | "与"));
    groups
}

fn arabic_to_chinese(n: u32) -> Option<String> {
    if n == 0 || n > 9999 {
        return None;
    }
    const DIGITS: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    const UNITS: [&str; 4] = ["", "十", "百", "千"];
    let mut out = String::new();
    let mut zero_pending = false;
    for pos in (0..4).rev() {
        let divisor = 10u32.pow(pos);
        let digit = (n / divisor) % 10;
        if digit == 0 {
            if !out.is_empty() && !n.is_multiple_of(divisor) {
                zero_pending = true;
            }
            continue;
        }
        if zero_pending {
            out.push('零');
            zero_pending = false;
        }
        if !(digit == 1 && pos == 1 && out.is_empty()) {
            out.push_str(DIGITS[digit as usize]);
        }
        out.push_str(UNITS[pos as usize]);
    }
    Some(out)
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    haystack.match_indices(needle).count()
}

fn extract_title(path: &Path, content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut start = 0usize;
    if lines.first().is_some_and(|l| l.trim() == "---") {
        if let Some(end) = lines.iter().skip(1).position(|l| l.trim() == "---") {
            for line in &lines[1..end + 1] {
                let (key, value) = line.split_once(':').unwrap_or(("", ""));
                if key.trim() == "title" && !value.trim().is_empty() {
                    return value.trim().trim_matches(['\'', '"']).to_string();
                }
            }
            start = end + 2;
        }
    }
    lines
        .iter()
        .skip(start)
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("未命名")
                .to_string()
        })
}

fn doc_type_for(relative: &str) -> String {
    let rel = relative.replace('\\', "/");
    if rel.starts_with("wiki/topics/") {
        "topic"
    } else if rel.starts_with("wiki/sources/") {
        "source"
    } else if rel.starts_with("raw/yuandian-cache/") {
        "yuandian"
    } else if rel.starts_with("raw/") || rel == "gap-log.md" {
        "raw"
    } else {
        "custom"
    }
    .to_string()
}

fn best_snippet(content: &str, query: &LexicalQuery, snippet_chars: usize) -> String {
    let cmp = if query.case_sensitive {
        content.to_string()
    } else {
        content.to_lowercase()
    };
    let anchor = std::iter::once(query.exact.as_str())
        .chain(query.article_markers.iter().map(String::as_str))
        .chain(query.terms.iter().map(String::as_str))
        .filter(|s| !s.is_empty())
        .find_map(|needle| cmp.find(needle).map(|pos| (pos, needle.len())));
    let Some((pos, len)) = anchor else {
        return content.chars().take(snippet_chars).collect();
    };
    let half = snippet_chars / 2;
    let start = pos.saturating_sub(half);
    let end = (pos + len + half).min(content.len());
    safe_char_slice(content, start, end)
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
    let historical_only = super::validity::is_inactive_regulation_text(&content);
    let chars: Vec<char> = content.chars().collect();
    let start = offset.unwrap_or(0).min(chars.len());
    let take = length.unwrap_or(10_000).min(chars.len() - start);
    let selected: String = chars[start..start + take].iter().collect();
    if historical_only {
        Ok(format!(
            "> [!WARNING] historical_research_only：该文件标注为失效、废止或尚未生效，仅供历史适用法研究；不得作为现行有效依据，引用前必须核实施行区间和后续修订。\n\n{selected}"
        ))
    } else {
        Ok(selected)
    }
}
