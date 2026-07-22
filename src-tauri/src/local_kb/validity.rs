//! 现行法源的统一时效门禁。
//!
//! 元典详情使用 `sxx`，CaseBoard 写回的法规正文使用“时效性：...”元数据。
//! 这里统一识别不能作为现行办案依据的状态，供在线工具、缓存、BM25、向量索引和
//! 精确法条读取共同复用，避免各层各写一套后发生规则漂移。

use std::path::Path;

use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InactiveLegalSourceInfo {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone)]
pub struct GatedLegalResponse {
    pub value: Value,
    pub inactive_sources: Vec<InactiveLegalSourceInfo>,
}

pub fn is_inactive_status(status: &str) -> bool {
    let normalized = status.trim();
    ["失效", "废止", "尚未生效", "未生效", "已被修改"]
        .iter()
        .any(|marker| normalized.contains(marker))
}

pub fn is_inactive_regulation_text(text: &str) -> bool {
    text.lines().any(|line| {
        let line = line.trim();
        let status = line
            .split_once("时效性：")
            .or_else(|| line.split_once("时效性:"))
            .or_else(|| line.split_once("sxx:"))
            .or_else(|| line.split_once("validity:"))
            .map(|(_, value)| value.split('|').next().unwrap_or(value).trim());
        status.is_some_and(is_inactive_status)
    })
}

pub fn is_inactive_regulation_file(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .is_some_and(|text| is_inactive_regulation_text(&text))
}

fn is_legal_query_type(query_type: &str) -> bool {
    matches!(
        query_type,
        "rh_ft_search" | "rh_fg_search" | "law_vector_search" | "rh_fg_detail" | "rh_ft_detail"
    )
}

fn explicitly_requests_non_current(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(status)) => status.trim() != "现行有效",
        Some(Value::Array(statuses)) => statuses.iter().any(|status| {
            status
                .as_str()
                .is_some_and(|status| status.trim() != "现行有效")
        }),
        _ => false,
    }
}

/// 只有明确的历史时点或非现行状态过滤才进入历史研究模式。普通请求、缺省参数和
/// 非法律端点一律保持现行法门禁，避免模型仅凭正文自行“猜测”用户想查旧法。
pub fn historical_research_requested(query_type: &str, params: &Value) -> bool {
    if !is_legal_query_type(query_type) {
        return false;
    }
    let has_refer_date = params
        .get("refer_date")
        .and_then(Value::as_str)
        .is_some_and(|date| !date.trim().is_empty());
    has_refer_date
        || explicitly_requests_non_current(params.get("validity"))
        || explicitly_requests_non_current(params.get("sxx"))
        || explicitly_requests_non_current(params.get("validities"))
}

fn inactive_source_info(value: &Value) -> Option<InactiveLegalSourceInfo> {
    let status = value
        .get("sxx")
        .or_else(|| value.get("validity"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|status| is_inactive_status(status))
        .map(String::from)
        .or_else(|| {
            value
                .get("valid")
                .and_then(Value::as_bool)
                .is_some_and(|valid| !valid)
                .then(|| "失效(valid=false)".to_string())
        })?;
    let name = value
        .get("fgmc")
        .or_else(|| value.get("fgtitle"))
        .or_else(|| value.get("title"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("未命名法规");
    Some(InactiveLegalSourceInfo {
        name: name.to_string(),
        status,
    })
}

fn filter_inactive_items(items: &mut Vec<Value>, removed: &mut Vec<InactiveLegalSourceInfo>) {
    let mut kept = Vec::with_capacity(items.len());
    for item in std::mem::take(items) {
        if let Some(info) = inactive_source_info(&item) {
            removed.push(info);
        } else {
            kept.push(item);
        }
    }
    *items = kept;
}

/// 对元典法律检索/详情响应执行统一时效门禁。失效、废止和尚未生效的正文会在
/// 交给模型、写搜索缓存或入主库之前被移除；调用方可依据 `inactive_sources`
/// 继续检索现行修订版或替代法规。
pub fn sanitize_yuandian_legal_response(query_type: &str, mut value: Value) -> GatedLegalResponse {
    let mut inactive_sources = Vec::new();
    match query_type {
        "rh_ft_search" | "rh_fg_search" => {
            if let Some(items) = value.get_mut("data").and_then(Value::as_array_mut) {
                filter_inactive_items(items, &mut inactive_sources);
            }
        }
        "law_vector_search" => {
            for pointer in ["/extra/fatiao", "/data/extra/fatiao"] {
                if let Some(items) = value.pointer_mut(pointer).and_then(Value::as_array_mut) {
                    filter_inactive_items(items, &mut inactive_sources);
                }
            }
        }
        "rh_fg_detail" | "rh_ft_detail" => {
            if let Some(info) = value.get("data").and_then(inactive_source_info) {
                inactive_sources.push(info);
                if let Some(object) = value.as_object_mut() {
                    object.insert("data".into(), Value::Null);
                }
            }
        }
        _ => {}
    }
    if !inactive_sources.is_empty() {
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "_caseboard_validity_gate".into(),
                json!({
                    "inactive_removed": inactive_sources.len(),
                    "usable_as_authority": false,
                    "content_omitted": true,
                    "note": "失效、废止或尚未生效的法规已被剔除，不得引用、写入报告或知识库；应继续检索现行修订版或替代法规，未找到时如实说明。"
                }),
            );
        }
    }
    GatedLegalResponse {
        value,
        inactive_sources,
    }
}

/// 根据本次真实请求决定是执行现行法硬门禁，还是保留明确要求的历史法源。历史法源
/// 仍保留完整正文以便时点研究，但增加机器可读警告，绝不能冒充现行依据。
pub fn legal_response_for_request(
    query_type: &str,
    params: &Value,
    mut value: Value,
) -> GatedLegalResponse {
    let detected = sanitize_yuandian_legal_response(query_type, value.clone());
    if !historical_research_requested(query_type, params) || detected.inactive_sources.is_empty() {
        return detected;
    }
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "_caseboard_validity_context".into(),
            json!({
                "mode": "historical_research_only",
                "usable_as_current_authority": false,
                "effective_date_must_be_verified": true,
                "refer_date": params.get("refer_date").cloned().unwrap_or(Value::Null),
                "inactive_sources": detected.inactive_sources.iter().map(|source| json!({
                    "name": source.name,
                    "validity": source.status
                })).collect::<Vec<_>>(),
                "note": "用户已明确要求历史时点或非现行状态法源。正文仅供历史适用法研究；引用时必须核对案件行为时点、施行区间和后续修订，不得表述为现行有效依据。"
            }),
        );
    }
    GatedLegalResponse {
        value,
        inactive_sources: detected.inactive_sources,
    }
}
