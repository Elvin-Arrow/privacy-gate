# [0002] Design spec generation and three-model review

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Produce the component-level design spec for Privacy Gate v1, between the SRS (requirements) and
the architecture/API/UI/testing specs. Resolve the design-owned open questions (OQ-1, OQ-2, OQ-7,
OQ-15, OQ-16), defer the architecture-owned ones, and run the three-model review per decision
0001.

## Implementation

- Asked two scoping questions: design-vs-architecture boundary (chose "component-level design"),
  and tech stack (chose Tauri + Rust + TS).
- Created decision 0003 (v1 tech stack: Tauri 2.x, Rust core, TS frontend; macOS/Windows/Linux;
  in-process detection; in-process first-party plugins for v1).
- Drafted `docs/specs/design.md`: eight components (Importer, Detector, Approval Engine, Vault,
  Share Engine, Audit Trail, Plugin Host, TS Frontend), data structures (Document,
  ApprovedVersion, FieldDecision), the canonical Import→Detect→Approve→Store→Share flow,
  variant lifecycle, overlapping-field precedence, re-import behavior, and a performance budget.
- Updated `specs/index.md`, `decisions/index.md`, and `notes/open-questions.md` to reflect the
  new spec and resolved OQs.
- Ran three-model review (Gemini, gpt-oss, qwen-3.5); raw output in `docs/notes/reviews/`.

## Problems Encountered

- **Critical data-model flaw (all 3 reviewers):** `ApprovedVersion` stored only span decisions,
  not rendered content. Once an original was discarded (the GP-letter flow), export and AI-share
  had no content to render. The user-story flow was broken.
- **Missing components (Gemini, gpt-oss):** no component owned global retention config (FR-1.4)
  or first-run account/key creation (FR-8.1/8.2).
- **Audit Trail unimplementable (qwen, gpt-oss):** "tamper-evident" responsibility with no
  integrity data fields and no read/query interface.
- **Circular deferrals (Gemini):** OQ-4-remainder and OQ-6 were tagged "design + UI/testing" —
  design was punting its own share.
- **C-DES-1 contradiction (Gemini):** frontend was forbidden from seeing not-yet-redacted
  content, but the review/approve step requires it.
- **Overlap rule incomplete (Gemini, gpt-oss):** "innermost wins" only handles strict nesting,
  not partial overlaps.
- **Variant/ShareRequest structs missing (all 3);** raw-bytes destruction hand-off ambiguous
  (gpt-oss); Plugin Host interface didn't say overrides are pre-applied (qwen); Vault missing
  variant ops (qwen); first-paint budget leaked to UI (gpt-oss).

## Resolution

- Added `RedactedDocument` to `ApprovedVersion` so discard-original shares work; updated §3.2,
  §3.3, and the §4 Approval-Engine→Vault interface.
- Added two components: **Config** (retention default + paranoid enforcement) and **Key
  Manager** (first-run account, passphrase, key gen, unlock; boundary with Vault).
- Added `AuditEntry` struct with `prev_entry_hash` + `entry_signature` (§3.8) and a read/query
  interface (§2.6, §4); crypto primitive stays architecture spec.
- Resolved the design halves of OQ-4 (single-doc = PDF; bundle order = user selection order,
  §3.7) and OQ-6 (assertion true iff discard or share transmits only approved version, §2.6);
  remainders stay open for UI/API/testing.
- Fixed C-DES-1 to distinguish review/approve (frontend sees unapproved content + spans) from
  share preview (frontend sees only redacted artifact).
- Completed the overlap rule for partial overlaps (Redact wins unless a strictly nested
  sub-span is kept, §3.5) and added `DetectedField.parent_field_id` (§3.1).
- Added `Variant` (§3.4) and `ShareRequest` (§3.7) structs; Vault variant store/load/delete ops
  (§2.4, §4); raw-bytes destruction hand-off to Vault (§2.1); Plugin Host receives
  overrides-applied content (§2.7); moved first-paint budget to UI spec ownership (§7).
- Rejected gpt-oss's "OS list is design creep" (NFR-PORT1 explicitly defers to design; decision
  0003 records it) and "export = true removal is implementation leakage" (decision 0002 settled
  it as a requirement; only the mechanism is architecture spec).

## Verification

- Design spec traces every FR/NFR to a component in §8, including the new Config, Key Manager,
  and the Audit Trail read interface.
- Open-questions register marks OQ-1, OQ-2, OQ-7, OQ-15, OQ-16 resolved, and OQ-4/OQ-6
  partially resolved (design parts); OQ-3, OQ-4-remainder, OQ-6-remainder, OQ-5, OQ-12, OQ-13,
  OQ-17, OQ-18 remain open and tagged with their owner.
- Knowledge-governance skill applied: design spec in `docs/specs/`, decision 0003 in
  `docs/decisions/`, this log in `docs/dev-log/`, indexes updated, links verified.

## Lessons

- The ApprovedVersion data model was the highest-impact catch — a spec that looks complete at
  the component-responsibility level can still be broken at the data-structure level. Always
  trace a representative flow (the user-story GP-letter discard-original case) end-to-end through
  the data structures, not just the component boxes.
- Three-model review again converged on the critical items (ApprovedVersion content, missing
  components, audit-trail implementability) and diverged usefully on smaller ones (gpt-oss
  flagged the raw-bytes hand-off; qwen flagged the Vault variant-ops gap; Gemini flagged the
  circular deferrals).
- "Defer to downstream spec" must be specific about *which part* is deferred; "design + UI" is
  not a real owner. Splitting OQ-4 and OQ-6 into design/remainder parts fixed two circular
  deferrals.

## Related Documentation

- [Spec — design](../specs/design.md)
- [Spec — SRS](../specs/srs.md)
- [Decision 0001 — review approach](../decisions/0001-multi-model-spec-review.md)
- [Decision 0002 — resolved SRS clarifications](../decisions/0002-resolved-srs-clarifications.md)
- [Decision 0003 — v1 tech stack](../decisions/0003-v1-tech-stack.md)
- [Open questions](../notes/open-questions.md)
- [Raw reviews](../notes/reviews/)