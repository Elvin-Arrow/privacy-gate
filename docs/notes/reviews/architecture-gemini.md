# Architecture Specification Review: Privacy Gate v1

Reviewer: Gemini (via `agy --effort high`). Date: 2026-08-23.

---

## A. Alignment architecture → SRS

### Missing FR/NFR
- **FR-7.4 / NFR-U1 (Inspectable Audit Trail under Tamper Detection):**
  - *Gap:* §6.3 specifies that upon any hash-chain or anti-truncation head mismatch during unlock, the system executes a "hard failure: the vault does not open... There is no 'open anyway' in v1." While fail-closed security is sound for document decryption, refusing to open the application entirely prevents the user from inspecting the audit log itself to see what was modified, corrupted, or truncated, leaving FR-7.4 (audit inspection) and NFR-U1 (human-readable audit log) uncovered in the exact scenario (tamper detection) for which the audit trail exists.
- **FR-5.1 (Plaintext Source Redaction Export):**
  - *Gap:* §11 states: *"Plain-text sources are re-rendered to PDF the same way; v1 has no plaintext export path."* If plain-text document import (FR-1.1) is supported, forcing all exports into PDF format without a plain-text export path slightly narrows the output flexibility implied by FR-5.1, though it satisfies true-removal export (NFR-S4).

### Contradictions
- **NFR-R1 (Tamper Evidence & Audit Append Reliability) vs. §6.2 / §6.3 (2-Phase Commit Bricking Risk):**
  - *Contradiction:* In §6.2, `audit_head` is updated in the OS keystore after an audit entry is committed to the SQLCipher database. In §6.3, unlock requires a strict match between the DB tail and `audit_head`. Because SQLCipher disk writes and OS keystore IPC are not an atomic transaction, a crash or power cut between the SQLite commit and keystore write leaves the database at entry $N+1$ and the keystore at entry $N$. On next unlock, §6.3 unconditionally treats this as tampering and permanently locks the user out of their vault. This turns a common fault (crash/power loss) into permanent data loss, contradicting the reliability requirements of NFR-R1 and FR-4.1.

### Added behaviour not required by the SRS
- **UK-Specific Pattern Pinning (§10.1):**
  - *Status: Legitimate Architecture Choice.* Pinning `pg-patterns-uk-v1` (NHS, NI, Sort Code) specializes the detector for the target persona (Aisha) without scope creep, provided the regex pack remains decoupled from the core engine.
- **Fail-Closed Permanent Vault Lockout (§6.3):**
  - *Status: Architecture Over-Enforcement.* Hard refusal to mount the database without even a read-only audit diagnostic mode goes beyond SRS requirements and introduces severe availability risks.

---

## B. Alignment architecture → design

### Violations or Inabilities to Implement `design.md`
- **Audit Verification vs. Frontend Audit Log Viewer (`design.md` §2.6):**
  - `design.md` §2.6 requires the Audit Trail component to provide an inspectable log to the UI. Architecture §6.3 prevents the database from opening if an integrity mismatch occurs, making it impossible for the Audit Trail component to serve audit rows to the UI to display the error context.

### Circular Deferrals back to Architecture
- **None.** Deferrals to the API spec (Tauri commands), UI spec (copy/framework), and Testing spec (AC verification) are cleanly directed outward.

### Missing Fill-Ins Explicitly Assigned to Architecture
- **Canonical Serialization Format for Audit Hashing (§6.1 / OQ-3):**
  - *Missing:* §6.1 states: *"Canonical encoding is a versioned, explicit field order (implementation must be bit-stable). Changing it is an architecture amendment."* However, the spec fails to define what that canonical encoding actually is (e.g., Canonical CBOR / RFC 8949, Canonical JSON / RFC 8785, or length-prefixed binary) and what the exact field sequence is. Leaving the hashing serialization format undefined blocks implementation of the hash chain.
- **SQLCipher Raw Keying Configuration (§3.1, §4.2):**
  - *Missing:* §3.1 defines `sqlcipher_key = HKDF-SHA-256(vault_master_key, info="pg-db-v1")`. SQLCipher by default executes 256,000 iterations of PBKDF2 on strings passed to `PRAGMA key`. The architecture fails to specify raw binary keying (`PRAGMA key = "x'...'"` or `sqlite3_key_v2`) and setting `kdf_iter = 0` to prevent redundant key derivation.
- **AAD Format for Envelope Encryption (§3.1):**
  - *Missing:* §3.1 specifies AAD as `artifact_kind || doc_id || version`, but does not specify delimiters or length-prefixing, leaving it vulnerable to canonicalization collisions (e.g., `"doc"` + `"12"` + `"1"` vs. `"do"` + `"c1"` + `"21"`).

---

## C. Alignment architecture ↔ idea

### Breaks of Core Constraints
- **Local-first / No hosting / Key never leaves device / On-device detection / Vault-as-product:**
  - **Clean.** The architecture strictly complies with all core invariants. Accounts are 100% local (§7), detection is fully on-device via in-process ONNX Runtime and regex (§10), export uses true-removal PDF generation (§11), and Cloud AI is isolated to host-mediated HTTPS in Rust with user-supplied keys (§9).

### "Gemma" in Idea Audit Example vs. `pg-hybrid-v1`
- **Status: Legitimate Architecture Choice (No `idea.md` amendment required).**
  - The mention of "Gemma detected..." in `idea.md` was an illustrative audit payload mockup, not an architectural mandate to embed a 2B+ parameter LLM in a local desktop vault.
  - Selecting `pg-hybrid-v1` (combining deterministic pattern matching with INT8 quantized `GLiNER-small-v2.1` via ONNX Runtime) respects the laptop resource budget ($\le 1\text{ GB}$ RAM, sub-second latency) while fulfilling the on-device NER requirement (FR-2.2, NFR-P1).

---

## D. Architecture quality / implementability

### Gaps that Block Implementation
1. **Unspecified Audit Canonical Encoding (§6.1):** Without a deterministic serialization schema (exact byte layout, field order, integer endianness), independent implementations cannot compute stable SHA-256 / HMAC digests.
2. **Missing SQLCipher Keying PRAGMAs (§3.1, §4.2):** Omitting raw binary keying instructions (`PRAGMA key = "x'...'"` and `PRAGMA kdf_iter = 0`) will cause runtime errors or massive unlock latency due to SQLCipher's default PBKDF2 wrapping.

### Threat-Model Holes and Overclaims
1. **Audit Anti-Truncation on Linux Fallback (§3.2, §6.2 vs. §2.4):**
  - In §3.2, if Linux Secret Service is unavailable, `KeystoreItem` (containing `audit_head`) is saved as a file in the app-data directory.
  - An attacker who copies or rolls back the app-data directory gains both `vault.db` and the fallback `KeystoreItem`. The attacker can roll back or truncate both files simultaneously, completely bypassing the anti-truncation check. Claiming anti-truncation protection under the Linux fallback is an overclaim.
2. **Audit Verification "Without Faith" (§6.4):**
  - The spec accurately identifies that HMACs only prove authenticity relative to the vault key held by the TCB, but fails to note that if an attacker obtains the passphrase, they can forge or re-sign modified audit logs at will.

### Cross-Platform Holes
1. **OS Keystore Performance & Write Bottlenecks (§6.2):**
  - Updating the OS keystore (`KeystoreItem.audit_head`) on *every single audit entry* across macOS Keychain, Windows Credential Manager, and Linux Secret Service introduces significant IPC latency (often 50–150ms per call), which will stall UI operations during batch actions.
2. **`mlock` Implementation and Allocator Constraints (§3.1, §5.2):**
  - §5.2 states that `vault_master_key` is `mlock`ed. `mlock` on POSIX and `VirtualLock` on Windows operate on page boundaries (4KB or 16KB on Apple Silicon), not single 32-byte variables. Standard heap allocations cannot be safely `mlock`ed without page-aligned secure memory allocators (e.g., `sodiumoxide::alloc` or `secmem-proc`). Furthermore, unprivileged users on Linux frequently hit strict `RLIMIT_MEMLOCK` limits.
3. **Native ONNX Runtime Distribution (§10.1):**
  - The spec pins the `ort` crate but does not specify linking strategy (dynamic shared library bundling vs. static linking) across target architectures (`x86_64` and `aarch64` for macOS/Windows/Linux).

---

## E. Scope discipline

- **API Leakage:** **Clean.** Avoids concrete Tauri command names, leaving endpoint design and argument schemas to the API spec (§13, §14).
- **UI Leakage:** **Clean.** Defers UI layout, TS frameworks, error dialog phrasing, and interaction models to the UI spec (§5.4, §6.3, §14, §18).
- **Testing-Spec Leakage:** **Clean.** Defers test procedures and AC verification mechanics to the testing spec (§14, §18).
- **Product Decisions:** **Clean.** Correctly defers product-level defaults (e.g., OQ-14 retention defaults, lack of recovery mechanisms) to `idea.md` amendments without making unilateral product choices (§3.3, §18).

---

## F. Deferral health

| Item | Deferred To | Health Status | Assessment |
|---|---|---|---|
| **OQ-4 (remainder)** (Export file naming & metadata fields) | UI / API Specs | **Genuinely Deferred** | Cleanly separated; architecture handles stream sanitization and envelope encryption, while filename defaults and user export dialogs belong in API/UI. |
| **OQ-6 (remainder)** (Independent verification of "no originals left device") | Testing Spec | **Genuinely Deferred** | Architecture provides the core guarantee (never routing `raw_bytes` to network) and emits the boolean flag; test harness verification belongs in testing. |
| **OQ-14** (Default retention period value) | `idea.md` amendment | **Genuinely Deferred** | Properly recognizes this as a product policy decision rather than an architectural parameter. |
| **API Command Surface** | API Spec | **Genuinely Deferred** | Avoids command leakage; defines only interface responsibilities. |
| **UI Framework & Copy** | UI Spec | **Genuinely Deferred** | Avoids frontend framework or copy lock-in. |
| **Audit Canonical Encoding (§6.1)** | *(Dropped / Circular)* | **Unhealthy / Omission** | Declares changes to canonical encoding to be an "architecture amendment" but fails to specify the initial encoding in the architecture spec itself. |

---

## G. Top 5 changes you would make

1. **Fix the Audit-Head 2-Phase Commit Bricking Risk (§6.2, §6.3)**
   - *Severity: Critical (Data Loss Prevention)*
   - *Action:* Define a deterministic crash-recovery protocol on unlock. If the DB hash chain is intact up to entry N+1 and entry N matches `audit_head`, verify the HMAC of entry N+1; if valid, automatically fast-forward `audit_head` to N+1 instead of locking the user out.

2. **Specify the Canonical Byte Serialization for the Audit Chain (§6.1)**
   - *Severity: High (Implementation Blocker)*
   - *Action:* Pin a deterministic, bit-stable serialization standard (e.g., Canonical CBOR via RFC 8949 or length-prefixed binary) and explicitly define the ordered field sequence: `sequence (u64 BE) || timestamp (i64 BE) || event_type (u8) || actor (str) || len(details) || details || prev_entry_hash (32 bytes)`.

3. **Specify SQLCipher Raw Binary Keying and KDF Iteration Bypass (§3.1, §4.2)**
   - *Severity: High (Performance & Interoperability)*
   - *Action:* Mandate that the 32-byte HKDF-derived `sqlcipher_key` is passed via raw binary key syntax (`PRAGMA key = "x'...'"` or `sqlite3_key_v2`) and set `PRAGMA kdf_iter = 0` / raw key mode to bypass SQLCipher’s default PBKDF2 wrapper.

4. **Address Linux Fallback Threat Model & Keystore IPC Latency (§3.2, §6.2)**
   - *Severity: Medium (Security & Performance)*
   - *Action:* Document the degraded threat model for the Linux file fallback (anti-truncation cannot survive directory rollback when stored in app-data), require atomic file replacement (`temp file + fsync + rename`), and batch/debounce keystore head synchronization to prevent UI lag on rapid audit events.

5. **Specify Secure Memory Allocator and Page Handling for `mlock` (§3.1, §5.2)**
   - *Severity: Medium (Portability & Memory Safety)*
   - *Action:* Pin a dedicated page-aligned secure memory crate (e.g., `secmem-proc` or `sodiumoxide::alloc`) for root key management, extend `mlock` protection to transient keys (`wrap_key`, `sqlcipher_key`, `audit_mac_key`), and define graceful fallback if the OS denies `RLIMIT_MEMLOCK` / `VirtualLock`.
