//! Linux fallback keystore backend (`architecture.md` §3.2).
//!
//! > **Linux fallback:** if Secret Service is unavailable, persist the same
//! > `KeystoreItem` as a `0600` file under the app-data directory, written via temp-file
//! > + `fsync` + atomic `rename`. This is weaker: the wrapped blob sits next to the DB,
//! > so a stolen app-data directory is one artifact, not two, and **anti-truncation
//! > (§6.2) does not survive a coordinated rollback of `vault.db` together with the
//! > fallback file**. Passphrase wrapping still holds.
//!
//! The write protocol matters more than it looks. A plain `write()` over the existing
//! file has a window in which the only copy of the wrapped master key is a truncated
//! prefix — a crash there **bricks the vault**, which is exactly the failure class
//! decision 0004's dev-log flagged. So: write a sibling temp file, `fsync` it, `rename`
//! it over the target (atomic within a directory on POSIX), then `fsync` the directory so
//! the rename itself is durable. A crash at any point leaves either the complete old item
//! or the complete new one.
//!
//! Deciding *whether* to use this backend instead of [`super::OsKeystore`] (probing
//! Secret Service) is **W7**, not W2.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::{KeystoreBackend, KeystoreBackendKind, KeystoreError, KeystoreItem};

/// Filename used under the app-data directory when the caller does not name one.
pub const FALLBACK_FILE_NAME: &str = "keystore.json";

/// A `KeystoreItem` persisted as a `0600` file.
#[derive(Debug, Clone)]
pub struct FileKeystore {
    path: PathBuf,
}

impl FileKeystore {
    /// Use `path` as the keystore file. The parent directory must exist.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Use the conventional [`FALLBACK_FILE_NAME`] inside `app_data_dir`.
    #[must_use]
    pub fn in_dir(app_data_dir: impl AsRef<Path>) -> Self {
        Self::new(app_data_dir.as_ref().join(FALLBACK_FILE_NAME))
    }

    /// The file this backend reads and writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn temp_path(&self) -> PathBuf {
        let mut name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| FALLBACK_FILE_NAME.to_string());
        name.push_str(".tmp");
        self.path.with_file_name(name)
    }
}

/// Open the temp file with `0600` from the moment it exists, rather than creating it
/// world-readable and chmod-ing afterwards — that ordering has a window in which another
/// local user can open the descriptor and keep reading it after the mode change.
#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> std::io::Result<File> {
    // Non-Unix targets get the platform default ACL; on Windows the real backend is the
    // Credential Manager (`OsKeystore`) and this fallback is not the intended path.
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

impl KeystoreBackend for FileKeystore {
    fn load(&self) -> Result<Option<KeystoreItem>, KeystoreError> {
        match fs::read(&self.path) {
            Ok(bytes) => KeystoreItem::from_bytes(&bytes).map(Some),
            // The *only* condition that may report "no account".
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(KeystoreError::Backend("keystore file read failed")),
        }
    }

    fn store(&self, item: &KeystoreItem) -> Result<(), KeystoreError> {
        let bytes = item.to_bytes();
        let tmp = self.temp_path();

        let write = |tmp: &Path| -> std::io::Result<()> {
            let mut f = create_private(tmp)?;
            f.write_all(&bytes)?;
            f.sync_all()?; // fsync: contents durable before the rename exposes them
            Ok(())
        };
        if write(&tmp).is_err() {
            let _ = fs::remove_file(&tmp);
            return Err(KeystoreError::Backend("keystore temp write failed"));
        }

        if fs::rename(&tmp, &self.path).is_err() {
            let _ = fs::remove_file(&tmp);
            return Err(KeystoreError::Backend("keystore rename failed"));
        }

        // fsync the directory so the rename survives a power loss too. Best effort: some
        // filesystems refuse to open a directory for this, and the rename is already
        // atomic with respect to a crash of *this process*.
        if let Some(dir) = self.path.parent() {
            if let Ok(d) = File::open(dir) {
                let _ = d.sync_all();
            }
        }
        Ok(())
    }

    fn delete(&self) -> Result<(), KeystoreError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(KeystoreError::Backend("keystore file delete failed")),
        }
    }

    fn kind(&self) -> KeystoreBackendKind {
        KeystoreBackendKind::FileFallback
    }
}
