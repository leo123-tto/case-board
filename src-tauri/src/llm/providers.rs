//! 通用 OpenAI 兼容云端 LLM 提供商预设(2026-06-16)。
//!
//! 仅服务于「OpenAI 兼容」这一条后端(`cloud_llm_backend` ∈ {glm, mimo, kimi, custom});
//! DeepSeek / MiniMax 各有专属处理,**不**走这里。每个预设只提供「默认 endpoint + 默认模型名」
//! 两样,作为前端预填值 + 后端兜底(用户没填时不至于打空请求)。模型名一律可在设置里改。
//!
//! endpoint / 模型默认值取自外部贡献者 gcheng-001 的 PR #9(case-board);其中 GLM 用本人
//! 校准过的稳定型号,MiMo 直接沿用 PR 值(本机无 key 实测,以服务商控制台为准、可改)。

/// 一个 OpenAI 兼容服务商预设。
pub struct CompatPreset {
    /// 标识(= `cloud_llm_backend` 取值之一)
    pub id: &'static str,
    /// 展示名
    pub label: &'static str,
    /// 默认 chat completions 完整 URL(已含到 `/chat/completions`)
    pub default_endpoint: &'static str,
    /// 默认模型名(可在设置里改)
    pub default_model: &'static str,
}

pub static GLM: CompatPreset = CompatPreset {
    id: "glm",
    label: "智谱 GLM",
    default_endpoint: "https://open.bigmodel.cn/api/paas/v4/chat/completions",
    default_model: "glm-4.6",
};

pub static MIMO: CompatPreset = CompatPreset {
    id: "mimo",
    label: "小米 MiMo",
    default_endpoint: "https://token-plan-cn.xiaomimimo.com/v1/chat/completions",
    default_model: "mimo-v2.5",
};

pub static KIMI: CompatPreset = CompatPreset {
    id: "kimi",
    label: "Kimi Coding Plan",
    default_endpoint: "https://api.kimi.com/coding/v1/chat/completions",
    default_model: "kimi-for-coding",
};

pub static CUSTOM: CompatPreset = CompatPreset {
    id: "custom",
    label: "自定义(OpenAI 兼容)",
    default_endpoint: "",
    default_model: "",
};

/// 按后端 id 取预设;非兼容档(deepseek/minimax/未知)返回 `None`。
pub fn compat_preset(id: &str) -> Option<&'static CompatPreset> {
    match id.trim() {
        "glm" => Some(&GLM),
        "mimo" => Some(&MIMO),
        "kimi" => Some(&KIMI),
        "custom" => Some(&CUSTOM),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kimi_coding_plan_uses_official_openai_preset() {
        let preset = compat_preset("kimi").expect("Kimi preset should exist");
        assert_eq!(preset.id, "kimi");
        assert_eq!(
            preset.default_endpoint,
            "https://api.kimi.com/coding/v1/chat/completions"
        );
        assert_eq!(preset.default_model, "kimi-for-coding");
    }
}
