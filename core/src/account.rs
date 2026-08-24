//! Local account record (`data-model.md` §5.6, `architecture.md` §7).
//!
//! > v1 accounts are **local-only**. First-run does not contact a server. No email, no
//! > network identity, no remote account id.
//!
//! ```text
//! LocalAccount {
//!   id: AccountId,             // UUID, generated on device
//!   display_name: String,      // user-chosen; not a secret; 1..=80 trimmed (api.md)
//!   created_at: Timestamp,
//! }
//! ```
//!
//! # Where this lives
//!
//! data-model §5.6 puts `LocalAccount` in the SQLCipher vault ("`display_name` is
//! SQLCipher-only, not envelope-encrypted"). **W3 owns that database**, so W2 keeps the
//! record behind an [`AccountStore`] trait with a process-local implementation and W3
//! swaps in a SQLCipher-backed one without touching `SessionManager`. That is also why
//! `get_account` is `unlocked`-only in api.md §2: the record is inside the vault.
//!
//! Nothing here is reachable while locked. `first_run` vs `locked` is answered by the
//! keystore alone (see `crate::keystore` module docs).

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// data-model §5.6.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalAccount {
    /// Device-generated UUID. Not a network identity (architecture §7).
    pub id: String,
    /// User-chosen display name. Not a secret; trimmed, 1..=80 chars (api.md §5.1).
    pub display_name: String,
    /// RFC 3339 UTC timestamp.
    pub created_at: String,
}

/// Failure modes of an account store. Coarse and non-secret, like every other error class
/// in the core.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AccountStoreError {
    /// The store could not be read or written.
    Backend(&'static str),
}

impl core::fmt::Display for AccountStoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AccountStoreError::Backend(class) => write!(f, "account store failure: {class}"),
        }
    }
}

impl std::error::Error for AccountStoreError {}

/// Where the `LocalAccount` record is kept. v1 holds at most one (architecture §7).
///
/// W3 replaces the W2 implementation with a SQLCipher-backed one.
pub trait AccountStore: Send + Sync {
    /// Read the account record, if one exists.
    fn load(&self) -> Result<Option<LocalAccount>, AccountStoreError>;
    /// Write (or overwrite) the account record.
    fn store(&self, account: &LocalAccount) -> Result<(), AccountStoreError>;
    /// Remove the account record. Idempotent.
    fn delete(&self) -> Result<(), AccountStoreError>;
}

/// Process-local account record. W2 only — see the module docs.
#[derive(Debug, Default)]
pub struct InMemoryAccountStore {
    slot: Mutex<Option<LocalAccount>>,
}

impl InMemoryAccountStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl AccountStore for InMemoryAccountStore {
    fn load(&self) -> Result<Option<LocalAccount>, AccountStoreError> {
        Ok(self
            .slot
            .lock()
            .map_err(|_| AccountStoreError::Backend("poisoned"))?
            .clone())
    }

    fn store(&self, account: &LocalAccount) -> Result<(), AccountStoreError> {
        *self
            .slot
            .lock()
            .map_err(|_| AccountStoreError::Backend("poisoned"))? = Some(account.clone());
        Ok(())
    }

    fn delete(&self) -> Result<(), AccountStoreError> {
        *self
            .slot
            .lock()
            .map_err(|_| AccountStoreError::Backend("poisoned"))? = None;
        Ok(())
    }
}

/// A fresh device-generated account id (data-model §5.6: "UUID, generated on device").
///
/// UUID v4 from the OS CSPRNG. No hostname, MAC address, or counter goes into it —
/// architecture §7 forbids anything that could act as a cross-device identifier.
#[must_use]
pub fn new_account_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The current time as an RFC 3339 UTC timestamp, e.g. `2026-08-23T12:34:56Z`.
///
/// Hand-rolled rather than pulling a date-time crate: v1 needs exactly one format, in
/// UTC, at second resolution, and a smaller dependency surface is worth more than the
/// convenience here.
#[must_use]
pub fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    format_rfc3339(secs)
}

/// Render Unix seconds as RFC 3339 UTC.
///
/// Uses Howard Hinnant's `civil_from_days` — the standard proleptic-Gregorian conversion,
/// correct across leap years and century rules.
#[must_use]
pub fn format_rfc3339(unix_secs: i64) -> String {
    let days = unix_secs.div_euclid(86_400);
    let secs_of_day = unix_secs.rem_euclid(86_400);

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y,
        m,
        d,
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known values for the civil-from-days conversion, including a leap day and a
    /// century boundary.
    #[test]
    fn rfc3339_known_values() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(1), "1970-01-01T00:00:01Z");
        assert_eq!(format_rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_rfc3339(1_709_164_800), "2024-02-29T00:00:00Z");
        assert_eq!(format_rfc3339(1_767_225_599), "2025-12-31T23:59:59Z");
        assert_eq!(format_rfc3339(1_756_857_600), "2025-09-03T00:00:00Z");
    }

    #[test]
    fn now_is_after_the_specs_were_written() {
        // Sanity: 2026-01-01T00:00:00Z.
        assert!(now_rfc3339().as_str() > "2026-01-01T00:00:00Z");
        assert!(now_rfc3339().ends_with('Z'));
    }
}
