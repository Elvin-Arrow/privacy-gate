//! Overlap / nested-field redaction (design.md §3.5; W17).
//!
//! A byte offset is redacted **iff** a decided field covering it is `Redact` **and** no
//! more-specific covering field is `KeepVisible`. Innermost explicit decisions win;
//! partial (non-nested) overlaps are redact-wins. This is the single rule share/export
//! (W18+) applies — not a second policy.
//!
//! Nesting is geometric strict containment, plus `DetectedField.parent_field_id` when the
//! detector recorded it (design §3.5 / §3.1). Fields without a decision are ignored.

use std::collections::{HashMap, HashSet};

use crate::catalog::{DetectedField, FieldDecisionKind, FieldId, RedactedDocument, RedactedPage};
use crate::importer::{Document, TextSpan};

/// Half-open `[start, end)` ranges that must be omitted from export (design §3.5 last
/// bullet). Adjacent redacted bytes are merged.
#[must_use]
pub fn redacted_ranges(
    doc_len: u64,
    fields: &[DetectedField],
    decisions: &HashMap<FieldId, FieldDecisionKind>,
) -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut start: Option<u64> = None;
    for offset in 0..doc_len {
        if offset_is_redacted(offset, fields, decisions) {
            if start.is_none() {
                start = Some(offset);
            }
        } else if let Some(s) = start.take() {
            out.push((s, offset));
        }
    }
    if let Some(s) = start {
        out.push((s, doc_len));
    }
    out
}

/// design §3.5: redacted iff any covering field is Redact and no more-specific covering
/// field is Keep.
#[must_use]
pub fn offset_is_redacted(
    offset: u64,
    fields: &[DetectedField],
    decisions: &HashMap<FieldId, FieldDecisionKind>,
) -> bool {
    let by_id: HashMap<&str, &DetectedField> =
        fields.iter().map(|f| (f.id.as_str(), f)).collect();
    let covering: Vec<(&DetectedField, FieldDecisionKind)> = fields
        .iter()
        .filter_map(|f| {
            let decision = *decisions.get(&f.id)?;
            covers(offset, f).then_some((f, decision))
        })
        .collect();
    let redacts: Vec<&DetectedField> = covering
        .iter()
        .filter(|(_, d)| *d == FieldDecisionKind::Redact)
        .map(|(f, _)| *f)
        .collect();
    if redacts.is_empty() {
        return false;
    }
    let keeps: Vec<&DetectedField> = covering
        .iter()
        .filter(|(_, d)| *d == FieldDecisionKind::KeepVisible)
        .map(|(f, _)| *f)
        .collect();
    let nested_keep = keeps.iter().any(|k| {
        redacts
            .iter()
            .any(|r| more_specific(k, r, &by_id))
    });
    !nested_keep
}

fn span_end(field: &DetectedField) -> u64 {
    field.span.byte_offset.saturating_add(field.span.byte_length)
}

fn covers(offset: u64, field: &DetectedField) -> bool {
    field.span.byte_length > 0
        && offset >= field.span.byte_offset
        && offset < span_end(field)
}

/// Strict containment: `inner`'s range is a proper subset of `outer`'s.
fn strictly_contained(inner: &DetectedField, outer: &DetectedField) -> bool {
    let (is, ie) = (inner.span.byte_offset, span_end(inner));
    let (os, oe) = (outer.span.byte_offset, span_end(outer));
    inner.span.byte_length > 0
        && is >= os
        && ie <= oe
        && (is > os || ie < oe)
}

fn is_descendant(
    child: &DetectedField,
    ancestor: &DetectedField,
    by_id: &HashMap<&str, &DetectedField>,
) -> bool {
    let mut seen = HashSet::new();
    let mut cur = child.parent_field_id.as_deref();
    while let Some(pid) = cur {
        if !seen.insert(pid) {
            break;
        }
        if pid == ancestor.id {
            return true;
        }
        cur = by_id.get(pid).and_then(|f| f.parent_field_id.as_deref());
    }
    false
}

fn more_specific(
    inner: &DetectedField,
    outer: &DetectedField,
    by_id: &HashMap<&str, &DetectedField>,
) -> bool {
    strictly_contained(inner, outer) || is_descendant(inner, outer, by_id)
}

/// Apply [`redacted_ranges`] to each page span: redacted bytes are omitted, not overlayed
/// (data-model §6.3). Coordinate space is page-span offsets (extracted text), not PDF
/// `raw_bytes` length.
#[must_use]
pub fn redact_document(
    doc: &Document,
    fields: &[DetectedField],
    decisions: &HashMap<FieldId, FieldDecisionKind>,
) -> RedactedDocument {
    let doc_len = content_len(doc);
    let ranges = redacted_ranges(doc_len, fields, decisions);
    let pages = doc
        .pages
        .iter()
        .enumerate()
        .map(|(i, page)| {
            let page_index = page
                .spans
                .first()
                .map(|s| s.page_index)
                .unwrap_or(i as u32);
            let mut spans = Vec::new();
            for span in &page.spans {
                spans.extend(visible_subspans(span, &ranges));
            }
            RedactedPage { page_index, spans }
        })
        .collect();
    RedactedDocument {
        format: doc.source_format,
        pages,
    }
}

fn content_len(doc: &Document) -> u64 {
    doc.pages
        .iter()
        .flat_map(|p| &p.spans)
        .map(|s| s.byte_offset.saturating_add(s.byte_length))
        .max()
        .unwrap_or(0)
}

fn visible_subspans(span: &TextSpan, redacted: &[(u64, u64)]) -> Vec<TextSpan> {
    let start = span.byte_offset;
    let end = start.saturating_add(span.byte_length);
    let mut cursor = start;
    let mut out = Vec::new();
    for &(rs, re) in redacted {
        let clip_s = rs.max(start);
        let clip_e = re.min(end);
        if clip_s >= clip_e {
            continue;
        }
        if cursor < clip_s {
            if let Some(piece) = slice_span(span, cursor, clip_s) {
                out.push(piece);
            }
        }
        cursor = cursor.max(clip_e);
    }
    if cursor < end {
        if let Some(piece) = slice_span(span, cursor, end) {
            out.push(piece);
        }
    }
    out
}

fn slice_span(span: &TextSpan, from: u64, to: u64) -> Option<TextSpan> {
    let rel_from = from.saturating_sub(span.byte_offset) as usize;
    let rel_to = to.saturating_sub(span.byte_offset) as usize;
    let text = span.text.get(rel_from..rel_to)?;
    Some(TextSpan {
        byte_offset: from,
        byte_length: to.saturating_sub(from),
        text: text.to_string(),
        page_index: span.page_index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(id: &str, start: u64, len: u64, parent: Option<&str>) -> DetectedField {
        DetectedField {
            id: id.to_string(),
            label: id.to_string(),
            classification: "test".to_string(),
            span: TextSpan {
                byte_offset: start,
                byte_length: len,
                text: String::new(),
                page_index: 0,
            },
            parent_field_id: parent.map(str::to_string),
        }
    }

    #[test]
    fn strictly_contained_rejects_equal_spans() {
        let a = field("a", 0, 4, None);
        let b = field("b", 0, 4, None);
        assert!(!strictly_contained(&a, &b));
    }

    #[test]
    fn covers_is_half_open() {
        let f = field("a", 2, 3, None);
        assert!(!covers(1, &f));
        assert!(covers(2, &f));
        assert!(covers(4, &f));
        assert!(!covers(5, &f));
    }
}
