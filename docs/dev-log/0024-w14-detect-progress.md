# [0024] W14 — `pg://detect-progress`

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Give `import_document` an in-process `pg://detect-progress` emitter so a subscriber sees a
monotonic `fraction` in 0..1 while detect runs. The UI bar is W32; Tauri `emit` is W29.
Must not report 100% before `detect` returns.

Explicitly **not** in this chunk: `phase: "warming_model"` emission (W15b), UI, blocking
pool / IPC (W29).

Per the [agent roster](../agent-roster.md), W14 is Haiku tier ("Simple event emission,
contract already fixed by `api.md`").

## Implementation

### `core/src/session.rs`

- `DETECT_PROGRESS_EVENT` = `"pg://detect-progress"`.
- `DetectProgress` / `DetectPhase` match api.md §6. `WarmingModel` is on the enum so the
  payload shape is already the spec's; W14 only emits `Detecting`.
- `ProgressSink` trait; default `NullProgressSink`. `SessionManager::with_progress_sink`
  is the registration point, same builder pattern as `with_detector`.
- `import_document` emits `fraction: 0.0` immediately before `detector.detect`, then
  `fraction: 1.0` after it returns. Early returns (retention gate, unsupported document)
  emit nothing because detect never runs.

Emit is synchronous: that is what lets a later blocking-pool import (api.md §5.3) flush
to the webview between fractions. No thread is spawned here.

## Resolution

- `core/tests/detect_progress_w14.rs`: **5** tests — event name; monotonic detecting
  fractions ending at 1.0 with matching `doc_id`; a probe `Detector` proves 1.0 is not
  already in the sink when `detect` is entered; retention-unset and empty-bytes paths
  emit zero events.
- `cargo test -p pg-core` green, including every prior import test (null sink is the
  default). `cargo clippy -p pg-core -- -D warnings` clean.

Next: W15a — hybrid ONNX (`pg-hybrid-v1`).

## Related Documentation

- [Development Plan — W14](../dev-plan.md#w14--pgdetect-progress)
- [Agent roster — W14](../agent-roster.md)
- [Spec — API §6 (events)](../specs/api.md)
- [Spec — UI §7.2 (progress bar)](../specs/ui.md)
- [Dev log 0023 — W13 pattern pack](./0023-w13-pattern-pack-uk.md)
