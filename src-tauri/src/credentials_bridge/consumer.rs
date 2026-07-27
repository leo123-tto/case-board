use std::fmt;
use std::time::{Duration, Instant};

use zeroize::{Zeroize, Zeroizing};

use super::{
    BridgeCredentialConsumer, CredentialBroker, CredentialRefV1, PendingSecretLeaseRequest,
};

/// 非秘密的 legacy pending/active bridge 定位器。
///
/// 生产调用面只长期保存 consumer、stable inventory ID、provider/connector ID 与可选
/// authenticated reference。真正的明文只在单次操作开始后进入 [`BridgeSecretMaterial`]。
#[derive(Clone)]
pub struct PendingCredentialSource {
    broker: Option<CredentialBroker>,
    consumer: BridgeCredentialConsumer,
    stable_inventory_id: String,
    provider_or_connector_id: String,
    credential_ref: Option<CredentialRefV1>,
}

impl PendingCredentialSource {
    pub fn new(
        broker: CredentialBroker,
        consumer: BridgeCredentialConsumer,
        stable_inventory_id: impl Into<String>,
        provider_or_connector_id: impl Into<String>,
        credential_ref: CredentialRefV1,
    ) -> Self {
        Self {
            broker: Some(broker),
            consumer,
            stable_inventory_id: stable_inventory_id.into(),
            provider_or_connector_id: provider_or_connector_id.into(),
            credential_ref: Some(credential_ref),
        }
    }

    pub fn pending(
        consumer: BridgeCredentialConsumer,
        stable_inventory_id: impl Into<String>,
        provider_or_connector_id: impl Into<String>,
    ) -> Self {
        Self {
            broker: None,
            consumer,
            stable_inventory_id: stable_inventory_id.into(),
            provider_or_connector_id: provider_or_connector_id.into(),
            credential_ref: None,
        }
    }

    pub fn stable_inventory_id(&self) -> &str {
        &self.stable_inventory_id
    }

    pub fn provider_or_connector_id(&self) -> &str {
        &self.provider_or_connector_id
    }

    async fn broker(&self) -> Result<CredentialBroker, String> {
        match &self.broker {
            Some(broker) => Ok(broker.clone()),
            None => {
                let app_data_root = crate::db::app_data_dir()
                    .map_err(|error| format!("credential_bridge_unavailable: {error}"))?;
                CredentialBroker::initialize(app_data_root)
                    .await
                    .map_err(|error| format!("credential_bridge_unavailable: {error}"))
            }
        }
    }

    /// metadata-only readiness。只认证 3A journal reference，不打开 vault。
    pub async fn is_ready(&self) -> Result<bool, String> {
        let broker = self.broker().await?;
        match &self.credential_ref {
            Some(credential_ref) => Ok(broker
                .pending_reference(
                    &self.stable_inventory_id,
                    self.consumer,
                    &self.provider_or_connector_id,
                )
                .await
                .map_err(|error| error.to_string())?
                .is_some_and(|actual| &actual == credential_ref)),
            None => Ok(broker
                .pending_reference(
                    &self.stable_inventory_id,
                    self.consumer,
                    &self.provider_or_connector_id,
                )
                .await
                .map_err(|error| error.to_string())?
                .is_some()),
        }
    }

    pub async fn issue_material(&self) -> Result<BridgeSecretMaterial, String> {
        let broker = self.broker().await?;
        let credential_ref = match &self.credential_ref {
            Some(credential_ref) => credential_ref.clone(),
            None => broker
                .pending_reference(
                    &self.stable_inventory_id,
                    self.consumer,
                    &self.provider_or_connector_id,
                )
                .await
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("credential_missing: {}", self.stable_inventory_id))?,
        };
        let request = PendingSecretLeaseRequest::new(
            self.consumer,
            &self.stable_inventory_id,
            &self.provider_or_connector_id,
            credential_ref,
            Instant::now() + Duration::from_secs(30),
        )
        .map_err(|error| error.to_string())?;
        let mut lease = broker
            .issue_pending_lease(request)
            .await
            .map_err(|error| error.to_string())?;
        lease
            .with_secret(|bytes| {
                std::str::from_utf8(bytes)
                    .map(str::to_owned)
                    .map(BridgeSecretMaterial::new)
                    .map_err(|_| "credential material 不是有效 UTF-8".to_owned())
            })
            .map_err(|error| error.to_string())?
    }
}

impl fmt::Debug for PendingCredentialSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingCredentialSource")
            .field("consumer", &self.consumer)
            .field("stable_inventory_id", &self.stable_inventory_id)
            .field("provider_or_connector_id", &self.provider_or_connector_id)
            .field("credential_ref", &self.credential_ref)
            .field("broker", &self.broker.as_ref().map(|_| "<configured>"))
            .finish()
    }
}

pub struct BridgeSecretMaterial(Zeroizing<String>);

impl BridgeSecretMaterial {
    fn new(secret: String) -> Self {
        Self(Zeroizing::new(secret))
    }

    pub fn with_secret<T>(&self, consume: impl FnOnce(&str) -> T) -> T {
        consume(self.0.as_str())
    }

    pub(crate) fn expose(&self) -> &str {
        self.0.as_str()
    }

    pub fn redact(&self, value: &str) -> String {
        let secret = self.0.as_str();
        if secret.is_empty() {
            value.to_owned()
        } else {
            value
                .replace(&format!("Bearer {secret}"), "Bearer [REDACTED]")
                .replace(secret, "[REDACTED]")
        }
    }
}

impl Drop for BridgeSecretMaterial {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for BridgeSecretMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BridgeSecretMaterial(<redacted>)")
    }
}
