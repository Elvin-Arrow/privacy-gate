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
