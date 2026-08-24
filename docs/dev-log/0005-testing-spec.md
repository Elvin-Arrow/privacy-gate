# [0005] Testing spec generation, TDD + mutation, Claude+Gemini review

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Produce the v1 testing spec: TDD as the development method, mutation testing on the Rust
TCB, acceptance-test mechanics for AC-1..AC-6, and the independent OQ-6 oracle. Review with
Claude and Gemini only (decision 0005; do not invoke Ollama).

## Implementation

- Recorded decision 0006 (TDD required for TCB work; `cargo-mutants` as the mutation gate;
  TypeScript/Stryker not a v1 gate because the TCB is Rust).
- Drafted `docs/specs/testing.md`: layers, tools, gated modules, AC scenarios, egress-spy
  oracle, architecture security checks, CI jobs.
- Reviewed with Gemini (`agy --effort high`) and Claude (`claude -p`). Raw output:
  `docs/notes/reviews/testing-gemini.md`, `docs/notes/reviews/testing-claude.md`.
- Reconciled both reviews into the spec and decision 0006, then updated parent specs,
  open-questions, and indexes.

## Problems Encountered

- **Mutation threshold contradiction (Gemini):** §5.3 said S ≥ 0.80 while §5.5 said any
  survivor fails CI (S = 1.00).
- **DEK-erasure oracle was cryptographically wrong (Gemini):** decrypting leftover
  ciphertext with a DEK copied *before* delete would still succeed; NFR-R2 is vault-level
  wrapped-DEK destruction.
- **OQ-6 false positives (Gemini):** `|s| ≥ 4` scans hit PDF keywords (`Type`, `Font`, …).
- **Discard matrix could skip egress checks (Gemini).**
- **OQ-6 false negatives (Claude):** no self-test that the PDF oracle fails on a planted leak.
- **TDD not CI-enforceable (Claude):** git cannot prove the red phase.
- **`1..=4000` looked invented (Claude):** it is api.md §4; needed a citation.

## Resolution

- Gated TCB modules: no unexplained survivors after equivalent-mutant annotations
  (effective S = 1.00). Other core: S ≥ 0.70. PR job is `cargo mutants --file` on the
  explicit gated path list, not "files changed on this branch."
- DEK test asserts Vault load fails and wrapped DEK row is gone/zeroized.
- High-entropy canaries (≥ 8 codepoints, not PDF/JSON keywords); oracle items 1–2 apply
  even when retention is `discard`; negative fixture must make the oracle fail.
- TDD: PR attestation + reviewer rejection of tests-after; mutants are the automated audit.
- `ai_instruction` bound cites api.md §4. C-API-1..6 each have a named row. Degraded
  session defers to the api.md §2 table test.

Rejected: two-commit git convention as a CI gate; StrykerJS as a v1 gate; inventing OQ-14;
webview E2E in this spec; live Cloud AI in CI.

## Verification

- OQ-6 marked resolved (design predicate + testing oracle). OQ-2 enforcement points at the
  perf job. Remaining product/UI: OQ-14, OQ-4 save-dialog.
- AC-1..AC-6, NFR-S3/S4, NFR-R1/R2, Linux fallback honesty, crash-window vs tamper, and
  C-API-1..6 are traced in testing.md §12.
- Knowledge-governance: spec in `docs/specs/`, decision 0006, this log, indexes, raw reviews
  in notes.

## Lessons

- A mutation "score" and "no survivors" cannot both be the gate unless equivalent mutants
  are excluded from the denominator and the score is then 1.00 on the gated set.
- "Decrypt with a saved key fails" is the opposite of cryptographic erasure of the *wrap*.
- An egress oracle that cannot fail on a planted leak is not an oracle.

## Related Documentation

- [Spec — testing](../specs/testing.md)
- [Decision 0006 — TDD + mutation](../decisions/0006-tdd-and-mutation-testing.md)
- [Decision 0005 — Claude + Gemini review](../decisions/0005-review-claude-gemini.md)
- [Spec — API](../specs/api.md)
- [Spec — architecture](../specs/architecture.md)
- [Open questions](../notes/open-questions.md)
- [Raw reviews](../notes/reviews/)
