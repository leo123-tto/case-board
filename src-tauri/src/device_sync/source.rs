//! 副设备源文件向主力设备的单向归集。
//!
//! 主力设备绝不向外发送源文件，副设备之间也不交换。协议只携带相对路径和分块正文，
//! 对端绝对路径不会离开本机。每块仍包在 device_sync 的 AEAD 信封中。

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use super::DeviceSyncIdentity;

pub const CHUNK_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct SourceRef {
    pub document_id: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSummary {
    pub document_id: String,
    pub case_sync_key: String,
    pub filename: String,
    pub relative_path: String,
    pub content_hash: String,
    pub size_bytes: u64,
    pub origin_device_id: String,
    pub origin_device_name: String,
}

impl SourceSummary {
    pub fn key(&self) -> SourceRef {
        SourceRef {
            document_id: self.document_id.clone(),
            content_hash: self.content_hash.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SourceCandidate {
    pub summary: SourceSummary,
    pub local_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceNeed {
    pub source: SourceRef,
    pub offset: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceChunk {
    pub summary: SourceSummary,
    pub offset: u64,
    pub data_base64: String,
}

#[derive(Debug, Clone, Default)]
pub struct SourceApplyReport {
    pub completed: usize,
    pub accepted_bytes: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, FromRow)]
struct SourceDocRow {
    id: String,
    source_path: String,
    filename: String,
    source_folder: String,
    sync_key: String,
}

#[derive(Debug, FromRow)]
struct SourceLedgerRow {
    local_path: String,
    fingerprint: String,
    content_hash: String,
    uploaded_to_primary_at: Option<String>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buf).map_err(|e| e.to_string())?;
        if read == 0 {
            break;
        }
        hash.update(&buf[..read]);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn file_fingerprint(meta: &std::fs::Metadata) -> String {
    let modified = meta
        .modified()
        .ok()
        .and_then(|v| v.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |v| v.as_nanos());
    format!("{}:{modified}", meta.len())
}

fn safe_name(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            _ => c,
        })
        .take(120)
        .collect();
    if cleaned.trim().is_empty() {
        "未命名材料".into()
    } else {
        cleaned
    }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn safe_relative(value: &str, fallback: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for component in Path::new(value).components() {
        if let Component::Normal(part) = component {
            let part = safe_name(&part.to_string_lossy());
            if !part.is_empty() {
                out.push(part);
            }
        }
    }
    if out.as_os_str().is_empty() {
        out.push(safe_name(fallback));
    }
    out
}

fn canonical_inside(path: &Path, root: &Path) -> bool {
    let (Ok(path), Ok(root)) = (path.canonicalize(), root.canonicalize()) else {
        return false;
    };
    path.starts_with(root)
}

pub async fn build_candidates(
    pool: &SqlitePool,
    identity: &DeviceSyncIdentity,
) -> Result<Vec<SourceCandidate>, String> {
    if identity.is_primary() {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, SourceDocRow>(
        "SELECT d.id,d.source_path,d.filename,c.source_folder,c.sync_key \
         FROM documents d JOIN cases c ON c.id=d.case_id \
         WHERE d.is_ai_artifact=0 AND d.deleted_at IS NULL AND d.missing=0 \
           AND c.sync_key IS NOT NULL ORDER BY d.created_at,d.id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    for row in rows {
        let path = PathBuf::from(&row.source_path);
        let root = PathBuf::from(&row.source_folder);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() || !canonical_inside(&path, &root) {
            continue;
        }
        if meta.len() > i64::MAX as u64 {
            continue;
        }
        let fingerprint = file_fingerprint(&meta);
        let ledger = sqlx::query_as::<_, SourceLedgerRow>(
            "SELECT local_path,fingerprint,content_hash,uploaded_to_primary_at \
             FROM device_sync_source_files WHERE document_id=?",
        )
        .bind(&row.id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
        let content_hash = match &ledger {
            Some(old) if old.fingerprint == fingerprint && old.local_path == row.source_path => {
                old.content_hash.clone()
            }
            _ => {
                let hash_path = path.clone();
                tokio::task::spawn_blocking(move || sha256_file(&hash_path))
                    .await
                    .map_err(|e| e.to_string())??
            }
        };
        let uploaded = ledger
            .as_ref()
            .and_then(|old| {
                (old.content_hash == content_hash).then(|| old.uploaded_to_primary_at.clone())
            })
            .flatten();
        sqlx::query(
            "INSERT INTO device_sync_source_files \
             (document_id,local_path,fingerprint,content_hash,size_bytes,uploaded_to_primary_at,last_error) \
             VALUES(?,?,?,?,?,?,NULL) ON CONFLICT(document_id) DO UPDATE SET \
             local_path=excluded.local_path,fingerprint=excluded.fingerprint, \
             content_hash=excluded.content_hash,size_bytes=excluded.size_bytes, \
             uploaded_to_primary_at=excluded.uploaded_to_primary_at,last_error=NULL",
        )
        .bind(&row.id)
        .bind(&row.source_path)
        .bind(&fingerprint)
        .bind(&content_hash)
        .bind(meta.len() as i64)
        .bind(uploaded)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
        let relative_path = path
            .strip_prefix(&root)
            .ok()
            .filter(|v| !v.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new(&row.filename))
            .to_string_lossy()
            .into_owned();
        candidates.push(SourceCandidate {
            summary: SourceSummary {
                document_id: row.id,
                case_sync_key: row.sync_key,
                filename: row.filename,
                relative_path,
                content_hash,
                size_bytes: meta.len(),
                origin_device_id: identity.device_id.clone(),
                origin_device_name: identity.device_name.clone(),
            },
            local_path: path,
        });
    }
    Ok(candidates)
}

pub fn summaries(candidates: &[SourceCandidate]) -> Vec<SourceSummary> {
    candidates.iter().map(|v| v.summary.clone()).collect()
}

/// 主力设备决定需要哪些源文件，并返回断点偏移。副设备调用时始终返回空。
pub async fn plan_needs(
    pool: &SqlitePool,
    identity: &DeviceSyncIdentity,
    incoming: &[SourceSummary],
) -> Result<Vec<SourceNeed>, String> {
    if !identity.is_primary() {
        return Ok(Vec::new());
    }
    let mut needs = Vec::new();
    for summary in incoming {
        let completed: Option<(String, String)> = sqlx::query_as(
            "SELECT content_hash,local_path FROM device_sync_source_files WHERE document_id=?",
        )
        .bind(&summary.document_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
        if completed
            .is_some_and(|(hash, path)| hash == summary.content_hash && Path::new(&path).is_file())
        {
            continue;
        }
        let partial: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT content_hash,received_bytes,temp_path FROM device_sync_source_inbox WHERE document_id=?",
        )
        .bind(&summary.document_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
        let offset = partial
            .filter(|(hash, _, path)| hash == &summary.content_hash && Path::new(path).is_file())
            .map_or(0, |(_, received, path)| {
                std::fs::metadata(path)
                    .ok()
                    .map_or(0, |m| m.len().min(received.max(0) as u64))
            });
        needs.push(SourceNeed {
            source: summary.key(),
            offset,
        });
    }
    Ok(needs)
}

pub fn read_chunk(candidate: &SourceCandidate, offset: u64) -> Result<SourceChunk, String> {
    if offset > candidate.summary.size_bytes {
        return Err("源文件断点超过文件大小".into());
    }
    let mut file = std::fs::File::open(&candidate.local_path).map_err(|e| e.to_string())?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| e.to_string())?;
    let remaining = candidate.summary.size_bytes.saturating_sub(offset) as usize;
    let mut data = vec![0u8; remaining.min(CHUNK_BYTES)];
    file.read_exact(&mut data).map_err(|e| e.to_string())?;
    Ok(SourceChunk {
        summary: candidate.summary.clone(),
        offset,
        data_base64: base64::engine::general_purpose::STANDARD.encode(data),
    })
}

fn default_primary_root() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.document_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| crate::db::app_data_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("案件看板")
        .join("跨设备归集")
}

async fn target_case_root(
    pool: &SqlitePool,
    summary: &SourceSummary,
) -> Result<(String, PathBuf), String> {
    let row: Option<(String, String, String)> =
        sqlx::query_as("SELECT id,name,source_folder FROM cases WHERE sync_key=?")
            .bind(&summary.case_sync_key)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    let (case_id, case_name, source_folder) = row.ok_or("主力设备尚未收到对应案件")?;
    let configured = PathBuf::from(source_folder);
    let placeholder_root = crate::db::app_data_dir()
        .map_err(|e| e.to_string())?
        .join("device_sync")
        .join("source_placeholders");
    let root = if configured.is_dir() && !configured.starts_with(&placeholder_root) {
        configured
    } else {
        let root = default_primary_root().join(format!(
            "{}_{}",
            safe_name(&case_name),
            &case_id[..8.min(case_id.len())]
        ));
        std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
        sqlx::query("UPDATE cases SET source_folder=? WHERE id=?")
            .bind(root.to_string_lossy().into_owned())
            .bind(&case_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
        root
    };
    Ok((case_id, root))
}

fn suffixed_target(path: &Path, summary: &SourceSummary) -> PathBuf {
    let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("材料");
    let ext = path.extension().and_then(|v| v.to_str());
    let doc_part: String = safe_name(&summary.document_id).chars().take(8).collect();
    let suffix = format!(
        "{}（来自{}-{}-{}）",
        stem,
        safe_name(&summary.origin_device_name),
        &summary.content_hash[..8],
        doc_part
    );
    path.with_file_name(match ext {
        Some(ext) if !ext.is_empty() => format!("{suffix}.{ext}"),
        _ => suffix,
    })
}

fn collision_safe_target(path: PathBuf, summary: &SourceSummary) -> Result<PathBuf, String> {
    if !path.exists() {
        return Ok(path);
    }
    if path.is_file() && sha256_file(&path)? == summary.content_hash {
        return Ok(path);
    }
    Ok(suffixed_target(&path, summary))
}

async fn finish_file(
    pool: &SqlitePool,
    summary: &SourceSummary,
    temp_path: &Path,
) -> Result<(), String> {
    let actual = sha256_file(temp_path)?;
    if actual != summary.content_hash {
        return Err("源文件完整性校验失败".into());
    }
    let (case_id, root) = target_case_root(pool, summary).await?;
    let relative = safe_relative(&summary.relative_path, &summary.filename);
    let desired = root.join("跨设备收件").join(relative);
    let mut target = collision_safe_target(desired.clone(), summary)?;
    let existing_doc: Option<String> =
        sqlx::query_scalar("SELECT id FROM documents WHERE case_id=? AND source_path=?")
            .bind(&case_id)
            .bind(target.to_string_lossy().into_owned())
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
    if existing_doc.is_some_and(|id| id != summary.document_id) {
        target = suffixed_target(&desired, summary);
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if target != temp_path && std::fs::rename(temp_path, &target).is_err() {
        std::fs::copy(temp_path, &target).map_err(|e| e.to_string())?;
        std::fs::remove_file(temp_path).map_err(|e| e.to_string())?;
    }
    let target_text = target.to_string_lossy().into_owned();
    sqlx::query(
        "INSERT INTO documents(id,case_id,source_path,filename,is_ai_artifact,size_bytes,missing,source,extraction_status) \
         VALUES(?,?,?,?,0,?,0,'scan','pending') ON CONFLICT(id) DO UPDATE SET \
         case_id=excluded.case_id,source_path=excluded.source_path,filename=excluded.filename, \
         size_bytes=excluded.size_bytes,missing=0,deleted_at=NULL",
    )
    .bind(&summary.document_id)
    .bind(&case_id)
    .bind(&target_text)
    .bind(
        target
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or(&summary.filename),
    )
    .bind(summary.size_bytes as i64)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query(
        "INSERT INTO device_sync_source_files \
         (document_id,local_path,fingerprint,content_hash,size_bytes,uploaded_to_primary_at,last_error) \
         VALUES(?,?,?,?,?,?,NULL) ON CONFLICT(document_id) DO UPDATE SET \
         local_path=excluded.local_path,fingerprint=excluded.fingerprint,content_hash=excluded.content_hash, \
         size_bytes=excluded.size_bytes,uploaded_to_primary_at=excluded.uploaded_to_primary_at,last_error=NULL",
    )
    .bind(&summary.document_id)
    .bind(&target_text)
    .bind(format!("received:{}", summary.size_bytes))
    .bind(&summary.content_hash)
    .bind(summary.size_bytes as i64)
    .bind(now())
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM device_sync_source_inbox WHERE document_id=?")
        .bind(&summary.document_id)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn apply_chunks(
    pool: &SqlitePool,
    identity: &DeviceSyncIdentity,
    chunks: &[SourceChunk],
) -> Result<SourceApplyReport, String> {
    if !identity.is_primary() && !chunks.is_empty() {
        return Err("只有主力设备可以接收源文件".into());
    }
    let mut report = SourceApplyReport::default();
    for chunk in chunks {
        let result: Result<bool, String> = async {
            if !valid_sha256(&chunk.summary.content_hash) {
                return Err("源文件哈希格式无效".into());
            }
            if chunk.summary.size_bytes > i64::MAX as u64 {
                return Err("源文件大小超过支持范围".into());
            }
            if chunk.summary.origin_device_id == identity.device_id {
                return Err("主力设备不会向自己归集源文件".into());
            }
            let data = base64::engine::general_purpose::STANDARD
                .decode(&chunk.data_base64)
                .map_err(|_| "源文件分块编码无效")?;
            if data.len() > CHUNK_BYTES {
                return Err("源文件分块超过限制".into());
            }
            if chunk.offset.saturating_add(data.len() as u64) > chunk.summary.size_bytes {
                return Err("源文件分块越界".into());
            }
            let base = crate::db::app_data_dir()
                .map_err(|e| e.to_string())?
                .join("device_sync")
                .join("source_inbox");
            std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
            let temp = base.join(format!(
                "{}-{}.part",
                safe_name(&chunk.summary.document_id),
                &chunk.summary.content_hash[..12.min(chunk.summary.content_hash.len())]
            ));
            let existing: Option<(String, i64, String)> = sqlx::query_as(
                "SELECT content_hash,received_bytes,temp_path FROM device_sync_source_inbox WHERE document_id=?",
            )
            .bind(&chunk.summary.document_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;
            let received = existing
                .filter(|(hash, _, path)| {
                    hash == &chunk.summary.content_hash && path == &temp.to_string_lossy()
                })
                .map_or(0, |(_, count, path)| {
                    std::fs::metadata(path)
                        .ok()
                        .map_or(0, |meta| meta.len().min(count.max(0) as u64))
                });
            if chunk.offset != received {
                return Err(format!("源文件断点不一致，主力设备需要从 {received} 继续"));
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&temp)
                .map_err(|e| e.to_string())?;
            file.write_all(&data).map_err(|e| e.to_string())?;
            file.flush().map_err(|e| e.to_string())?;
            let next = received + data.len() as u64;
            sqlx::query(
                "INSERT INTO device_sync_source_inbox \
                 (document_id,case_sync_key,filename,relative_path,content_hash,total_size,origin_device_id,origin_name,temp_path,received_bytes,updated_at) \
                 VALUES(?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(document_id) DO UPDATE SET \
                 case_sync_key=excluded.case_sync_key,filename=excluded.filename,relative_path=excluded.relative_path, \
                 content_hash=excluded.content_hash,total_size=excluded.total_size,origin_device_id=excluded.origin_device_id, \
                 origin_name=excluded.origin_name,temp_path=excluded.temp_path,received_bytes=excluded.received_bytes,updated_at=excluded.updated_at",
            )
            .bind(&chunk.summary.document_id)
            .bind(&chunk.summary.case_sync_key)
            .bind(&chunk.summary.filename)
            .bind(&chunk.summary.relative_path)
            .bind(&chunk.summary.content_hash)
            .bind(chunk.summary.size_bytes as i64)
            .bind(&chunk.summary.origin_device_id)
            .bind(&chunk.summary.origin_device_name)
            .bind(temp.to_string_lossy().into_owned())
            .bind(next as i64)
            .bind(now())
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
            report.accepted_bytes += data.len();
            if next == chunk.summary.size_bytes {
                finish_file(pool, &chunk.summary, &temp).await?;
                return Ok(true);
            }
            Ok(false)
        }
        .await;
        match result {
            Ok(true) => report.completed += 1,
            Ok(false) => {}
            Err(e) => report
                .errors
                .push(format!("{}：{e}", chunk.summary.filename)),
        }
    }
    Ok(report)
}

pub async fn mark_uploaded(pool: &SqlitePool, source: &SourceRef) -> Result<(), String> {
    sqlx::query(
        "UPDATE device_sync_source_files SET uploaded_to_primary_at=?,last_error=NULL \
         WHERE document_id=? AND content_hash=?",
    )
    .bind(now())
    .bind(&source.document_id)
    .bind(&source.content_hash)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn pending_count(
    pool: &SqlitePool,
    identity: Option<&DeviceSyncIdentity>,
) -> Result<i64, String> {
    if identity.is_some_and(DeviceSyncIdentity::is_primary) {
        sqlx::query_scalar("SELECT COUNT(*) FROM device_sync_source_inbox")
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())
    } else {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM device_sync_source_files WHERE uploaded_to_primary_at IS NULL",
        )
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
    }
}
