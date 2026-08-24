# Data Model Specification Review: Privacy Gate v1

Reviewer: Claude (via `claude -p`; keychain auth, not `--bare`). Date: 2026-08-23.

(Raw Claude output from the shared review prompt.)

## A. Alignment data-model → SRS

The packet contains no SRS text, only "parent facts." Against those: `document_meta` (§5, kind 8) is explicitly framed as filling "FR-4.3 catalog metadata... without putting `raw_bytes` in the catalog" and is cross-referenced in §13. `document.approved_artifact_id` is claimed as the FR-3.2 "one canonical version" mechanism, backed by the `UNIQUE` constraint on that column (§6) and C-DM-4 (§12). Both claims are internally consistent with the stated FR labels, but I cannot verify the FR text itself wasn't reworded — flag as unverifiable, not wrong.

## B. Alignment data-model → design

- Design's `Document`, `ApprovedVersion`, `Variant`, `AuditEntry` each get a table or artifact kind (§4, §6). Consistent.
- `ShareRequest` (design: ephemeral) is correctly kept out of SQL (§3 "Process memory only", §10), but §10's row for it is anonymized as "Share preview token + PDF/AI bytes" rather than named `ShareRequest` — a traceability nit, not an error.
- §5.1 cites design's `DetectedField` field list (`ids, label, classification, byte_offset, byte_length, page_index, parent_field_id`) but then asserts in the same paragraph that "`text` is stored here" for `open_approval`. The JSON schema block for `DocumentMetaRecord` shows no field carrying that text (no `text`/`span` key anywhere under `detected_fields`). Either the cited design field list is incomplete or the JSON schema is — this is a real gap between §5.1's prose and its own schema (see §F/I).

## C. Alignment data-model → architecture

- AAD kinds 1–7 are architecture's; kind 8 is this spec's addition, called out explicitly (§5, §6 CHECK comment, C-DM-3). Consistent with the parent fact.
- Audit HMAC/canonical row encoding is correctly deferred, not restated (§8: "Canonical encoding of the row for HMAC is architecture §6.1... This spec does not restate that byte layout"). Good scope discipline.
- `account.display_name` SQLCipher-only / not-envelope matches the architecture note cited in §3 and §6. Consistent.
- Cloud AI secret: §5.6 keeps it envelope-encrypted (kind 5) and stores `key_last4` as the one field explicitly called out as non-secret — consistent with the parent fact.

## D. Alignment data-model → API (incl. decision 0007)

- §1 and §13 state API DTOs are "projection of envelopes + audit; not a second schema" — matches the parent fact verbatim and is respected: no DTO shapes are defined here, only server-side records.
- Decision 0007 (retain/discard gating on first import) is represented purely as data: `ConfigRecord.confirmed` (§5.5) and the factory default `policy=discard, confirmed=false`. The spec correctly does not duplicate the *gating logic* (that belongs to API/command layer) — good boundary-keeping, cited in §15.
- §7's `import_document (confirmed policy)` row assumes the gate already happened upstream; the data model never states what a rejected/unconfirmed import does to rows, which is correct scope (API's job) but should be a one-line note rather than left implicit, since a reader could otherwise think unconfirmed imports are also modeled here.

## E. Alignment ↔ idea (local-first, later sync, no hosted DB)

§11 is a clean match to the parent fact set: artifacts as copyable ciphertext units, no v1 server DB, `DocId` not treated as global identity, no assumption of a pre-shared `vault_master_key`, no recovery/key-migration. It also states "v1 does not add columns 'for sync'" — actively resists scope creep rather than just deferring it. Good.

## F. Quality / implementability

1. **§5.1 vs its own JSON schema**: prose claims `text` "is stored here" for `open_approval` post-lock, but neither the `DocumentMetaRecord` block nor `DetectedFieldRecord`'s cited field list shows where that text lives. As written, an implementer cannot locate the byte offset → text mapping needed to redraw the approval UI after lock. This is the single largest hole in the spec.
2. **`audit_entry.originals_flag`** (§6): three-state encoding (`0 unset, 1 false, 2 true`) is defined with no semantic meaning and no mapping to the event-type payload table in §8. Every other audit column traces to §8; this one doesn't.
3. **C-DM-4 ("Variants require approved")** is a stated constraint (§12) but §6's Integrity paragraph only commits to enforcing the *kind* FKs "in Vault code," not the approved-before-variant invariant — leaves an unenforced invariant undocumented as to where it's checked.
4. Minor: `variant.name` cache-vs-envelope split (§6) is well-reasoned (integrity-failure-on-mismatch), a good implementability call, not a defect.

## G. Scope discipline

Strong. §14 (Deferred) explicitly excludes UI ephemeral state, sync/merge schema, device list, key migration, extra plugin_secret kinds, and a `display_name` envelope. §11 similarly fences off sync with a "MUST NOT assume" list rather than silently leaving it open. No crypto algorithm detail leaks in from architecture, no Tauri command shapes leak in from api.md (§1 says so directly). This section is a model of not inventing requirements.

## H. Deferral health

Deferrals are justified, not swept-under-rug: each item in §14 has either an owning future phase (sync) or an owning parent (UI-spec) named. §11's non-goals are testable ("MUST NOT... a Privacy Gate–hosted SQL server in v1") rather than vague. One weak spot: the §5.1 text-location gap (F.1) is not listed as a deferral — it reads as settled ("is stored here") when it isn't, which is worse than an honest deferral because it will silently mislead an implementer instead of prompting a question.

## I. Top 5 changes

1. Fix §5.1: either add the missing text/span field to `DocumentMetaRecord`'s JSON schema (or to `DetectedFieldRecord`) or correct the prose to say where post-lock approval text actually lives — this is a blocking ambiguity for `open_approval`.
2. Document `audit_entry.originals_flag` semantics and its mapping into §8's per-event payload table, or drop the column if unused.
3. State explicitly in §6's Integrity paragraph (alongside the kind-FK note) that C-DM-4 (variant requires approved) is enforced in Vault code, matching how the kind checks are handled.
4. Rename or annotate §10's "Share preview token + PDF/AI bytes" row to name it as design's `ShareRequest`, for 1:1 traceability with design.md.
5. Add one sentence to §7 noting that unconfirmed-policy import attempts are rejected upstream (API layer) and produce no rows here, to close the implicit gap noted in D.
