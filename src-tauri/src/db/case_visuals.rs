//! 案件可视化工作区的领域模型、校验、修订历史与 AI 提案持久化。

use std::collections::{BTreeMap, BTreeSet, HashSet};

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};
use uuid::Uuid;

const MAX_NODES: usize = 300;
const MAX_EDGES: usize = 600;
const MAX_VIEWS: usize = 20;
const MAX_DATASETS: usize = 20;
const MAX_DATASET_ROWS: usize = 1000;
const MAX_DATASET_COLUMNS: usize = 30;
const MAX_LABEL_CHARS: usize = 160;
const MAX_DETAIL_CHARS: usize = 4000;
const MAX_QUOTE_CHARS: usize = 500;

#[derive(Debug, thiserror::Error)]
pub enum VisualError {
    #[error("可视化数据无效：{0}")]
    Validation(String),
    #[error("可视化工作区不存在")]
    WorkspaceNotFound,
    #[error("可视化修订不存在")]
    RevisionNotFound,
    #[error("可视化提案不存在")]
    ProposalNotFound,
    #[error("可视化提案已过期，请重新分析后再应用")]
    StaleProposal,
    #[error("可视化版本已变化，请刷新后重试")]
    RevisionConflict,
    #[error("可视化提案当前状态不可应用：{0}")]
    ProposalState(String),
    #[error("数据库操作失败：{0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("可视化 JSON 解析失败：{0}")]
    Json(#[from] serde_json::Error),
}

impl serde::Serialize for VisualError {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Actor,
    Event,
    Claim,
    Issue,
    LegalBasis,
    Element,
    Defense,
    Evidence,
    Amount,
    Document,
    Action,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactStatus {
    Confirmed,
    OurClaim,
    OpponentClaim,
    Disputed,
    Inferred,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provenance {
    Ai,
    User,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeImportance {
    Critical,
    Normal,
    Context,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    RelatesTo,
    Represents,
    ContractsWith,
    Pays,
    Owes,
    Guarantees,
    Causes,
    Precedes,
    Supports,
    Refutes,
    Proves,
    Requires,
    RespondsTo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    Timeline,
    Relationship,
    Mindmap,
    EvidenceMatrix,
    Bar,
    Line,
    Heatmap,
    BarTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetColumnType {
    Text,
    Number,
    Date,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DatasetCell {
    Text(String),
    Number(f64),
    Null,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ViewConfigValue {
    Text(String),
    Number(f64),
    Bool(bool),
    Texts(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub document_id: String,
    pub filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseGraphNode {
    pub id: String,
    pub kind: NodeKind,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    pub status: FactStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub importance: Option<NodeImportance>,
    pub source_refs: Vec<SourceRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locked_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseGraphEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub kind: EdgeKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub status: FactStatus,
    pub source_refs: Vec<SourceRef>,
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locked_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseGraphDatasetColumn {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub column_type: DatasetColumnType,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseGraphDataset {
    pub id: String,
    pub title: String,
    pub columns: Vec<CaseGraphDatasetColumn>,
    pub rows: Vec<BTreeMap<String, DatasetCell>>,
    pub source_refs: Vec<SourceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseGraphView {
    pub id: String,
    pub kind: ViewKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub node_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset_id: Option<String>,
    pub config: BTreeMap<String, ViewConfigValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseGraph {
    pub schema_version: i64,
    pub case_id: String,
    pub title: String,
    pub summary: String,
    pub nodes: Vec<CaseGraphNode>,
    pub edges: Vec<CaseGraphEdge>,
    pub datasets: Vec<CaseGraphDataset>,
    pub views: Vec<CaseGraphView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseGraphPatch {
    pub base_revision: i64,
    pub upsert_nodes: Vec<CaseGraphNode>,
    pub remove_node_ids: Vec<String>,
    pub upsert_edges: Vec<CaseGraphEdge>,
    pub remove_edge_ids: Vec<String>,
    pub upsert_datasets: Vec<CaseGraphDataset>,
    pub remove_dataset_ids: Vec<String>,
    pub upsert_views: Vec<CaseGraphView>,
    pub remove_view_ids: Vec<String>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VisualWorkspace {
    pub id: String,
    pub case_id: String,
    pub schema_version: i64,
    pub graph: CaseGraph,
    pub layout: Value,
    pub revision: i64,
    pub source_fingerprint: Option<String>,
    pub created_by_message_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VisualWorkspaceSummary {
    pub id: String,
    pub case_id: String,
    pub title: String,
    pub revision: i64,
    pub view_count: usize,
    pub view_kinds: Vec<ViewKind>,
    pub created_by_message_id: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualProposalStatus {
    Pending,
    Accepted,
    Rejected,
    Stale,
}

impl VisualProposalStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
            Self::Stale => "stale",
        }
    }
}

impl TryFrom<&str> for VisualProposalStatus {
    type Error = VisualError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "pending" => Ok(Self::Pending),
            "accepted" => Ok(Self::Accepted),
            "rejected" => Ok(Self::Rejected),
            "stale" => Ok(Self::Stale),
            _ => Err(VisualError::Validation(format!("未知提案状态 {value}"))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VisualProposal {
    pub id: String,
    pub workspace_id: String,
    pub base_revision: i64,
    pub patch: CaseGraphPatch,
    pub summary: Value,
    pub status: VisualProposalStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct NewVisualWorkspace<'a> {
    pub id: &'a str,
    pub case_id: &'a str,
    pub graph: &'a CaseGraph,
    pub layout: &'a Value,
    pub source_fingerprint: Option<&'a str>,
    pub created_by_message_id: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct SaveVisualRevision<'a> {
    pub workspace_id: &'a str,
    pub expected_revision: i64,
    pub graph: &'a CaseGraph,
    pub layout: &'a Value,
    pub summary: &'a str,
}

#[derive(Debug, Clone)]
pub struct NewVisualProposal<'a> {
    pub id: &'a str,
    pub workspace_id: &'a str,
    pub base_revision: i64,
    pub patch: &'a CaseGraphPatch,
    pub summary: &'a Value,
}

#[derive(Debug, FromRow)]
struct WorkspaceRow {
    id: String,
    case_id: String,
    schema_version: i64,
    graph_json: String,
    layout_json: String,
    revision: i64,
    source_fingerprint: Option<String>,
    created_by_message_id: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Debug, FromRow)]
struct ProposalRow {
    id: String,
    workspace_id: String,
    base_revision: i64,
    patch_json: String,
    summary_json: String,
    status: String,
    created_at: String,
    updated_at: String,
}

fn validation(message: impl Into<String>) -> VisualError {
    VisualError::Validation(message.into())
}

fn unsafe_text(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    (value.contains('<') && value.contains('>'))
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("javascript:")
}

fn validate_text(
    value: &str,
    label: &str,
    limit: usize,
    allow_empty: bool,
) -> Result<(), VisualError> {
    if !allow_empty && value.trim().is_empty() {
        return Err(validation(format!("{label}不能为空")));
    }
    if value.chars().count() > limit {
        return Err(validation(format!("{label}超过长度上限 {limit}")));
    }
    if unsafe_text(value) {
        return Err(validation(format!("{label}包含非法文本")));
    }
    Ok(())
}

fn validate_source(source: &SourceRef, label: &str) -> Result<(), VisualError> {
    validate_text(
        &source.document_id,
        &format!("{label}.document_id"),
        200,
        false,
    )?;
    validate_text(&source.filename, &format!("{label}.filename"), 500, false)?;
    if let Some(locator) = &source.locator {
        validate_text(locator, &format!("{label}.locator"), 500, false)?;
    }
    if let Some(quote) = &source.quote {
        validate_text(quote, &format!("{label}.quote"), MAX_QUOTE_CHARS, false)?;
    }
    Ok(())
}

fn validate_sources(sources: &[SourceRef], label: &str) -> Result<(), VisualError> {
    if sources.len() > 100 {
        return Err(validation(format!("{label}超过数量上限 100")));
    }
    for (index, source) in sources.iter().enumerate() {
        validate_source(source, &format!("{label}[{index}]"))?;
    }
    Ok(())
}

fn validate_node(node: &CaseGraphNode, label: &str) -> Result<(), VisualError> {
    validate_text(&node.id, &format!("{label}.id"), 200, false)?;
    validate_text(
        &node.label,
        &format!("{label}.label"),
        MAX_LABEL_CHARS,
        false,
    )?;
    for (field, value, max) in [
        ("detail", node.detail.as_deref(), MAX_DETAIL_CHARS),
        ("date_label", node.date_label.as_deref(), MAX_LABEL_CHARS),
        ("phase", node.phase.as_deref(), MAX_LABEL_CHARS),
        ("side", node.side.as_deref(), MAX_LABEL_CHARS),
    ] {
        if let Some(value) = value {
            validate_text(value, &format!("{label}.{field}"), max, false)?;
        }
    }
    if let Some(date) = &node.date {
        validate_text(date, &format!("{label}.date"), 10, false)?;
        NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| validation(format!("{label}.date 必须是有效 ISO 日期")))?;
    }
    validate_sources(&node.source_refs, &format!("{label}.source_refs"))?;
    if node.importance == Some(NodeImportance::Critical)
        && node.status == FactStatus::Confirmed
        && node.source_refs.is_empty()
    {
        return Err(validation(format!("{label}的关键节点必须包含来源")));
    }
    for (index, tag) in node.tags.iter().enumerate() {
        validate_text(
            tag,
            &format!("{label}.tags[{index}]"),
            MAX_LABEL_CHARS,
            false,
        )?;
    }
    for (index, field) in node.locked_fields.iter().enumerate() {
        validate_text(
            field,
            &format!("{label}.locked_fields[{index}]"),
            MAX_LABEL_CHARS,
            false,
        )?;
    }
    Ok(())
}

fn validate_edge(edge: &CaseGraphEdge, label: &str) -> Result<(), VisualError> {
    validate_text(&edge.id, &format!("{label}.id"), 200, false)?;
    validate_text(&edge.source, &format!("{label}.source"), 200, false)?;
    validate_text(&edge.target, &format!("{label}.target"), 200, false)?;
    if let Some(value) = &edge.label {
        validate_text(value, &format!("{label}.label"), MAX_LABEL_CHARS, false)?;
    }
    validate_sources(&edge.source_refs, &format!("{label}.source_refs"))?;
    for (index, field) in edge.locked_fields.iter().enumerate() {
        validate_text(
            field,
            &format!("{label}.locked_fields[{index}]"),
            MAX_LABEL_CHARS,
            false,
        )?;
    }
    Ok(())
}

fn validate_dataset(dataset: &CaseGraphDataset, label: &str) -> Result<(), VisualError> {
    validate_text(&dataset.id, &format!("{label}.id"), 200, false)?;
    validate_text(
        &dataset.title,
        &format!("{label}.title"),
        MAX_LABEL_CHARS,
        false,
    )?;
    if dataset.columns.len() > MAX_DATASET_COLUMNS {
        return Err(validation(format!("{label}.columns超过数量上限")));
    }
    if dataset.rows.len() > MAX_DATASET_ROWS {
        return Err(validation(format!("{label}.rows超过数量上限")));
    }
    let mut column_keys = HashSet::new();
    for (index, column) in dataset.columns.iter().enumerate() {
        validate_text(
            &column.key,
            &format!("{label}.columns[{index}].key"),
            200,
            false,
        )?;
        validate_text(
            &column.label,
            &format!("{label}.columns[{index}].label"),
            MAX_LABEL_CHARS,
            false,
        )?;
        if !column_keys.insert(column.key.as_str()) {
            return Err(validation(format!("{label}.columns存在重复 ID")));
        }
    }
    let column_by_key: BTreeMap<_, _> = dataset
        .columns
        .iter()
        .map(|column| (column.key.as_str(), column))
        .collect();
    for (row_index, row) in dataset.rows.iter().enumerate() {
        for (key, cell) in row {
            let Some(column) = column_by_key.get(key.as_str()) else {
                return Err(validation(format!(
                    "{label}.rows[{row_index}]包含未声明列 {key}"
                )));
            };
            match (column.column_type, cell) {
                (_, DatasetCell::Null) => {}
                (DatasetColumnType::Text, DatasetCell::Text(value)) => validate_text(
                    value,
                    &format!("{label}.rows[{row_index}].{key}"),
                    MAX_DETAIL_CHARS,
                    true,
                )?,
                (DatasetColumnType::Date, DatasetCell::Text(value)) => {
                    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
                        validation(format!(
                            "{label}.rows[{row_index}].{key}必须是有效 ISO 日期"
                        ))
                    })?;
                }
                (DatasetColumnType::Number, DatasetCell::Number(value)) if value.is_finite() => {}
                _ => {
                    return Err(validation(format!(
                        "{label}.rows[{row_index}].{key}类型不匹配"
                    )))
                }
            }
        }
    }
    validate_sources(&dataset.source_refs, &format!("{label}.source_refs"))?;
    Ok(())
}

fn validate_view(view: &CaseGraphView, label: &str) -> Result<(), VisualError> {
    validate_text(&view.id, &format!("{label}.id"), 200, false)?;
    validate_text(
        &view.title,
        &format!("{label}.title"),
        MAX_LABEL_CHARS,
        false,
    )?;
    if let Some(description) = &view.description {
        validate_text(
            description,
            &format!("{label}.description"),
            MAX_DETAIL_CHARS,
            false,
        )?;
    }
    for (key, value) in &view.config {
        validate_text(key, &format!("{label}.config key"), 200, false)?;
        match value {
            ViewConfigValue::Text(value) => {
                validate_text(value, &format!("{label}.config.{key}"), 500, true)?
            }
            ViewConfigValue::Texts(values) => {
                if values.len() > 100 {
                    return Err(validation(format!("{label}.config.{key}数组过长")));
                }
                for (index, value) in values.iter().enumerate() {
                    validate_text(value, &format!("{label}.config.{key}[{index}]"), 500, true)?;
                }
            }
            ViewConfigValue::Number(value) if !value.is_finite() => {
                return Err(validation(format!("{label}.config.{key}必须是有限数字")))
            }
            ViewConfigValue::Number(_) | ViewConfigValue::Bool(_) => {}
        }
    }
    Ok(())
}

fn duplicate_id<'a>(mut values: impl Iterator<Item = &'a String>) -> Option<String> {
    let mut seen = HashSet::new();
    values.find_map(|value| {
        if seen.insert(value.as_str()) {
            None
        } else {
            Some(value.clone())
        }
    })
}

pub fn validate_graph(graph: &CaseGraph) -> Result<(), VisualError> {
    if graph.schema_version != 1 {
        return Err(validation("schema_version 仅支持 1"));
    }
    validate_text(&graph.case_id, "case_id", 200, false)?;
    validate_text(&graph.title, "title", MAX_LABEL_CHARS, false)?;
    validate_text(&graph.summary, "summary", MAX_DETAIL_CHARS, true)?;
    for (name, actual, limit) in [
        ("nodes", graph.nodes.len(), MAX_NODES),
        ("edges", graph.edges.len(), MAX_EDGES),
        ("datasets", graph.datasets.len(), MAX_DATASETS),
        ("views", graph.views.len(), MAX_VIEWS),
    ] {
        if actual > limit {
            return Err(validation(format!("{name}超过数量上限 {limit}")));
        }
    }
    for (index, node) in graph.nodes.iter().enumerate() {
        validate_node(node, &format!("nodes[{index}]"))?;
    }
    for (index, edge) in graph.edges.iter().enumerate() {
        validate_edge(edge, &format!("edges[{index}]"))?;
    }
    for (index, dataset) in graph.datasets.iter().enumerate() {
        validate_dataset(dataset, &format!("datasets[{index}]"))?;
    }
    for (index, view) in graph.views.iter().enumerate() {
        validate_view(view, &format!("views[{index}]"))?;
    }
    for (label, duplicate) in [
        (
            "nodes",
            duplicate_id(graph.nodes.iter().map(|item| &item.id)),
        ),
        (
            "edges",
            duplicate_id(graph.edges.iter().map(|item| &item.id)),
        ),
        (
            "datasets",
            duplicate_id(graph.datasets.iter().map(|item| &item.id)),
        ),
        (
            "views",
            duplicate_id(graph.views.iter().map(|item| &item.id)),
        ),
    ] {
        if let Some(id) = duplicate {
            return Err(validation(format!("{label}存在重复 ID {id}")));
        }
    }
    let node_ids: HashSet<_> = graph.nodes.iter().map(|item| item.id.as_str()).collect();
    let edge_ids: HashSet<_> = graph.edges.iter().map(|item| item.id.as_str()).collect();
    let dataset_ids: HashSet<_> = graph.datasets.iter().map(|item| item.id.as_str()).collect();
    for edge in &graph.edges {
        if !node_ids.contains(edge.source.as_str()) || !node_ids.contains(edge.target.as_str()) {
            return Err(validation(format!("关系 {} 是悬空关系", edge.id)));
        }
    }
    for view in &graph.views {
        if view
            .node_ids
            .iter()
            .any(|id| !node_ids.contains(id.as_str()))
            || view
                .edge_ids
                .iter()
                .any(|id| !edge_ids.contains(id.as_str()))
        {
            return Err(validation(format!("视图 {} 包含悬空引用", view.id)));
        }
        if view
            .dataset_id
            .as_ref()
            .is_some_and(|id| !dataset_ids.contains(id.as_str()))
        {
            return Err(validation(format!("视图 {} 引用了不存在的数据集", view.id)));
        }
    }
    Ok(())
}

fn validate_json(value: &Value, label: &str, depth: usize) -> Result<(), VisualError> {
    if depth > 12 {
        return Err(validation(format!("{label}嵌套过深")));
    }
    match value {
        Value::String(value) => validate_text(value, label, MAX_DETAIL_CHARS, true),
        Value::Number(value) if value.as_f64().is_none_or(f64::is_finite) => Ok(()),
        Value::Number(_) => Err(validation(format!("{label}必须是有限数字"))),
        Value::Array(values) => {
            if values.len() > MAX_DATASET_ROWS {
                return Err(validation(format!("{label}数组过长")));
            }
            for (index, item) in values.iter().enumerate() {
                validate_json(item, &format!("{label}[{index}]"), depth + 1)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, item) in values {
                validate_text(key, &format!("{label} key"), 200, false)?;
                validate_json(item, &format!("{label}.{key}"), depth + 1)?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) => Ok(()),
    }
}

fn validate_patch(patch: &CaseGraphPatch) -> Result<(), VisualError> {
    if patch.base_revision < 1 {
        return Err(validation("patch.base_revision 必须不小于 1"));
    }
    validate_text(&patch.summary, "patch.summary", MAX_DETAIL_CHARS, false)?;
    for (name, actual, limit) in [
        ("upsert_nodes", patch.upsert_nodes.len(), MAX_NODES),
        ("remove_node_ids", patch.remove_node_ids.len(), MAX_NODES),
        ("upsert_edges", patch.upsert_edges.len(), MAX_EDGES),
        ("remove_edge_ids", patch.remove_edge_ids.len(), MAX_EDGES),
        ("upsert_datasets", patch.upsert_datasets.len(), MAX_DATASETS),
        (
            "remove_dataset_ids",
            patch.remove_dataset_ids.len(),
            MAX_DATASETS,
        ),
        ("upsert_views", patch.upsert_views.len(), MAX_VIEWS),
        ("remove_view_ids", patch.remove_view_ids.len(), MAX_VIEWS),
    ] {
        if actual > limit {
            return Err(validation(format!("patch.{name}超过数量上限 {limit}")));
        }
    }
    for (index, node) in patch.upsert_nodes.iter().enumerate() {
        validate_node(node, &format!("patch.upsert_nodes[{index}]"))?;
    }
    for (index, edge) in patch.upsert_edges.iter().enumerate() {
        validate_edge(edge, &format!("patch.upsert_edges[{index}]"))?;
    }
    for (index, dataset) in patch.upsert_datasets.iter().enumerate() {
        validate_dataset(dataset, &format!("patch.upsert_datasets[{index}]"))?;
    }
    for (index, view) in patch.upsert_views.iter().enumerate() {
        validate_view(view, &format!("patch.upsert_views[{index}]"))?;
    }
    for (label, ids) in [
        ("remove_node_ids", &patch.remove_node_ids),
        ("remove_edge_ids", &patch.remove_edge_ids),
        ("remove_dataset_ids", &patch.remove_dataset_ids),
        ("remove_view_ids", &patch.remove_view_ids),
    ] {
        for (index, id) in ids.iter().enumerate() {
            validate_text(id, &format!("patch.{label}[{index}]"), 200, false)?;
        }
        if let Some(id) = duplicate_id(ids.iter()) {
            return Err(validation(format!("patch.{label}存在重复 ID {id}")));
        }
    }
    for (label, duplicate) in [
        (
            "upsert_nodes",
            duplicate_id(patch.upsert_nodes.iter().map(|item| &item.id)),
        ),
        (
            "upsert_edges",
            duplicate_id(patch.upsert_edges.iter().map(|item| &item.id)),
        ),
        (
            "upsert_datasets",
            duplicate_id(patch.upsert_datasets.iter().map(|item| &item.id)),
        ),
        (
            "upsert_views",
            duplicate_id(patch.upsert_views.iter().map(|item| &item.id)),
        ),
    ] {
        if let Some(id) = duplicate {
            return Err(validation(format!("patch.{label}存在重复 ID {id}")));
        }
    }
    let removed_nodes: HashSet<_> = patch.remove_node_ids.iter().map(String::as_str).collect();
    if let Some(edge) = patch.upsert_edges.iter().find(|edge| {
        removed_nodes.contains(edge.source.as_str()) || removed_nodes.contains(edge.target.as_str())
    }) {
        return Err(validation(format!(
            "补丁关系 {} 引用了同时删除的节点",
            edge.id
        )));
    }
    Ok(())
}

fn workspace_from_row(row: WorkspaceRow) -> Result<VisualWorkspace, VisualError> {
    let graph: CaseGraph = serde_json::from_str(&row.graph_json)?;
    let layout: Value = serde_json::from_str(&row.layout_json)?;
    validate_graph(&graph)?;
    validate_json(&layout, "layout", 0)?;
    Ok(VisualWorkspace {
        id: row.id,
        case_id: row.case_id,
        schema_version: row.schema_version,
        graph,
        layout,
        revision: row.revision,
        source_fingerprint: row.source_fingerprint,
        created_by_message_id: row.created_by_message_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn proposal_from_row(row: ProposalRow) -> Result<VisualProposal, VisualError> {
    let patch: CaseGraphPatch = serde_json::from_str(&row.patch_json)?;
    let summary: Value = serde_json::from_str(&row.summary_json)?;
    validate_patch(&patch)?;
    validate_json(&summary, "proposal.summary", 0)?;
    Ok(VisualProposal {
        id: row.id,
        workspace_id: row.workspace_id,
        base_revision: row.base_revision,
        patch,
        summary,
        status: VisualProposalStatus::try_from(row.status.as_str())?,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

const WORKSPACE_COLUMNS: &str = "id, case_id, schema_version, graph_json, layout_json, revision, source_fingerprint, created_by_message_id, created_at, updated_at";
const PROPOSAL_COLUMNS: &str =
    "id, workspace_id, base_revision, patch_json, summary_json, status, created_at, updated_at";

pub async fn get_workspace(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Option<VisualWorkspace>, VisualError> {
    let query = format!("SELECT {WORKSPACE_COLUMNS} FROM case_visual_workspaces WHERE case_id = ?");
    let row = sqlx::query_as::<_, WorkspaceRow>(&query)
        .bind(case_id)
        .fetch_optional(pool)
        .await?;
    row.map(workspace_from_row).transpose()
}

pub async fn get_workspace_by_id(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Option<VisualWorkspace>, VisualError> {
    let query = format!("SELECT {WORKSPACE_COLUMNS} FROM case_visual_workspaces WHERE id = ?");
    let row = sqlx::query_as::<_, WorkspaceRow>(&query)
        .bind(workspace_id)
        .fetch_optional(pool)
        .await?;
    row.map(workspace_from_row).transpose()
}

pub async fn list_summaries(
    pool: &SqlitePool,
    case_id: &str,
) -> Result<Vec<VisualWorkspaceSummary>, VisualError> {
    let Some(workspace) = get_workspace(pool, case_id).await? else {
        return Ok(Vec::new());
    };
    let view_kinds: BTreeSet<_> = workspace.graph.views.iter().map(|view| view.kind).collect();
    Ok(vec![VisualWorkspaceSummary {
        id: workspace.id,
        case_id: workspace.case_id,
        title: workspace.graph.title,
        revision: workspace.revision,
        view_count: workspace.graph.views.len(),
        view_kinds: view_kinds.into_iter().collect(),
        created_by_message_id: workspace.created_by_message_id,
        updated_at: workspace.updated_at,
    }])
}

pub async fn create_workspace(
    pool: &SqlitePool,
    input: NewVisualWorkspace<'_>,
) -> Result<VisualWorkspace, VisualError> {
    validate_text(input.id, "workspace.id", 200, false)?;
    validate_text(input.case_id, "workspace.case_id", 200, false)?;
    validate_graph(input.graph)?;
    validate_json(input.layout, "workspace.layout", 0)?;
    if input.graph.case_id != input.case_id {
        return Err(validation("workspace.case_id 与 graph.case_id 不一致"));
    }
    let graph_json = serde_json::to_string(input.graph)?;
    let layout_json = serde_json::to_string(input.layout)?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO case_visual_workspaces \
         (id, case_id, schema_version, graph_json, layout_json, revision, source_fingerprint, created_by_message_id) \
         VALUES (?, ?, 1, ?, ?, 1, ?, ?)",
    )
    .bind(input.id)
    .bind(input.case_id)
    .bind(&graph_json)
    .bind(&layout_json)
    .bind(input.source_fingerprint)
    .bind(input.created_by_message_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO case_visual_revisions \
         (id, workspace_id, revision, base_revision, graph_json, layout_json, source, summary) \
         VALUES (?, ?, 1, 0, ?, ?, 'ai_initial', '首次生成')",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(input.id)
    .bind(&graph_json)
    .bind(&layout_json)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get_workspace(pool, input.case_id)
        .await?
        .ok_or(VisualError::WorkspaceNotFound)
}

async fn prune_revisions(pool: &SqlitePool, workspace_id: &str) -> Result<(), VisualError> {
    for (layout_only, limit) in [(0_i64, 100_i64), (1_i64, 20_i64)] {
        sqlx::query(
            "DELETE FROM case_visual_revisions \
             WHERE workspace_id = ? AND is_layout_only = ? AND id NOT IN (\
               SELECT id FROM case_visual_revisions \
               WHERE workspace_id = ? AND is_layout_only = ? \
               ORDER BY revision DESC LIMIT ?\
             )",
        )
        .bind(workspace_id)
        .bind(layout_only)
        .bind(workspace_id)
        .bind(layout_only)
        .bind(limit)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn save_user_revision(
    pool: &SqlitePool,
    input: SaveVisualRevision<'_>,
) -> Result<VisualWorkspace, VisualError> {
    validate_graph(input.graph)?;
    validate_json(input.layout, "workspace.layout", 0)?;
    validate_text(input.summary, "revision.summary", MAX_DETAIL_CHARS, false)?;
    let graph_json = serde_json::to_string(input.graph)?;
    let layout_json = serde_json::to_string(input.layout)?;
    let mut tx = pool.begin().await?;
    let query = format!("SELECT {WORKSPACE_COLUMNS} FROM case_visual_workspaces WHERE id = ?");
    let row = sqlx::query_as::<_, WorkspaceRow>(&query)
        .bind(input.workspace_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(VisualError::WorkspaceNotFound)?;
    let workspace = workspace_from_row(row)?;
    if workspace.revision != input.expected_revision {
        return Err(VisualError::RevisionConflict);
    }
    if workspace.case_id != input.graph.case_id {
        return Err(validation("workspace.case_id 与 graph.case_id 不一致"));
    }
    let next_revision = workspace.revision + 1;
    sqlx::query(
        "UPDATE case_visual_workspaces \
         SET graph_json = ?, layout_json = ?, revision = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ? AND revision = ?",
    )
    .bind(&graph_json)
    .bind(&layout_json)
    .bind(next_revision)
    .bind(input.workspace_id)
    .bind(input.expected_revision)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO case_visual_revisions \
         (id, workspace_id, revision, base_revision, graph_json, layout_json, source, summary) \
         VALUES (?, ?, ?, ?, ?, ?, 'user_edit', ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(input.workspace_id)
    .bind(next_revision)
    .bind(workspace.revision)
    .bind(&graph_json)
    .bind(&layout_json)
    .bind(input.summary)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE case_visual_proposals \
         SET status = 'stale', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE workspace_id = ? AND status = 'pending'",
    )
    .bind(input.workspace_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    prune_revisions(pool, input.workspace_id).await?;
    get_workspace_by_id(pool, input.workspace_id)
        .await?
        .ok_or(VisualError::WorkspaceNotFound)
}

fn merge_node(current: &CaseGraphNode, mut proposed: CaseGraphNode) -> CaseGraphNode {
    for field in &current.locked_fields {
        match field.as_str() {
            "kind" => proposed.kind = current.kind,
            "label" => proposed.label.clone_from(&current.label),
            "detail" => proposed.detail.clone_from(&current.detail),
            "date" => proposed.date.clone_from(&current.date),
            "date_label" => proposed.date_label.clone_from(&current.date_label),
            "phase" => proposed.phase.clone_from(&current.phase),
            "side" => proposed.side.clone_from(&current.side),
            "status" => proposed.status = current.status,
            "importance" => proposed.importance = current.importance,
            "source_refs" => proposed.source_refs.clone_from(&current.source_refs),
            "tags" => proposed.tags.clone_from(&current.tags),
            "provenance" => proposed.provenance = current.provenance,
            _ => {}
        }
    }
    proposed.locked_fields.clone_from(&current.locked_fields);
    if current.provenance == Provenance::User {
        proposed.provenance = Provenance::User;
    }
    proposed
}

fn merge_edge(current: &CaseGraphEdge, mut proposed: CaseGraphEdge) -> CaseGraphEdge {
    for field in &current.locked_fields {
        match field.as_str() {
            "source" => proposed.source.clone_from(&current.source),
            "target" => proposed.target.clone_from(&current.target),
            "kind" => proposed.kind = current.kind,
            "label" => proposed.label.clone_from(&current.label),
            "status" => proposed.status = current.status,
            "source_refs" => proposed.source_refs.clone_from(&current.source_refs),
            "provenance" => proposed.provenance = current.provenance,
            _ => {}
        }
    }
    proposed.locked_fields.clone_from(&current.locked_fields);
    if current.provenance == Provenance::User {
        proposed.provenance = Provenance::User;
    }
    proposed
}

fn apply_patch(graph: &CaseGraph, patch: &CaseGraphPatch) -> CaseGraph {
    let mut nodes: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .cloned()
        .map(|node| (node.id.clone(), node))
        .collect();
    let mut edges: BTreeMap<_, _> = graph
        .edges
        .iter()
        .cloned()
        .map(|edge| (edge.id.clone(), edge))
        .collect();
    let mut datasets: BTreeMap<_, _> = graph
        .datasets
        .iter()
        .cloned()
        .map(|dataset| (dataset.id.clone(), dataset))
        .collect();
    let mut views: BTreeMap<_, _> = graph
        .views
        .iter()
        .cloned()
        .map(|view| (view.id.clone(), view))
        .collect();

    for node_id in &patch.remove_node_ids {
        let connected: Vec<_> = edges
            .values()
            .filter(|edge| edge.source == *node_id || edge.target == *node_id)
            .cloned()
            .collect();
        if connected.iter().any(|edge| !edge.locked_fields.is_empty()) {
            continue;
        }
        nodes.remove(node_id);
        for edge in connected {
            edges.remove(&edge.id);
        }
    }
    for edge_id in &patch.remove_edge_ids {
        if edges
            .get(edge_id)
            .is_some_and(|edge| edge.locked_fields.is_empty())
        {
            edges.remove(edge_id);
        }
    }
    for node in &patch.upsert_nodes {
        let merged = nodes
            .get(&node.id)
            .map_or_else(|| node.clone(), |current| merge_node(current, node.clone()));
        nodes.insert(node.id.clone(), merged);
    }
    for edge in &patch.upsert_edges {
        let merged = edges
            .get(&edge.id)
            .map_or_else(|| edge.clone(), |current| merge_edge(current, edge.clone()));
        if nodes.contains_key(&merged.source) && nodes.contains_key(&merged.target) {
            edges.insert(edge.id.clone(), merged);
        }
    }
    for dataset_id in &patch.remove_dataset_ids {
        datasets.remove(dataset_id);
    }
    for dataset in &patch.upsert_datasets {
        datasets.insert(dataset.id.clone(), dataset.clone());
    }
    for view_id in &patch.remove_view_ids {
        views.remove(view_id);
    }
    for view in &patch.upsert_views {
        views.insert(view.id.clone(), view.clone());
    }

    let node_ids: HashSet<_> = nodes.keys().cloned().collect();
    let edge_ids: HashSet<_> = edges.keys().cloned().collect();
    let dataset_ids: HashSet<_> = datasets.keys().cloned().collect();
    let views = views
        .into_values()
        .filter_map(|mut view| {
            if view
                .dataset_id
                .as_ref()
                .is_some_and(|id| !dataset_ids.contains(id))
            {
                return None;
            }
            view.node_ids.retain(|id| node_ids.contains(id));
            view.edge_ids.retain(|id| edge_ids.contains(id));
            Some(view)
        })
        .collect();
    CaseGraph {
        nodes: nodes.into_values().collect(),
        edges: edges.into_values().collect(),
        datasets: datasets.into_values().collect(),
        views,
        ..graph.clone()
    }
}

fn ensure_patch_subset(
    accepted: &CaseGraphPatch,
    proposed: &CaseGraphPatch,
) -> Result<(), VisualError> {
    let approved = accepted.base_revision == proposed.base_revision
        && accepted
            .upsert_nodes
            .iter()
            .all(|item| proposed.upsert_nodes.contains(item))
        && accepted
            .remove_node_ids
            .iter()
            .all(|id| proposed.remove_node_ids.contains(id))
        && accepted
            .upsert_edges
            .iter()
            .all(|item| proposed.upsert_edges.contains(item))
        && accepted
            .remove_edge_ids
            .iter()
            .all(|id| proposed.remove_edge_ids.contains(id))
        && accepted
            .upsert_datasets
            .iter()
            .all(|item| proposed.upsert_datasets.contains(item))
        && accepted
            .remove_dataset_ids
            .iter()
            .all(|id| proposed.remove_dataset_ids.contains(id))
        && accepted
            .upsert_views
            .iter()
            .all(|item| proposed.upsert_views.contains(item))
        && accepted
            .remove_view_ids
            .iter()
            .all(|id| proposed.remove_view_ids.contains(id));
    if !approved {
        return Err(validation("接受补丁包含原提案之外的修改"));
    }
    Ok(())
}

pub async fn create_proposal(
    pool: &SqlitePool,
    input: NewVisualProposal<'_>,
) -> Result<VisualProposal, VisualError> {
    validate_text(input.id, "proposal.id", 200, false)?;
    validate_patch(input.patch)?;
    validate_json(input.summary, "proposal.summary", 0)?;
    if input.patch.base_revision != input.base_revision {
        return Err(validation("proposal 的 base_revision 不一致"));
    }
    let workspace = get_workspace_by_id(pool, input.workspace_id)
        .await?
        .ok_or(VisualError::WorkspaceNotFound)?;
    let candidate = apply_patch(&workspace.graph, input.patch);
    validate_graph(&candidate)?;
    let status = if workspace.revision == input.base_revision {
        VisualProposalStatus::Pending
    } else {
        VisualProposalStatus::Stale
    };
    let patch_json = serde_json::to_string(input.patch)?;
    let summary_json = serde_json::to_string(input.summary)?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO case_visual_proposals \
         (id, workspace_id, base_revision, patch_json, summary_json, status) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(input.id)
    .bind(input.workspace_id)
    .bind(input.base_revision)
    .bind(patch_json)
    .bind(summary_json)
    .bind(status.as_str())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get_proposal(pool, input.id)
        .await?
        .ok_or(VisualError::ProposalNotFound)
}

pub async fn get_proposal(
    pool: &SqlitePool,
    id: &str,
) -> Result<Option<VisualProposal>, VisualError> {
    let query = format!("SELECT {PROPOSAL_COLUMNS} FROM case_visual_proposals WHERE id = ?");
    let row = sqlx::query_as::<_, ProposalRow>(&query)
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(proposal_from_row).transpose()
}

pub async fn list_proposals(
    pool: &SqlitePool,
    workspace_id: &str,
) -> Result<Vec<VisualProposal>, VisualError> {
    let query = format!(
        "SELECT {PROPOSAL_COLUMNS} FROM case_visual_proposals \
         WHERE workspace_id = ? ORDER BY created_at DESC"
    );
    let rows = sqlx::query_as::<_, ProposalRow>(&query)
        .bind(workspace_id)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(proposal_from_row).collect()
}

pub async fn reject_proposal(pool: &SqlitePool, id: &str) -> Result<VisualProposal, VisualError> {
    let mut tx = pool.begin().await?;
    let query = format!("SELECT {PROPOSAL_COLUMNS} FROM case_visual_proposals WHERE id = ?");
    let row = sqlx::query_as::<_, ProposalRow>(&query)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(VisualError::ProposalNotFound)?;
    let proposal = proposal_from_row(row)?;
    if matches!(
        proposal.status,
        VisualProposalStatus::Accepted | VisualProposalStatus::Rejected
    ) {
        return Err(VisualError::ProposalState(
            proposal.status.as_str().to_string(),
        ));
    }
    sqlx::query(
        "UPDATE case_visual_proposals \
         SET status = 'rejected', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ?",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    get_proposal(pool, id)
        .await?
        .ok_or(VisualError::ProposalNotFound)
}

pub async fn resolve_proposal(
    pool: &SqlitePool,
    id: &str,
    accepted_patch: &CaseGraphPatch,
) -> Result<VisualWorkspace, VisualError> {
    validate_patch(accepted_patch)?;
    let mut tx = pool.begin().await?;
    let proposal_query =
        format!("SELECT {PROPOSAL_COLUMNS} FROM case_visual_proposals WHERE id = ?");
    let proposal_row = sqlx::query_as::<_, ProposalRow>(&proposal_query)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(VisualError::ProposalNotFound)?;
    let proposal = proposal_from_row(proposal_row)?;
    if proposal.status == VisualProposalStatus::Stale {
        return Err(VisualError::StaleProposal);
    }
    if proposal.status != VisualProposalStatus::Pending {
        return Err(VisualError::ProposalState(
            proposal.status.as_str().to_string(),
        ));
    }
    ensure_patch_subset(accepted_patch, &proposal.patch)?;
    let workspace_query =
        format!("SELECT {WORKSPACE_COLUMNS} FROM case_visual_workspaces WHERE id = ?");
    let workspace_row = sqlx::query_as::<_, WorkspaceRow>(&workspace_query)
        .bind(&proposal.workspace_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(VisualError::WorkspaceNotFound)?;
    let workspace = workspace_from_row(workspace_row)?;
    if workspace.revision != proposal.base_revision
        || accepted_patch.base_revision != proposal.base_revision
    {
        sqlx::query(
            "UPDATE case_visual_proposals \
             SET status = 'stale', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Err(VisualError::StaleProposal);
    }
    let graph = apply_patch(&workspace.graph, accepted_patch);
    validate_graph(&graph)?;
    let graph_json = serde_json::to_string(&graph)?;
    let layout_json = serde_json::to_string(&workspace.layout)?;
    let next_revision = workspace.revision + 1;
    sqlx::query(
        "UPDATE case_visual_workspaces \
         SET graph_json = ?, revision = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ? AND revision = ?",
    )
    .bind(&graph_json)
    .bind(next_revision)
    .bind(&workspace.id)
    .bind(workspace.revision)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO case_visual_revisions \
         (id, workspace_id, revision, base_revision, graph_json, layout_json, source, summary) \
         VALUES (?, ?, ?, ?, ?, ?, 'ai_merge', ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&workspace.id)
    .bind(next_revision)
    .bind(workspace.revision)
    .bind(&graph_json)
    .bind(&layout_json)
    .bind(&accepted_patch.summary)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE case_visual_proposals \
         SET status = CASE WHEN id = ? THEN 'accepted' ELSE 'stale' END, \
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE workspace_id = ? AND status = 'pending'",
    )
    .bind(id)
    .bind(&workspace.id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    prune_revisions(pool, &workspace.id).await?;
    get_workspace_by_id(pool, &workspace.id)
        .await?
        .ok_or(VisualError::WorkspaceNotFound)
}

pub async fn restore_revision(
    pool: &SqlitePool,
    workspace_id: &str,
    revision: i64,
) -> Result<VisualWorkspace, VisualError> {
    let mut tx = pool.begin().await?;
    let workspace_query =
        format!("SELECT {WORKSPACE_COLUMNS} FROM case_visual_workspaces WHERE id = ?");
    let workspace_row = sqlx::query_as::<_, WorkspaceRow>(&workspace_query)
        .bind(workspace_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(VisualError::WorkspaceNotFound)?;
    let workspace = workspace_from_row(workspace_row)?;
    let snapshot: Option<(String, String)> = sqlx::query_as(
        "SELECT graph_json, layout_json FROM case_visual_revisions \
         WHERE workspace_id = ? AND revision = ?",
    )
    .bind(workspace_id)
    .bind(revision)
    .fetch_optional(&mut *tx)
    .await?;
    let (graph_json, layout_json) = snapshot.ok_or(VisualError::RevisionNotFound)?;
    let graph: CaseGraph = serde_json::from_str(&graph_json)?;
    let layout: Value = serde_json::from_str(&layout_json)?;
    validate_graph(&graph)?;
    validate_json(&layout, "revision.layout", 0)?;
    let next_revision = workspace.revision + 1;
    sqlx::query(
        "UPDATE case_visual_workspaces \
         SET graph_json = ?, layout_json = ?, revision = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE id = ? AND revision = ?",
    )
    .bind(&graph_json)
    .bind(&layout_json)
    .bind(next_revision)
    .bind(workspace_id)
    .bind(workspace.revision)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO case_visual_revisions \
         (id, workspace_id, revision, base_revision, graph_json, layout_json, source, summary) \
         VALUES (?, ?, ?, ?, ?, ?, 'restore', ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(workspace_id)
    .bind(next_revision)
    .bind(workspace.revision)
    .bind(&graph_json)
    .bind(&layout_json)
    .bind(format!("恢复至修订 {revision}"))
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE case_visual_proposals \
         SET status = 'stale', updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
         WHERE workspace_id = ? AND status = 'pending'",
    )
    .bind(workspace_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    prune_revisions(pool, workspace_id).await?;
    get_workspace_by_id(pool, workspace_id)
        .await?
        .ok_or(VisualError::WorkspaceNotFound)
}
