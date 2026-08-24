# Decision: v1 frontend is Svelte 5 + Vite + TypeScript

- **Status:** Accepted
- **Date:** 2026-08-23

## Context

[Decision 0003](./0003-v1-tech-stack.md) fixed the shell (Tauri 2.x) and the frontend language
(TypeScript) and explicitly deferred the UI framework to the UI spec. The webview is untrusted
for secrets and for redacted-field text ([architecture.md](../specs/architecture.md) C-ARCH-1).
A heavier client runtime is not a product feature; it is attack surface and bytes in an
untrusted heap.

## Decision

v1's TypeScript UI is **Svelte 5** compiled with **Vite**, using TypeScript throughout.
Styling is bundled CSS (no runtime CSS-in-JS library, no webfont or stylesheet CDN).
Routing is in-memory (no HTTP router, no History API that implies a server).

Tauri 2 has a first-party Svelte template; IPC uses `@tauri-apps/api` only, plus the
save-dialog plugin scoped in the UI spec.

## Rationale

- The webview holds unapproved span text during review (C-DES-1 exception). A compiler that
  emits small, framework-light JS reduces how much interpreter machinery sits next to that
  text. Svelte 5's compiled components avoid a permanent virtual-DOM runtime.
- Vite is the Tauri 2 default bundler; one toolchain for macOS, Windows, and Linux.
- React/Vue would also work; they add a larger client runtime without a v1 product need
  (no public component ecosystem, no SSR, no third-party widget set).

## Alternatives considered

- **React 19 + Vite.** Largest hiring pool and Tauri examples. Rejected: larger runtime in
  the untrusted webview for no v1 product gain.
- **Solid.** Fine performance; smaller community and fewer Tauri examples than Svelte.
- **Vanilla TS.** Lowest runtime, highest cost to implement the approval two-pane and
  share-preview flows correctly.

## Consequences

- UI spec (`docs/specs/ui.md`) is written against Svelte 5 + Vite.
- Testing spec's "TS/Stryker is not a v1 gate" still holds ([decision 0006](./0006-tdd-and-mutation-testing.md)).
  The UI spec may add Vitest component tests; they are not a mutation gate.
- No npm package may be added that performs network I/O from the webview at runtime.

## Related documentation

- [Decision 0003 — v1 tech stack](./0003-v1-tech-stack.md)
- [Spec — UI](../specs/ui.md)
- [Spec — architecture](../specs/architecture.md)
