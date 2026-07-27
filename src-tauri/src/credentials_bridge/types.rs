use std::fmt;
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const BRIDGE_SCHEMA: &str = "caseboard-credential-bridge/v1";
pub const ENVELOPE_VERSION: u16 = 1;

#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("凭据桥路径包含符号链接: {path}")]
    SymlinkNotAllowed { path: PathBuf },
    #[error("凭据桥要求普通文件: {path}")]
    NonRegularFile { path: PathBuf },
    #[error("凭据桥要求目录: {path}")]
    NotDirectory { path: PathBuf },
    #[error("凭据桥权限不安全: {path}（expected {expected:o}, actual {actual:o}）")]
    UnsafePermissions {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
    #[error("凭据桥 owner 不匹配: {path}（expected uid {expected}, actual uid {actual}）")]
    OwnerMismatch {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
    #[error("凭据桥 Windows ACL 不是 current-user-only: {path}（{reason}）")]
    WindowsAclNotCurrentUserOnly { path: PathBuf, reason: String },
    #[error("凭据桥 I/O 失败（{operation}, {path}）: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("凭据桥数据库失败: {0}")]
    Database(#[from] sqlx::Error),
    #[error("凭据桥 manifest 无效: {0}")]
    InvalidManifest(String),
    #[error("凭据迁移正在运行: {path}")]
    MigrationLocked { path: PathBuf },
    #[error("MCP instance_id 无效: {0}")]
    InvalidMcpInstanceId(String),
    #[error("凭据桥 master key 无效")]
    InvalidMasterKey,
    #[error("凭据字段无效: {0}")]
    InvalidInput(&'static str),
    #[error("凭据 metadata 损坏: {0}")]
    CorruptMetadata(String),
    #[error("凭据不存在: {handle}")]
    CredentialNotFound { handle: CredentialHandle },
    #[error("凭据 revision 已过期（requested {requested}, current {current}）")]
    StaleRevision { requested: i64, current: i64 },
    #[error("凭据认证解密失败（handle {handle}, revision {revision}）")]
    AuthenticationFailed {
        handle: CredentialHandle,
        revision: i64,
    },
    #[error("凭据加密失败")]
    EncryptionFailed,
    #[error("SecretLease 已过期")]
    LeaseExpired,
    #[error("SecretLease 已消费")]
    LeaseConsumed,
    #[error("SecretLease binding 不匹配: {field}")]
    LeaseBindingMismatch { field: &'static str },
    #[error("凭据消费方 {consumer} 不允许访问 {provider_or_connector_id}")]
    ConsumerNotAllowed {
        consumer: &'static str,
        provider_or_connector_id: String,
    },
    #[error("pending 凭据映射不匹配: {field}")]
    PendingCredentialMismatch { field: &'static str },
    #[error("credential_missing: {stable_inventory_id}")]
    PendingCredentialMissing { stable_inventory_id: String },
    #[error("credential_unreadable: {stable_inventory_id}")]
    PendingCredentialUnreadable { stable_inventory_id: String },
    #[error("凭据迁移模拟崩溃点: {point}")]
    SimulatedMigrationCrash { point: &'static str },
}

pub type BridgeResult<T> = Result<T, BridgeError>;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialHandle(String);

impl CredentialHandle {
    pub fn new(value: impl Into<String>) -> BridgeResult<Self> {
        let value = value.into();
        validate_identifier(&value, "handle")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CredentialHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    ApiKey,
    PiCredentialBundle,
    OAuthAccessToken,
    OAuthRefreshToken,
    Password,
    WebhookSecret,
    AppToken,
    AppSecret,
    VerificationToken,
    EncryptionKey,
    McpSecret,
    SyncKey,
    SessionCookie,
}

impl CredentialKind {
    pub(crate) fn as_storage_str(&self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::PiCredentialBundle => "pi_credential_bundle",
            Self::OAuthAccessToken => "oauth_access_token",
            Self::OAuthRefreshToken => "oauth_refresh_token",
            Self::Password => "password",
            Self::WebhookSecret => "webhook_secret",
            Self::AppToken => "app_token",
            Self::AppSecret => "app_secret",
            Self::VerificationToken => "verification_token",
            Self::EncryptionKey => "encryption_key",
            Self::McpSecret => "mcp_secret",
            Self::SyncKey => "sync_key",
            Self::SessionCookie => "session_cookie",
        }
    }

    pub(crate) fn from_storage_str(value: &str) -> BridgeResult<Self> {
        match value {
            "api_key" => Ok(Self::ApiKey),
            "pi_credential_bundle" => Ok(Self::PiCredentialBundle),
            "oauth_access_token" => Ok(Self::OAuthAccessToken),
            "oauth_refresh_token" => Ok(Self::OAuthRefreshToken),
            "password" => Ok(Self::Password),
            "webhook_secret" => Ok(Self::WebhookSecret),
            "app_token" => Ok(Self::AppToken),
            "app_secret" => Ok(Self::AppSecret),
            "verification_token" => Ok(Self::VerificationToken),
            "encryption_key" => Ok(Self::EncryptionKey),
            "mcp_secret" => Ok(Self::McpSecret),
            "sync_key" => Ok(Self::SyncKey),
            "session_cookie" => Ok(Self::SessionCookie),
            other => Err(BridgeError::CorruptMetadata(format!(
                "unknown credential kind: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", content = "id", rename_all = "snake_case")]
pub enum CredentialOwnerScope {
    Global,
    Provider(String),
    Connector(String),
    Workspace(String),
    Case(String),
    DeviceGroup(String),
    Team(String),
}

impl CredentialOwnerScope {
    fn validate(&self) -> BridgeResult<()> {
        match self {
            Self::Global => Ok(()),
            Self::Provider(id)
            | Self::Connector(id)
            | Self::Workspace(id)
            | Self::Case(id)
            | Self::DeviceGroup(id)
            | Self::Team(id) => validate_identifier(id, "owner_scope"),
        }
    }

    pub(crate) fn to_storage_string(&self) -> String {
        match self {
            Self::Global => "global".to_owned(),
            Self::Provider(id) => format!("provider:{id}"),
            Self::Connector(id) => format!("connector:{id}"),
            Self::Workspace(id) => format!("workspace:{id}"),
            Self::Case(id) => format!("case:{id}"),
            Self::DeviceGroup(id) => format!("device_group:{id}"),
            Self::Team(id) => format!("team:{id}"),
        }
    }

    pub(crate) fn from_storage_str(value: &str) -> BridgeResult<Self> {
        if value == "global" {
            return Ok(Self::Global);
        }
        let (kind, id) = value
            .split_once(':')
            .ok_or_else(|| BridgeError::CorruptMetadata("invalid owner scope".to_owned()))?;
        let scope = match kind {
            "provider" => Self::Provider(id.to_owned()),
            "connector" => Self::Connector(id.to_owned()),
            "workspace" => Self::Workspace(id.to_owned()),
            "case" => Self::Case(id.to_owned()),
            "device_group" => Self::DeviceGroup(id.to_owned()),
            "team" => Self::Team(id.to_owned()),
            _ => Err(BridgeError::CorruptMetadata(
                "unknown owner scope".to_owned(),
            ))?,
        };
        scope
            .validate()
            .map_err(|_| BridgeError::CorruptMetadata("invalid owner scope id".to_owned()))?;
        Ok(scope)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CredentialConsumer(String);

impl CredentialConsumer {
    pub fn new(value: impl Into<String>) -> BridgeResult<Self> {
        let value = value.into();
        validate_identifier(&value, "consumer")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BridgeCredentialConsumer {
    LlmProvider,
    OcrProvider,
    YuandianConnector,
    EmbeddingProvider,
    CourierConnector,
    FeishuConnector,
    CourtFiling,
    McpTransport,
    PiProviderAuth,
    PiProviderRun,
    NetworkResearch,
    TeamTransport,
    DeviceSyncTransport,
}

impl BridgeCredentialConsumer {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LlmProvider => "llm_provider",
            Self::OcrProvider => "ocr_provider",
            Self::YuandianConnector => "yuandian_connector",
            Self::EmbeddingProvider => "embedding_provider",
            Self::CourierConnector => "courier_connector",
            Self::FeishuConnector => "feishu_connector",
            Self::CourtFiling => "court_filing",
            Self::McpTransport => "mcp_transport",
            Self::PiProviderAuth => "pi_provider_auth",
            Self::PiProviderRun => "pi_provider_run",
            Self::NetworkResearch => "network_research",
            Self::TeamTransport => "team_transport",
            Self::DeviceSyncTransport => "device_sync_transport",
        }
    }

    pub(crate) fn permits(self, stable_inventory_id: &str, provider_or_connector_id: &str) -> bool {
        match self {
            Self::LlmProvider => {
                stable_inventory_id.starts_with("settings:")
                    && provider_or_connector_id.starts_with("llm.")
            }
            Self::OcrProvider => {
                stable_inventory_id.starts_with("settings:")
                    && provider_or_connector_id.starts_with("ocr.")
            }
            Self::YuandianConnector => {
                stable_inventory_id == "settings:yuandian_api_key"
                    && provider_or_connector_id == "connector.yuandian"
            }
            Self::EmbeddingProvider => {
                stable_inventory_id == "settings:embedding_api_key"
                    && provider_or_connector_id == "embedding"
            }
            Self::CourierConnector => {
                stable_inventory_id == "settings:kuaidi100_key"
                    && provider_or_connector_id == "connector.kuaidi100"
            }
            Self::FeishuConnector => {
                matches!(
                    stable_inventory_id,
                    "settings:feishu_app_token" | "settings:feishu_webhook_url"
                ) && provider_or_connector_id.starts_with("feishu.")
            }
            Self::CourtFiling => {
                matches!(
                    stable_inventory_id,
                    "settings:court_filing_password" | "court_filing:cookie_store"
                ) && provider_or_connector_id == "court_filing"
            }
            Self::McpTransport => {
                stable_inventory_id.starts_with("settings:mcp:")
                    && matches!(
                        stable_inventory_id.rsplit(':').next(),
                        Some("env" | "headers")
                    )
                    && provider_or_connector_id.starts_with("mcp:")
            }
            Self::PiProviderAuth | Self::PiProviderRun => {
                stable_inventory_id.starts_with("pi:")
                    && provider_or_connector_id.starts_with("pi.")
            }
            Self::NetworkResearch => {
                stable_inventory_id.starts_with("network_research:")
                    && provider_or_connector_id.starts_with("research.")
            }
            Self::TeamTransport => {
                stable_inventory_id.starts_with("settings:team:")
                    && provider_or_connector_id == "team"
            }
            Self::DeviceSyncTransport => {
                stable_inventory_id.starts_with("settings:device_sync:")
                    && provider_or_connector_id == "device_sync"
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BridgeCredentialState {
    PendingMigration,
    Unverified,
    Valid,
    Expired,
    Revoked,
    Unreadable,
    LegacySystemPending,
    LegacySystemDeclined,
    LegacySystemFailed,
}

impl BridgeCredentialState {
    pub(crate) fn as_storage_str(self) -> &'static str {
        match self {
            Self::PendingMigration => "pending_migration",
            Self::Unverified => "unverified",
            Self::Valid => "valid",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::Unreadable => "unreadable",
            Self::LegacySystemPending => "legacy_system_pending",
            Self::LegacySystemDeclined => "legacy_system_declined",
            Self::LegacySystemFailed => "legacy_system_failed",
        }
    }

    pub(crate) fn from_storage_str(value: &str) -> BridgeResult<Self> {
        match value {
            "pending_migration" => Ok(Self::PendingMigration),
            "unverified" => Ok(Self::Unverified),
            "valid" => Ok(Self::Valid),
            "expired" => Ok(Self::Expired),
            "revoked" => Ok(Self::Revoked),
            "unreadable" => Ok(Self::Unreadable),
            "legacy_system_pending" => Ok(Self::LegacySystemPending),
            "legacy_system_declined" => Ok(Self::LegacySystemDeclined),
            "legacy_system_failed" => Ok(Self::LegacySystemFailed),
            other => Err(BridgeError::CorruptMetadata(format!(
                "unknown credential state: {other}"
            ))),
        }
    }
}

pub struct NewBridgeCredential {
    pub(crate) handle: CredentialHandle,
    pub(crate) provider_or_connector_id: String,
    pub(crate) kind: CredentialKind,
    pub(crate) owner_scope: CredentialOwnerScope,
    pub(crate) state: BridgeCredentialState,
    pub(crate) secret: Zeroizing<Vec<u8>>,
}

impl NewBridgeCredential {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        handle: impl Into<String>,
        provider_or_connector_id: impl Into<String>,
        kind: CredentialKind,
        owner_scope: CredentialOwnerScope,
        state: BridgeCredentialState,
        secret: Vec<u8>,
    ) -> BridgeResult<Self> {
        let provider_or_connector_id = provider_or_connector_id.into();
        validate_identifier(&provider_or_connector_id, "provider_or_connector_id")?;
        owner_scope.validate()?;
        if secret.is_empty() {
            return Err(BridgeError::InvalidInput("secret must not be empty"));
        }
        Ok(Self {
            handle: CredentialHandle::new(handle)?,
            provider_or_connector_id,
            kind,
            owner_scope,
            state,
            secret: Zeroizing::new(secret),
        })
    }
}

impl fmt::Debug for NewBridgeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NewBridgeCredential(<redacted>)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeCredentialMetadata {
    pub handle: CredentialHandle,
    pub provider_or_connector_id: String,
    pub kind: CredentialKind,
    pub owner_scope: CredentialOwnerScope,
    pub revision: i64,
    pub state: BridgeCredentialState,
    pub secret_present: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingMigrationEntry {
    pub stable_inventory_id: String,
    pub handle: CredentialHandle,
    pub provider_or_connector_id: String,
    pub revision: i64,
    pub authenticated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRefV1 {
    pub handle: CredentialHandle,
    pub revision: i64,
}

impl CredentialRefV1 {
    pub fn new(handle: impl Into<String>, revision: i64) -> BridgeResult<Self> {
        if revision < 1 {
            return Err(BridgeError::InvalidInput("revision must be positive"));
        }
        Ok(Self {
            handle: CredentialHandle::new(handle)?,
            revision,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingMigrationStatus {
    Succeeded,
    Partial,
    Failed,
    SkippedLocked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingMigrationSourceError {
    pub source: String,
    pub error: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingMigrationReport {
    pub status: PendingMigrationStatus,
    pub entries: Vec<PendingMigrationEntry>,
    pub sealed_count: usize,
    pub authenticated_count: usize,
    pub mcp_instance_ids_added: usize,
    pub source_errors: Vec<PendingMigrationSourceError>,
    #[serde(default)]
    pub sanitized: bool,
    #[serde(default)]
    pub activated_count: usize,
}

impl PendingMigrationReport {
    pub(crate) fn from_outcome(
        entries: Vec<PendingMigrationEntry>,
        sealed_count: usize,
        authenticated_count: usize,
        mcp_instance_ids_added: usize,
        source_errors: Vec<PendingMigrationSourceError>,
    ) -> Self {
        let status = if source_errors.is_empty() {
            PendingMigrationStatus::Succeeded
        } else if entries.is_empty() {
            PendingMigrationStatus::Failed
        } else {
            PendingMigrationStatus::Partial
        };
        Self {
            status,
            entries,
            sealed_count,
            authenticated_count,
            mcp_instance_ids_added,
            source_errors,
            sanitized: false,
            activated_count: 0,
        }
    }

    pub(crate) fn failed(source: impl Into<String>, error: impl Into<String>) -> Self {
        Self::from_outcome(
            Vec::new(),
            0,
            0,
            0,
            vec![PendingMigrationSourceError {
                source: source.into(),
                error: error.into(),
            }],
        )
    }

    pub(crate) fn skipped_locked(error: impl Into<String>) -> Self {
        Self {
            status: PendingMigrationStatus::SkippedLocked,
            entries: Vec::new(),
            sealed_count: 0,
            authenticated_count: 0,
            mcp_instance_ids_added: 0,
            source_errors: vec![PendingMigrationSourceError {
                source: "credential-bridge/pending-migration.lock".to_owned(),
                error: error.into(),
            }],
            sanitized: false,
            activated_count: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutomaticMigrationCrashPoint {
    AfterLegacySettingsRead,
    AfterFirstPendingEnvelope,
    AfterAllPendingEnvelopes,
    AfterSanitizedSettingsTempWrite,
    AfterSanitizedSettingsFsync,
    AfterSettingsRenameBeforeActivation,
    AfterActiveManifestUpdate,
}

impl AutomaticMigrationCrashPoint {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::AfterLegacySettingsRead => "after_legacy_settings_read",
            Self::AfterFirstPendingEnvelope => "after_first_pending_envelope",
            Self::AfterAllPendingEnvelopes => "after_all_pending_envelopes",
            Self::AfterSanitizedSettingsTempWrite => "after_sanitized_settings_temp_write",
            Self::AfterSanitizedSettingsFsync => "after_sanitized_settings_fsync",
            Self::AfterSettingsRenameBeforeActivation => "after_settings_rename_before_activation",
            Self::AfterActiveManifestUpdate => "after_active_manifest_update",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LeaseBinding {
    pub(crate) consumer: CredentialConsumer,
    pub(crate) provider_or_connector_id: String,
    pub(crate) handle: CredentialHandle,
    pub(crate) revision: i64,
}

impl LeaseBinding {
    pub fn new(
        consumer: CredentialConsumer,
        provider_or_connector_id: impl Into<String>,
        handle: impl Into<String>,
        revision: i64,
    ) -> BridgeResult<Self> {
        let provider_or_connector_id = provider_or_connector_id.into();
        validate_identifier(&provider_or_connector_id, "provider_or_connector_id")?;
        if revision < 1 {
            return Err(BridgeError::InvalidInput("revision must be positive"));
        }
        Ok(Self {
            consumer,
            provider_or_connector_id,
            handle: CredentialHandle::new(handle)?,
            revision,
        })
    }
}

pub struct SecretLeaseRequest {
    pub(crate) binding: LeaseBinding,
    pub(crate) expires_at: Instant,
}

impl SecretLeaseRequest {
    pub fn new(
        consumer: CredentialConsumer,
        provider_or_connector_id: impl Into<String>,
        handle: impl Into<String>,
        revision: i64,
        expires_at: Instant,
    ) -> BridgeResult<Self> {
        Ok(Self {
            binding: LeaseBinding::new(consumer, provider_or_connector_id, handle, revision)?,
            expires_at,
        })
    }
}

pub struct PendingSecretLeaseRequest {
    pub(crate) consumer: BridgeCredentialConsumer,
    pub(crate) stable_inventory_id: String,
    pub(crate) provider_or_connector_id: String,
    pub(crate) credential_ref: CredentialRefV1,
    pub(crate) expires_at: Instant,
}

impl PendingSecretLeaseRequest {
    pub fn new(
        consumer: BridgeCredentialConsumer,
        stable_inventory_id: impl Into<String>,
        provider_or_connector_id: impl Into<String>,
        credential_ref: CredentialRefV1,
        expires_at: Instant,
    ) -> BridgeResult<Self> {
        let stable_inventory_id = stable_inventory_id.into();
        if stable_inventory_id.is_empty()
            || stable_inventory_id.len() > 1024
            || stable_inventory_id.chars().any(char::is_control)
        {
            return Err(BridgeError::InvalidInput("stable_inventory_id"));
        }
        let provider_or_connector_id = provider_or_connector_id.into();
        validate_identifier(&provider_or_connector_id, "provider_or_connector_id")?;
        Ok(Self {
            consumer,
            stable_inventory_id,
            provider_or_connector_id,
            credential_ref,
            expires_at,
        })
    }
}

fn validate_identifier(value: &str, field: &'static str) -> BridgeResult<()> {
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(BridgeError::InvalidInput(field));
    }
    Ok(())
}
