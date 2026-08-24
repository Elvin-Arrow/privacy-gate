//! Per-artifact data encryption keys (architecture.md §3.1, §4.3).
//!
//! > **Per-artifact DEK**: 256-bit, CSPRNG per stored object […] Wrapped by
//! > `vault_master_key` and stored beside the ciphertext. **Irrevocable delete
//! > (FR-4.6, NFR-R2) is cryptographic erasure:** the Vault destroys the wrapped
//! > DEK and the ciphertext.
//!
//! Because delete is *cryptographic* erasure, a DEK that outlives its intended
//! lifetime in process memory silently defeats FR-4.6. Hence `ZeroizeOnDrop`
//! and the explicit [`Dek::zeroize`] contract, both gated by `testing.md` §5.3
//! ("DEK destroy helpers").
//!
//! W1 scope: generate, hold, compare, destroy. Persistence of the *wrapped* DEK
//! is W3.

use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// DEK length: 256 bits (architecture §3.1).
pub const DEK_LEN: usize = 32;

/// A 256-bit data encryption key.
///
/// Zeroized on drop. `Debug` never renders key material. `Clone` is *not*
/// implemented on purpose: every copy is another place the bytes must be
/// destroyed before FR-4.6 erasure is honest.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Dek {
    bytes: [u8; DEK_LEN],
}

impl Dek {
    /// Generate a fresh DEK from the operating system CSPRNG.
    ///
    /// # Panics
    /// If the OS CSPRNG fails. Returning a predictable or partially initialized
    /// key would be a silent, unrecoverable compromise, so this fails loudly.
    /// Use [`Dek::try_generate`] to handle the failure explicitly.
    #[must_use]
    pub fn generate() -> Self {
        Self::try_generate().expect("system CSPRNG must be available to generate a DEK")
    }

    /// Fallible form of [`Dek::generate`].
    ///
    /// # Errors
    /// [`super::CryptoError::Rng`] if the system CSPRNG fails. Never falls back
    /// to a weaker source.
    pub fn try_generate() -> Result<Self, super::CryptoError> {
        let mut bytes = [0u8; DEK_LEN];
        match getrandom::fill(&mut bytes) {
            Ok(()) => Ok(Self { bytes }),
            Err(_) => {
                bytes.zeroize();
                Err(super::CryptoError::Rng)
            }
        }
    }

    /// Adopt existing key bytes — used when unwrapping a stored DEK (W3).
    #[must_use]
    pub fn from_bytes(bytes: [u8; DEK_LEN]) -> Self {
        Self { bytes }
    }

    /// Borrow the raw key bytes for a wrap/unwrap call.
    ///
    /// Do not copy the returned slice into a container that is not itself
    /// zeroized.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; DEK_LEN] {
        &self.bytes
    }

    /// Constant-time equality. Never compare key material with `==`.
    #[must_use]
    pub fn ct_eq_dek(&self, other: &Dek) -> bool {
        self.bytes.ct_eq(&other.bytes).into()
    }

    /// True once the key material has been destroyed. Constant-time.
    ///
    /// A zeroized DEK is unusable: it no longer opens anything it sealed.
    #[must_use]
    pub fn is_zeroized(&self) -> bool {
        self.bytes.ct_eq(&[0u8; DEK_LEN]).into()
    }
}

impl core::fmt::Debug for Dek {
    /// Never renders key bytes (architecture §3.2: secrets never reach logs).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Dek(redacted)")
    }
}
