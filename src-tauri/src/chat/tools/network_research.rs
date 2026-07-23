use async_trait::async_trait;
use serde_json::{json, Value};

use crate::network_research::credentials::{read_key, OsResearchCredentialVault, ResearchProvider};
use crate::network_research::exa::ExaClient;
use crate::network_research::firecrawl::FirecrawlClient;

use super::{opt_str, opt_u32, require_str, Tool, ToolContext, ToolError, ToolResult};

fn provider_key(provider: ResearchProvider) -> Result<String, ToolError> {
    read_key(&OsResearchCredentialVault, provider)
        .map_err(ToolError::Runtime)?
        .ok_or_else(|| {
            ToolError::Runtime(format!(
                "{} API Key 未配置；请到设置 → Pi Runtime → 网络研究中配置并验证",
                provider.as_str()
            ))
        })
}

fn tool_result<T: serde::Serialize>(value: &T) -> Result<ToolResult, ToolError> {
    serde_json::to_string_pretty(value)
        .map(ToolResult::plain)
        .map_err(|_| ToolError::Runtime("网络研究结果序列化失败".into()))
}

fn map_error(error: crate::network_research::http::ResearchError) -> ToolError {
    ToolError::Runtime(error.to_string())
}

pub struct ExaSearch;

#[async_trait]
impl Tool for ExaSearch {
    fn name(&self) -> &str {
        "exa_search"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/exa_search.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{
            "query":{"type":"string","description":"已匿名化的公开网络检索词"},
            "search_type":{"type":"string","enum":["auto","instant","deep"]},
            "max_results":{"type":"integer","minimum":1,"maximum":10},
            "include_domains":{"type":"array","maxItems":10,"items":{"type":"string"}}
        },"required":["query"]})
    }
    async fn execute(&self, args: &Value, _ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let domains = args
            .get("include_domains")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .take(10)
            .map(str::to_string)
            .collect::<Vec<_>>();
        let result = ExaClient::new()
            .map_err(map_error)?
            .search(
                &provider_key(ResearchProvider::Exa)?,
                require_str(args, "query")?,
                &domains,
                opt_u32(args, "max_results").unwrap_or(5) as usize,
                opt_str(args, "search_type").unwrap_or("auto"),
            )
            .await
            .map_err(map_error)?;
        tool_result(&result)
    }
}

pub struct ExaContents;

#[async_trait]
impl Tool for ExaContents {
    fn name(&self) -> &str {
        "exa_contents"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/exa_contents.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"url":{"type":"string"}},"required":["url"]})
    }
    async fn execute(&self, args: &Value, _ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let result = ExaClient::new()
            .map_err(map_error)?
            .contents(
                &provider_key(ResearchProvider::Exa)?,
                &[require_str(args, "url")?.to_string()],
            )
            .await
            .map_err(map_error)?;
        tool_result(&result)
    }
}

pub struct ExaFindSimilar;

#[async_trait]
impl Tool for ExaFindSimilar {
    fn name(&self) -> &str {
        "exa_find_similar"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/exa_find_similar.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"url":{"type":"string"},"max_results":{"type":"integer","minimum":1,"maximum":10}},"required":["url"]})
    }
    async fn execute(&self, args: &Value, _ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let result = ExaClient::new()
            .map_err(map_error)?
            .find_similar(
                &provider_key(ResearchProvider::Exa)?,
                require_str(args, "url")?,
                opt_u32(args, "max_results").unwrap_or(5) as usize,
            )
            .await
            .map_err(map_error)?;
        tool_result(&result)
    }
}

pub struct FirecrawlSearch;

#[async_trait]
impl Tool for FirecrawlSearch {
    fn name(&self) -> &str {
        "firecrawl_search"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/firecrawl_search.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"query":{"type":"string","description":"已匿名化的公开网络检索词"},"max_results":{"type":"integer","minimum":1,"maximum":10}},"required":["query"]})
    }
    async fn execute(&self, args: &Value, _ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let result = FirecrawlClient::new()
            .map_err(map_error)?
            .search(
                &provider_key(ResearchProvider::Firecrawl)?,
                require_str(args, "query")?,
                opt_u32(args, "max_results").unwrap_or(5) as usize,
            )
            .await
            .map_err(map_error)?;
        tool_result(&result)
    }
}

pub struct FirecrawlScrape;

pub(crate) fn parse_proxy(value: &str) -> Result<&str, ToolError> {
    match value {
        "auto" | "basic" | "enhanced" => Ok(value),
        _ => Err(ToolError::InvalidArgs(
            "proxy 只支持 auto、basic、enhanced".into(),
        )),
    }
}

fn effective_firecrawl_proxy(url: &str, requested: &str) -> Result<String, ToolError> {
    let requested = parse_proxy(requested)?;
    let is_wechat_article = reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .is_some_and(|host| host.eq_ignore_ascii_case("mp.weixin.qq.com"));
    Ok(if is_wechat_article {
        "enhanced".to_string()
    } else {
        requested.to_string()
    })
}

#[async_trait]
impl Tool for FirecrawlScrape {
    fn name(&self) -> &str {
        "firecrawl_scrape"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/firecrawl_scrape.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"url":{"type":"string"},"proxy":{"type":"string","enum":["auto","basic","enhanced"]}},"required":["url"]})
    }
    async fn execute(&self, args: &Value, _ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let url = require_str(args, "url")?;
        let proxy = effective_firecrawl_proxy(url, opt_str(args, "proxy").unwrap_or("auto"))?;
        let result = FirecrawlClient::new()
            .map_err(map_error)?
            .scrape(&provider_key(ResearchProvider::Firecrawl)?, url, &proxy)
            .await
            .map_err(map_error)?;
        tool_result(&result)
    }
}
