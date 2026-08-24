# UI Specification Review: Privacy Gate v1

Reviewer: Gemini (via `agy --effort high`). Date: 2026-08-23.

(Raw Gemini output from the shared review prompt; command names in the review sometimes
follow the prompt packet, not api.md. Reconciliation used api.md names.)

## J. Top 5 (as received)

1. Broaden Settings to passphrase change and retention default (§11, §4, §6).
2. Name the Tauri 2 `plugin-fs` scoped write grant for save-dialog persist.
3. Specify blob URL lifecycle/teardown on preview regeneration and unmount.
4. Use `get_document` for single-row refresh after approval / delete original.
5. Empty states for variants and audit trail.

Alignment A–I: Gemini reported SRS/design/architecture/API/testing/idea alignment as
healthy; OQ-4 chrome treated as resolved by §10.4.
