//! W13 — Pattern pack `pg-patterns-uk-v1`.
//!
//! Spec sources:
//! - `docs/specs/architecture.md` §10.1 (`pg-patterns-uk-v1`: UK sort code, account
//!   number, National Insurance number, NHS number, email, phone, IBAN, payment-card/Luhn)
//! - `docs/specs/testing.md` §8 ("Pattern pack | UK sort code, account, NI, NHS, email,
//!   phone, IBAN, Luhn card on golden strings"); §7.2 (goldens must not be PDF/JSON
//!   keywords; example NI `QQ123456C`, sort code `20-40-60`)
//! - `docs/specs/srs.md` FR-2.1
//! - `docs/dev-plan.md` W13 ("Tests first: goldens hit; PDF/JSON keywords are not
//!   false-positive oracles (testing.md §7.2)."; "Import still works with stub in unit
//!   tests."; "Do not: require ONNX to pass PR.")
//!
//! Seam: [`PatternsUkV1::detect`] — the same [`Detector`] trait as the W12 stub. This file
//! does not go through `SessionManager`; the default import path stays `StubDetector` so
//! AC-1..AC-4 cannot be coupled to pattern-pack behaviour (testing.md §10).
//!
//! Expected values are independent literals from testing.md §7.2 (and published
//! test-range identifiers for the rest of architecture §10.1). They are not derived from
//! the production regexes.

use pg_core::detector::{Detector, PatternsUkV1, PATTERNS_UK_V1_ID};
use pg_core::importer;

const DOC_ID: &str = "00000000-0000-4000-8000-000000000013";

/// testing.md §7.2's NI example. Shape-only: HMRC would reject a Q-prefix, which is why
/// this is a safe synthetic canary rather than a real number.
const GOLDEN_NI: &str = "QQ123456C";
/// testing.md §7.2's sort-code example.
const GOLDEN_SORT_CODE: &str = "20-40-60";
/// 8-digit UK account shape; not a PDF token, not a JSON key.
const GOLDEN_ACCOUNT: &str = "31926819";
/// Published NHS test number (Mod-11 valid), not a person's number.
const GOLDEN_NHS: &str = "9434765919";
const GOLDEN_EMAIL: &str = "aisha@example.com";
/// Ofcom UK test range 07700 900xxx, unspaced.
const GOLDEN_PHONE: &str = "07700900123";
/// IBAN registry example (GB).
const GOLDEN_IBAN: &str = "GB82WEST12345698765432";
/// Well-known Visa test PAN (Luhn-valid), not a live card.
const GOLDEN_CARD: &str = "4111111111111111";

fn detect_text(text: &str) -> Vec<pg_core::catalog::DetectedField> {
    let doc = importer::import_text(text.as_bytes(), DOC_ID).expect("import_text");
    PatternsUkV1.detect(&doc)
}

fn field_texts_for<'a>(
    fields: &'a [pg_core::catalog::DetectedField],
    label: &str,
) -> Vec<&'a str> {
    fields
        .iter()
        .filter(|f| f.label == label)
        .map(|f| f.span.text.as_str())
        .collect()
}

fn assert_locatable(text: &str, field: &pg_core::catalog::DetectedField) {
    let start = field.span.byte_offset as usize;
    let end = start + field.span.byte_length as usize;
    assert_eq!(
        &text.as_bytes()[start..end],
        field.span.text.as_bytes(),
        "reported span must point at its own text inside the source"
    );
    assert_eq!(field.span.page_index, 0);
    assert!(field.parent_field_id.is_none());
}

// ---------------------------------------------------------------------------
// dev-plan W13: "goldens hit"
// testing.md §7.2: NI `QQ123456C`, sort code `20-40-60`
// ---------------------------------------------------------------------------

#[test]
fn patterns_uk_v1_id_is_the_architecture_pack_name() {
    assert_eq!(PATTERNS_UK_V1_ID, "pg-patterns-uk-v1");
}

#[test]
fn golden_ni_is_located_at_its_real_byte_offset() {
    let text = format!("National Insurance {GOLDEN_NI} on file.");
    let fields = detect_text(&text);
    let hits = field_texts_for(&fields, "uk_nino");
    assert_eq!(hits, vec![GOLDEN_NI]);
    let field = fields.iter().find(|f| f.label == "uk_nino").unwrap();
    assert_eq!(
        field.span.byte_offset,
        text.find(GOLDEN_NI).unwrap() as u64
    );
    assert_eq!(field.classification, "structured_identifier");
    assert_locatable(&text, field);
}

#[test]
fn golden_sort_code_is_located_at_its_real_byte_offset() {
    let text = format!("Sort code {GOLDEN_SORT_CODE}.");
    let fields = detect_text(&text);
    let hits = field_texts_for(&fields, "uk_sort_code");
    assert_eq!(hits, vec![GOLDEN_SORT_CODE]);
    let field = fields.iter().find(|f| f.label == "uk_sort_code").unwrap();
    assert_eq!(
        field.span.byte_offset,
        text.find(GOLDEN_SORT_CODE).unwrap() as u64
    );
    assert_locatable(&text, field);
}

#[test]
fn golden_account_number_is_located_at_its_real_byte_offset() {
    let text = format!("Account {GOLDEN_ACCOUNT} is the destination.");
    let fields = detect_text(&text);
    let hits = field_texts_for(&fields, "uk_account_number");
    assert_eq!(hits, vec![GOLDEN_ACCOUNT]);
    let field = fields.iter().find(|f| f.label == "uk_account_number").unwrap();
    assert_eq!(
        field.span.byte_offset,
        text.find(GOLDEN_ACCOUNT).unwrap() as u64
    );
    assert_locatable(&text, field);
}

#[test]
fn architecture_10_1_remaining_types_hit_on_goldens() {
    let text = format!(
        "NHS {GOLDEN_NHS} email {GOLDEN_EMAIL} phone {GOLDEN_PHONE} iban {GOLDEN_IBAN} card {GOLDEN_CARD}"
    );
    let fields = detect_text(&text);
    assert_eq!(field_texts_for(&fields, "uk_nhs_number"), vec![GOLDEN_NHS]);
    assert_eq!(field_texts_for(&fields, "email"), vec![GOLDEN_EMAIL]);
    assert_eq!(field_texts_for(&fields, "phone"), vec![GOLDEN_PHONE]);
    assert_eq!(field_texts_for(&fields, "iban"), vec![GOLDEN_IBAN]);
    assert_eq!(field_texts_for(&fields, "payment_card"), vec![GOLDEN_CARD]);
    for field in &fields {
        assert_locatable(&text, field);
    }
}

#[test]
fn one_document_with_the_aisha_shaped_triple_reports_all_three() {
    let text = format!("{GOLDEN_NI} {GOLDEN_SORT_CODE} {GOLDEN_ACCOUNT}");
    let fields = detect_text(&text);
    assert_eq!(field_texts_for(&fields, "uk_nino"), vec![GOLDEN_NI]);
    assert_eq!(field_texts_for(&fields, "uk_sort_code"), vec![GOLDEN_SORT_CODE]);
    assert_eq!(
        field_texts_for(&fields, "uk_account_number"),
        vec![GOLDEN_ACCOUNT]
    );
}

// ---------------------------------------------------------------------------
// dev-plan W13: "PDF/JSON keywords are not false-positive oracles (testing.md §7.2)"
// ---------------------------------------------------------------------------

#[test]
fn pdf_and_json_keywords_are_not_detected() {
    let text = r#"Type Font true null xref form stream obj {"id":"type","null":true}"#;
    let fields = detect_text(text);
    assert!(
        fields.is_empty(),
        "pattern pack must not treat PDF/JSON tokens as PII, got {fields:?}"
    );
}

#[test]
fn stub_canary_tokens_are_not_pattern_matches() {
    let text = "ordinary prose with PG-CANARY-X1 planted for the stub";
    let fields = detect_text(text);
    assert!(
        fields.is_empty(),
        "W12 stub tokens are not UK-shaped identifiers, got {fields:?}"
    );
}

/// An 8-digit account number must not also be reported as a 10-digit NHS number or a
/// 13–19 digit card just because it is numeric.
#[test]
fn eight_digit_account_is_not_also_nhs_or_card() {
    let text = format!("Account {GOLDEN_ACCOUNT} only.");
    let fields = detect_text(&text);
    assert!(field_texts_for(&fields, "uk_nhs_number").is_empty());
    assert!(field_texts_for(&fields, "payment_card").is_empty());
    assert_eq!(field_texts_for(&fields, "uk_account_number"), vec![GOLDEN_ACCOUNT]);
}
