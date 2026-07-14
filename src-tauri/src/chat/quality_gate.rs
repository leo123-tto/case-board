//! Task-level output quality gate for the case AI assistant.

use super::agent_loop::ToolCallRecord;
use super::citations::Citation;
use super::context::TaskType;
use super::task_contract::{ArtifactPolicy, CitationPolicy, TaskContract};

#[derive(Debug, Clone)]
pub struct QualityGateInput<'a> {
    pub task: TaskType,
    pub content: &'a str,
    pub citations: &'a [Citation],
    pub tool_calls: &'a [ToolCallRecord],
    pub ask_user_present: bool,
    pub artifact_doc_id: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualityGateReport {
    pub passed: bool,
    /// true 表示当前内容只是过程稿/半截终稿，不能作为正常完成消息落库。
    pub incomplete: bool,
    pub warnings: Vec<String>,
}

pub fn evaluate_task_quality(input: QualityGateInput<'_>) -> QualityGateReport {
    let contract = TaskContract::for_task(input.task);
    let mut warnings = Vec::new();

    // ask_user 是中间确认态,不是最终法律输出;本轮应停下来等用户,不要求引用或 artifact。
    if input.ask_user_present {
        return QualityGateReport {
            passed: true,
            incomplete: false,
            warnings,
        };
    }

    warnings.extend(missing_tool_requirement_warnings(
        &contract,
        input.tool_calls,
    ));

    // 2026-07-14:VisualizeCase 专项 — 文本级反伪造检测。LLM 可能不调 ask_user
    // 但在正文里写「已确认/已收到你的多选/全选」伪造用户授权;若已发出通用
    // 「既未发起视图多选,也未创建或更新工作区」警告,这里把它升级为更尖锐的
    // 专门警告,直接点出"伪造"问题,让律师一眼看清根因。
    if contract.task == TaskType::VisualizeCase
        && !has_successful_tool(input.tool_calls, "ask_user")
        && !has_successful_tool(input.tool_calls, "save_case_visualization")
        && !has_successful_tool(input.tool_calls, "apply_case_visual_update")
        && looks_like_fake_user_confirmation(input.content)
    {
        warnings.retain(|w| w != "案情可视化任务既未发起视图多选，也未创建或更新工作区。");
        warnings.push(
            "案情可视化任务伪造了用户授权:正文里出现「已确认/已收到多选/全选/你已选择」类措辞,但本轮没有真正调用 ask_user 工具发起选项式追问。用户实际并未从选项卡片里点选,所有「已选/全选」均为模型伪造。请改用 ask_user 让用户真正选择;在用户点选前不得在正文声称「已收到/已确认/全选」。"
                .into(),
        );
    }

    let incomplete = task_output_incomplete(input.task, input.content);
    if incomplete {
        warnings.push(
            "输出未完成: 模拟对抗必须形成对方主张、我方应对、待证事实/证据缺口和整体风险，不能以检索计划或中间步骤代替终稿。"
                .into(),
        );
    }

    if matches!(
        contract.citation_policy,
        CitationPolicy::RequiredForLegalConclusion
    ) && input.citations.is_empty()
        && looks_like_legal_output(input.content)
    {
        warnings
            .push("缺少引用: 本任务形成了法律分析或法律结论,但未解析到 <CITATIONS> 来源。".into());
    }

    if matches!(contract.artifact_policy, ArtifactPolicy::Required)
        && input.artifact_doc_id.is_none()
        && !has_successful_tool(input.tool_calls, "save_artifact")
        && looks_like_final_analysis(input.content)
    {
        warnings.push(
            "未落 artifact: 本任务的最终成果应使用 save_artifact 落盘,聊天只保留摘要和入口。"
                .into(),
        );
    }

    QualityGateReport {
        passed: warnings.is_empty(),
        incomplete,
        warnings,
    }
}

pub fn task_output_incomplete(task: TaskType, content: &str) -> bool {
    if task != TaskType::SimulateOpposition {
        return false;
    }
    let text = content.trim();
    if text.is_empty() {
        return true;
    }
    let has_opponent_position = ["对方可能主张", "对方主张", "对方最强论点"]
        .iter()
        .any(|marker| text.contains(marker));
    let has_our_response = text.contains("我方应对");
    let has_evidence_work = ["待证事实", "证据缺口", "补强证据"]
        .iter()
        .any(|marker| text.contains(marker));
    let has_risk = ["整体风险", "风险提示", "薄弱点"]
        .iter()
        .any(|marker| text.contains(marker));
    !(has_opponent_position && has_our_response && has_evidence_work && has_risk)
}

pub fn format_quality_gate_note(report: &QualityGateReport) -> String {
    if report.passed || report.warnings.is_empty() {
        return String::new();
    }
    let mut out =
        String::from("\n\n> 质量提示: 本轮输出可能不满足任务契约,请复核后再作为正式办案依据。\n");
    for warning in &report.warnings {
        out.push_str(&format!("> - {}\n", warning));
    }
    out.trim_end().to_string()
}

fn missing_tool_requirement_warnings(
    contract: &TaskContract,
    tool_calls: &[ToolCallRecord],
) -> Vec<String> {
    match contract.task {
        TaskType::FreeChat => Vec::new(),
        TaskType::CompileLegalBasis => require_any(
            tool_calls,
            "缺少法律依据核验: 本任务应至少成功调用本地知识库全文工具或元典法规工具中的一项。",
            &[
                "search_local_kb",
                "semantic_search_local_kb",
                "read_kb_file",
                "search_laws",
                "get_law_article",
                "law_vector_search",
            ],
        ),
        TaskType::FindSimilarCases => {
            let mut warnings = require_any(
                tool_calls,
                "缺少类案检索: 本任务应至少成功调用本地知识库或元典类案检索工具中的一项。",
                &[
                    "search_local_kb",
                    "semantic_search_local_kb",
                    "search_cases_authority",
                    "search_cases_normal",
                    "case_vector_search",
                ],
            );
            warnings.extend(require_any(
                tool_calls,
                "缺少类案详情核验: 精选类案应读取本地全文或调用元典案例详情后再评价支持度。",
                &["read_kb_file", "get_case_detail"],
            ));
            warnings
        }
        TaskType::VerifyMyDraft => {
            let mut warnings = require_tool(
                tool_calls,
                "缺少草稿读取: 引用核校应先成功调用 read_case_doc 读取草稿全文。",
                "read_case_doc",
            );
            warnings.extend(require_tool(
                tool_calls,
                "缺少引用核校: 本任务应成功调用 verify_legal_citations 后再给出核校结论。",
                "verify_legal_citations",
            ));
            warnings
        }
        TaskType::SimulateOpposition => {
            let mut warnings = require_any(
                tool_calls,
                "缺少案件材料核验: 模拟对抗应至少成功调用 list_case_docs / read_case_doc 中的一项。",
                &["list_case_docs", "read_case_doc"],
            );
            warnings.extend(require_any(
                tool_calls,
                "缺少攻防依据核验: 模拟对抗应至少成功调用法条或类案检索工具。",
                &[
                    "search_laws",
                    "get_law_article",
                    "law_vector_search",
                    "search_local_kb",
                    "semantic_search_local_kb",
                    "read_kb_file",
                    "search_cases_authority",
                    "search_cases_normal",
                    "case_vector_search",
                ],
            ));
            warnings
        }
        TaskType::VisualizeCase => require_any(
            tool_calls,
            "案情可视化任务既未发起视图多选，也未创建或更新工作区。",
            &[
                "ask_user",
                "save_case_visualization",
                "apply_case_visual_update",
            ],
        ),
        TaskType::DeepAnalysis | TaskType::CriminalDeepAnalysis => {
            let mut warnings = require_any(
                tool_calls,
                "缺少案件材料核验: 深度分析应至少成功调用 list_case_docs / read_case_doc / semantic_search_case_docs 中的一项。",
                &["list_case_docs", "read_case_doc", "semantic_search_case_docs"],
            );
            warnings.extend(require_any(
                tool_calls,
                "缺少法条核验: 深度分析引用法条前应至少成功调用本地知识库全文工具或元典法规工具中的一项。",
                &[
                    "search_local_kb",
                    "semantic_search_local_kb",
                    "read_kb_file",
                    "search_laws",
                    "get_law_article",
                    "law_vector_search",
                ],
            ));
            warnings
        }
    }
}

fn require_any(tool_calls: &[ToolCallRecord], warning: &str, names: &[&str]) -> Vec<String> {
    if tool_calls
        .iter()
        .any(|call| call.success && names.contains(&call.tool.as_str()))
    {
        Vec::new()
    } else {
        vec![warning.to_string()]
    }
}

fn require_tool(tool_calls: &[ToolCallRecord], warning: &str, name: &str) -> Vec<String> {
    if has_successful_tool(tool_calls, name) {
        Vec::new()
    } else {
        vec![warning.to_string()]
    }
}

fn has_successful_tool(tool_calls: &[ToolCallRecord], name: &str) -> bool {
    tool_calls
        .iter()
        .any(|call| call.success && call.tool == name)
}

fn looks_like_legal_output(content: &str) -> bool {
    let text = content.trim();
    if text.is_empty() {
        return false;
    }
    [
        "法条",
        "法律依据",
        "民法典",
        "刑法",
        "司法解释",
        "判决",
        "裁定",
        "案例",
        "案号",
        "请求权",
        "抗辩",
        "违约",
        "侵权",
        "责任",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

fn looks_like_final_analysis(content: &str) -> bool {
    let text = content.trim();
    if text.is_empty() {
        return false;
    }
    text.contains("##")
        || text.contains("结论")
        || text.contains("分析")
        || text.contains("请求权")
        || text.contains("三阶层")
        || text.chars().count() >= 800
}

/// 2026-07-14:VisualizeCase 专项 — 检测 LLM 是否在正文里伪造「用户已选择/已确认」。
///
/// 真实合法选择只能来自本轮真正调起 `ask_user` 工具并收到用户答复。LLM 在没有
/// 调过 `ask_user` 的情况下,在正文里写「已确认/已收到多选/全选/你已选择」类
/// 措辞 = 伪造用户授权,既骗律师又破坏任务契约。质量门会专门拦这一组合。
///
/// 故意只覆盖「声称用户已选」的措辞;不拦 AI 在多选问题里写「请选择」之类的
/// 正常引导语,也不拦 AI 在多选问题中给「建议全选」类选项描述(那个写在
/// ask_user 参数里,不是正文)。
fn looks_like_fake_user_confirmation(content: &str) -> bool {
    let text = content.trim();
    if text.is_empty() {
        return false;
    }
    // 「声称已收到/已确认/全选/多选结果/你已选择」类措辞,任一命中即视为伪造。
    const PHRASES: &[&str] = &[
        "已确认收到",
        "已收到你的多选",
        "已收到你的选择",
        "已收到你选择",
        "已确认你的多选",
        "已确认你的选择",
        "已确认你选择",
        "已收到选择",
        "已确认选择",
        "你的多选结果",
        "你的多选结果为",
        "你的选择是",
        "你已选择",
        "你已勾选",
        "你已确认",
        "我看到你选择了",
        "我看到你勾选了",
        "你选择的是",
        "已选定为",
        "已选定了",
    ];
    PHRASES.iter().any(|p| text.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(tool: &str, success: bool) -> ToolCallRecord {
        ToolCallRecord {
            tool: tool.to_string(),
            args: serde_json::json!({}),
            kb_hit: false,
            credits_used: 0,
            success,
            error_short: None,
            started_at_ms: 0,
            finished_at_ms: 0,
        }
    }

    #[test]
    fn fake_confirmation_patterns_are_detected() {
        assert!(looks_like_fake_user_confirmation(
            "已确认收到你的多选结果 1+2+3+4 全选。"
        ));
        assert!(looks_like_fake_user_confirmation(
            "你已选择时间线、关系图、证据矩阵、思维导图。"
        ));
        assert!(looks_like_fake_user_confirmation(
            "我看到你选择了 4 张视图,正在生成..."
        ));
        assert!(looks_like_fake_user_confirmation(
            "你的多选结果: 1、2、3、4。"
        ));
    }

    #[test]
    fn normal_guidance_text_is_not_flagged() {
        // 引导语(让用户选)不应被误判
        assert!(!looks_like_fake_user_confirmation(
            "为把这份内容写准确,我先和你确认几点 👇"
        ));
        assert!(!looks_like_fake_user_confirmation("请选择要生成的视图"));
        assert!(!looks_like_fake_user_confirmation(
            "下面请你选择(可多选,建议全选)—— 4 张图覆盖了..."
        ));
        // 空内容 / 不相关文本
        assert!(!looks_like_fake_user_confirmation(""));
        assert!(!looks_like_fake_user_confirmation("已收到其他类型反馈"));
    }

    #[test]
    fn visualize_case_without_any_tool_but_with_fake_text_fires_specific_warning() {
        let input = QualityGateInput {
            task: TaskType::VisualizeCase,
            content: "已确认收到你的多选结果 1+2+3+4 全选,正在生成 4 张视图。",
            citations: &[],
            tool_calls: &[],
            ask_user_present: false,
            artifact_doc_id: None,
        };
        let report = evaluate_task_quality(input);
        assert!(!report.passed);
        assert!(
            report.warnings.iter().any(|w| w.contains("伪造")),
            "应当点出伪造问题;实际 warnings: {:?}",
            report.warnings
        );
        // 旧的通用警告不应再出现,被替换为更尖锐的版本
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| w == "案情可视化任务既未发起视图多选，也未创建或更新工作区。"),
            "通用警告应被替换;实际 warnings: {:?}",
            report.warnings
        );
    }

    #[test]
    fn visualize_case_without_tools_and_no_fake_text_keeps_generic_warning() {
        let input = QualityGateInput {
            task: TaskType::VisualizeCase,
            content: "为把这份内容写准确,我先和你确认几点 👇",
            citations: &[],
            tool_calls: &[],
            ask_user_present: false,
            artifact_doc_id: None,
        };
        let report = evaluate_task_quality(input);
        assert!(!report.passed);
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w == "案情可视化任务既未发起视图多选，也未创建或更新工作区。"),
            "没有伪造措辞时应保留通用警告;实际 warnings: {:?}",
            report.warnings
        );
        assert!(
            !report.warnings.iter().any(|w| w.contains("伪造")),
            "无伪造措辞时不应触发伪造警告;实际 warnings: {:?}",
            report.warnings
        );
    }

    #[test]
    fn visualize_case_with_ask_user_tool_passes() {
        let calls = [rec("ask_user", true)];
        let input = QualityGateInput {
            task: TaskType::VisualizeCase,
            content: "为把这份内容写准确,我先和你确认几点 👇",
            citations: &[],
            tool_calls: &calls,
            ask_user_present: true,
            artifact_doc_id: None,
        };
        let report = evaluate_task_quality(input);
        assert!(report.passed, "ask_user 路径应 pass;warnings: {:?}", report.warnings);
    }

    #[test]
    fn visualize_case_with_save_tool_passes() {
        let calls = [rec("save_case_visualization", true)];
        let input = QualityGateInput {
            task: TaskType::VisualizeCase,
            content: "已生成 4 张视图并直接加入工作台,可在工作台查看。",
            citations: &[],
            tool_calls: &calls,
            ask_user_present: false,
            artifact_doc_id: None,
        };
        let report = evaluate_task_quality(input);
        assert!(
            report.passed,
            "save 工具调用路径应 pass;warnings: {:?}",
            report.warnings
        );
    }
}
