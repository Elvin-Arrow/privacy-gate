//! The audit chain (`architecture.md` §6, `data-model.md` §5.8–§5.9).
//!
//! W5 delivers: append-only rows with the canonical HMAC encoding v1, chain replay and
//! verification against a persisted `AuditHead`, and the three architecture §6.3 unlock
//! outcomes (clean, crash-window fast-forward, integrity failure). `crate::session` owns
//! *when* verification runs (on `unlock`) and what happens to `SessionState` as a result;
//! this module owns the encoding, the hashing/HMAC, and the verification algorithm itself.
//!
//! # Scope fence (dev-plan.md W5)
//!
//! No concrete `EventPayload` variants for import/detect/approve/share/etc. — those
//! commands don't exist yet, and data-model §5.8.1's payload shapes are theirs to produce,
//! not this chunk's. [`append`] takes an already-canonicalized (RFC 8785 JCS) payload
//! string; a future chunk's command handler builds that string from its own typed payload
//! before calling this module. Nothing here writes an audit row from a command — "every
//! later mutating command must append audit" (dev-plan W5 "Integrate") is enforced as
//! those commands land, not retrofitted onto commands that don't exist.
//!
//! No UI integrity screen (W35), no user-initiated vault restore.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

use crate::keystore::AuditHead;

/// architecture §6.1: "Canonical encoding v1 (bit-stable; changing it is an architecture
/// amendment)".
pub const ENCODING_VERSION: u8 = 1;

/// architecture §6.1: "Genesis uses a fixed 32-byte zero digest" — both the `prev_entry_hash`
/// of sequence 1 and `AuditHead::GENESIS.head_hash` (`crate::keystore`).
pub const GENESIS_DIGEST: [u8; 32] = [0u8; 32];

/// data-model §5.8.1 / §7 SQL comment: `event_type` u8 codes, 1..6.
///
/// `Serialize`/`Deserialize` (W28) use the api.md §5.8 wire strings (`"import"`,
/// `"detect"`, `"approve"`, `"share"`, `"discard_original"`, `"delete"`) — the same
/// strings `AuditEventDto.event_type` and `list_audit_events`'s `event_type` filter use.
/// This is the one mapping; `crate::session` reuses it rather than inventing a second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum EventType {
    Import = 1,
    Detect = 2,
    Approve = 3,
    Share = 4,
    DiscardOriginal = 5,
    Delete = 6,
}

impl EventType {
    /// The wire byte (architecture §6.1 canonical encoding; data-model §7 SQL column).
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// # Errors
    /// [`AuditError::Backend`] if `v` is not one of the six defined codes — a corrupt or
    /// tampered row, not a recognized event.
    pub fn from_u8(v: u8) -> Result<Self, AuditError> {
        match v {
            1 => Ok(EventType::Import),
            2 => Ok(EventType::Detect),
            3 => Ok(EventType::Approve),
            4 => Ok(EventType::Share),
            5 => Ok(EventType::DiscardOriginal),
            6 => Ok(EventType::Delete),
            _ => Err(AuditError::Backend("unknown event_type code")),
        }
    }
}

/// data-model §5.8: on-disk `originals_flag` — "0 unset, 1 false, 2 true." Share events use
/// only `False`/`True`; every other event type is `Unset`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OriginalsFlag {
    Unset = 0,
    False = 1,
    True = 2,
}

impl OriginalsFlag {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// # Errors
    /// [`AuditError::Backend`] if `v` is not 0, 1, or 2.
    pub fn from_u8(v: u8) -> Result<Self, AuditError> {
        match v {
            0 => Ok(OriginalsFlag::Unset),
            1 => Ok(OriginalsFlag::False),
            2 => Ok(OriginalsFlag::True),
            _ => Err(AuditError::Backend("unknown originals_flag code")),
        }
    }
}

/// One row of the audit chain, as stored (data-model §5.8 `AuditEntry`, architecture §6.1
/// canonical encoding).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub sequence: u64,
    pub event_type: EventType,
    pub doc_id: Option<String>,
    pub produced_at_unix_ms: u64,
    pub originals_flag: OriginalsFlag,
    /// RFC 8785 JCS UTF-8 text. This module does not parse it — canonical encoding only
    /// needs its length and bytes (architecture §6.1: "`[u8; payload_len]` EventPayload as
    /// UTF-8 RFC 8785 JCS").
    pub payload_jcs: String,
    pub prev_entry_hash: [u8; 32],
    pub entry_signature: [u8; 32],
}

/// Failure modes of the audit backend. Coarse and non-secret (api.md §3 / C-API-5: "No
/// command returns keystore material, DEKs, HMAC bytes, or SQLCipher keys") — same
/// discipline as every other error class in the core.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuditError {
    Backend(&'static str),
}

impl core::fmt::Display for AuditError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            AuditError::Backend(class) => write!(f, "audit backend failure: {class}"),
        }
    }
}

impl std::error::Error for AuditError {}

/// Where audit rows live. `crate::vault::SqlCipherVault` implements this over the
/// `audit_entry` table W3's schema already creates.
pub trait AuditStore: Send + Sync {
    /// Append one already-signed row. Callers use [`append`], not this directly, so that
    /// `prev_entry_hash`/`entry_signature` are always computed by this module rather than
    /// supplied by a caller who could get the chain math wrong.
    ///
    /// # Errors
    /// [`AuditError::Backend`] on any I/O/backend failure.
    fn append_row(&self, row: &AuditRow) -> Result<(), AuditError>;

    /// Every row, ordered by `sequence` ascending. architecture §6.3: "On every unlock, the
    /// Audit Trail replays the chain from genesis."
    ///
    /// # Errors
    /// [`AuditError::Backend`] on any I/O/backend failure, including a row whose stored
    /// `event_type`/`originals_flag` code or hash/signature length is invalid — a
    /// corrupt/tampered row surfaces as an error here, not as a `None`.
    fn replay(&self) -> Result<Vec<AuditRow>, AuditError>;
}

// ---------------------------------------------------------------------------
// Canonical encoding v1, hashing, HMAC (architecture §6.1)
// ---------------------------------------------------------------------------

/// architecture §6.1's exact byte layout, excluding `entry_signature` (the signature is
/// computed *over* this, and the next entry's `prev_entry_hash` is SHA-256 of this same
/// byte string for the entry it points at).
///
/// `doc_id_len` and `payload_len` are declared `u16`/`u32` in the wire format; the
/// truncating `as` casts below are safe in practice (a `DocId` and a JCS payload are both
/// far under those ceilings by construction elsewhere in the system) and are exactly the
/// kind of silent-corruption risk the mutation gate (testing.md §5.3 lists this encoding)
/// exists to catch if that assumption is ever violated.
#[must_use]
pub fn canonical_bytes(row: &AuditRow) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64 + row.payload_jcs.len());
    buf.push(ENCODING_VERSION);
    buf.extend_from_slice(&row.sequence.to_be_bytes());
    buf.push(row.event_type.as_u8());
    buf.extend_from_slice(&row.produced_at_unix_ms.to_be_bytes());
    match &row.doc_id {
        Some(id) => {
            buf.push(1);
            let bytes = id.as_bytes();
            buf.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            buf.extend_from_slice(bytes);
        }
        None => {
            buf.push(0);
            buf.extend_from_slice(&0u16.to_be_bytes());
        }
    }
    buf.push(row.originals_flag.as_u8());
    let payload = row.payload_jcs.as_bytes();
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);
    buf.extend_from_slice(&row.prev_entry_hash);
    buf
}

/// SHA-256 of `bytes`.
#[must_use]
fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

type HmacSha256 = Hmac<Sha256>;

/// HMAC-SHA-256(`key`, `bytes`) (architecture §6.1: "HMAC (not an asymmetric signature) is
/// the v1 primitive").
#[must_use]
fn hmac_sha256(key: &[u8; 32], bytes: &[u8]) -> [u8; 32] {
    // `Hmac::new_from_slice` only fails for a key length its `KeySize` bound rejects;
    // `Sha256`'s HMAC accepts any length, and this key is always exactly 32 bytes.
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC-SHA-256 accepts a 32-byte key");
    mac.update(bytes);
    mac.finalize().into_bytes().into()
}

/// An entry's own `head_hash` — what a persisted [`AuditHead`] records for it, and exactly
/// what the *next* entry's `prev_entry_hash` must equal (architecture §6.1/§6.2). One
/// function for both, so the two can never silently diverge.
#[must_use]
fn head_hash_of(row: &AuditRow) -> [u8; 32] {
    sha256(&canonical_bytes(row))
}

/// The [`AuditHead`] a caller should persist if it wants `row` to be the new tip —
/// `crate::session` uses this after each successful [`append`] (architecture §6.2's
/// "every 32 appends" cadence; the on-lock persist) so nothing outside this module ever
/// hand-computes a head hash a second, possibly-inconsistent way.
#[must_use]
pub fn head_for(row: &AuditRow) -> AuditHead {
    AuditHead {
        sequence: row.sequence,
        head_hash: head_hash_of(row),
    }
}

// ---------------------------------------------------------------------------
// Append (architecture §6.1)
// ---------------------------------------------------------------------------

/// Append one signed row to `store`, chained onto its current tail (or genesis if empty).
///
/// This is the **only** place `prev_entry_hash`/`entry_signature` are computed — callers
/// never construct an [`AuditRow`] by hand for a real append, which is what makes "an
/// attacker who edits the DB without the vault key cannot produce a valid HMAC"
/// (architecture §6.1) also true of this codebase's own honest write path: there is no
/// second code path that could accidentally sign something inconsistently.
///
/// # Errors
/// Whatever `store.replay()` or `store.append_row()` return.
pub fn append(
    store: &dyn AuditStore,
    mac_key: &[u8; 32],
    event_type: EventType,
    doc_id: Option<&str>,
    produced_at_unix_ms: u64,
    originals_flag: OriginalsFlag,
    payload_jcs: &str,
) -> Result<AuditRow, AuditError> {
    // architecture §6.1's `doc_id_len`/`payload_len` fields are `u16`/`u32`. Reject
    // oversized input here rather than let `canonical_bytes`' `as u16`/`as u32` casts
    // silently wrap it into a length prefix inconsistent with the bytes that follow —
    // that would be a canonical-encoding ambiguity, which "bit-stable" (architecture
    // §6.1) forbids. Neither ceiling is reachable in practice (a `DocId` is a UUID; a JCS
    // payload for the event types this module knows about is nowhere near 4 GiB), so this
    // is a defensive fail-closed check, not a real operating constraint.
    if doc_id.is_some_and(|id| id.len() > usize::from(u16::MAX)) {
        return Err(AuditError::Backend("doc_id too long for canonical encoding"));
    }
    if payload_jcs.len() > u32::MAX as usize {
        return Err(AuditError::Backend("payload too long for canonical encoding"));
    }

    let existing = store.replay()?;
    let (sequence, prev_entry_hash) = match existing.last() {
        Some(tail) => (tail.sequence + 1, head_hash_of(tail)),
        None => (1, GENESIS_DIGEST),
    };

    let mut row = AuditRow {
        sequence,
        event_type,
        doc_id: doc_id.map(String::from),
        produced_at_unix_ms,
        originals_flag,
        payload_jcs: payload_jcs.to_string(),
        prev_entry_hash,
        entry_signature: [0u8; 32], // placeholder; canonical_bytes excludes it either way
    };
    row.entry_signature = hmac_sha256(mac_key, &canonical_bytes(&row));

    store.append_row(&row)?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// Verification (architecture §6.3)
// ---------------------------------------------------------------------------

/// architecture §6.3's crash-window ceiling: "`T.sequence == H.sequence + k` for k in
/// 1..32".
pub const CRASH_WINDOW_MAX: u64 = 32;

/// Why a chain failed to verify cleanly against the persisted head — feeds
/// `crate::session::IntegrityReport.kind` (`"truncation"` | `"modification"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// `T.sequence < H.sequence`: the persisted head points past the end of what replays.
    Truncation,
    /// Anything else architecture §6.3 calls a failure: a broken HMAC/chain link
    /// mid-replay, or a chain that is internally self-consistent but does not reach (or
    /// exceeds by more than [`CRASH_WINDOW_MAX`]) the persisted head, or whose entry at
    /// `H.sequence` does not hash to `H.head_hash`.
    Modification,
}

/// The three architecture §6.3 unlock outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// `T == H`. Open normally; nothing to persist.
    Clean,
    /// Expected crash window: DB committed, keystore persist not yet done. Open normally;
    /// the caller must persist `new_head` (architecture §6.3: "Fast-forward `audit_head`
    /// to `T`").
    FastForward { new_head: AuditHead },
    /// Integrity failure: no document decrypt, degraded session.
    Failure {
        kind: FailureKind,
        /// First sequence number that failed verification, where one specific sequence is
        /// identifiable. `None` when the failure is a head/tail relationship rather than a
        /// single bad row (e.g. a chain that is internally valid throughout but does not
        /// reach the persisted head at all).
        first_bad_sequence: Option<u64>,
        /// The last sequence number that verified cleanly from genesis — api.md's
        /// `IntegrityReport.tail_sequence` ("verified tail sequence of the DB chain").
        /// Stated explicitly here rather than left for the caller to re-derive from a raw
        /// row count, which would silently overcount whenever rows exist *after* a
        /// mid-chain break.
        verified_tail_sequence: u64,
    },
}

/// Replay-verify `rows` (already ordered by `sequence`, e.g. from
/// [`AuditStore::replay`]) against the persisted `head`, and classify the result per
/// architecture §6.3.
///
/// Every row's `prev_entry_hash`, `sequence` contiguity, and `entry_signature` are checked
/// against `mac_key` in one linear pass. An attacker without `mac_key` cannot produce a row
/// that passes the `entry_signature` check, so any tampering — a flipped payload byte, a
/// deleted middle row, a truncated tail, a wholesale rewrite — breaks verification at or
/// before the first point it touches; this function never needs to distinguish attacker
/// intent, only "does the replay match `mac_key`'s signatures, in order, from genesis."
#[must_use]
pub fn verify_against_head(rows: &[AuditRow], mac_key: &[u8; 32], head: AuditHead) -> VerifyOutcome {
    let mut expected_prev = GENESIS_DIGEST;
    let mut first_bad: Option<u64> = None;
    let mut verified_count: usize = 0;

    for (i, row) in rows.iter().enumerate() {
        let expected_sequence = i as u64 + 1;
        let sequence_ok = row.sequence == expected_sequence;
        let prev_ok = row.prev_entry_hash == expected_prev;
        let sig_ok = row.entry_signature == hmac_sha256(mac_key, &canonical_bytes(row));

        if !sequence_ok || !prev_ok || !sig_ok {
            first_bad = Some(expected_sequence);
            break;
        }
        expected_prev = head_hash_of(row);
        verified_count = i + 1;
    }

    if let Some(first_bad_sequence) = first_bad {
        // A break inside the replay itself: always Modification (architecture §6.3 lists
        // "HMAC/chain break" under the same "integrity failure" bucket as a head mismatch,
        // but a mid-chain break is never merely "fewer rows than expected" — Truncation is
        // reserved for a fully-valid-but-short chain, handled below).
        return VerifyOutcome::Failure {
            kind: FailureKind::Modification,
            first_bad_sequence: Some(first_bad_sequence),
            verified_tail_sequence: verified_count as u64,
        };
    }

    // The whole replay verified internally. Compare its tail to the persisted head.
    let tail_sequence = verified_count as u64;
    let tail_head_hash = rows.get(verified_count.wrapping_sub(1)).map_or(GENESIS_DIGEST, head_hash_of);

    if tail_sequence == head.sequence && tail_head_hash == head.head_hash {
        return VerifyOutcome::Clean;
    }

    if tail_sequence < head.sequence {
        return VerifyOutcome::Failure {
            kind: FailureKind::Truncation,
            // The persisted head claims a sequence the replay never reaches; the first
            // missing one is the most useful pointer for a report.
            first_bad_sequence: Some(tail_sequence + 1),
            verified_tail_sequence: tail_sequence,
        };
    }

    // tail_sequence > head.sequence: either a legitimate crash window, or too large a gap
    // / a head that doesn't match what the replay actually has at that sequence.
    let k = tail_sequence - head.sequence;
    let head_entry_matches = if head.sequence == 0 {
        head.head_hash == GENESIS_DIGEST
    } else {
        // `head.sequence` is 1-based; rows[head.sequence - 1] is that entry, if present.
        usize::try_from(head.sequence)
            .ok()
            .and_then(|idx| rows.get(idx - 1))
            .is_some_and(|entry| head_hash_of(entry) == head.head_hash)
    };

    if (1..=CRASH_WINDOW_MAX).contains(&k) && head_entry_matches {
        return VerifyOutcome::FastForward {
            new_head: AuditHead {
                sequence: tail_sequence,
                head_hash: tail_head_hash,
            },
        };
    }

    VerifyOutcome::Failure {
        kind: FailureKind::Modification,
        first_bad_sequence: if head_entry_matches {
            None // internally valid, just too large a gap to fast-forward
        } else {
            Some(head.sequence)
        },
        verified_tail_sequence: tail_sequence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(sequence: u64, prev: [u8; 32], payload: &str) -> AuditRow {
        AuditRow {
            sequence,
            event_type: EventType::Import,
            doc_id: None,
            produced_at_unix_ms: 1000,
            originals_flag: OriginalsFlag::Unset,
            payload_jcs: payload.to_string(),
            prev_entry_hash: prev,
            entry_signature: [0u8; 32],
        }
    }

    #[test]
    fn canonical_bytes_layout_matches_architecture_6_1() {
        let r = row(7, [0xAB; 32], "{}");
        let bytes = canonical_bytes(&r);
        assert_eq!(bytes[0], 1, "encoding_version");
        assert_eq!(&bytes[1..9], &7u64.to_be_bytes(), "sequence");
        assert_eq!(bytes[9], EventType::Import.as_u8(), "event_type");
        assert_eq!(&bytes[10..18], &1000u64.to_be_bytes(), "produced_at_unix_ms");
        assert_eq!(bytes[18], 0, "doc_id_present = 0");
        assert_eq!(&bytes[19..21], &0u16.to_be_bytes(), "doc_id_len = 0");
        assert_eq!(bytes[21], OriginalsFlag::Unset.as_u8(), "originals_flag");
        assert_eq!(&bytes[22..26], &2u32.to_be_bytes(), "payload_len");
        assert_eq!(&bytes[26..28], b"{}", "payload bytes");
        assert_eq!(&bytes[28..60], &[0xABu8; 32], "prev_entry_hash");
        assert_eq!(bytes.len(), 60, "no trailing entry_signature bytes");
    }

    #[test]
    fn canonical_bytes_encodes_doc_id_when_present() {
        let mut r = row(1, GENESIS_DIGEST, "{}");
        r.doc_id = Some("abc".to_string());
        let bytes = canonical_bytes(&r);
        assert_eq!(bytes[18], 1, "doc_id_present = 1");
        assert_eq!(&bytes[19..21], &3u16.to_be_bytes(), "doc_id_len = 3");
        assert_eq!(&bytes[21..24], b"abc");
    }
}
