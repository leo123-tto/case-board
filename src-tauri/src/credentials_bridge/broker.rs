use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use sqlx::FromRow;
use sqlx::SqlitePool;
use zeroize::{Zeroize, Zeroizing};

use super::encrypted_vault::{
    EncryptedCredentialVault, EncryptedEnvelopeV1, EnvelopeContext, VaultBackend, ALGORITHM_NAME,
};
use super::master_key::ensure_master_key;
use super::metadata::MetadataStore;
use super::migration_journal::MigrationJournal;
use super::paths::BridgePaths;
use super::platform_permissions::ensure_bridge_directories;
use super::schema::ensure_manifest;
use super::secret_lease::{SecretLease, TypedSecretLease};
use super::types::{
    BridgeCredentialConsumer, BridgeCredentialMetadata, BridgeCredentialState, BridgeError,
    BridgeResult, CredentialConsumer, CredentialHandle, CredentialRefV1, LeaseBinding,
    NewBridgeCredential, PendingSecretLeaseRequest, SecretLeaseRequest, ENVELOPE_VERSION,
};

#[derive(Clone)]
pub struct CredentialBroker {
    paths: BridgePaths,
    metadata: MetadataStore,
    vault: Arc<dyn VaultBackend>,
}

impl fmt::Debug for CredentialBroker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialBroker")
            .field("paths", &self.paths)
            .field("metadata", &self.metadata)
            .field("vault", &"<redacted>")
            .finish()
    }
}

impl CredentialBroker {
    pub async fn initialize(app_data_root: impl AsRef<Path>) -> BridgeResult<Self> {
        let paths = BridgePaths::new(app_data_root);
        let vault = Arc::new(EncryptedCredentialVault::new(paths.master_key_path()));
        Self::initialize_parts(paths, vault).await
    }

    pub async fn initialize_with_vault<V>(
        app_data_root: impl AsRef<Path>,
        vault: Arc<V>,
    ) -> BridgeResult<Self>
    where
        V: VaultBackend + 'static,
    {
        Self::initialize_parts(BridgePaths::new(app_data_root), vault).await
    }

    async fn initialize_parts(
        paths: BridgePaths,
        vault: Arc<dyn VaultBackend>,
    ) -> BridgeResult<Self> {
        ensure_bridge_directories(&paths)?;
        ensure_master_key(paths.master_key_path())?;
        ensure_manifest(paths.manifest_path())?;
        let metadata = MetadataStore::open(paths.database_path()).await?;
        Ok(Self {
            paths,
            metadata,
            vault,
        })
    }

    pub fn paths(&self) -> &BridgePaths {
        &self.paths
    }

    pub(crate) fn metadata_pool(&self) -> &SqlitePool {
        &self.metadata.pool
    }

    pub async fn status(
        &self,
        handle: impl AsRef<str>,
    ) -> BridgeResult<Option<BridgeCredentialMetadata>> {
        self.metadata
            .get(&CredentialHandle::new(handle.as_ref())?)
            .await
    }

    pub async fn list(&self) -> BridgeResult<Vec<BridgeCredentialMetadata>> {
        self.metadata.list().await
    }

    pub async fn pending_reference(
        &self,
        stable_inventory_id: &str,
        consumer: BridgeCredentialConsumer,
        provider_or_connector_id: &str,
    ) -> BridgeResult<Option<CredentialRefV1>> {
        if !consumer.permits(stable_inventory_id, provider_or_connector_id) {
            return Err(BridgeError::ConsumerNotAllowed {
                consumer: consumer.as_str(),
                provider_or_connector_id: provider_or_connector_id.to_owned(),
            });
        }
        let journal = MigrationJournal::new(self.metadata_pool());
        let Some(entry) = journal.get(stable_inventory_id).await? else {
            return Ok(None);
        };
        if entry.descriptor.provider_or_connector_id != provider_or_connector_id {
            return Err(BridgeError::PendingCredentialMismatch {
                field: "provider_or_connector_id",
            });
        }
        let Some(revision) = entry.revision else {
            return Err(BridgeError::PendingCredentialUnreadable {
                stable_inventory_id: stable_inventory_id.to_owned(),
            });
        };
        if !entry.authenticated {
            return Err(BridgeError::PendingCredentialUnreadable {
                stable_inventory_id: stable_inventory_id.to_owned(),
            });
        }
        let metadata = self.metadata.get(&entry.handle).await?.ok_or_else(|| {
            BridgeError::PendingCredentialUnreadable {
                stable_inventory_id: stable_inventory_id.to_owned(),
            }
        })?;
        if metadata.provider_or_connector_id != entry.descriptor.provider_or_connector_id {
            return Err(BridgeError::PendingCredentialMismatch {
                field: "provider_or_connector_id",
            });
        }
        if metadata.kind != entry.descriptor.kind {
            return Err(BridgeError::PendingCredentialMismatch { field: "kind" });
        }
        if metadata.owner_scope != entry.descriptor.owner_scope {
            return Err(BridgeError::PendingCredentialMismatch {
                field: "owner_scope",
            });
        }
        if metadata.revision != revision {
            return Err(BridgeError::PendingCredentialMismatch { field: "revision" });
        }
        if !matches!(
            metadata.state,
            BridgeCredentialState::PendingMigration | BridgeCredentialState::Valid
        ) {
            return Err(BridgeError::PendingCredentialUnreadable {
                stable_inventory_id: stable_inventory_id.to_owned(),
            });
        }
        Ok(Some(CredentialRefV1 {
            handle: entry.handle,
            revision,
        }))
    }

    pub async fn issue_pending_lease(
        &self,
        request: PendingSecretLeaseRequest,
    ) -> BridgeResult<TypedSecretLease> {
        if Instant::now() >= request.expires_at {
            return Err(BridgeError::LeaseExpired);
        }
        let Some(actual_ref) = self
            .pending_reference(
                &request.stable_inventory_id,
                request.consumer,
                &request.provider_or_connector_id,
            )
            .await?
        else {
            return Err(BridgeError::PendingCredentialMissing {
                stable_inventory_id: request.stable_inventory_id,
            });
        };
        if actual_ref.handle != request.credential_ref.handle {
            return Err(BridgeError::PendingCredentialMismatch { field: "handle" });
        }
        if actual_ref.revision != request.credential_ref.revision {
            return Err(BridgeError::PendingCredentialMismatch { field: "revision" });
        }
        let consumer = CredentialConsumer::new(request.consumer.as_str())?;
        let lease_request = SecretLeaseRequest::new(
            consumer.clone(),
            &request.provider_or_connector_id,
            request.credential_ref.handle.as_str(),
            request.credential_ref.revision,
            request.expires_at,
        )?;
        let binding = LeaseBinding::new(
            consumer,
            &request.provider_or_connector_id,
            request.credential_ref.handle.as_str(),
            request.credential_ref.revision,
        )?;
        let lease = self.issue_lease(lease_request).await?;
        Ok(TypedSecretLease::new(lease, binding))
    }

    pub async fn save(
        &self,
        credential: NewBridgeCredential,
    ) -> BridgeResult<BridgeCredentialMetadata> {
        let NewBridgeCredential {
            handle,
            provider_or_connector_id,
            kind,
            owner_scope,
            state,
            mut secret,
        } = credential;

        let mut transaction = self.metadata.pool.begin().await?;
        let existing: Option<(i64, i64)> = sqlx::query_as(
            "SELECT revision, created_at_ms
             FROM credential_metadata
             WHERE handle = ?",
        )
        .bind(handle.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let revision = existing
            .as_ref()
            .map_or(1, |(current, _)| current.saturating_add(1));
        if revision < 1 {
            return Err(BridgeError::CorruptMetadata(
                "credential revision overflow".to_owned(),
            ));
        }
        let now = chrono::Utc::now().timestamp_millis();
        let created_at_ms = existing.map_or(now, |(_, created)| created);
        let context = EnvelopeContext {
            handle: handle.clone(),
            revision,
            provider_or_connector_id: provider_or_connector_id.clone(),
            kind: kind.clone(),
            owner_scope: owner_scope.clone(),
        };
        let envelope = seal_and_clear(self.vault.as_ref(), &context, &mut secret)?;
        drop(secret);

        sqlx::query(
            "INSERT INTO credential_envelopes(
                handle, revision, version, algorithm, nonce, ciphertext, created_at_ms
             ) VALUES(?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(envelope.handle.as_str())
        .bind(envelope.revision)
        .bind(i64::from(envelope.version))
        .bind(envelope.algorithm)
        .bind(envelope.nonce.as_slice())
        .bind(envelope.ciphertext)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        sqlx::query(
            "INSERT INTO credential_metadata(
                handle, provider_or_connector_id, kind, owner_scope, revision,
                state, secret_present, created_at_ms, updated_at_ms
             ) VALUES(?, ?, ?, ?, ?, ?, 1, ?, ?)
             ON CONFLICT(handle) DO UPDATE SET
                provider_or_connector_id = excluded.provider_or_connector_id,
                kind = excluded.kind,
                owner_scope = excluded.owner_scope,
                revision = excluded.revision,
                state = excluded.state,
                secret_present = 1,
                updated_at_ms = excluded.updated_at_ms",
        )
        .bind(handle.as_str())
        .bind(&provider_or_connector_id)
        .bind(kind.as_storage_str())
        .bind(owner_scope.to_storage_string())
        .bind(revision)
        .bind(state.as_storage_str())
        .bind(created_at_ms)
        .bind(now)
        .execute(&mut *transaction)
        .await?;

        transaction.commit().await?;
        Ok(BridgeCredentialMetadata {
            handle,
            provider_or_connector_id,
            kind,
            owner_scope,
            revision,
            state,
            secret_present: true,
            created_at_ms,
            updated_at_ms: now,
        })
    }

    pub async fn issue_lease(&self, request: SecretLeaseRequest) -> BridgeResult<SecretLease> {
        let metadata = self
            .metadata
            .get(&request.binding.handle)
            .await?
            .ok_or_else(|| BridgeError::CredentialNotFound {
                handle: request.binding.handle.clone(),
            })?;
        if request.binding.revision != metadata.revision {
            return Err(BridgeError::StaleRevision {
                requested: request.binding.revision,
                current: metadata.revision,
            });
        }
        if request.binding.provider_or_connector_id != metadata.provider_or_connector_id {
            return Err(BridgeError::LeaseBindingMismatch {
                field: "provider_or_connector_id",
            });
        }

        let row = sqlx::query_as::<_, EnvelopeRow>(
            "SELECT handle, revision, version, algorithm, nonce, ciphertext
             FROM credential_envelopes
             WHERE handle = ? AND revision = ?",
        )
        .bind(metadata.handle.as_str())
        .bind(metadata.revision)
        .fetch_optional(&self.metadata.pool)
        .await?
        .ok_or_else(|| BridgeError::AuthenticationFailed {
            handle: metadata.handle.clone(),
            revision: metadata.revision,
        })?;
        let envelope = row.try_into_envelope(&metadata.handle)?;
        let context = EnvelopeContext {
            handle: metadata.handle.clone(),
            revision: metadata.revision,
            provider_or_connector_id: metadata.provider_or_connector_id,
            kind: metadata.kind,
            owner_scope: metadata.owner_scope,
        };
        let secret = match self.vault.open(&context, &envelope) {
            Ok(secret) => secret,
            Err(error @ BridgeError::AuthenticationFailed { .. }) => {
                self.metadata
                    .mark_unreadable(&context.handle, context.revision)
                    .await?;
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        Ok(SecretLease::new(
            request.binding,
            request.expires_at,
            secret,
        ))
    }
}

fn seal_and_clear(
    vault: &dyn VaultBackend,
    context: &EnvelopeContext,
    secret: &mut Zeroizing<Vec<u8>>,
) -> BridgeResult<EncryptedEnvelopeV1> {
    let result = vault.seal(context, secret.as_slice());
    secret.zeroize();
    secret.clear();
    result
}

#[derive(Debug, FromRow)]
struct EnvelopeRow {
    handle: String,
    revision: i64,
    version: i64,
    algorithm: String,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

impl EnvelopeRow {
    fn try_into_envelope(
        self,
        expected_handle: &CredentialHandle,
    ) -> BridgeResult<EncryptedEnvelopeV1> {
        let authentication_error = || BridgeError::AuthenticationFailed {
            handle: expected_handle.clone(),
            revision: self.revision,
        };
        if self.version != i64::from(ENVELOPE_VERSION)
            || self.algorithm != ALGORITHM_NAME
            || self.handle != expected_handle.as_str()
        {
            return Err(authentication_error());
        }
        let nonce: [u8; 24] = self.nonce.try_into().map_err(|_| authentication_error())?;
        Ok(EncryptedEnvelopeV1 {
            version: ENVELOPE_VERSION,
            handle: expected_handle.clone(),
            revision: self.revision,
            algorithm: ALGORITHM_NAME,
            nonce,
            ciphertext: self.ciphertext,
        })
    }
}
