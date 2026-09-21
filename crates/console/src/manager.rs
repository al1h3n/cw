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

/// How often a watched device is asked for a fresh screen in the grid.
const REFRESH: Duration = Duration::from_secs(1);
/// How often the screen a teacher has opened is refreshed: smooth enough to follow what is happening.
const FOCUSED_REFRESH: Duration = Duration::from_millis(250);
/// How long to wait before retrying a device that failed to connect.
const RETRY: Duration = Duration::from_secs(5);
/// Preview width used until the teacher picks one, in pixels.
pub const DEFAULT_GRID_WIDTH: u16 = 480;
/// Preview width for the screen a teacher has opened.
pub const DEFAULT_FOCUSED_WIDTH: u16 = 1280;

/// What the UI shows for one student PC.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct DeviceView {
    /// The six-character handle a teacher sees.
    pub device_id: String,
    /// The teacher's own name for this PC, if they set one. Shown above the id.
    pub name: Option<String>,
    /// The endpoint public key, hex encoded (used to dial).
    pub key: String,
    /// Connection state.
    pub status: DeviceStatus,
    /// Latest screen as a `data:` URL, or `None` if we have not received one yet.
    pub screen: Option<String>,
    /// Why the device is not usable, when `status` says something is wrong.
    pub detail: Option<String>,
    /// The monitors this PC has, once it has told us.
    pub monitors: Vec<proto::Monitor>,
    /// Which monitor is being shown.
    pub monitor: u8,
    /// This PC's MAC addresses, learned while it was connected, for Wake-on-LAN when it is off.
    pub macs: Vec<String>,
    /// What happened to the last action sent to this PC, for the UI to show.
    pub last_action: Option<ActionReport>,
}

/// The answer to one action, in codes the UI translates (D17: Rust sends codes, not text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ActionReport {
    /// The stable action name, e.g. `shutdown` (see [`proto::Action::name`]).
    pub action: &'static str,
    /// `started`, or why not: `notSupported`, `notPermitted`, `nothingScheduled`, `failed`.
    pub result: &'static str,
    /// Countdown the PC actually started, after its own clamping.
    pub delay_seconds: u16,
    /// When the answer arrived, so the UI can fade old reports.
    pub at_ms: u64,
}

impl ActionReport {
    fn new(action: proto::Action, outcome: proto::ActionOutcome) -> Self {
        use proto::{ActionFailure, ActionOutcome};
        let (result, delay_seconds) = match outcome {
            ActionOutcome::Started { delay_seconds } => ("started", delay_seconds),
            ActionOutcome::Failed(ActionFailure::NotSupported) => ("notSupported", 0),
            ActionOutcome::Failed(ActionFailure::NotPermitted) => ("notPermitted", 0),
            ActionOutcome::Failed(ActionFailure::NothingScheduled) => ("nothingScheduled", 0),
            ActionOutcome::Failed(ActionFailure::Failed) => ("failed", 0),
        };
        Self {
            action: action.name(),
            result,
            delay_seconds,
            at_ms: net::endpoint::now_ms(),
        }
    }
}

/// Turns the UI's action name back into a typed action. Unknown names are refused, never guessed.
#[must_use]
pub fn parse_action(name: &str, delay_seconds: u16) -> Option<proto::Action> {
    use proto::Action;
    [
        Action::Shutdown { delay_seconds },
        Action::Reboot { delay_seconds },
        Action::LogOff,
        Action::LockScreen,
        Action::CancelShutdown,
        Action::LockWallpaper,
        Action::UnlockWallpaper,
    ]
    .into_iter()
    .find(|action| action.name() == name)
}

/// Something the UI wants from one device, carried to that device's own task.
///
/// Every device's connection is owned by its own task, so the UI cannot simply call the network.
/// It parks a request here with a one-shot reply channel; the task picks it up on its next turn and
/// answers. The same pattern serves recordings and the app launcher, and it keeps a slow or offline
/// PC from ever blocking the window.
enum DeviceRequest {
    /// Start recording, with the codec/size/rate the teacher chose.
    StartRecording {
        monitor: u8,
        options: proto::RecordOptions,
        reply: tokio::sync::oneshot::Sender<proto::RecordingInfo>,
    },
    /// Stop the recording and report the final state.
    StopRecording {
        reply: tokio::sync::oneshot::Sender<proto::RecordingInfo>,
    },
    /// How the recording is going.
    RecordingStatus {
        reply: tokio::sync::oneshot::Sender<proto::RecordingInfo>,
    },
    /// What recordings are stored on this PC.
    ListRecordings {
        reply: tokio::sync::oneshot::Sender<Vec<proto::StoredRecording>>,
    },
    /// Download one recording to the teacher's PC; the reply is the saved path or an error.
    FetchRecording {
        file: String,
        reply: tokio::sync::oneshot::Sender<Result<String, String>>,
    },
    /// What this PC can start.
    ListApps {
        reply: tokio::sync::oneshot::Sender<Vec<proto::AppEntry>>,
    },
    /// One program's icon (lazy), as `(width, height, top-down BGRA)` or `None`.
    AppIcon {
        id: u32,
        reply: tokio::sync::oneshot::Sender<Option<(u16, u16, Vec<u8>)>>,
    },
    /// Start one published program.
    LaunchApp {
        id: u32,
        reply: tokio::sync::oneshot::Sender<(String, bool)>,
    },
    /// What is running and closable.
    ListRunning {
        reply: tokio::sync::oneshot::Sender<Vec<proto::RunningApp>>,
    },
    /// Close a running program.
    CloseApp {
        pid: u32,
        reply: tokio::sync::oneshot::Sender<bool>,
    },
    /// Start or end exam lockdown; the reply is `(locked, problem)`.
    SetExam {
        on: bool,
        message: String,
        reply: tokio::sync::oneshot::Sender<(bool, String)>,
    },
    /// Show one broadcast frame on this PC; the reply is `(showing, problem)`.
    ShowBroadcast {
        jpeg: Vec<u8>,
        locked: bool,
        reply: tokio::sync::oneshot::Sender<(bool, String)>,
    },
    /// Take the broadcast off this PC; the reply is `(showing, problem)`.
    StopBroadcast {
        reply: tokio::sync::oneshot::Sender<(bool, String)>,
    },
    /// Set this PC's desktop wallpaper to the given image; the reply is `(ok, problem)`.
    SetWallpaper {
        image: Vec<u8>,
        reply: tokio::sync::oneshot::Sender<(bool, String)>,
    },
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
///
/// Not `Debug`: it holds one-shot reply channels, which have nothing useful to print.
struct DeviceState {
    key: [u8; 32],
    /// The teacher's own name for this PC (e.g. "Row 3, seat 2"), shown above the id. Persisted.
    name: Option<String>,
    status: DeviceStatus,
    screen: Option<String>,
    detail: Option<String>,
    monitors: Vec<proto::Monitor>,
    monitor: u8,
    /// MAC addresses reported while connected; kept across a drop so an offline PC can be woken.
    macs: Vec<String>,
    /// Actions the teacher asked for that the device's task has not sent yet.
    pending: Vec<proto::Action>,
    /// Input events waiting to be sent while this PC is being controlled.
    pending_input: Vec<proto::InputEvent>,
    /// Requests from the UI waiting for this device's task to answer them.
    requests: Vec<DeviceRequest>,
    last_action: Option<ActionReport>,
}

impl DeviceState {
    /// A freshly paired or freshly loaded device: known, but not being watched.
    fn new(key: [u8; 32]) -> Self {
        Self {
            key,
            name: None,
            status: DeviceStatus::Idle,
            screen: None,
            detail: None,
            monitors: Vec::new(),
            monitor: 0,
            macs: Vec::new(),
            pending: Vec::new(),
            pending_input: Vec::new(),
            requests: Vec::new(),
            last_action: None,
        }
    }
}

/// How much audio to pull per request: a fifth of a second, so the sound keeps up without
/// large replies.
const AUDIO_CHUNK_MS: u32 = 200;
/// Never let more than this much audio pile up on the teacher's PC; late sound is useless.
const AUDIO_QUEUE_MS: u32 = 600;

/// What the teacher has chosen about how screens are shown.
#[derive(Debug, Clone, Copy)]
struct Preview {
    /// Width requested for the tiles in the grid.
    grid_width: u16,
    /// Width requested for the one screen a teacher has opened.
    focused_width: u16,
    /// The device currently opened full-size, which is refreshed faster and larger.
    focused: Option<[u8; 32]>,
    /// The one device being listened to. Listening to a whole room at once would be unusable noise
    /// and heavy on the network, so it is deliberately exclusive.
    listening: Option<[u8; 32]>,
    /// The one device being driven. Exclusive for the same reason a mouse has one pointer.
    controlling: Option<[u8; 32]>,
}

impl Default for Preview {
    fn default() -> Self {
        Self {
            grid_width: DEFAULT_GRID_WIDTH,
            focused_width: DEFAULT_FOCUSED_WIDTH,
            focused: None,
            listening: None,
            controlling: None,
        }
    }
}

/// Owns the console identity, the trust store, and the per-device tasks.
pub struct DeviceManager {
    identity: Identity,
    trust: Arc<Mutex<TrustStore>>,
    trust_path: std::path::PathBuf,
    devices: Arc<Mutex<BTreeMap<String, DeviceState>>>,
    tasks: Mutex<BTreeMap<String, JoinHandle<()>>>,
    endpoint: Mutex<Option<iroh::Endpoint>>,
    preview: Arc<Mutex<Preview>>,
    /// Open only while listening to some PC; recreated when the agent's sample rate is known.
    playback: Arc<Mutex<Option<media::audio::AudioPlayback>>>,
    /// The room-wide blocklist and a version that bumps on every edit, so each device's task knows
    /// to re-send it. Kept here (not per device) because "no games" applies to the whole class.
    blocklist: Arc<Mutex<Blocklist>>,
    blocklist_path: std::path::PathBuf,
    /// The room every invited device joins, and whose password they need to leave.
    room: Arc<Mutex<crate::room::Room>>,
    /// Where the room file lives, for renames and password changes.
    data_dir: std::path::PathBuf,
}

/// The list of blocked programs plus a version counter.
#[derive(Default)]
struct Blocklist {
    programs: Vec<String>,
    version: u64,
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
        let room = crate::room::load_or_create(dir)?;
        let blocklist_path = dir.join("blocklist.txt");
        let programs = std::fs::read_to_string(&blocklist_path)
            .map(|t| {
                t.lines()
                    .map(str::to_string)
                    .filter(|l| !l.trim().is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let names = load_names(&dir.join("names.txt"));
        let devices = trust
            .keys()
            .map(|key| {
                let id = DeviceId::from_public_key(key).to_string();
                let mut state = DeviceState::new(*key);
                state.name = names.get(&id).cloned();
                (id, state)
            })
            .collect();

        Ok(Self {
            identity,
            trust: Arc::new(Mutex::new(trust)),
            trust_path,
            devices: Arc::new(Mutex::new(devices)),
            tasks: Mutex::new(BTreeMap::new()),
            endpoint: Mutex::new(None),
            preview: Arc::new(Mutex::new(Preview::default())),
            playback: Arc::new(Mutex::new(None)),
            blocklist: Arc::new(Mutex::new(Blocklist {
                programs,
                version: 1,
            })),
            blocklist_path,
            room: Arc::new(Mutex::new(room)),
            data_dir: dir.to_path_buf(),
        })
    }

    /// This console's own six-character id.
    #[must_use]
    pub fn device_id(&self) -> String {
        self.identity.device_id().to_string()
    }

    /// This console's endpoint key, which a student PC types to pair.
    #[must_use]
    pub fn public_key(&self) -> String {
        self.identity.public_key().to_string()
    }

    /// One device's endpoint key in the canonical form the viewer parses, resolved from the stored
    /// public key so it always round-trips (never a hand-formatted hex string).
    ///
    /// # Errors
    /// The device is unknown, or its key is not a valid endpoint id.
    pub fn endpoint_key(&self, device_id: &str) -> Result<String, String> {
        let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        let state = devices.get(device_id).ok_or("unknown device")?;
        iroh::EndpointId::from_bytes(&state.key)
            .map(|id| id.to_string())
            .map_err(|e| e.to_string())
    }

    /// Sets (or clears, with an empty name) the teacher's custom name for one PC, and saves it.
    ///
    /// # Errors
    /// The device is unknown, or the names file cannot be written.
    pub fn rename(&self, device_id: &str, name: &str) -> Result<(), String> {
        let trimmed = name.trim();
        {
            let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
            let state = devices.get_mut(device_id).ok_or("unknown device")?;
            state.name = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        self.save_names()
    }

    /// Writes every custom name to `names.txt`, one `id = name` per line.
    fn save_names(&self) -> Result<(), String> {
        let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        let mut out = String::new();
        for (id, state) in devices.iter() {
            if let Some(name) = &state.name {
                out.push_str(&format!("{id} = {}\n", name.replace('\n', " ")));
            }
        }
        drop(devices);
        std::fs::write(self.data_dir.join("names.txt"), out).map_err(|e| e.to_string())
    }

    /// A snapshot of every paired device for the UI.
    #[must_use]
    pub fn devices(&self) -> Vec<DeviceView> {
        let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices
            .iter()
            .map(|(id, state)| DeviceView {
                device_id: id.clone(),
                name: state.name.clone(),
                key: hex(&state.key),
                status: state.status,
                screen: state.screen.clone(),
                detail: state.detail.clone(),
                monitors: state.monitors.clone(),
                monitor: state.monitor,
                macs: state.macs.clone(),
                last_action: state.last_action,
            })
            .collect()
    }

    /// The preview widths currently in use, as `(grid, focused)`.
    #[must_use]
    pub fn preview_widths(&self) -> (u16, u16) {
        let preview = self.preview.lock().unwrap_or_else(|e| e.into_inner());
        (preview.grid_width, preview.focused_width)
    }

    /// Sets how wide the captured images should be. Bigger is sharper and costs more bandwidth.
    ///
    /// Values are clamped to something sane so a typo cannot ask for a 1-pixel or 20000-pixel image.
    pub fn set_preview_widths(&self, grid: u16, focused: u16) {
        let mut preview = self.preview.lock().unwrap_or_else(|e| e.into_inner());
        preview.grid_width = grid.clamp(160, 3840);
        preview.focused_width = focused.clamp(320, 3840);
    }

    /// Marks one device as the opened screen, which refreshes faster and at the focused width.
    /// Passing `None` returns every device to grid pace.
    pub fn set_focused(&self, device_id: Option<&str>) {
        let key = device_id.and_then(|id| {
            let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
            devices.get(id).map(|state| state.key)
        });
        self.preview
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .focused = key;
    }

    /// Starts listening to one PC, or stops listening entirely with `None`.
    ///
    /// Only one PC is listened to at a time: switching moves the ear, it does not add a second one.
    ///
    /// # Errors
    /// Returns a message if the device is unknown.
    pub fn set_listening(&self, device_id: Option<&str>) -> Result<(), String> {
        let key = match device_id {
            Some(id) => {
                let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
                Some(
                    devices
                        .get(id)
                        .ok_or_else(|| "unknown device".to_string())?
                        .key,
                )
            }
            None => None,
        };
        let mut preview = self.preview.lock().unwrap_or_else(|e| e.into_inner());
        if preview.listening != key {
            preview.listening = key;
            // Drop playback now; the device's own task opens a fresh one at the agent's sample rate.
            *self.playback.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        Ok(())
    }

    /// The device currently being listened to, if any.
    #[must_use]
    pub fn listening(&self) -> Option<String> {
        let key = self
            .preview
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .listening?;
        Some(DeviceId::from_public_key(&key).to_string())
    }

    /// Chooses which monitor of a multi-monitor PC to show.
    ///
    /// # Errors
    /// Returns a message if the device is unknown or does not have that monitor.
    pub fn set_monitor(&self, device_id: &str, monitor: u8) -> Result<(), String> {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        let state = devices
            .get_mut(device_id)
            .ok_or_else(|| "unknown device".to_string())?;
        if !state.monitors.is_empty() && !state.monitors.iter().any(|m| m.index == monitor) {
            return Err(format!("that PC has no monitor {monitor}"));
        }
        state.monitor = monitor;
        // Drop the old screen so the tile does not show the previous monitor while the new one loads.
        state.screen = None;
        Ok(())
    }

    /// Queues an action for PCs: one by id, or every connected PC when `device_id` is `None`.
    ///
    /// Only connected PCs are acted on. Queuing a shutdown for an offline PC would fire it the next
    /// morning when someone switches that PC on, which nobody wants. Returns how many PCs it went to.
    ///
    /// # Errors
    /// Returns a message if the named PC is unknown or not connected.
    pub fn perform(&self, device_id: Option<&str>, action: proto::Action) -> Result<usize, String> {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(id) = device_id {
            let state = devices
                .get_mut(id)
                .ok_or_else(|| "unknown device".to_string())?;
            if state.status != DeviceStatus::Live {
                return Err("that PC is not connected".into());
            }
            state.pending.push(action);
            return Ok(1);
        }
        let mut sent = 0;
        for state in devices.values_mut() {
            if state.status == DeviceStatus::Live {
                state.pending.push(action);
                sent += 1;
            }
        }
        Ok(sent)
    }

    /// Takes control of one PC's mouse and keyboard, or gives it back.
    ///
    /// Control is exclusive, like listening: driving two PCs at once with one mouse is meaningless,
    /// and it makes "which PC am I typing into?" impossible for a teacher to answer.
    ///
    /// # Errors
    /// Returns a message if the device is unknown or not connected.
    pub fn set_controlling(&self, device_id: Option<&str>) -> Result<(), String> {
        let key = match device_id {
            Some(id) => {
                let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
                let state = devices
                    .get(id)
                    .ok_or_else(|| "unknown device".to_string())?;
                if state.status != DeviceStatus::Live {
                    return Err("that PC is not connected".into());
                }
                Some(state.key)
            }
            None => None,
        };
        let mut preview = self.preview.lock().unwrap_or_else(|e| e.into_inner());
        preview.controlling = key;
        Ok(())
    }

    /// Which PC the teacher is currently driving, if any.
    #[must_use]
    pub fn controlling(&self) -> Option<String> {
        let key = self
            .preview
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .controlling?;
        Some(DeviceId::from_public_key(&key).to_string())
    }

    /// Queues input events for the PC currently being controlled.
    ///
    /// # Errors
    /// Returns a message if no PC is being controlled, or that PC is unknown.
    pub fn queue_input(&self, events: Vec<proto::InputEvent>) -> Result<(), String> {
        let key = self
            .preview
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .controlling
            .ok_or_else(|| "no PC is being controlled".to_string())?;
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        let state = devices
            .values_mut()
            .find(|s| s.key == key)
            .ok_or_else(|| "unknown device".to_string())?;
        // Drop the oldest rather than grow without bound: stale pointer positions are worthless,
        // and a teacher would rather the pointer jump to "now" than replay a backlog.
        state.pending_input.extend(events);
        let overflow = state
            .pending_input
            .len()
            .saturating_sub(proto::MAX_INPUT_BATCH);
        if overflow > 0 {
            state.pending_input.drain(0..overflow);
        }
        Ok(())
    }

    /// Takes the queued input for one device.
    fn take_input(&self, id: &str) -> Vec<proto::InputEvent> {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices
            .get_mut(id)
            .map(|state| std::mem::take(&mut state.pending_input))
            .unwrap_or_default()
    }

    /// Parks a request for one device and waits for its task to answer.
    ///
    /// # Errors
    /// Returns a message if the PC is unknown, not connected, or drops before answering.
    async fn ask<T>(
        &self,
        device_id: &str,
        make: impl FnOnce(tokio::sync::oneshot::Sender<T>) -> DeviceRequest,
    ) -> Result<T, String> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        {
            let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
            let state = devices
                .get_mut(device_id)
                .ok_or_else(|| "unknown device".to_string())?;
            if state.status != DeviceStatus::Live {
                return Err("that PC is not connected".into());
            }
            state.requests.push(make(reply));
        }
        answer
            .await
            .map_err(|_| "that PC stopped responding".to_string())
    }

    /// Starts recording on one PC, returning what it is actually recording after clamping.
    ///
    /// # Errors
    /// See [`DeviceManager::ask`].
    pub async fn start_recording(
        &self,
        device_id: &str,
        options: proto::RecordOptions,
    ) -> Result<proto::RecordingInfo, String> {
        let monitor = self
            .devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(device_id)
            .map_or(0, |s| s.monitor);
        self.ask(device_id, |reply| DeviceRequest::StartRecording {
            monitor,
            options,
            reply,
        })
        .await
    }

    /// Stops the recording on one PC.
    ///
    /// # Errors
    /// See [`DeviceManager::ask`].
    pub async fn stop_recording(&self, device_id: &str) -> Result<proto::RecordingInfo, String> {
        self.ask(device_id, |reply| DeviceRequest::StopRecording { reply })
            .await
    }

    /// How the recording on one PC is going.
    ///
    /// # Errors
    /// See [`DeviceManager::ask`].
    /// Starts or ends exam lockdown on one PC. Returns `(locked, problem)`.
    ///
    /// # Errors
    /// The PC is unknown or not connected.
    pub async fn set_exam(
        &self,
        device_id: &str,
        on: bool,
        message: &str,
    ) -> Result<(bool, String), String> {
        let message = message.to_string();
        self.ask(device_id, move |reply| DeviceRequest::SetExam {
            on,
            message,
            reply,
        })
        .await
    }

    /// Sets one PC's desktop wallpaper to `image` (raw PNG/JPEG/BMP bytes). Returns `(ok, problem)`.
    ///
    /// # Errors
    /// The PC is unknown or not connected.
    pub async fn set_wallpaper(
        &self,
        device_id: &str,
        image: Vec<u8>,
    ) -> Result<(bool, String), String> {
        self.ask(device_id, move |reply| DeviceRequest::SetWallpaper {
            image,
            reply,
        })
        .await
    }

    /// Lists the recordings stored on one PC.
    ///
    /// # Errors
    /// The PC is unknown or not connected.
    pub async fn list_recordings(
        &self,
        device_id: &str,
    ) -> Result<Vec<proto::StoredRecording>, String> {
        self.ask(device_id, |reply| DeviceRequest::ListRecordings { reply })
            .await
    }

    /// Downloads one recording from a PC to this teacher's `recordings` folder, returning the saved
    /// path.
    ///
    /// # Errors
    /// The PC is unknown, not connected, or the file could not be transferred.
    pub async fn download_recording(&self, device_id: &str, file: &str) -> Result<String, String> {
        let file = file.to_string();
        self.ask(device_id, move |reply| DeviceRequest::FetchRecording {
            file,
            reply,
        })
        .await?
    }

    pub async fn recording_status(&self, device_id: &str) -> Result<proto::RecordingInfo, String> {
        self.ask(device_id, |reply| DeviceRequest::RecordingStatus { reply })
            .await
    }

    /// The programs one PC offers to start.
    ///
    /// # Errors
    /// See [`DeviceManager::ask`].
    pub async fn list_apps(&self, device_id: &str) -> Result<Vec<proto::AppEntry>, String> {
        self.ask(device_id, |reply| DeviceRequest::ListApps { reply })
            .await
    }

    /// One program's icon, as `(width, height, top-down BGRA)` or `None` if the PC has none.
    ///
    /// # Errors
    /// See [`DeviceManager::ask`].
    pub async fn app_icon(
        &self,
        device_id: &str,
        id: u32,
    ) -> Result<Option<(u16, u16, Vec<u8>)>, String> {
        self.ask(device_id, |reply| DeviceRequest::AppIcon { id, reply })
            .await
    }

    /// Starts one published program on a PC.
    ///
    /// # Errors
    /// See [`DeviceManager::ask`].
    pub async fn launch_app(&self, device_id: &str, id: u32) -> Result<(String, bool), String> {
        self.ask(device_id, |reply| DeviceRequest::LaunchApp { id, reply })
            .await
    }

    /// What is running and closable on a PC.
    ///
    /// # Errors
    /// See [`DeviceManager::ask`].
    pub async fn list_running(&self, device_id: &str) -> Result<Vec<proto::RunningApp>, String> {
        self.ask(device_id, |reply| DeviceRequest::ListRunning { reply })
            .await
    }

    /// Closes a running program on a PC.
    ///
    /// # Errors
    /// See [`DeviceManager::ask`].
    pub async fn close_app(&self, device_id: &str, pid: u32) -> Result<bool, String> {
        self.ask(device_id, |reply| DeviceRequest::CloseApp { pid, reply })
            .await
    }

    /// Shows one broadcast frame on a PC. `(showing, problem)`.
    ///
    /// # Errors
    /// See [`DeviceManager::ask`].
    pub async fn show_broadcast(
        &self,
        device_id: &str,
        jpeg: Vec<u8>,
        locked: bool,
    ) -> Result<(bool, String), String> {
        self.ask(device_id, move |reply| DeviceRequest::ShowBroadcast {
            jpeg,
            locked,
            reply,
        })
        .await
    }

    /// Takes the broadcast off a PC. `(showing, problem)`.
    ///
    /// # Errors
    /// See [`DeviceManager::ask`].
    pub async fn stop_broadcast(&self, device_id: &str) -> Result<(bool, String), String> {
        self.ask(device_id, |reply| DeviceRequest::StopBroadcast { reply })
            .await
    }

    /// Takes the queued UI requests for one device.
    fn take_requests(&self, id: &str) -> Vec<DeviceRequest> {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices
            .get_mut(id)
            .map(|state| std::mem::take(&mut state.requests))
            .unwrap_or_default()
    }

    /// Takes the queued actions for one device, leaving its queue empty.
    fn take_pending(&self, id: &str) -> Vec<proto::Action> {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices
            .get_mut(id)
            .map(|state| std::mem::take(&mut state.pending))
            .unwrap_or_default()
    }

    fn set_last_action(&self, id: &str, report: ActionReport) {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = devices.get_mut(id) {
            state.last_action = Some(report);
        }
    }

    /// The room's name and its password, for the teacher to see and write down.
    #[must_use]
    pub fn room(&self) -> (String, String) {
        let room = self.room.lock().unwrap_or_else(|e| e.into_inner());
        (room.name.clone(), room.password_grouped())
    }

    /// The welcome handed to a device as it joins: room name plus the password hash.
    ///
    /// # Errors
    /// Returns a message if the password cannot be hashed.
    pub fn welcome(&self) -> Result<proto::Welcome, String> {
        let room = self.room.lock().unwrap_or_else(|e| e.into_inner());
        room.welcome()
    }

    /// Renames the room. Devices already in it keep working; they learn the new name when re-invited.
    ///
    /// # Errors
    /// Returns a message if the name is unusable or cannot be saved.
    pub fn rename_room(&self, name: &str) -> Result<(), String> {
        let mut room = self.room.lock().unwrap_or_else(|e| e.into_inner());
        *room = crate::room::rename(&self.data_dir, &room, name)?;
        Ok(())
    }

    /// Issues a brand-new room password.
    ///
    /// # Errors
    /// Returns a message if it cannot be saved.
    pub fn new_room_password(&self) -> Result<(), String> {
        let mut room = self.room.lock().unwrap_or_else(|e| e.into_inner());
        *room = crate::room::regenerate_password(&self.data_dir, &room)?;
        Ok(())
    }

    /// The room-wide blocklist as the teacher sees it.
    #[must_use]
    pub fn blocklist(&self) -> Vec<String> {
        self.blocklist
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .programs
            .clone()
    }

    /// Replaces the room-wide blocklist and saves it. Connected PCs pick it up within a second or
    /// two; a PC that connects later gets it on its first handshake.
    ///
    /// # Errors
    /// Returns a message if the list cannot be saved to disk.
    pub fn set_blocklist(&self, programs: Vec<String>) -> Result<(), String> {
        let programs: Vec<String> = programs
            .into_iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();
        std::fs::write(&self.blocklist_path, programs.join("\n")).map_err(|e| e.to_string())?;
        let mut list = self.blocklist.lock().unwrap_or_else(|e| e.into_inner());
        list.programs = programs;
        list.version += 1;
        Ok(())
    }

    /// Wakes a paired PC that is switched off, by broadcasting a magic packet for every MAC we
    /// learned while it was last connected.
    ///
    /// Broadcasts from this Console directly, which reaches any PC on the same LAN — the classroom
    /// case. Requires that the PC connected at least once (so we know its MAC) and that its BIOS and
    /// network card have Wake-on-LAN enabled, which is a one-time setting the school's IT makes.
    ///
    /// # Errors
    /// Returns a message if the device is unknown, was never seen online, or no packet could be sent.
    pub fn wake(&self, device_id: &str) -> Result<usize, String> {
        let macs = {
            let devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
            devices
                .get(device_id)
                .ok_or_else(|| "unknown device".to_string())?
                .macs
                .clone()
        };
        if macs.is_empty() {
            return Err("this PC has never been online, so its MAC address is unknown".into());
        }
        let mut woken = 0;
        for mac in &macs {
            if let Ok(mac) = platform::wol::MacAddress::parse(mac)
                && platform::wol::wake(mac).is_ok()
            {
                woken += 1;
            }
        }
        if woken == 0 {
            return Err("could not send a wake packet".into());
        }
        Ok(woken)
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

        // Ask once per connection which monitors this PC has, so the teacher can pick one.
        let monitors = session
            .request_monitors()
            .await
            .map_err(|e| e.to_string())?;
        self.set_monitors(id, monitors);

        // Learn its MAC addresses too, so it can be woken by Wake-on-LAN once it is switched off.
        if let Ok(macs) = session.request_macs().await {
            let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(state) = devices.get_mut(id) {
                state.macs = macs;
            }
        }

        // Audio is per-connection state on the agent, so this tracks what we have switched on here.
        let mut audio_on: Option<proto::AudioFormat> = None;
        // Send the blocklist whenever its version moves; 0 forces a send on the first pass.
        let mut sent_blocklist: u64 = 0;
        // Whether this connection has been granted control of the PC.
        let mut controlling = false;

        loop {
            // The wallpaper is blacked out by the Agent only during a *full* live preview (the native
            // viewer's H.264 stream), not for the grid or its focused thumbnail view — a teacher
            // glancing at low-fps thumbnails should still see the real desktop, and it costs almost
            // nothing to send. So the Console no longer drives set_watched from the focused state.
            let (programs, version) = {
                let list = self.blocklist.lock().unwrap_or_else(|e| e.into_inner());
                (list.programs.clone(), list.version)
            };
            if version != sent_blocklist {
                session
                    .set_blocklist(programs)
                    .await
                    .map_err(|e| e.to_string())?;
                sent_blocklist = version;
            }

            // Control and input come first: a click must not wait behind a screen refresh.
            let wants_control = self
                .preview
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .controlling
                == Some(key);
            if wants_control != controlling {
                controlling = session
                    .set_control(wants_control)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            if controlling {
                let events = self.take_input(id);
                if !events.is_empty() {
                    session
                        .send_input(events)
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }

            // Answer whatever the window asked for. A dropped receiver (the teacher closed the
            // dialog) is not an error: the send simply fails and we move on.
            for request in self.take_requests(id) {
                match request {
                    DeviceRequest::StartRecording {
                        monitor,
                        options,
                        reply,
                    } => {
                        let info = session
                            .start_recording(monitor, options)
                            .await
                            .map_err(|e| e.to_string())?;
                        let _ = reply.send(info);
                    }
                    DeviceRequest::StopRecording { reply } => {
                        let info = session.stop_recording().await.map_err(|e| e.to_string())?;
                        let _ = reply.send(info);
                    }
                    DeviceRequest::RecordingStatus { reply } => {
                        let info = session
                            .recording_status()
                            .await
                            .map_err(|e| e.to_string())?;
                        let _ = reply.send(info);
                    }
                    DeviceRequest::ListRecordings { reply } => {
                        let list = session.list_recordings().await.map_err(|e| e.to_string())?;
                        let _ = reply.send(list);
                    }
                    DeviceRequest::FetchRecording { file, reply } => {
                        let dest = self.data_dir.join("recordings");
                        let result = session
                            .fetch_recording(&file, &dest)
                            .await
                            .map(|p| p.display().to_string())
                            .map_err(|e| e.to_string());
                        let _ = reply.send(result);
                    }
                    DeviceRequest::ListApps { reply } => {
                        let apps = session.request_apps().await.map_err(|e| e.to_string())?;
                        let _ = reply.send(apps);
                    }
                    DeviceRequest::AppIcon { id, reply } => {
                        let icon = session
                            .request_app_icon(id)
                            .await
                            .map_err(|e| e.to_string())?;
                        let _ = reply.send(icon);
                    }
                    DeviceRequest::LaunchApp { id, reply } => {
                        let result = session.launch_app(id).await.map_err(|e| e.to_string())?;
                        let _ = reply.send(result);
                    }
                    DeviceRequest::ListRunning { reply } => {
                        let running = session.request_running().await.map_err(|e| e.to_string())?;
                        let _ = reply.send(running);
                    }
                    DeviceRequest::CloseApp { pid, reply } => {
                        let closed = session.close_app(pid).await.map_err(|e| e.to_string())?;
                        let _ = reply.send(closed);
                    }
                    DeviceRequest::SetExam { on, message, reply } => {
                        let state = session
                            .set_exam(on, message)
                            .await
                            .map_err(|e| e.to_string())?;
                        let _ = reply.send(state);
                    }
                    DeviceRequest::ShowBroadcast {
                        jpeg,
                        locked,
                        reply,
                    } => {
                        let state = session
                            .show_broadcast(jpeg, locked)
                            .await
                            .map_err(|e| e.to_string())?;
                        let _ = reply.send(state);
                    }
                    DeviceRequest::StopBroadcast { reply } => {
                        let state = session.stop_broadcast().await.map_err(|e| e.to_string())?;
                        let _ = reply.send(state);
                    }
                    DeviceRequest::SetWallpaper { image, reply } => {
                        let state = session
                            .set_wallpaper(image)
                            .await
                            .map_err(|e| e.to_string())?;
                        let _ = reply.send(state);
                    }
                }
            }

            // Actions first: a teacher's click should not wait behind a screen refresh.
            // ponytail: picked up on the next loop turn, so up to one refresh interval (1 s) late;
            // wake the loop with a Notify if teachers find that sluggish.
            for action in self.take_pending(id) {
                let outcome = session.perform(action).await.map_err(|e| e.to_string())?;
                self.set_last_action(id, ActionReport::new(action, outcome));
            }

            let (monitor, width, focused) = self.request_shape(id, key);
            match session.request_thumbnail(monitor, width).await {
                Ok(jpeg) => self.set_screen(id, &jpeg),
                // The student's screen is momentarily uncapturable — locked, or a UAC prompt is up.
                // Keep the connection and the last frame; the screen comes back on a later tick.
                // Dropping the session here is what produced the reconnect storm seen in testing.
                Err(net::EndpointError::ScreenUnavailable) => {}
                Err(e) => return Err(e.to_string()),
            }

            audio_on = self.pump_audio(&mut session, key, audio_on).await?;
            tokio::time::sleep(if focused { FOCUSED_REFRESH } else { REFRESH }).await;
        }
    }

    /// Turns listening on or off for this device as the teacher's choice changes, and moves one
    /// chunk of sound to the speakers. Returns the format currently running, if any.
    async fn pump_audio(
        &self,
        session: &mut ControlSession,
        key: [u8; 32],
        audio_on: Option<proto::AudioFormat>,
    ) -> Result<Option<proto::AudioFormat>, String> {
        let wanted = self
            .preview
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .listening
            == Some(key);

        match (wanted, audio_on) {
            // Nothing to do.
            (false, None) => Ok(None),
            // The teacher stopped listening to this PC: tell it to stop recording.
            (false, Some(_)) => {
                let _ = session.set_audio(false).await;
                Ok(None)
            }
            // Newly listening: start, and open playback at whatever rate the agent reports.
            (true, None) => {
                let format = session.set_audio(true).await.map_err(|e| e.to_string())?;
                if let Some(format) = format {
                    match media::audio::AudioPlayback::start(format.sample_rate) {
                        Ok(playback) => {
                            *self.playback.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(playback);
                        }
                        Err(err) => return Err(format!("no speakers on this PC: {err}")),
                    }
                }
                Ok(format)
            }
            // Already listening: collect a chunk and play it.
            (true, Some(format)) => {
                let max = format.sample_rate * AUDIO_CHUNK_MS / 1000;
                let samples = session
                    .request_audio(max)
                    .await
                    .map_err(|e| e.to_string())?;
                if !samples.is_empty()
                    && let Some(playback) = self
                        .playback
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_ref()
                {
                    let queue_cap = (format.sample_rate * AUDIO_QUEUE_MS / 1000) as usize;
                    playback.push(&samples, queue_cap);
                }
                Ok(Some(format))
            }
        }
    }

    /// What to ask for next: which monitor, how wide, and whether this is the opened screen.
    fn request_shape(&self, id: &str, key: [u8; 32]) -> (u8, u16, bool) {
        let preview = *self.preview.lock().unwrap_or_else(|e| e.into_inner());
        let focused = preview.focused == Some(key);
        let monitor = self
            .devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .map_or(0, |state| state.monitor);
        let width = if focused {
            preview.focused_width
        } else {
            preview.grid_width
        };
        (monitor, width, focused)
    }

    /// Records the monitors a PC reported, keeping the teacher's choice if it still exists.
    fn set_monitors(&self, id: &str, monitors: Vec<proto::Monitor>) {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = devices.get_mut(id) {
            // If the chosen monitor was unplugged, fall back to the first one that remains.
            if !monitors.iter().any(|m| m.index == state.monitor) {
                state.monitor = monitors.first().map_or(0, |m| m.index);
            }
            state.monitors = monitors;
        }
    }

    fn set_status(&self, id: &str, status: DeviceStatus, detail: Option<String>) {
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(state) = devices.get_mut(id) {
            if status != DeviceStatus::Live {
                // Anything still queued was for a connection that no longer exists; see `perform`.
                state.pending.clear();
                state.pending_input.clear();
                // Drop the reply channels: every waiting caller learns at once that the PC went
                // away, instead of hanging until it times out.
                state.requests.clear();
            }
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

    /// Keeps accepting student PCs that dial in with `code`, sending each one's device id on
    /// `events`, until `stop` is signalled. This is what lets a teacher enrol a whole lab from one
    /// open "Add a PC" panel with a single code — every connection gets a fresh session, so the code
    /// never goes stale while the panel is open.
    pub async fn pair_loop(
        &self,
        code: net::PairingCode,
        stop: std::sync::Arc<tokio::sync::Notify>,
        events: tokio::sync::mpsc::UnboundedSender<Result<String, String>>,
    ) {
        let endpoint = match self.endpoint().await {
            Ok(endpoint) => endpoint,
            Err(err) => {
                let _ = events.send(Err(err));
                return;
            }
        };
        endpoint.online().await;
        let welcome = match self.welcome() {
            Ok(welcome) => welcome,
            Err(err) => {
                let _ = events.send(Err(err));
                return;
            }
        };

        loop {
            tokio::select! {
                biased;
                () = stop.notified() => break,
                incoming = endpoint.accept() => {
                    let Some(incoming) = incoming else { break }; // endpoint closed
                    let mut trust = self.trust.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    // A wrong code or a dropped handshake just falls through — keep waiting.
                    if let Ok(peer) =
                        net::console_pair_connection(incoming, code, &mut trust, &welcome).await
                    {
                        if let Err(err) = trust.save(&self.trust_path) {
                            let _ = events.send(Err(err.to_string()));
                            continue;
                        }
                        let id = peer.device_id.to_string();
                        *self.trust.lock().unwrap_or_else(|e| e.into_inner()) = trust;
                        self.devices
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .entry(id.clone())
                            .or_insert_with(|| DeviceState::new(peer.public_key));
                        let _ = events.send(Ok(id));
                    }
                }
            }
        }
    }
}

/// Lowercase hex, for showing and re-parsing keys.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Loads the teacher's custom PC names from `names.txt` (`id = name` per line). Missing file is fine.
fn load_names(path: &std::path::Path) -> std::collections::HashMap<String, String> {
    let mut names = std::collections::HashMap::new();
    if let Ok(text) = std::fs::read_to_string(path) {
        for line in text.lines() {
            if let Some((id, name)) = line.split_once(" = ") {
                let (id, name) = (id.trim(), name.trim());
                if !id.is_empty() && !name.is_empty() {
                    names.insert(id.to_string(), name.to_string());
                }
            }
        }
    }
    names
}

/// Minimal base64 (standard alphabet, padded) so screens can go straight into an `<img src>`.
pub(crate) fn base64(bytes: &[u8]) -> String {
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
    fn every_action_name_parses_back_to_itself() {
        for name in [
            "shutdown",
            "reboot",
            "log-off",
            "lock-screen",
            "cancel-shutdown",
        ] {
            assert_eq!(parse_action(name, 30).map(proto::Action::name), Some(name));
        }
    }

    #[test]
    fn an_unknown_action_name_is_refused() {
        assert_eq!(parse_action("format-c", 0), None);
    }

    #[test]
    fn the_delay_reaches_power_actions() {
        assert_eq!(
            parse_action("reboot", 45),
            Some(proto::Action::Reboot { delay_seconds: 45 })
        );
    }

    #[test]
    fn hex_is_lowercase_and_padded() {
        assert_eq!(hex(&[0x00, 0x0f, 0xff]), "000fff");
    }
}
