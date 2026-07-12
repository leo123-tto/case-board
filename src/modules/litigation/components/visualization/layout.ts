import ELK from "elkjs/lib/elk.bundled.js";
import type { ElkNode } from "elkjs/lib/elk-api";

import type { CaseGraph, CaseGraphView } from "./types";

const NODE_WIDTH = 220;
const NODE_HEIGHT = 84;

export interface PositionedVisualNode {
  id: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PositionedVisualEdge {
  id: string;
  source: string;
  target: string;
}

export interface CaseGraphLayout {
  nodes: PositionedVisualNode[];
  edges: PositionedVisualEdge[];
  width: number;
  height: number;
}

export async function layoutCaseGraph(
  graph: CaseGraph,
  view: CaseGraphView,
): Promise<CaseGraphLayout> {
  const selectedNodeIds = new Set(view.node_ids ?? graph.nodes.map((node) => node.id));
  const selectedEdgeIds = new Set(view.edge_ids ?? graph.edges.map((edge) => edge.id));
  const nodes = graph.nodes
    .filter((node) => selectedNodeIds.has(node.id))
    .sort((left, right) => left.id.localeCompare(right.id));
  const edges = graph.edges
    .filter(
      (edge) =>
        selectedEdgeIds.has(edge.id) &&
        selectedNodeIds.has(edge.source) &&
        selectedNodeIds.has(edge.target),
    )
    .sort((left, right) => left.id.localeCompare(right.id));
  if (nodes.length === 0) return { nodes: [], edges: [], width: 0, height: 0 };

  const elk = new ELK();
  const input: ElkNode = {
    id: "case-graph",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": view.kind === "mindmap" ? "RIGHT" : "DOWN",
      "elk.edgeRouting": "ORTHOGONAL",
      "elk.layered.spacing.nodeNodeBetweenLayers": "72",
      "elk.spacing.nodeNode": "36",
      "elk.padding": "[top=32,left=32,bottom=32,right=32]",
    },
    children: nodes.map((node) => ({
      id: node.id,
      width: NODE_WIDTH,
      height: NODE_HEIGHT,
    })),
    edges: edges.map((edge) => ({
      id: edge.id,
      sources: [edge.source],
      targets: [edge.target],
    })),
  };
  const result = await elk.layout(input);

  return {
    nodes: (result.children ?? []).map((node) => ({
      id: node.id,
      x: Math.round(node.x ?? 0),
      y: Math.round(node.y ?? 0),
      width: Math.round(node.width ?? NODE_WIDTH),
      height: Math.round(node.height ?? NODE_HEIGHT),
    })),
    edges: edges.map((edge) => ({ id: edge.id, source: edge.source, target: edge.target })),
    width: Math.round(result.width ?? 0),
    height: Math.round(result.height ?? 0),
  };
}
