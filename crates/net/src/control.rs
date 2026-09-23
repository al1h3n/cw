//! The ongoing control session between a Console and an Agent (Phase 1.5).
//!
//! A control session is opened *after* pairing: both endpoints are authenticated by public key at the
//! transport, and this layer additionally refuses any peer whose key is not in the local
//! [`TrustStore`] — so only a paired Console can drive an Agent, and an Agent only serves a paired
//! Console. After the [`proto::Hello`] handshake the Console can request thumbnails on demand; the
//! Agent captures **only** in response to a request (an idle Console means zero capture, per D11).

use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, RecvStream, SendStream},
};
use proto::{
    Capabilities, Control, DeviceId, Hello, Monitor, PROTOCOL_VERSION, ProtocolError, Role,
};

use crate::{
    TrustStore,
    endpoint::{CONTROL_ALPN, EndpointError, read_message, write_message},
};

/// What the local device announces about itself in the handshake.
#[derive(Debug, Clone, Copy)]
pub struct LocalHello {
    /// This device's role.
    pub role: Role,
    /// This device's short handle.
    pub device_id: DeviceId,
    /// What this device can do.
    pub capabilities: Capabilities,
}

/// What we learned about the peer during the handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerInfo {
    /// The peer's transport public key (its real, pinned identity).
    pub public_key: [u8; 32],
    /// The handle the peer announced.
    pub device_id: DeviceId,
    /// The peer's role.
    pub role: Role,
    /// The peer's capabilities.
    pub capabilities: Capabilities,
}

/// Everything a Console can ask of a student PC: screens, sound, and the fixed list of actions.
/// The Agent supplies the real one; tests supply a fake.
///
/// Capture happens only when [`capture_thumbnail`](AgentDevice::capture_thumbnail) is called, which
/// is only when a Console asks — there is no background capture loop to leave running.
pub trait AgentDevice {
    /// Carries out one [`proto::Action`] and reports what happened.
    ///
    /// The default refuses everything, so a device that only shares its screen is still valid.
    /// `from` is the Console asking, so the device can write it into its audit log (D3).
    fn perform(&self, from: &PeerInfo, action: proto::Action) -> proto::ActionOutcome {
        let _ = (from, action);
        proto::ActionOutcome::Failed(proto::ActionFailure::NotSupported)
    }

    /// Returns a JPEG of `monitor`, scaled to at most `max_width` pixels wide and encoded at the given
    /// JPEG `quality` (`1..=100`).
    ///
    /// # Errors
    /// Returns [`CaptureError`] if the monitor is unavailable or capture fails.
    fn capture_thumbnail(
        &self,
        monitor: u8,
        max_width: u16,
        quality: u8,
    ) -> Result<Vec<u8>, CaptureError>;

    /// The monitors this device has, so the Console can offer them.
    fn monitors(&self) -> Vec<Monitor>;

    /// Starts or stops recording what this PC is playing, reporting the resulting format.
    ///
    /// The default refuses: a source that cannot do audio simply reports "not available", and the
    /// Console shows listening as unsupported rather than failing the session.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if audio cannot be started on this device.
    fn set_audio(&self, enabled: bool) -> Result<Option<proto::AudioFormat>, CaptureError> {
        let _ = enabled;
        Err(CaptureError("this device cannot share audio".into()))
    }

    /// Returns sound recorded since the previous call, at most `max_samples`.
    fn take_audio(&self, max_samples: usize) -> Vec<i16> {
        let _ = max_samples;
        Vec::new()
    }

    /// Replaces the set of programs this device blocks, returning how many rules it kept after
    /// capping to [`proto::MAX_BLOCKLIST`] and dropping blanks. The default keeps none.
    fn set_blocklist(&self, programs: Vec<String>) -> u16 {
        let _ = programs;
        0
    }

    /// Names of programs blocked since the previous call (newest last), for reporting to a teacher.
    fn take_blocked(&self) -> Vec<String> {
        Vec::new()
    }

    /// Starts encoding this screen as H.264, returning the settings actually used and a channel of
    /// encoded packets. The Agent opens a uni-stream and pumps the channel down it.
    ///
    /// The default refuses, so a device that cannot encode reports that instead of going silent.
    ///
    /// # Errors
    /// A message explaining why no stream can start.
    fn start_stream(
        &self,
        from: &PeerInfo,
        monitor: u8,
        settings: proto::VideoSettings,
    ) -> Result<(proto::VideoSettings, tokio::sync::mpsc::Receiver<Vec<u8>>), String> {
        let _ = (from, monitor, settings);
        Err("this device cannot stream video".to_string())
    }

    /// Stops any running video stream.
    fn stop_stream(&self) {}

    /// Tells the device a teacher is (or is no longer) watching, so it can black out its wallpaper.
    /// Returns whether the wallpaper is now black. The default does nothing.
    fn set_watched(&self, watched: bool) -> bool {
        let _ = watched;
        false
    }

    /// This PC's wakeable MAC addresses, so a Console can store them and wake it later.
    fn list_macs(&self) -> Vec<String> {
        Vec::new()
    }

    /// Broadcasts a Wake-on-LAN packet for another PC on this Agent's LAN. Returns whether it went.
    fn wake_on_lan(&self, from: &PeerInfo, mac: &str) -> bool {
        let _ = (from, mac);
        false
    }

    /// Shows one frame of the teacher's screen full-screen on this PC.
    ///
    /// Returns whether the broadcast is on screen, and a reason when it is not. The default refuses,
    /// so a device that cannot present simply reports that.
    fn show_broadcast(&self, from: &PeerInfo, jpeg: &[u8], locked: bool) -> (bool, String) {
        let _ = (from, jpeg, locked);
        (false, "this device cannot show a broadcast".to_string())
    }

    /// Takes the broadcast off the screen.
    fn stop_broadcast(&self, from: &PeerInfo) -> (bool, String) {
        let _ = from;
        (false, String::new())
    }

    /// Starts or ends exam lockdown (a fullscreen lock on a separate desktop). Returns whether the
    /// PC is now locked, and a reason if it could not be. The default cannot lock.
    ///
    /// `duration_seconds` of 0 means "until released"; a positive value asks the device to auto-release
    /// the lock after that long, enforced by the device itself so it ends even if the Console goes away.
    fn set_exam(
        &self,
        from: &PeerInfo,
        on: bool,
        message: &str,
        duration_seconds: u32,
    ) -> (bool, String) {
        let _ = (from, on, message, duration_seconds);
        (false, "this device cannot lock for an exam".to_string())
    }

    /// Lists a directory inside the shared workspace folder (workspace-relative; empty = root).
    /// Returns the entries, or a reason it could not. The default has no workspace.
    fn list_files(&self, from: &PeerInfo, dir: &str) -> Result<Vec<proto::FileEntry>, String> {
        let _ = (from, dir);
        Err("this device has no file workspace".to_string())
    }

    /// The absolute path of a workspace file to **send** to the Console, or `None` if it may not be
    /// read (outside the workspace, missing). The default has nothing to send.
    fn file_read_path(&self, from: &PeerInfo, path: &str) -> Option<std::path::PathBuf> {
        let _ = (from, path);
        None
    }

    /// Recursively lists every file in the workspace (for "collect all" / diffing). The default has none.
    fn workspace_manifest(&self, from: &PeerInfo) -> Result<Vec<proto::FileEntry>, String> {
        let _ = from;
        Err("this device has no file workspace".to_string())
    }

    /// Deletes one workspace file. Returns an empty string on success, else why not. Default: refuses.
    fn delete_file(&self, from: &PeerInfo, path: &str) -> String {
        let _ = (from, path);
        "this device has no file workspace".to_string()
    }

    /// Wipes the whole workspace, returning the number of entries removed, or a reason. Default: refuses.
    fn clear_workspace(&self, from: &PeerInfo) -> Result<u32, String> {
        let _ = from;
        Err("this device has no file workspace".to_string())
    }

    /// The absolute path to **write** an uploaded file to (directory created), or `None` if the
    /// destination is outside the workspace or the name is not a plain file name. Default: no writes.
    fn file_write_path(
        &self,
        from: &PeerInfo,
        dir: &str,
        name: &str,
    ) -> Option<std::path::PathBuf> {
        let _ = (from, dir, name);
        None
    }

    /// Sets this PC's desktop wallpaper to `image` (raw PNG/JPEG/BMP bytes), laid out as `fit`.
    /// Returns whether it was applied, and a reason if not. The default cannot change the wallpaper.
    fn set_wallpaper(
        &self,
        from: &PeerInfo,
        image: &[u8],
        fit: proto::WallpaperFit,
    ) -> (bool, String) {
        let _ = (from, image, fit);
        (false, "this device cannot set its wallpaper".to_string())
    }

    /// Freezes (or releases) the student's own mouse and keyboard without the teacher taking control.
    /// Returns whether input is now blocked, and a reason if not. The default cannot lock input.
    fn set_screen_lock(&self, from: &PeerInfo, on: bool) -> (bool, String) {
        let _ = (from, on);
        (false, "this device cannot lock the screen".to_string())
    }

    /// Starts recording this PC's screen, returning what it is actually recording.
    ///
    /// The default refuses by reporting an inactive recording, so a device that cannot record simply
    /// shows as not recording.
    fn start_recording(
        &self,
        from: &PeerInfo,
        monitor: u8,
        options: proto::RecordOptions,
    ) -> proto::RecordingInfo {
        let _ = (from, monitor, options);
        proto::RecordingInfo {
            active: false,
            file: String::new(),
            frames: 0,
            width: 0,
            height: 0,
            fps: 0,
            problem: "this device cannot record its screen".into(),
        }
    }

    /// Stops any recording and reports the final state.
    fn stop_recording(&self, from: &PeerInfo) -> proto::RecordingInfo {
        self.start_recording(from, 0, proto::RecordOptions::default())
    }

    /// How the current recording is going.
    fn recording_status(&self) -> proto::RecordingInfo {
        proto::RecordingInfo {
            active: false,
            file: String::new(),
            frames: 0,
            width: 0,
            height: 0,
            fps: 0,
            problem: String::new(),
        }
    }

    /// The recordings stored on this PC.
    fn list_recordings(&self) -> Vec<proto::StoredRecording> {
        Vec::new()
    }

    /// Resolves a stored recording's **bare file name** to a full path, but only if it is a real
    /// file inside this device's recordings folder — the trust boundary that keeps "fetch a
    /// recording" from becoming "read any file". The default has none.
    fn recording_path(&self, file: &str) -> Option<std::path::PathBuf> {
        let _ = file;
        None
    }

    /// The programs this PC offers to start.
    ///
    /// The Agent publishes its own catalogue; a Console can only pick from it. The default offers
    /// nothing, so a device with no launcher simply shows an empty list.
    fn list_apps(&self) -> Vec<proto::AppEntry> {
        Vec::new()
    }

    /// The icon for a published catalogue entry, as `(width, height, top-down BGRA)`.
    ///
    /// Resolved by id the same way [`AgentDevice::launch_app`] resolves one — the Console never sends
    /// a path. The default has none, so a device without icons simply shows names only.
    fn app_icon(&self, id: u32) -> Option<(u16, u16, Vec<u8>)> {
        let _ = id;
        None
    }

    /// Starts a published program by id, returning its name and whether it started.
    ///
    /// An id this PC did not publish must not resolve — that is what keeps "launch an app" from
    /// becoming "run anything" (AGENTS.md 5).
    fn launch_app(&self, from: &PeerInfo, id: u32) -> (String, bool) {
        let _ = (from, id);
        (String::new(), false)
    }

    /// The running programs a teacher may close. System-critical ones are filtered out here.
    fn list_running(&self) -> Vec<proto::RunningApp> {
        Vec::new()
    }

    /// The icon for a running process, as `(width, height, top-down BGRA)`, resolved from its
    /// executable. The default has none, so a device without icons simply shows names only.
    fn running_icon(&self, pid: u32) -> Option<(u16, u16, Vec<u8>)> {
        let _ = pid;
        None
    }

    /// Closes a running program by process id.
    fn close_app(&self, from: &PeerInfo, pid: u32) -> bool {
        let _ = (from, pid);
        false
    }

    /// Grants or withdraws permission for a Console to drive this PC's mouse and keyboard.
    ///
    /// Returns whether control is now granted. The default refuses, so a device that cannot inject
    /// input reports "not controllable" instead of silently swallowing events.
    fn set_control(&self, from: &PeerInfo, enabled: bool) -> bool {
        let _ = (from, enabled);
        false
    }

    /// Applies input events in order, returning how many reached the OS.
    ///
    /// The second value is `true` when the device is not currently granting control, so the Console
    /// can say "that PC is not letting you drive" rather than "your click vanished".
    fn apply_input(&self, events: &[proto::InputEvent]) -> (u16, bool) {
        let _ = events;
        (0, true)
    }
}

/// A capture failure, carrying a human-readable reason.
#[derive(Debug, thiserror::Error)]
#[error("capture failed: {0}")]
pub struct CaptureError(pub String);

/// An established, trusted control session.
pub struct ControlSession {
    conn: Connection,
    send: SendStream,
    recv: RecvStream,
    peer: PeerInfo,
}

impl ControlSession {
    /// The peer on the other end.
    #[must_use]
    pub fn peer(&self) -> PeerInfo {
        self.peer
    }

    /// Console side: dial a paired Agent and establish a control session.
    ///
    /// # Errors
    /// Fails to connect, on a version mismatch, or if the Agent's key is not trusted.
    pub async fn connect(
        endpoint: &Endpoint,
        agent: impl Into<EndpointAddr>,
        trust: &TrustStore,
        local: LocalHello,
    ) -> Result<Self, EndpointError> {
        let conn = endpoint
            .connect(agent, CONTROL_ALPN)
            .await
            .map_err(|e| EndpointError::Connect(e.to_string()))?;
        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| EndpointError::Connection(e.to_string()))?;
        let peer = handshake(&conn, &mut send, &mut recv, local, trust, true).await?;
        Ok(Self {
            conn,
            send,
            recv,
            peer,
        })
    }

    /// Agent side: accept one incoming control session from a paired Console.
    ///
    /// # Errors
    /// Connection failure, a version mismatch, or an untrusted Console.
    pub async fn accept(
        endpoint: &Endpoint,
        trust: &TrustStore,
        local: LocalHello,
    ) -> Result<Self, EndpointError> {
        let incoming = endpoint.accept().await.ok_or(EndpointError::NoConnection)?;
        let conn = incoming
            .await
            .map_err(|e| EndpointError::Connection(e.to_string()))?;
        let (mut send, mut recv) = conn
            .accept_bi()
            .await
            .map_err(|e| EndpointError::Connection(e.to_string()))?;
        let peer = handshake(&conn, &mut send, &mut recv, local, trust, false).await?;
        Ok(Self {
            conn,
            send,
            recv,
            peer,
        })
    }

    /// Console side: ask for one thumbnail and wait for it.
    ///
    /// # Errors
    /// Stream failure, or the Agent replies with something other than the matching thumbnail.
    pub async fn request_thumbnail(
        &mut self,
        monitor: u8,
        max_width: u16,
        quality: u8,
    ) -> Result<Vec<u8>, EndpointError> {
        write_message(
            &mut self.send,
            &Control::RequestThumbnail {
                monitor,
                max_width,
                quality,
            },
        )
        .await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::Thumbnail {
                monitor: got, jpeg, ..
            } if got == monitor => Ok(jpeg),
            // A transient "can't capture right now" (lock screen, UAC): the caller keeps the
            // session and retries, so it is not folded into the generic refusal.
            Control::Error(ProtocolError::ScreenUnavailable) => {
                Err(EndpointError::ScreenUnavailable)
            }
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: ask which monitors the student PC has.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn request_monitors(&mut self) -> Result<Vec<Monitor>, EndpointError> {
        write_message(&mut self.send, &Control::ListMonitors).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::Monitors(monitors) => Ok(monitors),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: turn listening on or off for this PC.
    ///
    /// Returns the audio format when it started, or `None` when it stopped.
    ///
    /// # Errors
    /// Stream failure, or the Agent cannot share audio.
    pub async fn set_audio(
        &mut self,
        enabled: bool,
    ) -> Result<Option<proto::AudioFormat>, EndpointError> {
        write_message(&mut self.send, &Control::SetAudio { enabled }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::AudioState(format) => Ok(format),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: collect the sound recorded since the last call.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn request_audio(&mut self, max_samples: u32) -> Result<Vec<i16>, EndpointError> {
        write_message(&mut self.send, &Control::RequestAudio { max_samples }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::Audio { samples, .. } => Ok(samples),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: set (or clear, with an empty list) which programs the PC blocks.
    ///
    /// Returns the rule count the Agent kept and any programs it has closed since the last call.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn set_blocklist(
        &mut self,
        programs: Vec<String>,
    ) -> Result<(u16, Vec<String>), EndpointError> {
        write_message(&mut self.send, &Control::SetBlocklist { programs }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::BlocklistState { rules, closed } => Ok((rules, closed)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: tell this PC whether a teacher is watching (so it blacks its wallpaper).
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn set_watched(&mut self, watched: bool) -> Result<bool, EndpointError> {
        write_message(&mut self.send, &Control::SetWatched { watched }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::WatchedState { black } => Ok(black),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: ask for a full-resolution H.264 stream of one screen.
    ///
    /// Returns the settings the Agent will actually use. The frames arrive on a separate uni-stream:
    /// call [`ControlSession::accept_video`] next to read them.
    ///
    /// # Errors
    /// Stream failure, an unexpected reply, or a message explaining why the Agent will not stream.
    pub async fn start_stream(
        &mut self,
        monitor: u8,
        settings: proto::VideoSettings,
    ) -> Result<proto::VideoSettings, EndpointError> {
        write_message(&mut self.send, &Control::StartStream { monitor, settings }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::StreamStarted { settings, problem } if problem.is_empty() => Ok(settings),
            Control::StreamStarted { problem, .. } => Err(EndpointError::Capture(problem)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: stop the video stream.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn stop_stream(&mut self) -> Result<(), EndpointError> {
        write_message(&mut self.send, &Control::StopStream).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::StreamStopped => Ok(()),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: accept the video uni-stream the Agent opened after [`start_stream`].
    ///
    /// # Errors
    /// [`EndpointError::Connection`] if no stream arrives.
    pub async fn accept_video(&self) -> Result<VideoStream, EndpointError> {
        // QUIC only reveals a uni-stream to the peer once the sender writes bytes, so an Agent that
        // starts a stream but produces no frames would leave us waiting for ever. Bound the wait and
        // report it, rather than hanging a teacher's window.
        let recv = tokio::time::timeout(VIDEO_START_TIMEOUT, self.conn.accept_uni())
            .await
            .map_err(|_| {
                EndpointError::Connection("no video arrived from that PC in time".to_string())
            })?
            .map_err(|e| EndpointError::Connection(e.to_string()))?;
        Ok(VideoStream { recv })
    }

    /// Console side: ask this PC for its MAC addresses (to store for waking it later).
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn request_macs(&mut self) -> Result<Vec<String>, EndpointError> {
        write_message(&mut self.send, &Control::ListMacs).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::Macs(macs) => Ok(macs),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: ask this (awake) PC to broadcast a wake packet for a sleeping peer.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn wake_on_lan(&mut self, mac: String) -> Result<bool, EndpointError> {
        write_message(&mut self.send, &Control::WakeOnLan { mac }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::WakeSent { sent } => Ok(sent),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: put one frame of this console's screen on the student PC.
    ///
    /// Returns whether it is showing, plus a reason when it is not.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn show_broadcast(
        &mut self,
        jpeg: Vec<u8>,
        locked: bool,
    ) -> Result<(bool, String), EndpointError> {
        write_message(&mut self.send, &Control::ShowBroadcast { jpeg, locked }).await?;
        self.read_broadcast_state().await
    }

    /// Console side: take the broadcast off the student's screen.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn stop_broadcast(&mut self) -> Result<(bool, String), EndpointError> {
        write_message(&mut self.send, &Control::StopBroadcast).await?;
        self.read_broadcast_state().await
    }

    /// Console side: start or end exam lockdown on this PC. Returns `(locked, problem)`.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn set_exam(
        &mut self,
        on: bool,
        message: String,
        duration_seconds: u32,
    ) -> Result<(bool, String), EndpointError> {
        write_message(
            &mut self.send,
            &Control::SetExam {
                on,
                message,
                duration_seconds,
            },
        )
        .await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::ExamState { active, problem } => Ok((active, problem)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: set this PC's desktop wallpaper to `image`, laid out as `fit`. `(ok, problem)`.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn set_wallpaper(
        &mut self,
        image: Vec<u8>,
        fit: proto::WallpaperFit,
    ) -> Result<(bool, String), EndpointError> {
        write_message(&mut self.send, &Control::SetWallpaper { image, fit }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::WallpaperSet { ok, problem } => Ok((ok, problem)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: freeze (or release) the student's own input without taking control.
    /// Returns `(locked, problem)`.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn set_screen_lock(&mut self, on: bool) -> Result<(bool, String), EndpointError> {
        write_message(&mut self.send, &Control::SetScreenLock { on }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::ScreenLockState { locked, problem } => Ok((locked, problem)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// The peer's direct IP address (host:port), if a direct path is open. Returns `None` while the
    /// connection is still relay-only (no hole-punched path yet). Best-effort, for display.
    #[must_use]
    pub fn remote_ip(&self) -> Option<String> {
        self.conn
            .paths()
            .iter()
            .find_map(|path| match path.remote_addr() {
                iroh::TransportAddr::Ip(addr) => Some(addr.to_string()),
                _ => None,
            })
    }

    /// Reads the reply both broadcast requests produce.
    async fn read_broadcast_state(&mut self) -> Result<(bool, String), EndpointError> {
        match read_message::<Control>(&mut self.recv).await? {
            Control::BroadcastState { showing, problem } => Ok((showing, problem)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: start recording this PC's screen.
    ///
    /// Returns what the PC is *actually* recording after clamping.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn start_recording(
        &mut self,
        monitor: u8,
        options: proto::RecordOptions,
    ) -> Result<proto::RecordingInfo, EndpointError> {
        write_message(
            &mut self.send,
            &Control::StartRecording { monitor, options },
        )
        .await?;
        self.read_recording_state().await
    }

    /// Console side: stop the recording on this PC.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn stop_recording(&mut self) -> Result<proto::RecordingInfo, EndpointError> {
        write_message(&mut self.send, &Control::StopRecording).await?;
        self.read_recording_state().await
    }

    /// Console side: ask how the recording is going.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn recording_status(&mut self) -> Result<proto::RecordingInfo, EndpointError> {
        write_message(&mut self.send, &Control::RecordingStatus).await?;
        self.read_recording_state().await
    }

    /// Console side: list the recordings kept on this PC.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn list_recordings(&mut self) -> Result<Vec<proto::StoredRecording>, EndpointError> {
        write_message(&mut self.send, &Control::ListRecordings).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::Recordings(list) => Ok(list),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: download a stored recording from this PC into `dest_dir`, returning the saved
    /// path. The bytes arrive on their own uni-stream, so a large file never blocks the control
    /// channel.
    ///
    /// # Errors
    /// Stream failure, an unexpected reply, or the Agent cannot send that file.
    pub async fn fetch_recording(
        &mut self,
        file: &str,
        dest_dir: &std::path::Path,
    ) -> Result<std::path::PathBuf, EndpointError> {
        use tokio::io::AsyncWriteExt;
        write_message(
            &mut self.send,
            &Control::FetchRecording {
                file: file.to_string(),
            },
        )
        .await?;
        let size = match read_message::<Control>(&mut self.recv).await? {
            Control::RecordingTransfer { size, problem } if problem.is_empty() => size,
            Control::RecordingTransfer { problem, .. } => {
                return Err(EndpointError::Capture(problem));
            }
            Control::Error(err) => return Err(EndpointError::ControlRefused(err)),
            _ => return Err(EndpointError::Protocol),
        };
        let mut recv = tokio::time::timeout(VIDEO_START_TIMEOUT, self.conn.accept_uni())
            .await
            .map_err(|_| EndpointError::Connection("the recording did not start in time".into()))?
            .map_err(|e| EndpointError::Connection(e.to_string()))?;

        std::fs::create_dir_all(dest_dir).map_err(|e| EndpointError::Stream(e.to_string()))?;
        let dest = dest_dir.join(sanitize_file_name(file));
        let mut out = tokio::fs::File::create(&dest)
            .await
            .map_err(|e| EndpointError::Stream(e.to_string()))?;
        let mut remaining = size;
        let mut buf = vec![0u8; 64 * 1024];
        while remaining > 0 {
            let want = buf
                .len()
                .min(usize::try_from(remaining).unwrap_or(buf.len()));
            recv.read_exact(&mut buf[..want])
                .await
                .map_err(|e| EndpointError::Stream(e.to_string()))?;
            out.write_all(&buf[..want])
                .await
                .map_err(|e| EndpointError::Stream(e.to_string()))?;
            remaining -= want as u64;
        }
        out.flush()
            .await
            .map_err(|e| EndpointError::Stream(e.to_string()))?;
        Ok(dest)
    }

    /// Console side: list a directory in the student's shared workspace (empty = root).
    ///
    /// # Errors
    /// Stream failure, an unexpected reply, or the directory is outside the workspace / unreadable.
    pub async fn list_files(&mut self, dir: &str) -> Result<Vec<proto::FileEntry>, EndpointError> {
        write_message(
            &mut self.send,
            &Control::ListFiles {
                dir: dir.to_string(),
            },
        )
        .await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::Files { entries, problem } if problem.is_empty() => Ok(entries),
            Control::Files { problem, .. } => Err(EndpointError::Capture(problem)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: download a workspace file (workspace-relative `path`) into `dest_dir`, returning
    /// the saved path. The bytes ride their own uni-stream, like a recording.
    ///
    /// # Errors
    /// Stream failure, an unexpected reply, or the Agent cannot send that file.
    pub async fn fetch_file(
        &mut self,
        path: &str,
        dest_dir: &std::path::Path,
    ) -> Result<std::path::PathBuf, EndpointError> {
        use tokio::io::AsyncWriteExt;
        write_message(
            &mut self.send,
            &Control::FetchFile {
                path: path.to_string(),
            },
        )
        .await?;
        let size = match read_message::<Control>(&mut self.recv).await? {
            Control::FileTransfer { size, problem } if problem.is_empty() => size,
            Control::FileTransfer { problem, .. } => return Err(EndpointError::Capture(problem)),
            Control::Error(err) => return Err(EndpointError::ControlRefused(err)),
            _ => return Err(EndpointError::Protocol),
        };
        let mut recv = tokio::time::timeout(VIDEO_START_TIMEOUT, self.conn.accept_uni())
            .await
            .map_err(|_| EndpointError::Connection("the file did not start in time".into()))?
            .map_err(|e| EndpointError::Connection(e.to_string()))?;

        std::fs::create_dir_all(dest_dir).map_err(|e| EndpointError::Stream(e.to_string()))?;
        let dest = dest_dir.join(sanitize_file_name(path));
        let mut out = tokio::fs::File::create(&dest)
            .await
            .map_err(|e| EndpointError::Stream(e.to_string()))?;
        let mut remaining = size;
        let mut buf = vec![0u8; 64 * 1024];
        while remaining > 0 {
            let want = buf
                .len()
                .min(usize::try_from(remaining).unwrap_or(buf.len()));
            recv.read_exact(&mut buf[..want])
                .await
                .map_err(|e| EndpointError::Stream(e.to_string()))?;
            out.write_all(&buf[..want])
                .await
                .map_err(|e| EndpointError::Stream(e.to_string()))?;
            remaining -= want as u64;
        }
        out.flush()
            .await
            .map_err(|e| EndpointError::Stream(e.to_string()))?;
        Ok(dest)
    }

    /// Console side: upload `bytes` to the student's workspace as `name` inside directory `dir`.
    ///
    /// # Errors
    /// Stream failure, the Agent refusing the destination, or the write failing part way.
    pub async fn send_file(
        &mut self,
        dir: &str,
        name: &str,
        bytes: &[u8],
    ) -> Result<(), EndpointError> {
        write_message(
            &mut self.send,
            &Control::SendFile {
                dir: dir.to_string(),
                name: name.to_string(),
                size: bytes.len() as u64,
            },
        )
        .await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::FileSendReady { problem } if problem.is_empty() => {}
            Control::FileSendReady { problem } => return Err(EndpointError::Capture(problem)),
            Control::Error(err) => return Err(EndpointError::ControlRefused(err)),
            _ => return Err(EndpointError::Protocol),
        }
        // The Agent is now waiting on a uni-stream for the bytes.
        let mut uni = self
            .conn
            .open_uni()
            .await
            .map_err(|e| EndpointError::Connection(e.to_string()))?;
        uni.write_all(bytes)
            .await
            .map_err(|e| EndpointError::Stream(e.to_string()))?;
        uni.finish()
            .map_err(|e| EndpointError::Stream(e.to_string()))?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::FileSent { problem } if problem.is_empty() => Ok(()),
            Control::FileSent { problem } => Err(EndpointError::Capture(problem)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: recursively list every file in the student's workspace (paths in each entry's
    /// `name`), for collecting or diffing against a baseline.
    ///
    /// # Errors
    /// Stream failure, an unexpected reply, or the workspace could not be walked.
    pub async fn list_workspace(&mut self) -> Result<Vec<proto::FileEntry>, EndpointError> {
        write_message(&mut self.send, &Control::ListWorkspace).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::WorkspaceManifest { files, problem } if problem.is_empty() => Ok(files),
            Control::WorkspaceManifest { problem, .. } => Err(EndpointError::Capture(problem)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: delete one file (workspace-relative `path`) from the student's workspace.
    ///
    /// # Errors
    /// Stream failure, an unexpected reply, or the Agent refusing the delete.
    pub async fn delete_file(&mut self, path: &str) -> Result<(), EndpointError> {
        write_message(
            &mut self.send,
            &Control::DeleteFile {
                path: path.to_string(),
            },
        )
        .await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::FileDeleted { problem } if problem.is_empty() => Ok(()),
            Control::FileDeleted { problem } => Err(EndpointError::Capture(problem)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: wipe the whole workspace, returning how many entries were removed.
    ///
    /// # Errors
    /// Stream failure, an unexpected reply, or the wipe failing.
    pub async fn clear_workspace(&mut self) -> Result<u32, EndpointError> {
        write_message(&mut self.send, &Control::ClearWorkspace).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::WorkspaceCleared { removed, problem } if problem.is_empty() => Ok(removed),
            Control::WorkspaceCleared { problem, .. } => Err(EndpointError::Capture(problem)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Reads the one reply every recording request produces.
    async fn read_recording_state(&mut self) -> Result<proto::RecordingInfo, EndpointError> {
        match read_message::<Control>(&mut self.recv).await? {
            Control::RecordingState(info) => Ok(info),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: ask what programs this PC can start.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn request_apps(&mut self) -> Result<Vec<proto::AppEntry>, EndpointError> {
        write_message(&mut self.send, &Control::ListApps).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::Apps(apps) => Ok(apps),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: ask for one program's icon (lazy — call only for rows on screen).
    ///
    /// Returns `(width, height, top-down BGRA)`, or `None` if the PC has no icon for that id.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn request_app_icon(
        &mut self,
        id: u32,
    ) -> Result<Option<(u16, u16, Vec<u8>)>, EndpointError> {
        write_message(&mut self.send, &Control::FetchAppIcon { id }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::AppIcon {
                width,
                height,
                bgra,
                ..
            } => {
                if bgra.is_empty() || width == 0 || height == 0 {
                    Ok(None)
                } else {
                    Ok(Some((width, height, bgra)))
                }
            }
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: ask for the icon of a running process (lazy, matches [`Self::request_app_icon`]).
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn request_running_icon(
        &mut self,
        pid: u32,
    ) -> Result<Option<(u16, u16, Vec<u8>)>, EndpointError> {
        write_message(&mut self.send, &Control::FetchRunningIcon { pid }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::AppIcon {
                width,
                height,
                bgra,
                ..
            } => {
                if bgra.is_empty() || width == 0 || height == 0 {
                    Ok(None)
                } else {
                    Ok(Some((width, height, bgra)))
                }
            }
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: start one of the programs this PC published.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn launch_app(&mut self, id: u32) -> Result<(String, bool), EndpointError> {
        write_message(&mut self.send, &Control::LaunchApp { id }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::AppLaunched { name, started } => Ok((name, started)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: ask what is running on this PC.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn request_running(&mut self) -> Result<Vec<proto::RunningApp>, EndpointError> {
        write_message(&mut self.send, &Control::ListRunning).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::Running(apps) => Ok(apps),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: close a running program by process id.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn close_app(&mut self, pid: u32) -> Result<bool, EndpointError> {
        write_message(&mut self.send, &Control::CloseApp { pid }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::AppClosed { closed } => Ok(closed),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: take or release control of the PC's mouse and keyboard.
    ///
    /// Returns whether control is now granted.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn set_control(&mut self, enabled: bool) -> Result<bool, EndpointError> {
        write_message(&mut self.send, &Control::SetControl { enabled }).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::ControlState { enabled } => Ok(enabled),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: send a batch of input events and wait for the acknowledgement.
    ///
    /// Returns `(applied, refused)`.
    ///
    /// # Errors
    /// Stream failure, an unexpected reply, or a batch longer than [`proto::MAX_INPUT_BATCH`].
    pub async fn send_input(
        &mut self,
        events: Vec<proto::InputEvent>,
    ) -> Result<(u16, bool), EndpointError> {
        if events.len() > proto::MAX_INPUT_BATCH {
            return Err(EndpointError::Protocol);
        }
        write_message(&mut self.send, &Control::Input(events)).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::InputDone { applied, refused } => Ok((applied, refused)),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Console side: ask the Agent to do one [`proto::Action`] and wait for its answer.
    ///
    /// # Errors
    /// Stream failure, or a reply that does not match the requested action.
    pub async fn perform(
        &mut self,
        action: proto::Action,
    ) -> Result<proto::ActionOutcome, EndpointError> {
        write_message(&mut self.send, &Control::Perform(action)).await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::ActionDone {
                action: done,
                outcome,
            } if done == action => Ok(outcome),
            Control::Error(err) => Err(EndpointError::ControlRefused(err)),
            _ => Err(EndpointError::Protocol),
        }
    }

    /// Agent side: serve Console requests until the Console closes the session.
    ///
    /// Returns `Ok(())` on a clean close. Capture happens only inside a request, so this loop is idle
    /// (no CPU, no capture) whenever the Console is not asking.
    ///
    /// # Errors
    /// A capture error ends the session with [`EndpointError::Capture`].
    pub async fn serve(mut self, source: &impl AgentDevice) -> Result<(), EndpointError> {
        let result = self.serve_inner(source).await;
        // Whatever ended the session — a clean close, a write error, or an unexpected message — must
        // not leave the student with their input blocked or a modifier stuck down because control or a
        // screen lock was still on when the Console vanished. Both are idempotent, so releasing them is
        // a no-op when neither was set. (A teacher who wants a lasting freeze re-applies on reconnect.)
        source.set_control(&self.peer, false);
        source.set_screen_lock(&self.peer, false);
        result
    }

    /// The request loop; [`ControlSession::serve`] wraps this so control is always released on exit.
    async fn serve_inner(&mut self, source: &impl AgentDevice) -> Result<(), EndpointError> {
        let mut seq = 0u64;
        let mut audio_seq = 0u64;
        loop {
            let request = match read_message::<Control>(&mut self.recv).await {
                Ok(message) => message,
                Err(_) => return Ok(()), // Console closed the stream: clean end.
            };
            match request {
                Control::RequestThumbnail {
                    monitor,
                    max_width,
                    quality,
                } => {
                    // A capture failure is almost always transient — a lock screen, a UAC secure
                    // desktop, a resolution change. Ending the whole session on it made both sides
                    // reconnect at once and again, an endless storm (seen live). Instead, tell the
                    // Console the screen is momentarily unavailable and keep serving; the next
                    // request usually succeeds once the desktop is back.
                    match source.capture_thumbnail(monitor, max_width, quality) {
                        Ok(jpeg) => {
                            seq += 1;
                            write_message(
                                &mut self.send,
                                &Control::Thumbnail { monitor, seq, jpeg },
                            )
                            .await?;
                        }
                        Err(_) => {
                            write_message(
                                &mut self.send,
                                &Control::Error(proto::ProtocolError::ScreenUnavailable),
                            )
                            .await?;
                        }
                    }
                }
                Control::ListMonitors => {
                    write_message(&mut self.send, &Control::Monitors(source.monitors())).await?;
                }
                Control::SetAudio { enabled } => {
                    // A PC that cannot share audio says so; it must not kill the screen session.
                    let reply = match source.set_audio(enabled) {
                        Ok(format) => Control::AudioState(format),
                        Err(_) => Control::AudioState(None),
                    };
                    write_message(&mut self.send, &reply).await?;
                }
                Control::RequestAudio { max_samples } => {
                    let samples = source.take_audio(max_samples as usize);
                    audio_seq += 1;
                    write_message(
                        &mut self.send,
                        &Control::Audio {
                            seq: audio_seq,
                            samples,
                        },
                    )
                    .await?;
                }
                Control::StartStream { monitor, settings } => {
                    match source.start_stream(&self.peer, monitor, settings) {
                        Ok((actual, mut packets)) => {
                            write_message(
                                &mut self.send,
                                &Control::StreamStarted {
                                    settings: actual,
                                    problem: String::new(),
                                },
                            )
                            .await?;
                            // Video rides its own uni-stream, so a slow decoder can never stall the
                            // control channel. The task ends when the encoder's channel closes.
                            let conn = self.conn.clone();
                            tokio::spawn(async move {
                                let Ok(mut video) = conn.open_uni().await else {
                                    return;
                                };
                                while let Some(packet) = packets.recv().await {
                                    if write_video_frame(&mut video, &packet).await.is_err() {
                                        break;
                                    }
                                }
                                let _ = video.finish();
                            });
                        }
                        Err(problem) => {
                            write_message(
                                &mut self.send,
                                &Control::StreamStarted { settings, problem },
                            )
                            .await?;
                        }
                    }
                }
                Control::StopStream => {
                    source.stop_stream();
                    write_message(&mut self.send, &Control::StreamStopped).await?;
                }
                Control::SetWatched { watched } => {
                    let black = source.set_watched(watched);
                    write_message(&mut self.send, &Control::WatchedState { black }).await?;
                }
                Control::ListMacs => {
                    write_message(&mut self.send, &Control::Macs(source.list_macs())).await?;
                }
                Control::WakeOnLan { mac } => {
                    let sent = source.wake_on_lan(&self.peer, &mac);
                    write_message(&mut self.send, &Control::WakeSent { sent }).await?;
                }
                Control::ShowBroadcast { jpeg, locked } => {
                    let (showing, problem) = source.show_broadcast(&self.peer, &jpeg, locked);
                    write_message(
                        &mut self.send,
                        &Control::BroadcastState { showing, problem },
                    )
                    .await?;
                }
                Control::StopBroadcast => {
                    let (showing, problem) = source.stop_broadcast(&self.peer);
                    write_message(
                        &mut self.send,
                        &Control::BroadcastState { showing, problem },
                    )
                    .await?;
                }
                Control::SetExam {
                    on,
                    message,
                    duration_seconds,
                } => {
                    let (active, problem) =
                        source.set_exam(&self.peer, on, &message, duration_seconds);
                    write_message(&mut self.send, &Control::ExamState { active, problem }).await?;
                }
                Control::SetWallpaper { image, fit } => {
                    let (ok, problem) = source.set_wallpaper(&self.peer, &image, fit);
                    write_message(&mut self.send, &Control::WallpaperSet { ok, problem }).await?;
                }
                Control::SetScreenLock { on } => {
                    let (locked, problem) = source.set_screen_lock(&self.peer, on);
                    write_message(
                        &mut self.send,
                        &Control::ScreenLockState { locked, problem },
                    )
                    .await?;
                }
                Control::StartRecording { monitor, options } => {
                    let info = source.start_recording(&self.peer, monitor, options);
                    write_message(&mut self.send, &Control::RecordingState(info)).await?;
                }
                Control::StopRecording => {
                    let info = source.stop_recording(&self.peer);
                    write_message(&mut self.send, &Control::RecordingState(info)).await?;
                }
                Control::RecordingStatus => {
                    let info = source.recording_status();
                    write_message(&mut self.send, &Control::RecordingState(info)).await?;
                }
                Control::ListRecordings => {
                    let list = source.list_recordings();
                    write_message(&mut self.send, &Control::Recordings(list)).await?;
                }
                Control::FetchRecording { file } => match source.recording_path(&file) {
                    Some(path) => {
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        write_message(
                            &mut self.send,
                            &Control::RecordingTransfer {
                                size,
                                problem: String::new(),
                            },
                        )
                        .await?;
                        // The bytes ride their own uni-stream so a big file never stalls control.
                        let conn = self.conn.clone();
                        tokio::spawn(async move {
                            use tokio::io::AsyncReadExt;
                            let Ok(mut uni) = conn.open_uni().await else {
                                return;
                            };
                            if let Ok(mut file) = tokio::fs::File::open(&path).await {
                                let mut buf = vec![0u8; 64 * 1024];
                                loop {
                                    match file.read(&mut buf).await {
                                        Ok(0) => break,
                                        Ok(n) if uni.write_all(&buf[..n]).await.is_ok() => {}
                                        _ => break,
                                    }
                                }
                            }
                            let _ = uni.finish();
                        });
                    }
                    None => {
                        write_message(
                            &mut self.send,
                            &Control::RecordingTransfer {
                                size: 0,
                                problem: "no such recording on this PC".into(),
                            },
                        )
                        .await?;
                    }
                },
                Control::ListFiles { dir } => {
                    let (entries, problem) = match source.list_files(&self.peer, &dir) {
                        Ok(entries) => (entries, String::new()),
                        Err(problem) => (Vec::new(), problem),
                    };
                    write_message(&mut self.send, &Control::Files { entries, problem }).await?;
                }
                Control::FetchFile { path } => match source.file_read_path(&self.peer, &path) {
                    Some(path) => {
                        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        write_message(
                            &mut self.send,
                            &Control::FileTransfer {
                                size,
                                problem: String::new(),
                            },
                        )
                        .await?;
                        // The bytes ride their own uni-stream so a big file never stalls control.
                        let conn = self.conn.clone();
                        tokio::spawn(async move {
                            use tokio::io::AsyncReadExt;
                            let Ok(mut uni) = conn.open_uni().await else {
                                return;
                            };
                            if let Ok(mut file) = tokio::fs::File::open(&path).await {
                                let mut buf = vec![0u8; 64 * 1024];
                                loop {
                                    match file.read(&mut buf).await {
                                        Ok(0) => break,
                                        Ok(n) if uni.write_all(&buf[..n]).await.is_ok() => {}
                                        _ => break,
                                    }
                                }
                            }
                            let _ = uni.finish();
                        });
                    }
                    None => {
                        write_message(
                            &mut self.send,
                            &Control::FileTransfer {
                                size: 0,
                                problem: "no such file in the workspace".into(),
                            },
                        )
                        .await?;
                    }
                },
                Control::SendFile { dir, name, size } => {
                    match source.file_write_path(&self.peer, &dir, &name) {
                        Some(dest) => {
                            write_message(
                                &mut self.send,
                                &Control::FileSendReady {
                                    problem: String::new(),
                                },
                            )
                            .await?;
                            // Receive the bytes inline (one upload at a time) so the FileSent ack that
                            // follows is accurate. They arrive on a uni-stream the Console opens.
                            let problem = receive_file(&self.conn, &dest, size).await;
                            write_message(&mut self.send, &Control::FileSent { problem }).await?;
                        }
                        None => {
                            write_message(
                                &mut self.send,
                                &Control::FileSendReady {
                                    problem: "that destination is not allowed".into(),
                                },
                            )
                            .await?;
                        }
                    }
                }
                Control::ListWorkspace => {
                    let (files, problem) = match source.workspace_manifest(&self.peer) {
                        Ok(files) => (files, String::new()),
                        Err(problem) => (Vec::new(), problem),
                    };
                    write_message(
                        &mut self.send,
                        &Control::WorkspaceManifest { files, problem },
                    )
                    .await?;
                }
                Control::DeleteFile { path } => {
                    let problem = source.delete_file(&self.peer, &path);
                    write_message(&mut self.send, &Control::FileDeleted { problem }).await?;
                }
                Control::ClearWorkspace => {
                    let (removed, problem) = match source.clear_workspace(&self.peer) {
                        Ok(removed) => (removed, String::new()),
                        Err(problem) => (0, problem),
                    };
                    write_message(
                        &mut self.send,
                        &Control::WorkspaceCleared { removed, problem },
                    )
                    .await?;
                }
                Control::ListApps => {
                    write_message(&mut self.send, &Control::Apps(source.list_apps())).await?;
                }
                Control::FetchAppIcon { id } => {
                    let (width, height, bgra) = source.app_icon(id).unwrap_or((0, 0, Vec::new()));
                    write_message(
                        &mut self.send,
                        &Control::AppIcon {
                            id,
                            width,
                            height,
                            bgra,
                        },
                    )
                    .await?;
                }
                Control::FetchRunningIcon { pid } => {
                    let (width, height, bgra) =
                        source.running_icon(pid).unwrap_or((0, 0, Vec::new()));
                    write_message(
                        &mut self.send,
                        &Control::AppIcon {
                            id: pid,
                            width,
                            height,
                            bgra,
                        },
                    )
                    .await?;
                }
                Control::LaunchApp { id } => {
                    let (name, started) = source.launch_app(&self.peer, id);
                    write_message(&mut self.send, &Control::AppLaunched { name, started }).await?;
                }
                Control::ListRunning => {
                    write_message(&mut self.send, &Control::Running(source.list_running())).await?;
                }
                Control::CloseApp { pid } => {
                    let closed = source.close_app(&self.peer, pid);
                    write_message(&mut self.send, &Control::AppClosed { closed }).await?;
                }
                Control::SetControl { enabled } => {
                    let enabled = source.set_control(&self.peer, enabled);
                    write_message(&mut self.send, &Control::ControlState { enabled }).await?;
                }
                Control::Input(events) => {
                    // Trust boundary: an over-long batch is refused outright, never truncated and
                    // never applied in part, so a malformed peer cannot flood the input queue.
                    let (applied, refused) = if events.len() > proto::MAX_INPUT_BATCH {
                        (0, true)
                    } else {
                        source.apply_input(&events)
                    };
                    write_message(&mut self.send, &Control::InputDone { applied, refused }).await?;
                }
                Control::SetBlocklist { programs } => {
                    let rules = source.set_blocklist(programs);
                    let closed = source.take_blocked();
                    write_message(&mut self.send, &Control::BlocklistState { rules, closed })
                        .await?;
                }
                Control::Perform(action) => {
                    let outcome = source.perform(&self.peer, action);
                    write_message(&mut self.send, &Control::ActionDone { action, outcome }).await?;
                }
                Control::Ping(nonce) => {
                    write_message(&mut self.send, &Control::Pong(nonce)).await?
                }
                _ => return Ok(()), // unexpected message: end the session
            }
        }
    }

    /// Closes the session, letting the peer tear down gracefully.
    pub fn close(self) {
        self.conn.close(0u32.into(), b"session closed");
    }
}

/// How long to wait for the first video bytes before giving up on a stream.
const VIDEO_START_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// The largest single encoded video frame we will accept, as a trust boundary. A 4K keyframe is a
/// few hundred kilobytes; 8 MiB is far above anything legitimate and far below anything dangerous.
const MAX_VIDEO_FRAME: u32 = 8 * 1024 * 1024;

/// Reduces a name to its last path component, so a name from a peer can never write outside the
/// chosen folder (`..\..\evil` becomes `evil`).
fn sanitize_file_name(name: &str) -> String {
    std::path::Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "recording.bin".to_string())
}

/// Agent side: receives an uploaded file. Accepts the uni-stream the Console opened and writes exactly
/// `size` bytes to `dest`. Returns an empty string on success, else a reason; a failed transfer removes
/// the half-written file so a student is never left a truncated one.
async fn receive_file(conn: &Connection, dest: &std::path::Path, size: u64) -> String {
    use tokio::io::AsyncWriteExt;
    let mut recv = match tokio::time::timeout(VIDEO_START_TIMEOUT, conn.accept_uni()).await {
        Ok(Ok(recv)) => recv,
        _ => return "the upload did not start in time".to_string(),
    };
    let mut out = match tokio::fs::File::create(dest).await {
        Ok(file) => file,
        Err(err) => return format!("could not create the file: {err}"),
    };
    let mut remaining = size;
    let mut buf = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let want = buf
            .len()
            .min(usize::try_from(remaining).unwrap_or(buf.len()));
        if recv.read_exact(&mut buf[..want]).await.is_err() {
            let _ = tokio::fs::remove_file(dest).await;
            return "the upload was interrupted".to_string();
        }
        if out.write_all(&buf[..want]).await.is_err() {
            let _ = tokio::fs::remove_file(dest).await;
            return "could not write the file".to_string();
        }
        remaining -= want as u64;
    }
    if out.flush().await.is_err() {
        return "could not finish writing the file".to_string();
    }
    String::new()
}

/// Writes one length-prefixed encoded frame to the video uni-stream.
async fn write_video_frame(send: &mut SendStream, packet: &[u8]) -> Result<(), EndpointError> {
    let len = u32::try_from(packet.len()).map_err(|_| EndpointError::TooLarge(u32::MAX))?;
    send.write_all(&len.to_be_bytes())
        .await
        .map_err(|e| EndpointError::Stream(e.to_string()))?;
    send.write_all(packet)
        .await
        .map_err(|e| EndpointError::Stream(e.to_string()))?;
    Ok(())
}

/// The incoming side of a video stream: length-prefixed H.264 packets.
pub struct VideoStream {
    recv: RecvStream,
}

impl VideoStream {
    /// Reads the next encoded frame, or `None` when the Agent closed the stream.
    ///
    /// # Errors
    /// [`EndpointError`] on a stream failure or an over-long frame.
    pub async fn next_frame(&mut self) -> Result<Option<Vec<u8>>, EndpointError> {
        let mut header = [0u8; 4];
        match self.recv.read_exact(&mut header).await {
            Ok(()) => {}
            Err(_) => return Ok(None), // clean end of stream
        }
        let len = u32::from_be_bytes(header);
        if len > MAX_VIDEO_FRAME {
            return Err(EndpointError::TooLarge(len));
        }
        let mut packet = vec![0u8; len as usize];
        self.recv
            .read_exact(&mut packet)
            .await
            .map_err(|e| EndpointError::Stream(e.to_string()))?;
        Ok(Some(packet))
    }
}

/// Exchanges [`proto::Hello`], checks the protocol version, and enforces mutual trust.
///
/// The peer's identity is its transport public key (`conn.remote_id()`), not the `device_id` it
/// announces — the handle is only for display. `initiator` writes its Hello first (the dialer).
async fn handshake(
    conn: &Connection,
    send: &mut SendStream,
    recv: &mut RecvStream,
    local: LocalHello,
    trust: &TrustStore,
    initiator: bool,
) -> Result<PeerInfo, EndpointError> {
    let my_hello = Control::Hello(Hello {
        protocol_version: PROTOCOL_VERSION,
        role: local.role,
        device_id: local.device_id,
        capabilities: local.capabilities,
    });
    let peer_hello = if initiator {
        write_message(send, &my_hello).await?;
        read_hello(recv).await?
    } else {
        let peer = read_hello(recv).await?;
        write_message(send, &my_hello).await?;
        peer
    };

    let peer_key = *conn.remote_id().as_bytes();
    if !trust.is_trusted(&peer_key) {
        // Tell the peer why, best-effort, before refusing.
        let _ = write_message(send, &Control::Error(ProtocolError::Unauthorized)).await;
        return Err(EndpointError::ControlRefused(ProtocolError::Unauthorized));
    }

    Ok(PeerInfo {
        public_key: peer_key,
        device_id: peer_hello.device_id,
        role: peer_hello.role,
        capabilities: peer_hello.capabilities,
    })
}

/// Reads the peer's opening message, which must be a version-compatible [`Control::Hello`].
async fn read_hello(recv: &mut RecvStream) -> Result<Hello, EndpointError> {
    match read_message::<Control>(recv).await? {
        Control::Hello(hello) => {
            proto::version_compatible(hello.protocol_version)
                .map_err(EndpointError::ControlRefused)?;
            Ok(hello)
        }
        _ => Err(EndpointError::Protocol),
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize_file_name;

    #[test]
    fn a_recording_name_can_never_escape_its_folder() {
        // Whatever a peer sends, only the final component survives, so a download can never be
        // written outside the chosen directory.
        assert_eq!(sanitize_file_name("recording-abc.mp4"), "recording-abc.mp4");
        assert_eq!(
            sanitize_file_name(r"..\..\Windows\system32\evil.dll"),
            "evil.dll"
        );
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name(""), "recording.bin");
        assert_eq!(sanitize_file_name("/"), "recording.bin");
    }
}
