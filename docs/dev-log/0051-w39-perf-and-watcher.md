# [0051] W39 — Perf budgets + no-plaintext-to-disk watcher

- **Status:** Complete
- **Date:** 2026-08-25

## Objective

Add the design.md §7 performance budgets as a nightly (non-flaky-PR-gate) job, and the
testing.md §8 no-plaintext-to-disk component test, per dev-plan W39. Unlock ≤ 1s after
passphrase on the documented runner is part of the same perf suite, not a separate chunk.

## Implementation

- **`core/tests/no_plaintext_watcher_w39.rs`** (new, PR-gated — deterministic, not
  timing-based, so it runs in the `test` job like any other component test): runs a full
  import -> detect -> approve (redact one field, keep another) -> export -> `lock()` flow
  inside one `tempfile::tempdir()` sandbox, using `FileKeystore` so the Linux 0600 fallback
  blob is written into that sandbox on every OS this runs on (same trick AC-5 uses).
  After `lock()` closes every handle, a recursive walk asserts no file under the sandbox —
  `vault.db` and the fallback blob included, not exempted — contains the passphrase or
  either canary marker in raw UTF-8 or UTF-16 form. A `self_test_catches_a_planted_leak`
  test (same convention as the OQ-6 oracle's `inject_flate_canary` self-test) proves the
  walker isn't vacuously passing by planting a real leak and asserting it's caught.
- **`core/tests/perf_w39.rs`** (new, nightly-only): five `#[ignore]`d tests (same
  convention as `ollama_w15b.rs`'s `nightly_real_ollama`) — unlock ≤1s, fused
  import+detect of a ~1MB fixture, ≤200-field approval payload ≤1s, single-document export
  ≤5s, audit query ≤500ms. Two spec/implementation gaps surfaced and are documented in the
  test file rather than papered over:
  - `import_document` fuses import and detect into one call (W14: detect runs
    synchronously inside it); there is no seam to time detect alone, so that test budgets
    the fused call at 2s + 5s = 7s rather than inventing a detect-only entry point.
  - `list_audit_events`'s real `limit` is capped at 200 (`session.rs`: `"limit must be
    1..=200"`), not the design.md §7 "last 1000 events" — the test queries at the actual
    maximum against a smaller corpus; a true 1000-row scale fixture is a known gap.
- **`.github/workflows/ci.yml`**: `no_plaintext_watcher_w39` runs in the PR `test` job
  right after the acceptance pack.
- **`.github/workflows/nightly.yml`**: new `perf-budgets` job runs
  `cargo test -p pg-core --test perf_w39 -- --ignored --nocapture`.

## Tests first

All five perf assertions and the watcher passed on first run against the existing
implementation — this chunk's production code changes were zero; the deliverable is the
tests plus the two documented spec/implementation gaps they exposed (fused import+detect,
`limit` cap) for a later pass to reconcile against design.md/api.md.

## Traceability

- design.md §7 (resolves OQ-2)
- testing.md §8 "Unlock budget", "No plaintext-to-disk"
- architecture.md §5 (no new plaintext-to-disk path, ever)
- Next: nothing scheduled past W39 in dev-plan.md's chunk sequence — see decisions/dev-plan
  for what comes after "harden" (slice H).
