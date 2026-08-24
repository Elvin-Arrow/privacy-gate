# Privacy Gate — consent-aware document redaction vault

## One line
A local-first app that detects sensitive content in your documents, lets you approve exactly what
stays hidden per field, stores the approved result encrypted on your own machine, and can share the
approved content to a person or an AI plugin of your choice.

## The problem
People routinely need AI help, or need to share, material they cannot safely hand to a cloud
service or another person unfiltered. Bank statements, medical letters, payslips, contracts,
immigration paperwork. Today the choice is binary. Hand over everything, or get no help and do
the work manually. Most people overshare without understanding what left their machine, or give up
entirely.

Privacy Gate replaces that binary choice with a per-field approval step the user controls.

## What it is
A single-user desktop app, local-first, that acts as a consent-aware redaction vault.

- Import a document. An on-device model identifies sensitive fields and classifies them.
- The user reviews the detected fields and decides for each one. Keep visible, or redact.
- The resulting approved version is stored, encrypted, on the user's own machine.
- The user can then export that approved version to share with another person, or share it to an
  AI (via a plugin) for reasoning.

The vault is the product. AI reasoning is an optional plugin on top, not the core loop.

## The flow

```
Import  →  Detect  →  User approves per field  →  Store encrypted  →  Share (people | AI plugin)
```

At import the user also decides, per document, whether to retain the original (encrypted)
alongside the approved version, or discard the original after the approved version is produced.
A global default sets this retention policy. The factory default is **discard originals**. The
first time the user uploads a document they are asked to set that default (discard is
pre-selected; they may choose retain or a paranoid "never retain originals" policy). After
that, each import can override the default in either direction, except that a global "never
retain originals" paranoid default may not be loosened per-import to retain.

## Documents in scope (v1)
Born-digital text and PDFs with extractable text. Scanned documents and images (OCR) are out of
scope for v1 and revisited later.

## Sharing, two distinct flows
Sharing is not one action. Privacy Gate supports two separate flows with separate redaction
needs.

1. **Share to a person** produces a redacted file the user hands off themselves (email, upload,
   print). v1 is export-only. The app does not mediate delivery. App-mediated sharing (links,
   view controls, revocation) is a later phase.

2. **Share to an AI** sends the approved content to a cloud model (via the Cloud AI plugin) for
   reasoning. Explain in plain language, compare documents, produce a checklist, draft a
   response. v1 AI outputs are read-only text. Agentic action-taking (the AI filing, sending, or
   submitting on the user's behalf) is a named future phase, not v1.

Each document has one canonical approved version, but before any specific share the user can
make ad-hoc overrides (reveal more, or hide more). Overrides are ephemeral by default. The user
can optionally save them as a named variant to reuse next time.

## The plugin model
Privacy Gate's value extends through plugins. The plugin surface has three parts.

1. **Output consumers** take an approved document and do something with it (send to an AI,
   export to a format, post to an API).
2. **Detectors** add new sensitive-field recognizers to the redaction engine (e.g.,
   country-specific tax IDs, medical terminology).
3. **New flows** orchestrate across documents (e.g., compare two documents, draft a cover
   letter) rather than just consume one.

Plugins can be user-invoked (the user picks one and runs it on chosen documents) or
event-triggered, opt-in (a plugin reacts automatically on import, on redact, or on share,
with the user's explicit opt-in per plugin).

v1 is first-party only. The Cloud AI plugin ships as the showcase output consumer. The detector
and new-flow surfaces ship as empty extension points that are plugin-capable but not yet
exercised by first-party plugins, and reactive hooks likewise ship empty. The architecture is
designed so third-party plugins can be opened up later without rework.

## Audit trail
A live audit trail is a core feature, present whether or not any AI is connected. It records
what was detected, what the user approved, what was redacted, and what left the vault and to
whom.

```
Gemma detected account number, address
  → user redacted account number, approved address
    → user exported approved version (PDF)
      → no private originals left the device
```

The trail gives the user a verifiable answer to "what did I actually share?" by person or by
AI, without requiring them to trust the app on faith.

## Trust and security
All content is envelope-encrypted at rest. A key the user unlocks protects the stored
originals (if retained) and approved versions, plus their metadata. A stolen data file is
useless without the unlock step.

Unlock is tied to an account + on-device key model. The user has an account (for future
backup, cross-device sync, and mediated sharing), but the encryption key lives on the device and
is unlocked locally. No network identity is required to open the vault day-to-day.

## Scope

**In v1**
- Import born-digital text and PDFs.
- On-device detection and per-field approval.
- Encrypted local storage of approved versions (and originals if retained), envelope encryption,
  user-unlocked.
- Canonical approved version + ephemeral ad-hoc overrides before a share, optionally saveable.
- Export-based share-to-people.
- Cloud AI plugin (showcase) for read-only reasoning on approved content.
- Audit trail, core.
- Plugin hooks for output consumers, detectors, new flows, and reactive events. Present but
  mostly empty in v1.

**Later phases (named, not v1)**
- App-mediated share-to-people (links, view controls, revocation).
- OCR for scanned documents and images.
- Agentic AI action-taking (file, send, submit) on the user's behalf.
- Third-party plugins.
- Detector and new-flow first-party plugins filling the empty hooks.
- Reactive/event-triggered plugins exercised.
- Multi-user / team mode.
- Vault backup / restore, including reinstall re-attachment: after the app is reinstalled,
  the user can continue with existing vault data (at minimum when OS app-data and keystore
  survive; a deliberate backup/restore path if they do not). This is not passphrase recovery
  and is distinct from share-to-person PDF export.

## Out of scope for this idea
Anything that requires Privacy Gate to host or relay user content, or to hold the user's
encryption key off the device, is out of scope at the idea stage and will be revisited only when
a later phase (mediated sharing, cross-device sync) explicitly calls for it.

See `docs/user-story.md` for a worked example of the idea, following Aisha, a user assembling a
spouse-visa application.