//! 每日待办提醒：企微/飞书 Webhook 推送 + 后台定时调度器。
//!
//! 设计：
//!   1. 后台 tokio task 每 60 秒轮询一次，检查是否到了配置的提醒时间（如 09:00）。
//!   2. 当天已发过则跳过（存在 `once_cell` 全局的 `LAST_SENT_DATE`）。
//!   3. 从三处数据源查询未来 `webhook_remind_days` 天内的事项：
//!      - cases.agg_key_dates JSON  → AI 抽取的关键节点（开庭、举证期等）
//!      - events 表（is_done=0）    → 手动录入的案件事件
//!      - calendar_events 表       → 用户独立日程（不绑案件）
//!   4. 构建 Markdown 消息 → 分别发企微/飞书 webhook。
//!
//! 消息格式（企微 Markdown / 飞书文本）按案件分组展示。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicI32, Ordering};

use sqlx::SqlitePool;

use crate::settings;

/// 上次发送的日期，格式 YYYYMMDD。0 表示从未发送。
static LAST_SENT_DATE: AtomicI32 = AtomicI32::new(0);

// ============================================================================
// 公共入口
// ============================================================================

/// 启动后台定时调度器（由 lib.rs setup() 在启动时 spawn）。
/// 每 60 秒检查一次是否该发提醒。
pub async fn start_scheduler(pool: SqlitePool) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;

        let s = match settings::read_settings() {
            Ok(s) => s,
            Err(_) => continue,
        };

        let wecom_enabled = s.webhook_wecom_enabled.unwrap_or(false);
        let feishu_enabled = s.webhook_feishu_enabled.unwrap_or(false);

        if !wecom_enabled && !feishu_enabled {
            continue;
        }

        let daily_time = s.webhook_daily_time.unwrap_or_else(|| "09:00".to_string());
        let now = chrono::Local::now();
        let current_time = now.format("%H:%M").to_string();
        let current_date = now.format("%Y%m%d").to_string();
        let current_date_i32: i32 = current_date.parse().unwrap_or(0);

        if current_time != daily_time {
            continue;
        }

        // 今天已发过 → 跳过
        if LAST_SENT_DATE.load(Ordering::Relaxed) == current_date_i32 {
            continue;
        }

        // 查询待办
        let remind_days = s.webhook_remind_days.unwrap_or(7) as i64;
        let milestones = match get_pending_items(&pool, remind_days).await {
            Ok(m) => m,
            Err(e) => {
                crate::dlog!("[notify] 查询待办失败: {e}");
                continue;
            }
        };

        if milestones.is_empty() {
            LAST_SENT_DATE.store(current_date_i32, Ordering::Relaxed);
            continue;
        }

        // 构建消息内容
        let content = build_markdown_message(&milestones);

        // 发企微
        if wecom_enabled {
            if let Some(ref url) = s.webhook_wecom_url {
                if let Err(e) = send_wecom(url, &content).await {
                    crate::dlog!("[notify] 企业微信 webhook 发送失败: {e}");
                }
            }
        }

        // 发飞书
        if feishu_enabled {
            if let Some(ref url) = s.webhook_feishu_url {
                if let Err(e) = send_feishu(url, &content).await {
                    crate::dlog!("[notify] 飞书 webhook 发送失败: {e}");
                }
            }
        }

        // 标记今天已发
        LAST_SENT_DATE.store(current_date_i32, Ordering::Relaxed);
    }
}

// ============================================================================
// Webhook 发送（公有，lib.rs 命令可调）
// ============================================================================

/// 测试 webhook（企微或飞书），发一条简单的测试消息。
pub async fn test_webhook(provider: &str, url: &str) -> Result<(), String> {
    let test_msg = build_markdown_message(&[GroupedMilestone {
        case_name: "CaseBoard 测试".to_string(),
        items: vec![GroupedItem {
            date: chrono::Local::now().format("%Y-%m-%d").to_string(),
            kind: "测试提醒".to_string(),
            content: "这是一条来自 CaseBoard 的测试消息 ✅".to_string(),
        }],
    }]);

    match provider {
        "wecom" => send_wecom(url, &test_msg).await,
        "feishu" => send_feishu(url, &test_msg).await,
        _ => Err(format!("不支持的 provider: {}", provider)),
    }
}

// ============================================================================
// Webhook 发送（内部）
// ============================================================================

/// 企业微信机器人：Markdown 消息。
async fn send_wecom(url: &str, content: &str) -> Result<(), String> {
    let body = serde_json::json!({
        "msgtype": "markdown",
        "markdown": {
            "content": content
        }
    });

    let resp = reqwest::Client::new()
        .post(url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    let status = resp.status();
    let resp_body = resp.text().await.unwrap_or_default();

    if status.is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {status}: {resp_body}"))
    }
}

/// 飞书自定义机器人：文本消息。
async fn send_feishu(url: &str, content: &str) -> Result<(), String> {
    let body = serde_json::json!({
        "msg_type": "text",
        "content": {
            "text": content
        }
    });

    let resp = reqwest::Client::new()
        .post(url)
        .json(&body)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    let status = resp.status();
    let resp_body = resp.text().await.unwrap_or_default();

    if status.is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {status}: {resp_body}"))
    }
}

// ============================================================================
// 消息构建
// ============================================================================

#[derive(Debug, Clone)]
struct GroupedItem {
    date: String,
    kind: String,
    content: String,
}

#[derive(Debug, Clone)]
struct GroupedMilestone {
    case_name: String,
    items: Vec<GroupedItem>,
}

/// 构建 Markdown 横幅消息（企微/飞书通用）。
fn build_markdown_message(groups: &[GroupedMilestone]) -> String {
    let mut lines: Vec<String> = Vec::new();

    let total: usize = groups.iter().map(|g| g.items.len()).sum();
    lines.push(format!(
        "📋 **案件待办提醒**\n> {} 个案件 · {} 条待办即将到期",
        groups.len(),
        total
    ));
    lines.push(String::new());

    for g in groups {
        lines.push(format!(
            "**{}**（{} 条）",
            g.case_name,
            g.items.len()
        ));
        for item in &g.items {
            let content = if item.content.is_empty() {
                String::new()
            } else {
                format!(" · {}", item.content)
            };
            lines.push(format!("- {} · {}{}", item.date, item.kind, content));
        }
        lines.push(String::new());
    }

    lines.push(format!(
        "⏰ 共 {} 条待办事项即将到期，请及时处理。",
        total
    ));

    lines.join("\n")
}

// ============================================================================
// 数据查询（⭐ 适配 main 分支：查 agg_key_dates + events + calendar_events）
// ============================================================================

/// 从三处数据源查询未来 `within_days` 天内的事项，按案件分组。
async fn get_pending_items(
    pool: &SqlitePool,
    within_days: i64,
) -> Result<Vec<GroupedMilestone>, String> {
    let mut case_items: BTreeMap<String, Vec<GroupedItem>> = BTreeMap::new();

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let future = chrono::Local::now()
        .checked_add_days(chrono::Days::new(within_days as u64))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "2099-12-31".to_string());

    // ── 1) cases.agg_key_dates JSON → AI 抽取的关键节点 ──
    let ai_rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, name, agg_key_dates FROM cases WHERE agg_key_dates IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查 cases 失败: {e}"))?;

    for (_case_id, case_name, agg_json) in &ai_rows {
        let Some(agg) = agg_json else { continue };
        let Ok(parsed) = serde_json::from_str::<serde_json::Value>(agg) else {
            continue;
        };
        let Some(arr) = parsed.as_array() else { continue };

        for item in arr {
            let date = item
                .get("date")
                .and_then(|v| v.as_str())
                .map(|d| normalize_date(d))
                .unwrap_or_default();

            if date.is_empty() || date < today || date > future {
                continue;
            }

            let note = item
                .get("note")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let kind = item
                .get("event")
                .and_then(|v| v.as_str())
                .unwrap_or("其他提醒")
                .to_string();

            case_items
                .entry(case_name.clone())
                .or_default()
                .push(GroupedItem { date, kind, content: note });
        }
    }

    // ── 2) events 表（is_done=0，手动录入事件）──
    let event_rows: Vec<(String, Option<String>, String, String, String)> = sqlx::query_as(
        "SELECT e.occurred_at, e.event_type, e.title, e.case_id, c.name \
         FROM events e \
         JOIN cases c ON e.case_id = c.id \
         WHERE e.is_done = 0 \
           AND e.occurred_at >= date('now') \
           AND e.occurred_at <= date('now', '+' || ? || ' days') \
         ORDER BY c.name, e.occurred_at",
    )
    .bind(within_days)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查 events 失败: {e}"))?;

    for (date, kind, title, _case_id, case_name) in &event_rows {
        let kind = kind.clone().unwrap_or_else(|| "事件".to_string());
        case_items
            .entry(case_name.clone())
            .or_default()
            .push(GroupedItem {
                date: date.clone(),
                kind,
                content: title.clone(),
            });
    }

    // ── 3) calendar_events 表（用户独立日程，不绑案件）──
    let cal_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT date, title FROM calendar_events \
         WHERE date >= date('now') \
           AND date <= date('now', '+' || ? || ' days') \
         ORDER BY date",
    )
    .bind(within_days)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("查 calendar_events 失败: {e}"))?;

    for (date, title) in &cal_rows {
        case_items
            .entry("📅 个人日程".to_string())
            .or_default()
            .push(GroupedItem {
                date: date.clone(),
                kind: "日程".to_string(),
                content: title.clone(),
            });
    }

    // 按案件排序，构建结果
    let mut out: Vec<GroupedMilestone> = Vec::new();
    for (case_name, mut items) in case_items {
        items.sort_by(|a, b| a.date.cmp(&b.date));
        if !items.is_empty() {
            out.push(GroupedMilestone { case_name, items });
        }
    }
    out.sort_by(|a, b| a.case_name.cmp(&b.case_name));

    Ok(out)
}

/// 格式化 AI 日期：如果只有月份（如 "2024-09"）补充为月末 "2024-09-30"。
fn normalize_date(date: &str) -> String {
    let trimmed = date.trim();
    if trimmed.len() == 10
        && trimmed.chars().nth(4) == Some('-')
        && trimmed.chars().nth(7) == Some('-')
    {
        return trimmed.to_string();
    }
    if trimmed.len() == 7 && trimmed.chars().nth(4) == Some('-') {
        return format!("{}-30", trimmed);
    }
    trimmed.to_string()
}
