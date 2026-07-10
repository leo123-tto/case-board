//! 飞书日历读取(精简自外部贡献 PR #9,gcheng-001)。
//!
//! 只复用本机 `lark-cli --as user` 的登录态,CaseBoard **不保存飞书 token**。
//! 本模块只做两件事:
//!   1. 读飞书日历(`lark-cli calendar +agenda`)→ 首页月历展示;
//!   2. (可选)按事件标题在飞书"案件池"多维表格里反查本地案件目录 → 一键导入。
//!
//! 原 PR 的反向同步(案件→飞书表)、到期提醒推送未纳入(摘增量、避免 dead code)。
//!
//! 跨平台:lark-cli 在 macOS 走 Homebrew 路径,其他平台(Windows/Linux)靠 PATH
//! 找 `lark-cli`(Windows 会自动匹配 `lark-cli.exe`);也可在设置里填 CLI 全路径。

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

use crate::settings::Settings;

const LARK_CLI_TIMEOUT: Duration = Duration::from_secs(30);

/// 飞书日历事件(传给前端月历)。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeishuCalendarEvent {
    pub event_id: String,
    pub summary: String,
    pub start_date: String,
    pub end_date: Option<String>,
    pub is_all_day: bool,
    pub description: Option<String>,
    pub location: Option<String>,
    pub app_link: Option<String>,
}

/// 计算 lark-cli 可执行文件:优先用设置里填的全路径,否则按平台兜底。
pub fn lark_bin(settings: &Settings) -> String {
    if let Some(p) = settings.feishu_lark_cli_path.as_deref() {
        let p = p.trim();
        if !p.is_empty() {
            return shellexpand::tilde(p).into_owned();
        }
    }
    default_lark_bin()
}

fn default_lark_bin() -> String {
    #[cfg(target_os = "macos")]
    {
        if Path::new("/opt/homebrew/bin/lark-cli").exists() {
            return "/opt/homebrew/bin/lark-cli".to_string();
        }
        if Path::new("/usr/local/bin/lark-cli").exists() {
            return "/usr/local/bin/lark-cli".to_string();
        }
    }
    // Windows / Linux:靠系统 PATH。Windows 的 npm `.cmd` shim 由 proc_util 包装 cmd /C。
    "lark-cli".to_string()
}

/// 统一注入 lark-cli 运行环境。
///
/// PATH 注入只在 Unix 生效:macOS 下 Tauri 应用进程的 PATH 常缺 Homebrew 目录,
/// 必须补上才找得到 lark-cli。**Windows 上绝不能覆盖 PATH** —— 那会让系统找不到
/// `lark-cli.exe`(它不在这些 Unix 目录里),是致命 bug。
fn apply_lark_env(cmd: &mut Command) {
    cmd.env("LARK_CLI_NO_PROXY", "1");
    #[cfg(unix)]
    cmd.env(
        "PATH",
        "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
    );
    // Windows 下隐藏 lark-cli 控制台窗口,否则每次取飞书日历都闪一个黑色命令框。
    crate::proc_util::hide_console_window(cmd);
}

/// 调一次 lark-cli 的 `api` 子命令(复用用户登录态),返回解析后的 JSON。
async fn lark_cli_api(
    bin: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    let mut cmd = crate::proc_util::tokio_command(bin);
    apply_lark_env(&mut cmd);
    cmd.arg("api")
        .arg(method)
        .arg(path)
        .arg("--as")
        .arg("user")
        .arg("--format")
        .arg("json");

    if let Some(body) = body {
        cmd.arg("--data")
            .arg(serde_json::to_string(&body).map_err(|e| e.to_string())?);
    }

    let output = timeout(LARK_CLI_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "lark-cli 调用超时".to_string())?
        .map_err(|e| format!("无法启动 lark-cli(确认已安装并加入 PATH): {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "lark-cli 调用失败: {}{}",
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!(" · {}", stdout.trim())
            }
        ));
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|e| format!("lark-cli 输出非 UTF-8: {}", e))?;
    let value: Value =
        serde_json::from_str(&stdout).map_err(|e| format!("lark-cli 输出非 JSON: {}", e))?;
    ensure_lark_ok(value)
}

fn ensure_lark_ok(value: Value) -> Result<Value, String> {
    if let Some(code) = value.get("code").and_then(Value::as_i64) {
        if code != 0 {
            let msg = value
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            return Err(format!("飞书 API 返回 code={}: {}", code, msg));
        }
    }
    Ok(value)
}

fn clean_required(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

/// 从飞书日历获取指定日期范围内的事件。
///
/// 使用 `lark-cli calendar +agenda --as user` 获取(复用本机登录态)。
pub async fn fetch_calendar_events(
    bin: &str,
    start: &str,
    end: &str,
) -> Result<Vec<FeishuCalendarEvent>, String> {
    let mut cmd = crate::proc_util::tokio_command(bin);
    apply_lark_env(&mut cmd);
    cmd.arg("calendar")
        .arg("+agenda")
        .arg("--as")
        .arg("user")
        .arg("--start")
        .arg(start)
        .arg("--end")
        .arg(end)
        .arg("--format")
        .arg("json");

    let output = timeout(LARK_CLI_TIMEOUT, cmd.output())
        .await
        .map_err(|_| "lark-cli 日历查询超时".to_string())?
        .map_err(|e| format!("无法启动 lark-cli(确认已安装并加入 PATH): {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "飞书日历查询失败: {}{}",
            stderr.trim(),
            if stdout.trim().is_empty() {
                String::new()
            } else {
                format!(" · {}", stdout.trim())
            }
        ));
    }

    let stdout =
        String::from_utf8(output.stdout).map_err(|e| format!("lark-cli 输出非 UTF-8: {}", e))?;
    let value: Value =
        serde_json::from_str(&stdout).map_err(|e| format!("lark-cli 输出非 JSON: {}", e))?;

    let events = value
        .pointer("/data")
        .and_then(Value::as_array)
        .ok_or_else(|| "飞书日历响应缺少 data".to_string())?;

    let mut result = Vec::new();
    for event in events {
        let event_id = event
            .get("event_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let summary = event
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("(无标题)")
            .to_string();

        // 解析开始时间(date=全天 / datetime=带时刻)
        let start_time = event.get("start_time");
        let (start_date, is_all_day) = if let Some(st) = start_time {
            if let Some(date) = st.get("date").and_then(Value::as_str) {
                (date.to_string(), true)
            } else if let Some(datetime) = st.get("datetime").and_then(Value::as_str) {
                let date = datetime.split('T').next().unwrap_or(datetime);
                (date.to_string(), false)
            } else {
                continue;
            }
        } else {
            continue;
        };

        let end_date = event.get("end_time").and_then(|et| {
            et.get("date")
                .or_else(|| et.get("datetime"))
                .and_then(Value::as_str)
                .map(|s| {
                    if s.contains('T') {
                        s.split('T').next().unwrap_or(s).to_string()
                    } else {
                        s.to_string()
                    }
                })
        });

        let description = event
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string);
        let location = event
            .get("location")
            .and_then(|l| l.get("name").or_else(|| l.get("address")))
            .and_then(Value::as_str)
            .map(str::to_string);
        let app_link = event
            .get("app_link")
            .and_then(Value::as_str)
            .map(str::to_string);

        result.push(FeishuCalendarEvent {
            event_id,
            summary,
            start_date,
            end_date,
            is_all_day,
            description,
            location,
            app_link,
        });
    }

    Ok(result)
}

/// 去掉事件标题里的常见后缀,得到用于匹配的"干净"案件名片段。
fn clean_event_summary(summary: &str) -> String {
    summary
        .trim()
        .trim_end_matches("案件开庭")
        .trim_end_matches("开庭")
        .trim_end_matches("案件")
        .trim_end_matches("续封")
        .trim_end_matches("到期")
        .trim()
        .to_string()
}

/// 根据事件标题在飞书"案件池"多维表格里查找匹配的本地案件目录。
///
/// 匹配规则:事件标题包含案件名(如"张三案件开庭"匹配"张三"),或案件名与清理后的
/// 标题互相包含。返回第一个匹配且本地目录真实存在的记录路径。
///
/// 需要配置 `feishu_app_token` + `feishu_cases_table_id`(案件池表),且表里有
/// "案件名称""本地路径"两列;未配置则返回 None(不报错)。
pub async fn find_case_local_path(
    settings: &Settings,
    event_summary: &str,
) -> Result<Option<String>, String> {
    if !settings.feishu_enabled.unwrap_or(false) {
        return Ok(None);
    }

    let Some(app_token) = clean_required(settings.feishu_app_token.as_deref()) else {
        return Ok(None);
    };
    let Some(table_id) = clean_required(settings.feishu_cases_table_id.as_deref()) else {
        return Ok(None);
    };

    let bin = lark_bin(settings);
    let path = format!(
        "/open-apis/bitable/v1/apps/{}/tables/{}/records?page_size=500&field_names=true",
        app_token, table_id
    );
    let value = lark_cli_api(&bin, "GET", &path, None).await?;

    let items = value
        .pointer("/data/items")
        .and_then(Value::as_array)
        .ok_or_else(|| "飞书记录列表响应缺少 data.items".to_string())?;

    let clean_summary = clean_event_summary(event_summary);

    for item in items {
        let Some(fields) = item.get("fields") else {
            continue;
        };

        let case_name = fields.get("案件名称").and_then(Value::as_str).unwrap_or("");
        if case_name.is_empty() {
            continue;
        }

        let matches = event_summary.contains(case_name)
            || case_name.contains(&clean_summary)
            || clean_summary.contains(case_name);
        if !matches {
            continue;
        }

        let local_path = fields
            .get("本地路径")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty());

        if let Some(p) = local_path {
            if Path::new(p).exists() {
                return Ok(Some(p.to_string()));
            }
        }
    }

    Ok(None)
}
