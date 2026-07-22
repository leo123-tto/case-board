use reqwest::Method;
use serde_json::{json, Value};

use super::http::{request_json, research_client, validate_research_query, ResearchError};
use super::{ResearchDocument, ResearchSearchResult};

const EXA_BASE: &str = "https://api.exa.ai";

pub struct ExaClient {
    base_url: String,
    client: reqwest::Client,
}

impl ExaClient {
    pub fn new() -> Result<Self, ResearchError> {
        Self::with_base_url(EXA_BASE)
    }

    pub fn with_base_url(base_url: impl Into<String>) -> Result<Self, ResearchError> {
        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: research_client()?,
        })
    }

    pub async fn search(
        &self,
        key: &str,
        query: &str,
        include_domains: &[String],
        num_results: usize,
        search_type: &str,
    ) -> Result<ResearchSearchResult, ResearchError> {
        let query = validate_research_query(query)?;
        let search_type = match search_type {
            "auto" | "instant" | "deep" => search_type,
            _ => return Err(ResearchError::new("invalid_args", "Exa 搜索模式无效")),
        };
        let body = json!({
            "query": query,
            "type": search_type,
            "numResults": num_results.clamp(1, 10),
            "includeDomains": include_domains.iter().take(10).collect::<Vec<_>>(),
            "contents": {"highlights": {"maxCharacters": 2000}}
        });
        let value = request_json(
            &self.client,
            Method::POST,
            &format!("{}/search", self.base_url),
            "Exa",
            key,
            Some(&body),
        )
        .await?;
        parse_search_response(value)
    }

    pub async fn contents(
        &self,
        key: &str,
        urls: &[String],
    ) -> Result<ResearchSearchResult, ResearchError> {
        if urls.is_empty() || urls.len() > 5 {
            return Err(ResearchError::new(
                "invalid_args",
                "Exa contents 每次需要 1 至 5 个 URL",
            ));
        }
        for url in urls {
            crate::chat::tools::web::validate_public_http_url_for_request(url)
                .await
                .map_err(|error| ResearchError::new("invalid_url", error.to_string()))?;
        }
        let body = json!({"urls": urls, "text": {"maxCharacters": 60000}});
        let value = request_json(
            &self.client,
            Method::POST,
            &format!("{}/contents", self.base_url),
            "Exa",
            key,
            Some(&body),
        )
        .await?;
        parse_search_response(value)
    }

    pub async fn find_similar(
        &self,
        key: &str,
        url: &str,
        num_results: usize,
    ) -> Result<ResearchSearchResult, ResearchError> {
        crate::chat::tools::web::validate_public_http_url_for_request(url)
            .await
            .map_err(|error| ResearchError::new("invalid_url", error.to_string()))?;
        let body = json!({
            "url": url,
            "numResults": num_results.clamp(1, 10),
            "contents": {"highlights": {"maxCharacters": 2000}}
        });
        let value = request_json(
            &self.client,
            Method::POST,
            &format!("{}/findSimilar", self.base_url),
            "Exa",
            key,
            Some(&body),
        )
        .await?;
        parse_search_response(value)
    }
}

pub fn parse_search_response(value: Value) -> Result<ResearchSearchResult, ResearchError> {
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| ResearchError::new("invalid_response", "Exa 响应缺少 results"))?
        .iter()
        .take(10)
        .filter_map(parse_document)
        .collect::<Vec<_>>();
    if results.is_empty() {
        return Err(ResearchError::new(
            "empty_results",
            "Exa 没有返回可用结果，请改写关键词或换用其他搜索工具",
        ));
    }
    let cost_dollars = value
        .get("costDollars")
        .and_then(|cost| cost.get("total").or(Some(cost)))
        .and_then(Value::as_f64);
    Ok(ResearchSearchResult {
        provider: "exa".into(),
        results,
        cost_dollars,
    })
}

fn parse_document(value: &Value) -> Option<ResearchDocument> {
    let url = value.get("url")?.as_str()?.trim();
    if url.is_empty() {
        return None;
    }
    let text = value
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (text, _) = super::http::truncate_chars(text, 60_000);
    Some(ResearchDocument {
        title: value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("未命名网页")
            .to_string(),
        url: url.to_string(),
        published_date: value
            .get("publishedDate")
            .and_then(Value::as_str)
            .map(str::to_string),
        author: value
            .get("author")
            .and_then(Value::as_str)
            .map(str::to_string),
        text,
        highlights: value
            .get("highlights")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(10)
            .map(|text| super::http::truncate_chars(text, 2_000).0)
            .collect(),
    })
}
