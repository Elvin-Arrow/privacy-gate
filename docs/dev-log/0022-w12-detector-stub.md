# [0022] W12 — Detector host + stub

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Give `import_document` a real in-process Detector so AC-1's detect step is unblocked, without
shipping ONNX weights or any network. The stub must return locatable spans for fixture
sidecars, never crash on an empty hook, and leave `SessionManager`'s existing catalog tests
working.

Explicitly **not** in this chunk (dev-plan.md W12 "Do not: ONNX weights"): the UK pattern pack
(W13), ONNX (W15a), Ollama (W15b/W15c).

Per the [agent roster](../agent-roster.md), W12 is Sonnet tier ("stub has no real detection
risk yet").

## Implementation

### `core/src/detector.rs` — `Detector` trait, `StubDetector`, `NullDetector`

The trait is the v1 plugin hook (FR-2.4 / FR-9.4): one method, `detect(&Document) ->
Vec<DetectedField>`. `SessionManager::with_detector` is the registration point; v1 registers
exactly one implementation.

`StubDetector` (now `SessionManager`'s default) scans whitespace-delimited tokens for
`PG-CANARY-` and reports each as a locatable span at its real byte offset. Real prose never
matches. `NullDetector` is kept for tests that want a guaranteed empty field list.

`detector_id` on the audit `detect` event is `pg-detector-stub-v1` until W15a/W15b pick a
real identity (dev-plan W12: "or stub id documented in tests until W15a/W15b").

### `core/src/session.rs` — `import_document` runs detect

After extraction, `import_document` calls `self.detector.detect`, stores the fields on
`DocumentMeta`, and appends a `detect` audit row (`backend`/`model_tag`/`fallback_reason`
all null for the stub). Catalog `detected_field_count` is the API-visible signal; field ids
are UUIDs minted per detection, not a fixed fixture string.

## Resolution

- `core/tests/detector_w12.rs`: **10** tests — locatable offsets (including multi-byte UTF-8
  and multi-page PDF), empty/no-marker documents do not panic, `import_document` default
  path yields the expected canary count, import+detect advances the audit chain by two rows.
- No network: structural (`StubDetector` holds no fields and opens no sockets).
- Scope held: `SessionManager` default is the stub; W10 catalog tests that inject
  `NullDetector` are unchanged.

Next: W13 — pattern pack `pg-patterns-uk-v1`.

## Related Documentation

- [Development Plan — W12](../dev-plan.md#w12--detector-host--stub-unblocks-ac-1)
- [Agent roster — W12](../agent-roster.md)
- [Spec — design §2.2 (Detector)](../specs/design.md)
- [Spec — architecture §10](../specs/architecture.md)
- [Spec — testing §10 (detector stub)](../specs/testing.md)
- [Dev log 0021 — W11 retention gate](./0021-w11-retention-gate.md)
