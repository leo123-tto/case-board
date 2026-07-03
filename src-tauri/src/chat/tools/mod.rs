//! 案件 AI 助手 V2 的工具集合(V0.2 D2-D3)。
//!
//! 30 个内置 tool 分 11 类(详 docs/V0.2-法律AI工作台-实施计划.md § 5):
//!   - 法规法条 5 (laws.rs)
//!   - 案例 4 (cases.rs)
//!   - 企业 6 (companies.rs)
//!   - 幻觉校验 1 (verify.rs · hall_detect,不缓存)
//!   - 案件文档 4 (docs.rs · sqlite + 案件 extracted_text_path;semantic.rs · 向量语义检索)
//!   - 本地知识库 3 (kb.rs · `~/Documents/知识库/` 整库;semantic_kb.rs · 语义检索)
//!   - 写作工具 2 (artifact.rs · save_artifact 文书生产 + edit_artifact 局部编辑,均 mutating)
//!   - 交互工具 1 (ask_user.rs · 选项式追问,agent_loop 拦截不进派发)
//!   - 文档维护 1 (reextract.rs · reextract_document,V0.3 触发后台重抽,mutating)
//!   - 入库工具 1 (save_kb.rs · save_company_report,企业报告入库 raw/companies/,P2,mutating)
//!   - 公开联网工具 2 (web.rs · web_search / web_fetch,只读兜底)
//!
//! 调用方:`chat::agent_loop`(D3-D4 实施)拿到 LLM 的 function_call,
//! 用 `ToolRegistry::find(name)` 查到 tool,调 `execute(args, ctx)`,
//! 把 `ToolResult.content` 回填到 LLM 的 messages。
//!
//! 三段式(本类 15 个走 KB cache 的工具):
//!   1. 调 `LocalKb::check_cache` 看本地有无命中
//!   2. miss → 调元典 API
//!   3. API 成功 → 调 `LocalKb::save_search` / `save_detail` 写回 KB
//!
//! 不走元典 cache 的工具包括:`verify_legal_citations`(实时校验)、`list_case_docs`、
//! `read_case_doc`、`find_in_document`(全部案件内查 sqlite/文件)、
//! `semantic_search_case_docs`、`search_local_kb`、`semantic_search_local_kb`、`read_kb_file`
//! (本身就是从 KB 读或查向量索引)、`save_artifact` / `edit_artifact` / `ask_user` /
//! `reextract_document` / `save_company_report` / `web_search` / `web_fetch`。

pub mod artifact;
pub mod ask_user;
pub mod cases;
pub mod companies;
pub mod docs;
pub mod kb;
pub mod law_fulltext;
pub mod laws;
pub mod reextract;
pub mod save_kb;
pub mod semantic;
pub mod semantic_kb;
pub mod verify;
pub mod web;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use thiserror::Error;

use crate::local_kb::cache::LocalKb;
use crate::settings::Settings;

/// 调 tool 时所需的运行时上下文。所有引用都是借用,生命周期 `'a` 跟 agent_loop 的
/// 一轮调度对齐。
pub struct ToolContext<'a> {
    pub pool: &'a SqlitePool,
    pub settings: &'a Settings,
    /// 当前 chat 所绑定的 case_id。某些工具(`list_case_docs` / `read_case_doc` /
    /// `find_in_document`)在 `None` 时直接报错。
    pub case_id: Option<&'a str>,
    /// V0.2 D2 的 `LocalKb` 实例,`None` = 用户没启用本地 KB,所有 KB-cache 路径跳过。
    pub local_kb: Option<&'a LocalKb>,
    /// Tauri `AppHandle`(cheap Arc clone),给需要触发后台任务并 emit 进度事件的工具用
    /// (如 `reextract_document` 走 `spawn_extraction`)。chat command 构造时传
    /// `Some(app.clone())`;单测 / 无 GUI 上下文传 `None`(此类工具会优雅报错)。
    pub app: Option<tauri::AppHandle>,
}

/// tool 执行结果。
///
/// 注:工具产生的引用统一走 `<CITATIONS>` 协议(由 agent_loop 解析 LLM 输出),
/// 不在 ToolResult 上单独挂 citations(旧 CitationSource 脚手架从未接线,已移除,D5-4)。
#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    /// 喂回 LLM 的文本(markdown 或 JSON,LLM 自己解析)。
    pub content: String,
    /// 元典积分消耗(KB hit 计 0)。
    pub yuandian_credits_used: u32,
    /// 是否命中本地 KB(true 时反馈 MD 加 1 计数)。
    pub kb_hit: bool,
}

impl ToolResult {
    pub fn plain(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            yuandian_credits_used: 0,
            kb_hit: false,
        }
    }
}

/// 工具执行错误。区分"参数错"和"运行时错",让 agent_loop 做不同的回退策略
/// (`InvalidArgs` 让 LLM 重写参数重试,`Runtime` 直接报给用户)。
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("参数错误:{0}")]
    InvalidArgs(String),
    #[error("当前对话没绑定案件,本工具需要 case_id")]
    NoCaseBound,
    #[error("元典 API key 未配置,无法外查;请到设置里填入")]
    NoYuandianKey,
    #[error("元典调用失败:{0}")]
    Yuandian(#[from] crate::yuandian::YuandianError),
    #[error("数据库错误:{0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("本地 KB 错误:{0}")]
    Kb(#[from] crate::local_kb::KbError),
    #[error("IO 错误:{0}")]
    Io(#[from] std::io::Error),
    #[error("内部错误:{0}")]
    Runtime(String),
}

impl serde::Serialize for ToolError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// 单个工具的 trait。dyn Tool 由 async-trait 宏支持。
#[async_trait]
pub trait Tool: Send + Sync {
    /// 工具名(英文,DeepSeek function calling 用)
    fn name(&self) -> &str;
    /// 中文 description(LLM 看到的;`include_str!` 编译期注入)
    fn description(&self) -> &str;
    /// JSON Schema 参数描述(给 DeepSeek tools 数组用)
    fn parameters_schema(&self) -> Value;
    /// 实际执行。`args` 是 LLM function_call 的 arguments(已 JSON 解析)。
    async fn execute(&self, args: &Value, ctx: &ToolContext<'_>) -> Result<ToolResult, ToolError>;

    /// 是否为 **mutating** 工具(写盘 / 改状态)。mutating 工具在 agent_loop 一轮里
    /// **串行独占**执行,read-only 工具仍并行 —— 防同轮多个改同一文书的 tool_call 在
    /// IO await 点交错导致丢更新(见 `parallel::run_parallel_subtasks`)。默认 false。
    fn is_mutating(&self) -> bool {
        false
    }

    /// DeepSeek tools 数组里的单个 entry。一般不需要 override。
    fn to_function_schema(&self) -> Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": self.description(),
                "parameters": self.parameters_schema(),
            }
        })
    }
}

/// 默认注册的全部 30 个内置工具:V0.2 的 21 个,加 V0.3 的 save_artifact / ask_user /
/// reextract_document,V0.3.3 的 semantic_search_case_docs,ADR-0003 的 edit_artifact,
/// P2 的 save_company_report(企业报告入库),以及公开联网兜底工具 web_search / web_fetch。
/// `ToolRegistry::default_v0_2()` 返回这个列表。
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    /// V0.2 全量注册。次序无关紧要,DeepSeek tools 数组无序。
    pub fn default_v0_2() -> Self {
        let tools: Vec<Box<dyn Tool>> = vec![
            // 法规法条 5
            Box::new(laws::SearchLaws),
            Box::new(laws::GetLawArticle),
            Box::new(laws::SearchRegulations),
            Box::new(laws::GetRegulationDetail),
            Box::new(laws::LawVectorSearch),
            // 案例 4
            Box::new(cases::SearchCasesNormal),
            Box::new(cases::SearchCasesAuthority),
            Box::new(cases::GetCaseDetail),
            Box::new(cases::CaseVectorSearch),
            // 企业 6
            Box::new(companies::EnterpriseSearch),
            Box::new(companies::EnterpriseAggregationSummary),
            Box::new(companies::EnterpriseBaseInfo),
            Box::new(companies::EnterpriseChangeInfo),
            Box::new(companies::EnterpriseWritList),
            Box::new(companies::EnterpriseAnnualReport),
            // 幻觉校验 1
            Box::new(verify::VerifyLegalCitations),
            // 案件文档 4(V0.3.3 加 semantic_search_case_docs · 语义检索本案全文)
            Box::new(docs::ListCaseDocs),
            Box::new(docs::ReadCaseDoc),
            Box::new(docs::FindInDocument),
            Box::new(semantic::SemanticSearchCaseDocs),
            // 本地 KB 2
            Box::new(kb::SearchLocalKb),
            Box::new(semantic_kb::SemanticSearchLocalKb),
            Box::new(kb::ReadKbFile),
            // 写作工具 2(V0.3 M1 + ADR-0003):文书生产 + 局部编辑(均 mutating)
            Box::new(artifact::SaveArtifact),
            Box::new(artifact::EditArtifact),
            // 交互工具 1(V0.3):选项式追问(agent_loop 拦截,不进 parallel 派发)
            Box::new(ask_user::AskUser),
            // 文档维护 1(V0.3):触发后台重抽某文档(mutating,会重跑 OCR/LLM 烧积分)
            Box::new(reextract::ReextractDocument),
            // 入库工具 1(P2):把企业调查报告写进本地 KB raw/companies/(mutating)
            Box::new(save_kb::SaveCompanyReport),
            // 通用联网工具 2:公开网页搜索 / 读取。只读,作为本地 KB + 元典之外的兜底。
            Box::new(web::WebSearch),
            Box::new(web::WebFetch),
        ];
        Self { tools }
    }

    pub fn find(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    /// 工具是否 mutating(写盘/改状态)。未注册的名字按 false(只读)处理,
    /// 不影响 `run_parallel_subtasks` 的派发(未注册工具会在 `find` 那里报错)。
    pub fn is_mutating(&self, name: &str) -> bool {
        self.find(name).map(|t| t.is_mutating()).unwrap_or(false)
    }

    pub fn iter(&self) -> impl Iterator<Item = &dyn Tool> {
        self.tools.iter().map(|t| t.as_ref())
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// V0.3.6 · 把外部 MCP server 发现的转发工具并入注册表(由 `mcp_bridge::connect_mcp_servers`
    /// 产出)。空 vec = 无变化。MCP 工具与内置工具一视同仁走 `find` / `execute` / schemas。
    pub fn with_mcp(mut self, extra: Vec<Box<dyn Tool>>) -> Self {
        self.tools.extend(extra);
        self
    }

    /// 给 DeepSeek `chat/completions` 请求体的 `tools` 数组用。
    pub fn to_function_schemas(&self) -> Vec<Value> {
        self.tools.iter().map(|t| t.to_function_schema()).collect()
    }
}

/// 工具共用工具函数:从 args 里安全拿一个必填 string。
pub(crate) fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| ToolError::InvalidArgs(format!("缺必填字段:{}", key)))
}

/// 可选 string(空串视为 None)。
pub(crate) fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
}

/// 可选 u32(支持数字和数字字符串)。
pub(crate) fn opt_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key).and_then(|v| {
        v.as_u64()
            .map(|n| n as u32)
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    })
}

/// 可选 bool。
pub(crate) fn opt_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

/// 拿元典 API key,空串 / 缺失返回 `NoYuandianKey`。
pub(crate) fn yuandian_key<'a>(ctx: &'a ToolContext<'_>) -> Result<&'a str, ToolError> {
    let k = ctx
        .settings
        .yuandian_api_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or(ToolError::NoYuandianKey)?;
    Ok(k)
}

// =============================================================================
// 喂 LLM 的搜索结果瘦身(ADR-0003 余波 · 2026-06-01 真机暴露:法规搜索 sidecar 250-290KB,
// 整部塞进主上下文、且工具消息打不中前缀缓存=全价。原则:整部/大段只缓存本地,**进主上下文
// 的只能是定位 + 短摘要**;要某条全文 LLM 用 get_law_article(fgid+ftnum) 取单条。
// 缓存(sidecar / .md 索引)仍存完整,瘦身只作用于**返回给 LLM 的内容**。)
// =============================================================================

/// 需要对「喂 LLM 内容」瘦身的搜索类 query_type(法规法条 / 法规 / 法规向量检索)。
const SLIM_SEARCH_TYPES: &[&str] = &["rh_ft_search", "rh_fg_search", "law_vector_search"];
/// 每条结果保留的正文上限(字符)——够 LLM 判断这是不是要的条,全文走 get_law_article。
const SLIM_CONTENT_CHARS: usize = 140;
/// 喂 LLM 时砍掉的噪音/重复字段(llm_content 与 content 重复;其余是 url/日期/分类/score)。
/// ⚠️ **保留 `id`(法条直接 id,get_law_article 首选入参)和 `tid`(阿拉伯条号,可当 ftnum)**
/// —— 它们小但是模型可靠取单条的定位锚;砍了模型只能传中文 ft_num,extract_article 会失配降级。
const SLIM_DROP_FIELDS: &[&str] = &[
    "llm_content",
    "url",
    "_score",
    "fbbm",
    "fbrq",
    "ssrq",
    "fwzh",
    "xljb_1",
    "xljb_2",
    "ftmc",
    "title",
];

/// 把搜索类元典响应瘦身成「喂 LLM 的紧凑版」:每条只留定位字段(fgmc/ft_num/fgid/sxx)+
/// 截断的 content,砍掉重复 llm_content 与一堆噪音。非搜索类返回 `None`(调用方走原样)。
fn slim_search_for_llm(query_type: &str, resp: &Value) -> Option<Value> {
    if !SLIM_SEARCH_TYPES.contains(&query_type) {
        return None;
    }
    let arr = resp.get("data")?.as_array()?;
    let slimmed: Vec<Value> = arr
        .iter()
        .map(|it| match it.as_object() {
            None => it.clone(),
            Some(obj) => {
                let mut m = serde_json::Map::new();
                for (k, v) in obj {
                    if SLIM_DROP_FIELDS.contains(&k.as_str()) {
                        continue;
                    }
                    match v.as_str() {
                        Some(s) if s.chars().count() > SLIM_CONTENT_CHARS => {
                            let kept: String = s.chars().take(SLIM_CONTENT_CHARS).collect();
                            m.insert(
                                k.clone(),
                                Value::String(format!(
                                    "{kept}…〔全文用 get_law_article(fgid+ftnum) 取〕"
                                )),
                            );
                        }
                        _ => {
                            m.insert(k.clone(), v.clone());
                        }
                    }
                }
                Value::Object(m)
            }
        })
        .collect();
    let mut out = serde_json::Map::new();
    out.insert("data".into(), Value::Array(slimmed));
    out.insert(
        "_note".into(),
        Value::String(
            "结果已精简(去重复正文/噪音,正文截断)。要某条全文用 get_law_article(fgid+ftnum) 取单条,\
             不要据此截断片段直接引用原文。"
                .into(),
        ),
    );
    Some(Value::Object(out))
}

// =============================================================================
// 案例检索瘦身(2026-06-02 · 真机:case_vector_search 单次 138KB / 45 案 × 656 字正文塞进主上下文;
// get_case_detail 的 content 还带个 llm_content 重复正文)。案例响应结构跟法条不同:
//   - case_vector_search 的案在 `extra.wenshu`;ptal/qwal/case_details 在 `data.lst`。
// 列表/向量检索:每案只留定位+短摘要(LLM 据此挑案,要全文 get_case_detail 取);
// get_case_detail:留 content(本就为取全文而调),只砍重复 llm_content + 噪音。
// 缓存(sidecar / .md)仍存完整,瘦身只作用于**返回给 LLM 的内容**。
// =============================================================================

/// 列表/向量检索类(每案正文截断)。`rh_case_details` 不在此列 —— 它要留全文。
const CASE_LIST_TYPES: &[&str] = &["rh_ptal_search", "rh_qwal_search", "case_vector_search"];
/// 每案只保留这些字段(定位 + 摘要锚);其余(llm_content 重复 / url / 区划 / score / 库 等)全砍。
const CASE_KEEP_FIELDS: &[&str] = &[
    "ah",      // 案号(取全文 get_case_detail 的 case_no)
    "jbdw",    // 经办单位/法院
    "ay",      // 案由
    "anyou",   // 案由(向量库字段名)
    "cprq",    // 裁判日期
    "cj",      // 审级
    "spcx",    // 审判程序
    "title",   // 标题
    "id",      // 定位 id
    "scid",    // 向量库定位 id
    "content", // 正文(列表型截断,详情型留全)
];
/// 列表型每案正文上限(字符)——够 LLM 判断是不是要的案,全文走 get_case_detail。
const CASE_CONTENT_CHARS: usize = 160;

/// 定位案例数组(不同 query_type 路径不同)。返回 `(案例数组, 是否列表型需截断)`。
fn locate_cases<'a>(query_type: &str, resp: &'a Value) -> Option<(&'a Vec<Value>, bool)> {
    let arr = match query_type {
        "case_vector_search" => resp.get("extra")?.get("wenshu")?.as_array()?,
        "rh_ptal_search" | "rh_qwal_search" | "rh_case_details" => {
            resp.get("data")?.get("lst")?.as_array()?
        }
        _ => return None,
    };
    Some((arr, CASE_LIST_TYPES.contains(&query_type)))
}

/// 把案例类元典响应瘦身成「喂 LLM 的紧凑版」。非案例类返回 `None`(调用方走原样)。
fn slim_cases_for_llm(query_type: &str, resp: &Value) -> Option<Value> {
    let (cases, truncate) = locate_cases(query_type, resp)?;
    let slimmed: Vec<Value> = cases
        .iter()
        .map(|c| {
            let Some(obj) = c.as_object() else {
                return c.clone();
            };
            let mut m = serde_json::Map::new();
            for &k in CASE_KEEP_FIELDS {
                let Some(v) = obj.get(k) else { continue };
                if truncate && k == "content" {
                    if let Some(s) = v.as_str() {
                        if s.chars().count() > CASE_CONTENT_CHARS {
                            let kept: String = s.chars().take(CASE_CONTENT_CHARS).collect();
                            m.insert(
                                k.to_string(),
                                Value::String(format!(
                                    "{kept}…〔全文用 get_case_detail(type+case_no) 取〕"
                                )),
                            );
                            continue;
                        }
                    }
                }
                m.insert(k.to_string(), v.clone());
            }
            Value::Object(m)
        })
        .collect();
    let note = if truncate {
        "案例结果已精简(每条只留 案号/法院/案由/日期/正文摘要,正文截断)。要某案全文用 \
         get_case_detail(type+case_no) 取,不要据此截断片段直接引用裁判理由。"
    } else {
        "已去重复正文(llm_content)与噪音字段;content 为判决正文。"
    };
    let mut out = serde_json::Map::new();
    out.insert("cases".into(), Value::Array(slimmed));
    out.insert("_note".into(), Value::String(note.into()));
    Some(Value::Object(out))
}

/// 决定一次工具结果**喂给 LLM** 的字符串:法条搜索类 / 案例类瘦身,其余原样 pretty JSON。
/// 缓存里仍存完整 resp(本函数不影响落盘)。
fn content_for_llm(query_type: &str, resp: &Value) -> String {
    let v = slim_search_for_llm(query_type, resp).or_else(|| slim_cases_for_llm(query_type, resp));
    let to_print = v.as_ref().unwrap_or(resp);
    serde_json::to_string_pretty(to_print).unwrap_or_else(|_| "{}".into())
}

// =============================================================================
// P1 · 缓存分层:详情类(全文)写「可读命名全文 MD」、空结果不缓存。
// 目的:让缓存目录里的全文成品一眼可读、可治理可提升,根治「详情 .md 显示 result_count:0、
// 全文藏 .raw.json,review 时像空垃圾被误删」的结构陷阱;并堵掉空结果污染。
// 注:这是**目录卫生**,不省积分(API 已经调过)。缓存命中仍走 hash 命名的 .raw.json sidecar。
// =============================================================================

/// 详情类(全文)query_type —— 走 `save_detail`(可读命名全文 MD + 索引)而非剥 content 的 SEARCH 索引。
const DETAIL_TYPES: &[&str] = &["rh_fg_detail", "rh_ft_detail", "rh_case_details"];

/// `render_detail_md` 的产物:写可读全文 MD 所需素材。
struct DetailDoc {
    type_label: &'static str,
    obj_id: String,
    display_name: String,
    body_md: String,
}

/// 把详情类响应渲染成可读全文 MD 的素材。**按响应形状分支**(关键:三类形状不同):
/// - `rh_fg_detail` / `rh_ft_detail`:`data` 是对象,正文在 `data.content`;
/// - `rh_case_details`:是 `get_case_detail` 用 search top_k=1 顶替的实现,正文在 `data.lst[0].content`。
///
/// 取不到正文 → `None`(调用方据此退回只写 sidecar,绝不写空壳 MD)。
fn render_detail_md(query_type: &str, resp: &Value) -> Option<DetailDoc> {
    let str_at = |p: &str| resp.pointer(p).and_then(|v| v.as_str());
    match query_type {
        "rh_fg_detail" => {
            let content = str_at("/data/content")?;
            if content.trim().is_empty() {
                return None;
            }
            let name = str_at("/data/fgmc").unwrap_or("未命名法规");
            let id = str_at("/data/fgid")
                .or_else(|| str_at("/data/id"))
                .unwrap_or("na");
            Some(DetailDoc {
                type_label: "法规",
                obj_id: id.to_string(),
                display_name: name.to_string(),
                body_md: content.to_string(),
            })
        }
        "rh_ft_detail" => {
            let content = str_at("/data/content")?;
            if content.trim().is_empty() {
                return None;
            }
            let fgmc = str_at("/data/fgmc").unwrap_or("");
            let ftnum = str_at("/data/ftnum")
                .or_else(|| str_at("/data/ft_num"))
                .unwrap_or("");
            let id = str_at("/data/id").unwrap_or("na");
            let name = if ftnum.is_empty() {
                fgmc.to_string()
            } else {
                format!("{} 第{}条", fgmc, ftnum)
            };
            Some(DetailDoc {
                type_label: "法条",
                obj_id: id.to_string(),
                display_name: name,
                body_md: content.to_string(),
            })
        }
        "rh_case_details" => {
            let first = resp
                .pointer("/data/lst")
                .and_then(|v| v.as_array())?
                .first()?;
            let content = first.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if content.trim().is_empty() {
                return None;
            }
            let ah = first
                .get("ah")
                .and_then(|v| v.as_str())
                .unwrap_or("未知案号");
            Some(DetailDoc {
                type_label: "案例",
                obj_id: ah.to_string(),
                display_name: ah.to_string(),
                body_md: content.to_string(),
            })
        }
        _ => None,
    }
}

/// 详情类持久化:可读全文 MD(+索引)+ 完整响应 sidecar。失败只 dlog,不致命。
/// 渲染不出正文(理论上 `response_is_empty` 已拦)→ 退回只写 sidecar,不丢数据。
pub(crate) fn persist_detail(
    kb: &LocalKb,
    query_type: &str,
    params: &Value,
    resp: &Value,
    body: &str,
) {
    if let Some(doc) = render_detail_md(query_type, resp) {
        if let Err(e) = kb.save_detail(
            query_type,
            params,
            doc.type_label,
            &doc.obj_id,
            &doc.display_name,
            &doc.body_md,
        ) {
            crate::dlog!("local_kb save_detail failed: {}", e);
        }
    } else {
        crate::dlog!("详情渲染不出正文,退回只写 sidecar: {}", query_type);
    }
    if let Err(e) = kb.save_raw_response(query_type, params, body) {
        crate::dlog!("local_kb save_raw_response failed: {}", e);
    }
}

/// 判断响应是否「空结果」—— 空就不缓存(目录卫生 + 不留 `kb_hit:true`/content 空 的迷惑信号;
/// 注:不省积分,API 已经调过)。**只对有把握判空的形状下结论**,其余一律当非空照存(保守,防误丢)。
pub(crate) fn response_is_empty(query_type: &str, resp: &Value) -> bool {
    match query_type {
        // 法规详情·对象型:正文在 data.content(已被 try_fulltext_article 佐证)。
        "rh_fg_detail" => resp
            .pointer("/data/content")
            .and_then(|v| v.as_str())
            .map(|s| s.trim().is_empty())
            .unwrap_or(true),
        // 法条单条详情:形状未经现行代码消费佐证(仿 fg_detail 的 data.content)。稳妥:
        // data 整个缺失 / null / 空对象 → 当空;有 data 但 content 字段缺失 → **不当空、照缓存**
        // (避免假设错时静默不缓存、反而每次重查费积分);只有 content 明确为空白才算空。
        "rh_ft_detail" => match resp.pointer("/data") {
            None | Some(Value::Null) => true,
            Some(Value::Object(o)) if o.is_empty() => true,
            Some(_) => resp
                .pointer("/data/content")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().is_empty())
                .unwrap_or(false),
        },
        // 详情·案例(search 顶替):看 data.lst 是否有案
        "rh_case_details" => resp
            .pointer("/data/lst")
            .and_then(|v| v.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(true),
        // 顶层 data 数组型搜索(ft/fg/ptal/qwal):空数组 = 空结果
        "rh_ft_search" | "rh_fg_search" | "rh_ptal_search" | "rh_qwal_search" => resp
            .get("data")
            .and_then(|v| v.as_array())
            .map(|a| a.is_empty())
            .unwrap_or(false),
        // 语义检索(嵌套形状不一)/ 企业类等 → 保守当非空照存
        _ => false,
    }
}

/// 三段式辅助 step 1:查 KB cache。命中 → 返回 `Some(ToolResult)` 让调用方提前 return,
/// miss → 返回 `None`,调用方接着调 API。
pub(crate) fn try_kb_hit(
    ctx: &ToolContext<'_>,
    query_type: &str,
    cache_params: &Value,
) -> Option<ToolResult> {
    let kb = ctx.local_kb?;
    let (_hit, _fresh) = kb.check_cache(query_type, cache_params)?;
    // 只认 sidecar 完整响应(含 content 全文,字节与未命中路径一致)。缺 sidecar
    //(老缓存 / 写失败)时**不**回退残缺 .md 索引 —— 残缺结果(丢 content)会让 LLM
    // 误以为没查全,用相同参数重复调用(实测 get_law_article 被 LoopGuard 拦下丢答案)。
    // 宁可当 miss 让上层重新调 API:多花一次积分,但拿完整响应 + 重建 sidecar,打断重复调循环。
    let raw = kb.load_raw_response(query_type, cache_params)?;
    // 缓存里存的是完整响应;喂 LLM 前对搜索类瘦身(与未命中路径 save_and_wrap 同一函数 →
    // 命中/未命中喂给 LLM 的字节一致,前缀缓存照样命中)。解析失败则原样返回(兜底)。
    let content = serde_json::from_str::<Value>(&raw)
        .map(|v| content_for_llm(query_type, &v))
        .unwrap_or(raw);
    Some(ToolResult {
        content,
        yuandian_credits_used: 0,
        kb_hit: true,
    })
}

/// 三段式辅助 step 3:元典 API 返回后,把 resp 序列化成 JSON 字符串给 LLM,
/// 顺手写回 KB(失败不致命,KB 写挂不影响本次调用)。
/// `credits` 不再由调用方传 —— 按 `query_type` 查官方计费表(`credits_for_query_type`),
/// 跟月账(`estimate_credits_for`)同源,杜绝各工具硬编码导致的逐条 trace / 反馈 MD 低估。
pub(crate) fn save_and_wrap(
    ctx: &ToolContext<'_>,
    query_type: &str,
    cache_params: &Value,
    summary: &str,
    resp: Value,
) -> ToolResult {
    let credits = crate::db::credits::credits_for_query_type(query_type);
    // sidecar 存「完整响应 pretty JSON」(含 content 全文,供命中复用 / 人读 / 可追溯)。
    let body = serde_json::to_string_pretty(&resp).unwrap_or_else(|_| "{}".into());
    if let Some(kb) = ctx.local_kb {
        if response_is_empty(query_type, &resp) {
            // P1 · 空结果不缓存(目录卫生:不囤空壳、不留 kb_hit:true/content 空 的迷惑信号)。
            // 不省积分 —— API 已经调过;省的是后续误导与垃圾堆积。
            crate::dlog!("空结果不写缓存: {}", query_type);
        } else if DETAIL_TYPES.contains(&query_type) {
            // P1 · 详情类(全文):写可读命名全文 MD + 索引 + sidecar(替代剥 content 的 SEARCH 空壳)。
            persist_detail(kb, query_type, cache_params, &resp, &body);
        } else {
            // 搜索类:.md 索引(给 Python skill / 人读)+ sidecar(写失败只 dlog,不应让 LLM 看不到结果)。
            let empty: Vec<Value> = Vec::new();
            let results: Vec<Value> = resp
                .get("data")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or(empty);
            if let Err(e) = kb.save_search(query_type, cache_params, &results, summary) {
                crate::dlog!("local_kb save_search failed: {}", e);
            }
            if let Err(e) = kb.save_raw_response(query_type, cache_params, &body) {
                crate::dlog!("local_kb save_raw_response failed: {}", e);
            }
        }
    }
    ToolResult {
        // 喂 LLM 的内容:搜索类瘦身(只给定位+短摘要),其余原样。缓存仍是上面的完整 body。
        content: content_for_llm(query_type, &resp),
        yuandian_credits_used: credits,
        kb_hit: false,
    }
}
