//! Crypto error type.
//!
//! Deliberately coarse: a caller learns *that* an envelope failed to open, not
//! *why*. Distinguishing "wrong key" from "wrong AAD" from "bad tag" would be a
//! decryption oracle. `architecture.md` §3.3 ("passphrase failure zeroizes and
//! refuses — no partial open").

use core::fmt;

/// Failure modes of the envelope primitives. All are fail-closed; none of the
/// primitives in this module panic on attacker-controlled input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CryptoError {
    /// AAD bytes are not a well-formed AAD v1 record (architecture §3.1), or a
    /// field exceeds what the wire format can represent.
    ///
    /// Covers: wrong `aad_version`, unknown `artifact_kind`, a `doc_id_len`
    /// that overruns the buffer, a non-UTF-8 `doc_id`, truncation, and trailing
    /// bytes.
    MalformedAad,

    /// The wrapped blob is structurally invalid (nonce is not 24 bytes, or the
    /// ciphertext is shorter than the Poly1305 tag) and therefore cannot be
    /// authentic. Rejected before any AEAD work.
    MalformedBlob,

    /// AEAD authentication failed: wrong key, wrong AAD, or tampered
    /// nonce/ciphertext/tag. Intentionally not broken down further.
    Decrypt,

    /// The operating system CSPRNG failed. Never silently degrade to a weaker
    /// source of randomness.
    Rng,
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            CryptoError::MalformedAad => "malformed AAD v1 record",
            CryptoError::MalformedBlob => "malformed wrapped blob",
            CryptoError::Decrypt => "AEAD authentication failed",
            CryptoError::Rng => "system CSPRNG failure",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for CryptoError {}
