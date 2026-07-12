import { useEffect, useMemo, useState } from "react";
import {
  Background,
  Controls,
  Handle,
  MarkerType,
  MiniMap,
  Position,
  ReactFlow,
  applyNodeChanges,
  type Edge,
  type Node,
  type NodeChange,
  type NodeProps,
  type NodeTypes,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { FileText, LockKeyhole } from "lucide-react";

import type { CaseGraph, CaseGraphNode, CaseGraphView } from "./types";
import { layoutCaseGraph } from "./layout";
import { NODE_KIND_LABELS, statusVisual } from "./visualizationTheme";

type VisualFlowNode = Node<{ graphNode: CaseGraphNode }, "caseNode">;

function CaseNode({ data, selected }: NodeProps<VisualFlowNode>) {
  const node = data.graphNode;
  const visual = statusVisual(node.status);
  return (
    <div
      className="w-[220px] rounded-md border-2 bg-background px-3 py-2.5 shadow-sm"
      style={{
        borderColor: visual.color,
        borderStyle: visual.borderStyle,
        boxShadow: selected ? `0 0 0 3px ${visual.color}26` : undefined,
      }}
    >
      <Handle type="target" position={Position.Left} className="!size-2 !border-0" style={{ background: visual.color }} />
      <div className="flex items-start justify-between gap-2">
        <span className="text-[10px] font-medium text-muted-foreground">{NODE_KIND_LABELS[node.kind]}</span>
        <span className="flex items-center gap-1 text-[10px] font-medium" style={{ color: visual.color }}>
          {visual.marker} {visual.label}
        </span>
      </div>
      <p className="mt-1 line-clamp-2 text-xs font-semibold leading-4 text-foreground">{node.label}</p>
      <div className="mt-2 flex items-center gap-2 text-[10px] text-muted-foreground">
        <span className="inline-flex items-center gap-1"><FileText className="size-3" />{node.source_refs.length}</span>
        {(node.locked_fields?.length ?? 0) > 0 && (
          <span className="inline-flex items-center gap-1"><LockKeyhole className="size-3" />人工锁定</span>
        )}
      </div>
      <Handle type="source" position={Position.Right} className="!size-2 !border-0" style={{ background: visual.color }} />
    </div>
  );
}

const nodeTypes: NodeTypes = { caseNode: CaseNode };

export interface CanvasPositions {
  [nodeId: string]: { x: number; y: number };
}

interface Props {
  graph: CaseGraph;
  view: CaseGraphView;
  positions?: CanvasPositions;
  onPositionsChange?: (positions: CanvasPositions) => void;
  onSelectNode?: (nodeId: string | null) => void;
}

export default function CaseGraphCanvas({
  graph,
  view,
  positions,
  onPositionsChange,
  onSelectNode,
}: Props) {
  const [nodes, setNodes] = useState<VisualFlowNode[]>([]);
  const [layoutError, setLayoutError] = useState<string | null>(null);
  const selectedNodeIds = useMemo(
    () => new Set(view.node_ids ?? graph.nodes.map((node) => node.id)),
    [graph.nodes, view.node_ids],
  );

  useEffect(() => {
    let cancelled = false;
    setLayoutError(null);
    void layoutCaseGraph(graph, view)
      .then((result) => {
        if (cancelled) return;
        const graphNodeById = new Map(graph.nodes.map((node) => [node.id, node]));
        setNodes(
          result.nodes.flatMap((item) => {
            const graphNode = graphNodeById.get(item.id);
            if (!graphNode) return [];
            return [
              {
                id: item.id,
                type: "caseNode" as const,
                position: positions?.[item.id] ?? { x: item.x, y: item.y },
                data: { graphNode },
                draggable: true,
                ariaLabel: `${NODE_KIND_LABELS[graphNode.kind]}：${graphNode.label}，${statusVisual(graphNode.status).label}`,
              },
            ];
          }),
        );
      })
      .catch((reason: unknown) => {
        if (!cancelled) setLayoutError(reason instanceof Error ? reason.message : "自动布局失败");
      });
    return () => {
      cancelled = true;
    };
  }, [graph, positions, view]);

  const edges = useMemo<Edge[]>(() => {
    const selectedEdgeIds = new Set(view.edge_ids ?? graph.edges.map((edge) => edge.id));
    return graph.edges
      .filter(
        (edge) =>
          selectedEdgeIds.has(edge.id) &&
          selectedNodeIds.has(edge.source) &&
          selectedNodeIds.has(edge.target),
      )
      .map((edge) => {
        const visual = statusVisual(edge.status);
        return {
          id: edge.id,
          source: edge.source,
          target: edge.target,
          label: edge.label,
          type: "smoothstep",
          markerEnd: { type: MarkerType.ArrowClosed, color: visual.color, width: 16, height: 16 },
          style: { stroke: visual.color, strokeWidth: 1.6, strokeDasharray: visual.lineStyle === "solid" ? undefined : "6 4" },
          labelStyle: { fill: visual.color, fontSize: 11 },
          labelBgStyle: { fill: "var(--background)", fillOpacity: 0.9 },
        };
      });
  }, [graph.edges, selectedNodeIds, view.edge_ids]);

  function handleNodesChange(changes: NodeChange<VisualFlowNode>[]) {
    setNodes((current) => applyNodeChanges(changes, current));
  }

  if (layoutError) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-center text-sm text-destructive">
        关系图自动布局失败：{layoutError}
      </div>
    );
  }
  if (nodes.length === 0) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        正在计算关系图布局…
      </div>
    );
  }
  return (
    <div data-visual-export-root className="h-full w-full">
    <ReactFlow<VisualFlowNode, Edge>
      nodes={nodes}
      edges={edges}
      nodeTypes={nodeTypes}
      onNodesChange={handleNodesChange}
      onNodeClick={(_event, node) => onSelectNode?.(node.id)}
      onPaneClick={() => onSelectNode?.(null)}
      onNodeDragStop={(_event, draggedNode) => {
        onPositionsChange?.(
          Object.fromEntries(
            nodes.map((node) => {
              const position = node.id === draggedNode.id ? draggedNode.position : node.position;
              return [node.id, { x: position.x, y: position.y }];
            }),
          ),
        );
      }}
      nodesConnectable={false}
      edgesReconnectable={false}
      fitView
      fitViewOptions={{ padding: 0.16, minZoom: 0.45, maxZoom: 1.25 }}
      minZoom={0.25}
      maxZoom={1.8}
      ariaLabelConfig={{
        "controls.ariaLabel": "关系图控制",
        "controls.zoomIn.ariaLabel": "放大",
        "controls.zoomOut.ariaLabel": "缩小",
        "controls.fitView.ariaLabel": "适合窗口",
        "minimap.ariaLabel": "关系图缩略图",
      }}
      proOptions={{ hideAttribution: true }}
      className="bg-background"
    >
      <Background color="var(--border)" gap={24} size={1} />
      <MiniMap pannable zoomable nodeColor={(node) => statusVisual((node.data as { graphNode: CaseGraphNode }).graphNode.status).color} />
      <Controls showInteractive={false} />
    </ReactFlow>
    </div>
  );
}
