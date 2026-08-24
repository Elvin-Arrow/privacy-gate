//! AAD v1 — length-prefixed additional authenticated data.
//!
//! `docs/specs/architecture.md` §3.1:
//!
//! ```text
//! AAD v1 =
//!   u8    aad_version = 1
//!   u8    artifact_kind     // ArtifactKind; codes only in data-model.md §6
//!   u16be doc_id_len
//!   doc_id UTF-8 bytes      // len 0 if not document-scoped
//!   u32be format_version    // artifact schema version, not Unix time
//! ```
//!
//! The length prefix is the whole point: it makes the encoding **injective**, so
//! no two distinct `(artifact_kind, doc_id, format_version)` tuples can ever
//! produce the same AAD bytes. Without it, `("ab", …)` and `("a", "b"…)` would
//! collide under naive concatenation and an attacker could substitute one
//! artifact's ciphertext for another's. This is why `testing.md` §5.3 gates this
//! module at S = 1.00.

use super::error::CryptoError;

/// `aad_version` for the v1 wire format (architecture §3.1).
const AAD_VERSION: u8 = 1;

/// Fixed-size portion of the record: version + kind + u16 length + u32 format
/// version. Anything shorter than this cannot be a valid record.
const AAD_FIXED_LEN: usize = 1 + 1 + 2 + 4;

/// Artifact kind codes. **`data-model.md` §6 is the single source of these
/// codes** — do not fork a second kind list (data-model.md §6, C-DM-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ArtifactKind {
    /// Approved (redacted) version of a document. Document-scoped.
    Approved = 1,
    /// Retained original document bytes. Document-scoped.
    Original = 2,
    /// A named variant of an approved document. Document-scoped.
    Variant = 3,
    /// Global config blob. Not document-scoped.
    Config = 4,
    /// Plugin secret (v1: Cloud AI). Not document-scoped.
    PluginSecret = 5,
    /// Wrapped `vault_master_key`. Lives in the OS keystore, never in SQL.
    WrappedMaster = 6,
    /// AAD kind used when wrapping a per-artifact DEK. Not a SQL row kind.
    WrappedDek = 7,
    /// Catalog metadata for a document (FR-4.3). Document-scoped.
    DocumentMeta = 8,
}

impl ArtifactKind {
    /// Wire code for this kind.
    #[must_use]
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Parse a wire code. Unknown codes are rejected — an artifact kind the
    /// build does not understand must never be treated as authentic.
    fn from_code(code: u8) -> Result<Self, CryptoError> {
        match code {
            1 => Ok(ArtifactKind::Approved),
            2 => Ok(ArtifactKind::Original),
            3 => Ok(ArtifactKind::Variant),
            4 => Ok(ArtifactKind::Config),
            5 => Ok(ArtifactKind::PluginSecret),
            6 => Ok(ArtifactKind::WrappedMaster),
            7 => Ok(ArtifactKind::WrappedDek),
            8 => Ok(ArtifactKind::DocumentMeta),
            _ => Err(CryptoError::MalformedAad),
        }
    }
}

/// A parsed AAD v1 record.
///
/// Construct one per wrap/unwrap call and pass the *same* logical value to both
/// sides; any difference makes `unwrap` fail (that is the binding property).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aad {
    kind: ArtifactKind,
    doc_id: String,
    format_version: u32,
}

impl Aad {
    /// Build an AAD record.
    ///
    /// # Panics
    /// If `doc_id` is longer than [`u16::MAX`] bytes, which cannot be expressed
    /// in the v1 wire format. Document ids are UUIDs (data-model.md §5.1), so
    /// this is a programming error, not attacker-reachable input. Use
    /// [`Aad::try_new`] when the length is not statically known.
    #[must_use]
    pub fn new(kind: ArtifactKind, doc_id: &str, format_version: u32) -> Self {
        Self::try_new(kind, doc_id, format_version)
            .expect("doc_id must fit in u16 bytes for AAD v1")
    }

    /// Fallible constructor: rejects a `doc_id` that the u16 length prefix
    /// cannot represent, rather than truncating it (a truncating encoder would
    /// reintroduce collisions).
    pub fn try_new(
        kind: ArtifactKind,
        doc_id: &str,
        format_version: u32,
    ) -> Result<Self, CryptoError> {
        if doc_id.len() > u16::MAX as usize {
            return Err(CryptoError::MalformedAad);
        }
        Ok(Self {
            kind,
            doc_id: doc_id.to_owned(),
            format_version,
        })
    }

    /// Document-scoped AAD (kinds 1, 2, 3, 7, 8 — data-model.md §6).
    #[must_use]
    pub fn for_document(kind: ArtifactKind, doc_id: &str, format_version: u32) -> Self {
        Self::new(kind, doc_id, format_version)
    }

    /// Non-document-scoped AAD (kinds 4, 5, 6 — `doc_id_len` is 0).
    #[must_use]
    pub fn global(kind: ArtifactKind, format_version: u32) -> Self {
        Self::new(kind, "", format_version)
    }

    /// The artifact kind bound by this AAD.
    #[must_use]
    pub fn kind(&self) -> ArtifactKind {
        self.kind
    }

    /// The document id bound by this AAD (empty if not document-scoped).
    #[must_use]
    pub fn doc_id(&self) -> &str {
        &self.doc_id
    }

    /// The artifact schema version bound by this AAD.
    #[must_use]
    pub fn format_version(&self) -> u32 {
        self.format_version
    }

    /// Serialize to the AAD v1 wire format (architecture §3.1).
    ///
    /// Field order and widths are load-bearing: they are what the ciphertext is
    /// authenticated against, and what makes the encoding injective.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let doc_id = self.doc_id.as_bytes();
        debug_assert!(doc_id.len() <= u16::MAX as usize);
        let doc_id_len = doc_id.len() as u16;

        let mut out = Vec::with_capacity(AAD_FIXED_LEN + doc_id.len());
        out.push(AAD_VERSION);
        out.push(self.kind.code());
        out.extend_from_slice(&doc_id_len.to_be_bytes());
        out.extend_from_slice(doc_id);
        out.extend_from_slice(&self.format_version.to_be_bytes());
        out
    }

    /// Parse the AAD v1 wire format. Fails closed on anything that is not an
    /// exact, well-formed record — truncation, trailing bytes, an unknown
    /// version or kind, a `doc_id_len` that overruns the buffer, or a `doc_id`
    /// that is not valid UTF-8. Never panics.
    pub fn decode(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() < AAD_FIXED_LEN {
            return Err(CryptoError::MalformedAad);
        }
        if bytes[0] != AAD_VERSION {
            return Err(CryptoError::MalformedAad);
        }
        let kind = ArtifactKind::from_code(bytes[1])?;

        let doc_id_len = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;

        // The record is exactly the fixed part plus the declared doc_id length.
        // A shorter buffer is truncation; a longer one is trailing garbage.
        // Both are rejected — accepting either would let two byte strings map to
        // the same logical AAD.
        let expected_len = AAD_FIXED_LEN
            .checked_add(doc_id_len)
            .ok_or(CryptoError::MalformedAad)?;
        if bytes.len() != expected_len {
            return Err(CryptoError::MalformedAad);
        }

        let doc_id_end = 4 + doc_id_len;
        let doc_id = core::str::from_utf8(&bytes[4..doc_id_end])
            .map_err(|_| CryptoError::MalformedAad)?
            .to_owned();

        let format_version = u32::from_be_bytes([
            bytes[doc_id_end],
            bytes[doc_id_end + 1],
            bytes[doc_id_end + 2],
            bytes[doc_id_end + 3],
        ]);

        Ok(Self {
            kind,
            doc_id,
            format_version,
        })
    }
}
