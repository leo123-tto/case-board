import { useCallback, useEffect, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { BookOpen, Copy, Loader2, Sparkles, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  getLocalKbGuide,
  installLocalKbAiEntry,
  type LocalKbAiEntry,
} from "@/lib/api";

export function KbGuideCard({
  aiMaintenanceEnabled = false,
  onAiMaintenanceChange,
}: {
  aiMaintenanceEnabled?: boolean;
  onAiMaintenanceChange?: (enabled: boolean) => void;
}) {
  const [open, setOpen] = useState(false);
  const [content, setContent] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [entry, setEntry] = useState<LocalKbAiEntry | null>(null);
  const [installing, setInstalling] = useState(false);
  const [entryError, setEntryError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  const showGuide = useCallback(async () => {
    setOpen(true);
    setError(null);
    try {
      setContent(await getLocalKbGuide());
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const installAiEntry = useCallback(async () => {
    setInstalling(true);
    setEntryError(null);
    setCopied(false);
    try {
      setEntry(await installLocalKbAiEntry());
    } catch (e) {
      setEntryError(String(e));
    } finally {
      setInstalling(false);
    }
  }, []);

  const copyAiInstruction = useCallback(async () => {
    if (!entry) return;
    try {
      await navigator.clipboard.writeText(entry.instruction);
      setCopied(true);
    } catch (e) {
      setEntryError(`复制失败：${String(e)}`);
    }
  }, [entry]);

  useEffect(() => {
    if (!open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [open]);

  return (
    <section>
      <div className="mb-3">
        <h3 className="text-sm font-semibold text-foreground">检索与维护说明</h3>
        <p className="mt-0.5 text-xs text-muted-foreground">
          说明目录怎么放、各层怎么检索、元典材料如何写回；设置页与 AI 读取同一版本。
        </p>
      </div>
      <div className="flex items-center justify-between gap-4 rounded-lg border border-border bg-background/50 p-4">
        <p className="text-xs leading-relaxed text-muted-foreground">
          Wiki 是导航层，raw 是正文层；只有符合目录职责的材料才会进入对应检索流程。
        </p>
        <Button type="button" size="sm" variant="outline" onClick={showGuide} className="shrink-0">
          <BookOpen className="size-3.5" />
          查看完整说明
        </Button>
      </div>

      <div className="mt-3 rounded-lg border border-border bg-background/50 p-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <p className="text-xs font-medium text-foreground">给外部 AI 的本机入口</p>
            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
              为 Codex、Claude Code、WorkBuddy 等创建统一说明、机器清单及 AGENTS/CLAUDE 引导；AI 获得该知识库目录权限后可自行读取，无需复制整篇说明。
            </p>
          </div>
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => void installAiEntry()}
            disabled={installing}
          >
            {installing ? <Loader2 className="size-3.5 animate-spin" /> : <Sparkles className="size-3.5" />}
            生成或更新 AI 入口
          </Button>
        </div>
        {entry && (
          <div className="mt-3 rounded-md border border-border bg-muted/30 px-3 py-2.5">
            <p className="break-all font-mono text-[11px] text-foreground">{entry.guide_path}</p>
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <Button type="button" size="sm" onClick={() => void copyAiInstruction()}>
                <Copy className="size-3.5" />
                {copied ? "已复制" : "复制给外部 AI"}
              </Button>
              <span className="text-[11px] text-muted-foreground">
                把这句发给外部 AI，并授权它访问知识库目录即可。
              </span>
            </div>
          </div>
        )}
        {entryError && <p className="mt-2 text-xs text-red-600">{entryError}</p>}
      </div>

      <label className="mt-3 flex cursor-pointer items-start justify-between gap-4 rounded-lg border border-border bg-background/50 p-4">
        <span>
          <span className="block text-xs font-medium text-foreground">允许内置 AI 新增 L1 raw 材料</span>
          <span className="mt-1 block text-xs leading-relaxed text-muted-foreground">
            默认关闭。开启后 Native 与 Pi 只能新建 raw/notes 材料，不能覆盖、删除、移动或直接写 Wiki。
          </span>
        </span>
        <input
          type="checkbox"
          aria-label="允许内置 AI 新增 L1 raw 材料"
          checked={aiMaintenanceEnabled}
          onChange={(event) => onAiMaintenanceChange?.(event.target.checked)}
          className="mt-0.5 size-4 shrink-0 accent-sky-600"
        />
      </label>

      {open && (
        <div
          className="fixed inset-0 z-[70] flex items-center justify-center bg-foreground/20 px-4 py-8 backdrop-blur-sm"
          onClick={() => setOpen(false)}
        >
          <div
            className="flex h-full max-h-[86vh] w-full max-w-4xl flex-col overflow-hidden rounded-xl border border-border bg-card shadow-2xl"
            onClick={(event) => event.stopPropagation()}
          >
            <header className="flex items-center justify-between border-b border-border px-5 py-3.5">
              <div>
                <h2 className="text-sm font-semibold text-foreground">本地知识库检索与维护说明</h2>
                <p className="mt-0.5 text-caption text-muted-foreground">人和 CaseBoard AI 共用的规则</p>
              </div>
              <Button type="button" size="sm" variant="ghost" onClick={() => setOpen(false)}>
                <X className="size-4" />
                关闭
              </Button>
            </header>
            <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
              {!content && !error && (
                <p className="text-sm text-muted-foreground">
                  <Loader2 className="mr-1.5 inline size-4 animate-spin" />
                  正在读取当前知识库规则…
                </p>
              )}
              {error && <p className="text-sm text-red-600">读取失败：{error}</p>}
              {content && (
                <article className="prose prose-sm max-w-none text-foreground prose-headings:scroll-mt-4 prose-table:text-xs">
                  <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>
                </article>
              )}
            </div>
          </div>
        </div>
      )}
    </section>
  );
}
