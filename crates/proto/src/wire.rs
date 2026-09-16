//! Version 1 of the control-channel wire format.
//!
//! Messages are encoded with [postcard](https://docs.rs/postcard) (compact, `no_std`-friendly,
//! serde-based). This module is a **trust boundary**: [`decode`] must be safe on arbitrary bytes
//! from a peer, so every type here avoids unbounded collections — the only variable-size field is
//! bounded by validation before it is trusted. New fields or variants require bumping
//! [`crate::PROTOCOL_VERSION`].

use serde::{Deserialize, Serialize};

use crate::{DeviceId, PROTOCOL_VERSION};

/// The most blocklist rules an Agent keeps. A classroom "no games" list is a few dozen names; a
/// bound this size keeps the watch loop cheap and stops a malformed message asking for millions.
pub const MAX_BLOCKLIST: usize = 256;

/// What role a peer plays. Sent in [`Control::Hello`] so each side knows who it is talking to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// The teacher/admin application.
    Console,
    /// A managed student device.
    Agent,
    /// The always-on directory / mailbox service.
    Hub,
}

/// The set of features a peer supports, as a forward-compatible bit set.
///
/// Unknown bits set by a newer peer are preserved and ignored, so adding a capability does not by
/// itself break the wire (it still needs a `PROTOCOL_VERSION` bump if behaviour depends on it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Capabilities(u64);

impl Capabilities {
    /// No capabilities.
    pub const EMPTY: Self = Self(0);
    /// The device can capture and stream its screen.
    pub const SCREEN_CAPTURE: Self = Self(1 << 0);
    /// The device can capture and stream audio.
    pub const AUDIO: Self = Self(1 << 1);
    /// The device accepts remote mouse and keyboard input.
    pub const REMOTE_INPUT: Self = Self(1 << 2);
    /// The device can lock its screen on command.
    pub const LOCK: Self = Self(1 << 3);
    /// The device can shut down, reboot or log off on command.
    pub const POWER: Self = Self(1 << 4);
    /// The device can block programs from running.
    pub const BLOCK: Self = Self(1 << 5);

    /// Combines two capability sets.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// True if every capability in `other` is present in `self`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The raw bits (for storage or logging).
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }
}

/// One monitor attached to a student PC.
///
/// Virtual desktops are deliberately absent: Windows does not render an inactive virtual desktop, so
/// nothing can capture one. Capture always shows the desktop the student is currently on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Monitor {
    /// Index to use in [`Control::RequestThumbnail`].
    pub index: u8,
    /// Native width in pixels.
    pub width: u32,
    /// Native height in pixels.
    pub height: u32,
    /// Whether this is the PC's primary monitor.
    pub primary: bool,
}

/// The shape of the audio an Agent is sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    /// Samples per second.
    pub sample_rate: u32,
    /// Channel count; currently always 1, because the stream is downmixed to mono before sending.
    pub channels: u8,
}

/// The most input events carried in one [`Control::Input`] batch.
///
/// Mouse movement produces events far faster than a network round trip, so the Console coalesces
/// them into batches. The cap keeps one message small and bounds the work an Agent does per message.
pub const MAX_INPUT_BATCH: usize = 64;

/// A mouse button, as named on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PointerButton {
    /// Primary button.
    Left,
    /// Context-menu button.
    Right,
    /// Wheel click.
    Middle,
}

/// One remote input event.
///
/// Positions are **fractions of the screen** expressed as `0..=65535`, not pixels: the teacher's
/// screen is rarely the same size or scale as the student's, and a fraction survives a resolution
/// change, a scaled display and a different monitor. It is also exactly the range Windows'
/// absolute-positioning API uses, so nothing is lost in translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputEvent {
    /// Move the pointer to this fraction of the screen.
    MoveTo {
        /// Horizontal position, `0..=65535` across the screen.
        x: u16,
        /// Vertical position, `0..=65535` down the screen.
        y: u16,
    },
    /// Press or release a mouse button where the pointer is.
    Button {
        /// Which button.
        button: PointerButton,
        /// True to press, false to release.
        down: bool,
    },
    /// Turn the wheel. Positive is away from the user.
    Scroll {
        /// Notches to scroll.
        delta: i16,
    },
    /// Press or release a key, by Windows virtual-key code.
    ///
    /// A key code rather than a letter, so the **student's** keyboard layout decides what character
    /// appears — the same choice RDP and VNC make.
    Key {
        /// Windows virtual-key code.
        virtual_key: u16,
        /// True to press, false to release.
        down: bool,
    },
    /// Type one character directly, for symbols the student's layout cannot otherwise produce.
    Text(char),
    /// Release every modifier. Sent when control ends, so no key is left stuck down.
    ReleaseAll,
}

/// The shape of a live video stream.
///
/// The video codec a recording is encoded with (when ffmpeg is available on the student PC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Codec {
    /// H.264 — the compatible default, plays everywhere.
    #[default]
    H264,
    /// H.265/HEVC — smaller files, needs a newer player.
    H265,
    /// AV1 — smallest, slowest to encode.
    Av1,
}

/// The encoder speed/efficiency trade-off (x264/x265 names; mapped for AV1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Preset {
    /// Fastest, largest files.
    Ultrafast,
    /// Very fast.
    Superfast,
    /// Faster than realtime on most PCs.
    Veryfast,
    /// A little faster than default.
    Faster,
    /// Fast.
    Fast,
    /// A balanced default.
    #[default]
    Medium,
    /// Slower, smaller.
    Slow,
    /// Slower still.
    Slower,
    /// Slowest, smallest files.
    Veryslow,
}

/// How frames are scaled to the recording size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Scaler {
    /// Fast, softer.
    Bilinear,
    /// Sharper than bilinear.
    Bicubic,
    /// Sharpest downscale for text — the default.
    #[default]
    Lanczos,
    /// Nearest-neighbour, blocky.
    Neighbor,
}

/// Everything the teacher chose about how to record: size, rate and encoding.
///
/// Applied by ffmpeg on the student PC when it is present; if it is not, the Agent falls back to its
/// built-in MJPEG recorder and only `max_width`/`max_height`/`fps` apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordOptions {
    /// Widest the saved video may be; the real size keeps the screen's aspect ratio.
    pub max_width: u32,
    /// Tallest the saved video may be.
    pub max_height: u32,
    /// Frames per second to capture.
    pub fps: u32,
    /// Which codec.
    pub codec: Codec,
    /// Encoder preset.
    pub preset: Preset,
    /// Constant-quality value (CRF): lower is better quality and a bigger file. 0..=51.
    pub quality: u8,
    /// Maximum consecutive B-frames (compression; 0 disables). Defaults high ("each").
    pub bframes: u8,
    /// Scaling filter.
    pub scaler: Scaler,
    /// Two-pass encode for the best size/quality (a post-record re-encode; ignored by MJPEG).
    pub two_pass: bool,
}

impl Default for RecordOptions {
    fn default() -> Self {
        Self {
            max_width: 1920,
            max_height: 1080,
            fps: 15,
            codec: Codec::H264,
            preset: Preset::Medium,
            quality: 23,
            bframes: 8,
            scaler: Scaler::Lanczos,
            two_pass: false,
        }
    }
}

/// Separate from [`RecordingInfo`] because they answer different questions: a recording is written
/// to the student's disk, a stream is watched now. Both are clamped by the Agent, never trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoSettings {
    /// Frame width in pixels (forced even — H.264 chroma is subsampled).
    pub width: u32,
    /// Frame height in pixels (forced even).
    pub height: u32,
    /// Frames per second.
    pub fps: u32,
    /// Target bitrate in kbit/s. This is the knob that decides whether a room of streams fits.
    pub kbps: u32,
}

/// How a recording is going, as the Agent reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingInfo {
    /// Whether a recording is running right now.
    pub active: bool,
    /// The file being written, by name only — a Console has no business knowing a student's disk
    /// layout, and a name is all it needs to ask for the file later.
    pub file: String,
    /// Frames written so far.
    pub frames: u32,
    /// The size actually being recorded, after clamping and keeping the aspect ratio.
    pub width: u32,
    /// The height actually being recorded.
    pub height: u32,
    /// The frame rate actually in use, after clamping.
    pub fps: u32,
    /// Empty unless something went wrong, so a teacher learns why a recording stopped.
    pub problem: String,
}

/// One recording stored on a student PC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredRecording {
    /// File name, which begins with a time-ordered [`crate::RecordId`] so the list sorts by age.
    pub file: String,
    /// Size on disk, so a teacher can see what collecting it would cost.
    pub bytes: u64,
}

/// One program a PC offers to start, as published by the Agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppEntry {
    /// The id a Console uses to ask for it. Meaningful only on the PC that published it.
    pub id: u32,
    /// What to show a teacher.
    pub name: String,
}

/// One running program a teacher may close.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningApp {
    /// Process id on that PC.
    pub pid: u32,
    /// Executable name, lower-cased.
    pub name: String,
}

/// Something a Console can make a student PC do.
///
/// This is a **closed list on purpose**: there is no "run this command" variant in any tier, so a
/// stolen Console key can only do these named things, never execute arbitrary code (AGENTS.md §5).
/// Adding a member is a deliberate, reviewable protocol change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    /// Power the PC off after `delay_seconds`, showing the student a countdown.
    Shutdown {
        /// Grace period before it happens, so a student can save work. Clamped by the Agent.
        delay_seconds: u16,
    },
    /// Restart the PC after `delay_seconds`.
    Reboot {
        /// Grace period before it happens. Clamped by the Agent.
        delay_seconds: u16,
    },
    /// Sign the student out, closing their programs.
    LogOff,
    /// Lock the session, exactly as Win+L does. The student's programs keep running.
    LockScreen,
    /// Call off a shutdown or reboot that is still counting down.
    CancelShutdown,
    /// Stop the student changing their desktop wallpaper. Persists across reboots.
    LockWallpaper,
    /// Let the student change their wallpaper again.
    UnlockWallpaper,
}

impl Action {
    /// A short, stable identifier for logs and the audit trail. Never translated.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Shutdown { .. } => "shutdown",
            Self::Reboot { .. } => "reboot",
            Self::LogOff => "log-off",
            Self::LockScreen => "lock-screen",
            Self::CancelShutdown => "cancel-shutdown",
            Self::LockWallpaper => "lock-wallpaper",
            Self::UnlockWallpaper => "unlock-wallpaper",
        }
    }

    /// Whether this action needs the "power" capability (the rest need "lock").
    #[must_use]
    pub const fn needs_power(self) -> bool {
        matches!(
            self,
            Self::Shutdown { .. } | Self::Reboot { .. } | Self::LogOff | Self::CancelShutdown
        )
    }
}

/// Why a device could not carry out an [`Action`].
///
/// A fixed set rather than a message string: the reason is shown to a teacher in their own language,
/// and a remote peer must never be able to put arbitrary text on someone's screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum ActionFailure {
    /// The device does not support this action (wrong OS, no such feature).
    #[error("this device cannot do that")]
    NotSupported,
    /// The Agent lacks the rights — typically the shutdown privilege was denied.
    #[error("the device refused: not permitted")]
    NotPermitted,
    /// `CancelShutdown` arrived when nothing was counting down.
    #[error("nothing was scheduled to cancel")]
    NothingScheduled,
    /// The OS rejected the request for another reason.
    #[error("the device could not do it")]
    Failed,
}

/// What happened to a requested [`Action`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionOutcome {
    /// Accepted and under way. For a delayed shutdown this means the countdown started.
    Started {
        /// Seconds until it actually happens; 0 for immediate actions.
        delay_seconds: u16,
    },
    /// Not done, with the reason.
    Failed(ActionFailure),
}

/// The handshake a peer sends first, before any other message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    /// The sender's [`crate::PROTOCOL_VERSION`].
    pub protocol_version: u32,
    /// What the sender is.
    pub role: Role,
    /// The sender's short handle (its public key, the real identity, is verified by the transport).
    pub device_id: DeviceId,
    /// What the sender can do.
    pub capabilities: Capabilities,
}

/// A typed error one peer can report to the other on the control channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
pub enum ProtocolError {
    /// The peers speak incompatible protocol versions.
    #[error("incompatible protocol version: ours {ours}, theirs {theirs}")]
    UnsupportedVersion {
        /// The reporting side's version.
        ours: u32,
        /// The version the reporting side received.
        theirs: u32,
    },
    /// The sender is not allowed to do what it asked (bad pairing, revoked key).
    #[error("not authorized")]
    Unauthorized,
    /// A message arrived out of the expected order (e.g. before the handshake).
    #[error("unexpected message for the current state")]
    UnexpectedMessage,
    /// The sender is shutting the connection down cleanly.
    #[error("peer is going away")]
    GoingAway,
    /// The screen cannot be captured *right now* — a lock screen, a UAC secure desktop, or a
    /// display mode change. Unlike the others this is **transient**: the receiver should keep the
    /// session open and retry, not disconnect. Kills the reconnect storm a hard error used to cause.
    #[error("screen temporarily unavailable")]
    ScreenUnavailable,
}

/// A control-channel message. This is the top-level type carried over the control stream.
///
/// Not `Copy`: [`Control::Thumbnail`] carries an owned JPEG. That payload is variable-length but
/// bounded by the transport's per-message frame cap, so decoding stays safe on hostile input.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Control {
    /// The opening handshake.
    Hello(Hello),
    /// Liveness probe carrying a nonce the peer echoes in [`Control::Pong`].
    Ping(u64),
    /// Reply to [`Control::Ping`] with the same nonce.
    Pong(u64),
    /// Console → Agent: capture one image of `monitor`, no wider than `max_width` pixels.
    /// The Agent captures only in response to this, so an idle Console means zero capture (D11).
    /// `max_width` is how the Console picks preview quality: small for the grid, large when a
    /// teacher opens one screen.
    RequestThumbnail {
        /// Which monitor (0-based).
        monitor: u8,
        /// Maximum width in pixels; the Agent scales down to fit and keeps the aspect ratio.
        max_width: u16,
    },
    /// Console → Agent: what monitors does this PC have?
    ListMonitors,
    /// Agent → Console: the attached monitors.
    Monitors(Vec<Monitor>),
    /// Console → Agent: start or stop recording what this PC is playing.
    ///
    /// Audio is off until asked for, and only one PC is listened to at a time, because raw PCM is
    /// far heavier than a thumbnail. Turning it off stops the recording entirely.
    SetAudio {
        /// Whether the Agent should be capturing sound.
        enabled: bool,
    },
    /// Agent → Console: whether audio is running, and in what shape.
    AudioState(Option<AudioFormat>),
    /// Console → Agent: give me the sound recorded since the last request.
    RequestAudio {
        /// Never return more than this many samples, so one reply stays bounded.
        max_samples: u32,
    },
    /// Agent → Console: mono PCM samples recorded since the previous request.
    Audio {
        /// Increases by one per reply, so a gap is visible.
        seq: u64,
        /// Signed 16-bit mono samples at the rate given in [`Control::AudioState`].
        samples: Vec<i16>,
    },
    /// Console → Agent: block these programs from running. An empty list clears blocking.
    ///
    /// Each entry is an executable name such as `steam.exe` (the `.exe` is optional). The Agent caps
    /// the list to [`MAX_BLOCKLIST`] entries and ignores blanks; matching is by exact file name, so
    /// a rule never kills an unrelated program that merely contains the word.
    SetBlocklist {
        /// Programs to end on sight. Bounded by the Agent, not trusted as sent.
        programs: Vec<String>,
    },
    /// Agent → Console: blocking is now in force with this many rules, and this is what it has
    /// closed most recently (newest last), so a teacher sees blocking actually working.
    BlocklistState {
        /// How many rules the Agent accepted after capping and dropping blanks.
        rules: u16,
        /// Names closed since the previous state message, capped so the reply stays small.
        closed: Vec<String>,
    },
    /// Console → Agent: start sending full-resolution H.264 video of this screen.
    ///
    /// The frames do **not** come back on this stream. The Agent opens a separate QUIC uni-stream
    /// and pushes encoded packets down it, so a slow decoder or a dropped frame never blocks the
    /// control channel — the design Phase-0 spike 0.6 validated.
    StartStream {
        /// Which monitor to stream.
        monitor: u8,
        /// The picture size and rate asked for; the Agent clamps and reports what it will do.
        settings: VideoSettings,
    },
    /// Agent → Console: the stream is starting with these (clamped) settings, or could not start.
    StreamStarted {
        /// What the Agent will actually send.
        settings: VideoSettings,
        /// Empty on success; otherwise why no stream is coming.
        problem: String,
    },
    /// Console → Agent: stop the video stream.
    StopStream,
    /// Agent → Console: the stream is stopped.
    StreamStopped,
    /// Console → Agent: a teacher is now watching (or stopped watching) this PC's screen.
    ///
    /// While watched, the Agent replaces the desktop wallpaper with black (D11) — both a privacy
    /// signal and a way to keep an inappropriate wallpaper out of the teacher's view. The student's
    /// own wallpaper returns when watching stops or the session drops.
    SetWatched {
        /// True while a teacher is watching this screen.
        watched: bool,
    },
    /// Agent → Console: whether the wallpaper is currently blacked out.
    WatchedState {
        /// True if the black wallpaper is in place.
        black: bool,
    },
    /// Console → Agent: what MAC addresses do you have? Stored so the PC can be woken later.
    ListMacs,
    /// Agent → Console: this PC's wakeable MAC addresses (`AA:BB:CC:DD:EE:FF` form).
    Macs(Vec<String>),
    /// Console → Agent (to an *awake* PC): broadcast a Wake-on-LAN packet for `mac` on your LAN.
    ///
    /// This is how a teacher wakes a switched-off PC: an Agent still awake in the same room puts the
    /// magic packet on the wire, because the target has no IP to reach directly.
    WakeOnLan {
        /// The sleeping PC's MAC, `AA:BB:CC:DD:EE:FF`.
        mac: String,
    },
    /// Agent → Console: whether the wake packet went out.
    WakeSent {
        /// True if the packet was broadcast.
        sent: bool,
    },
    /// Console → Agent: show this frame of the teacher's screen, full-screen, on the student PC.
    ///
    /// Each message carries one complete JPEG, so a student who joins late or misses a frame sees
    /// the right picture on the very next one — there is no stream to resynchronise with.
    ShowBroadcast {
        /// One frame of the teacher's screen, JPEG encoded.
        jpeg: Vec<u8>,
    },
    /// Console → Agent: take the broadcast off the screen and give the student their desktop back.
    StopBroadcast,
    /// Agent → Console: whether the broadcast window is on screen, and why not if it is not.
    BroadcastState {
        /// True while the student is seeing the teacher's screen.
        showing: bool,
        /// Empty unless something went wrong.
        problem: String,
    },
    /// Console → Agent: start recording this PC's screen to a file on that PC.
    ///
    /// The Agent clamps every value and reports back what it is *actually* recording, so a teacher
    /// who asks for 144 fps sees that they are getting 30 rather than being silently ignored.
    StartRecording {
        /// Which monitor to record.
        monitor: u8,
        /// How to encode the recording (codec, preset, quality, size, rate).
        options: RecordOptions,
    },
    /// Console → Agent: stop recording and close the file.
    StopRecording,
    /// Console → Agent: how is the recording going?
    RecordingStatus,
    /// Agent → Console: the state of the recording on that PC.
    RecordingState(RecordingInfo),
    /// Console → Agent: what recordings are stored on this PC?
    ListRecordings,
    /// Agent → Console: the recordings it has kept.
    Recordings(Vec<StoredRecording>),
    /// Console → Agent: what programs can this PC start?
    ListApps,
    /// Agent → Console: the programs it offers, as `(id, name)` pairs.
    ///
    /// The **path is deliberately absent**: a Console names an app by id and never by location, so
    /// there is no way to ask a PC to run something it did not itself publish (AGENTS.md §5).
    Apps(Vec<AppEntry>),
    /// Console → Agent: start the catalogue entry with this id.
    LaunchApp {
        /// An id from a previous [`Control::Apps`] reply.
        id: u32,
    },
    /// Agent → Console: whether the program started, and what it was called.
    AppLaunched {
        /// Empty when the id was not in this PC's catalogue.
        name: String,
        /// True if it started.
        started: bool,
    },
    /// Console → Agent: what is running right now?
    ListRunning,
    /// Agent → Console: the running programs a teacher may close.
    ///
    /// System-critical processes are filtered out here, so they cannot even be offered.
    Running(Vec<RunningApp>),
    /// Console → Agent: close this running program.
    CloseApp {
        /// A process id from a previous [`Control::Running`] reply.
        pid: u32,
    },
    /// Agent → Console: whether it closed.
    AppClosed {
        /// True if the process is gone.
        closed: bool,
    },
    /// Console → Agent: take (or give up) control of this PC's mouse and keyboard.
    ///
    /// Taking control is explicit and shows on the student's screen, so nobody is driven silently
    /// (D3). Giving it up releases every held modifier.
    SetControl {
        /// True to take control, false to release it.
        enabled: bool,
    },
    /// Agent → Console: whether this PC is currently accepting remote input.
    ControlState {
        /// True if remote input is being applied.
        enabled: bool,
    },
    /// Console → Agent: apply these input events in order.
    ///
    /// Batched because pointer movement outruns a network round trip. Longer than
    /// [`MAX_INPUT_BATCH`] is refused rather than truncated.
    Input(Vec<InputEvent>),
    /// Agent → Console: how many events were applied, so the Console can tell "refused" from "lost".
    InputDone {
        /// Events actually delivered to the OS.
        applied: u16,
        /// True if the PC is not currently granting control.
        refused: bool,
    },
    /// Console → Agent: do this one named thing (power, lock). See [`Action`].
    Perform(Action),
    /// Agent → Console: what happened to the [`Control::Perform`] request.
    ActionDone {
        /// Echoed back, so a reply is never mistaken for another action's.
        action: Action,
        /// Started, or refused with a reason.
        outcome: ActionOutcome,
    },
    /// Agent → Console: the requested thumbnail as JPEG bytes.
    Thumbnail {
        /// Which monitor this is for.
        monitor: u8,
        /// A monotonically increasing sequence number, for ordering/staleness.
        seq: u64,
        /// JPEG-encoded image.
        jpeg: Vec<u8>,
    },
    /// A typed error from the peer.
    Error(ProtocolError),
}

/// Error decoding bytes from a peer. Carries no attacker-controlled data.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("malformed wire message")]
pub struct DecodeError;

/// Encodes a message to bytes for sending.
///
/// # Panics
/// Never in practice: the message types here have no serializer that can fail. The `expect` guards
/// against a future field whose `Serialize` is fallible, which would be a bug to catch in tests.
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "serialization of these fixed types is infallible; a failure is a bug"
)]
pub fn encode<T: Serialize>(message: &T) -> Vec<u8> {
    postcard::to_allocvec(message).expect("wire message serialization must not fail")
}

/// Decodes exactly one message from a peer. Safe to call on arbitrary, hostile input.
///
/// Rejects trailing bytes: the input must be exactly one encoded message and nothing more, so
/// corruption or a framing bug is caught rather than silently ignored.
///
/// # Errors
/// Returns [`DecodeError`] if the bytes are not a valid encoding of `T`, or if bytes remain after it.
pub fn decode<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, DecodeError> {
    let (value, rest) = postcard::take_from_bytes(bytes).map_err(|_| DecodeError)?;
    if rest.is_empty() {
        Ok(value)
    } else {
        Err(DecodeError)
    }
}

/// Whether a peer's protocol version is compatible with ours.
///
/// For now compatibility means an exact match; when we start supporting a range this is the one
/// place to widen. Returns [`ProtocolError::UnsupportedVersion`] describing the mismatch.
///
/// # Errors
/// Returns [`ProtocolError::UnsupportedVersion`] if `peer_version` differs from ours.
pub fn version_compatible(peer_version: u32) -> Result<(), ProtocolError> {
    if peer_version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ProtocolError::UnsupportedVersion {
            ours: PROTOCOL_VERSION,
            theirs: peer_version,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hello() -> Control {
        Control::Hello(Hello {
            protocol_version: PROTOCOL_VERSION,
            role: Role::Agent,
            device_id: DeviceId::from_public_key(&[5u8; 32]),
            capabilities: Capabilities::SCREEN_CAPTURE
                .union(Capabilities::LOCK)
                .union(Capabilities::POWER),
        })
    }

    #[test]
    fn round_trip_every_variant() {
        for message in [
            sample_hello(),
            Control::Ping(7),
            Control::Pong(7),
            Control::SetBlocklist {
                programs: vec!["steam.exe".into(), "roblox".into()],
            },
            Control::BlocklistState {
                rules: 2,
                closed: vec!["steam.exe".into()],
            },
            Control::SetControl { enabled: true },
            Control::ControlState { enabled: false },
            Control::Input(vec![
                InputEvent::MoveTo { x: 0, y: 65_535 },
                InputEvent::Button {
                    button: PointerButton::Left,
                    down: true,
                },
                InputEvent::Scroll { delta: -3 },
                InputEvent::Key {
                    virtual_key: 0x5B,
                    down: true,
                },
                InputEvent::Text('D'),
                InputEvent::ReleaseAll,
            ]),
            Control::InputDone {
                applied: 6,
                refused: false,
            },
            Control::Perform(Action::Shutdown { delay_seconds: 60 }),
            Control::ActionDone {
                action: Action::LockScreen,
                outcome: ActionOutcome::Started { delay_seconds: 0 },
            },
            Control::ActionDone {
                action: Action::CancelShutdown,
                outcome: ActionOutcome::Failed(ActionFailure::NothingScheduled),
            },
            Control::Error(ProtocolError::Unauthorized),
        ] {
            let bytes = encode(&message);
            assert_eq!(decode::<Control>(&bytes).unwrap(), message);
        }
    }

    #[test]
    fn every_action_has_a_distinct_log_name() {
        let actions = [
            Action::Shutdown { delay_seconds: 0 },
            Action::Reboot { delay_seconds: 0 },
            Action::LogOff,
            Action::LockScreen,
            Action::CancelShutdown,
            Action::LockWallpaper,
            Action::UnlockWallpaper,
        ];
        let mut names: Vec<&str> = actions.iter().map(|a| a.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), actions.len(), "audit names must be unique");
    }

    #[test]
    fn only_locking_needs_no_power_right() {
        assert!(!Action::LockScreen.needs_power());
        assert!(Action::Shutdown { delay_seconds: 0 }.needs_power());
        assert!(Action::Reboot { delay_seconds: 0 }.needs_power());
        assert!(Action::LogOff.needs_power());
        assert!(Action::CancelShutdown.needs_power());
    }

    #[test]
    fn capabilities_contains_and_union() {
        let caps = Capabilities::SCREEN_CAPTURE.union(Capabilities::AUDIO);
        assert!(caps.contains(Capabilities::SCREEN_CAPTURE));
        assert!(caps.contains(Capabilities::AUDIO));
        assert!(!caps.contains(Capabilities::REMOTE_INPUT));
        assert!(caps.contains(Capabilities::EMPTY));
    }

    #[test]
    fn matching_version_is_compatible() {
        assert!(version_compatible(PROTOCOL_VERSION).is_ok());
    }

    #[test]
    fn mismatched_version_reports_both_sides() {
        let wrong = PROTOCOL_VERSION.wrapping_add(1);
        assert_eq!(
            version_compatible(wrong),
            Err(ProtocolError::UnsupportedVersion {
                ours: PROTOCOL_VERSION,
                theirs: wrong
            }),
        );
    }

    #[test]
    fn decode_rejects_trailing_garbage() {
        let mut bytes = encode(&Control::Ping(1));
        bytes.push(0xFF); // postcard must reject leftover bytes
        assert_eq!(decode::<Control>(&bytes), Err(DecodeError));
    }

    /// Poor-man's fuzz runnable on stable now (the real libfuzzer target is in `fuzz/`, run in CI):
    /// decode must never panic on arbitrary bytes, only return Ok or Err.
    #[test]
    fn decode_never_panics_on_arbitrary_bytes() {
        let mut state: u32 = 0x1234_5678;
        let mut next = || {
            // xorshift PRNG: deterministic, no dependency.
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for _ in 0..10_000 {
            let len = (next() % 64) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| (next() & 0xFF) as u8).collect();
            let _ = decode::<Control>(&bytes); // must not panic
        }
    }
}
