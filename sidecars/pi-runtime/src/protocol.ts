import type { Credential, ModelThinkingLevel } from "@earendil-works/pi-ai";

export const PROTOCOL_VERSION = 3 as const;
export const MAX_LINE_BYTES = 16 * 1024 * 1024;

export type JsonObject = Record<string, unknown>;

export interface HistoryMessage {
  role: "user" | "assistant";
  content: string;
}

export interface HostToolDefinition {
  name: string;
  description: string;
  parameters: JsonObject;
  mutating: boolean;
}

export interface HostSkillDefinition {
  name: string;
  description: string;
  file_path: string;
  base_dir: string;
  source: string;
  version: string;
  sha256: string;
}

export interface HostModelConfig {
  provider_id: string;
  model_id: string;
  thinking_level?: ModelThinkingLevel;
  credential?: Credential;
  caseboard_custom?: {
    base_url: string;
    auth_header: boolean;
    reasoning: boolean;
    context_window: number;
    max_tokens: number;
    temperature: number;
    headers?: Record<string, string>;
    compat?: JsonObject;
  };
}

export interface StartRequest {
  type: "start";
  protocol_version: typeof PROTOCOL_VERSION;
  request_id: string;
  system_prompt: string;
  history: HistoryMessage[];
  user_message: string;
  model: HostModelConfig;
  tools: HostToolDefinition[];
  skills: HostSkillDefinition[];
}

export interface ToolResultResponse {
  type: "tool_result";
  protocol_version: typeof PROTOCOL_VERSION;
  request_id: string;
  tool_call_id: string;
  content: string;
  is_error: boolean;
  kb_hit: boolean;
  credits_used: number;
}

export interface CancelRequest {
  type: "cancel";
  protocol_version: typeof PROTOCOL_VERSION;
  request_id: string;
}

export interface SteeringRequest {
  type: "steer";
  protocol_version: typeof PROTOCOL_VERSION;
  request_id: string;
  content: string;
}

export interface HealthCheckRequest {
  type: "health_check";
  protocol_version: typeof PROTOCOL_VERSION;
}

export interface CatalogRequest {
  type: "catalog_request";
  protocol_version: typeof PROTOCOL_VERSION;
}

export interface AuthStartRequest {
  type: "auth_start";
  protocol_version: typeof PROTOCOL_VERSION;
  request_id: string;
  provider_id: string;
  auth_type: "api_key" | "oauth";
}

export interface AuthPromptResponse {
  type: "auth_prompt_response";
  protocol_version: typeof PROTOCOL_VERSION;
  request_id: string;
  prompt_id: string;
  value: string;
}

export interface AuthCancelRequest {
  type: "auth_cancel";
  protocol_version: typeof PROTOCOL_VERSION;
  request_id: string;
}

export interface PiModelSummary {
  id: string;
  name: string;
  api: string;
  reasoning: boolean;
  thinking_levels: ModelThinkingLevel[];
  context_window: number;
  max_tokens: number;
  input: string[];
}

export interface PiProviderSummary {
  id: string;
  name: string;
  auth_types: Array<"api_key" | "oauth">;
  models: PiModelSummary[];
}

export interface PiProviderCatalog {
  providers: PiProviderSummary[];
}

export type HostMessage =
  | StartRequest
  | ToolResultResponse
  | CancelRequest
  | SteeringRequest
  | HealthCheckRequest
  | CatalogRequest
  | AuthStartRequest
  | AuthPromptResponse
  | AuthCancelRequest;

export type SidecarMessage =
  | {
      type: "ready";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      sidecar_version: string;
      pi_sdk_version: string;
    }
  | { type: "turn_start"; protocol_version: typeof PROTOCOL_VERSION; request_id: string }
  | { type: "turn_end"; protocol_version: typeof PROTOCOL_VERSION; request_id: string; elapsed_ms: number }
  | { type: "delta"; protocol_version: typeof PROTOCOL_VERSION; request_id: string; content: string }
  | { type: "reasoning"; protocol_version: typeof PROTOCOL_VERSION; request_id: string; content: string }
  | {
      type: "retry_started";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      attempt: number;
      max_attempts: number;
      delay_ms: number;
      error_message: string;
    }
  | {
      type: "retry_finished";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      attempt: number;
      success: boolean;
      error_message?: string;
    }
  | {
      type: "tool_request";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      tool_call_id: string;
      tool: string;
      args: JsonObject;
    }
  | {
      type: "tool_complete";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      tool_call_id: string;
      tool: string;
      is_error: boolean;
    }
  | {
      type: "ask_user";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      tool_call_id: string;
      args: JsonObject;
    }
  | {
      type: "done";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      content: string;
      stop_reason: string;
      usage: {
        input: number;
        output: number;
        cache_read: number;
        cache_write: number;
        reasoning: number;
        total_tokens: number;
      };
    }
  | { type: "error"; protocol_version: typeof PROTOCOL_VERSION; request_id?: string; code: string; message: string }
  | {
      type: "health";
      protocol_version: typeof PROTOCOL_VERSION;
      sidecar_version: string;
      pi_sdk_version: string;
      platform: string;
      arch: string;
      capabilities: {
        subagents: {
          package: "pi-subagents";
          version: "0.35.1";
          child_mode: true;
        };
      };
    }
  | {
      type: "catalog";
      protocol_version: typeof PROTOCOL_VERSION;
      providers: PiProviderSummary[];
    }
  | {
      type: "credential_update";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      provider_id: string;
      credential: Credential;
    }
  | {
      type: "auth_prompt";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      prompt_id: string;
      prompt_type: "text" | "secret" | "select" | "manual_code";
      message: string;
      placeholder?: string;
      options?: Array<{ id: string; label: string; description?: string }>;
    }
  | {
      type: "auth_info";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      message: string;
      links?: Array<{ url: string; label?: string }>;
    }
  | {
      type: "auth_url";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      url: string;
      instructions?: string;
    }
  | {
      type: "auth_device_code";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      user_code: string;
      verification_uri: string;
      interval_seconds?: number;
      expires_in_seconds?: number;
    }
  | {
      type: "auth_progress";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      message: string;
    }
  | {
      type: "auth_success";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      provider_id: string;
      credential: Credential;
    }
  | {
      type: "auth_error";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
      message: string;
    }
  | {
      type: "auth_cancelled";
      protocol_version: typeof PROTOCOL_VERSION;
      request_id: string;
    };

function object(value: unknown, field: string): JsonObject {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error(`${field} 必须是对象`);
  }
  return value as JsonObject;
}

function string(value: unknown, field: string, allowEmpty = false): string {
  if (typeof value !== "string" || (!allowEmpty && value.trim().length === 0)) {
    throw new Error(`${field} 必须是非空字符串`);
  }
  return value;
}

function number(value: unknown, field: string, minimum = 0): number {
  if (typeof value !== "number" || !Number.isFinite(value) || value < minimum) {
    throw new Error(`${field} 必须是有效数字`);
  }
  return value;
}

function boolean(value: unknown, field: string): boolean {
  if (typeof value !== "boolean") throw new Error(`${field} 必须是布尔值`);
  return value;
}

function protocolVersion(value: unknown): typeof PROTOCOL_VERSION {
  if (value !== PROTOCOL_VERSION) throw new Error("不支持的协议版本");
  return PROTOCOL_VERSION;
}

function thinkingLevel(value: unknown, field: string): ModelThinkingLevel {
  if (
    value !== "off"
    && value !== "minimal"
    && value !== "low"
    && value !== "medium"
    && value !== "high"
    && value !== "xhigh"
    && value !== "max"
  ) {
    throw new Error(`${field} 不是 Pi 支持的推理强度`);
  }
  return value;
}

function stringMap(value: unknown, field: string): Record<string, string> {
  const record = object(value, field);
  return Object.fromEntries(
    Object.entries(record).map(([key, item]) => [key, string(item, `${field}.${key}`, true)]),
  );
}

function parseCredential(value: unknown, field: string): Credential {
  const credential = object(value, field);
  if (credential.type === "api_key") {
    const key = credential.key === undefined ? undefined : string(credential.key, `${field}.key`, true);
    const env = credential.env === undefined ? undefined : stringMap(credential.env, `${field}.env`);
    return { type: "api_key", ...(key === undefined ? {} : { key }), ...(env === undefined ? {} : { env }) };
  }
  if (credential.type === "oauth") {
    return {
      ...credential,
      type: "oauth",
      access: string(credential.access, `${field}.access`, true),
      refresh: string(credential.refresh, `${field}.refresh`, true),
      expires: number(credential.expires, `${field}.expires`),
    };
  }
  throw new Error(`${field}.type 只允许 api_key 或 oauth`);
}

function parseStart(value: JsonObject): StartRequest {
  if (!Array.isArray(value.history)) throw new Error("history 必须是数组");
  if (!Array.isArray(value.tools)) throw new Error("tools 必须是数组");
  if (!Array.isArray(value.skills)) throw new Error("skills 必须是数组");
  const model = object(value.model, "model");
  const history = value.history.map((item, index) => {
    const entry = object(item, `history[${index}]`);
    if (entry.role !== "user" && entry.role !== "assistant") {
      throw new Error(`history[${index}].role 只允许 user 或 assistant`);
    }
    const role: "user" | "assistant" = entry.role;
    return { role, content: string(entry.content, `history[${index}].content`, true) };
  });
  const tools = value.tools.map((item, index) => {
    const entry = object(item, `tools[${index}]`);
    return {
      name: string(entry.name, `tools[${index}].name`),
      description: string(entry.description, `tools[${index}].description`, true),
      parameters: object(entry.parameters, `tools[${index}].parameters`),
      mutating: boolean(entry.mutating, `tools[${index}].mutating`),
    };
  });
  const skills = value.skills.map((item, index) => {
    const entry = object(item, `skills[${index}]`);
    return {
      name: string(entry.name, `skills[${index}].name`),
      description: string(entry.description, `skills[${index}].description`),
      file_path: string(entry.file_path, `skills[${index}].file_path`),
      base_dir: string(entry.base_dir, `skills[${index}].base_dir`),
      source: string(entry.source, `skills[${index}].source`),
      version: string(entry.version, `skills[${index}].version`),
      sha256: string(entry.sha256, `skills[${index}].sha256`),
    };
  });
  return {
    type: "start",
    protocol_version: protocolVersion(value.protocol_version),
    request_id: string(value.request_id, "request_id"),
    system_prompt: string(value.system_prompt, "system_prompt", true),
    history,
    user_message: string(value.user_message, "user_message"),
    model: {
      provider_id: string(model.provider_id, "model.provider_id"),
      model_id: string(model.model_id, "model.model_id"),
      ...(model.thinking_level === undefined
        ? {}
        : { thinking_level: thinkingLevel(model.thinking_level, "model.thinking_level") }),
      ...(model.credential === undefined
        ? {}
        : { credential: parseCredential(model.credential, "model.credential") }),
      ...(model.caseboard_custom === undefined
        ? {}
        : (() => {
            const custom = object(model.caseboard_custom, "model.caseboard_custom");
            return {
              caseboard_custom: {
                base_url: string(custom.base_url, "model.caseboard_custom.base_url"),
                auth_header: boolean(custom.auth_header, "model.caseboard_custom.auth_header"),
                reasoning: boolean(custom.reasoning, "model.caseboard_custom.reasoning"),
                context_window: number(custom.context_window, "model.caseboard_custom.context_window", 1),
                max_tokens: number(custom.max_tokens, "model.caseboard_custom.max_tokens", 1),
                temperature: number(custom.temperature, "model.caseboard_custom.temperature"),
                ...(custom.headers === undefined
                  ? {}
                  : { headers: stringMap(custom.headers, "model.caseboard_custom.headers") }),
                ...(custom.compat === undefined
                  ? {}
                  : { compat: object(custom.compat, "model.caseboard_custom.compat") }),
              },
            };
          })()),
    },
    tools,
    skills,
  };
}

export function parseHostMessage(line: string): HostMessage {
  if (Buffer.byteLength(line, "utf8") > MAX_LINE_BYTES) throw new Error("消息过大");
  let parsed: unknown;
  try {
    parsed = JSON.parse(line);
  } catch {
    throw new Error("JSON 消息格式无效");
  }
  const value = object(parsed, "message");
  const type = string(value.type, "type");
  const version = protocolVersion(value.protocol_version);
  if (type === "health_check") {
    return { type, protocol_version: version };
  }
  if (type === "catalog_request") {
    return { type, protocol_version: version };
  }
  if (type === "auth_start") {
    if (value.auth_type !== "api_key" && value.auth_type !== "oauth") {
      throw new Error("auth_type 只允许 api_key 或 oauth");
    }
    return {
      type,
      protocol_version: version,
      request_id: string(value.request_id, "request_id"),
      provider_id: string(value.provider_id, "provider_id"),
      auth_type: value.auth_type,
    };
  }
  if (type === "auth_prompt_response") {
    return {
      type,
      protocol_version: version,
      request_id: string(value.request_id, "request_id"),
      prompt_id: string(value.prompt_id, "prompt_id"),
      value: string(value.value, "value", true),
    };
  }
  if (type === "auth_cancel") {
    return {
      type,
      protocol_version: version,
      request_id: string(value.request_id, "request_id"),
    };
  }
  if (type === "start") return parseStart(value);
  if (type === "cancel") {
    return {
      type,
      protocol_version: version,
      request_id: string(value.request_id, "request_id"),
    };
  }
  if (type === "steer") {
    return {
      type,
      protocol_version: version,
      request_id: string(value.request_id, "request_id"),
      content: string(value.content, "content"),
    };
  }
  if (type === "tool_result") {
    return {
      type,
      protocol_version: version,
      request_id: string(value.request_id, "request_id"),
      tool_call_id: string(value.tool_call_id, "tool_call_id"),
      content: string(value.content, "content", true),
      is_error: boolean(value.is_error, "is_error"),
      kb_hit: boolean(value.kb_hit, "kb_hit"),
      credits_used: number(value.credits_used, "credits_used"),
    };
  }
  throw new Error("不支持的消息类型");
}
