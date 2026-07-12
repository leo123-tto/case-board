import type { DashboardAssistantContext } from "@/lib/api";
import type { StatusId } from "@/modules/litigation/lib/inferStatus";

interface DashboardCaseState {
  statusId: StatusId;
  statusLabel: string;
  isCriminal: boolean;
}

interface DashboardDocumentState {
  extractionStatus: string;
  deleted: boolean;
}

interface BuildDashboardAssistantContextInput {
  cases: DashboardCaseState[];
  documents: DashboardDocumentState[];
  openTodoCount: number;
  reminderUrgencies: Array<"overdue" | "urgent" | "normal">;
  snapshotComplete: boolean;
  capturedAt?: string;
}

/**
 * 首页看板助手数据快照的唯一构造入口。
 * 只接收安全聚合字段，避免把案件名、当事人、案号、金额、路径或正文传给产品助手。
 */
export function buildDashboardAssistantContext({
  cases,
  documents,
  openTodoCount,
  reminderUrgencies,
  snapshotComplete,
  capturedAt = new Date().toISOString(),
}: BuildDashboardAssistantContextInput): DashboardAssistantContext {
  const openCaseCount = cases.filter(({ statusId }) => statusId !== "closed").length;
  const statusCounts = cases.reduce<Record<string, number>>((counts, item) => {
    counts[item.statusLabel] = (counts[item.statusLabel] ?? 0) + 1;
    return counts;
  }, {});
  const liveDocuments = documents.filter(({ deleted }) => !deleted);

  return {
    total_case_count: cases.length,
    open_case_count: openCaseCount,
    closed_case_count: cases.length - openCaseCount,
    criminal_case_count: cases.filter(({ isCriminal }) => isCriminal).length,
    execution_case_count: cases.filter(({ statusId }) => statusId === "execution").length,
    status_counts: statusCounts,
    document_count: liveDocuments.length,
    pending_document_count: liveDocuments.filter(
      ({ extractionStatus }) => extractionStatus === "pending",
    ).length,
    processing_document_count: liveDocuments.filter(
      ({ extractionStatus }) => extractionStatus === "processing",
    ).length,
    failed_document_count: liveDocuments.filter(
      ({ extractionStatus }) => extractionStatus === "failed",
    ).length,
    open_todo_count: openTodoCount,
    upcoming_event_count: reminderUrgencies.length,
    urgent_reminder_count: reminderUrgencies.filter((urgency) => urgency === "urgent").length,
    overdue_reminder_count: reminderUrgencies.filter((urgency) => urgency === "overdue").length,
    snapshot_complete: snapshotComplete,
    captured_at: capturedAt,
  };
}
