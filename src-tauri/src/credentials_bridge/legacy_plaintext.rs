use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use zeroize::{Zeroize, Zeroizing};

use super::migration_journal::PendingCredentialDescriptor;
use super::platform_permissions::SecureAtomicFile;
use super::types::{
    BridgeError, BridgeResult, CredentialKind, CredentialOwnerScope, PendingMigrationSourceError,
};

pub(crate) struct LegacyCredentialCandidate {
    pub descriptor: PendingCredentialDescriptor,
    pub secret: Zeroizing<Vec<u8>>,
}

pub(crate) struct LegacyPlaintextSnapshot {
    pub candidates: Vec<LegacyCredentialCandidate>,
    pub mcp_instance_ids_added: usize,
    pub source_errors: Vec<PendingMigrationSourceError>,
}

pub(crate) struct PreparedSanitizedSettings {
    pub candidates: Vec<LegacyCredentialCandidate>,
    pub bytes: Zeroizing<Vec<u8>>,
    pub sha256: String,
    source_sha256: String,
    path: PathBuf,
}

pub(crate) struct SanitizedSettingsTemp {
    temp: SecureAtomicFile,
    path: PathBuf,
    expected_source_sha256: String,
}

struct ZeroizingJsonValue(Value);

impl Deref for ZeroizingJsonValue {
    type Target = Value;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ZeroizingJsonValue {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for ZeroizingJsonValue {
    fn drop(&mut self) {
        zeroize_json_strings(&mut self.0);
    }
}

fn zeroize_json_strings(value: &mut Value) {
    match value {
        Value::String(text) => text.zeroize(),
        Value::Array(items) => items.iter_mut().for_each(zeroize_json_strings),
        Value::Object(object) => object.values_mut().for_each(zeroize_json_strings),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

pub(crate) fn read_legacy_plaintext(app_data_root: &Path) -> LegacyPlaintextSnapshot {
    let settings_path = app_data_root.join("settings.json");
    let mut candidates = Vec::new();
    let mut source_errors = Vec::new();
    let mut mcp_instance_ids_added = 0;

    let mut settings = ZeroizingJsonValue(match read_json_value(&settings_path) {
        Ok(value) => value.unwrap_or_else(|| Value::Object(Map::new())),
        Err(error) => {
            push_source_error(&mut source_errors, "settings.json", error);
            Value::Null
        }
    });
    if !settings.is_null() {
        let mut prepared_mcp = prepare_mcp_instance_ids(&mut settings, &mut source_errors);
        if prepared_mcp.added > 0 {
            if prepared_mcp.has_errors {
                for index in &prepared_mcp.generated_indexes {
                    prepared_mcp.eligible_indexes.remove(index);
                }
            } else {
                match atomic_write_json(&settings_path, &settings) {
                    Ok(()) => mcp_instance_ids_added = prepared_mcp.added,
                    Err(error) => {
                        push_source_error(&mut source_errors, "settings.json:mcp_servers", error);
                        for index in &prepared_mcp.generated_indexes {
                            prepared_mcp.eligible_indexes.remove(index);
                        }
                    }
                }
            }
        }
        collect_settings_scalars(&mut settings, &mut candidates, &mut source_errors);
        collect_nested_settings(&mut settings, &mut candidates, &mut source_errors);
        collect_mcp_bundles(
            &mut settings,
            &prepared_mcp.eligible_indexes,
            &mut candidates,
            &mut source_errors,
        );
        collect_court_cookie(&settings, &mut candidates, &mut source_errors);
    }
    collect_ticktick(app_data_root, &mut candidates, &mut source_errors);

    LegacyPlaintextSnapshot {
        candidates,
        mcp_instance_ids_added,
        source_errors,
    }
}

pub(crate) fn prepare_sanitized_settings(
    app_data_root: &Path,
) -> BridgeResult<PreparedSanitizedSettings> {
    let path = app_data_root.join("settings.json");
    let raw = if verify_legacy_regular_file(&path)? {
        Zeroizing::new(fs::read(&path).map_err(|source| BridgeError::Io {
            operation: "read settings for sanitization",
            path: path.clone(),
            source,
        })?)
    } else {
        Zeroizing::new(b"{}\n".to_vec())
    };
    let mut settings = ZeroizingJsonValue(if raw.iter().all(u8::is_ascii_whitespace) {
        Value::Object(Map::new())
    } else {
        serde_json::from_slice(&raw)
            .map_err(|_| BridgeError::InvalidInput("legacy credential JSON"))?
    });
    if !settings.is_object() {
        return Err(BridgeError::InvalidInput("settings root object"));
    }

    let mut source_errors = Vec::new();
    let prepared_mcp = prepare_mcp_instance_ids(&mut settings, &mut source_errors);
    if prepared_mcp.added > 0 {
        return Err(BridgeError::CorruptMetadata(
            "Task 3B requires Task 3A to persist MCP instance_id first".to_owned(),
        ));
    }
    let mut candidates = Vec::new();
    collect_task3b_settings_scalars(&mut settings, &mut candidates, &mut source_errors);
    collect_mcp_bundles(
        &mut settings,
        &prepared_mcp.eligible_indexes,
        &mut candidates,
        &mut source_errors,
    );
    if let Some(error) = source_errors.first() {
        return Err(BridgeError::CorruptMetadata(format!(
            "sanitization source invalid: {}: {}",
            error.source, error.error
        )));
    }

    let source_sha256 = sha256_hex(&raw);
    let bytes = Zeroizing::new(sanitize_settings_bytes_preserving_other_fields(&raw)?);
    let sha256 = { sha256_hex(&bytes) };
    Ok(PreparedSanitizedSettings {
        candidates,
        bytes,
        sha256,
        source_sha256,
        path,
    })
}

impl PreparedSanitizedSettings {
    pub(crate) fn write_temp(&self) -> BridgeResult<SanitizedSettingsTemp> {
        let parent = self
            .path
            .parent()
            .ok_or(BridgeError::InvalidInput("settings parent"))?
            .to_path_buf();
        fs::create_dir_all(&parent).map_err(|source| BridgeError::Io {
            operation: "create settings directory",
            path: parent.clone(),
            source,
        })?;
        let mut temp = SecureAtomicFile::new(&self.path)?;
        temp.as_file_mut()
            .write_all(&self.bytes)
            .map_err(|source| BridgeError::Io {
                operation: "write sanitized settings candidate",
                path: self.path.clone(),
                source,
            })?;
        Ok(SanitizedSettingsTemp {
            temp,
            path: self.path.clone(),
            expected_source_sha256: self.source_sha256.clone(),
        })
    }
}

impl SanitizedSettingsTemp {
    pub(crate) fn sync(&mut self) -> BridgeResult<()> {
        self.temp.sync()
    }

    pub(crate) fn persist(self) -> BridgeResult<()> {
        let current = if verify_legacy_regular_file(&self.path)? {
            Zeroizing::new(fs::read(&self.path).map_err(|source| BridgeError::Io {
                operation: "re-read settings before sanitized rename",
                path: self.path.clone(),
                source,
            })?)
        } else {
            Zeroizing::new(b"{}\n".to_vec())
        };
        if sha256_hex(&current) != self.expected_source_sha256 {
            return Err(BridgeError::CorruptMetadata(
                "settings changed during sanitization; retry without overwriting".to_owned(),
            ));
        }
        self.temp.persist()
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn read_json_value(path: &Path) -> BridgeResult<Option<Value>> {
    if !verify_legacy_regular_file(path)? {
        return Ok(None);
    }
    let bytes = Zeroizing::new(fs::read(path).map_err(|source| BridgeError::Io {
        operation: "read legacy credential source",
        path: path.to_path_buf(),
        source,
    })?);
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Some(Value::Object(Map::new())));
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| BridgeError::InvalidInput("legacy credential JSON"))
}

struct PreparedMcpSources {
    eligible_indexes: HashSet<usize>,
    generated_indexes: HashSet<usize>,
    added: usize,
    has_errors: bool,
}

fn prepare_mcp_instance_ids(
    settings: &mut Value,
    source_errors: &mut Vec<PendingMigrationSourceError>,
) -> PreparedMcpSources {
    let Some(servers_value) = settings.get_mut("mcp_servers") else {
        return PreparedMcpSources {
            eligible_indexes: HashSet::new(),
            generated_indexes: HashSet::new(),
            added: 0,
            has_errors: false,
        };
    };
    let Some(servers) = servers_value.as_array_mut() else {
        push_source_error(
            source_errors,
            "settings.json:mcp_servers",
            BridgeError::InvalidMcpInstanceId("mcp_servers 不是数组".to_owned()),
        );
        return PreparedMcpSources {
            eligible_indexes: HashSet::new(),
            generated_indexes: HashSet::new(),
            added: 0,
            has_errors: true,
        };
    };
    let initial_error_count = source_errors.len();
    let mut indexes_by_id = BTreeMap::<String, Vec<usize>>::new();
    let mut eligible_indexes = HashSet::new();
    let mut generated_indexes = HashSet::new();
    let mut added = 0;
    for (index, server) in servers.iter_mut().enumerate() {
        let source = format!("settings.json:mcp_servers[{index}]");
        let Some(object) = server.as_object_mut() else {
            push_source_error(
                source_errors,
                source,
                BridgeError::InvalidMcpInstanceId(format!("mcp_servers[{index}] 不是对象")),
            );
            continue;
        };
        if let Err(error) = validate_mcp_shape(object, index) {
            push_source_error(source_errors, source, error);
            continue;
        }
        let instance_id = match object.get("instance_id") {
            None => {
                let generated = uuid::Uuid::new_v4().to_string();
                object.insert("instance_id".to_owned(), Value::String(generated.clone()));
                added += 1;
                generated_indexes.insert(index);
                generated
            }
            Some(Value::String(value)) => value.clone(),
            Some(_) => {
                push_source_error(
                    source_errors,
                    source,
                    BridgeError::InvalidMcpInstanceId(format!(
                        "mcp_servers[{index}].instance_id 不是字符串"
                    )),
                );
                continue;
            }
        };
        if !is_uuid_v4(&instance_id) {
            push_source_error(
                source_errors,
                source,
                BridgeError::InvalidMcpInstanceId(format!(
                    "mcp_servers[{index}].instance_id 不是 UUID v4"
                )),
            );
            continue;
        }
        eligible_indexes.insert(index);
        indexes_by_id.entry(instance_id).or_default().push(index);
    }
    for indexes in indexes_by_id.values().filter(|indexes| indexes.len() > 1) {
        for index in indexes {
            eligible_indexes.remove(index);
            push_source_error(
                source_errors,
                format!("settings.json:mcp_servers[{index}]"),
                BridgeError::InvalidMcpInstanceId("mcp_servers.instance_id 重复".to_owned()),
            );
        }
    }
    PreparedMcpSources {
        eligible_indexes,
        generated_indexes,
        added,
        has_errors: source_errors.len() > initial_error_count,
    }
}

fn is_uuid_v4(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.get_version() == Some(uuid::Version::Random))
}

fn validate_mcp_shape(object: &Map<String, Value>, index: usize) -> BridgeResult<()> {
    if !object.get("name").is_some_and(Value::is_string)
        || !object.get("transport").is_some_and(Value::is_object)
    {
        return Err(BridgeError::InvalidMcpInstanceId(format!(
            "mcp_servers[{index}] 配置形状无效"
        )));
    }
    let transport = object["transport"]
        .as_object()
        .ok_or(BridgeError::InvalidInput("MCP transport"))?;
    match transport.get("type").and_then(Value::as_str) {
        Some("stdio") if transport.get("command").is_some_and(Value::is_string) => Ok(()),
        Some("http") if transport.get("url").is_some_and(Value::is_string) => Ok(()),
        _ => Err(BridgeError::InvalidMcpInstanceId(format!(
            "mcp_servers[{index}].transport 配置无效"
        ))),
    }
}

fn collect_settings_scalars(
    settings: &mut Value,
    candidates: &mut Vec<LegacyCredentialCandidate>,
    source_errors: &mut Vec<PendingMigrationSourceError>,
) {
    let fields = [
        ("mineru_api_key", "ocr.mineru", CredentialKind::ApiKey),
        ("paddle_vl_api_key", "ocr.paddle_vl", CredentialKind::ApiKey),
        ("cloud_llm_api_key", "llm.deepseek", CredentialKind::ApiKey),
        ("minimax_api_key", "llm.minimax", CredentialKind::ApiKey),
        ("compat_llm_api_key", "llm.compat", CredentialKind::ApiKey),
        ("glm_llm_api_key", "llm.glm", CredentialKind::ApiKey),
        ("mimo_llm_api_key", "llm.mimo", CredentialKind::ApiKey),
        ("kimi_llm_api_key", "llm.kimi", CredentialKind::ApiKey),
        ("custom_llm_api_key", "llm.custom", CredentialKind::ApiKey),
        (
            "yuandian_api_key",
            "connector.yuandian",
            CredentialKind::ApiKey,
        ),
        (
            "kuaidi100_key",
            "connector.kuaidi100",
            CredentialKind::ApiKey,
        ),
        ("embedding_api_key", "embedding", CredentialKind::ApiKey),
        (
            "feishu_app_token",
            "feishu.calendar",
            CredentialKind::AppToken,
        ),
        (
            "feishu_webhook_url",
            "feishu.reminder",
            CredentialKind::WebhookSecret,
        ),
        (
            "court_filing_password",
            "court_filing",
            CredentialKind::Password,
        ),
    ];
    for (field, provider, kind) in fields {
        let source = format!("settings:{field}");
        match take_secret_string(settings.get_mut(field)) {
            Ok(Some(secret)) => candidates.push(candidate(
                source,
                provider,
                kind,
                CredentialOwnerScope::Global,
                secret,
            )),
            Ok(None) => {}
            Err(error) => push_source_error(source_errors, source, error),
        }
    }
}

fn collect_task3b_settings_scalars(
    settings: &mut Value,
    candidates: &mut Vec<LegacyCredentialCandidate>,
    source_errors: &mut Vec<PendingMigrationSourceError>,
) {
    let fields = [
        ("mineru_api_key", "ocr.mineru", CredentialKind::ApiKey),
        ("paddle_vl_api_key", "ocr.paddle_vl", CredentialKind::ApiKey),
        ("cloud_llm_api_key", "llm.deepseek", CredentialKind::ApiKey),
        ("minimax_api_key", "llm.minimax", CredentialKind::ApiKey),
        ("compat_llm_api_key", "llm.compat", CredentialKind::ApiKey),
        ("glm_llm_api_key", "llm.glm", CredentialKind::ApiKey),
        ("mimo_llm_api_key", "llm.mimo", CredentialKind::ApiKey),
        ("kimi_llm_api_key", "llm.kimi", CredentialKind::ApiKey),
        ("custom_llm_api_key", "llm.custom", CredentialKind::ApiKey),
        (
            "yuandian_api_key",
            "connector.yuandian",
            CredentialKind::ApiKey,
        ),
        (
            "kuaidi100_key",
            "connector.kuaidi100",
            CredentialKind::ApiKey,
        ),
        ("embedding_api_key", "embedding", CredentialKind::ApiKey),
        (
            "feishu_webhook_url",
            "feishu.reminder",
            CredentialKind::WebhookSecret,
        ),
    ];
    for (field, provider, kind) in fields {
        let source = format!("settings:{field}");
        match take_secret_string(settings.get_mut(field)) {
            Ok(Some(secret)) => candidates.push(candidate(
                source,
                provider,
                kind,
                CredentialOwnerScope::Global,
                secret,
            )),
            Ok(None) => {}
            Err(error) => push_source_error(source_errors, source, error),
        }
    }
}

fn collect_nested_settings(
    settings: &mut Value,
    candidates: &mut Vec<LegacyCredentialCandidate>,
    source_errors: &mut Vec<PendingMigrationSourceError>,
) {
    if let Some(team) = settings.get_mut("team").and_then(Value::as_object_mut) {
        match required_non_secret_id(team, "team_id", "team") {
            Ok(team_id) => {
                for (field, kind) in [
                    ("team_secret", CredentialKind::SyncKey),
                    ("pairing_code", CredentialKind::Password),
                ] {
                    let source = format!("settings:team:{team_id}:{field}");
                    match take_secret_string(team.get_mut(field)) {
                        Ok(Some(secret)) => candidates.push(candidate(
                            source,
                            "team",
                            kind,
                            CredentialOwnerScope::Team(team_id.clone()),
                            secret,
                        )),
                        Ok(None) => {}
                        Err(error) => push_source_error(source_errors, source, error),
                    }
                }
            }
            Err(error) => push_source_error(source_errors, "settings.json:team", error),
        }
    }
    if let Some(device) = settings
        .get_mut("device_sync")
        .and_then(Value::as_object_mut)
    {
        match required_non_secret_id(device, "group_id", "device_sync") {
            Ok(group_id) => {
                for (field, kind) in [
                    ("group_secret", CredentialKind::SyncKey),
                    ("pairing_code", CredentialKind::Password),
                ] {
                    let source = format!("settings:device_sync:{group_id}:{field}");
                    match take_secret_string(device.get_mut(field)) {
                        Ok(Some(secret)) => candidates.push(candidate(
                            source,
                            "device_sync",
                            kind,
                            CredentialOwnerScope::DeviceGroup(group_id.clone()),
                            secret,
                        )),
                        Ok(None) => {}
                        Err(error) => push_source_error(source_errors, source, error),
                    }
                }
            }
            Err(error) => push_source_error(source_errors, "settings.json:device_sync", error),
        }
    }
}

fn collect_mcp_bundles(
    settings: &mut Value,
    eligible_indexes: &HashSet<usize>,
    candidates: &mut Vec<LegacyCredentialCandidate>,
    source_errors: &mut Vec<PendingMigrationSourceError>,
) {
    let Some(servers) = settings
        .get_mut("mcp_servers")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for (index, server) in servers.iter_mut().enumerate() {
        if !eligible_indexes.contains(&index) {
            continue;
        }
        let Some(object) = server.as_object_mut() else {
            push_source_error(
                source_errors,
                "settings.json:mcp_servers",
                BridgeError::InvalidInput("MCP server object"),
            );
            continue;
        };
        let Some(instance_id) = object
            .get("instance_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
        else {
            push_source_error(
                source_errors,
                "settings.json:mcp_servers",
                BridgeError::InvalidInput("MCP instance_id"),
            );
            continue;
        };
        let Some(transport) = object.get_mut("transport").and_then(Value::as_object_mut) else {
            push_source_error(
                source_errors,
                format!("settings:mcp:{instance_id}"),
                BridgeError::InvalidInput("MCP transport"),
            );
            continue;
        };
        let slot = match transport.get("type").and_then(Value::as_str) {
            Some("stdio") => "env",
            Some("http") => "headers",
            _ => {
                push_source_error(
                    source_errors,
                    format!("settings:mcp:{instance_id}"),
                    BridgeError::InvalidInput("MCP transport type"),
                );
                continue;
            }
        };
        let source = format!("settings:mcp:{instance_id}:{slot}");
        match take_string_map_bundle(transport.get_mut(slot)) {
            Ok(Some(secret)) => candidates.push(candidate(
                source,
                format!("mcp:{instance_id}"),
                CredentialKind::McpSecret,
                CredentialOwnerScope::Connector(format!("mcp:{instance_id}")),
                secret,
            )),
            Ok(None) => {}
            Err(error) => push_source_error(source_errors, source, error),
        }
    }
}

fn collect_ticktick(
    app_data_root: &Path,
    candidates: &mut Vec<LegacyCredentialCandidate>,
    source_errors: &mut Vec<PendingMigrationSourceError>,
) {
    let path = app_data_root.join("ticktick_sync.json");
    let mut ticktick = match read_json_value(&path) {
        Ok(Some(value)) => value,
        Ok(None) => return,
        Err(error) => {
            push_source_error(source_errors, "ticktick_sync.json", error);
            return;
        }
    };
    let Some(tokens) = ticktick.get_mut("tokens").and_then(Value::as_object_mut) else {
        return;
    };
    for (field, kind) in [
        ("accessToken", CredentialKind::OAuthAccessToken),
        ("refreshToken", CredentialKind::OAuthRefreshToken),
    ] {
        let source = format!("ticktick:tokens:{field}");
        match take_secret_string(tokens.get_mut(field)) {
            Ok(Some(secret)) => candidates.push(candidate(
                source,
                "ticktick",
                kind,
                CredentialOwnerScope::Global,
                secret,
            )),
            Ok(None) => {}
            Err(error) => push_source_error(source_errors, source, error),
        }
    }
}

fn collect_court_cookie(
    settings: &Value,
    candidates: &mut Vec<LegacyCredentialCandidate>,
    source_errors: &mut Vec<PendingMigrationSourceError>,
) {
    let Some(raw_dir) = settings
        .get("court_filing_cookie_dir")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let Some(account) = settings
        .get("court_filing_account")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let safe_account = account
        .chars()
        .filter(|character| character.is_alphanumeric())
        .collect::<String>();
    if safe_account.is_empty() {
        return;
    }
    let expanded = shellexpand::tilde(raw_dir).into_owned();
    let cookie_path = PathBuf::from(expanded).join(format!("court_zxfw_{safe_account}.json"));
    match verify_legacy_regular_file(&cookie_path) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            push_source_error(source_errors, "court_filing:cookie_store", error);
            return;
        }
    }
    let bytes = match fs::read(&cookie_path) {
        Ok(bytes) => bytes,
        Err(source) => {
            push_source_error(
                source_errors,
                "court_filing:cookie_store",
                BridgeError::Io {
                    operation: "read court cookie store",
                    path: cookie_path,
                    source,
                },
            );
            return;
        }
    };
    if bytes.is_empty() {
        return;
    }
    if serde_json::from_slice::<serde::de::IgnoredAny>(&bytes).is_err() {
        push_source_error(
            source_errors,
            "court_filing:cookie_store",
            BridgeError::InvalidInput("court cookie JSON"),
        );
        return;
    }
    candidates.push(candidate(
        "court_filing:cookie_store".to_owned(),
        "court_filing",
        CredentialKind::SessionCookie,
        CredentialOwnerScope::Global,
        Zeroizing::new(bytes),
    ));
}

fn push_source_error(
    source_errors: &mut Vec<PendingMigrationSourceError>,
    source: impl Into<String>,
    error: BridgeError,
) {
    source_errors.push(PendingMigrationSourceError {
        source: source.into(),
        error: error.to_string(),
    });
}

fn verify_legacy_regular_file(path: &Path) -> BridgeResult<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(BridgeError::Io {
                operation: "inspect legacy credential source",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || is_windows_reparse_point(&metadata) {
        return Err(BridgeError::SymlinkNotAllowed {
            path: path.to_path_buf(),
        });
    }
    if !metadata.is_file() {
        return Err(BridgeError::NonRegularFile {
            path: path.to_path_buf(),
        });
    }
    Ok(true)
}

#[cfg(windows)]
fn is_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn required_non_secret_id(
    object: &Map<String, Value>,
    field: &'static str,
    owner: &'static str,
) -> BridgeResult<String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .ok_or(BridgeError::InvalidInput(owner))
}

fn take_secret_string(value: Option<&mut Value>) -> BridgeResult<Option<Zeroizing<Vec<u8>>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::String(secret) => {
            let secret = std::mem::take(secret);
            if secret.is_empty() {
                Ok(None)
            } else {
                Ok(Some(Zeroizing::new(secret.into_bytes())))
            }
        }
        _ => Err(BridgeError::InvalidInput("legacy secret string")),
    }
}

fn take_string_map_bundle(value: Option<&mut Value>) -> BridgeResult<Option<Zeroizing<Vec<u8>>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let map = value
        .as_object_mut()
        .ok_or(BridgeError::InvalidInput("legacy secret bundle"))?;
    if map.is_empty() {
        return Ok(None);
    }
    let mut secrets = BTreeMap::new();
    for (key, value) in map {
        let Value::String(secret) = value else {
            secrets.values_mut().for_each(Zeroize::zeroize);
            return Err(BridgeError::InvalidInput("legacy secret bundle value"));
        };
        secrets.insert(key.clone(), std::mem::take(secret));
    }
    let encoded = serde_json::to_vec(&secrets)
        .map_err(|_| BridgeError::InvalidInput("legacy secret bundle encoding"))?;
    secrets.values_mut().for_each(Zeroize::zeroize);
    Ok(Some(Zeroizing::new(encoded)))
}

fn candidate(
    stable_inventory_id: String,
    provider_or_connector_id: impl Into<String>,
    kind: CredentialKind,
    owner_scope: CredentialOwnerScope,
    secret: Zeroizing<Vec<u8>>,
) -> LegacyCredentialCandidate {
    LegacyCredentialCandidate {
        descriptor: PendingCredentialDescriptor {
            stable_inventory_id,
            provider_or_connector_id: provider_or_connector_id.into(),
            kind,
            owner_scope,
        },
        secret,
    }
}

fn atomic_write_json(path: &Path, value: &Value) -> BridgeResult<()> {
    let parent = path
        .parent()
        .ok_or(BridgeError::InvalidInput("settings parent"))?;
    fs::create_dir_all(parent).map_err(|source| BridgeError::Io {
        operation: "create settings directory",
        path: parent.to_path_buf(),
        source,
    })?;
    let mut temp = SecureAtomicFile::new(path)?;
    serde_json::to_writer_pretty(temp.as_file_mut(), value)
        .map_err(|_| BridgeError::InvalidInput("settings serialization"))?;
    temp.as_file_mut()
        .write_all(b"\n")
        .map_err(|source| BridgeError::Io {
            operation: "write additive settings candidate",
            path: path.to_path_buf(),
            source,
        })?;
    temp.persist()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum JsonPathSegment {
    Key(String),
    Index(usize),
}

struct JsonSanitizer<'a> {
    bytes: &'a [u8],
    replacements: Vec<(usize, usize, &'static [u8])>,
    target_counts: BTreeMap<String, usize>,
}

impl<'a> JsonSanitizer<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            replacements: Vec::new(),
            target_counts: BTreeMap::new(),
        }
    }

    fn parse(mut self) -> BridgeResult<Vec<(usize, usize, &'static [u8])>> {
        let mut cursor = 0;
        self.skip_whitespace(&mut cursor);
        self.parse_value(&mut cursor, &mut Vec::new())?;
        self.skip_whitespace(&mut cursor);
        if cursor != self.bytes.len() {
            return Err(BridgeError::InvalidInput("settings JSON trailing bytes"));
        }
        if self.target_counts.values().any(|count| *count > 1) {
            return Err(BridgeError::CorruptMetadata(
                "duplicate Task 3B secret field in settings JSON".to_owned(),
            ));
        }
        Ok(self.replacements)
    }

    fn parse_value(
        &mut self,
        cursor: &mut usize,
        path: &mut Vec<JsonPathSegment>,
    ) -> BridgeResult<()> {
        self.skip_whitespace(cursor);
        let start = *cursor;
        match self.bytes.get(*cursor).copied() {
            Some(b'{') => self.parse_object(cursor, path)?,
            Some(b'[') => self.parse_array(cursor, path)?,
            Some(b'"') => {
                self.parse_string(cursor)?;
            }
            Some(_) => self.parse_scalar(cursor)?,
            None => return Err(BridgeError::InvalidInput("settings JSON value")),
        }
        let end = *cursor;
        if let Some(replacement) = replacement_for_path(path) {
            let key = path_key(path);
            *self.target_counts.entry(key).or_default() += 1;
            self.replacements.push((start, end, replacement));
        }
        Ok(())
    }

    fn parse_object(
        &mut self,
        cursor: &mut usize,
        path: &mut Vec<JsonPathSegment>,
    ) -> BridgeResult<()> {
        *cursor += 1;
        self.skip_whitespace(cursor);
        if self.bytes.get(*cursor) == Some(&b'}') {
            *cursor += 1;
            return Ok(());
        }
        loop {
            self.skip_whitespace(cursor);
            let key_start = *cursor;
            self.parse_string(cursor)?;
            let key: String = serde_json::from_slice(&self.bytes[key_start..*cursor])
                .map_err(|_| BridgeError::InvalidInput("settings JSON object key"))?;
            self.skip_whitespace(cursor);
            if self.bytes.get(*cursor) != Some(&b':') {
                return Err(BridgeError::InvalidInput("settings JSON object colon"));
            }
            *cursor += 1;
            path.push(JsonPathSegment::Key(key));
            self.parse_value(cursor, path)?;
            path.pop();
            self.skip_whitespace(cursor);
            match self.bytes.get(*cursor) {
                Some(b',') => *cursor += 1,
                Some(b'}') => {
                    *cursor += 1;
                    return Ok(());
                }
                _ => return Err(BridgeError::InvalidInput("settings JSON object delimiter")),
            }
        }
    }

    fn parse_array(
        &mut self,
        cursor: &mut usize,
        path: &mut Vec<JsonPathSegment>,
    ) -> BridgeResult<()> {
        *cursor += 1;
        self.skip_whitespace(cursor);
        if self.bytes.get(*cursor) == Some(&b']') {
            *cursor += 1;
            return Ok(());
        }
        let mut index = 0;
        loop {
            path.push(JsonPathSegment::Index(index));
            self.parse_value(cursor, path)?;
            path.pop();
            index += 1;
            self.skip_whitespace(cursor);
            match self.bytes.get(*cursor) {
                Some(b',') => *cursor += 1,
                Some(b']') => {
                    *cursor += 1;
                    return Ok(());
                }
                _ => return Err(BridgeError::InvalidInput("settings JSON array delimiter")),
            }
        }
    }

    fn parse_string(&self, cursor: &mut usize) -> BridgeResult<()> {
        if self.bytes.get(*cursor) != Some(&b'"') {
            return Err(BridgeError::InvalidInput("settings JSON string"));
        }
        *cursor += 1;
        let mut escaped = false;
        while let Some(byte) = self.bytes.get(*cursor).copied() {
            *cursor += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                return Ok(());
            }
        }
        Err(BridgeError::InvalidInput(
            "unterminated settings JSON string",
        ))
    }

    fn parse_scalar(&self, cursor: &mut usize) -> BridgeResult<()> {
        let start = *cursor;
        while let Some(byte) = self.bytes.get(*cursor).copied() {
            if byte.is_ascii_whitespace() || matches!(byte, b',' | b']' | b'}') {
                break;
            }
            *cursor += 1;
        }
        if *cursor == start {
            return Err(BridgeError::InvalidInput("settings JSON scalar"));
        }
        Ok(())
    }

    fn skip_whitespace(&self, cursor: &mut usize) {
        while self.bytes.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
            *cursor += 1;
        }
    }
}

fn replacement_for_path(path: &[JsonPathSegment]) -> Option<&'static [u8]> {
    match path {
        [JsonPathSegment::Key(field)] if is_task3b_scalar_field(field) => Some(b"null"),
        [JsonPathSegment::Key(servers), JsonPathSegment::Index(_), JsonPathSegment::Key(transport), JsonPathSegment::Key(slot)]
            if servers == "mcp_servers"
                && transport == "transport"
                && matches!(slot.as_str(), "env" | "headers") =>
        {
            Some(b"{}")
        }
        _ => None,
    }
}

fn is_task3b_scalar_field(field: &str) -> bool {
    matches!(
        field,
        "mineru_api_key"
            | "paddle_vl_api_key"
            | "cloud_llm_api_key"
            | "minimax_api_key"
            | "compat_llm_api_key"
            | "glm_llm_api_key"
            | "mimo_llm_api_key"
            | "kimi_llm_api_key"
            | "custom_llm_api_key"
            | "yuandian_api_key"
            | "embedding_api_key"
            | "kuaidi100_key"
            | "feishu_webhook_url"
    )
}

fn path_key(path: &[JsonPathSegment]) -> String {
    path.iter()
        .map(|segment| match segment {
            JsonPathSegment::Key(key) => key.clone(),
            JsonPathSegment::Index(index) => index.to_string(),
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn sanitize_settings_bytes_preserving_other_fields(raw: &[u8]) -> BridgeResult<Vec<u8>> {
    let mut replacements = JsonSanitizer::new(raw).parse()?;
    replacements.sort_by_key(|(start, _, _)| *start);
    let mut sanitized = raw.to_vec();
    for (start, end, replacement) in replacements.into_iter().rev() {
        sanitized.splice(start..end, replacement.iter().copied());
    }
    Ok(sanitized)
}
