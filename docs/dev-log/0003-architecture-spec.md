# [0003] Architecture spec generation and review

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Produce the v1 architecture spec inside the design's component boundaries: crypto, key
storage, trust boundaries, plugin runtime shape, detection-model host, export sanitization,
and the architecture-owned open questions (OQ-3, OQ-5, OQ-12, OQ-13, OQ-17, OQ-18). Run the
three-model review per decision 0001.

## Implementation

- Drafted `docs/specs/architecture.md` and decision 0004 (crypto suite, local-only account,
  no recovery, plugin host API, Cloud AI auth, hybrid detector, re-render export).
- Ran Gemini review via `agy --effort high`. Raw output:
  `docs/notes/reviews/architecture-gemini.md`.
- Ollama Cloud (`gpt-oss:120b-cloud`, `qwen3.5:cloud`) returned HTTP 429 session-usage
  limit; a probe to `gemma4:31b-cloud` failed the same way. Local `qwen3.5:9b` truncated the
  prompt and did not produce a section A–G review. Gap recorded in
  `architecture-gpt-oss.md` and `architecture-qwen-3.5.md`.
- Reconciled from the Gemini review plus an author implementability pass, then updated
  indexes, `design.md` deferrals, the open-questions register, and SRS §10.

## Problems Encountered

- **Crash-window bricking (Gemini, critical):** persisting `audit_head` after the SQLCipher
  append, then fail-closed on any head/tail mismatch, would lock the user out after a power
  loss. Two-phase commit of DB + OS keystore is not atomic.
- **Canonical encoding unspecified (Gemini):** OQ-3 named HMAC/SHA-256 but not the byte
  layout, so the chain was not implementable.
- **SQLCipher passphrase KDF (Gemini):** passing `sqlcipher_key` as a string would apply
  ~256k PBKDF2 on top of HKDF and blow the ≤ 1 s unlock budget.
- **AAD concatenation collisions (Gemini):** `kind || doc_id || version` without lengths.
- **FR-7.4 vs fail-closed (Gemini):** a hard refusal to open hid the audit trail at the
  moment tamper-evidence fired.
- **Linux fallback overclaim (Gemini):** anti-truncation does not survive rolling back
  `vault.db` together with the fallback keystore file.
- **mlock / keystore IPC / ONNX bundling (Gemini):** page-granular mlock, per-append
  keystore writes, and unbundled ONNX Runtime were cross-platform holes.
- **Ollama Cloud 429:** could not obtain gpt-oss or qwen-3.5 cloud reviews.

## Resolution

- Crash-window **fast-forward** when the DB is 1..32 valid HMAC'd entries ahead of the
  persisted head; true integrity failure is fail-closed for document decrypt and
  **degraded-open** for a verification report (FR-7.4).
- Pinned **canonical encoding v1** (length-prefixed big-endian envelope + RFC 8785 JCS
  payloads) and **length-prefixed AAD v1**.
- SQLCipher opened with raw `x'<64 hex>'` / `sqlite3_key_v2`; no passphrase KDF.
- Batched keystore head (every 32 appends, every Share, on lock); Linux fallback is atomic
  replace with a documented degraded threat model.
- `memsec` page-aligned locked pages for key material; mlock failure degrades to zeroize-only.
- ONNX Runtime shared libraries bundled per v1 target triple; no runtime download.
- PDF-only export left as design.md §3.7, not an architecture narrowing of FR-5.1.
- Gemma→`pg-hybrid-v1` treated as a legitimate architecture choice (Gemini agreed; no
  idea.md amendment).

Rejected: treating Gemini's "add plaintext export" as an SRS miss (design already fixed
single-doc export to PDF). Rejected: claiming anti-truncation on the Linux file fallback.

## Verification

- Architecture spec traces FR-4, FR-7.3/7.4, FR-8, FR-9.5, NFR-S/P/R/E, and the design
  deferrals in §16.
- Open-questions register marks OQ-3, OQ-5, OQ-13, OQ-17, OQ-18 resolved and OQ-12
  architecture-part resolved; OQ-4 remainder, OQ-6 remainder, OQ-14 remain open.
- Knowledge-governance skill applied: spec in `docs/specs/`, decision 0004 in
  `docs/decisions/`, this log in `docs/dev-log/`, indexes updated, Gemini review in notes.

## Lessons

- An integrity mechanism that fails closed without a crash-recovery rule will brick the
  vault on the first power loss between two stores. "Fail closed" has to distinguish
  "DB is one step ahead" from "the chain is broken."
- Pinning an algorithm without pinning the canonical encoding is the same class of bug as
  the design spec's `ApprovedVersion` without `redacted_content`: it looks complete and is
  not implementable.
- Decision 0001's three-model step is only as available as the reviewers. Record a miss;
  do not invent a third review.

## Related Documentation

- [Spec — architecture](../specs/architecture.md)
- [Spec — design](../specs/design.md)
- [Spec — SRS](../specs/srs.md)
- [Decision 0004 — v1 architecture](../decisions/0004-v1-architecture.md)
- [Decision 0001 — review approach](../decisions/0001-multi-model-spec-review.md)
- [Open questions](../notes/open-questions.md)
- [Raw reviews](../notes/reviews/)
