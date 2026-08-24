# Testing Specification Review: Privacy Gate v1

Reviewer: Gemini (via `agy --effort high`). Date: 2026-08-23.

(Raw Gemini output from the shared review prompt.)

## A. Alignment testing → SRS

- **AC-1 through AC-6 Coverage (§6.1–§6.6):** Fully mapped and traced (§12).
  - **AC-1 (§6.1):** Accurately tests born-digital PDF import, field extraction, canonical `ApprovedVersion` creation, catalog `has_approved_version`, and `already_approved` error on subsequent approval attempts.
  - **AC-2 (§6.2):** Validates ephemeral overrides, preview PDF generation, byte-identical export on `commit_share` (FR-6.1), absence of redacted text in PDF streams/info dicts, and multi-doc user selection order preservation (design §3.7).
  - **AC-3 (§6.3):** Asserts `cloud_ai_set_config` returns only `key_last4` (C-API-1), `cloud_ai_test` sends no documents (C-API-4), `preview_share` requires `ai_instruction` (1..=4000), `commit_share` POSTs the identical approved body, and missing keys return `cloud_ai_not_configured`.
  - **AC-4 (§6.4):** Verifies `list_audit_events` returns classifications/decisions without redacted text/keys/originals, verifies `no_originals_left_device`, and validates that `degraded_integrity` returns only the verified HMAC prefix while document commands yield `not_in_session`.
  - **AC-5 (§6.5):** Verifies stolen locked `vault.db` and Linux keystore fallback file without passphrase cannot be decrypted or queried (NFR-S3 / FR-4.4), with no recovery backdoors.
  - **AC-6 (§6.6):** Asserts global `never_retain` forbids loosening (`retention_loosen_forbidden`), while tightening (discard) is permitted.
- **OQ-14 Discipline (§6.6):** The spec honours OQ-14 by not assuming an out-of-box default retention policy, explicitly parameterizing test setups.
- **Error Condition Completeness (§8):** Covers API error variants (`preview_expired`, `variant_name_conflict`, `retention_loosen_forbidden`, `unlock_failed`, `not_in_session`). PDF parse failures (`document_corrupt`, `document_encrypted`) are noted in API mappings.
- **Invented Requirements:** None. The testing spec strictly derives assertions from SRS functional and non-functional requirements.

---

## B. Alignment testing → design

- **Overlap Rule (§5.3, §8):** Placed in the mutation gate (§5.3) and verified via table-driven tests and `proptest` (§8) against design §3.5 byte-offset rules (nesting keep-inside-redact, partial overlap redact-wins, export resolution).
- **Variants (§8):** Accurately tests create/apply/delete lifecycle, immutability (no edit), per-document name uniqueness (`variant_name_conflict`), and exclusion of span plaintext in `get_variant` (C-API-2).
- **Re-import (§8):** Validates that importing identical document bytes yields two distinct `doc_id`s with independent cryptographic lifecycles.
- **Performance Budgets (§3, §8, §11):** Enforces design §7 budgets (unlock ≤ 1 s, import/detect/approval ≤ 1 s for ≤ 200 fields, export, audit query) inside a dedicated nightly/pre-release performance job on the specified runner class (mainstream laptop: 8 GB RAM, SSD), preventing PR CI flakiness. Correctly asserts that functional import over budget returns `over_budget: true` without failing document processing.
- **OQ-6 Predicate (§7.2, §7.3):** Strictly follows design §2.6 (`no_originals_left_device` is true iff retention was `discard` OR share transmits only approved version).
- **ShareRequest & `ai_instruction` (§6.3):** Enforces 1..=4000 character constraints, payload identity between preview and egress, and audit event recording `has_ai_instruction: true` without logging instruction text.

---

## C. Alignment testing → architecture

- **DEK Destruction (§5.3, §8, §12):** Enforces architecture §4.3 (NFR-R2). Tests assert that upon document/original/variant deletion, the wrapped DEK in SQLCipher is overwritten/dropped and subsequent vault decrypt attempts fail. *(See Section F & I regarding the test oracle phrasing).*
- **Audit Chain, Tamper, Truncation & Degraded State (§5.3, §6.4, §8):**
  - Bit-flip in DB payload triggers `degraded_integrity` on unlock and blocks document decryption.
  - Tail truncation below persisted `audit_head` triggers `degraded_integrity`.
  - Crash-window fast-forward (1..32 valid uncommitted HMAC rows) cleanly transitions to `unlocked`.
  - HMAC integrity failure within the crash window is not fast-forwarded and triggers `degraded_integrity`.
- **Linux Fallback Honesty (§3, §4, §6.5, §8, §11):** Correctly executes against the real `0600` file backend on Linux CI. Explicitly reflects the architecture's degraded threat model: coordinated rollback of both `vault.db` and the keystore file is acknowledged as undetectable by design.
- **SQLCipher Raw Key (§5.3, §8):** Tests verify opening SQLCipher via `sqlite3_key_v2` / raw hex key format (`x'<64 hex>'`), bypassing slow passphrase KDF during normal unlock.
- **Export Re-render & PDF Sanitization (§5.3, §7.2, §8):** Mandates verifying that generated PDFs contain no incremental updates (`/Prev`), orphaned content streams, or metadata leaks containing redacted text (architecture §11).
- **No Plaintext-to-Disk & C-ARCH-2 (§8):** Component test monitors sandbox directories during import/detect/export to verify zero plaintext leaks outside encrypted SQLite ciphertext and keystore wrap files. Export save-dialog disk writes are cleanly isolated as UI spec responsibilities (C-TEST-8).
- **Cloud AI HTTP Mock (§4, §6.3, §7.1, §10):** Directs Plugin Host tests through local `wiremock`/`hyper` TLS servers over `rustls`, asserting that outbound HTTPS originates strictly from the Rust core and enforces allowlisted origins (C-ARCH-3).

---

## D. Alignment testing → API

- **Session Gating (§5.3, §8):** Comprehensive unit/acceptance coverage across all state transitions (`locked`, `unlocked`, `degraded_integrity`) against the `api.md` §2 command matrix, asserting `not_in_session` on illegal transitions.
- **Error Types & Wire Contract (§6, §8):** Strictly tests snake_case API error strings (`already_approved`, `retention_loosen_forbidden`, `cloud_ai_not_configured`, `preview_expired`, `variant_name_conflict`, `unlock_failed`, `not_in_session`).
- **Preview Identity Guarantee (§6.2, §6.3, §7.1):** Enforces that `commit_share` consumes the `PreviewToken` issued by `preview_share` and delivers the exact byte payload verified during preview.
- **C-API Constraints (§6.1, §6.3, §6.4, §8):**
  - **C-API-1:** API keys never appear in outputs (`key_last4` only).
  - **C-API-2:** Span text appears only in approval payloads, never in catalog, variants, or audit logs.
  - **C-API-3:** Original document bytes are strictly inbound (`import_document`).
  - **C-API-4:** `cloud_ai_test` sends zero document content.
  - **C-API-5 / C-API-6:** Binary IPC payload handling and prohibition of ambient network calls.
- **Command-Level In-Process Execution (§1, §2, §3):** Acceptance tests call Rust API command functions in-process, decoupling the core verification from Tauri IPC/webview serialization.

---

## E. Alignment testing ↔ idea

- **Local-First & On-Device Processing:** Testing operates fully offline; detector contract tests use local regex packs (`pg-patterns-uk-v1`) and local ONNX runtimes without network dependencies.
- **Key Stays on Device:** Validates local OS keystore abstractions and file fallbacks; confirms zero remote key escrow or cloud sync endpoints.
- **Vault-as-Product:** Enforces single-user vault semantics, envelope encryption, single canonical approved version, and irrevocable DEK deletion.
- **No Hosting / Optional Cloud AI:** Cloud AI is tested strictly as an optional outbound plugin driven by user-supplied API keys, verifying that document data never leaves the device without explicit user approval.

---

## F. Quality / implementability

- **Threshold Contradiction (§5.3 vs §5.5):** §5.3 sets a mutation score threshold of S ≥ 0.80 for gated modules, whereas §5.5 states: *"A survived mutant on a gated module fails CI."* Requiring zero surviving mutants makes the effective threshold S = 1.00 (100%), directly contradicting the 80% numerical gate.
- **DEK Erasure Oracle Phrasing (§8):** In §8, the check states: *"ciphertext decrypt with a saved DEK copy (test-only extract before delete) fails"*. Mathematically, symmetric ciphertext decrypted directly with a valid extracted key will still decrypt. The oracle must assert that **vault-level decryption** fails because the wrapped DEK in SQLCipher is destroyed, and querying the document record yields no valid key material.
- **OQ-6 Oracle False Positives (§7.2):** In §7.2 Item 1, scanning raw PDF bytes for redacted strings where |s| ≥ 4 risks false positives if synthetic PII fields match common English tokens or PDF structural keywords (e.g., `"true"`, `"Type"`, `"Font"`, `"null"`, `"form"`, `"xref"`). Synthetic fixtures must be required to use high-entropy/distinct tokens (e.g. `"QQ123456C"`, `"20-40-60"`).
- **OQ-6 Retention Matrix Clarification (§7.3):** In §7.3, the extra check for `discard` states: *"Flag may be true even before oracle (no original exists)"*. This could lead implementers to bypass verifying Oracle Item 1 (absence of redacted text in egress) when retention is `discard`. The spec must state that Oracle Item 1 and Item 2 apply universally to all shares regardless of retention mode.
- **TDD Enforcement via CI:** Decision 0006 mandates TDD for all TCB changes. Because git commit history cannot reliably enforce the "red" phase in CI, `cargo-mutants` serves as the practical, automated enforcement mechanism.

---

## G. Scope discipline

- **UI Isolation (C-TEST-8, §3, §4):** Correctly excludes webview rendering, DOM inspection, TS framework logic, and OS save-dialog chrome from the Rust testing spec, deferring them to the UI specification.
- **OQ-14 Retention Default:** Strictly avoids deciding the default out-of-box retention policy, testing both `never_retain` and `retain` paths by setting configuration explicitly.
- **No Webview Mutation Testing (§4, Decision 0006):** Correctly rejects StrykerJS/TypeScript mutation testing for v1 core gates, focusing verification entirely on the Rust TCB.
- **Mocked Cloud AI (C-TEST-3, §4, §10):** Strictly bans live Cloud AI network calls in automated suites, using local TLS mocks.
- **Spec vs Decision Separation:** Decision 0006 cleanly records the architectural context and trade-offs for TDD and `cargo-mutants`, while `testing.md` contains executable technical requirements without process chatter.

---

## H. Deferral health

- **OQ-14 (Retention Default → `idea.md`):** **Healthy.** The testing spec defines acceptance tests for both retention behaviors without hardcoding an unapproved product default.
- **OQ-4 / UI Acceptance (Save-dialog & Webview Chrome → UI Spec):** **Healthy.** The boundary between in-process command acceptance and OS UI dialogs prevents flaky E2E tests in core CI.
- **ONNX Nightly / Model Pinning (§3, §11):** **Healthy.** Running regex goldens on every PR while deferring heavy ONNX inference to nightly/pre-release jobs keeps PR CI fast and deterministic while still validating model integrity.

---

## I. Top 5 changes

### 1. Reconcile Mutation Threshold Contradiction in §5.3 and §5.5
* **Issue:** §5.3 specifies S ≥ 0.80 on gated modules, while §5.5 states any surviving mutant fails CI (S = 1.00).
* **Fix:** Update §5.3 and §5.5 to clarify that for gated TCB modules, all surviving mutants must either be killed by tests or annotated with an explicit equivalent-mutant exemption (effective non-equivalent mutation score S = 1.00), while remaining non-gated core modules require S ≥ 0.70.

### 2. Correct DEK Destruction Oracle in §8
* **Issue:** Stating that decrypting ciphertext with a saved pre-deletion DEK copy fails is cryptographically incorrect.
* **Fix:** In §8 (NFR-R2 row), replace:
  > *"ciphertext decrypt with a saved DEK copy (test-only extract before delete) fails"*
  with:
  > *"vault document decryption fails due to wrapped DEK zeroization/deletion in SQLCipher; raw wrapped DEK row is verified purged/empty"*.

### 3. Enforce Distinct / High-Entropy Synthetic Tokens for OQ-6 Oracle (§7.2, §10)
* **Issue:** Searching raw PDF bytes for any redacted substring with |s| ≥ 4 can false-positive on standard PDF keywords or dictionary words.
* **Fix:** In §7.2 and §10, add an explicit requirement that synthetic test fixtures must use unique, high-entropy canary strings for PII spans (e.g., standard UK format sort codes, NI numbers, and dedicated distinct tokens) that do not collide with PDF syntax or JSON structure keys.

### 4. Clarify §7.3 Discard Retention Matrix Check
* **Issue:** §7.3 notes that for `discard`, *"Flag may be true even before oracle (no original exists)"*, which could be misinterpreted as skipping egress sanitization verification.
* **Fix:** Update §7.3 to clarify:
  > *"Even when retention is `discard`, Oracle Item 1 (no redacted plaintext in egress) and Item 2 (keep-visible text present) are mandatory. The flag is asserted true only after all applicable oracle checks pass."*

### 5. Add Scoping Flags for PR Mutation Job in §5.6
* **Issue:** Running unconstrained `cargo-mutants` on PRs can cause CI timeouts.
* **Fix:** In §5.6, specify the exact scoping mechanism for PR CI (e.g., `cargo mutants --file <gated_paths>` or filtering by changed files in git diff) to ensure PR mutation runs complete within predictable time limits.
