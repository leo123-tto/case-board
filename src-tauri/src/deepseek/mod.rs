//! DeepSeek 余额查询 + 今日消费计算(2026-05-24 e)。
//!
//! 参考内部 DeepSeek 用量监控服务的余额计算方式:
//! - DeepSeek 只提供 `GET https://api.deepseek.com/user/balance`(当前余额),无"今日消费"端点
//! - 我们靠"昨日快照 vs 今日 fetch 余额 delta"算今日消费
//! - 每天保存一次快照(`deepseek_balance_snapshots` 表)
//!
//! API key 从 `settings.cloud_llm_api_key` 读(已经存在用户设置里)。

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use thiserror::Error;

const BALANCE_ENDPOINT: &str = "https://api.deepseek.com/user/balance";
const TIMEOUT_SECS: u64 = 15;

/// DeepSeek 余额信息(给前端的 IPC 返回值)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepSeekBalance {
    /// 当前总余额(元)— DeepSeek API 返回的 `total_balance`
    pub total_balance: f64,
    /// 赠送余额(元)
    pub granted_balance: f64,
    /// 充值余额(元)
    pub topped_up_balance: f64,
    /// 今日消费(元)= 今日 0 点的快照 - 当前 fetch 的余额。
    /// 没有昨日快照时返回 None(首次启动情况)
    pub today_consumed: Option<f64>,
    /// 最近一次 fetch 时间(ISO 8601)
    pub fetched_at: String,
}

#[derive(Debug, Error)]
pub enum DeepSeekError {
    #[error("DeepSeek API key 未配置")]
    NoApiKey,
    #[error("网络请求失败:{0}")]
    Network(String),
    #[error("DeepSeek 返回非 200 状态:{0} - {1}")]
    HttpStatus(u16, String),
    #[error("响应格式异常:{0}")]
    ResponseFormat(String),
    #[error("数据库错误:{0}")]
    Db(#[from] sqlx::Error),
}

impl serde::Serialize for DeepSeekError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// DeepSeek API 返回的 balance_infos 数组的单个元素。
#[derive(Deserialize)]
struct BalanceInfo {
    currency: String,
    total_balance: String,
    #[serde(default)]
    granted_balance: String,
    #[serde(default)]
    topped_up_balance: String,
}

#[derive(Deserialize)]
struct BalanceResponse {
    balance_infos: Vec<BalanceInfo>,
}

/// 拉 DeepSeek 当前余额 + 计算今日消费 + 写当天快照。
///
/// 流程:
///   1. 读 settings.cloud_llm_api_key
///   2. fetch GET /user/balance + Bearer Authorization
///   3. 找 currency=CNY 那条
///   4. 算今日消费:今天 0 点之前最近的一条快照 - 当前 totalBalance
///   5. UPSERT 今天的快照(每天保留一条,刷新 fetched_at + total_balance)
pub async fn fetch_balance_and_persist(
    pool: &SqlitePool,
    api_key: &str,
) -> Result<DeepSeekBalance, DeepSeekError> {
    if api_key.trim().is_empty() {
        return Err(DeepSeekError::NoApiKey);
    }

    // 1. fetch
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .build()
        .map_err(|e| DeepSeekError::Network(e.to_string()))?;

    let resp = client
        .get(BALANCE_ENDPOINT)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .header("Cache-Control", "no-cache")
        .send()
        .await
        .map_err(|e| DeepSeekError::Network(e.to_string()))?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(DeepSeekError::HttpStatus(status.as_u16(), body));
    }

    let body: BalanceResponse = resp
        .json()
        .await
        .map_err(|e| DeepSeekError::ResponseFormat(e.to_string()))?;

    // 2. 找 CNY
    let cny = body
        .balance_infos
        .into_iter()
        .find(|b| b.currency == "CNY")
        .ok_or_else(|| DeepSeekError::ResponseFormat("balance_infos 里没有 CNY 条目".into()))?;

    let total: f64 = cny
        .total_balance
        .parse()
        .map_err(|e: std::num::ParseFloatError| DeepSeekError::ResponseFormat(e.to_string()))?;
    let granted: f64 = cny.granted_balance.parse().unwrap_or(0.0);
    let topped_up: f64 = cny.topped_up_balance.parse().unwrap_or(0.0);

    // 3. 算今日消费(2026-05-24 i 修)
    //
    // 旧逻辑:用「昨天 snapshot - 今天 fetch」,但今天才开始用 App 时 DB 没"昨天",一直 None
    // 新逻辑:用「今天 snapshot 的 day_start_balance - 当前 total」
    //   - 今天第一次 fetch:DB 里没 today 行,INSERT 时 day_start_balance = current total → consumed = 0
    //   - 今天第 N 次 fetch:UPSERT **不覆盖** day_start_balance → consumed = day_start - current (递增)
    let today = chrono::Local::now().date_naive();
    let today_str = today.format("%Y-%m-%d").to_string();
    let now_iso = chrono::Utc::now().to_rfc3339();

    // 先尝试 INSERT(今天还没记过 → 当前 total 同时存入 total_balance + day_start_balance)。
    // 已有今天的话 ON CONFLICT 只更新 total_balance / fetched_at,保留 day_start_balance。
    sqlx::query(
        "INSERT INTO deepseek_balance_snapshots \
            (date, total_balance, granted_balance, topped_up_balance, fetched_at, day_start_balance) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(date) DO UPDATE SET \
            total_balance = excluded.total_balance, \
            granted_balance = excluded.granted_balance, \
            topped_up_balance = excluded.topped_up_balance, \
            fetched_at = excluded.fetched_at",
    )
    .bind(&today_str)
    .bind(total)
    .bind(granted)
    .bind(topped_up)
    .bind(&now_iso)
    .bind(total) // 第一次插入时 day_start_balance = 当前 total
    .execute(pool)
    .await?;

    // 读 day_start_balance(刚 INSERT/UPSERT 完肯定有值)
    let day_start: Option<f64> = sqlx::query_scalar(
        "SELECT day_start_balance FROM deepseek_balance_snapshots WHERE date = ?",
    )
    .bind(&today_str)
    .fetch_optional(pool)
    .await?;

    let today_consumed = day_start.map(|ds| (ds - total).max(0.0));

    Ok(DeepSeekBalance {
        total_balance: total,
        granted_balance: granted,
        topped_up_balance: topped_up,
        today_consumed,
        fetched_at: now_iso,
    })
}

/// 读最近一条已缓存的余额(不发请求,用于启动时立刻显示一个值)。
pub async fn cached_balance(pool: &SqlitePool) -> Result<Option<DeepSeekBalance>, sqlx::Error> {
    let today_str = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    // 2026-05-24 i:也读 day_start_balance 算今日消费
    type Row = (String, f64, f64, f64, String, Option<f64>);
    let row: Option<Row> = sqlx::query_as(
        "SELECT date, total_balance, granted_balance, topped_up_balance, fetched_at, day_start_balance \
         FROM deepseek_balance_snapshots ORDER BY date DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;

    let Some((date, total, granted, topped_up, fetched_at, day_start)) = row else {
        return Ok(None);
    };

    // 注意:cached_balance 可能是昨天的(用户开机时还没刷新),today_consumed 只在 date == today 时有意义
    let today_consumed = if date == today_str {
        day_start.map(|ds| (ds - total).max(0.0))
    } else {
        None
    };

    Ok(Some(DeepSeekBalance {
        total_balance: total,
        granted_balance: granted,
        topped_up_balance: topped_up,
        today_consumed,
        fetched_at,
    }))
}
