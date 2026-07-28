//! OCR 后端模块。
//!
//! 2026-05-23 作者隐私分流决策(详见 docs/产品决策与理念.md 第 2 节):
//! - **默认纯本地** —— 律师对数据敏感度高,默认不上云
//! - **用户明确开启 `cloud_enabled` 后**,才走 MinerU 在线 OCR
//! - 纯本地模式下走**本机 MiniCPM-V vision**(:8899 多模态 chat completions)
//!
//! 2026-05-25 V0.1.8 作者拍板:云端模式**全走精准 API**,不再用 flash-extract。
//! - 用户既然填了 token(Settings 里 mineru_api_key 必填),就让 token 发挥作用
//! - 精准 API 免费额度 1000 份/天,文件上限 200MB / 600 页(flash 只有 10MB / 20 页)
//! - 精准 API 限流比 flash 宽松,配合 pipeline 三轮动态降级足够稳
//!
//! 后端选择:
//! 1. cloud_enabled=true → MinerU `extract` 精准模式(带 --token)
//! 2. cloud_enabled=false → 本机 MiniCPM-V vision(PDF 先 pdftoppm 转 PNG,然后喂图)
//!
//! 性能(R&D 实测,M3 Max):
//! - MinerU precision:~15-30 秒/份(取决于页数 + 服务端排队)
//! - 本机 MiniCPM-V vision:13-15 秒/份(内部基准测试)

use std::path::{Path, PathBuf};

use crate::credentials_bridge::{BridgeCredentialConsumer, PendingCredentialSource};

const MB: u64 = 1024 * 1024;

/// MinerU 精准 API 的文件大小上限(200MB,API 硬上限)。
const PRECISION_MAX_BYTES: u64 = 200 * MB;

/// AI Studio 异步 OCR API 的本地 multipart 上传上限。
///
/// 官方文档当前口径:PDF 最多 1000 页;URL 文件最大 200MB;本地上传最大 50MB。
/// CaseBoard 现有 PaddleOCR-VL / PP-OCRv6 客户端都走本地 multipart 上传,所以先按 50MB
/// 预检,避免大文件盲提交后失败。后续若接入 URL 模式,再按 URL 能力单独放宽。
const AISTUDIO_LOCAL_UPLOAD_MAX_BYTES: u64 = 50 * MB;

/// MinerU 精准 API 单次调用超时(秒)
///
/// CLI 默认 single=300s,但大 PDF(14MB 开庭笔录之类)+ 服务端排队偶尔很慢,
/// 给 900s 安全。三轮重试 worst case 单文件总耗时 = 2700s,可接受。
const MINERU_PRECISION_TIMEOUT_SEC: u64 = 900;

/// 有备用云端 OCR 时,**主力**的轮询超时收紧到 300s(2026-06-12)。
///
/// 实测正常处理 15-30 秒/份;300s 还没出结果基本是服务端排队严重,
/// 与其干等 900s 不如尽早切备用。备用(最后一棒)仍给足 900s。
const PRIMARY_TIMEOUT_WITH_FALLBACK_SEC: u64 = 300;

/// PaddleOCR VL-1.6 单次调用超时(秒),作为最后一棒时使用(同 MinerU 语义)。
const PADDLE_VL_TIMEOUT_SEC: u64 = 900;

/// MinerU 精准模型版本选择(官网推荐)
///
/// - `vlm` — 官方文档对复杂文档(扫描件/手写/表格)的推荐
/// - `pipeline` — 默认管线,兼容性好但 vlm 通常更准
/// - `auto` — CLI 默认,但官网文档明确说 vlm 对复杂文档更好
///
/// 律师扫描件 + 复杂表格场景多 → vlm 更稳。
const MINERU_MODEL: &str = "vlm";

/// 本机 vision 后端 endpoint(llama-server :8899 多模态)
const LOCAL_VISION_ENDPOINT: &str = "http://127.0.0.1:8899/v1/chat/completions";
/// 本机 vision 模型(作者 M3 Max 实测可跑)
const LOCAL_VISION_MODEL: &str = "MiniCPM-V-4_6-Q8_0.gguf";

/// 本机 vision OCR 的 prompt(2026-05-23 实测验证,关键字段 100% 命中)
const LOCAL_VISION_OCR_PROMPT: &str = "这是一份诉讼文书的扫描件。请把图片上所有的中文文字按原文顺序识别出来,输出纯文本。\n\n要求:\n1. 保持原文换行和段落结构\n2. 不要总结、不要解释、不要补充\n3. 不要添加 Markdown 格式\n4. 数字、案号、日期、姓名要逐字精确识别\n5. 如果某处看不清,用 [模糊] 标记,不要瞎猜\n\n直接输出识别的文字内容,不要任何额外说明。";

/// PDF 转 PNG 的 DPI(150 是质量和大小的平衡点)
const PDF_TO_PNG_DPI: u32 = 150;

/// 本机 vision OCR 单次推理超时(秒)
const LOCAL_VISION_TIMEOUT_SEC: u64 = 180;

/// OCR 抽取结果
///
/// 字段允许 unused — 它们是为前端 / 日志保留的诊断信息,
/// V0.1 的 caller 只 match 顶层 variant,但调试时这些字段不可或缺。
#[derive(Debug)]
#[allow(dead_code)]
pub enum OcrResult {
    Ok {
        text: String,
        backend: &'static str,
        elapsed_ms: u128,
    },
    Skipped {
        reason: String,
    },
    Failed {
        error: String,
        attempted: Vec<&'static str>,
    },
}

/// 云端 OCR 轮询过程中的实时状态(2026-06-14:治"看着卡死")。
///
/// 大图扫描件(如微信长截图)云端排队 / 处理可达数分钟,期间进度条停在某文件不动。
/// 两家云端后端轮询本来就拿到 state,这里把它透传到前端,显示"排队中 / 识别中(已 N 秒)"。
#[derive(Debug, Clone)]
pub struct OcrPollUpdate {
    /// 归一化后端状态:`"queued"`(排队)/ `"processing"`(识别中)/ `"converting"`(转换中)。
    pub phase: String,
    /// 从提交任务到本次轮询已等待的秒数。
    pub elapsed_secs: u64,
    /// 已解析页数(仅 PaddleOCR extractProgress 提供;无则 None)。
    pub pages_done: Option<i64>,
    /// 总页数(同上)。
    pub pages_total: Option<i64>,
}

/// OCR 调用上下文 —— 从 Settings 里来,决定走云端还是本地。
///
/// 显式参数化避免 ocr.rs 直接依赖 settings 模块,保持职责单一。
#[derive(Debug, Clone, Default)]
pub struct OcrContext {
    /// 用户是否明确允许调用云端 API(MinerU / PaddleOCR)
    ///
    /// - `true` → 走云端 OCR(需至少一个 token)
    /// - `false` → 走本机 MiniCPM-V vision(慢一点,但 0 上传)
    pub cloud_enabled: bool,
    /// MinerU typed credential locator，不含 secret。
    pub mineru_credential: Option<PendingCredentialSource>,
    /// PaddleOCR/PP-OCRv6 typed credential locator，不含 secret。
    pub paddle_vl_credential: Option<PendingCredentialSource>,
    /// 云端 OCR 主力:`"mineru"`(默认)/ `"paddle-vl"`(Settings.effective_ocr_cloud_primary)。
    /// 主力失败 / 超时 / 额度用完时,若另一家 token 已填则自动切换(动态主备)。
    pub cloud_primary: String,
    /// 2026-06-13:文档级强制后端(目前只有 `"ppocrv6"` 去水印)。
    /// Some 时**只走该后端、不回退**(用户已明确选去水印,回退到 VL 会把水印垃圾又喂回来)。
    /// 由 process_one_doc 从 `documents.ocr_backend_override` 注入。
    pub force_backend: Option<String>,
    /// 2026-06-14:云端 OCR 轮询进度回传通道(单文档级,由 process_one_doc 注入)。
    /// 轮询循环每拍 `send` 一次 [`OcrPollUpdate`],前端据此显示"排队 / 识别中(已 N 秒)"。
    /// `None` = 不上报(测试 / 本地模式 / 不关心进度的调用)。`#[derive(Default)]` 下默认 None。
    pub poll_tx: Option<tokio::sync::mpsc::UnboundedSender<OcrPollUpdate>>,
}

impl OcrContext {
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let cloud = settings.effective_ocr_provider() == "cloud";
        Self {
            cloud_enabled: cloud,
            mineru_credential: cloud.then(|| {
                PendingCredentialSource::pending(
                    BridgeCredentialConsumer::OcrProvider,
                    "settings:mineru_api_key",
                    "ocr.mineru",
                )
            }),
            paddle_vl_credential: cloud.then(|| {
                PendingCredentialSource::pending(
                    BridgeCredentialConsumer::OcrProvider,
                    "settings:paddle_vl_api_key",
                    "ocr.paddle_vl",
                )
            }),
            cloud_primary: settings.effective_ocr_cloud_primary().to_string(),
            force_backend: None,
            poll_tx: None,
        }
    }
}

/// 把本地文件转换成流式 HTTP body。旧实现会先 `std::fs::read` 整份文件；8 路并发遇到
/// 100~200MB PDF 时可能瞬间占用 0.8~1.6GB 内存。流式读取把每个上传的常驻缓冲压到 64KB。
pub(crate) async fn streaming_file_body(path: &Path) -> Result<(reqwest::Body, u64), String> {
    use tokio::io::AsyncReadExt;

    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("打开待上传文件失败: {}", error))?;
    let length = file
        .metadata()
        .await
        .map_err(|error| format!("读取待上传文件信息失败: {}", error))?
        .len();
    let stream = futures::stream::try_unfold(file, |mut file| async move {
        let mut chunk = vec![0u8; 64 * 1024];
        let read = file.read(&mut chunk).await?;
        if read == 0 {
            Ok::<_, std::io::Error>(None)
        } else {
            chunk.truncate(read);
            Ok(Some((chunk, file)))
        }
    });
    Ok((reqwest::Body::wrap_stream(stream), length))
}

/// 走 MinerU 精准 HTTP API(2026-05-25 V0.1.10 替代 CLI 子进程)。
///
/// 历史:V0.1.8 用 `mineru-open-api extract` CLI 子进程,但发现 CLI 是 npm 安装的
/// Node.js 脚本,无法打包进 dmg → 朋友的 Mac 没装 npm 包就报"mineru-open-api 未安装"。
/// V0.1.10 改用纯 Rust 调 HTTP API(`crate::ingest::mineru_http`),零外部依赖。
async fn run_mineru_precision(
    path: &Path,
    token: &str,
    timeout_secs: u64,
    poll_tx: Option<&tokio::sync::mpsc::UnboundedSender<OcrPollUpdate>>,
) -> Result<String, String> {
    crate::ingest::mineru_http::extract_with_mineru_http(
        path,
        token,
        MINERU_MODEL,
        timeout_secs,
        poll_tx,
    )
    .await
}

/// OCR 主入口:按 ctx.cloud_enabled 分流。
///
/// - `cloud_enabled = true` → 云端 OCR / 文档解析(需 token;按后端能力预检)
/// - `cloud_enabled = false` → 本机 MiniCPM-V vision(慢一点,0 上传)
///
/// 2026-05-23 作者隐私分流决策。代码层强制:`cloud_enabled = false` 时
/// **绝不调用任何云端 API**。
/// 2026-05-25 V0.1.10:改成 async 函数。
/// 原因:MinerU 改用 reqwest HTTP 客户端(async),不再 spawn_blocking CLI 子进程。
/// 本机 vision 那条仍 sync(ureq),在内部用 spawn_blocking 包起来。
pub async fn extract_with_ocr(path: &Path, ctx: &OcrContext) -> OcrResult {
    let started = std::time::Instant::now();

    // 1. 文件存在性检查
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            return OcrResult::Failed {
                error: format!("无法读取文件元数据: {}", e),
                attempted: vec![],
            }
        }
    };

    // ===== 文档级强制后端(去水印,2026-06-13):只走该后端、不回退 =====
    // 用户对带水印的工商调档件点了「去水印重识别」→ 强制 PP-OCRv6 + 去水印。
    // 失败直接透传(回退到 VL 会把水印垃圾又喂回来,正是要逃离的)。
    if let Some(fb) = ctx.force_backend.as_deref() {
        if fb == "ppocrv6" {
            // PP-OCRv6 走 AIStudio,复用 PaddleOCR 的 token(同平台同账号)。
            let Some(source) = ctx.paddle_vl_credential.as_ref() else {
                return OcrResult::Failed {
                    error: "credential_missing: settings:paddle_vl_api_key".into(),
                    attempted: vec!["ppocrv6"],
                };
            };
            if let Err(e) = preflight_cloud_backend_local_file("ppocrv6", meta.len()) {
                return OcrResult::Failed {
                    error: e,
                    attempted: vec!["ppocrv6"],
                };
            }
            let material = match source.issue_material().await {
                Ok(material) => material,
                Err(error) => {
                    return OcrResult::Failed {
                        error,
                        attempted: vec!["ppocrv6"],
                    }
                }
            };
            return match crate::ingest::ppocrv6_http::extract_with_ppocrv6(
                path,
                material.expose(),
                PADDLE_VL_TIMEOUT_SEC,
                ctx.poll_tx.as_ref(),
            )
            .await
            {
                Ok(text) => OcrResult::Ok {
                    text: crate::ingest::sanitize::sanitize_ocr_markup(&text),
                    backend: "ppocrv6",
                    elapsed_ms: started.elapsed().as_millis(),
                },
                Err(e) => OcrResult::Failed {
                    error: format!("ppocrv6: {}", material.redact(&e)),
                    attempted: vec!["ppocrv6"],
                },
            };
        }
    }

    if ctx.cloud_enabled {
        // ===== 云端模式:主/备动态切换(2026-06-12) =====
        // 顺序 = 主力在前 + 备用(token 已填才上场)在后;主力失败/超时/额度用完自动切备用。
        if meta.len() > PRECISION_MAX_BYTES {
            return OcrResult::Skipped {
                reason: format!(
                    "文件 {:.1}MB 超过云端 OCR 上限 200MB",
                    meta.len() as f64 / 1024.0 / 1024.0
                ),
            };
        }
        // (backend 名, non-secret credential locator)。每个真实后端尝试独立签发 lease。
        let mineru_entry = ctx
            .mineru_credential
            .as_ref()
            .map(|source| ("mineru-precision", source));
        let paddle_entry = ctx
            .paddle_vl_credential
            .as_ref()
            .map(|source| ("paddle-vl", source));

        // 2026-06-16:office 文档(doc/rtf/odt/ppt/xls 等)**只有 MinerU 能解析,Paddle 不支持**。
        // 这类文件强制只走 MinerU(跳过 Paddle,别浪费一次必失败的调用);只配了 Paddle 没配
        // MinerU 时给明确引导报错,不静默跳过(守已知坑#8 透传真错)。
        let is_office = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| crate::ingest::extractor::is_office_cloud_ext(&n.to_lowercase()))
            .unwrap_or(false);
        let order: Vec<(&'static str, &PendingCredentialSource)> = if is_office {
            match mineru_entry {
                Some(e) => vec![e],
                None => {
                    return OcrResult::Failed {
                        error: "该 Office 文档(.doc/.rtf/.odt/.ppt/.xls 等)需 MinerU 云端解析,\
                                当前只配了 PaddleOCR(不支持 Office 文档)。\
                                请在「设置 → 功能模型」申请并填入 MinerU OCR API key 后重试。"
                            .into(),
                        attempted: vec![],
                    };
                }
            }
        } else if ctx.cloud_primary == "paddle-vl" {
            [paddle_entry, mineru_entry].into_iter().flatten().collect()
        } else {
            [mineru_entry, paddle_entry].into_iter().flatten().collect()
        };
        if order.is_empty() {
            return OcrResult::Failed {
                error: "云端模式缺 OCR token(请在设置里填 MinerU 或 PaddleOCR key)".into(),
                attempted: vec![],
            };
        }

        let total = order.len();
        let mut attempted: Vec<&'static str> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        for (i, (backend, source)) in order.into_iter().enumerate() {
            // 有备用时主力收紧到 300s(排队严重早点切);最后一棒给足 900s
            let timeout = if i + 1 < total {
                PRIMARY_TIMEOUT_WITH_FALLBACK_SEC
            } else if backend == "paddle-vl" {
                PADDLE_VL_TIMEOUT_SEC
            } else {
                MINERU_PRECISION_TIMEOUT_SEC
            };
            attempted.push(backend);
            if let Err(e) = preflight_cloud_backend_local_file(backend, meta.len()) {
                if i + 1 < total {
                    crate::dlog!("[ocr] 后端 {} 本地文件预检不通过,切换备用: {}", backend, e);
                }
                errors.push(format!("{}: {}", backend, e));
                continue;
            }
            let material = match source.issue_material().await {
                Ok(material) => material,
                Err(error) => {
                    if i + 1 < total {
                        crate::dlog!("[ocr] 后端 {} 凭据不可用，切换备用", backend);
                    }
                    errors.push(format!("{backend}: {error}"));
                    continue;
                }
            };
            let result = match backend {
                "paddle-vl" => {
                    crate::ingest::paddle_vl_http::extract_with_paddle_vl(
                        path,
                        material.expose(),
                        timeout,
                        ctx.poll_tx.as_ref(),
                    )
                    .await
                }
                _ => {
                    run_mineru_precision(path, material.expose(), timeout, ctx.poll_tx.as_ref())
                        .await
                }
            };
            match result {
                Ok(text) => {
                    return OcrResult::Ok {
                        text: crate::ingest::sanitize::sanitize_ocr_markup(&text),
                        backend,
                        elapsed_ms: started.elapsed().as_millis(),
                    }
                }
                Err(e) => {
                    let error = material.redact(&e);
                    if i + 1 < total {
                        crate::dlog!("[ocr] 主力 {} 失败,自动切换备用: {}", backend, error);
                    }
                    errors.push(format!("{}: {}", backend, error));
                }
            }
        }
        OcrResult::Failed {
            error: errors.join(" ;且备用也失败→ "),
            attempted,
        }
    } else {
        // ===== 纯本地模式:走本机 MiniCPM-V vision(sync,用 spawn_blocking 包) =====
        // 代码层强制不上云 —— 用户没勾 cloud_enabled,这里**绝对不调云端**
        let attempted = vec!["local-vision"];
        let path_owned = path.to_path_buf();
        let result = tokio::task::spawn_blocking(move || run_local_vision(&path_owned))
            .await
            .map_err(|e| format!("spawn_blocking 失败: {}", e));
        let result = match result {
            Ok(r) => r,
            Err(e) => {
                return OcrResult::Failed {
                    error: e,
                    attempted,
                }
            }
        };
        match result {
            Ok(text) => OcrResult::Ok {
                text: crate::ingest::sanitize::sanitize_ocr_markup(&text),
                backend: "local-vision",
                elapsed_ms: started.elapsed().as_millis(),
            },
            Err(e) => OcrResult::Failed {
                error: format!("local-vision: {}", e),
                attempted,
            },
        }
    }
}

fn preflight_cloud_backend_local_file(backend: &str, file_size_bytes: u64) -> Result<(), String> {
    match backend {
        "paddle-vl" if file_size_bytes > AISTUDIO_LOCAL_UPLOAD_MAX_BYTES => {
            return Err(format!(
                "PaddleOCR-VL 当前走 AI Studio 本地上传接口,官方上限 50MB;当前文件 {:.1}MB。请改用 MinerU 备用、拆分文件,或后续接入 URL 模式后再解析。",
                bytes_to_mb(file_size_bytes)
            ));
        }
        "ppocrv6" if file_size_bytes > AISTUDIO_LOCAL_UPLOAD_MAX_BYTES => {
            return Err(format!(
                "PP-OCRv6 去水印当前走 AI Studio 本地上传接口,官方上限 50MB;当前文件 {:.1}MB。请先拆分带水印材料,或改走 MinerU 后人工复核水印污染。",
                bytes_to_mb(file_size_bytes)
            ));
        }
        "mineru-precision" if file_size_bytes > PRECISION_MAX_BYTES => {
            return Err(format!(
                "MinerU 精准解析单文件上限 200MB;当前文件 {:.1}MB。请先拆分文件后再解析。",
                bytes_to_mb(file_size_bytes)
            ));
        }
        _ => {}
    }
    Ok(())
}

fn bytes_to_mb(bytes: u64) -> f64 {
    bytes as f64 / MB as f64
}

/// 本机 vision 单次最多识别多少页 PDF(D3-2:旧实现只识别首页,多页扫描件静默丢页)。
/// 超出部分不识别并落 dlog 告警,而非静默截断。
const LOCAL_VISION_MAX_PAGES: u32 = 50;

/// 本机 vision OCR:调 :8899 多模态。
///
/// 流程:
/// 1. PDF → 用 pdftoppm 逐页转 PNG(全部页,上限 `LOCAL_VISION_MAX_PAGES`),逐页 OCR 拼接
/// 2. PNG/JPG → 直接读单图
/// 3. base64 编码后塞进 chat completions 的 `image_url` 字段(见 `ocr_image_via_local_vision`)
/// 4. 调 :8899/v1/chat/completions(MiniCPM-V 4.6 + mmproj)
/// 5. 返回纯文本
///
/// 2026-05-23 R&D 实测:M3 Max 13-15 秒/页。
fn run_local_vision(path: &Path) -> Result<String, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if ext == "pdf" {
        // PDF → 逐页 PNG(pdftoppm),转**全部页**(上限 LOCAL_VISION_MAX_PAGES),逐页 OCR 后拼接。
        // 修 D3-2:旧实现写死 `-f 1 -l 1` 只识别首页,多页判决书/笔录在本地默认模式下静默丢内容。
        // 用临时目录避免污染源目录;Drop 时自动清理。
        let tmp_dir = tempfile::tempdir().map_err(|e| format!("创建临时目录失败: {}", e))?;
        let out_prefix = tmp_dir.path().join(path.file_stem().unwrap_or_default());
        let mut pdftoppm_cmd = std::process::Command::new("pdftoppm");
        pdftoppm_cmd
            .arg("-png")
            .arg("-r")
            .arg(PDF_TO_PNG_DPI.to_string())
            .arg("-f")
            .arg("1")
            .arg("-l")
            .arg(LOCAL_VISION_MAX_PAGES.to_string())
            .arg(path)
            .arg(&out_prefix);
        crate::proc_util::hide_console_window_std(&mut pdftoppm_cmd);
        let status = pdftoppm_cmd.status().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                "pdftoppm 未安装(brew install poppler)".to_string()
            } else {
                format!("调 pdftoppm 失败: {}", e)
            }
        })?;
        if !status.success() {
            return Err(format!("pdftoppm 退出码 {:?}", status.code()));
        }

        // 收集所有页 PNG,按页号排序。pdftoppm 会按总页数零填充页号(<prefix>-1 或 -01),
        // 故不假设具体格式:取文件名最后一个 '-' 后的数字作页号。
        let mut pages: Vec<(u32, PathBuf)> = Vec::new();
        for entry in
            std::fs::read_dir(tmp_dir.path()).map_err(|e| format!("读临时目录失败: {}", e))?
        {
            let p = entry.map_err(|e| format!("读目录项失败: {}", e))?.path();
            if p.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }
            let page_no = p
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|stem| stem.rsplit('-').next())
                .and_then(|n| n.parse::<u32>().ok());
            if let Some(n) = page_no {
                pages.push((n, p));
            }
        }
        pages.sort_by_key(|(n, _)| *n);
        if pages.is_empty() {
            return Err("pdftoppm 没生成任何 PNG 页".to_string());
        }
        let total = pages.len();
        if total >= LOCAL_VISION_MAX_PAGES as usize {
            crate::dlog!(
                "[local-vision] PDF 页数达上限 {} 页,超出部分未识别(可能截断)",
                LOCAL_VISION_MAX_PAGES
            );
        }

        // 逐页 OCR;单页失败不致命(记 dlog 跳过保留其它页),全部失败才报错。
        let mut acc = String::new();
        let mut ok_pages = 0usize;
        for (n, png) in &pages {
            let bytes = match std::fs::read(png) {
                Ok(b) => b,
                Err(e) => {
                    crate::dlog!("[local-vision] 读第 {} 页 PNG 失败(跳过): {}", n, e);
                    continue;
                }
            };
            match ocr_image_via_local_vision(&bytes, "image/png") {
                Ok(text) => {
                    if total > 1 {
                        acc.push_str(&format!("\n\n--- 第 {} 页 ---\n\n", n));
                    }
                    acc.push_str(&text);
                    ok_pages += 1;
                }
                Err(e) => {
                    crate::dlog!("[local-vision] 第 {} 页 OCR 失败(跳过): {}", n, e);
                }
            }
        }
        if ok_pages == 0 {
            return Err(format!("本机 vision 全部 {} 页 OCR 均失败", total));
        }
        let trimmed = acc.trim();
        if trimmed.chars().count() < 30 {
            return Err(format!(
                "本机 vision 抽出文本太短({} 字,{}/{} 页成功)",
                trimmed.chars().count(),
                ok_pages,
                total
            ));
        }
        Ok(trimmed.to_string())
    } else if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "tiff" | "bmp") {
        let image_bytes = std::fs::read(path).map_err(|e| format!("读图片失败: {}", e))?;
        let mime = if ext == "jpg" || ext == "jpeg" {
            "image/jpeg"
        } else {
            "image/png"
        };
        let text = ocr_image_via_local_vision(&image_bytes, mime)?;
        if text.chars().count() < 30 {
            return Err(format!(
                "本机 vision 抽出文本太短({} 字)",
                text.chars().count()
            ));
        }
        Ok(text)
    } else {
        Err(format!("本机 vision 不支持扩展名: {}", ext))
    }
}

/// 调本机 :8899 多模态(MiniCPM-V)对**单张图片**做 OCR,返回 trim 后的纯文本。
/// 不做长度校验(由调用方按整份文档判定),便于多页逐页复用。
fn ocr_image_via_local_vision(image_bytes: &[u8], mime: &str) -> Result<String, String> {
    use base64::Engine;
    let img_b64 = base64::engine::general_purpose::STANDARD.encode(image_bytes);
    let data_url = format!("data:{};base64,{}", mime, img_b64);

    let payload = serde_json::json!({
        "model": LOCAL_VISION_MODEL,
        "messages": [{
            "role": "user",
            "content": [
                {"type": "text", "text": LOCAL_VISION_OCR_PROMPT},
                {"type": "image_url", "image_url": {"url": data_url}}
            ]
        }],
        "temperature": 0.0,
        "max_tokens": 4096,
        "stream": false,
    });
    let body = serde_json::to_vec(&payload).map_err(|e| format!("序列化失败: {}", e))?;

    let resp = ureq::post(LOCAL_VISION_ENDPOINT)
        .timeout(std::time::Duration::from_secs(LOCAL_VISION_TIMEOUT_SEC))
        .set("Content-Type", "application/json")
        .send_bytes(&body)
        .map_err(|e| {
            format!(
                "调用 llama-server :8899 失败: {} (检查 llama-server 是否在跑)",
                e
            )
        })?;

    let resp_json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 llama-server 响应失败: {}", e))?;

    let text = resp_json
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("响应格式异常: {:.200}", resp_json))?
        .trim()
        .to_string();
    Ok(text)
}
