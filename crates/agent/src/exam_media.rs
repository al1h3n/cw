//! A synchronised, play-once media exam (feature 14).
//!
//! The teacher preloads a media file (stored privately, not in the shared workspace, so a student
//! cannot copy it first), then all PCs start playing it together. "Together" uses a **relative** delay
//! from when each Agent received the play command — never the student's wall clock, which they can
//! change (the Feature-5 "time must not matter" rule). Playback has **no controls** (there is no player
//! window); when `lock` is set the exam-lock overlay both shows the notice and stops the student doing
//! anything else. The file is **deleted after it plays once**.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// A running (or about-to-start) media playback. Dropping it stops playback, releases any lock and
/// removes the media file.
pub struct MediaExam {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl MediaExam {
    /// Schedules the media at `path` to play once, `start_in` after now, optionally behind a lock
    /// overlay showing `message`.
    #[must_use]
    pub fn start(path: PathBuf, start_in: Duration, message: Option<String>, lock: bool) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let join = std::thread::spawn({
            let stop = Arc::clone(&stop);
            move || run(&path, start_in, message.as_deref(), lock, &stop)
        });
        Self {
            stop,
            join: Some(join),
        }
    }
}

impl Drop for MediaExam {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn run(path: &Path, start_in: Duration, message: Option<&str>, lock: bool, stop: &AtomicBool) {
    // Wait for the synchronised start, staying responsive to a stop request.
    if !sleep_interruptible(start_in, stop) {
        cleanup(path);
        return;
    }
    // The lock overlay is both the "notification" and the "no controls" — the student cannot do
    // anything else while the audio plays. Dropping it at the end restores the desktop.
    let _lock = if lock {
        platform::examlock::ExamLock::start(message.unwrap_or("")).ok()
    } else {
        None
    };
    match platform::audio::Playback::start(path) {
        Ok(playback) => {
            // Give MCI a moment to actually start before polling its mode.
            std::thread::sleep(Duration::from_millis(300));
            while !stop.load(Ordering::SeqCst) && !playback.finished() {
                std::thread::sleep(Duration::from_millis(200));
            }
        }
        Err(err) => eprintln!("media playback failed: {err}"),
    }
    // Play once: remove the media after it finishes (or is stopped).
    cleanup(path);
}

/// Sleeps `dur` in small slices; returns false as soon as `stop` is set.
fn sleep_interruptible(dur: Duration, stop: &AtomicBool) -> bool {
    let deadline = Instant::now() + dur;
    while Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    !stop.load(Ordering::SeqCst)
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}
