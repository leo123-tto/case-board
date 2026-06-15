pub mod chat;
pub mod court_sms;
pub mod db;
pub mod deepseek;
pub mod diagnostic_log;
pub mod docx_filing;
pub mod embedding;
pub mod export;
pub mod express;
pub mod feedback;
pub mod ingest;
pub mod lifecycle;
pub mod llm;
pub mod local_kb;
// 私人专属功能 Rust 侧(双轨发布模型)。开源仓此文件为桩(命令返回 Err),照样编译。
pub mod private;
pub mod settings;
pub mod team;
pub mod telemetry;
pub mod update;
pub mod verify;
pub mod yuandian;

use std::path::Path;

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{path::BaseDirectory, Emitter, Manager};

use crate::db::cases::{self as cases_db, Case};
use crate::db::documents::{self as documents_db, Document};
use crate::ingest::case_split;
use crate::ingest::pipeline;
use crate::ingest::scanner::{scan_folder, ScannedDoc};

// ============================================================================
// 公共类型
// ============================================================================

#[derive(Debug, Serialize)]
pub struct DbHealth {
    pub ok: bool,
    pub table_count: i64,
    pub case_count: i64,
    pub db_path: String,
}

/// 导入一个案件文件夹后返回的完整结果:案件 + 扫描出的文档清单。
#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub case: Case,
    pub docs: Vec<ScannedDoc>,
    /// 是否是 upsert 命中已存在的案件(true = 之前导入过,这次只刷新)
    pub is_existing: bool,
}

/// 案件 + 它的文档列表(用于详情页)。
#[derive(Debug, Serialize)]
pub struct CaseWithDocs {
    pub case: Case,
    pub documents: Vec<Document>,
}

// ============================================================================
// Tauri Commands
// ============================================================================

/// 读一个文本文件的内容(用于 AI 产物 markdown 渲染)。
///
/// 安全限制:
///   - 只读 UTF-8 文本(.md / .txt / .html / .htm)
///   - 大小上限 5MB(超过的可能是误识别,前端展示不动)
const TEXT_FILE_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// 用系统默认应用打开一个文件(PDF → Preview,docx → Word,图片 → Preview,etc.)。
///
/// 2026-06-15:原先硬编码 macOS `open`,Windows 上无此命令直接失败(用户反映"打不开源文件")。
/// 改用 `tauri-plugin-opener` 跨平台自由函数(mac `open` / win `start` / linux `xdg-open`)。
#[tauri::command]
fn open_in_default_app(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("文件不存在: {}", path));
    }
    tauri_plugin_opener::open_path(&path, None::<&str>).map_err(|e| format!("无法打开: {}", e))
}

/// 用系统默认浏览器打开一个 URL(Settings 里的"申请 token"链接用)。
///
/// 2026-05-24 k:Tauri WebView 里 `<a target="_blank">` 不会跳系统浏览器,必须走原生 open。
/// 2026-06-15:原 `Command::new("open")` 只在 macOS 工作,Windows 申请按钮点了没反应。
/// 改用 `tauri-plugin-opener` 跨平台自由函数。
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(format!("不是合法 http(s) URL: {}", url));
    }
    tauri_plugin_opener::open_url(&url, None::<&str>).map_err(|e| format!("无法打开浏览器: {}", e))
}

/// 在文件管理器中显示该路径(选中并打开父目录)。
///
/// 2026-06-15:跨平台改造(原 macOS `open -R` → opener 的 reveal_item_in_dir)。
#[tauri::command]
fn reveal_in_finder(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("路径不存在: {}", path));
    }
    tauri_plugin_opener::reveal_item_in_dir(&path)
        .map_err(|e| format!("无法在文件管理器中显示: {}", e))
}

#[tauri::command]
fn read_text_file(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("文件不存在: {}", path));
    }
    if !p.is_file() {
        return Err(format!("不是文件: {}", path));
    }
    let size = std::fs::metadata(p)
        .map(|m| m.len())
        .map_err(|e| format!("读不到文件元信息: {}", e))?;
    if size > TEXT_FILE_MAX_BYTES {
        return Err(format!(
            "文件太大({} 字节),超过 {} MB 上限,渲染会卡",
            size,
            TEXT_FILE_MAX_BYTES / 1024 / 1024
        ));
    }
    std::fs::read_to_string(p).map_err(|e| format!("读文件失败: {}", e))
}

/// 抽 Word/RTF/ODT 等 office 文档的纯文本(用 macOS 自带 textutil)。
///
/// V0.1 阶段最简单可靠的方案:`textutil -convert txt -stdout <path>` 把
/// `.docx / .doc / .rtf / .odt / .html / .webarchive` 都能转纯文本。
/// 不依赖 Rust office crate(它们很多在中文场景上有坑),不用 Word 启动开销。
///
/// 后续 Layer 2 完整版做 .pdf(走 MinerU/pdfium)。
#[tauri::command]
fn extract_doc_text(path: String) -> Result<String, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("文件不存在: {}", path));
    }
    if !p.is_file() {
        return Err(format!("不是文件: {}", path));
    }
    let size = std::fs::metadata(p)
        .map(|m| m.len())
        .map_err(|e| format!("读不到文件元信息: {}", e))?;
    // 50MB 上限——比纯文本宽,因为 docx 含图片/字体会大
    const DOC_MAX_BYTES: u64 = 50 * 1024 * 1024;
    if size > DOC_MAX_BYTES {
        return Err(format!(
            "文件太大({:.1} MB),超过 {} MB 上限",
            size as f64 / 1024.0 / 1024.0,
            DOC_MAX_BYTES / 1024 / 1024
        ));
    }
    let output = std::process::Command::new("textutil")
        .arg("-convert")
        .arg("txt")
        .arg("-stdout")
        .arg(&path)
        .output()
        .map_err(|e| format!("调 textutil 失败: {}(macOS 自带,正常情况不会出错)", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "textutil 转换失败: {}",
            if stderr.is_empty() {
                "未知错误"
            } else {
                stderr.trim()
            }
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("textutil 输出不是 UTF-8: {}", e))
}

/// 纯扫描(不入库),给前端"先看看"用。保留兼容 task #5 时的接口。
#[tauri::command]
fn scan_case_folder(path: String) -> Result<Vec<ScannedDoc>, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("路径不存在: {}", path));
    }
    if !p.is_dir() {
        return Err(format!("不是文件夹: {}", path));
    }
    Ok(scan_folder(p))
}

/// 导入一个案件文件夹:扫描 + upsert 案件 + 替换文档列表 + 后台 spawn 字段抽取。
///
/// 入库后重启 App,这个案件依然在,这是 V0.1 端到端的核心动作。
/// 字段抽取在后台 tokio task 跑,前端订阅 "extraction_progress" 事件看进度。
#[tauri::command]
async fn import_case_folder(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    path: String,
) -> Result<ImportResult, String> {
    let p = Path::new(&path);
    if !p.exists() {
        return Err(format!("路径不存在: {}", path));
    }
    if !p.is_dir() {
        return Err(format!("不是文件夹: {}", path));
    }

    // 1) 用文件夹最后一段做默认案件名
    let default_name = p
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "未命名案件".to_string());

    // 2) 先看是不是已经导入过(判断 is_existing 标记)
    let pre_existing = cases_db::find_case_by_folder(pool.inner(), &path)
        .await
        .map_err(db_err)?;
    let is_existing = pre_existing.is_some();

    // 3) Upsert 案件
    let case = cases_db::upsert_case_for_folder(pool.inner(), &path, &default_name, "诉讼")
        .await
        .map_err(db_err)?;

    // 4) 扫描文件夹(这里是同步,scanner 很快)
    let scanned = scan_folder(p);

    // 5) 替换文档列表
    documents_db::replace_documents_for_case(pool.inner(), &case.id, &scanned)
        .await
        .map_err(db_err)?;

    // 6) 后台启动字段抽取(立即返回,前端通过事件订阅进度)
    let docs_for_extraction = documents_db::list_documents_by_case(pool.inner(), &case.id)
        .await
        .map_err(db_err)?;
    pipeline::spawn_extraction(
        app.clone(),
        pool.inner().clone(),
        case.id.clone(),
        docs_for_extraction,
        true, // 导入新案件:全部文档抽完后跑一次全案分析
    );

    Ok(ImportResult {
        case,
        docs: scanned,
        is_existing,
    })
}

/// 多案件检测:对一个文件夹做「拆分预案」(只读,不写库)。前端据此决定是否弹拆分预览。
/// `multi=false` 时按现状走 `import_case_folder` 单案导入即可。
/// 详见 `docs/提案-多案件文件夹识别-2026-06-04.md`。
#[tauri::command]
async fn plan_import_folder(
    pool: tauri::State<'_, SqlitePool>,
    path: String,
) -> Result<case_split::ImportPlan, String> {
    let p = Path::new(&path);
    if !p.is_dir() {
        return Err(format!("不是文件夹: {}", path));
    }
    let mut plan = case_split::plan_folder(p);
    // 根文件夹此前是否已作为「单个案件」导入过(拆分会与旧案重复 → 前端告警)
    plan.root_already_imported = cases_db::find_case_by_folder(pool.inner(), &path)
        .await
        .map_err(db_err)?
        .is_some();
    Ok(plan)
}

/// 确认后的一个待建案件(前端可改名)。
#[derive(Debug, serde::Deserialize)]
pub struct CommitCase {
    pub dir: String,
    pub name: String,
}

/// 拆分批量建案的**写库部分**(不含后台抽取,便于真库集成测试)。
///
/// - `root`:被拖入的上层文件夹。若它此前已作为**单个案件**导入过(且不在本次要建的子案件里),
///   先把那个旧的整体案件删掉 —— 否则它的文档行占着 `source_path`,子案件 INSERT 会撞唯一约束。
///   删旧案 = 用拆分结果替换它。
/// - 每个案件 = upsert(子目录) + 扫描 + 替换文档。
/// - 共用材料(Phase 2,migration 0019 后)挂到**每个**案件:`(case_id, source_path)` 复合唯一,
///   同一文件在各案各一行。各案件子目录互不相交、共用目录是独立兄弟,故同一案内不会出现重复 source_path。
async fn build_split_cases(
    pool: &SqlitePool,
    root: &str,
    cases: &[CommitCase],
    shared_dirs: &[String],
) -> Result<Vec<ImportResult>, String> {
    if cases.is_empty() {
        return Err("没有要导入的案件".to_string());
    }
    // 旧的「整体作单案」记录 → 删掉,释放其文档占用的 source_path
    if !root.is_empty() && !cases.iter().any(|c| c.dir == root) {
        if let Some(old) = cases_db::find_case_by_folder(pool, root)
            .await
            .map_err(db_err)?
        {
            cases_db::delete_case(pool, &old.id).await.map_err(db_err)?;
        }
    }

    let mut results = Vec::with_capacity(cases.len());
    for c in cases.iter() {
        let dir = Path::new(&c.dir);
        if !dir.is_dir() {
            return Err(format!("案件目录不存在: {}", c.dir));
        }
        let name = if c.name.trim().is_empty() {
            dir.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "未命名案件".to_string())
        } else {
            c.name.trim().to_string()
        };
        let is_existing = cases_db::find_case_by_folder(pool, &c.dir)
            .await
            .map_err(db_err)?
            .is_some();
        let case = cases_db::upsert_case_for_folder(pool, &c.dir, &name, "诉讼")
            .await
            .map_err(db_err)?;
        let mut scanned = scan_folder(dir);
        // 共用材料挂到**每个**案件(migration 0019 后 (case_id, source_path) 复合唯一,可多挂)
        for sd in shared_dirs {
            let sp = Path::new(sd);
            if sp.is_dir() {
                scanned.extend(scan_folder(sp));
            }
        }
        documents_db::replace_documents_for_case(pool, &case.id, &scanned)
            .await
            .map_err(db_err)?;
        results.push(ImportResult {
            case,
            docs: scanned,
            is_existing,
        });
    }
    Ok(results)
}

/// 按确认后的拆分预案**批量建案**(写库 + 每案后台抽取)。前端拆分弹窗点确认后调用。
#[tauri::command]
async fn commit_import_folder(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    root: String,
    cases: Vec<CommitCase>,
    shared_dirs: Vec<String>,
) -> Result<Vec<ImportResult>, String> {
    let results = build_split_cases(pool.inner(), &root, &cases, &shared_dirs).await?;
    // 写库成功后,逐案启动后台抽取(年份大文件夹会并发 N 个,Phase 1 限制,后续做节流)
    for r in &results {
        let docs = documents_db::list_documents_by_case(pool.inner(), &r.case.id)
            .await
            .map_err(db_err)?;
        pipeline::spawn_extraction(
            app.clone(),
            pool.inner().clone(),
            r.case.id.clone(),
            docs,
            true,
        );
    }
    Ok(results)
}

/// 列出所有已导入的案件(用于"最近案件" / 案件列表页)。
#[tauri::command]
async fn list_cases(pool: tauri::State<'_, SqlitePool>) -> Result<Vec<Case>, String> {
    cases_db::list_cases(pool.inner()).await.map_err(db_err)
}

/// 删除一个案件(级联删除所有关联文档/事件/联系人)。
///
/// 不动原始文件夹,只删 CaseBoard 数据库里这个案件的记录。
#[tauri::command]
async fn delete_case(pool: tauri::State<'_, SqlitePool>, id: String) -> Result<(), String> {
    cases_db::delete_case(pool.inner(), &id)
        .await
        .map_err(db_err)
}

/// 取案件详情 + 文档列表(详情页用)。
#[tauri::command]
async fn get_case_with_docs(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
) -> Result<CaseWithDocs, String> {
    let case = cases_db::get_case(pool.inner(), &id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| format!("案件不存在: {}", id))?;
    let documents = documents_db::list_documents_by_case(pool.inner(), &id)
        .await
        .map_err(db_err)?;
    Ok(CaseWithDocs { case, documents })
}

/// 读取用户设置(给前端 SettingsModal 用)。
///
/// 自动补上默认 endpoint(MinerU / Ollama),但 api_key 不补默认值。
#[tauri::command]
fn get_settings() -> Result<settings::Settings, String> {
    settings::read_settings().map(|s| s.with_defaults_for_display())
}

/// 2026-05-25 V0.1.6 · 若 cases 表为空,seed 一个示例案件「张三 诉 李四 民间借贷」。
/// onboarding 完成时(开始使用 / 稍后再配置都触发)调一次。
#[tauri::command]
async fn seed_demo_case_if_empty(pool: tauri::State<'_, SqlitePool>) -> Result<bool, String> {
    db::seed::seed_demo_case_if_empty(pool.inner())
        .await
        .map_err(db_err)
}

/// 2026-05-25 V0.1.6 · 验证 MinerU API token,前端「验证」按钮触发。
#[tauri::command]
async fn verify_mineru_key(token: String) -> verify::VerifyResult {
    verify::verify_mineru_key(&token).await
}

/// 2026-06-12 · 验证 PaddleOCR VL(AI Studio)访问令牌,前端「验证」按钮触发。
#[tauri::command]
async fn verify_paddle_vl_key(token: String) -> verify::VerifyResult {
    verify::verify_paddle_vl_key(&token).await
}

/// 2026-05-25 V0.1.6 · 验证 DeepSeek API key,前端「验证」按钮触发。
#[tauri::command]
async fn verify_deepseek_key(api_key: String, endpoint: Option<String>) -> verify::VerifyResult {
    verify::verify_deepseek_key(&api_key, endpoint.as_deref()).await
}

/// 2026-05-25 V0.1.8 · 验证元典(open.chineselaw.com)API key,前端「验证」按钮触发。
#[tauri::command]
async fn verify_yuandian_key(api_key: String) -> verify::VerifyResult {
    verify::verify_yuandian_key(&api_key).await
}

/// 2026-06-15 · 验证 MiniMax API key,前端「验证」按钮触发。
#[tauri::command]
async fn verify_minimax_key(api_key: String, endpoint: Option<String>) -> verify::VerifyResult {
    verify::verify_minimax_key(&api_key, endpoint.as_deref()).await
}

/// 2026-05-25 V0.1.8 · 检测版本更新。
///
/// 前端启动时调一次(静默,失败不报错),设置页「检查更新」按钮也调。
/// 数据源:发布站点的 version.json。返回 UpdateInfo 给前端判断是否弹提示。
#[tauri::command]
async fn check_for_update() -> update::UpdateInfo {
    update::check_for_update().await
}

/// 2026-05-25 V0.1.8 · 拿当前 App 版本(等同于 Tauri 的 getVersion,但走 Cargo.toml)。
///
/// 前端有 @tauri-apps/api 的 getVersion 也行,这里提供同步包装方便偶尔单文件用。
#[tauri::command]
fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// 写入用户设置(全量覆盖,前端发来什么就存什么)。
///
/// **例外:`team` 字段以磁盘现值为准。** 团队身份只能通过 team_* 命令改(后台直写),
/// 设置页表单从打开到保存之间团队状态可能已变(建团/退团/被踢),全量覆盖会用打开时的
/// 旧值把团队身份冲掉/复活 —— 结构上掐死这条路,不依赖前端记得同步镜像。
#[tauri::command]
fn save_settings(payload: settings::Settings) -> Result<(), String> {
    let mut payload = payload;
    payload.team = settings::read_settings().ok().and_then(|s| s.team);
    settings::write_settings(&payload)
}

/// 2026-05-26 V0.1.13 · 单独写"首页在办案件"用户拖动后的顺序。
///
/// 不让前端走 save_settings 全量覆盖 — 那会跟 SettingsModal 同时写的话有覆盖竞态。
/// 这里 read-modify-write:只动 home_case_order 字段,其他不动。
#[tauri::command]
fn update_home_case_order(case_ids: Vec<String>) -> Result<(), String> {
    let mut s = settings::read_settings()?;
    s.home_case_order = if case_ids.is_empty() {
        None
    } else {
        Some(case_ids)
    };
    settings::write_settings(&s)
}

/// 检测本机模型 + llama-server 状态(给 onboarding 和 Settings 用)。
#[tauri::command]
fn detect_local_readiness(model_dir: Option<String>) -> lifecycle::LocalReadiness {
    lifecycle::detect_local_readiness(model_dir.as_deref())
}

/// 后台启动 llama-server。如果已经在跑 / 模型缺失 / 二进制缺失就报错。
#[tauri::command]
async fn ensure_local_ready() -> Result<(), String> {
    let s = settings::read_settings().unwrap_or_default();
    lifecycle::ensure_local_ready(s.local_model_dir.as_deref()).await
}

/// 从一段诉讼文书纯文本里抽取结构化字段(案号/法院/原告/被告/案由/金额/起诉日期)。
///
/// 2026-05-25:修 bug — 之前硬编码 `local_llamacpp_default()`,导致用户即使
/// 选了云端 LLM,在 MarkdownModal 打开单份文档预览时也走本机,本机服务没起
/// 来就报"LLM 提取失败"。现改成 `from_settings`,跟主 pipeline 保持一致。
#[tauri::command]
async fn extract_fields_from_text(text: String) -> Result<llm::ExtractedFields, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let config = llm::LlmConfig::from_settings(&settings);
    llm::extract_case_fields(&config, &text)
        .await
        .map_err(|e| e.to_string())
}

/// 对所有案件重跑一次 LLM 全局抽(2026-05-24 h · 替代旧规则 aggregator)。
///
/// 用途:升级 prompt(或新增字段)后,把存量案件的 cases.agg_* + 案件分析报告**全部刷新**。
/// 用户在详情页"↻ 重新计算画像"按钮触发。串行跑,每个案件约 10-30 秒(取决于文档数 + 网络)。
///
/// 注意:**会重新调 LLM**(不像旧 aggregator 不调 LLM),所以耗时更长但更准。
/// 前端按钮文案要提示"重抽中,可能需要几分钟"。
#[tauri::command]
async fn reaggregate_all_cases(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<ingest::global_pipeline::ReaggregateReport, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let llm_config = llm::LlmConfig::from_settings(&settings);
    ingest::global_pipeline::rerun_all_cases(pool.inner(), &llm_config)
        .await
        .map_err(db_err)
}

/// 2026-05-24 k · 元典 P1 主动触发:查被执行人 + LLM 风险提示报告。
///
/// 流程:
///   1. 从 cases.agg_party_contacts 拿被执行人 → 跑 yuandian_basic_query → raw JSON 落盘
///   2. 喂 DeepSeek 出风险报告 + 深挖建议 JSON
///   3. 写 cases.risk_assessment_path / risk_assessment_at
///   4. 返回报告路径 + 深挖建议(给前端)
#[derive(serde::Serialize)]
struct YuandianP1Response {
    orchestrator: yuandian::orchestrator::OrchestratorReport,
    assessment: yuandian::risk_assessment::AssessmentReport,
}

/* ============================================================
 * 2026-05-25 · 还款记录 (case_payments) commands
 * ============================================================ */

#[tauri::command]
async fn add_payment(
    pool: tauri::State<'_, SqlitePool>,
    new: db::payments::NewPayment,
) -> Result<db::payments::Payment, String> {
    db::payments::add(pool.inner(), new).await.map_err(db_err)
}

#[tauri::command]
async fn list_payments(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<Vec<db::payments::Payment>, String> {
    db::payments::list_by_case(pool.inner(), &case_id)
        .await
        .map_err(db_err)
}

#[tauri::command]
async fn delete_payment(pool: tauri::State<'_, SqlitePool>, id: String) -> Result<u64, String> {
    db::payments::delete(pool.inner(), &id)
        .await
        .map_err(db_err)
}

/* ============================================================
 * 2026-06-13 · 案件待办清单 (case_todos) commands(胡彬律师反馈)
 * ============================================================ */

#[tauri::command]
async fn add_todo(
    pool: tauri::State<'_, SqlitePool>,
    new: db::todos::NewTodo,
) -> Result<db::todos::Todo, String> {
    db::todos::add(pool.inner(), new).await.map_err(db_err)
}

#[tauri::command]
async fn list_todos(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<Vec<db::todos::Todo>, String> {
    db::todos::list_by_case(pool.inner(), &case_id)
        .await
        .map_err(db_err)
}

/// 跨案件未完成待办(首页"待办汇总"用)。
#[tauri::command]
async fn list_open_todos(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<Vec<db::todos::OpenTodoRow>, String> {
    db::todos::list_open(pool.inner()).await.map_err(db_err)
}

#[tauri::command]
async fn update_todo(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
    upd: db::todos::UpdateTodo,
) -> Result<u64, String> {
    db::todos::update(pool.inner(), &id, &upd)
        .await
        .map_err(db_err)
}

#[tauri::command]
async fn delete_todo(pool: tauri::State<'_, SqlitePool>, id: String) -> Result<u64, String> {
    db::todos::delete(pool.inner(), &id).await.map_err(db_err)
}

// ===== 独立日历日程(2026-06-14;不绑案件,首页日历右键添加 / 删除) =====
#[tauri::command]
async fn add_calendar_event(
    pool: tauri::State<'_, SqlitePool>,
    new: db::calendar_events::NewCalendarEvent,
) -> Result<db::calendar_events::CalendarEvent, String> {
    db::calendar_events::add(pool.inner(), new)
        .await
        .map_err(db_err)
}

#[tauri::command]
async fn list_calendar_events(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<Vec<db::calendar_events::CalendarEvent>, String> {
    db::calendar_events::list_all(pool.inner())
        .await
        .map_err(db_err)
}

#[tauri::command]
async fn delete_calendar_event(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
) -> Result<u64, String> {
    db::calendar_events::delete(pool.inner(), &id)
        .await
        .map_err(db_err)
}

/* ============================================================
 * 2026-06-11 · 审级实例 (case_instances) commands
 * ============================================================ */

#[tauri::command]
async fn list_case_instances(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<Vec<db::case_instances::CaseInstance>, String> {
    db::case_instances::list_by_case(pool.inner(), &case_id)
        .await
        .map_err(db_err)
}

#[tauri::command]
async fn add_case_instance(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
    new: db::case_instances::NewInstance,
) -> Result<db::case_instances::CaseInstance, String> {
    db::case_instances::add_user_instance(pool.inner(), &case_id, &new)
        .await
        .map_err(db_err)
}

#[tauri::command]
async fn update_case_instance(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
    new: db::case_instances::NewInstance,
) -> Result<u64, String> {
    db::case_instances::update_instance(pool.inner(), &id, &new)
        .await
        .map_err(db_err)
}

#[tauri::command]
async fn delete_case_instance(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
) -> Result<u64, String> {
    db::case_instances::delete(pool.inner(), &id)
        .await
        .map_err(db_err)
}

/// V0.2.2 · 软删一个文档(用户从材料列表手动移除,主要给 AI artifact 用)。只标 deleted_at,不动磁盘。
#[tauri::command]
async fn delete_document(pool: tauri::State<'_, SqlitePool>, id: String) -> Result<u64, String> {
    let now = chrono::Local::now().to_rfc3339();
    db::documents::soft_delete_document(pool.inner(), &id, &now)
        .await
        .map_err(db_err)
}

/// 2026-05-31 V0.3 · 强制重抽单个文档(源文件列表「重新抽取」按钮)。
///
/// 重置该文档 extraction_status='pending' + 清 last_error → spawn 后台抽取(单文档,
/// 走现有 `extraction_progress` 事件通道,前端订阅看进度 + 完成自动刷新)。
/// ⚠️ 会重跑 OCR/LLM(PDF 走云端 OCR 会再烧 MinerU 积分,用户主动选择)。
#[tauri::command]
async fn reextract_document(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    doc_id: String,
) -> Result<(), String> {
    // 复用共享入口(与 chat 工具 reextract_document 同一逻辑,防漂移)。
    // None = 普通重识别,顺带清除该文档之前可能设过的去水印覆盖。
    pipeline::trigger_reextract(app, pool.inner(), &doc_id, None)
        .await
        .map(|_| ())
}

/// 2026-06-13(胡彬律师反馈)· 去水印重新识别:对带大幅水印的工商调档件,
/// 强制走 PP-OCRv6(纯文字)+ 去水印过滤(不回退 VL)。同样不自动跑全案分析(省钱)。
#[tauri::command]
async fn reextract_document_dewatermark(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    doc_id: String,
) -> Result<(), String> {
    pipeline::trigger_reextract(app, pool.inner(), &doc_id, Some("ppocrv6"))
        .await
        .map(|_| ())
}

/// 2026-05-25 V0.1.7 · 完整报告:合并风险报告 + 深挖报告 → DeepSeek 总结出第三份
#[tauri::command]
async fn yuandian_full_report(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<yuandian::full_report::FullReportResult, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let llm_config = llm::LlmConfig::from_settings(&settings);
    Ok(yuandian::full_report::run_full_report(pool.inner(), &case_id, &llm_config).await)
}

/// 2026-05-24 k-9 · P2 深挖:用 P1 LLM 给的 dig_hints 拉关联公司 / 案号 / 第三方主体,
/// 出深查报告(参考股权转让案件 yuandian_深查 格式)
#[tauri::command]
async fn yuandian_deep_dive(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<yuandian::deep_dive::DeepDiveReport, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let api_key = settings
        .yuandian_api_key
        .as_deref()
        .ok_or_else(|| "元典 API key 未配置 — 请到 Settings 里填".to_string())?;
    let llm_config = llm::LlmConfig::from_settings(&settings);
    Ok(yuandian::deep_dive::run_deep_dive(pool.inner(), &case_id, api_key, &llm_config).await)
}

#[tauri::command]
async fn yuandian_basic_query(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<YuandianP1Response, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let api_key = settings
        .yuandian_api_key
        .as_deref()
        .ok_or_else(|| "元典 API key 未配置 — 请到 Settings 里填".to_string())?;

    // P1.1 跑元典 16 端点
    let orch = yuandian::orchestrator::basic_query(pool.inner(), &case_id, api_key).await?;

    // P1.2 LLM 写风险报告
    let llm_config = llm::LlmConfig::from_settings(&settings);
    let assess = yuandian::risk_assessment::run_assessment(
        pool.inner(),
        &case_id,
        &llm_config,
        &orch.raw_files,
    )
    .await;

    Ok(YuandianP1Response {
        orchestrator: orch,
        assessment: assess,
    })
}

/// 2026-05-24 j · 导出案件分析报告为 HTML(陶土红 × 羊皮纸专业风格)。
/// 前端先用 dialog/save 拿到 save_path,然后调本 command。
#[tauri::command]
async fn export_report_html(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
    save_path: String,
) -> Result<String, String> {
    let p = export::export_report_html_to(pool.inner(), &case_id, std::path::Path::new(&save_path))
        .await?;
    Ok(p.to_string_lossy().to_string())
}

/// 2026-05-24 j · 导出案件分析报告为 Word(.docx)。2026-06-04 改走 `docx_filing` base 档
/// (原生 OOXML,零外部依赖),替代旧的 macOS textutil 路径。
#[tauri::command]
async fn export_report_docx(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
    save_path: String,
) -> Result<String, String> {
    let p = export::export_report_docx_to(pool.inner(), &case_id, std::path::Path::new(&save_path))
        .await?;
    Ok(p.to_string_lossy().to_string())
}

/// 2026-05-25 V0.1.7 · 通用 MD → HTML 导出。
/// 用于风险报告 / 深挖报告 / 完整报告(任何 MD 文件 + 标题)。
#[tauri::command]
async fn export_md_html(
    md_path: String,
    title: String,
    save_path: String,
) -> Result<String, String> {
    let p = export::export_md_html_to(
        std::path::Path::new(&md_path),
        &title,
        std::path::Path::new(&save_path),
    )
    .await?;
    Ok(p.to_string_lossy().to_string())
}

/// 2026-05-25 V0.1.7 · 通用 MD → Word 导出。
#[tauri::command]
async fn export_md_docx(
    md_path: String,
    title: String,
    save_path: String,
) -> Result<String, String> {
    let p = export::export_md_docx_to(
        std::path::Path::new(&md_path),
        &title,
        std::path::Path::new(&save_path),
    )
    .await?;
    Ok(p.to_string_lossy().to_string())
}

/// 2026-05-31 V0.3 M1 · 把 save_artifact 生成的文书导出为 **Word(法律格式)**。
///
/// 走 `docx_filing` **filing 档**(MD→原生 OOXML),复刻 quote.law 样本排版(方正小标宋标题 /
/// 黑体小标题 / 仿宋正文 / 两端对齐 / 首行缩进2字 / 1.5倍行距);与 `export_md_docx` 的 base 档
/// 共享同一引擎,仅多几条法律叠加(列表去圆点 / 软换行并段 / 丢分隔线)。
/// `doc_id` 是 documents 行 id;标题优先取文书元信息头,缺则用文件名兜底。
#[tauri::command]
async fn export_filing_docx(
    pool: tauri::State<'_, SqlitePool>,
    doc_id: String,
    save_path: String,
) -> Result<String, String> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT source_path, filename FROM documents WHERE id = ?")
            .bind(&doc_id)
            .fetch_optional(pool.inner())
            .await
            .map_err(|e| format!("查文书失败:{}", e))?;
    let (md_path, filename) = row.ok_or_else(|| "文书不存在(doc_id 无效)".to_string())?;
    let md = std::fs::read_to_string(&md_path).map_err(|e| format!("读文书 MD 失败:{}", e))?;
    let title = docx_filing::extract_filing_title(&md)
        .unwrap_or_else(|| filename.trim_end_matches(".md").to_string());
    let bytes = docx_filing::build_filing_docx_bytes(&title, &md)?;
    std::fs::write(&save_path, &bytes).map_err(|e| format!("写 docx 失败:{}", e))?;
    Ok(save_path)
}

/// save_editor_doc 返回:被写回的 document id(本批永远 = 传入 doc_id,原地覆盖)。
#[derive(serde::Serialize)]
struct EditorSaveResult {
    doc_id: String,
}

/// 2026-05-31 V0.3 D1+D2 · Milkdown 编辑器保存:把编辑后的正文写回该文书 .md 文件。
///
/// 前端只传 (title, 正文 content_md,**不含注释头**);后端按 doc_id 查 category(=doc_type)
/// 重建 filing 注释头(对齐 `chat::tools::artifact::persist_filing` 格式)→ 原地覆盖 source_path
/// → 更新 size_bytes/modified_at。**只允许编辑 AI 产物文书**(is_ai_artifact=1),绝不覆盖
/// 导入的原始文件。导出/解析头依赖此格式,见 docs/V0.3-Milkdown编辑器-实施落地.md §1.4/§1.6。
#[tauri::command]
async fn save_editor_doc(
    pool: tauri::State<'_, SqlitePool>,
    doc_id: String,
    title: String,
    content_md: String,
) -> Result<EditorSaveResult, String> {
    let id = write_editor_doc(pool.inner(), &doc_id, &title, &content_md).await?;
    Ok(EditorSaveResult { doc_id: id })
}

/// `save_editor_doc` 的可测内核(不依赖 Tauri State,单测直接调)。
async fn write_editor_doc(
    pool: &SqlitePool,
    doc_id: &str,
    title: &str,
    content_md: &str,
) -> Result<String, String> {
    if content_md.len() as u64 > TEXT_FILE_MAX_BYTES {
        return Err(format!(
            "文书内容太大({} 字节),超过 {} MB 上限",
            content_md.len(),
            TEXT_FILE_MAX_BYTES / 1024 / 1024
        ));
    }
    let row: Option<(String, Option<String>, bool, String)> = sqlx::query_as(
        "SELECT source_path, category, is_ai_artifact, source FROM documents \
         WHERE id = ? AND deleted_at IS NULL",
    )
    .bind(doc_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("查文书失败:{}", e))?;
    let (source_path, category, is_ai_artifact, source) =
        row.ok_or_else(|| "文书不存在或已删除(doc_id 无效)".to_string())?;
    // 安全闸 1:绝不让编辑器覆盖导入的原始文件(诉状/合同/判决书等原始证据)。
    if !is_ai_artifact {
        return Err("只能编辑 AI 生成的文书,不能覆盖导入的原始文件".to_string());
    }
    // 安全闸 2(V0.3):只允许编辑 app 自有的 chat 文档(source='chat' 分析产物 / 'chat_artifact'
    // 起草文书,均落在 app data 的 extracts/ 内)。编辑器现在对更多 AI 文档开放,但 scanner 会把
    // 用户拖进来的 AI 名 .md/.html 也标 is_ai_artifact(source='scan',source_path 在用户案件
    // 文件夹)—— **原地覆写会改用户原文件(数据丢失)**。用 source 死锁覆写范围,与前端编辑闸同源。
    if source != "chat" && source != "chat_artifact" {
        return Err("只能编辑 AI 助手生成的文书,不能覆盖导入/扫描的原始文件".to_string());
    }

    // 标题写进 HTML 注释,安全化防破坏注释结构(去换行 / 去 `-->`)。
    let safe_title = title.replace(['\n', '\r'], " ").replace("-->", "—>");
    let safe_title = safe_title.trim();
    let doc_type = category.unwrap_or_default();
    let now_iso = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // 重建 filing 头 + 正文(格式对齐 artifact.rs::persist_filing)。
    let body = format!(
        "<!-- filing · doc_type={} · title={} · ts={} -->\n\n{}",
        doc_type, safe_title, now_iso, content_md
    );

    tokio::fs::write(&source_path, &body)
        .await
        .map_err(|e| format!("写文书失败:{}", e))?;

    sqlx::query("UPDATE documents SET size_bytes = ?, modified_at = ? WHERE id = ?")
        .bind(body.len() as i64)
        .bind(&now_iso)
        .bind(doc_id)
        .execute(pool)
        .await
        .map_err(|e| format!("更新文书元信息失败:{}", e))?;

    Ok(doc_id.to_string())
}

/// 2026-05-24 i · 单个案件主动跑一次 LLM 全局抽(用户点「📖 案件报告」按钮触发)。
///
/// 用途:用户进入详情页,如果案件还没生成报告(case_report_path 为 null),
/// 点报告按钮即可立刻触发抽取 + 等结果。也用于强制刷新已有报告。
///
/// 阻塞返回(前端 await 完才弹报告 Modal),时间通常 10-30 秒。
#[tauri::command]
async fn global_extract_case(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<ingest::global_pipeline::GlobalExtractReport, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let llm_config = llm::LlmConfig::from_settings(&settings);
    let report =
        ingest::global_pipeline::run_global_extract(pool.inner(), &case_id, &llm_config).await;
    // 出报告后后台增量索引(新进缓存的法条/案例补进语义索引)
    spawn_kb_auto_index(app);
    Ok(report)
}

/// 项目1:把(通常已结案/判决的)案件提炼成「办案经验卡片」写入本地知识库。
/// 用户在案件详情页点「沉淀为办案经验」触发;返回写入文件的绝对路径。
/// 经验卡片落 `<kb>/raw/cases-experience/`,search_local_kb 整库可检索复用(不脱敏,本机自用)。
#[tauri::command]
async fn distill_case_experience(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<String, String> {
    let settings = settings::read_settings().unwrap_or_default();
    if settings.local_kb_root.is_none() || settings.local_kb_enabled != Some(true) {
        return Err("尚未配置或启用本地知识库,请先在设置里设定知识库目录".into());
    }
    let case = db::cases::get_case(pool.inner(), &case_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("案件不存在")?;
    let report_path = case
        .case_report_path
        .clone()
        .ok_or("该案件还没有分析报告,请先生成「案件报告 / 重新分析」后再沉淀")?;
    let report_md =
        std::fs::read_to_string(&report_path).map_err(|e| format!("读案件报告失败: {e}"))?;
    let brief = case_brief_for_experience(&case);
    let llm_config = llm::LlmConfig::from_settings(&settings);
    let card = llm::global_extract::distill_experience(&llm_config, &brief, &report_md)
        .await
        .map_err(|e| format!("提炼经验卡片失败: {e}"))?;
    let full = format!(
        "{card}\n\n---\n> 来源案件:{} · CaseBoard 自动沉淀\n",
        case.name
    );
    let path = local_kb::experience::save_case_experience(&settings, &case_id, &case.name, &full)
        .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

/// 拼一段案件结构化摘要,补充分析报告未必涵盖的字段,喂给经验提炼 LLM。
fn case_brief_for_experience(case: &db::cases::Case) -> String {
    let mut s = String::new();
    s.push_str(&format!("案件名称:{}\n", case.name));
    if let Some(v) = &case.agg_case_no {
        s.push_str(&format!("案号:{v}\n"));
    }
    if let Some(v) = &case.agg_court {
        s.push_str(&format!("法院:{v}\n"));
    }
    if let Some(v) = &case.agg_cause {
        s.push_str(&format!("案由:{v}\n"));
    }
    if let Some(v) = &case.agg_our_side {
        s.push_str(&format!("我方立场:{v}\n"));
    }
    if let Some(v) = &case.agg_resolution {
        s.push_str(&format!("处理结果:{v}\n"));
    }
    s
}

/// 2026-05-24 e:收集反馈用的诊断信息(给前端弹窗预填用)。
///
/// 收集内容:版本 / OS / provider / 案件数 / 文档统计 / 最近失败 / 匿名 client_id /
/// **2026-05-26 V0.1.11 新加**:Settings 脱敏快照 + 系统级(磁盘/DB) + App stderr ring buffer +
/// 前端 console 错误(由前端打开弹窗时累积传入)。
/// **不含**案件名 / 当事人 / 文档内容(隐私铁律)。
#[tauri::command]
async fn collect_feedback_diagnostic(
    pool: tauri::State<'_, SqlitePool>,
    console_errors: Option<Vec<feedback::ConsoleError>>,
) -> Result<feedback::DiagnosticInfo, String> {
    feedback::collect(pool.inner(), console_errors.unwrap_or_default()).await
}

/// 2026-05-24 e:把诊断信息 + 用户描述拼成 MD,写到 ~/Desktop/。
/// 返回最终文件的绝对路径(前端用来 reveal_in_finder)。
#[tauri::command]
async fn save_feedback_md(
    info: feedback::DiagnosticInfo,
    description: String,
) -> Result<String, String> {
    let path = feedback::save_to_desktop(&info, &description)?;
    Ok(path.to_string_lossy().to_string())
}

/// 2026-05-27 V0.1.13+:打开默认邮件客户端发反馈给作者。
///
/// 实现策略(macOS 主路径):
///   1. 先 osascript 调 Mail.app:自动填收件人 / 主题 / 正文 +「带附件」
///   2. AppleScript 失败(用户没装 Mail.app / 没授权 AppleScript)→ fallback
///      `open mailto:` 链接(默认邮件客户端打开新邮件,但**不带附件**,需要用户手动拖入)
///
/// 返回 `("applescript"|"mailto", warning_message_or_empty)`,
/// 让前端 toast 提示用户:走的哪条路径 + 是否需要手动拖附件。
#[tauri::command]
async fn send_feedback_email(
    md_path: String,
    to: String,
    subject: String,
) -> Result<(String, String), String> {
    feedback::send_via_default_mail(&md_path, &to, &subject).await
}

/// 2026-05-24 e:取 DeepSeek 当前余额 + 今日消费。
///
/// `refresh = true`:发请求拉新数据,落 DB 快照;失败返回错误
/// `refresh = false`:只读 DB 缓存(立即返回,不发请求);无缓存时返回 None
///
/// 仅在 settings.effective_llm_provider() == "cloud" + 有 api_key 时有意义,
/// 前端调用前自己判断(本 command 不做 provider 校验,api_key 缺失时返回 NoApiKey 错误)。
#[tauri::command]
async fn get_deepseek_balance(
    pool: tauri::State<'_, SqlitePool>,
    refresh: bool,
) -> Result<Option<deepseek::DeepSeekBalance>, String> {
    if refresh {
        let settings = settings::read_settings().unwrap_or_default();
        let Some(api_key) = settings.cloud_llm_api_key.as_deref() else {
            return Err("尚未配置 DeepSeek API key".into());
        };
        let bal = deepseek::fetch_balance_and_persist(pool.inner(), api_key)
            .await
            .map_err(|e| e.to_string())?;
        Ok(Some(bal))
    } else {
        deepseek::cached_balance(pool.inner()).await.map_err(db_err)
    }
}

/// 2026-05-24 e:手工覆盖案件工作流状态(看板卡片右上角的 chip)。
///
/// `status = None` → 清空,前端走自动推断;
/// `status = Some(...)` → 用户手工选过,优先取用户值。
///
/// 不校验 status 字面值(枚举约束在前端 TS),DB 层透传。
#[tauri::command]
async fn update_workflow_status(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
    status: Option<String>,
) -> Result<(), String> {
    cases_db::update_workflow_status(pool.inner(), &case_id, status.as_deref())
        .await
        .map_err(db_err)
}

/// 2026-05-26 V0.1.13 · 写入案件 user_overrides JSON(编辑模式手改 overlay)。
///
/// `overrides_json = None` → 清空所有用户改动;
/// `overrides_json = Some(...)` → 整段覆盖。
///
/// 结构由前端 `userOverrides.ts` 定义(fields / hidden_sections / deleted_rows /
/// section_order),后端不解析,sqlite 列定义见 migration 0016。
#[tauri::command]
async fn update_case_overrides(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
    overrides_json: Option<String>,
) -> Result<(), String> {
    cases_db::update_user_overrides(pool.inner(), &case_id, overrides_json.as_deref())
        .await
        .map_err(db_err)
}

/// 2026-05-24 (T3) 重抽该案件的所有 LLM 抽取。
///
/// 用途:升级 prompt(扩字段 / 反诉视角 / is_our_side 等)后,存量
/// `documents.extracted_fields` 是旧 prompt 出的,需要让 LLM 按新 prompt 重抽一遍。
///
/// 做法:
///   1. 把该案件下所有 `extraction_status='done'` 的文档重置为 `'pending'`
///      并清掉 `extracted_fields` / `extracted_text_path`(留 cache_key)
///   2. 触发 pipeline(spawn_extraction)在后台跑,前端订阅 `extraction_progress` 看进度
///   3. 立即返回被重置的文档数,UI 用来 toast 提示"重抽中 · N 份文档"
///
/// 注意:`skipped` / `failed` 的文档**不重置**(用户可能手工跳过了某些噪音文档;
/// failed 的可能是 LLM 暂时不可用,下次重新导入会重试)。
#[tauri::command]
async fn recompute_case_extraction(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<usize, String> {
    // 1) 重置 done → pending,清抽取产物
    let res = sqlx::query(
        "UPDATE documents \
         SET extraction_status = 'pending', \
             extracted_fields = NULL, \
             extracted_text_path = NULL \
         WHERE case_id = ? AND extraction_status = 'done'",
    )
    .bind(&case_id)
    .execute(pool.inner())
    .await
    .map_err(|e| format!("重置失败: {}", e))?;
    let reset_count = res.rows_affected() as usize;

    if reset_count == 0 {
        return Ok(0); // 没什么可重抽的,直接返回
    }

    // 2) 触发 pipeline 后台跑(立即返回,前端通过 extraction_progress 事件看进度)
    let documents = documents_db::list_documents_by_case(pool.inner(), &case_id)
        .await
        .map_err(db_err)?;
    pipeline::spawn_extraction(
        app.clone(),
        pool.inner().clone(),
        case_id.clone(),
        documents,
        true,
    );

    Ok(reset_count)
}

/// 2026-05-25 V0.1.5 「🔄 刷新源文件」按钮触发。
///
/// 增量逻辑:
///   1. 找到案件,定位 `source_folder`
///   2. `scan_folder` 重扫一遍
///   3. `sync_documents_for_case` 做 diff:
///      - 全新文件 → INSERT,status=pending
///      - mtime+size 变了 → UPDATE,清抽取产物,status=pending
///      - 完全没变 → 不动
///      - 磁盘消失 → 标 `deleted_at`(软删,LLM corpus 不再带它,但 DB 留痕)
///   4. 如果有任何变化(added/updated/deleted),后台 spawn_extraction 跑 pending +
///      自动重跑 global_extract 生成新画像 + 新报告
///   5. 返回 `SyncStats` 给前端 toast 显示
///
/// 用户体验:点按钮 → 立即弹 toast「新增 X / 更新 Y / 移除 Z」→ 后台慢慢跑抽取,
/// 完成后前端通过 `extraction_progress` 事件自动刷新卡片和报告。
#[tauri::command]
async fn refresh_case_files(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<documents_db::SyncStats, String> {
    // 1) 拿案件 + source_folder
    let case = cases_db::get_case(pool.inner(), &case_id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| format!("案件不存在: {}", case_id))?;

    let folder = Path::new(&case.source_folder);
    if !folder.exists() {
        return Err(format!("案件源文件夹已不存在: {}", case.source_folder));
    }
    if !folder.is_dir() {
        return Err(format!("案件源路径不是文件夹: {}", case.source_folder));
    }

    // 2) 扫文件夹(scanner 很快,同步即可)
    let scanned = scan_folder(folder);

    // 3) diff sync,拿统计
    let stats = documents_db::sync_documents_for_case(pool.inner(), &case_id, &scanned)
        .await
        .map_err(db_err)?;

    // 4) 有任何变化 或 DB 里还有 pending 文档 → 后台跑抽取(pipeline 自带重跑 global_extract)
    //
    // 2026-05-25 V0.1.8 加 pending 检测:这样老板手工把 failed 重置成 pending 后,
    // 点一下「刷新源文件」就能触发重抽,不用加新按钮。
    let pending_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents \
         WHERE case_id = ? AND deleted_at IS NULL AND extraction_status = 'pending'",
    )
    .bind(&case_id)
    .fetch_one(pool.inner())
    .await
    .unwrap_or(0);
    if stats.added > 0 || stats.updated > 0 || stats.deleted > 0 || pending_count > 0 {
        let documents = documents_db::list_documents_by_case(pool.inner(), &case_id)
            .await
            .map_err(db_err)?;
        pipeline::spawn_extraction(
            app.clone(),
            pool.inner().clone(),
            case_id.clone(),
            documents,
            true,
        );
    }

    Ok(stats)
}

// ─────────────────────────── 法院短信处理(V0.3) ───────────────────────────
// 一张网(zxfw.court.gov.cn)送达短信 → 解析 → 拉文书 → 匹配案件 → 下载进 source_folder
// → 复用刷新管线抽取上看板。纯逻辑在 court_sms 模块,这里做命令编排。

#[derive(serde::Serialize)]
struct CourtSmsDocBrief {
    name: String,
    ext: String,
}

#[derive(serde::Serialize)]
struct CourtSmsPreview {
    court: Option<String>,
    case_no: Option<String>,
    has_link: bool,
    /// 透传给 ingest(只传链接参数,**不传时效性的 wjlj**)
    link: Option<court_sms::ZxfwLink>,
    docs: Vec<CourtSmsDocBrief>,
    matched_case_id: Option<String>,
    matched_case_name: Option<String>,
    note: Option<String>,
    /// 2026-06-11 反馈修复:案号没匹配上时(典型:短信是执行案号,库里存诉讼案号),
    /// 按当事人姓名反向匹配的候选案件(命中名多的在前)。前端预选第一个并让用户确认。
    #[serde(default)]
    name_matches: Vec<CourtSmsNameMatch>,
}

/// 按当事人姓名匹配到的候选案件。
#[derive(serde::Serialize)]
struct CourtSmsNameMatch {
    case_id: String,
    case_name: String,
    /// 在短信原文里命中的当事人姓名(给用户看"凭什么匹配上")
    matched_names: Vec<String>,
}

#[derive(serde::Serialize)]
struct CourtSmsIngestResult {
    downloaded: Vec<String>,
    skipped: Vec<String>,
    sync: documents_db::SyncStats,
}

/// 案号归一化后比对 `agg_case_no` **以及 case_instances 全部审级案号**(2026-06-11:
/// 短信里是一审案号、库里 agg 已是二审时也要能匹配),返回首个匹配案件 (id, 展示名)。
async fn find_case_by_case_no(
    pool: &SqlitePool,
    case_no: &str,
) -> (Option<String>, Option<String>) {
    let target = court_sms::normalize_case_no(case_no);
    if target.is_empty() {
        return (None, None);
    }
    let cases = match cases_db::list_cases(pool).await {
        Ok(c) => c,
        Err(_) => return (None, None),
    };
    for c in &cases {
        if let Some(no) = &c.agg_case_no {
            if court_sms::normalize_case_no(no) == target {
                let name = c.agg_cause.clone().unwrap_or_else(|| c.name.clone());
                return (Some(c.id.clone()), Some(name));
            }
        }
    }
    // 审级表兜底:任何审级的案号命中都算(仲裁案号/一审案号/二审案号)
    let inst_rows: Vec<(String, String)> =
        sqlx::query_as("SELECT case_id, case_no FROM case_instances WHERE case_no IS NOT NULL")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    for (cid, no) in inst_rows {
        if court_sms::normalize_case_no(&no) == target {
            if let Some(c) = cases.iter().find(|c| c.id == cid) {
                let name = c.agg_cause.clone().unwrap_or_else(|| c.name.clone());
                return (Some(cid), Some(name));
            }
        }
    }
    (None, None)
}

/// 2026-06-11 反馈修复:按**当事人姓名**反向匹配案件 —— 拿每个案件的当事人名
/// (agg_plaintiffs / agg_defendants / agg_party_contacts)去短信原文里做包含检查。
/// 典型场景:执行立案短信只有执行案号「(2026)苏0205执2376号」,库里存的是诉讼案号,
/// 案号匹配必失败;但短信里有当事人姓名,反向包含即可命中。
/// 返回按命中名数量降序的候选(全部返回,由前端预选第一个 + 用户确认)。
fn find_cases_by_party_names(cases: &[cases_db::Case], sms_text: &str) -> Vec<CourtSmsNameMatch> {
    let parse_names = |json: &Option<String>| -> Vec<String> {
        let Some(s) = json else { return vec![] };
        serde_json::from_str::<Vec<String>>(s).unwrap_or_default()
    };
    let mut out: Vec<CourtSmsNameMatch> = Vec::new();
    for c in cases {
        // 示例案件不参与匹配(张三/李四撞名真实短信会闹笑话)
        if c.source_folder == "__DEMO__" {
            continue;
        }
        let mut names: Vec<String> = vec![];
        names.extend(parse_names(&c.agg_plaintiffs));
        names.extend(parse_names(&c.agg_defendants));
        // party_contacts JSON: [{name,role,...}]
        if let Some(s) = &c.agg_party_contacts {
            if let Ok(serde_json::Value::Array(arr)) = serde_json::from_str(s) {
                for item in arr {
                    if let Some(n) = item.get("name").and_then(|v| v.as_str()) {
                        names.push(n.to_string());
                    }
                }
            }
        }
        let mut matched: Vec<String> = names
            .into_iter()
            .map(|n| n.trim().to_string())
            .filter(|n| n.chars().count() >= 2 && sms_text.contains(n.as_str()))
            .collect();
        matched.sort();
        matched.dedup();
        if !matched.is_empty() {
            out.push(CourtSmsNameMatch {
                case_id: c.id.clone(),
                case_name: c.agg_cause.clone().unwrap_or_else(|| c.name.clone()),
                matched_names: matched,
            });
        }
    }
    // 命中名多的在前(两个名都中的比只中一个的可信)
    out.sort_by_key(|m| std::cmp::Reverse(m.matched_names.len()));
    out
}

fn sanitize_filename(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\n' | '\r' | '\t' => '_',
            _ => c,
        })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "court_document".into()
    } else {
        trimmed.chars().take(80).collect()
    }
}

/// 在 `folder` 下取一个不与现有文件冲突的路径:`base.ext`,占用则 `base (2).ext`…
fn unique_path(folder: &Path, base: &str, ext: &str) -> std::path::PathBuf {
    let first = folder.join(format!("{}.{}", base, ext));
    if !first.exists() {
        return first;
    }
    for n in 2..1000 {
        let p = folder.join(format!("{} ({}).{}", base, n, ext));
        if !p.exists() {
            return p;
        }
    }
    first
}

/// 预览:解析短信 → 拉文书名列表 → 匹配在办案件。**不下载、不落盘**(无副作用)。
#[tauri::command]
async fn preview_court_sms(
    pool: tauri::State<'_, SqlitePool>,
    sms_text: String,
) -> Result<CourtSmsPreview, String> {
    let parsed = court_sms::parse_sms(&sms_text);
    let Some(link) = parsed.link.clone() else {
        return Ok(CourtSmsPreview {
            court: parsed.court,
            case_no: parsed.case_no,
            has_link: false,
            link: None,
            docs: vec![],
            matched_case_id: None,
            matched_case_name: None,
            note: Some(
                "没识别到「人民法院在线服务/一张网」(zxfw.court.gov.cn)送达链接。\
                 目前只支持一张网;其它平台(江苏微解纷等)暂不支持自动下载。"
                    .into(),
            ),
            name_matches: vec![],
        });
    };
    let docs = court_sms::fetch_zxfw_doc_list(&link).await?;
    let (matched_id, matched_name) = match &parsed.case_no {
        Some(cn) => find_case_by_case_no(pool.inner(), cn).await,
        None => (None, None),
    };
    let docs_out: Vec<CourtSmsDocBrief> = docs
        .iter()
        .map(|d| CourtSmsDocBrief {
            name: d.name.clone(),
            ext: d.ext.clone(),
        })
        .collect();
    // 案号没匹配上 → 按当事人姓名反向匹配(短信是执行案号时的兜底)
    let name_matches = if matched_id.is_none() {
        match cases_db::list_cases(pool.inner()).await {
            Ok(cases) => find_cases_by_party_names(&cases, &sms_text),
            Err(_) => vec![],
        }
    } else {
        vec![]
    };
    let note = if matched_id.is_some() {
        None
    } else if !name_matches.is_empty() {
        Some(format!(
            "案号没直接匹配上,按当事人姓名「{}」匹配到候选案件,请确认是不是下面选中的案件。",
            name_matches[0].matched_names.join("、")
        ))
    } else {
        Some("没自动匹配到在办案件,请手动选择要归档到哪个案件。".into())
    };
    Ok(CourtSmsPreview {
        court: parsed
            .court
            .or_else(|| docs.first().and_then(|d| d.court.clone())),
        case_no: parsed.case_no,
        has_link: true,
        link: Some(link),
        docs: docs_out,
        matched_case_id: matched_id,
        matched_case_name: matched_name,
        note,
        name_matches,
    })
}

/// 导入:重新拉新鲜文书列表(wjlj 有时效)→ 下载进案件 source_folder → 复用刷新管线抽取。
#[tauri::command]
async fn ingest_court_sms(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
    link: court_sms::ZxfwLink,
) -> Result<CourtSmsIngestResult, String> {
    let case = cases_db::get_case(pool.inner(), &case_id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| format!("案件不存在: {}", case_id))?;
    let folder = Path::new(&case.source_folder);
    if !folder.is_dir() {
        return Err(format!("案件源文件夹不可用: {}", case.source_folder));
    }
    // wjlj 有时效,这里重新拉一次拿新鲜下载地址
    let docs = court_sms::fetch_zxfw_doc_list(&link).await?;
    if docs.is_empty() {
        return Err("一张网未返回任何文书(链接可能已失效,请重新粘贴最新短信)".into());
    }
    let mut downloaded = vec![];
    let mut skipped = vec![];
    for d in &docs {
        let dest = unique_path(folder, &sanitize_filename(&d.name), &d.ext);
        match court_sms::download_doc(&d.wjlj, &dest).await {
            Ok(_) => downloaded.push(
                dest.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| d.name.clone()),
            ),
            Err(e) => {
                crate::dlog!("court_sms 下载失败 {}: {}", d.name, e);
                skipped.push(format!("{}({})", d.name, e));
            }
        }
    }
    // 复用「刷新源文件」:扫描 → diff → 新 PDF 进 documents(pending)→ 后台抽取 + 重跑画像/看板
    let scanned = scan_folder(folder);
    let sync = documents_db::sync_documents_for_case(pool.inner(), &case_id, &scanned)
        .await
        .map_err(db_err)?;
    let documents = documents_db::list_documents_by_case(pool.inner(), &case_id)
        .await
        .map_err(db_err)?;
    pipeline::spawn_extraction(
        app.clone(),
        pool.inner().clone(),
        case_id.clone(),
        documents,
        true,
    );
    Ok(CourtSmsIngestResult {
        downloaded,
        skipped,
        sync,
    })
}

/// 快递100 凭证(每次读不缓存,改了实时生效)。
fn kuaidi100_creds() -> (String, String) {
    let s = settings::read_settings().unwrap_or_default();
    (
        s.kuaidi100_customer.unwrap_or_default(),
        s.kuaidi100_key.unwrap_or_default(),
    )
}

/// 查询并跟踪一个单号:实时查 → 落本地 express_tracks.json → 返回最新全列表。
/// com=快递公司编码(ems/shunfeng…),com_name=中文名(展示),num=运单号。
#[tauri::command]
async fn query_express(
    com: String,
    com_name: String,
    num: String,
    phone: String,
) -> Result<Vec<express::TrackRecord>, String> {
    let (customer, key) = kuaidi100_creds();
    express::query_and_track(&customer, &key, &com, &com_name, &num, &phone).await
}

/// 列出本地所有跟踪记录(不联网)。
#[tauri::command]
fn list_express_tracks() -> Vec<express::TrackRecord> {
    express::load_tracks()
}

/// 刷新在跟踪的单号(未签收 + 30 天内 + 距上次轮询≥6 小时)。同单号 40 天内重查免费。
#[tauri::command]
async fn refresh_express_tracks() -> Result<Vec<express::TrackRecord>, String> {
    let (customer, key) = kuaidi100_creds();
    if customer.is_empty() || key.is_empty() {
        return Ok(express::load_tracks());
    }
    express::refresh_active(&customer, &key, 6).await
}

/// 删除一个跟踪记录。
#[tauri::command]
fn delete_express_track(num: String) -> Result<Vec<express::TrackRecord>, String> {
    express::delete_track(&num)
}

/// 数据库健康检查:返回表数量 + 数据库文件路径。
#[tauri::command]
async fn db_health(pool: tauri::State<'_, SqlitePool>) -> Result<DbHealth, String> {
    let (table_count,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'")
            .fetch_one(pool.inner())
            .await
            .map_err(|e| format!("查询失败: {}", e))?;

    let (case_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM cases")
        .fetch_one(pool.inner())
        .await
        .map_err(|e| format!("查询案件数失败: {}", e))?;

    Ok(DbHealth {
        ok: true,
        table_count,
        case_count,
        db_path: db::default_db_path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "<unknown>".to_string()),
    })
}

/// sqlx::Error → 前端友好字符串
fn db_err(e: sqlx::Error) -> String {
    format!("数据库错误: {}", e)
}

// ============================================================================
// 案件 AI 助手(case-aware chat)— 2026-05-27 V0.1.13+
// ============================================================================

/// 启动一次案件聊天:边接 SSE 边把 delta `emit` 到 `chat-stream-{message_id}` 频道。
/// 流式完成后入库一对 user/assistant 消息,长输出自动落 artifact。
#[tauri::command]
async fn case_chat(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    registry: tauri::State<'_, chat::ChatCancelRegistry>,
    input: chat::CaseChatInput,
) -> Result<chat::CaseChatResult, String> {
    chat::case_chat_impl(app, pool.inner(), registry.inner(), input).await
}

/// 取案件聊天历史(升序,前端直接渲染)。
#[tauri::command]
async fn list_chat_history(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
    limit: Option<i64>,
) -> Result<Vec<crate::db::chat::ChatMessage>, String> {
    chat::list_chat_history_impl(pool.inner(), &case_id, limit).await
}

/// 取消进行中的 chat。message_id 必须跟 case_chat 入参的 message_id 相同。
#[tauri::command]
fn cancel_chat(registry: tauri::State<'_, chat::ChatCancelRegistry>, message_id: String) -> bool {
    chat::cancel_chat_impl(registry.inner(), &message_id)
}

/// 清空某案件下全部聊天记录(用户主动)。
#[tauri::command]
async fn clear_chat_history(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<u64, String> {
    chat::clear_chat_history_impl(pool.inner(), &case_id).await
}

// ============================================================================
// MCP 数据源接入(智能粘贴识别 + 连接测试)
// ============================================================================

/// 智能粘贴:把平台「接入指南」复制来的配置文本解析成 MCP server 列表。
/// 纯本地确定性解析(JSON / claude mcp add 命令行),不联网、不调 LLM。
// ============================================================================
// 法院一张网在线立案 — 2026-06-15
// ============================================================================

/// 立案任务事件（emit 到前端）。
#[derive(Clone, serde::Serialize)]
struct CourtFilingProgress {
    job_id: String,
    case_id: String,
    phase: String,
    stage: String,
    level: String,
    message: String,
    detail: Option<String>,
    round: Option<i64>,
    task_id: Option<String>,
    image_base64: Option<String>,
    timing: Option<serde_json::Value>,
}

/// 验证码请求事件（弹窗用）。
#[derive(Clone, serde::Serialize)]
struct CourtFilingCaptcha {
    job_id: String,
    case_id: String,
    task_id: String,
    round: i64,
    image_base64: String,
    timeout_sec: i64,
}

/// 法院立案 CLI 的内置资源路径。
const COURT_FILING_CLI_RESOURCE: &str = "standalone/court_filing_cli";

fn bundled_court_filing_cli_path(app: &tauri::AppHandle) -> Option<String> {
    if let Ok(path) = app
        .path()
        .resolve(COURT_FILING_CLI_RESOURCE, BaseDirectory::Resource)
    {
        if path.join("__main__.py").exists() {
            return Some(path.to_string_lossy().to_string());
        }
    }

    let dev_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")))
        .join(COURT_FILING_CLI_RESOURCE);
    if dev_path.join("__main__.py").exists() {
        return Some(dev_path.to_string_lossy().to_string());
    }

    None
}

fn resolve_court_filing_cli_path(app: &tauri::AppHandle, configured: Option<String>) -> String {
    let configured = configured
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(path) = configured {
        return path;
    }

    bundled_court_filing_cli_path(app).unwrap_or_else(|| COURT_FILING_CLI_RESOURCE.to_string())
}

#[derive(Clone, serde::Serialize)]
struct CourtFilingMaterialCandidate {
    doc_id: String,
    source_path: String,
    filename: String,
    category: Option<String>,
    stage: Option<String>,
    size_bytes: i64,
    slot: i32,
    slot_label: String,
    confidence: i32,
    included: bool,
    reasons: Vec<String>,
    warnings: Vec<String>,
}

struct CourtFilingSourceDoc {
    id: String,
    source_path: String,
    filename: String,
    category: Option<String>,
    stage: Option<String>,
    mime_type: Option<String>,
    size_bytes: i64,
    missing: bool,
}

/// 通用材料包匹配：只处理用户本次选择的材料文件夹。
///
/// 立案材料必须由用户先放进一个独立文件夹。后端只上传能明确识别到槽位的 PDF，
/// 识别不出来的文件保留在预检报告里，避免把法院通知书、传票等旧材料误传。
fn build_court_filing_materials(
    docs: &[CourtFilingSourceDoc],
    material_folder: &str,
    filing_type: &str,
) -> (serde_json::Value, serde_json::Value) {
    use serde_json::{Map, Value};

    let slot_keywords: Vec<(i32, &str, Vec<&str>)> = if filing_type == "execution" {
        vec![
            (
                0,
                "执行申请书",
                vec!["执行申请书", "申请执行", "强制执行申请"],
            ),
            (
                1,
                "执行依据",
                vec!["执行依据", "判决书", "裁定书", "调解书", "仲裁裁决"],
            ),
            (2, "授权委托手续", vec!["授权", "委托", "律所函", "所函"]),
            (
                3,
                "主体资格材料",
                vec![
                    "身份证明",
                    "身份证",
                    "营业执照",
                    "身份信息",
                    "统一社会信用代码",
                ],
            ),
            (4, "送达地址确认", vec!["送达地址", "地址确认"]),
        ]
    } else {
        vec![
            (0, "起诉状", vec!["起诉状", "诉状", "民事起诉"]),
            (
                1,
                "主体资格材料",
                vec![
                    "身份证明",
                    "身份证",
                    "营业执照",
                    "身份信息",
                    "统一社会信用代码",
                ],
            ),
            (2, "授权委托手续", vec!["授权", "委托", "律所函", "所函"]),
            (3, "证据材料", vec!["证据", "证据目录", "证据材料", "附件"]),
            (4, "送达地址确认", vec!["送达地址", "地址确认"]),
        ]
    };

    let mut map: Map<String, Value> = Map::new();
    let mut candidates: Vec<CourtFilingMaterialCandidate> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let pdf_docs: Vec<&CourtFilingSourceDoc> = docs
        .iter()
        .filter(|doc| {
            doc.source_path.to_lowercase().ends_with(".pdf")
                || doc.mime_type.as_deref().unwrap_or("").contains("pdf")
        })
        .collect();

    for doc in &pdf_docs {
        let signal = format!(
            "{} {} {} {}",
            doc.category.as_deref().unwrap_or(""),
            doc.stage.as_deref().unwrap_or(""),
            doc.filename,
            doc.source_path
        );
        let signal_lower = signal.to_lowercase();

        let mut best_slot: Option<(i32, &str, i32, Vec<String>)> = None;
        for (slot, label, keywords) in &slot_keywords {
            let mut score = 0;
            let mut reasons = Vec::new();
            for kw in keywords {
                let kw_lower = kw.to_lowercase();
                if signal_lower.contains(&kw_lower) {
                    let weight = if doc
                        .category
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&kw_lower)
                    {
                        45
                    } else if doc.filename.to_lowercase().contains(&kw_lower) {
                        35
                    } else if doc
                        .stage
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&kw_lower)
                    {
                        25
                    } else {
                        12
                    };
                    score += weight;
                    reasons.push(format!("命中关键词「{}」", kw));
                }
            }
            if best_slot
                .as_ref()
                .map(|(_, _, best, _)| score > *best)
                .unwrap_or(true)
            {
                best_slot = Some((*slot, *label, score, reasons));
            }
        }

        let (slot, slot_label, score, reasons) =
            best_slot.filter(|(_, _, score, _)| *score > 0).unwrap_or((
                -1,
                "未识别材料",
                0,
                vec!["没有命中立案材料关键词，未自动上传".to_string()],
            ));
        let mut doc_warnings = Vec::new();
        let file_exists = std::path::Path::new(&doc.source_path).is_file();
        if slot < 0 {
            doc_warnings.push("未识别为必备立案材料，已跳过上传".to_string());
        } else if score < 30 {
            doc_warnings.push("材料类型置信度偏低，必要时需人工核对".to_string());
        }
        if doc.size_bytes <= 0 {
            doc_warnings.push("文件大小异常，可能无法上传".to_string());
        }
        if doc.missing || !file_exists {
            doc_warnings.push("源文件已失联，上传前需要重新选择文件".to_string());
        }
        if !doc_warnings.is_empty() {
            warnings.push(format!("{}：{}", doc.filename, doc_warnings.join("；")));
        }

        let included = slot >= 0 && file_exists && !doc.missing && doc.size_bytes > 0;
        if included {
            let key = slot.to_string();
            let entry = map.entry(key).or_insert_with(|| Value::Array(vec![]));
            if let Value::Array(arr) = entry {
                arr.push(Value::Array(vec![
                    Value::String(doc.source_path.clone()),
                    Value::String(doc.filename.clone()),
                ]));
            }
        }
        candidates.push(CourtFilingMaterialCandidate {
            doc_id: doc.id.clone(),
            source_path: doc.source_path.clone(),
            filename: doc.filename.clone(),
            category: doc.category.clone(),
            stage: doc.stage.clone(),
            size_bytes: doc.size_bytes,
            slot,
            slot_label: slot_label.to_string(),
            confidence: score.min(100),
            included,
            reasons,
            warnings: doc_warnings,
        });
    }

    let required_slots: Vec<i32> = if filing_type == "execution" {
        vec![0, 1, 3]
    } else {
        vec![0, 1, 3]
    };
    let missing_required: Vec<Value> = required_slots
        .into_iter()
        .filter(|slot| !map.contains_key(&slot.to_string()))
        .map(|slot| {
            let label = slot_keywords
                .iter()
                .find(|(s, _, _)| *s == slot)
                .map(|(_, label, _)| *label)
                .unwrap_or("必备材料");
            Value::String(label.to_string())
        })
        .collect();
    if pdf_docs.is_empty() {
        warnings.push("本案没有可用于立案上传的 PDF，请先导入或生成 PDF 材料".to_string());
    }
    if !missing_required.is_empty() {
        let labels: Vec<String> = missing_required
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
        warnings.push(format!("疑似缺少必备材料：{}", labels.join("、")));
    }

    let included_count = candidates.iter().filter(|c| c.included).count();
    let report = serde_json::json!({
        "filing_type": filing_type,
        "material_folder": material_folder,
        "generated_at": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        "total_documents": docs.len(),
        "pdf_documents": pdf_docs.len(),
        "matched_documents": included_count,
        "scanned_pdf_documents": candidates.len(),
        "missing_required": missing_required,
        "warnings": warnings,
        "materials": candidates,
        "slots": slot_keywords.iter().map(|(slot, label, _)| {
            serde_json::json!({
                "slot": slot,
                "label": label,
                "count": map.get(&slot.to_string()).and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
            })
        }).collect::<Vec<_>>(),
    });

    (Value::Object(map), report)
}

fn collect_pdf_files(
    dir: &std::path::Path,
    out: &mut Vec<CourtFilingSourceDoc>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("读取材料文件夹失败：{}", e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取材料文件失败：{}", e))?;
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("pdf") {
            continue;
        }
        let meta = std::fs::metadata(&path).map_err(|e| format!("读取 PDF 信息失败：{}", e))?;
        let filename = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("未命名.pdf")
            .to_string();
        out.push(CourtFilingSourceDoc {
            id: path.to_string_lossy().to_string(),
            source_path: path.to_string_lossy().to_string(),
            filename,
            category: None,
            stage: Some("立案".to_string()),
            mime_type: Some("application/pdf".to_string()),
            size_bytes: meta.len() as i64,
            missing: false,
        });
    }
    Ok(())
}

fn scan_material_folder(material_folder: &str) -> Result<Vec<CourtFilingSourceDoc>, String> {
    let dir = std::path::Path::new(material_folder);
    if !dir.exists() || !dir.is_dir() {
        return Err("请选择一个有效的立案材料文件夹。".to_string());
    }
    let mut docs = Vec::new();
    collect_pdf_files(dir, &mut docs)?;
    if docs.is_empty() {
        return Err("这个材料文件夹里没有 PDF，不能开始立案。".to_string());
    }
    Ok(docs)
}

async fn append_jsonl(path: &std::path::Path, value: &serde_json::Value) {
    use tokio::io::AsyncWriteExt;
    if let Ok(mut file) = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
    {
        if let Ok(line) = serde_json::to_string(value) {
            let _ = file.write_all(line.as_bytes()).await;
            let _ = file.write_all(b"\n").await;
        }
    }
}

fn court_filing_stage_label(stage: &str) -> &'static str {
    match stage {
        "login.failed" => "登录法院平台",
        "playwright.step.open_case_type" => "进入立案页面",
        "playwright.step.select_court" => "选择受理法院",
        "playwright.step.read_notice" => "确认立案须知",
        "playwright.step.select_cause" => "选择案由",
        "playwright.step.upload_materials" => "上传立案材料",
        "playwright.step.fill_case_info" => "填写当事人信息",
        "playwright.step.next" => "进入预览页",
        "playwright.failed" => "法院页面办理",
        "captcha.required" => "输入验证码",
        _ => "办理立案",
    }
}

fn court_filing_user_error(stage: &str, message: &str, detail: Option<&str>) -> String {
    let raw = [message, detail.unwrap_or("")]
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join("；");
    let step = if raw.contains("原告信息") || raw.contains("当事人") || raw.contains("标的金额")
    {
        "填写当事人信息"
    } else {
        court_filing_stage_label(stage)
    };
    let reason = if raw.contains("无法找到法院") {
        "没有找到对应法院，请先确认案件档案里的受理法院是否正确。"
    } else if raw.contains("省份为空") || raw.contains("判断所属省份") {
        "没有判断出法院所属省份，请把法院名称补充完整。"
    } else if raw.contains("验证码") {
        "验证码没有完成或已超时。"
    } else if raw.contains("登录") {
        "法院平台登录失败，请检查账号、密码或验证码。"
    } else if raw.contains("原告信息") || raw.contains("当事人") || raw.contains("标的金额")
    {
        "法院页面没有出现当事人信息填写区，通常是材料上传后没有进入正确页面。请先检查材料文件夹是否只放本次立案必需 PDF。"
    } else if raw.contains("材料") || raw.contains("上传") {
        "材料上传没有完成，请检查材料文件夹里的 PDF 是否齐全、是否能打开。"
    } else {
        "法院页面没有完成这一步。"
    };
    format!("失败步骤：{}。{}", step, reason)
}

#[derive(Debug, Clone, serde::Serialize)]
struct CourtRegion {
    province: String,
    city: String,
    district: String,
    confidence: i32,
    reason: String,
}

fn municipality_districts(province: &str) -> &'static [&'static str] {
    match province {
        "北京市" => &[
            "东城区",
            "西城区",
            "朝阳区",
            "丰台区",
            "石景山区",
            "海淀区",
            "门头沟区",
            "房山区",
            "通州区",
            "顺义区",
            "昌平区",
            "大兴区",
            "怀柔区",
            "平谷区",
            "密云区",
            "延庆区",
        ],
        "天津市" => &[
            "和平区",
            "河东区",
            "河西区",
            "南开区",
            "河北区",
            "红桥区",
            "东丽区",
            "西青区",
            "津南区",
            "北辰区",
            "武清区",
            "宝坻区",
            "滨海新区",
            "宁河区",
            "静海区",
            "蓟州区",
        ],
        "上海市" => &[
            "黄浦区",
            "徐汇区",
            "长宁区",
            "静安区",
            "普陀区",
            "虹口区",
            "杨浦区",
            "闵行区",
            "宝山区",
            "嘉定区",
            "浦东新区",
            "金山区",
            "松江区",
            "青浦区",
            "奉贤区",
            "崇明区",
        ],
        "重庆市" => &[
            "万州区",
            "涪陵区",
            "渝中区",
            "大渡口区",
            "江北区",
            "沙坪坝区",
            "九龙坡区",
            "南岸区",
            "北碚区",
            "綦江区",
            "大足区",
            "渝北区",
            "巴南区",
            "黔江区",
            "长寿区",
            "江津区",
            "合川区",
            "永川区",
            "南川区",
            "璧山区",
            "铜梁区",
            "潼南区",
            "荣昌区",
            "开州区",
            "梁平区",
            "武隆区",
        ],
        _ => &[],
    }
}

fn county_region_hints(
    name: &str,
) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    let _ = name;
    None
}

fn infer_court_region(court_name: &str) -> CourtRegion {
    let provinces = [
        ("北京市", &["北京"][..]),
        ("天津市", &["天津"][..]),
        (
            "河北省",
            &[
                "河北",
                "石家庄",
                "唐山",
                "秦皇岛",
                "邯郸",
                "邢台",
                "保定",
                "张家口",
                "承德",
                "沧州",
                "廊坊",
                "衡水",
            ][..],
        ),
        (
            "山西省",
            &[
                "山西", "太原", "大同", "阳泉", "长治", "晋城", "朔州", "晋中", "运城", "忻州",
                "临汾", "吕梁",
            ][..],
        ),
        (
            "内蒙古自治区",
            &[
                "内蒙古",
                "呼和浩特",
                "包头",
                "乌海",
                "赤峰",
                "通辽",
                "鄂尔多斯",
                "呼伦贝尔",
                "巴彦淖尔",
                "乌兰察布",
                "兴安盟",
                "锡林郭勒",
                "阿拉善",
            ][..],
        ),
        (
            "辽宁省",
            &[
                "辽宁",
                "沈阳",
                "大连",
                "鞍山",
                "抚顺",
                "本溪",
                "丹东",
                "锦州",
                "营口",
                "阜新",
                "辽阳",
                "盘锦",
                "铁岭",
                "朝阳",
                "葫芦岛",
            ][..],
        ),
        (
            "吉林省",
            &[
                "吉林", "长春", "四平", "辽源", "通化", "白山", "松原", "白城", "延边",
            ][..],
        ),
        (
            "黑龙江省",
            &[
                "黑龙江",
                "哈尔滨",
                "齐齐哈尔",
                "鸡西",
                "鹤岗",
                "双鸭山",
                "大庆",
                "伊春",
                "佳木斯",
                "七台河",
                "牡丹江",
                "黑河",
                "绥化",
                "大兴安岭",
            ][..],
        ),
        ("上海市", &["上海"][..]),
        (
            "江苏省",
            &[
                "江苏",
                "南京",
                "无锡",
                "徐州",
                "常州",
                "苏州",
                "南通",
                "连云港",
                "淮安",
                "盐城",
                "扬州",
                "镇江",
                "泰州",
                "宿迁",
            ][..],
        ),
        (
            "浙江省",
            &[
                "浙江", "杭州", "宁波", "嘉兴", "湖州", "绍兴", "金华", "衢州", "舟山", "台州",
                "丽水",
            ][..],
        ),
        (
            "安徽省",
            &[
                "安徽",
                "合肥",
                "芜湖",
                "蚌埠",
                "淮南",
                "马鞍山",
                "淮北",
                "铜陵",
                "安庆",
                "黄山",
                "滁州",
                "阜阳",
                "宿州",
                "六安",
                "亳州",
                "池州",
                "宣城",
            ][..],
        ),
        (
            "福建省",
            &[
                "福建", "福州", "厦门", "莆田", "三明", "泉州", "漳州", "南平", "龙岩", "宁德",
            ][..],
        ),
        (
            "江西省",
            &[
                "江西",
                "南昌",
                "景德镇",
                "萍乡",
                "九江",
                "新余",
                "鹰潭",
                "赣州",
                "吉安",
                "宜春",
                "抚州",
                "上饶",
            ][..],
        ),
        (
            "山东省",
            &[
                "山东", "济南", "青岛", "淄博", "枣庄", "东营", "烟台", "潍坊", "济宁", "泰安",
                "威海", "日照", "临沂", "德州", "聊城", "滨州", "菏泽",
            ][..],
        ),
        (
            "河南省",
            &[
                "河南",
                "郑州",
                "开封",
                "洛阳",
                "平顶山",
                "安阳",
                "鹤壁",
                "新乡",
                "焦作",
                "濮阳",
                "许昌",
                "漯河",
                "三门峡",
                "南阳",
                "商丘",
                "信阳",
                "周口",
                "驻马店",
                "济源",
            ][..],
        ),
        (
            "湖北省",
            &[
                "湖北", "武汉", "黄石", "十堰", "宜昌", "襄阳", "鄂州", "荆门", "孝感", "荆州",
                "黄冈", "咸宁", "随州", "恩施",
            ][..],
        ),
        (
            "湖南省",
            &[
                "湖南",
                "长沙",
                "株洲",
                "湘潭",
                "衡阳",
                "邵阳",
                "岳阳",
                "常德",
                "张家界",
                "益阳",
                "郴州",
                "永州",
                "怀化",
                "娄底",
                "湘西",
            ][..],
        ),
        (
            "广东省",
            &[
                "广东", "广州", "深圳", "珠海", "汕头", "佛山", "韶关", "河源", "梅州", "惠州",
                "汕尾", "东莞", "中山", "江门", "阳江", "湛江", "茂名", "肇庆", "清远", "潮州",
                "揭阳", "云浮",
            ][..],
        ),
        (
            "广西壮族自治区",
            &[
                "广西",
                "南宁",
                "柳州",
                "桂林",
                "梧州",
                "北海",
                "防城港",
                "钦州",
                "贵港",
                "玉林",
                "百色",
                "贺州",
                "河池",
                "来宾",
                "崇左",
            ][..],
        ),
        ("海南省", &["海南", "海口", "三亚", "三沙", "儋州"][..]),
        ("重庆市", &["重庆"][..]),
        (
            "四川省",
            &[
                "四川",
                "成都",
                "自贡",
                "攀枝花",
                "泸州",
                "德阳",
                "绵阳",
                "广元",
                "遂宁",
                "内江",
                "乐山",
                "南充",
                "眉山",
                "宜宾",
                "广安",
                "达州",
                "雅安",
                "巴中",
                "资阳",
                "阿坝",
                "甘孜",
                "凉山",
            ][..],
        ),
        (
            "贵州省",
            &[
                "贵州",
                "贵阳",
                "六盘水",
                "遵义",
                "安顺",
                "毕节",
                "铜仁",
                "黔西南",
                "黔东南",
                "黔南",
            ][..],
        ),
        (
            "云南省",
            &[
                "云南",
                "昆明",
                "曲靖",
                "玉溪",
                "保山",
                "昭通",
                "丽江",
                "普洱",
                "临沧",
                "楚雄",
                "红河",
                "文山",
                "西双版纳",
                "大理",
                "德宏",
                "怒江",
                "迪庆",
            ][..],
        ),
        (
            "西藏自治区",
            &[
                "西藏",
                "拉萨",
                "日喀则",
                "昌都",
                "林芝",
                "山南",
                "那曲",
                "阿里",
            ][..],
        ),
        (
            "陕西省",
            &[
                "陕西", "西安", "铜川", "宝鸡", "咸阳", "渭南", "延安", "汉中", "榆林", "安康",
                "商洛",
            ][..],
        ),
        (
            "甘肃省",
            &[
                "甘肃",
                "兰州",
                "嘉峪关",
                "金昌",
                "白银",
                "天水",
                "武威",
                "张掖",
                "平凉",
                "酒泉",
                "庆阳",
                "定西",
                "陇南",
                "临夏",
                "甘南",
            ][..],
        ),
        (
            "青海省",
            &[
                "青海", "西宁", "海东", "海北", "黄南", "海南", "果洛", "玉树", "海西",
            ][..],
        ),
        (
            "宁夏回族自治区",
            &["宁夏", "银川", "石嘴山", "吴忠", "固原", "中卫"][..],
        ),
        (
            "新疆维吾尔自治区",
            &[
                "新疆",
                "乌鲁木齐",
                "克拉玛依",
                "吐鲁番",
                "哈密",
                "昌吉",
                "博尔塔拉",
                "巴音郭楞",
                "阿克苏",
                "克孜勒苏",
                "喀什",
                "和田",
                "伊犁",
                "塔城",
                "阿勒泰",
            ][..],
        ),
    ];

    let mut province = String::new();
    let mut city = String::new();
    let mut confidence = 0;
    let mut reason = String::new();

    if let Some((_, p, c, d)) = county_region_hints(court_name) {
        return CourtRegion {
            province: p.to_string(),
            city: c.to_string(),
            district: d.to_string(),
            confidence: 95,
            reason: format!("法院名称命中县区地名「{}」", d),
        };
    }

    for (p, hints) in provinces {
        if court_name.contains(p) || court_name.contains(p.trim_end_matches('省')) {
            province = p.to_string();
            confidence = 100;
            reason = format!("法院名称包含省级地名「{}」", p);
            break;
        }
        if let Some(hit) = hints.iter().find(|hint| court_name.contains(**hint)) {
            province = p.to_string();
            if *hit != p.trim_end_matches('省') {
                city = if hit.ends_with('盟') || hit.ends_with('州') || hit.ends_with("地区") {
                    hit.to_string()
                } else {
                    format!("{}市", hit)
                };
            }
            confidence = 80;
            reason = format!("法院名称命中地市关键词「{}」", hit);
            break;
        }
    }

    if ["北京市", "天津市", "上海市", "重庆市"].contains(&province.as_str()) {
        city = province.clone();
    }

    if city.is_empty() {
        if let Some(pos) = court_name.find('市') {
            city = court_name[..=pos].to_string();
        }
    }

    let mut district = String::new();
    if !province.is_empty() && city == province {
        for d in municipality_districts(&province) {
            if court_name.contains(d) {
                district = d.to_string();
                break;
            }
        }
    } else if let Some(city_pos) = court_name.find('市') {
        let after_city = &court_name[city_pos + '市'.len_utf8()..];
        for suffix in ["区", "县", "市"] {
            if let Some(pos) = after_city.find(suffix) {
                district = after_city[..=pos].to_string();
                break;
            }
        }
    }

    CourtRegion {
        province,
        city,
        district,
        confidence,
        reason,
    }
}

/// 从委托手续 PDF OCR 提取代理人信息（姓名、执业证号、电话、律所）。
/// 使用 MinerU batch API 上传 PDF → OCR → 解析文本。
async fn extract_agents_from_pdf(
    pdf_path: &str,
    settings: &crate::settings::Settings,
) -> Result<Vec<serde_json::Value>, String> {
    let api_key = settings
        .mineru_api_key
        .clone()
        .ok_or_else(|| "未配置 MinerU API Key".to_string())?;

    let client = reqwest::Client::new();
    let file_name = std::path::Path::new(pdf_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    // 1. 获取上传 URL
    let batch_resp = client
        .post("https://mineru.net/api/v4/file-urls/batch")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&serde_json::json!({
            "files": [{"name": file_name, "data_id": uuid::Uuid::new_v4().to_string()}],
            "model_version": "vlm"
        }))
        .send()
        .await
        .map_err(|e| format!("MinerU batch 请求失败: {}", e))?;

    let batch_result: serde_json::Value = batch_resp
        .json()
        .await
        .map_err(|e| format!("解析 MinerU batch 响应失败: {}", e))?;

    if batch_result.get("code").and_then(|v| v.as_i64()) != Some(0) {
        return Err(format!(
            "MinerU batch 失败: {}",
            batch_result
                .get("msg")
                .unwrap_or(&serde_json::json!("未知错误"))
        ));
    }

    let batch_id = batch_result["data"]["batch_id"]
        .as_str()
        .ok_or_else(|| "batch_id 缺失".to_string())?
        .to_string();
    let upload_url = batch_result["data"]["file_urls"][0]
        .as_str()
        .ok_or_else(|| "upload_url 缺失".to_string())?
        .to_string();

    // 2. 上传文件
    let file_bytes = tokio::fs::read(pdf_path)
        .await
        .map_err(|e| format!("读取 PDF 失败: {}", e))?;
    client
        .put(&upload_url)
        .body(file_bytes)
        .send()
        .await
        .map_err(|e| format!("上传 PDF 失败: {}", e))?;

    // 3. 轮询结果（最多 60 秒）
    for _ in 0..20 {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let result_resp = client
            .get(format!(
                "https://mineru.net/api/v4/extract-results/batch/{}",
                batch_id
            ))
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
            .map_err(|e| format!("轮询 MinerU 结果失败: {}", e))?;

        let result: serde_json::Value = result_resp
            .json()
            .await
            .map_err(|e| format!("解析 MinerU 结果失败: {}", e))?;

        let extract = result
            .get("data")
            .and_then(|d| d.get("extract_result"))
            .and_then(|e| e.as_array())
            .and_then(|a| a.first());

        let state = extract
            .and_then(|e| e.get("state"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        if state == "done" {
            let zip_url = extract
                .and_then(|e| e.get("full_zip_url"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "zip_url 缺失".to_string())?;

            // 4. 下载 ZIP 并提取文本
            let zip_bytes = client
                .get(zip_url)
                .send()
                .await
                .map_err(|e| format!("下载 ZIP 失败: {}", e))?
                .bytes()
                .await
                .map_err(|e| format!("读取 ZIP 失败: {}", e))?;

            let mut text = String::new();
            let reader = std::io::Cursor::new(zip_bytes.to_vec());
            if let Ok(mut archive) = zip::ZipArchive::new(reader) {
                for i in 0..archive.len() {
                    if let Ok(mut file) = archive.by_index(i) {
                        if file.name().ends_with(".md") {
                            let mut content = String::new();
                            if std::io::Read::read_to_string(&mut file, &mut content).is_ok() {
                                text.push_str(&content);
                            }
                        }
                    }
                }
            }

            // 5. 从文本提取代理人信息
            return Ok(parse_agents_from_ocr_text(&text));
        }
    }

    Err("MinerU OCR 超时（60秒）".to_string())
}

/// 从 OCR 文本里提取代理人信息。
/// 匹配模式：律师：XXX 执业证号码：XXX，联系电话：XXX + 身份证号
fn parse_agents_from_ocr_text(text: &str) -> Vec<serde_json::Value> {
    let mut agents = Vec::new();

    // 匹配 "律师：XXX 执业证号码/执业证号：XXX，联系电话：XXX"
    let re = regex::Regex::new(
        r"(?m)律师[：:]\s*(\S+?)[\s　]+执业证号[码]?[：:]\s*(\d+)[\s,，]+联系电话[：:]\s*(1\d{10})",
    )
    .unwrap();

    // 提取所有身份证号（顺序对应持证人顺序）
    let id_re = regex::Regex::new(r"身份证号\s*(\d{17}[\dXx])").unwrap();
    let id_numbers: Vec<String> = id_re
        .captures_iter(text)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect();

    for (i, cap) in re.captures_iter(text).enumerate() {
        let name = cap.get(1).map(|m| m.as_str()).unwrap_or("").to_string();
        let bar_number = cap.get(2).map(|m| m.as_str()).unwrap_or("").to_string();
        let phone = cap.get(3).map(|m| m.as_str()).unwrap_or("").to_string();

        // 身份证号按顺序匹配
        let id_number = id_numbers.get(i).cloned().unwrap_or_default();

        // 提取律所名
        let law_firm = extract_law_firm(text, &name);

        agents.push(serde_json::json!({
            "name": name,
            "bar_number": bar_number,
            "phone": phone,
            "law_firm": law_firm,
            "id_number": id_number,
            "address": "",
        }));
    }

    agents
}

/// 从 OCR 文本里提取律所名。
fn extract_law_firm(text: &str, _agent_name: &str) -> String {
    // 匹配 "执业机构 XXX律师事务所" 或 "受托人：... XXX律师事务所"
    let patterns = [
        regex::Regex::new(r"执业机构\s*[：:]?\s*(\S+律师事务所)").unwrap(),
        regex::Regex::new(r"受托人[：:]\s*\S+\s*\S+\s*律师\s*(\S+律师事务所)").unwrap(),
    ];
    for re in &patterns {
        if let Some(cap) = re.captures(text) {
            return cap
                .get(1)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
        }
    }
    String::new()
}

/// 启动一次法院在线立案：读案件数据 → 组装 JSON → spawn CLI → 流式读 stdout。
/// 前端通过 "court-filing-progress" + "court-filing-captcha" 事件订阅进度。
#[tauri::command]
async fn start_court_filing(
    app: tauri::AppHandle,
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
    filing_type: String,
    agent_ids: Vec<String>,
    original_case_number: Option<String>,
    material_folder: String,
) -> Result<db::court_filing::CourtFilingJob, String> {
    // 1. 校验案件存在
    let case = cases_db::get_case(pool.inner(), &case_id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| format!("案件不存在: {}", case_id))?;

    // 2. 防重复草稿
    let active = db::court_filing::has_active_job(pool.inner(), &case_id)
        .await
        .map_err(db_err)?;
    if active {
        return Err("该案件有立案任务进行中，请等待完成后再发起".to_string());
    }

    // 3. 读取用户本次指定的立案材料文件夹。只上传这个文件夹里的 PDF，避免把全案材料都传上去。
    let docs = scan_material_folder(&material_folder)?;

    // 4. 读设置
    let settings = crate::settings::read_settings().unwrap_or_default();
    let account = settings
        .court_filing_account
        .clone()
        .ok_or_else(|| "未配置一张网账号（设置→法院立案）".to_string())?;
    let password = settings
        .court_filing_password
        .clone()
        .ok_or_else(|| "未配置一张网密码（设置→法院立案）".to_string())?;
    let cli_path = resolve_court_filing_cli_path(&app, settings.court_filing_cli_path.clone());
    let python = settings
        .court_filing_python
        .clone()
        .unwrap_or_else(|| "python3".to_string());
    let cookie_dir = settings.court_filing_cookie_dir.clone();

    // 5. 组装 case_data.json
    // user_overrides_json 优先（用户手动编辑的字段），fallback 到 agg_court
    let overrides: serde_json::Value = case
        .user_overrides_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::json!({}));
    let ov_fields = overrides
        .get("fields")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    let court_name = ov_fields
        .get("agg_court")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| case.agg_court.as_deref().filter(|s| !s.is_empty()))
        .or_else(|| case.court.as_deref().filter(|s| !s.is_empty()))
        .unwrap_or("");
    if court_name.is_empty() {
        return Err("案件缺少法院名称（请先在案件档案里填写法院信息）".to_string());
    }
    let cause = ov_fields
        .get("agg_cause")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| case.agg_cause.as_deref().filter(|s| !s.is_empty()))
        .or_else(|| case.cause.as_deref().filter(|s| !s.is_empty()))
        .unwrap_or("");
    let amount = case
        .agg_claim_amount
        .map(|a| a.to_string())
        .unwrap_or_else(|| "0".to_string());
    let court_region = infer_court_region(court_name);
    if court_region.province.is_empty() {
        return Err(format!(
            "无法从法院名称「{}」判断所属省份。请在案件法院名称中补充完整省、市、区县信息。",
            court_name
        ));
    }

    let mut case_data = serde_json::json!({
        "court_name": court_name,
        "cause_of_action": cause,
        "target_amount": amount,
        "province": court_region.province,
        "city": court_region.city,
        "district": court_region.district,
        "court_region": court_region,
        "filing_type": filing_type,
        "case_id": case_id,
        "filing_engine": "playwright",
        "original_case_number": original_case_number.as_deref().unwrap_or(""),
    });

    // 解析当事人 JSON（兼容字符串数组 ["潘尖"] 和对象数组 [{name:"潘尖"}]）
    for (key, field) in [
        ("plaintiffs", &case.agg_plaintiffs),
        ("defendants", &case.agg_defendants),
        ("third_parties", &case.agg_third_parties),
    ] {
        let raw_items: Vec<serde_json::Value> = field
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let items: Vec<serde_json::Value> = raw_items
            .into_iter()
            .map(|v| {
                if let Some(name) = v.as_str() {
                    // 根据名字判断当事人类型
                    let client_type = if name.contains("公司") || name.contains("集团")
                        || name.contains("企业") || name.contains("有限")
                        || name.contains("合伙") || name.contains("工厂")
                        || name.contains("店") || name.contains("商行")
                    {
                        "legal"
                    } else {
                        "natural"
                    };
                    serde_json::json!({"name": name, "client_type": client_type, "type": client_type})
                } else {
                    v
                }
            })
            .collect();
        case_data[key] = serde_json::json!(items);
    }

    // 补充当事人详情从 agg_party_contacts
    if let Some(contacts_str) = &case.agg_party_contacts {
        if let Ok(contacts) = serde_json::from_str::<Vec<serde_json::Value>>(contacts_str) {
            for key in ["plaintiffs", "defendants", "third_parties"] {
                if let Some(arr) = case_data[key].as_array_mut() {
                    for party in arr.iter_mut() {
                        let party_name = party
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !party_name.is_empty() {
                            if let Some(contact) = contacts.iter().find(|c| {
                                c.get("name").and_then(|n| n.as_str()) == Some(&party_name)
                            }) {
                                // 补充通用字段
                                for field in ["phone", "address"] {
                                    if party.get(field).is_none()
                                        || party[field].as_str() == Some("")
                                    {
                                        if let Some(val) = contact.get(field) {
                                            party[field] = val.clone();
                                        }
                                    }
                                }
                                // 法人：id_no → uscc（统一社会信用代码）
                                let client_type = party
                                    .get("client_type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                if client_type == "legal" {
                                    if let Some(id_no) =
                                        contact.get("id_no").and_then(|v| v.as_str())
                                    {
                                        if !id_no.is_empty() {
                                            party["uscc"] = serde_json::json!(id_no);
                                        }
                                    }
                                    // 查找法定代表人（role 包含"法定代表人"的联系人）
                                    if let Some(rep_contact) = contacts.iter().find(|c| {
                                        c.get("role")
                                            .and_then(|r| r.as_str())
                                            .unwrap_or("")
                                            .contains("法定代表人")
                                    }) {
                                        if let Some(rep_name) =
                                            rep_contact.get("name").and_then(|v| v.as_str())
                                        {
                                            party["legal_rep"] = serde_json::json!(rep_name);
                                        }
                                        if let Some(rep_id) =
                                            rep_contact.get("id_no").and_then(|v| v.as_str())
                                        {
                                            party["legal_rep_id_number"] =
                                                serde_json::json!(rep_id);
                                        }
                                        // 法定代表人的地址作为公司地址（如果没有公司地址）
                                        if party
                                            .get("address")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .is_empty()
                                        {
                                            if let Some(rep_addr) =
                                                rep_contact.get("address").and_then(|v| v.as_str())
                                            {
                                                party["address"] = serde_json::json!(rep_addr);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 6. 查律师档案组装 agents
    let mut agents = Vec::new();
    for agent_id in &agent_ids {
        if let Some(profile) = db::lawyer_profiles::get(pool.inner(), agent_id)
            .await
            .map_err(db_err)?
        {
            agents.push(serde_json::json!({
                "name": profile.name,
                "id_number": profile.id_number.as_deref().unwrap_or(""),
                "bar_number": profile.bar_number.as_deref().unwrap_or(""),
                "law_firm": profile.law_firm.as_deref().unwrap_or(""),
                "address": profile.address.as_deref().unwrap_or(""),
                "phone": profile.phone.as_deref().unwrap_or(""),
            }));
        }
    }
    case_data["agents"] = serde_json::json!(agents);
    if let Some(first) = agents.first() {
        case_data["agent"] = first.clone();
    }

    // 补充：我方当事人（原告）的电话填律师的电话（如果没有电话）
    // 被告/第三人的电话有就填，没有就空着
    let agent_phone = agents
        .first()
        .and_then(|a| a.get("phone"))
        .and_then(|p| p.as_str())
        .unwrap_or("");
    if !agent_phone.is_empty() {
        // 只给 plaintiffs 填律师电话（我方代理的当事人）
        if let Some(arr) = case_data["plaintiffs"].as_array_mut() {
            for party in arr.iter_mut() {
                let phone = party.get("phone").and_then(|v| v.as_str()).unwrap_or("");
                if phone.is_empty() {
                    party["phone"] = serde_json::json!(agent_phone);
                }
            }
        }
    }

    // 7. 组装 materials.json + material_preflight.json。
    // 前端只暴露简单流程；排错时看预检报告即可知道每个 PDF 被归到哪个法院材料槽位。
    let (materials, material_report) =
        build_court_filing_materials(&docs, &material_folder, &filing_type);
    let missing_required_count = material_report
        .get("missing_required")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    if missing_required_count > 0 {
        let missing = material_report
            .get("missing_required")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            })
            .unwrap_or_default();
        return Err(format!(
            "立案材料还没准备完整，缺少：{}。请把这些 PDF 放进同一个材料文件夹后再开始。",
            missing
        ));
    }
    case_data["materials"] = materials.clone();
    case_data["materials_manifest"] = material_report.clone();
    case_data["material_folder"] = serde_json::json!(material_folder);

    // 7.5 从委托手续 PDF OCR 提取代理人信息，补充到 agents
    if let Some(mats) = materials.as_object() {
        if let Some(slot2) = mats.get("2").and_then(|v| v.as_array()) {
            if let Some(first_mat) = slot2.first().and_then(|v| v.as_array()) {
                if let Some(pdf_path) = first_mat.first().and_then(|v| v.as_str()) {
                    match extract_agents_from_pdf(pdf_path, &settings).await {
                        Ok(extracted_agents) => {
                            if !extracted_agents.is_empty() {
                                // 合并：保留原有律师档案里的代理人，追加从 PDF 提取的（去重）
                                let existing_names: Vec<String> = agents
                                    .iter()
                                    .filter_map(|a| {
                                        a.get("name").and_then(|v| v.as_str()).map(String::from)
                                    })
                                    .collect();
                                for agent in extracted_agents {
                                    let name =
                                        agent.get("name").and_then(|v| v.as_str()).unwrap_or("");
                                    if !existing_names.contains(&name.to_string()) {
                                        agents.push(agent);
                                    }
                                }
                                case_data["agents"] = serde_json::json!(agents);
                                if let Some(first) = agents.first() {
                                    case_data["agent"] = first.clone();
                                }
                            }
                        }
                        Err(e) => {
                            // OCR 失败不阻塞立案，只记录警告
                            crate::dlog!("从委托手续提取代理人失败（不影响立案）: {}", e);
                        }
                    }
                }
            }
        }
    }

    // 8. 插 pending 记录
    let job = db::court_filing::insert(
        pool.inner(),
        &db::court_filing::NewCourtFilingJob {
            case_id: case_id.clone(),
            filing_type: filing_type.clone(),
            court_name: court_name.to_string(),
            cookie_account: Some(account.clone()),
            output_dir: None, // 后面更新
        },
    )
    .await
    .map_err(db_err)?;

    // 9. 建 output_dir + 写 JSON 文件
    let output_dir = crate::db::app_data_dir()
        .map_err(|e| e.to_string())?
        .join("court_filing")
        .join(&job.id);
    tokio::fs::create_dir_all(&output_dir)
        .await
        .map_err(|e| format!("创建输出目录失败: {}", e))?;
    let output_dir_str = output_dir.to_string_lossy().to_string();

    let case_data_path = output_dir.join("case_data.json");
    tokio::fs::write(
        &case_data_path,
        serde_json::to_string_pretty(&case_data).unwrap(),
    )
    .await
    .map_err(|e| format!("写 case_data.json 失败: {}", e))?;

    let materials_path = output_dir.join("materials.json");
    tokio::fs::write(
        &materials_path,
        serde_json::to_string_pretty(&materials).unwrap(),
    )
    .await
    .map_err(|e| format!("写 materials.json 失败: {}", e))?;

    let preflight_path = output_dir.join("material_preflight.json");
    tokio::fs::write(
        &preflight_path,
        serde_json::to_string_pretty(&material_report).unwrap(),
    )
    .await
    .map_err(|e| format!("写 material_preflight.json 失败: {}", e))?;

    // 更新 job 的 output_dir
    sqlx::query("UPDATE court_filing_jobs SET output_dir = ? WHERE id = ?")
        .bind(&output_dir_str)
        .bind(&job.id)
        .execute(pool.inner())
        .await
        .map_err(db_err)?;

    // 10. spawn CLI（流式 stdout）
    let app_clone = app.clone();
    let pool_clone = pool.inner().clone();
    let job_id = job.id.clone();
    let case_id_clone = case_id.clone();
    let cli_path_clone = cli_path.clone();
    let output_dir_clone = output_dir_str.clone();
    let material_report_clone = material_report.clone();

    tauri::async_runtime::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        use tokio::process::Command;

        let output_dir_path = std::path::PathBuf::from(&output_dir_clone);
        let progress_log_path = output_dir_path.join("progress_events.jsonl");
        let stderr_log_path = output_dir_path.join("stderr.log");
        let diagnosis_path = output_dir_path.join("final_diagnosis.json");

        let _ = app_clone.emit(
            "court-filing-progress",
            CourtFilingProgress {
                job_id: job_id.clone(),
                case_id: case_id_clone.clone(),
                phase: "system".into(),
                stage: "filing.start".into(),
                level: "info".into(),
                message: "正在启动立案流程...".into(),
                detail: None,
                round: None,
                task_id: None,
                image_base64: None,
                timing: None,
            },
        );
        append_jsonl(
            &progress_log_path,
            &serde_json::json!({
                "phase": "system",
                "stage": "filing.start",
                "level": "info",
                "message": "正在启动立案流程...",
                "ts": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            }),
        )
        .await;

        let preflight_warnings = material_report_clone
            .get("warnings")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("；")
            })
            .unwrap_or_default();
        let matched_documents = material_report_clone
            .get("matched_documents")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let preflight_message = if preflight_warnings.is_empty() {
            format!("材料预检完成：已自动匹配 {} 份 PDF", matched_documents)
        } else {
            format!(
                "材料预检完成：已自动匹配 {} 份 PDF，{}",
                matched_documents, preflight_warnings
            )
        };
        let _ = app_clone.emit(
            "court-filing-progress",
            CourtFilingProgress {
                job_id: job_id.clone(),
                case_id: case_id_clone.clone(),
                phase: "system".into(),
                stage: "materials.preflight".into(),
                level: if preflight_warnings.is_empty() {
                    "info"
                } else {
                    "warning"
                }
                .into(),
                message: preflight_message.clone(),
                detail: Some("完整预检报告已保存到 material_preflight.json".into()),
                round: None,
                task_id: None,
                image_base64: None,
                timing: None,
            },
        );
        append_jsonl(
            &progress_log_path,
            &serde_json::json!({
                "phase": "system",
                "stage": "materials.preflight",
                "level": if preflight_warnings.is_empty() { "info" } else { "warning" },
                "message": preflight_message,
                "detail": "完整预检报告已保存到 material_preflight.json",
                "ts": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            }),
        )
        .await;

        let case_data_path_str = case_data_path.to_string_lossy().to_string();
        let materials_path_str = materials_path.to_string_lossy().to_string();

        let mut args = vec![
            "-m".to_string(),
            "court_filing_cli".to_string(),
            "--account".to_string(),
            account.clone(),
            "--password".to_string(),
            password.clone(),
            "--filing-type".to_string(),
            filing_type.clone(),
            "--case-data".to_string(),
            case_data_path_str,
            "--materials".to_string(),
            materials_path_str,
            "--output-dir".to_string(),
            output_dir_clone.clone(),
            "--log-level".to_string(),
            "INFO".to_string(),
        ];
        if let Some(ref cd) = cookie_dir {
            args.extend(["--cookie-dir".to_string(), cd.clone()]);
        }

        // 从 cli_path 的父目录运行，这样 python -m court_filing_cli 能找到包
        let cli_parent = std::path::Path::new(&cli_path_clone)
            .parent()
            .unwrap_or(std::path::Path::new(&cli_path_clone));
        let spawn_result = Command::new(&python)
            .current_dir(cli_parent)
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn();

        let mut child = match spawn_result {
            Ok(c) => c,
            Err(e) => {
                let err_msg = format!("启动 CLI 失败（确认 python3 + 依赖已装）: {}", e);
                let _ = db::court_filing::update_status(
                    &pool_clone,
                    &job_id,
                    "failed",
                    Some(&err_msg),
                    None,
                    None,
                )
                .await;
                let _ = app_clone.emit(
                    "court-filing-progress",
                    CourtFilingProgress {
                        job_id: job_id.clone(),
                        case_id: case_id_clone.clone(),
                        phase: "system".into(),
                        stage: "cli.spawn_failed".into(),
                        level: "error".into(),
                        message: err_msg.clone(),
                        detail: None,
                        round: None,
                        task_id: None,
                        image_base64: None,
                        timing: None,
                    },
                );
                let _ = tokio::fs::write(&diagnosis_path, serde_json::to_string_pretty(&serde_json::json!({
                    "status": "failed",
                    "failed_stage": "cli.spawn_failed",
                    "summary": "CLI 启动失败",
                    "error": err_msg,
                    "material_preflight": material_report_clone,
                    "output_dir": output_dir_clone,
                    "generated_at": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                })).unwrap_or_default()).await;
                return;
            }
        };

        let _ = db::court_filing::update_status(&pool_clone, &job_id, "running", None, None, None)
            .await;

        // 逐行读 stderr，避免子进程错误输出过多导致管道阻塞；同时落盘供失败诊断。
        let stderr_task = child.stderr.take().map(|stderr| {
            let stderr_log_path = stderr_log_path.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let reader = tokio::io::BufReader::new(stderr);
                let mut lines = reader.lines();
                let mut excerpt: Vec<String> = Vec::new();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(mut file) = tokio::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&stderr_log_path)
                        .await
                    {
                        let _ = file.write_all(line.as_bytes()).await;
                        let _ = file.write_all(b"\n").await;
                    }
                    excerpt.push(line);
                    if excerpt.len() > 40 {
                        excerpt.remove(0);
                    }
                }
                excerpt
            })
        });

        // 逐行读 stdout
        let stdout = child.stdout.take().expect("stdout piped");
        let reader = tokio::io::BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut last_progress_json = String::new();
        let mut last_stage = "filing.start".to_string();
        let mut last_message = "正在启动立案流程...".to_string();
        let mut last_detail: Option<String> = None;

        while let Ok(Some(line)) = lines.next_line().await {
            let ev: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue, // 非 JSON 行跳过
            };

            let progress_str = serde_json::to_string(&ev).unwrap_or_default();
            last_progress_json = progress_str.clone();
            let _ = db::court_filing::update_progress(&pool_clone, &job_id, &progress_str).await;

            let phase = ev.get("phase").and_then(|v| v.as_str()).unwrap_or("");
            let stage = ev.get("stage").and_then(|v| v.as_str()).unwrap_or("");
            let level = ev.get("level").and_then(|v| v.as_str()).unwrap_or("info");
            let message = ev.get("message").and_then(|v| v.as_str()).unwrap_or("");
            let detail = ev.get("detail").and_then(|v| v.as_str()).map(String::from);
            let round = ev.get("round").and_then(|v| v.as_i64());
            let task_id = ev.get("task_id").and_then(|v| v.as_str()).map(String::from);
            let image_base64 = ev
                .get("image_base64")
                .and_then(|v| v.as_str())
                .map(String::from);
            let timing = ev.get("timing").cloned();
            if !stage.is_empty() {
                last_stage = stage.to_string();
            }
            if !message.is_empty() {
                last_message = message.to_string();
            }
            last_detail = detail.clone();
            let mut log_ev = ev.clone();
            if let Some(obj) = log_ev.as_object_mut() {
                obj.insert(
                    "ts".to_string(),
                    serde_json::Value::String(
                        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                    ),
                );
            }
            append_jsonl(&progress_log_path, &log_ev).await;

            let _ = app_clone.emit(
                "court-filing-progress",
                CourtFilingProgress {
                    job_id: job_id.clone(),
                    case_id: case_id_clone.clone(),
                    phase: phase.to_string(),
                    stage: stage.to_string(),
                    level: level.to_string(),
                    message: message.to_string(),
                    detail,
                    round,
                    task_id: task_id.clone(),
                    image_base64: image_base64.clone(),
                    timing,
                },
            );

            // 处理验证码事件
            if phase == "captcha" && stage == "captcha.required" {
                let _ = db::court_filing::set_captcha_active(&pool_clone, &job_id, true).await;
                if let (Some(img), Some(tid), Some(rd)) =
                    (image_base64.as_deref(), task_id.as_deref(), round)
                {
                    let _ = app_clone.emit(
                        "court-filing-captcha",
                        CourtFilingCaptcha {
                            job_id: job_id.clone(),
                            case_id: case_id_clone.clone(),
                            task_id: tid.to_string(),
                            round: rd,
                            image_base64: img.to_string(),
                            timeout_sec: 300,
                        },
                    );
                }
            }
            if phase == "captcha" && (stage == "captcha.answered" || stage == "captcha.timeout") {
                let _ = db::court_filing::set_captcha_active(&pool_clone, &job_id, false).await;
            }
        }

        // 等子进程结束
        let exit_status = child.wait().await;
        let exit_code = exit_status
            .as_ref()
            .map(|s| s.code().unwrap_or(-1))
            .unwrap_or(-1);
        let stderr_excerpt = match stderr_task {
            Some(task) => task.await.unwrap_or_default(),
            None => Vec::new(),
        };

        // 判断最终状态
        let final_status = if exit_code == 0 {
            "completed"
        } else {
            "failed"
        };
        let error_msg = if exit_code != 0 {
            Some(court_filing_user_error(
                &last_stage,
                &last_message,
                last_detail.as_deref(),
            ))
        } else {
            None
        };

        // 从最后一条 progress 提取 preview_url
        let preview_url: Option<String> =
            serde_json::from_str::<serde_json::Value>(&last_progress_json)
                .ok()
                .and_then(|v| v.get("result")?.get("url")?.as_str().map(String::from));

        let _ = db::court_filing::update_status(
            &pool_clone,
            &job_id,
            final_status,
            error_msg.as_deref(),
            preview_url.as_deref(),
            None,
        )
        .await;

        let diagnosis = serde_json::json!({
            "status": final_status,
            "exit_code": exit_code,
            "failed_stage": if final_status == "failed" { Some(last_stage.clone()) } else { None },
            "last_message": last_message,
            "last_detail": last_detail,
            "error": error_msg,
            "preview_url": preview_url,
            "stderr_excerpt": stderr_excerpt,
            "material_preflight": material_report_clone,
            "files": {
                "case_data": case_data_path.to_string_lossy().to_string(),
                "materials": materials_path.to_string_lossy().to_string(),
                "material_preflight": preflight_path.to_string_lossy().to_string(),
                "progress_events": progress_log_path.to_string_lossy().to_string(),
                "stderr": stderr_log_path.to_string_lossy().to_string()
            },
            "output_dir": output_dir_clone,
            "generated_at": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        });
        let _ = tokio::fs::write(
            &diagnosis_path,
            serde_json::to_string_pretty(&diagnosis).unwrap_or_default(),
        )
        .await;
        append_jsonl(&progress_log_path, &serde_json::json!({
            "phase": "system",
            "stage": if final_status == "completed" { "filing.success" } else { "filing.failed" },
            "level": if final_status == "completed" { "info" } else { "error" },
            "message": if final_status == "completed" { "立案流程执行完成（已到预览页，未提交）" } else { "立案流程失败" },
            "detail": error_msg,
            "ts": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        })).await;

        let final_msg = if final_status == "completed" {
            "立案流程执行完成（已到预览页，未提交）"
        } else {
            "立案流程失败"
        };
        let _ = app_clone.emit(
            "court-filing-progress",
            CourtFilingProgress {
                job_id: job_id.clone(),
                case_id: case_id_clone.clone(),
                phase: "system".into(),
                stage: if final_status == "completed" {
                    "filing.success"
                } else {
                    "filing.failed"
                }
                .into(),
                level: if final_status == "completed" {
                    "info"
                } else {
                    "error"
                }
                .into(),
                message: final_msg.into(),
                detail: error_msg,
                round: None,
                task_id: None,
                image_base64: None,
                timing: None,
            },
        );
    });

    Ok(db::court_filing::CourtFilingJob {
        id: job.id,
        case_id: job.case_id,
        filing_type: job.filing_type,
        court_name: job.court_name,
        cookie_account: job.cookie_account,
        status: "pending".into(),
        output_dir: Some(output_dir_str),
        preview_url: None,
        progress_json: None,
        captcha_active: 0,
        error: None,
        timing_json: None,
        created_at: job.created_at,
        updated_at: job.updated_at,
    })
}

/// 提交验证码答案（写 captcha_answer.json 到 output_dir，CLI 轮询读取）。
#[tauri::command]
async fn submit_captcha_answer(
    pool: tauri::State<'_, SqlitePool>,
    job_id: String,
    task_id: String,
    round: i64,
    answer: String,
) -> Result<(), String> {
    let job = db::court_filing::get(pool.inner(), &job_id)
        .await
        .map_err(db_err)?
        .ok_or_else(|| format!("立案任务不存在: {}", job_id))?;

    let output_dir = job
        .output_dir
        .ok_or_else(|| "任务输出目录不存在".to_string())?;
    let answer_path = std::path::PathBuf::from(&output_dir).join("captcha_answer.json");

    let answer_json = serde_json::json!({
        "task_id": task_id,
        "round": round,
        "answer": answer,
        "submitted_ts": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
    });
    tokio::fs::write(&answer_path, serde_json::to_string(&answer_json).unwrap())
        .await
        .map_err(|e| format!("写 captcha_answer.json 失败: {}", e))?;

    let _ = db::court_filing::set_captcha_active(pool.inner(), &job_id, false)
        .await
        .map_err(db_err)?;
    let _ = db::court_filing::update_status(pool.inner(), &job_id, "running", None, None, None)
        .await
        .map_err(db_err)?;

    Ok(())
}

/// 列出某案件的全部立案记录。
#[tauri::command]
async fn list_court_filing_jobs(
    pool: tauri::State<'_, SqlitePool>,
    case_id: String,
) -> Result<Vec<db::court_filing::CourtFilingJob>, String> {
    db::court_filing::list_by_case(pool.inner(), &case_id)
        .await
        .map_err(db_err)
}

/// 查单条立案任务。
#[tauri::command]
async fn get_court_filing_job(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
) -> Result<Option<db::court_filing::CourtFilingJob>, String> {
    db::court_filing::get(pool.inner(), &id)
        .await
        .map_err(db_err)
}

/// 列出全部律师档案。
#[tauri::command]
async fn list_lawyer_profiles(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<Vec<db::lawyer_profiles::LawyerProfile>, String> {
    db::lawyer_profiles::list(pool.inner())
        .await
        .map_err(db_err)
}

/// 新增律师档案。
#[tauri::command]
async fn save_lawyer_profile(
    pool: tauri::State<'_, SqlitePool>,
    profile: db::lawyer_profiles::SaveLawyerProfile,
) -> Result<db::lawyer_profiles::LawyerProfile, String> {
    db::lawyer_profiles::insert(pool.inner(), &profile)
        .await
        .map_err(db_err)
}

/// 更新律师档案。
#[tauri::command]
async fn update_lawyer_profile(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
    profile: db::lawyer_profiles::SaveLawyerProfile,
) -> Result<db::lawyer_profiles::LawyerProfile, String> {
    db::lawyer_profiles::update(pool.inner(), &id, &profile)
        .await
        .map_err(db_err)
}

/// 删除律师档案。
#[tauri::command]
async fn delete_lawyer_profile(
    pool: tauri::State<'_, SqlitePool>,
    id: String,
) -> Result<(), String> {
    db::lawyer_profiles::delete(pool.inner(), &id)
        .await
        .map_err(db_err)
}

/// 设为默认律师。
#[tauri::command]
async fn set_default_lawyer(pool: tauri::State<'_, SqlitePool>, id: String) -> Result<(), String> {
    db::lawyer_profiles::set_default(pool.inner(), &id)
        .await
        .map_err(db_err)
}

// ============================================================================

#[tauri::command]
fn parse_mcp_paste(text: String) -> Result<chat::mcp_paste::ParsedPaste, String> {
    chat::mcp_paste::parse_pasted_config(&text)
}

/// MCP 连接测试结果(给设置页「测试连接」按钮)。
#[derive(serde::Serialize)]
struct McpTestReport {
    tool_count: usize,
    /// 前若干个工具名(给用户确认接对了;不全量,防几十个工具刷屏)。
    tool_names: Vec<String>,
}

/// 连接测试:真连一次 server(initialize 握手 + tools/list),返回工具清单。
/// 失败透传真实原因(401=令牌不对/过期、403=服务未购买等,已知坑 #8)。
#[tauri::command]
async fn test_mcp_server(
    config: chat::mcp_bridge::McpServerConfig,
) -> Result<McpTestReport, String> {
    config.validate()?;
    let client = chat::mcp_bridge::McpClient::connect(&config).await?;
    let tools = client.list_tools().await?;
    Ok(McpTestReport {
        tool_count: tools.len(),
        tool_names: tools.iter().take(8).map(|t| t.name.clone()).collect(),
    })
}

// ============================================================================
// 团队版 Phase 1(LAN 接力同步,docs/提案-团队版-2026-06-10.md §6)
// ============================================================================

/// 团队网络运行时句柄(监听+广播+周期同步),随团队配置启停。
pub struct TeamNetState(tokio::sync::Mutex<Option<team::net::TeamNet>>);

impl Default for TeamNetState {
    fn default() -> Self {
        Self(tokio::sync::Mutex::new(None))
    }
}

/// 按当前 settings.team 重启团队网络(入团→启动;退团→停止)。
async fn team_net_restart(pool: &SqlitePool, state: &TeamNetState) -> Result<(), String> {
    let mut guard = state.0.lock().await;
    if let Some(old) = guard.take() {
        old.shutdown();
    }
    if settings::read_settings()?.team.is_some() {
        *guard = Some(team::net::start(pool.clone()).await?);
    }
    Ok(())
}

/// 清掉本机团队身份与数据(退出/解散/被踢共用)。
async fn team_clear_local(pool: &SqlitePool, state: &TeamNetState) -> Result<(), String> {
    let mut guard = state.0.lock().await;
    if let Some(old) = guard.take() {
        old.shutdown();
    }
    drop(guard);
    team::store::clear_team_data(pool).await?;
    let mut s = settings::read_settings()?;
    s.team = None;
    settings::write_settings(&s)?;
    Ok(())
}

#[derive(Serialize)]
struct TeamStatusDto {
    in_team: bool,
    /// 被踢出的团队名(一次性提示;返回即已自动清理本机团队配置)。
    kicked_from: Option<String>,
    identity: Option<team::TeamIdentity>,
    roster: Option<team::Roster>,
}

/// 团队状态(设置页团队卡数据源)。顺带处理「被踢」:发现自己不在名单 → 自动清理并告知。
#[tauri::command]
async fn team_status(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, TeamNetState>,
) -> Result<TeamStatusDto, String> {
    let Some(identity) = settings::read_settings()?.team else {
        return Ok(TeamStatusDto {
            in_team: false,
            kicked_from: None,
            identity: None,
            roster: None,
        });
    };
    let roster = match team::store::load_signed_roster(pool.inner()).await? {
        Some(sr) => sr.verify(&identity.team_secret).ok(),
        None => None,
    };
    // 被踢检测:同步时留的通知,或当前名单里已没有我(团队长除外)
    let kicked_notice = team::store::take_kicked_notice(pool.inner()).await?;
    let not_in_roster = roster
        .as_ref()
        .is_some_and(|r| r.find(&identity.member_id).is_none());
    if !identity.is_leader() && (kicked_notice.is_some() || not_in_roster) {
        let team_name = kicked_notice.unwrap_or_else(|| identity.team_name.clone());
        team_clear_local(pool.inner(), state.inner()).await?;
        return Ok(TeamStatusDto {
            in_team: false,
            kicked_from: Some(team_name),
            identity: None,
            roster: None,
        });
    }
    Ok(TeamStatusDto {
        in_team: true,
        kicked_from: None,
        identity: Some(identity),
        roster,
    })
}

/// 创建团队(我成为团队长):生成密钥/配对码,roster 只有我,启动网络。
#[tauri::command]
async fn team_create(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, TeamNetState>,
    team_name: String,
    my_name: String,
) -> Result<TeamStatusDto, String> {
    let team_name = team_name.trim().to_string();
    let my_name = my_name.trim().to_string();
    if team_name.is_empty() || my_name.is_empty() {
        return Err("团队名和你的姓名都不能为空".into());
    }
    let mut settings = settings::read_settings()?;
    if settings.team.is_some() {
        return Err("已在团队中,请先退出当前团队".into());
    }
    let identity = team::TeamIdentity {
        team_id: uuid::Uuid::new_v4().to_string(),
        team_name: team_name.clone(),
        team_secret: team::gen_secret(),
        member_id: uuid::Uuid::new_v4().to_string(),
        my_name: my_name.clone(),
        role: "leader".into(),
        pairing_code: Some(team::gen_pairing_code()),
    };
    let roster = team::Roster {
        team_id: identity.team_id.clone(),
        team_name,
        seq: 1,
        members: vec![team::RosterMember {
            member_id: identity.member_id.clone(),
            name: my_name,
            role: "leader".into(),
            view: None,
            edit: vec![],
        }],
        updated_at: chrono::Local::now().to_rfc3339(),
    };
    let signed = team::SignedRoster::sign(&roster, &identity.team_secret)?;
    team::store::save_signed_roster(pool.inner(), &signed).await?;
    settings.team = Some(identity.clone());
    settings::write_settings(&settings)?;
    team::store::rebuild_own_snapshot(pool.inner(), &identity).await?;
    team_net_restart(pool.inner(), state.inner()).await?;
    Ok(TeamStatusDto {
        in_team: true,
        kicked_from: None,
        identity: Some(identity),
        roster: Some(roster),
    })
}

/// 扫描局域网内可加入的团队(约 3 秒)。
#[tauri::command]
async fn team_discover() -> Result<Vec<team::net::DiscoveredTeam>, String> {
    team::net::discover_teams().await
}

/// 加入团队(需团队长在线 + 配对码),成功即启动网络并跑一轮同步。
#[tauri::command]
async fn team_join(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, TeamNetState>,
    team_id: String,
    code: String,
    my_name: String,
) -> Result<TeamStatusDto, String> {
    if my_name.trim().is_empty() {
        return Err("请先填你的姓名(团队里显示用)".into());
    }
    if settings::read_settings()?.team.is_some() {
        return Err("已在团队中,请先退出当前团队".into());
    }
    team::net::join_team(pool.inner(), &team_id, &code, &my_name).await?;
    team_net_restart(pool.inner(), state.inner()).await?;
    // 入队后立即拉一轮(拿到全队现状);失败不拦断,周期同步会补
    let _ = team::net::sync_round(pool.inner()).await;
    team_status(pool, state).await
}

/// 退出团队(成员)/ 解散团队(团队长)。
/// 团队长解散 = 先签发"空名单"墓碑并尽力广播给**当前在场**的成员(他们收到即走被踢
/// 流程自动清理);不在局域网的成员收不到墓碑,只能回所后从在场队友处接力收到,
/// 或自己手动「退出团队」—— 无服务器架构的诚实边界。最后清本机。
#[tauri::command]
async fn team_leave(
    pool: tauri::State<'_, SqlitePool>,
    state: tauri::State<'_, TeamNetState>,
) -> Result<(), String> {
    if let Some(identity) = settings::read_settings()?.team {
        if identity.is_leader() {
            let ok = team::net::mutate_roster(pool.inner(), &identity, |r| {
                r.members.clear();
            })
            .await
            .is_ok();
            if ok {
                // 尽力广播(等几秒值得:在场成员当场收到当场清);失败不拦断解散
                let _ = team::net::sync_round(pool.inner()).await;
            }
        }
    }
    team_clear_local(pool.inner(), state.inner()).await
}

/// 团队长移出成员:roster 删人 seq+1,随同步下发;被踢成员 App 端自动清理。
#[tauri::command]
async fn team_kick(
    pool: tauri::State<'_, SqlitePool>,
    member_id: String,
) -> Result<team::Roster, String> {
    let identity = settings::read_settings()?.team.ok_or("未加入团队")?;
    if member_id == identity.member_id {
        return Err("不能移出自己(要散伙请用「退出团队」)".into());
    }
    let signed = team::net::mutate_roster(pool.inner(), &identity, |r| {
        r.members.retain(|m| m.member_id != member_id);
    })
    .await?;
    // 尽快把新名单传出去(后台跑,不阻塞 UI)
    let p = pool.inner().clone();
    tauri::async_runtime::spawn(async move {
        let _ = team::net::sync_round(&p).await;
    });
    signed.verify(&identity.team_secret)
}

/// 团队长配置某成员权限:可见范围(null=全队)+ 可编辑哪些人(Phase 1 配置下发,动作 1.5)。
#[tauri::command]
async fn team_set_permissions(
    pool: tauri::State<'_, SqlitePool>,
    member_id: String,
    view: Option<Vec<String>>,
    edit: Vec<String>,
) -> Result<team::Roster, String> {
    let identity = settings::read_settings()?.team.ok_or("未加入团队")?;
    let signed = team::net::mutate_roster(pool.inner(), &identity, |r| {
        if let Some(m) = r.members.iter_mut().find(|m| m.member_id == member_id) {
            m.view = view;
            m.edit = edit;
        }
    })
    .await?;
    let p = pool.inner().clone();
    tauri::async_runtime::spawn(async move {
        let _ = team::net::sync_round(&p).await;
    });
    signed.verify(&identity.team_secret)
}

/// 团队长刷新配对码(旧码立即作废)。
#[tauri::command]
fn team_refresh_code() -> Result<String, String> {
    let mut settings = settings::read_settings()?;
    let Some(identity) = settings.team.as_mut() else {
        return Err("未加入团队".into());
    };
    if !identity.is_leader() {
        return Err("仅团队长有配对码".into());
    }
    let code = team::gen_pairing_code();
    identity.pairing_code = Some(code.clone());
    settings::write_settings(&settings)?;
    Ok(code)
}

/// 立即同步(老板说的"发信号"):扫描在场队友并互换。
#[tauri::command]
async fn team_sync_now(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<team::net::SyncReport, String> {
    team::net::sync_round(pool.inner()).await
}

#[derive(Serialize)]
struct TeamMemberViewDto {
    member_id: String,
    name: String,
    role: String,
    is_me: bool,
    /// 我对他有无编辑权(Phase 1 仅驱动按钮占位显示)。
    can_edit: bool,
    /// 还没收到过他的快照时为 None。
    updated_at: Option<String>,
    cases: Vec<team::SnapshotCase>,
}

#[derive(Serialize)]
struct TeamViewDto {
    team_name: String,
    my_member_id: String,
    my_role: String,
    members: Vec<TeamMemberViewDto>,
    /// 编辑请求/改动记录(备注展示、待生效标记、所有人撤销列表共用这一份)。
    edits: Vec<team::TeamEdit>,
}

/// 团队看板数据:按 roster 顺序(团队长在前),**按我的可见权限过滤后**返回。
#[tauri::command]
async fn team_view(pool: tauri::State<'_, SqlitePool>) -> Result<TeamViewDto, String> {
    let identity = settings::read_settings()?.team.ok_or("未加入团队")?;
    // 自己的快照现重建,保证看板里"我"永远是最新的
    team::store::rebuild_own_snapshot(pool.inner(), &identity).await?;
    let roster = match team::store::load_signed_roster(pool.inner()).await? {
        Some(sr) => sr.verify(&identity.team_secret)?,
        None => return Err("本地没有团队名单(数据异常,请退出后重新加入)".into()),
    };
    let snapshots = team::store::load_all_snapshots(pool.inner()).await?;
    let snap_of = |mid: &str| snapshots.iter().find(|s| s.member_id == mid);

    let mut members: Vec<&team::RosterMember> = roster.members.iter().collect();
    members.sort_by_key(|m| (m.role != "leader", m.name.clone()));

    let me = &identity.member_id;
    let mut out = Vec::new();
    for m in members {
        if !roster.can_view(me, &m.member_id) {
            continue; // 权限过滤 = 默认不显示(老板拍板口径)
        }
        let snap = snap_of(&m.member_id);
        let cases = snap
            .and_then(|s| serde_json::from_str::<team::SnapshotPayload>(&s.payload).ok())
            .map(|p| p.cases)
            .unwrap_or_default();
        out.push(TeamMemberViewDto {
            member_id: m.member_id.clone(),
            name: m.name.clone(),
            role: m.role.clone(),
            is_me: &m.member_id == me,
            can_edit: roster.can_edit(me, &m.member_id) && &m.member_id != me,
            updated_at: snap.map(|s| s.updated_at.clone()),
            cases,
        });
    }
    // 改动记录:只给「我能看到的目标」或「我自己发起的」(同可见权限口径)
    let edits = team::store::load_recent_edits(pool.inner())
        .await?
        .into_iter()
        .filter(|e| e.editor_id == identity.member_id || roster.can_view(me, &e.target_member_id))
        .collect();
    Ok(TeamViewDto {
        team_name: roster.team_name.clone(),
        my_member_id: identity.member_id.clone(),
        my_role: identity.role.clone(),
        members: out,
        edits,
    })
}

/// 提交一条对队友案件的编辑请求(需有编辑权;经接力转交,所有人应用后生效)。
#[tauri::command]
async fn team_submit_edit(
    pool: tauri::State<'_, SqlitePool>,
    target_member_id: String,
    case_id: String,
    case_name: String,
    field: String,
    value: String,
) -> Result<(), String> {
    let identity = settings::read_settings()?.team.ok_or("未加入团队")?;
    if !team::EDITABLE_FIELDS.contains(&field.as_str()) {
        return Err("只允许改案件登记层字段(状态/备注)".into());
    }
    let value = value.trim().to_string();
    if value.is_empty() || value.chars().count() > 500 {
        return Err("内容不能为空且不超过 500 字".into());
    }
    let roster = match team::store::load_signed_roster(pool.inner()).await? {
        Some(sr) => sr.verify(&identity.team_secret)?,
        None => return Err("本地没有团队名单".into()),
    };
    // 备注 = 可见即可写(老板拍板);改状态才要编辑权
    let allowed = match field.as_str() {
        "note" => roster.can_view(&identity.member_id, &target_member_id),
        _ => roster.can_edit(&identity.member_id, &target_member_id),
    };
    if !allowed {
        return Err("你没有编辑这位成员案件的权限(找团队长开)".into());
    }
    let edit = team::TeamEdit {
        id: uuid::Uuid::new_v4().to_string(),
        team_id: identity.team_id.clone(),
        editor_id: identity.member_id.clone(),
        editor_name: identity.my_name.clone(),
        target_member_id,
        case_id,
        case_name,
        field,
        value,
        prev_value: None,
        status: "pending".into(),
        created_at: chrono::Local::now().to_rfc3339(),
        applied_at: None,
    };
    let self_target = edit.target_member_id == identity.member_id;
    team::store::insert_pending_edit(pool.inner(), &edit).await?;
    if self_target {
        // 给自己案件留备注/改状态:立即应用,不等下一轮接力
        let applied = team::store::apply_my_pending_edits(pool.inner(), &identity, &roster).await?;
        if applied > 0 {
            team::store::rebuild_own_snapshot(pool.inner(), &identity).await?;
        }
    }
    // 尽快送出去(后台跑;对方不在线就等下一轮接力)
    let p = pool.inner().clone();
    tauri::async_runtime::spawn(async move {
        let _ = team::net::sync_round(&p).await;
    });
    Ok(())
}

/// 案件所有人撤销一条已生效的队友改动(状态恢复原值/备注隐藏)。
#[tauri::command]
async fn team_revert_edit(
    pool: tauri::State<'_, SqlitePool>,
    edit_id: String,
) -> Result<(), String> {
    let identity = settings::read_settings()?.team.ok_or("未加入团队")?;
    team::store::revert_edit(pool.inner(), &identity, &edit_id).await?;
    // 撤销后重建快照 + 传播
    team::store::rebuild_own_snapshot(pool.inner(), &identity).await?;
    let p = pool.inner().clone();
    tauri::async_runtime::spawn(async move {
        let _ = team::net::sync_round(&p).await;
    });
    Ok(())
}

// ============================================================================
// V0.2 D7 · 本地知识库 + 元典积分 Settings 卡片用的 5 个 commands
// ============================================================================

/// 检测本地 KB 状态:Bound / Unbound / PermissionDenied,带统计字段。
/// 前端 Settings 卡片按 status.state 切换三态 UI(7.5.A/B/C)。
#[tauri::command]
fn detect_kb_status() -> local_kb::status::KbStatus {
    let settings = settings::read_settings().unwrap_or_default();
    local_kb::status::detect_kb_status(&settings)
}

/// 在指定路径创建空 KB(已存在则只补缺失子目录,不覆盖任何已有文件)。
/// 创建成功后会自动写 settings.local_kb_root = path,local_kb_enabled = true。
#[tauri::command]
async fn create_local_kb(path: String) -> Result<local_kb::init::KbInitResult, String> {
    // tilde 展开,允许 "~/Documents/知识库" 这种
    let expanded = shellexpand::tilde(&path).into_owned();
    let target = std::path::PathBuf::from(&expanded);
    let result = local_kb::init::create_empty_kb(&target).map_err(|e| e.to_string())?;
    // 自动写回 settings,让下次 auto_detect 拿到这个路径
    let mut s = settings::read_settings().unwrap_or_default();
    s.local_kb_root = Some(path);
    s.local_kb_enabled = Some(true);
    settings::write_settings(&s).map_err(|e| format!("写 settings 失败: {}", e))?;
    Ok(result)
}

/// 启动兜底:老版本(1.x,无本地 KB 功能)升级用户的 settings 里没有 `local_kb_root` →
/// `LocalKb::auto_detect` 返回 None → 找到的法规/案例不写回 KB、本地命中省积分全失效。
/// 这里在默认路径 `~/Documents/知识库` 创建(已存在则只补目录不覆盖)+ 写回 settings,
/// 让所有用户(含老用户、新装用户)开箱即用「越用越省钱」。
/// 幂等:只在 `local_kb_root` 为空且用户没显式禁用(`local_kb_enabled != Some(false)`)时动作;
/// 返回 `Some(展示路径)` 表示本次新配置了 KB(用于前端提示),`None` = 已配置过 / 已禁用 / 失败。
/// 失败(权限 / macOS Documents TCC 拒绝等)非致命,只 dlog,不阻断启动 —— 下次启动会重试,
/// 用户也可在设置里手动新建。
fn ensure_default_local_kb() -> Option<String> {
    let mut s = settings::read_settings().ok()?;
    // 用户显式停用 → 尊重选择
    if s.local_kb_enabled == Some(false) {
        return None;
    }
    // 已配置过路径(新装走 onboarding 或老用户已手动建过)→ 不动
    let has_root = s
        .local_kb_root
        .as_deref()
        .map(str::trim)
        .map(|r| !r.is_empty())
        .unwrap_or(false);
    if has_root {
        return None;
    }
    const DEFAULT_KB: &str = "~/Documents/知识库";
    let expanded = shellexpand::tilde(DEFAULT_KB).into_owned();
    let target = std::path::PathBuf::from(&expanded);
    match local_kb::init::create_empty_kb(&target) {
        Ok(r) => {
            s.local_kb_root = Some(DEFAULT_KB.to_string());
            s.local_kb_enabled = Some(true);
            if let Err(e) = settings::write_settings(&s) {
                crate::dlog!("[startup] 写 KB settings 失败(非致命): {}", e);
                return None;
            }
            crate::dlog!(
                "[startup] 自动配置本地知识库 at {:?} (reused={})",
                r.path,
                r.reused_existing
            );
            Some(DEFAULT_KB.to_string())
        }
        Err(e) => {
            crate::dlog!("[startup] 自动创建本地知识库失败(非致命): {}", e);
            None
        }
    }
}

/// 从 zip 导入资料包合并进当前 KB。`on_conflict` 是 "skip" / "overwrite_older" / "always_overwrite"。
#[tauri::command]
async fn import_kb_from_zip(
    zip_path: String,
    on_conflict: String,
) -> Result<local_kb::share::ImportResult, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let kb = local_kb::cache::LocalKb::auto_detect(&settings)
        .ok_or_else(|| "本地知识库未启用,无法导入".to_string())?;
    let strategy = match on_conflict.as_str() {
        "skip" => local_kb::share::ConflictStrategy::Skip,
        "overwrite_older" => local_kb::share::ConflictStrategy::OverwriteOlder,
        "always_overwrite" => local_kb::share::ConflictStrategy::AlwaysOverwrite,
        _ => return Err(format!("未知冲突策略: {}", on_conflict)),
    };
    local_kb::share::import_from_zip(
        &kb,
        local_kb::share::ImportOptions {
            zip_path: std::path::PathBuf::from(zip_path),
            on_conflict: strategy,
        },
    )
    .map_err(|e| e.to_string())
}

/// 把当前 KB 的元典缓存打包成 zip 导出。
#[tauri::command]
async fn export_kb_to_zip(output_path: String) -> Result<local_kb::share::ExportResult, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let kb = local_kb::cache::LocalKb::auto_detect(&settings)
        .ok_or_else(|| "本地知识库未启用,无法导出".to_string())?;
    local_kb::share::export_to_zip(
        &kb,
        local_kb::share::ExportOptions {
            yuandian_cache_only: true,
            output_path: std::path::PathBuf::from(output_path),
            include_readme: true,
            exporter_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )
    .map_err(|e| e.to_string())
}

/// P2 · 清理「搜索 / 向量类、且超 `max_age_days` 天」的元典缓存(.md + .raw.json + index 条目)。
/// **只清检索列表**,法规 / 法条 / 案例**全文详情**与企业类一律不动(是复用资产)。
/// 显式触发(Settings 维护按钮),绝不自动跑(§7 删除需确认)。
#[tauri::command]
async fn prune_yuandian_cache(max_age_days: u32) -> Result<local_kb::cache::PruneStats, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let kb = local_kb::cache::LocalKb::auto_detect(&settings)
        .ok_or_else(|| "本地知识库未启用,无法清理".to_string())?;
    kb.prune_stale(max_age_days).map_err(|e| e.to_string())
}

/// 重建/更新本地知识库语义向量索引(整部法律按法条切片 embed)。增量:只对变了的文件重 embed。
/// 首建会 embed 整库,耗时可能分钟级 —— 显式按钮触发,避免首次 chat 检索时卡住。
/// 返回索引规模(文件数 / 切片数)。需先在设置配 embedding(硅基流动 bge-m3 免费)。
#[tauri::command]
async fn build_local_kb_semantic_index(
    app: tauri::AppHandle,
) -> Result<local_kb::semantic::KbIndexStats, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let kb = local_kb::cache::LocalKb::auto_detect(&settings)
        .ok_or_else(|| "本地知识库未启用,无法建索引".to_string())?;
    let key = settings
        .embedding_api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "未配置 embedding API key,请到设置里填写(硅基流动 bge-m3 免费)".to_string()
        })?;
    let endpoint = settings.embedding_endpoint.as_deref().unwrap_or("");
    let model = settings.embedding_model.as_deref().unwrap_or("");
    let index =
        local_kb::semantic::build_or_update_index(&kb.root, endpoint, model, key, Some(&app))
            .await?;
    Ok(index.stats())
}

/// 读本地知识库语义索引现有规模(文件数 / 切片数),不建不改。给设置页状态显示。
#[tauri::command]
async fn get_local_kb_index_stats() -> Result<local_kb::semantic::KbIndexStats, String> {
    Ok(local_kb::semantic::index_stats().await)
}

/// 后台自动增量索引:读设置(开关 + embedding key)→ 没开/没配则跳过 → 否则 spawn 后台增量。
/// 触发点:App 启动、出报告后、chat 任务完成后。非阻塞、错误只 dlog。
pub(crate) fn spawn_kb_auto_index(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let settings = settings::read_settings().unwrap_or_default();
        // 开关:None/Some(true)=开,Some(false)=关
        if settings.kb_semantic_auto_index == Some(false) {
            return;
        }
        let key = settings
            .embedding_api_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(key) = key else { return }; // 没配 embedding 不自动索引
        let Some(kb) = local_kb::cache::LocalKb::auto_detect(&settings) else {
            return;
        };
        let endpoint = settings.embedding_endpoint.as_deref().unwrap_or("");
        let model = settings.embedding_model.as_deref().unwrap_or("");
        local_kb::semantic::auto_update_index(&kb.root, endpoint, model, key, app).await;
    });
}

/// embedding 测速:embed 一批 `n` 条探针文本,返回耗时毫秒 + 向量维度 + 单条均耗。
/// 先探模型「快不快 / 会不会排队」,再决定要不要全量建索引(老板的「先拿几个文件试」)。
#[tauri::command]
async fn embedding_speed_test(n: u32) -> Result<serde_json::Value, String> {
    let settings = settings::read_settings().unwrap_or_default();
    let key = settings
        .embedding_api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "未配置 embedding API key,请到设置里填写(硅基流动 bge-m3 免费)".to_string()
        })?;
    let endpoint = settings.embedding_endpoint.as_deref().unwrap_or("");
    let model = settings.embedding_model.as_deref().unwrap_or("");
    let count = n.clamp(1, 32) as usize;
    let probes: Vec<String> = (0..count)
        .map(|i| format!("法律检索速度测试探针第{i}条:合同解除的法定情形与违约责任。"))
        .collect();
    let t0 = std::time::Instant::now();
    let v = embedding::embed(endpoint, model, key, &probes).await?;
    let ms = t0.elapsed().as_millis() as u64;
    let dim = v.first().map(|e| e.len()).unwrap_or(0);
    Ok(serde_json::json!({
        "count": count,
        "elapsed_ms": ms,
        "dim": dim,
        "per_item_ms": if count > 0 { ms / count as u64 } else { 0 },
    }))
}

/// 取当前月份元典积分账。给 Settings 元典积分卡显示「本月已用 / 上限 / KB 节省」。
#[tauri::command]
async fn get_yuandian_monthly_stats(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<db::credits::MonthlyCredits, String> {
    let ym = db::credits::current_year_month();
    db::credits::get_monthly_stats(pool.inner(), &ym)
        .await
        .map_err(db_err)
}

/// 取元典积分账总览(当月 + 上月 + 累计)。当月跨月归 0 时,前端用上月/累计补显示,避免误以为数据丢了。
#[tauri::command]
async fn get_yuandian_credits_overview(
    pool: tauri::State<'_, SqlitePool>,
) -> Result<db::credits::CreditsOverview, String> {
    let ym = db::credits::current_year_month();
    db::credits::get_overview(pool.inner(), &ym)
        .await
        .map_err(db_err)
}

/// 验证 embedding 配置:embed 一个探针词,成功返回向量维度。给设置页「验证」按钮。
#[tauri::command]
async fn verify_embedding_key(
    endpoint: String,
    model: String,
    api_key: String,
) -> Result<usize, String> {
    embedding::verify(&endpoint, &model, &api_key).await
}

// ============================================================================
// 测试
// ============================================================================

/// 启动早期(创建 webview 之前)检测系统 WebView 运行时是否可用。
///
/// `tauri::webview_version()` 直接探测底层运行时:`Ok` = 可用(直接返回,什么都不做);
/// `Err` = 缺失/装坏 —— **实际只会在 Windows 缺 Microsoft Edge WebView2 时发生**
/// (macOS / Linux 系统自带 WebKit,恒 `Ok`)。此时 app 起不了窗口,继续走只会无声闪退,
/// 所以弹一个**不依赖 webview 的原生对话框**引导用户联网下载,然后干净退出。
///
/// 不做 `#[cfg(windows)]` 门控:让本机(mac)`cargo` 门禁也能编译校验这段;mac 上恒 `Ok` → 空操作。
fn ensure_webview2_runtime() {
    if tauri::webview_version().is_ok() {
        return;
    }

    // 微软官方常青引导安装器固定链接(小巧、自动拉最新;国内一般可直连)。
    // 想换国内镜像 / 自建落地页(让用户在镜像和官方间选),只改这一个常量即可。
    const WEBVIEW2_DOWNLOAD_URL: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

    let choice = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Warning)
        .set_title("案件看板 · 缺少运行环境")
        .set_description(
            "检测到系统未安装 Microsoft Edge WebView2 运行时,案件看板无法启动。\n\n\
             点「确定」前往微软官方下载(免费、安全,约几 MB),安装完成后重新打开案件看板即可。\n\
             若下载很慢,可在浏览器搜索「WebView2 运行时 下载」从国内镜像获取。",
        )
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show();

    if choice == rfd::MessageDialogResult::Ok {
        let _ = tauri_plugin_opener::open_url(WEBVIEW2_DOWNLOAD_URL, None::<&str>);
    }
    // 没有 WebView2,继续构建 Tauri 只会崩 → 直接退出。
    std::process::exit(0);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 2026-05-26 V0.1.11:启动早期装 panic hook,把 panic 信息落到 diagnostic_log
    // ring buffer,反馈通道带出来。
    diagnostic_log::install_panic_hook();

    // 2026-06-15:创建 webview 之前先检测 WebView2 运行时。Windows 缺 WebView2 时
    // app 根本起不了窗口(老 Win10/弱网/CDN 被墙,装机时没下成)→ 这里弹原生对话框
    // 引导用户联网下载,然后退出(避免无声闪退、用户一脸懵)。详见 docs/反馈问题排查-2026-06-15.md。
    ensure_webview2_runtime();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            // 启动时同步初始化数据库连接池 + 跑 migrations
            // 用 tauri::async_runtime::block_on 避免前端在 pool 就绪前发命令
            let db_path = db::default_db_path()
                .map_err(|e| Box::<dyn std::error::Error>::from(format!("找不到数据目录: {}", e)))?
                .to_string_lossy()
                .to_string();
            let pool = tauri::async_runtime::block_on(db::init_pool(&db_path)).map_err(|e| {
                Box::<dyn std::error::Error>::from(format!("初始化数据库失败: {}", e))
            })?;

            // V0.2 D5.5 · 启动时把上次崩溃前没收尾的 chat_tasks 标 failed,
            // 让前端展示「重试」按钮。阈值 5 分钟,跟实施计划 § 6.9 对齐。
            // 用 spawn 异步跑(不阻塞 setup),失败也不阻止启动 — 内部已经走 dlog。
            {
                let pool_for_resume = pool.clone();
                tauri::async_runtime::spawn(async move {
                    let n =
                        db::chat_tasks::resume_orphaned_chat_tasks(&pool_for_resume, 5 * 60).await;
                    if n > 0 {
                        crate::dlog!("[startup] resume_orphaned_chat_tasks 标记 {} 个 orphan", n);
                    }
                });
            }

            // 团队版:已配置团队 → 后台启动监听+广播+周期同步(失败只记日志不阻启动)
            let team_pool = pool.clone();
            app.manage(pool);
            // chat 模块全局 cancel 注册表(V0.1.13+)
            app.manage(chat::ChatCancelRegistry::default());
            app.manage(TeamNetState::default());
            {
                let app_handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let has_team = settings::read_settings()
                        .ok()
                        .and_then(|s| s.team)
                        .is_some();
                    if has_team {
                        let state = app_handle.state::<TeamNetState>();
                        if let Err(e) = team_net_restart(&team_pool, state.inner()).await {
                            crate::dlog!("[startup] 团队网络启动失败: {e}");
                        }
                    }
                });
            }
            // 匿名使用遥测(只在编译期注入了 key 的 release 构建启用;dev/test 静默)。
            // fire-and-forget,失败不影响启动。
            telemetry::start();

            // V0.3:老版本(1.x)升级 / 新装用户兜底自动创建本地知识库,让「越用越省钱」
            // (法规/案例自动写回 + 本地命中)开箱即用。独立线程跑(含 FS IO + 可能的 macOS
            // Documents TCC 提示),非阻塞、非致命;新配置成功后 emit 事件让前端弹一次提示。
            {
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    if let Some(path) = ensure_default_local_kb() {
                        // 等前端挂好事件监听(启动期 webview 仍在加载,emit 太早会丢)再提示。
                        std::thread::sleep(std::time::Duration::from_secs(3));
                        let _ = app_handle.emit("local-kb-auto-created", path);
                    }
                });
            }

            // 启动后台增量索引:把上次会话期间新进缓存的法条/案例补进语义索引(增量,几秒;
            // 冷启动且量大则跳过 + 提示去设置手动重建,见 semantic::auto_update_index)。
            spawn_kb_auto_index(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_case_folder,
            import_case_folder,
            plan_import_folder,
            commit_import_folder,
            list_cases,
            get_case_with_docs,
            delete_case,
            read_text_file,
            extract_doc_text,
            extract_fields_from_text,
            open_in_default_app,
            open_url,
            reveal_in_finder,
            get_settings,
            save_settings,
            update_home_case_order,
            detect_local_readiness,
            ensure_local_ready,
            db_health,
            reaggregate_all_cases,
            global_extract_case,
            distill_case_experience,
            yuandian_basic_query,
            yuandian_deep_dive,
            add_payment,
            list_payments,
            delete_payment,
            add_todo,
            list_todos,
            list_open_todos,
            update_todo,
            delete_todo,
            add_calendar_event,
            list_calendar_events,
            delete_calendar_event,
            list_case_instances,
            add_case_instance,
            update_case_instance,
            delete_case_instance,
            delete_document,
            reextract_document,
            reextract_document_dewatermark,
            export_report_html,
            export_report_docx,
            recompute_case_extraction,
            refresh_case_files,
            preview_court_sms,
            ingest_court_sms,
            query_express,
            list_express_tracks,
            refresh_express_tracks,
            delete_express_track,
            update_workflow_status,
            update_case_overrides,
            get_deepseek_balance,
            collect_feedback_diagnostic,
            save_feedback_md,
            send_feedback_email,
            verify_mineru_key,
            verify_paddle_vl_key,
            verify_deepseek_key,
            verify_minimax_key,
            start_court_filing,
            submit_captcha_answer,
            list_court_filing_jobs,
            get_court_filing_job,
            list_lawyer_profiles,
            save_lawyer_profile,
            update_lawyer_profile,
            delete_lawyer_profile,
            set_default_lawyer,
            verify_yuandian_key,
            check_for_update,
            app_version,
            seed_demo_case_if_empty,
            yuandian_full_report,
            export_md_html,
            export_md_docx,
            export_filing_docx,
            save_editor_doc,
            case_chat,
            list_chat_history,
            cancel_chat,
            clear_chat_history,
            // MCP 数据源接入(粘贴识别 + 连接测试)
            parse_mcp_paste,
            test_mcp_server,
            // 团队版 Phase 1(LAN 接力同步)
            team_status,
            team_create,
            team_discover,
            team_join,
            team_leave,
            team_kick,
            team_set_permissions,
            team_refresh_code,
            team_sync_now,
            team_view,
            team_submit_edit,
            team_revert_edit,
            // V0.2 D7 · 本地知识库 + 元典积分
            detect_kb_status,
            create_local_kb,
            import_kb_from_zip,
            export_kb_to_zip,
            prune_yuandian_cache,
            build_local_kb_semantic_index,
            get_local_kb_index_stats,
            embedding_speed_test,
            get_yuandian_monthly_stats,
            get_yuandian_credits_overview,
            verify_embedding_key,
            // 私人专属功能(双轨发布模型;开源仓为桩命令)
            private::telemetry_get,
            private::reset_yuandian_credits,
        ])
        .on_window_event(|_window, event| {
            // App 退出时清理子进程(llama-server)
            if let tauri::WindowEvent::Destroyed = event {
                lifecycle::shutdown();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
