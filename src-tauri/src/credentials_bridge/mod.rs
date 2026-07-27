pub mod broker;
mod consumer;
pub mod encrypted_vault;
pub mod fakes;
mod legacy_plaintext;
mod legacy_system_import;
mod manifest;
pub mod master_key;
pub mod metadata;
mod migration_journal;
pub mod paths;
pub mod platform_permissions;
pub mod schema;
pub mod secret_lease;
pub mod types;

pub use broker::CredentialBroker;
pub use consumer::{BridgeSecretMaterial, PendingCredentialSource};
pub use legacy_system_import::{
    import_legacy_system_credentials, import_legacy_system_credentials_with_source,
    legacy_system_import_status, legacy_system_import_targets, LegacyCredentialMigrationStatus,
    LegacySystemCredentialSource, LegacySystemImportItem, LegacySystemImportState,
    LegacySystemImportTarget, SystemLegacyCredentialSource,
};
pub use paths::BridgePaths;
pub use secret_lease::{SecretLease, TypedSecretLease};
pub use types::{
    AutomaticMigrationCrashPoint, BridgeCredentialConsumer, BridgeCredentialMetadata,
    BridgeCredentialState, BridgeError, CredentialConsumer, CredentialHandle, CredentialKind,
    CredentialOwnerScope, CredentialRefV1, LeaseBinding, NewBridgeCredential,
    PendingMigrationEntry, PendingMigrationReport, PendingMigrationSourceError,
    PendingMigrationStatus, PendingSecretLeaseRequest, SecretLeaseRequest,
};

use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use legacy_plaintext::{prepare_sanitized_settings, read_legacy_plaintext};
use manifest::{
    is_task3b_inventory_id, read_sanitization_manifest, write_pending_manifest,
    write_sanitization_manifest, SanitizationManifestEntry, SanitizationStage,
};
use migration_journal::MigrationJournal;
use platform_permissions::{ensure_bridge_directories, open_or_create_secure_file};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

static ACTIVE_PENDING_MIGRATIONS: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

pub struct PendingMigrationLock {
    app_data_root: PathBuf,
    advisory_lock: Option<File>,
}

pub fn acquire_pending_migration_lock(
    app_data_root: impl AsRef<Path>,
) -> Result<PendingMigrationLock, BridgeError> {
    let app_data_root = app_data_root
        .as_ref()
        .canonicalize()
        .unwrap_or_else(|_| app_data_root.as_ref().to_path_buf());
    let active = ACTIVE_PENDING_MIGRATIONS.get_or_init(|| Mutex::new(HashSet::new()));
    let mut active = active
        .lock()
        .map_err(|_| BridgeError::CorruptMetadata("pending migration lock poisoned".to_owned()))?;
    if !active.insert(app_data_root.clone()) {
        return Err(BridgeError::MigrationLocked {
            path: app_data_root,
        });
    }
    drop(active);

    let mut migration_lock = PendingMigrationLock {
        app_data_root,
        advisory_lock: None,
    };
    let paths = BridgePaths::new(&migration_lock.app_data_root);
    ensure_bridge_directories(&paths)?;
    let lock_path = paths.pending_migration_lock_path();
    let advisory_lock = open_or_create_secure_file(lock_path)?;
    match fs4::FileExt::try_lock(&advisory_lock) {
        Ok(()) => migration_lock.advisory_lock = Some(advisory_lock),
        Err(fs4::TryLockError::WouldBlock) => {
            return Err(BridgeError::MigrationLocked {
                path: lock_path.to_path_buf(),
            });
        }
        Err(fs4::TryLockError::Error(source)) => {
            return Err(BridgeError::Io {
                operation: "acquire pending migration advisory lock",
                path: lock_path.to_path_buf(),
                source,
            });
        }
    }
    Ok(migration_lock)
}

impl Drop for PendingMigrationLock {
    fn drop(&mut self) {
        drop(self.advisory_lock.take());
        if let Some(active) = ACTIVE_PENDING_MIGRATIONS.get() {
            if let Ok(mut active) = active.lock() {
                active.remove(&self.app_data_root);
            }
        }
    }
}

pub async fn run_pending_migration(
    app_data_root: impl AsRef<Path>,
) -> Result<PendingMigrationReport, BridgeError> {
    let app_data_root = app_data_root.as_ref();
    let _lock = match acquire_pending_migration_lock(app_data_root) {
        Ok(lock) => lock,
        Err(error @ BridgeError::MigrationLocked { .. }) => {
            return Ok(PendingMigrationReport::skipped_locked(error.to_string()));
        }
        Err(error) => return Err(error),
    };
    let broker = CredentialBroker::initialize(app_data_root).await?;
    run_pending_migration_inner(&broker, None).await
}

pub async fn run_automatic_migration(
    app_data_root: impl AsRef<Path>,
) -> Result<PendingMigrationReport, BridgeError> {
    run_automatic_migration_inner(app_data_root.as_ref(), None).await
}

pub(crate) fn settings_sanitization_is_active(
    app_data_root: impl AsRef<Path>,
) -> Result<bool, BridgeError> {
    let paths = BridgePaths::new(app_data_root);
    Ok(
        read_sanitization_manifest(paths.sanitization_manifest_path())?
            .is_some_and(|manifest| manifest.stage == SanitizationStage::Active),
    )
}

#[doc(hidden)]
pub async fn run_automatic_migration_with_crash_point(
    app_data_root: impl AsRef<Path>,
    crash_point: AutomaticMigrationCrashPoint,
) -> Result<PendingMigrationReport, BridgeError> {
    run_automatic_migration_inner(app_data_root.as_ref(), Some(crash_point)).await
}

async fn run_automatic_migration_inner(
    app_data_root: &Path,
    crash_point: Option<AutomaticMigrationCrashPoint>,
) -> Result<PendingMigrationReport, BridgeError> {
    let _lock = match acquire_pending_migration_lock(app_data_root) {
        Ok(lock) => lock,
        Err(error @ BridgeError::MigrationLocked { .. }) => {
            return Ok(PendingMigrationReport::skipped_locked(error.to_string()));
        }
        Err(error) => return Err(error),
    };
    let broker = CredentialBroker::initialize(app_data_root).await?;
    let mut report = run_pending_migration_inner(&broker, crash_point).await?;
    sanitize_and_activate(app_data_root, &broker, &mut report, crash_point).await?;
    Ok(report)
}

async fn run_pending_migration_inner(
    broker: &CredentialBroker,
    crash_point: Option<AutomaticMigrationCrashPoint>,
) -> Result<PendingMigrationReport, BridgeError> {
    let app_data_root = broker.paths().app_data_root();
    let snapshot = read_legacy_plaintext(app_data_root);
    maybe_crash(
        crash_point,
        AutomaticMigrationCrashPoint::AfterLegacySettingsRead,
    )?;
    let journal = MigrationJournal::new(broker.metadata_pool());
    let mut sealed_count = 0;
    let mut authenticated_count = 0;
    let mut source_errors = snapshot.source_errors;
    let mut completed_candidates = 0;

    for mut candidate in snapshot.candidates {
        let stable_inventory_id = candidate.descriptor.stable_inventory_id.clone();
        let outcome: Result<bool, BridgeError> = async {
            let mapping = journal.reserve(&candidate.descriptor).await?;
            let (metadata, sealed) = match broker.status(mapping.handle.as_str()).await? {
                Some(metadata) => (metadata, false),
                None => {
                    let secret = std::mem::take(&mut *candidate.secret);
                    let saved = broker
                        .save(NewBridgeCredential::new(
                            mapping.handle.as_str(),
                            &candidate.descriptor.provider_or_connector_id,
                            candidate.descriptor.kind.clone(),
                            candidate.descriptor.owner_scope.clone(),
                            BridgeCredentialState::PendingMigration,
                            secret,
                        )?)
                        .await?;
                    (saved, true)
                }
            };
            let state_matches = metadata.state == BridgeCredentialState::PendingMigration
                || (metadata.state == BridgeCredentialState::Valid && mapping.authenticated);
            if metadata.provider_or_connector_id != candidate.descriptor.provider_or_connector_id
                || metadata.kind != candidate.descriptor.kind
                || metadata.owner_scope != candidate.descriptor.owner_scope
                || !state_matches
                || mapping
                    .revision
                    .is_some_and(|revision| revision != metadata.revision)
            {
                return Err(BridgeError::CorruptMetadata(format!(
                    "pending envelope does not match journal: {}",
                    candidate.descriptor.stable_inventory_id
                )));
            }
            journal
                .mark_sealed(
                    &candidate.descriptor.stable_inventory_id,
                    &metadata.handle,
                    metadata.revision,
                )
                .await?;
            let consumer = CredentialConsumer::new("migration-authenticate")?;
            let request = SecretLeaseRequest::new(
                consumer,
                &metadata.provider_or_connector_id,
                metadata.handle.as_str(),
                metadata.revision,
                Instant::now() + Duration::from_secs(5),
            )?;
            broker.issue_lease(request).await?.close();
            journal
                .mark_authenticated(
                    &candidate.descriptor.stable_inventory_id,
                    &metadata.handle,
                    metadata.revision,
                )
                .await?;
            Ok(sealed)
        }
        .await;
        match outcome {
            Ok(sealed) => {
                sealed_count += usize::from(sealed);
                authenticated_count += 1;
                completed_candidates += 1;
                if completed_candidates == 1 {
                    maybe_crash(
                        crash_point,
                        AutomaticMigrationCrashPoint::AfterFirstPendingEnvelope,
                    )?;
                }
            }
            Err(error) => source_errors.push(PendingMigrationSourceError {
                source: stable_inventory_id,
                error: error.to_string(),
            }),
        }
    }

    maybe_crash(
        crash_point,
        AutomaticMigrationCrashPoint::AfterAllPendingEnvelopes,
    )?;
    let entries = journal.list().await?;
    write_pending_manifest(broker.paths().pending_manifest_path(), &entries)?;
    Ok(PendingMigrationReport::from_outcome(
        entries,
        sealed_count,
        authenticated_count,
        snapshot.mcp_instance_ids_added,
        source_errors,
    ))
}

async fn sanitize_and_activate(
    app_data_root: &Path,
    broker: &CredentialBroker,
    report: &mut PendingMigrationReport,
    crash_point: Option<AutomaticMigrationCrashPoint>,
) -> Result<(), BridgeError> {
    let manifest_path = broker.paths().sanitization_manifest_path();
    if let Some(manifest) = read_sanitization_manifest(manifest_path)? {
        if current_settings_sha256(app_data_root)? == manifest.sanitized_settings_sha256 {
            authenticate_manifest_entries(broker, &manifest.entries).await?;
            let journal = MigrationJournal::new(broker.metadata_pool());
            report.activated_count = journal.activate_authenticated(&manifest.entries).await?;
            if manifest.stage != SanitizationStage::Active {
                write_sanitization_manifest(
                    manifest_path,
                    SanitizationStage::Active,
                    &manifest.sanitized_settings_sha256,
                    &manifest.entries,
                )?;
            }
            maybe_crash(
                crash_point,
                AutomaticMigrationCrashPoint::AfterActiveManifestUpdate,
            )?;
            report.sanitized = true;
            return Ok(());
        }
        if manifest.stage == SanitizationStage::Active {
            let prepared = prepare_sanitized_settings(app_data_root)?;
            let current_entries =
                authenticate_current_settings_candidates(broker, &prepared.candidates).await?;
            for current in current_entries {
                let still_same_inventory_revision = manifest.entries.iter().any(|active| {
                    active.stable_inventory_id == current.stable_inventory_id
                        && active.handle == current.handle
                        && active.provider_or_connector_id == current.provider_or_connector_id
                        && active.revision == current.revision
                });
                if !still_same_inventory_revision {
                    return Err(BridgeError::CorruptMetadata(format!(
                        "active sanitization manifest drift: {}",
                        current.stable_inventory_id
                    )));
                }
            }
            authenticate_manifest_entries(broker, &manifest.entries).await?;
            let mut temp = prepared.write_temp()?;
            temp.sync()?;
            write_sanitization_manifest(
                manifest_path,
                SanitizationStage::ReadyToRename,
                &prepared.sha256,
                &manifest.entries,
            )?;
            temp.persist()?;
            let journal = MigrationJournal::new(broker.metadata_pool());
            report.activated_count = journal.activate_authenticated(&manifest.entries).await?;
            write_sanitization_manifest(
                manifest_path,
                SanitizationStage::Active,
                &prepared.sha256,
                &manifest.entries,
            )?;
            report.sanitized = true;
            return Ok(());
        }
    }

    let prepared = prepare_sanitized_settings(app_data_root)?;
    let entries = authenticate_current_settings_candidates(broker, &prepared.candidates).await?;
    let mut temp = prepared.write_temp()?;
    maybe_crash(
        crash_point,
        AutomaticMigrationCrashPoint::AfterSanitizedSettingsTempWrite,
    )?;
    temp.sync()?;
    maybe_crash(
        crash_point,
        AutomaticMigrationCrashPoint::AfterSanitizedSettingsFsync,
    )?;
    write_sanitization_manifest(
        manifest_path,
        SanitizationStage::ReadyToRename,
        &prepared.sha256,
        &entries,
    )?;
    temp.persist()?;
    maybe_crash(
        crash_point,
        AutomaticMigrationCrashPoint::AfterSettingsRenameBeforeActivation,
    )?;

    authenticate_manifest_entries(broker, &entries).await?;
    let journal = MigrationJournal::new(broker.metadata_pool());
    report.activated_count = journal.activate_authenticated(&entries).await?;
    write_sanitization_manifest(
        manifest_path,
        SanitizationStage::Active,
        &prepared.sha256,
        &entries,
    )?;
    maybe_crash(
        crash_point,
        AutomaticMigrationCrashPoint::AfterActiveManifestUpdate,
    )?;
    report.sanitized = true;
    Ok(())
}

async fn authenticate_current_settings_candidates(
    broker: &CredentialBroker,
    candidates: &[legacy_plaintext::LegacyCredentialCandidate],
) -> Result<Vec<SanitizationManifestEntry>, BridgeError> {
    let journal = MigrationJournal::new(broker.metadata_pool());
    let journal_entries = journal.list().await?;
    let mut entries = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !is_task3b_inventory_id(&candidate.descriptor.stable_inventory_id) {
            return Err(BridgeError::CorruptMetadata(
                "deferred credential entered Task 3B sanitization".to_owned(),
            ));
        }
        let entry = journal_entries
            .iter()
            .find(|entry| entry.stable_inventory_id == candidate.descriptor.stable_inventory_id)
            .ok_or_else(|| BridgeError::PendingCredentialMissing {
                stable_inventory_id: candidate.descriptor.stable_inventory_id.clone(),
            })?;
        if !entry.authenticated
            || entry.provider_or_connector_id != candidate.descriptor.provider_or_connector_id
        {
            return Err(BridgeError::PendingCredentialUnreadable {
                stable_inventory_id: candidate.descriptor.stable_inventory_id.clone(),
            });
        }
        let metadata = broker.status(entry.handle.as_str()).await?.ok_or_else(|| {
            BridgeError::PendingCredentialUnreadable {
                stable_inventory_id: candidate.descriptor.stable_inventory_id.clone(),
            }
        })?;
        if metadata.provider_or_connector_id != candidate.descriptor.provider_or_connector_id
            || metadata.kind != candidate.descriptor.kind
            || metadata.owner_scope != candidate.descriptor.owner_scope
            || metadata.revision != entry.revision
            || !matches!(
                metadata.state,
                BridgeCredentialState::PendingMigration | BridgeCredentialState::Valid
            )
        {
            return Err(BridgeError::PendingCredentialUnreadable {
                stable_inventory_id: candidate.descriptor.stable_inventory_id.clone(),
            });
        }
        authenticate_and_compare(
            broker,
            entry,
            Some(candidate.secret.as_slice()),
            &candidate.descriptor.stable_inventory_id,
        )
        .await?;
        entries.push(SanitizationManifestEntry::from(entry));
    }
    entries.sort_by(|left, right| left.stable_inventory_id.cmp(&right.stable_inventory_id));
    Ok(entries)
}

async fn authenticate_manifest_entries(
    broker: &CredentialBroker,
    entries: &[SanitizationManifestEntry],
) -> Result<(), BridgeError> {
    for entry in entries {
        authenticate_and_compare(
            broker,
            &PendingMigrationEntry {
                stable_inventory_id: entry.stable_inventory_id.clone(),
                handle: entry.handle.clone(),
                provider_or_connector_id: entry.provider_or_connector_id.clone(),
                revision: entry.revision,
                authenticated: true,
            },
            None,
            &entry.stable_inventory_id,
        )
        .await?;
    }
    Ok(())
}

async fn authenticate_and_compare(
    broker: &CredentialBroker,
    entry: &PendingMigrationEntry,
    expected: Option<&[u8]>,
    stable_inventory_id: &str,
) -> Result<(), BridgeError> {
    let consumer = CredentialConsumer::new("migration-authenticate")?;
    let binding = LeaseBinding::new(
        consumer.clone(),
        &entry.provider_or_connector_id,
        entry.handle.as_str(),
        entry.revision,
    )?;
    let request = SecretLeaseRequest::new(
        consumer,
        &entry.provider_or_connector_id,
        entry.handle.as_str(),
        entry.revision,
        Instant::now() + Duration::from_secs(5),
    )?;
    let mut lease = broker.issue_lease(request).await?;
    let matches = lease.with_secret(&binding, |secret| {
        expected.is_none_or(|expected| expected == secret)
    })?;
    if !matches {
        return Err(BridgeError::CorruptMetadata(format!(
            "pending envelope no longer matches active source: {stable_inventory_id}"
        )));
    }
    Ok(())
}

fn current_settings_sha256(app_data_root: &Path) -> Result<String, BridgeError> {
    let path = app_data_root.join("settings.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => Zeroizing::new(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Zeroizing::new(b"{}\n".to_vec())
        }
        Err(source) => {
            return Err(BridgeError::Io {
                operation: "read settings for resume hash",
                path,
                source,
            });
        }
    };
    Ok(format!("{:x}", Sha256::digest(bytes.as_slice())))
}

fn maybe_crash(
    requested: Option<AutomaticMigrationCrashPoint>,
    current: AutomaticMigrationCrashPoint,
) -> Result<(), BridgeError> {
    if requested == Some(current) {
        return Err(BridgeError::SimulatedMigrationCrash {
            point: current.as_str(),
        });
    }
    Ok(())
}
