//! The Importer — plain-text and PDF extraction (`design.md` §2.1, §3.1; `data-model.md`
//! §5.1; FR-1.1, FR-1.2).
//!
//! W8 delivered plain text ("extract UTF-8 text to in-memory pages/spans. No catalog
//! yet."); W9 adds PDF ("PDF with extractable text → pages + byte offsets. No text →
//! `unsupported_document`. In-memory PDF I/O only."). This module is a **library only** —
//! there is no Tauri command, no `SessionManager` method, and it never touches the vault or
//! the disk. Both `import_text` and `import_pdf` are pure functions: bytes in, a
//! [`Document`] or an error out.
//!
//! # Scope fence (dev-plan.md W9 "Do not: re-render export (W23); OCR")
//!
//! - **No OCR.** A PDF with no extractable text (scanned pages, images) is refused
//!   (`ImportPdfError::NoText`), never run through an OCR pipeline — SRS explicitly scopes
//!   OCR out of v1 (design §2.1 "Non-goal: OCR").
//! - **No export re-rendering.** `pdf-extract`'s job here is read-only text extraction;
//!   nothing in this module writes a PDF. The from-scratch PDF writer for export
//!   (architecture §11, `pdf-writer` or equivalent) is a separate, later dependency (W23).
//! - **No detection.** Same as W8: `Document` carries no `DetectedField`s yet (W12).
//! - **No retention gate, no filename validation, no `Document.id` generation.** Same
//!   command-layer/catalog deferrals as W8 (see below) — `import_pdf` takes the same
//!   `(bytes, doc_id)` shape as `import_text`.
//! - **No re-render / over-budget flag.** dev-plan W9 "Done when: component tests
//!   green. Over-budget flag is W10." — `import_pdf` has no size/budget awareness; that's
//!   the catalog command's `over_budget` field (api.md), computed once there's an actual
//!   import command to attach it to.
//!
//! Inherited from W8 unchanged: no retention gate, no filename validation (both
//! command-layer, api.md `import_document`, W10/W11 — neither `import_text` nor
//! `import_pdf` takes a filename or a retention policy), and no `Document.id` generation
//! (the catalog's concern, W10 — testing.md §8 "Re-import": two imports of identical bytes
//! must produce two different `doc_id`s, which only holds if id assignment never happens in
//! this module).

use serde::{Deserialize, Serialize};

/// data-model §5.1: `"text" | "pdf"`. Only [`SourceFormat::Text`] is ever produced by this
/// module (see module scope fence); [`SourceFormat::Pdf`] is here because the type is
/// shared with W9, not because this chunk emits it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Text,
    Pdf,
}

/// data-model §5.1 `TextSpan`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSpan {
    /// Byte offset into `raw_bytes` (or, for a multi-page source, into that page's bytes).
    pub byte_offset: u64,
    /// Octet length of the span — **not** a character count, so this always matches the
    /// slice of `raw_bytes` the span covers even when the text contains multi-byte UTF-8
    /// characters.
    pub byte_length: u64,
    pub text: String,
    pub page_index: u32,
}

/// data-model §5.1 `Page`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Page {
    pub spans: Vec<TextSpan>,
}

/// data-model §5.1 `Document` — the Importer → Detector → Approval Engine intermediate
/// representation. `raw_bytes` lives here only in process memory (design §2.1: "held only
/// in process memory during import... overwritten by Importer on Vault ack"); this module
/// has no Vault to acknowledge anything to yet (W10), so nothing here ever overwrites it —
/// that hand-off is the catalog command's responsibility once it exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub id: String,
    pub source_format: SourceFormat,
    pub pages: Vec<Page>,
    pub raw_bytes: Vec<u8>,
}

/// Why `import_text` refused the input. Coarse and non-secret — never the bytes
/// themselves, never a decode-position-specific message that could leak content shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportTextError {
    /// FR-1.2: "reject inputs it cannot extract text from... with a clear message." Zero
    /// bytes is the unambiguous case for plain text; W10's command layer is what actually
    /// returns `unsupported_document` to the caller (dev-plan W8 "Tests first": "empty →
    /// `unsupported_document` at command layer (W10)") — this module only signals "there
    /// is nothing here," not the API error code.
    Empty,
    /// FR-1.1 scopes text import to UTF-8. Bytes that don't decode are refused rather than
    /// lossily reinterpreted (which would silently fabricate or drop content — exactly what
    /// FR-1.2 forbids: "never silently process them as if redactable").
    NotUtf8,
}

impl core::fmt::Display for ImportTextError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ImportTextError::Empty => f.write_str("no extractable text"),
            ImportTextError::NotUtf8 => f.write_str("not valid UTF-8"),
        }
    }
}

impl std::error::Error for ImportTextError {}

/// Extract `bytes` as a born-digital plain-text document (FR-1.1).
///
/// The whole document becomes one [`Page`] (plain text has no inherent page concept) with
/// one [`TextSpan`] covering the entire decoded string — `byte_offset: 0`,
/// `byte_length: bytes.len()`. Splitting into finer-grained spans (lines, paragraphs) is a
/// Detector-time concern once one exists (W12+), not an Importer one: nothing downstream of
/// this module needs sub-document granularity until detection produces `DetectedField`s
/// that reference specific ranges.
///
/// `doc_id` is supplied by the caller (see module scope fence — this library never mints
/// one).
///
/// # Errors
/// [`ImportTextError::Empty`] for zero-length input; [`ImportTextError::NotUtf8`] if
/// `bytes` is not valid UTF-8.
pub fn import_text(bytes: &[u8], doc_id: &str) -> Result<Document, ImportTextError> {
    if bytes.is_empty() {
        return Err(ImportTextError::Empty);
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ImportTextError::NotUtf8)?
        .to_string();

    let span = TextSpan {
        byte_offset: 0,
        byte_length: bytes.len() as u64,
        text,
        page_index: 0,
    };

    Ok(Document {
        id: doc_id.to_string(),
        source_format: SourceFormat::Text,
        pages: vec![Page { spans: vec![span] }],
        raw_bytes: bytes.to_vec(),
    })
}

// ---------------------------------------------------------------------------
// PDF (W9)
// ---------------------------------------------------------------------------

/// Why `import_pdf` refused the input. Same non-secret discipline as
/// [`ImportTextError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportPdfError {
    /// FR-1.2: "e.g., scanned PDFs, images" — every page's extracted text is empty (after
    /// trimming whitespace). The library-level signal; `unsupported_document` is W10's
    /// mapping, same split as `ImportTextError::Empty`.
    NoText,
    /// The bytes are not a PDF `pdf-extract`/`lopdf` can parse at all (corrupt file,
    /// unsupported encryption, wrong magic bytes). Distinct from `NoText` — this is "could
    /// not read the document," not "read it and there was nothing in it."
    Malformed,
}

impl core::fmt::Display for ImportPdfError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ImportPdfError::NoText => f.write_str("no extractable text"),
            ImportPdfError::Malformed => f.write_str("not a readable PDF"),
        }
    }
}

impl std::error::Error for ImportPdfError {}

/// Extract `bytes` as a born-digital PDF (FR-1.1) — "PDF with extractable text → pages +
/// byte offsets" (dev-plan W9).
///
/// One [`Page`] per PDF page, each with one [`TextSpan`] covering that page's whole
/// extracted text (`byte_offset: 0`, `byte_length` = the octet length of the extracted
/// text) — the same "no finer granularity than the Detector needs" choice `import_text`
/// makes, applied per page instead of to the whole document, since a PDF's own page
/// boundaries are real structure `data-model.md` §5.1's `Page` is meant to carry. The
/// offset is into **the extracted text of that page**, not into the raw PDF file bytes —
/// PDF content streams are frequently compressed/encoded, so a raw-file-byte offset
/// couldn't locate anything meaningful; api.md's `DetectedFieldDto.span` documents offsets
/// as being "in the page/source," which for a PDF source is its extracted text.
///
/// Extraction runs entirely in memory (`pdf_extract::extract_text_from_mem_by_pages`) —
/// architecture §5.1: "PDF import and export run in memory. If a library cannot be
/// configured for memory-only I/O it shall not be used."
///
/// # Errors
/// [`ImportPdfError::Malformed`] if the bytes don't parse as a PDF at all;
/// [`ImportPdfError::NoText`] if every page has no visible-glyph text once whitespace and
/// control characters are discounted (FR-1.2's scanned-PDF/image case).
pub fn import_pdf(bytes: &[u8], doc_id: &str) -> Result<Document, ImportPdfError> {
    let page_texts =
        pdf_extract::extract_text_from_mem_by_pages(bytes).map_err(|_| ImportPdfError::Malformed)?;

    // Not just `.trim().is_empty()`: `pdf-extract` can emit stray control/null characters
    // as text-positioning artifacts even for a content stream with no real glyphs (a
    // whitespace-only `Tj` string is enough to trigger it) — `char::is_whitespace` alone
    // doesn't cover those, so a page like that would slip past a plain trim-and-check as
    // "has text" when it plainly does not. "No visible glyph" is the actual FR-1.2
    // property (a scanned page has none); whitespace and control characters are never
    // visible glyphs.
    if page_texts
        .iter()
        .all(|t| t.chars().all(|c| c.is_whitespace() || c.is_control()))
    {
        return Err(ImportPdfError::NoText);
    }

    let pages = page_texts
        .into_iter()
        .enumerate()
        .map(|(index, text)| {
            let span = TextSpan {
                byte_offset: 0,
                byte_length: text.len() as u64,
                text,
                page_index: index as u32,
            };
            Page { spans: vec![span] }
        })
        .collect();

    Ok(Document {
        id: doc_id.to_string(),
        source_format: SourceFormat::Pdf,
        pages,
        raw_bytes: bytes.to_vec(),
    })
}
