//! Which room this PC belongs to, and the password needed to take it out again.
//!
//! Joining is one click for the teacher. Leaving is not: a student must not be able to unenrol
//! their own PC in the middle of a lesson, so [`leave`] refuses without the room password.
//!
//! Only the Argon2id **hash** is stored here, so reading this file teaches a curious student
//! nothing. Every attempt — right or wrong — goes to the audit log, and wrong attempts are
//! rate-limited so the password cannot be ground down by guessing.
//!
//! The honest limit, also stated in `docs/FEATURES.md`: none of this stops a local **administrator**,
//! who can stop the service and delete the files. Students must not have admin rights; that is an
//! organisational control, not one software can supply.

use std::path::{Path, PathBuf};

use net::RoomSecret;

/// How long to refuse attempts after [`MAX_ATTEMPTS`] wrong ones in a row.
const LOCKOUT_SECONDS: u64 = 60;
/// Wrong attempts allowed before the lockout applies.
const MAX_ATTEMPTS: u32 = 5;

/// This device's room membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    /// The room name, for display.
    pub room: String,
    /// The Argon2id hash of the password required to leave.
    pub secret: RoomSecret,
}

/// Where the membership file lives in the Agent's data directory.
#[must_use]
pub fn path(dir: &Path) -> PathBuf {
    dir.join("room.txt")
}

/// The file recording failed leave attempts, so a reboot does not reset the lockout.
fn attempts_path(dir: &Path) -> PathBuf {
    dir.join("leave-attempts.txt")
}

/// Reads this device's membership, if it is in a room.
#[must_use]
pub fn load(dir: &Path) -> Option<Membership> {
    let text = std::fs::read_to_string(path(dir)).ok()?;
    let mut lines = text.lines();
    let room = lines.next()?.to_string();
    let secret = lines.next()?.to_string();
    if room.is_empty() || secret.is_empty() {
        return None;
    }
    Some(Membership {
        room,
        secret: RoomSecret::from_stored(&secret),
    })
}

/// Records the room this device has joined.
///
/// # Errors
/// Returns an I/O error if the file cannot be written.
pub fn save(dir: &Path, welcome: &proto::Welcome) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        path(dir),
        format!("{}\n{}", welcome.room, welcome.room_secret),
    )?;
    let _ = std::fs::remove_file(attempts_path(dir)); // a fresh join starts with a clean slate
    Ok(())
}

/// Why leaving a room was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LeaveError {
    /// This device is not in a room, so there is nothing to leave.
    #[error("this PC is not in a room")]
    NotInARoom,
    /// The password did not match.
    #[error("wrong room password")]
    WrongPassword,
    /// Too many wrong attempts recently.
    #[error("too many wrong attempts; try again in {seconds} s")]
    LockedOut {
        /// Seconds still to wait.
        seconds: u64,
    },
    /// The stored hash is unusable.
    #[error("the stored room password is unreadable; the PC must be re-enrolled")]
    CorruptSecret,
    /// Files could not be updated.
    #[error("could not update this PC's files: {0}")]
    Io(String),
}

/// How many wrong attempts have been made, and when the last one was.
fn read_attempts(dir: &Path) -> (u32, u64) {
    std::fs::read_to_string(attempts_path(dir))
        .ok()
        .and_then(|t| {
            let mut parts = t.split_whitespace();
            Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
        })
        .unwrap_or((0, 0))
}

fn write_attempts(dir: &Path, count: u32, at_s: u64) {
    let _ = std::fs::write(attempts_path(dir), format!("{count} {at_s}"));
}

/// Decides whether an attempt is allowed right now, given the failure history.
///
/// Pure so the lockout rule can be tested without waiting a minute.
#[must_use]
pub fn lockout_remaining(failures: u32, last_failure_s: u64, now_s: u64) -> u64 {
    if failures < MAX_ATTEMPTS {
        return 0;
    }
    let elapsed = now_s.saturating_sub(last_failure_s);
    LOCKOUT_SECONDS.saturating_sub(elapsed)
}

/// Takes this device out of its room, if the password is right.
///
/// On success the membership file **and the trust store** are removed, so the device no longer
/// serves any Console until it is paired again.
///
/// # Errors
/// See [`LeaveError`].
pub fn leave(dir: &Path, password: &str, now_s: u64) -> Result<String, LeaveError> {
    let membership = load(dir).ok_or(LeaveError::NotInARoom)?;

    let (failures, last) = read_attempts(dir);
    let remaining = lockout_remaining(failures, last, now_s);
    if remaining > 0 {
        return Err(LeaveError::LockedOut { seconds: remaining });
    }

    match membership.secret.verify(password) {
        Ok(()) => {
            std::fs::remove_file(path(dir)).map_err(|e| LeaveError::Io(e.to_string()))?;
            // Forget every paired Console too: leaving the room means leaving properly.
            let _ = std::fs::remove_file(dir.join("trust.bin"));
            let _ = std::fs::remove_file(attempts_path(dir));
            Ok(membership.room)
        }
        Err(net::RoomError::CorruptHash) => Err(LeaveError::CorruptSecret),
        Err(_) => {
            // Count this failure, and start the clock from the newest one.
            write_attempts(dir, failures.saturating_add(1), now_s);
            Err(LeaveError::WrongPassword)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cw-member-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn welcome_for(password: &str) -> proto::Welcome {
        proto::Welcome {
            room: "Lab 2".to_string(),
            room_secret: net::RoomPassword::from_text(password)
                .hash()
                .expect("hash")
                .as_str()
                .to_string(),
        }
    }

    #[test]
    fn a_device_with_no_file_is_in_no_room() {
        let dir = temp_dir("none");
        assert_eq!(load(&dir), None);
        assert_eq!(leave(&dir, "anything", 0), Err(LeaveError::NotInARoom));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn joining_records_the_room_and_survives_a_restart() {
        let dir = temp_dir("join");
        save(&dir, &welcome_for("K7M2Q9XR4T6B")).expect("save");
        let loaded = load(&dir).expect("loaded");
        assert_eq!(loaded.room, "Lab 2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_password_itself_is_never_written_to_disk() {
        let dir = temp_dir("sealed");
        save(&dir, &welcome_for("K7M2Q9XR4T6B")).expect("save");
        let text = std::fs::read_to_string(path(&dir)).expect("read");
        assert!(!text.contains("K7M2Q9XR4T6B"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_right_password_takes_the_device_out_of_the_room() {
        let dir = temp_dir("leave-ok");
        std::fs::write(dir.join("trust.bin"), b"pretend trust store").unwrap();
        save(&dir, &welcome_for("K7M2Q9XR4T6B")).expect("save");

        assert_eq!(leave(&dir, "K7M2Q9XR4T6B", 0), Ok("Lab 2".to_string()));
        assert_eq!(load(&dir), None, "membership is gone");
        assert!(
            !dir.join("trust.bin").exists(),
            "leaving also forgets paired consoles"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_password_leaves_the_device_exactly_where_it_was() {
        let dir = temp_dir("leave-bad");
        save(&dir, &welcome_for("K7M2Q9XR4T6B")).expect("save");
        assert_eq!(leave(&dir, "GUESS", 0), Err(LeaveError::WrongPassword));
        assert!(load(&dir).is_some(), "still a member");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn guessing_is_locked_out_after_five_tries_and_recovers_later() {
        let dir = temp_dir("lockout");
        save(&dir, &welcome_for("K7M2Q9XR4T6B")).expect("save");
        for _ in 0..MAX_ATTEMPTS {
            assert_eq!(leave(&dir, "GUESS", 100), Err(LeaveError::WrongPassword));
        }
        // Even the *correct* password waits out the lockout: no timing side door.
        assert_eq!(
            leave(&dir, "K7M2Q9XR4T6B", 100),
            Err(LeaveError::LockedOut {
                seconds: LOCKOUT_SECONDS
            })
        );
        // ... and works once the lockout has passed.
        assert_eq!(
            leave(&dir, "K7M2Q9XR4T6B", 100 + LOCKOUT_SECONDS),
            Ok("Lab 2".to_string())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_lockout_rule_itself_is_simple_and_bounded() {
        assert_eq!(lockout_remaining(0, 0, 0), 0);
        assert_eq!(lockout_remaining(MAX_ATTEMPTS - 1, 100, 100), 0);
        assert_eq!(lockout_remaining(MAX_ATTEMPTS, 100, 100), LOCKOUT_SECONDS);
        assert_eq!(
            lockout_remaining(MAX_ATTEMPTS, 100, 130),
            LOCKOUT_SECONDS - 30
        );
        assert_eq!(lockout_remaining(MAX_ATTEMPTS, 100, 999), 0);
        // A clock that jumps backwards must not create a permanent lockout.
        assert_eq!(lockout_remaining(MAX_ATTEMPTS, 100, 50), LOCKOUT_SECONDS);
    }

    #[test]
    fn a_corrupt_stored_secret_says_so_instead_of_pretending_the_password_is_wrong() {
        let dir = temp_dir("corrupt");
        std::fs::write(path(&dir), "Lab 2\nnot-a-real-hash").unwrap();
        assert_eq!(leave(&dir, "anything", 0), Err(LeaveError::CorruptSecret));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
