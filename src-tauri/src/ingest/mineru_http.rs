//! MinerU 精准 API Rust HTTP 客户端(2026-05-25 V0.1.10 加,替代 npm 的 mineru-open-api CLI)。
//!
//! 触发原因:朋友实测 V0.1.9 报告"mineru-open-api 未安装"。查实 `mineru-open-api` 是 npm
//! 包的 Node.js 脚本,**无法**作为二进制打包进 dmg。改用纯 Rust 直接调 HTTP API,零外部依赖。
//!
//! API 参考:`docs/MinerU精准解析API使用整理.md` 第 5 节「本地文件批量上传解析」
//!
//! 流程(单文件):
//!   1. POST `/api/v4/file-urls/batch` 申请上传 URL,拿 `batch_id` + 1 个 file_url
//!   2. PUT 文件二进制到 file_url(**不设 Content-Type**,这是官方要求)
//!   3. 轮询 GET `/api/v4/extract-results/batch/{batch_id}` 直到 state=done
//!      (轮询间隔 3s,1000 次/分钟限制下安全)
//!   4. 下载 `full_zip_url`,zip 解压,读 `full.md` 返回
//!
//! 限制:
//!   - 单文件 ≤200MB / ≤200 页
//!   - 提交接口 50 文件/分钟(已有上层 SubmitThrottle 兜底)
//!   - 查询接口 1000 次/分钟
//!   - 结果文件 30 天有效期(我们抽出 full.md 后落本地不依赖远程)

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

const BASE_URL: &str = "https://mineru.net/api/v4";
const POLL_INTERVAL_MS: u64 = 3000;
const HTTP_TIMEOUT_SEC: u64 = 60;
/// 200MB 上限文件在普通上行带宽下可能明显超过 60 秒；仅上传请求放宽，轮询仍保持短超时。
const UPLOAD_TIMEOUT_SEC: u64 = 300;
/// 下载结果 zip 的重试次数 / 退避(MinerU 处理已成功、zip_url 有效 → 重下不额外烧积分,
/// 救 openxlab 结果 CDN 偶发抖动;持续被网络层拦截则重试无用,见 classify_dl_err 的提示)。
const ZIP_DL_RETRIES: u32 = 4;
const ZIP_DL_BACKOFF_MS: u64 = 1500;

/// 通用响应壳:`{"code": 0|200, "data": {...}, "msg": "ok"}`
#[derive(Debug, Deserialize)]
struct ApiEnvelope<T> {
    code: i32,
    data: Option<T>,
    #[serde(default)]
    msg: Option<String>,
}

#[derive(Debug, Deserialize)]
struct BatchUploadData {
    batch_id: String,
    file_urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BatchResultData {
    #[allow(dead_code)]
    batch_id: String,
    extract_result: Vec<FileResult>,
}

#[derive(Debug, Deserialize)]
struct FileResult {
    #[allow(dead_code)]
    file_name: String,
    state: String,
    full_zip_url: Option<String>,
    err_msg: Option<String>,
}

/// 调 MinerU HTTP API 抽一个文件的纯文本 Markdown。
///
/// `model` 一般传 `"vlm"`(官网对复杂文档推荐);HTML 文件该传 `"MinerU-HTML"`。
/// `timeout_secs` 是从提交到拿到结果的总超时,不是单次 HTTP 超时。
pub async fn extract_with_mineru_http(
    path: &Path,
    token: &str,
    model: &str,
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

    // ---- Step 1: 申请上传 URL ----
    let mut file_obj = serde_json::Map::new();
    file_obj.insert("name".to_string(), serde_json::Value::String(filename));
    file_obj.insert("is_ocr".to_string(), serde_json::Value::Bool(true));
    let req_body = serde_json::json!({
        "files": [serde_json::Value::Object(file_obj)],
        "model_version": model,
        "language": "ch",
        "enable_formula": true,
        "enable_table": true,
    });

    let resp = client
        .post(format!("{}/file-urls/batch", BASE_URL))
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .json(&req_body)
        .send()
        .await
        .map_err(|e| format!("申请上传 URL 失败: {}", e))?;

    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "申请上传 URL HTTP {}: {}",
            code,
            body.chars().take(300).collect::<String>()
        ));
    }

    let env: ApiEnvelope<BatchUploadData> = resp
        .json()
        .await
        .map_err(|e| format!("解析上传 URL 响应失败: {}", e))?;

    // MinerU 用 code=0 表示成功(有的接口用 200)
    if env.code != 0 && env.code != 200 {
        return Err(format!(
            "MinerU 业务错误 code={}: {}",
            env.code,
            env.msg.unwrap_or_default()
        ));
    }
    let upload_data = env.data.ok_or("响应缺 data 字段")?;
    let batch_id = upload_data.batch_id;
    let upload_url = upload_data
        .file_urls
        .into_iter()
        .next()
        .ok_or("响应 file_urls 为空")?;

    // ---- Step 2: PUT 上传文件二进制 ----
    // 官方要求:**不设 Content-Type**
    let (file_body, file_length) = crate::ingest::ocr::streaming_file_body(path).await?;
    let resp = client
        .put(&upload_url)
        .header(reqwest::header::CONTENT_LENGTH, file_length)
        .body(file_body)
        .timeout(Duration::from_secs(UPLOAD_TIMEOUT_SEC))
        .send()
        .await
        .map_err(|e| format!("上传文件失败: {}", e))?;

    if !resp.status().is_success() {
        let code = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "上传 HTTP {}: {}",
            code,
            body.chars().take(200).collect::<String>()
        ));
    }

    // ---- Step 3: 轮询直到 done / failed / 超时 ----
    let start = std::time::Instant::now();
    let zip_url = loop {
        if start.elapsed().as_secs() > timeout_secs {
            return Err(format!(
                "轮询超时 {}s(batch_id={}),可能服务端排队中",
                timeout_secs, batch_id
            ));
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;

        let resp = match client
            .get(format!("{}/extract-results/batch/{}", BASE_URL, batch_id))
            .bearer_auth(token)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                crate::dlog!("[mineru_http] 轮询请求失败(继续重试): {}", e);
                continue;
            }
        };

        if !resp.status().is_success() {
            crate::dlog!("[mineru_http] 轮询 HTTP {}(继续重试)", resp.status());
            continue;
        }

        let env: ApiEnvelope<BatchResultData> = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                crate::dlog!("[mineru_http] 轮询响应解析失败(继续重试): {}", e);
                continue;
            }
        };

        let Some(data) = env.data else {
            continue;
        };
        let Some(file_result) = data.extract_result.into_iter().next() else {
            continue;
        };

        match file_result.state.as_str() {
            "done" => {
                let url = file_result
                    .full_zip_url
                    .ok_or("state=done 但 full_zip_url 缺失")?;
                break url;
            }
            "failed" => {
                return Err(format!(
                    "MinerU 解析失败: {}",
                    file_result.err_msg.unwrap_or_else(|| "(无说明)".into())
                ));
            }
            // pending / running / converting / waiting-file → 上报进度后继续轮询
            other => {
                if let Some(tx) = poll_tx {
                    let phase = match other {
                        "pending" | "waiting-file" => "queued",
                        "converting" => "converting",
                        _ => "processing", // running / 未知
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

    // ---- Step 4: 下载结果 zip(带重试)+ 解压找 full.md ----
    // MinerU 处理已成功(积分已扣)、zip_url 是有效签名 URL → 下载偶发失败时重试不再烧积分。
    // 2026-06-01 真机暴露:整批 PDF 全卡在这步 —— openxlab.org.cn 结果 CDN 被网络层 TLS 重置。
    let mut dl_err = String::new();
    for attempt in 1..=ZIP_DL_RETRIES {
        match download_zip_bytes(&client, &zip_url).await {
            Ok(bytes) => return extract_full_md_from_zip(bytes),
            Err(e) => dl_err = e,
        }
        if attempt < ZIP_DL_RETRIES {
            crate::dlog!(
                "[mineru_http] 下载结果 zip 第 {}/{} 次失败,重试: {}",
                attempt,
                ZIP_DL_RETRIES,
                dl_err
            );
            tokio::time::sleep(Duration::from_millis(ZIP_DL_BACKOFF_MS * attempt as u64)).await;
        }
    }
    Err(dl_err)
}

/// 下载结果 zip 字节;失败翻成可操作错误(尤其连不上 openxlab 结果 CDN 时)。
async fn download_zip_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| classify_dl_err(&e))?;
    let resp = resp
        .error_for_status()
        .map_err(|e| format!("下载结果 zip HTTP 错误: {}", e))?;
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| classify_dl_err(&e))
}

/// 连接/超时类错误 → 可操作提示(MinerU 结果存 openxlab CDN,常被线路层 TLS 重置)。
fn classify_dl_err(e: &reqwest::Error) -> String {
    if e.is_connect() || e.is_timeout() {
        format!(
            "MinerU 已处理完成,但从你当前网络连不上存放结果的 CDN(cdn-mineru.openxlab.org.cn / \
             openxlab.org.cn,TLS 连接被重置或超时)。多为线路/网络拦截(已确认与系统代理无关)。\
             请换个网络(如手机热点)后对该文件「重新抽取」即可;注意重抽会再消耗 MinerU 积分。原始错误: {}",
            e
        )
    } else {
        format!("下载结果 zip 失败: {}", e)
    }
}

/// 从结果 zip 字节里找 full.md 返回其内容。
fn extract_full_md_from_zip(zip_bytes: Vec<u8>) -> Result<String, String> {
    let cursor = std::io::Cursor::new(zip_bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("解析 zip 失败: {}", e))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("读 zip entry {} 失败: {}", i, e))?;
        let name = entry.name().to_string();
        // full.md 可能在 zip 根或子目录,后缀匹配即可
        if name.ends_with("full.md") {
            let mut content = String::new();
            entry
                .read_to_string(&mut content)
                .map_err(|e| format!("读 full.md 失败: {}", e))?;
            if content.trim().chars().count() < 30 {
                return Err(format!(
                    "MinerU 返回的 full.md 太短({} 字),可能是空文档",
                    content.trim().chars().count()
                ));
            }
            return Ok(content);
        }
    }

    Err("zip 里没找到 full.md(可能 MinerU 返回结构变了)".into())
}
