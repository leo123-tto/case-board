//! 单个文档的字段抽取。
//!
//! 流程:
//!   1. 根据文件扩展名分派文本抽取方式(docx 原生 / 直接读 / pdf-inspector / office→MinerU)
//!   2. PDF 文本 < 200 字时自动 fallback 到 OCR 后端(按 OcrContext.cloud_enabled 分流)
//!   3. 图片 + office 文档(doc/rtf/odt/ppt/xls)直接走 OCR / MinerU 云端解析
//!   4. 把抽出的纯文本喂给本机 LLM 抽 7 个字段
//!   5. 返回 ExtractedFields(或 Skip / Error)
//!
//! 这一层是纯函数,不碰数据库;数据库写入由 pipeline.rs 编排。
//!
//! 2026-05-23 加:PDF 支持(R&D 阶段已在 5 个真实案件上验证)+ OCR 兜底链路 + 云本机分流。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

use crate::db::metrics::MetricEntry;
use crate::docx_extract;
use crate::ingest::ocr::{self, OcrContext};
use crate::llm::{self, ExtractedFields};

/// PDF 文本抽取后字数低于这个阈值,认为是扫描件,转 OCR 兜底
const PDF_TEXT_MIN_CHARS: usize = 200;

/// 抽出的文字里 CJK(中日韩)字符占比下限。低于这个值认为是乱码 / CID 没解出,
/// 走下一档兜底(pdftotext / OCR)。2026-05-26 V0.1.11 加,
/// 防止 pdf-extract 在某些 CID 编码 PDF 上返回拉丁字符乱码却看似"抽到东西"。
const PDF_CJK_MIN_RATIO: f64 = 0.30;

/// 2026-05-26 V0.1.11 加 · 律师文档几乎全中文,文字型 PDF 必有大量 CJK。
/// CID 编码不识别时 pdf-extract 会返回大量 `?` / 拉丁字母,字符数像够了但实际乱码。
fn cjk_ratio(s: &str) -> f64 {
    let total = s.chars().filter(|c| !c.is_whitespace()).count();
    if total == 0 {
        return 0.0;
    }
    let cjk = s
        .chars()
        .filter(|c| {
            let n = *c as u32;
            (0x4E00..=0x9FFF).contains(&n)        // CJK 基本区
                || (0x3400..=0x4DBF).contains(&n) // 扩展 A
                || (0xF900..=0xFAFF).contains(&n) // 兼容汉字
        })
        .count();
    cjk as f64 / total as f64
}

/// 文本是否"够用"(字数 + CJK 比例都达标),决定是否要走下一档兜底
fn pdf_text_usable(text: &str) -> bool {
    let chars = text.trim().chars().count();
    chars >= PDF_TEXT_MIN_CHARS && cjk_ratio(text) >= PDF_CJK_MIN_RATIO
}

/// 2026-05-25 V0.1.10 加:朋友的 Mac 没装 poppler → pdftotext 命令缺失 → 之前直接报错
/// 改成"没装就直接走 OCR"兜底(每个 PDF 都走 mineru 多花点积分,但能用)
fn pdftotext_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let mut cmd = std::process::Command::new("pdftotext");
        cmd.arg("-v");
        crate::proc_util::hide_console_window_std(&mut cmd);
        cmd.output()
            .map(|o| o.status.success() || o.status.code() == Some(99)) // pdftotext -v 退出码 99 也算正常
            .unwrap_or(false)
    })
}

/// 纯 Rust 抽 PDF 文字 + 智能分类(2026-05-26 V0.1.11 用 pdf-inspector)。
///
/// pdf-inspector 比 pdf-extract 的关键优势:
/// 1. 直接给出 PdfType { TextBased / Scanned / Mixed / ImageBased } 分类,不用靠"字数<200"启发式
/// 2. 提供 has_encoding_issues 检测 CID 字体 / Identity-H 乱码情况
/// 3. 5 个真实法律 PDF 实测:扫描件识别 100% 准,速度普遍 2-20x 快
/// 4. 输出已是 Markdown(表格被保留为 markdown 表格,LLM 抽取更准)
///
/// 返回:
/// - Ok(text) 表示拿到可用文字
/// - Err("__NEEDS_OCR__") 表示明确的扫描件 / 编码异常 / 文字太少,该走 OCR
fn extract_pdf_with_inspector(path: &Path) -> Result<String, String> {
    use pdf_inspector::{process_pdf, PdfType};

    let result = process_pdf(path).map_err(|e| format!("pdf-inspector: {}", e))?;

    // 1. 编码乱(CID 字体 / Identity-H 没解出) → 走 OCR
    if result.has_encoding_issues {
        return Err("__NEEDS_OCR__".into());
    }

    // 2. 明确扫描件 / 图片型 → 走 OCR
    if matches!(result.pdf_type, PdfType::Scanned | PdfType::ImageBased) {
        return Err("__NEEDS_OCR__".into());
    }

    // 3. **Mixed PDF 半抽风险**:inspector 对 Mixed 只返回文字层页面的 markdown,
    //    扫描页直接漏。这里 pages_needing_ocr 非空就把整份转 OCR,
    //    避免 LLM 拿到半截内容却以为是完整文档(按页 OCR 是 V0.2 路线图)。
    //    2026-05-26 V0.1.11 加,根据 advisor 提示防"半抽"
    if matches!(result.pdf_type, PdfType::Mixed) && !result.pages_needing_ocr.is_empty() {
        crate::dlog!(
            "[pdf] Mixed PDF 共 {} 页,其中 {} 页是扫描页,整份转 OCR 兜底",
            result.page_count,
            result.pages_needing_ocr.len()
        );
        return Err("__NEEDS_OCR__".into());
    }

    // 4. 拿 markdown(TextBased / 纯文字 Mixed)
    let text = result.markdown.unwrap_or_default();

    // 5. 文本质量门槛
    if pdf_text_usable(&text) {
        Ok(text)
    } else {
        Err("__NEEDS_OCR__".into())
    }
}

/// 单文档抽取结果。2026-05-23 晚十 加 text_md(用于写盘到 extracts/)
///
/// 2026-05-26 V0.1.12:加 metrics,每个 stage(文本/OCR/LLM)的 backend + 耗时 + outcome,
/// 由 pipeline.rs 批量 insert 进 extraction_metrics 表,反馈通道带出来做本地 vs 云端对比。
#[derive(Debug)]
#[allow(clippy::large_enum_variant)] // text_md 可能上万字,enum 大点能接受
pub enum ExtractResult {
    /// 成功抽出字段(text_md = 抽出的纯文本,caller 写盘)
    Extracted {
        fields: ExtractedFields,
        text_md: String,
        metrics: Vec<MetricEntry>,
    },
    /// 已知不支持的格式,跳过(.pdf / 图片等),不报错
    Skipped {
        reason: String,
        metrics: Vec<MetricEntry>,
    },
    /// 只抽了文本、**没**跑 LLM 字段抽取(省 LLM 成本,但保证文档对 chat 工具 /
    /// 全文搜索可读)。2026-05-31:修「证据类核心合同被完全跳过、AI 读不到」的 bug。
    TextOnly {
        text_md: String,
        metrics: Vec<MetricEntry>,
    },
    /// 出错(textutil 失败 / LLM 不可达 / JSON 解析失败等)
    Failed {
        error: String,
        metrics: Vec<MetricEntry>,
    },
}

/// 只抽文本、不跑 LLM 字段 —— 给「低价值/证据类」被 skip 的文档用,让它们仍可被
/// `read_case_doc` / `find_in_document` / 全文搜索读到。
///
/// **成本红线**:复用已有 `extract_text`(txt/md 直读、docx 原生解析、PDF 走
/// pdf-inspector),**绝不触发云端 OCR**。扫描件 PDF / 图片 / office 文档(doc/ppt/xls,
/// `extract_text` 返回 `__NEEDS_OCR__`)或文本太少时返回 `Ok(None)`,调用方保持「跳过(无文本)」——
/// 不为省下来的字段抽取偷偷烧 MinerU 积分(归档类 office 文档因此只在「完整抽」档才走 MinerU)。
///
/// 返回:
///   - `Ok(Some(text))` —— 便宜直抽成功,文本可用
///   - `Ok(None)` —— 需昂贵 OCR 才能读(扫描件/图片)或文本太少,按成本红线放弃
///   - `Err(_)` —— 真错(文件读不了等)
pub async fn extract_text_only_cheap(
    path: &Path,
    filename: &str,
) -> Result<Option<String>, String> {
    let kind = text_extraction_kind(filename);
    // 图片 / 不支持格式:直抽拿不到 → 不 OCR,放弃
    if matches!(kind, TextKind::Image | TextKind::Unsupported) {
        return Ok(None);
    }
    match extract_text(path, kind) {
        Ok((t, _backend)) if t.trim().chars().count() >= 10 => Ok(Some(t)),
        Ok(_) => Ok(None),                          // 文本太少
        Err(e) if e == "__NEEDS_OCR__" => Ok(None), // 扫描件:成本红线,不 OCR
        Err(e) => Err(e),
    }
}

/// 根据文件名后缀决定怎么抽文本。
fn text_extraction_kind(filename: &str) -> TextKind {
    let f = filename.to_lowercase();
    if f.ends_with(".md")
        || f.ends_with(".markdown")
        || f.ends_with(".txt")
        || f.ends_with(".html")
        || f.ends_with(".htm")
    {
        TextKind::ReadDirect
    } else if f.ends_with(".docx") {
        // 2026-06-15 V0.3.18 fix:.docx 走原生 OOXML 解析(跨平台,替代 macOS textutil)
        TextKind::Docx
    } else if is_office_textutil_ext(&f) {
        // 2026-06-16:.doc / .rtf / .odt → macOS textutil 优先(免费即时),失败/非 mac → MinerU 兜底
        TextKind::OfficeTextutil
    } else if is_office_mineru_only_ext(&f) {
        // 2026-06-16:.ppt(x) / .xls(x) → textutil 不支持,统一走 MinerU 云端解析(Paddle 也不支持)
        TextKind::OfficeCloud
    } else if f.ends_with(".pdf") {
        TextKind::Pdf
    } else if is_ocr_image_ext(&f) {
        TextKind::Image
    } else {
        TextKind::Unsupported
    }
}

/// 可走 OCR 的图片扩展名(传入**已小写**的文件名)。云端 MinerU 全支持这 8 种;
/// 本机 vision 仅 png/jpg/jpeg/tiff/bmp,其余在本机模式会透传「不支持扩展名」错误
/// (非静默跳过)。与 `pipeline::might_hit_mineru` 共用此集合,防三处漂移
/// (2026-06-03 底座审计 B5:原来这里只认 png/jpg/jpeg,tiff/bmp/webp/gif/jp2 扫描件
/// 被静默标 Skipped、永不 OCR)。
pub(crate) fn is_ocr_image_ext(lower_filename: &str) -> bool {
    [
        ".png", ".jpg", ".jpeg", ".tiff", ".bmp", ".webp", ".gif", ".jp2",
    ]
    .iter()
    .any(|ext| lower_filename.ends_with(ext))
}

/// `.doc` / `.rtf` / `.odt`(传入**已小写**文件名)—— macOS 系统 textutil 能免费即时抽,
/// 失败或非 macOS 平台则交 MinerU 云端兜底。
fn is_office_textutil_ext(lower_filename: &str) -> bool {
    [".doc", ".rtf", ".odt"]
        .iter()
        .any(|ext| lower_filename.ends_with(ext))
}

/// `.ppt(x)` / `.xls(x)`(传入**已小写**文件名)—— textutil 不支持,所有平台只能走 MinerU 云端解析。
fn is_office_mineru_only_ext(lower_filename: &str) -> bool {
    [".ppt", ".pptx", ".xls", ".xlsx"]
        .iter()
        .any(|ext| lower_filename.ends_with(ext))
}

/// 所有"本地无跨平台纯 Rust 方案、最终可能落到 MinerU"的 office 格式(并集,传入**已小写**文件名)。
/// **不含 `.docx`**(本地原生 `docx_extract` 优先,免费快)。与 `ocr::extract_with_ocr`(office 强制
/// MinerU)、`pipeline::might_hit_mineru`(节流闸门)、`extract_one`(失败 metric 标签)共用,防四处漂移。
/// 2026-06-16(作者拍板):doc/rtf/odt 在 macOS 走 textutil 免费快路径;ppt/xls + 非 mac 的 doc 组走 MinerU。
pub(crate) fn is_office_cloud_ext(lower_filename: &str) -> bool {
    is_office_textutil_ext(lower_filename) || is_office_mineru_only_ext(lower_filename)
}

enum TextKind {
    /// 纯文本,直接 fs::read_to_string
    ReadDirect,
    /// 2026-06-15 V0.3.18 fix:.docx 走 docx_extract 原生 OOXML 解析(跨平台,替代 macOS textutil)。
    /// 2026-06-16:本地解析失败时兜底 MinerU 云端(MinerU 也支持 docx)。
    Docx,
    /// 2026-06-16:.doc / .rtf / .odt —— macOS 用系统 textutil 免费即时抽;失败或非 macOS 平台
    /// 落 MinerU 云端兜底(整份上传,Windows 也能用)。
    OfficeTextutil,
    /// 2026-06-16:.ppt(x) / .xls(x) —— textutil 不支持,所有平台统一走 MinerU 云端文档解析。
    /// Paddle 不支持 → ocr.rs 对 office 强制 MinerU。
    OfficeCloud,
    /// PDF,走 pdf-inspector;扫描件 / 抽取失败 fallback OCR
    Pdf,
    /// 图片,直接走 OCR 后端
    Image,
    /// 其他格式
    Unsupported,
}

/// 抽一个文档的纯文本。
/// 2026-05-26 V0.1.12:返回 (text, backend_used) — backend 写进 metrics 表用,
/// 让朋友实测后能区分"是 pdf-inspector 直抽快还是 pdftotext 兜底快"。
fn extract_text(path: &Path, kind: TextKind) -> Result<(String, &'static str), String> {
    match kind {
        TextKind::ReadDirect => std::fs::read_to_string(path)
            .map(|t| (t, "read_direct"))
            .map_err(|e| format!("读文件失败: {}", e)),
        TextKind::Docx => {
            // 2026-06-15 V0.3.18 fix:跨平台 .docx 抽取(zip + XML),替代 macOS textutil。
            // Windows / Linux 上 textutil 不存在,会 "program not found"。
            let path_str = path
                .to_str()
                .ok_or_else(|| format!("路径含非 UTF-8 字符: {:?}", path))?;
            match docx_extract::extract_docx_text(path_str) {
                Ok((text, true)) => Ok((text, "docx-native")),
                // 2026-06-16:本地原生解析失败(文件损坏 / 异常 OOXML)→ 兜底 MinerU 云端(也支持 docx)
                Ok((_, false)) | Err(_) => Err("__NEEDS_OCR__".into()),
            }
        }
        TextKind::OfficeTextutil => {
            // 2026-06-16(作者拍板):.doc / .rtf / .odt —— macOS 用系统 textutil 免费即时抽
            //(作者主用 mac,省积分 + 不联网);textutil 失败(罕见)或非 macOS 平台 → 落 MinerU 云端兜底。
            #[cfg(target_os = "macos")]
            {
                match textutil_extract(path) {
                    Ok(t) if !t.trim().is_empty() => Ok((t, "textutil")),
                    _ => Err("__NEEDS_OCR__".into()),
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                Err("__NEEDS_OCR__".into())
            }
        }
        TextKind::OfficeCloud => {
            // 2026-06-16:.ppt(x) / .xls(x) 无跨平台纯 Rust 方案,交给 OCR 链路 → MinerU 云端文档解析
            //(extract_one 接力;ocr.rs 对 office 强制走 MinerU,只配 Paddle 没配 MinerU 时给明确引导报错)。
            Err("__NEEDS_OCR__".into())
        }
        TextKind::Pdf => {
            // 2026-05-26 V0.1.11 PDF 抽取链路:
            //   1. pdf-inspector 主链路(分类 + Markdown 一站式)
            //      - TextBased + 无编码问题 + 文本达标 → 用 markdown
            //      - Scanned / ImageBased / has_encoding_issues / 文本不达标 → __NEEDS_OCR__
            //   2. (兼容)pdftotext 仅在 pdf-inspector 自身 panic/error 时兜底
            //      之前的链路把 pdftotext 当二档抽,实测在 pdf-inspector 加持下不需要;
            //      但 pdf-inspector 是 git dep 新项目,保留 pdftotext 作 inspector 自身崩溃的最后防线
            match extract_pdf_with_inspector(path) {
                Ok(t) => Ok((t, "pdf-inspector")),
                Err(e) if e == "__NEEDS_OCR__" => Err("__NEEDS_OCR__".into()),
                Err(_inspector_err) => {
                    if pdftotext_available() {
                        match extract_pdf_with_pdftotext(path) {
                            Ok(t) if pdf_text_usable(&t) => Ok((t, "pdftotext")),
                            _ => Err("__NEEDS_OCR__".into()),
                        }
                    } else {
                        Err("__NEEDS_OCR__".into())
                    }
                }
            }
        }
        TextKind::Image => Err("__NEEDS_OCR__".into()), // 由 extract_one 接力 OCR 后端
        TextKind::Unsupported => Err("不支持的格式".into()),
    }
}

/// macOS 系统 textutil 抽 .doc/.rtf/.odt 纯文本(免费、即时、离线)。
/// 仅 macOS 编译;非 macOS 平台这些格式直接走 MinerU(见 `extract_text` 的 OfficeTextutil 分支)。
#[cfg(target_os = "macos")]
fn textutil_extract(path: &Path) -> Result<String, String> {
    let output = std::process::Command::new("textutil")
        .arg("-convert")
        .arg("txt")
        .arg("-stdout")
        .arg(path)
        .output()
        .map_err(|e| format!("调 textutil 失败: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "textutil 转换失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("textutil 输出不是 UTF-8: {}", e))
}

/// 用 pdftotext (poppler) 抽 PDF 纯文本。
///
/// 注意:`pdftotext` 必须在 PATH 里。打包 DMG 时要把 poppler 静态链或者提示用户 `brew install poppler`。
fn extract_pdf_with_pdftotext(path: &Path) -> Result<String, String> {
    let mut cmd = std::process::Command::new("pdftotext");
    cmd.arg("-layout")
        .arg("-enc")
        .arg("UTF-8")
        .arg(path)
        .arg("-"); // stdout
    crate::proc_util::hide_console_window_std(&mut cmd);
    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "pdftotext 未安装(brew install poppler)".into()
        } else {
            format!("调 pdftotext 失败: {}", e)
        }
    })?;
    if !output.status.success() {
        return Err(format!(
            "pdftotext 转换失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("pdftotext 输出不是 UTF-8: {}", e))
}

/// 对一个文档跑完整的抽取流程:文本 → LLM 字段。
///
/// 不抛 panic,所有错误转成 ExtractResult,让调用方决定怎么记录。
///
/// 2026-05-26 V0.1.12:每个 stage 埋时间 + backend 写进 metrics,pipeline 负责落库。
/// 设计意图:朋友实测后,作者拿反馈 MD 对比本地 vs 云端 OCR 哪个更快更准。
pub async fn extract_one(
    llm_config: &llm::LlmConfig,
    ocr_ctx: &OcrContext,
    path: &Path,
    filename: &str,
    category: Option<&str>,
) -> ExtractResult {
    let kind = text_extraction_kind(filename);

    // 文件元数据(给 metrics 用)
    let file_size_bytes = std::fs::metadata(path).map(|m| m.len() as i64).unwrap_or(0);
    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mut metrics: Vec<MetricEntry> = Vec::new();

    // 不支持的格式直接 Skip,不算错(也不记 metric — 没真正跑任何后端)
    if matches!(kind, TextKind::Unsupported) {
        return ExtractResult::Skipped {
            reason: "不支持的格式".into(),
            metrics,
        };
    }

    // 1. 文本抽取(textutil / read_direct / pdf-inspector→pdftotext / 图片→OCR 标记)
    let t0 = Instant::now();
    // 2026-06-13:去水印重识别(force_backend=ppocrv6)时强制走 OCR —— 用户明确要 OCR 去水印,
    // 不要因为带水印 PDF 恰好有可抽文本层就跳过 OCR(那层往往也被水印污染)。
    let text_extract_result = if ocr_ctx.force_backend.is_some() {
        Err("__NEEDS_OCR__".to_string())
    } else {
        extract_text(path, kind)
    };
    let text = match text_extract_result {
        Ok((t, backend)) => {
            let chars = t.chars().count() as i64;
            metrics.push(MetricEntry {
                filename: filename.into(),
                ext: ext.clone(),
                file_size_bytes,
                stage: "text_extract".into(),
                backend: backend.into(),
                outcome: "ok".into(),
                elapsed_ms: t0.elapsed().as_millis() as i64,
                text_chars: Some(chars),
                error_short: None,
            });
            t
        }
        Err(e) if e == "__NEEDS_OCR__" => {
            // text_extract 阶段没抽出来 → 走 OCR(也记一条 OCR metric)
            // 失败时的 backend 标签:云端写主力名(实际可能主备都试过,error_short 里有全程)
            let ocr_backend = if ocr_ctx.force_backend.as_deref() == Some("ppocrv6") {
                "ppocrv6"
            } else if ocr_ctx.cloud_enabled {
                // 2026-06-16:office 文档强制走 MinerU(Paddle 不支持,见 ocr.rs),失败标签也写 MinerU
                if is_office_cloud_ext(&filename.to_lowercase()) {
                    "mineru-precision"
                } else if ocr_ctx.cloud_primary == "paddle-vl" {
                    "paddle-vl"
                } else {
                    "mineru-precision"
                }
            } else {
                "local-vision"
            };
            let t_ocr = Instant::now();
            match ocr_fallback(path.to_path_buf(), ocr_ctx.clone()).await {
                Ok((t, used_backend)) => {
                    let chars = t.chars().count() as i64;
                    metrics.push(MetricEntry {
                        filename: filename.into(),
                        ext: ext.clone(),
                        file_size_bytes,
                        stage: "ocr".into(),
                        // 成功时记**实际用到**的后端(主力失败切备用时是备用那家)
                        backend: used_backend.into(),
                        outcome: "ok".into(),
                        elapsed_ms: t_ocr.elapsed().as_millis() as i64,
                        text_chars: Some(chars),
                        error_short: None,
                    });
                    t
                }
                Err(e) => {
                    let short = crate::feedback::sanitize_paths(&e)
                        .chars()
                        .take(200)
                        .collect::<String>();
                    metrics.push(MetricEntry {
                        filename: filename.into(),
                        ext,
                        file_size_bytes,
                        stage: "ocr".into(),
                        backend: ocr_backend.into(),
                        outcome: "failed".into(),
                        elapsed_ms: t_ocr.elapsed().as_millis() as i64,
                        text_chars: None,
                        error_short: Some(short),
                    });
                    return ExtractResult::Failed {
                        error: format!("OCR 兜底失败:{}", e),
                        metrics,
                    };
                }
            }
        }
        Err(e) => {
            let short = crate::feedback::sanitize_paths(&e)
                .chars()
                .take(200)
                .collect::<String>();
            metrics.push(MetricEntry {
                filename: filename.into(),
                ext,
                file_size_bytes,
                stage: "text_extract".into(),
                backend: "unknown".into(),
                outcome: "failed".into(),
                elapsed_ms: t0.elapsed().as_millis() as i64,
                text_chars: None,
                error_short: Some(short),
            });
            return ExtractResult::Failed { error: e, metrics };
        }
    };

    // 2. 文本太短 → 跳过(空文档/OCR 没抽出来)
    if text.trim().chars().count() < 30 {
        return ExtractResult::Skipped {
            reason: format!("文本太短({} 字符)", text.trim().chars().count()),
            metrics,
        };
    }

    // 3. 文本太长 → 截断(给 LLM 的)
    const MAX_CHARS: usize = 10000;
    let text_for_llm: String = if text.chars().count() > MAX_CHARS {
        text.chars().take(MAX_CHARS).collect::<String>()
    } else {
        text.clone()
    };

    // 4. LLM 抽取
    let llm_backend = llm_backend_label(llm_config);
    let t_llm = Instant::now();
    match llm::extract_case_fields_with_hint(llm_config, &text_for_llm, Some(filename), category)
        .await
    {
        Ok(fields) => {
            metrics.push(MetricEntry {
                filename: filename.into(),
                ext,
                file_size_bytes,
                stage: "llm_extract".into(),
                backend: llm_backend.clone(),
                outcome: "ok".into(),
                elapsed_ms: t_llm.elapsed().as_millis() as i64,
                text_chars: Some(text_for_llm.chars().count() as i64),
                error_short: None,
            });
            ExtractResult::Extracted {
                fields,
                text_md: text, // pipeline 用这个写盘到 extracts/<case_id>/<doc_id>.md
                metrics,
            }
        }
        Err(e) => {
            let short = crate::feedback::sanitize_paths(&format!("{}", e))
                .chars()
                .take(200)
                .collect::<String>();
            metrics.push(MetricEntry {
                filename: filename.into(),
                ext,
                file_size_bytes,
                stage: "llm_extract".into(),
                backend: llm_backend.clone(),
                outcome: "failed".into(),
                elapsed_ms: t_llm.elapsed().as_millis() as i64,
                text_chars: None,
                error_short: Some(short),
            });
            ExtractResult::Failed {
                error: format!("LLM 抽取失败: {}", e),
                metrics,
            }
        }
    }
}

/// LLM 后端标签 = endpoint 类型 + 模型名,这样 metric 既能区分 local/cloud,
/// 又能验证"模型切换是否实时生效"(Flash vs Pro 在 metric 里直接分桶 A/B)。
/// 例:"deepseek-v4-flash" / "deepseek-v4-pro" / "local-llm:MiniCPM-V-4_6-Q8_0"
fn llm_backend_label(cfg: &llm::LlmConfig) -> String {
    if cfg.endpoint.contains("127.0.0.1") || cfg.endpoint.contains("localhost") {
        format!("local-llm:{}", cfg.model)
    } else {
        // 云端:直接用模型名,DeepSeek 的 deepseek-v4-flash / deepseek-v4-pro 等
        cfg.model.clone()
    }
}

/// 用 OCR 后端兜底抽文本,失败返回真错。
///
/// 2026-05-25 V0.1.10 改:`extract_with_ocr` 变成 async(MinerU 切到 HTTP 客户端),
/// 直接 await 即可。本机 vision sync 调用由 ocr.rs 内部 spawn_blocking 包好。
/// 2026-06-12 改:返回 `(text, backend)` —— 云端有主/备自动切换后,实际用的
/// 后端可能不是主力,metric 必须记真实那家。
pub async fn extract_text_for_element_conversion(
    path: &Path,
    filename: &str,
    ocr_ctx: &OcrContext,
) -> Result<String, String> {
    let kind = text_extraction_kind(filename);
    if matches!(kind, TextKind::Unsupported) {
        return Err("仅支持 .docx、.doc 和 .pdf 格式".into());
    }

    let text = match extract_text(path, kind) {
        Ok((text, _)) => text,
        Err(error) if error == "__NEEDS_OCR__" => {
            ocr_fallback(path.to_path_buf(), ocr_ctx.clone()).await?.0
        }
        Err(error) => return Err(error),
    };
    if text.trim().chars().count() < 30 {
        return Err("文书可识别文字太少，无法进行要素化".into());
    }
    Ok(text)
}

/// 用 OCR 后端兜底抽文本,失败返回真错。
///
/// 2026-05-25 V0.1.10 改:`extract_with_ocr` 变成 async(MinerU 切到 HTTP 客户端),
/// 直接 await 即可。本机 vision sync 调用由 ocr.rs 内部 spawn_blocking 包好。
/// 2026-06-12 改:返回 `(text, backend)` —— 云端有主/备自动切换后,实际用的
/// 后端可能不是主力,metric 必须记真实那家。
async fn ocr_fallback(path: PathBuf, ctx: OcrContext) -> Result<(String, &'static str), String> {
    match ocr::extract_with_ocr(&path, &ctx).await {
        ocr::OcrResult::Ok { text, backend, .. } => Ok((text, backend)),
        ocr::OcrResult::Failed { error, attempted } => {
            Err(format!("{}(尝试后端:{})", error, attempted.join(", ")))
        }
        ocr::OcrResult::Skipped { reason } => Err(format!("OCR 跳过:{}", reason)),
    }
}
