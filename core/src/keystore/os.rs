//! OS keystore backend (`architecture.md` §3.1 library list, §3.2).
//!
//! > OS keystore via `keyring` (macOS Keychain, Windows Credential Manager, Linux Secret
//! > Service).
//!
//! This is the production backend on macOS and Windows, and on Linux desktops where
//! Secret Service is running. Where it is not, `architecture.md` §3.2 calls for the
//! [`super::FileKeystore`] fallback; **choosing between them is W7** ("Linux keystore
//! fallback" in `dev-plan.md`). W2 only exposes [`OsKeystore::is_available`] so W7 has
//! something to probe with.
//!
//! ## Why there is no automated test of this backend in CI
//!
//! `dev-plan.md` W2 asks for "OS keystore mock in tests; real backend on one platform job
//! **when practical**". It is not practical here: the whole dev environment is a headless
//! Linux container (CONTRIBUTING.md), which has no Keychain, no Credential Manager, and
//! no D-Bus session bus for Secret Service. A test that touched a real keystore would
//! either fail on every developer machine or silently no-op. So the real backend is
//! covered by [`os_keystore_smoke`] — `#[ignore]`d, run by hand on a desktop with
//! `cargo test -- --ignored os_keystore` — and everything above this seam is covered
//! through [`super::InMemoryKeystore`]. The trait is what makes that split honest: the
//! `SessionManager` cannot tell the two apart.

use keyring::Entry;

use super::{KeystoreBackend, KeystoreBackendKind, KeystoreError, KeystoreItem};

/// Keystore service name. Stable: changing it orphans every existing vault.
pub const KEYSTORE_SERVICE: &str = "com.privacygate.vault";

/// The single well-known slot. v1 is one local account (architecture §7), and the slot
/// must be readable while locked to answer `first_run` vs `locked` — see the module docs
/// of [`super`].
pub const KEYSTORE_SLOT: &str = "vault-keystore-item";

/// `KeystoreItem` storage backed by the platform credential store.
#[derive(Debug, Clone)]
pub struct OsKeystore {
    service: String,
    slot: String,
}

impl Default for OsKeystore {
    fn default() -> Self {
        Self::new()
    }
}

impl OsKeystore {
    /// The production service/slot pair.
    #[must_use]
    pub fn new() -> Self {
        Self {
            service: KEYSTORE_SERVICE.to_string(),
            slot: KEYSTORE_SLOT.to_string(),
        }
    }

    /// A backend on a caller-chosen service/slot. Used by the ignored smoke test so a
    /// manual run cannot clobber a real vault's item.
    #[must_use]
    pub fn with_slot(service: impl Into<String>, slot: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            slot: slot.into(),
        }
    }

    /// Whether a platform credential store is usable in this process.
    ///
    /// This is the probe W7 needs to decide between this backend and the Linux `0600`
    /// fallback (architecture §3.2). W2 does not call it.
    #[must_use]
    pub fn is_available() -> bool {
        Entry::store_status().is_ok()
    }

    fn entry(&self) -> Result<Entry, KeystoreError> {
        Entry::new(&self.service, &self.slot).map_err(|_| KeystoreError::Unavailable)
    }
}

impl KeystoreBackend for OsKeystore {
    fn load(&self) -> Result<Option<KeystoreItem>, KeystoreError> {
        match self.entry()?.get_secret() {
            Ok(bytes) => KeystoreItem::from_bytes(&bytes).map(Some),
            // The only condition that may report "no account". Every other error is a
            // read failure and must not be mistaken for an empty keystore, or first-run
            // would offer to overwrite a live vault's wrapped master key.
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(keyring::Error::NoStorageAccess(_)) => Err(KeystoreError::Unavailable),
            Err(_) => Err(KeystoreError::Backend("os keystore read failed")),
        }
    }

    fn store(&self, item: &KeystoreItem) -> Result<(), KeystoreError> {
        self.entry()?
            .set_secret(&item.to_bytes())
            .map_err(|_| KeystoreError::Backend("os keystore write failed"))
    }

    fn delete(&self) -> Result<(), KeystoreError> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(KeystoreError::Backend("os keystore delete failed")),
        }
    }

    fn kind(&self) -> KeystoreBackendKind {
        KeystoreBackendKind::OsKeystore
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{wrap, Aad, ArtifactKind};
    use crate::keystore::{Argon2idParams, AuditHead};

    /// Real-backend round trip against the platform credential store.
    ///
    /// `#[ignore]`d on purpose: the Docker-only dev environment (CONTRIBUTING.md) is a
    /// headless Linux container with no Keychain / Credential Manager / Secret Service.
    /// Run it by hand on a desktop:
    ///
    /// ```text
    /// cargo test -p pg-core -- --ignored os_keystore_smoke
    /// ```
    ///
    /// It uses a throwaway slot so it can never touch a real vault's item.
    #[test]
    #[ignore = "requires a real OS keystore; not available in the headless dev container"]
    fn os_keystore_smoke() {
        let ks = OsKeystore::with_slot(KEYSTORE_SERVICE, "pg-test-slot-do-not-use");
        let _ = ks.delete();
        assert!(ks.load().unwrap().is_none());

        let item = KeystoreItem {
            account_id: "00000000-0000-4000-8000-000000000000".to_string(),
            kdf: Argon2idParams {
                salt: vec![0x5A; 16],
                ..Argon2idParams::CURRENT
            },
            wrapped_master_key: wrap(
                &[0x11; 32],
                &[0x22; 32],
                &Aad::global(ArtifactKind::WrappedMaster, 1),
            )
            .unwrap(),
            audit_head: AuditHead::GENESIS,
        };

        ks.store(&item).unwrap();
        assert_eq!(ks.load().unwrap().unwrap(), item);
        ks.delete().unwrap();
        assert!(ks.load().unwrap().is_none());
    }
}
