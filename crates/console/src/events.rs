//! A transport-agnostic event sink.
//!
//! Some commands push progress out of band — a PC joined during pairing, a broadcast ended, Surey
//! finished a step. On the desktop those go to the Tauri window; in `cowatcher-console web` the same
//! events must reach every browser over SSE. [`Emitter`] lets one piece of logic serve both, so the
//! pairing loop, broadcast fan-out and AI turn are written once and don't care where they are running.

use serde::Serialize;

/// One event as it travels to a browser over SSE: the event name and its JSON payload.
#[derive(Debug, Clone, Serialize)]
pub struct WebEvent {
    pub event: String,
    pub payload: serde_json::Value,
}

/// Where out-of-band events go.
#[derive(Clone)]
#[expect(
    clippy::large_enum_variant,
    reason = "a Tauri Window is a cheap-to-clone handle despite its size; boxing it would add an \
              indirection for no real memory saving on the single long-lived emitter per session"
)]
pub enum Emitter {
    /// The desktop console: emit onto the Tauri window.
    Tauri(tauri::Window),
    /// The web dashboard: broadcast to every connected browser's SSE stream.
    Web(tokio::sync::broadcast::Sender<WebEvent>),
}

impl Emitter {
    /// Sends one named event with a serializable payload. Failures are swallowed: a dropped UI or a
    /// browser that just disconnected must never break the work that emitted the event.
    pub fn emit<S: Serialize + Clone>(&self, event: &str, payload: S) {
        match self {
            Self::Tauri(window) => {
                use tauri::Emitter as _;
                let _ = window.emit(event, payload);
            }
            Self::Web(tx) => {
                if let Ok(value) = serde_json::to_value(payload) {
                    // `send` errs only when there are no receivers; that is fine.
                    let _ = tx.send(WebEvent {
                        event: event.to_string(),
                        payload: value,
                    });
                }
            }
        }
    }
}
