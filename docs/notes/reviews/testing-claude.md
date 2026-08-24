# Testing Specification Review: Privacy Gate v1

Reviewer: Claude (via `claude -p`; keychain auth, not `--bare`). Date: 2026-08-23.

(Raw Claude output from the shared review prompt.)

## A. Alignment testing → SRS

- **Coverage is essentially complete for AC-1..AC-6** (§6.1–§6.6), each cited to a command sequence with concrete oracles, and §12's traceability table ties them back to FR/NFR numbers. No AC is left silently untested.
- **Possible invented bound:** §6.3 asserts `preview_share` (AI kind) "requires `ai_instruction` (1..=4000)". This is a specific numeric validation range. Nothing in the parent-fact list ("ShareRequest/ai_instruction") establishes that exact bound came from `api.md`; if `api.md` doesn't itself specify `1..=4000`, this is a test inventing a product constraint rather than verifying one. Flag for confirmation against `api.md` (also raised in D).
- **No contradiction with idea.md found** — retention default (OQ-14), no-recovery, and stolen-file behaviour are all tested as SRS requires without overreaching into product decisions (§6.6, §8 "No recovery" row).
- Minor redundancy, not contradiction: §6.6 states the retain/discard tightening rule twice ("Per-import discard is allowed (tighten)" and again under the `retain` default bullet) — harmless but worth tightening for clarity.

## B. Alignment testing → design

- **Overlap rule** (design §3.5): table-driven + `proptest` coverage in §8 ("nesting keep-inside-redact; partial overlap redact-wins; one rule at export") is correctly scoped to the design-owned rule, not re-derived.
- **Variants**: create/apply/delete, no-edit, per-doc uniqueness (`variant_name_conflict`), `get_variant` no span text — §8 row is complete and matches the architecture/design facts.
- **Re-import**: "two imports of the same bytes → two `doc_id`s" (§8) is testable and doesn't assume a dedup behaviour design never specified.
- **Performance budgets**: §8 and §13 correctly split "design owns the numbers, testing owns where they run" (perf job, nightly/pre-release, non-flaky PR gate) — matches OQ-2's stated split.
- **OQ-6 predicate**: §7 restates the design §2.6 predicate ("discard OR approved-only transmission") verbatim as the thing being checked, not a new predicate — compliant with the constraint that testing owns the oracle, not the predicate.
- **ShareRequest/ai_instruction**: covered functionally (§6.3), but the `1..=4000` literal (see A) should be sourced from `api.md`, not restated as if testing is the origin of the bound.

## C. Alignment testing → architecture

- **DEK-erasure deletion**: §8 row extracts a DEK copy pre-delete (test-only) and asserts post-delete decrypt failure — correctly tests erasure without needing key escrow.
- **Audit HMAC chain / tamper / truncation / crash-window / degraded**: §8 rows match architecture facts exactly, including the crash-window range **1..32** cited verbatim from the parent fact — no invented range.
- **Linux Secret Service fallback**: §8 explicitly declines to assert coordinated DB+file rollback is detectable ("is not asserted as detectable"), correctly honoring the documented degraded threat model rather than overclaiming security the architecture doesn't promise.
- **SQLCipher raw key**: §8 tests the `x'<64 hex>'` / `sqlite3_key_v2` open path, matching "SQLCipher raw key" (not passphrase-KDF) exactly.
- **Re-render PDF / no plaintext-to-disk**: §8 rows ("Export true-removal", "No plaintext-to-disk") are properly component-tested with a temp-dir watcher and reference architecture §11.
- **C-ARCH-2 vs save-dialog**: correctly deferred — §7.1 asserts "No other network or filesystem write... is permitted (C-ARCH-2)" for the egress spy while §12/§14/C-TEST-8 push save-dialog chrome to the UI spec. This is the one place C-ARCH-2 has a documented exception, and testing.md doesn't quietly re-absorb or ignore it.
- **Cloud AI HTTP mock**: §4 and C-TEST-3 correctly constrain Cloud AI testing to a mock allowlisted host reached only via the Rust core (in-process command test), matching "HTTP only from Rust."

## D. Alignment testing → API

- **Session gating**: §8's "Session table" row (every `api.md` §2 cell, allowed vs `not_in_session`) is the correct general mechanism. However, §6.4 separately asserts a *specific* cell result for `degraded_integrity` ("document commands return `not_in_session`") inline in the AC-4 scenario. This duplicates/could drift from the canonical session table test — recommend §6.4 reference the api.md table cell rather than assert the error code independently, so there is one source of truth.
- **Preview token identity**: §6.2 (`commit_share` byte-identical to preview) and §8 ("Preview expiry: 10 min / lock / replaced token → `preview_expired`") match the stated identity/expiry facts.
- **C-API-1..6**: §8's row is titled "C-API-1..6" but only enumerates ~4 behaviours (key never in outputs; span text only on approval views; originals inbound-only; `cloud_ai_test` sends no docs). It's not clear all six numbered constraints have a named test — this reads as a coverage claim broader than what's itemized. Recommend expanding to one line per constraint number.
- **Command-level, not webview**: explicitly and repeatedly enforced (§2, C-TEST-5) — acceptance tests call in-process command functions, webview gets no fs/http/shell. Correctly matches "command-level (not webview) acceptance."
- Same `ai_instruction` bound concern as A/B applies here — if this is genuinely an `api.md`-defined range, cite the section; if not, don't hard-code it in testing.md.

## E. Alignment testing ↔ idea

- **Local-first / no hosting**: no test requires a server component; CI runs macOS/Windows/Linux locally (§11), consistent.
- **Key on device / no off-device vault key**: AC-5 (§6.5) tests stolen `vault.db` + OS keystore item/fallback file yields no plaintext without the passphrase — directly verifies the no-off-device-key claim.
- **No recovery in v1**: §8's "No recovery" row ("wrong passphrase is `unlock_failed`") matches idea.md exactly; testing doesn't invent a recovery path to make anything pass, and explicitly warns against doing so in §6.5.
- **Detection on-device**: pattern-pack and ONNX golden tests (§3, §8) run against local fixtures, not a cloud detector — consistent with on-device detection.
- **Vault-as-product / one canonical approved version**: §6.1 tests exactly one canonical `ApprovedVersion` per document — matches idea.md.

No idea.md contradictions found.

## F. Quality / implementability

- **TDD rule is not CI-enforceable as written.** §2's "No TCB production code lands without a test that failed before the code existed" is a real requirement but nothing in §11 (CI) or decision 0006 §"Consequences" specifies *how* this is checked — it currently relies on reviewer attestation, which the spec doesn't mention. This is a genuine gap between "required" and "verifiable."
- **OQ-6 oracle has no self-test.** §7.2's plaintext-scan oracle depends entirely on a "read-only PDF library" correctly decoding all content streams (including compressed/FlateDecode objects, subsetted fonts, XMP). If that library silently fails to decode a stream, the oracle reports "clean" when a leak exists — a false negative on the single most safety-critical check in the spec. There's no golden test asserting the oracle *itself* catches a deliberately injected leak.
- **Short-token oracle risk acknowledged but not resolved.** §7.2 notes tokens `<4` codepoints are excluded "too many false positives," but doesn't define what "surrounding context" pairing actually means operationally — this is likely to be either under-specified (tests skipped) or flaky when implementers guess.
- **Mutation gate breadth is coherent and implementable**: `cargo-mutants` with standard operators, S≥0.80 on 8 explicitly named modules, S≥0.70 elsewhere, PR-blocking timeout tuned to avoid flaking on slow HMAC/delete paths (§5.6) — this is a realistic, non-gameable setup as written, with equivalent-mutant annotations requiring a reason (§5.4) to prevent silent gaming.
- **CI is runnable**: no job requires infrastructure the spec doesn't already provide (mocks for keystore/Cloud AI, temp-dir watchers); Linux-only fallback-file job is correctly scoped to Linux CI (§11).

## G. Scope discipline

- No UI leakage found: §3, §8 ("C-ARCH-2 save-dialog exception"), §12, §14, and C-TEST-8 all consistently push UI E2E, save-dialog chrome, and copy/wording to the (unwritten) UI spec rather than testing it here.
- OQ-14 is not invented: §6.6 and C-TEST-6 explicitly refuse to assume an out-of-box retention default, setting it explicitly per test.
- Webview is not mutated: mutation gate is Rust-only (§5.2, decision 0006 "Alternatives Considered"), StrykerJS explicitly excluded from the v1 gate.
- Live Cloud AI is explicitly forbidden (C-TEST-3, §4's mock-only network double) — no scope creep into requiring a real external service.
- No process chatter misplaced in testing.md — the "why" content (§1, §5.1) is spec-appropriate rationale, not dev-log narration; decision 0006's process history correctly lives in the decision doc, not duplicated in testing.md.

## H. Deferral health

- **OQ-14**: healthy, not circular. Genuinely deferred to `idea.md` with an explicit compensating rule (tests set the default) so the deferral doesn't block any test from running (§6.6, §14, C-TEST-6).
- **UI E2E**: healthy for now — cleanly pointered to a UI spec that doesn't yet exist, but every UI-adjacent behaviour still has a command-level test here (§3's closing note: "the core contract still has a command-level test here"), so nothing is silently untested pending the UI spec's arrival.
- **ONNX nightly**: healthy — decoupled from PR-blocking tests via a stub detector used for AC-1..AC-4 (§10), so vault/audit/crypto correctness isn't gated on model availability, while pattern-pack goldens (deterministic, fast) still run every PR (§3, §11). Not circular; it's a legitimate flakiness/cost tradeoff for a heavy model artifact.

## I. Top 5 changes

1. **Cite or remove the `1..=4000` `ai_instruction` bound** (§6.3): either add "(api.md §X)" if that's where it's defined, or rewrite as "within api.md's specified bound" so testing.md doesn't become the de facto source of an API constraint.
2. **Enumerate C-API-1..6 individually** in §8's constraints row instead of one row naming ~4 behaviours under a "1..6" label — makes the coverage claim auditable.
3. **Deduplicate the degraded_integrity assertion**: have §6.4's degraded-session claim reference the generic session-gating table test (§8) rather than assert the error code independently, avoiding two sources of truth that could drift apart.
4. **Add a self-test for the OQ-6 PDF oracle** (§7.2): a fixture where a known plaintext leak is deliberately injected into export bytes, asserting the oracle *fails* the test — otherwise a PDF-library decoding gap produces a silent false "clean," which is the worst failure mode for the spec's most safety-critical check.
5. **Give the TDD rule (§2, decision 0006) a concrete enforcement mechanism** — e.g., require the failing-test commit to precede the implementation commit and name a check (reviewer attestation in the PR template, or a two-commit convention) — since as written it's a stated process with no verification path in §11's CI table.
