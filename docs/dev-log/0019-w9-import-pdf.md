# [0019] W9 — Import PDF (text-bearing) and reject scans

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Extend the Importer (W8) with PDF: "PDF with extractable text → pages + byte offsets. No
text → `unsupported_document`. In-memory PDF I/O only" (dev-plan.md W9). Still
library-only — no command, no vault, no disk I/O.

Explicitly **not** in this chunk (dev-plan.md W9 "Do not: re-render export (W23); OCR"): no
PDF writing (architecture §11's from-scratch export writer is a separate, later
dependency), no OCR fallback for image-only pages, no detection, no `over_budget` flag or
`unsupported_document` API mapping (both W10).

Per the [agent roster](../agent-roster.md), W9 is Sonnet tier, no mandatory second review
("Parsing/validation logic, well-bounded").

## Implementation

### `core/src/importer.rs` — `import_pdf`

- New dependency: `pdf-extract` (pure Rust, built on `lopdf` internally) —
  `extract_text_from_mem_by_pages` never touches a file path, satisfying architecture
  §5.1's "PDF import and export run in memory... If a library cannot be configured for
  memory-only I/O it shall not be used."
- One `Page` per PDF page, one `TextSpan` per page covering that page's whole extracted
  text — the same "no finer granularity than the Detector needs yet" choice `import_text`
  made for the whole document, applied per page since PDF page boundaries are real
  structure. `byte_offset`/`byte_length` are into the **extracted text of that page**, not
  the raw PDF file bytes — a raw-file-byte offset couldn't locate anything meaningful once
  content streams are decoded/decompressed.
- `ImportPdfError::{NoText, Malformed}` — `NoText` when every page has no visible-glyph
  text; `Malformed` when the bytes don't parse as a PDF at all. Kept distinct because
  they're different failures ("read it, found nothing" vs. "could not read it"), even
  though both are likely to map to the same `unsupported_document` API code later (W10's
  call, not this chunk's).

### The `NoText` check had to be more than `.trim().is_empty()`

Building the "image-only PDF" test fixture as a page with a **whitespace-only** `Tj`
string (rather than an empty content stream) surfaced a real `pdf-extract` quirk: it emits
stray control/null characters as text-positioning artifacts even when the only content
shown is whitespace. A plain `.trim().is_empty()` check — Unicode whitespace only — left
those null bytes behind, so the page read as "has text" when it plainly has none. Fixed by
checking for **any non-whitespace, non-control character** instead of trim-and-emptiness;
the FR-1.2 property that actually matters is "no visible glyph," and neither whitespace nor
control characters are visible glyphs. Caught by a test that was originally meant to be a
minor variant of the empty-content-stream fixture, not a targeted regression test — worth
noting because it's exactly the kind of gap a hand-picked "obviously empty" fixture alone
would have missed.

### `core/tests/importer_w9.rs` — fixtures built with `lopdf`, not hand-typed bytes

Both the born-digital and no-text PDF fixtures are constructed programmatically with
`lopdf` (a dev-dependency; already pulled in transitively by `pdf-extract`, so this adds no
new dependency tree, only pins the version directly for test use). `lopdf` computes its own
xref table and byte offsets from the objects it's given, so the fixtures are guaranteed
structurally valid regardless of their text content — a hand-maintained PDF byte literal
with manually computed xref offsets would be fragile and error-prone to get right, and
wrong in a way that would silently produce a `Malformed` result instead of exercising the
intended code path.

## Resolution

- `cargo test -p pg-core` green: **8/8** new in `importer_w9.rs`, all prior tests (W1
  through W8, 165 total) unmodified and green.
- Full workspace `cargo test` and `npm run check` both green; `cargo clippy -p pg-core
  --all-targets` zero warnings — including a drive-by fix of two pre-existing warnings in
  `core/src/keystore/mod.rs`'s `from_hex` (unrelated to this chunk; surfaced only because
  this was the first `cargo clippy --all-targets` run since a stricter lint
  (`chunks_exact_to_as_chunks`) started firing on the toolchain in use) — fixed since they
  were trivial and the project's standing bar is zero warnings on every touched build, not
  just this chunk's own new files.
- dev-plan W9 "Tests first" line, verified: born-digital PDF fixture extracts a known
  canary (`born_digital_pdf_extracts_the_known_canary`, plus a two-page variant proving
  `page_index` tracks correctly); image-only PDF rejected
  (`no_text_pdf_is_rejected` via an empty content stream, standing in for a scanned/image
  page — `import_pdf` only ever looks at text-showing operators, so this exercises the
  same code path a real embedded-image page would); watcher: no plaintext sidecar files
  (`import_pdf_writes_no_file_to_disk`, plus the structural fact that `import_pdf` takes no
  path parameter at all).
- Extra coverage: `Malformed` vs. `NoText` kept distinct and both tested; `import_pdf`
  never derives `Document.id` from content (same re-import property W8 established).
- Scope held: no PDF writing, no OCR, no detection, no `over_budget`/`unsupported_document`
  mapping.

Next: W10 — Catalog and `import_document` (no detector yet).

## Related Documentation

- [Development Plan — W9 specification](../dev-plan.md#w9--import-pdf-text-bearing-and-reject-scans)
- [Agent roster — W9](../agent-roster.md)
- [Spec — SRS FR-1.1, FR-1.2](../specs/srs.md)
- [Spec — Architecture §5.1 (in-memory PDF I/O)](../specs/architecture.md)
- [Spec — Design §2.1 (Importer), §3.1 (in-memory IR)](../specs/design.md)
- [Dev log 0018 — W8 import plain text](./0018-w8-import-plain-text.md)
