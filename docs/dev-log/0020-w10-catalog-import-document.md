# [0020] W10 — Catalog and `import_document` (no detector yet)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Deliver `import_document`, `list_documents`, `get_document` (api.md §5.3) and the catalog
storage underneath them: `DocumentMeta` (kind 8) and, when retained, `OriginalRecord`
(kind 2), both envelope-encrypted and document-scoped, plus the `document` SQL row joining
them. Detection is a no-op empty field list — dev-plan.md's own escape hatch ("Detection may
be a no-op empty field list only if W12 is the next PR"), which it is, in this session's
sequence.

Explicitly **not** in this chunk (per its own "Do not" list plus what W11 owns instead):
`retention_policy_unset` when the global default isn't confirmed, `retention_loosen_forbidden`
for a `retain` override against `never_retain`, real detection, `already_approved` paths,
approval UI.

Per the [agent roster](../agent-roster.md), W10 is Sonnet tier, no mandatory second review
("CRUD over data-model schema").

## Implementation

### `DetectedField` landed here, not in W12

`crate::importer`'s W8/W9 module docs predicted the Detector's chunk would introduce
`DetectedField`. That was wrong in one narrow sense: `DocumentMeta.detected_fields`
(data-model §6.1) is typed against it, and `DocumentMeta` is this chunk's own deliverable —
a struct field can't be typed against a type that doesn't exist. `DetectedField` is defined
in the new `core/src/catalog.rs`, but **W10 never constructs a non-empty one** — a
`NullDetector` (`core/src/detector.rs`, also new) is the only thing that ever runs, and it
returns `Vec::new()` unconditionally. `SessionManager::with_detector` is the seam W12 fills
without reshaping `import_document`'s call site.

### The audit-persist cadence had to be built now, not deferred again

`import_document` is the first command that actually appends an audit row — W5 built the
whole chain but had "no production command exists yet" as its own explicit scope fence.
Architecture §6.2 specifies the persist cadence as "on: lock, process-exit flush, after
every Share event, and every 32 appends" — if that cadence weren't implemented, a session
that imported more than 32 documents without locking would report a false
`degraded_integrity` on its next unlock (32 unpersisted appends is the crash-window
ceiling; import #33 would look identical to real tampering). This is a correctness gap, not
scope creep, so it's built here:

- `OpenSession` gained `live_head: AuditHead` and `appends_since_persist: u32`.
- `SessionManager::record_audit_append` (new, private) — appends via `crate::audit::append`,
  advances `live_head` via the new `crate::audit::head_for` (a public wrapper the module
  needed anyway to keep append-side and verify-side head computation from ever diverging),
  and persists to the keystore immediately at 32 unpersisted appends.
- `lock` now persists `live_head` if it's ahead of what's stored — best-effort (a failure
  doesn't fail `lock` itself; the crash-window fast-forward exists precisely to tolerate a
  missed persist within the window).

### The `VaultMasterKey`-has-no-`Clone` constraint, worked around correctly

`import_document` needs `&VaultMasterKey` (for `Config`/`DocumentStore` calls) **and**
`&mut self` (for `record_audit_append`) in the same function. `VaultMasterKey` deliberately
has no `Clone` (`crate::keys`: "every copy is another place key material must be
destroyed"), so the fix is not to extract-and-hold a `master` binding across the whole
function — that would tie an immutable borrow of `self.open` to the function's scope and
conflict with the later `&mut self` call. Instead, `&self.require_open()?.master` is
re-borrowed at each call site that needs it, each borrow ending at that statement, so the
later `&mut self` call is never in conflict. No unsafe, no cloning, no new type.

### `SessionManager` constructor growth stopped here

W10 needed a 6th backend (`documents: Arc<dyn DocumentStore>`) on top of keystore/accounts/
vault/audit/config. Rather than another `new_with_x_and_y` positional overload,
`with_documents`/`with_detector` are builder-style methods on top of the existing
`new_full` — every prior constructor (`new`, `new_with_vault`, `new_with_vault_and_audit`,
`new_full`) is untouched, and any future backend gets a `with_x` method instead of growing
the constructor list further.

### Format switch, filename, and error mapping

- `import_document` sniffs `bytes.starts_with(b"%PDF-")` to choose `import_pdf` vs.
  `import_text` — content, not the caller-supplied filename (a mislabeled file must still
  go through the extraction path matching its real bytes).
- `validate_import_filename` rejects both `/` and `\` (not just the host OS's separator)
  and empty input.
- `ImportTextError`/`ImportPdfError` (both non-secret, per W8/W9) collapse to
  `unsupported_document` at this command layer — exactly the deferral both modules'
  documentation predicted.
- data-model §6.1's `never_retain` → document `retention: discard` mapping is implemented
  here (not deferred to W11): it's not the confirmation *gate*, just what a document row
  under a paranoid default is required to say about itself.

## Resolution

- `cargo test -p pg-core` green: **20/20** new in `catalog_w10.rs`, all prior tests (W1
  through W9, 173 total) unmodified and green.
- Full workspace `cargo test` and `npm run check` both green; `cargo clippy -p pg-core
  --all-targets` zero warnings on every file this chunk touches.
- dev-plan W10 "Tests first" line, verified: basename only (both `/` and `\` rejected,
  empty rejected); `over_budget` true still completes (25 MB + 1 byte still imports, with
  the flag set; under-budget is unflagged); two imports of identical bytes → two `doc_id`s;
  `get_document` has no span text (checked both by value equality with the import response
  and structurally — `DocumentSummary`'s serialized keys are exactly api.md §4's eight
  fields, nothing more); newest first (checked fresh and after a lock/unlock round trip).
- Extra coverage: the `never_retain`/`retain`/per-import-override retention resolution
  (data-model §6.1), the format switch dispatching on content not filename, the new
  audit-persist cadence actually producing a clean (not fast-forwarded, not degraded)
  unlock after a single import + lock, and a retained original surviving a lock/unlock
  cycle intact.
- Scope held: no real detection (`detected_field_count` is `0` everywhere in this file), no
  `retention_policy_unset`/`retention_loosen_forbidden` gate, no approval command, no UI.

Next: W11 — Import blocked until retention confirmed.

## Related Documentation

- [Development Plan — W10 specification](../dev-plan.md#w10--catalog-and-import_document-no-detector-yet)
- [Agent roster — W10](../agent-roster.md)
- [Spec — API §5.3 (import/catalog commands), §4 (`DocumentSummary`)](../specs/api.md)
- [Spec — SRS FR-1.3–1.5](../specs/srs.md)
- [Spec — Data model §6.1 (`DocumentMeta`), §6.2 (`OriginalRecord`)](../specs/data-model.md)
- [Dev log 0019 — W9 import PDF](./0019-w9-import-pdf.md)
