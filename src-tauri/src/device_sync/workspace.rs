//! 个人空间的逻辑记录同步。
//!
//! 这里同步业务实体和逐项设置，而不是复制 SQLite。账本只保存哈希与版本；案件正文、
//! API Key 等载荷只会进入 `net` 层的 AEAD 加密消息。所有绝对路径字段都在协议边界剔除。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use sqlx::{Row, Sqlite, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::DeviceSyncIdentity;

pub const MAX_RECORD_BATCH_BYTES: usize = 8 * 1024 * 1024;
const MAX_DERIVED_MD_BYTES: usize = 8 * 1024 * 1024;
const TICKTICK_ENTITY: &str = "ticktick_state";
const TICKTICK_RECORD_ID: &str = "default";

#[derive(Clone, Copy)]
struct TableSpec {
    name: &'static str,
    filter: Option<&'static str>,
    excluded: &'static [&'static str],
}

// 父表必须排在子表之前。运行任务、缓存、统计账本、团队态和本机授权不进入个人空间。
const TABLES: &[TableSpec] = &[
    TableSpec {
        name: "cases",
        filter: None,
        excluded: &[
            "source_folder",
            "last_scanned_at",
            "judge_id",
            "case_report_path",
            "risk_assessment_path",
            "deep_dive_report_path",
            "full_report_path",
        ],
    },
    TableSpec {
        name: "parties",
        filter: None,
        excluded: &["id_doc_path"],
    },
    TableSpec {
        name: "documents",
        filter: Some("is_ai_artifact=0"),
        excluded: &[
            "source_path",
            "extracted_text_path",
            "cache_key",
            "modified_at",
            "missing",
            "last_error",
        ],
    },
    TableSpec {
        name: "contacts",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "events",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "mail_records",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "execution_targets",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "case_stages",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "case_fees",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "case_logs",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "personal_tasks",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "execution_payments",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "case_preservations",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "case_payments",
        filter: None,
        excluded: &["source_path"],
    },
    TableSpec {
        name: "case_instances",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "case_todos",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "calendar_events",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "lawyer_profiles",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "contract_drafts",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "contract_draft_versions",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "contract_preferences",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "chat_tasks",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "chat_messages",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "document_tags",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "document_bookmarks",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "case_memories",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "global_memories",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "memory_events",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "memory_candidates",
        filter: None,
        excluded: &[],
    },
    TableSpec {
        name: "memory_evidence",
        filter: None,
        excluded: &[],
    },
];

// 与 OS、路径、设备身份或本机登录态绑定的字段不跨设备。其余设置（含各云 API 配置）
// 拆成逐字段记录，以免两台设备同时改不同 API 时互相覆盖整个 settings.json。
const LOCAL_SETTING_KEYS: &[&str] = &[
    "ocr_provider",
    "llm_provider",
    "local_model_dir",
    "local_server_auto_start",
    "cloud_enabled",
    "ollama_endpoint",
    "ollama_model",
    "kb_semantic_auto_index",
    "client_id",
    "feishu_lark_cli_path",
    "court_filing_cli_path",
    "court_filing_python",
    "court_filing_cookie_dir",
    "local_kb_root",
    "local_kb_enabled",
    "mcp_servers",
    "team",
    "device_sync_enabled",
    "device_sync",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecordSummary {
    pub entity_type: String,
    pub record_id: String,
    pub content_hash: String,
    pub parent_hash: Option<String>,
    pub revision: i64,
    pub origin_device_id: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
}

impl RecordSummary {
    pub fn key(&self) -> RecordRef {
        RecordRef {
            entity_type: self.entity_type.clone(),
            record_id: self.record_id.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct RecordRef {
    pub entity_type: String,
    pub record_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordPacket {
    pub summary: RecordSummary,
    pub payload: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct RecordApplyReport {
    pub applied: usize,
    pub unchanged: usize,
    pub conflicts: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, sqlx::FromRow)]
struct LedgerRow {
    entity_type: String,
    record_id: String,
    content_hash: String,
    parent_hash: Option<String>,
    revision: i64,
    origin_device_id: String,
    updated_at: String,
    deleted_at: Option<String>,
}

impl LedgerRow {
    fn summary(&self) -> RecordSummary {
        RecordSummary {
            entity_type: self.entity_type.clone(),
            record_id: self.record_id.clone(),
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
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn payload_hash(value: &Value) -> Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    Ok(sha256_hex(&bytes))
}

fn table_spec(name: &str) -> Option<&'static TableSpec> {
    TABLES.iter().find(|spec| spec.name == name)
}

fn table_rank(name: &str) -> usize {
    if matches!(name, "setting" | TICKTICK_ENTITY) {
        return TABLES.len();
    }
    TABLES
        .iter()
        .position(|spec| spec.name == name)
        .unwrap_or(usize::MAX - 1)
}

fn quote_ident(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

async fn columns_for(pool: &SqlitePool, spec: &TableSpec) -> Result<Vec<String>, String> {
    let rows = sqlx::query(&format!("PRAGMA table_info({})", quote_ident(spec.name)))
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
    let mut columns = Vec::new();
    for row in rows {
        let name: String = row.try_get(1).map_err(|e| e.to_string())?;
        if !spec.excluded.contains(&name.as_str()) {
            columns.push(name);
        }
    }
    if !columns.iter().any(|c| c == "id") {
        return Err(format!("同步表 {} 缺少 id 主键", spec.name));
    }
    Ok(columns)
}

fn json_select_sql(spec: &TableSpec, columns: &[String]) -> String {
    let args = columns
        .iter()
        .flat_map(|c| [format!("'{}'", c.replace('\'', "''")), quote_ident(c)])
        .collect::<Vec<_>>()
        .join(",");
    let filter = spec
        .filter
        .map(|v| format!(" WHERE {v}"))
        .unwrap_or_default();
    format!(
        "SELECT json_object({args}) FROM {}{filter} ORDER BY id",
        quote_ident(spec.name)
    )
}

fn path_is_inside(path: &Path, root: &Path) -> bool {
    let (Ok(path), Ok(root)) = (path.canonicalize(), root.canonicalize()) else {
        return false;
    };
    path.starts_with(root)
}

async fn add_derived_markdown(
    pool: &SqlitePool,
    payload: &mut Value,
    app_data: &Path,
) -> Result<(), String> {
    let Some(id) = payload.get("id").and_then(Value::as_str) else {
        return Ok(());
    };
    let path: Option<String> = sqlx::query_scalar(
        "SELECT extracted_text_path FROM documents WHERE id=? AND is_ai_artifact=0",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?
    .flatten();
    let Some(path) = path.map(PathBuf::from) else {
        return Ok(());
    };
    if !path_is_inside(&path, app_data)
        || !path
            .extension()
            .and_then(|v| v.to_str())
            .is_some_and(|v| v.eq_ignore_ascii_case("md"))
    {
        return Ok(());
    }
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() as usize > MAX_DERIVED_MD_BYTES {
        return Ok(());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    payload
        .as_object_mut()
        .ok_or("文档同步记录格式错误")?
        .insert("derived_md".into(), Value::String(content));
    Ok(())
}

async fn load_ledger(pool: &SqlitePool) -> Result<HashMap<(String, String), LedgerRow>, String> {
    let rows = sqlx::query_as::<_, LedgerRow>(
        "SELECT entity_type,record_id,content_hash,parent_hash,revision,origin_device_id,updated_at,deleted_at \
         FROM device_sync_records",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| ((row.entity_type.clone(), row.record_id.clone()), row))
        .collect())
}

async fn write_ledger(pool: &SqlitePool, summary: &RecordSummary) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO device_sync_records \
         (entity_type,record_id,content_hash,parent_hash,revision,origin_device_id,updated_at,deleted_at) \
         VALUES (?,?,?,?,?,?,?,?) ON CONFLICT(entity_type,record_id) DO UPDATE SET \
         content_hash=excluded.content_hash,parent_hash=excluded.parent_hash,revision=excluded.revision, \
         origin_device_id=excluded.origin_device_id,updated_at=excluded.updated_at,deleted_at=excluded.deleted_at",
    )
    .bind(&summary.entity_type)
    .bind(&summary.record_id)
    .bind(&summary.content_hash)
    .bind(&summary.parent_hash)
    .bind(summary.revision)
    .bind(&summary.origin_device_id)
    .bind(&summary.updated_at)
    .bind(&summary.deleted_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn track_payload(
    pool: &SqlitePool,
    ledger: &mut HashMap<(String, String), LedgerRow>,
    identity: &DeviceSyncIdentity,
    entity_type: &str,
    record_id: &str,
    payload: Value,
) -> Result<RecordPacket, String> {
    let hash = payload_hash(&payload)?;
    let key = (entity_type.to_string(), record_id.to_string());
    let summary = match ledger.get(&key) {
        Some(old) if old.content_hash == hash && old.deleted_at.is_none() => old.summary(),
        Some(old) => RecordSummary {
            entity_type: entity_type.into(),
            record_id: record_id.into(),
            content_hash: hash,
            parent_hash: Some(old.content_hash.clone()),
            revision: old.revision + 1,
            origin_device_id: identity.device_id.clone(),
            updated_at: now(),
            deleted_at: None,
        },
        None => RecordSummary {
            entity_type: entity_type.into(),
            record_id: record_id.into(),
            content_hash: hash,
            parent_hash: None,
            revision: 1,
            origin_device_id: identity.device_id.clone(),
            updated_at: now(),
            deleted_at: None,
        },
    };
    if ledger
        .get(&key)
        .is_none_or(|old| old.content_hash != summary.content_hash || old.deleted_at.is_some())
    {
        write_ledger(pool, &summary).await?;
        ledger.insert(
            key,
            LedgerRow {
                entity_type: summary.entity_type.clone(),
                record_id: summary.record_id.clone(),
                content_hash: summary.content_hash.clone(),
                parent_hash: summary.parent_hash.clone(),
                revision: summary.revision,
                origin_device_id: summary.origin_device_id.clone(),
                updated_at: summary.updated_at.clone(),
                deleted_at: None,
            },
        );
    }
    Ok(RecordPacket {
        summary,
        payload: Some(payload),
    })
}

fn shared_settings_from(
    settings: &crate::settings::Settings,
) -> Result<Map<String, Value>, String> {
    let value = serde_json::to_value(settings).map_err(|e| format!("序列化同步设置失败：{e}"))?;
    let mut object = value.as_object().cloned().ok_or("设置格式错误")?;
    for key in LOCAL_SETTING_KEYS {
        object.remove(*key);
    }
    Ok(object)
}

fn shared_settings() -> Result<Map<String, Value>, String> {
    shared_settings_from(&crate::settings::read_settings()?)
}

fn ticktick_state_path() -> Result<PathBuf, String> {
    Ok(crate::db::app_data_dir()
        .map_err(|e| e.to_string())?
        .join("ticktick_sync.json"))
}

fn read_ticktick_state() -> Result<Option<Value>, String> {
    let path = ticktick_state_path()?;
    if !path.is_file() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let value = serde_json::from_str(&raw).map_err(|e| format!("滴答同步配置格式错误：{e}"))?;
    Ok(Some(value))
}

fn is_shared_setting(key: &str) -> bool {
    !LOCAL_SETTING_KEYS.contains(&key)
}

/// 扫描当前业务状态。这里只做逻辑快照和增量头，不复制数据库文件。
pub async fn build_packets(
    pool: &SqlitePool,
    identity: &DeviceSyncIdentity,
) -> Result<Vec<RecordPacket>, String> {
    let mut ledger = load_ledger(pool).await?;
    let mut packets = Vec::new();
    let mut active = HashSet::new();
    let app_data = crate::db::app_data_dir().map_err(|e| e.to_string())?;

    for spec in TABLES {
        let columns = columns_for(pool, spec).await?;
        let sql = json_select_sql(spec, &columns);
        let rows = sqlx::query_scalar::<_, String>(&sql)
            .fetch_all(pool)
            .await
            .map_err(|e| format!("扫描同步表 {} 失败：{e}", spec.name))?;
        for json in rows {
            let mut payload: Value = serde_json::from_str(&json).map_err(|e| e.to_string())?;
            if spec.name == "documents" {
                add_derived_markdown(pool, &mut payload, &app_data).await?;
            }
            let id = payload
                .get("id")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("同步表 {} 的记录缺 id", spec.name))?
                .to_string();
            active.insert((spec.name.to_string(), id.clone()));
            packets
                .push(track_payload(pool, &mut ledger, identity, spec.name, &id, payload).await?);
        }
    }

    for (key, value) in shared_settings()? {
        active.insert(("setting".into(), key.clone()));
        let payload = serde_json::json!({"value": value});
        packets.push(track_payload(pool, &mut ledger, identity, "setting", &key, payload).await?);
    }

    let ticktick_payload = {
        let _guard = crate::ticktick::state_lock().lock().await;
        read_ticktick_state()?
    };
    if let Some(payload) = ticktick_payload {
        active.insert((TICKTICK_ENTITY.into(), TICKTICK_RECORD_ID.into()));
        packets.push(
            track_payload(
                pool,
                &mut ledger,
                identity,
                TICKTICK_ENTITY,
                TICKTICK_RECORD_ID,
                payload,
            )
            .await?,
        );
    }

    // 删除墓碑必须持续保留，错峰在线的第三台设备才不会永久错过删除。
    let tracked_types: HashSet<&str> = TABLES
        .iter()
        .map(|spec| spec.name)
        .chain(["setting", TICKTICK_ENTITY])
        .collect();
    let existing: Vec<_> = ledger.keys().cloned().collect();
    for key in existing {
        if !tracked_types.contains(key.0.as_str()) || active.contains(&key) {
            continue;
        }
        let Some(old) = ledger.get(&key) else {
            continue;
        };
        let summary = if old.deleted_at.is_some() {
            old.summary()
        } else {
            let stamp = now();
            let summary = RecordSummary {
                entity_type: key.0.clone(),
                record_id: key.1.clone(),
                content_hash: old.content_hash.clone(),
                parent_hash: Some(old.content_hash.clone()),
                revision: old.revision + 1,
                origin_device_id: identity.device_id.clone(),
                updated_at: stamp.clone(),
                deleted_at: Some(stamp),
            };
            write_ledger(pool, &summary).await?;
            summary
        };
        packets.push(RecordPacket {
            summary,
            payload: None,
        });
    }

    Ok(packets)
}

pub fn summaries(packets: &[RecordPacket]) -> Vec<RecordSummary> {
    packets
        .iter()
        .map(|packet| packet.summary.clone())
        .collect()
}

pub fn plan_response(
    local: &[RecordPacket],
    caller: &[RecordSummary],
) -> (Vec<RecordPacket>, Vec<RecordRef>) {
    let caller_map: HashMap<(&str, &str), &RecordSummary> = caller
        .iter()
        .map(|s| ((s.entity_type.as_str(), s.record_id.as_str()), s))
        .collect();
    let local_map: HashMap<(&str, &str), &RecordPacket> = local
        .iter()
        .map(|p| {
            (
                (p.summary.entity_type.as_str(), p.summary.record_id.as_str()),
                p,
            )
        })
        .collect();
    let mut packets = Vec::new();
    let mut used = 0usize;
    for packet in local.iter().filter(|packet| {
        caller_map
            .get(&(
                packet.summary.entity_type.as_str(),
                packet.summary.record_id.as_str(),
            ))
            .is_none_or(|remote| {
                remote.content_hash != packet.summary.content_hash
                    || remote.deleted_at != packet.summary.deleted_at
            })
    }) {
        let weight = packet
            .payload
            .as_ref()
            .and_then(|v| serde_json::to_vec(v).ok())
            .map_or(256, |v| v.len() + 512);
        if !packets.is_empty() && used.saturating_add(weight) > MAX_RECORD_BATCH_BYTES {
            break;
        }
        used = used.saturating_add(weight);
        packets.push(packet.clone());
    }
    let need = caller
        .iter()
        .filter(|remote| {
            local_map
                .get(&(remote.entity_type.as_str(), remote.record_id.as_str()))
                .is_none_or(|local| {
                    local.summary.content_hash != remote.content_hash
                        || local.summary.deleted_at != remote.deleted_at
                })
        })
        .map(RecordSummary::key)
        .collect();
    (packets, need)
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '_',
            _ => c,
        })
        .take(120)
        .collect()
}

fn bind_json<'q>(
    query: sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    value: &'q Value,
) -> sqlx::query::Query<'q, Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    match value {
        Value::Null => query.bind(Option::<String>::None),
        Value::Bool(v) => query.bind(if *v { 1_i64 } else { 0_i64 }),
        Value::Number(v) => {
            if let Some(n) = v.as_i64() {
                query.bind(n)
            } else if let Some(n) = v.as_u64() {
                query.bind(n as i64)
            } else {
                query.bind(v.as_f64().unwrap_or_default())
            }
        }
        Value::String(v) => query.bind(v),
        Value::Array(_) | Value::Object(_) => query.bind(value.to_string()),
    }
}

async fn write_derived_md(record_id: &str, payload: &mut Map<String, Value>) -> Result<(), String> {
    let Some(content) = payload
        .remove("derived_md")
        .and_then(|value| value.as_str().map(str::to_string))
    else {
        return Ok(());
    };
    if content.len() > MAX_DERIVED_MD_BYTES {
        return Err("派生 Markdown 超过 8MB".into());
    }
    let base = crate::db::app_data_dir()
        .map_err(|e| e.to_string())?
        .join("extracts")
        .join("device_sync");
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let path = base.join(format!("{}.md", safe_component(record_id)));
    std::fs::write(&path, &content).map_err(|e| e.to_string())?;
    payload.insert(
        "extracted_text_path".into(),
        Value::String(path.to_string_lossy().into_owned()),
    );
    payload.insert(
        "extracted_text_hash".into(),
        Value::String(sha256_hex(content.as_bytes())),
    );
    payload.insert("extraction_status".into(), Value::String("done".into()));
    Ok(())
}

async fn apply_setting(packet: &RecordPacket) -> Result<(), String> {
    if !is_shared_setting(&packet.summary.record_id) {
        return Err("收到禁止跨设备的本机设置".into());
    }
    let incoming = packet
        .payload
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|v| v.get("value"))
        .cloned()
        .ok_or("设置同步包格式错误")?;
    let current = crate::settings::read_settings()?;
    let mut value = serde_json::to_value(current).map_err(|e| e.to_string())?;
    value
        .as_object_mut()
        .ok_or("本机设置格式错误")?
        .insert(packet.summary.record_id.clone(), incoming);
    let merged: crate::settings::Settings =
        serde_json::from_value(value).map_err(|e| format!("合并同步设置失败：{e}"))?;
    crate::settings::write_settings(&merged)
}

async fn apply_payload(pool: &SqlitePool, packet: &RecordPacket) -> Result<(), String> {
    if payload_hash(packet.payload.as_ref().ok_or("同步记录缺正文")?)?
        != packet.summary.content_hash
    {
        return Err("业务记录内容哈希不一致".into());
    }
    if packet.summary.entity_type == "setting" {
        apply_setting(packet).await
    } else if packet.summary.entity_type == TICKTICK_ENTITY {
        let value = packet.payload.as_ref().ok_or("滴答同步记录缺正文")?;
        let state: crate::ticktick::state::TickTickState =
            serde_json::from_value(value.clone()).map_err(|e| format!("滴答同步状态无效：{e}"))?;
        let raw = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
        let _guard = crate::ticktick::state_lock().lock().await;
        std::fs::write(ticktick_state_path()?, raw).map_err(|e| e.to_string())
    } else {
        upsert_table_record_with_pool(pool, packet).await
    }
}

async fn upsert_table_record_with_pool(
    pool: &SqlitePool,
    packet: &RecordPacket,
) -> Result<(), String> {
    let spec = table_spec(&packet.summary.entity_type).ok_or("未知同步实体")?;
    let mut payload = packet
        .payload
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .ok_or("业务记录同步包格式错误")?;
    if payload.get("id").and_then(Value::as_str) != Some(packet.summary.record_id.as_str()) {
        return Err("业务记录 id 与同步头不一致".into());
    }
    for forbidden in spec.excluded {
        if payload.contains_key(*forbidden) {
            return Err(format!("同步记录包含禁止字段：{forbidden}"));
        }
    }
    let app_data = crate::db::app_data_dir().map_err(|e| e.to_string())?;
    let mut insert_only = HashSet::new();
    match spec.name {
        "cases" => {
            let path = app_data
                .join("device_sync/source_placeholders")
                .join(&packet.summary.record_id);
            payload.insert(
                "source_folder".into(),
                Value::String(path.to_string_lossy().into_owned()),
            );
            insert_only.insert("source_folder");
        }
        "documents" => {
            write_derived_md(&packet.summary.record_id, &mut payload).await?;
            let filename = payload
                .get("filename")
                .and_then(Value::as_str)
                .unwrap_or("远端材料");
            let path = app_data
                .join("device_sync/source_placeholders")
                .join(&packet.summary.record_id)
                .join(safe_component(filename));
            payload.insert(
                "source_path".into(),
                Value::String(path.to_string_lossy().into_owned()),
            );
            payload.insert("missing".into(), Value::Number(1.into()));
            insert_only.insert("source_path");
            insert_only.insert("missing");
        }
        _ => {}
    }
    if spec.name == "chat_tasks"
        && payload
            .get("status")
            .and_then(Value::as_str)
            .is_some_and(|v| matches!(v, "planning" | "executing" | "synthesizing" | "verifying"))
    {
        payload.insert("status".into(), Value::String("failed".into()));
        payload.insert(
            "error_short".into(),
            Value::String("任务仍在另一台设备执行；完成结果会自动同步".into()),
        );
    }
    let columns: Vec<String> = payload.keys().cloned().collect();
    let placeholders = std::iter::repeat_n("?", columns.len())
        .collect::<Vec<_>>()
        .join(",");
    let updates = columns
        .iter()
        .filter(|c| c.as_str() != "id" && !insert_only.contains(c.as_str()))
        .map(|c| format!("{}=excluded.{}", quote_ident(c), quote_ident(c)))
        .collect::<Vec<_>>()
        .join(",");
    let conflict = if updates.is_empty() {
        "DO NOTHING".to_string()
    } else {
        format!("DO UPDATE SET {updates}")
    };
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({placeholders}) ON CONFLICT(id) {conflict}",
        quote_ident(spec.name),
        columns
            .iter()
            .map(|c| quote_ident(c))
            .collect::<Vec<_>>()
            .join(",")
    );
    let mut query = sqlx::query(&sql);
    for column in &columns {
        query = bind_json(query, payload.get(column).ok_or("同步列缺值")?);
    }
    query
        .execute(pool)
        .await
        .map_err(|e| format!("写入 {} 失败：{e}", spec.name))?;
    Ok(())
}

async fn delete_record(pool: &SqlitePool, summary: &RecordSummary) -> Result<(), String> {
    if summary.entity_type == "setting" {
        if !is_shared_setting(&summary.record_id) {
            return Ok(());
        }
        let current = crate::settings::read_settings()?;
        let mut value = serde_json::to_value(current).map_err(|e| e.to_string())?;
        value
            .as_object_mut()
            .ok_or("本机设置格式错误")?
            .remove(&summary.record_id);
        let merged = serde_json::from_value(value).map_err(|e| e.to_string())?;
        return crate::settings::write_settings(&merged);
    }
    if summary.entity_type == TICKTICK_ENTITY {
        let _guard = crate::ticktick::state_lock().lock().await;
        let path = ticktick_state_path()?;
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
        }
        return Ok(());
    }
    let spec = table_spec(&summary.entity_type).ok_or("未知同步实体")?;
    sqlx::query(&format!(
        "DELETE FROM {} WHERE id=?",
        quote_ident(spec.name)
    ))
    .bind(&summary.record_id)
    .execute(pool)
    .await
    .map_err(|e| format!("删除 {} 失败：{e}", spec.name))?;
    Ok(())
}

fn remote_wins(remote: &RecordSummary, local: &LedgerRow, primary_device_id: &str) -> bool {
    if matches!(remote.entity_type.as_str(), "setting" | TICKTICK_ENTITY) {
        let remote_is_primary = remote.origin_device_id == primary_device_id;
        let local_is_primary = local.origin_device_id == primary_device_id;
        if remote_is_primary != local_is_primary {
            return remote_is_primary;
        }
    }
    (
        remote.revision,
        remote.updated_at.as_str(),
        remote.origin_device_id.as_str(),
        remote.content_hash.as_str(),
    ) > (
        local.revision,
        local.updated_at.as_str(),
        local.origin_device_id.as_str(),
        local.content_hash.as_str(),
    )
}

pub async fn apply_packets(
    pool: &SqlitePool,
    identity: &DeviceSyncIdentity,
    packets: &[RecordPacket],
) -> Result<RecordApplyReport, String> {
    let mut report = RecordApplyReport::default();
    let mut ledger = load_ledger(pool).await?;
    let mut ordered = packets.to_vec();
    ordered.sort_by_key(|packet| {
        let rank = table_rank(&packet.summary.entity_type);
        if packet.summary.deleted_at.is_some() {
            usize::MAX - rank
        } else {
            rank
        }
    });
    for packet in &ordered {
        let key = (
            packet.summary.entity_type.clone(),
            packet.summary.record_id.clone(),
        );
        let result: Result<bool, String> = async {
            let local = ledger.get(&key);
            if local.is_some_and(|v| {
                v.content_hash == packet.summary.content_hash
                    && v.deleted_at == packet.summary.deleted_at
            }) {
                return Ok(false);
            }
            let is_fast_forward = local.is_none_or(|v| {
                packet.summary.parent_hash.as_deref() == Some(v.content_hash.as_str())
                    || (v.deleted_at.is_some() && packet.summary.deleted_at.is_none())
            });
            let local_is_successor = local.is_some_and(|v| {
                v.parent_hash.as_deref() == Some(packet.summary.content_hash.as_str())
            });
            if local_is_successor {
                return Ok(false);
            }
            let concurrent = local.is_some() && !is_fast_forward;
            if concurrent
                && !remote_wins(
                    &packet.summary,
                    local.expect("checked"),
                    identity.primary_device_id(),
                )
            {
                report.conflicts += 1;
                return Ok(false);
            }
            if packet.summary.deleted_at.is_some() {
                // 编辑优先于无共同父版本的删除，避免另一台机器静默删掉仍在处理的案件。
                if concurrent && local.is_some_and(|v| v.deleted_at.is_none()) {
                    report.conflicts += 1;
                    return Ok(false);
                }
                delete_record(pool, &packet.summary).await?;
            } else {
                apply_payload(pool, packet).await?;
            }
            write_ledger(pool, &packet.summary).await?;
            Ok(true)
        }
        .await;
        match result {
            Ok(true) => {
                report.applied += 1;
                ledger.insert(
                    key,
                    LedgerRow {
                        entity_type: packet.summary.entity_type.clone(),
                        record_id: packet.summary.record_id.clone(),
                        content_hash: packet.summary.content_hash.clone(),
                        parent_hash: packet.summary.parent_hash.clone(),
                        revision: packet.summary.revision,
                        origin_device_id: packet.summary.origin_device_id.clone(),
                        updated_at: packet.summary.updated_at.clone(),
                        deleted_at: packet.summary.deleted_at.clone(),
                    },
                );
            }
            Ok(false) => report.unchanged += 1,
            Err(error) => report.errors.push(format!(
                "{}/{}：{error}",
                packet.summary.entity_type, packet.summary.record_id
            )),
        }
    }
    Ok(report)
}

pub async fn remember_device(
    pool: &SqlitePool,
    device_id: &str,
    device_name: &str,
    platform: &str,
) -> Result<(), String> {
    let stamp = now();
    sqlx::query(
        "INSERT INTO device_sync_devices(device_id,device_name,platform,first_seen_at,last_seen_at) \
         VALUES(?,?,?,?,?) ON CONFLICT(device_id) DO UPDATE SET \
         device_name=excluded.device_name,platform=excluded.platform,last_seen_at=excluded.last_seen_at,revoked_at=NULL",
    )
    .bind(device_id)
    .bind(device_name)
    .bind(platform)
    .bind(&stamp)
    .bind(&stamp)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn record_count(pool: &SqlitePool) -> Result<i64, String> {
    sqlx::query_scalar("SELECT COUNT(*) FROM device_sync_records WHERE deleted_at IS NULL")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}

pub async fn device_count(pool: &SqlitePool) -> Result<i64, String> {
    sqlx::query_scalar("SELECT COUNT(*) FROM device_sync_devices WHERE revoked_at IS NULL")
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())
}
