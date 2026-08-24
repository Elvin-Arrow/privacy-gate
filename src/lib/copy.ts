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
