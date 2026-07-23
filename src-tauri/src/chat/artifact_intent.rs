//! 用户显式要求“保存为文件”时的交付契约。
//!
//! 模型仍可以主动调用 `save_artifact`，但“是否真的产生工作区文件”由宿主层兜底，
//! 不能只依赖模型是否记得调用工具。

const ARTIFACT_CONTRACT_PROMPT: &str = r#"

【本轮交付契约：必须保存工作区文件】
用户明确要求把本轮成果保存为文件。请在完成全文后调用 `create_workspace_file`（正式诉讼文书也可用 `save_artifact`），将完整 Markdown 保存到当前派生工作区；最终回复应简要说明已保存的成果，不得只声称保存而没有实际工具结果。若工具暂时不可用，仍须在最终回复中给出完整、可直接保存的 Markdown 正文，CaseBoard 会在成功完成后执行宿主兜底保存。
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactIntent {
    None,
    Required,
}

impl ArtifactIntent {
    pub fn from_user_message(message: &str) -> Self {
        let text = message.trim().to_lowercase();
        if text.is_empty() || contains_save_negation(&text) {
            return Self::None;
        }

        let explicit_phrase = [
            "保存下来",
            "保存为",
            "保存成",
            "保存到工作区",
            "保存至工作区",
            "存下来",
            "存为",
            "存成",
            "存到工作区",
            "存进工作区",
            "新存一个",
            "新存一份",
            "重新存一个",
            "重新存一份",
            "写入工作区",
            "放到工作区",
            "落盘",
            "输出为文件",
            "输出成文件",
            "生成文件",
            "导出为",
            "导出成",
        ]
        .iter()
        .any(|phrase| text.contains(phrase));

        let save_with_file_target = ["保存", "存储", "写入"]
            .iter()
            .any(|verb| text.contains(verb))
            && ["markdown", "md 文件", "md文档", "文件", "工作区"]
                .iter()
                .any(|target| text.contains(target));

        if explicit_phrase || save_with_file_target {
            Self::Required
        } else {
            Self::None
        }
    }

    pub const fn requires_artifact(self) -> bool {
        matches!(self, Self::Required)
    }

    pub const fn prompt_contract(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Required => ARTIFACT_CONTRACT_PROMPT,
        }
    }

    /// 仅在一次运行已经正常完成、没有追问、正文完整且尚无真实 artifact 时兜底。
    pub fn should_create_fallback(
        self,
        ask_user_present: bool,
        output_incomplete: bool,
        content: &str,
        artifact_doc_id: Option<&str>,
    ) -> bool {
        self.requires_artifact()
            && !ask_user_present
            && !output_incomplete
            && !content.trim().is_empty()
            && artifact_doc_id.is_none()
    }
}

fn contains_save_negation(text: &str) -> bool {
    [
        "不要保存",
        "不用保存",
        "无需保存",
        "不必保存",
        "暂不保存",
        "暂时不保存",
        "先不保存",
        "先别保存",
        "别保存",
        "不要存",
        "不用存",
        "无需生成文件",
        "不要生成文件",
        "不用生成文件",
        "不要导出",
        "不用导出",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}
