use std::fmt;
use std::fs;
use std::path::Path;

use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{FromRow, SqlitePool};

use super::platform_permissions::{create_secure_file, verify_secure_file};
use super::schema::initialize_store_schema;
use super::types::{
    BridgeCredentialMetadata, BridgeCredentialState, BridgeError, BridgeResult, CredentialHandle,
    CredentialKind, CredentialOwnerScope,
};

#[derive(Clone)]
pub(crate) struct MetadataStore {
    pub(crate) pool: SqlitePool,
}

impl fmt::Debug for MetadataStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MetadataStore(<dedicated credential database>)")
    }
}

impl MetadataStore {
    pub(crate) async fn open(database_path: &Path) -> BridgeResult<Self> {
        ensure_database_file(database_path)?;
        let options = SqliteConnectOptions::new()
            .filename(database_path)
            .create_if_missing(false)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Delete)
            .synchronous(sqlx::sqlite::SqliteSynchronous::Full);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        initialize_store_schema(&pool).await?;
        verify_secure_file(database_path)?;
        Ok(Self { pool })
    }

    pub(crate) async fn get(
        &self,
        handle: &CredentialHandle,
    ) -> BridgeResult<Option<BridgeCredentialMetadata>> {
        let row = sqlx::query_as::<_, MetadataRow>(
            "SELECT handle, provider_or_connector_id, kind, owner_scope,
                    revision, state, secret_present, created_at_ms, updated_at_ms
             FROM credential_metadata
             WHERE handle = ?",
        )
        .bind(handle.as_str())
        .fetch_optional(&self.pool)
        .await?;
        row.map(MetadataRow::try_into_metadata).transpose()
    }

    pub(crate) async fn list(&self) -> BridgeResult<Vec<BridgeCredentialMetadata>> {
        sqlx::query_as::<_, MetadataRow>(
            "SELECT handle, provider_or_connector_id, kind, owner_scope,
                    revision, state, secret_present, created_at_ms, updated_at_ms
             FROM credential_metadata
             ORDER BY handle",
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(MetadataRow::try_into_metadata)
        .collect()
    }

    pub(crate) async fn mark_unreadable(
        &self,
        handle: &CredentialHandle,
        expected_revision: i64,
    ) -> BridgeResult<()> {
        sqlx::query(
            "UPDATE credential_metadata
             SET state = 'unreadable', updated_at_ms = ?
             WHERE handle = ? AND revision = ?",
        )
        .bind(chrono::Utc::now().timestamp_millis())
        .bind(handle.as_str())
        .bind(expected_revision)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

fn ensure_database_file(path: &Path) -> BridgeResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => verify_secure_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match create_secure_file(path) {
                Ok(file) => {
                    file.sync_all().map_err(|source| BridgeError::Io {
                        operation: "sync credential database file",
                        path: path.to_path_buf(),
                        source,
                    })?;
                }
                Err(BridgeError::Io { source, .. })
                    if source.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
            verify_secure_file(path)
        }
        Err(source) => Err(BridgeError::Io {
            operation: "inspect credential database",
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[derive(Debug, FromRow)]
struct MetadataRow {
    handle: String,
    provider_or_connector_id: String,
    kind: String,
    owner_scope: String,
    revision: i64,
    state: String,
    secret_present: i64,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl MetadataRow {
    fn try_into_metadata(self) -> BridgeResult<BridgeCredentialMetadata> {
        Ok(BridgeCredentialMetadata {
            handle: CredentialHandle::new(self.handle)
                .map_err(|_| BridgeError::CorruptMetadata("invalid handle".to_owned()))?,
            provider_or_connector_id: self.provider_or_connector_id,
            kind: CredentialKind::from_storage_str(&self.kind)?,
            owner_scope: CredentialOwnerScope::from_storage_str(&self.owner_scope)?,
            revision: self.revision,
            state: BridgeCredentialState::from_storage_str(&self.state)?,
            secret_present: self.secret_present == 1,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        })
    }
}
