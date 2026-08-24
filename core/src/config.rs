//! Global `Config` — the envelope-encrypted `kind=4` artifact (`data-model.md` §5.5,
//! architecture §3.1, §4.2).
//!
//! W6 delivers `get_retention_default` / `set_retention_default` (api.md §5.2) and the
//! storage underneath them: `Config` is the **first** real envelope-encrypted artifact this
//! codebase writes to the `artifact` table (W3's schema; the SQLCipher layer only, no
//! envelope, was enough for `LocalAccount`). Two AEAD layers, exactly architecture §3.1's
//! diagram: a fresh per-write DEK wraps the plaintext JSON under AAD kind 4
//! (`ArtifactKind::Config`), and `vault_master_key` wraps that DEK under AAD kind 7
//! (`ArtifactKind::WrappedDek`) — the same two-layer shape `crate::keys` already uses for
//! the keystore's wrapped master key, just with an extra DEK indirection so this artifact
//! (unlike the master key) can be cryptographically erased independently later.
//!
//! # Scope fence (dev-plan.md W6 "Do not: first-import modal UI (W32); per-import override
//! (W10)")
//!
//! `detector_preference` is part of `Config`'s on-disk shape (data-model §5.5) so a later
//! chunk (W15c) never needs a format bump to add it — but no command in this chunk reads or
//! writes it. `import_document` does not exist yet, so nothing here gates an import; that
//! gate is W11.

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::{Aad, ArtifactKind, Dek, DEK_LEN};
use crate::keys::VaultMasterKey;

/// `format_version` bound into the AAD of both layers (config plaintext and its DEK wrap).
/// Bumping it is a storage format break for this artifact only.
pub const CONFIG_FORMAT_VERSION: u32 = 1;

/// data-model §5.5 / architecture §5.1: `"retain" | "discard" | "never_retain"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionPolicy {
    Retain,
    Discard,
    NeverRetain,
}

impl RetentionPolicy {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            RetentionPolicy::Retain => "retain",
            RetentionPolicy::Discard => "discard",
            RetentionPolicy::NeverRetain => "never_retain",
        }
    }
}

/// data-model §5.5 / decision 0009: `"auto" | "bundled_only"`. Not read or written by any
/// W6 command (see module scope fence); stored so the on-disk shape is final now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectorPreference {
    Auto,
    BundledOnly,
}

/// data-model §5.5 `Config`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub policy: RetentionPolicy,
    pub confirmed: bool,
    pub detector_preference: DetectorPreference,
}

impl Default for Config {
    /// decision 0007 / OQ-14: "Factory value of the global retention default is
    /// `discard`... unconfirmed until the user explicitly sets a policy." decision 0009:
    /// `detector_preference` factory is `"auto"`.
    fn default() -> Self {
        Self {
            policy: RetentionPolicy::Discard,
            confirmed: false,
            detector_preference: DetectorPreference::Auto,
        }
    }
}

/// Failure modes of the config backend. Coarse and non-secret, same discipline as every
/// other error class in the core.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    Backend(&'static str),
}

impl core::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ConfigError::Backend(class) => write!(f, "config backend failure: {class}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// Where the `Config` artifact lives. `crate::vault::SqlCipherVault` implements this over
/// the `artifact` table's unique `kind=4` row (`uq_artifact_config`, W3's schema).
///
/// Unlike `AccountStore`/`AuditStore`, both methods take the live `vault_master_key`: the
/// artifact is envelope-encrypted, not merely SQLCipher-protected, so every read/write
/// needs it to unwrap/wrap the artifact's DEK.
pub trait ConfigStore: Send + Sync {
    /// `None` if no config row exists yet (should not happen once `create_account` has run,
    /// but callers still default to [`Config::default`] rather than treat that as an error —
    /// same fail-open-to-factory-values posture as the keystore's `first_run` read).
    ///
    /// # Errors
    /// [`ConfigError::Backend`] on any I/O/backend/decrypt failure.
    fn load(&self, master: &VaultMasterKey) -> Result<Option<Config>, ConfigError>;

    /// Replace the config row. Always the whole object — there is no partial update at
    /// this layer (`SessionManager::set_retention_default` reads, mutates one field, then
    /// calls this with the full struct).
    ///
    /// # Errors
    /// [`ConfigError::Backend`] on any I/O/backend/encrypt failure.
    fn store(&self, master: &VaultMasterKey, config: &Config) -> Result<(), ConfigError>;
}

/// The W2–W5-era no-op backend: `load` reports "no row" (so callers see
/// [`Config::default`]), `store` errors (nothing to write to). Exists so every constructor
/// that predates W6 keeps working unmodified — see `crate::session::SessionManager::new`
/// and friends.
#[derive(Debug, Default)]
pub struct NullConfigStore;

impl ConfigStore for NullConfigStore {
    fn load(&self, _master: &VaultMasterKey) -> Result<Option<Config>, ConfigError> {
        Ok(None)
    }
    fn store(&self, _master: &VaultMasterKey, _config: &Config) -> Result<(), ConfigError> {
        Err(ConfigError::Backend("no config store configured"))
    }
}

/// AAD for the config plaintext layer (architecture §3.1, kind 4, not document-scoped).
#[must_use]
pub fn config_plaintext_aad() -> Aad {
    Aad::global(ArtifactKind::Config, CONFIG_FORMAT_VERSION)
}

/// AAD for the config artifact's DEK-wrap layer (architecture §3.1, kind 7, not
/// document-scoped — mirrors `crate::keys::wrapped_master_aad`'s shape one level down).
#[must_use]
pub fn config_dek_wrap_aad() -> Aad {
    Aad::global(ArtifactKind::WrappedDek, CONFIG_FORMAT_VERSION)
}

/// Encrypt `config` under a fresh DEK, then wrap that DEK under `master`. Returns
/// `(wrapped_dek, artifact_blob)` — both `crate::crypto::WrappedBlob`s, ready for a caller
/// to persist as `artifact.wrapped_dek`/`artifact.nonce`+`artifact.ciphertext` respectively
/// (the wrapped DEK is itself a nonce+ciphertext pair; `crate::vault` owns how those map
/// onto SQL columns).
///
/// # Errors
/// Whatever the underlying AEAD wrap calls return (CSPRNG failure; never a key/AAD
/// mismatch, since both AADs are constructed here).
pub fn seal_config(
    master: &VaultMasterKey,
    config: &Config,
) -> Result<(crate::crypto::WrappedBlob, crate::crypto::WrappedBlob), ConfigError> {
    let dek = Dek::generate();
    let plaintext = serde_json::to_vec(config).map_err(|_| ConfigError::Backend("serialize failed"))?;
    let artifact_blob = crate::crypto::wrap(dek.as_bytes(), &plaintext, &config_plaintext_aad())
        .map_err(|_| ConfigError::Backend("artifact wrap failed"))?;
    let wrapped_dek = crate::crypto::wrap(master.as_bytes(), dek.as_bytes(), &config_dek_wrap_aad())
        .map_err(|_| ConfigError::Backend("dek wrap failed"))?;
    Ok((wrapped_dek, artifact_blob))
}

/// The inverse of [`seal_config`]: unwrap the DEK under `master`, then unwrap the artifact
/// blob under the recovered DEK, then parse the JSON.
///
/// # Errors
/// [`ConfigError::Backend`] if either AEAD layer fails to authenticate, or the plaintext is
/// not valid `Config` JSON — a tampered or foreign-key row surfaces as an error, never as a
/// silently wrong `Config`.
pub fn open_config(
    master: &VaultMasterKey,
    wrapped_dek: &crate::crypto::WrappedBlob,
    artifact_blob: &crate::crypto::WrappedBlob,
) -> Result<Config, ConfigError> {
    let dek_bytes: Zeroizing<Vec<u8>> = Zeroizing::new(
        crate::crypto::unwrap(master.as_bytes(), wrapped_dek, &config_dek_wrap_aad())
            .map_err(|_| ConfigError::Backend("dek unwrap failed"))?,
    );
    let dek_array: [u8; DEK_LEN] = dek_bytes
        .as_slice()
        .try_into()
        .map_err(|_| ConfigError::Backend("dek has the wrong length"))?;
    let dek = Dek::from_bytes(dek_array);

    let plaintext = crate::crypto::unwrap(dek.as_bytes(), artifact_blob, &config_plaintext_aad())
        .map_err(|_| ConfigError::Backend("artifact unwrap failed"))?;
    serde_json::from_slice(&plaintext).map_err(|_| ConfigError::Backend("malformed config JSON"))
}
