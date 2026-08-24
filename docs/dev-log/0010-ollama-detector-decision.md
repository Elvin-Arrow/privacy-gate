# [0010] Ollama detector backend — draft, Gemini review, reconciliation, spec updates

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Add an optional Ollama-hosted Gemma detector backend that is preferred over the existing
in-process ONNX hybrid pipeline (decision 0004) when a compatible local Ollama is present,
falling back to the existing pipeline otherwise — per owner direction — without silently
reopening decision 0004's rejection of an in-process LLM detector, and without silently
loosening NFR-P1/C-5 ("on-device," "no network calls").

## Implementation

- Confirmed the locally-installed Ollama model via `ollama list`: **`gemma4:e2b`** (7.2 GB,
  local). Noted `gemma4:31b-cloud` is also present but is a cloud-relay tag, not eligible for
  an on-device guarantee.
- Drafted a decision at `/tmp/.../decision-0009-draft.md` covering identity split, backend
  selection, network boundary, output contract, model pin, chunking/budget, and downstream
  spec impact, with five open questions (OQ-A..E) left for review rather than guessed.
- Ran a structured critical review via `agy --effort high` (Gemini), scoped against decision
  0004, architecture.md, design.md, srs.md, testing.md, data-model.md, api.md, and dev-plan.md.
  Saved at `docs/notes/reviews/ollama-detector-gemini.md`.
- Gemini returned **"Reject as-is."** Findings: the RAM-budget reversal of decision 0004's
  in-process-Gemma rejection was "an accounting sleight-of-hand" (moving a 7.2 GB model to a
  sibling process doesn't remove memory pressure on an 8 GB machine); NFR-P1/C-5 are hardened
  elsewhere ("no network calls," not a literal "on-device" reading) so loopback HTTP needs an
  explicit named exception, not a reinterpretation; unauthenticated loopback HTTP is a local
  port-hijacking / ambient-proxy-leak risk; the draft had **no algorithm** for mapping Gemma's
  generative text output back to byte offsets (flagged CRITICAL BLOCKER); Gemma's context
  window can't hold a 1 MB+ document, and the draft didn't address chunking; CI runners won't
  have Ollama, so a test-double requirement was missing; and stuffing everything into one W15
  chunk violates the plan's single-scope-per-PR rule.
- Reconciled every finding into [decision 0009](../decisions/0009-ollama-detector-backend.md):
  a verify-then-trust, chunk-relative offset algorithm (never search, never fuzzy-match); a
  hardcoded local-tag allowlist with Ollama-reported digest pinning (rejects `-cloud` tags
  outright); IP-literal-only / no-DNS / no-ambient-proxy / handshake-before-content network
  rules, documented as a mitigation with an explicit accepted residual (mirroring the existing
  webview-heap residual in architecture §5.2); a two-tier warming/steady-state performance
  budget; a named, narrow SRS/architecture exception clause instead of a blanket
  reinterpretation; and a W15 → W15a/W15b/W15c split.
- Updated every downstream spec named in the decision's "Downstream spec impact" section:
  `srs.md` (FR-2.3, NFR-P1, C-5), `design.md` (§2.2, C-DES-2), `architecture.md` (§2.1
  process-model clarification, §2.3 new trust-boundary row, §4.2 digest-pin addendum, §5.2
  transient-plaintext exception, full §10 rewrite adding `pg-hybrid-ollama-v1`, C-ARCH-3),
  `data-model.md` (`Config.detector_preference`, audit `detect` payload fields), `api.md`
  (`get_detector_preference`/`set_detector_preference`, `pg://detect-progress` phase field,
  `import_document`'s detection-identity note), `testing.md` (§5.3 two new gated modules, new
  §7.4 detector-loopback-boundary test section, §10 Ollama mock fixture, §11 CI row),
  `dev-plan.md` (W15 split into W15a/b/c, map diagram, merge-train slice G), and
  `agent-roster.md` (W15 row replaced with three Opus-tier rows; W15b added to the
  mandatory-second-pass-review list).
- Marked decision 0004's header as partially superseded, pointing at 0009, while leaving its
  body text untouched (it remains the accurate record of what was decided and why at the time).

## Problems Encountered

- The user's initial shorthand ("Gemm4") was ambiguous between a typo for "Gemma 4" and an
  actual installed tag; asked a clarifying question, then verified directly via `ollama list`
  rather than trusting either guess — found `gemma4:e2b` (local) and `gemma4:31b-cloud`
  (cloud-relay, explicitly excluded from eligibility in the decision).
- The first draft's RAM-budget argument ("Ollama manages its own memory, so decision 0004's
  objection doesn't apply") was Gemini-flagged as not fully holding: an 8 GB laptop still
  feels a 7.2 GB model regardless of which process carries it. The final decision doesn't
  claim the RAM problem disappeared — it keeps `pg-hybrid-v1` as the unconditional fallback so
  the app's own guarantee (works with zero external dependencies) is untouched, and treats the
  Ollama path as opt-in capacity the user already chose to carry.
- The draft under-specified the single highest-risk piece of engineering here: turning
  free-text LLM output into trustworthy byte offsets. Resolved with a verify-known-value
  (not search) algorithm that has no cross-occurrence ambiguity by construction.

## Resolution

Decision 0009 is Accepted. All named downstream specs are updated and consistent with it.
Dev-plan chunking (W15a/b/c) and the agent roster are updated to match. No implementation of
W15a/b/c has started yet — this entry covers the specification work only.

## Related Documentation

- [Decision 0009 — Ollama detector backend](../decisions/0009-ollama-detector-backend.md)
- [Decision 0004 — v1 architecture](../decisions/0004-v1-architecture.md) (superseded in part)
- [Gemini pre-acceptance review](../notes/reviews/ollama-detector-gemini.md)
- [Spec — SRS](../specs/srs.md), [design](../specs/design.md),
  [architecture](../specs/architecture.md), [data model](../specs/data-model.md),
  [API](../specs/api.md), [testing](../specs/testing.md)
- [dev-plan.md](../dev-plan.md), [agent-roster.md](../agent-roster.md)
