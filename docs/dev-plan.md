# Development Plan — Privacy Gate v1

> Implementation sequence, not a product spec. Behaviour lives in
> [`specs/`](./specs/index.md). If this plan and a spec disagree, the spec wins.
> Product intent: [`idea.md`](./idea.md). Process: TDD + `cargo-mutants`
> ([decision 0006](./decisions/0006-tdd-and-mutation-testing.md),
> [`specs/testing.md`](./specs/testing.md)). Command names:
> [`specs/api.md`](./specs/api.md). Types: [`specs/data-model.md`](./specs/data-model.md).
> UI: [`specs/ui.md`](./specs/ui.md).
>
> Out of this plan: vault backup/restore and reinstall re-attachment ([idea.md](./idea.md)
> later phases). Do not add commands or screens for them in v1.

---

## 1. How to use this plan

Work **one chunk at a time**. A chunk is one pull request (or a stacked pair: tests then
code). Do not start the next chunk until the current one is green and integrated at its
stated seam.

For every chunk:

1. **Red.** Write the failing tests listed under that chunk. Cite the spec clause in the
   test name or a comment (`FR-…`, `AC-…`, `C-ARCH-…`, `api.md` command).
2. **Green.** Minimum production code to pass.
3. **Integrate.** Wire into the in-process command functions (the same functions Tauri IPC
   will call — [testing.md](./specs/testing.md) §3). Run `cargo test` for the core. If the
   chunk is UI, also run the UI unit tests in [ui.md](./specs/ui.md) §16.
4. **Stop.** Do not pull in the next chunk’s behaviour “while you are in the file.”

**Integration seam for v1 core:** in-process API commands, not the webview. Acceptance
criteria AC-1..AC-7 are command tests. The webview is a client of those commands.

**Do not invent requirements.** No OCR, no passphrase recovery, no vault export, no
filesystem read plugin, no extra Tauri commands.

---

## 2. Map

```
W0 skeleton
  → W1 crypto    → W2 keys/session → W3 vault schema → W4 session table
                      ↓
                   W5 audit HMAC      W6 retention config      W7 Linux keystore fallback
                      ↓
                   W8 import text → W9 import PDF reject → W10 catalog → W11 AC-7 gate
                      ↓
                   W12 detector stub → W13 UK patterns → W14 progress event
                      → W15a ONNX hybrid → W15b Ollama backend → W15c backend selection
                      ↓
                   W16 approval session → W17 overlap rule → W18 submit/discard → W19 abort/lock
                      ↓
                   W20 delete doc → W21 drop original → W22 variants
                      ↓
                   W23 PDF re-render → W24 preview/commit export → W25 OQ-6 oracle
                      → W26 ephemeral overrides → W27 Cloud AI mock → W28 audit list
                      ↓
                   W29 Tauri IPC + CSP
                      ↓
                   W30–W36 UI slices (each after its commands exist)
                      ↓
                   W37 AC-1..AC-7 green  → W38 mutation gate  → W39 perf/nightly
```

UI slices may be prepared against fakes, but they **merge** only after the matching command
chunk is green.

---

## 3. Definition of done (every chunk)

- Tests existed in red before the implementation (PR description attests TDD for TCB).
- `cargo test` (and UI tests if the chunk touches `src/`) green locally.
- No new plaintext-to-disk path (architecture §5).
- No new command name that is not in api.md.
- Gated module (testing.md §5.3) introduced in this chunk: `cargo mutants --file` on that
  path is clean, or the chunk is explicitly “stub, mutation in W38.”

---

## 4. Chunks

### W0 — Repo skeleton

- **Delivers:** Tauri 2 + Rust core crate + Svelte 5/Vite webview; CI runs `cargo test`
  (empty/smoke) and a frontend typecheck. Capability file denies fs-read, HTTP, shell
  (architecture §12, api.md §8).
- **Depends on:** nothing (specs only).
- **Specs:** decision 0003, 0008; architecture §12; ui.md §2–§3.
- **Tests first:** compile-level; CI workflow exists; capability JSON reviewed as a fixture
  (deny list).
- **Integrate:** app launches to a blank shell that can call `get_session_state` once W2
  exists. Until then, no product commands.
- **Done when:** `cargo test` and `npm run check` (or equivalent) pass in CI on macOS or
  Linux; production CSP string is in source (may still be unused).
- **Do not:** implement unlock, SQLCipher, or screens.

---

### W1 — Envelope crypto primitives

- **Delivers:** HKDF labels, wrap AEAD, AAD length-prefix, DEK generate/zeroize. No disk
  yet.
- **Depends on:** W0.
- **Specs:** architecture §3.1; data-model wrap fields; testing.md §5.3 (AAD, DEK destroy
  helpers).
- **Tests first:** unit tests for wrap/unwrap; wrong AAD fails; zeroize leaves key unusable;
  length-prefix mutants would break unwrap.
- **Integrate:** internal module used by W2/W3. No API command.
- **Done when:** unit tests green; module listed for later mutation (W38).
- **Do not:** SQLCipher, keystore, passphrase KDF beyond what architecture specifies for
  wrap_key.

---

### W2 — Account, keystore, session

- **Delivers:** `get_session_state`, `create_account`, `unlock`, `lock`, `change_passphrase`,
  `get_account`. States: `first_run` | `locked` | `unlocked`. Degraded path stub returns
  `unlocked` until W5.
- **Depends on:** W1.
- **Specs:** api.md §2, §5.1; architecture §3.2–§3.4, §7; data-model `KeystoreItem`,
  `LocalAccount`.
- **Tests first:** first_run → create → unlocked; lock → locked; wrong passphrase
  `unlock_failed`; `account_exists`; passphrase min length 8; change_passphrase wrong current
  `passphrase_mismatch`; outputs never contain passphrase (C-API-1).
- **Integrate:** in-process command functions. OS keystore mock in tests; real backend on
  one platform job when practical.
- **Done when:** session commands work against mock keystore; lock zeroizes session key
  material (assert via subsequent decrypt/command failure, not log lines).
- **Do not:** integrity report, documents, UI.

---

### W3 — Empty vault (SQLCipher)

- **Delivers:** create/open SQLCipher DB in app-data dir; schema from data-model.md (tables
  may exist empty); open with raw key (not passphrase-KDF on the DB).
- **Depends on:** W2.
- **Specs:** architecture §4; data-model §6–§7; testing.md SQLCipher raw-key check.
- **Tests first:** create vault on first account; reopen after lock/unlock; stolen file
  without wrap key cannot query (prelude to AC-5); schema_version = 1.
- **Integrate:** `create_account` / `unlock` open the DB; `lock` closes it.
- **Done when:** temp-dir component tests pass; no plaintext document columns.
- **Do not:** import, audit HMAC (can insert later), detector.

---

### W4 — Session gating table (commands that exist)

- **Delivers:** api.md §2 matrix for W2–W3 commands; `not_in_session` everywhere else
  (unimplemented commands still return `not_in_session` or `not_found` only if listed — prefer
  not registering them yet).
- **Depends on:** W2.
- **Specs:** api.md §2; testing.md “Session table.”
- **Tests first:** table-driven every cell for registered commands.
- **Integrate:** single gate in the command dispatcher.
- **Done when:** adding a command requires a new row in the table test (will fail until
  filled).
- **Do not:** implement gated-but-unwritten commands.

---

### W5 — Audit chain and integrity

- **Delivers:** append-only audit rows, canonical HMAC encoding v1, keystore `audit_head`,
  crash-window fast-forward (1..=32), true tamper → `degraded_integrity`,
  `get_integrity_report`. `unlock` can now return `degraded_integrity`.
- **Depends on:** W3.
- **Specs:** architecture §6; data-model audit; api.md `IntegrityReport`; testing.md §8
  tamper/truncation/crash-window.
- **Tests first:** happy append; flip payload byte → degraded, no document decrypt (none
  exist yet); truncation vs head; crash window fast-forward still `unlocked`; HMAC break is
  **not** fast-forwarded; degraded session cannot import/approve/share (C-API-6) — those
  commands fail even if not fully implemented.
- **Integrate:** every later mutating command must append audit (enforced as they land).
- **Done when:** integrity tests green; `get_integrity_report` matches unlock outcome.
- **Do not:** UI integrity screen (W35); user vault restore.

---

### W6 — Retention config (AC-7 core)

- **Delivers:** `get_retention_default`, `set_retention_default`. Factory `discard`,
  `confirmed: false`. First `set_retention_default` confirms.
- **Depends on:** W3, W4.
- **Specs:** decision 0007; api.md §5.2; testing.md AC-7 / C-TEST-6.
- **Tests first:** factory values; set confirms; `never_retain` → `retain` global change
  allowed; import still absent so only config tests.
- **Integrate:** config blob in vault (data-model kind 4).
- **Done when:** AC-7 config half green. Import gate is W11.
- **Do not:** first-import modal UI (W32); per-import override (W10).

---

### W7 — Linux keystore fallback

- **Delivers:** Secret Service unavailable → `0600` wrap file next to DB (architecture §3.2).
  AC-5 still holds. Coordinated rollback of DB+file **not** claimed detectable.
- **Depends on:** W2, W3.
- **Specs:** architecture §3.2; testing.md Linux fallback.
- **Tests first:** fallback backend reported; wrong passphrase still fails; stolen dir
  without passphrase cannot decrypt.
- **Integrate:** Key Manager backend switch; CI Linux job.
- **Done when:** Linux CI job green. Skip on macOS/Windows CI except mock.
- **Do not:** change threat model to claim coordinated rollback detection.

---

### W8 — Import plain text

- **Delivers:** extract UTF-8 text to in-memory pages/spans. No catalog yet if W10 is
  separate — prefer W8 as a library + W10 as the command. If split is painful, keep W8
  library-only.
- **Depends on:** W1.
- **Specs:** design Importer; FR-1.1 text; architecture no plaintext-to-disk.
- **Tests first:** `.txt` bytes → pages; empty → `unsupported_document` at command layer
  (W10); path separators in filename rejected at command layer (W10).
- **Integrate:** Importer module.
- **Done when:** unit tests on fixtures in `testdata/` (synthetic, not real PII).
- **Do not:** PDF, detection, retention.

---

### W9 — Import PDF (text-bearing) and reject scans

- **Delivers:** PDF with extractable text → pages + byte offsets. No text →
  `unsupported_document`. In-memory PDF I/O only.
- **Depends on:** W8.
- **Specs:** FR-1.1, FR-1.2; architecture §5 / §11 import side; design Importer.
- **Tests first:** born-digital PDF fixture extracts known canary; image-only PDF rejected;
  watcher: no plaintext sidecar files.
- **Integrate:** Importer format switch.
- **Done when:** component tests green. Over-budget flag is W10.
- **Do not:** re-render export (W23); OCR.

---

### W10 — Catalog and `import_document` (no detector yet)

- **Delivers:** `import_document`, `list_documents`, `get_document`. Detection may be a
  no-op empty field list **only if** W12 is the next PR; otherwise block merge until W12
  is stacked. Preferred: W10 stores document + retention, W12 runs in the same command.
  **Merge rule:** do not ship `import_document` to UI until W12 is in the same command
  path (SRS: detect on import). Implement W10+W12 as consecutive PRs, W10 behind a
  module boundary, W12 filling Detector before calling the command done.
- **Depends on:** W6, W8, W9, W5 (audit `import`).
- **Specs:** api.md §5.3; FR-1.3–1.5; data-model `Document`.
- **Tests first:** basename only; `over_budget` true still completes; two imports of same
  bytes → two `doc_id`s; `get_document` has no span text; newest first.
- **Integrate:** command + vault envelopes for original/meta.
- **Done when:** catalog tests green **and** W11/W12 attached before UI import.
- **Do not:** approval UI; `already_approved` paths.

---

### W11 — Import blocked until retention confirmed

- **Delivers:** `import_document` → `retention_policy_unset` if `confirmed === false`.
  Per-import `retention_override`; `never_retain` + override retain →
  `retention_loosen_forbidden`.
- **Depends on:** W6, W10.
- **Specs:** decision 0007; api.md §5.3; testing.md AC-6, AC-7.
- **Tests first:** AC-7 command scenario; AC-6 paranoid loosen forbidden.
- **Integrate:** Importer reads Config before detect.
- **Done when:** AC-6 and AC-7 green (AC-7 without UI modal).
- **Do not:** UI first-import modal (W32).

---

### W12 — Detector host + stub (unblocks AC-1)

- **Delivers:** in-process Detector; v1 plugin hook present and empty; stub returns
  sidecar fields for fixtures. `import_document` runs detect; audit `detect` with
  `detector_id: "pg-hybrid-v1"` (or stub id documented in tests until W15a/W15b).
- **Depends on:** W10.
- **Specs:** design Detector; architecture §10; testing.md §10 detector stub; FR-2.1–2.4.
- **Tests first:** fixture sidecar fields appear as locatable spans; no network; empty
  hook does not crash.
- **Integrate:** import command calls Detector then stores field list in envelope.
- **Done when:** import of fixture yields known `field_id`s. Real model is W13/W15a/W15b/W15c.
- **Do not:** ONNX weights in this PR if they bloat CI; stub is enough for AC-1.

---

### W13 — Pattern pack `pg-patterns-uk-v1`

- **Delivers:** golden strings for UK-shaped NI, sort code, account number (synthetic
  canaries). Runs every PR.
- **Depends on:** W12.
- **Specs:** architecture detector; testing.md detector contract.
- **Tests first:** goldens hit; PDF/JSON keywords are not false-positive oracles
  (testing.md §7.2).
- **Integrate:** first stage of hybrid (or parallel to stub behind a flag). Import still
  works with stub in unit tests.
- **Done when:** pattern goldens in PR CI.
- **Do not:** require ONNX to pass PR.

---

### W14 — `pg://detect-progress`

- **Delivers:** progress events 0..1 during import/detect so the UI can paint a bar.
- **Depends on:** W12.
- **Specs:** api.md events; ui.md §7.2.
- **Tests first:** in-process subscriber sees monotonic `fraction`; command tests don’t
  require UI.
- **Integrate:** emit from blocking pool as api.md requires.
- **Done when:** event test green. UI bar is W32.
- **Do not:** fake 100% before detect finishes.

---

### W15a — Hybrid ONNX (`pg-hybrid-v1`)

- **Delivers:** in-process ONNX NER + patterns; pin + hash (architecture §10.2). Nightly/
  release golden; PR may skip heavy weights per testing.md.
- **Depends on:** W12, W13.
- **Specs:** architecture §10.2; testing.md ONNX golden job.
- **Tests first:** tiny fixture golden; mismatched pin fails closed.
- **Integrate:** production import can use `pg-hybrid-v1` directly; tests keep stub for
  AC-1..AC-4. Backend selection between this and W15b is W15c — this chunk does not add
  Ollama awareness.
- **Done when:** nightly job defined; pin documented.
- **Do not:** download models at runtime; Cloud AI for detection; anything Ollama-related
  (W15b/W15c).

---

### W15b — Ollama backend (`pg-hybrid-ollama-v1`)

- **Delivers:** decision 0009's optional detector backend: an HTTP client constrained to
  IP-literal loopback only (no DNS, no ambient proxy — architecture §10.1.1), the
  handshake/allowlist/digest verification against a pinned local Gemma tag (§10.1.2), the
  chunking + verify-then-trust offset-mapping algorithm (§10.1.4–§10.1.5), and the
  `pg://detect-progress` `phase: "warming_model" | "detecting"` payload extension.
- **Depends on:** W12, W13, W14.
- **Specs:** architecture §10.1 (all subsections); decision 0009; testing.md §5.3
  (loopback-boundary and offset-verification gated modules), §7.4, §10 (Ollama mock), §11
  (Ollama nightly golden).
- **Tests first:** against the Ollama mock double (testing.md §10) — handshake success/
  failure, allowlist rejection, cloud-tag rejection, digest mismatch, offset-verification
  pass/reject/threshold-fallback (testing.md §7.4's self-test fixtures), IP-literal-only
  connect assertion, zero ambient-proxy-routed requests.
- **Integrate:** exposed as a selectable backend; not yet wired into `import_document`'s
  default path (that's W15c). Real-Ollama nightly golden is separate from PR CI.
- **Done when:** every §7.4 test is green against the mock; nightly golden job defined
  (informational if the runner lacks Ollama); verified context-window/chunk-size figures for
  the pinned tag recorded per architecture §10.1.5.
- **Do not:** trust an unverified offset; fall back on partial/fuzzy matching; resolve
  `localhost` via DNS; honor ambient proxy env vars; wire this as the default backend yet.

---

### W15c — Backend selection + fallback orchestration

- **Delivers:** `Config.detector_preference` (`"auto"` factory default, `"bundled_only"`),
  `get_detector_preference` / `set_detector_preference` commands (api.md §5.2), the
  per-detect selection logic between `pg-hybrid-v1` and `pg-hybrid-ollama-v1` (architecture
  §10.1.3), and the audit `detect` event's `backend` / `model_tag` / `fallback_reason` fields
  (data-model.md, api.md).
- **Depends on:** W15a, W15b.
- **Specs:** architecture §10.1.3; decision 0009; data-model.md §5.5, audit detect payload;
  api.md §5.2, §6.
- **Tests first:** `bundled_only` never probes Ollama; `auto` + Ollama unreachable/
  unallowlisted/digest-mismatched each fall back with the matching `fallback_reason`; `auto`
  + healthy allowlisted Ollama selects `pg-hybrid-ollama-v1`; mid-document Ollama failure
  falls back for that document, not silently partial.
- **Integrate:** this is what makes `import_document` actually choose a backend in
  production — wires W15a and W15b behind the selection logic.
- **Done when:** AC-1..AC-4 still pass with the stub (unaffected); a new fixture-driven test
  exercises the full auto/bundled_only/fallback matrix end to end against the Ollama mock.
- **Do not:** cache the backend choice for the life of a session (probe is per-detect,
  §10.1.3); invent a third preference value.

---

### W16 — Approval session

- **Delivers:** `open_approval`, `get_approval_view`, `set_field_decisions`. One session
  per process. Span text only here (C-API-2). Lifecycle `awaiting_decisions` | `decided`.
- **Depends on:** W12.
- **Specs:** api.md §5.4; design Approval Engine; C-DES-1.
- **Tests first:** `approval_busy`; `already_approved` after submit (W18); span text on
  view, absent on `get_document`; partial decisions leave `awaiting_decisions`; all
  decided → `decided`; `approval_bad_state` on wrong lifecycle.
- **Integrate:** RAM session; lock/abort clears (W19).
- **Done when:** decision commands green; submit is W18.
- **Do not:** share preview; variants.

---

### W17 — Overlap / nested fields (design §3.5)

- **Delivers:** innermost explicit decision wins; partial overlap redact-wins; one
  redaction rule at export. Table-driven + `proptest`.
- **Depends on:** W16.
- **Specs:** design §3.5; testing.md overlap row; testing.md §5.3 gated module.
- **Tests first:** nested keep-inside-redact; partial overlap; property tests.
- **Integrate:** used at `submit_approval` and later at share render.
- **Done when:** unit/proptest green. Mutation on this module from here on (or W38).
- **Do not:** change SRS (no manual draw-redact).

---

### W18 — `submit_approval` (AC-1 core)

- **Delivers:** canonical `ApprovedVersion` with `redacted_content`. Retention discard →
  destroy original here. Audit `approve`. Catalog `has_approved_version`.
- **Depends on:** W16, W17, W3.
- **Specs:** FR-3.1–3.2; api.md `submit_approval`; architecture DEK/original destroy.
- **Tests first:** AC-1 through store; discard original not decryptable; retain original
  still encrypted; second `open_approval` → `already_approved`.
- **Integrate:** Vault write + Importer/Vault destruction hand-off (design §2.1).
- **Done when:** AC-1 green with stub detector.
- **Do not:** export PDF yet.

---

### W19 — `abort_approval` and lock vs retention

- **Delivers:** abort/lock rules: retain → catalog remains; discard → zeroize original and
  drop catalog row (api.md §5.4).
- **Depends on:** W16, W18.
- **Specs:** api.md abort/lock; architecture §5.2.
- **Tests first:** both retention paths; span text gone after abort (no `get_approval_view`).
- **Integrate:** session teardown.
- **Done when:** those tests green.
- **Do not:** UI copy (W33).

---

### W20 — `delete_document` (DEK destroy)

- **Delivers:** irrevocable delete of approved, original, variants. Audit `delete`. Vault
  load fails; wrapped DEK gone (NFR-R2).
- **Depends on:** W18.
- **Specs:** FR-4.6; architecture §4.3; testing.md DEK destroy row; gated module.
- **Tests first:** after delete, open/get fail `not_found`; pre-copied DEK decrypt of old
  ciphertext is **not** the oracle (testing.md §8).
- **Integrate:** catalog + envelopes.
- **Done when:** component + command tests green.
- **Do not:** OS secure-erase of whole disk.

---

### W21 — `delete_retained_original`

- **Delivers:** drop original only; idempotent; audit `discard_original` if one existed.
- **Depends on:** W18.
- **Specs:** api.md; FR-4.6 sibling.
- **Tests first:** retain → delete original → approved remains; second call ok.
- **Integrate:** Vault.
- **Done when:** command tests green.
- **Do not:** change canonical approved bytes.

---

### W22 — Variants

- **Delivers:** `list_variants`, `get_variant`, `save_variant`, `delete_variant`. No edit;
  per-doc unique name; `get_variant` has no span text.
- **Depends on:** W18.
- **Specs:** design §3.4; api.md §5.5; FR-5.5.
- **Tests first:** create/apply-on-share later (W26); `variant_name_conflict`; delete.
- **Integrate:** after approved only.
- **Done when:** testing.md variants row green.
- **Do not:** global variants; edit-in-place.

---

### W23 — PDF re-render (true removal)

- **Delivers:** from-scratch PDF writer from approved content; no incremental update of
  source PDF; no redacted canary in content stream (architecture §11).
- **Depends on:** W18, W17.
- **Specs:** architecture §11; NFR-S4; testing.md export sanitization (gated).
- **Tests first:** canary `R` absent in raw bytes + extracted text; no `/Prev`; keep canary
  present.
- **Integrate:** library used by W24.
- **Done when:** export sanitization tests green on fixtures.
- **Do not:** save dialog; plaintext `.txt` export.

---

### W24 — Share preview + commit (export)

- **Delivers:** `preview_share` / `commit_share` for person-export. Preview token, 10 min /
  lock / replace expiry. `pdf_bytes` identical between preview and commit. Suggested
  filename + PDF info dictionary (api.md §7).
- **Depends on:** W23.
- **Specs:** api.md §5.6, §7; FR-5.1, FR-6.1; C-API-4.
- **Tests first:** `not_approved`; `preview_expired`; byte-identical commit; filename
  algorithm; metadata omits original path and redacted text; cancel-at-UI is W34 (core:
  commit is the only person-share audit success).
- **Integrate:** Share Engine + Audit `share`.
- **Done when:** command tests green. Save dialog is W34. OQ-6 spy is W25.
- **Do not:** write files from core; Cloud AI.

---

### W25 — OQ-6 egress oracle

- **Delivers:** spies + high-entropy canary oracle (testing.md §7). Mandatory for AC-2,
  AC-3, and AC-4 share assertion. Do not trust `no_originals_left_device` alone.
- **Depends on:** W24 (export); W27 before AC-3.
- **Specs:** testing.md §7; design §2.6.
- **Tests first:** oracle self-test (plant canary in otherwise-clean PDF → fail); AC-2
  uses oracle.
- **Integrate:** test harness only.
- **Done when:** AC-2 green with oracle. AC-3 when W27 lands.
- **Do not:** weaken canary rules.

---

### W26 — Ephemeral overrides + variants on share (AC-2)

- **Delivers:** share-time keep/redact overrides; `overrides_in_effect`; does not mutate
  canonical approved. Apply named variant at preview.
- **Depends on:** W24, W22, W17.
- **Specs:** FR-5.4, FR-6.2; api.md ShareRequest.
- **Tests first:** AC-2; vault approved unchanged after share with overrides.
- **Integrate:** preview must show warning flag for UI (W34).
- **Done when:** AC-2 green.
- **Do not:** persist overrides as a second canonical version.

---

### W27 — Cloud AI plugin (mock HTTP)

- **Delivers:** `cloud_ai_set_config` / `get` / `clear` / `test`; `preview_share` +
  `commit_share` AI kind. HTTP only from Rust to allowlisted host. Key never in outputs.
  `cloud_ai_test` sends no documents. Failed HTTP still audits attempt.
- **Depends on:** W18, W5.
- **Specs:** architecture §8–§9; api.md §5.6–§5.7; FR-5.2; AC-3; C-API-4.
- **Tests first:** not configured → `cloud_ai_not_configured` before assemble; mock
  server receives **approved** body identical to preview; redacted canaries absent
  (OQ-6); redirect-to-other-host refused; `get` has `key_last4` only.
- **Integrate:** Plugin Host; no webview HTTP.
- **Done when:** AC-3 green against mock. No real vendor in CI.
- **Do not:** bundled API key; send originals.

---

### W28 — `list_audit_events` (AC-4)

- **Delivers:** filtered audit DTOs; no span text, keys, or API keys. Degraded session:
  verified prefix only.
- **Depends on:** W5 plus import/approve/share events from earlier chunks.
- **Specs:** FR-7; api.md §5.8; AC-4.
- **Tests first:** AC-4 “what did I share?”; C-API-1/2 on DTOs.
- **Integrate:** read path.
- **Done when:** AC-4 green.
- **Do not:** webview HMAC verify.

---

### W29 — Tauri IPC, CSP, events

- **Delivers:** Tauri command shims (thin; not mutation-gated) over tested functions.
  Capabilities per api.md §8 and ui.md §3. Listen `pg://detect-progress`,
  `pg://session-changed`. Dialog **save** only. `plugin-fs` **write** only of in-memory
  bytes to the save-dialog path.
- **Depends on:** commands from W2–W28 that exist; can land incrementally after W2 but
  **must** be complete before UI slices that invoke them. Prefer one PR when the first
  UI slice needs it (after W2 for lock screen; expand allowlist as commands land).
- **Specs:** api.md §8; ui.md §3; architecture C-ARCH-2.
- **Tests first:** capability fixture denies read/HTTP/shell; shims round-trip a
  command already tested in-process.
- **Integrate:** webview can invoke.
- **Done when:** lock/unlock round-trip from a throwaway harness or W30.
- **Do not:** `plugin-fs` read; `plugin-http`; dialog open.

---

### W30 — UI: first run, lock, unlock

- **Delivers:** screens ui.md §5. Copy for lost passphrase. No vault list yet (empty
  state ok if W10 exists).
- **Depends on:** W2, W29.
- **Specs:** ui.md §5, §15, §16; FR-8.
- **Tests first:** Vitest: mismatch; min length 8; no “forgot password” control;
  `unlock_failed` copy.
- **Integrate:** real invoke.
- **Done when:** UI tests + manual launch unlocks empty vault.
- **Do not:** Settings passphrase change (W31) can be same slice if small.

---

### W31 — UI: Settings (account, passphrase, retention, Cloud AI form)

- **Delivers:** ui.md §11. Retention controls; Cloud AI set/get/clear/test **without**
  sending documents on test. Passphrase change.
- **Depends on:** W6, W2, W27 (Cloud AI section waits for W27; ship retention+account
  first if splitting).
- **Specs:** ui.md §11, §15.
- **Tests first:** retention confirm calls `set_retention_default`; API key not stored
  in DOM after set returns.
- **Integrate:** Settings nav.
- **Done when:** UI tests green.
- **Do not:** invent extra settings.

---

### W32 — UI: vault, first-import modal, import

- **Delivers:** ui.md §6–§7. Blocking retention modal **before** picker; discard
  pre-selected; then `set_retention_default` then file `File` bytes → `import_document`.
  Progress bar. Catalog rows via `list_documents` / `get_document` refresh.
- **Depends on:** W11, W12, W14, W30.
- **Specs:** ui.md §6, §7, §16; decision 0007.
- **Tests first:** fake modal: Continue sets policy before import; cancel does not
  import; no `plugin-fs` read.
- **Integrate:** vault screen.
- **Done when:** UI tests + one real txt/PDF import on a dev machine.
- **Do not:** duplicate-file warning (deferred).

---

### W33 — UI: approval

- **Delivers:** two panes; locatable spans; keep/redact not colour-only; first paint
  first page + first 200 rows (ui.md §14). Submit when `lifecycle === "decided"`.
- **Depends on:** W16–W19, W32.
- **Specs:** ui.md §8, §14, §16; FR-2.2, FR-3.1.
- **Tests first:** Approve disabled until decided; first-paint fake clock; keyboard
  operable list + keep/redact.
- **Integrate:** navigate from vault.
- **Done when:** UI tests green; AC-1 still green at command layer.
- **Do not:** re-approve after commit.

---

### W34 — UI: share, preview, save dialog (OQ-4)

- **Delivers:** ui.md §10. Preview then OS save dialog; default name
  `suggested_filename`; PDF only; documents folder; **cancel = no `commit_share`**;
  write fail → retry save, no second commit. Ephemeral warning not a toast. Blob URL
  teardown.
- **Depends on:** W24–W26, W33.
- **Specs:** ui.md §10.4, §15, §16; C-ARCH-2.
- **Tests first:** fake dialog cancel → no commit; confirm → commit then write mock;
  FR-6.2 warning visible.
- **Integrate:** share flow.
- **Done when:** those tests green. Optional Playwright not a PR mutation gate.
- **Do not:** core writing files; open dialog.

---

### W35 — UI: audit + integrity failure

- **Delivers:** audit table (ui.md §12); integrity full-screen (ui.md §13); save
  integrity JSON via same save-dialog rules; no “Open anyway.”
- **Depends on:** W5, W28, W29, W30.
- **Specs:** ui.md §12–§13, §16.
- **Tests first:** degraded session cannot navigate to Vault; save report uses fake
  dialog.
- **Integrate:** `pg://session-changed` to integrity screen.
- **Done when:** UI tests green.
- **Do not:** repair/restore vault.

---

### W36 — UI: variants empty/list + Cloud AI share confirm

- **Delivers:** variants empty state; save/delete; AI confirm copy; preview AI payload
  read-only.
- **Depends on:** W22, W27, W34.
- **Specs:** ui.md §9, §10, §15.
- **Tests first:** empty state; AI confirm visible before commit.
- **Integrate:** document menu.
- **Done when:** UI tests green.
- **Do not:** marketplace.

---

### W37 — Acceptance pack AC-1..AC-7

- **Delivers:** all testing.md §6 scenarios green in-process on CI. Fill any gaps left
  by “partial AC” in earlier chunks.
- **Depends on:** W1–W28 (UI not required).
- **Specs:** testing.md §6–§8.
- **Tests first:** already written; this chunk is the gate that they all run in CI as
  one job.
- **Integrate:** `cargo test` acceptance binary/module.
- **Done when:** AC-1..AC-7 listed in CI logs.
- **Do not:** hit a real Cloud AI host.

---

### W38 — Mutation gate

- **Delivers:** PR job `cargo mutants --file` on testing.md §5.3 paths; S = 1.00 after
  equivalent skips. Nightly full core minus exclusions.
- **Depends on:** gated modules exist (W1, W4, W5, W6, W17, W20, W23, W24/W25).
- **Specs:** testing.md §5; decision 0006.
- **Tests first:** a known mutant (or first survivor) is killed by a new test.
- **Integrate:** CI.
- **Done when:** PR mutation job required.
- **Do not:** Stryker on TypeScript; silent threshold drop.

---

### W39 — Perf + no-plaintext watcher jobs

- **Delivers:** nightly/pre-release budgets (design §7, ui.md §14 command-side);
  temp-dir watcher on import/detect/export (testing.md §8). Unlock ≤ 1 s after
  passphrase on documented runner.
- **Depends on:** W18, W24, W2.
- **Specs:** design §7; testing.md §8, §11.
- **Tests first:** perf job may be assert-with-timeout; watcher fails if fixture
  plaintext appears outside ciphertext.
- **Integrate:** nightly CI, not flaky PR gate.
- **Done when:** jobs documented and running.
- **Do not:** make perf a PR flake gate.

---

## 5. Suggested merge train (vertical slices)

Use this if you want running software early without waiting for W39.

| Slice | Chunks | You can |
|---|---|---|
| A Empty vault | W0–W4, W29–W30 | Create account, unlock, lock |
| B Policy + import | W5–W14, W31–W32 | Confirm retention, import, see catalog |
| C Consent | W16–W19, W33 | Approve one document |
| D Destroy | W20–W21 | Delete doc / original |
| E Share PDF | W22–W26, W34 | Preview and save redacted PDF |
| F AI + audit | W27–W28, W35–W36 | Mock AI share; inspect audit |
| G Real detector | W15a, W15b, W15c | Hybrid model (ONNX baseline + optional Ollama backend) in production import |
| H Harden | W37–W39 | AC pack, mutants, perf |

Each slice ends with: core tests green, then UI tests for that slice, then a short
manual pass of that slice only.

---

## 6. What this plan does not schedule

- Vault backup/restore, reinstall re-attachment (idea.md later).
- OCR, agentic AI, third-party plugins, team mode.
- Visual design system, i18n, WCAG certificate.
- Playwright as a mutation gate.
- Invented Tauri commands.

---

## 7. Related documents

- [Spec — SRS](./specs/srs.md)
- [Spec — design](./specs/design.md)
- [Spec — architecture](./specs/architecture.md)
- [Spec — API](./specs/api.md)
- [Spec — testing](./specs/testing.md)
- [Spec — data model](./specs/data-model.md)
- [Spec — UI](./specs/ui.md)
- [Decision 0006](./decisions/0006-tdd-and-mutation-testing.md)
- [Decision 0003](./decisions/0003-v1-tech-stack.md)
- [Decision 0008](./decisions/0008-frontend-svelte.md)
