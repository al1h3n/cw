//! Room membership: the secret that lets a device *leave*.
//!
//! Joining a room is easy on purpose — a teacher clicks Invite, the PC accepts. Leaving is not:
//! a student must not be able to quietly unenrol their PC halfway through a lesson. So an Agent
//! only unpairs when it is given the **room password**, which the Console generated when the room
//! was created and which the teacher keeps.
//!
//! What is stored on the student PC is an **Argon2id hash** of that password, never the password
//! itself (AGENTS.md D10: no hardcoded master password, and secrets are never written in the clear).
//! Someone who steals the file gets a hash that costs real memory and time to attack, and that is
//! useless on any other room because of the per-room salt.
//!
//! ## The honest limit
//!
//! This stops a *student*. It does not stop a local administrator, who can stop any service and
//! delete any file — that is what administrator means, and it is listed in `docs/FEATURES.md` under
//! things that are impossible by design. The answer there is organisational (do not give students
//! admin), not cryptographic.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

/// How many characters a generated room password has.
///
/// Twelve characters from a 32-symbol alphabet is 60 bits of entropy — far past guessing, while
/// still being something a teacher can write on a card and read out. Printed in groups of four.
pub const PASSWORD_SYMBOLS: usize = 12;

/// The alphabet for generated passwords: Crockford base32, same as a Device ID, so the same
/// "no `I`, `L`, `O`, `U`" reading rules apply and nothing is ambiguous over the phone.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Things that can go wrong with a room secret.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RoomError {
    /// The supplied password did not match.
    #[error("wrong room password")]
    WrongPassword,
    /// The stored hash is corrupt or was written by a newer version.
    #[error("the stored room password is unreadable")]
    CorruptHash,
    /// Hashing itself failed (out of memory).
    #[error("could not hash the room password")]
    HashFailed,
}

/// A freshly generated room password, in plain text.
///
/// Deliberately **not** `Serialize`: it exists to be shown to the teacher once and hashed. Only the
/// hash is ever stored or sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomPassword(String);

impl RoomPassword {
    /// Generates a new random room password.
    #[must_use]
    pub fn generate() -> Self {
        let text: String = (0..PASSWORD_SYMBOLS)
            .map(|_| ALPHABET[rand::random_range(0..ALPHABET.len())] as char)
            .collect();
        Self(text)
    }

    /// Wraps a password the teacher typed themselves.
    #[must_use]
    pub fn from_text(text: &str) -> Self {
        Self(text.trim().to_string())
    }

    /// The raw password text, for showing to the teacher or hashing.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Grouped in fours for reading aloud and writing down: `K7M2-Q9XR-4T6B`.
    #[must_use]
    pub fn grouped(&self) -> String {
        self.0
            .as_bytes()
            .chunks(4)
            .map(|c| String::from_utf8_lossy(c).to_string())
            .collect::<Vec<_>>()
            .join("-")
    }

    /// Hashes this password for storage on an Agent.
    ///
    /// # Errors
    /// [`RoomError::HashFailed`] if Argon2 cannot allocate.
    pub fn hash(&self) -> Result<RoomSecret, RoomError> {
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(self.0.as_bytes(), &salt)
            .map_err(|_| RoomError::HashFailed)?;
        Ok(RoomSecret(hash.to_string()))
    }
}

/// The Argon2id hash of a room password, as stored on an Agent and carried in the invite.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RoomSecret(String);

impl RoomSecret {
    /// The PHC-string form, for storing on disk or putting on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Rebuilds a secret from its stored PHC string.
    #[must_use]
    pub fn from_stored(text: &str) -> Self {
        Self(text.to_string())
    }

    /// Checks a password a student (or teacher) typed against this hash.
    ///
    /// Argon2's verifier compares in constant time, so this leaks nothing by timing.
    ///
    /// # Errors
    /// [`RoomError::WrongPassword`] on a mismatch, [`RoomError::CorruptHash`] if the stored hash
    /// cannot be parsed.
    pub fn verify(&self, attempt: &str) -> Result<(), RoomError> {
        let parsed = PasswordHash::new(&self.0).map_err(|_| RoomError::CorruptHash)?;
        Argon2::default()
            .verify_password(attempt.trim().as_bytes(), &parsed)
            .map_err(|_| RoomError::WrongPassword)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_password_has_the_advertised_length_and_alphabet() {
        let password = RoomPassword::generate();
        assert_eq!(password.as_str().len(), PASSWORD_SYMBOLS);
        assert!(
            password.as_str().bytes().all(|b| ALPHABET.contains(&b)),
            "{} used a symbol outside the alphabet",
            password.as_str()
        );
    }

    #[test]
    fn a_generated_password_never_contains_a_look_alike_letter() {
        for _ in 0..200 {
            let text = RoomPassword::generate().0;
            assert!(!text.contains(['I', 'L', 'O', 'U']), "{text}");
        }
    }

    #[test]
    fn two_generated_passwords_differ() {
        assert_ne!(RoomPassword::generate(), RoomPassword::generate());
    }

    #[test]
    fn grouped_form_is_readable_in_fours() {
        let password = RoomPassword::from_text("K7M2Q9XR4T6B");
        assert_eq!(password.grouped(), "K7M2-Q9XR-4T6B");
    }

    #[test]
    fn the_right_password_verifies() {
        let password = RoomPassword::from_text("K7M2Q9XR4T6B");
        let secret = password.hash().expect("hash");
        assert_eq!(secret.verify("K7M2Q9XR4T6B"), Ok(()));
    }

    #[test]
    fn a_wrong_password_is_refused() {
        let secret = RoomPassword::from_text("K7M2Q9XR4T6B")
            .hash()
            .expect("hash");
        assert_eq!(secret.verify("K7M2Q9XR4T6C"), Err(RoomError::WrongPassword));
        assert_eq!(secret.verify(""), Err(RoomError::WrongPassword));
    }

    #[test]
    fn surrounding_whitespace_is_forgiven_when_typing_it_back() {
        let secret = RoomPassword::from_text("K7M2Q9XR4T6B")
            .hash()
            .expect("hash");
        assert_eq!(secret.verify("  K7M2Q9XR4T6B \n"), Ok(()));
    }

    #[test]
    fn the_stored_form_never_contains_the_password() {
        let secret = RoomPassword::from_text("K7M2Q9XR4T6B")
            .hash()
            .expect("hash");
        assert!(
            !secret.as_str().contains("K7M2Q9XR4T6B"),
            "the hash must not embed the password"
        );
        assert!(secret.as_str().starts_with("$argon2id$"));
    }

    #[test]
    fn the_same_password_hashes_differently_each_time() {
        // Per-hash salt: two rooms with the same password share no stored bytes.
        let a = RoomPassword::from_text("SAMEPASSWORD").hash().expect("a");
        let b = RoomPassword::from_text("SAMEPASSWORD").hash().expect("b");
        assert_ne!(a, b);
        assert_eq!(a.verify("SAMEPASSWORD"), Ok(()));
        assert_eq!(b.verify("SAMEPASSWORD"), Ok(()));
    }

    #[test]
    fn a_corrupt_stored_hash_is_reported_as_corrupt_not_as_a_wrong_password() {
        let secret = RoomSecret::from_stored("not-a-phc-string");
        assert_eq!(secret.verify("anything"), Err(RoomError::CorruptHash));
    }

    #[test]
    fn a_stored_secret_survives_a_save_and_load_round_trip() {
        let secret = RoomPassword::from_text("K7M2Q9XR4T6B")
            .hash()
            .expect("hash");
        let reloaded = RoomSecret::from_stored(secret.as_str());
        assert_eq!(reloaded.verify("K7M2Q9XR4T6B"), Ok(()));
    }
}
