//! 案件分析报告导出(2026-05-24 j)。
//!
//! 输入:案件 ID(读 cases.case_report_path 拿到 MD 文件)
//! 输出:
//!   - HTML:**Kami 风格**(ink-blue + parchment + 衬线 + 封面/目录)单文件
//!     · 2026-05-26 V0.1.11 重写,对齐 https://github.com/tw93/Kami long-doc 模板设计 token
//!     · 主色 ink-blue #1B365D,标题左侧 2.5pt 色条而不是底部下划线
//!     · 字体 TsangerJinKai02 走 jsdelivr CDN + font-display: swap,断网回落 Source Han Serif
//!     · 加封面页 + 目录页(从 H2 提取),适合 A4 打印 / 发当事人 / 入卷
//!   - DOCX:走 `docx_filing` base 档(MD → 原生 OOXML,仿宋正文 / 黑体标题 / 表格 / 首行缩进),
//!     2026-06-04 从旧的 macOS `textutil` 路径切过来 —— **零外部依赖**(编进二进制,跨平台,表格不再散架)
//!
//! 前端通过 Tauri 文件保存 dialog 拿到目标路径,调下面两个 command 即可。

use std::io::Write as _;
use std::path::{Path, PathBuf};

use pulldown_cmark::{html, Options, Parser};
use sqlx::SqlitePool;

/// 把案件报告 MD 渲染成自定义样式的 HTML(单文件,内嵌 CSS)。
pub async fn render_report_html(pool: &SqlitePool, case_id: &str) -> Result<String, String> {
    // 1. 拿案件元数据 + 报告 MD 路径
    type CaseHeaderRow = (
        String,         // name
        Option<String>, // agg_case_no
        Option<String>, // agg_court
        Option<String>, // agg_status_text
        Option<String>, // case_summary
        Option<String>, // case_report_path
    );
    let row: Option<CaseHeaderRow> = sqlx::query_as(
        "SELECT name, agg_case_no, agg_court, agg_status_text, case_summary, case_report_path \
         FROM cases WHERE id = ?",
    )
    .bind(case_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查案件失败:{}", e))?;

    let (name, case_no, court, status_text, summary, report_path) =
        row.ok_or_else(|| "案件不存在".to_string())?;

    let path = report_path
        .ok_or_else(|| "该案件还未生成分析报告(请先点「📖 案件报告」按钮生成)".to_string())?;

    let md = std::fs::read_to_string(&path).map_err(|e| format!("读报告 MD 失败:{}", e))?;

    // 2. Markdown → HTML(走 pulldown-cmark)
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    // B13:不开智能标点 —— 与 Word 导出口径统一,避免把法律文书的直引号/连字符
    // 静默替换成弯引号/破折号(导出文本须与原 MD 字符一致)。
    let parser = Parser::new_ext(&md, opts);
    let mut body_html = String::new();
    html::push_html(&mut body_html, parser);

    // 3. 拼最终 HTML(陶土红 × 羊皮纸样式)
    let generated = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let html = build_html_template(BuildInputs {
        name: &name,
        case_no: case_no.as_deref(),
        court: court.as_deref(),
        status: status_text.as_deref(),
        summary: summary.as_deref(),
        body_html: &body_html,
        generated_at: &generated,
    });

    Ok(html)
}

struct BuildInputs<'a> {
    name: &'a str,
    case_no: Option<&'a str>,
    court: Option<&'a str>,
    status: Option<&'a str>,
    summary: Option<&'a str>,
    body_html: &'a str,
    generated_at: &'a str,
}

/// 从 body_html 提取 H2 章节标题给目录用。
///
/// pulldown-cmark 渲染出来的 H2 是 `<h2>章节名</h2>`(无 id)。返回 (anchor_id, title)。
/// 副作用:同时给原 body_html 里每个 `<h2>` 加 id="sec-N" 让 TOC 链接可点。
fn rewrite_h2_with_anchors_and_extract_toc(body_html: &str) -> (String, Vec<(String, String)>) {
    let mut toc: Vec<(String, String)> = Vec::new();
    let mut out = String::with_capacity(body_html.len() + 200);
    let mut i = 0;
    let mut idx = 1;
    while i < body_html.len() {
        if body_html[i..].starts_with("<h2>") {
            if let Some(end) = body_html[i + 4..].find("</h2>") {
                let title_html = &body_html[i + 4..i + 4 + end];
                let id = format!("sec-{}", idx);
                idx += 1;
                let title_text = strip_inline_tags(title_html);
                toc.push((id.clone(), title_text));
                out.push_str(&format!(r#"<h2 id="{}">"#, id));
                out.push_str(title_html);
                out.push_str("</h2>");
                i += 4 + end + 5;
                continue;
            }
        }
        // UTF-8 安全推一个字符(原版用 bytes[i] as char 把多字节中文拆烂了)
        let ch = body_html[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    (out, toc)
}

/// 把 HTML 内嵌标签去掉只留文本(只处理简单情况:`<strong>X</strong>` → `X`)
fn strip_inline_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }
    out
}

fn build_html_template(b: BuildInputs) -> String {
    // 1. body_html 加 H2 anchor + 提取 TOC
    let (body_with_anchors, toc) = rewrite_h2_with_anchors_and_extract_toc(b.body_html);

    // 2. 封面 meta 行(案号 / 法院 / 状态),按 kami long-doc 风格
    let cover_meta: String = [
        b.case_no.map(|x| ("案号", x)),
        b.court.map(|x| ("受理法院", x)),
        b.status.map(|x| ("当前状态", x)),
    ]
    .into_iter()
    .flatten()
    .map(|(label, val)| {
        format!(
            r#"<div class="cover-meta-row"><strong>{}</strong><span>{}</span></div>"#,
            html_escape(label),
            html_escape(val)
        )
    })
    .collect();

    // 3. 案件速览 block(封面下半)
    let summary_block = b
        .summary
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            format!(
                r#"<section class="cover-summary"><span class="cover-summary-label">案件速览</span><p>{}</p></section>"#,
                html_escape(s)
            )
        })
        .unwrap_or_default();

    // 4. 目录(2 个以上 H2 才显示)
    let toc_block = if toc.len() >= 2 {
        let items: String = toc
            .iter()
            .enumerate()
            .map(|(i, (id, title))| {
                format!(
                    r##"<a class="toc-item" href="#{id}"><span class="toc-num">{num:02}</span><span class="toc-title">{title}</span></a>"##,
                    id = id,
                    num = i + 1,
                    title = html_escape(title),
                )
            })
            .collect();
        format!(
            r#"<section class="toc">
  <h2 class="toc-h">目录</h2>
  {items}
</section>"#
        )
    } else {
        String::new()
    };

    format!(
        r##"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1.0">
<title>{title} · 案件分析报告</title>
<meta name="author" content="CaseBoard">
<meta name="generator" content="CaseBoard / kami long-doc">
<style>
{css}
</style>
</head>
<body>
  <!-- 封面 -->
  <section class="cover">
    <div class="cover-eyebrow">案件看板 · CASEBOARD</div>
    <h1 class="cover-title">{title_html}</h1>
    <div class="cover-sub">案件分析报告</div>
    <div class="cover-meta">{cover_meta}</div>
    {summary}
    <div class="cover-foot">报告生成于 {generated}</div>
  </section>

  <!-- 目录(可选) -->
  {toc}

  <!-- 正文 -->
  <article class="report-body">
    {body}
  </article>

  <footer class="report-footer">
    <span>报告由 CaseBoard 自动生成 · {generated}</span>
    <span class="footer-note">数据来源:案件全部文档,经 LLM 通读分析整理</span>
  </footer>
</body>
</html>"##,
        title = b.name,
        title_html = html_escape(b.name),
        css = REPORT_CSS,
        cover_meta = cover_meta,
        summary = summary_block,
        toc = toc_block,
        body = body_with_anchors,
        generated = b.generated_at,
    )
}

/// 陶土红 × 羊皮纸 法律文书专业风格 CSS
/// Kami long-doc 风格 CSS。设计 token 对齐 https://github.com/tw93/Kami
/// 主色 ink-blue #1B365D,parchment 米色底,TsangerJinKai02 衬线(CDN 拉)。
const REPORT_CSS: &str = r#"
/* ── Kami fonts (CDN with font-display: swap so 断网/慢网时立即用 fallback) ── */
@font-face {
  font-family: "TsangerJinKai02";
  src: url("https://cdn.jsdelivr.net/gh/tw93/Kami@main/assets/fonts/TsangerJinKai02-W04.ttf") format("truetype");
  font-weight: 400;
  font-style: normal;
  font-display: swap;
}
@font-face {
  font-family: "TsangerJinKai02";
  src: url("https://cdn.jsdelivr.net/gh/tw93/Kami@main/assets/fonts/TsangerJinKai02-W05.ttf") format("truetype");
  font-weight: 500;
  font-style: normal;
  font-display: swap;
}

* { box-sizing: border-box; margin: 0; padding: 0; }

:root {
  /* Kami parchment 体系 */
  --parchment: #f5f4ed;
  --ivory:     #faf9f5;
  --sand:      #e8e6dc;
  --border:    #e8e6dc;
  --border-soft:#e5e3d8;
  /* Kami ink-blue 品牌色 */
  --brand:        #1B365D;
  --brand-tint:   #EEF2F7;
  --brand-tint-2: #E4ECF5;
  /* 文字阶梯 */
  --near-black: #141413;
  --dark-warm:  #3d3d3a;
  --charcoal:   #4d4c48;
  --olive:      #504e49;
  --stone:      #6b6a64;
  /* 衬线字体栈,中文优先;Kami CDN 失败时回落系统衬线 */
  --serif: "TsangerJinKai02", "Source Han Serif SC", "Noto Serif CJK SC",
           "Songti SC", "STSong", Georgia, serif;
  --sans:  var(--serif);
  --shadow-soft: 0 8px 32px rgba(20, 20, 19, 0.06);
}

/* 打印:A4 + 适合订卷 / 发当事人,封面不带页眉页脚 */
@page {
  size: A4;
  margin: 20mm 22mm 22mm 22mm;
  background: var(--parchment);
}
@page:first {
  margin: 0;
}
html, body {
  background: var(--parchment);
  color: var(--near-black);
  font-family: var(--serif);
  font-size: 10.5pt;
  line-height: 1.65;
  letter-spacing: 0.3pt;
  -webkit-font-smoothing: antialiased;
  widows: 3;
  orphans: 3;
}

/* 屏幕预览时居中限宽 + 模拟 A4 卡片视觉 */
@media screen {
  body {
    max-width: 210mm;
    margin: 0 auto;
    padding: 16mm 22mm 22mm 22mm;
    background: var(--ivory);
    box-shadow: var(--shadow-soft);
    min-height: 100vh;
  }
}

/* ========== 封面 ========== */
.cover {
  min-height: 250mm;
  padding: 36mm 0 0 0;
  display: flex;
  flex-direction: column;
  break-after: page;
}
.cover-eyebrow {
  font-size: 10pt;
  color: var(--brand);
  letter-spacing: 1.5pt;
  font-weight: 500;
  margin-bottom: 18pt;
}
.cover-title {
  font-family: var(--serif);
  font-size: 32pt;
  font-weight: 500;
  color: var(--near-black);
  line-height: 1.18;
  letter-spacing: 0.3pt;
  margin-bottom: 12pt;
}
.cover-sub {
  font-size: 13pt;
  color: var(--olive);
  margin-bottom: 28pt;
}
.cover-meta {
  font-size: 10.5pt;
  color: var(--stone);
  line-height: 1.9;
  margin-bottom: 24pt;
}
.cover-meta-row {
  display: flex;
  gap: 14pt;
  align-items: baseline;
}
.cover-meta-row strong {
  display: inline-block;
  min-width: 60pt;
  color: var(--dark-warm);
  font-weight: 500;
  letter-spacing: 0.5pt;
}
.cover-meta-row span {
  color: var(--charcoal);
}
.cover-summary {
  margin-top: 12pt;
  padding: 14pt 18pt;
  background: var(--brand-tint);
  border-left: 2.5pt solid var(--brand);
  border-radius: 0 2pt 2pt 0;
  max-width: 90%;
}
.cover-summary-label {
  display: block;
  font-size: 9.5pt;
  letter-spacing: 1.2pt;
  color: var(--brand);
  font-weight: 500;
  margin-bottom: 6pt;
}
.cover-summary p {
  font-size: 11pt;
  color: var(--near-black);
  line-height: 1.7;
}
.cover-foot {
  margin-top: auto;
  padding-top: 24pt;
  font-size: 9pt;
  color: var(--stone);
  letter-spacing: 0.6pt;
}

/* ========== 目录 ========== */
.toc {
  break-after: page;
  padding-top: 8mm;
}
.toc-h {
  font-size: 22pt;
  font-weight: 500;
  margin-bottom: 16pt;
  border-left: 2.5pt solid var(--brand);
  padding-left: 8pt;
  color: var(--near-black);
}
.toc-item {
  display: flex;
  align-items: baseline;
  gap: 12pt;
  padding: 7pt 0;
  border-bottom: 0.3pt dotted var(--border);
  font-size: 11pt;
  text-decoration: none;
  color: var(--near-black);
}
.toc-item:last-of-type { border-bottom: none; }
.toc-num {
  color: var(--brand);
  font-weight: 500;
  min-width: 30pt;
  font-variant-numeric: tabular-nums;
}
.toc-title {
  flex: 1;
  color: var(--near-black);
}
.toc-item:hover {
  background: var(--brand-tint);
}

/* ========== 正文 ========== */
.report-body h2 {
  font-family: var(--serif);
  font-size: 16pt;
  font-weight: 500;
  line-height: 1.25;
  margin: 22pt 0 8pt;
  color: var(--near-black);
  border-left: 2.5pt solid var(--brand);
  border-radius: 1.5pt;
  padding-left: 8pt;
  break-after: avoid;
}
.report-body h2:first-child { margin-top: 0; }

.report-body h3 {
  font-size: 12pt;
  font-weight: 500;
  color: var(--near-black);
  margin: 14pt 0 6pt;
  break-after: avoid;
}

.report-body p {
  margin: 6pt 0;
  color: var(--dark-warm);
  line-height: 1.7;
  text-align: justify;
}
.report-body strong {
  color: var(--near-black);
  font-weight: 500;
}
.report-body ul, .report-body ol {
  margin: 8pt 0 8pt 18pt;
  color: var(--dark-warm);
}
.report-body li {
  margin: 4pt 0;
  line-height: 1.7;
}
.report-body li::marker {
  color: var(--brand);
}
.report-body blockquote {
  margin: 12pt 0;
  padding: 10pt 16pt;
  border-left: 2.5pt solid var(--brand);
  background: var(--brand-tint);
  border-radius: 0 2pt 2pt 0;
  color: var(--near-black);
  font-style: normal;
}
.report-body code {
  background: var(--sand);
  padding: 1pt 6pt;
  border-radius: 2pt;
  font-family: "SF Mono", "JetBrains Mono", Consolas, monospace;
  font-size: 9.5pt;
  color: var(--brand);
}
.report-body pre {
  background: var(--near-black);
  color: var(--sand);
  padding: 12pt 16pt;
  border-radius: 4pt;
  overflow-x: auto;
  margin: 12pt 0;
  font-family: "SF Mono", "JetBrains Mono", Consolas, monospace;
  font-size: 9pt;
  line-height: 1.5;
}
.report-body pre code {
  background: transparent;
  border: 0;
  color: inherit;
  padding: 0;
}
.report-body table {
  border-collapse: collapse;
  width: 100%;
  margin: 12pt 0;
  font-size: 10pt;
}
.report-body th, .report-body td {
  border: 0.5pt solid var(--border);
  padding: 7pt 10pt;
  text-align: left;
  vertical-align: top;
}
.report-body th {
  background: var(--brand-tint);
  font-weight: 500;
  color: var(--near-black);
  font-size: 9.5pt;
  letter-spacing: 0.3pt;
  border-bottom: 1pt solid var(--brand);
}
.report-body hr {
  margin: 20pt 0;
  border: 0;
  height: 0.5pt;
  background: var(--border);
}
.report-body a {
  color: var(--brand);
  text-decoration: none;
  border-bottom: 0.5pt solid var(--brand-tint-2);
}
.report-body a:hover { border-bottom-color: var(--brand); }

/* ========== 报告脚 ========== */
.report-footer {
  margin-top: 36pt;
  padding-top: 12pt;
  border-top: 0.5pt solid var(--border);
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 9pt;
  color: var(--stone);
  letter-spacing: 0.3pt;
}
.footer-note {
  font-style: italic;
}

/* ========== 打印优化 ========== */
@media print {
  body { background: white; padding: 0; max-width: 100%; box-shadow: none; }
  .cover, .toc, .report-body h2, .report-body h3 { page-break-after: avoid; }
  .toc-item { page-break-inside: avoid; }
  .report-body table { page-break-inside: auto; }
  .report-body blockquote, .report-body pre { page-break-inside: avoid; }
}
"#;

/// 简易 HTML escape — 防止案件名 / 元数据里有 `<` / `>` / `&` 等字符破坏 HTML
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// HTML 写盘到指定路径(前端通过 dialog/save 拿到 path 后调这个 command)。
pub async fn export_report_html_to(
    pool: &SqlitePool,
    case_id: &str,
    save_path: &Path,
) -> Result<PathBuf, String> {
    let html = render_report_html(pool, case_id).await?;
    std::fs::write(save_path, html).map_err(|e| format!("写 HTML 失败:{}", e))?;
    Ok(save_path.to_path_buf())
}

/// DOCX 导出:案件分析报告 → **原生 OOXML**(docx_filing base 档,仿宋正文 / 黑体标题 /
/// 表格 / 首行缩进 / 1.5 行距)。2026-06-04 从旧的 textutil HTML 路径切过来,**零外部依赖**
/// (编进二进制,装了 app 即可用,不再依赖系统 textutil;表格不再散架)。
pub async fn export_report_docx_to(
    pool: &SqlitePool,
    case_id: &str,
    save_path: &Path,
) -> Result<PathBuf, String> {
    type CaseHeaderRow = (
        String,         // name
        Option<String>, // agg_case_no
        Option<String>, // agg_court
        Option<String>, // agg_status_text
        Option<String>, // case_summary
        Option<String>, // case_report_path
    );
    let row: Option<CaseHeaderRow> = sqlx::query_as(
        "SELECT name, agg_case_no, agg_court, agg_status_text, case_summary, case_report_path \
         FROM cases WHERE id = ?",
    )
    .bind(case_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查案件失败:{}", e))?;
    let (name, case_no, court, status, summary, report_path) =
        row.ok_or_else(|| "案件不存在".to_string())?;
    let path = report_path.ok_or_else(|| "该案件还未生成分析报告".to_string())?;
    let md = std::fs::read_to_string(&path).map_err(|e| format!("读报告 MD 失败:{}", e))?;
    let title = if name.trim().is_empty() {
        "案件分析报告".to_string()
    } else {
        name
    };

    // 元信息抬头:与 HTML/Kami 导出口径一致(案号 / 受理法院 / 当前状态 / 案件速览)。
    // 报告正文是叙述体、不含案号等结构字段,补在标题与正文之间,避免 Word 比 HTML 少内容。
    let mut preamble = String::new();
    let mut push_meta = |label: &str, val: &Option<String>| {
        if let Some(v) = val.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            preamble.push_str(&format!("**{}:**{}\n\n", label, v));
        }
    };
    push_meta("案号", &case_no);
    push_meta("受理法院", &court);
    push_meta("当前状态", &status);
    push_meta("案件速览", &summary);
    if !preamble.is_empty() {
        preamble.push_str("---\n\n");
    }
    let generated = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    let body = format!("{preamble}{md}\n\n---\n\n报告由 CaseBoard 自动生成 · {generated}");

    let bytes = crate::docx_filing::build_report_docx_bytes(&title, &body)?;
    std::fs::write(save_path, &bytes).map_err(|e| format!("写 docx 失败:{}", e))?;
    Ok(save_path.to_path_buf())
}

/* ============================================================
 * 2026-05-25 V0.1.7 · 通用 MD 导出(任意 MD 路径 + 标题)
 *
 * 用途:让风险报告 / 深挖报告 / 完整报告 等"非主案件报告"
 * 也能走相同的 HTML / Word 导出管道。
 *
 * 跟 export_report_html_to 的区别:
 *   - 输入是 (md_path, title),不依赖 cases 表 / 不查 case_no/court 等元数据
 *   - 2026-05-26 V0.1.11:输出样式跟主案件报告一致 — **Kami long-doc 风格**(ink-blue + 衬线 + 封面 + 目录)
 *   - 没有 case_no / court / summary 时,封面只显示标题 + 副标题,目录仍会提取(如果 MD 有 ≥2 个 H2)
 *   - DOCX 路径(export_md_docx_to)2026-06-04 改走 docx_filing base 档(原生 OOXML,零外部依赖),HTML 路径仍走 Kami
 * ============================================================ */

/// 清掉 AI artifact 文件里的「非正文垃圾」:开头 `<!-- filing/chat artifact ... -->` 注释头,
/// 以及结尾 `<CITATIONS>...` 协议块(闭合或未闭合都剥)。给导出 / 渲染用,防元信息泄漏进
/// Word / HTML。新写入端已不带这些(write_chat_artifact 去头、citations.rs 剥未闭合块),
/// 本函数是「存量脏文件」的兜底(老板真机已生成带垃圾的 artifact)。
pub(crate) fn strip_artifact_cruft(md: &str) -> String {
    // 1) 开头 HTML 注释头(仅剥紧贴开头的一段)
    let mut body = md.trim_start();
    if body.starts_with("<!--") {
        if let Some(end) = body.find("-->") {
            body = body[end + 3..].trim_start();
        }
    }
    // 2) 结尾 <CITATIONS> 块(rfind;闭合到 </CITATIONS> 之后、未闭合到结尾,统一从 open 截断)
    let mut out = body.to_string();
    if let Some(open) = out.rfind("<CITATIONS>") {
        out.truncate(open);
    }
    out.trim_end().to_string()
}

/// 把 Milkdown 工作区文稿渲染为单文件 HTML。不套案件报告的封面/目录，
/// 只还原编辑器本身的文档层级、字体、行距、列表、表格、引用与预格式块。
pub(crate) fn render_editor_html(title: &str, md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(md, opts);
    let mut body_html = String::new();
    html::push_html(&mut body_html, parser);
    let title = html_escape(title.trim());

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title>
<style>
  * {{ box-sizing: border-box; }}
  html {{ background: #f5f5f4; }}
  body {{ margin: 0 auto; max-width: 50rem; min-height: 100vh; padding: 2.5rem 3rem 6rem; background: #fff; color: #1a1a1a; font-family: "Songti SC", "STSong", "SimSun", "Noto Serif SC", serif; font-size: 16px; line-height: 1.9; }}
  .document-title {{ margin: 0 0 1.4rem; font-size: 1.45rem; font-weight: 700; line-height: 1.5; text-align: center; }}
  main h1 {{ margin: 1.6rem 0 1.2rem; font-size: 1.45rem; font-weight: 700; line-height: 1.5; text-align: center; }}
  main h2 {{ margin: 1.4rem 0 .8rem; font-size: 1.18rem; font-weight: 700; line-height: 1.5; }}
  main h3 {{ margin: 1.1rem 0 .6rem; font-size: 1.05rem; font-weight: 700; line-height: 1.5; }}
  main h4, main h5, main h6 {{ margin: 1rem 0 .5rem; font-size: 1rem; font-weight: 700; line-height: 1.5; }}
  main p {{ margin: .6rem 0; text-align: justify; }}
  main ul, main ol {{ margin: .6rem 0; padding-left: 1.8rem; }}
  main li {{ margin: .25rem 0; }}
  main table {{ width: 100%; margin: 1rem 0; border-collapse: collapse; font-size: .95rem; }}
  main th, main td {{ padding: .45rem .7rem; border: 1px solid #c8c8c8; text-align: left; vertical-align: top; }}
  main th {{ background: #f4f4f4; font-weight: 700; }}
  main blockquote {{ margin: .8rem 0; padding-left: 1rem; border-left: 3px solid #d0d0d0; color: #555; }}
  main hr {{ margin: 1.5rem 0; border: 0; border-top: 1px solid #d8d8d8; }}
  main code {{ padding: .1rem .3rem; border-radius: 3px; background: #f0f0f0; font-family: ui-monospace, "SFMono-Regular", Menlo, monospace; font-size: .9em; }}
  main pre {{ margin: .8rem 0; padding: 1rem 1.25rem; overflow-x: auto; border-radius: 10px; background: #f3f4f6; white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.9; }}
  main pre code {{ padding: 0; background: transparent; font-family: inherit; font-size: 1em; }}
  @page {{ size: A4; margin: 25.4mm; }}
  @media print {{ html {{ background: #fff; }} body {{ max-width: none; min-height: auto; padding: 0; }} h1, h2, h3, h4, h5, h6 {{ break-after: avoid; }} table, blockquote, pre {{ break-inside: avoid; }} }}
</style>
</head>
<body>
<header class="document-title">{title}</header>
<main>{body_html}</main>
</body>
</html>"#,
    )
}

/// 两个 Milkdown 工作区共用的统一导出入口。
pub async fn export_editor_document_to(
    md_path: &Path,
    title: &str,
    format: &str,
    save_path: &Path,
) -> Result<PathBuf, String> {
    export_editor_document_with_template_to(
        md_path,
        title,
        format,
        save_path,
        None,
        &crate::docx_filing::HeaderInputs::default(),
    )
    .await
}

fn write_editor_export_atomically(
    save_path: &Path,
    bytes: &[u8],
    format_label: &str,
) -> Result<(), String> {
    let parent = save_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("写 {format_label} 失败:创建临时文件:{error}"))?;
    temporary
        .write_all(bytes)
        .map_err(|error| format!("写 {format_label} 失败:写临时文件:{error}"))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("写 {format_label} 失败:同步临时文件:{error}"))?;
    temporary
        .persist(save_path)
        .map_err(|error| format!("写 {format_label} 失败:原子替换:{}", error.error))?;
    Ok(())
}

/// 模板感知的编辑器导出入口。`word_template=None` 对 docx 稳定回退 Editor；
/// HTML 不接受 Word 模板。所有组合校验均发生在写盘之前。
pub async fn export_editor_document_with_template_to(
    md_path: &Path,
    title: &str,
    format: &str,
    save_path: &Path,
    word_template: Option<&str>,
    header_inputs: &crate::docx_filing::HeaderInputs,
) -> Result<PathBuf, String> {
    let template = match format {
        "docx" => Some(match word_template {
            Some(value) => crate::docx_filing::WordTemplate::parse(value)?,
            None => crate::docx_filing::WordTemplate::default(),
        }),
        "html" if word_template.is_some() => {
            return Err("HTML 导出不接受 Word 模板".to_string());
        }
        "html" => None,
        _ => return Err("仅支持导出 docx 或 html".to_string()),
    };
    let raw = std::fs::read_to_string(md_path).map_err(|e| format!("读 MD 失败:{e}"))?;
    let md = strip_artifact_cruft(&raw);
    match format {
        "docx" => {
            let bytes = crate::docx_filing::build_editor_document_docx_bytes(
                title,
                &md,
                template.expect("docx template resolved"),
                header_inputs,
            )?;
            write_editor_export_atomically(save_path, &bytes, "docx")?;
        }
        "html" => {
            let html = render_editor_html(title, &md);
            write_editor_export_atomically(save_path, html.as_bytes(), "HTML")?;
        }
        _ => unreachable!("format validated before reading"),
    }
    Ok(save_path.to_path_buf())
}

/// 从本地默认律师档案与可选案件记录解析法律页眉。前端只传案件 id，
/// 不传律所、案件名或案号正文。
pub(crate) async fn load_editor_header_inputs(
    pool: &SqlitePool,
    document_title: &str,
    case_id: Option<&str>,
) -> Result<crate::docx_filing::HeaderInputs, String> {
    let law_firm: Option<String> = sqlx::query_scalar(
        "SELECT trim(law_firm) FROM lawyer_profiles \
         WHERE is_default = 1 AND law_firm IS NOT NULL AND length(trim(law_firm)) > 0 \
         ORDER BY updated_at DESC, created_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("查默认律师档案失败:{error}"))?;

    if let Some(case_id) = case_id {
        let case_row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT trim(name), \
                    COALESCE(NULLIF(trim(agg_case_no), ''), NULLIF(trim(case_no), '')) \
             FROM cases WHERE id = ?",
        )
        .bind(case_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("查案件页眉信息失败:{error}"))?;
        let (case_name, case_no) =
            case_row.ok_or_else(|| "案件不存在，无法生成可信页眉".to_string())?;
        return Ok(crate::docx_filing::HeaderInputs {
            law_firm,
            case_name: (!case_name.is_empty()).then_some(case_name),
            case_no,
            document_title: None,
        });
    }

    let document_title = document_title.trim();
    Ok(crate::docx_filing::HeaderInputs {
        law_firm,
        document_title: (!document_title.is_empty()).then(|| document_title.to_string()),
        case_name: None,
        case_no: None,
    })
}

/// 通用 MD → 自定义样式 HTML(单文件,内嵌 CSS)。
pub async fn render_md_html(md_path: &Path, title: &str) -> Result<String, String> {
    let raw = std::fs::read_to_string(md_path).map_err(|e| format!("读 MD 失败:{}", e))?;
    let md = strip_artifact_cruft(&raw);
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    // B13:不开智能标点 —— 与 Word 导出口径统一,避免把法律文书的直引号/连字符
    // 静默替换成弯引号/破折号(导出文本须与原 MD 字符一致)。
    let parser = Parser::new_ext(&md, opts);
    let mut body_html = String::new();
    html::push_html(&mut body_html, parser);
    let generated = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
    Ok(build_html_template(BuildInputs {
        name: title,
        case_no: None,
        court: None,
        status: None,
        summary: None,
        body_html: &body_html,
        generated_at: &generated,
    }))
}

pub async fn export_md_html_to(
    md_path: &Path,
    title: &str,
    save_path: &Path,
) -> Result<PathBuf, String> {
    let html = render_md_html(md_path, title).await?;
    std::fs::write(save_path, html).map_err(|e| format!("写 HTML 失败:{}", e))?;
    Ok(save_path.to_path_buf())
}

/// 通用 MD → Word(docx_filing base 档,原生 OOXML)。风险/深挖报告、文书草稿等任意 MD 走这条。
/// 2026-06-04 从 textutil 切到原生引擎:零外部依赖、表格不散架、仿宋排版。
pub async fn export_md_docx_to(
    md_path: &Path,
    title: &str,
    save_path: &Path,
) -> Result<PathBuf, String> {
    let raw = std::fs::read_to_string(md_path).map_err(|e| format!("读 MD 失败:{}", e))?;
    let md = strip_artifact_cruft(&raw);
    let bytes = crate::docx_filing::build_report_docx_bytes(title, &md)?;
    std::fs::write(save_path, &bytes).map_err(|e| format!("写 docx 失败:{}", e))?;
    Ok(save_path.to_path_buf())
}
