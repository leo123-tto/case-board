//! 个人空间中 Markdown/报告文件层的索引、增量比较与落盘。
//!
//! 业务记录与设置由 `workspace` 负责，副设备源文件归集由 `source` 负责。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::source::{SourceChunk, SourceNeed, SourceSummary};
use super::workspace::{RecordPacket, RecordRef, RecordSummary};
use super::DeviceSyncIdentity;

const MAX_ARTIFACT_BYTES: usize = 8 * 1024 * 1024;
/// 加密 envelope 还会 Base64 膨胀约 1/3；控制明文批次，确保低于网络层 32MB 上限。
pub const MAX_BATCH_PLAIN_BYTES: usize = 16 * 1024 * 1024;
const ALLOWED_SOURCES: &[&str] = &[
    "chat",
    "chat_artifact",
    "case_note",
    "closing_materials",
    "llm_extract",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactSummary {
    pub artifact_id: String,
    pub case_sync_key: String,
    pub content_hash: String,
    pub parent_hash: Option<String>,
    pub revision: i64,
    pub origin_device_id: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactPacket {
    pub summary: ArtifactSummary,
    pub case_name: String,
    pub case_no: Option<String>,
    pub filename: String,
    pub category: Option<String>,
    pub source: String,
    /// cases 表里的报告路径槽；None = documents 工作产物。
    pub report_slot: Option<String>,
    /// 只接受 UTF-8 Markdown；删除墓碑为 None。
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexRequest {
    pub from_device_id: String,
    pub from_device_name: String,
    #[serde(default)]
    pub from_platform: String,
    pub summaries: Vec<ArtifactSummary>,
    #[serde(default)]
    pub record_summaries: Vec<RecordSummary>,
    #[serde(default)]
    pub source_summaries: Vec<SourceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexResponse {
    pub from_device_id: String,
    pub packets: Vec<ArtifactPacket>,
    pub need_from_caller: Vec<String>,
    #[serde(default)]
    pub record_packets: Vec<RecordPacket>,
    #[serde(default)]
    pub need_records_from_caller: Vec<RecordRef>,
    #[serde(default)]
    pub source_needs: Vec<SourceNeed>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushRequest {
    pub from_device_id: String,
    #[serde(default)]
    pub packets: Vec<ArtifactPacket>,
    #[serde(default)]
    pub record_packets: Vec<RecordPacket>,
    #[serde(default)]
    pub source_chunks: Vec<SourceChunk>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyReport {
    pub applied: usize,
    pub unchanged: usize,
    pub conflicts: usize,
    pub pending_cases: usize,
    pub errors: Vec<String>,
}

impl ApplyReport {
    pub fn add(&mut self, other: ApplyReport) {
        self.applied += other.applied;
        self.unchanged += other.unchanged;
        self.conflicts += other.conflicts;
        self.pending_cases += other.pending_cases;
        self.errors.extend(other.errors);
    }
}

#[derive(Debug, FromRow)]
struct CaseRow {
    id: String,
    name: String,
    case_no: Option<String>,
    agg_case_no: Option<String>,
    sync_key: Option<String>,
    case_report_path: Option<String>,
    risk_assessment_path: Option<String>,
    deep_dive_report_path: Option<String>,
    full_report_path: Option<String>,
}

#[derive(Debug, FromRow)]
struct DocRow {
    id: String,
    case_id: String,
    source_path: String,
    filename: String,
    category: Option<String>,
    source: String,
}

#[derive(Debug, FromRow)]
struct LedgerRow {
    artifact_id: String,
    case_sync_key: String,
    content_hash: String,
    parent_hash: Option<String>,
    revision: i64,
    origin_device_id: String,
    updated_at: String,
    deleted_at: Option<String>,
}

impl LedgerRow {
    fn summary(&self) -> ArtifactSummary {
        ArtifactSummary {
            artifact_id: self.artifact_id.clone(),
            case_sync_key: self.case_sync_key.clone(),
            content_hash: self.content_hash.clone(),
            parent_hash: self.parent_hash.clone(),
            revision: self.revision,
            origin_device_id: self.origin_device_id.clone(),
            updated_at: self.updated_at.clone(),
            deleted_at: self.deleted_at.clone(),
        }
    }
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn deterministic_case_key(case: &CaseRow) -> String {
    let case_no = case
        .agg_case_no
        .as_deref()
        .or(case.case_no.as_deref())
        .map(normalized)
        .filter(|v| !v.is_empty());
    let (kind, raw) = match case_no {
        Some(v) => ("case-no", v),
        None => ("case-name", normalized(&case.name)),
    };
    let digest = sha256_hex(raw.as_bytes());
    format!("{kind}:{}", &digest[..32])
}

async fn load_cases(pool: &SqlitePool) -> Result<Vec<CaseRow>, String> {
    sqlx::query_as::<_, CaseRow>(
        "SELECT id, name, case_no, agg_case_no, sync_key, case_report_path, \
         risk_assessment_path, deep_dive_report_path, full_report_path FROM cases",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())
}

async fn ensure_case_keys(pool: &SqlitePool) -> Result<Vec<CaseRow>, String> {
    let cases = load_cases(pool).await?;
    for case in &cases {
        if case.sync_key.is_none() {
            let key = deterministic_case_key(case);
            // 同名且无案号的两个案件可能撞键：第二个回退随机键，交给首次关联解决。
            if sqlx::query("UPDATE cases SET sync_key=? WHERE id=? AND sync_key IS NULL")
                .bind(&key)
                .bind(&case.id)
                .execute(pool)
                .await
                .is_err()
            {
                let fallback = format!("case-id:{}", uuid::Uuid::new_v4());
                sqlx::query("UPDATE cases SET sync_key=? WHERE id=? AND sync_key IS NULL")
                    .bind(fallback)
                    .bind(&case.id)
                    .execute(pool)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    load_cases(pool).await
}

fn path_is_inside_app_data(path: &Path, app_data: &Path) -> bool {
    let Ok(path) = path.canonicalize() else {
        return false;
    };
    let Ok(root) = app_data.canonicalize() else {
        return false;
    };
    path.starts_with(root)
}

fn read_markdown(path: &Path, app_data: &Path) -> Result<String, String> {
    if !path
        .extension()
        .and_then(|v| v.to_str())
        .is_some_and(|v| v.eq_ignore_ascii_case("md"))
    {
        return Err("不是 Markdown 文件".into());
    }
    if !path_is_inside_app_data(path, app_data) {
        return Err("工作产物不在 CaseBoard 数据目录，拒绝同步".into());
    }
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() as usize > MAX_ARTIFACT_BYTES {
        return Err("单份工作区 Markdown 超过 8MB，拒绝同步".into());
    }
    std::fs::read_to_string(path).map_err(|e| format!("读取 Markdown 失败：{e}"))
}

async fn load_ledger(pool: &SqlitePool) -> Result<HashMap<String, LedgerRow>, String> {
    let rows = sqlx::query_as::<_, LedgerRow>(
        "SELECT artifact_id, case_sync_key, content_hash, parent_hash, revision, \
         origin_device_id, updated_at, deleted_at FROM device_sync_artifacts",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|r| (r.artifact_id.clone(), r))
        .collect())
}

async fn track_content(
    pool: &SqlitePool,
    ledger: &mut HashMap<String, LedgerRow>,
    identity: &DeviceSyncIdentity,
    artifact_id: &str,
    case_key: &str,
    content: &str,
) -> Result<ArtifactSummary, String> {
    let hash = sha256_hex(content.as_bytes());
    let stamp = now();
    match ledger.get(artifact_id) {
        Some(old) if old.content_hash == hash && old.deleted_at.is_none() => Ok(old.summary()),
        Some(old) => {
            let revision = old.revision + 1;
            sqlx::query(
                "UPDATE device_sync_artifacts SET case_sync_key=?, parent_hash=?, content_hash=?, \
                 revision=?, origin_device_id=?, updated_at=?, deleted_at=NULL WHERE artifact_id=?",
            )
            .bind(case_key)
            .bind(&old.content_hash)
            .bind(&hash)
            .bind(revision)
            .bind(&identity.device_id)
            .bind(&stamp)
            .bind(artifact_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
            Ok(ArtifactSummary {
                artifact_id: artifact_id.into(),
                case_sync_key: case_key.into(),
                content_hash: hash,
                parent_hash: Some(old.content_hash.clone()),
                revision,
                origin_device_id: identity.device_id.clone(),
                updated_at: stamp,
                deleted_at: None,
            })
        }
        None => {
            sqlx::query(
                "INSERT INTO device_sync_artifacts \
                 (artifact_id,case_sync_key,content_hash,parent_hash,revision,origin_device_id,updated_at) \
                 VALUES (?,?,?,NULL,1,?,?)",
            )
            .bind(artifact_id)
            .bind(case_key)
            .bind(&hash)
            .bind(&identity.device_id)
            .bind(&stamp)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
            Ok(ArtifactSummary {
                artifact_id: artifact_id.into(),
                case_sync_key: case_key.into(),
                content_hash: hash,
                parent_hash: None,
                revision: 1,
                origin_device_id: identity.device_id.clone(),
                updated_at: stamp,
                deleted_at: None,
            })
        }
    }
}

/// 扫描白名单工作产物并刷新本机同步头。远端路径从不进入 packet。
pub async fn build_packets(
    pool: &SqlitePool,
    identity: &DeviceSyncIdentity,
) -> Result<Vec<ArtifactPacket>, String> {
    reconcile_inbox(pool, identity).await?;
    let app_data = crate::db::app_data_dir().map_err(|e| e.to_string())?;
    let cases = ensure_case_keys(pool).await?;
    let case_map: HashMap<String, &CaseRow> = cases.iter().map(|c| (c.id.clone(), c)).collect();
    let mut ledger = load_ledger(pool).await?;
    let docs = sqlx::query_as::<_, DocRow>(
        "SELECT id, case_id, source_path, filename, category, source FROM documents \
         WHERE is_ai_artifact=1 AND deleted_at IS NULL AND source IN \
         ('chat','chat_artifact','case_note','closing_materials','llm_extract') \
         AND (mime_type='text/markdown' OR filename LIKE '%.md')",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut packets = Vec::new();
    let mut active_ids = HashSet::new();
    for doc in docs {
        if !ALLOWED_SOURCES.contains(&doc.source.as_str()) {
            continue;
        }
        let Some(case) = case_map.get(&doc.case_id) else {
            continue;
        };
        let Some(case_key) = case.sync_key.as_deref() else {
            continue;
        };
        let content = match read_markdown(Path::new(&doc.source_path), &app_data) {
            Ok(v) => v,
            Err(e) => {
                crate::dlog!("[device-sync] 跳过 {}：{}", doc.id, e);
                continue;
            }
        };
        let summary =
            track_content(pool, &mut ledger, identity, &doc.id, case_key, &content).await?;
        active_ids.insert(doc.id.clone());
        packets.push(ArtifactPacket {
            summary,
            case_name: case.name.clone(),
            case_no: case.agg_case_no.clone().or_else(|| case.case_no.clone()),
            filename: doc.filename,
            category: doc.category,
            source: doc.source,
            report_slot: None,
            content: Some(content),
        });
    }

    for case in &cases {
        let Some(case_key) = case.sync_key.as_deref() else {
            continue;
        };
        let slots = [
            ("case_report", case.case_report_path.as_deref()),
            ("risk_assessment", case.risk_assessment_path.as_deref()),
            ("deep_dive", case.deep_dive_report_path.as_deref()),
            ("full_report", case.full_report_path.as_deref()),
        ];
        for (slot, path) in slots {
            let Some(path) = path else { continue };
            let content = match read_markdown(Path::new(path), &app_data) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let artifact_id = format!("report:{case_key}:{slot}");
            let summary = track_content(
                pool,
                &mut ledger,
                identity,
                &artifact_id,
                case_key,
                &content,
            )
            .await?;
            active_ids.insert(artifact_id.clone());
            packets.push(ArtifactPacket {
                summary,
                case_name: case.name.clone(),
                case_no: case.agg_case_no.clone().or_else(|| case.case_no.clone()),
                filename: format!("{slot}.md"),
                category: Some("案件报告".into()),
                source: "llm_extract".into(),
                report_slot: Some(slot.into()),
                content: Some(content),
            });
        }
    }

    // 本机曾同步、现在已被删除/软删的产物传播为墓碑；正文不物理删除。
    for old in ledger.values() {
        if !active_ids.contains(&old.artifact_id) {
            let mut summary = old.summary();
            if old.deleted_at.is_none() {
                let stamp = now();
                sqlx::query(
                    "UPDATE device_sync_artifacts SET parent_hash=content_hash, deleted_at=?, \
                     updated_at=?, revision=revision+1, origin_device_id=? WHERE artifact_id=?",
                )
                .bind(&stamp)
                .bind(&stamp)
                .bind(&identity.device_id)
                .bind(&old.artifact_id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
                summary.parent_hash = Some(old.content_hash.clone());
                summary.revision += 1;
                summary.updated_at = stamp.clone();
                summary.deleted_at = Some(stamp);
                summary.origin_device_id = identity.device_id.clone();
            }
            // 墓碑每轮都留在索引中，不能只广播一次；否则对方离线时会永久错过删除。
            packets.push(ArtifactPacket {
                summary,
                case_name: String::new(),
                case_no: None,
                filename: String::new(),
                category: None,
                source: String::new(),
                report_slot: None,
                content: None,
            });
        }
    }
    Ok(packets)
}

pub fn summaries(packets: &[ArtifactPacket]) -> Vec<ArtifactSummary> {
    packets.iter().map(|p| p.summary.clone()).collect()
}

pub fn plan_response(
    local: &[ArtifactPacket],
    caller: &[ArtifactSummary],
    device_id: &str,
) -> IndexResponse {
    let caller_map: HashMap<&str, &ArtifactSummary> =
        caller.iter().map(|s| (s.artifact_id.as_str(), s)).collect();
    let local_map: HashMap<&str, &ArtifactPacket> = local
        .iter()
        .map(|p| (p.summary.artifact_id.as_str(), p))
        .collect();
    let mut packets = Vec::new();
    let mut batch_bytes = 0usize;
    for packet in local.iter().filter(|p| {
        caller_map
            .get(p.summary.artifact_id.as_str())
            .is_none_or(|c| {
                c.content_hash != p.summary.content_hash || c.deleted_at != p.summary.deleted_at
            })
    }) {
        let weight = packet.content.as_ref().map_or(256, |v| v.len() + 1024);
        if !packets.is_empty() && batch_bytes.saturating_add(weight) > MAX_BATCH_PLAIN_BYTES {
            break;
        }
        batch_bytes = batch_bytes.saturating_add(weight);
        packets.push(packet.clone());
    }
    let need_from_caller = caller
        .iter()
        .filter(|c| {
            local_map.get(c.artifact_id.as_str()).is_none_or(|p| {
                p.summary.content_hash != c.content_hash || p.summary.deleted_at != c.deleted_at
            })
        })
        .map(|c| c.artifact_id.clone())
        .collect();
    IndexResponse {
        from_device_id: device_id.into(),
        packets,
        need_from_caller,
        record_packets: Vec::new(),
        need_records_from_caller: Vec::new(),
        source_needs: Vec::new(),
    }
}

fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            _ => c,
        })
        .take(100)
        .collect();
    if cleaned.to_ascii_lowercase().ends_with(".md") {
        cleaned
    } else {
        format!("{cleaned}.md")
    }
}

async fn find_local_case(
    pool: &SqlitePool,
    packet: &ArtifactPacket,
) -> Result<Option<String>, String> {
    if let Some(id) = sqlx::query_scalar::<_, String>("SELECT id FROM cases WHERE sync_key=?")
        .bind(&packet.summary.case_sync_key)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Some(id));
    }
    let cases = load_cases(pool).await?;
    let remote_no = packet
        .case_no
        .as_deref()
        .map(normalized)
        .filter(|v| !v.is_empty());
    let mut matches: Vec<&CaseRow> = if let Some(remote_no) = remote_no {
        cases
            .iter()
            .filter(|c| {
                c.agg_case_no
                    .as_deref()
                    .or(c.case_no.as_deref())
                    .map(normalized)
                    .is_some_and(|v| v == remote_no)
            })
            .collect()
    } else {
        let remote_name = normalized(&packet.case_name);
        cases
            .iter()
            .filter(|c| normalized(&c.name) == remote_name)
            .collect()
    };
    if matches.len() != 1 {
        return Ok(None);
    }
    let id = matches.remove(0).id.clone();
    sqlx::query("UPDATE cases SET sync_key=? WHERE id=?")
        .bind(&packet.summary.case_sync_key)
        .bind(&id)
        .execute(pool)
        .await
        .map_err(|e| format!("关联同步案件失败：{e}"))?;
    Ok(Some(id))
}

async fn save_pending(pool: &SqlitePool, packet: &ArtifactPacket) -> Result<(), String> {
    let base = crate::db::app_data_dir()
        .map_err(|e| e.to_string())?
        .join("device_sync")
        .join("inbox");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let path = base.join(format!(
        "{}.md",
        sha256_hex(packet.summary.artifact_id.as_bytes())
    ));
    if let Some(content) = &packet.content {
        std::fs::write(&path, content).map_err(|e| e.to_string())?;
    }
    sqlx::query(
        "INSERT INTO device_sync_inbox \
         (artifact_id,case_sync_key,case_name,case_no,packet_json,local_path,received_at) \
         VALUES (?,?,?,?,?,?,?) ON CONFLICT(artifact_id) DO UPDATE SET \
         packet_json=excluded.packet_json,local_path=excluded.local_path,received_at=excluded.received_at",
    )
    .bind(&packet.summary.artifact_id)
    .bind(&packet.summary.case_sync_key)
    .bind(&packet.case_name)
    .bind(&packet.case_no)
    .bind(serde_json::to_string(packet).map_err(|e| e.to_string())?)
    .bind(path.to_string_lossy().into_owned())
    .bind(now())
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn write_packet(
    pool: &SqlitePool,
    packet: &ArtifactPacket,
    local_case_id: &str,
) -> Result<String, String> {
    let content = packet.content.as_deref().ok_or("同步包缺 Markdown 正文")?;
    if content.len() > MAX_ARTIFACT_BYTES {
        return Err("同步 Markdown 超过 8MB".into());
    }
    if sha256_hex(content.as_bytes()) != packet.summary.content_hash {
        return Err("同步 Markdown 内容哈希不一致".into());
    }
    let artifact_id = packet.summary.artifact_id.clone();
    let base = crate::db::app_data_dir().map_err(|e| e.to_string())?;
    let dir = if packet.report_slot.is_some() {
        base.join("reports").join("device_sync").join(local_case_id)
    } else {
        base.join("extracts")
            .join(local_case_id)
            .join("synced_artifacts")
    };
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let filename = safe_filename(&packet.filename);
    // 已在本机登记的同一产物优先原位更新，避免每轮同步留下孤儿副本；远端路径从未参与。
    let existing_path: Option<String> = if let Some(slot) = packet.report_slot.as_deref() {
        let sql = match slot {
            "case_report" => "SELECT case_report_path FROM cases WHERE id=?",
            "risk_assessment" => "SELECT risk_assessment_path FROM cases WHERE id=?",
            "deep_dive" => "SELECT deep_dive_report_path FROM cases WHERE id=?",
            "full_report" => "SELECT full_report_path FROM cases WHERE id=?",
            _ => "SELECT NULL FROM cases WHERE id=?",
        };
        sqlx::query_scalar(sql)
            .bind(local_case_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
            .flatten()
    } else {
        sqlx::query_scalar("SELECT source_path FROM documents WHERE id=? AND is_ai_artifact=1")
            .bind(&packet.summary.artifact_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?
    };
    let path = existing_path
        .map(PathBuf::from)
        .filter(|p| path_is_inside_app_data(p, &base))
        .unwrap_or_else(|| {
            dir.join(format!(
                "{}_{}",
                &artifact_id[..8.min(artifact_id.len())],
                filename
            ))
        });
    std::fs::write(&path, content).map_err(|e| e.to_string())?;
    let path_text = path.to_string_lossy().into_owned();

    if let Some(slot) = packet.report_slot.as_deref() {
        let sql = match slot {
            "case_report" => "UPDATE cases SET case_report_path=? WHERE id=?",
            "risk_assessment" => "UPDATE cases SET risk_assessment_path=? WHERE id=?",
            "deep_dive" => "UPDATE cases SET deep_dive_report_path=? WHERE id=?",
            "full_report" => "UPDATE cases SET full_report_path=? WHERE id=?",
            _ => return Err("未知报告槽位".into()),
        };
        sqlx::query(sql)
            .bind(&path_text)
            .bind(local_case_id)
            .execute(pool)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        let source = if ALLOWED_SOURCES.contains(&packet.source.as_str()) {
            packet.source.as_str()
        } else {
            "chat_artifact"
        };
        sqlx::query(
            "INSERT INTO documents \
             (id,case_id,source_path,filename,category,is_ai_artifact,mime_type,size_bytes,modified_at, \
              extraction_status,extracted_text_path,source,created_at) \
             VALUES (?,?,?,?,?,1,'text/markdown',?,?, 'done',?,?,?) \
             ON CONFLICT(id) DO UPDATE SET case_id=excluded.case_id,source_path=excluded.source_path, \
              filename=excluded.filename,category=excluded.category,size_bytes=excluded.size_bytes, \
              modified_at=excluded.modified_at,extracted_text_path=excluded.extracted_text_path, \
              source=excluded.source,deleted_at=NULL",
        )
        .bind(&artifact_id)
        .bind(local_case_id)
        .bind(&path_text)
        .bind(&filename)
        .bind(&packet.category)
        .bind(content.len() as i64)
        .bind(&packet.summary.updated_at)
        .bind(&path_text)
        .bind(source)
        .bind(&packet.summary.updated_at)
        .execute(pool)
        .await
        .map_err(|e| format!("登记同步工作产物失败：{e}"))?;
    }

    let summary = packet.summary.clone();
    sqlx::query(
        "INSERT INTO device_sync_artifacts \
         (artifact_id,case_sync_key,content_hash,parent_hash,revision,origin_device_id,updated_at,deleted_at) \
         VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(artifact_id) DO UPDATE SET \
         case_sync_key=excluded.case_sync_key,content_hash=excluded.content_hash, \
         parent_hash=excluded.parent_hash,revision=excluded.revision, \
         origin_device_id=excluded.origin_device_id,updated_at=excluded.updated_at,deleted_at=excluded.deleted_at",
    )
    .bind(&summary.artifact_id)
    .bind(&summary.case_sync_key)
    .bind(&summary.content_hash)
    .bind(&summary.parent_hash)
    .bind(summary.revision)
    .bind(&summary.origin_device_id)
    .bind(&summary.updated_at)
    .bind(&summary.deleted_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(artifact_id)
}

/// 同一原文书的同一落败内容在所有设备上得到同一个 conflict id，避免每轮重复造副本。
fn conflict_packet(
    original: &ArtifactPacket,
    losing_hash: &str,
    losing_content: String,
    origin_device_id: &str,
) -> ArtifactPacket {
    let original_key = sha256_hex(original.summary.artifact_id.as_bytes());
    let artifact_id = format!(
        "conflict-{}-{}",
        &original_key[..12],
        &losing_hash[..16.min(losing_hash.len())]
    );
    let base = safe_filename(&original.filename);
    let stem = base.trim_end_matches(".md");
    ArtifactPacket {
        summary: ArtifactSummary {
            artifact_id,
            case_sync_key: original.summary.case_sync_key.clone(),
            content_hash: losing_hash.to_string(),
            parent_hash: None,
            revision: 1,
            origin_device_id: origin_device_id.to_string(),
            updated_at: now(),
            deleted_at: None,
        },
        case_name: original.case_name.clone(),
        case_no: original.case_no.clone(),
        filename: format!(
            "{stem}（冲突副本 {}）.md",
            &losing_hash[..8.min(losing_hash.len())]
        ),
        category: original.category.clone(),
        source: "chat_artifact".into(),
        report_slot: None,
        content: Some(losing_content),
    }
}

async fn read_current_content(
    pool: &SqlitePool,
    packet: &ArtifactPacket,
    local_case_id: &str,
) -> Option<String> {
    let path: Option<String> = if let Some(slot) = packet.report_slot.as_deref() {
        let sql = match slot {
            "case_report" => "SELECT case_report_path FROM cases WHERE id=?",
            "risk_assessment" => "SELECT risk_assessment_path FROM cases WHERE id=?",
            "deep_dive" => "SELECT deep_dive_report_path FROM cases WHERE id=?",
            "full_report" => "SELECT full_report_path FROM cases WHERE id=?",
            _ => return None,
        };
        sqlx::query_scalar::<_, Option<String>>(sql)
            .bind(local_case_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .flatten()
    } else {
        sqlx::query_scalar::<_, String>(
            "SELECT source_path FROM documents WHERE id=? AND is_ai_artifact=1",
        )
        .bind(&packet.summary.artifact_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
    };
    let app_data = crate::db::app_data_dir().ok()?;
    read_markdown(Path::new(&path?), &app_data).ok()
}

pub async fn apply_packets(
    pool: &SqlitePool,
    _identity: &DeviceSyncIdentity,
    packets: &[ArtifactPacket],
) -> Result<ApplyReport, String> {
    let mut report = ApplyReport::default();
    let ledger = load_ledger(pool).await?;
    for packet in packets {
        let result: Result<(), String> = async {
            if packet.summary.deleted_at.is_some() {
                match ledger.get(&packet.summary.artifact_id) {
                    Some(local)
                        if local.content_hash == packet.summary.content_hash
                            && packet.summary.revision > local.revision =>
                    {
                        sqlx::query(
                            "UPDATE documents SET deleted_at=? WHERE id=? AND is_ai_artifact=1",
                        )
                        .bind(packet.summary.deleted_at.as_deref())
                        .bind(&packet.summary.artifact_id)
                        .execute(pool)
                        .await
                        .map_err(|e| e.to_string())?;
                        sqlx::query(
                            "UPDATE device_sync_artifacts SET parent_hash=?,deleted_at=?,revision=?, \
                             updated_at=?,origin_device_id=? WHERE artifact_id=?",
                        )
                        .bind(&packet.summary.parent_hash)
                        .bind(packet.summary.deleted_at.as_deref())
                        .bind(packet.summary.revision)
                        .bind(&packet.summary.updated_at)
                        .bind(&packet.summary.origin_device_id)
                        .bind(&packet.summary.artifact_id)
                        .execute(pool)
                        .await
                        .map_err(|e| e.to_string())?;
                        report.applied += 1;
                    }
                    Some(local) if local.content_hash != packet.summary.content_hash => {
                        // 一边删除、另一边继续编辑：保留编辑版，禁止远端墓碑静默覆盖。
                        report.conflicts += 1;
                    }
                    Some(_) => report.unchanged += 1,
                    None => {
                        sqlx::query(
                            "INSERT INTO device_sync_artifacts \
                             (artifact_id,case_sync_key,content_hash,parent_hash,revision,origin_device_id,updated_at,deleted_at) \
                             VALUES (?,?,?,?,?,?,?,?)",
                        )
                        .bind(&packet.summary.artifact_id)
                        .bind(&packet.summary.case_sync_key)
                        .bind(&packet.summary.content_hash)
                        .bind(&packet.summary.parent_hash)
                        .bind(packet.summary.revision)
                        .bind(&packet.summary.origin_device_id)
                        .bind(&packet.summary.updated_at)
                        .bind(&packet.summary.deleted_at)
                        .execute(pool)
                        .await
                        .map_err(|e| e.to_string())?;
                        report.applied += 1;
                    }
                }
                return Ok(());
            }
            let Some(local_case_id) = find_local_case(pool, packet).await? else {
                save_pending(pool, packet).await?;
                report.pending_cases += 1;
                return Ok(());
            };
            match ledger.get(&packet.summary.artifact_id) {
                Some(local) if local.content_hash == packet.summary.content_hash => {
                    report.unchanged += 1;
                }
                Some(local)
                    if packet.summary.parent_hash.as_deref() == Some(&local.content_hash) =>
                {
                    write_packet(pool, packet, &local_case_id).await?;
                    report.applied += 1;
                }
                Some(local)
                    if local.parent_hash.as_deref() == Some(&packet.summary.content_hash) =>
                {
                    report.unchanged += 1; // 本机是远端版本的后继，不回退。
                }
                Some(local) => {
                    // 两端同时编辑：按哈希选同一个主版本，落败版用稳定 ID 保存。
                    // 两边独立执行也会在一轮后收敛，不会无限制造随机冲突副本。
                    if packet.summary.content_hash > local.content_hash {
                        if let Some(local_content) =
                            read_current_content(pool, packet, &local_case_id).await
                        {
                            let copy = conflict_packet(
                                packet,
                                &local.content_hash,
                                local_content,
                                &local.origin_device_id,
                            );
                            write_packet(pool, &copy, &local_case_id).await?;
                        }
                        write_packet(pool, packet, &local_case_id).await?;
                    } else if let Some(remote_content) = packet.content.clone() {
                        let copy = conflict_packet(
                            packet,
                            &packet.summary.content_hash,
                            remote_content,
                            &packet.summary.origin_device_id,
                        );
                        write_packet(pool, &copy, &local_case_id).await?;
                    }
                    report.conflicts += 1;
                }
                None => {
                    write_packet(pool, packet, &local_case_id).await?;
                    report.applied += 1;
                }
            }
            Ok(())
        }
        .await;
        if let Err(e) = result {
            report.errors.push(format!("{}：{e}", packet.filename));
        }
    }
    Ok(report)
}

pub async fn reconcile_inbox(
    pool: &SqlitePool,
    _identity: &DeviceSyncIdentity,
) -> Result<usize, String> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT artifact_id,packet_json,local_path FROM device_sync_inbox ORDER BY received_at",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    let mut applied = 0;
    for (id, json, local_path) in rows {
        let Ok(packet) = serde_json::from_str::<ArtifactPacket>(&json) else {
            continue;
        };
        let Some(case_id) = find_local_case(pool, &packet).await? else {
            continue;
        };
        if write_packet(pool, &packet, &case_id).await.is_ok() {
            sqlx::query("DELETE FROM device_sync_inbox WHERE artifact_id=?")
                .bind(&id)
                .execute(pool)
                .await
                .map_err(|e| e.to_string())?;
            let _ = std::fs::remove_file(local_path);
            applied += 1;
        }
    }
    Ok(applied)
}

pub async fn get_state(pool: &SqlitePool, key: &str) -> Result<Option<String>, String> {
    sqlx::query_scalar("SELECT value FROM device_sync_state WHERE key=?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())
}

pub async fn set_state(pool: &SqlitePool, key: &str, value: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO device_sync_state(key,value) VALUES(?,?) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn counts(pool: &SqlitePool) -> Result<(i64, i64, i64), String> {
    let artifacts = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM device_sync_artifacts WHERE deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    let pending = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM device_sync_inbox")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;
    let conflicts = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM documents WHERE filename LIKE '%冲突副本%' AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok((artifacts, pending, conflicts))
}
