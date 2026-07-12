//! 案例 4 个 tool(V0.2 D2-D3.C)。
//!
//! 注意 wire 现状:
//! - `search_ptal` / `search_qwal` 当前 yuandian/mod.rs 签名只接 `(keyword, top_k)`,
//!   高级过滤(court / cause / region / case_no)未启用。description 已标注。
//! - `get_case_detail` 直接调用官方 `rh_case_details` GET 端点，以案号或 id 定位详情。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{
    opt_u32, require_str, save_and_wrap, try_kb_hit, yuandian_key, Tool, ToolContext, ToolError,
    ToolResult,
};
use crate::yuandian;

pub struct SearchCasesNormal;

#[async_trait]
impl Tool for SearchCasesNormal {
    fn name(&self) -> &str {
        "search_cases_normal"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/search_cases_normal.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "qw": {"type": "string", "description": "中文关键词,支持「+」组合,如「合同解除+违约金」"},
                "top_k": {"type": "integer", "description": "默认 20"}
            },
            "required": ["qw"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let qw = require_str(args, "qw")?;
        let top_k = opt_u32(args, "top_k").unwrap_or(20);
        let cache_params = json!({"qw": qw, "top_k": top_k});
        if let Some(r) = try_kb_hit(ctx, "rh_ptal_search", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_key(ctx)?;
        let resp = yuandian::search_ptal(api_key, qw, top_k).await?;
        Ok(save_and_wrap(
            ctx,
            "rh_ptal_search",
            &cache_params,
            qw,
            resp,
        ))
    }
}

pub struct SearchCasesAuthority;

#[async_trait]
impl Tool for SearchCasesAuthority {
    fn name(&self) -> &str {
        "search_cases_authority"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/search_cases_authority.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "qw": {"type": "string", "description": "中文关键词"},
                "top_k": {"type": "integer", "description": "默认 20"}
            },
            "required": ["qw"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let qw = require_str(args, "qw")?;
        let top_k = opt_u32(args, "top_k").unwrap_or(20);
        let cache_params = json!({"qw": qw, "top_k": top_k});
        if let Some(r) = try_kb_hit(ctx, "rh_qwal_search", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_key(ctx)?;
        let resp = yuandian::search_qwal(api_key, qw, top_k).await?;
        Ok(save_and_wrap(
            ctx,
            "rh_qwal_search",
            &cache_params,
            qw,
            resp,
        ))
    }
}

pub struct GetCaseDetail;

#[async_trait]
impl Tool for GetCaseDetail {
    fn name(&self) -> &str {
        "get_case_detail"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/get_case_detail.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "type": {"type": "string", "description": "ptal=普通案例库 / qwal=权威案例库"},
                "case_no": {"type": "string", "description": "完整案号,如「(2021)苏02民终123号」"}
            },
            "required": ["type", "case_no"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let lib = require_str(args, "type")?;
        let case_no = require_str(args, "case_no")?;
        let lib_normalized = match lib {
            "ptal" | "qwal" => lib,
            _ => {
                return Err(ToolError::InvalidArgs(format!(
                    "type 必须是 'ptal' 或 'qwal',收到 '{}'",
                    lib
                )))
            }
        };
        let cache_key = format!("{}-{}", lib_normalized, case_no);
        let cache_params = json!({"type": lib_normalized, "case_no": case_no});
        if let Some(r) = try_kb_hit(ctx, "rh_case_details", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_key(ctx)?;
        // 直接走官方 rh_case_details(5 分)，不再用 10 分关键词检索冒充详情。
        // 取详情是**尽力而为**:某些案号(尤其外地/冷门库)元典会返回 404/无结果。
        // 这不是致命错误 —— LLM 手上已有 search 列表里的摘要,应据此继续,不该让整个
        // 工具调用带着原始 nginx 404 HTML 崩掉。故捕获错误,降级成一条明确提示(反虚构:
        // 让 LLM 用摘要、勿编全文)。真正的鉴权/网络错误仍会在 search 阶段如实反映。
        let resp = match yuandian::case_details(api_key, lib_normalized, None, Some(case_no)).await
        {
            Ok(r) => r,
            Err(e) => {
                crate::dlog!("get_case_detail 取全文失败(降级): {}", e);
                return Ok(ToolResult::plain(format!(
                    "未能取到案号「{}」的判决全文(元典返回:{})。\
                     请基于 search_cases_* 结果里该案的摘要继续分析,**不要编造**全文或裁判要旨;\
                     若摘要不足以支撑结论,如实说明「该案全文未取到」。",
                    case_no, e
                )));
            }
        };
        Ok(save_and_wrap(
            ctx,
            "rh_case_details",
            &cache_params,
            &cache_key,
            resp,
        ))
    }
}

pub struct CaseVectorSearch;

#[async_trait]
impl Tool for CaseVectorSearch {
    fn name(&self) -> &str {
        "case_vector_search"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/case_vector_search.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "自然语言描述,不是关键词"},
                "top_k": {"type": "integer", "description": "默认 10"}
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let query = require_str(args, "query")?;
        let top_k = opt_u32(args, "top_k");
        let cache_params = json!({"query": query, "top_k": top_k.unwrap_or(10)});
        if let Some(r) = try_kb_hit(ctx, "case_vector_search", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_key(ctx)?;
        let params = yuandian::CaseVectorSearchParams {
            query: query.to_string(),
            wenshu_filter: None,
            top_k,
        };
        let resp = yuandian::case_vector_search(api_key, &params).await?;
        Ok(save_and_wrap(
            ctx,
            "case_vector_search",
            &cache_params,
            query,
            resp,
        ))
    }
}
