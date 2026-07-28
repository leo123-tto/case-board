use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use zeroize::{Zeroize, Zeroizing};

use super::platform_permissions::{verify_secure_file, SecureAtomicFile};
use super::{
    acquire_pending_migration_lock, BridgeCredentialState, BridgeError, BridgePaths,
    CredentialBroker, CredentialConsumer, CredentialHandle, CredentialKind, CredentialOwnerScope,
    LeaseBinding, NewBridgeCredential, SecretLeaseRequest,
};
use crate::chat::runtime::pi_credentials::validate_credential;
use crate::chat::runtime::pi_protocol::PiCredential;

const IMPORT_MANIFEST_SCHEMA: &str = "caseboard-legacy-system-import/v1";
const PI_CREDENTIAL_SERVICE: &str = "CaseBoard/pi";
const RESEARCH_CREDENTIAL_SERVICE: &str = "CaseBoard/network-research";
const MAX_PI_CREDENTIAL_BYTES: usize = 64 * 1024;
const MAX_RESEARCH_ENVELOPE_BYTES: usize = 24 * 1024;
const MAX_RESEARCH_KEY_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LegacyPayloadKind {
    Pi {
        allows_api_key: bool,
        allows_oauth: bool,
    },
    ResearchApiKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacySystemImportTarget {
    pub stable_inventory_id: &'static str,
    pub provider_or_connector_id: &'static str,
    pub kind: CredentialKind,
    pub legacy_service: &'static str,
    pub legacy_account: &'static str,
    payload_kind: LegacyPayloadKind,
}

macro_rules! pi_target {
    ($provider:literal, $api_key:literal, $oauth:literal) => {
        LegacySystemImportTarget {
            stable_inventory_id: concat!("legacy-system:pi:", $provider, ":credential"),
            provider_or_connector_id: concat!("pi.", $provider),
            kind: CredentialKind::PiCredentialBundle,
            legacy_service: PI_CREDENTIAL_SERVICE,
            legacy_account: $provider,
            payload_kind: LegacyPayloadKind::Pi {
                allows_api_key: $api_key,
                allows_oauth: $oauth,
            },
        }
    };
}

const LEGACY_SYSTEM_IMPORT_TARGETS: &[LegacySystemImportTarget] = &[
    pi_target!("amazon-bedrock", true, false),
    pi_target!("ant-ling", true, false),
    pi_target!("anthropic", true, true),
    pi_target!("azure-openai-responses", true, false),
    pi_target!("cerebras", true, false),
    pi_target!("cloudflare-ai-gateway", true, false),
    pi_target!("cloudflare-workers-ai", true, false),
    pi_target!("deepseek", true, false),
    pi_target!("fireworks", true, false),
    pi_target!("github-copilot", true, true),
    pi_target!("google", true, false),
    pi_target!("google-vertex", true, false),
    pi_target!("groq", true, false),
    pi_target!("huggingface", true, false),
    pi_target!("kimi-coding", true, false),
    pi_target!("minimax", true, false),
    pi_target!("minimax-cn", true, false),
    pi_target!("mistral", true, false),
    pi_target!("moonshotai", true, false),
    pi_target!("moonshotai-cn", true, false),
    pi_target!("nvidia", true, false),
    pi_target!("openai", true, false),
    pi_target!("openai-codex", false, true),
    pi_target!("opencode", true, false),
    pi_target!("opencode-go", true, false),
    pi_target!("openrouter", true, false),
    pi_target!("together", true, false),
    pi_target!("vercel-ai-gateway", true, false),
    pi_target!("xai", true, true),
    pi_target!("xiaomi", true, false),
    pi_target!("xiaomi-token-plan-ams", true, false),
    pi_target!("xiaomi-token-plan-cn", true, false),
    pi_target!("xiaomi-token-plan-sgp", true, false),
    pi_target!("zai", true, false),
    pi_target!("zai-coding-cn", true, false),
    LegacySystemImportTarget {
        stable_inventory_id: "legacy-system:research:exa:api-key",
        provider_or_connector_id: "research.exa",
        kind: CredentialKind::ApiKey,
        legacy_service: RESEARCH_CREDENTIAL_SERVICE,
        legacy_account: "exa",
        payload_kind: LegacyPayloadKind::ResearchApiKey,
    },
    LegacySystemImportTarget {
        stable_inventory_id: "legacy-system:research:firecrawl:api-key",
        provider_or_connector_id: "research.firecrawl",
        kind: CredentialKind::ApiKey,
        legacy_service: RESEARCH_CREDENTIAL_SERVICE,
        legacy_account: "firecrawl",
        payload_kind: LegacyPayloadKind::ResearchApiKey,
    },
];

pub fn legacy_system_import_targets() -> &'static [LegacySystemImportTarget] {
    LEGACY_SYSTEM_IMPORT_TARGETS
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacySystemImportState {
    Pending,
    Imported,
    AlreadyImported,
    Missing,
    Unreadable,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacySystemImportItem {
    pub stable_inventory_id: String,
    pub provider_or_connector_id: String,
    pub state: LegacySystemImportState,
    pub handle: Option<String>,
    pub revision: Option<i64>,
    pub expired: bool,
    pub reconnect_required: bool,
    pub error: Option<String>,
}

impl LegacySystemImportItem {
    fn pending(target: &LegacySystemImportTarget) -> Self {
        Self {
            stable_inventory_id: target.stable_inventory_id.to_owned(),
            provider_or_connector_id: target.provider_or_connector_id.to_owned(),
            state: LegacySystemImportState::Pending,
            handle: None,
            revision: None,
            expired: false,
            reconnect_required: false,
            error: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyCredentialMigrationStatus {
    pub pending: bool,
    pub pending_count: usize,
    pub declined: bool,
    pub failed: bool,
    pub attempted: bool,
    pub last_attempted_at_ms: Option<i64>,
    pub items: Vec<LegacySystemImportItem>,
}

impl LegacyCredentialMigrationStatus {
    fn initial() -> Self {
        Self {
            pending: true,
            pending_count: LEGACY_SYSTEM_IMPORT_TARGETS.len(),
            declined: false,
            failed: false,
            attempted: false,
            last_attempted_at_ms: None,
            items: LEGACY_SYSTEM_IMPORT_TARGETS
                .iter()
                .map(LegacySystemImportItem::pending)
                .collect(),
        }
    }

    fn refresh_summary(&mut self) {
        self.pending_count = self
            .items
            .iter()
            .filter(|item| {
                matches!(
                    item.state,
                    LegacySystemImportState::Pending
                        | LegacySystemImportState::Unreadable
                        | LegacySystemImportState::Failed
                )
            })
            .count();
        self.pending = self.pending_count > 0;
        self.failed = self.items.iter().any(|item| {
            matches!(
                item.state,
                LegacySystemImportState::Unreadable | LegacySystemImportState::Failed
            )
        });
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacySystemImportManifest {
    schema: String,
    status: LegacyCredentialMigrationStatus,
}

pub trait LegacySystemCredentialSource: Send + Sync {
    fn read(&self, target: &LegacySystemImportTarget)
        -> Result<Option<Zeroizing<Vec<u8>>>, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemLegacyCredentialSource;

impl LegacySystemCredentialSource for SystemLegacyCredentialSource {
    fn read(
        &self,
        target: &LegacySystemImportTarget,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, String> {
        let entry = keyring::Entry::new(target.legacy_service, target.legacy_account)
            .map_err(|error| format!("无法打开系统凭据库:{error}"))?;
        match entry.get_password() {
            Ok(encoded) => Ok(Some(Zeroizing::new(encoded.into_bytes()))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(format!("无法读取系统凭据库:{error}")),
        }
    }
}

pub fn legacy_system_import_status(
    app_data_root: impl AsRef<Path>,
) -> Result<LegacyCredentialMigrationStatus, BridgeError> {
    let paths = BridgePaths::new(app_data_root);
    let path = paths.legacy_system_import_manifest_path();
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LegacyCredentialMigrationStatus::initial());
        }
        Err(source) => {
            return Err(BridgeError::Io {
                operation: "inspect legacy system import manifest",
                path: path.to_path_buf(),
                source,
            });
        }
    }
    verify_secure_file(path)?;
    let bytes = fs::read(path).map_err(|source| BridgeError::Io {
        operation: "read legacy system import manifest",
        path: path.to_path_buf(),
        source,
    })?;
    let manifest: LegacySystemImportManifest = serde_json::from_slice(&bytes)
        .map_err(|error| BridgeError::InvalidManifest(error.to_string()))?;
    validate_manifest(&manifest)?;
    Ok(manifest.status)
}

pub async fn import_legacy_system_credentials(
    app_data_root: impl AsRef<Path>,
    confirmed: bool,
) -> Result<LegacyCredentialMigrationStatus, BridgeError> {
    import_legacy_system_credentials_with_source(
        app_data_root,
        confirmed,
        &SystemLegacyCredentialSource,
    )
    .await
}

pub async fn import_legacy_system_credentials_with_source(
    app_data_root: impl AsRef<Path>,
    confirmed: bool,
    source: &dyn LegacySystemCredentialSource,
) -> Result<LegacyCredentialMigrationStatus, BridgeError> {
    let app_data_root = app_data_root.as_ref();
    if !confirmed {
        return legacy_system_import_status(app_data_root);
    }

    let _migration_lock = acquire_pending_migration_lock(app_data_root)?;
    let broker = CredentialBroker::initialize(app_data_root).await?;
    let mut status = LegacyCredentialMigrationStatus::initial();
    status.attempted = true;
    status.last_attempted_at_ms = Some(chrono::Utc::now().timestamp_millis());

    for (index, target) in LEGACY_SYSTEM_IMPORT_TARGETS.iter().enumerate() {
        status.items[index] = import_one(&broker, source, target).await;
        status.refresh_summary();
        write_manifest(broker.paths().legacy_system_import_manifest_path(), &status)?;
    }
    Ok(status)
}

async fn import_one(
    broker: &CredentialBroker,
    source: &dyn LegacySystemCredentialSource,
    target: &LegacySystemImportTarget,
) -> LegacySystemImportItem {
    let raw = match source.read(target) {
        Ok(Some(raw)) => raw,
        Ok(None) => {
            return result_without_secret(target, LegacySystemImportState::Missing, None);
        }
        Err(error) => {
            return result_without_secret(
                target,
                LegacySystemImportState::Unreadable,
                Some(safe_error(&error)),
            );
        }
    };
    let NormalizedPayload {
        secret,
        expired,
        reconnect_required,
    } = match normalize_payload(target, raw) {
        Ok(payload) => payload,
        Err(error) => {
            return result_without_secret(target, LegacySystemImportState::Unreadable, Some(error));
        }
    };

    match save_snapshot_idempotently(broker, target, secret).await {
        Ok((state, handle, revision)) => LegacySystemImportItem {
            stable_inventory_id: target.stable_inventory_id.to_owned(),
            provider_or_connector_id: target.provider_or_connector_id.to_owned(),
            state,
            handle: Some(handle),
            revision: Some(revision),
            expired,
            reconnect_required,
            error: None,
        },
        Err(error) => result_without_secret(
            target,
            LegacySystemImportState::Failed,
            Some(safe_error(&error.to_string())),
        ),
    }
}

fn result_without_secret(
    target: &LegacySystemImportTarget,
    state: LegacySystemImportState,
    error: Option<String>,
) -> LegacySystemImportItem {
    LegacySystemImportItem {
        stable_inventory_id: target.stable_inventory_id.to_owned(),
        provider_or_connector_id: target.provider_or_connector_id.to_owned(),
        state,
        handle: None,
        revision: None,
        expired: false,
        reconnect_required: false,
        error,
    }
}

struct NormalizedPayload {
    secret: Zeroizing<Vec<u8>>,
    expired: bool,
    reconnect_required: bool,
}

#[derive(Deserialize)]
struct LegacyResearchCredential {
    version: u8,
    key: String,
    #[allow(dead_code)]
    verified_at: Option<String>,
    #[allow(dead_code)]
    last_error_kind: Option<String>,
}

fn normalize_payload(
    target: &LegacySystemImportTarget,
    raw: Zeroizing<Vec<u8>>,
) -> Result<NormalizedPayload, String> {
    match target.payload_kind {
        LegacyPayloadKind::Pi {
            allows_api_key,
            allows_oauth,
        } => {
            if raw.len() > MAX_PI_CREDENTIAL_BYTES {
                return Err("Pi credential 超过安全长度限制".to_owned());
            }
            let mut credential: PiCredential =
                serde_json::from_slice(&raw).map_err(|_| "Pi credential 格式无效".to_owned())?;
            validate_credential(&credential).map_err(|_| "Pi credential 字段无效".to_owned())?;
            let allowed = matches!(&credential, PiCredential::ApiKey { .. }) && allows_api_key
                || matches!(&credential, PiCredential::OAuth { .. }) && allows_oauth;
            if !allowed {
                zeroize_pi_credential(&mut credential);
                return Err("Pi credential 认证形态与固定 provider registry 不匹配".to_owned());
            }
            let expired = matches!(
                &credential,
                PiCredential::OAuth { expires, .. }
                    if *expires <= chrono::Utc::now().timestamp_millis().max(0) as u64
            );
            let encoded = serde_json::to_vec(&credential)
                .map_err(|_| "Pi credential 规范化失败".to_owned())?;
            zeroize_pi_credential(&mut credential);
            Ok(NormalizedPayload {
                secret: Zeroizing::new(encoded),
                expired,
                reconnect_required: expired,
            })
        }
        LegacyPayloadKind::ResearchApiKey => {
            if raw.len() > MAX_RESEARCH_ENVELOPE_BYTES {
                return Err("网络研究凭据超过安全长度限制".to_owned());
            }
            let mut credential: LegacyResearchCredential =
                serde_json::from_slice(&raw).map_err(|_| "网络研究凭据格式无效".to_owned())?;
            if credential.version != 1
                || credential.key.trim().is_empty()
                || credential.key.len() > MAX_RESEARCH_KEY_BYTES
                || credential.key.chars().any(char::is_control)
            {
                credential.key.zeroize();
                return Err("网络研究凭据字段无效".to_owned());
            }
            let secret = Zeroizing::new(credential.key.as_bytes().to_vec());
            credential.key.zeroize();
            Ok(NormalizedPayload {
                secret,
                expired: false,
                reconnect_required: false,
            })
        }
    }
}

fn zeroize_pi_credential(credential: &mut PiCredential) {
    match credential {
        PiCredential::ApiKey { key, env } => {
            if let Some(key) = key {
                key.zeroize();
            }
            for (mut name, mut value) in std::mem::take(env) {
                name.zeroize();
                value.zeroize();
            }
        }
        PiCredential::OAuth {
            access,
            refresh,
            expires,
            extra,
        } => {
            access.zeroize();
            refresh.zeroize();
            expires.zeroize();
            for (mut name, mut value) in std::mem::take(extra) {
                name.zeroize();
                zeroize_json(&mut value);
            }
        }
    }
}

fn zeroize_json(value: &mut Value) {
    match value {
        Value::String(value) => value.zeroize(),
        Value::Array(values) => values.iter_mut().for_each(zeroize_json),
        Value::Object(values) => {
            for (mut name, mut value) in std::mem::take(values) {
                name.zeroize();
                zeroize_json(&mut value);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

async fn save_snapshot_idempotently(
    broker: &CredentialBroker,
    target: &LegacySystemImportTarget,
    secret: Zeroizing<Vec<u8>>,
) -> Result<(LegacySystemImportState, String, i64), BridgeError> {
    let owner_scope = owner_scope(target);
    let mapping = sqlx::query_as::<_, (String, String, String, String, Option<i64>, i64)>(
        "SELECT handle, provider_or_connector_id, kind, owner_scope, revision, authenticated
         FROM pending_migration_journal
         WHERE stable_inventory_id = ?",
    )
    .bind(target.stable_inventory_id)
    .fetch_optional(broker.metadata_pool())
    .await?;
    let handle = if let Some((handle, provider, kind, owner, _, _)) = &mapping {
        if provider != target.provider_or_connector_id
            || kind != target.kind.as_storage_str()
            || owner != &owner_scope.to_storage_string()
        {
            return Err(BridgeError::PendingCredentialMismatch {
                field: "legacy system import descriptor",
            });
        }
        CredentialHandle::new(handle)?
    } else {
        let handle = CredentialHandle::new(uuid::Uuid::new_v4().to_string())?;
        let now = chrono::Utc::now().timestamp_millis();
        sqlx::query(
            "INSERT INTO pending_migration_journal(
                stable_inventory_id, handle, provider_or_connector_id, kind, owner_scope,
                revision, authenticated, created_at_ms, updated_at_ms
             ) VALUES(?, ?, ?, ?, ?, NULL, 0, ?, ?)",
        )
        .bind(target.stable_inventory_id)
        .bind(handle.as_str())
        .bind(target.provider_or_connector_id)
        .bind(target.kind.as_storage_str())
        .bind(owner_scope.to_storage_string())
        .bind(now)
        .bind(now)
        .execute(broker.metadata_pool())
        .await?;
        handle
    };

    if let Some(metadata) = broker.status(handle.as_str()).await? {
        if metadata.provider_or_connector_id != target.provider_or_connector_id
            || metadata.kind.as_storage_str() != target.kind.as_storage_str()
            || metadata.owner_scope != owner_scope
        {
            return Err(BridgeError::PendingCredentialMismatch {
                field: "legacy system import metadata",
            });
        }
        let current_matches = current_secret_matches(broker, &metadata, &secret).await?;
        if current_matches {
            activate_snapshot(
                broker,
                target,
                &owner_scope,
                handle.as_str(),
                metadata.revision,
            )
            .await?;
            return Ok((
                LegacySystemImportState::AlreadyImported,
                handle.to_string(),
                metadata.revision,
            ));
        }
    }

    let saved = broker
        .save(NewBridgeCredential::new(
            handle.as_str(),
            target.provider_or_connector_id,
            target.kind.clone(),
            owner_scope.clone(),
            BridgeCredentialState::PendingMigration,
            secret.as_slice().to_vec(),
        )?)
        .await?;
    if !current_secret_matches(broker, &saved, &secret).await? {
        return Err(BridgeError::PendingCredentialMismatch {
            field: "legacy system import plaintext",
        });
    }
    activate_snapshot(
        broker,
        target,
        &owner_scope,
        handle.as_str(),
        saved.revision,
    )
    .await?;
    Ok((
        LegacySystemImportState::Imported,
        handle.to_string(),
        saved.revision,
    ))
}

async fn current_secret_matches(
    broker: &CredentialBroker,
    metadata: &super::BridgeCredentialMetadata,
    expected: &[u8],
) -> Result<bool, BridgeError> {
    let consumer = CredentialConsumer::new("legacy_system_import")?;
    let mut lease = broker
        .issue_lease(SecretLeaseRequest::new(
            consumer.clone(),
            metadata.provider_or_connector_id.clone(),
            metadata.handle.as_str(),
            metadata.revision,
            Instant::now() + Duration::from_secs(30),
        )?)
        .await?;
    lease.with_secret(
        &LeaseBinding {
            consumer,
            provider_or_connector_id: metadata.provider_or_connector_id.clone(),
            handle: metadata.handle.clone(),
            revision: metadata.revision,
        },
        |actual| actual == expected,
    )
}

async fn activate_snapshot(
    broker: &CredentialBroker,
    target: &LegacySystemImportTarget,
    owner_scope: &CredentialOwnerScope,
    handle: &str,
    revision: i64,
) -> Result<(), BridgeError> {
    let now = chrono::Utc::now().timestamp_millis();
    let mut transaction = broker.metadata_pool().begin().await?;
    let updated = sqlx::query(
        "UPDATE credential_metadata
         SET state = 'valid', updated_at_ms = ?
         WHERE handle = ? AND revision = ? AND secret_present = 1",
    )
    .bind(now)
    .bind(handle)
    .bind(revision)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() != 1 {
        return Err(BridgeError::PendingCredentialMismatch {
            field: "legacy system import activation",
        });
    }
    sqlx::query(
        "INSERT INTO pending_migration_journal(
            stable_inventory_id, handle, provider_or_connector_id, kind, owner_scope,
            revision, authenticated, created_at_ms, updated_at_ms
         ) VALUES(?, ?, ?, ?, ?, ?, 1, ?, ?)
         ON CONFLICT(stable_inventory_id) DO UPDATE SET
            handle = excluded.handle,
            provider_or_connector_id = excluded.provider_or_connector_id,
            kind = excluded.kind,
            owner_scope = excluded.owner_scope,
            revision = excluded.revision,
            authenticated = 1,
            updated_at_ms = excluded.updated_at_ms",
    )
    .bind(target.stable_inventory_id)
    .bind(handle)
    .bind(target.provider_or_connector_id)
    .bind(target.kind.as_storage_str())
    .bind(owner_scope.to_storage_string())
    .bind(revision)
    .bind(now)
    .bind(now)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

fn owner_scope(target: &LegacySystemImportTarget) -> CredentialOwnerScope {
    match target.payload_kind {
        LegacyPayloadKind::Pi { .. } => {
            CredentialOwnerScope::Provider(target.provider_or_connector_id.to_owned())
        }
        LegacyPayloadKind::ResearchApiKey => {
            CredentialOwnerScope::Connector(target.provider_or_connector_id.to_owned())
        }
    }
}

fn write_manifest(
    path: &Path,
    status: &LegacyCredentialMigrationStatus,
) -> Result<(), BridgeError> {
    let mut temp = SecureAtomicFile::new(path)?;
    serde_json::to_writer_pretty(
        temp.as_file_mut(),
        &LegacySystemImportManifest {
            schema: IMPORT_MANIFEST_SCHEMA.to_owned(),
            status: status.clone(),
        },
    )
    .map_err(|error| BridgeError::InvalidManifest(error.to_string()))?;
    temp.as_file_mut()
        .write_all(b"\n")
        .map_err(|source| BridgeError::Io {
            operation: "write legacy system import manifest",
            path: path.to_path_buf(),
            source,
        })?;
    temp.persist()
}

fn validate_manifest(manifest: &LegacySystemImportManifest) -> Result<(), BridgeError> {
    if manifest.schema != IMPORT_MANIFEST_SCHEMA
        || manifest.status.items.len() != LEGACY_SYSTEM_IMPORT_TARGETS.len()
    {
        return Err(BridgeError::InvalidManifest(
            "legacy system import manifest contract mismatch".to_owned(),
        ));
    }
    let expected = LEGACY_SYSTEM_IMPORT_TARGETS
        .iter()
        .map(|target| target.stable_inventory_id)
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for item in &manifest.status.items {
        let Some(target) = LEGACY_SYSTEM_IMPORT_TARGETS
            .iter()
            .find(|target| target.stable_inventory_id == item.stable_inventory_id)
        else {
            return Err(BridgeError::InvalidManifest(
                "unknown legacy system import target".to_owned(),
            ));
        };
        if item.provider_or_connector_id != target.provider_or_connector_id
            || !actual.insert(item.stable_inventory_id.as_str())
        {
            return Err(BridgeError::InvalidManifest(
                "legacy system import identity mismatch".to_owned(),
            ));
        }
    }
    if actual != expected {
        return Err(BridgeError::InvalidManifest(
            "legacy system import target set mismatch".to_owned(),
        ));
    }
    let mut clone = manifest.status.clone();
    clone.refresh_summary();
    if clone.pending != manifest.status.pending
        || clone.pending_count != manifest.status.pending_count
        || clone.failed != manifest.status.failed
    {
        return Err(BridgeError::InvalidManifest(
            "legacy system import summary mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn safe_error(error: &str) -> String {
    let normalized = error.to_ascii_lowercase();
    if normalized.contains("denied")
        || normalized.contains("authorization")
        || normalized.contains("permission")
    {
        "系统凭据库读取被拒绝".to_owned()
    } else {
        "凭据导入失败；可在设置中重试或重新连接".to_owned()
    }
}
