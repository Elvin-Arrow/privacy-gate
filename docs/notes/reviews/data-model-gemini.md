# Data Model Specification Review: Privacy Gate v1

Reviewer: Gemini (via `agy --effort high`). Date: 2026-08-23.

(Raw Gemini output from the shared review prompt.)

## A. Alignment data-model → SRS

- **Vault & Storage Security (FR-4.1, FR-4.2, FR-4.3, FR-4.4, NFR-S1):** Fully aligned. §1, §3, and §6 establish the single local SQLCipher database (`vault.db`) with envelope encryption for all sensitive payloads. Document metadata (labels, offsets, filenames) is isolated in Kind 8 `DocumentMetaRecord` (§5.1) and encrypted under a dedicated DEK rather than exposed in unencrypted SQL columns (C-DM-2).
- **Single Canonical Approved Version (FR-3.2, Decision 0002):** Fully aligned. §4 and §6 enforce at most one `approved_artifact_id` per `document` record (`UNIQUE REFERENCES artifact(artifact_id)`). Re-importing a file produces a fresh `DocId` with no content-hash deduplication index (§2), preserving independent document lifecycles.
- **Variants (FR-5.5, OQ-7):** Fully aligned. §4, §5.4, and §6 model variants as child records referencing `document(doc_id)` with a `UNIQUE(doc_id, name)` constraint and Kind 3 `VariantRecord` payload envelopes containing only decision overrides.
- **Retention & Discard Policy (FR-1.4, Decision 0007):** Fully aligned. §5.5 implements factory default `policy="discard"` with `confirmed=false`. §3, §7, and §10 ensure discard-unapproved file bodies remain strictly in RAM and are purged on lock/abort without writing Kind 2 (`OriginalRecord`) rows.
- **Destructive Deletion (NFR-R2):** Fully aligned. §6 and §7 specify cryptographic erasure by removing/zeroizing `wrapped_dek` and `ciphertext` upon document/original deletion.
- **Audit Logging (FR-7, OQ-3):** Fully aligned. §6 and §8 define append-only `audit_entry` rows referencing monotonic sequences, storing RFC 8785 JCS payloads stripped of raw text.
- **Account & Identity (FR-8):** Fully aligned. §3 and §6 store local account metadata in `account`, keeping `display_name` inside SQLCipher without an unnecessary envelope layer.

## B. Alignment data-model → design

- **Core Design Structs:** §4 and §5 accurately map `Document`, `ApprovedVersion`, `Variant`, and `AuditEntry` to database and envelope representations.
- **Detected Fields & Spans:** §5.1 `DetectedFieldRecord` directly mirrors `DetectedField` from `design.md` §3.1 (offsets, lengths, page indexes, labels, hierarchy).
- **Post-Redaction Content Persistence:** §5.3 `ApprovedVersionRecord.redacted_content` stores materialized post-redaction span text, fulfilling `design.md` §3.2 (enabling the Share Engine to export approved documents without loading `OriginalRecord` or raw document bytes).
- **Ephemeral State Handling:** `ShareRequest`, share preview tokens, approval session state, and discard-unapproved byte caches are explicitly marked as RAM-only with zero database tables (§3, §10), aligning with `design.md` §3.7.
- **Re-import Semantics:** §2 conforms to `design.md` §3.6 by requiring a new UUID `DocId` per import and explicitly prohibiting uniqueness indexes on file hashes.

## C. Alignment data-model → architecture

- **Envelope Cryptography & AAD Kinds:** §5 and §6 align with `architecture.md` §3.1 and Decision 0004. Kinds 1–5 and 8 are correctly assigned to SQL rows, Kind 6 (`wrapped_master`) is kept in the OS Keystore (§9), and Kind 7 (`wrapped_dek`) is properly restricted to wrap AAD rather than SQL row storage (C-DM-3).
- **Keystore Representation:** §9 `KeystoreItem` accurately reflects `architecture.md` §3.2, capturing Argon2id KDF parameters, salt, audit head state, and the AAD Kind 6 wrapped master key.
- **Audit Cryptographic Integrity:** §6 and §8 preserve the separation of concerns outlined in `architecture.md` §6. Canonical byte serialization for HMAC computation remains governed by architecture §6.1, while `data-model.md` defines only the on-disk column layout and RFC 8785 JCS payload contents.
- **Unencrypted vs Encrypted Boundaries:** §3 and §6 align with `architecture.md` §7: `account.display_name` resides in plain SQLCipher columns (since whole-DB encryption applies and it is not a secret), while sensitive document metadata is wrapped in Kind 8 envelopes.

## D. Alignment data-model → API (incl. decision 0007)

- **DTOs as Projections:** §1 and §13 confirm API DTOs are direct projections of envelope records and audit rows rather than a divergent schema.
- **Decision 0007 Enforcement:** §5.5 and §7 define the factory state as `policy="discard"` with `confirmed=false`, preventing imports until the user confirms or updates retention preferences.
- **Lock / Abort Lifecycle:** §5.1, §7, and §10 align with `api.md` command lifecycles: aborting or locking during an unapproved discard session purges the `document` row and Kind 8 artifact from SQLCipher and wipes RAM.
- **Redaction & Variant Queries:** §5.4 aligns with `api.md` `get_variant` by storing only `field_id` override decisions without duplicating span text.
- **Plugin Secret Exposure:** §5.6 stores `key_last4` in `CloudAiSecretRecord`, allowing `cloud_ai_get_config` to return masked keys without decrypt-and-slice runtime complexity.
- **Audit Payloads:** §8 mirrors `api.md` §5.8 event shapes, omitting field text and credentials from all logged events.

## E. Alignment ↔ idea (local-first, later sync, no hosted DB)

- **Local-First Architecture:** The data model is strictly embedded (`vault.db` via SQLCipher), requiring no central authority, remote registry, or hosted database (§1, §3).
- **Later Sync Readiness:** §11 models each envelope artifact (`artifact` table with kinds 1–5, 8) as an opaque, self-contained ciphertext unit (`wrapped_dek`, `nonce`, `ciphertext`) capable of being synchronized across devices without server-side decryption.
- **No Premature Sync Bloat:** §11 and §14 maintain strict discipline by omitting speculative sync columns, vector clocks, multi-device conflict tables, or key migration schemes from v1.

## F. Quality / implementability

- **Relational Integrity & Typing:** §6 provides well-defined tables with explicit SQLite types (`TEXT` for canonical UUID strings, `INTEGER` for Unix millisecond timestamps and enums, `BLOB` for crypto payloads). Foreign keys are properly established across `document`, `variant`, and `plugin_secret`.
- **Append-Only Audit Independence:** `audit_entry.doc_id` correctly omits a foreign key constraint to `document(doc_id)` (§6), ensuring document deletions do not break referential integrity or cascade-delete audit entries.
- **Canonical Serialization:** Enforcing RFC 8785 JCS for JSON payloads inside envelopes and audit entries (§5, §8) provides deterministic hashing and cross-platform consistency in Rust.
- **Integrity Verifications:** Caching `variant.name` in SQL with fallback integrity checks against the authoritative decrypted `VariantRecord.name` (§6) balances query efficiency with data integrity.

## G. Scope discipline

- **Minimalist Schema:** Contains only 6 tables (`schema_meta`, `account`, `artifact`, `document`, `variant`, `audit_entry`, `plugin_secret`).
- **No Extraneous Tables:** Ephemeral approval sessions, preview tokens, and detector intermediate caches are kept in RAM (§10).
- **Plugin Boundaries:** Kind 5 and `plugin_secret` are strictly scoped to the v1 `cloud_ai` plugin without building over-generalized multi-plugin registries (§5.6, §6, §14).

## H. Deferral health

- **Documented Non-Goals:** §11 and §14 explicitly document deferrals: UI transient states, sync merge algorithms, multi-device identity registries, passphrase recovery/key rotation, and additional plugin tables.
- **Clean Architectural Separation:** Cryptographic algorithms remain delegated to `architecture.md`, IPC commands to `api.md`, and UI flows to `design.md`.

## I. Top 5 changes

1. Specify FK actions and deletion sequence (§6, §7).
2. Harmonize `never_retain` vs document meta `retain`|`discard`.
3. Add partial unique indexes for singleton config (and cloud AI) in `artifact`.
4. Document why `ApprovedVersionRecord` embeds full `DetectedFieldRecord`.
5. Pin Base64 framing for `OriginalRecord.raw_bytes_b64`.
