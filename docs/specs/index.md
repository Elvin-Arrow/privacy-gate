# Specs Index

## Purpose

Authoritative current and intended behavior for Privacy Gate. Specs answer *what this is, what
it does, its interfaces and constraints, its current expected behavior, and its dependencies*.
They are not diaries or transcripts.

## Contents

- [srs](./srs.md) — Software Requirements Specification (v1): functional, non-functional, data, constraints, acceptance criteria, scope, and traceability to idea.md. Requirements only.
- [design](./design.md) — Component-level design (v1): ten components, flows, performance budget, and variant/overlap/re-import rules. Stack: Tauri + Rust + TS (decision 0003). Types: data-model.
- [architecture](./architecture.md) — Crypto, key storage, trust boundaries, plugin host API, detection-model host, export sanitization. Resolves OQ-3, OQ-5, OQ-12 (storage/HTTP), OQ-13, OQ-17, OQ-18 (decision 0004).
- [api](./api.md) — Tauri 2 command/event surface, errors, session gating, preview tokens. Resolves OQ-4 filename/PDF metadata and OQ-12 command shape.
- [testing](./testing.md) — TDD, mutation testing (`cargo-mutants`), AC-1..AC-7 mechanics, OQ-6 oracle. Decision 0006.
- [data-model](./data-model.md) — Single source for types, identifiers, SQLCipher schema, envelope artifacts (including document_meta), audit rows, keystore item. Other specs link here and do not restate field lists.
- [ui](./ui.md) — Svelte 5 webview: screens, copy, CSP, save-dialog chrome (OQ-4), first-paint budgets, UI tests. Decision 0008.

## Navigation

- Source of truth for product intent: [../idea.md](../idea.md).
- Worked example used to validate requirements: [../user-story.md](../user-story.md).
- Decision records: [0001 review approach](../decisions/0001-multi-model-spec-review.md), [0002 resolved SRS clarifications](../decisions/0002-resolved-srs-clarifications.md), [0003 v1 tech stack](../decisions/0003-v1-tech-stack.md), [0004 v1 architecture](../decisions/0004-v1-architecture.md), [0005 Claude+Gemini review](../decisions/0005-review-claude-gemini.md), [0006 TDD + mutation](../decisions/0006-tdd-and-mutation-testing.md), [0007 retention default](../decisions/0007-retention-default-discard.md), [0008 Svelte frontend](../decisions/0008-frontend-svelte.md).
- Open questions remaining from the SRS: [../notes/open-questions.md](../notes/open-questions.md).
- Work items: [0001 SRS generation](../dev-log/0001-srs-generation.md), [0002 design spec](../dev-log/0002-design-spec.md), [0003 architecture spec](../dev-log/0003-architecture-spec.md), [0004 API spec](../dev-log/0004-api-spec.md), [0005 testing spec](../dev-log/0005-testing-spec.md), [0006 OQ-14 retention](../dev-log/0006-oq14-retention-default.md), [0007 data model](../dev-log/0007-data-model-spec.md), [0008 UI spec](../dev-log/0008-ui-spec.md).
- Implementation sequence (not a spec): [../dev-plan.md](../dev-plan.md).