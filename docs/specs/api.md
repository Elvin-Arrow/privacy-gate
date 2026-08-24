# API Specification — Privacy Gate v1

> Scope: the Tauri 2.x command and event surface between the TypeScript frontend and the Rust
> core. This spec names commands, JSON DTOs, errors, session gating, and the API-owned remainders
> of OQ-4 (export filename + PDF metadata fields) and OQ-12 (Cloud AI config commands). It does
> **not** specify UI layout or TS framework (→ [ui.md](./ui.md); decision 0008), crypto/key storage (→ architecture),
> types or on-disk schema (→ [`data-model.md`](./data-model.md); DTOs here are IPC
> projections), or test plans (→ testing spec).
>
> Parent specs: [`srs.md`](./srs.md), [`design.md`](./design.md),
> [`architecture.md`](./architecture.md), [`data-model.md`](./data-model.md). Review roster: [decision 0005](../decisions/0005-review-claude-gemini.md).
>
> Open questions: [`../notes/open-questions.md`](../notes/open-questions.md).

---

## 1. Purpose

The frontend talks to the core **only** through this surface (C-ARCH-2, C-DES-1). Commands are
the product's external API for v1: if a capability is not listed here, the webview cannot do it.

Transport: Tauri 2 IPC. Identifiers are UUID strings. Timestamps are RFC 3339 UTC strings.
`SessionState` and all other enums on the wire are **snake_case strings**:
`first_run` | `locked` | `unlocked` | `degraded_integrity`. Design/architecture PascalCase
names (`Unlocked`, `AwaitingDecisions`, …) map 1:1 to these strings.

**Bytes:** fields typed `bytes` (`filename` is not bytes) are `Vec<u8>` / `Uint8Array`.
Implementations SHALL use Tauri 2's binary IPC mapping for these fields, **not** JSON arrays
of numbers. The logical type is opaque octets. Inbound `import_document` bytes are the
plaintext original entering the TCB (the only inbound original-bytes path). Originals and
`raw_bytes` never **return** to the webview (C-API-3).

---

## 2. Session model

```
SessionState = "first_run" | "locked" | "unlocked" | "degraded_integrity"
```

`get_session_state` is callable in every state (including before first run).

| Command group | `first_run` | `locked` | `unlocked` | `degraded_integrity` |
|---|---|---|---|---|
| `get_session_state` | yes | yes | yes | yes |
| `create_account` | yes | no | no | no |
| `unlock` | no | yes | no (already open) | no (already in degraded) |
| `lock` | no | no | yes | yes |
| `change_passphrase` | no | no | yes | no |
| `get_integrity_report` | no | no | yes | yes |
| `list_audit_events` | no | no | yes | yes (verified prefix only) |
| All document / approval / share / config / cloud-ai / variant / delete | no | no | yes | **no** |
| `get_account` | no | no | yes | yes (id + display_name only) |

`unlock` that detects a crash window fast-forwards and returns `"unlocked"` (architecture §6.3).
`unlock` that detects true integrity failure returns `"degraded_integrity"` plus an integrity
report — it does **not** return `"locked"` and it does **not** decrypt artifacts.

---

## 3. Error model

Every command returns `Result<T, ApiError>`. `ApiError` is:

```
ApiError {
  code: ErrorCode,          // stable string, machine-readable
  message: string,          // non-secret; never includes passphrase, key, field text, document text
}
```

| `ErrorCode` | When |
|---|---|
| `not_in_session` | Command forbidden in the current `SessionState` (§2) |
| `invalid_input` | Schema/validation failure (empty passphrase, bad URL, unknown id format) |
| `not_found` | Unknown `doc_id` / `variant_id` / `preview_token` / `approval_session_id` |
| `unsupported_document` | No extractable text (FR-1.2) |
| `retention_loosen_forbidden` | Per-import retain while global default is `never_retain` (FR-1.4) |
| `retention_policy_unset` | `import_document` before the user has confirmed a retention default (dec 0007) |
| `approval_busy` | Another approval session is already active |
| `approval_bad_state` | Approval command does not match session lifecycle |
| `already_approved` | `open_approval` on a document that already has a canonical `ApprovedVersion` |
| `variant_name_conflict` | `save_variant` name already used on this `doc_id` |
| `not_approved` | Share/preview of a doc with no canonical `ApprovedVersion` |
| `preview_expired` | `preview_token` missing, expired, or `ShareRequest` no longer matches |
| `cloud_ai_not_configured` | Share-to-AI or test without a stored key |
| `cloud_ai_network` | TLS/HTTP failure talking to the allowlisted host (no body, no key in `message`) |
| `cloud_ai_refused` | Endpoint returned 4xx/5xx; `message` is a class (`unauthorized`, `timeout`, `other`) |
| `unlock_failed` | Wrong passphrase |
| `account_exists` | `create_account` when not `"first_run"` |
| `passphrase_mismatch` | `change_passphrase` current passphrase wrong |
| `internal` | Unexpected core failure; `message` is a non-secret class |

Wrong passphrase and unknown account are the same `unlock_failed` (no account-enumeration
beyond `"first_run"` vs `"locked"`, which is visible from `get_session_state`).

---

## 4. Shared DTOs

Logical types, enums, and field lists: [`data-model.md`](./data-model.md). This section is the
**IPC projection** (snake_case JSON, RFC 3339 timestamps, `span.text` visibility). Do not
treat these DTOs as a second schema; they map 1:1 onto the data-model types.

JSON field names are snake_case.

```
RetentionPolicy = "retain" | "discard" | "never_retain"

FieldDecisionKind = "keep_visible" | "redact"

ShareKind = "export_to_person" | "share_to_ai"

EventType =
  | "import" | "detect" | "approve" | "share"
  | "discard_original" | "delete"

DetectedFieldDto {
  id: string,                    // FieldId
  label: string,
  classification: string,
  span: {
    byte_offset: number,         // u64, JSON number (docs ≤ 25 MB; safe in JS)
    byte_length: number,         // octet length of the span in the page/source
    text: string | null,         // unapproved field text — approval commands only
    page_index: number
  },
  parent_field_id: string | null
}

FieldDecisionDto {
  field_id: string,
  decision: FieldDecisionKind
}

ShareRequestDto {
  kind: ShareKind,
  doc_ids: string[],             // order = bundle order (design §3.7)
  per_doc_overrides: { [doc_id: string]: FieldDecisionDto[] },
  applied_variant_ids: { [doc_id: string]: string },
  recipient_note: string | null, // person shares; must be null for AI
  ai_instruction: string | null  // required non-empty when kind = share_to_ai; 1..=4000 chars
}

DocumentSummary {
  doc_id: string,
  source_filename: string,       // encrypted-at-rest catalog metadata; not a filesystem path
  source_format: "text" | "pdf",
  imported_at: string,
  retention: "retain" | "discard",
  has_approved_version: boolean,
  has_retained_original: boolean,
  detected_field_count: number
}
```

`source_filename` is the name supplied at import (basename only, no path separators). The
Vault stores it as envelope-encrypted metadata (FR-4.3). Re-imports are new documents
(design §3.6); the API does not dedupe on filename.

`DetectedFieldDto.span.text` is **omitted** (null / absent) on every command except
`open_approval` / `get_approval_view`. Share preview, audit rows, and document summaries never
include field text.

---

## 5. Commands

Tauri 2 capability ACL allows **only** the commands in this section. No `fs`, `http`, or
`shell` plugins for the frontend (architecture §12). Save-to-disk of an export is **not** a
core filesystem write of plaintext source: the frontend receives redacted PDF **bytes** and
the UI spec owns the user-facing save/download control. That is the hand-off of a share-to-
person file (FR-5.1), not ambient filesystem access.

### 5.1 Session and account

**`get_session_state`**
- In: `{}`
- Out: `{ state: SessionState }`

**`create_account`**
- In: `{ display_name: string, passphrase: string }`
- Out: `{ account_id: string, state: "unlocked" }`
- `display_name` trimmed, 1..=80 chars. `passphrase` min length 8 (API floor; UI spec may
  urge longer). Empty display_name → `invalid_input`.
- Side effect: Key Manager first-run (architecture §3.4). Config initializes retention
  default `discard`, **unconfirmed** (decision 0007). No network.

**`unlock`**
- In: `{ passphrase: string }`
- Out: `{ state: "unlocked" | "degraded_integrity", integrity: IntegrityReport | null }`
- `integrity` is non-null iff `degraded_integrity`.

**`lock`**
- In: `{}`
- Out: `{ state: "locked" }`
- Zeroizes session material (architecture §3.3). Invalidates approval sessions and preview
  tokens.

**`change_passphrase`**
- In: `{ current: string, new_passphrase: string }`
- Out: `{ ok: true }`
- Re-wraps the same master key (architecture §3.3). No recovery command exists (C-ARCH-7).

**`get_account`**
- In: `{}`
- Out: `{ account_id: string, display_name: string, created_at: string }`

**`get_integrity_report`**
- In: `{}`
- Out: `IntegrityReport`

```
IntegrityReport {
  ok: boolean,                   // true after crash-window fast-forward or clean unlock
  kind: "ok" | "crash_window_fast_forwarded" | "truncation" | "modification",
  head_sequence: number,
  tail_sequence: number,
  first_bad_sequence: number | null
}
```

### 5.2 Config

**`get_retention_default`**
- Out: `{ policy: RetentionPolicy, confirmed: boolean }`
- Factory: `policy` is `"discard"`, `confirmed` is `false` (decision 0007 / OQ-14).

**`set_retention_default`**
- In: `{ policy: RetentionPolicy }`
- Out: `{ policy: RetentionPolicy, confirmed: true }`
- Sets the global default and marks it **confirmed**. First successful call is what unblocks
  `import_document`.
- Changing the **global** default from `never_retain` to `retain` is allowed (it is not a
  per-import override). Per-import loosening is rejected on `import_document`.

**`get_detector_preference`** (decision 0009)
- Out: `{ preference: "auto" | "bundled_only" }`
- Factory: `"auto"`.

**`set_detector_preference`** (decision 0009)
- In: `{ preference: "auto" | "bundled_only" }`
- Out: `{ preference: "auto" | "bundled_only" }`
- `"auto"` prefers the optional Ollama backend when a loopback Ollama with an allowlisted
  model is verified reachable (architecture §10.1); otherwise falls back to `pg-hybrid-v1`.
  `"bundled_only"` always uses `pg-hybrid-v1`, no Ollama probe ever performed.

### 5.3 Import and catalog

**`import_document`**
- In: `{
    filename: string,            // basename; path separators rejected (`invalid_input`)
    bytes: bytes,                // binary IPC; inbound original only
    retention_override: "retain" | "discard" | null  // null = use default
  }`
- Out: `{
    summary: DocumentSummary,
    over_budget: boolean         // true if size > design §7 25 MB interactive budget
  }`
- Runs import + in-process detection as a Tauri **async** command. CPU-bound work runs on a
  blocking pool so `pg://detect-progress` can flush to the webview. Documents over the design
  §7 25 MB interactive budget still run to completion; `over_budget` is true so the UI spec
  can warn. The command does not reject over-budget inputs.
- Rejects empty bytes, rejected formats, and path-like filenames.
- If `get_retention_default.confirmed` is false → `retention_policy_unset` (no detection, no
  catalog row). The UI spec owns the first-upload prompt; this command does not accept a
  policy inline — call `set_retention_default` first.
- `never_retain` default + `retention_override: "retain"` → `retention_loosen_forbidden`.
- Detection identity recorded in the audit `detect` event is `pg-hybrid-v1` or, when the
  optional Ollama backend was selected and verified, `pg-hybrid-ollama-v1` with its
  `model_tag` (architecture §10.1, decision 0009) — never a productized "Gemma" string on its
  own. A `fallback_reason` is recorded when `detector_preference` is `"auto"` but the Ollama
  path was not used.

**`list_documents`**
- Out: `{ documents: DocumentSummary[] }`  // newest import first

**`get_document`**
- In: `{ doc_id: string }`
- Out: `{ summary: DocumentSummary }`
- Does **not** return pages, field text, or approved content. Use `open_approval` or share
  preview for content.

**`delete_document`**
- In: `{ doc_id: string }`
- Out: `{ ok: true }`
- Irrevocable: approved version, retained original (if any), and variants (FR-4.6). Audit
  `delete`.

**`delete_retained_original`**
- In: `{ doc_id: string }`
- Out: `{ summary: DocumentSummary }`
- Idempotent if already discarded. Audit `discard_original` if an original was present.

### 5.4 Approval

One active approval session per process (design §2.3). Re-approval of a document that already
has a canonical `ApprovedVersion` is **out of v1**: later shares use ephemeral overrides or
variants (FR-5.4/5.5). `open_approval` on such a document returns `already_approved`.

**`open_approval`**
- In: `{ doc_id: string }`
- Out: `ApprovalView`
- Errors: `already_approved`, `approval_busy`, `not_found`.

A document with `has_approved_version == false` (including after `abort_approval`) may be
opened. If retention was `discard` and the in-memory original was dropped on abort/lock, the
catalog row is removed (`abort_approval` / `lock` rules below) so `open_approval` cannot
target a body-less document.

**`open_approval`**
- In: `{ doc_id: string }`
- Out: `ApprovalView`

```
ApprovalView {
  approval_session_id: string,
  doc_id: string,
  lifecycle: "awaiting_decisions",
  pages: { page_index: number, spans: { byte_offset: number, text: string, page_index: number }[] }[],
  fields: DetectedFieldDto[]     // includes span.text
}
```

This is the C-DES-1 exception: unapproved content for the consent step. Core must not serve
this payload after `submit_approval` or `abort_approval` (architecture §5.2).

**`get_approval_view`**
- In: `{ approval_session_id: string }`
- Out: `ApprovalView` (same as open; `lifecycle` may be `awaiting_decisions` | `decided`)

**`set_field_decisions`**
- In: `{ approval_session_id: string, decisions: FieldDecisionDto[] }`
- Out: `{ lifecycle: "awaiting_decisions" | "decided", unresolved_field_ids: string[] }`
- All detected fields must have a decision before submit. Partial updates are allowed.
  `lifecycle` is `"decided"` **iff** `unresolved_field_ids` is empty; otherwise
  `"awaiting_decisions"`. Wrong session lifecycle → `approval_bad_state`.

**`submit_approval`**
- In: `{ approval_session_id: string }`
- Out: `{ summary: DocumentSummary, lifecycle: "committed" }`
- Requires `lifecycle == "decided"`. Overlap rule is core-side (design §3.5). Writes canonical
  `ApprovedVersion` including `redacted_content`. If retention is discard, original destruction
  happens here (design §2.1). Otherwise `approval_bad_state`.

**`abort_approval`**
- In: `{ approval_session_id: string }`
- Out: `{ lifecycle: "aborted" }`
- No stored approved version.
  - Retention `retain`: catalog row and encrypted original remain; user may `open_approval` again.
  - Retention `discard`: in-memory original is zeroized and the catalog row is deleted (there is
    no persisted body to approve later).

### 5.5 Variants

**`list_variants`**
- In: `{ doc_id: string }`
- Out: `{ variants: { variant_id: string, name: string, created_at: string }[] }`

**`get_variant`**
- In: `{ doc_id: string, variant_id: string }`
- Out: `{ variant_id: string, name: string, created_at: string, overrides: FieldDecisionDto[] }`
- Overrides are field_id + decision only (no span text). The unlocked owner already made
  these decisions; this is not a C-API-2 violation.

**`save_variant`**
- In: `{ doc_id: string, name: string, overrides: FieldDecisionDto[] }`
- Out: `{ variant_id: string, name: string, created_at: string }`
- Name 1..=80 chars, unique per doc. Duplicate → `variant_name_conflict`. Edit is not
  supported (design §3.4): save a new one.

**`delete_variant`**
- In: `{ doc_id: string, variant_id: string }`
- Out: `{ ok: true }`

### 5.6 Share: preview token, export, Cloud AI

FR-6.1 requires the preview to be **exactly** what will leave. The API uses a preview token:

1. `preview_share(ShareRequestDto)` materialises the redacted artifact in core memory and
   returns a token plus a view the frontend may show.
2. `commit_share(preview_token)` emits the audit event and returns the **same bytes** (export)
   or sends the **same approved text** (AI). Changing the request requires a new preview.

Tokens expire on `lock`, after 10 minutes, or when a newer `preview_share` replaces them
(one live token per process in v1). `SharePreview.expires_at` is the absolute expiry.
Expired token → `preview_expired`. A second `preview_share` invalidates the previous token
before returning the new one.

**`preview_share`**
- In: `{ request: ShareRequestDto }`
- Out: `SharePreview`

```
SharePreview {
  preview_token: string,
  expires_at: string,                     // RFC 3339
  kind: ShareKind,
  overrides_in_effect: boolean,           // FR-6.2 flag; wording is UI spec
  suggested_filename: string | null,      // non-null for export_to_person
  pdf_bytes: bytes | null,                // redacted PDF; export kind only
  ai_payload_preview: string | null,      // approved text that will be sent; AI kind only
  manifest: {
    doc_id: string,
    visible_field_ids: string[],
    redacted_field_ids: string[]
  }[],
  no_originals_left_device: boolean[]     // parallel to doc_ids; design §2.6 condition
}
```

- Export kind: `pdf_bytes` is the newly rendered PDF (architecture §11). No redacted field
  text. `ai_payload_preview` is null. `ai_instruction` must be null.
- AI kind: `ai_instruction` required. `ai_payload_preview` is the exact approved document
  text (overrides/variants already applied) that will be POSTed, **not** including the
  instruction wrapper the plugin adds around it. `pdf_bytes` is null. Requires Cloud AI
  configured, else `cloud_ai_not_configured` (fail **before** assembling a send).
  Empty instruction → `invalid_input`.
- Empty `doc_ids` or unknown ids → `invalid_input` / `not_found`. Docs without approved
  version → `not_approved`.

**`commit_share`**
- In: `{ preview_token: string }`
- Out:
  - export: `{ kind: "export_to_person", pdf_bytes: bytes, suggested_filename: string, audit_event_id: number }`
  - AI: `{ kind: "share_to_ai", output_text: string, audit_event_id: number }`
- Export `pdf_bytes` must be byte-identical to the preview's `pdf_bytes`.
- AI: the approved-document body POSTed must be **identical** to `ai_payload_preview`. The
  plugin may wrap that body with `ai_instruction` and a fixed system preamble; the preamble
  is first-party, contains no vault secrets, and is not shown in the preview (the preview is
  "exactly what **content** will leave," FR-5.3 / FR-6.1). HTTP is Rust-side (architecture
  §9). `output_text` is read-only model output (FR-5.2). Failed HTTP → `cloud_ai_network` /
  `cloud_ai_refused`; a share audit event still records the attempt and error class
  (architecture §9.3). Audit payload does not store `ai_instruction` text
  (`has_ai_instruction: true` only).
- Drops the token after success or definitive failure.

### 5.7 Cloud AI configuration (resolves OQ-12 command shape)

**`cloud_ai_set_config`**
- In: `{ endpoint_url: string, model: string, api_key: string }`
- Out: `{ configured: true, endpoint_host: string, model: string, key_last4: string }`
- `endpoint_url` must be `https://` with a host; `file://`, `http://`, and userinfo in the URL
  are `invalid_input`. Redirects that change host are a runtime refuse (architecture §9.2).
- The frontend may hold `api_key` only until this command returns. Subsequent gets never
  include it (architecture §9.1).

**`cloud_ai_get_config`**
- Out: `{ configured: boolean, endpoint_url: string | null, endpoint_host: string | null, model: string | null, key_last4: string | null }`
- Never `api_key`.

**`cloud_ai_clear_config`**
- Out: `{ configured: false }`
- Cryptographic erase of the plugin-secret DEK (architecture §4.3).

**`cloud_ai_test`**
- Out: `{ ok: boolean, error_class: string | null }`
- Sends **no** vault document content. The handshake used to prove the credential works is
  an implementation choice (must not attach vault documents).

### 5.8 Audit

**`list_audit_events`**
- In: `{
    doc_id: string | null,
    event_type: EventType | null,
    after_sequence: number | null,   // cursor; exclusive
    limit: number                    // 1..=200, default 50
  }`
- Out: `{ events: AuditEventDto[], next_sequence: number | null }`

```
AuditEventDto {
  sequence: number,
  event_type: EventType,
  doc_id: string | null,
  produced_at: string,
  no_originals_left_device: boolean | null,
  payload: object                    // see below; never field text, never keys, never API keys
}
```

Payload shapes (informative; all ids/labels, no span text):

- `import`: `{ retention: "retain" | "discard", source_filename: string, detector_id: null }`
- `detect`: `{ detector_id: "pg-hybrid-v1", field_ids: string[], labels: string[] }`
- `approve`: `{ decisions: { field_id: string, label: string, decision: FieldDecisionKind }[] }`
- `share`: `{ kind: ShareKind, recipient_note: string | null, endpoint_host: string | null, doc_ids: string[], error_class: string | null, has_ai_instruction: boolean }`
- `discard_original` / `delete`: `{ doc_id: string }`

`entry_signature` and `prev_entry_hash` are **not** on the DTO (webview does not verify the
chain; the core already did at unlock). Integrity mismatch is `IntegrityReport`, not raw MACs.

In `"degraded_integrity"`, `list_audit_events` returns only the verified prefix (sequences
`< first_bad_sequence`) and does not decrypt document artifacts.

---

## 6. Events

The core may emit:

| Event | Payload | Notes |
|---|---|---|
| `pg://detect-progress` | `{ doc_id: string, fraction: number, phase: "detecting" \| "warming_model" }` | `fraction` 0..1; `phase` additive (decision 0009) — `"warming_model"` during Ollama cold-start (architecture §10.1.5, budget ≤ 20 s), `"detecting"` otherwise; UI spec for display |
| `pg://session-changed` | `{ state: SessionState }` | After lock/unlock/create/degraded |

No event contains field text, keys, or passphrases.

---

## 7. Export filename and PDF metadata (resolves OQ-4 remainder — API part)

Design already fixed format (PDF) and bundle order (user selection order). This spec fixes
**what the API returns** and **what is embedded in the PDF**. How the save dialog is worded
and laid out is UI spec.

### 7.1 Suggested filename
- Sanitize `source_filename` stem: Unicode letters/digits, then `-`; collapse; max 40 chars;
  if empty, use `document`.
- Single document: `{stem}-redacted-{YYYYMMDD}.pdf` (UTC date of commit/preview).
- Multiple documents: `privacy-gate-{n}docs-redacted-{YYYYMMDD}.pdf`.
- No original path, no account display name, no detector labels in the filename.

### 7.2 PDF info dictionary
Written by the re-renderer (architecture §11):

| Field | Value |
|---|---|
| Title | suggested filename without `.pdf` |
| Producer | `Privacy Gate` |
| Creator | `Privacy Gate` |
| CreationDate / ModDate | export timestamp |
| Author | **omitted** |
| Subject / Keywords / custom | **omitted / empty** |

No redacted text, original filename path, account id, or API endpoint in PDF metadata.

---

## 8. Tauri capability allowlist

The production capability file shall grant the webview:

- `core:event:allow-listen` for `pg://detect-progress`, `pg://session-changed`
- `core:default` as required by Tauri 2
- Each command in §5 by name

and shall **deny** filesystem read, HTTP, shell, and opener-with-arbitrary-URL. The UI spec may
additionally allow a **save dialog** that persists in-memory bytes the core already returned
(previewed `pdf_bytes`, or `get_integrity_report` JSON); that dialog must not open arbitrary
files into the webview ([ui.md §10.4](./ui.md)).

---

## 9. Constraints

- **C-API-1** Passphrase and `api_key` appear only as command **inputs** on
  `create_account` / `unlock` / `change_passphrase` / `cloud_ai_set_config`. They never appear
  in outputs, events, or audit DTOs.
- **C-API-2** Redacted field text never crosses IPC. Unapproved span text crosses only on
  `open_approval` / `get_approval_view`.
- **C-API-3** Export PDF bytes may cross IPC outbound (they are the share artifact). Original
  bytes cross IPC **inbound only** on `import_document`. Originals and `raw_bytes` never
  return to the webview.
- **C-API-4** `commit_share` is the only command that causes Cloud AI HTTP or a person-share
  audit success path. `preview_share` and `cloud_ai_test` do not send vault documents to the
  network.
- **C-API-5** No command exposes keystore material, DEKs, HMAC bytes, or SQLCipher keys.
- **C-API-6** Degraded session cannot import, approve, share, or read document content.

---

## 10. Traceability

| SRS / design / arch | API coverage |
|---|---|
| FR-1.1..1.5 import + retention + audit | `import_document`, config, audit `import`; `retention_policy_unset` |
| FR-2.1..2.3 detection on-device | `import_document` (core-side detect); no detect command that takes a URL |
| FR-3.1..3.4 approval | `open_approval`, `set_field_decisions`, `submit_approval` |
| FR-4.6 delete | `delete_document`, `delete_retained_original` |
| FR-5.1 export | `preview_share` + `commit_share` (export kind) |
| FR-5.2 Cloud AI | `preview_share` + `commit_share` (AI kind); §5.7 |
| FR-5.3..5.5 overrides / variants | `ShareRequestDto`, `list_variants` / `get_variant` / `save_variant` / `delete_variant` |
| FR-5.6 / FR-7.* audit | `list_audit_events`; commit emits share |
| FR-6.1..6.2 preview | preview token; `overrides_in_effect` |
| FR-8.1..8.3 account | §5.1 |
| OQ-4 remainder | §7 (filename + PDF metadata); save-dialog chrome → [ui.md](./ui.md) §10.4 |
| OQ-12 command shape | §5.7 |
| C-DES-1 / C-ARCH-1..2 | §2, §8, C-API-1..3 |
| Architecture §6.3 degraded | §2, `get_integrity_report` |
| Design §3.7 / data-model `ShareRequest` | `ShareRequestDto` |

---

## 11. Open questions owned here

- **OQ-4 (API part)** Suggested filename algorithm and PDF info-dictionary fields → §7.
  Save-dialog chrome → [ui.md §10.4](./ui.md).
- **OQ-12 (API part)** Cloud AI set/get/clear/test commands; key never in outputs → §5.7.
- **OQ-14 (API part)** Factory `discard`, `confirmed` flag, `retention_policy_unset` on
  import → §5.2 / §5.3 (decision 0007).

## 12. Deferred

- TS visual design system, i18n, WCAG certification → later UI work (v1 copy and keyboard
  approval are in [ui.md](./ui.md)).
- Exact handshake used by `cloud_ai_test` → implementation choice behind `{ ok, error_class }`;
  must not send documents.
- Vault backup / restore commands and reinstall re-attachment → later phase (idea.md); v1
  has no such commands.

---

## 13. Related Decisions

- [0002](../decisions/0002-resolved-srs-clarifications.md) — true-removal export; one canonical
  version; PDF bundle.
- [0003](../decisions/0003-v1-tech-stack.md) — Tauri 2 IPC.
- [0004](../decisions/0004-v1-architecture.md) — no key to frontend; HTTP in Rust; degraded
  unlock.
- [0005](../decisions/0005-review-claude-gemini.md) — Claude + Gemini review.
- [0006](../decisions/0006-tdd-and-mutation-testing.md) — TDD + mutation.
- [0007](../decisions/0007-retention-default-discard.md) — factory discard; first-import confirm.
- [0008](../decisions/0008-frontend-svelte.md) — Svelte 5 webview.

## 14. Related Work

- [0001-srs-generation](../dev-log/0001-srs-generation.md)
- [0002-design-spec](../dev-log/0002-design-spec.md)
- [0003-architecture-spec](../dev-log/0003-architecture-spec.md)
- [0004-api-spec](../dev-log/0004-api-spec.md)
- [0005-testing-spec](../dev-log/0005-testing-spec.md)
- [0007-data-model-spec](../dev-log/0007-data-model-spec.md)
- [0008-ui-spec](../dev-log/0008-ui-spec.md)
- [Spec — SRS](./srs.md)
- [Spec — design](./design.md)
- [Spec — architecture](./architecture.md)
- [Spec — testing](./testing.md)
- [Spec — data model](./data-model.md)
- [Spec — UI](./ui.md)
