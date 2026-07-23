use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};
use thiserror::Error;

const CASEBOARD_MINISIGN_PUBLIC_KEY: &str =
    "RWQl2PkYtaOpWs0Jr4SdN+BlXVs8+IYLo1oNxyNh4Nn5O2uW87Plq8Hx";
const MAX_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum VerifyError {
    #[error("Runtime 资产大小不符:期望 {expected}, 实际 {actual}")]
    Size { expected: u64, actual: u64 },
    #[error("Runtime 资产 SHA-256 不匹配")]
    Sha256,
    #[error("Runtime Minisign 签名无效:{0}")]
    Signature(String),
    #[error("Runtime ZIP 无效:{0}")]
    Zip(String),
    #[error("Runtime ZIP 包含不允许的路径或文件:{0}")]
    UnsafeArchive(String),
    #[error("Runtime 文件写入失败:{0}")]
    Io(#[from] std::io::Error),
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn verify_size_and_sha256(
    bytes: &[u8],
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), VerifyError> {
    let actual_size = bytes.len() as u64;
    if actual_size != expected_size {
        return Err(VerifyError::Size {
            expected: expected_size,
            actual: actual_size,
        });
    }
    if sha256_hex(bytes) != expected_sha256 {
        return Err(VerifyError::Sha256);
    }
    Ok(())
}

pub fn verify_manifest_signature(
    manifest_bytes: &[u8],
    signature_text: &str,
) -> Result<(), VerifyError> {
    verify_minisign_with_key(
        manifest_bytes,
        signature_text,
        CASEBOARD_MINISIGN_PUBLIC_KEY,
    )
}

/// The asset signature covers the lowercase SHA-256 text from the signed manifest.
pub fn verify_asset_signature(sha256: &str, signature_text: &str) -> Result<(), VerifyError> {
    verify_minisign_with_key(
        sha256.as_bytes(),
        signature_text,
        CASEBOARD_MINISIGN_PUBLIC_KEY,
    )
}

fn verify_minisign_with_key(
    bytes: &[u8],
    signature_text: &str,
    public_key_base64: &str,
) -> Result<(), VerifyError> {
    let public_key = PublicKey::from_base64(public_key_base64)
        .map_err(|error| VerifyError::Signature(error.to_string()))?;
    let signature = Signature::decode(signature_text)
        .map_err(|error| VerifyError::Signature(error.to_string()))?;
    public_key
        .verify(bytes, &signature, false)
        .map_err(|error| VerifyError::Signature(error.to_string()))
}

pub fn extract_runtime_archive(
    archive_bytes: &[u8],
    destination: &Path,
) -> Result<PathBuf, VerifyError> {
    fs::create_dir_all(destination)?;
    let cursor = Cursor::new(archive_bytes);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|error| VerifyError::Zip(error.to_string()))?;
    if archive.is_empty() || archive.len() > 3 {
        return Err(VerifyError::UnsafeArchive(format!(
            "文件数量 {} 超出预期",
            archive.len()
        )));
    }

    let binary_name = runtime_binary_name();
    let allowed = [
        binary_name,
        "runtime-metadata.json",
        "THIRD_PARTY_NOTICES.txt",
    ];
    let mut seen = HashSet::new();
    let mut total_size = 0_u64;

    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| VerifyError::Zip(error.to_string()))?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| VerifyError::UnsafeArchive(entry.name().to_string()))?
            .to_path_buf();
        let name = enclosed
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| VerifyError::UnsafeArchive(entry.name().to_string()))?;
        if enclosed.components().count() != 1
            || !allowed.contains(&name)
            || entry.is_dir()
            || !seen.insert(name.to_string())
        {
            return Err(VerifyError::UnsafeArchive(entry.name().to_string()));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(VerifyError::UnsafeArchive(format!("{name}(symlink)")));
        }

        total_size = total_size.saturating_add(entry.size());
        if total_size > MAX_UNCOMPRESSED_BYTES {
            return Err(VerifyError::UnsafeArchive("解压后体积超限".into()));
        }

        let output = destination.join(name);
        let mut file = fs::File::create(&output)?;
        let expected_entry_size = entry.size();
        let copied = std::io::copy(&mut entry.take(MAX_UNCOMPRESSED_BYTES + 1), &mut file)?;
        if copied != expected_entry_size {
            return Err(VerifyError::UnsafeArchive(format!("{name} 解压长度不一致")));
        }
        file.sync_all()?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = if name == binary_name { 0o700 } else { 0o600 };
            fs::set_permissions(&output, fs::Permissions::from_mode(mode))?;
        }
    }

    for required in allowed {
        if !seen.contains(required) {
            return Err(VerifyError::UnsafeArchive(format!("缺少 {required}")));
        }
    }
    Ok(destination.join(binary_name))
}

fn runtime_binary_name() -> &'static str {
    if cfg!(windows) {
        "caseboard-pi-runtime.exe"
    } else {
        "caseboard-pi-runtime"
    }
}
