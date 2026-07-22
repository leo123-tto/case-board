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

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use walkdir::WalkDir;

use crate::embedding::index::{chunk_text, Chunk};

/// 普通材料的目标切片长度。法规仍严格逐条切，不受这个下限影响。
const CHUNK_TARGET_CHARS: usize = 700;
/// 案例、专题和普通笔记不保留孤立的标题/短段；会与相邻正文合并。
const MIN_PROSE_CHUNK_CHARS: usize = 120;
/// 单次 embed 批量上限(兼容硅基/智谱)。
const EMBED_BATCH: usize = 32;
/// 整库切片硬上限:超过只索引前 N 片并 dlog 告警(不静默截断,防索引爆炸)。
const MAX_TOTAL_CHUNKS: usize = 80_000;
/// 大索引若每完成一个文件就重写整份 JSON，会对 500MB 级索引产生巨量重复 IO。
/// 按切片数或文件数做有界 checkpoint：异常退出最多重做这一小段，不会整库归零。
const CHECKPOINT_CHUNKS: usize = 512;
const CHECKPOINT_FILES: usize = 25;
/// 单文件大小上限(跟 search.rs 对齐),超过跳过。
const MAX_FILE_SIZE: u64 = 5 * 1024 * 1024;
/// 向量语料/切片规则版本。旧索引仍可只读查询，但更新时不得复用旧规则生成的向量。
const INDEX_SCHEMA_VERSION: u32 = 3;

/// 判定「整部法律全文」的最少条文数（“第 X 条”标题行）。
/// 达到阈值的法规按条文正文指纹去重；普通 raw/notes 仍按原始正文进入语料。
const LAW_ARTICLE_THRESHOLD: usize = 20;

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
    #[serde(default)]
    pub schema_version: u32,
    /// 2026-06-15:索引对应的 KB 根目录(canonical 绝对路径字符串)。用户改了
    /// `settings.local_kb_root` 后,旧索引里的 rel_path 已对不上新根,必须视作废索引
    /// 全量重建,否则 `search_local_kb` 会召回旧根的内容(用户视角=搜到不存在的文件)。
    /// 用字符串而非 PathBuf:避开 Windows 反斜杠/正斜杠差异。`#[serde(default)]` 兼容老索引
    /// 文件(无此字段 → 空串 → 必定视为根不一致 → 一次性全量重建)。
    #[serde(default)]
    pub kb_root: String,
    /// 本次增量计划的目标文件/切片总量。中途退出后仍可显示稳定的「已完成 / 总量」。
    #[serde(default)]
    pub target_files: u32,
    #[serde(default)]
    pub target_chunks: u32,
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
    pub total_files: u32,
    pub total_chunks: u32,
}

impl KbIndex {
    pub fn stats(&self) -> KbIndexStats {
        let files = self.files.len() as u32;
        let chunks = self.files.iter().map(|f| f.chunks.len()).sum::<usize>() as u32;
        KbIndexStats {
            files,
            chunks,
            // 兼容旧索引：旧 JSON 没有 target_*，反序列化为 0；至少不能小于已完成量。
            total_files: self.target_files.max(files),
            total_chunks: self.target_chunks.max(chunks),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProgressCounts {
    done: usize,
    total: usize,
    remaining: usize,
}

fn calculate_progress(
    reused_chunks: usize,
    completed_pending_chunks: usize,
    pending_chunks: usize,
) -> ProgressCounts {
    let completed_pending_chunks = completed_pending_chunks.min(pending_chunks);
    let total = reused_chunks.saturating_add(pending_chunks);
    let done = reused_chunks.saturating_add(completed_pending_chunks);
    ProgressCounts {
        done,
        total,
        remaining: total.saturating_sub(done),
    }
}

fn should_checkpoint(chunks_since_last: usize, files_since_last: usize) -> bool {
    chunks_since_last >= CHECKPOINT_CHUNKS || files_since_last >= CHECKPOINT_FILES
}

static INDEX_BUILD_RUNNING: AtomicBool = AtomicBool::new(false);

/// 手动按钮与后台自动维护共用同一把进程内租约，避免设置页重进或自动任务重叠后重复 embed。
struct IndexBuildLease<'a> {
    running: &'a AtomicBool,
}

impl<'a> IndexBuildLease<'a> {
    fn acquire(running: &'a AtomicBool) -> Result<Self, String> {
        running
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| "知识库向量索引已有任务运行中，请等待当前任务完成".to_string())?;
        Ok(Self { running })
    }
}

impl Drop for IndexBuildLease<'_> {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
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
pub fn chunk_kb_file(rel_path: &str, text: &str) -> Vec<String> {
    if count_article_markers(text) >= 5 {
        let arts = split_by_article(text);
        if !arts.is_empty() {
            let title = law_display_name(rel_path);
            return arts
                .into_iter()
                .map(|article| format!("【法规：{title}】\n{article}"))
                .collect();
        }
    }
    balanced_prose_chunks(text)
}

fn law_display_name(rel_path: &str) -> String {
    let file_name = Path::new(rel_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("未命名法规");
    let mut name = file_name.to_string();
    if name.len() >= 11 {
        let bytes = name.as_bytes();
        if bytes[0..4].iter().all(u8::is_ascii_digit)
            && bytes[4] == b'-'
            && bytes[5..7].iter().all(u8::is_ascii_digit)
            && bytes[7] == b'-'
            && bytes[8..10].iter().all(u8::is_ascii_digit)
            && bytes[10] == b'-'
        {
            name = name[11..].to_string();
        }
    }
    for prefix in ["[国法]", "[元典法规]", "[元典法条]"] {
        name = name.trim_start_matches(prefix).trim_start().to_string();
    }
    for prefix in ["法规-", "法条-"] {
        if let Some(rest) = name.strip_prefix(prefix) {
            if let Some(separator) = rest.find('_') {
                let id = &rest[..separator];
                if !id.is_empty() && id.chars().all(|c| c.is_ascii_hexdigit()) {
                    name = rest[separator + 1..].to_string();
                }
            }
        }
    }
    name.trim().to_string()
}

fn balanced_prose_chunks(text: &str) -> Vec<String> {
    let mut chunks = chunk_text(text, CHUNK_TARGET_CHARS);
    let mut index = 0usize;
    while index < chunks.len() {
        if chunks.len() == 1 || chunks[index].chars().count() >= MIN_PROSE_CHUNK_CHARS {
            index += 1;
            continue;
        }
        if index + 1 < chunks.len() {
            let next = chunks.remove(index + 1);
            chunks[index].push('\n');
            chunks[index].push_str(&next);
        } else if index > 0 {
            let tail = chunks.remove(index);
            chunks[index - 1].push('\n');
            chunks[index - 1].push_str(&tail);
        } else {
            index += 1;
        }
    }
    chunks
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
    // 去来源标签前缀(可能带空格)。这些标签不属于法规正式名称。
    for prefix in ["[国法]", "[元典法规]", "[元典法条]"] {
        s = s.trim_start_matches(prefix).trim_start().to_string();
    }
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

/// 去重：同一正文指纹的多个法规副本只留一个。正文指纹由调用方传入；不同历史版本只要
/// 条文实质不同就会保留，不再因为规范化名称相同而误删。优先保留已提升的 `raw/notes`。
/// 入参 `(body_fingerprint, articles, rel)`，返回保留的 `rel` 集合。
pub fn dedup_law_rels(items: &[(String, usize, String)]) -> std::collections::HashSet<String> {
    use std::collections::HashMap;
    let mut best: HashMap<&str, (usize, &str)> = HashMap::new();
    for (fingerprint, arts, rel) in items {
        let e = best
            .entry(fingerprint.as_str())
            .or_insert((*arts, rel.as_str()));
        let better = *arts > e.0
            || (*arts == e.0
                && (corpus_source_priority(rel), rel.len())
                    < (corpus_source_priority(e.1), e.1.len()));
        if better {
            *e = (*arts, rel.as_str());
        }
    }
    best.values().map(|(_, rel)| rel.to_string()).collect()
}

fn normalized_text_digest<'a>(parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        for token in part.split_whitespace() {
            digest.update(token.as_bytes());
        }
        digest.update([0]);
    }
    format!("{:x}", digest.finalize())
}

fn law_body_fingerprint(text: &str) -> String {
    let articles = split_by_article(text);
    if articles.is_empty() {
        normalized_text_digest([text])
    } else {
        normalized_text_digest(articles.iter().map(String::as_str))
    }
}

fn corpus_source_priority(rel: &str) -> u8 {
    if rel.starts_with("raw/notes/") {
        0
    } else if rel.starts_with("wiki/topics/") || rel.starts_with("raw/cases-experience/") {
        1
    } else if rel.starts_with("raw/yuandian-cache/") {
        2
    } else {
        3
    }
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
    let sig_ok =
        existing.signature == new_signature && existing.schema_version == INDEX_SCHEMA_VERSION;
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
        if should_exclude_semantic_hit(&f.rel_path) {
            continue;
        }
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
            .then_with(|| {
                corpus_source_priority(&a.rel_path).cmp(&corpus_source_priority(&b.rel_path))
            })
            .then_with(|| a.rel_path.cmp(&b.rel_path))
    });
    let mut seen_text = std::collections::HashSet::new();
    scored.retain(|hit| {
        let normalized = hit.text.split_whitespace().collect::<String>();
        seen_text.insert(normalized)
    });
    scored.truncate(top_n);
    scored
}

fn should_exclude_semantic_hit(rel: &str) -> bool {
    let normalized = rel.replace('\\', "/");
    let is_raw_note = normalized.starts_with("raw/notes/");
    let is_detail_cache = normalized
        .strip_prefix("raw/yuandian-cache/")
        .and_then(|rest| rest.rsplit('/').next())
        .is_some_and(|name| {
            (name.starts_with("法规-") || name.starts_with("法条-") || name.starts_with("案例-"))
                && !name.starts_with("SEARCH-")
        });
    (!is_raw_note && !is_detail_cache)
        || normalized
            .split('/')
            .any(|segment| matches!(segment, "_inbox" | "_deprecated" | "00_ARCHIVE"))
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
    match load_index_at(&path).await {
        Ok(index) => index,
        Err(e) => {
            // 解析失败不能再静默伪装成「从未建过」，否则下一次会误走整库冷启动。
            crate::dlog!("[kb-semantic] 读取索引失败: {}", e);
            KbIndex::default()
        }
    }
}

fn index_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("local_kb.json");
    path.with_file_name(format!("{name}.{suffix}"))
}

async fn parse_index_file(path: &Path) -> Result<KbIndex, String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let file =
            std::fs::File::open(&path).map_err(|e| format!("读取 {} 失败:{e}", path.display()))?;
        let reader = std::io::BufReader::with_capacity(1024 * 1024, file);
        serde_json::from_reader(reader).map_err(|e| format!("解析 {} 失败:{e}", path.display()))
    })
    .await
    .map_err(|e| format!("读取 KB 索引任务失败:{e}"))?
}

/// 主文件若在旧版本的覆盖写期间被中断，优先恢复原子替换留下的最后一份完整备份。
async fn load_index_at(path: &Path) -> Result<KbIndex, String> {
    let main_existed = tokio::fs::metadata(path).await.is_ok();
    match parse_index_file(path).await {
        Ok(index) => Ok(index),
        Err(main_error) => {
            let backup = index_sidecar_path(path, "bak");
            let recovered = parse_index_file(&backup)
                .await
                .map_err(|backup_error| format!("{main_error}; 备份也不可用:{backup_error}"))?;

            // 主文件原本不存在，通常表示写入者正处于 main→backup→main 的替换窗口；
            // 此时只读备份，不能把它抢走。主文件原本存在但损坏，才执行一次恢复。
            if main_existed {
                let _ = tokio::fs::remove_file(path).await;
                if let Err(e) = tokio::fs::rename(&backup, path).await {
                    crate::dlog!("[kb-semantic] 恢复索引备份失败: {}", e);
                }
            }
            Ok(recovered)
        }
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
        // 旧索引没有 kb_root 字段。若抽样路径仍明确属于当前根，原向量可安全复用，
        // 只补根标识，不花 embedding 额度重建 5 万个切片。
        if existing.kb_root.is_empty() && legacy_index_matches_root(&existing, kb_root) {
            existing.kb_root = current_root.clone();
            if let Err(e) = save_index(&existing).await {
                crate::dlog!("[kb-semantic] 旧索引补 kb_root 失败: {}", e);
            }
            return (existing, current_root);
        }
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

/// 旧格式索引迁移安全阈值：最多抽样 64 个相对路径，至少 80% 在当前根真实存在。
fn legacy_index_matches_root(index: &KbIndex, kb_root: &Path) -> bool {
    if index.files.is_empty() {
        return false;
    }
    let sample = index.files.iter().take(64).collect::<Vec<_>>();
    let existing = sample
        .iter()
        .filter(|file| kb_root.join(&file.rel_path).is_file())
        .count();
    existing * 5 >= sample.len() * 4
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

#[derive(Serialize)]
struct KbIndexSnapshot<'a> {
    signature: &'a str,
    schema_version: u32,
    kb_root: &'a str,
    target_files: u32,
    target_chunks: u32,
    files: &'a [KbFileIndex],
}

/// 先完整写同目录临时文件并 sync，再用 backup 两段 rename 替换主文件。
/// 这兼容 Windows 的「rename 不能覆盖现有文件」，任一时刻至少保留主文件或备份之一。
async fn save_serializable_at<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("建 embeddings 目录失败: {e}"))?;
    }

    let temp = index_sidecar_path(path, "tmp");
    let backup = index_sidecar_path(path, "bak");
    {
        use std::io::{BufWriter, Write};

        let file =
            std::fs::File::create(&temp).map_err(|e| format!("创建 KB 索引临时文件失败:{e}"))?;
        let mut writer = BufWriter::new(&file);
        serde_json::to_writer(&mut writer, value).map_err(|e| format!("序列化 KB 索引失败:{e}"))?;
        writer
            .flush()
            .map_err(|e| format!("刷新 KB 索引临时文件失败:{e}"))?;
        drop(writer);
        file.sync_all()
            .map_err(|e| format!("同步 KB 索引临时文件失败:{e}"))?;
    }

    if tokio::fs::metadata(&backup).await.is_ok() {
        tokio::fs::remove_file(&backup)
            .await
            .map_err(|e| format!("清理旧 KB 索引备份失败:{e}"))?;
    }
    let had_main = tokio::fs::metadata(path).await.is_ok();
    if had_main {
        tokio::fs::rename(path, &backup)
            .await
            .map_err(|e| format!("备份现有 KB 索引失败:{e}"))?;
    }
    if let Err(e) = tokio::fs::rename(&temp, path).await {
        if had_main {
            let _ = tokio::fs::rename(&backup, path).await;
        }
        let _ = tokio::fs::remove_file(&temp).await;
        return Err(format!("安装新 KB 索引失败:{e}"));
    }
    if had_main {
        // 主文件已完整安装，备份仅用于替换窗口中的崩溃恢复，不长期占用数百 MB 磁盘。
        if let Err(e) = tokio::fs::remove_file(&backup).await {
            crate::dlog!("[kb-semantic] 清理 KB 索引备份失败: {}", e);
        }
    }
    Ok(())
}

async fn save_index_at(path: &Path, index: &KbIndex) -> Result<(), String> {
    save_serializable_at(path, index).await
}

async fn save_index(index: &KbIndex) -> Result<(), String> {
    let path = index_path()?;
    save_index_at(&path, index).await
}

async fn save_index_checkpoint(
    signature: &str,
    kb_root: &str,
    target_files: usize,
    target_chunks: usize,
    files: &[KbFileIndex],
) -> Result<(), String> {
    let path = index_path()?;
    let snapshot = KbIndexSnapshot {
        signature,
        schema_version: INDEX_SCHEMA_VERSION,
        kb_root,
        target_files: target_files.min(u32::MAX as usize) as u32,
        target_chunks: target_chunks.min(u32::MAX as usize) as u32,
        files,
    };
    save_serializable_at(&path, &snapshot).await
}

/// 采集 embedding 语料。返回 `(rel_path, abs_path, cache_key)`。
/// - 案例(元典)= `yuandian-cache/案例-*`(get_case_detail 详情,元典 id 唯一,直接收)。
/// - 法规 = `yuandian-cache/法规-·法条-` + `raw/notes` 所有整部法规；按条文正文指纹
///   跨目录去重，保留实质不同的历史版本。
/// - 普通 `raw/notes` 案例/笔记继续纳入。
/// - `wiki/*`、企业档案、办案经验卡、自建导航目录、管理文件和归档由目录/BM25 层承担，
///   不重复 embedding。也就是说，向量层只切原始或近原始的完整正文。
fn collect_corpus(kb_root: &Path) -> Vec<(String, PathBuf, String)> {
    let root = match kb_root.canonicalize() {
        Ok(p) => p,
        Err(_) => kb_root.to_path_buf(),
    };
    let mut out: Vec<(String, PathBuf, String)> = Vec::new();

    // 法律去重池(法规/法条,跨 yuandian-cache 与 raw/notes 去重)
    let mut law_candidates: Vec<(String, usize, String)> = Vec::new(); // (body fingerprint, articles, rel)
    let mut law_meta: std::collections::HashMap<String, (PathBuf, String)> =
        std::collections::HashMap::new(); // rel -> (abs, cache_key)
    let mut push_law = |rel: String, abs: PathBuf, ck: String, text: &str, arts: usize| {
        law_candidates.push((law_body_fingerprint(text), arts, rel.clone()));
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
                let text = std::fs::read_to_string(p).unwrap_or_default();
                if super::validity::is_inactive_regulation_text(&text) {
                    continue;
                }
                let arts = count_article_markers(&text);
                push_law(rel, p.to_path_buf(), file_cache_key(&meta), &text, arts);
            }
            // 其余(企业 SEARCH 碎片等)不在此收
        }
    }

    // ③ raw/notes 全收(排除废止法):所有整部法规走正文指纹去重池；案例原文 / 笔记直接索引。
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
            let text = std::fs::read_to_string(p).unwrap_or_default();
            if super::validity::is_inactive_regulation_text(&text) {
                continue;
            }
            let arts = count_article_markers(&text);
            if arts >= LAW_ARTICLE_THRESHOLD {
                push_law(rel, p.to_path_buf(), file_cache_key(&meta), &text, arts);
                continue;
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

    dedup_exact_corpus_files(out)
}

fn dedup_exact_corpus_files(
    files: Vec<(String, PathBuf, String)>,
) -> Vec<(String, PathBuf, String)> {
    let mut best: std::collections::HashMap<String, (String, PathBuf, String)> =
        std::collections::HashMap::new();
    for (rel, abs, cache_key) in files {
        let text = std::fs::read_to_string(&abs).unwrap_or_default();
        if is_low_value_semantic_text(&text) {
            continue;
        }
        let fingerprint = normalized_text_digest([text.as_str()]);
        let keep_existing = matches!(
            best.get(&fingerprint),
            Some((kept_rel, _, _))
                if (corpus_source_priority(kept_rel), kept_rel.len())
                    <= (corpus_source_priority(&rel), rel.len())
        );
        if !keep_existing {
            best.insert(fingerprint, (rel, abs, cache_key));
        }
    }
    let mut out = best.into_values().collect::<Vec<_>>();
    out.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    out
}

fn is_low_value_semantic_text(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.chars().count() < 20 {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.contains("请求参数json异常")
        || lower.contains("调用频率超过限制")
        || lower.contains("<title>502 bad gateway")
        || lower.contains("<title>503 service unavailable")
}

/// 进度事件名。前端 `listen("kb_index_progress", ...)` 拿 `{done, total, phase}`。
pub const PROGRESS_EVENT: &str = "kb_index_progress";

/// 懒加载 + 增量建/更新 KB 向量索引。没配 key / 网络错 → 透传，调用方静默回退。
/// `app=Some` 时按切片批次 emit 进度；分母始终是「已复用 + 待处理」的全量目标，
/// 所以中断续跑时会从已完成量继续，而不是把剩余量误显示成新的总量。
/// 索引按有界批次原子 checkpoint：避免 500MB 级 JSON 每个文件都全量重写，同时保住续跑进度。
pub async fn build_or_update_index(
    kb_root: &Path,
    endpoint: &str,
    model: &str,
    key: &str,
    app: Option<&tauri::AppHandle>,
) -> Result<KbIndex, String> {
    use tauri::Emitter;

    let _lease = IndexBuildLease::acquire(&INDEX_BUILD_RUNNING)?;
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

    let pending_total: usize = pending.iter().map(|(_, _, p)| p.len()).sum();
    let reused_chunks = files.iter().map(|file| file.chunks.len()).sum::<usize>();
    let target_files = files.len().saturating_add(pending.len());
    let target_chunks = reused_chunks.saturating_add(pending_total);
    let mut completed_pending = 0usize;
    let emit = |phase: &str, completed_pending: usize| {
        if let Some(a) = app {
            let progress = calculate_progress(reused_chunks, completed_pending, pending_total);
            let _ = a.emit(
                PROGRESS_EVENT,
                serde_json::json!({
                    "done": progress.done,
                    "total": progress.total,
                    "remaining": progress.remaining,
                    "phase": phase
                }),
            );
        }
    };
    emit("start", completed_pending);

    let metadata_changed = existing.signature != sig
        || existing.schema_version != INDEX_SCHEMA_VERSION
        || existing.kb_root != current_root
        || existing.target_files != target_files as u32
        || existing.target_chunks != target_chunks as u32
        || existing.files.len() != target_files
        || !to_embed.is_empty();
    let has_pending = !pending.is_empty();
    let mut last_saved_file_count = None;
    if has_pending {
        // 先持久化计划总量与已复用切片。即使第一批网络请求前退出，下次也能显示稳定总量。
        save_index_checkpoint(&sig, &current_root, target_files, target_chunks, &files).await?;
        last_saved_file_count = Some(files.len());
    }

    let mut chunks_since_checkpoint = 0usize;
    let mut files_since_checkpoint = 0usize;
    // 逐文件 embed(文件内按 EMBED_BATCH 分批,每批后报全库稳定进度)。
    for (rel, ck, pieces) in pending {
        let file_chunk_count = pieces.len();
        let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(pieces.len());
        for batch in pieces.chunks(EMBED_BATCH) {
            let v = match crate::embedding::embed(endpoint, model, key, batch).await {
                Ok(v) => v,
                Err(e) => {
                    // 已完成但尚未达到常规 checkpoint 阈值的文件也要在报错前保住。
                    if files_since_checkpoint > 0 {
                        if let Err(save_error) = save_index_checkpoint(
                            &sig,
                            &current_root,
                            target_files,
                            target_chunks,
                            &files,
                        )
                        .await
                        {
                            return Err(format!("{e}; 同时保存已完成进度失败:{save_error}"));
                        }
                    }
                    return Err(e);
                }
            };
            if v.len() != batch.len() {
                return Err(format!(
                    "embedding 返回数量不符:期望 {} 得到 {}",
                    batch.len(),
                    v.len()
                ));
            }
            vectors.extend(v);
            completed_pending += batch.len();
            emit("embedding", completed_pending);
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
        chunks_since_checkpoint += file_chunk_count;
        files_since_checkpoint += 1;
        if should_checkpoint(chunks_since_checkpoint, files_since_checkpoint) {
            save_index_checkpoint(&sig, &current_root, target_files, target_chunks, &files).await?;
            last_saved_file_count = Some(files.len());
            chunks_since_checkpoint = 0;
            files_since_checkpoint = 0;
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
        schema_version: INDEX_SCHEMA_VERSION,
        kb_root: current_root,
        target_files: target_files.min(u32::MAX as usize) as u32,
        target_chunks: target_chunks.min(u32::MAX as usize) as u32,
        files,
    };
    // 兜底保存最后一个未满阈值的批次；纯复用且元数据没变化时不重写数百 MB 文件。
    if last_saved_file_count != Some(index.files.len()) || (!has_pending && metadata_changed) {
        save_index(&index).await?;
    }
    invalidate_cache().await; // 重建后清内存缓存,下次检索重载新索引
    emit("done", pending_total);
    Ok(index)
}

/// KB 语义检索:**读已建好的索引**(内存缓存) → embed query → top-N 片段(含 rel_path)。
/// **不在这里建索引**(核心法索引可能几分钟,不能卡 chat 工具调用):索引由「重建向量索引」
/// 显式构建。索引不存在 / 空 / 跟当前 embedding 模型签名不符 → 返回空,工具层回退关键词。
pub async fn semantic_search(
    kb_root: &Path,
    query: &str,
    top_n: usize,
    endpoint: &str,
    model: &str,
    key: &str,
) -> Result<Vec<KbHit>, String> {
    let mut index = load_index_cached().await;
    let current_root = canonical_root(kb_root);
    if index.kb_root != current_root {
        if index.kb_root.is_empty() && legacy_index_matches_root(&index, kb_root) {
            // 兼容当前机器已有的无根标识索引：补标识并原地保存，不重做 embedding。
            let mut migrated = (*index).clone();
            migrated.kb_root = current_root.clone();
            save_index(&migrated).await?;
            invalidate_cache().await;
            index = Arc::new(migrated);
        } else {
            // 根不一致时绝不返回旧库命中；调用层会回退 BM25，并提示用户重建索引。
            return Ok(vec![]);
        }
    }
    let cur_sig = crate::embedding::index::signature(endpoint, model);
    if index.files.is_empty() || index.signature != cur_sig {
        // 没建索引 / 换了 embedding 模型(向量维度/语义变了)→ 当未命中,提示重建
        return Ok(vec![]);
    }
    let qv = crate::embedding::embed(endpoint, model, key, &[query.to_string()]).await?;
    let qv = qv.into_iter().next().ok_or("query embedding 返回空")?;
    let overfetch = top_n.saturating_mul(4).max(top_n);
    Ok(filter_inactive_semantic_hits(
        kb_root,
        rank_hits(&index, &qv, overfetch),
        top_n,
    ))
}

fn filter_inactive_semantic_hits(kb_root: &Path, mut hits: Vec<KbHit>, top_n: usize) -> Vec<KbHit> {
    hits.retain(|hit| !super::validity::is_inactive_regulation_file(&kb_root.join(&hit.rel_path)));
    hits.truncate(top_n);
    hits
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
    let cold = existing.files.is_empty()
        || existing.signature != sig
        || existing.schema_version != INDEX_SCHEMA_VERSION;
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
