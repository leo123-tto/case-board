/**
 * 「处理后文本」预览的净化层(2026-07-27 真机反馈:OCR 文本里满屏 HTML/LaTeX 代码)。
 *
 * MinerU 等云端 OCR 输出的 Markdown 里混着两类标记:
 *   - `<table border=1 ...>` HTML 表格 —— 是表格数据本体(规格/公差/单价都在里面),
 *     对 AI 有价值必须保留在抽取文本里;但预览层不该给律师看裸代码,要渲染成真表格。
 *   - `$ \underline{\text{XX}} $` 等 LaTeX 片段 —— 表示原文下划线/上标,预览时解开取正文。
 *
 * 本模块只做**展示层**转换,绝不改写落盘的抽取文本(AI 读的还是原始 MD)。
 * 纯函数、不依赖 React,单测友好;表格 DOM 解析交给浏览器 DOMParser。
 */

export type DerivedSegment =
  | { kind: "markdown"; text: string }
  | { kind: "table"; html: string };

const TABLE_RE = /<table\b[\s\S]*?<\/table>/gi;

/** 把处理后文本切成「markdown 段」与「HTML 表格段」交替序列。 */
export function splitDerivedSegments(text: string): DerivedSegment[] {
  const segments: DerivedSegment[] = [];
  let last = 0;
  for (const match of text.matchAll(TABLE_RE)) {
    const index = match.index ?? 0;
    if (index > last) segments.push({ kind: "markdown", text: text.slice(last, index) });
    segments.push({ kind: "table", html: match[0] });
    last = index + match[0].length;
  }
  if (last < text.length) segments.push({ kind: "markdown", text: text.slice(last) });
  return segments;
}

const LATEX_SYMBOLS: [RegExp, string][] = [
  [/\\times\b/g, "×"],
  [/\\pm\b/g, "±"],
  [/\\mp\b/g, "∓"],
  [/\\leq?\b/g, "≤"],
  [/\\geq?\b/g, "≥"],
  [/\\approx\b/g, "≈"],
  [/\\sim\b/g, "~"],
  [/\\cdot\b/g, "·"],
  [/\\div\b/g, "÷"],
];

/** 全套 LaTeX 记号还原(花括号剥离只在 `$...$` 片段内做,避免误伤正文)。 */
function applyLatex(raw: string): string {
  let s = raw;
  for (let i = 0; i < 4; i++) {
    const next = s.replace(
      /\\(?:text|underline|mathrm|mathbf|mathit|textbf|operatorname)\{([^{}]*)\}/g,
      "$1",
    );
    if (next === s) break;
    s = next;
  }
  s = s.replace(/\\frac\{([^{}]*)\}\{([^{}]*)\}/g, "$1/$2");
  s = s.replace(/\^\{*\s*\\circ\s*\}*|\\circ\b/g, "°");
  for (const [re, symbol] of LATEX_SYMBOLS) s = s.replace(re, symbol);
  s = s.replace(/\^\{+\s*([^{}]*?)\s*\}+/g, "^$1");
  return s.replace(/\\(?:begin|end)\{[a-z*]*\}/g, "");
}

function imageMarker(src: string): string {
  return src.includes("seal") ? "〔印章〕" : "〔图〕";
}

/**
 * 解开 OCR 混入的 LaTeX 片段(与 Rust `ingest::sanitize` 同规则,覆盖旧落盘文件):
 *   `\pm 0.04` → `± 0.04`;`5.0 \times 1500` → `5.0 × 1500`;
 *   `$ \underline{\text{无锡宝思迪}} $` → `无锡宝思迪`;`$ ^{{a}} $` → `^a`。
 * `$...$` 只在内含 `\` `^` `{` 时按公式解开,普通美元金额不动。
 */
export function cleanDerivedInline(raw: string): string {
  const s = raw.replace(/\$\s*([^$\n]{1,200}?)\s*\$/g, (whole, inner: string) =>
    /[\\^{]/.test(inner) ? applyLatex(inner).replace(/[{}]/g, "") : whole,
  );
  return applyLatex(s);
}

/**
 * markdown 段净化:解 LaTeX + 清 `<!-- -->` 注释 + 图片占位压成小标记(`seal`=印章框)
 * + `<br>` 转换行 + 剥 `<div>/<span>` 排版壳(保留内容)。
 */
export function cleanDerivedMarkdown(raw: string): string {
  return cleanDerivedInline(raw)
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/<img\b[^>]*\/?>/gi, (tag) => {
      const src = /src\s*=\s*["']([^"']*)["']/.exec(tag)?.[1] ?? "";
      return imageMarker(src);
    })
    .replace(/!\[[^\]\r\n]*\]\(([^)\r\n]*)\)/g, (_whole, src: string) => imageMarker(src))
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/?(?:div|span)[^>]*>/gi, "");
}

export interface DerivedTableCell {
  text: string;
  th: boolean;
  colSpan: number;
  rowSpan: number;
}

/**
 * 用 DOMParser 把 OCR 的 HTML 表格解析成行×格结构(保留 rowspan/colspan,合同表格大量使用)。
 * 只取文本与跨行跨列属性,style/事件等一律丢弃 —— 输出交给 React 建元素,天然无注入面。
 * 解析不出表格时返回 null,调用方回退显示原文。
 */
export function parseDerivedTable(html: string): DerivedTableCell[][] | null {
  try {
    const doc = new DOMParser().parseFromString(html, "text/html");
    const table = doc.querySelector("table");
    if (!table) return null;
    const rows = Array.from(table.querySelectorAll("tr")).map((tr) =>
      Array.from(tr.querySelectorAll("th,td")).map((cell) => ({
        th: cell.tagName === "TH",
        text: cleanDerivedInline(cell.textContent ?? "").trim(),
        colSpan: clampSpan(cell.getAttribute("colspan")),
        rowSpan: clampSpan(cell.getAttribute("rowspan")),
      })),
    );
    return rows.some((row) => row.length > 0) ? rows : null;
  } catch {
    return null;
  }
}

function clampSpan(raw: string | null): number {
  const value = Number.parseInt(raw ?? "1", 10);
  return Number.isFinite(value) && value >= 1 ? Math.min(value, 100) : 1;
}
