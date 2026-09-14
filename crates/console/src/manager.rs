//! Keeps one live session per paired device and caches each device's latest screen.
//!
//! The UI never talks to the network directly: it reads [`DeviceView`]s out of here, which are plain
//! data. One background task per device owns its connection, so a slow or offline PC can never block
//! the grid — it just shows as offline while the others keep updating.
//!
//! Watching is **opt-in and stoppable**: no task runs, and therefore no Agent captures anything,
//! until [`DeviceManager::start_watching`] is called (D11: zero capture when nobody is looking).

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use net::{ControlSession, Identity, LocalHello, TrustStore};
use proto::{Capabilities, DeviceId, Role};
use tokio::task::JoinHandle;

/// How often a watched device is asked for a fresh screen.
const REFRESH: Duration = Duration::from_secs(1);
/// How long to wait before retrying a device that failed to connect.
const RETRY: Duration = Duration::from_secs(5);
/// Thumbnail width requested from agents, in pixels.
const THUMB_WIDTH: u16 = 480;

/// What the UI shows for one student PC.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeviceView {
    /// The nine-digit handle a teacher sees.
    pub device_id: String,
    /// The endpoint public key, hex encoded (used to dial).
    pub key: String,
    /// Connection state.
    pub status: DeviceStatus,
    /// Latest screen as a `data:` URL, or `None` if we have not received one yet.
    pub screen: Option<String>,
    /// Why the device is not usable, when `status` says something is wrong.
    pub detail: Option<String>,
}

/// Connection state of one device, in the order the UI colours them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceStatus {
    /// Paired but not being watched (nothing is captured).
    Idle,
    /// Trying to reach the device.
    Connecting,
    /// Connected and receiving screens.
    Live,
    /// Unreachable: powered off, asleep, or off the network.
    Offline,
}

/// One device's mutable state, shared between its task and the UI.
#[derive(Debug)]
struct DeviceState {
    key: [u8; 32],
    status: DeviceStatus,
    screen: Option<String>,
    detail: Option<String>,
}

/// Owns the console identity, the trust store, and the per-device tasks.
pub struct DeviceManager {
    identity: Identity,
    trust: Arc<Mutex<TrustStore>>,
    trust_path: std::path::PathBuf,
    devices: Arc<Mutex<BTreeMap<String, DeviceState>>>,
    tasks: Mutex<BTreeMap<String, JoinHandle<()>>>,
    endpoint: Mutex<Option<iroh::Endpoint>>,
}

impl DeviceManager {
    /// Loads the console's identity and paired devices from `dir`.
    ///
    /// # Errors
    /// Returns a message if the identity or trust store cannot be read.
    pub fn load(dir: &std::path::Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        let identity =
            Identity::load_or_create(&dir.join("device.key")).map_err(|e| e.to_string())?;
        let trust_path = dir.join("trust.bin");
        let trust = TrustStore::load(&trust_path).map_err(|e| e.to_string())?;

        let devices = trust
            .keys()
            .map(|key| {
                let id = DeviceId::from_public_key(key).to_string();
                (
                    id,
                    DeviceState {
                        key: *key,
                        status: DeviceStatus::Idle,
                        screen: None,
                        detail: None,
                    },
                )
            })
            .collect();

        Ok(Self {
            identity,
            trust: Arc::new(Mutex::new(trust)),
            trust_path,
            devices: Arc::new(Mutex::new(devices)),
            tasks: Mutex::new(BTreeMap::new()),
            endpoint: Mutex::new(None),
        })
    }

    /// This console's own nine-digit id.
    #[must_use]
    pub fn device_id(&self) -> String {
        self.identity.device_id().to_string()
    }

    /// This console's endpoint key, which a student PC types to pair.
    #[must_use]
    pub fn public_key(&self) -> String {
        self.identity.public_key().to_string()
    }

    /// A snapshot of every paired device for the UI.
    #[must_use]
    pub fn devices(&self) -> Vec<DeviceView> {
        let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices
            .iter()
            .map(|(id, state)| DeviceView {
                device_id: id.clone(),
                key: hex(&state.key),
                status: state.status,
                screen: state.screen.clone(),
                detail: state.detail.clone(),
            })
            .collect()
    }

    /// Binds the shared endpoint once, reusing it for every device.
    async fn endpoint(&self) -> Result<iroh::Endpoint, String> {
        if let Some(endpoint) = self
            .endpoint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Ok(endpoint);
        }
        let endpoint = net::bind(&self.identity).await.map_err(|e| e.to_string())?;
        *self.endpoint.lock().unwrap_or_else(|e| e.into_inner()) = Some(endpoint.clone());
        Ok(endpoint)
    }

    /// Starts watching every paired device. Idempotent: already-running devices are left alone.
    ///
    /// # Errors
    /// Returns a message if the network endpoint cannot be opened.
    pub async fn start_watching(self: &Arc<Self>) -> Result<(), String> {
        let endpoint = self.endpoint().await?;
        let entries: Vec<(String, [u8; 32])> = {
            let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
            devices.iter().map(|(id, s)| (id.clone(), s.key)).collect()
        };
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        for (id, key) in entries {
            if tasks.get(&id).is_some_and(|t| !t.is_finished()) {
                continue;
            }
            let manager = Arc::clone(self);
            let endpoint = endpoint.clone();
            let task_id = id.clone();
            tasks.insert(
                id,
                tokio::spawn(async move { manager.watch_device(endpoint, task_id, key).await }),
            );
        }
        Ok(())
    }

    /// Stops all watching. Agents go back to capturing nothing.
    pub fn stop_watching(&self) {
        let mut tasks = self.tasks.lock().unwrap_or_else(|e| e.into_inner());
        for (_, task) in std::mem::take(&mut *tasks) {
            task.abort();
        }
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        for state in devices.values_mut() {
            state.status = DeviceStatus::Idle;
            state.detail = None;
        }
    }

    /// Connects to one device and refreshes its screen until the task is aborted.
    async fn watch_device(&self, endpoint: iroh::Endpoint, id: String, key: [u8; 32]) {
        loop {
            self.set_status(&id, DeviceStatus::Connecting, None);
            match self.pull_screens(&endpoint, &id, key).await {
                Ok(()) => self.set_status(&id, DeviceStatus::Offline, Some("disconnected".into())),
                Err(err) => self.set_status(&id, DeviceStatus::Offline, Some(err)),
            }
            tokio::time::sleep(RETRY).await;
        }
    }

    /// One connection's lifetime: request a screen every [`REFRESH`] until it fails.
    async fn pull_screens(
        &self,
        endpoint: &iroh::Endpoint,
        id: &str,
        key: [u8; 32],
    ) -> Result<(), String> {
        let trust = self.trust.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let local = LocalHello {
            role: Role::Console,
            device_id: self.identity.device_id(),
            capabilities: Capabilities::EMPTY,
        };
        let agent = iroh::EndpointId::from_bytes(&key).map_err(|e| e.to_string())?;
        let mut session =
            ControlSession::connect(endpoint, iroh::EndpointAddr::new(agent), &trust, local)
                .await
                .map_err(|e| e.to_string())?;

        loop {
            let jpeg = session
                .request_thumbnail(0, THUMB_WIDTH)
                .await
                .map_err(|e| e.to_string())?;
            self.set_screen(id, &jpeg);
            tokio::time::sleep(REFRESH).await;
        }
    }

    fn set_status(&self, id: &str, status: DeviceStatus, detail: Option<String>) {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = devices.get_mut(id) {
            state.status = status;
            state.detail = detail;
        }
    }

    fn set_screen(&self, id: &str, jpeg: &[u8]) {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = devices.get_mut(id) {
            state.status = DeviceStatus::Live;
            state.detail = None;
            state.screen = Some(format!("data:image/jpeg;base64,{}", base64(jpeg)));
        }
    }

    /// Shows a pairing code and enrols the one device that dials in with it.
    ///
    /// # Errors
    /// Returns a message if the endpoint fails or the device is refused.
    pub async fn pair_once(&self, code: net::PairingCode) -> Result<String, String> {
        let endpoint = self.endpoint().await?;
        endpoint.online().await;
        let mut session = net::PairingSession::new(code, net::endpoint::now_ms());
        let mut trust = self.trust.lock().unwrap_or_else(|e| e.into_inner()).clone();

        let peer = net::console_accept_pairing(&endpoint, &mut session, &mut trust)
            .await
            .map_err(|e| e.to_string())?;
        trust.save(&self.trust_path).map_err(|e| e.to_string())?;

        let id = peer.device_id.to_string();
        *self.trust.lock().unwrap_or_else(|e| e.into_inner()) = trust;
        self.devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(
                id.clone(),
                DeviceState {
                    key: peer.public_key,
                    status: DeviceStatus::Idle,
                    screen: None,
                    detail: None,
                },
            );
        Ok(id)
    }
}

/// Lowercase hex, for showing and re-parsing keys.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Minimal base64 (standard alphabet, padded) so screens can go straight into an `<img src>`.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let indices = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (position, index) in indices.iter().enumerate() {
            if position <= chunk.len() {
                out.push(ALPHABET[*index as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_high_bytes() {
        // JPEG starts with 0xFF 0xD8 0xFF: make sure the non-ASCII path is right.
        assert_eq!(base64(&[0xFF, 0xD8, 0xFF]), "/9j/");
    }

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex(&[0x00, 0x0f, 0xff]), "000fff");
    }
}
