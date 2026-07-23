use std::path::Path;

use serde::Serialize;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct LawCatalogEntry {
    pub regulation_name: String,
    pub normalized_name: String,
    pub local_source: String,
    pub article_count: usize,
    pub fgid: Option<String>,
    pub validity: Option<String>,
    pub preview: String,
}

pub fn lookup_law(kb_root: &Path, law_name: &str, query: &str) -> Vec<LawCatalogEntry> {
    let Ok(root) = kb_root.canonicalize() else {
        return Vec::new();
    };
    let wanted = super::semantic::normalize_law_name(law_name);
    if wanted.is_empty() {
        return Vec::new();
    }
    let mut entries = Vec::new();
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path.is_file()
            || !matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("md" | "txt")
            )
        {
            continue;
        }
        let Ok(relative) = path.strip_prefix(&root) else {
            continue;
        };
        let rel = relative.to_string_lossy().replace('\\', "/");
        if should_skip(&rel) {
            continue;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let file_match = super::semantic::normalize_law_name(file_name) == wanted;
        // 目录层只按文件名清单筛候选，避免每次查询读取数千篇正文。入库流程本就以
        // 法规正式名称命名全文；正文标题仅用于二次校验，不能把任意评论文件冒充法规。
        if !file_match {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let title = first_markdown_title(&text).unwrap_or_else(|| file_name.to_string());
        let normalized_name = super::semantic::normalize_law_name(&title);
        if normalized_name != wanted {
            continue;
        }
        let article_count = count_structured_articles(&text);
        if article_count == 0 {
            continue;
        }
        let candidate = LawCatalogEntry {
            regulation_name: title,
            normalized_name,
            local_source: rel,
            article_count,
            fgid: metadata_value(&text, "法规 ID"),
            validity: metadata_value(&text, "时效性"),
            preview: preview(&text, query, 500),
        };
        if !is_current(&candidate) {
            continue;
        }
        entries.push(candidate);
    }
    entries.sort_by(|a, b| {
        is_current(b)
            .cmp(&is_current(a))
            .then(b.article_count.cmp(&a.article_count))
            .then(b.fgid.is_some().cmp(&a.fgid.is_some()))
            .then(source_priority(b).cmp(&source_priority(a)))
            .then(a.local_source.cmp(&b.local_source))
    });
    // 同一法规在 raw/notes、缓存目录和历史副本中可能出现多份；目录层只返回最佳全文，
    // 避免模型把副本数量误认为多部法规。历史版本按明确日期查询时再走专门检索。
    entries.truncate(1);
    entries
}

fn source_priority(entry: &LawCatalogEntry) -> u8 {
    if entry.local_source.starts_with("raw/notes/") {
        2
    } else if entry.local_source.starts_with("raw/yuandian-cache/") {
        1
    } else {
        0
    }
}

fn should_skip(rel: &str) -> bool {
    rel.starts_with("wiki/")
        || rel.starts_with("_inbox/")
        || rel.starts_with("raw/companies/")
        || rel.starts_with("raw/cases-experience/")
        || rel.contains("_deprecated")
        || rel.contains("00_ARCHIVE")
        || rel.ends_with(".raw.json")
        || rel.starts_with("raw/yuandian-cache/SEARCH-")
}

fn is_current(entry: &LawCatalogEntry) -> bool {
    !entry
        .validity
        .as_deref()
        .is_some_and(super::validity::is_inactive_status)
}

fn first_markdown_title(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(String::from)
}

fn count_structured_articles(text: &str) -> usize {
    text.lines()
        .filter(|line| {
            let line = line.trim_start();
            let Some(rest) = line.strip_prefix('第') else {
                return false;
            };
            let Some(pos) = rest.find('条') else {
                return false;
            };
            let number = rest[..pos].trim();
            !number.is_empty()
                && number
                    .chars()
                    .all(|c| c.is_ascii_digit() || "一二三四五六七八九十百千零〇两".contains(c))
        })
        .count()
}

fn metadata_value(text: &str, label: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (_, value) = line.split_once(&format!("{label}："))?;
        let value = value
            .split('|')
            .next()
            .unwrap_or(value)
            .trim()
            .trim_matches('`');
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn preview(text: &str, query: &str, max_chars: usize) -> String {
    let anchor = query
        .split_whitespace()
        .filter(|term| term.chars().count() >= 2)
        .filter_map(|term| text.find(term))
        .min();
    let chars: Vec<char> = text.chars().collect();
    let Some(byte_pos) = anchor else {
        return chars.into_iter().take(max_chars).collect();
    };
    let char_pos = text[..byte_pos].chars().count();
    let start = char_pos.saturating_sub(max_chars / 3);
    let end = (start + max_chars).min(chars.len());
    chars[start..end].iter().collect()
}
