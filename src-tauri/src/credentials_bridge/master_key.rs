use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::OsRng;
use zeroize::Zeroizing;

use super::platform_permissions::{create_secure_file, verify_secure_file};
use super::types::{BridgeError, BridgeResult};

pub(crate) const MASTER_KEY_BYTES: usize = 32;

pub(crate) struct MasterKey(Zeroizing<[u8; MASTER_KEY_BYTES]>);

impl MasterKey {
    pub(crate) fn as_bytes(&self) -> &[u8; MASTER_KEY_BYTES] {
        &self.0
    }
}

pub(crate) fn ensure_master_key(path: &Path) -> BridgeResult<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => verify_master_key_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => create_master_key(path),
        Err(source) => Err(BridgeError::Io {
            operation: "inspect master key",
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub(crate) fn load_master_key(path: &Path) -> BridgeResult<MasterKey> {
    verify_master_key_file(path)?;
    let mut file = fs::File::open(path).map_err(|source| BridgeError::Io {
        operation: "read master key",
        path: path.to_path_buf(),
        source,
    })?;
    let mut key = Zeroizing::new([0u8; MASTER_KEY_BYTES]);
    file.read_exact(key.as_mut())
        .map_err(|source| BridgeError::Io {
            operation: "read master key",
            path: path.to_path_buf(),
            source,
        })?;
    Ok(MasterKey(key))
}

fn verify_master_key_file(path: &Path) -> BridgeResult<()> {
    verify_secure_file(path)?;
    let length = fs::metadata(path)
        .map_err(|source| BridgeError::Io {
            operation: "inspect master key length",
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if length != MASTER_KEY_BYTES as u64 {
        return Err(BridgeError::InvalidMasterKey);
    }
    Ok(())
}

fn create_master_key(path: &Path) -> BridgeResult<()> {
    let mut generated = Zeroizing::new([0u8; MASTER_KEY_BYTES]);
    OsRng.fill_bytes(generated.as_mut());
    let mut file = match create_secure_file(path) {
        Ok(file) => file,
        Err(BridgeError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            return verify_master_key_file(path);
        }
        Err(error) => return Err(error),
    };

    let write_result = file
        .write_all(generated.as_ref())
        .and_then(|_| file.sync_all());
    if let Err(source) = write_result {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(BridgeError::Io {
            operation: "persist master key",
            path: path.to_path_buf(),
            source,
        });
    }
    drop(file);
    verify_master_key_file(path)
}
