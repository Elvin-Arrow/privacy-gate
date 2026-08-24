//! The document catalog (`data-model.md` §5.1 `DetectedField`, §6.1 `DocumentMeta`, §6.2
//! `OriginalRecord`; `architecture.md` §4.2; W10).
//!
//! Delivers `import_document` / `list_documents` / `get_document`'s storage layer: the
//! `document` SQL row plus its `kind=8` (`document_meta`, always) and `kind=2`
//! (`original`, iff retention is `retain`) envelope-encrypted artifacts, plus kind=1
//! (`approved`) after `submit_approval` (W18) and kind=3 (`variant`) after `save_variant`
//! (W22). Same two-AEAD-layer shape `crate::config`
//! established for `kind=4` — a fresh per-artifact DEK wraps the plaintext,
//! `vault_master_key` wraps that DEK — except these kinds are **document-scoped**: AADs
//! carry `doc_id`, unlike `Config`'s global ones.
//!
//! # `DetectedField` lands here, out of strict necessity
//!
//! `crate::importer`'s module docs (W8/W9) said the Detector's chunk (W12) would introduce
//! `DetectedField`. That turned out not to be quite right: `DocumentMeta.detected_fields`
//! (data-model §6.1) is typed against it, and `DocumentMeta` is this chunk's own
//! deliverable — a struct field can't be typed against a type that doesn't exist yet. So
//! `DetectedField` is defined here, in W10, but **W10 never constructs a non-empty one**:
//! `import_document` calls a `Detector` seam that returns `Vec::new()` until W12 fills it
//! in (dev-plan W10: "Detection may be a no-op empty field list only if W12 is the next
//! PR" — it is, in this sequence).

use serde::{Deserialize, Serialize};

use crate::crypto::{Aad, ArtifactKind, Dek, WrappedBlob, DEK_LEN};
use crate::importer::{SourceFormat, TextSpan};
use crate::keys::VaultMasterKey;

/// `format_version` bound into the AAD of every artifact this module seals. Bumping it is
/// a storage format break for `document_meta`/`original` artifacts.
pub const CATALOG_FORMAT_VERSION: u32 = 1;

/// data-model §5.1 `DetectedField.id`.
pub type FieldId = String;

/// data-model §5.1 `DetectedField`. **Not constructed with any entries by this module** —
/// see the module scope note above. The Detector (W12) is what actually classifies spans.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedField {
    pub id: FieldId,
    pub label: String,
    pub classification: String,
    pub span: TextSpan,
    pub parent_field_id: Option<FieldId>,
}

/// api.md §4 / data-model §5.2 `FieldDecisionKind`. Lives here so `ApprovedVersion` JSON
/// does not pull `crate::session` into the catalog (W18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldDecisionKind {
    KeepVisible,
    Redact,
}

/// data-model §5.2 `FieldDecision` — on-disk snapshot inside `ApprovedVersion`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldDecision {
    pub field: DetectedField,
    pub decision: FieldDecisionKind,
}

/// data-model §6.3 `redacted_content.pages[]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedPage {
    pub page_index: u32,
    pub spans: Vec<TextSpan>,
}

/// data-model §5.2 / §6.3 `RedactedDocument`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactedDocument {
    pub format: SourceFormat,
    pub pages: Vec<RedactedPage>,
}

/// data-model §6.3 `ApprovedVersion` on disk. `doc_id` is in AAD, not plaintext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedVersion {
    pub produced_at_unix_ms: u64,
    pub decisions: Vec<FieldDecision>,
    pub redacted_content: RedactedDocument,
}

/// data-model §6.4 `Variant.overrides[]` — `field_id` + decision only (no span text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantOverride {
    pub field_id: FieldId,
    pub decision: FieldDecisionKind,
}

/// data-model §6.4 `Variant` on disk. `variant_id` / `doc_id` are SQL + AAD, not plaintext.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantRecord {
    pub name: String,
    pub created_at_unix_ms: u64,
    pub overrides: Vec<VariantOverride>,
}

/// SQL cache row for `list_variants` (data-model §7: `variant.name` is a cache).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VariantListRow {
    pub variant_id: String,
    pub name: String,
    pub created_at_unix_ms: u64,
}

/// data-model §6.1: `DocumentMeta.retention` is `"retain" | "discard"` — **never**
/// `"never_retain"`. A separate type from `crate::config::RetentionPolicy` (which has all
/// three) rather than a runtime check, so a `never_retain` value here is a compile error,
/// not a bug waiting for a test to catch it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveRetention {
    Retain,
    Discard,
}

impl EffectiveRetention {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            EffectiveRetention::Retain => "retain",
            EffectiveRetention::Discard => "discard",
        }
    }
}

/// data-model §6.1 `DocumentMeta` — kind 8, document-scoped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentMeta {
    /// Basename only (api.md `import_document`: "path separators rejected").
    pub source_filename: String,
    pub source_format: SourceFormat,
    pub imported_at_unix_ms: u64,
    pub retention: EffectiveRetention,
    pub detected_fields: Vec<DetectedField>,
}

/// data-model §6.2 `OriginalRecord` — kind 2, document-scoped, present iff retention is
/// `retain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OriginalRecord {
    pub source_format: SourceFormat,
    /// RFC 4648 §4 Base64, standard alphabet, with padding (data-model §6.2, verbatim).
    pub raw_bytes_b64: String,
}

impl OriginalRecord {
    /// Encode `raw_bytes` per data-model §6.2's exact base64 profile.
    #[must_use]
    pub fn new(source_format: SourceFormat, raw_bytes: &[u8]) -> Self {
        use base64::Engine as _;
        Self {
            source_format,
            raw_bytes_b64: base64::engine::general_purpose::STANDARD.encode(raw_bytes),
        }
    }

    /// Decode `raw_bytes_b64` back to bytes.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] if the stored string is not valid standard-alphabet,
    /// padded base64 — a corrupt or tampered record, never silently truncated.
    pub fn raw_bytes(&self) -> Result<Vec<u8>, CatalogError> {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(&self.raw_bytes_b64)
            .map_err(|_| CatalogError::Backend("malformed raw_bytes_b64"))
    }
}

/// Failure modes of the catalog backend. Coarse and non-secret, same discipline as every
/// other error class in the core.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CatalogError {
    Backend(&'static str),
    /// `UNIQUE(doc_id, name)` (api.md `variant_name_conflict`).
    VariantNameConflict,
    /// C-DM-4: variant insert requires a canonical approved artifact.
    NotApproved,
}

impl core::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CatalogError::Backend(class) => write!(f, "catalog backend failure: {class}"),
            CatalogError::VariantNameConflict => write!(f, "variant name already used on this document"),
            CatalogError::NotApproved => write!(f, "document has no approved version"),
        }
    }
}

impl std::error::Error for CatalogError {}

/// Where catalog rows live. `crate::vault::SqlCipherVault` implements this over the
/// `document`/`artifact` tables (W3's schema).
pub trait DocumentStore: Send + Sync {
    /// Insert a new document: the `kind=8` meta artifact always, the `kind=2` original
    /// artifact iff `original` is `Some` (i.e. retention is `retain`), and the `document`
    /// row referencing both — one transaction (data-model §7's delete/insert-ordering
    /// discipline applies to inserts too: never a `document` row with a dangling
    /// `meta_artifact_id`).
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on any I/O/backend/encrypt failure, or if `doc_id`
    /// already exists (the catalog's primary key — callers mint a fresh id per import,
    /// testing.md §8 "Re-import").
    fn insert(
        &self,
        master: &VaultMasterKey,
        doc_id: &str,
        meta: &DocumentMeta,
        original: Option<&OriginalRecord>,
        imported_at_unix_ms: u64,
    ) -> Result<(), CatalogError>;

    /// Every `doc_id`, newest import first (api.md `list_documents`: "newest import
    /// first"). No decryption — `imported_at_unix_ms` is a plain SQL column.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on any I/O/backend failure.
    fn list_ids_newest_first(&self) -> Result<Vec<String>, CatalogError>;

    /// Decrypt and return one document's meta. `None` if `doc_id` doesn't exist.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on any I/O/backend/decrypt failure.
    fn load_meta(&self, master: &VaultMasterKey, doc_id: &str) -> Result<Option<DocumentMeta>, CatalogError>;

    /// Whether `doc_id` has a canonical `ApprovedVersion` (`document.approved_artifact_id`
    /// non-null). No decryption needed — SQL column presence only.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on any I/O/backend failure.
    fn has_approved_version(&self, doc_id: &str) -> Result<bool, CatalogError>;

    /// Whether `doc_id` currently has a retained original (`document.original_artifact_id`
    /// non-null). No decryption needed.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on any I/O/backend failure.
    fn has_retained_original(&self, doc_id: &str) -> Result<bool, CatalogError>;

    /// Insert kind=1 and set `document.approved_artifact_id` (data-model §8 / C-DM-4).
    /// Fails if an approved artifact is already stored for `doc_id`.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on any I/O/backend/encrypt failure, or if `doc_id` is
    /// missing / already has an approved version.
    fn store_approved(
        &self,
        master: &VaultMasterKey,
        doc_id: &str,
        approved: &ApprovedVersion,
    ) -> Result<(), CatalogError>;

    /// Decrypt the canonical `ApprovedVersion`. `None` if `doc_id` is unknown or has no
    /// approved artifact.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on any I/O/backend/decrypt failure.
    fn load_approved(
        &self,
        master: &VaultMasterKey,
        doc_id: &str,
    ) -> Result<Option<ApprovedVersion>, CatalogError>;

    /// Decrypt the retained original. `None` if `doc_id` is unknown or retention was
    /// discard (no kind=2).
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on any I/O/backend/decrypt failure.
    fn load_original(
        &self,
        master: &VaultMasterKey,
        doc_id: &str,
    ) -> Result<Option<OriginalRecord>, CatalogError>;

    /// Remove an unapproved document and its kind=8 / kind=2 artifacts (data-model §8:
    /// abort/lock while discard and not approved). No-op if `doc_id` is already absent.
    /// Refuses if a canonical approved version exists (that path is W20 `delete_document`).
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on I/O/backend failure, or if the document is approved.
    fn drop_unapproved(&self, doc_id: &str) -> Result<(), CatalogError>;

    /// Irrevocable delete (FR-4.6 / architecture §4.3): overwrite-and-drop every
    /// document-scoped artifact (meta, original, approved, variants), then the catalog
    /// row. No-op if `doc_id` is already absent.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on I/O/backend failure.
    fn destroy_document(&self, doc_id: &str) -> Result<(), CatalogError>;

    /// Overwrite-and-drop the retained original only (architecture §4.3). Leaves the
    /// catalog row and approved artifact. Returns whether an original was present.
    /// No-op (`Ok(false)`) if `doc_id` is missing or already has no original.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on I/O/backend failure.
    fn destroy_original(&self, doc_id: &str) -> Result<bool, CatalogError>;

    /// Insert kind=3 + `variant` row (data-model §8 / C-DM-4). `variant_id` is caller-minted.
    ///
    /// # Errors
    /// [`CatalogError::NotApproved`] if `doc_id` has no approved artifact;
    /// [`CatalogError::VariantNameConflict`] if `name` is already used on this doc;
    /// [`CatalogError::Backend`] on I/O/backend/encrypt failure, or if `doc_id` is missing.
    fn store_variant(
        &self,
        master: &VaultMasterKey,
        doc_id: &str,
        variant_id: &str,
        record: &VariantRecord,
    ) -> Result<(), CatalogError>;

    /// SQL `variant` rows for `doc_id`, newest first. Empty if the document has none.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on I/O/backend failure.
    fn list_variants(&self, doc_id: &str) -> Result<Vec<VariantListRow>, CatalogError>;

    /// Decrypt one variant. `None` if `doc_id`/`variant_id` do not match a row.
    /// Envelope `name` must match the SQL cache (data-model §7); mismatch is an error,
    /// not a served row.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on I/O/backend/decrypt/integrity failure.
    fn load_variant(
        &self,
        master: &VaultMasterKey,
        doc_id: &str,
        variant_id: &str,
    ) -> Result<Option<VariantRecord>, CatalogError>;

    /// Overwrite-and-drop one kind=3 variant (architecture §4.3). Returns whether a
    /// matching row was present. No-op (`Ok(false)`) if missing.
    ///
    /// # Errors
    /// [`CatalogError::Backend`] on I/O/backend failure.
    fn destroy_variant(&self, doc_id: &str, variant_id: &str) -> Result<bool, CatalogError>;
}

/// The W2–W9-era no-op backend. Exists so every constructor that predates W10 keeps
/// working unmodified.
#[derive(Debug, Default)]
pub struct NullDocumentStore;

impl DocumentStore for NullDocumentStore {
    fn insert(
        &self,
        _master: &VaultMasterKey,
        _doc_id: &str,
        _meta: &DocumentMeta,
        _original: Option<&OriginalRecord>,
        _imported_at_unix_ms: u64,
    ) -> Result<(), CatalogError> {
        Err(CatalogError::Backend("no document store configured"))
    }
    fn list_ids_newest_first(&self) -> Result<Vec<String>, CatalogError> {
        Ok(Vec::new())
    }
    fn load_meta(&self, _master: &VaultMasterKey, _doc_id: &str) -> Result<Option<DocumentMeta>, CatalogError> {
        Ok(None)
    }
    fn has_approved_version(&self, _doc_id: &str) -> Result<bool, CatalogError> {
        Ok(false)
    }
    fn has_retained_original(&self, _doc_id: &str) -> Result<bool, CatalogError> {
        Ok(false)
    }
    fn store_approved(
        &self,
        _master: &VaultMasterKey,
        _doc_id: &str,
        _approved: &ApprovedVersion,
    ) -> Result<(), CatalogError> {
        Err(CatalogError::Backend("no document store configured"))
    }
    fn load_approved(
        &self,
        _master: &VaultMasterKey,
        _doc_id: &str,
    ) -> Result<Option<ApprovedVersion>, CatalogError> {
        Ok(None)
    }
    fn load_original(
        &self,
        _master: &VaultMasterKey,
        _doc_id: &str,
    ) -> Result<Option<OriginalRecord>, CatalogError> {
        Ok(None)
    }
    fn drop_unapproved(&self, _doc_id: &str) -> Result<(), CatalogError> {
        Err(CatalogError::Backend("no document store configured"))
    }
    fn destroy_document(&self, _doc_id: &str) -> Result<(), CatalogError> {
        Err(CatalogError::Backend("no document store configured"))
    }
    fn destroy_original(&self, _doc_id: &str) -> Result<bool, CatalogError> {
        Err(CatalogError::Backend("no document store configured"))
    }
    fn store_variant(
        &self,
        _master: &VaultMasterKey,
        _doc_id: &str,
        _variant_id: &str,
        _record: &VariantRecord,
    ) -> Result<(), CatalogError> {
        Err(CatalogError::Backend("no document store configured"))
    }
    fn list_variants(&self, _doc_id: &str) -> Result<Vec<VariantListRow>, CatalogError> {
        Ok(Vec::new())
    }
    fn load_variant(
        &self,
        _master: &VaultMasterKey,
        _doc_id: &str,
        _variant_id: &str,
    ) -> Result<Option<VariantRecord>, CatalogError> {
        Ok(None)
    }
    fn destroy_variant(&self, _doc_id: &str, _variant_id: &str) -> Result<bool, CatalogError> {
        Err(CatalogError::Backend("no document store configured"))
    }
}

// ---------------------------------------------------------------------------
// Crypto (architecture §3.1's two-layer shape, document-scoped AAD)
// ---------------------------------------------------------------------------

/// AAD for the `document_meta` plaintext layer (kind 8, document-scoped).
#[must_use]
pub fn meta_plaintext_aad(doc_id: &str) -> Aad {
    Aad::for_document(ArtifactKind::DocumentMeta, doc_id, CATALOG_FORMAT_VERSION)
}

/// AAD for the `original` plaintext layer (kind 2, document-scoped).
#[must_use]
pub fn original_plaintext_aad(doc_id: &str) -> Aad {
    Aad::for_document(ArtifactKind::Original, doc_id, CATALOG_FORMAT_VERSION)
}

/// AAD for the `approved` plaintext layer (kind 1, document-scoped).
#[must_use]
pub fn approved_plaintext_aad(doc_id: &str) -> Aad {
    Aad::for_document(ArtifactKind::Approved, doc_id, CATALOG_FORMAT_VERSION)
}

/// AAD for the `variant` plaintext layer (kind 3, document-scoped). `doc_id` is bound
/// here; `variant_id` lives in SQL (data-model §6.4; architecture §3.1 AAD has one id).
#[must_use]
pub fn variant_plaintext_aad(doc_id: &str) -> Aad {
    Aad::for_document(ArtifactKind::Variant, doc_id, CATALOG_FORMAT_VERSION)
}

/// AAD for a document-scoped artifact's DEK-wrap layer (kind 7 — data-model §6: "as
/// wrapped artifact", i.e. the same `doc_id` as whatever it wraps).
#[must_use]
pub fn wrap_dek_aad(doc_id: &str) -> Aad {
    Aad::for_document(ArtifactKind::WrappedDek, doc_id, CATALOG_FORMAT_VERSION)
}

/// Seal `meta` under a fresh DEK, then wrap that DEK under `master`. Returns
/// `(wrapped_dek, artifact_blob)`, ready for a caller to persist onto `artifact` columns.
///
/// # Errors
/// Whatever the underlying AEAD wrap calls return (CSPRNG failure).
pub fn seal_document_meta(
    master: &VaultMasterKey,
    doc_id: &str,
    meta: &DocumentMeta,
) -> Result<(WrappedBlob, WrappedBlob), CatalogError> {
    seal(master, meta, &meta_plaintext_aad(doc_id), &wrap_dek_aad(doc_id))
}

/// The inverse of [`seal_document_meta`].
///
/// # Errors
/// [`CatalogError::Backend`] if either AEAD layer fails to authenticate, or the plaintext
/// is not valid `DocumentMeta` JSON.
pub fn open_document_meta(
    master: &VaultMasterKey,
    doc_id: &str,
    wrapped_dek: &WrappedBlob,
    artifact_blob: &WrappedBlob,
) -> Result<DocumentMeta, CatalogError> {
    open(
        master,
        wrapped_dek,
        artifact_blob,
        &meta_plaintext_aad(doc_id),
        &wrap_dek_aad(doc_id),
    )
}

/// Seal an [`OriginalRecord`]. Same shape as [`seal_document_meta`], kind 2 instead of 8.
///
/// # Errors
/// Whatever the underlying AEAD wrap calls return (CSPRNG failure).
pub fn seal_original(
    master: &VaultMasterKey,
    doc_id: &str,
    original: &OriginalRecord,
) -> Result<(WrappedBlob, WrappedBlob), CatalogError> {
    seal(master, original, &original_plaintext_aad(doc_id), &wrap_dek_aad(doc_id))
}

/// The inverse of [`seal_original`].
///
/// # Errors
/// [`CatalogError::Backend`] if either AEAD layer fails to authenticate, or the plaintext
/// is not valid `OriginalRecord` JSON.
pub fn open_original(
    master: &VaultMasterKey,
    doc_id: &str,
    wrapped_dek: &WrappedBlob,
    artifact_blob: &WrappedBlob,
) -> Result<OriginalRecord, CatalogError> {
    open(
        master,
        wrapped_dek,
        artifact_blob,
        &original_plaintext_aad(doc_id),
        &wrap_dek_aad(doc_id),
    )
}

/// Seal an [`ApprovedVersion`]. Same shape as [`seal_document_meta`], kind 1.
///
/// # Errors
/// Whatever the underlying AEAD wrap calls return (CSPRNG failure).
pub fn seal_approved(
    master: &VaultMasterKey,
    doc_id: &str,
    approved: &ApprovedVersion,
) -> Result<(WrappedBlob, WrappedBlob), CatalogError> {
    seal(master, approved, &approved_plaintext_aad(doc_id), &wrap_dek_aad(doc_id))
}

/// The inverse of [`seal_approved`].
///
/// # Errors
/// [`CatalogError::Backend`] if either AEAD layer fails to authenticate, or the plaintext
/// is not valid `ApprovedVersion` JSON.
pub fn open_approved(
    master: &VaultMasterKey,
    doc_id: &str,
    wrapped_dek: &WrappedBlob,
    artifact_blob: &WrappedBlob,
) -> Result<ApprovedVersion, CatalogError> {
    open(
        master,
        wrapped_dek,
        artifact_blob,
        &approved_plaintext_aad(doc_id),
        &wrap_dek_aad(doc_id),
    )
}

/// Seal a [`VariantRecord`]. Same shape as [`seal_approved`], kind 3.
///
/// # Errors
/// Whatever the underlying AEAD wrap calls return (CSPRNG failure).
pub fn seal_variant(
    master: &VaultMasterKey,
    doc_id: &str,
    record: &VariantRecord,
) -> Result<(WrappedBlob, WrappedBlob), CatalogError> {
    seal(master, record, &variant_plaintext_aad(doc_id), &wrap_dek_aad(doc_id))
}

/// The inverse of [`seal_variant`].
///
/// # Errors
/// [`CatalogError::Backend`] if either AEAD layer fails to authenticate, or the plaintext
/// is not valid `VariantRecord` JSON.
pub fn open_variant(
    master: &VaultMasterKey,
    doc_id: &str,
    wrapped_dek: &WrappedBlob,
    artifact_blob: &WrappedBlob,
) -> Result<VariantRecord, CatalogError> {
    open(
        master,
        wrapped_dek,
        artifact_blob,
        &variant_plaintext_aad(doc_id),
        &wrap_dek_aad(doc_id),
    )
}

/// The generic two-layer seal `seal_document_meta`/`seal_original` both specialize —
/// mirrors `crate::config::seal_config`'s shape exactly, parametrized on AAD instead of
/// hardcoding the global one.
fn seal<T: Serialize>(
    master: &VaultMasterKey,
    value: &T,
    plaintext_aad: &Aad,
    dek_wrap_aad: &Aad,
) -> Result<(WrappedBlob, WrappedBlob), CatalogError> {
    let dek = Dek::generate();
    let plaintext = serde_json::to_vec(value).map_err(|_| CatalogError::Backend("serialize failed"))?;
    let artifact_blob = crate::crypto::wrap(dek.as_bytes(), &plaintext, plaintext_aad)
        .map_err(|_| CatalogError::Backend("artifact wrap failed"))?;
    let wrapped_dek = crate::crypto::wrap(master.as_bytes(), dek.as_bytes(), dek_wrap_aad)
        .map_err(|_| CatalogError::Backend("dek wrap failed"))?;
    Ok((wrapped_dek, artifact_blob))
}

fn open<T: for<'de> Deserialize<'de>>(
    master: &VaultMasterKey,
    wrapped_dek: &WrappedBlob,
    artifact_blob: &WrappedBlob,
    plaintext_aad: &Aad,
    dek_wrap_aad: &Aad,
) -> Result<T, CatalogError> {
    use zeroize::Zeroizing;
    let dek_bytes: Zeroizing<Vec<u8>> = Zeroizing::new(
        crate::crypto::unwrap(master.as_bytes(), wrapped_dek, dek_wrap_aad)
            .map_err(|_| CatalogError::Backend("dek unwrap failed"))?,
    );
    let dek_array: [u8; DEK_LEN] = dek_bytes
        .as_slice()
        .try_into()
        .map_err(|_| CatalogError::Backend("dek has the wrong length"))?;
    let dek = Dek::from_bytes(dek_array);

    let plaintext = crate::crypto::unwrap(dek.as_bytes(), artifact_blob, plaintext_aad)
        .map_err(|_| CatalogError::Backend("artifact unwrap failed"))?;
    serde_json::from_slice(&plaintext).map_err(|_| CatalogError::Backend("malformed JSON"))
}
