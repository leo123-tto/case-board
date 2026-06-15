//! 本地 LLM 客户端,用于从案件文档抽取结构化字段。
//!
//! 默认对接 llama.cpp server(`MiniCPM-V-4.6` GGUF)的 OpenAI 兼容接口:
//!   `POST http://127.0.0.1:8899/v1/chat/completions`
//!
//! 设计:
//!   - **不内置任何 token**,endpoint / model 由用户在设置页配置
//!   - 后端是 OpenAI 兼容,所以 Ollama / LM Studio / llama.cpp / 云端都能用
//!   - V0.1 阶段只用纯文本接口(文档先用 textutil 抽文本,再喂给 LLM)
//!   - V0.2 用 vision 接口直接喂图片/PDF(MiniCPM-V 是多模态的)

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod global_extract;
pub mod prompts;
pub mod providers;

/// LLM 抽出的结构化字段(对应 documents.extracted_fields JSON)。
///
/// 2026-05-23 晚十三 大扩字段(参考"信息集中管理"小红书参考图)。
/// 所有字段都可为空 / 空数组(LLM 可能识别不到)。前端展示时用 "未识别" 占位。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtractedFields {
    // ===== 案件基本信息 =====
    /// 案号,如 "(2024)苏02民初9999号"
    pub case_no: Option<String>,
    /// 案件类型:民事 / 刑事 / 行政 / 仲裁 / 执行 / 非诉
    pub case_type: Option<String>,
    /// 受理法院全称(参考图叫"承办机关")
    pub court: Option<String>,
    /// 案由,如 "民间借贷纠纷" / "股权转让纠纷"
    pub cause: Option<String>,
    /// 当前阶段:立案 / 一审 / 二审 / 再审 / 执行 / 已结案
    pub case_stage: Option<String>,
    /// 案件状态:进行中 / 已结案 / 已撤诉 / 已调解
    pub case_status: Option<String>,
    /// 起诉/立案日期(ISO 8601: YYYY-MM-DD)
    pub filed_at: Option<String>,
    /// 预计结案日期(YYYY-MM-DD,通常法院文书里有"审限届满日"或起诉书提到的)
    pub expected_close_at: Option<String>,
    /// 案件备注(从文书中抽出的特别提示,如"涉及金额较大,需重点跟进")
    pub case_note: Option<String>,

    // ===== 当事人 =====
    /// 原告/申请人姓名列表(自然人或公司)
    #[serde(deserialize_with = "deserialize_null_default")]
    pub plaintiffs: Vec<String>,
    /// 被告/被申请人姓名列表
    #[serde(deserialize_with = "deserialize_null_default")]
    pub defendants: Vec<String>,
    /// 第三人姓名列表
    #[serde(deserialize_with = "deserialize_null_default")]
    pub third_parties: Vec<String>,
    /// 当事人联系人(委托人 / 法务负责人 / 个人代理人 + 电话 / 邮箱)
    #[serde(deserialize_with = "deserialize_null_default")]
    pub party_contacts: Vec<PartyContact>,

    // ===== 金额 / 收费 =====
    /// 诉讼请求金额(人民币元,可能含小数)。
    /// 容错:LLM 可能输出数字也可能输出字符串(`"50000"` / `"50,000"` 等),
    /// 自定义 deserializer 两种都接受。
    #[serde(deserialize_with = "deserialize_flexible_amount")]
    pub claim_amount: Option<f64>,
    /// 收费记录(案件受理费 / 律师代理费 / 财产保全费 / 材料费 等)
    #[serde(deserialize_with = "deserialize_null_default")]
    pub fees: Vec<FeeRecord>,

    // ===== 法院人员 =====
    /// 承办法官姓名(可能多人合议庭)— V0.1 兼容老 UI 保留,实际用 court_contacts 更全
    #[serde(deserialize_with = "deserialize_null_default")]
    pub judges: Vec<String>,
    /// 法院相关人员的联系方式(主办法官 / 审判员 / 书记员 / 法官助理 + 电话)
    #[serde(deserialize_with = "deserialize_null_default")]
    pub court_contacts: Vec<CourtContact>,

    // ===== 时间线 / 保全 =====
    /// 案件关键日期事件(开庭 / 上诉期 / 举证期 / 保全到期 / 续封 等)
    #[serde(deserialize_with = "deserialize_null_default")]
    pub key_dates: Vec<KeyDate>,
    /// 财产保全记录
    #[serde(deserialize_with = "deserialize_null_default")]
    pub preservations: Vec<Preservation>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CourtContact {
    /// 姓名(2026-05-26 V0.1.12:改 Option,LLM 合议庭只知道职务无名时会返回 null)
    pub name: Option<String>,
    /// 角色:主办法官 / 法官 / 书记员 / 法官助理 / 审判员 / 审判长
    pub role: Option<String>,
    /// 电话(可能没有)
    pub phone: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PartyContact {
    /// 关联的当事人(姓名或简称,例:"林明远 / 张三 / 委托人 / 被告")
    pub party: String,
    /// 联系人姓名(若联系人=当事人本人,name 跟 party 相同)
    /// 2026-05-26 V0.1.12:改 Option — 委托合同里只有机构名(如"江苏隆亿建设工程公司")无具体联系人时,
    /// LLM 会合理返回 null,之前是 String 类型导致 deserialize 失败。
    pub name: Option<String>,
    /// 角色:本人 / 代理人 / 法务负责人 / 法定代表人 / 家属
    pub role: Option<String>,
    /// 联系电话
    pub phone: Option<String>,
    /// 邮箱
    pub email: Option<String>,
    /// 是否为我方当事人(true=委托方/我方;false=对方;null=未知)
    /// 2026-05-23 晚十五:律师只关心自己客户的联系人,对方无需显示
    pub is_our_side: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FeeRecord {
    /// 收费项目:案件受理费 / 律师代理费 / 财产保全费 / 材料费 / 鉴定费 等
    pub item: String,
    /// 金额(元)
    #[serde(deserialize_with = "deserialize_flexible_amount")]
    pub amount: Option<f64>,
    /// 收费/缴费时间 YYYY-MM-DD
    pub charged_at: Option<String>,
    /// 收据号 / 凭证号
    pub receipt_no: Option<String>,
    /// 备注
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyDate {
    /// 事件类型:开庭 / 上诉期 / 举证期 / 质证 / 保全到期 / 续封 / 限消查询 / 缴费 / 送达
    pub event_type: String,
    /// 日期 YYYY-MM-DD
    pub date: Option<String>,
    /// 备注(如"庭前会议" / "二审")
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Preservation {
    /// 保全标的描述(如"股权 / 房产 / 银行存款")
    pub target: String,
    /// 保全金额
    #[serde(deserialize_with = "deserialize_flexible_amount")]
    pub amount: Option<f64>,
    /// 起算日期 YYYY-MM-DD
    pub started_at: Option<String>,
    /// 期限(年),通常 2 或 3
    pub duration_years: Option<u32>,
    /// 到期日 YYYY-MM-DD(若文书里直接有写)
    pub expires_at: Option<String>,
}

/// 容错:把 LLM 可能返回的显式 `null` 兜成 `T::default()`(主要给 `Vec` 数组字段用)。
///
/// 背景(2026-05-31 真机暴露):struct 上的 `#[serde(default)]` 只兜「缺失的键」,
/// **对「键在、值是 null」无效** —— flash 偶尔对空数组字段输出 `"plaintiffs": null`,
/// serde 直接报 `invalid type: null, expected a sequence` 让整份文档抽取 failed
/// (离婚补偿协议.pdf 就这么挂的)。套上本 deserializer:null → 空 Vec,正常数组照解。
fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// 容错反序列化金额:接受数字 / 数字字符串 / 带千分位逗号的字符串 / null。
///
/// 例子:`50000` / `"50000"` / `"50,000"` / `"50000.00"` / `null` 都能解析。
fn deserialize_flexible_amount<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct FlexibleAmount;

    impl<'de> Visitor<'de> for FlexibleAmount {
        type Value = Option<f64>;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a number, a numeric string, or null")
        }

        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            d.deserialize_any(FlexibleAmount)
        }
        fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
            Ok(Some(v))
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(v as f64))
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(v as f64))
        }
        fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
            let cleaned: String = v
                .chars()
                .filter(|c| *c != ',' && *c != ' ' && *c != '\u{ff0c}') // 半角逗号 / 空格 / 全角逗号
                .collect();
            if cleaned.is_empty() {
                return Ok(None);
            }
            cleaned.parse::<f64>().map(Some).map_err(de::Error::custom)
        }
    }

    deserializer.deserialize_any(FlexibleAmount)
}

/// LLM 客户端配置。
#[derive(Debug, Clone)]
pub struct LlmConfig {
    /// chat completions 完整 URL,比如 `http://127.0.0.1:8899/v1/chat/completions`
    pub endpoint: String,
    /// 模型名,比如 `MiniCPM-V-4_6-Q8_0.gguf` 或 `qwen2.5:7b`
    pub model: String,
    /// 可选 API key(云端时填,本机不填)
    pub api_key: Option<String>,
    /// 单次请求超时(秒)
    pub timeout_secs: u64,
    /// 抽取类请求(extract_case_fields / global_extract)用的温度。
    /// DeepSeek / 本机:0.0(确定性);MiniMax M 系列**不能用 0.0**(思考会死循环),用 0.3。
    /// 2026-06-15。注意:chat 链路温度走 `model_router::ModelChoice`,不读本字段。
    pub temperature: f32,
}

impl LlmConfig {
    /// 本机 llama.cpp + MiniCPM-V 4.6 的默认配置。
    pub fn local_llamacpp_default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:8899/v1/chat/completions".to_string(),
            model: "MiniCPM-V-4_6-Q8_0.gguf".to_string(),
            api_key: None,
            timeout_secs: 180,
            temperature: 0.0,
        }
    }

    /// 从用户 Settings 构造 LlmConfig,根据 `effective_llm_provider()` 自动选本机 / 云端。
    ///
    /// 2026-05-23 晚六:LLM 单独维度,跟 OCR 解耦。
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let preset = providers::preset_for(settings);
        if settings.effective_llm_provider() == "cloud" {
            // 2026-06-15:云端后端二选一。MiniMax 协议路径与 DeepSeek 不同(详 from_settings 注释)。
            if settings.effective_cloud_llm_backend() == "minimax" {
                let provider_minimax = matches!(
                    settings.cloud_llm_provider.as_deref().map(str::trim),
                    Some("minimax")
                );
                // MiniMax:自有 v2 协议,聊天路径 /v1/text/chatcompletion_v2(**不是** OpenAI 兼容)。
                let base = if provider_minimax {
                    settings
                        .cloud_llm_endpoint
                        .as_deref()
                        .or(settings.minimax_endpoint.as_deref())
                } else {
                    settings
                        .minimax_endpoint
                        .as_deref()
                        .or(settings.cloud_llm_endpoint.as_deref())
                }
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("https://api.minimaxi.com");
                let endpoint = if base.contains("/chatcompletion_v2") {
                    base.to_string() // 用户已填完整路径,原样用
                } else {
                    format!("{}/v1/text/chatcompletion_v2", base.trim_end_matches('/'))
                };
                let model = if provider_minimax {
                    settings
                        .cloud_llm_model
                        .as_deref()
                        .or(settings.minimax_model.as_deref())
                } else {
                    settings
                        .minimax_model
                        .as_deref()
                        .or(settings.cloud_llm_model.as_deref())
                }
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "auto")
                .unwrap_or("MiniMax-M2")
                .to_string();
                let api_key = if provider_minimax {
                    settings
                        .cloud_llm_api_key
                        .clone()
                        .or_else(|| settings.minimax_api_key.clone())
                } else {
                    settings
                        .minimax_api_key
                        .clone()
                        .or_else(|| settings.cloud_llm_api_key.clone())
                };
                // M 系列恒思考,抽取也不能用 0.0(会死循环);0.3 兼顾确定性与可用。
                return Self {
                    endpoint,
                    model,
                    api_key,
                    timeout_secs: 120, // M 系列思考慢,给足
                    temperature: 0.3,
                };
            }
            // 云端模式:用 cloud_llm_* 字段
            let endpoint = settings
                .cloud_llm_endpoint
                .clone()
                .unwrap_or_else(|| preset.default_endpoint.to_string());
            // 支持 OpenAI-compatible base(/v1)、DeepSeek base、智谱 GLM /paas/v4。
            let endpoint = if endpoint.ends_with("/chat/completions") {
                endpoint
            } else if preset.id == "glm" {
                format!("{}/chat/completions", endpoint.trim_end_matches('/'))
            } else if endpoint.trim_end_matches('/').ends_with("/v1") {
                format!("{}/chat/completions", endpoint.trim_end_matches('/'))
            } else if endpoint.ends_with('/') {
                format!("{}v1/chat/completions", endpoint)
            } else {
                format!("{}/v1/chat/completions", endpoint)
            };
            // cloud_llm_model 现在是「档位」:flash / pro / 'auto'。
            // 'auto'(自动挡)不是合法 API 模型名 → 基础 config 落 flash(具体每次调用的模型
            // 由 model_router::route_model 决定并覆盖 LlmConfig.model,见 chat/commands.rs)。
            let base_model = match settings.cloud_llm_model.as_deref().map(str::trim) {
                Some("auto") | Some("") | None => {
                    if preset.flash_model.is_empty() {
                        "deepseek-v4-flash".to_string()
                    } else {
                        preset.flash_model.to_string()
                    }
                }
                Some(m) => m.to_string(),
            };
            Self {
                endpoint,
                model: base_model,
                api_key: settings.cloud_llm_api_key_for(preset.id),
                timeout_secs: 60, // 云端比本机快,60s 足够;本机要 180s
                temperature: 0.0,
            }
        } else {
            // 纯本地模式:用 ollama_* 字段(实际是 llama-server :8899)
            let endpoint = settings
                .ollama_endpoint
                .clone()
                .unwrap_or_else(|| "http://127.0.0.1:8899".to_string());
            let endpoint = if endpoint.ends_with("/v1/chat/completions") {
                endpoint
            } else if endpoint.ends_with('/') {
                format!("{}v1/chat/completions", endpoint)
            } else {
                format!("{}/v1/chat/completions", endpoint)
            };
            Self {
                endpoint,
                model: settings
                    .ollama_model
                    .clone()
                    .unwrap_or_else(|| "MiniCPM-V-4_6-Q8_0.gguf".to_string()),
                api_key: None,
                timeout_secs: 180,
                temperature: 0.0,
            }
        }
    }
}

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("LLM 服务不可达:{0}")]
    Network(String),
    #[error("LLM 返回错误状态 {0}:{1}")]
    HttpStatus(u16, String),
    #[error("LLM 返回的不是预期 JSON 格式:{0}")]
    ResponseFormat(String),
    #[error("LLM 输出不是有效 JSON:{0}")]
    ContentJson(String),
}

impl serde::Serialize for LlmError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

/// OpenAI 兼容请求体的简化版(只用 messages + temperature + max_tokens)。
#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
}

#[derive(Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

/// OpenAI 兼容响应体(只解析我们关心的部分)。
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}

/// 给一段纯文本(诉状/判决书/笔录等),让 LLM 抽出结构化字段。
///
/// 失败不 panic,返回 LlmError 让调用方决定怎么降级(可以记 `extraction_status = failed`)。
pub async fn extract_case_fields(
    config: &LlmConfig,
    text: &str,
) -> Result<ExtractedFields, LlmError> {
    extract_case_fields_with_hint(config, text, None, None).await
}

/// 2026-05-23 晚十:加 filename + category 提示,让 LLM 更针对性抽取
pub async fn extract_case_fields_with_hint(
    config: &LlmConfig,
    text: &str,
    filename: Option<&str>,
    category: Option<&str>,
) -> Result<ExtractedFields, LlmError> {
    let prompt = prompts::case_fields_extraction_with_hint(text, filename, category);

    let body = ChatRequest {
        model: &config.model,
        messages: vec![ChatMessage {
            role: "user",
            content: &prompt,
        }],
        max_tokens: 4096, // 2026-05-23 晚十三 扩字段后(party_contacts/fees/court_contacts/key_dates/preservations 等),输出可能 1.5-3k tokens
        temperature: config.temperature, // DeepSeek/本机=0.0;MiniMax=0.3(M 系列禁 0.0)
        stream: false,
    };

    let mut req = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.timeout_secs))
        .build()
        .map_err(|e| LlmError::Network(e.to_string()))?
        .post(&config.endpoint)
        .json(&body);

    if let Some(key) = &config.api_key {
        req = req.bearer_auth(key);
    }

    let response = req
        .send()
        .await
        .map_err(|e| LlmError::Network(e.to_string()))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(LlmError::HttpStatus(status.as_u16(), body));
    }

    let parsed: ChatResponse = response
        .json()
        .await
        .map_err(|e| LlmError::ResponseFormat(e.to_string()))?;

    let content = parsed
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| LlmError::ResponseFormat("choices 为空".into()))?;

    // LLM 输出可能带 markdown ```json ... ``` 包裹,容错剥离
    let cleaned = extract_json_from_content(&content);

    serde_json::from_str::<ExtractedFields>(&cleaned)
        .map_err(|e| LlmError::ContentJson(format!("{}; raw = {}", e, content)))
}

/// 从 LLM 返回的内容里抽取出 JSON 对象部分,处理几种常见的包裹:
///
/// - 纯 JSON: `{"case_no": ...}` → 直接返回
/// - markdown 代码块: \`\`\`json\n{...}\n\`\`\` → 剥离围栏
/// - 含前缀:`这是结果:{...}` → 找第一个 `{` 到最后一个 `}`
/// - 含 `<think>...</think>` 思考块: 忽略,取后面的 JSON
fn extract_json_from_content(content: &str) -> String {
    let mut text = content.trim();

    // 1) 剥 <think>...</think> 思考块(MiniCPM/qwen 等推理模型可能输出)
    if let Some(end) = text.find("</think>") {
        text = text[end + "</think>".len()..].trim();
    }

    // 2) 剥 markdown 代码围栏
    if let Some(stripped) = text.strip_prefix("```json") {
        text = stripped.trim();
    } else if let Some(stripped) = text.strip_prefix("```") {
        text = stripped.trim();
    }
    if let Some(stripped) = text.strip_suffix("```") {
        text = stripped.trim();
    }

    // 3) 如果还有前缀,取第一个 { 到匹配的最后一个 }
    if let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) {
        if end > start {
            return text[start..=end].to_string();
        }
    }
    text.to_string()
}
