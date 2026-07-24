//! Word 导出引擎(MD → 原生 OOXML · V0.3 起 · 2026-06-04 泛化为 base+profile)。
//!
//! **全 app 唯一的 docx 生成器**,固化 quote.law 15 份样本提炼出的法律级排版
//! (方正小标宋标题 / 黑体小标题 / 仿宋正文 / 两端对齐 / 首行缩进 2 字 / 1.5 倍行距),
//! 同一 run 上 `ascii=Times` + `eastAsia=仿宋` 的双字体是正式法院文书的灵魂 ——
//! 这正是被替代的 macOS `textutil` CSS 路径**结构上做不到**的(且 textutil 把 GFM 表格转散架)。
//! 纯 Rust、**零外部依赖**(编进二进制、跨平台,装了 app 即用)。
//!
//! ## 两档 [`Profile`](Profile)(共享上面全部排版,只差「是否忠实保留 MD 结构」)
//! - **base**([`build_report_docx_bytes`]):案件分析报告 / 风险·深挖报告 / 通用 MD 导出走这条。
//!   忠实渲染 —— 无序列表带圆点 + 嵌套左悬挂缩进、`---` 渲染成下边框分隔段。
//! - **filing**([`build_filing_docx_bytes`]):法律文书(起诉状等)走这条 = base + 法律叠加
//!   (无序列表去圆点、软/硬换行并段、`---` 丢弃),贴合法院文书惯例。
//!
//! ## 排版词汇表(从 15 份 quote.law 样本 docx XML **确定性提取**,见 docs/V0.3-文书格式规范)
//! | 角色 | eastAsia | sz(半点) | 对齐 | 首行缩进 |
//! |---|---|---|---|---|
//! | 文书标题 | 方正小标宋简体 | 32(16pt) | 居中 | 无 |
//! | 一级标题 | SimHei(黑体) | 30(15pt) | 两端 | 560twip(2字) |
//! | 二级标题 | SimHei(黑体) | 28(14pt) | 两端 | 560twip |
//! | 正文 | 仿宋_GB2312 | 28(14pt) | 两端 | 560twip |
//! | 强调正文 | 仿宋_GB2312 + 加粗 | 28 | 两端 | 560twip |
//!
//! 全局:A4(11906×16838)、四边页边距 1440twip(1 英寸)、docGrid linePitch=360、1.5 倍行距、
//! ascii=Times New Roman、**inline rPr 不靠段落样式**(quote.law 签名,本模块完全复刻)。
//!
//! ## Markdown → 角色映射约定
//!
//! - `title` 参数 → 文书标题(居中)
//! - MD 一/二级标题(`#` `##`)→ 一级标题(黑体 15pt)
//! - MD 三级及以下(`###`+)→ 二级标题(黑体 14pt)
//! - 普通段落 / 列表项 → 正文(仿宋 14pt);有序列表编号写进文本,无序列表 base 档加圆点(filing 不加)
//! - `---` 分隔线 → base 渲染成下边框段,filing 丢弃
//! - 段内 `**加粗**` → 该 run 加粗(强调正文)
//! - GFM 表格 → 仿宋正文表格(表头加粗,单线边框)
//!
//! 容器骨架(`[Content_Types]`/`_rels`/`styles`/`settings`/`sectPr`)取自真实样本,换取 Word 有效性。

use std::io::Write;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

// ───────────────────────── 角色 → 精确 OOXML 数值 ─────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Role {
    Title,
    H1,
    H2,
    H3,
    Body,
}

impl Role {
    fn east_asia(self, profile: Profile) -> &'static str {
        if profile == Profile::Editor {
            return "Songti SC";
        }
        match self {
            Role::Title => "方正小标宋简体",
            Role::H1 | Role::H2 | Role::H3 => "SimHei",
            Role::Body => "仿宋_GB2312",
        }
    }
    /// 字号(半点)
    fn sz(self, profile: Profile) -> &'static str {
        if profile == Profile::Editor {
            return match self {
                Role::Title | Role::H1 => "34",
                Role::H2 => "28",
                Role::H3 => "26",
                Role::Body => "24",
            };
        }
        match self {
            Role::Title => "32",
            Role::H1 => "30",
            Role::H2 | Role::H3 | Role::Body => "28",
        }
    }
    fn centered(self, profile: Profile) -> bool {
        matches!(self, Role::Title) || (profile == Profile::Editor && matches!(self, Role::H1))
    }
    fn first_line_indent(self, profile: Profile) -> bool {
        profile != Profile::Editor && !matches!(self, Role::Title)
    }
    fn default_bold(self, profile: Profile) -> bool {
        profile == Profile::Editor && !matches!(self, Role::Body)
    }
}

/// 导出排版档位。**base** = 忠实 MD 渲染(报告 / 通用 MD 走这条):保留无序列表圆点、
/// 嵌套缩进、分隔线;**filing** = 在 base 之上叠加法律文书的刻意简化(列表去圆点、
/// 软换行并段、不渲染分隔线),其余排版(仿宋正文 / 黑体标题 / 方正小标宋居中标题 /
/// 首行缩进 / 两端对齐 / 1.5 倍行距)两档完全一致。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Profile {
    /// 通用报告:忠实保留 MD 结构
    #[default]
    Base,
    /// 法律文书:base + 法律叠加
    Filing,
    /// Milkdown 可视编辑器文稿：标题层级、宋体正文、1.9 倍行距和预格式块
    /// 尽可能对齐 `src/components/editor/editor.css`。
    Editor,
}

struct Run {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
}

enum Block {
    /// `list_depth`:0 = 普通段落(按角色首行缩进);≥1 = 列表项嵌套层级(左悬挂缩进)
    Para {
        role: Role,
        runs: Vec<Run>,
        list_depth: u8,
    },
    Table {
        rows: Vec<TableRow>,
    },
    /// 围栏代码块 / 预格式块：保留原文换行和行首空格。
    Preformatted {
        text: String,
    },
    /// 分隔线(`---`),只在 base 档渲染
    Rule,
}

struct TableRow {
    header: bool,
    cells: Vec<Vec<Run>>,
}

// ───────────────────────── XML escape ─────────────────────────

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// 把中文语境里的半角标点规范化为全角(导出 Word 时静默执行,借鉴开源「文格」audit_text 的自动版)。
/// 保守策略:仅当标点紧邻 CJK 时转 —— 数字小数点/千分位(3.5 / 1,000)、英文、时间(3:30)、
/// 案号里的字母数字都不受影响;括号/引号因配对与语境(英文括号、代码)易误改,暂不自动转。
fn normalize_cjk_punct(s: &str) -> String {
    fn is_cjk(c: char) -> bool {
        matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    for (i, &c) in chars.iter().enumerate() {
        let prev_cjk = i > 0 && is_cjk(chars[i - 1]);
        let next = chars.get(i + 1).copied();
        let next_cjk = next.map(is_cjk).unwrap_or(false);
        let next_digit = next.map(|c| c.is_ascii_digit()).unwrap_or(false);
        let mapped = match c {
            // 逗号/句号:前后都须 CJK,避免 1,000 / 3.5 / 行尾英文缩写被误改
            ',' if prev_cjk && next_cjk => '，',
            '.' if prev_cjk && next_cjk => '。',
            // 分号/问号/叹号:前为 CJK 即可(无数字歧义)
            ';' if prev_cjk => '；',
            '?' if prev_cjk => '？',
            '!' if prev_cjk => '！',
            // 冒号:前为 CJK 且后非数字(避免时间 3:30、比例 2:1)
            ':' if prev_cjk && !next_digit => '：',
            _ => c,
        };
        out.push(mapped);
    }
    out
}

/// 去掉 HTML 注释(artifact MD 头部带 `<!-- chat artifact ... -->`),避免 pulldown 当内联 HTML。
fn strip_html_comments(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut rest = md;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find("-->") {
            rest = &rest[start + end + 3..];
        } else {
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

// ───────────────────────── Markdown → Blocks ─────────────────────────

#[derive(Default)]
struct Walker {
    profile: Profile,
    blocks: Vec<Block>,
    cur: Option<(Role, Vec<Run>, u8)>,
    ordered: Vec<Option<u64>>,
    bold_depth: u32,
    italic_depth: u32,
    code_block_text: Option<String>,
    // 表格累积
    table_rows: Option<Vec<TableRow>>,
    cur_row: Option<(bool, Vec<Vec<Run>>)>,
    cur_cell: Option<Vec<Run>>,
}

impl Walker {
    fn flush_cur(&mut self) {
        if let Some((role, runs, list_depth)) = self.cur.take() {
            // 丢弃完全空白段(只有空格)
            if runs.iter().any(|r| !r.text.trim().is_empty()) {
                self.blocks.push(Block::Para {
                    role,
                    runs,
                    list_depth,
                });
            }
        }
    }

    fn push_text(&mut self, t: &str) {
        // pulldown 对 CJK 紧邻的 `**` 不识别为加粗,会把分隔符当字面量 Text("*") 漏出来;
        // 纯星号 run 几乎不可能是正文内容(中文法律文书不含裸 `*`),直接丢弃防止 Word 里露出 `**`。
        if !t.is_empty() && t.chars().all(|c| c == '*') {
            return;
        }
        let bold = self.bold_depth > 0;
        let italic = self.italic_depth > 0;
        if let Some(cell) = self.cur_cell.as_mut() {
            cell.push(Run {
                text: t.to_string(),
                bold,
                italic,
                code: false,
            });
            return;
        }
        if self.cur.is_none() {
            self.cur = Some((Role::Body, Vec::new(), 0));
        }
        self.cur.as_mut().unwrap().1.push(Run {
            text: t.to_string(),
            bold,
            italic,
            code: false,
        });
    }

    fn push_code(&mut self, t: &str) {
        let bold = self.bold_depth > 0;
        let italic = self.italic_depth > 0;
        if let Some(cell) = self.cur_cell.as_mut() {
            cell.push(Run {
                text: t.to_string(),
                bold,
                italic,
                code: true,
            });
            return;
        }
        if self.cur.is_none() {
            self.cur = Some((Role::Body, Vec::new(), 0));
        }
        self.cur.as_mut().unwrap().1.push(Run {
            text: t.to_string(),
            bold,
            italic,
            code: true,
        });
    }

    fn walk(&mut self, parser: Parser) {
        for ev in parser {
            match ev {
                Event::Start(tag) => self.start(tag),
                Event::End(tag) => self.end(tag),
                // 正文文本做中文标点规范化(导出 Word 静默规范);Code(行内代码)原样不碰。
                Event::Text(t) => {
                    if let Some(code) = self.code_block_text.as_mut() {
                        code.push_str(&t);
                    } else {
                        self.push_text(&normalize_cjk_punct(&t));
                    }
                }
                Event::Code(t) => self.push_code(&t),
                // 内联/块级 HTML 当字面量文本处理(转义后输出),既不丢内容也不注入 HTML
                Event::Html(t) | Event::InlineHtml(t) => self.push_text(&t),
                // 软换行:中文文书同段内不插空格;硬换行同样并段(MVP)
                Event::SoftBreak | Event::HardBreak => {}
                // 分隔线 `---`:base 忠实渲染成下边框段,filing 沿用旧行为(丢弃)
                Event::Rule => {
                    self.flush_cur();
                    if matches!(self.profile, Profile::Base | Profile::Editor) {
                        self.blocks.push(Block::Rule);
                    }
                }
                _ => {}
            }
        }
        self.flush_cur();
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_cur();
                let role = match (self.profile, level) {
                    (Profile::Editor, HeadingLevel::H1) => Role::H1,
                    (Profile::Editor, HeadingLevel::H2) => Role::H2,
                    (Profile::Editor, _) => Role::H3,
                    (_, HeadingLevel::H1 | HeadingLevel::H2) => Role::H1,
                    _ => Role::H2,
                };
                self.cur = Some((role, Vec::new(), 0));
            }
            Tag::Paragraph | Tag::BlockQuote(_)
                if self.cur.is_none() && self.cur_cell.is_none() =>
            {
                self.cur = Some((Role::Body, Vec::new(), 0));
            }
            Tag::List(start) => self.ordered.push(start),
            Tag::Item => {
                self.flush_cur();
                let depth = self.ordered.len().max(1) as u8;
                let mut runs = Vec::new();
                // 有序列表:编号写进文本(两档一致);无序列表:base 加圆点,filing 不加(沿用旧行为)
                let prefix = match self.ordered.last_mut() {
                    Some(Some(n)) => {
                        let s = format!("{}. ", n);
                        *n += 1;
                        s
                    }
                    Some(None) if matches!(self.profile, Profile::Base | Profile::Editor) => {
                        "• ".to_string()
                    }
                    _ => String::new(),
                };
                if !prefix.is_empty() {
                    runs.push(Run {
                        text: prefix,
                        bold: false,
                        italic: false,
                        code: false,
                    });
                }
                // base 给列表项左悬挂缩进(按嵌套层级);filing 保持普通段落(首行缩进)
                let list_depth = if matches!(self.profile, Profile::Base | Profile::Editor) {
                    depth
                } else {
                    0
                };
                self.cur = Some((Role::Body, runs, list_depth));
            }
            Tag::Strong => self.bold_depth += 1,
            Tag::Emphasis => self.italic_depth += 1,
            Tag::CodeBlock(_) => {
                self.flush_cur();
                self.code_block_text = Some(String::new());
            }
            Tag::Table(_) => {
                self.flush_cur();
                self.table_rows = Some(Vec::new());
            }
            Tag::TableHead => self.cur_row = Some((true, Vec::new())),
            Tag::TableRow => self.cur_row = Some((false, Vec::new())),
            Tag::TableCell => self.cur_cell = Some(Vec::new()),
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::Item | TagEnd::BlockQuote(_) => {
                self.flush_cur()
            }
            TagEnd::List(_) => {
                self.ordered.pop();
            }
            TagEnd::Strong => self.bold_depth = self.bold_depth.saturating_sub(1),
            TagEnd::Emphasis => self.italic_depth = self.italic_depth.saturating_sub(1),
            TagEnd::CodeBlock => {
                if let Some(mut text) = self.code_block_text.take() {
                    // pulldown-cmark 会把围栏闭合前的换行收进 Text；这个结尾换行
                    // 是 Markdown 语法边界，不是用户想在 Word 中多出的空行。
                    if text.ends_with('\n') {
                        text.pop();
                        if text.ends_with('\r') {
                            text.pop();
                        }
                    }
                    if !text.is_empty() {
                        self.blocks.push(Block::Preformatted { text });
                    }
                }
            }
            TagEnd::TableCell => {
                if let (Some(cell), Some(row)) = (self.cur_cell.take(), self.cur_row.as_mut()) {
                    row.1.push(cell);
                }
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if let (Some((header, cells)), Some(rows)) =
                    (self.cur_row.take(), self.table_rows.as_mut())
                {
                    rows.push(TableRow { header, cells });
                }
            }
            TagEnd::Table => {
                if let Some(rows) = self.table_rows.take() {
                    self.blocks.push(Block::Table { rows });
                }
            }
            _ => {}
        }
    }
}

fn parse_blocks(body_md: &str, profile: Profile) -> Vec<Block> {
    let cleaned = strip_html_comments(body_md);
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(&cleaned, opts);
    let mut w = Walker {
        profile,
        ..Default::default()
    };
    w.walk(parser);
    w.blocks
}

// ───────────────────────── Blocks → document.xml ─────────────────────────

fn render_run(run: &Run, role: Role, profile: Profile) -> String {
    if run.text.is_empty() {
        return String::new();
    }
    let b = if run.bold || role.default_bold(profile) {
        "<w:b/><w:bCs/>"
    } else {
        ""
    };
    let i = if run.italic { "<w:i/><w:iCs/>" } else { "" };
    let code = if run.code && profile == Profile::Editor {
        "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F0F0F0\"/>"
    } else {
        ""
    };
    let ascii = if run.code && profile == Profile::Editor {
        "Menlo"
    } else {
        "Times New Roman"
    };
    format!(
        "<w:r><w:rPr><w:rFonts w:ascii=\"{ascii}\" w:eastAsia=\"{ea}\"/>{b}{i}{code}\
         <w:sz w:val=\"{sz}\"/><w:szCs w:val=\"{sz}\"/></w:rPr>\
         <w:t xml:space=\"preserve\">{t}</w:t></w:r>",
        ascii = ascii,
        ea = role.east_asia(profile),
        b = b,
        i = i,
        code = code,
        sz = role.sz(profile),
        t = xml_escape(&run.text),
    )
}

fn render_para(role: Role, runs: &[Run], list_depth: u8, profile: Profile) -> String {
    let spacing = if profile == Profile::Editor {
        match role {
            Role::Title | Role::H1 => {
                "<w:spacing w:before=\"320\" w:after=\"240\" w:line=\"360\" w:lineRule=\"auto\"/><w:keepNext/>"
            }
            Role::H2 => {
                "<w:spacing w:before=\"280\" w:after=\"160\" w:line=\"360\" w:lineRule=\"auto\"/><w:keepNext/>"
            }
            Role::H3 => {
                "<w:spacing w:before=\"220\" w:after=\"120\" w:line=\"360\" w:lineRule=\"auto\"/><w:keepNext/>"
            }
            Role::Body => {
                "<w:spacing w:before=\"0\" w:after=\"144\" w:line=\"456\" w:lineRule=\"auto\"/>"
            }
        }
    } else {
        "<w:spacing w:line=\"360\" w:lineRule=\"auto\"/>"
    };
    let mut s = format!("<w:p><w:pPr>{spacing}");
    if list_depth > 0 {
        // 列表项:按嵌套层级左缩进 + 悬挂缩进(换行后对齐到圆点/编号之后)
        let left = 420 * list_depth as i32 + 280;
        s.push_str(&format!("<w:ind w:left=\"{}\" w:hanging=\"280\"/>", left));
    } else if role.first_line_indent(profile) {
        s.push_str("<w:ind w:firstLine=\"560\"/>");
    }
    s.push_str(if role.centered(profile) {
        "<w:jc w:val=\"center\"/>"
    } else if profile == Profile::Editor && matches!(role, Role::H2 | Role::H3) {
        "<w:jc w:val=\"left\"/>"
    } else {
        "<w:jc w:val=\"both\"/>"
    });
    s.push_str("</w:pPr>");
    for r in runs {
        s.push_str(&render_run(r, role, profile));
    }
    s.push_str("</w:p>");
    s
}

/// 分隔线(`---`)→ 一个带下边框的空段(base 档)。
fn render_rule() -> &'static str {
    "<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"auto\"/></w:pBdr></w:pPr></w:p>"
}

/// 表格单元格里的段落:正文字体,不缩进(表格内),表头加粗。
fn render_cell(cell: &[Run], header: bool, profile: Profile) -> String {
    let fill = if header && profile == Profile::Editor {
        "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F4F4F4\"/>"
    } else {
        ""
    };
    let mut s = String::from("<w:tc><w:tcPr><w:tcW w:w=\"0\" w:type=\"auto\"/>");
    s.push_str(fill);
    s.push_str("</w:tcPr><w:p><w:pPr><w:spacing w:line=\"360\" w:lineRule=\"auto\"/><w:jc w:val=\"left\"/></w:pPr>");
    // 空单元格也合法:留一个 run-less 段(上面的 <w:p> 已含),Word 才认
    for r in cell {
        let run = Run {
            text: r.text.clone(),
            bold: r.bold || header,
            italic: r.italic,
            code: r.code,
        };
        s.push_str(&render_run(&run, Role::Body, profile));
    }
    s.push_str("</w:p></w:tc>");
    s
}

fn render_table(rows: &[TableRow], profile: Profile) -> String {
    let cols = rows.iter().map(|r| r.cells.len()).max().unwrap_or(1).max(1);
    // 文字区宽 = 11906 - 左右各 1440 = 9026 twip,均分
    let colw = 9026 / cols as i32;
    let mut grid = String::from("<w:tblGrid>");
    for _ in 0..cols {
        grid.push_str(&format!("<w:gridCol w:w=\"{}\"/>", colw));
    }
    grid.push_str("</w:tblGrid>");

    let mut s = String::from(
        "<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/>\
         <w:tblBorders>\
         <w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"000000\"/>\
         <w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"000000\"/>\
         <w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"000000\"/>\
         <w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"000000\"/>\
         <w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"000000\"/>\
         <w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"000000\"/>\
         </w:tblBorders></w:tblPr>",
    );
    s.push_str(&grid);
    for row in rows {
        s.push_str("<w:tr>");
        for cell in &row.cells {
            s.push_str(&render_cell(cell, row.header, profile));
        }
        s.push_str("</w:tr>");
    }
    s.push_str("</w:tbl>");
    s
}

/// 编辑器中的预格式块→ Word 浅灰底文本块。用 `<w:br/>` 明确表示换行，
/// 避免 OOXML 把 `<w:t>` 里的原始换行归一化为空格，导致流程图横向串行。
fn render_preformatted(text: &str) -> String {
    let mut s = String::from(
        "<w:p><w:pPr><w:spacing w:before=\"120\" w:after=\"120\" w:line=\"360\" w:lineRule=\"auto\"/>\
         <w:ind w:left=\"280\" w:right=\"280\"/><w:jc w:val=\"left\"/><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F3F4F6\"/>\
         <w:keepLines/></w:pPr>",
    );
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            s.push_str("<w:r><w:br/></w:r>");
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        if !line.is_empty() {
            s.push_str(&format!(
                "<w:r><w:rPr><w:rFonts w:ascii=\"Menlo\" w:eastAsia=\"Songti SC\"/>\
                 <w:sz w:val=\"24\"/><w:szCs w:val=\"24\"/></w:rPr>\
                 <w:t xml:space=\"preserve\">{}</w:t></w:r>",
                xml_escape(line)
            ));
        }
    }
    s.push_str("</w:p>");
    s
}

/// 生成完整的 `word/document.xml`。`pub(crate)` 供测试做结构断言。
pub(crate) fn render_document_xml(title: &str, body_md: &str, profile: Profile) -> String {
    let mut blocks = parse_blocks(body_md, profile);

    // 去重:若正文首块是与 title 同名的标题,丢掉(LLM 常在正文重复写标题)
    let title_trim = title.trim();
    if let Some(Block::Para { role, runs, .. }) = blocks.first() {
        if matches!(role, Role::H1) {
            let txt: String = runs.iter().map(|r| r.text.as_str()).collect();
            if txt.trim() == title_trim && !title_trim.is_empty() {
                blocks.remove(0);
            }
        }
    }

    let mut body = String::new();
    // 文书标题(总在最前)
    if !title_trim.is_empty() {
        body.push_str(&render_para(
            Role::Title,
            &[Run {
                text: title_trim.to_string(),
                bold: false,
                italic: false,
                code: false,
            }],
            0,
            profile,
        ));
    }

    let last_is_table = matches!(blocks.last(), Some(Block::Table { .. }));
    for b in &blocks {
        match b {
            Block::Para {
                role,
                runs,
                list_depth,
            } => body.push_str(&render_para(*role, runs, *list_depth, profile)),
            Block::Table { rows } => body.push_str(&render_table(rows, profile)),
            Block::Preformatted { text } => body.push_str(&render_preformatted(text)),
            Block::Rule => body.push_str(render_rule()),
        }
    }
    // OOXML 要求表格后须有段落;末块是表格时补一个空段
    if last_is_table {
        body.push_str("<w:p/>");
    }

    format!(
        "{decl}{open}<w:body>{body}{sect}</w:body></w:document>",
        decl = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        open = DOC_OPEN,
        body = body,
        sect = SECTPR,
    )
}

// ───────────────────────── 打包成 docx(zip) ─────────────────────────

/// 法律文书档:base + 法律叠加(列表去圆点 / 软换行并段 / 不渲染分隔线)。
/// 排版本身(方正小标宋居中标题 / 黑体小标题 / 仿宋正文 / 首行缩进 / 两端对齐 / 1.5 行距)与 base 一致。
pub fn build_filing_docx_bytes(title: &str, body_md: &str) -> Result<Vec<u8>, String> {
    build_docx_bytes(title, body_md, Profile::Filing)
}

/// 通用报告档:忠实 MD 渲染(无序列表带圆点 / 嵌套缩进 / 分隔线 / 保留结构)。
/// 案件分析报告、风险/深挖报告、通用 MD 导出走这条(替代旧的 textutil HTML 路径)。
pub fn build_report_docx_bytes(title: &str, body_md: &str) -> Result<Vec<u8>, String> {
    build_docx_bytes(title, body_md, Profile::Base)
}

/// Milkdown 可视编辑器导出档：保留编辑器的标题层级、正文字号/行距、
/// 列表、表格、加粗/斜体和预格式块，与报告档和法律文书档相互隔离。
pub fn build_editor_docx_bytes(title: &str, body_md: &str) -> Result<Vec<u8>, String> {
    build_docx_bytes(title, body_md, Profile::Editor)
}

/// 把 (标题, 正文 MD, 档位) 打包成完整 .docx 字节流。纯函数,便于测试。
fn build_docx_bytes(title: &str, body_md: &str, profile: Profile) -> Result<Vec<u8>, String> {
    let document_xml = render_document_xml(title, body_md, profile);
    let mut buf = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut zip = ZipWriter::new(cursor);
        let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
        let mut put = |name: &str, data: &str| -> Result<(), String> {
            zip.start_file(name, opts)
                .map_err(|e| format!("zip start_file {} 失败:{}", name, e))?;
            zip.write_all(data.as_bytes())
                .map_err(|e| format!("zip write {} 失败:{}", name, e))?;
            Ok(())
        };
        put("[Content_Types].xml", CONTENT_TYPES)?;
        put("_rels/.rels", RELS_DOTRELS)?;
        put("word/_rels/document.xml.rels", DOCUMENT_RELS)?;
        put("word/styles.xml", STYLES_XML)?;
        put("word/settings.xml", SETTINGS_XML)?;
        put("word/fontTable.xml", FONT_TABLE_XML)?;
        put("word/footer1.xml", FOOTER1_XML)?;
        put("word/document.xml", &document_xml)?;
        zip.finish().map_err(|e| format!("zip finish 失败:{}", e))?;
    }
    Ok(buf)
}

/// 从 `save_artifact` 写的元信息头 `<!-- filing · doc_type=.. · title=.. · ts=.. -->`
/// 解析出文书标题(导出 Word 时作居中大标题)。无头则返回 None,调用方用文件名兜底。
pub fn extract_filing_title(md: &str) -> Option<String> {
    let start = md.find("<!-- filing")?;
    let end = md[start..].find("-->")? + start;
    let header = &md[start..end];
    let key = "title=";
    let kpos = header.find(key)? + key.len();
    let rest = &header[kpos..];
    // title 值到下一个 ` · ` 分隔或注释尾
    let val = rest.split(" · ").next().unwrap_or(rest).trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

// ───────────────────────── 内嵌容器骨架(取自真实样本) ─────────────────────────

const DOC_OPEN: &str = r#"<w:document xmlns:wpc="http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:o="urn:schemas-microsoft-com:office:office" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" xmlns:v="urn:schemas-microsoft-com:vml" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:w10="urn:schemas-microsoft-com:office:word" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" mc:Ignorable="w14 w15">"#;

/// 页面/页边距/版式网格 —— 与全部 15 份样本字节级一致(A4 / 1英寸边距 / docGrid linePitch=360)。
/// 页脚引用 rId4 → word/footer1.xml(PAGE / NUMPAGES 字段,见 `FOOTER1_XML`)。
const SECTPR: &str = r#"<w:sectPr><w:footerReference r:id="rId4" w:type="default"/><w:pgSz w:w="11906" w:h="16838" w:orient="portrait"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/><w:docGrid w:linePitch="360"/></w:sectPr>"#;

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/><Override PartName="/word/fontTable.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/></Types>"#;

const RELS_DOTRELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const DOCUMENT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable" Target="fontTable.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/></Relationships>"#;

/// 页脚部件:居中「第 PAGE 页 / 共 NUMPAGES 页」。PAGE/NUMPAGES 用 `fldSimple` 字段,
/// **不硬编码数字**(文格 / 法院对「页码硬编码」零容忍,删段后字段会自动重排)。
/// 字体:小五号(18 半点),ascii=Times, eastAsia=仿宋,与正文同字体族。
const FOOTER1_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" mc:Ignorable="w14 w15"><w:p><w:pPr><w:jc w:val="center"/><w:spacing w:line="240" w:lineRule="auto"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:eastAsia="仿宋_GB2312"/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t xml:space="preserve">第 </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:eastAsia="仿宋_GB2312"/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t>1</w:t></w:r></w:fldSimple><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:eastAsia="仿宋_GB2312"/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t xml:space="preserve"> 页 / 共 </w:t></w:r><w:fldSimple w:instr=" NUMPAGES "><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:eastAsia="仿宋_GB2312"/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t>1</w:t></w:r></w:fldSimple><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:eastAsia="仿宋_GB2312"/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t xml:space="preserve"> 页</w:t></w:r></w:p></w:ftr>"#;

const SETTINGS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" mc:Ignorable="w14 w15"><w:evenAndOddHeaders w:val="false"/><w:compat><w:compatSetting w:val="15" w:uri="http://schemas.microsoft.com/office/word" w:name="compatibilityMode"/></w:compat></w:settings>"#;

const FONT_TABLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:fonts xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" mc:Ignorable="w14"/>"#;

/// styles.xml —— 取自样本(含 docDefaults + Word 默认标题样式定义)。本模块用 inline rPr,
/// 这些样式实际不引用,但保留以保证 Word 完整打开(reuse sample container)。
const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles mc:Ignorable="w14 w15" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w:docDefaults><w:rPrDefault/><w:pPrDefault/></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style></w:styles>"#;
