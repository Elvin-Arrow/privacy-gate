# Data Model Specification — Privacy Gate v1

> Scope: the v1 **data model** — every named type, identifier, relationship, in-memory
> structure, SQLCipher schema, envelope-artifact plaintext, audit row, and OS-keystore item.
> This is the **single source** for those facts. Other specs use these names; they do not
> restate field lists.
>
> It does **not** specify crypto algorithms or HMAC byte encoding (→ [`architecture.md`](./architecture.md)),
> component flows or overlap/re-import *rules* (→ [`design.md`](./design.md)), Tauri commands
> or IPC DTOs (→ [`api.md`](./api.md); DTOs are projections of types here), UI layout, or a
> sync protocol. Future cross-device sync is a later phase; this model keeps **per-artifact
> envelopes** as the unit a sync phase can move, and does **not** introduce a server database.
>
> Parent specs: [`srs.md`](./srs.md), [`design.md`](./design.md),
> [`architecture.md`](./architecture.md), [`api.md`](./api.md). Review roster:
> [decision 0005](../decisions/0005-review-claude-gemini.md).
>
> Open questions: [`../notes/open-questions.md`](../notes/open-questions.md).

---

## 1. Purpose

One local vault, one SQLCipher file, envelope-encrypted artifacts inside it. Logical types
(§5) are what components hold and pass. Persistence (§6–§7) is how those types sit on disk.
The relational catalog is for joins and lifecycle flags; anything an attacker would want from
a stolen file is either SQLCipher-protected (whole DB) **and**, for document/config/secret
payloads, **DEK-wrapped** (architecture §3.1, FR-4.3, NFR-S1).

API DTOs are projections of these types (snake_case JSON, RFC 3339 timestamps, field-text
visibility rules). They are not a second schema.

---

## 2. Identifiers and conventions

| Kind | Type | Notes |
|---|---|---|
| `AccountId`, `DocId`, `FieldId`, `VariantId`, `ArtifactId` | UUID v4 | Canonical text: lowercase hex with hyphens (`8-4-4-4-12`). API strings are these. |
| `sequence` | `u64` | Audit append index, starts at 1. |
| `Timestamp` (logical) | instant | On disk: `INTEGER` unix milliseconds UTC. On the API: RFC 3339. |
| Enums (logical) | PascalCase in this spec | Wire / envelope JSON: snake_case ([`api.md`](./api.md)). SQL `event_type` / `kind`: INTEGER as specified. |
| `schema_version` | `INTEGER` | **1** for this spec. Bump is this spec + architecture amendment. |

No natural key on file bytes. Re-import → new `DocId` (design §3.6). Do **not** unique-index
a content hash (that would silently merge documents).

### 2.1 Canonical enums

| Logical | Wire / envelope JSON |
|---|---|
| `RetentionPolicy`: Retain, Discard, NeverRetain | `"retain"` \| `"discard"` \| `"never_retain"` |
| `FieldDecisionKind`: KeepVisible, Redact | `"keep_visible"` \| `"redact"` |
| `ShareKind`: ExportToPerson, ShareToAI | `"export_to_person"` \| `"share_to_ai"` |
| `SourceFormat`: Text, Pdf | `"text"` \| `"pdf"` |
| `EventType`: Import, Detect, Approve, Share, DiscardOriginal, Delete | `"import"` … `"delete"`; SQL / HMAC `u8`: 1 Import, 2 Detect, 3 Approve, 4 Share, 5 DiscardOriginal, 6 Delete |
| `ApprovalLifecycle`: AwaitingDecisions, Decided, Committed, Aborted | `"awaiting_decisions"` \| `"decided"` \| `"committed"` \| `"aborted"` |
| `ArtifactKind` | INTEGER 1..8 (§6) |

Per-document retention stored on `DocumentMeta` is only Retain or Discard. Global
`NeverRetain` is not stored on the document (see §5.5, §6.1).

---

## 3. Stores (where bytes live)

```
OS keystore (or Linux 0600 fallback)     SQLCipher vault.db (app-data)
┌─────────────────────────────┐          ┌──────────────────────────────┐
│ KeystoreItem                │          │ schema_meta, account         │
│  wrapped vault_master_key   │          │ document, variant            │
│  Argon2id params, audit_head│          │ artifact (DEK-wrapped blobs) │
└─────────────────────────────┘          │ plugin_secret pointer        │
                                         │ audit_entry                  │
                                         └──────────────────────────────┘
Process memory only (not a table)
  approval session, share preview token, discard-unapproved raw_bytes + pages
```

- **Not persisted:** approval sessions, preview tokens, `ShareRequest` overrides, discard-
  unapproved `Document.raw_bytes` / page IR (api.md lock/abort).
- **SQLCipher-only (no per-row DEK):** `LocalAccount.display_name` (architecture: not a secret),
  audit rows (architecture §6: encrypted at rest *inside SQLCipher*; HMAC is separate),
  integer/UUID catalog keys used for joins.
- **Envelope (per-artifact DEK):** original, approved version, variant, config, plugin
  secret, **document_meta** (§6). Wrapped DEKs sit in `artifact.wrapped_dek`.

A stolen `vault.db` without `sqlcipher_key` yields neither catalog plaintext nor blobs
(FR-4.4). Envelope DEKs additionally make delete = destroy wrap (NFR-R2).

---

## 4. Entity-relationship (logical)

```
LocalAccount (1)
    │
    ├── Config (1)                    ── artifact kind=4
    ├── PluginSecret (0..1 Cloud AI)  ── artifact kind=5
    └── Document catalog (0..*)       ── SQL document + artifact kind=8
            ├── Original (0..1)       ── artifact kind=2  (retention retain)
            ├── ApprovedVersion (0..1)── artifact kind=1  (after submit)
            └── Variant (0..*)        ── artifact kind=3

AuditEntry (append-only, 0..*)  ── may reference DocId
KeystoreItem (1, not in SQL)
```

Cardinalities: one account per vault file in v1. One canonical `ApprovedVersion` per
document (dec 0002 / Q8). Variants cannot exist without an approved version.

`Document` (§5.2) is the in-memory import/detect IR, not the SQL catalog row.

---

## 5. Logical types

These are the types components hold and pass. Persistence mapping is §6–§7. IPC mapping is
[`api.md`](./api.md). Crypto of envelopes and HMAC encoding of audit rows is
[`architecture.md`](./architecture.md).

### 5.1 `TextSpan`, `DetectedField`, `Page`, `Document`

In-memory intermediate representation (Importer → Detector → Approval Engine).

```
TextSpan {
  byte_offset: u64,          // offset into raw_bytes / page
  byte_length: u64,          // octet length of the span
  text: String,
  page_index: u32,
}

DetectedField {
  id: FieldId,
  label: String,             // classifier label
  classification: String,
  span: TextSpan,
  parent_field_id: Option<FieldId>,  // design §3.5 overlap/nesting
}

Page { spans: Vec<TextSpan> }

Document {
  id: DocId,
  source_format: SourceFormat,
  pages: Vec<Page>,
  raw_bytes: Vec<u8>         // process memory during import;
                             //   handed to Vault iff retention = Retain;
                             //   overwritten by Importer on Vault ack (design §2.1)
}
```

### 5.2 `FieldDecision`, `RedactedDocument`, `ApprovedVersion`

```
FieldDecision {
  field: DetectedField,
  decision: FieldDecisionKind,
}

RedactedDocument {
  // Document with redacted spans truly removed (NFR-S4 / dec 0002 / Q11),
  // in a form the Share Engine can export or hand to the Cloud AI plugin
  // WITHOUT needing Document.raw_bytes.
  format: SourceFormat,
  pages: Vec<Page>,          // redacted pages
}

ApprovedVersion {
  doc_id: DocId,
  decisions: Vec<FieldDecision>,
  redacted_content: RedactedDocument,
  produced_at: Timestamp,
}
```

One canonical `ApprovedVersion` per `DocId` (FR-3.2, dec 0002 / Q8). Variants (§5.3) are not
separate approved versions. `redacted_content` is produced by the Approval Engine at commit
and stored encrypted by the Vault.

### 5.3 `Variant`

```
Variant {
  id: VariantId,
  doc_id: DocId,             // scoped to a single document
  name: String,              // user-chosen; 1..=80 chars; unique per doc_id
  overrides: Vec<FieldDecision>,  // relative to the canonical ApprovedVersion
  created_at: Timestamp,
}
```

Lifecycle (create / apply / delete; no edit in v1) is design §3.4. On disk, overrides are
stored as `{ field_id, decision }` deltas against the approved snapshot (§6.4), not a second
copy of every `DetectedField`.

### 5.4 `ShareRequest`

Ephemeral. No table.

```
ShareRequest {
  kind: ShareKind,
  doc_ids: Vec<DocId>,       // order = bundle order (design §3.7)
  per_doc_overrides: Map<DocId, Vec<FieldDecision>>,  // ad-hoc; this share only
  applied_variant_ids: Map<DocId, VariantId>,
  recipient_note: Option<String>,  // audit (FR-7.2); person shares
  ai_instruction: Option<String>,  // required for ShareToAI; length → api.md
}
```

Export format and bundle ordering: design §3.7. Filename / PDF info dictionary: api.md §7.

### 5.5 `Config`

```
Config {
  policy: RetentionPolicy,
  confirmed: bool,
  detector_preference: "auto" | "bundled_only",  // decision 0009
}
```

Factory after `create_account`: `policy = Discard`, `confirmed = false` (decision 0007),
`detector_preference = "auto"` (decision 0009). Global `NeverRetain` is stored here only.
Import under `NeverRetain` writes document meta `retention = Discard`. Per-import retain
against `NeverRetain` is `retention_loosen_forbidden` and produces no rows.

`detector_preference = "auto"` prefers the optional `pg-hybrid-ollama-v1` backend when a
loopback Ollama with an allowlisted model is verified reachable (architecture §10.1),
falling back to `pg-hybrid-v1` otherwise. `"bundled_only"` always uses `pg-hybrid-v1`, no
Ollama probe.

### 5.6 `LocalAccount`

```
LocalAccount {
  id: AccountId,             // UUID, generated on device
  display_name: String,      // user-chosen; not a secret; 1..=80 trimmed (api.md)
  created_at: Timestamp,
}
```

v1 accounts are local-only (architecture §7, OQ-5). `display_name` is SQLCipher-only, not
envelope-encrypted (architecture: not a secret).

### 5.7 `CloudAiSecret`

```
CloudAiSecret {
  endpoint_url: String,
  model: String,
  api_key: String,
  key_last4: String,         // not a secret; for get-config without returning the key
}
```

Absence = not configured. HTTP and allowlist: architecture §9. Commands: api.md §5.7.

### 5.8 `AuditEntry` and event payloads

```
AuditEntry {
  sequence: u64,
  event_type: EventType,
  doc_id: Option<DocId>,
  payload: EventPayload,     // §5.8.1; no span text, no keys
  no_originals_left_device: Option<bool>,  // Share events only; else None
  produced_at: Timestamp,
  prev_entry_hash: [u8; 32],
  entry_signature: [u8; 32], // HMAC-SHA-256; byte layout → architecture §6.1
}
```

Integrity fields make the Audit Trail implementable against architecture §6. This spec owns
the row contents; architecture owns the canonical encoding used for HMAC / `prev_entry_hash`.

On disk, `no_originals_left_device` is `originals_flag`: `0` unset, `1` false, `2` true.
Share events use only `1` or `2`. The column exists because HMAC encoding is not the JSON
payload (architecture §6.1). Same fact as the share payload boolean.

#### 5.8.1 `EventPayload` (RFC 8785 JCS on disk)

| `event_type` | payload keys |
|---|---|
| Import | `retention`, `source_filename`, `detector_id` (null on import row) |
| Detect | `detector_id` (`pg-hybrid-v1` \| `pg-hybrid-ollama-v1`), `field_ids`, `labels`, `backend` (`"onnx"` \| `"ollama"`), `model_tag` (string \| null, e.g. `"gemma4:e2b"`), `fallback_reason` (string \| null — decision 0009) |
| Approve | `decisions` [{`field_id`,`label`,`decision`}] |
| Share | `kind`, `recipient_note`, `endpoint_host`, `doc_ids`, `error_class`, `has_ai_instruction` |
| DiscardOriginal | `doc_id` |
| Delete | `doc_id` |

These match api.md §5.8 objects. No span text, no credentials.

### 5.9 `KeystoreItem`

Not SQL. OS keystore (or Linux 0600 fallback — architecture §3.2).

```
Argon2idParams {
  m_cost: u32,
  t_cost: u32,
  p_cost: u32,
  salt: Blob,
}

AuditHead {
  sequence: u64,
  head_hash: [u8; 32],       // latest persisted accepted entry
}

KeystoreItem {
  account_id: AccountId,     // matches LocalAccount.id
  kdf: Argon2idParams,
  wrapped_master_key: Blob,  // AEAD(wrap_key, vault_master_key); AAD kind 6
  audit_head: AuditHead,
}
```

Wrap algorithm, Argon2id floors, Linux fallback threat model: architecture §3. Anti-truncation
head update cadence: architecture §6.2. Passphrase is never a field here.

### 5.10 Approval session (RAM)

One active session per process (design §2.3). Types the core holds; `ApprovalView` on the
wire is api.md.

```
ApprovalSession {
  approval_session_id: String,   // UUID
  doc_id: DocId,
  lifecycle: ApprovalLifecycle,  // AwaitingDecisions → Decided → Committed | Aborted
  document: Document,            // pages + fields for C-DES-1
  decisions: Map<FieldId, FieldDecisionKind>,
}
```

Until submit, abort, or lock. After abort/lock with discard-unapproved, the catalog row and
kind-8 artifact are deleted (§8).

---

## 6. Envelope artifacts

On-disk wrap is architecture §3.1 (XChaCha20-Poly1305, AAD v1, wrapped DEK). This spec owns
**plaintext JSON inside the ciphertext** (`format_version` = 1) and **kind codes**. Architecture
AAD `artifact_kind` is this table; do not fork a second kind list.

| `kind` | Name | `doc_id` in AAD | Plaintext (JSON object, UTF-8, RFC 8785 JCS) |
|---|---|---|---|
| 1 | `approved` | yes | `ApprovedVersion` encoding (§6.3) |
| 2 | `original` | yes | `OriginalRecord` (§6.2) |
| 3 | `variant` | yes | `Variant` encoding (§6.4) |
| 4 | `config` | empty | `Config` |
| 5 | `plugin_secret` | empty | `CloudAiSecret` (v1 only this plugin) |
| 6 | `wrapped_master` | empty | **Not in SQL** — OS keystore |
| 7 | `wrapped_dek` | as wrapped artifact | AAD kind for the DEK wrap blob, not a SQL row kind |
| 8 | `document_meta` | yes | `DocumentMeta` (§6.1) |

Kind **8** fills FR-4.3 catalog metadata (filename, retention, detection labels) without
putting `raw_bytes` in the catalog.

JSON encoding of logical types: map keys snake_case; `Timestamp` → `*_unix_ms` integer;
enums → snake_case strings; `Option` → `null`. Writers SHALL emit RFC 8785 JCS (or
canonicalize before HMAC/AAD consumers).

### 6.1 `DocumentMeta`

```
DocumentMeta {
  source_filename: string,       // basename only
  source_format: "text" | "pdf",
  imported_at_unix_ms: number,
  retention: "retain" | "discard",  // effective per-document; never "never_retain"
  detected_fields: DetectedField[]  // nested span, including span.text
}
```

`DetectedField.span.text` is stored so `open_approval` after lock works when
`retention=retain`. When `retention=discard`, this row+artifact are **deleted on lock or
abort** (api.md); span text must not outlive the in-memory original.

Global policy `never_retain` (decision 0002 / 0007) is **not** stored on the document.
Import under `never_retain` always writes `retention: "discard"` here.

Page IR and `raw_bytes` are **not** in meta. They live in `OriginalRecord` or in RAM.

### 6.2 `OriginalRecord`

```
{
  "source_format": "text" | "pdf",
  "raw_bytes_b64": string
}
```

`raw_bytes_b64` is RFC 4648 §4 Base64 (standard alphabet, **with** padding, no whitespace
or line breaks). Logical type is `Document.raw_bytes`. Present iff retention is `retain`
(written at import).

### 6.3 `ApprovedVersion` on disk

```
{
  "produced_at_unix_ms": number,
  "decisions": [
    { "field": DetectedField, "decision": "keep_visible" | "redact" }
  ],
  "redacted_content": {
    "format": "text" | "pdf",
    "pages": [ { "page_index": number, "spans": [ TextSpan ] } ]
  }
}
```

`doc_id` is in AAD, not duplicated in plaintext. `redacted_content.spans.text` is
**post-redaction** (redacted spans omitted, not overlay). Share Engine exports this without
`raw_bytes`.

`decisions[].field` is a **full `DetectedField` snapshot** (including `span.text` of what
the user saw at approve time). Share/export must not depend on `document_meta` still existing
if meta is later rewritten; the approved artifact is self-contained.

### 6.4 `Variant` on disk

```
{
  "name": string,
  "created_at_unix_ms": number,
  "overrides": [ { "field_id": string, "decision": "keep_visible" | "redact" } ]
}
```

`variant_id` / `doc_id` are SQL + AAD. Overrides are `field_id` + decision only (deltas
against the approved snapshot). No span text (api.md `get_variant`).

### 6.5 `Config` and `CloudAiSecret` on disk

JCS of §5.5 and §5.7. `key_last4` is stored so `cloud_ai_get_config` need not be specified
as “decrypt then slice”; it is not a secret.

---

## 7. SQLCipher schema (version 1)

One database. Foreign keys ON. Types: `TEXT` UUIDs, `INTEGER` enums/timestamps, `BLOB` crypto.

```sql
CREATE TABLE schema_meta (
  k TEXT PRIMARY KEY,
  v TEXT NOT NULL
);
-- ('schema_version', '1')

CREATE TABLE account (
  account_id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  created_at_unix_ms INTEGER NOT NULL
);

CREATE TABLE artifact (
  artifact_id TEXT PRIMARY KEY,
  kind INTEGER NOT NULL,          -- 1,2,3,4,5,8  (6=keystore, 7=wrap AAD only)
  doc_id TEXT,                    -- NULL for kind 4 and 5
  format_version INTEGER NOT NULL, -- 1
  wrapped_dek BLOB NOT NULL,
  nonce BLOB NOT NULL,            -- 24 bytes
  ciphertext BLOB NOT NULL,
  created_at_unix_ms INTEGER NOT NULL,
  CHECK (kind IN (1, 2, 3, 4, 5, 8))
);

CREATE TABLE document (
  doc_id TEXT PRIMARY KEY,
  meta_artifact_id TEXT NOT NULL UNIQUE
    REFERENCES artifact(artifact_id) ON DELETE RESTRICT,
  original_artifact_id TEXT UNIQUE
    REFERENCES artifact(artifact_id) ON DELETE RESTRICT,
  approved_artifact_id TEXT UNIQUE
    REFERENCES artifact(artifact_id) ON DELETE RESTRICT,
  imported_at_unix_ms INTEGER NOT NULL
);

CREATE TABLE variant (
  variant_id TEXT PRIMARY KEY,
  doc_id TEXT NOT NULL REFERENCES document(doc_id) ON DELETE RESTRICT,
  artifact_id TEXT NOT NULL UNIQUE
    REFERENCES artifact(artifact_id) ON DELETE RESTRICT,
  name TEXT NOT NULL,
  created_at_unix_ms INTEGER NOT NULL,
  UNIQUE (doc_id, name)
);

CREATE TABLE plugin_secret (
  plugin_id TEXT PRIMARY KEY,     -- v1: 'cloud_ai'
  artifact_id TEXT NOT NULL UNIQUE
    REFERENCES artifact(artifact_id) ON DELETE RESTRICT
);

CREATE TABLE audit_entry (
  sequence INTEGER PRIMARY KEY,   -- 1..
  event_type INTEGER NOT NULL,    -- 1=import … 6=delete (architecture §6.1)
  doc_id TEXT,                    -- no FK; survives document delete
  produced_at_unix_ms INTEGER NOT NULL,
  originals_flag INTEGER NOT NULL, -- §5.8: 0=unset, 1=false, 2=true
  payload_jcs TEXT NOT NULL,      -- UTF-8 RFC 8785 of EventPayload
  prev_entry_hash BLOB NOT NULL,  -- 32 bytes
  entry_signature BLOB NOT NULL   -- 32 bytes HMAC
);

CREATE INDEX audit_entry_doc ON audit_entry(doc_id);
CREATE INDEX artifact_doc ON artifact(doc_id);
CREATE UNIQUE INDEX uq_artifact_config ON artifact(kind) WHERE kind = 4;
CREATE UNIQUE INDEX uq_artifact_cloud_ai ON artifact(kind) WHERE kind = 5;
```

`PRAGMA foreign_keys = ON`.

**Config** is the unique `artifact` row with `kind=4` (`doc_id` NULL). No extra table.

**Integrity (Vault code, not only SQL):** `document.meta_artifact_id` → `kind=8`; original /
approved / variant FKs → kinds 2 / 1 / 3. **C-DM-4:** insert into `variant` only if
`document.approved_artifact_id` is non-null. Kind-8 `retention` is `retain` or `discard` only.

**`variant.name`** is duplicated in SQL for `UNIQUE(doc_id, name)` / list without decrypting
every blob. It is user-chosen, not a secret; SQLCipher still covers the stolen-file case.
The envelope remains authoritative; SQL `name` is a cache — on decrypt mismatch, treat as
integrity failure of that variant (do not serve).

**Deletes (one Vault transaction):** because FKs are `ON DELETE RESTRICT`, never delete an
`artifact` while a `document` / `variant` / `plugin_secret` row still points at it.

1. Delete `variant` rows for the `doc_id`.
2. Delete the `document` row.
3. Destroy then delete the orphaned `artifact` rows (kinds 8, 2, 1, 3 as applicable):
   zeroize/drop `wrapped_dek` and `ciphertext` (architecture §4.3).
4. Append audit `delete` (or `discard_original`). Audit rows are **not** deleted.

`cloud_ai_clear_config`: delete `plugin_secret` row, then destroy kind=5 artifact.

Cryptographic erasure (destroy wrap, do not rely on `VACUUM`) is architecture §4.3. This
section owns the SQL order.

---

## 8. Lifecycle ↔ rows

| Event | Rows |
|---|---|
| `create_account` | `account`; `artifact` kind=4 factory config; empty audit genesis not required (first import starts sequence 1). |
| `import_document` while `confirmed=false` | **No rows.** API returns `retention_policy_unset` (decision 0007). |
| `import_document` (confirmed policy) | `document` + `artifact` kind=8; if retain, `artifact` kind=2 and `original_artifact_id`; audit import + detect. |
| `import` discard | No kind=2. Body in RAM until approve / abort / lock. |
| `lock` or `abort` while discard and not approved | Delete `document` + kind=8 artifact; zeroize RAM. |
| `submit_approval` | Insert kind=1; set `approved_artifact_id`. If discard, destroy RAM original (no kind=2). |
| `save_variant` | `variant` + kind=3. Requires `approved_artifact_id` (C-DM-4). |
| `delete_document` | §7 Deletes sequence; audit delete. |
| `set_retention_default` | Overwrite kind=4 plaintext (`confirmed=true`). |
| `cloud_ai_set_config` | Upsert `plugin_secret` + kind=5. Clear = destroy DEK + rows. |

Audit sequences are monotonic in this DB; never reuse a sequence.

---

## 9. In-memory only

| Object | Lifetime |
|---|---|
| `ApprovalSession` + page text | Until submit, abort, or lock |
| `Document.raw_bytes` if discard | Until submit, abort, or lock |
| `ShareRequest` + preview token + PDF/AI bytes | Until commit, expiry, lock, or replaced preview |
| Unwrapped DEKs, master key, SQLCipher key | Until lock (architecture §3.3 / §5) |

None of these have tables.

---

## 10. Future sync (non-goals, constraints)

This schema is **local-embedded**. v1 is not a portable vault-backup format and does not
define reinstall re-attachment (idea.md later phase: vault backup / restore). A later
sync or backup phase (idea.md) SHOULD treat `artifact` rows (kind 1–5 and 8) as copyable
ciphertext units. It MUST NOT assume:

- a Privacy Gate–hosted SQL server in v1;
- that `audit_entry` and `KeystoreItem` merge across devices (HMAC + head are device-local);
- that `DocId` is a global content identity;
- that `vault_master_key` is already on a second device (no recovery in v1).

Those need a new decision. v1 does not add columns “for sync.”

---

## 11. Constraints

- **C-DM-1** One vault file, one `account` row, schema_version 1.
- **C-DM-2** No plaintext document, field text, API key, or passphrase in SQL columns.
  Span text lives only inside envelopes (kind 8 / 1) or RAM.
- **C-DM-3** Kind 6 never appears in `artifact`. Kind 7 is AAD for wraps, not a row.
- **C-DM-4** `approved` ≤ 1 per `doc_id`. Variants require approved.
- **C-DM-5** Factory config blob exists before first import (decision 0007).
- **C-DM-6** Do not persist discard-unapproved bodies.
- **C-DM-7** Field lists and envelope kind codes are defined only in this spec. Other specs
  link here; they do not maintain a parallel struct.

---

## 12. Traceability

| Source | Data-model coverage |
|---|---|
| FR-4.1..4.4 envelope + metadata | §3, §6, `artifact` |
| FR-3.2 one canonical version | `document.approved_artifact_id` |
| FR-5.5 / OQ-7 variants | `Variant` + kind 3 |
| FR-1.4 / dec 0007 config | `Config` kind 4 |
| FR-5.2 / OQ-12 Cloud AI | `CloudAiSecret` + `plugin_secret` |
| FR-7 / OQ-3 audit | `AuditEntry` + §5.8; HMAC → architecture §6 |
| FR-8 account | `LocalAccount` + `KeystoreItem` |
| SRS D-2 / D-4 stored artifacts, encryption | §3, §6, §7 |
| design.md components / flows | consume §5 types; rules stay in design |
| api.md DTOs | projection of §5; not a second schema |
| NFR-R2 delete | §7 Deletes |
| Later sync (idea.md) | §10 non-goal |

---

## 13. Deferred

- UI-owned ephemeral screen state (not persisted) → [ui.md](./ui.md).
- Sync/merge schema, device list, key migration, vault backup/restore and reinstall
  re-attachment (idea.md later phase).
- Extra plugin_secret rows beyond `cloud_ai`.
- `display_name` envelope (not required; architecture: not a secret).

---

## 14. Related Decisions

- [0002](../decisions/0002-resolved-srs-clarifications.md) — one canonical version.
- [0003](../decisions/0003-v1-tech-stack.md) — SQLCipher in-process.
- [0004](../decisions/0004-v1-architecture.md) — envelope kinds, audit, local account.
- [0005](../decisions/0005-review-claude-gemini.md) — Claude + Gemini review.
- [0007](../decisions/0007-retention-default-discard.md) — config factory.

## 15. Related Work

- [0007-data-model-spec](../dev-log/0007-data-model-spec.md)
- [Spec — SRS](./srs.md)
- [Spec — design](./design.md)
- [Spec — architecture](./architecture.md)
- [Spec — API](./api.md)
- [Spec — testing](./testing.md)
- [Spec — UI](./ui.md)
