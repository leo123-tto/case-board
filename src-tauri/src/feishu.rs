//! 飞书多维表格同步。
//!
//! 同步只复用本机 `lark-cli --as user` 登录态,CaseBoard 不保存飞书 token。
//! 失败不应该影响本地案件状态更新,调用方决定是否把错误暴露给前端。

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::process::Command;
use tokio::time::timeout;

use crate::db::cases::Case;
use crate::settings::Settings;

const LARK_CLI_TIMEOUT: Duration = Duration::from_secs(30);

const CASE_ID_FIELDS: &[&str] = &["CaseBoard ID", "案件ID", "本地ID", "case_id"];
const NAME_FIELDS: &[&str] = &["案件名称", "案件名", "名称", "Name"];
const PATH_FIELDS: &[&str] = &[
    "本地路径",
    "案件目录",
    "本地案件目录",
    "CaseBoard路径",
    "source_folder",
];
const STAGE_FIELDS: &[&str] = &["当前阶段", "案件阶段", "工作流状态", "阶段"];
const STATUS_FIELDS: &[&str] = &["案件状态", "状态"];
const CASE_NO_FIELDS: &[&str] = &["案号", "案件编号", "case_no"];
const COURT_FIELDS: &[&str] = &["受理法院", "受理机关", "法院"];
const CAUSE_FIELDS: &[&str] = &["案由", "纠纷类型"];
const SUMMARY_FIELDS: &[&str] = &["CaseBoard备注", "看板备注", "同步备注"];
const UPDATED_FIELDS: &[&str] = &["CaseBoard更新时间", "看板更新时间"];
const NEXT_MILESTONE_FIELDS: &[&str] = &["下一节点", "下一关键节点"];

#[derive(Debug, Clone, Serialize)]
pub struct FeishuSyncResult {
    pub enabled: bool,
    pub synced: bool,
    /// disabled / missing_config / skipped / created / updated
    pub action: String,
    pub record_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone)]
struct FieldMeta {
    name: String,
    type_code: Option<i64>,
}

#[derive(Debug, Clone)]
struct Record {
    id: String,
    fields: Map<String, Value>,
}

pub async fn sync_case(settings: &Settings, case_data: &Case) -> Result<FeishuSyncResult, String> {
    if !settings.feishu_enabled.unwrap_or(false) {
        return Ok(FeishuSyncResult {
            enabled: false,
            synced: false,
            action: "disabled".into(),
            record_id: None,
            message: "飞书案件池同步未启用".into(),
        });
    }

    if case_data.source_folder == "__DEMO__" {
        return Ok(FeishuSyncResult {
            enabled: true,
            synced: false,
            action: "skipped".into(),
            record_id: None,
            message: "示例案件没有真实案件目录,已跳过飞书同步".into(),
        });
    }

    let app_token = clean_required(settings.feishu_app_token.as_deref());
    let table_id = clean_required(settings.feishu_cases_table_id.as_deref());
    let (Some(app_token), Some(table_id)) = (app_token, table_id) else {
        return Ok(FeishuSyncResult {
            enabled: true,
            synced: false,
            action: "missing_config".into(),
            record_id: None,
            message: "飞书同步缺少 App Token 或案件池 Table ID".into(),
        });
    };

    let fields = list_fields(app_token, table_id).await?;
    let fields_by_name = fields
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect::<HashMap<_, _>>();
    let payload = build_case_fields(case_data, &fields_by_name);
    if payload.is_empty() {
        return Ok(FeishuSyncResult {
            enabled: true,
            synced: false,
            action: "skipped".into(),
            record_id: None,
            message: "目标表没有可写入的案件字段".into(),
        });
    }

    let records = list_records(app_token, table_id).await?;
    let existing = find_matching_record(&records, case_data);
    let (action, record_id, response) = if let Some(record) = existing {
        let path = format!(
            "/open-apis/bitable/v1/apps/{}/tables/{}/records/{}",
            app_token, table_id, record.id
        );
        (
            "updated",
            Some(record.id.clone()),
            lark_cli_api("PUT", &path, Some(json!({ "fields": payload }))).await?,
        )
    } else {
        let path = format!(
            "/open-apis/bitable/v1/apps/{}/tables/{}/records",
            app_token, table_id
        );
        (
            "created",
            None,
            lark_cli_api("POST", &path, Some(json!({ "fields": payload }))).await?,
        )
    };

    let record_id = record_id.or_else(|| {
        response
            .pointer("/data/record/record_id")
            .and_then(Value::as_str)
            .or_else(|| response.pointer("/data/record_id").and_then(Value::as_str))
            .map(str::to_string)
    });

    Ok(FeishuSyncResult {
        enabled: true,
        synced: true,
        action: action.into(),
        record_id,
        message: match action {
            "updated" => "飞书案件池记录已更新".into(),
            _ => "飞书案件池记录已创建".into(),
        },
    })
}

async fn list_fields(app_token: &str, table_id: &str) -> Result<Vec<FieldMeta>, String> {
    let path = format!(
        "/open-apis/bitable/v1/apps/{}/tables/{}/fields?page_size=100",
        app_token, table_id
    );
    let value = lark_cli_api("GET", &path, None).await?;
    let items = value
        .pointer("/data/items")
        .and_then(Value::as_array)
        .ok_or_else(|| "飞书字段列表响应缺少 data.items".to_string())?;

    Ok(items
        .iter()
        .filter_map(|item| {
            let name = item
                .get("field_name")
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)?
                .to_string();
            Some(FieldMeta {
                name,
                type_code: item.get("type").and_then(Value::as_i64),
            })
        })
        .collect())
}

async fn list_records(app_token: &str, table_id: &str) -> Result<Vec<Record>, String> {
    let path = format!(
        "/open-apis/bitable/v1/apps/{}/tables/{}/records?page_size=500&field_names=true",
        app_token, table_id
    );
    let value = lark_cli_api("GET", &path, None).await?;
    let items = value
        .pointer("/data/items")
        .and_then(Value::as_array)
        .or_else(|| value.pointer("/data/records").and_then(Value::as_array))
        .ok_or_else(|| "飞书记录列表响应缺少 data.items".to_string())?;

    Ok(items
        .iter()
        .filter_map(|item| {
            let id = item
                .get("record_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)?
                .to_string();
            let fields = item.get("fields")?.as_object()?.clone();
            Some(Record { id, fields })
        })
        .collect())
}

async fn lark_cli_api(method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
    let mut cmd = Command::new(lark_cli_bin());
    cmd.env("LARK_CLI_NO_PROXY", "1")
        .env(
            "PATH",
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        )
        .arg("api")
        .arg(method)
        .arg(path)
        .arg("--as")
        .arg("user")
        .arg("--format")
        .arg("json");

    if let Some(body) = body {
        cmd.arg("--data")
            .arg(serde_json::to_string(&body).map_err(|e| e.to_string())?);
    }

    let output = timeout(LARK_CLI_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "lark-cli 调用超时".to_string())?
        .map_err(|e| format!("无法启动 lark-cli: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "lark-cli 调用失败: {}{}",
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!(" · {}", stdout.trim())
            }
        ));
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|e| format!("lark-cli 输出非 UTF-8: {}", e))?;
    let value: Value =
        serde_json::from_str(&stdout).map_err(|e| format!("lark-cli 输出非 JSON: {}", e))?;
    ensure_lark_ok(value)
}

fn lark_cli_bin() -> &'static str {
    if Path::new("/opt/homebrew/bin/lark-cli").exists() {
        "/opt/homebrew/bin/lark-cli"
    } else if Path::new("/usr/local/bin/lark-cli").exists() {
        "/usr/local/bin/lark-cli"
    } else {
        "lark-cli"
    }
}

fn ensure_lark_ok(value: Value) -> Result<Value, String> {
    if let Some(code) = value.get("code").and_then(Value::as_i64) {
        if code != 0 {
            let msg = value
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(format!("飞书 API 返回 code={}: {}", code, msg));
        }
    }
    Ok(value)
}

fn build_case_fields(case_data: &Case, fields: &HashMap<&str, &FieldMeta>) -> Map<String, Value> {
    let mut out = Map::new();
    let stage = case_data
        .workflow_status
        .as_deref()
        .map(workflow_status_label)
        .filter(|s| !s.trim().is_empty());
    let status_text = stage.unwrap_or(case_data.case_status.as_str());

    set_first(&mut out, fields, CASE_ID_FIELDS, &case_data.id);
    set_first(&mut out, fields, NAME_FIELDS, &case_data.name);
    set_all(&mut out, fields, STAGE_FIELDS, status_text);
    set_all(&mut out, fields, STATUS_FIELDS, status_text);
    set_first(&mut out, fields, PATH_FIELDS, &case_data.source_folder);
    if let Some(v) = non_empty(
        case_data
            .case_no
            .as_deref()
            .or(case_data.agg_case_no.as_deref()),
    ) {
        set_first(&mut out, fields, CASE_NO_FIELDS, v);
    }
    if let Some(v) = non_empty(
        case_data
            .court
            .as_deref()
            .or(case_data.agg_court.as_deref()),
    ) {
        set_first(&mut out, fields, COURT_FIELDS, v);
    }
    if let Some(v) = non_empty(
        case_data
            .cause
            .as_deref()
            .or(case_data.agg_cause.as_deref()),
    ) {
        set_first(&mut out, fields, CAUSE_FIELDS, v);
    }
    if let Some(v) = non_empty(case_data.case_summary.as_deref()) {
        set_first(&mut out, fields, SUMMARY_FIELDS, v);
    }
    set_first(&mut out, fields, UPDATED_FIELDS, &case_data.updated_at);
    if let Some(v) = non_empty(case_data.next_milestone_note.as_deref()) {
        set_first(&mut out, fields, NEXT_MILESTONE_FIELDS, v);
    }
    out
}

fn set_first(
    out: &mut Map<String, Value>,
    fields: &HashMap<&str, &FieldMeta>,
    candidates: &[&str],
    value: &str,
) {
    let Some(value) = non_empty(Some(value)) else {
        return;
    };
    if let Some(name) = candidates.iter().copied().find(|name| {
        fields
            .get(name)
            .is_some_and(|field| field_accepts_string(field))
    }) {
        out.insert(name.to_string(), Value::String(value.to_string()));
    }
}

fn set_all(
    out: &mut Map<String, Value>,
    fields: &HashMap<&str, &FieldMeta>,
    candidates: &[&str],
    value: &str,
) {
    let Some(value) = non_empty(Some(value)) else {
        return;
    };
    for name in candidates.iter().copied().filter(|name| {
        fields
            .get(name)
            .is_some_and(|field| field_accepts_string(field))
    }) {
        out.insert(name.to_string(), Value::String(value.to_string()));
    }
}

fn field_accepts_string(field: &FieldMeta) -> bool {
    // 1=文本,3=单选,13=电话。None 用于兼容 CLI/API 字段响应缺类型的情况。
    matches!(field.type_code, None | Some(1 | 3 | 13))
}

fn find_matching_record<'a>(records: &'a [Record], case_data: &Case) -> Option<&'a Record> {
    records
        .iter()
        .find(|record| any_field_equals(&record.fields, CASE_ID_FIELDS, &case_data.id))
        .or_else(|| {
            records.iter().find(|record| {
                any_field_equals(&record.fields, PATH_FIELDS, &case_data.source_folder)
            })
        })
        .or_else(|| {
            records
                .iter()
                .find(|record| any_field_equals(&record.fields, NAME_FIELDS, &case_data.name))
        })
}

fn any_field_equals(fields: &Map<String, Value>, names: &[&str], expected: &str) -> bool {
    let expected = expected.trim();
    if expected.is_empty() {
        return false;
    }
    names.iter().any(|name| {
        fields
            .get(*name)
            .map(value_to_plain_text)
            .is_some_and(|text| text.trim() == expected)
    })
}

fn value_to_plain_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(items) => items
            .iter()
            .map(value_to_plain_text)
            .filter(|s| !s.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        Value::Object(obj) => obj
            .get("text")
            .or_else(|| obj.get("name"))
            .or_else(|| obj.get("url"))
            .or_else(|| obj.get("link"))
            .map(value_to_plain_text)
            .unwrap_or_else(|| value.to_string()),
    }
}

fn workflow_status_label(status: &str) -> &str {
    match status {
        "intake" => "接案",
        "filing" => "立案中",
        "awaiting_hearing" => "待开庭",
        "trial" => "审理中",
        "mediated" => "已调解",
        "appeal_window" => "上诉期",
        "appeal" => "二审中",
        "execution" => "执行中",
        "closed" => "已结案",
        other => other,
    }
}

fn clean_required(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

/* ------------------------------------------------------------------ */
/* 日历表同步                                                            */
/* ------------------------------------------------------------------ */

/// 从所有 active cases 的 agg_key_dates 展开事件，同步到飞书日历表。
///
/// 日历表字段（需在飞书多维表格中手动创建）：
/// - 日期（日期类型）
/// - 事件类型（文本：开庭 / 续封 / 还款期 等）
/// - 案件名称（文本）
/// - 案号（文本）
/// - 备注（文本）
/// - 紧急度（单选：逾期 / 紧急 / 常规）
///
/// 同步策略：全量覆盖（先清空再写入），因为日历表数据量小。
pub async fn sync_calendar_table(
    settings: &Settings,
    cases: &[Case],
) -> Result<FeishuSyncResult, String> {
    if !settings.feishu_enabled.unwrap_or(false) {
        return Ok(FeishuSyncResult {
            enabled: false,
            synced: false,
            action: "disabled".into(),
            record_id: None,
            message: "飞书同步未启用".into(),
        });
    }

    let app_token = clean_required(settings.feishu_app_token.as_deref());
    let table_id = clean_required(settings.feishu_calendar_table_id.as_deref());
    let (Some(app_token), Some(table_id)) = (app_token, table_id) else {
        return Ok(FeishuSyncResult {
            enabled: true,
            synced: false,
            action: "missing_config".into(),
            record_id: None,
            message: "飞书日历表缺少 App Token 或 Table ID".into(),
        });
    };

    // 展开所有 case 的 key_dates 为扁平事件列表
    let events = expand_calendar_events(cases);
    if events.is_empty() {
        return Ok(FeishuSyncResult {
            enabled: true,
            synced: false,
            action: "skipped".into(),
            record_id: None,
            message: "没有可同步的日历事件".into(),
        });
    }

    // 读取日历表字段，确认可写字段
    let fields = list_fields(app_token, table_id).await?;
    let fields_by_name = fields
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect::<HashMap<_, _>>();

    // 清空现有记录（全量覆盖策略）
    let existing = list_records(app_token, table_id).await?;
    for record in &existing {
        let path = format!(
            "/open-apis/bitable/v1/apps/{}/tables/{}/records/{}",
            app_token, table_id, record.id
        );
        // 删除失败不阻塞（可能权限不足）
        let _ = lark_cli_api("DELETE", &path, None).await;
    }

    // 批量写入新记录
    let mut created = 0;
    for event in &events {
        let mut payload = Map::new();
        set_first(
            &mut payload,
            &fields_by_name,
            &["日期", "Date", "date"],
            &event.date,
        );
        set_first(
            &mut payload,
            &fields_by_name,
            &["事件类型", "事件", "Type"],
            &event.event_type,
        );
        set_first(
            &mut payload,
            &fields_by_name,
            &["案件名称", "案件名", "Name"],
            &event.case_name,
        );
        if let Some(v) = non_empty(event.case_no.as_deref()) {
            set_first(&mut payload, &fields_by_name, &["案号", "Case No"], v);
        }
        if let Some(v) = non_empty(event.note.as_deref()) {
            set_first(&mut payload, &fields_by_name, &["备注", "Note"], v);
        }
        set_first(
            &mut payload,
            &fields_by_name,
            &["紧急度", "Urgency"],
            &event.urgency,
        );

        if !payload.is_empty() {
            let path = format!(
                "/open-apis/bitable/v1/apps/{}/tables/{}/records",
                app_token, table_id
            );
            match lark_cli_api("POST", &path, Some(json!({ "fields": payload }))).await {
                Ok(_) => created += 1,
                Err(e) => crate::dlog!("[feishu] calendar sync record failed: {}", e),
            }
        }
    }

    Ok(FeishuSyncResult {
        enabled: true,
        synced: true,
        action: "synced".into(),
        record_id: None,
        message: format!("日历表已同步 {} 条事件", created),
    })
}

/// 日历事件（展开后的扁平结构）
struct CalendarEvent {
    date: String,
    event_type: String,
    case_name: String,
    case_no: Option<String>,
    note: Option<String>,
    urgency: String,
}

/* ------------------------------------------------------------------ */
/* 飞书日历读取                                                          */
/* ------------------------------------------------------------------ */

/// 飞书日历事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuCalendarEvent {
    pub event_id: String,
    pub summary: String,
    pub start_date: String,
    pub end_date: Option<String>,
    pub is_all_day: bool,
    pub description: Option<String>,
    pub location: Option<String>,
    pub app_link: Option<String>,
}

/// 从飞书日历获取指定日期范围内的事件。
///
/// 使用 `lark-cli calendar +agenda --as user` 获取。
pub async fn fetch_calendar_events(
    start: &str,
    end: &str,
) -> Result<Vec<FeishuCalendarEvent>, String> {
    let mut cmd = Command::new(lark_cli_bin());
    cmd.env("LARK_CLI_NO_PROXY", "1")
        .env(
            "PATH",
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        )
        .arg("calendar")
        .arg("+agenda")
        .arg("--as")
        .arg("user")
        .arg("--start")
        .arg(start)
        .arg("--end")
        .arg(end)
        .arg("--format")
        .arg("json");

    let output = timeout(LARK_CLI_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "lark-cli 日历查询超时".to_string())?
        .map_err(|e| format!("无法启动 lark-cli: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "飞书日历查询失败: {}{}",
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!(" · {}", stdout.trim())
            }
        ));
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|e| format!("lark-cli 输出非 UTF-8: {}", e))?;
    let value: Value =
        serde_json::from_str(&stdout).map_err(|e| format!("lark-cli 输出非 JSON: {}", e))?;

    let events = value
        .pointer("/data")
        .and_then(Value::as_array)
        .ok_or_else(|| "飞书日历响应缺少 data".to_string())?;

    let mut result = Vec::new();
    for event in events {
        let event_id = event
            .get("event_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let summary = event
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("(无标题)")
            .to_string();

        // 解析开始时间
        let start_time = event.get("start_time");
        let (start_date, is_all_day) = if let Some(st) = start_time {
            if let Some(date) = st.get("date").and_then(Value::as_str) {
                (date.to_string(), true)
            } else if let Some(datetime) = st.get("datetime").and_then(Value::as_str) {
                // 提取日期部分
                let date = datetime.split('T').next().unwrap_or(datetime);
                (date.to_string(), false)
            } else {
                continue;
            }
        } else {
            continue;
        };

        // 解析结束时间
        let end_date = event.get("end_time").and_then(|et| {
            et.get("date")
                .or_else(|| et.get("datetime"))
                .and_then(Value::as_str)
                .map(|s| {
                    if s.contains('T') {
                        s.split('T').next().unwrap_or(s).to_string()
                    } else {
                        s.to_string()
                    }
                })
        });

        let description = event
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string);
        let location = event
            .get("location")
            .and_then(|l| l.get("name").or_else(|| l.get("address")))
            .and_then(Value::as_str)
            .map(str::to_string);

        let app_link = event
            .get("app_link")
            .and_then(Value::as_str)
            .map(str::to_string);

        result.push(FeishuCalendarEvent {
            event_id,
            summary,
            start_date,
            end_date,
            is_all_day,
            description,
            location,
            app_link,
        });
    }

    Ok(result)
}

/// 根据事件标题在飞书案件池表格中查找匹配的本地路径。
///
/// 匹配规则：事件标题包含案件名称的任意子串（如"曾程炜案件开庭"匹配"曾程炜"），
/// 或案件名称包含事件标题的任意子串。返回第一个匹配且有本地路径的记录。
pub async fn find_case_local_path(
    settings: &Settings,
    event_summary: &str,
) -> Result<Option<String>, String> {
    if !settings.feishu_enabled.unwrap_or(false) {
        return Ok(None);
    }

    let app_token = match clean_required(settings.feishu_app_token.as_deref()) {
        Some(t) => t,
        None => return Ok(None),
    };
    let table_id = match clean_required(settings.feishu_cases_table_id.as_deref()) {
        Some(t) => t,
        None => return Ok(None),
    };

    let path = format!(
        "/open-apis/bitable/v1/apps/{}/tables/{}/records?page_size=500&field_names=true",
        app_token, table_id
    );
    let value = lark_cli_api("GET", &path, None).await?;

    let items = value
        .pointer("/data/items")
        .and_then(Value::as_array)
        .ok_or_else(|| "飞书记录列表响应缺少 data.items".to_string())?;

    // 提取事件标题中的关键部分（去掉"案件开庭"等后缀）
    let clean_summary = event_summary
        .trim()
        .trim_end_matches("案件开庭")
        .trim_end_matches("开庭")
        .trim_end_matches("案件")
        .trim_end_matches("续封")
        .trim_end_matches("到期")
        .to_string();

    for item in items {
        let fields = match item.get("fields") {
            Some(f) => f,
            None => continue,
        };

        // 检查案件名称是否匹配
        let case_name = fields.get("案件名称").and_then(Value::as_str).unwrap_or("");
        if case_name.is_empty() {
            continue;
        }

        // 匹配：事件标题包含案件名，或案件名包含事件标题（清理后）
        let matches = event_summary.contains(case_name)
            || case_name.contains(&clean_summary)
            || clean_summary.contains(case_name);

        if !matches {
            continue;
        }

        // 检查本地路径字段
        let local_path = fields
            .get("本地路径")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty());

        if let Some(path) = local_path {
            // 验证路径是否存在
            if Path::new(path).exists() {
                return Ok(Some(path.to_string()));
            }
        }
    }

    Ok(None)
}

/// 从所有 cases 的 agg_key_dates 展开日历事件
fn expand_calendar_events(cases: &[Case]) -> Vec<CalendarEvent> {
    let now = chrono::Local::now().date_naive();
    let mut events = Vec::new();

    for case_data in cases {
        let kd_json = match &case_data.agg_key_dates {
            Some(j) => j,
            None => continue,
        };
        let parsed: serde_json::Value = match serde_json::from_str(kd_json) {
            Ok(v) => v,
            _ => continue,
        };
        let arr = match parsed.as_array() {
            Some(a) => a,
            None => continue,
        };

        let case_name = case_data
            .agg_cause
            .as_deref()
            .unwrap_or(&case_data.name)
            .to_string();
        let case_no = case_data
            .case_no
            .as_deref()
            .or(case_data.agg_case_no.as_deref())
            .map(str::to_string);

        for kd in arr {
            // 开庭事件：用 date 字段
            let date = kd.get("date").and_then(Value::as_str);
            let event = kd.get("event").and_then(Value::as_str);
            if let (Some(date), Some(event)) = (date, event) {
                if event.contains("开庭") {
                    let event_date = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d");
                    if let Ok(d) = event_date {
                        let days = (d - now).num_days();
                        if days >= -7 && days <= 365 {
                            let urgency = if days < 0 {
                                "逾期".to_string()
                            } else if days <= 30 {
                                "紧急".to_string()
                            } else {
                                "常规".to_string()
                            };
                            events.push(CalendarEvent {
                                date: date.to_string(),
                                event_type: event.to_string(),
                                case_name: case_name.clone(),
                                case_no: case_no.clone(),
                                note: kd.get("note").and_then(Value::as_str).map(str::to_string),
                                urgency,
                            });
                        }
                    }
                }
            }

            // 到期事件：用 expires_at 字段
            if let Some(expires) = kd.get("expires_at").and_then(Value::as_str) {
                let event_date = chrono::NaiveDate::parse_from_str(expires, "%Y-%m-%d");
                if let Ok(d) = event_date {
                    let days = (d - now).num_days();
                    if days >= -30 && days <= 365 {
                        let event_type = event.unwrap_or("到期");
                        let urgency = if days < 0 {
                            "逾期".to_string()
                        } else if days <= 90 {
                            "紧急".to_string()
                        } else {
                            "常规".to_string()
                        };
                        events.push(CalendarEvent {
                            date: expires.to_string(),
                            event_type: event_type.to_string(),
                            case_name: case_name.clone(),
                            case_no: case_no.clone(),
                            note: kd.get("note").and_then(Value::as_str).map(str::to_string),
                            urgency,
                        });
                    }
                }
            }
        }
    }

    // 按日期排序
    events.sort_by(|a, b| a.date.cmp(&b.date));
    events
}

/// 检查即将到期的事件并通过飞书 IM 推送提醒。
///
/// 推送去重：在 `_caseboard/notified_events.json` 记录已推送的 (case_id, event_type, date) 三元组。
/// 推送身份：--as user（复用本机 lark-cli 登录态）。
pub async fn check_and_notify_expiries(
    settings: &Settings,
    cases: &[Case],
) -> Result<usize, String> {
    if !settings.feishu_notify_enabled.unwrap_or(false) {
        return Ok(0);
    }

    let user_id = match clean_required(settings.feishu_notify_user_id.as_deref()) {
        Some(id) => id.to_string(),
        None => return Err("飞书推送未配置 user_id".to_string()),
    };
    let days_before = settings.feishu_notify_days_before.unwrap_or(7) as i64;

    let now = chrono::Local::now().date_naive();
    let mut to_notify: Vec<(String, String, String, String, String)> = Vec::new();

    for case_data in cases {
        let kd_json = match &case_data.agg_key_dates {
            Some(j) => j,
            None => continue,
        };
        let parsed: serde_json::Value = match serde_json::from_str(kd_json) {
            Ok(v) => v,
            _ => continue,
        };
        let arr = match parsed.as_array() {
            Some(a) => a,
            None => continue,
        };

        let case_name = case_data.agg_cause.as_deref().unwrap_or(&case_data.name);

        for kd in arr {
            // 开庭事件
            if let (Some(date), Some(event)) = (
                kd.get("date").and_then(Value::as_str),
                kd.get("event").and_then(Value::as_str),
            ) {
                if event.contains("开庭") {
                    if let Ok(d) = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
                        let days = (d - now).num_days();
                        if days >= 0 && days <= days_before {
                            let note = kd.get("note").and_then(Value::as_str).unwrap_or("");
                            to_notify.push((
                                case_data.id.clone(),
                                event.to_string(),
                                date.to_string(),
                                case_name.to_string(),
                                format!(
                                    "📅 {} · {} · {}（距今 {} 天）{}",
                                    case_name,
                                    event,
                                    date,
                                    days,
                                    if note.is_empty() {
                                        String::new()
                                    } else {
                                        format!(" · {}", note)
                                    }
                                ),
                            ));
                        }
                    }
                }
            }

            // 到期事件
            if let Some(expires) = kd.get("expires_at").and_then(Value::as_str) {
                if let Ok(d) = chrono::NaiveDate::parse_from_str(expires, "%Y-%m-%d") {
                    let days = (d - now).num_days();
                    if days >= 0 && days <= days_before {
                        let event_type = kd.get("event").and_then(Value::as_str).unwrap_or("到期");
                        let note = kd.get("note").and_then(Value::as_str).unwrap_or("");
                        to_notify.push((
                            case_data.id.clone(),
                            event_type.to_string(),
                            expires.to_string(),
                            case_name.to_string(),
                            format!(
                                "⏰ {} · {} · {}（距今 {} 天）{}",
                                case_name,
                                event_type,
                                expires,
                                days,
                                if note.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · {}", note)
                                }
                            ),
                        ));
                    }
                }
            }
        }
    }

    if to_notify.is_empty() {
        return Ok(0);
    }

    // 去重：读取已推送记录
    let notified_path = notified_events_path();
    let mut notified: std::collections::HashSet<String> =
        if let Ok(content) = std::fs::read_to_string(&notified_path) {
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Default::default()
        };

    let mut sent = 0;
    for (case_id, event_type, date, _case_name, message) in &to_notify {
        let key = format!("{}|{}|{}", case_id, event_type, date);
        if notified.contains(&key) {
            continue;
        }

        match send_feishu_message(user_id.as_str(), message).await {
            Ok(_) => {
                notified.insert(key);
                sent += 1;
            }
            Err(e) => {
                crate::dlog!("[feishu] notify failed for {}: {}", case_id, e);
            }
        }
    }

    // 持久化已推送记录
    if sent > 0 {
        if let Ok(json) = serde_json::to_string(&notified) {
            let _ = std::fs::write(&notified_path, json);
        }
    }

    Ok(sent)
}

/// 通过飞书 IM 发送消息（--as user）。
async fn send_feishu_message(user_id: &str, message: &str) -> Result<(), String> {
    let mut cmd = Command::new(lark_cli_bin());
    cmd.env("LARK_CLI_NO_PROXY", "1")
        .env(
            "PATH",
            "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
        )
        .arg("im")
        .arg("+messages-send")
        .arg("--as")
        .arg("user")
        .arg("--user-id")
        .arg(user_id)
        .arg("--markdown")
        .arg(message);

    let output = timeout(LARK_CLI_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "lark-cli 消息发送超时".to_string())?
        .map_err(|e| format!("无法启动 lark-cli: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "飞书消息发送失败: {}{}",
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!(" · {}", stdout.trim())
            }
        ));
    }
    Ok(())
}

/// 已推送事件记录文件路径
fn notified_events_path() -> std::path::PathBuf {
    let base = crate::db::app_data_dir().unwrap_or_else(|_| std::path::PathBuf::from("/tmp"));
    let _ = std::fs::create_dir_all(&base);
    base.join("notified_events.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_case() -> Case {
        Case {
            id: "case-1".into(),
            name: "张三诉李四买卖合同纠纷".into(),
            case_type: "诉讼".into(),
            cause: Some("买卖合同纠纷".into()),
            case_no: Some("(2026)浙0304民初1号".into()),
            court: Some("温州市瓯海区人民法院".into()),
            judge_id: None,
            stage: None,
            source_folder: "/tmp/case-1".into(),
            ai_summary_md: None,
            created_at: "2026-06-10T00:00:00Z".into(),
            updated_at: "2026-06-10T01:00:00Z".into(),
            last_scanned_at: None,
            agg_case_no: None,
            agg_court: None,
            agg_cause: None,
            agg_plaintiffs: None,
            agg_defendants: None,
            agg_third_parties: None,
            agg_judges: None,
            agg_claim_amount: None,
            agg_filed_at: None,
            agg_computed_at: None,
            next_milestone_type: None,
            next_milestone_at: None,
            next_milestone_status: None,
            next_milestone_note: None,
            case_status: "进行中".into(),
            execution_total: None,
            execution_total_breakdown: None,
            execution_started_at: None,
            execution_received: None,
            execution_remaining: None,
            workflow_status: Some("trial".into()),
            case_summary: Some("等待开庭".into()),
            case_report_path: None,
            case_report_generated_at: None,
            agg_resolution: None,
            agg_status_text: None,
            agg_party_contacts: None,
            agg_court_contacts: None,
            agg_key_dates: None,
            agg_fees: None,
            risk_assessment_path: None,
            risk_assessment_at: None,
            deep_dive_report_path: None,
            deep_dive_at: None,
            full_report_path: None,
            full_report_at: None,
            user_overrides_json: None,
            agg_court_type: None,
            agg_our_side: None,
            workflow_status_locked: 0,
        }
    }

    #[test]
    fn maps_workflow_status_to_cn_label() {
        assert_eq!(workflow_status_label("trial"), "审理中");
        assert_eq!(workflow_status_label("unknown"), "unknown");
    }

    #[test]
    fn normalizes_feishu_text_segments() {
        let value = json!([
            { "type": "text", "text": "张三" },
            { "type": "text", "text": "诉李四" }
        ]);
        assert_eq!(value_to_plain_text(&value), "张三 诉李四");
    }

    #[test]
    fn finds_matching_record_by_path() {
        let mut fields = Map::new();
        fields.insert("本地路径".into(), Value::String("/tmp/case-1".into()));
        let records = vec![Record {
            id: "rec1".into(),
            fields,
        }];
        let case_data = test_case();
        assert_eq!(
            find_matching_record(&records, &case_data).map(|r| r.id.as_str()),
            Some("rec1")
        );
    }

    #[test]
    fn builds_payload_only_for_existing_writable_fields() {
        let metas = [
            FieldMeta {
                name: "案件名称".into(),
                type_code: Some(1),
            },
            FieldMeta {
                name: "当前阶段".into(),
                type_code: Some(3),
            },
            FieldMeta {
                name: "公式字段".into(),
                type_code: Some(20),
            },
        ];
        let fields = metas
            .iter()
            .map(|f| (f.name.as_str(), f))
            .collect::<HashMap<_, _>>();
        let payload = build_case_fields(&test_case(), &fields);
        assert_eq!(
            payload.get("案件名称"),
            Some(&Value::String("张三诉李四买卖合同纠纷".into()))
        );
        assert_eq!(
            payload.get("当前阶段"),
            Some(&Value::String("审理中".into()))
        );
        assert!(!payload.contains_key("公式字段"));
    }
}
