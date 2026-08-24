# Open questions — Privacy Gate SRS

> Non-canonical register of gaps surfaced while writing `docs/specs/srs.md` (and by subsequent
> reviews — see `docs/notes/reviews/`). These are **not requirements**. Status is per item:
> most are resolved by design, architecture, API, testing, or UI specs.
>
> Q8–Q11 and the Q4 PDF-bundle portion live in `docs/decisions/0002-resolved-srs-clarifications.md`.

## Format

Each entry: ID, question, where it bites in the SRS, owner (which downstream spec should resolve
it), status.

## Questions

- **OQ-1** *(was Q1)* Which desktop OSes does v1 support? Bites: NFR-PORT1. Owner: design spec.
  Status: **resolved** by decision 0003 + design spec §6 C-DES-7 — macOS, Windows, Linux.
- **OQ-2** *(was Q2)* Concrete performance thresholds for "interactive" detection and approval
  review (e.g., latency bounds for given document sizes). Bites: NFR-PERF1. Owner: design +
  Status: **resolved** by design spec §7 (budgets) + testing spec §8 / §11 (perf job
  enforcement; PR CI stays non-flaky).
- **OQ-3** *(was Q3)* Audit-trail integrity mechanism. NFR-R1 requires tamper-evidence
  (post-creation modification detectable) but leaves the mechanism — append-only log, hash chain,
  signatures — to design. Owner: architecture spec. Status: **resolved** by decision 0004 +
  architecture spec §6 — SHA-256 hash chain, HMAC-SHA-256, canonical encoding v1, batched
  keystore head, crash-window fast-forward, degraded session on true integrity failure.
- **OQ-4** *(was Q4, remainder)* Export format details beyond the single-PDF-bundle decision:
  single-document export format, document ordering within a bundle, naming, same-as-source vs.
  always-PDF. The multi-doc single-PDF-bundle question is resolved (decision 0002). Bites:
  FR-5.1. Owner: design + API + UI specs. Status: **resolved** — design spec §3.7 (PDF +
  selection order); `docs/specs/api.md` §7 (suggested filename + PDF info dictionary);
  `docs/specs/ui.md` §10.4 (native save dialog, default name, documents folder, cancel = no
  commit).
- **OQ-5** *(was Q5)* Is the v1 "account" local-only, or does account creation talk to a server
  for identity? The idea doc says an account exists but no network identity is needed day-to-day.
  Bites: §2.3, FR-8.1, FR-8.3. Owner: architecture + design specs. Status: **resolved** by
  decision 0004 + architecture spec §7 — local-only; no server at first run or unlock; later
  network identity is an additive binding.
- **OQ-6** *(was Q6)* Precise meaning of the audit-trail assertion "no private originals left the
  device" when originals are retained. Bites: FR-7.2. Owner: design + testing specs. Status:
  **design part resolved** by design spec §2.6 — assertion is true iff retention was "discard" or
  the share transmits only the approved version. **Verification resolved** by testing spec §7 —
  egress spy + high-entropy canary oracle; do not trust the flag alone.
- **OQ-7** *(was Q7)* Variant lifecycle: can variants be deleted, edited, scoped to a document or
  global? Bites: FR-5.5. Owner: design + UI specs. Status: **resolved** by design spec §3.4 —
  create / apply / delete; no edit; per-document scope.
- **OQ-12** *(was Q12)* Cloud AI plugin authentication. The app holds no keys off-device (C-4) and
  is local-first, yet the plugin sends content to a cloud model. How does it authenticate
  (user-supplied API key stored where? bundled credential?)? Bites: FR-5.2, C-4. Owner: API +
  architecture specs. Status: **resolved** by decision 0004 + architecture spec §9 + API spec
  §5.7 — user-supplied API key, envelope-encrypted in the vault; HTTP only from Rust to an
  allowlisted host; no bundled credential; commands set/get/clear/test; key never in outputs.
- **OQ-13** *(was Q13)* Plugin security model. The idea doc requires the architecture to support
  future third-party plugins "without rework," but does not say whether v1 must already include
  signing/sandboxing infrastructure. Bites: FR-9.5, NFR-E1. Owner: architecture spec. Status:
  **resolved** by decision 0004 + architecture spec §8 — versioned host API `pg-host-api-1`; v1
  in-process first-party; later WASM maps the same API; no signing PKI in v1.
- **OQ-14** *(was Q14)* Retention default initial value (retain vs. discard out of the box). The
  user story implied "retain"; the idea doc did not specify. Bites: FR-1.4. Owner: product
  decision (idea.md amendment). Status: **resolved** by decision 0007 — factory value is
  `discard` (unconfirmed); first successful import requires an explicit `set_retention_default`
  (first-upload prompt pre-selects discard). Per-document override remains FR-1.3.
- **OQ-15** *(was Q15)* Re-import / updated-document behavior. What happens when a user imports a
  revised version of a document already in the vault? Bites: §3.1. Owner: design + UI specs.
  Status: **resolved** by design spec §3.6 — treated as a new document in v1; no revision flow.
- **OQ-16** *(was Q16)* Overlapping/nested detected fields. Which decision wins when fields
  overlap or nest? Bites: §3.2, §3.3. Owner: design spec. Status: **resolved** by design spec
  §3.5 — innermost explicit user decision wins; one deterministic redaction rule at export.
- **OQ-17** *(was Q17)* Temporary plaintext during processing. Is plaintext document content ever
  written to disk transiently (e.g., during detection)? At-rest encryption (§3.4) covers storage,
  not transient working state. Bites: NFR-S1..S3. Owner: architecture spec. Status: **resolved**
  by decision 0004 + architecture spec §5 — no plaintext-to-disk; zeroize; page-aligned mlock
  for keys with fallback; no app crash reporter.
- **OQ-18** *(was Q18)* Key rotation / recovery. Not addressed by the idea doc at the v1 stage.
  Bites: §3.4, §3.8. Owner: architecture spec. Status: **resolved** by decision 0004 +
  architecture spec §3.3 — passphrase change re-wraps the same master key; no recovery in v1;
  irrevocable delete = DEK destruction.

## Resolved (for reference)

- Q8, Q9, Q10, Q11, and the Q4 single-PDF-bundle portion →
  `docs/decisions/0002-resolved-srs-clarifications.md`.
- OQ-1, OQ-2, OQ-7, OQ-15, OQ-16 → resolved by `docs/specs/design.md` (see design spec §9 and
  decision 0003).
- OQ-3, OQ-5, OQ-13, OQ-17, OQ-18 → resolved by `docs/specs/architecture.md` and decision 0004.
- OQ-12 → resolved by decision 0004 + architecture spec §9 + API spec §5.7.
- OQ-4 (design: PDF format + bundle order; API: filename + PDF info dictionary; UI: save
  dialog) → design §3.7, API §7, ui.md §10.4.
- OQ-6 → design §2.6 (predicate) + testing spec §7 (independent oracle).
- OQ-2 enforcement → testing spec §8 / §11.
- OQ-14 → decision 0007 (factory discard; first-import confirmation).