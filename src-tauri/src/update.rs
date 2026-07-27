//! 版本检测 —— 启动时 / 手动触发,按运行环境选择 Tauri updater 签名 manifest。
//!
//! 2026-05-25 V0.1.8 加。
//!
//! 设计:
//!   - endpoint 模板与 `tauri.conf.json` 完全一致,提示与实际安装不会跨 Stable/Legacy 线
//!   - macOS Apple Silicon 13+、Windows x64 → Stable
//!   - macOS Intel、macOS Apple Silicon 11/12/版本未知 → Legacy
//!   - 未知平台/架构 → 不检查更新,绝不误推 Stable
//!   - 当前版本:`env!("CARGO_PKG_VERSION")`,跟 Cargo.toml 一致
//!   - 比对:桥接层只接受 canonical SemVer,远程严格大于本地才算落后
//!   - 超时:8s。失败不报错,返回 `has_update=false` + error 字段给前端日志用
//!
//! 作者明确要求(2026-05-25):**不强制更新**,只提示。用户可点「取消」。

use semver::Version;
use serde::{Deserialize, Serialize};

const UPDATE_MANIFEST_URL_TEMPLATE: &str = "https://lawtools.top/caseboard/updates/{{target}}.json";
const FETCH_TIMEOUT_SEC: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum UpdateChannel {
    Stable,
    Legacy,
}

impl UpdateChannel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Legacy => "legacy",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateRoute {
    channel: UpdateChannel,
    updater_target: String,
}

impl UpdateRoute {
    fn new(channel: UpdateChannel, platform_target: &str) -> Self {
        Self {
            channel,
            updater_target: format!("{}-{platform_target}", channel.as_str()),
        }
    }

    fn metadata_url(&self) -> String {
        UPDATE_MANIFEST_URL_TEMPLATE.replace("{{target}}", &self.updater_target)
    }
}

/// 动态 Tauri updater manifest。额外的 channel/target/download_url 字段由
/// CaseBoard 校验和手动下载兜底使用,Tauri 会忽略它们。
#[derive(Debug, Clone, Deserialize)]
struct RemoteVersion {
    version: String,
    #[serde(default)]
    pub_date: Option<String>,
    #[serde(default)]
    notes: Option<String>,
    url: String,
    signature: String,
    download_url: Option<String>,
    channel: UpdateChannel,
    target: String,
}

/// 给前端的检测结果(序列化为 JSON)
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    /// 当前本机版本(Cargo.toml)
    pub current: String,
    /// 远程最新版本(失败时 None)
    pub latest: Option<String>,
    /// 是否落后(latest > current 才 true)
    pub has_update: bool,
    /// 发布日期(YYYY-MM-DD)
    pub released_at: Option<String>,
    /// 更新说明(Markdown / 纯文本均可,前端按纯文本渲染避免 XSS)
    pub notes: Option<String>,
    /// 与当前通道/平台对应的手动下载 URL(用户点「去下载」开浏览器去这里)
    pub download_url: Option<String>,
    /// 本机进入的更新通道。失败/不支持时可能仍保留已安全判定的通道。
    pub channel: Option<String>,
    /// 传给 Tauri updater `check({ target })` 的精确 target。
    pub updater_target: Option<String>,
    /// 检测失败时的错误描述(成功为 None)。前端只在调试时显示。
    pub error: Option<String>,
}

impl UpdateInfo {
    fn fail(current: &str, route: Option<&UpdateRoute>, msg: impl Into<String>) -> Self {
        Self {
            current: current.to_string(),
            latest: None,
            has_update: false,
            released_at: None,
            notes: None,
            download_url: None,
            channel: route.map(|route| route.channel.as_str().to_string()),
            updater_target: route.map(|route| route.updater_target.clone()),
            error: Some(msg.into()),
        }
    }
}

fn select_update_route(os: &str, arch: &str, macos_major: Option<u32>) -> Option<UpdateRoute> {
    match (os, arch) {
        ("macos", "aarch64") => {
            let channel = match macos_major {
                Some(major) if major >= 13 => UpdateChannel::Stable,
                _ => UpdateChannel::Legacy,
            };
            Some(UpdateRoute::new(channel, "darwin-aarch64"))
        }
        ("macos", "x86_64") => Some(UpdateRoute::new(UpdateChannel::Legacy, "darwin-x86_64")),
        ("windows", "x86_64") => Some(UpdateRoute::new(UpdateChannel::Stable, "windows-x86_64")),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
fn runtime_macos_major() -> Option<u32> {
    use objc2_foundation::NSProcessInfo;

    let version = NSProcessInfo::processInfo().operatingSystemVersion();
    u32::try_from(version.majorVersion).ok()
}

#[cfg(not(target_os = "macos"))]
fn runtime_macos_major() -> Option<u32> {
    None
}

fn runtime_update_route() -> Option<UpdateRoute> {
    select_update_route(
        std::env::consts::OS,
        std::env::consts::ARCH,
        runtime_macos_major(),
    )
}

fn is_caseboard_distribution_url(value: &str) -> bool {
    reqwest::Url::parse(value)
        .map(|url| {
            url.scheme() == "https"
                && url.host_str() == Some("lawtools.top")
                && url.port().is_none()
                && url.path().starts_with("/caseboard/")
        })
        .unwrap_or(false)
}

fn updater_release_date(pub_date: Option<String>) -> Result<Option<String>, String> {
    match pub_date {
        None => Ok(None),
        Some(value) => chrono::DateTime::parse_from_rfc3339(&value)
            .map(|date| Some(date.format("%Y-%m-%d").to_string()))
            .map_err(|_| "更新 manifest pub_date 不是有效 RFC 3339 时间".to_string()),
    }
}

fn parse_canonical_semver(value: &str) -> Option<Version> {
    let version = Version::parse(value).ok()?;
    (version.to_string() == value).then_some(version)
}

fn build_update_info(
    current: &str,
    route: &UpdateRoute,
    remote: Result<RemoteVersion, String>,
) -> UpdateInfo {
    let remote = match remote {
        Ok(remote) => remote,
        Err(error) => return UpdateInfo::fail(current, Some(route), error),
    };

    if remote.channel != route.channel {
        return UpdateInfo::fail(
            current,
            Some(route),
            format!(
                "更新 manifest channel 不匹配:期望 {},发现 {}",
                route.channel.as_str(),
                remote.channel.as_str()
            ),
        );
    }
    if remote.target != route.updater_target {
        return UpdateInfo::fail(
            current,
            Some(route),
            format!(
                "更新 manifest target 不匹配:期望 {},发现 {}",
                route.updater_target, remote.target
            ),
        );
    }
    if remote.signature.trim().is_empty() {
        return UpdateInfo::fail(current, Some(route), "更新 manifest 缺少签名");
    }
    if !is_caseboard_distribution_url(&remote.url) {
        return UpdateInfo::fail(
            current,
            Some(route),
            "更新 manifest 安装 URL 必须位于 https://lawtools.top/caseboard/",
        );
    }
    let download_url = match remote.download_url {
        Some(url) if is_caseboard_distribution_url(&url) => url,
        _ => {
            return UpdateInfo::fail(
                current,
                Some(route),
                "更新 manifest 手动下载 URL 必须位于 https://lawtools.top/caseboard/",
            )
        }
    };

    let released_at = match updater_release_date(remote.pub_date) {
        Ok(released_at) => released_at,
        Err(error) => return UpdateInfo::fail(current, Some(route), error),
    };
    let remote_version = match parse_canonical_semver(&remote.version) {
        Some(version) => version,
        None => {
            return UpdateInfo::fail(
                current,
                Some(route),
                format!("更新 manifest 版本不是有效 SemVer:{}", remote.version),
            )
        }
    };
    let current_version = match parse_canonical_semver(current) {
        Some(version) => version,
        None => {
            return UpdateInfo::fail(
                current,
                Some(route),
                format!("当前 App 版本不是有效 SemVer:{current}"),
            )
        }
    };
    if route.channel == UpdateChannel::Legacy
        && (current_version.major != 0
            || current_version.minor != 4
            || remote_version.major != 0
            || remote_version.minor != 4)
    {
        return UpdateInfo::fail(
            current,
            Some(route),
            format!(
                "Legacy 仅允许 0.4.x 系列内更新，当前 {current_version}，发现 {remote_version}"
            ),
        );
    }
    let has_update = remote_version > current_version;

    UpdateInfo {
        current: current.to_string(),
        latest: Some(remote.version),
        has_update,
        released_at,
        notes: remote.notes,
        download_url: Some(download_url),
        channel: Some(route.channel.as_str().to_string()),
        updater_target: Some(route.updater_target.clone()),
        error: None,
    }
}

/// 检测远程最新版本。
pub async fn check_for_update() -> UpdateInfo {
    let current = env!("CARGO_PKG_VERSION").to_string();
    let route = match runtime_update_route() {
        Some(route) => route,
        None => {
            return UpdateInfo::fail(
                &current,
                None,
                format!(
                    "当前平台不在自动更新支持范围:{}-{}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
            )
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SEC))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return UpdateInfo::fail(
                &current,
                Some(&route),
                format!("HTTP 客户端创建失败: {}", e),
            )
        }
    };

    let resp = match client
        .get(route.metadata_url())
        .header("Accept", "application/json")
        .header("User-Agent", format!("CaseBoard/{}", current))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return build_update_info(
                &current,
                &route,
                Err(format!("拉取更新 manifest 失败: {}", e)),
            )
        }
    };

    if !resp.status().is_success() {
        return build_update_info(
            &current,
            &route,
            Err(format!("更新 manifest HTTP {}", resp.status().as_u16())),
        );
    }

    let remote = match resp.json().await {
        Ok(remote) => Ok(remote),
        Err(e) => Err(format!("解析更新 manifest 失败: {}", e)),
    };
    build_update_info(&current, &route, remote)
}
