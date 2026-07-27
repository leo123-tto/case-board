use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::platform_permissions::verify_secure_file;
use super::types::{BridgeError, BridgeResult, CredentialHandle, PendingMigrationEntry};

#[derive(Debug, Serialize)]
struct PendingMigrationManifest<'a> {
    schema: &'static str,
    generated_at_ms: i64,
    entries: &'a [PendingMigrationEntry],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SanitizationStage {
    ReadyToRename,
    Active,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SanitizationManifestEntry {
    pub stable_inventory_id: String,
    pub handle: CredentialHandle,
    pub provider_or_connector_id: String,
    pub revision: i64,
}

impl From<&PendingMigrationEntry> for SanitizationManifestEntry {
    fn from(value: &PendingMigrationEntry) -> Self {
        Self {
            stable_inventory_id: value.stable_inventory_id.clone(),
            handle: value.handle.clone(),
            provider_or_connector_id: value.provider_or_connector_id.clone(),
            revision: value.revision,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SanitizationManifest {
    pub schema: String,
    pub stage: SanitizationStage,
    pub sanitized_settings_sha256: String,
    pub entries: Vec<SanitizationManifestEntry>,
}

pub(crate) fn write_pending_manifest(
    path: &Path,
    entries: &[PendingMigrationEntry],
) -> BridgeResult<()> {
    let parent = path
        .parent()
        .ok_or(BridgeError::InvalidInput("pending manifest parent"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|source| BridgeError::Io {
        operation: "create pending migration manifest",
        path: path.to_path_buf(),
        source,
    })?;
    let manifest = PendingMigrationManifest {
        schema: "caseboard-credential-pending-migration/v1",
        generated_at_ms: chrono::Utc::now().timestamp_millis(),
        entries,
    };
    serde_json::to_writer_pretty(&mut temp, &manifest)
        .map_err(|error| BridgeError::InvalidManifest(error.to_string()))?;
    temp.write_all(b"\n")
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|source| BridgeError::Io {
            operation: "sync pending migration manifest",
            path: path.to_path_buf(),
            source,
        })?;
    temp.persist(path).map_err(|error| BridgeError::Io {
        operation: "replace pending migration manifest",
        path: path.to_path_buf(),
        source: error.error,
    })?;
    verify_secure_file(path)
}

pub(crate) fn read_sanitization_manifest(
    path: &Path,
) -> BridgeResult<Option<SanitizationManifest>> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(BridgeError::Io {
                operation: "inspect sanitization manifest",
                path: path.to_path_buf(),
                source,
            });
        }
    }
    verify_secure_file(path)?;
    let bytes = fs::read(path).map_err(|source| BridgeError::Io {
        operation: "read sanitization manifest",
        path: path.to_path_buf(),
        source,
    })?;
    let manifest: SanitizationManifest = serde_json::from_slice(&bytes)
        .map_err(|error| BridgeError::InvalidManifest(error.to_string()))?;
    if manifest.schema != "caseboard-credential-sanitization/v1"
        || !sanitization_contract_is_valid(&manifest.sanitized_settings_sha256, &manifest.entries)
    {
        return Err(BridgeError::InvalidManifest(
            "sanitization manifest contract mismatch".to_owned(),
        ));
    }
    Ok(Some(manifest))
}

pub(crate) fn write_sanitization_manifest(
    path: &Path,
    stage: SanitizationStage,
    sanitized_settings_sha256: &str,
    entries: &[SanitizationManifestEntry],
) -> BridgeResult<()> {
    if !sanitization_contract_is_valid(sanitized_settings_sha256, entries) {
        return Err(BridgeError::InvalidManifest(
            "invalid sanitization manifest input".to_owned(),
        ));
    }
    let parent = path
        .parent()
        .ok_or(BridgeError::InvalidInput("sanitization manifest parent"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|source| BridgeError::Io {
        operation: "create sanitization manifest",
        path: path.to_path_buf(),
        source,
    })?;
    let manifest = SanitizationManifest {
        schema: "caseboard-credential-sanitization/v1".to_owned(),
        stage,
        sanitized_settings_sha256: sanitized_settings_sha256.to_owned(),
        entries: entries.to_vec(),
    };
    serde_json::to_writer_pretty(&mut temp, &manifest)
        .map_err(|error| BridgeError::InvalidManifest(error.to_string()))?;
    temp.write_all(b"\n")
        .and_then(|_| temp.as_file().sync_all())
        .map_err(|source| BridgeError::Io {
            operation: "sync sanitization manifest",
            path: path.to_path_buf(),
            source,
        })?;
    temp.persist(path).map_err(|error| BridgeError::Io {
        operation: "replace sanitization manifest",
        path: path.to_path_buf(),
        source: error.error,
    })?;
    verify_secure_file(path)?;
    sync_parent(parent, "sync sanitization manifest parent")
}

fn sanitization_contract_is_valid(
    sanitized_settings_sha256: &str,
    entries: &[SanitizationManifestEntry],
) -> bool {
    if sanitized_settings_sha256.len() != 64
        || !sanitized_settings_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return false;
    }
    let mut inventory_ids = HashSet::with_capacity(entries.len());
    let mut handles = HashSet::with_capacity(entries.len());
    entries.iter().all(|entry| {
        entry.revision >= 1
            && is_task3b_inventory_id(&entry.stable_inventory_id)
            && !entry.provider_or_connector_id.trim().is_empty()
            && !entry.provider_or_connector_id.chars().any(char::is_control)
            && !entry.handle.as_str().is_empty()
            && !entry.handle.as_str().chars().any(char::is_control)
            && inventory_ids.insert(entry.stable_inventory_id.as_str())
            && handles.insert(entry.handle.as_str())
    })
}

fn sync_parent(parent: &Path, operation: &'static str) -> BridgeResult<()> {
    let directory = fs::File::open(parent).map_err(|source| BridgeError::Io {
        operation,
        path: parent.to_path_buf(),
        source,
    })?;
    directory.sync_all().map_err(|source| BridgeError::Io {
        operation,
        path: parent.to_path_buf(),
        source,
    })
}

pub(crate) fn is_task3b_inventory_id(stable_inventory_id: &str) -> bool {
    matches!(
        stable_inventory_id,
        "settings:mineru_api_key"
            | "settings:paddle_vl_api_key"
            | "settings:cloud_llm_api_key"
            | "settings:minimax_api_key"
            | "settings:compat_llm_api_key"
            | "settings:glm_llm_api_key"
            | "settings:mimo_llm_api_key"
            | "settings:kimi_llm_api_key"
            | "settings:custom_llm_api_key"
            | "settings:yuandian_api_key"
            | "settings:embedding_api_key"
            | "settings:kuaidi100_key"
            | "settings:feishu_webhook_url"
    ) || (stable_inventory_id.starts_with("settings:mcp:")
        && matches!(
            stable_inventory_id.rsplit(':').next(),
            Some("env" | "headers")
        ))
}
