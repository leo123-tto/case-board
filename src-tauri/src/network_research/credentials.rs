use serde::{Deserialize, Serialize};

const RESEARCH_CREDENTIAL_SERVICE: &str = "CaseBoard/network-research";
const MAX_KEY_BYTES: usize = 16 * 1024;
const MAX_ENVELOPE_BYTES: usize = 24 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResearchProvider {
    Exa,
    Firecrawl,
}

impl ResearchProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exa => "exa",
            Self::Firecrawl => "firecrawl",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StoredResearchCredential {
    version: u8,
    key: String,
    verified_at: Option<String>,
    last_error_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ResearchCredentialStatus {
    pub provider: ResearchProvider,
    pub configured: bool,
    pub verified_at: Option<String>,
    pub last_error_kind: Option<String>,
}

pub(crate) trait ResearchCredentialVault: Send + Sync {
    fn read(&self, provider: ResearchProvider) -> Result<Option<StoredResearchCredential>, String>;
    fn write(
        &self,
        provider: ResearchProvider,
        credential: &StoredResearchCredential,
    ) -> Result<(), String>;
    fn delete(&self, provider: ResearchProvider) -> Result<(), String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsResearchCredentialVault;

impl ResearchCredentialVault for OsResearchCredentialVault {
    fn read(&self, provider: ResearchProvider) -> Result<Option<StoredResearchCredential>, String> {
        let entry = keyring::Entry::new(RESEARCH_CREDENTIAL_SERVICE, provider.as_str())
            .map_err(|error| format!("无法打开系统凭据库:{error}"))?;
        let encoded = match entry.get_password() {
            Ok(value) => value,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(error) => return Err(format!("无法读取系统凭据库:{error}")),
        };
        if encoded.len() > MAX_ENVELOPE_BYTES {
            return Err("系统凭据库中的网络研究凭据超过安全长度限制".into());
        }
        let credential: StoredResearchCredential = serde_json::from_str(&encoded)
            .map_err(|_| "系统凭据库中的网络研究凭据格式无效".to_string())?;
        validate_stored(&credential)?;
        Ok(Some(credential))
    }

    fn write(
        &self,
        provider: ResearchProvider,
        credential: &StoredResearchCredential,
    ) -> Result<(), String> {
        validate_stored(credential)?;
        let encoded =
            serde_json::to_string(credential).map_err(|_| "网络研究凭据序列化失败".to_string())?;
        if encoded.len() > MAX_ENVELOPE_BYTES {
            return Err("网络研究凭据超过安全长度限制".into());
        }
        keyring::Entry::new(RESEARCH_CREDENTIAL_SERVICE, provider.as_str())
            .map_err(|error| format!("无法打开系统凭据库:{error}"))?
            .set_password(&encoded)
            .map_err(|error| format!("无法写入系统凭据库:{error}"))
    }

    fn delete(&self, provider: ResearchProvider) -> Result<(), String> {
        let entry = keyring::Entry::new(RESEARCH_CREDENTIAL_SERVICE, provider.as_str())
            .map_err(|error| format!("无法打开系统凭据库:{error}"))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("无法删除系统凭据:{error}")),
        }
    }
}

pub(crate) fn save_key(
    vault: &dyn ResearchCredentialVault,
    provider: ResearchProvider,
    key: &str,
) -> Result<(), String> {
    let key = key.trim();
    validate_key(key)?;
    vault.write(
        provider,
        &StoredResearchCredential {
            version: 1,
            key: key.to_string(),
            verified_at: None,
            last_error_kind: None,
        },
    )
}

pub(crate) fn read_key(
    vault: &dyn ResearchCredentialVault,
    provider: ResearchProvider,
) -> Result<Option<String>, String> {
    Ok(vault.read(provider)?.map(|value| value.key))
}

pub(crate) fn status(
    vault: &dyn ResearchCredentialVault,
    provider: ResearchProvider,
) -> Result<ResearchCredentialStatus, String> {
    let stored = vault.read(provider)?;
    Ok(ResearchCredentialStatus {
        provider,
        configured: stored.is_some(),
        verified_at: stored.as_ref().and_then(|value| value.verified_at.clone()),
        last_error_kind: stored.and_then(|value| value.last_error_kind),
    })
}

pub(crate) fn mark_verified(
    vault: &dyn ResearchCredentialVault,
    provider: ResearchProvider,
    verified_at: &str,
    last_error_kind: Option<&str>,
) -> Result<(), String> {
    let mut stored = vault
        .read(provider)?
        .ok_or_else(|| format!("{} API Key 尚未配置", provider.as_str()))?;
    stored.verified_at = Some(verified_at.to_string());
    stored.last_error_kind = last_error_kind.map(str::to_string);
    vault.write(provider, &stored)
}

pub(crate) fn mark_failed(
    vault: &dyn ResearchCredentialVault,
    provider: ResearchProvider,
    error_kind: &str,
) -> Result<(), String> {
    let Some(mut stored) = vault.read(provider)? else {
        return Ok(());
    };
    stored.verified_at = None;
    stored.last_error_kind = Some(error_kind.to_string());
    vault.write(provider, &stored)
}

pub(crate) fn remove_key(
    vault: &dyn ResearchCredentialVault,
    provider: ResearchProvider,
) -> Result<(), String> {
    vault.delete(provider)
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("API Key 不能为空".into());
    }
    if key.len() > MAX_KEY_BYTES {
        return Err("API Key 超过安全长度限制".into());
    }
    if key.chars().any(char::is_control) {
        return Err("API Key 包含无效控制字符".into());
    }
    Ok(())
}

fn validate_stored(credential: &StoredResearchCredential) -> Result<(), String> {
    if credential.version != 1 {
        return Err("网络研究凭据版本不受支持".into());
    }
    validate_key(&credential.key)
}
