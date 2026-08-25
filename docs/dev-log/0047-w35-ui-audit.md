# [0047] W35 — UI: audit + integrity failure

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Build the audit trail screen (ui.md §12) and finish the integrity-failure path (ui.md §13 /
C-UI-5): **Audit trail** is a real primary-nav target, the table answers “what did I share,
and to whom?” without field text, a degraded session cannot reach Vault, and Save report
uses the same save-dialog rules as §10.4. Full TDD on the Vitest + Testing Library setup
from W30–W34.

## Implementation

- **`src/lib/api.ts`** (extended): `EventType`, `AuditEventDto`, `ListAuditEventsIn` /
  `ListAuditEventsOut`, `listAuditEvents`. Wire JSON is snake_case
  (`"discard_original"`, `"export_to_person"`). Same `invoke(name, { input })` convention.
  `payload` is `Record<string, unknown>` — ids/labels only; span text is never read.
- **`src/lib/copy.ts`** (extended): `AUDIT_TITLE`, `AUDIT_EMPTY_FILTER_COPY` (ui.md §12
  verbatim), event-type words, `AUDIT_SHARE_EXPORT_LABEL` / `AUDIT_SHARE_AI_LABEL`
  (“Exported PDF” / “Asked Cloud AI”), originals-remained copy matching the mockup.
- **`src/lib/AppShell.svelte`**: **Audit trail** is a button (was a disabled label in W31).
  `active` gained `'audit'`; `onNavigateAudit` is required.
- **`src/screens/AuditScreen.svelte`** (new): `list_documents` then `list_audit_events`
  (limit 50). Filters for document and event type re-fetch from the start. Table columns:
  time, event in words, document (import `source_filename` or catalog lookup), destination
  (`recipient_note` or `endpoint_host`), share kind, originals flag. Empty/filtered uses
  `AUDIT_EMPTY_FILTER_COPY`. No integrity banner on a healthy trail. `Load more` when
  `next_sequence` is set.
- **`src/screens/IntegrityScreen.svelte`** (edited): save-dialog default path is
  `join(documentDir(), privacy-gate-integrity-report.json)` — same documents-folder rule
  as §10.4. Still only Save report and Lock; no “Open anyway.”
- **`src/App.svelte`**: `view` gained `'audit'`. `pg://session-changed` with
  `degraded_integrity` switches `sessionState` (IntegrityScreen is the `{#if}` branch,
  so Vault is unreachable). Unlock still supplies `IntegrityReport` on the command
  response; Save report fetches `get_integrity_report` itself.

## Tests

`src/screens/AuditScreen.test.ts` (5, new): table shows Imported / Detected / Approved /
Exported PDF and document names, not a field-text dump and not the §13 integrity copy;
share rows show recipient note / endpoint host and originals remained/gone; empty filter
copy; `list_audit_events` input follows Document and Event type selects; smuggled
`payload.text` is not rendered (C-UI-2).

`src/App.test.ts` (+2 → 10): **degraded session cannot navigate to Vault** via
`pg://session-changed` (dev-plan's named integrate) — unlocked vault, event fires
`degraded_integrity`, Integrity title, no vault empty-state, no Vault button, no Open
anyway. **Audit trail nav** from the unlocked chrome lands on the Audit heading.

`src/screens/IntegrityScreen.test.ts`: Save report defaultPath is the documents folder +
canonical filename (fake dialog still: cancel writes nothing; confirm writes JSON).

88 Vitest tests total, all green. `npm run check`: 0 errors / 0 warnings. No Rust changes.
This repo's Docker-only, no-display environment cannot launch the Tauri webview; Vite on
`:5173` stays on the loading screen because `get_session_state` never resolves without
IPC. Component tests plus typecheck are the available verification; a manual pass of the
audit/integrity slice is deferred to a human with a Tauri window.

## Ambiguities resolved

- **`create_account` is not an `EventType`.** ui.md §12 says a new vault still shows
  `create_account` events. The wire enum is only import / detect / approve / share /
  discard_original / delete (api.md §4). The table shows what `list_audit_events`
  returns; the empty-filter copy is used when that set is empty. Do not invent a
  vault-created row.
- **Degraded sessions and the audit table.** api.md allows `list_audit_events` while
  degraded (verified prefix). ui.md §13 is full-screen fail-closed with only Save report
  and Lock. The audit table is therefore unlocked-only; degraded never offers Vault or
  Audit trail chrome.
- **First paint of the empty copy.** Showing “No audit events match this filter.” while
  `list_audit_events` is in flight made tests (and users) see a false empty state.
  Loading starts `true`; the empty copy waits until the first fetch returns nothing.

## Traceability

- ui.md §12 (table, filters, share-row reading level, empty copy, no field text, no
  healthy-integrity banner), §13.1 / C-UI-5 (no Open anyway, Save report JSON), §10.4
  (documents folder), §16 (integrity: no navigation to Vault; save report fake dialog),
  §4 (Audit trail primary nav).
- api.md §5.8 (`list_audit_events`), §6 (`pg://session-changed`), §4 `EventType`.
- FR-7 / NFR-U1.
- dev-plan.md W35 ("degraded session cannot navigate to Vault"; "save report uses fake
  dialog"; "Integrate: `pg://session-changed` to integrity screen"; "Do not:
  repair/restore vault").

Next: W36 — UI: variants empty/list + Cloud AI share confirm.
