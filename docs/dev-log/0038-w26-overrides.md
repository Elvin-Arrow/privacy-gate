# [0038] W26 — Ephemeral overrides + variants on share (AC-2)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Let a user override the canonical `ApprovedVersion`'s keep/redact decisions for a single
share (FR-5.4), and apply a saved variant at preview (FR-5.5), without ever mutating the
canonical version, and surface `overrides_in_effect` per FR-6.2. `submit_approval` (W18)
discards the original `Document` after storing `redacted_content`, so re-rendering with a
different decision set can't just re-run the W23 pipeline against `Document.raw_bytes` —
that's gone by design (dev-plan §3.3).

## Implementation

- `core/src/overlap.rs`:
  - Split `redact_document` into a thin wrapper over a new `redact_pages(format, pages,
    fields, decisions)` — same precedence rule, just no longer tied to a full `Document`.
  - Added `redact_with_overrides(canonical, redacted_content, effective_decisions)`. Works
    from what `ApprovedVersion.decisions` already keeps at rest: every `FieldDecision`
    carries its own field's span *text*, redacted or not — that's what makes "reveal more"
    possible without the retained original. It recomputes the canonical cut ranges via the
    existing `redacted_ranges`, slices the missing text back out of the covering field,
    merges that with the already-kept spans in `redacted_content`, and re-applies
    `redact_pages` with the override decision map. One precedence rule, reused, not a
    second policy.
- `core/src/session.rs` `preview_share`: per doc_id, builds an `effective` decision map
  from canonical ± `applied_variant_ids[doc_id]` (via `DocumentStore::load_variant`) ±
  `per_doc_overrides[doc_id]`, validating every overridden `field_id` is known
  (`invalid_input` otherwise) and every variant id resolves (`not_found` otherwise).
  `overrides_in_effect` is true iff either mechanism was actually used for any requested
  doc. Skips `redact_with_overrides` entirely (uses `approved.redacted_content` as-is)
  when `effective == canonical` — keeps the no-override path byte-identical to before this
  chunk. Manifest `visible_field_ids` / `redacted_field_ids` reflect the effective
  decisions, not the canonical ones.

## Resolution

- `core/tests/overrides_w26.rs`: pure `redact_with_overrides` tests (reveal a redacted
  field, hide a kept field, nested-field override, identity when overrides match
  canonical) plus session-level `preview_share`/`commit_share` tests — AC-2 reveal +
  `overrides_in_effect`, canonical preview unaffected by an earlier overridden one (vault
  approved unchanged), applied-variant reveal without mutating the variant, ad-hoc override
  layered on top of an applied variant, unknown `field_id`/`variant_id` error codes, and
  the W25 OQ-6 oracle holding when an override redacts further than canonical.
- `cargo test -p pg-core` green (all suites, W26 included); no clippy run this session —
  Docker Desktop's storage backend hit a host-level I/O error immediately after the green
  test run, before clippy/`npm run check` could run. Re-verify those once Docker is back.

Next: W27 — Cloud AI plugin (mock HTTP).

## Related Documentation

- [Development Plan — W26](../dev-plan.md#w26--ephemeral-overrides--variants-on-share-ac-2)
- [Spec — srs.md FR-5.4/FR-5.5/FR-6.2](../specs/srs.md)
- [Spec — design.md §2.5/§3.4/§3.7/C-DES-5](../specs/design.md)
- [Spec — api.md §4 ShareRequestDto / §5.6 SharePreview](../specs/api.md)
- [Dev log 0037 — W25 OQ-6 oracle](./0037-w25-oq6-oracle.md)
