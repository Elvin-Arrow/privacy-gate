//! `pg-patterns-uk-v1` — deterministic recognizers for structured identifiers
//! (architecture.md §10.1; W13).
//!
//! Architecture pins the *types*, not the regex text: UK sort code, account number,
//! National Insurance number, NHS number, email, phone, IBAN, payment-card (Luhn).
//! This module is the first stage of the eventual hybrid host; it is not
//! `SessionManager`'s default (that stays [`super::StubDetector`] until W15 wires
//! selection).
//!
//! Matching is per [`TextSpan`]: offsets are span-relative, then shifted by
//! `span.byte_offset`, the same locatability contract as [`super::StubDetector`].

use std::sync::LazyLock;

use regex::{Regex, RegexBuilder};

use crate::catalog::DetectedField;
use crate::importer::{Document, TextSpan};

use super::Detector;

/// architecture.md §10.1 identity for this pack. Recorded on audit `detect` once a host
/// actually selects this adapter (W15c); exposed now so tests can pin the string.
pub const PATTERNS_UK_V1_ID: &str = "pg-patterns-uk-v1";

const CLASSIFICATION: &str = "structured_identifier";

#[derive(Debug, Default)]
pub struct PatternsUkV1;

impl Detector for PatternsUkV1 {
    fn id(&self) -> &'static str {
        PATTERNS_UK_V1_ID
    }

    fn detect(&self, doc: &Document) -> Vec<DetectedField> {
        let mut fields = Vec::new();
        for page in &doc.pages {
            for span in &page.spans {
                collect_from_span(span, &mut fields);
            }
        }
        fields
    }
}

fn collect_from_span(span: &TextSpan, fields: &mut Vec<DetectedField>) {
    push_all(span, &NI, "uk_nino", fields);
    push_all(span, &SORT_CODE, "uk_sort_code", fields);
    push_all(span, &ACCOUNT, "uk_account_number", fields);
    for m in NHS.find_iter(&span.text) {
        if nhs_checksum_ok(m.as_str()) {
            push_field(span, m.start(), m.as_str(), "uk_nhs_number", fields);
        }
    }
    push_all(span, &EMAIL, "email", fields);
    push_all(span, &PHONE, "phone", fields);
    push_all(span, &IBAN, "iban", fields);
    for m in CARD.find_iter(&span.text) {
        if luhn_ok(m.as_str()) {
            push_field(span, m.start(), m.as_str(), "payment_card", fields);
        }
    }
}

fn push_all(span: &TextSpan, re: &Regex, label: &str, fields: &mut Vec<DetectedField>) {
    for m in re.find_iter(&span.text) {
        push_field(span, m.start(), m.as_str(), label, fields);
    }
}

fn push_field(
    span: &TextSpan,
    offset_in_span: usize,
    text: &str,
    label: &str,
    fields: &mut Vec<DetectedField>,
) {
    fields.push(DetectedField {
        id: uuid::Uuid::new_v4().to_string(),
        label: label.to_string(),
        classification: CLASSIFICATION.to_string(),
        span: TextSpan {
            byte_offset: span.byte_offset + offset_in_span as u64,
            byte_length: text.len() as u64,
            text: text.to_string(),
            page_index: span.page_index,
        },
        parent_field_id: None,
    });
}

/// Shape-only NI (2 letters, 6 digits, A–D). HMRC prefix exclusions are deliberately
/// omitted so testing.md §7.2's synthetic `QQ123456C` is a hit.
static NI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b[A-Z]{2}\d{6}[A-D]\b").expect("NI"));

static SORT_CODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d{2}-\d{2}-\d{2}\b").expect("sort code"));

static ACCOUNT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{8}\b").expect("account"));

static NHS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{10}\b").expect("NHS"));

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(r"\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b")
        .case_insensitive(true)
        .build()
        .expect("email")
});

static PHONE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:0\d{10}|\+44\d{10})\b").expect("phone"));

static IBAN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Z]{2}\d{2}[A-Z0-9]{11,30}\b").expect("IBAN"));

static CARD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{13,19}\b").expect("card"));

fn nhs_checksum_ok(digits: &str) -> bool {
    let ds: Vec<u32> = digits.chars().filter_map(|c| c.to_digit(10)).collect();
    if ds.len() != 10 {
        return false;
    }
    let sum: u32 = ds
        .iter()
        .take(9)
        .enumerate()
        .map(|(i, d)| d * (10 - i as u32))
        .sum();
    let check = 11 - (sum % 11);
    if check == 10 {
        return false;
    }
    let expected = if check == 11 { 0 } else { check };
    ds[9] == expected
}

fn luhn_ok(digits: &str) -> bool {
    let mut sum = 0u32;
    let mut double = false;
    for c in digits.chars().rev() {
        let Some(mut n) = c.to_digit(10) else {
            return false;
        };
        if double {
            n *= 2;
            if n > 9 {
                n -= 9;
            }
        }
        sum += n;
        double = !double;
    }
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nhs_checksum_accepts_the_published_test_number() {
        assert!(nhs_checksum_ok("9434765919"));
    }

    #[test]
    fn nhs_checksum_rejects_a_transposed_check_digit() {
        assert!(!nhs_checksum_ok("9434765918"));
    }

    #[test]
    fn luhn_accepts_the_visa_test_pan() {
        assert!(luhn_ok("4111111111111111"));
    }

    #[test]
    fn luhn_rejects_a_single_digit_flip() {
        assert!(!luhn_ok("4111111111111112"));
    }
}
