//! HKDF-SHA-256 subkey derivation (architecture.md §3.1).
//!
//! > **HKDF**: HKDF-SHA-256 (RFC 5869). Salt = the ASCII `privacy-gate-hkdf-v1`.
//! > Info labels as in the diagram (`pg-db-v1`, `pg-audit-mac-v1`). Output
//! > length 32 bytes.
//!
//! This module is deliberately label-agnostic. W2 calls it with `pg-db-v1` to
//! get `sqlcipher_key`; W5 calls it with `pg-audit-mac-v1` to get
//! `audit_mac_key`. W1 does not own either call site — only the primitive.

use hkdf::Hkdf;
use sha2::Sha256;

/// Fixed HKDF salt (architecture §3.1). The literal ASCII string is normative;
/// the spec prose says "19-byte" but the string is 20 bytes — the string wins,
/// and the spec's byte count is a typo (noted in dev-log 0011).
pub const HKDF_SALT: &[u8; 20] = b"privacy-gate-hkdf-v1";

/// Output length of every v1 derivation: 256 bits.
pub const HKDF_OUTPUT_LEN: usize = 32;

/// Derive a 32-byte subkey from `ikm` under `info`, using HKDF-SHA-256 with the
/// project's fixed salt.
///
/// Distinct `info` labels give cryptographically independent subkeys, which is
/// what keeps `sqlcipher_key` and `audit_mac_key` from being interchangeable.
#[must_use]
pub fn derive(ikm: &[u8; 32], info: &str) -> [u8; HKDF_OUTPUT_LEN] {
    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), ikm);
    let mut okm = [0u8; HKDF_OUTPUT_LEN];
    // 32 bytes is far below HKDF-SHA-256's 255*32-byte ceiling, so `expand`
    // cannot fail for this fixed output length.
    hk.expand(info.as_bytes(), &mut okm)
        .expect("32-byte HKDF-SHA-256 output is always a valid length");
    okm
}
