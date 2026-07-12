//! 飞书手机每日提醒。
//!
//! 具体实现由本文件测试约束：沿用外部 PR #22 的详细消息格式，复用现有案件日期数据，
//! 并补齐持久化去重、错过整点补发和飞书业务错误校验。

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use chrono::{Datelike, Local, NaiveDate, NaiveTime};
use serde_json::Value;
use sqlx::SqlitePool;

use crate::settings;

const DEFAULT_TIME: &str = "09:00";
const DEFAULT_DAYS: u32 = 7;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReminderItem {
    date: String,
    kind: String,
    content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReminderGroup {
    case_name: String,
    items: Vec<ReminderItem>,
}

fn build_message(groups: &[ReminderGroup]) -> String {
    let total: usize = groups.iter().map(|group| group.items.len()).sum();
    let mut lines = vec![format!(
        "📋 **案件待办提醒**\n> {} 个案件 · {} 条待办即将到期",
        groups.len(),
        total
    )];
    lines.push(String::new());

    for group in groups {
        lines.push(format!(
            "**{}**（{} 条）",
            group.case_name,
            group.items.len()
        ));
        for item in &group.items {
            if item.content.is_empty() {
                lines.push(format!("- {} · {}", item.date, item.kind));
            } else {
                lines.push(format!(
                    "- {} · {} · {}",
                    item.date, item.kind, item.content
                ));
            }
        }
        lines.push(String::new());
    }

    lines.push(format!("⏰ 共 {total} 条待办事项即将到期，请及时处理。"));
    lines.join("\n")
}

fn parse_feishu_response(status: u16, body: &str) -> Result<(), String> {
    if !(200..300).contains(&status) {
        return Err(format!("飞书 Webhook HTTP {status}: {body}"));
    }
    let value: Value =
        serde_json::from_str(body).map_err(|e| format!("飞书 Webhook 响应不是 JSON: {e}"))?;
    let code = value
        .get("code")
        .or_else(|| value.get("StatusCode"))
        .or_else(|| value.get("status_code"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if code != 0 {
        let message = value
            .get("msg")
            .or_else(|| value.get("StatusMessage"))
            .or_else(|| value.get("status_message"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return Err(format!("飞书 Webhook 返回 code={code}: {message}"));
    }
    Ok(())
}

fn should_send(
    today: NaiveDate,
    now: NaiveTime,
    target: NaiveTime,
    last_sent: Option<NaiveDate>,
) -> bool {
    now >= target && last_sent != Some(today)
}

fn normalize_date(raw: &str) -> Option<NaiveDate> {
    let trimmed = raw.trim();
    if let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Some(date);
    }
    if trimmed.len() == 7 {
        let first = NaiveDate::parse_from_str(&format!("{trimmed}-01"), "%Y-%m-%d").ok()?;
        let (year, month) = if first.month() == 12 {
            (first.year() + 1, 1)
        } else {
            (first.year(), first.month() + 1)
        };
        return NaiveDate::from_ymd_opt(year, month, 1)?.pred_opt();
    }
    None
}

fn push_item(
    groups: &mut BTreeMap<String, Vec<ReminderItem>>,
    seen: &mut BTreeSet<(String, String, String, String)>,
    case_name: String,
    item: ReminderItem,
) {
    let key = (
        case_name.clone(),
        item.date.clone(),
        item.kind.clone(),
        item.content.clone(),
    );
    if seen.insert(key) {
        groups.entry(case_name).or_default().push(item);
    }
}

async fn collect_pending_between(
    pool: &SqlitePool,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<ReminderGroup>, String> {
    let start_text = start.format("%Y-%m-%d").to_string();
    let end_text = end.format("%Y-%m-%d").to_string();
    let mut groups: BTreeMap<String, Vec<ReminderItem>> = BTreeMap::new();
    let mut seen = BTreeSet::new();

    let cases: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT name, agg_key_dates FROM cases WHERE agg_key_dates IS NOT NULL")
            .fetch_all(pool)
            .await
            .map_err(|e| format!("查询案件关键日期失败: {e}"))?;
    for (case_name, raw_dates) in cases {
        let Some(raw_dates) = raw_dates else { continue };
        let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&raw_dates) else {
            continue;
        };
        for value in items {
            let Some(date) = value
                .get("date")
                .and_then(Value::as_str)
                .and_then(normalize_date)
            else {
                continue;
            };
            if date < start || date > end {
                continue;
            }
            push_item(
                &mut groups,
                &mut seen,
                case_name.clone(),
                ReminderItem {
                    date: date.format("%Y-%m-%d").to_string(),
                    kind: value
                        .get("event")
                        .and_then(Value::as_str)
                        .unwrap_or("其他提醒")
                        .to_string(),
                    content: value
                        .get("note")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                },
            );
        }
    }

    let todos: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT t.due_date, t.title, c.name FROM case_todos t \
         JOIN cases c ON c.id = t.case_id \
         WHERE t.done = 0 AND t.due_date >= ? AND t.due_date <= ? \
         ORDER BY c.name, t.due_date",
    )
    .bind(&start_text)
    .bind(&end_text)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询案件待办失败: {e}"))?;
    for (date, title, case_name) in todos {
        push_item(
            &mut groups,
            &mut seen,
            case_name,
            ReminderItem {
                date,
                kind: "待办".into(),
                content: title,
            },
        );
    }

    let events: Vec<(String, Option<String>, String, String)> = sqlx::query_as(
        "SELECT substr(e.occurred_at, 1, 10), e.type, e.title, c.name FROM events e \
         JOIN cases c ON c.id = e.case_id \
         WHERE substr(e.occurred_at, 1, 10) >= ? AND substr(e.occurred_at, 1, 10) <= ? \
         ORDER BY c.name, e.occurred_at",
    )
    .bind(&start_text)
    .bind(&end_text)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询案件事件失败: {e}"))?;
    for (date, kind, title, case_name) in events {
        push_item(
            &mut groups,
            &mut seen,
            case_name,
            ReminderItem {
                date,
                kind: kind.unwrap_or_else(|| "事件".into()),
                content: title,
            },
        );
    }

    let calendar: Vec<(String, String)> = sqlx::query_as(
        "SELECT date, title FROM calendar_events WHERE date >= ? AND date <= ? ORDER BY date",
    )
    .bind(&start_text)
    .bind(&end_text)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查询个人日程失败: {e}"))?;
    for (date, title) in calendar {
        push_item(
            &mut groups,
            &mut seen,
            "📅 个人日程".into(),
            ReminderItem {
                date,
                kind: "日程".into(),
                content: title,
            },
        );
    }

    let mut result: Vec<ReminderGroup> = groups
        .into_iter()
        .map(|(case_name, mut items)| {
            items.sort_by(|a, b| a.date.cmp(&b.date).then(a.kind.cmp(&b.kind)));
            ReminderGroup { case_name, items }
        })
        .collect();
    result.sort_by(|a, b| a.case_name.cmp(&b.case_name));
    Ok(result)
}

async fn last_sent_date(pool: &SqlitePool) -> Result<Option<NaiveDate>, String> {
    let value: Option<String> = sqlx::query_scalar(
        "SELECT sent_date FROM feishu_reminder_runs ORDER BY sent_date DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查询飞书提醒发送记录失败: {e}"))?;
    Ok(value.and_then(|date| NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()))
}

async fn mark_sent(pool: &SqlitePool, date: NaiveDate, item_count: usize) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO feishu_reminder_runs (sent_date, item_count) VALUES (?, ?) \
         ON CONFLICT(sent_date) DO UPDATE SET sent_at = datetime('now'), item_count = excluded.item_count",
    )
    .bind(date.format("%Y-%m-%d").to_string())
    .bind(item_count as i64)
    .execute(pool)
    .await
    .map_err(|e| format!("保存飞书提醒发送记录失败: {e}"))?;
    Ok(())
}

pub async fn send_webhook(url: &str, content: &str) -> Result<(), String> {
    let url = url.trim();
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err("飞书 Webhook URL 必须以 http:// 或 https:// 开头".into());
    }
    let response = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({
            "msg_type": "text",
            "content": { "text": content }
        }))
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "飞书 Webhook 请求超时".to_string()
            } else if error.is_connect() {
                "无法连接飞书 Webhook".to_string()
            } else {
                "飞书 Webhook 请求失败".to_string()
            }
        })?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    parse_feishu_response(status, &body)
}

pub async fn test_webhook(url: &str) -> Result<(), String> {
    let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let message = build_message(&[ReminderGroup {
        case_name: "CaseBoard 测试".into(),
        items: vec![ReminderItem {
            date: today,
            kind: "测试提醒".into(),
            content: "这是一条来自 CaseBoard 的测试消息 ✅".into(),
        }],
    }]);
    send_webhook(url, &message).await
}

async fn run_once(pool: &SqlitePool) -> Result<(), String> {
    if !crate::device_sync::is_automation_owner() {
        return Ok(());
    }
    let current = settings::read_settings()?;
    if !current.feishu_reminder_enabled.unwrap_or(false) {
        return Ok(());
    }
    let url = current
        .feishu_webhook_url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .ok_or_else(|| "飞书手机提醒已开启，但未填写 Webhook URL".to_string())?;
    let target = current
        .feishu_reminder_time
        .as_deref()
        .and_then(|value| NaiveTime::parse_from_str(value, "%H:%M").ok())
        .unwrap_or_else(|| NaiveTime::parse_from_str(DEFAULT_TIME, "%H:%M").unwrap());
    let now = Local::now();
    let today = now.date_naive();
    if !should_send(today, now.time(), target, last_sent_date(pool).await?) {
        return Ok(());
    }

    let days = current.feishu_reminder_days.unwrap_or(DEFAULT_DAYS);
    let end = today
        .checked_add_days(chrono::Days::new(days as u64))
        .ok_or_else(|| "飞书提醒日期范围溢出".to_string())?;
    let groups = collect_pending_between(pool, today, end).await?;
    let item_count = groups.iter().map(|group| group.items.len()).sum();
    if !groups.is_empty() {
        send_webhook(url, &build_message(&groups)).await?;
    }
    mark_sent(pool, today, item_count).await
}

pub fn spawn_scheduler(pool: SqlitePool) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(error) = run_once(&pool).await {
                crate::dlog!("[feishu-reminder] {}", error);
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}
