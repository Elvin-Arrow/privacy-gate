# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Privacy Gate is a local-first, single-user desktop app (Tauri 2 + Rust core + Svelte 5) that lets a
user import a document, review/approve which detected fields (PII) get redacted, and share only the
redacted, re-rendered PDF — with an audit trail and no plaintext ever written to disk outside an
encrypted vault. Full product intent: `docs/idea.md`.

## Development environment — Docker-only

**Do not install Rust, Node, or the Tauri CLI on the host.** All builds, tests, and scaffolding run
inside the dev container defined by `Dockerfile.dev` / `docker-compose.yml`. This is a local-machine
constraint, not a CI constraint — CI (`.github/workflows/ci.yml`) installs its own native toolchain
on GitHub-hosted runners and does not use Docker.

```bash
docker compose build dev      # first time / after Dockerfile.dev changes
make shell                    # interactive shell in the dev container
```

Inside the container (or via `docker compose run --rm dev <cmd>` from the host):

```bash
npm install                        # first time only
cargo test                         # all Rust tests (pg-core + privacy-gate crates)
cargo test -p pg-core <test_name>  # a single test in the core crate
npm run check                      # Svelte/TypeScript typecheck
npm run dev                        # Vite dev server (frontend only)
npm run tauri:dev                  # full Tauri app, dev mode
npm run tauri:build                # full Tauri app, release build
```

Makefile wraps the common ones (`make test`, `make check`, `make build`, `make clean`, `make help`),
each running inside the container automatically.

The container mounts the repo at `/workspace` and caches the Cargo registry, Cargo target dir, and
npm packages in named volumes — first run is slow (image build + full fetch), later runs are fast.

## Architecture

### Process and trust model

One OS process (the Tauri binary). The Rust core (`core/`, crate `pg-core`) is a library linked into
the Tauri binary (`src-tauri/`, crate `privacy-gate`) and invoked only through Tauri IPC commands —
no sidecar or daemon that this app manages. The Svelte/TS frontend (`src/`) runs in the OS webview and
is treated as a **separate, untrusted-for-secrets** trust domain even where the OS keeps it in the
same process.

```
Webview (untrusted for secrets)             src/  — Svelte 5 + Vite
   │  Tauri IPC only — no fs/http/shell from the webview (capability ACL denies them)
   ▼
Rust core — the trusted computing base       core/ — crate pg-core
  Key Manager · Vault · Importer · Detector · Approval · Share Engine · Audit Trail
  Plugin Host · Config
   │                              │
   ▼                              ▼
OS keystore (wrapped        SQLCipher DB in app-data dir
master key + audit head)    (envelope-encrypted artifacts, never plaintext)
```

Only two things are allowed to cross the webview↔core boundary during their respective flows:
unapproved document structure + detected spans during review/approve, and the redacted preview
artifact during share preview. Master key, passphrase, DEKs, API keys, redacted field text, and
retained originals never cross it. See `docs/specs/architecture.md` §2.3 for the full boundary table.

### Crypto / key hierarchy (`core/src/keys.rs`, `core/src/crypto/`)

```
passphrase (never stored)
  └─ Argon2id ──► wrap_key (ephemeral)
        └─ AEAD wrap ──► vault_master_key (random, 256-bit)
              ├─ HKDF-SHA-256 "pg-db-v1"        → sqlcipher_key
              ├─ HKDF-SHA-256 "pg-audit-mac-v1" → audit_mac_key
              └─ AEAD wrap of per-artifact DEKs → document/metadata/plugin-secret ciphertext
```

AEAD is XChaCha20-Poly1305 with a length-prefixed AAD (`core/src/crypto/aad.rs`) to prevent
concatenation collisions. Deletion is cryptographic erasure: dropping a wrapped DEK makes its
ciphertext permanently unrecoverable even if disk pages persist — there is no secure-wipe of the
underlying blocks. No hand-rolled crypto; only the vetted crates already pinned in `core/Cargo.toml`.

### Crate/module layout

- `core/` (`pg-core`) — all product logic and the TCB. Modules per `core/src/lib.rs`: `crypto`
  (envelope primitives), `keys` (Key Manager / master key derivation), `keystore` (`KeystoreItem` +
  backends: OS keystore, Linux `0600` fallback, in-memory mock), `account`, `session` (in-process
  session/account commands), `api` (shared `ApiError`/`ErrorCode` model — every user-facing error
  goes through this so a passphrase or key can never leak into a message string).
- `src-tauri/` (`privacy-gate`) — thin Tauri 2 binary; depends on `pg-core` via the Cargo workspace.
  IPC command shims here are intentionally thin and call already-tested `pg-core` functions.
- `src/` — Svelte 5 + Vite frontend, webview-side only.
- `design/mockups/` — `.dc.html` UI mockups (Claude Design canvas format) per screen.

## Development process

This project is built as a specced, TDD, chunk-by-chunk sequence — read before writing code in
`core/` or non-trivial code in `src/`.

**Authority order when documents disagree:** `docs/idea.md` (product intent) → `docs/specs/*.md`
(current/intended behavior) → `docs/dev-plan.md` (implementation sequence — not authoritative on
behavior) → `docs/decisions/*.md` (rationale) → `docs/dev-log/*.md` (history) → `docs/notes/*.md`
(non-authoritative working notes).

- `docs/specs/architecture.md` — crypto, key storage, trust boundaries, plugin runtime, detector
  hosting, export sanitization.
- `docs/specs/api.md` — the Tauri command surface and session-gating table. Do not invent command
  names not listed here.
- `docs/specs/data-model.md` — types, SQLCipher schema, envelope plaintext, keystore item fields.
- `docs/specs/testing.md` — TDD process and the mutation-testing gate (below).
- `docs/specs/ui.md` — screen behavior for the Svelte frontend.
- `docs/dev-plan.md` — the ordered chunk sequence (`W0`, `W1`, `W2`, …) this codebase is built in;
  each chunk lists what it delivers, depends on, and explicitly must **not** do yet. Work one chunk
  at a time; don't pull in a later chunk's behavior "while in the file."

**TDD is mandatory for every TCB (Rust core) behavior change**, and for any frontend code beyond view
wiring: write a failing test that cites the spec clause it protects (`FR-…`, `AC-…`, `C-ARCH-…`, an
`api.md` command name), then the minimum code to pass it.

**Mutation gate** (`cargo-mutants`, PR-blocking on the modules listed in `docs/specs/testing.md`
§5.3 — overlap/redaction, export sanitization, share egress/OQ-6, audit HMAC + crash-window,
retention paranoid default, session gating table, DEK-destroy delete, envelope AAD/SQLCipher raw-key
opening, and the Ollama loopback/offset-verification code once it lands): these require mutation
score 1.00 after annotated equivalent-mutant exclusions — an unexplained survivor fails CI. Every
other core module needs S ≥ 0.70.

**Hard invariants, enforced structurally, not just by review:**
- No new plaintext-to-disk path, ever (`docs/specs/architecture.md` §5).
- No new Tauri command name absent from `api.md`.
- Passphrases/keys never enter an `ApiError` message (`core/src/api.rs` only builds errors from
  `&'static str` classes — no `format!` with caller input).

## Skills

- `skills/project-knowledge/` (name: `knowledge-governance`; mirrored at
  `.cursor/skills/knowledge-governance/` for Cursor) — the skill that governs how `docs/` itself is
  maintained: specs hold current truth, decisions hold rationale, dev-log holds implementation
  history, indexes are navigation only. Load it before restructuring `docs/`, adding a new spec/
  decision/dev-log entry, or reviewing documentation health — it defines what belongs in each of
  those, when to update vs. create, and how to keep indexes skimmable.

## Current state

Implementation has completed `W0` (repo skeleton), `W1` (envelope crypto primitives —
`core/src/crypto/`), `W2` (account/keystore/session commands — `core/src/keys.rs`,
`core/src/keystore/`, `core/src/account.rs`, `core/src/session.rs`), `W3` (SQLCipher vault —
`core/src/vault.rs`; Opus-reviewed per the agent roster, 2 blocking findings fixed), `W4` (session
gating table — `SESSION_TABLE`/`command_allowed` in `core/src/session.rs`), `W5` (audit chain
and integrity — `core/src/audit.rs`; Opus-reviewed, 3 blocking findings fixed, each verified
against a real mutation; `SessionState::DegradedIntegrity` is now reachable), `W6` (retention
config — `core/src/config.rs`, the first envelope-encrypted artifact this codebase writes), `W7` (Linux keystore fallback — `select_backend`/`select_backend_with` in
`core/src/keystore/mod.rs`), and `W8` (import plain text — `core/src/importer.rs`,
library-only). `W9`
(import PDF — `import_pdf` in `core/src/importer.rs`, via `pdf-extract`), `W10` (catalog
and `import_document`/`list_documents`/`get_document` — `core/src/catalog.rs`, plus the
architecture §6.2 audit-persist cadence), and `W11` (retention-confirmed gate on
`import_document` — AC-6/AC-7). `W12` (detector host + stub — `StubDetector` in
`core/src/detector/`, wired into `import_document`). `W13` (pattern pack
`pg-patterns-uk-v1` — `core/src/detector/patterns_uk.rs`). `W14` (`pg://detect-progress`
— `ProgressSink` on `SessionManager`; synchronous 0→1 around detect). `W15a` (hybrid ONNX
`pg-hybrid-v1` — `HybridV1` in `core/src/detector/hybrid.rs`; SHA-256 pin). `W15b`
(optional Ollama backend `pg-hybrid-ollama-v1` — `core/src/detector/ollama.rs`; loopback
HTTP, handshake/allowlist/digest, verify-then-trust offsets). `W15c` (backend selection —
`get_detector_preference`/`set_detector_preference` in `core/src/session.rs`; per-detect
choice between `pg-hybrid-v1` and `pg-hybrid-ollama-v1`; audit `detect` honesty; AC-1..AC-4
keep the stub via `with_detector`). `W16` (`open_approval` / `get_approval_view` /
`set_field_decisions` — one RAM approval session in `core/src/session.rs`; span text only
on those commands). `W17` (overlap / nested fields — `core/src/overlap.rs`; innermost keep,
partial overlap redact-wins; table-driven + `proptest`). `W18` (`submit_approval` —
canonical `ApprovedVersion` with `redacted_content`; discard RAM original; audit `approve`).
`W19` (`abort_approval` and lock vs retention — discard unapproved rows dropped; retain
may `open_approval` again). `W20` (`delete_document` — overwrite-and-drop wrapped DEKs; audit `delete`).
`W21` (`delete_retained_original` — idempotent; audit `discard_original` only when one
existed). `W22` (variants) is next;
see `docs/dev-plan.md` for the full sequence and `docs/dev-log/` for what each completed
chunk did and any problems hit along the way.

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues (`Elvin-Arrow/privacy-gate`), via the `gh` CLI. See
`docs/agents/issue-tracker.md`.

### Domain docs

Single-context: this repo's own `docs/idea.md` + `docs/specs/` (current truth) and
`docs/decisions/` (rationale, in place of `docs/adr/`) — governed by the `knowledge-governance`
skill above, not the generic `CONTEXT.md` convention. See `docs/agents/domain.md`.
