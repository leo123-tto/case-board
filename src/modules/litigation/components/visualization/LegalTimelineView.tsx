import { CalendarDays, FileText } from "lucide-react";

import { cn } from "@/lib/utils";

import type { CaseGraph, CaseGraphNode, CaseGraphView } from "./types";
import { statusVisual } from "./visualizationTheme";

export interface TimelineItem {
  node: CaseGraphNode;
  dateText: string;
  precision: "exact" | "approximate" | "unknown";
  sourceCount: number;
}

export function buildTimelineItems(graph: CaseGraph, view: CaseGraphView): TimelineItem[] {
  const selectedIds = new Set(view.node_ids ?? graph.nodes.map((node) => node.id));
  return graph.nodes
    .filter((node) => selectedIds.has(node.id) && node.kind === "event")
    .map((node, index) => ({
      node,
      dateText: node.date ?? node.date_label ?? "日期待确认",
      precision: node.date ? ("exact" as const) : node.date_label ? ("approximate" as const) : ("unknown" as const),
      sourceCount: node.source_refs.length,
      index,
    }))
    .sort((left, right) => {
      const rank = { exact: 0, approximate: 1, unknown: 2 } as const;
      const precision = rank[left.precision] - rank[right.precision];
      if (precision !== 0) return precision;
      if (left.precision === "exact" && right.precision === "exact") {
        return left.dateText.localeCompare(right.dateText);
      }
      return left.index - right.index;
    })
    .map(({ index: _index, ...item }) => item);
}

interface Props {
  graph: CaseGraph;
  view: CaseGraphView;
  selectedNodeId?: string | null;
  onSelectNode?: (nodeId: string) => void;
}

export default function LegalTimelineView({
  graph,
  view,
  selectedNodeId,
  onSelectNode,
}: Props) {
  const items = buildTimelineItems(graph, view);
  if (items.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        当前视图还没有事件节点
      </div>
    );
  }
  return (
    <div data-visual-export-root className="h-full overflow-auto px-8 py-7">
      <div className="mx-auto max-w-4xl">
        {items.map((item, index) => {
          const visual = statusVisual(item.node.status);
          return (
            <button
              key={item.node.id}
              type="button"
              onClick={() => onSelectNode?.(item.node.id)}
              className={cn(
                "group grid w-full grid-cols-[140px_24px_minmax(0,1fr)] text-left outline-none",
                selectedNodeId === item.node.id && "rounded-md bg-brand-soft/55",
              )}
            >
              <div className="pr-4 pt-1 text-right">
                <p className="font-mono text-xs font-medium text-foreground">{item.dateText}</p>
                <p className="mt-1 text-[11px] text-muted-foreground">
                  {item.precision === "exact"
                    ? "日期明确"
                    : item.precision === "approximate"
                      ? "日期范围"
                      : "待核对"}
                </p>
              </div>
              <div className="relative flex justify-center">
                {index < items.length - 1 && (
                  <span className="absolute bottom-0 top-4 w-px bg-border" aria-hidden />
                )}
                <span
                  className="relative mt-1 flex size-5 items-center justify-center rounded-full border-2 bg-background text-[10px] font-semibold"
                  style={{ borderColor: visual.color, color: visual.color }}
                  aria-label={visual.label}
                >
                  {visual.marker}
                </span>
              </div>
              <div className="mb-5 ml-3 border-l-2 px-4 pb-1" style={{ borderLeftColor: visual.color }}>
                <div className="flex flex-wrap items-center gap-2">
                  <h3 className="text-sm font-semibold text-foreground">{item.node.label}</h3>
                  <span
                    className="rounded border px-1.5 py-0.5 text-[10px] font-medium"
                    style={{
                      color: visual.color,
                      backgroundColor: visual.background,
                      borderColor: visual.color,
                      borderStyle: visual.borderStyle,
                    }}
                  >
                    {visual.label}
                  </span>
                </div>
                {item.node.detail && (
                  <p className="mt-1.5 text-xs leading-5 text-muted-foreground">{item.node.detail}</p>
                )}
                <div className="mt-2 flex items-center gap-3 text-[11px] text-muted-foreground">
                  {item.node.phase && (
                    <span className="inline-flex items-center gap-1">
                      <CalendarDays className="size-3" />
                      {item.node.phase}
                    </span>
                  )}
                  <span className="inline-flex items-center gap-1">
                    <FileText className="size-3" />
                    {item.sourceCount > 0 ? `${item.sourceCount} 项材料依据` : "未绑定材料"}
                  </span>
                </div>
              </div>
            </button>
          );
        })}
      </div>
    </div>
  );
}
