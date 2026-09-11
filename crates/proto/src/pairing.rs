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

/// A message on the pairing stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairMessage {
    /// Agent → Console: "the operator gave me this code." The code is six digits, sent as a plain
    /// integer (`0..1_000_000`); the channel encrypts it.
    Request {
        /// The six-digit code as an integer.
        code: u32,
    },
    /// Console → Agent: pairing succeeded. Both sides now pin the other's transport public key.
    Accepted,
    /// Console → Agent: pairing refused, with the reason.
    Rejected(PairRejection),
}
