# [0011] W1 — Envelope crypto primitives

- **Status:** Complete
- **Date:** 2026-08-23

## Objective

Deliver the leaf primitives of the key hierarchy in [architecture §3.1](../specs/architecture.md):
HKDF-SHA-256 label derivation, XChaCha20-Poly1305 wrap/unwrap, the length-prefixed AAD v1
wire format, and DEK generate/zeroize. Pure in-memory library code, TDD-first, sized so W2
(account/keystore/session) and W3 (vault schema) can call it without reshaping anything.

Explicitly **not** in this chunk (dev-plan.md §1 chunking discipline, W1 "Do not"): SQLCipher,
the OS keystore, and Argon2id passphrase → `wrap_key` derivation.

## Implementation

### Module layout — `core/src/crypto/`

- **`mod.rs`** — module doc restating where W1 sits in the §3.1 hierarchy diagram, the scope
  fence (no disk / no keystore / no Argon2id / no Tauri command), and the testing.md §5.3
  mutation-gate note. Re-exports the public surface.
- **`aad.rs`** — `ArtifactKind` (codes 1–8, sourced from [data-model.md §6](../specs/data-model.md);
  no second kind list) and `Aad` with `encode()` / `decode()` for the AAD v1 wire format:
  ```
  u8 aad_version = 1 | u8 artifact_kind | u16be doc_id_len | doc_id UTF-8 | u32be format_version
  ```
  Constructors: `Aad::for_document(kind, doc_id, format_version)`, `Aad::global(kind, format_version)`
  for the non-document-scoped kinds (4, 5, 6), and a fallible `try_new` that rejects a
  `doc_id` longer than `u16::MAX` rather than truncating it.
- **`aead.rs`** — `wrap` / `unwrap` over `XChaCha20Poly1305`, plus `WrappedBlob { nonce, ciphertext }`,
  which maps 1:1 onto the `artifact.nonce` / `artifact.ciphertext` columns of
  [data-model.md §7](../specs/data-model.md). Fresh 24-byte nonce per `wrap` call from the OS
  CSPRNG. `WrappedBlob`'s `Debug` prints lengths only.
- **`kdf.rs`** — `derive(ikm: &[u8; 32], info: &str) -> [u8; 32]`, HKDF-SHA-256 with the fixed
  salt `privacy-gate-hkdf-v1`. Label-agnostic on purpose: W2 will call it with `pg-db-v1` for
  `sqlcipher_key`, W5 with `pg-audit-mac-v1` for `audit_mac_key`. W1 owns the primitive, not
  the call sites.
- **`dek.rs`** — `Dek`, a 256-bit key with `#[derive(Zeroize, ZeroizeOnDrop)]`, `generate()` /
  `try_generate()` (OS CSPRNG, never a weaker fallback), `from_bytes`, `as_bytes`,
  constant-time `ct_eq_dek` and `is_zeroized` (`subtle`), and a `Debug` that renders
  `Dek(redacted)`. `Clone` is deliberately not implemented — every copy is another place the
  bytes have to be destroyed before FR-4.6 cryptographic erasure is honest.
- **`error.rs`** — `CryptoError { MalformedAad, MalformedBlob, Decrypt, Rng }`. Deliberately
  coarse: `Decrypt` does not distinguish wrong key from wrong AAD from bad tag, because
  splitting them would be a decryption oracle.

`core/src/lib.rs` now exposes `pub mod crypto;` and the W0 `add`/`it_works` placeholder is gone.

### Dependencies (added inside the container, per CONTRIBUTING.md)

`chacha20poly1305 0.11`, `hkdf 0.13`, `sha2 0.11`, `subtle 2.6`, `zeroize 1.9` (`derive`
feature), `getrandom 0.4`; `hmac 0.13` + `sha2 0.11` as dev-dependencies only. All added via
`docker compose run --rm dev … cargo add`; nothing installed on the host. `argon2`, `hmac`
(library), `memsec`, `rusqlite`/SQLCipher and `keyring` are intentionally still absent — they
belong to W2/W3/W5.

### Tests — `core/tests/crypto_w1.rs` (35 tests)

Written and compiled red before any of `core/src/crypto/` existed. Every test name or doc
comment cites its clause (`architecture_3_1`, `data_model_6`, `data_model_7`, `testing_5_3`).

- **AAD:** exact byte-for-byte wire format for a document-scoped and a global AAD; kind codes
  match data-model §6; decode round-trip; injectivity swept over 8 kinds × 5 doc_ids ×
  4 format_versions; prefix-freeness (below); fail-closed decode against every truncation
  prefix, trailing garbage, wrong `aad_version`, unknown `artifact_kind`, an overrunning
  `doc_id_len`, and non-UTF-8 `doc_id` — all `Err`, none panicking.
- **AEAD:** round-trip; 24-byte nonce; fresh nonce per call; ciphertext = plaintext + 16-byte
  tag and is not the plaintext; empty plaintext; wrapping a DEK under a kind-7 AAD (the
  W2/W3 call shape); wrong AAD (four different tuple mutations), wrong key, flipped
  ciphertext bit, flipped tag bit, and flipped nonce bit all fail with `Decrypt`; malformed
  blobs (23-byte nonce, 25-byte nonce, sub-tag-length ciphertext) fail with `MalformedBlob`
  without panicking.
- **HKDF:** salt literal; 32-byte output; determinism; `pg-db-v1` and `pg-audit-mac-v1` yield
  distinct subkeys, both distinct from the IKM; different IKM yields different output; and two
  known-answer tests against a longhand RFC 5869 extract-and-expand written out in the test
  file, so the assertion does not simply mirror the production call.
- **DEK:** 256 bits; two generations differ and neither is all-zero; `.zeroize()` clears the
  bytes and `is_zeroized()` reports it; a zeroized DEK can no longer unwrap its own ciphertext
  ("zeroize leaves key unusable"); constant-time equality; `Debug` leaks no key bytes.

Per the task brief, the zeroize contract is tested through the explicit `.zeroize()` call
rather than by reading memory through a raw pointer after drop, which would be UB-adjacent
and not an honest test.

## Problems Encountered

1. **AAD v1 salt length disagrees with the spec prose.** [architecture §3.1](../specs/architecture.md)
   says "Salt = the 19-byte ASCII `privacy-gate-hkdf-v1`", but that string is **20** bytes.
   Resolved in favour of the literal string (it is the interoperability-relevant value; the
   byte count is a typo). `HKDF_SALT` is declared `&[u8; 20]` and a test pins it. The spec
   prose should be corrected to "20-byte" in a later editorial pass — flagging rather than
   editing a spec from an implementation chunk.

2. **RustCrypto major-version churn.** `cargo add` resolved to the new `chacha20poly1305 0.11`
   / `hkdf 0.13` / `sha2 0.11` / `hmac 0.13` line, which is built on `hybrid-array` and
   `crypto-common 0.2` rather than `generic-array` / `digest 0.10`. The API differs from the
   long-standing 0.10/0.12 releases: `Mac::new_from_slice` moved to `digest::KeyInit`, and
   nonce/key types are `Array<u8, U24>` rather than `GenericArray`. Verified the actual APIs
   by reading the vendored crate sources inside the container instead of guessing.

3. **`rand` was not the right dependency.** `cargo add rand` pulled `rand 0.10`, whose OS-RNG
   surface is now re-exported from `getrandom`. For key material there is no reason to route
   through a userspace PRNG at all, so `rand` was dropped and `getrandom 0.4` (`getrandom::fill`)
   is used directly for both the DEK and the AEAD nonce — an explicit OS CSPRNG call with no
   seeded state to reason about.

4. **The original "concatenation collision" test was not rigorous.** The first draft asserted
   that two AADs whose naive concatenation collides do not collide under AAD v1 — but with
   `doc_id` as the only variable-length field followed by a fixed-width `format_version`,
   naive concatenation is already injective, so the assertion held vacuously. Replaced with the
   property that actually distinguishes the two encodings and that the length prefix genuinely
   buys: **prefix-freeness**. Naive `kind || doc_id || format_version` for
   `doc_id = "a", format_version = 0x62636401` renders `[1, k, 'a', 'b', 'c', 'd', 0x01]`,
   which is a proper prefix of the naive rendering of `doc_id = "abcd\x01ef"`; with the u16be
   length prefix the declared length fixes the total record size, so no encoding can be a
   proper prefix of another. Two tests cover it, including an exhaustive sweep over doc_ids
   chosen to be prefixes of one another.

## Resolution

- `make test` green: 35/35 in `core/tests/crypto_w1.rs`, plus the `pg-core` and `privacy-gate`
  unit/doc-test targets (0 tests each). No new build warnings; the only warning is W0's
  pre-existing "profiles for the non root package will be ignored".
- Manual mutation spot-check on the gated path: dropping
  `out.extend_from_slice(&doc_id_len.to_be_bytes())` from `Aad::encode` was killed by three
  tests (`aad_v1_encodes_exact_wire_format_architecture_3_1`,
  `aad_v1_non_document_scoped_has_zero_length_doc_id_architecture_3_1`,
  `aad_decode_round_trips`). The mutation was reverted; the full `cargo mutants` gate is W38.
- Scope held: no disk I/O, no SQLCipher, no OS keystore, no Argon2id or passphrase handling,
  no Tauri command, no new command name. `core/src/crypto/` is an internal library consumed
  by W2/W3.
- `core/src/crypto/` is the module to list for the W38 mutation gate under testing.md §5.3
  "Envelope AAD length-prefixing" and the DEK destroy helpers.

Next: W2 — account, keystore, session (`get_session_state`, `create_account`, `unlock`,
`lock`, `change_passphrase`, `get_account`), which is where Argon2id and the `keyring`
dependency land.

## Related Documentation

- [Development Plan — W1 specification](../dev-plan.md#w1--envelope-crypto-primitives)
- [Spec — Architecture §3.1 (key hierarchy, AEAD, AAD v1, HKDF)](../specs/architecture.md)
- [Spec — Data model §6 (artifact kinds), §7 (nonce/ciphertext columns)](../specs/data-model.md)
- [Spec — Testing §5.3 (gated modules)](../specs/testing.md)
- [Decision 0004 — v1 architecture (XChaCha20-Poly1305 over AES-GCM)](../decisions/0004-v1-architecture.md)
- [Decision 0006 — TDD and mutation testing](../decisions/0006-tdd-and-mutation-testing.md)
- [Dev log 0009 — W0 repo skeleton](./0009-w0-repo-skeleton.md)
