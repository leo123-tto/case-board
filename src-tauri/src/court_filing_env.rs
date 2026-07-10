//! 在线立案运行环境:检测 + 一键安装(流式进度)。
//!
//! 在线立案依赖一套本机 Python 运行时(playwright / ddddocr / httpx + Chromium 内核),
//! 不打包进安装包。本模块把"检测能不能跑 + 缺了一键装"做进 App:
//!
//! - 检测:跑 `python -m court_filing_cli.env_check --json`,拿到组件清单。
//! - 安装:在 `<app_data>/court_filing_venv` 建一个独立 venv,pip 装依赖 + 下载 Chromium,
//!   全程把子进程输出按行 emit 到前端事件 `court-filing-env-progress`,装完把
//!   `settings.court_filing_python` 指到这个 venv,检测/立案都自动用它。
//!
//! 跨平台(CLAUDE.md 已知坑 #21):venv 落 app_data(可写,而非只读的 Resource 目录);
//! 解释器名按平台取(Windows `python`/`py -3`,macOS/Linux `python3`);pip / playwright
//! 走国内镜像加速(律师用户多在国内,默认 PyPI / Azure CDN 慢甚至连不上)。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

use crate::proc_util::hide_console_window;

/// venv 目录名(落在 app_data 下)。
const VENV_DIR: &str = "court_filing_venv";
/// pip 主镜像(清华 TUNA,全量 PyPI 镜像,稳定;2026-06-18 实测可达、装 playwright 1.60 正常)。
const PIP_INDEX: &str = "https://pypi.tuna.tsinghua.edu.cn/simple";
/// pip 兜底源(清华挂了/缺包时回落官方 PyPI,不至于整体失败)。
const PIP_EXTRA_INDEX: &str = "https://pypi.org/simple";
// ⚠️ 不给 Chromium 设 PLAYWRIGHT_DOWNLOAD_HOST 镜像:2026-06-18 实测 npmmirror **没有**
// playwright 1.60 的新 "Chrome for Testing"(/builds/cft/...)布局包(mac/win 均 404),
// 且一旦设了下载主机,playwright 官方自带的 fallback 源就不再生效 → 反而下载必败。
// 默认源 cdn.playwright.dev(带微软 prss fallback)是原 PR 作者 + Windows 反馈人跑通的已验证路径。

/// 单个组件的体检结果(对齐 env_check.py 的 JSON)。
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct EnvComponent {
    pub name: String,
    #[serde(default)]
    pub id: String,
    pub version: String,
    pub ok: bool,
}

/// 整体体检报告。
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct EnvReport {
    pub ok: bool,
    pub components: Vec<EnvComponent>,
    pub missing: Vec<String>,
    /// 是否检测到可用的 Python 解释器(false = 连 Python 都没有,需先装 Python)。
    #[serde(default)]
    pub python_found: bool,
    /// 检测/安装层面的错误(非组件缺失,而是连体检都没跑起来)。
    #[serde(default)]
    pub error: Option<String>,
}

/// 安装进度事件(emit 到 `court-filing-env-progress`)。
#[derive(Clone, Serialize)]
struct EnvInstallEvent {
    /// 步骤标识:`python` / `venv` / `deps` / `chromium` / `verify`。
    step: String,
    label: String,
    /// `running` / `done` / `error`。
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    /// 子进程实时输出的一行(供前端滚动展示)。
    #[serde(skip_serializing_if = "Option::is_none")]
    log: Option<String>,
}

/// venv 内 python 可执行文件路径。
pub fn venv_python_path() -> Option<PathBuf> {
    let base = crate::db::app_data_dir().ok()?.join(VENV_DIR);
    let exe = if cfg!(windows) {
        base.join("Scripts").join("python.exe")
    } else {
        base.join("bin").join("python")
    };
    Some(exe)
}

/// 平台默认解释器名。
fn default_python() -> String {
    if cfg!(windows) {
        "python".to_string()
    } else {
        "python3".to_string()
    }
}

/// 解析"立案实际使用的 python":用户配置 > 我们装的 venv > 平台默认。
///
/// `start_court_filing` 与环境检测都走这里,保证装完 venv 后自动生效、口径一致。
pub fn resolve_python(configured: Option<&str>) -> String {
    if let Some(c) = configured {
        let c = c.trim();
        if !c.is_empty() {
            return shellexpand::tilde(c).into_owned();
        }
    }
    if let Some(v) = venv_python_path() {
        if v.exists() {
            return v.to_string_lossy().to_string();
        }
    }
    default_python()
}

fn emit(
    app: &AppHandle,
    step: &str,
    label: &str,
    status: &str,
    detail: Option<String>,
    log: Option<String>,
) {
    let _ = app.emit(
        "court-filing-env-progress",
        EnvInstallEvent {
            step: step.to_string(),
            label: label.to_string(),
            status: status.to_string(),
            detail,
            log,
        },
    );
}

/// 跑一条命令,把 stdout+stderr 按行 emit 成 `running` 日志;失败返回末尾输出。
async fn run_streamed(
    app: &AppHandle,
    step: &str,
    label: &str,
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
    envs: &[(&str, &str)],
) -> Result<(), String> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    // 统一强制 UTF-8:子进程都是 Python,Windows 默认 CP936 会让中文输出/日志变乱码、
    // 进而被 JSON 解析当"无有效输出"(坑:env_check 输出中文组件名)。
    cmd.env("PYTHONUTF8", "1").env("PYTHONIOENCODING", "utf-8");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    hide_console_window(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("启动失败({program}): {e}"))?;

    let tail: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        readers.push(spawn_line_reader(
            app.clone(),
            step.to_string(),
            label.to_string(),
            tail.clone(),
            Box::new(out),
        ));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(spawn_line_reader(
            app.clone(),
            step.to_string(),
            label.to_string(),
            tail.clone(),
            Box::new(err),
        ));
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("等待进程失败: {e}"))?;
    for r in readers {
        let _ = r.await;
    }

    if status.success() {
        Ok(())
    } else {
        let tail = tail.lock().map(|t| t.join("\n")).unwrap_or_default();
        Err(if tail.is_empty() {
            format!("退出码 {:?}", status.code())
        } else {
            tail
        })
    }
}

fn spawn_line_reader(
    app: AppHandle,
    step: String,
    label: String,
    tail: Arc<Mutex<Vec<String>>>,
    reader: Box<dyn tokio::io::AsyncRead + Unpin + Send>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(mut t) = tail.lock() {
                t.push(trimmed.to_string());
                if t.len() > 30 {
                    t.remove(0);
                }
            }
            emit(
                &app,
                &step,
                &label,
                "running",
                None,
                Some(trimmed.to_string()),
            );
        }
    })
}

/// 找一个能用的"基础 Python"(用来建 venv)。返回 `(program, 前置参数)`。
async fn find_base_python(configured: Option<&str>) -> Option<(String, Vec<String>)> {
    let mut candidates: Vec<(String, Vec<String>)> = Vec::new();
    if let Some(c) = configured {
        let c = c.trim();
        // 配置值若就是我们的 venv python,跳过(建 venv 要用系统 python)。
        let is_our_venv = venv_python_path()
            .map(|v| v.to_string_lossy() == c)
            .unwrap_or(false);
        if !c.is_empty() && !is_our_venv {
            candidates.push((c.to_string(), vec![]));
        }
    }
    if cfg!(windows) {
        candidates.push(("py".to_string(), vec!["-3".to_string()]));
        candidates.push(("python".to_string(), vec![]));
        candidates.push(("python3".to_string(), vec![]));
    } else {
        candidates.push(("python3".to_string(), vec![]));
        candidates.push(("python".to_string(), vec![]));
    }

    for (prog, pre) in candidates {
        // 用 `-c` 打印哨兵 + 版本号,一举两得:
        // ① 挡 Windows「应用商店占位 python.exe」假壳(退出码 0 但无输出、还弹商店);
        // ② 校验 >=3.11(CLI 的 pyproject 明确要求;3.9/3.10 装到后面才炸)。
        let mut cmd = Command::new(&prog);
        cmd.args(&pre)
            .arg("-c")
            .arg("import sys;print('PYOK',sys.version_info[0],sys.version_info[1])")
            .env("PYTHONUTF8", "1")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        hide_console_window(&mut cmd);
        if let Ok(out) = cmd.output().await {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                let parts: Vec<&str> = s.split_whitespace().collect();
                if let ["PYOK", maj, min] = parts.as_slice() {
                    let major: u32 = maj.parse().unwrap_or(0);
                    let minor: u32 = min.parse().unwrap_or(0);
                    if major == 3 && minor >= 11 {
                        return Some((prog, pre));
                    }
                }
            }
        }
    }
    None
}

/// 跑环境体检,返回结构化报告(不依赖前端)。
pub async fn run_check(python: &str, cli_parent: &Path) -> EnvReport {
    let mut cmd = Command::new(python);
    cmd.current_dir(cli_parent)
        .args(["-m", "court_filing_cli.env_check", "--json"])
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console_window(&mut cmd);
    let output = cmd.output().await;

    let out = match output {
        Ok(o) => o,
        Err(e) => {
            // 进程都拉不起来 = 真的没 Python(或路径错)→ python_found=false,引导去装 Python。
            return EnvReport {
                ok: false,
                python_found: false,
                missing: vec!["python".to_string()],
                error: Some(format!("未检测到可用的 Python({python}): {e}")),
                ..Default::default()
            };
        }
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    // env_check 只往 stdout 写一行 JSON;取最后一个非空行解析。
    let json_line = stdout.lines().rev().find(|l| l.trim().starts_with('{'));
    match json_line.and_then(|l| serde_json::from_str::<EnvReport>(l).ok()) {
        Some(mut r) => {
            r.python_found = true;
            r
        }
        // 进程跑起来了但没拿到有效 JSON(import 失败 / 编码乱码 / 资源路径错等)——
        // **Python 是有的**,只是体检没跑通。别误报成"没装 Python"(否则前端会藏掉安装按钮、
        // 错误引导去下载 Python)。python_found=true、不把 python 列进 missing。
        None => EnvReport {
            ok: false,
            python_found: true,
            missing: vec![],
            error: Some(format!(
                "环境体检脚本未返回有效结果(Python 在,但体检没跑通):{}",
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            ..Default::default()
        },
    }
}

/// 一键安装:建 venv → pip 装依赖 → 下载 Chromium → 体检 → 把 venv 写回设置。
///
/// 全程 emit `court-filing-env-progress` 事件。任一步骤失败提前返回 Err(并已 emit error)。
pub async fn run_install(
    app: &AppHandle,
    cli_parent: &Path,
    configured_python: Option<String>,
) -> Result<EnvReport, String> {
    // ── 步骤 1:基础 Python ──
    emit(app, "python", "检测 Python 运行时", "running", None, None);
    let (base_prog, base_pre) = match find_base_python(configured_python.as_deref()).await {
        Some(p) => p,
        None => {
            let msg = "未检测到 Python。请先到 python.org 下载安装 Python 3.11+(Windows 安装时务必勾选 “Add Python to PATH”),装完重启 App 再点检测。".to_string();
            emit(
                app,
                "python",
                "检测 Python 运行时",
                "error",
                Some(msg.clone()),
                None,
            );
            return Err(msg);
        }
    };
    emit(
        app,
        "python",
        "检测 Python 运行时",
        "done",
        Some(format!("使用 {base_prog}")),
        None,
    );

    // ── 步骤 2:创建独立运行环境(venv) ──
    let venv_py = venv_python_path().ok_or_else(|| "无法定位数据目录".to_string())?;
    let venv_dir = venv_py
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_default();
    if venv_py.exists() {
        emit(
            app,
            "venv",
            "创建独立运行环境",
            "done",
            Some("已存在,复用".into()),
            None,
        );
    } else {
        emit(app, "venv", "创建独立运行环境", "running", None, None);
        let mut venv_args = base_pre.clone();
        venv_args.extend([
            "-m".into(),
            "venv".into(),
            venv_dir.to_string_lossy().to_string(),
        ]);
        run_streamed(
            app,
            "venv",
            "创建独立运行环境",
            &base_prog,
            &venv_args,
            None,
            &[],
        )
        .await
        .map_err(|e| {
            emit(
                app,
                "venv",
                "创建独立运行环境",
                "error",
                Some(e.clone()),
                None,
            );
            format!("创建 venv 失败: {e}")
        })?;
        emit(app, "venv", "创建独立运行环境", "done", None, None);
    }
    let venv_py_str = venv_py.to_string_lossy().to_string();

    // ── 步骤 3:pip 装依赖(playwright / ddddocr / httpx + 间接依赖) ──
    emit(
        app,
        "deps",
        "安装依赖库",
        "running",
        Some("playwright / ddddocr / httpx(清华镜像 + 官方源兜底)".into()),
        None,
    );
    // 不在这里 `--upgrade pip`:Windows 上 pip 自升级会因 pip.exe 被占用而失败;
    // 新建的 venv 自带的 pip 足够新。只装这三个包(间接依赖 numpy/onnxruntime/opencv 自动带)。
    let deps_args: Vec<String> = [
        "-m",
        "pip",
        "install",
        "--upgrade",
        "--disable-pip-version-check",
        "-i",
        PIP_INDEX,
        "--extra-index-url",
        PIP_EXTRA_INDEX,
        "playwright",
        "ddddocr",
        "httpx",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    run_streamed(
        app,
        "deps",
        "安装依赖库",
        &venv_py_str,
        &deps_args,
        None,
        &[],
    )
    .await
    .map_err(|e| {
        emit(app, "deps", "安装依赖库", "error", Some(e.clone()), None);
        format!("安装依赖失败: {e}")
    })?;
    emit(app, "deps", "安装依赖库", "done", None, None);

    // ── 步骤 4:下载 Chromium 内核 ──
    emit(
        app,
        "chromium",
        "下载 Chromium 浏览器内核(约 130MB)",
        "running",
        None,
        None,
    );
    let chromium_args: Vec<String> = ["-m", "playwright", "install", "chromium"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    run_streamed(
        app,
        "chromium",
        "下载 Chromium 浏览器内核(约 130MB)",
        &venv_py_str,
        &chromium_args,
        None,
        &[], // 走 playwright 默认源(已验证 + 带 fallback);不设镜像主机,理由见 PLAYWRIGHT 注释
    )
    .await
    .map_err(|e| {
        emit(
            app,
            "chromium",
            "下载 Chromium 浏览器内核(约 130MB)",
            "error",
            Some(e.clone()),
            None,
        );
        format!("下载 Chromium 失败: {e}")
    })?;
    emit(
        app,
        "chromium",
        "下载 Chromium 浏览器内核(约 130MB)",
        "done",
        None,
        None,
    );

    // ── 步骤 5:体检 + 写回设置 ──
    emit(app, "verify", "校验所有组件", "running", None, None);
    let report = run_check(&venv_py_str, cli_parent).await;
    if report.ok {
        // 把立案解释器指到这个 venv,检测/立案都自动用它。
        if let Ok(mut s) = crate::settings::read_settings() {
            s.court_filing_python = Some(venv_py_str.clone());
            let _ = crate::settings::write_settings(&s);
        }
        emit(
            app,
            "verify",
            "校验所有组件",
            "done",
            Some("环境已就绪".into()),
            None,
        );
    } else {
        emit(
            app,
            "verify",
            "校验所有组件",
            "error",
            Some(format!("仍缺少: {}", report.missing.join(", "))),
            None,
        );
    }
    Ok(report)
}
