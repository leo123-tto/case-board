//! 团队版 Phase 1:LAN 接力同步(gossip,人人都是节点,无枢纽无服务器)。
//!
//! 设计:`docs/提案-团队版-2026-06-10.md` §6。核心成立根基 = 数据模型:
//! **每个成员只写自己的快照**(单写者),合并规则 = 同一成员按 seq 新者胜
//! → 任意两台机器互传永远不冲突,不需要任何"权威中心"。
//!
//! 模块分工:
//! - `mod.rs`(本文件):身份/roster(成员+权限清单,团队长 HMAC 签发)/快照信封/合并规则
//! - `store.rs`:SQLite 存取(team_snapshots / team_state)+ 从 cases 表构建本人快照
//! - `net.rs`:mDNS 发现广播 + 内嵌极简 HTTP 服务(/exchange 接力互换、/join 入队)+ 同步轮
//!
//! 安全模型(老板 2026-06-10 拍板):配对码 + HMAC 挡**团队外**的人(同所隔壁团队);
//! 团队内是信任关系,分组权限=客户端默认不显示(不加密)。快照只到"案件登记表"粒度。

pub mod net;
pub mod store;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

// ============================================================================
// 身份(存 settings.json)
// ============================================================================

/// 本机的团队身份。None = 未入团。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TeamIdentity {
    pub team_id: String,
    pub team_name: String,
    /// 全队共享鉴权密钥(64 hex)。跟 API key 同级:只存本机。
    ///
    /// `serde(default)`:0.4.17 起前端设置副本被脱敏(不持有该字段),save_settings 回传的
    /// payload 会缺失它;后端保存时 team 整体以磁盘现值覆盖,空串不会被使用或落盘。
    /// 缺了 default 会让入过团队的用户所有设置保存直接反序列化失败(2026-07-27 真机)。
    #[serde(default)]
    pub team_secret: String,
    pub member_id: String,
    pub my_name: String,
    /// "leader" | "member"
    pub role: String,
    /// 6 位配对码,**仅团队长持有**(入队请求只有团队长能批)。
    #[serde(default)]
    pub pairing_code: Option<String>,
}

impl TeamIdentity {
    pub fn is_leader(&self) -> bool {
        self.role == "leader"
    }
}

/// 生成 64 hex 随机密钥(两个 v4 uuid 的 simple 形式拼接,各 32 hex)。
pub fn gen_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// 生成 6 位数字配对码(取 uuid 随机字节模 10^6,首位补零)。
pub fn gen_pairing_code() -> String {
    let b = uuid::Uuid::new_v4().into_bytes();
    let n = u32::from_le_bytes([b[0], b[1], b[2], b[3]]) % 1_000_000;
    format!("{n:06}")
}

// ============================================================================
// Roster:成员 + 权限清单(只有团队长能改,seq 单调递增,HMAC 签发随 gossip 传播)
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Roster {
    pub team_id: String,
    pub team_name: String,
    /// 单调递增版本号:节点只接受「签名有效且 seq 更高」的清单。
    pub seq: i64,
    pub members: Vec<RosterMember>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RosterMember {
    pub member_id: String,
    pub name: String,
    /// "leader" | "member"
    pub role: String,
    /// 可见范围:None = 全队可见(默认);Some(ids) = 仅这些成员(自己恒可见,显示层兜底)。
    #[serde(default)]
    pub view: Option<Vec<String>>,
    /// 可编辑哪些成员的案件登记字段(edit ⊆ view;Phase 1 只配置/下发,编辑动作 Phase 1.5)。
    #[serde(default)]
    pub edit: Vec<String>,
}

impl Roster {
    pub fn find(&self, member_id: &str) -> Option<&RosterMember> {
        self.members.iter().find(|m| m.member_id == member_id)
    }

    /// `viewer` 能否看到 `target` 的快照(自己恒可见;不在名单=不可见)。
    pub fn can_view(&self, viewer: &str, target: &str) -> bool {
        if viewer == target {
            return true;
        }
        match self.find(viewer) {
            Some(m) if m.role == "leader" => true,
            Some(m) => match &m.view {
                None => true,
                Some(ids) => ids.iter().any(|i| i == target),
            },
            None => false,
        }
    }

    /// `editor` 能否编辑 `target` 的案件登记字段(团队长恒可;须先可见)。
    pub fn can_edit(&self, editor: &str, target: &str) -> bool {
        if !self.can_view(editor, target) {
            return false;
        }
        match self.find(editor) {
            Some(m) if m.role == "leader" => true,
            Some(m) => m.edit.iter().any(|i| i == target),
            None => false,
        }
    }
}

/// 签名信封:**对序列化后的字符串原文签名**(收方先验签、再解析,绕开 JSON 规范化坑)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignedRoster {
    pub roster_json: String,
    pub hmac: String,
}

impl SignedRoster {
    pub fn sign(roster: &Roster, secret: &str) -> Result<Self, String> {
        let roster_json = serde_json::to_string(roster).map_err(|e| e.to_string())?;
        let hmac = hmac_hex(secret, roster_json.as_bytes());
        Ok(Self { roster_json, hmac })
    }

    /// 验签 + 解析。签名不对 → Err(防隔壁团队/篡改)。
    pub fn verify(&self, secret: &str) -> Result<Roster, String> {
        if hmac_hex(secret, self.roster_json.as_bytes()) != self.hmac {
            return Err("roster 签名校验失败".into());
        }
        serde_json::from_str(&self.roster_json).map_err(|e| format!("roster 解析失败: {e}"))
    }
}

// ============================================================================
// 快照(每个成员只写自己的;登记表粒度)
// ============================================================================

/// 一个成员的进度快照信封(gossip 传播单元;存 team_snapshots 一行)。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotEnvelope {
    pub member_id: String,
    pub name: String,
    /// 该成员自己的单调递增序号(防时钟漂移:seq 优先,updated_at 只作展示)。
    pub seq: i64,
    pub updated_at: String,
    /// `SnapshotPayload` 的 JSON 字符串。
    pub payload: String,
}

/// 快照内容:案件登记表粒度。**绝不含**文档原文/报告/聊天/路径/API key。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SnapshotPayload {
    pub cases: Vec<SnapshotCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SnapshotCase {
    /// 案件在所有人本机的 id(uuid)。编辑请求靠它精确定位;老快照缺省空串。
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub case_no: Option<String>,
    #[serde(default)]
    pub parties: Option<String>,
    #[serde(default)]
    pub case_type: Option<String>,
    /// 看板工作流状态(接案/审理中/执行中…,与个人版徽章同口径)。
    #[serde(default)]
    pub stage: Option<String>,
    #[serde(default)]
    pub status_detail: Option<String>,
    #[serde(default)]
    pub claim_amount: Option<f64>,
    /// [{date, event}](来自 agg_key_dates + next_milestone;详情页时间轴用,最多 20 条)。
    #[serde(default)]
    pub key_dates: Vec<SnapshotDate>,
    /// 最近动态一句话(更新时间 + 概括)。
    #[serde(default)]
    pub last_activity: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,

    // ===== v2(0.3.11 团队详情页):更全的登记层字段,老快照缺省即空 =====
    /// 时间轴里**已发生**(≤今天)的最新一件事 —— 案件卡"最新进展"的数据源(老板需求)。
    #[serde(default)]
    pub latest_event: Option<SnapshotDate>,
    #[serde(default)]
    pub court: Option<String>,
    #[serde(default)]
    pub cause: Option<String>,
    #[serde(default)]
    pub filed_at: Option<String>,
    #[serde(default)]
    pub plaintiffs: Vec<String>,
    #[serde(default)]
    pub defendants: Vec<String>,
    #[serde(default)]
    pub third_parties: Vec<String>,
    #[serde(default)]
    pub execution_total: Option<f64>,
    #[serde(default)]
    pub execution_received: Option<f64>,
    #[serde(default)]
    pub execution_remaining: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapshotDate {
    pub date: String,
    pub event: String,
}

/// 合并规则(整个方案零冲突的根基):同一成员,seq 高者胜;seq 相同不动。
/// 返回 true = incoming 应覆盖 existing。
pub fn should_replace(existing_seq: Option<i64>, incoming_seq: i64) -> bool {
    match existing_seq {
        None => true,
        Some(e) => incoming_seq > e,
    }
}

// ============================================================================
// 编辑请求(Phase 2:签名留言式接力转交,详提案 §3.2bis)
// ============================================================================

/// 跨成员编辑请求。生命周期:editor 创建 pending → gossip 传播 → 案件所有人应用
/// (applied,回填 prev_value)或拒绝(rejected:无权限/案件不存在)→ 所有人可撤销
/// (reverted)。状态只升不降([`edit_status_rank`]),多节点合并无冲突。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TeamEdit {
    pub id: String,
    pub team_id: String,
    pub editor_id: String,
    pub editor_name: String,
    pub target_member_id: String,
    pub case_id: String,
    pub case_name: String,
    /// "workflow_status"(改状态,落所有人 cases 表)| "note"(团队备注,仅团队层)。
    pub field: String,
    pub value: String,
    /// 应用时回填的原值(撤销用;note 无)。
    #[serde(default)]
    pub prev_value: Option<String>,
    /// "pending" | "applied" | "rejected" | "reverted"
    pub status: String,
    pub created_at: String,
    #[serde(default)]
    pub applied_at: Option<String>,
}

/// 编辑请求允许改的字段白名单(防滑向协同编辑的硬边界,提案 §3.2bis)。
/// 2026-06-13 老板拍板:团队是本地案件的**只读镜像 + 备注**,不具备编辑本地案件数据的能力
/// (状态等只在各自本地诉讼看板改 → cases::update_workflow_status,本地 owner 是唯一权威源)。
/// 故只允许 "note"(可见即可写),去掉 "workflow_status"(曾导致团队改状态回写本地、刷掉本地手设,bug C)。
pub const EDITABLE_FIELDS: [&str; 1] = ["note"];

/// 状态序:只升不降(合并取 rank 高者)。未知状态 -1(永远被覆盖)。
pub fn edit_status_rank(status: &str) -> i32 {
    match status {
        "pending" => 0,
        "applied" => 1,
        "rejected" | "reverted" => 2,
        _ => -1,
    }
}

// ============================================================================
// HMAC 工具(/exchange 请求体鉴权 + roster 签名共用)
// ============================================================================

pub fn hmac_hex(secret: &str, data: &[u8]) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC 任意长度 key 不会失败");
    mac.update(data);
    let out = mac.finalize().into_bytes();
    let mut s = String::with_capacity(out.len() * 2);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
