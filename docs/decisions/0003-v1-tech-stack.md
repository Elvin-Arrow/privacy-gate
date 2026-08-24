# Decision: v1 tech stack — Tauri + Rust core + TypeScript frontend

- **Status:** Accepted
- **Date:** 2026-08-23

## Context

Privacy Gate v1 is a local-first, single-user desktop app with on-device sensitive-field
detection, envelope-encrypted storage, an audit trail, and a first-party Cloud AI plugin. The
SRS leaves the stack open (NFR-PORT1 defers OS support to design; the detection model and crypto
library are unspecified). The design spec needs a concrete stack to describe components, and the
stack constrains the architecture spec (plugin runtime, key storage, transient-plaintext
handling, OS support).

## Decision

Build Privacy Gate v1 on:

- **Shell: Tauri 2.x.** Native webview desktop shell. v1 targets macOS, Windows, and Linux
  (resolves OQ-1), the platforms Tauri 2.x supports out of the box.
- **Core: Rust.** Vault, encryption, audit trail, detection-model host, plugin host, and
  document processing all live in a Rust core invoked through Tauri commands. Rust provides
  strong crypto crates (e.g. `ring`/`rusqlite`/SQLCipher-compatible options) and memory-safe
  handling of untrusted document input.
- **Frontend: TypeScript** (framework choice deferred to the UI spec). Communicates with the
  core via Tauri's typed command/IPC boundary.
- **On-device detection: a small local model hosted in the Rust core.** Concrete model identity
  (e.g. a Gemma variant, regex + NER hybrid) is an architecture spec decision; this decision
  fixes only that detection runs in-process on-device, never via a network call (NFR-P1, C-5).
- **Plugin runtime: in-process Rust plugins for v1, behind the three-part plugin surface
  (output consumers, detectors, new flows).** The v1 runtime does not yet host third-party code;
  only the first-party Cloud AI output consumer ships. A WASM-based sandboxed runtime is the
  expected path for the later third-party phase (OQ-13) but is not committed to here.

## Rationale

- **Tauri over Electron.** Smaller binaries, no Node runtime, and a clean Rust↔frontend IPC
  boundary that keeps the vault and crypto logic in memory-safe Rust. Strong local-first story
  and lower ambient permission surface, which matters for an app whose whole point is trust.
- **Rust core.** Memory safety for parsing untrusted PDFs/text, mature crypto crates, and a
  single core that the frontend, CLI, and plugins all call — minimizing the surface where
  plaintext is handled.
- **TS frontend.** Fast to iterate on the review/approve UI, broad UI-spec options, and the
  Tauri IPC boundary keeps all secrets in Rust.
- **Detection in-process.** Keeps the NFR-P1/C-5 guarantee structural rather than a policy the
  app has to enforce at every call site.
- **Plugin runtime deferred.** v1 ships only first-party code, so a sandboxed runtime is not
  needed yet; the plugin surface is designed so the WASM path can be added later without rework
  (FR-9.5, NFR-E1).

## Alternatives Considered

### Electron + Node/TS

Larger binary and a Node runtime with broader ambient permissions; crypto and untrusted-PDF
parsing in a GC'd JS process is a weaker safety story. Faster plugin ecosystem, but v1 ships no
third-party plugins, so the advantage does not land yet.

### Pure native (Swift/WinUI/Qt)

Best perf and smallest surface, but triples the implementation cost across macOS/Windows/Linux
and fragments the plugin runtime. Not justified for v1 single-user scope.

### Leave stack open in the design spec

Rejected: the design spec cannot name components or draw module boundaries without committing to
a stack. Stack selection is exactly the kind of consequential, "why did we do it this way?"
choice that belongs in a decision record.

## Consequences

- **Resolves OQ-1**: v1 supports macOS, Windows, Linux.
- **Architecture spec inherits these constraints**: key storage uses OS keystore via Tauri/Rust
  (OQ-18 key rotation still open); transient-plaintext handling (OQ-17) must be reasoned about
  in Rust process memory, not a JS layer.
- **UI spec picks the TS framework** (React/Svelte/Solid/etc.) within the Tauri boundary.
  **Resolved** by [decision 0008](./0008-frontend-svelte.md): Svelte 5 + Vite.
- **API spec fixes the Tauri command surface** between frontend and core.
- **Testing spec covers Rust core (unit + property tests), TS frontend, and cross-IPC
  integration.**
- A future switch to a WASM plugin runtime (OQ-13) is enabled but not committed to.

## Related Documentation

- [Spec — SRS](../specs/srs.md)
- [Spec — design](../specs/design.md)
- [Open questions](../notes/open-questions.md)
- [Decision 0002 — resolved SRS clarifications](./0002-resolved-srs-clarifications.md)