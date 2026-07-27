//! 用户反馈通道(2026-05-24 e · 作者拍板的"MD 文件方案")。
//!
//! 流程:
//!   1. 前端调 `collect_diagnostic_info()` 拿一份"无标识"的系统快照
//!      (版本、OS、provider、统计数字 — 永不含案件名/当事人/文档内容)
//!   2. 前端弹窗显示快照预览 + 用户描述输入框
//!   3. 用户确认 → 可调 `upload_to_cloud(snapshot, description)` 上传到作者私有收件箱
//!   4. 也可调 `save_feedback_to_desktop(snapshot, description)` 写到本地 MD 备用
//!
//! 隐私铁律:
//!   - 不含案件名 / 当事人 / 案号 / 文档内容
//!   - 案件数 / 文档数 是聚合数字,不带 ID
//!   - "最近错误"只列文件名后缀(.docx / .pdf)+ 错误信息,不带文件路径
//!   - 云端上传必须由用户点击确认触发,不做静默上送

use base64::Engine;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

const FEEDBACK_SUPABASE_URL: Option<&str> = option_env!("CASEBOARD_TELEMETRY_URL");
const FEEDBACK_SUPABASE_KEY: Option<&str> = option_env!("CASEBOARD_TELEMETRY_KEY");
const FEEDBACK_UPLOAD_TIMEOUT_SECS: u64 = 20;
const FEEDBACK_REPORT_MAX_CHARS: usize = 200_000;
const FEEDBACK_DESCRIPTION_PREVIEW_CHARS: usize = 180;
const FEEDBACK_SCREENSHOT_MAX_COUNT: usize = 3;
const FEEDBACK_SCREENSHOT_MAX_BYTES: usize = 5 * 1024 * 1024;
const FEEDBACK_SCREENSHOT_TOTAL_MAX_BYTES: usize = 10 * 1024 * 1024;
const FEEDBACK_SCREENSHOT_BUCKET: &str = "feedback-screenshots";

/// 自动收集的诊断信息(给反馈 MD 用)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticInfo {
    /// 匿名反馈识别码(UUID v4 前 8 位),作者可识别"同一个人重复反馈"
    pub client_id_short: String,
    /// CaseBoard App 版本(来自 Cargo.toml)
    pub app_version: String,
    /// macOS 版本 + 架构(如 "macOS 14.5 · arm64")
    pub os_version: String,
    /// 系统语言(zh-CN / en-US 等)
    pub language: String,

    /// LLM provider("local" / "cloud" / "未配置")
    pub llm_provider: String,
    /// OCR provider("local" / "cloud" / "未配置")
    pub ocr_provider: String,
    /// 本机 llama-server 状态描述
    pub local_server_status: String,
    /// 云端 DeepSeek 余额(元;未配置时 None)
    pub deepseek_balance: Option<f64>,

    /// 数据库聚合统计(数字,无 ID)
    pub stats: AppStats,

    /// 最近失败的抽取记录(最多 5 条,只含 filename + extraction_status)
    pub recent_failures: Vec<RecentFailure>,

    /// 2026-05-26 V0.1.11:Settings 关键配置(脱敏 - api_key / token 全替换成 [SET]/[EMPTY])
    pub settings_snapshot: SettingsSnapshot,

    /// 2026-05-26 V0.1.11:磁盘 / DB 大小 / 路径权限 / RAM 等系统级诊断
    pub system_info: SystemInfo,

    /// 2026-06-28:本地 KB / 记忆目录诊断。只含路径、权限、计数,不含记忆正文。
    #[serde(default)]
    pub kb_memory: KbMemoryDiagnostic,

    /// 2026-05-26 V0.1.11:App 自身最近的 stderr 日志(diagnostic_log ring buffer 快照)
    /// 最多 200 行,新→旧(每行已 sanitize_paths,不含 /Users/.../<case-name>/ 路径)
    pub stderr_tail: Vec<String>,

    /// 最近 24 小时的 Pi Runtime 结构化元数据。只含 Provider、模型、耗时、
    /// 重试、错误分类和请求规模，不含 prompt、回答、推理或工具载荷。
    #[serde(default)]
    pub runtime_traces: Vec<crate::chat::diagnostics::RuntimeTraceEvent>,

    /// 2026-05-26 V0.1.11:前端累积的 console.error / window.onerror(由前端打开弹窗时传入)
    /// 不持久化,刷新页面就丢
    #[serde(default)]
    pub console_errors: Vec<ConsoleError>,

    /// 2026-05-26 V0.1.12:最近 200 条抽取性能埋点(stage / backend / 耗时 / 字数 / 成败)
    /// 给作者拿来对比"本机 OCR vs 云端 OCR 哪个更快更准",决定后续要不要砍掉本机档全走云端
    pub metrics_tail: Vec<MetricSample>,

    /// 2026-05-26 V0.1.12:metrics 按 backend 聚合的统计(平均/中位耗时 + 成功率 + 样本数)
    /// 直接给"本机 vs 云端"的 A/B 结论
    pub metrics_summary: Vec<BackendStat>,

    /// 2026-05-27 V0.1.13+:案件 AI 助手用量统计(按 model 聚合的最近 200 条 chat)。
    /// **隐私**:绝不携带 chat_messages.content,只取 model / tokens / latency / task_type
    /// / char count。回归测试在 chat::commands::tests::chat_content_never_leaks_into_feedback_md。
    #[serde(default)]
    pub chat_usage: Vec<ChatUsageBucket>,

    /// 2026-05-27 V0.1.13+:功能模块用量统计 —— 老板用来一眼判断
    /// 「同事用没用 chat / 跑没跑 LLM 报告 / 是诉讼还是非诉为主」。
    /// 全是聚合数字,无任何业务标识。
    #[serde(default)]
    pub feature_usage: FeatureUsage,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CloudFeedbackPayload {
    pub id: String,
    pub client_id_short: String,
    pub app_version: String,
    pub os_version: String,
    pub description_preview: String,
    pub report_md: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub screenshots: Vec<CloudFeedbackScreenshot>,
    pub status: String,
}

/// 前端主动选择或粘贴的截图。字段按 Tauri IPC 的 camelCase 契约反序列化。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackScreenshotInput {
    pub filename: String,
    pub mime_type: String,
    pub data_base64: String,
}

/// 写入 feedback_reports.screenshots(jsonb) 的 Storage 对象引用,不含图片正文。
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CloudFeedbackScreenshot {
    pub filename: String,
    pub mime_type: String,
    pub storage_path: String,
    pub size_bytes: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedFeedbackScreenshot {
    pub metadata: CloudFeedbackScreenshot,
    pub bytes: Vec<u8>,
}

/// 功能模块用量统计(2026-05-27 V0.1.13+)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeatureUsage {
    /// chat_messages 总条数(含 user + assistant)
    pub chat_messages_total: i64,
    /// chat artifact 总数(documents WHERE source='chat')
    pub chat_artifacts_total: i64,
    /// 已生成「案件分析报告」(LLM 全局抽)的案件数
    pub cases_with_analysis_report: i64,
    /// 已生成元典风险报告的案件数
    pub cases_with_risk_report: i64,
    /// 已生成元典深挖报告的案件数
    pub cases_with_deep_dive_report: i64,
    /// 已生成完整执行追踪报告的案件数
    pub cases_with_full_report: i64,
    /// 有用户手改 overrides 的案件数(反映编辑模式使用度)
    pub cases_with_user_overrides: i64,
    /// 案件类型分布(case_type → count)— 民事/刑事/执行/非诉/...
    pub case_types: Vec<KeyCount>,
    /// document.source 分布:scan / llm_extract / chat 各多少
    pub document_sources: Vec<KeyCount>,
    /// 元典 API 累计调用次数(approximation:cases.risk_assessment_at IS NOT NULL ...)
    pub yuandian_queried_cases: i64,
    /// 写材料工具:save_artifact 生成的文书总数(documents WHERE source='chat_artifact')。
    /// **注意区别于** chat_artifacts_total(source='chat',旧的生成型 chat 产物)。
    pub save_artifacts_total: i64,
    /// 写材料 doc_type 分布(category → count):民事起诉状/答辩状/执行悬赏申请书... 各多少。
    /// 脱敏安全:category 是文书类型名,不含当事人信息。
    pub save_artifact_by_type: Vec<KeyCount>,
    /// 本地 KB 累计命中次数(SUM yuandian_credits_monthly.kb_hits)—— 「越用越省钱」的"省"。
    pub kb_total_hits: i64,
    /// 元典 API 累计调用次数(SUM yuandian_credits_monthly.api_calls)—— 跟 kb_total_hits 比看命中率。
    pub yuandian_total_calls: i64,
}

/// `(key, count)` 简易计数对(用于分布统计)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyCount {
    pub key: String,
    pub count: i64,
}

/// chat 用量按 (model, task_type) 聚合的统计样本。
///
/// 给作者拿来对比"flash vs pro" / "自由问 vs 固定任务"的 token / 耗时。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatUsageBucket {
    pub model: String,
    /// "free_chat" / "generate_case_overview" / ...
    pub task_type: String,
    pub samples: i64,
    pub ok_samples: i64,
    /// 平均 prompt_tokens
    pub avg_prompt: i64,
    /// 平均 completion_tokens
    pub avg_completion: i64,
    /// 平均 latency_ms
    pub avg_latency_ms: i64,
    /// 平均 assistant 输出字符数
    pub avg_chars: i64,
}

/// 反馈 MD 用的 metrics 行。原是与 db::metrics::MetricRow 逐字节相同的独立 struct + 手抄转换,
/// 2026-06-03 直接复用 MetricRow(JSON 形状一致,前端 TS MetricSample 契约不变,B3/B8 收口)。
pub use crate::db::metrics::MetricRow as MetricSample;

/// 按 backend 聚合的统计样本。给反馈 MD 顶部一眼能看出对比。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendStat {
    pub backend: String,
    pub stage: String,
    pub samples: i64,
    pub ok_samples: i64,
    pub avg_ms: i64,
    pub p50_ms: i64,
    pub avg_chars: Option<i64>,
}

/// Settings 关键配置脱敏快照(2026-05-26 V0.1.11)。
///
/// 安全规则:
/// - 凡是 api_key / token / password 字段一律不出现原文,只标 `[SET]` / `[EMPTY]`
/// - endpoint 字段会过一次 strip_endpoint_auth,把 `https://user:pass@host` 里的凭证去掉
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSnapshot {
    pub setup_completed: bool,
    pub user_display_name_set: bool,
    pub mineru_endpoint: Option<String>,
    #[serde(default)]
    pub ocr_cloud_primary: String,
    pub deepseek_endpoint: Option<String>,
    #[serde(default)]
    pub credential_statuses: BTreeMap<String, CredentialStatusSnapshot>,
    pub local_model_dir: Option<String>,
    pub local_server_endpoint: Option<String>,
    pub local_server_auto_start: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialStatusSnapshot {
    pub handle: String,
    pub revision: i64,
    pub state: crate::credentials_bridge::BridgeCredentialState,
    pub secret_present: bool,
}

/// 系统级诊断(2026-05-26 V0.1.11)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    /// 数据目录绝对路径(脱敏前——这里 /Users/<username>/Library/... 一般不含当事人名)
    pub data_dir: String,
    /// 数据目录是否可写(权限自检)
    pub data_dir_writable: bool,
    /// 数据库文件大小(MB);文件不存在时 None
    pub db_size_mb: Option<f64>,
    /// extracts/ 目录文件数 + 总大小(MB);不存在时 None
    pub extracts_files: Option<u64>,
    pub extracts_size_mb: Option<f64>,
    /// reports/ + external/ 一起的总大小(MB)
    pub reports_size_mb: Option<f64>,
    /// 数据目录所在盘的剩余空间(GB)
    pub disk_free_gb: Option<f64>,
    /// pdftotext 是否在 PATH(影响 PDF 抽取链路第 2 档)
    pub pdftotext_available: bool,
    /// pdftoppm 是否在 PATH(本机 vision OCR 需要)
    pub pdftoppm_available: bool,
}

/// 本地 KB / 记忆目录诊断。反馈只收路径状态与数量,不读取记忆正文。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KbMemoryDiagnostic {
    pub local_kb_enabled: Option<bool>,
    pub local_kb_root_configured: Option<String>,
    pub local_kb_root_resolved: Option<String>,
    pub local_kb_state: String,
    pub memory_root_resolved: Option<String>,
    pub memory_root_exists: bool,
    pub memory_root_writable: bool,
    pub memory_notes_count: Option<u64>,
    pub memory_error: Option<String>,
}

/// 前端 console 上报的错误条目(2026-05-26 V0.1.11)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleError {
    /// "error" / "warn" / "unhandled" 等
    pub level: String,
    /// 错误消息(可能含 stack trace 片段)
    pub message: String,
    /// 时间戳 (ISO 8601 字符串,前端 new Date().toISOString())
    pub at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppStats {
    pub cases_total: i64,
    pub documents_total: i64,
    pub documents_done: i64,
    pub documents_skipped: i64,
    pub documents_failed: i64,
    pub documents_pending: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentFailure {
    /// 文件名(只显示文件名后缀部分,不含完整路径)
    pub filename: String,
    /// category 标签(起诉状 / 判决书 / ...)
    pub category: Option<String>,
    /// 创建时间
    pub created_at: String,
    /// 2026-05-25 V0.1.8 加:抽取失败的具体原因(三轮重试全失败后落库的 last_error)。
    /// 输出到反馈 MD 前会经 `sanitize_paths` 把绝对路径替换成 `<path>/<basename>`,
    /// 防止泄漏当事人姓名出现在路径里(如 `<home>/cases/张三/...`)。
    pub last_error: Option<String>,
}

/// 把 endpoint URL 里 `https://user:pass@host/...` 这种内嵌凭证脱掉,只留 scheme + host + path。
fn strip_endpoint_auth(url: &str) -> String {
    // 标准库没 URL 解析,手写够用:看是否含 "://" 后面是否有 "@" 在第一个 "/" 之前
    let trimmed = url.trim();
    if let Some(scheme_end) = trimmed.find("://") {
        let scheme = &trimmed[..scheme_end + 3];
        let rest = &trimmed[scheme_end + 3..];
        // rest 形如 "user:pass@host/path"
        let host_end = rest.find('/').unwrap_or(rest.len());
        let host_part = &rest[..host_end];
        let path_part = &rest[host_end..];
        let cleaned_host = match host_part.rfind('@') {
            Some(at) => &host_part[at + 1..],
            None => host_part,
        };
        return format!("{}{}{}", scheme, cleaned_host, path_part);
    }
    trimmed.to_string()
}

fn build_settings_snapshot(
    s: &crate::settings::Settings,
    credential_statuses: BTreeMap<String, CredentialStatusSnapshot>,
) -> SettingsSnapshot {
    SettingsSnapshot {
        setup_completed: s.setup_completed,
        user_display_name_set: s
            .user_display_name
            .as_deref()
            .map(|x| !x.trim().is_empty())
            .unwrap_or(false),
        mineru_endpoint: s.mineru_endpoint.as_deref().map(strip_endpoint_auth),
        ocr_cloud_primary: s.effective_ocr_cloud_primary().to_string(),
        deepseek_endpoint: s.cloud_llm_endpoint.as_deref().map(strip_endpoint_auth),
        credential_statuses,
        local_model_dir: s.local_model_dir.clone(),
        local_server_endpoint: s.ollama_endpoint.as_deref().map(strip_endpoint_auth),
        local_server_auto_start: s.local_server_auto_start.unwrap_or(true),
    }
}

pub fn settings_snapshot_for(
    settings: &crate::settings::Settings,
    credential_statuses: BTreeMap<String, CredentialStatusSnapshot>,
) -> SettingsSnapshot {
    build_settings_snapshot(settings, credential_statuses)
}

fn dir_size_and_count(dir: &std::path::Path) -> (u64, u64) {
    // 浅扫一层 + 子目录递归(extracts/<case_id>/*.md 这种两层结构)
    let mut size = 0u64;
    let mut count = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if let Ok(ft) = entry.file_type() {
                if ft.is_file() {
                    if let Ok(m) = entry.metadata() {
                        size += m.len();
                        count += 1;
                    }
                } else if ft.is_dir() {
                    let (s, c) = dir_size_and_count(&entry.path());
                    size += s;
                    count += c;
                }
            }
        }
    }
    (size, count)
}

fn cmd_in_path(cmd: &str) -> bool {
    let mut c = std::process::Command::new(cmd);
    c.arg("-v");
    crate::proc_util::hide_console_window_std(&mut c);
    c.output()
        .map(|o| o.status.success() || o.status.code() == Some(99))
        .unwrap_or(false)
}

fn collect_system_info() -> SystemInfo {
    use std::path::PathBuf;
    let data_dir = crate::db::app_data_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    let data_dir_pb = PathBuf::from(&data_dir);

    // 写权限:尝试创建 + 删一个探针文件
    let writable = {
        let probe = data_dir_pb.join(".caseboard_write_probe");
        match std::fs::write(&probe, b"x") {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                true
            }
            Err(_) => false,
        }
    };

    let db_size_mb = std::fs::metadata(data_dir_pb.join("caseboard.db"))
        .ok()
        .map(|m| m.len() as f64 / 1024.0 / 1024.0);

    let (ex_size, ex_count) = dir_size_and_count(&data_dir_pb.join("extracts"));
    let extracts_size_mb = if ex_count > 0 {
        Some(ex_size as f64 / 1024.0 / 1024.0)
    } else {
        None
    };
    let extracts_files = if ex_count > 0 { Some(ex_count) } else { None };

    let (rep_size, _) = dir_size_and_count(&data_dir_pb.join("reports"));
    let (ext_size, _) = dir_size_and_count(&data_dir_pb.join("external"));
    let rep_total = rep_size + ext_size;
    let reports_size_mb = if rep_total > 0 {
        Some(rep_total as f64 / 1024.0 / 1024.0)
    } else {
        None
    };

    // 磁盘剩余:用 df -k 取数据目录所在卷的 Available
    let disk_free_gb = std::process::Command::new("df")
        .arg("-k")
        .arg(&data_dir)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            // 第二行第 4 列是 Available (KB)
            let lines: Vec<&str> = s.lines().collect();
            lines.get(1).and_then(|line| {
                let cols: Vec<&str> = line.split_whitespace().collect();
                cols.get(3).and_then(|kb| kb.parse::<u64>().ok())
            })
        })
        .map(|kb| kb as f64 / 1024.0 / 1024.0);

    SystemInfo {
        data_dir,
        data_dir_writable: writable,
        db_size_mb,
        extracts_files,
        extracts_size_mb,
        reports_size_mb,
        disk_free_gb,
        pdftotext_available: cmd_in_path("pdftotext"),
        pdftoppm_available: cmd_in_path("pdftoppm"),
    }
}

fn can_write_dir(dir: &Path) -> bool {
    if !dir.is_dir() {
        return false;
    }
    let probe = dir.join(format!(".caseboard_write_probe_{}", uuid::Uuid::new_v4()));
    match std::fs::write(&probe, b"x") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn collect_kb_memory_diagnostic(settings: &crate::settings::Settings) -> KbMemoryDiagnostic {
    let local_kb_state = match crate::local_kb::status::detect_kb_status(settings) {
        crate::local_kb::status::KbStatus::Bound { .. } => "bound",
        crate::local_kb::status::KbStatus::Unbound { .. } => "unbound",
        crate::local_kb::status::KbStatus::PermissionDenied { .. } => "permission_denied",
    }
    .to_string();

    let mut out = KbMemoryDiagnostic {
        local_kb_enabled: settings.local_kb_enabled,
        local_kb_root_configured: settings.local_kb_root.clone(),
        local_kb_state,
        ..KbMemoryDiagnostic::default()
    };

    match crate::memory_vault::resolve_memory_paths_from_settings(settings) {
        Ok(paths) => {
            out.local_kb_root_resolved = Some(paths.kb_root.to_string_lossy().to_string());
            out.memory_root_resolved = Some(paths.memory_root.to_string_lossy().to_string());
            out.memory_root_exists = paths.memory_root.is_dir();
            out.memory_root_writable = can_write_dir(&paths.memory_root);
            if out.memory_root_exists {
                match crate::memory_vault::list_notes_in_root(&paths.memory_root) {
                    Ok(notes) => out.memory_notes_count = Some(notes.len() as u64),
                    Err(e) => out.memory_error = Some(e),
                }
            }
        }
        Err(e) => out.memory_error = Some(e),
    }

    out
}

/// 从 DB + settings + 系统调用收集诊断信息。
///
/// `console_errors`:前端打开反馈弹窗时把累积的 console.error / window.onerror 数组传过来。
/// 默认空向量即可——这只是观测信号,缺失不阻塞反馈。
pub async fn collect(
    pool: &SqlitePool,
    console_errors: Vec<ConsoleError>,
) -> Result<DiagnosticInfo, String> {
    let app_version = env!("CARGO_PKG_VERSION").to_string();

    // 匿名识别码:首次启动时生成 + 持久化(uuid v4 前 8 位足够区分)
    let client_id_short = crate::settings::ensure_client_id()
        .ok()
        .map(|s| s.chars().take(8).collect::<String>())
        .unwrap_or_else(|| "unknown".to_string());

    // 系统版本 + 架构
    let os_version = collect_os_version();
    let language = std::env::var("LANG").unwrap_or_else(|_| "zh-CN".to_string());

    // settings
    let settings = crate::settings::read_settings().unwrap_or_default();
    let llm_provider = settings.effective_llm_provider().to_string();
    let ocr_provider = settings.effective_ocr_provider().to_string();

    // 本机服务状态(轻量探测,不连不报错)
    let local_server_status = if settings.needs_local_server() {
        match reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(500))
            .build()
        {
            Ok(client) => match client.get("http://127.0.0.1:8899/health").send().await {
                Ok(r) if r.status().is_success() => "运行中(:8899)".to_string(),
                _ => "未启动 / 不可达".to_string(),
            },
            Err(_) => "无法探测".to_string(),
        }
    } else {
        "未启用(走云端)".to_string()
    };

    // DeepSeek 余额(只读缓存,不发请求 — 反馈对话框不该慢)
    let deepseek_balance = crate::deepseek::cached_balance(pool)
        .await
        .ok()
        .flatten()
        .map(|b| b.total_balance);

    // 统计
    let cases_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cases")
        .fetch_one(pool)
        .await
        .map_err(|e| format!("查 cases 数失败: {}", e))?;

    let documents_total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE deleted_at IS NULL")
            .fetch_one(pool)
            .await
            .map_err(|e| format!("查 documents 数失败: {}", e))?;

    let documents_done: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents WHERE deleted_at IS NULL AND extraction_status = 'done'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    let documents_skipped: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents WHERE deleted_at IS NULL AND extraction_status = 'skipped'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    let documents_failed: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents WHERE deleted_at IS NULL AND extraction_status = 'failed'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);
    let documents_pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents WHERE deleted_at IS NULL AND extraction_status = 'pending'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    // 最近失败的最多 200 条(filename + category + created_at + last_error)
    // 2026-05-25 V0.1.8 加 last_error
    // 2026-05-25 V0.1.9 改 LIMIT 5 → LIMIT 200:朋友反馈 34 个失败只看到 5 条没法诊断,
    //                  作者要看全量。200 上限防止极端情况撑爆 MD 文件
    let rows: Vec<(String, Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT filename, category, created_at, last_error FROM documents \
         WHERE deleted_at IS NULL AND extraction_status = 'failed' \
         ORDER BY created_at DESC LIMIT 200",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let recent_failures = rows
        .into_iter()
        .map(
            |(filename, category, created_at, last_error)| RecentFailure {
                filename,
                category,
                created_at,
                last_error,
            },
        )
        .collect();

    // 2026-05-26 V0.1.11 加:settings 脱敏快照 + 系统级 + stderr ring buffer + 前端 console
    let credential_statuses = crate::credentials_commands::list_credential_metadata(None)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|metadata| {
            (
                metadata.provider_or_connector_id,
                CredentialStatusSnapshot {
                    handle: metadata.handle.as_str().to_owned(),
                    revision: metadata.revision,
                    state: metadata.state,
                    secret_present: metadata.secret_present,
                },
            )
        })
        .collect();
    let settings_snapshot = build_settings_snapshot(&settings, credential_statuses);
    let system_info = collect_system_info();
    let kb_memory = collect_kb_memory_diagnostic(&settings);
    let stderr_tail = crate::diagnostic_log::snapshot();
    let runtime_traces =
        crate::chat::diagnostics::read_recent_runtime_traces().unwrap_or_else(|error| {
            crate::dlog!("[feedback] 读取 Pi Runtime 结构化诊断失败: {}", error);
            Vec::new()
        });

    // 2026-05-26 V0.1.12:最近 200 条抽取 metrics + 按 backend 聚合
    // (metrics_tail 现直接是 Vec<MetricRow>,原逐字段手抄随 B3/B8 收口删除)
    let metrics_tail: Vec<MetricSample> = crate::db::metrics::query_recent(pool, 200)
        .await
        .unwrap_or_default();
    let metrics_summary = summarize_metrics(&metrics_tail);

    // 2026-05-27 V0.1.13+:案件 AI 助手用量统计(只取数字 + meta,**绝不**含 content)
    let chat_usage = summarize_chat_usage(pool).await.unwrap_or_default();

    // 2026-05-27 V0.1.13+:功能模块用量(聚合数字,无业务标识)
    let feature_usage = collect_feature_usage(pool).await.unwrap_or_default();

    Ok(DiagnosticInfo {
        client_id_short,
        app_version,
        os_version,
        language,
        llm_provider,
        ocr_provider,
        local_server_status,
        deepseek_balance,
        stats: AppStats {
            cases_total,
            documents_total,
            documents_done,
            documents_skipped,
            documents_failed,
            documents_pending,
        },
        recent_failures,
        settings_snapshot,
        system_info,
        kb_memory,
        stderr_tail,
        runtime_traces,
        console_errors,
        metrics_tail,
        metrics_summary,
        chat_usage,
        feature_usage,
    })
}

/// 收集功能模块用量(2026-05-27 V0.1.13+)。全部 SQL COUNT 聚合,无业务标识。
async fn collect_feature_usage(pool: &SqlitePool) -> Result<FeatureUsage, sqlx::Error> {
    let chat_messages_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages")
        .fetch_one(pool)
        .await
        .unwrap_or(0);

    let chat_artifacts_total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents WHERE source = 'chat' AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let cases_with_analysis_report: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cases WHERE case_report_path IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    let cases_with_risk_report: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cases WHERE risk_assessment_path IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    let cases_with_deep_dive_report: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cases WHERE deep_dive_report_path IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    let cases_with_full_report: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cases WHERE full_report_path IS NOT NULL")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    let cases_with_user_overrides: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cases WHERE user_overrides_json IS NOT NULL \
         AND user_overrides_json != '' AND user_overrides_json != '{}'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let case_type_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT case_type, COUNT(*) FROM cases GROUP BY case_type ORDER BY COUNT(*) DESC",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let case_types: Vec<KeyCount> = case_type_rows
        .into_iter()
        .map(|(k, c)| KeyCount { key: k, count: c })
        .collect();

    let source_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT source, COUNT(*) FROM documents WHERE deleted_at IS NULL \
         GROUP BY source ORDER BY COUNT(*) DESC",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let document_sources: Vec<KeyCount> = source_rows
        .into_iter()
        .map(|(k, c)| KeyCount { key: k, count: c })
        .collect();

    let yuandian_queried_cases: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM cases WHERE risk_assessment_at IS NOT NULL \
         OR deep_dive_at IS NOT NULL OR full_report_at IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    // V0.3 写材料工具:save_artifact 落的文书 source='chat_artifact'(区别于旧 'chat')。
    let save_artifacts_total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents WHERE source = 'chat_artifact' AND deleted_at IS NULL",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let artifact_type_rows: Vec<(Option<String>, i64)> = sqlx::query_as(
        "SELECT category, COUNT(*) FROM documents \
         WHERE source = 'chat_artifact' AND deleted_at IS NULL \
         GROUP BY category ORDER BY COUNT(*) DESC",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    let save_artifact_by_type: Vec<KeyCount> = artifact_type_rows
        .into_iter()
        .map(|(k, c)| KeyCount {
            key: k.unwrap_or_else(|| "未分类".into()),
            count: c,
        })
        .collect();

    // 本地 KB 命中 vs 元典调用(看「越用越省钱」是否生效:命中越高越省积分)。
    // 真实可靠源 = `yuandian_credits_monthly`(hooks.rs 的 record_kb_hit / record_yuandian_call
    // 在工具实时路径写入,也是设置页积分卡的数据源)。**注意**:chat_tasks.kb_hits 在当前
    // agent_loop 流程下从不写、恒 0,别从那取(否则误报「省钱功能坏了」)。
    let kb_total_hits: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(kb_hits), 0) FROM yuandian_credits_monthly")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
    let yuandian_total_calls: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(api_calls), 0) FROM yuandian_credits_monthly")
            .fetch_one(pool)
            .await
            .unwrap_or(0);

    Ok(FeatureUsage {
        chat_messages_total,
        chat_artifacts_total,
        cases_with_analysis_report,
        cases_with_risk_report,
        cases_with_deep_dive_report,
        cases_with_full_report,
        cases_with_user_overrides,
        case_types,
        document_sources,
        yuandian_queried_cases,
        save_artifacts_total,
        save_artifact_by_type,
        kb_total_hits,
        yuandian_total_calls,
    })
}

/// 单行 chat 用量(只取 model / task_type / 数字字段,**绝不** SELECT content)。
#[derive(sqlx::FromRow)]
struct ChatUsageRawRow {
    model: String,
    task_type: Option<String>,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
    latency_ms: Option<i64>,
    char_count: i64,
    error_short: Option<String>,
}

/// 查 chat_messages 表的近 200 条 assistant 行,按 (model, task_type) 聚合。
/// **隐私铁律**:此函数**只**查数字 + meta 字段,绝不 SELECT content。
async fn summarize_chat_usage(pool: &SqlitePool) -> Result<Vec<ChatUsageBucket>, sqlx::Error> {
    // 注意 SELECT 列表里 **没有** content:这是隐私铁律 #3 的最后一道防线
    let rows: Vec<ChatUsageRawRow> = sqlx::query_as(
        "SELECT \
                COALESCE(model, '(unknown)') AS model, \
                task_type, \
                prompt_tokens, \
                completion_tokens, \
                latency_ms, \
                LENGTH(content) AS char_count, \
                error_short \
             FROM chat_messages \
             WHERE role = 'assistant' \
             ORDER BY created_at DESC \
             LIMIT 200",
    )
    .fetch_all(pool)
    .await?;

    use std::collections::HashMap;
    #[derive(Default)]
    struct Acc {
        samples: i64,
        ok: i64,
        p_sum: i64,
        c_sum: i64,
        lat_sum: i64,
        char_sum: i64,
        p_cnt: i64,
        c_cnt: i64,
        lat_cnt: i64,
        char_cnt: i64,
    }
    let mut buckets: HashMap<(String, String), Acc> = HashMap::new();
    for row in rows {
        let task = row.task_type.unwrap_or_else(|| "free_chat".into());
        let acc = buckets.entry((row.model, task)).or_default();
        acc.samples += 1;
        if row.error_short.is_none() {
            acc.ok += 1;
        }
        if let Some(v) = row.prompt_tokens {
            acc.p_sum += v;
            acc.p_cnt += 1;
        }
        if let Some(v) = row.completion_tokens {
            acc.c_sum += v;
            acc.c_cnt += 1;
        }
        if let Some(v) = row.latency_ms {
            acc.lat_sum += v;
            acc.lat_cnt += 1;
        }
        if row.char_count > 0 {
            acc.char_sum += row.char_count;
            acc.char_cnt += 1;
        }
    }
    let mut out: Vec<ChatUsageBucket> = buckets
        .into_iter()
        .map(|((model, task_type), a)| ChatUsageBucket {
            model,
            task_type,
            samples: a.samples,
            ok_samples: a.ok,
            avg_prompt: if a.p_cnt > 0 { a.p_sum / a.p_cnt } else { 0 },
            avg_completion: if a.c_cnt > 0 { a.c_sum / a.c_cnt } else { 0 },
            avg_latency_ms: if a.lat_cnt > 0 {
                a.lat_sum / a.lat_cnt
            } else {
                0
            },
            avg_chars: if a.char_cnt > 0 {
                a.char_sum / a.char_cnt
            } else {
                0
            },
        })
        .collect();
    out.sort_by_key(|b| std::cmp::Reverse(b.samples));
    Ok(out)
}

/// 按 (backend, stage) 分桶聚合,算样本数 / 成功数 / 平均 / 中位 / 平均字数。
/// 目的:反馈 MD 顶部一眼能看出本机 vs 云端速度对比。
fn summarize_metrics(samples: &[MetricSample]) -> Vec<BackendStat> {
    use std::collections::HashMap;
    let mut buckets: HashMap<(String, String), Vec<&MetricSample>> = HashMap::new();
    for s in samples {
        buckets
            .entry((s.backend.clone(), s.stage.clone()))
            .or_default()
            .push(s);
    }
    let mut stats: Vec<BackendStat> = buckets
        .into_iter()
        .map(|((backend, stage), entries)| {
            let samples = entries.len() as i64;
            let ok_entries: Vec<&&MetricSample> =
                entries.iter().filter(|e| e.outcome == "ok").collect();
            let ok_samples = ok_entries.len() as i64;
            let mut elapsed: Vec<i64> = ok_entries.iter().map(|e| e.elapsed_ms).collect();
            elapsed.sort_unstable();
            let avg_ms = if !elapsed.is_empty() {
                elapsed.iter().sum::<i64>() / elapsed.len() as i64
            } else {
                0
            };
            let p50_ms = if !elapsed.is_empty() {
                elapsed[elapsed.len() / 2]
            } else {
                0
            };
            let chars: Vec<i64> = ok_entries.iter().filter_map(|e| e.text_chars).collect();
            let avg_chars = if !chars.is_empty() {
                Some(chars.iter().sum::<i64>() / chars.len() as i64)
            } else {
                None
            };
            BackendStat {
                backend,
                stage,
                samples,
                ok_samples,
                avg_ms,
                p50_ms,
                avg_chars,
            }
        })
        .collect();
    // 排序:stage(text_extract → ocr → llm_extract)→ backend 字典序
    stats.sort_by(|a, b| {
        let order = |s: &str| match s {
            "text_extract" => 0,
            "ocr" => 1,
            "llm_extract" => 2,
            _ => 9,
        };
        order(&a.stage)
            .cmp(&order(&b.stage))
            .then(a.backend.cmp(&b.backend))
    });
    stats
}

fn collect_os_version() -> String {
    let arch = std::env::consts::ARCH; // aarch64 / x86_64
    let os = std::env::consts::OS;
    format_os_version(os, collect_platform_version(), arch)
}

fn format_os_version(os: &str, version: Option<String>, arch: &str) -> String {
    let ver = version
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    format!("{} {} · {}", os, ver, arch)
}

#[cfg(target_os = "macos")]
fn collect_platform_version() -> Option<String> {
    std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
}

#[cfg(target_os = "windows")]
fn collect_platform_version() -> Option<String> {
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/C", "ver"]);
    crate::proc_util::hide_console_window_std(&mut cmd);
    cmd.output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn collect_platform_version() -> Option<String> {
    None
}

/// 把诊断信息 + 用户描述拼成 MD,写到 ~/Desktop/案件看板反馈_<timestamp>.md。
/// 返回最终文件的绝对路径。
/// 2026-05-27 V0.1.13+ · 调用默认邮件客户端发反馈。
///
/// macOS 主路径:osascript 调 Mail.app 起草邮件并带附件。
/// 其他平台 / macOS fallback:用系统默认邮件客户端打开 `mailto:` 链接,附件需用户手动添加。
///
/// 返回 `(path_used, warning)`:
///   - path_used: "applescript" 或 "mailto"
///   - warning: 走 mailto fallback 时填提示用户"附件请手动拖入";applescript 时空
pub async fn send_via_default_mail(
    md_path: &str,
    to: &str,
    subject: &str,
) -> Result<(String, String), String> {
    #[cfg(not(target_os = "macos"))]
    {
        return fallback_mailto(to, subject).await.map(|_| {
            (
                "mailto".to_string(),
                format!(
                    "已打开默认邮件客户端。当前系统不支持自动添加附件,请手动把这个 MD 添加到邮件:{}",
                    md_path
                ),
            )
        });
    }

    #[cfg(target_os = "macos")]
    {
        // 安全:转义路径中的 " 和 \ 防 AppleScript 注入
        fn escape_for_apple_script(s: &str) -> String {
            s.replace('\\', "\\\\").replace('"', "\\\"")
        }
        let escaped_path = escape_for_apple_script(md_path);
        let escaped_to = escape_for_apple_script(to);
        let escaped_subject = escape_for_apple_script(subject);

        let script = format!(
            r#"tell application "Mail"
    activate
    set newMessage to make new outgoing message with properties {{subject:"{subject}", visible:true}}
    tell newMessage
        make new to recipient at end of to recipients with properties {{address:"{to}"}}
        try
            make new attachment with properties {{file name:(POSIX file "{path}")}} at after the last paragraph
        end try
    end tell
end tell"#,
            subject = escaped_subject,
            to = escaped_to,
            path = escaped_path,
        );

        // 调 osascript
        let output = tokio::process::Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => Ok(("applescript".to_string(), String::new())),
            Ok(out) => {
                // AppleScript 失败 — 走 fallback mailto
                let stderr = String::from_utf8_lossy(&out.stderr);
                crate::dlog!("[feedback] AppleScript 失败,fallback mailto: {}", stderr);
                fallback_mailto(to, subject)
                .await
                .map(|_| {
                    (
                        "mailto".to_string(),
                        format!(
                            "Mail.app 未成功(可能没装 / 未授权 AppleScript),已用 mailto 打开默认邮件客户端。\
                            **附件请手动把这个 MD 拖进邮件**:{}",
                            md_path
                        ),
                    )
                })
            }
            Err(e) => {
                crate::dlog!("[feedback] osascript 进程启动失败: {}", e);
                fallback_mailto(to, subject).await.map(|_| {
                    (
                        "mailto".to_string(),
                        format!(
                            "osascript 不可用,已用 mailto 打开默认邮件客户端。\
                        **附件请手动把这个 MD 拖进邮件**:{}",
                            md_path
                        ),
                    )
                })
            }
        }
    }
}

/// fallback 路径:用系统默认邮件客户端打开新邮件(无附件)。
async fn fallback_mailto(to: &str, subject: &str) -> Result<(), String> {
    let url = mailto_url(to, subject);
    tauri_plugin_opener::open_url(&url, None::<&str>)
        .map_err(|e| format!("打开默认邮件客户端失败: {}", e))
}

fn mailto_url(to: &str, subject: &str) -> String {
    format!(
        "mailto:{}?subject={}",
        urlencode_simple(to),
        urlencode_simple(subject),
    )
}

/// 极简 URL encode(仅处理 mailto 必须的字符)。不引入新 crate。
///
/// 保留:RFC 3986 unreserved (A-Za-z0-9-_.~) + mailto 安全字符 `@ .`。
/// 空格转 `%20`,其它(含中文)按 utf-8 byte percent-encode。
fn urlencode_simple(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        let keep = c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '@');
        if keep {
            out.push(c);
        } else if c == ' ' {
            out.push_str("%20");
        } else {
            // 多字节 / 其它字符 → percent-encode 每个 utf-8 byte
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{:02X}", b));
            }
        }
    }
    out
}

pub fn save_to_desktop(info: &DiagnosticInfo, user_description: &str) -> Result<PathBuf, String> {
    let md = render_md(info, user_description);
    let ts = chrono::Local::now().format("%Y-%m-%d_%H%M").to_string();
    let filename = format!("案件看板反馈_{}.md", ts);

    // 2026-06-23 v0.3.26.1 Windows fix:先试桌面;写失败(OneDrive 重定向、只读、权限等)
    // 兜底到 ~/Documents/CaseBoard/feedback/。不卡死反馈流程。
    if let Some(desktop) = dirs_desktop() {
        let path = desktop.join(&filename);
        if std::fs::write(&path, &md).is_ok() {
            return Ok(path);
        }
    }

    let fallback = dirs_fallback_feedback_dir()
        .ok_or_else(|| "无法定位任何可写目录(桌面/Documents/CaseBoard/feedback)".to_string())?;
    std::fs::create_dir_all(&fallback).map_err(|e| format!("建反馈目录失败: {}", e))?;
    let path = fallback.join(&filename);
    std::fs::write(&path, md).map_err(|e| format!("写入失败: {}", e))?;
    Ok(path)
}

pub async fn upload_to_cloud(
    info: &DiagnosticInfo,
    user_description: &str,
    screenshots: &[FeedbackScreenshotInput],
) -> Result<(), String> {
    let (base, key) = match (FEEDBACK_SUPABASE_URL, FEEDBACK_SUPABASE_KEY) {
        (Some(base), Some(key)) if !base.trim().is_empty() && !key.trim().is_empty() => {
            (base.trim_end_matches('/'), key)
        }
        _ => return Err("反馈上传未配置:缺少 CASEBOARD_TELEMETRY_URL / KEY".into()),
    };
    let report_id = uuid::Uuid::new_v4().to_string();
    let (payload, prepared_screenshots) =
        build_cloud_feedback_upload(info, user_description, screenshots, &report_id)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(FEEDBACK_UPLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("HTTP client 构建失败:{e}"))?;

    for screenshot in &prepared_screenshots {
        upload_feedback_screenshot(&client, base, key, screenshot).await?;
    }

    let url = format!("{base}/rest/v1/feedback_reports");
    let response = client
        .post(&url)
        .header("apikey", key)
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .header("Prefer", "return=minimal")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("上传反馈失败:{e}"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    let snippet: String = body.chars().take(300).collect();
    Err(format!(
        "上传反馈记录失败:Supabase {}:{snippet}",
        status.as_u16()
    ))
}

async fn upload_feedback_screenshot(
    client: &reqwest::Client,
    base: &str,
    key: &str,
    screenshot: &PreparedFeedbackScreenshot,
) -> Result<(), String> {
    let url = format!(
        "{base}/storage/v1/object/{}/{}",
        FEEDBACK_SCREENSHOT_BUCKET, screenshot.metadata.storage_path
    );
    let response = client
        .post(url)
        .header("apikey", key)
        .header("Authorization", format!("Bearer {key}"))
        .header("Content-Type", &screenshot.metadata.mime_type)
        .header("x-upsert", "false")
        .body(screenshot.bytes.clone())
        .send()
        .await
        .map_err(|e| format!("上传截图失败:{e}"))?;
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }
    let body = response.text().await.unwrap_or_default();
    let snippet: String = body.chars().take(300).collect();
    Err(format!(
        "上传截图失败:Supabase Storage {}:{snippet}",
        status.as_u16()
    ))
}

pub(crate) fn build_cloud_feedback_upload(
    info: &DiagnosticInfo,
    user_description: &str,
    screenshots: &[FeedbackScreenshotInput],
    report_id: &str,
) -> Result<(CloudFeedbackPayload, Vec<PreparedFeedbackScreenshot>), String> {
    let prepared_screenshots = prepare_feedback_screenshots(report_id, screenshots)?;
    let report_md = truncate_chars(
        &render_md(info, user_description),
        FEEDBACK_REPORT_MAX_CHARS,
        "\n\n> 反馈内容过长,云端副本已截断;可让用户另存本地 MD 补充。\n",
    );
    let desc = sanitize_paths(user_description.trim());
    let description_preview = if desc.is_empty() {
        "(用户未填写)".to_string()
    } else {
        truncate_chars(&desc, FEEDBACK_DESCRIPTION_PREVIEW_CHARS, "…")
    };
    let payload = CloudFeedbackPayload {
        id: report_id.to_string(),
        client_id_short: info.client_id_short.clone(),
        app_version: info.app_version.clone(),
        os_version: info.os_version.clone(),
        description_preview,
        report_md,
        screenshots: prepared_screenshots
            .iter()
            .map(|screenshot| screenshot.metadata.clone())
            .collect(),
        status: "open".to_string(),
    };
    Ok((payload, prepared_screenshots))
}

fn prepare_feedback_screenshots(
    report_id: &str,
    screenshots: &[FeedbackScreenshotInput],
) -> Result<Vec<PreparedFeedbackScreenshot>, String> {
    let report_id = uuid::Uuid::parse_str(report_id)
        .map_err(|_| "反馈编号格式无效".to_string())?
        .to_string();
    if screenshots.len() > FEEDBACK_SCREENSHOT_MAX_COUNT {
        return Err(format!("最多上传 {} 张截图", FEEDBACK_SCREENSHOT_MAX_COUNT));
    }

    let mut total_bytes = 0usize;
    let mut prepared = Vec::with_capacity(screenshots.len());
    for (index, screenshot) in screenshots.iter().enumerate() {
        let mime_type = screenshot.mime_type.trim().to_ascii_lowercase();
        if !matches!(
            mime_type.as_str(),
            "image/png" | "image/jpeg" | "image/webp"
        ) {
            return Err(format!(
                "第 {} 张截图格式不支持,仅支持 PNG、JPEG、WebP",
                index + 1
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(screenshot.data_base64.trim())
            .map_err(|_| format!("第 {} 张截图数据损坏", index + 1))?;
        if bytes.is_empty() {
            return Err(format!("第 {} 张截图为空", index + 1));
        }
        if bytes.len() > FEEDBACK_SCREENSHOT_MAX_BYTES {
            return Err(format!("单张截图不能超过 5 MB(第 {} 张超限)", index + 1));
        }
        total_bytes = total_bytes.saturating_add(bytes.len());
        if total_bytes > FEEDBACK_SCREENSHOT_TOTAL_MAX_BYTES {
            return Err("截图总大小不能超过 10 MB".into());
        }
        let signature_matches = match mime_type.as_str() {
            "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
            "image/webp" => {
                bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
            }
            _ => false,
        };
        if !signature_matches {
            return Err(format!("第 {} 张图片内容与格式不一致", index + 1));
        }

        let extension = match mime_type.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            _ => "webp",
        };
        let ordinal = index + 1;
        prepared.push(PreparedFeedbackScreenshot {
            metadata: CloudFeedbackScreenshot {
                filename: format!("反馈截图 {ordinal}.{extension}"),
                mime_type,
                storage_path: format!("{report_id}/{ordinal}.{extension}"),
                size_bytes: bytes.len(),
            },
            bytes,
        });
    }
    Ok(prepared)
}

fn truncate_chars(value: &str, max_chars: usize, suffix: &str) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let suffix_len = suffix.chars().count();
    let keep = max_chars.saturating_sub(suffix_len);
    let mut out: String = value.chars().take(keep).collect();
    out.push_str(suffix);
    out
}

/// 2026-06-23 v0.3.26.1:桌面不可写时反馈 MD 的兜底目录。
/// `UserDirs::document_dir()` 处理 Win 已知文件夹(能识别重定向的 OneDrive Documents),
/// Linux/mac 走 ~/Documents。失败再退回 $HOME / $USERPROFILE + Documents。
fn dirs_fallback_feedback_dir() -> Option<PathBuf> {
    if let Some(ud) = directories::UserDirs::new() {
        if let Some(doc) = ud.document_dir() {
            return Some(doc.join("CaseBoard").join("feedback"));
        }
        return Some(
            ud.home_dir()
                .join("Documents")
                .join("CaseBoard")
                .join("feedback"),
        );
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(
        PathBuf::from(home)
            .join("Documents")
            .join("CaseBoard")
            .join("feedback"),
    )
}

/// 跨平台拿桌面目录路径。
///
/// 2026-06-15:原来只读 `$HOME` 再 join "Desktop" —— **Windows 上 `HOME` 多半没设**
/// (Windows 用 `USERPROFILE`)→ 直接返回 None → 反馈按钮报「无法定位桌面路径」存不下。
/// 改用 `directories::UserDirs::desktop_dir()`:Windows 走 Known Folder(能正确处理 OneDrive
/// 重定向 / 本地化桌面名),mac/Linux 走 ~/Desktop / XDG。多层兜底确保拿到可写目录。
fn dirs_desktop() -> Option<PathBuf> {
    if let Some(ud) = directories::UserDirs::new() {
        if let Some(d) = ud.desktop_dir() {
            return Some(d.to_path_buf());
        }
        // 个别环境 desktop_dir 为 None(如 Linux 未配 XDG)→ 退到 home/Desktop
        return Some(ud.home_dir().join("Desktop"));
    }
    // 最后兜底:HOME(mac/Linux)/ USERPROFILE(Windows)
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    Some(PathBuf::from(home).join("Desktop"))
}

fn render_md(info: &DiagnosticInfo, user_description: &str) -> String {
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();

    let mut md = String::new();
    md.push_str("# 案件看板 · 用户反馈\n\n");
    md.push_str(&format!("**反馈方 ID**:`{}`(匿名)\n", info.client_id_short));
    md.push_str(&format!("**时间**:{}\n", ts));
    md.push_str(&format!("**App 版本**:{}\n", info.app_version));
    md.push_str(&format!("**操作系统**:{}\n", info.os_version));
    md.push_str(&format!("**语言**:{}\n", info.language));
    md.push('\n');

    md.push_str("## 当前配置\n\n");
    md.push_str(&format!(
        "- LLM 后端:**{}**\n",
        provider_display(&info.llm_provider)
    ));
    md.push_str(&format!(
        "- OCR 后端:**{}**\n",
        provider_display(&info.ocr_provider)
    ));
    md.push_str(&format!("- 本机服务:{}\n", info.local_server_status));
    if let Some(b) = info.deepseek_balance {
        md.push_str(&format!("- DeepSeek 余额:¥{:.2}\n", b));
    }
    md.push('\n');

    md.push_str("## 数据状态\n\n");
    md.push_str(&format!("- 已导入案件:{}\n", info.stats.cases_total));
    md.push_str(&format!(
        "- 文档总数:{}(done {} / skipped {} / failed {} / pending {})\n",
        info.stats.documents_total,
        info.stats.documents_done,
        info.stats.documents_skipped,
        info.stats.documents_failed,
        info.stats.documents_pending,
    ));
    md.push('\n');

    // 2026-05-27 V0.1.13+:功能模块用量(老板用来判断同事用了哪些功能)
    {
        let f = &info.feature_usage;
        md.push_str("## 功能模块用量\n\n");
        md.push_str(&format!(
            "- 案件 AI 助手:**{}** 条聊天消息 · **{}** 份 chat artifact\n",
            f.chat_messages_total, f.chat_artifacts_total
        ));
        // V0.3 写材料工具用量(同事测试写起诉状/答辩状/律师函等的覆盖)
        let artifact_types = if f.save_artifact_by_type.is_empty() {
            String::new()
        } else {
            let t: Vec<String> = f
                .save_artifact_by_type
                .iter()
                .map(|k| format!("{}×{}", k.key, k.count))
                .collect();
            format!("({})", t.join(" / "))
        };
        md.push_str(&format!(
            "- 写材料(save_artifact 文书):**{}** 份 {}\n",
            f.save_artifacts_total, artifact_types
        ));
        // 本地 KB 命中 vs 元典调用 —— 看「越用越省钱」是否真生效
        let kb_rate = if f.kb_total_hits + f.yuandian_total_calls > 0 {
            format!(
                " · 命中率 {:.0}%",
                100.0 * f.kb_total_hits as f64 / (f.kb_total_hits + f.yuandian_total_calls) as f64
            )
        } else {
            String::new()
        };
        md.push_str(&format!(
            "- 本地 KB 命中 **{}** 次 vs 元典 API 调用 **{}** 次{}\n",
            f.kb_total_hits, f.yuandian_total_calls, kb_rate
        ));
        md.push_str(&format!(
            "- 案件分析报告(LLM 全局抽):**{}** 个案件已生成\n",
            f.cases_with_analysis_report
        ));
        md.push_str(&format!(
            "- 元典风险报告:**{}** 个案件 · 深挖:**{}** · 完整报告:**{}**\n",
            f.cases_with_risk_report, f.cases_with_deep_dive_report, f.cases_with_full_report,
        ));
        md.push_str(&format!(
            "- 元典 API 调用过的案件:**{}** 个(risk / deep_dive / full 任一)\n",
            f.yuandian_queried_cases
        ));
        md.push_str(&format!(
            "- 编辑模式 user overrides:**{}** 个案件手改过字段\n",
            f.cases_with_user_overrides
        ));
        if !f.case_types.is_empty() {
            let types: Vec<String> = f
                .case_types
                .iter()
                .map(|k| format!("{}×{}", k.key, k.count))
                .collect();
            md.push_str(&format!("- 案件类型分布:{}\n", types.join(" / ")));
        }
        if !f.document_sources.is_empty() {
            let srcs: Vec<String> = f
                .document_sources
                .iter()
                .map(|k| format!("{}×{}", k.key, k.count))
                .collect();
            md.push_str(&format!("- documents.source 分布:{}\n", srcs.join(" / ")));
        }
        md.push('\n');
    }

    if !info.recent_failures.is_empty() {
        md.push_str(&format!(
            "## 最近抽取失败的文档({} 条,最多 200)\n\n",
            info.recent_failures.len()
        ));
        for f in &info.recent_failures {
            let cat = f.category.as_deref().unwrap_or("未分类");
            md.push_str(&format!(
                "- **[{}] {}** · {}\n",
                cat, f.filename, f.created_at
            ));
            // 失败原因(sanitize 后),帮作者快速判断是哪类问题(限流 / token / 大小 / 网络...)
            if let Some(err) = &f.last_error {
                let safe = sanitize_runtime_log_entry(err).replace('`', "'");
                md.push_str(&format!("  - 错误:`{}`\n", safe));
            }
        }
        md.push('\n');
    }

    // 2026-05-26 V0.1.11:Settings 脱敏快照
    let s = &info.settings_snapshot;
    md.push_str("## Settings 脱敏快照\n\n");
    md.push_str(&format!(
        "- 完成 onboarding:{}\n",
        if s.setup_completed { "是" } else { "否" }
    ));
    md.push_str(&format!(
        "- 用户称呼已设置:{}\n",
        if s.user_display_name_set {
            "是"
        } else {
            "否"
        }
    ));
    // 2026-05-26 V0.1.11 老板补强:key 状态显式三态(已填+已验证 / **已填+未验证** / 未填),
    //   方便作者拿到反馈 MD 第一眼就能识别"朋友 key 没验证"这种盲区
    md.push_str(&format!(
        "- MinerU key:{} · endpoint:{}\n",
        key_state_display(s.credential_statuses.get("ocr.mineru")),
        s.mineru_endpoint.as_deref().unwrap_or("(默认)"),
    ));
    md.push_str(&format!(
        "- PaddleOCR key:{} · 云端 OCR 主力:{}\n",
        key_state_display(s.credential_statuses.get("ocr.paddle_vl")),
        if s.ocr_cloud_primary.is_empty() {
            "mineru"
        } else {
            &s.ocr_cloud_primary
        },
    ));
    md.push_str(&format!(
        "- DeepSeek key:{} · endpoint:{}\n",
        key_state_display(s.credential_statuses.get("llm.deepseek")),
        s.deepseek_endpoint.as_deref().unwrap_or("(默认)"),
    ));
    md.push_str(&format!(
        "- 元典 key:{}\n",
        key_state_display(s.credential_statuses.get("connector.yuandian")),
    ));
    md.push_str(&format!(
        "- 本机模型目录:{}\n",
        s.local_model_dir
            .as_deref()
            .unwrap_or("(默认 ~/.cache/caseboard/models)"),
    ));
    md.push_str(&format!(
        "- 本机 server endpoint:{} · 自动启动:{}\n",
        s.local_server_endpoint.as_deref().unwrap_or("(默认 :8899)"),
        if s.local_server_auto_start {
            "是"
        } else {
            "否"
        },
    ));
    md.push('\n');

    // 2026-05-26 V0.1.11:系统级
    let si = &info.system_info;
    md.push_str("## 系统级诊断\n\n");
    md.push_str(&format!("- 数据目录:`{}`\n", si.data_dir));
    md.push_str(&format!(
        "- 数据目录可写:{}\n",
        if si.data_dir_writable {
            "是"
        } else {
            "**否(权限有问题!)**"
        }
    ));
    if let Some(s) = si.db_size_mb {
        md.push_str(&format!("- 数据库大小:{:.2} MB\n", s));
    } else {
        md.push_str("- 数据库:**未找到**\n");
    }
    if let (Some(c), Some(s)) = (si.extracts_files, si.extracts_size_mb) {
        md.push_str(&format!("- extracts/ 目录:{} 文件 / {:.2} MB\n", c, s));
    }
    if let Some(s) = si.reports_size_mb {
        md.push_str(&format!("- reports/ + external/:{:.2} MB\n", s));
    }
    if let Some(g) = si.disk_free_gb {
        md.push_str(&format!("- 数据盘剩余:{:.2} GB\n", g));
    }
    md.push_str(&format!(
        "- pdftotext 可用:{} · pdftoppm 可用:{}\n",
        if si.pdftotext_available { "是" } else { "否" },
        if si.pdftoppm_available { "是" } else { "否" },
    ));
    md.push('\n');

    // 2026-06-28:本地 KB / 记忆目录诊断。排查 Windows Known Folder、只读目录、
    // `~/Documents/知识库` 未展开等问题时,这段比单纯错误弹窗更有用。
    {
        let km = &info.kb_memory;
        md.push_str("## 本地 KB / 记忆诊断\n\n");
        md.push_str(&format!(
            "- KB 开关:{}\n",
            match km.local_kb_enabled {
                Some(true) => "启用",
                Some(false) => "关闭",
                None => "未设置",
            }
        ));
        md.push_str(&format!(
            "- KB 状态:{}\n",
            match km.local_kb_state.as_str() {
                "bound" => "已绑定",
                "permission_denied" => "**无权限/不可读写**",
                "unbound" => "未绑定或路径不存在",
                other => other,
            }
        ));
        md.push_str(&format!(
            "- KB 配置路径:`{}`\n",
            km.local_kb_root_configured
                .as_deref()
                .unwrap_or("(未设置,使用默认 Documents/知识库)")
        ));
        md.push_str(&format!(
            "- KB 解析路径:`{}`\n",
            km.local_kb_root_resolved.as_deref().unwrap_or("(无法解析)")
        ));
        md.push_str(&format!(
            "- 记忆目录:`{}`\n",
            km.memory_root_resolved.as_deref().unwrap_or("(无法解析)")
        ));
        md.push_str(&format!(
            "- 记忆目录存在:{} · 可写:{}\n",
            if km.memory_root_exists { "是" } else { "否" },
            if km.memory_root_writable {
                "是"
            } else {
                "**否**"
            },
        ));
        match km.memory_notes_count {
            Some(n) => md.push_str(&format!("- 记忆文件数:{}\n", n)),
            None => md.push_str("- 记忆文件数:(未统计)\n"),
        }
        if let Some(err) = &km.memory_error {
            let safe = sanitize_runtime_log_entry(err).replace('`', "'");
            md.push_str(&format!("- 记忆检测错误:`{}`\n", safe));
        }
        md.push('\n');
    }

    render_pi_runtime_diagnosis(&mut md, &info.runtime_traces);

    // 2026-05-26 V0.1.11:App 自身 stderr ring buffer
    if !info.stderr_tail.is_empty() {
        md.push_str(&format!(
            "## App 运行时日志(最近 {} 行,新→旧)\n\n```\n",
            info.stderr_tail.len()
        ));
        for line in &info.stderr_tail {
            // stderr_tail 属于非结构化诊断输入。即使调用方漏做了写入侧清理，
            // 反馈边界也只允许单行、有界摘要，不能携带模型原始输出或案件正文。
            md.push_str(&sanitize_runtime_log_entry(line));
            md.push('\n');
        }
        md.push_str("```\n\n");
    }

    // 2026-05-26 V0.1.12:抽取性能 A/B 对比(按 backend 聚合)
    if !info.metrics_summary.is_empty() {
        md.push_str("## 抽取性能样本(本地 vs 云端 A/B)\n\n");
        md.push_str("| Stage | Backend | 样本 | 成功 | avg ms | p50 ms | avg chars |\n");
        md.push_str("|:---|:---|---:|---:|---:|---:|---:|\n");
        for s in &info.metrics_summary {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                s.stage,
                s.backend,
                s.samples,
                s.ok_samples,
                s.avg_ms,
                s.p50_ms,
                s.avg_chars
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "—".into()),
            ));
        }
        md.push('\n');
    }

    // 最近 20 条原始样本(给作者复盘个例用)
    if !info.metrics_tail.is_empty() {
        let preview_n = info.metrics_tail.len().min(20);
        md.push_str(&format!(
            "## 最近抽取样本(显示前 {} 条,共 {} 条入库)\n\n",
            preview_n,
            info.metrics_tail.len()
        ));
        md.push_str("| 时间 | 文件 | 大小 | Stage | Backend | 结果 | 耗时 | 字数 |\n");
        md.push_str("|:---|:---|---:|:---|:---|:---|---:|---:|\n");
        for s in info.metrics_tail.iter().take(preview_n) {
            let size = if s.file_size_bytes >= 1024 * 1024 {
                format!("{:.1}M", s.file_size_bytes as f64 / 1024.0 / 1024.0)
            } else {
                format!("{}K", s.file_size_bytes / 1024)
            };
            let chars = s
                .text_chars
                .map(|c| c.to_string())
                .unwrap_or_else(|| "—".into());
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} ms | {} |\n",
                s.created_at, s.filename, size, s.stage, s.backend, s.outcome, s.elapsed_ms, chars,
            ));
        }
        md.push('\n');
    }

    // 2026-05-27 V0.1.13+:案件 AI 助手用量(按 model + task_type 聚合)
    // **隐私**:本段只渲染数字 + model + task_type,**绝不**渲染 chat content
    if !info.chat_usage.is_empty() {
        md.push_str("## 案件 AI 助手用量(近 200 条 assistant 消息)\n\n");
        md.push_str("| Model | Task | 样本 | 成功 | avg prompt | avg completion | avg latency | avg chars |\n");
        md.push_str("|:---|:---|---:|---:|---:|---:|---:|---:|\n");
        for b in &info.chat_usage {
            md.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} ms | {} |\n",
                b.model,
                b.task_type,
                b.samples,
                b.ok_samples,
                b.avg_prompt,
                b.avg_completion,
                b.avg_latency_ms,
                b.avg_chars,
            ));
        }
        md.push('\n');
    }

    // 2026-05-26 V0.1.11:前端 console 报错
    if !info.console_errors.is_empty() {
        md.push_str(&format!(
            "## 前端 console / window.onerror({} 条)\n\n",
            info.console_errors.len()
        ));
        for e in &info.console_errors {
            let at = e.at.as_deref().unwrap_or("?");
            md.push_str(&format!("- **[{}]** {}\n", e.level, at));
            md.push_str("  > ");
            md.push_str(&sanitize_runtime_log_entry(&e.message));
            md.push('\n');
        }
        md.push('\n');
    }

    md.push_str("## 用户描述\n\n");
    if user_description.trim().is_empty() {
        md.push_str("(用户未填写)\n");
    } else {
        md.push_str(user_description.trim());
        md.push('\n');
    }
    md.push('\n');

    md.push_str("---\n\n");
    md.push_str("> 此反馈由 CaseBoard 自动生成,**不包含案件名、当事人、文档内容**。\n");
    md.push_str(
        "> 反馈方 ID 是匿名 UUID 前 8 位,跟用户名/邮箱无关,只用于关联「同一人多次反馈」。\n",
    );
    md.push_str("> 把这个 MD 文件发给项目维护者即可,无需粘贴文字。\n");

    // 安全网:整份 MD 再过一次 sanitize_paths,兜底 stderr/console 里可能漏的路径
    sanitize_paths(&md)
}

fn render_pi_runtime_diagnosis(
    md: &mut String,
    traces: &[crate::chat::diagnostics::RuntimeTraceEvent],
) {
    use crate::chat::diagnostics::{
        FailureOrigin, RuntimeRequestFinished, RuntimeRequestStarted, RuntimeTraceEvent,
    };

    if traces.is_empty() {
        return;
    }
    let mut request_ids = Vec::new();
    for event in traces {
        if !request_ids
            .iter()
            .any(|request_id| *request_id == event.request_id())
        {
            request_ids.push(event.request_id());
        }
    }
    if request_ids.is_empty() {
        return;
    }

    md.push_str("## Pi Runtime 诊断\n\n");
    md.push_str("| 时间 | Provider | 模型 | 错误来源 | 分类 / HTTP | 重试 | 耗时 | 终止原因 |\n");
    md.push_str("|:---|:---|:---|:---|:---|---:|---:|:---|\n");
    for request_id in &request_ids {
        let started = traces.iter().find_map(|event| match event {
            RuntimeTraceEvent::RequestStarted(started)
                if started.common.request_id == **request_id =>
            {
                Some(started)
            }
            _ => None,
        });
        let finished = traces.iter().find_map(|event| match event {
            RuntimeTraceEvent::RequestFinished(finished)
                if finished.common.request_id == **request_id =>
            {
                Some(finished)
            }
            _ => None,
        });
        let timestamp = finished
            .map(|event| event.common.ts.as_str())
            .or_else(|| started.map(|event| event.common.ts.as_str()))
            .unwrap_or("—");
        let provider = finished
            .and_then(|event| event.provider_id.as_deref())
            .or_else(|| started.map(|event| event.provider_id.as_str()))
            .unwrap_or("—");
        let model = finished
            .and_then(|event| event.model_id.as_deref())
            .or_else(|| started.map(|event| event.model_id.as_str()))
            .unwrap_or("—");
        let origin = finished
            .and_then(|event| event.failure_origin)
            .map(failure_origin_display)
            .unwrap_or("—");
        let category = finished
            .and_then(|event| event.error_category)
            .map(|category| category.as_str())
            .unwrap_or("—");
        let category_http = match finished.and_then(|event| event.http_status) {
            Some(status) => format!("{category} / HTTP {status}"),
            None => category.to_string(),
        };
        let retry_count = finished.map_or(0, |event| event.retry_count);
        let elapsed = finished
            .map(|event| format!("{:.2} s", event.elapsed_ms as f64 / 1000.0))
            .unwrap_or_else(|| "—".into());
        let stop_reason = finished
            .and_then(|event| event.stop_reason.as_deref())
            .unwrap_or("未记录");
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} 次 | {} | {} |\n",
            timestamp, provider, model, origin, category_http, retry_count, elapsed, stop_reason
        ));
    }
    md.push('\n');

    for request_id in request_ids {
        let started: Option<&RuntimeRequestStarted> = traces.iter().find_map(|event| match event {
            RuntimeTraceEvent::RequestStarted(started)
                if started.common.request_id == request_id =>
            {
                Some(started)
            }
            _ => None,
        });
        let finished: Option<&RuntimeRequestFinished> =
            traces.iter().find_map(|event| match event {
                RuntimeTraceEvent::RequestFinished(finished)
                    if finished.common.request_id == request_id =>
                {
                    Some(finished)
                }
                _ => None,
            });
        let provider = finished
            .and_then(|event| event.provider_id.as_deref())
            .or_else(|| started.map(|event| event.provider_id.as_str()))
            .unwrap_or("—");
        let model = finished
            .and_then(|event| event.model_id.as_deref())
            .or_else(|| started.map(|event| event.model_id.as_str()))
            .unwrap_or("—");
        md.push_str(&format!("### {} / {}\n\n", provider, model));
        md.push_str(&format!("- 请求 ID:`{}`\n", request_id));
        if let Some(started) = started {
            md.push_str(&format!(
                "- 凭据:{} · 来源:{}\n",
                started.credential_type.as_deref().unwrap_or("未记录"),
                started.credential_source.as_deref().unwrap_or("未记录")
            ));
            md.push_str(&format!(
                "- Runtime:{} · Pi SDK:{} · 协议:v{} · 平台:{}/{}\n",
                started.sidecar_version.as_deref().unwrap_or("未记录"),
                started.pi_sdk_version.as_deref().unwrap_or("未记录"),
                started.protocol_version,
                started.platform,
                started.arch
            ));
            md.push_str(&format!(
                "- 请求规模:system {} 字 · 历史 {} 条/{} 字 · 用户 {} 字 · 工具 {} · Skill {} · max tokens {}\n",
                started.system_prompt_chars,
                started.history_messages,
                started.history_chars,
                started.user_message_chars,
                started.tool_count,
                started.skill_count,
                started.max_tokens
            ));
        }
        if let Some(finished) = finished {
            md.push_str(&format!(
                "- 运行统计:轮次 {} · 工具 {}（失败 {}）· 重试 {} · 正文 {} 字 · 推理 {} 字\n",
                finished.turns,
                finished.tool_calls,
                finished.tool_failures,
                finished.retry_count,
                finished.text_chars,
                finished.reasoning_chars
            ));
            if finished.prompt_tokens.is_some() || finished.completion_tokens.is_some() {
                md.push_str(&format!(
                    "- Token:输入 {} · 输出 {} · cache read {} · cache write {} · 总计 {}\n",
                    display_optional_u64(finished.prompt_tokens),
                    display_optional_u64(finished.completion_tokens),
                    display_optional_u64(finished.cache_read_tokens),
                    display_optional_u64(finished.cache_write_tokens),
                    display_optional_u64(finished.total_tokens)
                ));
            }
            if let Some(error) = &finished.error_message {
                md.push_str(&format!(
                    "- 最终错误:`{}`\n",
                    redact_feedback_runtime_secret(error).replace('`', "'")
                ));
            }
            if !finished.stderr_tail.is_empty() {
                md.push_str("- 异常 stderr 尾部:\n\n```text\n");
                for line in &finished.stderr_tail {
                    md.push_str(&redact_feedback_runtime_secret(line));
                    md.push('\n');
                }
                md.push_str("```\n");
            }
        }
        md.push('\n');
    }

    fn failure_origin_display(origin: FailureOrigin) -> &'static str {
        match origin {
            FailureOrigin::Provider => "大模型供应商",
            FailureOrigin::Network => "网络",
            FailureOrigin::PiRuntime => "Pi Runtime",
            FailureOrigin::Protocol => "协议",
            FailureOrigin::HostTool => "CaseBoard 工具",
            FailureOrigin::Cancelled => "用户取消",
            FailureOrigin::Unknown => "无法确定",
        }
    }
}

fn display_optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| "—".into(), |value| value.to_string())
}

fn redact_feedback_runtime_secret(value: &str) -> String {
    static BEARER: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static SK_TOKEN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static NAMED_SECRET: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let safe = BEARER
        .get_or_init(|| regex::Regex::new(r"(?i)bearer\s+[^\s,;]+").unwrap())
        .replace_all(value, "Bearer [REDACTED]");
    let safe = SK_TOKEN
        .get_or_init(|| regex::Regex::new(r"(?i)\bsk-[a-z0-9._-]+").unwrap())
        .replace_all(&safe, "[REDACTED]");
    NAMED_SECRET
        .get_or_init(|| {
            regex::Regex::new(r"(?i)(authorization|cookie|api[_ -]?key|token)\s*[:=]\s*[^\s,;]+")
                .unwrap()
        })
        .replace_all(&safe, "$1: [REDACTED]")
        .chars()
        .take(2_000)
        .collect()
}

/// 反馈 MD 里 key 状态的统一文案。三态:
/// - metadata `valid` → "已填 · 已验证 ✓"
/// - metadata 存在且 secret_present → "**已填 · ⚠ 未通过验证(可能无效 key)**"
/// - metadata 缺失或已无 secret → "未填"
fn key_state_display(status: Option<&CredentialStatusSnapshot>) -> String {
    match status {
        Some(status)
            if status.secret_present
                && status.state == crate::credentials_bridge::BridgeCredentialState::Valid =>
        {
            "已填 · 已验证 ✓".to_string()
        }
        Some(status) if status.secret_present => {
            "**已填 · ⚠ 未通过验证(可能无效 key)**".to_string()
        }
        _ => "未填".to_string(),
    }
}

fn provider_display(p: &str) -> &str {
    match p {
        "local" => "本机 MiniCPM-V",
        "cloud" => "云端(DeepSeek / MinerU)",
        _ => p,
    }
}

/// 把错误信息里的绝对路径替换成 `<path>/<basename>`,防止案件路径(常含当事人名)泄漏。
///
/// 例:`<home>/cases/张三/foo.pdf` → `<path>/foo.pdf`
///
/// 只匹配 macOS 常见的根前缀。
///
/// **已知限制**:**含空格的路径**(如 `<home>/Nutstore Files/y/z.pdf`)只能正确处理
/// **引号包围**的版本(`"..."` / `'...'` / `` `...` ``).无引号 unquoted 路径会在
/// 第一个空格切断,留下后段(如 `Files/张三/z.pdf` 仍含敏感名).MinerU CLI 通常会
/// 引号包围 path,std::io::Error::Display 不含 path,所以这个简化可接受;但**绝不要**
/// 把不可信用户输入喂进 sanitize_paths——它只用于工具 stderr 等受控来源.
pub(crate) fn sanitize_paths(s: &str) -> String {
    const PREFIXES: &[&str] = &["/Users/", "/Volumes/", "/private/", "/tmp/", "/var/"];
    // unquoted 模式下的路径结束字符(含空格)
    const UNQUOTED_END: &[char] = &[
        ' ', '\t', '\n', '\r', ',', ';', '"', '\'', ')', ']', '}', '<', '>', '`',
    ];
    fn is_quote(c: char) -> bool {
        matches!(c, '"' | '\'' | '`')
    }

    let mut out = String::with_capacity(s.len());
    let len = s.len();
    let mut i = 0;

    while i < len {
        // 找第一个匹配的前缀
        let mut matched_prefix: Option<usize> = None;
        for &prefix in PREFIXES {
            if s[i..].starts_with(prefix) {
                matched_prefix = Some(prefix.len());
                break;
            }
        }

        if let Some(prefix_len) = matched_prefix {
            // 看 path 前一个字符是不是引号(quoted 模式下,空格不算结束)
            let is_quoted = s[..i].chars().last().map(is_quote).unwrap_or(false);

            let path_start = i;
            let mut j = i + prefix_len;
            while j < len {
                let ch = s[j..].chars().next().unwrap();
                let is_end = if is_quoted {
                    // quoted:只在引号 / 换行处结束
                    is_quote(ch) || ch == '\n' || ch == '\r'
                } else if ch == ' ' || ch == '\t' {
                    // unquoted 空白:**heuristic** — 只在"下一个 token 末尾紧跟 `/`"时
                    // 视作路径段名(覆盖 "Application Support" / "Nutstore Files")。
                    // 不能用"同行任何位置还有 /"——会把空格分隔的两条独立路径错误合并。
                    // 2026-05-26 V0.1.11:修 `<home>/Application Support/external/张三/foo.md`
                    // 之前在第一个空格断,"张三" 漏出来。
                    let rest = &s[j + 1..];
                    // 找下一个 token 边界:`/` 视为分隔(因为我们要检测 "Support/" 这种形状)
                    let next_break = rest
                        .find([' ', '\t', '\n', '\r', '/'])
                        .unwrap_or(rest.len());
                    let next_token = &rest[..next_break];
                    let next_continues = !next_token.is_empty()
                        && next_break < rest.len()
                        && rest.as_bytes()[next_break] == b'/';
                    !next_continues
                } else {
                    // unquoted 其他标点:任何 UNQUOTED_END 都终止
                    ch.is_ascii() && UNQUOTED_END.contains(&ch)
                };
                if is_end {
                    break;
                }
                j += ch.len_utf8();
            }
            let path = &s[path_start..j];
            // basename:最后一个 `/` 之后(可能为空)
            let basename = path.rsplit_once('/').map(|(_, b)| b).unwrap_or("");
            out.push_str("<path>");
            if !basename.is_empty() {
                out.push('/');
                out.push_str(basename);
            }
            i = j;
        } else {
            // 不在前缀位置,推一个字符
            let ch = s[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}
/// 把非结构化运行时日志压成可进入本机日志和在线反馈的单行摘要。
///
/// LLM JSON 解析错误可能把 `---原始---` 后的完整模型输出拼进错误文本；该输出通常
/// 含案件正文和个人信息。这里只保留第一行错误类型/位置，并对超长单行设置硬上限。
pub(crate) fn sanitize_runtime_log_entry(value: &str) -> String {
    const MAX_SUMMARY_CHARS: usize = 1_000;

    let first_line_end = value.find(['\r', '\n']).unwrap_or(value.len());
    let first_line = &value[..first_line_end];
    let raw_marker_end = first_line.find("---原始---").unwrap_or(first_line.len());
    let summary = first_line[..raw_marker_end].trim_end().replace('\t', " ");
    let summary_chars = summary.chars().count();
    let mut safe = sanitize_paths(&summary.chars().take(MAX_SUMMARY_CHARS).collect::<String>());

    let removed_details = first_line_end < value.len() || raw_marker_end < first_line.len();
    if removed_details {
        safe.push_str(" [多行详情已移除]");
    } else if summary_chars > MAX_SUMMARY_CHARS {
        safe.push_str(" [超长详情已截断]");
    }
    safe
}
