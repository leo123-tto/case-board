//! 本地知识库**语义检索**(向量)。V0.3.x · 治元典本地命中率低。
//!
//! 关键词检索(`search.rs`)快但不准:同义改写命中不了,且整部大法(民法典 1322 条)
//! 靠 match_count 排序定位不到对的那一条。本模块用 embedding 向量 + 余弦相似度做**语义 +
//! 条文级**检索 —— 整部法律按「第X条」切片,每条一个向量,query 直接命中最相关条文。
//!
//! 复用案件文档语义检索的基建(`crate::embedding`):`embed` / `cosine_similarity` /
//! `chunk_text` / `Chunk`。区别:本索引以**文件相对路径**为主键(KB 是散文件,不是 DB doc),
//! 索引落 `app_data_dir/embeddings/local_kb.json`(全库一份,非按 case)。
//!
//! 增量 + 失效与案件索引同源:cache_key=`mtime:size` 没变就复用旧向量;
//! signature=`endpoint|model` 变了(换 embedding 模型/维度)整库重建。
//!
//! 没配 embedding key / 网络错 → `embed` 透传真错(坑#8),接入层静默回退关键词工具。

use std::path::{Path, PathBuf};

use std::sync::{Arc, OnceLock};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use walkdir::WalkDir;

use crate::embedding::index::{chunk_text, Chunk};

/// 普通文件切片目标字数(跟案件索引一致)。
const CHUNK_TARGET_CHARS: usize = 500;
/// 单次 embed 批量上限(兼容硅基/智谱)。
const EMBED_BATCH: usize = 32;
/// 整库切片硬上限:超过只索引前 N 片并 dlog 告警(不静默截断,防索引爆炸)。
const MAX_TOTAL_CHUNKS: usize = 80_000;
/// 单文件大小上限(跟 search.rs 对齐),超过跳过。
const MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;

/// **小而精的目录**:整目录纳入(文件少、价值高)。企业=companies / 案例经验=cases-experience / 专题=topics。
const ALWAYS_DIRS: &[&str] = &["raw/cases-experience", "raw/companies", "wiki/topics"];

/// 判定「整部法律全文」的最少条文数(「第X条」标题行)。
/// 注:`raw/notes` 有 2500+ 文件、大多是判例,只挑「核心常用法整部全文」入索引(关键词+内容+去重),
/// 不整目录纳入(否则索引爆炸);`raw/yuandian-cache` 的 法规/法条/案例 详情直接收(排除 SEARCH 碎片)。
const LAW_ARTICLE_THRESHOLD: usize = 20;

/// 核心常用法关键词:文件名含其一,才作为候选核心法(老板选的「核心常用法,去重」)。
/// 偏民商 + 诉讼 + 常用司法解释 + 老板常办的建工/公司类。需要扩就往这加。
const CORE_LAW_KEYWORDS: &[&str] = &[
    "民法典",
    "民事诉讼法",
    "民诉",
    "公司法",
    "合伙企业法",
    "个人独资企业",
    "担保",
    "物权",
    "合同编",
    "买卖合同",
    "建设工程",
    "物业服务",
    "劳动合同法",
    "劳动法",
    "劳动争议",
    "社会保险法",
    "婚姻家庭",
    "继承",
    "侵权责任",
    "仲裁法",
    "企业破产",
    "破产法",
    "证券法",
    "票据法",
    "保险法",
    "行政诉讼法",
    "行政处罚法",
    "行政许可法",
    "行政复议法",
    "行政强制法",
    "国家赔偿法",
    "刑法",
    "刑事诉讼",
];

/// 文件名含这些 → 是判例/个案,**不是**法律全文,直接跳过(避免读 2500 个判决书)。
const CASE_NAME_MARKERS: &[&str] = &[
    "判决书",
    "裁定书",
    "调解书",
    "决定书",
    "案例",
    "纠纷",
    "_deprecated",
];

// =============================================================================
// 落盘结构
// =============================================================================

/// 一个文件的索引条目(以相对路径为主键)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KbFileIndex {
    pub rel_path: String,
    /// `mtime:size`,跟案件索引同思路;变了 → 重新切片 + embed。
    pub cache_key: String,
    pub chunks: Vec<Chunk>,
}

/// 整个本地 KB 的向量索引(落 `embeddings/local_kb.json`)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KbIndex {
    /// `<endpoint>|<model>`;变了 → 整库失效重建(维度也会变)。
    pub signature: String,
    /// 2026-06-15:索引对应的 KB 根目录(canonical 绝对路径字符串)。用户改了
    /// `settings.local_kb_root` 后,旧索引里的 rel_path 已对不上新根,必须视作废索引
    /// 全量重建,否则 `search_local_kb` 会召回旧根的内容(用户视角=搜到不存在的文件)。
    /// 用字符串而非 PathBuf:避开 Windows 反斜杠/正斜杠差异。`#[serde(default)]` 兼容老索引
    /// 文件(无此字段 → 空串 → 必定视为根不一致 → 一次性全量重建)。
    #[serde(default)]
    pub kb_root: String,
    pub files: Vec<KbFileIndex>,
}

/// 一条语义命中(给工具层拼结果)。
#[derive(Debug, Clone)]
pub struct KbHit {
    pub rel_path: String,
    pub score: f32,
    pub text: String,
}

/// 索引规模统计(给「重建索引」按钮显示,不含向量)。
#[derive(Debug, Clone, Serialize)]
pub struct KbIndexStats {
    pub files: u32,
    pub chunks: u32,
}

impl KbIndex {
    pub fn stats(&self) -> KbIndexStats {
        KbIndexStats {
            files: self.files.len() as u32,
            chunks: self.files.iter().map(|f| f.chunks.len()).sum::<usize>() as u32,
        }
    }
}

// =============================================================================
// 纯函数:语料采集 / cache_key / 切片 / 增量 / 排序
// =============================================================================

/// 是否纳入语义索引的文件:`.md`/`.txt`,且**不是** yuandian-cache 的 `SEARCH-*` 片段
/// (搜索结果缓存是零碎片段,会污染语义召回;整部全文 `法规-`/`法条-`/`案例-` 才要)。
pub fn is_indexable_file(rel_path: &str, file_name: &str) -> bool {
    let ext_ok = file_name
        .rsplit('.')
        .next()
        .map(|e| matches!(e.to_lowercase().as_str(), "md" | "txt"))
        .unwrap_or(false);
    if !ext_ok {
        return false;
    }
    // index.json 等非语料文件天然被 ext_ok 挡掉;这里再排除 SEARCH-* 片段。
    if rel_path.contains("yuandian-cache") && file_name.starts_with("SEARCH-") {
        return false;
    }
    true
}

/// 文件 cache_key:`mtime:size`(跟案件索引 `documents.cache_key` 同形)。取不到 mtime 用 0。
pub fn file_cache_key(meta: &std::fs::Metadata) -> String {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}:{}", mtime, meta.len())
}

/// 整部法律(含足够多「第X条」)→ **按法条切片**,每条独立 chunk;否则走 `chunk_text`。
/// 条文级切片是本功能核心:让 query 直接命中对的那一条,而不是整部 334K 一个块。
/// 按**内容**(条标题行数)判定,不看目录 —— 法律全文在 yuandian-cache 也在 raw/notes。
pub fn chunk_kb_file(_rel_path: &str, text: &str) -> Vec<String> {
    if count_article_markers(text) >= 5 {
        let arts = split_by_article(text);
        if !arts.is_empty() {
            return arts;
        }
    }
    chunk_text(text, CHUNK_TARGET_CHARS)
}

/// 文件名是否「核心常用法候选」:含核心法关键词、且不含判例/个案标记。
pub fn is_core_law_candidate(file_name: &str) -> bool {
    if CASE_NAME_MARKERS.iter().any(|m| file_name.contains(m)) {
        return false;
    }
    CORE_LAW_KEYWORDS.iter().any(|k| file_name.contains(k))
}

/// 把文件名归一成「法律规范名」用于去重:剥日期前缀 / `[国法]` / `法规-<hex>_` 前缀 /
/// `_<hex>` 后缀 / 版本括号 / `中华人民共和国` / `全文` / 扩展名 / 空白。
/// 同一部法的多个副本(`民法典全文` / `[国法]中华人民共和国民法典` / `法规-xx_中华人民共和国民法典`)→ 同一 key。
pub fn normalize_law_name(file_name: &str) -> String {
    // 去扩展名
    let mut s: String = file_name
        .strip_suffix(".md")
        .or_else(|| file_name.strip_suffix(".txt"))
        .unwrap_or(file_name)
        .to_string();
    // 去日期前缀 YYYY-MM-DD-
    if s.len() >= 11 {
        let b = s.as_bytes();
        if b[0..4].iter().all(|c| c.is_ascii_digit())
            && b[4] == b'-'
            && b[5..7].iter().all(|c| c.is_ascii_digit())
            && b[7] == b'-'
            && b[8..10].iter().all(|c| c.is_ascii_digit())
            && b[10] == b'-'
        {
            s = s[11..].to_string();
        }
    }
    // 去 [国法] 前缀(可能带空格)
    s = s.trim_start_matches("[国法]").trim_start().to_string();
    // 去 yuandian 详情前缀 法规-/法条-/案例- 后跟 <hex>_
    for pfx in ["法规-", "法条-", "案例-"] {
        let stripped = s.strip_prefix(pfx).and_then(|rest| {
            rest.find('_').and_then(|us| {
                let (hex, after) = rest.split_at(us);
                if !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    Some(after[1..].to_string())
                } else {
                    None
                }
            })
        });
        if let Some(news) = stripped {
            s = news;
        }
    }
    // 去尾部 _<hex>(≥6 位)
    let tail_stripped = s.rfind('_').and_then(|us| {
        let suffix = &s[us + 1..];
        if suffix.len() >= 6 && suffix.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(s[..us].to_string())
        } else {
            None
        }
    });
    if let Some(news) = tail_stripped {
        s = news;
    }
    // 去版本括号(含 修订/修正/修改/年 的括号)
    s = strip_version_parens(&s);
    // 「X刑法修正案十一」「刑法全文」→ 归一到「刑法」:截断「修正案…」尾巴
    if let Some(i) = s.find("修正案") {
        s.truncate(i);
    }
    // 司法解释长短名归一:「最高人民法院关于适用《X》的解释」「X解释」→ 提取《》内法名 + 「解释」
    s = normalize_interpretation(&s);
    // 去 中华人民共和国 / 全文 / 书名号 / 空白(含全角)
    s = s
        .replace("中华人民共和国", "")
        .replace("全文", "")
        .replace(['《', '》'], "");
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 司法解释长短名归一:「最高人民法院关于适用《中华人民共和国民事诉讼法》的解释」→「民事诉讼法解释」,
/// 让它跟简称「民诉法解释」之外的长名互相对齐(简称如「民诉」无法对齐,只能靠这步收一部分)。
fn normalize_interpretation(s: &str) -> String {
    if !s.contains("解释") && !s.contains("规定") {
        return s.to_string();
    }
    // 取《》里的法名 + 尾缀(解释/规定)
    if let (Some(a), Some(b)) = (s.find('《'), s.find('》')) {
        if a < b {
            let inner = &s[a + '《'.len_utf8()..b];
            let suffix = if s.contains("解释") {
                "解释"
            } else {
                "规定"
            };
            return format!("{inner}{suffix}");
        }
    }
    s.to_string()
}

/// 去掉含修订/修正/修改/年份的括号片段(全角半角都认)。
fn strip_version_parens(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut buf = String::new();
    let mut depth = 0u32;
    for c in s.chars() {
        match c {
            '(' | '（' => {
                depth += 1;
                buf.clear();
            }
            ')' | '）' if depth > 0 => {
                depth -= 1;
                // 括号内不含版本/年份关键词 → 保留(回填);否则丢弃
                if !buf.contains('修') && !buf.contains('年') {
                    out.push('(');
                    out.push_str(&buf);
                    out.push(')');
                }
                buf.clear();
            }
            _ if depth > 0 => buf.push(c),
            _ => out.push(c),
        }
    }
    out.push_str(&buf); // 未闭合的残留
    out
}

/// 去重:同一 `normalize_law_name` 的多个副本只留一个 —— 留**条文最多**(最全),并列取路径最短。
/// 入参 `(canonical, articles, rel)`,返回保留的 `rel` 集合(HashSet 便于 collect 过滤)。
pub fn dedup_law_rels(items: &[(String, usize, String)]) -> std::collections::HashSet<String> {
    use std::collections::HashMap;
    let mut best: HashMap<&str, (usize, &str)> = HashMap::new(); // canonical -> (articles, rel)
    for (canon, arts, rel) in items {
        let e = best.entry(canon.as_str()).or_insert((*arts, rel.as_str()));
        let better = *arts > e.0 || (*arts == e.0 && rel.len() < e.1.len());
        if better {
            *e = (*arts, rel.as_str());
        }
    }
    best.values().map(|(_, rel)| rel.to_string()).collect()
}

/// 数「第X条」条标题行的数量(判断是不是整部法律)。
fn count_article_markers(text: &str) -> usize {
    text.lines().filter(|l| is_article_head(l)).count()
}

/// 一行是否「条标题行」:去掉行首空白(含全角空格 U+3000,Rust `is_whitespace` 覆盖)后,
/// 形如 `第<中文数字/数字>条…`。避免句中引用(如「适用第五百条规定」,第前面是 CJK 字)被误切。
fn is_article_head(line: &str) -> bool {
    let t = line.trim_start();
    let Some(rest) = t.strip_prefix('第') else {
        return false;
    };
    // 取到第一个「条」之前的部分,必须非空且全是数字/中文数字
    let Some(pos) = rest.find('条') else {
        return false;
    };
    is_article_number(&rest[..pos])
}

/// 「第」和「条」之间是否只有数字 / 中文数字(允许少量空格)。空则否。
fn is_article_number(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        return false;
    }
    t.chars()
        .all(|c| c.is_ascii_digit() || "一二三四五六七八九十百千零〇两".contains(c))
}

/// 按法条边界切:从一个条标题行到下一个条标题行之间为一片(含中间的款项)。
/// 第一个条标题行之前的序言(标题/章节)丢弃。
pub fn split_by_article(text: &str) -> Vec<String> {
    // 先定位所有条标题行的行号
    let lines: Vec<&str> = text.lines().collect();
    let heads: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_article_head(l))
        .map(|(i, _)| i)
        .collect();
    if heads.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(heads.len());
    for (k, &start) in heads.iter().enumerate() {
        let end = heads.get(k + 1).copied().unwrap_or(lines.len());
        let piece = lines[start..end].join("\n");
        let piece = piece.trim();
        if piece.is_empty() {
            continue;
        }
        // 单条仍可能很长(附带列表),超目标再保底切,避免极端长 chunk。
        if piece.chars().count() > CHUNK_TARGET_CHARS * 4 {
            out.extend(chunk_text(piece, CHUNK_TARGET_CHARS * 2));
        } else {
            out.push(piece.to_string());
        }
    }
    out
}

/// 增量计划:返回 (复用的 rel_path, 需重新 embed 的 rel_path)。signature 变 → 全部重建。
pub fn plan_update(
    existing: &KbIndex,
    new_signature: &str,
    current: &[(String, String)], // (rel_path, cache_key)
) -> (Vec<String>, Vec<String>) {
    let sig_ok = existing.signature == new_signature;
    let mut reuse = Vec::new();
    let mut embed = Vec::new();
    for (rel, ck) in current {
        let can_reuse = sig_ok
            && existing
                .files
                .iter()
                .find(|f| &f.rel_path == rel)
                .map(|f| &f.cache_key == ck && !f.chunks.is_empty())
                .unwrap_or(false);
        if can_reuse {
            reuse.push(rel.clone());
        } else {
            embed.push(rel.clone());
        }
    }
    (reuse, embed)
}

/// 余弦排序，返回 top-N。
pub fn rank_hits(index: &KbIndex, query_vec: &[f32], top_n: usize) -> Vec<KbHit> {
    let mut scored: Vec<KbHit> = Vec::new();
    for f in &index.files {
        for c in &f.chunks {
            scored.push(KbHit {
                rel_path: f.rel_path.clone(),
                score: crate::embedding::cosine_similarity(query_vec, &c.vector),
                text: c.text.clone(),
            });
        }
    }
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(top_n);
    scored
}

// =============================================================================
// 落盘 + 网络编排
// =============================================================================

fn index_path() -> Result<PathBuf, String> {
    let base = crate::db::app_data_dir().map_err(|e| format!("无法定位 app data dir: {e}"))?;
    Ok(base.join("embeddings").join("local_kb.json"))
}

async fn load_index() -> KbIndex {
    let Ok(path) = index_path() else {
        return KbIndex::default();
    };
    match tokio::fs::read_to_string(&path).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => KbIndex::default(),
    }
}

/// 当前 KB 根的 canonical 绝对路径字符串(失败退回原路径)。Windows 上 canonicalize
/// 还会归一化盘符大小写,顺带消除"同根不同大小写"误判。
fn canonical_root(kb_root: &Path) -> String {
    kb_root
        .canonicalize()
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| kb_root.to_string_lossy().into_owned())
}

/// 加载索引,并在「KB 根已切换」时把旧索引视作废索引(清空 signature+files),避免旧根的
/// rel_path 残留导致 `search_local_kb` 召回不存在的文件。返回 (索引, 当前根 canonical 字符串)。
/// 两个建索引入口(手动 build_or_update_index + 自动 maybe_auto_index)共用,确保两条路径
/// 都在根切换时全量重建,不只修其一。
async fn load_index_for_root(kb_root: &Path) -> (KbIndex, String) {
    let current_root = canonical_root(kb_root);
    let mut existing = load_index().await;
    if existing.kb_root != current_root {
        if !existing.files.is_empty() {
            crate::dlog!(
                "[kb-semantic] KB 根切换:{} → {},清空旧索引 {} 个文件,全量重建",
                existing.kb_root,
                current_root,
                existing.files.len()
            );
        }
        existing.signature.clear();
        existing.files.clear();
        existing.kb_root = current_root.clone();
    }
    (existing, current_root)
}

/// 内存缓存:索引可能上百 MB,每次检索都从磁盘读+parse 会卡几秒。
/// 缓存按索引文件 mtime 失效;重建后 `invalidate_cache()` 主动清,下次检索重载一次。
struct CacheEntry {
    mtime: std::time::SystemTime,
    index: Arc<KbIndex>,
}
fn index_cache() -> &'static RwLock<Option<CacheEntry>> {
    static C: OnceLock<RwLock<Option<CacheEntry>>> = OnceLock::new();
    C.get_or_init(|| RwLock::new(None))
}

async fn invalidate_cache() {
    *index_cache().write().await = None;
}

/// 读索引(内存缓存优先,按 mtime 失效)。检索热路径用,避免每次读百 MB。
async fn load_index_cached() -> Arc<KbIndex> {
    let Ok(path) = index_path() else {
        return Arc::new(KbIndex::default());
    };
    let mtime = tokio::fs::metadata(&path)
        .await
        .ok()
        .and_then(|m| m.modified().ok());
    if let Some(mt) = mtime {
        let g = index_cache().read().await;
        if let Some(e) = g.as_ref() {
            if e.mtime == mt {
                return e.index.clone();
            }
        }
    }
    let idx = load_index().await;
    let arc = Arc::new(idx);
    if let Some(mt) = mtime {
        *index_cache().write().await = Some(CacheEntry {
            mtime: mt,
            index: arc.clone(),
        });
    }
    arc
}

async fn save_index(index: &KbIndex) -> Result<(), String> {
    let path = index_path()?;
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("建 embeddings 目录失败: {e}"))?;
    }
    let json = serde_json::to_string(index).map_err(|e| format!("序列化 KB 索引失败: {e}"))?;
    tokio::fs::write(&path, json)
        .await
        .map_err(|e| format!("写 KB 索引失败: {e}"))?;
    Ok(())
}

/// 采集语料,覆盖**三块:法条 + 案例 + 企业**。返回 (rel_path, abs_path, cache_key)。
/// - 企业 = `raw/companies`(整目录);案例经验 = `raw/cases-experience`(整目录);专题 = `wiki/topics`。
/// - 案例(元典)= `yuandian-cache/案例-*`(get_case_detail 详情,元典 id 唯一,直接收)。
/// - 法条 = `yuandian-cache/法规-·法条-` + `raw/notes` 核心常用法全文,**跨目录去重**(同一部法多副本留最全)。
fn collect_corpus(kb_root: &Path) -> Vec<(String, PathBuf, String)> {
    let root = match kb_root.canonicalize() {
        Ok(p) => p,
        Err(_) => kb_root.to_path_buf(),
    };
    let mut out: Vec<(String, PathBuf, String)> = Vec::new();

    // ① 小而精目录:整目录纳入(企业档案 / 办案经验 / 专题)
    for dir in ALWAYS_DIRS {
        collect_dir_all(&root, dir, &mut out);
    }

    // 法律去重池(法规/法条,跨 yuandian-cache 与 raw/notes 去重)
    let mut law_candidates: Vec<(String, usize, String)> = Vec::new(); // (canonical, articles, rel)
    let mut law_meta: std::collections::HashMap<String, (PathBuf, String)> =
        std::collections::HashMap::new(); // rel -> (abs, cache_key)
    let mut push_law = |rel: String, abs: PathBuf, ck: String, file_name: &str, arts: usize| {
        law_candidates.push((normalize_law_name(file_name), arts, rel.clone()));
        law_meta.insert(rel, (abs, ck));
    };

    // ② yuandian-cache 详情:案例 直接收;法规/法条 进去重池;SEARCH-* 碎片排除
    let ycache = root.join("raw/yuandian-cache");
    if ycache.exists() {
        for entry in WalkDir::new(&ycache)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let Some(file_name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let rel = p
                .strip_prefix(&root)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.to_string_lossy().into_owned());
            if !is_indexable_file(&rel, file_name) {
                continue; // 排除 SEARCH-* 碎片 / 非 .md
            }
            let Ok(meta) = std::fs::metadata(p) else {
                continue;
            };
            if meta.len() > MAX_FILE_SIZE {
                continue;
            }
            if file_name.starts_with("案例-") {
                // 案例详情:元典 id 命名唯一,直接收
                out.push((rel, p.to_path_buf(), file_cache_key(&meta)));
            } else if file_name.starts_with("法规-") || file_name.starts_with("法条-") {
                let arts = count_article_markers(&std::fs::read_to_string(p).unwrap_or_default());
                push_law(rel, p.to_path_buf(), file_cache_key(&meta), file_name, arts);
            }
            // 其余(企业 SEARCH 碎片等)不在此收
        }
    }

    // ③ raw/notes 全收(排除废止法):**法律**走去重池(多副本留最全);**案例原文 / 笔记**直接索引。
    //   案例原文(判决书/裁定书…)也在这里,老板要三块齐全 → 不再排除判例。
    let notes = root.join("raw/notes");
    if notes.exists() {
        for entry in WalkDir::new(&notes)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let Some(file_name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let rel = p
                .strip_prefix(&root)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|_| p.to_string_lossy().into_owned());
            // 废止法(_deprecated_laws/)绝不索引
            if rel.contains("_deprecated") || !is_indexable_file(&rel, file_name) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(p) else {
                continue;
            };
            if meta.len() > MAX_FILE_SIZE {
                continue;
            }
            // 核心法命名 + 内容确认是整部法律 → 去重池;否则(案例原文 / 笔记)直接索引
            if is_core_law_candidate(file_name) {
                let arts = count_article_markers(&std::fs::read_to_string(p).unwrap_or_default());
                if arts >= LAW_ARTICLE_THRESHOLD {
                    push_law(rel, p.to_path_buf(), file_cache_key(&meta), file_name, arts);
                    continue;
                }
            }
            out.push((rel, p.to_path_buf(), file_cache_key(&meta)));
        }
    }

    // 法律去重:同一部法多副本只留条文最全的一个,放进 out
    // (push_law 的可变借用在上面最后一次调用后即结束,这里可直接读 law_candidates/law_meta)
    let keep = dedup_law_rels(&law_candidates);
    for rel in keep {
        if let Some((abs, ck)) = law_meta.remove(&rel) {
            out.push((rel, abs, ck));
        }
    }

    // ④ 整根目录补扫:不同系统 / 不同知识库迁移过来时,文件夹命名未必遵守
    //    `raw/*` + `wiki/*`。标准目录先走特殊逻辑(法律去重 / 元典详情过滤 SEARCH),
    //    然后把未覆盖的 .md/.txt 从整根补进来,避免自建分类漏索引。
    collect_root_remaining(&root, &mut out);

    out
}

/// 把一个目录下所有可索引文件加入 out(给 ALWAYS_DIRS 用)。
fn collect_dir_all(root: &Path, dir: &str, out: &mut Vec<(String, PathBuf, String)>) {
    let target = root.join(dir);
    if !target.exists() {
        return;
    }
    for entry in WalkDir::new(&target)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(file_name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let rel = p
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_else(|_| p.to_string_lossy().into_owned());
        if !is_indexable_file(&rel, file_name) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(p) else {
            continue;
        };
        if meta.len() > MAX_FILE_SIZE {
            continue;
        }
        out.push((rel, p.to_path_buf(), file_cache_key(&meta)));
    }
}

/// 补扫整根 KB 目录,只补当前 out 里还没有的 `.md/.txt`。
///
/// 排除:
/// - `raw/yuandian-cache`:上面已经用详情规则专门收集,这里不能把 `SEARCH-*` 碎片卷进来;
/// - `_deprecated` / `00_ARCHIVE`:沿用旧语义索引的排除口径;
/// - `.git` / `node_modules` / `target` / `dist`:明显不是知识库正文。
fn collect_root_remaining(root: &Path, out: &mut Vec<(String, PathBuf, String)>) {
    let mut seen: std::collections::HashSet<String> =
        out.iter().map(|(rel, _, _)| rel.clone()).collect();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let Some(file_name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let rel = p
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| p.to_string_lossy().replace('\\', "/"));
        if seen.contains(&rel) || should_skip_root_corpus_path(&rel) {
            continue;
        }
        if !is_indexable_file(&rel, file_name) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(p) else {
            continue;
        };
        if meta.len() > MAX_FILE_SIZE {
            continue;
        }
        seen.insert(rel.clone());
        out.push((rel, p.to_path_buf(), file_cache_key(&meta)));
    }
}

fn should_skip_root_corpus_path(rel: &str) -> bool {
    if rel == "raw/yuandian-cache" || rel.starts_with("raw/yuandian-cache/") {
        return true;
    }
    rel.split('/').any(|seg| {
        matches!(
            seg,
            "_deprecated" | "00_ARCHIVE" | ".git" | "node_modules" | "target" | "dist"
        )
    })
}

/// 进度事件名。前端 `listen("kb_index_progress", ...)` 拿 `{done, total, phase}`。
pub const PROGRESS_EVENT: &str = "kb_index_progress";

/// 懒加载 + 增量建/更新 KB 向量索引。没配 key / 网络错 → 透传，调用方静默回退。
/// `app=Some` 时按切片批次 emit 进度(给「重建索引」按钮显示 X/Y);`None`(工具内懒建)不发。
/// **每个文件 embed 完就落盘一次**:长任务可观察(文件在长大)、中断也保住已完成部分(下次增量续)。
pub async fn build_or_update_index(
    kb_root: &Path,
    endpoint: &str,
    model: &str,
    key: &str,
    app: Option<&tauri::AppHandle>,
) -> Result<KbIndex, String> {
    use tauri::Emitter;

    let sig = crate::embedding::index::signature(endpoint, model);
    // 根切换 → 全量重建(见 load_index_for_root)。
    let (existing, current_root) = load_index_for_root(kb_root).await;
    let corpus = collect_corpus(kb_root);
    let current: Vec<(String, String)> = corpus
        .iter()
        .map(|(rel, _, ck)| (rel.clone(), ck.clone()))
        .collect();
    let (reuse, to_embed) = plan_update(&existing, &sig, &current);

    let mut files: Vec<KbFileIndex> = Vec::with_capacity(corpus.len());
    // 复用未变文件
    for rel in &reuse {
        if let Some(prev) = existing.files.iter().find(|f| &f.rel_path == rel) {
            files.push(prev.clone());
        }
    }

    // 先把要 embed 的文件切片(纯本地、无网络),拿到总切片数 → 进度分母。
    let mut pending: Vec<(String, String, Vec<String>)> = Vec::new(); // (rel, cache_key, pieces)
    let mut total_chunks: usize = files.iter().map(|f| f.chunks.len()).sum();
    let mut capped = false;
    for rel in &to_embed {
        if total_chunks >= MAX_TOTAL_CHUNKS {
            capped = true;
            break;
        }
        let Some((_, abs, ck)) = corpus.iter().find(|(r, _, _)| r == rel) else {
            continue;
        };
        let text = tokio::fs::read_to_string(abs).await.unwrap_or_default();
        let mut pieces = chunk_kb_file(rel, &text);
        if pieces.is_empty() {
            continue;
        }
        let room = MAX_TOTAL_CHUNKS.saturating_sub(total_chunks);
        if pieces.len() > room {
            pieces.truncate(room);
            capped = true;
        }
        total_chunks += pieces.len();
        pending.push((rel.clone(), ck.clone(), pieces));
    }

    let grand_total: usize = pending.iter().map(|(_, _, p)| p.len()).sum();
    let mut done = 0usize;
    let emit = |phase: &str, done: usize| {
        if let Some(a) = app {
            let _ = a.emit(
                PROGRESS_EVENT,
                serde_json::json!({"done": done, "total": grand_total, "phase": phase}),
            );
        }
    };
    emit("start", 0);

    // 逐文件 embed(文件内按 EMBED_BATCH 分批,每批后报进度);每个文件完成后落盘一次。
    for (rel, ck, pieces) in pending {
        let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(pieces.len());
        for batch in pieces.chunks(EMBED_BATCH) {
            let v = crate::embedding::embed(endpoint, model, key, batch).await?;
            if v.len() != batch.len() {
                return Err(format!(
                    "embedding 返回数量不符:期望 {} 得到 {}",
                    batch.len(),
                    v.len()
                ));
            }
            vectors.extend(v);
            done += batch.len();
            emit("embedding", done);
        }
        let chunks = pieces
            .into_iter()
            .zip(vectors)
            .map(|(text, vector)| Chunk { text, vector })
            .collect();
        files.push(KbFileIndex {
            rel_path: rel,
            cache_key: ck,
            chunks,
        });
        // 每个文件完成即落盘:长任务可中断续跑 + 文件可见增长。
        let snapshot = KbIndex {
            signature: sig.clone(),
            kb_root: current_root.clone(),
            files: files.clone(),
        };
        if let Err(e) = save_index(&snapshot).await {
            crate::dlog!("[kb-semantic] 增量落盘失败: {}", e);
        }
    }
    if capped {
        crate::dlog!(
            "[kb-semantic] 切片数达上限 {} 已截断,部分文件未索引(检索仍可用,覆盖不全)",
            MAX_TOTAL_CHUNKS
        );
    }

    let index = KbIndex {
        signature: sig,
        kb_root: current_root,
        files,
    };
    // 兜底再存一次(纯复用、无 pending 时上面循环不落盘,这里确保 signature/集合落地)。
    let changed = existing.signature != index.signature
        || grand_total > 0
        || index.files.len() != existing.files.len();
    if changed {
        if let Err(e) = save_index(&index).await {
            crate::dlog!("[kb-semantic] 写索引失败: {}", e);
        }
    }
    invalidate_cache().await; // 重建后清内存缓存,下次检索重载新索引
    emit("done", grand_total);
    Ok(index)
}

/// KB 语义检索:**读已建好的索引**(内存缓存) → embed query → top-N 片段(含 rel_path)。
/// **不在这里建索引**(核心法索引可能几分钟,不能卡 chat 工具调用):索引由「重建向量索引」
/// 显式构建。索引不存在 / 空 / 跟当前 embedding 模型签名不符 → 返回空,工具层回退关键词。
pub async fn semantic_search(
    _kb_root: &Path,
    query: &str,
    top_n: usize,
    endpoint: &str,
    model: &str,
    key: &str,
) -> Result<Vec<KbHit>, String> {
    let index = load_index_cached().await;
    let cur_sig = crate::embedding::index::signature(endpoint, model);
    if index.files.is_empty() || index.signature != cur_sig {
        // 没建索引 / 换了 embedding 模型(向量维度/语义变了)→ 当未命中,提示重建
        return Ok(vec![]);
    }
    let qv = crate::embedding::embed(endpoint, model, key, &[query.to_string()]).await?;
    let qv = qv.into_iter().next().ok_or("query embedding 返回空")?;
    Ok(rank_hits(&index, &qv, top_n))
}

/// 只读现有索引规模(不建/不改);给设置页状态显示。无索引返回全 0。
pub async fn index_stats() -> KbIndexStats {
    load_index_cached().await.stats()
}

/// 冷启动(无索引)时,自动索引最多放行多少个待 embed 文件;超过则跳过自动、提示手动重建,
/// 避免新装机/换模型后后台默默 embed 几十分钟。增量补充(已有索引)不受此限。
const AUTO_COLD_MAX_FILES: usize = 40;
/// 自动索引单飞:同一时刻只跑一个,后续触发直接跳过(防报告+启动叠加重复 embed)。
static AUTO_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// **后台自动增量索引**(启动 / 出报告 / chat 完成后触发)。非阻塞、错误只 dlog。
/// 规则:① 单飞 ② 没新增就早退 ③ 冷启动且待建文件过多 → 跳过 + 发 `needs_manual` 事件让用户去设置手动重建。
pub async fn auto_update_index(
    kb_root: &Path,
    endpoint: &str,
    model: &str,
    key: &str,
    app: tauri::AppHandle,
) {
    use std::sync::atomic::Ordering;
    use tauri::Emitter;
    if AUTO_RUNNING.swap(true, Ordering::SeqCst) {
        return; // 已有自动索引在跑
    }
    let sig = crate::embedding::index::signature(endpoint, model);
    // 根切换 → 清空旧索引,避免 plan_update 拿旧根 rel_path 误判"已索引"而跳过重建。
    let (existing, _current_root) = load_index_for_root(kb_root).await;
    let corpus = collect_corpus(kb_root);
    let current: Vec<(String, String)> = corpus
        .iter()
        .map(|(rel, _, ck)| (rel.clone(), ck.clone()))
        .collect();
    let (_, to_embed) = plan_update(&existing, &sig, &current);
    let cold = existing.files.is_empty() || existing.signature != sig;
    if to_embed.is_empty() && !cold {
        AUTO_RUNNING.store(false, Ordering::SeqCst);
        return; // 没新增、签名也没变 → 无需动
    }
    if cold && to_embed.len() > AUTO_COLD_MAX_FILES {
        // 冷启动且量大:不默默后台 embed 几十分钟,提示用户去设置手动重建
        crate::dlog!(
            "[kb-semantic] 自动索引跳过:冷启动待建 {} 文件 > {},请手动重建",
            to_embed.len(),
            AUTO_COLD_MAX_FILES
        );
        let _ = app.emit(
            PROGRESS_EVENT,
            serde_json::json!({"done": 0, "total": to_embed.len(), "phase": "needs_manual"}),
        );
        AUTO_RUNNING.store(false, Ordering::SeqCst);
        return;
    }
    if let Err(e) = build_or_update_index(kb_root, endpoint, model, key, Some(&app)).await {
        crate::dlog!("[kb-semantic] 自动索引失败: {}", e);
    }
    AUTO_RUNNING.store(false, Ordering::SeqCst);
}

// =============================================================================
// 测试(纯函数,无网络)
// =============================================================================
