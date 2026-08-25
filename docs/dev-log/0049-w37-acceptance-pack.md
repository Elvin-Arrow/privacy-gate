# [0049] W37 — Acceptance pack AC-1..AC-7

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Gate AC-1..AC-7 as one in-process command-level pack that CI prints by name
(testing.md §6, C-TEST-3/5/7). Earlier chunks already held partial coverage; this
chunk fills the remaining command-level gaps and makes the scenarios show up in
CI logs as a single `cargo test` binary. No real Cloud AI host.

## Implementation

- **`core/tests/acceptance_w37.rs`** (new): seven named tests, stub detector, synthetic
  canaries only.
  - **AC-1** — born-digital PDF import, locatable stub spans, approve/store,
    `already_approved`, lock/unlock metadata with no canary on command output.
  - **AC-2** — ephemeral override preview + byte-identical commit; OQ-6 oracle;
    two-doc bundle preserves selection order (B then A) in manifest and extracted
    text; suggested filename `privacy-gate-2docs-redacted-…`; no `/Author` /
    `/Subject` / `/Keywords`.
  - **AC-3** — `cloud_ai_not_configured` with zero mock connections; `set_config`
    HTTPS origin + `get` is `key_last4` only; loopback mock via
    `test_only_set_cloud_ai_secret` (same TLS-mock gap as W27); `cloud_ai_test`
    sends no vault document; commit POSTs the previewed body; oracle on the wire
    body.
  - **AC-4** — after import/detect/approve/export/AI, `list_audit_events` shows
    those types; share payloads have kind and `no_originals_left_device`, never
    field text / instruction / HMAC fields; oracle on the export PDF (C-TEST-7).
  - **AC-5** — import+approve (retain) then lock; copy `vault.db` + Linux fallback
    keystore; stolen bytes contain no canary/passphrase; wrong SQLCipher key is
    `WrongKey`; wrap still holds; stolen-copy `unlock` is `unlock_failed`.
  - **AC-6** — confirmed `never_retain` forbids per-import retain (no catalog
    row); per-import discard is allowed; confirmed `retain` still allows
    per-import discard.
  - **AC-7** — factory `{ discard, confirmed: false }`; unconfirmed import
    (null or retain override) is `retention_policy_unset` with an empty catalog;
    confirm then first import may override to retain.
- **`.github/workflows/ci.yml`**: after `cargo test --lib`, a dedicated step
  `cargo test -p pg-core --test acceptance_w37 -- --nocapture` so the seven
  `AC-n` lines appear in the job log.

## Tests

7/7 green in `acceptance_w37`. Clippy on that test binary is clean (`-D warnings`).
No production Rust changes. `cargo test -p pg-core --tests` in this Docker
environment was killed by the linker (signal 9) — the same memory ceiling W29
hit on `--workspace`. CI therefore runs the named pack as its own step rather
than every integration binary at once. GitHub-hosted runners still execute
`--lib` plus this pack every PR.

## Ambiguities resolved

- **TLS-mock gap is unchanged.** testing.md §6.3 asks for a mock allowlisted HTTPS
  origin; there is still no TLS testing double. HTTPS is asserted at
  `cloud_ai_set_config`; the send path uses the W27 loopback mock. C-TEST-3
  (no real host) holds.
- **AC-2 multi-doc was the real hole.** W24/W26 covered single-doc override +
  oracle; selection-order in a two-document bundle had no command test.
- **AC-5 after import+approve was the other hole.** W3/W7 stole an empty vault
  or the fallback file alone. The pack copies a vault that actually contains an
  approved document and a retained original.
- **Degraded-session audit prefix** stays in `audit_list_w28` / session-gating
  (§8). AC-4's pack scenario is “what did I share?” on a healthy unlocked
  session; testing.md says that scenario does not re-state the gating table.

## Traceability

- testing.md §6.1–§6.7, §7 (OQ-6 on AC-2/3/4), C-TEST-3/4/5/7
- api.md command surface used as-is; no new names
- Next: W38 — mutation gate (`docs/dev-plan.md`)
