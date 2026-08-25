# [0045] W33 — UI: approval

- **Status:** Complete
- **Date:** 2026-08-24

## Objective

Build the two-pane approval / consent screen (ui.md §8): locatable spans in the document
text, a keep/redact control per detected field that is not colour-only (NFR-U2), first
paint of the first page + first 200 field rows (ui.md §14), and **Approve and store**
enabled only when `lifecycle === "decided"`. Navigate here from the vault (Open on an
unapproved row, and automatically after a successful import — the §7.3 gap W32 left).
Full TDD on the Vitest + Testing Library setup from W30–W32.

## Implementation

- **`src/lib/api.ts`** (extended): `FieldDecisionKind`, `ApprovalLifecycle`,
  `DetectedFieldDto` / `DetectedFieldSpanDto`, `FieldDecisionDto`, `ApprovalPage` /
  `ApprovalPageSpan`, `ApprovalView`, `SetFieldDecisionsOut`, `SubmitApprovalOut`,
  `AbortApprovalOut`, plus `openApproval` / `getApprovalView` / `setFieldDecisions` /
  `submitApproval` / `abortApproval`. Cross-checked field-for-field against
  `core/src/session.rs` (`ApprovalView`, `DetectedFieldDto`, `FieldDecisionKind`'s
  `#[serde(rename_all = "snake_case")]` → `"keep_visible" | "redact"`). Same
  `invoke(name, { input })` convention. Added `already_approved`, `approval_busy`,
  `approval_bad_state` to `ErrorCode`.
- **`src/lib/copy.ts`** (extended): `APPROVE_AND_STORE_LABEL` / `APPROVAL_CANCEL_LABEL`
  / `KEEP_LABEL` / `REDACT_LABEL` (ui.md §8 / §2.3); screen title `APPROVAL_TITLE`
  ("Review before approving" — §8 doesn't name a heading, the mockup does);
  pending/decided status lines; `ALREADY_APPROVED_COPY` / `APPROVAL_BUSY_COPY` (plain
  language for api.md codes §15 doesn't give verbatim); `SHARE_NOT_YET_AVAILABLE_COPY`
  for the approved-row Open placeholder until W34.
- **`src/lib/approvalLayout.ts`** (new): splits a page's concatenated spans into
  highlight segments so nested fields stay visible (design §3.5). Innermost covering
  field owns the overlapping slice; parents still render around it. Byte offsets are
  mapped through UTF-8 so a multibyte character can't shift a later field.
- **`src/lib/KeepRedactControl.svelte`** (new): the §2.3 segmented control — open-eye /
  crossed-eye icons **and** the words Keep / Redact, selected Keep using
  `tertiary-container`, selected Redact using `error-container`, undecided with neither
  `aria-pressed`. Extracted because share-time overrides (W34) will want the same
  control.
- **`src/screens/ApprovalScreen.svelte`** (new): `open_approval` on mount; left pane is
  `ApprovalView.pages` with in-place span buttons (undecided = dashed underline, kept =
  solid underline, redacted = diagonal hatch + underline); right pane is one row per
  field (label + classification + KeepRedactControl), nested rows indented from
  `parent_field_id`. Incremental `set_field_decisions`; **Approve and store** (hero)
  disabled until `lifecycle === "decided"`, then `submit_approval` and `onDone`;
  **Cancel** calls `abort_approval` then `onDone`. First 200 field rows paint with the
  view; the rest after a `setTimeout(0)` so first paint is not blocked on the full list
  (ui.md §8 / §14). Selecting a field row highlights the matching span and vice versa.
  No original-download control. Leaving the screen (unmount, or Cancel before
  `open_approval` returns) aborts the RAM session so a later Open isn't `approval_busy`.
- **`src/screens/VaultScreen.svelte`** (edited): Open on `has_approved_version === false`
  calls `onOpenApproval(doc_id, source_filename)` instead of the W32 placeholder. Open
  on an already-approved row does **not** call `open_approval` (dev-plan: "Do not:
  re-approve after commit") and shows the share placeholder until W34. A successful
  import of an unapproved document also calls `onOpenApproval` (ui.md §7.3).
- **`src/App.svelte`** (edited): `view` gained `'approval'` (still not a `SessionState`);
  `approvalDocId` / `approvalFilename` carry the vault row into `ApprovalScreen`. Reset
  on lock, unlock, and `onDone`.
- **`src/lib/tokens.css`**: `tertiary-container` / `on-tertiary-container` /
  `surface-container-highest` / `radius-xs` — the Keep-selected and undecided-span
  tokens §2.2 specifies that earlier chrome chunks hadn't needed.

## Tests

`src/screens/ApprovalScreen.test.ts` (10, new): **Approve disabled until decided**
(dev-plan's explicitly named test) — two fields, Keep on the first leaves the button
disabled, Redact on the second enables it, then **Approve and store** fires
`submit_approval` with the session id and `onDone`. **First-paint fake clock** (ui.md
§16 / §14) — 250 fields, after mocked `open_approval` resolves, first page text +
`Field 0` + `Field 199` are in the document and `Field 200` is not, before 300 ms of
fake time. **Keyboard** — Enter on Keep and Space on Redact fire `set_field_decisions`
without a pointer; ArrowDown moves `aria-pressed` to the next field-select button.
Two-pane layout with Keep/Redact **words** (NFR-U2); nested field label + span text
both present (not hidden inside the parent); list-row click ↔ span `data-selected`;
no Download control; Cancel → `abort_approval` then `onDone`; `already_approved` from
`open_approval` shows `ALREADY_APPROVED_COPY` and does not render Approve.

`src/screens/VaultScreen.test.ts` (+3 → 22): Open on an unapproved row calls
`onOpenApproval` with that `doc_id` and filename; Open on an approved row does not,
and shows the share placeholder; a successful import of an unapproved document
navigates via `onOpenApproval` (§7.3).

`src/App.test.ts` (+1 → 7): Open from the unlocked vault lands on the approval
heading with Approve disabled — the navigation seam proven through `App.svelte`, not
only in isolation.

72 Vitest tests total (58 from W30–W32 + 14 new), all green. `npm run check`
(svelte-check): 0 errors / 0 warnings. No Rust changes; `cargo test` not re-run.
This repo's Docker-only, no-display environment cannot launch the Tauri webview, so
the component tests plus typecheck are the available verification for "UI tests
green"; a manual pass of the consent slice is deferred to a human with a display.

## Ambiguities resolved

- **Keep/Redact inside a listbox option.** A first pass used `role="listbox"` /
  `role="option"` for the field list (keyboard list semantics). Testing Library's
  accessibility tree then **hid the nested Keep/Redact buttons**, which would also
  have been wrong for a real screen reader. Switched to a focusable
  `<button class="field-select">` per row (aria-label = field label, aria-pressed =
  selected) sitting beside the KeepRedactControl buttons, so every control is a
  real button. ArrowUp/ArrowDown still move selection.
- **Cancel vs in-flight `open_approval`.** Cancel lives in the topbar and is
  clickable before the view returns. Clicking it then would `onDone` without
  aborting, and the in-flight open would leave a RAM session (`approval_busy` on the
  next Open). Cancel now sets a `cancelled` flag; if `open_approval` returns after
  that (or after unmount), the session is aborted and the view is not applied.
- **Approved-row Open.** ui.md §7.1 says Open on an approved document goes to share /
  variants. Share is W34. Same gap-handling as W32 used for Approval: the action
  stays visible and shows "Share is not yet available." rather than calling
  `open_approval` (which would be `already_approved`) or faking a share screen.

## Traceability

- ui.md §8 (two panes, locatable spans, keep/redact, Approve when decided, Cancel
  aborts), §2.3 (segmented control, span treatments, hero button), §7.1 / §7.3
  (Open / after-import navigation), §14 / §16 (first paint ≤ 200 fields, fake
  clock; keyboard operable list + keep/redact), §17 C-UI-2.
- api.md §5.4 (`open_approval` / `get_approval_view` / `set_field_decisions` /
  `submit_approval` / `abort_approval`), §4 (`DetectedFieldDto`, `FieldDecisionKind`).
- design.md §3.5 (nested fields stay visible).
- NFR-U2 (not colour-only).
- FR-2.2, FR-3.1.
- dev-plan.md W33 ("Approve disabled until decided"; "first-paint fake clock";
  "keyboard operable list + keep/redact"; "Integrate: navigate from vault"; "Do
  not: re-approve after commit").

Next: W34 — UI: share, preview, save dialog (OQ-4).
