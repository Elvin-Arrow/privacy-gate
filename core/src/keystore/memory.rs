//! Process-local keystore backend.
//!
//! `dev-plan.md` W2 Integrate: "OS keystore mock in tests". This is that mock. It is
//! public (integration tests in `core/tests/` cannot see `#[cfg(test)]` items) but it is
//! never a production configuration — [`KeystoreBackendKind::Memory`] marks it as such.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

use super::{KeystoreBackend, KeystoreBackendKind, KeystoreError, KeystoreItem};

/// An in-memory `KeystoreItem` slot with injectable failures.
#[derive(Debug, Default)]
pub struct InMemoryKeystore {
    slot: Mutex<Option<Vec<u8>>>,
    fail_next_store: AtomicBool,
    fail_loads: AtomicUsize,
}

impl InMemoryKeystore {
    /// An empty keystore — the `first_run` starting point.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Make the next [`KeystoreBackend::store`] call fail, so a test can exercise the
    /// crash/rollback path of first-run and passphrase change without a real OS keystore.
    pub fn fail_next_store(&self) {
        self.fail_next_store.store(true, Ordering::SeqCst);
    }

    /// Make the next `n` [`KeystoreBackend::load`] calls fail.
    ///
    /// A count rather than a flag because one command can read the keystore more than
    /// once — `unlock` consults it to compute the session state and again to fetch the
    /// item — and a test of the read-failure path has to cover both reads.
    pub fn fail_next_loads(&self, n: usize) {
        self.fail_loads.store(n, Ordering::SeqCst);
    }
}

impl KeystoreBackend for InMemoryKeystore {
    fn load(&self) -> Result<Option<KeystoreItem>, KeystoreError> {
        if self
            .fail_loads
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(KeystoreError::Backend("injected load failure"));
        }
        let slot = self.slot.lock().map_err(|_| KeystoreError::Backend("poisoned"))?;
        match slot.as_deref() {
            // Round-trips through the real encoding, so the mock exercises the same
            // codec the OS and file backends do.
            Some(bytes) => KeystoreItem::from_bytes(bytes).map(Some),
            None => Ok(None),
        }
    }

    fn store(&self, item: &KeystoreItem) -> Result<(), KeystoreError> {
        if self.fail_next_store.swap(false, Ordering::SeqCst) {
            return Err(KeystoreError::Backend("injected store failure"));
        }
        let mut slot = self.slot.lock().map_err(|_| KeystoreError::Backend("poisoned"))?;
        *slot = Some(item.to_bytes());
        Ok(())
    }

    fn delete(&self) -> Result<(), KeystoreError> {
        let mut slot = self.slot.lock().map_err(|_| KeystoreError::Backend("poisoned"))?;
        *slot = None;
        Ok(())
    }

    fn kind(&self) -> KeystoreBackendKind {
        KeystoreBackendKind::Memory
    }
}
