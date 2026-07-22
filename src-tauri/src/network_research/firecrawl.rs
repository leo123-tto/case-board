use reqwest::Method;
use serde::Serialize;
use serde_json::{json, Value};

use super::http::{request_json, research_client, validate_research_query, ResearchError};
use super::{ResearchDocument, ResearchSearchResult};

const FIRECRAWL_BASE: &str = "https://api.firecrawl.dev";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FirecrawlCreditUsage {
    pub total: u64,
    pub used: u64,
    pub remaining: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FirecrawlScrapeResult {
    pub provider: String,
    pub proxy: String,
    pub document: ResearchDocument,
    pub truncated: bool,
}

pub struct FirecrawlClient {
    base_url: String,
    client: reqwest::Client,
}

impl FirecrawlClient {
    pub fn new() -> Result<Self, ResearchError> {
        Self::with_base_url(FIRECRAWL_BASE)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, ResearchError> {
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: research_client()?,
        })
    }

    pub async fn credit_usage(&self, key: &str) -> Result<FirecrawlCreditUsage, ResearchError> {
        let value = request_json(
            &self.client,
            Method::GET,
            &format!("{}/v2/team/credit-usage", self.base_url),
            "Firecrawl",
            key,
            None,
        )
        .await?;
        parse_credit_usage(value)
    }

    pub async fn search(
        &self,
        key: &str,
        query: &str,
        limit: usize,
    ) -> Result<ResearchSearchResult, ResearchError> {
        let query = validate_research_query(query)?;
        let body = json!({
            "query": query,
            "limit": limit.clamp(1, 10),
            "sources": ["web"],
            "country": "CN",
            "ignoreInvalidURLs": true
        });
        let value = request_json(
            &self.client,
            Method::POST,
            &format!("{}/v2/search", self.base_url),
            "Firecrawl",
            key,
            Some(&body),
        )
        .await?;
        parse_search_response(value)
    }

    pub async fn scrape(
        &self,
        key: &str,
        url: &str,
        proxy: &str,
    ) -> Result<FirecrawlScrapeResult, ResearchError> {
        crate::chat::tools::web::validate_public_http_url_for_request(url)
            .await
            .map_err(|error| ResearchError::new("invalid_url", error.to_string()))?;
        let proxy = match proxy {
            "auto" | "basic" | "enhanced" => proxy,
            _ => {
                return Err(ResearchError::new(
                    "invalid_args",
                    "Firecrawl proxy 只支持 auto、basic、enhanced",
                ));
            }
        };
        let mut body = json!({
            "url": url,
            "formats": ["markdown"],
            "proxy": proxy,
            "onlyMainContent": true
        });
        if proxy == "enhanced" {
            body["waitFor"] = json!(3_000);
            body["location"] = json!({"country": "CN", "languages": ["zh-CN"]});
        }
        let value = request_json(
            &self.client,
            Method::POST,
            &format!("{}/v2/scrape", self.base_url),
            "Firecrawl",
            key,
            Some(&body),
        )
        .await?;
        parse_scrape_response(value, proxy)
    }
}

pub fn parse_credit_usage(value: Value) -> Result<FirecrawlCreditUsage, ResearchError> {
    let data = value.get("data").unwrap_or(&value);
    let total = data
        .get("planCredits")
        .or_else(|| data.get("totalCredits"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let remaining = data
        .get("remainingCredits")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ResearchError::new("invalid_response", "Firecrawl 响应缺少 remainingCredits")
        })?;
    let used = data
        .get("usedCredits")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| total.saturating_sub(remaining));
    Ok(FirecrawlCreditUsage {
        total,
        used,
        remaining,
    })
}

pub fn parse_scrape_response(
    value: Value,
    proxy: &str,
) -> Result<FirecrawlScrapeResult, ResearchError> {
    let data = value.get("data").unwrap_or(&value);
    let markdown = data
        .get("markdown")
        .and_then(Value::as_str)
        .ok_or_else(|| ResearchError::new("empty_content", "Firecrawl 没有返回网页正文"))?;
    let metadata = data.get("metadata").unwrap_or(&Value::Null);
    let (text, truncated) = super::http::truncate_chars(markdown, 60_000);
    Ok(FirecrawlScrapeResult {
        provider: "firecrawl".into(),
        proxy: proxy.into(),
        document: ResearchDocument {
            title: metadata
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("未命名网页")
                .to_string(),
            url: metadata
                .get("sourceURL")
                .or_else(|| metadata.get("url"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            published_date: metadata
                .get("publishedTime")
                .or_else(|| metadata.get("publishedDate"))
                .and_then(Value::as_str)
                .map(str::to_string),
            author: metadata
                .get("author")
                .or_else(|| metadata.get("ogSiteName"))
                .and_then(Value::as_str)
                .map(str::to_string),
            text,
            highlights: Vec::new(),
        },
        truncated,
    })
}

fn parse_search_response(value: Value) -> Result<ResearchSearchResult, ResearchError> {
    let data = value.get("data").unwrap_or(&value);
    let web_results = data.get("web").or_else(|| value.get("web")).unwrap_or(data);
    let results = web_results
        .as_array()
        .ok_or_else(|| {
            ResearchError::new("invalid_response", "Firecrawl 响应缺少 data.web 搜索结果")
        })?
        .iter()
        .take(10)
        .filter_map(|item| {
            let url = item.get("url")?.as_str()?.to_string();
            Some(ResearchDocument {
                title: item
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("未命名网页")
                    .to_string(),
                url,
                published_date: item
                    .get("publishedDate")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                author: item
                    .get("description")
                    .and_then(Value::as_str)
                    .map(|value| super::http::truncate_chars(value, 500).0),
                text: String::new(),
                highlights: Vec::new(),
            })
        })
        .collect::<Vec<_>>();
    if results.is_empty() {
        return Err(ResearchError::new(
            "empty_results",
            "Firecrawl 没有返回可用搜索结果",
        ));
    }
    Ok(ResearchSearchResult {
        provider: "firecrawl".into(),
        results,
        cost_dollars: None,
    })
}
