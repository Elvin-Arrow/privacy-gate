# [0048] W36 — UI: variants empty/list + Cloud AI share confirm

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Build the variants list (ui.md §9) and finish the share screen's Ask Cloud AI path
(ui.md §10.1–§10.3 / §15): empty-state copy, save from the share override set, delete
with confirm and no in-place edit, AI confirm copy plus a read-only payload pane
visible before `commit_share`, and **Manage variants** on an approved vault row.
Full TDD on the Vitest + Testing Library setup from W30–W35.

Do not: marketplace, a real Cloud AI host.

## Implementation

- **`src/lib/api.ts`** (extended): `variant_name_conflict`, `cloud_ai_network`,
  `cloud_ai_refused`; `VariantSummary`, `listVariants` / `getVariant` / `saveVariant` /
  `deleteVariant`. Wire JSON is snake_case. Same `invoke(name, { input })` convention.
- **`src/lib/copy.ts`** (extended): `VARIANTS_TITLE`, `VARIANTS_EMPTY_COPY` (ui.md §9
  verbatim), `VARIANT_NO_EDIT_COPY`, `MANAGE_VARIANTS_LABEL`, `SAVE_VARIANT_LABEL`,
  `DELETE_VARIANT_LABEL`, `VARIANT_NAME_CONFLICT_COPY`, `EXPORT_PDF_LABEL`,
  `ASK_CLOUD_AI_LABEL`, `SEND_TO_AI_LABEL`, `AI_PREVIEW_LABEL`, `AI_CONFIRM_COPY`
  (§15 verbatim), `SHARE_AI_FAILED_COPY`, `OPEN_SETTINGS_LABEL`.
- **`src/lib/tokens.css`**: `--md-secondary-container` / `--md-on-secondary-container`
  for the Export PDF / Ask Cloud AI tabs.
- **`src/screens/VariantsScreen.svelte`** (new): `list_variants` on mount. Empty state
  is `VARIANTS_EMPTY_COPY`. Listed rows show name + created time, **Delete variant**
  with Yes/Cancel confirm, and the no-edit copy. No Edit control. Save is not on this
  screen — `save_variant` is from ShareScreen's override set.
- **`src/screens/ShareScreen.svelte`** (edited): tabs for **Export PDF** (W34 path) and
  **Ask Cloud AI**. AI: instruction textarea → **Preview** (`preview_share` with
  `kind: "share_to_ai"`) → `AI_CONFIRM_COPY` plus read-only `ai_payload_preview` →
  **Send to Cloud AI** (`commit_share`) → `output_text`. Do not auto-send on preview.
  `cloud_ai_not_configured` shows canonical copy and **Open Settings**. Keep/Redact on
  manifest field ids (`KeepRedactControl`); deltas go in `per_doc_overrides`. **Save as
  variant** from the current override set; apply-variant `<select>` when
  `list_variants` is non-empty.
- **`src/screens/VaultScreen.svelte`** (edited): approved rows get **Manage variants**
  → `onOpenVariants`. Unapproved rows do not.
- **`src/App.svelte`** (edited): `view` gained `'variants'`; `variantsDocId` /
  `variantsFilename`. Reset on lock, unlock, audit, and `pg://session-changed` to
  `degraded_integrity`.

## Tests

`src/screens/VariantsScreen.test.ts` (2, new): empty state when `list_variants`
returns none (dev-plan named test); list + delete-with-confirm; no Edit button; after
delete, empty copy returns.

`src/screens/ShareScreen.test.ts` (+2 → 10): **AI confirm visible before commit**
(dev-plan named test) — Ask Cloud AI, instruction, Preview paints `AI_CONFIRM_COPY`
and the payload, `commit_share` is not called until Send; **not configured → Settings**.
Existing person-export tests still mock `list_variants`.

`src/screens/VaultScreen.test.ts` (+1): Manage variants on an approved row calls
`onOpenVariants` with that `doc_id` and filename.

`src/App.test.ts` (+1 → 11): Manage variants lands on the variants empty-state
heading — the navigation seam proven through `App.svelte`.

94 Vitest tests total, all green. `npm run check`: 0 errors / 0 warnings. No Rust
changes. This repo's Docker-only, no-display environment cannot launch the Tauri
webview; component tests plus typecheck are the available verification. A manual
pass of the variants/AI-confirm slice is deferred to a human with a Tauri window.

## Ambiguities resolved

- **In-flight person-export preview vs Ask Cloud AI.** Clicking the AI tab before
  `preview_share` for export returned applied that preview onto the AI pane (empty
  payload, no Instruction field). `previewSeq` invalidates stale loads on tab switch
  and on every new `loadPreview`.
- **Two Settings buttons.** AppShell nav is already named Settings. The
  `cloud_ai_not_configured` CTA is **Open Settings** so the accessible names stay
  distinct.
- **AI preview is not automatic.** Empty `ai_instruction` is `invalid_input` from
  core. Switching to Ask Cloud AI clears the export preview and waits for Preview.
  Switching back to Export PDF reloads person-export.
- **Discard-original is not this chunk.** ui.md §7.1 lists it on the document menu;
  W36's named tests are variants empty state and AI confirm. Leave it for a later
  UI polish pass.

## Traceability

- ui.md §9 (empty copy, no edit, save from share overrides, delete with confirm),
  §10.1–§10.3 (two modes, preview before commit, read-only payload, explicit AI
  confirm, `cloud_ai_not_configured` → Settings), §15 (`AI_CONFIRM_COPY`)
- api.md §5.5 / §5.6 (`list_variants` / `save_variant` / `delete_variant`,
  `preview_share` / `commit_share` `share_to_ai`)
- C-UI-3 (always preview before commit); C-API-2 (manifest field ids, no span text
  on `get_variant`)
- Next: W37 — acceptance pack AC-1..AC-7 (`docs/dev-plan.md`)
