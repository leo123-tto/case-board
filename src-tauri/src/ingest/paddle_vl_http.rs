//! PaddleOCR VL-1.6(百度 AI Studio 星河社区)HTTP 客户端(2026-06-12)。
//!
//! 背景:作者实测两份真实材料 + 61 页审计报告,VL-1.6 与 MinerU vlm 精度打平
//! (文字/表格/旋转页全命中),速度约快一倍(61 页 14s vs 28s),免费额度
//! 20,000 页/天/模型(MinerU 为 1,000 页/天)。作为 MinerU 的主/备可切换方案接入。
//!
//! API(作者 token 实测,2026-06-12):
//!   1. POST `{base}/ocr/jobs` multipart(file + model=PaddleOCR-VL-1.6)→ `data.jobId`
//!   2. 轮询 GET `{base}/ocr/jobs/{jobId}` → `data.state`:pending/running/done/failed
//!   3. done → 拉 `data.resultUrl.jsonUrl`(JSONL),每行
//!      `result.layoutParsingResults[].markdown.text` 即一页 markdown,拼接返回
//!
//! 限制(切换主力时必须防):
//!   - **单文件 100 页,超出部分静默忽略不解析** —— 用 extractProgress 的
//!     extractedPages < totalPages 检测,截断按失败处理(让上层切 MinerU,600 页上限)
//!   - 每模型 20,000 页/天,超出返回 429

use std::path::Path;
use std::time::Duration;

const BASE_URL: &str = "https://paddleocr.aistudio-app.com/api/v2";
const MODEL: &str = "PaddleOCR-VL-1.6";
const POLL_INTERVAL_MS: u64 = 3000;
const HTTP_TIMEOUT_SEC: u64 = 60;
const UPLOAD_TIMEOUT_SEC: u64 = 180;

/// 调 AI Studio PaddleOCR VL-1.6 抽一个文件的 markdown。
///
/// `timeout_secs` 是从提交到拿到结果的总超时(与 mineru_http 同语义)。
///
/// `poll_tx`:可选的轮询进度回传通道(2026-06-14),每拍把后端 state 透传到前端
/// (显示"排队 / 识别中(已 N 秒)"),治大图扫描件"看着卡死"。
pub async fn extract_with_paddle_vl(
    path: &Path,
    token: &str,
    timeout_secs: u64,
    poll_tx: Option<&tokio::sync::mpsc::UnboundedSender<crate::ingest::ocr::OcrPollUpdate>>,
) -> Result<String, String> {
    let filename = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| "文件名解析失败".to_string())?
        .to_string();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(HTTP_TIMEOUT_SEC))
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {}", e))?;

    // ---- Step 1: multipart 提交任务 ----
    let (file_body, file_length) = crate::ingest::ocr::streaming_file_body(path).await?;
    let part =
        reqwest::multipart::Part::stream_with_length(file_body, file_length).file_name(filename);
    let form = reqwest::multipart::Form::new()
        .text("model", MODEL)
        .part("file", part);

    let resp = client
        .post(format!("{}/ocr/jobs", BASE_URL))
        .bearer_auth(token)
        .multipart(form)
        .timeout(Duration::from_secs(UPLOAD_TIMEOUT_SEC))
        .send()
        .await
        .map_err(|e| format!("提交任务失败: {}", e))?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if status.as_u16() == 429 {
        return Err("PaddleOCR 当日免费额度(20,000 页)已用完(HTTP 429)".into());
    }
    if !status.is_success() {
        return Err(format!(
            "提交任务 HTTP {}: {}",
            status.as_u16(),
            body.chars().take(300).collect::<String>()
        ));
    }
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("解析提交响应失败: {}", e))?;
    let job_id = v
        .pointer("/data/jobId")
        .and_then(|x| {
            x.as_str()
                .map(String::from)
                .or_else(|| x.as_i64().map(|n| n.to_string()))
        })
        .ok_or_else(|| format!("提交响应缺 jobId: {:.200}", body))?;

    // ---- Step 2: 轮询直到 done / failed / 超时 ----
    let start = std::time::Instant::now();
    let (json_url, extracted, total) = loop {
        if start.elapsed().as_secs() > timeout_secs {
            return Err(format!(
                "轮询超时 {}s(jobId={}),可能服务端排队中",
                timeout_secs, job_id
            ));
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;

        let resp = match client
            .get(format!("{}/ocr/jobs/{}", BASE_URL, job_id))
            .bearer_auth(token)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                crate::dlog!("[paddle_vl] 轮询请求失败(继续重试): {}", e);
                continue;
            }
        };
        if !resp.status().is_success() {
            crate::dlog!("[paddle_vl] 轮询 HTTP {}(继续重试)", resp.status());
            continue;
        }
        let v: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                crate::dlog!("[paddle_vl] 轮询响应解析失败(继续重试): {}", e);
                continue;
            }
        };

        match v.pointer("/data/state").and_then(|s| s.as_str()) {
            Some("done") => {
                let url = v
                    .pointer("/data/resultUrl/jsonUrl")
                    .and_then(|s| s.as_str())
                    .ok_or("state=done 但 resultUrl.jsonUrl 缺失")?
                    .to_string();
                let extracted = v
                    .pointer("/data/extractProgress/extractedPages")
                    .and_then(|n| n.as_i64());
                let total = v
                    .pointer("/data/extractProgress/totalPages")
                    .and_then(|n| n.as_i64());
                break (url, extracted, total);
            }
            Some("failed") => {
                let msg = v
                    .pointer("/data/errorMsg")
                    .and_then(|s| s.as_str())
                    .unwrap_or("(无说明)");
                return Err(format!("PaddleOCR 解析失败: {}", msg));
            }
            // pending / running / 未知状态 → 上报进度后继续轮询
            other => {
                if let Some(tx) = poll_tx {
                    let phase = match other {
                        Some("pending") => "queued",
                        _ => "processing", // running / 未知都按"识别中"处理
                    };
                    let pages_done = v
                        .pointer("/data/extractProgress/extractedPages")
                        .and_then(|n| n.as_i64());
                    let pages_total = v
                        .pointer("/data/extractProgress/totalPages")
                        .and_then(|n| n.as_i64());
                    let _ = tx.send(crate::ingest::ocr::OcrPollUpdate {
                        phase: phase.to_string(),
                        elapsed_secs: start.elapsed().as_secs(),
                        pages_done,
                        pages_total,
                    });
                }
                continue;
            }
        }
    };

    // 100 页上限静默截断检测:截断 = 失败(让上层切 MinerU,别让用户拿到半份文档)
    if let (Some(e), Some(t)) = (extracted, total) {
        if e < t {
            return Err(format!(
                "文件 {} 页超过 PaddleOCR 单文件 100 页上限,仅解析了 {} 页(截断按失败处理)",
                t, e
            ));
        }
    }

    // ---- Step 3: 拉 JSONL 结果,拼接每页 markdown ----
    let jsonl = client
        .get(&json_url)
        .send()
        .await
        .map_err(|e| format!("下载结果 JSONL 失败: {}", e))?
        .error_for_status()
        .map_err(|e| format!("下载结果 JSONL HTTP 错误: {}", e))?
        .text()
        .await
        .map_err(|e| format!("读结果 JSONL 失败: {}", e))?;

    let md = markdown_from_jsonl(&jsonl)?;
    if md.trim().chars().count() < 30 {
        return Err(format!(
            "PaddleOCR 返回的 markdown 太短({} 字),可能是空文档",
            md.trim().chars().count()
        ));
    }
    Ok(md)
}

/// 从结果 JSONL 拼出整份 markdown(每行一个任务分片,每个分片含若干页)。
fn markdown_from_jsonl(jsonl: &str) -> Result<String, String> {
    let mut pages: Vec<String> = Vec::new();
    for line in jsonl.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(line).map_err(|e| format!("解析结果行失败: {}", e))?;
        let Some(results) = v
            .pointer("/result/layoutParsingResults")
            .and_then(|r| r.as_array())
        else {
            continue;
        };
        for res in results {
            if let Some(text) = res.pointer("/markdown/text").and_then(|t| t.as_str()) {
                pages.push(text.to_string());
            }
        }
    }
    if pages.is_empty() {
        return Err("结果 JSONL 里没有 layoutParsingResults.markdown(可能返回结构变了)".into());
    }
    // 每页前插页码标记(对齐本机 vision 的 `--- 第 N 页 ---`),给「搜索定位到页」用。
    // 单页不插(免干扰);多页才插。
    if pages.len() <= 1 {
        return Ok(pages.join("\n\n"));
    }
    let mut out = String::new();
    for (i, p) in pages.iter().enumerate() {
        if i > 0 {
            out.push_str("\n\n");
        }
        out.push_str(&format!("--- 第 {} 页 ---\n\n", i + 1));
        out.push_str(p);
    }
    Ok(out)
}
