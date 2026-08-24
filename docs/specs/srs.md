# Software Requirements Specification — Privacy Gate

> Scope of this document: requirements only. Design, data-model, API, architecture, testing, and UI
> specifications are produced separately. This SRS captures *what* the system must do and the
> qualities it must satisfy, not *how* it is built.
>
> Source of truth: `docs/idea.md` (product idea) and `docs/user-story.md` (worked example). Where
> this SRS and the idea doc disagree, the idea doc is authoritative and this SRS must be
> corrected; gaps in the idea doc surface as resolved clarifications in
> `docs/decisions/0002-resolved-srs-clarifications.md` or as open questions in
> `docs/notes/open-questions.md` (see §10).

---

## 1. Introduction

### 1.1 Purpose
This document specifies the functional and non-functional requirements for **Privacy Gate**, a
local-first, single-user desktop application that acts as a consent-aware redaction vault for
sensitive documents. The vault is the product; AI reasoning is an optional plugin layered on top.

### 1.2 Document conventions
- "shall" = binding requirement.
- "should" = strong recommendation, not binding.
- "may" = optional capability.
- v1 = the first release this SRS governs. Later phases are named in §8 but not specified here.
- "Field" = a detected sensitive region of a document (a span), not a form control.

### 1.3 Intended audience
Implementers, reviewers, and future spec authors (design, data-model, API, architecture, testing, UI).

### 1.4 References
- `docs/idea.md` — product idea (authoritative).
- `docs/user-story.md` — worked example used to validate requirements.

---

## 2. Overall description

### 2.1 Product perspective
Privacy Gate is a self-contained desktop application. It is not a hosted service. It does not
relay, store off-device, or hold the user's encryption key on any server. An account exists for
future backup/sync/mediated-sharing phases, but v1 day-to-day vault access requires no network
identity.

### 2.2 User classes
| Class | Description | v1? |
|---|---|---|
| Vault owner | The single user who installs, unlocks, imports, approves, and shares. | Yes |
| Share recipient (person) | Receives an exported redacted file out-of-band; is not a user of the app. | Yes (passive) |
| AI provider | Receives approved content via the Cloud AI plugin over the network. | Yes (passive, via plugin) |
| Plugin author | Writes output-consumer / detector / new-flow plugins against v1 extension points. | Hooks present; first-party plugins in v1 limited to Cloud AI. |

### 2.3 Operating environment
Desktop OS (local-first). On-device detection model. Network access is required only for the
Cloud AI plugin when invoked; account creation may use the network at first run, but day-to-day
vault unlock and use require no network identity (see FR-8.3). Precise OS versions are a design
decision, out of scope here.

### 2.4 Assumptions and dependencies
- Input documents are born-digital text or PDFs with extractable text. Scanned/image/OCR inputs
  are not assumed (§8).
- The user has an account for unlock-credential binding; the encryption key lives on-device.
- A local detection model is available to the app at runtime.

---

## 3. Functional requirements

Requirements are grouped by capability. Each has a stable ID (FR-x.y).

### 3.1 Import

**FR-1.1** The app shall import born-digital text documents and PDFs with extractable text.

**FR-1.2** The app shall reject inputs it cannot extract text from (e.g., scanned PDFs, images)
with a clear message; it shall not silently process them as if redactable.

**FR-1.3** At import the user shall decide, per document, whether to retain the original
(encrypted) alongside the approved version or discard the original after the approved version is
produced.

**FR-1.4** A global retention default shall exist. The factory value is **discard originals**,
and it is unconfirmed until the user explicitly sets a policy. The first successful import
shall not proceed until that policy is confirmed; the first-upload prompt shall pre-select
discard. After confirmation, each import may override the default in either direction, except
that a global "never retain originals" (paranoid) default may not be loosened per-import to
retain. (See [decision 0002](../decisions/0002-resolved-srs-clarifications.md) Q9 and
[decision 0007](../decisions/0007-retention-default-discard.md).)

**FR-1.5** The app shall record the import event and the per-document retention decision in the
audit trail (see §3.7).

### 3.2 Detection

**FR-2.1** On import, an on-device model shall identify sensitive fields in the document and
classify each with a label.

**FR-2.2** Each detected field shall be presented to the user as a labeled, locatable span in the
document so the user can identify it (precise presentation is a UI decision).

**FR-2.3** Detection shall run locally; no document content shall leave the device for detection
in v1. **Exception (decision 0009):** detection may reach a pre-existing local service (the
optional Ollama backend) over a strictly loopback, IP-literal, non-DNS, non-proxied connection,
subject to architecture §10's pin/allowlist/verification rules. This is a narrow, named
exception for that one backend, not a general license for the core to make network calls.

**FR-2.4** Detection shall be extensible via detector plugin hooks (see §3.9); v1 ships these
hooks present but not exercised by first-party detector plugins.

### 3.3 Per-field approval (the consent step)

**FR-3.1** For each detected field the user shall decide: keep visible, or redact.

**FR-3.2** The app shall produce exactly one canonical approved version per document from the
user's per-field decisions.

**FR-3.3** The canonical approved version shall be the basis for all subsequent shares unless an
ad-hoc override is applied at share time (see §3.5).

**FR-3.4** The app shall record, per field, the detected classification, the user's decision, and
the resulting state in the audit trail.

### 3.4 Encrypted storage (the vault)

**FR-4.1** The app shall store approved versions encrypted at rest using envelope encryption
unlocked by the user.

**FR-4.2** Retained originals (when the retention decision is "retain") shall be stored encrypted
under the same envelope-encryption scheme.

**FR-4.3** Metadata associated with originals and approved versions shall likewise be encrypted
at rest.

**FR-4.4** A stolen data file shall be unusable without the unlock step; no plaintext document
content or document metadata shall be recoverable from a data file alone.

**FR-4.5** The encryption key shall live on the device and be unlocked locally; the app shall not
require a network identity to open the vault for day-to-day use.

**FR-4.6** The user shall be able to delete vault contents; deletion of an approved version and
of a retained original (if any) shall be irrevocable.

### 3.5 Sharing

**FR-5.1 — Share to a person.** The app shall produce a redacted file (export) from the approved
version that the user hands off themselves. The user may select one or multiple approved
documents for a single share; when multiple are selected the export shall be a single combined
bundle (the user story specifies a single PDF bundle). v1 is export-only; the app shall not
mediate delivery (no links, view controls, or revocation in v1).

**FR-5.2 — Share to an AI.** The Cloud AI plugin shall send the approved content of selected
documents to a cloud model and return read-only text output (e.g., explain, compare, checklist,
draft). v1 AI outputs shall be read-only text; agentic action-taking is out of scope (§8).

**FR-5.3** Only approved content shall leave the vault in any share. Redacted fields shall not
leave the device in any share, person or AI.

**FR-5.4 — Ad-hoc overrides.** Before any specific share, the user may override the canonical
approved version to reveal more or hide more. Overrides shall be ephemeral: an override set
applies only to the single share for which it was made and is discarded after that share
completes; it shall not alter the canonical approved version. (Saving an override set as a
reusable variant is a separate, explicit action — see FR-5.5.)

**FR-5.5 — Variants.** The user may optionally save an ad-hoc override set as a named variant for
reuse on future shares.

**FR-5.6** Each share (person or AI) shall be recorded in the audit trail (see §3.7).

### 3.6 Pre-share preview

**FR-6.1** Before any share, the app shall show the user a preview of exactly what will leave.

**FR-6.2** Ephemeral overrides applied at share time shall be communicated to the user as
ephemeral before the share is committed (precise UI treatment is a design decision).

### 3.7 Audit trail

**FR-7.1** A live audit trail shall be a core feature, present whether or not any AI is connected.

**FR-7.2** The audit trail shall record, at minimum:
- what was detected (fields, classifications);
- what the user approved vs. redacted;
- what was exported/shared and to whom (recipient is a user-entered note in v1);
- what was sent to the AI plugin;
- import events and per-document retention decisions;
- confirmation that no private originals left the device (where applicable).

**FR-7.3** The audit trail shall give the user a verifiable answer to "what did I actually share?"
by person or by AI, without requiring trust in the app on faith.

**FR-7.4** The audit trail shall be inspectable by the user at any time while the vault is
unlocked.

### 3.8 Account and unlock

**FR-8.1** The user shall have an account bound to an on-device key model for unlock-credential
binding. (Backup, cross-device sync, and mediated sharing enabled by the account are later
phases, not v1 — see §8.)

**FR-8.2** First run shall let the user create an account and set an unlock passphrase, and shall
generate an encryption key that never leaves the machine.

**FR-8.3** The app shall unlock the vault locally; no network identity is required for day-to-day
vault access.

### 3.9 Plugin model

**FR-9.1** The plugin surface shall have three parts:
1. Output consumers — take an approved document and do something with it.
2. Detectors — add sensitive-field recognizers to the redaction engine.
3. New flows — orchestrate across documents.

**FR-9.2** Plugins may be user-invoked (user picks one and runs it on chosen documents) or
event-triggered, opt-in (a plugin reacts automatically on import/redact/share with the user's
explicit opt-in per plugin).

**FR-9.3** v1 shall ship the Cloud AI plugin as the showcase output consumer.

**FR-9.4** v1 shall ship the detector and new-flow surfaces as empty extension points that are
plugin-capable but not exercised by first-party plugins, and reactive hooks likewise empty.

**FR-9.5** The v1 architecture shall not preclude opening third-party plugins in a later phase
without architectural rework. This is an architectural constraint (verified by the separate
Architecture Specification), not a v1 user-facing/acceptance-test requirement.

---

## 4. Non-functional requirements

### 4.1 Security
- **NFR-S1** All content (originals if retained, approved versions, metadata) shall be
  envelope-encrypted at rest.
- **NFR-S2** The encryption key shall not leave the device.
- **NFR-S3** No plaintext document content or metadata shall be recoverable from a data file
  without the unlock step.
- **NFR-S4** Redacted fields shall never leave the device in any share. Exported files shall
  not contain redacted text or metadata recoverable from the file (e.g., redaction must not be
  a visual-only overlay leaving the underlying text stream intact).

### 4.2 Privacy
- **NFR-P1** Detection shall run on-device; document content shall not leave the device for
  detection in v1. Same bounded exception as FR-2.3 (decision 0009): a strictly loopback,
  IP-literal, non-DNS, non-proxied call to the optional local Ollama backend, per architecture
  §10.
- **NFR-P2** Only approved content shall be transmitted in any share; the app shall not transmit
  redacted fields.
- **NFR-P3** The app shall not host or relay user content, nor hold the encryption key
  off-device, at the v1 stage.

### 4.3 Performance / responsiveness
- **NFR-PERF1** Detection and approval review shall remain interactive for the document sizes in
  v1 scope (born-digital text/PDF with extractable text). Concrete thresholds are a design
  decision, out of scope here.

### 4.4 Reliability / data integrity
- **NFR-R1** The audit trail shall be durable across app restarts and shall be tamper-evident
  (any post-creation modification of an entry shall be detectable by a defined integrity
  mechanism). The specific mechanism (e.g., append-only log, hash chain, signatures) is a
  design decision.
- **NFR-R2** Deletion of vault contents shall be irrevocable.

### 4.5 Usability
- **NFR-U1** The user shall be able to answer "what did I share, and to whom?" from the UI
  without external tooling.
- **NFR-U2** Pre-share previews and ephemeral-override warnings shall be comprehensible to a
  non-technical user (cf. Aisha persona).

### 4.6 Portability
- **NFR-PORT1** The app shall be a desktop application; precise supported OSes are a design
  decision.

### 4.7 Extensibility
- **NFR-E1** Detector, new-flow, and reactive plugin hooks shall be present in v1 such that
  third-party plugins can be opened up later without architectural rework. This is an
  architectural constraint (verified by the Architecture Specification), not a v1 acceptance-test
  requirement.

---

## 5. Data requirements

The schema that satisfies D-2 and D-4 (tables, envelopes, identifiers) is
[`data-model.md`](./data-model.md). This SRS states *what* must be stored, not columns.

- **D-1** Inputs: born-digital text and PDFs with extractable text.
- **D-2** Stored artifacts: approved versions; retained originals (when retained); named
  variants; audit-trail entries; account + on-device key material.
- **D-3** Transmitted artifacts: approved content only, to the Cloud AI plugin; redacted export
  files handed off by the user.
- **D-4** All stored document content and metadata is encrypted at rest.

---

## 6. Constraints

- **C-1** v1 is first-party plugins only; the only first-party output consumer is the Cloud AI
  plugin.
- **C-2** v1 is export-only for share-to-people; no app-mediated delivery.
- **C-3** v1 AI outputs are read-only text; no agentic action.
- **C-4** No hosting/relaying of user content and no off-device key custody at the v1 stage.
- **C-5** Local-first; detection on-device (bounded loopback-only exception for the optional
  Ollama backend, decision 0009 / architecture §10).

---

## 7. Acceptance criteria (derived, not exhaustive)

- **AC-1** A user can import a born-digital PDF, see detected fields with labels and locatable
  spans, make per-field keep/redact decisions, and store a canonical approved version encrypted.
- **AC-2** A user can export a redacted file (single document or multi-document bundle) for a
  person, having applied an ephemeral override and seen a pre-share preview, and the export is
  logged in the audit trail; the exported file contains no recoverable redacted text.
- **AC-3** A user can run the Cloud AI plugin on selected approved documents and receive
  read-only text output; only approved content is sent; the share is logged.
- **AC-4** A user can open the audit trail and determine, per document, what was detected,
  approved, redacted, exported, and sent to AI, and confirm no redacted field left the device.
- **AC-5** With the vault locked, a stolen data file cannot yield plaintext content or metadata.
- **AC-6** A user can set a paranoid "never retain originals" default and confirm per-import
  overrides cannot loosen it to retain (see
  [decision 0002](../decisions/0002-resolved-srs-clarifications.md)).
- **AC-7** On a fresh vault, the retention default is discard and unconfirmed; the first import
  is refused until the user sets a policy; the first-upload prompt pre-selects discard
  ([decision 0007](../decisions/0007-retention-default-discard.md)).

---

## 8. Out of scope (v1) — named later phases

These are explicitly not part of this SRS's binding requirements; they are listed to preserve
alignment with `docs/idea.md`.

- App-mediated share-to-people (links, view controls, revocation).
- OCR for scanned documents and images.
- Agentic AI action-taking (file, send, submit) on the user's behalf.
- Third-party plugins.
- First-party detector and new-flow plugins filling the empty hooks.
- Reactive/event-triggered plugins exercised.
- Multi-user / team mode.
- Vault backup / restore and reinstall re-attachment (continue with existing vault data after
  reinstall; not passphrase recovery; distinct from FR-5.1 PDF export). See idea.md later
  phases.
- Any feature requiring Privacy Gate to host/relay user content or hold the key off-device.

---

## 9. Traceability to idea doc

| idea.md concern | SRS coverage |
|---|---|
| Local-first, consent-aware redaction vault | §2.1, §3.3, §4.1, C-5 |
| Import + retention decision + global default | §3.1 (FR-1.3/1.4), dec 0002 (Q9), dec 0007 |
| On-device detection, labeled locatable spans | §3.2, dec 0002 (Q10), OQ-16 |
| Per-field approve → one canonical approved version | §3.3, dec 0002 (Q8) |
| Encrypted storage, envelope encryption, on-device key | §3.4, §4.1 |
| Share-to-person (export-only) vs share-to-AI (read-only text) | §3.5 (FR-5.1/5.2), C-1..C-3, dec 0002 (Q11, Q4-bundle), OQ-12 |
| Ad-hoc overrides ephemeral + named variants | §3.5 (FR-5.4/5.5), §3.6, OQ-7 |
| Pre-share preview | §3.6 |
| Audit trail core | §3.7, NFR-R1, NFR-U1, OQ-3, OQ-6 |
| Plugin model (3 parts; user-invoked or opt-in event) | §3.9 |
| v1 first-party only; Cloud AI showcase; hooks present but empty | FR-9.3/9.4, C-1, OQ-12, OQ-13 |
| Account + on-device key, no network needed day-to-day | §3.8, OQ-5, OQ-18 |
| Out-of-scope list | §8, C-1..C-4 |
| Vault backup / restore, reinstall re-attachment (later) | §8 |
| Out-of-scope hosting/relay/key-off-device | §8, C-4, NFR-P3 |

---

## 10. Open questions and resolved clarifications

Gaps surfaced while writing this SRS (and by the three-model review — see
`docs/notes/reviews/`) are **not** tracked inside this spec. Per the knowledge-governance skill,
resolved clarifications live in decision records and open questions live in a separate register.

- **Resolved clarifications** (one canonical approved version; paranoid-default semantics;
  manual redaction out of v1; export = true removal; multi-doc single PDF bundle):
  [`docs/decisions/0002-resolved-srs-clarifications.md`](../decisions/0002-resolved-srs-clarifications.md).
- **Open questions** (register; OQ-4 save-dialog chrome resolved in ui.md):
  [`docs/notes/open-questions.md`](../notes/open-questions.md). Architecture-, API-,
  testing-, and UI-owned items are resolved in [`architecture.md`](./architecture.md),
  [`api.md`](./api.md), [`testing.md`](./testing.md), [`ui.md`](./ui.md), and decisions
  0003–0008 (review roster: decision 0005).

These are not requirements. Downstream specs and idea.md amendments resolve them; SRS-era
items are recorded as resolved in the register.