//! MCP-bridge:CaseBoard 当 MCP **客户端**,消费外部 MCP server 工具(治「扩展麻烦」=加能力不必改 Rust 重出 dmg)。
//!
//! 详见 `docs/adr/0008-MCP-bridge-CaseBoard当客户端消费外部工具.md`。**已落地(2026-06-02)**:
//! 手搓零依赖 stdio JSON-RPC 客户端(`McpClient`:initialize→initialized→tools/list→tools/call,
//! 按 id 匹配跳过通知 + 超时 + `kill_on_drop`)+ 配置形状(`McpServerConfig`/`McpTransport`)+
//! 转发工具(`McpForwardingTool` impl `Tool`)+ 编排(`connect_mcp_servers`,失败跳过+dlog+按名排序)。
//! 配置存 `settings.mcp_servers`(白名单,默认空 = 桥接关闭、零开销);在 `commands::case_chat_impl`
//! 起手连接,绑一次 chat 调用(registry drop → 子进程被杀)。前端配置 UI = `SettingsModal` 的
//! `McpServersCard`(增删/启用/stdio command·args·env)。
//!
//! **端到端已实测(2026-06-04)**:① python stub 协议往返(`mcp_roundtrip`,本地无网);
//! ② 真实官方 server `@modelcontextprotocol/server-everything`(`mcp_real_server`,需网络+npx);
//! ③ 真实 inputSchema(带 `$schema`/`additionalProperties`/`default`)过 `to_function_schema`
//! 后被 DeepSeek function-calling 正常接受并回 tool_call(真 key 实测,无需 schema 清洗)。
//! 真连测均 `#[ignore]`(离线不挂)。
//!
//! **HTTP 传输(Streamable HTTP)已实现(2026-06-10)**:元典 / 企查查 / 万得 / 北大法宝等
//! 国内数据平台的云端 MCP 全是「URL + Bearer 头」的 Streamable HTTP 型(用户零环境依赖,
//! 比 stdio 更适合小白)。POST JSON-RPC → 响应兼容 `application/json` 与 `text/event-stream`
//! 两种;处理 `Mcp-Session-Id` 会话头 + `MCP-Protocol-Version` 协商头。401/403 等鉴权错误
//! **透传真实状态码**(已知坑 #8)。真连测 `mcp_real_http_yuandian`(`#[ignore]`,需元典 key)。
//!
//! 标 `allow(dead_code)`:`parse_server_configs` / `DiscoveredTool::to_function_schema`
//! 暂留作未来/测试用,非死代码遗留。

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

use super::tools::{Tool, ToolContext, ToolError, ToolResult};

/// 外部 MCP server 的传输方式。两种传输共用此配置形状,与「rmcp 还是手搓」的实现决策无关。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpTransport {
    /// 本地子进程,走 stdio JSON-RPC(如 `npx -y @modelcontextprotocol/server-xxx`)。
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        /// 额外环境变量(放 token 等;**不进 git/日志**)。
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    /// 远端 Streamable HTTP endpoint(如元典/企查查/万得/北大法宝的云端 MCP)。
    Http {
        url: String,
        /// 额外请求头(放 `Authorization: Bearer xxx` 等;**不进 git/日志**)。
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

/// 一个外部 MCP server 的配置项(存 settings.json 或表,**存储无关**:从任意 JSON 反序列化)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// 人读名,也用作工具命名空间前缀(见 [`DiscoveredTool::namespaced_name`])。
    pub name: String,
    pub transport: McpTransport,
    /// 是否启用。白名单语义:只连 `enabled=true` 的;整个列表默认空 = 桥接关闭、行为不变。
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl McpServerConfig {
    /// 校验配置可用:name 非空;stdio 的 command 非空 / http 的 url 非空。
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("MCP server name 不能为空".into());
        }
        match &self.transport {
            McpTransport::Stdio { command, .. } if command.trim().is_empty() => Err(format!(
                "MCP server「{}」的 stdio command 不能为空",
                self.name
            )),
            McpTransport::Http { url, .. } if url.trim().is_empty() => {
                Err(format!("MCP server「{}」的 http url 不能为空", self.name))
            }
            _ => Ok(()),
        }
    }
}

/// 从一段 JSON(期望是 server 配置数组,如 `settings.mcp_servers`)防御式解析出配置列表。
/// 非数组 → 空;单条反序列化失败 → 跳过该条(不整体失败)。**不**做 enabled/validate 过滤,
/// 调用方再 `.filter(|c| c.enabled && c.validate().is_ok())` 取「该连的 server」。
pub fn parse_server_configs(value: &Value) -> Vec<McpServerConfig> {
    let Some(arr) = value.as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| serde_json::from_value::<McpServerConfig>(v.clone()).ok())
        .collect()
}

/// 远端 MCP server `tools/list` 返回的单个工具元数据。
///
/// 这是「能直接并进 DeepSeek tools 数组」的形态:[`Self::to_function_schema`] 跟内置
/// `Tool::to_function_schema` 同形。无论传输怎么实现,远端工具都归一到此形状。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// MCP 的 `inputSchema`(JSON Schema)。
    #[serde(rename = "inputSchema", default)]
    pub input_schema: Value,
}

/// 把一段名字清洗成 DeepSeek/OpenAI function 名允许的字符集(`[A-Za-z0-9_-]`),其余 → `_`。
fn sanitize_fn_segment(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl DiscoveredTool {
    /// 加 server 命名空间前缀避免跟内置工具 / 其他 server 重名(`mcp__<server>__<tool>`,
    /// 与 Claude Code 的 MCP 工具命名一致)。
    ///
    /// DeepSeek/OpenAI function 名约 `^[A-Za-z0-9_-]+$`。server 名用户可填中文、远端工具名也可能
    /// 带怪字符 → 非法字符清洗成 `_`,**兜底不让整个 tools 数组被 API 拒**(一个坏名会废掉整轮 chat)。
    /// 实际调用远端用的是 `McpForwardingTool::remote_name`(原 `self.name`),不受此清洗影响。
    pub fn namespaced_name(&self, server: &str) -> String {
        format!(
            "mcp__{}__{}",
            sanitize_fn_segment(server),
            sanitize_fn_segment(&self.name)
        )
    }

    /// 转成 DeepSeek `tools` 数组单条。`tool_name` 由调用方传(一般是 namespaced)。
    pub fn to_function_schema(&self, tool_name: &str) -> Value {
        let parameters = if self.input_schema.is_null() {
            serde_json::json!({ "type": "object", "properties": {} })
        } else {
            self.input_schema.clone()
        };
        serde_json::json!({
            "type": "function",
            "function": {
                "name": tool_name,
                "description": self.description,
                "parameters": parameters,
            }
        })
    }
}

// =============================================================================
// MCP JSON-RPC 客户端(手搓零依赖,见 ADR-0008 §4:对齐已知坑 #5 MinerU 客户端先例)。
// 两种传输共用同一套握手语义:initialize → notifications/initialized → tools/list / tools/call。
// - stdio:newline-delimited JSON-RPC 2.0 over 子进程管道。
// - http:Streamable HTTP —— 每条消息 POST 到 endpoint,响应可能是单条 JSON,也可能是
//   SSE 流(`text/event-stream`,事件 data 载荷即 JSON-RPC 消息)。
// **真连外部 server 无法 headless 验**,有 #[ignore] 的 python stub / 真实 server 测兜底。
// =============================================================================

/// stdio 用(2026-06-04 已对真实 server 实测,别乱升)。
const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
/// http 用:Streamable HTTP 自 2025-03-26 版进 spec,旧版本没有该传输。
const MCP_PROTOCOL_VERSION_HTTP: &str = "2025-03-26";
const MCP_INIT_TIMEOUT: Duration = Duration::from_secs(15);
const MCP_LIST_TIMEOUT: Duration = Duration::from_secs(15);
const MCP_CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// 一条 stdio 连接的 IO。字段按声明序 drop:先关 stdin/stdout(server 多半随之退出),
/// 再 drop child(`kill_on_drop` 兜底杀进程)。
struct McpIo {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    _child: Child,
}

/// 已完成 initialize 握手的外部 MCP server 连接(stdio 或 http,调用方无感)。
///
/// stdio:单条管道上的请求/响应必须**串行**,故 `Mutex` 包 IO;`McpClient` drop → 子进程
/// 被杀(`kill_on_drop`,生命周期绑一次 chat 调用)。http:无子进程,每条消息独立 POST,
/// 会话状态(`Mcp-Session-Id`)在 connect 时定下后只读。多 server = 多 client 互不干扰。
pub struct McpClient {
    inner: ClientInner,
    next_id: AtomicI64,
}

enum ClientInner {
    Stdio(Mutex<McpIo>),
    Http(HttpConn),
}

/// Streamable HTTP 连接(connect 完成握手后字段全只读,天然可并发)。
struct HttpConn {
    http: reqwest::Client,
    url: String,
    /// 用户配置的额外请求头(典型:`Authorization: Bearer xxx`)。
    extra_headers: BTreeMap<String, String>,
    /// initialize 响应头里的 `Mcp-Session-Id`(server 可选下发;有则后续请求必须带)。
    session_id: Option<String>,
    /// initialize 协商出的协议版本(spec 要求后续请求放 `MCP-Protocol-Version` 头)。
    protocol_version: Option<String>,
}

impl McpClient {
    fn next_id(&self) -> i64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// 建立连接 + 完成 initialize 握手。失败返回可读原因(http 鉴权失败透传真实状态码)。
    pub async fn connect(cfg: &McpServerConfig) -> Result<Self, String> {
        let inner = match &cfg.transport {
            McpTransport::Stdio { command, args, env } => {
                ClientInner::Stdio(Mutex::new(connect_stdio(command, args, env).await?))
            }
            McpTransport::Http { url, headers } => {
                ClientInner::Http(HttpConn::connect(url, headers).await?)
            }
        };
        Ok(Self {
            inner,
            next_id: AtomicI64::new(1),
        })
    }

    /// 按传输分发一条 JSON-RPC 请求。
    async fn request(&self, method: &str, params: Value, to: Duration) -> Result<Value, String> {
        let id = self.next_id();
        match &self.inner {
            ClientInner::Stdio(io) => {
                let mut io = io.lock().await;
                rpc_request(&mut io, id, method, params, to).await
            }
            ClientInner::Http(conn) => {
                let body =
                    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
                conn.post_rpc(body, Some(id), to).await.map(|(v, _)| v)
            }
        }
    }

    /// tools/list:发现远端工具。
    pub async fn list_tools(&self) -> Result<Vec<DiscoveredTool>, String> {
        let result = self
            .request("tools/list", json!({}), MCP_LIST_TIMEOUT)
            .await?;
        let arr = result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(arr
            .iter()
            .filter_map(|t| serde_json::from_value(t.clone()).ok())
            .collect())
    }

    /// tools/call:调远端工具,返回拼好的文本结果。
    pub async fn call_tool(&self, name: &str, arguments: &Value) -> Result<String, String> {
        let params = json!({ "name": name, "arguments": arguments });
        let result = self.request("tools/call", params, MCP_CALL_TIMEOUT).await?;
        Ok(extract_tool_text(&result))
    }
}

/// spawn stdio 子进程 + initialize 握手。
async fn connect_stdio(
    command: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> Result<McpIo, String> {
    let expanded_command = shellexpand::tilde(command).into_owned();
    let mut cmd = crate::proc_util::tokio_command(&expanded_command);
    cmd.args(args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null()) // 排空 stderr,防其缓冲填满挂死子进程
        .kill_on_drop(true);
    // Windows 下隐藏 stdio MCP server 子进程的控制台窗口,避免闪黑框。
    crate::proc_util::hide_console_window(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("启动失败: {e}"))?;
    let stdin = child.stdin.take().ok_or("无法取得 stdin")?;
    let stdout = BufReader::new(child.stdout.take().ok_or("无法取得 stdout")?);
    let mut io = McpIo {
        stdin,
        stdout,
        _child: child,
    };

    // initialize(id=0)
    let init = json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": { "name": "CaseBoard", "version": env!("CARGO_PKG_VERSION") }
    });
    rpc_request(&mut io, 0, "initialize", init, MCP_INIT_TIMEOUT).await?;
    // initialized 通知(spec 要求;缺它部分 server 拒 tools/list)
    rpc_notify(&mut io, "notifications/initialized").await?;
    Ok(io)
}

impl HttpConn {
    /// 建 HTTP 客户端 + initialize 握手 + initialized 通知。
    async fn connect(url: &str, headers: &BTreeMap<String, String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| format!("构建 HTTP 客户端失败: {e}"))?;
        let mut conn = Self {
            http,
            url: url.trim().to_string(),
            extra_headers: headers.clone(),
            session_id: None,
            protocol_version: None,
        };

        // initialize(id=0):从响应头拿会话 ID、从结果拿协商版本,之后每条请求都带上。
        let init = json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
            "protocolVersion": MCP_PROTOCOL_VERSION_HTTP,
            "capabilities": {},
            "clientInfo": { "name": "CaseBoard", "version": env!("CARGO_PKG_VERSION") }
        }});
        let (result, resp_headers) = conn.post_rpc(init, Some(0), MCP_INIT_TIMEOUT).await?;
        if let Some(sid) = resp_headers
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
        {
            conn.session_id = Some(sid.to_string());
        }
        if let Some(pv) = result.get("protocolVersion").and_then(|v| v.as_str()) {
            conn.protocol_version = Some(pv.to_string());
        }

        // initialized 通知:spec 要求(server 应答 202)。国内网关实现参差,失败只记日志
        // 不拦断 —— 真坏掉的连接会在 tools/list 立刻暴露,这里宽容能多兼容一批 server。
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        if let Err(e) = conn.post_rpc(note, None, MCP_INIT_TIMEOUT).await {
            crate::dlog!("MCP http initialized 通知未被接受(继续): {e}");
        }
        Ok(conn)
    }

    /// POST 一条 JSON-RPC 消息。`want_id=None` 表示通知(2xx 即成功,不读 body);
    /// 否则等匹配 id 的响应(兼容单条 JSON 与 SSE 流两种响应格式)。
    async fn post_rpc(
        &self,
        body: Value,
        want_id: Option<i64>,
        to: Duration,
    ) -> Result<(Value, reqwest::header::HeaderMap), String> {
        match timeout(to, self.post_rpc_inner(body, want_id)).await {
            Ok(r) => r,
            Err(_) => Err(format!("MCP HTTP 请求超时({}s)", to.as_secs())),
        }
    }

    async fn post_rpc_inner(
        &self,
        body: Value,
        want_id: Option<i64>,
    ) -> Result<(Value, reqwest::header::HeaderMap), String> {
        let mut req = self
            .http
            .post(&self.url)
            .header("Content-Type", "application/json")
            // spec 要求 Accept 同时声明两种;少一个会被部分 server 拒
            .header("Accept", "application/json, text/event-stream");
        if let Some(pv) = &self.protocol_version {
            req = req.header("MCP-Protocol-Version", pv.as_str());
        }
        if let Some(sid) = &self.session_id {
            req = req.header("Mcp-Session-Id", sid.as_str());
        }
        for (k, v) in &self.extra_headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP 请求失败: {e}"))?;
        let status = resp.status();
        let resp_headers = resp.headers().clone();
        if !status.is_success() {
            // 真错透传(已知坑 #8):401=令牌不对/过期、403=服务未购买/到期,状态码是用户自查的关键
            let text = resp.text().await.unwrap_or_default();
            let snippet: String = text.chars().take(300).collect();
            return Err(format!("HTTP {status}: {snippet}"));
        }
        let Some(want) = want_id else {
            return Ok((Value::Null, resp_headers)); // 通知:常见 202 Accepted,无 body
        };

        let ct = resp_headers
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ct.starts_with("text/event-stream") {
            // SSE:增量读流;事件 data 载荷是 JSON-RPC 消息,拿到匹配 id 的响应即返回
            // (随即 drop 流断连,server 端按 spec 在响应后也会主动关流)。
            use futures::StreamExt;
            let mut stream = resp.bytes_stream();
            let mut buf = String::new();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| format!("读 SSE 流失败: {e}"))?;
                buf.push_str(&String::from_utf8_lossy(&chunk).replace('\r', ""));
                for payload in sse_drain_events(&mut buf) {
                    let Ok(v) = serde_json::from_str::<Value>(&payload) else {
                        continue; // 非 JSON 的事件(心跳注释等)→ 跳过
                    };
                    if let Some(r) = rpc_take_response(&v, want) {
                        return r.map(|val| (val, resp_headers));
                    }
                }
            }
            Err("SSE 流已结束,仍未等到响应".into())
        } else {
            let v: Value = resp
                .json()
                .await
                .map_err(|e| format!("解析 MCP 响应失败: {e}"))?;
            match rpc_take_response(&v, want) {
                Some(r) => r.map(|val| (val, resp_headers)),
                None => Err(format!("MCP 响应 id 不匹配(期望 {want})")),
            }
        }
    }
}

/// 从累积缓冲里取出所有**完整** SSE 事件的 data 载荷(事件以空行结尾),不完整的留在 buf。
/// 调用方需先把 `\r` 剥掉。一个事件多条 `data:` 行按 spec 用 `\n` 连接;
/// 其他字段行(`event:`/`id:`/`retry:`/注释)忽略;无 data 的事件不产出。
fn sse_drain_events(buf: &mut String) -> Vec<String> {
    let mut out = Vec::new();
    while let Some(pos) = buf.find("\n\n") {
        let event: String = buf[..pos].to_string();
        buf.drain(..pos + 2);
        let mut data_lines: Vec<&str> = Vec::new();
        for line in event.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                data_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
            }
        }
        if !data_lines.is_empty() {
            out.push(data_lines.join("\n"));
        }
    }
    out
}

/// 一条 JSON-RPC 消息是否是 `want_id` 的响应:是 → `Some(结果或错误)`;
/// 通知/别的 id → `None`(调用方继续等)。stdio 与 http 共用,保两种传输语义一致。
fn rpc_take_response(v: &Value, want_id: i64) -> Option<Result<Value, String>> {
    match v.get("id").and_then(|i| i.as_i64()) {
        Some(id) if id == want_id => {
            if let Some(err) = v.get("error") {
                Some(Err(format!("MCP 返回错误: {err}")))
            } else {
                Some(Ok(v.get("result").cloned().unwrap_or(Value::Null)))
            }
        }
        _ => None,
    }
}

/// 发 JSON-RPC 请求 + 读到匹配 id 的响应(跳过通知/日志/别的 id),带超时。
async fn rpc_request(
    io: &mut McpIo,
    id: i64,
    method: &str,
    params: Value,
    to: Duration,
) -> Result<Value, String> {
    let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    write_line(&mut io.stdin, &msg).await?;
    match timeout(to, read_matching(&mut io.stdout, id)).await {
        Ok(r) => r,
        Err(_) => Err(format!("MCP {method} 超时({}s)", to.as_secs())),
    }
}

/// 发 JSON-RPC 通知(无 id,不等响应)。
async fn rpc_notify(io: &mut McpIo, method: &str) -> Result<(), String> {
    let msg = json!({ "jsonrpc": "2.0", "method": method });
    write_line(&mut io.stdin, &msg).await
}

async fn write_line(stdin: &mut ChildStdin, msg: &Value) -> Result<(), String> {
    let mut line = serde_json::to_string(msg).map_err(|e| e.to_string())?;
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| format!("写 MCP 请求失败: {e}"))?;
    stdin.flush().await.map_err(|e| format!("flush 失败: {e}"))
}

/// 逐行读到 id 匹配的响应;跳过通知(无 id)、日志、不同 id 的行 —— server 会在响应前穿插
/// log 通知,"读一行=我的响应"是经典 bug。泛型化以便单测。
async fn read_matching<R: tokio::io::AsyncBufRead + Unpin>(
    stdout: &mut R,
    want_id: i64,
) -> Result<Value, String> {
    loop {
        let mut line = String::new();
        let n = stdout
            .read_line(&mut line)
            .await
            .map_err(|e| format!("读 MCP 响应失败: {e}"))?;
        if n == 0 {
            return Err("MCP server 关闭了连接(EOF)".into());
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(trimmed) else {
            continue; // 非 JSON(日志噪音)→ 跳过
        };
        match rpc_take_response(&v, want_id) {
            Some(r) => return r,
            None => continue, // 通知 / 其它 id → 跳过
        }
    }
}

/// 从 tools/call 结果抽文本(`result.content = [{type:text,text},...]`);带 isError 标记。
fn extract_tool_text(result: &Value) -> String {
    let mut out = String::new();
    if let Some(blocks) = result.get("content").and_then(|c| c.as_array()) {
        for b in blocks {
            if let Some(t) = b.get("text").and_then(|t| t.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        // 无文本块(图片/资源等少见类型)→ 整个 result 压成 JSON 兜底
        out = serde_json::to_string(result).unwrap_or_default();
    }
    if result
        .get("isError")
        .and_then(|e| e.as_bool())
        .unwrap_or(false)
    {
        format!("[MCP 工具报错] {out}")
    } else {
        out
    }
}

/// 把一个远端 MCP 工具包成本仓的一等 `Tool`,execute 转发到远端。
pub struct McpForwardingTool {
    full_name: String, // mcp__<server>__<tool>
    description: String,
    parameters: Value, // inputSchema
    remote_name: String,
    client: Arc<McpClient>,
}

#[async_trait]
impl Tool for McpForwardingTool {
    fn name(&self) -> &str {
        &self.full_name
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn parameters_schema(&self) -> Value {
        self.parameters.clone()
    }
    async fn execute(&self, args: &Value, _ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        match self.client.call_tool(&self.remote_name, args).await {
            Ok(text) => Ok(ToolResult::plain(text)),
            Err(e) => Err(ToolError::Runtime(format!(
                "MCP 工具 {} 调用失败: {e}",
                self.full_name
            ))),
        }
    }
}

/// 连接所有 enabled 的 MCP server、发现工具、包成转发工具。失败(配置非法/连不上/列不出)
/// 的 server **跳过 + dlog**,绝不拖垮 chat。结果按工具名**确定性排序**(保前缀缓存稳定)。
/// **隐私**:只 dlog server 名 + 工具数,绝不记 tool-call 参数(含案件内容)。
pub async fn connect_mcp_servers(configs: &[McpServerConfig]) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    for cfg in configs.iter().filter(|c| c.enabled) {
        if let Err(e) = cfg.validate() {
            crate::dlog!("MCP server「{}」配置无效,跳过: {}", cfg.name, e);
            continue;
        }
        let client = match McpClient::connect(cfg).await {
            Ok(c) => Arc::new(c),
            Err(e) => {
                crate::dlog!("MCP server「{}」连接失败,跳过: {}", cfg.name, e);
                continue;
            }
        };
        let discovered = match client.list_tools().await {
            Ok(d) => d,
            Err(e) => {
                crate::dlog!("MCP server「{}」列工具失败,跳过: {}", cfg.name, e);
                continue;
            }
        };
        crate::dlog!(
            "MCP server「{}」已连,发现 {} 个工具",
            cfg.name,
            discovered.len()
        );
        for dt in discovered {
            let parameters = if dt.input_schema.is_null() {
                json!({ "type": "object", "properties": {} })
            } else {
                dt.input_schema.clone()
            };
            tools.push(Box::new(McpForwardingTool {
                full_name: dt.namespaced_name(&cfg.name),
                description: dt.description.clone(),
                parameters,
                remote_name: dt.name.clone(),
                client: client.clone(),
            }));
        }
    }
    // 确定性顺序 → 前缀缓存稳定(prefix_cache 观测 tools 指纹漂移)
    tools.sort_by(|a, b| a.name().cmp(b.name()));
    tools
}
