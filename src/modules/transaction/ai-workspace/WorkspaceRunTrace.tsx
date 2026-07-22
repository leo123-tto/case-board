import { AiRunTrace } from "@/components/AiRunTrace";
import type { ChatActivity } from "@/lib/api";
import type { AgentRuntime } from "@/lib/piRuntime";

import type { AiWorkspaceMessageStatus, WorkspaceToolCallRecord } from "./types";

export function WorkspaceRunTrace({
  status,
  elapsedMs,
  reasoningObserved,
  toolCalls,
  activities = [],
  runtimeHint,
}: {
  status: AiWorkspaceMessageStatus;
  elapsedMs: number;
  reasoningObserved: boolean;
  toolCalls: WorkspaceToolCallRecord[];
  activities?: ChatActivity[];
  runtimeHint?: AgentRuntime | null;
}) {
  return (
    <AiRunTrace
      status={status}
      elapsedMs={elapsedMs}
      reasoningObserved={reasoningObserved}
      toolCalls={toolCalls}
      activities={activities}
      runtimeHint={runtimeHint}
    />
  );
}
