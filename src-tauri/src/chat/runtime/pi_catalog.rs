use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::pi_locator::resolve_pi_runtime_binary;
use super::pi_protocol::{
    PiHostMessage, PiModelSummary, PiProviderCatalog, PiSidecarMessage, PI_PROTOCOL_VERSION,
};
use super::pi_sidecar::check_pi_runtime_health;

const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
const CATALOG_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum PiCatalogError {
    #[error("Pi Runtime catalog 不可用:{0}")]
    Unavailable(String),
    #[error("Pi Runtime catalog 不兼容:{0}")]
    Incompatible(String),
}

#[derive(Clone)]
struct CachedCatalog {
    binary: PathBuf,
    sidecar_version: String,
    catalog: PiProviderCatalog,
}

fn catalog_cache() -> &'static RwLock<Option<CachedCatalog>> {
    static CACHE: OnceLock<RwLock<Option<CachedCatalog>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(None))
}

pub async fn load_pi_catalog(
    app: Option<&tauri::AppHandle>,
) -> Result<PiProviderCatalog, PiCatalogError> {
    let resolved = resolve_pi_runtime_binary(app)
        .map_err(|error| PiCatalogError::Unavailable(error.to_string()))?;
    let health = check_pi_runtime_health(&resolved.binary)
        .await
        .map_err(|error| PiCatalogError::Unavailable(error.to_string()))?;
    if let Some(cached) = catalog_cache()
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .filter(|cached| {
            cached.binary == resolved.binary && cached.sidecar_version == health.sidecar_version
        })
    {
        return Ok(cached.catalog);
    }

    let catalog = load_catalog_from_binary(&resolved.binary).await?;
    if let Ok(mut guard) = catalog_cache().write() {
        *guard = Some(CachedCatalog {
            binary: resolved.binary,
            sidecar_version: health.sidecar_version,
            catalog: catalog.clone(),
        });
    }
    Ok(catalog)
}

async fn load_catalog_from_binary(binary: &Path) -> Result<PiProviderCatalog, PiCatalogError> {
    let mut command = Command::new(binary);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::proc_util::hide_console_window(&mut command);
    let mut child = command
        .spawn()
        .map_err(|error| PiCatalogError::Unavailable(error.to_string()))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| PiCatalogError::Unavailable("stdin 不可用".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PiCatalogError::Unavailable("stdout 不可用".into()))?;
    let stderr_task = child.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            let mut buffer = [0_u8; 4096];
            while stderr.read(&mut buffer).await.unwrap_or(0) > 0 {}
        })
    });

    let mut encoded = serde_json::to_vec(&PiHostMessage::CatalogRequest {
        protocol_version: PI_PROTOCOL_VERSION,
    })
    .map_err(|error| PiCatalogError::Unavailable(error.to_string()))?;
    encoded.push(b'\n');
    stdin
        .write_all(&encoded)
        .await
        .map_err(|error| PiCatalogError::Unavailable(error.to_string()))?;
    stdin
        .flush()
        .await
        .map_err(|error| PiCatalogError::Unavailable(error.to_string()))?;
    drop(stdin);

    let mut stdout = BufReader::new(stdout);
    let line = tokio::time::timeout(CATALOG_TIMEOUT, read_bounded_line(&mut stdout))
        .await
        .map_err(|_| PiCatalogError::Unavailable("catalog 读取超时".into()))?
        .map_err(|error| PiCatalogError::Unavailable(error.to_string()))?
        .ok_or_else(|| PiCatalogError::Unavailable("没有 catalog 响应".into()))?;
    let message: PiSidecarMessage = serde_json::from_str(&line)
        .map_err(|_| PiCatalogError::Unavailable("catalog 响应不是有效 JSON".into()))?;
    let PiSidecarMessage::Catalog {
        protocol_version,
        providers,
    } = message
    else {
        return Err(PiCatalogError::Unavailable("响应类型不是 catalog".into()));
    };
    if protocol_version != PI_PROTOCOL_VERSION {
        return Err(PiCatalogError::Incompatible(format!(
            "协议版本 {protocol_version}，当前需要 {PI_PROTOCOL_VERSION}"
        )));
    }
    let status = tokio::time::timeout(CATALOG_TIMEOUT, child.wait())
        .await
        .map_err(|_| PiCatalogError::Unavailable("catalog 进程退出超时".into()))?
        .map_err(|error| PiCatalogError::Unavailable(error.to_string()))?;
    if !status.success() {
        return Err(PiCatalogError::Unavailable("catalog 进程异常退出".into()));
    }
    if let Some(task) = stderr_task {
        let _ = task.await;
    }
    Ok(PiProviderCatalog { providers })
}

pub fn validate_selection<'a>(
    catalog: &'a PiProviderCatalog,
    provider_id: &str,
    model_id: &str,
) -> Result<&'a PiModelSummary, String> {
    let provider = catalog
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| "provider 不在当前 Pi Runtime 目录".to_string())?;
    provider
        .models
        .iter()
        .find(|model| model.id == model_id)
        .ok_or_else(|| "模型不在当前 Pi Runtime 目录".to_string())
}

async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
            if line.len().saturating_add(position) > MAX_LINE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Pi Sidecar 单行消息超过 16 MiB",
                ));
            }
            line.extend_from_slice(&available[..position]);
            reader.consume(position + 1);
            break;
        }
        if line.len().saturating_add(available.len()) > MAX_LINE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Pi Sidecar 单行消息超过 16 MiB",
            ));
        }
        let length = available.len();
        line.extend_from_slice(available);
        reader.consume(length);
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    String::from_utf8(line).map(Some).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "Pi Sidecar 输出不是 UTF-8")
    })
}
