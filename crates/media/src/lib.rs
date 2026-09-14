//! Screen capture and encoding.
//!
//! Today: Windows thumbnails via DXGI Desktop Duplication, downscaled on the GPU and JPEG-encoded
//! (the pipeline measured in Phase-0 spike 0.4 at under 0.03 % of one core). Other platforms and the
//! full-resolution H.264 path land in later phases.
//!
//! This crate is one of the two allowed to contain `unsafe` (AGENTS.md §5); every block is FFI with a
//! `// SAFETY:` comment.
#![allow(unsafe_code)]

/// A capture failure.
#[derive(Debug, thiserror::Error)]
#[error("capture failed: {0}")]
pub struct CaptureError(pub String);

impl CaptureError {
    fn new(what: impl std::fmt::Display) -> Self {
        Self(what.to_string())
    }
}

#[cfg(windows)]
mod gdi;
#[cfg(windows)]
mod windows_capture;

#[cfg(windows)]
pub use windows_capture::ThumbnailCapturer;

#[cfg(not(windows))]
mod stub {
    use super::CaptureError;

    /// Placeholder until the Linux/macOS backends land (Phase 5).
    #[derive(Debug, Default)]
    pub struct ThumbnailCapturer;

    impl ThumbnailCapturer {
        /// Creates a capturer.
        ///
        /// # Errors
        /// Always fails on non-Windows platforms for now.
        pub fn new() -> Result<Self, CaptureError> {
            Err(CaptureError("screen capture is Windows-only so far".into()))
        }

        /// Captures a thumbnail.
        ///
        /// # Errors
        /// Always fails on non-Windows platforms for now.
        pub fn capture_jpeg(
            &mut self,
            _monitor: u8,
            _max_width: u16,
        ) -> Result<Vec<u8>, CaptureError> {
            Err(CaptureError("screen capture is Windows-only so far".into()))
        }

        /// Number of monitors.
        #[must_use]
        pub fn monitor_count(&self) -> u8 {
            0
        }
    }
}

#[cfg(not(windows))]
pub use stub::ThumbnailCapturer;
