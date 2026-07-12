use serde::{Deserialize, Serialize};

use crate::llm::capability::ProviderCapability;
use crate::llm::gateway::{complete_non_stream_chat, LlmChatMessage, NonStreamChatRequest};
use crate::llm::LlmConfig;

const MAX_HISTORY_MESSAGES: usize = 8;
const MAX_MESSAGE_CHARS: usize = 1_200;
const MAX_REPLY_CHARS: usize = 600;
const MAX_FEEDBACK_DRAFT_CHARS: usize = 1_500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardAssistantMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DashboardAssistantInput {
    pub messages: Vec<DashboardAssistantMessage>,
    pub active_case_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardAssistantResponse {
    pub reply: String,
    pub action: String,
    pub feedback_draft: Option<String>,
    pub source: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelAnswer {
    reply: String,
    #[serde(default)]
    action: String,
    #[serde(default)]
    feedback_draft: Option<String>,
}

pub async fn chat_dashboard_assistant(
    input: DashboardAssistantInput,
) -> DashboardAssistantResponse {
    let messages = sanitize_messages(input.messages);
    let fallback = fallback_answer(&messages);
    if messages.is_empty() {
        return fallback;
    }

    let settings = crate::settings::read_settings().unwrap_or_default();
    let config = LlmConfig::from_settings(&settings);
    if config.endpoint.trim().is_empty() {
        return with_error(fallback, "LLM endpoint 未配置");
    }
    if is_remote_endpoint(&config.endpoint)
        && config
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .is_none()
    {
        return with_error(fallback, "云端 LLM API Key 未配置");
    }

    let capability = ProviderCapability::from_settings(&settings, &config);
    let mut request_messages = vec![LlmChatMessage::system(system_prompt(
        input.active_case_count,
    ))];
    request_messages.extend(messages.iter().map(|message| LlmChatMessage {
        role: message.role.clone(),
        content: message.content.clone(),
    }));

    let request = NonStreamChatRequest {
        messages: request_messages,
        max_output_tokens: 520,
        temperature: config.temperature.max(0.3),
        timeout_secs: Some(config.timeout_secs.clamp(8, 40)),
        response_format_json_object: true,
    };

    match complete_non_stream_chat(&config, &capability, request).await {
        Ok(output) => match parse_model_answer(&output.content) {
            Some(answer) => response_from_model(answer),
            None => with_error(fallback, "看板助手返回格式无法解析"),
        },
        Err(error) => with_error(fallback, &format!("看板助手请求失败: {error}")),
    }
}

fn system_prompt(active_case_count: usize) -> String {
    format!(
        "你是 macOS/Windows 桌面软件“案件看板 · CaseBoard”的看板助手。你只负责轻量聊天、产品功能介绍、使用指导、设置排查和反馈草稿整理，不是案件法律分析助手。\n\n\
         当前在办案件数: {active_case_count}。这只是汇总数字，不得猜测任何案件事实。\n\n\
         产品事实:\n\
         - 首页汇总民事、刑事和行政等全部在办案件；刑事案件另有独立标签页。\n\
         - 导入案件文件夹后，程序只记录路径，不移动、不改名原文件；随后完成材料分类、OCR、字段抽取和全案画像。\n\
         - 案件详情可查看材料、状态、提醒、案件报告、待办和工作记录，并可打开案件内 AI 助手。\n\
         - 案件内 AI 助手用于基于本案材料做分析、检索依据和起草材料；看板助手不要代替它分析具体案件。\n\
         - 执行管理可做被执行人查询、风险深挖、回款和利息管理；法律工具包含利息计算、法院短信、辅助立案、合同工具等，实际入口受功能开关和配置影响。\n\
         - 设置页可配置 LLM、OCR、元典、用户称呼和界面功能开关。\n\
         - 反馈窗口会自动附带脱敏诊断信息；你可以整理反馈草稿并建议打开窗口，但不得代替用户上传或发送。\n\n\
         回答规则:\n\
         1. 默认用简洁自然的中文，先给结论，再给最多 3 步操作。简单闲聊可以正常回应。\n\
         2. 不编造不存在的按钮、设置、自动化能力或当前状态；不确定时明确说需要在设置或对应页面核对。\n\
         3. 用户贴入具体案情时，提醒其进入对应案件的 AI 助手，并避免复述敏感事实。\n\
         4. 用户想反馈、报告问题、提建议，或要求润色反馈时，把 action 设为 open_feedback，并生成可直接放入反馈框的 feedback_draft。草稿应包含问题/建议、复现或背景、期望结果；缺失信息留“待补充”，不要虚构。\n\
         5. 其他场景 action 必须是 none，feedback_draft 必须是 null。\n\n\
         只输出一个 JSON 对象，不要 Markdown 围栏:\n\
         {{\"reply\":\"给用户的回复\",\"action\":\"none 或 open_feedback\",\"feedback_draft\":null 或 \"反馈草稿\"}}"
    )
}

fn sanitize_messages(messages: Vec<DashboardAssistantMessage>) -> Vec<DashboardAssistantMessage> {
    messages
        .into_iter()
        .rev()
        .filter_map(|message| {
            let role = match message.role.trim() {
                "user" => "user",
                "assistant" => "assistant",
                _ => return None,
            };
            let content = trim_chars(message.content.trim(), MAX_MESSAGE_CHARS);
            if content.is_empty() {
                None
            } else {
                Some(DashboardAssistantMessage {
                    role: role.to_string(),
                    content,
                })
            }
        })
        .take(MAX_HISTORY_MESSAGES)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn parse_model_answer(raw: &str) -> Option<ModelAnswer> {
    let cleaned = extract_json_object(raw);
    serde_json::from_str::<ModelAnswer>(&cleaned).ok()
}

fn response_from_model(answer: ModelAnswer) -> DashboardAssistantResponse {
    let reply = trim_chars(answer.reply.trim(), MAX_REPLY_CHARS);
    if reply.is_empty() {
        return DashboardAssistantResponse {
            reply: "这次没有生成有效回复。你可以换个说法再问我。".into(),
            action: "none".into(),
            feedback_draft: None,
            source: "fallback".into(),
            error: Some("模型回复为空".into()),
        };
    }

    let action = if answer.action.trim() == "open_feedback" {
        "open_feedback"
    } else {
        "none"
    };
    let feedback_draft = if action == "open_feedback" {
        answer
            .feedback_draft
            .map(|draft| trim_chars(draft.trim(), MAX_FEEDBACK_DRAFT_CHARS))
            .filter(|draft| !draft.is_empty())
    } else {
        None
    };

    DashboardAssistantResponse {
        reply,
        action: action.into(),
        feedback_draft,
        source: "ai".into(),
        error: None,
    }
}

fn fallback_answer(messages: &[DashboardAssistantMessage]) -> DashboardAssistantResponse {
    let user_text = messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.content.trim())
        .unwrap_or("");

    if contains_any(
        user_text,
        &[
            "反馈",
            "建议",
            "bug",
            "报错",
            "不好用",
            "润色反馈",
            "整理反馈",
            "提意见",
            "遇到问题",
            "出了问题",
            "有个问题",
            "故障",
            "闪退",
            "白屏",
        ],
    ) {
        let detail = user_text.trim_matches(|c: char| c.is_whitespace() || matches!(c, '：' | ':'));
        let draft = format!(
            "【问题或建议】\n{}\n\n【复现步骤或使用背景】\n待补充\n\n【期望结果】\n待补充",
            if detail.is_empty() {
                "待补充"
            } else {
                detail
            }
        );
        return DashboardAssistantResponse {
            reply: "可以。我先整理成一份可编辑的反馈草稿，点下方按钮就会打开反馈窗口；提交前你仍可修改。".into(),
            action: "open_feedback".into(),
            feedback_draft: Some(trim_chars(&draft, MAX_FEEDBACK_DRAFT_CHARS)),
            source: "fallback".into(),
            error: None,
        };
    }

    let reply = if contains_any(user_text, &["导入", "添加案件", "案件文件夹"]) {
        "在“在办案件”标题右侧点“导入案件”，选择案件文件夹即可。程序只记录原文件路径，不会移动、复制或重命名原文件。"
    } else if contains_any(
        user_text,
        &["设置", "模型", "deepseek", "ocr", "mineru", "元典", "key"],
    ) {
        "请打开顶部“设置”，按需要进入模型、OCR、元典或界面开关。填完密钥后先做连接验证；如果仍失败，可以让我整理反馈并附上脱敏诊断。"
    } else if contains_any(user_text, &["刑事", "行政", "在办案件", "为什么不显示"]) {
        "首页“在办案件”会汇总民事、刑事和行政等全部未结案件；刑事案件同时保留独立标签页，便于使用刑事专属视图。"
    } else if contains_any(user_text, &["功能", "能做什么", "介绍", "怎么用"]) {
        "案件看板可以导入案件文件夹，完成材料分类、OCR、案件画像、提醒和报告；案件内 AI 助手可基于本案材料分析和起草，执行管理与法律工具则处理查询、回款、利息和常用办案任务。"
    } else if contains_any(user_text, &["你好", "晚上好", "早上好", "谢谢", "辛苦"]) {
        "你好，我在。可以问我软件怎么用、功能在哪里、设置怎么配，也可以让我先帮你整理一段反馈。"
    } else {
        "我可以回答案件看板的功能、使用和设置问题，也能陪你简单聊几句，或把问题整理成反馈草稿。具体案件分析请进入对应案件的 AI 助手。"
    };

    DashboardAssistantResponse {
        reply: reply.into(),
        action: "none".into(),
        feedback_draft: None,
        source: "fallback".into(),
        error: None,
    }
}

fn with_error(mut response: DashboardAssistantResponse, error: &str) -> DashboardAssistantResponse {
    response.error = Some(trim_chars(error, 300));
    response
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    needles
        .iter()
        .any(|needle| lower.contains(&needle.to_ascii_lowercase()))
}

fn is_remote_endpoint(endpoint: &str) -> bool {
    let endpoint = endpoint.to_ascii_lowercase();
    !endpoint.contains("127.0.0.1") && !endpoint.contains("localhost")
}

fn trim_chars(text: &str, max_chars: usize) -> String {
    let mut value = text.chars().take(max_chars).collect::<String>();
    if text.chars().count() > max_chars {
        value.push('…');
    }
    value
}

fn extract_json_object(raw: &str) -> String {
    let mut text = raw.trim();
    if let Some(end) = text.find("</think>") {
        text = text[end + "</think>".len()..].trim();
    }
    if let Some(stripped) = text.strip_prefix("```json") {
        text = stripped.trim();
    } else if let Some(stripped) = text.strip_prefix("```") {
        text = stripped.trim();
    }
    if let Some(stripped) = text.strip_suffix("```") {
        text = stripped.trim();
    }
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        if end > start {
            return text[start..=end].to_string();
        }
    }
    text.to_string()
}
