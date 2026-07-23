use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::Manager;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PiRuntimeSource {
    AppData,
    Bundled,
    Development,
}

#[derive(Debug, Clone)]
pub struct ResolvedPiRuntime {
    pub binary: PathBuf,
    pub source: PiRuntimeSource,
    pub version: Option<String>,
}

#[derive(Debug, Error)]
pub enum PiRuntimeError {
    #[error("Pi Runtime 未安装或当前构建未包含 Sidecar")]
    Missing,
    #[error("Pi Runtime 路径解析失败:{0}")]
    Resolve(String),
}

#[derive(Debug, Clone)]
struct PiRuntimeRoots {
    app_data: Option<PathBuf>,
    bundled: Option<PathBuf>,
    development: Option<PathBuf>,
}

#[derive(Deserialize)]
struct CurrentRuntime {
    version: String,
}

#[derive(Default, Deserialize)]
struct BadVersions {
    #[serde(default)]
    versions: HashSet<String>,
}

pub fn runtime_binary_name() -> &'static str {
    if cfg!(windows) {
        "caseboard-pi-runtime.exe"
    } else {
        "caseboard-pi-runtime"
    }
}

pub fn resolve_pi_runtime_binary(
    app: Option<&tauri::AppHandle>,
) -> Result<ResolvedPiRuntime, PiRuntimeError> {
    let app_data = crate::db::app_data_dir()
        .map(Some)
        .map_err(|error| PiRuntimeError::Resolve(error.to_string()))?;
    let bundled = match app {
        Some(app) => Some(
            app.path()
                .resource_dir()
                .map_err(|error| PiRuntimeError::Resolve(error.to_string()))?
                .join("pi-runtime"),
        ),
        None => None,
    };
    let development = if cfg!(debug_assertions) {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|root| root.join("sidecars/pi-runtime/dist/bundle"))
    } else {
        None
    };
    resolve_from_roots(&PiRuntimeRoots {
        app_data,
        bundled,
        development,
    })
}

fn resolve_from_roots(roots: &PiRuntimeRoots) -> Result<ResolvedPiRuntime, PiRuntimeError> {
    // Dev 模式必须直接使用当前工作区刚构建的 Sidecar。Tauri 的 resource_dir 可能还留着
    // 上一次打包/复制的 Runtime；若沿用正式版优先级，源码热重载后仍会悄悄运行旧二进制。
    if cfg!(debug_assertions) {
        if let Some(root) = &roots.development {
            let binary = root.join(runtime_binary_name());
            if binary.is_file() {
                return Ok(ResolvedPiRuntime {
                    binary,
                    source: PiRuntimeSource::Development,
                    version: None,
                });
            }
        }
    }

    if let Some(app_data) = &roots.app_data {
        let runtime_root = app_data.join("runtimes/pi");
        if let Ok(text) = std::fs::read_to_string(runtime_root.join("current.json")) {
            if let Ok(current) = serde_json::from_str::<CurrentRuntime>(&text) {
                let bad = std::fs::read_to_string(runtime_root.join("bad-versions.json"))
                    .ok()
                    .and_then(|text| serde_json::from_str::<BadVersions>(&text).ok())
                    .unwrap_or_default();
                if !current.version.trim().is_empty() && !bad.versions.contains(&current.version) {
                    let binary = runtime_root
                        .join("versions")
                        .join(&current.version)
                        .join(runtime_binary_name());
                    if binary.is_file() {
                        return Ok(ResolvedPiRuntime {
                            binary,
                            source: PiRuntimeSource::AppData,
                            version: Some(current.version),
                        });
                    }
                }
            }
        }
    }

    if let Some(root) = &roots.bundled {
        let binary = root.join(runtime_binary_name());
        if binary.is_file() {
            return Ok(ResolvedPiRuntime {
                binary,
                source: PiRuntimeSource::Bundled,
                version: None,
            });
        }
    }

    if let Some(root) = &roots.development {
        let binary = root.join(runtime_binary_name());
        if binary.is_file() {
            return Ok(ResolvedPiRuntime {
                binary,
                source: PiRuntimeSource::Development,
                version: None,
            });
        }
    }

    Err(PiRuntimeError::Missing)
}
