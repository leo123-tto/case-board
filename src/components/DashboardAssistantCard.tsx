import { useRef, useState } from "react";
import { Bot, Loader2, MessageCircle, Send } from "lucide-react";

import { HomeCompanionStrip, type DailyBrief } from "@/components/HomeCompanionStrip";
import { Button } from "@/components/ui/button";
import {
  chatDashboardAssistant,
  type DashboardAssistantMessage,
} from "@/lib/api";
import { openFeedback } from "@/lib/feedbackLauncher";
import { cn } from "@/lib/utils";

interface ChatItem extends DashboardAssistantMessage {
  id: string;
  action?: "none" | "open_feedback" | string;
  feedbackDraft?: string | null;
}

const QUICK_PROMPTS = ["介绍主要功能", "怎么导入案件", "我想反馈问题"];

export function DashboardAssistantCard({
  greeting,
  monthLabel,
  openCaseCount,
  displayName,
  activeCaseCount,
  reminderSummaries,
  dailyBrief,
  onDailyBriefAction,
}: {
  greeting: string;
  monthLabel: string;
  openCaseCount: number;
  displayName: string | null;
  activeCaseCount: number;
  reminderSummaries: string[];
  dailyBrief?: DailyBrief | null;
  onDailyBriefAction?: () => void;
}) {
  const [messages, setMessages] = useState<ChatItem[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const send = async (prompt = input) => {
    const text = prompt.trim();
    if (!text || sending) return;

    const userMessage: ChatItem = {
      id: crypto.randomUUID(),
      role: "user",
      content: text,
    };
    const history: DashboardAssistantMessage[] = [...messages, userMessage]
      .slice(-8)
      .map(({ role, content }) => ({ role, content }));
    setMessages((prev) => [...prev, userMessage]);
    setInput("");
    setSending(true);

    try {
      const response = await chatDashboardAssistant({
        messages: history,
        active_case_count: activeCaseCount,
      });
      setMessages((prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content: response.reply,
          action: response.action,
          feedbackDraft: response.feedback_draft,
        },
      ]);
    } catch {
      setMessages((prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content: "这次没有连上看板助手。你仍可以直接点顶部“反馈”，或稍后重试。",
        },
      ]);
    } finally {
      setSending(false);
      window.setTimeout(() => inputRef.current?.focus(), 0);
    }
  };

  return (
    <section
      className="relative overflow-hidden rounded-xl border border-brand/15 bg-brand-soft/55 p-5 shadow-[inset_0_1px_0_oklch(1_0_0/0.72)]"
      aria-label="看板助手"
    >
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="font-mono text-caption uppercase tracking-wider text-brand">
            看板助手 · {monthLabel}
          </p>
          <h1 className="mt-2 text-3xl font-semibold tracking-tight text-foreground xl:text-4xl">
            {greeting}
          </h1>
          <p className="mt-2 text-sm text-muted-foreground">
            你正在办 {openCaseCount} 个案件，可以问我软件怎么用。
          </p>
        </div>
        <button
          type="button"
          onClick={() => inputRef.current?.focus()}
          className="hidden size-9 shrink-0 items-center justify-center rounded-lg border border-brand/15 bg-background/75 text-brand transition hover:bg-background md:flex"
          title="和看板助手聊聊"
        >
          <Bot className="size-4" />
          <span className="sr-only">和看板助手聊聊</span>
        </button>
      </div>

      <div>
        <HomeCompanionStrip
          displayName={displayName}
          activeCaseCount={activeCaseCount}
          reminderSummaries={reminderSummaries}
          dailyBrief={dailyBrief}
          onDailyBriefAction={onDailyBriefAction}
        />
      </div>

      {messages.length > 0 && (
        <div
          className="mt-3 max-h-52 space-y-2 overflow-y-auto rounded-lg border border-brand/10 bg-background/55 p-2.5"
          aria-live="polite"
        >
          {messages.map((message) => (
            <div
              key={message.id}
              className={cn(
                "max-w-[92%] rounded-lg px-3 py-2 text-xs leading-relaxed",
                message.role === "user"
                  ? "ml-auto bg-foreground text-background"
                  : "border border-border bg-card text-foreground",
              )}
            >
              <p className="whitespace-pre-wrap">{message.content}</p>
              {message.role === "assistant" && message.action === "open_feedback" && (
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  className="mt-2 h-7 bg-background text-xs"
                  onClick={() => openFeedback(message.feedbackDraft ?? message.content)}
                >
                  <MessageCircle className="size-3.5" />
                  打开反馈并带入草稿
                </Button>
              )}
            </div>
          ))}
          {sending && (
            <div className="flex items-center gap-2 px-2 py-1 text-xs text-muted-foreground">
              <Loader2 className="size-3.5 animate-spin" />
              看板助手正在整理…
            </div>
          )}
        </div>
      )}

      {messages.length === 0 && (
        <div className="mt-3 flex flex-wrap gap-1.5">
          {QUICK_PROMPTS.map((prompt) => (
            <button
              key={prompt}
              type="button"
              onClick={() => void send(prompt)}
              className="rounded-md border border-brand/15 bg-background/65 px-2 py-1 text-[11px] text-muted-foreground transition hover:bg-background hover:text-foreground"
            >
              {prompt}
            </button>
          ))}
        </div>
      )}

      <div className="mt-3 flex items-end gap-2 rounded-lg border border-brand/15 bg-background/80 p-2 focus-within:border-brand/35 focus-within:ring-1 focus-within:ring-brand/10">
        <textarea
          ref={inputRef}
          rows={1}
          value={input}
          onChange={(event) => setInput(event.target.value.slice(0, 1_200))}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void send();
            }
          }}
          placeholder="问功能、用法、设置，或让我整理反馈…"
          aria-label="给看板助手发消息"
          className="min-h-7 flex-1 resize-none bg-transparent px-1 py-1 text-sm text-foreground outline-none placeholder:text-muted-foreground/60"
        />
        <button
          type="button"
          onClick={() => void send()}
          disabled={!input.trim() || sending}
          className="flex size-8 shrink-0 items-center justify-center rounded-md bg-foreground text-background transition hover:bg-foreground/90 disabled:cursor-not-allowed disabled:opacity-35"
          title="发送"
        >
          {sending ? <Loader2 className="size-3.5 animate-spin" /> : <Send className="size-3.5" />}
          <span className="sr-only">发送</span>
        </button>
      </div>
      <p className="mt-1.5 text-[10px] text-muted-foreground/70">
        产品答疑与简单聊天；具体案情请进入案件内 AI 助手。
      </p>
    </section>
  );
}
