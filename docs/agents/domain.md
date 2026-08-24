# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the
codebase.

This repo does **not** use the generic `CONTEXT.md` / `docs/adr/` convention. It already has its own
established doc governance — the `knowledge-governance` skill (`skills/project-knowledge/`, mirrored
at `.cursor/skills/knowledge-governance/`) — with its own structure. Read that skill before writing
to `docs/`; the mapping below is only for skills that expect the generic layout.

## Before exploring, read these

- **`docs/idea.md`** — product intent (equivalent role to `CONTEXT.md`'s "what is this": problem,
  flow, in/out of scope). Authoritative source for product intent.
- **`docs/specs/`** (`docs/specs/index.md` for navigation) — current/intended behavior, split by
  concern (`architecture.md`, `api.md`, `data-model.md`, `testing.md`, `ui.md`, `srs.md`,
  `design.md`). Read the specs relevant to the area you're about to work in, same as you'd read a
  context-scoped `CONTEXT.md`.
- **`docs/decisions/`** (`docs/decisions/index.md`) — this repo's equivalent of `docs/adr/`.
  Read decisions that touch the area you're about to work in before proposing something that
  contradicts one.
- **`docs/dev-plan.md`** — the ordered implementation sequence (chunk by chunk). Not authoritative on
  behavior (specs win on conflict), but tells you what's already built and what isn't yet.
- **`docs/dev-log/`** — implementation history per chunk; useful for "why does this code look the way
  it does" without re-deriving it from the diff.

If any of these don't exist yet in a given area, proceed silently — don't flag their absence or
suggest creating them upfront.

## File structure (single-context)

```
/
├── docs/
│   ├── idea.md          ← product intent (like CONTEXT.md)
│   ├── specs/            ← current/intended behavior, by concern
│   ├── decisions/        ← rationale (like docs/adr/)
│   ├── dev-plan.md       ← implementation sequence
│   ├── dev-log/          ← implementation history
│   └── notes/            ← non-authoritative working notes
├── core/                 ← Rust core (pg-core)
├── src-tauri/             ← Tauri binary
└── src/                   ← Svelte frontend
```

This is a single-context repo: one Cargo workspace (`core` + `src-tauri`) plus one frontend
(`src`), no `packages/*` or workspace-per-domain split. There is no `CONTEXT-MAP.md` and none is
expected.

## Use the spec/decision vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a
test name), use the term as defined in the relevant `docs/specs/*.md` file — e.g. `DEK`, `wrap_key`,
`KeystoreItem`, `ApprovedVersion`, the `W<n>` chunk names from `dev-plan.md`. Don't drift to
synonyms the specs don't use.

If the concept you need isn't in a spec yet, that's a signal: either you're inventing language the
project doesn't use (reconsider) or there's a real gap (note it — the `knowledge-governance` skill
owns updating specs/decisions/dev-log lazily as concepts get resolved, not this file).

## Flag decision conflicts

If your output contradicts an existing entry in `docs/decisions/`, surface it explicitly rather than
silently overriding:

> _Contradicts decision 0007 (retention default = discard), but worth reopening because…_

## Authority order

Per `docs/index.md`: `docs/idea.md` (product intent) → `docs/specs/*.md` (current/intended behavior)
→ `docs/dev-plan.md` (sequence, not behavior) → `docs/decisions/*.md` (rationale) →
`docs/dev-log/*.md` (history) → `docs/notes/*.md` (non-authoritative). If two documents disagree,
the higher one wins.
