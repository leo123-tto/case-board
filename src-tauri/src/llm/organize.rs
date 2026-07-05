//! AI 自动整理(源文件看板 Phase 3b):通读每份材料 → 判 重要度 + 归类 + 证据目录辅助标签。
//! 一次 LLM 调用处理整案材料(省积分);输出建议,由命令层写成 `ai_suggest` 标记。

use serde::{Deserialize, Serialize};

use super::capability::{LlmProviderKind, ProviderCapability};
use super::gateway::{complete_non_stream_chat, LlmChatMessage, NonStreamChatRequest};
use super::{LlmConfig, LlmError};

/// 喂给 AI 的单份材料(id + 文件名 + 正文摘要)。
#[derive(Debug, Clone, Serialize)]
pub struct OrganizeDocInput {
    pub id: String,
    pub filename: String,
    pub snippet: String,
}

/// AI 对单份材料的分类结果。
#[derive(Debug, Clone, Deserialize)]
pub struct DocClassification {
    pub id: String,
    /// 重要 / 普通 / 忽略
    pub importance: String,
    /// 起诉材料 / 证据 / 法院文书 / 对方材料 / 程序文书 / 参考材料 / 其他
    pub category: String,
    /// 原告 / 被告 / 第三人。可多值;判断不出则空数组。
    #[serde(default)]
    pub party_side: Vec<String>,
    /// 有利 / 不利 / 中性。非证据材料或判断不出可为空。
    #[serde(default)]
    pub evidence_attitude: Option<String>,
    /// 起诉/答辩随附 / 举证期限内 / 补充提交 / 二审新证据 / 未提交或待确认。判断不出可为空。
    #[serde(default)]
    pub submission_stage: Option<String>,
    /// 建议的板内显示名(干净、带类型前缀的中文名,如「证据-微信聊天记录」)。
    /// 空字符串 / 缺省 = 不改名(沿用原文件名)。
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClassifyResult {
    items: Vec<DocClassification>,
}

const SYSTEM_PROMPT: &str = r###"你是资深律师助理,擅长把一堆杂乱的案件材料快速整理分类。我会给你同一个案件的若干材料,每份含 id、文件名、正文摘要。

请你**通读后输出一个 JSON 对象**,对**每一份**材料判断六件事:
1. `importance`(三选一):
   - "重要":核心证据 / 裁判文书 / 起诉应诉的关键材料 / 直接影响事实认定或金额的材料。
   - "普通":一般材料。
   - "忽略":明显无关、重复、空白、宣传/模板/广告、与本案无实质关系的材料；参考案例、参考判决和检索资料默认忽略。
2. `category`(**只能从这七个里选一个**):起诉材料 / 证据 / 法院文书 / 对方材料 / 程序文书 / 参考材料 / 其他。
   - 身份证、营业执照、法定代表人身份证明、主体资格证明、被告身份信息、被告身份证等用于立案/起诉的身份材料,默认归入 "起诉材料",不要归入 "证据" 或 "对方材料"。
3. `party_side`:数组,只能填 "原告" / "被告" / "第三人"。优先按材料提交主体/用途判断,不要因为证件主体、文件名里的当事人身份就机械归属。例如原告起诉时随附的原告身份证、被告身份证、被告身份信息、被告营业执照,都属于原告方起诉材料,填["原告"],不要因为证件主体是被告就填["被告"]。只有文件名、目录或正文明确显示是被告/第三人应诉、答辩、举证时提交的本方主体材料,才填对应的["被告"]或["第三人"]。法院文书/传票/裁定可空数组;一份材料同时涉及多方可多填。
4. `evidence_attitude`:只能填 "有利" / "不利" / "中性" 或空字符串。站在材料提交方自身立场判断;证据能支持该方主张=有利,明显削弱该方=不利,仅程序/背景=中性。非证据材料填空字符串。
5. `submission_stage`:只能填 "起诉/答辩随附" / "举证期限内" / "补充提交" / "二审新证据" / "未提交或待确认" 或空字符串。根据文件名、目录、文书内容判断;拿不准填"未提交或待确认";非诉讼提交材料填空字符串。
6. `name`:给这份材料起一个**简洁、能一眼看懂、带类型前缀**的中文显示名,用于在看板里替代杂乱的原始文件名。规则:
   - 证据类 → `证据-<内容简述>`,如「证据-微信聊天记录」「证据-XX买卖合同」「证据-银行转账回单」。
   - 其它类 → 直接用规范文书名,如「民事起诉状」「答辩状」「授权委托书」「(2024)X民初X号判决书」。
   - 控制在 20 字内;**不要带文件扩展名**;不要自己编号(原件本身有案号则保留)。
   - 拿不准 / 原文件名已经够清楚 → 把 name 设为空字符串 ""(表示不改名)。

输出格式严格为(不要任何多余文字、不要 markdown 代码块):
{"items":[{"id":"<原样回填的 id>","importance":"重要","category":"证据","party_side":["原告"],"evidence_attitude":"有利","submission_stage":"举证期限内","name":"证据-微信聊天记录"}]}

要求:每份材料对应且仅对应一项,id 必须原样回填,不要遗漏也不要新增。"###;

/// 摘要单份材料正文的最大字符数(控制 corpus 体积 / 成本)。
const SNIPPET_CAP: usize = 600;
pub const ORGANIZE_BATCH_SIZE: usize = 30;

#[derive(Debug)]
pub struct OrganizeBatch {
    pub docs: Vec<OrganizeDocInput>,
    original_ids: std::collections::HashMap<String, String>,
}

pub fn prepare_batches(docs: &[OrganizeDocInput]) -> Vec<OrganizeBatch> {
    docs.chunks(ORGANIZE_BATCH_SIZE)
        .map(|chunk| {
            let mut original_ids = std::collections::HashMap::new();
            let docs = chunk
                .iter()
                .enumerate()
                .map(|(index, doc)| {
                    let short_id = (index + 1).to_string();
                    original_ids.insert(short_id.clone(), doc.id.clone());
                    OrganizeDocInput {
                        id: short_id,
                        filename: doc.filename.clone(),
                        snippet: doc.snippet.clone(),
                    }
                })
                .collect();
            OrganizeBatch { docs, original_ids }
        })
        .collect()
}

pub fn restore_batch_ids(
    batch: &OrganizeBatch,
    mut results: Vec<DocClassification>,
) -> Result<Vec<DocClassification>, LlmError> {
    if results.len() != batch.docs.len() {
        return Err(LlmError::ResponseFormat(format!(
            "AI 整理返回 {} 项，应为 {} 项",
            results.len(),
            batch.docs.len()
        )));
    }
    let mut seen = std::collections::HashSet::new();
    for result in &mut results {
        let original = batch.original_ids.get(&result.id).ok_or_else(|| {
            LlmError::ResponseFormat(format!("AI 整理返回未知短编号 {}", result.id))
        })?;
        if !seen.insert(result.id.clone()) {
            return Err(LlmError::ResponseFormat(format!(
                "AI 整理重复返回短编号 {}",
                result.id
            )));
        }
        result.id = original.clone();
    }
    Ok(results)
}

/// 截取正文摘要(按字符,不按字节,避免切坏中文)。
pub fn snippet_of(text: &str) -> String {
    text.chars().take(SNIPPET_CAP).collect()
}

/// 一次 LLM 调用对整案材料做 重要度 + 归类 分类。
pub async fn classify_documents(
    config: &LlmConfig,
    docs: &[OrganizeDocInput],
) -> Result<Vec<DocClassification>, LlmError> {
    let mut corpus = String::with_capacity(docs.len() * 200);
    for d in docs {
        corpus.push_str("\n---\nid: ");
        corpus.push_str(&d.id);
        corpus.push_str("\n文件名: ");
        corpus.push_str(&d.filename);
        corpus.push_str("\n正文摘要: ");
        corpus.push_str(if d.snippet.trim().is_empty() {
            "(无可用正文)"
        } else {
            &d.snippet
        });
        corpus.push('\n');
    }

    let capability = ProviderCapability::from_backend("", &config.endpoint, &config.model);
    let max_output_tokens = if capability.kind == LlmProviderKind::MiniMaxNative {
        32768
    } else {
        8192
    };
    let output = complete_non_stream_chat(
        config,
        &capability,
        NonStreamChatRequest {
            messages: vec![
                LlmChatMessage::system(SYSTEM_PROMPT),
                LlmChatMessage::user(corpus),
            ],
            max_output_tokens,
            temperature: config.temperature,
            timeout_secs: Some(config.timeout_secs * 3),
            response_format_json_object: true,
        },
    )
    .await
    .map_err(super::gateway_error_to_llm_error)?;

    let cleaned = super::extract_json_from_content(&output.content);
    let mut result = serde_json::from_str::<ClassifyResult>(&cleaned)
        .map_err(|e| LlmError::ContentJson(format!("{}\n---原始---\n{}", e, cleaned)))?;
    normalize_identity_material_classifications(docs, &mut result.items);
    Ok(result.items)
}

fn normalize_identity_material_classifications(
    docs: &[OrganizeDocInput],
    results: &mut [DocClassification],
) {
    let by_id: std::collections::HashMap<&str, &OrganizeDocInput> =
        docs.iter().map(|doc| (doc.id.as_str(), doc)).collect();
    for result in results {
        let Some(doc) = by_id.get(result.id.as_str()) else {
            continue;
        };
        if is_filing_identity_material(doc) && !is_explicit_defense_identity_material(doc) {
            result.category = "起诉材料".into();
            result.party_side = vec!["原告".into()];
            result.evidence_attitude = Some(String::new());
            if result
                .submission_stage
                .as_deref()
                .map(str::trim)
                .unwrap_or_default()
                .is_empty()
            {
                result.submission_stage = Some("起诉/答辩随附".into());
            }
        }
    }
}

fn is_filing_identity_material(doc: &OrganizeDocInput) -> bool {
    let hay = format!("{} {}", doc.filename, doc.snippet);
    [
        "身份证",
        "身份信息",
        "身份证明",
        "主体资格",
        "营业执照",
        "统一社会信用代码",
        "法定代表人证明",
        "法定代表人身份证明",
        "被告身份",
    ]
    .iter()
    .any(|keyword| hay.contains(keyword))
}

fn is_explicit_defense_identity_material(doc: &OrganizeDocInput) -> bool {
    let hay = format!("{} {}", doc.filename, doc.snippet);
    [
        "答辩",
        "应诉",
        "反诉",
        "被告提交",
        "被告举证",
        "被告证据",
        "第三人提交",
    ]
    .iter()
    .any(|keyword| hay.contains(keyword))
}
