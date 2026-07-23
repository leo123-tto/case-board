import type { AiWorkspaceTask } from "./types";

const ACTIVE_TASK_STATUSES = new Set<AiWorkspaceTask["status"]>(["queued", "streaming"]);

function parseWorkspaceTimestamp(value: string | null): number | null {
  if (!value) return null;
  const hasTimezone = /(?:Z|[+-]\d{2}:?\d{2})$/i.test(value);
  const iso = value.includes("T") ? value : value.replace(" ", "T");
  const parsed = Date.parse(hasTimezone ? iso : `${iso}Z`);
  return Number.isFinite(parsed) ? parsed : null;
}

export function workspaceTaskStartedAtMs(task: AiWorkspaceTask): number | null {
  return parseWorkspaceTimestamp(task.created_at);
}

export function findActiveWorkspaceTask(tasks: AiWorkspaceTask[]): AiWorkspaceTask | null {
  return tasks.reduce<AiWorkspaceTask | null>((latest, task) => {
    if (!ACTIVE_TASK_STATUSES.has(task.status)) return latest;
    if (!latest) return task;
    return (workspaceTaskStartedAtMs(task) ?? 0) >= (workspaceTaskStartedAtMs(latest) ?? 0)
      ? task
      : latest;
  }, null);
}

export function workspaceTaskElapsedMs(task: AiWorkspaceTask, nowMs = Date.now()): number {
  const startedAt = workspaceTaskStartedAtMs(task);
  if (startedAt === null) return 0;
  const finishedAt = ACTIVE_TASK_STATUSES.has(task.status)
    ? nowMs
    : parseWorkspaceTimestamp(task.finished_at) ?? parseWorkspaceTimestamp(task.updated_at) ?? nowMs;
  return Math.max(0, finishedAt - startedAt);
}
