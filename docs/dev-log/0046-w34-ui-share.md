# [0046] W34 — UI: share, preview, save dialog (OQ-4)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Build the person-export share screen (ui.md §10): always `preview_share` before
`commit_share` (C-UI-3), PDF preview from in-memory `pdf_bytes` via a blob URL + iframe,
FR-6.2 warning as a persistent banner (not a toast) when `overrides_in_effect`, then the
OS **save** dialog (OQ-4 / C-ARCH-2). Cancel never commits. Write fail after a successful
commit offers Retry save without a second `commit_share`. Default name is
`suggested_filename`, not `source_filename`. Full TDD on the Vitest + Testing Library
setup from W30–W33.

Person-export only this chunk. Ask Cloud AI confirm, variant picker, and share-time
override editing are W36.

## Implementation

- **`src/lib/api.ts`** (extended): `ShareKind`, `ShareRequestDto`, `ShareManifestEntry`,
  `SharePreview`, `CommitShareOut`, plus `previewShare` / `commitShare`. Wire JSON is
  snake_case (`kind: "export_to_person"`). `pdf_bytes` is `number[] | null` because Tauri
  JSON-serializes `Vec<u8>`. Added `not_approved` and `preview_expired` to `ErrorCode`.
  Same `invoke(name, { input })` convention as W29–W33.
- **`src/lib/copy.ts`** (extended): `SHARE_TITLE` ("Share preview"),
  `SAVE_REDACTED_PDF_LABEL` (hero + dialog title, ui.md §10.4), `EPHEMERAL_OVERRIDE_COPY`
  (ui.md §15 FR-6.2, verbatim), `PREVIEW_EXPIRED_COPY`, `RETRY_SAVE_LABEL`,
  `SHARE_WRITE_FAILED_COPY`. Removed `SHARE_NOT_YET_AVAILABLE_COPY` — the W33 approved-row
  placeholder is gone.
- **`src/lib/tokens.css`**: `--md-warning` / `--md-warning-container` /
  `--md-on-warning-container` for the FR-6.2 banner (ui.md §2.2 extended warning role).
- **`src/screens/ShareScreen.svelte`** (new): `preview_share` on mount with
  `kind: "export_to_person"`, empty override/variant maps (canonical approved decisions
  only). PDF iframe `title="Redacted PDF preview"` from `URL.createObjectURL`; revoked
  on unmount and after a successful write. Manifest lists field **ids** only (C-API-2).
  Save: `plugin-dialog` `save` (not `open`) with default path
  `join(documentDir(), suggested_filename)`, PDF filter. Null path → return, no
  `commit_share`. Path chosen → `commit_share` then `plugin-fs` `writeFile` of the commit
  `pdf_bytes`. Write fail keeps those bytes in RAM, swaps the hero for **Retry save**
  (dialog again, no second commit). `preview_expired` on commit shows
  `PREVIEW_EXPIRED_COPY` and rebuilds via a new `preview_share`. Cancel is `onDone`
  (reuses `APPROVAL_CANCEL_LABEL`); it does not call `commit_share`.
- **`src/screens/VaultScreen.svelte`** (edited): Open on `has_approved_version` calls
  `onOpenShare(doc_id, source_filename)` instead of the W33 placeholder. Unused
  `.open-placeholder` CSS removed. Unapproved Open still goes to approval.
- **`src/App.svelte`** (edited): `view` gained `'share'`; `shareDocId` / `shareFilename`
  carry the vault row into `ShareScreen`. Reset on lock, unlock, and `onDone`.

## Tests

`src/screens/ShareScreen.test.ts` (8, new): **dialog cancel → no commit** (dev-plan's
explicitly named test) — `save` returns `null`, `commit_share` and `writeFile` are not
called, screen stays on preview, defaultPath uses `suggested_filename` not
`source_filename`. **Confirm → commit then write** — `commit_share` after `preview_share`,
`writeFile` gets the commit `pdf_bytes`, blob URL revoked, "Saved out.pdf" status.
**FR-6.2 warning visible** — `overrides_in_effect: true` paints `EPHEMERAL_OVERRIDE_COPY`
in a `role="status"` banner while Save stays enabled; false does not show it. Write-fail
→ Retry save without a second commit; `preview_expired` rebuilds without writing; blob
URL revoked on unmount. jsdom has no `URL.createObjectURL`; tests stub it.

`src/screens/VaultScreen.test.ts`: Open on an approved row calls `onOpenShare` with that
`doc_id` and filename, not `onOpenApproval`.

`src/App.test.ts` (+1 → 8): Open on an approved vault row lands on the Share preview
heading — the navigation seam proven through `App.svelte`. Mocks for `plugin-dialog`,
`plugin-fs` (`writeFile`), and `@tauri-apps/api/path`.

81 Vitest tests total (72 from W30–W33 + 9 new), all green. `npm run check`
(svelte-check): 0 errors / 0 warnings. No Rust changes; `cargo test` not re-run.
This repo's Docker-only, no-display environment cannot launch the Tauri webview, so
the component tests plus typecheck are the available verification for "UI tests
green"; a manual pass of the save-dialog slice is deferred to a human with a display.

## Ambiguities resolved

- **Scope is person-export.** ui.md §10 also specifies Ask Cloud AI and share-time
  overrides/variants. Dev-plan W34's named tests and "Do not: open dialog" are the
  save-dialog chrome; W36 owns AI confirm + variants. Preview always sends empty
  `per_doc_overrides` / `applied_variant_ids`. The FR-6.2 banner is still proven by
  mocking `overrides_in_effect` — W33's `KeepRedactControl` is not reused yet.
- **Save button exists before preview resolves.** The topbar always paints Save, disabled
  until `preview_share` returns. Tests that waited only for the button's *existence*
  asserted the iframe/warning against a still-empty pane. `loaded()` now waits for Save
  **enabled**.
- **`loadPreview` vs `preview_expired` copy.** Rebuilding the preview used to clear
  `actionError`, which hid `PREVIEW_EXPIRED_COPY` as soon as the new preview landed.
  Reload no longer clears `actionError`; save / retry still do.
- **`Uint8Array` is not a `BlobPart` under current TypeScript.** `new Blob([uint8])`
  failed svelte-check (`Uint8Array<ArrayBufferLike>` vs `ArrayBufferView<ArrayBuffer>`).
  Preview copies bytes into a fresh `ArrayBuffer` first. `writeFile` still takes a
  `Uint8Array`.

## Traceability

- ui.md §10.2 (preview iframe + manifest ids, FR-6.2 banner not a toast, `preview_expired`
  rebuilds), §10.4 (save not open; `suggested_filename`; documents folder; cancel = no
  commit; commit then write; retry without second commit; blob teardown), §15, §16, §3.3
  (blob URL revoke), C-UI-3, C-ARCH-2 / NFR-S4.
- api.md §5.6 (`preview_share` / `commit_share`), §4 (`ShareKind`, `ShareRequestDto`).
- FR-6.1, FR-6.2.
- OQ-4 save-dialog chrome.
- dev-plan.md W34 ("fake dialog cancel → no commit"; "confirm → commit then write mock";
  "FR-6.2 warning visible"; "Do not: core writing files; open dialog").

Next: W35 — UI: audit + integrity failure.
