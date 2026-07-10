/**
 * PDF 精读模式(pdf.js 自渲染 · 2026-06-20)。
 *
 * 为什么单独一套:WKWebView 原生 PDF(默认 iframe)无法被外部控制缩放、拿不到文本层
 * (选字/高亮做不了)。精读模式用 pdf.js 自己把页画到 canvas + 文本层,于是能:
 *   - 真·缩放(按钮 + 触控板双指)、按 devicePixelRatio 高清渲染(不糊)
 *   - 选中文字 → 弹「加书签」按钮,点了才加;加过的选区黄色高亮常亮
 *
 * 书签 + 高亮**联动**:书签存父层(PdfView)、落库;高亮(选区坐标)也存父层、按书签 id 绑,
 * 删书签连带删高亮、切模式不丢。高亮坐标存页内归一化(PDF point),随缩放重算。
 * (跨「关掉重开」持久化仍待落库,目前本次打开内有效。)
 *
 * ⚠️ 取舍(老板已知):pdf.js 逐页栅格化,上百页大扫描件比原生更吃内存;默认仍原生 iframe,
 * 只渲滚到的页(IntersectionObserver)。跨平台:worker 走 Vite `?url`、readFile 拿字节。
 */
import { useEffect, useRef, useState } from "react";
import { Loader2, Minus, Plus, Maximize2, ExternalLink, X, BookmarkPlus } from "lucide-react";
import "pdfjs-dist/web/pdf_viewer.css";

import { readCaseFileBytes } from "@/lib/api";

// pdfjs 类型不强约束(动态 import),用 any 保持松耦合。
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type PdfDoc = any;

/** 页内归一化矩形(PDF point 空间,= CSS px / zoom),渲染时 ×zoom 还原。 */
export interface NormRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** 一条高亮(跟书签 id 绑定,删书签连带删)。 */
export interface Highlight {
  id: string;
  page: number;
  rects: NormRect[];
}

/** 待加书签的选区(选中后暂存,点按钮才落地)。 */
interface PendingSel {
  btnX: number;
  btnY: number;
  page: number;
  text: string;
  rects: NormRect[];
}

export function PdfReaderView({
  sourcePath,
  gotoPage,
  focusY,
  jumpNonce,
  highlights,
  onAddBookmark,
  onOpenExternal,
  onExit,
}: {
  sourcePath: string;
  /** 跳页目标(父层跳页/书签/搜索命中时改) */
  gotoPage: number | null;
  /** 页内精确 Y(PDF point);非空 → 滚到页内该位置(点带高亮的书签),null → 页顶 */
  focusY: number | null;
  /** 跳页计数:同页重点也能重新滚动 */
  jumpNonce: number;
  /** 父层维护的高亮(按书签 id) */
  highlights: Highlight[];
  /** 选中文字 + 点按钮 → 父层加书签 + 存这段选区高亮(rects=页内归一化坐标) */
  onAddBookmark: (page: number, label: string, rects: NormRect[]) => void;
  onOpenExternal: () => void;
  /** 退出精读(回原生) */
  onExit: () => void;
}) {
  const [status, setStatus] = useState<"loading" | "ok" | "error">("loading");
  const [err, setErr] = useState<string | null>(null);
  const [zoom, setZoom] = useState(1.2);
  const [numPages, setNumPages] = useState(0);
  const [pending, setPending] = useState<PendingSel | null>(null);
  const pdfRef = useRef<PdfDoc | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const pageRefs = useRef<Array<HTMLDivElement | null>>([]);

  const clampZoom = (z: number) => Math.min(4, Math.max(0.4, Math.round(z * 20) / 20));

  // 加载 PDF
  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setErr(null);
    pdfRef.current = null;
    setPending(null);
    (async () => {
      try {
        const pdfjs = await import("pdfjs-dist");
        const workerUrl = (await import("pdfjs-dist/build/pdf.worker.min.mjs?url")).default;
        pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;
        const bytes = await readCaseFileBytes(sourcePath);
        if (cancelled) return;
        const pdf = await pdfjs.getDocument({ data: bytes }).promise;
        if (cancelled) return;
        pdfRef.current = pdf;
        pageRefs.current = new Array(pdf.numPages).fill(null);
        setNumPages(pdf.numPages);
        setStatus("ok");
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
  }, [sourcePath]);

  // 跳页(同页重点靠 jumpNonce 也重滚)。focusY 非空 → 滚到页内精确 Y(点带高亮的书签)。
  useEffect(() => {
    if (status !== "ok" || !gotoPage) return;
    const box = pageRefs.current[gotoPage - 1];
    const cont = scrollRef.current;
    if (!box) return;
    if (focusY != null && cont) {
      const delta =
        box.getBoundingClientRect().top - cont.getBoundingClientRect().top + focusY * zoom - 48;
      cont.scrollTo({ top: cont.scrollTop + delta, behavior: "smooth" });
    } else {
      box.scrollIntoView({ block: "start", behavior: "smooth" });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [gotoPage, jumpNonce, status, numPages]);

  // 触控板双指缩放
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      setPending(null);
      setZoom((z) => clampZoom(z - e.deltaY * 0.01));
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  // 选中文字 → 暂存选区 + 弹「加书签」按钮(不自动加)
  const handleMouseUp = () => {
    const sel = window.getSelection();
    const text = sel?.toString().trim();
    if (!text || !sel || sel.rangeCount === 0) {
      setPending(null);
      return;
    }
    let node = sel.anchorNode as HTMLElement | null;
    let pageEl: HTMLElement | null = null;
    while (node) {
      if (node instanceof HTMLElement && node.dataset?.page) {
        pageEl = node;
        break;
      }
      node = (node.parentNode as HTMLElement) ?? null;
    }
    if (!pageEl) {
      setPending(null);
      return;
    }
    const page = parseInt(pageEl.dataset.page!, 10);
    const boxRect = pageEl.getBoundingClientRect();
    const range = sel.getRangeAt(0);
    const clientRects = Array.from(range.getClientRects());
    const rects: NormRect[] = clientRects.map((r) => ({
      x: (r.left - boxRect.left) / zoom,
      y: (r.top - boxRect.top) / zoom,
      w: r.width / zoom,
      h: r.height / zoom,
    }));
    const last = clientRects[clientRects.length - 1];
    setPending({
      btnX: last ? last.right : boxRect.left,
      btnY: last ? last.bottom : boxRect.top,
      page,
      text,
      rects,
    });
  };

  const confirmBookmark = () => {
    if (!pending) return;
    const label = pending.text.length > 40 ? `${pending.text.slice(0, 40)}…` : pending.text;
    onAddBookmark(pending.page, label, pending.rects);
    window.getSelection()?.removeAllRanges();
    setPending(null);
  };

  const tbBtn =
    "flex items-center gap-1 rounded border border-stone-300 bg-white px-2 py-0.5 hover:bg-stone-100";

  return (
    <div className="relative flex h-full flex-col">
      <div className="flex shrink-0 items-center gap-2 border-b border-stone-100 bg-stone-50 px-3 py-1.5 text-xs text-stone-500">
        <button
          onClick={onExit}
          className="flex items-center gap-1 rounded border border-violet-300 bg-violet-50 px-2 py-0.5 font-medium text-violet-600 hover:bg-violet-100"
          title="退出精读模式,回到原生预览"
        >
          <X className="size-3" />
          退出精读
        </button>
        <span className="text-stone-300">|</span>
        <button onClick={() => setZoom((z) => clampZoom(z - 0.2))} className={tbBtn} title="缩小">
          <Minus className="size-3" />
        </button>
        <span className="w-10 text-center tabular-nums">{Math.round(zoom * 100)}%</span>
        <button onClick={() => setZoom((z) => clampZoom(z + 0.2))} className={tbBtn} title="放大">
          <Plus className="size-3" />
        </button>
        <button onClick={() => setZoom(1.2)} className={tbBtn} title="恢复">
          <Maximize2 className="size-3" />
          适应
        </button>
        <span className="ml-auto text-stone-400">选中文字 → 点「加书签」· 双指捏合缩放</span>
      </div>

      <div
        ref={scrollRef}
        className="min-h-0 flex-1 select-text overflow-auto bg-stone-200"
        onMouseUp={handleMouseUp}
        onMouseDown={() => setPending(null)}
      >
        {status === "loading" && (
          <div className="flex h-full items-center justify-center text-stone-400">
            <Loader2 className="size-5 animate-spin" />
          </div>
        )}
        {status === "error" && (
          <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center text-sm text-stone-500">
            <p>精读模式加载失败:{err}</p>
            <p className="text-xs text-stone-400">可点「退出精读」用原生预览,或用系统程序打开。</p>
            <button
              onClick={onOpenExternal}
              className="flex items-center gap-1.5 rounded-lg bg-sky-500 px-4 py-2 text-sm font-medium text-white hover:bg-sky-600"
            >
              <ExternalLink className="size-4" />
              用系统默认程序打开
            </button>
          </div>
        )}
        {status === "ok" &&
          pdfRef.current &&
          Array.from({ length: numPages }, (_, i) => (
            <PdfPageBox
              key={i + 1}
              pdf={pdfRef.current}
              pageNumber={i + 1}
              zoom={zoom}
              registerRef={(el) => (pageRefs.current[i] = el)}
              highlights={highlights.filter((h) => h.page === i + 1).flatMap((h) => h.rects)}
            />
          ))}
      </div>

      {pending && (
        <button
          onClick={confirmBookmark}
          style={{ position: "fixed", left: pending.btnX, top: pending.btnY + 6, zIndex: 140 }}
          className="flex items-center gap-1 rounded-md bg-amber-500 px-2 py-1 text-xs font-medium text-white shadow-lg hover:bg-amber-600"
        >
          <BookmarkPlus className="size-3" />
          加书签
        </button>
      )}
    </div>
  );
}

/** 单页:进视口才渲染(高清 canvas + 文本层 + 已加书签的黄色高亮)。zoom 变则重渲染。 */
function PdfPageBox({
  pdf,
  pageNumber,
  zoom,
  registerRef,
  highlights,
}: {
  pdf: PdfDoc;
  pageNumber: number;
  zoom: number;
  registerRef: (el: HTMLDivElement | null) => void;
  highlights: NormRect[];
}) {
  const boxRef = useRef<HTMLDivElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const textRef = useRef<HTMLDivElement>(null);
  const [dims, setDims] = useState<{ w: number; h: number } | null>(null);
  const renderedScale = useRef<number | null>(null);
  const visibleRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    pdf.getPage(pageNumber).then((page: PdfDoc) => {
      if (cancelled) return;
      const vp = page.getViewport({ scale: zoom });
      setDims({ w: vp.width, h: vp.height });
    });
    return () => {
      cancelled = true;
    };
  }, [pdf, pageNumber, zoom]);

  const renderPage = async () => {
    if (!visibleRef.current || renderedScale.current === zoom) return;
    const page = await pdf.getPage(pageNumber);
    const vp = page.getViewport({ scale: zoom });
    const canvas = canvasRef.current;
    const ctx = canvas?.getContext("2d");
    if (!canvas || !ctx) return;
    // 高清:bitmap 按 devicePixelRatio 放大,CSS 尺寸保持 vp(否则 Retina 上糊)
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.floor(vp.width * dpr);
    canvas.height = Math.floor(vp.height * dpr);
    canvas.style.width = `${Math.floor(vp.width)}px`;
    canvas.style.height = `${Math.floor(vp.height)}px`;
    await page.render({
      canvasContext: ctx,
      viewport: vp,
      transform: dpr !== 1 ? [dpr, 0, 0, dpr, 0, 0] : undefined,
    }).promise;
    renderedScale.current = zoom;
    try {
      const tl = textRef.current;
      if (tl) {
        tl.innerHTML = "";
        tl.style.setProperty("--scale-factor", String(zoom));
        tl.style.width = `${vp.width}px`;
        tl.style.height = `${vp.height}px`;
        const { TextLayer } = await import("pdfjs-dist");
        const layer = new TextLayer({
          textContentSource: page.streamTextContent(),
          container: tl,
          viewport: vp,
        });
        await layer.render();
      }
    } catch {
      /* 文本层失败:canvas 仍可看,选字降级 */
    }
  };

  useEffect(() => {
    renderedScale.current = null;
    const el = boxRef.current;
    if (!el) return;
    const io = new IntersectionObserver(
      (entries) => {
        visibleRef.current = entries[0]?.isIntersecting ?? false;
        if (visibleRef.current) void renderPage();
      },
      { rootMargin: "400px" },
    );
    io.observe(el);
    return () => io.disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [zoom]);

  return (
    <div
      ref={(el) => {
        boxRef.current = el;
        registerRef(el);
      }}
      data-page={pageNumber}
      className="relative mx-auto my-3 bg-white shadow-md"
      style={dims ? { width: dims.w, height: dims.h } : { width: "80%", height: 600 }}
    >
      <canvas ref={canvasRef} className="block" />
      {highlights.map((r, i) => (
        <div
          key={i}
          className="pointer-events-none absolute bg-yellow-300/45"
          style={{ left: r.x * zoom, top: r.y * zoom, width: r.w * zoom, height: r.h * zoom }}
        />
      ))}
      <div ref={textRef} className="textLayer" style={{ userSelect: "text" }} />
      <span className="absolute -top-2.5 left-1 rounded bg-stone-700/70 px-1 text-[10px] text-white">
        {pageNumber}
      </span>
    </div>
  );
}
