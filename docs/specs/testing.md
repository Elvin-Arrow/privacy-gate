# Testing Specification — Privacy Gate v1

> Scope: how v1 is developed and verified. This spec owns the TDD process, test layers, tools,
> mutation-testing gate, acceptance-test mechanics for AC-1..AC-6, independent verification of
> OQ-6, and the architecture-owned security checks (DEK erasure, audit tamper/truncation,
> keystore fallback, crash-window, export sanitization, no plaintext-to-disk).
>
> It does **not** specify UI layout, TS framework, or webview E2E chrome (→ [ui.md](./ui.md) §16). It does
> not replace SRS requirements with tests: a passing suite that contradicts `idea.md` is still
> wrong. Retention factory default and first-import confirmation: [decision 0007](../decisions/0007-retention-default-discard.md).
>
> Parent specs: [`srs.md`](./srs.md), [`design.md`](./design.md),
> [`architecture.md`](./architecture.md), [`api.md`](./api.md),
> [`data-model.md`](./data-model.md). Process:
> [decision 0006](../decisions/0006-tdd-and-mutation-testing.md). Review roster:
> [decision 0005](../decisions/0005-review-claude-gemini.md).
>
> Open questions: [`../notes/open-questions.md`](../notes/open-questions.md).

---

## 1. Purpose

Make the product's privacy and integrity claims **falsifiable**. Tests are written first
(TDD). Mutation testing then asks whether those tests would fail if the implementation were
subtly wrong. Acceptance tests drive the **Rust core through the API spec's command
functions** (in-process), not the webview.

---

## 2. Development process (TDD)

Required for every behaviour change in the Rust TCB and for any TypeScript that is more than
view wiring ([decision 0006](../decisions/0006-tdd-and-mutation-testing.md)).

1. **Red.** Write one failing test. The test name or a `#[doc]` / comment cites the clause it
   protects (`FR-…`, `AC-…`, `C-ARCH-…`, `api.md` command, design §3.5 overlap rule, …).
2. **Green.** Write the minimum production code that makes that test pass.
3. **Refactor.** Keep tests green. No new behaviour without a new red test.

Rules:

- No TCB production code lands without a test that failed before the code existed (new
  module) or that fails if the change is reverted (bugfix / behaviour change).
- Tests assert **observable outcomes** (command `Result`, bytes on an egress spy, vault
  decrypt failure after delete). They do not assert log lines or private field names.
- A test that needs the real GLiNER model, a real Cloud AI host, or a real OS dialog is in
  the wrong layer (§3). Use the doubles in §10.

CI cannot prove the red step happened first. Enforcement is: (1) PR description attests TDD
for TCB changes; reviewers reject "tests after" for gated modules; (2) `cargo-mutants` (§5)
is the automated audit that the tests actually constrain behaviour.

---

## 3. Test layers

| Layer | What | Runs |
|---|---|---|
| **Unit** | Pure functions and small modules: overlap rule, canonical encoding v1, AAD v1, filename algorithm, session gating table, retention constraint, DEK destroy, HKDF labels. | Every PR |
| **Component** | One component + fakes: Vault+SQLCipher (temp dir), Key Manager+mock keystore, Audit Trail replay, Share Engine+PDF writer, Plugin Host+HTTP mock. | Every PR |
| **Command / acceptance** | API spec commands invoked in-process (same functions the Tauri IPC will call). AC-1..AC-6 and OQ-6 live here. | Every PR |
| **Security / config** | Linux keystore fallback as a distinct config; crash-window; degraded session; stolen-file (AC-5). | Every PR where the platform can host the backend; Linux fallback job required on Linux CI |
| **Detector contract** | `pg-patterns-uk-v1` on golden strings; optional `pg-hybrid-v1` ONNX golden on a tiny fixture. | Patterns: every PR. ONNX: nightly / release |
| **Performance** | Design.md §7 budgets on a documented runner class. | Nightly / pre-release; not a PR flake gate |
| **Mutation** | `cargo-mutants` on gated TCB modules (§5). | PR-blocking on gated modules; full crate nightly |
| **UI E2E** | Webview, copy, save-dialog chrome. | **UI spec** — not this document |

There is no requirement to drive the OS webview for v1 acceptance. If a behaviour is only
visible in the UI, the UI spec owns that test; the core contract still has a command-level
test here.

---

## 4. Tools

| Role | Tool |
|---|---|
| Rust unit / component / acceptance | `cargo test` (`#[test]`, `tokio::test` where async) |
| Property tests (encouraged on overlap + encoding) | `proptest` |
| HTTP / TLS double for Cloud AI | `wiremock` or `hyper` local server over `rustls`; host must be the configured allowlist |
| PDF oracle | Parse export bytes with a **read-only** PDF library used only in tests; plus raw-byte search (§7) |
| Keystore in unit tests | In-memory / temp mock implementing the same `KeystoreItem` layout |
| Linux fallback | Real `0600` file backend on Linux CI |
| Mutation | `cargo-mutants` |
| Coverage (informational, not a gate) | `cargo llvm-cov` |

TypeScript unit tests and StrykerJS are **not** a v1 gate. Non-trivial logic in the webview
is a C-DES-1 defect, not a testing-tool problem. The UI spec may add view tests.

---

## 5. Mutation testing

### 5.1 Why
Statement coverage can execute `destroy_dek` and still pass if a mutant comments the call
out. Mutation testing is the audit that TDD tests actually constrain the TCB.

### 5.2 Tool and operators
Use **`cargo-mutants`** on the Rust workspace. Default operators (replace boolean, arithmetic,
relational, delete statements, replace function body with default/Err) are enough. Do not
add custom operators in v1.

### 5.3 Gated modules (PR-blocking)
These paths (names may match crate modules; the implementation maps 1:1) **shall** run in
the PR mutation job.

Mutation score **S** = killed / (killed + survived), after excluding annotated equivalent
mutants. Timeouts count as killed.

**Gated TCB modules — no unexplained survivors.** Every mutant is killed, times out, or is
skipped with an equivalent-mutant annotation that includes a one-line reason. After that
exclusion, **S = 1.00**.

Gated modules:

- Overlap / redaction decision (design.md §3.5 byte-offset rule)
- Export sanitization (omit redacted spans from the PDF content stream; no source mutation)
- Share egress helper that computes `no_originals_left_device` **and** the code that selects
  bytes for PDF / HTTP (OQ-6)
- Audit canonical encoding v1 + HMAC verify + crash-window fast-forward vs integrity failure
- Retention paranoid default (`retention_loosen_forbidden`)
- Session gating table (api.md §2), including `degraded_integrity`
- Vault delete = overwrite-and-drop wrapped DEK (architecture §4.3)
- Envelope AAD length-prefixing; SQLCipher opened with raw key form (not passphrase KDF)
- Ollama detector loopback boundary: IP-literal-only connect, no DNS resolution of
  `localhost`, no ambient proxy, handshake/allowlist/digest verification before any document
  text is sent (architecture §10.1.1, §10.1.2, decision 0009)
- Ollama detector offset-verification algorithm: chunk-relative entity offsets verified
  byte-exact against the source chunk before being trusted; rejection-threshold fallback
  (architecture §10.1.4, decision 0009)

**Other Rust core modules** (not in §5.4): **S ≥ 0.70** after excluding annotated equivalent
mutants. Unexplained survivors below that fail CI.

### 5.4 Excluded from mutation
- Generated bindings, `include_bytes!` model weights, vendored ONNX Runtime
- Test-only crates and fixtures
- Thin Tauri command shims that only deserialize and call a tested function (mutate the
  function, not the shim)
- Equivalent mutants, listed in `cargo-mutants` skip annotations with a one-line reason

A skip without a reason is a spec violation.

### 5.5 Surviving mutants
On **gated** modules, an unexplained survivor fails CI (effective S = 1.00 after equivalent
exclusions). On **other** core modules, unexplained survivors that drop S below 0.70 fail CI.
The fix is a new TDD test that kills the mutant, or an equivalent-mutant annotation. Lowering
a threshold or removing a gated module requires a new decision.

### 5.6 Runtime
PR job: **only the gated module files**, invoked as
`cargo mutants --file <gated_paths>` (explicit path list in CI config, matching §5.3 — not
"files changed on this branch", which would skip untouched TCB). Timeout per mutant high
enough that a hung delete/HMAC test is a kill, not a flake (start at 30 s; raise in CI
config, not by skipping). Nightly: whole core crate minus §5.4. Mutation does not replace
`cargo test`.

The PR file list, `--examine-re` scopes for `session.rs` / `vault.rs`, and the colocated
`--test` filters live in [`scripts/mutation-gate.sh`](../../scripts/mutation-gate.sh). CI
runs that script once per shard and requires every shard.

---

## 6. Acceptance tests (AC-1..AC-6)

Each AC is a command-level scenario. Fixtures are synthetic (Aisha-shaped UK paperwork:
sort code, account number, NI number, address) with **known** plaintext spans. No real
personal data in the repo.

Commands and wire types are [`api.md`](./api.md). Session starts `unlocked` unless noted.

### 6.1 AC-1 — Import, detect, approve, store
- Policy already confirmed (AC-7). Factory default remains discard unless the scenario sets
  another policy.
- Import a born-digital PDF with extractable text (`import_document`).
- Detection yields labeled locatable spans (`byte_offset` + `byte_length`). A **stub
  detector** may supply fields for this scenario; `pg-patterns-uk-v1` still has its own
  contract tests (§8).
- `open_approval` / `set_field_decisions` / `submit_approval` produce one canonical
  `ApprovedVersion`.
- Catalog shows `has_approved_version`. A second `open_approval` returns `already_approved`.
- Unlock after lock still returns the approved document metadata; original bytes never appear
  on any command **output**.

### 6.2 AC-2 — Export with ephemeral override + preview
- After AC-1, `preview_share` (export kind) with an ephemeral override; preview PDF bytes
  are the artifact.
- `commit_share` returns **byte-identical** `pdf_bytes` (api.md FR-6.1).
- Audit has a `share` event. PDF oracle (§7) finds **no** redacted field plaintext.
- Multi-doc: two approved docs, user selection order preserved in the bundle (design.md §3.7).
- Suggested filename and PDF info dictionary match api.md §7 (no Author/Subject/Keywords;
  no redacted text in metadata).

### 6.3 AC-3 — Cloud AI, approved content only
- Configure Cloud AI with a mock allowlisted HTTPS origin (`cloud_ai_set_config`).
  `get` returns `key_last4` only; never the key.
- `cloud_ai_test` hits the mock and **does not** send vault documents (C-API-4).
- `preview_share` (AI kind) requires `ai_instruction` per [api.md §4](./api.md) (1..=4000
  chars). `ai_payload_preview` is the approved body.
- `commit_share` POSTs **that same body** (api.md identity guarantee). The mock records the
  request. Oracle: body contains no redacted field plaintext; originals not present.
- Response is read-only text. Audit `share` with `has_ai_instruction: true` and no
  instruction text.
- Missing key → `cloud_ai_not_configured` and **no** HTTP with document content.

### 6.4 AC-4 — Audit trail answers "what did I share?"
- After import/detect/approve/export/AI, `list_audit_events` (filter by `doc_id`) shows
  those types.
- Payloads have classifications and keep/redact decisions, not redacted field text, not
  originals, not keys.
- `no_originals_left_device` is present on share events. Independent check is §7, not
  "the flag is true."
- Degraded session: `list_audit_events` returns only the verified prefix. Document /
  approval / share commands are forbidden per the api.md §2 table (tested in §8; this
  scenario does not re-state error codes).

### 6.5 AC-5 — Stolen data file, vault locked
- Create account, import+approve, `lock`.
- Copy `vault.db` (and, in a second case, the Linux fallback keystore file). Without the
  passphrase: SQLCipher open fails or yields no plaintext catalog/document fields.
- Stolen DB + OS keystore item (or fallback file) **without passphrase** still yields no
  plaintext (architecture §3.2: wrap still holds).
- This is NFR-S3 / FR-4.4. Do not implement a "recovery" path to make the test pass.

### 6.6 AC-6 — Paranoid retention default
- After the policy is confirmed, set global default `never_retain`. Per-import retain →
  `retention_loosen_forbidden`; no catalog original. Per-import discard is allowed (tighten).
- When confirmed default is `retain`, per-import discard is also allowed.

### 6.7 AC-7 — Factory discard and first-import confirmation (decision 0007)
- After `create_account`, `get_retention_default` is `{ policy: "discard", confirmed: false }`.
- `import_document` (any override, including null) → `retention_policy_unset`; no catalog row.
- `set_retention_default` (discard, retain, or never_retain) → `confirmed: true`.
- Subsequent `import_document` proceeds. Confirming discard then overriding that first import
  to `retain` is allowed (FR-1.3 vs FR-1.4 stay distinct).
- Pre-select chrome is UI spec; this scenario only asserts the API gate and factory value.

---

## 7. OQ-6 — Independent verification of `no_originals_left_device`

Design.md §2.6: the Share Engine **sets** the flag true iff retention was `discard` **or**
the share transmits only the approved version (never the retained original). This spec owns
how that claim is checked **without trusting the flag**.

### 7.1 Egress spy
Every share under test installs spies:

- **Export:** the `pdf_bytes` returned by `preview_share` / `commit_share` (must be
  identical).
- **Cloud AI:** the HTTPS request body the Plugin Host actually writes (mock server). Not
  the plugin's in-memory string before wrap: the bytes on the wire after TLS is terminated
  at the mock.

No other network or filesystem write of document content is permitted (C-ARCH-2). Tests
fail if the spy sees a second destination.

### 7.2 Oracle (must all hold)

Let `R` be the set of plaintext strings of fields decided **Redact** for this share
(canonical decisions ± ephemeral overrides ± variant). Let `K` be Keep-visible strings.
Let `O` be the original document plaintext (if retention was retain; else `O` is empty
because the original was discarded).

Fixtures SHALL use **high-entropy canaries** as redacted/keep spans: unique strings that do
not collide with PDF tokens (`Type`, `Font`, `true`, `null`, `xref`, `form`, `stream`,
`obj`, …) or JSON keys. Prefer Aisha-shaped but distinct values (e.g. NI `QQ123456C`, sort
code `20-40-60`, plus a planted token such as `PG-CANARY-REDACT-7F3A`). Redacted canaries
used as the oracle SHALL be ≥ 8 codepoints. Tokens with `|s| < 4` are never the sole
oracle and SHOULD not appear as the only redacted span in a fixture.

1. **No redacted plaintext in egress.** For each canary `s` in `R`:
   `s` does not occur in:
   - raw egress bytes (UTF-8 and UTF-16LE/BE scans);
   - text extracted from the PDF content stream and strings;
   - PDF info dictionary / XMP if present;
   - Cloud AI JSON/text body.
   Placeholders may exist; they must not equal `s`.
2. **Keep-visible still present** (export/AI of a document that had keep decisions): each
   canary `s` in `K` appears in the extracted approved text or PDF text, unless an
   override/variant redacted it for this share.
3. **Originals do not leave.** If `O` is non-empty, no unique substring of `O` that is
   absent from the approved version (e.g. a discarded-original-only header, or a redacted
   span) appears in egress. `raw_bytes` are never an HTTP or export payload.
4. **Flag consistency.** After 1–3 pass, assert the audit flag equals the design predicate
   (discard **or** approved-only transmission). If 1–3 pass and the flag is false, that is
   a product bug (over-pessimistic flag). If the flag is true and 1–3 fail, that is a
   **critical** privacy bug.

**Oracle self-test (required):** a negative fixture takes otherwise-clean export bytes and
injects a known redacted canary into a raw byte region **and** into a FlateDecode-style
compressed stream if the test PDF writer can emit one. The oracle MUST fail (report a
leak). If it passes, the oracle is broken — do not ship. This guards against a PDF library
that silently skips compressed objects.

### 7.3 Retention matrix

Oracle items 1 and 2 apply to **every** share, including `discard`. Item 3 applies when an
original still exists (`retain`). The flag is asserted only **after** those checks pass.

| Retention | After successful share | Extra check |
|---|---|---|
| `discard` | Original DEK gone; decrypt original via Vault fails; no original blob | Flag true only after items 1–2 pass (no original exists, so item 3 is N/A) |
| `retain` | Original still decrypts **inside** the vault after share | Flag true iff items 1–3 show approved-only egress |

### 7.4 Detector loopback boundary (decision 0009 — separate from OQ-6)

OQ-6 (§7.1–§7.3) governs what a **share** transmits. This section governs what the
**detector** may reach at all when `pg-hybrid-ollama-v1` is eligible — a different trust
boundary (architecture §10.1.1), tested independently so a regression here isn't masked by
the share oracle passing.

- **Address assertion.** Instrument the HTTP client (or use a loopback-only test proxy) to
  assert every outbound connection attempted for detection resolves to a literal
  `127.0.0.1`/`::1` socket address — never a DNS lookup of `localhost`, never any other host.
  A fixture that redirects/points the detector at a non-loopback address MUST be refused, not
  silently followed.
- **No ambient proxy.** With `HTTP_PROXY`/`ALL_PROXY` set in the test environment to a mock
  proxy that would record any request routed through it, assert the mock proxy sees **zero**
  requests from the detector's Ollama client.
- **Handshake/allowlist/digest gate.** Against the Ollama mock double (§10 below): a
  response that doesn't match Ollama's documented `/api/tags`/`/api/show` shape, an
  unallowlisted tag, a `-cloud`-suffixed tag, and a digest mismatch must each independently
  produce `pg-hybrid-v1` fallback with the matching `fallback_reason` — and must NOT send
  document content first in any of those cases (assert the mock records zero document-bearing
  requests before the gate is satisfied).
- **Offset-verification self-test (required, mirrors §7.2's oracle self-test):** a fixture
  where the mock Ollama returns a `start`/`length` that does NOT match `text` at that
  position in the chunk it was given. The detector MUST reject that entity and it MUST NOT
  appear in the resulting field list. A second fixture pushes the rejection rate over the
  implementation threshold and asserts the whole document falls back to `pg-hybrid-v1` with
  `fallback_reason: "offset_verification_failed"`.

Delete of a retained original after share is a separate `delete` / `discard_original`
flow (NFR-R2); the share oracle does not require deletion.

---

## 8. Architecture and design checks (not only ACs)

Command-level or component tests, TDD, mutation-gated where listed in §5.3.

| Claim | How |
|---|---|
| NFR-R2 / §4.3 DEK destroy | After `delete_document` / delete original / delete variant: Vault load of that artifact fails; wrapped DEK row is absent or zeroized; catalog has no usable key material. Do **not** assert that ciphertext decrypted with a DEK copied *before* delete fails — that would still succeed and is not the NFR-R2 guarantee. Audit `delete` appended. |
| NFR-R1 tamper | Flip a byte in an audit payload in the DB; unlock → `degraded_integrity`, no document decrypt |
| NFR-R1 truncation | Truncate tail below persisted `audit_head`; same degraded outcome (not Linux fallback coordinated rollback — that is the documented degraded threat model) |
| Crash window | Append 1..32 valid HMAC'd rows without persisting head; unlock fast-forwards to `"unlocked"` |
| Integrity vs crash | HMAC break is **not** fast-forwarded |
| Linux fallback | Key Manager reports fallback backend; AC-5 still holds; coordinated rollback of DB+file is **not** asserted as detectable |
| SQLCipher raw key | Open uses `x'<64 hex>'` / `sqlite3_key_v2` path; a test that passphrase-KDF would exceed unlock budget is not required if the open API is asserted |
| Unlock budget | Perf job: ≤ 1 s after passphrase (design.md §7) on the documented runner |
| No plaintext-to-disk | Component test: import/detect/export with a temp-dir watcher; no file under the sandbox contains fixture plaintext except `vault.db` ciphertext (must not match plaintext) and the Linux fallback wrap blob (must not match passphrase or document text) |
| Export true-removal | PDF has no incremental update / previous content stream containing `R`; architecture §11 |
| Overlap §3.5 | Table-driven + `proptest`: nesting keep-inside-redact; partial overlap redact-wins; one rule at export |
| Variants | create / apply / delete; no edit; per-doc uniqueness `variant_name_conflict`; `get_variant` has no span text |
| Re-import | Two imports of the same bytes → two `doc_id`s |
| Session table | Every api.md §2 cell: allowed vs `not_in_session` |
| Preview expiry | 10 min / lock / replaced token → `preview_expired` |
| C-API-1 | Passphrase / `api_key` never in outputs, events, or audit DTOs; Cloud AI `get` is `key_last4` only |
| C-API-2 | Span text only on `open_approval` / `get_approval_view`; not on catalog, variants, share preview, audit |
| C-API-3 | Originals inbound-only on `import_document`; never on command outputs |
| C-API-4 | `preview_share` and `cloud_ai_test` send no vault documents; only `commit_share` (AI) POSTs |
| C-API-5 | No command returns keystore material, DEKs, HMAC bytes, or SQLCipher keys |
| C-API-6 | Degraded session: no import/approve/share/document-content (api.md §2 table) |
| Pattern pack | UK sort code, account, NI, NHS, email, phone, IBAN, Luhn card on golden strings |
| Model pin | Mismatched ONNX SHA-256 → hard fail of NER stage (nightly) |
| No recovery | No command recovers a lost passphrase (api.md); wrong passphrase is `unlock_failed` |

Performance budgets (design.md §7) other than unlock: import, detect, approval payload ≤ 1 s
for ≤ 200 fields, export, audit query — measured in the perf job, same runner class
(mainstream laptop: 8 GB RAM, SSD). Over-budget **import still completes** with
`over_budget: true` (api.md); the perf job fails the *budget*, the functional test only
checks the flag.

---

## 9. Constraints

- **C-TEST-1** TDD for TCB changes (decision 0006). CI cannot prove the red step; PRs attest
  it and `cargo-mutants` audits test strength.
- **C-TEST-2** Mutation gate on §5.3 modules; no silent threshold drop.
- **C-TEST-3** Acceptance tests do not call the real Cloud AI host or write real PII fixtures.
- **C-TEST-4** Tests never log passphrases, API keys, or redacted field text. Failure
  messages may include field **ids** and labels.
- **C-TEST-5** Tests do not grant the webview fs/http/shell. Command tests are in-process.
- **C-TEST-6** Factory retention is `discard` and unconfirmed; imports fail with
  `retention_policy_unset` until `set_retention_default` (decision 0007 / AC-7). Paranoid
  tests call `set_retention_default` first.
- **C-TEST-7** Independent OQ-6 oracle (§7) is mandatory for AC-2, AC-3, and AC-4's share
  assertion. Trusting the boolean alone is a spec violation.
- **C-TEST-8** UI E2E, save-dialog chrome, integrity-failure copy, and first-paint budget
  belong to [ui.md](./ui.md) §16.

---

## 10. Fixtures and doubles

- **Documents:** small PDF and UTF-8 text compiled in `testdata/`; spans documented in a
  sidecar JSON (offsets + canary plaintext) used only by tests. Canaries follow §7.2
  (high-entropy, ≥ 8 codepoints for redacted oracles, not PDF/JSON keywords).
- **Detector stub:** implements the same host-facing trait as `pg-hybrid-v1`; returns the
  sidecar fields. Used by AC-1..AC-4 so model drift cannot hide a vault bug.
- **Keystore mock:** in-memory; Linux CI also runs fallback-file config.
- **Clock:** injectable for preview 10-minute expiry.
- **Network:** allowlisted mock only. Redirect-to-other-host tests assert refuse (architecture
  §9.2).
- **Ollama mock (decision 0009):** an in-process HTTP mock (same style as the Cloud AI mock)
  simulating `/api/tags`, `/api/show`, `/api/generate` — success, timeout, malformed JSON,
  digest mismatch, cloud-tag rejection, and offset-verification failure responses. CI never
  requires a real Ollama install or real model weights; used by §7.4 and detector unit tests.
- **Crash reporter:** none in the app; tests must not enable one.

---

## 11. CI

| Job | When | Must pass |
|---|---|---|
| `cargo test` unit + component + acceptance | Every PR | Yes |
| Linux fallback config | Linux PR | Yes |
| `cargo-mutants` gated modules | Every PR | Yes (§5.3) |
| `cargo-mutants` full core | Nightly | Yes (minus §5.4) |
| Pattern-pack goldens | Every PR | Yes |
| ONNX golden + model pin | Nightly / release | Yes |
| Ollama golden (real Ollama + pinned tag, if runner has it; else informational) | Nightly / release | Yes if runner has Ollama; otherwise informational (decision 0009) |
| Perf budgets | Nightly / pre-release | Yes for release |
| `cargo llvm-cov` | Nightly | Informational |

v1 CI covers macOS, Windows, and Linux (decision 0003). Keystore tests use the mock on all
OSes plus the real backend when the runner has Keychain / Credential Manager / Secret
Service. Fallback-file tests are Linux-only.

The ONNX golden + model pin job is [`.github/workflows/nightly.yml`](../../.github/workflows/nightly.yml)
(`cargo test -p pg-core --test hybrid_w15a`). When `models/ner-pii.onnx` is absent the
shipped-artifact assertion skips and `NER_PII_ONNX_SHA256` must remain `None`; a present
file must match that constant. Weights are never fetched at runtime (architecture §4.2).

---

## 12. Traceability

| Source | Testing coverage |
|---|---|
| AC-1..AC-6 | §6 |
| OQ-6 remainder | §7 |
| OQ-2 budgets | §8 perf job; design.md §7 numbers unchanged |
| NFR-S3 / FR-4.4 / AC-5 | §6.5 |
| NFR-S4 / C-DES-4 / arch §11 | §7.2, §8 export |
| NFR-R1 / OQ-3 | §8 tamper, truncation, crash-window, degraded |
| NFR-R2 / arch §4.3 | §8 DEK destroy |
| FR-1.4 / AC-6 / dec 0002 Q9 | §6.6 |
| FR-1.4 / AC-7 / dec 0007 | §6.7 |
| FR-3.2 / already_approved | §6.1 |
| FR-5.3 / NFR-P2 / C-API-4 | §6.3, §7 |
| FR-6.1 preview identity | §6.2, §6.3 |
| FR-7.3 / FR-7.4 | §6.4, degraded session |
| design.md §3.5 overlap | §8 |
| api.md §2 session | §8 |
| C-ARCH-2 save-dialog exception | [ui.md](./ui.md) §10.4 / §16 (C-TEST-8) |
| FR-9.5 / NFR-E1 | Architecture review, not an acceptance test (SRS) |

---

## 13. Open questions owned here

- **OQ-6 (remainder)** Independent verification of "no originals left the device" → §7.
- **OQ-2 (enforcement)** Design budgets are the numbers; this spec says where they run
  (perf job) so PR CI stays non-flaky.
- **OQ-14** Factory discard + first-import confirmation → §6.7 (decision 0007).

## 14. Deferred

- Third-party plugin / WASM tests → later phase (FR-9.5 is architectural).

---

## 15. Related Decisions

- [0002](../decisions/0002-resolved-srs-clarifications.md) — true-removal; one canonical
  version; paranoid default; PDF bundle.
- [0003](../decisions/0003-v1-tech-stack.md) — Tauri 2 + Rust + TS; three OSes.
- [0004](../decisions/0004-v1-architecture.md) — crypto, audit, detector, export; testing
  owns DEK, chain, fallback, OQ-6.
- [0005](../decisions/0005-review-claude-gemini.md) — Claude + Gemini review.
- [0006](../decisions/0006-tdd-and-mutation-testing.md) — TDD + `cargo-mutants`.
- [0007](../decisions/0007-retention-default-discard.md) — factory discard; first-import confirm.
- [0008](../decisions/0008-frontend-svelte.md) — Svelte 5; UI tests in ui.md §16.

## 16. Related Work

- [0005-testing-spec](../dev-log/0005-testing-spec.md)
- [0008-ui-spec](../dev-log/0008-ui-spec.md)
- [Spec — SRS](./srs.md)
- [Spec — design](./design.md)
- [Spec — architecture](./architecture.md)
- [Spec — API](./api.md)
- [Spec — data model](./data-model.md)
- [Spec — UI](./ui.md)
