import { randomBytes, randomUUID } from "node:crypto";

import type { AgentToolResult } from "@earendil-works/pi-coding-agent";

import type { HostToolDefinition, JsonObject } from "../protocol";

type ToolResult = AgentToolResult<{ kb_hit: boolean; credits_used: number }>;

export interface SubagentToolBrokerOptions {
  tools: HostToolDefinition[];
  agents: Array<{ name: string; tools: string[] }>;
  execute(id: string, tool: string, args: JsonObject): Promise<ToolResult>;
}

export interface SubagentToolBroker {
  url: string;
  token: string;
  close(): Promise<void>;
}

const MAX_BODY_BYTES = 128 * 1024;

function json(value: unknown, status = 200): Response {
  return Response.json(value, {
    status,
    headers: { "cache-control": "no-store" },
  });
}

function toolText(result: ToolResult): string {
  return result.content
    .filter((item): item is Extract<(typeof result.content)[number], { type: "text" }> => item.type === "text")
    .map((item) => item.text)
    .join("\n");
}

export function startSubagentToolBroker(options: SubagentToolBrokerOptions): SubagentToolBroker {
  const token = randomBytes(32).toString("base64url");
  const definitions = new Map(options.tools.map((tool) => [tool.name, tool]));
  const agentTools = new Map(
    options.agents.map((agent) => [agent.name, new Set(agent.tools.filter((name) => definitions.has(name)))]),
  );
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: 0,
    async fetch(request) {
      if (request.headers.get("authorization") !== `Bearer ${token}`) {
        return json({ error: "unauthorized" }, 401);
      }
      const url = new URL(request.url);
      const manifestMatch = url.pathname.match(/^\/v1\/manifest\/([A-Za-z0-9._-]+)$/);
      if (request.method === "GET" && manifestMatch) {
        const allowed = agentTools.get(manifestMatch[1]!);
        if (!allowed) return json({ error: "unknown_agent" }, 404);
        return json({ tools: [...allowed].map((name) => definitions.get(name)!) });
      }

      const toolMatch = url.pathname.match(/^\/v1\/tools\/([A-Za-z0-9._-]+)\/([A-Za-z0-9._-]+)$/);
      if (request.method !== "POST" || !toolMatch) return json({ error: "not_found" }, 404);
      const [, agent, tool] = toolMatch;
      if (!agentTools.get(agent!)?.has(tool!)) return json({ error: "tool_not_allowed" }, 403);
      const declaredLength = Number(request.headers.get("content-length") ?? "0");
      if (Number.isFinite(declaredLength) && declaredLength > MAX_BODY_BYTES) {
        return json({ error: "request_too_large" }, 413);
      }
      const bodyText = await request.text();
      if (Buffer.byteLength(bodyText, "utf8") > MAX_BODY_BYTES) {
        return json({ error: "request_too_large" }, 413);
      }
      let body: { tool_call_id?: unknown; args?: unknown };
      try {
        body = JSON.parse(bodyText) as typeof body;
      } catch {
        return json({ error: "invalid_json" }, 400);
      }
      if (!body.args || typeof body.args !== "object" || Array.isArray(body.args)) {
        return json({ error: "invalid_args" }, 400);
      }
      try {
        const childCallId = typeof body.tool_call_id === "string" ? body.tool_call_id : "call";
        const result = await options.execute(
          `subagent:${agent}:${randomUUID()}:${childCallId.slice(0, 80)}`,
          tool!,
          body.args as JsonObject,
        );
        return json({
          content: toolText(result),
          kb_hit: result.details?.kb_hit ?? false,
          credits_used: result.details?.credits_used ?? 0,
        });
      } catch (error) {
        const message = error instanceof Error ? error.message : "tool execution failed";
        return json({ error: message.replace(/[\r\n\t]+/g, " ").slice(0, 500) }, 502);
      }
    },
  });

  return {
    url: `http://127.0.0.1:${server.port}`,
    token,
    async close() {
      await server.stop(true);
    },
  };
}
