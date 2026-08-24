# Decision: Ollama-hosted Gemma as a constrained, optional primary detector; ONNX hybrid stays the always-available fallback

- **Status:** Accepted
- **Date:** 2026-08-23

## Context

Decision 0004 (§"Detector identity"; "Alternatives Considered → In-process Gemma") rejected an
in-process 2B+ parameter LLM as the v1 detector, against the 8 GB RAM laptop budget and the
≤5 s / 1 MB detection budget (design.md §7), and pinned `pg-hybrid-v1` (deterministic UK
pattern pack + in-process GLiNER-small-v2.1 INT8 ONNX) instead. That decision states:
"Replacing GLiNER-small-v2.1 ... requires a new decision, not a silent implementation change."
This document is that decision.

Product direction (owner instruction, 2026-08-23): when **Ollama** is present on the machine,
prefer it to run a local Gemma model as the detector's NER stage, using a model the user already
has installed locally — verified on the development machine as **`gemma4:e2b`** (`ollama list`,
7.2 GB, local). **When Ollama is absent**, fall back to the existing in-process ONNX hybrid
pipeline from decision 0004, unchanged.

A draft of this decision was reviewed by Gemini (`agy --effort high`) against decision 0004 and
every downstream spec. That review (`docs/notes/reviews/ollama-detector-gemini.md`) returned
**"Reject as-is"** on the first draft: it identified that this reopens the in-process-Gemma
rejection on grounds that don't fully hold (moving RAM to a sibling process doesn't remove
memory pressure on an 8 GB machine), that `NFR-P1`/`C-5` are hardened elsewhere to "no network
calls" more strictly than a literal "on-device" reading, that unauthenticated loopback HTTP is a
local trust-boundary gap, that the draft had no algorithm for mapping generative text output
back to byte offsets, and that it ignored Gemma's context-window limits against the 25 MB
document budget. This decision incorporates concrete resolutions to every one of those findings.

## Decision

### 1. Two detector identities, one always available

- **`pg-hybrid-v1`** (decision 0004, unchanged): `pg-patterns-uk-v1` + in-process
  GLiNER-small-v2.1 ONNX. Remains the zero-external-dependency baseline; ships everything it
  needs; works with Ollama never installed.
- **`pg-hybrid-ollama-v1`** (new): `pg-patterns-uk-v1` (same stage, unchanged) + NER via a
  **local Ollama server**, subject to every constraint in §2–§6 below.

### 2. Backend selection — probed per detect, not cached at unlock

`Config` (data-model.md §5.5) gains `detector_preference: "auto" | "bundled_only"`, factory
default `"auto"` (§9 below has the exact schema/command shape).

At the start of each `import_document` detect phase:

1. If `detector_preference == "bundled_only"` → use `pg-hybrid-v1`. No Ollama probe.
2. Else, probe: `GET` on the **IP-literal** loopback socket `127.0.0.1:11434` (Ollama's
   documented default), **200 ms** timeout. The probe and every subsequent call in this
   session use an HTTP client built with proxying explicitly disabled (`no_proxy`) — see §3.
3. If unreachable, times out, or the response fails schema verification (§4) → `pg-hybrid-v1`.
   Record `fallback_reason` in the audit `detect` event (§9).
4. If reachable, verify the served model is on the pinned local allowlist (§5) via
   `/api/show`. If not → `pg-hybrid-v1`, `fallback_reason: "model_not_allowlisted"`.
5. Otherwise → `pg-hybrid-ollama-v1`. Any Ollama-side failure *during* detection (connection
   drop, malformed generation, digest change mid-session) fails that document's Ollama pass
   and falls back to `pg-hybrid-v1` for that document — never a partial, unverified result.

Probing per-detect (not once per unlocked session) means a user who starts Ollama mid-session
is picked up on the next import without a re-lock; the 200 ms timeout keeps this cheap when
Ollama isn't running.

### 3. Network boundary (preserves the spirit of NFR-P1 / C-5 — see §8 for the wording amendment)

- **IP-literal only.** The core connects to `127.0.0.1` (or `::1`), never to the hostname
  `localhost` — no DNS resolution occurs for this feature, ever.
- **No ambient proxy.** The HTTP client is built with proxy environment variables
  (`HTTP_PROXY`, `ALL_PROXY`, etc.) explicitly disabled, mirroring the existing rule for Cloud
  AI (architecture §9.2: "no ambient proxy credentials from the app").
- **Handshake before content.** Before any document text is sent, `/api/tags` and `/api/show`
  responses are verified against Ollama's documented shape and the pinned digest (§4, §5). A
  listener that does not speak Ollama's actual API fails this check and the app falls back.
  This is a **mitigation, not a guarantee** against local port-squatting — recorded as an
  accepted residual risk (same house style as architecture §5.2's webview-heap residual): a
  sufficiently capable local attacker who can bind `127.0.0.1:11434` *and* replay a
  byte-correct Ollama API could still intercept a detect call. This risk is bounded by the
  existing v1 threat model (architecture §2.4), which already excludes a fully compromised
  local machine from its guarantees.
- This entire boundary (IP-literal-only, no-DNS, no-proxy, handshake-verified) is a new
  **gated mutation-testing module, S = 1.00** (testing §5.3), and is checked by tests separate
  from the OQ-6 share-egress oracle — OQ-6 governs what a *share* transmits; this governs what
  the *detector* may reach at all. See §10.

### 4. Output contract and the offset-mapping algorithm (resolves the reviewer's "critical blocker")

GLiNER is a span-classification model; Gemma is generative. Free-text output cannot be safely
mapped back to `[byte_offset, byte_length]` by searching for the returned substring — a
document with the same name/number repeated many times makes that search ambiguous, and model
text normalization can break exact matching entirely.

Instead, **verify-then-trust against the exact chunk the model was given**, never search:

1. Document text is split into bounded chunks (§6) with each chunk's absolute byte offset in
   the document recorded.
2. Each chunk is sent to Ollama with a system prompt requiring **strict JSON-schema-constrained
   output** (Ollama's structured-output / grammar-constrained decoding, not best-effort
   `format: "json"`): an array of `{ start: u32, length: u32, label: enum, text: string }`,
   where `start`/`length` are **relative to the chunk text exactly as sent**.
3. For each returned entity, verify byte-exact:
   `chunk_text[start .. start+length] == text`. This is a plain equality check against the
   *known* chunk, not a search — there is no cross-occurrence ambiguity because the model was
   never asked to search, only to point at what it was given.
4. **Pass** → accept, map to the document-absolute offset (`chunk_start + start`).
   **Fail** → reject that one entity (not the whole chunk); count rejections.
5. If a chunk's rejection rate exceeds a fixed threshold (implementation constant, tuned
   against the fixture corpus in testing.md §10), treat the **whole document's** Ollama pass
   as failed → fall back to `pg-hybrid-v1` for that document, `fallback_reason:
   "offset_verification_failed"`.

Never attempt fuzzy matching, nearest-occurrence guessing, or partial acceptance of an
unverified span. A rejected entity is a silent-fail-open risk (an undetected field); the
per-chunk threshold plus whole-document fallback is what keeps that bounded and auditable
rather than silent.

### 5. Model pin: hardcoded local-tag allowlist, digest-verified

- The architecture spec's implementation carries a **small, hardcoded allowlist** of Ollama
  model tags eligible for `pg-hybrid-ollama-v1`, starting with `gemma4:e2b` (the tag verified
  present on the development machine). Extending the allowlist is an architecture amendment,
  the same discipline decision 0004 already applies to GLiNER-small-v2.1's SHA-256 pin.
- Any tag with a `-cloud` suffix (Ollama's cloud-relay models, e.g. the `gemma4:31b-cloud` tag
  also present on the dev machine) is **never** eligible — those route through Ollama's own
  remote relay, which would violate on-device detection outright, not just its wording. The
  allowlist check and the cloud-suffix rejection are the same code path: an unrecognized or
  cloud-suffixed tag is treated identically to "Ollama absent."
- At `/api/show` time, the core additionally records the model's Ollama-reported digest and
  compares it against the digest pinned for that allowlist entry (recorded at implementation
  time, the same discipline as the GLiNER SHA-256 pin in architecture §4.2). A digest mismatch
  (the user has since pulled a different build under the same tag) is a hard fallback, not a
  silent re-pin — `fallback_reason: "digest_mismatch"`.

### 6. Chunking and the performance budget (two-tier: warming vs. steady-state)

- Gemma's context window is far smaller than a 25 MB / ~350k-token document. The document text
  is processed as a **sliding window** of bounded chunks with an overlap region sized to be
  larger than any plausible single-entity span, so no entity is split across a chunk boundary
  without appearing whole in at least one chunk. Exact chunk size and overlap length are
  implementation constants tuned against the pinned tag's **verified** context window — the
  architecture spec requires that verified number be recorded before the Ollama path ships to
  production (nightly job, §10), rather than this decision asserting an unverified figure.
- Detections from overlapping chunk regions are de-duplicated by absolute-offset containment:
  when two chunks report the same label at the same (or containing) absolute span, keep one,
  preferring the detection further from its chunk's edge (less likely to be a boundary
  truncation artifact).
- **Two-tier performance budget**, distinct from design.md §7's existing ≤5 s/1 MB figure
  (which continues to govern `pg-hybrid-v1` unchanged):
  - **Cold-start / "warming"**: Ollama loading the model into its own memory before first use
    this session. Budget: ≤ 20 s. Surfaced to the UI as a distinct phase (see §9).
  - **Steady-state per-chunk detection**: measured and recorded (not asserted here) against
    the pinned tag on the nightly golden job (§10), the same pattern as the existing ONNX
    nightly golden (testing §11). If steady-state throughput cannot support the design.md §7
    interactive budget on realistic document sizes, that is an architecture amendment to this
    decision, not a silent regression the app ships anyway.

### 7. Process model clarification

Architecture §2.1's "one OS process... no sidecar, daemon, or helper process" describes what
**this app spawns and manages**. It spawns nothing new here: Ollama, when used, is a
pre-existing, independently-installed, independently-managed local service the user runs on
their own account — never bundled, launched, or supervised by Privacy Gate. The app's TCB
process boundary is unchanged; what changes is that the TCB may, under the constraints above,
make a constrained outbound call to a service outside that process.

### 8. SRS / design / architecture wording amendment (resolves the reviewer's finding B)

The reviewer's position, adopted here: a literal reading of "shall not leave the device" could
technically admit loopback traffic, but design.md §2.2 / C-DES-2 and architecture C-ARCH-3
already harden this to **"no network calls,"** and that hardened reading is what the rest of
the spec suite (trust-boundary table, transient-plaintext table) was written against. Loosening
it needs an explicit, narrow, named exception — not a reinterpretation of the existing words.

- **SRS FR-2.3, NFR-P1, C-5** gain an explicit, bounded exception clause: detection may reach a
  **pre-existing local service over a strictly loopback, IP-literal, non-DNS, non-proxied
  connection**, subject to architecture §10's pin/allowlist/verification rules, and **only**
  for the optional Ollama backend — not a general license for the core to make network calls.
- **design.md §2.2 / C-DES-2** and **architecture C-ARCH-3 / §2.3's trust-boundary table**
  gain the same bounded exception, with §2.3 getting a **new, separate table row** ("Rust core
  ↔ local Ollama (loopback only)") rather than folding this into the existing "Rust core ↔
  network" row that governs Cloud AI — these are different trust boundaries with different
  rules (Cloud AI sends *approved* content only; the Ollama path sends *unapproved* document
  content, which is why the extra verification machinery in §3–§5 exists).

Exact wording lands in each spec file directly (§ list in "Downstream spec impact" below); this
decision is the rationale record, the specs remain the implementable source per house
convention (decision 0004's own "Consequences" precedent).

### 9. Audit honesty and API surface

- `Config`: `detector_preference: "auto" | "bundled_only"`, factory `"auto"`. New commands
  `get_detector_preference` / `set_detector_preference`, same shape as
  `get_retention_default` / `set_retention_default` (api.md §5.2).
- Audit `detect` event payload (data-model.md §5.8.1-equivalent) gains: `backend: "ollama" |
  "onnx"`, `model_tag: string | null` (e.g. `"gemma4:e2b"`), `fallback_reason: string | null`
  (`"ollama_unreachable" | "model_not_allowlisted" | "digest_mismatch" |
  "schema_verification_failed" | "offset_verification_failed" | null`). Never a synthesized
  "hybrid detector ran" that hides which backend actually produced the result — the same
  document detected on two machines (one with Ollama, one without) may legitimately produce a
  different field set, and the audit trail must make that visible.
- `pg://detect-progress` (api.md §6) gains an additive `phase: "warming_model" | "detecting"`
  field alongside the existing `fraction`, so the UI (later chunk) can show a distinct
  "warming up the local model" state instead of a stalled progress bar during Ollama cold-start.

### 10. Testing and CI

- New fixtures (testing.md §10): an Ollama HTTP mock double (in the same style as the existing
  network mock for Cloud AI) simulating `/api/tags`, `/api/show`, `/api/generate` — success,
  timeout, malformed JSON, digest mismatch, cloud-tag rejection, and offset-verification
  failure. CI never requires real Ollama or real model weights.
- New nightly/release-only job (testing.md §11), parallel to the existing "ONNX golden + model
  pin" row: a real-Ollama golden run (only on a runner that has Ollama + the pinned tag
  available; otherwise informational, not blocking) that also records the steady-state
  per-chunk throughput figure §6 requires before the Ollama path ships to production.
- New gated mutation modules (testing.md §5.3, S = 1.00): the offset-verification algorithm
  (§4) and the loopback/allowlist/digest enforcement (§3, §5) — same tier as the existing
  Cloud AI host-allowlist and envelope AAD gates, because a silent bypass here is exactly the
  "sensitive field left unredacted" or "document content reached an unverified local listener"
  failure mode this project's gated-module list already exists to catch.

### 11. Dev-plan chunking

`W15` (single chunk in the existing plan) is split to keep each PR single-scoped, per dev-plan
§1's own rule ("do not pull in the next chunk's behaviour"):

- **W15a — Hybrid ONNX (`pg-hybrid-v1`)**: exactly the original W15 scope, unchanged.
- **W15b — Ollama backend (`pg-hybrid-ollama-v1`)**: HTTP client (loopback-literal, no-DNS,
  no-proxy), handshake/allowlist/digest verification, chunking + offset-verification
  algorithm, warming-phase progress payload.
- **W15c — Backend selection + fallback orchestration**: `detector_preference` config +
  commands, per-detect probe/selection logic (§2), audit fields (§9).

All three stay **Opus tier** in `docs/agent-roster.md` — if anything this raises the stakes
over the original single W15, since W15b now carries a new local trust boundary and a
correctness-critical offset-verification algorithm, both exactly the class of bug this
project's own spec reviews have caught before (crash-window bricking, DEK-erasure oracle,
AAD collisions).

## Rationale

- Ollama manages model lifecycle and (optionally) GPU offload outside the app's own process —
  the reviewer correctly noted this doesn't remove memory pressure on an 8 GB machine wholesale
  (a 7.2 GB model is still real RAM), but it does mean the app ships nothing extra and the
  choice to run a large local model is the user's own (they already had Ollama + the model
  installed), not a bundling decision Privacy Gate makes for every install. The always-available
  `pg-hybrid-v1` fallback is what keeps decision 0004's core guarantee — works with zero
  external dependencies — fully intact regardless.
- Verify-then-trust (§4) is what makes a generative model's output safe to redact on: the app
  never trusts an offset it hasn't independently checked against the exact text the model saw,
  which closes the duplicate-occurrence and text-mutation failure modes the reviewer identified
  without needing fuzzy matching or heuristics.
- A named, narrow SRS/architecture exception (§8) — rather than reinterpreting "on-device" or
  "no network calls" wholesale — keeps every other place those invariants are already relied on
  (Cloud AI's network boundary, the trust-boundary table, the transient-plaintext table)
  unchanged and legible.

## Alternatives Considered

### Loosen NFR-P1 / C-5 globally instead of a named exception

Rejected: every other spec (Cloud AI network path, trust-boundary table) is written against
the hardened "no network calls" reading; a blanket reinterpretation would silently widen what
those specs already forbid elsewhere.

### Trust Ollama's returned offsets without verification

Rejected outright by the reviewer's finding (C.4, F.1) — this is a silent fail-open path to
unredacted sensitive fields. The verify-then-trust algorithm (§4) is required, not optional.

### Ship the Ollama path as a single W15 chunk

Rejected: violates dev-plan §1's single-scope-per-PR rule once the HTTP client, verification
machinery, chunking, and fallback orchestration are all accounted for. Split per §11.

### No fallback allowlist; trust whatever Gemma tag Ollama reports

Rejected: decision 0004's own precedent ("pinning the artifact prevents silent model swaps")
applies identically here, and a `-cloud` tag would silently violate on-device detection, not
just its wording.

## Consequences

- `docs/specs/srs.md` (FR-2.3, NFR-P1, C-5), `docs/specs/design.md` (§2.2, C-DES-2),
  `docs/specs/architecture.md` (§2.1, §2.3, §4.2, §5.2, §10), `docs/specs/data-model.md`
  (§5.5 `Config`, audit `detect` payload), `docs/specs/api.md` (§5.2 config commands, §6
  `pg://detect-progress`), `docs/specs/testing.md` (§5.3, §10, §11), `docs/dev-plan.md` (W15
  split into W15a/b/c, merge-train slice G), and `docs/agent-roster.md` (W15a/b/c rows) are all
  updated to match this decision.
- Decision 0004's detector-identity clause is **superseded in part**: `pg-hybrid-v1` is no
  longer the sole detector identity, but remains defined exactly as before and is still the
  always-available baseline. Decision 0004's RAM-budget objection to in-process Gemma is not
  overturned — it still correctly forbids embedding a multi-GB model *in the app's own
  process*; this decision is compatible with that because Ollama is never in-process.
- The offset-verification algorithm, the loopback/allowlist/digest boundary, and the
  fallback-selection logic are new mutation-gated (S = 1.00) modules.
- A future third Gemma tag or a non-Ollama local-LLM runtime requires a new decision, not a
  silent allowlist edit.

## Related Documentation

- [Decision 0004 — v1 architecture](./0004-v1-architecture.md) (superseded in part)
- [Decision 0003 — v1 tech stack](./0003-v1-tech-stack.md)
- [Gemini pre-acceptance review](../notes/reviews/ollama-detector-gemini.md)
- [Spec — SRS FR-2.3, NFR-P1, C-5](../specs/srs.md)
- [Spec — design §2.2, C-DES-2, §7](../specs/design.md)
- [Spec — architecture §2.1, §2.3, §4.2, §5.2, §9, §10](../specs/architecture.md)
- [Spec — data model §5.5 Config](../specs/data-model.md)
- [Spec — API §5.2, §6](../specs/api.md)
- [Spec — testing §5.3, §7, §10, §11](../specs/testing.md)
- [dev-plan.md W15a/b/c](../dev-plan.md)
- [agent-roster.md](../agent-roster.md)
