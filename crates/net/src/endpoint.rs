//! The iroh endpoint and the pairing handshake carried over it (Phase 1.3b).
//!
//! [`bind`] turns a device [`Identity`] into a live iroh endpoint (dial-by-public-key, NAT traversal,
//! relay fallback, plus mDNS so LAN peers find each other with no internet). [`console_accept_pairing`]
//! and [`agent_request_pairing`] run the [`crate::pairing`] logic over a real connection: the
//! transport authenticates both public keys, and on success each side pins the other.
//!
//! Framing: every message is a 4-byte big-endian length followed by its postcard bytes, capped at
//! [`MAX_MESSAGE_BYTES`] so a peer can never make us allocate without bound (trust boundary).

use iroh::{
    Endpoint, EndpointAddr, SecretKey,
    endpoint::{RecvStream, SendStream, presets},
};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use proto::{DeviceId, PairMessage, PairRejection};
use serde::{Serialize, de::DeserializeOwned};

use crate::{Identity, PairingCode, PairingSession, TrustStore};

/// ALPN for the pairing protocol. Bumping the trailing number is a breaking change.
pub const PAIRING_ALPN: &[u8] = b"cowatcher/pair/1";

/// ALPN for the ongoing control session (used after pairing).
pub const CONTROL_ALPN: &[u8] = b"cowatcher/control/1";

/// Largest message we will read from a peer.
///
/// Sized for one full-resolution JPEG frame (a 4K screen compresses to well under this), not for
/// control messages, which are tiny. It is still a hard bound, so a hostile peer cannot make us
/// allocate without limit.
pub const MAX_MESSAGE_BYTES: u32 = 8 * 1024 * 1024;

/// How long the Console keeps a finished connection alive so its reply is delivered before drop.
const CLOSE_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// A peer we just paired with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairedPeer {
    /// The peer's transport public key (the pinned identity).
    pub public_key: [u8; 32],
    /// The short handle derived from it.
    pub device_id: DeviceId,
}

/// Errors from the endpoint and the pairing handshake.
#[derive(Debug, thiserror::Error)]
pub enum EndpointError {
    /// Binding the local endpoint failed.
    #[error("bind endpoint: {0}")]
    Bind(String),
    /// Dialing the peer failed.
    #[error("connect: {0}")]
    Connect(String),
    /// An established connection failed.
    #[error("connection: {0}")]
    Connection(String),
    /// Reading or writing a stream failed.
    #[error("stream i/o: {0}")]
    Stream(String),
    /// A peer announced a message larger than [`MAX_MESSAGE_BYTES`].
    #[error("peer message too large: {0} bytes")]
    TooLarge(u32),
    /// A message could not be decoded, or was the wrong kind for this step.
    #[error("malformed or unexpected message")]
    Protocol,
    /// The Console refused the pairing (also returned to the operator so they see the reason).
    #[error("pairing refused: {0}")]
    Rejected(#[from] PairRejection),
    /// A control-session request was refused (e.g. an untrusted or version-mismatched peer).
    #[error("control refused: {0}")]
    ControlRefused(#[from] proto::ProtocolError),
    /// Capturing a thumbnail failed on the Agent.
    #[error("capture failed: {0}")]
    Capture(String),
    /// No incoming connection was available to accept.
    #[error("no incoming connection")]
    NoConnection,
}

/// Milliseconds since the Unix epoch — the wall clock the transport verifies pairing against.
///
/// A [`PairingSession`] handed to [`console_accept_pairing`] **must** be created with this same
/// clock (`PairingSession::new(code, endpoint::now_ms())`), or every attempt looks expired. The
/// pure [`crate::pairing`] tests inject their own time instead.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Binds a live endpoint for this device: its key is the device key, and mDNS is enabled so peers on
/// the same LAN are found without internet. It accepts incoming [`PAIRING_ALPN`] connections.
///
/// # Errors
/// Returns [`EndpointError::Bind`] if the socket cannot be opened.
pub async fn bind(identity: &Identity) -> Result<Endpoint, EndpointError> {
    // Reconstruct an owned SecretKey from bytes rather than relying on Clone.
    let secret_key = SecretKey::from_bytes(&identity.secret_key().to_bytes());
    Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .alpns(vec![PAIRING_ALPN.to_vec(), CONTROL_ALPN.to_vec()])
        .address_lookup(MdnsAddressLookup::builder())
        .bind()
        .await
        .map_err(|e| EndpointError::Bind(e.to_string()))
}

/// Console side: accept one incoming connection and run the pairing check against `session`.
///
/// On success the peer's public key is pinned into `trust` and returned. On refusal the peer is told
/// why and the same reason is returned as [`EndpointError::Rejected`].
///
/// # Errors
/// Connection/stream failures, a malformed request, or a rejected code.
pub async fn console_accept_pairing(
    endpoint: &Endpoint,
    session: &mut PairingSession,
    trust: &mut TrustStore,
) -> Result<PairedPeer, EndpointError> {
    let incoming = endpoint.accept().await.ok_or(EndpointError::NoConnection)?;
    let conn = incoming
        .await
        .map_err(|e| EndpointError::Connection(e.to_string()))?;
    let peer_key = *conn.remote_id().as_bytes();
    let (mut send, mut recv) = conn
        .accept_bi()
        .await
        .map_err(|e| EndpointError::Connection(e.to_string()))?;

    let outcome = match read_message::<PairMessage>(&mut recv).await? {
        PairMessage::Request { code } => match PairingCode::from_u32(code) {
            Some(attempt) => session.verify(attempt, now_ms()),
            None => Err(PairRejection::WrongCode), // out-of-range code cannot be valid
        },
        _ => return Err(EndpointError::Protocol),
    };

    let reply = match outcome {
        Ok(()) => PairMessage::Accepted,
        Err(rejection) => PairMessage::Rejected(rejection),
    };
    write_message(&mut send, &reply).await?;
    let _ = send.finish();
    // Keep the connection alive until the agent has read the reply and closed. Dropping it here
    // would send CONNECTION_CLOSE and race the reply's delivery ("connection lost" on the agent).
    let _ = tokio::time::timeout(CLOSE_GRACE, conn.closed()).await;

    match outcome {
        Ok(()) => {
            trust.pin(&peer_key);
            Ok(PairedPeer {
                public_key: peer_key,
                device_id: DeviceId::from_public_key(&peer_key),
            })
        }
        Err(rejection) => Err(EndpointError::Rejected(rejection)),
    }
}

/// Agent side: dial the Console and offer `code`. On acceptance the Console's public key is pinned
/// into `trust` and returned.
///
/// # Errors
/// Connection/stream failures, a malformed reply, or a rejected code.
pub async fn agent_request_pairing(
    endpoint: &Endpoint,
    console: impl Into<EndpointAddr>,
    code: PairingCode,
    trust: &mut TrustStore,
) -> Result<PairedPeer, EndpointError> {
    let conn = endpoint
        .connect(console, PAIRING_ALPN)
        .await
        .map_err(|e| EndpointError::Connect(e.to_string()))?;
    let console_key = *conn.remote_id().as_bytes();
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| EndpointError::Connection(e.to_string()))?;

    write_message(
        &mut send,
        &PairMessage::Request {
            code: code.as_u32(),
        },
    )
    .await?;
    let _ = send.finish();

    let result = match read_message::<PairMessage>(&mut recv).await? {
        PairMessage::Accepted => {
            trust.pin(&console_key);
            Ok(PairedPeer {
                public_key: console_key,
                device_id: DeviceId::from_public_key(&console_key),
            })
        }
        PairMessage::Rejected(rejection) => Err(EndpointError::Rejected(rejection)),
        PairMessage::Request { .. } => Err(EndpointError::Protocol),
    };
    // We initiate the close now that we have the reply; this lets the Console's `closed()` wait
    // resolve and both sides tear down gracefully.
    conn.close(0u32.into(), b"pairing done");
    result
}

/// Writes a length-prefixed postcard message.
pub(crate) async fn write_message<T: Serialize>(
    send: &mut SendStream,
    message: &T,
) -> Result<(), EndpointError> {
    let bytes = proto::encode(message);
    let len = u32::try_from(bytes.len()).map_err(|_| EndpointError::TooLarge(u32::MAX))?;
    let map = |e: iroh::endpoint::WriteError| EndpointError::Stream(e.to_string());
    send.write_all(&len.to_be_bytes()).await.map_err(map)?;
    send.write_all(&bytes).await.map_err(map)?;
    Ok(())
}

/// Reads one length-prefixed postcard message, refusing anything over [`MAX_MESSAGE_BYTES`].
pub(crate) async fn read_message<T: DeserializeOwned>(
    recv: &mut RecvStream,
) -> Result<T, EndpointError> {
    let map = |e: iroh::endpoint::ReadExactError| EndpointError::Stream(e.to_string());
    let mut len_bytes = [0u8; 4];
    recv.read_exact(&mut len_bytes).await.map_err(map)?;
    let len = u32::from_be_bytes(len_bytes);
    if len > MAX_MESSAGE_BYTES {
        return Err(EndpointError::TooLarge(len));
    }
    let mut buf = vec![0u8; len as usize];
    recv.read_exact(&mut buf).await.map_err(map)?;
    proto::decode(&buf).map_err(|_| EndpointError::Protocol)
}
