# API Specification Review: Privacy Gate v1

Reviewer: Gemini (via `agy --effort high`). Date: 2026-08-23.

(Raw Gemini output from the shared review prompt.)

## A. Alignment API → SRS

* **Missing functional requirements (FR-5.2):** `ShareRequestDto` (§4) provides no field for a user prompt, query, or task instructions when executing a Cloud AI share. `recipient_note` is explicitly marked as "ignored for AI" (§4). Without an instruction/prompt payload, the core cannot construct a valid prompt/completion payload for the remote AI model.
* **Span representation gap (FR-2.2, FR-3.1):** `DetectedFieldDto.span` (§4) provides `byte_offset: number`, `text: string`, and `page_index: number`, but omits `byte_length` or `end_offset`. When `span.text` is stripped/null (required on all non-approval commands per C-API-2), the frontend has no length metadata to map, highlight, or count span boundaries.
* **Synchronous import vs detection progress events (FR-1.1, FR-2.1):** `import_document` (§5.3) is specified as synchronous while simultaneously emitting `pg://detect-progress` (§6). In Tauri 2 async command handlers, a purely synchronous CPU-bound pipeline on the IPC thread can delay event delivery to the webview unless offloaded to a worker thread.
* **Legitimate added behavior vs scope creep:**
  * Added `over_budget: boolean` on `import_document` (§5.3): Legitimate operational signal aligning with design §7 performance budgets.
  * Added `suggested_filename` sanitization and PDF info-dictionary stripping (§7): Legitimate resolution of the API-owned remainder of OQ-4.
  * Added `key_last4` on `cloud_ai_get_config` / `cloud_ai_set_config` (§5.7): Legitimate UX convenience that avoids leaking the secret key (C-API-1).
  * Added `no_originals_left_device` array in `SharePreview` (§5.6) and `AuditEventDto` (§5.8): Legitimate transport of the design §2.6 audit verification flag.

## B. Alignment API → design

* **Approval lifecycle violation (§5.4 vs design §2.3, §3.5):** `set_field_decisions` returns `{ lifecycle: "decided", unresolved_field_ids: string[] }`. If `unresolved_field_ids` is non-empty, transitioning or reporting the state as `"decided"` violates the approval lifecycle state machine; the session is still `"awaiting_decisions"`.
* **Variant inspectability gap (§5.5 vs design §3.4):** `list_variants` returns only `{ variant_id, name, created_at }`, and no `get_variant` command exists. Omitting override definitions prevents the UI from displaying what decisions a variant contains prior to generating a preview.
* **Overlap handling in core (design §3.5):** Correctly maintained.
* **One active approval session & Re-import semantics (design §2.3, §3.6):** Fully aligned.

## C. Alignment API → architecture

* Webview isolation, key non-exposure, degraded integrity gating, Cloud AI network boundary, and span text isolation: **obeyed**.

## D. Alignment API ↔ idea

* Local-first, on-device key/detection, vault-as-product, export-only share-to-person: **aligned**.

## E. Quality / implementability

* Missing AI prompt parameter in `ShareRequestDto`.
* Span offset ambiguity — need `byte_length`.
* JSON `number[]` for 25 MB files is an IPC hazard; prescribe binary payloads.
* `set_field_decisions` lifecycle state ambiguity.
* Enum serialization PascalCase vs snake_case.
* Duplicate variant name lacks a dedicated error code.

## F–G. Scope / deferrals

* Clean. OQ-4/12 resolved; OQ-14/OQ-6 remainder healthy.

## H. Top 5 changes

1. Add `ai_prompt` to `ShareRequestDto`.
2. Add `byte_length` to `DetectedFieldDto.span`.
3. Fix `set_field_decisions` return lifecycle state.
4. Add `get_variant`.
5. Formally specify binary transport for file payloads.
