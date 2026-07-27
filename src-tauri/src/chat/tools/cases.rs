//! 案例 4 个 tool(V0.2 D2-D3.C)。
//!
//! 关键词、权威和向量检索均按元典当前官方字段透传完整过滤条件；
//! `get_case_detail` 直接调用官方 `rh_case_details` GET 端点，以案号或 id 定位详情。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{
    opt_u32, require_str, save_and_wrap, try_kb_hit, yuandian_credential, Tool, ToolContext,
    ToolError, ToolResult,
};
use crate::yuandian;

type CaseDetailRequest = (Option<String>, Option<String>, Option<String>);

fn opt_string_array(args: &Value, key: &str) -> Result<Option<Vec<String>>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let values = value
        .as_array()
        .ok_or_else(|| ToolError::InvalidArgs(format!("{key} 必须是字符串数组")))?;
    let mut out = Vec::with_capacity(values.len());
    for value in values {
        let text = value
            .as_str()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .ok_or_else(|| ToolError::InvalidArgs(format!("{key} 只能包含非空字符串")))?;
        out.push(text.to_string());
    }
    Ok((!out.is_empty()).then_some(out))
}

fn ptal_params_from_args(args: &Value) -> Result<yuandian::PtalSearchParams, ToolError> {
    let params = yuandian::PtalSearchParams {
        ah: super::opt_str(args, "ah").map(String::from),
        title: super::opt_str(args, "title").map(String::from),
        ssqy: super::opt_str(args, "ssqy").map(String::from),
        ay: opt_string_array(args, "ay")?,
        jbdw: opt_string_array(args, "jbdw")?,
        xzqh_p: opt_string_array(args, "xzqh_p")?,
        wszl: opt_string_array(args, "wszl")?,
        ajlb: super::opt_str(args, "ajlb").map(String::from),
        ja_start: super::opt_str(args, "ja_start").map(String::from),
        ja_end: super::opt_str(args, "ja_end").map(String::from),
        qw: super::opt_str(args, "qw").map(String::from),
        fxgc: super::opt_str(args, "fxgc").map(String::from),
        search_mode: super::opt_str(args, "search_mode").map(String::from),
        yyft: opt_string_array(args, "yyft")?,
        ft_search_mode: super::opt_str(args, "ft_search_mode").map(String::from),
        top_k: Some(opt_u32(args, "top_k").unwrap_or(20)),
    };
    ptal_summary(&params)?;
    Ok(params)
}

fn qwal_params_from_args(args: &Value) -> Result<yuandian::QwalSearchParams, ToolError> {
    let params = yuandian::QwalSearchParams {
        ah: super::opt_str(args, "ah").map(String::from),
        title: super::opt_str(args, "title").map(String::from),
        ay: opt_string_array(args, "ay")?,
        jbdw: opt_string_array(args, "jbdw")?,
        source: opt_string_array(args, "source")?,
        xzqh_p: opt_string_array(args, "xzqh_p")?,
        wszl: opt_string_array(args, "wszl")?,
        ajlb: super::opt_str(args, "ajlb").map(String::from),
        ja_start: super::opt_str(args, "ja_start").map(String::from),
        ja_end: super::opt_str(args, "ja_end").map(String::from),
        qw: super::opt_str(args, "qw").map(String::from),
        search_mode: super::opt_str(args, "search_mode").map(String::from),
        top_k: Some(opt_u32(args, "top_k").unwrap_or(20)),
    };
    qwal_summary(&params)?;
    Ok(params)
}

fn ptal_summary(params: &yuandian::PtalSearchParams) -> Result<String, ToolError> {
    params
        .qw
        .as_ref()
        .or(params.fxgc.as_ref())
        .or(params.ah.as_ref())
        .or(params.title.as_ref())
        .or_else(|| params.ay.as_ref().and_then(|values| values.first()))
        .or_else(|| params.yyft.as_ref().and_then(|values| values.first()))
        .cloned()
        .ok_or_else(|| ToolError::InvalidArgs("普通案例检索至少填写一个关键词或过滤条件".into()))
}

fn qwal_summary(params: &yuandian::QwalSearchParams) -> Result<String, ToolError> {
    params
        .qw
        .as_ref()
        .or(params.ah.as_ref())
        .or(params.title.as_ref())
        .or_else(|| params.ay.as_ref().and_then(|values| values.first()))
        .or_else(|| params.source.as_ref().and_then(|values| values.first()))
        .cloned()
        .ok_or_else(|| ToolError::InvalidArgs("权威案例检索至少填写一个关键词或过滤条件".into()))
}

fn case_vector_params_from_args(
    args: &Value,
) -> Result<yuandian::CaseVectorSearchParams, ToolError> {
    let filter = match args.get("wenshu_filter") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Object(filter)) => Value::Object(filter.clone()),
        Some(_) => return Err(ToolError::InvalidArgs("wenshu_filter 必须是对象".into())),
    };
    let wenshu_filter = if filter.is_object() {
        Some(yuandian::WenshuFilter {
            wenshu_type: super::opt_str(&filter, "wenshu_type").map(String::from),
            ay: opt_string_array(&filter, "ay")?,
            wszl: opt_string_array(&filter, "wszl")?,
            ja_start: super::opt_str(&filter, "ja_start").map(String::from),
            ja_end: super::opt_str(&filter, "ja_end").map(String::from),
            dianxing: super::opt_bool(&filter, "dianxing"),
            fayuan: opt_string_array(&filter, "fayuan")?,
            source: opt_string_array(&filter, "source")?,
            cj: super::opt_str(&filter, "cj").map(String::from),
            xzqh_p: super::opt_str(&filter, "xzqh_p").map(String::from),
            xzqh_c: super::opt_str(&filter, "xzqh_c").map(String::from),
        })
    } else {
        None
    };
    Ok(yuandian::CaseVectorSearchParams {
        query: require_str(args, "query")?.to_string(),
        rewrite_flag: super::opt_bool(args, "rewrite_flag"),
        wenshu_filter,
        return_num: Some(opt_u32(args, "return_num").unwrap_or(45)),
        top_k: None,
    })
}

fn case_detail_request_from_args(args: &Value) -> Result<CaseDetailRequest, ToolError> {
    let case_type = super::opt_str(args, "type").map(String::from);
    if case_type
        .as_deref()
        .is_some_and(|value| !matches!(value, "ptal" | "qwal"))
    {
        return Err(ToolError::InvalidArgs("type 只能是 ptal 或 qwal".into()));
    }
    let id = super::opt_str(args, "id").map(String::from);
    let case_no = super::opt_str(args, "ah")
        .or_else(|| super::opt_str(args, "case_no"))
        .map(String::from);
    if id.is_none() && case_no.is_none() {
        return Err(ToolError::InvalidArgs("需要填 id 或 ah 二选一".into()));
    }
    Ok((case_type, id, case_no))
}

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
        super::yuandian_schema::case_keyword_search(false)
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let params = ptal_params_from_args(args)?;
        let summary = ptal_summary(&params)?;
        let cache_params = serde_json::to_value(&params).map_err(|error| {
            ToolError::Runtime(format!("普通案例检索缓存参数序列化失败:{error}"))
        })?;
        if let Some(r) = try_kb_hit(ctx, "rh_ptal_search", &cache_params) {
            return Ok(r);
        }
        if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
            crate::chat::retrieval_policy::local_first_gate(
                ctx,
                crate::local_kb::retrieval::RetrievalDomain::Case,
                &summary,
            )
            .await?
        {
            return Ok(result);
        }
        let api_key = yuandian_credential(ctx).await?;
        let resp = yuandian::search_ptal_with_params(api_key, &params).await?;
        Ok(save_and_wrap(
            ctx,
            "rh_ptal_search",
            &cache_params,
            &summary,
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
        super::yuandian_schema::case_keyword_search(true)
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let params = qwal_params_from_args(args)?;
        let summary = qwal_summary(&params)?;
        let cache_params = serde_json::to_value(&params).map_err(|error| {
            ToolError::Runtime(format!("权威案例检索缓存参数序列化失败:{error}"))
        })?;
        if let Some(r) = try_kb_hit(ctx, "rh_qwal_search", &cache_params) {
            return Ok(r);
        }
        if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
            crate::chat::retrieval_policy::local_first_gate(
                ctx,
                crate::local_kb::retrieval::RetrievalDomain::Case,
                &summary,
            )
            .await?
        {
            return Ok(result);
        }
        let api_key = yuandian_credential(ctx).await?;
        let resp = yuandian::search_qwal_with_params(api_key, &params).await?;
        Ok(save_and_wrap(
            ctx,
            "rh_qwal_search",
            &cache_params,
            &summary,
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
        super::yuandian_schema::case_detail()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let (case_type, id, case_no) = case_detail_request_from_args(args)?;
        let identifier = id.as_deref().or(case_no.as_deref()).unwrap_or_default();
        let cache_key = format!("{}-{}", case_type.as_deref().unwrap_or("all"), identifier);
        let cache_params = json!({"type": case_type, "id": id, "ah": case_no});
        if let Some(r) = try_kb_hit(ctx, "rh_case_details", &cache_params) {
            return Ok(r);
        }
        if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
            crate::chat::retrieval_policy::local_first_gate(
                ctx,
                crate::local_kb::retrieval::RetrievalDomain::Case,
                identifier,
            )
            .await?
        {
            return Ok(result);
        }
        let api_key = yuandian_credential(ctx).await?;
        // 直接走官方 rh_case_details(5 分)，不再用 10 分关键词检索冒充详情。
        // 取详情是**尽力而为**:某些案号(尤其外地/冷门库)元典会返回 404/无结果。
        // 这不是致命错误 —— LLM 手上已有 search 列表里的摘要,应据此继续,不该让整个
        // 工具调用带着原始 nginx 404 HTML 崩掉。故捕获错误,降级成一条明确提示(反虚构:
        // 让 LLM 用摘要、勿编全文)。真正的鉴权/网络错误仍会在 search 阶段如实反映。
        let resp = match yuandian::case_details(
            api_key,
            case_type.as_deref(),
            id.as_deref(),
            case_no.as_deref(),
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                crate::dlog!("get_case_detail 取全文失败(降级): {}", e);
                return Ok(ToolResult::plain(format!(
                    "未能取到案号「{}」的判决全文(元典返回:{})。\
                     请基于 search_cases_* 结果里该案的摘要继续分析,**不要编造**全文或裁判要旨;\
                     若摘要不足以支撑结论,如实说明「该案全文未取到」。",
                    identifier, e
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
        super::yuandian_schema::case_vector_search()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let params = case_vector_params_from_args(args)?;
        let query = params.query.clone();
        let cache_params = serde_json::to_value(&params).map_err(|error| {
            ToolError::Runtime(format!("案例语义检索缓存参数序列化失败:{error}"))
        })?;
        if let Some(r) = try_kb_hit(ctx, "case_vector_search", &cache_params) {
            return Ok(r);
        }
        if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
            crate::chat::retrieval_policy::local_first_gate(
                ctx,
                crate::local_kb::retrieval::RetrievalDomain::Case,
                &query,
            )
            .await?
        {
            return Ok(result);
        }
        let api_key = yuandian_credential(ctx).await?;
        let resp = yuandian::case_vector_search(api_key, &params).await?;
        Ok(save_and_wrap(
            ctx,
            "case_vector_search",
            &cache_params,
            &query,
            resp,
        ))
    }
}
