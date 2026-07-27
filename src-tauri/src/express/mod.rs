//! 快递查询(V0.3)。快递100 实时查询 API(`poll/query.do`,POST 表单 + MD5 签名)。
//! 支持 EMS(`ems`)/ 顺丰(`shunfeng`)等(com 编号)。需用户在设置填 customer + key
//! (申请 https://api.kuaidi100.com/,个人免费版约 50 次/天,无需企业资质)。
//! ⚠️ 顺丰 / 中通查询**必须带收寄件人手机号**(`phone`,后 4 位即可),否则快递100 报 408;
//! 其它快递 phone 选填。phone 随 TrackRecord 落本地,自动刷新时一并带上(否则刷新顺丰又 408)。
//!
//! 签名规则:`sign = 大写( MD5(param + key + customer) )`,param 是查询 JSON 字符串。
//! 错误透传真错(已知坑#8),不用固定文案。

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::credentials_bridge::{BridgeCredentialConsumer, PendingCredentialSource};

pub fn credential_source() -> PendingCredentialSource {
    PendingCredentialSource::pending(
        BridgeCredentialConsumer::CourierConnector,
        "settings:kuaidi100_key",
        "connector.kuaidi100",
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackNode {
    /// 节点时间(ftime 优先,退化到 time)
    pub time: String,
    /// 节点描述(如「【无锡市】已签收」)
    pub context: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExpressResult {
    pub com: String,
    pub num: String,
    /// 物流状态码:0在途 1揽收 2疑难 3签收 4退签 5派送 6退回…
    pub state: String,
    pub state_text: String,
    pub nodes: Vec<TrackNode>,
}

const QUERY_API: &str = "https://poll.kuaidi100.com/poll/query.do";

fn state_text(state: &str) -> &'static str {
    match state {
        "0" => "在途",
        "1" => "已揽收",
        "2" => "疑难",
        "3" => "已签收",
        "4" => "退签",
        "5" => "派送中",
        "6" => "退回",
        "7" => "转投",
        _ => "未知",
    }
}

/// 计算快递100 签名:大写 MD5(param + key + customer)。
fn sign(param: &str, key: &str, customer: &str) -> String {
    let digest = md5::compute(format!("{}{}{}", param, key, customer));
    format!("{:x}", digest).to_uppercase()
}

/// 构造查询 param JSON 字符串。`resultv2=4` 返回行政区划+高级状态;
/// phone 非空才带(顺丰/中通必填,否则 408;其它选填)。
fn build_param(com: &str, num: &str, phone: &str) -> String {
    let mut obj = serde_json::Map::new();
    obj.insert("com".into(), com.into());
    obj.insert("num".into(), num.trim().into());
    obj.insert("resultv2".into(), "4".into());
    if !phone.trim().is_empty() {
        obj.insert("phone".into(), phone.trim().into());
    }
    serde_json::Value::Object(obj).to_string()
}

/// 实时查询一个运单。`com` = 快递公司编号(ems / shunfeng …),`num` = 运单号,
/// `phone` = 收寄件人手机号(顺丰/中通必填,其它选填)。
pub async fn query(
    customer: &str,
    credential: &PendingCredentialSource,
    com: &str,
    num: &str,
    phone: &str,
) -> Result<ExpressResult, String> {
    if customer.trim().is_empty() {
        return Err("未配置快递100 customer / key,请到设置里填写(申请见 api.kuaidi100.com)".into());
    }
    if num.trim().is_empty() {
        return Err("运单号不能为空".into());
    }
    let param = build_param(com, num, phone);
    let material = credential.issue_material().await?;
    let signature = sign(&param, material.expose(), customer);
    drop(material);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {}", e))?;
    let resp = client
        .post(QUERY_API)
        .form(&[
            ("customer", customer),
            ("sign", signature.as_str()),
            ("param", param.as_str()),
        ])
        .send()
        .await
        .map_err(|e| format!("请求快递100失败: {}", e))?;
    let text = resp
        .text()
        .await
        .map_err(|e| format!("读取快递100响应失败: {}", e))?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
        format!(
            "快递100响应非 JSON: {} · {}",
            e,
            text.chars().take(200).collect::<String>()
        )
    })?;
    // 失败形如 {"result":false,"message":"参数错误","returnCode":"500"}
    if v.get("result").and_then(|r| r.as_bool()) == Some(false) {
        let msg = v
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("查询失败");
        return Err(format!("快递100: {}", msg));
    }
    let state = v
        .get("state")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let nodes: Vec<TrackNode> = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .map(|item| TrackNode {
                    time: item
                        .get("ftime")
                        .or_else(|| item.get("time"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string(),
                    context: item
                        .get("context")
                        .and_then(|c| c.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ExpressResult {
        com: com.to_string(),
        num: num.trim().to_string(),
        state_text: state_text(&state).to_string(),
        state,
        nodes,
    })
}

// ───────────── 持久化跟踪(本地 express_tracks.json,无需 DB migration) ─────────────

/// 一条被跟踪的快递记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackRecord {
    pub num: String,
    pub com: String,
    pub com_name: String,
    /// 收寄件人手机号(顺丰/中通查询必填);旧数据无此字段,默认空。
    #[serde(default)]
    pub phone: String,
    pub state: String,
    pub state_text: String,
    pub delivered: bool,
    pub nodes: Vec<TrackNode>,
    pub created_at: String,
    pub last_polled_at: String,
}

fn store_path() -> Result<std::path::PathBuf, String> {
    let dir = crate::db::app_data_dir().map_err(|e| format!("找不到数据目录: {}", e))?;
    Ok(dir.join("express_tracks.json"))
}

pub fn load_tracks() -> Vec<TrackRecord> {
    let Ok(p) = store_path() else {
        return vec![];
    };
    let Ok(s) = std::fs::read_to_string(&p) else {
        return vec![];
    };
    serde_json::from_str(&s).unwrap_or_default()
}

pub fn save_tracks(tracks: &[TrackRecord]) -> Result<(), String> {
    let p = store_path()?;
    let s = serde_json::to_string_pretty(tracks).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(&p, s).map_err(|e| format!("写入失败: {}", e))
}

/// state 以 "3" 开头(签收)视为已送达,停止自动刷新。
fn is_delivered(state: &str) -> bool {
    state.starts_with('3')
}

fn now_iso() -> String {
    chrono::Local::now().to_rfc3339()
}

fn days_since(iso: &str, now: chrono::DateTime<chrono::Local>) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|t| (now - t.with_timezone(&chrono::Local)).num_days())
        .unwrap_or(0)
}

fn hours_since(iso: &str, now: chrono::DateTime<chrono::Local>) -> i64 {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|t| (now - t.with_timezone(&chrono::Local)).num_hours())
        .unwrap_or(99999)
}

/// 查询并跟踪一个单号:实时查 → upsert 进本地列表 → 返回最新全列表(倒序)。
pub async fn query_and_track(
    customer: &str,
    credential: &PendingCredentialSource,
    com: &str,
    com_name: &str,
    num: &str,
    phone: &str,
) -> Result<Vec<TrackRecord>, String> {
    let r = query(customer, credential, com, num, phone).await?;
    let mut tracks = load_tracks();
    let now = now_iso();
    let delivered = is_delivered(&r.state);
    if let Some(t) = tracks.iter_mut().find(|t| t.num == r.num && t.com == com) {
        t.state = r.state;
        t.state_text = r.state_text;
        t.delivered = delivered;
        t.nodes = r.nodes;
        t.last_polled_at = now;
        t.com_name = com_name.to_string();
        if !phone.trim().is_empty() {
            t.phone = phone.trim().to_string();
        }
    } else {
        tracks.insert(
            0,
            TrackRecord {
                num: r.num,
                com: com.to_string(),
                com_name: com_name.to_string(),
                phone: phone.trim().to_string(),
                state: r.state,
                state_text: r.state_text,
                delivered,
                nodes: r.nodes,
                created_at: now.clone(),
                last_polled_at: now,
            },
        );
    }
    save_tracks(&tracks)?;
    Ok(tracks)
}

/// 刷新所有"在跟踪"的单号(未签收 + 30 天内 + 距上次轮询≥`min_hours` 小时)。
/// 同单号 40 天内重查免费,所以每天刷一次不额外花钱;已签收 / 超 30 天不刷。
pub async fn refresh_active(
    customer: &str,
    credential: &PendingCredentialSource,
    min_hours: i64,
) -> Result<Vec<TrackRecord>, String> {
    let mut tracks = load_tracks();
    let now = chrono::Local::now();
    let mut changed = false;
    // 先选出要刷新的 (com, num, phone),避免在 await 期间持有 tracks 借用
    let to_poll: Vec<(String, String, String)> = tracks
        .iter()
        .filter(|t| {
            !t.delivered
                && days_since(&t.created_at, now) <= 30
                && hours_since(&t.last_polled_at, now) >= min_hours
        })
        .map(|t| (t.com.clone(), t.num.clone(), t.phone.clone()))
        .collect();
    for (com, num, phone) in to_poll {
        if let Ok(r) = query(customer, credential, &com, &num, &phone).await {
            if let Some(t) = tracks.iter_mut().find(|t| t.com == com && t.num == num) {
                t.state = r.state.clone();
                t.state_text = r.state_text;
                t.delivered = is_delivered(&r.state);
                t.nodes = r.nodes;
                t.last_polled_at = now_iso();
                changed = true;
            }
        }
    }
    if changed {
        save_tracks(&tracks)?;
    }
    Ok(tracks)
}

/// 删除一个跟踪记录。
pub fn delete_track(num: &str) -> Result<Vec<TrackRecord>, String> {
    let mut tracks = load_tracks();
    tracks.retain(|t| t.num != num);
    save_tracks(&tracks)?;
    Ok(tracks)
}
