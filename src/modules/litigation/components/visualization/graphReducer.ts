import type {
  CaseGraph,
  CaseGraphEdge,
  CaseGraphNode,
  CaseGraphPatch,
} from "./types";

export interface PatchConflict {
  entity: "node" | "edge";
  id: string;
  field: string;
  current: unknown;
  proposed: unknown;
}

export interface HistoryResult<T> {
  past: T[];
  current: T;
  future: T[];
  changed: boolean;
}

export type PatchSelectionKey =
  | `node:upsert:${string}`
  | `node:remove:${string}`
  | `edge:upsert:${string}`
  | `edge:remove:${string}`
  | `dataset:upsert:${string}`
  | `dataset:remove:${string}`
  | `view:upsert:${string}`
  | `view:remove:${string}`;

type LockableEntity = {
  id: string;
  locked_fields?: string[];
};

function valuesEqual(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  return JSON.stringify(left) === JSON.stringify(right);
}

function mergeEntity<T extends LockableEntity>(
  current: T,
  proposed: T,
  actor: "ai" | "user",
  entity: PatchConflict["entity"],
  conflicts: PatchConflict[],
): T {
  if (actor === "user") return { ...current, ...proposed };

  const output = { ...current } as T;
  const locked = new Set(current.locked_fields ?? []);
  for (const key of Object.keys(proposed) as Array<keyof T>) {
    if (key === "id") continue;
    const currentValue = current[key];
    const proposedValue = proposed[key];
    const field = String(key);
    if (field === "locked_fields" && !valuesEqual(currentValue, proposedValue)) {
      conflicts.push({
        entity,
        id: current.id,
        field,
        current: currentValue,
        proposed: proposedValue,
      });
      continue;
    }
    if (locked.has(field) && !valuesEqual(currentValue, proposedValue)) {
      conflicts.push({
        entity,
        id: current.id,
        field,
        current: currentValue,
        proposed: proposedValue,
      });
      continue;
    }
    output[key] = proposedValue;
  }
  return output;
}

function cleanViewReferences(graph: CaseGraph): CaseGraph["views"] {
  const nodeIds = new Set(graph.nodes.map((node) => node.id));
  const edgeIds = new Set(graph.edges.map((edge) => edge.id));
  const datasetIds = new Set(graph.datasets.map((dataset) => dataset.id));
  return graph.views
    .filter((view) => !view.dataset_id || datasetIds.has(view.dataset_id))
    .map((view) => ({
      ...view,
      node_ids: view.node_ids?.filter((nodeId) => nodeIds.has(nodeId)),
      edge_ids: view.edge_ids?.filter((edgeId) => edgeIds.has(edgeId)),
    }));
}

export function selectCaseGraphPatchChanges(
  patch: CaseGraphPatch,
  selected: ReadonlySet<PatchSelectionKey>,
): CaseGraphPatch {
  return {
    ...patch,
    upsert_nodes: patch.upsert_nodes.filter((node) => selected.has(`node:upsert:${node.id}`)),
    remove_node_ids: patch.remove_node_ids.filter((id) => selected.has(`node:remove:${id}`)),
    upsert_edges: patch.upsert_edges.filter((edge) => selected.has(`edge:upsert:${edge.id}`)),
    remove_edge_ids: patch.remove_edge_ids.filter((id) => selected.has(`edge:remove:${id}`)),
    upsert_datasets: patch.upsert_datasets.filter((dataset) => selected.has(`dataset:upsert:${dataset.id}`)),
    remove_dataset_ids: patch.remove_dataset_ids.filter((id) => selected.has(`dataset:remove:${id}`)),
    upsert_views: patch.upsert_views.filter((view) => selected.has(`view:upsert:${view.id}`)),
    remove_view_ids: patch.remove_view_ids.filter((id) => selected.has(`view:remove:${id}`)),
    summary: `部分接受：${patch.summary}`,
  };
}

export function applyCaseGraphPatch(
  graph: CaseGraph,
  patch: CaseGraphPatch,
  actor: "ai" | "user",
): { graph: CaseGraph; conflicts: PatchConflict[] } {
  const conflicts: PatchConflict[] = [];
  const nodes = new Map(graph.nodes.map((node) => [node.id, { ...node }]));
  const edges = new Map(graph.edges.map((edge) => [edge.id, { ...edge }]));
  const datasets = new Map(graph.datasets.map((dataset) => [dataset.id, { ...dataset }]));
  const views = new Map(graph.views.map((view) => [view.id, { ...view }]));

  for (const nodeId of patch.remove_node_ids) {
    if (!nodes.has(nodeId)) continue;
    const connected = [...edges.values()].filter(
      (edge) => edge.source === nodeId || edge.target === nodeId,
    );
    const locked = actor === "ai"
      ? connected.filter((edge) => (edge.locked_fields?.length ?? 0) > 0)
      : [];
    if (locked.length > 0) {
      for (const edge of locked) {
        conflicts.push({
          entity: "edge",
          id: edge.id,
          field: "$remove",
          current: edge,
          proposed: null,
        });
      }
      continue;
    }
    nodes.delete(nodeId);
    for (const edge of connected) edges.delete(edge.id);
  }

  for (const edgeId of patch.remove_edge_ids) {
    const current = edges.get(edgeId);
    if (!current) continue;
    if (actor === "ai" && (current.locked_fields?.length ?? 0) > 0) {
      conflicts.push({
        entity: "edge",
        id: edgeId,
        field: "$remove",
        current,
        proposed: null,
      });
      continue;
    }
    edges.delete(edgeId);
  }

  for (const node of patch.upsert_nodes) {
    const current = nodes.get(node.id);
    nodes.set(
      node.id,
      current ? mergeEntity(current, node, actor, "node", conflicts) : { ...node },
    );
  }

  for (const edge of patch.upsert_edges) {
    const missingFields = [
      !nodes.has(edge.source) ? "source" : null,
      !nodes.has(edge.target) ? "target" : null,
    ].filter((field): field is string => field !== null);
    if (missingFields.length > 0) {
      for (const field of missingFields) {
        conflicts.push({
          entity: "edge",
          id: edge.id,
          field,
          current: undefined,
          proposed: edge[field as "source" | "target"],
        });
      }
      continue;
    }
    const current = edges.get(edge.id);
    edges.set(
      edge.id,
      current ? mergeEntity(current, edge, actor, "edge", conflicts) : { ...edge },
    );
  }

  for (const datasetId of patch.remove_dataset_ids) datasets.delete(datasetId);
  for (const dataset of patch.upsert_datasets) datasets.set(dataset.id, { ...dataset });
  for (const viewId of patch.remove_view_ids) views.delete(viewId);
  for (const view of patch.upsert_views) views.set(view.id, { ...view });

  const output: CaseGraph = {
    ...graph,
    nodes: [...nodes.values()] as CaseGraphNode[],
    edges: [...edges.values()] as CaseGraphEdge[],
    datasets: [...datasets.values()],
    views: [...views.values()],
  };
  output.views = cleanViewReferences(output);
  return { graph: output, conflicts };
}

export function removeGraphView(graph: CaseGraph, viewId: string): CaseGraph {
  const removed = graph.views.find((view) => view.id === viewId);
  if (!removed) return graph;
  const views = graph.views.filter((view) => view.id !== viewId);
  const removedDatasetId = removed.dataset_id;
  const datasetStillUsed = removedDatasetId
    ? views.some((view) => view.dataset_id === removedDatasetId)
    : true;
  return {
    ...graph,
    views,
    datasets: removedDatasetId && !datasetStillUsed
      ? graph.datasets.filter((dataset) => dataset.id !== removedDatasetId)
      : graph.datasets,
  };
}

export function pushHistory<T>(past: T[], current: T, limit = 50): T[] {
  const boundedLimit = Math.max(0, Math.floor(limit));
  if (boundedLimit === 0) return [];
  return [...past, current].slice(-boundedLimit);
}

export function undoHistory<T>(past: T[], current: T, future: T[]): HistoryResult<T> {
  if (past.length === 0) return { past, current, future, changed: false };
  return {
    past: past.slice(0, -1),
    current: past[past.length - 1],
    future: [current, ...future],
    changed: true,
  };
}

export function redoHistory<T>(past: T[], current: T, future: T[]): HistoryResult<T> {
  if (future.length === 0) return { past, current, future, changed: false };
  return {
    past: pushHistory(past, current),
    current: future[0],
    future: future.slice(1),
    changed: true,
  };
}
