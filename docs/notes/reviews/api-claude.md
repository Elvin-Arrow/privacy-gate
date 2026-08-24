# API Specification Review: Privacy Gate v1

Reviewer: Claude (via `claude -p`; keychain auth, not `--bare`). Date: 2026-08-23.

(Raw Claude output from the shared review prompt, lightly kept as received.)

## A. Alignment API → SRS

- Covered adequately for FR-1 through FR-8 in the traceability table. No missing FR surfaced.
- No contradictions found (FR-1.4, FR-4.3 honored).
- Borderline: `unlock_failed` Argon2id/lockout rationale is not a command-contract fact.
- `cloud_ai_test` handshake examples are illustrative, not SRS-derived.

## B. Alignment API → design

- Approval lifecycle, one-at-a-time, re-import, overlap-in-core, no-variant-edit: correctly reflected.
- **Defect:** `set_field_decisions` always returns `lifecycle: "decided"` even with `unresolved_field_ids`.
- **Gap:** no defined behavior for `open_approval` when `has_approved_version == true`.

## C. Alignment API → architecture

- Capability ACL, keys never returned, degraded session, Cloud AI HTTP only at commit: compliant.
- Preview = exactly what leaves: enforced for PDF; **not** for the AI payload.
- Inbound `import_document` bytes vs "originals never cross IPC": need an explicit direction.

## D. Alignment API ↔ idea

- Local-first, key on device, detection on-device, vault-as-product, export-only share-to-person: aligned.

## E. Quality / implementability

1. `set_field_decisions` lifecycle typing bug.
2. No byte-identity guarantee for the AI share payload.
3. Preview-token: one live token; no `expires_at`; re-preview races commit.
4. `over_budget` semantics undefined (still processed?).
5. `approval_conflict` conflates two causes.
6. Re-approval gap.

## F. Scope discipline

- Crypto leakage: Argon2id/lockout rationale belongs in architecture, not the API error table.
- `cloud_ai_test` examples undercut the §12 implementation-choice deferral.
- OQ-14 not silently defaulted. No UI/testing leakage of substance.

## G. Deferral health

- OQ-4, OQ-6 remainder, OQ-12, OQ-14: healthy.

## H. Top 5 changes

1. Fix `set_field_decisions` lifecycle contract.
2. Add identity guarantee for AI `ai_payload_preview` vs commit POST body.
3. Remove Argon2id/lockout rationale from the API error table.
4. Define re-approval semantics (`open_approval` on an already-approved doc).
5. Add `expires_at` to `SharePreview` and clarify `over_budget` handling.
