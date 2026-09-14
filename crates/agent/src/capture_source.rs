//! The real [`net::AgentDevice`]: screen capture and audio from [`media`], actions from [`platform`].
//!
//! Keeping the trait in `net` and the implementation here means `media` stays a standalone capture
//! crate with no knowledge of the network, and the Agent binary is the only place the two meet.

use std::{path::Path, sync::Mutex, time::Duration};

use net::{AgentDevice, CaptureError, PeerInfo};
use proto::{Action, ActionFailure, ActionOutcome};

use crate::audit::AuditLog;

/// A [`AgentDevice`] backed by the real screen.
///
/// The underlying capturer keeps a Desktop Duplication warm and needs `&mut`, so it lives behind a
/// mutex: thumbnail requests are answered one at a time, which is exactly the pace we want.
pub struct ScreenCapture {
    capturer: Mutex<media::ThumbnailCapturer>,
    /// Present only while a Console is listening; dropping it stops the recording.
    audio: Mutex<Option<media::audio::AudioCapture>>,
    /// Every remote action is written here, whatever its outcome (D3).
    audit: AuditLog,
    /// Enforces the blocklist on its own thread, independent of any Console (D9: offline too).
    blocker: crate::blocker::Blocker,
}

impl ScreenCapture {
    /// Opens the screen capturer.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if the graphics device is unavailable (e.g. a headless session).
    pub fn new(audit_path: &Path, blocklist_path: &Path) -> Result<Self, CaptureError> {
        let capturer = media::ThumbnailCapturer::new().map_err(|e| CaptureError(e.to_string()))?;
        Ok(Self {
            capturer: Mutex::new(capturer),
            audio: Mutex::new(None),
            audit: AuditLog::new(audit_path),
            blocker: crate::blocker::Blocker::start(blocklist_path),
        })
    }

    /// How many blocklist rules are in force (for the `serve` banner).
    #[must_use]
    pub fn blocked_count(&self) -> u16 {
        self.blocker.rule_count()
    }

    /// How many monitors this device has.
    #[must_use]
    pub fn monitor_count(&self) -> u8 {
        self.capturer.lock().map_or(0, |c| c.monitor_count())
    }
}

/// Carries out one action on this PC through `platform::power`. Pure dispatch; auditing is the
/// caller's job so it happens for every outcome, including refusals.
fn carry_out(action: Action) -> ActionOutcome {
    use platform::power::{self, PowerError};

    let delay = |seconds: u16, reboot: bool| {
        let plan = power::plan_shutdown(Duration::from_secs(seconds.into()), reboot);
        u16::try_from(plan.timeout_seconds).unwrap_or(u16::MAX)
    };
    let result = match action {
        Action::Shutdown { delay_seconds } => {
            power::shutdown(Duration::from_secs(delay_seconds.into()), false)
                .map(|()| delay(delay_seconds, false))
        }
        Action::Reboot { delay_seconds } => {
            power::shutdown(Duration::from_secs(delay_seconds.into()), true)
                .map(|()| delay(delay_seconds, true))
        }
        Action::LogOff => power::log_off().map(|()| 0),
        Action::LockScreen => power::lock_screen().map(|()| 0),
        Action::CancelShutdown => power::cancel_shutdown().map(|()| 0),
        Action::LockWallpaper | Action::UnlockWallpaper => {
            let lock = matches!(action, Action::LockWallpaper);
            let result = if lock {
                platform::wallpaper::lock()
            } else {
                platform::wallpaper::unlock()
            };
            return match result {
                Ok(()) => ActionOutcome::Started { delay_seconds: 0 },
                Err(err) => {
                    eprintln!("{} failed: {err}", action.name());
                    ActionOutcome::Failed(ActionFailure::Failed)
                }
            };
        }
    };
    match result {
        Ok(delay_seconds) => ActionOutcome::Started { delay_seconds },
        Err(err) => {
            eprintln!("{} failed: {err}", action.name());
            ActionOutcome::Failed(match err {
                PowerError::NotPermitted => ActionFailure::NotPermitted,
                PowerError::NothingScheduled => ActionFailure::NothingScheduled,
                PowerError::NotSupported => ActionFailure::NotSupported,
                PowerError::Os(_) => ActionFailure::Failed,
            })
        }
    }
}

impl AgentDevice for ScreenCapture {
    fn perform(&self, from: &PeerInfo, action: Action) -> ActionOutcome {
        let outcome = carry_out(action);
        if let Err(err) =
            self.audit
                .record(net::endpoint::now_ms(), from.device_id, action, outcome)
        {
            eprintln!("audit log write failed: {err}");
        }
        println!(
            "console {} → {}: {outcome:?}",
            from.device_id,
            action.name()
        );
        outcome
    }

    fn monitors(&self) -> Vec<proto::Monitor> {
        self.capturer.lock().map_or_else(
            |_| Vec::new(),
            |c| {
                c.monitors()
                    .into_iter()
                    .map(|m| proto::Monitor {
                        index: m.index,
                        width: m.width,
                        height: m.height,
                        primary: m.primary,
                    })
                    .collect()
            },
        )
    }

    fn set_audio(&self, enabled: bool) -> Result<Option<proto::AudioFormat>, CaptureError> {
        let mut audio = self
            .audio
            .lock()
            .map_err(|_| CaptureError("audio lock poisoned".into()))?;
        if !enabled {
            // Dropping the capture stops the loopback stream and its thread.
            *audio = None;
            return Ok(None);
        }
        if let Some(existing) = audio.as_ref() {
            let format = existing.format();
            return Ok(Some(proto::AudioFormat {
                sample_rate: format.sample_rate,
                channels: format.channels,
            }));
        }
        let capture =
            media::audio::AudioCapture::start().map_err(|e| CaptureError(e.to_string()))?;
        let format = capture.format();
        *audio = Some(capture);
        Ok(Some(proto::AudioFormat {
            sample_rate: format.sample_rate,
            channels: format.channels,
        }))
    }

    fn take_audio(&self, max_samples: usize) -> Vec<i16> {
        self.audio.lock().map_or_else(
            |_| Vec::new(),
            |audio| {
                audio
                    .as_ref()
                    .map_or_else(Vec::new, |c| c.take(max_samples))
            },
        )
    }

    fn set_blocklist(&self, programs: Vec<String>) -> u16 {
        self.blocker.set_rules(programs)
    }

    fn take_blocked(&self) -> Vec<String> {
        self.blocker.take_closed()
    }

    fn capture_thumbnail(&self, monitor: u8, max_width: u16) -> Result<Vec<u8>, CaptureError> {
        let mut capturer = self
            .capturer
            .lock()
            .map_err(|_| CaptureError("capture lock poisoned".into()))?;
        capturer
            .capture_jpeg(monitor, max_width)
            .map_err(|e| CaptureError(e.to_string()))
    }
}
