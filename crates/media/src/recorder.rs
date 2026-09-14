//! Writing a screen recording to a file the teacher can just double-click.
//!
//! # Why Motion-JPEG in an AVI
//!
//! The brief asks for recordings at a chosen resolution and frame rate — for example a 2560×1440
//! 144 Hz screen saved as 1080p30. The resolution part is [`crate::resize`]; the frame rate part is
//! simply how often we ask for a frame, because we drive the capture loop ourselves.
//!
//! For the container, MJPEG-in-AVI is the lazy choice that actually works:
//!
//! * We already produce JPEG frames for the grid, so there is no new encoder and no new dependency.
//! * Every frame is independent, so a recording cut off by a power failure is still watchable up to
//!   the moment the power went — which is exactly what PLAN 2.12 asks for. A long-GOP format would
//!   lose everything back to the last keyframe.
//! * AVI's structure is four small headers and a chunk per frame. Windows Media Player, VLC and
//!   ffmpeg all open it by double-click, with no codec to install.
//!
//! The cost is size: MJPEG is perhaps 5–10× larger than H.264 for the same quality. That is an
//! acceptable trade for a lesson-length capture at 5–30 fps, and it is why PLAN 2.12 keeps the
//! hardware H.264 encoder as the upgrade. `ffmpeg -i lesson.avi lesson.mp4` converts one losslessly
//! into the other today.
//!
//! The index is rewritten every [`INDEX_EVERY`] frames, so an interrupted file is not merely
//! *recoverable* but already correct: at worst the last second of frames is missing.

use std::{
    fs::File,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

/// How often the header and index are rewritten, in frames.
const INDEX_EVERY: u32 = 30;

/// Errors writing a recording.
#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    /// The file could not be created or written.
    #[error("recording file: {0}")]
    Io(String),
    /// The requested settings are impossible.
    #[error("{0}")]
    BadSettings(String),
}

impl From<std::io::Error> for RecorderError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// What a recording should look like. Clamped on arrival so a remote peer cannot ask for nonsense.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingSettings {
    /// Longest edge of the saved video, in pixels. The real size keeps the screen's aspect ratio.
    pub max_width: u32,
    /// Tallest the saved video may be.
    pub max_height: u32,
    /// Frames per second to capture and to write into the file's header.
    pub fps: u32,
}

impl RecordingSettings {
    /// The smallest sensible recording.
    pub const MIN_WIDTH: u32 = 160;
    /// 4K is past anything a classroom needs and a lot of disk.
    pub const MAX_WIDTH: u32 = 3840;
    /// The tallest we will write.
    pub const MAX_HEIGHT: u32 = 2160;
    /// One frame a second still shows what a student was doing.
    pub const MIN_FPS: u32 = 1;
    /// Above 30 the file grows fast and adds nothing for a lesson recording.
    pub const MAX_FPS: u32 = 30;

    /// Clamps every field into a range this recorder can actually deliver.
    ///
    /// A 144 Hz screen recorded "at 144 fps" would produce an enormous file that no one watches; the
    /// clamp to 30 is deliberate and is reported back to the Console so the teacher sees what they
    /// really got rather than what they asked for.
    #[must_use]
    pub fn clamped(self) -> Self {
        Self {
            max_width: self.max_width.clamp(Self::MIN_WIDTH, Self::MAX_WIDTH),
            max_height: self.max_height.clamp(Self::MIN_WIDTH, Self::MAX_HEIGHT),
            fps: self.fps.clamp(Self::MIN_FPS, Self::MAX_FPS),
        }
    }

    /// How long to wait between frames.
    #[must_use]
    pub fn frame_interval(self) -> std::time::Duration {
        std::time::Duration::from_micros(1_000_000 / u64::from(self.fps.max(1)))
    }
}

impl Default for RecordingSettings {
    fn default() -> Self {
        // 720p at 10 fps: legible for "what was this student doing", and about a gigabyte an hour.
        Self {
            max_width: 1280,
            max_height: 720,
            fps: 10,
        }
    }
}

/// Writes JPEG frames into an AVI file as they arrive.
pub struct Recorder {
    file: File,
    path: PathBuf,
    width: u32,
    height: u32,
    fps: u32,
    /// Byte offset and length of every frame, for the index.
    frames: Vec<(u32, u32)>,
    /// Bytes written into the `movi` list so far.
    movi_bytes: u32,
    /// Largest single frame, which the AVI header must declare.
    largest_frame: u32,
}

impl Recorder {
    /// Creates the file and writes placeholder headers.
    ///
    /// # Errors
    /// [`RecorderError::Io`] if the file cannot be created, [`RecorderError::BadSettings`] for a
    /// zero-sized frame.
    pub fn create(path: &Path, width: u32, height: u32, fps: u32) -> Result<Self, RecorderError> {
        if width == 0 || height == 0 {
            return Err(RecorderError::BadSettings(
                "a recording needs a non-zero size".into(),
            ));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut recorder = Self {
            file: File::create(path)?,
            path: path.to_path_buf(),
            width,
            height,
            fps: fps.max(1),
            frames: Vec::new(),
            movi_bytes: 4, // the "movi" FourCC itself
            largest_frame: 0,
        };
        recorder.write_headers()?;
        Ok(recorder)
    }

    /// Where this recording is being written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// How many frames have been written.
    #[must_use]
    pub fn frame_count(&self) -> u32 {
        self.frames.len() as u32
    }

    /// How many seconds of video that is, at the chosen frame rate.
    #[must_use]
    pub fn seconds(&self) -> f32 {
        self.frame_count() as f32 / self.fps as f32
    }

    /// Appends one JPEG frame.
    ///
    /// # Errors
    /// [`RecorderError::Io`] if the write fails.
    pub fn push_jpeg(&mut self, jpeg: &[u8]) -> Result<(), RecorderError> {
        let length = u32::try_from(jpeg.len())
            .map_err(|_| RecorderError::BadSettings("frame too large".into()))?;
        // Chunk: "00dc" + size + data, padded to an even length as RIFF requires.
        self.file.write_all(b"00dc")?;
        self.file.write_all(&length.to_le_bytes())?;
        self.file.write_all(jpeg)?;
        let padding = length % 2;
        if padding == 1 {
            self.file.write_all(&[0])?;
        }
        // Offsets in the index are relative to the start of the "movi" FourCC.
        self.frames.push((self.movi_bytes, length));
        self.movi_bytes += 8 + length + padding;
        self.largest_frame = self.largest_frame.max(length);

        if self.frame_count().is_multiple_of(INDEX_EVERY) {
            self.flush_index()?;
        }
        Ok(())
    }

    /// Corrects the frame rate written into the file to the rate actually achieved.
    ///
    /// This matters more than it looks. If capture cannot keep up with the requested rate — a
    /// 1440p screen resized properly is real work — then a file that still *claims* 30 fps plays
    /// back too fast, and a lesson recording becomes a comedy. Writing the measured rate instead
    /// makes the video play in real time, whatever the PC managed.
    ///
    /// Ignored when fewer than two frames were captured, where there is no rate to measure.
    pub fn set_measured_fps(&mut self, elapsed_seconds: f32) {
        let frames = self.frame_count();
        if frames < 2 || elapsed_seconds <= 0.0 {
            return;
        }
        let measured = (frames as f32 / elapsed_seconds).round() as u32;
        self.fps = measured.clamp(1, RecordingSettings::MAX_FPS);
    }

    /// The frame rate this file will declare.
    #[must_use]
    pub fn fps(&self) -> u32 {
        self.fps
    }

    /// Writes the final index and flushes. Call before dropping to leave a tidy file.
    ///
    /// # Errors
    /// [`RecorderError::Io`] if the write fails.
    pub fn finish(mut self) -> Result<PathBuf, RecorderError> {
        self.flush_index()?;
        Ok(self.path.clone())
    }

    /// Rewrites the headers and appends a fresh index, then returns to the end of the frame data.
    ///
    /// Doing this periodically is what makes an interrupted recording playable: the file on disk is
    /// a complete, correct AVI as of the last flush.
    fn flush_index(&mut self) -> Result<(), RecorderError> {
        let movi_end = SeekFrom::Start(u64::from(HEADER_BYTES) + u64::from(self.movi_bytes) - 4);
        // The index goes immediately after the frame data.
        self.file.seek(movi_end)?;
        self.file.write_all(b"idx1")?;
        let index_bytes = u32::try_from(self.frames.len() * 16).unwrap_or(0);
        self.file.write_all(&index_bytes.to_le_bytes())?;
        for (offset, length) in &self.frames {
            self.file.write_all(b"00dc")?;
            self.file.write_all(&0x10u32.to_le_bytes())?; // AVIIF_KEYFRAME: every frame is one
            self.file.write_all(&offset.to_le_bytes())?;
            self.file.write_all(&length.to_le_bytes())?;
        }
        // Now that the sizes are known, rewrite the headers in place.
        self.file.seek(SeekFrom::Start(0))?;
        self.write_headers()?;
        self.file.flush()?;
        // Back to the end of the frame data so the next frame appends in the right place.
        self.file.seek(movi_end)?;
        Ok(())
    }

    /// Writes the RIFF/AVI headers using the current frame count and sizes.
    fn write_headers(&mut self) -> Result<(), RecorderError> {
        let frames = self.frame_count();
        let index_bytes = frames * 16 + 8;
        // RIFF size covers everything after the size field itself. `movi_bytes` counts the "movi"
        // FourCC, which HEADER_BYTES already includes, so subtract it once: 8 for the RIFF size
        // field and FourCC, plus 4 for the double-counted "movi".
        let riff_size = HEADER_BYTES - 12 + self.movi_bytes + index_bytes;
        let micros_per_frame = 1_000_000 / self.fps.max(1);

        let mut header = Vec::with_capacity(HEADER_BYTES as usize);
        header.extend_from_slice(b"RIFF");
        header.extend_from_slice(&riff_size.to_le_bytes());
        header.extend_from_slice(b"AVI ");

        // hdrl list: the main header plus one video stream.
        header.extend_from_slice(b"LIST");
        header.extend_from_slice(&208u32.to_le_bytes());
        header.extend_from_slice(b"hdrl");

        // avih — the main AVI header.
        header.extend_from_slice(b"avih");
        header.extend_from_slice(&56u32.to_le_bytes());
        header.extend_from_slice(&micros_per_frame.to_le_bytes());
        header.extend_from_slice(&(self.largest_frame * self.fps).to_le_bytes()); // max data rate
        header.extend_from_slice(&0u32.to_le_bytes()); // padding granularity
        header.extend_from_slice(&0x10u32.to_le_bytes()); // AVIF_HASINDEX
        header.extend_from_slice(&frames.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes()); // initial frames
        header.extend_from_slice(&1u32.to_le_bytes()); // streams
        header.extend_from_slice(&self.largest_frame.to_le_bytes());
        header.extend_from_slice(&self.width.to_le_bytes());
        header.extend_from_slice(&self.height.to_le_bytes());
        header.extend_from_slice(&[0u8; 16]); // reserved

        // strl list: stream header + stream format.
        header.extend_from_slice(b"LIST");
        header.extend_from_slice(&132u32.to_le_bytes());
        header.extend_from_slice(b"strl");

        // strh — stream header.
        header.extend_from_slice(b"strh");
        header.extend_from_slice(&56u32.to_le_bytes());
        header.extend_from_slice(b"vids");
        header.extend_from_slice(b"MJPG");
        header.extend_from_slice(&0u32.to_le_bytes()); // flags
        header.extend_from_slice(&0u16.to_le_bytes()); // priority
        header.extend_from_slice(&0u16.to_le_bytes()); // language
        header.extend_from_slice(&0u32.to_le_bytes()); // initial frames
        header.extend_from_slice(&1u32.to_le_bytes()); // scale
        header.extend_from_slice(&self.fps.to_le_bytes()); // rate -> fps = rate/scale
        header.extend_from_slice(&0u32.to_le_bytes()); // start
        header.extend_from_slice(&frames.to_le_bytes()); // length
        header.extend_from_slice(&self.largest_frame.to_le_bytes());
        header.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes()); // quality: default
        header.extend_from_slice(&0u32.to_le_bytes()); // sample size: variable
        header.extend_from_slice(&0u16.to_le_bytes()); // rcFrame left
        header.extend_from_slice(&0u16.to_le_bytes()); // top
        header.extend_from_slice(&(self.width as u16).to_le_bytes());
        header.extend_from_slice(&(self.height as u16).to_le_bytes());

        // strf — BITMAPINFOHEADER describing the MJPEG frames.
        header.extend_from_slice(b"strf");
        header.extend_from_slice(&40u32.to_le_bytes());
        header.extend_from_slice(&40u32.to_le_bytes()); // biSize
        header.extend_from_slice(&self.width.to_le_bytes());
        header.extend_from_slice(&self.height.to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes()); // planes
        header.extend_from_slice(&24u16.to_le_bytes()); // bit count
        header.extend_from_slice(b"MJPG");
        header.extend_from_slice(&(self.width * self.height * 3).to_le_bytes()); // image size
        header.extend_from_slice(&0u32.to_le_bytes()); // x pixels per metre
        header.extend_from_slice(&0u32.to_le_bytes()); // y pixels per metre
        header.extend_from_slice(&0u32.to_le_bytes()); // colours used
        header.extend_from_slice(&0u32.to_le_bytes()); // colours important

        // movi list header. Its size covers the FourCC plus every frame chunk.
        header.extend_from_slice(b"LIST");
        header.extend_from_slice(&self.movi_bytes.to_le_bytes());
        header.extend_from_slice(b"movi");

        debug_assert_eq!(
            header.len() as u32,
            HEADER_BYTES,
            "HEADER_BYTES must match what write_headers emits"
        );
        self.file.write_all(&header)?;
        Ok(())
    }
}

/// Exact size of everything written before the first frame chunk.
///
/// 12 (RIFF) + 12 (hdrl LIST) + 64 (avih) + 12 (strl LIST) + 64 (strh) + 48 (strf) + 12 (movi LIST).
const HEADER_BYTES: u32 = 224;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cw-rec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(format!("{tag}.avi"))
    }

    /// A tiny but structurally valid JPEG (SOI … EOI). Enough to exercise the container.
    fn fake_jpeg(size: usize) -> Vec<u8> {
        let mut data = vec![0xFF, 0xD8];
        data.resize(size - 2, 0x42);
        data.extend_from_slice(&[0xFF, 0xD9]);
        data
    }

    #[test]
    fn settings_clamp_an_absurd_request_into_something_deliverable() {
        let asked = RecordingSettings {
            max_width: 99_999,
            max_height: 99_999,
            fps: 144,
        };
        let got = asked.clamped();
        assert_eq!(got.max_width, RecordingSettings::MAX_WIDTH);
        assert_eq!(got.max_height, RecordingSettings::MAX_HEIGHT);
        assert_eq!(
            got.fps,
            RecordingSettings::MAX_FPS,
            "144 Hz clamps to 30 fps"
        );
    }

    #[test]
    fn settings_clamp_upwards_too() {
        let got = RecordingSettings {
            max_width: 1,
            max_height: 1,
            fps: 0,
        }
        .clamped();
        assert_eq!(got.max_width, RecordingSettings::MIN_WIDTH);
        assert_eq!(got.fps, RecordingSettings::MIN_FPS);
    }

    #[test]
    fn the_frame_interval_matches_the_frame_rate() {
        let thirty = RecordingSettings {
            fps: 30,
            ..RecordingSettings::default()
        };
        assert_eq!(thirty.frame_interval().as_micros(), 33_333);
        let one = RecordingSettings {
            fps: 1,
            ..RecordingSettings::default()
        };
        assert_eq!(one.frame_interval().as_secs(), 1);
    }

    #[test]
    fn a_zero_sized_recording_is_refused() {
        assert!(matches!(
            Recorder::create(&temp_path("zero"), 0, 720, 10),
            Err(RecorderError::BadSettings(_))
        ));
    }

    #[test]
    fn a_finished_file_starts_with_a_riff_avi_signature_and_has_the_frames() {
        let path = temp_path("basic");
        let mut recorder = Recorder::create(&path, 640, 360, 10).expect("create");
        for _ in 0..3 {
            recorder.push_jpeg(&fake_jpeg(100)).expect("push");
        }
        assert_eq!(recorder.frame_count(), 3);
        let written = recorder.finish().expect("finish");

        let bytes = std::fs::read(&written).expect("read");
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"AVI ");
        assert_eq!(
            &bytes[HEADER_BYTES as usize - 4..HEADER_BYTES as usize],
            b"movi"
        );
        // Frame count lands in the avih header.
        let frames = u32::from_le_bytes(bytes[48..52].try_into().unwrap());
        assert_eq!(frames, 3, "the header records three frames");
        assert!(
            bytes.windows(4).any(|w| w == b"idx1"),
            "the index must be present"
        );
        let _ = std::fs::remove_file(&written);
    }

    #[test]
    fn the_declared_riff_size_matches_the_real_file_length() {
        // A wrong size here is exactly what makes a player refuse to open a file.
        let path = temp_path("size");
        let mut recorder = Recorder::create(&path, 320, 240, 5).expect("create");
        for n in 0..7 {
            recorder.push_jpeg(&fake_jpeg(50 + n * 10)).expect("push");
        }
        let written = recorder.finish().expect("finish");
        let bytes = std::fs::read(&written).expect("read");
        let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(
            declared as usize + 8,
            bytes.len(),
            "RIFF size must describe the whole file"
        );
        let _ = std::fs::remove_file(&written);
    }

    #[test]
    fn an_odd_length_frame_is_padded_so_the_next_chunk_stays_aligned() {
        let path = temp_path("odd");
        let mut recorder = Recorder::create(&path, 320, 240, 5).expect("create");
        recorder.push_jpeg(&fake_jpeg(101)).expect("odd frame");
        recorder.push_jpeg(&fake_jpeg(51)).expect("odd frame");
        let written = recorder.finish().expect("finish");
        let bytes = std::fs::read(&written).expect("read");
        // The second chunk header must start on an even offset after the padded first.
        let second = HEADER_BYTES as usize + 8 + 101 + 1;
        assert_eq!(&bytes[second..second + 4], b"00dc");
        let _ = std::fs::remove_file(&written);
    }

    #[test]
    fn a_recording_interrupted_without_finish_is_still_a_complete_file() {
        // Simulates power loss: write enough frames to trigger an index flush, then drop the
        // recorder without calling finish(). PLAN 2.12 requires the file to remain playable.
        let path = temp_path("crash");
        let mut recorder = Recorder::create(&path, 320, 240, 10).expect("create");
        for _ in 0..INDEX_EVERY {
            recorder.push_jpeg(&fake_jpeg(60)).expect("push");
        }
        drop(recorder); // no finish()

        let bytes = std::fs::read(&path).expect("read");
        assert_eq!(&bytes[0..4], b"RIFF");
        let frames = u32::from_le_bytes(bytes[48..52].try_into().unwrap());
        assert_eq!(frames, INDEX_EVERY, "the flushed header knows its frames");
        assert!(bytes.windows(4).any(|w| w == b"idx1"));
        let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(declared as usize + 8, bytes.len());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn seconds_follow_the_frame_rate_not_the_wall_clock() {
        let path = temp_path("secs");
        let mut recorder = Recorder::create(&path, 320, 240, 10).expect("create");
        for _ in 0..25 {
            recorder.push_jpeg(&fake_jpeg(40)).expect("push");
        }
        assert!((recorder.seconds() - 2.5).abs() < 0.001);
        let written = recorder.finish().expect("finish");
        let _ = std::fs::remove_file(&written);
    }
}
