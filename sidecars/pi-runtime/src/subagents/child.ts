import { main, type ExtensionAPI, type ExtensionFactory } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

import type { HostToolDefinition, JsonObject } from "../protocol";
import registerSubagentPromptRuntime from "../vendor/pi-subagent-prompt-runtime.js";

export interface ChildBrokerConfig {
  url: string;
  token: string;
  agent: string;
}

function brokerHeaders(config: ChildBrokerConfig): Record<string, string> {
  return {
    authorization: `Bearer ${config.token}`,
    "content-type": "application/json",
  };
}

async function responseJson<T>(response: Response): Promise<T> {
  const value = await response.json() as T & { error?: unknown };
  if (!response.ok) {
    const message = typeof value?.error === "string" ? value.error : `broker HTTP ${response.status}`;
    throw new Error(message);
  }
  return value;
}

export function createBrokerToolExtension(config: ChildBrokerConfig): ExtensionFactory {
  return async (pi: ExtensionAPI) => {
    const manifestResponse = await fetch(
      `${config.url}/v1/manifest/${encodeURIComponent(config.agent)}`,
      { headers: brokerHeaders(config) },
    );
    const manifest = await responseJson<{ tools: HostToolDefinition[] }>(manifestResponse);
    for (const definition of manifest.tools) {
      pi.registerTool({
        name: definition.name,
        label: definition.name,
        description: definition.description,
        parameters: Type.Unsafe<JsonObject>(definition.parameters),
        executionMode: definition.mutating ? "sequential" : "parallel",
        execute: async (toolCallId, params, signal) => {
          const response = await fetch(
            `${config.url}/v1/tools/${encodeURIComponent(config.agent)}/${encodeURIComponent(definition.name)}`,
            {
              method: "POST",
              headers: brokerHeaders(config),
              body: JSON.stringify({ tool_call_id: toolCallId, args: params }),
              signal,
            },
          );
          const result = await responseJson<{ content: string; kb_hit: boolean; credits_used: number }>(response);
          return {
            content: [{ type: "text", text: result.content }],
            details: { kb_hit: result.kb_hit, credits_used: result.credits_used },
          };
        },
      });
    }
  };
}

const FORCED_FLAGS = [
  "--no-extensions",
  "--no-context-files",
  "--no-skills",
  "--no-prompt-templates",
  "--no-themes",
  "--no-approve",
  "--offline",
] as const;

export function sanitizeSubagentChildArgs(args: string[]): string[] {
  const sanitized: string[] = [];
  for (let index = 0; index < args.length; index++) {
    if (args[index] === "--extension" || args[index] === "-e") {
      index += 1;
      continue;
    }
    if (args[index] === "--approve" || args[index] === "-a") continue;
    sanitized.push(args[index]!);
  }
  for (const flag of FORCED_FLAGS) {
    if (!sanitized.includes(flag)) sanitized.push(flag);
  }
  return sanitized;
}

function childBrokerConfigFromEnv(): ChildBrokerConfig {
  const url = process.env.CASEBOARD_PI_SUBAGENT_BROKER_URL;
  const token = process.env.CASEBOARD_PI_SUBAGENT_BROKER_TOKEN;
  const agent = process.env.PI_SUBAGENT_CHILD_AGENT;
  if (!url || !token || !agent) throw new Error("CaseBoard 子代理缺少受限工具桥配置");
  const parsed = new URL(url);
  if (parsed.protocol !== "http:" || parsed.hostname !== "127.0.0.1") {
    throw new Error("CaseBoard 子代理工具桥必须绑定 127.0.0.1");
  }
  return { url: parsed.origin, token, agent };
}

export async function runSubagentChild(args = process.argv.slice(2)): Promise<void> {
  const broker = childBrokerConfigFromEnv();
  await main(sanitizeSubagentChildArgs(args), {
    extensionFactories: [
      { name: "pi-subagents-child-runtime", factory: registerSubagentPromptRuntime },
      { name: "caseboard-host-tools", factory: createBrokerToolExtension(broker) },
    ],
  });
}
