import { useCallback, useEffect, useState } from "react";
import {
  ChevronDown,
  ChevronUp,
  Clock3,
  ExternalLink,
  Loader2,
  Save,
  Sparkles,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import {
  createCaseLog,
  listCaseLogs,
  openInDefaultApp,
  organizeCaseLog,
} from "@/lib/api";
import type { CaseLog } from "@/lib/types";

function nowParts() {
  const now = new Date();
  const local = new Date(now.getTime() - now.getTimezoneOffset() * 60_000)
    .toISOString();
  return { date: local.slice(0, 10), time: local.slice(11, 16) };
}

export function CaseWorkLogSection({
  caseId,
  onSaved,
}: {
  caseId: string;
  onSaved?: () => void;
}) {
  const initial = nowParts();
  const [date, setDate] = useState(initial.date);
  const [time, setTime] = useState(initial.time);
  const [input, setInput] = useState("");
  const [organized, setOrganized] = useState<string | null>(null);
  const [logs, setLogs] = useState<CaseLog[]>([]);
  const [busy, setBusy] = useState<"save" | "ai" | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);

  const reload = useCallback(() => {
    listCaseLogs(caseId).then(setLogs).catch(() => setLogs([]));
  }, [caseId]);
  useEffect(reload, [reload]);

  async function save(useOrganized: boolean) {
    if (!input.trim() || busy) return;
    setBusy("save");
    try {
      await createCaseLog(
        caseId,
        `${date}T${time}`,
        input,
        useOrganized ? organized : null,
      );
      setInput("");
      setOrganized(null);
      reload();
      onSaved?.();
      toast("工作记录已保存", "success");
    } catch (error) {
      toast(`保存失败:${error}`, "error");
    } finally {
      setBusy(null);
    }
  }

  async function organize() {
    if (!input.trim() || busy) return;
    setBusy("ai");
    try {
      setOrganized(await organizeCaseLog(caseId, input));
    } catch (error) {
      toast(`${error} · 原始输入仍保留，可直接保存`, "error", 8000);
    } finally {
      setBusy(null);
    }
  }

  async function openLog(log: CaseLog) {
    if (!log.source_path) {
      toast("这条记录没有可打开的原文文件", "info");
      return;
    }
    try {
      await openInDefaultApp(log.source_path);
    } catch (error) {
      toast(`打开工作记录失败:${error}`, "error");
    }
  }

  return (
    <section className="rounded-xl border border-border bg-card p-4">
      <div className="mb-3 flex items-center justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold">工作记录</h2>
          <p className="mt-0.5 text-xs text-muted-foreground">
            随手记下无书面材料的办案进展
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Clock3 className="size-4 text-muted-foreground" />
        </div>
      </div>
      <div className="grid items-center gap-2 md:grid-cols-[140px_110px_minmax(0,1fr)_auto_auto]">
        <input
          type="date"
          value={date}
          onChange={(event) => setDate(event.target.value)}
          className="h-9 rounded-md border border-border bg-background px-2 text-xs"
        />
        <input
          type="time"
          value={time}
          onChange={(event) => setTime(event.target.value)}
          className="h-9 rounded-md border border-border bg-background px-2 text-xs"
        />
        <input
          value={input}
          onChange={(event) => {
            setInput(event.target.value);
            setOrganized(null);
          }}
          placeholder="记录沟通、会见等办案进展…"
          className="h-9 min-w-0 rounded-md border border-border bg-background px-3 text-sm"
        />
        <Button
          variant="outline"
          size="sm"
          className="h-9"
          disabled={!input.trim() || !!busy}
          onClick={() => void save(false)}
        >
          {busy === "save" ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <Save className="size-3.5" />
          )}
          保存
        </Button>
        <Button
          size="sm"
          className="h-9"
          disabled={!input.trim() || !!busy}
          onClick={() => void organize()}
        >
          {busy === "ai" ? (
            <Loader2 className="size-3.5 animate-spin" />
          ) : (
            <Sparkles className="size-3.5" />
          )}
          AI 整理
        </Button>
      </div>
      {organized && (
        <div className="mt-3 rounded-lg border border-border bg-muted/30 p-3">
          <div className="mb-2 text-xs font-medium">AI 整理预览（可修改）</div>
          <textarea
            value={organized}
            onChange={(event) => setOrganized(event.target.value)}
            className="h-44 w-full resize-none rounded-md border border-border bg-background px-3 py-2 font-mono text-xs leading-5"
          />
          <div className="mt-2 flex justify-end">
            <Button size="sm" onClick={() => void save(true)} disabled={!!busy}>
              确认并保存
            </Button>
          </div>
        </div>
      )}
      <div className="mt-3 border-t border-border pt-3">
        <div className="mb-2 flex items-center justify-between gap-3">
          <p className="text-xs font-medium text-muted-foreground">已保存记录</p>
          {logs.length > 0 && (
            <span className="text-[11px] text-muted-foreground">
              共 {logs.length} 条，可展开查看完整内容
            </span>
          )}
        </div>
        {logs.length === 0 ? (
          <p className="text-center text-xs text-muted-foreground">暂无工作记录</p>
        ) : (
          <div className="max-h-72 space-y-2 overflow-y-auto pr-1">
            {logs.map((log) => {
              const expanded = expandedId === log.id;
              const text = displayLogContent(log.content);
              return (
                <div key={log.id} className="rounded-md bg-muted/30 px-3 py-2">
                  <div className="flex items-center justify-between gap-3">
                    <div className="text-[11px] text-muted-foreground">
                      {log.occurred_at.replace("T", " ")} ·{" "}
                      {log.source === "ai" ? "AI整理" : "直接记录"}
                    </div>
                    <div className="flex shrink-0 items-center gap-1">
                      <button
                        type="button"
                        className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground"
                        onClick={() => setExpandedId(expanded ? null : log.id)}
                      >
                        {expanded ? (
                          <ChevronUp className="size-3" />
                        ) : (
                          <ChevronDown className="size-3" />
                        )}
                        {expanded ? "收起" : "查看"}
                      </button>
                      <button
                        type="button"
                        className="inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[11px] text-muted-foreground hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
                        disabled={!log.source_path}
                        onClick={() => void openLog(log)}
                      >
                        <ExternalLink className="size-3" />
                        原文
                      </button>
                    </div>
                  </div>
                  <p
                    className={
                      expanded
                        ? "mt-1 whitespace-pre-wrap text-xs leading-5"
                        : "mt-1 line-clamp-2 whitespace-pre-wrap text-xs leading-5"
                    }
                  >
                    {text || "（空记录）"}
                  </p>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
}

function displayLogContent(markdown: string): string {
  const normalized = markdown.replace(/\r\n/g, "\n");
  const organized = normalized.match(/## 整理内容\n\n([\s\S]*?)(?:\n\n## 原始记录|$)/);
  if (organized) return organized[1].trim();
  const direct = normalized.match(/## 记录内容\n\n([\s\S]*)$/);
  if (direct) return direct[1].trim();
  return normalized.replace(/^#.*$/gm, "").trim();
}
