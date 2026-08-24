# Agent roster — model assignment per dev-plan chunk

> Non-authoritative execution aid. Maps each [`dev-plan.md`](./dev-plan.md) chunk (W0–W39) to a
> recommended model tier, so implementation effort is spent where correctness risk is highest.
> If this and `dev-plan.md` disagree on what a chunk *does*, `dev-plan.md` wins — this file only
> advises *who* should build it.

## Tier definitions

| Tier | Use for | Why |
|---|---|---|
| **Opus** | Crypto, key handling, deletion/destruction, audit-chain integrity, redaction correctness, detector precedence/overlap logic, mutation-gate hardening, the AC-1..AC-7 acceptance pack | Wrong-by-default failure mode here is a privacy leak or a bricked vault. Needs the most rigorous adversarial reasoning, not just "tests pass." |
| **Sonnet** | Everything else that is genuine logic: import pipeline, session gating, IPC wiring, plugin host, share/export flow, most spec-to-code translation | Default work-horse — solid reasoning, cheaper/faster than Opus, no adversarial-security judgment required. |
| **Haiku** | UI screens once the design/API contract is fixed, boilerplate event plumbing, repetitive test scaffolding, docs/log updates | Mechanical translation of an already-specified contract (`ui.md`, `api.md`) into Svelte components; low judgment required. |

## Chunk-by-chunk roster

| Chunk | Task | Model | Rationale |
|---|---|---|---|
| W0 | Repo skeleton | **Haiku** | Pure scaffolding, no domain logic. |
| W1 | Envelope crypto primitives | **Opus** | Wrong AAD/nonce reuse/key derivation = silent, unrecoverable compromise. |
| W2 | Account, keystore, session | **Opus** | OS-keystore integration + passphrase wrapping is TCB; crash/lockout bugs here brick the vault (per decision 0004's crash-window finding). |
| W3 | Empty vault (SQLCipher) | **Sonnet**, Opus review | Mostly plumbing over a well-defined schema, but touches at-rest encryption — cheap to have Opus sanity-check the diff. |
| W4 | Session gating table | **Sonnet** | State-machine logic against `api.md`, not novel crypto. |
| W5 | Audit chain and integrity | **Opus** | Hash-chain + HMAC + degraded-session logic is exactly the "two-phase commit isn't atomic" class of bug Gemini already caught once (dev-log 0003). |
| W6 | Retention config (AC-7 core) | **Sonnet** | Business-rule logic (decision 0007), not TCB-crypto. |
| W7 | Linux keystore fallback | **Sonnet** | Platform-integration work, contained blast radius. |
| W8 | Import plain text | **Sonnet** | Straightforward pipeline stage. |
| W9 | Import PDF (text-bearing), reject scans | **Sonnet** | Parsing/validation logic, well-bounded. |
| W10 | Catalog + `import_document` | **Sonnet** | CRUD over data-model schema. |
| W11 | Import blocked until retention confirmed | **Sonnet** | Gate logic, low ambiguity. |
| W12 | Detector host + stub | **Sonnet** | Interface/host wiring; stub has no real detection risk yet. |
| W13 | Pattern pack `pg-patterns-uk-v1` | **Sonnet** (Opus spot-check on regex edge cases) | Regex/pattern authoring; false negatives are a privacy miss worth a second look but not full Opus depth. |
| W14 | `pg://detect-progress` event | **Haiku** | Simple event emission, contract already fixed by `api.md`. |
| W15a | Hybrid ONNX (`pg-hybrid-v1`) | **Opus** | Model-integration correctness directly feeds AC-1 (what gets flagged as PII); errors are undetectable false negatives. |
| W15b | Ollama backend (`pg-hybrid-ollama-v1`) — HTTP client, handshake/allowlist/digest verification, offset-mapping algorithm (decision 0009) | **Opus** | New local trust boundary plus a correctness-critical verify-then-trust offset algorithm — a silent bypass here is a fail-open privacy bug, the exact class of issue this project's own reviews keep catching. |
| W15c | Backend selection + fallback orchestration (decision 0009) | **Opus** | Fallback correctness and audit honesty (never hide which backend ran) — same tier reasoning as W17-class precedence/state chunks. |
| W16 | Approval session | **Sonnet** | Session/state management. |
| W17 | Overlap / nested fields (design §3.5) | **Opus** | Precedence logic across overlapping spans — subtle, and getting it wrong silently under- or over-redacts. |
| W18 | `submit_approval` (AC-1 core) | **Opus** | Core acceptance criterion; this is the command the whole trust model hangs on. |
| W19 | `abort_approval`, lock vs retention | **Sonnet** | Well-bounded state transition, once W17/W18 land. |
| W20 | `delete_document` (DEK destroy) | **Opus** | Destruction correctness — same class of bug testing spec 0005 flagged (decrypting leftover ciphertext with a pre-copied DEK). |
| W21 | `delete_retained_original` | **Opus** | Same destruction-correctness risk as W20, smaller surface. |
| W22 | Variants | **Sonnet** | CRUD-shaped, no key destruction. |
| W23 | PDF re-render (true removal) | **Opus** | This is where "redacted" must mean the bytes are actually gone, not just visually covered — highest-consequence correctness bug in the whole plan. |
| W24 | Share preview + commit (export) | **Sonnet** | Pipeline orchestration once W23 is trustworthy. |
| W25 | OQ-6 egress oracle | **Opus** | It's the independent verifier for "no plaintext left the device" — the oracle itself must not have false negatives. |
| W26 | Ephemeral overrides + variants on share (AC-2) | **Sonnet** | Applies already-validated primitives. |
| W27 | Cloud AI plugin (mock HTTP) | **Sonnet** | Plugin host API + allowlisted egress; mocked, so lower stakes than W25. |
| W28 | `list_audit_events` (AC-4) | **Haiku** | Read-only query/projection over an already-correct audit chain. |
| W29 | Tauri IPC, CSP, events | **Sonnet** | Wiring against a fixed API surface. |
| W30 | UI: first run, lock, unlock | **Haiku** | Svelte screens against a finished `ui.md` spec. |
| W31 | UI: Settings | **Haiku** | Same — form-shaped UI. |
| W32 | UI: vault, first-import modal | **Haiku** | Same. |
| W33 | UI: approval | **Sonnet** | More interaction complexity (span selection, overlap display) — worth the extra reasoning. |
| W34 | UI: share, preview, save dialog | **Sonnet** | Native save-dialog chrome + preview correctness (OQ-4) has UX-security overlap. |
| W35 | UI: audit + integrity failure | **Haiku** | Display-only once backend is correct. |
| W36 | UI: variants + Cloud AI share confirm | **Haiku** | Same. |
| W37 | Acceptance pack AC-1..AC-7 | **Opus** | This is the gate proving every above chunk actually holds; needs adversarial test design, not implementation speed. |
| W38 | Mutation gate | **Opus** | `cargo-mutants` survivors need genuinely adversarial thinking to kill — mechanical test-padding won't cut it. |
| W39 | Perf + no-plaintext watcher jobs | **Sonnet** | Well-specified budgets (design §7) and a defined watcher contract — verification work, not novel judgment. |

## Policy notes

- Every Opus chunk gets a Sonnet-authored red phase first (tests from `testing.md`), then Opus
  writes/reviews the green phase — keeps the adversarial model focused on the part that matters.
- **W17, W18, W20, W21, W23, W25, W15b** get a mandatory second-pass review even after Opus
  writes them — the first six map directly to the four "critical" bugs the three-model spec
  reviews already caught once (dev-log 0002, 0003, 0005); W15b was itself rejected once on
  first draft by Gemini review (decision 0009) for exactly this class of gap (unverified
  offsets, an unauthenticated local trust boundary). History suggests this exact seam is where
  this project's bugs live.
- UI chunks (W30–W36) are Haiku by default *only* because `ui.md` is a finished, reviewed spec —
  if it's still being interpreted rather than transcribed, bump to Sonnet.
- Don't run W15a/W15b/W15c (detector: ONNX, Ollama, selection) or W38 (mutation gate) on Haiku
  or Sonnet even to save cost — both have a track record in this project's own spec reviews of
  hiding correctness bugs behind passing tests.

## Related documents

- [dev-plan.md](./dev-plan.md) — the chunks this roster assigns.
- [decisions/0006-tdd-and-mutation-testing.md](./decisions/0006-tdd-and-mutation-testing.md)
- [specs/testing.md](./specs/testing.md)
