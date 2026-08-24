# [0026] W15b — Ollama backend `pg-hybrid-ollama-v1`

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Ship the optional local-Ollama NER host as a **selectable** [`Detector`]: IP-literal
loopback only, no DNS, no ambient proxy, handshake (`GET /api/tags` + `POST /api/show`)
before any document text, allowlist/`-cloud`/digest gate, verify-then-trust chunk
offsets, rejection-threshold fallback. Stub remains `SessionManager`'s default
(selection is W15c).

Explicitly **not** in this chunk: `Config.detector_preference` commands or per-import
selection (W15c), Cloud AI, wiring this as the import default, downloading models.

Per the [agent roster](../agent-roster.md), W15b is Opus tier.

## Implementation

### `core/src/detector/ollama.rs`

- `OllamaClient::connect` takes a [`SocketAddr`] — non-loopback is refused; there is no
  hostname constructor, so `localhost` is never resolved. `reqwest` blocking client with
  `.no_proxy()` and `redirect::Policy::none()`. Handshake timeout 200 ms.
- `/api/show` is **POST** `{"model": "<tag>"}`, matching Ollama's documented API. The
  architecture spec previously said GET; corrected in this chunk.
- `verify_chunk_entity` is the §10.1.4 equality check (never a search). A chunk whose
  rejected/total rate exceeds `OFFSET_REJECT_THRESHOLD` (0.5) fails the whole Ollama pass
  (`fallback_reason: offset_verification_failed`); the pattern pack still runs.
- `OLLAMA_GEMMA4_E2B_DIGEST` and `GEMMA4_E2B_CONTEXT_TOKENS` are `None` until a nightly
  with a real daemon records them — fail-closed, same discipline as the ONNX pin.

`HybridOllamaV1` is selected via `with_detector`. `detect_with_outcome` exposes
`fallback_reason` / `model_tag` for W15c's audit fields.

### Nightly

`.github/workflows/nightly.yml` runs the mock-backed `ollama_w15b` suite always, and the
`#[ignore]` real-handshake test only when `ollama` is on the runner (otherwise
informational skip).

## Resolution

- `core/tests/ollama_w15b.rs`: non-loopback refuse; redirect not followed; `HTTP_PROXY`
  sees zero requests; malformed tags / unallowlisted / `-cloud` / digest mismatch each
  send zero `/api/generate`; unreachable → `ollama_unreachable`; happy path locates
  PERSON + NI and sends a JSON-schema `format` object (not `"json"`); offset mismatch
  rejected not searched; 2/3 rejection rate fails the whole pass; patterns still run on
  fallback.
- `cargo test -p pg-core` green; `cargo clippy -p pg-core -- -D warnings` clean.

`pg://detect-progress` `phase: "warming_model"` is already on the W14 enum; emitting it
from `import_document` waits until W15c actually selects this host on that path.

Next: W15c — backend selection + fallback orchestration.

## Related Documentation

- [Development Plan — W15b](../dev-plan.md#w15b--ollama-backend-pg-hybrid-ollama-v1)
- [Agent roster — W15b](../agent-roster.md)
- [Decision 0009](../decisions/0009-ollama-detector-backend.md)
- [Spec — architecture §10.1](../specs/architecture.md)
- [Spec — testing §7.4, §10, §11](../specs/testing.md)
- [Dev log 0025 — W15a hybrid ONNX](./0025-w15a-hybrid-onnx.md)
