//! Networking for the Console, the Agent and the Hub: device identity, the iroh endpoint, pairing
//! and stream transport.
//!
//! So far: device identity (Phase 1.1), the pairing protocol logic (Phase 1.3), and the iroh endpoint
//! that carries pairing over the wire with mDNS LAN discovery (Phase 1.3b).

pub mod control;
pub mod endpoint;
pub mod identity;
pub mod pairing;
pub mod room;

pub use control::{AgentDevice, CaptureError, ControlSession, LocalHello, PeerInfo};
pub use endpoint::{
    EndpointError, PairedPeer, agent_request_pairing, bind, console_accept_pairing,
};
pub use identity::{Identity, IdentityError};
pub use pairing::{
    PairingCode, PairingCodeParseError, PairingSession, TrustStore, TrustStoreError,
};
pub use room::{RoomError, RoomPassword, RoomSecret};
