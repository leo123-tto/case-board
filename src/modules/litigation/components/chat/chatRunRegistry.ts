/**
 * 进行中 chat 运行的**模块级**登记表 —— 跨组件挂载/卸载存活。
 *
 * 解决的真实 bug(2026-05-31 作者实测):流式输出中点「首页」→ CaseChatPanel 卸载
 * → 监听器被清理 → 回到案件详情时「输出停止、没内容」;且因为看起来卡住了,作者
 * 重新点了一次任务 → 后台两次都在跑 → 出现两份 AI 摘要。
 *
 * 这里把"一次运行"的状态(累积文本 / tool 调用 / 完成态)从 React 组件里抽到模块级:
 *   - 监听器在这里挂,**不随面板卸载而断**,deltas 持续累积到 registry
 *   - 面板重新挂载时从 registry **重连**,恢复已累积的内容 + 继续看流式
 *   - 同一案件已有运行在跑时,**拦住重复点击**(根治重复摘要)
 *
 * 注:真正发起请求(invoke case_chat)仍由面板做;本 registry 只管「跨挂载存活的
 * 流式状态 + 监听器 + 去重锁」。后端本就会完成并落库,这里保证前端不丢显示、不误重发。
 */

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type { ChatActivity, ChatStreamEvent } from "@/lib/api";
import type { AgentRuntime } from "@/lib/piRuntime";
import type { ToolCallRecord } from "@/lib/types";

/** 流式时间线的一段:正文文字 或 一次工具调用。按事件到达顺序排列 → 实现
 *  「思考文字 → 调工具 → 继续文字 → 再调工具」交错展示(Claude Code 风格)。 */
export type ChatSegment =
  | { kind: "text"; text: string }
  | { kind: "tool"; record: ToolCallRecord };

export interface ChatRunState {
  caseId: string;
  conversationId: string;
  messageId: string;
  text: string;
  toolCalls: ToolCallRecord[];
  activities: ChatActivity[];
  /** 发起任务时已确定的 Runtime；activity 尚未到达时也不回退成含糊的 AI。 */
  runtime: AgentRuntime;
  /** 本轮开始时间，用于两处助手共用的紧凑执行耗时展示。 */
  startedAtMs: number;
  /** 交错时间线(渲染用);text/toolCalls 保留作兼容 + autoScroll 依赖。 */
  segments: ChatSegment[];
  /** thinking 模型本轮已累计的 reasoning_content 字数(深度推理进度反馈用,涨=活着没卡死)。 */
  reasoningChars: number;
  error: string | null;
  /** running = 流式进行中;done = 已结束(成功/失败),等面板收尾后清除 */
  status: "running" | "done";
}

interface InternalRun extends ChatRunState {
  unlisten: UnlistenFn | null;
}

/** caseId → 进行中的运行(一个案件同一时刻只允许一个运行) */
const runs = new Map<string, InternalRun>();
/** 订阅者(面板)回调,run 状态变化时通知 */
const subscribers = new Map<string, Set<() => void>>();

function notify(caseId: string) {
  subscribers.get(caseId)?.forEach((fn) => fn());
}

/**
 * 流式 delta 节流:token 到达频率很高,若每个 delta 都 notify → 面板每 token 全量重渲染
 * + MarkdownView 把不断增长的 streamingText 整段重新解析(长答案 O(n²) 卡顿,连历史气泡也跟着重渲染)。
 * 这里把 delta 触发的通知合并到约 60ms 一次(~16fps,肉眼仍顺滑);非 delta 事件(工具调用/
 * 错误/结束)仍即时 notify。finishRun/clearRun 清掉挂起定时器(其自身会做最终 notify)。
 */
const pendingNotify = new Map<string, ReturnType<typeof setTimeout>>();
const NOTIFY_THROTTLE_MS = 60;

function scheduleNotify(caseId: string) {
  if (pendingNotify.has(caseId)) return; // 窗口内已排期,delta 已累积进 text,无需重复排
  const timer = setTimeout(() => {
    pendingNotify.delete(caseId);
    notify(caseId);
  }, NOTIFY_THROTTLE_MS);
  pendingNotify.set(caseId, timer);
}

function clearPendingNotify(caseId: string) {
  const t = pendingNotify.get(caseId);
  if (t) {
    clearTimeout(t);
    pendingNotify.delete(caseId);
  }
}

/** 面板订阅某案件的运行状态变化。返回取消订阅函数。 */
export function subscribeRun(caseId: string, fn: () => void): () => void {
  let set = subscribers.get(caseId);
  if (!set) {
    set = new Set();
    subscribers.set(caseId, set);
  }
  set.add(fn);
  return () => {
    set?.delete(fn);
  };
}

/** 取某案件当前进行中/刚结束的运行快照(给面板渲染)。 */
export function getRun(caseId: string | null): ChatRunState | null {
  if (!caseId) return null;
  const r = runs.get(caseId);
  if (!r) return null;
  return {
    caseId: r.caseId,
    conversationId: r.conversationId,
    messageId: r.messageId,
    text: r.text,
    toolCalls: r.toolCalls,
    activities: r.activities,
    runtime: r.runtime,
    startedAtMs: r.startedAtMs,
    segments: r.segments,
    reasoningChars: r.reasoningChars,
    error: r.error,
    status: r.status,
  };
}

/** 该案件是否有运行正在进行(去重锁:面板据此拦重复点击)。 */
export function isRunning(caseId: string | null): boolean {
  if (!caseId) return false;
  return runs.get(caseId)?.status === "running";
}

/**
 * 登记并启动一次运行的**流式监听**(模块级,不随面板卸载而断)。
 * 调用方随后自行 invoke case_chat;完成时调 `finishRun`。
 * 若该案件已有运行在跑,返回 false(调用方应放弃本次发起)。
 */
export async function startRun(
  caseId: string,
  conversationId: string,
  messageId: string,
  runtime: AgentRuntime = "native",
): Promise<boolean> {
  if (isRunning(caseId)) return false;
  const run: InternalRun = {
    caseId,
    conversationId,
    messageId,
    text: "",
    toolCalls: [],
    activities: [],
    runtime,
    startedAtMs: Date.now(),
    segments: [],
    reasoningChars: 0,
    error: null,
    status: "running",
    unlisten: null,
  };
  runs.set(caseId, run);
  notify(caseId);

  try {
    run.unlisten = await listen<ChatStreamEvent>(
      `chat-stream-${messageId}`,
      (e) => {
        const cur = runs.get(caseId);
        if (!cur || cur.messageId !== messageId) return;
        const p = e.payload;
        if (p.kind === "activity") {
          cur.activities = [...cur.activities, p.activity];
          notify(caseId);
          return;
        }
        if (p.kind === "reasoning") {
          // thinking 模型推理增量:只累加字数做进度反馈。高频 → 节流通知 + 必须 return,
          // 否则落到末尾的即时 notify 会每个 token 全量重渲染(O(n²) 卡顿)。
          cur.reasoningChars += p.text.length;
          scheduleNotify(caseId);
          return;
        }
        if (p.kind === "delta") {
          cur.text += p.text;
          cur.reasoningChars = 0; // 开始吐正文 → 本段推理结束,清零(下一段思考重新计)
          // 追加到时间线最后一个 text 段(不可变替换以触发 memo 重渲染);否则起新 text 段
          const segs = cur.segments;
          const last = segs[segs.length - 1];
          if (last && last.kind === "text") {
            cur.segments = [
              ...segs.slice(0, -1),
              { kind: "text", text: last.text + p.text },
            ];
          } else {
            cur.segments = [...segs, { kind: "text", text: p.text }];
          }
          // 高频 token:节流通知(~60ms 合并),避免每 token 全量重渲染 + Markdown 重解析
          scheduleNotify(caseId);
          return;
        }
        if (p.kind === "tool_call") {
          cur.toolCalls = [...cur.toolCalls, p.record];
          cur.segments = [...cur.segments, { kind: "tool", record: p.record }];
          cur.reasoningChars = 0; // 本轮已出工具调用 → 清零,下一轮思考重新计
        } else if (p.kind === "error") {
          cur.error = p.message;
        }
        notify(caseId); // 非 delta(工具调用/错误)即时通知
      },
    );
  } catch {
    // 监听挂不上不致命:后端仍会落库,面板靠 finishRun 后刷历史兜底
    run.unlisten = null;
  }
  return true;
}

/** 运行结束(成功/失败)。标记 done + 解绑监听。面板刷完历史后调 clearRun 移除。 */
export function finishRun(caseId: string, error?: string | null) {
  const r = runs.get(caseId);
  if (!r) return;
  r.status = "done";
  if (error) r.error = error;
  r.unlisten?.();
  r.unlisten = null;
  clearPendingNotify(caseId); // 取消挂起的节流通知,下面立即做最终 notify(含完整文本)
  notify(caseId);
}

/** 面板已把结果并入历史,移除 registry 记录。 */
export function clearRun(caseId: string) {
  const r = runs.get(caseId);
  if (r) {
    r.unlisten?.();
    clearPendingNotify(caseId);
    runs.delete(caseId);
    notify(caseId);
  }
}
