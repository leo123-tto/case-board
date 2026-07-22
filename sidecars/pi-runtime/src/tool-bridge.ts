import type { AgentToolResult } from "@earendil-works/pi-coding-agent";

import {
  PROTOCOL_VERSION,
  type JsonObject,
  type SidecarMessage,
  type ToolResultResponse,
} from "./protocol";

type PendingCall = {
  resolve: (result: AgentToolResult<{ kb_hit: boolean; credits_used: number }>) => void;
  reject: (error: Error) => void;
};

export class ToolBridge {
  private readonly pending = new Map<string, PendingCall>();

  constructor(
    private readonly requestId: string,
    private readonly emit: (message: SidecarMessage) => void,
  ) {}

  execute(
    toolCallId: string,
    tool: string,
    args: JsonObject,
  ): Promise<AgentToolResult<{ kb_hit: boolean; credits_used: number }>> {
    if (tool === "ask_user") {
      this.emit({
        type: "ask_user",
        protocol_version: PROTOCOL_VERSION,
        request_id: this.requestId,
        tool_call_id: toolCallId,
        args,
      });
      return Promise.resolve({
        content: [{ type: "text", text: "已向用户发起追问，等待用户下一条消息。" }],
        details: { kb_hit: false, credits_used: 0 },
        terminate: true,
      });
    }
    if (this.pending.has(toolCallId)) throw new Error("重复的 tool_call_id");
    const promise = new Promise<AgentToolResult<{ kb_hit: boolean; credits_used: number }>>(
      (resolve, reject) => this.pending.set(toolCallId, { resolve, reject }),
    );
    this.emit({
      type: "tool_request",
      protocol_version: PROTOCOL_VERSION,
      request_id: this.requestId,
      tool_call_id: toolCallId,
      tool,
      args,
    });
    return promise;
  }

  resolve(message: ToolResultResponse): boolean {
    if (message.request_id !== this.requestId) return false;
    const pending = this.pending.get(message.tool_call_id);
    if (!pending) return false;
    this.pending.delete(message.tool_call_id);
    if (message.is_error) {
      pending.reject(new Error(message.content || "工具执行失败"));
    } else {
      pending.resolve({
        content: [{ type: "text", text: message.content }],
        details: { kb_hit: message.kb_hit, credits_used: message.credits_used },
      });
    }
    return true;
  }

  cancel(reason = "Sidecar 已结束"): void {
    for (const pending of this.pending.values()) pending.reject(new Error(reason));
    this.pending.clear();
  }
}
