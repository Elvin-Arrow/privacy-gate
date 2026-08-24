# Decision: v1 architecture — crypto, identity, plugins, detection, export

- **Status:** Accepted; **detector-identity clause partially superseded by
  [decision 0009](./0009-ollama-detector-backend.md)** — `pg-hybrid-v1` remains defined
  exactly as below and is still the always-available baseline, but is no longer the sole
  detector identity.
- **Date:** 2026-08-23

## Context

The design spec names ten components and defers the choices that make them implementable:
envelope-encryption primitives and libraries, where the master key lives, key rotation and
recovery (OQ-18), transient plaintext (OQ-17), audit-trail integrity (OQ-3), whether v1
account creation talks to a server (OQ-5), Cloud AI authentication (OQ-12), the plugin
security model that must satisfy "third-party later without rework" (OQ-13, FR-9.5, NFR-E1),
detection-model identity, and the true-removal export mechanism (decision 0002 Q11 / C-DES-4).
Decision 0003 already requires Tauri 2.x, a Rust core, OS-keystore key storage, in-process
detection, and in-process first-party plugins.

## Decision

1. **Crypto suite.** Envelope encryption with a 256-bit vault master key, per-artifact 256-bit
   DEKs, XChaCha20-Poly1305, Argon2id passphrase wrapping, HKDF-SHA-256 for derived keys
   (`sqlcipher_key`, `audit_mac_key`). Libraries: RustCrypto (`argon2`, `chacha20poly1305`,
   `hkdf`, `sha2`, `hmac`) plus `zeroize`, `rusqlite`+bundled SQLCipher, `keyring`. No
   hand-rolled crypto.
2. **Key storage.** OS keystore holds the wrapped master key, KDF params, and audit-head
   cursor. The SQLCipher DB in app-data does not contain the master key. Linux Secret Service
   may fall back to a `0600` app-data file of the same blob. Passphrase is never stored.
3. **Rotation and recovery (OQ-18).** v1 supports passphrase change (re-wrap the same master
   key). v1 does **not** support passphrase recovery or user-facing master-key rotation.
   Irrevocable delete is destruction of the artifact's wrapped DEK (and ciphertext), not a
   disk-overwrite guarantee.
4. **Transient plaintext (OQ-17).** No plaintext documents or keys on disk (including temp
   files). Keys are zeroized and the master key is mlocked; document buffers are zeroized but
   not mlocked. No third-party crash reporter. Webview heap during review/approve is an
   accepted residual (C-DES-1).
5. **Audit integrity (OQ-3).** SHA-256 hash chain + HMAC-SHA-256 with `audit_mac_key`.
   Canonical encoding v1 is length-prefixed big-endian plus RFC 8785 JCS payloads.
   Anti-truncation head stored in the keystore, persisted in batches (every 32 appends, every
   share, on lock). Unlock fast-forwards a bounded crash window (DB ahead of head). True
   integrity failure is fail-closed for document decrypt and degraded-open for a verification
   report (FR-7.4). HMAC is owner-verifiable, not a public third-party proof; a compromised
   TCB or passphrase-holder can still rewrite the log. Linux file-fallback does not provide
   anti-truncation against directory rollback.
6. **Account (OQ-5).** Local-only. No server at first run or unlock. A future network identity
   is an additive binding and must not become required to open the vault.
7. **Plugins (OQ-13).** A versioned host API (declared capabilities, no ambient Vault/Key
   Manager access) is the extension point. v1 runtime is in-process first-party; a later WASM
   host implements the same API. No signing PKI in v1.
8. **Cloud AI auth (OQ-12).** User-supplied API key, envelope-encrypted in the vault. HTTP only
   from Rust to the user-configured host (allowlisted). No bundled credential. Webview has no
   network capability.
9. **Detector identity.** `pg-hybrid-v1`: UK-justified deterministic pattern pack plus
   in-process GLiNER-small-v2.1 (INT8 ONNX via `ort`), shipped and hash-pinned. The idea doc's
   "Gemma" line is illustrative, not a model commitment. No network at detection time.
10. **Export sanitization.** Re-render a new PDF from `RedactedDocument`; never mutate the
    source PDF; no overlay-only redaction.

## Rationale

- **XChaCha20-Poly1305** gives large random nonces (no GCM nonce-reuse footgun) and does not
  require AES-NI, which matters on mixed desktop hardware. Envelope DEKs make deletion a key
  destruction, which is the only NFR-R2 story that does not depend on filesystem forensics.
- **OS keystore + passphrase wrap** honours decision 0003 and FR-4.4 together: a stolen data
  file lacks the wrapped key; a stolen keystore item still needs the passphrase.
- **No recovery in v1** follows C-4 / NFR-S2 and the idea doc's "key never leaves the device."
  A printed recovery secret is a second key that *does* leave the device in the user's hands
  and is easy to get wrong; it needs an explicit product decision.
- **HMAC rather than Ed25519** for the audit log matches a single-user local verifier. The
  keystore head closes truncation, which a chain alone does not. Claiming independent
  public audit would oversell NFR-R1.
- **Local account** is the only choice that does not introduce hosting (idea-doc out of
  scope) while still leaving a stable `AccountId` for later sync/sharing phases.
- **Host API rather than a v1 WASM sandbox** keeps v1 first-party simple (decision 0003) but
  makes the third-party phase a new runtime, not a new plugin contract — that is the FR-9.5
  test.
- **User-supplied AI keys in the vault, HTTP in Rust** keeps C-4 (no off-device *vault* key)
  and makes "what left the device" auditable in one place. Bundled credentials would be a
  secret in the binary and a vendor lock-in.
- **Hybrid detector, not Gemma**, is what can plausibly hit design.md §7 on an 8 GB laptop
  without a detection-time network call. Structured UK identifiers are regex-reliable;
  GLiNER covers names/addresses/orgs. Pinning the artifact prevents silent model swaps.
- **Re-render** is the only PDF strategy that satisfies true removal; in-place stripping and
  overlay boxes leave recoverable streams (incremental updates especially).

## Alternatives Considered

### AES-256-GCM instead of XChaCha20-Poly1305

Rejected for nonce-management risk on high-volume per-artifact encryption and weaker
hardware-agnostic performance. GCM remains a possible later swap behind the AEAD interface.

### SQLCipher-only (no per-artifact DEKs)

Rejected: deleting a row does not cryptographically erase it; WAL/pages persist. Envelope
DEKs make NFR-R2 implementable. SQLCipher stays as defense in depth for the catalog.

### Store the master key only on disk next to the DB

Rejected: contradicts decision 0003's OS-keystore requirement and collapses two artifacts
into one stolen-file event.

### Recovery key printed at first run

Rejected for v1 as an idea-doc amendment, not an architecture default. Revisit when backup /
cross-device sync is in scope.

### Ed25519 audit signatures

Gives a public verifying key, but v1 has no independent verifier and would add a second key
to persist and rotate. HMAC + keystore head meets NFR-R1. Asymmetric signatures can be a
later phase.

### Network account at first run

Rejected: v1 has nothing to register with, and it would make "no network identity to open
the vault" a policy rather than a structural property. Additive binding later is cheaper
than ripping out a server dependency.

### WASM plugin runtime in v1

Rejected as unnecessary for first-party-only code (decision 0003). Shipping WASM without
third-party plugins does not reduce risk and delays the vault.

### Bundled Cloud AI credential / frontend `fetch`

A bundled key is an off-binary-secret we do not control and cannot rotate per user.
Frontend `fetch` would let the webview see approved content *and* the key and would bypass
core audit of the exact bytes on the wire.

### In-process Gemma (or similar 2B+ LLM) as the v1 detector

Rejected against the 8 GB RAM laptop budget and the ≤ 5 s / 1 MB detection budget. The idea
doc uses "Gemma" as an audit-trail illustration, not a stack requirement.

### Overlay or in-place PDF redaction

Rejected by decision 0002 Q11; overlays and incremental updates leave recoverable text.

## Consequences

- Architecture spec [`docs/specs/architecture.md`](../specs/architecture.md) is the
  implementable source for these choices (C-ARCH-1..9).
- Gemini review (2026-08-23) forced: crash-window fast-forward, canonical encoding v1,
  SQLCipher raw-key form, length-prefixed AAD, batched keystore head, `memsec` mlock,
  degraded integrity session, Linux-fallback threat-model honesty.
- API spec must not return API keys to the frontend after submit, must not grant webview
  HTTP/fs, and must expose Cloud AI config set/clear/test.
- UI spec must cover fail-closed audit-integrity messaging and the lost-passphrase
  consequence (no recovery).
- Testing spec [`docs/specs/testing.md`](../specs/testing.md) owns independent checks of
  DEK-erasure deletion, audit-chain tamper/truncation, keystore fallback, and the OQ-6
  remainder (now specified in testing.md §7–§8).
- Replacing GLiNER-small-v2.1, the AEAD algorithm, or adding recovery requires a new
  decision, not a silent implementation change.
- OQ-3, OQ-5, OQ-12, OQ-13, OQ-17, OQ-18 are resolved for v1.

## Related Documentation

- [Spec — architecture](../specs/architecture.md)
- [Spec — design](../specs/design.md)
- [Spec — SRS](../specs/srs.md)
- [Spec — API](../specs/api.md)
- [Spec — testing](../specs/testing.md)
- [Decision 0003 — v1 tech stack](./0003-v1-tech-stack.md)
- [Open questions](../notes/open-questions.md)
- [Work item](../dev-log/0003-architecture-spec.md)
