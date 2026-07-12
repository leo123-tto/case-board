export const NODE_KINDS = [
  "actor",
  "event",
  "claim",
  "issue",
  "legal_basis",
  "element",
  "defense",
  "evidence",
  "amount",
  "document",
  "action",
] as const;

export type NodeKind = (typeof NODE_KINDS)[number];

export const FACT_STATUSES = [
  "confirmed",
  "our_claim",
  "opponent_claim",
  "disputed",
  "inferred",
  "unknown",
] as const;

export type FactStatus = (typeof FACT_STATUSES)[number];

export const EDGE_KINDS = [
  "relates_to",
  "represents",
  "contracts_with",
  "pays",
  "owes",
  "guarantees",
  "causes",
  "precedes",
  "supports",
  "refutes",
  "proves",
  "requires",
  "responds_to",
] as const;

export type EdgeKind = (typeof EDGE_KINDS)[number];

export const VIEW_KINDS = [
  "timeline",
  "relationship",
  "mindmap",
  "evidence_matrix",
  "bar",
  "line",
  "heatmap",
  "bar_table",
] as const;

export type ViewKind = (typeof VIEW_KINDS)[number];

export type Provenance = "ai" | "user";
export type NodeImportance = "critical" | "normal" | "context";
export type DatasetColumnType = "text" | "number" | "date";
export type DatasetCell = string | number | null;
export type ViewConfigValue = string | number | boolean | string[];
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };

export interface SourceRef {
  document_id: string;
  filename: string;
  locator?: string;
  quote?: string;
}

export interface CaseGraphNode {
  id: string;
  kind: NodeKind;
  label: string;
  detail?: string;
  date?: string;
  date_label?: string;
  phase?: string;
  side?: string;
  status: FactStatus;
  importance?: NodeImportance;
  source_refs: SourceRef[];
  tags?: string[];
  provenance: Provenance;
  locked_fields?: string[];
}

export interface CaseGraphEdge {
  id: string;
  source: string;
  target: string;
  kind: EdgeKind;
  label?: string;
  status: FactStatus;
  source_refs: SourceRef[];
  provenance: Provenance;
  locked_fields?: string[];
}

export interface CaseGraphDatasetColumn {
  key: string;
  label: string;
  type: DatasetColumnType;
}

export interface CaseGraphDataset {
  id: string;
  title: string;
  columns: CaseGraphDatasetColumn[];
  rows: Array<Record<string, DatasetCell>>;
  source_refs: SourceRef[];
}

export interface CaseGraphView {
  id: string;
  kind: ViewKind;
  title: string;
  description?: string;
  node_ids?: string[];
  edge_ids?: string[];
  dataset_id?: string;
  config: Record<string, ViewConfigValue>;
}

export interface CaseGraph {
  schema_version: 1;
  case_id: string;
  title: string;
  summary: string;
  nodes: CaseGraphNode[];
  edges: CaseGraphEdge[];
  datasets: CaseGraphDataset[];
  views: CaseGraphView[];
}

export interface CaseGraphPatch {
  base_revision: number;
  upsert_nodes: CaseGraphNode[];
  remove_node_ids: string[];
  upsert_edges: CaseGraphEdge[];
  remove_edge_ids: string[];
  upsert_datasets: CaseGraphDataset[];
  remove_dataset_ids: string[];
  upsert_views: CaseGraphView[];
  remove_view_ids: string[];
  summary: string;
}

export interface VisualWorkspaceSummary {
  id: string;
  case_id: string;
  title: string;
  revision: number;
  view_count: number;
  view_kinds: ViewKind[];
  created_by_message_id: string | null;
  updated_at: string;
}

export interface VisualWorkspace {
  id: string;
  case_id: string;
  schema_version: 1;
  graph: CaseGraph;
  layout: Record<string, JsonValue>;
  revision: number;
  source_fingerprint: string | null;
  created_by_message_id: string | null;
  created_at: string;
  updated_at: string;
}

export type VisualProposalStatus = "pending" | "accepted" | "rejected" | "stale";

export interface VisualProposal {
  id: string;
  workspace_id: string;
  base_revision: number;
  patch: CaseGraphPatch;
  summary: Record<string, JsonValue>;
  status: VisualProposalStatus;
  created_at: string;
  updated_at: string;
}

export interface VisualExportResult {
  format: "json" | "markdown";
  filename: string;
  content: string;
}
