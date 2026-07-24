use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

use super::cases::{effective_our_side, Case};

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CaseLog {
    pub id: String,
    pub case_id: String,
    pub occurred_at: String,
    pub content: String,
    pub source: Option<String>,
    pub source_doc_id: Option<String>,
    pub source_path: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TimelineItem {
    date: Option<String>,
    event: Option<String>,
    event_type: Option<String>,
    note: Option<String>,
}

pub async fn list(pool: &SqlitePool, case_id: &str) -> Result<Vec<CaseLog>, String> {
    sqlx::query_as::<_, CaseLog>(
        "SELECT l.id, l.case_id, l.occurred_at, l.content, l.source, l.source_doc_id, \
                d.source_path AS source_path, l.created_at \
         FROM case_logs l \
         LEFT JOIN documents d ON d.id = l.source_doc_id \
         WHERE l.case_id = ? ORDER BY l.occurred_at DESC, l.created_at DESC",
    )
    .bind(case_id)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())
}

pub async fn generate_work_report(pool: &SqlitePool, case_id: &str) -> Result<String, String> {
    let case = super::cases::get_case(pool, case_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("案件不存在:{case_id}"))?;
    let logs = list(pool, case_id).await?;
    Ok(build_work_report_markdown(&case, &logs))
}

pub async fn export_work_report_docx(
    pool: &SqlitePool,
    case_id: &str,
    save_path: &str,
) -> Result<String, String> {
    let body = generate_work_report(pool, case_id).await?;
    let bytes = crate::docx_filing::build_report_docx_bytes("案件工作汇报", &body, None)?;
    std::fs::write(save_path, bytes).map_err(|e| format!("写入工作汇报 Word 失败:{e}"))?;
    Ok(save_path.to_string())
}

pub async fn create(
    pool: &SqlitePool,
    case_id: &str,
    occurred_at: &str,
    raw_input: &str,
    organized_markdown: Option<&str>,
) -> Result<CaseLog, String> {
    let base = crate::db::app_data_dir().map_err(|e| e.to_string())?;
    create_at_base(
        pool,
        &base,
        case_id,
        occurred_at,
        raw_input,
        organized_markdown,
    )
    .await
}

async fn create_at_base(
    pool: &SqlitePool,
    base: &std::path::Path,
    case_id: &str,
    occurred_at: &str,
    raw_input: &str,
    organized_markdown: Option<&str>,
) -> Result<CaseLog, String> {
    let raw = raw_input.trim();
    if raw.is_empty() {
        return Err("工作记录不能为空".into());
    }
    let id = Uuid::new_v4().to_string();
    let dir = base.join("case_notes").join(case_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建工作记录目录失败:{e}"))?;
    let path = dir.join(format!("{id}.md"));
    let body = if let Some(organized) = organized_markdown.filter(|v| !v.trim().is_empty()) {
        format!(
            "# 案件工作记录\n\n- 记录时间：{occurred_at}\n- 记录方式：AI 整理（律师复核）\n\n## 整理内容\n\n{}\n\n## 原始记录\n\n{}\n",
            organized.trim(), raw
        )
    } else {
        format!(
            "# 案件工作记录\n\n- 记录时间：{occurred_at}\n- 记录方式：直接记录\n\n## 记录内容\n\n{raw}\n"
        )
    };
    std::fs::write(&path, &body).map_err(|e| format!("写入工作记录失败:{e}"))?;

    let path_text = path.to_string_lossy().into_owned();
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    let result = async {
        sqlx::query(
            "INSERT INTO documents \
             (id, case_id, source_path, filename, category, is_ai_artifact, size_bytes, \
              extracted_text_path, extraction_status, source) \
             VALUES (?, ?, ?, ?, '工作记录', 1, ?, ?, 'done', 'case_note')",
        )
        .bind(&id)
        .bind(case_id)
        .bind(&path_text)
        .bind(format!(
            "工作记录-{}.md",
            occurred_at.replace([':', 'T'], "-")
        ))
        .bind(body.len() as i64)
        .bind(&path_text)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO case_logs (id, case_id, occurred_at, content, source, source_doc_id) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(case_id)
        .bind(occurred_at)
        .bind(&body)
        .bind(if organized_markdown.is_some() {
            "ai"
        } else {
            "manual"
        })
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(error) = result {
        let _ = std::fs::remove_file(&path);
        return Err(error.to_string());
    }
    sqlx::query_as::<_, CaseLog>(
        "SELECT l.id, l.case_id, l.occurred_at, l.content, l.source, l.source_doc_id, \
                d.source_path AS source_path, l.created_at \
         FROM case_logs l \
         LEFT JOIN documents d ON d.id = l.source_doc_id \
         WHERE l.id = ?",
    )
    .bind(&id)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())
}

fn build_work_report_markdown(case: &Case, logs: &[CaseLog]) -> String {
    let timeline = parse_timeline(case);
    let mut out = String::new();
    out.push_str("# 案件工作汇报\n\n");
    out.push_str("> 本汇报基于 CaseBoard 当前案件画像、办案时间轴和已保存工作记录自动生成,供沟通进展使用;具体法律意见、诉讼策略和案件结果仍以正式书面文件为准。\n\n");
    out.push_str("## 一、案件概况\n\n");
    push_fact(&mut out, "案件名称", &case.name);
    push_fact(
        &mut out,
        "案号",
        case.agg_case_no
            .as_deref()
            .or(case.case_no.as_deref())
            .unwrap_or("未填写"),
    );
    push_fact(
        &mut out,
        "承办机构",
        case.agg_court
            .as_deref()
            .or(case.court.as_deref())
            .unwrap_or("未填写"),
    );
    push_fact(
        &mut out,
        "案由",
        case.agg_cause
            .as_deref()
            .or(case.cause.as_deref())
            .unwrap_or("未填写"),
    );
    push_fact(&mut out, "案件状态", &display_case_status(case));
    push_fact(
        &mut out,
        "我方立场",
        &effective_our_side(
            case.agg_our_side.as_deref(),
            case.user_overrides_json.as_deref(),
        )
        .unwrap_or_else(|| "未识别".to_string()),
    );
    if let Some(amount) = case.agg_claim_amount {
        push_fact(&mut out, "标的金额", &format_money(amount));
    }
    out.push('\n');

    out.push_str("## 二、办案时间轴\n\n");
    if timeline.is_empty() {
        out.push_str("暂无已识别办案时间轴节点。\n\n");
    } else {
        for item in &timeline {
            let event = item.event_type.as_deref().or(item.event.as_deref());
            out.push_str(&format!(
                "- **{}**:{}{}\n",
                item.date.as_deref().unwrap_or("日期待补"),
                event.unwrap_or("办案节点"),
                item.note
                    .as_deref()
                    .filter(|v| !v.trim().is_empty())
                    .map(|v| format!(" — {}", v.trim()))
                    .unwrap_or_default()
            ));
        }
        out.push('\n');
    }

    out.push_str("## 三、工作记录概览\n\n");
    out.push_str(&format!("- 已记录工作事项:{} 条\n", logs.len()));
    out.push_str(&format!("- 已识别办案节点:{} 个\n", timeline.len()));
    if let (Some(first), Some(last)) = (logs.last(), logs.first()) {
        out.push_str(&format!(
            "- 记录时间范围:{} 至 {}\n",
            first.occurred_at.replace('T', " "),
            last.occurred_at.replace('T', " ")
        ));
    }
    out.push_str("- 说明:以下按时间顺序列示,仅呈现已保存到本案的工作日志。\n\n");

    out.push_str("## 四、工作日志明细\n\n");
    if logs.is_empty() {
        out.push_str("暂无已保存工作记录。\n");
    } else {
        for log in logs.iter().rev() {
            out.push_str(&format!(
                "### {} · {}\n\n",
                log.occurred_at.replace('T', " "),
                if log.source.as_deref() == Some("ai") {
                    "整理记录"
                } else {
                    "直接记录"
                }
            ));
            out.push_str(&display_log_content(&log.content));
            out.push_str("\n\n");
        }
    }
    out
}

fn parse_timeline(case: &Case) -> Vec<TimelineItem> {
    let mut items: Vec<TimelineItem> = case
        .agg_key_dates
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<TimelineItem>>(raw).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|item| {
            item.date.as_deref().is_some_and(|v| !v.trim().is_empty())
                || item
                    .event
                    .as_deref()
                    .or(item.event_type.as_deref())
                    .is_some_and(|v| !v.trim().is_empty())
        })
        .collect();
    apply_timeline_overrides(&mut items, case.user_overrides_json.as_deref());
    items.retain(|item| {
        item.date.as_deref().is_some_and(|v| !v.trim().is_empty())
            || item
                .event
                .as_deref()
                .or(item.event_type.as_deref())
                .is_some_and(|v| !v.trim().is_empty())
    });
    items.sort_by(|a, b| {
        let ad = a.date.as_deref().unwrap_or("");
        let bd = b.date.as_deref().unwrap_or("");
        ad.cmp(bd)
            .then_with(|| timeline_label(a).cmp(&timeline_label(b)))
    });
    items
}

fn apply_timeline_overrides(items: &mut Vec<TimelineItem>, overrides_json: Option<&str>) {
    let Some(raw) = overrides_json else {
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let deleted = v
        .get("deleted_rows")
        .and_then(|d| d.get("agg_key_dates"))
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();
    if !deleted.is_empty() {
        items.retain(|item| !deleted.contains(&timeline_row_key(item)));
    }
    let Some(fields) = v.get("fields").and_then(|f| f.as_object()) else {
        return;
    };
    for item in items {
        let key = timeline_row_key(item);
        for inner in ["event_type", "date", "note"] {
            let path = format!("agg_key_dates.{{{key}}}.{inner}");
            let Some(raw_value) = fields.get(&path) else {
                continue;
            };
            let value = raw_value.as_str().map(str::trim).filter(|s| !s.is_empty());
            match inner {
                "event_type" => item.event_type = value.map(str::to_string),
                "date" => item.date = value.map(str::to_string),
                "note" => item.note = value.map(str::to_string),
                _ => {}
            }
        }
    }
}

fn timeline_row_key(item: &TimelineItem) -> String {
    format!(
        "{}|{}",
        timeline_label(item),
        item.date.as_deref().unwrap_or("")
    )
}

fn timeline_label(item: &TimelineItem) -> String {
    item.event_type
        .as_deref()
        .or(item.event.as_deref())
        .unwrap_or("")
        .to_string()
}

fn push_fact(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!("- **{}**: {}\n", label, value.trim()));
}

fn display_case_status(case: &Case) -> String {
    if let Some(status) = case.workflow_status.as_deref() {
        return workflow_status_label(status).to_string();
    }
    case.agg_status_text
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(case.case_status.as_str())
        .to_string()
}

fn workflow_status_label(status: &str) -> &str {
    match status {
        "intake" => "接案",
        "filing" => "立案中",
        "arbitration" => "仲裁中",
        "awaiting_hearing" => "待开庭",
        "trial" => "审理中",
        "mediated" => "已调解",
        "appeal_window" => "上诉期",
        "appeal" => "二审中",
        "retrial" => "再审中",
        "execution" => "执行中",
        "closed" => "已结案",
        other => other,
    }
}

fn display_log_content(markdown: &str) -> String {
    let normalized = markdown.replace("\r\n", "\n");
    if let Some(content) = between(&normalized, "## 整理内容\n\n", "\n\n## 原始记录") {
        return content.trim().to_string();
    }
    if let Some((_, content)) = normalized.split_once("## 记录内容\n\n") {
        return content.trim().to_string();
    }
    normalized
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let (_, rest) = text.split_once(start)?;
    let (content, _) = rest.split_once(end)?;
    Some(content)
}

fn format_money(value: f64) -> String {
    if value >= 10_000_000.0 {
        format!("{:.2} 亿元", value / 100_000_000.0)
    } else if value >= 10_000.0 {
        format!("{:.2} 万元", value / 10_000.0)
    } else {
        format!("{:.2} 元", value)
    }
}
