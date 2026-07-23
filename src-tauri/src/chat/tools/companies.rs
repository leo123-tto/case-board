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

fn entity_lookup_term(eid: &EntityId) -> &str {
    match eid {
        EntityId::Id(value) | EntityId::Uscc(value) => value,
    }
}

fn entity_local_query(eid: &EntityId, facet: Option<&str>) -> String {
    match facet.map(str::trim).filter(|value| !value.is_empty()) {
        Some(facet) => format!("{} {facet}", entity_lookup_term(eid)),
        None => entity_lookup_term(eid).to_string(),
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
        super::yuandian_schema::enterprise_search()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let name = require_str(args, "name")?;
        let top_k = opt_u32(args, "top_k").unwrap_or(10).clamp(1, 50);
        let cache_params = json!({"name": name, "top_k": top_k});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseSearch", &cache_params) {
            return Ok(r);
        }
        if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
            crate::chat::retrieval_policy::local_first_gate(
                ctx,
                crate::local_kb::retrieval::RetrievalDomain::Enterprise,
                name,
            )
            .await?
        {
            return Ok(result);
        }
        let api_key = yuandian_key(ctx)?;
        let resp = yuandian::enterprise_search_with_limit(api_key, name, top_k).await?;
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
        super::yuandian_schema::enterprise_entity()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let eid = entity_from_args(args)?;
        let cache_key = entity_cache_key(&eid);
        let cache_params = json!({"entity": cache_key});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseAggregationSummary", &cache_params) {
            return Ok(r);
        }
        if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
            crate::chat::retrieval_policy::local_first_gate(
                ctx,
                crate::local_kb::retrieval::RetrievalDomain::Enterprise,
                &entity_local_query(&eid, None),
            )
            .await?
        {
            return Ok(result);
        }
        let api_key = yuandian_key(ctx)?;
        let resp = yuandian::enterprise_aggregation_summary(api_key, &eid).await?;
        // 聚合 10 积分
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
        super::yuandian_schema::enterprise_entity()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let eid = entity_from_args(args)?;
        let cache_key = entity_cache_key(&eid);
        let cache_params = json!({"entity": cache_key});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseBaseInfo", &cache_params) {
            return Ok(r);
        }
        if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
            crate::chat::retrieval_policy::local_first_gate(
                ctx,
                crate::local_kb::retrieval::RetrievalDomain::Enterprise,
                &entity_local_query(&eid, Some("基本信息")),
            )
            .await?
        {
            return Ok(result);
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
        super::yuandian_schema::enterprise_paged()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let eid = entity_from_args(args)?;
        let page = opt_u32(args, "page").unwrap_or(1);
        let cache_key = entity_cache_key(&eid);
        let cache_params = json!({"entity": cache_key, "page": page});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseChangeInfo", &cache_params) {
            return Ok(r);
        }
        if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
            crate::chat::retrieval_policy::local_first_gate(
                ctx,
                crate::local_kb::retrieval::RetrievalDomain::Enterprise,
                &entity_local_query(&eid, Some("变更")),
            )
            .await?
        {
            return Ok(result);
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
        super::yuandian_schema::enterprise_paged()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let eid = entity_from_args(args)?;
        let page = opt_u32(args, "page").unwrap_or(1);
        let cache_key = entity_cache_key(&eid);
        let cache_params = json!({"entity": cache_key, "page": page});
        if let Some(r) = try_kb_hit(ctx, "rh_enterpriseWritList", &cache_params) {
            return Ok(r);
        }
        if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
            crate::chat::retrieval_policy::local_first_gate(
                ctx,
                crate::local_kb::retrieval::RetrievalDomain::Enterprise,
                &entity_local_query(&eid, Some("裁判文书")),
            )
            .await?
        {
            return Ok(result);
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
        super::yuandian_schema::enterprise_annual_report()
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
        if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
            crate::chat::retrieval_policy::local_first_gate(
                ctx,
                crate::local_kb::retrieval::RetrievalDomain::Enterprise,
                &entity_local_query(&eid, Some(&format!("{year} 年报"))),
            )
            .await?
        {
            return Ok(result);
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
