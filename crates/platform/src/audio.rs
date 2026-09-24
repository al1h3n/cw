//! Playing one preloaded audio file, once, for a listening exam (feature 14).
//!
//! Uses Windows MCI (`mciSendStringW`, the multimedia control interface) rather than pulling an audio
//! playback crate: MCI plays WAV/MP3/WMA through the codecs already on the machine, needs no window
//! (so the student gets **no controls**), and is stopped cleanly by closing the device. That keeps the
//! Agent free of a second `cpal`/`alsa` stack that conflicts with the capture path's `cpal`.

/// Why audio playback failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    /// No implementation on this platform yet.
    #[error("audio playback is not supported on this platform")]
    NotSupported,
    /// MCI refused the command (bad file, missing codec) with this error code.
    #[error("could not play the audio (MCI error {0})")]
    Mci(u32),
}

/// A running one-shot playback. Dropping it stops the audio and releases the device.
pub struct Playback(imp::Playback);

impl Playback {
    /// Opens `path` and starts playing it once (no controls, no window).
    ///
    /// # Errors
    /// [`AudioError`] if the file cannot be opened or played.
    pub fn start(path: &std::path::Path) -> Result<Self, AudioError> {
        imp::Playback::start(path).map(Playback)
    }

    /// Whether playback has reached the end (or was stopped).
    #[must_use]
    pub fn finished(&self) -> bool {
        self.0.finished()
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows::{Win32::Media::Multimedia::mciSendStringW, core::PCWSTR};

    use super::AudioError;

    /// Makes each opened device a unique MCI alias, so overlapping plays never clash.
    static NEXT_ALIAS: AtomicU32 = AtomicU32::new(1);

    pub struct Playback {
        alias: String,
    }

    /// Sends one MCI command; `reply` (if given) receives the textual result. Returns the MCI code.
    fn mci(command: &str, reply: Option<&mut [u16]>) -> u32 {
        let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is a NUL-terminated command that outlives the call; `reply`, when present, is a
        // real slice we hand straight through. No callback window is used.
        unsafe { mciSendStringW(PCWSTR(wide.as_ptr()), reply, None) }
    }

    impl Playback {
        pub fn start(path: &std::path::Path) -> Result<Self, AudioError> {
            let alias = format!("cw_media_{}", NEXT_ALIAS.fetch_add(1, Ordering::Relaxed));
            let path = path.display();
            // Quote the path so spaces are handled; let MCI infer the device from the file.
            let code = mci(&format!("open \"{path}\" alias {alias}"), None);
            if code != 0 {
                return Err(AudioError::Mci(code));
            }
            let code = mci(&format!("play {alias}"), None);
            if code != 0 {
                let _ = mci(&format!("close {alias}"), None);
                return Err(AudioError::Mci(code));
            }
            Ok(Self { alias })
        }

        pub fn finished(&self) -> bool {
            let mut buffer = [0u16; 32];
            if mci(&format!("status {} mode", self.alias), Some(&mut buffer)) != 0 {
                return true; // the device is gone: treat as finished
            }
            let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
            let mode = String::from_utf16_lossy(&buffer[..end]);
            // "playing" (or "seeking") means still going; "stopped"/"paused"/"" means done.
            !(mode == "playing" || mode == "seeking")
        }
    }

    impl Drop for Playback {
        fn drop(&mut self) {
            let _ = mci(&format!("close {}", self.alias), None);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::AudioError;

    pub struct Playback;

    impl Playback {
        pub fn start(_path: &std::path::Path) -> Result<Self, AudioError> {
            // ponytail: Linux would play via PulseAudio/PipeWire or `paplay`; macOS via `afplay`.
            Err(AudioError::NotSupported)
        }
        pub fn finished(&self) -> bool {
            true
        }
    }
}
