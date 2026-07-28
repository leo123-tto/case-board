//! 云端 OCR 输出规范化(2026-07-27 真机反馈:抽取文本混入 LaTeX 记号 / img 占位 / 注释噪音)。
//!
//! 只在 OCR 结果出口(`extract_with_ocr` 的 `OcrResult::Ok`)调用一次,新识别的材料落盘前
//! 就是干净文本;已落盘的旧文件不回写(展示层另有同规则净化,见前端 `derivedText.ts`)。
//!
//! 处置原则(作者 2026-07-27 拍板:有用保留并渲染好,没用清掉别占大模型上下文):
//! - HTML 表格 `<table>` = 表格数据本体 → **原样保留**(前端渲染成真表格,模型读结构化数据);
//! - LaTeX 数学记号(`\pm 0.04`、`5.0 \times 1500`、`$ \underline{\text{甲}} $`)→ 翻译成
//!   正常符号(± 0.04、5.0 × 1500、甲)。OCR 识别的数据没错,只是表示法要还原;
//! - `<img src="imgs/...">` / `![Image](imgs/...)` 抠图占位 → 压成 `[图]`;路径带 seal
//!   (印章框)压成 `[图:印章]` —— "此处有章"对律师是有效信号,长路径本身是纯上下文浪费;
//! - `<!-- image-->` 等注释、`<div>/<span>` 排版壳、`<br>` → 清壳留内容,不做更激进的
//!   通用 HTML 清洗(对齐 `ai_workspace::material_processor` 的保守边界,避免误伤正文)。

use std::sync::OnceLock;

use regex::Regex;

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid ocr sanitize regex"))
}

static IMG_TAG: OnceLock<Regex> = OnceLock::new();
static IMG_SRC: OnceLock<Regex> = OnceLock::new();
static MD_IMAGE: OnceLock<Regex> = OnceLock::new();
static COMMENT: OnceLock<Regex> = OnceLock::new();
static BR_TAG: OnceLock<Regex> = OnceLock::new();
static DIV_SPAN_TAG: OnceLock<Regex> = OnceLock::new();
static DOLLAR_FRAGMENT: OnceLock<Regex> = OnceLock::new();
static LATEX_WRAPPER: OnceLock<Regex> = OnceLock::new();
static LATEX_FRAC: OnceLock<Regex> = OnceLock::new();
static LATEX_CIRC: OnceLock<Regex> = OnceLock::new();
static LATEX_SUPERSCRIPT: OnceLock<Regex> = OnceLock::new();
static LATEX_ENV: OnceLock<Regex> = OnceLock::new();
static LATEX_SYMBOLS: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();

fn latex_symbols() -> &'static [(Regex, &'static str)] {
    LATEX_SYMBOLS.get_or_init(|| {
        [
            (r"\\times\b", "×"),
            (r"\\pm\b", "±"),
            (r"\\mp\b", "∓"),
            (r"\\leq?\b", "≤"),
            (r"\\geq?\b", "≥"),
            (r"\\approx\b", "≈"),
            (r"\\sim\b", "~"),
            (r"\\cdot\b", "·"),
            (r"\\div\b", "÷"),
        ]
        .into_iter()
        .map(|(pattern, symbol)| {
            (
                Regex::new(pattern).expect("valid latex symbol regex"),
                symbol,
            )
        })
        .collect()
    })
}

/// 应用全部 LaTeX 还原(不含花括号剥离——花括号只在 `$...$` 片段内剥,避免误伤正文)。
fn apply_latex(text: &str) -> String {
    let wrapper = re(
        &LATEX_WRAPPER,
        r"\\(?:text|underline|mathrm|mathbf|mathit|textbf|operatorname)\{([^{}]*)\}",
    );
    let mut s = text.to_string();
    for _ in 0..4 {
        let next = wrapper.replace_all(&s, "$1").into_owned();
        if next == s {
            break;
        }
        s = next;
    }
    s = re(&LATEX_FRAC, r"\\frac\{([^{}]*)\}\{([^{}]*)\}")
        .replace_all(&s, "$1/$2")
        .into_owned();
    s = re(&LATEX_CIRC, r"\^\{*\s*\\circ\s*\}*|\\circ\b")
        .replace_all(&s, "°")
        .into_owned();
    for (symbol_re, symbol) in latex_symbols() {
        s = symbol_re.replace_all(&s, *symbol).into_owned();
    }
    s = re(&LATEX_SUPERSCRIPT, r"\^\{+\s*([^{}]*?)\s*\}+")
        .replace_all(&s, "^$1")
        .into_owned();
    s = re(&LATEX_ENV, r"\\(?:begin|end)\{[a-z*]*\}")
        .replace_all(&s, "")
        .into_owned();
    s
}

fn looks_like_latex(fragment: &str) -> bool {
    fragment.contains('\\') || fragment.contains('^') || fragment.contains('{')
}

fn image_marker(src: &str) -> &'static str {
    if src.contains("seal") {
        "[图:印章]"
    } else {
        "[图]"
    }
}

/// OCR 出口统一净化。见模块注释;对无标记的纯文本是近零成本 no-op。
pub fn sanitize_ocr_markup(text: &str) -> String {
    let mut s = re(&COMMENT, r"<!--[\s\S]*?-->")
        .replace_all(text, "")
        .into_owned();
    let img_src = re(&IMG_SRC, r#"src\s*=\s*["']([^"']*)["']"#);
    s = re(&IMG_TAG, r"(?i)<img\b[^>]*/?>")
        .replace_all(&s, |caps: &regex::Captures<'_>| {
            let tag = caps.get(0).map_or("", |m| m.as_str());
            let src = img_src
                .captures(tag)
                .and_then(|c| c.get(1))
                .map_or("", |m| m.as_str());
            image_marker(src)
        })
        .into_owned();
    s = re(&MD_IMAGE, r"!\[[^\]\r\n]*\]\(([^)\r\n]*)\)")
        .replace_all(&s, |caps: &regex::Captures<'_>| {
            image_marker(caps.get(1).map_or("", |m| m.as_str()))
        })
        .into_owned();
    // 只有内容确实像公式($ 内含 \ ^ {)才解开,普通金额里的 $ 不动。
    s = re(&DOLLAR_FRAGMENT, r"\$\s*([^$\n]{1,200}?)\s*\$")
        .replace_all(&s, |caps: &regex::Captures<'_>| {
            let inner = caps.get(1).map_or("", |m| m.as_str());
            if looks_like_latex(inner) {
                apply_latex(inner).replace(['{', '}'], "")
            } else {
                caps.get(0).map_or("", |m| m.as_str()).to_string()
            }
        })
        .into_owned();
    s = apply_latex(&s);
    s = re(&BR_TAG, r"(?i)<br\s*/?>")
        .replace_all(&s, "\n")
        .into_owned();
    s = re(&DIV_SPAN_TAG, r"(?i)</?(?:div|span)[^>]*>")
        .replace_all(&s, "")
        .into_owned();
    s
}
