//! The Console's side of a room: its name, and the password a device needs in order to leave.
//!
//! The Console is the authority here, so it is the one place the room password exists in readable
//! form — a teacher has to be able to look it up again months later. It is therefore stored
//! **encrypted with DPAPI** (the same protection as the device key, AGENTS.md §5), so the file is
//! useless if copied to another machine or another Windows account.
//!
//! What goes out to a student PC is only the Argon2id *hash* (`net::RoomSecret`). The password
//! itself never leaves this machine.

use std::path::{Path, PathBuf};

use net::{RoomPassword, RoomSecret};

/// The default room name until a teacher renames it.
const DEFAULT_NAME: &str = "Classroom";

/// A room as the Console knows it.
#[derive(Debug, Clone)]
pub struct Room {
    /// What the teacher calls this room.
    pub name: String,
    /// The password, in the clear, for showing to the teacher. Never sent anywhere.
    password: RoomPassword,
}

impl Room {
    /// The password, grouped for reading aloud: `K7M2-Q9XR-4T6B`.
    #[must_use]
    pub fn password_grouped(&self) -> String {
        self.password.grouped()
    }

    /// The password exactly as it must be typed.
    #[must_use]
    pub fn password(&self) -> &str {
        self.password.as_str()
    }

    /// The Argon2id hash to hand a joining device.
    ///
    /// # Errors
    /// Returns a message if hashing fails.
    pub fn secret(&self) -> Result<RoomSecret, String> {
        self.password.hash().map_err(|e| e.to_string())
    }

    /// The welcome message sent to a device as it joins.
    ///
    /// # Errors
    /// Returns a message if hashing fails.
    pub fn welcome(&self) -> Result<proto::Welcome, String> {
        Ok(proto::Welcome {
            room: self.name.clone(),
            room_secret: self.secret()?.as_str().to_string(),
        })
    }
}

/// Where the room file lives inside the Console's data directory.
#[must_use]
pub fn path(dir: &Path) -> PathBuf {
    dir.join("room.dat")
}

/// Loads the room, creating one with a fresh random password on first run.
///
/// A teacher never has to invent a password, and there is no default password to leak — the room is
/// born with a strong random one (AGENTS.md D10: no hardcoded master password, ever).
///
/// # Errors
/// Returns a message if the file exists but cannot be read or decrypted.
pub fn load_or_create(dir: &Path) -> Result<Room, String> {
    let file = path(dir);
    if file.exists() {
        let sealed = std::fs::read(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
        let plain = platform::secret::unprotect(&sealed)
            .map_err(|e| format!("the room file could not be decrypted: {}", e.message()))?;
        let text = String::from_utf8(plain).map_err(|_| "the room file is corrupt".to_string())?;
        // Format: first line the name, second line the password.
        let mut lines = text.lines();
        let name = lines.next().unwrap_or_default().to_string();
        let password = lines.next().unwrap_or_default().to_string();
        if !name.is_empty() && !password.is_empty() {
            return Ok(Room {
                name,
                password: RoomPassword::from_text(&password),
            });
        }
        return Err("the room file is corrupt".into());
    }

    let room = Room {
        name: DEFAULT_NAME.to_string(),
        password: RoomPassword::generate(),
    };
    save(dir, &room)?;
    Ok(room)
}

/// Writes the room back, encrypted.
///
/// # Errors
/// Returns a message if encryption or the write fails.
pub fn save(dir: &Path, room: &Room) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let text = format!("{}\n{}", room.name, room.password.as_str());
    let sealed = platform::secret::protect(text.as_bytes())
        .map_err(|e| format!("could not protect the room file: {}", e.message()))?;
    let file = path(dir);
    // Write-then-rename so a crash never leaves a half-written room file.
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, sealed).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &file).map_err(|e| format!("replace {}: {e}", file.display()))
}

/// Renames the room, keeping the same password.
///
/// # Errors
/// Returns a message if the name is unusable or the save fails.
pub fn rename(dir: &Path, room: &Room, new_name: &str) -> Result<Room, String> {
    let name = new_name.trim();
    if name.is_empty() {
        return Err("the room needs a name".into());
    }
    if name.len() > proto::MAX_ROOM_NAME {
        return Err(format!(
            "the room name may be at most {} characters",
            proto::MAX_ROOM_NAME
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("the room name may not contain control characters".into());
    }
    let updated = Room {
        name: name.to_string(),
        password: room.password.clone(),
    };
    save(dir, &updated)?;
    Ok(updated)
}

/// Replaces the room password with a fresh random one.
///
/// Devices already in the room keep the **old** hash until they are re-invited, so changing the
/// password does not silently strand them — the Console shows which devices still hold the old one.
///
/// # Errors
/// Returns a message if the save fails.
pub fn regenerate_password(dir: &Path, room: &Room) -> Result<Room, String> {
    let updated = Room {
        name: room.name.clone(),
        password: RoomPassword::generate(),
    };
    save(dir, &updated)?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cw-room-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_new_room_gets_a_strong_random_password_with_no_default() {
        let dir = temp_dir("new");
        let a = load_or_create(&dir).expect("create");
        assert_eq!(a.name, DEFAULT_NAME);
        assert_eq!(a.password().len(), net::room::PASSWORD_SYMBOLS);

        let other = temp_dir("new2");
        let b = load_or_create(&other).expect("create");
        assert_ne!(
            a.password(),
            b.password(),
            "two installs must not share a password"
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn the_room_survives_a_restart() {
        let dir = temp_dir("reload");
        let first = load_or_create(&dir).expect("create");
        let second = load_or_create(&dir).expect("reload");
        assert_eq!(first.name, second.name);
        assert_eq!(first.password(), second.password());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_password_is_not_readable_in_the_file_on_disk() {
        let dir = temp_dir("sealed");
        let room = load_or_create(&dir).expect("create");
        let raw = std::fs::read(path(&dir)).expect("read");
        let needle = room.password().as_bytes();
        assert!(
            !raw.windows(needle.len()).any(|w| w == needle),
            "the password must not be stored in the clear"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn renaming_keeps_the_password_and_persists() {
        let dir = temp_dir("rename");
        let room = load_or_create(&dir).expect("create");
        let renamed = rename(&dir, &room, "  Lab 2  ").expect("rename");
        assert_eq!(renamed.name, "Lab 2", "the name is trimmed");
        assert_eq!(renamed.password(), room.password());
        assert_eq!(load_or_create(&dir).expect("reload").name, "Lab 2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_blank_or_over_long_name_is_refused() {
        let dir = temp_dir("badname");
        let room = load_or_create(&dir).expect("create");
        assert!(rename(&dir, &room, "   ").is_err());
        assert!(rename(&dir, &room, &"x".repeat(proto::MAX_ROOM_NAME + 1)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn regenerating_changes_the_password_but_not_the_name() {
        let dir = temp_dir("regen");
        let room = rename(&dir, &load_or_create(&dir).expect("create"), "Lab 7").expect("rename");
        let fresh = regenerate_password(&dir, &room).expect("regen");
        assert_eq!(fresh.name, "Lab 7");
        assert_ne!(fresh.password(), room.password());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_welcome_carries_a_hash_and_never_the_password() {
        let dir = temp_dir("welcome");
        let room = load_or_create(&dir).expect("create");
        let welcome = room.welcome().expect("welcome");
        assert!(welcome.is_well_formed());
        assert!(welcome.room_secret.starts_with("$argon2id$"));
        assert!(!welcome.room_secret.contains(room.password()));
        // and the hash it carries really does accept the room password
        assert!(
            RoomSecret::from_stored(&welcome.room_secret)
                .verify(room.password())
                .is_ok()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
