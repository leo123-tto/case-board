use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use crate::credentials_bridge::{
    BridgeCredentialMetadata, BridgeCredentialState, CredentialBroker, CredentialConsumer,
    CredentialKind, CredentialOwnerScope, NewBridgeCredential, SecretLeaseRequest,
};

fn owner_scope_for_new_credential(provider_or_connector_id: &str) -> CredentialOwnerScope {
    if provider_or_connector_id.starts_with("mcp:") {
        CredentialOwnerScope::Connector(provider_or_connector_id.to_owned())
    } else {
        CredentialOwnerScope::Global
    }
}

fn default_stable_inventory_id(
    provider_or_connector_id: &str,
    kind: &CredentialKind,
) -> Option<&'static str> {
    match (provider_or_connector_id, kind) {
        ("ocr.mineru", CredentialKind::ApiKey) => Some("settings:mineru_api_key"),
        ("ocr.paddle_vl", CredentialKind::ApiKey) => Some("settings:paddle_vl_api_key"),
        ("llm.deepseek", CredentialKind::ApiKey) => Some("settings:cloud_llm_api_key"),
        ("llm.minimax", CredentialKind::ApiKey) => Some("settings:minimax_api_key"),
        ("llm.compat", CredentialKind::ApiKey) => Some("settings:compat_llm_api_key"),
        ("llm.glm", CredentialKind::ApiKey) => Some("settings:glm_llm_api_key"),
        ("llm.mimo", CredentialKind::ApiKey) => Some("settings:mimo_llm_api_key"),
        ("llm.kimi", CredentialKind::ApiKey) => Some("settings:kimi_llm_api_key"),
        ("llm.custom", CredentialKind::ApiKey) => Some("settings:custom_llm_api_key"),
        ("connector.yuandian", CredentialKind::ApiKey) => Some("settings:yuandian_api_key"),
        ("connector.kuaidi100", CredentialKind::ApiKey) => Some("settings:kuaidi100_key"),
        ("embedding", CredentialKind::ApiKey) => Some("settings:embedding_api_key"),
        ("feishu.calendar", CredentialKind::AppToken) => Some("settings:feishu_app_token"),
        ("feishu.reminder", CredentialKind::WebhookSecret) => Some("settings:feishu_webhook_url"),
        ("court_filing", CredentialKind::Password) => Some("settings:court_filing_password"),
        _ => None,
    }
}

async fn list_with_broker(
    broker: &CredentialBroker,
    owner_scope: Option<CredentialOwnerScope>,
) -> Result<Vec<BridgeCredentialMetadata>, String> {
    let metadata = broker.list().await.map_err(|error| error.to_string())?;
    Ok(metadata
        .into_iter()
        .filter(|item| {
            owner_scope
                .as_ref()
                .is_none_or(|expected| &item.owner_scope == expected)
        })
        .collect())
}

async fn save_with_broker(
    broker: &CredentialBroker,
    handle: Option<String>,
    provider_or_connector_id: String,
    kind: CredentialKind,
    secret_input: String,
    stable_inventory_id: Option<String>,
) -> Result<BridgeCredentialMetadata, String> {
    if secret_input.is_empty() {
        return Err("secret_input 不能为空".to_owned());
    }
    let stable_inventory_id = stable_inventory_id.or_else(|| {
        default_stable_inventory_id(&provider_or_connector_id, &kind).map(ToOwned::to_owned)
    });
    if let Some(stable_inventory_id) = stable_inventory_id.as_deref() {
        if stable_inventory_id.is_empty()
            || stable_inventory_id.len() > 1024
            || stable_inventory_id.chars().any(char::is_control)
        {
            return Err("stable_inventory_id 无效".to_owned());
        }
    }
    let mapped_handle = if let Some(stable_inventory_id) = stable_inventory_id.as_deref() {
        sqlx::query_scalar::<_, String>(
            "SELECT handle FROM pending_migration_journal WHERE stable_inventory_id = ?",
        )
        .bind(stable_inventory_id)
        .fetch_optional(broker.metadata_pool())
        .await
        .map_err(|error| error.to_string())?
    } else {
        None
    };
    if handle.is_some() && mapped_handle.is_some() && handle != mapped_handle {
        return Err("既有 stable inventory mapping 的 handle 不匹配".to_owned());
    }
    let handle = handle
        .or(mapped_handle)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let owner_scope = match broker
        .status(&handle)
        .await
        .map_err(|error| error.to_string())?
    {
        Some(existing) => {
            if existing.provider_or_connector_id != provider_or_connector_id {
                return Err("既有 handle 的 provider_or_connector_id 不匹配".to_owned());
            }
            if existing.kind != kind {
                return Err("既有 handle 的 credential kind 不匹配".to_owned());
            }
            existing.owner_scope
        }
        None => owner_scope_for_new_credential(&provider_or_connector_id),
    };
    let secret_input = Zeroizing::new(secret_input);
    let saved = broker
        .save(
            NewBridgeCredential::new(
                handle,
                provider_or_connector_id,
                kind,
                owner_scope,
                BridgeCredentialState::Unverified,
                secret_input.as_bytes().to_vec(),
            )
            .map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
    if let Some(stable_inventory_id) = stable_inventory_id {
        let now = chrono::Utc::now().timestamp_millis();
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
        .bind(stable_inventory_id)
        .bind(saved.handle.as_str())
        .bind(&saved.provider_or_connector_id)
        .bind(saved.kind.as_storage_str())
        .bind(saved.owner_scope.to_storage_string())
        .bind(saved.revision)
        .bind(now)
        .bind(now)
        .execute(broker.metadata_pool())
        .await
        .map_err(|error| error.to_string())?;
    }
    Ok(saved)
}

async fn verify_with_broker(
    broker: &CredentialBroker,
    handle: &str,
    revision: i64,
) -> Result<BridgeCredentialMetadata, String> {
    let metadata = broker
        .status(handle)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "credential not found".to_owned())?;
    if metadata.revision != revision {
        return Err(format!(
            "stale revision: requested {revision}, current {}",
            metadata.revision
        ));
    }
    if !metadata.secret_present
        || matches!(
            metadata.state,
            BridgeCredentialState::Revoked | BridgeCredentialState::Unreadable
        )
    {
        return Err("credential_missing/unreadable".to_owned());
    }
    let lease = broker
        .issue_lease(
            SecretLeaseRequest::new(
                CredentialConsumer::new("credential_editor").map_err(|error| error.to_string())?,
                metadata.provider_or_connector_id.clone(),
                handle,
                revision,
                Instant::now() + Duration::from_secs(30),
            )
            .map_err(|error| error.to_string())?,
        )
        .await
        .map_err(|error| error.to_string())?;
    lease.close();
    sqlx::query(
        "UPDATE credential_metadata
         SET state = 'valid', updated_at_ms = ?
         WHERE handle = ? AND revision = ? AND secret_present = 1",
    )
    .bind(chrono::Utc::now().timestamp_millis())
    .bind(handle)
    .bind(revision)
    .execute(broker.metadata_pool())
    .await
    .map_err(|error| error.to_string())?;
    broker
        .status(handle)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "credential not found after verify".to_owned())
}

async fn revoke_with_broker(
    broker: &CredentialBroker,
    handle: &str,
    revision: i64,
) -> Result<BridgeCredentialMetadata, String> {
    let result = sqlx::query(
        "UPDATE credential_metadata
         SET state = 'revoked', updated_at_ms = ?
         WHERE handle = ? AND revision = ?",
    )
    .bind(chrono::Utc::now().timestamp_millis())
    .bind(handle)
    .bind(revision)
    .execute(broker.metadata_pool())
    .await
    .map_err(|error| error.to_string())?;
    if result.rows_affected() != 1 {
        let current = broker
            .status(handle)
            .await
            .map_err(|error| error.to_string())?;
        return match current {
            Some(metadata) => Err(format!(
                "stale revision: requested {revision}, current {}",
                metadata.revision
            )),
            None => Err("credential not found".to_owned()),
        };
    }
    broker
        .status(handle)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "credential not found after revoke".to_owned())
}

async fn broker() -> Result<CredentialBroker, String> {
    let app_data_root = crate::db::app_data_dir().map_err(|error| error.to_string())?;
    CredentialBroker::initialize(app_data_root)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn list_credential_metadata(
    owner_scope: Option<CredentialOwnerScope>,
) -> Result<Vec<BridgeCredentialMetadata>, String> {
    list_with_broker(&broker().await?, owner_scope).await
}

#[tauri::command]
pub(crate) async fn save_credential(
    handle: Option<String>,
    provider_or_connector_id: String,
    kind: CredentialKind,
    secret_input: String,
    stable_inventory_id: Option<String>,
) -> Result<BridgeCredentialMetadata, String> {
    save_with_broker(
        &broker().await?,
        handle,
        provider_or_connector_id,
        kind,
        secret_input,
        stable_inventory_id,
    )
    .await
}

#[tauri::command]
pub(crate) async fn verify_credential(
    handle: String,
    revision: i64,
) -> Result<BridgeCredentialMetadata, String> {
    verify_with_broker(&broker().await?, &handle, revision).await
}

#[tauri::command]
pub(crate) async fn revoke_credential(
    handle: String,
    revision: i64,
) -> Result<BridgeCredentialMetadata, String> {
    revoke_with_broker(&broker().await?, &handle, revision).await
}

#[tauri::command]
pub(crate) async fn get_legacy_credential_migration_status(
) -> Result<crate::credentials_bridge::LegacyCredentialMigrationStatus, String> {
    let app_data_root = crate::db::app_data_dir().map_err(|error| error.to_string())?;
    crate::credentials_bridge::legacy_system_import_status(app_data_root)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn start_legacy_credential_migration(
    confirmed: bool,
) -> Result<crate::credentials_bridge::LegacyCredentialMigrationStatus, String> {
    let app_data_root = crate::db::app_data_dir().map_err(|error| error.to_string())?;
    crate::credentials_bridge::import_legacy_system_credentials(app_data_root, confirmed)
        .await
        .map_err(|error| error.to_string())
}
