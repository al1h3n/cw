//! Recording this PC's screen to a file, at a resolution and frame rate the teacher chooses.
//!
//! The recording runs **on the student PC** (PLAN 2.12), on its own thread, and keeps running
//! whether or not a Console stays connected — a lesson recording that stops because the teacher
//! closed their laptop would be useless. Each recording gets a [`proto::RecordId`] (UUIDv7), so file
//! names sort in the order the recordings were made and never collide, with no central counter.
//!
//! The pipeline per frame is: grab the screen at its native size → [`media::resize::area_average`]
//! down to the chosen size → JPEG → append to the AVI. Doing the resize properly (rather than
//! sampling every Nth pixel) is what makes 1440p-to-1080p text stay readable.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Instant,
};

use media::{
    ffmpeg::{FfmpegOptions, FfmpegRecorder, find_ffmpeg},
    recorder::{Recorder, RecordingSettings},
    resize,
};
use proto::RecordOptions;

/// How long `start` waits for the worker to report the size it settled on.
const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// What a Console can see about a recording in progress.
#[derive(Debug, Clone, Default)]
pub struct RecordingStatus {
    /// Whether a recording is running.
    pub active: bool,
    /// File name (not the full path — the Console has no business knowing the student's disk layout).
    pub file: String,
    /// Frames written so far.
    pub frames: u32,
    /// Seconds of video that represents.
    pub seconds: f32,
    /// The size actually being recorded, after clamping and aspect-ratio fitting.
    pub width: u32,
    /// Height actually being recorded.
    pub height: u32,
    /// The frame rate actually being used, after clamping.
    pub fps: u32,
    /// Set when the recording stopped by itself, so a teacher learns why.
    pub problem: String,
}

/// A running screen recording.
pub struct Recording {
    status: Arc<Mutex<RecordingStatus>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Recording {
    /// Starts recording `monitor` into `dir`, returning the handle that owns the worker thread.
    ///
    /// Settings are clamped before use, and the clamped values are what the status reports — so a
    /// teacher who asks for 144 fps sees that they are getting 30, rather than being quietly ignored.
    #[must_use]
    pub fn start(dir: &Path, monitor: u8, options: RecordOptions) -> Self {
        let options = clamp_options(options);
        // Prefer ffmpeg (real codecs/presets/quality) when it is present; else the built-in MJPEG.
        let ffmpeg = find_ffmpeg();
        let id = crate::record_id::now();
        let ext = if ffmpeg.is_some() { "mp4" } else { "avi" };
        let file = format!("recording-{}.{ext}", id.to_compact());
        let path = dir.join(&file);

        let status = Arc::new(Mutex::new(RecordingStatus {
            active: true,
            file,
            fps: options.fps,
            ..RecordingStatus::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));

        let worker = {
            let status = Arc::clone(&status);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || run(&path, monitor, options, ffmpeg, &status, &stop))
        };

        // Wait briefly for the worker to publish the real output size, so the Console is told
        // what it is actually getting rather than 0x0. Bounded, because a PC that cannot capture
        // at all must still return promptly with its problem reported.
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            let ready = {
                let status = status.lock().unwrap_or_else(|e| e.into_inner());
                status.width > 0 || !status.problem.is_empty() || !status.active
            };
            if ready {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        Self {
            status,
            stop,
            worker: Some(worker),
        }
    }

    /// A snapshot of how the recording is going.
    #[must_use]
    pub fn status(&self) -> RecordingStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Stops the recording, waits for the file to be closed, and returns the **final** status.
    ///
    /// Reading [`status`](Self::status) and then dropping would report the rate that was *asked
    /// for*, because the worker only writes the rate it actually achieved as it closes the file.
    /// Joining first is what makes the reply honest.
    #[must_use]
    pub fn finish(mut self) -> RecordingStatus {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.status()
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The recording loop: capture, feed the sink, wait for the next frame time.
fn run(
    path: &Path,
    monitor: u8,
    options: RecordOptions,
    ffmpeg: Option<PathBuf>,
    status: &Arc<Mutex<RecordingStatus>>,
    stop: &Arc<AtomicBool>,
) {
    /// Records why the recording ended and marks it finished.
    fn fail(status: &Arc<Mutex<RecordingStatus>>, problem: String) {
        let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
        status.active = false;
        status.problem = problem;
    }

    // The capturer must live on this thread: it holds Direct3D objects that are not Send.
    let mut capturer = match media::ThumbnailCapturer::new() {
        Ok(capturer) => capturer,
        Err(err) => return fail(status, format!("no screen capture: {err}")),
    };

    // Take one frame first, so the output size follows the real screen's shape.
    let (first, src_w, src_h) = match capturer.capture_bgra(monitor) {
        Ok(frame) => frame,
        Err(err) => return fail(status, format!("capture failed: {err}")),
    };
    let (width, height) = resize::fit_within(src_w, src_h, options.max_width, options.max_height);

    let mut sink = match make_sink(
        ffmpeg.as_deref(),
        path,
        src_w,
        src_h,
        width,
        height,
        &options,
    ) {
        Ok(sink) => sink,
        Err(err) => return fail(status, err),
    };
    {
        let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
        status.width = width;
        status.height = height;
    }

    let interval = frame_interval(options.fps);
    let mut frame = Some((first, src_w, src_h));
    let started_at = Instant::now();
    let mut next_frame_at = Instant::now();

    while !stop.load(Ordering::SeqCst) {
        let captured = match frame.take() {
            Some(first) => Ok(first),
            None => capturer.capture_bgra(monitor),
        };
        match captured {
            Ok((pixels, w, h)) => {
                if let Err(err) = sink.push(&pixels, w, h) {
                    return fail(status, format!("writing the recording failed: {err}"));
                }
                let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
                status.frames = sink.frames();
                status.seconds = sink.seconds();
            }
            Err(err) => {
                // A single failed grab (a UAC prompt, a mode change) must not end the lesson's
                // recording; only report it and try again on the next tick.
                let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
                status.problem = format!("a frame was missed: {err}");
            }
        }

        // Sleep until the next frame is due, in slices so stopping stays responsive.
        next_frame_at += interval;
        while Instant::now() < next_frame_at {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let remaining = next_frame_at.saturating_duration_since(Instant::now());
            std::thread::sleep(remaining.min(std::time::Duration::from_millis(50)));
        }
        // If capture ran slower than the frame interval, do not try to catch up in a burst.
        if next_frame_at < Instant::now() {
            next_frame_at = Instant::now();
        }
    }

    let elapsed = started_at.elapsed().as_secs_f32();
    let (real_fps, frames, finished) = sink.finalize(elapsed, options.fps);

    // Two-pass: a live pipe cannot do it, so re-encode the finished ffmpeg file in the background for
    // a smaller file at the same quality. It replaces the file in place when done; a teacher who
    // downloads before then simply gets the single-pass version, which is already valid.
    if finished.is_ok()
        && options.two_pass
        && path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
        && let Some(ffmpeg) = ffmpeg
    {
        let (codec_lib, preset, use_bframes, _) = ffmpeg_params(&options);
        let target = two_pass_kbps(width, height, options.fps);
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            let temp = path.with_extension("2pass.mp4");
            let opts = media::ffmpeg::TwoPass {
                codec_lib: &codec_lib,
                preset: &preset,
                bframes: options.bframes,
                use_bframes,
                target_kbps: target,
            };
            match media::ffmpeg::reencode_two_pass(&ffmpeg, &path, &temp, &opts) {
                Ok(()) => {
                    let _ = std::fs::rename(&temp, &path);
                    println!("two-pass re-encode done: {}", path.display());
                }
                Err(err) => {
                    eprintln!("two-pass re-encode failed: {err}");
                    let _ = std::fs::remove_file(&temp);
                }
            }
        });
    }

    let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
    status.active = false;
    status.fps = real_fps;
    status.frames = frames;
    status.seconds = frames as f32 / real_fps.max(1) as f32;
    if real_fps < options.fps {
        status.problem = format!(
            "this PC managed {real_fps} fps, not {}; the file plays at {real_fps} fps so it is \
             the right speed",
            options.fps
        );
    }
    if let Err(err) = finished {
        status.problem = format!("closing the recording failed: {err}");
    }
}

/// A two-pass target bitrate (kbit/s) for screen content: ~0.07 bits per pixel per frame, clamped.
fn two_pass_kbps(width: u32, height: u32, fps: u32) -> u32 {
    let bits = f64::from(width) * f64::from(height) * f64::from(fps) * 0.07;
    ((bits / 1000.0) as u32).clamp(500, 20_000)
}

/// The frame interval for a target rate (at least 1 fps).
fn frame_interval(fps: u32) -> std::time::Duration {
    std::time::Duration::from_secs_f64(1.0 / f64::from(fps.max(1)))
}

/// Clamps the teacher's choices into ranges the recorder can deliver.
fn clamp_options(mut o: RecordOptions) -> RecordOptions {
    o.max_width = o
        .max_width
        .clamp(RecordingSettings::MIN_WIDTH, RecordingSettings::MAX_WIDTH)
        & !1;
    o.max_height = o
        .max_height
        .clamp(RecordingSettings::MIN_WIDTH, RecordingSettings::MAX_HEIGHT)
        & !1;
    o.fps = o
        .fps
        .clamp(RecordingSettings::MIN_FPS, RecordingSettings::MAX_FPS);
    o.quality = o.quality.min(51);
    o.bframes = o.bframes.min(16);
    o
}

/// Where finished frames go: ffmpeg (real codecs) if present, else the built-in MJPEG writer.
enum Sink {
    Ffmpeg(FfmpegRecorder),
    Mjpeg {
        recorder: Recorder,
        out_w: u32,
        out_h: u32,
    },
}

impl Sink {
    fn push(&mut self, native: &[u8], src_w: u32, src_h: u32) -> Result<(), String> {
        match self {
            Sink::Ffmpeg(rec) => rec.push_bgra(native),
            Sink::Mjpeg {
                recorder,
                out_w,
                out_h,
            } => {
                let scaled = resize::area_average(native, src_w, src_h, *out_w, *out_h);
                if scaled.is_empty() {
                    return Ok(());
                }
                let jpeg =
                    media::encode_bgra(&scaled, *out_w, *out_h).map_err(|e| e.to_string())?;
                recorder.push_jpeg(&jpeg).map_err(|e| e.to_string())
            }
        }
    }

    fn frames(&self) -> u32 {
        match self {
            Sink::Ffmpeg(rec) => rec.frame_count(),
            Sink::Mjpeg { recorder, .. } => recorder.frame_count(),
        }
    }

    fn seconds(&self) -> f32 {
        match self {
            Sink::Ffmpeg(rec) => rec.seconds(),
            Sink::Mjpeg { recorder, .. } => recorder.seconds(),
        }
    }

    /// Closes the file and returns `(fps written, frame count, result)`.
    fn finalize(self, elapsed: f32, target_fps: u32) -> (u32, u32, Result<(), String>) {
        match self {
            // ffmpeg tags at the requested rate; the loop paces to it, so it is real-time when the
            // PC keeps up (and slightly fast if it cannot — a known limit of a live pipe).
            Sink::Ffmpeg(rec) => {
                let frames = rec.frame_count();
                (target_fps, frames, rec.finish())
            }
            Sink::Mjpeg { mut recorder, .. } => {
                recorder.set_measured_fps(elapsed);
                let (fps, frames) = (recorder.fps(), recorder.frame_count());
                let result = recorder.finish().map(|_| ()).map_err(|e| e.to_string());
                (fps, frames, result)
            }
        }
    }
}

/// Builds the sink, translating the wire options into ffmpeg arguments.
fn make_sink(
    ffmpeg: Option<&Path>,
    path: &Path,
    src_w: u32,
    src_h: u32,
    out_w: u32,
    out_h: u32,
    options: &RecordOptions,
) -> Result<Sink, String> {
    if let Some(ffmpeg) = ffmpeg {
        let (codec_lib, preset, use_bframes, scaler) = ffmpeg_params(options);
        let opts = FfmpegOptions {
            out_width: out_w,
            out_height: out_h,
            fps: options.fps,
            codec_lib: &codec_lib,
            preset: &preset,
            crf: options.quality,
            bframes: options.bframes,
            use_bframes,
            scaler: &scaler,
        };
        let rec = FfmpegRecorder::create(ffmpeg, path, src_w, src_h, &opts)
            .map_err(|e| format!("ffmpeg: {e}"))?;
        Ok(Sink::Ffmpeg(rec))
    } else {
        let rec = Recorder::create(path, out_w, out_h, options.fps)
            .map_err(|e| format!("cannot write the recording: {e}"))?;
        Ok(Sink::Mjpeg {
            recorder: rec,
            out_w,
            out_h,
        })
    }
}

/// Maps the wire codec/preset/scaler to the ffmpeg encoder library, preset string, whether `-bf`
/// applies, and the scale filter name.
fn ffmpeg_params(o: &RecordOptions) -> (String, String, bool, String) {
    use proto::{Codec, Preset, Scaler};
    let codec_lib = match o.codec {
        Codec::H264 => "libx264",
        Codec::H265 => "libx265",
        Codec::Av1 => "libsvtav1",
    };
    let x264_preset = match o.preset {
        Preset::Ultrafast => "ultrafast",
        Preset::Superfast => "superfast",
        Preset::Veryfast => "veryfast",
        Preset::Faster => "faster",
        Preset::Fast => "fast",
        Preset::Medium => "medium",
        Preset::Slow => "slow",
        Preset::Slower => "slower",
        Preset::Veryslow => "veryslow",
    };
    // libsvtav1 presets are numbers, 0 (slowest) .. 13 (fastest).
    let av1_preset = match o.preset {
        Preset::Ultrafast => "12",
        Preset::Superfast => "11",
        Preset::Veryfast => "10",
        Preset::Faster => "9",
        Preset::Fast => "8",
        Preset::Medium => "7",
        Preset::Slow => "5",
        Preset::Slower => "3",
        Preset::Veryslow => "1",
    };
    let (preset, use_bframes) = match o.codec {
        Codec::Av1 => (av1_preset.to_string(), false),
        _ => (x264_preset.to_string(), true),
    };
    let scaler = match o.scaler {
        Scaler::Bilinear => "bilinear",
        Scaler::Bicubic => "bicubic",
        Scaler::Lanczos => "lanczos",
        Scaler::Neighbor => "neighbor",
    };
    (
        codec_lib.to_string(),
        preset,
        use_bframes,
        scaler.to_string(),
    )
}

/// Where recordings are kept on this PC.
#[must_use]
pub fn directory(data_dir: &Path) -> PathBuf {
    data_dir.join("recordings")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_starts_inactive_and_empty() {
        let status = RecordingStatus::default();
        assert!(!status.active);
        assert_eq!(status.frames, 0);
        assert!(status.problem.is_empty());
    }

    #[test]
    fn recordings_live_in_their_own_folder() {
        let dir = directory(Path::new("C:\\state"));
        assert!(dir.ends_with("recordings"));
    }
}
