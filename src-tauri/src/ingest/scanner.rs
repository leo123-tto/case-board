//! 文件夹扫描器(纯规则,不调 LLM)。
//!
//! 设计基于真实案件目录结构观察(详见 conversation history + `docs/`):
//! - 顶层目录天然是阶段化的:`立案材料 / 一审 / 二审 / 执行 / 证据材料 / 身份信息`
//! - AI 产物(总览/调查/精要/汇报)单独识别,**不混在普通文档里**
//! - `_archive / 归档 / .DS_Store / node_modules` 等噪音目录/文件全部忽略

use serde::Serialize;
use std::path::Path;
use walkdir::WalkDir;

/// 扫描出来的单个文档元数据。
///
/// 注意:**只记录路径,不复制原文件**。这是 CaseBoard 的核心铁律。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ScannedDoc {
    /// 原文件绝对路径(只读引用,不复制)
    pub source_path: String,
    /// 文件名(不含路径)
    pub filename: String,
    /// 阶段:立案 / 一审 / 二审 / 再审 / 执行 / 证据 / 身份信息 / None
    pub stage: Option<String>,
    /// 类别:起诉状 / 判决书 / 笔录 / ... / None
    pub category: Option<String>,
    /// 是否是 AI 生成的中间产物(.md/.html 含"总览"等关键词)
    pub is_ai_artifact: bool,
    /// 文件大小(字节)
    pub size_bytes: u64,
    /// 文件最后修改时间(ISO 8601),2026-05-23 晚十 加 — 用于缓存键
    pub modified_at: Option<String>,
}

/// 一次扫描的完整结果。
///
/// 过去 `WalkDir::flatten()` 会把权限不足、目录瞬时失联等错误直接吞掉，用户只会看到
/// “扫描完成但材料变少”。主导入/刷新链路现在读取本报告并把 warning 透传到前端；老的
/// `scan_folder*` 包装仍保留，供不需要改变接口的内部调用复用。
#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanReport {
    pub docs: Vec<ScannedDoc>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ScanOptions {
    pub reference_materials: bool,
}

/// 这些**文件名**直接忽略(macOS/Windows 噪音)
const IGNORED_FILES: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini", ".gitkeep"];

/// 这些**目录名**整个跳过(归档、依赖、版本控制)
const IGNORED_DIRS: &[&str] = &[
    "_archive",
    "归档",
    ".git",
    "node_modules",
    ".idea",
    ".vscode",
];

/// 根据**路径**(注意是路径,不是文件名)识别阶段。
///
/// 规则:遍历路径的每一段,首次命中即返回。顺序按"具体优先"排:
/// 再审 > 二审 > 一审 > 仲裁 > 立案 > 执行 > 证据 > 身份
fn classify_stage(path: &Path) -> Option<String> {
    // 路径里的中文段
    let segments: Vec<String> = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    let joined = segments.join("/");

    // 注意顺序:更具体的先匹配,避免"二审"被"一审"误命中
    if joined.contains("再审") {
        return Some("再审".into());
    }
    if joined.contains("二审") || joined.contains("2审") {
        return Some("二审".into());
    }
    if joined.contains("一审") || joined.contains("1审") {
        return Some("一审".into());
    }
    // 2026-06-11 审级模型:劳动仲裁等(诉讼前置程序),放一审后立案前
    if joined.contains("仲裁") {
        return Some("仲裁".into());
    }
    if joined.contains("立案") || joined.contains("起诉材料") {
        return Some("立案".into());
    }
    if joined.contains("执行") {
        return Some("执行".into());
    }
    if joined.contains("证据") || joined.contains("物证") {
        return Some("证据".into());
    }
    if joined.contains("身份") || joined.contains("主体") {
        return Some("身份信息".into());
    }
    None
}

/// 根据**文件名**识别类别(诉讼文书类型)。
fn classify_category(filename: &str) -> Option<String> {
    let f = filename;
    // 顺序很重要:更具体/优先级高的放前面
    // 注释里的 [R&D] 标记来自 2026-05-23 跑 5 个真实案件发现的命名习惯

    // 仲裁类(2026-06-11 审级模型加)— 放最前:"仲裁申请书"含"申请",
    // "仲裁裁决书"含"裁决",必须先于诉讼类规则匹配,避免被吞
    if f.contains("仲裁") {
        if f.contains("裁决") {
            return Some("仲裁裁决书".into());
        }
        if f.contains("申请") {
            return Some("仲裁申请书".into());
        }
        if f.contains("答辩") {
            return Some("仲裁答辩状".into());
        }
        if f.contains("受理") {
            return Some("仲裁受理通知".into());
        }
        if f.contains("开庭") {
            return Some("仲裁开庭通知".into());
        }
        if f.contains("笔录") {
            return Some("仲裁庭审笔录".into());
        }
        return Some("仲裁文书".into());
    }

    // 诉状类
    if f.contains("民事诉状") || f.contains("起诉状") || f.contains("要素式诉状") {
        return Some("起诉状".into());
    }
    if f.contains("上诉状") {
        return Some("上诉状".into());
    }
    if f.contains("反诉") {
        return Some("反诉状".into());
    }
    if f.contains("答辩") {
        return Some("答辩状".into());
    }
    if f.contains("管辖权异议") {
        return Some("管辖权异议".into()); // [R&D] 03-杰瑞典当
    }

    // 裁判文书
    if f.contains("判决书") {
        return Some("判决书".into());
    }
    if f.contains("裁定书") {
        return Some("裁定书".into());
    }
    if f.contains("调解书") {
        return Some("调解书".into());
    }
    if f.contains("调解协议") {
        return Some("调解书".into());
    }

    // 程序类
    if f.contains("受理案件通知") || f.contains("案件受理通知") || f.contains("受理通知")
    {
        return Some("受理通知".into()); // 跟 aggregator 优先级一致
    }
    if f.contains("应诉通知") {
        return Some("应诉通知".into());
    }
    if f.contains("举证通知") {
        return Some("举证通知".into());
    }
    if f.contains("传票") {
        return Some("传票".into());
    }
    if f.contains("开庭通知") || f.contains("开庭传票") {
        return Some("开庭通知".into());
    }
    if f.contains("送达地址") {
        return Some("送达地址确认书".into());
    }
    if f.contains("送达回证") {
        return Some("送达回证".into());
    }
    if f.contains("延期申请") {
        return Some("延期申请".into()); // [R&D] 03-杰瑞典当
    }
    if f.contains("出庭函") {
        return Some("出庭函".into()); // [R&D] 案件 05
    }

    // 笔录类
    if f.contains("庭审笔录") {
        return Some("庭审笔录".into());
    }
    if f.contains("询问笔录") {
        return Some("询问笔录".into());
    }
    if f.contains("笔录") || f.contains("谈话") {
        return Some("笔录".into());
    }

    // 律师工作产物 [R&D 发现的高频文档]
    if f.contains("代理合同") || f.contains("委托合同") || f.contains("委托书") {
        return Some("委托合同".into());
    }
    if f.contains("代理意见") || f.contains("代理词") {
        return Some("代理意见".into());
    }
    if f.contains("辩护词") {
        return Some("辩护词".into());
    }
    if f.contains("民事诉讼案件收案呈批") || f.contains("收案呈批") {
        return Some("收案呈批表".into()); // [R&D] 5 个案件每个都有
    }
    if f.contains("办案笔记") {
        return Some("办案笔记".into()); // [R&D] 01/03
    }
    if f.contains("风险告知") {
        return Some("风险告知".into()); // [R&D] 04/05
    }
    if f.contains("律师工作反馈") || f.contains("反馈卡") {
        return Some("反馈卡".into()); // [R&D] 04/05
    }
    if f.contains("介绍信") {
        return Some("介绍信".into()); // [R&D] 03
    }
    if f.contains("律师函") {
        return Some("律师函".into());
    }
    if f.contains("催告函") || f.contains("催款") {
        return Some("催告函".into());
    }
    if f.contains("敦促履约") || f.contains("严正通知") {
        return Some("催告函".into()); // [R&D] 05
    }
    if f.contains("诉讼思路") {
        return Some("办案笔记".into()); // [R&D] 05
    }

    // 证据类
    if f.contains("证据清单") || f.contains("证据目录") || f.contains("举证清单") {
        return Some("证据清单".into());
    }
    if f.contains("邮寄证据") || (f.contains("邮寄") && f.contains("记录")) {
        return Some("邮寄证据".into()); // [R&D] 03
    }

    // 保全 + 执行
    if f.contains("财产保全") || f.contains("保全申请") {
        return Some("财产保全".into()); // 跟 aggregator 优先级一致
    }
    if f.contains("申请执行") || f.contains("强制执行") {
        return Some("执行申请".into());
    }
    if f.contains("执行通知") {
        return Some("执行通知".into());
    }
    if f.contains("执行查询") || f.contains("查控申请") {
        return Some("查控申请".into()); // [R&D] 05
    }
    if f.contains("限消") || f.contains("限制消费") {
        return Some("限制消费令".into());
    }
    if f.contains("失信") {
        return Some("失信被执行人".into());
    }
    if f.contains("终本") {
        return Some("终本裁定".into());
    }

    // 财务
    if f.contains("诉讼费") {
        return Some("诉讼费".into());
    }
    if f.contains("缴费通知") || f.contains("缴费") {
        return Some("缴费通知".into());
    }
    if f.contains("发票") {
        return Some("发票".into());
    }
    if f.contains("收据") || f.contains("收款凭证") {
        return Some("收据".into());
    }

    // 基础合同/证据
    if f.contains("合同") {
        return Some("合同".into());
    }
    if f.contains("协议") {
        return Some("协议".into());
    }
    if f.contains("欠条") {
        return Some("欠条".into());
    }
    if f.contains("借条") {
        return Some("借条".into());
    }
    if f.contains("银行流水") {
        return Some("银行流水".into());
    }

    // 身份信息
    if f.contains("身份证") {
        return Some("身份证".into());
    }
    if f.contains("户口") {
        return Some("户口".into());
    }
    if f.contains("营业执照") {
        return Some("营业执照".into());
    }
    if f.contains("授权委托") {
        return Some("授权委托书".into());
    }

    None
}

/// 判断是否是 AI 跑出来的中间产物(总览/调查/精要/汇报/yuandian)。
fn is_ai_artifact(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    let is_text_doc = lower.ends_with(".md") || lower.ends_with(".html") || lower.ends_with(".htm");
    if !is_text_doc {
        return false;
    }
    // 命中任一关键词即认为是 AI 产物
    const HINTS: &[&str] = &[
        "总览", "调查", "精要", "汇报", "yuandian", "summary", "overview",
    ];
    // HINTS 全是小写 ASCII 或 CJK,lower.contains 是 filename.contains 的超集
    HINTS.iter().any(|w| lower.contains(w))
}

/// 扫描一个案件文件夹,返回所有有效文档的元数据。
///
/// 自动忽略:
/// - 噪音文件(`.DS_Store` 等)
/// - 归档目录(`_archive / 归档`)
/// - 版本控制 / 依赖 / IDE 目录
///
/// 不读取文件内容,纯元数据 + 文件名规则。
pub fn scan_folder(root: &Path) -> Vec<ScannedDoc> {
    scan_folder_with_options(root, ScanOptions::default())
}

pub fn scan_folder_with_options(root: &Path, options: ScanOptions) -> Vec<ScannedDoc> {
    let report = scan_folder_with_report(root, options);
    for warning in &report.warnings {
        crate::dlog!("[scanner] {}", warning);
    }
    report.docs
}

/// 带错误明细的扫描入口。warning 最多保留 20 条，避免异常网络盘产生上万条提示；超出部分
/// 会合并成一条计数摘要。
pub fn scan_folder_with_report(root: &Path, options: ScanOptions) -> ScanReport {
    let mut docs = Vec::new();
    let mut warnings = Vec::new();
    let mut suppressed_warnings = 0usize;

    let mut push_warning = |warning: String| {
        if warnings.len() < 20 {
            warnings.push(warning);
        } else {
            suppressed_warnings += 1;
        }
    };

    let walker = WalkDir::new(root).into_iter().filter_entry(|e| {
        // 跳过整个忽略目录(不递归进去)
        let name = e.file_name().to_string_lossy();
        !IGNORED_DIRS.iter().any(|d| name.as_ref() == *d)
    });

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                let location = error
                    .path()
                    .map(|path| path.to_string_lossy().to_string())
                    .unwrap_or_else(|| root.to_string_lossy().to_string());
                push_warning(format!("无法扫描 {}: {}", location, error));
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let filename = entry.file_name().to_string_lossy().to_string();
        if IGNORED_FILES.contains(&filename.as_str()) {
            continue;
        }
        // 隐藏文件也跳过(以 . 开头)
        if filename.starts_with('.') {
            continue;
        }

        let path = entry.path();
        let meta = entry.metadata();
        if let Err(error) = &meta {
            push_warning(format!(
                "无法读取文件信息 {}: {}",
                path.to_string_lossy(),
                error
            ));
        }
        let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_at = meta.as_ref().ok().and_then(|m| m.modified().ok()).map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        });

        let reference = options.reference_materials
            && path.components().any(|component| {
                let segment = component.as_os_str().to_string_lossy();
                ["参考材料", "参考文件", "参考案例", "类案", "检索材料"]
                    .iter()
                    .any(|keyword| segment.contains(keyword))
            });
        docs.push(ScannedDoc {
            source_path: path.to_string_lossy().to_string(),
            filename: filename.clone(),
            stage: classify_stage(path),
            category: if reference {
                Some("参考材料".into())
            } else {
                classify_category(&filename)
            },
            is_ai_artifact: is_ai_artifact(&filename),
            size_bytes,
            modified_at,
        });
    }

    if suppressed_warnings > 0 {
        warnings.push(format!(
            "另有 {} 条扫描错误未逐条显示，请检查文件夹权限或网络盘连接",
            suppressed_warnings
        ));
    }

    ScanReport { docs, warnings }
}

// ============================================================================
// 单元测试 —— 用通用/虚构的文件名,不暴露任何真实当事人/案件信息
// ============================================================================
