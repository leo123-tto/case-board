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
  onStop,
  stopping,
}: {
  progress: ProgressEvent;
  /** 2026-06-14:当前文档云端 OCR 轮询子状态(独立于主进度,不影响百分比) */
  ocrSub?: DocOcrStatusEvent | null;
  minimized: boolean;
  onToggleMinimize: () => void;
  onClose: () => void;
  /**
   * 2026-07-28:停止本案材料处理。此前后端的取消能力只在删除案件时被调用,界面没有入口 ——
   * 用户拖进几百份材料后只能等它跑完或退出 App。已处理完的保留,剩下的不再启动。
   */
  onStop: (caseId: string) => void;
  stopping: boolean;
}) {
  let percent = 0;
  let label = "";
  let filename: string | null = null;
  let ocrProvider: "local" | "cloud" | null = null;
  let llmProvider: "local" | "cloud" | null = null;
  // 2026-06-16 V0.3.19 fix:具体模型名(MiniMax-M2.7 / deepseek-v4-flash / mineru / paddle)
  // 给 BackendChip 用,按 model 判定具体后端而不是写死 "云端 DeepSeek"。
  let ocrModel: string | undefined = undefined;
  let llmModel: string | undefined = undefined;

  switch (progress.stage) {
    case "started":
      percent = 0;
      label = `开始处理 ${progress.total} 份文档…`;
      ocrProvider = progress.ocr_provider;
      llmProvider = progress.llm_provider;
      // 2026-06-16 V0.3.19 fix:Started 事件有 llm_model 字段,BackendChip 按它判断具体后端。
      // OCR model 暂不传(后端 ProgressEvent 没 ocr_model 字段,OCR 后端默认按 settings.ocr_cloud_primary 显示)
      llmModel = progress.llm_model;
      break;
    case "doc_started":
      // 2026-05-24 i:并发场景下 index 不能算 percent(回退 bug),DocStarted 没 completed_count,
      // 这里不更新 percent(沿用上一个 DocFinished 的 percent),只更新 filename / providers
      // 2026-06-19:doc_started 也带 index/total,显示「第 i/N 份」整体进度 —— 否则首个大文件
      // OCR 1~3 分钟时只看到「处理中 · 文件名 0%」,以为卡死/不知道总共多少份(反馈)。
      label = `处理中 · 第 ${progress.index}/${progress.total} 份`;
      filename = progress.filename;
      ocrProvider = progress.ocr_provider;
      llmProvider = progress.llm_provider;
      // DocStarted 事件没 llm_model,沿用 Started 阶段设的 llmModel 变量
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
        ? `${progress.analysis_warning ? "⚠️ 已完成，需复核" : progress.failed > 0 ? "⚠️ 已完成，有失败材料" : "✓ 全部完成"} · 抽出 ${progress.extracted} · 跳过 ${progress.skipped} · 失败 ${progress.failed} · 用时 ${(progress.elapsed_ms / 1000).toFixed(1)} s${progress.analysis_warning ? `：${progress.analysis_warning}` : ""}`
        : `⚠️ 文档处理完成,但全案分析未完整完成:${progress.analysis_error ?? "未知原因"} — 可在案件里点「重新分析」重试`;
      break;
    case "error":
      percent = 0;
      label = `❌ 抽取失败:${progress.error}`;
      break;
  }

  const done = progress.stage === "completed";
  const analysisFailed = done && !progress.analysis_ok;
  const analysisWarning = done && progress.analysis_ok && Boolean(progress.analysis_warning);
  const documentFailures = done && progress.failed > 0;
  const needsAttention = analysisFailed || analysisWarning || documentFailures;
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
            done && !needsAttention
              ? "border-emerald-200/70 bg-emerald-50/95 text-emerald-800 hover:bg-emerald-100"
              : needsAttention
                ? "border-amber-300/70 bg-amber-50/95 text-amber-800 hover:bg-amber-100"
                : "border-border bg-card/95 hover:bg-muted",
          )}
          title="点击展开进度条"
        >
          {!done && <Loader2 className="size-3.5 animate-spin" />}
          <span className="font-mono text-xs font-medium">
            {done
              ? needsAttention
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
          done && !needsAttention
            ? "border-emerald-200/70 bg-emerald-50/95"
            : needsAttention
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
              needsAttention && "text-amber-800",
              done && !needsAttention && "text-emerald-800",
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
          {/* 停止 / 最小化 / 关闭按钮 */}
          <div className="ml-1 flex shrink-0 items-center gap-0.5">
            {!errored && !done && progress.stage !== "analyzing" && (
              <button
                type="button"
                onClick={() => onStop(progress.case_id)}
                disabled={stopping}
                title="停止处理剩余材料(已处理完的保留)。适合先停下来,把不需要识别的材料标为忽略再继续。"
                className={cn(
                  "mr-1 rounded border border-border bg-background px-1.5 py-0.5 text-caption text-muted-foreground transition-colors hover:bg-muted hover:text-foreground",
                  stopping && "cursor-wait opacity-60",
                )}
              >
                {stopping ? "停止中…" : "停止"}
              </button>
            )}
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
              <BackendChip type="OCR" provider={ocrProvider} model={ocrModel} />
            )}
            {llmProvider && (
              <BackendChip type="LLM" provider={llmProvider} model={llmModel} />
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
  model,
}: {
  type: "OCR" | "LLM";
  provider: "local" | "cloud";
  /** 2026-06-16 V0.3.19 fix:从 progress.llm_model 传进来,按模型名判定具体后端
   *  (DeepSeek / MiniMax),不再硬编码 "云端 DeepSeek"。
   *  OCR 后端目前有 2 家(MinerU / PaddleOCR VL),也按 model 字段匹配。 */
  model?: string;
}) {
  const isLocal = provider === "local";
  // 按 model 名前缀判定具体后端,fallback 到 provider 默认
  const lowerModel = (model ?? "").toLowerCase();
  let label: string;
  if (type === "OCR") {
    if (isLocal) {
      label = "🖥️ 本机 MiniCPM-V";
    } else if (!model) {
      // ProgressEvent 没 ocr_model 字段,不知道是 mineru 还是 paddle,显示通用文案
      label = "☁️ 云端 OCR";
    } else if (lowerModel.includes("paddle")) {
      label = "☁️ 云端 PaddleOCR VL";
    } else {
      label = "☁️ 云端 MinerU";
    }
  } else if (isLocal) {
    label = "🖥️ 本机 MiniCPM-V";
  } else if (lowerModel.includes("minimax") || lowerModel.startsWith("m")) {
    // M 系列 / M2 / M2.7 / M2.7-highspeed / M3 都归为 MiniMax
    label = "☁️ 云端 MiniMax";
  } else if (
    lowerModel.includes("deepseek") ||
    lowerModel.includes("v4-flash") ||
    lowerModel.includes("v4-pro")
  ) {
    label = "☁️ 云端 DeepSeek";
  } else {
    // 未知云端模型 → 显示实际 model 名,避免误标
    label = model ? `☁️ 云端 ${model}` : "☁️ 云端 LLM";
  }
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
