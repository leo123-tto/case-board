use std::fmt;
use std::time::Instant;

use zeroize::{Zeroize, Zeroizing};

use super::types::{BridgeError, BridgeResult, LeaseBinding};

pub struct SecretLease {
    binding: LeaseBinding,
    expires_at: Instant,
    secret: Zeroizing<Vec<u8>>,
    consumed: bool,
}

impl SecretLease {
    pub(crate) fn new(
        binding: LeaseBinding,
        expires_at: Instant,
        secret: Zeroizing<Vec<u8>>,
    ) -> Self {
        Self {
            binding,
            expires_at,
            secret,
            consumed: false,
        }
    }

    pub fn with_secret<T>(
        &mut self,
        presented: &LeaseBinding,
        consume: impl FnOnce(&[u8]) -> T,
    ) -> BridgeResult<T> {
        if self.consumed {
            return Err(BridgeError::LeaseConsumed);
        }
        if Instant::now() >= self.expires_at {
            self.consumed = true;
            self.zeroize_now();
            return Err(BridgeError::LeaseExpired);
        }
        self.verify_binding(presented)?;
        self.consumed = true;

        let guard = ExposureGuard {
            secret: &mut self.secret,
        };
        let result = consume(guard.secret.as_slice());
        drop(guard);
        Ok(result)
    }

    pub fn close(mut self) {
        self.consumed = true;
        self.zeroize_now();
    }

    fn verify_binding(&self, presented: &LeaseBinding) -> BridgeResult<()> {
        if presented.consumer != self.binding.consumer {
            return Err(BridgeError::LeaseBindingMismatch { field: "consumer" });
        }
        if presented.provider_or_connector_id != self.binding.provider_or_connector_id {
            return Err(BridgeError::LeaseBindingMismatch {
                field: "provider_or_connector_id",
            });
        }
        if presented.handle != self.binding.handle {
            return Err(BridgeError::LeaseBindingMismatch { field: "handle" });
        }
        if presented.revision != self.binding.revision {
            return Err(BridgeError::LeaseBindingMismatch { field: "revision" });
        }
        Ok(())
    }

    fn zeroize_now(&mut self) {
        self.secret.zeroize();
        self.secret.clear();
    }
}

impl fmt::Debug for SecretLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretLease(<redacted>)")
    }
}

impl Drop for SecretLease {
    fn drop(&mut self) {
        self.zeroize_now();
    }
}

pub struct TypedSecretLease {
    lease: SecretLease,
    binding: LeaseBinding,
}

impl TypedSecretLease {
    pub(crate) fn new(lease: SecretLease, binding: LeaseBinding) -> Self {
        Self { lease, binding }
    }

    pub fn with_secret<T>(&mut self, consume: impl FnOnce(&[u8]) -> T) -> BridgeResult<T> {
        self.lease.with_secret(&self.binding, consume)
    }

    pub fn close(self) {
        self.lease.close();
    }
}

impl fmt::Debug for TypedSecretLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TypedSecretLease(<redacted>)")
    }
}

struct ExposureGuard<'a> {
    secret: &'a mut Zeroizing<Vec<u8>>,
}

impl Drop for ExposureGuard<'_> {
    fn drop(&mut self) {
        self.secret.zeroize();
        self.secret.clear();
    }
}
