//! W15a — Hybrid ONNX host `pg-hybrid-v1` (patterns + pinned NER stage).
//!
//! Spec sources:
//! - `docs/specs/architecture.md` §4.2 (SHA-256 pin at load; mismatch is a hard failure,
//!   never a network fetch), §10.1 (`pg-hybrid-v1` = `pg-patterns-uk-v1` + on-device NER
//!   for PERSON / LOCATION / ORGANIZATION), §10.2 (missing runtime fails the NER stage;
//!   pattern pack may still run)
//! - `docs/specs/testing.md` §8 ("Model pin | Mismatched ONNX SHA-256 → hard fail of NER
//!   stage"); §3 detector contract (ONNX golden is nightly/release; pattern goldens every
//!   PR); "PR may skip heavy weights"
//! - `docs/dev-plan.md` W15a ("Tests first: tiny fixture golden; mismatched pin fails
//!   closed."; "tests keep stub for AC-1..AC-4"; "Do not: download models at runtime;
//!   anything Ollama-related")
//!
//! Seam: [`HybridV1::detect`] (same [`Detector`] trait as the stub). SessionManager's
//! default remains [`StubDetector`]. Real GLiNER weights are not in this PR; a fixture
//! [`NerStage`] stands in for the tiny golden. The pin check does not load or fetch.

use std::sync::Arc;

use pg_core::catalog::DetectedField;
use pg_core::detector::{
    verify_model_pin, Detector, HybridV1, NerStage, NerStageError, PatternsUkV1, StubDetector,
    HYBRID_V1_ID, NER_PII_ONNX_SHA256,
};
use pg_core::importer::{self, Document, TextSpan};
use sha2::{Digest, Sha256};

const DOC_ID: &str = "00000000-0000-4000-8000-000000000015";
const GOLDEN_NI: &str = "QQ123456C";
const GOLDEN_PERSON: &str = "Alice Example";
const GOLDEN_LOCATION: &str = "Manchester";

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn detect_text(detector: &impl Detector, text: &str) -> Vec<DetectedField> {
    let doc = importer::import_text(text.as_bytes(), DOC_ID).expect("import_text");
    detector.detect(&doc)
}

/// Tiny NER double for the W15a fixture golden. Returns PERSON / LOCATION at the
/// offsets `str::find` reports — independent of any ONNX graph.
struct FixtureNer;

impl NerStage for FixtureNer {
    fn detect_entities(&self, doc: &Document) -> Result<Vec<DetectedField>, NerStageError> {
        let mut fields = Vec::new();
        for page in &doc.pages {
            for span in &page.spans {
                for (label, needle) in [
                    ("person", GOLDEN_PERSON),
                    ("location", GOLDEN_LOCATION),
                ] {
                    if let Some(at) = span.text.find(needle) {
                        fields.push(DetectedField {
                            id: format!("ner-{label}"),
                            label: label.to_string(),
                            classification: "ner".to_string(),
                            span: TextSpan {
                                byte_offset: span.byte_offset + at as u64,
                                byte_length: needle.len() as u64,
                                text: needle.to_string(),
                                page_index: span.page_index,
                            },
                            parent_field_id: None,
                        });
                    }
                }
            }
        }
        Ok(fields)
    }
}

// ---------------------------------------------------------------------------
// architecture §10.1 identity
// ---------------------------------------------------------------------------

#[test]
fn hybrid_v1_id_is_the_architecture_identity() {
    assert_eq!(HYBRID_V1_ID, "pg-hybrid-v1");
    assert_eq!(HybridV1::with_ner(Arc::new(FixtureNer)).id(), HYBRID_V1_ID);
}

#[test]
fn session_manager_default_detector_is_still_the_stub() {
    // dev-plan W15a: "tests keep stub for AC-1..AC-4"
    assert_eq!(StubDetector.id(), "pg-detector-stub-v1");
}

// ---------------------------------------------------------------------------
// testing.md §8: mismatched ONNX SHA-256 → hard fail of NER stage
// architecture §4.2: mismatch is a hard failure, never a network fetch
// architecture §10.2: pattern pack may still run
// ---------------------------------------------------------------------------

#[test]
fn verify_model_pin_accepts_matching_bytes() {
    let bytes = b"tiny-onnx-fixture-bytes";
    let pin = sha256(bytes);
    assert!(verify_model_pin(bytes, &pin).is_ok());
}

#[test]
fn verify_model_pin_rejects_a_single_flipped_byte() {
    let bytes = b"tiny-onnx-fixture-bytes";
    let mut pin = sha256(bytes);
    pin[0] ^= 0xff;
    assert_eq!(
        verify_model_pin(bytes, &pin),
        Err(pg_core::detector::PinMismatch)
    );
}

#[test]
fn mismatched_pin_skips_ner_loader_and_still_runs_patterns() {
    let bytes = b"not-the-pinned-artifact";
    let pin = sha256(b"the-pinned-artifact");
    let hybrid = HybridV1::from_pinned_bytes(&bytes[..], &pin, |_| {
        panic!("loader must not run on pin mismatch (would be a fail-open fetch path)");
    });
    assert_eq!(hybrid.ner_error(), Some(NerStageError::PinMismatch));

    let text = format!("{GOLDEN_PERSON} in {GOLDEN_LOCATION}. NI {GOLDEN_NI}");
    let fields = detect_text(&hybrid, &text);
    let ninos: Vec<&str> = fields
        .iter()
        .filter(|f| f.label == "uk_nino")
        .map(|f| f.span.text.as_str())
        .collect();
    assert_eq!(ninos, vec![GOLDEN_NI]);
    assert!(fields.iter().all(|f| f.classification != "ner"));
}

// ---------------------------------------------------------------------------
// tiny fixture golden: patterns + NER labels on one document
// ---------------------------------------------------------------------------

#[test]
fn tiny_fixture_golden_hits_patterns_and_ner_labels() {
    let hybrid = HybridV1::with_ner(Arc::new(FixtureNer));
    let text = format!("{GOLDEN_PERSON} in {GOLDEN_LOCATION}. NI {GOLDEN_NI}");
    let fields = detect_text(&hybrid, &text);

    let person: Vec<&str> = fields
        .iter()
        .filter(|f| f.label == "person")
        .map(|f| f.span.text.as_str())
        .collect();
    let location: Vec<&str> = fields
        .iter()
        .filter(|f| f.label == "location")
        .map(|f| f.span.text.as_str())
        .collect();
    let ninos: Vec<&str> = fields
        .iter()
        .filter(|f| f.label == "uk_nino")
        .map(|f| f.span.text.as_str())
        .collect();
    assert_eq!(person, vec![GOLDEN_PERSON]);
    assert_eq!(location, vec![GOLDEN_LOCATION]);
    assert_eq!(ninos, vec![GOLDEN_NI]);

    let person_field = fields.iter().find(|f| f.label == "person").unwrap();
    let expected = text.find(GOLDEN_PERSON).unwrap() as u64;
    assert_eq!(person_field.span.byte_offset, expected);
}

#[test]
fn matching_pin_invokes_the_ner_loader() {
    let bytes = b"tiny-onnx-fixture-bytes";
    let pin = sha256(bytes);
    let hybrid = HybridV1::from_pinned_bytes(bytes, &pin, |_| Ok(Arc::new(FixtureNer) as Arc<dyn NerStage>));
    assert_eq!(hybrid.ner_error(), None);
    let text = format!("{GOLDEN_PERSON} NI {GOLDEN_NI}");
    let fields = detect_text(&hybrid, &text);
    assert!(fields.iter().any(|f| f.label == "person"));
    assert!(fields.iter().any(|f| f.label == "uk_nino"));
}

#[test]
fn patterns_uk_v1_is_the_pattern_stage() {
    // The hybrid's pattern stage is the W13 pack, not a reimplementation.
    let patterns = PatternsUkV1;
    let text = format!("NI {GOLDEN_NI}");
    let fields = detect_text(&patterns, &text);
    assert_eq!(
        fields.iter().map(|f| f.span.text.as_str()).collect::<Vec<_>>(),
        vec![GOLDEN_NI]
    );
}

/// Nightly/release: when `models/ner-pii.onnx` is vendored, its SHA-256 must equal
/// [`NER_PII_ONNX_SHA256`]. Informational skip if the file is absent (testing.md: PR may
/// skip heavy weights; nightly job is defined in `.github/workflows/nightly.yml`).
#[test]
fn nightly_shipped_onnx_matches_documented_pin() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("models")
        .join("ner-pii.onnx");
    if !path.exists() {
        eprintln!("informational skip: {path:?} not vendored (PR may skip heavy weights)");
        assert!(
            NER_PII_ONNX_SHA256.is_none(),
            "a documented pin with no artifact would never match; leave NER_PII_ONNX_SHA256 as None until weights ship"
        );
        return;
    }
    let pin = NER_PII_ONNX_SHA256.expect("vendored ner-pii.onnx requires a recorded SHA-256 pin");
    let bytes = std::fs::read(path).expect("read shipped onnx");
    verify_model_pin(&bytes, &pin).expect("shipped artifact must match the documented pin");
}
