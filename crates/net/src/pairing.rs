//! The pairing protocol: enrol a new device and pin its public key.
//!
//! This module is the deterministic core (no I/O, no clock of its own), so it is exhaustively
//! unit-tested. The transport layer feeds it the current time (as milliseconds) and the peer's
//! transport-authenticated public key; it decides accept/reject and records the pin.
//!
//! See [`proto::pairing`] for why a six-digit code is enough here (the iroh transport already gives
//! an encrypted, key-authenticated channel, so the code only needs to resist online guessing:
//! it expires, is single-use, and locks after a few wrong tries).

use std::{collections::HashSet, fmt, str::FromStr};

use proto::PairRejection;

/// How long a freshly shown code stays valid (five minutes).
pub const CODE_TTL_MS: u64 = 5 * 60 * 1000;
/// How many wrong guesses a code tolerates before it locks.
pub const MAX_ATTEMPTS: u32 = 5;

/// One past the largest code value: codes are six digits, `0..1_000_000`.
const MODULUS: u32 = 1_000_000;

/// A six-digit pairing code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairingCode(u32);

impl PairingCode {
    /// Number of decimal digits in the printed form.
    pub const DIGITS: usize = 6;

    /// Generates a fresh random code, uniformly over `0..1_000_000` (no modulo bias).
    #[must_use]
    pub fn generate() -> Self {
        Self(rand::random_range(0..MODULUS))
    }

    /// Wraps a raw integer if it is a valid six-digit code.
    #[must_use]
    pub fn from_u32(value: u32) -> Option<Self> {
        (value < MODULUS).then_some(Self(value))
    }

    /// The numeric value, in `0..1_000_000`.
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Constant-time equality: compares the whole value with no data-dependent branch, so a timing
    /// side channel cannot leak how many leading digits matched. (The attempt limit is the primary
    /// defence; this removes the secondary timing signal.)
    #[must_use]
    fn ct_eq(self, other: Self) -> bool {
        (self.0 ^ other.0) == 0
    }
}

impl fmt::Display for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:0width$}", self.0, width = Self::DIGITS)
    }
}

/// Error parsing a [`PairingCode`] from text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PairingCodeParseError {
    /// A character other than a digit, space or dash was present.
    #[error("pairing code may contain only digits, spaces and dashes")]
    InvalidCharacter,
    /// The number of digits was not exactly [`PairingCode::DIGITS`].
    #[error("pairing code must be exactly {} digits", PairingCode::DIGITS)]
    WrongLength,
}

impl FromStr for PairingCode {
    type Err = PairingCodeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut value = 0u32;
        let mut digits = 0usize;
        for ch in s.chars() {
            match ch {
                ' ' | '-' => continue,
                '0'..='9' => {
                    digits += 1;
                    if digits > Self::DIGITS {
                        return Err(PairingCodeParseError::WrongLength);
                    }
                    value = value * 10 + (ch as u32 - '0' as u32);
                }
                _ => return Err(PairingCodeParseError::InvalidCharacter),
            }
        }
        if digits == Self::DIGITS {
            Ok(Self(value))
        } else {
            Err(PairingCodeParseError::WrongLength)
        }
    }
}

/// The Console's side of one pairing attempt: the code it is currently showing, and the guard rails
/// around it. Create one when the operator opens the "pair a device" screen; drop it when they close
/// it or a device pairs successfully.
#[derive(Debug)]
pub struct PairingSession {
    code: PairingCode,
    expires_at_ms: u64,
    attempts_left: u32,
    used: bool,
}

impl PairingSession {
    /// Starts a session showing `code`, valid for [`CODE_TTL_MS`] from `now_ms`.
    #[must_use]
    pub fn new(code: PairingCode, now_ms: u64) -> Self {
        Self::with_ttl(code, CODE_TTL_MS, now_ms)
    }

    /// The code this session is showing.
    #[must_use]
    pub fn code(&self) -> PairingCode {
        self.code
    }

    /// Starts a session with an explicit time-to-live (used by tests and future configuration).
    #[must_use]
    pub fn with_ttl(code: PairingCode, ttl_ms: u64, now_ms: u64) -> Self {
        Self {
            code,
            expires_at_ms: now_ms.saturating_add(ttl_ms),
            attempts_left: MAX_ATTEMPTS,
            used: false,
        }
    }

    /// Checks a code a device offered. On `Ok` the caller pins the device's public key; the session
    /// is then spent and rejects any further attempt.
    ///
    /// Order matters: a spent code is [`PairRejection::AlreadyUsed`] (replay), an expired one
    /// [`PairRejection::Expired`], a locked one [`PairRejection::TooManyAttempts`], and only then is
    /// the code compared. A wrong code consumes one attempt.
    ///
    /// # Errors
    /// Returns the reason the attempt was refused.
    pub fn verify(&mut self, attempt: PairingCode, now_ms: u64) -> Result<(), PairRejection> {
        if self.used {
            return Err(PairRejection::AlreadyUsed);
        }
        if now_ms >= self.expires_at_ms {
            return Err(PairRejection::Expired);
        }
        if self.attempts_left == 0 {
            return Err(PairRejection::TooManyAttempts);
        }
        if self.code.ct_eq(attempt) {
            self.used = true;
            Ok(())
        } else {
            self.attempts_left -= 1;
            Err(PairRejection::WrongCode)
        }
    }

    /// How many wrong guesses remain.
    #[must_use]
    pub const fn attempts_left(&self) -> u32 {
        self.attempts_left
    }

    /// Whether the code has completed a successful pairing.
    #[must_use]
    pub const fn is_used(&self) -> bool {
        self.used
    }
}

/// The set of peer public keys this device trusts. Both the Console and the Agent keep one.
///
/// Trust is keyed by the 32-byte public key alone, with **no network address** — so it is inherently
/// address-independent. When a paired device's IP changes, iroh reconnects to the same public key
/// and this store still recognises it; nothing here needs updating. Persisting the store to disk
/// reuses the same protected storage as the device key (added when the Agent service lands).
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrustStore {
    pinned: HashSet<[u8; 32]>,
}

/// Errors loading or saving a [`TrustStore`].
#[derive(Debug, thiserror::Error)]
pub enum TrustStoreError {
    /// Reading or writing the file failed.
    #[error("trust store I/O at {path}: {source}")]
    Io {
        /// The file involved.
        path: std::path::PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The stored file could not be decoded.
    #[error("trust store file is corrupt")]
    Corrupt,
}

impl TrustStore {
    /// An empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Pins a peer's public key as trusted. Idempotent.
    pub fn pin(&mut self, public_key: &[u8; 32]) {
        self.pinned.insert(*public_key);
    }

    /// Removes trust in a peer (e.g. an Org revoked the device).
    pub fn revoke(&mut self, public_key: &[u8; 32]) {
        self.pinned.remove(public_key);
    }

    /// Whether a peer's public key is trusted.
    #[must_use]
    pub fn is_trusted(&self, public_key: &[u8; 32]) -> bool {
        self.pinned.contains(public_key)
    }

    /// Number of trusted peers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pinned.len()
    }

    /// Whether no peers are trusted yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pinned.is_empty()
    }

    /// Every trusted key, for display and for pinning a peer the Console wants to dial.
    pub fn keys(&self) -> impl Iterator<Item = &[u8; 32]> {
        self.pinned.iter()
    }

    /// Loads a store from disk. A missing file is an empty store (first run).
    ///
    /// The contents are public keys, not secrets — but an attacker who can *write* this file grants
    /// themselves control, so it must live in a directory only administrators can write.
    ///
    /// # Errors
    /// Returns [`TrustStoreError`] if the file exists but cannot be read or decoded.
    pub fn load(path: &std::path::Path) -> Result<Self, TrustStoreError> {
        match std::fs::read(path) {
            Ok(bytes) => proto::decode(&bytes).map_err(|_| TrustStoreError::Corrupt),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(source) => Err(TrustStoreError::Io {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Writes the store to disk atomically (temp file, then rename).
    ///
    /// # Errors
    /// Returns [`TrustStoreError`] if the file cannot be written.
    pub fn save(&self, path: &std::path::Path) -> Result<(), TrustStoreError> {
        let io = |source| TrustStoreError::Io {
            path: path.to_path_buf(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, proto::encode(self)).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_000_000; // an arbitrary "now" in ms

    fn code(n: u32) -> PairingCode {
        PairingCode::from_u32(n).unwrap()
    }

    #[test]
    fn correct_code_pairs_once() {
        let mut session = PairingSession::new(code(123_456), T0);
        assert!(session.verify(code(123_456), T0).is_ok());
        assert!(session.is_used());
    }

    #[test]
    fn wrong_code_is_rejected_and_consumes_an_attempt() {
        let mut session = PairingSession::new(code(123_456), T0);
        assert_eq!(
            session.verify(code(000_000), T0),
            Err(PairRejection::WrongCode)
        );
        assert_eq!(session.attempts_left(), MAX_ATTEMPTS - 1);
        assert!(!session.is_used());
    }

    #[test]
    fn code_expires_after_five_minutes() {
        let mut session = PairingSession::new(code(123_456), T0);
        let just_after = T0 + CODE_TTL_MS;
        assert_eq!(
            session.verify(code(123_456), just_after),
            Err(PairRejection::Expired)
        );
    }

    #[test]
    fn valid_just_before_expiry() {
        let mut session = PairingSession::new(code(123_456), T0);
        assert!(session.verify(code(123_456), T0 + CODE_TTL_MS - 1).is_ok());
    }

    #[test]
    fn used_code_rejects_replay() {
        let mut session = PairingSession::new(code(123_456), T0);
        assert!(session.verify(code(123_456), T0).is_ok());
        // A second presentation of the same correct code must not pair again.
        assert_eq!(
            session.verify(code(123_456), T0),
            Err(PairRejection::AlreadyUsed)
        );
    }

    #[test]
    fn locks_after_max_attempts() {
        let mut session = PairingSession::new(code(123_456), T0);
        for _ in 0..MAX_ATTEMPTS {
            assert_eq!(
                session.verify(code(999_999), T0),
                Err(PairRejection::WrongCode)
            );
        }
        // Now locked: even the correct code is refused.
        assert_eq!(
            session.verify(code(123_456), T0),
            Err(PairRejection::TooManyAttempts)
        );
    }

    #[test]
    fn code_display_and_parse_round_trip() {
        let parsed: PairingCode = "007123".parse().unwrap();
        assert_eq!(parsed, code(7123));
        assert_eq!(parsed.to_string(), "007123");
        assert_eq!("12 34-56".parse::<PairingCode>().unwrap(), code(123_456));
    }

    #[test]
    fn code_parse_rejects_wrong_length_and_letters() {
        assert_eq!(
            "123".parse::<PairingCode>(),
            Err(PairingCodeParseError::WrongLength)
        );
        assert_eq!(
            "1234567".parse::<PairingCode>(),
            Err(PairingCodeParseError::WrongLength)
        );
        assert_eq!(
            "12x456".parse::<PairingCode>(),
            Err(PairingCodeParseError::InvalidCharacter)
        );
    }

    #[test]
    fn from_u32_rejects_out_of_range() {
        assert!(PairingCode::from_u32(999_999).is_some());
        assert!(PairingCode::from_u32(1_000_000).is_none());
    }

    #[test]
    fn trust_store_round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("cw-trust-{}", std::process::id()));
        let path = dir.join("trust.bin");
        let _ = std::fs::remove_file(&path);

        // A missing file is simply an empty store.
        assert!(TrustStore::load(&path).unwrap().is_empty());

        let key = *iroh::SecretKey::generate().public().as_bytes();
        let mut store = TrustStore::new();
        store.pin(&key);
        store.save(&path).unwrap();

        let loaded = TrustStore::load(&path).unwrap();
        assert!(loaded.is_trusted(&key), "pins survive a restart");
        assert_eq!(loaded.len(), 1);

        std::fs::write(&path, b"not a trust store").unwrap();
        assert!(matches!(
            TrustStore::load(&path),
            Err(TrustStoreError::Corrupt)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trust_is_by_key_and_survives_address_change() {
        // The transport identifies a paired device by its public key; the store holds no address, so
        // an IP change cannot un-trust it.
        let key = iroh::SecretKey::generate().public();
        let bytes = *key.as_bytes();
        let mut store = TrustStore::new();
        assert!(!store.is_trusted(&bytes));
        store.pin(&bytes);
        assert!(store.is_trusted(&bytes));
        assert_eq!(store.len(), 1);
        // Pinning again is idempotent; a different key is not trusted.
        store.pin(&bytes);
        assert_eq!(store.len(), 1);
        let other = *iroh::SecretKey::generate().public().as_bytes();
        assert!(!store.is_trusted(&other));
        store.revoke(&bytes);
        assert!(!store.is_trusted(&bytes));
    }
}
