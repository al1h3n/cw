//! At-rest protection for small secrets (device keys).
//!
//! On Windows this is DPAPI; on other platforms it is a no-op and the caller must rely on file
//! permissions (0600) instead. Callers pass raw bytes to [`protect`] before writing to disk and run
//! the stored bytes back through [`unprotect`] on load.

/// Error from the OS secret-protection API.
#[derive(Debug, thiserror::Error)]
#[error("OS secret protection failed: {0}")]
pub struct SecretError(String);

impl SecretError {
    /// The human-readable reason the OS reported.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.0
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_LOCAL_MACHINE, CRYPTPROTECT_UI_FORBIDDEN,
            CryptProtectData, CryptUnprotectData,
        },
    };

    use super::SecretError;

    // Machine scope so the SYSTEM Agent service can read the key regardless of which user is logged
    // in; UI_FORBIDDEN so DPAPI never blocks on a prompt in a service context.
    // ponytail: no extra entropy yet; add an app-specific salt to bind the blob to the app if wanted.
    const FLAGS: u32 = CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN;

    pub fn protect(plaintext: &[u8]) -> Result<Vec<u8>, SecretError> {
        crypt(plaintext, false)
    }

    pub fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>, SecretError> {
        crypt(ciphertext, true)
    }

    fn crypt(input: &[u8], decrypt: bool) -> Result<Vec<u8>, SecretError> {
        let in_blob = CRYPT_INTEGER_BLOB {
            cbData: input.len() as u32,
            pbData: input.as_ptr().cast_mut(),
        };
        let mut out_blob = CRYPT_INTEGER_BLOB::default();
        // SAFETY: in_blob points at `input` for the whole call; out_blob is a valid zeroed blob that
        // DPAPI fills with a LocalAlloc'd buffer, which we copy out and free below.
        let result = unsafe {
            if decrypt {
                CryptUnprotectData(&in_blob, None, None, None, None, FLAGS, &mut out_blob)
            } else {
                CryptProtectData(&in_blob, None, None, None, None, FLAGS, &mut out_blob)
            }
        };
        result.map_err(|e| SecretError(e.message()))?;

        // SAFETY: on success DPAPI set pbData/cbData to a buffer it owns; copy it, then LocalFree it.
        let output = unsafe {
            let slice =
                std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize).to_vec();
            let _ = LocalFree(Some(HLOCAL(out_blob.pbData.cast())));
            slice
        };
        Ok(output)
    }
}

#[cfg(not(windows))]
mod imp {
    use super::SecretError;

    pub fn protect(plaintext: &[u8]) -> Result<Vec<u8>, SecretError> {
        Ok(plaintext.to_vec())
    }

    pub fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>, SecretError> {
        Ok(ciphertext.to_vec())
    }
}

/// Protects `plaintext` for storage on disk. On Windows the result is a DPAPI blob; elsewhere it is
/// the input unchanged (protection is then the file's 0600 permissions).
pub fn protect(plaintext: &[u8]) -> Result<Vec<u8>, SecretError> {
    imp::protect(plaintext)
}

/// Reverses [`protect`], recovering the original bytes.
pub fn unprotect(ciphertext: &[u8]) -> Result<Vec<u8>, SecretError> {
    imp::unprotect(ciphertext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_recovers_input() {
        let secret = [3u8; 32];
        let protected = protect(&secret).unwrap();
        assert_eq!(unprotect(&protected).unwrap(), secret);
    }

    #[cfg(windows)]
    #[test]
    fn windows_output_is_encrypted() {
        let secret = [3u8; 32];
        let protected = protect(&secret).unwrap();
        assert_ne!(protected.as_slice(), secret.as_slice());
        assert!(protected.len() > secret.len());
    }
}
