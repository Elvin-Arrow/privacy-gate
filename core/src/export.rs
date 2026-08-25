//! From-scratch PDF re-render (architecture.md §11; W23).
//!
//! The Share Engine (W24) calls [`render_redacted_pdf`] with
//! `ApprovedVersion.redacted_content` (post-redaction spans already omitted by W17/W18).
//! This module **never** takes a source PDF: that is the structural guarantee against
//! incremental updates and overlay redaction (C-ARCH-6 / NFR-S4).
//!
//! Content streams are uncompressed so the testing.md §7.2 raw-byte scan can see keep
//! canaries and so a FlateDecode skip cannot hide a leak. Compression, if added later,
//! must keep the oracle self-test (W25) green.

use pdf_writer::{Content, Date, Name, Pdf, Rect, Ref, Str, TextStr};

use crate::catalog::{RedactedDocument, RedactedPage};

const FONT_NAME: Name = Name(b"F1");
const PAGE_WIDTH: f32 = 595.0;
const PAGE_HEIGHT: f32 = 842.0;
const MARGIN: f32 = 72.0;
const FONT_SIZE: f32 = 12.0;
const LINE_HEIGHT: f32 = 16.0;
const MAX_LINE_CHARS: usize = 90;

/// Info dictionary fields owned by the re-renderer (api.md §7.2). Title is the suggested
/// filename without `.pdf`; dates are the export timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfExportInfo {
    pub title: String,
    pub created_unix_ms: u64,
}

/// Render `doc` to a newly generated PDF (architecture §11).
///
/// Only remaining span text is written. Producer/Creator are `Privacy Gate`; Author,
/// Subject, and Keywords are omitted (api.md §7.2).
#[must_use]
pub fn render_redacted_pdf(doc: &RedactedDocument) -> Vec<u8> {
    render_redacted_pages(&doc.pages, None)
}

/// Render ordered pages (one document or a multi-doc bundle) to a new PDF.
#[must_use]
pub fn render_redacted_pages(pages: &[RedactedPage], info: Option<&PdfExportInfo>) -> Vec<u8> {
    let page_texts: Vec<String> = if pages.is_empty() {
        vec![String::new()]
    } else {
        pages.iter().map(page_plain_text).collect()
    };

    let mut alloc = Ref::new(1);
    let catalog_id = alloc.bump();
    let page_tree_id = alloc.bump();
    let font_id = alloc.bump();
    let info_id = alloc.bump();
    let page_ids: Vec<Ref> = page_texts.iter().map(|_| alloc.bump()).collect();
    let content_ids: Vec<Ref> = page_texts.iter().map(|_| alloc.bump()).collect();

    let mut pdf = Pdf::new();
    pdf.catalog(catalog_id).pages(page_tree_id);
    pdf.pages(page_tree_id)
        .kids(page_ids.iter().copied())
        .count(page_ids.len() as i32);
    {
        let mut di = pdf.document_info(info_id);
        di.producer(TextStr("Privacy Gate"))
            .creator(TextStr("Privacy Gate"));
        if let Some(info) = info {
            di.title(TextStr(&info.title));
            let date = pdf_date(info.created_unix_ms);
            di.creation_date(date).modified_date(date);
        }
    }
    pdf.type1_font(font_id).base_font(Name(b"Helvetica"));

    for (i, text) in page_texts.iter().enumerate() {
        {
            let mut page = pdf.page(page_ids[i]);
            page.media_box(Rect::new(0.0, 0.0, PAGE_WIDTH, PAGE_HEIGHT));
            page.parent(page_tree_id);
            page.contents(content_ids[i]);
            page.resources().fonts().pair(FONT_NAME, font_id);
        }
        let content = page_content(text);
        pdf.stream(content_ids[i], &content);
    }

    pdf.finish()
}

fn page_plain_text(page: &RedactedPage) -> String {
    page.spans.iter().map(|s| s.text.as_str()).collect()
}

fn page_content(text: &str) -> Vec<u8> {
    let mut content = Content::new();
    content.begin_text();
    content.set_font(FONT_NAME, FONT_SIZE);
    content.next_line(MARGIN, PAGE_HEIGHT - MARGIN);
    let mut first = true;
    for line in wrap_lines(text) {
        if !first {
            content.next_line(0.0, -LINE_HEIGHT);
        }
        first = false;
        let bytes = winansi_bytes(&line);
        content.show(Str(&bytes));
    }
    content.end_text();
    content.finish().to_vec()
}

/// Helvetica's built-in encoding is Latin-1-shaped; non-Latin-1 codepoints become `?`
/// so we never emit a UTF-8 sequence that could smuggle a canary in a second encoding.
fn winansi_bytes(text: &str) -> Vec<u8> {
    text.chars()
        .map(|c| if (c as u32) <= 0xff { c as u8 } else { b'?' })
        .collect()
}

fn wrap_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split(' ') {
            if current.is_empty() {
                current.push_str(word);
                continue;
            }
            if current.chars().count() + 1 + word.chars().count() > MAX_LINE_CHARS {
                lines.push(current);
                current = word.to_string();
            } else {
                current.push(' ');
                current.push_str(word);
            }
        }
        lines.push(current);
    }
    lines
}

fn pdf_date(unix_ms: u64) -> Date {
    let rfc = crate::account::format_rfc3339((unix_ms / 1000) as i64);
    let year: u16 = rfc.get(0..4).and_then(|s| s.parse().ok()).unwrap_or(1970);
    let month: u8 = rfc.get(5..7).and_then(|s| s.parse().ok()).unwrap_or(1);
    let day: u8 = rfc.get(8..10).and_then(|s| s.parse().ok()).unwrap_or(1);
    let hour: u8 = rfc.get(11..13).and_then(|s| s.parse().ok()).unwrap_or(0);
    let minute: u8 = rfc.get(14..16).and_then(|s| s.parse().ok()).unwrap_or(0);
    let second: u8 = rfc.get(17..19).and_then(|s| s.parse().ok()).unwrap_or(0);
    Date::new(year)
        .month(month)
        .day(day)
        .hour(hour)
        .minute(minute)
        .second(second)
        .utc_offset_hour(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_lines_breaks_only_when_the_next_word_would_exceed_max() {
        let left = "a".repeat(88);
        // 88 + space + 1 = 90, not > MAX_LINE_CHARS: stays one line (`>` vs `>=`).
        assert_eq!(wrap_lines(&format!("{left} b")), vec![format!("{left} b")]);
        // 88 + space + 2 = 91 > 90: wraps. `+`→`-`/`*` on the length sum would not.
        assert_eq!(wrap_lines(&format!("{left} bb")), vec![left, "bb".into()]);
    }

    #[test]
    fn page_content_starts_below_the_top_margin_and_leads_later_lines_down() {
        let one = String::from_utf8_lossy(&page_content("hello")).into_owned();
        let two = String::from_utf8_lossy(&page_content("hello\nworld")).into_owned();
        let start_y = PAGE_HEIGHT - MARGIN;
        assert!(
            one.contains(&start_y.to_string()),
            "Td y must be PAGE_HEIGHT - MARGIN ({start_y}), got {one:?}"
        );
        assert!(
            !one.contains(&format!("-{}", LINE_HEIGHT)),
            "a single line must not also lead by -LINE_HEIGHT (deleted `!first`): {one:?}"
        );
        assert!(
            two.contains(&format!("-{}", LINE_HEIGHT)),
            "subsequent lines lead down by -LINE_HEIGHT, got {two:?}"
        );
    }
}
