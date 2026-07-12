import { useState } from "react";
import { ArrowUpRight, GitBranch, LoaderCircle, Sparkles } from "lucide-react";

import { Button } from "@/components/ui/button";
import { getCaseVisualWorkspace } from "@/lib/api";

import type {
  ViewKind,
  VisualWorkspace,
  VisualWorkspaceSummary,
} from "./types";

interface Props {
  caseId: string;
  summary: VisualWorkspaceSummary;
  onOpen: (workspace: VisualWorkspace) => void;
}

const VIEW_KIND_LABELS: Record<ViewKind, string> = {
  timeline: "时间线",
  relationship: "关系图",
  mindmap: "思维导图",
  evidence_matrix: "证据矩阵",
  bar: "柱状图",
  line: "折线图",
  heatmap: "热力图",
  bar_table: "数据条表格",
};

export function indexVisualSummariesByMessageId(
  summaries: VisualWorkspaceSummary[],
): Map<string, VisualWorkspaceSummary> {
  const indexed = new Map<string, VisualWorkspaceSummary>();
  for (const summary of summaries) {
    if (summary.created_by_message_id) {
      indexed.set(summary.created_by_message_id, summary);
    }
  }
  return indexed;
}

export default function VisualizationPreviewCard({ caseId, summary, onOpen }: Props) {
  const [state, setState] = useState<"idle" | "loading" | "missing" | "error">("idle");

  async function openWorkspace() {
    setState("loading");
    try {
      const workspace = await getCaseVisualWorkspace(caseId);
      if (!workspace || workspace.id !== summary.id) {
        setState("missing");
        return;
      }
      onOpen(workspace);
      setState("idle");
    } catch {
      setState("error");
    }
  }

  const viewLabels = summary.view_kinds
    .map((kind) => VIEW_KIND_LABELS[kind] ?? "可视化视图")
    .join("、");

  return (
    <section className="mt-2 w-full max-w-[95%] overflow-hidden rounded-lg border border-brand/20 bg-background shadow-sm" aria-label="案情可视化预览">
      <div className="flex items-start gap-2.5 px-3 py-2.5">
        <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-brand-soft text-brand" aria-hidden>
          <Sparkles className="size-3.5" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-xs font-semibold text-foreground">案情可视化已生成</p>
          <p className="mt-0.5 truncate text-[11px] text-muted-foreground">{summary.title}</p>
          <div className="mt-2 flex items-center gap-1.5 text-[10px] leading-4 text-muted-foreground">
            <GitBranch className="size-3 shrink-0" />
            <span>{viewLabels || `${summary.view_count} 个视图`}</span>
            <span aria-hidden>·</span>
            <span>修订 {summary.revision}</span>
          </div>
        </div>
      </div>

      {(state === "missing" || state === "error") && (
        <div className="border-t border-border bg-surface-muted px-3 py-2 text-[11px] text-muted-foreground" role="status">
          {state === "missing" ? "该可视化工作区已不存在" : "暂时无法打开可视化，请重试"}
        </div>
      )}

      <div className="flex justify-end border-t border-border bg-surface-muted/60 px-2.5 py-2">
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="h-7 px-2 text-xs"
          disabled={state === "loading" || state === "missing"}
          onClick={() => void openWorkspace()}
          aria-label={state === "loading" ? "正在打开" : state === "error" ? "重试打开" : "进入工作台"}
        >
          {state === "loading" ? <LoaderCircle className="size-3.5 animate-spin" /> : <ArrowUpRight className="size-3.5" />}
          {state === "loading" ? "正在打开" : state === "error" ? "重试打开" : "进入工作台"}
        </Button>
      </div>
    </section>
  );
}
