use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Global,
    Case,
}

impl MemoryScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Case => "case",
        }
    }

    pub fn parse_db(value: &str) -> Self {
        match value {
            "global" => Self::Global,
            _ => Self::Case,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTrigger {
    Explicit,
    Implicit,
}

impl MemoryTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::Implicit => "implicit",
        }
    }

    pub fn parse_db(value: &str) -> Self {
        match value {
            "implicit" => Self::Implicit,
            _ => Self::Explicit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryCandidateDraft {
    pub scope: MemoryScope,
    pub case_id: Option<String>,
    pub content: String,
    pub trigger: MemoryTrigger,
    pub confidence: f64,
}

pub fn extract_memory_candidates_from_turn(
    case_id: &str,
    user_message: &str,
    assistant_message: &str,
) -> Vec<MemoryCandidateDraft> {
    let user = user_message.trim();
    if user.is_empty() {
        return Vec::new();
    }

    let lower_signal = format!("{} {}", user, assistant_message.trim());
    let explicit = contains_any(
        &lower_signal,
        &["记住", "记一下", "记下来", "以后都", "以后所有", "下次都"],
    );
    let implicit = contains_any(
        &lower_signal,
        &[
            "搞错了",
            "弄错了",
            "别再",
            "不要再",
            "后面分析都",
            "以后不要",
        ],
    );
    if !explicit && !implicit {
        return Vec::new();
    }

    let scope = if contains_any(
        user,
        &[
            "以后所有案件",
            "所有案件",
            "全局",
            "以后回答",
            "以后都先",
            "下次都先",
        ],
    ) {
        MemoryScope::Global
    } else {
        MemoryScope::Case
    };
    let content = clean_candidate_content(user);
    if content.chars().count() < 8 {
        return Vec::new();
    }

    vec![MemoryCandidateDraft {
        scope,
        case_id: (scope == MemoryScope::Case).then(|| case_id.to_string()),
        content,
        trigger: if explicit {
            MemoryTrigger::Explicit
        } else {
            MemoryTrigger::Implicit
        },
        confidence: if explicit { 0.95 } else { 0.78 },
    }]
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn clean_candidate_content(input: &str) -> String {
    let mut text = input.trim();
    for prefix in [
        "请记住：",
        "请记住:",
        "记住：",
        "记住:",
        "记一下：",
        "记一下:",
        "记下来：",
        "记下来:",
    ] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim();
            break;
        }
    }
    text.trim_matches(['。', '，', '；', ' ', '\n', '\t'])
        .to_string()
}
