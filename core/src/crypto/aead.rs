//! Envelope AEAD — XChaCha20-Poly1305 (architecture.md §3.1).
//!
//! > **AEAD**: XChaCha20-Poly1305. Random 24-byte nonce stored with the
//! > ciphertext. Additional authenticated data is length-prefixed to avoid
//! > concatenation collisions.
//!
//! XChaCha20 (not AES-GCM, not ChaCha20-Poly1305) because the 192-bit nonce
//! makes random per-message nonces safe at the volumes v1 wraps
//! (decision 0004: "large random nonces, no GCM nonce-reuse footgun").
//!
//! Nothing here touches disk. The caller (W2/W3) persists `nonce` and
//! `ciphertext` into `artifact.nonce` / `artifact.ciphertext`
//! (data-model.md §7).

use chacha20poly1305::aead::{Aead as _, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

use super::aad::Aad;
use super::error::CryptoError;

/// XChaCha20-Poly1305 nonce length. `data-model.md` §7: `nonce BLOB -- 24 bytes`.
pub const NONCE_LEN: usize = 24;

/// Poly1305 authentication tag length, appended to the ciphertext.
pub const TAG_LEN: usize = 16;

/// AEAD key length (256-bit, architecture §3.1).
const KEY_LEN: usize = 32;

/// A wrapped (sealed) blob: the random nonce and the ciphertext-with-tag.
///
/// Maps 1:1 onto the `artifact.nonce` / `artifact.ciphertext` columns
/// (data-model.md §7) so W2/W3 can persist it without reshaping.
#[derive(Clone, PartialEq, Eq)]
pub struct WrappedBlob {
    /// 24-byte XChaCha20 nonce, freshly random per wrap call.
    pub nonce: Vec<u8>,
    /// Ciphertext with the 16-byte Poly1305 tag appended.
    pub ciphertext: Vec<u8>,
}

impl core::fmt::Debug for WrappedBlob {
    /// Lengths only. Ciphertext is not secret, but dumping artifact bytes into
    /// logs is exactly the habit architecture §5 forbids.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WrappedBlob")
            .field("nonce_len", &self.nonce.len())
            .field("ciphertext_len", &self.ciphertext.len())
            .finish()
    }
}

/// Seal `plaintext` under `key`, bound to `aad`.
///
/// Generates a fresh 24-byte nonce from the OS CSPRNG on every call — never
/// reuse a nonce with the same key.
///
/// # Errors
/// [`CryptoError::Rng`] if the system CSPRNG fails, [`CryptoError::Decrypt`] if
/// the AEAD itself refuses the input (message too long). Never panics.
pub fn wrap(key: &[u8; KEY_LEN], plaintext: &[u8], aad: &Aad) -> Result<WrappedBlob, CryptoError> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce_bytes).map_err(|_| CryptoError::Rng)?;

    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| CryptoError::MalformedBlob)?;
    let nonce = XNonce::from(nonce_bytes);
    let aad_bytes = aad.encode();

    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: &aad_bytes,
            },
        )
        .map_err(|_| CryptoError::Decrypt)?;

    Ok(WrappedBlob {
        nonce: nonce_bytes.to_vec(),
        ciphertext,
    })
}

/// Open a blob sealed by [`wrap`], verifying the Poly1305 tag over both the
/// ciphertext and the encoded `aad`.
///
/// A wrong key, a wrong AAD, or any flipped bit in nonce/ciphertext/tag fails
/// with [`CryptoError::Decrypt`]. There is no partial or unauthenticated
/// output path.
///
/// # Errors
/// [`CryptoError::MalformedBlob`] if the blob cannot be structurally valid,
/// [`CryptoError::Decrypt`] on authentication failure. Never panics.
pub fn unwrap(
    key: &[u8; KEY_LEN],
    wrapped: &WrappedBlob,
    aad: &Aad,
) -> Result<Vec<u8>, CryptoError> {
    // Structural checks first, so attacker-controlled lengths can never reach a
    // slicing operation that could panic.
    if wrapped.nonce.len() != NONCE_LEN {
        return Err(CryptoError::MalformedBlob);
    }
    if wrapped.ciphertext.len() < TAG_LEN {
        return Err(CryptoError::MalformedBlob);
    }

    let mut nonce_bytes = [0u8; NONCE_LEN];
    nonce_bytes.copy_from_slice(&wrapped.nonce);
    let nonce = XNonce::from(nonce_bytes);

    let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| CryptoError::MalformedBlob)?;
    let aad_bytes = aad.encode();

    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: &wrapped.ciphertext,
                aad: &aad_bytes,
            },
        )
        .map_err(|_| CryptoError::Decrypt)
}
