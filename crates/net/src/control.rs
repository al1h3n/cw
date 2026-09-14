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

    /// Returns a JPEG of `monitor`, scaled to at most `max_width` pixels wide.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if the monitor is unavailable or capture fails.
    fn capture_thumbnail(&self, monitor: u8, max_width: u16) -> Result<Vec<u8>, CaptureError>;

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

    /// Shows one frame of the teacher's screen full-screen on this PC.
    ///
    /// Returns whether the broadcast is on screen, and a reason when it is not. The default refuses,
    /// so a device that cannot present simply reports that.
    fn show_broadcast(&self, from: &PeerInfo, jpeg: &[u8]) -> (bool, String) {
        let _ = (from, jpeg);
        (false, "this device cannot show a broadcast".to_string())
    }

    /// Takes the broadcast off the screen.
    fn stop_broadcast(&self, from: &PeerInfo) -> (bool, String) {
        let _ = from;
        (false, String::new())
    }

    /// Starts recording this PC's screen, returning what it is actually recording.
    ///
    /// The default refuses by reporting an inactive recording, so a device that cannot record simply
    /// shows as not recording.
    fn start_recording(
        &self,
        from: &PeerInfo,
        monitor: u8,
        max_width: u32,
        max_height: u32,
        fps: u32,
    ) -> proto::RecordingInfo {
        let _ = (from, monitor, max_width, max_height, fps);
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
        self.start_recording(from, 0, 0, 0, 0)
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

    /// The programs this PC offers to start.
    ///
    /// The Agent publishes its own catalogue; a Console can only pick from it. The default offers
    /// nothing, so a device with no launcher simply shows an empty list.
    fn list_apps(&self) -> Vec<proto::AppEntry> {
        Vec::new()
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
    ) -> Result<Vec<u8>, EndpointError> {
        write_message(
            &mut self.send,
            &Control::RequestThumbnail { monitor, max_width },
        )
        .await?;
        match read_message::<Control>(&mut self.recv).await? {
            Control::Thumbnail {
                monitor: got, jpeg, ..
            } if got == monitor => Ok(jpeg),
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

    /// Console side: put one frame of this console's screen on the student PC.
    ///
    /// Returns whether it is showing, plus a reason when it is not.
    ///
    /// # Errors
    /// Stream failure, or an unexpected reply.
    pub async fn show_broadcast(&mut self, jpeg: Vec<u8>) -> Result<(bool, String), EndpointError> {
        write_message(&mut self.send, &Control::ShowBroadcast { jpeg }).await?;
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
        max_width: u32,
        max_height: u32,
        fps: u32,
    ) -> Result<proto::RecordingInfo, EndpointError> {
        write_message(
            &mut self.send,
            &Control::StartRecording {
                monitor,
                max_width,
                max_height,
                fps,
            },
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
        let mut seq = 0u64;
        let mut audio_seq = 0u64;
        loop {
            let request = match read_message::<Control>(&mut self.recv).await {
                Ok(message) => message,
                Err(_) => return Ok(()), // Console closed the stream: clean end.
            };
            match request {
                Control::RequestThumbnail { monitor, max_width } => {
                    let jpeg = source
                        .capture_thumbnail(monitor, max_width)
                        .map_err(|e| EndpointError::Capture(e.0))?;
                    seq += 1;
                    write_message(&mut self.send, &Control::Thumbnail { monitor, seq, jpeg })
                        .await?;
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
                Control::ShowBroadcast { jpeg } => {
                    let (showing, problem) = source.show_broadcast(&self.peer, &jpeg);
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
                Control::StartRecording {
                    monitor,
                    max_width,
                    max_height,
                    fps,
                } => {
                    let info =
                        source.start_recording(&self.peer, monitor, max_width, max_height, fps);
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
                Control::ListApps => {
                    write_message(&mut self.send, &Control::Apps(source.list_apps())).await?;
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
