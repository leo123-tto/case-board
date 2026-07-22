//! V0.2 D4-D5.C · 并行子任务调度(详 § 6.3)。
//!
//! 主要 use case:agent_loop 一轮里 LLM 返回多个 tool_calls,本模块用 `futures::join_all`
//! **并发**派发(IO-bound 工具调用同时跑,显著缩短整体耗时)。允许部分失败 — 失败的
//! 在结果里标 ⚠️,不阻塞整体(对应 § 6.3 注意事项:**用 join!,不用 try_join!**)。
//!
//! 跟 agent_loop 关系:
//!   - agent_loop 决定本轮要调哪几个工具(LLM 自己规划)
//!   - 把工具调用清单包成 `Vec<Subtask>` 交给本模块
//!   - 本模块返回 `Vec<SubtaskResult>`,agent_loop 把每个结果填回 messages
//!
//! 注意:这里**不用 tokio::spawn**(跨线程)— 因为 `ToolContext` 含 `&SqlitePool` 等
//! 借用,Future 不是 `'static`。改用 `futures::future::join_all` 在同 task 里 cooperative
//! 并发,IO 等待自然并行(reqwest / sqlx 都是 async I/O)。

use serde::Serialize;
use serde_json::Value;

use super::tools::{Tool, ToolContext, ToolError, ToolRegistry};

/// 子任务定义(对应 LLM 一次 tool_call)。
#[derive(Debug, Clone)]
pub struct Subtask {
    /// LLM 给的 tool_call_id(回填 messages 时用,**必须一致**)
    pub tool_call_id: String,
    /// 工具名,从 ToolRegistry 找
    pub tool: String,
    /// 解析过的 args(LLM function_call.arguments JSON)
    pub args: Value,
}

/// 子任务结果。
#[derive(Debug, Clone, Serialize)]
pub struct SubtaskResult {
    pub tool_call_id: String,
    pub tool: String,
    pub args: Value,
    pub success: bool,
    /// 给 LLM 回填 messages 的 content(成功是工具返回 JSON,失败是 `{"error": ...}`)
    pub content: String,
    pub kb_hit: bool,
    pub credits_used: u32,
    pub error_short: Option<String>,
    /// 仅保留可向用户展示的审计摘要（公开 URL / 法规名和条号），不落整段工具正文。
    pub result_preview: Option<Value>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
}

fn collect_public_links(value: &Value, links: &mut Vec<Value>) {
    if links.len() >= 10 {
        return;
    }
    match value {
        Value::Object(map) => {
            if let Some(url) = map
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
            {
                if !links
                    .iter()
                    .any(|item| item.get("url").and_then(Value::as_str) == Some(url))
                {
                    let title = ["title", "name", "source"]
                        .iter()
                        .find_map(|key| map.get(*key).and_then(Value::as_str))
                        .unwrap_or(url);
                    links.push(serde_json::json!({ "title": title, "url": url }));
                }
            }
            for child in map.values() {
                collect_public_links(child, links);
                if links.len() >= 10 {
                    break;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_public_links(item, links);
                if links.len() >= 10 {
                    break;
                }
            }
        }
        _ => {}
    }
}

fn collect_law_hits(value: &Value, laws: &mut Vec<Value>) {
    if laws.len() >= 10 {
        return;
    }
    match value {
        Value::Object(map) => {
            if let Some(name) = map.get("fgmc").and_then(Value::as_str) {
                let article = ["ftnum", "tid", "ft_num"]
                    .iter()
                    .find_map(|key| map.get(*key).and_then(Value::as_str));
                let excerpt = map
                    .get("content")
                    .and_then(Value::as_str)
                    .map(|content| content.chars().take(1_200).collect::<String>());
                let duplicate = laws.iter().any(|item| {
                    item.get("law").and_then(Value::as_str) == Some(name)
                        && item.get("article").and_then(Value::as_str) == article
                });
                if !duplicate {
                    laws.push(serde_json::json!({
                        "law": name,
                        "article": article,
                        "excerpt": excerpt,
                    }));
                }
            }
            for child in map.values() {
                collect_law_hits(child, laws);
                if laws.len() >= 10 {
                    break;
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_law_hits(item, laws);
                if laws.len() >= 10 {
                    break;
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn tool_result_preview(tool: &str, content: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(content).ok()?;
    if matches!(
        tool,
        "web_search"
            | "web_fetch"
            | "exa_search"
            | "exa_contents"
            | "exa_find_similar"
            | "firecrawl_search"
            | "firecrawl_scrape"
    ) {
        let mut links = Vec::new();
        collect_public_links(&value, &mut links);
        return (!links.is_empty()).then(|| serde_json::json!({ "results": links }));
    }
    if matches!(
        tool,
        "search_laws"
            | "get_law_article"
            | "search_regulations"
            | "get_regulation_detail"
            | "law_vector_search"
    ) {
        let mut laws = Vec::new();
        collect_law_hits(&value, &mut laws);
        return (!laws.is_empty()).then(|| serde_json::json!({ "laws": laws }));
    }
    None
}

impl SubtaskResult {
    /// 本子任务是否算"严重失败"(影响整体回答)。
    /// 我们认为:`InvalidArgs / NoCaseBound` 是软失败(LLM 调用方式问题,重试即可),
    /// `Yuandian / Sqlx / Io / Runtime` 是硬失败(基础设施异常,应该提醒用户)。
    pub fn is_hard_failure(&self) -> bool {
        if self.success {
            return false;
        }
        // 简单识别:error_short 里包含 "InvalidArgs / NoCaseBound / 未注册" 视为软失败
        let s = self.error_short.as_deref().unwrap_or("");
        !(s.contains("参数错误") || s.contains("没绑定案件") || s.contains("未注册"))
    }
}

/// 跑一组子任务,**read-only 工具并发、mutating 工具串行独占**,allow 部分失败。
///
/// 为什么 mutating 要串行:本模块用 `join_all` 在**同一 task** 内协作并发(非真线程),
/// 工具在 IO `await` 点会让出。若同一轮里有两个改同一份文书的 `edit_artifact`(read→改→write),
/// 它们会在文件 read 的 await 点交错 → 都读到原文 → 各自写回 → **后写覆盖前写(丢更新)**。
/// 故 mutating 工具(`Tool::is_mutating`)一个跑完再跑下一个;read-only 仍并发。
///
/// 返回顺序跟入参 subtasks 一致(回填 messages 要按 tool_call 顺序匹配 tool_call_id)。
pub async fn run_parallel_subtasks(
    subtasks: Vec<Subtask>,
    registry: &ToolRegistry,
    ctx: &ToolContext<'_>,
) -> Vec<SubtaskResult> {
    let n = subtasks.len();
    let mut slots: Vec<Option<SubtaskResult>> = (0..n).map(|_| None).collect();

    // 分流:read-only(并发)/ mutating(串行),都记原始下标以便按序回填
    let mut ro: Vec<(usize, Subtask)> = Vec::new();
    let mut muts: Vec<(usize, Subtask)> = Vec::new();
    for (i, st) in subtasks.into_iter().enumerate() {
        if registry.is_mutating(&st.tool) {
            muts.push((i, st));
        } else {
            ro.push((i, st));
        }
    }

    // read-only 并发
    let (ro_idx, ro_tasks): (Vec<usize>, Vec<Subtask>) = ro.into_iter().unzip();
    let ro_results = futures::future::join_all(
        ro_tasks
            .into_iter()
            .map(|st| execute_registered_tool(st, registry, ctx)),
    )
    .await;
    for (i, r) in ro_idx.into_iter().zip(ro_results) {
        slots[i] = Some(r);
    }

    // mutating 串行(一个跑完再跑下一个)
    for (i, st) in muts {
        slots[i] = Some(execute_registered_tool(st, registry, ctx).await);
    }

    slots
        .into_iter()
        .map(|s| s.expect("每个下标都应被填充"))
        .collect()
}

/// 执行一次已注册工具并生成与原生并行循环完全相同的审计结果。
/// Pi Sidecar 只负责提出调用；路径脱敏、积分、KB 命中和错误形状仍由 Rust 统一负责。
pub async fn execute_registered_tool(
    st: Subtask,
    registry: &ToolRegistry,
    ctx: &ToolContext<'_>,
) -> SubtaskResult {
    let started = chrono::Local::now().timestamp_millis();
    let exec = run_tool_inner(&st.tool, &st.args, registry, ctx).await;
    let finished = chrono::Local::now().timestamp_millis();
    match exec {
        Ok(r) => {
            let result_preview = tool_result_preview(&st.tool, &r.content);
            SubtaskResult {
                tool_call_id: st.tool_call_id,
                tool: st.tool,
                args: st.args,
                success: true,
                result_preview,
                content: r.content,
                kb_hit: r.kb_hit,
                credits_used: r.yuandian_credits_used,
                error_short: None,
                started_at_ms: started,
                finished_at_ms: finished,
            }
        }
        Err(e) => {
            let err_str = e.to_string();
            // sanitize 路径(避免泄漏 /Users/xxx 这种)— 走反馈 MD 同一套
            let safe = crate::feedback::sanitize_paths(&err_str);
            let content = serde_json::to_string(&serde_json::json!({"error": &safe}))
                .unwrap_or_else(|_| format!("{{\"error\":\"{}\"}}", safe.replace('"', "'")));
            SubtaskResult {
                tool_call_id: st.tool_call_id,
                tool: st.tool,
                args: st.args,
                success: false,
                content,
                kb_hit: false,
                credits_used: 0,
                error_short: Some(safe),
                result_preview: None,
                started_at_ms: started,
                finished_at_ms: finished,
            }
        }
    }
}

async fn run_tool_inner(
    name: &str,
    args: &Value,
    registry: &ToolRegistry,
    ctx: &ToolContext<'_>,
) -> Result<super::tools::ToolResult, ToolError> {
    let tool: &dyn Tool = registry
        .find(name)
        .ok_or_else(|| ToolError::InvalidArgs(format!("未注册的工具:{}", name)))?;
    tool.execute(args, ctx).await
}
