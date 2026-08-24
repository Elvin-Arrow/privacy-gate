//! Key storage — the `KeystoreItem` and its backends (`architecture.md` §3.2).
//!
//! > The OS keystore holds a single `KeystoreItem` per local account. […] This section
//! > owns wrap AEAD, Linux fallback, and that the passphrase is never stored.
//!
//! Shapes are [`data-model.md` §5.9](../../../docs/specs/data-model.md):
//! `KeystoreItem { account_id, kdf: Argon2idParams, wrapped_master_key, audit_head }`.
//!
//! # Why the backend is a trait
//!
//! `dev-plan.md` W2 requires an "OS keystore mock in tests; real backend on one platform
//! job when practical", and `architecture.md` §3.2 requires a Linux `0600` file fallback
//! that "the testing spec can treat […] as a distinct configuration with a degraded
//! threat model". Both fall out of one seam: [`KeystoreBackend`]. Three implementations
//! ship in W2 — [`OsKeystore`] (the `keyring` crate), [`FileKeystore`] (the fallback),
//! and [`InMemoryKeystore`] (tests). Choosing between the first two by probing whether
//! Secret Service is actually available is **W7**; W2 only provides
//! [`OsKeystore::is_available`] and leaves selection to the caller.
//!
//! # Why the slot is not keyed by `account_id`
//!
//! v1 accounts are local-only and singular (`architecture.md` §7; §3.2 "a single
//! `KeystoreItem` per local account"). The keystore is the **only** thing readable while
//! locked — the `LocalAccount` row lives in the SQLCipher DB (data-model §5.6), which
//! cannot be opened without the key the keystore holds. So `first_run` vs `locked` has to
//! be answerable from a fixed, well-known slot, and the `account_id` is a *field of* the
//! item rather than a lookup key for it. A multi-account v1 would be a new decision.

mod file;
mod memory;
mod os;

pub use file::{FileKeystore, FALLBACK_FILE_NAME};
pub use memory::InMemoryKeystore;
pub use os::{OsKeystore, KEYSTORE_SERVICE, KEYSTORE_SLOT};

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::crypto::WrappedBlob;

/// Wire version of the serialized `KeystoreItem` envelope. Bumping it is a storage
/// format break and needs a decision record.
const KEYSTORE_ITEM_VERSION: u32 = 1;

/// Argon2id parameters, stored **with** the wrapped master key so a future tuning pass
/// can raise the cost without locking existing vaults out (`architecture.md` §3.1:
/// "Parameters are stored with the wrapped master key").
///
/// data-model §5.9.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Argon2idParams {
    /// Memory cost, in KiB.
    pub m_cost: u32,
    /// Time cost (number of passes).
    pub t_cost: u32,
    /// Degree of parallelism (lanes).
    pub p_cost: u32,
    /// Per-account random salt. Not a secret; it exists to make the derivation unique
    /// per vault so one precomputation cannot attack two of them.
    pub salt: Vec<u8>,
}

/// Salt length in bytes. 128 bits, comfortably above the 8-byte minimum the `argon2`
/// crate enforces and the usual 16-byte recommendation.
pub const SALT_LEN: usize = 16;

impl Argon2idParams {
    /// **OWASP minimum current at implementation** (`architecture.md` §3.1: "Floor: OWASP
    /// minimum current at implementation"), for Argon2id with a 32-byte output:
    /// `m = 19456 KiB (19 MiB)`, `t = 2`, `p = 1`.
    ///
    /// `architecture.md` §3.1 also says to "tune upward so unlock stays within the design
    /// budget (≤ 1 s on the mainstream laptop of design.md §7) without dropping below the
    /// floor". W2 ships the floor itself and keeps it tunable: the parameters travel in
    /// the `KeystoreItem`, so a later chunk can raise [`Argon2idParams::CURRENT`] and
    /// existing vaults keep opening with the parameters they were created under. The
    /// floor is a *lower bound* that `SessionManager` must never write below — it is not
    /// a benchmark, and W2 deliberately does not assert a wall-clock number (hardware in
    /// a container is not the mainstream laptop of design.md §7).
    pub const OWASP_FLOOR: Self = Self {
        m_cost: 19_456,
        t_cost: 2,
        p_cost: 1,
        salt: Vec::new(),
    };

    /// Cost parameters new accounts are created with. Currently exactly the floor; raise
    /// this (never below [`Argon2idParams::OWASP_FLOOR`]) when the unlock budget is
    /// measured on real hardware.
    pub const CURRENT: Self = Self::OWASP_FLOOR;

    /// Fresh parameters with a new random salt, for a new account or a passphrase change.
    ///
    /// # Errors
    /// [`KeystoreError::Rng`] if the OS CSPRNG fails. Never falls back to a weaker source
    /// or to a fixed salt.
    pub fn generate() -> Result<Self, KeystoreError> {
        let mut salt = vec![0u8; SALT_LEN];
        getrandom::fill(&mut salt).map_err(|_| KeystoreError::Rng)?;
        Ok(Self {
            salt,
            ..Self::CURRENT
        })
    }

    /// True if these parameters are at or above the OWASP floor on every axis.
    #[must_use]
    pub fn meets_floor(&self) -> bool {
        let f = Self::OWASP_FLOOR;
        self.m_cost >= f.m_cost
            && self.t_cost >= f.t_cost
            && self.p_cost >= f.p_cost
            && self.salt.len() >= SALT_LEN
    }
}

/// The persisted tip of the audit chain (data-model §5.9).
///
/// **W2 writes the genesis placeholder and never advances it.** There is no audit chain
/// until W5; `architecture.md` §6.2 owns the head-update cadence and §6.3 owns
/// verification, fast-forward and integrity failure. W2's contract is only that the field
/// exists, round-trips through every backend, and survives a passphrase change unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuditHead {
    /// Sequence number of the latest persisted accepted entry.
    pub sequence: u64,
    /// Hash of that entry.
    pub head_hash: [u8; 32],
}

impl AuditHead {
    /// The pre-chain placeholder: no entries yet. W5 replaces this with the real head.
    pub const GENESIS: Self = Self {
        sequence: 0,
        head_hash: [0u8; 32],
    };
}

impl Default for AuditHead {
    fn default() -> Self {
        Self::GENESIS
    }
}

/// The single secret the application persists outside the vault DB (data-model §5.9).
///
/// Note what is *not* here: the passphrase. `architecture.md` §3.2 — "The passphrase is
/// never written to disk, keystore, logs, or audit payloads."
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeystoreItem {
    /// Matches `LocalAccount.id` (data-model §5.6).
    pub account_id: String,
    /// Argon2id parameters used to derive the `wrap_key` from the passphrase.
    pub kdf: Argon2idParams,
    /// `AEAD(wrap_key, vault_master_key)` under AAD kind 6 (`ArtifactKind::WrappedMaster`).
    pub wrapped_master_key: WrappedBlob,
    /// Anti-truncation head (W5).
    pub audit_head: AuditHead,
}

// ---------------------------------------------------------------------------
// Serialization
//
// A private mirror struct rather than serde derives on the public types: the public
// types belong to the data model, and the on-keystore encoding is a storage concern that
// must be able to change (versioned) without reshaping them. Blobs are lowercase hex so
// the fallback file is inspectable and never accidentally byte-identical to key material.
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct StoredKdf {
    m_cost: u32,
    t_cost: u32,
    p_cost: u32,
    salt: String,
}

#[derive(Serialize, Deserialize)]
struct StoredHead {
    sequence: u64,
    head_hash: String,
}

#[derive(Serialize, Deserialize)]
struct StoredItem {
    v: u32,
    account_id: String,
    kdf: StoredKdf,
    nonce: String,
    wrapped_master_key: String,
    audit_head: StoredHead,
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).expect("nibble is < 16"));
        s.push(char::from_digit((b & 0x0f) as u32, 16).expect("nibble is < 16"));
    }
    s
}

fn from_hex(s: &str) -> Result<Vec<u8>, KeystoreError> {
    if !s.len().is_multiple_of(2) {
        return Err(KeystoreError::Corrupt);
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        let hi = (pair[0] as char).to_digit(16).ok_or(KeystoreError::Corrupt)?;
        let lo = (pair[1] as char).to_digit(16).ok_or(KeystoreError::Corrupt)?;
        out.push(((hi << 4) | lo) as u8);
    }
    Ok(out)
}

impl KeystoreItem {
    /// Encode for persistence. Used by every backend so the OS keystore and the Linux
    /// fallback store byte-identical payloads (architecture §3.2: "persist the same
    /// `KeystoreItem`").
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let stored = StoredItem {
            v: KEYSTORE_ITEM_VERSION,
            account_id: self.account_id.clone(),
            kdf: StoredKdf {
                m_cost: self.kdf.m_cost,
                t_cost: self.kdf.t_cost,
                p_cost: self.kdf.p_cost,
                salt: to_hex(&self.kdf.salt),
            },
            nonce: to_hex(&self.wrapped_master_key.nonce),
            wrapped_master_key: to_hex(&self.wrapped_master_key.ciphertext),
            audit_head: StoredHead {
                sequence: self.audit_head.sequence,
                head_hash: to_hex(&self.audit_head.head_hash),
            },
        };
        serde_json::to_vec(&stored).expect("KeystoreItem is always serializable")
    }

    /// Decode a persisted item.
    ///
    /// Fails closed with [`KeystoreError::Corrupt`] on anything that is not an exact,
    /// well-formed v1 record. A truncated or garbled item must never decode to
    /// "no account": that would offer a first-run flow over a live vault and overwrite
    /// the only copy of the wrapped master key.
    ///
    /// # Errors
    /// [`KeystoreError::Corrupt`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, KeystoreError> {
        let stored: StoredItem =
            serde_json::from_slice(bytes).map_err(|_| KeystoreError::Corrupt)?;
        if stored.v != KEYSTORE_ITEM_VERSION {
            return Err(KeystoreError::Corrupt);
        }
        let head_hash_bytes = from_hex(&stored.audit_head.head_hash)?;
        let head_hash: [u8; 32] = head_hash_bytes
            .try_into()
            .map_err(|_| KeystoreError::Corrupt)?;

        Ok(Self {
            account_id: stored.account_id,
            kdf: Argon2idParams {
                m_cost: stored.kdf.m_cost,
                t_cost: stored.kdf.t_cost,
                p_cost: stored.kdf.p_cost,
                salt: from_hex(&stored.kdf.salt)?,
            },
            wrapped_master_key: WrappedBlob {
                nonce: from_hex(&stored.nonce)?,
                ciphertext: from_hex(&stored.wrapped_master_key)?,
            },
            audit_head: AuditHead {
                sequence: stored.audit_head.sequence,
                head_hash,
            },
        })
    }
}

/// Which backend the Key Manager is using.
///
/// `architecture.md` §3.2: "The Key Manager records which backend is in use so the
/// testing spec can treat the fallback as a distinct configuration with a degraded threat
/// model."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeystoreBackendKind {
    /// Real OS keystore: macOS Keychain, Windows Credential Manager, Linux Secret Service.
    OsKeystore,
    /// Linux `0600` file fallback. Degraded: the wrapped blob sits next to the DB, so a
    /// stolen app-data directory is one artifact, not two.
    FileFallback,
    /// Process-local, non-persistent. Tests only — never a production configuration.
    Memory,
}

/// Failure modes of a keystore backend. Coarse on purpose: nothing here is allowed to
/// carry key material or a passphrase into a message.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeystoreError {
    /// A stored item exists but is not a well-formed record. Fails closed — never
    /// reported as "no account".
    Corrupt,
    /// The backend itself is not usable on this machine (e.g. no Secret Service). The
    /// caller may fall back (architecture §3.2); W7 owns that selection logic.
    Unavailable,
    /// I/O or backend failure. The payload is a fixed class, never a path or a secret.
    Backend(&'static str),
    /// The OS CSPRNG failed.
    Rng,
}

impl core::fmt::Display for KeystoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            KeystoreError::Corrupt => f.write_str("keystore item is corrupt"),
            KeystoreError::Unavailable => f.write_str("keystore backend is unavailable"),
            KeystoreError::Backend(class) => write!(f, "keystore backend failure: {class}"),
            KeystoreError::Rng => f.write_str("system CSPRNG failure"),
        }
    }
}

impl std::error::Error for KeystoreError {}

/// The seam between the Key Manager and wherever the `KeystoreItem` actually lives.
///
/// Implementations must be safe to share across threads: the Tauri command layer (W29)
/// will hold the `SessionManager` behind a lock and call these from the async runtime.
///
/// Single-slot by design — see the module docs for why there is no `account_id` key.
pub trait KeystoreBackend: Send + Sync {
    /// Read the stored item. `Ok(None)` means **there is genuinely no account** (this is
    /// what drives `first_run`), so a backend must return `Err` — never `Ok(None)` — when
    /// it merely failed to read.
    fn load(&self) -> Result<Option<KeystoreItem>, KeystoreError>;

    /// Write (or overwrite) the stored item.
    fn store(&self, item: &KeystoreItem) -> Result<(), KeystoreError>;

    /// Remove the stored item. Idempotent: deleting a missing item is `Ok(())`.
    fn delete(&self) -> Result<(), KeystoreError>;

    /// Which backend this is (architecture §3.2 recording requirement).
    fn kind(&self) -> KeystoreBackendKind;
}

// ---------------------------------------------------------------------------
// Backend selection (W7 — architecture §3.2's "if Secret Service is unavailable")
// ---------------------------------------------------------------------------

/// Probe [`OsKeystore::is_available`] and construct the right production backend:
/// [`OsKeystore`] when a platform credential store is usable, [`FileKeystore`] (at
/// `app_data_dir`/[`FALLBACK_FILE_NAME`]) when it is not — architecture §3.2's Linux
/// fallback, though the probe itself is platform-generic (`keyring::Entry::store_status`),
/// so the same fallback fires on any OS/session missing a usable credential store, not
/// only Linux without Secret Service.
///
/// This is a thin wrapper over [`select_backend_with`] with the real probe; call site for
/// production wiring (W29). Tests use [`select_backend_with`] directly so the branch taken
/// does not depend on whether the test runner happens to have a real keystore.
#[must_use]
pub fn select_backend(app_data_dir: &Path) -> Arc<dyn KeystoreBackend> {
    select_backend_with(OsKeystore::is_available, app_data_dir)
}

/// [`select_backend`] with the availability probe injected, so both branches are
/// deterministically testable regardless of the runner's actual environment (the headless
/// Docker dev container, per `CONTRIBUTING.md`, always reports Secret Service unavailable —
/// without this seam, the "OS keystore selected" branch would be untestable there at all).
#[must_use]
pub fn select_backend_with(
    is_os_keystore_available: impl FnOnce() -> bool,
    app_data_dir: &Path,
) -> Arc<dyn KeystoreBackend> {
    if is_os_keystore_available() {
        Arc::new(OsKeystore::new())
    } else {
        Arc::new(FileKeystore::new(app_data_dir.join(FALLBACK_FILE_NAME)))
    }
}
