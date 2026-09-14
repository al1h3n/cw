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
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            console_info,
            devices,
            start_watching,
            stop_watching,
            begin_pairing,
            await_pairing,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| e.to_string())
}
