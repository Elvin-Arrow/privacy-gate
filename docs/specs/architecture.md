# Architecture Specification — Privacy Gate v1

> Scope: how the v1 system is built inside the component boundaries of
> [`design.md`](./design.md). This spec owns crypto, key storage, trust boundaries, plugin
> runtime shape, detection-model hosting, export sanitization, and the architecture-owned open
> questions (OQ-3, OQ-5, OQ-12, OQ-13, OQ-17, OQ-18). Command surface:
> [`api.md`](./api.md). Verification: [`testing.md`](./testing.md). Types, SQLCipher schema,
> envelope plaintext, and keystore item fields: [`data-model.md`](./data-model.md). It does
> **not** specify UI layout or TS framework (→ [ui.md](./ui.md); decision 0008: Svelte 5), and does **not** restate data-model
> structs.
>
> Source of truth for requirements: [`srs.md`](./srs.md). Component decomposition:
> [`design.md`](./design.md). Types and persistence: [`data-model.md`](./data-model.md).
> Stack: [decision 0003](../decisions/0003-v1-tech-stack.md).
> Architecture resolutions: [decision 0004](../decisions/0004-v1-architecture.md).
>
> Open questions referenced as `OQ-x` live in [`../notes/open-questions.md`](../notes/open-questions.md).

---

## 1. Purpose

Privacy Gate v1 is a local-first, single-user desktop app. This spec makes the design
implementable: where secrets live, what the trusted computing base is, how envelope encryption
and deletion work, how the audit trail is tamper-evident, how plugins can grow into a third-party
phase without rework, and how detection and export meet the privacy constraints structurally
rather than by policy at each call site.

---

## 2. Architectural overview

### 2.1 Process model
v1 is **one OS process**: a Tauri 2.x binary. The Rust core is a library linked into that
binary and invoked through Tauri commands. There is no sidecar, daemon, or helper process
**that this app spawns or manages**. The optional Ollama detector backend (§10.1, decision
0009) is a pre-existing, independently-installed, independently-managed local service the user
runs on their own account; the app never bundles, launches, or supervises it. The app's own
process boundary is unchanged — what changes is that the TCB may, under §10's constraints, make
a constrained outbound call to a service outside that process.

The TypeScript frontend runs in the OS webview. On some platforms the webview is a separate
OS process; the architecture treats it as a **separate trust domain** regardless (C-ARCH-1).

Locking the vault zeroizes in-process secrets in the Rust core and drops the SQLCipher key so
the on-disk database is sealed. The process may keep running (the lock screen); it must not
retain the master key or decrypted artifacts.

### 2.2 Layering

```
┌─────────────────────────────────────────────────────────────┐
│  Webview (untrusted for secrets)                            │
│  TS frontend — review/approve, share, audit views           │
└──────────────────────────┬──────────────────────────────────┘
                           │ Tauri IPC (API spec); no fs/http/shell
                           ▼
┌─────────────────────────────────────────────────────────────┐
│  Trusted computing base — Rust core                         │
│  Key Manager · Vault · Importer · Detector · Approval       │
│  Share Engine · Audit Trail · Plugin Host · Config          │
│  (design.md §2)                                             │
└─────────────┬───────────────────────────────┬───────────────┘
              │                               │
              ▼                               ▼
   OS keystore (wrapped           SQLCipher DB in app-data dir
   master key + audit head)       (envelope-encrypted artifacts)
              │
              │  Cloud AI plugin only, when invoked
              ▼
         TLS to user-configured model endpoint
```

### 2.3 Trust boundaries
| Boundary | What may cross | What must not |
|---|---|---|
| Webview ↔ Rust core | Commands and responses defined by the API spec. During review/approve: unapproved document structure + detected spans (C-DES-1). During share preview: redacted preview artifact only. Audit rows, never document content. | Master key, passphrase, DEKs, API keys after submit, redacted field text, retained originals, SQLCipher key. |
| Rust core ↔ disk | SQLCipher ciphertext; wrapped master-key blob is in the OS keystore, not the data file. | Plaintext documents, plaintext metadata, keys. |
| Rust core ↔ OS keystore | Wrapped master key, KDF params, audit-head cursor. | Passphrase (never stored). |
| Rust core ↔ network | Cloud AI plugin: approved content + user-configured endpoint. | Detection traffic, originals, redacted fields, vault keys, API keys in logs. |
| Rust core ↔ local Ollama (loopback only, decision 0009) | Chunked document text, to a pinned/allowlisted local model, over `127.0.0.1`/`::1` **only** (no DNS, no proxy), after handshake verification (§10.1). Returned entities are byte-verified against the exact chunk before being trusted (§10.1). | Any non-loopback address; any tag not on the pinned allowlist, including `-cloud` tags; unverified offsets; document content when `detector_preference` is `bundled_only` or the handshake/allowlist check fails. |
| Plugin Host ↔ plugin | Approved content with overrides already applied; host-mediated I/O per §8. | Vault, Key Manager, `Document.raw_bytes`, DEKs, other plugins' secrets. |

### 2.4 Threat model (v1)
In scope:
- **Stolen data file** (the SQLCipher DB, copies, backups of the app-data dir): unusable without the passphrase (FR-4.4, NFR-S3).
- **Stolen data file + OS keystore item, vault locked**: still unusable without the passphrase (the keystore holds only a wrapped key).
- **Post-hoc modification or truncation of the audit log** by an attacker who does not have the vault key: detectable (NFR-R1, §6).
- **Export file in a recipient's hands**: contains no recoverable redacted text (NFR-S4, §11).

Out of scope for v1 (residual risk, documented):
- An attacker who can dump the process while the vault is **unlocked** (memory disclosure).
- A compromised trusted computing base (malware in the Rust core / replaced binary) forging new audit entries — HMAC authenticity is relative to the vault key the TCB holds.
- OS crash dumps or swap remnants of document plaintext (keys are mlocked when the OS allows;
  documents are not — §5).
- Linux Secret Service fallback: coordinated rollback of `vault.db` plus the fallback
  keystore file (anti-truncation does not apply; passphrase wrapping still does — §3.2).
- Physical observation of the review/approve UI.

---

## 3. Crypto and key management

Resolves OQ-18 for v1 scope. Rationale: [decision 0004](../decisions/0004-v1-architecture.md).

### 3.1 Key hierarchy
```
passphrase (user, never stored)
  └─ Argon2id ──► wrap_key (32 bytes, ephemeral)
        └─ AEAD wrap ──► vault_master_key (32 bytes, random at first run)
              ├─ HKDF-SHA-256 info="pg-db-v1"        → sqlcipher_key
              ├─ HKDF-SHA-256 info="pg-audit-mac-v1" → audit_mac_key
              └─ AEAD wrap of per-artifact DEKs
                    └─ AEAD(document | metadata | plugin secret)
```

- **`vault_master_key`**: 256-bit, CSPRNG at first run. Never written unwrapped. Lives in Key
  Manager memory only while unlocked (page-aligned mlock, §5.2; zeroized on lock).
- **Per-artifact DEK**: 256-bit, CSPRNG per stored object (approved version, retained original,
  variant, plugin secret, config blob, document_meta). Wrapped by `vault_master_key` and stored beside the
  ciphertext. **Irrevocable delete (FR-4.6, NFR-R2) is cryptographic erasure:** the Vault
  destroys the wrapped DEK and the ciphertext. Without the DEK the blob is unrecoverable even
  if pages remain on disk.
- **AEAD**: XChaCha20-Poly1305. Random 24-byte nonce stored with the ciphertext. Additional
  authenticated data is length-prefixed to avoid concatenation collisions:

  ```
  AAD v1 =
    u8  aad_version = 1
    u8  artifact_kind     // ArtifactKind; codes only in data-model.md §6
    u16be doc_id_len
    doc_id UTF-8 bytes    // len 0 if not document-scoped
    u32be format_version  // artifact schema version, not Unix time
  ```

- **KDF**: Argon2id v1.3. Parameters are stored with the wrapped master key. Floor: OWASP
  minimum current at implementation. Tune upward so unlock stays within the design budget
  (≤ 1 s on the mainstream laptop of design.md §7) without dropping below the floor.
- **HKDF**: HKDF-SHA-256 (RFC 5869). Salt = the 20-byte ASCII `privacy-gate-hkdf-v1`. Info
  labels as in the diagram (`pg-db-v1`, `pg-audit-mac-v1`). Output length 32 bytes.
- **SQLCipher keying:** `sqlcipher_key` is a raw 256-bit key, **not** a passphrase. Open with
  `PRAGMA key = "x'<64 lowercase hex chars>'"` (or `sqlite3_key_v2` with the 32 raw bytes).
  Do not pass the key as a UTF-8 passphrase; that would apply SQLCipher's default PBKDF2
  (~256k iterations) on top of HKDF and blow the unlock budget. Do not set `kdf_iter = 0`
  (invalid on some SQLCipher builds); raw `x'...'` form skips the passphrase KDF.
- **Libraries (Rust)**: `argon2`, `chacha20poly1305`, `hkdf`, `sha2`, `hmac`, `zeroize`,
  `subtle` (MAC compares), `memsec` (page-aligned locked pages for key material). No
  hand-rolled crypto. SQLCipher via `rusqlite` with bundled SQLCipher. OS keystore via
  `keyring` (macOS Keychain, Windows Credential Manager, Linux Secret Service).

### 3.2 Key storage (honours decision 0003)
The OS keystore holds a single `KeystoreItem` per local account. **Fields:**
[`data-model.md` §5.9](./data-model.md) (`KeystoreItem`, `Argon2idParams`, `AuditHead`).
This section owns wrap AEAD, Linux fallback, and that the passphrase is never stored.

The SQLCipher database in the app-data directory does **not** contain the master key. A stolen
data file alone has neither `sqlcipher_key` nor any DEK.

**Linux fallback:** if Secret Service is unavailable, persist the same `KeystoreItem` as a
`0600` file under the app-data directory, written via temp-file + `fsync` + atomic `rename`.
This is weaker: the wrapped blob sits next to the DB, so a stolen app-data directory is one
artifact, not two, and **anti-truncation (§6.2) does not survive a coordinated rollback of
`vault.db` together with the fallback file**. Passphrase wrapping still holds. The Key
Manager records which backend is in use so the testing spec can treat the fallback as a
distinct configuration with a degraded threat model.

The passphrase is never written to disk, keystore, logs, or audit payloads.

### 3.3 Unlock, lock, passphrase change
- **Unlock:** Key Manager loads `KeystoreItem`, derives `wrap_key`, unwraps `vault_master_key`,
  derives `sqlcipher_key` and `audit_mac_key`, opens the DB, verifies the audit chain against
  `audit_head` (§6.3). Fast-forward of a crash window opens normally. Integrity failure opens
  only the degraded audit-report session (§6.3) — no artifact decrypt. Passphrase failure
  zeroizes and refuses (no partial open).
- **Lock:** zeroize master key, wrap key, DEKs, decrypted artifact caches, and the SQLCipher
  connection key; close the DB.
- **Change passphrase (v1):** re-derive a new `wrap_key` from the new passphrase and a new
  salt; re-wrap the **same** `vault_master_key`. DEKs and ciphertext do not rotate. This is
  KEK rotation, not master-key rotation.
- **Master-key rotation:** not a v1 user feature. Envelope encryption makes a later rotation
  possible (unwrap all DEKs, re-wrap under a new master) without a storage-format break.
- **Recovery:** **not in v1.** There is no recovery key, no cloud escrow, no security questions.
  A lost passphrase makes vault contents unrecoverable. This is a product-visible consequence
  of C-4 and NFR-S2; a recovery feature would be an idea.md amendment plus a new decision.

### 3.4 First-run
Key Manager generates `vault_master_key`, creates the SQLCipher DB, writes `KeystoreItem`, and
creates the local account record (§7). No network step (OQ-5).

---

## 4. Storage

### 4.1 Locations
Platform app-data directory (exact path is implementation; not next to the install image):

```
privacy-gate/
  vault.db              # SQLCipher database
  models/ner-pii.onnx   # shipped detector NER weights (read-only resource; may live in the app bundle instead)
```

The wrapped master key lives in the OS keystore, not under this directory (except the Linux
fallback file, §3.2).

v1 has no vault backup/restore command and no reinstall-pickup product flow. Copies of this
directory (and of a stolen data file) remain a stolen-file threat (§2). A later phase
(idea.md: vault backup / restore, including reinstall re-attachment) will specify how a
reinstalled app continues with existing vault data when app-data and keystore survive, and
what a user-initiated backup looks like if they do not. That phase is not passphrase
recovery (C-ARCH-7).

### 4.2 Database
One SQLCipher database. The DB key is `sqlcipher_key` from §3.1. **Tables, envelope JSON,
kind codes, and catalog columns:** [`data-model.md`](./data-model.md).

Every document payload and every document-related metadata field that would be useful to an
attacker is an envelope-encrypted blob (DEK-wrapped). SQLCipher is defense in depth for the
catalog itself (FR-4.3, FR-4.4).

The Detector model file is not secret; it is integrity-checked by a pin (SHA-256 of the
shipped artifact) at load time. A mismatched pin is a hard failure, not a network fetch.

The optional Ollama backend (§10.1, decision 0009) has no shipped artifact to pin; instead the
core verifies the model's Ollama-reported digest (`/api/show`) against a digest pinned per
allowlisted tag at implementation time. A digest mismatch is a hard fallback to `pg-hybrid-v1`,
the same discipline as a mismatched ONNX pin, not a silent re-pin.

### 4.3 Deletion
SQL row order: [`data-model.md` §7](./data-model.md). This section owns cryptographic
erasure. Vault deletion of an approved version, retained original, or variant:

1. Overwrite-and-drop the wrapped DEK for that artifact.
2. Delete the ciphertext blob.
3. Append a `delete` audit event (§6).
4. Do not rely on filesystem overwrites or `VACUUM` for NFR-R2; cryptographic erasure is the
   guarantee. `VACUUM` may still run as hygiene.

1. Overwrite-and-drop the wrapped DEK for that artifact.
2. Delete the ciphertext blob.
3. Append a `delete` audit event (§6).
4. Do not rely on filesystem overwrites or `VACUUM` for NFR-R2; cryptographic erasure is the
   guarantee. `VACUUM` may still run as hygiene.

Audit entries are **not** user-deletable in v1 (append-only, §6).

---

## 5. Transient plaintext (resolves OQ-17)

### 5.1 Disk
Plaintext document content, redacted-field text, passphrases, master keys, DEKs, and Cloud AI
API keys shall **never** be written to disk, including temp files, crash-reporter payloads, and
debug dumps the app controls.

PDF import and export run in memory. If a library cannot be configured for memory-only I/O it
shall not be used. The Importer's `raw_bytes` hand-off in design.md §2.1 is the only
component-level destruction point; this spec adds: no library underneath the Importer may
persist those bytes.

### 5.2 Process memory
| Material | Handling |
|---|---|
| Passphrase, wrap_key, vault_master_key, DEKs, sqlcipher_key, audit_mac_key, API keys | `zeroize` on drop. `vault_master_key`, `wrap_key`, `sqlcipher_key`, and `audit_mac_key` live on **page-aligned locked pages** via `memsec` (`mlock` / `VirtualLock`). `mlock` is page-granular; do not lock ordinary heap `Vec`s. If the OS denies locked pages (`RLIMIT_MEMLOCK`, `VirtualLock` failure), continue with zeroize-only and record a non-content diagnostic; do not refuse unlock. |
| `Document.raw_bytes`, decrypted `ApprovedVersion`, detection buffers | Process memory only; zeroize on Vault ack / session end / lock. Not mlocked (size). **Exception:** when `pg-hybrid-ollama-v1` is selected (§10.1, decision 0009), chunked document text is sent over the verified loopback socket to Ollama for that one call; this is the one explicit exception to "process memory only" and is bounded by §10.1's IP-literal/no-DNS/no-proxy/handshake rules. Not sent at all when `detector_preference` is `bundled_only` or the handshake/allowlist check fails. |
| Frontend review/approve payload | Permitted by C-DES-1. Held only for the Approval Engine session (`AwaitingDecisions` / `Decided`). Core shall not resend it after `Committed` or `Aborted`. The webview heap is not zeroizable; this is an accepted residual. |
| Share preview | Redacted artifact only; no redacted-field text in the webview. |
| Logs | Never contain document text, field text, keys, or API keys. Detector labels and field ids are allowed in audit payloads. |

### 5.3 Crash reporters and cores
v1 ships **no** third-party crash reporter. The app does not write its own heap dumps. OS-level
core dumps and swap of document plaintext are residual risk on an unlocked machine; they are
out of the v1 threat model (§2.4).

### 5.4 Clipboard and screenshots
The core never writes the clipboard. Screenshot/clipboard UX is UI spec; architecture forbids
the core from placing redacted-field text or originals on the clipboard.

---

## 6. Audit-trail integrity (resolves OQ-3)

Fills the crypto primitive left open by design.md §2.6. Logical `AuditEntry` / `EventPayload`
and SQL `originals_flag`: [`data-model.md` §5.8](./data-model.md). This section owns HMAC,
the hash chain, anti-truncation, and the **canonical byte encoding**.

### 6.1 Mechanism
Append-only hash chain + HMAC, encrypted at rest inside SQLCipher.

- **`prev_entry_hash`**: SHA-256 over the canonical encoding of the previous `AuditEntry`
  excluding `entry_signature`. Genesis uses a fixed 32-byte zero digest.
- **`entry_signature`**: HMAC-SHA-256(`audit_mac_key`, canonical encoding of the entry
  including `prev_entry_hash`, excluding `entry_signature`).

**Canonical encoding v1** (bit-stable; changing it is an architecture amendment), concatenated
big-endian fields, no JSON envelope:

```
u8   encoding_version = 1
u64  sequence
u8   event_type        // data-model EventType integer (1..6)
u64  produced_at_unix_ms
u8   doc_id_present    // 0=None, 1=Some
u16  doc_id_len        // 0 if present=0
[u8; doc_id_len]       // UTF-8 DocId
u8   originals_flag    // data-model §5.8: 0=unset, 1=false, 2=true
u32  payload_len
[u8; payload_len]      // EventPayload as UTF-8 RFC 8785 JCS
[u8; 32] prev_entry_hash
```

`entry_signature` is not part of this byte string. HMAC and the next entry's `prev_entry_hash`
SHA-256 are both over it. EventPayload objects must be serialized with RFC 8785 (JCS) so map
key order is deterministic.

HMAC (not an asymmetric signature) is the v1 primitive: the only verifier is the vault owner
who already holds `vault_master_key`. Independent public verification is a later phase.

### 6.2 Anti-truncation head
The OS keystore `KeystoreItem.audit_head` stores `{sequence, head_hash}` of the latest
**persisted** accepted entry.

While unlocked, the live head is held in Key Manager memory. Persist to the keystore (same
item, not a second slot) on: lock, process-exit flush, after every `Share` event, and every
32 appends, whichever comes first. Keystore IPC on every append would stall batch import /
detect / approve.

An attacker who edits the DB without the vault key cannot produce a valid HMAC. An attacker
who truncates the log still fails verification against a **persisted** `audit_head` that is
ahead of the truncated tail — except on the Linux file fallback, where rolling back the
whole app-data directory rolls back DB and head together (§3.2, §2.4).

### 6.3 Verification and crash recovery
On every unlock, the Audit Trail replays the chain from genesis and checks each HMAC.

Let `H` be the persisted `audit_head` and `T` the verified tail of the DB chain.

- **`T == H`:** open normally.
- **`T.sequence == H.sequence + k` for k in 1..32, chain valid, every extra entry HMAC-valid,
  and entry `H.sequence` matches `H.head_hash`:** expected crash window (DB committed,
  keystore persist not yet done). **Fast-forward** `audit_head` to `T` and open normally.
  Do not treat this as tampering.
- **`T.sequence < H.sequence`, or HMAC/chain break, or the entry at `H.sequence` does not
  match `H.head_hash`:** integrity failure.

On integrity failure the vault does **not** decrypt documents, originals, variants, or plugin
secrets (no "open anyway" for content). It **does** enter a degraded session: Key Manager has
unwrapped `vault_master_key` (passphrase was correct) only far enough to verify the chain and
to let the Audit Trail return a verification report (first bad `sequence`, `H` vs `T`, whether
the break is truncation vs modification). That report is what FR-7.4 / NFR-U1 need when
tamper-evidence fires. Wording: UI spec. Recovering a tampered vault (restore from user
backup, etc.) is out of v1; user-initiated vault backup/restore is a later phase (idea.md).

### 6.4 What this does and does not prove
- Detects modification of stored entries and truncation against a keystore-persisted head
  (NFR-R1), with crash-window fast-forward so a power loss cannot brick the vault.
- Does **not** detect a compromised TCB appending new, well-MAC'd entries while unlocked
  (§2.4). An attacker who knows the passphrase can decrypt everything *and* re-HMAC a rewritten
  log; that is the same TCB compromise, not extra.
- Does **not** provide anti-truncation when the Linux fallback file and `vault.db` are rolled
  back together (§3.2).
- FR-7.3's "without trusting the app on faith" is inspectability of a complete, tamper-evident
  record (FR-7.4, NFR-U1), not a third-party audit proof.
- Audit payloads never include redacted field text, originals, keys, or API keys. Share events
  record destination (recipient note or plugin id + endpoint host), doc ids, and the
  `no_originals_left_device` flag per design.md §2.6.

---

## 7. Account and identity (resolves OQ-5)

v1 accounts are **local-only**. First-run does not contact a server. No email, no network
identity, no remote account id.

`LocalAccount` fields: [`data-model.md` §5.6](./data-model.md). This section owns the
identity policy (no network required to unlock).

Day-to-day unlock is passphrase + on-device `KeystoreItem` (FR-8.3). This matches the idea
doc: an account exists so later backup, sync, and mediated sharing can bind to it; v1 does
not implement those phases and therefore must not introduce a server.

A later network identity is an **additive binding** (a new field / new decision) on
`LocalAccount`. It must not become a requirement to open the vault. The Key Manager first-run
flow in design.md §2.10 stays local.

---

## 8. Plugin architecture (resolves OQ-13; verifies FR-9.5 / NFR-E1)

### 8.1 Host API (the extension point that avoids rework)
Host API version: **`pg-host-api-1`**. All plugins — v1 in-process first-party and a later
third-party runtime — talk to this versioned **host API**. The Plugin Host is the only object
that implements it. Bumping the version is an architecture amendment.

```
Host capabilities a plugin may be granted (declared, not ambient):
  - consume_approved_content   // always; this is the input
  - network_https(allowlist)   // Cloud AI: the user-configured host only
  - emit_text_output           // read-only text back to the UI
  - register_detector_fields   // detector plugins
  - subscribe_event(import|redact|share)  // reactive; empty in v1

Host capabilities a plugin is never granted:
  - vault_read_original
  - vault_read_unapproved
  - key_manager_access
  - raw_filesystem
  - ambient_network
  - other_plugin_secrets
```

Rust traits (names indicative; API spec / crate layout may rename):

- `OutputConsumer` — approved documents in, `PluginOutput` (read-only text or export bytes)
  out.
- `DetectorPlugin` — `Document` intermediate representation in (no `raw_bytes` required beyond
  what Detector already holds), `Vec<DetectedField>` out.
- `NewFlow` — orchestration across documents; v1 ships the trait and an empty registry.

The Cloud AI plugin is an `OutputConsumer` compiled into the binary. Detector and new-flow
registries exist and start empty of extra first-party plugins (FR-9.4). The built-in hybrid
detector (§10) is the Detector component, not a plugin.

### 8.2 v1 runtime vs later WASM
- **v1:** in-process Rust, first-party only (decision 0003, C-DES-6). Plugins are compiled in
  and called through the traits. They still receive only host-mediated capabilities; they do
  not `use` Vault or Key Manager.
- **Later third-party phase:** a WASM (or equivalent sandbox) runtime that maps the **same**
  host API to guest imports. Signing, store distribution, and resource limits belong to that
  phase. v1 does not ship a signing PKI or a sandbox, and must not grow call sites that bypass
  the host API — that bypass would be the rework FR-9.5 forbids.

Verification of NFR-E1: every plugin-visible behaviour is expressed via the host API;
third-party support is a new *host implementation*, not a new plugin contract.

### 8.3 Invocation
User-invoked and event-triggered opt-in remain as in design.md §2.7. v1 exercises only
user-invoked Cloud AI. Event subscriptions are registered but have no first-party subscribers.

---

## 9. Cloud AI authentication and network (resolves OQ-12)

Co-owned with the API spec: this spec owns **where the secret lives and which process speaks
HTTP**; [api.md §5.7](./api.md) owns the Tauri commands (`cloud_ai_set_config` / `get` /
`clear` / `test`).

### 9.1 Credential
- **User-supplied** API key (and OpenAI-compatible base URL + model id). **No bundled
  credential** in the binary or resources.
- Stored as an envelope-encrypted plugin secret in the Vault (its own DEK). Readable by the
  Plugin Host only while unlocked, only at invocation.
- The frontend may hold the key only in the submit field until the set-command returns; the
  core never returns the key to the frontend afterwards (presence / last-four is API spec).
- Never written to the audit trail, logs, or share preview.

### 9.2 Network path
- **All Cloud AI HTTP runs in the Rust core** (`reqwest` + `rustls`). The webview has no HTTP
  capability (C-ARCH-2). This makes "what left the device" a core-auditable event, not a
  browser request the UI might decorate.
- TLS 1.2+ to the user-configured host. Redirects that change host are refused.
- Allowlist: the host:port of the configured base URL, nothing else. No `file://`, no
  ambient proxy credentials from the app.
- Request body: approved content with overrides already applied (design.md §2.7). Redacted
  fields are not present.
- Response: read-only text. The plugin does not execute tools, fetch extra URLs, or write
  vault state.

### 9.3 Failure
Missing or invalid API key fails the share before any document content is sent. Failed sends
still emit a share event recording the attempt and error class (not the key, not the body).

---

## 10. Detection-model host

Concrete identity left open by design.md §2.2. The idea doc's "Gemma detected …" line is an
illustrative audit example, not a model commitment as such — but decision 0009 does make Gemma
(via Ollama) a real, optional v1 backend, under the constraints below.

### 10.1 Identity

Two detector identities exist. Selection between them is per-detect (§10.1.3), not fixed at
build time.

**`pg-hybrid-v1`** (decision 0004) — the always-available baseline, zero external
dependencies, two stages, both in-process, no network (NFR-P1, C-5):

1. **`pg-patterns-uk-v1`** — deterministic recognizers for structured identifiers justified by
   the Aisha persona: UK sort code, account number, National Insurance number, NHS number,
   plus email, phone, IBAN, payment-card (Luhn). Pattern expressions live with the
   implementation and are pinned by detector-pack version; this spec pins the *types*, not
   the regex text.
2. **On-device NER** — ONNX Runtime (`ort` crate) in-process, model artifact
   `ner-pii.onnx` shipped with the app (or in the app bundle). v1 model: **GLiNER-small-v2.1**
   (INT8 ONNX), used for PERSON, LOCATION/ADDRESS, ORGANIZATION. SHA-256 pin checked at load
   (§4.2). No download at detection time.

**`pg-hybrid-ollama-v1`** (decision 0009) — optional, preferred when available and allowed:
`pg-patterns-uk-v1` (identical stage) plus NER via a **local Ollama server**, entirely
replacing the ONNX NER stage for that document, never mixed per-field with it.

#### 10.1.1 Ollama network boundary

- **IP-literal loopback only.** The core connects to `127.0.0.1` or `::1`. The hostname
  `localhost` is never resolved via DNS for this feature — the connection target is a literal
  socket address, not a name.
- **No ambient proxy.** The HTTP client used for this feature has proxy environment variables
  (`HTTP_PROXY`, `ALL_PROXY`, `NO_PROXY`, etc.) explicitly disabled — mirrors §9.2's existing
  "no ambient proxy credentials from the app" rule for Cloud AI.
- **Handshake before content.** Before any document text is sent, `GET /api/tags` (probe,
  200 ms timeout) and `GET /api/show` (model detail) responses are checked against Ollama's
  documented response shape and the pinned digest (§10.1.2). A listener that does not speak
  Ollama's actual API — intentionally or by accident — fails this check and the app falls back
  to `pg-hybrid-v1`; document text is never sent to an unverified listener.
- This is a **mitigation, not a guarantee**: a sufficiently capable local process that both
  binds `127.0.0.1:11434` *and* replays a byte-correct Ollama API before the real Ollama
  claims the port could still intercept a detect call. This residual is accepted the same way
  §5.2 accepts the webview-heap residual — bounded by the existing v1 threat model (§2.4),
  which already excludes a fully compromised local machine.
- This boundary (IP-literal-only, no-DNS, no-proxy, handshake-verified) is a **gated
  mutation-testing module, S = 1.00** (testing.md §5.3), tested separately from the OQ-6
  share-egress oracle: OQ-6 governs what a *share* transmits; this governs what the *detector*
  may reach at all.

#### 10.1.2 Model pin and allowlist

- A small, hardcoded allowlist of local Ollama tags is eligible for `pg-hybrid-ollama-v1`,
  seeded with **`gemma4:e2b`** (verified present, local, non-cloud, on the development
  machine via `ollama list`). Extending the allowlist is an architecture amendment — the same
  discipline already applied to GLiNER-small-v2.1's SHA-256 pin (§4.2).
- Any tag with a **`-cloud` suffix** (Ollama's own cloud-relay models — e.g. the
  `gemma4:31b-cloud` tag also observed on the dev machine) is **never** eligible: those route
  through Ollama's remote relay, which would defeat on-device detection outright. An
  unrecognized or cloud-suffixed tag is treated identically to "Ollama absent."
- The core records, at `/api/show` time, the model's Ollama-reported digest and compares it to
  the digest pinned for that allowlist entry (recorded at implementation time). A digest
  mismatch is a hard fallback (`fallback_reason: "digest_mismatch"`, api.md/data-model.md),
  never a silent re-pin.

#### 10.1.3 Backend selection (per detect, not cached at unlock)

`Config.detector_preference` (data-model.md §5.5) is `"auto"` (factory default) or
`"bundled_only"`. At the start of each `import_document` detect phase:

1. `"bundled_only"` → `pg-hybrid-v1`, no Ollama probe.
2. `"auto"` → probe `127.0.0.1:11434` (§10.1.1, 200 ms timeout). Unreachable / timeout /
   handshake failure → `pg-hybrid-v1`, `fallback_reason: "ollama_unreachable"` or
   `"schema_verification_failed"`.
3. Reachable, but served model not on the allowlist or digest mismatched → `pg-hybrid-v1`,
   `fallback_reason: "model_not_allowlisted"` or `"digest_mismatch"`.
4. Otherwise → `pg-hybrid-ollama-v1`. Probing per-detect (not once per unlocked session) means
   a user who starts Ollama mid-session is picked up on the next import without a re-lock.
5. Any failure *during* the Ollama pass (connection drop, malformed generation, offset
   verification failure past the chunk threshold, §10.1.4) fails that document's Ollama pass
   and falls back to `pg-hybrid-v1` for that document — never a partial or unverified result.

Audit `detect` events (data-model.md, api.md §6) record which identity actually ran
(`pg-hybrid-v1` or `pg-hybrid-ollama-v1`), the `model_tag` when Ollama ran, and
`fallback_reason` when it didn't — never a synthesized "hybrid detector ran" that hides which
backend produced the result. The same document detected on two machines (one with Ollama, one
without) may legitimately produce a different field set; the audit trail must show that, not
hide it.

#### 10.1.4 Output contract: verify-then-trust, never search

GLiNER is a span-classification model; Gemma is generative. Free-text output cannot be safely
mapped back to `[byte_offset, byte_length]` by searching for the returned substring — a
document with the same name/number repeated many times makes that search ambiguous, and model
text normalization can break exact matching. Instead:

1. Document text is split into bounded, overlapping chunks (§10.1.5), each with its absolute
   byte offset in the document recorded.
2. Each chunk is sent to Ollama with strict JSON-schema-constrained output (not best-effort
   `format: "json"`): an array of `{ start: u32, length: u32, label: enum, text: string }`,
   `start`/`length` relative to **that chunk's text exactly as sent**.
3. For each returned entity, verify byte-exact: `chunk_text[start..start+length] == text`.
   This is a known-value equality check, not a search — there is no cross-occurrence ambiguity
   because the model was never asked to search, only to point at what it was given.
4. Pass → accept, map to document-absolute offset (`chunk_start + start`). Fail → reject that
   one entity (not the whole chunk); count rejections.
5. If a chunk's rejection rate exceeds an implementation-tuned threshold (tuned against the
   fixture corpus, testing.md §10), the **whole document's** Ollama pass is treated as failed
   → fall back to `pg-hybrid-v1`, `fallback_reason: "offset_verification_failed"`.

Never fuzzy-match, never guess the nearest occurrence, never partially accept an unverified
span. This algorithm — and its mutation coverage — is the load-bearing control against a
silent fail-open (an undetected field reaching export unredacted); it is a gated module
(testing.md §5.3, S = 1.00).

#### 10.1.5 Chunking and lifecycle/budget

- Chunk size and overlap length are implementation constants, tuned against the pinned tag's
  **verified** context window — that number must be recorded (nightly golden job, testing.md
  §11) before the Ollama path ships to production; this spec does not assert an unverified
  figure. Overlap length must exceed any plausible single-entity span so no entity is split
  across a chunk boundary without appearing whole in at least one chunk.
- Detections from overlapping regions are de-duplicated by absolute-offset containment:
  matching label + overlapping/containing absolute span → keep one, preferring the detection
  further from its chunk's edge (less likely to be a boundary truncation artifact).
- **Two-tier performance budget**, distinct from design.md §7's ONNX-path figures (unchanged):
  - **Cold-start / "warming"** (Ollama loading the model into its own memory this session):
    ≤ 20 s. Surfaced via `pg://detect-progress`'s additive `phase: "warming_model"` field
    (api.md §6).
  - **Steady-state per-chunk detection**: measured and recorded against the pinned tag on the
    nightly golden job (testing.md §11), parallel to the existing ONNX nightly golden. If
    steady-state throughput cannot support design.md §7's interactive budget on realistic
    document sizes, that is an architecture amendment, not a silent regression shipped anyway.
- Ollama's own model residency (RAM/VRAM) is outside the app's ≤ 1 GB in-process working-set
  budget (§10.2) — it is a sibling process's footprint the user separately chose to carry by
  installing and running Ollama, not memory this app's own process claims.

### 10.2 `pg-hybrid-v1` lifecycle and budget

Applies to the always-available ONNX baseline; §10.1.5 covers the optional Ollama path.

- Load the NER model lazily on first detect after unlock; unload on lock (RAM).
- Detector RAM budget: ≤ 1 GB working set on the mainstream laptop of design.md §7, on top of
  the rest of the app.
- Time budgets remain design.md §7. If GLiNER-small-v2.1 cannot meet them on the reference
  laptop, replacing the NER artifact is an architecture amendment (decision), not a silent
  swap. The pattern pack must still run.
- **ONNX Runtime distribution:** vendor `ort` with the ONNX Runtime shared library bundled
  inside the Tauri app for each v1 target: macOS aarch64 and x86_64, Windows x86_64,
  Linux x86_64 and aarch64. No download at runtime. Missing or ABI-mismatched runtime is a
  hard failure of the NER stage (pattern pack may still run); do not fetch a replacement.

Detector plugin hooks (§8) receive the same intermediate `Document` and append fields; they
cannot call out to the network (including the loopback exception in §10.1.1, which is
detector-host-only, not exposed to plugins).

---

## 11. Export sanitization

Resolves the mechanism deferred by C-DES-4 / decision 0002 Q11.

The Share Engine **re-renders a new PDF** from `ApprovedVersion.redacted_content` (and any
ephemeral overrides / variant applied for this `ShareRequest`). It never mutates the source
PDF, never uses visual overlays, and never writes an incremental-update PDF that would retain
old content streams.

Redacted spans are omitted from the content stream (true removal). A visible placeholder
glyph/box may be drawn where a span was removed so layout remains readable; the placeholder
must not encode the original characters.

Single-document and multi-document bundles are both newly generated PDFs (design.md §3.7 —
PDF is the v1 export format, including for documents that were imported as plain text; this
is not an architecture narrowing of FR-5.1). Generation uses a from-scratch PDF writer
(`pdf-writer` or equivalent); do not mutate source PDFs with `lopdf`-style incremental
updates.

Plain-text sources are re-rendered to PDF the same way; v1 has no plaintext export path
(design.md §3.7).

---

## 12. IPC and OS capabilities

- Tauri 2 capability ACL: the frontend **cannot** use filesystem, shell, or HTTP plugins
  except a UI-spec save dialog that only persists in-memory bytes the core already
  returned — previewed export `pdf_bytes`, or `get_integrity_report` JSON
  ([api.md §8](./api.md), [ui.md §10.4](./ui.md)). It must not open arbitrary files into
  the webview. It can only invoke the command surface in [`api.md`](./api.md).
- Commands that accept document bytes do so as command arguments into the Rust core, not via
  a frontend-readable path the webview opened. That inbound original is the only time plaintext
  source bytes cross IPC; they never return (API C-API-3).
- CSP on the webview shall deny network fetches (UI/implementation detail owned jointly with
  the UI spec; the architectural rule is C-ARCH-2).
- OS: macOS, Windows, Linux (C-DES-7). Keystore backends as in §3.2.

---

## 13. Interfaces

Responsibility-level; command names are API spec.

| From → To | Architecture contract |
|---|---|
| Key Manager → OS keystore | Load/store `KeystoreItem` (wrapped master key, KDF params, audit head). |
| Key Manager → Vault | Session `vault_master_key` materialised as `sqlcipher_key` + ability to wrap/unwrap DEKs. Passphrase never crosses this boundary. |
| Vault → disk | SQLCipher DB only. Envelope blobs + wrapped DEKs inside. |
| Importer / Detector / Approval / Share | In-process, in-memory; no disk temp (§5). |
| Plugin Host → Cloud AI | Host-mediated HTTPS to allowlisted host; API key injected from Vault secret store. |
| Plugin Host → plugin trait objects | Approved content in; no Vault/Key Manager handles. |
| Audit Trail → Key Manager | After append, update `audit_head`. On unlock, verify chain. |
| Detector → ONNX Runtime | In-process; pinned model file; no network. |
| Share Engine → PDF writer | New file bytes from `RedactedDocument`; no source-PDF mutation. |
| Frontend → Core | Tauri commands only (API spec). |

---

## 14. Dependencies

- **SRS** [`srs.md`](./srs.md) — FR/NFR this spec realises structurally.
- **Design** [`design.md`](./design.md) — components, flows, budgets.
- **Data model spec** [`data-model.md`](./data-model.md) — types, SQLCipher schema, envelope
  kinds, `KeystoreItem` / `LocalAccount` / `AuditEntry` fields.
- **Decision 0003** — Tauri 2.x + Rust + TS; OS keystore; in-process detection and v1 plugins.
- **Decision 0004** — crypto suite, local account, no recovery, plugin host API, Cloud AI
  auth, hybrid detector, re-render export.
- **API spec** [`api.md`](./api.md) — Tauri commands, including Cloud AI config set/clear/test
  and export filename/PDF metadata (OQ-4 API part).
- **Testing spec** [`testing.md`](./testing.md) — TDD, mutation gate, AC-1..AC-6, OQ-6 oracle,
  keystore-fallback configuration, DEK/audit/crash-window checks.
- **UI spec** [`ui.md`](./ui.md) — Svelte 5, screens, copy, save-dialog chrome, first paint.

---

## 15. Constraints

- **C-ARCH-1** The webview is untrusted for secrets and for redacted-field text. Keys, DEKs,
  API keys, originals, and redacted field text stay in the Rust core (C-DES-1 still governs
  the review/approve exception for *unapproved* content).
- **C-ARCH-2** The webview has no network or arbitrary filesystem capability. The only v1
  network path is the Cloud AI plugin in Rust, to a user-configured allowlisted host. A native
  save dialog that only writes in-memory bytes the core already returned (previewed export
  `pdf_bytes`, or `get_integrity_report` JSON) is a UI-spec exception
  ([api.md §8](./api.md), [ui.md §10.4](./ui.md)); it must not open arbitrary files into the
  webview.
- **C-ARCH-3** Detection never uses the network, **except** the optional `pg-hybrid-ollama-v1`
  backend's strictly loopback, IP-literal, non-DNS, non-proxied connection (NFR-P1, C-5,
  §10.1, decision 0009).
- **C-ARCH-4** No plaintext document or key material is written to disk (§5.1).
- **C-ARCH-5** Plugins interact only through the host API (§8). Compiled-in v1 plugins are not
  exempt.
- **C-ARCH-6** Export is a newly rendered PDF; redaction is omission from the content stream
  (§11, NFR-S4).
- **C-ARCH-7** Lost passphrase is unrecoverable in v1 (§3.3).
- **C-ARCH-8** Audit log is append-only. Unlock fast-forwards a bounded crash window; true
  integrity failure is fail-closed for document decrypt and degraded-open for the verification
  report (§6).
- **C-ARCH-9** v1 account is local-only; unlock must not require network (§7, FR-8.3, C-4).

---

## 16. Traceability to SRS and design

| Requirement / design deferral | Architecture coverage |
|---|---|
| FR-4.1..4.5, NFR-S1..S3 envelope encryption, on-device key, stolen file | §3, §4 |
| FR-4.6, NFR-R2 irrevocable delete | §3.1 DEK erasure, §4.3 |
| FR-8.1..8.3 account + local unlock | §7, §3.3–3.4 |
| NFR-R1 tamper-evident audit (OQ-3) | §6 |
| FR-7.3 / NFR-U1 verifiable share record | §6.4, payload rules |
| FR-9.5 / NFR-E1 third-party without rework (OQ-13) | §8 |
| FR-5.2 / C-4 Cloud AI auth (OQ-12) | §9 |
| FR-2.3 / NFR-P1 / C-5 on-device detection | §10 |
| NFR-S4 / C-DES-4 true-removal export | §11 |
| NFR-P3 / C-4 no host/relay/off-device key | §2, §7, §9 |
| OQ-17 transient plaintext | §5 |
| OQ-18 rotation / recovery | §3.3 (change-passphrase yes; recovery no) |
| OQ-5 account network role | §7 |
| design.md Key Manager key storage | §3.2 (item fields → data-model) |
| design.md AuditEntry signature primitive | §6.1 (row fields → data-model) |
| design.md detection model identity | §10 |
| design.md §7 budgets | §3.1 KDF tuning; §10.2 detector RAM/time |

---

## 17. Open questions owned by this spec (resolved here)

- **OQ-3** Hash-chained HMAC-SHA-256 with SHA-256 `prev_entry_hash`, canonical encoding v1
  (§6.1), `audit_mac_key` from HKDF, batched anti-truncation head in the OS keystore,
  crash-window fast-forward, degraded session on true integrity failure (§6).
- **OQ-5** Local-only account; no server at first run or unlock (§7).
- **OQ-12** User-supplied API key, envelope-encrypted in Vault; HTTP only from Rust to an
  allowlisted host; no bundled credential (§9). Command shape: [api.md §5.7](./api.md).
- **OQ-13** Versioned host API + traits; v1 in-process first-party; later WASM maps the same
  API. No signing PKI in v1. Plugins never receive Vault/Key Manager (§8).
- **OQ-17** No plaintext-to-disk; zeroize + mlock for keys; in-memory PDF I/O; no app crash
  reporter; webview residual documented (§5).
- **OQ-18** No recovery in v1; passphrase change re-wraps the same master key; irrevocable
  delete = DEK destruction; master-key rotation deferred as a non-breaking later feature
  (§3.3).

## 18. Open questions deferred (not owned here)

- **OQ-4 (remainder)** Export file naming and metadata fields → **API part resolved**
  ([api.md §7](./api.md)); save-dialog chrome → **resolved** ([ui.md §10.4](./ui.md)).
- **OQ-6 (remainder)** Independent verification of "no originals left device" → **resolved**
  ([testing.md §7](./testing.md)). Architecture emits the flag per design.md §2.6 and never
  transmits `raw_bytes` on a share path (§2.3, §9.2).
- **OQ-14** Retention default initial value → **resolved** by
  [decision 0007](../decisions/0007-retention-default-discard.md): factory `discard`,
  unconfirmed until the user sets a policy; Config/Importer refuse import while unconfirmed.
- TS framework and integrity-failure / lost-passphrase UX copy → **resolved** by
  [ui.md](./ui.md) (decision 0008: Svelte 5).
- Vault backup / restore and reinstall re-attachment → later phase
  ([idea.md](../idea.md)); v1 storage layout is §4.1, not a backup format.

---

## 19. Related Decisions

- [0002 — resolved SRS clarifications](../decisions/0002-resolved-srs-clarifications.md) —
  true-removal export; one canonical version.
- [0003 — v1 tech stack](../decisions/0003-v1-tech-stack.md) — Tauri + Rust + TS; OS keystore;
  in-process detection and v1 plugins.
- [0004 — v1 architecture](../decisions/0004-v1-architecture.md) — crypto suite, local
  account, audit MAC, plugin host API, Cloud AI auth, hybrid detector, re-render export.
- [0005 — review roster](../decisions/0005-review-claude-gemini.md) — Claude + Gemini.
- [0006 — TDD + mutation](../decisions/0006-tdd-and-mutation-testing.md).
- [0007 — retention default](../decisions/0007-retention-default-discard.md).
- [0008 — Svelte frontend](../decisions/0008-frontend-svelte.md).

## 20. Related Work

- [0001-srs-generation](../dev-log/0001-srs-generation.md)
- [0002-design-spec](../dev-log/0002-design-spec.md)
- [0003-architecture-spec](../dev-log/0003-architecture-spec.md)
- [0004-api-spec](../dev-log/0004-api-spec.md)
- [0005-testing-spec](../dev-log/0005-testing-spec.md)
- [0006-oq14-retention-default](../dev-log/0006-oq14-retention-default.md)
- [0007-data-model-spec](../dev-log/0007-data-model-spec.md)
- [0008-ui-spec](../dev-log/0008-ui-spec.md)
