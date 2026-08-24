# [0042] W30 — UI: first run, lock, unlock

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

First frontend chunk in this codebase: `src/` had only the W0 scaffold (`main.ts` + a
near-empty `App.svelte`) and no test runner. Build the `first_run` → create-account,
`locked` → unlock, and lock screens (ui.md §5) plus the minimal `degraded_integrity`
screen (ui.md §13.1) unlock's own routing can land on, wired to the real W29 IPC surface.
Full TDD applies to this frontend code per `CLAUDE.md`, so this chunk also stands up
Vitest + Testing Library from nothing.

## Implementation

- **Test runner**: `vitest` + `@testing-library/svelte` + `@testing-library/jest-dom` +
  `jsdom` as devDependencies; `@tauri-apps/plugin-fs` added as a runtime dependency
  (`@tauri-apps/plugin-dialog` was already present from W29). `vitest.config.ts` (separate
  from `vite.config.ts`, which is Tauri's dev/build config and must not gain a test
  runner's settings) sets `environment: 'jsdom'` and a setup file
  (`src/test/setup.ts`) that imports `@testing-library/jest-dom/vitest` matchers and
  calls Testing Library's `cleanup()` in `afterEach` (needed because the config runs with
  `globals: false`, so there is no framework-provided auto-cleanup hook). `npm run test`
  runs `vitest run`.
- **`src/lib/api.ts`** (new): typed `invoke` wrappers for `get_session_state`,
  `create_account`, `unlock`, `lock`, `get_integrity_report`, plus `SessionState`,
  `ApiError`/`isApiError`, `IntegrityReport` types and the `SESSION_CHANGED_EVENT` name
  constant. Field names are the exact wire shapes straight off `core/src/session.rs`'s
  DTOs — those derive `Serialize`/`Deserialize` with no `rename_all = "camelCase"` (only
  the `SessionState` enum has `rename_all = "snake_case"`, matching its own variants), so
  JSON keys stay snake_case (`display_name`, `account_id`, …); Tauri does not auto-camelCase
  struct fields on its own. Each gated command's Tauri argument name is `input` (confirmed
  from `src-tauri/src/commands.rs`), so calls are `invoke('create_account', { input })`.
- **`src/lib/copy.ts`** (new): the canonical strings this chunk needs — the C-ARCH-7
  non-recovery sentence (ui.md §5.1, repeated verbatim on both First Run and Unlock per
  §13.2: "Only the sentence in §5.1 / §5.2"), `unlock_failed`'s ui.md §15 copy, and the
  §13.1 integrity-screen title/body/report filename. One place so every screen quotes the
  same text.
- **`src/screens/FirstRunScreen.svelte`** (new): display name / passphrase / confirm
  fields, `autocomplete="off"` on both passphrase fields. Client-side validation
  (trimmed-empty name, passphrase `< 8` chars, mismatch) runs and blocks `invoke` entirely
  before `create_account` is ever called. `account_exists` (or any other) failure shows
  `ApiError.message` verbatim (ui.md §5.1: "Failed `create_account` shows
  `ApiError.message`"). Success calls an `onSuccess` callback prop with the raw
  `CreateAccountOut`.
- **`src/screens/UnlockScreen.svelte`** (new): single passphrase field,
  `autocomplete="off"`, no forgot-passphrase/reset/recover control anywhere in the markup.
  On `unlock_failed` specifically, shows the ui.md §15 canonical copy ("Could not unlock.
  Check the passphrase.") rather than the core's raw `ApiError.message` — see "Ambiguity"
  below for why those two differ. Success calls `onUnlocked` with the full `UnlockOut`
  (`state` + `integrity`), leaving the state → screen decision to the caller.
- **`src/screens/IntegrityScreen.svelte`** (new, minimal per the brief): §13.1 title, body,
  **Save report**, **Lock** — nothing else, no "open anyway" path. Save report calls
  `get_integrity_report`, opens the `@tauri-apps/plugin-dialog` **save** dialog (default
  filename `privacy-gate-integrity-report.json`, JSON filter), and on a chosen path writes
  the report via `@tauri-apps/plugin-fs`'s `writeTextFile` directly in this component — see
  "§10.4 scope decision" below.
- **`src/screens/VaultScreen.svelte`** (new, placeholder only per dev-plan W30: "No vault
  list yet"): minimal chrome (brand, Vault/Audit trail/Settings nav labels, a Lock button)
  over a one-line empty-state message. No real vault-list logic — that is a later chunk.
- **`src/App.svelte`** (rewritten, Svelte 5 runes): `onMount` calls `get_session_state`
  once and switches on `SessionState` with a plain `{#if}` chain — no router dependency for
  four screens, per the brief. Also subscribes to `pg://session-changed` (cheap, and W29
  already emits it) but the listener only ever updates `sessionState`, never `integrity`:
  the event payload is `SessionStateOut` (`{ state }` only, no `IntegrityReport`), so a
  `degraded_integrity` transition detected only through the event would have no report to
  show. Both `handleAccountCreated`/`handleUnlocked` set state from the **direct invoke
  response** instead, which does carry `integrity`; the listener exists for
  keeping-in-sync/future out-of-band transitions, not as this chunk's primary navigation
  path — matching the brief's steer toward the simpler, sufficient choice. `document.title`
  is set to `Privacy Gate — Locked` only when `sessionState === 'locked'`, `Privacy Gate`
  otherwise (ui.md §3.3: never a third value, never document content).
- **`src/lib/tokens.css`** (new): the Material Design 3 custom properties this chunk's
  screens use, trimmed from `design/mockups/_tokens.css.txt`. No Google Fonts `<link>` (the
  mockups import Roboto from a CDN; C-UI-7 forbids that in the shipped app) — a system font
  stack (`--md-font`) stands in. Imported once from `src/main.ts`.
- **`src/main.ts`**: switched `new App({ target })` to Svelte 5's `mount(App, { target })`
  — see "Ambiguity" below.
- **CI / Makefile**: `.github/workflows/ci.yml` gained a "Run frontend unit tests" step
  (`npm run test`) right after "Run frontend typecheck," in the same job, no new Linux
  system dependencies (jsdom needs no real webview/GTK). `Makefile` gained `make test-ui`
  (`docker compose run --rm dev npm run test`); `help` and `CLAUDE.md`'s command list
  updated to mention `npm run test`.

## Tests

All new, `src/**/*.test.ts`, run via `npm run test` (Vitest + Testing Library), `invoke`/
`listen`/`save`/`writeTextFile` mocked at the module level — no real Tauri runtime:

- `FirstRunScreen.test.ts` (7): C-ARCH-7 copy present; both passphrase fields
  `autocomplete="off"`; mismatch blocks submit with no `invoke` call; `< 8` chars blocks
  submit with no `invoke` call; empty/whitespace-only name blocks submit with no `invoke`
  call; `account_exists` surfaces `ApiError.message` verbatim; a valid submit calls
  `create_account` with the exact wire shape and then `onSuccess`.
- `UnlockScreen.test.ts` (6): no forgot/reset/recover control anywhere among every
  interactive (`link`/`button`/`menuitem`) element's accessible name, and no bare `<a>`/
  `<button>` whose own text matches either — a broad search, not just "we didn't add a
  button named X"; passphrase field `autocomplete="off"`; `unlock_failed` shows the exact
  ui.md §15 string; C-ARCH-7 copy present; success calls `onUnlocked` with `state:
  "unlocked"`; success also calls `onUnlocked` with `state: "degraded_integrity"` and the
  full `IntegrityReport` passed through untouched.
- `IntegrityScreen.test.ts` (6): §13.1 title/body present; no "open anyway"/"open
  documents" control by the same broad interactive-element search; exactly two buttons
  exist (Save report, Lock) — a positive assertion that nothing extra was added; Lock
  calls its callback; Save report fetches the report, opens the save dialog with the
  correct default filename, and writes the exact JSON to the chosen path; a cancelled save
  dialog (`null` path) never calls `writeTextFile`.
- `App.test.ts` (5): first-run and locked chrome each render (heading-role query) after
  exactly one `invoke` call (`get_session_state`) — the ui.md §14 first-paint-budget proxy
  this chunk's brief asked for, read as "no other awaited call gates the chrome," not a
  wall-clock assertion; unlock → `"unlocked"` reaches the Vault placeholder; unlock →
  `"degraded_integrity"` reaches the Integrity screen and — the ui.md §16-named test —
  neither the Vault empty-state text nor Vault-only chrome (a "Vault" nav label) exists
  anywhere in the DOM afterward, only Integrity's own Lock button; locking from the Vault
  placeholder returns to the Unlock screen and flips `document.title` to `Privacy Gate —
  Locked`.

24 Vitest tests, all green. `npm run check` (svelte-check) green, 0 errors. `cargo test
--workspace --jobs 2` (W29's noted OOM workaround still needed): 416 `pg-core` + 13
`privacy-gate` tests unchanged, 0 regressions — this chunk touched no Rust.

## Ambiguities resolved

- **`new App(...)` broke under svelte-check once `App.svelte` used runes.** The W0 scaffold
  compiled fine with `new App({ target })` because the old `App.svelte` used plain
  top-level `let` bindings (Svelte 5's "legacy" reactivity mode), which is still
  class-constructable. Rewriting it with `$state`/`$props`/`$effect` (runes mode, needed
  for the routing this chunk adds) makes the component no longer legacy-constructable;
  `svelte-check` failed with "Expected 2 arguments, but got 1" / implicit `any` on `new
  App(...)`. Fixed by switching `main.ts` to Svelte 5's `mount(App, { target })` API,
  which is the documented runes-mode entry point — confirmed by reverting the chunk's
  changes (`git stash`) and re-running `npm run check`, which came back clean on the old
  scaffold, proving the break was specifically the runes/legacy mismatch and not a
  pre-existing issue.
- **`unlock_failed` copy: canonical string vs. `ApiError.message`.** `core/src/api.rs`'s
  `ApiError::unlock_failed()` sets `message: "unlock failed"` — non-secret, but not the
  ui.md §15 canonical "Could not unlock. Check the passphrase." string. `account_exists`'s
  spec line says show `ApiError.message` directly; `unlock_failed`'s spec line separately
  names an exact canonical copy. Resolved by branching on `ApiError.code`:
  `UnlockScreen` shows the canonical §15 string specifically for `code === 'unlock_failed'`
  and falls back to the raw message for any other error class, so the two spec
  instructions don't collide.
- **§10.4 scope decision (integrity report save dialog).** §13.1 says "same save-dialog
  rules as §10.4," but §10.4's full sequence (retry-on-write-failure UX, blob-URL revoke,
  `commit_share` coupling) is written for the PDF-export flow that doesn't exist until a
  later chunk, and its own scaffolding (a shared save-dialog helper) doesn't exist yet
  either. Per the brief's explicit steer, implemented the minimal subset directly in
  `IntegrityScreen.svelte`: open `plugin-dialog`'s `save` with the JSON filter and default
  filename, no-op on cancel (no report ever fetched into a discarded state), write via
  `plugin-fs`'s `writeTextFile` on a chosen path. No shared save-dialog helper was
  extracted — a future chunk touching share/export can factor one out once there are two
  real call sites to generalize from, rather than guessing its shape now.
- **`pg://session-changed` listener: wire it, but don't let it drive navigation.** The
  brief left this to judgment. Wired `listen(SESSION_CHANGED_EVENT, ...)` in `App.svelte`
  (cheap, W29 already emits it, and not wiring it at all would mean a future out-of-band
  transition is silently missed) but deliberately keep the screens' own callback props
  (`onSuccess`/`onUnlocked`/`onLock`) as the actual navigation path for this chunk's three
  flows, since the event payload cannot carry `IntegrityReport` and using it for the
  `degraded_integrity` transition specifically would either drop the report or require a
  second `get_integrity_report` round trip the direct response already avoids.

## Traceability

- ui.md §4 (screens/navigation state machine), §5 (first run/unlock/lock), §13.1
  (`degraded_integrity`), §14 (first-paint budget), §15 (canonical copy), §16 (UI test
  layer requirements), §17 (C-UI-1/2/5/6/7).
- api.md §5.1 (`get_session_state`/`create_account`/`unlock`/`lock`/`get_integrity_report`
  In/Out shapes, `ApiError`), §2 (session table, referenced not re-implemented).
- srs.md FR-8.1/8.2/8.3.
- dev-plan.md W30 ("screens ui.md §5 … no vault list yet … Vitest: mismatch; min length 8;
  no 'forgot password' control; `unlock_failed` copy").

Next: W31 — UI: Settings (account, passphrase, retention, Cloud AI form).
