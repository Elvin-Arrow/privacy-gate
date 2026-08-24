# Decision: Retention default is discard; first import must confirm it (OQ-14)

- **Status:** Accepted
- **Date:** 2026-08-23

## Context

OQ-14 asked what a fresh vault's global retention default is (retain vs discard). The Aisha
user story previously implied retain; `idea.md` did not specify. Privacy Gate's product
intent is local-first and minimize leftover originals. The owner directed: the default is
**discard originals**, and the **first time the user uploads a document** they must be asked
to set the default retention policy.

## Decision

1. **Factory value** of the global retention default is `discard` (not `retain`, not
   `never_retain`).
2. That value is **unconfirmed** until the user explicitly sets a policy (`set_retention_default`
   or equivalent UI that calls it). Account creation does not count as confirmation.
3. **No import succeeds** until the policy is confirmed (`import_document` returns
   `retention_policy_unset`). The first-upload prompt is how most users confirm; Settings may
   confirm earlier. Dialog chrome is UI spec; the gate is API/design.
4. The first-upload prompt **pre-selects discard**. The user may confirm discard, choose
   retain, or choose paranoid `never_retain`. After confirmation, FR-1.4 overrides apply as
   already specified (decision 0002 / Q9).
5. Changing the confirmed global default later remains allowed, including leaving
   `never_retain`.

## Rationale

- Discard-by-default matches the product: originals are extra risk; keeping them is an
  explicit choice.
- A silent factory default would hide that choice. Forcing a confirmation on first upload
  makes the policy a user decision without blocking account creation or unlock.
- Pre-selecting discard (rather than a blank radio) encodes the product default in the prompt
  without pretending the user already agreed.

## Alternatives Considered

### Silent discard, no first-import prompt

Rejected: users who expected Aisha-style retain would lose originals without noticing.

### Factory retain (user-story implication)

Rejected: owner direction is discard; the user story is updated to match.

### Treat first import's per-document override as the global default

Rejected: FR-1.3 (per-doc) and FR-1.4 (global) stay distinct. The prompt sets the **default**;
that import may still pass a per-document override after the default is confirmed.

## Consequences

- `idea.md`, SRS FR-1.4, design Config, API config/import, testing AC coverage, and the
  Aisha story are updated. OQ-14 is resolved.
- UI spec still owns prompt wording and layout.
- Tests must cover unconfirmed → `retention_policy_unset`, factory `discard`, and
  confirmation before import.

## Related Documentation

- [Spec — SRS](../specs/srs.md)
- [Spec — design](../specs/design.md)
- [Spec — API](../specs/api.md)
- [Spec — testing](../specs/testing.md)
- [idea.md](../idea.md)
- [Open questions](../notes/open-questions.md)
- [Work item](../dev-log/0006-oq14-retention-default.md)
