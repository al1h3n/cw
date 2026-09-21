//! Surey — the Console's AI assistant.
//!
//! The teacher chats with Surey in a floating panel; Surey can *act* on the class by calling the same
//! typed fleet tools the Console exposes (`ai::tools`). All provider traffic is proxied here, in Rust,
//! so the API key never touches the web layer (D22). The web side sends the running conversation each
//! turn and listens for events — assistant prose, tool activity, and `ask_user` selection requests.

pub mod client;
pub mod provider;
pub mod tools;

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use tauri::Emitter;

use client::{Msg, Step};
use provider::{ProviderConfig, ProviderKind, Store};

use crate::manager::DeviceManager;

/// One message in the conversation, as the web layer stores and sends it.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ChatMessage {
    /// `user` or `assistant`.
    pub role: String,
    /// The message text.
    pub content: String,
}

/// A public view of the provider config for the settings UI (never includes the key).
#[derive(serde::Serialize)]
pub struct ConfigView {
    /// Provider family, lower-case.
    pub kind: ProviderKind,
    /// The endpoint base URL.
    pub base_url: String,
    /// The model id.
    pub model: String,
    /// Whether a key is stored (so the UI can show "key set" without ever seeing it).
    pub has_key: bool,
}

/// Never loop tool calls forever: a runaway model is stopped after this many rounds.
const MAX_TOOL_ROUNDS: usize = 10;
/// How long to wait for the teacher to answer an `ask_user` before giving up the turn.
const CHOICE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// The assistant's identity and standing rules — the "pre-prompt" sent first every turn.
const SUREY_BRIEF: &str = "\
You are Surey, the AI assistant built into Co-watcher, a classroom/computer-lab control console. You \
help a teacher watch and manage student PCs. You can ACT on the class only through the provided tools \
(there is no shell and no file access). Be concise and concrete.

How to work:
- Start from list_devices to learn the PCs, their names and status. Only 'live' PCs can be acted on; \
if a PC is not connected, say so and suggest the teacher start watching it.
- Map the teacher's words to device_ids yourself (e.g. 'PC 4' is usually a device whose name contains \
'4'). If it is ambiguous, use ask_user to let them pick.
- Confirm DESTRUCTIVE or disruptive actions (shutdown, reboot, log-off, starting an exam lock) with \
ask_user before doing them. cancel-shutdown undoes a countdown.
- Prefer the least forceful tool that meets the goal (block a game rather than shut a PC down).
- Every action is logged and attributed on the student PC. Never claim an action succeeded if the \
tool reported it failed.";

/// Holds the AI config, the HTTP client, and the in-flight `ask_user` requests.
pub struct AiState {
    store: Store,
    config: Mutex<ProviderConfig>,
    http: reqwest::Client,
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>,
    next_id: AtomicU64,
}

impl AiState {
    /// Loads the saved config from the Console data directory.
    #[must_use]
    pub fn load(data_dir: &std::path::Path) -> Self {
        let store = Store::new(data_dir);
        let config = store.load_config();
        Self {
            store,
            config: Mutex::new(config),
            http: reqwest::Client::new(),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// The current config as a UI view (no key).
    #[must_use]
    pub fn config_view(&self) -> ConfigView {
        let config = self.config.lock().unwrap_or_else(|e| e.into_inner());
        ConfigView {
            kind: config.kind,
            base_url: config.base_url.clone(),
            model: config.model.clone(),
            has_key: self.store.has_key(),
        }
    }

    /// Saves a new config (and the key when `key` is `Some`; `Some("")` clears it, `None` keeps it).
    ///
    /// # Errors
    /// If persisting the config or key fails.
    pub fn set_config(&self, config: ProviderConfig, key: Option<String>) -> Result<(), String> {
        self.store.save_config(&config)?;
        if let Some(key) = key {
            self.store.set_key(key.trim())?;
        }
        *self.config.lock().unwrap_or_else(|e| e.into_inner()) = config;
        Ok(())
    }

    /// Answers an outstanding `ask_user` request. Returns whether a request with that id was waiting.
    pub fn resolve_choice(&self, id: &str, value: String) -> bool {
        let sender = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        match sender {
            Some(sender) => sender.send(value).is_ok(),
            None => false,
        }
    }

    /// Lists the model ids the configured endpoint offers (OpenAI-compatible `/v1/models`). Handy for
    /// a local server (Ollama/LM Studio) where the teacher may not remember the exact model name.
    ///
    /// # Errors
    /// Network failure or a non-success status.
    pub async fn list_models(&self) -> Result<Vec<String>, String> {
        let (base, key) = {
            let config = self.config.lock().unwrap_or_else(|e| e.into_inner());
            (config.trimmed_base(), self.store.get_key())
        };
        let mut request = self.http.get(format!("{base}/models"));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("the endpoint returned {}", response.status()));
        }
        let value: Value = response.json().await.map_err(|e| e.to_string())?;
        let ids = value
            .get("data")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok(ids)
    }

    /// Transcribes recorded audio to text via the endpoint's `/audio/transcriptions` (Whisper-shape).
    ///
    /// # Errors
    /// Network failure, a non-success status, or the endpoint does not do transcription.
    pub async fn transcribe(&self, audio: Vec<u8>, filename: String) -> Result<String, String> {
        let (base, key) = {
            let config = self.config.lock().unwrap_or_else(|e| e.into_inner());
            (config.trimmed_base(), self.store.get_key())
        };
        let part = reqwest::multipart::Part::bytes(audio)
            .file_name(filename)
            .mime_str("application/octet-stream")
            .map_err(|e| e.to_string())?;
        let form = reqwest::multipart::Form::new()
            .text("model", "whisper-1")
            .part("file", part);
        let mut request = self
            .http
            .post(format!("{base}/audio/transcriptions"))
            .multipart(form);
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "transcription returned {} (does this endpoint support Whisper?)",
                response.status()
            ));
        }
        let value: Value = response.json().await.map_err(|e| e.to_string())?;
        value
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| "transcription reply had no text".to_string())
    }

    /// Runs one full assistant turn: it may call tools (looping) and ask the teacher to choose, then
    /// returns the assistant's final text. Progress is emitted as `surey://…` events on `window`.
    ///
    /// # Errors
    /// No key configured when one is needed, a provider error, or a tool round-trip failure.
    pub async fn run_turn(
        &self,
        window: &tauri::Window,
        manager: &DeviceManager,
        history: Vec<ChatMessage>,
    ) -> Result<String, String> {
        let (config, key) = {
            let config = self.config.lock().unwrap_or_else(|e| e.into_inner());
            (config.clone(), self.store.get_key())
        };
        if key.is_none() && config.kind != ProviderKind::Local {
            return Err("No API key set. Open Surey's settings and add your provider key.".into());
        }

        let mut messages = vec![Msg::System(self.system_prompt(manager, &config))];
        for message in history {
            match message.role.as_str() {
                "assistant" => messages.push(Msg::Assistant {
                    text: Some(message.content),
                    tool_calls: Vec::new(),
                }),
                _ => messages.push(Msg::User(message.content)),
            }
        }

        let tools = tools::specs();
        let mut final_text = String::new();

        for _ in 0..MAX_TOOL_ROUNDS {
            let step: Step =
                client::complete(&self.http, &config, key.as_deref(), &messages, &tools).await?;

            if step.tool_calls.is_empty() {
                final_text = step.text.unwrap_or_default();
                break;
            }

            // Record the assistant's tool-call turn before its results, as the wire shape requires.
            if let Some(text) = &step.text
                && !text.is_empty()
            {
                let _ = window.emit("surey://note", json!({ "text": text }));
            }
            messages.push(Msg::Assistant {
                text: step.text.clone(),
                tool_calls: step.tool_calls.clone(),
            });

            for call in step.tool_calls {
                let _ = window.emit(
                    "surey://tool",
                    json!({ "name": call.name, "arguments": call.arguments }),
                );
                let result = if call.name == tools::ASK_USER {
                    self.ask_user(window, &call.arguments).await
                } else {
                    tools::execute(manager, &call.name, &call.arguments).await
                };
                let content = match result {
                    Ok(value) => value.to_string(),
                    Err(message) => json!({ "error": message }).to_string(),
                };
                messages.push(Msg::ToolResult {
                    id: call.id,
                    content,
                });
            }
        }

        let _ = window.emit("surey://done", json!({ "text": final_text }));
        Ok(final_text)
    }

    /// Emits a selection request to the panel and waits for the teacher's answer.
    async fn ask_user(&self, window: &tauri::Window, args: &Value) -> Result<Value, String> {
        let prompt = args
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("Choose:");
        let options: Vec<String> = args
            .get("options")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let allow_custom = args
            .get("allow_custom")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), tx);

        let _ = window.emit(
            "surey://choice",
            json!({ "id": id, "prompt": prompt, "options": options, "allow_custom": allow_custom }),
        );

        match tokio::time::timeout(CHOICE_TIMEOUT, rx).await {
            Ok(Ok(choice)) => Ok(json!({ "chosen": choice })),
            _ => {
                // Clean up the abandoned request so the map does not grow.
                self.pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                Err("the teacher did not choose (dismissed or timed out)".into())
            }
        }
    }

    /// The system prompt: the standing brief plus a live snapshot of the class so the model can map
    /// names/ids without a first tool call.
    fn system_prompt(&self, manager: &DeviceManager, config: &ProviderConfig) -> String {
        let mut prompt = String::from(SUREY_BRIEF);
        prompt.push_str("\n\nCurrently paired PCs (device_id — name — status):");
        let devices = manager.devices();
        if devices.is_empty() {
            prompt.push_str("\n(none paired yet)");
        } else {
            for device in devices {
                let name = device.name.unwrap_or_else(|| "(unnamed)".to_string());
                let status = serde_json::to_value(device.status)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default();
                prompt.push_str(&format!("\n- {} — {} — {}", device.device_id, name, status));
            }
        }
        let _ = config; // reserved for model-specific prompt tweaks later
        prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> AiState {
        let dir = std::env::temp_dir().join(format!("cw-ai-state-{}", std::process::id()));
        AiState::load(&dir)
    }

    #[test]
    fn resolving_an_unknown_choice_is_false_not_a_panic() {
        assert!(!state().resolve_choice("nope", "x".into()));
    }

    #[test]
    fn the_brief_names_surey_and_forbids_a_shell() {
        assert!(SUREY_BRIEF.contains("Surey"));
        assert!(SUREY_BRIEF.contains("no shell"));
    }

    #[test]
    fn a_fresh_config_view_has_no_key() {
        let view = state().config_view();
        assert!(!view.has_key);
        assert_eq!(view.kind, ProviderKind::OpenAi);
    }
}
