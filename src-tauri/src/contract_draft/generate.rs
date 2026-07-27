//! 合同起草 LLM 调用(2026-06-18 · 非诉 tab「合同起草」B1)。
//!
//! **Clean-room**:起草方法论(三观四步法 / 三点一线法结构 / 双模式 / 立场化起草)是**思想**,
//! 不受版权保护;本文件 prompt、schema、措辞全部自建,**零照搬** `pa1nrui1/legal-skills`(MIT)
//! 的任何参考 md。方法论致谢小潘律师 / pa1nrui1 / legal-skills,见非诉「合同起草」UI 致谢行。
//!
//! 调用模式复刻 `contract_review::analyze::review_contract`:reqwest POST chat/completions,
//! 云端走 `response_format: json_object`,MiniMax 自有协议不发 response_format。
//!
//! 两段式(对应四步法):
//!   - `plan_contract`(步骤 1-3):三观分析判类型 → 给结构大纲 + 引导式信息采集清单 + 关键追问。
//!   - `generate_contract`(步骤 4):据已采集信息按三点一线法生成完整合同草案 + 关键条款 + 风险。

use serde::{Deserialize, Serialize};

use crate::llm::{LlmConfig, LlmError};

/// 起草立场(代表哪方,决定条款设计与风险倾斜)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftStance {
    /// 代表甲方(出具/主导方)
    PartyA,
    /// 代表乙方(相对方)
    PartyB,
    /// 中立平衡(双方公平,交易能安全落地)
    Neutral,
}

impl DraftStance {
    pub fn from_label(s: &str) -> Self {
        match s.trim() {
            "party_a" | "甲方" | "a" | "A" => DraftStance::PartyA,
            "party_b" | "乙方" | "b" | "B" => DraftStance::PartyB,
            _ => DraftStance::Neutral,
        }
    }
    fn label(self) -> &'static str {
        match self {
            DraftStance::PartyA => "甲方(我方代表甲方,条款设计优先保护甲方权益、控制甲方风险敞口)",
            DraftStance::PartyB => "乙方(我方代表乙方,条款设计优先保护乙方权益、控制乙方风险敞口)",
            DraftStance::Neutral => "中立平衡(不偏向任一方,以交易整体公平、能安全落地为目标)",
        }
    }
    pub fn cn_short(self) -> &'static str {
        match self {
            DraftStance::PartyA => "甲方",
            DraftStance::PartyB => "乙方",
            DraftStance::Neutral => "中立",
        }
    }
}

/// 引导式信息采集的单个要素。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequiredField {
    /// 要素名,如「标的物 / 服务范围」「价款与支付方式」
    pub field: String,
    /// 为什么需要这个信息(给用户解释)
    #[serde(default)]
    pub why: String,
    /// 填写示例 / 提示
    #[serde(default)]
    pub example: String,
    /// 是否必填(false = 建议补充)
    #[serde(default)]
    pub required: bool,
}

/// 步骤 1-3 产出:三观分析 + 结构大纲 + 采集清单。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractDraftPlan {
    /// 三观判定的合同类型(如「房屋租赁合同」)
    #[serde(default)]
    pub contract_type: String,
    /// 交易本质一句话(三观分析:主体/行为/客体)
    #[serde(default)]
    pub transaction_essence: String,
    /// 拟用的合同结构骨架(三点一线法:首部 / 交易结构 / 配套条款 的条款列表)
    #[serde(default)]
    pub structure_outline: Vec<String>,
    /// 引导式信息采集清单
    #[serde(default)]
    pub required_info: Vec<RequiredField>,
    /// 起草前必须先澄清的关键问题(影响合同走向的)
    #[serde(default)]
    pub clarifying_questions: Vec<String>,
    /// 补充说明(立场提醒、风险预警等)
    #[serde(default)]
    pub notes: String,
}

/// 关键条款及其设计理由。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftKeyClause {
    pub clause: String,
    #[serde(default)]
    pub rationale: String,
}

/// 步骤 4 产出:完整合同草案 + 元信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractDraftResult {
    /// 合同类型
    #[serde(default)]
    pub contract_type: String,
    /// 合同名称(用于文件名 / 标题,如「房屋租赁合同」)
    #[serde(default)]
    pub contract_name: String,
    /// 完整合同正文 Markdown(三点一线法结构;待填处用 ____ 占位)
    #[serde(default)]
    pub draft_md: String,
    /// 关键条款及为何这样写(立场化说明)
    #[serde(default)]
    pub key_clauses: Vec<DraftKeyClause>,
    /// 风险提示(本合同特有,差异化)
    #[serde(default)]
    pub risks: Vec<String>,
    /// 起草时所做的假设(用户未提供、AI 暂按常见做法填的;含 ____ 占位处)
    #[serde(default)]
    pub assumptions: Vec<String>,
    /// 仍建议用户补充/核实的信息
    #[serde(default)]
    pub missing_info: Vec<String>,
}

impl ContractDraftResult {
    /// 合同名兜底(LLM 没给时用类型,再兜底「合同」)。
    pub fn safe_name(&self) -> String {
        let n = self.contract_name.trim();
        if !n.is_empty() {
            return n.to_string();
        }
        let t = self.contract_type.trim();
        if !t.is_empty() {
            return t.to_string();
        }
        "合同".to_string()
    }
}

/// 修订(多轮)产出:修订后的完整合同 + 本轮改了什么。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractReviseResult {
    /// 修订后完整合同正文 Markdown
    #[serde(default)]
    pub draft_md: String,
    /// 本轮改了什么 / 为何改(给版本「修订历史」)
    #[serde(default)]
    pub change_summary: String,
    /// 关键改动条款说明
    #[serde(default)]
    pub key_clauses: Vec<DraftKeyClause>,
    /// 修订后仍存在的风险提示
    #[serde(default)]
    pub risks: Vec<String>,
}

fn stance_hint_block(stance: DraftStance, contract_type_hint: &str) -> String {
    let hint = if contract_type_hint.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n- 用户提示的合同类型(供参考,以你三观分析判断为准):{}",
            contract_type_hint.trim()
        )
    };
    format!("- 我方代表立场:{}{}", stance.label(), hint)
}

fn plan_system_prompt(stance: DraftStance, contract_type_hint: &str) -> String {
    let sh = stance_hint_block(stance, contract_type_hint);
    format!(
        r###"你是一名资深商事律师,精通中国合同起草实务。用户会用口语化方式描述一笔交易需求。你的任务是**先做起草前的分析与信息采集规划**(还不要写合同正文),**只输出一个 JSON 对象**(不要解释、不要 markdown 代码块)。

## 立场与提示
{sh}

## 分析方法(三观 → 类型决策)
- 主体观:交易各方的法律地位与利益诉求。
- 行为观:交易行为的法律性质(转让所有权 / 让渡使用权 / 提供劳务成果 / 提供服务 / 委托代办 / 合作经营 / 融资担保 / 人身关系等)。
- 客体观:标的物或权利的特征。
据此精准判定合同类型,并按「三点一线法」(合同首部 / 交易结构条款 / 配套条款)拟出结构骨架。

## 信息采集
按「当事人 → 标的 → 核心条款 → 附属条款」顺序,列出起草这份合同**必须**或**建议**用户提供的要素;对影响合同走向的关键不确定点,单独列为澄清问题。**宁多勿缺**,但不要问无关信息。

## 输出 JSON 结构(严格遵守字段名)
{{
  "contract_type": "你判断的合同类型(如 房屋租赁合同)",
  "transaction_essence": "一句话交易本质(主体/行为/客体)",
  "structure_outline": ["合同首部:主体信息、鉴于条款", "第一条 ...", "第二条 ...", "..."],
  "required_info": [
    {{"field": "要素名(如 出租房屋的坐落与面积)", "why": "为何需要", "example": "填写示例", "required": true}}
  ],
  "clarifying_questions": ["影响合同走向、必须先问清的关键问题"],
  "notes": "立场提醒 / 风险预警 / 特殊说明"
}}

## 硬性要求
- 不臆造法律依据;不确定的标注「待核实」。
- 立场为甲/乙方时,在 notes 里点明本方应特别争取或防范的点。
- 这一步**绝不输出合同正文**,只做类型判定 + 结构骨架 + 采集清单。"###,
        sh = sh,
    )
}

fn draft_system_prompt(stance: DraftStance, contract_type_hint: &str) -> String {
    let sh = stance_hint_block(stance, contract_type_hint);
    format!(
        r###"你是一名资深商事律师,精通中国合同起草实务。用户会给出交易需求与已采集的信息。请据此起草一份**完整、可用、表述严谨**的中文合同草案,**只输出一个 JSON 对象**(不要解释、不要 markdown 代码块,合同正文放进 JSON 字符串字段里)。

## 立场与提示
{sh}

## 起草方法(三点一线法结构)
合同正文按三个版块从上到下排列:
1. 合同首部:合同标题(《X合同》)、合同主体信息、鉴于条款(背景与缔约目的)。
2. 交易结构条款:定义、合同标的、价款与支付、履行方式时间地点、双方权利义务、陈述与保证。
3. 配套条款:违约责任、变更解除终止、争议解决、不可抗力、保密、通知送达、其他。
最后是签署部分(签署日期/地点、各方签字盖章)与附件(如有)。条款编号用「第X条」,款项用(一)(二)(三)。

## 起草要求
- 立场化:始终基于代理方立场设计条款、倾斜风险,但不得写入违法、无效或违反强制性规定的内容。
- 若随附「我已确认的起草偏好」,在**不违反法律强制性规定**的前提下据此取舍条款、设定谈判底线,并在对应 key_clauses 的 rationale 注明「据用户偏好」;偏好绝不能用来降低违法/无效/强制性规范相关风险。
- 用户未提供的具体信息(名称、金额、日期、地址等)一律用 `____` 占位,并在 assumptions / missing_info 中列出,**不要编造**当事人真实信息。
- 口语映射:把口语化表述准确映射到法律术语(如「门面」→「商业用房」)。
- 差异化风险提示:按本合同类型特有风险标注,不套通用模板。
- 语言准确无歧义,权利义务清晰、对等(中立)或合理倾斜(代理方)。

## 输出 JSON 结构(严格遵守字段名)
{{
  "contract_type": "合同类型",
  "contract_name": "合同名称(用于文件名,如 房屋租赁合同)",
  "draft_md": "完整合同正文(Markdown;标题用 # / 条款用 ## 或正文加粗;待填处用 ____)",
  "key_clauses": [{{"clause": "条款标题", "rationale": "为何这样写(立场化说明)"}}],
  "risks": ["本合同特有的风险提示"],
  "assumptions": ["起草时所做的假设(用户未提供、暂按常见做法填或留 ____ 的)"],
  "missing_info": ["仍建议用户补充或核实的信息"]
}}

## 硬性要求
- `draft_md` 必须是**完整可用**的合同全文,不是大纲、不是片段。
- 不臆造法律依据与当事人真实信息;不确定处用 ____ 占位并在 missing_info 说明。
- 涉及期限/自动续约/通知期等条款时,在 key_clauses 或 risks 中提示注意。"###,
        sh = sh,
    )
}

/// 步骤 1-3:起草前规划。`requirement` 是用户对交易的口语化描述。
pub async fn plan_contract(
    config: &LlmConfig,
    requirement: &str,
    stance: DraftStance,
    contract_type_hint: &str,
) -> Result<ContractDraftPlan, LlmError> {
    if requirement.trim().is_empty() {
        return Err(LlmError::ResponseFormat("交易需求为空,无法规划起草".into()));
    }
    let sys = plan_system_prompt(stance, contract_type_hint);
    let user = format!("交易需求描述:\n{}", requirement.trim());
    let cleaned = call_llm_json(config, &sys, &user).await?;
    serde_json::from_str::<ContractDraftPlan>(&cleaned)
        .map_err(|e| LlmError::ContentJson(format!("{}\n---原始---\n{}", e, cleaned)))
}

/// 步骤 4:据已采集信息生成完整合同草案。`collected_info` 是用户对采集清单/追问的回答汇总(可空)。
pub async fn generate_contract(
    config: &LlmConfig,
    requirement: &str,
    stance: DraftStance,
    contract_type_hint: &str,
    collected_info: &str,
) -> Result<ContractDraftResult, LlmError> {
    if requirement.trim().is_empty() {
        return Err(LlmError::ResponseFormat("交易需求为空,无法起草".into()));
    }
    let sys = draft_system_prompt(stance, contract_type_hint);
    let user = if collected_info.trim().is_empty() {
        format!("交易需求描述:\n{}", requirement.trim())
    } else {
        format!(
            "交易需求描述:\n{}\n\n已采集/补充的信息:\n{}",
            requirement.trim(),
            collected_info.trim()
        )
    };
    let cleaned = call_llm_json(config, &sys, &user).await?;
    serde_json::from_str::<ContractDraftResult>(&cleaned)
        .map_err(|e| LlmError::ContentJson(format!("{}\n---原始---\n{}", e, cleaned)))
}

fn revise_system_prompt(stance: DraftStance) -> String {
    format!(
        r###"你是一名资深商事律师。下面给你一份**现有合同草案**和**本轮修订要求**,请据修订要求改出**完整的新版合同**,**只输出一个 JSON 对象**(不要解释、不要 markdown 代码块,合同正文放进 JSON 字符串字段里)。

## 立场
- 我方代表立场:{stance}

## 修订要求
- 只按本轮要求改动,未要求改的条款保持稳定,不要无谓重写。
- 保持三点一线法结构与编号一致;新增/删除条款时保持编号连续。
- 不得写入违法、无效或违反强制性规定的内容;未提供的具体信息用 `____` 占位,不编造。
- 若随附「我已确认的起草偏好」,在不违反强制性规定的前提下据此取舍条款,绝不用偏好降低违法/无效风险。
- 在 change_summary 里**逐条**说明本轮改了哪些条款、新增/删除了什么、为什么改、是否影响对方利益。

## 输出 JSON 结构(严格遵守字段名)
{{
  "draft_md": "修订后的完整合同正文(Markdown)",
  "change_summary": "本轮修订说明(逐条:改了什么/为何改)",
  "key_clauses": [{{"clause": "改动条款", "rationale": "为何这样改"}}],
  "risks": ["修订后仍需注意的风险"]
}}"###,
        stance = stance.label(),
    )
}

/// 多轮修订:据 `feedback` 把 `current_md` 改成新版。
pub async fn revise_contract(
    config: &LlmConfig,
    current_md: &str,
    feedback: &str,
    stance: DraftStance,
) -> Result<ContractReviseResult, LlmError> {
    if current_md.trim().is_empty() {
        return Err(LlmError::ResponseFormat("现有合同为空,无法修订".into()));
    }
    if feedback.trim().is_empty() {
        return Err(LlmError::ResponseFormat("未填写修订要求".into()));
    }
    let sys = revise_system_prompt(stance);
    let user = format!(
        "现有合同草案:\n{}\n\n本轮修订要求:\n{}",
        current_md.trim(),
        feedback.trim()
    );
    let cleaned = call_llm_json(config, &sys, &user).await?;
    serde_json::from_str::<ContractReviseResult>(&cleaned)
        .map_err(|e| LlmError::ContentJson(format!("{}\n---原始---\n{}", e, cleaned)))
}

/// 跑一次 chat completion,返回剥壳后的 JSON 字符串。复刻 review_contract 的调用模式。
async fn call_llm_json(config: &LlmConfig, sys: &str, user: &str) -> Result<String, LlmError> {
    // MiniMax 自有协议不支持 response_format:json_object(对齐 contract_review 注释)。
    let is_minimax = config.endpoint.contains("chatcompletion_v2");
    let mut body = serde_json::json!({
        "model": config.model,
        "messages": [
            {"role": "system", "content": sys},
            {"role": "user", "content": user},
        ],
        // 起草输出(完整合同正文)可能很长;MiniMax 还叠思考 token。
        "max_tokens": if is_minimax { 32768 } else { 16384 },
        "temperature": config.temperature,
        "stream": false,
    });
    if !is_minimax {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }

    let req = reqwest::Client::builder()
        // 起草比抽取更长,给足超时
        .timeout(std::time::Duration::from_secs(config.timeout_secs * 4))
        .build()
        .map_err(|e| LlmError::Network(e.to_string()))?
        .post(&config.endpoint)
        .json(&body);
    let (req, credential) = config
        .authorize_request(req)
        .await
        .map_err(LlmError::Credential)?;

    let response = req.send().await.map_err(|e| {
        let message = credential
            .as_ref()
            .map_or_else(|| e.to_string(), |value| value.redact(&e.to_string()));
        LlmError::Network(message)
    })?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        let text = match credential.as_ref() {
            Some(value) => value.redact(&text),
            None => text,
        };
        return Err(LlmError::HttpStatus(status.as_u16(), text));
    }
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| LlmError::ResponseFormat(e.to_string()))?;
    let content = json
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .ok_or_else(|| LlmError::ResponseFormat("无 choices[0].message.content".into()))?;
    let content = credential
        .as_ref()
        .map_or_else(|| content.to_owned(), |value| value.redact(content));
    Ok(extract_json_object(&content))
}

/// 本地 JSON 提取(与 contract_review::analyze 同款,保持模块解耦):
/// 剥 <think> 块、剥 ```json fence、取首个 `{` 到末个 `}`。
fn extract_json_object(content: &str) -> String {
    let mut s = content.trim();
    if let Some(end) = s.find("</think>") {
        s = s[end + "</think>".len()..].trim();
    }
    if let Some(rest) = s.strip_prefix("```json") {
        s = rest.trim();
    } else if let Some(rest) = s.strip_prefix("```") {
        s = rest.trim();
    }
    if let Some(pos) = s.rfind("```") {
        s = s[..pos].trim();
    }
    if let (Some(start), Some(end)) = (s.find('{'), s.rfind('}')) {
        if end > start {
            return s[start..=end].to_string();
        }
    }
    s.to_string()
}
