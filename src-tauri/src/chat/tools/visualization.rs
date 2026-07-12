//! AI 案情可视化工具。所有写入都要求本轮用户已明确同意，并复用数据库领域校验。

use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::db::case_visuals::{
    self, CaseGraph, CaseGraphPatch, NewVisualProposal, NewVisualWorkspace,
};

use super::{Tool, ToolContext, ToolError, ToolResult};

pub struct SaveCaseVisualization;
pub struct ProposeCaseVisualUpdate;
pub struct GetCaseVisualization;

fn source_ref_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "document_id": {"type": "string"},
            "filename": {"type": "string"},
            "locator": {"type": "string"},
            "quote": {"type": "string"}
        },
        "required": ["document_id", "filename"]
    })
}

fn node_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": {"type": "string", "description": "稳定 UUID；更新时必须复用已有 id"},
            "kind": {"type": "string", "enum": ["actor", "event", "claim", "issue", "legal_basis", "element", "defense", "evidence", "amount", "document", "action"]},
            "label": {"type": "string"},
            "detail": {"type": "string"},
            "date": {"type": "string", "description": "仅确定日期可填 YYYY-MM-DD；不确定日期改填 date_label"},
            "date_label": {"type": "string"},
            "phase": {"type": "string"},
            "side": {"type": "string"},
            "status": {"type": "string", "enum": ["confirmed", "our_claim", "opponent_claim", "disputed", "inferred", "unknown"]},
            "importance": {"type": "string", "enum": ["critical", "normal", "context"]},
            "source_refs": {"type": "array", "items": source_ref_schema()},
            "tags": {"type": "array", "items": {"type": "string"}},
            "provenance": {"type": "string", "enum": ["ai", "user"]},
            "locked_fields": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["id", "kind", "label", "status", "source_refs", "provenance"]
    })
}

fn edge_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": {"type": "string"},
            "source": {"type": "string"},
            "target": {"type": "string"},
            "kind": {"type": "string", "enum": ["relates_to", "represents", "contracts_with", "pays", "owes", "guarantees", "causes", "precedes", "supports", "refutes", "proves", "requires", "responds_to"]},
            "label": {"type": "string"},
            "status": {"type": "string", "enum": ["confirmed", "our_claim", "opponent_claim", "disputed", "inferred", "unknown"]},
            "source_refs": {"type": "array", "items": source_ref_schema()},
            "provenance": {"type": "string", "enum": ["ai", "user"]},
            "locked_fields": {"type": "array", "items": {"type": "string"}}
        },
        "required": ["id", "source", "target", "kind", "status", "source_refs", "provenance"]
    })
}

fn dataset_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": {"type": "string"},
            "title": {"type": "string"},
            "columns": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "key": {"type": "string"},
                        "label": {"type": "string"},
                        "type": {"type": "string", "enum": ["text", "number", "date"]}
                    },
                    "required": ["key", "label", "type"]
                }
            },
            "rows": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": {"type": ["string", "number", "null"]}
                }
            },
            "source_refs": {"type": "array", "items": source_ref_schema()}
        },
        "required": ["id", "title", "columns", "rows", "source_refs"]
    })
}

fn view_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": {"type": "string"},
            "kind": {"type": "string", "enum": ["timeline", "relationship", "mindmap", "evidence_matrix", "bar", "line", "heatmap", "bar_table"]},
            "title": {"type": "string"},
            "description": {"type": "string"},
            "node_ids": {"type": "array", "items": {"type": "string"}},
            "edge_ids": {"type": "array", "items": {"type": "string"}},
            "dataset_id": {"type": "string"},
            "config": {
                "type": "object",
                "additionalProperties": {
                    "anyOf": [
                        {"type": "string"},
                        {"type": "number"},
                        {"type": "boolean"},
                        {"type": "array", "items": {"type": "string"}}
                    ]
                }
            }
        },
        "required": ["id", "kind", "title", "config"]
    })
}

fn graph_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "schema_version": {"type": "integer", "enum": [1]},
            "case_id": {"type": "string"},
            "title": {"type": "string"},
            "summary": {"type": "string"},
            "nodes": {"type": "array", "maxItems": 300, "items": node_schema()},
            "edges": {"type": "array", "maxItems": 600, "items": edge_schema()},
            "datasets": {"type": "array", "maxItems": 20, "items": dataset_schema()},
            "views": {"type": "array", "maxItems": 20, "items": view_schema()}
        },
        "required": ["schema_version", "case_id", "title", "summary", "nodes", "edges", "datasets", "views"]
    })
}

fn patch_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "base_revision": {"type": "integer", "minimum": 1},
            "upsert_nodes": {"type": "array", "maxItems": 300, "items": node_schema()},
            "remove_node_ids": {"type": "array", "maxItems": 300, "items": {"type": "string"}},
            "upsert_edges": {"type": "array", "maxItems": 600, "items": edge_schema()},
            "remove_edge_ids": {"type": "array", "maxItems": 600, "items": {"type": "string"}},
            "upsert_datasets": {"type": "array", "maxItems": 20, "items": dataset_schema()},
            "remove_dataset_ids": {"type": "array", "maxItems": 20, "items": {"type": "string"}},
            "upsert_views": {"type": "array", "maxItems": 20, "items": view_schema()},
            "remove_view_ids": {"type": "array", "maxItems": 20, "items": {"type": "string"}},
            "summary": {"type": "string"}
        },
        "required": ["base_revision", "upsert_nodes", "remove_node_ids", "upsert_edges", "remove_edge_ids", "upsert_datasets", "remove_dataset_ids", "upsert_views", "remove_view_ids", "summary"]
    })
}

fn ensure_consent(ctx: &ToolContext<'_>) -> Result<(), ToolError> {
    if !ctx.visualization_consent {
        return Err(ToolError::Runtime(
            "尚未获得用户同意，不能生成或更新案情可视化；请先用一次多选 ask_user 询问。".into(),
        ));
    }
    Ok(())
}

fn parse_graph(args: &Value) -> Result<CaseGraph, ToolError> {
    let value = args
        .get("graph")
        .cloned()
        .ok_or_else(|| ToolError::InvalidArgs("缺必填字段:graph".into()))?;
    let graph: CaseGraph = serde_json::from_value(value)
        .map_err(|error| ToolError::InvalidArgs(format!("graph 结构错误:{error}")))?;
    case_visuals::validate_graph(&graph).map_err(ToolError::Visualization)?;
    Ok(graph)
}

fn parse_patch(args: &Value) -> Result<CaseGraphPatch, ToolError> {
    let value = args
        .get("patch")
        .cloned()
        .ok_or_else(|| ToolError::InvalidArgs("缺必填字段:patch".into()))?;
    serde_json::from_value(value)
        .map_err(|error| ToolError::InvalidArgs(format!("patch 结构错误:{error}")))
}

#[async_trait]
impl Tool for GetCaseVisualization {
    fn name(&self) -> &str {
        "get_case_visualization"
    }

    fn description(&self) -> &str {
        include_str!("descriptions/get_case_visualization.md")
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        })
    }

    async fn execute(&self, _args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        let case_id = ctx.case_id.ok_or(ToolError::NoCaseBound)?;
        let workspace = case_visuals::get_workspace(ctx.pool, case_id)
            .await
            .map_err(ToolError::Visualization)?
            .ok_or_else(|| ToolError::Runtime("本案还没有可视化工作区。".into()))?;
        Ok(ToolResult::plain(
            serde_json::to_string(&json!({
                "workspace_id": workspace.id,
                "revision": workspace.revision,
                "graph": workspace.graph,
                "layout": workspace.layout,
                "updated_at": workspace.updated_at
            }))
            .map_err(|error| ToolError::Runtime(error.to_string()))?,
        ))
    }
}

#[async_trait]
impl Tool for SaveCaseVisualization {
    fn name(&self) -> &str {
        "save_case_visualization"
    }

    fn description(&self) -> &str {
        include_str!("descriptions/save_case_visualization.md")
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "graph": graph_schema(),
                "layout": {"type": "object", "description": "可选的初始布局；通常传空对象，由确定性布局器生成"}
            },
            "required": ["graph"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        ensure_consent(ctx)?;
        let case_id = ctx.case_id.ok_or(ToolError::NoCaseBound)?;
        let graph = parse_graph(args)?;
        if graph.case_id != case_id {
            return Err(ToolError::InvalidArgs(
                "graph.case_id 与当前案件不一致".into(),
            ));
        }
        if case_visuals::get_workspace(ctx.pool, case_id)
            .await
            .map_err(ToolError::Visualization)?
            .is_some()
        {
            return Err(ToolError::Runtime(
                "本案已有可视化工作区，请读取现有结构并调用 propose_case_visual_update 提交更新提案。"
                    .into(),
            ));
        }
        let layout = args.get("layout").cloned().unwrap_or_else(|| json!({}));
        if !layout.is_object() {
            return Err(ToolError::InvalidArgs("layout 必须是对象".into()));
        }
        let workspace_id = Uuid::new_v4().to_string();
        let workspace = case_visuals::create_workspace(
            ctx.pool,
            NewVisualWorkspace {
                id: &workspace_id,
                case_id,
                graph: &graph,
                layout: &layout,
                source_fingerprint: None,
                created_by_message_id: ctx.message_id,
            },
        )
        .await
        .map_err(ToolError::Visualization)?;
        let views: Vec<Value> = workspace
            .graph
            .views
            .iter()
            .map(|view| json!({"id": view.id, "kind": view.kind, "title": view.title}))
            .collect();
        Ok(ToolResult::plain(
            serde_json::to_string(&json!({
                "workspace_id": workspace.id,
                "revision": workspace.revision,
                "title": workspace.graph.title,
                "views": views,
                "message": "案情可视化工作区已保存，可提示用户进入工作台查看和编辑。"
            }))
            .map_err(|error| ToolError::Runtime(error.to_string()))?,
        ))
    }

    fn is_mutating(&self) -> bool {
        true
    }
}

#[async_trait]
impl Tool for ProposeCaseVisualUpdate {
    fn name(&self) -> &str {
        "propose_case_visual_update"
    }

    fn description(&self) -> &str {
        include_str!("descriptions/propose_case_visual_update.md")
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {"patch": patch_schema()},
            "required": ["patch"]
        })
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError> {
        ensure_consent(ctx)?;
        let case_id = ctx.case_id.ok_or(ToolError::NoCaseBound)?;
        let workspace = case_visuals::get_workspace(ctx.pool, case_id)
            .await
            .map_err(ToolError::Visualization)?
            .ok_or_else(|| {
                ToolError::Runtime(
                    "本案还没有可视化工作区，请调用 save_case_visualization 首次创建。".into(),
                )
            })?;
        let patch = parse_patch(args)?;
        if patch.base_revision != workspace.revision {
            return Err(ToolError::Runtime(format!(
                "工作区当前为修订 {}，补丁基于修订 {}；请读取最新工作区后重新生成补丁。",
                workspace.revision, patch.base_revision
            )));
        }
        let summary = json!({
            "nodes_upserted": patch.upsert_nodes.len(),
            "nodes_removed": patch.remove_node_ids.len(),
            "edges_upserted": patch.upsert_edges.len(),
            "edges_removed": patch.remove_edge_ids.len(),
            "datasets_upserted": patch.upsert_datasets.len(),
            "views_upserted": patch.upsert_views.len()
        });
        let proposal_id = Uuid::new_v4().to_string();
        let proposal = case_visuals::create_proposal(
            ctx.pool,
            NewVisualProposal {
                id: &proposal_id,
                workspace_id: &workspace.id,
                base_revision: workspace.revision,
                patch: &patch,
                summary: &summary,
            },
        )
        .await
        .map_err(ToolError::Visualization)?;
        Ok(ToolResult::plain(
            serde_json::to_string(&json!({
                "proposal_id": proposal.id,
                "workspace_id": proposal.workspace_id,
                "base_revision": proposal.base_revision,
                "summary": proposal.summary,
                "status": proposal.status,
                "message": "AI 更新提案已保存，须由用户审阅并选择接受的变更，不会自动覆盖人工内容。"
            }))
            .map_err(|error| ToolError::Runtime(error.to_string()))?,
        ))
    }

    fn is_mutating(&self) -> bool {
        true
    }
}
