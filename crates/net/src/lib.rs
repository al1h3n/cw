//! Networking for the Console, the Agent and the Hub: device identity, the iroh endpoint, pairing
//! and stream transport.
//!
//! Only the identity layer exists so far (Phase 1.1). The endpoint, pairing and stream multiplexing
//! land in later steps.

pub mod identity;

pub use identity::{Identity, IdentityError};
