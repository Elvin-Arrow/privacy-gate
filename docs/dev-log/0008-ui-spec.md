# [0008] UI spec generation and Claude+Gemini review

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Produce the v1 UI spec: Svelte 5 webview, screens, copy, CSP/capabilities, save-dialog
chrome (OQ-4 remainder), first-paint budgets, and UI-layer tests. Review with Gemini and
Claude only (decision 0005).

## Implementation

- Recorded [decision 0008](../decisions/0008-frontend-svelte.md) (Svelte 5 + Vite; small
  compiled runtime in the untrusted webview).
- Drafted `docs/specs/ui.md`. Reviewed with Gemini (`agy --effort high`) and Claude
  (`claude -p`). Raw notes: `docs/notes/reviews/ui-gemini.md`, `ui-claude.md`.
- Reconciled: Settings includes account / passphrase / retention / Cloud AI; first-import
  order is modal then picker; scoped `plugin-fs` write; blob-URL teardown; first-paint test
  row; architecture C-ARCH-2 extended to `get_integrity_report` JSON. OQ-4 closed.

## Problems encountered

- Dual first-import orderings (Claude): picker-before vs after confirm.
- Settings section covered only Cloud AI (Gemini) while nav promised Settings.
- Save exception silently covered integrity-report JSON (Claude).
- C-TEST-8 first-paint had no test row (Claude).
- `plugin-fs` write grant unnamed (Gemini).

## Resolution

- Deterministic sequence: `set_retention_default` then file picker.
- Settings §11.1–11.4. C-ARCH-2 + api.md §8 name both persist payloads.
- §16 first-paint row (jsdom fake clock). Scoped write-only fs grant. Progressive field
  list after 200 rows.

## Related documentation

- [Spec — UI](../specs/ui.md)
- [Decision 0008](../decisions/0008-frontend-svelte.md)
- [Raw reviews](../notes/reviews/)
