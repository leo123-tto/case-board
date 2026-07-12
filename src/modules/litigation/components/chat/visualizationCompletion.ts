import type { ToolCallRecord } from "@/lib/types";

import type { VisualWorkspace } from "../visualization/types";

const VISUAL_MUTATION_TOOLS = new Set([
  "save_case_visualization",
  "propose_case_visual_update",
]);

export function hasSuccessfulVisualMutation(
  toolCalls: ToolCallRecord[] | undefined,
): boolean {
  return toolCalls?.some(
    (tool) => VISUAL_MUTATION_TOOLS.has(tool.tool) && tool.success,
  ) ?? false;
}

export async function loadWorkspaceAfterVisualMutation(
  toolCalls: ToolCallRecord[] | undefined,
  refreshSummaries: () => Promise<void>,
  loadWorkspace: () => Promise<VisualWorkspace | null>,
): Promise<VisualWorkspace | null> {
  if (!hasSuccessfulVisualMutation(toolCalls)) return null;
  await refreshSummaries();
  return loadWorkspace();
}
