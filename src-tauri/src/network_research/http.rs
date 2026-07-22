use std::sync::OnceLock;
use std::time::Duration;

use futures::StreamExt;
use regex::Regex;
use reqwest::{Method, StatusCode};
use serde_json::Value;
use thiserror::Error;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Error)]
#[error("{message}")]
pub struct ResearchError {
    kind: &'static str,
    message: String,
}

impl ResearchError {
    pub fn new(kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> &'static str {
        self.kind
    }
}

pub fn validate_research_query(query: &str) -> Result<&str, ResearchError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(ResearchError::new("invalid_query", "检索词不能为空"));
    }
    if query.chars().count() > 500 {
        return Err(ResearchError::new(
            "invalid_query",
            "检索词超过 500 字符限制，请先匿名化并缩短",
        ));
    }
    static SENSITIVE: OnceLock<Regex> = OnceLock::new();
    let sensitive = SENSITIVE.get_or_init(|| {
        Regex::new(
            r"(?ix)((?:^|[^\d])\d{17}[\dX](?:[^\dX]|$)|(?:^|[^\d])1[3-9]\d{9}(?:[^\d]|$)|(?:^|\s)(?:/[U]sers/|/[h]ome/|[A-Z]:\\)|\b(?:sk-|api[_-]?key|token|bearer)[A-Za-z0-9_.-]{6,})",
        )
        .expect("sensitive research query regex")
    });
    if sensitive.is_match(query) {
        return Err(ResearchError::new(
            "sensitive_query",
            "检索词疑似包含身份信息、文件路径或凭据，请先匿名化",
        ));
    }
    Ok(query)
}

pub fn research_client() -> Result<reqwest::Client, ResearchError> {
    reqwest::Client::builder()
        .user_agent(concat!(
            "CaseBoard/",
            env!("CARGO_PKG_VERSION"),
            " network-research"
        ))
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ResearchError::new("client", "初始化网络研究客户端失败"))
}

pub async fn request_json(
    client: &reqwest::Client,
    method: Method,
    url: &str,
    provider: &'static str,
    key: &str,
    body: Option<&Value>,
) -> Result<Value, ResearchError> {
    let mut request = client.request(method, url);
    request = if provider == "Exa" {
        request.header("x-api-key", key)
    } else {
        request.bearer_auth(key)
    };
    if let Some(body) = body {
        request = request.json(body);
    }
    let response = request.send().await.map_err(|_| {
        ResearchError::new("network", format!("{provider} 网络请求失败，请稍后重试"))
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(provider_status_error(provider, status.as_u16(), "", key));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|_| ResearchError::new("response", format!("读取 {provider} 响应失败")))?;
        if bytes.len() + chunk.len() > MAX_RESPONSE_BYTES {
            return Err(ResearchError::new(
                "response_too_large",
                format!("{provider} 响应超过 2 MiB 安全限制"),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| ResearchError::new("invalid_response", format!("{provider} 响应格式无效")))
}

pub fn provider_status_error(
    provider: &'static str,
    status: u16,
    _upstream_body: &str,
    _secret: &str,
) -> ResearchError {
    match StatusCode::from_u16(status).ok() {
        Some(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) => ResearchError::new(
            "auth",
            format!("{provider} 凭据无效或无权访问，请到设置中重新验证"),
        ),
        Some(StatusCode::PAYMENT_REQUIRED) => ResearchError::new(
            "credits",
            format!("{provider} 额度不足，请充值或改用其他检索工具"),
        ),
        Some(StatusCode::TOO_MANY_REQUESTS) => ResearchError::new(
            "rate_limited",
            format!("{provider} 请求过于频繁，请稍后重试或改用其他工具"),
        ),
        Some(code) if code.is_server_error() => ResearchError::new(
            "upstream",
            format!("{provider} 服务暂时不可用（HTTP {status}）"),
        ),
        _ => ResearchError::new("http", format!("{provider} 请求失败（HTTP {status}）")),
    }
}

pub fn truncate_chars(value: &str, limit: usize) -> (String, bool) {
    if value.chars().count() <= limit {
        return (value.to_string(), false);
    }
    (value.chars().take(limit).collect(), true)
}
