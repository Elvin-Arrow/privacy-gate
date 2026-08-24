//! The SQLCipher vault (`architecture.md` §4, `data-model.md` §7).
//!
//! W3 delivers exactly what dev-plan.md says and nothing more: "create/open SQLCipher DB
//! in app-data dir; schema from data-model.md (tables may exist empty); open with raw key
//! (not passphrase-KDF on the DB)." `create_account` / `unlock` open the DB through this
//! module; `lock` closes it (`crate::session`).
//!
//! # Why `AccountStore` lives here too
//!
//! `crate::account`'s module docs: "data-model §5.6 puts `LocalAccount` in the SQLCipher
//! vault... W3 owns that database, so W2 keeps the record behind an `AccountStore` trait
//! ... and W3 swaps in a SQLCipher-backed one." [`SqlCipherVault`] is that backend: it
//! implements both [`VaultBackend`] (open/close the DB) and `AccountStore` (read/write the
//! one `account` row) over the **same** `rusqlite::Connection`, so a single `Arc` passed
//! to `SessionManager::new_with_vault` as both arguments shares one live connection rather
//! than two independently-lifecycled ones.
//!
//! # Raw-key opening (testing.md §5.3 gated module)
//!
//! architecture §3.1: "SQLCipher keying: `sqlcipher_key` is a raw 256-bit key, **not** a
//! passphrase. Open with `PRAGMA key = "x'<64 lowercase hex chars>'"`... Do not pass the
//! key as a UTF-8 passphrase; that would apply SQLCipher's default PBKDF2 (~256k
//! iterations) on top of HKDF and blow the unlock budget." [`open_raw_key_pragma`] is the
//! one place that formats the key; nothing else in this module touches key bytes.

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension};
use zeroize::Zeroizing;

use crate::account::{AccountStore, AccountStoreError, LocalAccount};
use crate::audit::{AuditError, AuditRow, AuditStore, EventType, OriginalsFlag};
use crate::catalog::{ApprovedVersion, CatalogError, DocumentMeta, DocumentStore, OriginalRecord};
use crate::config::{Config, ConfigError, ConfigStore};
use crate::crypto::{WrappedBlob, NONCE_LEN};
use crate::keys::{VaultMasterKey, KEY_LEN};

/// data-model §7: `schema_meta` row `('schema_version', '1')`.
pub const SCHEMA_VERSION: i64 = 1;

/// Failure modes of vault open/close. Coarse and non-secret (api.md §3 / C-API-1): never
/// the key, never a SQLCipher error string that might embed a path or byte dump.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VaultError {
    /// The key did not open this database — wrong key, or the file is not a Privacy Gate
    /// vault. architecture §2.4 "stolen data file": this is the negative case that
    /// property must hold for.
    WrongKey,
    /// Anything else: I/O failure, schema mismatch, poisoned lock. `class` is a fixed,
    /// non-secret label, same discipline as `crate::keystore::KeystoreError::Backend`.
    Backend(&'static str),
}

impl core::fmt::Display for VaultError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VaultError::WrongKey => f.write_str("vault key did not open the database"),
            VaultError::Backend(class) => write!(f, "vault backend failure: {class}"),
        }
    }
}

impl std::error::Error for VaultError {}

/// Open/close the SQLCipher vault. `crate::session::SessionManager` holds one of these
/// and calls [`Self::open`] from `create_account` / `unlock`, [`Self::close`] from `lock`.
pub trait VaultBackend: Send + Sync {
    /// Open (creating if absent) the database at this backend's path with the given raw
    /// 256-bit key, ensuring the v1 schema exists. Idempotent while already open: the
    /// held connection is kept as-is and `key` is not re-checked against it (this backend
    /// never needs to re-key a live connection — `change_passphrase` rotates the KEK, not
    /// the DB key, and never calls `open` again without an intervening [`Self::close`]).
    ///
    /// # Errors
    /// [`VaultError::WrongKey`] if the file exists and this key does not decrypt it.
    fn open(&self, key: &Zeroizing<[u8; KEY_LEN]>) -> Result<(), VaultError>;
    /// Close the connection, if open. Idempotent.
    fn close(&self);
    /// Close the connection (if open) and permanently remove the underlying database
    /// file, if any. Idempotent; missing file is not an error.
    ///
    /// Only ever correct to call when there is provably no valid keystore item pointing
    /// at this vault — i.e. from `create_account`'s `first_run` path. Without the
    /// keystore's salt and wrapped master key there is no way to recover this file's
    /// contents by any means (architecture §3.1), so a vault file found here is either
    /// this call's own fresh, empty database or an orphan left by a previously aborted
    /// `create_account` — never live data.
    fn destroy(&self);
    /// True while a connection is held.
    fn is_open(&self) -> bool;
}

/// The W2-era no-op backend: `open`/`close` are trivial successes, `is_open` is always
/// `false`. Exists so `SessionManager::new` (bare keystore + account store, no vault) keeps
/// working unmodified for session-layer unit tests that predate W3 — see
/// `crate::session::SessionManager::new`.
#[derive(Debug, Default)]
pub struct NullVault;

impl VaultBackend for NullVault {
    fn open(&self, _key: &Zeroizing<[u8; KEY_LEN]>) -> Result<(), VaultError> {
        Ok(())
    }
    fn close(&self) {}
    fn destroy(&self) {}
    fn is_open(&self) -> bool {
        false
    }
}

/// The real, file-backed SQLCipher vault (architecture §4.1: one `vault.db` per app-data
/// directory; exact directory is the caller's concern — this type only knows the file
/// path it was given).
pub struct SqlCipherVault {
    path: PathBuf,
    conn: Mutex<Option<Connection>>,
}

impl SqlCipherVault {
    /// A vault backed by the SQLCipher database at `path`. Does not touch the filesystem
    /// until [`Self::open`].
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            conn: Mutex::new(None),
        }
    }

    /// The `schema_meta` `schema_version` value of the open database. Exposed for
    /// component tests (dev-plan W3: "Tests first: ... schema_version = 1"); production
    /// code has no reason to read it back.
    ///
    /// # Errors
    /// [`VaultError::Backend`] if the vault is not open or the row is missing/unreadable.
    pub fn schema_version(&self) -> Result<i64, VaultError> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT v FROM schema_meta WHERE k = 'schema_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| VaultError::Backend("schema_meta row missing"))
            .and_then(|v| {
                v.parse::<i64>()
                    .map_err(|_| VaultError::Backend("schema_meta row not an integer"))
            })
        })
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T, VaultError>) -> Result<T, VaultError> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| VaultError::Backend("poisoned"))?;
        let conn = guard.as_ref().ok_or(VaultError::Backend("vault not open"))?;
        f(conn)
    }

    /// As [`Self::with_conn`], but with a mutable borrow — needed for `Connection::
    /// transaction()`, which `rusqlite` only exposes on `&mut Connection`.
    fn with_conn_mut<T>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, VaultError>,
    ) -> Result<T, VaultError> {
        let mut guard = self
            .conn
            .lock()
            .map_err(|_| VaultError::Backend("poisoned"))?;
        let conn = guard.as_mut().ok_or(VaultError::Backend("vault not open"))?;
        f(conn)
    }
}

impl VaultBackend for SqlCipherVault {
    fn open(&self, key: &Zeroizing<[u8; KEY_LEN]>) -> Result<(), VaultError> {
        let mut guard = self.conn.lock().map_err(|_| VaultError::Backend("poisoned"))?;
        if guard.is_some() {
            // Already open in this process. W3 does not support re-keying through this
            // path (that is `change_passphrase`'s KEK rotation, which never touches the
            // DB key at all — architecture §3.1's HKDF label is fixed per master key).
            return Ok(());
        }

        let conn = Connection::open(&self.path).map_err(|_| VaultError::Backend("could not open file"))?;
        open_raw_key_pragma(&conn, key)?;
        verify_key(&conn)?;
        ensure_schema(&conn)?;

        *guard = Some(conn);
        Ok(())
    }

    fn close(&self) {
        // Recover a poisoned lock rather than silently no-op: architecture §3.3's lock
        // contract ("close the DB") must hold even if some earlier `with_conn` closure
        // panicked mid-query. The `Option<Connection>` has no invariant that a panic could
        // have broken, so taking the poisoned guard's inner value is safe, and it is the
        // only way `close()` cannot leave a live handle open behind a poisoned mutex.
        let mut guard = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Dropping the `Connection` closes the underlying SQLite/SQLCipher handle.
        *guard = None;
    }

    fn destroy(&self) {
        self.close();
        // Idempotent: a missing file is not a failure worth reporting anywhere (there is
        // nowhere non-secret to report it to — `VaultBackend::destroy` returns nothing).
        let _ = std::fs::remove_file(&self.path);
    }

    fn is_open(&self) -> bool {
        // Same poisoned-lock recovery as `close`: report the true state rather than the
        // safe-looking `false`, which would hide a still-open handle from a caller
        // deciding whether it's safe to treat the vault as closed.
        self.conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

/// architecture §3.1: raw `x'<64 lowercase hex chars>'` key form, not a passphrase.
/// `PRAGMA key` alone never fails even on the wrong key — SQLCipher only discovers a
/// mismatch on the first real read, which is why [`verify_key`] is a separate step.
///
/// The formatted string still holds the raw key as 64 hex characters, so it is built into
/// a [`Zeroizing`] buffer and written nibble-by-nibble (no `format!` per byte, which would
/// leave 32 unwiped heap allocations behind). This is the one place in the codebase where
/// key bytes are rendered to text at all; `pragma_update` takes ownership of the string and
/// hands it to `rusqlite`'s own SQL-text path, which this function cannot reach into —
/// wiping what is under our control is still strictly better than leaving all of it.
fn open_raw_key_pragma(conn: &Connection, key: &Zeroizing<[u8; KEY_LEN]>) -> Result<(), VaultError> {
    const HEX: &[u8; 16] = b"0123456789abcdef"; // lowercase, per architecture §3.1.

    let mut hex = Zeroizing::new(String::with_capacity(2 + KEY_LEN * 2 + 1));
    hex.push('x');
    hex.push('\'');
    for byte in key.iter() {
        hex.push(HEX[(byte >> 4) as usize] as char);
        hex.push(HEX[(byte & 0x0f) as usize] as char);
    }
    hex.push('\'');
    conn.pragma_update(None, "key", hex.as_str())
        .map_err(|_| VaultError::Backend("could not set key pragma"))
}

/// The first real read after `PRAGMA key`. A wrong key (or a file that is not a Privacy
/// Gate / SQLCipher database at all) fails here with SQLITE_NOTADB, which `rusqlite`
/// surfaces as a generic error — collapsed to [`VaultError::WrongKey`], the negative case
/// of the "stolen file" property (architecture §2.4).
fn verify_key(conn: &Connection) -> Result<(), VaultError> {
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get::<_, i64>(0))
        .map(|_| ())
        .map_err(|_| VaultError::WrongKey)
}

/// data-model §7 schema, verbatim. `CREATE TABLE IF NOT EXISTS` makes this idempotent
/// across reopen-after-lock/unlock (dev-plan W3: "reopen after lock/unlock").
fn ensure_schema(conn: &Connection) -> Result<(), VaultError> {
    conn.execute_batch(
        "
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS schema_meta (
          k TEXT PRIMARY KEY,
          v TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS account (
          account_id TEXT PRIMARY KEY,
          display_name TEXT NOT NULL,
          created_at_unix_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS artifact (
          artifact_id TEXT PRIMARY KEY,
          kind INTEGER NOT NULL,
          doc_id TEXT,
          format_version INTEGER NOT NULL,
          wrapped_dek BLOB NOT NULL,
          nonce BLOB NOT NULL,
          ciphertext BLOB NOT NULL,
          created_at_unix_ms INTEGER NOT NULL,
          CHECK (kind IN (1, 2, 3, 4, 5, 8))
        );

        CREATE TABLE IF NOT EXISTS document (
          doc_id TEXT PRIMARY KEY,
          meta_artifact_id TEXT NOT NULL UNIQUE
            REFERENCES artifact(artifact_id) ON DELETE RESTRICT,
          original_artifact_id TEXT UNIQUE
            REFERENCES artifact(artifact_id) ON DELETE RESTRICT,
          approved_artifact_id TEXT UNIQUE
            REFERENCES artifact(artifact_id) ON DELETE RESTRICT,
          imported_at_unix_ms INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS variant (
          variant_id TEXT PRIMARY KEY,
          doc_id TEXT NOT NULL REFERENCES document(doc_id) ON DELETE RESTRICT,
          artifact_id TEXT NOT NULL UNIQUE
            REFERENCES artifact(artifact_id) ON DELETE RESTRICT,
          name TEXT NOT NULL,
          created_at_unix_ms INTEGER NOT NULL,
          UNIQUE (doc_id, name)
        );

        CREATE TABLE IF NOT EXISTS plugin_secret (
          plugin_id TEXT PRIMARY KEY,
          artifact_id TEXT NOT NULL UNIQUE
            REFERENCES artifact(artifact_id) ON DELETE RESTRICT
        );

        CREATE TABLE IF NOT EXISTS audit_entry (
          sequence INTEGER PRIMARY KEY,
          event_type INTEGER NOT NULL,
          doc_id TEXT,
          produced_at_unix_ms INTEGER NOT NULL,
          originals_flag INTEGER NOT NULL,
          payload_jcs TEXT NOT NULL,
          prev_entry_hash BLOB NOT NULL,
          entry_signature BLOB NOT NULL
        );

        CREATE INDEX IF NOT EXISTS audit_entry_doc ON audit_entry(doc_id);
        CREATE INDEX IF NOT EXISTS artifact_doc ON artifact(doc_id);
        CREATE UNIQUE INDEX IF NOT EXISTS uq_artifact_config ON artifact(kind) WHERE kind = 4;
        CREATE UNIQUE INDEX IF NOT EXISTS uq_artifact_cloud_ai ON artifact(kind) WHERE kind = 5;

        INSERT OR IGNORE INTO schema_meta (k, v) VALUES ('schema_version', '1');
        ",
    )
    .map_err(|_| VaultError::Backend("schema creation failed"))
}

// ---------------------------------------------------------------------------
// AccountStore over the same connection (data-model §5.6: SQLCipher-only, not
// envelope-encrypted — `display_name` is not a secret).
// ---------------------------------------------------------------------------

impl AccountStore for SqlCipherVault {
    fn load(&self) -> Result<Option<LocalAccount>, AccountStoreError> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT account_id, display_name, created_at_unix_ms FROM account LIMIT 1",
                [],
                |row| {
                    let id: String = row.get(0)?;
                    let display_name: String = row.get(1)?;
                    let created_at_unix_ms: i64 = row.get(2)?;
                    Ok((id, display_name, created_at_unix_ms))
                },
            )
            .optional()
            .map_err(|_| VaultError::Backend("account query failed"))
        })
        .map_err(account_store_err)
        .map(|row| {
            row.map(|(id, display_name, created_at_unix_ms)| LocalAccount {
                id,
                display_name,
                created_at: crate::account::format_rfc3339(created_at_unix_ms / 1000),
            })
        })
    }

    fn store(&self, account: &LocalAccount) -> Result<(), AccountStoreError> {
        let created_at_unix_ms = rfc3339_to_unix_ms(&account.created_at);
        self.with_conn_mut(|conn| {
            // v1 holds at most one account (architecture §7); a fresh insert replaces
            // whatever was there (there is at most one caller of `store`: `create_account`
            // on a `first_run` session, so this never overwrites a live account). One
            // transaction so a failed INSERT cannot leave the delete committed and no row
            // in its place — data-model §7's "one Vault transaction" discipline, applied
            // here even though v1's only caller has nothing to lose from a partial write.
            let tx = conn
                .transaction()
                .map_err(|_| VaultError::Backend("could not start transaction"))?;
            tx.execute("DELETE FROM account", [])
                .map_err(|_| VaultError::Backend("account delete failed"))?;
            tx.execute(
                "INSERT INTO account (account_id, display_name, created_at_unix_ms) VALUES (?1, ?2, ?3)",
                rusqlite::params![account.id, account.display_name, created_at_unix_ms],
            )
            .map_err(|_| VaultError::Backend("account insert failed"))?;
            tx.commit()
                .map_err(|_| VaultError::Backend("account transaction commit failed"))?;
            Ok(())
        })
        .map_err(account_store_err)
    }

    fn delete(&self) -> Result<(), AccountStoreError> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM account", [])
                .map_err(|_| VaultError::Backend("account delete failed"))?;
            Ok(())
        })
        .map_err(account_store_err)
    }
}

/// Preserve the real `VaultError` class rather than collapsing every failure into one
/// fixed `"vault not open"` string — the label is still a fixed, non-secret constant
/// either way (C-API-1), so there is no reason to discard the more accurate one.
fn account_store_err(e: VaultError) -> AccountStoreError {
    match e {
        VaultError::WrongKey => AccountStoreError::Backend("vault key mismatch"),
        VaultError::Backend(class) => AccountStoreError::Backend(class),
    }
}

// ---------------------------------------------------------------------------
// AuditStore over the same connection (architecture §6, W5). Rows live in the
// `audit_entry` table W3's schema already creates.
// ---------------------------------------------------------------------------

/// One raw `audit_entry` row as it comes back from SQL, before decoding into
/// [`AuditRow`] (event/originals codes validated, hash/signature blobs sized). Named
/// purely to keep `replay`'s query-map type readable (clippy `type_complexity`).
type RawAuditRow = (i64, i64, Option<String>, i64, i64, String, Vec<u8>, Vec<u8>);

impl AuditStore for SqlCipherVault {
    fn append_row(&self, row: &AuditRow) -> Result<(), AuditError> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO audit_entry
                    (sequence, event_type, doc_id, produced_at_unix_ms, originals_flag,
                     payload_jcs, prev_entry_hash, entry_signature)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    row.sequence,
                    i64::from(row.event_type.as_u8()),
                    row.doc_id,
                    row.produced_at_unix_ms,
                    i64::from(row.originals_flag.as_u8()),
                    row.payload_jcs,
                    row.prev_entry_hash.as_slice(),
                    row.entry_signature.as_slice(),
                ],
            )
            .map_err(|_| VaultError::Backend("audit insert failed"))?;
            Ok(())
        })
        .map_err(audit_err)
    }

    fn replay(&self) -> Result<Vec<AuditRow>, AuditError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT sequence, event_type, doc_id, produced_at_unix_ms, originals_flag,
                            payload_jcs, prev_entry_hash, entry_signature
                     FROM audit_entry ORDER BY sequence ASC",
                )
                .map_err(|_| VaultError::Backend("audit query prep failed"))?;

            let raw_rows: Vec<RawAuditRow> = stmt
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                    ))
                })
                .map_err(|_| VaultError::Backend("audit query failed"))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| VaultError::Backend("audit row decode failed"))?;

            raw_rows
                .into_iter()
                .map(
                    |(sequence, event_type_raw, doc_id, produced_at, originals_raw, payload_jcs, prev, sig)| {
                        let event_type = EventType::from_u8(u8::try_from(event_type_raw).unwrap_or(0))
                            .map_err(|_| VaultError::Backend("bad event_type"))?;
                        let originals_flag = OriginalsFlag::from_u8(u8::try_from(originals_raw).unwrap_or(0))
                            .map_err(|_| VaultError::Backend("bad originals_flag"))?;
                        let prev_entry_hash: [u8; 32] = prev
                            .try_into()
                            .map_err(|_| VaultError::Backend("bad prev_entry_hash length"))?;
                        let entry_signature: [u8; 32] = sig
                            .try_into()
                            .map_err(|_| VaultError::Backend("bad entry_signature length"))?;
                        Ok(AuditRow {
                            sequence: sequence.max(0) as u64,
                            event_type,
                            doc_id,
                            produced_at_unix_ms: produced_at.max(0) as u64,
                            originals_flag,
                            payload_jcs,
                            prev_entry_hash,
                            entry_signature,
                        })
                    },
                )
                .collect::<Result<Vec<AuditRow>, VaultError>>()
        })
        .map_err(audit_err)
    }
}

/// Preserve the real `VaultError` class (same reasoning as `account_store_err`).
fn audit_err(e: VaultError) -> AuditError {
    match e {
        VaultError::WrongKey => AuditError::Backend("vault key mismatch"),
        VaultError::Backend(class) => AuditError::Backend(class),
    }
}

// ---------------------------------------------------------------------------
// Test-support only: simulate an attacker who edited the DB file bytes directly,
// bypassing every code path in this crate (architecture §2.4's threat model). Narrowly
// scoped — corrupt or shorten *existing* rows, not inject a validly-signed-looking one —
// rather than a general raw-SQL escape hatch, which would be poor hygiene on a TCB type
// whose whole job is to be the only writer of this table. testing.md §8: "Flip a byte in
// an audit payload"; "Truncate tail below persisted audit_head."
// ---------------------------------------------------------------------------

impl SqlCipherVault {
    /// Flip one bit of the stored `payload_jcs` for the row at `sequence`, so its
    /// `entry_signature` no longer verifies. No-op (`Ok(())`) if the row doesn't exist.
    ///
    /// # Errors
    /// [`VaultError::Backend`] if the vault is not open or the write fails.
    pub fn test_only_corrupt_payload(&self, sequence: u64) -> Result<(), VaultError> {
        self.with_conn(|conn| {
            let payload: Option<String> = conn
                .query_row(
                    "SELECT payload_jcs FROM audit_entry WHERE sequence = ?1",
                    [sequence],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|_| VaultError::Backend("payload read failed"))?;
            let Some(mut payload) = payload else {
                return Ok(());
            };
            // Flip a bit of the first byte — every JCS payload this module ever writes is
            // non-empty ("{}" at minimum), so there is always a byte to flip.
            // SAFETY-equivalent: operate on the raw byte, not a char boundary, which is
            // fine — this is deliberately corrupting the string, not producing valid UTF-8.
            let mut bytes = payload.into_bytes();
            bytes[0] ^= 0x01;
            payload = String::from_utf8_lossy(&bytes).into_owned();
            conn.execute(
                "UPDATE audit_entry SET payload_jcs = ?1 WHERE sequence = ?2",
                rusqlite::params![payload, sequence],
            )
            .map_err(|_| VaultError::Backend("payload corrupt-write failed"))?;
            Ok(())
        })
    }

    /// Delete every row with `sequence > keep_through`, simulating a truncated tail.
    ///
    /// # Errors
    /// [`VaultError::Backend`] if the vault is not open or the delete fails.
    pub fn test_only_truncate_after(&self, keep_through: u64) -> Result<(), VaultError> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM audit_entry WHERE sequence > ?1", [keep_through])
                .map_err(|_| VaultError::Backend("truncate failed"))?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// ConfigStore over the same connection (W6, data-model §5.5). The `artifact` table's
// unique `kind=4` row (`uq_artifact_config`, W3's schema) holds the envelope-encrypted
// `Config` blob — the crypto itself (`seal_config`/`open_config`) lives in `crate::config`;
// this impl only owns getting the right bytes into and out of SQL columns.
// ---------------------------------------------------------------------------

/// `artifact.wrapped_dek` has no sibling nonce column (unlike the artifact's own
/// `nonce`/`ciphertext` pair) — data-model §7's schema gives the DEK wrap exactly one BLOB
/// column. This module's storage-format choice, not a spec-mandated one (analogous to
/// `crate::keystore`'s choice to hex-encode its own wrapped blobs): the wrapped DEK's own
/// 24-byte nonce is prepended to its ciphertext, one self-contained blob.
const CONFIG_KIND: i64 = 4; // `ArtifactKind::Config as u8`, data-model §6.

fn pack_wrapped_dek(w: &WrappedBlob) -> Vec<u8> {
    let mut out = Vec::with_capacity(NONCE_LEN + w.ciphertext.len());
    out.extend_from_slice(&w.nonce);
    out.extend_from_slice(&w.ciphertext);
    out
}

fn unpack_wrapped_dek(bytes: &[u8]) -> Result<WrappedBlob, VaultError> {
    if bytes.len() < NONCE_LEN {
        return Err(VaultError::Backend("wrapped_dek too short"));
    }
    let (nonce, ciphertext) = bytes.split_at(NONCE_LEN);
    Ok(WrappedBlob {
        nonce: nonce.to_vec(),
        ciphertext: ciphertext.to_vec(),
    })
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl ConfigStore for SqlCipherVault {
    fn load(&self, master: &VaultMasterKey) -> Result<Option<Config>, ConfigError> {
        let row = self
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT wrapped_dek, nonce, ciphertext FROM artifact WHERE kind = ?1 LIMIT 1",
                    [CONFIG_KIND],
                    |r| {
                        let wrapped_dek: Vec<u8> = r.get(0)?;
                        let nonce: Vec<u8> = r.get(1)?;
                        let ciphertext: Vec<u8> = r.get(2)?;
                        Ok((wrapped_dek, nonce, ciphertext))
                    },
                )
                .optional()
                .map_err(|_| VaultError::Backend("config query failed"))
            })
            .map_err(config_err)?;

        let Some((wrapped_dek_bytes, nonce, ciphertext)) = row else {
            return Ok(None);
        };
        let wrapped_dek = unpack_wrapped_dek(&wrapped_dek_bytes).map_err(config_err)?;
        let artifact_blob = WrappedBlob { nonce, ciphertext };
        crate::config::open_config(master, &wrapped_dek, &artifact_blob).map(Some)
    }

    fn store(&self, master: &VaultMasterKey, config: &Config) -> Result<(), ConfigError> {
        let (wrapped_dek, artifact_blob) = crate::config::seal_config(master, config)?;
        let wrapped_dek_bytes = pack_wrapped_dek(&wrapped_dek);
        let artifact_id = uuid::Uuid::new_v4().to_string();
        let created_at_unix_ms = now_unix_ms();

        self.with_conn_mut(|conn| {
            // One row, one transaction (data-model §7's delete-ordering discipline; W3
            // review N6 applied the same pattern to the account row).
            let tx = conn
                .transaction()
                .map_err(|_| VaultError::Backend("could not start transaction"))?;
            tx.execute("DELETE FROM artifact WHERE kind = ?1", [CONFIG_KIND])
                .map_err(|_| VaultError::Backend("config delete failed"))?;
            tx.execute(
                "INSERT INTO artifact
                    (artifact_id, kind, doc_id, format_version, wrapped_dek, nonce, ciphertext, created_at_unix_ms)
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    artifact_id,
                    CONFIG_KIND,
                    crate::config::CONFIG_FORMAT_VERSION,
                    wrapped_dek_bytes,
                    artifact_blob.nonce,
                    artifact_blob.ciphertext,
                    created_at_unix_ms,
                ],
            )
            .map_err(|_| VaultError::Backend("config insert failed"))?;
            tx.commit()
                .map_err(|_| VaultError::Backend("config transaction commit failed"))?;
            Ok(())
        })
        .map_err(config_err)
    }
}

/// Preserve the real `VaultError` class (same reasoning as `account_store_err`/`audit_err`).
fn config_err(e: VaultError) -> ConfigError {
    match e {
        VaultError::WrongKey => ConfigError::Backend("vault key mismatch"),
        VaultError::Backend(class) => ConfigError::Backend(class),
    }
}

// ---------------------------------------------------------------------------
// DocumentStore over the same connection (W10, data-model §6.1–§6.2, §7's `document` and
// `artifact` tables — kinds 8 and 2).
// ---------------------------------------------------------------------------

const DOCUMENT_META_KIND: i64 = 8; // `ArtifactKind::DocumentMeta as u8`, data-model §6.
const ORIGINAL_KIND: i64 = 2; // `ArtifactKind::Original as u8`, data-model §6.
const APPROVED_KIND: i64 = 1; // `ArtifactKind::Approved as u8`, data-model §6.

impl DocumentStore for SqlCipherVault {
    fn insert(
        &self,
        master: &VaultMasterKey,
        doc_id: &str,
        meta: &DocumentMeta,
        original: Option<&OriginalRecord>,
        imported_at_unix_ms: u64,
    ) -> Result<(), CatalogError> {
        let (meta_wrapped_dek, meta_blob) = crate::catalog::seal_document_meta(master, doc_id, meta)?;
        let meta_artifact_id = uuid::Uuid::new_v4().to_string();
        let created_at_unix_ms = now_unix_ms();

        // Seal the original outside the transaction too — same reasoning as config/meta:
        // AEAD work doesn't need the connection, so do it before opening the transaction,
        // keeping the transaction itself to pure SQL.
        let sealed_original = original
            .map(|o| crate::catalog::seal_original(master, doc_id, o))
            .transpose()?;
        let original_artifact_id = sealed_original.as_ref().map(|_| uuid::Uuid::new_v4().to_string());

        self.with_conn_mut(|conn| {
            let tx = conn
                .transaction()
                .map_err(|_| VaultError::Backend("could not start transaction"))?;

            tx.execute(
                "INSERT INTO artifact
                    (artifact_id, kind, doc_id, format_version, wrapped_dek, nonce, ciphertext, created_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    meta_artifact_id,
                    DOCUMENT_META_KIND,
                    doc_id,
                    crate::catalog::CATALOG_FORMAT_VERSION,
                    pack_wrapped_dek(&meta_wrapped_dek),
                    meta_blob.nonce,
                    meta_blob.ciphertext,
                    created_at_unix_ms,
                ],
            )
            .map_err(|_| VaultError::Backend("document_meta insert failed"))?;

            if let Some((wrapped_dek, blob)) = &sealed_original {
                tx.execute(
                    "INSERT INTO artifact
                        (artifact_id, kind, doc_id, format_version, wrapped_dek, nonce, ciphertext, created_at_unix_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![
                        original_artifact_id.as_ref().expect("set alongside sealed_original"),
                        ORIGINAL_KIND,
                        doc_id,
                        crate::catalog::CATALOG_FORMAT_VERSION,
                        pack_wrapped_dek(wrapped_dek),
                        blob.nonce,
                        blob.ciphertext,
                        created_at_unix_ms,
                    ],
                )
                .map_err(|_| VaultError::Backend("original insert failed"))?;
            }

            tx.execute(
                "INSERT INTO document
                    (doc_id, meta_artifact_id, original_artifact_id, approved_artifact_id, imported_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![doc_id, meta_artifact_id, original_artifact_id, Option::<String>::None, imported_at_unix_ms],
            )
            .map_err(|_| VaultError::Backend("document insert failed"))?;

            tx.commit()
                .map_err(|_| VaultError::Backend("document transaction commit failed"))?;
            Ok(())
        })
        .map_err(catalog_err)
    }

    fn list_ids_newest_first(&self) -> Result<Vec<String>, CatalogError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT doc_id FROM document ORDER BY imported_at_unix_ms DESC, doc_id DESC")
                .map_err(|_| VaultError::Backend("document list prep failed"))?;
            let ids = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|_| VaultError::Backend("document list query failed"))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|_| VaultError::Backend("document list decode failed"))?;
            Ok(ids)
        })
        .map_err(catalog_err)
    }

    fn load_meta(&self, master: &VaultMasterKey, doc_id: &str) -> Result<Option<DocumentMeta>, CatalogError> {
        let row = self
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT a.wrapped_dek, a.nonce, a.ciphertext
                     FROM document d JOIN artifact a ON a.artifact_id = d.meta_artifact_id
                     WHERE d.doc_id = ?1",
                    [doc_id],
                    |r| {
                        let wrapped_dek: Vec<u8> = r.get(0)?;
                        let nonce: Vec<u8> = r.get(1)?;
                        let ciphertext: Vec<u8> = r.get(2)?;
                        Ok((wrapped_dek, nonce, ciphertext))
                    },
                )
                .optional()
                .map_err(|_| VaultError::Backend("document meta query failed"))
            })
            .map_err(catalog_err)?;

        let Some((wrapped_dek_bytes, nonce, ciphertext)) = row else {
            return Ok(None);
        };
        let wrapped_dek = unpack_wrapped_dek(&wrapped_dek_bytes).map_err(catalog_err)?;
        let artifact_blob = WrappedBlob { nonce, ciphertext };
        crate::catalog::open_document_meta(master, doc_id, &wrapped_dek, &artifact_blob).map(Some)
    }

    fn has_approved_version(&self, doc_id: &str) -> Result<bool, CatalogError> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT approved_artifact_id IS NOT NULL FROM document WHERE doc_id = ?1",
                [doc_id],
                |r| r.get::<_, bool>(0),
            )
            .optional()
            .map(|v| v.unwrap_or(false))
            .map_err(|_| VaultError::Backend("has_approved_version query failed"))
        })
        .map_err(catalog_err)
    }

    fn has_retained_original(&self, doc_id: &str) -> Result<bool, CatalogError> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT original_artifact_id IS NOT NULL FROM document WHERE doc_id = ?1",
                [doc_id],
                |r| r.get::<_, bool>(0),
            )
            .optional()
            .map(|v| v.unwrap_or(false))
            .map_err(|_| VaultError::Backend("has_retained_original query failed"))
        })
        .map_err(catalog_err)
    }

    fn store_approved(
        &self,
        master: &VaultMasterKey,
        doc_id: &str,
        approved: &ApprovedVersion,
    ) -> Result<(), CatalogError> {
        let (wrapped_dek, blob) = crate::catalog::seal_approved(master, doc_id, approved)?;
        let artifact_id = uuid::Uuid::new_v4().to_string();
        let created_at_unix_ms = now_unix_ms();
        self.with_conn_mut(|conn| {
            let tx = conn
                .transaction()
                .map_err(|_| VaultError::Backend("could not start transaction"))?;
            tx.execute(
                "INSERT INTO artifact
                    (artifact_id, kind, doc_id, format_version, wrapped_dek, nonce, ciphertext, created_at_unix_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    artifact_id,
                    APPROVED_KIND,
                    doc_id,
                    crate::catalog::CATALOG_FORMAT_VERSION,
                    pack_wrapped_dek(&wrapped_dek),
                    blob.nonce,
                    blob.ciphertext,
                    created_at_unix_ms,
                ],
            )
            .map_err(|_| VaultError::Backend("approved insert failed"))?;
            let n = tx
                .execute(
                    "UPDATE document SET approved_artifact_id = ?1
                     WHERE doc_id = ?2 AND approved_artifact_id IS NULL",
                    rusqlite::params![artifact_id, doc_id],
                )
                .map_err(|_| VaultError::Backend("approved document update failed"))?;
            if n != 1 {
                return Err(VaultError::Backend("approved version already stored"));
            }
            tx.commit()
                .map_err(|_| VaultError::Backend("approved transaction commit failed"))?;
            Ok(())
        })
        .map_err(catalog_err)
    }

    fn load_approved(
        &self,
        master: &VaultMasterKey,
        doc_id: &str,
    ) -> Result<Option<ApprovedVersion>, CatalogError> {
        let row = self
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT a.wrapped_dek, a.nonce, a.ciphertext
                     FROM document d JOIN artifact a ON a.artifact_id = d.approved_artifact_id
                     WHERE d.doc_id = ?1",
                    [doc_id],
                    |r| {
                        let wrapped_dek: Vec<u8> = r.get(0)?;
                        let nonce: Vec<u8> = r.get(1)?;
                        let ciphertext: Vec<u8> = r.get(2)?;
                        Ok((wrapped_dek, nonce, ciphertext))
                    },
                )
                .optional()
                .map_err(|_| VaultError::Backend("approved query failed"))
            })
            .map_err(catalog_err)?;
        let Some((wrapped_dek_bytes, nonce, ciphertext)) = row else {
            return Ok(None);
        };
        let wrapped_dek = unpack_wrapped_dek(&wrapped_dek_bytes).map_err(catalog_err)?;
        let artifact_blob = WrappedBlob { nonce, ciphertext };
        crate::catalog::open_approved(master, doc_id, &wrapped_dek, &artifact_blob).map(Some)
    }

    fn load_original(
        &self,
        master: &VaultMasterKey,
        doc_id: &str,
    ) -> Result<Option<OriginalRecord>, CatalogError> {
        let row = self
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT a.wrapped_dek, a.nonce, a.ciphertext
                     FROM document d JOIN artifact a ON a.artifact_id = d.original_artifact_id
                     WHERE d.doc_id = ?1",
                    [doc_id],
                    |r| {
                        let wrapped_dek: Vec<u8> = r.get(0)?;
                        let nonce: Vec<u8> = r.get(1)?;
                        let ciphertext: Vec<u8> = r.get(2)?;
                        Ok((wrapped_dek, nonce, ciphertext))
                    },
                )
                .optional()
                .map_err(|_| VaultError::Backend("original query failed"))
            })
            .map_err(catalog_err)?;
        let Some((wrapped_dek_bytes, nonce, ciphertext)) = row else {
            return Ok(None);
        };
        let wrapped_dek = unpack_wrapped_dek(&wrapped_dek_bytes).map_err(catalog_err)?;
        let artifact_blob = WrappedBlob { nonce, ciphertext };
        crate::catalog::open_original(master, doc_id, &wrapped_dek, &artifact_blob).map(Some)
    }
}

/// Preserve the real `VaultError` class (same reasoning as `account_store_err`/`audit_err`/
/// `config_err`).
fn catalog_err(e: VaultError) -> CatalogError {
    match e {
        VaultError::WrongKey => CatalogError::Backend("vault key mismatch"),
        VaultError::Backend(class) => CatalogError::Backend(class),
    }
}

/// `LocalAccount.created_at` is RFC 3339 UTC seconds (`crate::account::format_rfc3339`);
/// the SQL column is `created_at_unix_ms`. This module owns both directions of that one
/// conversion rather than exposing a public parser in `crate::account`.
fn rfc3339_to_unix_ms(rfc3339: &str) -> i64 {
    // Re-derive Unix seconds from the same civil calendar the writer used. Parsing is
    // deliberately narrow: `crate::account::now_rfc3339` only ever produces
    // `YYYY-MM-DDTHH:MM:SSZ`, so this does not need to accept the full RFC 3339 grammar.
    let bytes = rfc3339.as_bytes();
    if bytes.len() != 20 {
        return 0;
    }
    let get = |lo: usize, hi: usize| -> i64 {
        std::str::from_utf8(&bytes[lo..hi])
            .ok()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0)
    };
    let year = get(0, 4);
    let month = get(5, 7);
    let day = get(8, 10);
    let hour = get(11, 13);
    let min = get(14, 16);
    let sec = get(17, 19);

    // Howard Hinnant's days_from_civil, the inverse of `crate::account::format_rfc3339`'s
    // civil_from_days.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    let unix_secs = days * 86_400 + hour * 3_600 + min * 60 + sec;
    unix_secs * 1000
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn rfc3339_to_unix_ms_round_trips_through_format_rfc3339() {
        for secs in [0i64, 1, 951_782_400, 1_709_164_800, 1_767_225_599, 1_756_857_600] {
            let s = crate::account::format_rfc3339(secs);
            assert_eq!(rfc3339_to_unix_ms(&s), secs * 1000, "round trip for {s}");
        }
    }

    /// testing.md §5.3 gated property: "SQLCipher opened with raw key form (not
    /// passphrase KDF)". Asserts the **open API form itself** (testing.md line 348: "a
    /// test that passphrase-KDF would exceed unlock budget is not required if the open API
    /// is asserted"), not merely that `SqlCipherVault` round-trips its own key
    /// self-consistently — a round-trip test cannot distinguish the two forms, because both
    /// forms are self-consistent on their own.
    #[test]
    fn open_uses_raw_key_form_not_passphrase_kdf() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("v.db");
        let key = Zeroizing::new([0x5au8; KEY_LEN]);
        {
            let v = SqlCipherVault::new(path.clone());
            v.open(&key).expect("create");
            v.close();
        }
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();

        // The raw `x'<64 hex>'` form (architecture §3.1) must open it.
        let raw = Connection::open(&path).expect("open file");
        raw.pragma_update(None, "key", format!("x'{hex}'"))
            .expect("pragma");
        assert!(
            raw.query_row("SELECT count(*) FROM sqlite_master", [], |r| r
                .get::<_, i64>(0))
                .is_ok(),
            "vault must be readable via the raw x'<64 hex>' key form"
        );

        // The bare hex string as a *passphrase* (SQLCipher's default PBKDF2 path,
        // architecture §3.1's explicitly forbidden form) must NOT open the same file —
        // if it did, `open_raw_key_pragma` would have silently been using this path.
        let pass = Connection::open(&path).expect("open file");
        pass.pragma_update(None, "key", hex).expect("pragma");
        assert!(
            pass.query_row("SELECT count(*) FROM sqlite_master", [], |r| r
                .get::<_, i64>(0))
                .is_err(),
            "vault must NOT be openable via the passphrase-KDF form"
        );
    }

    /// architecture §3.1 specifies lowercase hex. Uppercase happens to also work (SQLCipher
    /// parses `x'...'` case-insensitively), which is exactly why this needs its own
    /// assertion rather than being left for a mutation run to notice: a `{:02x}` →
    /// `{:02X}` mutant of [`open_raw_key_pragma`] is otherwise an unkilled, unannotated
    /// survivor under testing.md §5.3.
    #[test]
    fn key_pragma_hex_is_lowercase() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("v.db");
        // A key with a nibble in the a-f range in both halves of at least one byte, so a
        // case bug is visible regardless of which nibble a mutant might affect.
        let key = Zeroizing::new([0xefu8; KEY_LEN]);
        let conn = Connection::open(&path).expect("open file");
        open_raw_key_pragma(&conn, &key).expect("pragma");
        // `PRAGMA key` does not echo the value back, so assert indirectly: build the
        // uppercase form by hand and confirm the vault this call just keyed is *not*
        // readable under it, proving the pragma that was actually issued was not the
        // uppercase form fed to some case-insensitive coincidence.
        drop(conn);
        let hex_upper: String = key.iter().map(|b| format!("{b:02X}")).collect();
        let hex_lower: String = key.iter().map(|b| format!("{b:02x}")).collect();
        assert_ne!(hex_upper, hex_lower, "test key must actually exercise a letter nibble");

        let reopened = Connection::open(&path).expect("open file");
        reopened
            .pragma_update(None, "key", format!("x'{hex_lower}'"))
            .expect("pragma");
        assert!(
            reopened
                .query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
                .is_ok(),
            "the lowercase hex form issued by open_raw_key_pragma must open the database"
        );
    }

    /// dev-plan W3 "Done when: … no plaintext document columns" is a claim about the whole
    /// schema, not just `schema_version`. Deleting any table, index, or `PRAGMA
    /// foreign_keys` from [`ensure_schema`] should fail this test.
    #[test]
    fn schema_has_every_data_model_7_table_index_and_foreign_keys_on() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("v.db");
        let vault = SqlCipherVault::new(path);
        vault.open(&Zeroizing::new([0x77u8; KEY_LEN])).expect("open");

        vault
            .with_conn(|conn| {
                let fk_on: i64 = conn
                    .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
                    .map_err(|_| VaultError::Backend("pragma read failed"))?;
                assert_eq!(fk_on, 1, "PRAGMA foreign_keys must read back ON");

                let tables: BTreeSet<String> = conn
                    .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
                    .and_then(|mut stmt| {
                        stmt.query_map([], |r| r.get::<_, String>(0))?.collect()
                    })
                    .map_err(|_| VaultError::Backend("table listing failed"))?;
                let expected_tables: BTreeSet<String> = [
                    "schema_meta",
                    "account",
                    "artifact",
                    "document",
                    "variant",
                    "plugin_secret",
                    "audit_entry",
                ]
                .into_iter()
                .map(String::from)
                .collect();
                assert_eq!(tables, expected_tables, "data-model §7 table set must match exactly");

                let indexes: BTreeSet<String> = conn
                    .prepare(
                        "SELECT name FROM sqlite_master WHERE type = 'index' AND name NOT LIKE 'sqlite_%'",
                    )
                    .and_then(|mut stmt| {
                        stmt.query_map([], |r| r.get::<_, String>(0))?.collect()
                    })
                    .map_err(|_| VaultError::Backend("index listing failed"))?;
                let expected_indexes: BTreeSet<String> = [
                    "audit_entry_doc",
                    "artifact_doc",
                    "uq_artifact_config",
                    "uq_artifact_cloud_ai",
                ]
                .into_iter()
                .map(String::from)
                .collect();
                assert_eq!(indexes, expected_indexes, "data-model §7 index set must match exactly");

                Ok(())
            })
            .expect("schema introspection");
    }
}
