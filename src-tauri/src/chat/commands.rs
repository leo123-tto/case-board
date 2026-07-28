//! Tauri 命令层(由 lib.rs 注册成 `#[tauri::command]`,本文件提供纯函数实现)。
//!
//! 设计:
//!   - `case_chat`:启动一次流式 chat,边收 SSE 边 `app.emit("chat-stream-{id}", ...)`,
//!     完成后 INSERT 一对 user/assistant 消息;若是固定任务且输出 ≥1500 字,落 artifact
//!   - `list_chat_history`:取案件聊天记录
//!   - `cancel_chat`:通过共享 cancel registry 取消进行中的请求
//!   - `clear_chat_history`:清空案件聊天记录(用户主动)
//!
//! 并发模型:
//!   - 每次 case_chat 生成一个 assistant_message_id(uuid),作为流式 channel 名后缀
//!   - 同 case 下并发 chat 互不干扰(channel 名不同)
//!   - cancel registry 是全局 `Mutex<HashMap<msg_id, oneshot::Sender<()>>>`
//!     通过 message_id 找到对应请求并取消
//!
//! 隐私:
//!   - chat_messages.content 永远不进反馈 MD(在 feedback 那边把关)
//!   - 这里只做生成与持久化,不暴露内容到 stderr / 日志

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot};

use crate::chat::agent_loop::{AgentLoopRequest, ToolCallRecord};
use crate::chat::artifact_intent::ArtifactIntent;
use crate::chat::citations::{parse_with_doc_paths, Citation};
use crate::chat::constitution::build_system_prompt_with_memory;
use crate::chat::context::TaskType;
use crate::chat::model_router::route_model_with_context;
use crate::chat::prompts::task_user_prompt_for;
use crate::chat::quality_gate::{
    evaluate_task_quality, format_quality_gate_note, task_output_incomplete, QualityGateInput,
};
use crate::chat::runtime::{
    run_chat_with_runtime, validate_ai_assistant_ready, AgentRuntimeKind, ChatRunControl,
};
use crate::chat::stream::{ChatStreamEvent, ChatUsage};
use crate::chat::task_contract::task_contract_prompt;
use crate::chat::tools::{ToolContext, ToolRegistry};
use crate::db::chat::{insert_chat_message, ChatMessage, NewChatMessage};
use crate::llm::LlmConfig;
use crate::local_kb::cache::LocalKb;
use crate::settings::Settings;

// =============================================================================
// Cancel Registry(全局 State)
// =============================================================================

/// 全局 chat cancel 注册表。key = assistant_message_id。
///
/// case_chat 启动时注册 oneshot::Sender,完成时移除;
/// cancel_chat 命令通过 message_id 找到并 send(()) 触发取消。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatRunScope {
    Case {
        case_id: String,
        conversation_id: String,
    },
    Workspace {
        workspace_id: String,
        conversation_id: String,
    },
}

struct ActiveChatRun {
    cancel: oneshot::Sender<()>,
    steering: mpsc::UnboundedSender<String>,
    scope: ChatRunScope,
}

#[derive(Default)]
pub struct ChatCancelRegistry {
    inner: Mutex<HashMap<String, ActiveChatRun>>,
}

impl ChatCancelRegistry {
    pub(crate) fn register(
        &self,
        message_id: String,
        sender: oneshot::Sender<()>,
        scope: ChatRunScope,
    ) -> mpsc::UnboundedReceiver<String> {
        let (steering, receiver) = mpsc::unbounded_channel();
        let mut guard = self.inner.lock().expect("chat cancel registry poisoned");
        guard.insert(
            message_id,
            ActiveChatRun {
                cancel: sender,
                steering,
                scope,
            },
        );
        receiver
    }

    fn take(&self, message_id: &str) -> Option<ActiveChatRun> {
        let mut guard = self.inner.lock().expect("chat cancel registry poisoned");
        guard.remove(message_id)
    }

    pub(crate) fn steer(
        &self,
        message_id: &str,
        scope: &ChatRunScope,
        content: &str,
    ) -> Result<(), String> {
        let content = content.trim();
        if content.is_empty() {
            return Err("引导内容不能为空".into());
        }
        if content.chars().count() > 20_000 {
            return Err("单条引导不能超过 20000 字".into());
        }
        let guard = self.inner.lock().map_err(|_| "AI 运行状态不可用")?;
        let active = guard
            .get(message_id)
            .ok_or_else(|| "当前 AI 任务已经结束".to_string())?;
        if &active.scope != scope {
            return Err("引导目标与当前 AI 任务不匹配".into());
        }
        active
            .steering
            .send(content.to_string())
            .map_err(|_| "当前 AI 任务已经结束".to_string())
    }

    pub(crate) fn cancel(&self, message_id: &str) -> bool {
        self.take(message_id)
            .is_some_and(|active| active.cancel.send(()).is_ok())
    }

    pub(crate) fn finish(&self, message_id: &str) {
        let _ = self.take(message_id);
    }
}

// =============================================================================
// 公开命令实现
// =============================================================================

/// 配套 case_chat 返回的元数据(给前端组合消息列表用)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseChatResult {
    pub user_message_id: String,
    pub assistant_message_id: String,
    pub conversation_id: String,
    pub model: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub latency_ms: u64,
    /// 若产出落了 artifact,这里返回 documents.id(前端可以马上刷新文档列表)
    pub artifact_doc_id: Option<String>,
    pub strategy: String,
    pub based_on_doc_ids: Vec<String>,
    /// V0.2 D6.5 · `<CITATIONS>` 解析后的引用列表,直接传前端 CitationsCard 渲染,
    /// 不用等下一次 list_chat_history 再回拉。
    #[serde(default)]
    pub citations: Vec<crate::chat::citations::Citation>,
    /// V0.2 D6.5 · agent_loop 跑出的 tool_trace,流式期间前端已 listen 实时拿过,
    /// 这里再回一份方便兜底(网络抖动漏一两个 emit 也能恢复完整)。
    #[serde(default)]
    pub tool_calls: Vec<crate::chat::agent_loop::ToolCallRecord>,
    /// V0.2 D6.5 · 本会话的 chat_tasks.id(若走了 agent_loop)
    #[serde(default)]
    pub task_id: Option<String>,
    /// V0.3 · 本轮模型调 `ask_user` 发起的选项式追问;前端据此渲染选项卡片,
    /// 用户回答后当作下一条普通 user 消息回灌。`None` = 正常回答。
    #[serde(default)]
    pub ask_user: Option<Vec<crate::chat::agent_loop::AskQuestion>>,
}

/// V0.2 D6.5 · `case_chat_impl` 内部把"跑完一次 LLM"统一收成一个结构,
/// 让后续落库 / CaseChatResult 拼装代码读取干净 — agent_loop 和 stream 两路收口一致。
struct ChatRunFinish {
    /// `<CITATIONS>` 剥离后的纯净 content。入 chat_messages.content + artifact 落盘都用这个。
    content_cleaned: String,
    citations: Vec<crate::chat::citations::Citation>,
    tool_calls: Vec<crate::chat::agent_loop::ToolCallRecord>,
    usage: ChatUsage,
    /// V0.2.2 · agent_loop 路径的成本/缓存指标(stream 简易路径为 None)
    metrics: Option<crate::chat::agent_loop::CostMetrics>,
    /// V0.3 · agent_loop 拦截到 `ask_user` 时带回的问题列表(stream 路径恒 None)
    ask_user: Option<Vec<crate::chat::agent_loop::AskQuestion>>,
}

/// 一次自由问 / 固定任务的入参。
///
/// 前端传 `message_id`(uuid)作为流式 channel 名后缀;后端在内部生成
/// `user_message_id` 单独入库(避免前后端 id 撞)。
#[derive(Debug, Clone, Deserialize)]
pub struct CaseChatInput {
    pub case_id: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    pub user_message: String,
    pub task_type: Option<String>,
    /// 由快捷入口或用户显式选择的全局法律 Skill；不允许 Runtime 自行安装或生成。
    #[serde(default)]
    pub skill_name: Option<String>,
    /// 前端事先生成的 assistant message id(=channel 名后缀)
    pub message_id: String,
    /// V0.2 D3-D4 新增:本轮引用的文档 id 列表(`AttachmentPicker` 选了几份)。
    /// 非空时 case_chat 强制走 agent_loop 工具链路(让 LLM 调 read_case_doc 等)。
    #[serde(default)]
    pub attached_doc_ids: Option<Vec<String>>,
    /// V0.3 ADR-0003 Phase 1B · 写作模式下编辑器里正打开的 AI 文书 doc_id。
    /// 非空时注入 system prompt,让模型知道「要改的是这份」→ 用 `edit_artifact` 局部改。
    #[serde(default)]
    pub editing_doc_id: Option<String>,
    /// 本条用户消息是不是 `AskUserCard` 回灌的选项答案(前端点「提交回答」时置 true)。
    /// 宿主据此判可视化任务的阶段,**不再靠模型写的 question 措辞去 match 前缀**(见坑:0.4.17
    /// 可视化无限追问)。老前端/旧 payload 不带此字段时默认 false,行为同首轮。
    #[serde(default)]
    pub ask_user_reply: bool,
}

/// 写入可视化工作台的授权判定。
///
/// `ask_user_reply=true`(前端 `AskUserCard` 回灌的选项答案)本身就是用户的明确授权,不必再从
/// 措辞里猜关键词——模型写的 question 措辞不可控,靠关键词猜会漏授权。用户选「暂不生成」等
/// 拒绝项时仍然判为未授权。
fn visualization_consent_from_answer(message: &str, ask_user_reply: bool) -> bool {
    if ask_user_reply {
        return !contains_visualization_refusal(message);
    }
    visualization_consent_from_message(message)
}

fn contains_visualization_refusal(message: &str) -> bool {
    [
        "暂不生成",
        "不要画",
        "不用画",
        "不需要图",
        "无需生成",
        "先不生成",
        "先别画",
    ]
    .iter()
    .any(|word| message.contains(word))
}

fn visualization_consent_from_message(message: &str) -> bool {
    let text = message.trim();
    if text.is_empty() || contains_visualization_refusal(text) {
        return false;
    }
    let mentions_visual = [
        "可视化",
        "图表",
        "时间线",
        "关系图",
        "思维导图",
        "证据矩阵",
        "柱状图",
        "折线图",
        "热力图",
        "数据条表格",
    ]
    .iter()
    .any(|word| text.contains(word));
    if !mentions_visual {
        return false;
    }
    if text.contains('→') {
        return true;
    }
    [
        "画",
        "生成",
        "制作",
        "做个",
        "做一",
        "出图",
        "展示一下",
        "展示给我",
        "可视化一下",
    ]
    .iter()
    .any(|word| text.contains(word))
}

fn build_case_chat_constitution_prompt(
    case: &crate::db::cases::Case,
    docs: &[crate::db::documents::Document],
    attached_ids: &[String],
    editing_doc_id: Option<&str>,
    ai_soul_md: Option<&str>,
    global_memories: &[String],
    case_memories: &[String],
) -> Result<String, String> {
    build_system_prompt_with_memory(
        case,
        docs,
        attached_ids,
        editing_doc_id,
        ai_soul_md,
        global_memories,
        case_memories,
    )
    .map_err(|error| format!("案件精确委托人状态无效，请到案件详情重新确认具体委托人：{error}"))
}

/// `case_chat` 主入口。返回时流式已经完成(或取消 / 错误)。
pub async fn case_chat_impl(
    app: AppHandle,
    pool: &SqlitePool,
    registry: &ChatCancelRegistry,
    input: CaseChatInput,
) -> Result<CaseChatResult, String> {
    let started_at = std::time::Instant::now();
    let task = TaskType::from_str_loose(input.task_type.as_deref());
    let artifact_intent = ArtifactIntent::from_user_message(&input.user_message);
    let channel = format!("chat-stream-{}", input.message_id);

    let conversation = match input.conversation_id.as_deref() {
        Some(conversation_id) => crate::db::case_chat_conversations::get_conversation(
            pool,
            &input.case_id,
            conversation_id,
        )
        .await
        .map_err(|error| format!("读取对话失败: {error}"))?
        .ok_or_else(|| "所选对话不存在或不属于当前案件".to_string())?,
        None => crate::db::case_chat_conversations::ensure_conversation(pool, &input.case_id)
            .await
            .map_err(|error| format!("准备案件对话失败: {error}"))?,
    };
    let conversation_id = conversation.id;

    // ── 1. 取 settings + LlmConfig ────────────────────────────────────
    let settings: Settings = crate::settings::read_settings().unwrap_or_default();
    validate_ai_assistant_ready(&settings).await?;
    let mut llm_config = LlmConfig::from_settings(&settings);

    // ── 3. 读最近聊天历史(最近 6 对 = 12 条) ────────────────────────
    let history_rows = crate::db::chat::list_chat_messages_in_conversation(
        pool,
        &input.case_id,
        &conversation_id,
        Some(12),
    )
    .await
    .map_err(|e| format!("读取聊天历史失败: {}", e))?;
    let previous_model = history_rows
        .iter()
        .rev()
        .find(|row| row.role == "assistant")
        .and_then(|row| row.model.clone());
    let history = clip_history_for_replay(&history_rows, 4000);

    // ── 4. 预构建案件 Prompt ─────────────────────────────────────────
    // 在 user 消息、chat_task、cancel registry 与流式转发启动前完成。精确委托人状态
    // 损坏时，这里向 IPC 返回可修复错误，不会留下没有 terminal 状态的半截任务。
    let attached_doc_ids_clone = input.attached_doc_ids.clone();
    let attached_ids: Vec<String> = attached_doc_ids_clone.clone().unwrap_or_default();
    let case = crate::db::cases::get_case(pool, &input.case_id)
        .await
        .map_err(|e| format!("读案件失败: {e}"))?
        .ok_or_else(|| "案件不存在".to_string())?;
    let docs = crate::db::documents::list_documents_by_case(pool, &input.case_id)
        .await
        .map_err(|e| format!("读文档失败: {e}"))?;
    let based_on_doc_ids = crate::chat::constitution::material_doc_ids(&docs, &attached_ids);
    let case_memories = crate::db::case_memories::list_active(pool, &input.case_id)
        .await
        .map_err(|e| format!("读取案件记忆失败: {e}"))?
        .into_iter()
        .map(|m| m.content)
        .collect::<Vec<_>>();
    let mut global_memories = crate::db::case_memories::list_active_global_memories(pool)
        .await
        .map_err(|e| format!("读取全局记忆失败: {e}"))?
        .into_iter()
        .map(|m| m.content)
        .collect::<Vec<_>>();
    let memory_modes = allowed_memory_modes_for_chat(task, input.editing_doc_id.is_some());
    match crate::memory_vault::build_prompt_pack_for_modes(&settings, &memory_modes) {
        Ok(pack) => global_memories.extend(pack.items),
        Err(e) => crate::dlog!("[memory] 读取 Markdown 记忆库失败: {}", e),
    }
    let global_memories = cap_prompt_memories(global_memories, 8_000, "全局记忆");
    let case_memories = cap_prompt_memories(case_memories, 6_000, "本案记忆");
    let mut constitution_prompt = build_case_chat_constitution_prompt(
        &case,
        &docs,
        &attached_ids,
        input.editing_doc_id.as_deref(),
        settings.ai_soul_md.as_deref(),
        &global_memories,
        &case_memories,
    )?;
    constitution_prompt.push_str(&task_contract_prompt(task));
    constitution_prompt.push_str(artifact_intent.prompt_contract());

    let mut user_message_final = match task_user_prompt_for(task, input.ask_user_reply) {
        Some(template) if input.user_message.trim().is_empty() => template.to_string(),
        Some(template) => format!(
            "{}\n\n[用户附加要求]\n{}",
            template,
            input.user_message.trim()
        ),
        None => input.user_message.clone(),
    };
    let research_requirement =
        crate::chat::policy::explicit_research_requirement(&input.user_message);
    let runtime = AgentRuntimeKind::from_settings(&settings);
    if runtime == AgentRuntimeKind::Native {
        constitution_prompt.push_str(&crate::chat::skills::native_prompt(
            input.skill_name.as_deref(),
        )?);
    } else if let Some(skill_name) = input.skill_name.as_deref() {
        crate::chat::skills::resolve(skill_name)?;
        user_message_final = format!("/skill:{skill_name} {user_message_final}");
    }

    // ── 5. 入库 user 消息 ────────────────────────────────────────────
    let user_msg_id = uuid::Uuid::new_v4().to_string();
    // V0.2 D6.5 · user 消息上写 attached_doc_ids,方便 history 重放时还原引用清单
    let attached_doc_ids_json = input
        .attached_doc_ids
        .as_ref()
        .filter(|v| !v.is_empty())
        .and_then(|v| serde_json::to_string(v).ok());
    insert_chat_message(
        pool,
        NewChatMessage {
            id: &user_msg_id,
            case_id: &input.case_id,
            conversation_id: Some(&conversation_id),
            role: "user",
            content: &input.user_message,
            task_type: task.as_db_str(),
            model: None,
            prompt_tokens: None,
            completion_tokens: None,
            latency_ms: None,
            based_on: None,
            artifact_doc_id: None,
            error_short: None,
            attached_doc_ids: attached_doc_ids_json.as_deref(),
            citations_json: None,
            task_id: None,
        },
    )
    .await
    .map_err(|e| format!("入库 user 消息失败: {}", e))?;
    crate::db::case_chat_conversations::touch_after_user_message(
        pool,
        &input.case_id,
        &conversation_id,
        &input.user_message,
    )
    .await
    .map_err(|error| format!("更新对话状态失败: {error}"))?;

    // ── 5. 起 cancel channel + 注册 ───────────────────────────────────
    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    let steering_rx = registry.register(
        input.message_id.clone(),
        cancel_tx,
        ChatRunScope::Case {
            case_id: input.case_id.clone(),
            conversation_id: conversation_id.clone(),
        },
    );

    // ── 6. 起 stream channel + 转发到 window ──────────────────────────
    let (tx, mut rx) = mpsc::unbounded_channel::<ChatStreamEvent>();
    let app_for_emit = app.clone();
    let channel_for_emit = channel.clone();
    let diagnostics_request_id = input.message_id.clone();
    // 边转发边累积 delta 文本:出错时拿它落库当 partial,避免前端"全消失"。
    let forward = tokio::spawn(async move {
        let mut streamed = String::new();
        while let Some(ev) = rx.recv().await {
            if let ChatStreamEvent::Delta { text } = &ev {
                streamed.push_str(text);
            }
            if let ChatStreamEvent::Activity { activity } = &ev {
                crate::chat::diagnostics::append_runtime_activity(
                    "case",
                    &diagnostics_request_id,
                    activity,
                );
            }
            let _ = app_for_emit.emit(&channel_for_emit, ev);
        }
        streamed
    });

    // ── 8. 用 model_router 选择温度 / token 上限 ──────────────────────
    // V0.3 · model_router 统一读 cloud_llm_model 档位(全局 flash/pro 或 auto 自动挡);
    // 把选中的模型回写进 llm_config,**agent_loop 和 stream 两条路径都用同一个模型**(不再分叉)。
    // ⚠️ 只在云端档覆盖:本地档(ollama)的 model 是本机模型名,绝不能被 DeepSeek 档位名覆盖。
    let choice = route_model_with_context(
        task,
        &input.user_message,
        &history,
        previous_model.as_deref(),
        &settings,
    );
    if settings.effective_llm_provider() == "cloud" {
        llm_config.model = choice.model.clone();
    }

    // ── 9. 统一走 agent_loop(V0.3.3:已删无工具 stream 路径)──────────────
    // 所有 chat 都进 agent_loop:既能 save_artifact 起草落盘,也能 read_case_doc / 查法条 /
    // semantic_search_case_docs —— 兑现「有材料+上下文,想写什么都能写」。工具是否被调由模型按
    // 宪法 + tool_choice=auto 自行决定(简单问答仍可直接答)。
    // 每次 chat 都建 chat_task(落 tool_calls / citations / finish);失败不阻断聊天,task_id=null。
    let chat_task_id: Option<String> = {
        let tid = uuid::Uuid::new_v4().to_string();
        let task_type_for_chat = task.as_db_str().unwrap_or("free_chat");
        let attached_doc_ids_json = attached_doc_ids_clone
            .as_ref()
            .filter(|v| !v.is_empty())
            .and_then(|v| serde_json::to_string(v).ok());
        let create_res = crate::db::chat_tasks::create_chat_task(
            pool,
            crate::db::chat_tasks::NewChatTask {
                id: &tid,
                case_id: &input.case_id,
                conversation_id: Some(&conversation_id),
                message_id: &input.message_id,
                task_type: task_type_for_chat,
                status: "executing",
                attached_doc_ids: attached_doc_ids_json.as_deref(),
            },
        )
        .await;
        match create_res {
            Ok(()) => Some(tid),
            Err(e) => {
                // 不阻断 chat:落不上 chat_task 时 task_id=null,trace 不持久,聊天继续
                crate::dlog!("[chat] create_chat_task 失败,task_id 不写: {}", e);
                None
            }
        }
    };

    let registry_tools = ToolRegistry::for_current_credentials();
    // V0.3.6 · 外部 MCP server(白名单,默认空 = 零开销零变化)。连/列失败的 server 跳过+dlog,不拖垮 chat。
    // 连接生命周期绑本次调用:registry_tools(持 Arc<McpClient>)在本函数末尾 drop → 子进程被 kill_on_drop 杀。
    let registry_tools = if settings.mcp_servers.is_empty() {
        registry_tools
    } else {
        let mcp_tools = crate::chat::mcp_bridge::connect_mcp_servers(&settings.mcp_servers).await;
        registry_tools.with_mcp(mcp_tools)
    };
    constitution_prompt.push_str(&crate::chat::constitution::research_route_prompt(
        &registry_tools.registered_tool_names(),
    ));
    constitution_prompt.push_str(&crate::chat::policy::explicit_research_prompt(
        research_requirement,
        &registry_tools.registered_tool_names(),
    ));
    let local_kb = LocalKb::auto_detect(&settings);
    let ctx = ToolContext {
        pool,
        settings: &settings,
        case_id: Some(&input.case_id),
        local_kb: local_kb.as_ref(),
        // reextract_document 工具需要 AppHandle 触发后台抽取并 emit 进度事件
        app: Some(app.clone()),
        message_id: Some(&input.message_id),
        visualization_consent: visualization_consent_from_answer(
            &input.user_message,
            input.ask_user_reply,
        ),
    };
    // V0.2 D6.5 · 给 citations.parse_with_doc_filenames 用,校验 type=doc 的 quote 是否在文档里
    let mut case_doc_paths_for_citation_check: Vec<(String, String)> = Vec::new();
    for d in &docs {
        if let Some(p) = &d.extracted_text_path {
            case_doc_paths_for_citation_check.push((d.filename.clone(), p.clone()));
        }
    }
    let agent_req = AgentLoopRequest {
        task_type: task,
        system_prompt: constitution_prompt,
        history: history.clone(),
        user_message: user_message_final.clone(),
        temperature: choice.temperature,
        max_tokens: choice.max_tokens,
        // thinking 模型不支持 tool_choice="required"(DeepSeek 400),降级 auto;详 resolve_tool_choice。
        tool_choice: resolve_tool_choice(task.needs_tools(), &choice.model).into(),
        case_doc_paths_for_citation_check: case_doc_paths_for_citation_check.clone(),
        loop_guard_config: None,
        emit_turn_progress: false,
        tool_call_budget_config: None,
    };
    crate::chat::policy::register_active_route(&input.message_id, research_requirement);
    let result: Result<ChatRunFinish, String> = run_chat_with_runtime(
        runtime,
        &llm_config,
        agent_req,
        &registry_tools,
        ctx,
        tx,
        ChatRunControl {
            cancel: cancel_rx,
            steering: steering_rx,
        },
    )
    .await
    .map(|out| ChatRunFinish {
        content_cleaned: out.content_cleaned,
        citations: out.citations,
        tool_calls: out.tool_trace,
        usage: out.usage,
        metrics: Some(out.metrics),
        ask_user: out.ask_user,
    })
    .map_err(|e| e.to_string());
    crate::chat::policy::clear_active_route(&input.message_id);

    // 等 forward 把 channel 排空,拿回已流式产出的文本(出错时当 partial 落库)
    let streamed_partial = forward.await.unwrap_or_default();
    // 无论成败,清掉 registry(注册的 sender 可能已被消费,这里兜底)
    registry.finish(&input.message_id);

    let latency_ms = started_at.elapsed().as_millis() as u64;

    match result {
        Ok(ChatRunFinish {
            content_cleaned,
            citations: mut final_citations,
            tool_calls: final_tool_calls,
            usage,
            metrics,
            ask_user,
        }) => {
            // V0.2.2 · 成本/缓存指标落盘(只 agent_loop 路径有;失败不致命,不含任何案件内容)
            if let Some(m) = &metrics {
                append_agent_metrics(
                    &input.case_id,
                    task.as_db_str().unwrap_or("free_chat"),
                    &usage.model,
                    m,
                    &final_tool_calls,
                    latency_ms,
                );
            }
            let assistant_id = input.message_id.clone();
            // V0.2 D6.5 · 入 chat_messages.content 用 cleaned(剥掉 <CITATIONS> 块);
            // artifact 落盘也用 cleaned,防止 .md 文件里残留 JSON 引用块。
            let mut assistant_content = content_cleaned;
            let explicit_research_unmet = ask_user.as_ref().is_none_or(|items| items.is_empty())
                && crate::chat::policy::research_requirement_unmet(
                    research_requirement,
                    &final_tool_calls,
                );
            let output_incomplete = ask_user.as_ref().is_none_or(|items| items.is_empty())
                && (task_output_incomplete(task, &assistant_content) || explicit_research_unmet);
            // V0.3 D2 · save_artifact(自由聊天起草文书)写的是独立 document,不走上面 task-based
            // 路径,artifact_doc_id 仍为 None。这里补:本轮有成功的 save_artifact 工具调用时,
            // 取该案最新 chat_artifact 文档 id 回传 → 前端据此**自动进 Milkdown 编辑器打开**。
            // (MVP 一轮至多一个 save_artifact,最新即本轮所产;多个时取最新也合理。)
            let mut artifact_doc_id = None;
            let mut artifact_persistence_error = None;
            if let Some(id) = final_tool_calls.iter().rev().find_map(|call| {
                (call.success
                    && matches!(
                        call.tool.as_str(),
                        "write_workspace_file" | "rename_workspace_file"
                    ))
                .then(|| call.args.get("document_id")?.as_str().map(str::to_string))
                .flatten()
            }) {
                artifact_doc_id = Some(id);
            }
            if artifact_doc_id.is_none()
                && final_tool_calls.iter().any(|t| {
                    t.success
                        && matches!(
                            t.tool.as_str(),
                            "save_artifact" | "create_workspace_file" | "copy_workspace_file"
                        )
                })
            {
                match sqlx::query_scalar::<_, String>(
                    "SELECT id FROM documents \
                     WHERE case_id = ? AND source = 'chat_artifact' AND deleted_at IS NULL \
                     ORDER BY created_at DESC, rowid DESC LIMIT 1",
                )
                .bind(&input.case_id)
                .fetch_optional(pool)
                .await
                {
                    Ok(Some(id)) => artifact_doc_id = Some(id),
                    Ok(None) => {}
                    Err(e) => crate::dlog!("[chat] 查工作区 artifact doc_id 失败: {}", e),
                }
            }

            // 固定任务保留既有自动落盘；若模型已经成功保存，则不再制造重复文件。
            if artifact_doc_id.is_none() {
                if let Some(task_str) = task.as_db_str() {
                    if !output_incomplete && assistant_content.chars().count() >= 1500 {
                        match write_chat_artifact(
                            pool,
                            &input.case_id,
                            &assistant_id,
                            task_str,
                            &assistant_content,
                        )
                        .await
                        {
                            Ok(doc_id) => artifact_doc_id = Some(doc_id),
                            Err(e) => crate::dlog!("[chat] artifact 写盘失败: {}", e),
                        }
                    }
                }
            }

            // 用户明确要求“保存下来”时，宿主层验证真实 artifact。模型忘记调用工具也会
            // 把完整最终 Markdown 兜底保存；追问、空正文和不完整输出绝不误存。
            if artifact_intent.should_create_fallback(
                ask_user.as_ref().is_some_and(|items| !items.is_empty()),
                output_incomplete,
                &assistant_content,
                artifact_doc_id.as_deref(),
            ) {
                match write_chat_artifact(
                    pool,
                    &input.case_id,
                    &assistant_id,
                    "requested_report",
                    &assistant_content,
                )
                .await
                {
                    Ok(doc_id) => artifact_doc_id = Some(doc_id),
                    Err(e) => {
                        crate::dlog!("[chat] 用户请求的报告兜底保存失败: {}", e);
                        artifact_persistence_error =
                            Some("报告正文已生成，但保存到工作区失败，请重试保存。".to_string());
                    }
                }
            }

            if final_citations.is_empty() {
                final_citations = citations_from_save_artifact_tool_calls(
                    &final_tool_calls,
                    &case_doc_paths_for_citation_check,
                );
            }

            let mut quality_report = evaluate_task_quality(QualityGateInput {
                task,
                content: &assistant_content,
                citations: &final_citations,
                tool_calls: &final_tool_calls,
                ask_user_present: ask_user.as_ref().is_some_and(|items| !items.is_empty()),
                artifact_doc_id: artifact_doc_id.as_deref(),
            });
            crate::chat::policy::enforce_research_requirement(
                &mut quality_report,
                research_requirement,
                &final_tool_calls,
            );
            let incomplete_error = quality_report
                .incomplete
                .then(|| {
                    quality_report.warnings.first().cloned().unwrap_or_else(|| {
                        "本轮任务未形成可核验的完整结果；已保留过程内容，请重试。".into()
                    })
                })
                .or(artifact_persistence_error);
            let mut quality_note = format_quality_gate_note(&quality_report);
            if incomplete_error
                .as_deref()
                .is_some_and(|message| message.contains("保存到工作区失败"))
            {
                quality_note.push_str("\n\n> 保存提示：报告正文已生成，但没有成功写入工作区；系统未将本轮标记为完成。");
            }
            if !quality_note.is_empty() {
                assistant_content.push_str(&quality_note);
                let _ = app.emit(&channel, ChatStreamEvent::Delta { text: quality_note });
            }

            // ── 10. 入库 assistant 消息 ──────────────────────────────
            let based_on_json =
                serde_json::to_string(&based_on_doc_ids).unwrap_or_else(|_| "[]".into());
            // V0.2 D6.5 · citations + tool_calls 处理
            let citations_json = if !final_citations.is_empty() {
                serde_json::to_string(&final_citations).ok()
            } else {
                None
            };
            let tool_calls_json = if !final_tool_calls.is_empty() {
                serde_json::to_string(&final_tool_calls).ok()
            } else {
                None
            };

            // V0.2 D6.5 · 走 agent_loop 时本会话有 chat_task,落 tool_calls + citations + finish
            if let Some(tid) = &chat_task_id {
                let _ = crate::db::chat_tasks::update_chat_task(
                    pool,
                    tid,
                    crate::db::chat_tasks::UpdateChatTask {
                        tool_calls_json: tool_calls_json.as_deref(),
                        citations_json: citations_json.as_deref(),
                        model_used: Some(&usage.model),
                        prompt_tokens: usage.prompt_tokens.map(|x| x as i64),
                        completion_tokens: usage.completion_tokens.map(|x| x as i64),
                        artifact_doc_id: artifact_doc_id.as_deref(),
                        ..Default::default()
                    },
                )
                .await;
                let _ = if let Some(error) = incomplete_error.as_deref() {
                    crate::db::chat_tasks::finish_chat_task(pool, tid, "failed", Some(error)).await
                } else {
                    crate::db::chat_tasks::finish_chat_task(pool, tid, "done", None).await
                };
            }

            insert_chat_message(
                pool,
                NewChatMessage {
                    id: &assistant_id,
                    case_id: &input.case_id,
                    conversation_id: Some(&conversation_id),
                    role: "assistant",
                    content: &assistant_content,
                    task_type: task.as_db_str(),
                    model: Some(&usage.model),
                    prompt_tokens: usage.prompt_tokens.map(|x| x as i64),
                    completion_tokens: usage.completion_tokens.map(|x| x as i64),
                    latency_ms: Some(latency_ms as i64),
                    based_on: Some(&based_on_json),
                    artifact_doc_id: artifact_doc_id.as_deref(),
                    error_short: incomplete_error.as_deref(),
                    attached_doc_ids: None,
                    citations_json: citations_json.as_deref(),
                    task_id: chat_task_id.as_deref(),
                },
            )
            .await
            .map_err(|e| format!("入库 assistant 消息失败: {}", e))?;

            if incomplete_error.is_none() {
                match persist_memory_candidates_from_turn(
                    pool,
                    &input.case_id,
                    &user_msg_id,
                    &assistant_id,
                    &input.user_message,
                    &assistant_content,
                )
                .await
                {
                    Ok(n) if n > 0 => crate::dlog!("[memory] 新增 {} 条候选记忆", n),
                    Ok(_) => {}
                    Err(e) => crate::dlog!("[memory] 生成候选记忆失败: {}", e),
                }
            }

            // chat 完成后后台增量索引:本轮若调过 get_law_article/get_case_detail,新缓存的
            // 法条/案例补进语义索引(单飞 + 无新增早退,所以多数轮次是廉价 no-op)。
            crate::spawn_kb_auto_index(app.clone());

            Ok(CaseChatResult {
                user_message_id: user_msg_id,
                assistant_message_id: assistant_id,
                conversation_id: conversation_id.clone(),
                model: Some(usage.model),
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                latency_ms,
                artifact_doc_id,
                strategy: "agent-loop".to_string(),
                based_on_doc_ids,
                citations: final_citations,
                tool_calls: final_tool_calls,
                task_id: chat_task_id.clone(),
                ask_user,
            })
        }
        Err(err) => {
            // 出错也要 emit Error 给前端
            let msg = err.to_string();
            let cancelled = msg.contains("用户取消") || msg.to_lowercase().contains("cancel");
            crate::chat::diagnostics::append_runtime_terminal(
                "case",
                &input.message_id,
                runtime.as_str(),
                cancelled,
                latency_ms,
                crate::chat::diagnostics::runtime_error_category(&msg),
            );
            let _ = app.emit(
                &channel,
                ChatStreamEvent::Error {
                    message: msg.clone(),
                },
            );
            crate::dlog!("[chat] case_chat 失败: {}", msg);

            // V0.2 D6.5 · chat_task 收尾标 failed / cancelled
            if let Some(tid) = &chat_task_id {
                // 「用户取消」走 cancelled,其他走 failed(便于前端区分展示)
                let terminal = if cancelled { "cancelled" } else { "failed" };
                let _ = crate::db::chat_tasks::finish_chat_task(
                    pool,
                    tid,
                    terminal,
                    Some(&sanitize_error(&msg)),
                )
                .await;
            }

            // 失败也入库 assistant 行:content 落已流式产出的 partial(可空)+ error_short,
            // 让前端历史回放仍能看到"已生成的半截答案 + 出错中断"提示,而非整段消失。
            let assistant_id = input.message_id.clone();
            let _ = insert_chat_message(
                pool,
                NewChatMessage {
                    id: &assistant_id,
                    case_id: &input.case_id,
                    conversation_id: Some(&conversation_id),
                    role: "assistant",
                    content: &streamed_partial,
                    task_type: task.as_db_str(),
                    model: None,
                    prompt_tokens: None,
                    completion_tokens: None,
                    latency_ms: Some(latency_ms as i64),
                    based_on: None,
                    artifact_doc_id: None,
                    error_short: Some(&sanitize_error(&msg)),
                    attached_doc_ids: None,
                    citations_json: None,
                    task_id: chat_task_id.as_deref(),
                },
            )
            .await;
            Err(msg)
        }
    }
}

/// 取案件聊天记录(默认升序,前端直接渲染)。
pub async fn list_chat_history_impl(
    pool: &SqlitePool,
    case_id: &str,
    conversation_id: Option<&str>,
    limit: Option<i64>,
) -> Result<Vec<ChatMessage>, String> {
    let conversation = match conversation_id {
        Some(conversation_id) => {
            crate::db::case_chat_conversations::get_conversation(pool, case_id, conversation_id)
                .await
                .map_err(|error| format!("读取对话失败: {error}"))?
                .ok_or_else(|| "所选对话不存在或不属于当前案件".to_string())?
        }
        None => crate::db::case_chat_conversations::ensure_conversation(pool, case_id)
            .await
            .map_err(|error| format!("准备案件对话失败: {error}"))?,
    };
    crate::db::chat::list_chat_messages_in_conversation(pool, case_id, &conversation.id, limit)
        .await
        .map_err(|e| format!("读取聊天历史失败: {}", e))
}

/// 取消进行中的 chat。`message_id` 必须跟 case_chat 入参一致(=channel 后缀)。
pub fn cancel_chat_impl(registry: &ChatCancelRegistry, message_id: &str) -> bool {
    registry.cancel(message_id)
}

pub async fn steer_case_chat_impl(
    pool: &SqlitePool,
    registry: &ChatCancelRegistry,
    message_id: &str,
    case_id: &str,
    conversation_id: &str,
    content: &str,
) -> Result<String, String> {
    let scope = ChatRunScope::Case {
        case_id: case_id.to_string(),
        conversation_id: conversation_id.to_string(),
    };
    registry.steer(message_id, &scope, content)?;
    let id = uuid::Uuid::new_v4().to_string();
    insert_chat_message(
        pool,
        NewChatMessage {
            id: &id,
            case_id,
            conversation_id: Some(conversation_id),
            role: "user",
            content: content.trim(),
            task_type: Some("steering"),
            model: None,
            prompt_tokens: None,
            completion_tokens: None,
            latency_ms: None,
            based_on: None,
            artifact_doc_id: None,
            error_short: None,
            attached_doc_ids: None,
            citations_json: None,
            task_id: None,
        },
    )
    .await
    .map_err(|error| format!("保存引导消息失败: {error}"))?;
    crate::db::case_chat_conversations::touch_after_user_message(
        pool,
        case_id,
        conversation_id,
        content.trim(),
    )
    .await
    .map_err(|error| format!("更新对话状态失败: {error}"))?;
    Ok(id)
}

/// 清空某案件下所有聊天记录(用户主动)。
pub async fn clear_chat_history_impl(pool: &SqlitePool, case_id: &str) -> Result<u64, String> {
    crate::db::chat::delete_chat_history_for_case(pool, case_id)
        .await
        .map_err(|e| format!("清空聊天记录失败: {}", e))
}

pub async fn clear_chat_conversation_impl(
    pool: &SqlitePool,
    case_id: &str,
    conversation_id: &str,
) -> Result<u64, String> {
    crate::db::case_chat_conversations::get_conversation(pool, case_id, conversation_id)
        .await
        .map_err(|error| format!("读取对话失败: {error}"))?
        .ok_or_else(|| "所选对话不存在或不属于当前案件".to_string())?;
    crate::db::chat::delete_chat_history_for_conversation(pool, case_id, conversation_id)
        .await
        .map_err(|error| format!("清空当前对话失败: {error}"))
}

// =============================================================================
// 内部 helper
// =============================================================================

async fn persist_memory_candidates_from_turn(
    pool: &SqlitePool,
    case_id: &str,
    user_message_id: &str,
    assistant_message_id: &str,
    user_text: &str,
    assistant_text: &str,
) -> Result<usize, String> {
    let drafts = crate::chat::memory_extract::extract_memory_candidates_from_turn(
        case_id,
        user_text,
        assistant_text,
    );
    if drafts.is_empty() {
        return Ok(0);
    }
    let event = crate::db::case_memories::record_turn_event(
        pool,
        Some(case_id),
        user_message_id,
        assistant_message_id,
        user_text,
        assistant_text,
    )
    .await?;
    let mut created = 0;
    for draft in drafts {
        crate::db::case_memories::create_candidate_from_draft(pool, &event.id, &draft).await?;
        created += 1;
    }
    Ok(created)
}

/// 从历史里截最近 N 对 user/assistant,总字符不超 budget。
///
/// 返回值是 `(role, content)` 列表,**正序**,可以直接拼到 messages 后。
fn clip_history_for_replay(rows: &[ChatMessage], char_budget: usize) -> Vec<(String, String)> {
    // 从最新往前累计,达到 budget 停;输出时再反转
    let mut acc: Vec<(String, String)> = Vec::new();
    let mut chars_used = 0usize;
    for m in rows.iter().rev() {
        if m.role != "user" && m.role != "assistant" {
            continue;
        }
        // 跳过错误 assistant 行(空 content + error_short)
        if m.role == "assistant" && m.content.is_empty() && m.error_short.is_some() {
            continue;
        }
        let len = m.content.chars().count();
        if chars_used + len > char_budget {
            break;
        }
        chars_used += len;
        acc.push((m.role.clone(), m.content.clone()));
    }
    acc.reverse();
    acc
}

fn citations_from_save_artifact_tool_calls(
    tool_calls: &[ToolCallRecord],
    case_doc_paths_for_citation_check: &[(String, String)],
) -> Vec<Citation> {
    let mut citations = Vec::new();
    for call in tool_calls {
        if call.tool != "save_artifact" || !call.success {
            continue;
        }
        let Some(content_md) = call.args.get("content_md").and_then(|v| v.as_str()) else {
            continue;
        };
        citations
            .extend(parse_with_doc_paths(content_md, case_doc_paths_for_citation_check).citations);
    }
    citations
}

/// 把 chat 输出落成 artifact MD,同时 INSERT 一行 documents(source='chat')。
///
/// 路径:`<app_data>/extracts/<case_id>/chat_artifacts/<assistant_message_id>.md`。
/// 返回新建的 documents.id。
/// V0.2.2 · 把一次 agent_loop 任务的成本/缓存指标 append 成一行 JSONL,落到
/// `<app_data>/agent_metrics.jsonl`,用于离线分析缓存命中率 / 成本 / sub-agent 收益评估。
///
/// **隐私**:只记数字 / 任务类型枚举 / 模型名 / 计数 —— **不含**任何案件内容、query 文本、
/// 法条原文。case_id 是内部 uuid(非当事人姓名),仅用于按案件分组对比。失败静默(诊断不致命)。
fn append_agent_metrics(
    case_id: &str,
    task_type: &str,
    model: &str,
    m: &crate::chat::agent_loop::CostMetrics,
    tool_calls: &[crate::chat::agent_loop::ToolCallRecord],
    latency_ms: u64,
) {
    // DeepSeek 定价(RMB / 百万 token):flash 缓存0.02/输入1/输出2;pro 缓存0.025/输入3/输出6
    let is_flash = model.contains("flash");
    let (r_hit, r_miss, r_out) = if is_flash {
        (0.02, 1.0, 2.0)
    } else {
        (0.025, 3.0, 6.0)
    };
    let cost = m.cache_hit_tokens as f64 / 1e6 * r_hit
        + m.cache_miss_tokens as f64 / 1e6 * r_miss
        + m.completion_tokens as f64 / 1e6 * r_out;
    let total_in = m.cache_hit_tokens + m.cache_miss_tokens;
    let hit_ratio = if total_in > 0 {
        m.cache_hit_tokens as f64 / total_in as f64
    } else {
        0.0
    };
    let kb_hits = tool_calls.iter().filter(|t| t.kb_hit).count();
    let row = serde_json::json!({
        "ts": chrono::Local::now().to_rfc3339(),
        "case_id": case_id,
        "task_type": task_type,
        "model": model,
        "turns": m.turns,
        "tool_calls": tool_calls.len(),
        "kb_hits": kb_hits,
        "prompt_tokens": m.prompt_tokens,
        "completion_tokens": m.completion_tokens,
        "cache_hit_tokens": m.cache_hit_tokens,
        "cache_miss_tokens": m.cache_miss_tokens,
        "hit_ratio": (hit_ratio * 1000.0).round() / 1000.0,
        "est_cost_rmb": (cost * 10000.0).round() / 10000.0,
        "latency_ms": latency_ms,
        // V0.3.5 · 前缀指纹(哈希,不含内容):跨记录比对看缓存漂移。sys/tools 分量便于定位漂移来源。
        "prefix_fp": m.prefix_fp.as_str(),
        "prefix_sys": m.prefix_sys.as_str(),
        "prefix_tools": m.prefix_tools.as_str(),
    });
    let Ok(base) = crate::db::app_data_dir() else {
        return;
    };
    let path = base.join("agent_metrics.jsonl");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(f, "{}", row);
    }
}

/// chat artifact 落盘文件名用的可读任务名(替代原 `task_type__uuid` 一长串乱码)。
fn artifact_display_name(task_type: &str) -> &'static str {
    match task_type {
        "generate_case_overview" => "案件总览",
        "generate_evidence_list" => "证据目录",
        "generate_timeline" => "时间线",
        "generate_client_update" => "客户进展",
        "find_payment" => "付款梳理",
        "list_missing" => "待补材料",
        "compile_legal_basis" => "法律依据",
        "find_similar_cases" => "类案检索",
        "verify_my_draft" => "草稿核校",
        "simulate_opposition" => "模拟对抗",
        "requested_report" => "AI报告",
        _ => "AI助手",
    }
}

async fn write_chat_artifact(
    pool: &SqlitePool,
    case_id: &str,
    assistant_message_id: &str,
    task_type: &str,
    content: &str,
) -> Result<String, String> {
    let _ = assistant_message_id; // 关联走 DB(chat_messages.artifact_doc_id),不再塞进文件名
    let dir = chat_artifact_dir_for_case(case_id)?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("建目录 {} 失败: {}", dir.display(), e))?;
    // 简洁文件名:「类案检索.md」,重名自动「类案检索 2.md」(2026-07-27 老板反馈:
    // 旧式「类案检索_2026-07-27_200542.md」太繁琐;时间在列表和 created_at 里都有)。
    let filename = crate::chat::tools::artifact::unique_artifact_filename(
        &dir,
        artifact_display_name(task_type),
    );
    let path = dir.join(&filename);
    // V0.3 · 不再写 `<!-- chat artifact · task=.. -->` 注释头:元数据在 DB(category=task_type +
    // created_at),文件现在会进编辑器编辑 / 导出 Word,注释头只会泄漏成正文垃圾。直接存正文。
    // (content 已是 content_cleaned,CITATIONS 已在 agent_loop 剥掉,含未闭合块也剥 —— citations.rs。)
    tokio::fs::write(&path, content)
        .await
        .map_err(|e| format!("写 {} 失败: {}", path.display(), e))?;

    let doc_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let path_str = path.to_string_lossy().to_string();

    sqlx::query(
        "INSERT INTO documents \
         (id, case_id, source_path, filename, stage, category, is_ai_artifact, \
          mime_type, size_bytes, modified_at, extraction_status, \
          extracted_text_path, source, created_at) \
         VALUES (?, ?, ?, ?, NULL, ?, 1, 'text/markdown', ?, ?, 'done', ?, 'chat', ?)",
    )
    .bind(&doc_id)
    .bind(case_id)
    .bind(&path_str)
    .bind(&filename)
    .bind(task_type)
    .bind(content.len() as i64)
    .bind(&now)
    .bind(&path_str)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| format!("INSERT chat artifact 失败: {}", e))?;

    Ok(doc_id)
}

fn chat_artifact_dir_for_case(case_id: &str) -> Result<PathBuf, String> {
    let base = crate::db::app_data_dir().map_err(|e| format!("无法定位 app data dir: {}", e))?;
    Ok(base.join("extracts").join(case_id).join("chat_artifacts"))
}

/// 错误消息脱敏:截短 + 去掉绝对路径片段。
fn sanitize_error(s: &str) -> String {
    let snippet: String = s.chars().take(400).collect();
    // 走全局 sanitize(已有的 feedback 模块路径脱敏逻辑)
    crate::feedback::sanitize_paths(&snippet)
}

/// 解析 `tool_choice`。
///
/// **实测 2026-05-30**:DeepSeek V4 **全系**(`flash` / `pro`)都是思考模式,
/// 都**不支持** `tool_choice="required"`,会返回 400 `"Thinking mode does not support this tool_choice"`
/// (flash + required 实测同样 400)。旧逻辑按模型名判 thinking(只降级含 "pro"/"thinking" 的)是错的
/// —— flash 名字不含这俩却也拒 required,只是因"工具任务恰好都路由到 pro"才没在默认配置下爆。
/// 故一律用 `"auto"`:宪法第四条"工具优于直答"仍会驱动模型去调工具,不会漏查。
/// (保留入参以兼容调用点;若将来出现真正支持 required 的非思考模型,在此一处放开即可。)
fn resolve_tool_choice(_needs_tools: bool, _model: &str) -> &'static str {
    "auto"
}

fn cap_prompt_memories(items: Vec<String>, char_budget: usize, label: &str) -> Vec<String> {
    let budget = char_budget.max(500);
    let mut out = Vec::new();
    let mut used = 0usize;
    let mut omitted = 0usize;

    for item in items {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        let len = trimmed.chars().count() + 1;
        if used + len <= budget {
            used += len;
            out.push(trimmed.to_string());
        } else {
            omitted += 1;
        }
    }

    if omitted > 0 {
        while !out.is_empty() && budget.saturating_sub(used) < 120 {
            if let Some(removed) = out.pop() {
                used = used.saturating_sub(removed.chars().count() + 1);
                omitted += 1;
            }
        }
        let mut summary =
            format!("[压缩{label}] 另有 {omitted} 条记忆因上下文预算限制未展开;本轮优先遵守用户最新指令、案件材料和工具结果。");
        let remaining = budget.saturating_sub(used);
        if summary.chars().count() > remaining {
            summary = summary.chars().take(remaining).collect();
        }
        if !summary.is_empty() {
            out.push(summary);
        }
    }

    out
}

fn allowed_memory_modes_for_chat(task: TaskType, editing_doc: bool) -> Vec<&'static str> {
    let mut modes = vec!["global_prompt", "case_prompt"];
    if task.needs_tools() {
        modes.push("tool_prompt");
    }
    if editing_doc || matches!(task, TaskType::VerifyMyDraft) {
        modes.push("writing_prompt");
    }
    modes
}

// =============================================================================
// 测试
// =============================================================================
