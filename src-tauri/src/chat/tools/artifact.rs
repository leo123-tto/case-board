//! 写作工具:`save_artifact`(V0.3 M1 · 2026-05-31)。
//!
//! 案件 AI 助手从「问答/分析」跨到「文书生产」的核心:LLM 起草好正式法律文书后调本工具,
//! 把文书落成案件 document(`source='chat_artifact'`,`category=doc_type`,`is_ai_artifact=1`
//! → 不回喂 LLM 上下文),供律师在界面「导出为 Word(法律格式)」—— 走 `crate::docx_filing`
//! 复刻 quote.law 样本排版(方正小标宋标题 / 黑体小标题 / 仿宋正文 / 两端对齐 / 首行缩进2字)。
//!
//! **充分性把关**在 description 里要求 LLM 调用前先评估(缺信息→选项式追问,不盲吐)。
//! **反虚构**:文书引用走 agent_loop 既有 `verify_legal_citations` hook(final 前自动校验)。
//!
//! 2026-06-01 · 加 **`edit_artifact`**(ADR-0003 Phase 1):对已生成文书做**局部 find/replace**,
//! 让 AI 小改不重吐整篇(省 token、不漂移)。两级匹配(精确 → 剥 inline 标记+压空白归一)
//! 兜「`原告**主张**赔偿` vs AI 给 `原告主张赔偿`」失配;复用 `save_editor_doc` 同款守卫
//! (仅 `is_ai_artifact` + `source∈{chat,chat_artifact}` 可改,拒改扫描原件)。
//!
//! `save_artifact` / `edit_artifact` 均标 `is_mutating`(`Tool::is_mutating`),agent_loop
//! 一轮里串行独占执行(read-only 工具仍并行),防同轮两个改同一文书的 tool_call 并发丢更新。

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::chat::citations;

use super::{require_str, Tool, ToolContext, ToolError, ToolResult};

// V0.3 · doc_type 白名单已开放(任意文书类型可写),不再有枚举常量。
// 民事起诉状 / 证据目录 / 质证意见 / 法律意见书 / 律师函(函类)有固定结构,答辩状 / 代理词有建议结构,
// 其余类型按通用结构(均见 descriptions/save_artifact.md);这是纯 prompt 约束,代码不再 gate 类型。

/// 文件名安全化:剥掉路径分隔符 / 控制字符,限长,防穿越。
fn safe_stem(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' | '\t' => '_',
            _ => c,
        })
        .collect();
    cleaned.trim().chars().take(40).collect::<String>()
}

/// 写进 HTML comment 元信息头前的单行清洗。
/// HTML comment 不能包含 `--`;换行也会破坏导出时的 filing header 解析。
fn safe_filing_metadata(s: &str) -> String {
    s.replace("--", "—")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(200)
        .collect()
}

pub struct SaveArtifact;

#[async_trait]
impl Tool for SaveArtifact {
    fn name(&self) -> &str {
        "save_artifact"
    }
    fn is_mutating(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        include_str!("descriptions/save_artifact.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "doc_type": {
                    "type": "string",
                    "description": "文书类型,按用户要写的材料如实填(如 民事起诉状 / 答辩状 / 质证意见 / 代理词 / 催款律师函 / 证据目录 / 法律意见书 / 分析报告 等,不限)。民事起诉状 / 证据目录 / 质证意见 / 法律意见书 / 律师函(函类)有固定结构,答辩状 / 代理词有建议结构(均见工具说明)"
                },
                "title": {
                    "type": "string",
                    "description": "文书标题(居中大标题)。只放这里,不要在 content_md 重复"
                },
                "content_md": {
                    "type": "string",
                    "description": "正文 Markdown(不含标题)。# 一级标题=「一、」/ ### 二级=「（一）」/ 段落=正文 / **整行加粗**=强调正文。编号写进文本"
                }
            },
            "required": ["doc_type", "title", "content_md"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let case_id = ctx.case_id.ok_or(ToolError::NoCaseBound)?;
        let doc_type_raw = require_str(args, "doc_type")?;
        // V0.3 · 白名单已开放(任意文书类型可写)。doc_type 不再 enum 限制,但要做卫生处理:
        // 它会写进 documents.category + filing 注释头 `<!-- filing · doc_type=.. -->`,
        // 含换行/控制字符会破坏注释头 round-trip(stripFilingHeader 正则不跨行)。复用 safe_stem。
        let doc_type = safe_stem(doc_type_raw);
        if doc_type.is_empty() {
            return Err(ToolError::InvalidArgs("doc_type 不能为空".into()));
        }
        let title = require_str(args, "title")?;
        let content_md = require_str(args, "content_md")?;
        let cleaned = citations::parse(content_md).content_cleaned;

        let doc_id = persist_filing(ctx.pool, case_id, &doc_type, title, &cleaned)
            .await
            .map_err(ToolError::Runtime)?;

        Ok(ToolResult::plain(format!(
            "✅ 已生成《{title}》({doc_type}),已存为案件文档(doc_id={doc_id})。\
             \n用户可在文档预览界面点「导出为 Word(法律格式)」得到符合律所/法院排版的 .docx。\
             \n请勿把全文再整篇贴回答复;简要说明已生成 + 关键待律师核对项即可。\
             \n⚠️ 本文书为 AI 起草初稿,法条/案号/金额/日期/当事人信息须经执业律师核对后定稿。"
        )))
    }
}

/// 把文书落盘成 .md + INSERT 一行 documents(source='chat_artifact')。返回 doc_id。
///
/// MD 顶部写 `<!-- filing · doc_type=.. · title=.. -->` 元信息头(导出时解析 title;
/// `docx_filing` 渲染前会 strip 掉注释,不进 Word)。
pub(crate) async fn persist_filing(
    pool: &sqlx::SqlitePool,
    case_id: &str,
    doc_type: &str,
    title: &str,
    content_md: &str,
) -> Result<String, String> {
    let base = crate::db::app_data_dir().map_err(|e| format!("定位 app data dir 失败:{}", e))?;
    let dir = base.join("extracts").join(case_id).join("chat_artifacts");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("建目录失败:{}", e))?;

    let doc_id = uuid::Uuid::new_v4().to_string();
    let short = &doc_id[..8];
    let ts = chrono::Local::now().format("%Y-%m-%d_%H%M%S").to_string();
    let filename = format!("{}_{}_{}.md", safe_stem(title), ts, short);
    let path = dir.join(&filename);

    let body = format!(
        "<!-- filing · doc_type={} · title={} · ts={} -->\n\n{}",
        safe_filing_metadata(doc_type),
        safe_filing_metadata(title),
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
        content_md
    );
    if body.len() as u64 > ARTIFACT_MAX_BYTES {
        return Err(format!(
            "文书过大({:.1} MB),已拒绝写入;请拆成多份文书",
            body.len() as f64 / 1024.0 / 1024.0
        ));
    }
    tokio::fs::write(&path, &body)
        .await
        .map_err(|e| format!("写文书失败:{}", e))?;

    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let path_str = path.to_string_lossy().to_string();
    let inserted = sqlx::query(
        "INSERT INTO documents \
         (id, case_id, source_path, filename, stage, category, is_ai_artifact, \
          mime_type, size_bytes, modified_at, extraction_status, \
          extracted_text_path, source, created_at) \
         VALUES (?, ?, ?, ?, NULL, ?, 1, 'text/markdown', ?, ?, 'done', ?, 'chat_artifact', ?)",
    )
    .bind(&doc_id)
    .bind(case_id)
    .bind(&path_str)
    .bind(&filename)
    .bind(doc_type)
    .bind(body.len() as i64)
    .bind(&now)
    .bind(&path_str)
    .bind(&now)
    .execute(pool)
    .await;
    if let Err(db_err) = inserted {
        // 文件和 DB 必须一起成功。否则用户会看到「生成失败」,磁盘上却残留一个
        // 不受 CaseBoard 管理的半成品。
        return match tokio::fs::remove_file(&path).await {
            Ok(()) => Err(format!("登记文书失败:{db_err};已清理未登记文件")),
            Err(cleanup_err) => Err(format!(
                "登记文书失败:{db_err};未登记文件清理失败:{cleanup_err}"
            )),
        };
    }

    Ok(doc_id)
}

// =============================================================================
// edit_artifact(ADR-0003 Phase 1):对已生成文书做局部 find/replace
// =============================================================================

/// 修改后文书大小上限(防 replace 灌爆)。
const ARTIFACT_MAX_BYTES: u64 = 5 * 1024 * 1024;

pub struct EditArtifact;

#[async_trait]
impl Tool for EditArtifact {
    fn name(&self) -> &str {
        "edit_artifact"
    }
    fn is_mutating(&self) -> bool {
        true
    }
    fn description(&self) -> &str {
        include_str!("descriptions/edit_artifact.md")
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "doc_id": {
                    "type": "string",
                    "description": "要修改的文书 document id(用 save_artifact 返回的那个 / 系统提示里『当前编辑文书』的 doc_id)"
                },
                "find": {
                    "type": "string",
                    "description": "文书里要被替换掉的原文片段,需与文书内容逐字一致(标点也要对)。Markdown 加粗 ** 可省略(系统自动归一匹配)。片段要足够唯一,避免文书里出现多处相同文本"
                },
                "replace": {
                    "type": "string",
                    "description": "替换成的新内容,只写这一段的新文本,不要重写整篇。允许空字符串=删除该片段"
                },
                "context_before": {
                    "type": "string",
                    "description": "可选。find 紧邻的前文,文书里有多处相同 find 时用来定位"
                },
                "context_after": {
                    "type": "string",
                    "description": "可选。find 紧邻的后文,消歧用"
                }
            },
            "required": ["doc_id", "find", "replace"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let doc_id = require_str(args, "doc_id")?;
        let find = require_str(args, "find")?;
        // replace 允许空串(删除场景)→ 不能用拒空的 require_str
        let replace = args.get("replace").and_then(|v| v.as_str()).unwrap_or("");
        let ctx_before = args
            .get("context_before")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty());
        let ctx_after = args
            .get("context_after")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty());

        let out = apply_edit(ctx.pool, doc_id, find, replace, ctx_before, ctx_after)
            .await
            .map_err(|e| match e {
                // 匹配失败 / doc_id 无效 → 软失败(InvalidArgs),让模型换 find / 加锚点重试
                ApplyErr::Retryable(m) => ToolError::InvalidArgs(m),
                // 守卫拒绝 / IO / DB → 硬失败,提示用户
                ApplyErr::Hard(m) => ToolError::Runtime(m),
            })?;

        Ok(ToolResult::plain(format!(
            "✅ 已就地修改文书(doc_id={doc_id})。改动处当前内容:\n{}\n\
             若还要继续改本文书,直接再调 edit_artifact(同一 doc_id);**不要把全文贴回聊天**。",
            out.snippet
        )))
    }
}

/// `apply_edit` 的错误:区分「可重试」(改 find / 加锚点)和「硬失败」(守卫/IO/DB)。
enum ApplyErr {
    Retryable(String),
    Hard(String),
}

struct EditOutcome {
    /// 改动处附近的一小段新内容(给 AI 续改用,不回吐全文)。
    snippet: String,
}

/// edit_artifact 的可测内核:查文书 + 守卫 + 定位 find + 替换 + 写回 + 更新元信息。
/// **保留 filing 注释头原样**(不重生成 ts,避免文档无谓 churn)。
async fn apply_edit(
    pool: &sqlx::SqlitePool,
    doc_id: &str,
    find: &str,
    replace: &str,
    ctx_before: Option<&str>,
    ctx_after: Option<&str>,
) -> Result<EditOutcome, ApplyErr> {
    // 1. 查文书 + 守卫(复用 write_editor_doc 同款:仅 AI 产物 + source∈{chat,chat_artifact} 可改)
    let row: Option<(String, bool, String)> = sqlx::query_as(
        "SELECT source_path, is_ai_artifact, source FROM documents \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(doc_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApplyErr::Hard(format!("查文书失败:{}", e)))?;
    let (source_path, is_ai_artifact, source) =
        row.ok_or_else(|| ApplyErr::Retryable("文书不存在或已删除(doc_id 无效)".into()))?;
    if !is_ai_artifact {
        return Err(ApplyErr::Hard(
            "只能编辑 AI 生成的文书,不能改导入的原始文件".into(),
        ));
    }
    if source != "chat" && source != "chat_artifact" {
        return Err(ApplyErr::Hard(
            "只能编辑 AI 助手生成的文书,不能改导入/扫描的原始文件".into(),
        ));
    }

    // 2. 读文件 → 切 filing 头 → 在 body 上定位 find
    let raw = tokio::fs::read_to_string(&source_path)
        .await
        .map_err(|e| ApplyErr::Hard(format!("读文书失败:{}", e)))?;
    let (header, body) = split_filing_header(&raw);
    let (s, e) = locate_match(body, find, ctx_before, ctx_after).map_err(|le| match le {
        LocateError::NotFound => ApplyErr::Retryable(
            "在文书里没找到要替换的原文片段。请把 `find` 改成文书里逐字一致的一段\
             (可从刚才生成的内容里复制),或补 `context_before`/`context_after` 锚点后重试。"
                .into(),
        ),
        LocateError::Ambiguous(n) => ApplyErr::Retryable(format!(
            "要替换的片段在文书里出现了 {n} 处,无法确定改哪一处。\
             请把 `find` 写长一点(包含上下文使其唯一),或提供 context_before/context_after 锚点。"
        )),
    })?;

    // 3. 替换 + 重组(头原样保留)
    let mut new_body = String::with_capacity(body.len() + replace.len());
    new_body.push_str(&body[..s]);
    new_body.push_str(replace);
    new_body.push_str(&body[e..]);
    let new_raw = format!("{}{}", header, new_body);
    if new_raw.len() as u64 > ARTIFACT_MAX_BYTES {
        return Err(ApplyErr::Hard("修改后文书过大,已拒绝写入".into()));
    }

    tokio::fs::write(&source_path, &new_raw)
        .await
        .map_err(|e| ApplyErr::Hard(format!("写文书失败:{}", e)))?;
    let now_iso = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let update = sqlx::query("UPDATE documents SET size_bytes = ?, modified_at = ? WHERE id = ?")
        .bind(new_raw.len() as i64)
        .bind(&now_iso)
        .bind(doc_id)
        .execute(pool)
        .await;
    let update_error = match update {
        Ok(done) if done.rows_affected() == 1 => None,
        Ok(done) => Some(format!(
            "更新文书元信息失败:预期更新 1 行,实际更新 {} 行",
            done.rows_affected()
        )),
        Err(err) => Some(format!("更新文书元信息失败:{err}")),
    };
    if let Some(update_error) = update_error {
        // 内容写回成功但 DB 元信息失败时,把文件恢复到修改前,避免「磁盘内容已变、
        // 看板仍显示旧状态」的半成功。
        return match tokio::fs::write(&source_path, &raw).await {
            Ok(()) => Err(ApplyErr::Hard(format!("{update_error};已恢复修改前内容"))),
            Err(rollback_err) => Err(ApplyErr::Hard(format!(
                "{update_error};恢复修改前内容失败:{rollback_err}"
            ))),
        };
    }

    // 4. 改动处上下文(给 AI 续改用)
    let snippet = surrounding_snippet(&new_body, s, s + replace.len(), 40);
    Ok(EditOutcome { snippet })
}

/// 切出 filing 注释头(含其后的空白分隔)。返回 `(header_prefix, body)`,
/// body 上做 find/replace,重组时 `header_prefix + new_body` 原样保留头。
/// 无 filing 头(老格式 / 纯正文)时 header_prefix = "",body = 全文。
fn split_filing_header(raw: &str) -> (&str, &str) {
    if raw.starts_with("<!-- filing") {
        if let Some(end) = raw.find("-->") {
            let after = end + "-->".len();
            let rest = &raw[after..];
            // 把 --> 之后的换行/空白也并进 header_prefix,body 从正文首字符开始
            let ws_len = rest.len() - rest.trim_start_matches(['\n', '\r', ' ', '\t']).len();
            let split = after + ws_len;
            return (&raw[..split], &raw[split..]);
        }
    }
    ("", raw)
}

#[derive(Debug, PartialEq)]
enum LocateError {
    NotFound,
    Ambiguous(usize),
}

/// 在 body 中定位 find,返回唯一命中的 raw 字节范围 `[start, end)`。
/// ① 精确子串;② 失败则归一(剥 `*`/`_`/`` ` `` + 压空白)匹配,把边界 inline 标记一并吸收
/// (避免替换后留下悬空 `**`)。多处命中用 context_before/after(精确、紧邻)消歧。
fn locate_match(
    body: &str,
    find: &str,
    ctx_before: Option<&str>,
    ctx_after: Option<&str>,
) -> Result<(usize, usize), LocateError> {
    // tier 1:精确子串
    let mut cands = exact_ranges(body, find);
    // tier 2:归一匹配(精确没命中才用)
    if cands.is_empty() {
        cands = normalized_ranges(body, find);
    }
    if cands.is_empty() {
        return Err(LocateError::NotFound);
    }
    // context 消歧:前文以 ctx_before 结尾 且 后文以 ctx_after 开头
    let filtered: Vec<(usize, usize)> = cands
        .into_iter()
        .filter(|&(s, e)| {
            ctx_before.is_none_or(|c| body[..s].ends_with(c.trim()))
                && ctx_after.is_none_or(|c| body[e..].starts_with(c.trim()))
        })
        .collect();
    match filtered.len() {
        1 => Ok(filtered[0]),
        0 => Err(LocateError::NotFound),
        n => Err(LocateError::Ambiguous(n)),
    }
}

/// body 里 needle 的全部非重叠精确出现位置(raw 字节范围)。
fn exact_ranges(body: &str, needle: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut from = 0;
    while let Some(rel) = body[from..].find(needle) {
        let s = from + rel;
        let e = s + needle.len();
        out.push((s, e));
        from = e;
    }
    out
}

/// 归一表示:剥 inline 强调/代码标记 + 把连续空白压成单空格,
/// 同时保留「归一文本第 i 个 char → raw 字节范围」映射,命中后映射回源串。
struct NormMap {
    text: String,
    starts: Vec<usize>, // starts[i] = 第 i 个归一 char 的 raw 起始字节
    ends: Vec<usize>,   // ends[i]   = 第 i 个归一 char 的 raw 结束字节
}

fn normalize_for_match(raw: &str) -> NormMap {
    let mut text = String::with_capacity(raw.len());
    let mut starts = Vec::new();
    let mut ends = Vec::new();
    let mut prev_was_space = false;
    for (b, ch) in raw.char_indices() {
        let end = b + ch.len_utf8();
        if matches!(ch, '*' | '_' | '`') {
            // inline 标记:不进归一文本(prev_was_space 保持不变,让跨标记的空白 run 仍能合并)
            continue;
        }
        if ch.is_whitespace() {
            if prev_was_space {
                if let Some(last) = ends.last_mut() {
                    *last = end; // 延续空白 run:扩展上一个归一空格的 raw 结束位置
                }
            } else {
                text.push(' ');
                starts.push(b);
                ends.push(end);
                prev_was_space = true;
            }
        } else {
            text.push(ch);
            starts.push(b);
            ends.push(end);
            prev_was_space = false;
        }
    }
    NormMap { text, starts, ends }
}

/// 归一匹配:在 norm(body) 里找 norm(find),映射回 raw 范围并吸收边界 inline 标记。
fn normalized_ranges(body: &str, find: &str) -> Vec<(usize, usize)> {
    let nf = normalize_for_match(find);
    if nf.text.trim().is_empty() {
        return Vec::new();
    }
    let nb = normalize_for_match(body);
    let needle = nf.text.as_str();
    let hay = nb.text.as_str();
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let nstart = from + rel;
        let nend = nstart + needle.len();
        // norm 文本字节 offset → 归一 char index(均为 char 边界)
        let start_ci = hay[..nstart].chars().count();
        let end_ci = hay[..nend].chars().count(); // 排他
        if start_ci < nb.starts.len() && end_ci >= 1 && end_ci <= nb.ends.len() {
            let mut s = nb.starts[start_ci];
            let mut e = nb.ends[end_ci - 1];
            // 吸收紧邻边界的 inline 标记,避免替换后留悬空 **
            while s > 0 {
                if let Some(prev) = body[..s].chars().next_back() {
                    if matches!(prev, '*' | '_' | '`') {
                        s -= prev.len_utf8();
                        continue;
                    }
                }
                break;
            }
            while e < body.len() {
                if let Some(next) = body[e..].chars().next() {
                    if matches!(next, '*' | '_' | '`') {
                        e += next.len_utf8();
                        continue;
                    }
                }
                break;
            }
            out.push((s, e));
        }
        from = nend.max(from + 1);
    }
    out
}

/// 改动处附近的一小段新内容(char 安全,前后各取 ctx_chars 个 char),用 〖〗标出新片段。
fn surrounding_snippet(s: &str, start: usize, end: usize, ctx_chars: usize) -> String {
    let before: String = {
        let mut v: Vec<char> = s[..start].chars().rev().take(ctx_chars).collect();
        v.reverse();
        v.into_iter().collect()
    };
    let mid = &s[start..end];
    let after: String = s[end..].chars().take(ctx_chars).collect();
    format!("…{}〖{}〗{}…", before, mid, after)
}
