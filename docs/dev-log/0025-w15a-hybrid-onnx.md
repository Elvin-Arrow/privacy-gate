# [0025] W15a — Hybrid ONNX host `pg-hybrid-v1`

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Ship the always-available in-process hybrid (`pg-hybrid-v1`): W13's `pg-patterns-uk-v1`
plus an on-device NER stage behind a SHA-256 pin. Mismatched pin is a hard failure of
NER, never a network fetch; the pattern pack still runs. Nightly job defined; pin
documented. Stub remains `SessionManager`'s default so AC-1..AC-4 stay decoupled.

Explicitly **not** in this chunk: GLiNER weights / `ort` as a default PR dependency
(testing.md: PR may skip heavy weights), Ollama (W15b), backend selection (W15c),
swapping the import default.

Per the [agent roster](../agent-roster.md), W15a is Opus tier.

## Implementation

### `core/src/detector/hybrid.rs`

- `verify_model_pin` — SHA-256 of the supplied bytes vs a `[u8; 32]` pin (`subtle`
  constant-time compare). No I/O, no fetch.
- `NerStage` trait for the second stage; fixture doubles implement it in tests.
- `HybridV1::with_ner` for injected stages; `HybridV1::from_pinned_bytes` verifies the
  pin **before** invoking the loader, so a mismatch cannot become a fail-open download.
- `NER_PII_ONNX_SHA256: Option<[u8; 32]> = None` until `models/ner-pii.onnx` is vendored.
- `Detector::id()` is `"pg-hybrid-v1"`.

`SessionManager::import_document` now records `self.detector.id()` on the audit `detect`
event (was hardcoded to the stub). Default detector is still `StubDetector`.
`SessionManager::lock` calls `Detector::on_lock` (architecture §10.2 unload hook);
`HybridV1` leaves that a no-op until real weights can be dropped *and* lazily reloaded.

### Nightly + pin docs

- `.github/workflows/nightly.yml` runs `cargo test -p pg-core --test hybrid_w15a`.
- `models/README.md` records how to fill in the pin when weights land.
- `models/*.onnx` is gitignored.

## Resolution

- `core/tests/hybrid_w15a.rs`: identity; stub still default; matching/mismatched pin;
  loader not invoked on mismatch; tiny fixture golden (PERSON/LOCATION + NI); matching
  pin loads NER; pattern stage is `PatternsUkV1`; shipped-artifact pin skip when the
  ONNX file is absent.
- `cargo test -p pg-core` green, including every prior import/stub/pattern/progress test
  unmodified. `cargo clippy -p pg-core -- -D warnings` clean.

Next: W15b — Ollama backend (`pg-hybrid-ollama-v1`).

## Related Documentation

- [Development Plan — W15a](../dev-plan.md#w15a--hybrid-onnx-pg-hybrid-v1)
- [Agent roster — W15a](../agent-roster.md)
- [Spec — architecture §4.2, §10.1, §10.2](../specs/architecture.md)
- [Spec — testing §3, §8 (model pin), §11 (ONNX golden job)](../specs/testing.md)
- [Dev log 0024 — W14 detect-progress](./0024-w14-detect-progress.md)
