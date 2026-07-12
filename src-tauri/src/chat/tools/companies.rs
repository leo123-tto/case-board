//! 企业 6 个 tool(V0.2 D2-D3.D)。
//!
//! 精简版 — 砍了 14 个被聚合 Top 20 覆盖的细分接口(详 § 5.4)。
//! `enterprise_aggregation_summary` 是核心入口,5 积分一次拿全维度。
//! 接口按官方目录分别为 1 / 5 / 10 积分；聚合 Top 20 不够时再调明细，避免无目的翻页。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{
    opt_str, opt_u32, require_str, save_and_wrap, try_kb_hit, yuandian_key, Tool, ToolContext,
    ToolError, ToolResult,
};
use crate::yuandian::{self, EntityId};

/// 从 args 里拿 EntityId(id 或 tyshxydm 二选一)。
fn entity_from_args(args: &Value) -> Result<EntityId, ToolError> {
    if let Some(id) = opt_str(args, "id") {
        Ok(EntityId::Id(id.to_string()))
    } else if let Some(uscc) = opt_str(args, "tyshxydm") {
        Ok(EntityId::Uscc(uscc.to_string()))
    } else {
        Err(ToolError::InvalidArgs(
            "需要填 id 或 tyshxydm 二选一".into(),
        ))
    }
}

fn entity_cache_key(eid: &EntityId) -> String {
    match eid {
        EntityId::Id(s) => format!("id:{}", s),
        EntityId::Uscc(s) => format!("uscc:{}", s),
    }
}

pub struct EnterpriseSearch;

#[async_trait]
impl Tool for EnterpriseSearch {
    fn name(&self) -> &str {
        "enterprise_search"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/enterprise_search.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "description": "中文企业名(全称/简称/关键字)"}
            },
            "required": ["name"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        // D5-6:底层 enterprise_search 只接 name(top_k 硬编码 10),不再在 schema 暴露 LLM 设不动的 top_k
        let name = require_str(args, "name")?;
        let cache_params = json!({"name": name});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseSearch", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_key(ctx)?;
        // yuandian::enterprise_search 现签名只接 name(top_k 在底层硬编码 10),
        // V0.2 当前先用,后续若需要可扩 Params struct
        let resp = yuandian::enterprise_search(api_key, name).await?;
        Ok(save_and_wrap(
            ctx,
            "rh_enterpriseSearch",
            &cache_params,
            name,
            resp,
        ))
    }
}

pub struct EnterpriseAggregationSummary;

#[async_trait]
impl Tool for EnterpriseAggregationSummary {
    fn name(&self) -> &str {
        "enterprise_aggregation_summary"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/enterprise_aggregation_summary.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "元典企业 ID(优先填)"},
                "tyshxydm": {"type": "string", "description": "统一社会信用代码 18 位"}
            }
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let eid = entity_from_args(args)?;
        let cache_key = entity_cache_key(&eid);
        let cache_params = json!({"entity": cache_key});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseAggregationSummary", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_key(ctx)?;
        let resp = yuandian::enterprise_aggregation_summary(api_key, &eid).await?;
        // 聚合 5 积分
        Ok(save_and_wrap(
            ctx,
            "rh_enterpriseAggregationSummary",
            &cache_params,
            &cache_key,
            resp,
        ))
    }
}

pub struct EnterpriseBaseInfo;

#[async_trait]
impl Tool for EnterpriseBaseInfo {
    fn name(&self) -> &str {
        "enterprise_base_info"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/enterprise_base_info.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "tyshxydm": {"type": "string", "description": "USCC 18 位"}
            }
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let eid = entity_from_args(args)?;
        let cache_key = entity_cache_key(&eid);
        let cache_params = json!({"entity": cache_key});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseBaseInfo", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_key(ctx)?;
        let resp = yuandian::enterprise_base_info(api_key, &eid).await?;
        Ok(save_and_wrap(
            ctx,
            "rh_enterpriseBaseInfo",
            &cache_params,
            &cache_key,
            resp,
        ))
    }
}

pub struct EnterpriseChangeInfo;

#[async_trait]
impl Tool for EnterpriseChangeInfo {
    fn name(&self) -> &str {
        "enterprise_change_info"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/enterprise_change_info.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "tyshxydm": {"type": "string"},
                "page": {"type": "integer", "description": "默认 1,每页 20 条"}
            }
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let eid = entity_from_args(args)?;
        let page = opt_u32(args, "page").unwrap_or(1);
        let cache_key = entity_cache_key(&eid);
        let cache_params = json!({"entity": cache_key, "page": page});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseChangeInfo", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_key(ctx)?;
        let resp = yuandian::enterprise_change_info(api_key, &eid, page).await?;
        Ok(save_and_wrap(
            ctx,
            "rh_enterpriseChangeInfo",
            &cache_params,
            &cache_key,
            resp,
        ))
    }
}

pub struct EnterpriseWritList;

#[async_trait]
impl Tool for EnterpriseWritList {
    fn name(&self) -> &str {
        "enterprise_writ_list"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/enterprise_writ_list.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "tyshxydm": {"type": "string"},
                "page": {"type": "integer", "description": "默认 1,每页 20 条"}
            }
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let eid = entity_from_args(args)?;
        let page = opt_u32(args, "page").unwrap_or(1);
        let cache_key = entity_cache_key(&eid);
        let cache_params = json!({"entity": cache_key, "page": page});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseWritList", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_key(ctx)?;
        let resp = yuandian::enterprise_writ_list(api_key, &eid, page).await?;
        Ok(save_and_wrap(
            ctx,
            "rh_enterpriseWritList",
            &cache_params,
            &cache_key,
            resp,
        ))
    }
}

pub struct EnterpriseAnnualReport;

#[async_trait]
impl Tool for EnterpriseAnnualReport {
    fn name(&self) -> &str {
        "enterprise_annual_report"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/enterprise_annual_report.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": {"type": "string"},
                "tyshxydm": {"type": "string"},
                "year": {"type": "integer", "description": "自然年,如 2024"}
            },
            "required": ["year"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let eid = entity_from_args(args)?;
        let year =
            opt_u32(args, "year").ok_or_else(|| ToolError::InvalidArgs("year 必填".into()))?;
        let cache_key = entity_cache_key(&eid);
        let cache_params = json!({"entity": cache_key, "year": year});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseAnnualReport", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_key(ctx)?;
        let resp = yuandian::enterprise_annual_report(api_key, &eid, year).await?;
        Ok(save_and_wrap(
            ctx,
            "rh_enterpriseAnnualReport",
            &cache_params,
            &cache_key,
            resp,
        ))
    }
}
