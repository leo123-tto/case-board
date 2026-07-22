use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;
use tauri::Emitter;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

use super::pi_catalog::load_pi_catalog;
use super::pi_credentials::{OsPiCredentialVault, PiCredentialVault};
use super::pi_locator::resolve_pi_runtime_binary;
use super::pi_protocol::{PiHostMessage, PiSidecarMessage, PI_PROTOCOL_VERSION};

const AUTH_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

enum AuthInput {
    PromptResponse { prompt_id: String, value: String },
    Cancel,
}

struct ActiveAuthSession {
    id: String,
    sender: mpsc::UnboundedSender<AuthInput>,
    prompt_id: Arc<Mutex<Option<String>>>,
}

struct AuthProcessContext {
    binary: PathBuf,
    session_id: String,
    provider_id: String,
    auth_type: String,
    login_method: Option<String>,
    prompt_id: Arc<Mutex<Option<String>>>,
}

fn auth_registry() -> &'static Mutex<Option<ActiveAuthSession>> {
    static REGISTRY: OnceLock<Mutex<Option<ActiveAuthSession>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(None))
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PiProviderAuthEvent {
    Prompt {
        auth_session_id: String,
        prompt_id: String,
        prompt_type: String,
        message: String,
        placeholder: Option<String>,
        options: Option<Vec<super::pi_protocol::PiAuthOption>>,
    },
    Info {
        auth_session_id: String,
        message: String,
        links: Option<Vec<super::pi_protocol::PiAuthLink>>,
    },
    Url {
        auth_session_id: String,
        url: String,
        instructions: Option<String>,
        opened: bool,
    },
    DeviceCode {
        auth_session_id: String,
        user_code: String,
        verification_uri: String,
        interval_seconds: Option<u64>,
        expires_in_seconds: Option<u64>,
    },
    Progress {
        auth_session_id: String,
        message: String,
    },
    Success {
        auth_session_id: String,
        provider_id: String,
        credential_type: String,
    },
    Error {
        auth_session_id: String,
        message: String,
    },
    Cancelled {
        auth_session_id: String,
    },
}

pub async fn begin_pi_provider_auth(
    app: &tauri::AppHandle,
    provider_id: String,
    auth_type: String,
    login_method: Option<String>,
) -> Result<String, String> {
    if auth_type != "api_key" && auth_type != "oauth" {
        return Err("auth_type 只允许 api_key 或 oauth".into());
    }
    if let Some(method) = login_method.as_deref() {
        if auth_type != "oauth" || !matches!(method, "browser" | "device_code") {
            return Err("login_method 只允许 browser 或 device_code".into());
        }
    }
    let catalog = load_pi_catalog(Some(app))
        .await
        .map_err(|error| error.to_string())?;
    let provider = catalog
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| "provider 不在当前 Pi Runtime 目录".to_string())?;
    if !provider.auth_types.iter().any(|value| value == &auth_type) {
        return Err("该 provider 不支持所选认证方式".into());
    }
    let binary = resolve_pi_runtime_binary(Some(app))
        .map_err(|error| crate::feedback::sanitize_paths(&error.to_string()))?
        .binary;
    let auth_session_id = uuid::Uuid::new_v4().to_string();
    let (sender, receiver) = mpsc::unbounded_channel();
    let prompt_id = Arc::new(Mutex::new(None));
    {
        let mut registry = auth_registry().lock().map_err(|_| "认证状态不可用")?;
        if registry.is_some() {
            return Err("已有一个 Pi provider 认证正在进行".into());
        }
        *registry = Some(ActiveAuthSession {
            id: auth_session_id.clone(),
            sender,
            prompt_id: prompt_id.clone(),
        });
    }

    let app = app.clone();
    let session_id = auth_session_id.clone();
    tokio::spawn(async move {
        let context = AuthProcessContext {
            binary,
            session_id: session_id.clone(),
            provider_id,
            auth_type,
            login_method,
            prompt_id,
        };
        let result = run_auth_process(&app, context, receiver).await;
        if let Err(message) = result {
            emit(
                &app,
                PiProviderAuthEvent::Error {
                    auth_session_id: session_id.clone(),
                    message: crate::feedback::sanitize_paths(&message),
                },
            );
        }
        if let Ok(mut registry) = auth_registry().lock() {
            if registry
                .as_ref()
                .is_some_and(|session| session.id == session_id)
            {
                *registry = None;
            }
        }
    });
    Ok(auth_session_id)
}

pub fn respond_pi_provider_auth(
    auth_session_id: &str,
    prompt_id: &str,
    value: String,
) -> Result<(), String> {
    let registry = auth_registry().lock().map_err(|_| "认证状态不可用")?;
    let session = registry
        .as_ref()
        .filter(|session| session.id == auth_session_id)
        .ok_or_else(|| "认证会话不存在或已结束".to_string())?;
    let matches = session
        .prompt_id
        .lock()
        .map_err(|_| "认证状态不可用")?
        .as_deref()
        == Some(prompt_id);
    if !matches {
        return Err("prompt_id 与当前认证输入不匹配".into());
    }
    session
        .sender
        .send(AuthInput::PromptResponse {
            prompt_id: prompt_id.to_string(),
            value,
        })
        .map_err(|_| "认证会话已结束".to_string())
}

pub fn cancel_pi_provider_auth(auth_session_id: &str) -> Result<(), String> {
    let registry = auth_registry().lock().map_err(|_| "认证状态不可用")?;
    let session = registry
        .as_ref()
        .filter(|session| session.id == auth_session_id)
        .ok_or_else(|| "认证会话不存在或已结束".to_string())?;
    session
        .sender
        .send(AuthInput::Cancel)
        .map_err(|_| "认证会话已结束".to_string())
}

async fn run_auth_process(
    app: &tauri::AppHandle,
    context: AuthProcessContext,
    mut receiver: mpsc::UnboundedReceiver<AuthInput>,
) -> Result<(), String> {
    let mut command = Command::new(context.binary);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::proc_util::hide_console_window(&mut command);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let mut stdin = child.stdin.take().ok_or("Pi Sidecar stdin 不可用")?;
    let stdout = child.stdout.take().ok_or("Pi Sidecar stdout 不可用")?;
    let stderr_task = child.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            let mut buffer = [0_u8; 4096];
            while stderr.read(&mut buffer).await.unwrap_or(0) > 0 {}
        })
    });
    write_json_line(
        &mut stdin,
        &PiHostMessage::AuthStart {
            protocol_version: PI_PROTOCOL_VERSION,
            request_id: context.session_id.clone(),
            provider_id: context.provider_id.clone(),
            auth_type: context.auth_type.clone(),
        },
    )
    .await?;
    let mut stdout = BufReader::new(stdout);
    let deadline = tokio::time::sleep(AUTH_TIMEOUT);
    tokio::pin!(deadline);

    let completed = loop {
        tokio::select! {
            _ = &mut deadline => return Err("Pi provider 认证超时".into()),
            input = receiver.recv() => match input {
                Some(AuthInput::PromptResponse { prompt_id: response_id, value }) => {
                    write_json_line(&mut stdin, &PiHostMessage::AuthPromptResponse {
                        protocol_version: PI_PROTOCOL_VERSION,
                        request_id: context.session_id.clone(),
                        prompt_id: response_id,
                        value,
                    }).await?;
                    if let Ok(mut current) = context.prompt_id.lock() { *current = None; }
                }
                Some(AuthInput::Cancel) | None => {
                    write_json_line(&mut stdin, &PiHostMessage::AuthCancel {
                        protocol_version: PI_PROTOCOL_VERSION,
                        request_id: context.session_id.clone(),
                    }).await?;
                }
            },
            line = read_bounded_line(&mut stdout) => {
                let line = line.map_err(|error| error.to_string())?
                    .ok_or_else(|| "Pi Sidecar 在认证完成前退出".to_string())?;
                let message = parse_auth_sidecar_message(&line)?;
                message.validate_for_request(&context.session_id)?;
                match message {
                    PiSidecarMessage::AuthPrompt { prompt_id: next_id, prompt_type, message, placeholder, options, .. } => {
                        if let Some(method) = automatic_login_method(
                            &context.provider_id,
                            &context.auth_type,
                            context.login_method.as_deref(),
                            &prompt_type,
                            options.as_deref(),
                        ) {
                            write_json_line(&mut stdin, &PiHostMessage::AuthPromptResponse {
                                protocol_version: PI_PROTOCOL_VERSION,
                                request_id: context.session_id.clone(),
                                prompt_id: next_id,
                                value: method.to_string(),
                            }).await?;
                            continue;
                        }
                        if let Ok(mut current) = context.prompt_id.lock() { *current = Some(next_id.clone()); }
                        emit(app, PiProviderAuthEvent::Prompt {
                            auth_session_id: context.session_id.clone(), prompt_id: next_id, prompt_type,
                            message, placeholder, options,
                        });
                    }
                    PiSidecarMessage::AuthInfo { message, links, .. } => emit(app, PiProviderAuthEvent::Info {
                        auth_session_id: context.session_id.clone(), message, links,
                    }),
                    PiSidecarMessage::AuthUrl { url, instructions, .. } => {
                        let opened = maybe_open_codex_auth_url(&context.provider_id, &url);
                        emit(app, PiProviderAuthEvent::Url {
                            auth_session_id: context.session_id.clone(), url, instructions, opened,
                        });
                    }
                    PiSidecarMessage::AuthDeviceCode { user_code, verification_uri, interval_seconds, expires_in_seconds, .. } => {
                        emit(app, PiProviderAuthEvent::DeviceCode {
                            auth_session_id: context.session_id.clone(), user_code, verification_uri,
                            interval_seconds, expires_in_seconds,
                        });
                    }
                    PiSidecarMessage::AuthProgress { message, .. } => emit(app, PiProviderAuthEvent::Progress {
                        auth_session_id: context.session_id.clone(), message,
                    }),
                    PiSidecarMessage::AuthSuccess { provider_id: returned_provider, credential, .. } => {
                        if returned_provider != context.provider_id { return Err("认证结果 provider_id 不匹配".into()); }
                        let credential_type = credential.credential_type().to_string();
                        OsPiCredentialVault.write(&context.provider_id, &credential)?;
                        emit(app, PiProviderAuthEvent::Success {
                            auth_session_id: context.session_id.clone(),
                            provider_id: context.provider_id.clone(),
                            credential_type,
                        });
                        break true;
                    }
                    PiSidecarMessage::AuthError { message, .. } => return Err(message),
                    PiSidecarMessage::AuthCancelled { .. } => {
                        emit(app, PiProviderAuthEvent::Cancelled { auth_session_id: context.session_id.clone() });
                        break false;
                    }
                    _ => return Err("Pi Sidecar 认证期间返回了不支持的消息".into()),
                }
            }
        }
    };
    drop(stdin);
    finish_child(&mut child, completed).await;
    if let Some(task) = stderr_task {
        task.abort();
    }
    Ok(())
}

fn maybe_open_codex_auth_url(provider_id: &str, url: &str) -> bool {
    if !codex_auth_url_allowed(provider_id, url) {
        return false;
    }
    tauri_plugin_opener::open_url(url, None::<&str>).is_ok()
}

fn parse_auth_sidecar_message(line: &str) -> Result<PiSidecarMessage, String> {
    let value: serde_json::Value =
        serde_json::from_str(line).map_err(|_| "Pi Sidecar 认证响应不是有效 JSON".to_string())?;
    serde_json::from_value(value).map_err(|error| format!("Pi Sidecar 认证协议字段不兼容: {error}"))
}

fn codex_auth_url_allowed(provider_id: &str, url: &str) -> bool {
    if provider_id != "openai-codex" {
        return false;
    }
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    parsed.scheme() == "https" && parsed.host_str() == Some("auth.openai.com")
}

fn automatic_login_method<'a>(
    provider_id: &str,
    auth_type: &str,
    login_method: Option<&'a str>,
    prompt_type: &str,
    options: Option<&[super::pi_protocol::PiAuthOption]>,
) -> Option<&'a str> {
    if provider_id != "openai-codex" || auth_type != "oauth" || prompt_type != "select" {
        return None;
    }
    let method = login_method?;
    options?
        .iter()
        .any(|option| option.id == method)
        .then_some(method)
}

fn emit(app: &tauri::AppHandle, event: PiProviderAuthEvent) {
    let _ = app.emit("pi-provider-auth-event", event);
}

async fn write_json_line(stdin: &mut ChildStdin, message: &PiHostMessage) -> Result<(), String> {
    let mut encoded = serde_json::to_vec(message).map_err(|error| error.to_string())?;
    encoded.push(b'\n');
    stdin
        .write_all(&encoded)
        .await
        .map_err(|error| error.to_string())?;
    stdin.flush().await.map_err(|error| error.to_string())
}

async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
) -> std::io::Result<Option<String>> {
    let mut line = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(None);
            }
            break;
        }
        if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
            if line.len().saturating_add(position) > MAX_LINE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "消息过大",
                ));
            }
            line.extend_from_slice(&available[..position]);
            reader.consume(position + 1);
            break;
        }
        if line.len().saturating_add(available.len()) > MAX_LINE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "消息过大",
            ));
        }
        let length = available.len();
        line.extend_from_slice(available);
        reader.consume(length);
    }
    String::from_utf8(line)
        .map(Some)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "非 UTF-8 输出"))
}

async fn finish_child(child: &mut Child, _completed: bool) {
    if tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}
