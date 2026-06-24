/**
 * 案件 AI 助手独立路由 fallback。
 *
 * Windows WebView2 上独立 webview 会稳定复现空白/未响应,主入口已改成
 * CaseChatPanel 内的主窗口全屏覆盖层。本组件只保留给旧链接/旧命令兜底。
 */
import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { X } from "lucide-react";

export function DetachedChatWindow({
  caseId,
  caseName,
  domain: _domain,
}: {
  caseId: string | null;
  caseName?: string | null;
  domain?: "civil" | "criminal";
}) {
  useEffect(() => {
    document.title = caseName
      ? `案件 AI 助手 · ${caseName}（请改用主窗口全屏模式）`
      : "案件 AI 助手（请改用主窗口全屏模式）";
  }, [caseName]);

  return (
    <div
      data-testid="detached-chat-fallback"
      className="flex h-screen w-screen flex-col items-center justify-center gap-4 bg-slate-50 p-8 text-center text-slate-700 dark:bg-slate-950 dark:text-slate-200"
    >
      <div className="rounded-full bg-amber-100 p-3 text-amber-700 dark:bg-amber-950/40 dark:text-amber-400">
        <X className="size-6" />
      </div>
      <h1 className="text-xl font-semibold">独立 AI 助手窗口暂不可用</h1>
      <p className="max-w-md text-sm leading-relaxed text-slate-600 dark:text-slate-400">
        当前平台上独立窗口存在渲染兼容问题。请关闭此窗口，回到主窗口使用
        AI 助手右上角的
        <span className="mx-1 inline-flex items-center rounded border border-slate-300 px-1.5 py-0.5 align-middle font-mono text-xs dark:border-slate-700">
          ↗ 全屏模式
        </span>
        ，聊天记录和功能不变。
      </p>
      {caseName && (
        <p className="text-xs text-slate-500 dark:text-slate-500">
          案件：{caseName}
          {caseId && <span className="ml-2 font-mono opacity-60">({caseId})</span>}
        </p>
      )}
      <button
        type="button"
        onClick={() => getCurrentWindow().close()}
        className="mt-2 inline-flex items-center gap-1.5 rounded-md bg-blue-600 px-4 py-2 text-sm font-medium text-white shadow-sm transition-colors hover:bg-blue-700"
      >
        <X className="size-4" />
        关闭此窗口
      </button>
    </div>
  );
}
