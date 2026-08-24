# [0039] W27 — Cloud AI plugin (mock HTTP)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Deliver the Cloud AI plugin (architecture §8–§9, FR-5.2, AC-3): `cloud_ai_set_config` /
`_get_config` / `_clear_config` / `_test`, and the `share_to_ai` kind of `preview_share` /
`commit_share`. The API key is user-supplied, envelope-encrypted in the Vault, and never
crosses back to the webview after `cloud_ai_set_config` returns. All Cloud AI HTTP happens
in the Rust core, not the webview (C-ARCH-2) — this is what makes "what left the device" a
core-auditable event.

## Implementation

- `core/src/cloud_ai.rs` (new): `CloudAiSecret` (data-model §5.7) with a hand-written
  redacting `Debug`; `PluginSecretStore` trait + `NullPluginSecretStore`, mirroring
  `crate::config`'s `seal`/`open` two-AEAD-layer shape against AAD kind 5
  (`ArtifactKind::PluginSecret`) — except unlike `Config`, absence is meaningful ("not
  configured"), so the trait has an explicit `clear` rather than an always-write `store`.
  `validate_https_endpoint` is a small hand-rolled parser (`https://` scheme, non-empty
  host, no userinfo) rather than pulling in the `url` crate for one check — `reqwest`
  already depends on `url` transitively and `CloudAiClient` uses `reqwest::Url::parse` for
  its own host extraction, but the public validation surface stays dependency-free.
  `CloudAiClient` (`reqwest::blocking`, TLS 1.2+ floor) POSTs an OpenAI-compatible
  chat-completion body (`architecture.md` §9.1: "OpenAI-compatible base URL + model id") and
  parses `choices[0].message.content` as `output_text`. Its redirect policy is
  `Policy::custom`: refuse only a redirect that would change host (`Attempt::error`, which
  `reqwest::Error::is_redirect()` distinguishes from an ordinary network failure) — the
  literal architecture §9.2 wording ("redirects that change host are refused"), not the
  stricter "refuse every redirect" fallback the task notes offered; a same-host redirect
  still follows.
- `core/src/vault.rs`: `impl PluginSecretStore for SqlCipherVault` over the `plugin_secret`
  table's single `plugin_id = 'cloud_ai'` row (data-model §7) joined to its `kind=5`
  artifact — same `with_conn`/transaction shape as `ConfigStore`, plus a `clear` that
  deletes both rows inside one transaction (cryptographic erasure — architecture §4.3 —
  same discipline as `delete_variant`/`destroy_document`).
- `core/src/session.rs`: `plugin_secrets: Arc<dyn PluginSecretStore>` field defaulting to
  `NullPluginSecretStore` in `new_full`, plus `with_plugin_secrets` (mirrors
  `with_documents` — `new_full`'s signature is unchanged). Four new commands with DTOs
  matching api.md §5.7 exactly (`CloudAiSetConfigIn` has a hand-written redacting `Debug`
  too, so the key can't leak through a stray `{:?}` before the command returns). All four
  added to `SESSION_TABLE` under the generic `[Unlocked]` config/document row
  (`core/tests/session_gating_w4.rs` only enumerates the five W4-era commands per its own
  module docs, so it needed no new rows).
  `preview_share`/`commit_share`: `ShareKind::ShareToAi` no longer hard-fails. `preview_share`
  validates `doc_ids` non-empty, then branches — export keeps its existing
  `ai_instruction`-must-be-null check; AI validates `ai_instruction` is 1..=4000 chars, then
  checks `plugin_secrets.load(..).is_some()` **before** the per-doc loop that reads any
  document content, so `cloud_ai_not_configured` never touches the catalog or attempts HTTP.
  The per-doc override/variant loop (W26) is now shared by both kinds; only the tail differs
  — export renders a PDF via `crate::share`, AI concatenates each page's surviving
  (non-redacted) span text via a new `pages_to_text` helper, joined by blank lines. That
  text becomes both `ai_payload_preview` and (via the extended `LivePreview` — now carrying
  `ai_payload`/`ai_instruction` alongside the export fields, `suggested_filename` turned
  `Option`) exactly what `commit_share` POSTs — `CloudAiClient::send` only wraps it with the
  instruction and a fixed system preamble, never mutates it, giving the same byte-identical
  guarantee W24 established for the PDF. `commit_share`'s AI arm builds the client from the
  stored secret, appends the `share` audit event (`endpoint_host` real, `has_ai_instruction:
  true`, `error_class` set on failure, never the instruction text or response body)
  **before** returning either `Ok` or the mapped `cloud_ai_network`/`cloud_ai_refused` error
  — architecture §9.3's "failed sends still emit a share event."
- `core/src/api.rs`: added `ApiError::cloud_ai_network()` / `cloud_ai_refused()`
  constructors (fixed `&'static str` classes, same C-API-1 discipline as every other
  constructor in the file — no interpolated host or response text).

## Resolution

- `core/tests/cloud_ai_w27.rs` (new, 17 tests): session gating for all four commands;
  `cloud_ai_set_config` rejects `http://`/`file://`/userinfo and accepts a valid
  `https://` endpoint with the right `endpoint_host`/`key_last4`; `cloud_ai_get_config`
  never serializes `api_key`; `cloud_ai_clear_config` is idempotent; `share_to_ai` without
  config fails `cloud_ai_not_configured` before the mock records a single connection; empty
  and missing `ai_instruction` are `invalid_input`; a full preview→commit round trip proves
  the mock receives the exact `ai_payload_preview` text and the OQ-6 canary oracle holds
  (raw-byte scan arm only — the oracle's PDF-extraction "kept" arm doesn't apply to a
  plain-text payload, asserted directly instead); a redirect to a different loopback host is
  refused (`cloud_ai_refused`) without ever dialing the redirect target; a 500 response
  surfaces as `cloud_ai_network`; both failure paths still append a `share` audit event with
  `error_class` and no key/body; `cloud_ai_test` against a doc-bearing session sends a
  bodyless probe (asserted: no `PG-CANARY`/document text in the body) and reports
  `ok`/`error_class` for both a healthy and a failing endpoint.
- **TLS-mock gap (documented, not closed):** this repo has no TLS-capable test double
  anywhere (`ollama_w15b.rs`'s mock is plain HTTP too). `endpoint_url`'s `https://`-only
  rule is proven at `cloud_ai_set_config` (unit tests in `core/src/cloud_ai.rs` plus the
  integration rejection tests above); the HTTP-send/redirect/audit behavior is proven
  against a plain-HTTP mock reached via a new test-only seam,
  `SessionManager::test_only_set_cloud_ai_secret`, which bypasses the `https://` check the
  way production `cloud_ai_set_config` never would (mirrors `test_only_expire_preview`'s
  precedent). What is **not** proven: a real TLS handshake against a real HTTPS host behaves
  identically — that needs a TLS-capable mock, a gap carried forward the same way
  `OLLAMA_GEMMA4_E2B_DIGEST` stayed `None`-pinned through W15b.
- `cargo test -p pg-core` green: 405 passed / 2 ignored / 0 failed across every suite
  (lib unit tests including 10 new `cloud_ai.rs` tests, and every existing integration
  suite unmodified in behavior). No real vendor host is contacted anywhere in the suite.
  `npm run check` not run — this chunk is core-only (dev-plan W27 "Integrate: Plugin Host;
  no webview HTTP").

Next: W28 — `list_audit_events` (AC-4).

## Related Documentation

- [Development Plan — W27](../dev-plan.md#w27--cloud-ai-plugin-mock-http)
- [Spec — architecture.md §8 Plugin architecture / §9 Cloud AI authentication and network](../specs/architecture.md)
- [Spec — api.md §5.6 Share / §5.7 Cloud AI configuration](../specs/api.md)
- [Spec — data-model.md §5.7 CloudAiSecret / §7 plugin_secret / §8 lifecycle](../specs/data-model.md)
- [Spec — testing.md §6.3 AC-3](../specs/testing.md)
- [Dev log 0038 — W26 overrides](./0038-w26-overrides.md)
