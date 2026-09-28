//! The **break-glass** emergency code (AGENTS.md D10).
//!
//! When something goes wrong — a lesson has to be stopped, a teacher is unreachable — an admin can
//! type the break-glass code on the student PC (`cowatcher-agent unlock <code>`) to **pause
//! enforcement** (the games blocklist) for a few minutes. There is no hardcoded master code: the
//! Console generates a random code, keeps it, and provisions only its **Argon2id hash** here, exactly
//! like the room password. Every attempt is audit-logged and wrong ones are rate-limited so the code
//! cannot be ground down by guessing.
//!
//! The honest limit is the same as everywhere else: this stops a *student*, not a local administrator
//! (who can stop the service outright). That is an organisational control, not a cryptographic one.

use std::path::{Path, PathBuf};

use net::RoomSecret;

/// Wrong attempts allowed before the lockout applies.
const MAX_ATTEMPTS: u32 = 5;
/// How long to refuse attempts after [`MAX_ATTEMPTS`] wrong ones.
const LOCKOUT_SECONDS: u64 = 60;
/// The longest a single break-glass unlock may pause enforcement for.
const MAX_PAUSE_MINUTES: u64 = 240;
/// Used when the caller does not name a duration.
pub const DEFAULT_PAUSE_MINUTES: u64 = 15;

/// The file holding the Argon2id hash of the break-glass code.
fn hash_path(dir: &Path) -> PathBuf {
    dir.join("breakglass.hash")
}
/// The file recording failed unlock attempts (so a reboot does not reset the lockout).
fn attempts_path(dir: &Path) -> PathBuf {
    dir.join("breakglass-attempts.txt")
}
/// The file naming the Unix time (seconds) until which enforcement is paused.
pub fn pause_path(dir: &Path) -> PathBuf {
    dir.join("breakglass.until")
}

/// Stores the break-glass hash provisioned by the Console.
///
/// # Errors
/// I/O error if the file cannot be written.
pub fn store_hash(dir: &Path, hash: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(hash_path(dir), hash)
}

/// Whether enforcement is currently paused (a break-glass unlock is in effect) at `now_s`.
#[must_use]
pub fn is_paused(dir: &Path, now_s: u64) -> bool {
    std::fs::read_to_string(pause_path(dir))
        .ok()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .is_some_and(|until| now_s < until)
}

/// Why a break-glass unlock was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnlockError {
    /// No break-glass code has been provisioned on this PC yet.
    #[error("no break-glass code is set on this PC")]
    NotSet,
    /// The code did not match.
    #[error("wrong break-glass code")]
    WrongCode,
    /// Too many wrong attempts recently.
    #[error("too many wrong attempts; try again in {seconds} s")]
    LockedOut {
        /// Seconds still to wait.
        seconds: u64,
    },
    /// The stored hash is unusable.
    #[error("the stored break-glass code is unreadable")]
    CorruptHash,
    /// Files could not be updated.
    #[error("could not update this PC's files: {0}")]
    Io(String),
}

fn read_attempts(dir: &Path) -> (u32, u64) {
    std::fs::read_to_string(attempts_path(dir))
        .ok()
        .and_then(|t| {
            let mut parts = t.split_whitespace();
            Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
        })
        .unwrap_or((0, 0))
}

/// Seconds still to wait before another attempt is allowed. Pure, so the rule is tested without waiting.
#[must_use]
pub fn lockout_remaining(failures: u32, last_failure_s: u64, now_s: u64) -> u64 {
    if failures < MAX_ATTEMPTS {
        return 0;
    }
    LOCKOUT_SECONDS.saturating_sub(now_s.saturating_sub(last_failure_s))
}

/// Redeems the break-glass code: on success, pauses enforcement for `minutes` (clamped) and returns
/// the Unix time (seconds) the pause lasts until. Wrong attempts are counted and rate-limited.
///
/// # Errors
/// See [`UnlockError`].
pub fn redeem(dir: &Path, code: &str, minutes: u64, now_s: u64) -> Result<u64, UnlockError> {
    let stored = std::fs::read_to_string(hash_path(dir)).map_err(|_| UnlockError::NotSet)?;
    if stored.trim().is_empty() {
        return Err(UnlockError::NotSet);
    }

    let (failures, last) = read_attempts(dir);
    let remaining = lockout_remaining(failures, last, now_s);
    if remaining > 0 {
        return Err(UnlockError::LockedOut { seconds: remaining });
    }

    match RoomSecret::from_stored(stored.trim()).verify(code) {
        Ok(()) => {
            let minutes = minutes.clamp(1, MAX_PAUSE_MINUTES);
            let until = now_s.saturating_add(minutes.saturating_mul(60));
            std::fs::write(pause_path(dir), until.to_string())
                .map_err(|e| UnlockError::Io(e.to_string()))?;
            let _ = std::fs::remove_file(attempts_path(dir)); // clean slate after a correct code
            Ok(until)
        }
        Err(net::RoomError::CorruptHash) => Err(UnlockError::CorruptHash),
        Err(_) => {
            let _ = std::fs::write(
                attempts_path(dir),
                format!("{} {now_s}", failures.saturating_add(1)),
            );
            Err(UnlockError::WrongCode)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cw-bg-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn provision(dir: &Path, code: &str) {
        let hash = net::RoomPassword::from_text(code)
            .hash()
            .expect("hash")
            .as_str()
            .to_string();
        store_hash(dir, &hash).expect("store");
    }

    #[test]
    fn no_code_set_is_reported() {
        let dir = temp("none");
        assert_eq!(redeem(&dir, "ANY", 15, 0), Err(UnlockError::NotSet));
        assert!(!is_paused(&dir, 0));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_right_code_pauses_enforcement() {
        let dir = temp("ok");
        provision(&dir, "K7M2Q9XR4T6B");
        assert!(!is_paused(&dir, 100));
        let until = redeem(&dir, "K7M2Q9XR4T6B", 15, 100).expect("redeem");
        assert_eq!(until, 100 + 15 * 60);
        assert!(is_paused(&dir, 100));
        assert!(is_paused(&dir, until - 1));
        assert!(
            !is_paused(&dir, until),
            "the pause ends exactly at its deadline"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pause_duration_is_clamped() {
        let dir = temp("clamp");
        provision(&dir, "K7M2Q9XR4T6B");
        let until = redeem(&dir, "K7M2Q9XR4T6B", 99999, 0).expect("redeem");
        assert_eq!(until, MAX_PAUSE_MINUTES * 60);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guessing_is_locked_out_after_five_tries_and_recovers() {
        let dir = temp("lock");
        provision(&dir, "K7M2Q9XR4T6B");
        for _ in 0..MAX_ATTEMPTS {
            assert_eq!(redeem(&dir, "NOPE", 15, 100), Err(UnlockError::WrongCode));
        }
        // Even the correct code waits out the lockout.
        assert_eq!(
            redeem(&dir, "K7M2Q9XR4T6B", 15, 100),
            Err(UnlockError::LockedOut {
                seconds: LOCKOUT_SECONDS
            })
        );
        assert!(
            !is_paused(&dir, 100),
            "a locked-out attempt never paused anything"
        );
        // ... and works once the lockout has passed.
        assert!(redeem(&dir, "K7M2Q9XR4T6B", 15, 100 + LOCKOUT_SECONDS).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
