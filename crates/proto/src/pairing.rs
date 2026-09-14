//! Wire messages for enrolling a new device (pairing).
//!
//! Pairing establishes mutual trust in each other's public keys for the first time. It runs over the
//! iroh transport, which already encrypts the channel and authenticates both endpoints by their
//! public keys — so a network attacker can neither read the code nor substitute a key. The short
//! code is therefore an **authorization token** (does the operator intend to enrol *this* device?),
//! guarded by expiry, single use and an attempt limit, not a defence against passive interception.
//! That is why a PAKE (SPAKE2) is unnecessary here: the transport already provides the authenticated
//! channel a PAKE would otherwise build. See `net::pairing` for the logic.

use serde::{Deserialize, Serialize};

/// Why a pairing attempt was refused. Sent by the Console to the Agent, and returned by the
/// Console-side verifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum PairRejection {
    /// The code did not match.
    #[error("wrong pairing code")]
    WrongCode,
    /// The code's time window has passed.
    #[error("pairing code has expired")]
    Expired,
    /// The code already completed a successful pairing (replay).
    #[error("pairing code was already used")]
    AlreadyUsed,
    /// Too many wrong attempts; the code is now locked.
    #[error("too many failed pairing attempts")]
    TooManyAttempts,
}

/// The longest room name an Agent will accept, in bytes. A room is "Lab 2" or "Room 314"; this is
/// generous, and it is enforced at the trust boundary so a peer cannot send a megabyte of text.
pub const MAX_ROOM_NAME: usize = 64;

/// The longest room-secret string an Agent will accept. An Argon2id PHC string is about 100 bytes.
pub const MAX_ROOM_SECRET: usize = 256;

/// What a Console tells a device when it joins: which room it is in, and the hash it must match
/// before it is allowed to leave again.
///
/// The secret is an **Argon2id hash**, never the password itself, so the password never travels and
/// a student who reads this off their own disk learns nothing usable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    /// Human-readable room name, for the student PC and the audit log to show.
    pub room: String,
    /// Argon2id PHC string of the room password.
    pub room_secret: String,
}

impl Welcome {
    /// Whether this welcome is within the size limits an Agent will store.
    ///
    /// Checked on the Agent before anything is written to disk: a Console is trusted to be the
    /// teacher's, but "trusted" never means "unvalidated" (AGENTS.md §5).
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        !self.room.is_empty()
            && self.room.len() <= MAX_ROOM_NAME
            && !self.room_secret.is_empty()
            && self.room_secret.len() <= MAX_ROOM_SECRET
            && !self.room.chars().any(char::is_control)
    }
}

/// A message on the pairing stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairMessage {
    /// Agent → Console: "the operator gave me this code." The code is six digits, sent as a plain
    /// integer (`0..1_000_000`); the channel encrypts it.
    Request {
        /// The six-digit code as an integer.
        code: u32,
    },
    /// Console → Agent: pairing succeeded, and here is the room you have joined. Both sides now pin
    /// the other's transport public key.
    Accepted(Welcome),
    /// Console → Agent: pairing refused, with the reason.
    Rejected(PairRejection),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn welcome(room: &str, secret: &str) -> Welcome {
        Welcome {
            room: room.to_string(),
            room_secret: secret.to_string(),
        }
    }

    #[test]
    fn a_normal_welcome_is_well_formed() {
        assert!(welcome("Lab 2", "$argon2id$v=19$m=19456,t=2,p=1$abc$def").is_well_formed());
    }

    #[test]
    fn an_empty_room_or_secret_is_refused() {
        assert!(!welcome("", "$argon2id$x").is_well_formed());
        assert!(!welcome("Lab 2", "").is_well_formed());
    }

    #[test]
    fn an_over_long_room_name_is_refused() {
        assert!(!welcome(&"x".repeat(MAX_ROOM_NAME + 1), "$argon2id$x").is_well_formed());
        assert!(welcome(&"x".repeat(MAX_ROOM_NAME), "$argon2id$x").is_well_formed());
    }

    #[test]
    fn an_over_long_secret_is_refused() {
        assert!(!welcome("Lab 2", &"x".repeat(MAX_ROOM_SECRET + 1)).is_well_formed());
    }

    #[test]
    fn a_room_name_cannot_smuggle_new_lines_into_a_log_line() {
        let newline = char::from(10u8);
        let smuggled = format!("Lab 2{newline}FAKE AUDIT ROW");
        assert!(!welcome(&smuggled, "$argon2id$x").is_well_formed());
        let tab = char::from(9u8);
        assert!(!welcome(&format!("Lab{tab}2"), "$argon2id$x").is_well_formed());
    }
}
