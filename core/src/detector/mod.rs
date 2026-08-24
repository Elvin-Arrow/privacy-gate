//! The Detector host seam (`design.md` §2.2; architecture §10; W12/W13).
//!
//! `import_document` (W10) needed *something* here before real detection existed —
//! dev-plan W10: "Detection may be a no-op empty field list only if W12 is the next PR"
//! (it was, in this sequence). W12 is that PR: [`StubDetector`] is now
//! `SessionManager`'s default, in-process, no network, no model weights (dev-plan W12 "Do
//! not: ONNX weights in this PR if they bloat CI; stub is enough for AC-1").
//!
//! W13 adds [`PatternsUkV1`] (`pg-patterns-uk-v1`) as the hybrid's deterministic first
//! stage. W15a adds [`HybridV1`] (`pg-hybrid-v1`) — that pack plus an on-device NER stage
//! behind a SHA-256 pin ([`verify_model_pin`]). Neither is the import default (dev-plan
//! W15a: "tests keep stub for AC-1..AC-4"); `SessionManager::with_detector` selects them.
//! Real GLiNER weights are not in this crate (PR may skip them); Ollama is W15b.
//!
//! # What the stub actually does
//!
//! testing.md §10: "Detector stub: implements the same host-facing trait as
//! `pg-hybrid-v1`; returns the sidecar fields. Used by AC-1..AC-4 so model drift cannot
//! hide a vault bug." [`StubDetector`] scans every span for whitespace-delimited tokens
//! containing [`STUB_CANARY_MARKER`] (`"PG-CANARY-"`) and reports each as a
//! [`DetectedField`] at its real byte offset within that span. Real prose without a
//! planted marker never matches.
//!
//! # The "empty" plugin hook (FR-2.4 / FR-9.4)
//!
//! design §2.2: "Expose detector plugin hooks... v1 ships the hooks empty of first-party
//! plugins." The [`Detector`] trait *is* that hook — `SessionManager::with_detector` is
//! the registration point, and v1 registers exactly one implementation
//! ([`StubDetector`], later replaced by the real host) and no third-party plugins. A
//! separate plugin-loading mechanism is out of scope for v1 entirely (testing.md §14:
//! "Third-party plugin / WASM tests → later phase").

mod hybrid;
mod patterns_uk;

pub use hybrid::{
    verify_model_pin, HybridV1, NerStage, NerStageError, PinMismatch, HYBRID_V1_ID,
    NER_PII_ONNX_SHA256,
};
pub use patterns_uk::{PatternsUkV1, PATTERNS_UK_V1_ID};

use crate::catalog::DetectedField;
use crate::importer::{Document, TextSpan};

/// One in-process detection call over an already-imported [`Document`]. The real hosts
/// (W13's pattern pack, W15a's ONNX hybrid, W15b's optional Ollama backend) all implement
/// this same trait — "the same host-facing trait as `pg-hybrid-v1`" testing.md §10
/// requires of the stub is this one.
pub trait Detector: Send + Sync {
    /// Identity recorded on the audit `detect` event (`data-model.md` / api.md §6).
    fn id(&self) -> &'static str;

    fn detect(&self, doc: &Document) -> Vec<DetectedField>;

    /// Drop in-process NER weights (architecture §10.2). [`crate::session::SessionManager::lock`]
    /// always calls this. Default is a no-op; a real ONNX session will override once
    /// weights are vendored and can be reloaded lazily after the next unlock.
    fn on_lock(&self) {}
}

/// The W10-era placeholder: no network, no model, no fields, ever. Kept for tests that
/// want to assert an import produces zero detections regardless of content — distinct from
/// [`StubDetector`], which does real (if narrow) work.
#[derive(Debug, Default)]
pub struct NullDetector;

impl Detector for NullDetector {
    fn id(&self) -> &'static str {
        "pg-null-v1"
    }

    fn detect(&self, _doc: &Document) -> Vec<DetectedField> {
        Vec::new()
    }
}

/// `SessionManager`'s default detector (W12). See module docs for what it matches.
pub const STUB_DETECTOR_ID: &str = "pg-detector-stub-v1";

/// Any whitespace-delimited token containing this substring is a detected field. Chosen to
/// be unmistakably synthetic and never collide with real prose, PDF tokens, or JSON keys
/// (testing.md §7.2's canary discipline) — and distinct from W8/W9's own
/// `PG-FIXTURE-CANARY-...` importer fixtures, so detector tests and importer tests can
/// never accidentally cross-trigger each other.
pub const STUB_CANARY_MARKER: &str = "PG-CANARY-";

/// The real (if narrow) v1 default detector — see module docs.
#[derive(Debug, Default)]
pub struct StubDetector;

impl Detector for StubDetector {
    fn id(&self) -> &'static str {
        STUB_DETECTOR_ID
    }

    fn detect(&self, doc: &Document) -> Vec<DetectedField> {
        let mut fields = Vec::new();
        for page in &doc.pages {
            for span in &page.spans {
                for (offset_in_span, token) in whitespace_tokens(&span.text) {
                    if token.contains(STUB_CANARY_MARKER) {
                        fields.push(DetectedField {
                            id: uuid::Uuid::new_v4().to_string(),
                            label: "stub_canary".to_string(),
                            classification: "synthetic_canary".to_string(),
                            span: TextSpan {
                                byte_offset: span.byte_offset + offset_in_span as u64,
                                byte_length: token.len() as u64,
                                text: token.to_string(),
                                page_index: span.page_index,
                            },
                            parent_field_id: None,
                        });
                    }
                }
            }
        }
        fields
    }
}

/// Maximal runs of non-whitespace characters in `text`, each paired with its byte offset
/// from the start of `text`. `char_indices` gives byte positions directly, so this is
/// correct for multi-byte UTF-8 content without any separate length bookkeeping.
fn whitespace_tokens(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                out.push((s, &text[s..i]));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(s) = start {
        out.push((s, &text[s..]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_tokens_finds_byte_offsets_across_multi_byte_utf8() {
        let text = "café PG-CANARY-1 end";
        let tokens = whitespace_tokens(text);
        // "café" is 5 bytes (é is 2 bytes) + 1 byte for the following space = the next
        // token starts at byte 6 — pinned explicitly so a future edit that swaps byte
        // offsets for char offsets breaks this test immediately.
        assert_eq!("café".len(), 5);
        assert_eq!(tokens, vec![(0, "café"), (6, "PG-CANARY-1"), (18, "end")]);
    }
}
