import {
  clampThinkingLevel,
  type Api,
  type AssistantMessage,
  type Message,
  type Model,
  type ModelThinkingLevel,
} from "@earendil-works/pi-ai";
import {
  createAgentSession,
  defineTool,
  SessionManager,
  SettingsManager,
  type AgentSession,
  type AgentSessionEvent,
  type ToolDefinition,
} from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

import { createRequestModel } from "./model";
import {
  PROTOCOL_VERSION,
  type HistoryMessage,
  type HostToolDefinition,
  type JsonObject,
  type SidecarMessage,
  type StartRequest,
} from "./protocol";
import { createIsolatedResourceLoader, loadCaseBoardSubagentResources } from "./resources";
import { startSubagentToolBroker, type SubagentToolBroker } from "./subagents/broker";
import {
  configureSubagentCredential,
  createSubagentRuntimeEnvironment,
  type SubagentRuntimeEnvironment,
} from "./subagents/config";
import { ToolBridge } from "./tool-bridge";
import { PI_SDK_VERSION, SIDECAR_VERSION } from "./version";

const RESEARCH_TOOLS = new Set([
  "exa_search",
  "exa_contents",
  "exa_find_similar",
  "firecrawl_search",
  "firecrawl_scrape",
]);

const SUBAGENT_SYSTEM_PROMPT = `

<caseboard_subagents>
你可以使用原版 pi-subagents 的 subagent 工具，把复杂研究拆给 legal-researcher、source-reader、legal-analyst、source-verifier。适合并行检索、逐篇深读、交叉核验与归纳；简单任务直接完成即可。子代理继承父任务边界，只能调用它角色获准的 Exa/Firecrawl 工具，不能使用终端、修改原始材料或继续派生子代理。不要把当事人身份、案件原文、绝对路径、API Key 或 token 放进分派任务或联网检索词。
</caseboard_subagents>`;

const MAX_DIAGNOSTIC_ERROR_CHARS = 2000;

function truncateDiagnosticError(value: string | undefined): string | undefined {
  if (!value) return undefined;
  return Array.from(value).slice(0, MAX_DIAGNOSTIC_ERROR_CHARS).join("");
}

export function retrySidecarMessage(
  requestId: string,
  event: Extract<AgentSessionEvent, { type: "auto_retry_start" | "auto_retry_end" }>,
): Extract<SidecarMessage, { type: "retry_started" | "retry_finished" }> {
  if (event.type === "auto_retry_start") {
    return {
      type: "retry_started",
      protocol_version: PROTOCOL_VERSION,
      request_id: requestId,
      attempt: event.attempt,
      max_attempts: event.maxAttempts,
      delay_ms: event.delayMs,
      error_message: truncateDiagnosticError(event.errorMessage) ?? "Unknown error",
    };
  }
  return {
    type: "retry_finished",
    protocol_version: PROTOCOL_VERSION,
    request_id: requestId,
    attempt: event.attempt,
    success: event.success,
    ...(truncateDiagnosticError(event.finalError)
      ? { error_message: truncateDiagnosticError(event.finalError) }
      : {}),
  };
}

export function createReadyMessage(
  requestId: string,
): Extract<SidecarMessage, { type: "ready" }> {
  return {
    type: "ready",
    protocol_version: PROTOCOL_VERSION,
    request_id: requestId,
    sidecar_version: SIDECAR_VERSION,
    pi_sdk_version: PI_SDK_VERSION,
  };
}

export function supportsResearchSubagents(input: {
  provider_id: string;
  tools: HostToolDefinition[];
}): boolean {
  return input.provider_id !== "caseboard-custom" && input.tools.some((tool) => RESEARCH_TOOLS.has(tool.name));
}

export function sessionToolAllowlist(
  tools: HostToolDefinition[],
  subagentsEnabled: boolean,
): string[] {
  const names = tools.map((tool) => tool.name);
  return subagentsEnabled ? [...names, "subagent"] : names;
}

const EMPTY_USAGE: AssistantMessage["usage"] = {
  input: 0,
  output: 0,
  cacheRead: 0,
  cacheWrite: 0,
  totalTokens: 0,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
};

export function buildHistoryMessages(history: HistoryMessage[], model: Model<Api>): Message[] {
  const startedAt = Date.now();
  return history.map((item, index): Message => {
    if (item.role === "user") {
      return { role: "user", content: item.content, timestamp: startedAt + index };
    }
    return {
      role: "assistant",
      content: [{ type: "text", text: item.content }],
      api: model.api,
      provider: model.provider,
      model: model.id,
      usage: { ...EMPTY_USAGE, cost: { ...EMPTY_USAGE.cost } },
      stopReason: "stop",
      timestamp: startedAt + index,
    };
  });
}

export function createCustomTools(
  definitions: HostToolDefinition[],
  bridge: ToolBridge,
): ToolDefinition[] {
  return definitions.map((definition) =>
    defineTool({
      name: definition.name,
      label: definition.name,
      description: definition.description,
      parameters: Type.Unsafe<JsonObject>(definition.parameters),
      executionMode: definition.mutating ? "sequential" : "parallel",
      execute: async (toolCallId, params) => bridge.execute(toolCallId, definition.name, params),
    }),
  );
}

function assistantText(message: AssistantMessage): string {
  return message.content
    .filter((item) => item.type === "text")
    .map((item) => item.text)
    .join("");
}

export function finalSessionResult(messages: Message[]): {
  content: string;
  stopReason: string;
  usage: Extract<SidecarMessage, { type: "done" }>["usage"];
} {
  const assistant = messages.filter((message): message is AssistantMessage => message.role === "assistant");
  const last = assistant.at(-1);
  if (last?.stopReason === "error") {
    throw new Error(last.errorMessage?.trim() || "Pi Runtime 模型请求失败");
  }
  return {
    content: assistant.map(assistantText).filter(Boolean).join("\n\n"),
    stopReason: last?.stopReason ?? "error",
    usage: assistant.reduce(
      (sum, message) => ({
        input: sum.input + message.usage.input,
        output: sum.output + message.usage.output,
        cache_read: sum.cache_read + message.usage.cacheRead,
        cache_write: sum.cache_write + message.usage.cacheWrite,
        reasoning: sum.reasoning + (message.usage.reasoning ?? 0),
        total_tokens: sum.total_tokens + message.usage.totalTokens,
      }),
      { input: 0, output: 0, cache_read: 0, cache_write: 0, reasoning: 0, total_tokens: 0 },
    ),
  };
}

export interface ActiveSessionControl {
  abort(): Promise<void>;
  steer(content: string): void;
}

export function resolveSessionThinkingLevel(
  model: Model<Api>,
  requested: ModelThinkingLevel | undefined,
): ModelThinkingLevel {
  return clampThinkingLevel(model, requested ?? (model.reasoning ? "medium" : "off"));
}

export async function runPiSession(
  request: StartRequest,
  bridge: ToolBridge,
  emit: (message: SidecarMessage) => void,
  setControl: (control: ActiveSessionControl | undefined) => void,
): Promise<void> {
  const { modelRuntime, model, credentialStore } = await createRequestModel(request.model);
  const customTools = createCustomTools(request.tools, bridge);
  const history = buildHistoryMessages(request.history, model);
  const settingsManager = SettingsManager.inMemory({
    compaction: { enabled: false },
    retry: { enabled: true, maxRetries: 2 },
    enableSkillCommands: true,
  });
  let session: AgentSession | undefined;
  let subagentRuntime: SubagentRuntimeEnvironment | undefined;
  let subagentBroker: SubagentToolBroker | undefined;
  let restoreSubagentEnvironment: (() => void) | undefined;
  let turnStartedAt = 0;
  const emitCredentialUpdate = () => {
    const credential = credentialStore.takeUpdate();
    if (!credential) return;
    emit({
      type: "credential_update",
      protocol_version: PROTOCOL_VERSION,
      request_id: request.request_id,
      provider_id: request.model.provider_id,
      credential,
    });
  };

  try {
    const subagentsEnabled = supportsResearchSubagents({
      provider_id: request.model.provider_id,
      tools: request.tools,
    });
    let loadedSubagentResources;
    if (subagentsEnabled) {
      const availableResearchTools = new Set(
        request.tools.map((tool) => tool.name).filter((name) => RESEARCH_TOOLS.has(name)),
      );
      subagentRuntime = createSubagentRuntimeEnvironment(process.execPath, availableResearchTools);
      configureSubagentCredential(
        subagentRuntime,
        request.model.provider_id,
        request.model.credential,
      );
      subagentBroker = startSubagentToolBroker({
        tools: request.tools.filter((tool) => RESEARCH_TOOLS.has(tool.name)),
        agents: subagentRuntime.agents,
        execute: (id, tool, args) => bridge.execute(id, tool, args),
      });
      subagentRuntime.env.CASEBOARD_PI_SUBAGENT_BROKER_URL = subagentBroker.url;
      subagentRuntime.env.CASEBOARD_PI_SUBAGENT_BROKER_TOKEN = subagentBroker.token;
      restoreSubagentEnvironment = subagentRuntime.activate();
      loadedSubagentResources = await loadCaseBoardSubagentResources(process.cwd());
    }
    ({ session } = await createAgentSession({
      model,
      modelRuntime,
      thinkingLevel: resolveSessionThinkingLevel(model, request.model.thinking_level),
      noTools: "builtin",
      tools: sessionToolAllowlist(request.tools, subagentsEnabled),
      customTools,
      resourceLoader: createIsolatedResourceLoader(
        `${request.system_prompt}${subagentsEnabled ? SUBAGENT_SYSTEM_PROMPT : ""}`,
        request.skills,
        loadedSubagentResources,
      ),
      sessionManager: SessionManager.inMemory(),
      settingsManager,
    }));
    session.agent.state.messages = history;
    setControl({
      abort: () => session!.abort(),
      steer: (content) => session!.steer(content),
    });
    session.subscribe((event) => {
      if (event.type === "turn_start") {
        turnStartedAt = Date.now();
        emit({
          type: "turn_start",
          protocol_version: PROTOCOL_VERSION,
          request_id: request.request_id,
        });
      } else if (event.type === "turn_end") {
        emit({
          type: "turn_end",
          protocol_version: PROTOCOL_VERSION,
          request_id: request.request_id,
          elapsed_ms: turnStartedAt > 0 ? Math.max(0, Date.now() - turnStartedAt) : 0,
        });
      } else if (event.type === "message_update") {
        if (event.assistantMessageEvent.type === "text_delta") {
          emit({
            type: "delta",
            protocol_version: PROTOCOL_VERSION,
            request_id: request.request_id,
            content: event.assistantMessageEvent.delta,
          });
        } else if (event.assistantMessageEvent.type === "thinking_delta") {
          emit({
            type: "reasoning",
            protocol_version: PROTOCOL_VERSION,
            request_id: request.request_id,
            content: event.assistantMessageEvent.delta,
          });
        }
      } else if (event.type === "tool_execution_end") {
        emit({
          type: "tool_complete",
          protocol_version: PROTOCOL_VERSION,
          request_id: request.request_id,
          tool_call_id: event.toolCallId,
          tool: event.toolName,
          is_error: event.isError,
        });
      } else if (event.type === "auto_retry_start" || event.type === "auto_retry_end") {
        emit(retrySidecarMessage(request.request_id, event));
      }
    });
    emit(createReadyMessage(request.request_id));
    await session.prompt(request.user_message, { expandPromptTemplates: true });

    const generated = session.agent.state.messages.slice(history.length) as Message[];
    const result = finalSessionResult(generated);
    emitCredentialUpdate();
    emit({
      type: "done",
      protocol_version: PROTOCOL_VERSION,
      request_id: request.request_id,
      content: result.content,
      stop_reason: result.stopReason,
      usage: result.usage,
    });
  } finally {
    emitCredentialUpdate();
    setControl(undefined);
    bridge.cancel();
    session?.dispose();
    try {
      await subagentBroker?.close();
    } finally {
      restoreSubagentEnvironment?.();
      subagentRuntime?.cleanup();
    }
  }
}
