//! 元典查询编排器(2026-05-25 V0.1.9 重写 · 聚合优先 + 按需补抓 + 自然人占位)。
//!
//! 输入:case_id + 元典 API key
//!
//! 流程:
//! 1. 从 cases.agg_party_contacts 取被执行人 + cases.agg_filed_at 取立案日(拒执 cutoff)
//! 2. 区分自然人 / 企业(看身份证号 18 位 / 名字含"公司"等)
//! 3. 企业 — 聚合优先策略,分四步:(a) enterprise_search 拿 id + USCC;
//!    (b) enterprise_aggregation_summary 拿所有模块统计 + Top 20 摘要;
//!    (c) 必拉 change_info + out_invest + annual_report(拒执判断 / 财产线索硬需求);
//!    (d) 按需补抓 — 看聚合统计 >0 才拉 executions / executed_person / writ_list /
//!    frozen_equity / pledge / guaranty / court_notice / court_session_notice /
//!    punishment / corporate_tax / abnormal_operation / serious_illegal
//! 4. **自然人** — 元典开放平台未提供按身份证查询自然人涉诉的接口
//!    (llms.txt 36 个公开接口里没有 person 类),不调元典,
//!    只落一个 `<subject>_placeholder.md` 标记,prompt 会读到固定占位文案
//! 5. 所有数据落 `~/Library/.../external/<case_id>/yuandian_raw/`
//!
//! 返回:`OrchestratorReport` 给后续 LLM 风险评估用。
//!
//! **积分账**:V0.1.8 平均 ~16 接口/企业 → V0.1.9 ~5-8 接口/企业(省一半)。

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

use crate::yuandian::{self, file_name, save_json, EntityId};

type CaseExecutionLock = Arc<RwLock<()>>;
type BasicCaseQueryRow = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// 查询会持续数十秒，精确委托人不得在其间变更。锁按案件隔离，互不影响其它案件。
static CASE_EXECUTION_LOCKS: OnceLock<Mutex<HashMap<String, CaseExecutionLock>>> = OnceLock::new();

fn case_execution_lock(case_id: &str) -> CaseExecutionLock {
    let locks = CASE_EXECUTION_LOCKS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut locks = locks.lock().expect("案件查询锁注册表已损坏");
    locks
        .entry(case_id.to_string())
        .or_insert_with(|| Arc::new(RwLock::new(())))
        .clone()
}

/// 外部查询入口必须持有至全部元典、文件和 LLM 工作结束；读锁允许同案只读查询并行。
pub(crate) async fn acquire_execution_query_read_guard(case_id: &str) -> OwnedRwLockReadGuard<()> {
    case_execution_lock(case_id).read_owned().await
}

/// 人工修改精确委托人时只尝试获取写锁，绝不能在长查询后面默默排队。
pub(crate) fn try_acquire_case_representation_write_guard(
    case_id: &str,
) -> Result<OwnedRwLockWriteGuard<()>, String> {
    case_execution_lock(case_id)
        .try_write_owned()
        .map_err(|_| "执行查询正在进行，结束后再修改委托人".to_string())
}

/// 通用 overlay 写入同样会携带或覆盖委托人相关 JSON，必须与执行查询互斥。
pub(crate) async fn update_case_overrides_unless_execution_running(
    pool: &SqlitePool,
    case_id: &str,
    incoming: Option<&str>,
) -> Result<(), String> {
    let _write_guard = try_acquire_case_representation_write_guard(case_id)?;
    crate::db::cases::update_user_overrides_preserving_representation(pool, case_id, incoming)
        .await
        .map_err(|error| error.to_string())
}

/// 被执行主体(自然人或企业)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subject {
    pub name: String,
    pub kind: SubjectKind,
    /// 自然人有身份证号(可帮助文书搜索准确去重)
    pub id_no: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SubjectKind {
    /// 自然人(身份证 18 位)
    Person,
    /// 企业 / 组织(名字一般含"公司/事务所/合作社/中心"等)
    Enterprise,
}

/// 执行模块立场(2026-06-13 Phase 2):决定查谁、报告往哪个方向写。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecStance {
    /// 我方=申请执行人/债权人:查**对方(被执行人)**财产、找拒执线索(默认 / 原有行为)。
    Creditor,
    /// 我方=被执行人/债务人:查**我方客户**做暴露面自查,报告转防御
    /// (查封状态 / 财产豁免 / 执行异议复议 / 和解;拒执线索反向成合规提醒)。
    Debtor,
}

impl ExecStance {
    /// 从我方代理立场推:`被告方` → 债务人模式;其余(原告方/第三人/未知)→ 债权人模式(安全默认)。
    pub fn from_our_side(our_side: Option<&str>) -> Self {
        match our_side.map(str::trim) {
            Some("被告方") => ExecStance::Debtor,
            _ => ExecStance::Creditor,
        }
    }

    /// 给执行类报告的 system prompt 包一层立场。债权人=原样(原有行为);债务人=前置防御立场总纲。
    pub fn wrap_system(&self, base: &str) -> String {
        match self {
            ExecStance::Creditor => base.to_string(),
            ExecStance::Debtor => format!("{}\n\n{}", DEBTOR_EXEC_PREAMBLE, base),
        }
    }
}

/// 债务人(被执行人)立场总纲 —— 前置到三份执行报告的 system prompt,把"帮债权人执行"反转成"为我方客户防御"。
const DEBTOR_EXEC_PREAMBLE: &str = r###"⚠️【立场:我方代理「被执行人」(债务人)—— 本报告服务防御,不是帮债权人执行】
本案我方代理的是被执行人 / 债务人。下文数据里出现的"被执行人"= **我方客户本人**。请整体反转视角输出:
1. **不要**写成"挖被执行人财产线索供执行";改为为我方客户做**风险敞口自查**:哪些财产已被 / 可能被查封冻结、当前查封冻结状态与期限、价值评估、被执行的紧迫度。
2. **财产豁免**:盘点可依法主张不得执行 / 豁免的部分(生活必需品、必要生活费用、唯一住房居住权、社保 / 养老金、被扶养人应得份额等),给主张路径。
3. **执行异议 / 复议线索**:超标的查封、案外人财产被牵连、执行程序瑕疵(送达 / 评估 / 拍卖)、主体不适格、债务已部分清偿未扣减、保证期间 / 时效抗辩等,给可提的异议方向与依据。
4. 原"拒执风险线索"**反向为合规提醒**:明确告知我方客户在执行期间哪些处分财产的动作(转移 / 低价转让 / 抽逃 / 虚假诉讼)可能被认定拒执或被撤销,提示**规避守法**,**绝不教唆隐匿 / 逃避执行**。
5. **和解谈判**:基于双方实力与我方可承受能力,给和解空间与策略建议。
风险等级一律按"对**我方客户**的不利程度 / 紧迫度"判断,而非"对债权人执行的价值"。
以下为原通用指令(其中"被执行人 / 对方"按本立场理解为我方客户;凡"为申请执行人 / 债权人服务"的措辞按防御立场反向执行):"###;

/// 读案件的执行立场(用户在详情页确认的优先,否则用 LLM 抽的 agg_our_side)。
/// 查库失败/没数据时退回 Creditor(原有行为,不破坏既有债权人案件)。
pub async fn exec_stance_for_case(pool: &SqlitePool, case_id: &str) -> ExecStance {
    let row: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT agg_our_side, user_overrides_json FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    let our_side =
        row.and_then(|(a, u)| crate::db::cases::effective_our_side(a.as_deref(), u.as_deref()));
    ExecStance::from_our_side(our_side.as_deref())
}

/// 所有会发起元典或 LLM 外部调用的执行入口共用此守卫。
///
/// 精确委托人存在时，当前阶段只能用 `name + role` 与联系人匹配；每个精确元组
/// 必须恰好命中一条。解析损坏、缺失或同名同角色歧义一律在任何文件 I/O 或外部调用前
/// 返回错误，禁止各个 IPC 入口各自退回旧的粗立场逻辑。
pub(crate) async fn exact_representation_for_external_query(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Option<crate::db::case_representation::CaseRepresentation>, String> {
    let row: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT agg_party_contacts, agg_our_side, user_overrides_json FROM cases WHERE id = ?",
    )
    .bind(case_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("查案件精确委托人状态失败: {error}"))?;
    let Some((party_contacts, agg_our_side, user_overrides_json)) = row else {
        return Err(format!("案件不存在:{case_id}"));
    };

    let representation = crate::db::case_representation::effective_representation(
        user_overrides_json.as_deref(),
        agg_our_side.as_deref(),
    )
    .map_err(|error| format!("案件精确委托人状态无效: {error}"))?;
    let explicit_representation =
        representation.filter(|representation| !representation.parties.is_empty());
    if let Some(representation) = &explicit_representation {
        validate_exact_representation_contacts(
            party_contacts.as_deref(),
            representation.parties.as_slice(),
        )?;
    }
    Ok(explicit_representation)
}

/// 编排结果汇报(给前端 Toast + 给 LLM 评估用)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestratorReport {
    pub case_id: String,
    pub subjects: Vec<Subject>,
    /// 每个 subject 的 raw 文件相对路径列表(相对于 external/<case_id>/yuandian_raw/)
    pub raw_files: Vec<String>,
    /// 整体耗时
    pub elapsed_ms: u128,
    /// 失败的 endpoint(`<subject>::<endpoint>` 形式)+ 错误信息
    pub failures: Vec<(String, String)>,
    /// 2026-05-25 V0.1.9 加 · 案件立案日(从 cases.agg_filed_at 拉),
    /// 给 prompt 做拒执 cutoff 用。空表示 LLM 还没抽到立案日
    pub filed_at: Option<String>,
}

/// 跑一次完整 P1 元典查询。
pub async fn basic_query(
    pool: &SqlitePool,
    case_id: &str,
    api_key: &str,
) -> Result<OrchestratorReport, String> {
    let _query_guard = acquire_execution_query_read_guard(case_id).await;
    basic_query_while_read_locked(pool, case_id, api_key).await
}

/// 供 `yuandian_basic_query` 在同一读锁内串起 P1 与风险评估时调用。
/// 调用方必须持有 [`acquire_execution_query_read_guard`] 返回的 guard。
pub(crate) async fn basic_query_while_read_locked(
    pool: &SqlitePool,
    case_id: &str,
    api_key: &str,
) -> Result<OrchestratorReport, String> {
    let start = std::time::Instant::now();
    let explicit_representation = exact_representation_for_external_query(pool, case_id).await?;

    // 1. 拿 case 的 party_contacts + agg_filed_at(立案日,拒执 cutoff)
    let row: Option<BasicCaseQueryRow> = sqlx::query_as(
        "SELECT agg_party_contacts, agg_filed_at, agg_our_side, user_overrides_json \
             FROM cases WHERE id = ?",
    )
    .bind(case_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查 case 失败:{}", e))?;
    let (party_json, filed_at, agg_our_side, user_overrides_json) = match row {
        Some((p, f, side, overrides)) => (p, f, side, overrides),
        None => (None, None, None, None),
    };

    // Phase 2:按执行立场选查询对象。债权人模式查对方(被执行人);债务人模式查我方客户(做暴露面自查)。
    // 粗立场只负责决定债权人/债务人模式，不能再用于判断单个当事人是否受托。
    let our_side = crate::db::cases::effective_our_side(
        agg_our_side.as_deref(),
        user_overrides_json.as_deref(),
    );
    let stance = ExecStance::from_our_side(our_side.as_deref());
    let subjects = extract_target_subjects(
        party_json.as_deref(),
        stance,
        explicit_representation
            .as_ref()
            .map(|representation| representation.parties.as_slice()),
    );
    if subjects.is_empty() {
        let who = match stance {
            ExecStance::Creditor => "被执行人",
            ExecStance::Debtor => "我方当事人(被执行人)",
        };
        return Err(format!(
            "没找到{}(可能 LLM 还没抽 party_contacts,先生成案件报告)",
            who
        ));
    }

    // 2. 准备输出目录
    let dir = raw_dir_for_case(case_id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录失败:{}", e))?;

    let mut raw_files = Vec::new();
    let mut failures = Vec::new();

    for subject in &subjects {
        crate::dlog!(
            "[yuandian] case={} subject={:?} kind={:?}",
            case_id,
            subject.name,
            subject.kind
        );
        if subject.kind == SubjectKind::Enterprise {
            match query_enterprise(api_key, subject, &dir, filed_at.as_deref()).await {
                Ok(files) => raw_files.extend(files),
                Err(e) => {
                    failures.push((subject.name.clone(), e));
                }
            }
        } else {
            // 2026-05-25 V0.1.9:自然人不调元典(没有 person 类接口),只落 placeholder
            match write_person_placeholder(subject, &dir) {
                Ok(f) => raw_files.push(f),
                Err(e) => failures.push((subject.name.clone(), e)),
            }
        }
    }

    // D2-1:执行模块也按聚合优先口径记元典积分。原先记账只在 chat after_tool_call hook,
    // 执行三份报告直接调元典却完全不记 → 月度统计严重漏算。每个落盘 .json = 一次计费端点调用
    // (aggregation 5、其余 1),自然人占位 .md = 0。失败的端点不落文件、自然不计(坑 #12)。
    let year_month = crate::db::credits::current_year_month();
    for f in &raw_files {
        let c = crate::db::credits::credits_for_raw_file(f);
        if c > 0 {
            let _ = crate::db::credits::record_yuandian_call(pool, &year_month, c).await;
        }
    }

    Ok(OrchestratorReport {
        case_id: case_id.to_string(),
        subjects,
        raw_files,
        elapsed_ms: start.elapsed().as_millis(),
        failures,
        filed_at,
    })
}

/// 从 agg_party_contacts JSON 抽出要查询的主体列表。
/// - 有精确委托人时：债务人模式只查精确名单；债权人模式排除精确名单。
/// - 旧案件无精确名单：保留 is_our_side 与角色的原有兼容行为。
fn extract_target_subjects(
    json: Option<&str>,
    stance: ExecStance,
    represented_parties: Option<&[crate::db::case_representation::RepresentedParty]>,
) -> Vec<Subject> {
    let Some(j) = json else { return vec![] };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(j) else {
        return vec![];
    };
    let Some(arr) = parsed.as_array() else {
        return vec![];
    };
    let mut out: Vec<Subject> = Vec::new();
    let mut seen: std::collections::HashSet<String> = Default::default();

    for item in arr {
        let name = item
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let role = item.get("role").and_then(|v| v.as_str()).unwrap_or("");
        let is_our_side = item.get("is_our_side").and_then(|v| v.as_bool());
        let role_is_executee = role.contains("被告")
            || role.contains("被执行")
            || role.contains("被申请")
            || role.contains("被告人");

        let is_explicitly_represented = represented_parties.is_some_and(|represented| {
            represented
                .iter()
                .any(|party| party.name.trim() == name && party.role.trim() == role.trim())
        });

        let is_target = match stance {
            // 债权人:查对方(被执行人)。is_our_side==false,或按被执行类角色兜底;
            // 但**绝不查我方**(is_our_side==true 时即便角色像被执行人也排除,防数据矛盾误查自己客户)。
            ExecStance::Creditor => match represented_parties {
                Some(_) => !is_explicitly_represented && role_is_executee,
                None => {
                    is_our_side == Some(false) || (is_our_side != Some(true) && role_is_executee)
                }
            },
            // 债务人:查我方客户本人做暴露面自查。is_our_side==true(排除代理人/法务);
            // is_our_side 缺失时退回"被执行类角色"(债务人案件里这正是我方客户)。
            ExecStance::Debtor => match represented_parties {
                Some(_) => is_explicitly_represented && !role.contains("代理"),
                None => {
                    if role.contains("代理") {
                        false
                    } else {
                        is_our_side == Some(true) || (is_our_side.is_none() && role_is_executee)
                    }
                }
            },
        };
        if !is_target {
            continue;
        }
        let id_no = item
            .get("id_no")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty());

        // D2-6:按 (姓名, 身份证号) 去重 —— 同名但不同身份证是不同主体,不能合并丢一个。
        // 身份证缺失时退回仅按姓名(保守:同名无证仍合并,避免重复外查同一人)。
        let dedup_key = format!("{}|{}", name, id_no.as_deref().unwrap_or(""));
        if !seen.insert(dedup_key) {
            continue;
        }

        let kind = if looks_like_enterprise(&name) {
            SubjectKind::Enterprise
        } else {
            SubjectKind::Person
        };

        out.push(Subject { name, kind, id_no });
    }
    out
}

/// 精确名单没有稳定实体 ID 时，同名同角色必须唯一；否则不能决定哪一条联系人记录
/// 才是受托人。此校验必须在创建目录、写文件或调用外部接口之前完成。
fn validate_exact_representation_contacts(
    json: Option<&str>,
    represented_parties: &[crate::db::case_representation::RepresentedParty],
) -> Result<(), String> {
    let json = json.ok_or_else(|| {
        "精确委托人无法在当事人联系人中匹配，当前版本无法自动区分。请先重新分析并核对；若仍存在同名同角色，请勿继续查询并人工核验。"
            .to_string()
    })?;
    let parsed: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| format!("当事人联系人格式无效，拒绝执行外部查询: {error}"))?;
    let contacts = parsed
        .as_array()
        .ok_or_else(|| "当事人联系人格式无效，拒绝执行外部查询".to_string())?;

    for represented in represented_parties {
        let matched = contacts
            .iter()
            .filter(|contact| {
                contact
                    .get("name")
                    .and_then(|value| value.as_str())
                    .is_some_and(|name| name.trim() == represented.name)
                    && contact
                        .get("role")
                        .and_then(|value| value.as_str())
                        .is_some_and(|role| role.trim() == represented.role)
            })
            .count();
        if matched != 1 {
            return Err(ambiguous_exact_party_contact_error(
                &represented.name,
                &represented.role,
                matched,
            ));
        }
    }
    Ok(())
}

fn ambiguous_exact_party_contact_error(name: &str, role: &str, matched: usize) -> String {
    format!(
        "精确委托人“{name}”（{role}）在当事人联系人中匹配{matched}条，当前版本无法自动区分。请先重新分析并核对；若仍存在同名同角色，请勿继续查询并人工核验。"
    )
}

/// 简单启发式判断企业 vs 自然人(中文姓名通常 2-4 字,企业带"公司/事务所/合作社/中心/有限/股份"等关键词)
fn looks_like_enterprise(name: &str) -> bool {
    const KEYS: &[&str] = &[
        "公司",
        "事务所",
        "合作社",
        "中心",
        "厂",
        "店",
        "馆",
        "院",
        "集团",
        "联合",
        "委员会",
        "协会",
        "学会",
        "基金",
        "工作室",
        "Co.",
        "Ltd",
        "Inc",
    ];
    KEYS.iter().any(|k| name.contains(k))
}

/// 企业:聚合优先策略(2026-05-25 V0.1.9 重写,替代原 14 端点硬调)
///
/// 流程:
///   1. enterprise_search → 拿 id + USCC
///   2. enterprise_aggregation_summary → 一次拿所有模块统计 + Top 20
///   3. 必拉(拒执判断 / 财产线索硬需求):
///      - change_info / out_invest / annual_report(立案前一年 + 当年)
///   4. 按需补抓(看 aggregation 统计 >0):
///      - executions / executed_person / writ_list / frozen_equity / pledge /
///        guaranty / court_notice / court_session_notice / punishment /
///        corporate_tax / abnormal_operation / serious_illegal
async fn query_enterprise(
    api_key: &str,
    subject: &Subject,
    base_dir: &std::path::Path,
    filed_at: Option<&str>,
) -> Result<Vec<String>, String> {
    // ---- Step 1: 找企业 id ----
    let search = yuandian::enterprise_search(api_key, &subject.name)
        .await
        .map_err(|e| format!("search:{}", e))?;
    save_json(base_dir, &subject.name, "search", &search)?;
    let mut files = vec![file_name(&subject.name, "search")];

    // 拿第一个匹配 id(精确匹配优先,否则取第一个)
    let candidates = search
        .get("data")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let id = candidates
        .iter()
        .find(|c| {
            c.get("企业名称")
                .and_then(|v| v.as_str())
                .map(|n| n == subject.name)
                .unwrap_or(false)
        })
        .or_else(|| candidates.first())
        .and_then(|c| c.get("id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let Some(id) = id else {
        return Err("元典没找到该企业".into());
    };
    let entity = EntityId::Id(id);

    // ---- Step 2: 聚合摘要(看每个模块统计数,决定后面补抓哪些) ----
    let aggregation = match yuandian::enterprise_aggregation_summary(api_key, &entity).await {
        Ok(v) => {
            save_json(base_dir, &subject.name, "aggregation", &v)?;
            files.push(file_name(&subject.name, "aggregation"));
            v
        }
        Err(e) => {
            // D2-4:聚合失败 → 退化全拉兜底。显式标注,避免"省积分指标好看但实际拉满端点、积分偏高"不可见。
            crate::dlog!(
                "[yuandian] {} aggregation 失败 → 退化全拉兜底(本次会拉满端点、积分偏高): {}",
                subject.name,
                e
            );
            serde_json::Value::Null
        }
    };

    // call 宏:一次端点调用,失败不阻断
    macro_rules! call {
        ($ep:literal, $fn:expr) => {{
            match $fn.await {
                Ok(v) => {
                    save_json(base_dir, &subject.name, $ep, &v)?;
                    files.push(file_name(&subject.name, $ep));
                }
                Err(e) => crate::dlog!("[yuandian] {} {}: {}", subject.name, $ep, e),
            }
        }};
    }

    // ---- Step 3: 必拉(拒执判断 / 财产线索硬需求) ----
    call!(
        "change_info",
        yuandian::enterprise_change_info(api_key, &entity, 1)
    );
    call!(
        "out_invest",
        yuandian::enterprise_out_invest(api_key, &entity, 1)
    );
    // annual_report 按"立案前一年 + 当年"两份拉(拒执判断对比资产变化)
    // 没立案日就拉去年的(尽量获取一份)
    let years_to_pull = annual_report_years(filed_at);
    for year in &years_to_pull {
        let ep = format!("annual_report_{}", year);
        match yuandian::enterprise_annual_report(api_key, &entity, *year).await {
            Ok(v) => {
                save_json(base_dir, &subject.name, &ep, &v)?;
                files.push(file_name(&subject.name, &ep));
            }
            Err(e) => crate::dlog!("[yuandian] {} {}: {}", subject.name, ep, e),
        }
    }

    // ---- Step 4: 按需补抓 — 看 aggregation 统计 >0 才拉 ----
    // 注意:聚合 JSON 的字段名是元典内部约定,如果失败则保守全拉
    let need = NeedList::from_aggregation(&aggregation);
    if need.executions {
        call!(
            "executions",
            yuandian::enterprise_executions(api_key, &entity, 1)
        );
    }
    if need.executed_person {
        call!(
            "executed_person",
            yuandian::enterprise_executed_person(api_key, &entity, 1)
        );
    }
    if need.writ_list {
        call!(
            "writ_list",
            yuandian::enterprise_writ_list(api_key, &entity, 1)
        );
    }
    if need.frozen_equity {
        call!(
            "frozen_equity",
            yuandian::enterprise_frozen_equity(api_key, &entity, 1)
        );
    }
    if need.pledge {
        call!("pledge", yuandian::enterprise_pledge(api_key, &entity, 1));
    }
    if need.guaranty {
        call!(
            "guaranty",
            yuandian::enterprise_guaranty(api_key, &entity, 1)
        );
    }
    if need.court_notice {
        call!(
            "court_notice",
            yuandian::enterprise_court_notice(api_key, &entity, 1)
        );
    }
    if need.court_session_notice {
        call!(
            "court_session_notice",
            yuandian::enterprise_court_session_notice(api_key, &entity, 1)
        );
    }
    if need.punishment {
        call!(
            "punishment",
            yuandian::enterprise_punishment(api_key, &entity, 1)
        );
    }
    if need.corporate_tax {
        call!(
            "corporate_tax",
            yuandian::enterprise_corporate_tax(api_key, &entity, 1)
        );
    }
    if need.abnormal_operation {
        call!(
            "abnormal_operation",
            yuandian::enterprise_abnormal_operation(api_key, &entity, 1)
        );
    }
    if need.serious_illegal {
        call!(
            "serious_illegal",
            yuandian::enterprise_serious_illegal(api_key, &entity, 1)
        );
    }

    Ok(files)
}

/// 自然人占位:元典没有按身份证查询自然人的接口(llms.txt 36 个公开接口都是企业类)。
///
/// 不调元典 API,只落一个 `<subject>_placeholder.md` 标记。
/// 后面 risk_assessment / deep_dive / full_report 的 prompt 会读到这个文件,
/// 按 SYSTEM_PROMPT 指令输出固定占位文案"请律师自行通过裁判文书网 / 中国执行信息公开网核查"。
fn write_person_placeholder(
    subject: &Subject,
    base_dir: &std::path::Path,
) -> Result<String, String> {
    let fname = file_name(&subject.name, "placeholder").replace(".json", ".md");
    let path = base_dir.join(&fname);
    let body = format!(
        "# 自然人主体 · 占位说明\n\n\
         **姓名**:{}\n\
         **身份证号**:{}\n\n\
         > 元典法律开放平台未提供按身份证查询自然人涉诉 / 失信 / 执行信息的接口\n\
         > (36 个公开接口全是企业维度,案例库也只能按案号 / 涉诉企业 / 全文关键词过滤)。\n\
         > 自然人查询请律师自行通过:\n\
         > - 裁判文书网 https://wenshu.court.gov.cn/\n\
         > - 中国执行信息公开网 http://zxgk.court.gov.cn/\n\
         > - 信用中国 https://www.creditchina.gov.cn/\n\n\
         本工具的元典策略:**自然人主体只列出基本信息,不调元典 API,等用户线下核查**。\n",
        subject.name,
        subject.id_no.as_deref().unwrap_or("(未提供)")
    );
    std::fs::write(&path, body).map_err(|e| format!("写 {} 失败:{}", path.display(), e))?;
    Ok(fname)
}

/// 决定 annual_report 拉哪些年份:立案前一年 + 立案当年
fn annual_report_years(filed_at: Option<&str>) -> Vec<u32> {
    let now_year = chrono::Local::now().format("%Y").to_string();
    let default_year = now_year.parse::<u32>().unwrap_or(2025).saturating_sub(1);
    let Some(s) = filed_at else {
        // 没立案日 → 拉去年
        return vec![default_year];
    };
    // 立案日格式可能是 "2025-06-14" / "2025/06/14" / "2025年6月14日" 等,
    // 取前 4 位数字当年份
    let year_str: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let Ok(filed_year) = year_str.parse::<u32>() else {
        return vec![default_year];
    };
    // 拉立案前一年 + 立案当年(对比资产变化)
    vec![filed_year.saturating_sub(1), filed_year]
}

/// 按需补抓清单 — 看聚合数据,统计 >0 的模块标 true。
///
/// 聚合 JSON 失败时全部置 true(保守兜底)。
struct NeedList {
    executions: bool,
    executed_person: bool,
    writ_list: bool,
    frozen_equity: bool,
    pledge: bool,
    guaranty: bool,
    court_notice: bool,
    court_session_notice: bool,
    punishment: bool,
    corporate_tax: bool,
    abnormal_operation: bool,
    serious_illegal: bool,
}

impl NeedList {
    fn all_true() -> Self {
        Self {
            executions: true,
            executed_person: true,
            writ_list: true,
            frozen_equity: true,
            pledge: true,
            guaranty: true,
            court_notice: true,
            court_session_notice: true,
            punishment: true,
            corporate_tax: true,
            abnormal_operation: true,
            serious_illegal: true,
        }
    }

    /// 看聚合 JSON 各模块计数,>0 才需补抓(省积分)。
    ///
    /// 元典真实响应(实测 `external/*/yuandian_raw/*_aggregation.json`):
    ///   `data.<模块>统计.总数`(整数),例如 `data.失信被执行人统计.总数 = 0`。
    /// 字段名以「<模块>统计」为准,旧裸名 / camelCase 作兜底候选;计数键以「总数」为准,兼容 count/total。
    /// **聚合里没有「涉诉文书统计」字段** → 涉诉文书永远 fail-safe 全拉(执行律师最关心,保守正确)。
    fn from_aggregation(agg: &serde_json::Value) -> Self {
        // 聚合失败(Null)→ 全拉兜底
        if agg.is_null() {
            return Self::all_true();
        }
        let data = match agg.get("data") {
            Some(d) => d,
            None => return Self::all_true(),
        };

        Self {
            executions: has_data(data, &["失信被执行人统计", "失信被执行人", "executions"]),
            executed_person: has_data(data, &["被执行人统计", "被执行人", "executedPerson"]),
            writ_list: has_data(data, &["涉诉文书统计", "涉诉文书", "writList", "writs"]),
            frozen_equity: has_data(data, &["股权冻结统计", "股权冻结", "frozenEquity"]),
            pledge: has_data(data, &["股权出质统计", "股权出质", "pledge"]),
            guaranty: has_data(data, &["对外担保统计", "对外担保", "guaranty"]),
            court_notice: has_data(data, &["法院公告统计", "法院公告", "courtNotice"]),
            court_session_notice: has_data(
                data,
                &["开庭公告统计", "开庭公告", "courtSessionNotice"],
            ),
            punishment: has_data(data, &["行政处罚统计", "行政处罚", "punishment"]),
            corporate_tax: has_data(data, &["欠税公告统计", "欠税", "corporateTax"]),
            abnormal_operation: has_data(data, &["经营异常统计", "经营异常", "abnormalOperation"]),
            serious_illegal: has_data(data, &["严重违法统计", "严重违法", "seriousIllegal"]),
        }
    }
}

/// 看聚合响应里某模块的计数字段判断「有无数据」(决定是否补抓该端点,省积分)。
///
/// 命中任一候选字段 → **立刻按该字段值判定并返回**(不再无条件兜底):
///   - 直接数字 > 0 / 对象「总数」(元典实测键名)| count | total > 0 / 数组非空 → 有数据
///   - 命中但计数为 0 / 数组为空 → 无数据(跳过补抓,省积分)
///   - 命中但结构不认识 → 保守当有数据
///
/// 所有候选都对不上 → 保守全拉 + dlog 告警(暴露元典字段名漂移,避免静默多花积分)。
///
/// 2026-06-03 B6:修复原 bug —— 原实现命中字段但值为 0 时不返回,一路走到末尾 fail-safe `true`,
/// 等于 `has_data` 恒真、`from_aggregation` 恒 `all_true`,"按需补抓省积分"从未生效。
fn has_data(d: &serde_json::Value, candidates: &[&str]) -> bool {
    for k in candidates {
        let Some(v) = d.get(*k) else { continue };
        // 直接数字
        if let Some(n) = v.as_i64() {
            return n > 0;
        }
        // 对象:元典实测用「总数」,兼容 count / total
        for cnt_key in ["总数", "count", "total"] {
            if let Some(c) = v.get(cnt_key).and_then(|x| x.as_i64()) {
                return c > 0;
            }
        }
        // 数组非空
        if let Some(arr) = v.as_array() {
            return !arr.is_empty();
        }
        // 命中字段但结构不认识 → 保守当有数据
        return true;
    }
    // 所有候选都对不上 → 保守全拉 + 告警(暴露元典字段名漂移)
    crate::dlog!(
        "[yuandian] aggregation 字段未命中候选 {:?},退化为「有数据」全拉(可能元典字段名漂移、积分偏高)",
        candidates
    );
    true
}

/// `~/Library/Application Support/CaseBoard/external/<case_id>/yuandian_raw/`
fn raw_dir_for_case(case_id: &str) -> Result<PathBuf, String> {
    let base = crate::db::app_data_dir().map_err(|e| format!("无法定位 app data dir: {}", e))?;
    Ok(base.join("external").join(case_id).join("yuandian_raw"))
}
