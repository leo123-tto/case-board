//! 本地知识库 2 个 tool(V0.2 D2-D3.E)。
//!
//! `search_local_kb` / `read_kb_file` 全部读 `~/Documents/知识库/` 任意位置,
//! 走 `local_kb::search` 模块,**不消耗元典积分**。
//! KB 未启用时(`ctx.local_kb` = None)直接返回空,不报错(降级)。

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{opt_bool, opt_u32, require_str, Tool, ToolContext, ToolError, ToolResult};
use crate::local_kb::search::{read_kb_file as kb_read, search_kb_files, KbScope, SearchOptions};

pub struct SearchLocalKb;

#[async_trait]
impl Tool for SearchLocalKb {
    fn name(&self) -> &str {
        "search_local_kb"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/search_local_kb.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "keyword": {"type": "string", "description": "中文关键词"},
                "scope": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "[\"root\",\"notes\",\"companies\",\"sources\",\"topics\",\"gap_log\"] 任意子集,默认 root 整根知识库(companies=企业档案/调查报告)"
                },
                "include_yuandian_cache": {"type": "boolean", "description": "默认 false"},
                "max_results": {"type": "integer", "description": "默认 30,最大 100"}
            },
            "required": ["keyword"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let keyword = require_str(args, "keyword")?;
        let max_results = opt_u32(args, "max_results")
            .map(|n| (n as usize).min(100))
            .unwrap_or(30);
        let include_cache = opt_bool(args, "include_yuandian_cache").unwrap_or(false);
        let scopes = parse_scopes(args.get("scope"), include_cache);

        // KB 未启用 → 静默降级返回空(description 已说明这条)
        let Some(kb) = ctx.local_kb else {
            return Ok(ToolResult {
                content: "[]".into(),
                yuandian_credits_used: 0,
                kb_hit: false,
            });
        };

        let opts = SearchOptions {
            scopes: Some(scopes),
            max_results,
            snippet_chars: 200,
            case_sensitive: false,
        };
        let hits = search_kb_files(&kb.root, keyword, opts)?;
        let content = serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".into());
        Ok(ToolResult {
            content,
            yuandian_credits_used: 0,
            kb_hit: !hits.is_empty(),
        })
    }
}

/// KB 默认检索范围(scope 缺省 / 给了无效值时用)。
fn default_kb_scopes() -> Vec<KbScope> {
    vec![KbScope::Root]
}

/// 解析 args.scope。`None` / 空数组 / **全是无效值**(如模型误传数字 `[4]`)→ 退回默认全部,
/// 绝不返回空 scope(否则搜了个空范围、静默返回零结果,白调一次还误导模型「本地没有」)。
fn parse_scopes(raw: Option<&Value>, include_yuandian_cache: bool) -> Vec<KbScope> {
    let parsed: Vec<KbScope> = match raw.and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => arr
            .iter()
            .filter_map(|v| v.as_str())
            .filter_map(|s| match s {
                "root" | "all" => Some(KbScope::Root),
                "notes" => Some(KbScope::Notes),
                "companies" => Some(KbScope::Companies),
                "cases_experience" | "cases-experience" => Some(KbScope::CasesExperience),
                "sources" => Some(KbScope::Sources),
                "topics" => Some(KbScope::Topics),
                "gap_log" | "gap-log" => Some(KbScope::GapLog),
                _ => None,
            })
            .collect(),
        _ => default_kb_scopes(),
    };
    // 给了 scope 但没一个有效(类型错 / 拼错)→ 退回默认,别搜空。
    let mut scopes = if parsed.is_empty() {
        default_kb_scopes()
    } else {
        parsed
    };
    if include_yuandian_cache {
        scopes.push(KbScope::YuandianCache);
    }
    scopes
}

pub struct ReadKbFile;

#[async_trait]
impl Tool for ReadKbFile {
    fn name(&self) -> &str {
        "read_kb_file"
    }
    fn description(&self) -> &str {
        include_str!("descriptions/read_kb_file.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "relative_path": {"type": "string", "description": "KB 内相对路径,如 wiki/sources/X.md"},
                "offset": {"type": "integer", "description": "默认 0"},
                "length": {"type": "integer", "description": "默认 10000"}
            },
            "required": ["relative_path"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let rel = require_str(args, "relative_path")?;
        let offset = opt_u32(args, "offset").map(|n| n as usize);
        let length = opt_u32(args, "length").map(|n| n as usize);
        let kb = ctx.local_kb.ok_or_else(|| {
            ToolError::Runtime("本地知识库未启用(用户未设置 local_kb_root 或当前路径不存在)".into())
        })?;
        let content = kb_read(&kb.root, rel, offset, length)?;
        Ok(ToolResult {
            content,
            yuandian_credits_used: 0,
            kb_hit: true,
        })
    }
}
