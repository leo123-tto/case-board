use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use zeroize::Zeroizing;

use super::master_key::load_master_key;
use super::types::{
    BridgeError, BridgeResult, CredentialHandle, CredentialKind, CredentialOwnerScope,
    BRIDGE_SCHEMA, ENVELOPE_VERSION,
};

pub(crate) const ALGORITHM_NAME: &str = "xchacha20poly1305";

#[derive(Clone, Debug)]
pub struct EnvelopeContext {
    pub handle: CredentialHandle,
    pub revision: i64,
    pub provider_or_connector_id: String,
    pub kind: CredentialKind,
    pub owner_scope: CredentialOwnerScope,
}

#[derive(Clone, Debug)]
pub struct EncryptedEnvelopeV1 {
    pub version: u16,
    pub handle: CredentialHandle,
    pub revision: i64,
    pub algorithm: &'static str,
    pub nonce: [u8; 24],
    pub ciphertext: Vec<u8>,
}

pub trait VaultBackend: Send + Sync {
    fn seal(&self, context: &EnvelopeContext, secret: &[u8]) -> BridgeResult<EncryptedEnvelopeV1>;

    fn open(
        &self,
        context: &EnvelopeContext,
        envelope: &EncryptedEnvelopeV1,
    ) -> BridgeResult<Zeroizing<Vec<u8>>>;
}

#[derive(Clone, Debug)]
pub struct EncryptedCredentialVault {
    master_key_path: PathBuf,
}

impl EncryptedCredentialVault {
    pub fn new(master_key_path: impl AsRef<Path>) -> Self {
        Self {
            master_key_path: master_key_path.as_ref().to_path_buf(),
        }
    }
}

impl VaultBackend for EncryptedCredentialVault {
    fn seal(&self, context: &EnvelopeContext, secret: &[u8]) -> BridgeResult<EncryptedEnvelopeV1> {
        let master_key = load_master_key(&self.master_key_path)?;
        let cipher = XChaCha20Poly1305::new(master_key.as_bytes().into());
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let aad = build_aad(context);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: secret,
                    aad: &aad,
                },
            )
            .map_err(|_| BridgeError::EncryptionFailed)?;
        let mut nonce_bytes = [0u8; 24];
        nonce_bytes.copy_from_slice(&nonce);
        Ok(EncryptedEnvelopeV1 {
            version: ENVELOPE_VERSION,
            handle: context.handle.clone(),
            revision: context.revision,
            algorithm: ALGORITHM_NAME,
            nonce: nonce_bytes,
            ciphertext,
        })
    }

    fn open(
        &self,
        context: &EnvelopeContext,
        envelope: &EncryptedEnvelopeV1,
    ) -> BridgeResult<Zeroizing<Vec<u8>>> {
        if envelope.version != ENVELOPE_VERSION
            || envelope.algorithm != ALGORITHM_NAME
            || envelope.handle != context.handle
            || envelope.revision != context.revision
        {
            return Err(authentication_error(context));
        }
        let master_key = load_master_key(&self.master_key_path)?;
        let cipher = XChaCha20Poly1305::new(master_key.as_bytes().into());
        let aad = build_aad(context);
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(&envelope.nonce),
                Payload {
                    msg: &envelope.ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| authentication_error(context))?;
        Ok(Zeroizing::new(plaintext))
    }
}

fn build_aad(context: &EnvelopeContext) -> Vec<u8> {
    let mut aad = Vec::with_capacity(256);
    append_field(&mut aad, BRIDGE_SCHEMA.as_bytes());
    append_field(&mut aad, context.handle.as_str().as_bytes());
    aad.extend_from_slice(&context.revision.to_be_bytes());
    append_field(&mut aad, context.provider_or_connector_id.as_bytes());
    append_field(&mut aad, context.kind.as_storage_str().as_bytes());
    let owner_scope = context.owner_scope.to_storage_string();
    append_field(&mut aad, owner_scope.as_bytes());
    aad
}

fn append_field(target: &mut Vec<u8>, field: &[u8]) {
    let length = u32::try_from(field.len()).expect("credential AAD field length is bounded");
    target.extend_from_slice(&length.to_be_bytes());
    target.extend_from_slice(field);
}

fn authentication_error(context: &EnvelopeContext) -> BridgeError {
    BridgeError::AuthenticationFailed {
        handle: context.handle.clone(),
        revision: context.revision,
    }
}
