//! 元典 MCP 账户余额查询与本机积分账对账。
//!
//! 元典公开业务 API 目录只列法律、案例、企业等计费端点；`yuandian-law` MCP
//! server 另行暴露免费的 `yuandian_get_user_balance`。本模块通过现有 MCP client
//! 读取 `structuredContent.data.data.pointBalance/countBalance`，并把相邻余额快照与
//! CaseBoard 本机累计积分增量比较。

use std::collections::BTreeMap;

use chrono::Local;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};

use crate::chat::mcp_bridge::{McpClient, McpServerConfig, McpTransport};

const MCP_LAW_URL: &str = "https://open.chineselaw.com/mcp/law/stream";
const BALANCE_TOOL: &str = "yuandian_get_user_balance";

#[derive(Debug, Clone, FromRow)]
struct BalanceSnapshot {
    id: i64,
    key_fingerprint: String,
    point_balance: i64,
    count_balance: i64,
    local_credits_total: i64,
    local_api_calls_total: i64,
    fetched_at: String,
}

/// 给设置页展示的余额与对账视图。`difference` = 官方余额减少 - 本机记账；
/// 正数通常表示还有其他客户端/API 调用，负数通常表示充值、返还、计价变化或刷新时点差。
#[derive(Debug, Clone, Serialize)]
pub struct YuandianBalanceView {
    pub point_balance: i64,
    pub count_balance: i64,
    pub fetched_at: String,
    pub cached: bool,
    pub previous_point_balance: Option<i64>,
    pub previous_fetched_at: Option<String>,
    pub official_spent_since_previous: Option<i64>,
    pub local_recorded_since_previous: Option<i64>,
    pub local_api_calls_since_previous: Option<i64>,
    pub difference: Option<i64>,
    pub balance_increased_since_previous: Option<i64>,
    pub comparison_status: String,
    pub refresh_error: Option<String>,
}

impl YuandianBalanceView {
    /// 网络刷新失败但本地有快照时仍展示旧值，并把错误交给 UI 说明。
    pub fn with_refresh_error(mut self, error: String) -> Self {
        self.cached = true;
        self.refresh_error = Some(error);
        self
    }
}

fn key_fingerprint(api_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(api_key.as_bytes());
    let hex = format!("{:x}", hasher.finalize());
    hex[..16].to_string()
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

/// 优先解析 MCP 的 structuredContent；兼容服务端只返回 content.text 包装 JSON 的旧形态。
fn parse_balance_result(result: &Value) -> Result<(i64, i64), String> {
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err("元典 MCP 返回余额工具错误".into());
    }

    let structured = result
        .pointer("/structuredContent/data/data")
        .or_else(|| result.pointer("/structuredContent/data"));
    if let Some(data) = structured {
        if let Some(points) = data.get("pointBalance").and_then(value_as_i64) {
            let count = data.get("countBalance").and_then(value_as_i64).unwrap_or(0);
            return Ok((points, count));
        }
    }

    let text = result
        .pointer("/content/0/text")
        .and_then(Value::as_str)
        .ok_or("元典 MCP 余额响应缺少 structuredContent")?;
    let wrapper: Value =
        serde_json::from_str(text).map_err(|_| "元典 MCP 余额展示文本不是有效 JSON".to_string())?;
    let data = wrapper
        .pointer("/dataPreview/data")
        .or_else(|| wrapper.pointer("/data/data"))
        .ok_or("元典 MCP 余额响应缺少 data")?;
    let points = data
        .get("pointBalance")
        .and_then(value_as_i64)
        .ok_or("元典 MCP 余额响应缺少 pointBalance")?;
    let count = data.get("countBalance").and_then(value_as_i64).unwrap_or(0);
    Ok((points, count))
}

async fn fetch_mcp_balance(api_key: &str) -> Result<(i64, i64), String> {
    let mut headers = BTreeMap::new();
    headers.insert("Authorization".into(), format!("Bearer {api_key}"));
    let config = McpServerConfig {
        name: "yuandian-law".into(),
        transport: McpTransport::Http {
            url: MCP_LAW_URL.into(),
            headers,
        },
        enabled: true,
    };
    let client = McpClient::connect(&config).await?;
    let result = client.call_tool_value(BALANCE_TOOL, &json!({})).await?;
    parse_balance_result(&result)
}

/// 免费验证元典 key，并返回当前积分/次数余额。替代旧版用 1 分企业搜索 + 50 分
/// hall_detect 做探针的昂贵做法。
pub async fn verify_api_key(api_key: &str) -> Result<(i64, i64), String> {
    fetch_mcp_balance(api_key).await
}

async fn local_totals(pool: &SqlitePool) -> Result<(i64, i64), sqlx::Error> {
    let (credits, calls): (i64, i64) = sqlx::query_as(
        "SELECT COALESCE(SUM(credits_used), 0), COALESCE(SUM(api_calls), 0) \
         FROM yuandian_credits_monthly",
    )
    .fetch_one(pool)
    .await?;
    Ok((credits, calls))
}

async fn latest_snapshot(pool: &SqlitePool) -> Result<Option<BalanceSnapshot>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, key_fingerprint, point_balance, count_balance, local_credits_total, \
                local_api_calls_total, fetched_at \
         FROM yuandian_balance_snapshots ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
}

async fn snapshot_before(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<BalanceSnapshot>, sqlx::Error> {
    sqlx::query_as(
        "SELECT id, key_fingerprint, point_balance, count_balance, local_credits_total, \
                local_api_calls_total, fetched_at \
         FROM yuandian_balance_snapshots WHERE id < ? ORDER BY id DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

fn to_view(
    current: BalanceSnapshot,
    previous: Option<BalanceSnapshot>,
    cached: bool,
) -> YuandianBalanceView {
    let comparable = previous.filter(|p| p.key_fingerprint == current.key_fingerprint);
    let mut view = YuandianBalanceView {
        point_balance: current.point_balance,
        count_balance: current.count_balance,
        fetched_at: current.fetched_at,
        cached,
        previous_point_balance: comparable.as_ref().map(|p| p.point_balance),
        previous_fetched_at: comparable.as_ref().map(|p| p.fetched_at.clone()),
        official_spent_since_previous: None,
        local_recorded_since_previous: None,
        local_api_calls_since_previous: None,
        difference: None,
        balance_increased_since_previous: None,
        comparison_status: "baseline".into(),
        refresh_error: None,
    };

    let Some(previous) = comparable else {
        return view;
    };
    let balance_delta = previous.point_balance - current.point_balance;
    if balance_delta < 0 {
        view.balance_increased_since_previous = Some(-balance_delta);
        view.comparison_status = "recharged".into();
        return view;
    }

    let local_delta = current.local_credits_total - previous.local_credits_total;
    if local_delta < 0 {
        view.official_spent_since_previous = Some(balance_delta);
        view.comparison_status = "local_reset".into();
        return view;
    }
    view.official_spent_since_previous = Some(balance_delta);
    view.local_recorded_since_previous = Some(local_delta);
    view.local_api_calls_since_previous =
        Some(current.local_api_calls_total - previous.local_api_calls_total)
            .filter(|delta| *delta >= 0);
    view.difference = Some(balance_delta - local_delta);
    view.comparison_status = if balance_delta == local_delta {
        "matched".into()
    } else {
        "difference".into()
    };
    view
}

/// 通过免费 MCP 工具刷新余额、保存快照并返回相邻区间对账结果。
pub async fn fetch_and_persist(
    pool: &SqlitePool,
    api_key: &str,
) -> Result<YuandianBalanceView, String> {
    let previous = latest_snapshot(pool).await.map_err(|e| e.to_string())?;
    let (point_balance, count_balance) = fetch_mcp_balance(api_key).await?;
    let (local_credits_total, local_api_calls_total) =
        local_totals(pool).await.map_err(|e| e.to_string())?;
    let fingerprint = key_fingerprint(api_key);
    let fetched_at = Local::now().to_rfc3339();
    let result = sqlx::query(
        "INSERT INTO yuandian_balance_snapshots \
         (key_fingerprint, point_balance, count_balance, local_credits_total, \
          local_api_calls_total, fetched_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&fingerprint)
    .bind(point_balance)
    .bind(count_balance)
    .bind(local_credits_total)
    .bind(local_api_calls_total)
    .bind(&fetched_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    let current = BalanceSnapshot {
        id: result.last_insert_rowid(),
        key_fingerprint: fingerprint,
        point_balance,
        count_balance,
        local_credits_total,
        local_api_calls_total,
        fetched_at,
    };
    Ok(to_view(current, previous, false))
}

/// 读取当前 key 最近一次快照，不联网。若用户刚换 key，旧账户余额不会串过来。
pub async fn cached_balance(
    pool: &SqlitePool,
    api_key: &str,
) -> Result<Option<YuandianBalanceView>, String> {
    let fingerprint = key_fingerprint(api_key);
    let current: Option<BalanceSnapshot> = sqlx::query_as(
        "SELECT id, key_fingerprint, point_balance, count_balance, local_credits_total, \
                local_api_calls_total, fetched_at \
         FROM yuandian_balance_snapshots WHERE key_fingerprint = ? ORDER BY id DESC LIMIT 1",
    )
    .bind(fingerprint)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some(current) = current else {
        return Ok(None);
    };
    let previous = snapshot_before(pool, current.id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(to_view(current, previous, true)))
}
