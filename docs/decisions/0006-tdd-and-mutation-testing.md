# Decision: TDD and mutation testing for v1

- **Status:** Accepted
- **Date:** 2026-08-23

## Context

Privacy Gate's trusted computing base is a Rust core that encrypts vault contents, redacts
exports, and is the only process allowed to talk to the network (Cloud AI). Line coverage on
that code is a weak signal: a test can execute a redaction path and still not notice that a
mutated overlap rule or a skipped DEK destroy still "passes." The project owner directed that
development follow TDD and that the testing spec include mutation testing.

## Decision

1. **Test-driven development** is the required implementation method for v1 core work (the
   Rust TCB and any TypeScript that is more than view wiring). A behaviour change starts with
   a failing test that names the SRS/design/architecture/API clause it protects, then the
   minimum code that makes it pass, then refactor.
2. **Mutation testing** is a required quality gate on the Rust TCB, using **`cargo-mutants`**.
   A surviving mutant in a gated module is a CI failure unless it is classified equivalent
   and excluded by annotation. Mutation score is the gate, not statement coverage.
3. The concrete suites, oracles, CI jobs, and module list live in
   [`docs/specs/testing.md`](../specs/testing.md).

## Rationale

- TDD forces the API/architecture contracts to be executable before the implementation can
  drift. That matches a product whose bugs are privacy failures, not merely UX defects.
- Mutation testing asks "would this test fail if the redaction rule, HMAC check, or delete
  path were wrong?" Coverage cannot ask that.
- `cargo-mutants` is the maintained Rust tool; it works with ordinary `#[test]` functions
  produced by TDD. The TCB is Rust (decision 0003 / C-ARCH-1), so the v1 mutation gate is
  Rust-only. Non-trivial business logic in the webview would itself be a C-DES-1 violation;
  StrykerJS is therefore not a v1 gate.

## Alternatives Considered

### Coverage threshold only (e.g. 90% lines)

Rejected: executes code without proving the assertions constrain security behaviour.

### Mutation testing without TDD

Rejected: a late mutation run on tests written after the fact tends to produce equivalent
mutants and weak tests that were never intended to fail. TDD first, mutants as the audit.

### Mutate TypeScript as well as Rust in v1

Rejected for the gate. The webview is untrusted view code. If a module there is worth
mutating, it probably belongs in the core. UI spec may add Stryker later for view-model
helpers; that is not this decision.

### Property testing *instead of* mutation testing

Rejected as a substitute. Property tests (overlap rule, canonical encoding) are encouraged
inside TDD and pair well with mutants; they do not replace them.

## Consequences

- Implementers write the failing test before the production function. "Tests after" is a
  spec violation for TCB changes, not a style preference.
- CI cannot prove the red phase from git history. Pull requests attest TDD for TCB diffs;
  reviewers reject tests-after on gated modules. `cargo-mutants` is the automated audit of
  test strength (testing spec §5).
- Gated TCB modules have no unexplained surviving mutants (equivalent mutants annotated
  with a reason). Other core modules: mutation score ≥ 0.70.
- CI must run `cargo-mutants --file` on the gated module list in the testing spec. Slow
  full-tree mutation is nightly; gated modules remain PR-blocking.
- Replacing `cargo-mutants` or dropping the mutation gate requires a new decision.

## Related Documentation

- [Spec — testing](../specs/testing.md)
- [Decision 0003 — v1 tech stack](./0003-v1-tech-stack.md)
- [Decision 0004 — v1 architecture](./0004-v1-architecture.md)
