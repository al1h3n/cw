//! Wire protocol types shared by the Console, the Agent and the Hub.
//!
//! This crate performs no I/O. Everything in it must be safe to decode from untrusted input.

/// Working product name. The real name is not chosen yet: change it here and nowhere else.
pub const PRODUCT_NAME: &str = "Co-watcher";

/// Wire protocol version, exchanged in the handshake.
///
/// Bump it on every breaking change to message layout or meaning. Peers with different
/// versions must refuse the session with a clear error instead of guessing.
/// Version 21 added workspace file transfer (`ListFiles`/`Files`, `FetchFile`/`FileTransfer`,
/// `SendFile`/`FileSendReady`/`FileSent`) — browse, download and upload files inside a student PC's
/// shared workspace folder;
/// version 20 added a `duration_seconds` to `SetExam` (the Agent auto-releases a timed lock, so an
/// AI/MCP "lock for 10 s" no longer locks forever) and a `ResetWallpaper` action (put the wallpaper
/// back to the student's own, else the Windows default);
/// version 19 added a `fit` to `SetWallpaper` and `SetScreenLock`/`ScreenLockState` (freeze a
/// student's input without taking control);
/// version 18 added `FetchRunningIcon` (a running process's icon, replied via `AppIcon`);
/// version 17 added a per-request JPEG `quality` to `RequestThumbnail` (compression separate from
/// size), so a teacher can pick preview sharpness;
/// version 16 added setting a chosen desktop wallpaper on students (`SetWallpaper`);
/// version 9 added full-resolution H.264 streaming (`StartStream`, on its own uni-stream);
/// version 8 added the watched/black-wallpaper signal (`SetWatched`);
/// version 7 added Wake-on-LAN (`ListMacs`, `WakeOnLan`); version 6 added broadcasting the
/// teacher's screen (`ShowBroadcast`, `StopBroadcast`);
/// version 5 added screen recording (`StartRecording`, `RecordingState`, …); version 4 added the
/// app launcher (`ListApps`, `LaunchApp`, `ListRunning`, `CloseApp`); version 3
/// added remote mouse/keyboard input; version 2 added remote actions. Each shifted the later
/// `Control` discriminants, so a peer on an older version would decode them as the wrong message and
/// the handshake refuses it outright.
pub const PROTOCOL_VERSION: u32 = 21;

/// Default JPEG quality (`1..=100`) for screen thumbnails when the Console does not specify one, and
/// what non-preview callers (diagnostics, the MCP screenshot tool) pass. 60 measured ~5–7 KB at
/// 320×180 in spike 0.4 — a good size/clarity balance for the grid.
pub const DEFAULT_THUMBNAIL_QUALITY: u8 = 60;

mod device_id;
mod pairing;
mod record_id;
mod wire;

pub use device_id::{DeviceId, DeviceIdParseError};
pub use pairing::{MAX_ROOM_NAME, MAX_ROOM_SECRET, PairMessage, PairRejection, Welcome};
pub use record_id::{RecordId, RecordIdParseError};
pub use wire::{
    Action, ActionFailure, ActionOutcome, AppEntry, AudioFormat, Capabilities, Codec, Control,
    DecodeError, FileEntry, Hello, InputEvent, MAX_BLOCKLIST, MAX_FILE_LIST, MAX_FILE_PATH,
    MAX_INPUT_BATCH, Monitor, PointerButton, Preset, ProtocolError, RecordOptions, RecordingInfo,
    Role, RunningApp, Scaler, StoredRecording, VideoSettings, WallpaperFit, decode, encode,
    version_compatible,
};
