//! W8 — Import plain text.
//!
//! Spec sources:
//! - `docs/specs/design.md` §2.1 (Importer responsibilities), §3.1 (in-memory IR)
//! - `docs/specs/data-model.md` §5.1 (`TextSpan`, `Page`, `Document`)
//! - `docs/specs/srs.md` FR-1.1 (import born-digital text), FR-1.2 (reject non-extractable
//!   input with a clear message, never silently treat it as redactable)
//! - `docs/dev-plan.md` W8 ("Tests first: `.txt` bytes → pages; empty →
//!   `unsupported_document` at command layer (W10); path separators in filename rejected at
//!   command layer (W10).")
//!
//! `.txt` bytes → pages and the empty-input signal are this module's job and are tested
//! here. `unsupported_document` (the API error code) and filename validation are
//! explicitly **not** — dev-plan W8 names both as command-layer (W10) concerns, and
//! `import_text` doesn't even take a filename. No `SessionManager` or `ApiError` appears
//! anywhere in this file.
//!
//! Out of W8 scope and deliberately absent here: PDF (W9), detection (W12), retention
//! (config-gate is W11).

use pg_core::importer::{import_text, ImportTextError, SourceFormat};

const DOC_ID: &str = "00000000-0000-4000-8000-000000000001";

// ---------------------------------------------------------------------------
// dev-plan W8: ".txt bytes → pages"
// ---------------------------------------------------------------------------

#[test]
fn txt_bytes_become_one_page_with_one_span_covering_the_whole_text() {
    let bytes = std::fs::read("testdata/w8_sample.txt").expect("fixture must exist");
    let doc = import_text(&bytes, DOC_ID).expect("import_text on a synthetic .txt fixture");

    assert_eq!(doc.id, DOC_ID);
    assert_eq!(doc.source_format, SourceFormat::Text);
    assert_eq!(doc.raw_bytes, bytes, "raw_bytes must be exactly the input bytes");
    assert_eq!(doc.pages.len(), 1, "plain text has no inherent page concept");

    let page = &doc.pages[0];
    assert_eq!(page.spans.len(), 1);
    let span = &page.spans[0];
    assert_eq!(span.page_index, 0);
    assert_eq!(span.byte_offset, 0);
    assert_eq!(span.byte_length, bytes.len() as u64);
    assert_eq!(span.text.as_bytes(), bytes.as_slice());
    assert!(span.text.contains("PG-FIXTURE-CANARY-0001"));
}

/// A small, hand-written fixture with a multi-byte UTF-8 character, so `byte_length` is
/// verified to be an octet count and not a character count.
#[test]
fn byte_length_is_octets_not_characters_for_multi_byte_utf8() {
    // "café" — 4 characters, 5 bytes (é is 2 bytes in UTF-8).
    let bytes = "café".as_bytes();
    let doc = import_text(bytes, DOC_ID).expect("import_text on multi-byte UTF-8");
    let span = &doc.pages[0].spans[0];
    assert_eq!(span.byte_length, 5);
    assert_eq!(span.text, "café");
}

#[test]
fn raw_bytes_are_held_verbatim_in_the_returned_document() {
    let bytes = b"line one\nline two\n";
    let doc = import_text(bytes, DOC_ID).expect("import_text");
    assert_eq!(doc.raw_bytes, bytes);
}

// ---------------------------------------------------------------------------
// dev-plan W8: "empty" (library-level signal; the `unsupported_document` API mapping is
// W10's, per the module doc and this file's header)
// ---------------------------------------------------------------------------

#[test]
fn empty_input_is_refused() {
    let result = import_text(b"", DOC_ID);
    assert_eq!(result.unwrap_err(), ImportTextError::Empty);
}

// ---------------------------------------------------------------------------
// FR-1.1 scope: text import is UTF-8. Invalid UTF-8 must be refused, not lossily
// reinterpreted (FR-1.2: never silently treat non-extractable input as redactable).
// ---------------------------------------------------------------------------

#[test]
fn invalid_utf8_is_refused_not_lossily_decoded() {
    // 0xFF is not a valid UTF-8 lead byte in any position.
    let bytes: &[u8] = &[0xFF, 0xFE, 0x00];
    let result = import_text(bytes, DOC_ID);
    assert_eq!(result.unwrap_err(), ImportTextError::NotUtf8);
}

#[test]
fn truncated_multi_byte_utf8_sequence_is_refused() {
    // 0xE2 0x82 is the start of a 3-byte sequence (e.g. "€" is E2 82 AC) but is cut short.
    let bytes: &[u8] = &[b'a', 0xE2, 0x82];
    let result = import_text(bytes, DOC_ID);
    assert_eq!(result.unwrap_err(), ImportTextError::NotUtf8);
}

// ---------------------------------------------------------------------------
// dev-plan W8 "Do not: detection" — no DetectedField anywhere reachable from this module.
// ---------------------------------------------------------------------------

#[test]
fn document_type_has_no_detection_fields_yet() {
    // Compile-time proof by construction: `Document` has exactly the four data-model §5.1
    // fields this chunk owns. If a `detected_fields`-shaped field were added prematurely,
    // this struct-literal would need updating, making the scope violation visible in the
    // diff of *this* file rather than silently passing.
    let doc = pg_core::importer::Document {
        id: DOC_ID.to_string(),
        source_format: SourceFormat::Text,
        pages: vec![],
        raw_bytes: vec![],
    };
    assert_eq!(doc.id, DOC_ID);
}

// ---------------------------------------------------------------------------
// testing.md §8 "Re-import": two imports of the same bytes → two doc_ids. This module
// doesn't mint doc_ids (module scope fence), so the property here is narrower but load-
// bearing: import_text never derives an id from content, which is what would make that
// requirement impossible to satisfy at the command layer later.
// ---------------------------------------------------------------------------

#[test]
fn import_text_never_derives_the_document_id_from_content() {
    let bytes = b"identical content, different caller-supplied ids";
    let doc_a = import_text(bytes, "doc-a").expect("import_text");
    let doc_b = import_text(bytes, "doc-b").expect("import_text");
    assert_ne!(doc_a.id, doc_b.id);
    assert_eq!(doc_a.raw_bytes, doc_b.raw_bytes, "same bytes, different ids — the point");
}
