//! 案件级批处理管线:扫描完后台跑字段抽取,通过 Tauri Event 推送进度。
//!
//! 设计:
//!   - 输入: case_id + 该案件所有 documents
//!   - 流程: 对每个 doc 跑 extractor → 写入 documents.extracted_fields + extraction_status
//!   - 每个文档处理前后都 emit 一次 Event 给前端
//!   - 不阻塞调用方:在 tokio task 里跑
//!
//! 前端订阅 `extraction_progress` 事件即可看到实时进度。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use futures::stream::{self, StreamExt};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

use crate::db::documents::Document;
use crate::ingest::extractor::{extract_one, ExtractResult};
use crate::ingest::ocr::OcrContext;
use crate::llm;
use crate::settings;

/// 三轮动态降级:8 路 → 4 路 → 1 路
///
/// 2026-05-25 V0.1.8 加(替代原来固定 8 路):MinerU 精准 API 偶发限流时,
/// 第 1 轮失败的进第 2 轮(并发减半),第 2 轮还失败进第 3 轮(单线程串行),
/// 第 3 轮失败才算真失败,落 last_error。"要把它提取完毕"——作者原话。
const ROUND_CONCURRENCY: [usize; 3] = [8, 4, 1];

/// 每轮之间的缓冲 sleep(秒),给服务端限流计数器恢复
const INTER_ROUND_SLEEP_SEC: u64 = 3;

/// MinerU "提交任务"接口最小间隔(毫秒)
///
/// 官网限流:**50 文件/分钟**(提交任务接口共用频控,详见 docs/MinerU精准解析API使用整理.md 第 12 节)。
/// 1400ms 间隔 = ~43 次/分钟,留 7 次 buffer 避免撞顶。
/// 节流只对**云端 OCR**生效(本机 vision / pdftotext / textutil 不消耗配额)。
const SUBMIT_MIN_INTERVAL_MS: u64 = 1400;

/// 全应用同一时间只跑一个案件级抽取管线。案内仍按 8→4→1 并发，所以百份材料不会退化成
/// 单文件串行；这里防止用户在多个案件连续点击刷新/重试后形成 N×8 并发、进度事件互相覆盖。
static EXTRACTION_PIPELINE_SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

#[derive(Default)]
struct ExtractionCancel {
    cancelled: AtomicBool,
    notify: tokio::sync::Notify,
}

impl ExtractionCancel {
    fn cancel(&self) {
        self.cancelled.store(true, AtomicOrdering::SeqCst);
        self.notify.notify_waiters();
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(AtomicOrdering::SeqCst)
    }

    async fn cancelled(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

static EXTRACTION_CANCEL_REGISTRY: OnceLock<
    std::sync::Mutex<HashMap<String, Vec<Weak<ExtractionCancel>>>>,
> = OnceLock::new();

fn extraction_cancel_registry(
) -> &'static std::sync::Mutex<HashMap<String, Vec<Weak<ExtractionCancel>>>> {
    EXTRACTION_CANCEL_REGISTRY.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

struct ExtractionRegistration {
    case_id: String,
    signal: Arc<ExtractionCancel>,
}

impl ExtractionRegistration {
    fn new(case_id: &str) -> Self {
        let signal = Arc::new(ExtractionCancel::default());
        if let Ok(mut registry) = extraction_cancel_registry().lock() {
            let entries = registry.entry(case_id.to_string()).or_default();
            entries.retain(|entry| entry.strong_count() > 0);
            entries.push(Arc::downgrade(&signal));
        }
        Self {
            case_id: case_id.to_string(),
            signal,
        }
    }
}

impl Drop for ExtractionRegistration {
    fn drop(&mut self) {
        if let Ok(mut registry) = extraction_cancel_registry().lock() {
            if let Some(entries) = registry.get_mut(&self.case_id) {
                let own_signal = Arc::as_ptr(&self.signal);
                entries.retain(|entry| entry.as_ptr() != own_signal && entry.strong_count() > 0);
                if entries.is_empty() {
                    registry.remove(&self.case_id);
                }
            }
        }
    }
}

/// 删除案件前取消同案所有运行中或排队中的抽取任务。
pub fn cancel_case_extraction(case_id: &str) -> usize {
    let Ok(mut registry) = extraction_cancel_registry().lock() else {
        return 0;
    };
    let Some(entries) = registry.remove(case_id) else {
        return 0;
    };
    entries
        .into_iter()
        .filter_map(|entry| entry.upgrade())
        .map(|signal| {
            signal.cancel();
            1usize
        })
        .sum()
}

/// 真正跨案件、跨重试轮次共享的 MinerU 提交节流器。
static GLOBAL_SUBMIT_THROTTLE: OnceLock<SubmitThrottle> = OnceLock::new();

fn global_submit_throttle() -> &'static SubmitThrottle {
    GLOBAL_SUBMIT_THROTTLE.get_or_init(SubmitThrottle::new)
}

/// 全局节流闸门 —— 控制 MinerU API 提交频率,避开 50 文件/分钟限流。
///
/// 跨所有 buffer_unordered task 共享(Arc 包裹),跨三轮重试也共享。
/// 实现:Mutex 保护 last_submit 时间戳,acquire 时计算需要等多久,
/// 释放锁后 sleep,再回去更新时间戳(避免持锁 sleep 串行化所有 task)。
pub struct SubmitThrottle {
    last_submit: tokio::sync::Mutex<std::time::Instant>,
    min_interval: Duration,
}

impl Default for SubmitThrottle {
    fn default() -> Self {
        Self::new()
    }
}

impl SubmitThrottle {
    pub fn new() -> Self {
        Self {
            // 初始化"60 秒前",首次 acquire 不等
            last_submit: tokio::sync::Mutex::new(
                std::time::Instant::now() - Duration::from_secs(60),
            ),
            min_interval: Duration::from_millis(SUBMIT_MIN_INTERVAL_MS),
        }
    }

    pub async fn acquire(&self) {
        loop {
            let mut last = self.last_submit.lock().await;
            let now = std::time::Instant::now();
            let elapsed = now.duration_since(*last);
            if elapsed >= self.min_interval {
                *last = now;
                return;
            }
            let wait = self.min_interval - elapsed;
            drop(last); // 关键:释放锁再 sleep,允许别的 task 排队
            tokio::time::sleep(wait).await;
        }
    }
}

/// 判断这个文件名是否会触发**云端 OCR / 文档解析提交**(走 MinerU API)。
///
/// PDF / 图片 / office 文档(doc/rtf/odt/ppt/xls,2026-06-16 起统一走 MinerU 云端解析)
/// **且** cloud_enabled 时才占 MinerU 配额。docx / txt / md / html 走原生解析 / 直接读,
/// 不消耗 MinerU 配额,不必节流。
///
/// 注意:PDF 可能 pdf-inspector 直抽成功无需 OCR fallback,这种情况节流是"误打"——
/// 多 sleep 1.4s 而已,可接受(简化判断,避免在调度层重复 PDF 文本探测)。
fn might_hit_mineru(filename: &str) -> bool {
    let f = filename.to_lowercase();
    f.ends_with(".pdf")
        || super::extractor::is_ocr_image_ext(&f)
        || super::extractor::is_office_cloud_ext(&f)
}

/// 进度事件 payload,emit 给前端的 "extraction_progress" 事件。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum ProgressEvent {
    /// 整批开始(2026-05-23 加 backend 字段,前端显示用什么后端)
    Started {
        case_id: String,
        total: usize,
        ocr_provider: String, // "local" | "cloud"
        llm_provider: String,
        llm_model: String, // 用具体模型名,前端展示更细
    },
    /// 单个文档开始处理
    DocStarted {
        case_id: String,
        doc_id: String,
        filename: String,
        index: usize,
        total: usize,
        ocr_provider: String,
        llm_provider: String,
    },
    /// 2026-06-14:单个文档**云端 OCR 轮询中**的实时状态(治大图扫描件"看着卡死")。
    /// 不进 DocStarted/DocFinished 那条主进度线,前端作为附加子状态显示(不动百分比),
    /// 每 ~3 秒来一拍。`phase`:queued(排队)/ processing(识别中)/ converting(转换中)。
    DocOcrStatus {
        case_id: String,
        doc_id: String,
        filename: String,
        index: usize,
        total: usize,
        phase: String,
        elapsed_secs: u64,
        pages_done: Option<i64>,
        pages_total: Option<i64>,
    },
    /// 单个文档处理完成(成功/跳过/失败任意一种)。
    ///
    /// 2026-05-24 i:`index` 是 doc 在原列表里的固定序号(并发顺序不保证);
    /// `completed_count` 是**单调递增的完成计数**(用 AtomicUsize 算),
    /// 前端进度条 percent 应该用 `completed_count / total` 而不是 `index / total`,
    /// 否则并发完成顺序乱会让进度回退。
    DocFinished {
        case_id: String,
        doc_id: String,
        filename: String,
        index: usize,
        total: usize,
        completed_count: usize,
        outcome: DocOutcome,
    },
    /// 2026-06-11:逐文档 OCR 完成后、全案 LLM 分析开始(这步几十秒到几分钟,
    /// 没这事件前端浮层会停在"已完成 N/N 100%"转圈,被用户当成卡死)
    Analyzing { case_id: String },
    /// 整批完成
    Completed {
        case_id: String,
        total: usize,
        extracted: usize,
        skipped: usize,
        failed: usize,
        elapsed_ms: u128,
        /// 2026-06-11:全案 LLM 分析是否成功(失败时 agg_*/详情页不会更新,
        /// 此前静默吞掉、浮层照样显示"全部完成",用户以为成功)
        analysis_ok: bool,
        analysis_error: Option<String>,
        /// 基于不完整语料、字段逻辑冲突或下游附表写入失败等非致命问题。
        analysis_warning: Option<String>,
    },
    /// 本机服务 / 云端 token 没就绪,整批没法开跑 — 2026-05-23 加
    Error { case_id: String, error: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocOutcome {
    Extracted,
    Skipped { reason: String },
    Failed { error: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureDisposition {
    /// 网络抖动、限流、服务端错误或偶发格式错误:允许进入下一轮降并发重试。
    Retryable,
    /// 当前材料/请求确定性失败(如 400 上下文超限):本材料立即失败,不浪费后两轮。
    Terminal,
    /// 账号/鉴权/模型地址属于整批共享配置错误:当前材料失败并让本批其余材料快速收口。
    AbortBatch,
}

#[derive(Debug, Clone)]
struct ProcessOutcome {
    outcome: DocOutcome,
    disposition: FailureDisposition,
}

impl ProcessOutcome {
    fn finished(outcome: DocOutcome) -> Self {
        Self {
            outcome,
            disposition: FailureDisposition::Terminal,
        }
    }

    fn failed(error: String, disposition: FailureDisposition) -> Self {
        Self {
            outcome: DocOutcome::Failed { error },
            disposition,
        }
    }

    fn is_retryable(&self) -> bool {
        self.disposition == FailureDisposition::Retryable
    }
}

fn classify_failure(error: &str) -> FailureDisposition {
    let normalized = error.to_ascii_lowercase();
    // 这些状态来自同一批共享的 LLM key / endpoint / model。继续对几十份材料逐一请求
    // 不可能自愈,只会制造「抽取中半小时」的假卡死。
    if [401, 402, 403, 404].iter().any(|code| {
        normalized.contains(&format!("llm 返回错误状态 {code}:"))
            || normalized.contains(&format!("llm returned status {code}"))
    }) || normalized.contains("insufficient balance")
    {
        return FailureDisposition::AbortBatch;
    }
    // 其它 4xx 是确定性请求错误(上下文过长、参数/schema 不兼容等),同一材料降并发也无效。
    if [400, 405, 409, 410, 422].iter().any(|code| {
        normalized.contains(&format!("llm 返回错误状态 {code}:"))
            || normalized.contains(&format!("llm returned status {code}"))
    }) {
        return FailureDisposition::Terminal;
    }
    FailureDisposition::Retryable
}

fn batch_abort_error(first_error: &str) -> String {
    format!(
        "批量抽取已停止：检测到会影响整批材料的共享故障。请先按首个错误检查磁盘、数据库、余额、API Key、模型或接口，再点「重试失败材料」。首个错误:{}",
        first_error
    )
}

fn persistence_failure(action: &str, error: impl std::fmt::Display) -> ProcessOutcome {
    ProcessOutcome::failed(
        format!("保存抽取结果失败（{}）：{}", action, error),
        FailureDisposition::AbortBatch,
    )
}

/// 在 tokio task 里跑批处理。调用立即返回,前端通过事件订阅进度。
///
/// `app`: AppHandle 用于 emit 事件;`pool`: sqlx 连接池;`case_id`: 案件 ID;
/// `documents`: 该案件下所有要处理的文档(调用方先 list_documents_by_case)。
/// `run_analysis`:OCR 抽完后是否自动跑全案 LLM 分析(run_global_extract,烧 DeepSeek)。
/// 导入 / 刷新源文件 = true(一批文档完一次性分析);**单文档重识别 = false**
/// (否则连续重识别 N 个失败文档 = 触发 N 次全案分析,白白烧钱 —— 胡彬律师反馈)。
/// run_analysis=false 时,用户识别完一批后手动点「重新分析」一次即可。
pub fn spawn_extraction(
    app: AppHandle,
    pool: SqlitePool,
    case_id: String,
    documents: Vec<Document>,
    run_analysis: bool,
) {
    let registration = ExtractionRegistration::new(&case_id);
    tauri::async_runtime::spawn(async move {
        if let Err(e) = run_extraction(
            &app,
            &pool,
            &case_id,
            &documents,
            run_analysis,
            &registration.signal,
        )
        .await
        {
            crate::dlog!("[pipeline] case {} 抽取 fatal error: {}", case_id, e);
        }
    });
}

/// 批量后台抽取:**多个案件按顺序排队**跑(case A 全抽完 → 再 case B),而不是每案各起一个并发 pipeline。
///
/// 2026-06-16(反馈 ea761d3d 大量 OCR 429):旧的多案件导入对每个案件各调一次 `spawn_extraction`,
/// 每个内部又 `buffer_unordered(8)` → 导入 N 案 = N 个 pipeline 并发 × 各 8 文档 = 最多 **N×8 并发 OCR**,
/// 直接打爆 MinerU 免费限流(50 files/min、300 requests/min)。
/// 这里改成**单个后台任务里逐案 `await run_extraction`**:同一时刻只有一个案件在抽(案内仍 ≤8 并发,
/// 约 32 文档/min,在 MinerU 50/min 之下)。配合「导入上限 3 案」+ 建议用额度更高的 PaddleOCR,基本规避限流。
/// 每个案件的进度仍走各自 `extraction_progress`(case_id 标记),前端按案件订阅不受影响。
pub fn spawn_extraction_batch(
    app: AppHandle,
    pool: SqlitePool,
    jobs: Vec<(String, Vec<Document>)>,
    run_analysis: bool,
) {
    // 入队前同步注册全部案件，确保删除一个尚未轮到的案件也能取消其排队任务。
    let jobs: Vec<_> = jobs
        .into_iter()
        .map(|(case_id, documents)| {
            let registration = ExtractionRegistration::new(&case_id);
            (case_id, documents, registration)
        })
        .collect();
    tauri::async_runtime::spawn(async move {
        for (case_id, documents, registration) in jobs {
            if let Err(e) = run_extraction(
                &app,
                &pool,
                &case_id,
                &documents,
                run_analysis,
                &registration.signal,
            )
            .await
            {
                crate::dlog!("[pipeline] case {} 批量抽取 fatal error: {}", case_id, e);
            }
        }
    });
}

/// 强制重抽单个文档的共享入口:重置 `extraction_status='pending'` + 清 `last_error`
/// → 取回文档 → `spawn_extraction` 后台异步抽取(走现有 `extraction_progress` 事件通道,
/// 前端订阅看进度 + 完成自动刷新)。返回被重抽文档的 `filename`(给调用方做提示)。
///
/// 由两个调用方复用,防逻辑漂移:① 源文件列表「重新抽取」按钮的 `reextract_document` 命令;
/// ② 案件 AI 助手的 `reextract_document` chat 工具。
/// ⚠️ 会重跑 OCR/LLM(PDF 走云端 OCR 会再烧 MinerU 积分,须用户主动选择)。
/// `ocr_backend_override`:`Some("ppocrv6")` = 去水印重识别(强制 PP-OCRv6+去水印);
/// `None` = 普通重识别,**清除**该文档之前可能设过的覆盖(回到常规 OCR 策略)。
pub async fn trigger_reextract(
    app: AppHandle,
    pool: &SqlitePool,
    doc_id: &str,
    ocr_backend_override: Option<&str>,
) -> Result<String, String> {
    crate::db::documents::reset_for_reextract(pool, doc_id)
        .await
        .map_err(|e| e.to_string())?;
    // 写/清文档级 OCR 覆盖(必须在 get_document_by_id 之前,这样取回的 doc 带上覆盖,
    // 随 spawn_extraction → process_one_doc 生效)。
    crate::db::documents::set_ocr_backend_override(pool, doc_id, ocr_backend_override)
        .await
        .map_err(|e| e.to_string())?;
    let doc = crate::db::documents::get_document_by_id(pool, doc_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "文档不存在或已删除".to_string())?;
    let case_id = doc.case_id.clone();
    let filename = doc.filename.clone();
    if let Err(e) =
        crate::db::ai_jobs::mark_case_analysis_stale(pool, &case_id, "document_reextract_requested")
            .await
    {
        crate::dlog!("[pipeline] 标记案件分析过期失败: {}", e);
    }
    // run_analysis=false:重识别单文档不自动跑全案分析(省钱),用户识别完一批后手动点「重新分析」。
    spawn_extraction(app, pool.clone(), case_id, vec![doc], false);
    Ok(filename)
}

async fn run_extraction(
    app: &AppHandle,
    pool: &SqlitePool,
    case_id: &str,
    documents: &[Document],
    run_analysis: bool,
    cancel: &Arc<ExtractionCancel>,
) -> Result<(), String> {
    let _pipeline_permit = tokio::select! {
        permit = EXTRACTION_PIPELINE_SLOT.acquire() => {
            permit.expect("extraction pipeline semaphore must stay open")
        }
        _ = cancel.cancelled() => return finish_cancelled(pool, case_id).await,
    };
    if cancel.is_cancelled() {
        return finish_cancelled(pool, case_id).await;
    }

    // 等待全局槽位期间，前一个任务可能已处理了同一批文档。重新读当前状态，并只保留调用方
    // 原本提交的 doc id，避免用旧快照重复 OCR；单文档重抽也不会误带本案其它 pending 材料。
    let scoped_ids: std::collections::HashSet<&str> = documents
        .iter()
        .map(|document| document.id.as_str())
        .collect();
    let current_documents = match crate::db::documents::list_documents_by_case(pool, case_id).await
    {
        Ok(documents) => documents,
        Err(error) => {
            let _ = app.emit(
                "extraction_progress",
                ProgressEvent::Error {
                    case_id: case_id.to_string(),
                    error: format!("读取待处理材料失败: {}", error),
                },
            );
            return Ok(());
        }
    };
    let scoped_documents: Vec<Document> = current_documents
        .into_iter()
        .filter(|document| scoped_ids.contains(document.id.as_str()))
        .collect();

    // 2026-05-23 晚十:重扫不重抽 — 只处理 pending 状态的文档(done / skipped / failed 跳过)
    // 如果用户想强制重抽某文档,可以手工 UPDATE extraction_status='pending'(V0.X 加按钮)
    // 2026-05-24 g · 并发改造:收成 owned Vec<Document>,这样 buffer_unordered 的 closure 可以 move
    let pending: Vec<Document> = scoped_documents
        .iter()
        .filter(|d| d.extraction_status == "pending" && d.deleted_at.is_none())
        .cloned()
        .collect();

    let total = pending.len();
    let total_scanned = scoped_documents.len();
    crate::dlog!(
        "[pipeline] case={} 扫到 {} 份,本次需抽 {} 份(其余已 done / skipped / failed)",
        case_id,
        total_scanned,
        total
    );

    let start = std::time::Instant::now();

    // 2026-05-23 晚六:OCR 和 LLM 独立维度 — 先读 settings 再 emit Started(便于前端显示后端)
    let user_settings = settings::read_settings().unwrap_or_default();
    let llm_config = llm::LlmConfig::from_settings(&user_settings);
    let cloud_ocr = user_settings.effective_ocr_provider() == "cloud";
    let ocr_ctx = OcrContext {
        cloud_enabled: cloud_ocr,
        // 云端模式带两家 token(2026-06-12 主/备动态切换,见 ocr.rs)。本地模式 None。
        mineru_token: cloud_ocr
            .then(|| user_settings.mineru_api_key.clone())
            .flatten(),
        paddle_vl_token: cloud_ocr
            .then(|| user_settings.paddle_vl_api_key.clone())
            .flatten(),
        cloud_primary: user_settings.effective_ocr_cloud_primary().to_string(),
        // 默认无强制后端;去水印重识别时由 process_one_doc 从 doc.ocr_backend_override 逐文档注入。
        force_backend: None,
        // 轮询进度通道由 process_one_doc 逐文档注入(带 doc 上下文);批级模板这里留空。
        poll_tx: None,
    };
    let ocr_provider = user_settings.effective_ocr_provider().to_string();
    let llm_provider = user_settings.effective_llm_provider().to_string();

    let _ = app.emit(
        "extraction_progress",
        ProgressEvent::Started {
            case_id: case_id.to_string(),
            total,
            ocr_provider: ocr_provider.clone(),
            llm_provider: llm_provider.clone(),
            llm_model: llm_config.model.clone(),
        },
    );

    crate::dlog!(
        "[pipeline] OCR={}, LLM={} (endpoint={}, key={})",
        user_settings.effective_ocr_provider(),
        user_settings.effective_llm_provider(),
        llm_config.endpoint,
        if llm_config.api_key.is_some() {
            "set"
        } else {
            "—"
        },
    );

    // 2026-05-23 晚六:如果任一 provider 是本机,后台自动起 llama-server
    if user_settings.needs_local_server() {
        if let Err(e) =
            crate::lifecycle::ensure_local_ready(user_settings.local_model_dir.as_deref()).await
        {
            crate::dlog!("[pipeline] 本机服务启动失败: {}", e);
            let _ = app.emit(
                "extraction_progress",
                ProgressEvent::Error {
                    case_id: case_id.to_string(),
                    error: format!("本机模型未就绪: {}", e),
                },
            );
            return Ok(());
        }
    }

    // 2026-05-25 V0.1.8 三轮动态降级(替代原来固定 8 路):
    //   第 1 轮 buffer_unordered(8) — 全部 pending
    //   第 2 轮 buffer_unordered(4) — 第 1 轮失败的
    //   第 3 轮 buffer_unordered(1) — 第 2 轮还失败的;只有第 3 轮失败才标 failed 落 last_error
    //
    // 进度计数:每个 doc 在它**最终**完成时(成功 / skipped / 第 3 轮失败)才递增 completed_count
    // 并 emit DocFinished。中间轮失败静默(避免前端进度回弹)。
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let completed = Arc::new(AtomicUsize::new(0));
    // 共享账号/配置错误(401/402/403/404)一旦出现,后续尚未真正开始的文档直接失败收口,
    // 避免 N 份材料 × 3 轮继续撞同一个确定性错误。
    let batch_abort = Arc::new(std::sync::Mutex::new(None::<String>));

    // doc.id → 该 doc 的最终 outcome(每轮覆盖)
    let mut final_outcomes: HashMap<String, DocOutcome> = HashMap::with_capacity(total);
    // 待重试队列(每轮过滤失败的)
    let mut queue: Vec<Document> = pending;

    for (round_idx, &concurrency) in ROUND_CONCURRENCY.iter().enumerate() {
        if cancel.is_cancelled() {
            return finish_cancelled(pool, case_id).await;
        }
        if queue.is_empty() {
            break;
        }
        let round_num = round_idx + 1; // 1-indexed,日志友好
        let is_final = round_num == ROUND_CONCURRENCY.len();
        crate::dlog!(
            "[pipeline] case={} 第 {}/{} 轮: {} 路并发跑 {} 份",
            case_id,
            round_num,
            ROUND_CONCURRENCY.len(),
            concurrency,
            queue.len(),
        );

        let round_results: Vec<(Document, Option<ProcessOutcome>)> =
            stream::iter(queue.into_iter().enumerate())
                .map(|(index, doc)| {
                    let app = app.clone();
                    let pool = pool.clone();
                    let case_id_owned = case_id.to_string();
                    let llm_config = llm_config.clone();
                    let ocr_ctx = ocr_ctx.clone();
                    let ocr_provider = ocr_provider.clone();
                    let llm_provider = llm_provider.clone();
                    let completed = Arc::clone(&completed);
                    let throttle = global_submit_throttle();
                    let batch_abort = Arc::clone(&batch_abort);
                    let cancel = Arc::clone(cancel);
                    let filename = doc.filename.clone();
                    let doc_id = doc.id.clone();
                    async move {
                        let abort_error = batch_abort
                            .lock()
                            .ok()
                            .and_then(|guard| guard.clone());
                        let processed = if cancel.is_cancelled() {
                            None
                        } else if let Some(error) = abort_error {
                            let persist_result = sqlx::query(
                                "UPDATE documents SET extraction_status = 'failed', last_error = ? WHERE id = ?",
                            )
                            .bind(&error)
                            .bind(&doc.id)
                            .execute(&pool)
                            .await;
                            match persist_result {
                                Ok(_) => Some(ProcessOutcome::failed(error, FailureDisposition::Terminal)),
                                Err(persist_error) => {
                                    Some(persistence_failure("批量故障状态", persist_error))
                                }
                            }
                        } else {
                            tokio::select! {
                                _ = cancel.cancelled() => None,
                                result = process_one_doc(
                                    &app,
                                    &pool,
                                    &case_id_owned,
                                    &llm_config,
                                    &ocr_ctx,
                                    &ocr_provider,
                                    &llm_provider,
                                    index + 1,
                                    total,
                                    doc.clone(),
                                    round_num,
                                    is_final,
                                    throttle,
                                ) => Some(result),
                            }
                        };

                        if processed.as_ref().is_some_and(|result| result.disposition == FailureDisposition::AbortBatch) {
                            let processed = processed.as_ref().expect("checked Some");
                            if let DocOutcome::Failed { error } = &processed.outcome {
                                // 持久化故障本身可能已恢复（如瞬时 SQLite busy）；再尽力把当前文档
                                // 从 processing 收口为 failed，避免只能等下次启动恢复。
                                let _ = sqlx::query(
                                    "UPDATE documents SET extraction_status = 'failed', last_error = ? \
                                     WHERE id = ? AND extraction_status = 'processing'",
                                )
                                .bind(error)
                                .bind(&doc.id)
                                .execute(&pool)
                                .await;
                                if let Ok(mut guard) = batch_abort.lock() {
                                    if guard.is_none() {
                                        *guard = Some(batch_abort_error(error));
                                    }
                                }
                            }
                        }

                        // 只有最终结果(成功 / skipped / 第 3 轮失败)才递增 completed_count + emit DocFinished
                        let is_terminal = processed.as_ref().is_some_and(|result| {
                            !matches!(result.outcome, DocOutcome::Failed { .. })
                                || is_final
                                || !result.is_retryable()
                        });
                        if is_terminal {
                            let processed = processed.as_ref().expect("terminal result must exist");
                            let done_so_far = completed.fetch_add(1, Ordering::SeqCst) + 1;
                            let _ = app.emit(
                                "extraction_progress",
                                ProgressEvent::DocFinished {
                                    case_id: case_id_owned,
                                    doc_id,
                                    filename,
                                    index: index + 1,
                                    total,
                                    completed_count: done_so_far,
                                    outcome: processed.outcome.clone(),
                                },
                            );
                        }
                        (doc, processed)
                    }
                })
                .buffer_unordered(concurrency)
                .collect()
                .await;

        // 收集本轮结果:成功/skipped/最终失败 → 写入 final_outcomes;
        // 非最终轮的失败 → 进下一轮 queue
        let mut next_queue: Vec<Document> = Vec::new();
        for (doc, processed) in round_results {
            let Some(processed) = processed else {
                return finish_cancelled(pool, case_id).await;
            };
            match &processed.outcome {
                DocOutcome::Failed { .. } if !is_final && processed.is_retryable() => {
                    next_queue.push(doc);
                }
                _ => {
                    final_outcomes.insert(doc.id.clone(), processed.outcome);
                }
            }
        }
        queue = next_queue;

        // 还有下一轮 → 给 MinerU 限流计数器缓口气
        if !queue.is_empty() && !is_final {
            tokio::time::sleep(Duration::from_secs(INTER_ROUND_SLEEP_SEC)).await;
        }
    }

    let mut extracted = 0;
    let mut skipped = 0;
    let mut failed = 0;
    for outcome in final_outcomes.values() {
        match outcome {
            DocOutcome::Extracted => extracted += 1,
            DocOutcome::Skipped { .. } => skipped += 1,
            DocOutcome::Failed { .. } => failed += 1,
        }
    }

    // 2026-05-24 h · 新架构:所有文档 OCR 完成后,**让 LLM 全局抽**(替代旧 aggregator 规则)。
    // 拼所有 MD → DeepSeek 1M 上下文 → 两次并发 LLM call(填表 + 案件分析报告)
    // 2026-06-13(胡彬律师反馈):run_analysis=false(单文档重识别)时跳过,不烧 DeepSeek;
    //   用户识别完一批后手动点「重新分析」一次即可。
    let aborted_error = batch_abort.lock().ok().and_then(|guard| guard.clone());
    // 等待全局队列期间，同一批材料可能已经被前一个任务处理并完成分析。此时 total=0 且
    // analysis_stale=0，直接收口，避免重复烧一次全案 LLM；源文件删除等真正变化会先标 stale。
    let analysis_stale = if total == 0 && run_analysis {
        sqlx::query_scalar::<_, i64>("SELECT analysis_stale FROM cases WHERE id = ?")
            .bind(case_id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .unwrap_or(1)
            != 0
    } else {
        true
    };
    let (analysis_ok, analysis_error, analysis_warning) = if let Some(error) = aborted_error {
        crate::dlog!(
            "[global_extract] case={} 跳过:批量抽取已因共享配置错误停止",
            case_id
        );
        (false, Some(error), None)
    } else if run_analysis && (total > 0 || analysis_stale) {
        // 2026-06-11:这步耗时几十秒~几分钟,先 emit Analyzing 让前端浮层显示"通读全案分析中"
        let _ = app.emit(
            "extraction_progress",
            ProgressEvent::Analyzing {
                case_id: case_id.to_string(),
            },
        );
        let report = tokio::select! {
            _ = cancel.cancelled() => return finish_cancelled(pool, case_id).await,
            report = crate::ingest::global_pipeline::run_global_extract(pool, case_id, &llm_config) => report,
        };
        crate::dlog!(
            "[global_extract] case={} docs={} table_ok={} report_ok={} elapsed={}ms{}",
            case_id,
            report.docs_included,
            report.table_ok,
            report.report_ok,
            report.elapsed_ms,
            report
                .error
                .as_ref()
                .map(|e| format!(" err={}", e))
                .unwrap_or_default(),
        );
        let analysis_ok = report.table_ok && report.report_ok;
        (
            analysis_ok,
            report.error.clone().or_else(|| {
                (!report.report_ok).then_some("案件画像已更新，但分析报告未能保存".to_string())
            }),
            report.warning.clone(),
        )
    } else {
        // 单文档重识别或重复排队任务：不是失败，标 ok，前端 banner 不报警。
        crate::dlog!(
            "[global_extract] case={} 跳过自动分析(run_analysis={}, total={}, stale={})",
            case_id,
            run_analysis,
            total,
            analysis_stale,
        );
        (true, None, None)
    };

    let _ = app.emit(
        "extraction_progress",
        ProgressEvent::Completed {
            case_id: case_id.to_string(),
            total,
            extracted,
            skipped,
            failed,
            elapsed_ms: start.elapsed().as_millis(),
            analysis_ok,
            analysis_error,
            analysis_warning,
        },
    );

    Ok(())
}

async fn finish_cancelled(pool: &SqlitePool, case_id: &str) -> Result<(), String> {
    // 若删除 DB 失败，避免残留永久 processing；若删除已成功，本 UPDATE 自然影响 0 行。
    sqlx::query(
        "UPDATE documents SET extraction_status = 'pending' \
         WHERE case_id = ? AND extraction_status = 'processing'",
    )
    .bind(case_id)
    .execute(pool)
    .await
    .map_err(|error| format!("取消抽取后恢复材料状态失败: {}", error))?;
    crate::dlog!("[pipeline] case={} 抽取任务已取消", case_id);
    Ok(())
}

/// 单个 doc 的完整处理(emit 进度 + 调 extractor + 写 DB)。
///
/// 设计成 owned-by-task:所有参数 borrow,但传给它的实际值都是 task 自己 clone 的副本,
/// 这样 buffer_unordered 的多个 task 可以独立运行不互相阻塞。
///
/// 2026-05-25 V0.1.8 加 `round_num` / `is_final_round`:
///   - DocStarted 事件带轮次(前端可显示"重试中 N/3")
///   - 失败时只有 is_final_round=true 才写 status='failed' + last_error;
///     中间轮失败回退到 status='pending'(下一轮会重新 UPDATE 成 processing)
#[allow(clippy::too_many_arguments)]
async fn process_one_doc(
    app: &AppHandle,
    pool: &SqlitePool,
    case_id: &str,
    llm_config: &llm::LlmConfig,
    ocr_ctx: &OcrContext,
    ocr_provider: &str,
    llm_provider: &str,
    index: usize,
    total: usize,
    doc: Document,
    round_num: usize,
    is_final_round: bool,
    throttle: &SubmitThrottle,
) -> ProcessOutcome {
    // 文件名加轮次后缀(第 2/3 轮),前端能感知"在重试"
    let display_name = if round_num > 1 {
        format!(
            "{} (重试 {}/{})",
            doc.filename,
            round_num,
            ROUND_CONCURRENCY.len()
        )
    } else {
        doc.filename.clone()
    };
    let _ = app.emit(
        "extraction_progress",
        ProgressEvent::DocStarted {
            case_id: case_id.to_string(),
            doc_id: doc.id.clone(),
            filename: display_name.clone(),
            index,
            total,
            ocr_provider: ocr_provider.to_string(),
            llm_provider: llm_provider.to_string(),
        },
    );

    if let Err(error) =
        sqlx::query("UPDATE documents SET extraction_status = 'processing' WHERE id = ?")
            .bind(&doc.id)
            .execute(pool)
            .await
    {
        return persistence_failure("标记处理中", error);
    }

    // 第一轮正文提取成功但 LLM 失败时已落盘；后续轮次只重试 LLM，避免重复 OCR 和额度消耗。
    let cached_retry_text = (round_num > 1)
        .then(|| read_retry_text(case_id, &doc.id))
        .flatten();

    // 2026-06-14:云端 OCR 单文档轮询进度 → 前端"排队 / 识别中(已 N 秒)"子状态。
    // 建一个单文档级回传通道 + 转发任务:OCR 轮询循环每拍 send 一次 OcrPollUpdate,
    // 转发任务补上 doc 上下文后 emit DocOcrStatus(**不动主进度条百分比**,前端单独渲染)。
    // tx 随 doc_ocr_ctx 在本函数末尾 drop,转发任务届时自然结束(rx 收到 None)。
    // 仅在可能走云端 OCR(cloud_enabled 或去水印强制后端)时才建,本地/跳过文档不浪费。
    let mut doc_ocr_ctx = ocr_ctx.clone();
    if cached_retry_text.is_none() && (ocr_ctx.cloud_enabled || doc.ocr_backend_override.is_some())
    {
        let (tx, mut rx) =
            tokio::sync::mpsc::unbounded_channel::<crate::ingest::ocr::OcrPollUpdate>();
        doc_ocr_ctx.poll_tx = Some(tx);
        let app_fwd = app.clone();
        let case_id_fwd = case_id.to_string();
        let doc_id_fwd = doc.id.clone();
        let name_fwd = display_name.clone();
        tokio::spawn(async move {
            while let Some(u) = rx.recv().await {
                let _ = app_fwd.emit(
                    "extraction_progress",
                    ProgressEvent::DocOcrStatus {
                        case_id: case_id_fwd.clone(),
                        doc_id: doc_id_fwd.clone(),
                        filename: name_fwd.clone(),
                        index,
                        total,
                        phase: u.phase,
                        elapsed_secs: u.elapsed_secs,
                        pages_done: u.pages_done,
                        pages_total: u.pages_total,
                    },
                );
            }
        });
    }

    // 2026-05-31 抽取策略改版(作者:现在所有材料都要抽,做案件分析/对抗需要证据支撑)。
    // 三档:
    //   A. 完整抽(字段 + 文本,进 LLM 上下文):法院文书 + 我方文书 + **证据材料**
    //      (合同/催告函/对话记录等 —— 作者明确要进对抗分析)
    //   B. 仅文本归档(抽文本存着,但**不进** LLM 上下文):律所规范/程序材料
    //      (风险告知书/谈话笔录/反馈卡/送达确认书 等)+ 身份信息(隐私,无分析价值)。
    //      上下文排除在 constitution.rs 用 is_archival_category 把关;这里只负责抽文本归档。
    //   C. 纯跳过:AI 产物(已是结构化 .md,再抽回上下文会自证循环)。
    let result = if let Some(backend) = doc.ocr_backend_override.clone() {
        // 2026-06-13:用户对该文档点了「去水印重识别」→ 强制走该 OCR 后端(PP-OCRv6+去水印),
        // 绕过归档短路与文本层、不回退;让带水印的工商调档件也能完整抽。
        if cached_retry_text.is_none() && ocr_ctx.cloud_enabled && might_hit_mineru(&doc.filename) {
            throttle.acquire().await;
        }
        doc_ocr_ctx.force_backend = Some(backend);
        extract_one(
            llm_config,
            &doc_ocr_ctx,
            Path::new(&doc.source_path),
            &doc.filename,
            doc.category.as_deref(),
            cached_retry_text.clone(),
        )
        .await
    } else if doc.is_ai_artifact {
        // C. AI 产物纯跳过
        ExtractResult::Skipped {
            reason: "AI 产物已是结构化总结,跳过(详情页直接渲染原文)".to_string(),
            metrics: Vec::new(),
        }
    } else if is_archival_category(doc.category.as_deref())
        || doc.stage.as_deref() == Some("身份信息")
    {
        // B. 律所规范/程序/身份材料:只抽文本归档(便宜直抽,扫描件不烧 OCR),不抽 LLM 字段。
        // 文本仍写盘 → read_case_doc / 全文搜索可读;但 constitution 不把它塞进 system prompt。
        match crate::ingest::extractor::extract_text_only_cheap(
            Path::new(&doc.source_path),
            &doc.filename,
        )
        .await
        {
            Ok(Some(text_md)) => ExtractResult::TextOnly {
                text_md,
                metrics: Vec::new(),
            },
            // 直抽拿不到(扫描件/图片需 OCR)或真错 → 纯跳过(无文本),归档类不值得烧 OCR
            _ => ExtractResult::Skipped {
                reason: "律所规范/程序/身份材料(归档,不进 AI 上下文)".to_string(),
                metrics: Vec::new(),
            },
        }
    } else {
        // A. 其余全部完整抽(含证据 stage / 合同 / 催告函等)。证据现在要支撑对抗分析,
        //    走完整 extract_one(字段 + 文本)。PDF/扫描件会触发云端 OCR(作者主动选择:
        //    分析价值 > OCR 成本;扫描件直抽失败才 OCR 的既有链路保留,不浪费)。
        // 节流闸门 —— 仅云端 OCR + PDF/图片才需要(避开 MinerU 50 文件/分钟限流)
        if cached_retry_text.is_none() && ocr_ctx.cloud_enabled && might_hit_mineru(&doc.filename) {
            throttle.acquire().await;
        }
        extract_one(
            llm_config,
            &doc_ocr_ctx,
            Path::new(&doc.source_path),
            &doc.filename,
            doc.category.as_deref(),
            cached_retry_text,
        )
        .await
    };

    // 2026-05-26 V0.1.12:抽取性能埋点 — 拿到 metrics 后批量 insert 进表,反馈通道带出来
    let collected_metrics: Vec<crate::db::metrics::MetricEntry> = match &result {
        ExtractResult::Extracted { metrics, .. } => metrics.clone(),
        ExtractResult::PartialExtracted { metrics, .. } => metrics.clone(),
        ExtractResult::Skipped { metrics, .. } => metrics.clone(),
        ExtractResult::TextOnly { metrics, .. } => metrics.clone(),
        ExtractResult::Failed { metrics, .. } => metrics.clone(),
    };
    if !collected_metrics.is_empty() {
        if let Err(e) = crate::db::metrics::insert_many(pool, &collected_metrics).await {
            crate::dlog!("[pipeline] 写 extraction_metrics 失败(不阻塞抽取): {}", e);
        }
    }

    let outcome = match result {
        ExtractResult::Extracted {
            fields,
            text_md,
            metrics: _,
        } => {
            let json = match serde_json::to_string(&fields) {
                Ok(json) => json,
                Err(error) => return persistence_failure("序列化字段", error),
            };
            let extracted_text_path = match write_extracted_md(case_id, &doc.id, &text_md) {
                Ok(p) => p,
                Err(e) => {
                    crate::dlog!("[pipeline] 写 extracts/.md 失败: {}", e);
                    return persistence_failure("写入抽取正文", e);
                }
            };
            let extracted_text_hash = crate::db::documents::stable_text_hash(&text_md);
            // 成功 → 清掉 last_error(可能是上次失败留下的)
            if let Err(error) = sqlx::query(
                "UPDATE documents SET extracted_fields = ?, extracted_text_path = ?, extracted_text_hash = ?, \
                 extraction_status = 'done', last_error = NULL WHERE id = ?",
            )
            .bind(&json)
            .bind(&extracted_text_path)
            .bind(&extracted_text_hash)
            .bind(&doc.id)
            .execute(pool)
            .await
            {
                return persistence_failure("写入字段状态", error);
            }
            ProcessOutcome::finished(DocOutcome::Extracted)
        }
        ExtractResult::PartialExtracted {
            fields,
            text_md,
            warning,
            metrics: _,
        } => {
            let disposition = classify_failure(&warning);
            if is_final_round || disposition != FailureDisposition::Retryable {
                let json = match serde_json::to_string(&fields) {
                    Ok(json) => json,
                    Err(error) => return persistence_failure("序列化部分字段", error),
                };
                let extracted_text_path = match write_extracted_md(case_id, &doc.id, &text_md) {
                    Ok(p) => p,
                    Err(e) => {
                        crate::dlog!("[pipeline] PartialExtracted 写 extracts/.md 失败: {}", e);
                        return persistence_failure("写入部分抽取正文", e);
                    }
                };
                let extracted_text_hash = crate::db::documents::stable_text_hash(&text_md);
                if let Err(error) = sqlx::query(
                    "UPDATE documents SET extracted_fields = ?, extracted_text_path = ?, extracted_text_hash = ?, \
                     extraction_status = 'failed', last_error = ? WHERE id = ?",
                )
                .bind(&json)
                .bind(&extracted_text_path)
                .bind(&extracted_text_hash)
                .bind(&warning)
                .bind(&doc.id)
                .execute(pool)
                .await
                {
                    return persistence_failure("写入部分抽取状态", error);
                }
            } else {
                if let Err(error) = persist_retry_text(pool, case_id, &doc.id, &text_md).await {
                    return persistence_failure("缓存部分抽取正文", error);
                }
                if let Err(error) =
                    sqlx::query("UPDATE documents SET extraction_status = 'pending' WHERE id = ?")
                        .bind(&doc.id)
                        .execute(pool)
                        .await
                {
                    return persistence_failure("重新排队部分抽取材料", error);
                }
                crate::dlog!(
                    "[pipeline] case={} doc={} 第 {} 轮部分抽取失败,排队下一轮: {}",
                    case_id,
                    doc.filename,
                    round_num,
                    warning
                );
            }
            ProcessOutcome::failed(warning, disposition)
        }
        ExtractResult::Skipped { reason, metrics: _ } => {
            // skipped 不是失败，但原因必须持久化；顶部进度条消失后用户仍能在材料行看到为什么跳过。
            if let Err(error) = sqlx::query(
                "UPDATE documents SET extraction_status = 'skipped', last_error = ? WHERE id = ?",
            )
            .bind(&reason)
            .bind(&doc.id)
            .execute(pool)
            .await
            {
                return persistence_failure("写入跳过原因", error);
            }
            ProcessOutcome::finished(DocOutcome::Skipped { reason })
        }
        // 只抽了文本、没抽字段:状态保持 'skipped'(透明 — 没跑 LLM 字段),但写
        // extracted_text_path,使 read_case_doc / find_in_document / 全文搜索可读。
        ExtractResult::TextOnly {
            text_md,
            metrics: _,
        } => {
            let reason =
                "已抽文本未抽字段（归档/身份材料不进入自动案件画像，可由 AI 助手按需读取）";
            let extracted_text_path = match write_extracted_md(case_id, &doc.id, &text_md) {
                Ok(p) => p,
                Err(e) => {
                    crate::dlog!("[pipeline] TextOnly 写 extracts/.md 失败: {}", e);
                    return persistence_failure("写入归档正文", e);
                }
            };
            let extracted_text_hash = crate::db::documents::stable_text_hash(&text_md);
            if let Err(error) = sqlx::query(
                "UPDATE documents SET extracted_text_path = ?, extracted_text_hash = ?, \
                 extraction_status = 'skipped', last_error = ? WHERE id = ?",
            )
            .bind(&extracted_text_path)
            .bind(&extracted_text_hash)
            .bind(reason)
            .bind(&doc.id)
            .execute(pool)
            .await
            {
                return persistence_failure("写入归档抽取状态", error);
            }
            ProcessOutcome::finished(DocOutcome::Skipped {
                reason: reason.to_string(),
            })
        }
        ExtractResult::Failed {
            error,
            text_md,
            metrics: _,
        } => {
            let disposition = classify_failure(&error);
            if is_final_round || disposition != FailureDisposition::Retryable {
                let mut extracted_text_hash: Option<String> = None;
                let extracted_text_path = text_md.as_deref().and_then(|text| {
                    match write_extracted_md(case_id, &doc.id, text) {
                        Ok(p) => {
                            extracted_text_hash =
                                Some(crate::db::documents::stable_text_hash(text));
                            Some(p)
                        }
                        Err(e) => {
                            crate::dlog!("[pipeline] Failed-with-text 写 extracts/.md 失败: {}", e);
                            None
                        }
                    }
                });
                // 三轮都失败 → 真的 failed,落 last_error 给用户/事后排查看
                if let Err(persist_error) = sqlx::query(
                    "UPDATE documents SET extracted_text_path = COALESCE(?, extracted_text_path), \
                     extracted_text_hash = COALESCE(?, extracted_text_hash), \
                     extraction_status = 'failed', last_error = ? WHERE id = ?",
                )
                .bind(&extracted_text_path)
                .bind(&extracted_text_hash)
                .bind(&error)
                .bind(&doc.id)
                .execute(pool)
                .await
                {
                    return persistence_failure("写入失败状态", persist_error);
                }
                crate::dlog!(
                    "[pipeline] case={} doc={} {}: {}",
                    case_id,
                    doc.filename,
                    if is_final_round {
                        "三轮全失败"
                    } else {
                        "确定性失败,不再重试"
                    },
                    error
                );
            } else {
                // 中间轮失败 → 回退 pending 状态,等下一轮 caller 再喂进来
                // (不写 last_error,因为下一轮可能成功;只在最终失败才落 error)
                if let Some(text) = text_md.as_deref() {
                    if let Err(persist_error) =
                        persist_retry_text(pool, case_id, &doc.id, text).await
                    {
                        return persistence_failure("缓存失败材料正文", persist_error);
                    }
                }
                if let Err(persist_error) =
                    sqlx::query("UPDATE documents SET extraction_status = 'pending' WHERE id = ?")
                        .bind(&doc.id)
                        .execute(pool)
                        .await
                {
                    return persistence_failure("重新排队失败材料", persist_error);
                }
                crate::dlog!(
                    "[pipeline] case={} doc={} 第 {} 轮失败,排队下一轮: {}",
                    case_id,
                    doc.filename,
                    round_num,
                    error
                );
            }
            ProcessOutcome::failed(error, disposition)
        }
    };

    // DocFinished emit 现在挪到调用方(stream wrapper),那里有 completed_count 计数器
    // 且只在 doc**最终**完成时 emit(中间轮失败静默,避免前端进度回弹)
    outcome
}

/// 把抽出的纯文本写盘到 `~/Library/Application Support/CaseBoard/extracts/<case_id>/<doc_id>.md`。
///
/// 2026-05-23 晚十 Q1 作者拍板:落盘,方便全文搜索、用户预览、未来加编辑。
fn write_extracted_md(case_id: &str, doc_id: &str, text: &str) -> Result<String, String> {
    let dir = extracts_dir_for_case(case_id)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("建目录 {} 失败: {}", dir.display(), e))?;
    let path = dir.join(format!("{}.md", doc_id));
    std::fs::write(&path, text).map_err(|e| format!("写 {} 失败: {}", path.display(), e))?;
    Ok(path.to_string_lossy().into_owned())
}

async fn persist_retry_text(
    pool: &SqlitePool,
    case_id: &str,
    doc_id: &str,
    text: &str,
) -> Result<(), String> {
    let path = write_extracted_md(case_id, doc_id, text)?;
    let hash = crate::db::documents::stable_text_hash(text);
    sqlx::query(
        "UPDATE documents SET extracted_text_path = ?, extracted_text_hash = ? WHERE id = ?",
    )
    .bind(path)
    .bind(hash)
    .bind(doc_id)
    .execute(pool)
    .await
    .map_err(|error| format!("登记重试正文失败: {}", error))?;
    Ok(())
}

fn read_retry_text(case_id: &str, doc_id: &str) -> Option<String> {
    std::fs::read_to_string(
        extracts_dir_for_case(case_id)
            .ok()?
            .join(format!("{}.md", doc_id)),
    )
    .ok()
    .filter(|text| !text.trim().is_empty())
}

/// 判断这个文档类别是不是"律所规范 / 程序 / 身份归档类" —— 抽文本归档,但**不进 LLM 上下文**。
///
/// 2026-05-31 改版(作者):现在所有材料都要抽(证据也要,做案件分析/对抗需要证据支撑)。
/// 但有几类材料对实体分析无价值、只占 token / 加噪音,应"抽了存着可查、但不喂给 LLM":
///   - **律所规范材料**:风险告知书、谈话笔录、反馈卡(作者点名的三类)
///   - **程序性材料**:送达地址确认书、送达回证、回执、介绍信、收案呈批表、收案登记表
///   - **律师内部**:办案笔记
///   - **身份隐私**:身份证、户口(隐私 + 无分析价值)
///
/// ⚠️ 关键区别(与旧 `is_low_value_category` 的根本不同):
/// **证据材料不再在此列** —— 借条/欠条/发票/收据/票据/银行流水/营业执照/证据清单/合同 等
/// 现在要走**完整抽取并进上下文**(作者:对抗分析需要证据支撑)。
/// 本函数同时被 `constitution.rs` 用来把这些类别**排除出 system prompt**(归档不喂 LLM)。
pub(crate) fn is_archival_category(cat: Option<&str>) -> bool {
    matches!(
        cat,
        // 律所规范材料(作者点名)
        Some("风险告知")
            | Some("风险告知书")
            | Some("反馈卡")
            | Some("律师工作反馈卡")
            | Some("笔录")
            | Some("谈话笔录")
            | Some("首次谈话笔录")
            // 程序性材料
            | Some("送达回证")
            | Some("送达地址确认书")
            | Some("回执")
            | Some("介绍信")
            | Some("收案呈批表")
            | Some("收案登记表")
            // 律师内部
            | Some("办案笔记")
            // 参考案例/参考判决只供手工查阅引用，不能混进本案事实
            | Some("参考材料")
            // 身份隐私
            | Some("身份证")
            | Some("户口")
    )
}

fn extracts_dir_for_case(case_id: &str) -> Result<PathBuf, String> {
    // 跟 caseboard.db / settings.json 同一个 app data dir(~/Library/Application Support/CaseBoard/)
    let base = crate::db::app_data_dir().map_err(|e| format!("无法定位 app data dir: {}", e))?;
    Ok(base.join("extracts").join(case_id))
}
