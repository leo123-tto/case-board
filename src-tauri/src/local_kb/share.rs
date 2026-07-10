//! V0.2 D5.5 · 本地知识库资料包导入 / 导出(zip)。
//!
//! 设计同 docs/V0.2-法律AI工作台-实施计划.md § 6.8。第一版只导出
//! `raw/yuandian-cache/` 子目录(元典 API 缓存),不含用户笔记 / 案件资料 /
//! `wiki/` `sources/` 等个性化内容 — 隐私铁律。
//!
//! 文件结构:
//!
//! ```text
//! caseboard-kb-share-YYYY-MM-DD.zip
//! ├── manifest.json   { exported_at, exporter_version, items: [{ path, cached_at, size_bytes, query_type, summary }] }
//! ├── README.md       (可选,纯说明)
//! └── yuandian-cache/
//!     ├── index.json
//!     ├── SEARCH-*.md
//!     ├── 法规-*.md
//!     ├── 案例-*.md
//!     └── 公司-*.md
//! ```
//!
//! 冲突策略:
//! - `Skip`            — 已有相同 query_hash key 直接跳过
//! - `OverwriteOlder`  — 仅当 incoming.cached_at > existing.cached_at 时覆盖
//! - `AlwaysOverwrite` — 任何冲突都覆盖
//!
//! **不抄 legal-kb ingest.md 的 4 维查重**:CaseBoard 缓存 MD 只有 query_hash 一维,
//! 简单匹配就够;复杂场景留给 legal-kb skill 自己解决。

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::cache::{IndexEntry, LocalKb};
use super::KbError;

/// 导出 zip 入参。第一版固定 `yuandian_cache_only=true`,字段保留方便后续扩。
#[derive(Debug, Clone)]
pub struct ExportOptions {
    /// 仅导出 `yuandian-cache/` 子目录(第一版固定 true)。
    /// 设计上为 false 时会一并打包 `wiki/` `sources/` 等,但本版未实现,会返回错误。
    pub yuandian_cache_only: bool,
    /// 输出 zip 文件绝对路径。
    pub output_path: PathBuf,
    /// 是否生成 README.md。
    pub include_readme: bool,
    /// 写入 manifest 的 exporter 版本(由调用方传 `env!("CARGO_PKG_VERSION")`)。
    pub exporter_version: String,
}

/// 导出结果。
#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    pub output_path: PathBuf,
    pub total_items: usize,
    pub total_size_bytes: u64,
}

/// 导入 zip 入参。
#[derive(Debug, Clone)]
pub struct ImportOptions {
    pub zip_path: PathBuf,
    pub on_conflict: ConflictStrategy,
}

/// 冲突策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictStrategy {
    Skip,
    OverwriteOlder,
    AlwaysOverwrite,
}

/// 导入结果。
#[derive(Debug, Clone, Serialize)]
pub struct ImportResult {
    pub total_in_zip: usize,
    pub added: usize,
    pub skipped: usize,
    pub overwritten: usize,
    pub failed: usize,
    pub conflicts: Vec<ConflictRecord>,
}

/// 冲突 / 失败记录,给 UI 展示一份"哪些没进 / 为什么"。
#[derive(Debug, Clone, Serialize)]
pub struct ConflictRecord {
    pub path: String,
    /// `"skip"` / `"overwrite"` / `"failed"`
    pub action: String,
    pub reason: String,
}

/// manifest.json 顶层。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    exported_at: String,
    exporter_version: String,
    yuandian_cache_only: bool,
    total_items: usize,
    total_size_bytes: u64,
    items: Vec<ManifestItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ManifestItem {
    /// 相对 zip 根目录的路径,例如 `yuandian-cache/SEARCH-abc.md`。
    path: String,
    /// 同 `index.json::IndexEntry.cached_at`(`"%Y-%m-%d %H:%M:%S"` 本地时间字符串)。
    cached_at: String,
    size_bytes: u64,
    /// 例:`rh_ft_search` / `rh_enterprise_xxx` / `法规` / `案例` / `公司`。
    query_type: String,
    summary: String,
}

// ============================================================================
// 导出
// ============================================================================

/// 把 `kb.yuandian_cache_dir` 下的所有缓存打成 zip。
///
/// 行为:
/// - 读 `index.json` 列出所有 entry;index.json 不存在(空 KB)→ 导出空包但成功。
/// - 写 `yuandian-cache/index.json`(整文件复制,保持跟 Python 端 dump 一致)。
/// - 每个 entry 对应文件按 entry.path 复制到 `yuandian-cache/<entry.path>`。
/// - manifest.json + 可选 README.md 写在 zip 根目录。
/// - entry 在 index.json 里但文件不在磁盘 → 跳过(不算 failed,Python 端可能也这样)。
pub fn export_to_zip(kb: &LocalKb, opts: ExportOptions) -> Result<ExportResult, KbError> {
    if !opts.yuandian_cache_only {
        return Err(KbError::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "第一版只支持 yuandian_cache_only=true",
        )));
    }

    if let Some(parent) = opts.output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let file = File::create(&opts.output_path)?;
    let mut zip = ZipWriter::new(file);
    let zip_opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    // 1) 读 index.json
    let index_path = kb.index_path.clone();
    let index_map: HashMap<String, IndexEntry> = if index_path.exists() {
        let raw = std::fs::read_to_string(&index_path)?;
        if raw.trim().is_empty() {
            HashMap::new()
        } else {
            serde_json::from_str(&raw)?
        }
    } else {
        HashMap::new()
    };

    let mut items: Vec<ManifestItem> = Vec::new();
    let mut total_size: u64 = 0;

    // 2) 把 index.json 原样写进 zip(即使空也写一个空 map 文件,保持结构)
    let index_blob = if index_path.exists() {
        std::fs::read(&index_path)?
    } else {
        b"{}".to_vec()
    };
    zip.start_file("yuandian-cache/index.json", zip_opts)?;
    zip.write_all(&index_blob)?;

    // 3) 每个 entry 复制
    for entry in index_map.values() {
        let on_disk = kb.yuandian_cache_dir.join(&entry.path);
        if !on_disk.exists() {
            // index 跟磁盘不一致 — 静默跳过,不阻断整次导出
            crate::dlog!(
                "[kb-share] export: index.json 指向的文件不存在,跳过:{}",
                entry.path
            );
            continue;
        }
        let bytes = std::fs::read(&on_disk)?;
        let size = bytes.len() as u64;
        let zip_path = format!("yuandian-cache/{}", entry.path);
        zip.start_file(&zip_path, zip_opts)?;
        zip.write_all(&bytes)?;

        total_size += size;
        items.push(ManifestItem {
            path: zip_path,
            cached_at: entry.cached_at.clone(),
            size_bytes: size,
            query_type: entry.query_type.clone(),
            summary: entry.summary.clone(),
        });
    }

    // 4) manifest.json
    let manifest = Manifest {
        exported_at: chrono::Local::now()
            .format("%Y-%m-%dT%H:%M:%S%:z")
            .to_string(),
        exporter_version: opts.exporter_version.clone(),
        yuandian_cache_only: true,
        total_items: items.len(),
        total_size_bytes: total_size,
        items,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)?;
    zip.start_file("manifest.json", zip_opts)?;
    zip.write_all(manifest_json.as_bytes())?;

    // 5) README.md (可选)
    if opts.include_readme {
        let readme = build_readme(&manifest);
        zip.start_file("README.md", zip_opts)?;
        zip.write_all(readme.as_bytes())?;
    }

    zip.finish()?;

    Ok(ExportResult {
        output_path: opts.output_path,
        total_items: manifest.total_items,
        total_size_bytes: manifest.total_size_bytes,
    })
}

fn build_readme(m: &Manifest) -> String {
    format!(
        "# CaseBoard 法律知识库共享包\n\n\
         - 导出时间:{}\n\
         - 导出版本:CaseBoard {}\n\
         - 条目数量:{}\n\
         - 总大小:{} 字节\n\n\
         ## 包含\n\n\
         元典 API 缓存(法规 / 案例 / 企业风险等公开数据)。\n\n\
         ## 不含\n\n\
         - 个人案件资料\n\
         - 用户笔记\n\
         - 客户信息\n\n\
         ## 导入方式\n\n\
         CaseBoard → 设置 → 本地知识库 → 导入资料包,选择本 zip。\n",
        m.exported_at, m.exporter_version, m.total_items, m.total_size_bytes,
    )
}

// ============================================================================
// 导入
// ============================================================================

/// 把 zip 内的缓存合并进 `kb.yuandian_cache_dir`。
///
/// 流程:
/// 1. 打开 zip,读 manifest.json(无 manifest → 错误);
/// 2. 读现有 index.json(没有就当空)— 把 zip 里 `yuandian-cache/index.json` 解析出来拿到 incoming 索引;
/// 3. 对 manifest.items 逐条决定 Add/Skip/Overwrite(按 ConflictStrategy + cached_at 比较);
/// 4. 把决定要进的 entry 文件解压写入,合并到现有 index.json,最后写回。
///
/// 任何单文件 IO 失败计入 `failed`,不阻断整体。
pub fn import_from_zip(kb: &LocalKb, opts: ImportOptions) -> Result<ImportResult, KbError> {
    let file = File::open(&opts.zip_path)?;
    let mut archive = ZipArchive::new(file)?;

    // 1) 读 manifest
    let manifest = read_manifest(&mut archive)?;

    // 2) 读 incoming index.json(从 zip 拿)
    let incoming_index: HashMap<String, IndexEntry> = read_zip_index(&mut archive)?;
    // 3) 读现有 index.json
    let mut current_index: HashMap<String, IndexEntry> = if kb.index_path.exists() {
        let raw = std::fs::read_to_string(&kb.index_path)?;
        if raw.trim().is_empty() {
            HashMap::new()
        } else {
            serde_json::from_str(&raw)?
        }
    } else {
        HashMap::new()
    };

    let mut result = ImportResult {
        total_in_zip: manifest.total_items,
        added: 0,
        skipped: 0,
        overwritten: 0,
        failed: 0,
        conflicts: Vec::new(),
    };

    // incoming_index 的 key 是 query_hash,跟 ManifestItem 的 path 反查:从 path "yuandian-cache/SEARCH-abc.md"
    // 提取 hash 不可靠(法规-/案例-/公司- 这些前缀走不同命名规则),所以倒查:
    // 遍历 incoming_index,直接对每个 hash key 决定 add/skip/overwrite,文件路径用 entry.path。
    // manifest 只用 total_items 做汇总,逐条入库以 incoming_index 为准。
    let actual_total = incoming_index.len();
    result.total_in_zip = actual_total;

    for (key, incoming_entry) in incoming_index.iter() {
        // D6-1:incoming_entry.path 完全是 zip 提供方可控数据。写盘前先校验路径安全
        //(拒绝绝对路径 / `..` / 根 / 盘符组件),防恶意共享包路径穿越覆盖 KB 目录外任意文件。
        if !is_safe_cache_relpath(&incoming_entry.path) {
            result.failed += 1;
            result.conflicts.push(ConflictRecord {
                path: incoming_entry.path.clone(),
                action: "failed".to_string(),
                reason: "不安全的路径(绝对路径或含 ..),已拒绝导入".to_string(),
            });
            continue;
        }
        let zip_inner_path = format!("yuandian-cache/{}", incoming_entry.path);
        let dest_path = kb.yuandian_cache_dir.join(&incoming_entry.path);

        let action = match current_index.get(key) {
            None => ConflictAction::Add,
            Some(existing) => match opts.on_conflict {
                ConflictStrategy::Skip => ConflictAction::Skip("已存在(Skip 策略)"),
                ConflictStrategy::OverwriteOlder => {
                    // 字符串比 `"YYYY-MM-DD HH:MM:SS"` 在同格式下天然正确
                    if incoming_entry.cached_at > existing.cached_at {
                        ConflictAction::Overwrite("incoming 更新")
                    } else {
                        ConflictAction::Skip("现有更新或相同(OverwriteOlder)")
                    }
                }
                ConflictStrategy::AlwaysOverwrite => ConflictAction::Overwrite("AlwaysOverwrite"),
            },
        };

        match action {
            ConflictAction::Skip(reason) => {
                result.skipped += 1;
                result.conflicts.push(ConflictRecord {
                    path: incoming_entry.path.clone(),
                    action: "skip".to_string(),
                    reason: reason.to_string(),
                });
                continue;
            }
            ConflictAction::Add | ConflictAction::Overwrite(_) => {
                let bytes = match read_zip_file(&mut archive, &zip_inner_path) {
                    Ok(b) => b,
                    Err(e) => {
                        result.failed += 1;
                        result.conflicts.push(ConflictRecord {
                            path: incoming_entry.path.clone(),
                            action: "failed".to_string(),
                            reason: format!("zip 读失败: {}", e),
                        });
                        continue;
                    }
                };
                if let Some(parent) = dest_path.parent() {
                    if let Err(e) = std::fs::create_dir_all(parent) {
                        result.failed += 1;
                        result.conflicts.push(ConflictRecord {
                            path: incoming_entry.path.clone(),
                            action: "failed".to_string(),
                            reason: format!("建父目录失败: {}", e),
                        });
                        continue;
                    }
                }
                if let Err(e) = std::fs::write(&dest_path, &bytes) {
                    result.failed += 1;
                    result.conflicts.push(ConflictRecord {
                        path: incoming_entry.path.clone(),
                        action: "failed".to_string(),
                        reason: format!("写文件失败: {}", e),
                    });
                    continue;
                }

                current_index.insert(key.clone(), incoming_entry.clone());
                if matches!(action, ConflictAction::Add) {
                    result.added += 1;
                } else {
                    result.overwritten += 1;
                    result.conflicts.push(ConflictRecord {
                        path: incoming_entry.path.clone(),
                        action: "overwrite".to_string(),
                        reason: if let ConflictAction::Overwrite(r) = action {
                            r.to_string()
                        } else {
                            String::new()
                        },
                    });
                }
            }
        }
    }

    // 写回 index.json — 跟 cache.rs::save_index 一致,中文不转义 + 2 空格缩进
    let pretty = serde_json::to_string_pretty(&current_index)?;
    if let Some(parent) = kb.index_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&kb.index_path, pretty)?;

    Ok(result)
}

enum ConflictAction {
    Add,
    Skip(&'static str),
    Overwrite(&'static str),
}

fn read_manifest<R: Read + Seek>(archive: &mut ZipArchive<R>) -> Result<Manifest, KbError> {
    let mut file = archive.by_name("manifest.json").map_err(|e| {
        KbError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("zip 缺 manifest.json: {}", e),
        ))
    })?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    let parsed: Manifest = serde_json::from_str(&buf)?;
    Ok(parsed)
}

fn read_zip_index<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
) -> Result<HashMap<String, IndexEntry>, KbError> {
    let mut file = archive.by_name("yuandian-cache/index.json").map_err(|e| {
        KbError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("zip 缺 yuandian-cache/index.json: {}", e),
        ))
    })?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    if buf.trim().is_empty() {
        return Ok(HashMap::new());
    }
    let parsed: HashMap<String, IndexEntry> = serde_json::from_str(&buf)?;
    Ok(parsed)
}

fn read_zip_file<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    inner_path: &str,
) -> Result<Vec<u8>, KbError> {
    let mut file = archive.by_name(inner_path)?;
    let mut buf = Vec::with_capacity(file.size() as usize);
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

impl From<zip::result::ZipError> for KbError {
    fn from(e: zip::result::ZipError) -> Self {
        KbError::Io(std::io::Error::other(format!("zip 错误: {}", e)))
    }
}

/// 给前端/测试用的便捷构造,生成默认导出文件名:`caseboard-kb-share-YYYY-MM-DD.zip`。
/// D6-1:校验 zip 内 index 提供的相对路径安全 —— 仅允许"正常路径段 + 当前目录(.)",
/// 拒绝绝对路径、`..` 上跳、根 `/`、盘符前缀。用于导入资料包时防路径穿越覆盖 KB 目录外文件。
fn is_safe_cache_relpath(rel: &str) -> bool {
    use std::path::{Component, Path};
    if rel.trim().is_empty() {
        return false;
    }
    let p = Path::new(rel);
    if p.is_absolute() {
        return false;
    }
    p.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

// ============================================================================
// 测试
// ============================================================================
