//! `pg-hybrid-v1` — W13 patterns plus an on-device NER stage (architecture.md §10.1, §10.2; W15a).
//!
//! Real GLiNER ONNX weights are not in this crate yet (testing.md: PR may skip heavy
//! weights; no runtime download). [`verify_model_pin`] is the load-time integrity check
//! architecture §4.2 requires: a mismatch is a hard failure of the NER stage, never a
//! fetch. [`HybridV1::from_pinned_bytes`] refuses to invoke the NER loader on mismatch so
//! that path cannot become a fail-open download. The pattern pack still runs
//! (architecture §10.2).
//!
//! [`SessionManager`](crate::session::SessionManager) selects this host on
//! `"bundled_only"` and as the `"auto"` fallback (W15c). Tests that need the W12 stub
//! (AC-1..AC-4) still pass [`super::StubDetector`] via `with_detector`.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::catalog::DetectedField;
use crate::importer::Document;

use super::{Detector, PatternsUkV1};

/// architecture.md §10.1 identity for the always-available in-process hybrid.
pub const HYBRID_V1_ID: &str = "pg-hybrid-v1";

/// SHA-256 of the shipped `models/ner-pii.onnx` artifact (architecture §4.2 / §10.1).
///
/// `None` until that file is vendored. A `Some` pin with no artifact on disk can never
/// match — leave this `None` until the weights land, then record the digest here and in
/// `models/README.md`. Nightly asserts the two agree when the file is present.
pub const NER_PII_ONNX_SHA256: Option<[u8; 32]> = None;

/// The NER stage failed closed. Pattern results from [`PatternsUkV1`] are still returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NerStageError {
    /// architecture §4.2: SHA-256 of the bytes did not match the documented pin.
    PinMismatch,
    /// The loaded stage ran and failed (missing/ABI-mismatched runtime, inference error).
    InferenceFailed,
    /// Weights or runtime are not available in this build.
    Unavailable,
}

/// architecture §4.2 pin mismatch — distinct from a generic NER error so the loader
/// path can be skipped without constructing a stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinMismatch;

/// Second stage of [`HybridV1`]. Fixture doubles implement this in tests; the real ONNX
/// graph (when vendored) will too. Not a [`Detector`] of its own — it must not run the
/// pattern pack.
pub trait NerStage: Send + Sync {
    fn detect_entities(&self, doc: &Document) -> Result<Vec<DetectedField>, NerStageError>;
}

/// SHA-256 pin check at load (architecture §4.2). Mismatch is a hard failure, not a
/// network fetch — this function never I/O's and never downloads.
pub fn verify_model_pin(bytes: &[u8], expected: &[u8; 32]) -> Result<(), PinMismatch> {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    if bool::from(digest.ct_eq(expected)) {
        Ok(())
    } else {
        Err(PinMismatch)
    }
}

/// In-process hybrid: [`PatternsUkV1`] plus an optional [`NerStage`].
///
/// Construction via [`HybridV1::from_pinned_bytes`] is the only path that consults the
/// pin. [`HybridV1::with_ner`] is for tests that inject a fixture stage without an
/// artifact.
pub struct HybridV1 {
    patterns: PatternsUkV1,
    ner: Option<Arc<dyn NerStage>>,
    ner_error: Option<NerStageError>,
}

impl HybridV1 {
    /// The always-available bundled host (architecture §10.1.3): pattern pack plus NER
    /// when weights are present. Until `ner-pii.onnx` is vendored this is patterns-only
    /// (architecture §10.2: missing NER fails that stage; the pack still runs).
    pub fn bundled() -> Self {
        Self {
            patterns: PatternsUkV1,
            ner: None,
            ner_error: None,
        }
    }

    /// Inject a NER stage that is already trusted (fixture goldens). Does not consult
    /// [`NER_PII_ONNX_SHA256`].
    pub fn with_ner(ner: Arc<dyn NerStage>) -> Self {
        Self {
            patterns: PatternsUkV1,
            ner: Some(ner),
            ner_error: None,
        }
    }

    /// Verify `bytes` against `pin`, then invoke `loader` **only** on a match.
    ///
    /// Pin mismatch: `loader` is not called (architecture §4.2 — never a fetch),
    /// [`ner_error`](Self::ner_error) is [`NerStageError::PinMismatch`], and
    /// [`detect`](Detector::detect) still runs the pattern pack.
    pub fn from_pinned_bytes<F>(bytes: &[u8], pin: &[u8; 32], loader: F) -> Self
    where
        F: FnOnce(&[u8]) -> Result<Arc<dyn NerStage>, NerStageError>,
    {
        match verify_model_pin(bytes, pin) {
            Ok(()) => match loader(bytes) {
                Ok(ner) => Self {
                    patterns: PatternsUkV1,
                    ner: Some(ner),
                    ner_error: None,
                },
                Err(e) => Self {
                    patterns: PatternsUkV1,
                    ner: None,
                    ner_error: Some(e),
                },
            },
            Err(PinMismatch) => Self {
                patterns: PatternsUkV1,
                ner: None,
                ner_error: Some(NerStageError::PinMismatch),
            },
        }
    }

    pub fn ner_error(&self) -> Option<NerStageError> {
        self.ner_error
    }
}

impl Detector for HybridV1 {
    fn id(&self) -> &'static str {
        HYBRID_V1_ID
    }

    fn detect(&self, doc: &Document) -> Vec<DetectedField> {
        let mut fields = self.patterns.detect(doc);
        // architecture §10.2: a failed NER stage is fail-closed for NER only; patterns
        // already in `fields` still return.
        if let Some(ner) = &self.ner {
            if let Ok(more) = ner.detect_entities(doc) {
                fields.extend(more);
            }
        }
        fields
    }
}
