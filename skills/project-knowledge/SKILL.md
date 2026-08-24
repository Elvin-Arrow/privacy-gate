---
name: knowledge-governance
description: Establish and maintain a lightweight, agent-friendly knowledge architecture for software projects. Use when initializing a repository, creating or updating specs, decisions, development logs, indexes, or reviewing documentation health. Scale the structure to the project rather than forcing a fixed hierarchy.
---

# Knowledge Governance

Maintain project knowledge as a navigable, curated system. Help agents understand what the project is, what is currently true, why it is true, and how it evolved without turning documentation into a noisy transcript.

## Core principles

1. **Current truth lives in specs.** Specs describe current or intended behavior. Keep discussions, debugging transcripts, and implementation chatter out of them.
2. **Why lives in decisions.** Record meaningful architectural, technical, product, security, and trade-off decisions. Prefer concise records over transcripts.
3. **How we got here lives in development logs.** Record meaningful implementation history, problems, fixes, experiments, and lessons. Prefer ticket/issue/work-item logs over daily diaries.
4. **Indexes are navigation, not dumps.** Root indexes point to major areas; area indexes point to specific documents. Keep links purposeful.
5. **Link, don't copy.** Make one document authoritative for a fact and link to it elsewhere.
6. **Update before adding.** Search for an existing canonical document before creating a new one.
7. **Scale with the project.** A greenfield project may start with only `docs/idea.md`. Do not create empty structures for hypothetical future needs.
8. **Keep raw discussion out of canonical documents.** Distill useful discussion into specs, decisions, or logs.

## Greenfield workflow

### Phase 1 — Idea
If `docs/idea.md` exists, use it as the working source during discovery. Refine goals, constraints, ambiguities, and open questions without prematurely creating a large documentation tree.

### Phase 2 — Shape
When stable concepts emerge, create only what is useful:
- `docs/index.md`
- `docs/specs/`
- `docs/decisions/`
- `docs/dev-log/`

Add area indexes when an area becomes large enough to need navigation.

### Phase 3 — Build
During implementation:
- update specs when current behavior changes;
- create/update decisions for consequential choices;
- create/update dev logs for meaningful tickets or work units;
- link logs to relevant specs and decisions;
- keep process chatter in logs, not specs.

### Phase 4 — Maintain
After meaningful work:
- verify affected specs remain accurate;
- update/create relevant decisions;
- update the work-item log;
- update indexes after file creation, deletion, renaming, or repurposing;
- consolidate duplicates instead of proliferating documents.

## Suggested structure

Use only when justified:

```text
docs/
├── index.md
├── idea.md
├── specs/
│   ├── index.md
│   └── ...
├── decisions/
│   ├── index.md
│   └── ...
└── dev-log/
    ├── index.md
    └── ...
```

For larger projects, area indexes may introduce another level:

```text
docs/
├── specs/
│   ├── index.md
│   ├── architecture/
│   │   ├── index.md
│   │   └── ...
│   └── api/
│       ├── index.md
│       └── ...
├── decisions/
│   ├── index.md
│   └── ...
└── dev-log/
    ├── index.md
    ├── TICKET-123.md
    └── TICKET-456.md
```

Do not add hierarchy merely for appearance.

## Classification when new information arrives

Before writing, classify the information:

1. **Discard** — irrelevant, transient, duplicate, or not worth preserving.
2. **Update spec** — establishes or changes current project truth.
3. **Create/update decision** — records a consequential rationale or choice.
4. **Create/update dev log** — records meaningful implementation history.
5. **Link only** — useful context already exists elsewhere.

When uncertain, do not silently promote uncertain information into a canonical spec. Ask when the distinction matters.

Before writing:
- search related documentation;
- identify the canonical document;
- prefer updating over creating;
- preserve useful links.

## Dev-log policy

Use tickets/issues/features/milestones as the normal unit.

A useful log can contain:
- objective;
- implementation summary;
- important problems;
- meaningful attempted approaches;
- resolution;
- verification;
- lessons;
- links to affected specs and decisions.

Do not log trivial changes. The dev-log index should be a concise navigation layer and may group entries by ticket, feature, sprint, milestone, or release.

## Spec policy

A spec should answer:
- What is this?
- What does it do?
- What are its interfaces and constraints?
- What is the current expected behavior?
- What does it depend on?

Do not turn a spec into a diary, transcript, list of rejected ideas, or historical dump. Distill useful historical context into a decision or concise rationale and link it.

## Decision policy

Create a decision record when a future engineer may reasonably ask, “Why did we do it this way?”

Good candidates:
- architecture;
- technology selection;
- important data-model choices;
- security decisions;
- significant trade-offs;
- deliberate deviations from conventions.

Avoid decisions for trivial implementation choices.

## Index design

Indexes should be short enough to skim. Prefer each entry to contain:
- title;
- one-line purpose;
- link.

Add metadata such as status or last-reviewed date only when it improves navigation. Avoid duplicated prose and tangential links.

Useful relationship labels include:
- Related spec
- Related decision
- Related work item
- Depends on
- Supersedes

## Initialization behavior

When asked to initialize project knowledge:

1. Inspect the repository and existing `docs/`.
2. Preserve existing documentation.
3. Determine whether the project is still in idea/discovery mode.
4. If it is, `docs/idea.md` may be sufficient.
5. Otherwise create only useful directories and indexes.
6. Explain what was created and why.
7. Never create placeholder documents merely to satisfy the recommended tree.

## Documentation health review

When asked to review documentation health, look for:
- broken links;
- missing/stale indexes;
- orphaned documents;
- duplicate or competing canonical documents;
- specs containing excessive process/history chatter;
- decisions that exist only as discussions;
- dev logs that should be consolidated;
- references to deleted/renamed documents;
- outdated statements conflicting with newer knowledge;
- indexes too large to skim.

Prefer the smallest change that improves accuracy and navigability.

## Completion checklist

After substantive work, check:
- Behavior changed → update the relevant spec.
- Consequential decision occurred → create/update a decision.
- Meaningful implementation history occurred → create/update the work-item log.
- Documentation changed structurally → update indexes.
- Information was duplicated → consolidate and link.
- Historical/process chatter entered a spec → move or distill it.
- Navigation became harder → simplify it.
