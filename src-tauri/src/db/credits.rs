//! V0.2 D5.A · 月度元典积分账(`yuandian_credits_monthly` 表)的 CRUD。
//!
//! 表结构在 migration 0018 已建。设计目的:
//!   - **熔断**:AI 助手自动调元典前,Hook 查本月剩余配额,超限拒绝
//!   - **观察**:Settings → 元典卡片展示「本月已用 X / 上限 Y / KB 帮你省了 N 次」
//!   - **轻量**:每年只 12 行(每个月一行),O(1) 查询,不扫 chat_messages 历史
//!
//! 跟其他模块边界:
//!   - hooks::CreditsGuardHook 调本模块 `get_monthly_used` 决定 Continue / Deny
//!   - parallel::run_parallel_subtasks 后,after_tool_call hook 调本模块 `record_yuandian_call` / `record_kb_hit`
//!   - 单次 chat_task 的 credits 还会写到 chat_tasks.yuandian_credits_used 列,month 表是聚合视图
//!
//! 涉坑:CLAUDE.md 坑 #12 — `verified_at` 是 key 验证标记,跟 credits 无关,**不要混用**。
//! 元典 key 未验证时,tool 自然会调用失败,不应该被记账(只在 result.success && !kb_hit 时记)。

use chrono::{Datelike, Local};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

/// 月度积分账单行。一年 12 行(year_month=YYYY-MM)。
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, Default)]
pub struct MonthlyCredits {
    pub year_month: String,
    pub credits_used: i64,
    pub api_calls: i64,
    pub kb_hits: i64,
    pub updated_at: String,
}

/// `YYYY-MM`(本地时区当月)。给 hooks / commands 用作 PK。
pub fn current_year_month() -> String {
    let now = Local::now();
    format!("{:04}-{:02}", now.year(), now.month())
}

/// 读某月当前累计;不存在返回 default(全 0)。
pub async fn get_monthly_stats(
    pool: &SqlitePool,
    year_month: &str,
) -> Result<MonthlyCredits, sqlx::Error> {
    let row: Option<MonthlyCredits> =
        sqlx::query_as("SELECT * FROM yuandian_credits_monthly WHERE year_month = ?")
            .bind(year_month)
            .fetch_optional(pool)
            .await?;
    Ok(row.unwrap_or_else(|| MonthlyCredits {
        year_month: year_month.to_string(),
        ..Default::default()
    }))
}

/// 积分账总览:当月 + 上月(当前月之前最近一个有记录的月) + 全部累计。
/// 给 Settings 卡片在「当月为 0(每月 1 号归零/跨月)」时补显示历史,避免误以为数据丢了。
#[derive(Debug, Clone, Serialize, Default)]
pub struct CreditsOverview {
    pub current: MonthlyCredits,
    pub prev_month: Option<MonthlyCredits>,
    pub total_credits: i64,
    pub total_api_calls: i64,
    pub total_kb_hits: i64,
}

/// 读积分账总览(当月 + 上月 + 累计)。
pub async fn get_overview(
    pool: &SqlitePool,
    year_month: &str,
) -> Result<CreditsOverview, sqlx::Error> {
    let current = get_monthly_stats(pool, year_month).await?;
    let prev_month: Option<MonthlyCredits> = sqlx::query_as(
        "SELECT * FROM yuandian_credits_monthly \
         WHERE year_month < ? ORDER BY year_month DESC LIMIT 1",
    )
    .bind(year_month)
    .fetch_optional(pool)
    .await?;
    let (tc, ta, tk): (Option<i64>, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT SUM(credits_used), SUM(api_calls), SUM(kb_hits) FROM yuandian_credits_monthly",
    )
    .fetch_one(pool)
    .await?;
    Ok(CreditsOverview {
        current,
        prev_month,
        total_credits: tc.unwrap_or(0),
        total_api_calls: ta.unwrap_or(0),
        total_kb_hits: tk.unwrap_or(0),
    })
}

/// 给 Hook 用的便捷版:返回本月 `credits_used`。
pub async fn get_monthly_used(pool: &SqlitePool, year_month: &str) -> i64 {
    match get_monthly_stats(pool, year_month).await {
        Ok(m) => m.credits_used,
        Err(_) => 0, // DB 读失败不阻断 chat,保守认为「还没用过」让 chat 继续(降级)
    }
}

/// 算本月剩余配额。`limit` 是 settings.yuandian_monthly_credit_limit;`None` = 不限制。
/// 返回:正数 = 剩余,0 = 用尽,负数 = 已超限(理论上不会,record 时会算)。
/// `None` 时返回 `i64::MAX`(表达无限)。
pub async fn get_monthly_remaining(pool: &SqlitePool, year_month: &str, limit: Option<u32>) -> i64 {
    let used = get_monthly_used(pool, year_month).await;
    match limit {
        None => i64::MAX,
        Some(l) => (l as i64) - used,
    }
}

/// 在线元典调用成功后,记一笔。
/// `credits` 是工具结果回传的本次真实目录价；同一工具可能按路由走不同接口，不能只按工具名猜。
pub async fn record_yuandian_call(
    pool: &SqlitePool,
    year_month: &str,
    credits: u32,
) -> Result<(), sqlx::Error> {
    let now = Local::now().to_rfc3339();
    // SQLite UPSERT (3.24+);0017/0018 都用了类似语法,语法已经过验证。
    sqlx::query(
        "INSERT INTO yuandian_credits_monthly \
         (year_month, credits_used, api_calls, kb_hits, updated_at) \
         VALUES (?, ?, 1, 0, ?) \
         ON CONFLICT(year_month) DO UPDATE SET \
           credits_used = credits_used + excluded.credits_used, \
           api_calls    = api_calls + 1, \
           updated_at   = excluded.updated_at",
    )
    .bind(year_month)
    .bind(credits as i64)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// 本地 KB 命中(替元典省了 1 次外查),记一笔。**不增加 credits_used**。
pub async fn record_kb_hit(pool: &SqlitePool, year_month: &str) -> Result<(), sqlx::Error> {
    let now = Local::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO yuandian_credits_monthly \
         (year_month, credits_used, api_calls, kb_hits, updated_at) \
         VALUES (?, 0, 0, 1, ?) \
         ON CONFLICT(year_month) DO UPDATE SET \
           kb_hits    = kb_hits + 1, \
           updated_at = excluded.updated_at",
    )
    .bind(year_month)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

/// 给 tool name 估算积分消耗(详 § 5.4 + § 5.5)。
/// 不挂工具元数据表:简单 match 维护,新工具加进来再扩。
pub fn estimate_credits_for(tool: &str) -> u32 {
    // 按元典官方真实计费(docs/元典接口-积分计费明细.md,2026-06-01 校准):
    // 工具名 → 实际调的元典接口 → 真实积分。旧版统一估 1-5,严重低估(语义/案例检索实为 10、幻觉校验 50)。
    match tool {
        // 50 积分:法律幻觉校验(最贵)
        "verify_legal_citations" => 50,
        // 10 积分:语义检索 / 法规·案例关键词检索 / 企业详情类
        "law_vector_search"                 // 法律法规语义检索
        | "search_regulations"              // 法规关键词检索
        | "case_vector_search"              // 案例语义检索
        | "search_cases_normal"             // 普通案例关键词检索
        | "search_cases_authority"          // 权威案例关键词检索
        | "enterprise_aggregation_summary"  // 企业聚合总览
        | "enterprise_base_info"            // 企业基本信息
        | "enterprise_writ_list"            // 企业涉诉文书列表
        | "enterprise_annual_report"
        | "search_laws" => 10, // 法条关键词检索 2026-07 已从旧表 1 调整为 10
        // 5 积分:法规详情 / 案例详情 / 企业变更记录
        "get_regulation_detail"             // 法规详情
        | "get_law_article"                 // 默认整部法规详情入库；单条降级实际 1 分，以 ToolResult 为准
        | "get_case_detail"                 // 案例详情
        | "enterprise_change_info" => 5,    // 企业变更记录列表
        // 1 积分:企业模糊检索
        "enterprise_search" => 1,           // 企业检索
        // 本地工具:0 积分(不应该走 record_yuandian_call,这里兜底)
        _ => 0,
    }
}

/// 按**元典接口名(query_type)**返回真实积分。`save_and_wrap` 用它给 `ToolResult` 标注
/// 单次消耗(trace 显示 + 反馈 MD 会话累计),跟 `estimate_credits_for`(按工具名,月账用)
/// 同源于官方计费表(docs/元典接口-积分计费明细.md)。两者一致性由单测 `credit_tables_agree` 保。
pub fn credits_for_query_type(query_type: &str) -> u32 {
    match query_type {
        // 10 积分:语义检索 / 法规关键词 / 案例关键词 / 企业详情聚合类
        "law_vector_search"
        | "rh_fg_search"
        | "rh_ptal_search"
        | "rh_qwal_search"
        | "case_vector_search"
        | "rh_enterpriseAggregationSummary"
        | "rh_enterpriseBaseInfo"
        | "rh_enterpriseWritList"
        | "rh_enterpriseAnnualReport" => 10,
        // 5 积分:法规详情 / 案例详情 / 企业变更
        "rh_fg_detail" | "rh_case_details" | "rh_enterpriseChangeInfo" => 5,
        // 1 积分:法条详情 / 企业模糊检索；法条关键词现为 10
        "rh_ft_detail" | "rh_enterpriseSearch" => 1,
        "rh_ft_search" => 10,
        // 幻觉校验(若走 save_and_wrap)
        "hall_detect" => 50,
        _ => 0,
    }
}

/// D2-1:按"已落盘的元典原始文件名"估算该次调用积分(执行模块 orchestrator / deep_dive 记账用)。
///
/// 约定(见 yuandian::orchestrator::file_name):`{主体}_{端点}.json` = 一次**计费 API 调用**
///(`*_aggregation.json` = 聚合摘要 10 积分,明细按 5/10 区分);`.md` = 自然人占位,无 API 调用 → 0。
/// 这样 orchestrator/deep_dive 直接拿返回的 raw_files 列表逐个记账,无需把 pool 穿进底层查询函数。
pub fn credits_for_raw_file(filename: &str) -> u32 {
    if !filename.ends_with(".json") {
        return 0; // 自然人占位 .md 等,非 API 调用
    }
    let stem = filename.trim_end_matches(".json");
    // 按端点后缀匹配真实积分(端点名见 yuandian::orchestrator;文件名 {主体}_{端点}.json,
    // 主体名可能含下划线 → 从右 ends_with/contains 匹配端点)。校准见 docs/元典接口-积分计费明细.md。
    // 10 分:聚合总览 / 基本信息 / 涉诉文书 / 年报
    if stem.ends_with("_aggregation")
        || stem.ends_with("_base_info")
        || stem.ends_with("_writ_list")
        || stem.contains("_annual_report")
    {
        return 10;
    }
    // 1 分:企业模糊检索
    if stem.ends_with("_search") {
        return 1;
    }
    // 其余各明细列表(被执行/失信/股权冻结/出质/担保/公告/处罚/异常/违法/欠税/变更/对外投资)= 5
    5
}
