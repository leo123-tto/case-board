use serde::{Deserialize, Serialize};

use crate::llm::capability::ProviderCapability;
use crate::llm::gateway::{complete_non_stream_chat, LlmChatMessage, NonStreamChatRequest};
use crate::llm::LlmConfig;

const PRODUCT_MANUAL: &str = include_str!("../../CASEBOARD_PRODUCT_MANUAL.md");
const MAX_HISTORY_MESSAGES: usize = 8;
const MAX_MESSAGE_CHARS: usize = 1_200;
const MAX_REPLY_CHARS: usize = 600;
const MAX_FEEDBACK_DRAFT_CHARS: usize = 1_500;

const MANUAL_SECTIONS: &[(&str, &str)] = &[
    ("scope", "产品定位与安全边界"),
    ("navigation", "顶部导航与首页"),
    ("import", "导入、扫描与原文件"),
    ("litigation", "诉讼案件看板"),
    ("case-ai", "案件内 AI 助手"),
    ("criminal", "刑事案件"),
    ("execution", "执行管理"),
    ("transaction", "非诉"),
    ("tools", "法律工具"),
    ("memory-team", "记忆与团队"),
    ("settings", "设置"),
    ("feedback", "反馈、诊断与更新"),
    ("dashboard-data", "看板助手可使用的运行时数据"),
    ("dashboard-assistant", "看板助手回答规则"),
];

const DATA_KEYS: &[(&str, &str)] = &[
    ("total_case_count", "全部案件数"),
    ("open_case_count", "在办案件数"),
    ("closed_case_count", "已结案件数"),
    ("criminal_case_count", "刑事案件数"),
    ("execution_case_count", "执行中案件数"),
    ("status_counts", "工作流状态分布"),
    ("document_count", "材料总数"),
    ("pending_document_count", "待处理材料数"),
    ("processing_document_count", "处理中材料数"),
    ("failed_document_count", "失败材料数"),
    ("open_todo_count", "未完成待办数"),
    ("upcoming_event_count", "提醒总数"),
    ("urgent_reminder_count", "紧急提醒数"),
    ("overdue_reminder_count", "逾期提醒数"),
    ("snapshot_complete", "首页数据完整状态"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardAssistantMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DashboardAssistantInput {
    pub messages: Vec<DashboardAssistantMessage>,
    pub context: DashboardAssistantContext,
}

/// 首页已经按与用户界面相同的状态推断规则算好的安全聚合快照。
/// 不含案件名称、当事人、案号、金额、路径或材料正文。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DashboardAssistantContext {
    pub total_case_count: usize,
    pub open_case_count: usize,
    pub closed_case_count: usize,
    pub criminal_case_count: usize,
    pub execution_case_count: usize,
    pub status_counts: std::collections::BTreeMap<String, usize>,
    pub document_count: usize,
    pub pending_document_count: usize,
    pub processing_document_count: usize,
    pub failed_document_count: usize,
    pub open_todo_count: usize,
    pub upcoming_event_count: usize,
    pub urgent_reminder_count: usize,
    pub overdue_reminder_count: usize,
    pub snapshot_complete: bool,
    pub captured_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardAssistantResponse {
    pub reply: String,
    pub action: String,
    pub feedback_draft: Option<String>,
    pub source: String,
    pub error: Option<String>,
    pub references: Vec<String>,
    pub data_sources: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelAnswer {
    reply: String,
    #[serde(default)]
    action: String,
    #[serde(default)]
    feedback_draft: Option<String>,
    #[serde(default)]
    manual_sections: Vec<String>,
    #[serde(default)]
    data_keys: Vec<String>,
}

pub async fn chat_dashboard_assistant(
    input: DashboardAssistantInput,
) -> DashboardAssistantResponse {
    let messages = sanitize_messages(input.messages);
    let context = sanitize_context(input.context);
    let fallback = fallback_answer(&messages, &context);
    if messages.is_empty() {
        return fallback;
    }
    let latest_user_text = messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.content.as_str())
        .unwrap_or("");
    if should_use_manual_answer(latest_user_text) {
        return fallback;
    }

    let settings = crate::settings::read_settings().unwrap_or_default();
    let config = LlmConfig::from_settings(&settings);
    if config.endpoint.trim().is_empty() {
        return with_error(fallback, "LLM endpoint 未配置");
    }
    if is_remote_endpoint(&config.endpoint) && !config.credential_is_ready().await.unwrap_or(false)
    {
        return with_error(fallback, "云端 LLM API Key 未配置");
    }

    let capability = ProviderCapability::from_settings(&settings, &config);
    let mut request_messages = vec![LlmChatMessage::system(system_prompt(&context))];
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
            Some(answer) => response_from_model(answer, fallback),
            None => with_error(fallback, "看板助手返回格式无法解析"),
        },
        Err(error) => with_error(fallback, &format!("看板助手请求失败: {error}")),
    }
}

fn system_prompt(context: &DashboardAssistantContext) -> String {
    let context_json = serde_json::to_string_pretty(context).unwrap_or_else(|_| "{}".into());
    format!(
        "你是 macOS/Windows 桌面软件“案件看板 · CaseBoard”的看板助手。\n\n\
         你有且只有两类可信信息源：\n\
         A. 下方《CaseBoard 产品功能说明书》，用于回答功能事实、入口和边界；\n\
         B. 下方“运行时首页数据快照”，用于回答当前案件、材料、待办和提醒数量。\n\n\
         严格规则：\n\
         1. 禁止用模型常识补充产品能力。说明书没有确认的功能，必须明确回答“产品说明书中没有确认这项功能”。\n\
         2. 用户没有询问当前状态时，不要主动插入案件数或其他运行时数字。\n\
         3. 使用当前数字时，只能逐字读取快照字段；snapshot_complete=false 时必须提示材料和提醒数据仍可能加载中。\n\
         4. 不读取或猜测案件名称、当事人、案号、金额和正文；具体案件分析引导到对应案件内 AI 助手。\n\
         5. 回答只解决当前问题：先结论，再给最多 3 步；不要罗列无关功能。\n\
         6. 反馈类请求把 action 设为 open_feedback，并生成包含问题、复现/背景、期望结果的草稿；缺失信息写“待补充”。其他场景 action=none、feedback_draft=null。\n\
         7. manual_sections 必须填写实际使用的说明书章节 ID；data_keys 必须填写实际使用的快照字段名。禁止填写不存在的 ID 或字段。普通寒暄至少引用 dashboard-assistant。\n\n\
         《CaseBoard 产品功能说明书》开始\n\
         ----------------\n\
         {PRODUCT_MANUAL}\n\
         ----------------\n\
         《CaseBoard 产品功能说明书》结束\n\n\
         运行时首页数据快照（程序生成，不是用户描述）：\n\
         {context_json}\n\n\
         只输出一个 JSON 对象，不要 Markdown 围栏：\n\
         {{\"reply\":\"给用户的回复\",\"action\":\"none 或 open_feedback\",\"feedback_draft\":null 或 \"反馈草稿\",\"manual_sections\":[\"章节ID\"],\"data_keys\":[\"实际使用的字段名\"]}}"
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

fn sanitize_context(mut context: DashboardAssistantContext) -> DashboardAssistantContext {
    const MAX_SAFE_COUNT: usize = 1_000_000;
    context.total_case_count = context.total_case_count.min(MAX_SAFE_COUNT);
    context.open_case_count = context.open_case_count.min(context.total_case_count);
    context.closed_case_count = context.closed_case_count.min(
        context
            .total_case_count
            .saturating_sub(context.open_case_count),
    );
    context.criminal_case_count = context.criminal_case_count.min(context.total_case_count);
    context.execution_case_count = context.execution_case_count.min(context.total_case_count);
    context.document_count = context.document_count.min(MAX_SAFE_COUNT);
    context.pending_document_count = context.pending_document_count.min(context.document_count);
    context.processing_document_count = context
        .processing_document_count
        .min(context.document_count);
    context.failed_document_count = context.failed_document_count.min(context.document_count);
    context.open_todo_count = context.open_todo_count.min(MAX_SAFE_COUNT);
    context.upcoming_event_count = context.upcoming_event_count.min(MAX_SAFE_COUNT);
    context.urgent_reminder_count = context
        .urgent_reminder_count
        .min(context.upcoming_event_count);
    context.overdue_reminder_count = context
        .overdue_reminder_count
        .min(context.upcoming_event_count);
    context.status_counts = context
        .status_counts
        .into_iter()
        .filter_map(|(label, count)| {
            let label = trim_chars(label.trim(), 20);
            (!label.is_empty()).then_some((label, count.min(context.total_case_count)))
        })
        .take(16)
        .collect();
    context.captured_at = trim_chars(context.captured_at.trim(), 64);
    context
}

fn parse_model_answer(raw: &str) -> Option<ModelAnswer> {
    let cleaned = extract_json_object(raw);
    serde_json::from_str::<ModelAnswer>(&cleaned).ok()
}

fn response_from_model(
    answer: ModelAnswer,
    fallback: DashboardAssistantResponse,
) -> DashboardAssistantResponse {
    let reply = trim_chars(answer.reply.trim(), MAX_REPLY_CHARS);
    if reply.is_empty() {
        return with_error(fallback, "模型回复为空");
    }

    let references = resolve_labels(&answer.manual_sections, MANUAL_SECTIONS);
    if references.is_empty() {
        return with_error(fallback, "模型回复没有有效的产品说明书章节依据");
    }
    let data_sources = resolve_labels(&answer.data_keys, DATA_KEYS);
    if answer
        .data_keys
        .iter()
        .any(|key| !DATA_KEYS.iter().any(|(allowed, _)| key == allowed))
    {
        return with_error(fallback, "模型引用了不存在的首页数据字段");
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
        source: "ai_grounded".into(),
        error: None,
        references,
        data_sources,
    }
}

fn fallback_answer(
    messages: &[DashboardAssistantMessage],
    context: &DashboardAssistantContext,
) -> DashboardAssistantResponse {
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
            source: "manual".into(),
            error: None,
            references: vec!["反馈、诊断与更新".into()],
            data_sources: vec![],
        };
    }

    let asks_count = contains_any(user_text, &["几个", "多少", "数量", "当前", "现在"]);
    let (reply, references, data_sources) = if asks_count
        && contains_any(user_text, &["案件", "案子", "在办", "结案"])
    {
        (
            format!(
                "当前首页显示：在办 {} 件，全部 {} 件，已结 {} 件。其中刑事 {} 件、执行中 {} 件。",
                context.open_case_count,
                context.total_case_count,
                context.closed_case_count,
                context.criminal_case_count,
                context.execution_case_count,
            ),
            vec!["顶部导航与首页", "看板助手可使用的运行时数据"],
            vec![
                "在办案件数",
                "全部案件数",
                "已结案件数",
                "刑事案件数",
                "执行中案件数",
            ],
        )
    } else if asks_count && contains_any(user_text, &["材料", "文件", "ocr", "识别"]) {
        let loading_note = if context.snapshot_complete {
            ""
        } else {
            " 首页材料仍在加载，数字可能继续更新。"
        };
        (
            format!(
                "当前首页已载入 {} 份材料：待处理 {} 份、处理中 {} 份、失败 {} 份。{}",
                context.document_count,
                context.pending_document_count,
                context.processing_document_count,
                context.failed_document_count,
                loading_note,
            ),
            vec!["导入、扫描与原文件", "看板助手可使用的运行时数据"],
            vec![
                "材料总数",
                "待处理材料数",
                "处理中材料数",
                "失败材料数",
                "首页数据完整状态",
            ],
        )
    } else if asks_count && contains_any(user_text, &["待办", "提醒", "开庭", "期限"]) {
        let loading_note = if context.snapshot_complete {
            ""
        } else {
            " 首页材料仍在加载，提醒数字可能继续更新。"
        };
        (
            format!(
                "当前有 {} 条未完成待办、{} 个提醒，其中紧急 {} 个、逾期 {} 个。{}",
                context.open_todo_count,
                context.upcoming_event_count,
                context.urgent_reminder_count,
                context.overdue_reminder_count,
                loading_note,
            ),
            vec!["诉讼案件看板", "看板助手可使用的运行时数据"],
            vec![
                "未完成待办数",
                "提醒总数",
                "紧急提醒数",
                "逾期提醒数",
                "首页数据完整状态",
            ],
        )
    } else if contains_any(
        user_text,
        &["重新关联", "原文件失联", "文件夹移动", "文件夹改名"],
    ) {
        (
            "进入对应案件，在案件详情顶部点击“重新关联案件源文件夹”，重新选择移动或改名后的原文件夹。CaseBoard 会复用已有分析材料，不会移动或改名原文件。".into(),
            vec!["导入、扫描与原文件"],
            vec![],
        )
    } else if contains_any(user_text, &["重新识别", "识别失败", "抽取失败", "ocr失败"])
    {
        (
            "进入案件的原始材料区，先查看失败材料旁的真实错误原因；修复模型、余额、Key 或文件问题后点击“重新识别”。带大幅水印的材料可选择去水印识别路线。".into(),
            vec!["导入、扫描与原文件", "诉讼案件看板"],
            vec![],
        )
    } else if contains_any(user_text, &["重新分析", "全案分析", "案件画像", "案件报告"])
    {
        (
            "如果原始材料已经识别完成，只需在案件材料区使用“重新全案分析”，更新案件画像和报告，不会重跑 OCR；新增或修改了源文件时，先用案件详情顶部的“更新源文件”。".into(),
            vec!["导入、扫描与原文件", "诉讼案件看板"],
            vec![],
        )
    } else if contains_any(
        user_text,
        &["删除案件", "删案件", "删除原文件", "会不会删除"],
    ) {
        (
            "首页或案件详情中的删除操作只移除 CaseBoard 数据库里的看板记录，不删除原始案件文件夹；执行前仍会要求确认。".into(),
            vec!["产品定位与安全边界", "导入、扫描与原文件"],
            vec![],
        )
    } else if contains_any(user_text, &["导入", "添加案件", "案件文件夹"]) {
        (
            "在首页找到“在办案件”，点击标题右侧的“导入案件”，再选择案件文件夹。CaseBoard 只记录路径，不会移动、复制或重命名原文件。".into(),
            vec!["导入、扫描与原文件"],
            vec![],
        )
    } else if contains_any(
        user_text,
        &["设置", "模型", "deepseek", "ocr", "mineru", "元典", "key"],
    ) {
        (
            "打开顶部“设置”：对话模型在“大脑”，OCR 与 Embedding 在“功能模型”，元典和外部 MCP 在“数据源”，首页可选模块在“功能开关”。修改后先保存，再使用页面上的验证或测试按钮。".into(),
            vec!["设置"],
            vec![],
        )
    } else if contains_any(
        user_text,
        &[
            "案件内ai",
            "ai助手",
            "写起诉状",
            "写答辩状",
            "类案",
            "法律依据",
            "深度分析",
        ],
    ) {
        (
            "进入具体案件后使用右侧“AI 助手”。它可以基于该案材料查法律依据、模拟对抗、检索类案、做深度分析，以及起草起诉状、答辩状、证据目录和质证意见；生成成果可编辑和导出，但不是原始证据。".into(),
            vec!["案件内 AI 助手"],
            vec![],
        )
    } else if contains_any(
        user_text,
        &["执行", "被执行人", "回款", "财产线索", "迟延履行"],
    ) {
        (
            "顶部“执行”会汇总执行中案件。执行详情支持查被执行人、风险深挖、完整报告、回款记录，并可把案件数据带入“工具 → 利息 / 执行款计算器”。元典结果只作辅助线索，需核对来源和主体。".into(),
            vec!["执行管理"],
            vec![],
        )
    } else if contains_any(user_text, &["非诉", "合同审查", "合同起草"]) {
        (
            "顶部“非诉”目前提供两个 Beta 工具：合同审查和合同起草。合同审查可生成意见书与修订版 Word；合同起草可结合交易要素、立场和附件生成可修改、可导出的草案。其他非诉模块尚未在说明书中确认。".into(),
            vec!["非诉"],
            vec![],
        )
    } else if contains_any(
        user_text,
        &[
            "法律工具",
            "计算器",
            "快递",
            "辅助立案",
            "法院短信",
            "要素式",
        ],
    ) {
        (
            "打开顶部“工具”。这里有数字大写、天数、律师费、诉讼费、利息/执行款、交通事故赔偿、劳动解除赔偿计算器，以及知识库共享、案件资料包、办案画像、滴答/飞书、法院短信、快递、辅助在线立案和要素式文书转换。".into(),
            vec!["法律工具"],
            vec![],
        )
    } else if contains_any(user_text, &["记忆", "团队", "同步", "案件接力"]) {
        (
            "顶部“记忆”管理会按任务注入 AI 的 Markdown 记忆；顶部“团队”用于局域网团队看板和案件接力。案件资料包与工作区同步处理结构化数据和报告类产物，不应把 SQLite 数据库放进网盘直接双向同步。".into(),
            vec!["记忆与团队"],
            vec![],
        )
    } else if contains_any(user_text, &["导出", "word", "html", "成果", "文书编辑"]) {
        (
            "案件内 AI 生成文书后会进入可编辑的写作模式，可导出 Word 或 HTML。AI 成果是底稿，不属于原始证据；正式提交前应由律师复核事实、法律依据和格式。".into(),
            vec!["案件内 AI 助手", "产品定位与安全边界"],
            vec![],
        )
    } else if contains_any(user_text, &["刑事", "行政", "为什么不显示"]) {
        (
            "首页“在办案件”汇总当前全部未结案件；刑事案件同时进入独立“刑事”标签，使用刑事字段和“刑事深度分析”。行政案件仍在“诉讼”视图中。".into(),
            vec!["顶部导航与首页", "刑事案件", "诉讼案件看板"],
            vec![],
        )
    } else if contains_any(user_text, &["功能", "能做什么", "介绍", "怎么用"]) {
        (
            "CaseBoard 的主线是：导入案件文件夹并只读管理原件；完成材料分类、OCR、字段抽取和案件画像；在案件内维护状态、提醒、待办、报告和工作记录；用案件内 AI 助手做有材料依据的检索、分析与文书起草。此外还有刑事专属视图、执行管理、合同审查/起草、记忆、团队和法律工具。".into(),
            vec!["产品定位与安全边界", "诉讼案件看板", "案件内 AI 助手"],
            vec![],
        )
    } else if contains_any(user_text, &["你好", "晚上好", "早上好", "谢谢", "辛苦"]) {
        (
            "你好，我在。可以问我 CaseBoard 的功能入口、设置方法和当前首页汇总，也可以让我整理反馈。具体案件分析请进入对应案件的 AI 助手。".into(),
            vec!["看板助手回答规则"],
            vec![],
        )
    } else {
        (
            "产品说明书中没有确认这个问题对应的具体功能。你可以告诉我所在页面、想完成的动作或看到的错误；如果涉及某个具体案件，请进入该案件的 AI 助手。".into(),
            vec!["产品定位与安全边界", "看板助手回答规则"],
            vec![],
        )
    };

    DashboardAssistantResponse {
        reply,
        action: "none".into(),
        feedback_draft: None,
        source: "manual".into(),
        error: None,
        references: references.into_iter().map(str::to_string).collect(),
        data_sources: data_sources.into_iter().map(str::to_string).collect(),
    }
}

fn should_use_manual_answer(text: &str) -> bool {
    contains_any(
        text,
        &[
            "案件看板",
            "caseboard",
            "功能",
            "怎么",
            "如何",
            "哪里",
            "入口",
            "支持",
            "设置",
            "导入",
            "案件",
            "案子",
            "材料",
            "文件",
            "ocr",
            "模型",
            "元典",
            "助手",
            "工具",
            "刑事",
            "行政",
            "执行",
            "非诉",
            "合同",
            "团队",
            "记忆",
            "同步",
            "待办",
            "提醒",
            "开庭",
            "反馈",
            "建议",
            "报错",
            "bug",
            "删除",
            "导出",
            "word",
            "你好",
            "早上好",
            "晚上好",
            "谢谢",
        ],
    )
}

fn resolve_labels(values: &[String], allowed: &[(&str, &str)]) -> Vec<String> {
    let mut labels = Vec::new();
    for value in values {
        if let Some((_, label)) = allowed.iter().find(|(id, _)| value == id) {
            if !labels.iter().any(|existing| existing == label) {
                labels.push((*label).to_string());
            }
        }
    }
    labels
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
