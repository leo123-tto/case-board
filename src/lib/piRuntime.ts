export type AgentRuntime = "native" | "pi";

export function effectiveAgentRuntime(value: unknown): AgentRuntime {
  return typeof value === "string" && value.trim() === "pi" ? "pi" : "native";
}
