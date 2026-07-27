//! 法规法条 5 个 tool(V0.2 D2-D3.B)。
//!
//! 全部走三段式:`try_kb_hit` → 调元典 `yuandian::*` → `save_and_wrap`。
//! 走 KB cache(仅复用通过现行法源时效门禁的法规法条,本地命中等于免费)。

use async_trait::async_trait;
use serde_json::{json, Value};
use walkdir::WalkDir;

use super::{
    opt_bool, opt_str, opt_u32, require_str, save_and_wrap, try_kb_hit, yuandian_credential, Tool,
    ToolContext, ToolError, ToolResult,
};
use crate::yuandian;

fn inactive_source_from_response(
    query_type: &str,
    response: &Value,
) -> Option<crate::local_kb::validity::InactiveLegalSourceInfo> {
    crate::local_kb::validity::sanitize_yuandian_legal_response(query_type, response.clone())
        .inactive_sources
        .into_iter()
        .next()
}

fn cached_inactive_source(
    ctx: &ToolContext<'_>,
    query_type: &str,
    cache_params: &Value,
) -> Option<crate::local_kb::validity::InactiveLegalSourceInfo> {
    if crate::local_kb::validity::historical_research_requested(query_type, cache_params) {
        return None;
    }
    if crate::chat::policy::requires_direct_yuandian(ctx.message_id) {
        return None;
    }
    let kb = ctx.local_kb?;
    let raw = kb.load_raw_response(query_type, cache_params).or_else(|| {
        let key = cache_params.get("key")?.as_str()?;
        cache_params
            .get("refer_date")
            .and_then(Value::as_str)
            .is_some_and(str::is_empty)
            .then(|| kb.load_raw_response(query_type, &json!({"key": key})))
            .flatten()
    })?;
    let parsed = serde_json::from_str::<Value>(&raw).ok()?;
    let validated = crate::yuandian::validate_business_response(parsed).ok()?;
    inactive_source_from_response(query_type, &validated)
}

fn replacement_query(source_name: &str) -> String {
    format!("{source_name} 现行修订版 替代法律法规")
}

fn legal_search_has_usable_result(query_type: &str, response: &Value) -> bool {
    match query_type {
        "rh_ft_search" | "rh_fg_search" => response
            .get("data")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty()),
        "law_vector_search" => ["/extra/fatiao", "/data/extra/fatiao"]
            .iter()
            .any(|pointer| {
                response
                    .pointer(pointer)
                    .and_then(Value::as_array)
                    .is_some_and(|items| !items.is_empty())
            }),
        _ => false,
    }
}

fn should_apply_local_first(query_type: &str, cache_params: &Value) -> bool {
    !crate::local_kb::validity::historical_research_requested(query_type, cache_params)
}

fn inactive_replacement_result(
    source: &crate::local_kb::validity::InactiveLegalSourceInfo,
    base_credits: u32,
    search_status: &str,
    search_payload: Value,
    kb_hit: bool,
) -> ToolResult {
    ToolResult {
        content: serde_json::to_string_pretty(&json!({
            "status": "inactive_source_rejected",
            "usable_as_authority": false,
            "rejected_source": {
                "name": source.name,
                "validity": source.status,
                "content_omitted": true
            },
            "replacement_search": {
                "status": search_status,
                "query": replacement_query(&source.name),
                "result": search_payload
            },
            "_note": "该法规已失效、废止或尚未生效，不得引用、写入报告或知识库。仅可使用替代检索返回的现行有效法源；没有可用结果时必须如实说明未找到。"
        }))
        .unwrap_or_else(|_| "失效法规已拒绝；替代检索结果无法序列化。".into()),
        yuandian_credits_used: base_credits,
        kb_hit,
    }
}

async fn search_replacement_for_inactive_source(
    ctx: &ToolContext<'_>,
    source: crate::local_kb::validity::InactiveLegalSourceInfo,
    base_credits: u32,
) -> ToolResult {
    let query = replacement_query(&source.name);
    if !crate::chat::policy::requires_direct_yuandian(ctx.message_id) {
        let local = crate::local_kb::retrieval::retrieve_local(
            ctx.local_kb,
            ctx.settings,
            crate::local_kb::retrieval::RetrievalDomain::Law,
            &query,
        )
        .await;
        if let Ok(report) = local {
            if report.is_sufficient() {
                return inactive_replacement_result(
                    &source,
                    base_credits,
                    "local_current_law_found",
                    serde_json::to_value(report).unwrap_or(Value::Null),
                    base_credits == 0,
                );
            }
        }
    }

    let Ok(api_key) = yuandian_credential(ctx).await else {
        return inactive_replacement_result(
            &source,
            base_credits,
            "external_search_not_executed",
            json!({"reason": "未配置元典 API key，本地也未找到足够可靠的现行替代法源"}),
            false,
        );
    };
    if let Err(error) = ensure_yuandian_budget(ctx, base_credits.saturating_add(10)).await {
        return inactive_replacement_result(
            &source,
            base_credits,
            "external_search_not_executed",
            json!({"reason": error.to_string()}),
            false,
        );
    }
    let params = yuandian::LawVectorSearchParams {
        query: query.clone(),
        rewrite_flag: Some(true),
        validities: Some(vec!["现行有效".into()]),
        effect_levels: None,
        return_num: Some(30),
        effect_level: None,
        valid_only: None,
        implement_date_start: None,
        implement_date_end: None,
        top_k: None,
    };
    match yuandian::law_vector_search(api_key, &params).await {
        Ok(response) => {
            let cache_params = serde_json::to_value(&params).unwrap_or_else(|_| {
                json!({
                    "query": query,
                    "validities": ["现行有效"]
                })
            });
            let replacement =
                save_and_wrap(ctx, "law_vector_search", &cache_params, &query, response);
            let payload = serde_json::from_str(&replacement.content)
                .unwrap_or_else(|_| json!({"raw_result": replacement.content}));
            let mut result = inactive_replacement_result(
                &source,
                base_credits.saturating_add(replacement.yuandian_credits_used),
                "external_current_law_search_completed",
                payload,
                false,
            );
            result.kb_hit = replacement.kb_hit && base_credits == 0;
            result
        }
        Err(error) => inactive_replacement_result(
            &source,
            base_credits,
            "external_search_failed",
            json!({"reason": error.to_string()}),
            false,
        ),
    }
}

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

fn ft_search_params_from_args(args: &Value) -> Result<yuandian::FtSearchParams, ToolError> {
    Ok(yuandian::FtSearchParams {
        keyword: require_str(args, "keyword")?.to_string(),
        search_mode: opt_str(args, "search_mode").map(String::from),
        fgmc: opt_str(args, "fgmc").map(String::from),
        effect_level: opt_str(args, "xljb_1").map(String::from),
        region: opt_str(args, "dy").map(String::from),
        validity: opt_str(args, "sxx")
            .map(String::from)
            .or_else(|| Some("现行有效".into())),
        publisher: opt_str(args, "fbbm").map(String::from),
        valid_only: None,
        top_k: Some(opt_u32(args, "top_k").unwrap_or(20)),
        publish_date_start: opt_str(args, "fbrq_start").map(String::from),
        publish_date_end: opt_str(args, "fbrq_end").map(String::from),
        implement_date_start: opt_str(args, "ssrq_start").map(String::from),
        implement_date_end: opt_str(args, "ssrq_end").map(String::from),
    })
}

fn fg_search_params_from_args(args: &Value) -> Result<yuandian::FgSearchParams, ToolError> {
    let keyword = opt_str(args, "keyword").map(String::from);
    let fgmc = opt_str(args, "fgmc").map(String::from);
    if keyword.is_none() && fgmc.is_none() {
        return Err(ToolError::InvalidArgs(
            "keyword 跟 fgmc 至少填一个,纯过滤无关键词易返回过宽".into(),
        ));
    }
    Ok(yuandian::FgSearchParams {
        keyword,
        search_mode: opt_str(args, "search_mode").map(String::from),
        fgmc,
        effect_level: opt_str(args, "xljb_1").map(String::from),
        region: opt_str(args, "dy").map(String::from),
        validity: opt_str(args, "sxx")
            .map(String::from)
            .or_else(|| Some("现行有效".into())),
        publisher: opt_str(args, "fbbm").map(String::from),
        valid_only: None,
        top_k: Some(opt_u32(args, "top_k").unwrap_or(20)),
        publish_date_start: opt_str(args, "fbrq_start").map(String::from),
        publish_date_end: opt_str(args, "fbrq_end").map(String::from),
        implement_date_start: opt_str(args, "ssrq_start").map(String::from),
        implement_date_end: opt_str(args, "ssrq_end").map(String::from),
    })
}

fn law_vector_params_from_args(args: &Value) -> Result<yuandian::LawVectorSearchParams, ToolError> {
    let filter = match args.get("fatiao_filter") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Object(filter)) => Value::Object(filter.clone()),
        Some(_) => return Err(ToolError::InvalidArgs("fatiao_filter 必须是对象".into())),
    };
    Ok(yuandian::LawVectorSearchParams {
        query: require_str(args, "query")?.to_string(),
        rewrite_flag: opt_bool(args, "rewrite_flag"),
        validities: opt_string_array(&filter, "sxx")?.or_else(|| Some(vec!["现行有效".into()])),
        effect_levels: opt_string_array(&filter, "effect1")?,
        return_num: Some(opt_u32(args, "return_num").unwrap_or(45)),
        effect_level: None,
        valid_only: None,
        implement_date_start: opt_str(&filter, "law_start").map(String::from),
        implement_date_end: opt_str(&filter, "law_end").map(String::from),
        top_k: None,
    })
}

pub struct SearchLaws;

#[async_trait]
impl Tool for SearchLaws {
    fn name(&self) -> &str {
        "search_laws"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/search_laws.md")
    }
    fn parameters_schema(&self) -> Value {
        super::yuandian_schema::law_keyword_search(true)
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let params = ft_search_params_from_args(args)?;
        let keyword = params.keyword.clone();
        let cache_params = serde_json::to_value(&params)
            .map_err(|error| ToolError::Runtime(format!("法条检索缓存参数序列化失败:{error}")))?;
        if let Some(r) = try_kb_hit(ctx, "rh_ft_search", &cache_params) {
            return Ok(r);
        }
        if should_apply_local_first("rh_ft_search", &cache_params) {
            if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
                crate::chat::retrieval_policy::local_first_gate(
                    ctx,
                    crate::local_kb::retrieval::RetrievalDomain::Law,
                    &keyword,
                )
                .await?
            {
                return Ok(result);
            }
        }

        let api_key = yuandian_credential(ctx).await?;
        let resp = yuandian::ft_search(api_key, &params).await?;
        if !crate::local_kb::validity::historical_research_requested("rh_ft_search", &cache_params)
        {
            let gated = crate::local_kb::validity::sanitize_yuandian_legal_response(
                "rh_ft_search",
                resp.clone(),
            );
            if !legal_search_has_usable_result("rh_ft_search", &gated.value) {
                if let Some(inactive) = gated.inactive_sources.into_iter().next() {
                    return Ok(search_replacement_for_inactive_source(ctx, inactive, 10).await);
                }
            }
        }
        Ok(save_and_wrap(
            ctx,
            "rh_ft_search",
            &cache_params,
            &keyword,
            resp,
        ))
    }
}

pub struct GetLawArticle;

#[async_trait]
impl Tool for GetLawArticle {
    fn name(&self) -> &str {
        "get_law_article"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/get_law_article.md")
    }
    fn parameters_schema(&self) -> Value {
        super::yuandian_schema::law_article_detail()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let id = opt_str(args, "id").map(String::from);
        let fgmc = opt_str(args, "fgmc").map(String::from);
        let ftnum = opt_str(args, "ftnum").map(String::from);
        let fgid = opt_str(args, "fgid").map(String::from);
        let refer_date = opt_str(args, "refer_date").map(String::from);
        // D5-3:接受 id / (fgmc+ftnum) / (fgid+ftnum) 三选一 —— 原 guard 漏了 fgid+ftnum,
        // 导致只带 fgid+ftnum(无 id/fgmc)时被前置拒掉、走不到下方省积分的全文路径。
        let has_fgid_ft = fgid.is_some() && ftnum.is_some();
        let has_fgmc_ft = fgmc.is_some() && ftnum.is_some();
        if id.is_none() && !has_fgmc_ft && !has_fgid_ft {
            return Err(ToolError::InvalidArgs(
                "需要填 id / (fgmc + ftnum) / (fgid + ftnum) 之一".into(),
            ));
        }
        // local-first 第一层：已知法规名+条号时，先在主库整部法规里精确抽条。
        // refer_date 是历史时点查询，不能拿未核版本的当前本地副本冒充，故仅当前版本走此路。
        let direct_yuandian = crate::chat::policy::requires_direct_yuandian(ctx.message_id);
        if !direct_yuandian && refer_date.is_none() {
            if let (Some(kb), Some(fgmc), Some(ftnum)) =
                (ctx.local_kb, fgmc.as_deref(), ftnum.as_deref())
            {
                if let Some(local) = find_local_law_article(&kb.root, fgmc, ftnum) {
                    return Ok(ToolResult {
                        content: serde_json::to_string_pretty(&json!({
                            "fgmc": fgmc,
                            "ftnum": ftnum,
                            "content": local.article,
                            "local_source": local.relative_path,
                            "_note": "本地整部法规精确命中，未调用元典"
                        }))
                        .unwrap_or(local.article),
                        yuandian_credits_used: 0,
                        kb_hit: true,
                    });
                }
            }
        }

        // 有 fgid 时按 ID 下载整部法规；没有 fgid 时先在下方用单条详情解析精确版本 ID，
        // 再按该 ID 拉整部。当前官方计费：法规详情 5 分/次、法条详情 1 分/次。
        // 默认整部入库是长期复用策略；整部接口/抽条失败时仍保留单条结果，绝不编造。
        if !direct_yuandian {
            if let (Some(fgid), Some(ftnum)) = (fgid.as_deref(), ftnum.as_deref()) {
                if let Some(r) =
                    try_fulltext_article(ctx, Some(fgid), None, ftnum, refer_date.as_deref())
                        .await?
                {
                    return Ok(r);
                }
            }
        }
        let cache_key = id.clone().unwrap_or_else(|| {
            format!(
                "{}-{}",
                fgmc.as_deref().unwrap_or(""),
                ftnum.as_deref().unwrap_or("")
            )
        });
        let cache_params = json!({
            "key": cache_key,
            "refer_date": refer_date.as_deref().unwrap_or("")
        });
        let historical =
            crate::local_kb::validity::historical_research_requested("rh_ft_detail", &cache_params);
        if let Some(inactive) = cached_inactive_source(ctx, "rh_ft_detail", &cache_params) {
            return Ok(search_replacement_for_inactive_source(ctx, inactive, 0).await);
        }
        if let Some(r) = try_kb_hit(ctx, "rh_ft_detail", &cache_params) {
            return Ok(r);
        }
        let api_key = yuandian_credential(ctx).await?;
        ensure_yuandian_budget(ctx, 1).await?;
        let params = yuandian::FtDetailParams {
            id: id.clone(),
            fgmc: fgmc.clone(),
            ftnum: ftnum.clone(),
            refer_date: refer_date.clone(),
        };
        let resp = yuandian::ft_detail(api_key, &params).await?;
        if !historical {
            if let Some(inactive) = inactive_source_from_response("rh_ft_detail", &resp) {
                return Ok(search_replacement_for_inactive_source(ctx, inactive, 1).await);
            }
        }
        // 没有 fgid 时先用 1 分单条详情解析出精确法规版本 ID，再按该 ID 拉整部法规。
        // 绝不按法规名盲拉全文（同名不同修订版可能条号错位）。首条最多 1+5 分，
        // 整部正式入库后，同一法规后续所有条文均为 0 分本地抽取。
        if !direct_yuandian {
            if let (Some(resolved_fgid), Some(ftnum)) =
                (fgid_from_law_detail(&resp), ftnum.as_deref())
            {
                if let Some(mut full) = try_fulltext_article(
                    ctx,
                    Some(&resolved_fgid),
                    None,
                    ftnum,
                    refer_date.as_deref(),
                )
                .await?
                {
                    full.yuandian_credits_used = full.yuandian_credits_used.saturating_add(1);
                    return Ok(full);
                }
            }
        }
        Ok(save_and_wrap(
            ctx,
            "rh_ft_detail",
            &cache_params,
            &cache_key,
            resp,
        ))
    }
}

fn fgid_from_law_detail(resp: &Value) -> Option<String> {
    resp.pointer("/data/fgid")
        .or_else(|| resp.pointer("/data/lst/0/fgid"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
}

/// V0.2.2 · 法规全文路径:按 `fgid`(法规 ID,保证版本正确)拿整部法规全文,从中按条号提取单条。
///
/// 返回:
/// - `Ok(Some(单条 ToolResult))` —— 成功提取(本地命中 0 积分 / 拉全文 5 积分,该法规后续条 0 积分)。
/// - `Ok(None)` —— 无 key / 拉全文失败 / 全文无此条号 → 调用方应**降级到单条接口**(不得编造)。
///
/// 安全网:版本由 `fgid` 保证(绝不按法规名拉,会拉错修订版);提取不到 → None 降级。
async fn try_fulltext_article(
    ctx: &ToolContext<'_>,
    fgid: Option<&str>,
    fgmc: Option<&str>,
    ftnum: &str,
    refer_date: Option<&str>,
) -> Result<Option<ToolResult>, ToolError> {
    let regulation_key = fgid.or(fgmc).unwrap_or_default();
    let fg_params = json!({
        "key": regulation_key,
        "refer_date": refer_date.unwrap_or("")
    });
    // 1) 本地法规全文缓存(按法规 ID/名称 + 时点版本)
    let cached: Option<Value> = ctx.local_kb.and_then(|kb| {
        let current = kb.load_raw_response("rh_fg_detail", &fg_params);
        // 兼容 2026-07-11 前未把 refer_date 放进 key 的永久缓存：当前版本请求可直接复用，
        // 避免升级后同一部法规再花 5 分；历史时点不能回退旧 key。
        let body = current.or_else(|| {
            refer_date
                .is_none()
                .then(|| kb.load_raw_response("rh_fg_detail", &json!({"key": regulation_key})))
                .flatten()
        });
        body.and_then(|s| serde_json::from_str(&s).ok())
    });
    let (mut resp, hit) = match cached {
        Some(j) => (j, true),
        None => {
            // 2) 未命中 → 按 fgid 拉整部法规全文(版本正确),顺手缓存供后续 0 积分命中
            let Ok(api_key) = yuandian_credential(ctx).await else {
                return Ok(None); // 无 key → 降级单条
            };
            ensure_yuandian_budget(ctx, 5).await?;
            let params = yuandian::FgDetailParams {
                id: fgid.map(String::from),
                fgmc: fgmc.map(String::from),
                refer_date: refer_date.map(String::from),
            };
            match yuandian::fg_detail(api_key, &params).await {
                Ok(r) => (r, false),
                Err(_) => return Ok(None), // 拉全文失败 → 降级单条
            }
        }
    };
    let historical =
        crate::local_kb::validity::historical_research_requested("rh_fg_detail", &fg_params);
    if historical {
        resp =
            crate::local_kb::validity::legal_response_for_request("rh_fg_detail", &fg_params, resp)
                .value;
    }
    if !historical {
        if let Some(inactive) = inactive_source_from_response("rh_fg_detail", &resp) {
            let base_credits = if hit { 0 } else { 5 };
            return Ok(Some(
                search_replacement_for_inactive_source(ctx, inactive, base_credits).await,
            ));
        }
    }
    if let Some(kb) = ctx.local_kb {
        if hit {
            // 旧缓存首次复用时顺手按新 legal-kb 模板收口主库，不调 API。
            let _ = crate::local_kb::ingest::ingest_regulation_detail(kb, &resp);
        } else if !super::response_is_empty("rh_fg_detail", &resp) {
            // 新取全文先通过时效门禁，再写可读详情、sidecar 与主库 L1。
            let body = serde_json::to_string_pretty(&resp).unwrap_or_default();
            super::persist_detail(kb, "rh_fg_detail", &fg_params, &resp, &body);
        }
    }
    // 3) 从全文按条号提取单条
    let Some(content) = resp.pointer("/data/content").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    match super::law_fulltext::extract_article(content, ftnum) {
        Some(article) => {
            // D5-2:包成统一 JSON(与单条降级路径及其它工具的 pretty-JSON 结果一致),
            // 并保留法规名/fgid 元数据供引用协议用,而不是返回裸文本。
            let fgmc = resp
                .pointer("/data/fgmc")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let resolved_fgid = resp
                .pointer("/data/fgid")
                .or_else(|| resp.pointer("/data/id"))
                .and_then(|v| v.as_str())
                .or(fgid)
                .unwrap_or("");
            let mut wrapped = json!({
                "fgmc": fgmc,
                "fgid": resolved_fgid,
                "ftnum": ftnum,
                "content": article,
            });
            if historical {
                wrapped["validity"] = resp
                    .pointer("/data/sxx")
                    .cloned()
                    .unwrap_or(Value::String("历史时点版本".into()));
                wrapped["_caseboard_validity_context"] = json!({
                    "mode": "historical_research_only",
                    "usable_as_current_authority": false,
                    "effective_date_must_be_verified": true,
                    "note": "该条文来自用户明确指定的历史时点，仅供历史适用法研究，不得表述为现行有效依据。"
                });
            }
            Ok(Some(ToolResult {
                content: serde_json::to_string_pretty(&wrapped).unwrap_or(article),
                yuandian_credits_used: if hit { 0 } else { 5 },
                kb_hit: hit,
            }))
        }
        None => Ok(None), // 全文里没这条号 → 降级单条,绝不编造
    }
}

async fn ensure_yuandian_budget(ctx: &ToolContext<'_>, cost: u32) -> Result<(), ToolError> {
    let Some(limit) = ctx.settings.yuandian_monthly_credit_limit else {
        return Ok(());
    };
    let month = crate::db::credits::current_year_month();
    let remaining = crate::db::credits::get_monthly_remaining(ctx.pool, &month, Some(limit)).await;
    if remaining < cost as i64 {
        return Err(ToolError::Runtime(format!(
            "本月元典积分余额不足：剩余 {}，本次联网预计 {} 分。已完成本地检索但未命中；请调整月度上限后再试。",
            remaining.max(0),
            cost
        )));
    }
    Ok(())
}

struct LocalArticle {
    article: String,
    relative_path: String,
}

/// 在本地主库中定位整部法规并按条号抽取。按法规规范名匹配，优先现行有效、raw/notes
/// 与元典法规全文；跳过废止归档、搜索片段和只有摘要的 source 页。
fn find_local_law_article(
    kb_root: &std::path::Path,
    law_name: &str,
    article_no: &str,
) -> Option<LocalArticle> {
    let root = kb_root.canonicalize().ok()?;
    let wanted = crate::local_kb::semantic::normalize_law_name(law_name);
    if wanted.is_empty() {
        return None;
    }
    let mut best: Option<(i32, LocalArticle)> = None;
    for entry in WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Ok(relative) = path.strip_prefix(&root) else {
            continue;
        };
        let rel = relative.to_string_lossy().replace('\\', "/");
        if rel.contains("_deprecated")
            || rel.contains("00_ARCHIVE")
            || rel.starts_with("wiki/sources/")
            || (rel.starts_with("raw/yuandian-cache/SEARCH-") || rel.ends_with(".raw.json"))
        {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("md" | "txt")
        ) {
            continue;
        }
        let normalized = crate::local_kb::semantic::normalize_law_name(file_name);
        if normalized != wanted && !normalized.contains(&wanted) && !wanted.contains(&normalized) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        if crate::local_kb::validity::is_inactive_regulation_text(&text) {
            continue;
        }
        let Some(article) = super::law_fulltext::extract_article(&text, article_no) else {
            continue;
        };
        let mut score = if normalized == wanted { 100 } else { 50 };
        if text.contains("现行有效") {
            score += 20;
        }
        if rel.starts_with("raw/notes/") {
            score += 10;
        } else if rel.starts_with("raw/yuandian-cache/法规-") {
            score += 8;
        }
        let candidate = LocalArticle {
            article,
            relative_path: rel,
        };
        if best
            .as_ref()
            .is_none_or(|(best_score, _)| score > *best_score)
        {
            best = Some((score, candidate));
        }
    }
    best.map(|(_, article)| article)
}

pub struct SearchRegulations;

#[async_trait]
impl Tool for SearchRegulations {
    fn name(&self) -> &str {
        "search_regulations"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/search_regulations.md")
    }
    fn parameters_schema(&self) -> Value {
        super::yuandian_schema::law_keyword_search(false)
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let params = fg_search_params_from_args(args)?;
        let keyword = params.keyword.clone();
        let fgmc = params.fgmc.clone();
        let cache_params = serde_json::to_value(&params)
            .map_err(|error| ToolError::Runtime(format!("法规检索缓存参数序列化失败:{error}")))?;
        if let Some(r) = try_kb_hit(ctx, "rh_fg_search", &cache_params) {
            return Ok(r);
        }

        let summary = keyword.clone().or(fgmc.clone()).unwrap_or_default();
        if should_apply_local_first("rh_fg_search", &cache_params) {
            if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
                crate::chat::retrieval_policy::local_first_gate(
                    ctx,
                    crate::local_kb::retrieval::RetrievalDomain::Law,
                    &summary,
                )
                .await?
            {
                return Ok(result);
            }
        }

        let api_key = yuandian_credential(ctx).await?;
        let resp = yuandian::fg_search(api_key, &params).await?;
        if !crate::local_kb::validity::historical_research_requested("rh_fg_search", &cache_params)
        {
            let gated = crate::local_kb::validity::sanitize_yuandian_legal_response(
                "rh_fg_search",
                resp.clone(),
            );
            if !legal_search_has_usable_result("rh_fg_search", &gated.value) {
                if let Some(inactive) = gated.inactive_sources.into_iter().next() {
                    return Ok(search_replacement_for_inactive_source(ctx, inactive, 10).await);
                }
            }
        }
        Ok(save_and_wrap(
            ctx,
            "rh_fg_search",
            &cache_params,
            &summary,
            resp,
        ))
    }
}

pub struct GetRegulationDetail;

#[async_trait]
impl Tool for GetRegulationDetail {
    fn name(&self) -> &str {
        "get_regulation_detail"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/get_regulation_detail.md")
    }
    fn parameters_schema(&self) -> Value {
        super::yuandian_schema::regulation_detail()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let id = opt_str(args, "id").map(String::from);
        let fgmc = opt_str(args, "fgmc").map(String::from);
        if id.is_none() && fgmc.is_none() {
            return Err(ToolError::InvalidArgs("需要填 id 或 fgmc 二选一".into()));
        }
        let cache_key = id
            .clone()
            .unwrap_or_else(|| fgmc.clone().unwrap_or_default());
        let refer_date = opt_str(args, "refer_date").map(String::from);
        let cache_params = json!({
            "key": cache_key,
            "refer_date": refer_date.as_deref().unwrap_or("")
        });
        let historical =
            crate::local_kb::validity::historical_research_requested("rh_fg_detail", &cache_params);
        if let Some(inactive) = cached_inactive_source(ctx, "rh_fg_detail", &cache_params) {
            return Ok(search_replacement_for_inactive_source(ctx, inactive, 0).await);
        }
        if let Some(mut r) = try_kb_hit(ctx, "rh_fg_detail", &cache_params) {
            r.content = regulation_detail_for_llm(&r.content);
            return Ok(r);
        }
        let api_key = yuandian_credential(ctx).await?;
        ensure_yuandian_budget(ctx, 5).await?;
        let params = yuandian::FgDetailParams {
            id,
            fgmc,
            refer_date,
        };
        let resp = yuandian::fg_detail(api_key, &params).await?;
        if !historical {
            if let Some(inactive) = inactive_source_from_response("rh_fg_detail", &resp) {
                return Ok(search_replacement_for_inactive_source(ctx, inactive, 5).await);
            }
        }
        let mut r = save_and_wrap(ctx, "rh_fg_detail", &cache_params, &cache_key, resp);
        r.content = regulation_detail_for_llm(&r.content);
        Ok(r)
    }
}

/// 元典付费详情不做二次裁剪；缓存命中与新请求都把完整响应交给模型。
fn regulation_detail_for_llm(full_json: &str) -> String {
    full_json.to_string()
}

pub struct LawVectorSearch;

#[async_trait]
impl Tool for LawVectorSearch {
    fn name(&self) -> &str {
        "law_vector_search"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/law_vector_search.md")
    }
    fn parameters_schema(&self) -> Value {
        super::yuandian_schema::law_vector_search()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let params = law_vector_params_from_args(args)?;
        let query = params.query.clone();
        let cache_params = serde_json::to_value(&params).map_err(|error| {
            ToolError::Runtime(format!("法规语义检索缓存参数序列化失败:{error}"))
        })?;
        if let Some(r) = try_kb_hit(ctx, "law_vector_search", &cache_params) {
            return Ok(r);
        }
        if should_apply_local_first("law_vector_search", &cache_params) {
            if let crate::chat::retrieval_policy::ExternalGateDecision::UseLocal(result) =
                crate::chat::retrieval_policy::local_first_gate(
                    ctx,
                    crate::local_kb::retrieval::RetrievalDomain::Law,
                    &query,
                )
                .await?
            {
                return Ok(result);
            }
        }
        let api_key = yuandian_credential(ctx).await?;
        let resp = yuandian::law_vector_search(api_key, &params).await?;
        if !crate::local_kb::validity::historical_research_requested(
            "law_vector_search",
            &cache_params,
        ) {
            let gated = crate::local_kb::validity::sanitize_yuandian_legal_response(
                "law_vector_search",
                resp.clone(),
            );
            if !legal_search_has_usable_result("law_vector_search", &gated.value) {
                if let Some(inactive) = gated.inactive_sources.into_iter().next() {
                    return Ok(search_replacement_for_inactive_source(ctx, inactive, 10).await);
                }
            }
        }
        Ok(save_and_wrap(
            ctx,
            "law_vector_search",
            &cache_params,
            &query,
            resp,
        ))
    }
}
