//! 元典法规全文进入本地知识库主结构的收口层。
//!
//! `raw/yuandian-cache` 是可复用 API 缓存；成功取得整部法规后，再生成一份带来源、
//! 查询时间、版本/时效元数据的 `raw/notes/` 完整正文。自动写回只到 L1，不伪造
//! 已人工治理的 `wiki/sources` / `wiki/index`；后续由 legal-kb maintenance 复核提升。
//!
//! 文件名带法规 id，CaseBoard 只更新自己生成的稳定文件，不覆盖用户手工整理的同名资料。

use std::path::{Path, PathBuf};

use chrono::Local;
use serde::Serialize;
use serde_json::Value;

use super::cache::LocalKb;
use super::KbError;

#[derive(Debug, Clone, Serialize)]
pub struct RegulationIngestResult {
    pub raw_path: PathBuf,
    pub article_count: usize,
}

/// 把 `rh_fg_detail` 成功响应正式收口到主库。响应缺正文/名称时不写空壳。
pub fn ingest_regulation_detail(
    kb: &LocalKb,
    resp: &Value,
) -> Result<Option<RegulationIngestResult>, KbError> {
    let Some(data) = resp.get("data").and_then(Value::as_object) else {
        return Ok(None);
    };
    let content = data
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let name = data
        .get("fgmc")
        .or_else(|| data.get("title"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (Some(content), Some(name)) = (content, name) else {
        return Ok(None);
    };
    let id = data
        .get("fgid")
        .or_else(|| data.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let effect = str_field(data, "xljb_1").unwrap_or("未标注效力级别");
    let validity = str_field(data, "sxx").unwrap_or("未标注时效性");
    let publish_date = str_field(data, "fbrq").unwrap_or("未标注");
    let implement_date = str_field(data, "ssrq").unwrap_or("未标注");
    let issuer = str_field(data, "fbbm").unwrap_or("未标注");
    let article_count = count_articles(content);
    let historical_only = resp
        .pointer("/_caseboard_validity_context/mode")
        .and_then(Value::as_str)
        == Some("historical_research_only");
    let refer_date = resp
        .pointer("/_caseboard_validity_context/refer_date")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|date| !date.is_empty());
    let validity_scope = if historical_only {
        "historical_research_only"
    } else {
        "current_or_unspecified"
    };
    let now = Local::now();
    let fetched_at = now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let date = now.format("%Y-%m-%d").to_string();

    let raw_dir = kb.root.join("raw/notes");
    std::fs::create_dir_all(&raw_dir)?;

    let id_short: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect();
    let mut suffix = if id_short.is_empty() {
        "unknown".to_string()
    } else {
        id_short
    };
    if historical_only {
        suffix.push_str("_history");
        if let Some(refer_date) = refer_date {
            suffix.push('_');
            suffix.push_str(&sanitize_filename(refer_date));
        }
    }
    let stem = format!("[元典法规] {}_{}", sanitize_filename(name), suffix);
    let raw_path = raw_dir.join(format!("{stem}.md"));

    let raw = format!(
        "---\n\
         type: Raw Legal Material\n\
         kb_level: L1\n\
         wiki_status: pending_review\n\
         validity_scope: {validity_scope}\n\
         ingest_source: yuandian\n\
         refer_date: {}\n\
         fetched_at: {fetched_at}\n\
         ---\n\
         # {name}\n\n\
         > 来源：元典开放平台 `rh_fg_detail` | 查询时间：{fetched_at}\n\
         > 法规 ID：`{id}` | 效力级别：{effect} | 时效性：{validity}\n\
         > 发布日期：{publish_date} | 实施日期：{implement_date} | 发布部门：{issuer}\n\n\
         {content}\n",
        refer_date.unwrap_or("")
    );
    std::fs::write(&raw_path, raw)?;

    let raw_ref = display_kb_path(&kb.root, &raw_path);
    let log_id = if historical_only {
        format!("{id}@{}", refer_date.unwrap_or("historical"))
    } else {
        id.to_string()
    };
    append_log_once(&kb.root, &log_id, name, &date, &raw_ref)?;

    Ok(Some(RegulationIngestResult {
        raw_path,
        article_count,
    }))
}

fn str_field<'a>(data: &'a serde_json::Map<String, Value>, key: &str) -> Option<&'a str> {
    data.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            _ => c,
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .chars()
        .take(100)
        .collect::<String>()
        .trim_matches([' ', '.', '-', '_'])
        .to_string()
}

fn count_articles(content: &str) -> usize {
    content
        .lines()
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

fn display_kb_path(root: &Path, path: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if let Ok(rel) = path.strip_prefix(&home) {
            return format!("~/{}", rel.to_string_lossy().replace('\\', "/"));
        }
    }
    path.strip_prefix(root)
        .map(|rel| root.join(rel).to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
}

fn append_log_once(
    root: &Path,
    id: &str,
    name: &str,
    date: &str,
    raw_ref: &str,
) -> Result<(), KbError> {
    let log_path = root.join("log.md");
    let mut log = std::fs::read_to_string(&log_path).unwrap_or_else(|_| "# log\n".into());
    let marker = format!("元典法规 `{id}`");
    if !log.contains(&marker) {
        if !log.ends_with('\n') {
            log.push('\n');
        }
        log.push_str(&format!(
            "- {date} CaseBoard 写回 L1 {marker}《{name}》→ `{raw_ref}`（wiki_status=pending_review，待知识库维护复核提升）\n"
        ));
        std::fs::write(log_path, log)?;
    }
    Ok(())
}
