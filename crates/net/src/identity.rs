//! The device's long-lived cryptographic identity.
//!
//! Every device — Console, Agent or Hub — owns one Ed25519 keypair, created on first run and reused
//! forever after. The public key is the device's real identity in the transport handshake; the
//! [`proto::DeviceId`] is only a short handle derived from it.
//!
//! The secret key is the iroh endpoint's secret key, so we do not invent a second keypair. It is
//! stored with OS protection: DPAPI on Windows, owner-only file permissions elsewhere. The key is
//! never logged and never leaves the device.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use iroh::{PublicKey, SecretKey};

/// A device's persistent keypair, plus its derived [`proto::DeviceId`].
#[derive(Debug, Clone)]
pub struct Identity {
    secret_key: SecretKey,
}

/// Errors from loading or creating an [`Identity`].
#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    /// Reading or writing the key file failed.
    #[error("device key I/O at {path}: {source}")]
    Io {
        /// The file being read or written.
        path: PathBuf,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The stored key was the wrong size after unprotection (corrupt or tampered file).
    #[error("device key file is corrupt: expected 32 bytes, found {0}")]
    Corrupt(usize),
    /// The OS key-protection call (DPAPI on Windows) failed.
    #[error("OS key protection failed: {0}")]
    Protection(#[from] platform::secret::SecretError),
}

impl Identity {
    /// Loads the device identity from `path`, creating and persisting a new one if the file is
    /// absent.
    ///
    /// The parent directory must already exist and be access-controlled by the caller (the Agent
    /// runs as SYSTEM and keeps its key under a protected program-data directory). Writing is
    /// atomic: a fresh key is written to a temporary file and renamed into place, so a crash cannot
    /// leave a half-written key.
    pub fn load_or_create(path: &Path) -> Result<Self, IdentityError> {
        match fs::read(path) {
            Ok(protected) => {
                let bytes = platform::secret::unprotect(&protected)?;
                let key: [u8; 32] = bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| IdentityError::Corrupt(bytes.len()))?;
                Ok(Self {
                    secret_key: SecretKey::from_bytes(&key),
                })
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Self::create(path),
            Err(source) => Err(IdentityError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    fn create(path: &Path) -> Result<Self, IdentityError> {
        let secret_key = SecretKey::generate();
        let protected = platform::secret::protect(&secret_key.to_bytes())?;
        write_atomic(path, &protected)?;
        Ok(Self { secret_key })
    }

    /// The iroh secret key, used to build the endpoint. Never log or serialize this.
    #[must_use]
    pub fn secret_key(&self) -> &SecretKey {
        &self.secret_key
    }

    /// The device's public key: its real, verifiable identity.
    #[must_use]
    pub fn public_key(&self) -> PublicKey {
        self.secret_key.public()
    }

    /// The short, human-typable handle derived from the public key.
    #[must_use]
    pub fn device_id(&self) -> proto::DeviceId {
        proto::DeviceId::from_public_key(self.public_key().as_bytes())
    }
}

/// Writes `data` to `path` atomically: to a sibling temp file, then rename over the target.
fn write_atomic(path: &Path, data: &[u8]) -> Result<(), IdentityError> {
    let io = |source| IdentityError::Io {
        path: path.to_path_buf(),
        source,
    };
    let tmp = path.with_extension("tmp");
    {
        let mut file = fs::File::create(&tmp).map_err(io)?;
        restrict_permissions(&file);
        file.write_all(data).map_err(io)?;
        file.sync_all().map_err(io)?;
    }
    fs::rename(&tmp, path).map_err(io)
}

#[cfg(unix)]
fn restrict_permissions(file: &fs::File) {
    use std::os::unix::fs::PermissionsExt;
    // Owner read/write only (0600). Best-effort: if it fails the write still succeeds, but the
    // key would be world-readable, so a hardened deployment should verify the directory ACL too.
    let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_file: &fs::File) {
    // Windows relies on DPAPI (the bytes on disk are already encrypted) plus the directory ACL.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_then_reloads_the_same_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.key");

        let created = Identity::load_or_create(&path).unwrap();
        assert!(path.exists(), "the key file must be persisted");

        let reloaded = Identity::load_or_create(&path).unwrap();
        assert_eq!(
            created.public_key(),
            reloaded.public_key(),
            "key must survive a restart"
        );
        assert_eq!(
            created.device_id(),
            reloaded.device_id(),
            "device id must be stable"
        );
    }

    #[test]
    fn device_id_matches_the_public_key() {
        let dir = tempfile::tempdir().unwrap();
        let identity = Identity::load_or_create(&dir.path().join("device.key")).unwrap();
        let expected = proto::DeviceId::from_public_key(identity.public_key().as_bytes());
        assert_eq!(identity.device_id(), expected);
    }

    #[test]
    fn on_disk_key_is_not_the_raw_secret() {
        // The file must be protected, never the plaintext key. On Windows DPAPI encrypts it (so the
        // stored blob differs from and is longer than the 32 raw bytes); elsewhere this is a no-op
        // and the check is skipped.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.key");
        let identity = Identity::load_or_create(&path).unwrap();
        let on_disk = fs::read(&path).unwrap();
        if cfg!(windows) {
            assert_ne!(
                on_disk.as_slice(),
                identity.secret_key().to_bytes().as_slice()
            );
            assert!(on_disk.len() > 32);
        }
    }

    #[test]
    fn corrupt_file_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.key");
        // Non-Windows storage is identity, so a wrong-length file surfaces as Corrupt. On Windows a
        // random blob fails DPAPI first (Protection); both are clean, non-panicking errors.
        fs::write(&path, [1u8, 2, 3]).unwrap();
        assert!(Identity::load_or_create(&path).is_err());
    }
}
