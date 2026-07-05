use serde::Deserialize;

use super::capability::{LlmProviderKind, ProviderCapability};
use super::gateway::{complete_non_stream_chat, LlmChatMessage, NonStreamChatRequest};
use super::{LlmConfig, LlmError};

#[derive(Debug, Deserialize)]
struct OrganizedLog {
    #[serde(default)]
    title: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    progress: Vec<String>,
    #[serde(default)]
    todos: Vec<String>,
    #[serde(default)]
    deadlines: Vec<String>,
    #[serde(default)]
    risks: Vec<String>,
}

fn render_list(title: &str, items: &[String], out: &mut String) {
    let clean: Vec<&str> = items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .collect();
    if clean.is_empty() {
        return;
    }
    out.push_str(&format!("\n## {title}\n\n"));
    for item in clean {
        out.push_str(&format!("- {item}\n"));
    }
}

fn render(value: OrganizedLog) -> String {
    let title = if value.title.trim().is_empty() {
        "工作记录"
    } else {
        value.title.trim()
    };
    let mut out = format!("# {title}\n");
    if !value.kind.trim().is_empty() {
        out.push_str(&format!("\n- 事项类型：{}\n", value.kind.trim()));
    }
    if !value.summary.trim().is_empty() {
        out.push_str(&format!("\n## 情况摘要\n\n{}\n", value.summary.trim()));
    }
    render_list("最新进展", &value.progress, &mut out);
    render_list("待办建议（需律师确认）", &value.todos, &mut out);
    render_list("期限提示（需律师确认）", &value.deadlines, &mut out);
    render_list("风险与待核实事项", &value.risks, &mut out);
    out
}

pub async fn organize(
    config: &LlmConfig,
    case_context: &str,
    raw_input: &str,
) -> Result<String, LlmError> {
    let prompt = format!(
        "案件概况:\n{case_context}\n\n律师口述原文:\n{raw_input}\n\n\
         请把口述整理为工作记录。只整理，不补造事实；不确定内容放 risks。\
         todos/deadlines 只是建议，不得声称已经写入系统。严格输出 JSON:\n\
         {{\"title\":\"\",\"kind\":\"\",\"summary\":\"\",\"progress\":[],\"todos\":[],\"deadlines\":[],\"risks\":[]}}"
    );
    let capability = ProviderCapability::from_backend("", &config.endpoint, &config.model);
    let max_output_tokens = if capability.kind == LlmProviderKind::MiniMaxNative {
        8192
    } else {
        4096
    };
    let output = complete_non_stream_chat(
        config,
        &capability,
        NonStreamChatRequest {
            messages: vec![
                LlmChatMessage::system(
                    "你是律师案件工作记录整理助手。忠实保留原意，区分事实、建议和待核实事项。",
                ),
                LlmChatMessage::user(prompt),
            ],
            max_output_tokens,
            temperature: config.temperature,
            timeout_secs: Some(config.timeout_secs * 2),
            response_format_json_object: true,
        },
    )
    .await
    .map_err(super::gateway_error_to_llm_error)?;
    let cleaned = super::extract_json_from_content(&output.content);
    let value: OrganizedLog = serde_json::from_str(&cleaned)
        .map_err(|e| LlmError::ContentJson(format!("{e}; raw={cleaned}")))?;
    Ok(render(value))
}
