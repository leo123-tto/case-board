import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { AtSign, CircleStop, FilePenLine, FilePlus2, Loader2, Send, Sparkles } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { AskUserCard } from "@/components/AskUserCard";
import { getSettings } from "@/lib/api";
import { effectiveAgentRuntime, type AgentRuntime } from "@/lib/piRuntime";
import { cn } from "@/lib/utils";
import type { ChatActivity } from "@/lib/api";

import {
  archiveAiWorkspaceConversation,
  cancelAiWorkspaceChat,
  createAiWorkspaceConversation,
  createAiWorkspaceArtifactFromMessage,
  createAiWorkspaceDocumentProposal,
  ensureAiWorkspaceConversation,
  listAiWorkspaceConversations,
  listAiWorkspaceDocuments,
  listAiWorkspaceMessages,
  listAiWorkspaceTasks,
  renameAiWorkspaceConversation,
  runAiWorkspaceChat,
  selectAiWorkspaceConversation,
  steerAiWorkspaceChat,
} from "./api";
import { ConversationSwitcher } from "./ConversationSwitcher";
import type {
  AiWorkspaceChatStreamEvent,
  AiWorkspaceConversation,
  AiWorkspaceDocument,
  AiWorkspaceDocumentProgress,
  AiWorkspaceDocumentProposal,
  AiWorkspaceMessage,
  AiWorkspaceTask,
  RetrievedWorkspaceSource,
  WorkspaceCitation,
  WorkspaceAskQuestion,
  WorkspaceReference,
  WorkspaceToolCallRecord,
} from "./types";
import { WorkspaceCitations } from "./WorkspaceCitations";
import { WorkspaceReferencePicker } from "./WorkspaceReferencePicker";
import { WorkspaceRunTrace } from "./WorkspaceRunTrace";
import {
  findActiveWorkspaceTask,
  workspaceTaskElapsedMs,
  workspaceTaskStartedAtMs,
} from "./workspaceRunState";

function makeId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`;
}

const ARTIFACT_OUTPUT_NOUN = /(报告|函|通知|说明|协议|合同|意见书|方案|备忘录|清单|申请书|起诉状|答辩状|文书|文稿|文档|文件|材料)/;
const ARTIFACT_OUTPUT_ACTION = /(起草|撰写|拟定|制作|生成|形成|整理成|汇总成|输出|写一份|出一份|做一份|做个|给我)/;

function shouldAutoSaveArtifact(userRequest: string): boolean {
  return ARTIFACT_OUTPUT_NOUN.test(userRequest) && ARTIFACT_OUTPUT_ACTION.test(userRequest);
}

function artifactTitle(message: AiWorkspaceMessage, fallback: string): string {
  const heading = message.content
    .split("\n")
    .map((line) => line.match(/^#{1,6}\s+(.+?)\s*#*$/)?.[1]?.trim())
    .find(Boolean);
  return (heading || (fallback !== "新对话" ? fallback : "AI 生成文稿")).slice(0, 80);
}

function parseCitations(raw: string): WorkspaceCitation[] {
  try {
    const value = JSON.parse(raw) as unknown;
    return Array.isArray(value) ? (value as WorkspaceCitation[]) : [];
  } catch {
    return [];
  }
}

function parseToolCalls(raw: string): WorkspaceToolCallRecord[] {
  try {
    const value = JSON.parse(raw) as unknown;
    return Array.isArray(value) ? (value as WorkspaceToolCallRecord[]) : [];
  } catch {
    return [];
  }
}

function optimisticMessage(
  id: string,
  conversationId: string,
  role: "user" | "assistant",
  content: string,
  status: AiWorkspaceMessage["status"],
): AiWorkspaceMessage {
  const now = new Date().toISOString();
  return {
    id,
    conversation_id: conversationId,
    role,
    content,
    status,
    attached_document_ids_json: "[]",
    citations_json: "[]",
    artifact_document_id: null,
    model: null,
    prompt_tokens: null,
    completion_tokens: null,
    latency_ms: null,
    error_short: null,
    task_id: null,
    created_at: now,
    updated_at: now,
  };
}

export function WorkspaceChatPane({
  workspaceId,
  editingDocumentId,
  onDocumentCreated,
  onProposalCreated,
  beforeSend,
}: {
  workspaceId: string;
  editingDocumentId: string | null;
  onDocumentCreated?: (document: AiWorkspaceDocument) => void;
  onProposalCreated?: (proposal: AiWorkspaceDocumentProposal) => void;
  beforeSend?: () => Promise<boolean>;
}) {
  const [conversations, setConversations] = useState<AiWorkspaceConversation[]>([]);
  const [currentId, setCurrentId] = useState<string | null>(null);
  const [messages, setMessages] = useState<AiWorkspaceMessage[]>([]);
  const [documents, setDocuments] = useState<AiWorkspaceDocument[]>([]);
  const [references, setReferences] = useState<WorkspaceReference[]>([]);
  const [pendingAsk, setPendingAsk] =
    useState<WorkspaceAskQuestion[] | null>(null);
  const [showReferences, setShowReferences] = useState(false);
  const [draft, setDraft] = useState("");
  const [runningId, setRunningId] = useState<string | null>(null);
  const [reasoningChars, setReasoningChars] = useState(0);
  const [liveToolCalls, setLiveToolCalls] = useState<WorkspaceToolCallRecord[]>([]);
  const [liveActivities, setLiveActivities] = useState<ChatActivity[]>([]);
  const [runtimeHint, setRuntimeHint] = useState<AgentRuntime | null>(null);
  const [runStartedAt, setRunStartedAt] = useState<number | null>(null);
  const [liveElapsedMs, setLiveElapsedMs] = useState(0);
  const [tasksByMessage, setTasksByMessage] = useState<Record<string, AiWorkspaceTask>>({});
  const [error, setError] = useState<string | null>(null);
  const [sourcesByMessage, setSourcesByMessage] = useState<Record<string, RetrievedWorkspaceSource[]>>({});
  const [savingMessageId, setSavingMessageId] = useState<string | null>(null);
  const [proposingMessageId, setProposingMessageId] = useState<string | null>(null);
  const scrollerRef = useRef<HTMLDivElement>(null);
  const autoScrollRef = useRef(true);
  const currentIdRef = useRef<string | null>(null);
  const activeRunRef = useRef<{ messageId: string; conversationId: string } | null>(null);
  const referenceButtonRef = useRef<HTMLButtonElement>(null);
  const referencePickerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!showReferences) return;
    const closeOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (
        referenceButtonRef.current?.contains(target) ||
        referencePickerRef.current?.contains(target)
      ) {
        return;
      }
      setShowReferences(false);
    };
    document.addEventListener("pointerdown", closeOnOutsidePointer, true);
    return () =>
      document.removeEventListener("pointerdown", closeOnOutsidePointer, true);
  }, [showReferences]);

  const selectCurrentConversation = useCallback((conversationId: string) => {
    currentIdRef.current = conversationId;
    setCurrentId(conversationId);
  }, []);

  const clearActiveRun = useCallback((messageId?: string) => {
    if (messageId && activeRunRef.current?.messageId !== messageId) return;
    activeRunRef.current = null;
    setRunningId(null);
    setRunStartedAt(null);
    setLiveToolCalls([]);
    setLiveActivities([]);
  }, []);

  const applyRunSnapshot = useCallback((
    conversationId: string,
    messageRows: AiWorkspaceMessage[],
    taskRows: AiWorkspaceTask[],
  ) => {
    setMessages(messageRows);
    setTasksByMessage(
      Object.fromEntries(taskRows.map((task) => [task.assistant_message_id, task])),
    );
    const activeTask = findActiveWorkspaceTask(taskRows);
    if (activeTask) {
      activeRunRef.current = {
        messageId: activeTask.assistant_message_id,
        conversationId: activeTask.conversation_id,
      };
      setRunningId(activeTask.assistant_message_id);
      setRunStartedAt(workspaceTaskStartedAtMs(activeTask) ?? Date.now());
      setLiveElapsedMs(workspaceTaskElapsedMs(activeTask));
      setLiveToolCalls(parseToolCalls(activeTask.tool_calls_json));
      setLiveActivities([]);
      return;
    }
    const activeRun = activeRunRef.current;
    if (activeRun && activeRun.conversationId !== conversationId) return;
    setRunningId((current) => {
      if (!current) return null;
      const persisted = taskRows.find((task) => task.assistant_message_id === current);
      if (!persisted) return current;
      activeRunRef.current = null;
      setRunStartedAt(null);
      setLiveToolCalls([]);
      setLiveActivities([]);
      return null;
    });
  }, []);

  const refreshConversations = useCallback(async () => {
    const rows = await listAiWorkspaceConversations(workspaceId);
    setConversations(rows);
    return rows;
  }, [workspaceId]);

  const loadMessages = useCallback(async (conversationId: string) => {
    const [messageRows, taskRows] = await Promise.all([
      listAiWorkspaceMessages(workspaceId, conversationId),
      listAiWorkspaceTasks(workspaceId, conversationId),
    ]);
    applyRunSnapshot(conversationId, messageRows, taskRows);
    return messageRows;
  }, [applyRunSnapshot, workspaceId]);

  useEffect(() => {
    let cancelled = false;
    setPendingAsk(null);
    void (async () => {
      try {
        const [ensured, settings] = await Promise.all([
          ensureAiWorkspaceConversation(workspaceId),
          getSettings().catch(() => null),
        ]);
        const [conversationRows, documentRows, messageRows, taskRows] = await Promise.all([
          listAiWorkspaceConversations(workspaceId),
          listAiWorkspaceDocuments(workspaceId),
          listAiWorkspaceMessages(workspaceId, ensured.id),
          listAiWorkspaceTasks(workspaceId, ensured.id),
        ]);
        if (cancelled) return;
        setConversations(conversationRows);
        setDocuments(documentRows);
        if (settings) {
          setRuntimeHint(effectiveAgentRuntime(settings.agent_runtime));
        }
        selectCurrentConversation(ensured.id);
        applyRunSnapshot(ensured.id, messageRows, taskRows);
      } catch (cause) {
        if (!cancelled) setError(String(cause));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [applyRunSnapshot, selectCurrentConversation, workspaceId]);

  useEffect(() => {
    let cleanup: (() => void) | undefined;
    void listen<AiWorkspaceDocumentProgress>(
      "ai-workspace-document-progress",
      ({ payload }) => {
        if (payload.workspace_id === workspaceId) {
          void listAiWorkspaceDocuments(workspaceId).then(setDocuments);
        }
      },
    ).then((unlisten) => {
      cleanup = unlisten;
    });
    return () => cleanup?.();
  }, [workspaceId]);

  useEffect(() => {
    const scroller = scrollerRef.current;
    if (autoScrollRef.current && scroller) {
      scroller.scrollTop = scroller.scrollHeight;
    }
  }, [messages, reasoningChars, liveToolCalls, pendingAsk]);

  useEffect(() => {
    if (!runningId || runStartedAt === null) return;
    const update = () => setLiveElapsedMs(Date.now() - runStartedAt);
    update();
    const timer = window.setInterval(update, 1_000);
    return () => window.clearInterval(timer);
  }, [runStartedAt, runningId]);

  useEffect(() => {
    if (!runningId) return;
    let cancelled = false;
    const refresh = () => {
      const activeRun = activeRunRef.current;
      if (!activeRun) return;
      const request = activeRun.conversationId === currentIdRef.current
        ? loadMessages(activeRun.conversationId)
        : listAiWorkspaceTasks(workspaceId, activeRun.conversationId).then((taskRows) => {
            const task = taskRows.find(
              (item) => item.assistant_message_id === activeRun.messageId,
            );
            if (task && !["queued", "streaming"].includes(task.status)) {
              clearActiveRun(activeRun.messageId);
            }
          });
      void request.catch((cause) => {
        if (!cancelled) setError(`刷新 AI 任务状态失败：${String(cause)}`);
      });
    };
    const timer = window.setInterval(refresh, 1_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [clearActiveRun, loadMessages, runningId, workspaceId]);

  const switchConversation = async (conversationId: string) => {
    if (conversationId === currentId) return;
    setError(null);
    setPendingAsk(null);
    await selectAiWorkspaceConversation(workspaceId, conversationId);
    autoScrollRef.current = true;
    selectCurrentConversation(conversationId);
    setReferences([]);
    await loadMessages(conversationId);
  };

  const createConversation = async () => {
    if (runningId) return;
    const created = await createAiWorkspaceConversation(workspaceId);
    await refreshConversations();
    autoScrollRef.current = true;
    selectCurrentConversation(created.id);
    setMessages([]);
    setReferences([]);
    setPendingAsk(null);
    setDraft("");
  };

  const renameConversation = async (conversationId: string, title: string) => {
    await renameAiWorkspaceConversation(workspaceId, conversationId, title);
    await refreshConversations();
  };

  const archiveConversation = async (conversationId: string) => {
    if (runningId) return;
    await archiveAiWorkspaceConversation(workspaceId, conversationId);
    const ensured = await ensureAiWorkspaceConversation(workspaceId);
    await refreshConversations();
    autoScrollRef.current = true;
    selectCurrentConversation(ensured.id);
    setReferences([]);
    setPendingAsk(null);
    await loadMessages(ensured.id);
  };

  const send = async (answerText?: string) => {
    const text = (answerText ?? draft).trim();
    if (!text || !currentId) return;
    if (runningId) {
      if (activeRunRef.current?.conversationId !== currentId) {
        setError("当前 AI 任务属于另一段对话，请切回该对话后再继续引导。");
        return;
      }
      try {
        const id = await steerAiWorkspaceChat({
          messageId: runningId,
          workspaceId,
          conversationId: currentId,
          content: text,
        });
        setDraft("");
        setError(null);
        setMessages((current) => [
          ...current,
          optimisticMessage(id, currentId, "user", text, "completed"),
        ]);
      } catch (cause) {
        setError(`发送引导失败：${String(cause)}`);
      }
      return;
    }
    if (beforeSend && !(await beforeSend())) return;
    const userMessageId = makeId();
    const assistantMessageId = makeId();
    const conversationId = currentId;
    autoScrollRef.current = true;
    setDraft("");
    setError(null);
    setPendingAsk(null);
    setReasoningChars(0);
    setLiveToolCalls([]);
    setLiveActivities([]);
    setRunStartedAt(Date.now());
    setLiveElapsedMs(0);
    activeRunRef.current = {
      messageId: assistantMessageId,
      conversationId,
    };
    setRunningId(assistantMessageId);
    setMessages((current) => [
      ...current,
      optimisticMessage(userMessageId, conversationId, "user", text, "completed"),
      optimisticMessage(assistantMessageId, conversationId, "assistant", "", "streaming"),
    ]);

    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<AiWorkspaceChatStreamEvent>(
        `ai-workspace-chat-stream-${assistantMessageId}`,
        ({ payload }) => {
          if (payload.kind === "reasoning") {
            setReasoningChars((count) => count + payload.text.length);
          } else if (payload.kind === "tool_call") {
            setLiveToolCalls((current) => [...current, payload.record]);
          } else if (payload.kind === "activity") {
            setLiveActivities((current) => [...current, payload.activity]);
          } else if (payload.kind === "delta") {
            setMessages((current) => current.map((message) =>
              message.id === assistantMessageId
                ? { ...message, content: message.content + payload.text }
                : message,
            ));
          } else if (payload.kind === "error") {
            setError(payload.message);
          }
        },
      );
      const result = await runAiWorkspaceChat({
        workspace_id: workspaceId,
        conversation_id: conversationId,
        user_message: text,
        user_message_id: userMessageId,
        message_id: assistantMessageId,
        references,
        editing_document_id: editingDocumentId,
      });
      if (currentIdRef.current === conversationId) {
        setPendingAsk(result.ask_user?.length ? result.ask_user : null);
      }
      setSourcesByMessage((current) => ({ ...current, [assistantMessageId]: result.sources }));
      setReferences([]);
      const [finalMessages] = await Promise.all([
        currentIdRef.current === conversationId
          ? loadMessages(conversationId)
          : listAiWorkspaceMessages(workspaceId, conversationId),
        refreshConversations(),
      ]);
      if (result.artifact_doc_id) {
        const refreshedDocuments = await listAiWorkspaceDocuments(workspaceId);
        setDocuments(refreshedDocuments);
        const created = refreshedDocuments.find(
          (document) => document.id === result.artifact_doc_id,
        );
        if (created) onDocumentCreated?.(created);
      }
      const completedMessage = finalMessages.find(
        (message) => message.id === result.assistant_message_id,
      );
      if (
        !editingDocumentId
        && shouldAutoSaveArtifact(text)
        && completedMessage?.status === "completed"
        && completedMessage.content.trim()
        && !completedMessage.artifact_document_id
      ) {
        setSavingMessageId(completedMessage.id);
        try {
          const conversationTitle = conversations.find(
            (item) => item.id === conversationId,
          )?.title ?? "新对话";
          const artifact = await createAiWorkspaceArtifactFromMessage(
            workspaceId,
            completedMessage.id,
            artifactTitle(completedMessage, conversationTitle),
          );
          if (currentIdRef.current === conversationId) {
            setMessages((current) => current.map((message) =>
              message.id === completedMessage.id
                ? { ...message, artifact_document_id: artifact.document.id }
                : message,
            ));
          }
          onDocumentCreated?.(artifact.document);
        } catch (cause) {
          setError(`AI 已完成，但自动保存文稿失败：${String(cause)}`);
        } finally {
          setSavingMessageId(null);
        }
      }
    } catch (cause) {
      setError(String(cause));
      if (currentIdRef.current === conversationId) {
        await loadMessages(conversationId).catch(() => undefined);
      }
    } finally {
      unlisten?.();
      clearActiveRun(assistantMessageId);
    }
  };

  const saveAsArtifact = async (message: AiWorkspaceMessage) => {
    if (savingMessageId) return;
    setSavingMessageId(message.id);
    setError(null);
    try {
      const title = conversations.find((item) => item.id === message.conversation_id)?.title ?? "AI 文稿";
      const artifact = await createAiWorkspaceArtifactFromMessage(
        workspaceId,
        message.id,
        title,
      );
      setMessages((current) => current.map((item) =>
        item.id === message.id
          ? { ...item, artifact_document_id: artifact.document.id }
          : item,
      ));
      onDocumentCreated?.(artifact.document);
    } catch (cause) {
      setError(`保存文稿失败：${String(cause)}`);
    } finally {
      setSavingMessageId(null);
    }
  };

  const proposeForCurrentArtifact = async (message: AiWorkspaceMessage) => {
    if (!editingDocumentId || proposingMessageId) return;
    setProposingMessageId(message.id);
    setError(null);
    try {
      const proposal = await createAiWorkspaceDocumentProposal(
        workspaceId,
        editingDocumentId,
        message.conversation_id,
        message.id,
      );
      onProposalCreated?.(proposal);
    } catch (cause) {
      setError(`创建修改审阅失败：${String(cause)}`);
    } finally {
      setProposingMessageId(null);
    }
  };

  const readyDocuments = useMemo(
    () => documents.filter((document) => document.kind === "artifact" || ["ready", "review"].includes(document.extraction_status)),
    [documents],
  );
  const runBelongsToCurrentConversation = !runningId
    || activeRunRef.current?.conversationId === currentId;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ConversationSwitcher
        conversations={conversations}
        currentId={currentId}
        disabled={Boolean(runningId)}
        onSelect={(id) => void switchConversation(id)}
        onCreate={() => void createConversation()}
        onRename={(id, title) => void renameConversation(id, title)}
        onArchive={(id) => void archiveConversation(id)}
      />

      <div
        ref={scrollerRef}
        aria-label="对话消息"
        onScroll={(event) => {
          const scroller = event.currentTarget;
          autoScrollRef.current = scroller.scrollHeight
            - scroller.scrollTop
            - scroller.clientHeight < 80;
        }}
        className="min-h-0 flex-1 overflow-auto px-3 py-3"
      >
        {messages.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center px-5 text-center">
            <Sparkles className="mb-3 size-6 text-brand/70" />
            <p className="text-sm font-medium text-foreground">从一个要求开始</p>
            <p className="mt-1 text-xs leading-relaxed text-muted-foreground">可以不上传材料直接起草，也可以 @ 当前工作区文件作为依据。</p>
          </div>
        ) : (
          <div className="space-y-4">
            {messages.map((message) => {
              const citations = parseCitations(message.citations_json);
              const isRunning = message.id === runningId;
              const task = tasksByMessage[message.id];
              const toolCalls = isRunning
                ? liveToolCalls
                : parseToolCalls(task?.tool_calls_json ?? "[]");
              const traceStatus = isRunning
                ? "streaming"
                : task?.status ?? message.status;
              const traceElapsed = isRunning
                ? liveElapsedMs
                : task
                  ? workspaceTaskElapsedMs(task)
                  : message.latency_ms ?? 0;
              return (
                <article key={message.id} className={cn("text-xs", message.role === "user" ? "ml-8" : "mr-3")}>
                  {message.role === "assistant" && (isRunning || task || message.latency_ms !== null) ? (
                    <WorkspaceRunTrace
                      status={traceStatus}
                      elapsedMs={traceElapsed}
                      reasoningObserved={isRunning ? reasoningChars > 0 : Boolean(task)}
                      toolCalls={toolCalls}
                      activities={isRunning ? liveActivities : []}
                      runtimeHint={isRunning ? runtimeHint : null}
                    />
                  ) : null}
                  <div className={cn("rounded-xl px-3 py-2.5 leading-relaxed", message.role === "user" ? "bg-brand text-white" : "border border-border bg-card text-foreground")}>
                    {message.role === "assistant" && message.content ? (
                      <div className="prose prose-sm max-w-none text-xs"><ReactMarkdown remarkPlugins={[remarkGfm]}>{message.content}</ReactMarkdown></div>
                    ) : message.content ? message.content : (
                      <span className="inline-flex items-center gap-1.5 text-muted-foreground"><Loader2 className="size-3 animate-spin" />{reasoningChars > 0 && message.id === runningId ? "正在分析并规划下一步" : "正在准备结果"}</span>
                    )}
                  </div>
                  {message.error_short ? <p className="mt-1 text-[11px] text-destructive">{message.error_short}</p> : null}
                  {message.status === "incomplete" ? <p className="mt-1 text-[11px] text-amber-600">本轮未完整结束，已保留现有内容，可继续追问。</p> : null}
                  {citations.length > 0 ? (
                    <div className="mt-1.5 flex flex-wrap gap-1">
                      {citations.map((citation) => <span key={`${citation.ref}-${citation.source}`} className="rounded bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground">[{citation.ref}] {citation.source}{citation.type === "doc" && !citation.verified ? " · 待核对" : ""}</span>)}
                    </div>
                  ) : null}
                  <WorkspaceCitations citations={sourcesByMessage[message.id] ?? []} />
                  {message.role === "assistant" && message.content && ["completed", "incomplete"].includes(message.status) ? (
                    <div className="mt-1.5 flex flex-wrap gap-1">
                      <button
                        type="button"
                        aria-label="保存为新文稿"
                        disabled={savingMessageId === message.id}
                        onClick={() => void saveAsArtifact(message)}
                        className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] text-brand hover:bg-brand-soft disabled:opacity-50"
                      >
                        {savingMessageId === message.id ? <Loader2 className="size-3 animate-spin" /> : <FilePlus2 className="size-3" />}
                        {message.artifact_document_id ? "打开已保存文稿" : "保存为新文稿"}
                      </button>
                      {editingDocumentId && !message.artifact_document_id ? (
                        <button
                          type="button"
                          aria-label="审阅并应用到当前文稿"
                          disabled={proposingMessageId === message.id}
                          onClick={() => void proposeForCurrentArtifact(message)}
                          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] text-brand hover:bg-brand-soft disabled:opacity-50"
                        >
                          {proposingMessageId === message.id ? <Loader2 className="size-3 animate-spin" /> : <FilePenLine className="size-3" />}
                          应用到当前文稿
                        </button>
                      ) : null}
                    </div>
                  ) : null}
                </article>
              );
            })}
          </div>
        )}

        {pendingAsk?.length && !runningId ? (
          <div className="mt-3 border-t border-border/70 pb-1 pt-1">
            <AskUserCard
              questions={pendingAsk}
              onSubmit={(answer) => void send(answer)}
            />
          </div>
        ) : null}
      </div>
      {error ? <div className="border-t border-destructive/20 bg-destructive/5 px-3 py-2 text-[11px] text-destructive">{error}</div> : null}
      <div className="relative border-t border-border bg-card p-2.5">
        {showReferences ? (
          <div ref={referencePickerRef} className="absolute bottom-full left-2 right-2 z-30 mb-1">
            <WorkspaceReferencePicker documents={readyDocuments} value={references} onChange={setReferences} />
          </div>
        ) : null}
        <textarea
          aria-label="给 AI 的要求"
          value={draft}
          disabled={!currentId || !runBelongsToCurrentConversation}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              void send();
            }
          }}
          placeholder={runningId
            ? runBelongsToCurrentConversation
              ? "继续补充要求，作为当前任务的引导……"
              : "当前 AI 任务正在另一段对话中，请切回后继续引导……"
            : "告诉 AI 要写什么、怎么修改……"}
          className="min-h-20 w-full resize-none rounded-lg border border-border bg-background px-3 py-2 pr-10 text-xs leading-relaxed outline-none focus:border-brand/50 disabled:opacity-60"
        />
        <div className="mt-1.5 flex items-center gap-1.5">
          <button ref={referenceButtonRef} type="button" aria-label="引用工作区文件" onClick={() => setShowReferences((value) => !value)} className={cn("inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px]", references.length > 0 ? "bg-brand-soft text-brand" : "text-muted-foreground hover:bg-muted")}><AtSign className="size-3" />引用{references.length > 0 ? ` ${references.length}` : ""}</button>
          <span className="text-[10px] text-muted-foreground">Enter 发送 · Shift+Enter 换行</span>
          {runningId && (
            <button type="button" aria-label="停止生成" onClick={() => void cancelAiWorkspaceChat(runningId)} className="ml-auto inline-flex items-center gap-1 rounded-md bg-destructive/10 px-2 py-1 text-[11px] text-destructive"><CircleStop className="size-3" />停止</button>
          )}
          <button type="button" aria-label={runningId ? "引导当前 AI 任务" : "发送给 AI"} disabled={!draft.trim() || !currentId || !runBelongsToCurrentConversation} onClick={() => void send()} className={cn("inline-flex items-center gap-1 rounded-md bg-foreground px-2.5 py-1.5 text-[11px] text-background disabled:opacity-30", !runningId && "ml-auto")}><Send className="size-3" />{runningId ? "引导" : "发送"}</button>
        </div>
      </div>
    </div>
  );
}
