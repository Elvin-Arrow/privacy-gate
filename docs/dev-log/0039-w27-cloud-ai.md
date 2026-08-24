# [0039] W27 — Cloud AI plugin (mock HTTP)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Let a user send an approved document's text to a user-configured, OpenAI-compatible
endpoint for AI processing (FR-5.2), with the same "exactly what will leave" preview
guarantee export already has (FR-6.1), the API key never leaving the vault except at
invocation (architecture §9.1), and every attempt — successful or not — landing in the
audit trail (architecture §9.3). AC-3 is the acceptance scenario; no real vendor in CI
(dev-plan.md W27).

## Implementation

- `core/src/cloud_ai.rs` (new):
  - `CloudAiSecret` storage: same two-AEAD-layer envelope shape `crate::config::Config`
    established (kind 5 `PluginSecret` plaintext, kind 7 `WrappedDek` wrap), behind a
    `CloudAiStore` trait (`load`/`store`/`clear`) and `NullCloudAiStore` default — unlike
    `Config`, absence is meaningful ("not configured"), so `clear` is a real cryptographic
    erase (architecture §4.3), not "write the default back."
  - `validate_endpoint_url`: api.md §5.7's `https://` + host + no-userinfo rule, returning
    the host string `cloud_ai_get_config`/`cloud_ai_set_config`/the audit `share` payload
    all call `endpoint_host`.
  - `CloudAiClient`: the Rust-side-only HTTP boundary (architecture §9.2) — `reqwest`
    blocking, `no_proxy()`, `redirect::Policy::none()` (a redirect response is simply a
    non-2xx status, so "never followed" and "refused" are the same code path, mirroring how
    `crate::detector::ollama::OllamaClient` already treats a redirect). `test()` sends a
    fixed ping with no document content (C-API-4); `send()` posts an OpenAI
    Chat-Completions-shaped body (`model` + `messages[]`, matching architecture §9.1's
    "OpenAI-compatible base URL") with the approved text carried in its own top-level
    `"document"` field so the identity guarantee below is a field comparison, not a
    substring argument into a composed prompt.
- `core/src/vault.rs`: `CloudAiStore for SqlCipherVault` over the `artifact`/`plugin_secret`
  tables W3's schema already scaffolded (`kind IN (..., 5)`, `uq_artifact_cloud_ai`,
  `plugin_secret(plugin_id, artifact_id)` — all present since W3, unused until now). Delete
  order for `store`/`clear` is `plugin_secret` before `artifact` (the foreign key direction,
  `PRAGMA foreign_keys = ON`).
- `core/src/export.rs`: `plain_text_from_pages` — the AI share body, built by reusing
  `page_plain_text` (the same span-selection the PDF renderer already uses) rather than a
  second walk over `RedactedPage`s, so testing.md §5.3's gated "code that selects bytes for
  PDF / HTTP" stays one implementation.
- `core/src/session.rs`:
  - `cloud_ai_set_config` / `get_config` / `clear_config` / `test` — new session-table rows,
    generic config-command posture (`no | no | yes | no`, unavailable while degraded).
  - `preview_share`: `share_to_ai` now validates `ai_instruction` (1..=4000 chars, api.md
    §4) and `recipient_note` must be null, then checks a Cloud AI secret exists **before**
    any document is loaded (api.md §5.6 "fail before assembling a send") — same
    override/manifest loop as export, branching only at the end on PDF bytes vs.
    `plain_text_from_pages`.
  - `commit_share` split into `commit_export_share` (unchanged behavior) and
    `commit_ai_share`: reloads the secret fresh rather than trusting what `preview_share`
    saw (architecture §9.1: readable "only at invocation"), so a secret cleared between
    preview and commit surfaces as `cloud_ai_not_configured`, not a stale send. Every
    outcome — success, `cloud_ai_network`, `cloud_ai_refused`, or
    `cloud_ai_not_configured` — appends one audit `share` event (dev-plan W27 "Failed HTTP
    still audits attempt") and drops the token (api.md §5.6 "after success or definitive
    failure").
  - `test_only_set_cloud_ai_config`: a test seam (mirrors `test_only_expire_preview`) that
    stores a secret bypassing `cloud_ai_set_config`'s `https://` validation, so tests can
    point it at a plain-HTTP loopback mock — the TLS requirement is a production-endpoint
    property enforced at the command layer, not something a local test double needs to
    satisfy, the same reasoning `crate::detector::ollama`'s mock already relies on.
- `core/src/api.rs`: `ApiError::cloud_ai_network` / `cloud_ai_refused` constructors (the
  error codes already existed from an earlier chunk's api.md transcription; only the
  builders were missing).

## Resolution

- `core/tests/cloud_ai_w27.rs`: an in-process HTTP mock (`MockCloudAi`, same
  `TcpListener`-based shape as `ollama_w15b.rs`'s `MockOllama`, testing.md §10) plus 17
  tests — config validation (`https://` / userinfo / empty model or key rejected;
  `key_last4` correct; `api_key` never in `cloud_ai_get_config`'s type at all, not just at
  runtime), `cloud_ai_test` sending no document content, `preview_share` failing closed
  before any network call (missing instruction, over-length instruction, a `recipient_note`
  set, or no configured secret), the AC-3 flow (mock receives a `"document"` field
  byte-identical to `ai_payload_preview`; OQ-6 oracle on the raw wire body; second commit on
  the same token is `preview_expired`), audited failure paths (unreachable host →
  `cloud_ai_network`, mock 5xx → `cloud_ai_refused`, secret cleared after preview →
  `cloud_ai_not_configured` — each with the matching `error_class` in the audit row and the
  token still dropped), and the redirect-refusal case (a same-mock 302 to another host
  never gets a second connection).
- `cargo test -p pg-core`: full suite green (`cloud_ai_w27` 17/17; every pre-existing suite,
  including `share_w24.rs`'s `share_to_ai_is_cloud_ai_not_configured`, unmodified and
  green). `cargo clippy -p pg-core --lib` and `--test cloud_ai_w27` clean; a pre-existing
  `--all-targets` clippy failure in `ollama_w15b.rs` (a newer-clippy `assertions_on_constants`
  lint on a W15b test, unrelated to this chunk and not CI-gated today) was left alone.

Next: W28 — `list_audit_events` (AC-4).

## Related Documentation

- [Development Plan — W27](../dev-plan.md#w27--cloud-ai-plugin-mock-http)
- [Spec — srs.md FR-5.2](../specs/srs.md)
- [Spec — architecture.md §8–§9](../specs/architecture.md)
- [Spec — api.md §5.6–§5.7](../specs/api.md)
- [Spec — data-model.md §5.7 `CloudAiSecret`](../specs/data-model.md)
- [Spec — testing.md §6.3 AC-3](../specs/testing.md)
- [Dev log 0038 — W26 overrides](./0038-w26-overrides.md)
