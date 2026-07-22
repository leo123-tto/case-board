use std::collections::BTreeMap;

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

const SOURCE_REPOSITORY: &str = "https://github.com/earendil-works/pi";
const MAX_ARCHIVE_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PiRuntimeManifest {
    pub manifest_version: u32,
    pub runtime_version: String,
    pub pi_sdk_version: String,
    pub protocol_version: u32,
    pub minimum_caseboard_version: String,
    pub source_repository: String,
    pub source_commit: String,
    #[serde(default)]
    pub released_at: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    pub artifacts: BTreeMap<String, PiRuntimeArtifact>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PiRuntimeArtifact {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub signature: String,
}

#[derive(Debug, Clone)]
pub struct SelectedManifest {
    pub manifest: PiRuntimeManifest,
    pub target: String,
    pub artifact: PiRuntimeArtifact,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestError {
    #[error("Runtime manifest 不是有效 JSON:{0}")]
    InvalidJson(String),
    #[error("不支持的 Runtime manifest 版本:{0}")]
    ManifestVersion(u32),
    #[error("Runtime 版本号无效:{0}")]
    RuntimeVersion(String),
    #[error("Pi SDK 版本号必须是稳定的精确 semver:{0}")]
    PiSdkVersion(String),
    #[error("Runtime 协议不兼容:manifest={manifest}, app={app}")]
    Protocol { manifest: u32, app: u32 },
    #[error("当前 CaseBoard 版本过低:至少 {required}, 当前 {current}")]
    MinimumCaseboard { required: String, current: String },
    #[error("Runtime 源仓库不受信任")]
    InvalidSourceRepository,
    #[error("Runtime 源 commit 必须是 40 位小写 Git SHA")]
    InvalidSourceCommit,
    #[error("Runtime manifest 缺少当前平台资产:{0}")]
    PlatformUnavailable(String),
    #[error("Runtime 资产大小必须大于 0")]
    InvalidSize,
    #[error("Runtime 资产 SHA-256 必须是 64 位小写十六进制")]
    InvalidSha256,
    #[error("Runtime 资产签名为空")]
    MissingSignature,
    #[error("Runtime 资产 URL 不在批准的 HTTPS 发布源")]
    InvalidAssetUrl,
}

pub fn current_target() -> Result<&'static str, ManifestError> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("macos-aarch64"),
        ("macos", "x86_64") => Ok("macos-x86_64"),
        ("windows", "x86_64") => Ok("windows-x86_64"),
        _ => Err(ManifestError::PlatformUnavailable(format!(
            "{}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))),
    }
}

pub fn parse_and_select(
    manifest_bytes: &[u8],
    target: &str,
    current_caseboard_version: &str,
    protocol_version: u32,
) -> Result<SelectedManifest, ManifestError> {
    let manifest: PiRuntimeManifest = serde_json::from_slice(manifest_bytes)
        .map_err(|error| ManifestError::InvalidJson(error.to_string()))?;

    if manifest.manifest_version != 1 {
        return Err(ManifestError::ManifestVersion(manifest.manifest_version));
    }

    let runtime_version = Version::parse(&manifest.runtime_version)
        .map_err(|_| ManifestError::RuntimeVersion(manifest.runtime_version.clone()))?;
    let caseboard_build = runtime_version
        .pre
        .as_str()
        .strip_prefix("caseboard.")
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()));
    if !runtime_version.build.is_empty() || !caseboard_build {
        return Err(ManifestError::RuntimeVersion(
            manifest.runtime_version.clone(),
        ));
    }

    let pi_sdk = stable_version(&manifest.pi_sdk_version)
        .map_err(|_| ManifestError::PiSdkVersion(manifest.pi_sdk_version.clone()))?;
    let required = stable_version(&manifest.minimum_caseboard_version).map_err(|_| {
        ManifestError::MinimumCaseboard {
            required: manifest.minimum_caseboard_version.clone(),
            current: current_caseboard_version.to_string(),
        }
    })?;
    let current =
        stable_version(current_caseboard_version).map_err(|_| ManifestError::MinimumCaseboard {
            required: manifest.minimum_caseboard_version.clone(),
            current: current_caseboard_version.to_string(),
        })?;
    if required > current {
        return Err(ManifestError::MinimumCaseboard {
            required: required.to_string(),
            current: current.to_string(),
        });
    }
    if manifest.protocol_version != protocol_version {
        return Err(ManifestError::Protocol {
            manifest: manifest.protocol_version,
            app: protocol_version,
        });
    }
    if manifest.source_repository != SOURCE_REPOSITORY {
        return Err(ManifestError::InvalidSourceRepository);
    }
    if !is_lower_hex(&manifest.source_commit, 40) {
        return Err(ManifestError::InvalidSourceCommit);
    }

    // The parsed value is intentionally retained: parsing alone rejects shorthand/range syntax.
    let _ = pi_sdk;
    let artifact = manifest
        .artifacts
        .get(target)
        .cloned()
        .ok_or_else(|| ManifestError::PlatformUnavailable(target.to_string()))?;
    validate_artifact(&artifact)?;

    Ok(SelectedManifest {
        manifest,
        target: target.to_string(),
        artifact,
    })
}

fn stable_version(value: &str) -> Result<Version, semver::Error> {
    let version = Version::parse(value)?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Version::parse("not-a-stable-version");
    }
    Ok(version)
}

fn validate_artifact(artifact: &PiRuntimeArtifact) -> Result<(), ManifestError> {
    if artifact.size == 0 || artifact.size > MAX_ARCHIVE_SIZE {
        return Err(ManifestError::InvalidSize);
    }
    if !is_lower_hex(&artifact.sha256, 64) {
        return Err(ManifestError::InvalidSha256);
    }
    if artifact.signature.trim().is_empty() {
        return Err(ManifestError::MissingSignature);
    }
    if !approved_asset_url(&artifact.url) {
        return Err(ManifestError::InvalidAssetUrl);
    }
    Ok(())
}

fn approved_asset_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    if url.scheme() != "https" || url.query().is_some() || url.fragment().is_some() {
        return false;
    }
    match url.host_str() {
        Some("lawtools.top" | "www.lawtools.top") => {
            url.path().starts_with("/caseboard/pi-runtime/")
        }
        Some("github.com") => {
            url.path()
                .starts_with("/leo123-tto/caseboard/releases/download/")
                && url.path().ends_with(".zip")
        }
        _ => false,
    }
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
