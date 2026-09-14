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
fn begin_pairing(state: State<'_, AppState>) -> PairingInvite {
    let code = PairingCode::generate();
    *state.pairing_code.lock().unwrap_or_else(|e| e.into_inner()) = Some(code);
    PairingInvite {
        code: code.to_string(),
        command: format!(
            "cowatcher-agent pair {} {}",
            state.manager.public_key(),
            code
        ),
    }
}

/// Waits for a student PC to dial in with the shown code. Resolves with its device id.
#[tauri::command]
async fn await_pairing(state: State<'_, AppState>) -> Result<String, String> {
    let code = state
        .pairing_code
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .ok_or_else(|| "no pairing code is showing".to_string())?;
    let manager = Arc::clone(&state.manager);
    manager.pair_once(code).await
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
            set_listening,
            listening,
            translation,
            set_language,
            export_language_template,
            begin_pairing,
            await_pairing,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| e.to_string())
}
