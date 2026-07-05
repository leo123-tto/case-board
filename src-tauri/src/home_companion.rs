use chrono::Local;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::llm::capability::ProviderCapability;
use crate::llm::gateway::{complete_non_stream_chat, LlmChatMessage, NonStreamChatRequest};
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
    let capability = ProviderCapability::from_settings(settings, &config);
    let request = NonStreamChatRequest {
        messages: vec![LlmChatMessage::user(prompt)],
        max_output_tokens: 96,
        temperature: 0.6,
        timeout_secs: Some(config.timeout_secs.clamp(6, 18)),
        response_format_json_object: false,
    };

    let output = complete_non_stream_chat(&config, &capability, request)
        .await
        .map_err(|e| format!("LLM 问候请求失败: {e}"))?;
    let cleaned = safe_home_greeting(&output.content, input);
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
    let case_load = case_load_context(input.active_case_count);
    let assistant_mode =
        clean_optional(input.assistant_mode.as_deref()).unwrap_or_else(|| "日常值守".into());
    let reminder_context = reminder_context(&input.reminder_summaries);
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
         人设: 克制、可靠、熟悉律师办案节奏,像老同事一样提供一点情绪价值,不撒娇,不说教。\n\
         目标: 主要做日常关心、作息提醒或轻微鼓励;案件提醒只占很小比例,因为正式提醒由日历和重要日期承担。\n\
         用户称呼: {display_name}\n\
         日期: {}\n\
         时间段: {time_of_day}\n\
         当前状态: {assistant_mode}\n\
         天气: {weather}\n\
         案件概况: {case_load}\n\
         提醒状态: {reminder_context}\n\
         可参考的全局记忆:\n{memory_text}\n\n\
         严格要求:\n\
         1. 只输出一句中文,不要解释,不要引号,不要列表。\n\
         2. 30 字以内优先,最多 46 个汉字。\n\
         3. 不要说“作为 AI”。\n\
         4. 不要提任何案号、当事人、文件路径或具体案件事实。\n\
         5. 提醒只作为背景,不得说具体日程、庭审、开庭、待办、截止、到期、案件数量或“几场/几件/几条”。\n\
         6. 必须匹配时间段:上午不要说“今天辛苦了”“早点休息”“明天状态会更好”;夜间优先提醒早点休息。\n\
         7. 天气只作事实参考;天气未更新、不可信或没有明确降雨/降温/高温时,不要提天气、带伞、添衣或路况。\n\
         8. 如果要提提醒,只能泛泛说“提醒区扫一眼”,不要判断今天有没有开庭。\n\
         9. 不要鸡汤味太重,要克制、稳、轻。",
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
        "早上" | "上午" => format!("{name},早上开局不错,今天也稳稳推进。"),
        "中午" => format!("{name},中午缓一口气,下午继续稳住。"),
        "晚上" => format!("{name},晚上收个尾,差不多就早点休息。"),
        "夜间" => format!("{name},时间不早了,先把自己照顾好。"),
        _ => format!("{name},今天稳一点,不用一下子全扛完。"),
    }
}

pub(crate) fn safe_home_greeting(raw: &str, input: &HomeGreetingInput) -> String {
    let cleaned = clean_greeting(raw);
    if cleaned.is_empty()
        || is_unsafe_schedule_claim(&cleaned)
        || is_inconsistent_time_claim(&cleaned, input.time_of_day.as_deref())
        || is_unsupported_weather_claim(&cleaned, input.weather_summary.as_deref())
    {
        fallback_greeting(input.display_name.as_deref(), input.time_of_day.as_deref())
    } else {
        cleaned
    }
}

fn is_unsafe_schedule_claim(text: &str) -> bool {
    if text.contains("庭审")
        || text.contains("开庭")
        || text.contains("传票")
        || text.contains("法庭")
    {
        return true;
    }
    if text.contains("待办") || text.contains("截止") || text.contains("到期") {
        return true;
    }
    let schedule_count = regex::Regex::new(
        r"([0-9一二两三四五六七八九十]+)\s*(场|件|个|条).{0,8}(提醒|日程|待办|案件|庭审|开庭)",
    )
    .expect("valid home greeting schedule regex");
    schedule_count.is_match(text)
}

fn is_inconsistent_time_claim(text: &str, time_of_day: Option<&str>) -> bool {
    let period = clean_optional(time_of_day).unwrap_or_default();
    match period.as_str() {
        "早上" | "上午" => {
            text.contains("早点休息")
                || text.contains("早些休息")
                || text.contains("晚安")
                || text.contains("时间不早")
                || text.contains("明天状态")
                || text.contains("今天辛苦")
                || text.contains("辛苦了")
        }
        "夜间" | "晚上" => {
            text.contains("早上")
                || text.contains("上午")
                || text.contains("开局")
                || text.contains("新的一天")
        }
        _ => false,
    }
}

fn is_unsupported_weather_claim(text: &str, weather_summary: Option<&str>) -> bool {
    if has_trusted_weather_summary(weather_summary) {
        return false;
    }
    [
        "天气", "下雨", "雨", "带伞", "伞", "添衣", "降温", "升温", "高温", "低温", "冷", "热",
        "路上", "出门",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn has_trusted_weather_summary(weather_summary: Option<&str>) -> bool {
    let Some(summary) = clean_optional(weather_summary) else {
        return false;
    };
    !summary.contains("天气未更新")
        && !summary.contains("定位未确认")
        && !summary.contains("网络定位")
}

fn reminder_context(items: &[String]) -> &'static str {
    if items.iter().any(|item| item.contains("逾期")) {
        "有逾期或高优先级提醒,但提醒只作为背景,不要展开类型和数量。"
    } else if items.iter().any(|item| item.contains("紧急")) {
        "有高优先级提醒,但提醒只作为背景,不要展开类型和数量。"
    } else if items.iter().any(|item| !item.trim().is_empty()) {
        "有需要留意的事项,但提醒只作为背景,不要展开类型和数量。"
    } else {
        "暂无重要提醒。"
    }
}

fn case_load_context(count: Option<usize>) -> &'static str {
    match count {
        Some(0) => "当前没有在办案件。",
        Some(1..=7) => "有一些在办案件,但不要输出具体数量。",
        Some(_) => "在办案件较多,但不要输出具体数量。",
        None => "案件数量未更新。",
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
