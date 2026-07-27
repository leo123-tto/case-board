//! 合同审查 LLM 调用(2026-06-17 · 合同审查 P1)。
//!
//! **Clean-room**:审查方法论(三层扫描 / P0-P1-P2 分级 / 立场×口径 / 四步流程)是**思想**,
//! 不受版权保护;本文件 prompt、schema、措辞全部自建,**零照搬** `contract-copilot` 的任何
//! 知识 md / Python 代码。方法论致谢见 `docs/提案-合同审查-2026-06-17.md`。
//!
//! 调用模式复刻 `llm::global_extract::extract_combined`:reqwest POST `/chat/completions`,
//! 云端走 `response_format: json_object`,MiniMax 自有协议不发 response_format。

use serde::{Deserialize, Serialize};

use crate::llm::{LlmConfig, LlmError};

/// 我方审查立场。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stance {
    /// 代表甲方(出具/主导方,常见为买方/委托方/债权人,具体看合同)
    PartyA,
    /// 代表乙方(相对方,常见为卖方/受托方/债务人)
    PartyB,
    /// 中立审查(双方共用 / 内部合规)
    Neutral,
}

impl Stance {
    pub fn from_label(s: &str) -> Self {
        match s.trim() {
            "party_a" | "甲方" | "a" | "A" => Stance::PartyA,
            "party_b" | "乙方" | "b" | "B" => Stance::PartyB,
            _ => Stance::Neutral,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Stance::PartyA => "甲方(我方代表甲方,优先保护甲方权益、控制甲方风险敞口)",
            Stance::PartyB => "乙方(我方代表乙方,优先保护乙方权益、控制乙方风险敞口)",
            Stance::Neutral => "中立(不偏向任一方,以交易整体能安全落地为目标)",
        }
    }
    /// 给前端 / 意见书显示的简短中文。
    pub fn cn_short(self) -> &'static str {
        match self {
            Stance::PartyA => "甲方",
            Stance::PartyB => "乙方",
            Stance::Neutral => "中立",
        }
    }
}

/// 审查口径(强弱),只影响风险识别与结论表达强度,不直接决定落痕方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strictness {
    /// 克制:只挑硬伤与高风险,尽量不打扰交易
    Lenient,
    /// 常规:标准审查力度
    Normal,
    /// 强势:尽可能多地为我方争取,逐条挑剔
    Aggressive,
}

impl Strictness {
    pub fn from_label(s: &str) -> Self {
        match s.trim() {
            "lenient" | "克制" => Strictness::Lenient,
            "aggressive" | "强势" => Strictness::Aggressive,
            _ => Strictness::Normal,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Strictness::Lenient => "克制(只标 P0/P1 硬伤与高风险,尽量不在低价值表述上纠缠)",
            Strictness::Normal => "常规(标准审查力度,P0/P1/P2 全覆盖)",
            Strictness::Aggressive => {
                "强势(逐条挑剔,尽可能为我方争取更有利条款,允许多标 P2 优化项)"
            }
        }
    }
    /// 给前端 / 意见书显示的简短中文。
    pub fn cn_short(self) -> &'static str {
        match self {
            Strictness::Lenient => "克制",
            Strictness::Normal => "常规",
            Strictness::Aggressive => "强势",
        }
    }
}

/// 单条风险点。字段与 redline 阶段对齐:`paragraph_index` + `anchor_text` 用于定位,
/// `recommended_text` 用于落正文,`action` 决定修订还是仅批注。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewRisk {
    /// 风险等级:P0 / P1 / P2
    pub level: String,
    /// 风险名称(短)
    pub title: String,
    /// 条款位置(如「第3.2条」「付款条款」),给人看
    #[serde(default)]
    pub clause_ref: String,
    /// 该风险所在段落编号(对应喂给 LLM 的 [P{n}] 标号);无法定位时为 null
    #[serde(default)]
    pub paragraph_index: Option<usize>,
    /// 原文精确片段(必须是该段落里**逐字复制**的子串,供 redline 定位);仅批注/无定位时可空
    #[serde(default)]
    pub anchor_text: String,
    /// 风险后果
    #[serde(default)]
    pub consequence: String,
    /// 法律依据(可空)
    #[serde(default)]
    pub basis: String,
    /// 整改建议(说明性)
    #[serde(default)]
    pub suggestion: String,
    /// 推荐措辞(可直接落入合同正文替换 anchor_text);仅批注时可空
    #[serde(default)]
    pub recommended_text: String,
    /// 落痕方式:revise(改正文+批注说明)/ comment(仅批注)
    #[serde(default = "default_action")]
    pub action: String,
}

fn default_action() -> String {
    "comment".to_string()
}

impl ReviewRisk {
    /// 规范化等级,非法值落 P2。
    pub fn norm_level(&self) -> &str {
        match self.level.trim().to_uppercase().as_str() {
            "P0" => "P0",
            "P1" => "P1",
            _ => "P2",
        }
    }
    pub fn wants_revise(&self) -> bool {
        self.action.trim() == "revise" && !self.anchor_text.trim().is_empty()
    }
}

/// 审查结论。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewConclusion {
    /// 可签 / 有条件可签 / 不建议签
    #[serde(default)]
    pub verdict: String,
    /// 签署前先决事项
    #[serde(default)]
    pub preconditions: Vec<String>,
    /// 综合审查意见(几句话)
    #[serde(default)]
    pub summary: String,
}

/// 完整审查结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractReviewResult {
    /// 识别出的合同类型(LLM 判断,自由文本,如「设计委托合同(承揽)」)
    #[serde(default)]
    pub contract_type: String,
    pub conclusion: ReviewConclusion,
    #[serde(default)]
    pub risks: Vec<ReviewRisk>,
}

impl ContractReviewResult {
    /// 按等级排序(P0 → P1 → P2),给前端 / 报告稳定顺序。
    pub fn sorted_risks(&self) -> Vec<ReviewRisk> {
        let mut v = self.risks.clone();
        v.sort_by_key(|r| match r.norm_level() {
            "P0" => 0,
            "P1" => 1,
            _ => 2,
        });
        v
    }
}

fn system_prompt(stance: Stance, strictness: Strictness, contract_type_hint: &str) -> String {
    let hint = if contract_type_hint.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n用户提示的合同类型(供参考,以你判断为准):{}\n",
            contract_type_hint.trim()
        )
    };
    format!(
        r###"你是一名资深商事律师,精通中国合同审查实务。我会给你一份合同的全文,正文按段落用 [P编号] 标注。请你审查并**只输出一个 JSON 对象**(不要任何解释、不要 markdown 代码块)。

## 审查立场与口径
- 我方代表立场:{stance}
- 审查口径:{strictness}{hint}

## 审查方法(三层 + 分级)
1. 宏观层(交易结构):合同类型是否匹配交易实质;主体是否适格、授权是否完整;标的是否合法可履行;关键程序(审批/登记/备案/内部决议)是否完备;付款—交付—担保—退出闭环是否可执行。
2. 中观层(文本与形式):合同形式是否匹配业务阶段;格式条款是否合规、提示说明义务是否可举证;主合同与附件/订单是否一致。
3. 微观层(条款与语言):核心条款(标的/价款/履行/违约/解除/赔偿/争议解决/通知送达)是否齐全;权利义务是否清晰、对等、可执行;语言是否准确无歧义。

## 风险分级
- P0:可能影响合同效力、导致重大损失或重大争议。签署前必须优先处理。
- P1:显著增加争议或履约成本。建议优先谈判修改。
- P2:表述或流程优化项。

## 输出 JSON 结构(严格遵守字段名)
{{
  "contract_type": "你判断的合同类型",
  "conclusion": {{
    "verdict": "可签 | 有条件可签 | 不建议签",
    "preconditions": ["签署前必须完成的前置事项", "..."],
    "summary": "2-4 句综合审查意见"
  }},
  "risks": [
    {{
      "level": "P0 | P1 | P2",
      "title": "风险名称(短)",
      "clause_ref": "条款位置(如 第3.2条 / 付款条款)",
      "paragraph_index": 12,
      "anchor_text": "从该段落里**逐字复制**的、需要改或批注的原文片段",
      "consequence": "风险后果",
      "basis": "法律依据(可留空字符串)",
      "suggestion": "整改建议",
      "recommended_text": "可直接替换 anchor_text 落入正文的推荐措辞(仅批注类可留空)",
      "action": "revise | comment"
    }}
  ]
}}

## 硬性要求
- `anchor_text` 必须是对应 `paragraph_index` 段落文本里**连续、逐字**出现的子串(含标点),不得改写、不得跨段、不得概括。定位不到精确原文时,把 `action` 设为 "comment" 并尽量给出 `paragraph_index`。
- `action="revise"` 仅用于:确定性改正(笔误/术语统一)、或你能给出可直接落文的 `recommended_text` 的条款整改。涉及商务谈判、事实待补、需保留弹性的,一律 "comment"。
- 缺失关键条款(原文没有对应段落)时,`paragraph_index` 用 null、`anchor_text` 留空、`action="comment"`,在 `suggestion` 说明应补什么。
- 不臆造法律依据;无法确认的写「待核实」。立场为甲/乙方时,优先识别**对我方不利**的条款。
- 至少覆盖该合同最重要的若干风险点;按重要性,不要堆砌无价值的 P2。"###,
        stance = stance.label(),
        strictness = strictness.label(),
        hint = hint,
    )
}

/// 跑一次合同审查。`numbered_text` 来自 `parse::ParsedContract::numbered_text()`。
pub async fn review_contract(
    config: &LlmConfig,
    numbered_text: &str,
    stance: Stance,
    strictness: Strictness,
    contract_type_hint: &str,
) -> Result<ContractReviewResult, LlmError> {
    if numbered_text.trim().is_empty() {
        return Err(LlmError::ResponseFormat("合同正文为空,无法审查".into()));
    }
    let sys = system_prompt(stance, strictness, contract_type_hint);

    // MiniMax 自有协议不支持 response_format:json_object(对齐 global_extract 注释)。
    let is_minimax = config.endpoint.contains("chatcompletion_v2");
    let mut body = serde_json::json!({
        "model": config.model,
        "messages": [
            {"role": "system", "content": sys},
            {"role": "user", "content": numbered_text},
        ],
        // 审查输出可能较长(多风险点 + 推荐措辞);MiniMax 还叠思考 token。
        "max_tokens": if is_minimax { 32768 } else { 16384 },
        "temperature": config.temperature,
        "stream": false,
    });
    if !is_minimax {
        body["response_format"] = serde_json::json!({"type": "json_object"});
    }

    let req = reqwest::Client::builder()
        // 审查比抽取更长,给足超时
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
    let cleaned = extract_json_object(&content);
    serde_json::from_str::<ContractReviewResult>(&cleaned)
        .map_err(|e| LlmError::ContentJson(format!("{}\n---原始---\n{}", e, cleaned)))
}

/// 本地 JSON 提取(不依赖 llm 模块私有 helper,保持 contract_review 解耦):
/// 剥 ```json fence、剥 <think> 块、取首个 `{` 到末个 `}`。
fn extract_json_object(content: &str) -> String {
    let mut s = content.trim();
    // 剥 <think>...</think>(推理型模型可能前置)
    if let Some(end) = s.find("</think>") {
        s = s[end + "</think>".len()..].trim();
    }
    // 剥 markdown fence
    if let Some(rest) = s.strip_prefix("```json") {
        s = rest.trim();
    } else if let Some(rest) = s.strip_prefix("```") {
        s = rest.trim();
    }
    if let Some(pos) = s.rfind("```") {
        s = s[..pos].trim();
    }
    // 取第一个 { 到最后一个 }
    if let (Some(start), Some(end)) = (s.find('{'), s.rfind('}')) {
        if end > start {
            return s[start..=end].to_string();
        }
    }
    s.to_string()
}
