use serde::Serialize;

use super::pi_protocol::PiCredential;
use crate::settings::Settings;

const PI_CREDENTIAL_SERVICE: &str = "CaseBoard/pi";
const MAX_CREDENTIAL_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PiCredentialSource {
    SystemVault,
    LegacySettings,
}

impl PiCredentialSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SystemVault => "system_vault",
            Self::LegacySettings => "legacy_settings",
        }
    }
}

#[derive(Clone)]
pub struct ResolvedPiCredential {
    pub credential: PiCredential,
    pub source: PiCredentialSource,
}

impl std::fmt::Debug for ResolvedPiCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedPiCredential")
            .field("credential_type", &self.credential_type())
            .field("source", &self.source)
            .finish()
    }
}

impl ResolvedPiCredential {
    pub fn credential_type(&self) -> &'static str {
        self.credential.credential_type()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PiCredentialStatus {
    pub provider_id: String,
    pub configured: bool,
    pub credential_type: Option<String>,
    pub expires_at_ms: Option<u64>,
}

pub trait PiCredentialVault: Send + Sync {
    fn read(&self, provider_id: &str) -> Result<Option<PiCredential>, String>;
    fn write(&self, provider_id: &str, credential: &PiCredential) -> Result<(), String>;
    fn delete(&self, provider_id: &str) -> Result<(), String>;

    fn status(&self, provider_id: &str) -> Result<PiCredentialStatus, String> {
        let credential = self.read(provider_id)?;
        Ok(PiCredentialStatus {
            provider_id: provider_id.to_string(),
            configured: credential.is_some(),
            credential_type: credential
                .as_ref()
                .map(|value| value.credential_type().into()),
            expires_at_ms: credential.as_ref().and_then(PiCredential::expires_at_ms),
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OsPiCredentialVault;

impl PiCredentialVault for OsPiCredentialVault {
    fn read(&self, provider_id: &str) -> Result<Option<PiCredential>, String> {
        validate_provider_id(provider_id)?;
        let entry = keyring::Entry::new(PI_CREDENTIAL_SERVICE, provider_id)
            .map_err(|error| format!("无法打开系统凭据库:{error}"))?;
        let encoded = match entry.get_password() {
            Ok(value) => value,
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(error) => return Err(format!("无法读取系统凭据库:{error}")),
        };
        if encoded.len() > MAX_CREDENTIAL_BYTES {
            return Err("系统凭据库中的 Pi credential 超过安全长度限制".into());
        }
        let credential: PiCredential = serde_json::from_str(&encoded)
            .map_err(|_| "系统凭据库中的 Pi credential 格式无效".to_string())?;
        validate_credential(&credential)?;
        Ok(Some(credential))
    }

    fn write(&self, provider_id: &str, credential: &PiCredential) -> Result<(), String> {
        validate_provider_id(provider_id)?;
        validate_credential(credential)?;
        let encoded = serde_json::to_string(credential)
            .map_err(|_| "Pi credential 序列化失败".to_string())?;
        if encoded.len() > MAX_CREDENTIAL_BYTES {
            return Err("Pi credential 超过安全长度限制".into());
        }
        keyring::Entry::new(PI_CREDENTIAL_SERVICE, provider_id)
            .map_err(|error| format!("无法打开系统凭据库:{error}"))?
            .set_password(&encoded)
            .map_err(|error| format!("无法写入系统凭据库:{error}"))
    }

    fn delete(&self, provider_id: &str) -> Result<(), String> {
        validate_provider_id(provider_id)?;
        let entry = keyring::Entry::new(PI_CREDENTIAL_SERVICE, provider_id)
            .map_err(|error| format!("无法打开系统凭据库:{error}"))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(format!("无法删除系统凭据:{error}")),
        }
    }
}

impl PiCredential {
    pub fn credential_type(&self) -> &'static str {
        match self {
            Self::ApiKey { .. } => "api_key",
            Self::OAuth { .. } => "oauth",
        }
    }

    pub fn expires_at_ms(&self) -> Option<u64> {
        match self {
            Self::ApiKey { .. } => None,
            Self::OAuth { expires, .. } => Some(*expires),
        }
    }
}

pub fn resolve_pi_credential(
    settings: &Settings,
    vault: &dyn PiCredentialVault,
    provider_id: &str,
) -> Result<Option<ResolvedPiCredential>, String> {
    if let Some(credential) = vault.read(provider_id)? {
        return Ok(Some(ResolvedPiCredential {
            credential,
            source: PiCredentialSource::SystemVault,
        }));
    }
    // 兼容旧设置中的同服务商 key，但只有用户已经点过“验证”且成功的 key 才能复用。
    // 否则“填了一个占位值”会绕过聊天启动门禁，到真正请求时才报 401/400。
    let (legacy_key, verified_at) = match provider_id {
        "deepseek" => (
            settings.cloud_llm_api_key.as_deref(),
            settings.deepseek_verified_at.as_deref(),
        ),
        "minimax-cn" => (
            settings.minimax_api_key.as_deref(),
            settings.minimax_verified_at.as_deref(),
        ),
        "zai" => (
            settings.glm_llm_api_key.as_deref(),
            settings.glm_llm_verified_at.as_deref(),
        ),
        "xiaomi" => (
            settings.mimo_llm_api_key.as_deref(),
            settings.mimo_llm_verified_at.as_deref(),
        ),
        "kimi-coding" => (
            settings.kimi_llm_api_key.as_deref(),
            settings.kimi_llm_verified_at.as_deref(),
        ),
        "caseboard-custom" => (
            settings.custom_llm_api_key.as_deref(),
            settings.custom_llm_verified_at.as_deref(),
        ),
        _ => (None, None),
    };
    let verified = verified_at.is_some_and(|value| !value.trim().is_empty());
    let legacy = legacy_key
        .map(str::trim)
        .filter(|key| !key.is_empty() && verified);

    Ok(legacy.map(|key| ResolvedPiCredential {
        credential: PiCredential::ApiKey {
            key: Some(key.to_string()),
            env: std::collections::BTreeMap::new(),
        },
        source: PiCredentialSource::LegacySettings,
    }))
}

fn validate_provider_id(provider_id: &str) -> Result<(), String> {
    if provider_id.is_empty()
        || provider_id.len() > 100
        || !provider_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("provider_id 格式无效".into());
    }
    Ok(())
}

fn validate_credential(credential: &PiCredential) -> Result<(), String> {
    match credential {
        PiCredential::ApiKey { key, env } => {
            if key.as_deref().is_none_or(|value| value.trim().is_empty()) && env.is_empty() {
                return Err("API Key credential 不能为空".into());
            }
            if env
                .iter()
                .any(|(name, value)| name.trim().is_empty() || value.trim().is_empty())
            {
                return Err("API Key credential 的环境配置无效".into());
            }
        }
        PiCredential::OAuth {
            access,
            refresh,
            expires,
            ..
        } => {
            if access.trim().is_empty() || refresh.trim().is_empty() || *expires == 0 {
                return Err("OAuth credential 字段不完整".into());
            }
        }
    }
    Ok(())
}
