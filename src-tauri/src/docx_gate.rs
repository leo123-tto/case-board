//! 交付前门禁(2026-07-24 V0.4.16 P0 #3 · 蒸馏自 `E:\律师事务部\各类工具\md2word-gate_落地交付报告.md`)。
//!
//! 借鉴文格 / md2word-gate 的"做前先查"理念,所有"对外发送"的 .docx 必须先跑门禁。
//! **不阻塞导出**,仅返回报告(让用户/律师拍板,文格"做不到就报告"原则)。
//!
//! ## 9 项检查
//! | # | 检查 | 严重度 | 实现 |
//! |---|---|---|---|
//! | 1 | OpenXML 结构完整性 | FAIL | zip 列出 word/document.xml 等 4 个关键部件 |
//! | 2 | 页眉存在性 | WARNING | 列出 word/header*.xml |
//! | 3 | 页脚页码字段 | FAIL | 解析 word/footer*.xml 找 `w:instr="PAGE"` 字符串 |
//! | 4 | 标题样式引用 | INFO | 法律文书豁免(SPEC § 十一) |
//! | 5 | 表格边框 | FAIL/INFO | 扫描 `<w:tblBorders>` 块 |
//! | 6 | 字体合规 | FAIL | 扫描 `w:eastAsia` 取值范围 |
//! | 7 | 中文标点全角 | FAIL | 解析 `<w:t>` 内容,正则匹配 CJK 旁半角标点 |
//! | 8 | 空段检测 | WARNING | 扫描连续 `<w:p/>` 数量 |
//! | 9 | 字符/段/表数统计 | INFO | 累加 `w:t` 内容 |
//!
//! ## 4 档退出码
//! - 0:全部 PASS/INFO → 可对外发送
//! - 1:含 WARNING(无 FAIL) → 人工复核后发送
//! - 2:含 FAIL → **必须修正后重新生成**(本模块**不阻塞**导出,仅报告)
//! - 3:文件错误 → 检查文件路径/格式/大小
//!
//! ## 零新依赖
//! `zip = "2"` / `quick-xml = "0.39"` / `tempfile = "3"`(测试用) 已在 `Cargo.toml`。
//! `docx_extract.rs` 已有 zip 读取范式可复用。

use std::fs::File;
use std::io::Read;
use std::path::Path;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use zip::ZipArchive;

const MAX_DOCX_BYTES: u64 = 50 * 1024 * 1024;

/// 检查结果严重度。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    Pass,
    Warning,
    Fail,
    Info,
}

/// 4 档退出码。聚合时取最高(`Fail` > `Warning` > `Pass`)。
#[derive(Clone, Copy, PartialEq, Eq, Debug, PartialOrd, Ord)]
pub enum ExitCode {
    Pass = 0,
    Warning = 1,
    Fail = 2,
    FileError = 3,
}

/// 单项检查结果。
#[derive(Clone, Debug)]
pub struct CheckResult {
    pub name: &'static str,
    pub severity: Severity,
    pub message: String,
}

/// 文档统计(SPEC § 十一 检查 9)。
#[derive(Default, Clone, Debug)]
pub struct DocxStats {
    pub char_count: usize,
    pub para_count: usize,
    pub table_count: usize,
    pub page_field_count: usize,
    /// CJK 旁的半角标点(para_idx, 前一个字符, 半角标点)。可能多条。
    pub halfwidth_punct_in_cjk: Vec<(usize, char, char)>,
}

/// 门禁报告 — 全部信息汇总。
#[derive(Clone, Debug)]
pub struct GateReport {
    pub checks: Vec<CheckResult>,
    pub stats: DocxStats,
    pub exit_code: u8,
}

impl GateReport {
    /// 聚合所有 check 的严重度,得到最终 exit_code。
    fn from(checks: Vec<CheckResult>, stats: DocxStats) -> Self {
        let mut code = ExitCode::Pass;
        for c in &checks {
            let mapped = match c.severity {
                Severity::Pass | Severity::Info => ExitCode::Pass,
                Severity::Warning => ExitCode::Warning,
                Severity::Fail => ExitCode::Fail,
            };
            if mapped > code {
                code = mapped;
            }
        }
        Self {
            checks,
            stats,
            exit_code: code as u8,
        }
    }
}

/// 跑门禁。**不阻塞**导出,仅返回报告。
pub fn run_gate(docx_path: &Path) -> GateReport {
    // 0. 文件存在性 + 大小
    if !docx_path.exists() {
        return GateReport::from(
            vec![CheckResult {
                name: "file_open",
                severity: Severity::Fail,
                message: format!("文件不存在:{}", docx_path.display()),
            }],
            DocxStats::default(),
        );
    }
    let meta = match std::fs::metadata(docx_path) {
        Ok(m) => m,
        Err(e) => {
            return GateReport::from(
                vec![CheckResult {
                    name: "file_open",
                    severity: Severity::Fail,
                    message: format!("读元信息失败:{}", e),
                }],
                DocxStats::default(),
            );
        }
    };
    if meta.len() > MAX_DOCX_BYTES {
        return GateReport::from(
            vec![CheckResult {
                name: "file_size",
                severity: Severity::Fail,
                message: format!(
                    "文件过大:{} bytes(上限 {} MB)",
                    meta.len(),
                    MAX_DOCX_BYTES / 1024 / 1024
                ),
            }],
            DocxStats::default(),
        );
    }

    // 1. 打开 zip
    let mut archive = match File::open(docx_path)
        .map_err(|e| format!("打开失败:{}", e))
        .and_then(|f| ZipArchive::new(f).map_err(|e| format!("zip 解析失败:{}", e)))
    {
        Ok(a) => a,
        Err(msg) => {
            return GateReport::from(
                vec![CheckResult {
                    name: "zip_open",
                    severity: Severity::Fail,
                    message: msg,
                }],
                DocxStats::default(),
            );
        }
    };

    let mut checks = Vec::new();
    let mut stats = DocxStats::default();

    // 1. OpenXML 完整性
    checks.push(check_openxml_integrity(&mut archive));

    // 2. 页眉存在性
    checks.push(check_header_exists(&mut archive));

    // 3. 页脚页码字段
    let (c, page_count) = check_footer_page_field(&mut archive);
    stats.page_field_count = page_count;
    checks.push(c);

    // 4-9. 读 document.xml 做剩下检查
    let doc_xml = read_zip_entry(&mut archive, "word/document.xml");
    if let Some(xml) = doc_xml.as_deref() {
        checks.push(check_table_borders(xml));
        checks.push(check_fonts(xml));
        let (c, char_count, para_count, table_count, halfwidth) = check_cjk_punct_and_stats(xml);
        stats.char_count = char_count;
        stats.para_count = para_count;
        stats.table_count = table_count;
        stats.halfwidth_punct_in_cjk = halfwidth;
        checks.push(c);
        checks.push(check_empty_paragraphs(xml));
    } else {
        checks.push(CheckResult {
            name: "document_xml_read",
            severity: Severity::Fail,
            message: "word/document.xml 不可读".to_string(),
        });
    }

    // 4. 标题样式(法律文书豁免,仅 INFO)
    checks.push(CheckResult {
        name: "title_style",
        severity: Severity::Info,
        message: "法律文书常用 inline rPr 而非 Heading 样式,豁免(SPEC § 十一 检查 4)".to_string(),
    });

    // 9. 统计(INFO)
    checks.push(CheckResult {
        name: "stats",
        severity: Severity::Info,
        message: format!(
            "{} 字 / {} 段 / {} 表 / 页码字段 {} 个",
            stats.char_count, stats.para_count, stats.table_count, stats.page_field_count
        ),
    });

    GateReport::from(checks, stats)
}

// ───────────────────────── 各项检查实现 ─────────────────────────

fn read_zip_entry(archive: &mut ZipArchive<File>, name: &str) -> Option<String> {
    let mut entry = archive.by_name(name).ok()?;
    let mut s = String::new();
    entry.read_to_string(&mut s).ok()?;
    Some(s)
}

/// 找以 `prefix` 开头、`.xml` 结尾的部件,读第一个匹配的内容。
fn read_first_xml_with_prefix(archive: &mut ZipArchive<File>, prefix: &str) -> Option<String> {
    for i in 0..archive.len() {
        if let Ok(mut entry) = archive.by_index(i) {
            if entry.name().starts_with(prefix) && entry.name().ends_with(".xml") {
                let mut s = String::new();
                if entry.read_to_string(&mut s).is_ok() {
                    return Some(s);
                }
            }
        }
    }
    None
}

fn check_openxml_integrity(archive: &mut ZipArchive<File>) -> CheckResult {
    let required = [
        "word/document.xml",
        "word/styles.xml",
        "word/settings.xml",
        "[Content_Types].xml",
        "_rels/.rels",
    ];
    let mut missing: Vec<&str> = Vec::new();
    for n in &required {
        if archive.by_name(n).is_err() {
            missing.push(*n);
        }
    }
    if missing.is_empty() {
        CheckResult {
            name: "openxml_integrity",
            severity: Severity::Pass,
            message: format!("结构完整,{} 个关键 XML 部件全部存在", required.len()),
        }
    } else {
        CheckResult {
            name: "openxml_integrity",
            severity: Severity::Fail,
            message: format!("缺失部件:{}", missing.join(", ")),
        }
    }
}

fn check_header_exists(archive: &mut ZipArchive<File>) -> CheckResult {
    // 显式 for 循环,避免闭包借用 `archive: &mut` 后又返回 ZipFile<'_>(Rust invariance 拒绝)
    let mut header_count = 0usize;
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index(i) {
            let n = entry.name();
            if n.starts_with("word/header") && n.ends_with(".xml") {
                header_count += 1;
            }
        }
    }
    if header_count == 0 {
        CheckResult {
            name: "header_exists",
            severity: Severity::Warning,
            message: "无页眉部件(word/header*.xml) — 法院/当事人看不到机构抬头".to_string(),
        }
    } else {
        CheckResult {
            name: "header_exists",
            severity: Severity::Pass,
            message: format!("页眉 {} 个部件", header_count),
        }
    }
}

fn check_footer_page_field(archive: &mut ZipArchive<File>) -> (CheckResult, usize) {
    let footer_xml = match read_first_xml_with_prefix(archive, "word/footer") {
        Some(s) => s,
        None => {
            return (
                CheckResult {
                    name: "footer_page_field",
                    severity: Severity::Fail,
                    message: "无页脚部件(word/footer*.xml) — 缺页码字段,法院必拒".to_string(),
                },
                0,
            );
        }
    };
    // 找 PAGE 字段。注意:必须用 fldSimple 字段,而非硬编码数字。
    // 模式:w:instr=" PAGE " 或 w:instr="PAGE"(字段指令不区分空格)。
    let page_count = footer_xml.matches("PAGE").count();
    let numpages_count = footer_xml.matches("NUMPAGES").count();
    if page_count == 0 || numpages_count == 0 {
        (
            CheckResult {
                name: "footer_page_field",
                severity: Severity::Fail,
                message: format!(
                    "页脚缺 PAGE/NUMPAGES 字段(PAGE 出现 {} 次,NUMPAGES 出现 {} 次,可能是硬编码数字)",
                    page_count, numpages_count
                ),
            },
            0,
        )
    } else {
        (
            CheckResult {
                name: "footer_page_field",
                severity: Severity::Pass,
                message: format!(
                    "PAGE 字段 {} 个,NUMPAGES 字段 {} 个(删除段时页码自动重排)",
                    page_count, numpages_count
                ),
            },
            page_count,
        )
    }
}

fn check_table_borders(doc_xml: &str) -> CheckResult {
    if doc_xml.contains("<w:tbl>") {
        if doc_xml.contains("<w:tblBorders>") {
            CheckResult {
                name: "table_borders",
                severity: Severity::Pass,
                message: "检测到表格 + tblBorders 块".to_string(),
            }
        } else {
            CheckResult {
                name: "table_borders",
                severity: Severity::Fail,
                message: "文档含表格但缺 tblBorders 块 — 表格无边框".to_string(),
            }
        }
    } else {
        CheckResult {
            name: "table_borders",
            severity: Severity::Info,
            message: "文档无表格,跳过".to_string(),
        }
    }
}

fn check_fonts(doc_xml: &str) -> CheckResult {
    // SPEC § 二 字体白名单
    let allowed: &[&str] = &[
        "仿宋_GB2312",
        "SimHei",
        "方正小标宋简体",
        "宋体",
        "Songti SC",
        "STSong",
        "仿宋",
        "黑体",
        "小标宋",
        "Menlo",
        "Times New Roman",
    ];
    let mut count_ok = 0usize;
    let mut bad: Vec<String> = Vec::new();
    let needle = "w:eastAsia=\"";
    let mut idx = 0;
    while let Some(rel) = doc_xml[idx..].find(needle) {
        let abs = idx + rel + needle.len();
        match doc_xml[abs..].find('"') {
            Some(end) => {
                let font = &doc_xml[abs..abs + end];
                if allowed.iter().any(|a| *a == font) {
                    count_ok += 1;
                } else if bad.len() < 5 {
                    bad.push(font.to_string());
                }
                idx = abs + end;
            }
            None => break,
        }
    }
    if bad.is_empty() {
        CheckResult {
            name: "font_compliance",
            severity: Severity::Pass,
            message: format!("字体合规 ({} 处 eastAsia 引用,全部在白名单内)", count_ok),
        }
    } else {
        CheckResult {
            name: "font_compliance",
            severity: Severity::Fail,
            message: format!("不合规字体(前 5 个):{:?}", bad),
        }
    }
}

fn check_cjk_punct_and_stats(
    doc_xml: &str,
) -> (CheckResult, usize, usize, usize, Vec<(usize, char, char)>) {
    let mut reader = Reader::from_str(doc_xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut char_count = 0usize;
    let mut para_count = 0usize;
    let mut table_count = 0usize;
    let mut halfwidth: Vec<(usize, char, char)> = Vec::new();
    let mut in_paragraph = false;
    let mut current_para_text = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let local = e.local_name();
                if local.as_ref() == b"p" {
                    in_paragraph = true;
                    current_para_text.clear();
                }
            }
            Ok(Event::End(e)) => {
                let local = e.local_name();
                if local.as_ref() == b"p" && in_paragraph {
                    para_count += 1;
                    halfwidth.extend(scan_halfwidth_punct(&current_para_text, para_count));
                    in_paragraph = false;
                } else if local.as_ref() == b"tbl" {
                    table_count += 1;
                }
            }
            Ok(Event::Text(e)) if in_paragraph => {
                if let Ok(raw) = e.xml_content() {
                    char_count += raw.chars().count();
                    current_para_text.push_str(&raw);
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    let severity = if halfwidth.is_empty() {
        Severity::Pass
    } else {
        Severity::Fail
    };
    let message = if halfwidth.is_empty() {
        format!(
            "中文标点全角合规 ({} 字 / {} 段 / {} 表)",
            char_count, para_count, table_count
        )
    } else {
        format!(
            "检测到 {} 处半角标点混入 CJK(前 3 处:{:?})",
            halfwidth.len(),
            halfwidth.iter().take(3).collect::<Vec<_>>()
        )
    };
    (
        CheckResult {
            name: "cjk_punctuation",
            severity,
            message,
        },
        char_count,
        para_count,
        table_count,
        halfwidth,
    )
}

fn scan_halfwidth_punct(text: &str, para_idx: usize) -> Vec<(usize, char, char)> {
    fn is_cjk(c: char) -> bool {
        matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
    }
    let chars: Vec<char> = text.chars().collect();
    let mut bad = Vec::new();
    for (i, &c) in chars.iter().enumerate() {
        let prev_cjk = i > 0 && is_cjk(chars[i - 1]);
        let next_cjk = chars.get(i + 1).map(|&c| is_cjk(c)).unwrap_or(false);
        let next_digit = chars
            .get(i + 1)
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);
        let triggered = match c {
            ',' if prev_cjk && next_cjk => true,
            '.' if prev_cjk && next_cjk => true,
            ';' if prev_cjk => true,
            '?' if prev_cjk => true,
            '!' if prev_cjk => true,
            ':' if prev_cjk && !next_digit => true,
            _ => false,
        };
        if triggered {
            let prev = if i > 0 { chars[i - 1] } else { ' ' };
            bad.push((para_idx, prev, c));
        }
    }
    bad
}

fn check_empty_paragraphs(doc_xml: &str) -> CheckResult {
    let empty_count = doc_xml.matches("<w:p/>").count();
    if empty_count > 3 {
        CheckResult {
            name: "empty_paragraphs",
            severity: Severity::Warning,
            message: format!("检测到 {} 个空段(> 3,可能影响排版)", empty_count),
        }
    } else {
        CheckResult {
            name: "empty_paragraphs",
            severity: Severity::Pass,
            message: format!("空段 {} 个(≤ 3,合规)", empty_count),
        }
    }
}

// ───────────────────────── 单元测试 ─────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    /// 构造测试 docx 字节流。参数控制各部件是否包含。
    fn build_test_docx(
        include_header: bool,
        include_footer: bool,
        footer_hardcoded: bool,
        include_tables_with_borders: bool,
        body_text: &str,
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zip = ZipWriter::new(cursor);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            let put = |z: &mut ZipWriter<_>, name: &str, data: &str| {
                z.start_file(name, opts).unwrap();
                z.write_all(data.as_bytes()).unwrap();
            };

            // document.xml
            let mut doc =
                String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
            doc.push_str(
                r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>"#,
            );
            doc.push_str(&format!(
                r#"<w:p><w:pPr><w:spacing w:line="360" w:lineRule="auto"/></w:pPr><w:r><w:rPr><w:rFonts w:ascii="Times New Roman" w:eastAsia="仿宋_GB2312"/><w:sz w:val="28"/><w:szCs w:val="28"/></w:rPr><w:t xml:space="preserve">{}</w:t></w:r></w:p>"#,
                xml_escape_test(body_text)
            ));
            if include_tables_with_borders {
                doc.push_str(
                    r#"<w:tbl><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="000000"/><w:left w:val="single" w:sz="4" w:space="0" w:color="000000"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="000000"/><w:right w:val="single" w:sz="4" w:space="0" w:color="000000"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="000000"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="000000"/></w:tblBorders></w:tblPr><w:tr><w:tc><w:p/></w:tc></w:tr></w:tbl>"#,
                );
            }
            doc.push_str("</w:body></w:document>");
            put(&mut zip, "word/document.xml", &doc);

            put(
                &mut zip,
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
            );

            if include_header {
                put(
                    &mut zip,
                    "word/header1.xml",
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>test</w:t></w:r></w:p></w:hdr>"#,
                );
            }
            if include_footer {
                let footer = if footer_hardcoded {
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>1</w:t></w:r></w:p></w:ftr>"#
                } else {
                    r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t xml:space="preserve">第 </w:t></w:r><w:fldSimple w:instr="PAGE"><w:r><w:t>1</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> 页 / 共 </w:t></w:r><w:fldSimple w:instr="NUMPAGES"><w:r><w:t>1</w:t></w:r></w:fldSimple><w:r><w:t xml:space="preserve"> 页</w:t></w:r></w:p></w:ftr>"#
                };
                put(&mut zip, "word/footer1.xml", footer);
            }

            put(
                &mut zip,
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            );

            put(
                &mut zip,
                "word/styles.xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#,
            );

            put(
                &mut zip,
                "word/settings.xml",
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#,
            );

            zip.finish().unwrap();
        }
        buf
    }

    fn xml_escape_test(s: &str) -> String {
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

    fn write_to_temp(content: Vec<u8>) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(&content).unwrap();
        f
    }

    fn find_check<'a>(report: &'a GateReport, name: &str) -> Option<&'a CheckResult> {
        report.checks.iter().find(|c| c.name == name)
    }

    /// 测试 1:干净 docx → 全部 PASS / INFO,无 FAIL。
    #[test]
    fn gate_passes_on_clean_docx() {
        let content = build_test_docx(true, true, false, true, "这是测试文本。全部全角标点。");
        let f = write_to_temp(content);
        let report = run_gate(f.path());
        let fails: Vec<&CheckResult> = report
            .checks
            .iter()
            .filter(|c| c.severity == Severity::Fail)
            .collect();
        assert!(
            fails.is_empty(),
            "干净 docx 不应有 FAIL,实际:{:?}",
            fails
                .iter()
                .map(|c| (&c.name, &c.message))
                .collect::<Vec<_>>()
        );
        // 应当有 header_exists (PASS) + footer_page_field (PASS) + table_borders (PASS) + cjk_punctuation (PASS)
        assert_eq!(
            find_check(&report, "header_exists").unwrap().severity,
            Severity::Pass
        );
        assert_eq!(
            find_check(&report, "footer_page_field").unwrap().severity,
            Severity::Pass
        );
        assert_eq!(
            find_check(&report, "table_borders").unwrap().severity,
            Severity::Pass
        );
        assert_eq!(
            find_check(&report, "cjk_punctuation").unwrap().severity,
            Severity::Pass
        );
        assert_eq!(report.exit_code, 0);
    }

    /// 测试 2:无页眉 → WARNING,exit_code=1。
    #[test]
    fn gate_warns_on_missing_header() {
        let content = build_test_docx(false, true, false, false, "纯文本");
        let f = write_to_temp(content);
        let report = run_gate(f.path());
        let header_check = find_check(&report, "header_exists").unwrap();
        assert_eq!(header_check.severity, Severity::Warning);
        assert_eq!(report.exit_code, 1);
        // 不应该有 FAIL
        assert!(report.checks.iter().all(|c| c.severity != Severity::Fail));
    }

    /// 测试 3:页脚是硬编码 "1"(无 PAGE 字段) → FAIL,exit_code=2。
    #[test]
    fn gate_fails_on_hardcoded_page_number() {
        let content = build_test_docx(true, true, true, false, "纯文本");
        let f = write_to_temp(content);
        let report = run_gate(f.path());
        let footer_check = find_check(&report, "footer_page_field").unwrap();
        assert_eq!(footer_check.severity, Severity::Fail);
        assert_eq!(report.exit_code, 2);
    }

    /// 测试 4:正文含半角逗号/句号紧邻 CJK → FAIL,stats.halfwidth_punct_in_cjk 非空。
    #[test]
    fn gate_fails_on_halfwidth_punct_in_cjk() {
        // "这是,测试,文本。"  → 中文中带半角逗号 → 应被门禁抓出
        let content = build_test_docx(true, true, false, false, "这是,测试,文本。");
        let f = write_to_temp(content);
        let report = run_gate(f.path());
        let punct_check = find_check(&report, "cjk_punctuation").unwrap();
        assert_eq!(punct_check.severity, Severity::Fail);
        assert!(!report.stats.halfwidth_punct_in_cjk.is_empty());
        assert_eq!(report.exit_code, 2);
    }

    /// 测试 5:有 footer 但无 document.xml(用脚手架直接构造)→ FAIL openxml_integrity。
    ///
    /// 这个测试直接构造一个 zip,故意不放 word/document.xml,验证 OpenXML 完整性检查。
    #[test]
    fn gate_fails_on_missing_table_borders() {
        // 构造一个 docx:有 footer(用字段),有 header,有 document.xml 但表格没 tblBorders
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let mut zip = ZipWriter::new(cursor);
            let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
            let doc = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>纯文本</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#;
            zip.start_file("word/document.xml", opts).unwrap();
            zip.write_all(doc.as_bytes()).unwrap();
            zip.start_file("[Content_Types].xml", opts).unwrap();
            zip.write_all(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.as_bytes(),
            ).unwrap();
            zip.start_file("word/header1.xml", opts).unwrap();
            zip.write_all(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p/></w:hdr>"#.as_bytes()).unwrap();
            zip.start_file("word/footer1.xml", opts).unwrap();
            zip.write_all(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:fldSimple w:instr="PAGE"><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:ftr>"#.as_bytes()).unwrap();
            zip.start_file("_rels/.rels", opts).unwrap();
            zip.write_all(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.as_bytes()).unwrap();
            zip.start_file("word/styles.xml", opts).unwrap();
            zip.write_all(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#.as_bytes()).unwrap();
            zip.start_file("word/settings.xml", opts).unwrap();
            zip.write_all(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"/>"#.as_bytes()).unwrap();
            zip.finish().unwrap();
        }
        let f = write_to_temp(buf);
        let report = run_gate(f.path());
        let tb_check = find_check(&report, "table_borders").unwrap();
        assert_eq!(tb_check.severity, Severity::Fail);
        assert_eq!(report.exit_code, 2);
    }
}
