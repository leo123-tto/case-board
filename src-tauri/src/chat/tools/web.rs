//! 通用联网检索与网页读取工具。
//!
//! 设计边界:
//! - 只读,不写案件文件/数据库;
//! - 只允许公开 http(s) URL,拒绝 localhost / 内网 / link-local 等地址;
//! - 搜索与抓页都只是线索来源,法律结论仍应回到元典/官方来源核验。

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use reqwest::Url;
use serde_json::{json, Value};

use super::{opt_str, opt_u32, require_str, Tool, ToolContext, ToolError, ToolResult};

const WEB_USER_AGENT: &str = concat!("CaseBoard/", env!("CARGO_PKG_VERSION"), " legal-assistant");
const WEB_TIMEOUT_SECS: u64 = 15;
const SEARCH_MAX_RESULTS: usize = 10;
const FETCH_MAX_CHARS: usize = 60_000;
const DUCKDUCKGO_HTML_ENDPOINT: &str = "https://html.duckduckgo.com/html/";

pub struct WebSearch;

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        include_str!("descriptions/web_search.md")
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "公开互联网检索关键词。不要放案件隐私、当事人身份信息、文件路径、API key 或完整案情。"
                },
                "site": {
                    "type": "string",
                    "description": "可选,限定站点域名,如 court.gov.cn / gov.cn / pkulaw.com。不要带 http(s)://。"
                },
                "max_results": {
                    "type": "integer",
                    "description": "默认 5,最大 10。"
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let query = require_str(args, "query")?.trim();
        let max_results = opt_u32(args, "max_results")
            .map(|n| n as usize)
            .unwrap_or(5)
            .clamp(1, SEARCH_MAX_RESULTS);
        let mut q = query.to_string();
        if let Some(site) = opt_str(args, "site")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let site = site
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_matches('/');
            if !site.is_empty() {
                q = format!("site:{site} {q}");
            }
        }

        let url = build_duckduckgo_search_url(&q)?;
        let client = web_client()?;
        let html = client
            .get(url)
            .send()
            .await
            .map_err(|e| ToolError::Runtime(format!("联网搜索失败:{e}")))?
            .error_for_status()
            .map_err(|e| ToolError::Runtime(format!("联网搜索 HTTP 错误:{e}")))?
            .text()
            .await
            .map_err(|e| ToolError::Runtime(format!("读取搜索响应失败:{e}")))?;
        build_search_result(query, &html, max_results)
    }
}

fn build_duckduckgo_search_url(query: &str) -> Result<Url, ToolError> {
    Url::parse_with_params(DUCKDUCKGO_HTML_ENDPOINT, &[("q", query), ("kl", "cn-zh")])
        .map_err(|e| ToolError::Runtime(format!("构造搜索 URL 失败:{e}")))
}

fn build_search_result(
    query: &str,
    html: &str,
    max_results: usize,
) -> Result<ToolResult, ToolError> {
    let results = extract_duckduckgo_results(html, max_results);
    if results.is_empty() {
        return Err(ToolError::Runtime(
            "DuckDuckGo 没有返回可用结果；请停止继续联网搜索，改用本地知识库、元典或已有材料完成分析"
                .into(),
        ));
    }
    Ok(ToolResult::plain(
        serde_json::to_string_pretty(&json!({
            "engine": "DuckDuckGo HTML",
            "query": query,
            "results": results,
            "_note": "互联网搜索结果只作为线索。需要正文时继续调用 web_fetch。涉及法律依据、裁判口径或政策发布日期时,优先用元典/官方页面进一步核验;不得把案件隐私作为搜索词。"
        }))
        .unwrap_or_else(|_| "{}".into()),
    ))
}

pub struct WebFetch;

#[async_trait]
impl Tool for WebFetch {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        include_str!("descriptions/web_fetch.md")
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "要读取的公开 http(s) 网页 URL。localhost、内网地址、file:// 等会被拒绝。"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "返回正文字符上限,默认 20000,最大 60000。"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let url = validate_public_http_url_for_request(require_str(args, "url")?).await?;
        let max_chars = opt_u32(args, "max_chars")
            .map(|n| n as usize)
            .unwrap_or(20_000)
            .clamp(1_000, FETCH_MAX_CHARS);
        let client = web_client()?;
        let resp = fetch_public_url(&client, url).await?;
        let resp = resp
            .error_for_status()
            .map_err(|e| ToolError::Runtime(format!("网页 HTTP 错误:{e}")))?;
        let final_url = resp.url().to_string();
        let status = resp.status().as_u16();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !is_supported_content_type(&content_type) {
            return Err(ToolError::Runtime(format!(
                "暂不读取该内容类型:{content_type};请提供网页文本链接"
            )));
        }
        let body = resp
            .text()
            .await
            .map_err(|e| ToolError::Runtime(format!("读取网页正文失败:{e}")))?;
        let title = extract_title(&body);
        let text = html_to_text(&body);
        let (text, truncated) = truncate_chars(&text, max_chars);

        Ok(ToolResult::plain(
            serde_json::to_string_pretty(&json!({
                "url": final_url,
                "status": status,
                "content_type": content_type,
                "title": title,
                "text": text,
                "truncated": truncated,
                "_note": "网页内容来自公开互联网。引用时在 <CITATIONS> 使用 type=web,并保留 url;重要法律结论仍需用权威来源核验。"
            }))
            .unwrap_or_else(|_| "{}".into()),
        ))
    }
}

fn web_client() -> Result<reqwest::Client, ToolError> {
    reqwest::Client::builder()
        .user_agent(WEB_USER_AGENT)
        .timeout(Duration::from_secs(WEB_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ToolError::Runtime(format!("初始化联网客户端失败:{e}")))
}

async fn fetch_public_url(
    client: &reqwest::Client,
    mut url: Url,
) -> Result<reqwest::Response, ToolError> {
    for redirect_count in 0..=5 {
        ensure_url_resolves_public(&url).await?;
        let resp = client
            .get(url.clone())
            .send()
            .await
            .map_err(|e| ToolError::Runtime(format!("网页读取失败:{e}")))?;
        if !resp.status().is_redirection() {
            return Ok(resp);
        }
        if redirect_count == 5 {
            return Err(ToolError::Runtime("网页跳转次数过多".into()));
        }
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ToolError::Runtime("网页返回跳转但缺少 Location".into()))?;
        url = validate_redirect_target(&url, location)?;
    }
    Err(ToolError::Runtime("网页跳转处理失败".into()))
}

fn is_supported_content_type(content_type: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    ct.is_empty()
        || ct.contains("text/")
        || ct.contains("application/json")
        || ct.contains("application/xml")
        || ct.contains("application/xhtml")
        || ct.contains("+xml")
}

fn validate_public_http_url(raw: &str) -> Result<Url, ToolError> {
    let url =
        Url::parse(raw.trim()).map_err(|e| ToolError::InvalidArgs(format!("URL 无效:{e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ToolError::InvalidArgs(
            "仅支持公开 http(s) URL,不支持 file:// 或其它协议".into(),
        ));
    }
    let Some(host) = url.host_str() else {
        return Err(ToolError::InvalidArgs("URL 缺少 host".into()));
    };
    if host_is_blocked(host) {
        return Err(ToolError::InvalidArgs(
            "拒绝读取 localhost、内网、link-local 或保留地址".into(),
        ));
    }
    Ok(url)
}

pub(crate) async fn validate_public_http_url_for_request(raw: &str) -> Result<Url, ToolError> {
    let url = validate_public_http_url(raw)?;
    ensure_url_resolves_public(&url).await?;
    Ok(url)
}

async fn ensure_url_resolves_public(url: &Url) -> Result<(), ToolError> {
    let Some(host) = url.host_str() else {
        return Err(ToolError::InvalidArgs("URL 缺少 host".into()));
    };
    let port = url.port_or_known_default().unwrap_or(80);
    let addrs = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| ToolError::Runtime(format!("解析网页域名失败:{e}")))?
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        return Err(ToolError::Runtime("网页域名没有解析结果".into()));
    }
    if addrs.iter().any(|addr| ip_is_blocked(addr.ip())) {
        return Err(ToolError::InvalidArgs(
            "拒绝读取解析到 localhost、内网、link-local 或保留地址的 URL".into(),
        ));
    }
    Ok(())
}

fn validate_redirect_target(base: &Url, location: &str) -> Result<Url, ToolError> {
    let next = base
        .join(location)
        .map_err(|e| ToolError::Runtime(format!("网页跳转 URL 无效:{e}")))?;
    validate_public_http_url(next.as_str())
}

fn host_is_blocked(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") {
        return true;
    }
    match h.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ipv4_is_blocked(ip),
        Ok(IpAddr::V6(ip)) => ipv6_is_blocked(ip),
        Err(_) => false,
    }
}

fn ip_is_blocked(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ipv4_is_blocked(ip),
        IpAddr::V6(ip) => ipv6_is_blocked(ip),
    }
}

fn ipv4_is_blocked(ip: Ipv4Addr) -> bool {
    let [a, b, _, _] = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || (a == 100 && (64..=127).contains(&b))
        || a >= 224
}

fn ipv6_is_blocked(ip: Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    ip.is_loopback()
        || ip.is_unspecified()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first & 0xff00) == 0xff00
}

fn extract_duckduckgo_results(html: &str, max_results: usize) -> Vec<Value> {
    let anchor_re = Regex::new(r#"(?is)<a\b([^>]*)>(.*?)</a>"#).expect("valid anchor regex");
    let href_re =
        Regex::new(r#"(?is)\bhref\s*=\s*(?:"([^"]+)"|'([^']+)')"#).expect("valid href regex");
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for caps in anchor_re.captures_iter(html) {
        let attrs = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let body = caps.get(2).map(|m| m.as_str()).unwrap_or("");
        if !attrs.contains("result__a") && !attrs.contains("uddg=") {
            continue;
        }
        let Some(href_caps) = href_re.captures(attrs) else {
            continue;
        };
        let href = href_caps
            .get(1)
            .or_else(|| href_caps.get(2))
            .map(|m| m.as_str())
            .unwrap_or("");
        let Some(url) = normalize_search_result_url(href) else {
            continue;
        };
        if !seen.insert(url.clone()) {
            continue;
        }
        let title = strip_tags(body);
        if title.is_empty() {
            continue;
        }
        out.push(json!({ "title": title, "url": url }));
        if out.len() >= max_results {
            break;
        }
    }
    out
}

fn normalize_search_result_url(href: &str) -> Option<String> {
    let href = decode_html_entities(href);
    let href = href.trim();
    let full = if href.starts_with("//") {
        format!("https:{href}")
    } else if href.starts_with('/') {
        format!("https://duckduckgo.com{href}")
    } else {
        href.to_string()
    };
    let url = Url::parse(&full).ok()?;
    if url.domain().is_some_and(|d| d.ends_with("duckduckgo.com")) && url.path().starts_with("/l/")
    {
        if let Some(target) = url
            .query_pairs()
            .find(|(k, _)| k == "uddg")
            .map(|(_, v)| v.into_owned())
        {
            return Some(target);
        }
    }
    if matches!(url.scheme(), "http" | "https") {
        Some(url.to_string())
    } else {
        None
    }
}

fn extract_title(html: &str) -> Option<String> {
    let re = Regex::new(r#"(?is)<title[^>]*>(.*?)</title>"#).expect("valid title regex");
    re.captures(html)
        .and_then(|c| c.get(1))
        .map(|m| strip_tags(m.as_str()))
        .filter(|s| !s.is_empty())
}

fn html_to_text(input: &str) -> String {
    let mut s = input.to_string();
    for pat in [
        r"(?is)<script[^>]*>.*?</script>",
        r"(?is)<style[^>]*>.*?</style>",
        r"(?is)<noscript[^>]*>.*?</noscript>",
    ] {
        let re = Regex::new(pat).expect("valid block regex");
        s = re.replace_all(&s, " ").to_string();
    }
    strip_tags(&s)
}

fn strip_tags(input: &str) -> String {
    let tag_re = Regex::new(r"(?is)<[^>]+>").expect("valid tag regex");
    let text = tag_re.replace_all(input, " ");
    normalize_ws(&decode_html_entities(&text))
}

fn decode_html_entities(input: &str) -> String {
    let mut s = input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">");
    let numeric_re = Regex::new(r"&#(x[0-9a-fA-F]+|\d+);").expect("valid entity regex");
    s = numeric_re
        .replace_all(&s, |caps: &regex::Captures<'_>| {
            let raw = caps.get(1).map(|m| m.as_str()).unwrap_or("");
            let value = if let Some(hex) = raw.strip_prefix('x').or_else(|| raw.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                raw.parse::<u32>().ok()
            };
            value
                .and_then(char::from_u32)
                .map(|c| c.to_string())
                .unwrap_or_else(|| caps.get(0).unwrap().as_str().to_string())
        })
        .to_string();
    s
}

fn normalize_ws(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(input: &str, max_chars: usize) -> (String, bool) {
    if input.chars().count() <= max_chars {
        return (input.to_string(), false);
    }
    (input.chars().take(max_chars).collect(), true)
}
