import { useMemo, useState } from "react";
import {
  FileCheck2,
  GitCompareArrows,
  Plus,
  RefreshCw,
  ShieldAlert,
  Trash2,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

import {
  selectCaseGraphPatchChanges,
  type PatchSelectionKey,
} from "./graphReducer";
import type {
  CaseGraph,
  CaseGraphEdge,
  CaseGraphNode,
  CaseGraphPatch,
  SourceRef,
  VisualProposal,
} from "./types";
import { NODE_KIND_LABELS } from "./visualizationTheme";

interface Props {
  proposal: VisualProposal;
  currentGraph: CaseGraph;
  onApply: (patch: CaseGraphPatch) => void | Promise<void>;
  onReject: () => void | Promise<void>;
  onRefresh: () => void | Promise<void>;
  applying?: boolean;
}

interface ReviewItem {
  key: PatchSelectionKey;
  label: string;
  detail: string;
  category: "addition" | "modification" | "source" | "relation" | "conflict";
  selectable: boolean;
}

function equal(left: unknown, right: unknown): boolean {
  return Object.is(left, right) || JSON.stringify(left) === JSON.stringify(right);
}

function changedFields<T extends object>(current: T, proposed: T): string[] {
  return Object.keys(proposed).filter((field) => field !== "id" && !equal(
    current[field as keyof T],
    proposed[field as keyof T],
  ));
}

function sourceSummary(sourceRefs: SourceRef[]): string {
  if (sourceRefs.length === 0) return "未关联材料";
  return sourceRefs.map((source) => source.locator
    ? `${source.filename}（${source.locator}）`
    : source.filename).join("、");
}

function nodeChangeDetail(current: CaseGraphNode, proposed: CaseGraphNode, fields: string[]): string {
  const details: string[] = [];
  if (fields.includes("label")) details.push(`标题：${current.label} → AI 建议标题`);
  if (fields.includes("date") || fields.includes("date_label")) {
    details.push(`时间：${current.date ?? current.date_label ?? "未填写"} → ${proposed.date ?? proposed.date_label ?? "未填写"}`);
  }
  const remaining = fields.filter((field) => !["label", "date", "date_label", "source_refs", "locked_fields"].includes(field));
  if (remaining.length > 0) details.push(`另有 ${remaining.length} 个事实字段变化`);
  return details.join("；") || proposed.label;
}

function edgeTitle(edge: CaseGraphEdge, graph: CaseGraph): string {
  const source = graph.nodes.find((node) => node.id === edge.source)?.label ?? edge.source;
  const target = graph.nodes.find((node) => node.id === edge.target)?.label ?? edge.target;
  return `${source} → ${target}`;
}

function buildReviewItems(proposal: VisualProposal, graph: CaseGraph): ReviewItem[] {
  const nodes = new Map(graph.nodes.map((node) => [node.id, node]));
  const edges = new Map(graph.edges.map((edge) => [edge.id, edge]));
  const datasets = new Map(graph.datasets.map((dataset) => [dataset.id, dataset]));
  const views = new Map(graph.views.map((view) => [view.id, view]));
  const items: ReviewItem[] = [];

  for (const proposed of proposal.patch.upsert_nodes) {
    const current = nodes.get(proposed.id);
    const key = `node:upsert:${proposed.id}` as const;
    if (!current) {
      items.push({
        key,
        label: proposed.label,
        detail: `${NODE_KIND_LABELS[proposed.kind]}，${sourceSummary(proposed.source_refs)}`,
        category: "addition",
        selectable: true,
      });
      continue;
    }
    const fields = changedFields(current, proposed);
    if (fields.length === 0) continue;
    const locks = new Set([...(current.locked_fields ?? []), ...(proposed.locked_fields ?? [])]);
    const lockedChanges = fields.filter((field) => field === "locked_fields" || locks.has(field));
    if (lockedChanges.length > 0) {
      items.push({
        key,
        label: proposed.label,
        detail: `${nodeChangeDetail(current, proposed, fields)}；涉及已由律师确认的字段`,
        category: "conflict",
        selectable: false,
      });
      continue;
    }
    items.push({
      key,
      label: proposed.label,
      detail: nodeChangeDetail(current, proposed, fields),
      category: fields.includes("source_refs") ? "source" : "modification",
      selectable: true,
    });
  }

  for (const nodeId of proposal.patch.remove_node_ids) {
    const current = nodes.get(nodeId);
    if (!current) continue;
    items.push({
      key: `node:remove:${nodeId}`,
      label: current.label,
      detail: `删除${NODE_KIND_LABELS[current.kind]}`,
      category: (current.locked_fields?.length ?? 0) > 0 ? "conflict" : "modification",
      selectable: (current.locked_fields?.length ?? 0) === 0,
    });
  }

  for (const proposed of proposal.patch.upsert_edges) {
    const current = edges.get(proposed.id);
    const fields = current ? changedFields(current, proposed) : [];
    const locks = new Set([...(current?.locked_fields ?? []), ...(proposed.locked_fields ?? [])]);
    const conflict = fields.some((field) => field === "locked_fields" || locks.has(field));
    items.push({
      key: `edge:upsert:${proposed.id}`,
      label: edgeTitle(proposed, graph),
      detail: current ? "调整事实之间的关系" : "新增事实之间的关系",
      category: conflict ? "conflict" : fields.includes("source_refs") ? "source" : "relation",
      selectable: !conflict,
    });
  }

  for (const edgeId of proposal.patch.remove_edge_ids) {
    const current = edges.get(edgeId);
    if (!current) continue;
    const conflict = (current.locked_fields?.length ?? 0) > 0;
    items.push({
      key: `edge:remove:${edgeId}`,
      label: edgeTitle(current, graph),
      detail: "删除事实之间的关系",
      category: conflict ? "conflict" : "relation",
      selectable: !conflict,
    });
  }

  for (const proposed of proposal.patch.upsert_datasets) {
    const current = datasets.get(proposed.id);
    items.push({
      key: `dataset:upsert:${proposed.id}`,
      label: proposed.title,
      detail: current ? "调整图表数据" : "新增图表数据",
      category: current ? "modification" : "addition",
      selectable: true,
    });
  }
  for (const id of proposal.patch.remove_dataset_ids) {
    const current = datasets.get(id);
    if (current) items.push({ key: `dataset:remove:${id}`, label: current.title, detail: "删除图表数据", category: "modification", selectable: true });
  }
  for (const proposed of proposal.patch.upsert_views) {
    const current = views.get(proposed.id);
    items.push({
      key: `view:upsert:${proposed.id}`,
      label: proposed.title,
      detail: current ? "调整可视化视图" : "新增可视化视图",
      category: current ? "modification" : "addition",
      selectable: true,
    });
  }
  for (const id of proposal.patch.remove_view_ids) {
    const current = views.get(id);
    if (current) items.push({ key: `view:remove:${id}`, label: current.title, detail: "删除可视化视图", category: "modification", selectable: true });
  }
  return items;
}

const sections = [
  { category: "addition" as const, title: "新增事件", icon: Plus },
  { category: "modification" as const, title: "事实变化", icon: GitCompareArrows },
  { category: "source" as const, title: "材料依据变化", icon: FileCheck2 },
  { category: "relation" as const, title: "关系变化", icon: Trash2 },
  { category: "conflict" as const, title: "与人工确认冲突", icon: ShieldAlert },
];

export default function ProposalReviewPanel({
  proposal,
  currentGraph,
  onApply,
  onReject,
  onRefresh,
  applying = false,
}: Props) {
  const [selected, setSelected] = useState<Set<PatchSelectionKey>>(new Set());
  const items = useMemo(() => buildReviewItems(proposal, currentGraph), [currentGraph, proposal]);
  const stale = proposal.status === "stale";

  function toggle(key: PatchSelectionKey) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function selectionLabel(item: ReviewItem): string {
    if (item.category === "conflict") return `冲突变更：${item.label}`;
    if (item.key.startsWith("node:upsert")) return `新增节点：${item.label}`;
    if (item.key.startsWith("edge:remove")) return `删除关系：${item.label}`;
    return `选择变更：${item.label}`;
  }

  return (
    <aside className="flex min-h-0 flex-col border-l border-border bg-surface-muted" aria-label="AI 变更审查">
      <div className="border-b border-border px-4 py-3">
        <p className="text-xs font-semibold text-foreground">AI 变更审查</p>
        <p className="mt-1 text-[11px] leading-4 text-muted-foreground">{proposal.patch.summary}</p>
      </div>

      {stale && (
        <div className="border-b border-amber-700/20 bg-amber-50 px-4 py-3 text-xs text-amber-900">
          <p className="font-medium">该建议基于旧版本工作区</p>
          <p className="mt-1 leading-4">先刷新到最新修订，再让 AI 重新核对变更。</p>
          <Button type="button" variant="outline" size="sm" className="mt-2" onClick={() => void onRefresh()}>
            <RefreshCw className="size-3.5" />
            刷新工作区
          </Button>
        </div>
      )}

      <div className="min-h-0 flex-1 overflow-y-auto px-3 py-3">
        <div className="space-y-4">
          {sections.map(({ category, title, icon: Icon }) => {
            const sectionItems = items.filter((item) => item.category === category);
            if (sectionItems.length === 0) return null;
            return (
              <section key={category} aria-labelledby={`proposal-${category}`}>
                <div className={cn("mb-1.5 flex items-center gap-1.5 px-1", category === "conflict" && "text-amber-800")}>
                  <Icon className="size-3.5" />
                  <h2 id={`proposal-${category}`} className="text-[11px] font-semibold">{title}</h2>
                  <span className="text-[10px] text-muted-foreground">{sectionItems.length}</span>
                </div>
                <div className="space-y-1.5">
                  {sectionItems.map((item) => (
                    <label
                      key={item.key}
                      className={cn(
                        "flex gap-2.5 rounded-md border border-border bg-background px-2.5 py-2",
                        item.selectable && !stale ? "cursor-pointer hover:border-brand/30" : "cursor-default bg-background/60",
                        item.category === "conflict" && "border-amber-700/20 bg-amber-50/70",
                      )}
                    >
                      <input
                        type="checkbox"
                        className="mt-0.5 size-3.5 accent-primary"
                        checked={selected.has(item.key)}
                        disabled={!item.selectable || stale || applying}
                        onChange={() => toggle(item.key)}
                        aria-label={selectionLabel(item)}
                      />
                      <span className="min-w-0">
                        <span className="block text-[11px] font-medium leading-4 text-foreground">{item.label}</span>
                        <span className="mt-0.5 block text-[10px] leading-4 text-muted-foreground">{item.detail}</span>
                      </span>
                    </label>
                  ))}
                </div>
              </section>
            );
          })}
        </div>
      </div>

      <div className="border-t border-border bg-background px-3 py-3">
        <Button
          type="button"
          size="sm"
          className="w-full"
          disabled={stale || selected.size === 0 || applying}
          onClick={() => void onApply(selectCaseGraphPatchChanges(proposal.patch, selected))}
        >
          {applying ? "正在应用" : `应用 ${selected.size} 项变更`}
        </Button>
        <Button type="button" variant="ghost" size="sm" className="mt-1 w-full text-muted-foreground" disabled={applying} onClick={() => void onReject()}>
          拒绝本次建议
        </Button>
      </div>
    </aside>
  );
}
