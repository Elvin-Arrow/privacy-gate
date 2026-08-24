# [0018] W8 — Import plain text

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Deliver the Importer's plain-text path (design §2.1, §3.1): extract UTF-8 bytes into the
in-memory `Document`/`Page`/`TextSpan` representation (data-model §5.1). Library-only, per
dev-plan.md's explicit preference ("prefer W8 as a library + W10 as the command") — no
Tauri command, no `SessionManager` method, no vault I/O.

Explicitly **not** in this chunk (dev-plan.md W8 "Do not: PDF, detection, retention"): no
PDF extraction (W9), no `DetectedField` (W12 — that type doesn't exist in this codebase at
all yet), no retention-confirmed gate or filename validation (both command-layer, W10/W11).

Per the [agent roster](../agent-roster.md), W8 is Sonnet tier, no mandatory second review
("Straightforward pipeline stage").

## Implementation

### `core/src/importer.rs` — the new module

- `SourceFormat`, `TextSpan`, `Page`, `Document` — data-model §5.1's IR types, transcribed
  directly. `SourceFormat::Pdf` exists on the enum (the type is shared with W9) but nothing
  in this module ever constructs it.
- `import_text(bytes, doc_id) -> Result<Document, ImportTextError>` — the whole function.
  Plain text has no inherent page concept, so the whole document becomes one `Page` with one
  `TextSpan` covering the entire decoded string (`byte_offset: 0`,
  `byte_length: bytes.len()`). `byte_length` is an **octet** count, not a character count —
  load-bearing for any multi-byte UTF-8 content, since spans must be byte-sliceable back
  into `raw_bytes` for later preview/export.
- `doc_id` is a caller-supplied parameter, not something `import_text` mints. This is
  deliberate scope discipline, not an oversight: testing.md §8's "Re-import" row requires
  two imports of identical bytes to produce two different `doc_id`s, which is only possible
  if id assignment lives at the catalog layer (W10) — a content-derived id inside the
  Importer would make that requirement unsatisfiable later.
- `ImportTextError::{Empty, NotUtf8}` — the library's own signal for "nothing here" / "not
  UTF-8." Deliberately **not** `ApiError`/`unsupported_document`: dev-plan W8 names that
  mapping as W10's job, and this module has no dependency on `crate::api` or
  `crate::session` at all.

### `core/testdata/w8_sample.txt` — the first fixture directory

New `core/testdata/`, per dev-plan W8 "Done when: unit tests on fixtures in `testdata/`
(synthetic, not real PII)." One small synthetic letter with a planted canary string
(`PG-FIXTURE-CANARY-0001`) and an `@example.invalid` contact — no real personal data,
matching testing.md §7.2's fixture discipline even though this chunk does no detection.

## Resolution

- `cargo test -p pg-core` green: **8/8** new in `importer_w8.rs`, all prior tests (W1
  through W7, 157 total) unmodified and green.
- Full workspace `cargo test` and `npm run check` both green; `cargo clippy -p pg-core
  --all-targets` zero warnings on every file this chunk touches.
- dev-plan W8 "Tests first" line, verified: `.txt` bytes → pages
  (`txt_bytes_become_one_page_with_one_span_covering_the_whole_text`, plus a multi-byte
  UTF-8 test proving `byte_length` is octets not characters); empty input's library-level
  signal (`empty_input_is_refused`) — the API-layer `unsupported_document` mapping is
  explicitly deferred to W10, both in the module doc and in this test file's own header;
  filename path-separator validation not attempted here at all (command-layer, W10) —
  `import_text` doesn't take a filename.
- Extra coverage beyond the literal "Tests first" list, still within FR-1.1/1.2's stated
  scope: invalid UTF-8 and a truncated multi-byte sequence are both refused rather than
  lossily reinterpreted (FR-1.2: "never silently process them as if redactable");
  `import_text` never derives `Document.id` from content (the property the W10 catalog's
  re-import uniqueness will depend on).
- Scope held: no `DetectedField`, no `SessionManager`/`ApiError` dependency anywhere in
  `importer.rs`, no PDF construction, no filesystem write.

Next: W9 — Import PDF (text-bearing) and reject scans.

## Related Documentation

- [Development Plan — W8 specification](../dev-plan.md#w8--import-plain-text)
- [Agent roster — W8](../agent-roster.md)
- [Spec — Design §2.1 (Importer), §3.1 (in-memory IR)](../specs/design.md)
- [Spec — Data model §5.1 (`TextSpan`, `Page`, `Document`)](../specs/data-model.md)
- [Spec — SRS FR-1.1, FR-1.2](../specs/srs.md)
- [Spec — Testing §8 "Re-import" row](../specs/testing.md)
- [Dev log 0017 — W7 Linux keystore fallback](./0017-w7-linux-keystore-fallback.md)
