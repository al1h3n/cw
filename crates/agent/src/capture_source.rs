//! The real [`net::AgentDevice`]: screen capture and audio from [`media`], actions from [`platform`].
//!
//! Keeping the trait in `net` and the implementation here means `media` stays a standalone capture
//! crate with no knowledge of the network, and the Agent binary is the only place the two meet.

use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

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
    /// True only while a Console has explicitly taken control of the mouse and keyboard.
    /// Input is dropped unless this is set, so a stray message can never move a student's pointer.
    controlled: Mutex<bool>,
    /// True while the teacher has *frozen* this PC's local input without taking control (screen lock).
    /// The student's physical input is blocked whenever this OR `controlled` is set.
    screen_locked: Mutex<bool>,
    /// The screen recording in progress, if any. Dropping it closes the file.
    recording: Mutex<Option<crate::recording::Recording>>,
    /// The full-screen broadcast window, while a teacher is presenting. Dropping it closes it.
    broadcast: Mutex<Option<platform::present::Presenter>>,
    /// The exam lock, while the PC is locked down for an exam. Dropping it restores the desktop.
    /// Behind an `Arc` so a timed lock's auto-release thread can drop it after the requested duration
    /// even if the Console has since disconnected.
    exam: Arc<Mutex<Option<platform::examlock::ExamLock>>>,
    /// Bumped on every exam change. A timed auto-release only fires if this still matches the value it
    /// captured when it was armed, so a manual release (or a fresh lock) cancels an earlier timer.
    exam_gen: Arc<AtomicU64>,
    exam_state_path: std::path::PathBuf,
    /// Where recordings are written.
    recordings_dir: std::path::PathBuf,
    /// How many recordings to keep (0 = all); older ones are pruned by creation order. Persisted so it
    /// keeps applying offline.
    retention_keep: Mutex<u16>,
    /// True while a teacher is watching and the wallpaper is blacked out (D11).
    watched: Mutex<bool>,
    /// True while *watching* has locked wallpaper changes, so we only unlock what watching locked
    /// (and never clobber a teacher's explicit wallpaper lock).
    watch_locked_wp: Mutex<bool>,
    /// The live H.264 stream, if a teacher has opened the full-resolution view.
    stream: Mutex<Option<crate::streaming::Stream>>,
    /// File holding the student's real wallpaper path while black is shown, so it can be restored
    /// even after a crash.
    wallpaper_save: std::path::PathBuf,
    /// The shared workspace folder file transfer is confined to (AGENTS §5). Every send/download/list
    /// is resolved against this and refused if it escapes.
    workspace: std::path::PathBuf,
    /// Where a preloaded exam-media file is stored — **private**, not the shared workspace, so a student
    /// cannot copy the material before it plays.
    media_dir: std::path::PathBuf,
    /// The path of the currently preloaded media file, if any.
    preloaded_media: Mutex<Option<std::path::PathBuf>>,
    /// The running (or scheduled) media playback; dropping it stops playback and deletes the file.
    media_exam: Mutex<Option<crate::exam_media::MediaExam>>,
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        // Backstop: never leave a student staring at a black desktop because the Agent went away.
        let _ = platform::wallpaper::restore(&self.wallpaper_save);
    }
}

/// The file holding this PC's recording-retention setting (kept beside the recordings so it needs no
/// extra constructor argument, and survives a reboot for offline enforcement).
fn retention_path(recordings_dir: &Path) -> std::path::PathBuf {
    recordings_dir.join(".retention")
}

/// Loads the saved retention count (0 = keep all) if any.
fn load_retention(recordings_dir: &Path) -> u16 {
    std::fs::read_to_string(retention_path(recordings_dir))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Converts the agent's own recording status into the wire shape.
fn to_wire_recording(status: &crate::recording::RecordingStatus) -> proto::RecordingInfo {
    proto::RecordingInfo {
        active: status.active,
        file: status.file.clone(),
        frames: status.frames,
        width: status.width,
        height: status.height,
        fps: status.fps,
        problem: status.problem.clone(),
    }
}

impl ScreenCapture {
    /// Opens the screen capturer.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if the graphics device is unavailable (e.g. a headless session).
    pub fn new(
        audit_path: &Path,
        blocklist_path: &Path,
        recordings_dir: &Path,
        wallpaper_save: &Path,
        workspace: &Path,
        media_dir: &Path,
    ) -> Result<Self, CaptureError> {
        let capturer = media::ThumbnailCapturer::new().map_err(|e| CaptureError(e.to_string()))?;
        // In case a previous run was killed mid-watch, put any saved wallpaper back on start-up, then
        // re-apply any wallpaper the teacher pushed so it survives a reboot (bug: a PC rebooted to
        // finish installing lost its pushed wallpaper).
        let _ = platform::wallpaper::restore(wallpaper_save);
        platform::wallpaper::reapply_pushed(wallpaper_save);
        Ok(Self {
            capturer: Mutex::new(capturer),
            audio: Mutex::new(None),
            audit: AuditLog::new(audit_path),
            blocker: crate::blocker::Blocker::start(blocklist_path),
            controlled: Mutex::new(false),
            screen_locked: Mutex::new(false),
            recording: Mutex::new(None),
            broadcast: Mutex::new(None),
            exam: Arc::new(Mutex::new(None)),
            exam_gen: Arc::new(AtomicU64::new(0)),
            exam_state_path: audit_path.with_file_name("exam-state.json"),
            recordings_dir: recordings_dir.to_path_buf(),
            retention_keep: Mutex::new(load_retention(recordings_dir)),
            watched: Mutex::new(false),
            watch_locked_wp: Mutex::new(false),
            stream: Mutex::new(None),
            wallpaper_save: wallpaper_save.to_path_buf(),
            workspace: workspace.to_path_buf(),
            media_dir: media_dir.to_path_buf(),
            preloaded_media: Mutex::new(None),
            media_exam: Mutex::new(None),
        })
    }

    /// Stops watching (restores the wallpaper). Called when a Console session ends, so a dropped
    /// connection never leaves the desktop black.
    pub fn end_session(&self) {
        net::AgentDevice::set_watched(self, false);
    }

    /// A sign-out kills the session helper. Recreate the teacher's active exam when the new
    /// helper starts in the next interactive session.
    pub fn restore_exam(&self) {
        match crate::exam_state::load(&self.exam_state_path) {
            Ok(Some((message, seconds))) => {
                if let Err(err) = self.start_exam(&message, seconds) {
                    eprintln!("could not restore exam lock: {err}");
                }
            }
            Ok(None) => {}
            Err(err) => eprintln!("could not read saved exam intent: {err}"),
        }
    }

    fn start_exam(&self, message: &str, duration_seconds: u32) -> Result<(), String> {
        crate::exam_state::save(&self.exam_state_path, message, duration_seconds)?;
        let generation = self.exam_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let mut slot = self.exam.lock().unwrap_or_else(|e| e.into_inner());
        *slot = None;
        let lock = platform::examlock::ExamLock::start(message).map_err(|e| e.to_string())?;
        *slot = Some(lock);
        if duration_seconds > 0 {
            let exam = Arc::clone(&self.exam);
            let exam_gen = Arc::clone(&self.exam_gen);
            let state_path = self.exam_state_path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(u64::from(duration_seconds)));
                if exam_gen.load(Ordering::SeqCst) == generation {
                    let mut slot = exam.lock().unwrap_or_else(|e| e.into_inner());
                    if slot.take().is_some() {
                        crate::exam_state::clear(&state_path);
                        println!("exam lock auto-released after {duration_seconds}s");
                    }
                }
            });
        }
        Ok(())
    }

    /// Blocks the student's physical input whenever a Console holds control OR a screen lock is up, and
    /// restores it only when neither is set. Called on every change to either state so control and the
    /// screen lock compose instead of one clobbering the other.
    fn apply_input_block(&self) {
        let controlled = *self.controlled.lock().unwrap_or_else(|e| e.into_inner());
        let locked = *self.screen_locked.lock().unwrap_or_else(|e| e.into_inner());
        let block = controlled || locked;
        if let Err(err) = platform::input::set_local_input_blocked(block) {
            eprintln!(
                "could not {} local input: {err}",
                if block { "block" } else { "restore" }
            );
        }
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

/// Translates one wire input event into the matching `platform::input` call.
fn apply_one(event: proto::InputEvent) -> Result<(), platform::input::InputError> {
    use platform::input::{self, ButtonState, MouseButton};
    use proto::{InputEvent, PointerButton};

    /// The wire sends positions as `0..=65535`; `platform::input` takes a `0.0..=1.0` fraction.
    const FULL: f32 = 65_535.0;

    let state = |down: bool| {
        if down {
            ButtonState::Down
        } else {
            ButtonState::Up
        }
    };
    match event {
        InputEvent::MoveTo { x, y } => {
            input::move_pointer(f32::from(x) / FULL, f32::from(y) / FULL)
        }
        InputEvent::Button { button, down } => input::mouse_button(
            match button {
                PointerButton::Left => MouseButton::Left,
                PointerButton::Right => MouseButton::Right,
                PointerButton::Middle => MouseButton::Middle,
            },
            state(down),
        ),
        InputEvent::Scroll { delta } => input::scroll(delta),
        InputEvent::Key { virtual_key, down } => input::key(virtual_key, state(down)),
        InputEvent::Text(ch) => {
            input::unicode_char(ch, ButtonState::Down)?;
            input::unicode_char(ch, ButtonState::Up)
        }
        InputEvent::ReleaseAll => {
            input::release_all_modifiers();
            Ok(())
        }
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
        // The reset itself needs the wallpaper save-path, which lives on `self`, so it is carried out
        // in `perform`; report success here and let `perform` downgrade it if the reset fails.
        Action::ResetWallpaper => return ActionOutcome::Started { delay_seconds: 0 },
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
        let mut outcome = carry_out(action);
        // Turning wallpaper lock off also puts the student's own wallpaper back and forgets any pushed
        // image (bug: "wallpapers should go back to default when wallpaper lock is turned off").
        if matches!(action, Action::UnlockWallpaper) {
            let _ = platform::wallpaper::revert(&self.wallpaper_save);
        }
        // Reset-to-default actively puts the wallpaper back (student's own if captured, else the Windows
        // default) even when nothing was locked — what "set the wallpaper back to default" needs.
        if matches!(action, Action::ResetWallpaper)
            && let Err(err) = platform::wallpaper::reset_to_default(&self.wallpaper_save)
        {
            eprintln!("reset wallpaper failed: {err}");
            outcome = ActionOutcome::Failed(ActionFailure::Failed);
        }
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

    fn start_stream(
        &self,
        from: &PeerInfo,
        monitor: u8,
        settings: proto::VideoSettings,
    ) -> Result<(proto::VideoSettings, tokio::sync::mpsc::Receiver<Vec<u8>>), String> {
        let mut slot = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        *slot = None; // dropping the old stream stops its encoder before a new one starts
        let (stream, actual, packets) = crate::streaming::Stream::start(monitor, settings)?;
        // Hand the sole Desktop Duplication of this output to the stream: drop the thumbnail
        // capturer's duplication so the grid's thumbnails switch to GDI instead of fighting it.
        if let Ok(mut capturer) = self.capturer.lock() {
            capturer.release();
        }
        println!(
            "console {} started a {}x{} @ {} fps stream ({} kbit/s)",
            from.device_id, actual.width, actual.height, actual.fps, actual.kbps
        );
        let _ = self
            .audit
            .note(net::endpoint::now_ms(), from.device_id, "stream-start");
        *slot = Some(stream);
        drop(slot);
        // Black the wallpaper only for a *full* live preview (D11: "while streaming"), not for the
        // low-cost grid thumbnails. Restored in stop_stream / when the session drops.
        net::AgentDevice::set_watched(self, true);
        Ok((actual, packets))
    }

    fn stop_stream(&self) {
        let mut slot = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(stream) = slot.take() {
            let (frames, dropped) = stream.counts();
            println!("stream stopped after {frames} frame(s), {dropped} dropped");
        }
        drop(slot);
        // Live preview over: put the student's own wallpaper back.
        net::AgentDevice::set_watched(self, false);
    }

    fn set_watched(&self, watched: bool) -> bool {
        let mut current = self.watched.lock().unwrap_or_else(|e| e.into_inner());
        if *current == watched {
            return watched;
        }
        let result = if watched {
            platform::wallpaper::set_black(&self.wallpaper_save)
        } else {
            platform::wallpaper::restore(&self.wallpaper_save)
        };
        // Also stop the student changing their wallpaper while being watched (bug: they could open
        // Personalisation and replace the black-out). Only unlock what watching itself locked, so an
        // explicit teacher wallpaper lock is never undone here. Best-effort: a failure is not fatal.
        let mut watch_locked = self
            .watch_locked_wp
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if watched {
            if platform::wallpaper::lock().is_ok() {
                *watch_locked = true;
            }
        } else if *watch_locked {
            let _ = platform::wallpaper::unlock();
            *watch_locked = false;
        }
        drop(watch_locked);
        match result {
            Ok(()) => {
                *current = watched;
                watched
            }
            Err(err) => {
                eprintln!("wallpaper black-out failed: {err}");
                false
            }
        }
    }

    fn list_macs(&self) -> Vec<String> {
        platform::wol::local_macs()
            .into_iter()
            .map(|m| m.to_hex())
            .collect()
    }

    fn wake_on_lan(&self, from: &PeerInfo, mac: &str) -> bool {
        let Ok(mac) = platform::wol::MacAddress::parse(mac) else {
            return false;
        };
        match platform::wol::wake(mac) {
            Ok(()) => {
                println!("console {} -> wake {}", from.device_id, mac.to_hex());
                let _ = self.audit.note(
                    net::endpoint::now_ms(),
                    from.device_id,
                    &format!("wake:{}", mac.to_hex()),
                );
                true
            }
            Err(err) => {
                eprintln!("wake failed: {err}");
                false
            }
        }
    }

    fn show_broadcast(&self, from: &PeerInfo, jpeg: &[u8], locked: bool) -> (bool, String) {
        let (pixels, width, height) = match media::jpeg::decode_to_bgra(jpeg) {
            Ok(frame) => frame,
            Err(err) => return (false, format!("unreadable broadcast frame: {err}")),
        };
        let mut slot = self.broadcast.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            // A locked broadcast traps the student on a separate desktop (no Alt+Tab / Win key) for
            // the duration; an unlocked one is just an always-on-top full-screen window.
            let opened = if locked {
                platform::present::Presenter::open_locked()
            } else {
                platform::present::Presenter::open()
            };
            match opened {
                Ok(presenter) => {
                    let how = if locked {
                        "broadcast-start-locked"
                    } else {
                        "broadcast-start"
                    };
                    println!(
                        "console {} started broadcasting (locked={locked})",
                        from.device_id
                    );
                    let _ = self
                        .audit
                        .note(net::endpoint::now_ms(), from.device_id, how);
                    *slot = Some(presenter);
                }
                Err(err) => return (false, err.to_string()),
            }
        }
        if let Some(presenter) = slot.as_ref() {
            presenter.show(pixels, width, height);
        }
        (true, String::new())
    }

    fn stop_broadcast(&self, from: &PeerInfo) -> (bool, String) {
        let mut slot = self.broadcast.lock().unwrap_or_else(|e| e.into_inner());
        if slot.take().is_some() {
            // Dropping the presenter destroys the window and gives the desktop back.
            println!("console {} stopped broadcasting", from.device_id);
            let _ = self
                .audit
                .note(net::endpoint::now_ms(), from.device_id, "broadcast-stop");
        }
        (false, String::new())
    }

    fn set_exam(
        &self,
        from: &PeerInfo,
        on: bool,
        message: &str,
        duration_seconds: u32,
    ) -> (bool, String) {
        if on {
            match self.start_exam(message, duration_seconds) {
                Ok(()) => {
                    println!(
                        "console {} started exam lock ({})",
                        from.device_id,
                        if duration_seconds == 0 {
                            "until released".to_string()
                        } else {
                            format!("{duration_seconds}s")
                        }
                    );
                    let _ = self
                        .audit
                        .note(net::endpoint::now_ms(), from.device_id, "exam-start");
                    (true, String::new())
                }
                Err(err) => {
                    crate::exam_state::clear(&self.exam_state_path);
                    (false, err)
                }
            }
        } else {
            self.exam_gen.fetch_add(1, Ordering::SeqCst);
            let mut slot = self.exam.lock().unwrap_or_else(|e| e.into_inner());
            // Dropping the lock switches the desktop back and closes the lock desktop.
            if slot.take().is_some() {
                println!("console {} ended exam lock", from.device_id);
                let _ = self
                    .audit
                    .note(net::endpoint::now_ms(), from.device_id, "exam-stop");
            }
            crate::exam_state::clear(&self.exam_state_path);
            (false, String::new())
        }
    }

    fn list_files(&self, _from: &PeerInfo, dir: &str) -> Result<Vec<proto::FileEntry>, String> {
        crate::workspace::list(&self.workspace, dir)
    }

    fn file_read_path(&self, from: &PeerInfo, path: &str) -> Option<std::path::PathBuf> {
        let resolved = crate::workspace::read_path(&self.workspace, path)?;
        let _ = self.audit.note(
            net::endpoint::now_ms(),
            from.device_id,
            &format!("file-send:{path}"),
        );
        Some(resolved)
    }

    fn file_write_path(
        &self,
        from: &PeerInfo,
        dir: &str,
        name: &str,
    ) -> Option<std::path::PathBuf> {
        let dest = crate::workspace::write_path(&self.workspace, dir, name)?;
        let _ = self.audit.note(
            net::endpoint::now_ms(),
            from.device_id,
            &format!("file-recv:{name}"),
        );
        Some(dest)
    }

    fn file_received(&self, name: &str) {
        platform::notification::file_received(name);
    }

    fn workspace_manifest(&self, _from: &PeerInfo) -> Result<Vec<proto::FileEntry>, String> {
        crate::workspace::manifest(&self.workspace)
    }

    fn delete_file(&self, from: &PeerInfo, path: &str) -> String {
        match crate::workspace::delete(&self.workspace, path) {
            Ok(()) => {
                let _ = self.audit.note(
                    net::endpoint::now_ms(),
                    from.device_id,
                    &format!("file-delete:{path}"),
                );
                String::new()
            }
            Err(problem) => problem,
        }
    }

    fn clear_workspace(&self, from: &PeerInfo) -> Result<u32, String> {
        let removed = crate::workspace::clear(&self.workspace)?;
        let _ = self.audit.note(
            net::endpoint::now_ms(),
            from.device_id,
            &format!("workspace-clear:{removed}"),
        );
        Ok(removed)
    }

    fn preload_media_path(&self, from: &PeerInfo, name: &str) -> Option<std::path::PathBuf> {
        if !crate::workspace::is_plain_name(name) {
            return None;
        }
        std::fs::create_dir_all(&self.media_dir).ok()?;
        let dest = self.media_dir.join(name);
        *self
            .preloaded_media
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(dest.clone());
        let _ = self.audit.note(
            net::endpoint::now_ms(),
            from.device_id,
            &format!("media-preload:{name}"),
        );
        Some(dest)
    }

    fn play_media(
        &self,
        from: &PeerInfo,
        start_in_ms: u32,
        message: &str,
        lock: bool,
    ) -> (bool, String) {
        let path = self
            .preloaded_media
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(path) = path.filter(|p| p.is_file()) else {
            return (false, "no media preloaded".to_string());
        };
        let message = (!message.is_empty()).then(|| message.to_string());
        let exam = crate::exam_media::MediaExam::start(
            path,
            Duration::from_millis(u64::from(start_in_ms)),
            message,
            lock,
        );
        // Replacing any current playback stops it first (Drop of the old handle).
        *self.media_exam.lock().unwrap_or_else(|e| e.into_inner()) = Some(exam);
        let _ = self
            .audit
            .note(net::endpoint::now_ms(), from.device_id, "media-play");
        (true, String::new())
    }

    fn stop_media(&self, from: &PeerInfo) -> (bool, String) {
        // Dropping the handle stops playback, releases the lock and deletes the media file.
        if self
            .media_exam
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .is_some()
        {
            let _ = self
                .audit
                .note(net::endpoint::now_ms(), from.device_id, "media-stop");
        }
        (false, String::new())
    }

    fn set_wallpaper(
        &self,
        from: &PeerInfo,
        image: &[u8],
        fit: proto::WallpaperFit,
    ) -> (bool, String) {
        let fit = match fit {
            proto::WallpaperFit::Fill => platform::wallpaper::Fit::Fill,
            proto::WallpaperFit::Fit => platform::wallpaper::Fit::Fit,
            proto::WallpaperFit::Stretch => platform::wallpaper::Fit::Stretch,
            proto::WallpaperFit::Center => platform::wallpaper::Fit::Center,
            proto::WallpaperFit::Tile => platform::wallpaper::Fit::Tile,
        };
        match platform::wallpaper::set_image(image, &self.wallpaper_save, fit) {
            Ok(()) => {
                println!(
                    "console {} set wallpaper ({} bytes)",
                    from.device_id,
                    image.len()
                );
                let _ = self
                    .audit
                    .note(net::endpoint::now_ms(), from.device_id, "wallpaper-set");
                (true, String::new())
            }
            Err(err) => (false, err.to_string()),
        }
    }

    fn set_screen_lock(&self, from: &PeerInfo, on: bool) -> (bool, String) {
        {
            let mut locked = self.screen_locked.lock().unwrap_or_else(|e| e.into_inner());
            *locked = on;
        }
        self.apply_input_block();
        let action = if on { "screen-lock" } else { "screen-unlock" };
        println!("console {} → {action}", from.device_id);
        let _ = self
            .audit
            .note(net::endpoint::now_ms(), from.device_id, action);
        (on, String::new())
    }

    fn start_recording(
        &self,
        from: &PeerInfo,
        monitor: u8,
        options: proto::RecordOptions,
    ) -> proto::RecordingInfo {
        let mut slot = self.recording.lock().unwrap_or_else(|e| e.into_inner());
        // Dropping the old recording closes its file tidily before a new one starts.
        *slot = None;
        let recording = crate::recording::Recording::start(&self.recordings_dir, monitor, options);
        let info = to_wire_recording(&recording.status());
        println!("console {} started recording", from.device_id);
        let _ = self
            .audit
            .note(net::endpoint::now_ms(), from.device_id, "record-start");
        *slot = Some(recording);
        info
    }

    fn stop_recording(&self, from: &PeerInfo) -> proto::RecordingInfo {
        let mut slot = self.recording.lock().unwrap_or_else(|e| e.into_inner());
        let Some(recording) = slot.take() else {
            return to_wire_recording(&crate::recording::RecordingStatus::default());
        };
        // finish() joins the worker, so the status it returns already carries the frame rate the
        // PC actually achieved and the final frame count.
        let status = recording.finish();
        println!("console {} stopped recording", from.device_id);
        let _ = self
            .audit
            .note(net::endpoint::now_ms(), from.device_id, "record-stop");
        // Enforce retention now that a new recording exists (clock-independent, by creation order).
        let keep = *self
            .retention_keep
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let _ = crate::recording::prune(&self.recordings_dir, keep);
        let mut info = to_wire_recording(&status);
        info.active = false;
        info
    }

    fn recording_status(&self) -> proto::RecordingInfo {
        let slot = self.recording.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            Some(recording) => to_wire_recording(&recording.status()),
            None => to_wire_recording(&crate::recording::RecordingStatus::default()),
        }
    }

    fn list_recordings(&self) -> Vec<proto::StoredRecording> {
        let Ok(entries) = std::fs::read_dir(&self.recordings_dir) else {
            return Vec::new();
        };
        let mut out: Vec<proto::StoredRecording> = entries
            .flatten()
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("avi"))
            })
            .map(|e| proto::StoredRecording {
                file: e.file_name().to_string_lossy().to_string(),
                bytes: e.metadata().map(|m| m.len()).unwrap_or(0),
            })
            .collect();
        // File names start with a UUIDv7, so sorting by name sorts by when it was recorded.
        out.sort_by(|a, b| a.file.cmp(&b.file));
        out
    }

    fn recording_path(&self, file: &str) -> Option<std::path::PathBuf> {
        // Trust boundary: a bare file name only, that really exists in our recordings folder — never
        // a path with separators or `..`, so a Console can never pull an arbitrary file off the PC.
        if file.is_empty() || file.contains('/') || file.contains('\\') || file.contains("..") {
            return None;
        }
        let path = self.recordings_dir.join(file);
        path.is_file().then_some(path)
    }

    fn list_apps(&self) -> Vec<proto::AppEntry> {
        platform::apps::list_apps()
            .into_iter()
            .map(|a| proto::AppEntry {
                id: a.id,
                name: a.name,
            })
            .collect()
    }

    fn app_icon(&self, id: u32) -> Option<(u16, u16, Vec<u8>)> {
        platform::apps::icon_bgra(id)
    }

    fn running_icon(&self, pid: u32) -> Option<(u16, u16, Vec<u8>)> {
        platform::apps::icon_for_pid(pid)
    }

    fn launch_app(&self, from: &PeerInfo, id: u32) -> (String, bool) {
        match platform::apps::launch(id) {
            Ok(name) => {
                println!("console {} launch {name}", from.device_id);
                let _ = self.audit.note(
                    net::endpoint::now_ms(),
                    from.device_id,
                    &format!("launch:{name}"),
                );
                (name, true)
            }
            Err(err) => {
                eprintln!("launch {id:#x} failed: {err}");
                (String::new(), false)
            }
        }
    }

    fn list_running(&self) -> Vec<proto::RunningApp> {
        platform::process::list_processes()
            .unwrap_or_default()
            .into_iter()
            // Never offer a system-critical process: a teacher must not be one click from a
            // bluescreen, and `terminate` would refuse it anyway.
            .filter(|p| !platform::process::is_protected(&p.name))
            .map(|p| proto::RunningApp {
                pid: p.pid,
                name: p.name,
            })
            .collect()
    }

    fn close_app(&self, from: &PeerInfo, pid: u32) -> bool {
        // Re-check against the live list: the pid must still belong to a closable program, so a
        // stale or invented pid cannot reach a protected process.
        let closable = platform::process::list_processes()
            .unwrap_or_default()
            .into_iter()
            .find(|p| p.pid == pid && !platform::process::is_protected(&p.name));
        let Some(process) = closable else {
            return false;
        };
        let closed = platform::process::terminate(pid).is_ok();
        println!(
            "console {} close {} ({closed})",
            from.device_id, process.name
        );
        let _ = self.audit.note(
            net::endpoint::now_ms(),
            from.device_id,
            &format!("close:{}", process.name),
        );
        closed
    }

    fn set_control(&self, from: &PeerInfo, enabled: bool) -> bool {
        {
            let mut controlled = self.controlled.lock().unwrap_or_else(|e| e.into_inner());
            if *controlled == enabled {
                return enabled;
            }
            *controlled = enabled;
        }
        // Take the student's own mouse and keyboard out of the way while the teacher drives, and give
        // them back the moment control is released — unless a screen lock is also up, in which case
        // `apply_input_block` keeps them blocked. Injected remote input still passes through, so this
        // only stops the *student* from fighting the pointer.
        self.apply_input_block();
        if !enabled {
            // Never leave a student with a modifier stuck down because the key-up never arrived.
            platform::input::release_all_modifiers();
        }
        // Being driven is exactly the kind of thing that must be on the record (D3).
        let action = if enabled {
            "control-start"
        } else {
            "control-stop"
        };
        println!("console {} → {action}", from.device_id);
        if let Err(err) = self
            .audit
            .note(net::endpoint::now_ms(), from.device_id, action)
        {
            eprintln!("audit log write failed: {err}");
        }
        enabled
    }

    fn apply_input(&self, events: &[proto::InputEvent]) -> (u16, bool) {
        if !*self.controlled.lock().unwrap_or_else(|e| e.into_inner()) {
            return (0, true); // refused: nobody has been granted control
        }
        let mut applied = 0u16;
        for event in events {
            if apply_one(*event).is_ok() {
                applied = applied.saturating_add(1);
            }
        }
        (applied, false)
    }

    fn set_blocklist(&self, programs: Vec<String>) -> u16 {
        self.blocker.set_rules(programs)
    }

    fn set_retention(&self, from: &PeerInfo, keep_last: u16) -> u16 {
        *self
            .retention_keep
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = keep_last;
        // Persist so it keeps applying after a reboot with no network (D9).
        let _ = std::fs::write(retention_path(&self.recordings_dir), keep_last.to_string());
        let removed = crate::recording::prune(&self.recordings_dir, keep_last);
        let _ = self.audit.note(
            net::endpoint::now_ms(),
            from.device_id,
            &format!("retention:{keep_last} pruned:{removed}"),
        );
        keep_last
    }

    fn set_break_glass(&self, from: &PeerInfo, hash: String) -> bool {
        // The break-glass files live in the data directory, which is the parent of the wallpaper-save
        // file (all agent state is rooted there).
        let Some(data_dir) = self.wallpaper_save.parent() else {
            return false;
        };
        match crate::breakglass::store_hash(data_dir, &hash) {
            Ok(()) => {
                let _ = self.audit.note(
                    net::endpoint::now_ms(),
                    from.device_id,
                    "breakglass-provisioned",
                );
                true
            }
            Err(err) => {
                eprintln!("could not store break-glass hash: {err}");
                false
            }
        }
    }

    fn set_url_blocklist(&self, from: &PeerInfo, patterns: Vec<String>) -> (u16, String) {
        let mut patterns = patterns;
        patterns.truncate(proto::MAX_URL_BLOCKLIST);
        match platform::weblock::set_url_blocklist(&patterns) {
            Ok(count) => {
                let _ = self.audit.note(
                    net::endpoint::now_ms(),
                    from.device_id,
                    &format!("url-block:{count}"),
                );
                (count, String::new())
            }
            Err(err) => (0, err.to_string()),
        }
    }

    fn take_blocked(&self) -> Vec<String> {
        self.blocker.take_closed()
    }

    fn capture_thumbnail(
        &self,
        monitor: u8,
        max_width: u16,
        quality: u8,
    ) -> Result<Vec<u8>, CaptureError> {
        // While a full-resolution stream is running it owns the one Desktop Duplication this output
        // allows, so the grid's thumbnails take the GDI path meanwhile — otherwise the two duplications
        // fight and both fail every frame (E_INVALIDARG). Read the flag and release the lock before
        // taking the capturer lock, keeping a single, consistent lock order (stream then capturer).
        let streaming = self
            .stream
            .lock()
            .map(|slot| slot.is_some())
            .unwrap_or(false);
        let mut capturer = self
            .capturer
            .lock()
            .map_err(|_| CaptureError("capture lock poisoned".into()))?;
        let result = if streaming {
            capturer.capture_jpeg_gdi(monitor, max_width, quality)
        } else {
            capturer.capture_jpeg(monitor, max_width, quality)
        };
        result.map_err(|e| CaptureError(e.to_string()))
    }
}
