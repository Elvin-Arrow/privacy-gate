//! W25 — OQ-6 egress oracle (testing.md §7).
//!
//! Spec sources:
//! - `docs/specs/testing.md` §7.2 (raw UTF-8/UTF-16, extracted text, FlateDecode)
//! - `docs/dev-plan.md` W25 (oracle self-test; AC-2 uses oracle)
//!
//! Test harness only. Cloud AI body checks wait for W27. Ephemeral-override AC-2 is W26;
//! this chunk asserts the oracle against canonical person-export (W24).

mod common;

use common::oracle::{check, inject_flate_canary, REDACT_CANARY};
use pg_core::catalog::{RedactedDocument, RedactedPage};
use pg_core::export::render_redacted_pdf;
use pg_core::importer::{SourceFormat, TextSpan};

const KEEP: &str = "PG-CANARY-KEEP-A91C";

fn clean_pdf() -> Vec<u8> {
    render_redacted_pdf(&RedactedDocument {
        format: SourceFormat::Text,
        pages: vec![RedactedPage {
            page_index: 0,
            spans: vec![TextSpan {
                byte_offset: 0,
                byte_length: KEEP.len() as u64,
                text: KEEP.to_string(),
                page_index: 0,
            }],
        }],
    })
}

#[test]
fn oracle_accepts_a_clean_export() {
    let pdf = clean_pdf();
    check(&pdf, &[REDACT_CANARY], &[KEEP]).expect("clean export must pass the oracle");
}

#[test]
fn oracle_self_test_raw_injection_must_fail() {
    let mut dirty = clean_pdf();
    dirty.extend_from_slice(REDACT_CANARY.as_bytes());
    let err = check(&dirty, &[REDACT_CANARY], &[KEEP]).expect_err("raw leak must fail");
    assert!(
        err.iter().any(|f| f.place == "raw-utf8"),
        "oracle missed a raw UTF-8 plant: {err:?}"
    );
}

#[test]
fn oracle_self_test_flate_injection_must_fail() {
    let dirty = inject_flate_canary(&clean_pdf(), REDACT_CANARY);
    let err = check(&dirty, &[REDACT_CANARY], &[KEEP]).expect_err("flate leak must fail");
    assert!(
        err.iter().any(|f| f.place == "flate-stream"),
        "oracle missed a FlateDecode plant (testing.md §7.2 self-test): {err:?}"
    );
}
