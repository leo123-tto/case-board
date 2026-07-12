//! V0.2 D3-D4 主入口:DeepSeek function calling 流式 + 多轮 turn loop + 工具派发。
//!
//! 跟现有 `chat::stream::run_chat`(V0.1.16 无工具简化路径)**并存** —
//! `case_chat_impl` 根据 task_type / attached_doc_ids 路由到两条路径之一。
//!
//! 流程:
//!   1. 拼初始 messages(system + history + user)
//!   2. 发请求到 /beta/chat/completions(strict tools schema)
//!   3. 流式解析:
//!      - delta.content 累积 → 发 ChatStreamEvent::Delta 给前端
//!      - delta.tool_calls 累积 StreamingToolCall 状态机
//!      - finish_reason == "tool_calls" → 派发工具
//!      - finish_reason == "stop" → 结束
//!   4. 工具执行(本轮顺序;并行版放 D4-D5 parallel.rs)
//!   5. 把 assistant 这条 + 每个 tool_result 塞回 messages,进入下一轮
//!   6. LoopGuard 每轮 / 每次 tool 派发前 / LLM 返回 usage 后都查 cap
//!
//! 暂未实现(留给后续阶段):
//!   - parallel.rs 并行 tool 派发(D4-D5)
//!   - hooks.rs 4 个 hook(D5)
//!   - `<CITATIONS>` 解析与协议落库(D5)
//!   - resume_orphaned_chat_tasks(D5.5)
//!   - chat_tasks 表 CRUD(D5.5)

use std::collections::HashMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use std::sync::Arc;
use std::sync::RwLock;

use super::context::TaskType;
use super::hooks::{HookChain, HookContext, HookOutcome, SessionStats};
use super::loop_guard::{LoopGuard, LoopGuardViolation};
use super::stream::{ChatStreamEvent, ChatUsage};
use super::tools::{ToolContext, ToolError, ToolRegistry};
use crate::llm::capability::{OutputTokenParam, ProviderCapability};
use crate::llm::LlmConfig;

/// agent_loop 调用入参(跟 `stream::ChatStreamRequest` 平行,字段略多)。
pub struct AgentLoopRequest {
    pub task_type: TaskType,
    pub system_prompt: String,
    pub history: Vec<(String, String)>,
    pub user_message: String,
    pub temperature: f32,
    pub max_tokens: u32,
    /// "auto" / "required" / "none";固定任务一般用 "required"
    pub tool_choice: String,
    /// 给 `<CITATIONS>` 校验 `type=doc` 引用。值为 `(filename, extracted_text_path)`，
    /// 最终只懒加载真正被引用的文件，避免每轮聊天预读整案全文。
    pub case_doc_paths_for_citation_check: Vec<(String, String)>,
}

/// agent_loop 跑完一次的回执(给 commands.rs 落库 + 反馈 MD 性能埋点)。
#[derive(Debug, Clone, Default)]
pub struct AgentLoopOutput {
    /// 原始 LLM 输出 — 末尾**可能**含 `<CITATIONS>...</CITATIONS>` JSON 块。
    /// 入库前应该用 `content_cleaned`,**不**用本字段。
    pub final_content: String,
    /// V0.2 D6.5 · `<CITATIONS>` 剥离后的纯净 content(给 markdown 渲染)。
    /// 如果 LLM 没写 `<CITATIONS>`,与 `final_content` 相同。
    pub content_cleaned: String,
    /// V0.2 D6.5 · 从 `<CITATIONS>` 解析出的引用列表(type=doc 时已做 quote 校验)。
    pub citations: Vec<super::citations::Citation>,
    pub usage: ChatUsage,
    pub tool_trace: Vec<ToolCallRecord>,
    pub iterations: u32,
    /// V0.2 D5:本会话 hook 累计统计(KB 命中率 / 成本估算 / cache 命中率)
    pub session_stats: SessionStats,
    /// V0.2.2 · 成本/缓存诊断指标(各轮求和),给 agent_metrics.jsonl 落盘分析。
    pub metrics: CostMetrics,
    /// V0.3 · 本轮模型调了 `ask_user` 发起选项式追问 → 这里带回问题列表,
    /// 循环已 break(未派发、未回传 tool_calls)。`None` = 正常收尾。
    /// 前端据此渲染选项卡片;用户回答当作下一条普通 user 消息回灌。
    pub ask_user: Option<Vec<AskQuestion>>,
}

/// 一次 agent_loop 的成本/缓存诊断指标(各轮 token 求和)。给落盘对比缓存命中率用。
#[derive(Debug, Clone, Default, Serialize)]
pub struct CostMetrics {
    /// LLM 轮数(= iterations)
    pub turns: u32,
    /// 各轮 prompt_tokens 求和(= cache_hit + cache_miss)
    pub prompt_tokens: u64,
    /// 各轮 completion_tokens 求和
    pub completion_tokens: u64,
    /// 各轮命中前缀缓存的 input token 求和(便宜)
    pub cache_hit_tokens: u64,
    /// 各轮未命中、全价 input token 求和
    pub cache_miss_tokens: u64,
    /// V0.3.5 · 前缀指纹(system+tools 的 md5 前 12 位):跨 jsonl 记录比对即看出哪轮把前缀缓存打破。
    /// 被动诊断,不影响请求本身;空串 = 未计算。
    pub prefix_fp: String,
    /// system prompt 分量指纹(前 12 位),用于区分「system 漂移 vs 工具集漂移」。
    pub prefix_sys: String,
    /// 工具集分量指纹(前 12 位)。
    pub prefix_tools: String,
}

/// V0.3 · `ask_user` 选项式追问的单个问题(给前端渲染选项卡片)。
/// 由 agent_loop 拦截 `ask_user` 工具调用时从其 args 解析,经 `ChatStreamEvent::AskUser`
/// 与 `AgentLoopOutput.ask_user` 两路带给前端。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskQuestion {
    /// 问题文本
    pub question: String,
    /// 预设选项(可空;空 → 前端只显自由输入框)
    #[serde(default)]
    pub options: Vec<String>,
    /// 是否允许自由输入(选项穷尽不了时为 true;无选项时前端强制可输入)
    #[serde(default)]
    pub allow_input: bool,
}

/// 从 `ask_user` 工具调用的 args 防御式解析出问题列表。
/// 期望形状 `{ "questions": [ {question, options?, allow_input?} ] }`;
/// 任何字段缺失 / 类型不符都跳过该条,question 为空的条目丢弃。**永不 panic**。
fn parse_ask_user_option(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        }
        Value::Object(map) => ["label", "text", "value", "title", "option"]
            .iter()
            .filter_map(|key| map.get(*key))
            .find_map(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        _ => None,
    }
}

fn parse_ask_user_args(args: &Value) -> Vec<AskQuestion> {
    let Some(arr) = args.get("questions").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let q = item.get("question").and_then(|v| v.as_str())?.trim();
            if q.is_empty() {
                return None;
            }
            let options = item
                .get("options")
                .and_then(|v| v.as_array())
                .map(|a| {
                    let mut seen = std::collections::HashSet::new();
                    a.iter()
                        .filter_map(parse_ask_user_option)
                        .filter(|s| seen.insert(s.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let allow_input = item
                .get("allow_input")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(AskQuestion {
                question: q.to_string(),
                options,
                allow_input,
            })
        })
        .collect()
}

/// 单次工具调用的 trace(给前端 ToolCallTrace 组件 + 落 chat_tasks.tool_calls_json)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool: String,
    pub args: Value,
    pub kb_hit: bool,
    pub credits_used: u32,
    pub success: bool,
    pub error_short: Option<String>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
}

#[derive(Debug, Error)]
pub enum AgentLoopError {
    #[error("LLM 不可达:{0}")]
    Network(String),
    #[error("LLM HTTP {0}:{1}")]
    HttpStatus(u16, String),
    #[error("LLM 流式响应解析失败:{0}")]
    Parse(String),
    #[error("用户取消")]
    Cancelled,
    #[error("LoopGuard 触发:{0}")]
    LoopGuard(#[from] LoopGuardViolation),
    #[error("工具调用失败:{0}")]
    Tool(#[from] ToolError),
}

impl serde::Serialize for AgentLoopError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

// ============================================================================
// 请求体 / DeepSeek beta function calling
// ============================================================================

#[derive(Serialize)]
struct ApiRequest<'a> {
    model: &'a str,
    messages: &'a [ApiMessage],
    stream: bool,
    stream_options: StreamOptions,
    temperature: f32,
    #[serde(flatten)]
    token_budget: ApiTokenBudget,
    tools: &'a [Value],
    tool_choice: &'a str,
}

#[derive(Serialize)]
struct ApiTokenBudget {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_completion_tokens: Option<u32>,
}

impl ApiTokenBudget {
    fn from_capability(max_output_tokens: u32, capability: &ProviderCapability) -> Self {
        match capability.output_token_param {
            OutputTokenParam::MaxTokens => Self {
                max_tokens: Some(max_output_tokens),
                max_completion_tokens: None,
            },
            OutputTokenParam::MaxCompletionTokens => Self {
                max_tokens: None,
                max_completion_tokens: Some(max_output_tokens),
            },
        }
    }
}

fn resolve_stream_tool_choice<'a>(
    tool_schemas: &[Value],
    requested: &'a str,
    capability: &ProviderCapability,
) -> &'a str {
    if tool_schemas.is_empty() {
        // 强制收尾轮没有工具时必须显式 none,避免 provider 收到 required+空工具报 400。
        "none"
    } else if requested == "required" && !capability.supports_tool_choice_required {
        "auto"
    } else {
        requested
    }
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum ApiMessage {
    Plain {
        role: String,
        content: String,
    },
    AssistantWithToolCalls {
        role: String,
        content: Option<String>,
        /// V0.2 · thinking 模型(deepseek-v4-pro)做工具调用时,本轮 reasoning_content
        /// 必须随该 assistant 消息回传,否则后续请求 DeepSeek 400
        /// ("reasoning_content in the thinking mode must be passed back")。
        #[serde(skip_serializing_if = "Option::is_none")]
        reasoning_content: Option<String>,
        tool_calls: Vec<ApiToolCall>,
    },
    ToolResult {
        role: String,
        tool_call_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize)]
struct ApiToolCall {
    id: String,
    r#type: String,
    function: ApiFunctionCall,
}

#[derive(Debug, Clone, Serialize)]
struct ApiFunctionCall {
    name: String,
    arguments: String,
}

// ============================================================================
// SSE 解析(独立于 stream.rs,因为要解析 tool_calls + finish_reason)
// ============================================================================

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<StreamUsage>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Deserialize, Default)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCallDelta>>,
    #[serde(default)]
    reasoning_content: Option<String>,
}

#[derive(Deserialize)]
struct StreamToolCallDelta {
    #[serde(default)]
    index: u32,
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "type")]
    _ty: Option<String>,
    #[serde(default)]
    function: Option<StreamFunctionDelta>,
}

#[derive(Deserialize)]
struct StreamFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct StreamUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    #[serde(default)]
    prompt_cache_hit_tokens: Option<u64>,
    #[serde(default)]
    prompt_cache_miss_tokens: Option<u64>,
    #[serde(default)]
    reasoning_tokens: Option<u64>,
}

// ============================================================================
// StreamingToolCall 状态机(多 chunk 拼 arguments)
// ============================================================================

#[derive(Debug, Default)]
struct StreamingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments_buf: String,
}

impl StreamingToolCall {
    fn merge(&mut self, d: &StreamToolCallDelta) {
        if let Some(id) = &d.id {
            self.id = Some(id.clone());
        }
        if let Some(f) = &d.function {
            if let Some(n) = &f.name {
                self.name = Some(n.clone());
            }
            if let Some(a) = &f.arguments {
                self.arguments_buf.push_str(a);
            }
        }
    }

    fn build(self) -> Result<FinishedToolCall, AgentLoopError> {
        let id = self
            .id
            .ok_or_else(|| AgentLoopError::Parse("tool_call 缺 id".into()))?;
        let name = self
            .name
            .ok_or_else(|| AgentLoopError::Parse("tool_call 缺 name".into()))?;
        let args: Value = if self.arguments_buf.trim().is_empty() {
            json!({})
        } else {
            // 正常路径:严格解析,行为与旧逻辑完全一致(零开销、不进修复阶梯)。
            match serde_json::from_str::<Value>(&self.arguments_buf) {
                Ok(v) => v,
                // 流式 SSE 把参数 JSON 切坏了 —— 跑确定性修复阶梯而不是炸整轮(strategy A)。
                Err(strict_err) => {
                    let repaired = super::arg_repair::repair(&self.arguments_buf).map_err(|e| {
                        AgentLoopError::Parse(format!(
                            "tool_call arguments 无法修复({}): {}",
                            name, e
                        ))
                    })?;
                    // 仅记错误形状(serde 错只含行列号,不含参数内容),不落 arguments_buf 防泄案件内容。
                    crate::dlog!(
                        "agent_loop: tool_call({}) arguments 流式损坏已确定性修复(strict err: {})",
                        name,
                        strict_err
                    );
                    repaired
                }
            }
        };
        Ok(FinishedToolCall { id, name, args })
    }
}

#[derive(Debug, Clone)]
struct FinishedToolCall {
    id: String,
    name: String,
    args: Value,
}

// ============================================================================
// 主入口
// ============================================================================

const MAX_REQUEST_RETRIES: usize = 3;

/// V0.2.2 · 达 LoopGuard 最大轮数时,发给 LLM 的"强制收尾"指令。
/// 关键:必须保留反虚构底线 —— 没查全/没核实的东西要明说,绝不能编造法条或判例。
const FORCE_FINISH_PROMPT: &str = "已达到本次会话的最大检索轮数,不能再调用任何工具。\
请立即基于以上已经获取到的信息,给出尽可能完整、有条理的最终答复。\
重要:凡是你尚未核实、未能查全或不确定的法条编号、条文内容、案例或事实,\
必须明确标注「未能核实」或「需进一步核查」,严禁编造或杜撰任何法规、条文、判例或事实。\
诚实的部分结论优于虚构的完整结论。";

/// 跑一次带工具的 chat 多轮循环。
pub async fn run_chat_with_tools(
    config: &LlmConfig,
    req: AgentLoopRequest,
    registry: &ToolRegistry,
    ctx: ToolContext<'_>,
    tx: UnboundedSender<ChatStreamEvent>,
    mut cancel: oneshot::Receiver<()>,
) -> Result<AgentLoopOutput, AgentLoopError> {
    let mut guard = LoopGuard::from_settings_for_task(ctx.settings, req.task_type);
    let mut messages = build_initial_messages(&req);
    let tool_schemas = registry.to_function_schemas();
    // V0.3.5 · 前缀缓存稳定性:被动算一次 system+tools 指纹,落 metrics 供离线看漂移(绝不改请求本身)。
    let prefix_fp =
        super::prefix_cache::PrefixFingerprint::compute(&req.system_prompt, &tool_schemas);
    let mut full_content = String::new();
    let mut usage = ChatUsage::default();
    let mut tool_trace: Vec<ToolCallRecord> = Vec::new();
    // V0.3 · 本轮若模型调 ask_user 发起选项式追问,拦截后存这里并 break(不派发、不回传 tool_calls)
    let mut ask_user_questions: Option<Vec<AskQuestion>> = None;
    // V0.2.2 · 成本/缓存指标各轮累加
    let mut m_prompt = 0u64;
    let mut m_completion = 0u64;
    let mut m_cache_hit = 0u64;
    let mut m_cache_miss = 0u64;
    // 2026-06-16(整合外部 PR #16 @MaxLijian):兼容后端(glm/mimo/custom/OpenRouter 等)不走
    // /beta 路径(它们不支持 → 实调 chat 报 404,但验证走原始 endpoint 故能通过)。
    let is_compat = ctx.settings.cloud_llm_is_compat();
    let endpoint = beta_endpoint(&config.endpoint, is_compat);

    // V0.2 D5:hook chain + session 统计共享
    let session = Arc::new(RwLock::new(SessionStats::default()));
    let chain = HookChain::default_v0_2();
    let hctx = HookContext::new(
        ctx.pool,
        ctx.settings,
        ctx.case_id,
        None, // V0.2 D5 暂不带 task_id;D5.5 加 chat_tasks 表 CRUD 时一起接
        session.clone(),
    );

    loop {
        // V0.2.2 · 达最大检索轮数:不再直接 abort 丢答案。发一次"强制收尾轮"
        // (去掉所有工具 + 反虚构指令),让 LLM 基于已获取信息给最终答复。
        if guard.check_iter_cap().is_err() {
            crate::dlog!(
                "agent_loop: 达最大轮数 max={} → 强制收尾轮(去工具)",
                guard.iter_count()
            );
            messages.push(ApiMessage::Plain {
                role: "user".into(),
                content: FORCE_FINISH_PROMPT.into(),
            });
            match stream_one_request(
                &endpoint,
                config,
                &messages,
                &req,
                &[],
                &tx,
                &mut cancel,
                &mut guard,
            )
            .await
            {
                Ok(o) => {
                    full_content.push_str(&o.content);
                    merge_usage(&mut usage, &o.usage_chunk);
                    m_prompt += o.usage_chunk.prompt_tokens.unwrap_or(0);
                    m_completion += o.usage_chunk.completion_tokens.unwrap_or(0);
                    m_cache_hit += o.usage_chunk.cache_hit_tokens.unwrap_or(0);
                    m_cache_miss += o.usage_chunk.cache_miss_tokens.unwrap_or(0);
                }
                Err(e) => {
                    crate::dlog!("agent_loop: 强制收尾轮失败 → {}", e);
                    // 收尾失败:有半截内容就保留半截,否则透传真错(别静默吞)
                    if full_content.trim().is_empty() {
                        return Err(e);
                    }
                }
            }
            break;
        }
        guard.check_duration_cap()?;
        guard.check_idle_cap()?;

        // 1) 跑一次流式请求,拿 (content_delta, tool_calls, finish_reason, usage_chunk)
        let turn_started = std::time::Instant::now();
        let one = match stream_one_request(
            &endpoint,
            config,
            &messages,
            &req,
            &tool_schemas,
            &tx,
            &mut cancel,
            &mut guard,
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                // 诊断:哪一轮、跑了多久、请求多大、什么错 —— elapsed≈超时阈值=客户端超时,
                // elapsed 很短=服务端/网关断流。落 dlog 给反馈 MD 带出来。
                crate::dlog!(
                    "agent_loop: 第 {} 轮请求失败 elapsed={:.1}s model={} msgs={} → {}",
                    guard.iter_count(),
                    turn_started.elapsed().as_secs_f64(),
                    config.model,
                    messages.len(),
                    e
                );
                return Err(e);
            }
        };
        // 诊断:本轮 DeepSeek 前缀缓存命中情况(优化成本的关键指标;命中价约输入价 1/120)
        let ch = one.usage_chunk.cache_hit_tokens.unwrap_or(0);
        let cm = one.usage_chunk.cache_miss_tokens.unwrap_or(0);
        m_cache_hit += ch;
        m_cache_miss += cm;
        m_prompt += one.usage_chunk.prompt_tokens.unwrap_or(0);
        m_completion += one.usage_chunk.completion_tokens.unwrap_or(0);
        let hit_pct = if ch + cm > 0 {
            ch as f64 / (ch + cm) as f64 * 100.0
        } else {
            0.0
        };
        crate::dlog!(
            "agent_loop: 第 {} 轮完成 elapsed={:.1}s finish={} tool_calls={} content_len={} \
             cache_hit={} miss={} hit={:.0}%",
            guard.iter_count(),
            turn_started.elapsed().as_secs_f64(),
            one.finish_reason.as_deref().unwrap_or("?"),
            one.tool_calls.len(),
            one.content.len(),
            ch,
            cm,
            hit_pct
        );

        full_content.push_str(&one.content);
        merge_usage(&mut usage, &one.usage_chunk);
        if let Some(rt) = one.usage_chunk.reasoning_tokens {
            guard.add_reasoning_tokens(rt)?;
        }

        match one.finish_reason.as_deref() {
            Some("tool_calls") => {
                // V0.3 · 选项式追问拦截:模型本轮若调了 `ask_user`,**不派发、不回传 tool_calls**,
                // 而是把问题抛回前端等用户回答。break 后存的是纯文本 assistant 消息(引导语),
                // 下一轮 user 回答自带「问→答」编号 —— replay 时没有孤儿 tool_call,无 400 风险。
                // 若同轮还混着别的工具调用,一律忽略(等用户答完模型下一轮重新决策)。
                if let Some(ask_tc) = one.tool_calls.iter().find(|tc| tc.name == "ask_user") {
                    let questions = parse_ask_user_args(&ask_tc.args);
                    if questions.is_empty() {
                        // 解析不出有效问题(模型乱填):不拦截,退回正常工具派发路径兜底。
                        crate::dlog!("agent_loop: ask_user 参数解析为空,退回正常派发");
                    } else {
                        // assistant 气泡只留一句引导语;问题清单走选项卡片,不抄进正文(免看两遍)。
                        if full_content.trim().is_empty() {
                            full_content.push_str("为把这份内容写准确,我需要先和你确认几点 👇");
                        }
                        guard.note_progress();
                        let _ = tx.send(ChatStreamEvent::AskUser {
                            questions: questions.clone(),
                        });
                        ask_user_questions = Some(questions);
                        break;
                    }
                }
                // assistant 这轮(可能含 partial content + tool_calls)塞回 messages
                let tool_calls = one
                    .tool_calls
                    .iter()
                    .map(|tc| ApiToolCall {
                        id: tc.id.clone(),
                        r#type: "function".to_string(),
                        function: ApiFunctionCall {
                            name: tc.name.clone(),
                            arguments: serde_json::to_string(&tc.args)
                                .unwrap_or_else(|_| "{}".into()),
                        },
                    })
                    .collect();
                messages.push(ApiMessage::AssistantWithToolCalls {
                    role: "assistant".into(),
                    content: if one.content.is_empty() {
                        None
                    } else {
                        Some(one.content.clone())
                    },
                    // thinking 模型本轮做了工具调用 → 必须回传 reasoning_content;
                    // 即使本轮 reasoning 为空也回传空串,避免 DeepSeek 400 复发。
                    reasoning_content: Some(one.reasoning_content.clone().unwrap_or_default()),
                    tool_calls,
                });

                // V0.2 D4-D5.D · 派发 tool 改用 parallel.rs 并发执行(allow 部分失败)
                // 注:重复调用 dedupe 检查下移到构造 subtasks 的循环里,改为"软拒绝"
                // (塞回提示而非 abort 丢答案,见下方)。
                guard.check_duration_cap()?;

                // V0.2 D5 · 先跑 before_tool_call hook(熔断 / Deny);Deny 的工具
                // 直接构造 deny ToolResult,**不进 parallel 派发**,但 LLM 仍能看到失败原因
                let mut subtasks: Vec<super::parallel::Subtask> = Vec::new();
                // (tool_call_id, deny_msg) — 派发后跟 parallel 结果合并回 messages
                let mut denied: Vec<(String, String, String, serde_json::Value)> = Vec::new();
                for fc in &one.tool_calls {
                    // V0.2.2 · 同 tool + 同参数重复调用:不再 abort 整个会话丢答案,
                    // 当作一次"软拒绝"走 denied 路径 → 仍 push 合成 ToolResult,避免
                    // assistant tool_call 无匹配 result 触发 DeepSeek 400。让 LLM 换参数/
                    // 换工具或直接收尾;真死循环由 iter_cap 强制收尾兜底。
                    if guard.check_duplicate_tool_call(&fc.name, &fc.args).is_err() {
                        denied.push((
                            fc.id.clone(),
                            fc.name.clone(),
                            format!(
                                "你已用完全相同的参数调用过 `{}`,结果见前文对应的 tool 消息,\
                                 请勿重复同一次查询。若已有信息足够,请直接给出结论;\
                                 若仍不足,请换不同参数或换工具。",
                                fc.name
                            ),
                            fc.args.clone(),
                        ));
                        continue;
                    }
                    match chain.run_before_tool_call(&fc.name, &fc.args, &hctx).await {
                        HookOutcome::Continue => subtasks.push(super::parallel::Subtask {
                            tool_call_id: fc.id.clone(),
                            tool: fc.name.clone(),
                            args: fc.args.clone(),
                        }),
                        HookOutcome::Deny(reason) => {
                            denied.push((fc.id.clone(), fc.name.clone(), reason, fc.args.clone()));
                        }
                    }
                }
                let sub_results =
                    super::parallel::run_parallel_subtasks(subtasks, registry, &ctx).await;

                // V0.2 D5 · after_tool_call hook 统计累加(KB 命中率 / credits 记账)
                for sr in &sub_results {
                    let rt = super::tools::ToolResult {
                        content: sr.content.clone(),
                        yuandian_credits_used: sr.credits_used,
                        kb_hit: sr.kb_hit,
                    };
                    chain
                        .run_after_tool_call(&sr.tool, &rt, sr.success, &hctx)
                        .await;
                }

                // 合并 sub_results + denied 回填 messages(顺序按原 tool_calls 顺序)
                let now_ms = chrono::Local::now().timestamp_millis();
                for fc in one.tool_calls {
                    if let Some(sr) = sub_results.iter().find(|s| s.tool_call_id == fc.id) {
                        messages.push(ApiMessage::ToolResult {
                            role: "tool".into(),
                            tool_call_id: sr.tool_call_id.clone(),
                            content: sr.content.clone(),
                        });
                        let rec = ToolCallRecord {
                            tool: sr.tool.clone(),
                            args: sr.args.clone(),
                            kb_hit: sr.kb_hit,
                            credits_used: sr.credits_used,
                            success: sr.success,
                            error_short: sr.error_short.clone(),
                            started_at_ms: sr.started_at_ms,
                            finished_at_ms: sr.finished_at_ms,
                        };
                        let _ = tx.send(ChatStreamEvent::ToolCall {
                            record: rec.clone(),
                        });
                        guard.note_progress();
                        tool_trace.push(rec);
                    } else if let Some((id, tool, reason, args)) =
                        denied.iter().find(|(id, ..)| id == &fc.id)
                    {
                        let content = serde_json::to_string(&json!({"error": reason}))
                            .unwrap_or_else(|_| format!("{{\"error\":\"{}\"}}", reason));
                        messages.push(ApiMessage::ToolResult {
                            role: "tool".into(),
                            tool_call_id: id.clone(),
                            content,
                        });
                        let rec = ToolCallRecord {
                            tool: tool.clone(),
                            args: args.clone(),
                            kb_hit: false,
                            credits_used: 0,
                            success: false,
                            error_short: Some(reason.clone()),
                            started_at_ms: now_ms,
                            finished_at_ms: now_ms,
                        };
                        let _ = tx.send(ChatStreamEvent::ToolCall {
                            record: rec.clone(),
                        });
                        guard.note_progress();
                        tool_trace.push(rec);
                    } else {
                        // V0.2.2 · 兜底:某 tool_call 既不在 sub_results 也不在 denied
                        //(理论不该发生,但若发生会缺 ToolResult → DeepSeek 400)。
                        // 回填 internal_error,保证每个 tool_call 都有匹配的 result。
                        crate::dlog!(
                            "agent_loop: tool_call_id={} 无派发结果也无 deny,回填 internal_error",
                            fc.id
                        );
                        messages.push(ApiMessage::ToolResult {
                            role: "tool".into(),
                            tool_call_id: fc.id.clone(),
                            content: "{\"error\":\"内部错误:工具结果丢失\"}".into(),
                        });
                    }
                }
                // 下一轮
                continue;
            }
            Some("stop") | None => {
                // 最终 — 完整内容已在 full_content 累积
                break;
            }
            Some("length") => {
                // 超 max_tokens,可能被截断;不重试,告诉前端
                crate::dlog!("agent_loop: finish_reason=length,本轮被 max_tokens 截断");
                break;
            }
            Some(other) => {
                return Err(AgentLoopError::Parse(format!(
                    "未知 finish_reason:{}",
                    other
                )));
            }
        }
    }

    // D4-1:usage 在多轮里被 merge_usage 覆盖成"最后一轮",这里回填为整次会话累计
    // (m_prompt/m_completion 已逐轮累加,含强制收尾轮),让成本 hook / Done 事件 / DB 记账都不再少算。
    // model 保留最后一轮(merge_usage 已设)。
    usage.prompt_tokens = Some(m_prompt);
    usage.completion_tokens = Some(m_completion);

    // V0.2 D5 · LLM 调用结束:走 after_llm_call hook(成本估算 + cache stats)
    chain.run_after_llm_call(&usage, &hctx).await;

    let _ = tx.send(ChatStreamEvent::Done {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        model: usage.model.clone(),
    });
    let session_stats = session.read().map(|s| s.clone()).unwrap_or_default();

    // V0.2 D6.5 · 切出 <CITATIONS> 块,校验 doc quote
    let parsed = super::citations::parse_with_doc_paths(
        &full_content,
        &req.case_doc_paths_for_citation_check,
    );

    Ok(AgentLoopOutput {
        final_content: full_content,
        content_cleaned: parsed.content_cleaned,
        citations: parsed.citations,
        usage,
        tool_trace,
        iterations: guard.iter_count(),
        session_stats,
        metrics: CostMetrics {
            turns: guard.iter_count(),
            prompt_tokens: m_prompt,
            completion_tokens: m_completion,
            cache_hit_tokens: m_cache_hit,
            cache_miss_tokens: m_cache_miss,
            prefix_fp: prefix_fp.short().to_string(),
            prefix_sys: prefix_fp.system_short().to_string(),
            prefix_tools: prefix_fp.tools_short().to_string(),
        },
        ask_user: ask_user_questions,
    })
}

// ============================================================================
// 内部:一次流式请求
// ============================================================================

struct OneStreamPass {
    content: String,
    /// thinking 模型本轮思维链(reasoning_content delta 累积);非 thinking 模型为 None。
    reasoning_content: Option<String>,
    tool_calls: Vec<FinishedToolCall>,
    finish_reason: Option<String>,
    usage_chunk: ChunkUsage,
}

#[derive(Default)]
struct ChunkUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    cache_hit_tokens: Option<u64>,
    cache_miss_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    model: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn stream_one_request(
    endpoint: &str,
    config: &LlmConfig,
    messages: &[ApiMessage],
    req: &AgentLoopRequest,
    tool_schemas: &[Value],
    tx: &UnboundedSender<ChatStreamEvent>,
    cancel: &mut oneshot::Receiver<()>,
    guard: &mut LoopGuard,
) -> Result<OneStreamPass, AgentLoopError> {
    let capability = ProviderCapability::from_backend("", endpoint, &config.model);
    let tool_choice =
        resolve_stream_tool_choice(tool_schemas, req.tool_choice.as_str(), &capability);
    let body = ApiRequest {
        model: &config.model,
        messages,
        stream: true,
        stream_options: StreamOptions {
            include_usage: true,
        },
        temperature: capability.normalize_temperature(req.temperature),
        token_budget: ApiTokenBudget::from_capability(req.max_tokens, &capability),
        tools: tool_schemas,
        tool_choice,
    };

    // 流式思考模型(deepseek-v4-pro 默认开思考)单轮可能很慢:回传 reasoning_content
    // 后请求体随轮次增大,首字节延迟(TTFB)更高。用 connect + read(空闲)超时,
    // **不**用总超时 —— 总超时会把还在持续吐 token 的健康长流误杀成
    // "error decoding response body";read_timeout 只在流真正卡死(两次读间隔超时)才触发。
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(guard.idle_timeout_secs().max(30)))
        .build()
        .map_err(|e| AgentLoopError::Network(e.to_string()))?;

    // 简化版 retry:整个请求最多 send 3 次(429 / 5xx / 网络错都重试)
    let mut last_err: Option<AgentLoopError> = None;
    for attempt in 0..MAX_REQUEST_RETRIES {
        let mut request = client.post(endpoint).json(&body);
        if let Some(key) = &config.api_key {
            request = request.bearer_auth(key);
        }
        let send_res = tokio::select! {
            biased;
            _ = &mut *cancel => return Err(AgentLoopError::Cancelled),
            r = request.send() => r,
        };
        let response = match send_res {
            Ok(r) => r,
            Err(e) => {
                crate::dlog!(
                    "agent_loop: 请求发送失败(attempt {}/{}):{} — 重试",
                    attempt + 1,
                    MAX_REQUEST_RETRIES,
                    e
                );
                last_err = Some(AgentLoopError::Network(e.to_string()));
                tokio::time::sleep(Duration::from_millis(300 * (1 << attempt))).await;
                continue;
            }
        };
        let status = response.status();
        if !status.is_success() {
            let raw = response.text().await.unwrap_or_default();
            let snippet: String = raw.chars().take(800).collect();
            if status.as_u16() == 401 || status.as_u16() == 403 {
                return Err(AgentLoopError::HttpStatus(status.as_u16(), snippet));
            }
            if status.as_u16() == 429 || status.is_server_error() {
                crate::dlog!(
                    "agent_loop: HTTP {} (attempt {}/{}) — 重试",
                    status.as_u16(),
                    attempt + 1,
                    MAX_REQUEST_RETRIES
                );
                last_err = Some(AgentLoopError::HttpStatus(status.as_u16(), snippet));
                tokio::time::sleep(Duration::from_millis(1000 * (1 << attempt))).await;
                continue;
            }
            // 4xx 其他(strict schema 不通过) — 不重试,透传
            return Err(AgentLoopError::HttpStatus(status.as_u16(), snippet));
        }

        // 成功 → 解析流
        return parse_stream(response, tx, cancel, guard).await;
    }
    Err(last_err.unwrap_or_else(|| AgentLoopError::Network("请求重试用尽".into())))
}

async fn parse_stream(
    response: reqwest::Response,
    tx: &UnboundedSender<ChatStreamEvent>,
    cancel: &mut oneshot::Receiver<()>,
    guard: &mut LoopGuard,
) -> Result<OneStreamPass, AgentLoopError> {
    use futures::StreamExt;
    let mut stream = response.bytes_stream();
    let mut buf = String::new();
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls_map: HashMap<u32, StreamingToolCall> = HashMap::new();
    let mut finish_reason: Option<String> = None;
    let mut usage_chunk = ChunkUsage::default();

    loop {
        tokio::select! {
            biased;
            _ = &mut *cancel => return Err(AgentLoopError::Cancelled),
            chunk = stream.next() => match chunk {
                None => break,
                Some(Err(e)) => return Err(AgentLoopError::Network(e.to_string())),
                Some(Ok(bytes)) => {
                    guard.note_progress();
                    guard.check_duration_cap()?;
                    guard.check_idle_cap()?;
                    let s = match std::str::from_utf8(&bytes) {
                        Ok(s) => s.to_string(),
                        Err(_) => String::from_utf8_lossy(&bytes).into_owned(),
                    };
                    buf.push_str(&s);
                    while let Some(idx) = buf.find("\n\n") {
                        let raw_event = buf[..idx].to_string();
                        buf = buf[idx + 2..].to_string();
                        if handle_sse_event(
                            &raw_event,
                            tx,
                            &mut content,
                            &mut reasoning,
                            &mut tool_calls_map,
                            &mut finish_reason,
                            &mut usage_chunk,
                        ) {
                            // 看到 [DONE]
                            break;
                        }
                    }
                }
            }
        }
    }

    // 思考模型单轮可能纯推理几分钟,落日志方便诊断「是否真卡死」(连不上会先报 Network 错)。
    if !reasoning.is_empty() {
        crate::dlog!(
            "[agent_loop] 本轮 reasoning_content {} 字,content {} 字,tool_calls {}",
            reasoning.chars().count(),
            content.chars().count(),
            tool_calls_map.len()
        );
    }

    // 收尾 tool_calls map → Vec(按 index 升序)
    let mut indexed: Vec<(u32, StreamingToolCall)> = tool_calls_map.into_iter().collect();
    indexed.sort_by_key(|(i, _)| *i);
    let mut tool_calls = Vec::with_capacity(indexed.len());
    for (_, sc) in indexed {
        tool_calls.push(sc.build()?);
    }
    Ok(OneStreamPass {
        content,
        reasoning_content: if reasoning.is_empty() {
            None
        } else {
            Some(reasoning)
        },
        tool_calls,
        finish_reason,
        usage_chunk,
    })
}

/// 处理一条 SSE 事件,**返回 true 表示流应该结束([DONE])**。
fn handle_sse_event(
    raw: &str,
    tx: &UnboundedSender<ChatStreamEvent>,
    content_acc: &mut String,
    reasoning_acc: &mut String,
    tool_calls: &mut HashMap<u32, StreamingToolCall>,
    finish_reason: &mut Option<String>,
    usage_chunk: &mut ChunkUsage,
) -> bool {
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if data == "[DONE]" {
            return true;
        }
        let Ok(chunk) = serde_json::from_str::<StreamChunk>(data) else {
            continue;
        };
        if let Some(m) = chunk.model {
            usage_chunk.model = Some(m);
        }
        if let Some(u) = chunk.usage {
            if u.prompt_tokens.is_some() {
                usage_chunk.prompt_tokens = u.prompt_tokens;
            }
            if u.completion_tokens.is_some() {
                usage_chunk.completion_tokens = u.completion_tokens;
            }
            if u.prompt_cache_hit_tokens.is_some() {
                usage_chunk.cache_hit_tokens = u.prompt_cache_hit_tokens;
            }
            if u.prompt_cache_miss_tokens.is_some() {
                usage_chunk.cache_miss_tokens = u.prompt_cache_miss_tokens;
            }
            if u.reasoning_tokens.is_some() {
                usage_chunk.reasoning_tokens = u.reasoning_tokens;
            }
        }
        for choice in chunk.choices {
            if let Some(fr) = choice.finish_reason {
                // 只取首个非空 finish_reason:正常一次请求只出现一次;若服务端异常发多个
                //(如先 tool_calls 后 stop),保留首个,避免工具调用信息被后续覆盖丢失 → 400。
                if finish_reason.is_none() {
                    *finish_reason = Some(fr);
                }
            }
            if let Some(text) = choice.delta.content {
                if !text.is_empty() {
                    content_acc.push_str(&text);
                    let _ = tx.send(ChatStreamEvent::Delta { text });
                }
            }
            if let Some(deltas) = choice.delta.tool_calls {
                for d in deltas {
                    let idx = d.index;
                    let entry = tool_calls.entry(idx).or_default();
                    entry.merge(&d);
                }
            }
            // thinking 模型思维链:累积起来,本轮做工具调用时必须随 assistant 消息回传
            // (DeepSeek 思考模式工具调用强约束)。不进 content;但发 Reasoning 事件给前端做
            // 「深度推理中…(N 字)」进度反馈 —— 否则大上下文单轮推理几分钟里 UI 零反馈像卡死。
            if let Some(rc) = choice.delta.reasoning_content {
                if !rc.is_empty() {
                    reasoning_acc.push_str(&rc);
                    let _ = tx.send(ChatStreamEvent::Reasoning { text: rc });
                }
            }
        }
    }
    false
}

// ============================================================================
// 内部:杂项 helper
// ============================================================================

fn build_initial_messages(req: &AgentLoopRequest) -> Vec<ApiMessage> {
    let mut msgs = Vec::with_capacity(2 + req.history.len());
    msgs.push(ApiMessage::Plain {
        role: "system".into(),
        content: req.system_prompt.clone(),
    });
    for (role, content) in &req.history {
        msgs.push(ApiMessage::Plain {
            role: role.clone(),
            content: content.clone(),
        });
    }
    msgs.push(ApiMessage::Plain {
        role: "user".into(),
        content: req.user_message.clone(),
    });
    msgs
}

/// 把用户在 Settings 填的 cloud_llm_endpoint 自动补到 `/beta/chat/completions`(支持工具调用)。
/// 已经以 `/beta/chat/completions` / `/v1/chat/completions` 结尾的不动 — 前者直接用,后者
/// V0.2 chat 切到 beta(老 stream::run_chat 仍走 v1)。
///
/// 2026-06-16(整合外部 PR #16 @MaxLijian):兼容后端(glm/mimo/custom/OpenRouter 等)不走
/// beta 路径(它们不支持 → 404)。只有 DeepSeek 等原生支持 beta 端点的后端才自动转换。
fn beta_endpoint(current: &str, is_compat_backend: bool) -> String {
    // 兼容后端不走 beta 路径(它们不支持),原样返回
    if is_compat_backend {
        return current.to_string();
    }
    // 2026-06-15:MiniMax 自有协议路径(/v1/text/chatcompletion_v2)就是工具调用路径,
    // **绝不能**再加 /beta 后缀(会 404)。原样返回。
    if current.contains("chatcompletion_v2") {
        return current.to_string();
    }
    if current.ends_with("/beta/chat/completions") {
        return current.to_string();
    }
    // 老的 /v1/chat/completions → 替换为 /beta/chat/completions
    if let Some(base) = current.strip_suffix("/v1/chat/completions") {
        return format!("{}/beta/chat/completions", base);
    }
    if current.ends_with('/') {
        format!("{}beta/chat/completions", current)
    } else {
        format!("{}/beta/chat/completions", current)
    }
}

fn merge_usage(dst: &mut ChatUsage, src: &ChunkUsage) {
    if let Some(n) = src.prompt_tokens {
        dst.prompt_tokens = Some(n);
    }
    if let Some(n) = src.completion_tokens {
        dst.completion_tokens = Some(n);
    }
    if let Some(m) = &src.model {
        dst.model = m.clone();
    }
}

// ============================================================================
// 测试(单元测,不联网)
// ============================================================================
