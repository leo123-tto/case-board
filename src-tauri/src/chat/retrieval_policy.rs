use super::tools::{ToolContext, ToolError, ToolResult};
use crate::local_kb::retrieval::{retrieve_local, RetrievalDomain};

pub enum ExternalGateDecision {
    UseLocal(ToolResult),
    AllowExternal,
}

pub async fn local_first_gate(
    ctx: &ToolContext<'_>,
    domain: RetrievalDomain,
    query: &str,
) -> Result<ExternalGateDecision, ToolError> {
    if crate::chat::policy::requires_direct_yuandian(ctx.message_id) {
        return Ok(ExternalGateDecision::AllowExternal);
    }
    let report = retrieve_local(ctx.local_kb, ctx.settings, domain, query)
        .await
        .map_err(ToolError::Runtime)?;
    let stage_summary = report
        .stages
        .iter()
        .map(|stage| {
            format!(
                "{:?}:{:?}:{}",
                stage.kind, stage.confidence, stage.result_count
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    if report.is_sufficient() {
        crate::dlog!(
            "[local-first-retrieval] domain={:?} local={:?} stages=[{}] external=false",
            domain,
            report.confidence,
            stage_summary
        );
        let content = serde_json::to_string_pretty(&serde_json::json!({
            "query": query,
            "local_retrieval": report,
            "_note": "本地检索已达到强命中，Rust 付费网关未调用元典。请优先读取命中文件并据此继续；不要换词重复外查。"
        }))
        .unwrap_or_else(|_| "本地检索强命中，未调用元典。".into());
        return Ok(ExternalGateDecision::UseLocal(ToolResult {
            content,
            yuandian_credits_used: 0,
            kb_hit: true,
        }));
    }
    crate::dlog!(
        "[local-first-retrieval] domain={:?} local={:?} stages=[{}] external=true",
        domain,
        report.confidence,
        stage_summary
    );
    Ok(ExternalGateDecision::AllowExternal)
}
