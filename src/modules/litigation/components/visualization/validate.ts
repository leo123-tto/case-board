import {
  EDGE_KINDS,
  FACT_STATUSES,
  NODE_KINDS,
  VIEW_KINDS,
  type CaseGraph,
  type CaseGraphDataset,
  type CaseGraphDatasetColumn,
  type CaseGraphEdge,
  type CaseGraphNode,
  type CaseGraphPatch,
  type CaseGraphView,
  type DatasetCell,
  type DatasetColumnType,
  type FactStatus,
  type JsonValue,
  type SourceRef,
  type ViewConfigValue,
  type VisualProposal,
  type VisualProposalStatus,
  type VisualWorkspace,
  type VisualWorkspaceSummary,
} from "./types";

export const VISUAL_LIMITS = {
  nodes: 300,
  edges: 600,
  views: 20,
  datasets: 20,
  datasetRows: 1000,
  datasetColumns: 30,
  labelChars: 160,
  detailChars: 4000,
  quoteChars: 500,
} as const;

const UNSAFE_TEXT = /<[^>]*>|https?:\/\/|javascript:/i;
const ID_LIMIT = 200;

type UnknownRecord = Record<string, unknown>;

function fail(message: string): never {
  throw new Error(`可视化数据无效：${message}`);
}

function record(value: unknown, label: string): UnknownRecord {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    fail(`${label}必须是对象`);
  }
  return value as UnknownRecord;
}

function array(value: unknown, label: string, limit: number): unknown[] {
  if (!Array.isArray(value)) fail(`${label}必须是数组`);
  if (value.length > limit) fail(`${label}超过数量上限 ${limit}`);
  return value;
}

function text(
  value: unknown,
  label: string,
  limit: number = VISUAL_LIMITS.detailChars,
  allowEmpty = false,
): string {
  if (typeof value !== "string" || (!allowEmpty && value.trim().length === 0)) {
    fail(`${label}必须是${allowEmpty ? "" : "非空"}文本`);
  }
  if (value.length > limit) fail(`${label}超过长度上限 ${limit}`);
  if (UNSAFE_TEXT.test(value)) fail(`${label}包含非法文本`);
  return value;
}

function optionalText(value: unknown, label: string, limit?: number): string | undefined {
  return value === undefined ? undefined : text(value, label, limit);
}

function id(value: unknown, label: string): string {
  return text(value, label, ID_LIMIT);
}

function integer(value: unknown, label: string, minimum = 0): number {
  if (!Number.isInteger(value) || (value as number) < minimum) {
    fail(`${label}必须是不小于 ${minimum} 的整数`);
  }
  return value as number;
}

function enumValue<const T extends readonly string[]>(
  value: unknown,
  allowed: T,
  label: string,
): T[number] {
  if (typeof value !== "string" || !(allowed as readonly string[]).includes(value)) {
    fail(`${label}不是允许值`);
  }
  return value as T[number];
}

function optionalEnum<const T extends readonly string[]>(
  value: unknown,
  allowed: T,
  label: string,
): T[number] | undefined {
  return value === undefined ? undefined : enumValue(value, allowed, label);
}

function unique(values: string[], label: string): void {
  if (new Set(values).size !== values.length) fail(`${label}存在重复 ID`);
}

function stringArray(value: unknown, label: string, limit = 100): string[] {
  return array(value, label, limit).map((item, index) => text(item, `${label}[${index}]`, 160));
}

function optionalStringArray(value: unknown, label: string): string[] | undefined {
  return value === undefined ? undefined : stringArray(value, label);
}

function isoDate(value: unknown, label: string): string {
  const date = text(value, label, 10);
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!match) fail(`${label}必须是有效 ISO 日期`);
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  const parsed = new Date(Date.UTC(year, month - 1, day));
  if (
    parsed.getUTCFullYear() !== year ||
    parsed.getUTCMonth() !== month - 1 ||
    parsed.getUTCDate() !== day
  ) {
    fail(`${label}必须是有效 ISO 日期`);
  }
  return date;
}

function timestamp(value: unknown, label: string): string {
  const result = text(value, label, 64);
  if (!/^\d{4}-\d{2}-\d{2}T/.test(result) || Number.isNaN(Date.parse(result))) {
    fail(`${label}必须是有效时间戳`);
  }
  return result;
}

function nullableText(value: unknown, label: string): string | null {
  return value === null ? null : text(value, label, 500);
}

function sourceRef(value: unknown, label: string): SourceRef {
  const input = record(value, label);
  return {
    document_id: id(input.document_id, `${label}.document_id`),
    filename: text(input.filename, `${label}.filename`, 500),
    locator: optionalText(input.locator, `${label}.locator`, 500),
    quote: optionalText(input.quote, `${label}.quote`, VISUAL_LIMITS.quoteChars),
  };
}

function sourceRefs(value: unknown, label: string): SourceRef[] {
  return array(value, label, 100).map((item, index) => sourceRef(item, `${label}[${index}]`));
}

function factStatus(value: unknown, label: string): FactStatus {
  return enumValue(value, FACT_STATUSES, label);
}

function node(value: unknown, label: string): CaseGraphNode {
  const input = record(value, label);
  const parsed: CaseGraphNode = {
    id: id(input.id, `${label}.id`),
    kind: enumValue(input.kind, NODE_KINDS, `${label}.kind`),
    label: text(input.label, `${label}.label`, VISUAL_LIMITS.labelChars),
    detail: optionalText(input.detail, `${label}.detail`, VISUAL_LIMITS.detailChars),
    date: input.date === undefined ? undefined : isoDate(input.date, `${label}.date`),
    date_label: optionalText(input.date_label, `${label}.date_label`, 160),
    phase: optionalText(input.phase, `${label}.phase`, 160),
    side: optionalText(input.side, `${label}.side`, 160),
    status: factStatus(input.status, `${label}.status`),
    importance: optionalEnum(
      input.importance,
      ["critical", "normal", "context"] as const,
      `${label}.importance`,
    ),
    source_refs: sourceRefs(input.source_refs, `${label}.source_refs`),
    tags: optionalStringArray(input.tags, `${label}.tags`),
    provenance: enumValue(input.provenance, ["ai", "user"] as const, `${label}.provenance`),
    locked_fields: optionalStringArray(input.locked_fields, `${label}.locked_fields`),
  };
  if (
    parsed.importance === "critical" &&
    parsed.status === "confirmed" &&
    parsed.source_refs.length === 0
  ) {
    fail(`${label}的关键节点必须包含来源`);
  }
  return parsed;
}

function edge(value: unknown, label: string): CaseGraphEdge {
  const input = record(value, label);
  return {
    id: id(input.id, `${label}.id`),
    source: id(input.source, `${label}.source`),
    target: id(input.target, `${label}.target`),
    kind: enumValue(input.kind, EDGE_KINDS, `${label}.kind`),
    label: optionalText(input.label, `${label}.label`, VISUAL_LIMITS.labelChars),
    status: factStatus(input.status, `${label}.status`),
    source_refs: sourceRefs(input.source_refs, `${label}.source_refs`),
    provenance: enumValue(input.provenance, ["ai", "user"] as const, `${label}.provenance`),
    locked_fields: optionalStringArray(input.locked_fields, `${label}.locked_fields`),
  };
}

function datasetColumn(value: unknown, label: string): CaseGraphDatasetColumn {
  const input = record(value, label);
  return {
    key: id(input.key, `${label}.key`),
    label: text(input.label, `${label}.label`, VISUAL_LIMITS.labelChars),
    type: enumValue(
      input.type,
      ["text", "number", "date"] as const,
      `${label}.type`,
    ) as DatasetColumnType,
  };
}

function datasetCell(value: unknown, column: CaseGraphDatasetColumn, label: string): DatasetCell {
  if (value === null) return null;
  if (column.type === "number") {
    if (typeof value !== "number" || !Number.isFinite(value)) fail(`${label}必须是有限数字`);
    return value;
  }
  if (column.type === "date") return isoDate(value, label);
  return text(value, label, VISUAL_LIMITS.detailChars, true);
}

function dataset(value: unknown, label: string): CaseGraphDataset {
  const input = record(value, label);
  const columns = array(
    input.columns,
    `${label}.columns`,
    VISUAL_LIMITS.datasetColumns,
  ).map((item, index) => datasetColumn(item, `${label}.columns[${index}]`));
  unique(columns.map((column) => column.key), `${label}.columns`);
  const columnByKey = new Map(columns.map((column) => [column.key, column]));
  const rows = array(input.rows, `${label}.rows`, VISUAL_LIMITS.datasetRows).map(
    (rowValue, rowIndex) => {
      const row = record(rowValue, `${label}.rows[${rowIndex}]`);
      const parsed: Record<string, DatasetCell> = {};
      for (const [key, cell] of Object.entries(row)) {
        const column = columnByKey.get(key);
        if (!column) fail(`${label}.rows[${rowIndex}]包含未声明列 ${key}`);
        parsed[key] = datasetCell(cell, column, `${label}.rows[${rowIndex}].${key}`);
      }
      return parsed;
    },
  );
  return {
    id: id(input.id, `${label}.id`),
    title: text(input.title, `${label}.title`, VISUAL_LIMITS.labelChars),
    columns,
    rows,
    source_refs: sourceRefs(input.source_refs, `${label}.source_refs`),
  };
}

function configValue(value: unknown, label: string): ViewConfigValue {
  if (typeof value === "string") return text(value, label, 500, true);
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "boolean") return value;
  if (Array.isArray(value)) return stringArray(value, label, 100);
  return fail(`${label}不是允许的视图配置值`);
}

function view(value: unknown, label: string): CaseGraphView {
  const input = record(value, label);
  const configInput = record(input.config, `${label}.config`);
  const config: Record<string, ViewConfigValue> = {};
  for (const [key, configItem] of Object.entries(configInput)) {
    const safeKey = id(key, `${label}.config key`);
    config[safeKey] = configValue(configItem, `${label}.config.${safeKey}`);
  }
  return {
    id: id(input.id, `${label}.id`),
    kind: enumValue(input.kind, VIEW_KINDS, `${label}.kind`),
    title: text(input.title, `${label}.title`, VISUAL_LIMITS.labelChars),
    description: optionalText(input.description, `${label}.description`, VISUAL_LIMITS.detailChars),
    node_ids: optionalStringArray(input.node_ids, `${label}.node_ids`),
    edge_ids: optionalStringArray(input.edge_ids, `${label}.edge_ids`),
    dataset_id: input.dataset_id === undefined ? undefined : id(input.dataset_id, `${label}.dataset_id`),
    config,
  };
}

function verifyGraphReferences(graph: CaseGraph): void {
  const nodeIds = new Set(graph.nodes.map((item) => item.id));
  const edgeIds = new Set(graph.edges.map((item) => item.id));
  const datasetIds = new Set(graph.datasets.map((item) => item.id));
  for (const relation of graph.edges) {
    if (!nodeIds.has(relation.source) || !nodeIds.has(relation.target)) {
      fail(`关系 ${relation.id} 是悬空关系`);
    }
  }
  for (const item of graph.views) {
    const missingNode = item.node_ids?.find((nodeId) => !nodeIds.has(nodeId));
    const missingEdge = item.edge_ids?.find((edgeId) => !edgeIds.has(edgeId));
    if (missingNode || missingEdge) fail(`视图 ${item.id} 包含悬空引用`);
    if (item.dataset_id && !datasetIds.has(item.dataset_id)) {
      fail(`视图 ${item.id} 引用了不存在的数据集`);
    }
  }
}

export function parseCaseGraph(value: unknown): CaseGraph {
  const input = record(value, "CaseGraph");
  if (input.schema_version !== 1) fail("schema_version 仅支持 1");
  const graph: CaseGraph = {
    schema_version: 1,
    case_id: id(input.case_id, "case_id"),
    title: text(input.title, "title", VISUAL_LIMITS.labelChars),
    summary: text(input.summary, "summary", VISUAL_LIMITS.detailChars, true),
    nodes: array(input.nodes, "nodes", VISUAL_LIMITS.nodes).map((item, index) =>
      node(item, `nodes[${index}]`),
    ),
    edges: array(input.edges, "edges", VISUAL_LIMITS.edges).map((item, index) =>
      edge(item, `edges[${index}]`),
    ),
    datasets: array(input.datasets, "datasets", VISUAL_LIMITS.datasets).map((item, index) =>
      dataset(item, `datasets[${index}]`),
    ),
    views: array(input.views, "views", VISUAL_LIMITS.views).map((item, index) =>
      view(item, `views[${index}]`),
    ),
  };
  unique(graph.nodes.map((item) => item.id), "nodes");
  unique(graph.edges.map((item) => item.id), "edges");
  unique(graph.datasets.map((item) => item.id), "datasets");
  unique(graph.views.map((item) => item.id), "views");
  verifyGraphReferences(graph);
  return graph;
}

function jsonValue(value: unknown, label: string, depth = 0): JsonValue {
  if (depth > 12) fail(`${label}嵌套过深`);
  if (value === null || typeof value === "boolean") return value;
  if (typeof value === "string") return text(value, label, VISUAL_LIMITS.detailChars, true);
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (Array.isArray(value)) {
    if (value.length > VISUAL_LIMITS.datasetRows) fail(`${label}数组过长`);
    return value.map((item, index) => jsonValue(item, `${label}[${index}]`, depth + 1));
  }
  const input = record(value, label);
  const output: Record<string, JsonValue> = {};
  for (const [key, item] of Object.entries(input)) {
    const safeKey = id(key, `${label} key`);
    output[safeKey] = jsonValue(item, `${label}.${safeKey}`, depth + 1);
  }
  return output;
}

function jsonRecord(value: unknown, label: string): Record<string, JsonValue> {
  const parsed = jsonValue(value, label);
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    fail(`${label}必须是对象`);
  }
  return parsed as Record<string, JsonValue>;
}

export function parseWorkspace(value: unknown): VisualWorkspace {
  const input = record(value, "workspace");
  if (input.schema_version !== 1) fail("workspace.schema_version 仅支持 1");
  const graph = parseCaseGraph(input.graph);
  const caseId = id(input.case_id, "workspace.case_id");
  if (graph.case_id !== caseId) fail("workspace.case_id 与 graph.case_id 不一致");
  return {
    id: id(input.id, "workspace.id"),
    case_id: caseId,
    schema_version: 1,
    graph,
    layout: jsonRecord(input.layout, "workspace.layout"),
    revision: integer(input.revision, "workspace.revision", 1),
    source_fingerprint: nullableText(input.source_fingerprint, "workspace.source_fingerprint"),
    created_by_message_id: nullableText(
      input.created_by_message_id,
      "workspace.created_by_message_id",
    ),
    created_at: timestamp(input.created_at, "workspace.created_at"),
    updated_at: timestamp(input.updated_at, "workspace.updated_at"),
  };
}

export function parseWorkspaceSummaries(value: unknown): VisualWorkspaceSummary[] {
  return array(value, "workspace summaries", 1000).map((item, index) => {
    const input = record(item, `workspace summaries[${index}]`);
    return {
      id: id(input.id, `workspace summaries[${index}].id`),
      case_id: id(input.case_id, `workspace summaries[${index}].case_id`),
      title: text(
        input.title,
        `workspace summaries[${index}].title`,
        VISUAL_LIMITS.labelChars,
      ),
      revision: integer(input.revision, `workspace summaries[${index}].revision`, 1),
      view_count: integer(input.view_count, `workspace summaries[${index}].view_count`),
      view_kinds: array(
        input.view_kinds,
        `workspace summaries[${index}].view_kinds`,
        VISUAL_LIMITS.views,
      ).map((kind, kindIndex) =>
        enumValue(
          kind,
          VIEW_KINDS,
          `workspace summaries[${index}].view_kinds[${kindIndex}]`,
        ),
      ),
      created_by_message_id: nullableText(
        input.created_by_message_id,
        `workspace summaries[${index}].created_by_message_id`,
      ),
      updated_at: timestamp(input.updated_at, `workspace summaries[${index}].updated_at`),
    };
  });
}

function patch(value: unknown): CaseGraphPatch {
  const input = record(value, "proposal.patch");
  const parsed: CaseGraphPatch = {
    base_revision: integer(input.base_revision, "proposal.patch.base_revision", 1),
    upsert_nodes: array(input.upsert_nodes, "proposal.patch.upsert_nodes", VISUAL_LIMITS.nodes).map(
      (item, index) => node(item, `proposal.patch.upsert_nodes[${index}]`),
    ),
    remove_node_ids: stringArray(
      input.remove_node_ids,
      "proposal.patch.remove_node_ids",
      VISUAL_LIMITS.nodes,
    ),
    upsert_edges: array(input.upsert_edges, "proposal.patch.upsert_edges", VISUAL_LIMITS.edges).map(
      (item, index) => edge(item, `proposal.patch.upsert_edges[${index}]`),
    ),
    remove_edge_ids: stringArray(
      input.remove_edge_ids,
      "proposal.patch.remove_edge_ids",
      VISUAL_LIMITS.edges,
    ),
    upsert_datasets: array(
      input.upsert_datasets,
      "proposal.patch.upsert_datasets",
      VISUAL_LIMITS.datasets,
    ).map((item, index) => dataset(item, `proposal.patch.upsert_datasets[${index}]`)),
    remove_dataset_ids: stringArray(
      input.remove_dataset_ids,
      "proposal.patch.remove_dataset_ids",
      VISUAL_LIMITS.datasets,
    ),
    upsert_views: array(input.upsert_views, "proposal.patch.upsert_views", VISUAL_LIMITS.views).map(
      (item, index) => view(item, `proposal.patch.upsert_views[${index}]`),
    ),
    remove_view_ids: stringArray(
      input.remove_view_ids,
      "proposal.patch.remove_view_ids",
      VISUAL_LIMITS.views,
    ),
    summary: text(input.summary, "proposal.patch.summary", VISUAL_LIMITS.detailChars),
  };
  unique(parsed.upsert_nodes.map((item) => item.id), "proposal.patch.upsert_nodes");
  unique(parsed.upsert_edges.map((item) => item.id), "proposal.patch.upsert_edges");
  unique(parsed.upsert_datasets.map((item) => item.id), "proposal.patch.upsert_datasets");
  unique(parsed.upsert_views.map((item) => item.id), "proposal.patch.upsert_views");
  unique(parsed.remove_node_ids, "proposal.patch.remove_node_ids");
  unique(parsed.remove_edge_ids, "proposal.patch.remove_edge_ids");
  unique(parsed.remove_dataset_ids, "proposal.patch.remove_dataset_ids");
  unique(parsed.remove_view_ids, "proposal.patch.remove_view_ids");
  const removedNodes = new Set(parsed.remove_node_ids);
  const badEdge = parsed.upsert_edges.find(
    (item) => removedNodes.has(item.source) || removedNodes.has(item.target),
  );
  if (badEdge) fail(`补丁关系 ${badEdge.id} 引用了同时删除的节点`);
  return parsed;
}

export function parseProposal(value: unknown): VisualProposal {
  const input = record(value, "proposal");
  const parsedPatch = patch(input.patch);
  const baseRevision = integer(input.base_revision, "proposal.base_revision", 1);
  if (parsedPatch.base_revision !== baseRevision) fail("proposal 的 base_revision 不一致");
  return {
    id: id(input.id, "proposal.id"),
    workspace_id: id(input.workspace_id, "proposal.workspace_id"),
    base_revision: baseRevision,
    patch: parsedPatch,
    summary: jsonRecord(input.summary, "proposal.summary"),
    status: enumValue(
      input.status,
      ["pending", "accepted", "rejected", "stale"] as const,
      "proposal.status",
    ) as VisualProposalStatus,
    created_at: timestamp(input.created_at, "proposal.created_at"),
    updated_at: timestamp(input.updated_at, "proposal.updated_at"),
  };
}
