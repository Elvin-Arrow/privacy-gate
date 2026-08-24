//! `pg-core` — the Privacy Gate Rust core library.
//!
//! Specs live in `docs/specs/`. Implementation sequence in `docs/dev-plan.md`.
//!
//! - [`crypto`] — envelope primitives: AAD v1, XChaCha20-Poly1305 wrap/unwrap,
//!   HKDF-SHA-256 labels, DEKs (W1, architecture §3.1).
//! - [`keys`] — Key Manager material: Argon2id passphrase → `wrap_key`, the live
//!   `vault_master_key` and its labelled subkeys (W2, architecture §3.1).
//! - [`keystore`] — the `KeystoreItem` and its backends: OS keystore, Linux `0600`
//!   fallback, in-memory mock (W2, architecture §3.2).
//! - [`account`] — the local-only `LocalAccount` record (W2, architecture §7).
//! - [`session`] — the in-process session and account commands (W2, api.md §2, §5.1).
//! - [`api`] — the shared `ApiError` / `ErrorCode` model (api.md §3).
//! - [`vault`] — the SQLCipher database: open/create with the raw `sqlcipher_key`, the v1
//!   schema, and the `LocalAccount` store backed by it (W3, architecture §4, data-model §7).
//! - [`audit`] — the audit chain: canonical encoding v1, append, and replay verification
//!   against a persisted `AuditHead` (W5, architecture §6, data-model §5.8–§5.9).
//! - [`config`] — the global `Config` artifact: retention default + confirmation,
//!   detector preference (field only; no command yet) (W6, data-model §5.5).
//! - [`importer`] — plain-text and PDF extraction: bytes to in-memory
//!   `Document`/`Page`/`TextSpan` (W8/W9, design §2.1/§3.1, data-model §5.1). Library only;
//!   no command yet.
//! - [`catalog`] — the document catalog: `DocumentMeta`/`OriginalRecord` envelope storage,
//!   `DetectedField` (W10, data-model §5.1, §6.1–§6.2).
//! - [`detector`] — Detector host + stub: `StubDetector` is `SessionManager`'s default
//!   (W12, design §2.2). Real patterns/ONNX/Ollama are W13/W15.

pub mod account;
pub mod api;
pub mod audit;
pub mod catalog;
pub mod config;
pub mod crypto;
pub mod detector;
pub mod importer;
pub mod keys;
pub mod keystore;
pub mod session;
pub mod vault;
