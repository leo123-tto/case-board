import { FileCheck2, FileQuestion, ShieldAlert } from "lucide-react";

import type { CaseGraph, CaseGraphNode, CaseGraphView } from "./types";
import { statusVisual } from "./visualizationTheme";

export interface EvidenceMatrixRow {
  issue: CaseGraphNode;
  proves: CaseGraphNode[];
  supports: CaseGraphNode[];
  refutes: CaseGraphNode[];
}

export function buildEvidenceMatrix(graph: CaseGraph, view: CaseGraphView): EvidenceMatrixRow[] {
  const selectedNodeIds = new Set(view.node_ids ?? graph.nodes.map((node) => node.id));
  const selectedEdgeIds = new Set(view.edge_ids ?? graph.edges.map((edge) => edge.id));
  const nodeById = new Map(graph.nodes.map((node) => [node.id, node]));
  return graph.nodes
    .filter((node) => selectedNodeIds.has(node.id) && (node.kind === "issue" || node.kind === "element"))
    .map((issue) => {
      const relations = graph.edges.filter(
        (edge) =>
          selectedEdgeIds.has(edge.id) &&
          edge.target === issue.id &&
          ["proves", "supports", "refutes"].includes(edge.kind),
      );
      const collect = (kind: "proves" | "supports" | "refutes") =>
        relations
          .filter((edge) => edge.kind === kind)
          .map((edge) => nodeById.get(edge.source))
          .filter((node): node is CaseGraphNode => Boolean(node));
      return {
        issue,
        proves: collect("proves"),
        supports: collect("supports"),
        refutes: collect("refutes"),
      };
    });
}

function EvidenceList({
  items,
  empty,
  onSelectNode,
}: {
  items: CaseGraphNode[];
  empty: string;
  onSelectNode?: (nodeId: string) => void;
}) {
  if (items.length === 0) return <span className="text-xs text-muted-foreground">{empty}</span>;
  return (
    <div className="space-y-1.5">
      {items.map((node) => {
        const visual = statusVisual(node.status);
        return (
          <button
            key={node.id}
            type="button"
            onClick={() => onSelectNode?.(node.id)}
            className="flex w-full items-start gap-2 rounded-md border border-border bg-background px-2.5 py-2 text-left hover:border-brand/35 hover:bg-brand-soft/35"
          >
            <span className="mt-0.5 text-[10px] font-semibold" style={{ color: visual.color }}>
              {visual.marker}
            </span>
            <span className="min-w-0">
              <span className="block truncate text-xs font-medium text-foreground">{node.label}</span>
              <span className="mt-0.5 block text-[10px] text-muted-foreground">
                {visual.label}，{node.source_refs.length} 项来源
              </span>
            </span>
          </button>
        );
      })}
    </div>
  );
}

interface Props {
  graph: CaseGraph;
  view: CaseGraphView;
  onSelectNode?: (nodeId: string) => void;
}

export default function EvidenceMatrixView({ graph, view, onSelectNode }: Props) {
  const rows = buildEvidenceMatrix(graph, view);
  if (rows.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        当前视图还没有争议焦点或构成要件
      </div>
    );
  }
  return (
    <div data-visual-export-root className="h-full overflow-auto p-6">
      <table className="w-full min-w-[860px] border-separate border-spacing-0 text-left">
        <thead>
          <tr className="text-xs text-muted-foreground">
            <th className="w-[24%] border-b border-border px-3 py-2 font-medium">争议焦点 / 要件</th>
            <th className="w-[25%] border-b border-border px-3 py-2 font-medium">
              <span className="inline-flex items-center gap-1.5"><FileCheck2 className="size-3.5" />直接证明</span>
            </th>
            <th className="w-[25%] border-b border-border px-3 py-2 font-medium">
              <span className="inline-flex items-center gap-1.5"><FileQuestion className="size-3.5" />辅助支持</span>
            </th>
            <th className="w-[26%] border-b border-border px-3 py-2 font-medium">
              <span className="inline-flex items-center gap-1.5"><ShieldAlert className="size-3.5" />反证 / 不利材料</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const visual = statusVisual(row.issue.status);
            return (
              <tr key={row.issue.id} className="align-top">
                <td className="border-b border-border/70 px-3 py-4">
                  <button type="button" onClick={() => onSelectNode?.(row.issue.id)} className="text-left">
                    <span className="block text-sm font-semibold text-foreground">{row.issue.label}</span>
                    <span className="mt-1 block text-[11px]" style={{ color: visual.color }}>
                      {visual.marker} {visual.label}
                    </span>
                  </button>
                </td>
                <td className="border-b border-border/70 px-3 py-4">
                  <EvidenceList items={row.proves} empty="尚无直接证据" onSelectNode={onSelectNode} />
                </td>
                <td className="border-b border-border/70 px-3 py-4">
                  <EvidenceList items={row.supports} empty="尚无辅助材料" onSelectNode={onSelectNode} />
                </td>
                <td className="border-b border-border/70 px-3 py-4">
                  <EvidenceList items={row.refutes} empty="尚未识别反证" onSelectNode={onSelectNode} />
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
