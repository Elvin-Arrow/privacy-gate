# [0021] W11 — Import blocked until retention confirmed

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Close the gap W10 deliberately left open: `import_document` → `retention_policy_unset`
when the global retention default isn't confirmed (decision 0007, AC-7), and
`retention_loosen_forbidden` for a per-import `retain` override against a `never_retain`
default (AC-6).

Explicitly **not** in this chunk (dev-plan.md W11 "Do not: UI first-import modal (W32)"):
no UI, no pre-select chrome — testing.md §6.7 itself says this scenario "only asserts the
API gate and factory value."

Per the [agent roster](../agent-roster.md), W11 is Sonnet tier, no mandatory second review
("Gate logic, low ambiguity").

## Implementation

### `core/src/session.rs` — `import_document` reordered

Two checks inserted right after filename validation, **before** the Importer or Detector
run at all (dev-plan W11 "Integrate: Importer reads Config before detect" — read as "before
touching the Importer," not merely "before the catalog write"):

1. `!config.confirmed` → `retention_policy_unset`. No extraction, no detection, no catalog
   row — matches AC-7's literal wording ("`import_document` (any override, including null)
   → `retention_policy_unset`; no catalog row"), including the case where the caller
   supplies a perfectly valid override; confirmation gates the command outright, not just
   the default-inference path.
2. `config.policy == NeverRetain && retention_override == Some(Retain)` →
   `retention_loosen_forbidden`. Only the *retain* direction is checked — a per-import
   `discard` override against `never_retain` is tightening (already the enforced outcome),
   never forbidden, and "no override at all" under `never_retain` isn't a loosening attempt
   either (data-model §6.1's mapping already produces `discard` in that case).

Retention resolution itself (the `never_retain` → document `retention: discard` mapping)
already existed from W10 — this chunk only added the two gates in front of it, reusing the
same `config` value already being loaded rather than a second read.

### `core/src/api.rs` — two new `ApiError` constructors

`retention_policy_unset()` and `retention_loosen_forbidden()`, matching the existing
`unlock_failed()`/`account_exists()`/`passphrase_mismatch()` pattern — both `ErrorCode`
variants already existed (declared since W2, per `api.rs`'s "carries the whole api.md §3
list even though W2 only produces..." note); this chunk just gave them the constructors
their call sites needed.

## Resolution

- `cargo test -p pg-core` green: **10/10** new in `retention_gate_w11.rs`, all prior tests
  (W1 through W10, 193 total) unmodified and green — including every `catalog_w10.rs` test,
  none of which needed touching because they all already confirm a policy before importing
  (matching C-TEST-6's "Paranoid tests call `set_retention_default` first," which turns out
  to be the right discipline for *every* import test, not just paranoid-default ones).
- Full workspace `cargo test` and `npm run check` both green; `cargo clippy -p pg-core
  --all-targets` zero warnings on every file this chunk touches.
- dev-plan W11 "Tests first" line, verified: the full AC-7 command scenario (factory
  discard/unconfirmed; unconfirmed import refused even with an override; `set_retention_default`
  confirms for all three policies; import proceeds after confirmation; confirming discard
  then overriding that same first import to retain is allowed — FR-1.3 vs. FR-1.4 staying
  distinct); the full AC-6 paranoid scenario (retain override forbidden; discard override
  allowed; no-override imports as discard; non-paranoid defaults never trigger the
  forbidden path).
- Scope held: no UI, no pre-select chrome, no change to detection or catalog storage beyond
  the two new early-return checks.

Next: W12 — Detector host + stub (unblocks AC-1).

## Related Documentation

- [Development Plan — W11 specification](../dev-plan.md#w11--import-blocked-until-retention-confirmed)
- [Agent roster — W11](../agent-roster.md)
- [Decision 0007 — retention default is discard; first import must confirm (OQ-14)](../decisions/0007-retention-default-discard.md)
- [Spec — API §5.3 (`import_document`)](../specs/api.md)
- [Spec — Testing §6.6 (AC-6), §6.7 (AC-7)](../specs/testing.md)
- [Dev log 0020 — W10 catalog and import_document](./0020-w10-catalog-import-document.md)
