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
