//! 滴答清单(dida365 / TickTick)双向同步 —— 本地状态。
//!
//! **公开功能**。**不建任何 SQLite migration、不碰 settings.rs**:API 口令 / 同步台账 / cutoff
//! 全落一个本地 JSON 文件 `<app_data_dir>/ticktick_sync.json`(不进 git,避开 migration
//! checksum 红线)。启用个人空间后，该文件作为端到端加密逻辑记录同步；同步账本不保存口令。
//!
//! 鉴权走「API 口令」(dida365 设置 → 账户与安全 → API 口令,dp_ 前缀的个人访问令牌),
//! 直接当 Bearer token 打 `/open/v1/` 接口 —— 免注册开发者应用、免 OAuth 授权、免刷新。
//!
//! 「独立滴答镜像」模型:在「独立」tab 里维护一份滴答某清单的镜像列表,完整双向、
//! 带完成状态,**不碰** 案件待办(case_todos)/ 首页日历(calendar_events)。

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickTickConfig {
    /// API 域:dida365 = https://api.dida365.com,国际版 = https://api.ticktick.com
    #[serde(default = "default_api_base")]
    pub api_base: String,
    /// 同步目标清单(project)。None = 未选。
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub project_name: Option<String>,
    /// 自动同步(每分钟 + 切回 App)。默认开。
    #[serde(default = "default_true")]
    pub auto_sync: bool,
}

fn default_api_base() -> String {
    "https://api.dida365.com".to_string()
}
fn default_true() -> bool {
    true
}

impl Default for TickTickConfig {
    fn default() -> Self {
        Self {
            api_base: default_api_base(),
            project_id: None,
            project_name: None,
            auto_sync: true,
        }
    }
}

/// 鉴权令牌。API 口令模型下只用 `access_token` 存用户粘贴的口令(长期有效、
/// 不过期、无刷新);`refresh_token`/`expires_at_ms` 保留为兼容字段,恒为 None/0。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickTickTokens {
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// 兼容字段:API 口令不过期,恒为 0。
    #[serde(default)]
    pub expires_at_ms: i64,
}

/// 镜像列表里的一条待办。本地 uuid 为主键,`ticktick_id` 是远端对应任务 id。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MirrorItem {
    pub id: String,
    #[serde(default)]
    pub ticktick_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub done: bool,
    /// ISO 日期或日期时间(可空)。
    #[serde(default)]
    pub due: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// 本地软删墓碑:下次同步把远端也删掉,删成功后真正移除本行。
    #[serde(default)]
    pub deleted: bool,
    /// 本地有未推送的改动(新建 / 改标题 / 勾完成 / 改日期)。
    #[serde(default)]
    pub dirty: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TickTickState {
    #[serde(default)]
    pub config: TickTickConfig,
    #[serde(default)]
    pub tokens: TickTickTokens,
    /// cutoff 展示用:连接成功的时间点(毫秒)。0 = 未连接。
    #[serde(default)]
    pub sync_enabled_at_ms: i64,
    /// 是否已建立基线。首次同步时把当时远端**已有任务 id** 全记入 `baseline_ids`(视为历史积压,
    /// 一律不拉),之后只拉「不在基线、也不在本地台账」的新任务。
    /// 用 id 集合而非时间戳比较 —— 滴答 `/project/{id}/data` 不保证返回 modifiedTime,
    /// 用时间戳会导致「永远拉不进任何新任务」的静默失败。
    #[serde(default)]
    pub baseline_captured: bool,
    #[serde(default)]
    pub baseline_ids: Vec<String>,
    #[serde(default)]
    pub last_sync_ms: i64,
    /// 最近一次连接/同步的错误(供前端展示),成功后清空。
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub items: Vec<MirrorItem>,
}

impl TickTickState {
    pub fn connected(&self) -> bool {
        self.tokens.access_token.is_some() && self.sync_enabled_at_ms > 0
    }
}

/// 状态文件绝对路径:`<app_data_dir>/ticktick_sync.json`。
pub fn state_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("取 app_data_dir 失败:{e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建数据目录失败:{e}"))?;
    Ok(dir.join("ticktick_sync.json"))
}

pub fn load(app: &AppHandle) -> Result<TickTickState, String> {
    let p = state_path(app)?;
    if !p.exists() {
        return Ok(TickTickState::default());
    }
    let raw = std::fs::read_to_string(&p).map_err(|e| format!("读取同步状态失败:{e}"))?;
    if raw.trim().is_empty() {
        return Ok(TickTickState::default());
    }
    serde_json::from_str(&raw).map_err(|e| format!("解析同步状态失败:{e}"))
}

pub fn save(app: &AppHandle, st: &TickTickState) -> Result<(), String> {
    let p = state_path(app)?;
    let raw = serde_json::to_string_pretty(st).map_err(|e| format!("序列化同步状态失败:{e}"))?;
    std::fs::write(&p, raw).map_err(|e| format!("写入同步状态失败:{e}"))
}

/// 解析滴答返回的时间(如 `2026-06-15T10:00:00.000+0000`)→ 毫秒。
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    use chrono::DateTime;
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S%.3f%z", "%Y-%m-%dT%H:%M:%S%z"] {
        if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
            return Some(dt.timestamp_millis());
        }
    }
    None
}
