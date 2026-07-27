use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use zeroize::Zeroizing;

use super::encrypted_vault::{EncryptedEnvelopeV1, EnvelopeContext, VaultBackend};
use super::types::{BridgeResult, CredentialHandle};

#[derive(Clone, Default)]
pub struct VaultAccessCounters {
    decrypt_count: Arc<AtomicUsize>,
    legacy_system_vault_read_count: Arc<AtomicUsize>,
}

impl VaultAccessCounters {
    pub fn decrypt_count(&self) -> usize {
        self.decrypt_count.load(Ordering::SeqCst)
    }

    pub fn legacy_system_vault_read_count(&self) -> usize {
        self.legacy_system_vault_read_count.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Default)]
pub struct PanicOnVaultOpen {
    counters: VaultAccessCounters,
}

impl PanicOnVaultOpen {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn counters(&self) -> VaultAccessCounters {
        self.counters.clone()
    }
}

impl VaultBackend for PanicOnVaultOpen {
    fn seal(
        &self,
        _context: &EnvelopeContext,
        _secret: &[u8],
    ) -> BridgeResult<EncryptedEnvelopeV1> {
        panic!("PanicOnVaultOpen must never seal a credential")
    }

    fn open(
        &self,
        _context: &EnvelopeContext,
        _envelope: &EncryptedEnvelopeV1,
    ) -> BridgeResult<Zeroizing<Vec<u8>>> {
        self.counters.decrypt_count.fetch_add(1, Ordering::SeqCst);
        panic!("PanicOnVaultOpen observed a forbidden decrypt")
    }
}

#[derive(Clone, Default)]
pub struct CountingLegacyVault {
    counters: VaultAccessCounters,
}

impl CountingLegacyVault {
    pub fn counters(&self) -> VaultAccessCounters {
        self.counters.clone()
    }

    pub fn read(&self, _handle: &CredentialHandle) {
        self.counters
            .legacy_system_vault_read_count
            .fetch_add(1, Ordering::SeqCst);
    }
}
