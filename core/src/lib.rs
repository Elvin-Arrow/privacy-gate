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
//! - [`session`] — the in-process session and account commands (W2, api.md §2, §5.1),
//!   including `pg://detect-progress` (W14).
//! - [`api`] — the shared `ApiError` / `ErrorCode` model (api.md §3).
//! - [`vault`] — the SQLCipher database: open/create with the raw `sqlcipher_key`, the v1
//!   schema, and the `LocalAccount` store backed by it (W3, architecture §4, data-model §7).
//! - [`audit`] — the audit chain: canonical encoding v1, append, and replay verification
//!   against a persisted `AuditHead` (W5, architecture §6, data-model §5.8–§5.9).
//! - [`config`] — the global `Config` artifact: retention default + confirmation,
//!   detector preference (`get_detector_preference` / `set_detector_preference`, W15c)
//!   (W6/W15c, data-model §5.5).
//! - [`importer`] — plain-text and PDF extraction: bytes to in-memory
//!   `Document`/`Page`/`TextSpan` (W8/W9, design §2.1/§3.1, data-model §5.1). Library only;
//!   no command yet.
//! - [`catalog`] — the document catalog: `DocumentMeta`/`OriginalRecord` envelope storage,
//!   `DetectedField` (W10, data-model §5.1, §6.1–§6.2).
//! - [`detector`] — Detector host + stub (`StubDetector`, W12), pattern pack
//!   [`detector::PatternsUkV1`] (`pg-patterns-uk-v1`, W13), hybrid
//!   [`detector::HybridV1`] (`pg-hybrid-v1`, W15a), and optional Ollama host
//!   [`detector::HybridOllamaV1`] (`pg-hybrid-ollama-v1`, W15b). `import_document` selects
//!   between hybrid and Ollama per detect (W15c); `with_detector` installs the stub for
//!   AC-1..AC-4. `open_approval` / `get_approval_view` / `set_field_decisions` (W16) hold
//!   one RAM approval session. `submit_approval` (W18) writes the canonical
//!   `ApprovedVersion` and drops that session. `abort_approval` / lock (W19) drop
//!   unapproved discard catalog rows; retain may reopen after lock. `delete_document`
//!   (W20) overwrite-and-drops wrapped DEKs. `delete_retained_original` (W21) drops kind=2
//!   only. Variants (W22): `list_variants` / `get_variant` / `save_variant` /
//!   `delete_variant`.
//! - [`overlap`] — design §3.5 byte-offset redaction (innermost keep; partial overlap
//!   redact-wins) applied at `submit_approval` (W17/W18).
//! - [`export`] — from-scratch PDF writer from `redacted_content` (W23, architecture §11).
//! - [`share`] — export filename + PDF assembly (W24); preview/commit commands live on
//!   [`session`].

pub mod account;
pub mod api;
pub mod audit;
pub mod catalog;
pub mod config;
pub mod crypto;
pub mod detector;
pub mod export;
pub mod importer;
pub mod keys;
pub mod keystore;
pub mod overlap;
pub mod session;
pub mod share;
pub mod vault;
