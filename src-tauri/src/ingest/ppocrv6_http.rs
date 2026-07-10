//! PP-OCRv6(百度 AI Studio,纯文字行)HTTP 客户端 + 去水印过滤(2026-06-13)。
//!
//! 背景(胡彬律师反馈 + 尽调实测,见 docs 外部笔记「带水印调档件 OCR 方法」):
//! 工商调档件套"市场监管/数据局/档案室验证章 + 公司名"水印,且**贯穿正文**。
//! - PaddleOCR-VL-1.6(带版面分析)会把水印当正文块按坐标排进正文 → 关键字段被淹没,基本不可用。
//! - PP-OCRv6 **只做行级识别、不做版面**,水印自成独立短行,可按"高频重复 + 水印词表"剔除,
//!   公司名称/注册资本/股东/法定代表人等登记字段可读。
//!
//! 与 paddle_vl_http 共用同一 AIStudio job 接口 + 同一 token,仅 MODEL 与结果字段不同:
//!   - MODEL = "PP-OCRv6"
//!   - 文字在 `result.ocrResults[i].prunedResult.rec_texts`(逐行字符串数组),非 VL 的 markdown
//!
//! 输出 = 去水印正文 + 末尾附「被过滤掉的行」供律师核对(去水印是启发式,可能误删;绝不静默丢内容)。

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

const BASE_URL: &str = "https://paddleocr.aistudio-app.com/api/v2";
const MODEL: &str = "PP-OCRv6";
const POLL_INTERVAL_MS: u64 = 3000;
const HTTP_TIMEOUT_SEC: u64 = 60;
const UPLOAD_TIMEOUT_SEC: u64 = 180;

/// 调 AI Studio PP-OCRv6 抽一个文件,去水印后返回正文(+ 被过滤行的核对附录)。
/// `timeout_secs`:从提交到拿到结果的总超时。**失败直接透传 Err,不回退**(调用方已明确选去水印)。
pub async fn extract_with_ppocrv6(
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
        return Err("PP-OCRv6 当日免费额度已用完(HTTP 429)".into());
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
    let json_url = loop {
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
                crate::dlog!("[ppocrv6] 轮询请求失败(继续重试): {}", e);
                continue;
            }
        };
        if !resp.status().is_success() {
            crate::dlog!("[ppocrv6] 轮询 HTTP {}(继续重试)", resp.status());
            continue;
        }
        let v: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                crate::dlog!("[ppocrv6] 轮询响应解析失败(继续重试): {}", e);
                continue;
            }
        };

        match v.pointer("/data/state").and_then(|s| s.as_str()) {
            Some("done") => {
                break v
                    .pointer("/data/resultUrl/jsonUrl")
                    .and_then(|s| s.as_str())
                    .ok_or("state=done 但 resultUrl.jsonUrl 缺失")?
                    .to_string();
            }
            Some("failed") => {
                let msg = v
                    .pointer("/data/errorMsg")
                    .and_then(|s| s.as_str())
                    .unwrap_or("(无说明)");
                return Err(format!("PP-OCRv6 解析失败: {}", msg));
            }
            other => {
                if let Some(tx) = poll_tx {
                    let phase = match other {
                        Some("pending") => "queued",
                        _ => "processing",
                    };
                    let _ = tx.send(crate::ingest::ocr::OcrPollUpdate {
                        phase: phase.to_string(),
                        elapsed_secs: start.elapsed().as_secs(),
                        pages_done: None,
                        pages_total: None,
                    });
                }
                continue;
            }
        }
    };

    // ---- Step 3: 拉 JSONL,取 rec_texts 逐行 → 去水印 ----
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

    let (lines, page_count) = rec_texts_from_jsonl(&jsonl)?;
    let (cleaned, removed) = filter_watermarks(&lines, page_count);

    let body_text = cleaned.join("\n");
    if body_text.trim().chars().count() < 30 {
        return Err(format!(
            "PP-OCRv6 去水印后正文太短({} 字),可能识别为空或被过度过滤",
            body_text.trim().chars().count()
        ));
    }

    // 被过滤行去重后附末尾,供律师核对(去水印是启发式,可能误删表头/字段)。
    let mut out = body_text;
    if !removed.is_empty() {
        let mut seen = std::collections::HashSet::new();
        let distinct: Vec<&String> = removed
            .iter()
            .filter(|l| seen.insert((*l).clone()))
            .collect();
        out.push_str("\n\n---以下为去水印过滤掉的行(供核对,可能误删要紧内容,请比对原件)---\n");
        for l in distinct {
            out.push_str(l);
            out.push('\n');
        }
    }
    Ok(out)
}

/// 从 PP-OCRv6 结果 JSONL 取逐行文本 + 页数(= ocrResults 条目数,一条约一页)。
fn rec_texts_from_jsonl(jsonl: &str) -> Result<(Vec<String>, usize), String> {
    let mut lines: Vec<String> = Vec::new();
    let mut page_count: usize = 0;
    for raw in jsonl.lines() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let v: serde_json::Value =
            serde_json::from_str(raw).map_err(|e| format!("解析结果行失败: {}", e))?;
        let Some(results) = v.pointer("/result/ocrResults").and_then(|r| r.as_array()) else {
            continue;
        };
        for res in results {
            page_count += 1;
            if let Some(texts) = res
                .pointer("/prunedResult/rec_texts")
                .and_then(|t| t.as_array())
            {
                for t in texts {
                    if let Some(s) = t.as_str() {
                        lines.push(s.to_string());
                    }
                }
            }
        }
    }
    if lines.is_empty() {
        return Err("结果 JSONL 里没有 ocrResults.prunedResult.rec_texts(可能返回结构变了)".into());
    }
    Ok((lines, page_count))
}

/// 去水印过滤(两条启发式,见外部笔记)。返回 (正文行, 被剔除行)。
///
/// 1. **高频短行**:trim 后出现次数 ≥ max(3, ⌈页数×0.3⌉) 且 ≤12 字 → 判水印/章戳重复。
/// 2. **水印词表碎片**:≤8 字短行且命中"市场监管/数据局/档案室/验证码/验证章"等(碎片化的章戳),
///    保守只删短碎片,避免误删"登记机关:上海市…管理局"这类完整字段。公司名碎片主要靠启发式 1。
fn filter_watermarks(lines: &[String], page_count: usize) -> (Vec<String>, Vec<String>) {
    let threshold = std::cmp::max(3, ((page_count as f64) * 0.3).ceil() as usize);

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for l in lines {
        let t = l.trim();
        if !t.is_empty() {
            *counts.entry(t).or_insert(0) += 1;
        }
    }

    const WORDS: &[&str] = &[
        "市场监管",
        "市场监督管理",
        "数据局",
        "档案室",
        "验证码",
        "验证章",
        "档案查询",
    ];

    let mut cleaned: Vec<String> = Vec::new();
    let mut removed: Vec<String> = Vec::new();
    for l in lines {
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        let len = t.chars().count();
        let high_freq = len <= 12 && counts.get(t).copied().unwrap_or(0) >= threshold;
        let wordlist_fragment = len <= 8 && WORDS.iter().any(|w| t.contains(w));
        if high_freq || wordlist_fragment {
            removed.push(t.to_string());
        } else {
            cleaned.push(t.to_string());
        }
    }
    (cleaned, removed)
}
