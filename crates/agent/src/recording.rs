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
    recorder::{Recorder, RecordingSettings},
    resize,
};

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
    pub fn start(dir: &Path, monitor: u8, settings: RecordingSettings) -> Self {
        let settings = settings.clamped();
        let id = crate::record_id::now();
        let file = format!("recording-{}.avi", id.to_compact());
        let path = dir.join(&file);

        let status = Arc::new(Mutex::new(RecordingStatus {
            active: true,
            file,
            fps: settings.fps,
            ..RecordingStatus::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));

        let worker = {
            let status = Arc::clone(&status);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || run(&path, monitor, settings, &status, &stop))
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

/// The recording loop: capture, resize, encode, append, wait for the next frame time.
fn run(
    path: &Path,
    monitor: u8,
    settings: RecordingSettings,
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
    let (width, height) = resize::fit_within(src_w, src_h, settings.max_width, settings.max_height);

    let mut recorder = match Recorder::create(path, width, height, settings.fps) {
        Ok(recorder) => recorder,
        Err(err) => return fail(status, format!("cannot write the recording: {err}")),
    };
    {
        let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
        status.width = width;
        status.height = height;
    }

    let interval = settings.frame_interval();
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
                let scaled = resize::area_average(&pixels, w, h, width, height);
                if !scaled.is_empty()
                    && let Ok(jpeg) = media::encode_bgra(&scaled, width, height)
                    && let Err(err) = recorder.push_jpeg(&jpeg)
                {
                    return fail(status, format!("writing the recording failed: {err}"));
                }
                let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
                status.frames = recorder.frame_count();
                status.seconds = recorder.seconds();
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

    // Write the rate we actually achieved, so the file plays back in real time even when the PC
    // could not keep up with the requested rate.
    recorder.set_measured_fps(started_at.elapsed().as_secs_f32());
    let real_fps = recorder.fps();
    let frames = recorder.frame_count();
    let finished = recorder.finish();

    let mut status = status.lock().unwrap_or_else(|e| e.into_inner());
    status.active = false;
    status.fps = real_fps;
    status.frames = frames;
    status.seconds = frames as f32 / real_fps.max(1) as f32;
    if real_fps < settings.fps {
        status.problem = format!(
            "this PC managed {real_fps} fps, not {}; the file is written at {real_fps} fps so it \
             plays at the right speed",
            settings.fps
        );
    }
    if let Err(err) = finished {
        status.problem = format!("closing the recording failed: {err}");
    }
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
