//! W17 — Overlap / nested fields (design §3.5).
//!
//! Spec sources:
//! - `docs/specs/design.md` §3.5 (innermost explicit decision wins; partial overlap
//!   redact-wins; one byte-offset rule at export)
//! - `docs/specs/testing.md` §8 overlap row; §5.3 gated module
//! - `docs/dev-plan.md` W17 ("Tests first: nested keep-inside-redact; partial overlap;
//!   property tests.")
//!
//! Seam: [`pg_core::overlap::offset_is_redacted`] / [`pg_core::overlap::redacted_ranges`].
//! `submit_approval` (W18) is the first command caller; this chunk does not add commands.

use std::collections::HashMap;

use pg_core::catalog::{DetectedField, FieldId};
use pg_core::importer::TextSpan;
use pg_core::overlap::{offset_is_redacted, redacted_ranges};
use pg_core::session::FieldDecisionKind;
use proptest::prelude::*;

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

fn dec(pairs: &[(&str, FieldDecisionKind)]) -> HashMap<FieldId, FieldDecisionKind> {
    pairs
        .iter()
        .map(|(id, d)| ((*id).to_string(), *d))
        .collect()
}

fn redacted_bytes(doc_len: u64, fields: &[DetectedField], decisions: &HashMap<FieldId, FieldDecisionKind>) -> Vec<u64> {
    (0..doc_len)
        .filter(|o| offset_is_redacted(*o, fields, decisions))
        .collect()
}

// ---------------------------------------------------------------------------
// design §3.5 table
// ---------------------------------------------------------------------------

#[test]
fn uncovered_bytes_are_not_redacted() {
    let fields = [field("a", 0, 3, None)];
    let decisions = dec(&[("a", FieldDecisionKind::Redact)]);
    assert!(!offset_is_redacted(3, &fields, &decisions));
    assert!(!offset_is_redacted(9, &fields, &decisions));
    assert_eq!(redacted_ranges(10, &fields, &decisions), vec![(0, 3)]);
}

#[test]
fn single_keep_redacts_nothing() {
    let fields = [field("a", 0, 5, None)];
    let decisions = dec(&[("a", FieldDecisionKind::KeepVisible)]);
    assert!(redacted_ranges(8, &fields, &decisions).is_empty());
}

#[test]
fn nested_keep_inside_redact_leaves_the_inner_span_visible() {
    // design §3.5: "A redact on an outer field does not cascade to an inner field the
    // user kept."
    let fields = [
        field("outer", 0, 10, None),
        field("inner", 3, 3, Some("outer")),
    ];
    let decisions = dec(&[
        ("outer", FieldDecisionKind::Redact),
        ("inner", FieldDecisionKind::KeepVisible),
    ]);
    assert_eq!(redacted_bytes(10, &fields, &decisions), vec![0, 1, 2, 6, 7, 8, 9]);
    assert_eq!(redacted_ranges(10, &fields, &decisions), vec![(0, 3), (6, 10)]);
}

#[test]
fn nested_keep_inside_redact_wins_by_geometry_without_parent_id() {
    let fields = [field("outer", 0, 10, None), field("inner", 3, 3, None)];
    let decisions = dec(&[
        ("outer", FieldDecisionKind::Redact),
        ("inner", FieldDecisionKind::KeepVisible),
    ]);
    assert_eq!(redacted_ranges(10, &fields, &decisions), vec![(0, 3), (6, 10)]);
}

#[test]
fn nested_redact_inside_keep_hides_only_the_inner_span() {
    // design §3.5: "a keep on an outer field does not force an inner field the user
    // redacted to be revealed."
    let fields = [
        field("outer", 0, 10, None),
        field("inner", 3, 3, Some("outer")),
    ];
    let decisions = dec(&[
        ("outer", FieldDecisionKind::KeepVisible),
        ("inner", FieldDecisionKind::Redact),
    ]);
    assert_eq!(redacted_ranges(10, &fields, &decisions), vec![(3, 6)]);
}

#[test]
fn partial_overlap_redact_wins_on_the_intersection() {
    // design §3.5: non-nested intersect → Redact wins on the intersection.
    let fields = [field("a", 0, 8, None), field("b", 4, 8, None)];
    let decisions = dec(&[
        ("a", FieldDecisionKind::Redact),
        ("b", FieldDecisionKind::KeepVisible),
    ]);
    // a covers [0,8), b covers [4,12). Intersection [4,8) is redacted; [0,4) redact; [8,12) keep.
    assert_eq!(redacted_ranges(12, &fields, &decisions), vec![(0, 8)]);
}

#[test]
fn keep_nested_in_partial_overlap_intersection_is_visible() {
    // design §3.5: "unless a third field strictly nested inside the intersection is
    // decided Keep."
    let fields = [
        field("a", 0, 10, None),
        field("b", 5, 10, None),
        field("c", 6, 2, Some("a")),
    ];
    let decisions = dec(&[
        ("a", FieldDecisionKind::Redact),
        ("b", FieldDecisionKind::KeepVisible),
        ("c", FieldDecisionKind::KeepVisible),
    ]);
    // [0,5) only a → redact; [5,6) a∩b no nest → redact; [6,8) nested keep → visible;
    // [8,10) a∩b → redact; [10,15) only b → visible.
    assert_eq!(
        redacted_ranges(15, &fields, &decisions),
        vec![(0, 6), (8, 10)]
    );
}

#[test]
fn adjacent_non_overlapping_spans_do_not_interact() {
    let fields = [field("a", 0, 5, None), field("b", 5, 5, None)];
    let decisions = dec(&[
        ("a", FieldDecisionKind::Redact),
        ("b", FieldDecisionKind::KeepVisible),
    ]);
    assert_eq!(redacted_ranges(10, &fields, &decisions), vec![(0, 5)]);
}

#[test]
fn equal_spans_without_nesting_are_redact_wins() {
    let fields = [field("a", 0, 4, None), field("b", 0, 4, None)];
    let decisions = dec(&[
        ("a", FieldDecisionKind::Redact),
        ("b", FieldDecisionKind::KeepVisible),
    ]);
    assert_eq!(redacted_ranges(4, &fields, &decisions), vec![(0, 4)]);
}

#[test]
fn parent_id_makes_equal_spans_innermost_keep_win() {
    // Same bytes, but `b` is recorded as nested in `a` (design §3.1 parent_field_id).
    let fields = [field("a", 0, 4, None), field("b", 0, 4, Some("a"))];
    let decisions = dec(&[
        ("a", FieldDecisionKind::Redact),
        ("b", FieldDecisionKind::KeepVisible),
    ]);
    assert!(redacted_ranges(4, &fields, &decisions).is_empty());
}

#[test]
fn undecided_fields_do_not_cover() {
    let fields = [field("a", 0, 5, None), field("b", 0, 5, None)];
    let decisions = dec(&[("a", FieldDecisionKind::KeepVisible)]);
    assert!(redacted_ranges(5, &fields, &decisions).is_empty());
}

#[test]
fn zero_length_field_covers_nothing() {
    let fields = [field("a", 3, 0, None)];
    let decisions = dec(&[("a", FieldDecisionKind::Redact)]);
    assert!(redacted_ranges(8, &fields, &decisions).is_empty());
}

#[test]
fn empty_document_has_no_ranges() {
    let fields = [field("a", 0, 3, None)];
    let decisions = dec(&[("a", FieldDecisionKind::Redact)]);
    assert!(redacted_ranges(0, &fields, &decisions).is_empty());
}

// ---------------------------------------------------------------------------
// testing.md: "one rule at export" — ranges match per-byte
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn redacted_ranges_match_per_byte_scan(
        spans in prop::collection::vec((0u64..16, 1u64..8), 0..5),
        keep_mask in 0u32..32,
    ) {
        let fields: Vec<DetectedField> = spans
            .iter()
            .enumerate()
            .map(|(i, (start, len))| field(&format!("f{i}"), *start, *len, None))
            .collect();
        let mut decisions = HashMap::new();
        for (i, f) in fields.iter().enumerate() {
            let kind = if (keep_mask & (1 << (i % 31))) != 0 {
                FieldDecisionKind::KeepVisible
            } else {
                FieldDecisionKind::Redact
            };
            decisions.insert(f.id.clone(), kind);
        }
        let doc_len = 24u64;
        let ranges = redacted_ranges(doc_len, &fields, &decisions);
        for offset in 0..doc_len {
            let in_range = ranges.iter().any(|&(s, e)| offset >= s && offset < e);
            prop_assert_eq!(
                in_range,
                offset_is_redacted(offset, &fields, &decisions),
                "offset {}",
                offset
            );
        }
        for w in ranges.windows(2) {
            prop_assert!(w[0].1 < w[1].0, "ranges must be merged and ordered: {:?}", ranges);
        }
    }

    #[test]
    fn nested_keep_inside_redact_never_redacts_the_inner_bytes(
        outer_len in 6u64..16,
        inner_start in 1u64..4,
        inner_len in 1u64..3,
    ) {
        prop_assume!(inner_start + inner_len < outer_len);
        let fields = [
            field("outer", 0, outer_len, None),
            field("inner", inner_start, inner_len, Some("outer")),
        ];
        let decisions = dec(&[
            ("outer", FieldDecisionKind::Redact),
            ("inner", FieldDecisionKind::KeepVisible),
        ]);
        for offset in inner_start..inner_start + inner_len {
            prop_assert!(
                !offset_is_redacted(offset, &fields, &decisions),
                "inner offset {} must stay visible",
                offset
            );
        }
    }

    #[test]
    fn partial_overlap_intersection_is_redacted_without_a_nested_keep(
        a_len in 4u64..10,
        b_start in 2u64..6,
        b_len in 4u64..10,
    ) {
        let a_end = a_len;
        let b_end = b_start + b_len;
        prop_assume!(b_start < a_end && b_end > a_end); // partial, b sticks out past a
        prop_assume!(b_start > 0); // a is not nested in b
        let fields = [field("a", 0, a_len, None), field("b", b_start, b_len, None)];
        let decisions = dec(&[
            ("a", FieldDecisionKind::Redact),
            ("b", FieldDecisionKind::KeepVisible),
        ]);
        for offset in b_start..a_end {
            prop_assert!(
                offset_is_redacted(offset, &fields, &decisions),
                "intersection offset {} must be redacted",
                offset
            );
        }
    }
}
