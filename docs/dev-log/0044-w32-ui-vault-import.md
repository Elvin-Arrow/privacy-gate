# [0044] W32 — UI: vault, first-import modal, import

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Replace the W30/W31 one-line Vault placeholder with the real vault list (`list_documents`)
and the import write path (`import_document`), gated by decision 0007's blocking
first-import retention modal (ui.md §6) and the per-import override control (§7.2). Full
TDD on top of the Vitest + Testing Library setup from W30/W31.

## Implementation

- **`src/lib/api.ts`** (extended): `ImportDocumentIn`/`Out`, `DocumentSummary`,
  `ListDocumentsOut`, `GetDocumentOut`, `DeleteDocumentOut`, `DetectProgressEvent`,
  `SourceFormat`, `EffectiveRetention`, and the `importDocument`/`listDocuments`/
  `getDocument`/`deleteDocument` wrappers — cross-checked field-for-field against
  `core/src/session.rs`'s `DocumentSummary`/`ImportDocumentIn` and
  `core/src/importer.rs`'s `SourceFormat` (`#[serde(rename_all = "snake_case")]` →
  `"text" | "pdf"`). Added `retention_policy_unset`, `retention_loosen_forbidden`,
  `unsupported_document` to `ErrorCode`. `DETECT_PROGRESS_EVENT` payload typed from api.md
  §6's table (`{ doc_id, fraction, phase }`); only `fraction` is rendered this chunk.
- **`src/lib/copy.ts`** (extended): §6 modal title/body verbatim, §7.2's compact-control
  labels, and §15's `retention_loosen_forbidden`/`over_budget`/`unsupported_document`
  copy, plus a `retention_policy_unset` and client-side invalid-filename string not given
  verbatim by §15 (plain-language equivalents, per the brief). Reused
  `RETENTION_POLICY_LABELS` from W31 rather than re-forking the three policy strings.
- **`src/lib/RetentionModal.svelte`** (new): the blocking §6 modal — Discard pre-selected,
  Continue/Cancel. Kept as its own component, separate from the compact override control,
  because the two have different triggers (blocking vs. inline-always-visible-once-
  confirmed) and different target commands (`set_retention_default` vs.
  `import_document.retention_override`); sharing one component behind a `mode` prop would
  have meant branching most of the template for reuse of only the three label strings,
  which already live in `copy.ts`.
- **`src/lib/RetentionOverrideControl.svelte`** (new): the §7.2 "Later imports" compact
  control (Use default / Keep original / Discard original). "Keep original" is `disabled`
  rather than hidden when the fetched default is `never_retain`; clicking it while disabled
  still surfaces `RETENTION_LOOSEN_FORBIDDEN_COPY` inline rather than doing nothing.
- **`src/screens/VaultScreen.svelte`** (rewritten): `list_documents` + `get_retention_default`
  on mount. Import affordance is a drop zone (`role="group"`, ui.md's a11y baseline) plus
  a hidden `<input type="file" accept=".pdf,.txt,text/plain,application/pdf">` triggered
  by an "Import a document" button — the button/drop handler check `retentionConfirmed`
  first and only open the picker / act on a drop once confirmed (§6 steps 4–5); an
  unconfirmed drop is stashed (`pendingDropFile`) and replayed after the modal's Continue
  resolves, rather than discarded (so the user isn't forced to re-drag). `startImport`
  reads the file via `FileReader.readAsArrayBuffer` (see "jsdom gotcha" below), converts to
  a plain `number[]` for the `Vec<u8>` wire shape, and calls `import_document` with
  `{ filename, bytes, retention_override }`. Progress: `listen('pg://detect-progress', …)`
  set up before the `import_document` call and torn down in `finally`, same pattern
  `App.svelte` established for `pg://session-changed`. `over_budget === true` sets a
  persistent notice and does not discard the just-imported row. The four documented error
  codes (`unsupported_document`, `retention_policy_unset`, `retention_loosen_forbidden`,
  `invalid_input`) map through `mapImportError` to `copy.ts` strings; unknown codes fall
  back to the server's own `message`. Row actions: **Open** (see "§7.3 gap" below) and
  **Delete**, gated by an inline confirm (`deleteConfirmDocId`), mirroring W31's Cloud-AI-
  Clear two-click pattern (`Delete` → confirm text + `Yes, delete`/`Cancel` → `Yes, delete`
  fires `delete_document`).
- **`src/App.test.ts`** (edited): the three existing tests that reach the unlocked/Vault
  state now also mock `list_documents` (and, where not already present, `get_retention_default`)
  — `VaultScreen`'s new `onMount` fetches would otherwise hit the tests' "unexpected
  command" throw. No test *behavior* changed, only its mock surface.

## §7.3 gap-handling decision (Approval screen not built yet)

ui.md §7.3 says: "If `has_approved_version === false`, navigate to Approval
(`open_approval`)." The Approval screen is W33, not this chunk. Building a stub Approval
screen just to satisfy this literally would mean shipping fake/incomplete UI a reviewer
could mistake for the real thing — explicitly disallowed by the brief.

**Decision:** no auto-navigation on import success. The newly-imported (or any other
unapproved) row stays visible in the vault list with `has_approved_version: false` shown
in its **Approved** column. Its **Open** action is present (not hidden, not a dead
link) and, on click, toggles an inline `"Approval screen not yet available."` line under
that row rather than navigating anywhere or calling `open_approval`. This mirrors how
`AppShell` (W31) renders "Audit trail" as a non-interactive label for the same
not-yet-built-screen reason, adapted here to a still-clickable action because Delete lives
in the same action cluster and Open needed to be visibly present, not silently absent.
W33 closes this gap by wiring `handleOpen` to call `open_approval` and navigate instead of
toggling the placeholder.

## Path-separator validation: defense-in-depth, not the sole guard

Confirmed by reading `core/src/session.rs`'s `validate_import_filename` (called from
`import_document` before any detection or catalog write): the core **already** rejects an
empty filename or one containing `/` or `\` with `invalid_input`, independent of the UI.
`VaultScreen.svelte`'s `basenameIsValid` check therefore is defense-in-depth: it fails
fast on an adversarial `File.name` (simulated in a test by constructing a `File` with a
`../../etc/evil.txt`-style name) without a round trip to the core, and keeps the picker/
drop state intact, but it is not the only thing standing between a malformed name and
`import_document` — the core would reject it either way. The UI does not attempt to
"strip and continue" a path-like name (an earlier reading of ui.md §7.2's "strip path" was
considered and rejected): it rejects outright, matching the core's own behavior exactly,
so no separator-bearing name is ever sent either from the UI check or from the core.

## jsdom gotcha: `File.prototype.arrayBuffer`

jsdom's `File` (used by `@testing-library/svelte` under `environment: 'jsdom'`) does not
implement `arrayBuffer()` — only Node's *global* `File` (a different constructor) does.
`file.arrayBuffer()` in `startImport` threw `TypeError: file.arrayBuffer is not a function`
in every import test until switched to `FileReader.readAsArrayBuffer`, which both jsdom and
a real webview support identically. Left a comment at the call site so a future edit
doesn't revert to `.arrayBuffer()` and silently break only under test.

## Tests

`src/screens/VaultScreen.test.ts` (19, new): blocking-modal show/hide on
`confirmed` false/true; Discard pre-selected; **Continue calls `set_retention_default`
before `import_document`** (asserted via call-index order, not just both having
happened — the dev-plan's explicitly named test); **Cancel does not import** and leaves no
modal/`set_retention_default` call; file selection calls `import_document` with the exact
`{ filename, bytes, retention_override }` shape (`bytes` asserted to be a non-empty
`number[]`); an adversarial `File.name` with `../` is rejected before `import_document` is
ever called; a `vi.mock('@tauri-apps/plugin-fs', () => { throw … })` at module scope proves
the import flow never imports that module at all (dev-plan's explicitly named "no
`plugin-fs` read" test — the throw would fail the whole suite if anything in the import
path imported it); progress bar reflects a synthetic `pg://detect-progress` event (mocked
`listen`, same pattern as `App.test.ts`); `over_budget` shows the §15 copy and the document
still appears in the refreshed list; all four documented error codes map to their `copy.ts`
strings (parametrized); empty-state shows the import prompt with no rows/table; populated
rows show the documented columns in API order (newest-first is the API's job, asserted via
DOM row order matching mock order) with no span text or field labels, only a numeric
`detected_field_count`; a successful import causes a second `list_documents` call and the
new row appears; re-importing an existing filename shows no "already imported"/"duplicate"
text anywhere (negative-space test); Delete requires the confirm step before
`delete_document` fires.

`src/App.test.ts` (unchanged count, 6): three existing tests' mocks extended with
`list_documents`/`get_retention_default` so `VaultScreen`'s new mount-time fetches don't
hit their "unexpected command" throw.

58 Vitest tests total (39 from W30/W31 + 19 new), all green. `npm run check`
(svelte-check): 0 errors (one round of fixing needed — see below). `cargo test --workspace
--jobs 2`: unchanged pass counts (23 total across `pg-core`/`privacy-gate`), 0 regressions
— this chunk touched no Rust. `cargo build -p privacy-gate`: clean, confirming the whole
Tauri binary still compiles/links with these frontend changes (this repo's Docker-only,
no-display environment cannot literally launch the app and drive a real file-open dialog,
so this build plus the component-level tests above are the strongest available proxy for
dev-plan's "Done when: … one real txt/PDF import on a dev machine" — that literal manual
step is deferred to a human with a display, and no manual launch was performed or claimed
here).

One `svelte-check`-adjacent a11y warning was hit and fixed during implementation: Svelte's
compiler flagged the drop zone's `<div>` (`dragover`/`dragleave`/`drop` handlers with no
ARIA role) — fixed with `role="group"` and an `aria-label`, not suppressed.

## Traceability

- ui.md §6 (blocking first-import modal, decision 0007), §7.1 (vault list columns/empty
  state/row actions), §7.2 (import picker, drag-and-drop, progress, error codes), §7.3
  (after-import — gap documented above), §15 (canonical copy), §16 (first-import modal
  test requirement), §17 C-UI-1/C-UI-2/C-UI-4.
- api.md §5.2 (`get_retention_default`/`set_retention_default`, reused from W31), §5.3
  (`import_document`/`list_documents`/`get_document`/`delete_document`), §6
  (`pg://detect-progress`).
- architecture.md §12 (import reads `File` bytes in memory, never `plugin-fs`).
- dev-plan.md W32 ("fake modal: Continue sets policy before import"; "cancel does not
  import"; "no `plugin-fs` read"; "Do not: duplicate-file warning").

Next: W33 — UI: approval (two-pane approval screen; this chunk's `handleOpen` placeholder
is the seam W33 replaces with real navigation to `open_approval`).
