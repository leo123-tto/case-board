//! 云端 LLM 提供商预设表。
//!
//! 集中管理 DeepSeek / 小米 MiMo / 自定义(Custom) 三个提供商的差异点:
//!   - 默认 endpoint / 模型名 / max_output_tokens
//!   - 是否走 DeepSeek 专属 /beta/ 路径
//!   - 是否有余额查询 API

/// 提供商预设。
pub struct ProviderPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub default_endpoint: &'static str,
    pub flash_model: &'static str,
    pub pro_model: &'static str,
    pub thinking_model: Option<&'static str>,
    pub max_output_tokens: u32,
    pub use_beta_path: bool,
    pub has_balance_api: bool,
}

pub static DEEPSEEK: ProviderPreset = ProviderPreset {
    id: "deepseek",
    label: "DeepSeek",
    default_endpoint: "https://api.deepseek.com",
    flash_model: "deepseek-v4-flash",
    pro_model: "deepseek-v4-pro",
    thinking_model: Some("deepseek-v4-pro-thinking"),
    max_output_tokens: 384_000,
    use_beta_path: true,
    has_balance_api: true,
};

pub static MIMO: ProviderPreset = ProviderPreset {
    id: "mimo",
    label: "小米 MiMo",
    default_endpoint: "https://token-plan-cn.xiaomimimo.com/v1",
    flash_model: "mimo-v2.5",
    pro_model: "mimo-v2.5-pro",
    thinking_model: None,
    max_output_tokens: 131_072,
    use_beta_path: false,
    has_balance_api: false,
};

pub static MINIMAX: ProviderPreset = ProviderPreset {
    id: "minimax",
    label: "MiniMax",
    default_endpoint: "https://api.minimaxi.com",
    flash_model: "MiniMax-M2",
    pro_model: "MiniMax-M2",
    thinking_model: None,
    max_output_tokens: 32_768,
    use_beta_path: false,
    has_balance_api: false,
};

pub static GLM: ProviderPreset = ProviderPreset {
    id: "glm",
    label: "智谱 GLM",
    default_endpoint: "https://open.bigmodel.cn/api/paas/v4",
    flash_model: "glm-4.7",
    pro_model: "glm-5.2",
    thinking_model: Some("glm-5-turbo"),
    max_output_tokens: 32_768,
    use_beta_path: false,
    has_balance_api: false,
};

pub static CUSTOM: ProviderPreset = ProviderPreset {
    id: "custom",
    label: "自定义",
    default_endpoint: "",
    flash_model: "",
    pro_model: "",
    thinking_model: None,
    max_output_tokens: 32_768,
    use_beta_path: false,
    has_balance_api: false,
};

/// 按 Settings.cloud_llm_provider 查找预设，未知值回退 DeepSeek。
pub fn preset_for_id(provider: Option<&str>) -> &'static ProviderPreset {
    match provider.map(str::trim).filter(|s| !s.is_empty()) {
        Some("mimo") => &MIMO,
        Some("minimax") => &MINIMAX,
        Some("glm") => &GLM,
        Some("custom") => &CUSTOM,
        _ => &DEEPSEEK,
    }
}

/// 从 Settings 取预设的便捷函数。
pub fn preset_for(settings: &crate::settings::Settings) -> &'static ProviderPreset {
    preset_for_id(settings.cloud_llm_provider.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_defaults_to_deepseek() {
        assert_eq!(preset_for_id(None).id, "deepseek");
        assert_eq!(preset_for_id(Some("")).id, "deepseek");
        assert_eq!(preset_for_id(Some("unknown")).id, "deepseek");
    }

    #[test]
    fn preset_resolves_mimo() {
        let p = preset_for_id(Some("mimo"));
        assert_eq!(p.id, "mimo");
        assert_eq!(p.flash_model, "mimo-v2.5");
        assert!(!p.use_beta_path);
        assert!(!p.has_balance_api);
    }

    #[test]
    fn preset_resolves_custom() {
        let p = preset_for_id(Some("custom"));
        assert_eq!(p.id, "custom");
        assert!(p.default_endpoint.is_empty());
    }

    #[test]
    fn deepseek_preset_values() {
        let p = &DEEPSEEK;
        assert_eq!(p.max_output_tokens, 384_000);
        assert!(p.use_beta_path);
        assert!(p.has_balance_api);
        assert_eq!(p.thinking_model, Some("deepseek-v4-pro-thinking"));
    }
}
