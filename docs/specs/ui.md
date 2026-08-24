# UI Specification — Privacy Gate v1

> Scope: the TypeScript webview: framework, screens, interaction, user-visible copy, first-paint
> budgets, webview CSP/capabilities, save-dialog chrome (OQ-4 remainder), and UI-layer tests.
> This spec makes the frontend implementable against [`api.md`](./api.md) without inventing
> commands.
>
> It does **not** specify crypto, SQL, envelope JSON, or Tauri command shapes (those stay in
> architecture / data-model / API). It does **not** replace SRS requirements. §2.2/§2.3 give
> the concrete M3 design tokens and signature components (colors, type, shape, elevation) an
> implementation should match; beyond that, further pixel mockups and custom branding are out
> of v1 unless needed to make a privacy rule testable.
>
> Parent specs: [`srs.md`](./srs.md), [`design.md`](./design.md),
> [`architecture.md`](./architecture.md), [`api.md`](./api.md),
> [`data-model.md`](./data-model.md), [`testing.md`](./testing.md). Framework:
> [decision 0008](../decisions/0008-frontend-svelte.md). Review roster:
> [decision 0005](../decisions/0005-review-claude-gemini.md).
>
> Open questions: [`../notes/open-questions.md`](../notes/open-questions.md).

---

## 1. Purpose

The webview is the only human interface. It is **untrusted** for keys, passphrases after
submit, DEKs, SQLCipher keys, Cloud AI API keys after submit, retained originals, and
**redacted** field text (C-ARCH-1, C-DES-1, C-API-2). It *does* show unapproved document
text and detected span text during the consent step, because that is the work the user is
doing (FR-2.2, FR-3.1). Share preview shows only the redacted artifact (FR-6.1).

If a capability is not a command in [`api.md`](./api.md), the UI cannot do it. Names below
match api.md: `create_account`, `import_document`, `open_approval`, `preview_share`,
`commit_share`, `pdf_bytes`, `suggested_filename`.

---

## 2. Framework and packaging

[Decision 0008](../decisions/0008-frontend-svelte.md): **Svelte 5 + Vite + TypeScript**.

- One Tauri 2 webview. No second window except OS dialogs (HTML file picker in-webview §7.2;
  OS save dialog §10.4).
- No SSR, no service worker, no webview HTTP server except Vite in development.
- Runtime: `@tauri-apps/api` and `@tauri-apps/plugin-dialog` (save only). No
  `@tauri-apps/plugin-http`, `plugin-shell`, or `plugin-fs` **read / open / readdir**.
  Persisting export bytes after a save dialog is §10.4 (write of in-memory `pdf_bytes` to
  the user-chosen path only).
- No CDN fonts, no CDN scripts, no analytics, no error-report SDK (architecture §5.3).
- In-memory router (Svelte stores + a screen enum). Do not use a path-based router that
  looks like URLs with document names.

### 2.1 Design system: Material Design 3

**Material Design 3 (M3) is the primary design guide** for this webview's component
behavior, layout conventions, and interaction patterns. No MD3 web-component library is
mandated (none is named in [decision 0008](../decisions/0008-frontend-svelte.md)); components
are hand-built Svelte, styled with CSS custom properties that carry M3 tokens (color roles,
type scale, shape, elevation).

- **Components:** filled / outlined / text buttons for primary / secondary / tertiary
  actions; an M3 dialog for the retention modal (§6) and confirms (delete, cancel); an M3
  linear progress indicator for `pg://detect-progress` (§7.2); an M3 snackbar for transient,
  non-blocking errors (§15); blocking errors that gate a flow (e.g. `unlock_failed`,
  `passphrase_mismatch`) stay inline on the field, not a snackbar; cards for vault list rows
  (§7.1) and variant rows (§9); a navigation rail or drawer for the **Vault / Audit trail /
  Settings** chrome (§4).
- **Color and state, not color alone:** M3 color roles (`primary`, `error`, `surface`,
  `on-surface`, …) style state, but NFR-U2's requirement that keep/redact (§8) and other
  paired states be distinguishable without color still governs: pair the M3 color role with
  a shape/label difference (filled vs. outlined, an icon, the word "Keep" / "Redact"), never
  color alone.
- **Type scale:** M3's type scale (display/headline/title/body/label) sets heading and body
  hierarchy across screens; no custom scale.
- **Out of scope:** custom branding and marketing surfaces remain deferred (§19); §2.2/§2.3
  below are pixel-accurate enough to implement from directly: treat them as source of truth
  over any earlier, looser reading of this section.

### 2.2 Design tokens

Concrete values, so an implementation matches the reference mockups without re-deriving them.
Colors are given in `oklch()`; a component may resolve them to hex at build time if the
toolchain needs it, but author the source values as oklch (design §-level tokens, not
hand-picked hex).

**Color roles**: one hue (~264, a muted indigo-blue) drives every "cool" role; `error` sits
at hue ~25 (red) and the extended `warning` role (below) at hue ~80 (amber). All accent roles
share roughly the same lightness/chroma pattern as their base (`X-container` ≈ 91–92% L, very
low chroma; `on-X-container` ≈ 22–27% L):

| Role | Value | Used for |
|---|---|---|
| `primary` | `oklch(45% 0.15 264)` | Filled buttons, focus rings, links, selected nav pill icon |
| `primary-dim` | `oklch(38% 0.14 267)` | Gradient stop for the hero button (§2.3); never used flat |
| `on-primary` | `oklch(99% 0 0)` | Text/icon on `primary` |
| `primary-container` | `oklch(91% 0.045 264)` | App-mark tile, dialog info icon badge |
| `on-primary-container` | `oklch(24% 0.09 264)` | Icon/text on `primary-container` |
| `secondary-container` | `oklch(91% 0.018 264)` | Selected nav-rail pill background |
| `on-secondary-container` | `oklch(26% 0.03 264)` | Icon/text on selected nav pill |
| `tertiary-container` | `oklch(91% 0.05 168)` | "Approved" / "Kept" status chips |
| `on-tertiary-container` | `oklch(22% 0.06 168)` | Text/icon on that chip |
| `error` | `oklch(50% 0.19 25)` | Redact-selected segment, delete icon badge |
| `error-dim` | `oklch(42% 0.18 26)` | Gradient stop for the destructive hero button |
| `on-error` | `oklch(99% 0 0)` | Text/icon on `error` |
| `error-container` | `oklch(92% 0.06 25)` | Redact-selected chip background, delete-dialog icon badge |
| `on-error-container` | `oklch(27% 0.1 25)` | Icon/text on `error-container` |
| `warning` *(extended role, not base M3)* | `oklch(58% 0.15 80)` | Icon in the ephemeral-override banner (§10.2) |
| `warning-container` | `oklch(92% 0.07 85)` | That banner's background |
| `on-warning-container` | `oklch(32% 0.09 75)` | Banner text/icon |
| `surface` | `oklch(98.5% 0.004 264)` | Page background |
| `surface-container-lowest` | `oklch(100% 0 0)` | Cards, dialogs, text-field fill |
| `surface-container-low` | `oklch(97.3% 0.004 264)` | Nav rail, PDF-preview stage backdrop |
| `surface-container` | `oklch(95.8% 0.005 264)` | Document-type icon tile fill |
| `surface-container-high` | `oklch(93.8% 0.006 264)` | (reserved, not the dialog fill; see §2.3) |
| `surface-container-highest` | `oklch(91.8% 0.007 264)` | "Needs review" neutral chip, unfocused switch track |
| `on-surface` | `oklch(20% 0.012 264)` | Primary text |
| `on-surface-variant` | `oklch(39% 0.016 264)` | Secondary/meta text, unselected nav icon |
| `outline` | `oklch(56% 0.014 264)` | Default input/button borders |
| `outline-variant` | `oklch(85% 0.008 264)` | Card borders, dividers, table rules |
| `scrim` | `oklch(0% 0 0)` | Dialog backdrop, at 45% mixed with transparent; **flat, never blurred** (§2.3) |

`warning` is a deliberate extension beyond base M3 roles (M3 leaves "extended colors" to the
product): it exists so a **non-blocking, expected** state (the FR-6.2 ephemeral-override
notice) reads differently from an **error** state. Never reuse `error` for something that
isn't a failure or an irreversible action: that's what sends users into a false-alarm read
(a warning banner that looks like a crash).

**Type scale**: Roboto, weights 400/500/700. **Self-host the font files** (e.g. a local
`@font-face` in the compiled CSS, bundled with the webview); the reference mockups load it
from `fonts.googleapis.com`, which is a concession to the design-canvas tool's own CSP and is
**not** permitted in the shipped app (architecture §5.3, C-UI-7, §3.1's CSP has no font-src
carve-out). Fallback stack: `-apple-system, 'Segoe UI', sans-serif`.

| Name | Size / line-height | Weight | Used for |
|---|---|---|---|
| Display-small | 30 / 36px | 500 | First-run "Create your vault" headline |
| Headline-small | 22–24 / 28–32px | 400 | Screen titles, dialog titles (24/32 on chrome, 22/28 in dialogs) |
| Title-large | 18 / 24px | 500 | Empty-state heading |
| Title-medium | 15–16 / 20–22px | 500 | Card titles, brand wordmark |
| Title-small | 14 / 20px | 500 | Section sub-headers (share preview sidebar) |
| Body-large | 15–16 / 22–24px | 400 | Screen subtitles |
| Body-medium | 14 / 20px | 400 | Default body copy |
| Body-small | 12–12.5 / 16–17px | 400 | Meta text, helper text, table cells |
| Label-large | 14 / 20px | 500 | Button labels |
| Label-medium | 11–11.5 / 16px | 500, uppercase, +0.4–1.2px tracking | Section eyebrows, nav labels |

**Shape (corner radius) scale**: one scale, reused everywhere; do not invent one-off radii:

| Token | Value | Used for |
|---|---|---|
| `radius-xs` | 4px | Text-field corners, table-cell chip corners |
| `radius-sm` | 10px | Icon tiles inside cards, small menu items |
| `radius-md` | 16px | Dialog icon badge, option-row cards inside dialogs |
| `radius-lg` | 20–22px | Interior content cards (vault document cards) |
| `radius-xl` | 32px | Floating auth cards, all dialogs |
| `radius-full` | 999px | Every button, chip, nav pill, segmented control |

**Elevation (shadow)**: M3 elevation levels, kept neutral (true black at low opacity; never
tint a shadow toward the accent hue: that reads as a colored glow, not a shadow):

| Level | Value | Used for |
|---|---|---|
| 1 | `0 1px 2px rgba(0,0,0,.15), 0 1px 3px 1px rgba(0,0,0,.08)` | Card hover |
| 2 | `0 1px 2px rgba(0,0,0,.12), 0 4px 10px rgba(0,0,0,.06)` | Open dropdown/context menu |
| 4 (floating card) | `0 8px 24px -8px rgba(0,0,0,.16), 0 2px 6px rgba(0,0,0,.08)` | Unlock/first-run card |
| Dialog | `0 16px 40px -8px rgba(0,0,0,.12), 0 6px 16px rgba(0,0,0,.06)` | Retention/delete dialogs |
| Hero button | `0 6px 14px -6px` (or `0 8px 18px -8px` on the larger auth buttons) of the button's own fill color at ~55% mixed with transparent | The one accent case where a shadow *is* tinted; see §2.3 |

### 2.3 Signature components

- **Navigation rail**: 80px fixed width, `surface-container-low` fill, 1px `outline-variant`
  right border. Top: a 40×40px `radius-md` tile in `primary-container` holding the app mark
  (a padlock glyph, stroke-based, 22px). Below: **Vault / Audit trail / Settings**, each a
  56×32px `radius-full` pill (selected: `secondary-container` fill) over a 12px label; the
  Vault item carries a small numeric badge (`primary` fill, `on-primary` text) showing
  `list_documents` length (§4). Bottom-pinned: the **Lock** control, same pill treatment,
  unselected style always (never shows a "selected" state).
- **Buttons**: M3's filled / outlined / text triad stays for every non-primary action
  (Cancel, Test connection, per-row actions, …). One additional pattern, the **hero button**,
  is reserved for **the single primary action of a screen or dialog**, never more than one
  per view: `linear-gradient(135deg, primary 0%, primary-dim 100%)`, `on-primary` text,
  `radius-full`, and a soft shadow tinted from the same color (§2.2's elevation table) instead
  of a neutral one. A destructive hero button (only the "Delete document" confirm) swaps
  `primary`/`primary-dim` for `error`/`error-dim`. Concretely: **Unlock**, **Create your
  vault**, **Import** (both the topbar and empty-state entry points, same action, same
  button), **Approve and store**, **Save redacted PDF** / the AI-share commit action, and
  **Continue** / **Delete document** inside a dialog all get the hero treatment; every other
  button on those same screens (Cancel, per-row menu items, Settings' three independent
  section actions) stays plain filled/outlined/text. Settings has no hero button anywhere:
  it has three unrelated section actions, not one primary task, so none should dominate.
- **Dialogs**: two variants, both `surface-container-lowest` fill (**not** `surface-
  container-high`, which reads as a murky system-alert gray), `radius-xl` (32px), the "Dialog"
  elevation from §2.2, 32px padding, a flat scrim (`scrim` at 45% opacity, **no**
  `backdrop-filter: blur()`: a blurred scrim is the same glassmorphism trope this system
  otherwise avoids), and content centered under a 48×48px `radius-md` icon badge. **Neutral /
  choice dialogs** (the retention default, §6): icon badge in `primary-container`, options as
  bordered `radius-md` cards (a selected option gets a 2px `primary` border + a 4%-mixed
  `primary` tint, not just a filled radio dot), footer right-aligned with **Cancel as a plain
  text button** (never a filled/outlined twin the same size as the primary action, which
  creates a false choice between two equally-weighted actions when one is the low-frequency
  escape hatch) and the hero button as Continue. **Destructive / delete confirmations** (§7.1,
  §9): identical shape, but the icon badge uses `error-container`/`on-error-container` and the
  confirm button is the destructive hero button, carrying a trash icon and the exact copy from
  §15.
- **Auth split-card** (`first_run` §5.1, `locked` §5.2): a single floating card,
  1160×660–720px, `radius-xl`, the "floating card" elevation, centered on a near-flat page
  background (a barely-visible single linear gradient wash, no radial "glow" blobs, no
  dot-grid texture on the page; those read as generic AI-generated-UI decoration, not a
  deliberate mark of this app). Left panel (520–560px, `surface-container-lowest`): app mark +
  wordmark, headline, the form, and the non-recovery sentence (§5.1) directly below the
  passphrase field(s). Right panel (flex-fill): a dark tone of the same primary hue
  (`oklch(26% 0.07 264)` → `oklch(16% 0.05 268)`, 160° linear gradient) holding a centered
  padlock mark in a ringed tile plus two small flat (no blur) document/status chips, and one
  line of positioning copy at the bottom. The gradient **may drift very slowly** (an oversized
  background-size animated between two `background-position`s, ~20–22s `ease-in-out infinite
  alternate`) as a subtle ambient touch; it must respect `prefers-reduced-motion: reduce`
  (animation off entirely, not just slower) and must never be the loud, fast, or multi-hue kind
  of gradient animation.
- **Keep/redact segmented control** (§8, NFR-U2): a two-segment `radius-full` control, each
  segment carrying an icon (open eye / crossed eye) **and** a label ("Keep" / "Redact"), never
  color alone: the selected segment gets its container role (`tertiary-container` for Keep,
  `error-container` for Redact) plus bold weight; an undecided field shows neither segment
  selected. In the document body, the same three states repeat as span treatments: kept =
  solid underline, redacted = a diagonal-hatch fill + underline, undecided = a dashed
  underline: shape/pattern differences, not color-only, satisfying the same constraint inside
  the text itself.
- **Warning vs. error banners:** the FR-6.2 ephemeral-override notice (§10.2, §15) uses the
  `warning` role, never `error`: it is expected, not a failure. Reserve `error`/
  `error-container` for things that actually went wrong or are irreversible (delete
  confirmations, `unlock_failed`, field-level validation).

---

## 3. Webview isolation

Complements architecture §12 and api.md §8. This spec owns the concrete CSP and dialog
allowlist.

### 3.1 CSP (production)

```
default-src 'self';
script-src 'self';
style-src 'self' 'unsafe-inline';
img-src 'self' blob: data:;
media-src 'none';
connect-src 'self' ipc: http://ipc.localhost;
frame-src blob:;
object-src 'none';
base-uri 'none';
form-action 'none';
```

- `blob:` is for the redacted PDF preview (`URL.createObjectURL` on `pdf_bytes` only).
- No `https:` in `connect-src`. Cloud AI HTTP is Rust-only (C-ARCH-2).
- `'unsafe-inline'` style is allowed so Svelte can emit component CSS; no inline **scripts**.
- `frame-src blob:` is only for an `<iframe>` showing **redacted** preview bytes. Do not
  iframe `file://` or `https://`.

Dev-mode Vite may widen `connect-src` for HMR. Production builds use the box above.

### 3.2 Tauri capabilities

Grant: every command in api.md §5; `core:event:allow-listen` for `pg://detect-progress` and
`pg://session-changed`; `@tauri-apps/plugin-dialog` **save** (not open).

Deny: filesystem **read**, HTTP, shell, opener-with-arbitrary-URL, dialog **open**.

Grant for persist only: `@tauri-apps/plugin-fs` **write** of in-memory bytes to the path
the save dialog just returned (`writeFile` / `writeTextFile`). Deny `read`, `readDir`,
`remove`, `exists`, and `watch`.

Import does **not** use a path the webview can re-read. The user chooses a file via the
HTML file picker or drag-and-drop; the UI reads `File` bytes in memory and passes
`filename` (basename) + `bytes` to `import_document` (architecture §12). Dropped `file://`
paths are not stored.

### 3.3 What the DOM may hold

| State | Allowed in DOM / JS heap | Forbidden |
|---|---|---|
| `first_run` / `locked` | Passphrase only in the input until invoke returns | Vault content |
| `unlocked` approval (`open_approval` / `get_approval_view`) | Pages, `DetectedFieldDto.span.text`, labels, decisions | Keys, original files, other docs' fields |
| Share preview | Redacted `pdf_bytes` / `ai_payload_preview`, labels, field ids | Redacted span text, API keys |
| Audit | `list_audit_events` DTOs (no span text) | Document bodies |
| After `lock` / `abort_approval` / leave preview | Nothing from the previous session | Stale blob URLs, leftover span strings |

On `lock`, `pg://session-changed`, `abort_approval` / `submit_approval`, leaving share
preview, generating a **new** preview, or unmounting the preview component: revoke every
blob URL created with `URL.createObjectURL` (Svelte `onDestroy` / `$effect` teardown).
Do not leave redacted preview bytes mapped after the view is gone.

Window title is `Privacy Gate` or `Privacy Gate — Locked`. Never a `source_filename` or
field label (OS overview / screenshots).

---

## 4. Screens and navigation

```
first_run → (create_account) → vault
locked    → (unlock)         → vault | degraded_integrity
degraded_integrity → integrity screen only (lock still available)
vault     → import | approval | share | document menu | audit | settings
```

No URL deep links into a `doc_id`. In-app navigation only.

**Chrome when `unlocked`:** app name; primary nav **Vault**, **Audit trail**, **Settings**;
a **Lock** control always visible. Badge on Vault is `list_documents` length, not field
counts.

**Chrome when `degraded_integrity`:** integrity screen (§13); Lock; no Vault/Share/Import.
`list_audit_events` may show the verified prefix (api.md §2).

One approval session at a time. A second `open_approval` while one is active surfaces
`approval_busy`.

---

## 5. First run and lock/unlock

### 5.1 First run (`first_run`)

Fields: display name (api.md: 1..=80 trimmed), passphrase, passphrase confirm. Submit
`create_account`. Client-side: mismatch and empty name; min length 8 matching the API
floor. The UI **should** hint that longer passphrases are stronger; it must not invent a
complexity alphabet (not in SRS).

Copy, below the passphrase field (C-ARCH-7):

> Privacy Gate cannot reset this passphrase. If you forget it, this vault cannot be opened.
> There is no recovery email or backup code in this version.

Failed `create_account` shows `ApiError.message` (already non-secret). `account_exists` if
not `first_run`.

### 5.2 Unlock (`locked`)

Single passphrase field. Submit `unlock`. Wrong passphrase: `unlock_failed` — same wording
for unknown/wrong (api.md). No “forgot passphrase” link.

Out `state: "unlocked"` → Vault. Out `state: "degraded_integrity"` → §13, never Vault.
Show `integrity` when non-null.

### 5.3 Lock

`lock` then show §5.2. Lock is available from every unlocked chrome including Settings.
Closing the window should call `lock` if the session is `unlocked` or
`degraded_integrity` (best-effort; architecture already zeroizes on process exit).

---

## 6. First-import retention prompt (decision 0007)

Before the first successful `import_document`, if `get_retention_default.confirmed === false`,
show a **blocking modal first**. The file picker must **not** open until Continue succeeds.
Cancel leaves `confirmed` false and does not import.

Order (deterministic; used by §16 tests):

1. `get_retention_default` → `confirmed === false`.
2. Modal (§6 copy). Discard is pre-selected.
3. Continue → `set_retention_default` with the chosen `policy`.
4. Only then open the HTML file picker / accept a drop.
5. `import_document` with that file.

`import_document` does not take a policy inline (api.md). `retention_policy_unset` means
this sequence was skipped — treat as a UI bug.

**Pre-select:** Discard originals (factory `policy: "discard"`). The other two options are
available.

| Control | `set_retention_default` `policy` |
|---|---|
| Discard originals after approval (recommended) | `"discard"` |
| Keep encrypted originals by default | `"retain"` |
| Never keep originals (cannot keep on a single file) | `"never_retain"` |

Title: **Choose a default for original files**

Body:

> Before the first import, choose what Privacy Gate should do with original files after you
> approve a redacted version. You can change the default later in Settings. For a single
> import you can keep or discard differently, unless you choose “never keep originals.”

Primary: **Continue**. Secondary: **Cancel** (no import; `confirmed` stays false).

Continue with Discard still pre-selected is a valid confirmation (decision 0007).

Later imports: compact control mapped to `import_document.retention_override`: **Use
default** (`null`), **Keep original** (`"retain"`), **Discard original** (`"discard"`). If
default is `never_retain`, **Keep original** is absent or disabled; a click explains
`retention_loosen_forbidden` in plain language (§15).

---

## 7. Vault and import

### 7.1 Vault list

`list_documents` on entry and after import/delete. After `submit_approval`,
`abort_approval`, or `delete_retained_original`, refresh that row with `get_document`
(do not require a full catalog refetch). Newest first (API). Each row:
`source_filename`, `source_format`, `imported_at` (locale from RFC 3339), `retention`,
`has_approved_version`, `has_retained_original`, `detected_field_count`. No span text.

Empty state: prompt to import a text file or PDF (born-digital; FR-1.1). No sample-cloud
documents.

Row actions: Open (`open_approval` if `has_approved_version === false`; otherwise share /
variants / `delete_retained_original` if kept / `delete_document`), Delete (confirm;
FR-4.6 irrevocable).

Re-import of the same file is a **new** document (design §3.6). v1 shows **no** duplicate
warning (the core does not identify duplicates; do not fake it from filename).

### 7.2 Import picker

HTML `<input type="file">` and drag-and-drop onto the vault. Accept `.pdf`, `.txt`, and
whatever the OS reports as `text/plain` / `application/pdf`. Read `File.name` as basename
only (strip path; path separators → `invalid_input`). Pass `{ filename, bytes,
retention_override }` to `import_document`.

While the command runs, show `pg://detect-progress` `{ fraction }` as a determinate bar
(0..1). Cancel is not a v1 API; the UI waits. `over_budget === true`: after completion,
show §15 over-budget copy; do not discard the document.

Errors: `unsupported_document`, `retention_policy_unset` (should not happen if §6 ran),
`retention_loosen_forbidden`, `invalid_input`. Map via §15.

### 7.3 After import

If `has_approved_version === false`, navigate to **Approval** (`open_approval`). If the user
aborts, the document may remain or the catalog row may be gone (discard + abort/lock,
api.md §5.4) — refresh `list_documents`.

---

## 8. Approval (consent step)

Owns FR-2.2 presentation and FR-3.1 decisions. Layout: **two panes**.

- **Left:** `ApprovalView.pages` text. Detected spans highlighted **in place** (locatable).
  Nested fields (`parent_field_id`) are visually nested; do not hide inner fields inside
  outer ones (design §3.5).
- **Right:** one row per `DetectedFieldDto`: label, classification, keep/redact control
  (`keep_visible` / `redact`). Keep and redact must not be distinguished by colour alone
  (NFR-U2).

Selecting a list row scrolls/highlights the span and vice versa.

`set_field_decisions` may be called incrementally. Primary **Approve and store** is enabled
only when `get_approval_view.lifecycle === "decided"` (all fields have a decision). It calls
`submit_approval`. **Cancel** calls `abort_approval` and returns to Vault.

First paint (§14) is measured on the first page of text and the first **200** field rows
(design §7 cap the core returns in ≤ 1 s). If more fields exist, additional rows may
render after first paint (progressive list). Do not block first paint on the full list.

`open_approval` on an already-approved document returns `already_approved`. Further changes
are share-time overrides or variants.

Do not offer a download of the original from this screen.

---

## 9. Variants

From an approved document: `list_variants`, `get_variant` (decisions, no span text),
`save_variant` from the share override set, `delete_variant` with confirm. No edit (design
§3.4): copy is “to change this, delete it and save a new variant.” Names 1..=80, unique per
document; surface `variant_name_conflict`.

Empty state: **No saved variants for this document.** Customize keep/redact during share
preview and save them as a variant to reuse later.

---

## 10. Share, preview, save dialog (resolves OQ-4 chrome)

### 10.1 Start share

User selects one or more approved documents (selection order = `doc_ids` order, design
§3.7 / api.md `ShareRequestDto`). Unapproved documents cannot be selected (`not_approved`).

Two modes: **Export PDF** (`kind: "export_to_person"`) and **Ask Cloud AI**
(`kind: "share_to_ai"`).

Optional per document: `applied_variant_ids`; `per_doc_overrides` as `FieldDecisionDto[]`.
Overrides use the same keep/redact language as approval.

`recipient_note`: optional, person export only; must be null for AI. `ai_instruction`:
required non-empty for AI, 1..=4000 (api.md).

### 10.2 Preview (FR-6.1)

Always call `preview_share` before `commit_share`. Show:

- Export: PDF from `pdf_bytes` (blob URL + `<iframe>`). Redacted content only.
- AI: exact `ai_payload_preview` in a read-only pane (body that will be POSTed, not the
  plugin wrapper).

Show `manifest` as visible / redacted **field ids/labels** without redacted span **text**.

If `overrides_in_effect`, show the FR-6.2 warning **before** Confirm (§15). Do not let the
user confirm without the warning visible (not a toast that disappears).

Token expiry: `preview_expired` → rebuild preview; do not commit. A new `preview_share`
invalidates the previous token (api.md).

### 10.3 Confirm

**Export:** §10.4 (dialog first). On path chosen: `commit_share` then write
`pdf_bytes` from the commit result (byte-identical to preview).
**AI:** `commit_share` then show `output_text` read-only. `cloud_ai_network` /
`cloud_ai_refused`: show error class; audit still recorded (api.md) — say so.

Do not auto-send AI on preview. Confirm is explicit. `cloud_ai_not_configured`: send the
user to §11.4 before previewing an AI share.

### 10.4 Save-dialog chrome (OQ-4 remainder)

This is the C-ARCH-2 exception: persist **in-memory bytes the core already returned**
(previewed `pdf_bytes`, or `get_integrity_report` JSON). It must not open files into the
webview. Architecture §12 / C-ARCH-2 names both payloads.

**Sequence (export):**

1. User confirms the preview (warning visible if `overrides_in_effect`).
2. Open the **OS save dialog** (`plugin-dialog` save).
   - Default filename: `suggested_filename` from `SharePreview` (api.md §7). Do not
     substitute `source_filename`.
   - Filter: PDF only (`*.pdf`).
   - Default directory: the platform documents directory, **not** the folder the source was
     imported from (avoids writing a redacted file next to the original by habit).
   - Title: **Save redacted PDF**.
3. If the user **cancels**: do **not** call `commit_share`; no audit share event; stay on
   preview.
4. If the user **chooses a path**: call `commit_share`. Write commit `pdf_bytes` to that
   path only (scoped write of in-memory bytes; no `plugin-fs` read). Then revoke the blob
   URL.
5. If commit succeeds and the write fails: keep `pdf_bytes` in memory, say the export is
   recorded in the audit trail (`audit_event_id`), offer **Retry save** (dialog again, no
   second `commit_share`). Do not silently skip the audit (commit already happened).

No in-webview fake save form. No open dialog. No reading the saved file back into the
webview.

Success confirmation may show the **filename** the user chose. Full path is allowed in a
secondary line so they can find the file; it is not an original path.

---

## 11. Settings

One Settings screen (primary nav). Commands are those in api.md §5; none are invented here.

### 11.1 Account

Read-only from `get_account`: display name, `account_id`, `created_at`. No remote identity.

### 11.2 Passphrase

`change_passphrase`: current, new, confirm. Client-side mismatch check; min length 8.
Surface `passphrase_mismatch`. Repeat the non-recovery sentence from §5.1.

### 11.3 Retention default

`get_retention_default` / `set_retention_default`. Same three policies as §6. Changing
`never_retain` → `retain` is allowed (api.md: global change, not a per-import loosen).
Per-import keep is still forbidden while the default is `never_retain`.

### 11.4 Cloud AI

Fields: `https` endpoint, model id, API key (write-only). `cloud_ai_get_config` shows host,
model, `key_last4`, never the key. Test calls `cloud_ai_test` (no documents). Clear calls
`cloud_ai_clear_config` with confirm.

Copy: sharing sends **approved** content to the host they typed; detection never uses this
endpoint.

Empty detector/new-flow plugin registries are **not** a v1 settings UI (hooks exist in core;
no first-party extra plugins).

---

## 12. Audit trail (NFR-U1)

`list_audit_events` when `unlocked` (verified prefix when `degraded_integrity`). Table:
time, `event_type` in words, `source_filename` / doc ids when present, destination
(`recipient_note` or `endpoint_host`), share `kind`, `no_originals_left_device` when the
event carries it. Filter by document and by type (api.md). No field text.

A share row must answer “what did I share, and to whom?” at NFR-U2 reading level:
“Exported PDF” / “Asked Cloud AI”, recipient note or endpoint **host**, document names,
whether originals remained on the device.

Empty / filtered: **No audit events match this filter.** A new vault still shows the
`create_account` (and later import) events; an empty table after first run is a bug.

Integrity wording for a healthy trail is not a banner. Failure is §13.

---

## 13. Integrity failure and lost passphrase

### 13.1 `degraded_integrity`

Full-screen, fail-closed for documents (architecture §6.3). `get_integrity_report` supplies
the report.

Title: **This vault cannot open documents**

Body:

> Privacy Gate checked the audit trail and found it does not match. Documents will not be
> decrypted. You can save a verification report. Restoring from your own backup is the
> recovery path; Privacy Gate cannot repair a tampered vault.

Actions: **Save report** (JSON of `IntegrityReport` via the **same** save-dialog rules as
§10.4, default name `privacy-gate-integrity-report.json`, not a PDF of documents); **Lock**;
no “Open anyway”.

### 13.2 Lost passphrase

Only the sentence in §5.1 / §5.2. No security-question UI, no file-based recovery (C-ARCH-7).

---

## 14. First paint and motion

Design §7: core returns approval payload in ≤ 1 s. This spec owns **first paint of chrome**.

| Screen | Budget |
|---|---|
| Lock / first-run chrome | ≤ 200 ms after webview `DOMContentLoaded` (static; `get_session_state` only) |
| Vault list after `list_documents` returns | ≤ 300 ms to first row or empty state |
| Approval after `open_approval` returns | ≤ 300 ms to first page text and field list (≤ 200 fields; design §7) |
| Share PDF preview after `preview_share` returns | ≤ 500 ms to first PDF page for a ≤ 25 MB-source export |

No blocking splash after unlock beyond a spinner if a command is in flight. Detection
progress uses `pg://detect-progress` when `fraction` is available.

Documents with `over_budget`: still import; warn with §15; no extra animation.

---

## 15. Canonical copy

Use these strings (or a strict paraphrase that keeps the meaning). Do not soften fail-closed
or recoverability.

| Situation | Copy |
|---|---|
| Ephemeral override (FR-6.2) | These changes apply to this share only. The approved version in your vault will not change. |
| `retention_loosen_forbidden` | The default is “never keep originals,” so this file cannot be kept. Change the default in Settings if you want to keep originals going forward. |
| `over_budget` | This file is larger than the size Privacy Gate is tuned for (25 MB). Import will finish; it may take longer than usual. |
| `unsupported_document` | Privacy Gate v1 only imports text and PDFs that already contain text. Scanned pages and photos are not supported yet. |
| `unlock_failed` | Could not unlock. Check the passphrase. |
| Delete document | This deletes the approved version, any kept original, and variants. It cannot be undone. |
| AI confirm | Only the approved, redacted text shown in the preview will be sent to the host you configured. |
| `cloud_ai_not_configured` | Cloud AI is not configured. Add an endpoint and key in Settings before asking a model. |
| `preview_expired` | This preview expired. Generate a new preview before exporting. |

Passphrase / API key fields: `autocomplete="off"`; never log values.

---

## 16. UI tests (C-TEST-8)

Core acceptance stays in-process ([testing.md](./testing.md)). This spec owns:

| Layer | Tool | Gate |
|---|---|---|
| Component (screens, copy, enabled/disabled Approve, warning visible) | Vitest + Testing Library against Svelte | Every PR that touches `src/` UI |
| Save-dialog sequence | Component test with a fake dialog: cancel → no `commit_share`; confirm path → `commit_share` then write mock | Every PR that touches share/save |
| First-import modal | Confirmed discard pre-selected; Continue calls `set_retention_default` before `import_document` | Every PR that touches import |
| Integrity screen | No navigation to Vault | Every PR that touches session |
| First paint (§14) | Component test with a fake clock: after mocked command resolve, first row/text is in the document within the budget (jsdom; not OS-dialog E2E) | Every PR that touches those screens |
| Webview E2E (OS dialogs, real PDF iframe) | Optional Playwright/Tauri driver | Not a PR mutation gate; not `cargo-mutants` |

Do not load the real Cloud AI host. Do not put canary PII in component snapshots.

Stryker/mutation on TypeScript is **not** a v1 gate (decision 0006).

---

## 17. Constraints

- **C-UI-1** The webview never calls HTTP or opens arbitrary files. Import is `File` bytes →
  `import_document`. Persist is save-dialog + in-memory core bytes only (`pdf_bytes` or
  integrity-report JSON).
- **C-UI-2** Redacted field text never appears in the DOM. Unapproved text appears only on
  the approval screen until `submit_approval` / `abort_approval` / `lock`.
- **C-UI-3** Share confirm is blocked until `preview_share` has been shown. Export
  `commit_share` is blocked until the save dialog succeeds; cancel = no commit.
- **C-UI-4** First import cannot call `import_document` until `set_retention_default`
  (decision 0007). Discard is pre-selected.
- **C-UI-5** Integrity failure never offers an “open documents anyway” path.
- **C-UI-6** Window title and OS notifications contain no document text or field labels.
- **C-UI-7** No runtime CDN, analytics, or crash reporter in the webview.
- **C-UI-8** UI tests in §16 do not replace testing.md command/AC tests.

---

## 18. Traceability

| Source | UI coverage |
|---|---|
| FR-1.1..1.4 import + retention prompt | §6, §7 |
| FR-2.2 locatable spans | §8 |
| FR-3.1..3.2 approve + one version | §8 |
| FR-4.6 delete copy | §7.1, §15 |
| FR-5.1..5.5 export, AI, overrides, variants | §9, §10, §11 |
| FR-6.1 / 6.2 preview + ephemeral warning | §10.2, §15 |
| FR-7 / NFR-U1 audit inspectable | §12 |
| FR-8 first run / unlock | §5 |
| NFR-U2 non-expert copy | §15, §6, §13 |
| NFR-S4 / C-ARCH-2 save exception | §10.4 |
| NFR-U2 non-colour state pairing | §2.1, §2.3, §8 |
| Design tokens (color/type/shape/elevation) | §2.2 |
| Signature components (nav rail, buttons, dialogs, auth card) | §2.3 |
| OQ-4 save-dialog chrome | §10.4 |
| Decision 0007 first-upload | §6 |
| Decision 0008 Svelte | §2 |
| C-TEST-8 | §16 |
| Design §7 first paint | §14 |

---

## 19. Deferred

- Custom branding and a marketing site (the M3 color/type/shape/elevation token file itself
  is now specified in full at §2.2/§2.3 — not deferred).
- i18n (v1 English copy in §15).
- Accessibility standard certification (keyboard: approval list and keep/redact must be
  operable without a pointer; no WCAG certificate in v1).
- OS notification centre, tray icon, mobile.
- In-app duplicate-file detection (design §3.6).
- Plugin marketplace / third-party plugin UI.
- Vault backup / restore screens and reinstall re-attachment (idea.md later phase; not
  passphrase recovery, not share PDF export).

---

## 20. Related decisions

- [0002](../decisions/0002-resolved-srs-clarifications.md) — one approved version; PDF bundle; true removal.
- [0003](../decisions/0003-v1-tech-stack.md) — Tauri + TS; framework was deferred here.
- [0004](../decisions/0004-v1-architecture.md) — webview untrusted; HTTP in Rust; no recovery.
- [0005](../decisions/0005-review-claude-gemini.md) — Claude + Gemini review.
- [0006](../decisions/0006-tdd-and-mutation-testing.md) — no TS mutation gate.
- [0007](../decisions/0007-retention-default-discard.md) — factory discard; first-import confirm.
- [0008](../decisions/0008-frontend-svelte.md) — Svelte 5 + Vite.

## 21. Related work

- [0008-ui-spec](../dev-log/0008-ui-spec.md)
- [Spec — SRS](./srs.md)
- [Spec — API](./api.md)
- [Spec — design](./design.md)
- [Spec — architecture](./architecture.md)
- [Spec — testing](./testing.md)
- [Spec — data model](./data-model.md)
