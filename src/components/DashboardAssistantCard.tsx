import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Bot, FilePenLine, Loader2, MessageCircle, Send, Trash2, X } from "lucide-react";

import { HomeCompanionStrip, type DailyBrief } from "@/components/HomeCompanionStrip";
import { HomeFeatureCard, HomeFeatureCardScrollArea } from "@/components/HomeFeatureCard";
import { Button } from "@/components/ui/button";
import {
  chatDashboardAssistant,
  type DashboardAssistantContext,
  type DashboardAssistantMessage,
} from "@/lib/api";
import { openFeedback } from "@/lib/feedbackLauncher";
import { cn } from "@/lib/utils";

interface ChatItem extends DashboardAssistantMessage {
  id: string;
  action?: "none" | "open_feedback" | string;
  feedbackDraft?: string | null;
  references?: string[];
  dataSources?: string[];
}

const QUICK_PROMPTS = ["介绍主要功能", "怎么导入案件", "我想反馈问题"];

export function DashboardAssistantCard({
  greeting,
  monthLabel,
  openCaseCount,
  displayName,
  activeCaseCount,
  assistantContext,
  reminderSummaries,
  dailyBrief,
  onDailyBriefAction,
  onOpenAiWorkspace,
}: {
  greeting: string;
  monthLabel: string;
  openCaseCount: number;
  displayName: string | null;
  activeCaseCount: number;
  assistantContext: DashboardAssistantContext;
  reminderSummaries: string[];
  dailyBrief?: DailyBrief | null;
  onDailyBriefAction?: () => void;
  onOpenAiWorkspace: () => void;
}) {
  const [messages, setMessages] = useState<ChatItem[]>([]);
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const [chatOpen, setChatOpen] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const sessionVersionRef = useRef(0);

  const closeChat = () => {
    sessionVersionRef.current += 1;
    setChatOpen(false);
    setMessages([]);
    setInput("");
    setSending(false);
  };

  const clearChat = () => {
    sessionVersionRef.current += 1;
    setMessages([]);
    setInput("");
    setSending(false);
    window.setTimeout(() => inputRef.current?.focus(), 0);
  };

  useEffect(() => {
    if (!chatOpen) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeChat();
    };
    window.addEventListener("keydown", onKeyDown);
    window.setTimeout(() => inputRef.current?.focus(), 0);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [chatOpen]);

  useEffect(() => {
    if (!chatOpen) return;
    messagesEndRef.current?.scrollIntoView({ block: "nearest" });
  }, [chatOpen, messages, sending]);

  const send = async (prompt = input) => {
    const text = prompt.trim();
    if (!text || sending) return;
    const sessionVersion = sessionVersionRef.current;

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
        context: assistantContext,
      });
      if (sessionVersionRef.current !== sessionVersion) return;
      setMessages((prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content: response.reply,
          action: response.action,
          feedbackDraft: response.feedback_draft,
          references: response.references,
          dataSources: response.data_sources,
        },
      ]);
    } catch {
      if (sessionVersionRef.current !== sessionVersion) return;
      setMessages((prev) => [
        ...prev,
        {
          id: crypto.randomUUID(),
          role: "assistant",
          content: "这次没有连上看板助手。你仍可以直接点顶部“反馈”，或稍后重试。",
        },
      ]);
    } finally {
      if (sessionVersionRef.current !== sessionVersion) return;
      setSending(false);
      window.setTimeout(() => inputRef.current?.focus(), 0);
    }
  };

  return (
    <HomeFeatureCard tone="brand" aria-label="看板助手">
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
        <div className="flex shrink-0 items-center gap-2">
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="border-brand/15 bg-background/75 text-brand hover:bg-background hover:text-brand"
            onClick={onOpenAiWorkspace}
          >
            <FilePenLine className="size-3.5" />
            写材料
          </Button>
          <button
            type="button"
            onClick={() => setChatOpen(true)}
            className="hidden size-9 shrink-0 items-center justify-center rounded-lg border border-brand/15 bg-background/75 text-brand transition hover:bg-background md:flex"
            title="和看板助手聊聊"
          >
            <Bot className="size-4" />
            <span className="sr-only">和看板助手聊聊</span>
          </button>
        </div>
      </div>

      <HomeFeatureCardScrollArea className="mt-2 pr-1">
        <HomeCompanionStrip
          displayName={displayName}
          activeCaseCount={activeCaseCount}
          reminderSummaries={reminderSummaries}
          dailyBrief={dailyBrief}
          onDailyBriefAction={onDailyBriefAction}
        />
      </HomeFeatureCardScrollArea>

      <div className="mt-3 flex items-center justify-between gap-3 border-t border-brand/10 pt-3">
        <p className="text-xs text-muted-foreground">
          产品任何问题和 Bug 反馈，都可以使用看板助手哦。
        </p>
        <Button
          type="button"
          size="sm"
          className="shrink-0"
          onClick={() => setChatOpen(true)}
        >
          <MessageCircle className="size-3.5" />
          打开助手
        </Button>
      </div>

      {chatOpen &&
        createPortal(
          <div
            className="fixed inset-0 z-[120] flex items-center justify-center bg-foreground/25 p-4 backdrop-blur-sm animate-in fade-in-0 duration-200 sm:p-8"
            role="presentation"
            onMouseDown={(event) => {
              if (event.target === event.currentTarget) closeChat();
            }}
          >
            <section
              className="flex h-[min(680px,calc(100dvh-4rem))] w-full max-w-2xl flex-col overflow-hidden rounded-xl border border-border bg-background shadow-2xl"
              role="dialog"
              aria-modal="true"
              aria-labelledby="dashboard-assistant-title"
            >
              <header className="flex items-center justify-between gap-3 border-b px-4 py-3">
                <div className="flex min-w-0 items-center gap-2.5">
                  <div className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-brand-soft text-brand">
                    <Bot className="size-4" />
                  </div>
                  <div className="min-w-0">
                    <h2
                      id="dashboard-assistant-title"
                      className="text-sm font-semibold text-foreground"
                    >
                      看板助手
                    </h2>
                    <p className="truncate text-[11px] text-muted-foreground">
                      软件功能、用法、设置与反馈整理
                    </p>
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  {messages.length > 0 && (
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      className="h-8 text-xs"
                      onClick={clearChat}
                    >
                      <Trash2 className="size-3.5" />
                      清空对话
                    </Button>
                  )}
                  <button
                    type="button"
                    onClick={closeChat}
                    className="flex size-8 items-center justify-center rounded-md text-muted-foreground transition hover:bg-muted hover:text-foreground"
                    title="关闭看板助手"
                  >
                    <X className="size-4" />
                    <span className="sr-only">关闭看板助手</span>
                  </button>
                </div>
              </header>

              <div
                className="min-h-0 flex-1 overflow-y-auto bg-muted/25 px-4 py-4"
                aria-live="polite"
              >
                {messages.length === 0 ? (
                  <div className="flex h-full min-h-56 flex-col items-center justify-center text-center">
                    <div className="flex size-11 items-center justify-center rounded-xl border border-brand/10 bg-brand-soft text-brand">
                      <Bot className="size-5" />
                    </div>
                    <p className="mt-3 text-sm font-medium text-foreground">有什么可以帮你？</p>
                    <p className="mt-1 max-w-sm text-xs leading-relaxed text-muted-foreground">
                      可以询问 CaseBoard 的功能与设置，也可以让我把问题整理成反馈草稿。
                    </p>
                    <div className="mt-4 flex flex-wrap justify-center gap-2">
                      {QUICK_PROMPTS.map((prompt) => (
                        <button
                          key={prompt}
                          type="button"
                          onClick={() => void send(prompt)}
                          className="rounded-md border border-border bg-background px-3 py-1.5 text-xs text-muted-foreground transition hover:border-brand/25 hover:text-foreground"
                        >
                          {prompt}
                        </button>
                      ))}
                    </div>
                  </div>
                ) : (
                  <div className="space-y-3">
                    {messages.map((message) => (
                      <div
                        key={message.id}
                        className={cn(
                          "max-w-[88%] rounded-lg px-3.5 py-2.5 text-sm leading-relaxed",
                          message.role === "user"
                            ? "ml-auto bg-foreground text-background"
                            : "border border-border bg-background text-foreground shadow-sm",
                        )}
                      >
                        <p className="whitespace-pre-wrap">{message.content}</p>
                        {message.role === "assistant" &&
                          ((message.references?.length ?? 0) > 0 ||
                            (message.dataSources?.length ?? 0) > 0) && (
                            <p className="mt-2 border-t border-border/70 pt-1.5 text-[10px] leading-relaxed text-muted-foreground">
                              {message.references?.length
                                ? `依据：产品说明书 · ${message.references.join("、")}`
                                : ""}
                              {message.dataSources?.length
                                ? `${message.references?.length ? "；" : "依据："}首页数据 · ${message.dataSources.join("、")}`
                                : ""}
                            </p>
                          )}
                        {message.role === "assistant" &&
                          message.action === "open_feedback" && (
                            <Button
                              type="button"
                              size="sm"
                              variant="outline"
                              className="mt-2 h-7 bg-background text-xs"
                              onClick={() =>
                                openFeedback(message.feedbackDraft ?? message.content)
                              }
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
                <div ref={messagesEndRef} />
              </div>

              <footer className="border-t bg-background p-3">
                <div className="flex items-end gap-2 rounded-lg border border-border bg-background p-2 focus-within:border-brand/35 focus-within:ring-1 focus-within:ring-brand/10">
                  <textarea
                    ref={inputRef}
                    rows={1}
                    value={input}
                    onChange={(event) => setInput(event.target.value.slice(0, 1_200))}
                    onKeyDown={(event) => {
                      if (
                        event.key === "Enter" &&
                        !event.shiftKey &&
                        !event.nativeEvent.isComposing
                      ) {
                        event.preventDefault();
                        void send();
                      }
                    }}
                    placeholder="问功能、用法、设置，或让我整理反馈…"
                    aria-label="给看板助手发消息"
                    className="max-h-28 min-h-8 flex-1 resize-none bg-transparent px-1 py-1.5 text-sm text-foreground outline-none placeholder:text-muted-foreground/60"
                  />
                  <button
                    type="button"
                    onClick={() => void send()}
                    disabled={!input.trim() || sending}
                    className="flex size-9 shrink-0 items-center justify-center rounded-md bg-foreground text-background transition hover:bg-foreground/90 disabled:cursor-not-allowed disabled:opacity-35"
                    title="发送"
                  >
                    {sending ? (
                      <Loader2 className="size-4 animate-spin" />
                    ) : (
                      <Send className="size-4" />
                    )}
                    <span className="sr-only">发送</span>
                  </button>
                </div>
                <p className="mt-1.5 px-1 text-[10px] text-muted-foreground/70">
                  关闭窗口会清空本次对话；具体案情请进入案件内 AI 助手。
                </p>
              </footer>
            </section>
          </div>,
          document.body,
        )}
    </HomeFeatureCard>
  );
}
