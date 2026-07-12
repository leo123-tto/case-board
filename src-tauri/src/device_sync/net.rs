//! 个人空间网络层：独立 mDNS 域 + AEAD 加密 HTTP 消息。

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::store::{self, ApplyReport, IndexRequest, IndexResponse, PushRequest};
use super::{decrypt, encrypt, DeviceSyncIdentity, EncryptedEnvelope};
use super::{source, workspace};
use crate::settings::{read_settings, write_settings};

pub const SERVICE_TYPE: &str = "_caseboard-self._tcp.local.";
const MAX_BODY: usize = 48 * 1024 * 1024;
const CONN_TIMEOUT: Duration = Duration::from_secs(30);

fn sync_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveredDeviceGroup {
    pub group_id: String,
    pub group_name: String,
    pub device_id: String,
    pub device_name: String,
    pub can_join: bool,
}

#[derive(Debug, Clone)]
struct PeerAddr {
    group_id: String,
    group_name: String,
    device_id: String,
    device_name: String,
    platform: String,
    can_join: bool,
    ip: String,
    port: u16,
}

#[derive(Debug, Serialize, Deserialize)]
struct JoinRequest {
    device_id: String,
    device_name: String,
    platform: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct JoinResponse {
    group_name: String,
    group_secret: String,
    primary_device_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub peers_found: usize,
    pub peers_synced: usize,
    pub sent: usize,
    pub received: usize,
    pub conflicts: usize,
    pub pending_cases: usize,
    pub records: usize,
    pub errors: Vec<String>,
}

pub struct DeviceSyncNet {
    mdns: mdns_sd::ServiceDaemon,
    fullname: String,
    listener_task: tokio::task::JoinHandle<()>,
    periodic_task: tokio::task::JoinHandle<()>,
    pub port: u16,
}

impl DeviceSyncNet {
    pub fn shutdown(self) {
        let _ = self.mdns.unregister(&self.fullname);
        let _ = self.mdns.shutdown();
        self.listener_task.abort();
        self.periodic_task.abort();
    }
}

pub async fn start(pool: SqlitePool) -> Result<DeviceSyncNet, String> {
    let settings = read_settings()?;
    if !settings.device_sync_enabled {
        return Err("个人设备同步未启用".into());
    }
    let identity = settings.device_sync.ok_or("尚未配置个人设备组")?;
    workspace::remember_device(
        &pool,
        &identity.device_id,
        &identity.device_name,
        std::env::consts::OS,
    )
    .await?;
    let listener = TcpListener::bind(("0.0.0.0", 0))
        .await
        .map_err(|e| format!("绑定个人设备同步端口失败：{e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let accept_pool = pool.clone();
    let listener_task = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let p = accept_pool.clone();
                    tokio::spawn(async move {
                        let _ = tokio::time::timeout(CONN_TIMEOUT, handle_conn(stream, p)).await;
                    });
                }
                Err(e) => {
                    crate::dlog!("[device-sync] accept 失败：{e}");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });

    let mdns = mdns_sd::ServiceDaemon::new().map_err(|e| format!("mDNS 启动失败：{e}"))?;
    let mut props = HashMap::new();
    props.insert("gid".into(), identity.group_id.clone());
    props.insert("gname".into(), identity.group_name.clone());
    props.insert("did".into(), identity.device_id.clone());
    props.insert("dname".into(), identity.device_name.clone());
    props.insert("os".into(), std::env::consts::OS.into());
    props.insert(
        "join".into(),
        if identity.pairing_code.is_some() {
            "1"
        } else {
            "0"
        }
        .into(),
    );
    let host = format!(
        "caseboard-self-{}.local.",
        &identity.device_id[..8.min(identity.device_id.len())]
    );
    let service =
        mdns_sd::ServiceInfo::new(SERVICE_TYPE, &identity.device_id, &host, "", port, props)
            .map_err(|e| format!("mDNS 服务信息构建失败：{e}"))?
            .enable_addr_auto();
    let fullname = service.get_fullname().to_string();
    mdns.register(service)
        .map_err(|e| format!("mDNS 注册失败：{e}"))?;

    let periodic_pool = pool.clone();
    let periodic_task = tokio::spawn(async move {
        loop {
            if let Err(e) = sync_round(&periodic_pool).await {
                let _ = store::set_state(&periodic_pool, "last_error", &e).await;
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
    Ok(DeviceSyncNet {
        mdns,
        fullname,
        listener_task,
        periodic_task,
        port,
    })
}

struct HttpReq {
    path: String,
    body: Vec<u8>,
}

async fn handle_conn(mut stream: TcpStream, pool: SqlitePool) {
    let Ok(req) = read_http_request(&mut stream).await else {
        let _ = write_http(&mut stream, 400, "{\"error\":\"bad request\"}").await;
        return;
    };
    let (status, body) = route(&req, &pool).await;
    let _ = write_http(&mut stream, status, &body).await;
}

async fn read_http_request(stream: &mut TcpStream) -> Result<HttpReq, String> {
    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 8192];
    let header_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        if buf.len() > 64 * 1024 {
            return Err("HTTP 头过大".into());
        }
        let n = stream.read(&mut tmp).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("连接提前关闭".into());
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..header_end]);
    let mut lines = head.lines();
    let mut request_line = lines.next().unwrap_or_default().split_whitespace();
    if request_line.next() != Some("POST") {
        return Err("只支持 POST".into());
    }
    let path = request_line.next().unwrap_or_default().to_string();
    let mut content_len = None;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_len = v.trim().parse::<usize>().ok();
            }
        }
    }
    let content_len = content_len.ok_or("缺 Content-Length")?;
    if content_len > MAX_BODY {
        return Err("同步消息超过 32MB".into());
    }
    let mut body = buf[header_end + 4..].to_vec();
    while body.len() < content_len {
        let n = stream.read(&mut tmp).await.map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("body 不完整".into());
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(content_len);
    Ok(HttpReq { path, body })
}

async fn write_http(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), String> {
    let text = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Error",
    };
    let response = format!(
        "HTTP/1.1 {status} {text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    stream.flush().await.map_err(|e| e.to_string())
}

fn error_json(message: &str) -> String {
    serde_json::json!({"error": message}).to_string()
}

async fn route(req: &HttpReq, pool: &SqlitePool) -> (u16, String) {
    match req.path.as_str() {
        "/device/join" => handle_join(req, pool).await,
        "/device/index" => handle_index(req, pool).await,
        "/device/push" => handle_push(req, pool).await,
        _ => (404, error_json("not found")),
    }
}

async fn handle_join(req: &HttpReq, pool: &SqlitePool) -> (u16, String) {
    let Ok(env) = serde_json::from_slice::<EncryptedEnvelope>(&req.body) else {
        return (400, error_json("bad envelope"));
    };
    let Ok(mut settings) = read_settings() else {
        return (403, error_json("no settings"));
    };
    let Some(identity) = settings.device_sync.as_mut() else {
        return (403, error_json("device sync not configured"));
    };
    if env.group_id != identity.group_id {
        return (403, error_json("group mismatch"));
    }
    let Some(code) = identity.pairing_code.clone() else {
        return (403, error_json("当前设备未开放配对"));
    };
    let Ok(join) = decrypt::<JoinRequest>(&code, &env) else {
        return (403, error_json("配对口令不正确"));
    };
    if join.device_id.trim().is_empty() || join.device_name.trim().is_empty() {
        return (400, error_json("设备信息不完整"));
    }
    let response = JoinResponse {
        group_name: identity.group_name.clone(),
        group_secret: identity.group_secret.clone(),
        primary_device_id: identity.primary_device_id().to_string(),
    };
    if let Err(e) =
        workspace::remember_device(pool, &join.device_id, &join.device_name, &join.platform).await
    {
        return (400, error_json(&e));
    }
    let out = match encrypt(&code, &identity.group_id, &identity.device_id, &response) {
        Ok(v) => v,
        Err(e) => return (400, error_json(&e)),
    };
    identity.pairing_code = Some(super::gen_pairing_code());
    if let Err(e) = write_settings(&settings) {
        return (400, error_json(&e));
    }
    (200, serde_json::to_string(&out).unwrap_or_default())
}

async fn authenticated_envelope(
    req: &HttpReq,
) -> Result<(DeviceSyncIdentity, EncryptedEnvelope), String> {
    let env: EncryptedEnvelope = serde_json::from_slice(&req.body).map_err(|_| "bad envelope")?;
    let settings = read_settings()?;
    if !settings.device_sync_enabled {
        return Err("个人设备同步未启用".into());
    }
    let identity = settings.device_sync.ok_or("尚未配置个人设备组")?;
    if env.group_id != identity.group_id || env.device_id == identity.device_id {
        return Err("设备组不匹配".into());
    }
    Ok((identity, env))
}

async fn handle_index(req: &HttpReq, pool: &SqlitePool) -> (u16, String) {
    let Ok((identity, env)) = authenticated_envelope(req).await else {
        return (403, error_json("认证失败"));
    };
    let incoming: IndexRequest = match decrypt(&identity.group_secret, &env) {
        Ok(v) => v,
        Err(e) => return (403, error_json(&e)),
    };
    if incoming.from_device_id != env.device_id
        || incoming
            .source_summaries
            .iter()
            .any(|source| source.origin_device_id != env.device_id)
    {
        return (403, error_json("设备身份不一致"));
    }
    if let Err(e) = workspace::remember_device(
        pool,
        &incoming.from_device_id,
        &incoming.from_device_name,
        &incoming.from_platform,
    )
    .await
    {
        return (400, error_json(&e));
    }
    let local = match store::build_packets(pool, &identity).await {
        Ok(v) => v,
        Err(e) => return (400, error_json(&e)),
    };
    let local_records = match workspace::build_packets(pool, &identity).await {
        Ok(v) => v,
        Err(e) => return (400, error_json(&e)),
    };
    let mut response = store::plan_response(&local, &incoming.summaries, &identity.device_id);
    let (record_packets, need_records) =
        workspace::plan_response(&local_records, &incoming.record_summaries);
    response.record_packets = record_packets;
    response.need_records_from_caller = need_records;
    response.source_needs =
        match source::plan_needs(pool, &identity, &incoming.source_summaries).await {
            Ok(v) => v,
            Err(e) => return (400, error_json(&e)),
        };
    let _ = store::set_state(pool, "last_sync_at", &chrono::Utc::now().to_rfc3339()).await;
    let _ = store::set_state(pool, "last_error", "").await;
    let out = match encrypt(
        &identity.group_secret,
        &identity.group_id,
        &identity.device_id,
        &response,
    ) {
        Ok(v) => v,
        Err(e) => return (400, error_json(&e)),
    };
    (200, serde_json::to_string(&out).unwrap_or_default())
}

async fn handle_push(req: &HttpReq, pool: &SqlitePool) -> (u16, String) {
    let Ok((identity, env)) = authenticated_envelope(req).await else {
        return (403, error_json("认证失败"));
    };
    let incoming: PushRequest = match decrypt(&identity.group_secret, &env) {
        Ok(v) => v,
        Err(e) => return (403, error_json(&e)),
    };
    if incoming.from_device_id != env.device_id
        || incoming
            .source_chunks
            .iter()
            .any(|chunk| chunk.summary.origin_device_id != env.device_id)
    {
        return (403, error_json("设备身份不一致"));
    }
    let record_report =
        match workspace::apply_packets(pool, &identity, &incoming.record_packets).await {
            Ok(v) => v,
            Err(e) => return (400, error_json(&e)),
        };
    let source_report = match source::apply_chunks(pool, &identity, &incoming.source_chunks).await {
        Ok(v) => v,
        Err(e) => return (400, error_json(&e)),
    };
    let mut report = match store::apply_packets(pool, &identity, &incoming.packets).await {
        Ok(v) => v,
        Err(e) => return (400, error_json(&e)),
    };
    report.applied += record_report.applied;
    report.unchanged += record_report.unchanged;
    report.conflicts += record_report.conflicts;
    report.errors.extend(record_report.errors);
    report.applied += source_report.completed;
    report.errors.extend(source_report.errors);
    let _ = store::set_state(pool, "last_sync_at", &chrono::Utc::now().to_rfc3339()).await;
    let _ = store::set_state(pool, "last_error", "").await;
    let out = match encrypt(
        &identity.group_secret,
        &identity.group_id,
        &identity.device_id,
        &report,
    ) {
        Ok(v) => v,
        Err(e) => return (400, error_json(&e)),
    };
    (200, serde_json::to_string(&out).unwrap_or_default())
}

async fn browse_peers(timeout_ms: u64) -> Result<Vec<PeerAddr>, String> {
    let mdns = mdns_sd::ServiceDaemon::new().map_err(|e| format!("mDNS 启动失败：{e}"))?;
    let receiver = mdns
        .browse(SERVICE_TYPE)
        .map_err(|e| format!("mDNS 浏览失败：{e}"))?;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
    let mut peers = Vec::new();
    loop {
        let now = tokio::time::Instant::now();
        if now >= deadline {
            break;
        }
        let event = tokio::task::block_in_place(|| receiver.recv_timeout(deadline - now));
        match event {
            Ok(mdns_sd::ServiceEvent::ServiceResolved(info)) => {
                let get = |key: &str| {
                    info.get_property_val_str(key)
                        .map(str::to_string)
                        .unwrap_or_default()
                };
                let ip = info
                    .get_addresses()
                    .iter()
                    .find(|a| a.is_ipv4())
                    .map(|a| a.to_string());
                if let Some(ip) = ip {
                    let peer = PeerAddr {
                        group_id: get("gid"),
                        group_name: get("gname"),
                        device_id: get("did"),
                        device_name: get("dname"),
                        platform: get("os"),
                        can_join: get("join") == "1",
                        ip,
                        port: info.get_port(),
                    };
                    if !peer.group_id.is_empty()
                        && !peers
                            .iter()
                            .any(|p: &PeerAddr| p.device_id == peer.device_id)
                    {
                        peers.push(peer);
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    let _ = mdns.shutdown();
    Ok(peers)
}

pub async fn discover_groups() -> Result<Vec<DiscoveredDeviceGroup>, String> {
    Ok(browse_peers(2500)
        .await?
        .into_iter()
        .filter(|p| p.can_join)
        .map(|p| DiscoveredDeviceGroup {
            group_id: p.group_id,
            group_name: p.group_name,
            device_id: p.device_id,
            device_name: p.device_name,
            can_join: p.can_join,
        })
        .collect())
}

async fn post_envelope(
    client: &reqwest::Client,
    url: &str,
    env: &EncryptedEnvelope,
) -> Result<EncryptedEnvelope, String> {
    let response = client
        .post(url)
        .json(env)
        .send()
        .await
        .map_err(|e| format!("连接失败：{e}"))?;
    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        let message = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v.get("error").and_then(|v| v.as_str()).map(str::to_string))
            .unwrap_or_else(|| format!("HTTP {status}"));
        return Err(message);
    }
    serde_json::from_str(&text).map_err(|e| format!("响应格式错误：{e}"))
}

pub async fn join_group(
    group_id: &str,
    pairing_code: &str,
    device_name: &str,
) -> Result<DeviceSyncIdentity, String> {
    let peers = browse_peers(3000).await?;
    let peer = peers
        .iter()
        .find(|p| p.group_id == group_id && p.can_join)
        .ok_or("没有找到开放配对的设备；请确认两台电脑在同一局域网且 CaseBoard 已打开")?;
    let device_id = uuid::Uuid::new_v4().to_string();
    let request = JoinRequest {
        device_id: device_id.clone(),
        device_name: device_name.trim().to_string(),
        platform: std::env::consts::OS.to_string(),
    };
    let env = encrypt(pairing_code, group_id, &device_id, &request)?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let url = format!("http://{}:{}/device/join", peer.ip, peer.port);
    let response_env = post_envelope(&client, &url, &env).await?;
    let response: JoinResponse = decrypt(pairing_code, &response_env)?;
    Ok(DeviceSyncIdentity {
        group_id: group_id.into(),
        group_name: response.group_name,
        group_secret: response.group_secret,
        device_id,
        device_name: device_name.trim().into(),
        primary_device_id: Some(response.primary_device_id),
        pairing_code: None,
    })
}

pub async fn sync_round(pool: &SqlitePool) -> Result<SyncReport, String> {
    let _guard = sync_lock().lock().await;
    let settings = read_settings()?;
    if !settings.device_sync_enabled {
        return Ok(SyncReport::default());
    }
    let identity = settings.device_sync.ok_or("尚未配置个人设备组")?;
    let local = store::build_packets(pool, &identity).await?;
    let local_records = workspace::build_packets(pool, &identity).await?;
    let local_sources = source::build_candidates(pool, &identity).await?;
    let peers: Vec<_> = browse_peers(1500)
        .await?
        .into_iter()
        .filter(|p| p.group_id == identity.group_id && p.device_id != identity.device_id)
        .collect();
    let mut report = SyncReport {
        peers_found: peers.len(),
        ..Default::default()
    };
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    for peer in peers {
        workspace::remember_device(pool, &peer.device_id, &peer.device_name, &peer.platform)
            .await?;
        let result: Result<(usize, ApplyReport), String> = async {
            let index = IndexRequest {
                from_device_id: identity.device_id.clone(),
                from_device_name: identity.device_name.clone(),
                from_platform: std::env::consts::OS.to_string(),
                summaries: store::summaries(&local),
                record_summaries: workspace::summaries(&local_records),
                source_summaries: source::summaries(&local_sources),
            };
            let env = encrypt(
                &identity.group_secret,
                &identity.group_id,
                &identity.device_id,
                &index,
            )?;
            let url = format!("http://{}:{}/device/index", peer.ip, peer.port);
            let response_env = post_envelope(&client, &url, &env).await?;
            let response: IndexResponse = decrypt(&identity.group_secret, &response_env)?;
            // 先落案件/文档逻辑记录，再落 Markdown，最后重试依赖 artifact FK 的子记录。
            let first_record_apply =
                workspace::apply_packets(pool, &identity, &response.record_packets).await?;
            let mut applied = store::apply_packets(pool, &identity, &response.packets).await?;
            let final_record_apply =
                workspace::apply_packets(pool, &identity, &response.record_packets).await?;
            applied.applied += first_record_apply.applied + final_record_apply.applied;
            applied.unchanged += first_record_apply.unchanged + final_record_apply.unchanged;
            applied.conflicts += first_record_apply.conflicts + final_record_apply.conflicts;
            applied.errors.extend(final_record_apply.errors);
            let needed_records: Vec<_> = local_records
                .iter()
                .filter(|packet| {
                    response
                        .need_records_from_caller
                        .contains(&packet.summary.key())
                })
                .cloned()
                .collect();
            let record_sent = needed_records.len();
            let mut record_chunks: Vec<Vec<workspace::RecordPacket>> = Vec::new();
            for packet in needed_records {
                let weight = packet
                    .payload
                    .as_ref()
                    .and_then(|v| serde_json::to_vec(v).ok())
                    .map_or(256, |v| v.len() + 512);
                let needs_new = record_chunks.last().is_some_and(|chunk| {
                    let used: usize = chunk
                        .iter()
                        .map(|p| {
                            p.payload
                                .as_ref()
                                .and_then(|v| serde_json::to_vec(v).ok())
                                .map_or(256, |v| v.len() + 512)
                        })
                        .sum();
                    !chunk.is_empty()
                        && used.saturating_add(weight) > workspace::MAX_RECORD_BATCH_BYTES
                });
                if record_chunks.is_empty() || needs_new {
                    record_chunks.push(Vec::new());
                }
                if let Some(chunk) = record_chunks.last_mut() {
                    chunk.push(packet);
                }
            }
            // 第一遍先确保父记录存在；此时 artifact 外键可能尚未到达，错误延后到第二遍判断。
            for chunk in &record_chunks {
                let push = PushRequest {
                    from_device_id: identity.device_id.clone(),
                    packets: Vec::new(),
                    record_packets: chunk.clone(),
                    source_chunks: Vec::new(),
                };
                let env = encrypt(
                    &identity.group_secret,
                    &identity.group_id,
                    &identity.device_id,
                    &push,
                )?;
                let url = format!("http://{}:{}/device/push", peer.ip, peer.port);
                let response_env = post_envelope(&client, &url, &env).await?;
                let _: ApplyReport = decrypt(&identity.group_secret, &response_env)?;
            }
            let needed: Vec<_> = local
                .iter()
                .filter(|p| response.need_from_caller.contains(&p.summary.artifact_id))
                .cloned()
                .collect();
            let sent = needed.len() + record_sent;
            let mut chunks: Vec<Vec<store::ArtifactPacket>> = Vec::new();
            for packet in needed {
                let weight = packet.content.as_ref().map_or(256, |v| v.len() + 1024);
                let needs_new = chunks.last().is_some_and(|chunk| {
                    let used: usize = chunk
                        .iter()
                        .map(|p| p.content.as_ref().map_or(256, |v| v.len() + 1024))
                        .sum();
                    !chunk.is_empty() && used.saturating_add(weight) > store::MAX_BATCH_PLAIN_BYTES
                });
                if chunks.is_empty() || needs_new {
                    chunks.push(Vec::new());
                }
                if let Some(chunk) = chunks.last_mut() {
                    chunk.push(packet);
                }
            }
            for chunk in chunks {
                let push = PushRequest {
                    from_device_id: identity.device_id.clone(),
                    packets: chunk,
                    record_packets: Vec::new(),
                    source_chunks: Vec::new(),
                };
                let env = encrypt(
                    &identity.group_secret,
                    &identity.group_id,
                    &identity.device_id,
                    &push,
                )?;
                let url = format!("http://{}:{}/device/push", peer.ip, peer.port);
                let response_env = post_envelope(&client, &url, &env).await?;
                let remote_report: ApplyReport = decrypt(&identity.group_secret, &response_env)?;
                applied.errors.extend(remote_report.errors);
            }
            // artifact 已落地后再发一次业务记录，保住 chat/artifact、日志/来源文档等关联。
            for chunk in &record_chunks {
                let push = PushRequest {
                    from_device_id: identity.device_id.clone(),
                    packets: Vec::new(),
                    record_packets: chunk.clone(),
                    source_chunks: Vec::new(),
                };
                let env = encrypt(
                    &identity.group_secret,
                    &identity.group_id,
                    &identity.device_id,
                    &push,
                )?;
                let url = format!("http://{}:{}/device/push", peer.ip, peer.port);
                let response_env = post_envelope(&client, &url, &env).await?;
                let remote_report: ApplyReport = decrypt(&identity.group_secret, &response_env)?;
                applied.errors.extend(remote_report.errors);
            }
            let mut source_sent = 0usize;
            if !identity.is_primary() && peer.device_id == identity.primary_device_id() {
                let needed_refs: std::collections::HashSet<_> = response
                    .source_needs
                    .iter()
                    .map(|need| need.source.clone())
                    .collect();
                for candidate in &local_sources {
                    if !needed_refs.contains(&candidate.summary.key()) {
                        source::mark_uploaded(pool, &candidate.summary.key()).await?;
                    }
                }
                for need in &response.source_needs {
                    let Some(candidate) = local_sources
                        .iter()
                        .find(|candidate| candidate.summary.key() == need.source)
                    else {
                        continue;
                    };
                    let mut offset = need.offset;
                    let mut first_chunk = true;
                    while offset < candidate.summary.size_bytes
                        || (first_chunk && candidate.summary.size_bytes == 0)
                    {
                        first_chunk = false;
                        let chunk = source::read_chunk(candidate, offset)?;
                        let chunk_len = base64::engine::general_purpose::STANDARD
                            .decode(&chunk.data_base64)
                            .map_err(|_| "本机源文件分块编码失败")?
                            .len() as u64;
                        let push = PushRequest {
                            from_device_id: identity.device_id.clone(),
                            packets: Vec::new(),
                            record_packets: Vec::new(),
                            source_chunks: vec![chunk],
                        };
                        let env = encrypt(
                            &identity.group_secret,
                            &identity.group_id,
                            &identity.device_id,
                            &push,
                        )?;
                        let url = format!("http://{}:{}/device/push", peer.ip, peer.port);
                        let response_env = post_envelope(&client, &url, &env).await?;
                        let remote_report: ApplyReport =
                            decrypt(&identity.group_secret, &response_env)?;
                        if !remote_report.errors.is_empty() {
                            return Err(remote_report.errors.join("；"));
                        }
                        offset = offset.saturating_add(chunk_len);
                    }
                    source::mark_uploaded(pool, &candidate.summary.key()).await?;
                    source_sent += 1;
                }
            }
            Ok((sent + source_sent, applied))
        }
        .await;
        match result {
            Ok((sent, applied)) => {
                report.peers_synced += 1;
                report.sent += sent;
                report.received += applied.applied;
                report.conflicts += applied.conflicts;
                report.pending_cases += applied.pending_cases;
                report.records += applied.applied;
                report.errors.extend(applied.errors);
            }
            Err(e) => report.errors.push(format!("{}：{e}", peer.device_name)),
        }
    }
    if report.peers_synced > 0 {
        let stamp = chrono::Utc::now().to_rfc3339();
        store::set_state(pool, "last_sync_at", &stamp).await?;
    }
    store::set_state(pool, "last_error", &report.errors.join("；")).await?;
    Ok(report)
}
