//! Envelope crypto primitives (chunk W1).
//!
//! Implements the leaf primitives of the key hierarchy in
//! `docs/specs/architecture.md` §3.1:
//!
//! ```text
//! passphrase (user, never stored)
//!   └─ Argon2id ──► wrap_key                       [W2, not here]
//!         └─ AEAD wrap ──► vault_master_key        [wrap/unwrap, here]
//!               ├─ HKDF-SHA-256 "pg-db-v1"         [derive, here]
//!               ├─ HKDF-SHA-256 "pg-audit-mac-v1"  [derive, here]
//!               └─ AEAD wrap of per-artifact DEKs  [Dek + wrap/unwrap, here]
//! ```
//!
//! **Scope discipline (dev-plan.md §1, W1).** This module is pure in-memory
//! crypto: no disk I/O, no SQLCipher, no OS keystore, no Argon2id /
//! passphrase-to-`wrap_key` derivation, and no Tauri command. Those belong to
//! W2/W3 and must not be pulled forward into this file.
//!
//! **Mutation gate.** `docs/specs/testing.md` §5.3 lists "Envelope AAD
//! length-prefixing" and the DEK destroy helpers as PR-blocking gated TCB
//! modules (S = 1.00, no unexplained survivors). Every branch here is expected
//! to be constrained by a test in `core/tests/crypto_w1.rs`.
//!
//! No hand-rolled crypto: `chacha20poly1305`, `hkdf`, `sha2`, `zeroize`,
//! `subtle`, `getrandom` (architecture §3.1 library list).

mod aad;
mod aead;
mod dek;
mod error;
mod kdf;

pub use aad::{Aad, ArtifactKind};
pub use aead::{unwrap, wrap, WrappedBlob, NONCE_LEN, TAG_LEN};
pub use dek::{Dek, DEK_LEN};
pub use error::CryptoError;
pub use kdf::{derive, HKDF_SALT, HKDF_OUTPUT_LEN};
