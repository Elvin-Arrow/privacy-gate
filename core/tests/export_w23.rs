//! W23 — from-scratch PDF re-render (architecture §11; NFR-S4).
//!
//! Spec sources:
//! - `docs/specs/architecture.md` §11 (new PDF from `redacted_content`; no source
//!   mutation; no incremental `/Prev`; redacted spans omitted, not overlayed)
//! - `docs/specs/testing.md` export sanitization (canary `R` absent in raw bytes +
//!   extracted text; keep canary present)
//! - `docs/dev-plan.md` W23
//!
//! Seam: [`pg_core::export::render_redacted_pdf`]. Share commands are W24.
//! Explicitly **not** in this chunk: save dialog; plaintext `.txt` export; preview
//! tokens; filename algorithm (api.md §7.1).

use pg_core::catalog::{RedactedDocument, RedactedPage};
use pg_core::export::render_redacted_pdf;
use pg_core::importer::{SourceFormat, TextSpan};

const REDACT: &str = "PG-CANARY-REDACT-7F3A";
const KEEP: &str = "PG-CANARY-KEEP-A91C";

fn remaining(text: &str) -> RedactedDocument {
    RedactedDocument {
        format: SourceFormat::Text,
        pages: vec![RedactedPage {
            page_index: 0,
            spans: vec![TextSpan {
                byte_offset: 0,
                byte_length: text.len() as u64,
                text: text.to_string(),
                page_index: 0,
            }],
        }],
    }
}

fn contains_utf16(haystack: &[u8], needle: &str) -> bool {
    let le: Vec<u8> = needle.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let be: Vec<u8> = needle.encode_utf16().flat_map(u16::to_be_bytes).collect();
    haystack.windows(le.len()).any(|w| w == le.as_slice())
        || haystack.windows(be.len()).any(|w| w == be.as_slice())
}

fn extracted(pdf: &[u8]) -> String {
    pdf_extract::extract_text_from_mem(pdf).expect("export PDF must be extractable")
}

#[test]
fn redacted_canary_is_absent_from_raw_bytes_and_extracted_text() {
    let body = format!("Dear Sir, {KEEP} both appear here.");
    assert!(!body.contains(REDACT), "fixture remaining text must already omit R");
    let pdf = render_redacted_pdf(&remaining(&body));
    assert!(
        !pdf.windows(REDACT.len()).any(|w| w == REDACT.as_bytes()),
        "redacted canary must not appear as UTF-8 in export bytes"
    );
    assert!(
        !contains_utf16(&pdf, REDACT),
        "redacted canary must not appear as UTF-16 in export bytes"
    );
    let text = extracted(&pdf);
    assert!(
        !text.contains(REDACT),
        "redacted canary must not appear in extracted text: {text:?}"
    );
}

#[test]
fn keep_canary_is_present_in_extracted_text() {
    let body = format!("Dear Sir, {KEEP} both appear here.");
    let pdf = render_redacted_pdf(&remaining(&body));
    let text = extracted(&pdf);
    assert!(
        text.contains(KEEP),
        "keep-visible canary must survive re-render: {text:?}"
    );
}

#[test]
fn export_pdf_is_not_an_incremental_update() {
    let pdf = render_redacted_pdf(&remaining(KEEP));
    assert!(pdf.starts_with(b"%PDF-"), "from-scratch writer must emit a PDF header");
    let as_str = String::from_utf8_lossy(&pdf);
    assert!(
        !as_str.contains("/Prev"),
        "incremental-update trailer /Prev would retain old content streams (architecture §11)"
    );
}

#[test]
fn info_dictionary_names_privacy_gate_and_omits_author() {
    let pdf = render_redacted_pdf(&remaining(KEEP));
    let as_str = String::from_utf8_lossy(&pdf);
    assert!(as_str.contains("Privacy Gate"));
    assert!(
        !as_str.contains("/Author"),
        "api.md §7.2: Author must be omitted"
    );
    assert!(!as_str.contains("/Subject"), "api.md §7.2: Subject omitted");
    assert!(!as_str.contains("/Keywords"), "api.md §7.2: Keywords omitted");
}

#[test]
fn empty_redacted_document_still_renders_a_pdf() {
    let doc = RedactedDocument {
        format: SourceFormat::Text,
        pages: Vec::new(),
    };
    let pdf = render_redacted_pdf(&doc);
    assert!(pdf.starts_with(b"%PDF-"));
    assert!(!String::from_utf8_lossy(&pdf).contains("/Prev"));
}

#[test]
fn two_pages_keep_their_own_remaining_text() {
    let doc = RedactedDocument {
        format: SourceFormat::Pdf,
        pages: vec![
            RedactedPage {
                page_index: 0,
                spans: vec![TextSpan {
                    byte_offset: 0,
                    byte_length: KEEP.len() as u64,
                    text: KEEP.to_string(),
                    page_index: 0,
                }],
            },
            RedactedPage {
                page_index: 1,
                spans: vec![TextSpan {
                    byte_offset: 20,
                    byte_length: 5,
                    text: "hello".to_string(),
                    page_index: 1,
                }],
            },
        ],
    };
    let text = extracted(&render_redacted_pdf(&doc));
    assert!(text.contains(KEEP), "{text:?}");
    assert!(text.contains("hello"), "{text:?}");
    assert!(!text.contains(REDACT), "{text:?}");
}
