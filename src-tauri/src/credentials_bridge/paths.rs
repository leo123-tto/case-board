use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BridgePaths {
    app_data_root: PathBuf,
    bridge_root: PathBuf,
    database_path: PathBuf,
    master_key_path: PathBuf,
    manifest_path: PathBuf,
    pending_manifest_path: PathBuf,
    sanitization_manifest_path: PathBuf,
    legacy_system_import_manifest_path: PathBuf,
    pending_migration_lock_path: PathBuf,
}

impl BridgePaths {
    pub fn new(app_data_root: impl AsRef<Path>) -> Self {
        let app_data_root = app_data_root.as_ref().to_path_buf();
        let bridge_root = app_data_root.join("credential-bridge").join("v1");
        Self {
            database_path: bridge_root.join("credential-store.sqlite"),
            master_key_path: bridge_root.join("master-key.v1"),
            manifest_path: bridge_root.join("manifest.json"),
            pending_manifest_path: bridge_root.join("pending-migration-manifest.json"),
            sanitization_manifest_path: bridge_root.join("sanitization-manifest.json"),
            legacy_system_import_manifest_path: bridge_root
                .join("legacy-system-import-manifest.json"),
            pending_migration_lock_path: bridge_root.join("pending-migration.lock"),
            app_data_root,
            bridge_root,
        }
    }

    pub fn app_data_root(&self) -> &Path {
        &self.app_data_root
    }

    pub fn bridge_root(&self) -> &Path {
        &self.bridge_root
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn master_key_path(&self) -> &Path {
        &self.master_key_path
    }

    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    pub fn pending_manifest_path(&self) -> &Path {
        &self.pending_manifest_path
    }

    pub fn sanitization_manifest_path(&self) -> &Path {
        &self.sanitization_manifest_path
    }

    pub fn legacy_system_import_manifest_path(&self) -> &Path {
        &self.legacy_system_import_manifest_path
    }

    pub fn pending_migration_lock_path(&self) -> &Path {
        &self.pending_migration_lock_path
    }
}
