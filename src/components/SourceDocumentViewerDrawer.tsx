/**
 * 源文件查看器抽屉(源文件看板重构 · Phase 1 · 2026-06-19)。
 *
 * 点案件源文件 → 右侧抽屉,顶部「处理后 MD / 原件」双 tab:
 * - **处理后 MD**:react-markdown 渲染抽取产物(AI 实际读到的内容,方便核对抽取质量)。
 * - **原件**:PDF/图片**板内**原生渲染(走流式 `asset://` 协议,大扫描件不占内存、自带 Range);
 *   `.docx/.xls/.ppt` 等渲不了的 → 「用系统默认程序打开」兜底。
 *
 * 铁律:只读渲染,绝不改原文件。打开前必须 `await allowCaseAssets(caseFolder)` 把案件目录
 * 加进 asset scope,否则 iframe 首次请求 403(详见 docs/源文件看板重构-执行清单.md Phase 0 结论③)。
 */
import { useEffect, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  X,
  Loader2,
  ExternalLink,
  FolderOpen,
  FileText,
  Pencil,
  Check,
  Search,
  BookOpen,
  Bookmark as BookmarkIcon,
  BookmarkPlus,
} from "lucide-react";

import {
  allowCaseAssets,
  readTextFile,
  readCaseFileBytes,
  openInDefaultApp,
  revealInFinder,
  convertDocToDocx,
  searchInDocument,
  listDocumentBookmarks,
  addDocumentBookmark,
  deleteDocumentBookmark,
} from "@/lib/api";
import {
  workbookToPreviewSheets,
  type SpreadsheetPreviewSheet,
  type SpreadsheetPreviewUtils,
} from "@/lib/spreadsheetPreview";
import type { Document, Bookmark, SearchHit } from "@/lib/types";
import { cn, docDisplayName } from "@/lib/utils";

import { PdfReaderView, type Highlight, type NormRect } from "./PdfReaderView";

type Tab = "md" | "original";

const isPdf = (name: string) => /\.pdf$/i.test(name);
const isImage = (name: string) =>
  /\.(png|jpe?g|webp|tiff?|bmp|gif|jp2)$/i.test(name);
const isNativeText = (name: string) => /\.(md|markdown|txt)$/i.test(name);
// 路 B(2026-06-19):.docx / Excel 用 JS 库板内渲染(零外部依赖、跨平台);
// 老 .doc / .rtf / .odt 先 convert_doc_to_docx(mac textutil / Win soffice)转 .docx 再渲;
// .ppt / .pptx 仍渲不了 → 退回「处理后 MD + 系统打开」。
const isDocx = (name: string) => /\.docx$/i.test(name);
const isSpreadsheet = (name: string) => /\.(xlsx?|csv)$/i.test(name);
// 非 .docx、但能转成 .docx 再渲的老格式(textutil/soffice 支持)
const isConvertibleDoc = (name: string) => /\.(doc|rtf|odt)$/i.test(name);
const isInBoardOffice = (name: string) =>
  isDocx(name) || isSpreadsheet(name) || isConvertibleDoc(name);

export function SourceDocumentViewerDrawer({
  doc,
  caseFolder,
  onClose,
  onRename,
}: {
  doc: Document;
  /** 案件源文件夹(用于 asset scope 授权) */
  caseFolder: string;
  onClose: () => void;
  /** 重命名板内显示名(name=null/空 → 清回原文件名);不传则不显示改名按钮 */
  onRename?: (docId: string, name: string | null) => void;
}) {
  const hasMd = doc.extraction_status === "done" && !!doc.extracted_text_path;
  const nativeText = isNativeText(doc.filename);
  // 处理后 MD 的来源:本身是文本→读原文件;否则读抽取产物
  const mdPath = nativeText ? doc.source_path : (doc.extracted_text_path ?? null);
  const canShowMd = nativeText || hasMd;

  // 默认 tab:能看处理后文本→md;否则直接原件
  const [tab, setTab] = useState<Tab>(canShowMd ? "md" : "original");

  // 头部内联重命名(板内显示名,不动磁盘原件)
  const [renaming, setRenaming] = useState(false);
  const [nameDraft, setNameDraft] = useState("");
  const startRename = () => {
    setNameDraft(docDisplayName(doc));
    setRenaming(true);
  };
  const commitRename = () => {
    const t = nameDraft.trim();
    onRename?.(doc.id, t.length > 0 ? t : null);
    setRenaming(false);
  };

  // Esc 关闭
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // ── 处理后 MD ──
  const [mdText, setMdText] = useState<string | null>(null);
  const [mdLoading, setMdLoading] = useState(false);
  const [mdError, setMdError] = useState<string | null>(null);
  useEffect(() => {
    if (tab !== "md" || !mdPath || mdText !== null) return;
    let cancelled = false;
    setMdLoading(true);
    setMdError(null);
    readTextFile(mdPath)
      .then((t) => !cancelled && setMdText(t))
      .catch((e) => !cancelled && setMdError(String(e)))
      .finally(() => !cancelled && setMdLoading(false));
    return () => {
      cancelled = true;
    };
  }, [tab, mdPath, mdText]);

  // ── 原件:先把案件目录加进 asset scope,再让 iframe/img 请求(避免首次 403) ──
  const [assetReady, setAssetReady] = useState(false);
  const [assetError, setAssetError] = useState<string | null>(null);
  useEffect(() => {
    if (tab !== "original" || assetReady) return;
    let cancelled = false;
    allowCaseAssets(caseFolder)
      .then(() => !cancelled && setAssetReady(true))
      .catch((e) => !cancelled && setAssetError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [tab, caseFolder, assetReady]);

  const assetUrl = assetReady ? convertFileSrc(doc.source_path) : null;

  return (
    // 居中模态(2026-06-20:原来贴右抽屉 → 居中,更聚焦)。点遮罩 / Esc 关闭。
    <div
      className="fixed inset-0 z-[110] flex items-center justify-center bg-black/40 p-4 backdrop-blur-sm sm:p-8"
      onClick={onClose}
    >
      <div
        className="flex h-full max-h-[92vh] w-[min(1080px,96vw)] flex-col overflow-hidden rounded-xl bg-white shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 头:文件名 + tab + 外部操作 */}
        <div className="flex items-center gap-3 border-b border-stone-200 px-5 py-3">
          <FileText className="size-5 shrink-0 text-stone-400" />
          <div className="min-w-0 flex-1">
            {renaming ? (
              <div className="flex items-center gap-1.5">
                <input
                  autoFocus
                  value={nameDraft}
                  onChange={(e) => setNameDraft(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      commitRename();
                    } else if (e.key === "Escape") {
                      e.preventDefault();
                      e.stopPropagation();
                      setRenaming(false);
                    }
                  }}
                  placeholder="如:证据-XX买卖合同"
                  className="min-w-0 flex-1 rounded border border-sky-400 px-2 py-1 text-sm text-stone-800 outline-none"
                />
                <button
                  onClick={commitRename}
                  className="flex items-center gap-1 rounded bg-sky-500 px-2 py-1 text-xs text-white hover:bg-sky-600"
                  title="保存显示名(Enter)"
                >
                  <Check className="size-3.5" />
                </button>
                <button
                  onClick={() => onRename?.(doc.id, null)}
                  className="shrink-0 rounded px-1.5 py-1 text-xs text-stone-400 hover:bg-stone-100"
                  title="恢复原文件名"
                >
                  恢复原名
                </button>
              </div>
            ) : (
              <div className="flex items-center gap-1.5">
                <p className="truncate text-sm font-medium text-stone-800">
                  {docDisplayName(doc)}
                </p>
                {onRename && (
                  <button
                    onClick={startRename}
                    className="shrink-0 rounded p-0.5 text-stone-400 hover:bg-stone-100 hover:text-sky-600"
                    title="重命名(只改板内显示名,不动原文件)"
                  >
                    <Pencil className="size-3.5" />
                  </button>
                )}
              </div>
            )}
            {!renaming && doc.display_name?.trim() && (
              <p className="truncate text-[11px] text-stone-400" title={doc.filename}>
                原文件名:{doc.filename}
              </p>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <button
              onClick={() => openInDefaultApp(doc.source_path)}
              className="flex items-center gap-1 rounded px-2 py-1 text-xs text-stone-500 hover:bg-stone-100"
              title="用系统默认程序打开原件"
            >
              <ExternalLink className="size-3.5" />
              打开
            </button>
            <button
              onClick={() => revealInFinder(doc.source_path)}
              className="flex items-center gap-1 rounded px-2 py-1 text-xs text-stone-500 hover:bg-stone-100"
              title="在文件管理器中显示"
            >
              <FolderOpen className="size-3.5" />
              定位
            </button>
            <button
              onClick={onClose}
              className="ml-1 rounded p-1 text-stone-400 hover:bg-stone-100 hover:text-stone-600"
              title="关闭 (Esc)"
            >
              <X className="size-4" />
            </button>
          </div>
        </div>

        {/* tab 切换 */}
        <div className="flex shrink-0 items-center gap-1 border-b border-stone-200 px-5">
          <TabButton active={tab === "md"} onClick={() => setTab("md")} disabled={!canShowMd}>
            处理后 MD
          </TabButton>
          <TabButton active={tab === "original"} onClick={() => setTab("original")}>
            原件
          </TabButton>
          {tab === "md" && canShowMd && (
            <span className="ml-auto py-1.5 text-[11px] text-stone-400">
              图片 / 表格 / 盖章页请切「原件」核对
            </span>
          )}
        </div>

        {/* 内容 */}
        <div className="min-h-0 flex-1 overflow-auto bg-stone-50">
          {tab === "md" ? (
            !canShowMd ? (
              <Empty text="这份材料没有处理后文本,请切「原件」查看。" />
            ) : mdLoading ? (
              <Loading />
            ) : mdError ? (
              <Empty text={`读取失败:${mdError}`} />
            ) : (
              <div
                className={cn(
                  "px-6 py-5 text-sm leading-relaxed text-foreground",
                  // 简易 prose 样式(对齐 MarkdownModal,避免引入 @tailwindcss/typography)
                  "[&_h1]:mb-3 [&_h1]:mt-5 [&_h1]:text-xl [&_h1]:font-semibold",
                  "[&_h2]:mb-2 [&_h2]:mt-4 [&_h2]:text-base [&_h2]:font-semibold",
                  "[&_h3]:mb-1.5 [&_h3]:mt-3 [&_h3]:text-sm [&_h3]:font-semibold",
                  "[&_p]:my-2",
                  "[&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-6",
                  "[&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-6",
                  "[&_li]:my-1",
                  "[&_code]:rounded [&_code]:bg-muted [&_code]:px-1 [&_code]:py-0.5 [&_code]:font-mono [&_code]:text-[12px]",
                  "[&_pre]:my-3 [&_pre]:overflow-auto [&_pre]:rounded-md [&_pre]:bg-muted [&_pre]:p-3",
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
                  {mdText ?? ""}
                </ReactMarkdown>
              </div>
            )
          ) : (
            <OriginalView
              docId={doc.id}
              filename={doc.filename}
              sourcePath={doc.source_path}
              assetUrl={assetUrl}
              assetError={assetError}
              onOpenExternal={() => openInDefaultApp(doc.source_path)}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function OriginalView({
  docId,
  filename,
  sourcePath,
  assetUrl,
  assetError,
  onOpenExternal,
}: {
  docId: string;
  filename: string;
  sourcePath: string;
  assetUrl: string | null;
  assetError: string | null;
  onOpenExternal: () => void;
}) {
  // PDF / 图片:走 asset 流式协议
  if (isPdf(filename) || isImage(filename)) {
    if (assetError) return <Empty text={`无法加载原件:${assetError}`} />;
    if (!assetUrl) return <Loading />;
    if (isImage(filename)) {
      return (
        <div className="flex min-h-full items-start justify-center p-4">
          <img src={assetUrl} alt={filename} className="max-w-full" />
        </div>
      );
    }
      return (
        <PdfView
          docId={docId}
          assetUrl={assetUrl}
          filename={filename}
          sourcePath={sourcePath}
        />
      );
  }
  // .docx / Excel:JS 库板内渲染(读字节)
  if (isInBoardOffice(filename)) {
    return (
      <OfficeView
        path={sourcePath}
        filename={filename}
        onOpenExternal={onOpenExternal}
      />
    );
  }
  // 老 .doc / .ppt / .rtf / .odt 等渲不了 → 系统打开兜底(内容仍可看「处理后 MD」)
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 text-stone-500">
      <FileText className="size-10 text-stone-300" />
      <p className="px-6 text-center text-sm">
        这种格式无法板内渲染原件。
        <br />
        文字内容可看上方「处理后 MD」,或用系统程序打开原件。
      </p>
      <button
        onClick={onOpenExternal}
        className="flex items-center gap-1.5 rounded-lg bg-sky-500 px-4 py-2 text-sm font-medium text-white hover:bg-sky-600"
      >
        <ExternalLink className="size-4" />
        用系统默认程序打开
      </button>
    </div>
  );
}

/**
 * PDF 板内渲染(asset 流式协议 + iframe)+ 可选工具条:跳页 / 缩放 / 书签。
 *
 * **设计原则(老板 2026-06-20 定):新功能不影响老用法** —— 这些控件只在 PDF 原件页出现,
 * 不碰 MD tab / 其它文件类型;不用书签/搜索 = 跟原来一样看 PDF。
 *
 * 跳页/缩放机制:把 `#page=N&zoom=Z` 追加到 asset URL,并用 `key` 强制 iframe 重挂
 * (WKWebView 原生 PDF 预览对纯 hash 变化不一定响应,重挂确保以新 fragment 重新加载)。
 * `#page` 已真机验证可跳;`#zoom` 同机制待验(不生效后续改 pdf.js / CSS)。
 * 书签:页码级,挂 doc_id(重抽不丢),开庭前标好重要页一点直达;有书签时默认跳到第一个。
 */
function PdfView({
  docId,
  assetUrl,
  filename,
  sourcePath,
}: {
  docId: string;
  assetUrl: string;
  filename: string;
  sourcePath: string;
}) {
  const [page, setPage] = useState<number | null>(null);
  const [jumpNonce, setJumpNonce] = useState(0); // 同页重点也能重跳(原生重挂 / 精读重滚)
  // 渲染模式:native=原生 iframe(默认,大扫描件轻快)/ reader=pdf.js 精读(可缩放/双指/选字)
  const [mode, setMode] = useState<"native" | "reader">("native");
  const [draft, setDraft] = useState("");
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([]);
  // 精读模式选区高亮(按书签 id 绑定,删书签连带删;存父层 → 切模式不丢)
  const [highlights, setHighlights] = useState<Highlight[]>([]);
  const [adding, setAdding] = useState(false);
  const [bmPage, setBmPage] = useState("");
  const [bmLabel, setBmLabel] = useState("");
  // 搜索(关键词 → 命中页)
  const [searchOpen, setSearchOpen] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchHits, setSearchHits] = useState<SearchHit[] | null>(null);
  const [searching, setSearching] = useState(false);

  // 加载书签;有书签则默认跳到第一个(老板:优先显示书签位置)
  useEffect(() => {
    let cancelled = false;
    listDocumentBookmarks(docId)
      .then((bm) => {
        if (cancelled) return;
        setBookmarks(bm);
        if (bm.length > 0) setPage((p) => p ?? bm[0].page);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [docId]);

  // 跳页:原生走 `#page=N`(真机验证可跳)+ key 重挂;精读走滚动(PdfReaderView 内处理)。
  // 缩放归精读模式 —— 原生 PDF 的内容缩放外部控制不了(#zoom 无效、改容器宽只缩放背景)。
  // 跳页:同页重点也重跳 —— 原生靠 reloadKey 含 nonce 重挂 iframe;精读靠 jumpNonce 重滚。
  // focusY(PDF point):精读模式跳到页内精确 Y(点带高亮的书签);原生只能到页级。
  const [focusY, setFocusY] = useState<number | null>(null);
  const src = page ? `${assetUrl}#page=${page}` : assetUrl;
  const reloadKey = `${page ?? 0}|${jumpNonce}`;

  const jumpTo = (p: number, y: number | null = null) => {
    if (p > 0) {
      setPage(p);
      setFocusY(y);
      setJumpNonce((n) => n + 1);
    }
  };
  const jumpToBookmark = (b: Bookmark) => {
    const h = highlights.find((x) => x.id === b.id);
    jumpTo(b.page, h?.rects[0]?.y ?? null); // 有高亮 → 精读跳到那段的精确位置
  };
  const jumpFromDraft = () => {
    const n = parseInt(draft, 10);
    if (Number.isFinite(n)) jumpTo(n);
  };

  // 加书签:rects 非空(精读模式选区)→ 同时存高亮、按新书签 id 绑定。
  const addBookmarkDirect = async (p: number, label: string | null, rects?: NormRect[]) => {
    try {
      const bm = await addDocumentBookmark(docId, p, label);
      setBookmarks((prev) =>
        [...prev, bm].sort((a, b) => a.page - b.page || a.created_at.localeCompare(b.created_at)),
      );
      if (rects && rects.length > 0) {
        setHighlights((prev) => [...prev, { id: bm.id, page: p, rects }]);
      }
    } catch {
      /* 静默:书签失败不影响看 PDF */
    }
  };
  const saveBookmark = async () => {
    const p = parseInt(bmPage, 10) || page || 1;
    await addBookmarkDirect(p, bmLabel.trim() || null);
    setAdding(false);
    setBmLabel("");
  };

  const runSearch = async () => {
    const q = searchQuery.trim();
    if (!q) return;
    setSearching(true);
    try {
      setSearchHits(await searchInDocument(docId, q));
    } catch {
      setSearchHits([]);
    } finally {
      setSearching(false);
    }
  };
  const removeBookmark = async (id: string) => {
    try {
      await deleteDocumentBookmark(id);
      setBookmarks((prev) => prev.filter((b) => b.id !== id));
      setHighlights((prev) => prev.filter((h) => h.id !== id)); // 删书签连带删黄色高亮
    } catch {
      /* 静默 */
    }
  };

  const tbBtn =
    "flex items-center gap-1 rounded border border-stone-300 bg-white px-2 py-0.5 text-stone-600 hover:bg-stone-100";

  return (
    <div className="flex h-full flex-col">
      {/* 工具条:跳页 + 缩放 + 书签(只在 PDF 出现) */}
      <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-stone-100 bg-stone-50 px-3 py-1.5 text-xs text-stone-500">
        {/* 跳页 */}
        <div className="flex items-center gap-1">
          <span>跳到第</span>
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value.replace(/[^0-9]/g, ""))}
            onKeyDown={(e) => {
              if (e.key === "Enter") jumpFromDraft();
            }}
            placeholder="页"
            inputMode="numeric"
            className="w-12 rounded border border-stone-300 px-1.5 py-0.5 text-center outline-none focus:border-sky-400"
          />
          <span>页</span>
          <button
            onClick={jumpFromDraft}
            className="rounded bg-sky-500 px-2 py-0.5 font-medium text-white hover:bg-sky-600"
          >
            跳转
          </button>
          {page && <span className="text-stone-400">第 {page} 页</span>}
        </div>

        {/* 搜索:关键词 → 命中页 */}
        <button
          onClick={() => setSearchOpen((v) => !v)}
          className={cn(tbBtn, searchOpen && "border-sky-400 bg-sky-50 text-sky-600")}
          title="在这份 PDF 里搜关键词、定位到页"
        >
          <Search className="size-3" />
          搜索
        </button>

        {/* 书签:加 */}
        <button
          onClick={() => {
            setBmPage(String(page ?? ""));
            setAdding((v) => !v);
          }}
          className={cn(tbBtn, adding && "border-sky-400 bg-sky-50 text-sky-600")}
          title="给某一页加书签(开庭前标好重要页)"
        >
          <BookmarkPlus className="size-3" />
          加书签
        </button>

        {/* 精读模式 toggle(pdf.js:真缩放/双指/选字书签;上百页大扫描件更吃内存,默认关) */}
        <button
          onClick={() => setMode((m) => (m === "native" ? "reader" : "native"))}
          className={cn(
            tbBtn,
            "ml-auto",
            mode === "reader" && "border-violet-400 bg-violet-50 text-violet-600",
          )}
          title="精读模式:pdf.js 渲染,可真缩放/双指/选中文字做书签(上百页大扫描件更吃内存)"
        >
          <BookOpen className="size-3" />
          {mode === "reader" ? "退出精读" : "精读模式"}
        </button>
      </div>

      {/* 搜索面板 */}
      {searchOpen && (
        <div className="shrink-0 border-b border-stone-100 bg-stone-50/80 px-3 py-2 text-xs">
          <div className="flex items-center gap-2">
            <input
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") runSearch();
              }}
              placeholder="输入关键词,回车搜索"
              autoFocus
              className="min-w-0 flex-1 rounded border border-stone-300 px-2 py-1 outline-none focus:border-sky-400"
            />
            <button
              onClick={runSearch}
              disabled={searching}
              className="rounded bg-sky-500 px-3 py-1 font-medium text-white hover:bg-sky-600 disabled:opacity-60"
            >
              {searching ? "搜索中…" : "搜索"}
            </button>
          </div>
          {searchHits && (
            <div className="mt-2 max-h-48 overflow-auto">
              {searchHits.length === 0 ? (
                <p className="text-stone-400">没找到「{searchQuery}」。</p>
              ) : (
                <ul className="space-y-1">
                  {searchHits.map((h, i) => (
                    <li key={i}>
                      <button
                        onClick={() => h.page && jumpTo(h.page)}
                        disabled={!h.page}
                        className={cn(
                          "flex w-full items-start gap-2 rounded px-2 py-1 text-left",
                          h.page ? "hover:bg-sky-100" : "cursor-default opacity-70",
                        )}
                        title={
                          h.page
                            ? `跳到第 ${h.page} 页`
                            : "该文档无页码信息(重新抽取后可定位到页)"
                        }
                      >
                        <span className="shrink-0 rounded bg-sky-100 px-1.5 py-0.5 font-medium text-sky-700">
                          {h.page ? `第 ${h.page} 页` : "未知页"}
                        </span>
                        <span className="min-w-0 flex-1 truncate text-stone-600">{h.snippet}</span>
                        {h.count > 1 && (
                          <span className="shrink-0 text-stone-400">×{h.count}</span>
                        )}
                      </button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          )}
        </div>
      )}

      {/* 加书签内联表单 */}
      {adding && (
        <div className="flex shrink-0 flex-wrap items-center gap-2 border-b border-stone-100 bg-sky-50/60 px-3 py-1.5 text-xs text-stone-600">
          <span>第</span>
          <input
            value={bmPage}
            onChange={(e) => setBmPage(e.target.value.replace(/[^0-9]/g, ""))}
            placeholder="页码"
            inputMode="numeric"
            className="w-12 rounded border border-stone-300 px-1.5 py-0.5 text-center outline-none focus:border-sky-400"
          />
          <span>页</span>
          <input
            value={bmLabel}
            onChange={(e) => setBmLabel(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") saveBookmark();
            }}
            placeholder="备注(可空,如:权利要求书)"
            className="min-w-0 flex-1 rounded border border-stone-300 px-2 py-0.5 outline-none focus:border-sky-400"
          />
          <button
            onClick={saveBookmark}
            className="rounded bg-sky-500 px-2.5 py-0.5 font-medium text-white hover:bg-sky-600"
          >
            保存
          </button>
          <button
            onClick={() => setAdding(false)}
            className="rounded px-2 py-0.5 text-stone-400 hover:bg-stone-100"
          >
            取消
          </button>
        </div>
      )}

      {/* 书签条:有书签默认显示,点击直达 */}
      {bookmarks.length > 0 && (
        <div className="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-stone-100 bg-white px-3 py-1.5 text-xs">
          <BookmarkIcon className="size-3.5 shrink-0 text-amber-500" />
          {bookmarks.map((b) => (
            <span
              key={b.id}
              className={cn(
                "group/bm flex items-center gap-1 rounded-full border px-2 py-0.5",
                page === b.page
                  ? "border-amber-400 bg-amber-50 text-amber-700"
                  : "border-stone-200 bg-stone-50 text-stone-600 hover:bg-stone-100",
              )}
            >
              <button
                onClick={() => jumpToBookmark(b)}
                title={`跳到第 ${b.page} 页`}
                className="flex items-center gap-1"
              >
                <span className="font-medium">P{b.page}</span>
                {b.label && <span className="max-w-[160px] truncate">{b.label}</span>}
              </button>
              <button
                onClick={() => removeBookmark(b.id)}
                className="text-stone-300 hover:text-destructive"
                title="删除书签"
                aria-label="删除书签"
              >
                <X className="size-3" />
              </button>
            </span>
          ))}
        </div>
      )}

      {/* 渲染:默认原生 iframe(轻快);精读模式走 pdf.js(可缩放/双指/选字) */}
      {mode === "native" ? (
        <iframe key={reloadKey} src={src} title={filename} className="min-h-0 flex-1 border-0" />
      ) : (
        <PdfReaderView
          sourcePath={sourcePath}
          gotoPage={page}
          focusY={focusY}
          jumpNonce={jumpNonce}
          highlights={highlights}
          onAddBookmark={(p, label, rects) => addBookmarkDirect(p, label, rects)}
          onOpenExternal={() => openInDefaultApp(sourcePath)}
          onExit={() => setMode("native")}
        />
      )}
    </div>
  );
}

/** .docx(docx-preview)/ Excel(SheetJS)板内渲染:读字节 → 板内预览。库都是动态 import(代码分包)。 */
function OfficeView({
  path,
  filename,
  onOpenExternal,
}: {
  path: string;
  filename: string;
  onOpenExternal: () => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<"loading" | "ok" | "error">("loading");
  const [err, setErr] = useState<string | null>(null);
  const [sheets, setSheets] = useState<SpreadsheetPreviewSheet[]>([]);

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setErr(null);
    setSheets([]);
    (async () => {
      try {
        if (isSpreadsheet(filename)) {
          // .xlsx / .xls / .csv → 只取文本矩阵,由 React 渲染为 inert text。
          const bytes = await readCaseFileBytes(path); // office 文件小,IPC 字节可接受
          const XLSX = await import("xlsx");
          const wb = XLSX.read(bytes, { type: "array" });
          const previewSheets = workbookToPreviewSheets(wb, XLSX.utils as SpreadsheetPreviewUtils);
          if (cancelled) return;
          setSheets(previewSheets);
        } else {
          // .docx 直接渲;老 .doc/.rtf/.odt 先转成 .docx(mac textutil / Win soffice)再渲
          const docxPath = isConvertibleDoc(filename)
            ? await convertDocToDocx(path)
            : path;
          if (cancelled) return;
          const bytes = await readCaseFileBytes(docxPath);
          if (cancelled || !containerRef.current) return;
          containerRef.current.innerHTML = "";
          const { renderAsync } = await import("docx-preview");
          if (cancelled || !containerRef.current) return;
          await renderAsync(bytes, containerRef.current, undefined, {
            inWrapper: true,
            ignoreWidth: false,
            ignoreHeight: false,
          });
        }
        if (!cancelled) setStatus("ok");
      } catch (e) {
        if (!cancelled) {
          setErr(String(e));
          setStatus("error");
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [path, filename]);

  return (
    <div className="relative min-h-full">
      {status === "loading" && (
        <div className="absolute inset-0 flex items-center justify-center">
          <Loader2 className="size-5 animate-spin text-stone-400" />
        </div>
      )}
      {status === "error" && (
        <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center text-sm text-stone-500">
          <p>板内渲染失败:{err}</p>
          <button
            onClick={onOpenExternal}
            className="flex items-center gap-1.5 rounded-lg bg-sky-500 px-4 py-2 text-sm font-medium text-white hover:bg-sky-600"
          >
            <ExternalLink className="size-4" />
            用系统默认程序打开
          </button>
        </div>
      )}
      {status === "ok" && isSpreadsheet(filename) && <SpreadsheetPreview sheets={sheets} />}
      {!isSpreadsheet(filename) && (
        // 容器常驻挂载(renderAsync 需要已挂载的 DOM 节点)
        <div
          ref={containerRef}
          className={cn(
            "bg-white px-4 py-4",
            status !== "ok" && "hidden",
          )}
        />
      )}
    </div>
  );
}

function SpreadsheetPreview({ sheets }: { sheets: SpreadsheetPreviewSheet[] }) {
  if (sheets.length === 0) {
    return (
      <div className="flex min-h-[180px] items-center justify-center bg-white text-sm text-stone-400">
        这份表格没有可预览内容
      </div>
    );
  }

  return (
    <div className="space-y-5 bg-white px-4 py-4">
      {sheets.map((sheet, sheetIndex) => (
        <section key={`${sheetIndex}-${sheet.name}`} className="min-w-0">
          <div className="mb-1 mt-1 text-xs font-semibold text-stone-500">
            {sheet.name}
          </div>
          {sheet.rows.length === 0 ? (
            <div className="rounded border border-stone-200 px-3 py-2 text-xs text-stone-400">
              空表
            </div>
          ) : (
            <div className="overflow-auto">
              <table className="border-collapse text-xs">
                <tbody>
                  {sheet.rows.map((row, rowIndex) => (
                    <tr key={rowIndex}>
                      {row.map((cell, cellIndex) => (
                        <td
                          key={cellIndex}
                          className="max-w-[360px] whitespace-pre-wrap break-words border border-stone-200 px-2 py-1 align-top text-stone-700"
                        >
                          {cell}
                        </td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          )}
        </section>
      ))}
    </div>
  );
}

function TabButton({
  active,
  disabled,
  onClick,
  children,
}: {
  active: boolean;
  disabled?: boolean;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "border-b-2 px-3 py-2 text-sm font-medium transition",
        active
          ? "border-sky-500 text-sky-600"
          : "border-transparent text-stone-500 hover:text-stone-700",
        disabled && "cursor-not-allowed opacity-40 hover:text-stone-500",
      )}
    >
      {children}
    </button>
  );
}

function Loading() {
  return (
    <div className="flex h-full items-center justify-center text-stone-400">
      <Loader2 className="size-5 animate-spin" />
    </div>
  );
}

function Empty({ text }: { text: string }) {
  return (
    <div className="flex h-full items-center justify-center px-6 text-center text-sm text-stone-400">
      {text}
    </div>
  );
}
