//! Recording the screen through an external `ffmpeg`, when one is present, for real codecs, presets
//! and quality control that the built-in MJPEG writer cannot offer.
//!
//! We do not link ffmpeg; we pipe **raw BGRA frames** to its standard input and let it scale (with
//! the chosen filter) and encode. That keeps the licensing simple (ffmpeg stays a separate program
//! the school drops next to the Agent) and means every codec ffmpeg supports is available for free.
//! If no ffmpeg is found the caller falls back to the MJPEG recorder, so recording always works.

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
};

/// How ffmpeg should encode — plain values the caller maps from its own wire types, so this module
/// depends on nothing above `media`.
pub struct FfmpegOptions<'a> {
    /// Output width in pixels (even).
    pub out_width: u32,
    /// Output height in pixels (even).
    pub out_height: u32,
    /// Frame rate the input is fed and the output is tagged at.
    pub fps: u32,
    /// The video encoder library, e.g. `libx264`, `libx265`, `libsvtav1`.
    pub codec_lib: &'a str,
    /// The encoder preset, e.g. `medium` (x264/x265) or a number string (svtav1).
    pub preset: &'a str,
    /// Constant-quality value (CRF): lower is better and larger.
    pub crf: u8,
    /// Maximum consecutive B-frames.
    pub bframes: u8,
    /// Whether to pass `-bf` (x264/x265 honour it; svtav1 does not).
    pub use_bframes: bool,
    /// The scaling filter flag, e.g. `lanczos`, `bicubic`, `bilinear`, `neighbor`.
    pub scaler: &'a str,
}

/// Finds an `ffmpeg` to use: one shipped next to our own binary first, then one on `PATH`.
///
/// Returns `None` if neither is present, so the caller can fall back to the built-in recorder.
#[must_use]
pub fn find_ffmpeg() -> Option<PathBuf> {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let local = dir.join(name);
        if local.exists() {
            return Some(local);
        }
    }
    // On PATH? Probe with `-version`, which prints and exits immediately.
    let ok = Command::new(name)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    ok.then(|| PathBuf::from(name))
}

/// A running ffmpeg encode: raw BGRA in, an encoded file out.
pub struct FfmpegRecorder {
    child: Child,
    stdin: Option<ChildStdin>,
    frame_bytes: usize,
    frames: u32,
    fps: u32,
}

impl FfmpegRecorder {
    /// Starts ffmpeg to read raw BGRA of `src_w`x`src_h` and write `output`.
    ///
    /// # Errors
    /// A message if ffmpeg cannot be started.
    pub fn create(
        ffmpeg: &Path,
        output: &Path,
        src_w: u32,
        src_h: u32,
        opts: &FfmpegOptions,
    ) -> Result<Self, String> {
        let mut command = Command::new(ffmpeg);
        command
            .arg("-y")
            .args(["-f", "rawvideo", "-pixel_format", "bgra"])
            .args(["-video_size", &format!("{src_w}x{src_h}")])
            .args(["-framerate", &opts.fps.to_string()])
            .args(["-i", "pipe:0"])
            .args([
                "-vf",
                &format!(
                    "scale={}:{}:flags={}",
                    opts.out_width, opts.out_height, opts.scaler
                ),
            ])
            .args(["-c:v", opts.codec_lib])
            .args(["-preset", opts.preset])
            .args(["-crf", &opts.crf.to_string()]);
        if opts.use_bframes {
            command.args(["-bf", &opts.bframes.to_string()]);
        }
        command
            .args(["-pix_fmt", "yuv420p"])
            .arg(output)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("could not start ffmpeg: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "ffmpeg gave no stdin".to_string())?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            frame_bytes: (src_w as usize) * (src_h as usize) * 4,
            frames: 0,
            fps: opts.fps,
        })
    }

    /// Feeds one native-size BGRA frame to ffmpeg.
    ///
    /// # Errors
    /// A message if the write fails (ffmpeg exited).
    pub fn push_bgra(&mut self, frame: &[u8]) -> Result<(), String> {
        if frame.len() < self.frame_bytes {
            return Ok(()); // a blank/partial grab: skip rather than corrupt the stream
        }
        let Some(stdin) = self.stdin.as_mut() else {
            return Err("ffmpeg input already closed".into());
        };
        stdin
            .write_all(&frame[..self.frame_bytes])
            .map_err(|e| format!("ffmpeg write: {e}"))?;
        self.frames += 1;
        Ok(())
    }

    /// How many frames have been fed.
    #[must_use]
    pub fn frame_count(&self) -> u32 {
        self.frames
    }

    /// Seconds of video that represents at the chosen rate.
    #[must_use]
    pub fn seconds(&self) -> f32 {
        if self.fps == 0 {
            0.0
        } else {
            self.frames as f32 / self.fps as f32
        }
    }

    /// Closes the input and waits for ffmpeg to finish writing the file.
    ///
    /// # Errors
    /// A message if ffmpeg exited with a failure.
    pub fn finish(mut self) -> Result<(), String> {
        drop(self.stdin.take()); // EOF: ffmpeg flushes and exits
        let status = self
            .child
            .wait()
            .map_err(|e| format!("waiting for ffmpeg: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("ffmpeg exited with {status}"))
        }
    }
}

impl Drop for FfmpegRecorder {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_synthetic_frames_to_a_real_mp4() {
        // Only runs where ffmpeg is installed (dev/CI with it on PATH); skips otherwise.
        let Some(ffmpeg) = find_ffmpeg() else {
            eprintln!("ffmpeg not found; skipping");
            return;
        };
        let out = std::env::temp_dir().join(format!("cw-ffmpeg-{}.mp4", std::process::id()));
        let opts = FfmpegOptions {
            out_width: 320,
            out_height: 180,
            fps: 10,
            codec_lib: "libx264",
            preset: "ultrafast",
            crf: 28,
            bframes: 3,
            use_bframes: true,
            scaler: "lanczos",
        };
        let mut rec =
            FfmpegRecorder::create(&ffmpeg, &out, 640, 360, &opts).expect("ffmpeg should start");
        let frame = vec![120u8; 640 * 360 * 4];
        for _ in 0..15 {
            rec.push_bgra(&frame).expect("push a frame");
        }
        assert_eq!(rec.frame_count(), 15);
        rec.finish().expect("ffmpeg should finish cleanly");

        let bytes = std::fs::read(&out).expect("the mp4 exists");
        assert!(bytes.len() > 100, "the mp4 should not be empty");
        assert_eq!(&bytes[4..8], b"ftyp", "a real mp4 begins with an ftyp box");
        let _ = std::fs::remove_file(&out);
    }
}
