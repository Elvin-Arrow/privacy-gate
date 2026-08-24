# Decisions Index

## Purpose

Records of consequential choices made on Privacy Gate — architectural, technical, product,
security, and trade-off decisions with their rationale. Created when a future engineer may
reasonably ask "why did we do it this way?"

## Contents

- [0001-multi-model-spec-review](./0001-multi-model-spec-review.md) — Review method (structured prompt, reconcile against idea.md). Original roster Gemini + gpt-oss + qwen-3.5; **roster superseded by 0005**.
- [0002-resolved-srs-clarifications](./0002-resolved-srs-clarifications.md) — Settles five SRS clarifications: one canonical approved version; paranoid-default retention semantics; manual redaction out of v1; export = true removal; multi-doc single PDF bundle.
- [0003-v1-tech-stack](./0003-v1-tech-stack.md) — v1 stack: Tauri 2.x shell, Rust core, TypeScript frontend, in-process on-device detection; macOS/Windows/Linux.
- [0004-v1-architecture](./0004-v1-architecture.md) — Crypto suite, local-only account, audit MAC + crash-window, plugin host API, Cloud AI auth, hybrid detector, re-render export; resolves OQ-3, OQ-5, OQ-12, OQ-13, OQ-17, OQ-18.
- [0005-review-claude-gemini](./0005-review-claude-gemini.md) — Spec review roster is Claude + Gemini; do not invoke Ollama. Supersedes 0001's reviewer list.
- [0006-tdd-and-mutation-testing](./0006-tdd-and-mutation-testing.md) — TDD required for TCB work; `cargo-mutants` gate on Rust TCB.
- [0007-retention-default-discard](./0007-retention-default-discard.md) — Factory retention is `discard` (unconfirmed); first successful import requires an explicit `set_retention_default`.
- [0008-frontend-svelte](./0008-frontend-svelte.md) — v1 TypeScript UI is Svelte 5 + Vite (untrusted webview; small compiled runtime).
- [0009-ollama-detector-backend](./0009-ollama-detector-backend.md) — Optional `pg-hybrid-ollama-v1` backend (local Ollama + pinned Gemma tag) preferred when available; `pg-hybrid-v1` (decision 0004) stays the always-available fallback. Loopback-only network boundary, verify-then-trust offset mapping, model allowlist/digest pin. Partially supersedes decision 0004's detector-identity clause.

## Navigation

- Affected specs: [../specs/srs.md](../specs/srs.md), [../specs/design.md](../specs/design.md), [../specs/architecture.md](../specs/architecture.md), [../specs/api.md](../specs/api.md), [../specs/testing.md](../specs/testing.md), [../specs/data-model.md](../specs/data-model.md), [../specs/ui.md](../specs/ui.md).
- Open questions remaining from the SRS: [../notes/open-questions.md](../notes/open-questions.md).
- Work items: [0001 SRS](../dev-log/0001-srs-generation.md), [0002 design](../dev-log/0002-design-spec.md), [0003 architecture](../dev-log/0003-architecture-spec.md), [0004 API](../dev-log/0004-api-spec.md), [0005 testing](../dev-log/0005-testing-spec.md), [0006 OQ-14](../dev-log/0006-oq14-retention-default.md), [0007 data model](../dev-log/0007-data-model-spec.md), [0008 UI](../dev-log/0008-ui-spec.md).