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

use crate::catalog::{
    DetectedField, FieldDecision, FieldDecisionKind, FieldId, RedactedDocument, RedactedPage,
};
use crate::importer::{Document, Page, SourceFormat, TextSpan};

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
    // `byte_length > 0` vs `>= 0` is tautological on u64 and is skipped on
    // [`span_nonempty`]; the half-open `offset < span_end` already excludes zeros.
    span_nonempty(field)
        && offset >= field.span.byte_offset
        && offset < span_end(field)
}

/// `u64 >= 0` is tautological, so `>`→`>=` here is equivalent (testing.md §5.4).
#[mutants::skip]
fn span_nonempty(field: &DetectedField) -> bool {
    field.span.byte_length > 0
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
    redact_pages(doc.source_format, &doc.pages, fields, decisions)
}

/// The page-content core of [`redact_document`], taking pages directly rather than a full
/// [`Document`] — the W26 override path (see [`redact_with_overrides`]) has no
/// `Document::raw_bytes` to reconstruct, only page spans.
#[must_use]
pub fn redact_pages(
    format: SourceFormat,
    pages: &[Page],
    fields: &[DetectedField],
    decisions: &HashMap<FieldId, FieldDecisionKind>,
) -> RedactedDocument {
    let doc_len = content_len(pages);
    let ranges = redacted_ranges(doc_len, fields, decisions);
    let out_pages = pages
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
        format,
        pages: out_pages,
    }
}

/// W26 (FR-5.4 / FR-6.2): re-render `redacted_content` for a share with a different
/// (ephemeral) decision set than the canonical `ApprovedVersion` — without mutating the
/// canonical version and without needing the discarded original `Document`.
///
/// This is possible because every `FieldDecision` snapshot in `ApprovedVersion.decisions`
/// keeps that field's own span *text*, redacted or not (that is what "reveal more" at
/// share time needs, since the retained original may not exist — design §3.2/§3.4). The
/// canonical `redacted_content` already carries the correctly-kept bytes for offsets
/// nothing canonically redacted; this reconstructs only the byte ranges the canonical
/// decisions cut, by slicing them back out of the covering field's stored text, then
/// re-applies [`redact_pages`] with `effective_decisions` — the *same* precedence rule
/// `submit_approval` used, not a second policy.
#[must_use]
pub fn redact_with_overrides(
    canonical: &[FieldDecision],
    redacted_content: &RedactedDocument,
    effective_decisions: &HashMap<FieldId, FieldDecisionKind>,
) -> RedactedDocument {
    let fields: Vec<DetectedField> = canonical.iter().map(|d| d.field.clone()).collect();
    let canonical_map: HashMap<FieldId, FieldDecisionKind> = canonical
        .iter()
        .map(|d| (d.field.id.clone(), d.decision))
        .collect();

    let fields_len = fields
        .iter()
        .map(|f| f.span.byte_offset.saturating_add(f.span.byte_length))
        .max()
        .unwrap_or(0);
    let kept_len = redacted_content
        .pages
        .iter()
        .flat_map(|p| &p.spans)
        .map(|s| s.byte_offset.saturating_add(s.byte_length))
        .max()
        .unwrap_or(0);
    let doc_len = fields_len.max(kept_len);

    let cut_ranges = redacted_ranges(doc_len, &fields, &canonical_map);
    let mut recovered: HashMap<u32, Vec<TextSpan>> = HashMap::new();
    for (rs, re) in cut_ranges {
        // Every cut range is, by construction of `redacted_ranges`, fully covered by at
        // least one decided field — pick the smallest (most specific) covering field so a
        // nested reveal recovers exactly that field's own text.
        let covering = fields
            .iter()
            .filter(|f| {
                f.span.byte_offset <= rs
                    && re <= f.span.byte_offset.saturating_add(f.span.byte_length)
            })
            .min_by_key(|f| f.span.byte_length);
        let Some(field) = covering else {
            // Should not happen: `redacted_ranges` only cuts offsets a decided field
            // covers. Skip rather than fabricate text for an unrecoverable gap.
            continue;
        };
        let rel_from = (rs - field.span.byte_offset) as usize;
        let rel_to = (re - field.span.byte_offset) as usize;
        let Some(text) = field.span.text.get(rel_from..rel_to) else {
            continue;
        };
        recovered.entry(field.span.page_index).or_default().push(TextSpan {
            byte_offset: rs,
            byte_length: re - rs,
            text: text.to_string(),
            page_index: field.span.page_index,
        });
    }

    let pages: Vec<Page> = redacted_content
        .pages
        .iter()
        .map(|rp| {
            let mut spans = rp.spans.clone();
            if let Some(extra) = recovered.get(&rp.page_index) {
                spans.extend(extra.iter().cloned());
            }
            spans.sort_by_key(|s| s.byte_offset);
            Page { spans }
        })
        .collect();

    redact_pages(redacted_content.format, &pages, &fields, effective_decisions)
}

fn content_len(pages: &[Page]) -> u64 {
    pages
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

    #[test]
    fn strictly_contained_is_a_proper_subset_including_shared_edges() {
        let outer = field("o", 0, 10, None);
        let inner = field("i", 3, 3, None);
        let prefix = field("p", 0, 3, None); // shares start
        let suffix = field("s", 7, 3, None); // shares end
        let zero = field("z", 3, 0, None);
        assert!(strictly_contained(&inner, &outer));
        assert!(strictly_contained(&prefix, &outer));
        assert!(strictly_contained(&suffix, &outer));
        assert!(!strictly_contained(&zero, &outer));
        assert!(!strictly_contained(&outer, &inner));
    }

    fn span(text: &str, offset: u64) -> TextSpan {
        TextSpan {
            byte_offset: offset,
            byte_length: text.len() as u64,
            text: text.to_string(),
            page_index: 0,
        }
    }

    #[test]
    fn redact_pages_omits_a_middle_range_and_keeps_prefix_and_suffix() {
        let pages = [Page {
            spans: vec![span("0123456789", 0)],
        }];
        let f = DetectedField {
            id: "r".into(),
            label: "r".into(),
            classification: "test".into(),
            span: span("3456", 3),
            parent_field_id: None,
        };
        let decisions = HashMap::from([("r".into(), FieldDecisionKind::Redact)]);
        let out = redact_pages(SourceFormat::Text, &pages, &[f], &decisions);
        let text: String = out
            .pages
            .iter()
            .flat_map(|p| &p.spans)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(text, "012789");
        assert!(
            out.pages[0].spans.iter().all(|s| s.byte_length > 0),
            "empty leftover spans would hide a cursor==end / cursor==clip_s mutant"
        );
    }

    #[test]
    fn redact_pages_keeps_a_span_that_does_not_overlap_the_cut() {
        let pages = [Page {
            spans: vec![span("ABCD", 0), span("WXYZ", 6)],
        }];
        let f = DetectedField {
            id: "r".into(),
            label: "r".into(),
            classification: "test".into(),
            span: span("WXYZ", 6),
            parent_field_id: None,
        };
        let decisions = HashMap::from([("r".into(), FieldDecisionKind::Redact)]);
        let out = redact_pages(SourceFormat::Text, &pages, &[f], &decisions);
        let text: String = out
            .pages
            .iter()
            .flat_map(|p| &p.spans)
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(text, "ABCD");
    }
}
