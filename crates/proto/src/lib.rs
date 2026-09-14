//! Wire protocol types shared by the Console, the Agent and the Hub.
//!
//! This crate performs no I/O. Everything in it must be safe to decode from untrusted input.

/// Working product name. The real name is not chosen yet: change it here and nowhere else.
pub const PRODUCT_NAME: &str = "Co-watcher";

/// Wire protocol version, exchanged in the handshake.
///
/// Bump it on every breaking change to message layout or meaning. Peers with different
/// versions must refuse the session with a clear error instead of guessing.
/// Version 2 added remote actions (`Perform`/`ActionDone`), which shifted the later `Control`
/// discriminants — a version-1 peer would decode them as the wrong message, so the handshake must
/// refuse it.
pub const PROTOCOL_VERSION: u32 = 2;

mod device_id;
mod pairing;
mod wire;

pub use device_id::{DeviceId, DeviceIdParseError};
pub use pairing::{PairMessage, PairRejection};
pub use wire::{
    Action, ActionFailure, ActionOutcome, AudioFormat, Capabilities, Control, DecodeError, Hello,
    MAX_BLOCKLIST, Monitor, ProtocolError, Role, decode, encode, version_compatible,
};
