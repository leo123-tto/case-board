//! 智能粘贴识别:把数据平台「接入指南」里复制来的配置文本解析成 [`McpServerConfig`] 列表。
//!
//! 设计原则:**只做确定性解析,不用 LLM 猜**——配置猜错(幻觉 URL/字段)比识别失败更糟;
//! 认不出来就老实报错让用户检查复制范围。自由文本的兜底以后走 chat agent 配置工具(计划 ④)。
//!
//! 支持的形态(覆盖元典 / 企查查 / 万得 / 北大法宝四家接入文档实测样式):
//! 1. 标准 `{"mcpServers": {<name>: <def>}}`(Claude Desktop / WorkBuddy / 通用格式)
//! 2. 裸 `<name> → <def>` map,或单个 `<def>` 对象(企查查聊天框样式)
//! 3. `claude mcp add --transport http <name> <url> --header "K: V"` 命令行(含 `\` 续行)
//! 4. 以上内容混在整页说明文字里(从文本中扫 JSON 块;`//`/`#` 整行注释先剥掉)
//!
//! `<def>` 形状:有 `url` → http(`type` 写 sse/streamablehttp 的也归 http;`headers` 透传);
//! 有 `command` → stdio(`args`/`env` 透传)。占位符令牌(YOUR_API_KEY 等)→ 产出人读警告。

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

use super::mcp_bridge::{McpServerConfig, McpTransport};

/// 解析结果:server 列表 + 人读警告(占位符令牌等,前端原样展示)。
#[derive(Debug, Serialize, PartialEq)]
pub struct ParsedPaste {
    pub servers: Vec<McpServerConfig>,
    pub warnings: Vec<String>,
}

/// 入口:从粘贴文本解析 MCP server 配置。一个都没认出 → Err(人读提示)。
pub fn parse_pasted_config(text: &str) -> Result<ParsedPaste, String> {
    let mut servers = parse_claude_mcp_add_commands(text);
    // JSON 与命令行可能同页共存(平台文档常给两种 tab),都收;按 name 去重保留先出现的
    servers.extend(parse_json_blocks(&strip_comment_lines(text)));
    dedup_by_name(&mut servers);

    if servers.is_empty() {
        return Err(
            "没有识别出 MCP 配置。请把平台接入文档里的配置整段复制进来(JSON 格式需包含 \
             mcpServers 或 url/command 字段;也支持 claude mcp add 命令行)。"
                .into(),
        );
    }
    let warnings = collect_placeholder_warnings(&servers);
    Ok(ParsedPaste { servers, warnings })
}

/// 剥掉整行 `//` / `#` 注释(平台示例 JSON 里常见,serde 不认)。
/// 只删「行首(允许前导空白)就是注释」的行,绝不碰行内的 `https://`。
fn strip_comment_lines(text: &str) -> String {
    text.lines()
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("//") || t.starts_with('#'))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn dedup_by_name(servers: &mut Vec<McpServerConfig>) {
    let mut seen = std::collections::BTreeSet::new();
    servers.retain(|s| seen.insert(s.name.clone()));
}

// ============================================================================
// JSON 形态
// ============================================================================

/// 扫描文本中的 JSON 对象(说明文字混排也能抓),解析出 server 列表。
/// 解析成功且产出 server 的块,跳过其消费的字节,避免把内层 def 重复再解析一遍。
fn parse_json_blocks(text: &str) -> Vec<McpServerConfig> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // 只在 ASCII '{' 处切片,保证字符边界安全
        if bytes[i] == b'{' {
            let mut iter = serde_json::Deserializer::from_str(&text[i..]).into_iter::<Value>();
            if let Some(Ok(v)) = iter.next() {
                let found = servers_from_value(&v);
                if !found.is_empty() {
                    out.extend(found);
                    i += iter.byte_offset().max(1);
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

/// 从一个 JSON Value 里抽 server:优先 mcpServers 包裹;其次裸 map / 单个 def。
fn servers_from_value(v: &Value) -> Vec<McpServerConfig> {
    if let Some(map) = v.get("mcpServers").and_then(|m| m.as_object()) {
        return map
            .iter()
            .filter_map(|(name, def)| server_from_def(name, def))
            .collect();
    }
    let Some(obj) = v.as_object() else {
        return Vec::new();
    };
    // 单个 def 对象(没有外层名字)→ 从 url/command 推导一个名字
    if obj.contains_key("url") || obj.contains_key("command") {
        return server_from_def("", v).into_iter().collect();
    }
    // 裸 name→def map:要求**每个** value 都解析得出,防止把无关 JSON 误吞
    let defs: Vec<_> = obj
        .iter()
        .filter_map(|(k, d)| server_from_def(k, d))
        .collect();
    if !defs.is_empty() && defs.len() == obj.len() {
        return defs;
    }
    Vec::new()
}

/// 单个 def 对象 → 配置。认不出(没 url 也没 command)→ None。
fn server_from_def(name: &str, def: &Value) -> Option<McpServerConfig> {
    let obj = def.as_object()?;
    let transport = if let Some(url) = obj.get("url").and_then(|u| u.as_str()) {
        let headers: BTreeMap<String, String> = obj
            .get("headers")
            .and_then(|h| h.as_object())
            .map(|h| {
                h.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        McpTransport::Http {
            url: url.to_string(),
            headers,
        }
    } else {
        let cmd = obj.get("command").and_then(|c| c.as_str())?;
        let args = obj
            .get("args")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let env: BTreeMap<String, String> = obj
            .get("env")
            .and_then(|e| e.as_object())
            .map(|e| {
                e.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        McpTransport::Stdio {
            command: cmd.to_string(),
            args,
            env,
        }
    };
    let name = if name.trim().is_empty() {
        derive_name(&transport)
    } else {
        name.trim().to_string()
    };
    Some(McpServerConfig {
        name,
        transport,
        enabled: true,
    })
}

/// 没有外层名字时从 transport 推导:http 取 URL 路径里最后一个有意义的段
/// (滤掉 mcp/stream/sse 这类协议词),退而取 host 首段;stdio 取命令基名。
fn derive_name(t: &McpTransport) -> String {
    match t {
        McpTransport::Http { url, .. } => {
            let no_scheme = url.split("://").nth(1).unwrap_or(url);
            let mut parts = no_scheme.split('/');
            let host = parts.next().unwrap_or("");
            let meaningful: Vec<&str> = parts
                .filter(|s| {
                    !s.is_empty()
                        && !matches!(
                            s.to_ascii_lowercase().as_str(),
                            "mcp" | "stream" | "sse" | "api" | "v1" | "v2"
                        )
                })
                .collect();
            meaningful
                .last()
                .copied()
                .or_else(|| host.split('.').next())
                .unwrap_or("server")
                .to_string()
        }
        McpTransport::Stdio { command, .. } => command
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("server")
            .to_string(),
    }
}

// ============================================================================
// `claude mcp add` 命令行形态
// ============================================================================

/// 解析文本里所有 `claude mcp add ...` 命令(支持 `\` 续行、引号、行前 `$`/`#` 提示符)。
fn parse_claude_mcp_add_commands(text: &str) -> Vec<McpServerConfig> {
    // 续行合并:`\` + 换行 → 空格
    let joined = text.replace("\\\r\n", " ").replace("\\\n", " ");
    let mut out = Vec::new();
    for line in joined.lines() {
        let Some(pos) = line.find("claude mcp add") else {
            continue;
        };
        let rest = &line[pos + "claude mcp add".len()..];
        if let Some(cfg) = parse_one_add_command(rest) {
            out.push(cfg);
        }
    }
    out
}

fn parse_one_add_command(rest: &str) -> Option<McpServerConfig> {
    let tokens = shell_tokens(rest);
    let mut transport_flag: Option<String> = None;
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    let mut positionals: Vec<String> = Vec::new();
    let mut stdio_cmd: Vec<String> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i].as_str() {
            "--transport" | "-t" => {
                transport_flag = tokens.get(i + 1).cloned();
                i += 2;
            }
            // 带值但与配置无关的 flag,跳过其值
            "--scope" | "-s" => i += 2,
            "--header" | "-H" => {
                if let Some(h) = tokens.get(i + 1) {
                    // CLI 风格是 "Key: Value"(冒号分隔)
                    if let Some((k, v)) = h.split_once(':') {
                        headers.insert(k.trim().to_string(), v.trim().to_string());
                    }
                }
                i += 2;
            }
            "--env" | "-e" => {
                if let Some(kv) = tokens.get(i + 1) {
                    if let Some((k, v)) = kv.split_once('=') {
                        env.insert(k.trim().to_string(), v.trim().to_string());
                    }
                }
                i += 2;
            }
            "--" => {
                stdio_cmd = tokens[i + 1..].to_vec();
                break;
            }
            t if t.starts_with('-') => i += 1, // 未知 flag:按无值跳过
            t => {
                positionals.push(t.to_string());
                i += 1;
            }
        }
    }
    let name = positionals.first()?.clone();
    let url_pos = positionals.iter().find(|p| p.starts_with("http"));
    let is_http = matches!(
        transport_flag
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("http") | Some("sse") | Some("streamablehttp") | Some("streamable-http")
    ) || (transport_flag.is_none() && url_pos.is_some());
    let transport = if is_http {
        McpTransport::Http {
            url: url_pos?.clone(),
            headers,
        }
    } else {
        // stdio:`-- cmd args...` 优先;否则第二个位置参数当命令
        let (command, args) = if !stdio_cmd.is_empty() {
            (stdio_cmd[0].clone(), stdio_cmd[1..].to_vec())
        } else if positionals.len() >= 2 {
            (positionals[1].clone(), positionals[2..].to_vec())
        } else {
            return None;
        };
        McpTransport::Stdio { command, args, env }
    };
    Some(McpServerConfig {
        name,
        transport,
        enabled: true,
    })
}

/// 极简 shell 分词:按空白切,支持双/单引号包裹(引号内空白保留,引号剥掉)。
fn shell_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in s.chars() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else {
                    cur.push(c);
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                    }
                }
                c => cur.push(c),
            },
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ============================================================================
// 占位符警告
// ============================================================================

/// 检查 headers/env/url 里的占位符令牌(YOUR_API_KEY / <在此填写> / 替换 等),
/// 产出人读警告。**只回显匹配到的占位符词本身**,绝不回显疑似真实密钥的值。
fn collect_placeholder_warnings(servers: &[McpServerConfig]) -> Vec<String> {
    let mut out = Vec::new();
    for s in servers {
        let values: Vec<&String> = match &s.transport {
            McpTransport::Http { url, headers } => {
                headers.values().chain(std::iter::once(url)).collect()
            }
            McpTransport::Stdio { args, env, .. } => env.values().chain(args.iter()).collect(),
        };
        for v in values {
            if let Some(ph) = find_placeholder(v) {
                out.push(format!(
                    "「{}」的配置里还有占位符 {ph},请换成你在平台生成的真实密钥,再点「测试连接」。",
                    s.name
                ));
                break; // 每个 server 提醒一次就够
            }
        }
    }
    out
}

/// 值里找占位符词:`YOUR_xxx` / `<...>` / 显式中文提示。找到 → 返回匹配片段。
fn find_placeholder(v: &str) -> Option<String> {
    let upper = v.to_uppercase();
    if let Some(pos) = upper.find("YOUR_") {
        let tail: String = v[pos..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        return Some(tail);
    }
    if v.contains('<') && v.contains('>') {
        return Some("<...>".into());
    }
    if v.contains("你的") || v.contains("替换") {
        return Some("「你的/替换」字样".into());
    }
    None
}
