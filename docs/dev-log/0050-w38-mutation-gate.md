# [0050] W38 — Mutation gate (testing.md §5.3)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Make the gated TCB modules PR-blocking at mutation score **S = 1.00** after annotated
equivalent skips (testing.md §5.3 / §5.5, decision 0006). Nightly runs the rest of
`pg-core` (S ≥ 0.70). No Stryker on TypeScript; no silent threshold drop.

## Implementation

- **`scripts/mutation-gate.sh`**: one shard per §5.3 path. Each shard passes
  `--cargo-arg --lib` plus the colocated `--test` binaries so the linker does not
  build every `core/tests/*.rs` (that OOMs a 2 GiB Docker linker). `--in-place` only
  when `MUTANTS_IN_PLACE=1` (CI). Config schema has no `jobs`/`timeout` keys; timeout
  is `--minimum-test-timeout 30`.
- **PR shards** (S = 1.00 after skips/unviable): overlap, export, share, audit, aad,
  dek, ollama, vault (`--re` on raw-key open / DEK overwrite-and-drop), session
  (`--re` on `command_allowed` / `retention_override_forbidden`).
- **`.github/workflows/ci.yml`**: matrix job `mutants` (nine shards), `build`
  `needs: [test, mutants]`. Install via `taiki-e/install-action` `tool: cargo-mutants`.
- **`.github/workflows/nightly.yml`**: `mutants-core` runs `scripts/mutation-gate.sh nightly`
  (full crate minus §5.4), 360 min timeout. First nightly may sit below 0.70 on
  non-gated modules; that is expected until those tests catch up. The PR gate is the
  shards.
- **`make mutants`**: containerized `CARGO_BUILD_JOBS=1 ./scripts/mutation-gate.sh all`.
- **`mutants = "0.0.3"`** in `core/Cargo.toml` for `#[mutants::skip]`.
- Small production helpers so the tests have something that is not an equivalent
  tautology: `share::no_originals_left_device`, `session::retention_override_forbidden`,
  plus skipped helpers called out below.

## Tests first (survivors killed, not skipped)

A known hole, then the first real survivors from each shard, each killed by a new
test that cites the spec clause:

- **Share / OQ-6:** `no_originals_left_device` truth table; preview flag is not a
  hardcoded `true`; `preview_ttl_ms` and `yyyymmdd_utc` cannot be constants;
  suggested filename does not re-derive the date from the same helper.
- **Session / AC-6:** `retention_override_forbidden` table; every `SESSION_TABLE`
  cell covered.
- **DEK / AAD / vault:** zeroize-until-destroy; AAD round-trip every `ArtifactKind`;
  overwrite-in-place zeros (delete tests previously only saw the DROP).
- **Export / overlap / audit:** wrap-line and info-dict dates from unix time;
  nested keep vs zero-length keep; `redact_pages` needs `overrides_w26` on the
  overlap shard; audit `||`→`&&` in `verify_against_head` only dies if the test
  asserts `verified_tail_sequence: 0` in-loop (a later head-mismatch Modification
  masks it).
- **Ollama (§10.1):** show-object without `details`; `Detector::detect` returns
  fields; 50% reject rate does not fail (`>` not `>=`, `/` not `*`); absolute
  offset is span + chunk + entity (all three nonzero); overlapping chunk prompts;
  `split_chunks` ASCII overlap + UTF-8 boundary walks.

## Equivalent skips (testing.md §5.4)

Each skip is a one-line reason on the helper, not a silent threshold drop:

- `overlap::span_nonempty` — `u64 >= 0` is tautological. `strictly_contained`'s
  `byte_length > 0` is **not** equivalent (zero-length Keep would un-redact).
- `audit::payload_exceeds_canonical_u32` — 4 GiB payloads are unallocatable.
- `default_ollama_allowlist` — body is already `vec![]` while the digest pin is
  `None`. Remove when `OLLAMA_GEMMA4_E2B_DIGEST` is `Some`.
- `chunk_reported_entities` / `chunk_start_in_range` / `rewind_to_char_boundary` —
  usize tautology; `start == len` never hits the loop because a full chunk
  `break`s; rewind is dead because `start` is already a char boundary.

## Ambiguities resolved

- **`-- --test foo` does not apply to `cargo test --no-run` baseline.** Must pass
  `--cargo-arg --lib` and `--cargo-arg --test --cargo-arg <binary>`.
- **`--in-place` cannot combine with `--jobs`.** CI is in-place on an ephemeral
  checkout; local Docker uses copy-out and `CARGO_BUILD_JOBS=1`.
- **`yyyymmdd_utc` tests that compute expected via the same function are
  equivalent mutants.** Assert a literal date from a known unix-ms input.
- **Ollama `+=`→`*=` on a UTF-8 walk hangs.** An `assert!(i > before)` turns that
  into a caught panic instead of cargo-mutants exit 3 (timeout is a kill per
  §5.3, but a non-zero exit would still fail CI).

## Verification

PR shards, local Docker, cargo-mutants 27.1.0: no missed mutants (unviable and
annotated skips only). Nightly full-crate job is defined, not run locally.

## Traceability

- testing.md §5.3–§5.6; decision 0006
- architecture.md §10.1.1 / §10.1.4 (Ollama gate)
- Next: W39 — perf + no-plaintext watcher (`docs/dev-plan.md`)
