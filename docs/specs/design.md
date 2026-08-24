# Design Specification — Privacy Gate v1

> Scope: component-level design. What the v1 system is composed of, each component's
> responsibilities, internal state, and the flows that cross components. This spec does
> **not** specify named types, field lists, or the SQL/envelope schema (→
> [`data-model.md`](./data-model.md)), the Tauri command API (→ API spec), the
> crypto/plugin/key-storage architecture (→ architecture spec), UI layout (→ [ui.md](./ui.md)), or
> test plans (→ testing spec).
>
> Source of truth for requirements: [`srs.md`](./srs.md). Stack: [decision 0003](../decisions/0003-v1-tech-stack.md)
> — Tauri 2.x shell, Rust core, TypeScript frontend, in-process on-device detection.
> Command surface: [`api.md`](./api.md). Crypto/keys/plugins/detector: [`architecture.md`](./architecture.md).
> Types and persistence: [`data-model.md`](./data-model.md).
>
> Open questions referenced as `OQ-x` live in [`../notes/open-questions.md`](../notes/open-questions.md).
> Resolved clarifications referenced as `dec 0002` live in [`../decisions/0002-resolved-srs-clarifications.md`](../decisions/0002-resolved-srs-clarifications.md).

---

## 1. Purpose

Privacy Gate is a local-first, single-user, consent-aware redaction vault. The vault is the
product; AI reasoning is an optional plugin. This spec describes the components that realize the
SRS, their boundaries, and the flows between them, so that the architecture, API, UI, and
testing specs can be written against a stable component decomposition. Named types those
flows carry live in [`data-model.md`](./data-model.md).

---

## 2. Components

The v1 system has ten components. The first nine live in the Rust core; the tenth is the
TypeScript frontend. All cross the Tauri IPC boundary only at the API spec's command surface.

```
                    ┌─────────────────────────────────────────────────┐
                    │  TypeScript Frontend (UI spec)                  │
                    │   review/approve, share, audit trail views      │
                    └────────────────────────┬────────────────────────┘
                                             │  Tauri commands (API spec)
                                             ▼
┌───────────────────────────────────────────────────────────────────────┐
│  Rust Core                                                            │
│                                                                       │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌──────────────┐    │
│  │ Importer   │→ │ Detector   │→ │ Approval   │→ │ Vault        │    │
│  │            │  │            │  │ Engine     │  │ (encrypted)  │    │
│  └────────────┘  └────────────┘  └────────────┘  └──────┬───────┘    │
│        │               │              │                 │            │
│        │               │              │                 ▼            │
│        │               │              │           ┌──────────────┐   │
│        │               │              │           │ Share Engine │   │
│        │               │              │           └──────┬───────┘   │
│        │               │              │                  │           │
│        ▼               ▼              ▼                  ▼           │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │  Audit Trail (tamper-evident; crypto primitive → arch spec) │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                  │                                    │
│                                  ▼                                    │
│  ┌─────────────────────────────────────────────────────────────┐    │
│  │  Plugin Host — output consumers / detectors / new flows     │    │
│  │  v1: Cloud AI output consumer only; other hooks empty       │    │
│  └─────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  ┌──────────────┐        ┌──────────────┐                            │
│  │ Config       │        │ Key Manager  │  first-run account + key   │
│  │ (retention   │        │ (unlock,     │  lifecycle; key storage     │
│  │  defaults)   │        │  key gen)    │  → arch spec (OQ-18)       │
│  └──────────────┘        └──────────────┘                            │
└───────────────────────────────────────────────────────────────────────┘
```

### 2.1 Importer
Responsibilities:
- Accept a born-digital text or PDF file from the frontend (FR-1.1).
- Extract text and document structure (pages, spans, reading order) into an intermediate
  in-memory representation (§3.1). For PDFs this must preserve byte offsets so spans can be
  mapped back to the source for preview/export.
- Reject inputs with no extractable text (scanned PDFs, images) with a clear message; never
  silently treat them as redactable (FR-1.2).
- Read the global retention default from Config and the per-import override, and pass the
  resolved retention decision to the Vault with the original bytes (FR-1.3, FR-1.4). If the
  default is still unconfirmed (decision 0007), refuse import — do not detect or store.
- Emit an `import` event to the Audit Trail with the retention decision (FR-1.5).
- **Raw-bytes destruction hand-off (FR-1.3):** the Importer holds `raw_bytes` in process memory
  until the Vault acknowledges it has encrypted-and-stored the retained original (if "retain") or
  acknowledges the approved version is stored (if "discard"). On either acknowledgement the
  Importer overwrites its `raw_bytes` buffer. The Vault is authoritative for the destruction
  point; the Importer never silently drops bytes. (Transient-plaintext handling:
  [architecture spec §5](./architecture.md); this spec fixes only the component hand-off.)

Non-goal: OCR (out of v1, §8 of SRS).

### 2.2 Detector
Responsibilities:
- Run the on-device detection model over the Importer's intermediate representation and produce
  a list of classified fields, each with a label and a byte-offset span (FR-2.1, FR-2.2).
- Run entirely in-process in the Rust core, **or** via the optional local Ollama backend
  reached over a strictly loopback, IP-literal, non-DNS, non-proxied connection (decision
  0009); no other network calls (NFR-P1, C-5).
- Expose detector plugin hooks (FR-2.4) so additional recognizers can be registered; v1 ships
  the hooks empty of first-party plugins (FR-9.4).
- Emit a `detect` event to the Audit Trail with the detected fields and classifications (FR-3.4
  precursor).

Non-goal: the concrete model identity — [`architecture.md` §10](./architecture.md)
(`pg-hybrid-v1`, and the optional `pg-hybrid-ollama-v1` backend, decision 0009).

### 2.3 Approval Engine
Responsibilities:
- Hold an approval session for one document at a time, with explicit lifecycle states:
  `AwaitingDecisions` → `Decided` → `Committed` (or `Aborted`). In `AwaitingDecisions` the
  frontend receives the unapproved document structure and detected spans (see C-DES-1) so the
  user can make keep/redact decisions; in `Decided` the user has confirmed; in `Committed` the
  canonical `ApprovedVersion` has been written to the Vault. `Aborted` discards the session with
  no stored artifact.
- For each detected field, accept the user's decision: keep visible, or redact (FR-3.1).
- After `Committed`, the document cannot re-enter approval in v1; later shares use ephemeral
  overrides or named variants (FR-5.4/5.5, API `already_approved`).
- Produce exactly one canonical approved version per document from those decisions (FR-3.2,
  dec 0002 / Q8). The approved version carries the rendered redacted content (§3.2), not just
  span decisions, so downstream share/export works whether or not the original was retained.
- Resolve overlapping/nested detected fields per §3.5 (resolves OQ-16).
- Emit an `approve` event to the Audit Trail with per-field classification + decision (FR-3.4).

### 2.4 Vault
Responsibilities:
- Store approved versions (including rendered redacted content, §3.2) encrypted at rest under
  envelope encryption (FR-4.1, NFR-S1).
- Store the import basename (`source_filename`) as encrypted catalog metadata (FR-4.3); not a
  filesystem path. Used by the API for export filename suggestions.
- Store retained originals under the same scheme when the retention decision is "retain"
  (FR-4.2). The Vault is the destruction authority for discarded originals (§2.1 hand-off).
- Store, retrieve, and delete named variants encrypted alongside the approved version, scoped to
  a single `DocId` (§3.4, FR-5.5, resolves OQ-7).
- Encrypt metadata alongside content (FR-4.3, NFR-S1).
- Provide irrevocable deletion of approved versions, retained originals, and variants
  (FR-4.6, NFR-R2). Deletion is a Vault-owned flow: the Vault removes the ciphertext and the
  corresponding envelope-encryption key material for that artifact so it cannot be decrypted
  again; the Audit Trail records a `delete` event. (Key-material erasure: architecture spec
  §3.1 / §4.3.)
- Guarantee that a stolen data file alone yields no plaintext content or metadata (FR-4.4,
  NFR-S3).
- Decrypt an approved version into process memory for the duration of one share, on demand from
  the Share Engine.

Non-goals: holding the master unlock key — that is the Key Manager (§2.10). The cipher, key
derivation, key storage, rotation, recovery, and transient-plaintext handling are architecture
spec (OQ-17, OQ-18).

### 2.5 Share Engine
Responsibilities:
- **Generate redacted preview artifact (ephemeral):** before any share, render a preview of
  exactly what will leave (FR-6.1) as a redacted preview artifact (a redacted PDF or raster
  representation plus a metadata manifest of what is visible/redacted). The artifact contains no
  redacted field text. The frontend displays it without ever receiving redacted content. The
  ephemeral-override warning (FR-6.2) is a UI-spec concern, but the Share Engine tags the
  artifact with whether overrides are in effect so the frontend can render the warning.
- **Share to a person (export):** render the approved version (plus any ad-hoc overrides) to a
  redacted file. Supports one or multiple approved documents in a single `ShareRequest` (§3.7);
  when multiple are selected, produce a single combined PDF bundle (FR-5.1, dec 0002 /
  Q4-bundle). Single-document export format and bundle ordering are fixed in §3.7 (resolves the
  design-owned part of OQ-4). Exported files must not contain recoverable redacted text or
  metadata — redaction is true removal, not a visual overlay (NFR-S4, dec 0002 / Q11). The
  precise sanitization mechanism is re-render of a new PDF ([architecture spec §11](./architecture.md)).
- **Share to an AI:** assemble the approved content of the selected documents and hand it to the
  Cloud AI plugin via the Plugin Host; return the read-only text output to the frontend
  (FR-5.2).
- Enforce that only approved content leaves the vault in any share; redacted fields never leave
  the device (FR-5.3, NFR-S4, NFR-P2).
- Apply ad-hoc overrides at share time. Overrides are ephemeral: they apply only to the single
  share in progress and are discarded after it completes; they never mutate the canonical
  approved version (FR-5.4, dec 0002 / Q4-ephemeral).
- Optionally save an ad-hoc override set as a named variant on explicit user action (FR-5.5),
  via the Vault's variant store/load ops.
- Emit a `share` event to the Audit Trail for every person-export and every AI-share (FR-5.6),
  including recipient (user-entered note, FR-7.2) and the "no private originals left the device"
  assertion condition (§2.6).

### 2.6 Audit Trail
Responsibilities:
- Record `import`, `detect`, `approve`, `share`, `discard-original`, and `delete` events
  (FR-7.2, FR-4.6).
- Be durable across app restarts and tamper-evident — any post-creation modification of an entry
  is detectable (NFR-R1). Each entry carries integrity data fields (`prev_entry_hash`,
  `entry_signature`) defined in [`data-model.md` §5.8](./data-model.md). The crypto primitive
  is HMAC-SHA-256 ([architecture spec §6](./architecture.md)).
- Expose a **read/query interface** to the frontend: list events, filter by `DocId`, filter by
  event type, paginate (FR-7.4, NFR-U1). This interface returns only audit data, never document
  content.
- Give the user a verifiable answer to "what did I actually share?" by person or AI, without
  trusting the app on faith (FR-7.3, NFR-U1).
- Be inspectable at any time while the vault is unlocked (FR-7.4).
- Record the "no private originals left the device" assertion for share events. The design-owned
  semantics (resolves the design half of OQ-6): the Share Engine sets the assertion true iff the
  document's retention decision was "discard" (so no original exists to leave) **or** the share
  transmits only the approved version and never the retained original. Independent
  verification: [`testing.md` §7](./testing.md) (egress spy + canary oracle; do not trust the
  flag alone).

### 2.7 Plugin Host
Responsibilities:
- Expose the three-part plugin surface: output consumers, detectors, new flows (FR-9.1).
- Support two invocation modes: user-invoked (user picks a plugin and chosen documents) and
  event-triggered opt-in (a plugin reacts on import/redact/share with explicit per-plugin opt-in)
  (FR-9.2).
- Receive **approved content with the Share Engine's overrides/variants already applied** — so
  the plugin never sees redacted fields and never has to re-apply overrides itself (FR-5.3,
  NFR-P2).
- v1 ships only the first-party Cloud AI output consumer (FR-9.3). Detector and new-flow hooks
  are present but empty of first-party plugins; reactive hooks likewise empty (FR-9.4).
- The v1 runtime hosts first-party code in-process; a WASM-sandboxed runtime for third-party
  plugins is the expected later path ([architecture spec §8](./architecture.md)); v1 does not
  ship a sandbox.

### 2.8 TypeScript Frontend
Responsibilities:
- Render the review/approve, share, variant, and audit-trail views. UI layout is UI spec; this
  spec fixes only that the frontend holds no secrets and issues Tauri commands to the core.
- During the **review/approve** step, receive the unapproved document structure and detected
  spans (labels + locatable spans) from the core so the user can make keep/redact decisions
  (FR-2.2, FR-3.1). This is the one place the frontend sees not-yet-redacted content, and it is
  content the user is choosing what to hide — not redacted fields being leaked.
- During **share preview**, receive only the redacted preview artifact from the Share Engine
  (§2.5); the frontend never receives redacted field text for a share it is previewing
  (supports FR-6.1 without leaking redacted content into the frontend layer).
- Communicate with the core solely through the API spec's Tauri command surface.

### 2.9 Config
Responsibilities:
- Store and serve global user configuration, in particular the retention default (retain vs.
  discard vs. paranoid "never retain originals") that FR-1.4 requires and that the Importer reads
  at import time (§2.1).
- Enforce the paranoid-default constraint: when the default is "never retain originals", per-import
  overrides may not loosen it to retain (FR-1.4, dec 0002 / Q9). Config validates override
  attempts; the Importer never sees a loosened value.
- Factory value is `discard`, stored as **unconfirmed** until the user sets a policy
  ([decision 0007](../decisions/0007-retention-default-discard.md)). The Importer refuses import
  while unconfirmed. Pre-select chrome for the first-upload prompt is UI spec.
- Config itself is stored encrypted at rest alongside vault metadata (NFR-S1), since it reveals
  user policy.

### 2.10 Key Manager
Responsibilities:
- Own first-run account creation, passphrase setup, and initial encryption-key generation
  (FR-8.1, FR-8.2). On first run the Key Manager generates a key that never leaves the machine
  and binds it to the account.
- Unlock the vault locally for day-to-day use (FR-8.3); no network identity is required to open
  the vault. The Key Manager derives the vault key from the passphrase and on-device key
  material and hands it to the Vault for the duration of a session.
- Boundary with the Vault: the Key Manager holds the master unlock key; the Vault holds the
  envelope-encrypted content and per-artifact key material. The Vault never sees the passphrase.

Non-goals: the key-derivation function, on-device key storage location, key rotation, and
recovery are specified in [architecture spec §3](./architecture.md) (OQ-18 resolved: passphrase
change yes, recovery no). Account creation is local-only ([architecture spec §7](./architecture.md),
OQ-5 resolved).

---

## 3. Current Behavior (flows)

Types named here (`Document`, `DetectedField`, `ApprovedVersion`, `Variant`, `ShareRequest`,
`AuditEntry`, …) are defined in [`data-model.md`](./data-model.md). This section owns the
rules that operate on them.

### 3.1 Document model (in-memory intermediate representation)

Importer → Detector → Approval Engine pass `Document` / `Page` / `TextSpan` / `DetectedField`
([`data-model.md` §5.1](./data-model.md)). `Document.raw_bytes` is held only in process
memory during import; handed to Vault iff retention = retain; overwritten by Importer on
Vault ack (§2.1). Nesting uses `DetectedField.parent_field_id` (§3.5).

### 3.2 Approved version

`ApprovedVersion`, `FieldDecision`, and `RedactedDocument`: [`data-model.md` §5.2](./data-model.md).

One canonical `ApprovedVersion` per `DocId` (FR-3.2, dec 0002 / Q8). Variants (§3.4) are not
separate approved versions. The `redacted_content` is produced by the Approval Engine at
commit time and stored encrypted by the Vault. That is what makes discard-original safe for
downstream sharing (§3.3): Share Engine never needs `Document.raw_bytes`.

### 3.3 Canonical flow
```
Import  → Document (in memory)
  → Detector → Vec<DetectedField>
    → Approval Engine → user per-field decisions → ApprovedVersion (with redacted_content)
      → Vault (encrypted at rest) ──── Audit Trail: import, detect, approve
        → Share Engine (on demand)
            ├── export: redacted file / PDF bundle  ── Audit Trail: share(person)
            └── Cloud AI plugin: approved content → read-only text ── Audit Trail: share(AI)
```
Retention of the original: at import, if "retain", `Document.raw_bytes` is encrypted and stored
by the Vault; if "discard", the Importer overwrites `raw_bytes` after the Vault acknowledges the
`ApprovedVersion` is stored, and a `discard-original` event is emitted (FR-1.3, FR-7.2, §2.1).
Because the `ApprovedVersion` carries `redacted_content`, export and AI-share work whether or
not the original was retained.

### 3.4 Variants (resolves OQ-7)

Type: [`data-model.md` §5.3](./data-model.md) (`Variant`).

- A **variant** is a named, encrypted set of overrides relative to a canonical `ApprovedVersion`,
  scoped to a single `DocId`.
- Lifecycle: create (from an ad-hoc override set via explicit user action), apply (at share
  time, ephemerally — applying a variant does not mutate the canonical version), delete.
- Editing a variant is not supported in v1: to change a variant, delete it and save a new one
  from a fresh ad-hoc override set. This is a deliberate design choice to keep a single mutation
  path alongside the canonical version and avoid variant-vs-canonical drift.
- Variants are stored encrypted alongside the approved version via the Vault's
  `store_variant` / `load_variant` / `delete_variant` operations (§2.4, §4).

### 3.5 Overlapping fields (resolves OQ-16)
- Detected fields may overlap or nest. The Approval Engine presents each field independently.
- **Nesting** is tracked via `DetectedField.parent_field_id` (§3.1). A field is "innermost" for
  a byte offset if no other decided field covering that offset has it as a parent.
- **Decision precedence:** the innermost (most specific) field's explicit user decision wins. A
  redact on an outer field does not cascade to an inner field the user kept; a keep on an outer
  field does not force an inner field the user redacted to be revealed.
- **Partial (non-nested) overlaps:** for two fields that intersect but neither contains the
  other, **Redact wins** on the intersection unless a third field strictly nested inside the
  intersection is decided Keep. This is conservative (favor hiding) and deterministic.
- At share/export time, a byte offset is redacted iff any field covering it is decided Redact
  and no more-specific field covering it is decided Keep. This is a single, deterministic rule
  the testing spec can check.

### 3.6 Re-import (resolves OQ-15)
- Importing a file whose content already maps to an existing `DocId` is treated as a **new
  document** with a new `DocId` in v1, not a revision of the existing one.
- Rationale: v1 has no document identity beyond content hash + import time, and no merge/diff
  UI. Treating re-imports as new documents is the simplest behavior that does not silently
  overwrite an approved version. A revision/replace flow is a later phase. The UI spec owns any
  duplicate-detection notice; the design does not deduplicate.

### 3.7 Share request (resolves the design-owned part of OQ-4)

Type: [`data-model.md` §5.4](./data-model.md) (`ShareRequest`). IPC: `ShareRequestDto` in
[`api.md`](./api.md).

- **Single-document export format:** PDF (matching the multi-doc bundle). A single-document
  export is a one-document PDF bundle. Plain-text export is not in v1.
- **Multi-document bundle ordering:** documents are ordered by the order the user selected them
  in the frontend (preserved through the API spec). Suggested filename algorithm and PDF
  info-dictionary fields: [api.md §7](./api.md). Save-dialog chrome: [ui.md §10.4](./ui.md).
- Export and AI-share both take a `ShareRequest`; the Share Engine applies overrides/variants,
  re-renders `redacted_content` for the request, and emits the share event.

### 3.8 Audit entry (supports NFR-R1; crypto primitive → architecture spec §6)

Type and payload keys: [`data-model.md` §5.8](./data-model.md) (`AuditEntry`, `EventPayload`).
HMAC canonical encoding: [`architecture.md` §6](./architecture.md).

The Audit Trail exposes a query interface over `AuditEntry` rows (§2.6, §4). `Share` events
set `no_originals_left_device` per §2.6.

---

## 4. Interfaces

This section lists component interfaces at the responsibility level. The concrete Tauri command
surface (names, argument types, return types, errors) is the API spec.

| From → To | Interface (level) |
|---|---|
| Frontend → Core | Tauri commands (API spec). Frontend never touches raw bytes, keys, or redacted field content directly. |
| Frontend → Audit Trail | Read/query: `list_events`, `filter_by_doc`, `filter_by_type`, paginate (FR-7.4, NFR-U1). Returns `AuditEntry` rows only, never document content. |
| Importer → Detector | `Document` intermediate representation (§3.1). |
| Importer → Config | Read retention default + validate per-import override (paranoid-default enforcement, §2.9). |
| Detector → Approval Engine | `Vec<DetectedField>` (label + span + classification + parent_field_id). |
| Approval Engine → Vault | `ApprovedVersion` (§3.2, incl. `redacted_content`); plus `Document.raw_bytes` iff retention = retain. |
| Vault → Share Engine | Decrypted `ApprovedVersion` (in process memory, for the duration of one share). |
| Vault ↔ Frontend (via Core) | `store_variant` / `load_variant` / `delete_variant` (FR-5.5, §3.4); `delete_approved_version` / `delete_retained_original` (FR-4.6, §2.4). Concrete command names: [api.md](./api.md). |
| Share Engine → Frontend | Redacted preview artifact (no redacted field content) for FR-6.1. |
| Share Engine → Plugin Host | Approved content with overrides/variants already applied. |
| Plugin Host → Cloud AI plugin | Approved content in; read-only text out (FR-5.2). |
| Every core component → Audit Trail | Event append (import/detect/approve/share/discard-original/delete), §3.8. |
| Key Manager → Vault | Session vault key (for the duration of an unlocked session). |
| Key Manager ↔ Frontend (via Core) | First-run account creation, passphrase entry, unlock (FR-8.1..8.3). |

---

## 5. Dependencies

- **SRS** [`srs.md`](./srs.md) — every component traces to FR/NFR IDs in §2–§7.
- **Decision 0003** [tech stack](../decisions/0003-v1-tech-stack.md) — Tauri + Rust + TS.
- **Decision 0002** [resolved clarifications](../decisions/0002-resolved-srs-clarifications.md)
  — one canonical version; paranoid-default semantics; manual redaction out of v1; export = true
  removal; multi-doc PDF bundle.
- **Architecture spec** [`architecture.md`](./architecture.md) — owns: crypto scheme and library, key storage and
  rotation (OQ-18, resolved), transient-plaintext handling (OQ-17, resolved), audit-trail integrity mechanism
  (OQ-3, resolved), account network role (OQ-5, resolved), plugin security/sandbox
  (OQ-13, resolved), Cloud AI auth (OQ-12, resolved in architecture §9 + [api.md](./api.md) §5.7), detection model identity.
- **API spec** [`api.md`](./api.md) — Tauri command surface, including Cloud AI config
  set/clear/test and export filename/metadata (OQ-4 remainder).
- **Testing spec** [`testing.md`](./testing.md) — TDD, mutation gate, AC-1..AC-7, OQ-6 oracle.
- **Data model spec** [`data-model.md`](./data-model.md) — single source for types, IDs,
  SQLCipher schema, envelope plaintext. This spec names those types; it does not redefine them.
- **UI spec** [`ui.md`](./ui.md) — Svelte 5 webview, screens, copy, save-dialog chrome, first paint.

---

## 6. Constraints

- **C-DES-1** All keys and redacted-field text live in the Rust core. The frontend receives
  only what it needs: during review/approve, the unapproved document structure and detected
  spans so the user can decide (FR-2.2, FR-3.1); during share preview, a redacted preview
  artifact with no redacted field text (FR-6.1); and audit-trail entries (no document content).
- **C-DES-2** Detection runs in-process, no network, **except** the optional local Ollama
  backend over a strictly loopback, IP-literal connection (NFR-P1, C-5, decision 0009).
- **C-DES-3** Only approved content leaves the vault in any share (FR-5.3, NFR-P2, NFR-S4).
- **C-DES-4** Export redaction is true removal (dec 0002 / Q11); the mechanism is architecture
  spec but the design guarantees no recoverable redacted text in exports.
- **C-DES-5** Overrides are ephemeral per share; variants are the only persistent override form
  and are scoped per document (§3.4, dec 0002 / Q4-ephemeral).
- **C-DES-6** v1 plugin runtime is in-process first-party only; third-party runtime is a later
  phase (FR-9.4, OQ-13).
- **C-DES-7** Supported OSes: macOS, Windows, Linux (decision 0003, resolves OQ-1).
- **C-DES-8** Re-imports are new documents in v1 (§3.6, resolves OQ-15).

---

## 7. Performance budget (resolves OQ-2)

These are design-level budgets the architecture and testing specs will refine and enforce:

- Import + text extraction: ≤ 2 s for documents up to 1 MB, ≤ 10 s up to 25 MB.
- Detection: ≤ 5 s for documents up to 1 MB, ≤ 30 s up to 25 MB.
- Approval review render: first paint budget is owned by the UI spec; the core returns the
  document + up to 200 detected fields to the frontend in ≤ 1 s.
- Export (single document, ≤ 25 MB): ≤ 5 s; multi-doc PDF bundle (≤ 10 documents, ≤ 50 MB
  total): ≤ 15 s.
- Vault unlock: ≤ 1 s after the user enters the passphrase.
- Audit trail query (last 1000 events): ≤ 500 ms.

The on-device detection model identity and host-device hardware assumptions are architecture
spec; these budgets assume a mainstream laptop (8 GB RAM, SSD). Documents beyond 25 MB are out
of the v1 interactive budget — the Importer reports that the document exceeds the budget and
continues processing (the wording/UI is UI spec).

---

## 8. Traceability to SRS

| SRS requirement | Design coverage |
|---|---|
| FR-1.1 / FR-1.2 import + reject | §2.1 Importer |
| FR-1.3 / FR-1.4 retention + paranoid default | §2.1, §2.9 Config, §3.3, dec 0002, dec 0007 |
| FR-1.5 audit import | §2.1 → Audit Trail |
| FR-2.1..2.4 detection | §2.2 Detector |
| FR-3.1..3.4 approval + one canonical version | §2.3, §3.2 (incl. redacted_content), §3.5 (dec 0002 / Q8, OQ-16) |
| FR-4.1..4.6 encrypted storage + irrevocable delete | §2.4 Vault (incl. variant ops + delete flow); crypto → architecture spec |
| FR-5.1 export (incl. PDF bundle) | §2.5, §3.7, dec 0002 / Q4-bundle |
| FR-5.2 Cloud AI | §2.5 + §2.7 |
| FR-5.3 only approved content leaves | §2.5, §2.7, C-DES-3 |
| FR-5.4 ephemeral overrides | §2.5, §3.4, §3.7, C-DES-5 |
| FR-5.5 variants | §2.4, §3.4 (resolves OQ-7) |
| FR-5.6 audit share | §2.5 → Audit Trail |
| FR-6.1 / 6.2 preview + ephemeral warning | §2.5 (preview artifact), §2.8; UI spec for wording |
| FR-7.1..7.4 audit trail (incl. read/query) | §2.6, §3.8, §4 |
| FR-8.1..8.3 account + on-device key | §2.10 Key Manager; local-only → architecture §7 |
| FR-9.1..9.5 plugin model | §2.7 |
| NFR-S1..S4 security | §2.4, §2.10, C-DES-1, C-DES-3, C-DES-4 |
| NFR-P1..P3 privacy | §2.2, C-DES-2, C-DES-3 |
| NFR-PERF1 performance | §7 (resolves OQ-2) |
| NFR-R1 / R2 integrity + irrevocable delete | §2.6, §3.8 (fields; crypto → architecture §6), §2.4 |
| NFR-U1 / U2 usability | §2.6, §2.8 (UI spec for wording) |
| NFR-PORT1 OS support | decision 0003, C-DES-7 (resolves OQ-1) |
| NFR-E1 extensibility | §2.7, C-DES-6 |

---

## 9. Open questions owned by this spec (resolved here)

- **OQ-1** Supported OSes → macOS, Windows, Linux (decision 0003, C-DES-7).
- **OQ-2** Performance thresholds → §7 budgets.
- **OQ-7** Variant lifecycle → §3.4 (create / apply / delete; no edit; per-document scope).
- **OQ-15** Re-import behavior → §3.6 (new document, no revision flow in v1).
- **OQ-16** Overlapping/nested fields → §3.5 (innermost explicit decision wins; partial overlaps
  → Redact wins unless a strictly nested sub-span is kept; one deterministic redaction rule).
- **OQ-4 (design part)** Single-document export format (PDF) and multi-doc bundle ordering
  (user selection order) → §3.7. Filename algorithm and PDF metadata → [api.md §7](./api.md).
  Save-dialog chrome → [ui.md §10.4](./ui.md).
- **OQ-6 (design part)** "No originals left device" assertion condition → §2.6 (true iff
  retention was "discard" or the share transmits only the approved version). **Verification**
  → [testing.md §7](./testing.md).

## 10. Open questions deferred (not owned here)

- **OQ-3** Audit-trail crypto primitive (hash/signature scheme) → **resolved** by
  [architecture spec §6](./architecture.md) and [decision 0004](../decisions/0004-v1-architecture.md).
- **OQ-4 (remainder)** Export file naming and metadata fields → **API part resolved** by
  [api.md §7](./api.md) (suggested filename algorithm + PDF info dictionary). Save-dialog
  chrome → **resolved** by [ui.md §10.4](./ui.md).
- **OQ-5** Account network role → **resolved** by [architecture spec §7](./architecture.md)
  (local-only; Key Manager first-run has no network step).
- **OQ-6 (remainder)** Independent verification of the "no originals left device" assertion →
  **resolved** by [testing.md §7](./testing.md).
- **OQ-12** Cloud AI auth → **resolved** by architecture spec §9 + [api.md §5.7](./api.md)
  (command shape; key never in outputs).
- **OQ-13** Plugin security/sandbox → **resolved** by [architecture spec §8](./architecture.md).
- **OQ-17** Transient plaintext handling → **resolved** by [architecture spec §5](./architecture.md).
- **OQ-18** Key rotation / recovery → **resolved** by [architecture spec §3.3](./architecture.md).
- **OQ-14** Retention default initial value → **resolved** by
  [decision 0007](../decisions/0007-retention-default-discard.md): factory `discard`,
  unconfirmed until `set_retention_default`; Importer refuses while unconfirmed.

---

## 11. Related Decisions

- [0002 — resolved SRS clarifications](../decisions/0002-resolved-srs-clarifications.md) — one
  canonical version; paranoid-default semantics; manual redaction out of v1; export = true
  removal; multi-doc PDF bundle.
- [0003 — v1 tech stack](../decisions/0003-v1-tech-stack.md) — Tauri + Rust + TS.
- [0004 — v1 architecture](../decisions/0004-v1-architecture.md) — crypto, local account, audit
  MAC, plugin host API, Cloud AI auth, hybrid detector, re-render export.
- [0005 — review roster](../decisions/0005-review-claude-gemini.md) — Claude + Gemini.
- [0006 — TDD + mutation](../decisions/0006-tdd-and-mutation-testing.md).
- [0007 — retention default](../decisions/0007-retention-default-discard.md) — factory discard; first-import confirm.
- [0008 — Svelte frontend](../decisions/0008-frontend-svelte.md).

## 12. Related Work

- [0001-srs-generation](../dev-log/0001-srs-generation.md) — produced the SRS this design traces to.
- [0002-design-spec](../dev-log/0002-design-spec.md) — produced this design spec (three-model
  review + reconciliation).
- [0003-architecture-spec](../dev-log/0003-architecture-spec.md) — filled the architecture deferrals.
- [0004-api-spec](../dev-log/0004-api-spec.md) — Tauri command surface.
- [0005-testing-spec](../dev-log/0005-testing-spec.md) — TDD, mutation, AC mechanics, OQ-6.
- [0007-data-model-spec](../dev-log/0007-data-model-spec.md) — SQLCipher / envelope schema; later made the single source for all named types.
- [0008-ui-spec](../dev-log/0008-ui-spec.md) — webview screens, copy, save-dialog chrome.