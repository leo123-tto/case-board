/**
 * 案件 AI 助手 — 案件详情页右侧聊天面板(V0.1.13+)。
 *
 * 用户视角:聊天框
 * 系统视角:case-aware chat —— 后端拼当前 case_id 的 snapshot + 文档 → DeepSeek 流式
 *
 * 设计要点:
 *   - 默认折叠成 32px sliver(localStorage 记忆),作者 13" MBP 窄屏友好
 *   - 快捷 chip(V0.3.3:6 个功能单一的生成型 chip 已删,AI 已是 agent、直接打字即可):
 *     4 个工具/分析型(法律依据/模拟对抗/类案检索/深度分析,走 agent_loop、联网查法条与类案)
 *     + 2 个写文书入口(写起诉状/写证据目录,走 save_artifact 落可编辑文书)。每个 chip 悬停有即时说明气泡
 *   - 流式输出:模块级 chatRunRegistry 监听 "chat-stream-{message_id}"(跨面板卸载存活)
 *   - 真错红条:LLM 真错(429/401/超时)按原文显示,不替换成"不可达?"
 *   - 进行中可点「停止」走 cancel_chat 走后端 oneshot
 *
 * 切忌:本面板**不**自己写 DB,所有持久化由后端 case_chat 命令负责
 */

import {
  memo,
  type ComponentProps,
  type MouseEvent as ReactMouseEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  ArrowDown,
  ArrowLeft,
  ChevronRight,
  CircleStop,
  ExternalLink,
  FileText,
  Loader2,
  Paperclip,
  Send,
  Sparkles,
  Trash2,
} from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import {
  type AskQuestion,
  caseChat,
  cancelChat,
  type CaseChatTaskType,
  type ChatMessage,
  clearChatHistory,
  getCaseWithDocs,
  listChatHistory,
  openUrl,
} from "@/lib/api";
import type {
  CaseWithDocs,
  Citation,
  Document,
  ToolCallRecord,
} from "@/lib/types";
import { confirmDialog } from "@/lib/dialog";

import { AskUserCard } from "./AskUserCard";
import { AttachmentChips } from "./AttachmentChips";
import { AttachmentPicker } from "./AttachmentPicker";
import { CitationsCard } from "./CitationsCard";
import { ToolCallTrace } from "./ToolCallTrace";
import {
  clearRun,
  finishRun,
  getRun,
  isRunning,
  startRun,
  subscribeRun,
  type ChatSegment,
} from "./chatRunRegistry";
/** localStorage key:chat 面板折叠状态。同 key 被 FeedbackButton 读取以避让。 */
export const CHAT_PANEL_COLLAPSED_KEY = "caseboard.chat-panel.collapsed";
/** localStorage value 含义:"1"=折叠,"0"=展开。默认展开。 */
const COLLAPSED_VALUE = "1";
const EXPANDED_VALUE = "0";
/** 主窗口内的展开宽度可拖拽调整并持久化。 */
export const CHAT_PANEL_WIDTH_KEY = "caseboard.chat-panel.width";
const CHAT_PANEL_WIDTH_DEFAULT = 420;
const CHAT_PANEL_WIDTH_MIN = 360;
const CHAT_PANEL_WIDTH_MAX = 720;
const CHAT_PANEL_REATTACH_EVENT = "caseboard:chat-window-reattach";

/** 自定义事件名 — 面板折叠状态变化时 dispatch,FeedbackButton 监听以同步位置。 */
const CHAT_PANEL_TOGGLE_EVENT = "caseboard:chat-panel-toggle";

/** V0.2 D6-D7 ·「⚖️ 法律依据」chip 会根据是否引用文档动态切换 task_type 与说明。 */
const LEGAL_BASIS_CHIP = {
  label: "⚖️ 法律依据",
  hintNoAttached:
    "围绕本案诉求联网查准法条 + 类案,整理成法律依据清单(会先判断在审理还是执行阶段)",
  hintWithAttached:
    "核对你引用文档里的法条/案号准不准,逐条标 ✅一致 / ⚠️不一致 / ❌查无此条",
} as const;

const PROVINCE_SUFFIX_PATTERN =
  /(北京市|天津市|上海市|重庆市|[^省市自治区特别行政区]+(?:省|自治区|特别行政区))/;

function inferCourtRegion(
  caseData?: CaseWithDocs["case"] | null,
): string | null {
  const court = caseData?.agg_court || caseData?.court;
  if (!court) return null;
  const province = court.match(PROVINCE_SUFFIX_PATTERN)?.[1];
  if (province) return province;
  return (
    court
      .replace(
        /(人民法院|法院|中级|高级|基层|互联网|知识产权|海事|金融)/g,
        "",
      )
      .trim() || court
  );
}

/**
 * 快捷任务 chip:一个按钮 + 悬停即时说明气泡。
 * 用自定义气泡(group-hover)取代原生 `title` —— 原生 tooltip 在 Tauri WebView 里
 * 延迟约 2-3 秒、还是不显眼的系统小黄条,作者反馈「不知道按钮干嘛」。
 * 气泡用 bg-foreground/text-background(明暗模式都反色高对比),往上弹(面板无 overflow-hidden,不裁切)。
 */
function QuickChip({
  label,
  hint,
  onClick,
  disabled,
  className,
}: {
  label: string;
  hint: string;
  onClick: () => void;
  disabled: boolean;
  /** 覆盖默认配色(彩色高级 chip 用) */
  className?: string;
}) {
  const wrapRef = useRef<HTMLSpanElement>(null);
  // 悬停时测一下:气泡从 chip 左缘展开(最宽 260)会不会超出窗口右边 → 超出就改成右对齐
  //(气泡向左展开),否则右侧那几个 chip 的说明会被切到看不见(老板真机反馈)。
  const [alignRight, setAlignRight] = useState(false);
  const onEnter = () => {
    const el = wrapRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    setAlignRight(rect.left + 268 > window.innerWidth - 8);
  };
  return (
    <span
      ref={wrapRef}
      className="group relative inline-flex"
      onMouseEnter={onEnter}
    >
      <button
        type="button"
        onClick={onClick}
        disabled={disabled}
        className={cn(
          "rounded-md border px-2 py-1 text-xs text-foreground transition-[color,background-color,border-color,transform] active:scale-[0.97] disabled:cursor-not-allowed disabled:opacity-60 disabled:active:scale-100",
          className ?? "border-border bg-background hover:bg-accent",
        )}
      >
        {label}
      </button>
      <span
        role="tooltip"
        className={cn(
          "pointer-events-none invisible absolute bottom-full z-30 mb-1.5 w-max max-w-[260px] translate-y-0.5 scale-95 rounded-md bg-foreground px-2.5 py-1.5 text-xs leading-relaxed text-background opacity-0 shadow-lg transition-[opacity,transform,visibility] duration-150 ease-out group-hover:visible group-hover:translate-y-0 group-hover:scale-100 group-hover:opacity-100",
          alignRight ? "right-0" : "left-0",
        )}
      >
        {hint}
      </span>
    </span>
  );
}

/** 三点跳动打字指示器(AI 还没吐字时显示,比静态「正在思考…」更有生命感)。 */
function TypingDots() {
  return (
    <span
      className="inline-flex items-center gap-1 py-1"
      aria-label="正在思考"
      role="status"
    >
      {[0, 150, 300].map((delay) => (
        <span
          key={delay}
          className="size-1.5 animate-bounce rounded-full bg-muted-foreground/60"
          style={{ animationDelay: `${delay}ms` }}
        />
      ))}
    </span>
  );
}

/** 深度推理进度(thinking 模型推理阶段还没吐正文时):字数在涨 = 没卡死。 */
function ReasoningIndicator({ chars }: { chars: number }) {
  return (
    <span
      className="inline-flex items-center gap-1.5 py-1 text-xs text-muted-foreground"
      role="status"
    >
      <Loader2 className="size-3 animate-spin" />
      <span>🧠 深度推理中…(已 {chars.toLocaleString()} 字)</span>
    </span>
  );
}

interface Props {
  caseId: string | null;
  caseName?: string | null;
  /** 落了 artifact 时回调(让 CaseView 刷新文档列表) */
  onArtifactCreated?: (docId: string) => void;
  /** V0.3 ADR-0003 Phase 1B · 编辑器里正打开的 AI 文书 doc_id(随 caseChat 传后端注入 prompt) */
  editingDocId?: string | null;
  /** V0.3 1B · 发送前钩子:写作模式下若编辑器有未保存改动,先 flush 到磁盘
   *  (让 AI 的 edit_artifact 在最新内容上操作)。await 完再发请求。 */
  onBeforeSend?: () => Promise<void>;
  /** V0.3 1B · 本轮 AI 调了 edit_artifact 局部改了文书 → 通知 CaseView 刷新 / 重载编辑器 */
  onArtifactEdited?: () => void;
  /**
   * 案件领域(2026-06-17)。"criminal" = 刑事 tab:只保留「刑事深度分析」单 chip,
   * 隐藏其余民事 chip(法律依据/模拟对抗/类案/深度分析/写起诉状/写证据目录)。默认 "civil"。
   */
  domain?: "civil" | "criminal";
  /** 分离模式:助手铺满独立界面,不显示折叠、拖宽和再次分离入口。 */
  detached?: boolean;
}

export function CaseChatPanel({
  caseId,
  caseName,
  onArtifactCreated,
  editingDocId,
  onBeforeSend,
  onArtifactEdited,
  domain = "civil",
  detached = false,
}: Props) {
  // 分离界面永远完整显示;停靠在主窗口时沿用用户上次的折叠选择。
  const [collapsed, setCollapsed] = useState<boolean>(() => {
    if (detached) return false;
    try {
      return localStorage.getItem(CHAT_PANEL_COLLAPSED_KEY) === COLLAPSED_VALUE;
    } catch {
      return false;
    }
  });
  const [panelWidth, setPanelWidth] = useState<number>(() => {
    try {
      const value = Number(localStorage.getItem(CHAT_PANEL_WIDTH_KEY));
      return value >= CHAT_PANEL_WIDTH_MIN && value <= CHAT_PANEL_WIDTH_MAX
        ? value
        : CHAT_PANEL_WIDTH_DEFAULT;
    } catch {
      return CHAT_PANEL_WIDTH_DEFAULT;
    }
  });
  const [history, setHistory] = useState<ChatMessage[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [restorePulse, setRestorePulse] = useState(false);
  const [input, setInput] = useState("");
  // V0.3 · 模型调 ask_user 发起的选项式追问;非 null 时在末尾渲染选项卡片。
  // 任何新消息(点选项 / 自己发)开头即清,切案件也清。
  const [pendingAsk, setPendingAsk] = useState<AskQuestion[] | null>(null);
  // 2026-05-31 · 流式状态来自模块级 registry(跨面板卸载存活)。forceRerender 强制重渲染。
  const [, forceRerender] = useState(0);
  const run = getRun(caseId);
  const streamingText = run?.status === "running" ? run.text : "";
  const streamingSegments = run?.status === "running" ? run.segments : [];
  const isStreaming = run?.status === "running";
  // thinking 模型本段推理已累计字数(>0 时显示「深度推理中…(N 字)」,字数在涨=没卡死)
  const streamingReasoningChars =
    run?.status === "running" ? run.reasoningChars : 0;
  // V0.2 D6-D7 · attachment 状态
  const [caseDocs, setCaseDocs] = useState<Document[]>([]);
  const [loadedCaseData, setLoadedCaseData] =
    useState<CaseWithDocs["case"] | null>(null);
  const [attachedDocIds, setAttachedDocIds] = useState<string[]>([]);
  const [pickerOpen, setPickerOpen] = useState(false);
  const scrollerRef = useRef<HTMLDivElement>(null);
  // V0.2.2 · 自由滚动:用户上滚查看历史时停止强制吸底,滚回底部附近再恢复自动跟随
  const [autoScroll, setAutoScroll] = useState(true);
  const courtRegion = inferCourtRegion(loadedCaseData);
  const similarCasesHint = courtRegion
    ? `检索全国相似判例,按本案法院所在地 ${courtRegion} 优先排序;外地高相关案例仍正常纳入,再判断对我方诉求的支持度和风险点`
    : "检索全国相似判例,按本案法院所在地优先排序;外地高相关案例仍正常纳入,再判断对我方诉求的支持度和风险点";

  const refreshHistory = useCallback(async () => {
    if (!caseId) return;
    const rows = await listChatHistory(caseId);
    setHistory(rows);
  }, [caseId]);

  const startResize = (event: ReactMouseEvent) => {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = panelWidth;
    let latestWidth = startWidth;
    const onMove = (moveEvent: MouseEvent) => {
      latestWidth = Math.min(
        CHAT_PANEL_WIDTH_MAX,
        Math.max(
          CHAT_PANEL_WIDTH_MIN,
          startWidth + (startX - moveEvent.clientX),
        ),
      );
      setPanelWidth(latestWidth);
    };
    const onUp = () => {
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
      try {
        localStorage.setItem(CHAT_PANEL_WIDTH_KEY, String(latestWidth));
      } catch {
        /* ignore quota */
      }
    };
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  };

  const detachChatPanel = async () => {
    if (!caseId) return;
    if (isRunning(caseId)) {
      setError("当前案件的 AI 任务完成后才能切换到独立界面。");
      return;
    }
    try {
      await invoke("detach_chat_window", {
        caseId,
        caseName: caseName ?? null,
        domain,
      });
      setRestorePulse(false);
      setCollapsed(true);
    } catch (e) {
      setError(formatError(e));
    }
  };

  const reattachChatPanel = async () => {
    try {
      await getCurrentWindow().close();
    } catch (e) {
      setError(formatError(e));
    }
  };

  // V0.2 D6-D7 · attached chip 用,对 caseDocs 按 id 索引
  const attachedDocs = useMemo(() => {
    const byId = new Map(caseDocs.map((d) => [d.id, d]));
    return attachedDocIds
      .map((id) => byId.get(id))
      .filter((d): d is Document => d !== undefined)
      .map((d) => ({
        id: d.id,
        filename: d.filename,
        is_ai_artifact: d.is_ai_artifact,
      }));
  }, [caseDocs, attachedDocIds]);

  // localStorage 持久化折叠状态 + 广播事件给 FeedbackButton 避让
  useEffect(() => {
    try {
      localStorage.setItem(
        CHAT_PANEL_COLLAPSED_KEY,
        collapsed ? COLLAPSED_VALUE : EXPANDED_VALUE,
      );
    } catch {
      /* ignore quota */
    }
    try {
      window.dispatchEvent(
        new CustomEvent(CHAT_PANEL_TOGGLE_EVENT, { detail: { collapsed } }),
      );
    } catch {
      /* ignore */
    }
  }, [collapsed]);

  // 分离界面放回侧栏后,同步聊天记录与材料列表。
  useEffect(() => {
    if (detached || !caseId) return;
    let unlisten: UnlistenFn | undefined;
    let pulseTimer: number | undefined;
    listen<{ caseId: string }>(CHAT_PANEL_REATTACH_EVENT, (event) => {
      if (event.payload.caseId !== caseId) return;
      setCollapsed(false);
      setRestorePulse(true);
      void refreshHistory().catch(() => {});
      onArtifactCreated?.("");
      if (pulseTimer) window.clearTimeout(pulseTimer);
      pulseTimer = window.setTimeout(() => setRestorePulse(false), 720);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((e) => console.warn("listen chat window restore failed", e));
    return () => {
      if (pulseTimer) window.clearTimeout(pulseTimer);
      unlisten?.();
    };
  }, [caseId, detached, onArtifactCreated, refreshHistory]);

  // case 切换时重新拉历史 + 拉 case docs(给 AttachmentPicker)+ 清 attached 状态
  useEffect(() => {
    // V0.2 D6-D7 · 切 case 必须清掉前一个案件的引用(streaming 由 registry 管,按 caseId 隔离)
    setAttachedDocIds([]);
    setPickerOpen(false);
    // V0.3 · 切案件清掉上一个案件遗留的选项卡片
    setPendingAsk(null);
    if (!caseId || collapsed) return;
    let abort = false;
    setHistoryLoading(true);
    Promise.all([listChatHistory(caseId), getCaseWithDocs(caseId)])
      .then(([rows, withDocs]) => {
        if (abort) return;
        setHistory(rows);
        setLoadedCaseData(withDocs.case);
        setCaseDocs(withDocs.documents);
      })
      .catch((e) => {
        if (!abort) setError(formatError(e));
      })
      .finally(() => {
        if (!abort) setHistoryLoading(false);
      });
    return () => {
      abort = true;
    };
  }, [caseId, collapsed]);

  // 滚动到底(仅当用户停在底部附近;上滚查看历史时不强制打扰)
  useEffect(() => {
    if (autoScroll && scrollerRef.current) {
      scrollerRef.current.scrollTop = scrollerRef.current.scrollHeight;
    }
  }, [history.length, streamingText, autoScroll]);

  // 独立窗口与主窗口共用 SQLite;窗口重新获得焦点时拉取最新记录。
  useEffect(() => {
    if (!caseId || collapsed) return;
    const refreshVisibleHistory = () => {
      if (document.visibilityState === "hidden") return;
      void refreshHistory().catch(() => {});
    };
    window.addEventListener("focus", refreshVisibleHistory);
    document.addEventListener("visibilitychange", refreshVisibleHistory);
    refreshVisibleHistory();
    return () => {
      window.removeEventListener("focus", refreshVisibleHistory);
      document.removeEventListener("visibilitychange", refreshVisibleHistory);
    };
  }, [caseId, collapsed, refreshHistory]);

  // 2026-05-31 · 订阅模块级 run registry:面板(重新)挂载时重连进行中的运行,
  // 运行结束时刷新历史并清除 registry。解决「流式中切走再回来 → 输出消失 + 重复点击出两份」。
  useEffect(() => {
    if (!caseId) return;
    const unsub = subscribeRun(caseId, () => {
      forceRerender((n) => n + 1);
      const r = getRun(caseId);
      // 运行刚结束 → 刷历史拿落库结果,再清掉 registry(避免残留 streaming UI)
      if (r && r.status === "done") {
        refreshHistory()
          .catch(() => {})
          .finally(() => clearRun(caseId));
      }
    });
    // 挂载即重渲染一次,显示可能已在进行的运行
    forceRerender((n) => n + 1);
    return unsub;
  }, [caseId, refreshHistory]);

  const disabled = !caseId || isStreaming;

  async function send(text: string, taskType: CaseChatTaskType | null) {
    if (!caseId) return;
    const trimmed = text.trim();
    if (!trimmed && !taskType) return;

    // 2026-05-31 · 去重锁:同案件已有运行在跑(可能在你切去首页时仍后台进行),拦住
    // 重复发起 —— 根治「以为卡住了重新点 → 出两份摘要」。
    if (isRunning(caseId)) {
      setError("当前案件已有一个 AI 任务在进行,请等它完成(切到首页也会在后台继续跑)。");
      return;
    }

    const messageId = crypto.randomUUID();
    setError(null);
    // V0.3 · 新一轮开始,上一轮的选项卡片作废(点了选项 or 自己发都算回答了)
    setPendingAsk(null);

    // 在模块级 registry 起监听(跨面板卸载存活)
    const ok = await startRun(caseId, messageId);
    if (!ok) {
      setError("当前案件已有一个 AI 任务在进行,请等它完成。");
      return;
    }

    // V0.2 D6-D7 · 把本轮引用快照下来,送出去后清掉(避免下一轮意外带上)
    const attachedSnapshot = attachedDocIds.length > 0 ? [...attachedDocIds] : null;
    setAttachedDocIds([]);
    setInput("");

    // V0.3 1B · 写作模式下编辑器若有未保存改动,先 flush 到磁盘 ——
    // AI 的 edit_artifact 改的是磁盘,必须先让磁盘=编辑器当前内容,否则改在旧版本上。
    try {
      await onBeforeSend?.();
    } catch {
      /* flush 失败不阻断发送(最坏 AI 在稍旧版本上改,后续 reload 会拉回磁盘真值) */
    }

    try {
      const result = await caseChat({
        case_id: caseId,
        user_message: trimmed,
        task_type: taskType,
        message_id: messageId,
        attached_doc_ids: attachedSnapshot,
        editing_doc_id: editingDocId ?? null,
      });
      finishRun(caseId);
      // 拿最新历史(后端已经 INSERT 完两条);registry 的 done 订阅也会刷,这里立即刷一次
      const fresh = await listChatHistory(caseId);
      setHistory(fresh);
      // V0.3 · 模型这轮发起了选项式追问 → 末尾渲染选项卡片,等用户点选/填写
      setPendingAsk(result.ask_user ?? null);
      // V0.3 · **只有「写文书」(save_artifact)才自动进编辑器**。分析类任务(类案检索/法律依据
      // 等)也会落 artifact_doc_id(write_chat_artifact),但它们是分析产物、不该自动跳编辑器
      //(老板真机反馈)—— 只 reload 让它出现在文档列表,用户想编辑再手动点。
      const calledSaveArtifact = result.tool_calls?.some(
        (t) => t.tool === "save_artifact" && t.success,
      );
      if (onArtifactCreated) {
        if (calledSaveArtifact && result.artifact_doc_id) {
          onArtifactCreated(result.artifact_doc_id); // 写文书 → 自动进编辑器
        } else if (result.artifact_doc_id || calledSaveArtifact) {
          onArtifactCreated(""); // 分析类 artifact / 兜底 → 只 reload,不自动进编辑器
        }
      }
      // V0.3 1B · 本轮 AI 调了 edit_artifact 局部改了文书 → 通知 CaseView 刷新 + 重载编辑器显示改动
      const calledEditArtifact = result.tool_calls?.some(
        (t) => t.tool === "edit_artifact" && t.success,
      );
      if (calledEditArtifact) onArtifactEdited?.();
    } catch (e) {
      const msg = formatError(e);
      setError(msg);
      finishRun(caseId, msg);
      // 即便错了也刷新历史(后端会写一行 error_short)
      try {
        const fresh = await listChatHistory(caseId);
        setHistory(fresh);
      } catch {
        /* swallow */
      }
    } finally {
      clearRun(caseId);
    }
  }

  async function stop() {
    const r = getRun(caseId);
    if (r?.messageId) {
      try {
        await cancelChat(r.messageId);
      } catch {
        /* ignore */
      }
    }
  }

  async function clearAll() {
    if (!caseId) return;
    if (
      !(await confirmDialog(
        "确定清空当前案件的所有聊天记录?(只清记录,不影响已生成的 artifact)",
        { danger: true, okLabel: "清空" },
      ))
    ) {
      return;
    }
    try {
      await clearChatHistory(caseId);
      setHistory([]);
      setError(null);
    } catch (e) {
      setError(formatError(e));
    }
  }

  // 折叠、拖宽和独立窗口共用同一个面板主体。
  return (
    <aside
      className={cn(
        "relative flex h-full shrink-0 flex-col border-border bg-card/30 transition-[width,box-shadow,background-color] duration-500 ease-[cubic-bezier(0.22,1,0.36,1)]",
        detached
          ? "w-full border-l-0"
          : collapsed
            ? "w-12 items-center border-l"
            : "border-l",
        restorePulse &&
          !detached &&
          "bg-sky-50/55 shadow-[-12px_0_28px_-24px_rgba(14,165,233,0.95)] dark:bg-sky-950/20",
      )}
      style={!detached && !collapsed ? { width: panelWidth } : undefined}
    >
      {collapsed ? (
        <button
          type="button"
          onClick={() => setCollapsed(false)}
          className="flex h-full w-full flex-col items-center gap-3 border-l-2 border-sky-400 bg-sky-50 py-3 text-sky-600 transition-colors hover:bg-sky-100 dark:bg-sky-950/30 dark:text-sky-400 dark:hover:bg-sky-900/40"
          title="展开案件 AI 助手"
          aria-label="展开聊天面板"
        >
          <Sparkles className="size-4" />
          <span
            className="font-medium tracking-wider"
            style={{ writingMode: "vertical-rl", textOrientation: "upright" }}
          >
            AI 助手
          </span>
        </button>
      ) : (
        <>
          {!detached && (
            <div
              onMouseDown={startResize}
              className="absolute left-0 top-0 z-20 h-full w-1 cursor-col-resize bg-transparent transition-colors hover:bg-primary/30"
              title="拖动调整宽度"
              aria-hidden
            />
          )}
          {/* Header */}
          <header className="flex items-center justify-between border-b border-border px-3 py-2.5">
        <div className="flex min-w-0 items-center gap-2">
          <Sparkles className="size-4 shrink-0 text-foreground" />
          <h2 className="truncate text-sm font-medium text-foreground">
            案件 AI 助手
          </h2>
          {caseName && (
            <span className="truncate text-xs text-muted-foreground">
              · {caseName}
            </span>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-0.5">
          <button
            type="button"
            onClick={clearAll}
            disabled={!caseId || isStreaming || history.length === 0}
            className="rounded p-1 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive disabled:cursor-not-allowed disabled:opacity-30"
            title="清空当前案件的聊天记录"
            aria-label="清空聊天记录"
          >
            <Trash2 className="size-3.5" />
          </button>
          {!detached && caseId && (
            <button
              type="button"
              onClick={detachChatPanel}
              disabled={isStreaming}
              className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-30"
              title={
                isStreaming
                  ? "当前任务完成后可独立显示"
                  : "独立显示(可最大化或拖到外接屏)"
              }
              aria-label="将 AI 助手独立显示"
            >
              <ExternalLink className="size-3.5" />
            </button>
          )}
          {!detached && (
            <button
              type="button"
              onClick={() => setCollapsed(true)}
              className="rounded p-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              title="收起面板"
              aria-label="收起面板"
            >
              <ChevronRight className="size-3.5" />
            </button>
          )}
          {detached && (
            <button
              type="button"
              onClick={reattachChatPanel}
              className="ml-1 inline-flex items-center gap-1 rounded-md border border-border px-2 py-1 text-xs font-medium text-foreground transition-colors hover:bg-accent"
              title="把 AI 助手重新停靠到案件页右侧"
            >
              <ArrowLeft className="size-3.5" />
              放回侧栏
            </button>
          )}
        </div>
      </header>

      {/* Messages 区:外层 relative 容器承载「回到底部」浮钮(不随内容滚动);min-h-0 保证 flex 子项可滚 */}
      <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scrollerRef}
        onScroll={(e) => {
          const el = e.currentTarget;
          // 距底 < 80px 视为「停在底部」→ 恢复自动跟随;否则用户在上滚 → 暂停
          setAutoScroll(el.scrollHeight - el.scrollTop - el.clientHeight < 80);
        }}
        className="flex-1 space-y-3 overflow-y-auto px-3 py-3 text-sm"
      >
        {!caseId && (
          <p className="py-10 text-center text-xs text-muted-foreground">
            请先选择一个案件
          </p>
        )}
        {caseId && historyLoading && history.length === 0 && (
          <p className="py-10 text-center text-xs text-muted-foreground">
            读取聊天记录…
          </p>
        )}
        {caseId && !historyLoading && history.length === 0 && !isStreaming && (
          <div className="rounded-md border border-dashed border-border bg-background/40 px-3 py-4 text-xs text-muted-foreground">
            <p className="mb-2 font-medium text-foreground">
              你可以这样问当前案件:
            </p>
            <ul className="space-y-1">
              <li>· 这个案子的争议焦点是什么?</li>
              <li>· 帮我列出所有付款记录</li>
              <li>· 现有材料缺什么、对方可能怎么打?</li>
              <li>· 下面的快捷按钮也可以直接点(把鼠标停在按钮上看说明)</li>
            </ul>
          </div>
        )}

        {history.map((msg) => (
          <MessageBubble key={msg.id} msg={msg} />
        ))}

        {/* 流式中的 assistant 消息(来自模块级 registry,切走再回来也能恢复) */}
        {isStreaming && (
          <div className="flex animate-in flex-col items-start fade-in-0 slide-in-from-bottom-1 duration-300">
            <div className="max-w-[95%] rounded-lg bg-background px-3 py-2 text-foreground shadow-sm ring-1 ring-border">
              {/* 交错时间线:思考文字 → 调工具 → 继续文字 → 再调工具,按事件到达顺序 */}
              <ChatSegments
                segments={streamingSegments}
                live
                reasoningChars={streamingReasoningChars}
              />
              {streamingSegments.length === 0 &&
                (streamingReasoningChars > 0 ? (
                  <ReasoningIndicator chars={streamingReasoningChars} />
                ) : (
                  <TypingDots />
                ))}
              <span className="ml-1 inline-block h-3 w-1 animate-pulse bg-foreground/60 align-middle" />
            </div>
          </div>
        )}

        {/* V0.3 · 选项式追问卡片(模型调 ask_user 后)。流式中不显,等本轮落定再让用户点。 */}
        {pendingAsk && pendingAsk.length > 0 && !isStreaming && (
          <AskUserCard
            questions={pendingAsk}
            disabled={disabled}
            onSubmit={(text) => send(text, null)}
          />
        )}
      </div>
        {/* 上滚查看历史时浮现:一键平滑回到最新(onScroll 到底后自动恢复跟随) */}
        {!autoScroll && (
          <button
            type="button"
            onClick={() =>
              scrollerRef.current?.scrollTo({
                top: scrollerRef.current.scrollHeight,
                behavior: "smooth",
              })
            }
            className="absolute bottom-3 right-3 z-10 flex size-7 items-center justify-center rounded-full border border-border bg-card text-muted-foreground shadow-md transition-all hover:bg-accent hover:text-foreground active:scale-95 animate-in fade-in-0 zoom-in-95 duration-150"
            title="回到最新"
            aria-label="滚动到底部"
          >
            <ArrowDown className="size-3.5" />
          </button>
        )}
      </div>

      {/* 真错红条 */}
      {error && (
        <div className="border-t border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
          <p className="font-medium">出错了</p>
          <p className="mt-0.5 break-all font-mono">{error}</p>
        </div>
      )}

      {/* V0.2 D6-D7 · 已引用文档 chip 区(空数组自动隐藏) */}
      <AttachmentChips
        docs={attachedDocs}
        onRemove={(id) =>
          setAttachedDocIds((cur) => cur.filter((x) => x !== id))
        }
        disabled={isStreaming}
      />

      {/* 刑事 tab(domain="criminal"):AI 助手只保留「刑事深度分析」单 chip,隐藏其余民事 chip。
          方法论借鉴游初(Youchu)gutachten-criminal-case(Apache 2.0),tooltip 内署名。 */}
      {domain === "criminal" ? (
        <div className="flex flex-wrap gap-1 border-t border-border px-3 py-2">
          <QuickChip
            label="⚖️ 刑事深度分析"
            hint="三阶层犯罪论+鉴定式刑事深度分析:先确认候选罪名清单,再确认三阶层检视大纲(构成要件该当性→违法性→有责性),然后逐要件论证(法条逐条校验),落一份刑事深度分析报告。会停下来问你两次(推理模式)。方法论借鉴游初 gutachten-criminal-case(Apache 2.0)"
            onClick={() => send("", "criminal_deep_analysis")}
            disabled={disabled}
            className="border-amber-700/40 bg-amber-700/5 hover:bg-amber-700/15"
          />
        </div>
      ) : (
      /* 快捷任务 chip(V0.3.3:6 个功能单一的生成型 chip 已删 —— AI 助手已是 agent,
          用户直接打字提需求即可,它自己拆解、调工具、产出直答或可编辑文书)。
          现留 4 个工具/分析型(法律依据/模拟对抗/类案检索/深度分析)+ 2 个写文书入口。
          每个 chip 悬停弹即时说明气泡。「⚖️ 法律依据」按是否引用文档切 task_type 与说明。 */
      <div className="flex flex-wrap gap-1 border-t border-border px-3 py-2">
        <QuickChip
          label={LEGAL_BASIS_CHIP.label}
          hint={
            attachedDocIds.length > 0
              ? LEGAL_BASIS_CHIP.hintWithAttached
              : LEGAL_BASIS_CHIP.hintNoAttached
          }
          onClick={() =>
            send(
              "",
              attachedDocIds.length > 0 ? "verify_my_draft" : "compile_legal_basis",
            )
          }
          disabled={disabled}
          className="border-amber-500/40 bg-amber-500/5 hover:bg-amber-500/15"
        />
        <QuickChip
          label="🥊 模拟对抗"
          hint="站在对方律师立场推演他会怎么打、援引什么法条/类案,再给我方反驳点。庭前攻防演练用(推演,非法律意见)"
          onClick={() => send("", "simulate_opposition")}
          disabled={disabled}
          className="border-rose-500/40 bg-rose-500/5 hover:bg-rose-500/15"
        />
        <QuickChip
          label="🔍 类案检索"
          hint={similarCasesHint}
          onClick={() => send("", "find_similar_cases")}
          disabled={disabled}
          className="border-sky-500/40 bg-sky-500/5 hover:bg-sky-500/15"
        />
        <QuickChip
          label="🔬 深度分析"
          hint="请求权基础+鉴定式深度分析:先让你确认候选请求权清单,再确认分析大纲,然后逐要件论证(法条逐条校验),落一份深度分析报告。复杂疑难案件用(会停下来问你两次,推理模式)"
          onClick={() => send("", "deep_analysis")}
          disabled={disabled}
          className="border-violet-500/40 bg-violet-500/5 hover:bg-violet-500/15"
        />
        {/* V0.3 · 写文书一键入口(走 save_artifact 出「可编辑+可导出 Word」的正式文书,
            等同在聊天里喊「帮我起草起诉状」。缺关键信息时会弹选项卡片追问)。 */}
        <QuickChip
          label="📝 写起诉状"
          hint="根据本案材料起草一份正式民事起诉状,落成可编辑文书、可导出 Word(法律格式)。信息不全会先弹选项问你"
          onClick={() =>
            send("请根据本案已有材料,帮我起草一份民事起诉状。", null)
          }
          disabled={disabled}
          className="border-emerald-500/40 bg-emerald-500/5 hover:bg-emerald-500/15"
        />
        <QuickChip
          label="📋 写证据目录"
          hint="根据本案证据材料起草一份正式证据目录(表格形式),落成可编辑文书、可导出 Word"
          onClick={() =>
            send("请根据本案证据材料,帮我起草一份证据目录(表格形式)。", null)
          }
          disabled={disabled}
          className="border-emerald-500/40 bg-emerald-500/5 hover:bg-emerald-500/15"
        />
      </div>
      )}

      {/* 输入区 */}
      <div className="border-t border-border bg-background/30 p-3">
        <div className="flex items-end gap-2">
          {/* V0.2 D6-D7 · 📎 引用文档按钮 */}
          <Button
            size="icon"
            variant="ghost"
            onClick={() => setPickerOpen(true)}
            disabled={!caseId || isStreaming || caseDocs.length === 0}
            title={
              caseDocs.length === 0
                ? "当前案件还没有可引用的文档"
                : "📎 引用文档(让 AI 重点读这几份)"
            }
            className="shrink-0 self-start"
            aria-label="选择引用文档"
          >
            <Paperclip className="size-4" />
          </Button>
          <textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault();
                send(input, null);
              }
            }}
            disabled={disabled}
            placeholder={
              caseId
                ? "向当前案件提问…(Enter 发送,Shift+Enter 换行)"
                : "请先选择一个案件"
            }
            rows={2}
            className="min-h-[44px] flex-1 resize-none rounded-md border border-border bg-background px-2 py-1.5 text-sm text-foreground placeholder:text-muted-foreground/60 transition-[border-color,box-shadow] focus:border-foreground focus:outline-none focus:ring-1 focus:ring-foreground/20 disabled:cursor-not-allowed disabled:opacity-50"
          />
          {isStreaming ? (
            <Button
              size="sm"
              variant="outline"
              onClick={stop}
              title="停止生成"
              className="shrink-0"
            >
              <CircleStop className="size-3.5" />
              停止
            </Button>
          ) : (
            <Button
              size="sm"
              onClick={() => send(input, null)}
              disabled={disabled || !input.trim()}
              title="发送(Enter)"
              className="shrink-0"
            >
              <Send className="size-3.5" />
              发送
            </Button>
          )}
        </div>
        <p className="mt-1.5 text-caption text-muted-foreground/70">
          AI 回答基于已抽取的案件材料。不得作为法律意见,关键判断请律师把关。
        </p>
      </div>

      {/* V0.2 D6-D7 · 引用文档选择 modal */}
      <AttachmentPicker
        open={pickerOpen}
        docs={caseDocs}
        initialSelected={attachedDocIds}
        onClose={() => setPickerOpen(false)}
        onConfirm={(ids) => {
          setAttachedDocIds(ids);
          setPickerOpen(false);
        }}
      />
        </>
      )}
    </aside>
  );
}

// =============================================================================
// 子组件
// =============================================================================

const MessageBubble = memo(function MessageBubble({ msg }: { msg: ChatMessage }) {
  // V0.2 D6-D7 · history 重放时把 citations_json parse 出来给 CitationsCard
  const citations = useMemo<Citation[]>(() => {
    if (!msg.citations_json) return [];
    try {
      const parsed = JSON.parse(msg.citations_json);
      return Array.isArray(parsed) ? (parsed as Citation[]) : [];
    } catch {
      return [];
    }
  }, [msg.citations_json]);

  if (msg.role === "user") {
    return (
      <div className="flex animate-in justify-end fade-in-0 slide-in-from-bottom-1 duration-300">
        <div className="max-w-[85%] whitespace-pre-wrap rounded-lg bg-foreground px-3 py-2 text-sm text-background">
          {msg.content}
        </div>
      </div>
    );
  }
  if (msg.role === "assistant") {
    // 错误行:content 为空 + error_short 有值
    if (!msg.content && msg.error_short) {
      return (
        <div className="flex justify-start">
          <div className="max-w-[95%] rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
            <p className="font-medium">该次回答失败</p>
            <p className="mt-0.5 font-mono">{msg.error_short}</p>
          </div>
        </div>
      );
    }
    return (
      <div className="flex animate-in flex-col items-start fade-in-0 slide-in-from-bottom-1 duration-300">
        <div className="max-w-[95%] rounded-lg bg-background px-3 py-2 text-sm text-foreground shadow-sm ring-1 ring-border">
          <MarkdownView text={msg.content} />
          {/* V0.2 D6.5 · 引用卡(若 LLM 落了 <CITATIONS>) */}
          {citations.length > 0 && <CitationsCard citations={citations} />}
          {/* 出错中断但有半截内容:展示已生成部分 + 中断提示(防"全消失") */}
          {msg.error_short && (
            <div className="mt-2 border-t border-destructive/30 pt-1.5 text-label text-destructive">
              ⚠ 该次回答未完成(出错中断):
              <span className="font-mono">{msg.error_short}</span>
            </div>
          )}
          {msg.artifact_doc_id && (
            <div className="mt-2 flex items-center gap-1.5 border-t border-border pt-1.5 text-label text-muted-foreground">
              <FileText className="size-3" />
              <span>已保存为 artifact</span>
            </div>
          )}
        </div>
        {/* meta */}
        <div className="mt-0.5 flex items-center gap-2 text-caption text-muted-foreground/60">
          {msg.model && <span>{msg.model}</span>}
          {msg.prompt_tokens !== null && msg.completion_tokens !== null && (
            <span>
              {msg.prompt_tokens}p / {msg.completion_tokens}c
            </span>
          )}
          {msg.latency_ms !== null && <span>{(msg.latency_ms / 1000).toFixed(1)}s</span>}
        </div>
      </div>
    );
  }
  return null;
});

const MarkdownView = memo(function MarkdownView({ text }: { text: string }) {
  return (
    <div
      className={cn(
        "min-w-0 max-w-none break-words font-sans text-sm leading-[1.7] text-foreground",
        "[&_h1]:mb-3 [&_h1]:mt-1 [&_h1]:text-center [&_h1]:text-xl [&_h1]:font-semibold",
        "[&_h2]:mb-2 [&_h2]:mt-4 [&_h2]:border-b [&_h2]:border-border [&_h2]:pb-1.5 [&_h2]:text-base [&_h2]:font-semibold",
        "[&_h3]:mb-1.5 [&_h3]:mt-3 [&_h3]:text-[15px] [&_h3]:font-semibold",
        "[&_h4]:mb-1 [&_h4]:mt-2.5 [&_h4]:text-sm [&_h4]:font-semibold",
        "[&_p]:my-2",
        "[&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5",
        "[&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5",
        "[&_li]:my-1 [&_li>p]:my-1",
        "[&_blockquote]:my-3 [&_blockquote]:border-l-2 [&_blockquote]:border-primary/40 [&_blockquote]:pl-3 [&_blockquote]:text-muted-foreground",
        "[&_hr]:my-4 [&_hr]:border-border",
        "[&_strong]:font-semibold",
        "[&_pre]:my-3 [&_pre]:max-w-full [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:border [&_pre]:border-border [&_pre]:bg-muted/60 [&_pre]:p-3 [&_pre]:text-xs [&_pre]:leading-relaxed",
        "[&_code]:rounded-sm [&_code]:bg-muted [&_code]:px-1 [&_code]:py-0.5 [&_code]:font-mono [&_code]:text-[12px]",
        "[&_pre_code]:whitespace-pre [&_pre_code]:bg-transparent [&_pre_code]:p-0",
        "[&_img]:max-w-full",
      )}
    >
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>
        {text}
      </ReactMarkdown>
    </div>
  );
});

type MarkdownComponents = ComponentProps<typeof ReactMarkdown>["components"];

const markdownComponents: MarkdownComponents = {
  a({ children, href, ...props }) {
    return (
      <a
        {...props}
        href={href}
        className="text-foreground underline decoration-border underline-offset-2 transition-colors hover:decoration-foreground"
        onClick={(event) => {
          event.preventDefault();
          if (!href) return;
          void openUrl(href).catch((e) =>
            console.warn("open markdown link failed", e),
          );
        }}
      >
        {children}
      </a>
    );
  },
  table({ children, ...props }) {
    return (
      <div className="my-3 w-full overflow-x-auto rounded-md border border-border bg-background">
        <table
          {...props}
          className="w-full min-w-max border-collapse text-left text-[13px] leading-relaxed"
        >
          {children}
        </table>
      </div>
    );
  },
  thead({ children, ...props }) {
    return (
      <thead {...props} className="bg-muted/70">
        {children}
      </thead>
    );
  },
  th({ children, ...props }) {
    return (
      <th
        {...props}
        className="border-b border-r border-border px-3 py-2 text-center text-label font-semibold text-muted-foreground last:border-r-0"
      >
        {children}
      </th>
    );
  },
  td({ children, ...props }) {
    return (
      <td
        {...props}
        className="border-b border-r border-border px-3 py-2 align-top last:border-r-0 [&_p]:my-1"
      >
        {children}
      </td>
    );
  },
  tr({ children, ...props }) {
    return (
      <tr {...props} className="last:[&_td]:border-b-0 last:[&_th]:border-b-0">
        {children}
      </tr>
    );
  },
};

// 把交错 segments 渲染成:text 段 → MarkdownView,连续 tool 段合并成一个 ToolCallTrace
// (同一轮的多个工具显示在一个 trace 框里)。live 时最后一组 trace 显示「正在思考下一步」。
function ChatSegments({
  segments,
  live = false,
  reasoningChars = 0,
}: {
  segments: ChatSegment[];
  live?: boolean;
  /** 末组 trace 的 live 行用:>0 显示「深度推理中…(N 字)」替代「正在思考下一步…」 */
  reasoningChars?: number;
}) {
  const blocks: (
    | { kind: "text"; text: string }
    | { kind: "tools"; records: ToolCallRecord[] }
  )[] = [];
  for (const seg of segments) {
    if (seg.kind === "text") {
      blocks.push({ kind: "text", text: seg.text });
    } else {
      const last = blocks[blocks.length - 1];
      if (last && last.kind === "tools") last.records.push(seg.record);
      else blocks.push({ kind: "tools", records: [seg.record] });
    }
  }
  return (
    <>
      {blocks.map((b, i) =>
        b.kind === "text" ? (
          <MarkdownView key={i} text={b.text} />
        ) : (
          <ToolCallTrace
            key={i}
            records={b.records}
            live={live && i === blocks.length - 1}
            reasoningChars={
              live && i === blocks.length - 1 ? reasoningChars : 0
            }
          />
        ),
      )}
    </>
  );
}

function formatError(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) {
    return String((e as { message: unknown }).message);
  }
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}
