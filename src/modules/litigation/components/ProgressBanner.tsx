import { Loader2 } from "lucide-react";

import { type DocOcrStatusEvent, type ProgressEvent } from "@/lib/types";
import { cn } from "@/lib/utils";

/** 云端 OCR 轮询子状态 → 一行人话(大图扫描件排队/识别中,治"看着卡死")。 */
function ocrSubLabel(s: DocOcrStatusEvent): string {
  const waited = `已等 ${s.elapsed_secs}s`;
  const pages =
    s.pages_total && s.pages_total > 0
      ? ` · 第 ${s.pages_done ?? 0}/${s.pages_total} 页`
      : "";
  switch (s.phase) {
    case "queued":
      return `☁️ 云端排队中…(${waited},大图扫描件常需 1~3 分钟,没卡)`;
    case "converting":
      return `☁️ 云端转换中 · ${waited}${pages}`;
    case "processing":
    default:
      return `☁️ 云端识别中 · ${waited}${pages}`;
  }
}

/* ------------------------------------------------------------------ */
/* 进度条:全局浮在顶部,显示后台 LLM 抽取进度                          */
/* ------------------------------------------------------------------ */

export function ProgressBanner({
  progress,
  ocrSub,
  minimized,
  onToggleMinimize,
  onClose,
}: {
  progress: ProgressEvent;
  /** 2026-06-14:当前文档云端 OCR 轮询子状态(独立于主进度,不影响百分比) */
  ocrSub?: DocOcrStatusEvent | null;
  minimized: boolean;
  onToggleMinimize: () => void;
  onClose: () => void;
}) {
  let percent = 0;
  let label = "";
  let filename: string | null = null;
  let ocrProvider: "local" | "cloud" | null = null;
  let llmProvider: "local" | "cloud" | null = null;

  switch (progress.stage) {
    case "started":
      percent = 0;
      label = `开始处理 ${progress.total} 份文档…`;
      ocrProvider = progress.ocr_provider;
      llmProvider = progress.llm_provider;
      break;
    case "doc_started":
      // 2026-05-24 i:并发场景下 index 不能算 percent(回退 bug),DocStarted 没 completed_count,
      // 这里不更新 percent(沿用上一个 DocFinished 的 percent),只更新 filename / providers
      label = `处理中 · ${progress.filename}`;
      filename = progress.filename;
      ocrProvider = progress.ocr_provider;
      llmProvider = progress.llm_provider;
      break;
    case "doc_ocr_status":
      // 不该走到这:App.tsx 把 doc_ocr_status 路由到独立 ocrSub prop,不会塞进 progress。
      // 留个显式分支,避免它意外当成默认把 percent 重置成 0(进度条闪回)。
      break;
    case "doc_finished":
      // 用 completed_count(单调递增),不要用 index(并发顺序乱)
      percent = Math.round((progress.completed_count / progress.total) * 100);
      label = `已完成 ${progress.completed_count} / ${progress.total}`;
      filename = progress.filename;
      break;
    case "analyzing":
      // 全案 LLM 分析:没有细粒度进度,保持 100%(文档都完成了)+ 转圈 + 明示在干什么
      percent = 100;
      label = "📖 AI 通读全案分析中(更新画像/报告,通常 1~3 分钟)…";
      break;
    case "completed":
      percent = 100;
      label = progress.analysis_ok
        ? `✓ 全部完成 · 抽出 ${progress.extracted} · 跳过 ${progress.skipped} · 失败 ${progress.failed} · 用时 ${(progress.elapsed_ms / 1000).toFixed(1)} s`
        : `⚠️ 文档处理完成,但全案分析失败(画像未更新):${progress.analysis_error ?? "未知原因"} — 可在案件里点「重新分析」重试`;
      break;
    case "error":
      percent = 0;
      label = `❌ 抽取失败:${progress.error}`;
      break;
  }

  const done = progress.stage === "completed";
  const analysisFailed = done && !progress.analysis_ok;
  const errored = progress.stage === "error";
  const currentIndex =
    progress.stage === "doc_finished"
      ? progress.completed_count
      : progress.stage === "doc_started"
        ? null
        : 0;
  const totalCount =
    progress.stage === "doc_started" ||
    progress.stage === "doc_finished" ||
    progress.stage === "started" ||
    progress.stage === "completed"
      ? progress.total
      : 0;

  // 最小化:右下角小卡片,只显示 N/M 进度 + 百分比
  if (minimized && !errored) {
    return (
      <div className="pointer-events-auto fixed bottom-4 right-4 z-40 animate-in fade-in-0 zoom-in-90 duration-200">
        <button
          type="button"
          onClick={onToggleMinimize}
          className={cn(
            "flex items-center gap-2 rounded-full border px-3 py-2 shadow-lg backdrop-blur transition-colors",
            done && !analysisFailed
              ? "border-emerald-200/70 bg-emerald-50/95 text-emerald-800 hover:bg-emerald-100"
              : analysisFailed
                ? "border-amber-300/70 bg-amber-50/95 text-amber-800 hover:bg-amber-100"
                : "border-border bg-card/95 hover:bg-muted",
          )}
          title="点击展开进度条"
        >
          {!done && <Loader2 className="size-3.5 animate-spin" />}
          <span className="font-mono text-xs font-medium">
            {done
              ? analysisFailed
                ? "⚠"
                : "✓"
              : progress.stage === "analyzing"
                ? "分析中"
                : `${currentIndex ?? "…"}/${totalCount}`}
          </span>
          <span className="font-mono text-caption text-muted-foreground">
            {percent}%
          </span>
        </button>
      </div>
    );
  }

  return (
    <div className="pointer-events-none fixed inset-x-0 top-0 z-40 flex justify-center pt-3 px-4 animate-in fade-in-0 duration-300">
      <div
        className={cn(
          "pointer-events-auto w-full max-w-3xl rounded-xl border px-4 py-3 shadow-lg backdrop-blur",
          done && !analysisFailed
            ? "border-emerald-200/70 bg-emerald-50/95"
            : analysisFailed
              ? "border-amber-300/70 bg-amber-50/95"
              : errored
                ? "border-destructive/50 bg-destructive/5"
                : "border-border bg-card/95",
        )}
      >
        {/* 顶行:状态 + 百分比 */}
        <div className="flex items-center gap-2 text-xs">
          {!done && !errored && (
            <Loader2 className="size-3.5 animate-spin text-foreground shrink-0" />
          )}
          <span
            className={cn(
              "flex-1 font-medium",
              analysisFailed && "text-amber-800",
              done && !analysisFailed && "text-emerald-800",
              errored && "text-destructive",
              !done && !errored && "text-foreground truncate",
            )}
          >
            {label}
          </span>
          {!errored && (
            <span className="shrink-0 font-mono text-muted-foreground">
              {percent}%
            </span>
          )}
          {/* 最小化 / 关闭按钮 */}
          <div className="ml-1 flex shrink-0 items-center gap-0.5">
            {!errored && !done && (
              <button
                type="button"
                onClick={onToggleMinimize}
                className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
                title="最小化到右下角"
              >
                <svg className="size-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                  <path d="M5 12h14"/>
                </svg>
              </button>
            )}
            <button
              type="button"
              onClick={onClose}
              className="rounded p-0.5 text-muted-foreground hover:bg-accent hover:text-foreground"
              title="关闭"
            >
              <svg className="size-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                <path d="M6 6l12 12M6 18L18 6"/>
              </svg>
            </button>
          </div>
        </div>

        {/* 当前文件 */}
        {filename && (
          <div className="mt-1.5 truncate text-label text-muted-foreground">
            📄 {filename}
          </div>
        )}

        {/* 云端 OCR 轮询子状态(大图扫描件排队/识别中,治"看着卡死";不影响百分比) */}
        {!done && !errored && ocrSub && (
          <div className="mt-1 truncate rounded bg-sky-50 px-2 py-1 text-label text-sky-700 dark:bg-sky-950/40 dark:text-sky-300">
            {ocrSubLabel(ocrSub)}
          </div>
        )}

        {/* 后端标签 */}
        {(ocrProvider || llmProvider) && (
          <div className="mt-2 flex flex-wrap gap-1.5">
            {ocrProvider && (
              <BackendChip type="OCR" provider={ocrProvider} />
            )}
            {llmProvider && (
              <BackendChip type="LLM" provider={llmProvider} />
            )}
          </div>
        )}

        {/* 进度条 */}
        <div className="mt-2 h-1 overflow-hidden rounded-full bg-muted">
          <div
            className={cn(
              "h-full transition-all duration-300",
              done && !analysisFailed
                ? "bg-emerald-500"
                : analysisFailed
                  ? "bg-amber-500"
                  : errored
                    ? "bg-destructive"
                    : "bg-foreground",
            )}
            style={{ width: `${percent}%` }}
          />
        </div>
      </div>
    </div>
  );
}

function BackendChip({
  type,
  provider,
}: {
  type: "OCR" | "LLM";
  provider: "local" | "cloud";
}) {
  const isLocal = provider === "local";
  const label =
    type === "OCR"
      ? isLocal
        ? "🖥️ 本机 MiniCPM-V"
        : "☁️ 云端 MinerU"
      : isLocal
        ? "🖥️ 本机 MiniCPM-V"
        : "☁️ 云端 AI";
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-caption font-medium",
        isLocal
          ? "bg-blue-100 text-blue-900 dark:bg-blue-900/30 dark:text-blue-100"
          : "bg-amber-100 text-amber-900 dark:bg-amber-900/30 dark:text-amber-100",
      )}
    >
      <span className="font-mono text-[9px]">{type}</span>
      <span>{label}</span>
    </span>
  );
}
