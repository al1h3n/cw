//! The teacher-facing window: a live grid of student screens.
//!
//! The UI is deliberately thin. Every command here returns plain data from [`DeviceManager`], so the
//! front end never deals with keys, sockets or retries — it renders devices and calls actions.

use std::sync::{Arc, Mutex};

use net::PairingCode;
use tauri::{Manager, State};

use crate::manager::{DeviceManager, DeviceView};

/// Everything the window needs, shared across commands.
struct AppState {
    manager: Arc<DeviceManager>,
    /// The pairing code currently on screen, if the teacher opened "Add a PC".
    pairing_code: Mutex<Option<PairingCode>>,
    /// Stops the current continuous-pairing loop when the teacher closes the panel.
    pairing_stop: Mutex<Option<Arc<tokio::sync::Notify>>>,
    /// Where per-user files live: the trust store, the device key and `languages/`.
    data_dir: std::path::PathBuf,
}

impl AppState {
    /// The folder a teacher drops extra `.ini` translations into.
    fn languages_dir(&self) -> std::path::PathBuf {
        self.data_dir.join("languages")
    }

    /// The language chosen last time, or the OS language, or English.
    fn saved_language(&self) -> Option<String> {
        std::fs::read_to_string(self.data_dir.join("language.txt"))
            .ok()
            .map(|s| s.trim().to_owned())
    }
}

/// One entry in the language switcher.
#[derive(serde::Serialize)]
struct LanguageOption {
    code: String,
    name: String,
}

/// The strings the window should display, plus the list of languages to offer.
#[derive(serde::Serialize)]
struct Translation {
    code: String,
    strings: std::collections::BTreeMap<String, String>,
    available: Vec<LanguageOption>,
    /// Files that failed to parse, so a translator sees their mistake instead of silence.
    problems: Vec<String>,
    /// Shown in the UI so the teacher knows where to put a new `.ini`.
    languages_dir: String,
}

/// Loads the requested language (or the remembered/system one when `code` is `None`).
#[tauri::command]
fn translation(
    state: State<'_, AppState>,
    code: Option<String>,
    system: Option<String>,
) -> Translation {
    let dir = state.languages_dir();
    let (catalogs, problems) = crate::i18n::available(&dir);

    // Preference order: what the UI asked for, what was saved, the OS language, English.
    let system_code = system.unwrap_or_default();
    let system_code = system_code
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let wanted = code
        .or_else(|| state.saved_language())
        .filter(|c| catalogs.iter().any(|k| &k.code == c))
        .or_else(|| {
            catalogs
                .iter()
                .find(|k| k.code == system_code)
                .map(|k| k.code.clone())
        })
        .unwrap_or_else(|| "en".to_owned());

    let resolved = crate::i18n::resolve(&catalogs, &wanted);
    Translation {
        code: resolved.code,
        strings: resolved.strings,
        available: catalogs
            .into_iter()
            .map(|c| LanguageOption {
                code: c.code,
                name: c.name,
            })
            .collect(),
        problems,
        languages_dir: dir.display().to_string(),
    }
}

/// Remembers the teacher's language choice for next launch.
#[tauri::command]
fn set_language(state: State<'_, AppState>, code: String) -> Result<(), String> {
    std::fs::create_dir_all(&state.data_dir).map_err(|e| e.to_string())?;
    std::fs::write(state.data_dir.join("language.txt"), code).map_err(|e| e.to_string())
}

/// Writes the English file into the languages folder as a starting point for a new translation.
#[tauri::command]
fn export_language_template(state: State<'_, AppState>) -> Result<String, String> {
    let dir = state.languages_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("template.ini");
    std::fs::write(&path, crate::i18n::template()).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

/// Who this console is, shown in the header and on the pairing card.
#[derive(serde::Serialize)]
struct ConsoleInfo {
    device_id: String,
    public_key: String,
}

#[tauri::command]
fn console_info(state: State<'_, AppState>) -> ConsoleInfo {
    ConsoleInfo {
        device_id: state.manager.device_id(),
        public_key: state.manager.public_key(),
    }
}

#[tauri::command]
fn devices(state: State<'_, AppState>) -> Vec<DeviceView> {
    state.manager.devices()
}

#[tauri::command]
async fn start_watching(state: State<'_, AppState>) -> Result<(), String> {
    let manager = Arc::clone(&state.manager);
    manager.start_watching().await
}

#[tauri::command]
fn stop_watching(state: State<'_, AppState>) {
    state.manager.stop_watching();
}

/// The preview widths in use, so the UI can show the current choice.
#[derive(serde::Serialize)]
struct PreviewWidths {
    grid: u16,
    focused: u16,
}

#[tauri::command]
fn preview_widths(state: State<'_, AppState>) -> PreviewWidths {
    let (grid, focused) = state.manager.preview_widths();
    PreviewWidths { grid, focused }
}

/// Sets how sharp the previews are: the grid tiles and the opened screen separately.
#[tauri::command]
fn set_preview_widths(state: State<'_, AppState>, grid: u16, focused: u16) {
    state.manager.set_preview_widths(grid, focused);
}

/// Tells the manager which screen is open, so it is refreshed faster and larger.
#[tauri::command]
fn set_focused(state: State<'_, AppState>, device_id: Option<String>) {
    state.manager.set_focused(device_id.as_deref());
}

/// Starts listening to one PC, or stops listening when `device_id` is absent.
#[tauri::command]
fn set_listening(state: State<'_, AppState>, device_id: Option<String>) -> Result<(), String> {
    state.manager.set_listening(device_id.as_deref())
}

/// Which PC is being listened to, if any.
#[tauri::command]
fn listening(state: State<'_, AppState>) -> Option<String> {
    state.manager.listening()
}

/// Chooses which monitor of a multi-monitor student PC to show.
#[tauri::command]
fn set_monitor(state: State<'_, AppState>, device_id: String, monitor: u8) -> Result<(), String> {
    state.manager.set_monitor(&device_id, monitor)
}

/// Wakes a switched-off PC by broadcasting a Wake-on-LAN packet for the MACs we learned earlier.
#[tauri::command]
fn wake(state: State<'_, AppState>, device_id: String) -> Result<usize, String> {
    state.manager.wake(&device_id)
}

/// Sets a teacher's own name for one PC (empty clears it back to just the id).
#[tauri::command]
fn rename_device(state: State<'_, AppState>, device_id: String, name: String) -> Result<(), String> {
    state.manager.rename(&device_id, &name)
}

/// Opens the native full-resolution viewer window for one PC (ADR D12).
///
/// The viewer is a separate binary that decodes the H.264 stream in Rust; the WebView cannot. It
/// dials as this same console (it reads the same state directory), so no key is passed around. When
/// `control` is true the teacher can immediately drive that PC; Ctrl+Alt+Esc toggles it in-window.
#[tauri::command]
fn open_viewer(
    state: State<'_, AppState>,
    device_id: String,
    width: u32,
    height: u32,
    fps: u32,
    kbps: u32,
    control: bool,
) -> Result<(), String> {
    let key = state.manager.endpoint_key(&device_id)?;
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("cowatcher-viewer.exe")))
        .ok_or("could not locate the viewer next to the console")?;
    if !exe.exists() {
        return Err(format!(
            "the viewer is not installed next to the console (looked for {})",
            exe.display()
        ));
    }
    let mut command = std::process::Command::new(exe);
    command
        .arg(key)
        .arg(width.to_string())
        .arg(height.to_string())
        .arg(fps.to_string())
        .arg(kbps.to_string());
    if control {
        command.arg("control");
    }
    // CREATE_NO_WINDOW: never flash a console window when launching the viewer.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not start the viewer: {e}"))
}

/// Takes control of one PC's mouse and keyboard, or releases it when `device_id` is absent.
#[tauri::command]
fn set_controlling(state: State<'_, AppState>, device_id: Option<String>) -> Result<(), String> {
    state.manager.set_controlling(device_id.as_deref())
}

/// Which PC is being driven, if any.
#[tauri::command]
fn controlling(state: State<'_, AppState>) -> Option<String> {
    state.manager.controlling()
}

/// One input event as the front end describes it, before it becomes a typed [`proto::InputEvent`].
///
/// The UI reports pointer positions as fractions of the *image* it is showing, which is exactly the
/// fraction of the student's screen — so a different resolution on either side changes nothing.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum UiInput {
    /// Pointer moved to this fraction of the screen (0.0 – 1.0).
    Move { x: f64, y: f64 },
    /// A mouse button changed state.
    Button { button: String, down: bool },
    /// The wheel turned.
    Scroll { delta: i32 },
    /// A key changed state, by Windows virtual-key code.
    Key { virtual_key: u16, down: bool },
    /// A character was typed.
    Text { text: String },
    /// Release every held modifier.
    ReleaseAll,
}

/// Converts the UI's description into wire events, dropping anything malformed rather than guessing.
fn to_wire(events: Vec<UiInput>) -> Vec<proto::InputEvent> {
    /// Fractions arrive as 0.0–1.0 and go out as 0–65535, the range Windows itself uses.
    fn fraction(value: f64) -> u16 {
        let clamped = if value.is_nan() {
            0.5
        } else {
            value.clamp(0.0, 1.0)
        };
        (clamped * f64::from(u16::MAX)) as u16
    }
    let mut out = Vec::new();
    for event in events {
        match event {
            UiInput::Move { x, y } => out.push(proto::InputEvent::MoveTo {
                x: fraction(x),
                y: fraction(y),
            }),
            UiInput::Button { button, down } => {
                let button = match button.as_str() {
                    "left" => proto::PointerButton::Left,
                    "right" => proto::PointerButton::Right,
                    "middle" => proto::PointerButton::Middle,
                    _ => continue, // unknown button: ignore rather than invent a click
                };
                out.push(proto::InputEvent::Button { button, down });
            }
            UiInput::Scroll { delta } => out.push(proto::InputEvent::Scroll {
                delta: delta.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
            }),
            UiInput::Key { virtual_key, down } => {
                out.push(proto::InputEvent::Key { virtual_key, down });
            }
            UiInput::Text { text } => out.extend(text.chars().map(proto::InputEvent::Text)),
            UiInput::ReleaseAll => out.push(proto::InputEvent::ReleaseAll),
        }
    }
    out
}

/// Queues input for the PC currently being controlled.
#[tauri::command]
fn send_input(state: State<'_, AppState>, events: Vec<UiInput>) -> Result<(), String> {
    state.manager.queue_input(to_wire(events))
}

/// Starts recording one PC's screen. Returns what it is actually recording after clamping.
#[tauri::command]
#[expect(clippy::too_many_arguments, reason = "a flat command over the recording options")]
async fn start_recording(
    state: State<'_, AppState>,
    device_id: String,
    max_width: u32,
    max_height: u32,
    fps: u32,
    codec: String,
    preset: String,
    quality: u8,
    bframes: u8,
    scaler: String,
    two_pass: bool,
) -> Result<proto::RecordingInfo, String> {
    let options = proto::RecordOptions {
        max_width,
        max_height,
        fps,
        codec: parse_codec(&codec),
        preset: parse_preset(&preset),
        quality,
        bframes,
        scaler: parse_scaler(&scaler),
        two_pass,
    };
    state.manager.start_recording(&device_id, options).await
}

fn parse_codec(s: &str) -> proto::Codec {
    match s {
        "h265" => proto::Codec::H265,
        "av1" => proto::Codec::Av1,
        _ => proto::Codec::H264,
    }
}

fn parse_preset(s: &str) -> proto::Preset {
    match s {
        "ultrafast" => proto::Preset::Ultrafast,
        "superfast" => proto::Preset::Superfast,
        "veryfast" => proto::Preset::Veryfast,
        "faster" => proto::Preset::Faster,
        "fast" => proto::Preset::Fast,
        "slow" => proto::Preset::Slow,
        "slower" => proto::Preset::Slower,
        "veryslow" => proto::Preset::Veryslow,
        _ => proto::Preset::Medium,
    }
}

fn parse_scaler(s: &str) -> proto::Scaler {
    match s {
        "bilinear" => proto::Scaler::Bilinear,
        "bicubic" => proto::Scaler::Bicubic,
        "neighbor" => proto::Scaler::Neighbor,
        _ => proto::Scaler::Lanczos,
    }
}

/// Starts or ends exam lockdown on one PC. Returns `[locked, problem]`.
#[tauri::command]
async fn set_exam(
    state: State<'_, AppState>,
    device_id: String,
    on: bool,
    message: String,
) -> Result<(bool, String), String> {
    state.manager.set_exam(&device_id, on, &message).await
}

/// Lists the recordings stored on one PC.
#[tauri::command]
async fn list_recordings(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<Vec<proto::StoredRecording>, String> {
    state.manager.list_recordings(&device_id).await
}

/// Downloads one recording to this teacher's PC, returning where it was saved.
#[tauri::command]
async fn download_recording(
    state: State<'_, AppState>,
    device_id: String,
    file: String,
) -> Result<String, String> {
    state.manager.download_recording(&device_id, &file).await
}

/// Stops the recording on one PC.
#[tauri::command]
async fn stop_recording(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<proto::RecordingInfo, String> {
    state.manager.stop_recording(&device_id).await
}

/// How the recording on one PC is going.
#[tauri::command]
async fn recording_status(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<proto::RecordingInfo, String> {
    state.manager.recording_status(&device_id).await
}

/// The programs a PC offers to start.
#[tauri::command]
async fn list_apps(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<Vec<proto::AppEntry>, String> {
    state.manager.list_apps(&device_id).await
}

/// Starts one of the programs a PC published.
#[tauri::command]
async fn launch_app(
    state: State<'_, AppState>,
    device_id: String,
    id: u32,
) -> Result<bool, String> {
    state
        .manager
        .launch_app(&device_id, id)
        .await
        .map(|(_, started)| started)
}

/// What is running and closable on a PC.
#[tauri::command]
async fn list_running(
    state: State<'_, AppState>,
    device_id: String,
) -> Result<Vec<proto::RunningApp>, String> {
    state.manager.list_running(&device_id).await
}

/// Closes a running program on a PC.
#[tauri::command]
async fn close_app(
    state: State<'_, AppState>,
    device_id: String,
    pid: u32,
) -> Result<bool, String> {
    state.manager.close_app(&device_id, pid).await
}

/// The room a device joins when invited, and the password needed to take one out again.
#[derive(serde::Serialize)]
struct RoomInfo {
    name: String,
    /// Grouped for reading aloud: `K7M2-Q9XR-4T6B`.
    password: String,
}

#[tauri::command]
fn room_info(state: State<'_, AppState>) -> RoomInfo {
    let (name, password) = state.manager.room();
    RoomInfo { name, password }
}

/// Renames the room. Devices already in it keep working.
#[tauri::command]
fn rename_room(state: State<'_, AppState>, name: String) -> Result<(), String> {
    state.manager.rename_room(&name)
}

/// Issues a new room password. Devices already invited keep the old one until re-invited.
#[tauri::command]
fn new_room_password(state: State<'_, AppState>) -> Result<(), String> {
    state.manager.new_room_password()
}

/// The room-wide blocklist, one program name per entry.
#[tauri::command]
fn blocklist(state: State<'_, AppState>) -> Vec<String> {
    state.manager.blocklist()
}

/// Replaces the room-wide blocklist; connected PCs enforce it within a second or two.
#[tauri::command]
fn set_blocklist(state: State<'_, AppState>, programs: Vec<String>) -> Result<(), String> {
    state.manager.set_blocklist(programs)
}

/// Sends one action to one PC, or to every connected PC when `device_id` is absent.
/// Returns how many PCs it was sent to; answers appear on each device's `last_action`.
#[tauri::command]
fn perform(
    state: State<'_, AppState>,
    device_id: Option<String>,
    action: String,
    delay_seconds: Option<u16>,
) -> Result<usize, String> {
    let action = crate::manager::parse_action(&action, delay_seconds.unwrap_or(0))
        .ok_or_else(|| format!("unknown action '{action}'"))?;
    state.manager.perform(device_id.as_deref(), action)
}

/// Generates the code the teacher reads out, and returns the exact command for the student PC.
#[derive(serde::Serialize)]
struct PairingInvite {
    code: String,
    command: String,
}

#[tauri::command]
fn begin_pairing(state: State<'_, AppState>, window: tauri::Window) -> PairingInvite {
    use tauri::Emitter;

    let code = PairingCode::generate();
    *state.pairing_code.lock().unwrap_or_else(|e| e.into_inner()) = Some(code);

    // Stop any earlier loop, then start a fresh one that accepts PC after PC until the panel closes.
    if let Some(previous) = state.pairing_stop.lock().unwrap_or_else(|e| e.into_inner()).take() {
        previous.notify_waiters();
    }
    let stop = Arc::new(tokio::sync::Notify::new());
    *state.pairing_stop.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::clone(&stop));

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();
    let manager = Arc::clone(&state.manager);
    tauri::async_runtime::spawn(async move { manager.pair_loop(code, stop, tx).await });
    // Forward each pairing result to the window as an event, so the panel can stay open and list
    // every PC that joins without blocking on a single call.
    tauri::async_runtime::spawn(async move {
        while let Some(result) = rx.recv().await {
            let _ = match result {
                Ok(id) => window.emit("cowatcher://paired", id),
                Err(err) => window.emit("cowatcher://pair-error", err),
            };
        }
    });

    PairingInvite {
        code: code.to_string(),
        command: format!(
            "cowatcher-agent pair {} {}",
            state.manager.public_key(),
            code
        ),
    }
}

/// Stops the continuous-pairing loop (the teacher closed the "Add a PC" panel).
#[tauri::command]
fn stop_pairing(state: State<'_, AppState>) {
    if let Some(stop) = state.pairing_stop.lock().unwrap_or_else(|e| e.into_inner()).take() {
        stop.notify_waiters();
    }
}

/// Opens the console window.
///
/// # Errors
/// Returns a message if state cannot be loaded or the window fails to start.
pub fn run(data_dir: std::path::PathBuf) -> Result<(), String> {
    let manager = Arc::new(DeviceManager::load(&data_dir)?);
    tauri::Builder::default()
        .setup(move |app| {
            app.manage(AppState {
                manager: Arc::clone(&manager),
                pairing_code: Mutex::new(None),
                pairing_stop: Mutex::new(None),
                data_dir: data_dir.clone(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            console_info,
            devices,
            start_watching,
            stop_watching,
            preview_widths,
            set_preview_widths,
            set_focused,
            set_monitor,
            perform,
            blocklist,
            set_blocklist,
            room_info,
            rename_room,
            new_room_password,
            set_controlling,
            controlling,
            send_input,
            wake,
            rename_device,
            open_viewer,
            start_recording,
            stop_recording,
            recording_status,
            list_recordings,
            download_recording,
            set_exam,
            list_apps,
            launch_app,
            list_running,
            close_app,
            set_listening,
            listening,
            translation,
            set_language,
            export_language_template,
            begin_pairing,
            stop_pairing,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| e.to_string())
}
