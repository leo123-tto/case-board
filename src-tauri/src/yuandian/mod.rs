//! 元典法律开放平台 API client(2026-05-24 k)。
//!
//! 用于执行案件查被执行人 / 财产线索:工商信息、被执行案件、失信、限消、股权出质 / 冻结、
//! 对外投资、欠税、行政处罚、法律文书、关联公司等。
//!
//! base: https://open.chineselaw.com/open
//! auth header: `X-Api-Key: sk_xxxxxxxxxx`
//!
//! 申请 key:https://open.chineselaw.com/
//! 不入 git;落 settings.json 本地保存。

pub(crate) mod artifact_binding;
pub mod balance;
pub mod deep_dive;
pub mod full_report;
pub mod orchestrator;
pub mod risk_assessment;

use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const BASE_URL: &str = "https://open.chineselaw.com/open";
const YUANDIAN_STABLE_INVENTORY_ID: &str = "settings:yuandian_api_key";
const YUANDIAN_CONNECTOR_ID: &str = "connector.yuandian";

pub type YuandianCredentialSource = crate::credentials_bridge::PendingCredentialSource;

/// 元典生产调用只长期保存 3A journal 的非秘密定位信息；每个实际 HTTP request
/// 在低层 `yd_get` / `yd_post` 内签发并消费一张 fresh lease。
pub fn credential_source() -> YuandianCredentialSource {
    YuandianCredentialSource::pending(
        crate::credentials_bridge::BridgeCredentialConsumer::YuandianConnector,
        YUANDIAN_STABLE_INVENTORY_ID,
        YUANDIAN_CONNECTOR_ID,
    )
}

// ───── 报告落盘的共享 helper(原本在 deep_dive / full_report / risk_assessment /
//        orchestrator 各抄一份,2026-06-03 收口到此,行为不变)─────

/// 某案件的元典报告目录:`<app_data>/external/<case_id>/reports`。
pub(crate) fn reports_dir_for_case(case_id: &str) -> Result<PathBuf, String> {
    let base = crate::db::app_data_dir().map_err(|e| format!("无法定位 app data dir: {}", e))?;
    Ok(base.join("external").join(case_id).join("reports"))
}

/// 把 JSON 值落成 `<subject>_<endpoint>.json` 到指定目录。
pub(crate) fn save_json(
    dir: &Path,
    subject: &str,
    endpoint: &str,
    v: &Value,
) -> Result<(), String> {
    let path = dir.join(file_name(subject, endpoint));
    let text = serde_json::to_string_pretty(v).map_err(|e| format!("序列化 JSON 失败:{}", e))?;
    std::fs::write(&path, text).map_err(|e| format!("写 {} 失败:{}", path.display(), e))?;
    Ok(())
}

/// 生成文件名:替换路径不友好字符(中英文括号 / 空格 / 分隔符)。
pub(crate) fn file_name(subject: &str, endpoint: &str) -> String {
    const UNSAFE: &[char] = &['/', '\\', ' ', '(', ')', '（', '）'];
    let safe: String = subject
        .chars()
        .map(|c| if UNSAFE.contains(&c) { '_' } else { c })
        .collect();
    format!("{}_{}.json", safe, endpoint)
}

/// 剥掉 LLM 输出最外层的 Markdown / JSON 代码围栏(```markdown / ```md / ```json / ```)。
///
/// 三份报告(deep_dive / full_report / risk_assessment)的 LLM 都被 prompt 要求「不要包围栏」,
/// 但模型偶尔不听会把整篇报告 / 整个 JSON 裹进 ``` 里 —— 不剥的话落盘 .md 渲染异常、
/// parse JSON 直接失败。只剥**最外层**一对围栏(前缀 + 后缀各一次),报告正文里合法的代码块不受影响。
/// (2026-06-03 收口:原本 full_report 处理 markdown/md、risk_assessment 处理 json、deep_dive 完全不剥 B12)
pub(crate) fn strip_md_fence(content: &str) -> String {
    let mut text = content.trim();
    for prefix in ["```markdown", "```md", "```json", "```"] {
        if let Some(stripped) = text.strip_prefix(prefix) {
            text = stripped.trim();
            break;
        }
    }
    if let Some(stripped) = text.strip_suffix("```") {
        text = stripped.trim();
    }
    text.to_string()
}

/// 拉案件元信息(立案日 / 案号 / 案件名)拼成 Markdown 段,prepend 到 LLM corpus 顶部,
/// 让模型拿到拒执 cutoff。三份报告(risk / deep_dive / full_report)原各抄一份逐字相同,2026-06-03 收口(B1)。
pub(crate) async fn fetch_case_meta_md(pool: &sqlx::SqlitePool, case_id: &str) -> String {
    let row: Option<(Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT name, case_no, agg_filed_at FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match row {
        Some((name, case_no, filed_at)) => format!(
            "========== 案件元信息 ==========\n\
             - 案件名称:{}\n\
             - 案号:{}\n\
             - **立案日(拒执 cutoff)**:{}\n\n\
             ⚠️ 请用立案日做时间切线:工商变更 / 对外投资 / 股东变更 / 出资变更里,\n\
             **立案日之后**的变更视为拒执风险线索;之前的不构成拒执。\n",
            name.as_deref().unwrap_or("(未知)"),
            case_no.as_deref().unwrap_or("(未知)"),
            filed_at
                .as_deref()
                .unwrap_or("(LLM 还没抽到立案日 — 无法做拒执 cutoff,请只列变更事实不做时间判断)"),
        ),
        None => "========== 案件元信息 ==========\n(找不到案件记录)\n".to_string(),
    }
}

/// 三份报告(risk / deep_dive / full_report)调 DeepSeek 的差异参数。
pub(crate) struct LlmCallOpts {
    pub max_tokens: u32,
    pub temperature: f64,
    /// 实际 timeout = cfg.timeout_secs * timeout_mult(deep_dive 用 4,其余 3)。
    pub timeout_mult: u64,
    /// 是否带 response_format = json_object(仅 risk_assessment 要 JSON 输出)。
    pub json_object: bool,
}

/// 构造 DeepSeek chat/completions 请求 body。抽成纯函数便于契约测试锁住 wire 形状(B2)。
fn build_llm_body(model: &str, system: &str, user: &str, opts: &LlmCallOpts) -> Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "max_tokens": opts.max_tokens,
        "temperature": opts.temperature,
        "stream": false,
    });
    if opts.json_object {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }
    body
}

/// 三份报告共用的 DeepSeek 单次调用:发 system + user,拿回 message.content 文本。
/// 原 risk / deep_dive / full_report 各抄 ~35 行 HTTP 样板,2026-06-03 收口(B2)。
/// 错误一律透传真因(坑#8):client 创建 / 网络 / HTTP 状态 / 响应解析 / 无 content。
pub(crate) async fn call_llm(
    cfg: &crate::llm::LlmConfig,
    system: &str,
    user: &str,
    opts: LlmCallOpts,
) -> Result<String, String> {
    let body = build_llm_body(&cfg.model, system, user, &opts);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            cfg.timeout_secs * opts.timeout_mult,
        ))
        .build()
        .map_err(|e| format!("HTTP client 创建失败:{}", e))?;
    let (req, credential) = cfg
        .authorize_request(client.post(&cfg.endpoint).json(&body))
        .await?;
    let resp = req.send().await.map_err(|e| {
        let message = credential
            .as_ref()
            .map_or_else(|| e.to_string(), |value| value.redact(&e.to_string()));
        format!("LLM 调用失败:{message}")
    })?;
    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let text = match credential.as_ref() {
            Some(value) => value.redact(&text),
            None => text,
        };
        return Err(format!("LLM HTTP {}: {}", code, text));
    }
    let json: Value = resp
        .json()
        .await
        .map_err(|e| format!("LLM 响应解析失败:{}", e))?;
    json.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .map(|s| {
            credential
                .as_ref()
                .map_or_else(|| s.to_string(), |value| value.redact(s))
        })
        .ok_or_else(|| "LLM 响应无 content".to_string())
}

#[derive(Debug, thiserror::Error)]
pub enum YuandianError {
    #[error("元典 API key 未配置(请到设置里填入)")]
    NoApiKey,
    #[error("元典网络错误:{0}")]
    Network(String),
    #[error("元典 HTTP {0}:{1}")]
    HttpStatus(u16, String),
    #[error("元典响应解析失败:{0}")]
    Json(String),
    #[error("元典业务错误 code={0}:{1}")]
    BusinessStatus(i64, String),
    #[error("{0}")]
    Credential(String),
}

impl serde::Serialize for YuandianError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
const HALL_DETECT_TIMEOUT_SECS: u64 = 120;

fn request_timeout_secs(path: &str) -> u64 {
    if path == "/hall_detect" {
        HALL_DETECT_TIMEOUT_SECS
    } else {
        DEFAULT_REQUEST_TIMEOUT_SECS
    }
}

/// 拿一个 reqwest client。幻觉核验需要服务端抽取并逐条比对引用，使用独立长超时；
/// 其它查询仍保持 30 秒，避免普通检索长时间挂起。
fn build_client(path: &str) -> Result<reqwest::Client, YuandianError> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(request_timeout_secs(path)))
        .build()
        .map_err(|e| YuandianError::Network(e.to_string()))
}

/// 通用 GET 请求(元典大部分接口都是 GET + query params)。
async fn yd_get(
    credential: &YuandianCredentialSource,
    path: &str,
    params: &[(&str, String)],
) -> Result<Value, YuandianError> {
    let material = credential
        .issue_material()
        .await
        .map_err(YuandianError::Credential)?;
    let client = build_client(path)?;
    let resp = client
        .get(format!("{}{}", BASE_URL, path))
        .header("X-Api-Key", material.expose())
        .header("accept", "application/json;charset=UTF-8")
        .query(params)
        .send()
        .await
        .map_err(|e| YuandianError::Network(material.redact(&e.to_string())))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| YuandianError::Network(material.redact(&e.to_string())))?;
    let body = material.redact(&body);
    if !status.is_success() {
        return Err(YuandianError::HttpStatus(status.as_u16(), body));
    }

    let value =
        serde_json::from_str::<Value>(&body).map_err(|e| YuandianError::Json(e.to_string()))?;
    validate_business_response(value)
}

/// 通用 POST 请求(裁判文书 / 法规等检索是 POST + JSON body)。
async fn yd_post(
    credential: &YuandianCredentialSource,
    path: &str,
    body: &Value,
) -> Result<Value, YuandianError> {
    let material = credential
        .issue_material()
        .await
        .map_err(YuandianError::Credential)?;
    let client = build_client(path)?;
    let resp = client
        .post(format!("{}{}", BASE_URL, path))
        .header("X-Api-Key", material.expose())
        .header("accept", "application/json;charset=UTF-8")
        .header("Content-Type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| YuandianError::Network(material.redact(&e.to_string())))?;

    let status = resp.status();
    let response_body = resp
        .text()
        .await
        .map_err(|e| YuandianError::Network(material.redact(&e.to_string())))?;
    let response_body = material.redact(&response_body);
    if !status.is_success() {
        return Err(YuandianError::HttpStatus(status.as_u16(), response_body));
    }

    let value = serde_json::from_str::<Value>(&response_body)
        .map_err(|e| YuandianError::Json(e.to_string()))?;
    validate_business_response(value)
}

/// 元典会在 HTTP 200 里返回业务错误（如 code=401/404/500）。只有 200/201 视为成功；
/// 没有 code 的历史响应保持兼容。业务失败必须在缓存/记账前变成 Err，避免把失败外壳
/// 当成“已查到”写入知识库并污染积分统计。
pub(crate) fn validate_business_response(value: Value) -> Result<Value, YuandianError> {
    let code = value.get("code").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
    });
    if let Some(code) = code {
        if !matches!(code, 200 | 201) {
            let message = value
                .get("message")
                .or_else(|| value.get("msg"))
                .and_then(Value::as_str)
                .unwrap_or("未提供错误信息")
                .to_string();
            return Err(YuandianError::BusinessStatus(code, message));
        }
    }
    Ok(value)
}

/* ============ 企业类(C1-C4)============ */

/// 企业名称 / 关键字搜索 — 拿候选 + id + 统一信用代码
fn enterprise_search_query(name: &str, top_k: u32) -> Vec<(&'static str, String)> {
    vec![
        ("name", name.to_string()),
        ("top_k", top_k.clamp(1, 50).to_string()),
    ]
}

pub async fn enterprise_search_with_limit(
    api_key: &YuandianCredentialSource,
    name: &str,
    top_k: u32,
) -> Result<Value, YuandianError> {
    yd_get(
        api_key,
        "/rh_enterpriseSearch",
        &enterprise_search_query(name, top_k),
    )
    .await
}

pub async fn enterprise_search(
    api_key: &YuandianCredentialSource,
    name: &str,
) -> Result<Value, YuandianError> {
    enterprise_search_with_limit(api_key, name, 10).await
}

/// 企业聚合摘要 — 一次拿所有维度(主体 / 风险 / 涉诉 / 财产线索摘要)
pub async fn enterprise_aggregation_summary(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
) -> Result<Value, YuandianError> {
    yd_get(
        api_key,
        "/rh_enterpriseAggregationSummary",
        &id_or_uscc.to_params(),
    )
    .await
}

/// 失信被执行人(老赖名单)
pub async fn enterprise_executions(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseExecutions", &params).await
}

/// 被执行人(普通执行,不一定老赖)
pub async fn enterprise_executed_person(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseExecutedPerson", &params).await
}

/// 法律文书列表(判决/裁定/调解/...)
pub async fn enterprise_writ_list(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseWritList", &params).await
}

/// 法院公告(含限消)
pub async fn enterprise_court_notice(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseCourtNotice", &params).await
}

/// 开庭公告
pub async fn enterprise_court_session_notice(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseCourtSessionNotice", &params).await
}

/// 股权出质
pub async fn enterprise_pledge(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterprisePledge", &params).await
}

/// 股权冻结(执行能查到的财产线索 ⭐)
pub async fn enterprise_frozen_equity(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseFrozenEquity", &params).await
}

/// 对外投资(关联公司 → 财产线索 ⭐)
pub async fn enterprise_out_invest(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseOutInvest", &params).await
}

/// 工商变更
pub async fn enterprise_change_info(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseChangeInfo", &params).await
}

/// 担保
pub async fn enterprise_guaranty(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseGuaranty", &params).await
}

/// 行政处罚
pub async fn enterprise_punishment(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterprisePunishment", &params).await
}

/// 经营异常
pub async fn enterprise_abnormal_operation(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseAbnormalOperation", &params).await
}

/// 严重违法
pub async fn enterprise_serious_illegal(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseSeriousIllegal", &params).await
}

/// 2026-05-25 V0.1.9 加 · 欠税公告(可作为财产线索)
pub async fn enterprise_corporate_tax(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    page: u32,
) -> Result<Value, YuandianError> {
    let mut params = id_or_uscc.to_params();
    params.push(("pageNo", page.to_string()));
    yd_get(api_key, "/rh_enterpriseCorporateTax", &params).await
}

const ENTERPRISE_ANNUAL_REPORT_METHOD: &str = "GET";

fn enterprise_annual_report_query(id_or_uscc: &EntityId, year: u32) -> Vec<(&'static str, String)> {
    let mut params = id_or_uscc.to_params();
    params.push(("year", year.to_string()));
    params
}

/// 2026-05-25 V0.1.9 加 · 企业年报详情(GET,按年份)
/// 拒执判断要拿"立案前一年 + 当年"两份年报,对比股东出资 / 总资产变化
pub async fn enterprise_annual_report(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
    year: u32,
) -> Result<Value, YuandianError> {
    debug_assert_eq!(ENTERPRISE_ANNUAL_REPORT_METHOD, "GET");
    yd_get(
        api_key,
        "/rh_enterpriseAnnualReport",
        &enterprise_annual_report_query(id_or_uscc, year),
    )
    .await
}

/* ============ 上市公司公告检索 ============ */

/// 上市公司公告关键词检索(`rh_ssgsgg_search`,10 积分)。
///
/// 官方约定:全部字段可选,但**请求体不能为空**(`top_k` 不计入),空会返回失败;
/// `fbrq_start` 不得晚于 `fbrq_end`。
///
/// 真机实测(2026-07-26,`cargo run --example yuandian_ssgsgg_probe`):
/// - `search_mode` 网关**大小写不敏感**,`AND` 与 `and` 都返回 code=200。
///   schema 按官方文档锁大写即可,不必像 rh_ptal_search 那样担心大小写判失败。
/// - 只带 `top_k` 的 body(`{"top_k":3}`)被官方判 code=501 参数异常 —— 印证
///   `ssgsgg_params_from_args` 提前拦截是必要的,否则白发一次请求。
#[derive(Serialize, Default, Debug, Clone)]
pub struct SsgsggSearchParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fbrq_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fbrq_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub market: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zsx_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

fn build_ssgsgg_search_body(params: &SsgsggSearchParams) -> Result<Value, YuandianError> {
    serde_json::to_value(params).map_err(|error| YuandianError::Json(error.to_string()))
}

/// 上市公司公告关键词检索 — 尽调时查目标公司的公开披露。
pub async fn search_ssgsgg_with_params(
    api_key: &YuandianCredentialSource,
    params: &SsgsggSearchParams,
) -> Result<Value, YuandianError> {
    let body = build_ssgsgg_search_body(params)?;
    yd_post(api_key, "/rh_ssgsgg_search", &body).await
}

/* ============ 案例 / 文书检索(对自然人有用)============ */

#[derive(Serialize, Default, Debug, Clone)]
pub struct PtalSearchParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ah: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssqy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ay: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jbdw: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xzqh_p: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wszl: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ajlb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ja_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ja_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fxgc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yyft: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ft_search_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

fn build_ptal_search_body(params: &PtalSearchParams) -> Result<Value, YuandianError> {
    serde_json::to_value(params).map_err(|error| YuandianError::Json(error.to_string()))
}

/// 普通案例库关键词检索 — 给自然人被执行人查涉诉文书。
pub async fn search_ptal_with_params(
    api_key: &YuandianCredentialSource,
    params: &PtalSearchParams,
) -> Result<Value, YuandianError> {
    let body = build_ptal_search_body(params)?;
    yd_post(api_key, "/rh_ptal_search", &body).await
}

/// 兼容旧调用；高级工具路径使用 `search_ptal_with_params`。
pub async fn search_ptal(
    api_key: &YuandianCredentialSource,
    keyword: &str,
    top_k: u32,
) -> Result<Value, YuandianError> {
    let body = serde_json::json!({
        "qw": keyword,
        "top_k": top_k,
    });
    yd_post(api_key, "/rh_ptal_search", &body).await
}

#[derive(Serialize, Default, Debug, Clone)]
pub struct QwalSearchParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ah: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ay: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jbdw: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xzqh_p: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wszl: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ajlb: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ja_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ja_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qw: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

fn build_qwal_search_body(params: &QwalSearchParams) -> Result<Value, YuandianError> {
    serde_json::to_value(params).map_err(|error| YuandianError::Json(error.to_string()))
}

pub async fn search_qwal_with_params(
    api_key: &YuandianCredentialSource,
    params: &QwalSearchParams,
) -> Result<Value, YuandianError> {
    let body = build_qwal_search_body(params)?;
    yd_post(api_key, "/rh_qwal_search", &body).await
}

/// 权威案例库检索(指导性 / 典型 / 公报案例)
pub async fn search_qwal(
    api_key: &YuandianCredentialSource,
    keyword: &str,
    top_k: u32,
) -> Result<Value, YuandianError> {
    let body = serde_json::json!({
        "qw": keyword,
        "top_k": top_k,
    });
    yd_post(api_key, "/rh_qwal_search", &body).await
}

/// 案例详情(GET)。官方接口支持按案例库类型 + 案号/ID 查询，返回 `data` 列表（最多 10 条）。
pub async fn case_details(
    api_key: &YuandianCredentialSource,
    case_type: Option<&str>,
    id: Option<&str>,
    case_no: Option<&str>,
) -> Result<Value, YuandianError> {
    let mut params = Vec::new();
    if let Some(case_type) = case_type.filter(|value| !value.trim().is_empty()) {
        params.push(("type", case_type.to_string()));
    }
    if let Some(id) = id.filter(|s| !s.trim().is_empty()) {
        params.push(("id", id.to_string()));
    }
    if let Some(case_no) = case_no.filter(|s| !s.trim().is_empty()) {
        params.push(("ah", case_no.to_string()));
    }
    yd_get(api_key, "/rh_case_details", &params).await
}

/* ============ EntityId(企业有 id / 统一信用代码两种方式查)============ */

#[derive(Debug, Clone)]
pub enum EntityId {
    Id(String),
    Uscc(String),
}

impl EntityId {
    pub fn to_params(&self) -> Vec<(&'static str, String)> {
        match self {
            EntityId::Id(id) => vec![("id", id.clone())],
            EntityId::Uscc(u) => vec![("tyshxydm", u.clone())],
        }
    }
}

/* ============ V0.2 新增 · 法规/法条/案例语义/幻觉校验/详细工商(8 个,详 § 17) ============
 *
 * 共 7 POST + 1 GET。POST 用 Params struct(Default + Serialize + skip_none),
 * 让上层 chat tool 局部填字段、未填的字段不进 JSON body。
 *
 * 命名遵循 routeKey:rh_ft_search → ft_search,case_vector_search 保持原名。
 */

/// § 17.1 · rh_ft_search 法条关键词检索
#[derive(Serialize, Default, Debug, Clone)]
pub struct FtSearchParams {
    pub keyword: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fgmc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish_date_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish_date_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implement_date_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implement_date_end: Option<String>,
}

pub async fn ft_search(
    api_key: &YuandianCredentialSource,
    params: &FtSearchParams,
) -> Result<Value, YuandianError> {
    let body = build_law_keyword_body(params)?;
    yd_post(api_key, "/rh_ft_search", &body).await
}

/// § 17.2 · rh_ft_detail 法条详情。`id` 跟 `(fgmc, ftnum)` 二选一必填(上层校验)。
#[derive(Serialize, Default, Debug, Clone)]
pub struct FtDetailParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fgmc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ftnum: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refer_date: Option<String>,
}

pub async fn ft_detail(
    api_key: &YuandianCredentialSource,
    params: &FtDetailParams,
) -> Result<Value, YuandianError> {
    let body = serde_json::to_value(params).map_err(|e| YuandianError::Json(e.to_string()))?;
    yd_post(api_key, "/rh_ft_detail", &body).await
}

/// § 17.3 · rh_fg_search 法规检索
#[derive(Serialize, Default, Debug, Clone)]
pub struct FgSearchParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fgmc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish_date_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publish_date_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implement_date_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implement_date_end: Option<String>,
}

pub async fn fg_search(
    api_key: &YuandianCredentialSource,
    params: &FgSearchParams,
) -> Result<Value, YuandianError> {
    let body = build_regulation_keyword_body(params)?;
    yd_post(api_key, "/rh_fg_search", &body).await
}

/// § 17.4 · rh_fg_detail 法规详情。`id` 跟 `fgmc` 二选一必填(上层校验)。
#[derive(Serialize, Default, Debug, Clone)]
pub struct FgDetailParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fgmc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refer_date: Option<String>,
}

pub async fn fg_detail(
    api_key: &YuandianCredentialSource,
    params: &FgDetailParams,
) -> Result<Value, YuandianError> {
    let body = serde_json::to_value(params).map_err(|e| YuandianError::Json(e.to_string()))?;
    yd_post(api_key, "/rh_fg_detail", &body).await
}

/// § 17.5 · law_vector_search 法条语义检索
#[derive(Serialize, Default, Debug, Clone)]
pub struct LawVectorSearchParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewrite_flag: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect_levels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_num: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effect_level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implement_date_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub implement_date_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

pub async fn law_vector_search(
    api_key: &YuandianCredentialSource,
    params: &LawVectorSearchParams,
) -> Result<Value, YuandianError> {
    let body = build_law_vector_body(params);
    yd_post(api_key, "/law_vector_search", &body).await
}

/// § 17.6 · case_vector_search 案例语义检索
#[derive(Serialize, Default, Debug, Clone)]
pub struct WenshuFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wenshu_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ay: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wszl: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ja_start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ja_end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dianxing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fayuan: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cj: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xzqh_p: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xzqh_c: Option<String>,
}

#[derive(Serialize, Default, Debug, Clone)]
pub struct CaseVectorSearchParams {
    pub query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rewrite_flag: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wenshu_filter: Option<WenshuFilter>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub return_num: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
}

pub async fn case_vector_search(
    api_key: &YuandianCredentialSource,
    params: &CaseVectorSearchParams,
) -> Result<Value, YuandianError> {
    let body = build_case_vector_body(params);
    yd_post(api_key, "/case_vector_search", &body).await
}

fn build_law_keyword_body(params: &FtSearchParams) -> Result<Value, YuandianError> {
    let mut body = serde_json::to_value(params).map_err(|e| YuandianError::Json(e.to_string()))?;
    normalize_keyword_law_fields(&mut body);
    Ok(body)
}

fn build_regulation_keyword_body(params: &FgSearchParams) -> Result<Value, YuandianError> {
    let mut body = serde_json::to_value(params).map_err(|e| YuandianError::Json(e.to_string()))?;
    normalize_keyword_law_fields(&mut body);
    Ok(body)
}

/// 把 CaseBoard 内部可读字段名转成元典 2026-07 官方 wire 字段。
fn normalize_keyword_law_fields(body: &mut Value) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    for (from, to) in [
        ("effect_level", "xljb_1"),
        ("region", "dy"),
        ("validity", "sxx"),
        ("publisher", "fbbm"),
        ("publish_date_start", "fbrq_start"),
        ("publish_date_end", "fbrq_end"),
        ("implement_date_start", "ssrq_start"),
        ("implement_date_end", "ssrq_end"),
    ] {
        if let Some(mut value) = obj.remove(from) {
            if to == "dy" {
                if let Some(region) = value.as_str() {
                    value = Value::String(normalize_region(region));
                }
            }
            obj.insert(to.to_string(), value);
        }
    }
    if obj.get("sxx").is_none() {
        if let Some(valid_only) = obj.remove("valid_only").and_then(|v| v.as_bool()) {
            if valid_only {
                obj.insert("sxx".into(), Value::String("现行有效".into()));
            }
        }
    } else {
        obj.remove("valid_only");
    }
    if let Some(search_mode) = obj.get_mut("search_mode") {
        if let Some(mode) = search_mode.as_str() {
            *search_mode = Value::String(mode.to_ascii_uppercase());
        }
    }
}

fn build_law_vector_body(params: &LawVectorSearchParams) -> Value {
    let mut filter = serde_json::Map::new();
    if let Some(effects) = params
        .effect_levels
        .as_ref()
        .filter(|values| !values.is_empty())
    {
        filter.insert("effect1".into(), json!(effects));
    } else if let Some(effect) = params.effect_level.as_deref() {
        filter.insert("effect1".into(), json!([effect]));
    }
    if let Some(validities) = params
        .validities
        .as_ref()
        .filter(|values| !values.is_empty())
    {
        filter.insert("sxx".into(), json!(validities));
    } else if params.valid_only == Some(true) {
        filter.insert("sxx".into(), json!(["现行有效"]));
    }
    if let Some(start) = params.implement_date_start.as_deref() {
        filter.insert("law_start".into(), json!(start));
    }
    if let Some(end) = params.implement_date_end.as_deref() {
        filter.insert("law_end".into(), json!(end));
    }
    let mut body = json!({
        "query": params.query,
        "rewrite_flag": params.rewrite_flag.unwrap_or(true),
        "return_num": params.return_num.or(params.top_k).unwrap_or(45),
    });
    if !filter.is_empty() {
        body["fatiao_filter"] = Value::Object(filter);
    }
    body
}

fn build_case_vector_body(params: &CaseVectorSearchParams) -> Value {
    let mut body = json!({
        "query": params.query,
        "rewrite_flag": params.rewrite_flag.unwrap_or(true),
        "return_num": params.return_num.or(params.top_k).unwrap_or(45),
    });
    let Some(filter) = params.wenshu_filter.as_ref() else {
        return body;
    };
    if let Ok(filter) = serde_json::to_value(filter) {
        if filter.as_object().is_some_and(|object| !object.is_empty()) {
            body["wenshu_filter"] = filter;
        }
    }
    body
}

fn normalize_region(region: &str) -> String {
    region.trim().trim_end_matches(['省', '市']).to_string()
}

/// § 17.7 · hall_detect 法律幻觉校验(核心)。把 LLM final answer 塞进 text,
/// 拿 citations 列表(每条带 verdict:一致/不一致/未命中 + 正确写法)。
pub async fn hall_detect(
    api_key: &YuandianCredentialSource,
    text: &str,
) -> Result<Value, YuandianError> {
    let body = serde_json::json!({ "text": text });
    yd_post(api_key, "/hall_detect", &body).await
}

/// § 17.8 · rh_enterpriseBaseInfo 详细工商(GET)。返回 basic / partner /
/// top10holder / top10circulate / members / branches。
pub async fn enterprise_base_info(
    api_key: &YuandianCredentialSource,
    id_or_uscc: &EntityId,
) -> Result<Value, YuandianError> {
    yd_get(api_key, "/rh_enterpriseBaseInfo", &id_or_uscc.to_params()).await
}

/* ============ tests ============ */
