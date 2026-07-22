mod manifest;
mod store;
mod verify;

use std::fs;
use std::path::Path;
use std::time::Duration;

use futures::StreamExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use thiserror::Error;

use manifest::{current_target, parse_and_select, SelectedManifest};
use store::RuntimeStore;

const MANIFEST_URL: &str = "https://lawtools.top/caseboard/pi-runtime/manifest.json";
const MANIFEST_SIGNATURE_URL: &str =
    "https://lawtools.top/caseboard/pi-runtime/manifest.json.minisig";
const BUNDLED_RUNTIME_VERSION: &str = "0.80.10-caseboard.4";
const MANIFEST_MAX_BYTES: u64 = 512 * 1024;
const SIGNATURE_MAX_BYTES: u64 = 32 * 1024;
const ARCHIVE_MAX_BYTES: u64 = 256 * 1024 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize)]
pub struct PiRuntimeUpdateInfo {
    pub state: &'static str,
    pub published: bool,
    pub current_version: Option<String>,
    pub bundled_version: String,
    pub latest_version: Option<String>,
    pub pi_sdk_version: Option<String>,
    pub has_update: bool,
    pub asset_size: Option<u64>,
    pub released_at: Option<String>,
    pub notes: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PiRuntimeInstallResult {
    pub state: &'static str,
    pub version: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
struct PiRuntimeUpdateProgress {
    stage: &'static str,
    message: &'static str,
}

#[derive(Debug, Error)]
enum UpdateError {
    #[error("独立 Pi Runtime 更新尚未发布")]
    NotPublished,
    #[error("Runtime 更新网络请求失败:{0}")]
    Network(String),
    #[error("Runtime 更新响应过大")]
    ResponseTooLarge,
    #[error("Runtime manifest 校验失败:{0}")]
    Manifest(String),
    #[error("Runtime 资产校验失败:{0}")]
    Verify(String),
    #[error("Runtime 安装失败:{0}")]
    Install(String),
    #[error("Runtime 健康检查失败:{0}")]
    Health(String),
    #[error("Runtime 版本已变化，请重新检查更新")]
    VersionChanged,
    #[error("当前 Runtime 已是最新版本")]
    AlreadyCurrent,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuntimeMetadata {
    runtime_version: String,
    pi_sdk_version: String,
    protocol_version: u32,
    source_commit: String,
    target: String,
}

pub async fn check_pi_runtime_update() -> PiRuntimeUpdateInfo {
    let store = match runtime_store() {
        Ok(store) => store,
        Err(error) => return failed_update_info(None, error.to_string()),
    };
    let current = store
        .selection()
        .ok()
        .and_then(|selection| selection.current);
    match fetch_signed_manifest().await {
        Ok(selected) => update_info_from_manifest(current, selected),
        Err(UpdateError::NotPublished) => PiRuntimeUpdateInfo {
            state: "not_published",
            published: false,
            current_version: current,
            bundled_version: BUNDLED_RUNTIME_VERSION.into(),
            latest_version: None,
            pi_sdk_version: None,
            has_update: false,
            asset_size: None,
            released_at: None,
            notes: None,
            message: Some("独立更新源暂未发布；当前仍使用安装包内置版本".into()),
        },
        Err(error) => failed_update_info(current, error.to_string()),
    }
}

pub async fn install_pi_runtime_update(
    app: &tauri::AppHandle,
    expected_version: &str,
) -> Result<PiRuntimeInstallResult, String> {
    install_inner(app, expected_version)
        .await
        .map_err(sanitized_error)
}

pub fn rollback_pi_runtime() -> Result<PiRuntimeInstallResult, String> {
    let store = runtime_store().map_err(sanitized_error)?;
    store.rollback_to_bundled().map_err(sanitized_error)?;
    Ok(PiRuntimeInstallResult {
        state: "rolled_back",
        version: Some(BUNDLED_RUNTIME_VERSION.into()),
        message: "已回退到安装包内置 Pi Runtime；已下载版本仍保留，可在后续版本中重新启用".into(),
    })
}

pub(crate) fn record_runtime_handshake_failure(version: &str) {
    if let Ok(store) = runtime_store() {
        let _ = store.record_failure(version);
    }
}

pub(crate) fn record_runtime_health_success(version: &str) {
    if let Ok(store) = runtime_store() {
        let _ = store.record_success(version);
    }
}

async fn install_inner(
    app: &tauri::AppHandle,
    expected_version: &str,
) -> Result<PiRuntimeInstallResult, UpdateError> {
    emit_progress(app, "manifest", "正在获取并验证签名 manifest");
    let selected = fetch_signed_manifest().await?;
    if selected.manifest.runtime_version != expected_version {
        return Err(UpdateError::VersionChanged);
    }
    let store = runtime_store().map_err(|error| UpdateError::Install(error.to_string()))?;
    let current = store
        .selection()
        .ok()
        .and_then(|selection| selection.current);
    if !is_newer_than_current(&selected.manifest.runtime_version, current.as_deref()) {
        return Err(UpdateError::AlreadyCurrent);
    }

    // The asset signature signs the digest text inside the already signed manifest.
    verify::verify_asset_signature(&selected.artifact.sha256, &selected.artifact.signature)
        .map_err(|error| UpdateError::Verify(error.to_string()))?;
    emit_progress(app, "download", "正在下载当前平台 Runtime 资产");
    let client = http_client()?;
    let archive = fetch_limited(&client, &selected.artifact.url, ARCHIVE_MAX_BYTES)
        .await?
        .ok_or(UpdateError::NotPublished)?;
    verify::verify_size_and_sha256(&archive, selected.artifact.size, &selected.artifact.sha256)
        .map_err(|error| UpdateError::Verify(error.to_string()))?;
    emit_progress(app, "verify", "签名与 SHA-256 已通过，正在安全解压");

    fs::create_dir_all(store.root()).map_err(|error| UpdateError::Install(error.to_string()))?;
    let staging = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(store.root())
        .map_err(|error| UpdateError::Install(error.to_string()))?;
    let unpacked = staging.path().join("unpacked");
    let binary = verify::extract_runtime_archive(&archive, &unpacked)
        .map_err(|error| UpdateError::Verify(error.to_string()))?;
    verify_runtime_metadata(&unpacked, &selected)?;
    verify_platform_code_signature(&binary).await?;
    emit_progress(app, "health", "平台签名已通过，正在执行协议健康检查");
    let health = crate::chat::runtime::pi_sidecar::check_pi_runtime_health(&binary)
        .await
        .map_err(|error| UpdateError::Health(error.to_string()))?;
    if health.protocol_version != selected.manifest.protocol_version
        || health.pi_sdk_version != selected.manifest.pi_sdk_version
        || health.sidecar_version != selected.manifest.runtime_version
    {
        return Err(UpdateError::Health(
            "Sidecar 上报版本与签名 manifest 不一致".into(),
        ));
    }

    let final_dir = store
        .version_dir(&selected.manifest.runtime_version)
        .map_err(|error| UpdateError::Install(error.to_string()))?;
    if final_dir.exists() {
        return Err(UpdateError::Install(format!(
            "版本目录已存在:{}",
            selected.manifest.runtime_version
        )));
    }
    if let Some(parent) = final_dir.parent() {
        fs::create_dir_all(parent).map_err(|error| UpdateError::Install(error.to_string()))?;
    }
    emit_progress(
        app,
        "activate",
        "健康检查已通过，正在原子切换下一轮 Runtime",
    );
    fs::rename(&unpacked, &final_dir).map_err(|error| UpdateError::Install(error.to_string()))?;
    store
        .activate(&selected.manifest.runtime_version)
        .map_err(|error| UpdateError::Install(error.to_string()))?;
    store
        .record_success(&selected.manifest.runtime_version)
        .map_err(|error| UpdateError::Install(error.to_string()))?;
    emit_progress(app, "complete", "Pi Runtime 更新完成");

    Ok(PiRuntimeInstallResult {
        state: "installed",
        version: Some(selected.manifest.runtime_version.clone()),
        message: "下载、签名、哈希、平台签名和健康检查均已通过；新会话将使用新版 Runtime".into(),
    })
}

fn emit_progress(app: &tauri::AppHandle, stage: &'static str, message: &'static str) {
    let _ = app.emit(
        "pi-runtime-update-progress",
        PiRuntimeUpdateProgress { stage, message },
    );
}

fn runtime_store() -> Result<RuntimeStore, UpdateError> {
    let app_data =
        crate::db::app_data_dir().map_err(|error| UpdateError::Install(error.to_string()))?;
    Ok(RuntimeStore::new(app_data.join("runtimes/pi")))
}

async fn fetch_signed_manifest() -> Result<SelectedManifest, UpdateError> {
    let client = http_client()?;
    let manifest_bytes = fetch_limited(&client, MANIFEST_URL, MANIFEST_MAX_BYTES)
        .await?
        .ok_or(UpdateError::NotPublished)?;
    let signature_bytes = fetch_limited(&client, MANIFEST_SIGNATURE_URL, SIGNATURE_MAX_BYTES)
        .await?
        .ok_or(UpdateError::NotPublished)?;
    let signature = std::str::from_utf8(&signature_bytes)
        .map_err(|_| UpdateError::Manifest("manifest 签名不是 UTF-8".into()))?;
    verify::verify_manifest_signature(&manifest_bytes, signature)
        .map_err(|error| UpdateError::Manifest(error.to_string()))?;
    let target = current_target().map_err(|error| UpdateError::Manifest(error.to_string()))?;
    parse_and_select(
        &manifest_bytes,
        target,
        env!("CARGO_PKG_VERSION"),
        crate::chat::runtime::pi_protocol::PI_PROTOCOL_VERSION,
    )
    .map_err(|error| UpdateError::Manifest(error.to_string()))
}

fn http_client() -> Result<reqwest::Client, UpdateError> {
    reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let approved = attempt.url().scheme() == "https"
                && matches!(
                    attempt.url().host_str(),
                    Some(
                        "lawtools.top"
                            | "www.lawtools.top"
                            | "github.com"
                            | "objects.githubusercontent.com"
                            | "release-assets.githubusercontent.com"
                    )
                );
            if approved && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .user_agent(concat!(
            "CaseBoard/",
            env!("CARGO_PKG_VERSION"),
            " pi-runtime-updater"
        ))
        .build()
        .map_err(|error| UpdateError::Network(error.to_string()))
}

async fn fetch_limited(
    client: &reqwest::Client,
    url: &str,
    max_bytes: u64,
) -> Result<Option<Vec<u8>>, UpdateError> {
    let response = client
        .get(url)
        .header("Accept", "application/octet-stream")
        .send()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(UpdateError::Network(format!(
            "更新源返回 HTTP {}",
            response.status().as_u16()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(UpdateError::ResponseTooLarge);
    }

    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| UpdateError::Network(error.to_string()))?;
        if bytes.len().saturating_add(chunk.len()) as u64 > max_bytes {
            return Err(UpdateError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(bytes))
}

fn update_info_from_manifest(
    current: Option<String>,
    selected: SelectedManifest,
) -> PiRuntimeUpdateInfo {
    let has_update = is_newer_than_current(&selected.manifest.runtime_version, current.as_deref());
    PiRuntimeUpdateInfo {
        state: if has_update {
            "update_available"
        } else {
            "up_to_date"
        },
        published: true,
        current_version: current,
        bundled_version: BUNDLED_RUNTIME_VERSION.into(),
        latest_version: Some(selected.manifest.runtime_version),
        pi_sdk_version: Some(selected.manifest.pi_sdk_version),
        has_update,
        asset_size: Some(selected.artifact.size),
        released_at: selected.manifest.released_at,
        notes: selected.manifest.notes,
        message: None,
    }
}

fn is_newer_than_current(latest: &str, current: Option<&str>) -> bool {
    let baseline = current.unwrap_or(BUNDLED_RUNTIME_VERSION);
    matches!(
        (Version::parse(latest), Version::parse(baseline)),
        (Ok(latest), Ok(current)) if latest > current
    )
}

fn verify_runtime_metadata(
    unpacked: &Path,
    selected: &SelectedManifest,
) -> Result<(), UpdateError> {
    let metadata_path = unpacked.join("runtime-metadata.json");
    let metadata_bytes = fs::read(&metadata_path)
        .map_err(|_| UpdateError::Verify("Runtime 包缺少 runtime-metadata.json".into()))?;
    if metadata_bytes.len() > 64 * 1024 {
        return Err(UpdateError::Verify("Runtime metadata 体积异常".into()));
    }
    let metadata: RuntimeMetadata = serde_json::from_slice(&metadata_bytes)
        .map_err(|error| UpdateError::Verify(format!("Runtime metadata 无效:{error}")))?;
    if metadata.runtime_version != selected.manifest.runtime_version
        || metadata.pi_sdk_version != selected.manifest.pi_sdk_version
        || metadata.protocol_version != selected.manifest.protocol_version
        || metadata.source_commit != selected.manifest.source_commit
        || metadata.target != selected.target
    {
        return Err(UpdateError::Verify(
            "Runtime metadata 与签名 manifest 不一致".into(),
        ));
    }
    let notices = fs::read(unpacked.join("THIRD_PARTY_NOTICES.txt"))
        .map_err(|_| UpdateError::Verify("Runtime 包缺少第三方许可证声明".into()))?;
    if notices.is_empty() || notices.len() > 4 * 1024 * 1024 {
        return Err(UpdateError::Verify("第三方许可证声明无效".into()));
    }
    Ok(())
}

async fn verify_platform_code_signature(binary: &Path) -> Result<(), UpdateError> {
    #[cfg(target_os = "macos")]
    {
        let status = tokio::process::Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict", "--verbose=2"])
            .arg(binary)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map_err(|error| UpdateError::Verify(format!("无法执行 macOS 签名校验:{error}")))?;
        if !status.success() {
            return Err(UpdateError::Verify("macOS Runtime 代码签名无效".into()));
        }
    }
    #[cfg(target_os = "windows")]
    {
        let script = "& { param([string]$Path) if ((Get-AuthenticodeSignature -LiteralPath $Path).Status -ne 'Valid') { exit 1 } }";
        let status = tokio::process::Command::new("powershell.exe")
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .arg(binary)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map_err(|error| UpdateError::Verify(format!("无法执行 Windows 签名校验:{error}")))?;
        if !status.success() {
            return Err(UpdateError::Verify(
                "Windows Runtime Authenticode 签名无效".into(),
            ));
        }
    }
    Ok(())
}

fn failed_update_info(current: Option<String>, message: String) -> PiRuntimeUpdateInfo {
    PiRuntimeUpdateInfo {
        state: "error",
        published: false,
        current_version: current,
        bundled_version: BUNDLED_RUNTIME_VERSION.into(),
        latest_version: None,
        pi_sdk_version: None,
        has_update: false,
        asset_size: None,
        released_at: None,
        notes: None,
        message: Some(crate::feedback::sanitize_paths(&message)),
    }
}

fn sanitized_error(error: impl ToString) -> String {
    crate::feedback::sanitize_paths(&error.to_string())
}
