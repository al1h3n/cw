//! Wire protocol types shared by the Console, the Agent and the Hub.
//!
//! This crate performs no I/O. Everything in it must be safe to decode from untrusted input.

/// Working product name. The real name is not chosen yet: change it here and nowhere else.
pub const PRODUCT_NAME: &str = "Co-watcher";

/// Wire protocol version, exchanged in the handshake.
///
/// Bump it on every breaking change to message layout or meaning. Peers with different
/// versions must refuse the session with a clear error instead of guessing.
/// Version 5 added screen recording (`StartRecording`, `RecordingState`, …); version 4 added the
/// app launcher (`ListApps`, `LaunchApp`, `ListRunning`, `CloseApp`); version 3
/// added remote mouse/keyboard input; version 2 added remote actions. Each shifted the later
/// `Control` discriminants, so a peer on an older version would decode them as the wrong message and
/// the handshake refuses it outright.
pub const PROTOCOL_VERSION: u32 = 5;

mod device_id;
mod pairing;
mod record_id;
mod wire;

pub use device_id::{DeviceId, DeviceIdParseError};
pub use pairing::{MAX_ROOM_NAME, MAX_ROOM_SECRET, PairMessage, PairRejection, Welcome};
pub use record_id::{RecordId, RecordIdParseError};
pub use wire::{
    Action, ActionFailure, ActionOutcome, AppEntry, AudioFormat, Capabilities, Control,
    DecodeError, Hello, InputEvent, MAX_BLOCKLIST, MAX_INPUT_BATCH, Monitor, PointerButton,
    ProtocolError, RecordingInfo, Role, RunningApp, StoredRecording, decode, encode,
    version_compatible,
};
