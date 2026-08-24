//! W1 — Envelope crypto primitives.
//!
//! Spec sources:
//! - `docs/specs/architecture.md` §3.1 (key hierarchy, AEAD, AAD v1 wire format, HKDF)
//! - `docs/specs/data-model.md` §6 (artifact kind codes), §7 (nonce BLOB is 24 bytes)
//! - `docs/specs/testing.md` §5.3 (gated module: "Envelope AAD length-prefixing",
//!   "DEK destroy helpers" — S = 1.00, no unexplained survivors)
//! - `docs/dev-plan.md` W1 ("Tests first: unit tests for wrap/unwrap; wrong AAD fails;
//!   zeroize leaves key unusable; length-prefix mutants would break unwrap")
//!
//! Out of W1 scope and deliberately absent here: disk I/O, SQLCipher, OS keystore,
//! Argon2id passphrase→wrap_key derivation (all W2).

use pg_core::crypto::{Aad, ArtifactKind, CryptoError, Dek, WrappedBlob, HKDF_SALT};
use zeroize::Zeroize;

const KEY_A: [u8; 32] = [0x11; 32];
const KEY_B: [u8; 32] = [0x22; 32];

// ---------------------------------------------------------------------------
// AAD v1 wire format — architecture.md §3.1
//
//   u8    aad_version = 1
//   u8    artifact_kind
//   u16be doc_id_len
//   doc_id UTF-8 bytes        (len 0 if not document-scoped)
//   u32be format_version
// ---------------------------------------------------------------------------

#[test]
fn aad_v1_encodes_exact_wire_format_architecture_3_1() {
    let aad = Aad::for_document(ArtifactKind::Approved, "abc", 1);
    assert_eq!(
        aad.encode(),
        vec![
            0x01, // aad_version
            0x01, // artifact_kind = approved (data-model.md §6)
            0x00, 0x03, // u16be doc_id_len
            b'a', b'b', b'c', // doc_id UTF-8
            0x00, 0x00, 0x00, 0x01, // u32be format_version
        ]
    );
}

#[test]
fn aad_v1_non_document_scoped_has_zero_length_doc_id_architecture_3_1() {
    let aad = Aad::global(ArtifactKind::Config, 1);
    assert_eq!(
        aad.encode(),
        vec![0x01, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]
    );
}

/// testing.md §5.3 gated property: the encoding is **self-delimiting**
/// (prefix-free). No encoded AAD is ever a proper prefix of another encoded
/// AAD, so the record stays unambiguous when it is embedded in, or extended by,
/// other bytes — e.g. if a v2 field is ever appended after `format_version`.
///
/// Naive `kind || doc_id || format_version` does *not* have this property: with
/// `doc_id = "a"` and `format_version = 0x62636401` it renders
/// `[1, k, 'a', 'b', 'c', 'd', 0x01]`, which is a proper prefix of the naive
/// rendering of `doc_id = "abcd\x01ef"`. The u16be length prefix is what rules
/// that out, because the declared length fixes the total record size.
///
/// A mutant that drops the length prefix, writes a constant, or writes the
/// wrong width makes the pair below overlap.
#[test]
fn aad_encoding_is_prefix_free_testing_5_3() {
    let short = Aad::for_document(ArtifactKind::Approved, "a", 0x6263_6401);
    let long = Aad::for_document(ArtifactKind::Approved, "abcd\u{1}ef", 7);

    let a = short.encode();
    let b = long.encode();

    // Under naive concatenation these two would overlap; under AAD v1 they must
    // not, in either direction.
    assert!(!b.starts_with(&a), "encode(short) must not prefix encode(long)");
    assert!(!a.starts_with(&b), "encode(long) must not prefix encode(short)");

    // The declared u16be length is what makes the record self-delimiting: it
    // must equal the actual doc_id byte length.
    assert_eq!(u16::from_be_bytes([a[2], a[3]]), 1);
    assert_eq!(u16::from_be_bytes([b[2], b[3]]), "abcd\u{1}ef".len() as u16);
    assert_eq!(a.len(), 8 + 1);
    assert_eq!(b.len(), 8 + "abcd\u{1}ef".len());
}

/// The same property, exhaustively over a set of doc_ids chosen so that several
/// are prefixes of each other and several embed format_version-looking bytes.
#[test]
fn no_encoded_aad_is_a_prefix_of_another_testing_5_3() {
    let doc_ids = [
        "",
        "a",
        "ab",
        "abc",
        "a\u{0}",
        "a\u{0}\u{0}\u{0}\u{1}",
        "a\u{0}\u{0}\u{0}\u{1}b",
    ];
    let encodings: Vec<Vec<u8>> = doc_ids
        .iter()
        .map(|d| Aad::for_document(ArtifactKind::Approved, d, 1).encode())
        .collect();

    for (i, a) in encodings.iter().enumerate() {
        for (j, b) in encodings.iter().enumerate() {
            if i == j {
                continue;
            }
            assert!(
                !b.starts_with(a),
                "encode({:?}) is a prefix of encode({:?})",
                doc_ids[i],
                doc_ids[j]
            );
        }
    }
}

/// Distinct (kind, doc_id, format_version) tuples must never share an encoding.
#[test]
fn aad_encoding_is_injective_over_tuple_space_testing_5_3() {
    let kinds = [
        ArtifactKind::Approved,
        ArtifactKind::Original,
        ArtifactKind::Variant,
        ArtifactKind::Config,
        ArtifactKind::PluginSecret,
        ArtifactKind::WrappedMaster,
        ArtifactKind::WrappedDek,
        ArtifactKind::DocumentMeta,
    ];
    let doc_ids = ["", "a", "ab", "abc", "a\u{0}b"];
    let format_versions = [0u32, 1, 2, u32::MAX];

    let mut seen: Vec<Vec<u8>> = Vec::new();
    for k in kinds {
        for d in doc_ids {
            for fv in format_versions {
                let enc = Aad::new(k, d, fv).encode();
                assert!(!seen.contains(&enc), "collision for ({k:?}, {d:?}, {fv})");
                seen.push(enc);
            }
        }
    }
}

#[test]
fn aad_decode_round_trips() {
    let aad = Aad::for_document(ArtifactKind::DocumentMeta, "doc-42", 1);
    let decoded = Aad::decode(&aad.encode()).expect("valid AAD decodes");
    assert_eq!(decoded, aad);
}

/// Fail closed, never panic, never silently accept (testing.md §5.3).
#[test]
fn aad_decode_rejects_truncated_and_malformed_input_fail_closed_testing_5_3() {
    let good = Aad::for_document(ArtifactKind::Approved, "abc", 1).encode();

    // Every strict prefix is malformed.
    for n in 0..good.len() {
        assert!(
            matches!(Aad::decode(&good[..n]), Err(CryptoError::MalformedAad)),
            "truncation at {n} must be rejected"
        );
    }

    // Trailing garbage is malformed (no silent accept of extra bytes).
    let mut long = good.clone();
    long.push(0xff);
    assert!(matches!(Aad::decode(&long), Err(CryptoError::MalformedAad)));

    // Wrong aad_version.
    let mut wrong_version = good.clone();
    wrong_version[0] = 2;
    assert!(matches!(
        Aad::decode(&wrong_version),
        Err(CryptoError::MalformedAad)
    ));

    // Unknown artifact_kind (data-model.md §6 defines 1..=8 only).
    let mut wrong_kind = good.clone();
    wrong_kind[1] = 9;
    assert!(matches!(
        Aad::decode(&wrong_kind),
        Err(CryptoError::MalformedAad)
    ));

    // doc_id_len that overruns the buffer must fail, not panic.
    let mut overrun = good.clone();
    overrun[2] = 0xff;
    overrun[3] = 0xff;
    assert!(matches!(
        Aad::decode(&overrun),
        Err(CryptoError::MalformedAad)
    ));

    // doc_id that is not valid UTF-8 must fail.
    let mut bad_utf8 = good.clone();
    bad_utf8[4] = 0xff;
    assert!(matches!(
        Aad::decode(&bad_utf8),
        Err(CryptoError::MalformedAad)
    ));
}

#[test]
fn aad_rejects_doc_id_longer_than_u16() {
    let too_long = "x".repeat(u16::MAX as usize + 1);
    assert!(matches!(
        Aad::try_new(ArtifactKind::Approved, &too_long, 1),
        Err(CryptoError::MalformedAad)
    ));
}

#[test]
fn artifact_kind_codes_match_data_model_6() {
    assert_eq!(ArtifactKind::Approved as u8, 1);
    assert_eq!(ArtifactKind::Original as u8, 2);
    assert_eq!(ArtifactKind::Variant as u8, 3);
    assert_eq!(ArtifactKind::Config as u8, 4);
    assert_eq!(ArtifactKind::PluginSecret as u8, 5);
    assert_eq!(ArtifactKind::WrappedMaster as u8, 6);
    assert_eq!(ArtifactKind::WrappedDek as u8, 7);
    assert_eq!(ArtifactKind::DocumentMeta as u8, 8);
}

// ---------------------------------------------------------------------------
// AEAD wrap/unwrap — XChaCha20-Poly1305, architecture.md §3.1
// ---------------------------------------------------------------------------

#[test]
fn wrap_then_unwrap_round_trips_architecture_3_1() {
    let aad = Aad::for_document(ArtifactKind::Approved, "doc-1", 1);
    let plaintext = b"the quick brown fox".to_vec();

    let blob = pg_core::crypto::wrap(&KEY_A, &plaintext, &aad).expect("wrap succeeds");
    let out = pg_core::crypto::unwrap(&KEY_A, &blob, &aad).expect("unwrap succeeds");

    assert_eq!(out, plaintext);
}

/// data-model.md §7: `nonce BLOB NOT NULL -- 24 bytes`.
#[test]
fn wrap_uses_24_byte_nonce_data_model_7() {
    let aad = Aad::global(ArtifactKind::Config, 1);
    let blob = pg_core::crypto::wrap(&KEY_A, b"x", &aad).unwrap();
    assert_eq!(blob.nonce.len(), 24);
}

/// Random nonce per wrap call (architecture.md §3.1). Same key + same plaintext
/// must not produce the same ciphertext.
#[test]
fn wrap_generates_a_fresh_random_nonce_per_call_architecture_3_1() {
    let aad = Aad::global(ArtifactKind::Config, 1);
    let a = pg_core::crypto::wrap(&KEY_A, b"same plaintext", &aad).unwrap();
    let b = pg_core::crypto::wrap(&KEY_A, b"same plaintext", &aad).unwrap();
    assert_ne!(a.nonce, b.nonce);
    assert_ne!(a.ciphertext, b.ciphertext);
}

#[test]
fn ciphertext_carries_the_poly1305_tag_and_is_not_plaintext() {
    let aad = Aad::global(ArtifactKind::Config, 1);
    let plaintext = b"secret material";
    let blob = pg_core::crypto::wrap(&KEY_A, plaintext, &aad).unwrap();
    assert_eq!(blob.ciphertext.len(), plaintext.len() + 16);
    assert_ne!(&blob.ciphertext[..plaintext.len()], &plaintext[..]);
}

/// dev-plan W1: "wrong AAD fails".
#[test]
fn unwrap_with_wrong_aad_fails_testing_5_3() {
    let bound = Aad::for_document(ArtifactKind::Approved, "doc-1", 1);
    let blob = pg_core::crypto::wrap(&KEY_A, b"payload", &bound).unwrap();

    for wrong in [
        Aad::for_document(ArtifactKind::Original, "doc-1", 1), // kind differs
        Aad::for_document(ArtifactKind::Approved, "doc-2", 1), // doc_id differs
        Aad::for_document(ArtifactKind::Approved, "doc-1", 2), // format_version differs
        Aad::global(ArtifactKind::Approved, 1),                // scope differs
    ] {
        assert!(
            matches!(
                pg_core::crypto::unwrap(&KEY_A, &blob, &wrong),
                Err(CryptoError::Decrypt)
            ),
            "unwrap must fail for AAD {wrong:?}"
        );
    }
}

#[test]
fn unwrap_with_wrong_key_fails() {
    let aad = Aad::global(ArtifactKind::Config, 1);
    let blob = pg_core::crypto::wrap(&KEY_A, b"payload", &aad).unwrap();
    assert!(matches!(
        pg_core::crypto::unwrap(&KEY_B, &blob, &aad),
        Err(CryptoError::Decrypt)
    ));
}

#[test]
fn unwrap_with_flipped_ciphertext_bit_fails() {
    let aad = Aad::global(ArtifactKind::Config, 1);
    let mut blob = pg_core::crypto::wrap(&KEY_A, b"payload", &aad).unwrap();
    blob.ciphertext[0] ^= 0x01;
    assert!(matches!(
        pg_core::crypto::unwrap(&KEY_A, &blob, &aad),
        Err(CryptoError::Decrypt)
    ));
}

#[test]
fn unwrap_with_flipped_tag_bit_fails() {
    let aad = Aad::global(ArtifactKind::Config, 1);
    let mut blob = pg_core::crypto::wrap(&KEY_A, b"payload", &aad).unwrap();
    let last = blob.ciphertext.len() - 1;
    blob.ciphertext[last] ^= 0x80;
    assert!(matches!(
        pg_core::crypto::unwrap(&KEY_A, &blob, &aad),
        Err(CryptoError::Decrypt)
    ));
}

#[test]
fn unwrap_with_flipped_nonce_bit_fails() {
    let aad = Aad::global(ArtifactKind::Config, 1);
    let mut blob = pg_core::crypto::wrap(&KEY_A, b"payload", &aad).unwrap();
    blob.nonce[0] ^= 0x01;
    assert!(matches!(
        pg_core::crypto::unwrap(&KEY_A, &blob, &aad),
        Err(CryptoError::Decrypt)
    ));
}

/// Fail closed, never panic, on structurally invalid blobs.
#[test]
fn unwrap_rejects_malformed_blob_without_panicking() {
    let aad = Aad::global(ArtifactKind::Config, 1);

    let short_nonce = WrappedBlob {
        nonce: vec![0u8; 23],
        ciphertext: vec![0u8; 32],
    };
    assert!(matches!(
        pg_core::crypto::unwrap(&KEY_A, &short_nonce, &aad),
        Err(CryptoError::MalformedBlob)
    ));

    let long_nonce = WrappedBlob {
        nonce: vec![0u8; 25],
        ciphertext: vec![0u8; 32],
    };
    assert!(matches!(
        pg_core::crypto::unwrap(&KEY_A, &long_nonce, &aad),
        Err(CryptoError::MalformedBlob)
    ));

    // Ciphertext shorter than the Poly1305 tag cannot be authentic.
    let stub = WrappedBlob {
        nonce: vec![0u8; 24],
        ciphertext: vec![0u8; 15],
    };
    assert!(matches!(
        pg_core::crypto::unwrap(&KEY_A, &stub, &aad),
        Err(CryptoError::MalformedBlob)
    ));
}

#[test]
fn empty_plaintext_round_trips() {
    let aad = Aad::global(ArtifactKind::Config, 1);
    let blob = pg_core::crypto::wrap(&KEY_A, b"", &aad).unwrap();
    assert_eq!(pg_core::crypto::unwrap(&KEY_A, &blob, &aad).unwrap(), Vec::<u8>::new());
}

/// A DEK is itself wrapped under the master key with AAD kind 7
/// (data-model.md §6: `wrapped_dek`). Exercises the W2/W3 call shape.
#[test]
fn dek_wraps_and_unwraps_under_kind_7_aad_data_model_6() {
    let master = [0x33u8; 32];
    let dek = Dek::generate();
    let aad = Aad::for_document(ArtifactKind::WrappedDek, "doc-9", 1);

    let wrapped = pg_core::crypto::wrap(&master, dek.as_bytes(), &aad).unwrap();
    let recovered = pg_core::crypto::unwrap(&master, &wrapped, &aad).unwrap();

    assert_eq!(recovered.as_slice(), dek.as_bytes());
    assert_eq!(recovered.len(), 32);
}

// ---------------------------------------------------------------------------
// HKDF-SHA-256 — architecture.md §3.1
// ---------------------------------------------------------------------------

#[test]
fn hkdf_salt_is_the_literal_ascii_label_architecture_3_1() {
    assert_eq!(HKDF_SALT, b"privacy-gate-hkdf-v1");
}

#[test]
fn hkdf_derives_32_bytes_architecture_3_1() {
    let ikm = [0x44u8; 32];
    let out = pg_core::crypto::derive(&ikm, "pg-db-v1");
    assert_eq!(out.len(), 32);
}

#[test]
fn hkdf_is_deterministic_for_the_same_ikm_and_label() {
    let ikm = [0x44u8; 32];
    assert_eq!(
        pg_core::crypto::derive(&ikm, "pg-db-v1"),
        pg_core::crypto::derive(&ikm, "pg-db-v1")
    );
}

/// The two v1 labels must produce independent subkeys (architecture §3.1:
/// `sqlcipher_key` vs `audit_mac_key`).
#[test]
fn hkdf_labels_yield_distinct_subkeys_architecture_3_1() {
    let ikm = [0x44u8; 32];
    let db = pg_core::crypto::derive(&ikm, "pg-db-v1");
    let mac = pg_core::crypto::derive(&ikm, "pg-audit-mac-v1");
    assert_ne!(db, mac);
    assert_ne!(db, ikm);
    assert_ne!(mac, ikm);
}

#[test]
fn hkdf_different_ikm_yields_different_subkeys() {
    assert_ne!(
        pg_core::crypto::derive(&[0x01u8; 32], "pg-db-v1"),
        pg_core::crypto::derive(&[0x02u8; 32], "pg-db-v1")
    );
}

/// Known-answer test: RFC 5869 HKDF-SHA-256 with this project's fixed salt and
/// the `pg-db-v1` info label. Pins the salt, the hash, and the info string so a
/// mutant that swaps any of them is killed.
#[test]
fn hkdf_known_answer_for_pg_db_v1_label_architecture_3_1() {
    let ikm = [0u8; 32];
    let got = pg_core::crypto::derive(&ikm, "pg-db-v1");
    let expected = hkdf_reference(HKDF_SALT, &ikm, b"pg-db-v1");
    assert_eq!(got, expected);
}

#[test]
fn hkdf_known_answer_for_pg_audit_mac_v1_label_architecture_3_1() {
    let ikm = [0u8; 32];
    let got = pg_core::crypto::derive(&ikm, "pg-audit-mac-v1");
    let expected = hkdf_reference(HKDF_SALT, &ikm, b"pg-audit-mac-v1");
    assert_eq!(got, expected);
}

/// Independent RFC 5869 extract-and-expand, written out longhand so the test
/// does not simply mirror the production call.
fn hkdf_reference(salt: &[u8], ikm: &[u8], info: &[u8]) -> [u8; 32] {
    use hmac::digest::KeyInit;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    type H = Hmac<Sha256>;

    // Extract
    let mut e = <H as KeyInit>::new_from_slice(salt).unwrap();
    e.update(ikm);
    let prk = e.finalize().into_bytes();

    // Expand, L = 32 => a single T(1) block
    let mut x = <H as KeyInit>::new_from_slice(&prk).unwrap();
    x.update(info);
    x.update(&[0x01]);
    let okm = x.finalize().into_bytes();

    let mut out = [0u8; 32];
    out.copy_from_slice(&okm[..32]);
    out
}

// ---------------------------------------------------------------------------
// DEK lifecycle — architecture.md §3.1 / §4.3, testing.md §5.3 destroy helpers
// ---------------------------------------------------------------------------

#[test]
fn dek_is_256_bits_architecture_3_1() {
    assert_eq!(Dek::generate().as_bytes().len(), 32);
}

#[test]
fn dek_generate_is_csprng_backed_and_not_all_zero_architecture_3_1() {
    let a = Dek::generate();
    let b = Dek::generate();
    assert_ne!(a.as_bytes(), b.as_bytes());
    assert_ne!(a.as_bytes(), &[0u8; 32]);
    assert_ne!(b.as_bytes(), &[0u8; 32]);
}

/// dev-plan W1: "zeroize leaves key unusable". After `.zeroize()` the bytes are
/// gone, so a wrap performed with the zeroized key no longer matches the
/// original key's ciphertext and the original key can no longer be recovered.
#[test]
fn dek_zeroize_clears_key_material_testing_5_3() {
    let mut dek = Dek::generate();
    let before = dek.as_bytes().to_vec();

    dek.zeroize();

    assert_ne!(dek.as_bytes(), before.as_slice());
    assert_eq!(dek.as_bytes(), &[0u8; 32]);
    assert!(dek.is_zeroized());
}

/// A zeroized DEK must not still decrypt what the live DEK encrypted.
#[test]
fn zeroized_dek_can_no_longer_unwrap_its_own_ciphertext_testing_5_3() {
    let aad = Aad::for_document(ArtifactKind::Approved, "doc-1", 1);
    let mut dek = Dek::generate();
    let blob = pg_core::crypto::wrap(dek.as_bytes(), b"payload", &aad).unwrap();

    dek.zeroize();

    assert!(matches!(
        pg_core::crypto::unwrap(dek.as_bytes(), &blob, &aad),
        Err(CryptoError::Decrypt)
    ));
}

#[test]
fn dek_from_bytes_and_generate_agree_on_representation() {
    let raw = [0x5au8; 32];
    let dek = Dek::from_bytes(raw);
    assert_eq!(dek.as_bytes(), &raw);
}

/// Constant-time equality (`subtle`) — architecture.md §3.1 library list.
#[test]
fn dek_equality_is_constant_time() {
    let a = Dek::from_bytes([0x01; 32]);
    let b = Dek::from_bytes([0x01; 32]);
    let c = Dek::from_bytes([0x02; 32]);
    assert!(a.ct_eq_dek(&b));
    assert!(!a.ct_eq_dek(&c));
}

/// Key material must never leak through `Debug` (architecture.md §5 —
/// no plaintext key bytes in logs).
#[test]
fn dek_debug_does_not_leak_key_bytes() {
    let dek = Dek::from_bytes([0xab; 32]);
    let rendered = format!("{dek:?}");
    assert!(!rendered.contains("ab"));
    assert!(!rendered.contains("171"));
    assert!(rendered.contains("Dek"));
    assert!(rendered.contains("redacted"));
}
