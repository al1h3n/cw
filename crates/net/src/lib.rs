//! Networking for the Console, the Agent and the Hub: device identity, the iroh endpoint, pairing
//! and stream transport.
//!
//! So far: device identity (Phase 1.1) and the pairing protocol logic (Phase 1.3). The iroh endpoint
//! that carries these over the wire, and LAN discovery, land next.

pub mod identity;
pub mod pairing;

pub use identity::{Identity, IdentityError};
pub use pairing::{PairingCode, PairingCodeParseError, PairingSession, TrustStore};
