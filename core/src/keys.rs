//! Key Manager key material (`architecture.md` §3.1).
//!
//! ```text
//! passphrase (user, never stored)
//!   └─ Argon2id ──► wrap_key (32 bytes, ephemeral)          [derive_wrap_key, here]
//!         └─ AEAD wrap ──► vault_master_key (32 bytes)      [VaultMasterKey, here]
//!               ├─ HKDF-SHA-256 info="pg-db-v1"        → sqlcipher_key
//!               ├─ HKDF-SHA-256 info="pg-audit-mac-v1" → audit_mac_key
//!               └─ AEAD wrap of per-artifact DEKs           [W3]
//! ```
//!
//! W1 (`crate::crypto`) owns the AEAD, HKDF and DEK primitives. This module owns the two
//! things W1 explicitly left out: the Argon2id step from passphrase to `wrap_key`, and
//! the `vault_master_key` as a live, zeroizing session object with its labelled subkeys.
//!
//! **The passphrase is borrowed, never stored.** `derive_wrap_key` takes a `&str` and
//! returns a [`Zeroizing`] key; nothing in this module has a passphrase field, so there
//! is no struct whose `Debug` could leak one (C-API-1).

use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use crate::crypto::{derive, Dek, DEK_LEN};
use crate::keystore::{Argon2idParams, KeystoreItem};

/// Length of every key in the v1 hierarchy: 256 bits.
pub const KEY_LEN: usize = DEK_LEN;

/// HKDF info label for the SQLCipher key (architecture §3.1).
pub const INFO_SQLCIPHER: &str = "pg-db-v1";

/// HKDF info label for the audit MAC key (architecture §3.1).
pub const INFO_AUDIT_MAC: &str = "pg-audit-mac-v1";

/// `format_version` bound into the AAD of the wrapped master key. Bumping it is a storage
/// format break.
pub const WRAPPED_MASTER_FORMAT_VERSION: u32 = 1;

/// Why a `wrap_key` derivation failed.
///
/// One coarse variant, carrying no input: reporting *how* the parameters were rejected
/// would say something about the stored item to a caller that only supplied a passphrase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyError {
    /// The Argon2id parameters are not usable (out of range, salt too short) or the
    /// derivation itself failed.
    Kdf,
}

impl core::fmt::Display for KeyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Argon2id key derivation failed")
    }
}

impl std::error::Error for KeyError {}

/// Derive the 32-byte `wrap_key` from a passphrase and the stored Argon2id parameters.
///
/// The parameters travel in the [`KeystoreItem`], not in this code, so raising the cost
/// for new accounts never locks an existing vault out (architecture §3.1: "Parameters are
/// stored with the wrapped master key").
///
/// The result is [`Zeroizing`]: the `wrap_key` is ephemeral and must not outlive the
/// unwrap that consumes it (architecture §3.3, "Lock: zeroize master key, wrap key …").
///
/// # Errors
/// [`KeyError::Kdf`] if the parameters are unusable or Argon2id fails. Never panics on
/// stored input.
pub fn derive_wrap_key(
    passphrase: &str,
    params: &Argon2idParams,
) -> Result<Zeroizing<[u8; KEY_LEN]>, KeyError> {
    let p = Params::new(
        params.m_cost,
        params.t_cost,
        params.p_cost,
        Some(KEY_LEN),
    )
    .map_err(|_| KeyError::Kdf)?;

    // Argon2id v1.3 (`Version::V0x13`) — architecture §3.1 names the variant and version
    // explicitly; Argon2i and Argon2d are not interchangeable here.
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, p);

    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(passphrase.as_bytes(), &params.salt, out.as_mut())
        .map_err(|_| KeyError::Kdf)?;
    Ok(out)
}

/// The live `vault_master_key` — 256 bits, CSPRNG at first run, never written unwrapped.
///
/// Held only while the session is unlocked, and dropped (not merely zeroized in place) on
/// lock: `SessionManager` stores it inside an `Option`, so after `lock()` there is no
/// field left to misuse. `Dek` provides `ZeroizeOnDrop`, so the bytes are also destroyed.
///
/// `Clone` is deliberately absent, inherited from [`Dek`]: every copy is another place
/// the bytes would have to be destroyed.
#[derive(Debug)]
pub struct VaultMasterKey(Dek);

impl VaultMasterKey {
    /// Generate a fresh master key from the OS CSPRNG (architecture §3.4, first-run).
    ///
    /// # Errors
    /// [`crate::crypto::CryptoError::Rng`] if the CSPRNG fails. Never falls back.
    pub fn generate() -> Result<Self, crate::crypto::CryptoError> {
        Ok(Self(Dek::try_generate()?))
    }

    /// Adopt master key bytes recovered by unwrapping the stored item.
    #[must_use]
    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(Dek::from_bytes(bytes))
    }

    /// Borrow the raw bytes, for the AEAD wrap/unwrap of per-artifact DEKs (W3).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        self.0.as_bytes()
    }

    /// Constant-time equality. Never compare key material with `==`.
    #[must_use]
    pub fn ct_eq(&self, other: &VaultMasterKey) -> bool {
        self.0.ct_eq_dek(&other.0)
    }

    /// `HKDF-SHA-256(vault_master_key, info="pg-db-v1")` — the raw SQLCipher key (W3).
    ///
    /// architecture §3.1: this is a **raw** 256-bit key opened with `PRAGMA key = "x'…'"`,
    /// not a passphrase; passing it as UTF-8 would stack SQLCipher's PBKDF2 on top and
    /// blow the ≤ 1 s unlock budget (design.md §7).
    #[must_use]
    pub fn sqlcipher_key(&self) -> Zeroizing<[u8; KEY_LEN]> {
        Zeroizing::new(derive(self.0.as_bytes(), INFO_SQLCIPHER))
    }

    /// `HKDF-SHA-256(vault_master_key, info="pg-audit-mac-v1")` — the audit chain MAC key
    /// (W5).
    #[must_use]
    pub fn audit_mac_key(&self) -> Zeroizing<[u8; KEY_LEN]> {
        Zeroizing::new(derive(self.0.as_bytes(), INFO_AUDIT_MAC))
    }
}

/// The AAD every wrapped master key is bound to: kind 6, not document-scoped
/// (data-model §5.9 "AAD kind 6"; architecture §3.1 AAD v1).
#[must_use]
pub fn wrapped_master_aad() -> crate::crypto::Aad {
    crate::crypto::Aad::global(
        crate::crypto::ArtifactKind::WrappedMaster,
        WRAPPED_MASTER_FORMAT_VERSION,
    )
}

/// Wrap `master` under a `wrap_key` derived from `passphrase` and fresh Argon2id
/// parameters, returning both halves of the resulting [`KeystoreItem`].
///
/// Used by first-run (architecture §3.4) and by change-passphrase (§3.3), which are the
/// same operation over a different master key provenance: first-run generates one, change
/// re-wraps the existing one.
///
/// # Errors
/// [`KeyError::Kdf`] if the derivation fails.
pub fn wrap_master_key(
    passphrase: &str,
    master: &VaultMasterKey,
    params: &Argon2idParams,
) -> Result<crate::crypto::WrappedBlob, KeyError> {
    let wrap_key = derive_wrap_key(passphrase, params)?;
    crate::crypto::wrap(&wrap_key, master.as_bytes(), &wrapped_master_aad())
        .map_err(|_| KeyError::Kdf)
}

/// Recover the `vault_master_key` from a stored item, or `None` if the passphrase is
/// wrong.
///
/// `None` covers wrong passphrase, a tampered blob, and unusable stored parameters alike.
/// architecture §3.3: "Passphrase failure zeroizes and refuses (no partial open)" — the
/// caller gets no way to tell those cases apart, and no partially derived material.
#[must_use]
pub fn unwrap_master_key(passphrase: &str, item: &KeystoreItem) -> Option<VaultMasterKey> {
    let wrap_key = derive_wrap_key(passphrase, &item.kdf).ok()?;
    let plaintext = Zeroizing::new(
        crate::crypto::unwrap(&wrap_key, &item.wrapped_master_key, &wrapped_master_aad()).ok()?,
    );
    let bytes: [u8; KEY_LEN] = plaintext.as_slice().try_into().ok()?;
    Some(VaultMasterKey::from_bytes(bytes))
}
