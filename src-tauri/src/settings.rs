//! 用户设置的读写。
//!
//! 设计原则(对应 CLAUDE.md 隐私铁律):
//!   - **每个用户填自己的 token**,工具不内置任何人的 key
//!   - 配置落本机 `~/Library/Application Support/CaseBoard/settings.json`
//!   - V0.1 明文存(本机用户文件保护即可);V0.2 升 macOS Keychain
//!   - 飞书反馈 webhook 不在这里(它是编译时常量,所有用户共用,
//!     接收方是作者;放在 task #8 单独处理)
//!
//! 文件结构(扁平,V0.1 简单优先):
//! ```json
//! {
//!   "mineru_api_key": "",
//!   "mineru_endpoint": "https://api.mineru.net/v1",
//!   "ollama_endpoint": "http://localhost:11434",
//!   "ollama_model": "qwen2.5:7b"
//! }
//! ```

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::chat::mcp_bridge::McpServerConfig;
use crate::db::app_data_dir;

/// 用户配置。字段全部 Option<String>,因为初始全是空的。
///
/// 这里**只放每个用户私有的配置**——不放飞书 webhook 这种"全局共享"的常量。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 用户的显示称呼(例:"刘律师" / "周律师"),首页问候用。
    /// 留空时显示"律师"作为兜底。2026-05-23 晚九加。
    pub user_display_name: Option<String>,

    /// 首页天气手动城市。None/空白 = 自动使用系统定位；定位失败时仅展示网络粗定位。
    pub weather_city: Option<String>,

    // ===== 2026-05-23 加(作者隐私分流决策,详见 docs/产品决策与理念.md 第 2 节) =====
    /// 用户是否完成过 onboarding。
    ///
    /// **默认 false** —— 首次启动 App 检测到为 false 时,强制弹 OnboardingWizard 让用户做选择。
    /// 完成后置 true,后续启动跳过 onboarding。
    pub setup_completed: bool,

    // ===== 2026-05-23 晚六 二维独立分流(替代原 cloud_enabled) =====
    /// OCR 后端选择:`"local"` = 本机 MiniCPM-V vision / `"cloud"` = MinerU 在线
    pub ocr_provider: Option<String>,
    /// LLM 后端选择:`"local"` = 本机 MiniCPM-V chat / `"cloud"` = DeepSeek
    pub llm_provider: Option<String>,

    /// AI 助手 Agent Runtime。`None` / `"native"` = CaseBoard 原生循环；`"pi"` = Pi Sidecar。
    /// 缺省必须保持原生，确保老用户升级后行为不变；未知值同样安全回退到原生。
    pub agent_runtime: Option<String>,
    /// Pi Runtime 独立 provider/model 选择。只保存非敏感 ID；凭据进入系统凭据库。
    pub pi_provider_id: Option<String>,
    pub pi_model_id: Option<String>,
    /// 按 Pi provider/model 记忆用户选择的原生推理强度；目录与合法档位仍以 Pi SDK 为准。
    pub pi_model_thinking_levels:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,

    /// 本机模型目录(放 MiniCPM-V-4_6-Q8_0.gguf 和 mmproj-model-f16.gguf)
    ///
    /// 默认会建议 `~/.cache/caseboard/models/`,用户也可以指定其他目录(比如已经下载在
    /// `~/.lmstudio/models/openbmb/MiniCPM-V-4.6-gguf/`)
    pub local_model_dir: Option<String>,
    /// 是否允许 App 自动拉起 llama-server(默认 true,用户不用动终端)
    pub local_server_auto_start: Option<bool>,

    // ===== 旧字段:保留作向后兼容,迁移到新字段后还会用一段时间 =====
    /// [DEPRECATED] 老的"全局云端开关",2026-05-23 晚六改成 ocr_provider/llm_provider 独立
    /// 仍保留是为了不破坏老配置 — read 时如果新字段空就 fall back 到这个
    pub cloud_enabled: bool,

    /// MinerU 在线 OCR 的 API key(用户自己注册账号拿)
    pub mineru_api_key: Option<String>,
    /// MinerU endpoint(一般不用改,默认值)
    pub mineru_endpoint: Option<String>,

    /// 2026-06-12:PaddleOCR VL-1.6(百度 AI Studio 星河社区)访问令牌。
    /// 申请:https://aistudio.baidu.com/account/accessToken,免费 20,000 页/天。
    /// 作者实测与 MinerU 精度打平、速度约快一倍;详 ingest/paddle_vl_http.rs 头注释。
    pub paddle_vl_api_key: Option<String>,
    /// PaddleOCR key 验证通过时间(坑#11:新 cloud key 必配 verified_at,改 key 重置)
    pub paddle_vl_verified_at: Option<String>,
    /// 云端 OCR 主力选择:`"mineru"`(默认,老用户零感知)/ `"paddle-vl"`。
    /// 另一个自动成为备用:主力失败 / 超时 / 额度用完时,**备用 key 已填**才自动切换。
    pub ocr_cloud_primary: Option<String>,

    /// 本机 llama-server endpoint(默认 http://127.0.0.1:8899)
    /// 字段名是历史包袱 "ollama_*",实际用的是 llama.cpp 的 llama-server
    pub ollama_endpoint: Option<String>,
    /// 本机 LLM 模型名(默认 MiniCPM-V-4_6-Q8_0.gguf)
    pub ollama_model: Option<String>,

    /// 云端 LLM endpoint(默认推荐 DeepSeek `https://api.deepseek.com`)
    pub cloud_llm_endpoint: Option<String>,
    /// 云端 LLM 模型档位(V0.3 统一为唯一的模型选择,被 `model_router::route_model` 读取):
    ///   - `'deepseek-v4-flash'`(默认)= 全局 Flash(便宜,约 pro 的 1/3 价)
    ///   - `'deepseek-v4-pro'` = 全局 Pro(更准更贵;实测 v4-pro 本身即思考模型,无独立 -thinking 变体)
    ///   - `'auto'` = 自动挡(简单走 flash,复杂走 pro)
    ///
    /// 默认 flash;不再有"工具型任务偷偷强制 pro"的隐藏逻辑。
    pub cloud_llm_model: Option<String>,
    /// 云端 LLM API key
    pub cloud_llm_api_key: Option<String>,

    /// 2026-06-15:云端 LLM 后端选择 —— `"deepseek"`(默认/缺省)/ `"minimax"`。
    /// **纯增量**:老用户(全是 DeepSeek)缺此字段 → 走 deepseek 分支,配置零改动、零重解释。
    /// 选 minimax 时改读下面一组 `minimax_*` 字段,DeepSeek 的 key/endpoint/档位完全不动。
    /// 设计见 docs/MiniMax模型接入-2026-06-15.md。
    pub cloud_llm_backend: Option<String>,
    /// MiniMax API key(独立于 DeepSeek key,切后端互不覆盖)。
    pub minimax_api_key: Option<String>,
    /// MiniMax endpoint base(默认 `https://api.minimaxi.com`;聊天真实路径
    /// `/v1/text/chatcompletion_v2` 由 LlmConfig 自动补,**不是** OpenAI 兼容的 /v1/chat/completions)。
    pub minimax_endpoint: Option<String>,
    /// MiniMax 模型名(**可编辑**自由文本,默认 `MiniMax-M2`)。MiniMax 官方型号名以控制台为准,
    /// 写错会 404 —— 故做成可填而非写死下拉,「以后适配更多模型」零改代码。
    pub minimax_model: Option<String>,
    /// MiniMax key 验证通过时间(坑#11:新 cloud key 必配 verified_at,改 key 重置)。
    pub minimax_verified_at: Option<String>,

    /// 2026-06-16:通用 OpenAI 兼容云端 LLM 后端(智谱 GLM / 小米 MiMo / 自定义)。
    /// `cloud_llm_backend` 取 `"glm"` / `"mimo"` / `"custom"` 时读对应服务商的独立配置。
    /// **纯增量调和**:DeepSeek(`cloud_llm_*`+档位)/ MiniMax(`minimax_*`+v2 协议)两条老路完全不动;
    /// 这条走标准 `/v1/chat/completions`,模型名是用户**显式填的具体型号**(不套 DeepSeek 的 flash/pro 档位,
    /// 同 MiniMax 处理)。glm/mimo/custom 的 key/endpoint/model 分开保存,切换服务商不会互相覆盖。
    /// 预设默认值见 `llm::providers`。
    ///
    /// 旧版 `compat_llm_*` 作为兼容字段保留:读当前后端时,如果新字段为空,会 fallback 到旧字段,
    /// 这样用户已经填过的配置不会因升级丢失。
    pub compat_llm_endpoint: Option<String>,
    /// 通用兼容后端模型名(具体型号,如 `glm-4.6`;自由文本,以服务商控制台为准)。
    pub compat_llm_model: Option<String>,
    /// 通用兼容后端 API key(独立于 DeepSeek / MiniMax)。
    pub compat_llm_api_key: Option<String>,
    /// 通用兼容后端 key 验证通过时间(坑#11)。
    pub compat_llm_verified_at: Option<String>,

    /// 智谱 GLM 独立配置(OpenAI-compatible chat completions)。
    pub glm_llm_endpoint: Option<String>,
    pub glm_llm_model: Option<String>,
    pub glm_llm_api_key: Option<String>,
    pub glm_llm_verified_at: Option<String>,

    /// 小米 MiMo 独立配置(OpenAI-compatible chat completions)。
    pub mimo_llm_endpoint: Option<String>,
    pub mimo_llm_model: Option<String>,
    pub mimo_llm_api_key: Option<String>,
    pub mimo_llm_verified_at: Option<String>,

    /// Kimi Coding Plan 独立配置，避免切换服务商时覆盖 DeepSeek 或其他兼容模型。
    pub kimi_llm_endpoint: Option<String>,
    pub kimi_llm_model: Option<String>,
    pub kimi_llm_api_key: Option<String>,
    pub kimi_llm_verified_at: Option<String>,

    /// 自定义 OpenAI 兼容模型独立配置。
    pub custom_llm_endpoint: Option<String>,
    pub custom_llm_model: Option<String>,
    pub custom_llm_api_key: Option<String>,
    pub custom_llm_verified_at: Option<String>,

    /// 2026-05-24 k:元典法律开放平台 API key — 执行案件查被执行人 / 失信 / 财产线索 用
    /// 申请:https://open.chineselaw.com/
    pub yuandian_api_key: Option<String>,

    /// 2026-06-01 V0.3:快递100 实时查询 customer 编号 + 授权 key(快递查询工具用)。
    /// 申请:https://api.kuaidi100.com/(个人免费版约 50 次/天,无需企业资质)。
    /// 签名 = 大写 MD5(param + key + customer)。两者都填了才启用快递查询。
    pub kuaidi100_customer: Option<String>,
    pub kuaidi100_key: Option<String>,

    /// 2026-06-01 V0.3.3:Embedding 云端模型(案件文档语义检索)。OpenAI 兼容 /embeddings。
    /// 默认硅基流动 BAAI/bge-m3；填了 api_key 才启用语义检索，否则回退关键词选材料。
    /// 申请:https://cloud.siliconflow.cn/me/account/ak
    pub embedding_endpoint: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_api_key: Option<String>,
    /// embedding key 验证通过时间(坑#11:新 cloud key 必配 verified_at,改 key 重置)
    pub embedding_verified_at: Option<String>,

    /// 本地知识库语义向量索引「自动维护」开关(出报告 / 启动后台增量索引)。
    /// `None`/`Some(true)` = 开(默认);`Some(false)` = 关(只手动重建)。
    pub kb_semantic_auto_index: Option<bool>,

    /// 允许 CaseBoard 内置 AI（Native / Pi 共用 Rust 工具）新增经过约束的 L1 raw 材料。
    /// 默认 false；仅允许 create-only 写入 raw/notes，不开放覆盖、删除、移动或 Wiki 提升。
    pub ai_kb_maintenance_enabled: bool,

    /// 2026-05-24 e:匿名反馈识别码(UUID v4),首次启动时自动生成 + 持久化。
    /// 跟用户名/邮箱无关 — 作者拿到反馈 MD 后可以识别"这个 ID 之前反馈过"。
    /// 用户能在设置里清空重生成(类比换匿名 ID)。
    pub client_id: Option<String>,

    /// 2026-05-25 V0.1.6:MinerU key 通过验证的时间(ISO 8601)。
    /// 非 null = 用户点过「验证」按钮且通过,UI 显示绿勾。
    /// 用户改 key 会被清空(前端逻辑控制)。
    pub mineru_verified_at: Option<String>,
    /// DeepSeek key 通过验证的时间(同上)。
    pub deepseek_verified_at: Option<String>,
    /// 2026-05-25 V0.1.8:元典 key 通过验证的时间(同上)。
    pub yuandian_verified_at: Option<String>,

    /// 2026-05-26 V0.1.13:首页"在办案件"卡片用户拖动排序。
    /// 数组里的 case_id 按用户拖动后的顺序排;**没在数组里的案件**
    /// 按 listCases 默认顺序追加在末尾(新建案件不会被忘记)。
    /// 删过的案件 id 留在数组里也无害(前端 filter 掉)。
    pub home_case_order: Option<Vec<String>>,

    /// 2026-06-14:首页"日程日历"功能开关(默认关闭)。
    /// 该功能与待办清单略重复且卡片较大,做成可选 —— 用户在设置里手动打开体验,
    /// 不好用可关掉,不影响其他功能。`#[serde(default)]` → 老 settings.json 缺此字段时为 false。
    pub home_calendar_enabled: bool,

    // ===== 2026-06-17 飞书日历(整合外部贡献 PR #9,gcheng-001;精简为只读日历)=====
    /// 飞书日历总开关。默认关闭;启用后复用本机 lark-cli 的登录态,不在 CaseBoard 保存飞书 token。
    /// 配好并打开后,首页显示飞书日历月历视图(替代本地"日程日历"卡片)。
    pub feishu_enabled: Option<bool>,
    /// lark-cli 可执行文件路径。`None`/空 = 按平台自动找(macOS 走 Homebrew,Windows/Linux 靠 PATH)。
    /// Windows 用户可在此填 `lark-cli.exe` 全路径(没加进 PATH 时)。
    pub feishu_lark_cli_path: Option<String>,
    /// (可选)飞书"案件池"多维表格 App Token。配了才能"点日历事件→反查并导入本地案件目录"。
    pub feishu_app_token: Option<String>,
    /// (可选)飞书"案件池"多维表格 Table ID(配合 app_token)。
    pub feishu_cases_table_id: Option<String>,
    /// 飞书机器人每日手机提醒开关。独立于日历读取；默认关闭，不影响未配置用户。
    pub feishu_reminder_enabled: Option<bool>,
    /// 飞书群自定义机器人 Webhook URL，只存本机 settings.json。
    pub feishu_webhook_url: Option<String>,
    /// 每日推送时间，格式 HH:MM。
    pub feishu_reminder_time: Option<String>,
    /// 提前汇总未来多少天的到期事项。
    pub feishu_reminder_days: Option<u32>,

    // ===== 2026-06-17 辅助在线立案(整合外部贡献 PR #8,gcheng-001)=====
    /// 立案 CLI 包根目录。None = 用应用内置 standalone/court_filing_cli(打包进 resources)。
    pub court_filing_cli_path: Option<String>,
    /// Python 解释器路径。None = 用 "python3"(Windows 用户需填 "python" 或 venv 内全路径)。
    pub court_filing_python: Option<String>,
    /// 全国法院一张网账号(手机号)。只存本机,不进 git。
    pub court_filing_account: Option<String>,
    /// 全国法院一张网密码。只存本机,不进 git。
    pub court_filing_password: Option<String>,
    /// 一张网登录态 cookie 缓存目录。None = 用默认应用数据目录。
    pub court_filing_cookie_dir: Option<String>,

    // ===== V0.2 D2 新增 · 本地知识库 + chat V2 budget =====
    /// 2026-05-27 V0.2:本地法律知识库根目录(支持 `~/` tilde 展开)。
    /// `None` = 不启用本地 KB,所有元典查询都走在线 + DB 临时缓存;
    /// 指向一个存在的目录 = LocalKb::auto_detect 启用,元典缓存写入
    /// `<root>/raw/yuandian-cache/`,chat 工具优先查本地。
    pub local_kb_root: Option<String>,
    /// 本地 KB 总开关(为 false 时即使 local_kb_root 有值也不启用,给用户临时停用的能力)
    pub local_kb_enabled: Option<bool>,

    /// 元典积分月度上限(整数；官方接口目录价为 1 / 5 / 10 / 50 分每次)。
    /// `None` = 不限制。超出阈值时,chat 自动降级到 KB Stale 命中,不再发起在线调用。
    pub yuandian_monthly_credit_limit: Option<u32>,

    // V0.3:模型档位已统一到 `cloud_llm_model`(flash / pro / 'auto' 自动挡),
    // 原 `chat_default_model` 字段已废弃移除(旧 settings.json 里的该键会被 serde 忽略)。
    /// chat 总上下文 char 预算(默认 300_000,~200K token)
    pub chat_context_budget_total: Option<u32>,
    /// chat system prompt + 案件快照 + 工具 schema 段 char 预算(默认 150_000)
    pub chat_context_budget_system: Option<u32>,
    /// chat 引用文档全文段 char 预算(默认 120_000)
    pub chat_context_budget_attached: Option<u32>,
    /// chat 历史对话段 char 预算(默认 40_000,超出走 compaction)
    pub chat_context_budget_history: Option<u32>,
    /// chat agent loop 最大迭代轮数(默认 16;见 with_defaults_for_display)
    pub chat_loop_max_iters: Option<u32>,
    /// chat 单条消息最多引用文档数(默认 5)
    pub chat_max_attached: Option<u32>,
    /// AI Soul:全局 AI 助手工作风格 / 长期偏好。只作为低优先级风格约束,不能覆盖系统宪法。
    pub ai_soul_md: Option<String>,
    /// V0.3.6 · 外部 MCP server 白名单(CaseBoard 当客户端消费其工具)。默认空 = 桥接关闭、零行为变化。
    /// 每项 `{name, transport:{type:"stdio",command,args,env}|{type:"http",url}, enabled}`,详 ADR-0008。
    pub mcp_servers: Vec<McpServerConfig>,

    /// 2026-06-10 团队版 Phase 1(LAN 接力同步,详 docs/提案-团队版-2026-06-10.md §6)。
    /// None = 未加入团队,团队功能整体关闭零开销。secret/配对码跟 API key 同级:只存本机不进 git。
    pub team: Option<crate::team::TeamIdentity>,

    /// 同一用户的 Mac / Windows 设备间同步个人工作空间。
    /// 业务状态、派生 Markdown 和可移植设置进入加密协议；副设备源文件只单向归集主力设备，
    /// 本机绝对路径永不进入协议。
    /// 默认关闭；身份和高熵密钥只由 device_sync_* commands 改，普通设置保存不得覆写。
    pub device_sync_enabled: bool,
    pub device_sync: Option<crate::device_sync::DeviceSyncIdentity>,
}

impl Settings {
    /// 获取**真实生效**的 OCR provider。
    ///
    /// **V0.3(2026-05-31)暂时隐藏本地模型 → 强制云端(MinerU)。** 本地分支代码
    /// (`ingest/ocr.rs` 的 vision 路径)保留休眠;无论存的是 `cloud` / `local`(含老配置
    /// 残留)/ None,一律返回 `"cloud"`,顺带消化老用户 `ocr_provider="local"` 残留。
    /// **恢复本地**:把本函数改回读 `self.ocr_provider`(原逻辑见 git),同时恢复
    /// `effective_llm_provider` + `needs_local_server` 下游(pipeline 自动起 llama-server /
    /// feedback 诊断 / detect_local_readiness 引导)+ 前端 UI 入口即可。
    pub fn effective_ocr_provider(&self) -> &str {
        "cloud"
    }

    /// 云端 LLM 后端(2026-06-15;2026-06-16 加 OpenAI 兼容三档)。
    /// 取值:`"deepseek"`(默认)/ `"minimax"` / `"glm"` / `"mimo"` / `"custom"`。
    /// 缺省 / 空 / 非法值一律回落 `"deepseek"`(老用户零感知)。
    pub fn effective_cloud_llm_backend(&self) -> &str {
        match self.cloud_llm_backend.as_deref().map(str::trim) {
            Some("minimax") => "minimax",
            Some("glm") => "glm",
            Some("mimo") => "mimo",
            Some("kimi") => "kimi",
            Some("custom") => "custom",
            _ => "deepseek",
        }
    }

    /// 是否走「通用 OpenAI 兼容」后端(glm / mimo / custom 共用 `compat_llm_*` + 标准 chat 协议)。
    pub fn cloud_llm_is_compat(&self) -> bool {
        matches!(
            self.effective_cloud_llm_backend(),
            "glm" | "mimo" | "kimi" | "custom"
        )
    }

    fn clean_string(value: &Option<String>) -> Option<String> {
        value
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    /// 当前兼容后端的 endpoint。新字段优先,旧版 compat_llm_* 兜底。
    pub fn effective_compat_llm_endpoint(&self) -> Option<String> {
        let current = match self.effective_cloud_llm_backend() {
            "glm" => Self::clean_string(&self.glm_llm_endpoint),
            "mimo" => Self::clean_string(&self.mimo_llm_endpoint),
            "kimi" => Self::clean_string(&self.kimi_llm_endpoint),
            "custom" => Self::clean_string(&self.custom_llm_endpoint),
            _ => None,
        };
        current.or_else(|| Self::clean_string(&self.compat_llm_endpoint))
    }

    /// 当前兼容后端的模型名。新字段优先,旧版 compat_llm_* 兜底。
    pub fn effective_compat_llm_model(&self) -> Option<String> {
        let current = match self.effective_cloud_llm_backend() {
            "glm" => Self::clean_string(&self.glm_llm_model),
            "mimo" => Self::clean_string(&self.mimo_llm_model),
            "kimi" => Self::clean_string(&self.kimi_llm_model),
            "custom" => Self::clean_string(&self.custom_llm_model),
            _ => None,
        };
        current.or_else(|| Self::clean_string(&self.compat_llm_model))
    }

    /// 当前兼容后端的 API key。新字段优先,旧版 compat_llm_* 兜底。
    pub fn effective_compat_llm_api_key(&self) -> Option<String> {
        let current = match self.effective_cloud_llm_backend() {
            "glm" => Self::clean_string(&self.glm_llm_api_key),
            "mimo" => Self::clean_string(&self.mimo_llm_api_key),
            "kimi" => Self::clean_string(&self.kimi_llm_api_key),
            "custom" => Self::clean_string(&self.custom_llm_api_key),
            _ => None,
        };
        current.or_else(|| Self::clean_string(&self.compat_llm_api_key))
    }

    /// 当前兼容后端的验证时间。与 endpoint/model/key 一样，专属字段优先、旧共享字段兜底。
    pub fn effective_compat_llm_verified_at(&self) -> Option<String> {
        let current = match self.effective_cloud_llm_backend() {
            "glm" => Self::clean_string(&self.glm_llm_verified_at),
            "mimo" => Self::clean_string(&self.mimo_llm_verified_at),
            "kimi" => Self::clean_string(&self.kimi_llm_verified_at),
            "custom" => Self::clean_string(&self.custom_llm_verified_at),
            _ => None,
        };
        current.or_else(|| Self::clean_string(&self.compat_llm_verified_at))
    }

    /// 案件材料处理（文档字段抽取、全案分析、报告生成）启动前的统一配置门禁。
    ///
    /// 不能只检查 key 非空：用户切换服务商或改过 key 后，必须重新验证成功，才能避免
    /// 后台批量任务启动后才逐份失败。错误文本直接给出 UI 中的准确入口。
    pub fn validate_material_llm_ready(&self) -> Result<(), String> {
        const SETTINGS_HINT: &str = "请前往「设置 → 大脑 → 材料处理模型」填写并验证后重试。";
        let backend = self.effective_cloud_llm_backend();
        let (label, api_key, verified_at) = if backend == "minimax" {
            (
                "MiniMax",
                Self::clean_string(&self.minimax_api_key),
                Self::clean_string(&self.minimax_verified_at),
            )
        } else if self.cloud_llm_is_compat() {
            let label = match backend {
                "glm" => "智谱 GLM",
                "mimo" => "小米 MiMo",
                "kimi" => "Kimi",
                "custom" => "自定义模型",
                _ => "云端模型",
            };
            if backend == "custom" {
                if self.effective_compat_llm_endpoint().is_none() {
                    return Err(format!(
                        "材料处理模型未配置完整：尚未填写自定义模型接口地址。{SETTINGS_HINT}"
                    ));
                }
                if self.effective_compat_llm_model().is_none() {
                    return Err(format!(
                        "材料处理模型未配置完整：尚未填写自定义模型名称。{SETTINGS_HINT}"
                    ));
                }
            }
            (
                label,
                self.effective_compat_llm_api_key(),
                self.effective_compat_llm_verified_at(),
            )
        } else {
            (
                "DeepSeek",
                Self::clean_string(&self.cloud_llm_api_key),
                Self::clean_string(&self.deepseek_verified_at),
            )
        };

        if api_key.is_none() {
            return Err(format!(
                "材料处理模型未配置完整：尚未填写 {label} API Key。{SETTINGS_HINT}"
            ));
        }
        if verified_at.is_none() {
            return Err(format!(
                "材料处理模型未配置完整：{label} API Key 尚未验证成功。{SETTINGS_HINT}"
            ));
        }
        Ok(())
    }

    /// 一次性迁移:把旧的「共享 `compat_llm_*`」搬进**当前兼容后端**的专属字段,然后清空旧字段。
    ///
    /// 背景:旧设计 glm/mimo/custom 共用一组 `compat_llm_*`(切兼容后端会清空旧 key),所以旧值
    /// 总归属「当前激活的那个兼容后端」。整合 PR#15 后改成各家独立字段 + 旧字段兜底,但「兜底」会让
    /// 旧值跨后端串味(切到没填 key 的 MiMo 时会回落到上一个后端的旧 key/endpoint,verified 也错挂)。
    /// 迁移一次把旧值归位到专属字段并清空旧字段,此后 `effective_*` 的兜底恒为 no-op,串味消失。
    ///
    /// 幂等:旧字段已空 / 当前非兼容后端 → 不动,返回 `false`。返回 `true` 表示有改动需回写。
    pub fn migrate_legacy_compat_inplace(&mut self) -> bool {
        let le = Self::clean_string(&self.compat_llm_endpoint);
        let lm = Self::clean_string(&self.compat_llm_model);
        let lk = Self::clean_string(&self.compat_llm_api_key);
        let lv = Self::clean_string(&self.compat_llm_verified_at);
        if le.is_none() && lm.is_none() && lk.is_none() && lv.is_none() {
            return false; // 旧字段已空,迁移过了 / 从没用过
        }
        // 只在当前是兼容后端时迁移(旧值归属当前兼容后端);非兼容后端时旧值归属未知,
        // 留着不动也无害(deepseek/minimax 路径不读 compat_llm_*,不会串味)。
        let backend = self.effective_cloud_llm_backend().to_string();
        let (ep, md, key, ver) = match backend.as_str() {
            "glm" => (
                &mut self.glm_llm_endpoint,
                &mut self.glm_llm_model,
                &mut self.glm_llm_api_key,
                &mut self.glm_llm_verified_at,
            ),
            "mimo" => (
                &mut self.mimo_llm_endpoint,
                &mut self.mimo_llm_model,
                &mut self.mimo_llm_api_key,
                &mut self.mimo_llm_verified_at,
            ),
            "kimi" => (
                &mut self.kimi_llm_endpoint,
                &mut self.kimi_llm_model,
                &mut self.kimi_llm_api_key,
                &mut self.kimi_llm_verified_at,
            ),
            "custom" => (
                &mut self.custom_llm_endpoint,
                &mut self.custom_llm_model,
                &mut self.custom_llm_api_key,
                &mut self.custom_llm_verified_at,
            ),
            _ => return false,
        };
        // 只填专属字段里为空的(已填的用户值优先,不覆盖)
        let fill = |dst: &mut Option<String>, src: Option<String>| {
            let dst_empty = dst
                .as_deref()
                .map(str::trim)
                .map(|x| x.is_empty())
                .unwrap_or(true);
            if dst_empty {
                if let Some(v) = src {
                    *dst = Some(v);
                }
            }
        };
        fill(ep, le);
        fill(md, lm);
        fill(key, lk);
        fill(ver, lv);
        // 清空旧共享字段:此后只认专属字段,杜绝跨后端串味
        self.compat_llm_endpoint = None;
        self.compat_llm_model = None;
        self.compat_llm_api_key = None;
        self.compat_llm_verified_at = None;
        true
    }

    /// 云端 OCR 主力(2026-06-12)。`"paddle-vl"` 仅当用户显式选择**且** key 已填才生效,
    /// 否则一律 `"mineru"`(老用户 / key 被清掉后零感知回到原行为)。
    pub fn effective_ocr_cloud_primary(&self) -> &str {
        let paddle_key_set = self
            .paddle_vl_api_key
            .as_deref()
            .is_some_and(|k| !k.trim().is_empty());
        match self.ocr_cloud_primary.as_deref() {
            Some("paddle-vl") if paddle_key_set => "paddle-vl",
            _ => "mineru",
        }
    }

    /// 获取**真实生效**的 LLM provider。**V0.3 暂时隐藏本地模型 → 强制云端(DeepSeek)。**
    /// 同 `effective_ocr_provider`:本地分支(`llm/mod.rs::from_settings` 的 else)保留休眠。
    pub fn effective_llm_provider(&self) -> &str {
        "cloud"
    }

    /// 任何一个 provider 用到了本机,就需要 llama-server。
    /// V0.3 隐藏本地后 `effective_*` 恒 cloud → 本函数恒 false(pipeline 不再自动起本机服务)。
    pub fn needs_local_server(&self) -> bool {
        self.effective_ocr_provider() == "local" || self.effective_llm_provider() == "local"
    }
}

impl Settings {
    /// 给前端返回时,用 sensible 默认值补全空字段(便于直接渲染表单)。
    /// 注意:**这里不返回任何 token 默认值**——key 一律保持用户输入。
    pub fn with_defaults_for_display(self) -> Self {
        // 只对「有内置默认值」的字段填默认,其余字段一律 `..self` 原样透传。
        // 用 `..self` 而非逐字段手列:以后给 Settings 加字段会自动继承原值,
        // 不会因为这里漏写一行而被静默丢成默认(B14 防漏映射)。
        Self {
            local_server_auto_start: self.local_server_auto_start.or(Some(true)),
            mineru_endpoint: self
                .mineru_endpoint
                .or_else(|| Some("https://mineru.net/api/v4".to_string())),
            ollama_endpoint: self
                .ollama_endpoint
                .or_else(|| Some("http://127.0.0.1:8899".to_string())),
            ollama_model: self
                .ollama_model
                .or_else(|| Some("MiniCPM-V-4_6-Q8_0.gguf".to_string())),
            cloud_llm_endpoint: self
                .cloud_llm_endpoint
                .or_else(|| Some("https://api.deepseek.com".to_string())),
            cloud_llm_model: self
                .cloud_llm_model
                .or_else(|| Some("deepseek-v4-flash".to_string())),
            embedding_endpoint: self
                .embedding_endpoint
                .or_else(|| Some(crate::embedding::DEFAULT_ENDPOINT.to_string())),
            embedding_model: self
                .embedding_model
                .or_else(|| Some(crate::embedding::DEFAULT_MODEL.to_string())),
            chat_context_budget_total: self.chat_context_budget_total.or(Some(300_000)),
            chat_context_budget_system: self.chat_context_budget_system.or(Some(150_000)),
            chat_context_budget_attached: self.chat_context_budget_attached.or(Some(120_000)),
            chat_context_budget_history: self.chat_context_budget_history.or(Some(40_000)),
            chat_loop_max_iters: self.chat_loop_max_iters.or(Some(16)),
            chat_max_attached: self.chat_max_attached.or(Some(5)),
            feishu_reminder_enabled: self.feishu_reminder_enabled.or(Some(false)),
            feishu_reminder_time: self
                .feishu_reminder_time
                .or_else(|| Some("09:00".to_string())),
            feishu_reminder_days: self.feishu_reminder_days.or(Some(7)),
            ..self
        }
    }
}

/// 拿到 settings.json 的路径(跟 caseboard.db 在同一个目录)。
pub fn settings_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()
        .map_err(|e| format!("找不到数据目录: {}", e))?
        .join("settings.json"))
}

/// 读取设置。文件不存在 / 解析失败,**不报错**,返回 `Settings::default()`。
/// 第一次启动时这是预期行为。
pub fn read_settings() -> Result<Settings, String> {
    let path = settings_path()?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("读 settings.json 失败: {}", e))?;
    if text.trim().is_empty() {
        return Ok(Settings::default());
    }
    let mut settings = serde_json::from_str::<Settings>(&text)
        .map_err(|e| format!("settings.json 格式错误: {}", e))?;
    // 一次性把旧共享 compat_llm_* 归位到当前兼容后端的专属字段(幂等;迁移过/没用过都是 no-op)。
    // 回写失败不致命:下次读再迁一次(in-memory 已是迁移后的值,本次调用方拿到的就是对的)。
    if settings.migrate_legacy_compat_inplace() {
        let _ = write_settings(&settings);
    }
    Ok(settings)
}

/// 写入设置(覆盖)。会自动创建父目录。
pub fn write_settings(settings: &Settings) -> Result<(), String> {
    let path = settings_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("建目录失败: {}", e))?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(&path, text).map_err(|e| format!("写 settings.json 失败: {}", e))?;
    Ok(())
}

/// 2026-05-24 e:确保 client_id 存在(给反馈通道用的匿名识别码)。
///
/// 如果 settings.client_id 为空,生成新 UUID v4 并持久化;返回最终的 client_id。
/// 跟用户名/邮箱无关,纯随机。作者拿到反馈 MD 可识别"是否同一个人多次反馈"。
pub fn ensure_client_id() -> Result<String, String> {
    let mut s = read_settings()?;
    if let Some(existing) = s.client_id.as_ref() {
        if !existing.trim().is_empty() {
            return Ok(existing.clone());
        }
    }
    let new_id = uuid::Uuid::new_v4().to_string();
    s.client_id = Some(new_id.clone());
    write_settings(&s)?;
    Ok(new_id)
}

// ============================================================================
// 测试
// ============================================================================
