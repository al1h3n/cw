//! Version 1 of the control-channel wire format.
//!
//! Messages are encoded with [postcard](https://docs.rs/postcard) (compact, `no_std`-friendly,
//! serde-based). This module is a **trust boundary**: [`decode`] must be safe on arbitrary bytes
//! from a peer, so every type here avoids unbounded collections — the only variable-size field is
//! bounded by validation before it is trusted. New fields or variants require bumping
//! [`crate::PROTOCOL_VERSION`].

use serde::{Deserialize, Serialize};

use crate::{DeviceId, PROTOCOL_VERSION};

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
