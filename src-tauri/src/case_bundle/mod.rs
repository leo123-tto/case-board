//! 案件资料包(双人办案材料合并)。
//!
//! 场景:复杂案件双人办案,两位律师各自电脑里有同一案件的材料,可能重复、也可能各有
//! 不同。本模块让律师 A 把案件导出成一个 zip 资料包,律师 B 导入并**合并**进自己的同名
//! 案件 —— 取材料并集、按内容去重、永不冲突。
//!
//! ## 为什么不走团队 gossip 同步
//! 团队同步是 KB 级「只读镜像」(每 10 分钟传几 KB 案件登记信息,零文件传输,2026-06-13
//! 老板拍板只读镜像)。把几十/几百 MB 的 PDF 塞进 gossip 既危险又违背只读哲学。材料合并
//! 走独立、显式、用户可控的「导出资料包 → 导入合并」通道。
//!
//! ## 无冲突的根基(单向并集 + 永不删除 + 目标方优先)
//! - **文件**:按内容 SHA-256 去重。哈希已在目标案件 → 跳过;哈希是新的 → 拷进
//!   `<app_data>/merged/<case_id>/` 并以 `source='merge'` 入库(folder 重扫不软删它,
//!   见 `db::documents::insert_merged_document`)。**只增不删**。
//! - **登记字段**:只填目标案件的空白字段(案号/概括),**绝不覆盖**目标方已有的非空值
//!   (目标方主人是权威源)。
//! - 合并是单调并集 → 双方互相合并也安全(同内容文件去重不会重复计数,各自保留自己的
//!   登记字段)。合并后建议用户「重新分析」,让合并后的材料集生成统一画像。
//!
//! ## v1 边界(诚实标注)
//! 只带过来**文件 + 案件登记**。对方手工录入、非由文件派生的数据(手敲的当事人/待办/备注)
//! 不跨机器搬运 —— 合并后的文件经抽取管线重新派生分析即可。

use crate::db::cases::{self, NewCase};
use crate::db::documents as docs_db;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

const SCHEMA_VERSION: u32 = 1;
const MANIFEST_NAME: &str = "manifest.json";
const MATERIALS_DIR: &str = "materials";
/// 单文件上限 200MB,防异常大文件撑爆 zip / 内存。
const MAX_FILE_SIZE: u64 = 200 * 1024 * 1024;

/// 资料包内一个材料文件的元信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleFile {
    /// 显示用原始文件名。
    pub name: String,
    /// zip 内相对路径(`materials/<idx>_<name>`,避免同名碰撞)。
    pub rel: String,
    /// 内容 SHA-256(十六进制),跨机器去重用。
    pub sha256: String,
    pub size: u64,
}

/// 资料包清单(zip 内 `manifest.json`)。只含案件登记信息,不含路径 / API key。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseManifest {
    pub schema: u32,
    pub name: String,
    pub case_no: Option<String>,
    pub parties: Option<String>,
    pub summary: Option<String>,
    pub exported_at: String,
    pub files: Vec<BundleFile>,
}

/// 导入预览结果(给前端选合并目标用)。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundlePreview {
    pub name: String,
    pub case_no: Option<String>,
    pub parties: Option<String>,
    pub summary: Option<String>,
    pub file_count: usize,
    /// 按案号自动匹配到的本地同一案件(建议合并目标),无匹配为 None。
    pub suggested_case_id: Option<String>,
    pub suggested_case_name: Option<String>,
}

/// 合并结果报告。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MergeReport {
    pub target_case_id: String,
    pub target_case_name: String,
    /// 是否新建了案件(未指定合并目标时)。
    pub created_new: bool,
    /// 新增的材料文件数。
    pub added: usize,
    /// 内容重复、已去重跳过的文件数。
    pub deduped: usize,
    /// 资料包里登记但实际文件读不出 / 缺失而跳过的数。
    pub skipped: usize,
    /// 跳过原因(本地展示,不再只给一个无从核对的数字)。
    pub skip_reasons: Vec<String>,
    /// 补到目标案件空白字段上的字段名(如 "案号" / "案件概括")。
    pub filled_fields: Vec<String>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// 把案件主体的「人话」当事人串拼出来(优先 agg_party_contacts 里的名字,退而用原被告 JSON)。
fn parties_string(c: &cases::Case) -> Option<String> {
    let joined = |json: &Option<String>| -> Vec<String> {
        json.as_deref()
            .and_then(|s| serde_json::from_str::<Vec<serde_json::Value>>(s).ok())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        v.as_str()
                            .map(|s| s.to_string())
                            .or_else(|| v.get("name").and_then(|n| n.as_str()).map(String::from))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let ps = joined(&c.agg_plaintiffs);
    let ds = joined(&c.agg_defendants);
    let p = ps.join("、");
    let d = ds.join("、");
    match (p.is_empty(), d.is_empty()) {
        (false, false) => Some(format!("{p} 诉 {d}")),
        (false, true) => Some(p),
        (true, false) => Some(d),
        (true, true) => None,
    }
}

/// 导出某案件为 zip 资料包。返回实际打进包的文件数。
pub async fn export_case_bundle(
    pool: &SqlitePool,
    case_id: &str,
    out_path: &Path,
) -> Result<usize, String> {
    let case = cases::get_case(pool, case_id)
        .await
        .map_err(|e| format!("读取案件失败:{e}"))?
        .ok_or_else(|| "案件不存在".to_string())?;
    let documents = docs_db::list_documents_by_case(pool, case_id)
        .await
        .map_err(|e| format!("读取案件材料失败:{e}"))?;

    let file = std::fs::File::create(out_path).map_err(|e| format!("创建资料包失败:{e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut bundle_files: Vec<BundleFile> = Vec::new();
    for (idx, doc) in documents.iter().enumerate() {
        // 只打包真实源文件(跳过 AI artifact / 已失联 / 读不出的)。
        if doc.is_ai_artifact {
            continue;
        }
        let src = PathBuf::from(&doc.source_path);
        let bytes = match std::fs::read(&src) {
            Ok(b) => b,
            Err(_) => continue, // 文件不在本机 / 读不出 → 跳过,不让整包失败
        };
        if bytes.len() as u64 > MAX_FILE_SIZE {
            continue;
        }
        let rel = format!("{MATERIALS_DIR}/{idx}_{}", doc.filename);
        zip.start_file(&rel, opts)
            .map_err(|e| format!("写资料包条目失败:{e}"))?;
        zip.write_all(&bytes)
            .map_err(|e| format!("写资料包内容失败:{e}"))?;
        bundle_files.push(BundleFile {
            name: doc.filename.clone(),
            rel,
            sha256: sha256_hex(&bytes),
            size: bytes.len() as u64,
        });
    }

    let manifest = CaseManifest {
        schema: SCHEMA_VERSION,
        name: case.name.clone(),
        case_no: case.agg_case_no.clone().or_else(|| case.case_no.clone()),
        parties: parties_string(&case),
        summary: case
            .case_summary
            .clone()
            .or_else(|| case.agg_status_text.clone()),
        exported_at: now_iso(),
        files: bundle_files,
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| format!("序列化清单失败:{e}"))?;
    zip.start_file(MANIFEST_NAME, opts)
        .map_err(|e| format!("写清单失败:{e}"))?;
    zip.write_all(manifest_json.as_bytes())
        .map_err(|e| format!("写清单内容失败:{e}"))?;
    zip.finish().map_err(|e| format!("收尾资料包失败:{e}"))?;

    Ok(manifest.files.len())
}

/// 读 zip 里的 manifest.json。
fn read_manifest(zip_path: &Path) -> Result<CaseManifest, String> {
    let file = std::fs::File::open(zip_path).map_err(|e| format!("打开资料包失败:{e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("解析资料包失败:{e}"))?;
    let mut entry = archive
        .by_name(MANIFEST_NAME)
        .map_err(|_| "资料包缺 manifest.json(不是案件资料包?)".to_string())?;
    let mut s = String::new();
    entry
        .read_to_string(&mut s)
        .map_err(|e| format!("读清单失败:{e}"))?;
    let m: CaseManifest = serde_json::from_str(&s).map_err(|e| format!("清单格式不对:{e}"))?;
    if m.schema != SCHEMA_VERSION {
        return Err(format!(
            "资料包版本 {} 与当前 {} 不匹配,请升级到同版本",
            m.schema, SCHEMA_VERSION
        ));
    }
    Ok(m)
}

/// 规整案号用于匹配(去空白)。
fn norm_case_no(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 预览资料包 + 按案号建议本地合并目标。
pub async fn preview_case_bundle(
    pool: &SqlitePool,
    zip_path: &Path,
) -> Result<BundlePreview, String> {
    let m = read_manifest(zip_path)?;

    let (mut suggested_id, mut suggested_name) = (None, None);
    if let Some(cn) = m
        .case_no
        .as_deref()
        .map(norm_case_no)
        .filter(|s| !s.is_empty())
    {
        // 在本地案件里按案号(agg_case_no / case_no)找同一案件。
        let all = cases::list_cases(pool)
            .await
            .map_err(|e| format!("读取本地案件失败:{e}"))?;
        for c in all {
            let hit = c
                .agg_case_no
                .as_deref()
                .or(c.case_no.as_deref())
                .map(norm_case_no)
                .is_some_and(|local| local == cn);
            if hit {
                suggested_id = Some(c.id.clone());
                suggested_name = Some(c.name.clone());
                break;
            }
        }
    }

    Ok(BundlePreview {
        name: m.name,
        case_no: m.case_no,
        parties: m.parties,
        summary: m.summary,
        file_count: m.files.len(),
        suggested_case_id: suggested_id,
        suggested_case_name: suggested_name,
    })
}

/// 合并资料包进目标案件(`target_case_id=None` → 新建案件)。单向并集、按内容去重、永不删除。
/// 合并完成后把新材料排队抽取,让合并后的材料集重新生成案件画像。
pub async fn merge_case_bundle(
    app: &AppHandle,
    pool: &SqlitePool,
    zip_path: &Path,
    target_case_id: Option<String>,
) -> Result<MergeReport, String> {
    let merged_root = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("取数据目录失败:{e}"))?
        .join("merged");
    let report = merge_into(pool, zip_path, target_case_id, &merged_root).await?;

    // 新材料排队抽取(让合并后的材料集重新生成案件画像)。
    if report.added > 0 {
        if let Ok(docs) = docs_db::list_documents_by_case(pool, &report.target_case_id).await {
            crate::ingest::pipeline::spawn_extraction(
                app.clone(),
                pool.clone(),
                report.target_case_id.clone(),
                docs,
                true,
            );
        }
    }
    Ok(report)
}

/// 合并核心(不依赖 AppHandle、不触发抽取),材料落 `<merged_root>/<case_id>/`。
/// 从 [`merge_case_bundle`] 拆出来便于单测(见本文件 tests)。
async fn merge_into(
    pool: &SqlitePool,
    zip_path: &Path,
    target_case_id: Option<String>,
    merged_root: &Path,
) -> Result<MergeReport, String> {
    let manifest = read_manifest(zip_path)?;

    // 1) 解析合并目标(已有案件 / 新建)。
    let mut report = MergeReport::default();
    let target = match target_case_id {
        Some(id) => cases::get_case(pool, &id)
            .await
            .map_err(|e| format!("读取目标案件失败:{e}"))?
            .ok_or_else(|| "目标案件不存在".to_string())?,
        None => {
            // 新建:source_folder 直接用合并目录(唯一 + 稳定),材料就落在这里。
            let folder = merged_root.join(Uuid::new_v4().to_string());
            std::fs::create_dir_all(&folder).map_err(|e| format!("创建合并目录失败:{e}"))?;
            let name = if manifest.name.trim().is_empty() {
                "合并导入的案件".to_string()
            } else {
                manifest.name.clone()
            };
            let c = cases::create_case(
                pool,
                NewCase {
                    name,
                    case_type: "诉讼".to_string(),
                    source_folder: folder.to_string_lossy().into_owned(),
                },
            )
            .await
            .map_err(|e| format!("新建案件失败:{e}"))?;
            report.created_new = true;
            c
        }
    };
    report.target_case_id = target.id.clone();
    report.target_case_name = target.name.clone();

    // 2) 目标案件现有文件的内容哈希集合(现算,不依赖任何 content_hash 列/migration)。
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let existing_docs = docs_db::list_documents_by_case(pool, &target.id)
        .await
        .map_err(|e| format!("读取目标案件材料失败:{e}"))?;
    for d in &existing_docs {
        if let Ok(bytes) = std::fs::read(&d.source_path) {
            seen.insert(sha256_hex(&bytes));
        }
    }

    // 3) 合并材料落盘目录:<merged_root>/<target_case_id>/
    let dest_dir = merged_root.join(&target.id);
    std::fs::create_dir_all(&dest_dir).map_err(|e| format!("创建合并目录失败:{e}"))?;

    // 4) 逐个材料:内容去重 → 拷盘 → 以 source='merge' 入库。
    let file = std::fs::File::open(zip_path).map_err(|e| format!("打开资料包失败:{e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("解析资料包失败:{e}"))?;
    for bf in &manifest.files {
        // 资料包内已被去重的同内容文件,也跳过(防同包重复)。
        if seen.contains(&bf.sha256) {
            report.deduped += 1;
            continue;
        }
        let mut bytes = Vec::new();
        let read_ok = archive
            .by_name(&bf.rel)
            .ok()
            .and_then(|mut e| e.read_to_end(&mut bytes).ok())
            .is_some();
        if !read_ok || bytes.is_empty() {
            report.skipped += 1;
            report
                .skip_reasons
                .push(format!("{}:资料包内文件缺失或为空", bf.name));
            continue;
        }
        // 二次校验内容哈希,顺带防包内同内容不同名重复。
        let actual = sha256_hex(&bytes);
        if seen.contains(&actual) {
            report.deduped += 1;
            continue;
        }
        let dest = unique_dest(&dest_dir, &bf.name);
        if let Err(e) = std::fs::write(&dest, &bytes) {
            report.skipped += 1;
            report
                .skip_reasons
                .push(format!("{}:无法落盘({})", bf.name, e.kind()));
            continue;
        }
        let dest_str = dest.to_string_lossy().into_owned();
        docs_db::insert_merged_document(
            pool,
            &target.id,
            &dest_str,
            &bf.name,
            bytes.len() as i64,
            Some(&now_iso()),
        )
        .await
        .map_err(|e| format!("入库合并文件失败:{e}"))?;
        seen.insert(actual);
        report.added += 1;
    }

    // 5) 登记字段「只补空白、不覆盖」(目标方主人是权威源)。
    if report.added > 0 || report.created_new {
        if target.case_no.as_deref().unwrap_or("").trim().is_empty() {
            if let Some(cn) = manifest.case_no.as_deref().filter(|s| !s.trim().is_empty()) {
                if cases::set_case_no_if_empty(pool, &target.id, cn)
                    .await
                    .unwrap_or(0)
                    > 0
                {
                    report.filled_fields.push("案号".to_string());
                }
            }
        }
        if target
            .case_summary
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            if let Some(sm) = manifest.summary.as_deref().filter(|s| !s.trim().is_empty()) {
                if cases::set_summary_if_empty(pool, &target.id, sm)
                    .await
                    .unwrap_or(0)
                    > 0
                {
                    report.filled_fields.push("案件概括".to_string());
                }
            }
        }
    }

    Ok(report)
}

/// 清洗文件名,保证**跨平台可写**(尤其 Windows)。
/// Mac/Linux 的文件名可能含 Windows 非法字符(`< > : " / \ | ? *`、控制符),或以点/空格结尾
/// (Windows 会拒)。把非法字符换成 `_`、去尾部点和空格、空了兜底 `file`。
/// 这是 B(Windows)导入 A(Mac)资料包时不让文件因文件名非法而静默丢失的关键。
fn sanitize_filename(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let trimmed = s.trim_end_matches([' ', '.']);
    if trimmed.len() != s.len() {
        s = trimmed.to_string();
    }
    if s.is_empty() {
        s = "file".to_string();
    }
    let stem = Path::new(&s)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
            });
    if reserved {
        s.insert(0, '_');
    }
    s
}

/// 在 `dir` 下为 `name` 找一个不覆盖现有文件的目标路径(先清洗文件名,冲突则加 `_1` `_2` …)。
fn unique_dest(dir: &Path, name: &str) -> PathBuf {
    let safe = sanitize_filename(name);
    let candidate = dir.join(&safe);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(&safe);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|s| s.to_str());
    for i in 1.. {
        let fname = match ext {
            Some(e) => format!("{stem}_{i}.{e}"),
            None => format!("{stem}_{i}"),
        };
        let c = dir.join(fname);
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}
