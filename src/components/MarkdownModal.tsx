import { useEffect, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { X, Loader2, ExternalLink, Sparkles, FileDown, FileText as FileTextIcon, Pencil } from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";

import { Button } from "@/components/ui/button";
import { stripFilingHeader } from "@/lib/filing";
import { formatYuan } from "@/lib/format";
import {
  exportFilingDocx,
  exportMdDocx,
  exportMdHtml,
  exportReportDocx,
  exportReportHtml,
  extractDocText,
  extractFieldsFromText,
  readTextFile,
  revealInFinder,
} from "@/lib/api";
import type { ExtractedFields } from "@/lib/types";
import { cn } from "@/lib/utils";

interface Props {
  /** 文件绝对路径 */
  path: string;
  /** 文件名,作为弹窗标题 */
  filename: string;
  /** 类型标签(如 "AI 产物" / "判决书") */
  badge?: string;
  /** 关闭时回调 */
  onClose: () => void;
  /**
   * 案件分析报告导出场景:传入案件 ID + 案件名,header 多出"导出 HTML / Word"按钮
   * (其他文档预览不传,不显示导出)
   */
  exportCase?: { id: string; name: string };
  /**
   * 2026-05-25 V0.1.7 · 通用 MD 导出场景(风险报告 / 深挖报告 / 完整报告)。
   * 直接传 MD 路径 + 导出后默认文件名标题。
   * 2026-05-31 V0.3 M1:文书(save_artifact 产物)额外传 `filing.docId`,
   * 多出一个「Word(法律格式)」按钮走原生 OOXML 法律排版。
   */
  exportMd?: { mdPath: string; title: string; filing?: { docId: string } };
  /**
   * V0.3 D1+D2 · 文书可进 Milkdown 编辑器。**可选** —— 只有 litigation 的 filing 文书传,
   * 报告/执行模块等只读预览不传(MarkdownModal 跨模块共享,不传则不显「✏️ 进行编辑」)。
   */
  onEdit?: () => void;
}

/**
 * 全屏遮罩 + 居中卡片,渲染一个 markdown / html / txt 文件。
 *
 * 不依赖 @radix-ui/react-dialog,纯 React + Tailwind 实现:
 *   - fixed inset-0 overlay
 *   - Esc 关闭
 *   - 点击遮罩关闭
 *   - 标题栏:文件名 + 关闭按钮
 *   - 内容区:react-markdown 渲染(支持 GFM 表格、删除线)
 */
/**
 * 按文件类型判断怎么抽取文本:
 *   - .md / .txt → 直接读
 *   - .html / .htm → 直接读(后面会走 iframe sandbox)
 *   - .docx / .doc / .rtf / .odt → 用 macOS textutil 转纯文本
 */
function pickExtractor(filename: string): {
  kind: "markdown" | "html" | "office";
  fetch: (path: string) => Promise<string>;
} {
  const f = filename.toLowerCase();
  if (/\.html?$/.test(f)) return { kind: "html", fetch: readTextFile };
  if (/\.(docx?|rtf|odt)$/.test(f)) return { kind: "office", fetch: extractDocText };
  return { kind: "markdown", fetch: readTextFile };
}

export function MarkdownModal({ path, filename, badge, onClose, exportCase, exportMd, onEdit }: Props) {
  const [content, setContent] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // LLM 字段抽取(只对 office 文档触发,因为非诉文档现在没明确字段集)
  const [fields, setFields] = useState<ExtractedFields | null>(null);
  const [fieldsLoading, setFieldsLoading] = useState(false);
  const [fieldsError, setFieldsError] = useState<string | null>(null);

  const extractor = pickExtractor(filename);

  useEffect(() => {
    let cancelled = false;
    extractor
      .fetch(path)
      .then((text) => {
        if (cancelled) return;
        setContent(text);
        // 文本就绪后,如果是 office 文档,触发 LLM 字段抽取
        if (extractor.kind === "office" && text.trim().length > 30) {
          setFieldsLoading(true);
          extractFieldsFromText(text)
            .then((f) => {
              if (!cancelled) setFields(f);
            })
            .catch((e) => {
              if (!cancelled) setFieldsError(String(e));
            })
            .finally(() => {
              if (!cancelled) setFieldsLoading(false);
            });
        }
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // extractor 通过 filename 派生,filename 不变就稳定
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [path]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // 渲染分派(跟 extractor 对齐):
  //   html → iframe srcdoc sandbox
  //   office (.docx 等) → 纯文本 pre 标签(保留换行)
  //   markdown → react-markdown
  const renderMode = extractor.kind;

  // 预览只显示正文:剥掉 `<!-- filing · doc_type=.. -->` 元信息头和结尾 <CITATIONS> 协议块
  // (跟编辑器同一套 stripFilingHeader,免得用户在预览里看到这些内部标记)。
  const markdownBody = useMemo(
    () =>
      renderMode === "markdown" && content !== null
        ? stripFilingHeader(content).body
        : "",
    [renderMode, content],
  );

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-foreground/20 px-4 py-8 backdrop-blur-sm animate-in fade-in-0 duration-200"
      onClick={onClose}
    >
      <div
        className="flex h-full max-h-[85vh] w-full max-w-3xl flex-col overflow-hidden rounded-xl border border-border bg-card shadow-2xl animate-in zoom-in-95 fade-in-0 duration-300 ease-out"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 标题栏 */}
        <header className="flex items-center justify-between gap-4 border-b border-border bg-card/95 px-5 py-3.5 backdrop-blur">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <h2 className="truncate text-sm font-semibold text-foreground">
                {filename}
              </h2>
              {badge && (
                <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-caption font-medium text-muted-foreground">
                  {badge}
                </span>
              )}
            </div>
            <p className="mt-0.5 truncate font-mono text-caption text-muted-foreground">
              {path}
            </p>
          </div>
          {onEdit && (
            <Button
              size="sm"
              onClick={onEdit}
              title="在 App 内打开 Milkdown 编辑器,所见即所得地修改这份文书"
              className="shrink-0"
            >
              <Pencil className="size-3.5" />
              进行编辑
            </Button>
          )}
          {exportCase && (
            <ReportExportButtons caseId={exportCase.id} caseName={exportCase.name} />
          )}
          {exportMd && (
            <MdExportButtons
              mdPath={exportMd.mdPath}
              title={exportMd.title}
              filing={exportMd.filing}
            />
          )}
          <Button variant="ghost" size="icon" onClick={onClose} aria-label="关闭">
            <X className="size-4" />
          </Button>
        </header>

        {/* 内容区 */}
        <div className="flex-1 overflow-auto">
          {loading && (
            <div className="flex h-full items-center justify-center">
              <Loader2 className="size-5 animate-spin text-muted-foreground" />
            </div>
          )}
          {error && !loading && (
            <div className="m-5 rounded-md border border-destructive/30 bg-destructive/5 p-4">
              <p className="text-sm font-medium text-destructive">读不出来</p>
              <p className="mt-1 font-mono text-xs text-muted-foreground">
                {error}
              </p>
            </div>
          )}
          {!loading && !error && content !== null && (
            <>
              {renderMode === "html" ? (
                <HtmlPreview html={content} />
              ) : renderMode === "office" ? (
                <>
                  <ExtractedFieldsCard
                    fields={fields}
                    loading={fieldsLoading}
                    error={fieldsError}
                  />
                  <OfficeTextPreview text={content} />
                </>
              ) : (
                <div
                  className={cn(
                    "px-6 py-5",
                    // 简易 prose 样式(避免引入 @tailwindcss/typography)
                    "prose-md text-sm leading-relaxed text-foreground",
                    "[&_h1]:mb-3 [&_h1]:mt-5 [&_h1]:text-xl [&_h1]:font-semibold",
                    "[&_h2]:mb-2 [&_h2]:mt-4 [&_h2]:text-base [&_h2]:font-semibold",
                    "[&_h3]:mb-1.5 [&_h3]:mt-3 [&_h3]:text-sm [&_h3]:font-semibold",
                    "[&_p]:my-2",
                    "[&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-6",
                    "[&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-6",
                    "[&_li]:my-1",
                    "[&_code]:rounded [&_code]:bg-muted [&_code]:px-1 [&_code]:py-0.5 [&_code]:font-mono [&_code]:text-[12px]",
                    "[&_pre]:my-3 [&_pre]:overflow-auto [&_pre]:rounded-md [&_pre]:bg-muted [&_pre]:p-3",
                    "[&_pre>code]:bg-transparent [&_pre>code]:p-0",
                    "[&_a]:text-foreground [&_a]:underline [&_a]:underline-offset-2",
                    "[&_strong]:font-semibold",
                    "[&_blockquote]:my-3 [&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-muted-foreground",
                    "[&_table]:my-3 [&_table]:w-full [&_table]:border-collapse [&_table]:text-xs",
                    "[&_th]:border [&_th]:border-border [&_th]:bg-muted/50 [&_th]:px-2 [&_th]:py-1.5 [&_th]:text-left [&_th]:font-medium",
                    "[&_td]:border [&_td]:border-border [&_td]:px-2 [&_td]:py-1.5",
                    "[&_hr]:my-4 [&_hr]:border-border",
                  )}
                >
                  <ReactMarkdown remarkPlugins={[remarkGfm]}>
                    {markdownBody}
                  </ReactMarkdown>
                </div>
              )}
            </>
          )}
        </div>

        {/* 底部 hint */}
        <footer className="border-t border-border bg-card/95 px-5 py-2 text-caption text-muted-foreground">
          按 Esc 关闭 · 原文件路径上面 · 工具不会修改原文件
        </footer>
      </div>
    </div>
  );
}

/** HTML 文件用 iframe srcdoc 沙盒渲染,避免污染主页面样式 */
function HtmlPreview({ html }: { html: string }) {
  return (
    <iframe
      title="HTML 预览"
      srcDoc={html}
      sandbox="allow-same-origin"
      className="size-full border-0"
    />
  );
}

/**
 * Office 文档(.docx 等)走 textutil 抽出的纯文本,直接 pre-wrap 显示。
 * 保留原始换行 + 中文等宽友好的衬线字体(贴近原 Word 排版感)。
 */
function OfficeTextPreview({ text }: { text: string }) {
  return (
    <div className="px-6 py-5">
      <div className="mb-3 rounded-md border border-amber-200/50 bg-amber-50/50 px-3 py-2 text-label text-amber-800">
        ⚠️ 这是用 macOS textutil 抽出的纯文本,会丢失图片、表格格式、字体样式。
        要看完整排版请双击外面的文档行用 Word/Pages 打开。
      </div>
      <pre className="whitespace-pre-wrap break-words font-sans text-sm leading-relaxed text-foreground">
        {text}
      </pre>
    </div>
  );
}

/**
 * AI 抽出的关键字段卡片。在 office 文档预览顶部显示,让律师一眼看到
 * 案号/当事人/金额/日期,不用通读全文。
 */
function ExtractedFieldsCard({
  fields,
  loading,
  error,
}: {
  fields: ExtractedFields | null;
  loading: boolean;
  error: string | null;
}) {
  if (!loading && !fields && !error) return null;

  return (
    <div className="border-b border-border bg-gradient-to-b from-muted/30 to-transparent px-6 py-4 animate-in fade-in-0 slide-in-from-top-2 duration-300">
      <div className="mb-2 flex items-center gap-1.5">
        <Sparkles className="size-3.5 text-foreground" />
        <span className="text-xs font-semibold text-foreground">
          AI 抽取的关键信息
        </span>
        <span className="text-caption text-muted-foreground">
          · 本机 MiniCPM-V 4.6
        </span>
        {loading && (
          <Loader2 className="size-3 animate-spin text-muted-foreground" />
        )}
      </div>

      {error && (
        <p className="font-mono text-caption text-destructive">
          抽取失败:{error}
        </p>
      )}

      {fields && (
        <div className="grid grid-cols-2 gap-x-6 gap-y-2 text-xs">
          <Field label="案号" value={fields.case_no} mono />
          <Field label="受理法院" value={fields.court} />
          <Field
            label="原告"
            value={fields.plaintiffs.length > 0 ? fields.plaintiffs.join("、") : null}
          />
          <Field
            label="被告"
            value={fields.defendants.length > 0 ? fields.defendants.join("、") : null}
          />
          <Field label="案由" value={fields.cause} />
          <Field
            label="诉讼金额"
            value={
              fields.claim_amount !== null
                ? formatYuan(fields.claim_amount)
                : null
            }
            mono
          />
          <Field label="起诉日期" value={fields.filed_at} mono />
        </div>
      )}
    </div>
  );
}

function Field({
  label,
  value,
  mono = false,
}: {
  label: string;
  value: string | null;
  mono?: boolean;
}) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span
        className={cn(
          "truncate font-medium",
          value ? "text-foreground" : "text-muted-foreground/40",
          mono && "font-mono",
        )}
      >
        {value ?? "—"}
      </span>
    </div>
  );
}

// 让 ExternalLink 标记为已用,以备后续加"用系统应用打开"按钮
void ExternalLink;

/* ============================ 报告导出按钮组(2026-05-24 j) ============================ */
function ReportExportButtons({ caseId, caseName }: { caseId: string; caseName: string }) {
  const [busy, setBusy] = useState<"html" | "docx" | null>(null);

  const runExport = async (kind: "html" | "docx") => {
    const defaultName = `${caseName}_案件分析报告.${kind === "html" ? "html" : "docx"}`;
    const filters =
      kind === "html"
        ? [{ name: "HTML", extensions: ["html"] }]
        : [{ name: "Word", extensions: ["docx"] }];
    let savePath: string | null;
    try {
      savePath = await save({ defaultPath: defaultName, filters });
    } catch (e) {
      alert(`打开保存对话框失败:${e}`);
      return;
    }
    if (!savePath) return; // 用户取消
    setBusy(kind);
    try {
      const fn = kind === "html" ? exportReportHtml : exportReportDocx;
      const written = await fn(caseId, savePath);
      // 完成 → Finder 中显示
      try {
        await revealInFinder(written);
      } catch {
        /* 不阻塞 */
      }
    } catch (e) {
      alert(`导出失败:${e}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="flex shrink-0 items-center gap-1.5">
      <Button
        variant="outline"
        size="sm"
        onClick={() => runExport("html")}
        disabled={busy !== null}
        title="导出为 HTML(陶土红 × 羊皮纸专业风格,内嵌 CSS,单文件可分享)"
      >
        {busy === "html" ? (
          <Loader2 className="size-3.5 animate-spin" />
        ) : (
          <FileDown className="size-3.5" />
        )}
        HTML
      </Button>
      <Button
        variant="outline"
        size="sm"
        onClick={() => runExport("docx")}
        disabled={busy !== null}
        title="导出为 Word 文档(.docx,简单排版,可编辑)"
      >
        {busy === "docx" ? (
          <Loader2 className="size-3.5 animate-spin" />
        ) : (
          <FileTextIcon className="size-3.5" />
        )}
        Word
      </Button>
    </div>
  );
}

/* ============================ 通用 MD 导出按钮组(V0.1.7) ============================ */
function MdExportButtons({
  mdPath,
  title,
  filing,
}: {
  mdPath: string;
  title: string;
  filing?: { docId: string };
}) {
  const [busy, setBusy] = useState<"html" | "docx" | "filing" | null>(null);

  const runExport = async (kind: "html" | "docx" | "filing") => {
    const defaultName = `${title}.${kind === "html" ? "html" : "docx"}`;
    const filters =
      kind === "html"
        ? [{ name: "HTML", extensions: ["html"] }]
        : [{ name: "Word", extensions: ["docx"] }];
    let savePath: string | null;
    try {
      savePath = await save({ defaultPath: defaultName, filters });
    } catch (e) {
      alert(`打开保存对话框失败:${e}`);
      return;
    }
    if (!savePath) return;
    setBusy(kind);
    try {
      let written: string;
      if (kind === "html") {
        written = await exportMdHtml(mdPath, title, savePath);
      } else if (kind === "filing" && filing) {
        // V0.3 M1:法律格式(原生 OOXML,复刻 quote.law 排版)
        written = await exportFilingDocx(filing.docId, savePath);
      } else {
        written = await exportMdDocx(mdPath, title, savePath);
      }
      try {
        await revealInFinder(written);
      } catch {
        /* 不阻塞 */
      }
    } catch (e) {
      alert(`导出失败:${e}`);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="flex shrink-0 items-center gap-1.5">
      <Button
        variant="outline"
        size="sm"
        onClick={() => runExport("html")}
        disabled={busy !== null}
        title="导出为 HTML(陶土红 × 羊皮纸专业风格,单文件可分享)"
      >
        {busy === "html" ? (
          <Loader2 className="size-3.5 animate-spin" />
        ) : (
          <FileDown className="size-3.5" />
        )}
        HTML
      </Button>
      {filing ? (
        <Button
          variant="outline"
          size="sm"
          onClick={() => runExport("filing")}
          disabled={busy !== null}
          title="导出为 Word(法律格式):方正小标宋标题 / 黑体小标题 / 仿宋正文 / 两端对齐 / 首行缩进2字,复刻律所文书排版"
        >
          {busy === "filing" ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <FileTextIcon className="size-3.5" />
          )}
          Word(法律格式)
        </Button>
      ) : (
        <Button
          variant="outline"
          size="sm"
          onClick={() => runExport("docx")}
          disabled={busy !== null}
          title="导出为 Word 文档(.docx,可编辑)"
        >
          {busy === "docx" ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <FileTextIcon className="size-3.5" />
          )}
          Word
        </Button>
      )}
    </div>
  );
}
