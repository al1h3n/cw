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

/// Produces screen thumbnails on demand. The Agent supplies one; tests supply a fake.
///
/// Capture happens only when [`capture_thumbnail`](CaptureSource::capture_thumbnail) is called, which
/// is only when a Console asks — there is no background capture loop to leave running.
pub trait CaptureSource {
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

    /// Agent side: serve thumbnail requests until the Console closes the session.
    ///
    /// Returns `Ok(())` on a clean close. Capture happens only inside a request, so this loop is idle
    /// (no CPU, no capture) whenever the Console is not asking.
    ///
    /// # Errors
    /// A capture error ends the session with [`EndpointError::Capture`].
    pub async fn serve_thumbnails(
        mut self,
        source: &impl CaptureSource,
    ) -> Result<(), EndpointError> {
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
