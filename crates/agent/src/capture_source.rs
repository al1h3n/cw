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
    /// True only while a Console has explicitly taken control of the mouse and keyboard.
    /// Input is dropped unless this is set, so a stray message can never move a student's pointer.
    controlled: Mutex<bool>,
    /// The screen recording in progress, if any. Dropping it closes the file.
    recording: Mutex<Option<crate::recording::Recording>>,
    /// The full-screen broadcast window, while a teacher is presenting. Dropping it closes it.
    broadcast: Mutex<Option<platform::present::Presenter>>,
    /// Where recordings are written.
    recordings_dir: std::path::PathBuf,
    /// True while a teacher is watching and the wallpaper is blacked out (D11).
    watched: Mutex<bool>,
    /// The live H.264 stream, if a teacher has opened the full-resolution view.
    stream: Mutex<Option<crate::streaming::Stream>>,
    /// File holding the student's real wallpaper path while black is shown, so it can be restored
    /// even after a crash.
    wallpaper_save: std::path::PathBuf,
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        // Backstop: never leave a student staring at a black desktop because the Agent went away.
        let _ = platform::wallpaper::restore(&self.wallpaper_save);
    }
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
    ) -> Result<Self, CaptureError> {
        let capturer = media::ThumbnailCapturer::new().map_err(|e| CaptureError(e.to_string()))?;
        // In case a previous run was killed mid-watch, put any saved wallpaper back on start-up.
        let _ = platform::wallpaper::restore(wallpaper_save);
        Ok(Self {
            capturer: Mutex::new(capturer),
            audio: Mutex::new(None),
            audit: AuditLog::new(audit_path),
            blocker: crate::blocker::Blocker::start(blocklist_path),
            controlled: Mutex::new(false),
            recording: Mutex::new(None),
            broadcast: Mutex::new(None),
            recordings_dir: recordings_dir.to_path_buf(),
            watched: Mutex::new(false),
            stream: Mutex::new(None),
            wallpaper_save: wallpaper_save.to_path_buf(),
        })
    }

    /// Stops watching (restores the wallpaper). Called when a Console session ends, so a dropped
    /// connection never leaves the desktop black.
    pub fn end_session(&self) {
        net::AgentDevice::set_watched(self, false);
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

    fn show_broadcast(&self, from: &PeerInfo, jpeg: &[u8]) -> (bool, String) {
        let (pixels, width, height) = match media::jpeg::decode_to_bgra(jpeg) {
            Ok(frame) => frame,
            Err(err) => return (false, format!("unreadable broadcast frame: {err}")),
        };
        let mut slot = self.broadcast.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none() {
            match platform::present::Presenter::open() {
                Ok(presenter) => {
                    println!("console {} started broadcasting", from.device_id);
                    let _ =
                        self.audit
                            .note(net::endpoint::now_ms(), from.device_id, "broadcast-start");
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
        if file.is_empty()
            || file.contains('/')
            || file.contains('\\')
            || file.contains("..")
        {
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
        let mut controlled = self.controlled.lock().unwrap_or_else(|e| e.into_inner());
        if *controlled == enabled {
            return enabled;
        }
        *controlled = enabled;
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

    fn take_blocked(&self) -> Vec<String> {
        self.blocker.take_closed()
    }

    fn capture_thumbnail(&self, monitor: u8, max_width: u16) -> Result<Vec<u8>, CaptureError> {
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
            capturer.capture_jpeg_gdi(monitor, max_width)
        } else {
            capturer.capture_jpeg(monitor, max_width)
        };
        result.map_err(|e| CaptureError(e.to_string()))
    }
}
