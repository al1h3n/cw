//! Glue between the real screen capture ([`media`]) and the transport's [`net::CaptureSource`].
//!
//! Keeping the trait in `net` and the implementation here means `media` stays a standalone capture
//! crate with no knowledge of the network, and the Agent binary is the only place the two meet.

use std::sync::Mutex;

use net::{CaptureError, CaptureSource};

/// A [`CaptureSource`] backed by the real screen.
///
/// The underlying capturer keeps a Desktop Duplication warm and needs `&mut`, so it lives behind a
/// mutex: thumbnail requests are answered one at a time, which is exactly the pace we want.
pub struct ScreenCapture {
    capturer: Mutex<media::ThumbnailCapturer>,
    /// Present only while a Console is listening; dropping it stops the recording.
    audio: Mutex<Option<media::audio::AudioCapture>>,
}

impl ScreenCapture {
    /// Opens the screen capturer.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if the graphics device is unavailable (e.g. a headless session).
    pub fn new() -> Result<Self, CaptureError> {
        let capturer = media::ThumbnailCapturer::new().map_err(|e| CaptureError(e.to_string()))?;
        Ok(Self {
            capturer: Mutex::new(capturer),
            audio: Mutex::new(None),
        })
    }

    /// How many monitors this device has.
    #[must_use]
    pub fn monitor_count(&self) -> u8 {
        self.capturer.lock().map_or(0, |c| c.monitor_count())
    }
}

impl CaptureSource for ScreenCapture {
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
