//! V0.2 D5.B · `<CITATIONS>` 协议解析(详 § 6.1 附录 A)。
//!
//! LLM final answer 结尾会 append 一段 `<CITATIONS>...</CITATIONS>` JSON block,
//! 列出本次回答里所有引用的法规 / 案例 / 文档 / KB 来源。前端 CitationsCard 组件按 type 分组渲染。
//!
//! 本模块职责:
//!   1. 从 raw content 中切出 `<CITATIONS>...</CITATIONS>` 段(支持 JSON 数组体)
//!   2. parse 出 `Vec<Citation>` 结构化数据
//!   3. **`type="doc"` 时校验 quote 必须是本案文档的真实子串**(防 LLM 编引用)
//!   4. 返回 (清理后的 content, 引用列表) — content 里 `<CITATIONS>` 块被剥掉,
//!      前端只看到 citations 卡片不会重复看到 JSON
//!
//! 容错原则(LLM 经常写不规范):
//!   - 整个 block 找不到 → 返回 (原 content, 空 vec),**不报错**
//!   - 单条 JSON 缺字段 → 跳过该条,其他正常 parse
//!   - quote 校验失败(type=doc 但 quote 在文档里找不到)→ 该条标记 `verified=false`,
//!     LLM 仍然把引用呈现给用户,但前端 CitationsCard 会标 ⚠️

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 单条引用。`extra` 是 type 特有字段(case 的 court、kb 的 cached_at 等)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Citation {
    #[serde(rename = "ref")]
    pub ref_num: u32,
    /// "law" / "case" / "doc" / "kb_local"
    #[serde(rename = "type")]
    pub kind: String,
    pub source: String,
    #[serde(default)]
    pub quote: Option<String>,
    /// type=case 时填(法院名)
    #[serde(default)]
    pub court: Option<String>,
    /// type=web 时填公开网页 URL。
    #[serde(default)]
    pub url: Option<String>,
    /// `verify_doc_quote` 后端校验结果。type=doc 时:true=quote 在文档里找得到,
    /// false=找不到(LLM 可能编造)。其他 type 默认 true(无法本地校验)。
    #[serde(default = "default_verified")]
    pub verified: bool,
    /// 工具调用 ID(如果该 citation 是工具返回的)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

fn default_verified() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct ParsedCitations {
    /// 清理掉 `<CITATIONS>...</CITATIONS>` 段后的 content(前端 markdown 渲染用)
    pub content_cleaned: String,
    pub citations: Vec<Citation>,
}

/// 切出 `<CITATIONS>...</CITATIONS>` 段并解析。**永不 panic,parse 失败返回空 vec**。
pub fn parse(content: &str) -> ParsedCitations {
    parse_with_doc_filenames(content, &[])
}

/// 带文档名单的校验版:`type=doc` 引用的 quote 会去对应 filename 内容里 grep,
/// 找不到时把 `verified` 标 false。
///
/// `case_docs` 是 `Vec<(filename, full_text)>`(调用方从 sqlite 读 extracted_text_path)。
pub fn parse_with_doc_filenames(content: &str, case_docs: &[(String, String)]) -> ParsedCitations {
    let (cleaned, json_block) = extract_block(content);
    let mut citations = match json_block {
        Some(j) => parse_json_array(&j),
        None => Vec::new(),
    };
    // type=doc 时校验 quote
    for c in &mut citations {
        if c.kind != "doc" {
            continue;
        }
        let Some(quote) = c.quote.as_deref() else {
            // doc 无 quote → 信任 LLM 没引,verified=true 不变(但前端可能想标"无 quote")
            continue;
        };
        let q_trim = quote.trim();
        if q_trim.is_empty() {
            continue;
        }
        // 找到 source 对应的文档(full = 该文档抽取出的 MD 文本,即 AI 实际读到的内容)。
        // quote 用宽松匹配:精确子串 → 去空白+归一化标点 → 拆段全中。**避免把"原文一字不差"
        // 当唯一标准**(AI 常拼接原文多处/全半角标点不同),真机已暴露好引用被误标"可能编造"。
        let doc = case_docs.iter().find(|(fname, _)| fname == &c.source);
        c.verified = match doc {
            Some((_, full)) => doc_quote_matches(full, q_trim),
            None => false, // 文档名都对不上 → LLM 编的
        };
    }
    ParsedCitations {
        content_cleaned: cleaned,
        citations,
    }
}

/// 按引用到的文件名懒加载正文进行校验。
///
/// 大案件可能有上百份、数百万字正文；聊天开始前把全部 MD 读进内存，只为最终可能出现的
/// 1~3 条引用做核验，会造成明显延迟和内存峰值。本入口先解析 citation，再只读取实际引用的文件。
pub fn parse_with_doc_paths(content: &str, case_docs: &[(String, String)]) -> ParsedCitations {
    let (cleaned, json_block) = extract_block(content);
    let mut citations = match json_block {
        Some(j) => parse_json_array(&j),
        None => Vec::new(),
    };
    let mut loaded: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    for citation in &mut citations {
        if citation.kind != "doc" {
            continue;
        }
        let Some(quote) = citation
            .quote
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty())
        else {
            continue;
        };
        let Some((_, path)) = case_docs
            .iter()
            .find(|(filename, _)| filename == &citation.source)
        else {
            citation.verified = false;
            continue;
        };
        let full = loaded
            .entry(path.clone())
            .or_insert_with(|| std::fs::read_to_string(path).ok());
        citation.verified = full
            .as_deref()
            .is_some_and(|content| doc_quote_matches(content, quote));
    }
    ParsedCitations {
        content_cleaned: cleaned,
        citations,
    }
}

/// 归一化用于宽松匹配:去掉所有空白 + 统一常见全/半角标点,降低"原文一字不差"误判。
fn normalize_for_match(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            '（' => '(',
            '）' => ')',
            '：' => ':',
            '，' => ',',
            '；' => ';',
            '、' => ',',
            '“' | '”' | '"' => '"',
            '‘' | '’' => '\'',
            _ => c,
        })
        .collect()
}

/// 文档 quote 宽松校验:`full` = 文档抽取文本,`quote` = LLM 引用。
/// 三级:① 精确子串 ② 去空白+归一化标点后子串 ③ 按句号/换行/分号拆段,各段(≥6 字)都能
/// 在原文找到即视为真(应对 LLM 拼接原文多处)。三级都不中才判"未找到"(疑似编造)。
fn doc_quote_matches(full: &str, quote: &str) -> bool {
    if full.contains(quote) {
        return true;
    }
    let nf = normalize_for_match(full);
    let nq = normalize_for_match(quote);
    if nq.is_empty() {
        return true;
    }
    if nf.contains(&nq) {
        return true;
    }
    let segs: Vec<String> = quote
        .split(['。', '\n', '；', ';'])
        .map(normalize_for_match)
        .filter(|s| s.chars().count() >= 6)
        .collect();
    !segs.is_empty() && segs.iter().all(|s| nf.contains(s))
}

/// 从 content 中切出 `<CITATIONS>...</CITATIONS>` 内层 JSON 字符串,返回 (清理后的 content, JSON 字符串)。
fn extract_block(content: &str) -> (String, Option<String>) {
    let open = match content.rfind("<CITATIONS>") {
        Some(p) => p,
        None => return (content.to_string(), None),
    };
    let body_start = open + "<CITATIONS>".len();
    // close 必须在 open 之后
    let close = match content[body_start..].find("</CITATIONS>") {
        Some(p) => body_start + p,
        None => {
            // 未闭合(LLM 截断 / 忘写闭合标签):从 `<CITATIONS>` 一路剥到结尾,整段当 block
            // 丢弃。否则残缺的 JSON 数组会泄漏进正文 → 入库 artifact、导出 Word 都带一坨
            //(`[ { ... ` 缩进还会被 Markdown 渲染成黑底 code block,真机已暴露)。
            let json_block = content[body_start..].trim().to_string();
            let cleaned = content[..open].trim_end().to_string();
            return (cleaned, Some(json_block));
        }
    };
    let json_block = content[body_start..close].trim().to_string();
    // 清理后的 content:open 之前 + close 之后(去掉 `</CITATIONS>` 标签)
    let after_close = close + "</CITATIONS>".len();
    let cleaned = format!(
        "{}{}",
        content[..open].trim_end(),
        content[after_close..].trim_start_matches('\n')
    );
    (cleaned.trim_end().to_string(), Some(json_block))
}

/// 解析 JSON 数组(允许内部有注释 / 不规范);单条解析失败跳过。
fn parse_json_array(raw: &str) -> Vec<Citation> {
    // 不容忍 JSON 不规范 — 但允许 LLM 在前后加 ```json fence。先剥 fence。
    let stripped = raw
        .strip_prefix("```json")
        .or_else(|| raw.strip_prefix("```"))
        .map(|s| s.trim_end_matches("```").trim())
        .unwrap_or(raw)
        .trim();
    let Ok(v) = serde_json::from_str::<Value>(stripped) else {
        return Vec::new();
    };
    let Some(arr) = v.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| serde_json::from_value::<Citation>(item.clone()).ok())
        .collect()
}
