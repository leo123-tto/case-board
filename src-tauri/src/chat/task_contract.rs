//! Explicit success contracts for case-aware AI tasks.
//!
//! The long task prompts still carry detailed workflow instructions. This module
//! keeps the non-negotiable success criteria in code so the agent runtime and
//! future quality gates can share the same task-level contract.

use super::context::TaskType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactPolicy {
    Optional,
    Required,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskUserPolicy {
    Optional,
    RequiredBeforeFinal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CitationPolicy {
    WhenCitingSources,
    RequiredForLegalConclusion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TaskContract {
    pub task: TaskType,
    pub name: &'static str,
    pub required_tools: &'static [&'static str],
    pub citation_policy: CitationPolicy,
    pub ask_user_policy: AskUserPolicy,
    pub artifact_policy: ArtifactPolicy,
    pub success_criteria: &'static [&'static str],
}

impl TaskContract {
    pub fn for_task(task: TaskType) -> Self {
        match task {
            TaskType::FreeChat => Self {
                task,
                name: "自由问答",
                required_tools: &[],
                citation_policy: CitationPolicy::WhenCitingSources,
                ask_user_policy: AskUserPolicy::Optional,
                artifact_policy: ArtifactPolicy::Optional,
                success_criteria: &[
                    "涉及本案事实时必须先基于案件快照、引用文件或工具结果;证据不足时明确说不确定",
                    "涉及法条、案例、法院口径时必须先用工具核验,不能凭训练记忆编条号或案号",
                    "用户要求起草、改写、生成长文档时优先落 artifact,聊天里只给摘要和待确认点",
                ],
            },
            TaskType::CompileLegalBasis => Self {
                task,
                name: "法律依据清单",
                required_tools: &[
                    "search_local_kb",
                    "read_kb_file",
                    "semantic_search_local_kb",
                    "search_laws",
                    "get_law_article",
                    "law_vector_search",
                ],
                citation_policy: CitationPolicy::RequiredForLegalConclusion,
                ask_user_policy: AskUserPolicy::Optional,
                artifact_policy: ArtifactPolicy::Optional,
                success_criteria: &[
                    "先判断案件阶段,执行阶段和审理阶段分别组织法律依据",
                    "每一条法律依据都必须有工具核验过的法规名、条号和原文要点",
                    "正文每条具体依据必须有 [N] 标记,末尾必须有对应 <CITATIONS> JSON 块",
                    "必须列出对方可能援引的不利依据或争议观点,不能只堆我方有利条文",
                ],
            },
            TaskType::FindSimilarCases => Self {
                task,
                name: "类案检索",
                required_tools: &[
                    "search_local_kb",
                    "semantic_search_local_kb",
                    "read_kb_file",
                    "search_cases_authority",
                    "search_cases_normal",
                    "case_vector_search",
                    "get_case_detail",
                ],
                citation_policy: CitationPolicy::RequiredForLegalConclusion,
                ask_user_policy: AskUserPolicy::Optional,
                artifact_policy: ArtifactPolicy::Optional,
                success_criteria: &[
                    "至少读取精选类案的本地全文或元典详情后再评价支持度,不得只根据标题或摘要下结论",
                    "类案支持度必须锚定我方代理立场,说明支持、不利或中性的理由",
                    "地域相似性只是排序因素,不能替代核心要件相似性分析",
                ],
            },
            TaskType::VerifyMyDraft => Self {
                task,
                name: "草稿引用核校",
                required_tools: &["read_case_doc", "verify_legal_citations"],
                citation_policy: CitationPolicy::RequiredForLegalConclusion,
                ask_user_policy: AskUserPolicy::Optional,
                artifact_policy: ArtifactPolicy::Optional,
                success_criteria: &[
                    "必须读取用户引用或正在编辑的草稿全文,逐条核校法条和案号",
                    "只把工具确认存在且内容一致的引用标为可放心使用",
                    "未命中或不一致的引用必须给出可操作的修正建议或人工核对提示",
                ],
            },
            TaskType::SimulateOpposition => Self {
                task,
                name: "模拟对抗",
                required_tools: &[
                    "list_case_docs",
                    "read_case_doc",
                    "search_local_kb",
                    "semantic_search_local_kb",
                    "read_kb_file",
                    "search_laws",
                    "get_law_article",
                    "search_cases_normal",
                ],
                citation_policy: CitationPolicy::RequiredForLegalConclusion,
                ask_user_policy: AskUserPolicy::Optional,
                artifact_policy: ArtifactPolicy::Optional,
                success_criteria: &[
                    "攻防方向必须锚定我方代理立场;立场未知时先询问,不得猜测",
                    "每个对方主张都要区分否认、抗辩、反抗辩和举证责任",
                    "必须列出我方最需要补强的证据缺口和对方最可能突破的薄弱点",
                ],
            },
            TaskType::VisualizeCase => Self {
                task,
                name: "案情可视化",
                required_tools: &[
                    "list_case_docs",
                    "read_case_doc",
                    "ask_user",
                    "get_case_visualization",
                    "save_case_visualization",
                    "apply_case_visual_update",
                ],
                citation_policy: CitationPolicy::WhenCitingSources,
                ask_user_policy: AskUserPolicy::RequiredBeforeFinal,
                artifact_policy: ArtifactPolicy::Optional,
                success_criteria: &[
                    "首次点击先分析后一次多选；用户选择前不写入，选择后只生成所选视图",
                    "正文不复述工具参数、不伪造授权或工具结果；已有工作区直接安全合并",
                ],
            },
            TaskType::DeepAnalysis => Self {
                task,
                name: "民事深度分析",
                required_tools: &[
                    "list_case_docs",
                    "read_case_doc",
                    "search_local_kb",
                    "semantic_search_local_kb",
                    "read_kb_file",
                    "search_laws",
                    "get_law_article",
                    "ask_user",
                    "save_artifact",
                ],
                citation_policy: CitationPolicy::RequiredForLegalConclusion,
                ask_user_policy: AskUserPolicy::RequiredBeforeFinal,
                artifact_policy: ArtifactPolicy::Required,
                success_criteria: &[
                    "必须经过候选请求权基础确认和分析大纲确认两道闸,不能跳步直接写全文",
                    "逐要件论证必须遵守请求权基础检视顺序和比例原则",
                    "最终深度分析报告必须落 artifact,聊天里只返回摘要、入口和律师需把关事项",
                ],
            },
            TaskType::CriminalDeepAnalysis => Self {
                task,
                name: "刑事深度分析",
                required_tools: &[
                    "list_case_docs",
                    "read_case_doc",
                    "search_local_kb",
                    "semantic_search_local_kb",
                    "read_kb_file",
                    "search_laws",
                    "get_law_article",
                    "ask_user",
                    "save_artifact",
                ],
                citation_policy: CitationPolicy::RequiredForLegalConclusion,
                ask_user_policy: AskUserPolicy::RequiredBeforeFinal,
                artifact_policy: ArtifactPolicy::Required,
                success_criteria: &[
                    "必须经过候选罪名确认和三阶层检视大纲确认两道闸,不能跳步直接写全文",
                    "逐要件论证必须按构成要件该当性、违法性、有责性展开",
                    "最终刑事深度分析报告必须落 artifact,并明确事实存疑和律师需把关事项",
                ],
            },
        }
    }

    pub fn system_prompt_section(&self) -> String {
        let tools = if self.required_tools.is_empty() {
            "按需使用工具".to_string()
        } else {
            self.required_tools.join(" / ")
        };
        let citation = match self.citation_policy {
            CitationPolicy::WhenCitingSources => "引用本案材料、法条或案例时必须给出来源",
            CitationPolicy::RequiredForLegalConclusion => {
                "凡形成法律结论、法条或类案判断,必须有工具核验来源和 <CITATIONS>"
            }
        };
        let ask_user = match self.ask_user_policy {
            AskUserPolicy::Optional => "信息不足时可追问,不要自行脑补关键事实",
            AskUserPolicy::RequiredBeforeFinal => {
                "最终输出前必须完成要求的 ask_user 确认闸,未确认不得继续写最终稿"
            }
        };
        let artifact = match self.artifact_policy {
            ArtifactPolicy::Optional => "长文档按用户意图落 artifact,短答可直接回复",
            ArtifactPolicy::Required => "最终成果必须使用 save_artifact 落盘,聊天只给摘要和入口",
        };

        let mut out = String::from("\n════════════════ 本轮任务契约 ════════════════\n");
        out.push_str(&format!("- 任务: {}\n", self.name));
        out.push_str(&format!("- 必要工具: {}\n", tools));
        out.push_str(&format!("- 引用要求: {}\n", citation));
        out.push_str(&format!("- 追问要求: {}\n", ask_user));
        out.push_str(&format!("- 成果形态: {}\n", artifact));
        out.push_str(
            "- 可视化工具: get_case_visualization 可读取现状；save_case_visualization / apply_case_visual_update 写入前须先取得用户同意；明确授权后直接应用，不再二次审核\n",
        );
        out.push_str("- 成功标准:\n");
        for item in self.success_criteria {
            out.push_str(&format!("  - {}\n", item));
        }
        out
    }
}

pub fn task_contract_prompt(task: TaskType) -> String {
    TaskContract::for_task(task).system_prompt_section()
}
