use chrono::Local;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::llm::LlmConfig;
use crate::settings::Settings;

const MEMORY_CHAR_BUDGET: usize = 2_400;
const MAX_GREETING_CHARS: usize = 46;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HomeGreetingInput {
    pub display_name: Option<String>,
    pub weather_summary: Option<String>,
    pub active_case_count: Option<usize>,
    pub reminder_summaries: Vec<String>,
    pub assistant_mode: Option<String>,
    pub local_date: String,
    pub time_of_day: Option<String>,
    pub force_refresh: Option<bool>,
}

impl Default for HomeGreetingInput {
    fn default() -> Self {
        Self {
            display_name: None,
            weather_summary: None,
            active_case_count: None,
            reminder_summaries: Vec::new(),
            assistant_mode: None,
            local_date: Local::now().format("%Y-%m-%d").to_string(),
            time_of_day: None,
            force_refresh: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HomeGreetingResponse {
    pub text: String,
    pub source: String,
    pub generated_at: String,
    pub memory_used_count: usize,
    pub error: Option<String>,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

pub async fn generate_home_greeting(
    pool: &SqlitePool,
    input: HomeGreetingInput,
) -> HomeGreetingResponse {
    let settings = crate::settings::read_settings().unwrap_or_default();
    let memories = load_home_memories(pool, &settings).await;
    let fallback = fallback_greeting(input.display_name.as_deref(), input.time_of_day.as_deref());

    match call_llm_for_greeting(&settings, &input, &memories).await {
        Ok(text) => HomeGreetingResponse {
            text,
            source: "ai".into(),
            generated_at: now_local(),
            memory_used_count: memories.len(),
            error: None,
        },
        Err(error) => HomeGreetingResponse {
            text: fallback,
            source: "fallback".into(),
            generated_at: now_local(),
            memory_used_count: memories.len(),
            error: Some(error),
        },
    }
}

async fn load_home_memories(pool: &SqlitePool, settings: &Settings) -> Vec<String> {
    let mut items = Vec::new();

    if let Ok(rows) = crate::db::case_memories::list_active_global_memories(pool).await {
        items.extend(rows.into_iter().map(|m| m.content));
    }

    match crate::memory_vault::build_prompt_pack_for_modes(
        settings,
        &["global_prompt", "cold_start_prompt", "workflow_prompt"],
    ) {
        Ok(pack) => items.extend(pack.items),
        Err(e) => crate::dlog!("[home-companion] 读取全局记忆失败: {}", e),
    }

    cap_memory_items(items, MEMORY_CHAR_BUDGET)
}

async fn call_llm_for_greeting(
    settings: &Settings,
    input: &HomeGreetingInput,
    memories: &[String],
) -> Result<String, String> {
    let config = LlmConfig::from_settings(settings);
    if config.endpoint.trim().is_empty() {
        return Err("LLM endpoint 未配置".into());
    }

    let prompt = build_home_greeting_prompt(input, memories);
    let body = ChatRequest {
        model: config.model.clone(),
        messages: vec![ChatMessage {
            role: "user".into(),
            content: prompt,
        }],
        max_tokens: 96,
        temperature: 0.6,
        stream: false,
    };

    let timeout = std::time::Duration::from_secs(config.timeout_secs.clamp(6, 18));
    let mut req = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("创建 LLM 客户端失败: {e}"))?
        .post(&config.endpoint)
        .json(&body);

    if let Some(key) = config
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        req = req.bearer_auth(key);
    }

    let response = req
        .send()
        .await
        .map_err(|e| format!("LLM 问候请求失败: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("LLM 问候返回 {}: {}", status.as_u16(), text));
    }
    let parsed = response
        .json::<ChatResponse>()
        .await
        .map_err(|e| format!("解析 LLM 问候失败: {e}"))?;
    let raw = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| "LLM 问候 choices 为空".to_string())?;
    let cleaned = clean_greeting(&raw);
    if cleaned.is_empty() {
        Err("LLM 问候为空".into())
    } else {
        Ok(cleaned)
    }
}

pub(crate) fn build_home_greeting_prompt(input: &HomeGreetingInput, memories: &[String]) -> String {
    let display_name =
        clean_optional(input.display_name.as_deref()).unwrap_or_else(|| "律师".into());
    let weather =
        clean_optional(input.weather_summary.as_deref()).unwrap_or_else(|| "天气未更新".into());
    let time_of_day = clean_optional(input.time_of_day.as_deref()).unwrap_or_else(|| "今天".into());
    let active_case_count = input
        .active_case_count
        .map(|n| format!("在办案件 {n} 个"))
        .unwrap_or_else(|| "在办案件数未更新".into());
    let assistant_mode =
        clean_optional(input.assistant_mode.as_deref()).unwrap_or_else(|| "日常值守".into());
    let reminder_text = clean_reminder_summaries(&input.reminder_summaries);
    let memory_text = if memories.is_empty() {
        "无可用全局记忆。".to_string()
    } else {
        memories
            .iter()
            .map(|m| format!("- {}", m.trim()))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "你是案件看板首页的“案件助手”。\n\
         人设: 克制、可靠、熟悉律师办案节奏,像老同事一样轻轻提醒,不撒娇,不说教。\n\
         目标: 结合首页案件概况、重要提醒、天气和全局记忆,给用户一句自然、轻、可执行的首页话术。\n\
         用户称呼: {display_name}\n\
         日期: {}\n\
         时间段: {time_of_day}\n\
         当前状态: {assistant_mode}\n\
         天气: {weather}\n\
         案件概况: {active_case_count}\n\
         重要提醒摘要:\n{reminder_text}\n\
         可参考的全局记忆:\n{memory_text}\n\n\
         严格要求:\n\
         1. 只输出一句中文,不要解释,不要引号,不要列表。\n\
         2. 30 字以内优先,最多 46 个汉字。\n\
         3. 不要说“作为 AI”。\n\
         4. 不要提任何案号、当事人、文件路径或具体案件事实。\n\
         5. 话术要贴合“当前状态”:重点提醒就提醒看紧急项;整理案件就强调抓重点;准备出门就提醒先过一遍;记录事项就强调记清节点;日常值守就轻一点。\n\
         6. 可以提“今天有提醒”“先看紧急项”“在办案件”,但不要虚构数字。\n\
         7. 不要鸡汤味太重,要克制、稳、轻。",
        input.local_date.trim()
    )
}

pub(crate) fn clean_greeting(raw: &str) -> String {
    let mut line = raw
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .trim_matches(['"', '\'', '“', '”', '‘', '’'])
        .to_string();

    for prefix in [
        "作为AI，",
        "作为 AI，",
        "作为AI,",
        "作为 AI,",
        "问候：",
        "问候:",
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            line = rest.trim().to_string();
        }
    }

    line = line
        .replace(['\r', '\n', '\t'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    let count = line.chars().count();
    if count > MAX_GREETING_CHARS {
        let mut truncated: String = line.chars().take(MAX_GREETING_CHARS).collect();
        truncated = truncated
            .trim_end_matches(['，', '、', ',', ' '])
            .to_string();
        if !truncated.ends_with(['。', '！', '～']) {
            truncated.push('。');
        }
        truncated
    } else {
        line
    }
}

pub(crate) fn fallback_greeting(display_name: Option<&str>, time_of_day: Option<&str>) -> String {
    let name = clean_optional(display_name).unwrap_or_else(|| "律师".into());
    let period = clean_optional(time_of_day).unwrap_or_else(|| "今天".into());
    match period.as_str() {
        "早上" | "上午" => format!("{name},早上先看最要紧的一件事。"),
        "晚上" | "夜间" => format!("{name},晚上收个尾,别把自己绷太紧。"),
        _ => format!("{name},今天稳一点,先处理最关键的事。"),
    }
}

fn clean_reminder_summaries(items: &[String]) -> String {
    let lines = items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .take(4)
        .map(|item| {
            let clipped: String = item.chars().take(42).collect();
            format!("- {clipped}")
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        "- 暂无重要提醒".into()
    } else {
        lines.join("\n")
    }
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(24).collect())
}

fn cap_memory_items(items: Vec<String>, budget: usize) -> Vec<String> {
    let mut used = 0usize;
    let mut out = Vec::new();
    for item in items {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let len = item.chars().count() + 1;
        if used + len > budget {
            break;
        }
        used += len;
        out.push(item.to_string());
    }
    out
}

fn now_local() -> String {
    Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}
