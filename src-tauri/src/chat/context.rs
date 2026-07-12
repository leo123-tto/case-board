//! 案件 AI 助手 — context builder。
//!
//! V0.3.3 起删除了老的「两策略 + 无工具 stream」链路(Lightweight / KeywordHits /
//! `build_context` / `strategy_for_task`):所有 chat 现在统一走 `agent_loop`(constitution
//! 完整宪法 + 工具)。案件材料由 `constitution::build_system_prompt` 用 `lightweight_docs_md`
//! 拼成轻量摘要,详细内容让 LLM 按需调 `read_case_doc` / `find_in_document` /
//! `semantic_search_case_docs` 工具拿。
//!
//! 本模块现在只剩三块、且都被 `constitution` 复用:
//!   - `TaskType`:任务路由枚举(FreeChat + 4 个工具/分析型 chip)
//!   - `case_snapshot_md`:案件聚合字段 → 「案件信息卡」MD
//!   - `lightweight_docs_md`:文档清单的轻量摘要(filename + category + extracted_fields)

use crate::db::cases::Case;
use crate::db::documents::Document;
use serde_json::Value;

// =============================================================================
// 公开类型
// =============================================================================

/// 案件 chat 的 task 枚举。前端传字符串,后端 parse。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    /// 自由问答(用户自己打字)
    FreeChat,
    /// 整理法律依据(并行调 search_laws / get_law_article / law_vector_search)
    CompileLegalBasis,
    /// 找类似案例(并行调 search_cases_normal / search_cases_authority / case_vector_search)
    FindSimilarCases,
    /// 核校用户已写的草稿里的法条/案号引用(走 verify_legal_citations)
    VerifyMyDraft,
    /// 模拟对抗:站对方立场推演抗辩/进攻 + 我方应对(走 agent_loop,查支持对方的法条/类案)
    SimulateOpposition,
    /// 深度分析:请求权基础 + 鉴定式方法论,两闸交互确认(候选请求权清单 → 大纲)后逐要件论证,
    /// 落一份深度分析报告 artifact(走 agent_loop,逐条 get_law_article 校验法条)。
    DeepAnalysis,
    /// 刑事深度分析:三阶层犯罪论 + 鉴定式方法论(借鉴游初 gutachten-criminal-case,Apache 2.0),
    /// 两闸交互确认(候选罪名清单 → 三阶层检视大纲)后逐要件论证,落一份刑事深度分析报告 artifact。
    /// 仅刑事 tab 的 AI 助手用。
    CriminalDeepAnalysis,
}

impl TaskType {
    /// 字符串(前端传入)→ TaskType。未知字符串当 FreeChat。
    pub fn from_str_loose(s: Option<&str>) -> Self {
        match s {
            Some("compile_legal_basis") => Self::CompileLegalBasis,
            Some("find_similar_cases") => Self::FindSimilarCases,
            Some("verify_my_draft") => Self::VerifyMyDraft,
            Some("simulate_opposition") => Self::SimulateOpposition,
            Some("deep_analysis") => Self::DeepAnalysis,
            Some("criminal_deep_analysis") => Self::CriminalDeepAnalysis,
            _ => Self::FreeChat,
        }
    }

    /// 回写到 chat_messages.task_type 用的稳定字符串。
    pub fn as_db_str(&self) -> Option<&'static str> {
        match self {
            Self::FreeChat => None,
            Self::CompileLegalBasis => Some("compile_legal_basis"),
            Self::FindSimilarCases => Some("find_similar_cases"),
            Self::VerifyMyDraft => Some("verify_my_draft"),
            Self::SimulateOpposition => Some("simulate_opposition"),
            Self::DeepAnalysis => Some("deep_analysis"),
            Self::CriminalDeepAnalysis => Some("criminal_deep_analysis"),
        }
    }

    /// 本任务是否属于「工具/分析型」(4 个)。V0.3.3 起**所有任务都走 agent_loop**;
    /// 本标志现仅用于 model_router auto 档分流(工具型 → pro)等细分,不再决定走哪条链路。
    pub fn needs_tools(&self) -> bool {
        matches!(
            self,
            Self::CompileLegalBasis
                | Self::FindSimilarCases
                | Self::VerifyMyDraft
                | Self::SimulateOpposition
                | Self::DeepAnalysis
                | Self::CriminalDeepAnalysis
        )
    }
}

/// 每份文档轻量摘要长度上限(filename + category + 摘要)。
const PER_DOC_LIGHT_CHAR_LIMIT: usize = 600;

// =============================================================================
// snapshot 拼装
// =============================================================================

/// 把 case.agg_* 字段拼成 MD 段(给 LLM 看的"案件信息卡")。
///
/// V0.2 D4-D5 起,`chat::constitution::build_system_prompt` 也复用本函数 — 因此 `pub(crate)`。
///
/// 2026-07-12:本函数现在会叠加 `cases.user_overrides_json` 里用户(或 AI 通过工具)确认的值,
/// 让 AI 助手看到的快照与前端编辑模式/详情页保持一致。
pub(crate) fn case_snapshot_md(case: &Case) -> String {
    let overrides = case
        .user_overrides_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or_default();
    let fields = overrides.get("fields").and_then(|v| v.as_object());

    let mut s = String::with_capacity(2048);

    // 基本信息
    s.push_str("【基本信息】\n");
    push_kv(&mut s, "案件名称", Some(&case.name));
    push_kv(&mut s, "案件类型", Some(&case.case_type));
    push_kv(
        &mut s,
        "案号",
        over_str(
            fields,
            "agg_case_no",
            case.agg_case_no.as_deref().or(case.case_no.as_deref()),
        ),
    );
    push_kv(
        &mut s,
        "法院",
        over_str(
            fields,
            "agg_court",
            case.agg_court.as_deref().or(case.court.as_deref()),
        ),
    );
    push_kv(
        &mut s,
        "案由",
        over_str(
            fields,
            "agg_cause",
            case.agg_cause.as_deref().or(case.cause.as_deref()),
        ),
    );
    push_kv(
        &mut s,
        "立案日",
        over_str(fields, "agg_filed_at", case.agg_filed_at.as_deref()),
    );
    push_kv(
        &mut s,
        "诉讼请求金额",
        over_amount(fields, "agg_claim_amount", case.agg_claim_amount)
            .map(format_amount)
            .as_deref(),
    );
    // D9-1:DB 存英文 StatusId,喂 LLM 时还原中文 label(更可读,且不依赖 agg_status_text)。
    push_kv(
        &mut s,
        "工作流状态",
        case.workflow_status
            .as_deref()
            .map(crate::ingest::global_pipeline::workflow_status_en_to_zh),
    );
    push_kv(
        &mut s,
        "LLM 状态描述",
        over_str(fields, "agg_status_text", case.agg_status_text.as_deref()),
    );
    push_kv(
        &mut s,
        "案件总状态",
        over_str(fields, "case_status", Some(&case.case_status)),
    );
    push_kv(
        &mut s,
        "一句话摘要",
        over_str(fields, "case_summary", case.case_summary.as_deref()),
    );

    // 当事人
    s.push_str("\n【当事人】\n");
    // 2026-06-13:我方代理立场置顶。用户确认值(override)权威,LLM 抽的 agg_our_side 次之。
    // 所有 chip(模拟对抗/类案检索/法律依据)和 AI 应答据此定攻防,不再"猜我方"。
    let llm_side = case
        .agg_our_side
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let user_side = crate::db::cases::user_override_our_side(case.user_overrides_json.as_deref());
    match user_side.as_deref().or(llm_side) {
        Some(side) => {
            // 律师改过立场、但 LLM 值(及每个当事人 is_our_side 标记)还没经「重新分析」同步时,
            // 二者会冲突 → 明确以案件级确认值为准,消除 AI 站反风险(advisor 命门:override 未重抽窗口)。
            if user_side.is_some() && user_side.as_deref() != llm_side {
                s.push_str(&format!(
                    "- 我方代理立场: {}(律师已确认,**与下方个别当事人 [我方]/[对方] 标记或案件报告冲突时一律以此为准**;旧标记/旧报告需「重新分析」后才同步)\n",
                    side
                ));
            } else {
                push_kv(&mut s, "我方代理立场", Some(side));
            }
        }
        None => s.push_str(
            "- 我方代理立场: 未确认(若要做立场化分析/对抗/检索,先确认我方是原告方还是被告方;未确认前保持中立、勿臆断)\n",
        ),
    }
    push_string_list(
        &mut s,
        "原告/申请人",
        &over_string_array(fields, "agg_plaintiffs", case.agg_plaintiffs.as_deref()),
    );
    push_string_list(
        &mut s,
        "被告/被申请人",
        &over_string_array(fields, "agg_defendants", case.agg_defendants.as_deref()),
    );
    push_string_list(
        &mut s,
        "第三人",
        &over_string_array(fields, "agg_third_parties", case.agg_third_parties.as_deref()),
    );
    push_string_list(
        &mut s,
        "承办法官",
        &over_string_array(fields, "agg_judges", case.agg_judges.as_deref()),
    );

    // 联系人(简略)
    if let Some(party_json) = &case.agg_party_contacts {
        let summary = summarize_party_contacts(party_json, fields);
        if !summary.is_empty() {
            s.push_str(&format!("- 当事人联系人:\n{}\n", indent_block(&summary, 2)));
        }
    }
    if let Some(court_json) = &case.agg_court_contacts {
        let summary = summarize_court_contacts(court_json, fields);
        if !summary.is_empty() {
            s.push_str(&format!("- 法院联系人:\n{}\n", indent_block(&summary, 2)));
        }
    }

    // 关键日期
    if let Some(kd_json) = &case.agg_key_dates {
        let summary = summarize_key_dates(kd_json, fields);
        if !summary.is_empty() {
            s.push_str("\n【关键日期】\n");
            s.push_str(&summary);
        }
    }

    // 费用
    if let Some(fees_json) = &case.agg_fees {
        let summary = summarize_fees(fees_json, fields);
        if !summary.is_empty() {
            s.push_str("\n【费用记录】\n");
            s.push_str(&summary);
        }
    }

    // 下一节点 / 执行进度
    if case.next_milestone_at.is_some() || case.next_milestone_type.is_some() {
        s.push_str("\n【下一关键节点】\n");
        push_kv(&mut s, "类型", case.next_milestone_type.as_deref());
        push_kv(&mut s, "日期", case.next_milestone_at.as_deref());
        push_kv(&mut s, "状态", case.next_milestone_status.as_deref());
        push_kv(&mut s, "备注", case.next_milestone_note.as_deref());
    }

    if case.execution_total.is_some() || case.execution_received.is_some() {
        s.push_str("\n【执行款追踪】\n");
        push_kv(
            &mut s,
            "执行总额",
            case.execution_total.map(format_amount).as_deref(),
        );
        push_kv(
            &mut s,
            "已收回",
            case.execution_received.map(format_amount).as_deref(),
        );
        push_kv(
            &mut s,
            "剩余",
            case.execution_remaining.map(format_amount).as_deref(),
        );
    }

    if let Some(reso) = &case.agg_resolution {
        let effective = over_str(fields, "agg_resolution", Some(reso.as_str()));
        if !effective.unwrap_or("").trim().is_empty() {
            s.push_str("\n【处理结果】\n");
            s.push_str(effective.unwrap());
            s.push('\n');
        }
    }

    // 用户覆盖透明说明(让 AI 知道哪些值是人工确认的,避免擅自改回去)
    if let Some(f) = fields {
        s.push_str("\n【用户已手工覆盖的字段】(这些值已覆盖 LLM 抽取结果)\n");
        for (k, v) in f {
            let val = if v.is_null() {
                "（已清空）".to_string()
            } else {
                v.as_str().unwrap_or("（非字符串）").to_string()
            };
            s.push_str(&format!("- {}: {}\n", k, val));
        }
    }

    s
}

/// 取字段覆盖值。返回 Some("") 表示用户清空,返回 None 表示未覆盖(用 raw)。
fn over_str<'a>(
    fields: Option<&'a serde_json::Map<String, Value>>,
    path: &str,
    raw: Option<&'a str>,
) -> Option<&'a str> {
    if let Some(v) = fields.and_then(|m| m.get(path)) {
        if v.is_null() {
            return Some("");
        }
        if let Some(s) = v.as_str() {
            return Some(s);
        }
    }
    raw
}

/// 数值字段覆盖。空串/null 返回 None,否则尝试 parse。
fn over_amount(
    fields: Option<&serde_json::Map<String, Value>>,
    path: &str,
    raw: Option<f64>,
) -> Option<f64> {
    if let Some(v) = fields.and_then(|m| m.get(path)) {
        if v.is_null() {
            return None;
        }
        if let Some(s) = v.as_str() {
            return parse_clean_number(s);
        }
    }
    raw
}

/// 把数组型字段的覆盖应用到原始 JSON 数组,返回过滤后的字符串列表。
fn over_string_array(
    fields: Option<&serde_json::Map<String, Value>>,
    field: &str,
    raw_json: Option<&str>,
) -> Vec<String> {
    let mut arr: Vec<Value> = raw_json
        .and_then(|s| serde_json::from_str::<Vec<Value>>(s).ok())
        .unwrap_or_default();
    if let Some(f) = fields {
        for (path, val) in f {
            if let Some(rest) = path.strip_prefix(&format!("{}.", field)) {
                if let Ok(idx) = rest.parse::<usize>() {
                    if idx < arr.len() {
                        arr[idx] = if val.is_null() {
                            Value::String(String::new())
                        } else {
                            val.clone()
                        };
                    }
                }
            }
        }
    }
    arr.into_iter()
        .filter_map(|v| v.as_str().map(String::from))
        .filter(|s| !s.trim().is_empty())
        .collect()
}

fn push_kv(s: &mut String, label: &str, val: Option<&str>) {
    if let Some(v) = val {
        if !v.trim().is_empty() {
            s.push_str(&format!("- {}: {}\n", label, v));
        }
    }
}

fn push_string_list(s: &mut String, label: &str, values: &[String]) {
    if !values.is_empty() {
        s.push_str(&format!("- {}: {}\n", label, values.join("、")));
    }
}

fn format_amount(amount: f64) -> String {
    if amount.abs() >= 10_000.0 {
        format!("{} 元({:.2} 万)", amount as i64, amount / 10_000.0)
    } else {
        format!("{} 元", amount as i64)
    }
}

/// 清洗 + parse 数值字符串。失败返回 None。
fn parse_clean_number(raw: &str) -> Option<f64> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !matches!(c, '¥' | '￥' | '$' | '元' | ',' | '，' | ' ' | '　'))
        .collect();
    cleaned.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// 把 Value 转成 row key 可用的字符串(支持字符串/数字/布尔)。
fn json_as_string(v: Option<&Value>) -> String {
    v.map(|val| match val {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    })
    .unwrap_or_default()
}

/// 按字段类型计算 stable row key,与前端 `userOverrides.ts` 的 `rowKeyOf` 保持一致。
fn row_key_for(field: &str, row: &Value) -> String {
    let obj = row.as_object();
    let s = |key: &str| json_as_string(obj.and_then(|o| o.get(key)));
    match field {
        "agg_party_contacts" | "agg_court_contacts" => format!("{}|{}", s("name"), s("role")),
        "agg_key_dates" => {
            let event = s("event_type");
            let event = if event.is_empty() { s("event") } else { event };
            format!("{}|{}", event, s("date"))
        }
        "agg_fees" => format!("{}|{}", s("item"), s("amount")),
        _ => String::new(),
    }
}

/// 取子表行内字段的覆盖值。
fn over_subtable_field<'a>(
    fields: Option<&'a serde_json::Map<String, Value>>,
    field: &str,
    row_key: &str,
    inner: &str,
    raw: Option<&'a str>,
) -> Option<&'a str> {
    let path = format!("{}.{}{}.{}", field, '{', row_key, inner);
    if let Some(v) = fields.and_then(|m| m.get(&path)) {
        if v.is_null() {
            return Some("");
        }
        if let Some(s) = v.as_str() {
            return Some(s);
        }
    }
    raw
}

fn parse_bool_str(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "true" | "是" | "1" | "yes" => Some(true),
        "false" | "否" | "0" | "no" => Some(false),
        _ => None,
    }
}

fn summarize_party_contacts(json: &str, fields: Option<&serde_json::Map<String, Value>>) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return String::new();
    };
    let Some(arr) = v.as_array() else {
        return String::new();
    };
    let mut out = String::new();
    for item in arr {
        let row_key = row_key_for("agg_party_contacts", item);
        let name = over_subtable_field(fields, "agg_party_contacts", &row_key, "name", item.get("name").and_then(|x| x.as_str())).unwrap_or("");
        let role = over_subtable_field(fields, "agg_party_contacts", &row_key, "role", item.get("role").and_then(|x| x.as_str())).unwrap_or("");
        let phone = over_subtable_field(fields, "agg_party_contacts", &row_key, "phone", item.get("phone").and_then(|x| x.as_str())).unwrap_or("");
        let aliases = item.get("aliases").and_then(|x| x.as_array());
        // 2026-06-13:把 is_our_side 标出来(此前读了却丢弃 → AI 看不到谁是我方,各 chip 只能瞎猜)。
        let is_our_side_raw = item.get("is_our_side").and_then(|x| x.as_bool());
        let is_our_side = over_subtable_field(fields, "agg_party_contacts", &row_key, "is_our_side", None)
            .and_then(parse_bool_str)
            .or(is_our_side_raw);
        if name.is_empty() && role.is_empty() {
            continue;
        }
        let side = match is_our_side {
            Some(true) => " [我方]",
            Some(false) => " [对方]",
            None => "",
        };
        out.push_str(&format!("- {} ({}){}", name, role, side));
        if !phone.is_empty() {
            out.push_str(&format!(", 电话 {}", phone));
        }
        if let Some(al) = aliases {
            let als: Vec<String> = al
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect();
            if !als.is_empty() {
                out.push_str(&format!(", 别名: {}", als.join("、")));
            }
        }
        out.push('\n');
    }
    out
}

fn summarize_court_contacts(json: &str, fields: Option<&serde_json::Map<String, Value>>) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return String::new();
    };
    let Some(arr) = v.as_array() else {
        return String::new();
    };
    let mut out = String::new();
    for item in arr {
        let row_key = row_key_for("agg_court_contacts", item);
        let name = over_subtable_field(fields, "agg_court_contacts", &row_key, "name", item.get("name").and_then(|x| x.as_str())).unwrap_or("");
        let role = over_subtable_field(fields, "agg_court_contacts", &row_key, "role", item.get("role").and_then(|x| x.as_str())).unwrap_or("");
        let phone = over_subtable_field(fields, "agg_court_contacts", &row_key, "phone", item.get("phone").and_then(|x| x.as_str())).unwrap_or("");
        if name.is_empty() && role.is_empty() {
            continue;
        }
        out.push_str(&format!("- {} ({})", name, role));
        if !phone.is_empty() {
            out.push_str(&format!(", 电话 {}", phone));
        }
        out.push('\n');
    }
    out
}

fn summarize_key_dates(json: &str, fields: Option<&serde_json::Map<String, Value>>) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return String::new();
    };
    let Some(arr) = v.as_array() else {
        return String::new();
    };
    let mut out = String::new();
    for item in arr {
        let row_key = row_key_for("agg_key_dates", item);
        let date = over_subtable_field(fields, "agg_key_dates", &row_key, "date", item.get("date").and_then(|x| x.as_str())).unwrap_or("");
        let event = over_subtable_field(fields, "agg_key_dates", &row_key, "event", item.get("event").and_then(|x| x.as_str())).unwrap_or("");
        let note = over_subtable_field(fields, "agg_key_dates", &row_key, "note", item.get("note").and_then(|x| x.as_str()));
        if date.is_empty() || event.is_empty() {
            continue;
        }
        out.push_str(&format!("- {} — {}", date, event));
        if let Some(n) = note {
            if !n.trim().is_empty() {
                out.push_str(&format!("({})", n));
            }
        }
        out.push('\n');
    }
    out
}

fn summarize_fees(json: &str, fields: Option<&serde_json::Map<String, Value>>) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return String::new();
    };
    let Some(arr) = v.as_array() else {
        return String::new();
    };
    let mut out = String::new();
    for item in arr {
        let row_key = row_key_for("agg_fees", item);
        let item_name = over_subtable_field(fields, "agg_fees", &row_key, "item", item.get("item").and_then(|x| x.as_str())).unwrap_or("");
        let amount_override = over_subtable_field(fields, "agg_fees", &row_key, "amount", None);
        let note = over_subtable_field(fields, "agg_fees", &row_key, "note", item.get("note").and_then(|x| x.as_str()));
        if item_name.is_empty() {
            continue;
        }
        let amount_str = if let Some(s) = amount_override {
            if s.trim().is_empty() {
                String::new()
            } else {
                parse_clean_number(s).map(format_amount).unwrap_or_default()
            }
        } else {
            match item.get("amount") {
                Some(Value::Number(n)) => n.as_f64().map(format_amount).unwrap_or_default(),
                Some(Value::String(s)) => s.clone(),
                _ => String::new(),
            }
        };
        out.push_str(&format!("- {} {}", item_name, amount_str));
        if let Some(n) = note {
            if !n.trim().is_empty() {
                out.push_str(&format!(" — {}", n));
            }
        }
        out.push('\n');
    }
    out
}

fn indent_block(s: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    s.lines()
        .map(|l| format!("{}{}", pad, l))
        .collect::<Vec<_>>()
        .join("\n")
}

// =============================================================================
// 文档段拼装
// =============================================================================

/// Lightweight:列每份文档的 filename + category + extracted_fields 关键字段。
/// 不读 extracted_text_path 全文。
/// V0.2 D4-D5 起 `chat::constitution` 复用 — `pub(crate)`。
pub(crate) fn lightweight_docs_md(docs: &[Document]) -> (String, Vec<String>) {
    let mut active: Vec<&Document> = docs
        .iter()
        .filter(|d| !d.missing && d.deleted_at.is_none())
        .collect();

    if active.is_empty() {
        return ("(本案暂无文档材料)\n".to_string(), vec![]);
    }

    // 🥇 重要性排序:文档摘要总长超 DOC_SECTION_CHAR_LIMIT 会从尾部截断,排序保证
    // 切掉的是「不重要的」,别把关键证据切没。优先级(排前=不被截):
    //   ① 置顶(pinned)② 非 AI 产物(原始材料 > AI 生成报告,防自证循环)
    //   ③ 非归档类(证据/实体材料 > 风险告知/笔录等程序归档)④ 最近(created_at 降序)。
    active.sort_by(|a, b| {
        b.pinned_at
            .is_some()
            .cmp(&a.pinned_at.is_some())
            .then_with(|| a.is_ai_artifact.cmp(&b.is_ai_artifact))
            .then_with(|| {
                crate::ingest::pipeline::is_archival_category(a.category.as_deref()).cmp(
                    &crate::ingest::pipeline::is_archival_category(b.category.as_deref()),
                )
            })
            .then_with(|| b.created_at.cmp(&a.created_at))
    });

    let mut out = String::with_capacity(active.len() * 200);
    let mut ids = Vec::with_capacity(active.len());
    out.push_str(&format!("共 {} 份文档:\n\n", active.len()));

    for d in &active {
        let block = format_doc_light(d);
        if block.chars().count() > PER_DOC_LIGHT_CHAR_LIMIT {
            let trimmed: String = block.chars().take(PER_DOC_LIGHT_CHAR_LIMIT).collect();
            out.push_str(&trimmed);
            out.push_str("[…摘要已截断]\n");
        } else {
            out.push_str(&block);
        }
        out.push('\n');
        ids.push(d.id.clone());
    }
    (out, ids)
}

fn format_doc_light(d: &Document) -> String {
    let mut s = String::with_capacity(256);
    s.push_str(&format!("### 文档 · {}\n", d.filename));
    if let Some(cat) = &d.category {
        s.push_str(&format!("- 分类: {}\n", cat));
    }
    if let Some(stage) = &d.stage {
        s.push_str(&format!("- 阶段: {}\n", stage));
    }
    if d.is_ai_artifact {
        s.push_str(&format!(
            "- AI 生成材料(来源: {}),供参考,**不能作为原始证据**\n",
            d.source
        ));
    }
    // extracted_fields 里挑几个关键字段摘要(避免整段 JSON 太长)
    if let Some(json) = &d.extracted_fields {
        if let Some(brief) = summarize_extracted_fields(json) {
            s.push_str(&brief);
        }
    }
    s
}

/// 从 extracted_fields JSON 里挑案件相关的字段简化输出。
fn summarize_extracted_fields(json: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let obj = v.as_object()?;
    let mut s = String::new();
    let pick = |k: &str| -> Option<String> {
        obj.get(k).and_then(|x| match x {
            serde_json::Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    };
    let mut push = |label: &str, v: Option<String>| {
        if let Some(val) = v {
            s.push_str(&format!("- {}: {}\n", label, val));
        }
    };
    push("案号", pick("case_no"));
    push("案由", pick("cause"));
    push("立案日", pick("filed_at"));
    push("受理法院", pick("court"));
    push("阶段", pick("case_stage"));
    push("金额", pick("claim_amount"));
    push("备注", pick("case_note"));

    // 当事人(取前 3 个)
    for key in ["plaintiffs", "defendants", "third_parties"] {
        if let Some(arr) = obj.get(key).and_then(|x| x.as_array()) {
            let names: Vec<String> = arr
                .iter()
                .filter_map(|x| x.as_str().map(String::from))
                .take(3)
                .collect();
            if !names.is_empty() {
                let label = match key {
                    "plaintiffs" => "原告",
                    "defendants" => "被告",
                    "third_parties" => "第三人",
                    _ => key,
                };
                s.push_str(&format!("- {}: {}\n", label, names.join("、")));
            }
        }
    }

    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// =============================================================================
// 测试
// =============================================================================
