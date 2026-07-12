//! 同一用户多设备的局域网工作区同步。
//!
//! 与团队同步严格隔离：这里使用独立 mDNS 域和独立密钥，同步个人工作空间的业务状态、
//! 派生 Markdown 和可移植设置。源文件只允许从副设备加密归集到唯一主力设备，绝不反向
//! 下发或在副设备间交换；任何对端绝对路径都不会进入协议。

pub mod net;
pub mod source;
pub mod store;
pub mod workspace;

use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceSyncIdentity {
    pub group_id: String,
    pub group_name: String,
    /// 64 hex；只存本机 settings.json，不通过普通设置表单覆写。
    pub group_secret: String,
    pub device_id: String,
    pub device_name: String,
    /// 设备组唯一主力设备。副设备源文件只允许加密上传到该设备，主力不向外分发源文件。
    #[serde(default)]
    pub primary_device_id: Option<String>,
    /// 高熵一次性配对口令；成功加入后创建方自动轮换。
    pub pairing_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeIdentity {
    pub group_id: String,
    pub group_name: String,
    pub device_id: String,
    pub device_name: String,
    pub primary_device_id: String,
    pub is_primary: bool,
    pub pairing_code: Option<String>,
}

impl From<&DeviceSyncIdentity> for SafeIdentity {
    fn from(value: &DeviceSyncIdentity) -> Self {
        Self {
            group_id: value.group_id.clone(),
            group_name: value.group_name.clone(),
            device_id: value.device_id.clone(),
            device_name: value.device_name.clone(),
            primary_device_id: value.primary_device_id().to_string(),
            is_primary: value.is_primary(),
            pairing_code: value.pairing_code.clone(),
        }
    }
}

impl DeviceSyncIdentity {
    /// 0041 期间创建的旧身份没有该字段；创建者安全迁移为本机主力。
    pub fn primary_device_id(&self) -> &str {
        self.primary_device_id
            .as_deref()
            .filter(|v| !v.is_empty())
            .unwrap_or(&self.device_id)
    }

    pub fn is_primary(&self) -> bool {
        self.primary_device_id() == self.device_id
    }
}

/// 会产生外部副作用的自动任务（飞书推送、滴答后台拉取）只由主力设备执行，避免同一
/// 个人空间的两三台电脑重复发消息或争抢远端状态。未配置个人空间时保持原有行为。
pub fn is_automation_owner() -> bool {
    crate::settings::read_settings()
        .ok()
        .and_then(|settings| settings.device_sync)
        .as_ref()
        .is_none_or(DeviceSyncIdentity::is_primary)
}

pub fn gen_secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// 128 bit 左右的高熵配对口令；分组只为跨 macOS / Windows 手输更清楚。
pub fn gen_pairing_code() -> String {
    let raw = uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .to_ascii_uppercase();
    raw.as_bytes()
        .chunks(4)
        .take(6)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

pub fn default_device_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "windows") {
                "Windows 电脑".into()
            } else if cfg!(target_os = "macos") {
                "Mac".into()
            } else {
                "我的电脑".into()
            }
        })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedEnvelope {
    pub group_id: String,
    pub device_id: String,
    pub nonce: String,
    pub ciphertext: String,
}

fn key_from_secret(secret: &str) -> [u8; 32] {
    Sha256::digest(secret.trim().as_bytes()).into()
}

fn aad(group_id: &str, device_id: &str) -> Vec<u8> {
    format!("caseboard-device-sync-v1\0{group_id}\0{device_id}").into_bytes()
}

pub fn encrypt<T: Serialize>(
    secret: &str,
    group_id: &str,
    device_id: &str,
    value: &T,
) -> Result<EncryptedEnvelope, String> {
    let plain = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let cipher = XChaCha20Poly1305::new((&key_from_secret(secret)).into());
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &plain,
                aad: &aad(group_id, device_id),
            },
        )
        .map_err(|_| "工作区同步加密失败".to_string())?;
    Ok(EncryptedEnvelope {
        group_id: group_id.to_string(),
        device_id: device_id.to_string(),
        nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
        ciphertext: base64::engine::general_purpose::STANDARD.encode(ciphertext),
    })
}

pub fn decrypt<T: DeserializeOwned>(secret: &str, env: &EncryptedEnvelope) -> Result<T, String> {
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(&env.nonce)
        .map_err(|_| "同步消息 nonce 无效".to_string())?;
    let nonce: [u8; 24] = nonce
        .try_into()
        .map_err(|_| "同步消息 nonce 长度无效".to_string())?;
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&env.ciphertext)
        .map_err(|_| "同步消息正文无效".to_string())?;
    let cipher = XChaCha20Poly1305::new((&key_from_secret(secret)).into());
    let plain = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: &aad(&env.group_id, &env.device_id),
            },
        )
        .map_err(|_| "同步消息认证失败（设备密钥不一致或数据被篡改）".to_string())?;
    serde_json::from_slice(&plain).map_err(|e| format!("同步消息解析失败：{e}"))
}
