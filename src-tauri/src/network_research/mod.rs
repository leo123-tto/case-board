pub mod credentials;
pub mod exa;
pub mod firecrawl;
pub mod http;

use async_trait::async_trait;
use chrono::Utc;
use serde::Serialize;

pub use credentials::{ResearchCredentialStatus, ResearchProvider};
pub use firecrawl::FirecrawlCreditUsage;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResearchDocument {
    pub title: String,
    pub url: String,
    pub published_date: Option<String>,
    pub author: Option<String>,
    pub text: String,
    pub highlights: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResearchSearchResult {
    pub provider: String,
    pub results: Vec<ResearchDocument>,
    pub cost_dollars: Option<f64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResearchVerificationResult {
    pub provider: ResearchProvider,
    pub ok: bool,
    pub verified_at: String,
    pub credit_usage: Option<FirecrawlCreditUsage>,
    pub message: String,
}

#[async_trait]
pub(crate) trait ResearchVerifier: Send + Sync {
    async fn verify_exa(&self, key: &str) -> Result<(), http::ResearchError>;
    async fn verify_firecrawl(
        &self,
        key: &str,
    ) -> Result<FirecrawlCreditUsage, http::ResearchError>;
}

struct LiveResearchVerifier;

#[async_trait]
impl ResearchVerifier for LiveResearchVerifier {
    async fn verify_exa(&self, key: &str) -> Result<(), http::ResearchError> {
        exa::ExaClient::new()?
            .search(key, "中华人民共和国最高人民法院", &[], 1, "instant")
            .await?;
        Ok(())
    }

    async fn verify_firecrawl(
        &self,
        key: &str,
    ) -> Result<FirecrawlCreditUsage, http::ResearchError> {
        firecrawl::FirecrawlClient::new()?.credit_usage(key).await
    }
}

pub(crate) async fn verify_with(
    vault: &dyn credentials::ResearchCredentialVault,
    verifier: &dyn ResearchVerifier,
    provider: ResearchProvider,
) -> Result<ResearchVerificationResult, String> {
    let key = credentials::read_key(vault, provider)?
        .ok_or_else(|| format!("{} API Key 尚未配置", provider.as_str()))?;
    let verification = match provider {
        ResearchProvider::Exa => verifier.verify_exa(&key).await.map(|()| None),
        ResearchProvider::Firecrawl => verifier.verify_firecrawl(&key).await.map(Some),
    };
    match verification {
        Ok(credit_usage) => {
            let verified_at = Utc::now().to_rfc3339();
            credentials::mark_verified(vault, provider, &verified_at, None)?;
            Ok(ResearchVerificationResult {
                provider,
                ok: true,
                verified_at,
                credit_usage,
                message: match provider {
                    ResearchProvider::Exa => {
                        "Exa API Key 验证成功；本次验证会产生极少量搜索用量".into()
                    }
                    ResearchProvider::Firecrawl => "Firecrawl API Key 与额度接口验证成功".into(),
                },
            })
        }
        Err(error) => {
            let _ = credentials::mark_failed(vault, provider, error.kind());
            Err(error.to_string())
        }
    }
}

pub fn get_statuses() -> Result<Vec<ResearchCredentialStatus>, String> {
    let vault = credentials::OsResearchCredentialVault;
    [ResearchProvider::Exa, ResearchProvider::Firecrawl]
        .into_iter()
        .map(|provider| credentials::status(&vault, provider))
        .collect()
}

pub fn save_provider_key(provider: ResearchProvider, key: &str) -> Result<(), String> {
    credentials::save_key(&credentials::OsResearchCredentialVault, provider, key)
}

pub async fn verify_provider(
    provider: ResearchProvider,
) -> Result<ResearchVerificationResult, String> {
    verify_with(
        &credentials::OsResearchCredentialVault,
        &LiveResearchVerifier,
        provider,
    )
    .await
}

pub fn remove_provider_key(provider: ResearchProvider) -> Result<(), String> {
    credentials::remove_key(&credentials::OsResearchCredentialVault, provider)
}

pub(crate) fn configured_providers() -> Result<(bool, bool), String> {
    let vault = credentials::OsResearchCredentialVault;
    Ok((
        credentials::status(&vault, ResearchProvider::Exa)?.configured,
        credentials::status(&vault, ResearchProvider::Firecrawl)?.configured,
    ))
}
