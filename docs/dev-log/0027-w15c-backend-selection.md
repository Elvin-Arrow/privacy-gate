# [0027] W15c — Backend selection + fallback orchestration

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Wire `Config.detector_preference` (`"auto"` factory, `"bundled_only"`) through
`get_detector_preference` / `set_detector_preference` (api.md §5.2) and select, **per
detect**, between `pg-hybrid-v1` and `pg-hybrid-ollama-v1` (architecture §10.1.3). Audit
`detect` rows record `backend`, `model_tag`, and `fallback_reason` so a fallback is never
hidden. `with_detector` still overrides selection so AC-1..AC-4 keep the stub.

Explicitly **not** in this chunk: caching the backend at unlock, a third preference
value, Cloud AI, recording a real Ollama digest, vendoring ONNX weights, approval (W16).

Per the [agent roster](../agent-roster.md), W15c is Opus tier.

## Implementation

### Preference commands

Unlocked-only, same generic config row as retention (C-API-6: unavailable while
degraded). `set_detector_preference` does **not** confirm retention. `set_retention_default`
preserves `detector_preference` (load-mutate-store of the whole `Config`).

### Per-detect selection (`SessionManager::detect_for_import`)

`detector_override: None` is now the production path (not a resident `StubDetector`):

1. `"bundled_only"` → `HybridV1::bundled()`, never probes Ollama.
2. `"auto"` + unreachable / schema / unallowlisted / digest mismatch → hybrid + matching
   `fallback_reason`.
3. `"auto"` + healthy allowlisted Ollama → `pg-hybrid-ollama-v1`, `backend: "ollama"`,
   `model_tag`.
4. Mid-document Ollama failure (e.g. offset threshold) → discard that NER pass and run
   hybrid for **that document**, `fallback_reason: "offset_verification_failed"`.

Handshake failure and `"bundled_only"` emit `pg://detect-progress` `phase: "detecting"`
only. `warming_model` is emitted only after a successful handshake.

Production `"auto"` probes `127.0.0.1:11434`. The digest pin is still `None`, so the
production allowlist is empty until a nightly records it — a live unpinned daemon cannot
be silently accepted.

## Resolution

- `core/tests/selection_w15c.rs`: preference persist/lock/degraded; the auto /
  bundled_only / fallback matrix against the W15b in-process mock; stub override; selection
  not cached at unlock; `warming_model` only on a successful handshake.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

Next: W16 — approval session.

## Related Documentation

- [Development Plan — W15c](../dev-plan.md#w15c--backend-selection--fallback-orchestration)
- [Agent roster — W15c](../agent-roster.md)
- [Decision 0009](../decisions/0009-ollama-detector-backend.md)
- [Spec — architecture §10.1.3](../specs/architecture.md)
- [Spec — api.md §5.2, §6](../specs/api.md)
- [Spec — data-model §5.5, §5.8.1](../specs/data-model.md)
- [Dev log 0026 — W15b Ollama backend](./0026-w15b-ollama-backend.md)
