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

use crate::chat::agent_loop::{run_chat_with_tools, AgentLoopRequest};
use crate::chat::constitution::build_system_prompt;
use crate::chat::context::TaskType;
use crate::chat::model_router::route_model;
use crate::chat::prompts::task_user_prompt;
use crate::chat::stream::{ChatStreamEvent, ChatUsage};
use crate::chat::tools::{ToolContext, ToolRegistry};
use crate::db::chat::{insert_chat_message, list_chat_messages, ChatMessage, NewChatMessage};
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
#[derive(Default)]
pub struct ChatCancelRegistry {
    inner: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

impl ChatCancelRegistry {
    fn register(&self, message_id: String, sender: oneshot::Sender<()>) {
        let mut guard = self.inner.lock().expect("chat cancel registry poisoned");
        guard.insert(message_id, sender);
    }

    fn take(&self, message_id: &str) -> Option<oneshot::Sender<()>> {
        let mut guard = self.inner.lock().expect("chat cancel registry poisoned");
        guard.remove(message_id)
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
    pub user_message: String,
    pub task_type: Option<String>,
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
    let channel = format!("chat-stream-{}", input.message_id);

    // ── 1. 取 settings + LlmConfig ────────────────────────────────────
    let settings: Settings = crate::settings::read_settings().unwrap_or_default();
    // 2026-06-15:按云端后端检查对应的 key(MiniMax / DeepSeek 各自独立字段)。
    if settings.effective_llm_provider() == "cloud" {
        let backend = settings.effective_cloud_llm_backend();
        // 2026-06-16:按后端取对应 key 字段(DeepSeek / MiniMax / 通用兼容 三选一)。
        let (key_value, name): (Option<String>, &str) = if backend == "minimax" {
            (settings.minimax_api_key.clone(), "MiniMax")
        } else if settings.cloud_llm_is_compat() {
            let label = crate::llm::providers::compat_preset(backend)
                .map(|p| p.label)
                .unwrap_or("云端 LLM");
            (settings.effective_compat_llm_api_key(), label)
        } else {
            (settings.cloud_llm_api_key.clone(), "DeepSeek")
        };
        let key_missing = key_value.as_deref().map(str::trim).unwrap_or("").is_empty();
        if key_missing {
            return Err(format!("尚未配置 {} API Key,请在设置页填入", name));
        }
    }
    let mut llm_config = LlmConfig::from_settings(&settings);

    // ── 3. 读最近聊天历史(最近 6 对 = 12 条) ────────────────────────
    let history_rows = list_chat_messages(pool, &input.case_id, Some(12))
        .await
        .map_err(|e| format!("读取聊天历史失败: {}", e))?;
    let history = clip_history_for_replay(&history_rows, 4000);

    // ── 4. 入库 user 消息 ────────────────────────────────────────────
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

    // ── 5. 起 cancel channel + 注册 ───────────────────────────────────
    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();
    registry.register(input.message_id.clone(), cancel_tx);

    // ── 6. 起 stream channel + 转发到 window ──────────────────────────
    let (tx, mut rx) = mpsc::unbounded_channel::<ChatStreamEvent>();
    let app_for_emit = app.clone();
    let channel_for_emit = channel.clone();
    // 边转发边累积 delta 文本:出错时拿它落库当 partial,避免前端"全消失"。
    let forward = tokio::spawn(async move {
        let mut streamed = String::new();
        while let Some(ev) = rx.recv().await {
            if let ChatStreamEvent::Delta { text } = &ev {
                streamed.push_str(text);
            }
            let _ = app_for_emit.emit(&channel_for_emit, ev);
        }
        streamed
    });

    // ── 7. 拼 user_prompt(固定任务前缀 + 用户原话) ──────────────────
    let user_message_final = match task_user_prompt(task) {
        Some(template) => {
            if input.user_message.trim().is_empty() {
                template.to_string()
            } else {
                format!(
                    "{}\n\n[用户附加要求]\n{}",
                    template,
                    input.user_message.trim()
                )
            }
        }
        None => input.user_message.clone(),
    };

    // V0.2 D4-D5.D · 用 model_router 替换硬编码 temperature/max_tokens
    // V0.3 · model_router 统一读 cloud_llm_model 档位(全局 flash/pro 或 auto 自动挡);
    // 把选中的模型回写进 llm_config,**agent_loop 和 stream 两条路径都用同一个模型**(不再分叉)。
    // ⚠️ 只在云端档覆盖:本地档(ollama)的 model 是本机模型名,绝不能被 DeepSeek 档位名覆盖。
    let choice = route_model(task, &input.user_message, &settings);
    if settings.effective_llm_provider() == "cloud" {
        llm_config.model = choice.model.clone();
    }

    // ── 8. 统一走 agent_loop(V0.3.3:已删无工具 stream 路径)──────────────
    // 所有 chat 都进 agent_loop:既能 save_artifact 起草落盘,也能 read_case_doc / 查法条 /
    // semantic_search_case_docs —— 兑现「有材料+上下文,想写什么都能写」。工具是否被调由模型按
    // 宪法 + tool_choice=auto 自行决定(简单问答仍可直接答)。
    let attached_doc_ids_clone = input.attached_doc_ids.clone();

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

    // 拉案件 + 文档,constitution 拼完整宪法 prompt(带 attached_ids 焦点段)
    let case = crate::db::cases::get_case(pool, &input.case_id)
        .await
        .map_err(|e| format!("读案件失败: {}", e))?
        .ok_or_else(|| "案件不存在".to_string())?;
    let docs = crate::db::documents::list_documents_by_case(pool, &input.case_id)
        .await
        .map_err(|e| format!("读文档失败: {}", e))?;
    let attached_ids: Vec<String> = attached_doc_ids_clone.clone().unwrap_or_default();
    // based_on:本轮喂进上下文的「材料文档」id(写 chat_messages.based_on);原由 build_context 返回
    let based_on_doc_ids = crate::chat::constitution::material_doc_ids(&docs, &attached_ids);
    let constitution_prompt =
        build_system_prompt(&case, &docs, &attached_ids, input.editing_doc_id.as_deref());

    let registry_tools = ToolRegistry::default_v0_2();
    // V0.3.6 · 外部 MCP server(白名单,默认空 = 零开销零变化)。连/列失败的 server 跳过+dlog,不拖垮 chat。
    // 连接生命周期绑本次调用:registry_tools(持 Arc<McpClient>)在本函数末尾 drop → 子进程被 kill_on_drop 杀。
    let registry_tools = if settings.mcp_servers.is_empty() {
        registry_tools
    } else {
        let mcp_tools = crate::chat::mcp_bridge::connect_mcp_servers(&settings.mcp_servers).await;
        registry_tools.with_mcp(mcp_tools)
    };
    let local_kb = LocalKb::auto_detect(&settings);
    let ctx = ToolContext {
        pool,
        settings: &settings,
        case_id: Some(&input.case_id),
        local_kb: local_kb.as_ref(),
        // reextract_document 工具需要 AppHandle 触发后台抽取并 emit 进度事件
        app: Some(app.clone()),
    };
    // V0.2 D6.5 · 给 citations.parse_with_doc_filenames 用,校验 type=doc 的 quote 是否在文档里
    let mut case_docs_for_citation_check: Vec<(String, String)> = Vec::new();
    for d in &docs {
        if let Some(p) = &d.extracted_text_path {
            if let Ok(text) = tokio::fs::read_to_string(p).await {
                case_docs_for_citation_check.push((d.filename.clone(), text));
            }
        }
    }
    let agent_req = AgentLoopRequest {
        system_prompt: constitution_prompt,
        history: history.clone(),
        user_message: user_message_final.clone(),
        temperature: choice.temperature,
        max_tokens: choice.max_tokens,
        // thinking 模型不支持 tool_choice="required"(DeepSeek 400),降级 auto;详 resolve_tool_choice。
        tool_choice: resolve_tool_choice(task.needs_tools(), &choice.model).into(),
        case_docs_for_citation_check,
    };
    let result: Result<ChatRunFinish, String> =
        run_chat_with_tools(&llm_config, agent_req, &registry_tools, ctx, tx, cancel_rx)
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

    // 等 forward 把 channel 排空,拿回已流式产出的文本(出错时当 partial 落库)
    let streamed_partial = forward.await.unwrap_or_default();
    // 无论成败,清掉 registry(注册的 sender 可能已被消费,这里兜底)
    let _ = registry.take(&input.message_id);

    let latency_ms = started_at.elapsed().as_millis() as u64;

    match result {
        Ok(ChatRunFinish {
            content_cleaned,
            citations: final_citations,
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
            let assistant_content = content_cleaned;
            // ── 9. 决定是否落 artifact ──────────────────────────────
            let mut artifact_doc_id = if let Some(task_str) = task.as_db_str() {
                if assistant_content.chars().count() >= 1500 {
                    match write_chat_artifact(
                        pool,
                        &input.case_id,
                        &assistant_id,
                        task_str,
                        &assistant_content,
                    )
                    .await
                    {
                        Ok(doc_id) => Some(doc_id),
                        Err(e) => {
                            crate::dlog!("[chat] artifact 写盘失败: {}", e);
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // V0.3 D2 · save_artifact(自由聊天起草文书)写的是独立 document,不走上面 task-based
            // 路径,artifact_doc_id 仍为 None。这里补:本轮有成功的 save_artifact 工具调用时,
            // 取该案最新 chat_artifact 文档 id 回传 → 前端据此**自动进 Milkdown 编辑器打开**。
            // (MVP 一轮至多一个 save_artifact,最新即本轮所产;多个时取最新也合理。)
            if artifact_doc_id.is_none()
                && final_tool_calls
                    .iter()
                    .any(|t| t.tool == "save_artifact" && t.success)
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
                    Err(e) => crate::dlog!("[chat] 查 save_artifact doc_id 失败: {}", e),
                }
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
                let _ = crate::db::chat_tasks::finish_chat_task(pool, tid, "done", None).await;
            }

            insert_chat_message(
                pool,
                NewChatMessage {
                    id: &assistant_id,
                    case_id: &input.case_id,
                    role: "assistant",
                    content: &assistant_content,
                    task_type: task.as_db_str(),
                    model: Some(&usage.model),
                    prompt_tokens: usage.prompt_tokens.map(|x| x as i64),
                    completion_tokens: usage.completion_tokens.map(|x| x as i64),
                    latency_ms: Some(latency_ms as i64),
                    based_on: Some(&based_on_json),
                    artifact_doc_id: artifact_doc_id.as_deref(),
                    error_short: None,
                    attached_doc_ids: None,
                    citations_json: citations_json.as_deref(),
                    task_id: chat_task_id.as_deref(),
                },
            )
            .await
            .map_err(|e| format!("入库 assistant 消息失败: {}", e))?;

            // chat 完成后后台增量索引:本轮若调过 get_law_article/get_case_detail,新缓存的
            // 法条/案例补进语义索引(单飞 + 无新增早退,所以多数轮次是廉价 no-op)。
            crate::spawn_kb_auto_index(app.clone());

            Ok(CaseChatResult {
                user_message_id: user_msg_id,
                assistant_message_id: assistant_id,
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
                let terminal = if msg.contains("用户取消") || msg.to_lowercase().contains("cancel")
                {
                    "cancelled"
                } else {
                    "failed"
                };
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
    limit: Option<i64>,
) -> Result<Vec<ChatMessage>, String> {
    list_chat_messages(pool, case_id, limit)
        .await
        .map_err(|e| format!("读取聊天历史失败: {}", e))
}

/// 取消进行中的 chat。`message_id` 必须跟 case_chat 入参一致(=channel 后缀)。
pub fn cancel_chat_impl(registry: &ChatCancelRegistry, message_id: &str) -> bool {
    if let Some(sender) = registry.take(message_id) {
        let _ = sender.send(());
        true
    } else {
        false
    }
}

/// 清空某案件下所有聊天记录(用户主动)。
pub async fn clear_chat_history_impl(pool: &SqlitePool, case_id: &str) -> Result<u64, String> {
    crate::db::chat::delete_chat_history_for_case(pool, case_id)
        .await
        .map_err(|e| format!("清空聊天记录失败: {}", e))
}

// =============================================================================
// 内部 helper
// =============================================================================

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
    // 可读文件名:「法律依据_2026-05-29_092234.md」(到秒,LLM 任务不会同秒重出 → 不冲突)
    let filename = format!(
        "{}_{}.md",
        artifact_display_name(task_type),
        chrono::Local::now().format("%Y-%m-%d_%H%M%S")
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

// =============================================================================
// 测试
// =============================================================================
