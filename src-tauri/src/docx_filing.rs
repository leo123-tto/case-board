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

use std::io::{Read, Write};

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use quick_xml::events::Event as XmlEvent;
use quick_xml::Reader as XmlReader;
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

/// 编辑器 Word 导出的稳定模板标识。
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WordTemplate {
    /// 保持 Milkdown 编辑器当前排版，也是缺省行为。
    #[default]
    Editor,
    /// 法律文书固定排版。
    LegalFiling,
}

impl WordTemplate {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "editor" => Ok(Self::Editor),
            "legal_filing" => Ok(Self::LegalFiling),
            _ => Err("Word 模板仅支持 editor 或 legal_filing".to_string()),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Editor => "editor",
            Self::LegalFiling => "legal_filing",
        }
    }
}

/// 法律文书页眉所需的可信本地字段。调用方只能用后端查询结果构造。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HeaderInputs {
    pub law_firm: Option<String>,
    pub document_title: Option<String>,
    pub case_name: Option<String>,
    pub case_no: Option<String>,
}

impl HeaderInputs {
    fn lines(&self) -> Vec<String> {
        let clean = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let mut lines = Vec::with_capacity(2);
        if let Some(law_firm) = clean(&self.law_firm) {
            lines.push(law_firm);
        }
        let case_line = [clean(&self.case_name), clean(&self.case_no)]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        if !case_line.is_empty() {
            lines.push(case_line);
        } else if let Some(document_title) = clean(&self.document_title) {
            lines.push(document_title);
        }
        lines
    }
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

fn validate_xml_1_0_text(context: &str, value: &str) -> Result<(), String> {
    if let Some(ch) = value.chars().find(|ch| {
        !matches!(
            *ch as u32,
            0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF
        )
    }) {
        return Err(format!(
            "OOXML {context}包含 XML 1.0 非法字符 U+{:04X}",
            ch as u32
        ));
    }
    Ok(())
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
    render_document_xml_with_references(title, body_md, profile, None, None)
}

fn render_document_xml_with_references(
    title: &str,
    body_md: &str,
    profile: Profile,
    header_relationship: Option<&str>,
    footer_relationship: Option<&str>,
) -> String {
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

    let sect = render_sectpr(header_relationship, footer_relationship);
    format!(
        "{decl}{open}<w:body>{body}{sect}</w:body></w:document>",
        decl = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#,
        open = DOC_OPEN,
        body = body,
        sect = sect,
    )
}

fn render_sectpr(header_relationship: Option<&str>, footer_relationship: Option<&str>) -> String {
    let mut references = String::new();
    if let Some(relationship) = header_relationship {
        references.push_str(&format!(
            r#"<w:headerReference w:type="default" r:id="{}"/>"#,
            xml_escape(relationship)
        ));
    }
    if let Some(relationship) = footer_relationship {
        references.push_str(&format!(
            r#"<w:footerReference w:type="default" r:id="{}"/>"#,
            xml_escape(relationship)
        ));
    }
    format!(
        r#"<w:sectPr>{references}<w:pgSz w:w="11906" w:h="16838" w:orient="portrait"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/><w:docGrid w:linePitch="360"/></w:sectPr>"#
    )
}

// ───────────────────────── 打包成 docx(zip) ─────────────────────────

/// 法律文书档:base + 法律叠加(列表去圆点 / 软换行并段 / 不渲染分隔线)。
/// 排版本身(方正小标宋居中标题 / 黑体小标题 / 仿宋正文 / 首行缩进 / 两端对齐 / 1.5 行距)与 base 一致。
pub fn build_filing_docx_bytes(title: &str, body_md: &str) -> Result<Vec<u8>, String> {
    build_docx_bytes(
        title,
        body_md,
        Profile::Filing,
        Some(&HeaderInputs::default()),
    )
}

/// 通用报告档:忠实 MD 渲染(无序列表带圆点 / 嵌套缩进 / 分隔线 / 保留结构)。
/// 案件分析报告、风险/深挖报告、通用 MD 导出走这条(替代旧的 textutil HTML 路径)。
pub fn build_report_docx_bytes(title: &str, body_md: &str) -> Result<Vec<u8>, String> {
    build_docx_bytes(title, body_md, Profile::Base, None)
}

/// Milkdown 可视编辑器导出档：保留编辑器的标题层级、正文字号/行距、
/// 列表、表格、加粗/斜体和预格式块，与报告档和法律文书档相互隔离。
pub fn build_editor_docx_bytes(title: &str, body_md: &str) -> Result<Vec<u8>, String> {
    build_docx_bytes(title, body_md, Profile::Editor, None)
}

/// 编辑器统一 Word 导出的模板感知入口。旧的 [`build_editor_docx_bytes`] 保持 Editor 默认。
pub fn build_editor_document_docx_bytes(
    title: &str,
    body_md: &str,
    template: WordTemplate,
    header_inputs: &HeaderInputs,
) -> Result<Vec<u8>, String> {
    match template {
        WordTemplate::Editor => build_docx_bytes(title, body_md, Profile::Editor, None),
        WordTemplate::LegalFiling => {
            build_docx_bytes(title, body_md, Profile::Filing, Some(header_inputs))
        }
    }
}

/// 把 (标题, 正文 MD, 档位) 打包成完整 .docx 字节流。`legal_header_inputs`
/// 为 `Some` 时才注册法律模板页眉/页脚部件。
fn build_docx_bytes(
    title: &str,
    body_md: &str,
    profile: Profile,
    legal_header_inputs: Option<&HeaderInputs>,
) -> Result<Vec<u8>, String> {
    validate_xml_1_0_text("标题", title)?;
    validate_xml_1_0_text("正文", body_md)?;
    let header_lines = legal_header_inputs
        .map(HeaderInputs::lines)
        .unwrap_or_default();
    for line in &header_lines {
        validate_xml_1_0_text("页眉", line)?;
    }
    let header_xml = (!header_lines.is_empty()).then(|| render_header_xml(&header_lines));
    let legal_template = legal_header_inputs.is_some();
    let document_xml = if legal_template {
        render_document_xml_with_references(
            title,
            body_md,
            profile,
            header_xml.as_ref().map(|_| "rId4"),
            Some("rId5"),
        )
    } else {
        render_document_xml(title, body_md, profile)
    };
    let content_types = render_content_types(legal_template, header_xml.is_some());
    let document_rels = render_document_relationships(legal_template, header_xml.is_some());
    let settings_xml = render_settings_xml(legal_template);
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
        put("[Content_Types].xml", &content_types)?;
        put("_rels/.rels", RELS_DOTRELS)?;
        put("word/_rels/document.xml.rels", &document_rels)?;
        put("word/styles.xml", STYLES_XML)?;
        put("word/settings.xml", &settings_xml)?;
        put("word/fontTable.xml", FONT_TABLE_XML)?;
        put("word/document.xml", &document_xml)?;
        if let Some(header_xml) = header_xml.as_deref() {
            put("word/header1.xml", header_xml)?;
        }
        if legal_template {
            put("word/footer1.xml", &render_footer_xml())?;
        }
        zip.finish().map_err(|e| format!("zip finish 失败:{}", e))?;
    }
    if legal_header_inputs.is_some() {
        validate_template_docx(&buf, WordTemplate::LegalFiling, !header_lines.is_empty())?;
    } else {
        validate_template_docx(&buf, WordTemplate::Editor, false)?;
    }
    Ok(buf)
}

fn render_header_xml(lines: &[String]) -> String {
    let mut body = String::new();
    for (index, line) in lines.iter().enumerate() {
        let border = if index + 1 == lines.len() {
            r#"<w:pBdr><w:bottom w:val="single" w:sz="6" w:space="4" w:color="808080"/></w:pBdr>"#
        } else {
            ""
        };
        body.push_str(&format!(
            r#"<w:p><w:pPr>{border}<w:spacing w:after="80"/><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:eastAsia="仿宋_GB2312"/><w:sz w:val="18"/><w:szCs w:val="18"/></w:rPr><w:t xml:space="preserve">{text}</w:t></w:r></w:p>"#,
            text = xml_escape(line)
        ));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">{body}</w:hdr>"#
    )
}

fn render_page_field(instruction: &str) -> String {
    format!(
        r#"<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> {instruction} </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>"#
    )
}

fn render_footer_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>第 </w:t></w:r>{page}<w:r><w:t> 页 / 共 </w:t></w:r>{pages}<w:r><w:t> 页</w:t></w:r></w:p></w:ftr>"#,
        page = render_page_field("PAGE"),
        pages = render_page_field("NUMPAGES")
    )
}

fn render_content_types(legal_template: bool, has_header: bool) -> String {
    let mut xml = CONTENT_TYPES
        .strip_suffix("</Types>")
        .expect("content types skeleton must end with Types")
        .to_string();
    if has_header {
        xml.push_str(r#"<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>"#);
    }
    if legal_template {
        xml.push_str(r#"<Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>"#);
    }
    xml.push_str("</Types>");
    xml
}

fn render_document_relationships(legal_template: bool, has_header: bool) -> String {
    let mut xml = DOCUMENT_RELS
        .strip_suffix("</Relationships>")
        .expect("relationship skeleton must end with Relationships")
        .to_string();
    if has_header {
        xml.push_str(r#"<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#);
    }
    if legal_template {
        xml.push_str(r#"<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>"#);
    }
    xml.push_str("</Relationships>");
    xml
}

fn render_settings_xml(legal_template: bool) -> String {
    if !legal_template {
        return SETTINGS_XML.to_string();
    }
    SETTINGS_XML.replacen(
        "<w:compat>",
        r#"<w:updateFields w:val="true"/><w:compat>"#,
        1,
    )
}

fn read_docx_entry(bytes: &[u8], name: &str) -> Result<String, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|error| format!("docx ZIP 无效:{error}"))?;
    let mut entry = archive
        .by_name(name)
        .map_err(|error| format!("docx 缺少 {name}:{error}"))?;
    let mut output = String::new();
    entry
        .read_to_string(&mut output)
        .map_err(|error| format!("读取 docx 部件 {name} 失败:{error}"))?;
    Ok(output)
}

fn docx_has_entry(bytes: &[u8], name: &str) -> Result<bool, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|error| format!("docx ZIP 无效:{error}"))?;
    let exists = archive.by_name(name).is_ok();
    Ok(exists)
}

fn validate_xml_part(part_name: &str, xml: &str, expected_root: &[u8]) -> Result<(), String> {
    let mut reader = XmlReader::from_str(xml);
    reader.config_mut().check_end_names = true;
    let mut depth = 0usize;
    let mut saw_root = false;
    let mut root_closed = false;

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(element)) => {
                if depth == 0 {
                    if saw_root {
                        return Err(format!("OOXML {part_name} 包含多个根元素"));
                    }
                    if element.name().local_name().as_ref() != expected_root {
                        return Err(format!("OOXML {part_name} 根元素无效"));
                    }
                    saw_root = true;
                } else if root_closed {
                    return Err(format!("OOXML {part_name} 根元素结束后仍有元素"));
                }
                depth += 1;
            }
            Ok(XmlEvent::Empty(element)) => {
                if depth == 0 {
                    if saw_root {
                        return Err(format!("OOXML {part_name} 包含多个根元素"));
                    }
                    if element.name().local_name().as_ref() != expected_root {
                        return Err(format!("OOXML {part_name} 根元素无效"));
                    }
                    saw_root = true;
                    root_closed = true;
                } else if root_closed {
                    return Err(format!("OOXML {part_name} 根元素结束后仍有元素"));
                }
            }
            Ok(XmlEvent::End(_)) => {
                if depth == 0 {
                    return Err(format!("OOXML {part_name} 包含多余结束标签"));
                }
                depth -= 1;
                if depth == 0 {
                    root_closed = true;
                }
            }
            Ok(XmlEvent::Text(text)) => {
                let raw: &[u8] = text.as_ref();
                if depth == 0 && raw.iter().any(|byte| !byte.is_ascii_whitespace()) {
                    return Err(format!("OOXML {part_name} 根元素外包含文本"));
                }
            }
            Ok(XmlEvent::CData(data)) => {
                let raw: &[u8] = data.as_ref();
                if depth == 0 && !raw.is_empty() {
                    return Err(format!("OOXML {part_name} 根元素外包含 CDATA"));
                }
            }
            Ok(XmlEvent::Eof) => {
                if !saw_root || depth != 0 || !root_closed {
                    return Err(format!("OOXML {part_name} XML 结构不完整"));
                }
                return Ok(());
            }
            Ok(_) => {}
            Err(error) => {
                return Err(format!("OOXML {part_name} XML 解析失败:{error}"));
            }
        }
    }
}

fn relationship_exists(
    part_name: &str,
    xml: &str,
    expected_type: &str,
    expected_target: &str,
) -> Result<bool, String> {
    let mut reader = XmlReader::from_str(xml);
    reader.config_mut().check_end_names = true;

    loop {
        match reader.read_event() {
            Ok(XmlEvent::Start(element)) | Ok(XmlEvent::Empty(element))
                if element.name().local_name().as_ref() == b"Relationship" =>
            {
                let mut relationship_type = None;
                let mut target = None;
                for attribute in element.attributes() {
                    let attribute = attribute
                        .map_err(|error| format!("OOXML {part_name} 属性解析失败:{error}"))?;
                    let value = attribute
                        .decode_and_unescape_value(reader.decoder())
                        .map_err(|error| format!("OOXML {part_name} 属性解码失败:{error}"))?;
                    match attribute.key.local_name().as_ref() {
                        b"Type" => relationship_type = Some(value.into_owned()),
                        b"Target" => target = Some(value.into_owned()),
                        _ => {}
                    }
                }
                if relationship_type.as_deref() == Some(expected_type)
                    && target.as_deref() == Some(expected_target)
                {
                    return Ok(true);
                }
            }
            Ok(XmlEvent::Eof) => return Ok(false),
            Ok(_) => {}
            Err(error) => {
                return Err(format!("OOXML {part_name} XML 解析失败:{error}"));
            }
        }
    }
}

/// 只校验本生成器承诺的模板关键结构，不扫描任意用户内容。
fn validate_template_docx(
    bytes: &[u8],
    template: WordTemplate,
    expects_header: bool,
) -> Result<(), String> {
    let content_types = read_docx_entry(bytes, "[Content_Types].xml")?;
    let root_relationships = read_docx_entry(bytes, "_rels/.rels")?;
    let relationships = read_docx_entry(bytes, "word/_rels/document.xml.rels")?;
    let document = read_docx_entry(bytes, "word/document.xml")?;
    let styles = read_docx_entry(bytes, "word/styles.xml")?;
    let settings = read_docx_entry(bytes, "word/settings.xml")?;
    let font_table = read_docx_entry(bytes, "word/fontTable.xml")?;
    for (part_name, xml, expected_root) in [
        (
            "[Content_Types].xml",
            content_types.as_str(),
            b"Types".as_slice(),
        ),
        (
            "_rels/.rels",
            root_relationships.as_str(),
            b"Relationships".as_slice(),
        ),
        (
            "word/_rels/document.xml.rels",
            relationships.as_str(),
            b"Relationships".as_slice(),
        ),
        (
            "word/document.xml",
            document.as_str(),
            b"document".as_slice(),
        ),
        ("word/styles.xml", styles.as_str(), b"styles".as_slice()),
        (
            "word/settings.xml",
            settings.as_str(),
            b"settings".as_slice(),
        ),
        (
            "word/fontTable.xml",
            font_table.as_str(),
            b"fonts".as_slice(),
        ),
    ] {
        validate_xml_part(part_name, xml, expected_root)?;
    }
    if !relationship_exists(
        "_rels/.rels",
        &root_relationships,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
        "word/document.xml",
    )? {
        return Err("docx 根关系缺少 officeDocument → word/document.xml".to_string());
    }

    match template {
        WordTemplate::Editor => {
            if docx_has_entry(bytes, "word/header1.xml")?
                || docx_has_entry(bytes, "word/footer1.xml")?
                || document.contains("w:headerReference")
                || document.contains("w:footerReference")
                || relationships.contains("relationships/header")
                || relationships.contains("relationships/footer")
                || content_types.contains("wordprocessingml.header+xml")
                || content_types.contains("wordprocessingml.footer+xml")
                || settings.contains("w:updateFields")
            {
                return Err("Editor 模板不得包含法律页眉页脚".to_string());
            }
        }
        WordTemplate::LegalFiling => {
            let footer = read_docx_entry(bytes, "word/footer1.xml")?;
            validate_xml_part("word/footer1.xml", &footer, b"ftr")?;
            if !document.contains(r#"<w:footerReference w:type="default" r:id="rId5"/>"#)
                || !relationships.contains(
                    r#"<Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>"#,
                )
                || !content_types.contains(
                    r#"<Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>"#,
                )
                || !settings.contains(r#"<w:updateFields w:val="true"/>"#)
            {
                return Err("法律文书模板页脚关系或字段更新设置无效".to_string());
            }
            for instruction in ["PAGE", "NUMPAGES"] {
                let node =
                    format!(r#"<w:instrText xml:space="preserve"> {instruction} </w:instrText>"#);
                if footer.matches(&node).count() != 1 {
                    return Err(format!("法律文书页脚字段 {instruction} 无效"));
                }
            }
            for field_type in ["begin", "separate", "end"] {
                let node = format!(r#"<w:fldChar w:fldCharType="{field_type}"/>"#);
                if footer.matches(&node).count() != 2 {
                    return Err(format!("法律文书页脚字段节点 {field_type} 无效"));
                }
            }
            let has_header = docx_has_entry(bytes, "word/header1.xml")?;
            if has_header != expects_header {
                return Err("法律文书页眉部件与可信元数据状态不一致".to_string());
            }
            if has_header {
                let header = read_docx_entry(bytes, "word/header1.xml")?;
                validate_xml_part("word/header1.xml", &header, b"hdr")?;
            }
            let document_has_header =
                document.contains(r#"<w:headerReference w:type="default" r:id="rId4"/>"#);
            let relationships_have_header = relationships.contains(
                r#"<Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>"#,
            );
            let content_types_have_header = content_types.contains(
                r#"<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>"#,
            );
            if [
                document_has_header,
                relationships_have_header,
                content_types_have_header,
            ]
            .into_iter()
            .any(|present| present != expects_header)
            {
                return Err("法律文书页眉关系与 section 引用不一致".to_string());
            }
        }
    }
    Ok(())
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

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/settings.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml"/><Override PartName="/word/fontTable.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.fontTable+xml"/></Types>"#;

const RELS_DOTRELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#;

const DOCUMENT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings" Target="settings.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/fontTable" Target="fontTable.xml"/></Relationships>"#;

const SETTINGS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml" mc:Ignorable="w14 w15"><w:evenAndOddHeaders w:val="false"/><w:compat><w:compatSetting w:val="15" w:uri="http://schemas.microsoft.com/office/word" w:name="compatibilityMode"/></w:compat></w:settings>"#;

const FONT_TABLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:fonts xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" mc:Ignorable="w14"/>"#;

/// styles.xml —— 取自样本(含 docDefaults + Word 默认标题样式定义)。本模块用 inline rPr,
/// 这些样式实际不引用,但保留以保证 Word 完整打开(reuse sample container)。
const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles mc:Ignorable="w14 w15" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"><w:docDefaults><w:rPrDefault/><w:pPrDefault/></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style></w:styles>"#;
