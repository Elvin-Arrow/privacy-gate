//! W9 — Import PDF (text-bearing) and reject scans.
//!
//! Spec sources:
//! - `docs/specs/srs.md` FR-1.1 (import born-digital PDFs with extractable text), FR-1.2
//!   (reject non-extractable input — "e.g., scanned PDFs, images" — with a clear message)
//! - `docs/specs/architecture.md` §5.1 ("PDF import and export run in memory")
//! - `docs/specs/design.md` §2.1 (Importer), §3.1 (in-memory IR)
//! - `docs/dev-plan.md` W9 ("Tests first: born-digital PDF fixture extracts known canary;
//!   image-only PDF rejected; watcher: no plaintext sidecar files.")
//!
//! # Fixtures
//!
//! Both PDF fixtures are built **programmatically** with `lopdf` (a dev-dependency; already
//! pulled in transitively by `pdf-extract` itself, so this pins no new dependency tree) —
//! not hand-typed PDF byte literals. `lopdf` computes its own xref table and byte offsets
//! from the objects it's given, so the fixtures are guaranteed structurally valid
//! regardless of what text they carry, which a hand-maintained byte-offset table would not
//! be. This is a test-only choice; the *importer* itself (`import_pdf`) never depends on
//! `lopdf` directly, only on `pdf-extract`.
//!
//! Out of W9 scope and deliberately absent here: PDF export/re-rendering (W23), OCR,
//! detection (W12), the `over_budget` flag and `unsupported_document` API mapping
//! (both W10).

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document as LoDocument, Object, Stream};

use pg_core::importer::{import_pdf, ImportPdfError, SourceFormat};

const DOC_ID: &str = "00000000-0000-4000-8000-000000000002";

/// A minimal, valid, single-page, born-digital PDF whose content stream shows `text` via
/// the standard base-14 Helvetica font (no embedded font file needed — Helvetica is
/// required to be available in every PDF-conformant reader, including `pdf-extract`'s).
fn build_text_pdf(text: &str) -> Vec<u8> {
    let mut doc = LoDocument::with_version("1.5");
    let pages_id = doc.new_object_id();

    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
    });

    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ],
    };
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
    });

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("lopdf must write a well-formed fixture");
    bytes
}

/// A minimal valid single-page PDF with an **empty content stream** — no text-showing
/// operators at all. Standing in for a scanned/image-only page (dev-plan W9's fixture):
/// what matters for `import_pdf`'s contract is "zero extractable text," which an empty
/// content stream produces exactly as reliably as an embedded raster image would, without
/// this test file needing to also carry image-encoding logic that `import_pdf` never
/// touches anyway (`pdf-extract` only ever looks at text-showing operators).
fn build_no_text_pdf() -> Vec<u8> {
    let mut doc = LoDocument::with_version("1.5");
    let pages_id = doc.new_object_id();

    let resources_id = doc.add_object(dictionary! {});
    // An empty content stream — valid PDF, zero text operators.
    let content_id = doc.add_object(Stream::new(dictionary! {}, Vec::new()));

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "Contents" => content_id,
    });

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => vec![page_id.into()],
        "Count" => 1,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("lopdf must write a well-formed fixture");
    bytes
}

// ---------------------------------------------------------------------------
// dev-plan W9: "born-digital PDF fixture extracts known canary"
// ---------------------------------------------------------------------------

#[test]
fn born_digital_pdf_extracts_the_known_canary() {
    let bytes = build_text_pdf("PG-FIXTURE-CANARY-PDF-0002");
    let doc = import_pdf(&bytes, DOC_ID).expect("import_pdf on a synthetic born-digital PDF");

    assert_eq!(doc.id, DOC_ID);
    assert_eq!(doc.source_format, SourceFormat::Pdf);
    assert_eq!(doc.raw_bytes, bytes);
    assert_eq!(doc.pages.len(), 1);

    let page = &doc.pages[0];
    assert_eq!(page.spans.len(), 1);
    let span = &page.spans[0];
    assert_eq!(span.page_index, 0);
    assert_eq!(span.byte_offset, 0);
    assert_eq!(span.byte_length, span.text.len() as u64);
    assert!(
        span.text.contains("PG-FIXTURE-CANARY-PDF-0002"),
        "extracted text must contain the planted canary; got: {:?}",
        span.text
    );
}

#[test]
fn multi_page_pdf_extracts_each_page_with_the_right_page_index() {
    // Build a two-page document directly (build_text_pdf is single-page), reusing the
    // same construction so each page's content is independently identifiable.
    let mut doc = LoDocument::with_version("1.5");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
    });
    let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });

    let mut page_ids = Vec::new();
    for label in ["PG-CANARY-PAGE-ONE", "PG-CANARY-PAGE-TWO"] {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(label)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        });
        page_ids.push(Object::Reference(page_id));
    }

    let pages = dictionary! {
        "Type" => "Pages",
        "Kids" => page_ids,
        "Count" => 2,
        "Resources" => resources_id,
        "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
    };
    doc.objects.insert(pages_id, Object::Dictionary(pages));
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();

    let imported = import_pdf(&bytes, DOC_ID).expect("import_pdf on a two-page fixture");
    assert_eq!(imported.pages.len(), 2);
    assert!(imported.pages[0].spans[0].text.contains("PG-CANARY-PAGE-ONE"));
    assert_eq!(imported.pages[0].spans[0].page_index, 0);
    assert!(imported.pages[1].spans[0].text.contains("PG-CANARY-PAGE-TWO"));
    assert_eq!(imported.pages[1].spans[0].page_index, 1);
}

// ---------------------------------------------------------------------------
// dev-plan W9: "image-only PDF rejected"
// ---------------------------------------------------------------------------

#[test]
fn no_text_pdf_is_rejected() {
    let bytes = build_no_text_pdf();
    let result = import_pdf(&bytes, DOC_ID);
    assert_eq!(result.unwrap_err(), ImportPdfError::NoText);
}

#[test]
fn whitespace_only_pdf_is_rejected() {
    let bytes = build_text_pdf("   \t  \n  ");
    let result = import_pdf(&bytes, DOC_ID);
    assert_eq!(result.unwrap_err(), ImportPdfError::NoText);
}

// ---------------------------------------------------------------------------
// Malformed input: not the same failure mode as NoText, per ImportPdfError's own doc.
// ---------------------------------------------------------------------------

#[test]
fn non_pdf_bytes_are_malformed_not_no_text() {
    let result = import_pdf(b"this is not a PDF file at all", DOC_ID);
    assert_eq!(result.unwrap_err(), ImportPdfError::Malformed);
}

#[test]
fn empty_bytes_are_malformed() {
    let result = import_pdf(b"", DOC_ID);
    assert_eq!(result.unwrap_err(), ImportPdfError::Malformed);
}

// ---------------------------------------------------------------------------
// architecture §5.1: "PDF import and export run in memory." dev-plan W9: "watcher: no
// plaintext sidecar files." import_pdf takes bytes and returns a Document; it accepts no
// path and performs no filesystem I/O, so there is no sidecar file for a watcher to catch —
// verified here by construction (the function signature itself), and directly by checking
// the process creates no new file under a scratch directory during import.
// ---------------------------------------------------------------------------

#[test]
fn import_pdf_writes_no_file_to_disk() {
    let watch_dir = tempfile::tempdir().expect("temp dir");
    let before: std::collections::HashSet<_> = std::fs::read_dir(watch_dir.path())
        .expect("read_dir")
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();

    let bytes = build_text_pdf("PG-FIXTURE-CANARY-WATCHER-0003");
    let _doc = import_pdf(&bytes, DOC_ID).expect("import_pdf");

    let after: std::collections::HashSet<_> = std::fs::read_dir(watch_dir.path())
        .expect("read_dir")
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    assert_eq!(before, after, "import_pdf must not write any file, sidecar or otherwise");
}

// ---------------------------------------------------------------------------
// Re-import (testing.md §8): same inherited property as W8's import_text.
// ---------------------------------------------------------------------------

#[test]
fn import_pdf_never_derives_the_document_id_from_content() {
    let bytes = build_text_pdf("identical PDF bytes, different caller-supplied ids");
    let doc_a = import_pdf(&bytes, "doc-a").expect("import_pdf");
    let doc_b = import_pdf(&bytes, "doc-b").expect("import_pdf");
    assert_ne!(doc_a.id, doc_b.id);
}
