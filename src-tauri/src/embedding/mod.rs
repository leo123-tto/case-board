//! Embedding 客户端(V0.3.3 · 语义检索基础设施)。
//!
//! OpenAI 兼容 `/embeddings` 接口 —— 硅基流动（`BAAI/bge-m3`，默认，1024 维）、
//! 智谱(`embedding-3`,2048 维)等都兼容。用户在设置填 endpoint + model + key;
//! 留空则语义检索**禁用、回退现有关键词选材料**(AI 无感)。
//!
//! 向量存本地文件(`embeddings/<case_id>.json`,不碰 DB → 无 migration)。
//! 错误透传真错(已知坑#8),不用固定文案。
//!
//! 切片 + 向量索引 + 语义检索见子模块 [`index`](self::index)。

pub mod index;

use crate::credentials_bridge::{BridgeCredentialConsumer, PendingCredentialSource};
use serde::Deserialize;
use std::time::Duration;

/// 默认 endpoint / model：硅基流动 bge-m3。设置留空时用这两个值兜底。
pub const DEFAULT_ENDPOINT: &str = "https://api.siliconflow.cn/v1/embeddings";
pub const DEFAULT_MODEL: &str = "BAAI/bge-m3";

pub fn credential_source() -> PendingCredentialSource {
    PendingCredentialSource::pending(
        BridgeCredentialConsumer::EmbeddingProvider,
        "settings:embedding_api_key",
        "embedding",
    )
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

/// 批量把文本转向量。OpenAI 兼容:`POST {model, input: [texts]}` → `data[].embedding`。
/// 返回顺序与输入对齐。空输入返回空。key 空报错(调用方应先判空跳过)。
pub async fn embed(
    endpoint: &str,
    model: &str,
    credential: &PendingCredentialSource,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, String> {
    if texts.is_empty() {
        return Ok(vec![]);
    }
    let ep = if endpoint.trim().is_empty() {
        DEFAULT_ENDPOINT
    } else {
        endpoint.trim()
    };
    let md = if model.trim().is_empty() {
        DEFAULT_MODEL
    } else {
        model.trim()
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))?;
    let body = serde_json::json!({ "model": md, "input": texts });
    let material = credential.issue_material().await?;
    let request = material.with_secret(|secret| {
        client
            .post(ep)
            .header("Authorization", format!("Bearer {secret}"))
            .json(&body)
    });
    let resp = request
        .send()
        .await
        .map_err(|e| format!("请求 embedding 失败: {}", material.redact(&e.to_string())))?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| {
        format!(
            "读取 embedding 响应失败: {}",
            material.redact(&e.to_string())
        )
    })?;
    let text = material.redact(&text);
    if !status.is_success() {
        return Err(format!(
            "embedding API {}: {}",
            status,
            text.chars().take(200).collect::<String>()
        ));
    }
    let parsed: EmbeddingResponse = serde_json::from_str(&text).map_err(|e| {
        format!(
            "解析 embedding 响应失败: {e} · {}",
            text.chars().take(200).collect::<String>()
        )
    })?;
    Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
}

/// 验证 embedding 配置:embed 一个探针词,成功返回向量维度(给设置页验证按钮显示)。
pub async fn verify(
    endpoint: &str,
    model: &str,
    credential: &PendingCredentialSource,
) -> Result<usize, String> {
    let v = embed(endpoint, model, credential, &["法律检索探针".to_string()]).await?;
    v.first()
        .map(|e| e.len())
        .ok_or_else(|| "embedding 返回空向量".to_string())
}

/// 余弦相似度。范围 [-1, 1],越大越相似。维度不一致 / 空向量返回 0。
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}
