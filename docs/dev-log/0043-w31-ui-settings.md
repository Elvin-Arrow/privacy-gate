# [0043] W31 — UI: Settings (account, passphrase, retention, Cloud AI form)

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Build the single Settings screen (ui.md §11): read-only account fields, passphrase
change, the global retention default, and the Cloud AI configuration form — the four
subsections and nothing more (dev-plan W31: "Do not: invent extra settings"). W27 (Cloud
AI core) was already done, so this landed as one pass rather than split around it. Full
TDD per `CLAUDE.md`, on top of the Vitest + Testing Library setup W30 stood up.

## Implementation

- **`src/lib/api.ts`** (extended): typed wrappers for `get_account`, `change_passphrase`,
  `get_retention_default`, `set_retention_default`, and the four `cloud_ai_*` commands,
  plus their DTOs, cross-checked field-for-field against `core/src/session.rs`
  (`GetAccountOut`, `ChangePassphraseIn`/`Out`, `RetentionDefaultOut`,
  `SetRetentionDefaultIn`, `CloudAiSetConfigIn`/`Out`, `CloudAiGetConfigOut`,
  `CloudAiClearConfigOut`, `CloudAiTestOut`) and `core/src/config.rs`'s `RetentionPolicy`
  (`#[serde(rename_all = "snake_case")]` → `"retain" | "discard" | "never_retain"` on the
  wire). Added `passphrase_mismatch` and `cloud_ai_not_configured` to the `ErrorCode`
  union. Same `invoke(name, { input })` convention W30 confirmed from `commands.rs`'s
  `cmd1!`/`cmd0!` macros — no second calling convention introduced.
- **`src/lib/copy.ts`** (extended): `PASSPHRASE_CONFIRM_MISMATCH_COPY` (client-side
  new/confirm typo) kept deliberately distinct from `PASSPHRASE_CURRENT_WRONG_COPY`
  (server `passphrase_mismatch` — current passphrase rejected); `RETENTION_POLICY_LABELS`
  (the exact §6 table strings, reused per §11.3's "same three policies as §6");
  `CLOUD_AI_SCOPE_COPY` (§11.4 paraphrase); `CLOUD_AI_NOT_CONFIGURED_COPY` (§15, not yet
  consumed — kept here for the future share-flow chunk that will need it rather than
  duplicating it later).
- **`src/lib/AppShell.svelte`** (new): the shared unlocked chrome — brand, primary nav
  (**Vault**, **Audit trail**, **Settings**), Lock — extracted from W30's inline markup in
  `VaultScreen.svelte` so `SettingsScreen` doesn't fork a second copy. See "Nav/chrome
  decision" below for why Audit trail stays non-interactive.
- **`src/screens/VaultScreen.svelte`** (edited): now renders `<AppShell active="vault" …>`
  instead of its own inline header; unchanged behavior/text otherwise. Gained an
  `onNavigateSettings` prop.
- **`src/screens/SettingsScreen.svelte`** (new, one file — see "File organization" below):
  four sections in one component, each independently fetched `onMount` (`get_account`,
  `get_retention_default`, `cloud_ai_get_config`) and independently submitted:
  - **Account**: `<dl>` of display name / account ID / formatted `created_at` — no
    `<input>` anywhere in this section, so there is structurally no editable account
    field. `created_at` formatted with `Intl.DateTimeFormat(undefined, { dateStyle:
    'medium', timeStyle: 'short' })`, not a raw ISO dump.
  - **Passphrase**: current/new/confirm, all `autocomplete="off"`. Client-side: new `< 8`
    chars or new/confirm mismatch each block submit before any `invoke`. Server
    `passphrase_mismatch` maps to `PASSPHRASE_CURRENT_WRONG_COPY`, never the client-side
    mismatch string — see "Ambiguity resolved" below. `NO_RECOVERY_COPY` repeated verbatim
    below the form (imported from `copy.ts`, not re-typed).
  - **Retention default**: three radios (`RETENTION_POLICY_LABELS` order matches §6's
    table), bound to local state seeded from `get_retention_default`, applied only on an
    explicit **Save default** click (`set_retention_default`) — the dev-plan's
    explicitly-named test. No client-side block on selecting `retain` while the fetched
    default is `never_retain` (§11.3 explicitly allows the global-default change; only
    `import_document`'s per-import loosen is forbidden, and that's a different screen's
    concern entirely).
  - **Cloud AI**: endpoint/model/API-key inputs feed `cloud_ai_set_config`;
    `cloudAiApiKeyInput` is reset to `''` in a `finally` block immediately after that call
    settles — success or failure — so the typed key cannot survive the round trip in
    either component state or the bound `<input>`'s DOM value. The read side
    (`cloud_ai_get_config`) only ever populates `endpoint_host`/`model`/`key_last4`; there
    is no state variable in the component capable of holding a full key from a `get`.
    **Test** calls `cloud_ai_test` only. **Clear** is a two-state confirm
    (`cloudAiConfirmingClear`) — first click reveals "Yes, clear" / "Cancel"; only the
    second click calls `cloud_ai_clear_config`.
- **`src/App.svelte`** (edited): added local `view: 'vault' | 'settings'` state — not a
  `SessionState` (api.md §2 has no such state), so kept as a small sibling reactive
  variable rather than stretched into the session state machine. Resets to `'vault'` on
  every fresh `unlock` and on `lock`. `unlocked` now renders `VaultScreen` or
  `SettingsScreen` based on `view`, each wired to flip the other way via a callback prop.

## Tests

All new/extended, run via `npm run test`:

- `SettingsScreen.test.ts` (14, new): Account — `get_account` values render and no
  `<input>` on the screen ever holds the account ID (broadest form of "no editable account
  fields" available without inventing a narrower query). Passphrase — new/confirm
  mismatch blocks submit with no `change_passphrase` call; new `< 8` chars blocks submit
  with no call; server `passphrase_mismatch` shows the distinct current-passphrase copy
  and explicitly asserts the client-side mismatch copy is *not* shown; non-recovery
  sentence present; a valid submit calls `change_passphrase` with the exact wire shape.
  Retention — parametrized over all three policies, each selecting the radio and clicking
  **Save default** asserts the matching `set_retention_default` call; a separate test
  starts the fetched default at `never_retain` and proves the `retain` radio is not
  `disabled` and a save still goes through (the explicitly-named "not wrongly imported"
  check). Cloud AI — a distinctive fabricated key string is typed, submitted, and then
  asserted absent from both `container.innerHTML` and the API-key input's own `.value`
  after the mocked `cloud_ai_set_config`/`cloud_ai_get_config` round trip settles; a
  separate test seeds `cloud_ai_get_config` with a full fabricated key baked into the mock
  response shape (which the DTO doesn't actually carry — this proves the component
  couldn't render it even if a future core regression added it) and asserts it never
  appears in `innerHTML`, only `key_last4`; clicking **Test** fires exactly one `invoke`
  call, `cloud_ai_test`; clicking **Clear** once does not call `cloud_ai_clear_config` and
  shows the confirm prompt, only the follow-up **Yes, clear** click calls it.
- `App.test.ts` (+1 → 6): clicking **Settings** from the unlocked Vault chrome renders
  `SettingsScreen`'s Account heading and the mocked `get_account` display name — the
  dev-plan's "Settings nav" integration point, proven end-to-end through `App.svelte`'s
  routing rather than only unit-testing `SettingsScreen` in isolation.

39 Vitest tests total (24 from W30 + 15 new), all green. `npm run check` (svelte-check): 0
errors — one round of fixing needed (see "Ambiguity resolved"). `cargo test --workspace
--jobs 2`: unchanged pass counts from W30 (this chunk touched no Rust), 0 regressions.

## Nav/chrome decision

Per the brief's explicit steer, `AppShell` renders **Audit trail** as a plain non-clickable
`<span>`, not a `<button>` or link — that screen is a later chunk and a dead link (or worse,
a link into faked content) would be worse than omitting interactivity. **Vault** and
**Settings** are real buttons that flip `App.svelte`'s `view` state. The existing Lock
control moved from `VaultScreen`'s inline header into `AppShell` so it isn't duplicated once
a second screen needs it — matching the brief's note that it "should logically move into
this shared chrome."

## File organization

Kept `SettingsScreen.svelte` as one file rather than splitting into
`src/screens/settings/*` sub-components. Each subsection is a handful of fields and one
submit handler; the shared pieces (the `onMount` fetch pattern, the card/`<dl>` styling)
would be the only thing extracted, and W30 didn't set a precedent for splitting a
single-screen chunk that size (its four screens were each already separate top-level
concerns — first run, unlock, integrity, vault — not subsections of one screen). If a
later chunk needs to reuse the Cloud AI form specifically (e.g. from a first-time-AI-share
prompt), it can be extracted then with a real second call site to shape it around.

## Ambiguities resolved

- **`passphrase_mismatch` semantics.** api.md §3 states plainly: "`change_passphrase`
  current passphrase wrong." This is unambiguous and distinct from the client-side
  new/confirm equality check the brief flagged as a trap — implemented as two entirely
  separate copy constants (`PASSPHRASE_CONFIRM_MISMATCH_COPY` vs.
  `PASSPHRASE_CURRENT_WRONG_COPY`) and a test that asserts the wrong one is *not* shown
  when the server rejects, mirroring the trap-avoidance pattern.
- **Retention-loosen distinction (global default vs. per-import).** ui.md §11.3 states it
  directly: "Changing `never_retain` → `retain` is allowed (api.md: global change, not a
  per-import loosen). Per-import keep is still forbidden while the default is
  `never_retain`." This screen only ever calls `set_retention_default` (the global-default
  command); `import_document`'s `retention_override` and its `retention_loosen_forbidden`
  rejection live entirely in the (unbuilt) import flow. No client-side block was added
  here, and a test pins that a fetched `never_retain` default doesn't disable the `retain`
  radio on this screen.
- **`svelte-check` typing bug in the test file itself.** A first pass typed
  `CLOUD_AI_UNCONFIGURED`/`RETENTION_DISCARD` as bare object literals and then wrote
  `mockMount`'s override parameters as `typeof CLOUD_AI_UNCONFIGURED`; TypeScript narrowed
  the literal's `null` fields to the literal type `null` (not `string | null`), so a test
  override supplying real strings for a *configured* Cloud AI mock failed to typecheck.
  Fixed by explicitly annotating the constants with the real `RetentionDefaultOut` /
  `CloudAiGetConfigOut` types imported from `api.ts`, so the override parameter types are
  the actual DTO shapes, not an inferred literal.

## Traceability

- ui.md §4 ("Chrome when unlocked: … Settings"), §11 (all four subsections), §15 (canonical
  copy table), §6 (retention policy labels reused by §11.3).
- api.md §5.1 (`get_account`, `change_passphrase`), §5.2 (`get_retention_default`/
  `set_retention_default`), §5.7 (`cloud_ai_set_config`/`_get_config`/`_clear_config`/
  `_test`), §3 (`passphrase_mismatch` semantics).
- dev-plan.md W31 ("retention confirm calls `set_retention_default`"; "API key not stored
  in DOM after set returns"; "Integrate: Settings nav"; "Do not: invent extra settings").

Next: W32 — UI: vault, first-import modal, import.
