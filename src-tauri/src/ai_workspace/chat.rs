use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use chrono::Datelike;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::{mpsc, oneshot};

use super::retrieval::{retrieve_workspace_context, RetrievedWorkspaceSource, WorkspaceReference};
use crate::chat::agent_loop::{
    AgentLoopRequest, AskQuestion, ToolCallBudgetConfig, ToolCallRecord,
};
use crate::chat::artifact_intent::ArtifactIntent;
use crate::chat::citations::Citation;
use crate::chat::commands::{ChatCancelRegistry, ChatRunScope};
use crate::chat::context::TaskType;
use crate::chat::loop_guard::LoopGuardConfig;
use crate::chat::model_router::route_model_with_context;
use crate::chat::quality_gate::{
    evaluate_task_quality, format_quality_gate_note, QualityGateInput,
};
use crate::chat::runtime::{
    run_chat_with_runtime, validate_ai_assistant_ready, AgentRuntimeKind, ChatRunControl,
};
use crate::chat::stream::ChatStreamEvent;
use crate::chat::tools::{ToolContext, ToolRegistry};
use crate::db::ai_workspace_chat;
use crate::db::ai_workspace_documents;
use crate::db::ai_workspaces;
use crate::llm::LlmConfig;
use crate::local_kb::cache::LocalKb;

#[derive(Debug, Clone, Deserialize)]
pub struct AiWorkspaceChatInput {
    pub workspace_id: String,
    pub conversation_id: String,
    pub user_message: String,
    pub user_message_id: String,
    pub message_id: String,
    #[serde(default)]
    pub references: Vec<WorkspaceReference>,
    #[serde(default)]
    pub editing_document_id: Option<String>,
    #[serde(default)]
    pub skill_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AiWorkspaceChatResult {
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub task_id: String,
    pub model: String,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub latency_ms: u64,
    pub citations: Vec<Citation>,
    pub sources: Vec<RetrievedWorkspaceSource>,
    pub tool_calls: Vec<ToolCallRecord>,
    pub ask_user: Option<Vec<AskQuestion>>,
    pub artifact_doc_id: Option<String>,
}

fn allows_new_workspace_file_while_editing(user_message: &str) -> bool {
    const NEGATED: &[&str] = &[
        "不要另存",
        "不另存",
        "不是另存",
        "不要新建",
        "不是新建",
        "不要复制",
        "不是复制",
    ];
    if NEGATED.iter().any(|phrase| user_message.contains(phrase)) {
        return false;
    }
    const EXPLICIT_NEW_FILE: &[&str] = &[
        "另存为",
        "另存一份",
        "另起一份",
        "新建一份",
        "创建一份新的",
        "复制一份",
        "保留原稿",
        "保留原版",
        "保留旧版",
        "生成一份新的",
        "再做一份",
        "新存一个",
        "新存一份",
        "重新存一个",
        "重新存一份",
    ];
    EXPLICIT_NEW_FILE
        .iter()
        .any(|phrase| user_message.contains(phrase))
}

fn allows_runtime_managed_workspace_files(runtime: AgentRuntimeKind, user_message: &str) -> bool {
    runtime == AgentRuntimeKind::Pi || allows_new_workspace_file_while_editing(user_message)
}

fn asks_about_previous_tool_use(user_message: &str) -> bool {
    [
        "有没有使用工具",
        "有使用工具",
        "使用工具去联网",
        "是否使用了工具",
        "是否调用了工具",
        "调用工具了吗",
        "用工具了吗",
        "有没有联网",
        "是否联网",
        "真的联网",
        "联网了吗",
        "真的检索",
        "实际检索了吗",
    ]
    .iter()
    .any(|marker| user_message.contains(marker))
}

fn is_network_tool(tool: &str) -> bool {
    crate::chat::policy::is_network_tool(tool)
}

fn format_previous_tool_audit(tool_calls_json: Option<&str>) -> String {
    let Some(tool_calls_json) = tool_calls_json else {
        return "这是 CaseBoard 宿主审计结果，不是大模型自述：没有找到可核验的上一轮任务记录。"
            .into();
    };
    let Ok(tool_calls) = serde_json::from_str::<Vec<ToolCallRecord>>(tool_calls_json) else {
        return "这是 CaseBoard 宿主审计结果，不是大模型自述：上一轮工具记录无法解析，不能确认其声称的操作。"
            .into();
    };
    let successful = tool_calls.iter().filter(|call| call.success).count();
    let network = tool_calls
        .iter()
        .filter(|call| call.success && is_network_tool(&call.tool))
        .count();
    let writes = tool_calls
        .iter()
        .filter(|call| {
            call.success
                && matches!(
                    call.tool.as_str(),
                    "save_artifact"
                        | "edit_artifact"
                        | "create_workspace_file"
                        | "write_workspace_file"
                        | "rename_workspace_file"
                        | "copy_workspace_file"
                )
        })
        .count();
    let mut names = BTreeMap::<&str, usize>::new();
    for call in &tool_calls {
        *names.entry(call.tool.as_str()).or_default() += 1;
    }
    let tool_summary = if names.is_empty() {
        "无".to_string()
    } else {
        names
            .into_iter()
            .map(|(name, count)| format!("`{name}` × {count}"))
            .collect::<Vec<_>>()
            .join("、")
    };
    let conclusion = if network == 0 {
        "结论：上一轮没有发生可核验的联网检索；正文中声称已调用联网工具的内容不可采信。"
    } else {
        "结论：上一轮存在真实联网工具记录。请展开上一轮消息的“处理过程”，逐项查看查询词、网址和结果摘要。"
    };
    format!(
        "这是 CaseBoard 宿主审计结果，不是大模型自述：\n\n- 真实工具调用：{} 次（成功 {} 次）\n- 成功联网调用：{} 次\n- 成功文稿写入：{} 次\n- 工具明细：{}\n\n{}",
        tool_calls.len(), successful, network, writes, tool_summary, conclusion
    )
}

fn format_workspace_sources(sources: &[RetrievedWorkspaceSource]) -> String {
    if sources.is_empty() {
        return "当前没有上传材料或没有检索到相关片段。可以根据用户明确提供的信息继续工作；信息不足时应说明假设或向用户追问。".into();
    }
    sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let page = source
                .page_no
                .map(|page| format!("，第 {page} 页"))
                .unwrap_or_default();
            let missing = if source.source_missing {
                "，原文件当前失联（以下仅为先前派生文本）"
            } else {
                ""
            };
            format!(
                "[材料 {}] {}（document_id={}{}{}，content_hash={}）\n{}",
                index + 1,
                source.title,
                source.document_id,
                page,
                missing,
                source.content_hash,
                source.excerpt
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn build_workspace_system_prompt(
    workspace_title: &str,
    workspace_description: &str,
    sources: &[RetrievedWorkspaceSource],
    registered_tool_names: &[&str],
    editing_target: Option<(&str, &str)>,
) -> String {
    let settings = crate::settings::read_settings().unwrap_or_default();
    let now = chrono::Local::now();
    let weekday = match now.weekday() {
        chrono::Weekday::Mon => "星期一",
        chrono::Weekday::Tue => "星期二",
        chrono::Weekday::Wed => "星期三",
        chrono::Weekday::Thu => "星期四",
        chrono::Weekday::Fri => "星期五",
        chrono::Weekday::Sat => "星期六",
        chrono::Weekday::Sun => "星期日",
    };
    let user_display_name = settings
        .user_display_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("未设置（不得猜测）");
    let region = settings
        .weather_city
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("未设置（不得猜测）");
    let runtime_context = format!(
        "当前日期时间：{} {}\n时区：本机时区 UTC{}\n使用人：{}\n所在地区：{}",
        now.format("%Y-%m-%d %H:%M:%S"),
        weekday,
        now.format("%:z"),
        user_display_name,
        region
    );
    let tool_manifest = if registered_tool_names.is_empty() {
        "（当前没有注册工具）".to_string()
    } else {
        registered_tool_names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join("、")
    };
    let has_exa = registered_tool_names
        .iter()
        .any(|name| name.starts_with("exa_"));
    let has_firecrawl = registered_tool_names
        .iter()
        .any(|name| name.starts_with("firecrawl_"));
    let network_stage = if has_exa || has_firecrawl {
        "   - 阶段 3：公开网络补检。只有用户明确要求新闻、舆情、官网、公众号文章或最新公开信息，且本地/元典覆盖不了该部分时，才使用当前已注册的 Exa/Firecrawl。Exa 负责发现与普通网页正文；Firecrawl 负责难抓页面和公众号正文。联网只补缺口，不能替代专业数据库。"
    } else {
        "   - 阶段 3：公开网络补检。只有用户明确要求新闻、舆情、官网或最新公开信息，且本地/元典覆盖不了该部分时，才使用 `web_search`；拿到明确 URL 后才用 `web_fetch` 读取正文。联网只补缺口，不能替代专业数据库。"
    };
    let network_capabilities = if has_exa || has_firecrawl {
        "5. 当前公开网络能力只以“当前实际可用工具”清单为准。当前已配置专业研究工具，不再提供基础搜索工具；优先按后附 Exa/Firecrawl 路线执行。`web_fetch` 仅用于读取已知的公开官网 URL，不承担搜索。没有浏览器登录能力，不得编造或模拟未注册工具。\n6. 专业研究工具报错或返回空结果后，可以改写一次查询或切换另一已注册 provider；仍失败就停止联网并用已取得的材料、本地知识库和元典结果收尾，不得无限重试。"
    } else {
        "5. 当前公开网络能力只以“当前实际可用工具”清单为准；未配置专业研究工具时，基础兜底是 `web_search`（DuckDuckGo HTML 搜索）和 `web_fetch`（读取一个公开网页）。没有浏览器登录能力，不得编造或模拟未注册工具。\n6. 单次任务最多 3 次 web_search、最多 5 次 web_fetch；搜索报错、返回空结果或达到上限后，停止联网并用已取得的材料、本地知识库和元典结果收尾。不要无限改关键词重试。"
    };
    let editing_contract = match editing_target {
        Some((document_id, title)) => format!(
            r#"

【当前编辑目标】
界面当前打开的是可编辑文稿《{title}》（document_id={document_id}）。用户说“当前文稿、原来的报告、这份材料、在里面修改、更新、补充、调整标题或排版”等，默认都指向这一份文稿。
- 开始修改前先用 `read_workspace_file` 读取完整当前正文；完成后必须用 `write_workspace_file` 原位更新 document_id={document_id}，保留未要求改动的内容。
- 只有用户明确要求另存、新建副本、复制一份或保留旧版时，才可调用 `create_workspace_file` / `copy_workspace_file`。不得自行添加“更新版”并改写副本。
- “标题放大、降一级、加粗、改列表或表格”等排版要求，转换成编辑器支持的 Markdown 标题层级（# / ## / ###）、粗体、列表和 GFM 表格；不要声称完成 Markdown 无法表达的任意字号或字体设置。
- 工具更新成功后明确说明“已更新当前文稿”，不要把同一内容再完整重复成一份新的聊天文稿。"#
        ),
        None => r#"

【文稿编辑规则】
如果界面存在当前打开的可编辑文稿，系统会在这里明确给出 document_id；修改、补充或排版默认原位更新它。只有用户明确要求另存、新建副本或保留旧版时才创建新文稿。排版修改使用编辑器支持的 Markdown 标题层级、粗体、列表和表格。"#
            .to_string(),
    };
    let mut prompt = format!(
        r#"你是 CaseBoard 的法律 AI 助手，当前处于“独立事务工作区”，未绑定案件看板中的诉讼案件记录。
这个工作区可能从一次临时任务开始，也可能随着材料、对话和文稿积累发展为持续、复杂的事务。不得因为入口不同而降低能力；但没有案件绑定时，不得虚构案号、诉讼身份、程序阶段或把事务伪装成已立案案件。

【工作区】
名称：{workspace_title}
说明：{workspace_description}

【当前运行环境】
{runtime_context}
以上是系统在本轮生成提示时取得的事实。涉及“今天、现在、当地、我是谁”等问题时直接据此回答，不得声称没有实时时钟，也不得用网络搜索结果反推当前日期。

【当前实际可用工具】
{tool_manifest}

【基本规则】
1. 可以在没有上传材料时按用户要求起草、修改、讨论材料；但不得虚构主体、日期、金额、事实、证据、法条或案例。
2. 区分“材料明示事实”“用户本轮陈述”“合理推断”和“待确认信息”。不确定内容用占位符或明确追问。
3. 引用工作区材料时只使用下方片段，并在回答末尾按既有 <CITATIONS> JSON 协议给出 type=doc 引用；source 必须使用材料标题，quote 必须能在片段中核对。
4. 检索通常可参考以下信息源层次；这是建议路线，不要求机械逐步执行。应结合当前任务自行跳步、调序或合并，但不得为了显得忙碌而重复搜索：
   - 阶段 0：工作区材料。先读本轮已引用/自动检索出的材料片段，提取主体、事实、时间、金额和待核问题；材料已经足够的事项不要外查。
   - 阶段 1：本地知识库。精确法规名/条号/案号/企业名先走目录或 BM25；描述型问题在词法弱命中时再用 `semantic_search_local_kb`，需要正文时用 `read_kb_file`。
   - 阶段 2：元典专业数据。本地不足时再查元典。法规/案例工具由 Rust 宿主强制重跑“精确目录 → Wiki 导航卡 → raw BM25 → raw embedding”；企业工具走企业档案名称/栏目/新鲜度检索，不使用 embedding。本地强命中时工具会直接返回本地材料。本地不足时可以为准确性继续多轮外查，但每次都要有新的检索目标或筛选条件，不得无意义重复。企业调查先用 `enterprise_search` 定位主体，再用 `enterprise_aggregation_summary` 查总览，确有需要才用 `enterprise_base_info`、`enterprise_change_info`、`enterprise_writ_list`、`enterprise_annual_report` 深入。
{network_stage}
{network_capabilities}
7. 检索、读取、核验、保存等动作统一遵守后附的“CaseBoard 法律工作台统一真实性契约”；本节只说明工作区工具路线，不另设一套较弱或相互冲突的真实性标准。
8. `verify_legal_citations` 用于核验法律引用，`ask_user` 用于缺少会实质影响结果的关键信息时追问；`save_company_report` 仅在已形成值得长期复用的企业调查报告且本地知识库已启用时写回知识库。除“当前实际可用工具”清单列明的工具外，不得自行假设任何能力。
9. 当前工具集不允许读取、修改任何案件，也不允许直接改动用户原始文件。
10. 用户要求写文稿时，先给可继续编辑的完整 Markdown 正文。除非用户明确要求，不要只给提纲。
11. 把本对话已有任务、工具结果和文稿视为同一事务的连续上下文；用户说“继续、接着做、重试、按上面的处理”时，应承接原任务，不得当成脱离上下文的新短问。

【本轮工作区材料】
{}
{}"#,
        format_workspace_sources(sources),
        editing_contract,
    );
    prompt.push_str(crate::chat::policy::LEGAL_WORKBENCH_INTEGRITY_CONTRACT);
    prompt.push_str(crate::chat::prompts::legal_research_reference_prompt());
    prompt.push_str(&crate::chat::constitution::research_route_prompt(
        registered_tool_names,
    ));
    prompt
}

fn history_for_agent(rows: Vec<ai_workspace_chat::AiWorkspaceMessage>) -> Vec<(String, String)> {
    rows.into_iter()
        .filter(|row| {
            matches!(row.role.as_str(), "user" | "assistant") && !row.content.trim().is_empty()
        })
        .map(|row| (row.role, row.content))
        .collect()
}

fn references_for_turn(
    explicit: &[WorkspaceReference],
    editing_document_id: Option<&str>,
) -> Vec<WorkspaceReference> {
    let mut references = Vec::new();
    let mut seen = HashSet::new();
    for reference in explicit {
        if seen.insert((reference.document_id.clone(), reference.page_no)) {
            references.push(reference.clone());
        }
    }
    if let Some(document_id) = editing_document_id {
        let key = (document_id.to_string(), None);
        if seen.insert(key) {
            references.push(WorkspaceReference {
                document_id: document_id.to_string(),
                page_no: None,
            });
        }
    }
    references
}

async fn citation_paths(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Vec<(String, String)>, String> {
    let documents = ai_workspace_documents::list_documents(pool, workspace_id)
        .await
        .map_err(|error| error.to_string())?;
    Ok(documents
        .into_iter()
        .filter_map(|document| {
            let path = if document.kind == "artifact" {
                document.content_path
            } else {
                document.extracted_text_path
            }?;
            Some((document.title, path))
        })
        .collect())
}

pub async fn ai_workspace_chat_impl(
    app: AppHandle,
    pool: &SqlitePool,
    cancel_registry: &ChatCancelRegistry,
    input: AiWorkspaceChatInput,
) -> Result<AiWorkspaceChatResult, String> {
    let started_at = std::time::Instant::now();
    let user_message = input.user_message.trim();
    if user_message.is_empty() {
        return Err("请输入要交给 AI 的内容".into());
    }
    if input.user_message_id.trim().is_empty() || input.message_id.trim().is_empty() {
        return Err("消息 ID 不能为空".into());
    }
    let artifact_intent = ArtifactIntent::from_user_message(user_message);
    let research_requirement = crate::chat::policy::explicit_research_requirement(user_message);
    let workspace = ai_workspaces::get_workspace(pool, &input.workspace_id)
        .await
        .map_err(|error| error.to_string())?
        .filter(|workspace| workspace.archived_at.is_none())
        .ok_or_else(|| "工作区不存在或已归档".to_string())?;
    ai_workspace_chat::get_conversation(pool, &input.workspace_id, &input.conversation_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "对话不存在或不属于当前工作区".to_string())?;

    let history_rows = ai_workspace_chat::list_messages(
        pool,
        &input.workspace_id,
        &input.conversation_id,
        Some(12),
    )
    .await
    .map_err(|error| format!("读取对话历史失败: {error}"))?;
    let previous_tool_calls_json = if asks_about_previous_tool_use(user_message) {
        sqlx::query_scalar::<_, String>(
            "SELECT tool_calls_json FROM ai_workspace_tasks \
             WHERE workspace_id = ? AND conversation_id = ? \
             ORDER BY created_at DESC, id DESC LIMIT 1",
        )
        .bind(&input.workspace_id)
        .bind(&input.conversation_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("读取上一轮工具审计记录失败: {error}"))?
    } else {
        None
    };
    let previous_model = history_rows
        .iter()
        .rev()
        .find(|row| row.role == "assistant")
        .and_then(|row| row.model.clone());
    let history = history_for_agent(history_rows);
    let editing_document = if let Some(document_id) = input.editing_document_id.as_deref() {
        let document = ai_workspace_documents::get_document(pool, &input.workspace_id, document_id)
            .await
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "正在编辑的文稿不属于当前工作区".to_string())?;
        if document.kind != "artifact" {
            return Err("正在编辑的文档不是工作区文稿".into());
        }
        Some(document)
    } else {
        None
    };
    let turn_references =
        references_for_turn(&input.references, input.editing_document_id.as_deref());
    let sources =
        retrieve_workspace_context(pool, &input.workspace_id, user_message, &turn_references)
            .await?;
    let citation_paths = citation_paths(pool, &input.workspace_id).await?;

    let settings = crate::settings::read_settings().unwrap_or_default();
    validate_ai_assistant_ready(&settings).await?;
    let runtime = AgentRuntimeKind::from_settings(&settings);
    let allow_new_workspace_file = allows_runtime_managed_workspace_files(runtime, user_message);
    let mut llm_config = LlmConfig::from_settings(&settings);
    let choice = route_model_with_context(
        TaskType::FreeChat,
        user_message,
        &history,
        previous_model.as_deref(),
        &settings,
    );
    if settings.effective_llm_provider() == "cloud" {
        llm_config.model = choice.model.clone();
    }

    let mut attached_ids = Vec::new();
    let mut seen = HashSet::new();
    for reference in &turn_references {
        if seen.insert(reference.document_id.clone()) {
            attached_ids.push(reference.document_id.clone());
        }
    }
    let attached_json = serde_json::to_string(&attached_ids).unwrap_or_else(|_| "[]".into());
    let task_id = uuid::Uuid::new_v4().to_string();
    ai_workspace_chat::begin_chat_run(
        pool,
        &input.workspace_id,
        &input.conversation_id,
        &input.user_message_id,
        &input.message_id,
        &task_id,
        user_message,
        &attached_json,
    )
    .await
    .map_err(|error| format!("保存对话消息失败: {error}"))?;
    sqlx::query(
        "UPDATE ai_workspace_tasks SET input_json = json_set(\
         input_json, '$.editing_document_id', ?, '$.allow_new_workspace_file', ?) \
         WHERE id = ? AND workspace_id = ?",
    )
    .bind(input.editing_document_id.as_deref())
    .bind(if allow_new_workspace_file { 1 } else { 0 })
    .bind(&task_id)
    .bind(&input.workspace_id)
    .execute(pool)
    .await
    .map_err(|error| format!("记录当前文稿编辑边界失败: {error}"))?;

    if asks_about_previous_tool_use(user_message) {
        let content = format_previous_tool_audit(previous_tool_calls_json.as_deref());
        let latency_ms = started_at.elapsed().as_millis() as u64;
        ai_workspace_chat::update_message_run(
            pool,
            &input.workspace_id,
            &input.message_id,
            &content,
            "completed",
            None,
            Some("caseboard-host-audit"),
            None,
            None,
            Some(latency_ms as i64),
            "[]",
            Some(&task_id),
        )
        .await
        .map_err(|error| format!("保存工具审计结果失败: {error}"))?;
        ai_workspace_chat::update_task_run(
            pool,
            &input.workspace_id,
            &task_id,
            "completed",
            "[]",
            None,
        )
        .await
        .map_err(|error| format!("保存工具审计任务状态失败: {error}"))?;
        let channel = format!("ai-workspace-chat-stream-{}", input.message_id);
        let _ = app.emit(
            &channel,
            &ChatStreamEvent::Delta {
                text: content.clone(),
            },
        );
        let _ = app.emit(
            &channel,
            &ChatStreamEvent::Done {
                prompt_tokens: None,
                completion_tokens: None,
                model: "caseboard-host-audit".into(),
            },
        );
        return Ok(AiWorkspaceChatResult {
            user_message_id: input.user_message_id,
            assistant_message_id: input.message_id,
            task_id,
            model: "caseboard-host-audit".into(),
            prompt_tokens: None,
            completion_tokens: None,
            latency_ms,
            citations: Vec::new(),
            sources,
            tool_calls: Vec::new(),
            ask_user: None,
            artifact_doc_id: None,
        });
    }

    let (cancel_tx, cancel_rx) = oneshot::channel();
    let steering_rx = cancel_registry.register(
        input.message_id.clone(),
        cancel_tx,
        ChatRunScope::Workspace {
            workspace_id: input.workspace_id.clone(),
            conversation_id: input.conversation_id.clone(),
        },
    );
    let channel = format!("ai-workspace-chat-stream-{}", input.message_id);
    let (tx, mut rx) = mpsc::unbounded_channel::<ChatStreamEvent>();
    let emit_app = app.clone();
    let checkpoint_pool = pool.clone();
    let checkpoint_workspace_id = input.workspace_id.clone();
    let checkpoint_message_id = input.message_id.clone();
    let checkpoint_task_id = task_id.clone();
    let diagnostics_request_id = input.message_id.clone();
    let forward = tokio::spawn(async move {
        let mut partial = String::new();
        let mut observed_tool_calls = Vec::new();
        let mut last_checkpoint_at = Instant::now();
        let mut needs_final_checkpoint = false;
        while let Some(event) = rx.recv().await {
            let force_checkpoint = matches!(
                event,
                ChatStreamEvent::ToolCall { .. }
                    | ChatStreamEvent::AskUser { .. }
                    | ChatStreamEvent::Done { .. }
                    | ChatStreamEvent::Error { .. }
            );
            match &event {
                ChatStreamEvent::Delta { text } => {
                    partial.push_str(text);
                    needs_final_checkpoint = true;
                }
                ChatStreamEvent::ToolCall { record } => {
                    observed_tool_calls.push(record.clone());
                    needs_final_checkpoint = true;
                }
                ChatStreamEvent::Activity { activity } => {
                    crate::chat::diagnostics::append_runtime_activity(
                        "workspace",
                        &diagnostics_request_id,
                        activity,
                    );
                }
                _ => {}
            }
            let _ = emit_app.emit(&channel, &event);
            let heartbeat_due = last_checkpoint_at.elapsed() >= Duration::from_secs(1);
            if force_checkpoint || heartbeat_due {
                let tool_calls_json =
                    serde_json::to_string(&observed_tool_calls).unwrap_or_else(|_| "[]".into());
                if let Err(error) = ai_workspace_chat::checkpoint_chat_run(
                    &checkpoint_pool,
                    &checkpoint_workspace_id,
                    &checkpoint_message_id,
                    &checkpoint_task_id,
                    &partial,
                    &tool_calls_json,
                )
                .await
                {
                    crate::dlog!("ai_workspace: 增量保存运行进度失败 → {}", error);
                }
                last_checkpoint_at = Instant::now();
                needs_final_checkpoint = false;
            }
        }
        if needs_final_checkpoint {
            let tool_calls_json =
                serde_json::to_string(&observed_tool_calls).unwrap_or_else(|_| "[]".into());
            if let Err(error) = ai_workspace_chat::checkpoint_chat_run(
                &checkpoint_pool,
                &checkpoint_workspace_id,
                &checkpoint_message_id,
                &checkpoint_task_id,
                &partial,
                &tool_calls_json,
            )
            .await
            {
                crate::dlog!("ai_workspace: 收尾前保存运行进度失败 → {}", error);
            }
        }
        (partial, observed_tool_calls)
    });

    let registry = ToolRegistry::matter_workspace_for_current_credentials();
    let registered_tool_names = registry.registered_tool_names();
    let description = workspace.description.as_deref().unwrap_or_default();
    let mut system_prompt = build_workspace_system_prompt(
        &workspace.title,
        description,
        &sources,
        &registered_tool_names,
        editing_document
            .as_ref()
            .map(|document| (document.id.as_str(), document.title.as_str())),
    );
    system_prompt.push_str(artifact_intent.prompt_contract());
    system_prompt.push_str(&crate::chat::policy::explicit_research_prompt(
        research_requirement,
        &registered_tool_names,
    ));
    let mut effective_user_message = user_message.to_string();
    if runtime == AgentRuntimeKind::Native {
        system_prompt.push_str(&crate::chat::skills::native_prompt(
            input.skill_name.as_deref(),
        )?);
    } else if let Some(skill_name) = input.skill_name.as_deref() {
        crate::chat::skills::resolve(skill_name)?;
        effective_user_message = format!("/skill:{skill_name} {effective_user_message}");
    }
    if runtime == AgentRuntimeKind::Pi {
        system_prompt.push_str(
            "\n\n【Pi Runtime 自主编排权限】Pi 应根据用户真实意图自行决定读取、原位更新、另存、复制或新建当前独立工作区中的派生文稿；即使界面已有编辑目标，也不要求用户必须先说出“另存”或“新建副本”等固定关键词。本段取代前文关于“只有用户明确要求才可新建/复制”的限制。导入的 source 原始材料仍然只读，所有写入仍必须通过 Rust 宿主提供的工作区工具并使用真实 document_id。\n\n【Pi Runtime 路由说明】Pi 可以自行决定研究思路、检索路径和关键词；实际调用法规、案例或企业工具时，CaseBoard Rust 宿主统一执行本地优先与法源时效门禁。法规、案例采用“精确目录 → Wiki 导航卡 → raw BM25 → raw 向量 → 元典”，企业采用本地企业档案名称、栏目和新鲜度核验后再外查，不使用向量。真实性、显式工具动作和失效法源规则完全以统一真实性契约为准，本段不得改变或放宽它。",
        );
    }
    let local_kb = LocalKb::auto_detect(&settings);
    let agent_request = AgentLoopRequest {
        task_type: TaskType::FreeChat,
        system_prompt,
        history,
        user_message: effective_user_message,
        temperature: choice.temperature,
        max_tokens: choice.max_tokens,
        tool_choice: "auto".into(),
        case_doc_paths_for_citation_check: citation_paths,
        loop_guard_config: Some(LoopGuardConfig::for_workspace_research(&settings)),
        emit_turn_progress: true,
        tool_call_budget_config: Some(ToolCallBudgetConfig::matter_workspace()),
    };
    let context = ToolContext {
        pool,
        settings: &settings,
        case_id: None,
        local_kb: local_kb.as_ref(),
        app: Some(app),
        message_id: Some(&input.message_id),
        visualization_consent: false,
    };
    crate::chat::policy::register_active_route(&input.message_id, research_requirement);
    let result = run_chat_with_runtime(
        runtime,
        &llm_config,
        agent_request,
        &registry,
        context,
        tx,
        ChatRunControl {
            cancel: cancel_rx,
            steering: steering_rx,
        },
    )
    .await;
    crate::chat::policy::clear_active_route(&input.message_id);
    let (partial, observed_tool_calls) = forward.await.unwrap_or_default();
    cancel_registry.finish(&input.message_id);
    let latency_ms = started_at.elapsed().as_millis() as u64;

    match result {
        Ok(output) => {
            let citations_json =
                serde_json::to_string(&output.citations).unwrap_or_else(|_| "[]".into());
            let tool_calls_json =
                serde_json::to_string(&output.tool_trace).unwrap_or_else(|_| "[]".into());
            ai_workspace_chat::update_message_run(
                pool,
                &input.workspace_id,
                &input.message_id,
                &output.content_cleaned,
                "completed",
                None,
                Some(&output.usage.model),
                output.usage.prompt_tokens.map(|value| value as i64),
                output.usage.completion_tokens.map(|value| value as i64),
                Some(latency_ms as i64),
                &citations_json,
                Some(&task_id),
            )
            .await
            .map_err(|error| format!("保存 AI 回答失败: {error}"))?;

            let ask_user_present = output
                .ask_user
                .as_ref()
                .is_some_and(|questions| !questions.is_empty());
            let mut existing_artifact_doc_id = output.tool_trace.iter().rev().find_map(|call| {
                (call.success
                    && matches!(
                        call.tool.as_str(),
                        "write_workspace_file" | "rename_workspace_file"
                    ))
                .then(|| call.args.get("document_id")?.as_str().map(str::to_string))
                .flatten()
            });
            if existing_artifact_doc_id.is_none()
                && output.tool_trace.iter().any(|call| {
                    call.success
                        && matches!(
                            call.tool.as_str(),
                            "create_workspace_file" | "copy_workspace_file"
                        )
                })
            {
                existing_artifact_doc_id = sqlx::query_scalar::<_, Option<String>>(
                    "SELECT last_document_id FROM ai_workspaces WHERE id = ?",
                )
                .bind(&input.workspace_id)
                .fetch_optional(pool)
                .await
                .unwrap_or(None)
                .flatten();
            }
            let editing_target_prevents_fallback = input
                .editing_document_id
                .as_deref()
                .filter(|_| !allow_new_workspace_file);
            let explicit_research_unmet = crate::chat::policy::research_requirement_unmet(
                research_requirement,
                &output.tool_trace,
            );
            let artifact_doc_id = if artifact_intent.should_create_fallback(
                ask_user_present,
                explicit_research_unmet,
                &output.content_cleaned,
                existing_artifact_doc_id
                    .as_deref()
                    .or(editing_target_prevents_fallback),
            ) {
                let app_data_root = crate::db::app_data_dir()
                    .map_err(|error| format!("无法定位工作区目录: {error}"))?;
                let title = format!("AI报告_{}", chrono::Local::now().format("%Y-%m-%d_%H%M%S"));
                match super::commands::create_ai_workspace_artifact_from_message_impl(
                    pool,
                    &app_data_root,
                    &input.workspace_id,
                    &input.message_id,
                    &title,
                )
                .await
                {
                    Ok(artifact) => Some(artifact.document.id),
                    Err(error) => {
                        let short = format!("报告正文已生成，但保存到工作区失败: {error}");
                        let _ = ai_workspace_chat::update_message_run(
                            pool,
                            &input.workspace_id,
                            &input.message_id,
                            &output.content_cleaned,
                            "incomplete",
                            Some(&short),
                            Some(&output.usage.model),
                            output.usage.prompt_tokens.map(|value| value as i64),
                            output.usage.completion_tokens.map(|value| value as i64),
                            Some(latency_ms as i64),
                            &citations_json,
                            Some(&task_id),
                        )
                        .await;
                        let _ = ai_workspace_chat::update_task_run(
                            pool,
                            &input.workspace_id,
                            &task_id,
                            "incomplete",
                            &tool_calls_json,
                            Some(&short),
                        )
                        .await;
                        return Err(short);
                    }
                }
            } else {
                existing_artifact_doc_id
            };
            if let Some(document_id) = artifact_doc_id.as_deref() {
                sqlx::query(
                    "UPDATE ai_workspace_messages SET artifact_document_id = ?, \
                     updated_at = datetime('now') WHERE id = ? AND conversation_id IN \
                       (SELECT id FROM ai_workspace_conversations WHERE workspace_id = ?)",
                )
                .bind(document_id)
                .bind(&input.message_id)
                .bind(&input.workspace_id)
                .execute(pool)
                .await
                .map_err(|error| format!("关联 AI 更新文稿失败: {error}"))?;
            }

            let mut quality_report = evaluate_task_quality(QualityGateInput {
                task: TaskType::FreeChat,
                content: &output.content_cleaned,
                citations: &output.citations,
                tool_calls: &output.tool_trace,
                ask_user_present,
                artifact_doc_id: artifact_doc_id.as_deref(),
            });
            crate::chat::policy::enforce_research_requirement(
                &mut quality_report,
                research_requirement,
                &output.tool_trace,
            );
            let quality_note = format_quality_gate_note(&quality_report);
            let quality_error = quality_report.incomplete.then(|| {
                quality_report
                    .warnings
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "本轮操作缺少可核验的真实工具记录，请重试。".into())
            });
            let final_content = if quality_note.is_empty() {
                output.content_cleaned.clone()
            } else {
                format!("{}{}", output.content_cleaned, quality_note)
            };
            let final_status = if quality_error.is_some() {
                "incomplete"
            } else {
                "completed"
            };
            if final_content != output.content_cleaned || quality_error.is_some() {
                ai_workspace_chat::update_message_run(
                    pool,
                    &input.workspace_id,
                    &input.message_id,
                    &final_content,
                    final_status,
                    quality_error.as_deref(),
                    Some(&output.usage.model),
                    output.usage.prompt_tokens.map(|value| value as i64),
                    output.usage.completion_tokens.map(|value| value as i64),
                    Some(latency_ms as i64),
                    &citations_json,
                    Some(&task_id),
                )
                .await
                .map_err(|error| format!("保存 AI 操作核验结果失败: {error}"))?;
            }
            ai_workspace_chat::update_task_run(
                pool,
                &input.workspace_id,
                &task_id,
                final_status,
                &tool_calls_json,
                quality_error.as_deref(),
            )
            .await
            .map_err(|error| format!("保存 AI 任务状态失败: {error}"))?;
            Ok(AiWorkspaceChatResult {
                user_message_id: input.user_message_id,
                assistant_message_id: input.message_id,
                task_id,
                model: output.usage.model,
                prompt_tokens: output.usage.prompt_tokens,
                completion_tokens: output.usage.completion_tokens,
                latency_ms,
                citations: output.citations,
                sources,
                tool_calls: output.tool_trace,
                ask_user: output.ask_user,
                artifact_doc_id,
            })
        }
        Err(error) => {
            let error_text = error.to_string();
            let cancelled =
                error_text.contains("用户取消") || error_text.to_lowercase().contains("cancel");
            crate::chat::diagnostics::append_runtime_terminal(
                "workspace",
                &input.message_id,
                runtime.as_str(),
                cancelled,
                latency_ms,
                crate::chat::diagnostics::runtime_error_category(&error_text),
            );
            let status = if cancelled {
                "cancelled"
            } else if partial.trim().is_empty() {
                "failed"
            } else {
                "incomplete"
            };
            let mut quality_report = evaluate_task_quality(QualityGateInput {
                task: TaskType::FreeChat,
                content: &partial,
                citations: &[],
                tool_calls: &observed_tool_calls,
                ask_user_present: false,
                artifact_doc_id: None,
            });
            crate::chat::policy::enforce_research_requirement(
                &mut quality_report,
                (!cancelled).then_some(research_requirement).flatten(),
                &observed_tool_calls,
            );
            let quality_note = format_quality_gate_note(&quality_report);
            let final_partial = if quality_note.is_empty() {
                partial.clone()
            } else {
                format!("{partial}{quality_note}")
            };
            let combined_error = quality_report.warnings.first().map_or_else(
                || error_text.clone(),
                |warning| format!("{error_text}；{warning}"),
            );
            let short: String = combined_error.chars().take(300).collect();
            let observed_tool_calls_json =
                serde_json::to_string(&observed_tool_calls).unwrap_or_else(|_| "[]".into());
            let _ = ai_workspace_chat::update_message_run(
                pool,
                &input.workspace_id,
                &input.message_id,
                &final_partial,
                status,
                Some(&short),
                Some(&llm_config.model),
                None,
                None,
                Some(latency_ms as i64),
                "[]",
                Some(&task_id),
            )
            .await;
            let _ = ai_workspace_chat::update_task_run(
                pool,
                &input.workspace_id,
                &task_id,
                status,
                &observed_tool_calls_json,
                Some(&short),
            )
            .await;
            Err(error_text)
        }
    }
}

#[tauri::command]
pub async fn ai_workspace_chat(
    app: AppHandle,
    pool: State<'_, SqlitePool>,
    cancel_registry: State<'_, ChatCancelRegistry>,
    input: AiWorkspaceChatInput,
) -> Result<AiWorkspaceChatResult, String> {
    ai_workspace_chat_impl(app, pool.inner(), cancel_registry.inner(), input).await
}

#[tauri::command]
pub fn cancel_ai_workspace_chat(
    cancel_registry: State<'_, ChatCancelRegistry>,
    message_id: String,
) -> Result<bool, String> {
    Ok(cancel_registry.cancel(&message_id))
}

#[tauri::command]
pub async fn steer_ai_workspace_chat(
    pool: State<'_, SqlitePool>,
    cancel_registry: State<'_, ChatCancelRegistry>,
    message_id: String,
    workspace_id: String,
    conversation_id: String,
    content: String,
) -> Result<String, String> {
    let scope = ChatRunScope::Workspace {
        workspace_id: workspace_id.clone(),
        conversation_id: conversation_id.clone(),
    };
    cancel_registry.steer(&message_id, &scope, &content)?;
    let id = uuid::Uuid::new_v4().to_string();
    ai_workspace_chat::insert_message(
        pool.inner(),
        ai_workspace_chat::NewWorkspaceMessage {
            id: &id,
            conversation_id: &conversation_id,
            role: "user",
            content: content.trim(),
            status: "completed",
            attached_document_ids_json: "[]",
        },
    )
    .await
    .map_err(|error| format!("保存引导消息失败: {error}"))?;
    Ok(id)
}
