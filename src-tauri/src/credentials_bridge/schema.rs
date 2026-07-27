use std::fs;
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use super::platform_permissions::{create_secure_file, verify_secure_file};
use super::types::{BridgeError, BridgeResult, BRIDGE_SCHEMA};

pub(crate) const STORE_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct BridgeManifest {
    schema: String,
    version: i64,
    database: String,
    master_key: String,
    algorithm: String,
}

impl BridgeManifest {
    fn expected() -> Self {
        Self {
            schema: BRIDGE_SCHEMA.to_owned(),
            version: STORE_SCHEMA_VERSION,
            database: "credential-store.sqlite".to_owned(),
            master_key: "master-key.v1".to_owned(),
            algorithm: "XChaCha20-Poly1305".to_owned(),
        }
    }
}

pub(crate) fn ensure_manifest(path: &Path) -> BridgeResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => verify_manifest(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_manifest(path),
        Err(source) => Err(BridgeError::Io {
            operation: "inspect credential bridge manifest",
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn create_manifest(path: &Path) -> BridgeResult<()> {
    let encoded = serde_json::to_vec_pretty(&BridgeManifest::expected())
        .map_err(|error| BridgeError::InvalidManifest(error.to_string()))?;
    let mut file = match create_secure_file(path) {
        Ok(file) => file,
        Err(BridgeError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            return verify_manifest(path);
        }
        Err(error) => return Err(error),
    };
    let write_result = file
        .write_all(&encoded)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all());
    if let Err(source) = write_result {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(BridgeError::Io {
            operation: "persist credential bridge manifest",
            path: path.to_path_buf(),
            source,
        });
    }
    drop(file);
    verify_manifest(path)
}

fn verify_manifest(path: &Path) -> BridgeResult<()> {
    verify_secure_file(path)?;
    let bytes = fs::read(path).map_err(|source| BridgeError::Io {
        operation: "read credential bridge manifest",
        path: path.to_path_buf(),
        source,
    })?;
    let actual: BridgeManifest = serde_json::from_slice(&bytes)
        .map_err(|error| BridgeError::InvalidManifest(error.to_string()))?;
    if actual != BridgeManifest::expected() {
        return Err(BridgeError::InvalidManifest(
            "manifest contract mismatch".to_owned(),
        ));
    }
    Ok(())
}

pub(crate) async fn initialize_store_schema(pool: &SqlitePool) -> BridgeResult<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS credential_store_schema (
            version INTEGER PRIMARY KEY CHECK (version = 1),
            migrated_at_ms INTEGER NOT NULL
        ) STRICT",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS credential_envelopes (
            handle TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK (revision >= 1),
            version INTEGER NOT NULL CHECK (version = 1),
            algorithm TEXT NOT NULL CHECK (algorithm = 'xchacha20poly1305'),
            nonce BLOB NOT NULL CHECK (length(nonce) = 24),
            ciphertext BLOB NOT NULL CHECK (length(ciphertext) >= 16),
            created_at_ms INTEGER NOT NULL,
            PRIMARY KEY (handle, revision)
        ) STRICT",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS credential_metadata (
            handle TEXT PRIMARY KEY,
            provider_or_connector_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            owner_scope TEXT NOT NULL,
            revision INTEGER NOT NULL CHECK (revision >= 1),
            state TEXT NOT NULL,
            secret_present INTEGER NOT NULL CHECK (secret_present = 1),
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            FOREIGN KEY (handle, revision)
              REFERENCES credential_envelopes(handle, revision)
              ON UPDATE RESTRICT ON DELETE RESTRICT
        ) STRICT",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pending_migration_journal (
            stable_inventory_id TEXT PRIMARY KEY,
            handle TEXT NOT NULL UNIQUE,
            provider_or_connector_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            owner_scope TEXT NOT NULL,
            revision INTEGER CHECK (revision IS NULL OR revision >= 1),
            authenticated INTEGER NOT NULL DEFAULT 0 CHECK (authenticated IN (0, 1)),
            created_at_ms INTEGER NOT NULL,
            updated_at_ms INTEGER NOT NULL,
            FOREIGN KEY (handle, revision)
              REFERENCES credential_envelopes(handle, revision)
              ON UPDATE RESTRICT ON DELETE RESTRICT
        ) STRICT",
    )
    .execute(pool)
    .await?;

    let now = chrono::Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO credential_store_schema(version, migrated_at_ms)
         VALUES(1, ?)
         ON CONFLICT(version) DO NOTHING",
    )
    .bind(now)
    .execute(pool)
    .await?;

    let versions: Vec<i64> =
        sqlx::query_scalar("SELECT version FROM credential_store_schema ORDER BY version")
            .fetch_all(pool)
            .await?;
    if versions != [STORE_SCHEMA_VERSION] {
        return Err(BridgeError::CorruptMetadata(format!(
            "unsupported credential store schema versions: {versions:?}"
        )));
    }
    Ok(())
}
