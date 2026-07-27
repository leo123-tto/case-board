use sqlx::{FromRow, SqlitePool};

use super::manifest::SanitizationManifestEntry;
use super::types::{
    BridgeError, BridgeResult, CredentialHandle, CredentialKind, CredentialOwnerScope,
    PendingMigrationEntry,
};

#[derive(Clone, Debug)]
pub(crate) struct PendingCredentialDescriptor {
    pub stable_inventory_id: String,
    pub provider_or_connector_id: String,
    pub kind: CredentialKind,
    pub owner_scope: CredentialOwnerScope,
}

#[derive(Clone, Debug)]
pub(crate) struct JournalEntry {
    pub descriptor: PendingCredentialDescriptor,
    pub handle: CredentialHandle,
    pub revision: Option<i64>,
    pub authenticated: bool,
}

pub(crate) struct MigrationJournal<'a> {
    pool: &'a SqlitePool,
}

impl<'a> MigrationJournal<'a> {
    pub(crate) fn new(pool: &'a SqlitePool) -> Self {
        Self { pool }
    }

    pub(crate) async fn reserve(
        &self,
        descriptor: &PendingCredentialDescriptor,
    ) -> BridgeResult<JournalEntry> {
        validate_stable_inventory_id(&descriptor.stable_inventory_id)?;
        let now = chrono::Utc::now().timestamp_millis();
        let handle = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO pending_migration_journal(
                stable_inventory_id, handle, provider_or_connector_id, kind, owner_scope,
                revision, authenticated, created_at_ms, updated_at_ms
             ) VALUES(?, ?, ?, ?, ?, NULL, 0, ?, ?)
             ON CONFLICT(stable_inventory_id) DO NOTHING",
        )
        .bind(&descriptor.stable_inventory_id)
        .bind(&handle)
        .bind(&descriptor.provider_or_connector_id)
        .bind(descriptor.kind.as_storage_str())
        .bind(descriptor.owner_scope.to_storage_string())
        .bind(now)
        .bind(now)
        .execute(self.pool)
        .await?;

        let entry = self
            .get(&descriptor.stable_inventory_id)
            .await?
            .ok_or_else(|| {
                BridgeError::CorruptMetadata("pending migration mapping disappeared".to_owned())
            })?;
        if entry.descriptor.provider_or_connector_id != descriptor.provider_or_connector_id
            || entry.descriptor.kind != descriptor.kind
            || entry.descriptor.owner_scope != descriptor.owner_scope
        {
            return Err(BridgeError::CorruptMetadata(format!(
                "pending migration descriptor drift: {}",
                descriptor.stable_inventory_id
            )));
        }
        Ok(entry)
    }

    pub(crate) async fn get(
        &self,
        stable_inventory_id: &str,
    ) -> BridgeResult<Option<JournalEntry>> {
        let row = sqlx::query_as::<_, JournalRow>(
            "SELECT stable_inventory_id, handle, provider_or_connector_id, kind, owner_scope,
                    revision, authenticated
             FROM pending_migration_journal
             WHERE stable_inventory_id = ?",
        )
        .bind(stable_inventory_id)
        .fetch_optional(self.pool)
        .await?;
        row.map(JournalRow::try_into_entry).transpose()
    }

    pub(crate) async fn mark_sealed(
        &self,
        stable_inventory_id: &str,
        handle: &CredentialHandle,
        revision: i64,
    ) -> BridgeResult<()> {
        let result = sqlx::query(
            "UPDATE pending_migration_journal
             SET revision = ?, updated_at_ms = ?
             WHERE stable_inventory_id = ? AND handle = ?
               AND (revision IS NULL OR revision = ?)",
        )
        .bind(revision)
        .bind(chrono::Utc::now().timestamp_millis())
        .bind(stable_inventory_id)
        .bind(handle.as_str())
        .bind(revision)
        .execute(self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(BridgeError::CorruptMetadata(format!(
                "pending migration revision drift: {stable_inventory_id}"
            )));
        }
        Ok(())
    }

    pub(crate) async fn mark_authenticated(
        &self,
        stable_inventory_id: &str,
        handle: &CredentialHandle,
        revision: i64,
    ) -> BridgeResult<()> {
        let result = sqlx::query(
            "UPDATE pending_migration_journal
             SET authenticated = 1, updated_at_ms = ?
             WHERE stable_inventory_id = ? AND handle = ? AND revision = ?",
        )
        .bind(chrono::Utc::now().timestamp_millis())
        .bind(stable_inventory_id)
        .bind(handle.as_str())
        .bind(revision)
        .execute(self.pool)
        .await?;
        if result.rows_affected() != 1 {
            return Err(BridgeError::CorruptMetadata(format!(
                "pending migration authentication drift: {stable_inventory_id}"
            )));
        }
        Ok(())
    }

    pub(crate) async fn list(&self) -> BridgeResult<Vec<PendingMigrationEntry>> {
        let rows = sqlx::query_as::<_, JournalRow>(
            "SELECT stable_inventory_id, handle, provider_or_connector_id, kind, owner_scope,
                    revision, authenticated
             FROM pending_migration_journal
             WHERE revision IS NOT NULL
             ORDER BY stable_inventory_id",
        )
        .fetch_all(self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let entry = row.try_into_entry()?;
                Ok(PendingMigrationEntry {
                    stable_inventory_id: entry.descriptor.stable_inventory_id,
                    handle: entry.handle,
                    provider_or_connector_id: entry.descriptor.provider_or_connector_id,
                    revision: entry.revision.ok_or_else(|| {
                        BridgeError::CorruptMetadata(
                            "sealed pending mapping lacks revision".to_owned(),
                        )
                    })?,
                    authenticated: entry.authenticated,
                })
            })
            .collect()
    }

    pub(crate) async fn activate_authenticated(
        &self,
        entries: &[SanitizationManifestEntry],
    ) -> BridgeResult<usize> {
        let mut transaction = self.pool.begin().await?;
        for entry in entries {
            let journal = sqlx::query_as::<_, (String, String, Option<i64>, i64)>(
                "SELECT handle, provider_or_connector_id, revision, authenticated
                 FROM pending_migration_journal
                 WHERE stable_inventory_id = ?",
            )
            .bind(&entry.stable_inventory_id)
            .fetch_optional(&mut *transaction)
            .await?
            .ok_or_else(|| {
                BridgeError::CorruptMetadata(format!(
                    "sanitization journal mapping missing: {}",
                    entry.stable_inventory_id
                ))
            })?;
            if journal.0 != entry.handle.as_str()
                || journal.1 != entry.provider_or_connector_id
                || journal.2 != Some(entry.revision)
                || journal.3 != 1
            {
                return Err(BridgeError::CorruptMetadata(format!(
                    "sanitization journal mapping drift: {}",
                    entry.stable_inventory_id
                )));
            }
            let updated = sqlx::query(
                "UPDATE credential_metadata
                 SET state = 'valid', updated_at_ms = ?
                 WHERE handle = ? AND revision = ? AND secret_present = 1
                   AND state IN ('pending_migration', 'valid')",
            )
            .bind(chrono::Utc::now().timestamp_millis())
            .bind(entry.handle.as_str())
            .bind(entry.revision)
            .execute(&mut *transaction)
            .await?;
            if updated.rows_affected() != 1 {
                return Err(BridgeError::CorruptMetadata(format!(
                    "sanitization activation drift: {}",
                    entry.stable_inventory_id
                )));
            }
        }
        transaction.commit().await?;
        Ok(entries.len())
    }
}

fn validate_stable_inventory_id(value: &str) -> BridgeResult<()> {
    if value.is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(BridgeError::InvalidInput("stable_inventory_id"));
    }
    Ok(())
}

#[derive(Debug, FromRow)]
struct JournalRow {
    stable_inventory_id: String,
    handle: String,
    provider_or_connector_id: String,
    kind: String,
    owner_scope: String,
    revision: Option<i64>,
    authenticated: i64,
}

impl JournalRow {
    fn try_into_entry(self) -> BridgeResult<JournalEntry> {
        Ok(JournalEntry {
            descriptor: PendingCredentialDescriptor {
                stable_inventory_id: self.stable_inventory_id,
                provider_or_connector_id: self.provider_or_connector_id,
                kind: CredentialKind::from_storage_str(&self.kind)?,
                owner_scope: CredentialOwnerScope::from_storage_str(&self.owner_scope)?,
            },
            handle: CredentialHandle::new(self.handle)
                .map_err(|_| BridgeError::CorruptMetadata("invalid journal handle".to_owned()))?,
            revision: self.revision,
            authenticated: self.authenticated == 1,
        })
    }
}
