//! `LocalKb` 主入口 + 元典缓存读写。
//!
//! MD 模板与 index.json 结构跟 Python `~/.claude/skills/yuandian-legal-search/
//! scripts/cache.py` **严格对齐**(D2.C 契约 — 改了请同步改 Python skill)。
//!
//! 关键对齐点:
//! 1. `cached_at` 格式 `"%Y-%m-%d %H:%M:%S"`(本地时间,无时区)
//! 2. `query_params` MD 里用 `json.dumps(params, ensure_ascii=False)` —— **不** sort_keys
//!    (跟 `_query_hash` 不同,Python 端这里就是普通 dumps)
//! 3. 文件名 `SEARCH-{hash}.md`(搜索结果)或 `{type}-{id}_{safe_name}.md`(详情)
//! 4. `safe_name`:替换 `/` → `／`,空格 → `_`,截前 40 字符
//! 5. index.json 是 `{<hash>: {path, query_type, summary, cached_at}}` 顶层 map

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{Local, NaiveDateTime};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::hash;
use super::KbError;
use crate::settings::Settings;

/// KB 主入口。`auto_detect` 拿到一个实例就保证 `raw/yuandian-cache/` 子目录可读写。
pub struct LocalKb {
    /// `<local_kb_root>` 绝对路径(tilde 展开后)
    pub root: PathBuf,
    /// `<root>/raw/yuandian-cache/`
    pub yuandian_cache_dir: PathBuf,
    /// `<yuandian_cache_dir>/index.json`
    pub index_path: PathBuf,
}

impl LocalKb {
    /// 三态自动检测:返回 `None` 表示当前 settings 下 KB 不可用(不存在 / 未配置 /
    /// 权限拒 / IO 错误 — 全部静默,前端 chat 可以无缝降级到"不查本地 KB")。
    ///
    /// 在 Settings 卡片 UI 里要区分三态的话,用 [`detect_kb_status`] 拿明细。
    pub fn auto_detect(settings: &Settings) -> Option<Self> {
        if settings.local_kb_enabled == Some(false) {
            return None;
        }
        let root_raw = settings.local_kb_root.as_deref()?;
        if root_raw.trim().is_empty() {
            return None;
        }
        let expanded = shellexpand::tilde(root_raw).into_owned();
        let root = PathBuf::from(expanded);
        if !root.exists() || !root.is_dir() {
            return None;
        }
        let cache_dir = root.join("raw").join("yuandian-cache");
        if std::fs::create_dir_all(&cache_dir).is_err() {
            return None;
        }
        let index_path = cache_dir.join("index.json");
        Some(LocalKb {
            root,
            yuandian_cache_dir: cache_dir,
            index_path,
        })
    }

    /// 检查缓存。返回 `Some` = 命中(Fresh/Permanent),`None` = 未命中(含过期)。
    pub fn check_cache(
        &self,
        query_type: &str,
        params: &Value,
    ) -> Option<(CacheHit, CacheHitFreshness)> {
        let key = hash::query_hash(query_type, params);
        let idx = self.load_index().ok()?;
        let entry = idx.get(&key)?;
        let file_path = self.yuandian_cache_dir.join(&entry.path);
        if !file_path.exists() {
            return None;
        }

        let cached_at = parse_cached_at(&entry.cached_at).ok()?;
        let freshness = match ttl_for(query_type) {
            None => CacheHitFreshness::Permanent,
            Some(ttl_days) => {
                let days_since = (Local::now().naive_local() - cached_at).num_days();
                if days_since <= ttl_days as i64 {
                    CacheHitFreshness::Fresh
                } else {
                    // 过期即视为未命中(D6-4:原 allow_stale/Stale 降级路径从无调用方触发,已移除)
                    return None;
                }
            }
        };
        let markdown = std::fs::read_to_string(&file_path).ok()?;
        Some((
            CacheHit {
                path: file_path,
                markdown,
                cached_at,
                query_type: query_type.to_string(),
                summary: entry.summary.clone(),
            },
            freshness,
        ))
    }

    /// 缓存搜索结果。MD 模板跟 Python `cache.py::cache_search_result` **逐字节对齐**。
    pub fn save_search(
        &self,
        query_type: &str,
        params: &Value,
        results: &[Value],
        summary: &str,
    ) -> Result<PathBuf, KbError> {
        let key = hash::query_hash(query_type, params);
        let file_name = format!("SEARCH-{}.md", key);
        let file_path = self.yuandian_cache_dir.join(&file_name);
        let now = now_local_str();
        // 注意:这里不能用 hash::canonical_json_str —— Python 端 MD 里是普通 dumps,无 sort_keys。
        let params_str = serde_json::to_string(params).map_err(KbError::from)?;
        let result_count = results.len();

        let mut content = String::new();
        content.push_str("---\n");
        content.push_str(&format!("cached_at: {}\n", now));
        content.push_str(&format!("query_type: {}\n", query_type));
        content.push_str(&format!("query_params: {}\n", params_str));
        content.push_str(&format!("summary: {}\n", summary));
        content.push_str(&format!("result_count: {}\n", result_count));
        content.push_str("---\n\n");
        content.push_str(&format!("# 元典检索缓存: {}\n\n", query_type));
        content.push_str(&format!("**查询时间:** {}\n", now));
        content.push_str(&format!("**查询参数:** `{}`\n", params_str));
        content.push_str(&format!("**结果数量:** {}\n\n", result_count));
        content.push_str("---\n\n");
        for (i, r) in results.iter().enumerate() {
            let idx_1based = i + 1;
            if let Some(obj) = r.as_object() {
                content.push_str(&format!("### 结果 {}\n", idx_1based));
                for (k, v) in obj.iter() {
                    if k == "content" {
                        continue;
                    }
                    if value_is_truthy(v) {
                        content.push_str(&format!("- **{}**: {}\n", k, value_inline(v)));
                    }
                }
                content.push('\n');
            } else {
                content.push_str(&format!("### 结果 {}\n", idx_1based));
                content.push_str(&format!("{}\n\n", value_inline(r)));
            }
        }
        std::fs::write(&file_path, &content)?;

        let mut idx = self.load_index().unwrap_or_default();
        idx.insert(
            key.clone(),
            IndexEntry {
                path: file_name,
                query_type: query_type.to_string(),
                summary: summary.to_string(),
                cached_at: now,
            },
        );
        self.save_index(&idx)?;
        Ok(file_path)
    }

    /// V0.2.2 · 写「完整响应」sidecar(`SEARCH-{hash}.raw.json`),与 `.md` 索引并存。
    ///
    /// `.md` 是给 Python skill / 人读的轻量索引(丢 `content`、带 `cached_at` 时间戳);
    /// sidecar 存元典完整响应原文,**专给 LLM 工具结果用**:① 含 `content` 全文(修复 KB
    /// 命中时信息比直连 API 少的问题)② 无时间戳、字节稳定(命中/未命中返回一致,利于
    /// DeepSeek 前缀缓存)。Python 端不认识 `.raw.json`,只读 index + `.md`,故互通不受影响。
    pub fn save_raw_response(
        &self,
        query_type: &str,
        params: &Value,
        body: &str,
    ) -> Result<(), KbError> {
        let key = hash::query_hash(query_type, params);
        let path = self
            .yuandian_cache_dir
            .join(format!("SEARCH-{}.raw.json", key));
        std::fs::write(path, body)?;
        Ok(())
    }

    /// 读 sidecar 完整响应。老缓存 / 写失败没有 sidecar 时返回 `None`。
    /// V0.2.2 起调用方(`try_kb_hit`)据此当 miss 重新调 API 拿完整响应,**不再**回退残缺 `.md`。
    pub fn load_raw_response(&self, query_type: &str, params: &Value) -> Option<String> {
        let key = hash::query_hash(query_type, params);
        let path = self
            .yuandian_cache_dir
            .join(format!("SEARCH-{}.raw.json", key));
        std::fs::read_to_string(path).ok()
    }

    /// P1 · 详情类(法规 / 法条 / 案例**全文**)写「可读命名全文 MD」+ 索引,跟 hash 命名的
    /// `.raw.json` sidecar 并存。
    ///
    /// 为什么要单独一个写法:`save_search` 对详情类(`data` 是对象、不是数组)会丢 `content`、
    /// 写出 `result_count:0` 的空壳 `.md` —— 人 / Python skill / 后续提升入库 review 时看着像空垃圾
    /// (Hermes 就是这么把真有全文的缓存误删的)。本方法改写成**带全文、按名字命名**
    /// (仿 Python `cache_detail_result`:`{类型}-{id}_{名}.md`),让目录里的全文成品一眼可读、可提升。
    ///
    /// 程序命中仍走 sidecar(`try_kb_hit` → `load_raw_response`,字节稳定利于前缀缓存);本 MD 只给
    /// 人读 / 治理。但 index entry 的 `path` 指向本 MD,故 `check_cache` 命中校验、status 统计、
    /// P2 提升入库都看得到它 —— 修复全文缓存「只有裸 `.raw.json`、无 `.md` 无索引」的隐身孤儿问题。
    pub fn save_detail(
        &self,
        query_type: &str,
        params: &Value,
        type_label: &str,
        obj_id: &str,
        display_name: &str,
        body_md: &str,
    ) -> Result<PathBuf, KbError> {
        let safe_id = sanitize_detail_name(obj_id);
        let safe_name = sanitize_detail_name(display_name);
        let historical_suffix = params
            .get("refer_date")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|date| !date.is_empty())
            .map(sanitize_detail_name);
        let file_name = if let Some(date) = historical_suffix {
            format!("{}-{}_{}_历史-{}.md", type_label, safe_id, safe_name, date)
        } else {
            format!("{}-{}_{}.md", type_label, safe_id, safe_name)
        };
        let file_path = self.yuandian_cache_dir.join(&file_name);
        let now = now_local_str();

        let mut content = String::new();
        content.push_str("---\n");
        content.push_str(&format!("cached_at: {}\n", now));
        content.push_str(&format!("query_type: {}\n", query_type));
        content.push_str(&format!("type: {}\n", type_label));
        content.push_str(&format!("id: {}\n", obj_id));
        content.push_str(&format!("name: {}\n", display_name));
        content.push_str("---\n\n");
        content.push_str(&format!("# {}\n\n", display_name));
        content.push_str(body_md);
        if !body_md.ends_with('\n') {
            content.push('\n');
        }
        std::fs::write(&file_path, &content)?;

        // 索引 key 用 query_hash(跟 try_kb_hit 的 check_cache 一致),path 指向本可读 MD。
        let key = hash::query_hash(query_type, params);
        let mut idx = self.load_index().unwrap_or_default();
        idx.insert(
            key,
            IndexEntry {
                path: file_name,
                query_type: query_type.to_string(),
                summary: display_name.to_string(),
                cached_at: now,
            },
        );
        self.save_index(&idx)?;
        Ok(file_path)
    }

    fn load_index(&self) -> Result<HashMap<String, IndexEntry>, KbError> {
        if !self.index_path.exists() {
            return Ok(HashMap::new());
        }
        let raw = std::fs::read_to_string(&self.index_path)?;
        if raw.trim().is_empty() {
            return Ok(HashMap::new());
        }
        let parsed: HashMap<String, IndexEntry> = serde_json::from_str(&raw)?;
        Ok(parsed)
    }

    fn save_index(&self, idx: &HashMap<String, IndexEntry>) -> Result<(), KbError> {
        // 跟 Python `_save_index` 对齐:超过 2000 条按 cached_at 降序裁到 1500
        let trimmed: HashMap<String, IndexEntry> = if idx.len() > 2000 {
            let mut entries: Vec<(&String, &IndexEntry)> = idx.iter().collect();
            entries.sort_by(|a, b| b.1.cached_at.cmp(&a.1.cached_at));
            entries
                .into_iter()
                .take(1500)
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        } else {
            idx.clone()
        };
        // Python 用 `json.dump(..., ensure_ascii=False, indent=2)`,Rust 这里也要带缩进 + 中文不转义。
        // serde_json::to_string_pretty 默认 ensure_ascii=False(直接输出 UTF-8),缩进 2 空格。
        let json = serde_json::to_string_pretty(&trimmed)?;
        std::fs::write(&self.index_path, json)?;
        Ok(())
    }

    /// P2 · 物理清理「**搜索 / 向量类、且超期**」的缓存(`.md` + 对应 `.raw.json` + index 条目)。
    ///
    /// 安全边界(删除是高风险动作,§7 要求显式确认 —— 本函数**只在用户显式触发时调,绝不自动跑**):
    /// - 只动 index 里登记过的、`is_prunable_search_type` 为真的搜索 / 向量 query_type;
    /// - **详情(法规 / 法条 / 案例全文)、企业类一律不清**(是复用资产);
    /// - **不删未登记在 index 的文件**(防误删 Python skill 的 `{类型}-{id}_{名}.md` detail / REPORT 等);
    /// - `.raw.json` 路径从 **hash key 推**(`SEARCH-{key}.raw.json`),不靠 string-munge `entry.path`
    ///   (P1 后详情 entry.path 已是 `法规-{id}_{名}.md`,跟 sidecar 名无关)。
    pub fn prune_stale(&self, max_age_days: u32) -> Result<PruneStats, KbError> {
        let mut idx = self.load_index().unwrap_or_default();
        let now = Local::now().naive_local();
        let stale_keys: Vec<String> = idx
            .iter()
            .filter(|(_, e)| {
                is_prunable_search_type(&e.query_type)
                    && parse_cached_at(&e.cached_at)
                        .ok()
                        .map(|t| (now - t).num_days() > max_age_days as i64)
                        .unwrap_or(false)
            })
            .map(|(k, _)| k.clone())
            .collect();

        let mut removed_files = 0u32;
        let mut removed_entries = 0u32;
        for key in &stale_keys {
            if let Some(e) = idx.get(key) {
                // .md(index 登记的 path)
                if std::fs::remove_file(self.yuandian_cache_dir.join(&e.path)).is_ok() {
                    removed_files += 1;
                }
                // 对应 .raw.json:从 hash key 推(key 本身就是 query_hash)
                let sidecar = self
                    .yuandian_cache_dir
                    .join(format!("SEARCH-{}.raw.json", key));
                if std::fs::remove_file(sidecar).is_ok() {
                    removed_files += 1;
                }
            }
            idx.remove(key);
            removed_entries += 1;
        }
        if removed_entries > 0 {
            self.save_index(&idx)?;
        }
        Ok(PruneStats {
            removed_entries,
            removed_files,
        })
    }
}

/// 哪些 query_type 属于「可清理的搜索 / 向量检索」(列表型,过期即无价值)。
/// 详情(`rh_*_detail` / `rh_case_details`)、企业(`rh_enterprise*`)**不在此列** —— 它们是复用资产。
fn is_prunable_search_type(query_type: &str) -> bool {
    matches!(
        query_type,
        "rh_ft_search"
            | "rh_fg_search"
            | "law_vector_search"
            | "rh_ptal_search"
            | "rh_qwal_search"
            | "case_vector_search"
    )
}

/// `prune_stale` 的回执(给 Settings/命令展示清了多少)。
#[derive(Debug, Clone, Serialize)]
pub struct PruneStats {
    pub removed_entries: u32,
    pub removed_files: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub path: String,
    pub query_type: String,
    pub summary: String,
    pub cached_at: String,
}

#[derive(Debug)]
pub struct CacheHit {
    pub path: PathBuf,
    pub markdown: String,
    pub cached_at: NaiveDateTime,
    pub query_type: String,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheHitFreshness {
    Fresh,
    Permanent,
}

/// 按 query_type 决定 TTL(天)。`None` = 永不过期。
/// 详 § 4.3。法规法条/案例不变(已发布判决不会变),企业 30 天,其他默认 30 天。
pub fn ttl_for(query_type: &str) -> Option<u32> {
    match query_type {
        // 法规法条 — 不过期(法律修订时元典通过 refer_date 提供时点版本)
        "rh_ft_search" | "rh_ft_detail" | "rh_fg_search" | "rh_fg_detail" | "law_vector_search" => {
            None
        }
        // 案例 — 不过期
        "rh_ptal_search" | "rh_qwal_search" | "case_vector_search" => None,
        // 法律幻觉校验 — 调用方应该用 NoCacheStrategy,这里 fallback 不缓存
        "hall_detect" => None,
        // 企业类 — 30 天,过期即未命中(重新外查)
        s if s.starts_with("rh_enterprise") => Some(30),
        _ => Some(30),
    }
}

fn now_local_str() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 详情缓存文件名安全化:仿 Python `cache_detail_result` 的 `safe_name`
/// (`/` → 全角、空格 → `_`),外加把换行换成空格(防破坏 YAML front matter 的单行),再截到 40 char
/// (按 char 不按 byte,防截断多字节中文)。只影响**文件名可读性**,不参与 hash / 索引 key。
fn sanitize_detail_name(s: &str) -> String {
    let one_line = s.replace(['\n', '\r'], " ");
    one_line
        .replace('/', "／")
        .replace(' ', "_")
        .chars()
        .take(40)
        .collect()
}

/// 解析 `cached_at`。Python check_cache 只看前 10 字符(`YYYY-MM-DD`)+ strptime,
/// 这里更精确,先按完整时间戳试,失败再按日期试(给 Python 端老缓存留兼容)。
fn parse_cached_at(s: &str) -> Result<NaiveDateTime, chrono::ParseError> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").or_else(|_| {
        let date_only = &s[..s.len().min(10)];
        chrono::NaiveDate::parse_from_str(date_only, "%Y-%m-%d")
            .map(|d| d.and_hms_opt(0, 0, 0).unwrap())
    })
}

/// Python `if v` 真值判断在 JSON 里的近似:null / "" / 0 / [] / {} 都算 falsy。
fn value_is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(true),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// 把 Value inline 成单行字符串(给 `- **k**: v` 格式用)。
fn value_inline(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        // 嵌套对象 / 数组 用 ensure_ascii=False JSON 表示(Python str(dict) 不一样,但这里走 JSON 更稳)
        _ => serde_json::to_string(v).unwrap_or_default(),
    }
}
