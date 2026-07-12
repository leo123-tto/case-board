//! API key 在线验证(2026-05-25 V0.1.6)。
//!
//! 给前端 Settings / Onboarding 里的「验证」按钮用,验证成功 → 前端在 input 旁打绿勾。
//!
//! 设计:
//!   - DeepSeek 用 `GET /user/balance`(免费,延时低,标准做法)
//!   - MinerU 没有专用 verify 端点(2026-05 查询官方文档),用 `GET /extract/task/<bogus-uuid>`
//!     看返回:401/403 = key 无效;200/404 + 业务错误码 = key 有效(只是任务不存在)
//!   - 元典(open.chineselaw.com)用 `GET /rh_enterpriseSearch?name=test&top_k=1`
//!     看返回:HTTP 401 + `code:401` = key 无效;HTTP 200 = key 有效。会消耗 1 次企业搜索配额。
//!
//! 三个函数都设 8s 超时,不阻塞主线程。

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VerifyResult {
    /// 是否验证通过
    pub ok: bool,
    /// 失败时给前端展示的中文消息(成功时为空)
    pub message: String,
}

impl VerifyResult {
    fn ok() -> Self {
        Self {
            ok: true,
            message: String::new(),
        }
    }
    fn fail(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            message: msg.into(),
        }
    }
}

/// 验证 MinerU API token。
///
/// 走 `GET https://mineru.net/api/v4/extract/task/<bogus-uuid>` 看响应:
///   - 401 / 403 → token 无效或过期
///   - 200 / 404 + 业务错误码("task not found" 等)→ token 通过认证
///
/// 用户的 bogus task id 全 0,MinerU 一定查不到,只能返回 token 错或业务"未找到"。
pub async fn verify_mineru_key(token: &str) -> VerifyResult {
    let token = token.trim();
    if token.is_empty() {
        return VerifyResult::fail("Token 为空");
    }
    if !token.starts_with("eyJ") {
        return VerifyResult::fail("格式不像 MinerU JWT token(应以 eyJ 开头)");
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => return VerifyResult::fail(format!("HTTP 客户端创建失败: {}", e)),
    };

    let resp = match client
        .get("https://mineru.net/api/v4/extract/task/00000000-0000-0000-0000-000000000000")
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return VerifyResult::fail(format!("网络错误: {}", e)),
    };

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();

    if status.as_u16() == 401 || status.as_u16() == 403 {
        return VerifyResult::fail("Token 无效或已过期,请到 MinerU 控制台重新获取");
    }

    // MinerU 也可能用 200 + 业务 errorCode 表达 token 错
    // 文档里 A0202 = Token Error
    if body.contains("A0202") || body.contains("\"errCode\":\"A0202\"") {
        return VerifyResult::fail("Token 无效(MinerU A0202)");
    }

    // 401 / 403 / A0202 之外都视为通过(查询不存在的任务返回 404 / 业务错都正常)
    VerifyResult::ok()
}

/// 验证 PaddleOCR VL(百度 AI Studio 星河社区)访问令牌(2026-06-12)。
///
/// AI Studio 没有专用 verify 端点,走 `GET /ocr/jobs/00000`(假 job id)看响应
/// (作者 token 实测):
///   - 401 / 403 → token 无效
///   - 404 + 业务 code 11001「jobId 不存在」→ token 通过认证
pub async fn verify_paddle_vl_key(token: &str) -> VerifyResult {
    let token = token.trim();
    if token.is_empty() {
        return VerifyResult::fail("访问令牌为空");
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => return VerifyResult::fail(format!("HTTP 客户端创建失败: {}", e)),
    };

    let resp = match client
        .get("https://paddleocr.aistudio-app.com/api/v2/ocr/jobs/00000")
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return VerifyResult::fail(format!("网络错误: {}", e)),
    };

    if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 {
        return VerifyResult::fail("访问令牌无效或已过期,请到 AI Studio 重新获取");
    }
    // 401/403 之外都视为通过(查不存在的 job 返回 404 + code 11001 是预期)
    VerifyResult::ok()
}

/// 验证 DeepSeek API key。
///
/// 走 `GET {endpoint}/user/balance` 看 200。
pub async fn verify_deepseek_key(api_key: &str, endpoint: Option<&str>) -> VerifyResult {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return VerifyResult::fail("API Key 为空");
    }
    if !api_key.starts_with("sk-") {
        return VerifyResult::fail("格式不像 DeepSeek API key(应以 sk- 开头)");
    }

    let base = endpoint
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("https://api.deepseek.com");
    let base = base.trim_end_matches('/');
    let url = format!("{}/user/balance", base);

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => return VerifyResult::fail(format!("HTTP 客户端创建失败: {}", e)),
    };

    let resp = match client
        .get(&url)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return VerifyResult::fail(format!("网络错误: {}", e)),
    };

    let status = resp.status();
    if status.is_success() {
        return VerifyResult::ok();
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return VerifyResult::fail("API Key 无效或已过期");
    }
    let body = resp.text().await.unwrap_or_default();
    VerifyResult::fail(format!(
        "HTTP {} · {}",
        status.as_u16(),
        body.chars().take(200).collect::<String>()
    ))
}

/// 验证 MiniMax API key(2026-06-15)。
///
/// MiniMax 聊天走自有协议 `/v1/text/chatcompletion_v2`,但**鉴权 / 列模型**用 OpenAI 兼容的
/// `/v1/models`。复用 DeepSeek 的 verify 套路:打 `/v1/models`,200 = ok / 401|403 = key 失效。
pub async fn verify_minimax_key(api_key: &str, endpoint: Option<&str>) -> VerifyResult {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return VerifyResult::fail("API Key 为空");
    }
    if api_key.len() < 8 {
        return VerifyResult::fail("API Key 太短");
    }

    let base = endpoint
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("https://api.minimaxi.com");
    // 容错:用户可能填了 base、/v1、或完整的 chatcompletion_v2 路径 —— 一律归一到 `<base>/v1/models`。
    let base = base.trim_end_matches('/');
    let base = base
        .trim_end_matches("/v1/text/chatcompletion_v2")
        .trim_end_matches("/v1")
        .trim_end_matches('/');
    let url = format!("{}/v1/models", base);

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => return VerifyResult::fail(format!("HTTP 客户端创建失败: {}", e)),
    };

    let resp = match client
        .get(&url)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return VerifyResult::fail(format!("网络错误: {}", e)),
    };

    let status = resp.status();
    if status.is_success() {
        return VerifyResult::ok();
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return VerifyResult::fail("API Key 无效或已过期(401/403)");
    }
    let body = resp.text().await.unwrap_or_default();
    VerifyResult::fail(format!(
        "HTTP {} · {}",
        status.as_u16(),
        body.chars().take(200).collect::<String>()
    ))
}

/// 验证「通用 OpenAI 兼容」后端(智谱 GLM / 小米 MiMo / 自定义 · 2026-06-16)。
///
/// 各家服务商 `/models` 路径不统一(GLM 是 `/api/paas/v4`,无标准 `/v1`),故**直接打一发最小
/// chat completion**(`max_tokens:16`)—— 一次验证 key + 接口地址 + 模型名三者是否都对。
/// 区分失败类型,便于用户定位:
///   - 401/403 → key 无效
///   - 404 / 400 → 接口地址或模型名可能不对(可在设置里改后重试)
pub async fn verify_openai_compat_key(api_key: &str, endpoint: &str, model: &str) -> VerifyResult {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return VerifyResult::fail("API Key 为空");
    }
    let model = model.trim();
    if model.is_empty() {
        return VerifyResult::fail("模型名为空(请在下方填具体型号,如 glm-4.6)");
    }
    let url = crate::llm::compat_chat_url(endpoint);
    if url.is_empty() {
        return VerifyResult::fail("接口地址为空");
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => return VerifyResult::fail(format!("HTTP 客户端创建失败: {}", e)),
    };

    let body = serde_json::json!({
        "model": model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 16,
        "stream": false,
    });

    let resp = match client
        .post(&url)
        .bearer_auth(api_key)
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return VerifyResult::fail(format!("网络错误: {}", e)),
    };

    let status = resp.status();
    if status.is_success() {
        return VerifyResult::ok();
    }
    let code = status.as_u16();
    if code == 401 || code == 403 {
        return VerifyResult::fail("API Key 无效或已过期(401/403)");
    }
    let snippet = resp
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect::<String>();
    if code == 404 || code == 400 {
        return VerifyResult::fail(format!(
            "接口地址或模型名可能不对({}),可在设置里修改后重试 · {}",
            code, snippet
        ));
    }
    VerifyResult::fail(format!("HTTP {} · {}", code, snippet))
}

/// 验证元典(open.chineselaw.com)API key。
///
/// 2026-07-12 起改用 `yuandian-law` MCP 自带的免费余额工具验证。旧版会先调 1 分
/// 企业搜索、再调 50 分 hall_detect，单点一次「验证」就可能浪费 51 分；余额工具既能
/// 证明 Bearer key 有效，又不消费法律业务接口积分。
pub async fn verify_yuandian_key(api_key: &str) -> VerifyResult {
    let api_key = api_key.trim();
    if api_key.is_empty() {
        return VerifyResult::fail("API Key 为空");
    }
    if !api_key.starts_with("sk_") {
        return VerifyResult::fail("格式不像元典 API key(应以 sk_ 开头,注意是下划线)");
    }

    match crate::yuandian::balance::verify_api_key(api_key).await {
        Ok(_) => VerifyResult::ok(),
        Err(error) => {
            let lower = error.to_ascii_lowercase();
            if lower.contains("401") || lower.contains("403") {
                VerifyResult::fail("API Key 无效、已过期或未授权 MCP 服务(401/403)")
            } else {
                VerifyResult::fail(format!(
                    "元典 MCP 余额验证失败: {}",
                    error.chars().take(200).collect::<String>()
                ))
            }
        }
    }
}
