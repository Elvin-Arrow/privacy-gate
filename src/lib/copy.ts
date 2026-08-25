// Canonical copy (ui.md §15, §5.1, §13.1). Kept in one module so every screen quotes the
// same string instead of independently retyping it (and drifting on a future edit).

/** ui.md §5.1 (C-ARCH-7), shown below the passphrase field on first run and repeated on
 * unlock's failure/lost-passphrase copy per §13.2 ("Only the sentence in §5.1 / §5.2."). */
export const NO_RECOVERY_COPY =
  'Privacy Gate cannot reset this passphrase. If you forget it, this vault cannot be opened. There is no recovery email or backup code in this version.'

/** ui.md §15: `unlock_failed` — same wording for unknown/wrong passphrase. */
export const UNLOCK_FAILED_COPY = 'Could not unlock. Check the passphrase.'

/** ui.md §13.1: `degraded_integrity` screen title. */
export const INTEGRITY_TITLE = 'This vault cannot open documents'

/** ui.md §13.1: `degraded_integrity` screen body. */
export const INTEGRITY_BODY =
  'Privacy Gate checked the audit trail and found it does not match. Documents will not be decrypted. You can save a verification report. Restoring from your own backup is the recovery path; Privacy Gate cannot repair a tampered vault.'

/** ui.md §13.1: default filename for the integrity report save dialog. */
export const INTEGRITY_REPORT_FILENAME = 'privacy-gate-integrity-report.json'

// ---------------------------------------------------------------------------
// W31 — Settings (ui.md §11, §15, §6's retention table)
// ---------------------------------------------------------------------------

/** ui.md §11.2 client-side check: new/confirm mismatch. Deliberately distinct wording
 * from `PASSPHRASE_CURRENT_WRONG_COPY` below — this is a same-form typo, not a rejected
 * credential (see docs/dev-log/0043-w31-ui-settings.md "Ambiguities"). */
export const PASSPHRASE_CONFIRM_MISMATCH_COPY = "New passphrase and confirmation don't match."

/** api.md §3: `passphrase_mismatch` means the *current* passphrase the user typed is
 * wrong — not a new/confirm mismatch. Distinct copy so the two are never conflated. */
export const PASSPHRASE_CURRENT_WRONG_COPY = 'Current passphrase is incorrect.'

/** ui.md §6's table, reused verbatim by §11.3 ("Same three policies as §6"). */
export const RETENTION_POLICY_LABELS = {
  discard: 'Discard originals after approval (recommended)',
  retain: 'Keep encrypted originals by default',
  never_retain: 'Never keep originals (cannot keep on a single file)',
} as const

/** ui.md §11.4, strict paraphrase. */
export const CLOUD_AI_SCOPE_COPY =
  'Sharing sends only the approved content you choose to the host you configure here. Detection never uses this endpoint.'

/** ui.md §15: shown before an AI share preview when Cloud AI has not been configured yet
 * (kept here for reuse by a future share-flow chunk, per the brief's steer). */
export const CLOUD_AI_NOT_CONFIGURED_COPY =
  'Cloud AI is not configured. Add an endpoint and key in Settings before asking a model.'

// ---------------------------------------------------------------------------
// W32 — Vault / first-import modal / import (ui.md §6, §7, §15; decision 0007)
// ---------------------------------------------------------------------------

/** ui.md §6: blocking first-import modal title. */
export const RETENTION_MODAL_TITLE = 'Choose a default for original files'

/** ui.md §6: blocking first-import modal body, verbatim. */
export const RETENTION_MODAL_BODY =
  'Before the first import, choose what Privacy Gate should do with original files after you approve a redacted version. You can change the default later in Settings. For a single import you can keep or discard differently, unless you choose "never keep originals."'

/** ui.md §7.2: compact per-import override control labels, mapped to
 * `import_document.retention_override`. */
export const RETENTION_OVERRIDE_LABELS = {
  default: 'Use default',
  retain: 'Keep original',
  discard: 'Discard original',
} as const

/** ui.md §15 `retention_loosen_forbidden`, shown when the compact override's "Keep
 * original" is clicked (or would apply) while the global default is `never_retain`. */
export const RETENTION_LOOSEN_FORBIDDEN_COPY =
  'The default is "never keep originals," so this file cannot be kept. Change the default in Settings if you want to keep originals going forward.'

/** ui.md §15 `over_budget`. */
export const OVER_BUDGET_COPY =
  'This file is larger than the size Privacy Gate is tuned for (25 MB). Import will finish; it may take longer than usual.'

/** ui.md §15 `unsupported_document`. */
export const UNSUPPORTED_DOCUMENT_COPY =
  'Privacy Gate v1 only imports text and PDFs that already contain text. Scanned pages and photos are not supported yet.'

/** ui.md §7.2: `retention_policy_unset` "should not happen if §6 ran" — treated as a UI
 * bug, but still needs comprehensible copy per the brief's error-mapping requirement. */
export const RETENTION_POLICY_UNSET_COPY =
  'Could not import: the retention default was not confirmed. Please try importing again.'

/** ui.md §7.2 client-side + core `invalid_input` (empty/path-like filename). */
export const IMPORT_INVALID_INPUT_COPY =
  'This file could not be imported: its name is not valid.'

/** ui.md §7.1: empty vault-list state. */
export const VAULT_EMPTY_STATE_COPY = 'No documents yet. Import a document to get started.'

/** ui.md §7.1 Delete row action confirm (FR-4.6, "irrevocable"), reusing §15's "Delete
 * document" copy. */
export const DELETE_DOCUMENT_CONFIRM_COPY =
  'This deletes the approved version, any kept original, and variants. It cannot be undone.'

// ---------------------------------------------------------------------------
// W33 — Approval / consent (ui.md §8, §2.3, §15)
// ---------------------------------------------------------------------------

/** ui.md §8 / §2.3 primary action. */
export const APPROVE_AND_STORE_LABEL = 'Approve and store'

/** ui.md §8 Cancel (calls `abort_approval`). */
export const APPROVAL_CANCEL_LABEL = 'Cancel'

/** ui.md §2.3 keep/redact segment labels — words, never colour alone (NFR-U2). */
export const KEEP_LABEL = 'Keep'
export const REDACT_LABEL = 'Redact'

/** Screen title from the §8 mockup; §8 itself doesn't name a heading string. */
export const APPROVAL_TITLE = 'Review before approving'

/** Shown while `lifecycle === "awaiting_decisions"`. */
export const APPROVAL_PENDING_COPY = 'Every detected field needs a Keep or Redact decision.'

/** Shown when `lifecycle === "decided"`. */
export const APPROVAL_DECIDED_COPY = 'All fields decided'

/** Open on an already-approved document (api.md `already_approved`). Further changes
 * are share-time overrides (W34), not re-approval (dev-plan W33: "Do not: re-approve
 * after commit"). */
export const ALREADY_APPROVED_COPY =
  'This document is already approved. Further changes are made when you share it.'

/** api.md `approval_busy`: one RAM session per process. */
export const APPROVAL_BUSY_COPY =
  'Another document is already being reviewed. Finish or cancel that review first.'

// ---------------------------------------------------------------------------
// W34 — Share preview / save dialog (ui.md §10, §15)
// ---------------------------------------------------------------------------

/** Mockup / §10 heading. */
export const SHARE_TITLE = 'Share preview'

/** ui.md §2.3 / §10.4 hero action and save-dialog title. */
export const SAVE_REDACTED_PDF_LABEL = 'Save redacted PDF'

/** ui.md §15 FR-6.2 ephemeral override — banner, never a toast. */
export const EPHEMERAL_OVERRIDE_COPY =
  'These changes apply to this share only. The approved version in your vault will not change.'

/** ui.md §15 `preview_expired`. */
export const PREVIEW_EXPIRED_COPY =
  'This preview expired. Generate a new preview before exporting.'

/** After commit succeeds but the scoped write fails (ui.md §10.4 step 5). */
export const RETRY_SAVE_LABEL = 'Retry save'

export const SHARE_WRITE_FAILED_COPY =
  'The export is recorded in the audit trail. You can retry saving the file.'

// ---------------------------------------------------------------------------
// W35 — Audit trail (ui.md §12, §15)
// ---------------------------------------------------------------------------

/** Mockup / §12 heading and primary-nav label. */
export const AUDIT_TITLE = 'Audit trail'

/** ui.md §12 empty / filtered table. */
export const AUDIT_EMPTY_FILTER_COPY = 'No audit events match this filter.'

/** Event types in words (ui.md §12). Share rows use the kind-specific labels below. */
export const AUDIT_EVENT_TYPE_LABELS = {
  import: 'Imported',
  detect: 'Detected',
  approve: 'Approved',
  share: 'Shared',
  discard_original: 'Discarded original',
  delete: 'Deleted',
} as const

/** ui.md §12 share-row reading level (NFR-U2). */
export const AUDIT_SHARE_EXPORT_LABEL = 'Exported PDF'
export const AUDIT_SHARE_AI_LABEL = 'Asked Cloud AI'

export const AUDIT_ORIGINALS_GONE_COPY = 'No originals remained on device'
export const AUDIT_ORIGINALS_KEPT_COPY = 'Original kept on device'

// ---------------------------------------------------------------------------
// W36 — Variants + Cloud AI share confirm (ui.md §9, §10, §15)
// ---------------------------------------------------------------------------

export const VARIANTS_TITLE = 'Variants'

/** ui.md §9 empty state, verbatim. */
export const VARIANTS_EMPTY_COPY =
  'No saved variants for this document. Customize keep/redact during share preview and save them as a variant to reuse later.'

/** ui.md §9 / design §3.4 — no in-place edit. */
export const VARIANT_NO_EDIT_COPY = 'To change this, delete it and save a new variant.'

export const MANAGE_VARIANTS_LABEL = 'Manage variants'
export const SAVE_VARIANT_LABEL = 'Save as variant'
export const DELETE_VARIANT_LABEL = 'Delete variant'

export const VARIANT_NAME_CONFLICT_COPY =
  'A variant with that name already exists for this document. Choose a different name.'

export const EXPORT_PDF_LABEL = 'Export PDF'
export const ASK_CLOUD_AI_LABEL = 'Ask Cloud AI'
export const SEND_TO_AI_LABEL = 'Send to Cloud AI'
export const AI_PREVIEW_LABEL = 'Preview'

/** ui.md §15 AI confirm — visible before commit, not a toast. */
export const AI_CONFIRM_COPY =
  'Only the approved, redacted text shown in the preview will be sent to the host you configured.'

export const SHARE_AI_FAILED_COPY =
  'The attempt is recorded in the audit trail. You can change the instruction and preview again.'

/** In-page CTA distinct from AppShell's Settings nav item. */
export const OPEN_SETTINGS_LABEL = 'Open Settings'
