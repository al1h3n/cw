//! How Surey turns recorded voice into text — chosen by the teacher, so voice never forces a setup.
//!
//! WebView2 has no built-in speech service, so transcription needs *some* backend. Rather than bundle
//! a large model, the teacher picks one of three modes:
//!
//! * **Chat** (default): reuse the already-configured chat AI endpoint's `/audio/transcriptions`
//!   (works when that provider is a full OpenAI-compatible endpoint with Whisper).
//! * **Endpoint**: a **separate** Whisper-shape HTTP endpoint (base URL + optional key + model). This
//!   is also how you use a locally-installed open-source server — run e.g. `whisper.cpp`'s server or
//!   `faster-whisper-server` on the teacher PC and point this at `http://localhost:PORT/v1`. Nothing is
//!   bundled; the binary lives on the host.
//! * **Local**: run a local transcription **program** the teacher installed, on the recorded audio.
//!   The command is a template with `{in}` (the audio file) and optional `{out}` (a text file the tool
//!   writes); its stdout is used when no `{out}` is given. This runs only on the teacher's own console
//!   machine from the teacher's own config — it is not a remote action and is never exposed to a
//!   student PC (AGENTS.md §5: remote actions stay a fixed typed enum; this is a local tool path).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where transcription happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TranscribeMode {
    /// Reuse the chat AI endpoint's `/audio/transcriptions`.
    #[default]
    Chat,
    /// A dedicated Whisper-shape HTTP endpoint (cloud, or a local open-source server).
    Endpoint,
    /// A local command-line program the teacher installed.
    Local,
}

/// The teacher's transcription choice (everything except the secret key).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct TranscribeConfig {
    pub mode: TranscribeMode,
    /// Base URL (up to `/v1`) for `Endpoint` mode.
    pub url: String,
    /// Model id for HTTP modes (defaults to `whisper-1` when empty).
    pub model: String,
    /// Executable path for `Local` mode.
    pub bin: String,
    /// Argument template for `Local` mode: `{in}` = audio file, `{out}` = optional text file.
    pub args: String,
}

impl TranscribeConfig {
    /// The model to send, defaulting to `whisper-1`.
    #[must_use]
    pub fn model_or_default(&self) -> String {
        let model = self.model.trim();
        if model.is_empty() {
            "whisper-1".to_string()
        } else {
            model.to_string()
        }
    }
}

/// A UI view of the transcription config (never the key).
#[derive(Debug, Clone, Serialize)]
pub struct TranscribeView {
    pub mode: TranscribeMode,
    pub url: String,
    pub model: String,
    pub bin: String,
    pub args: String,
    pub has_key: bool,
}

/// Stores the transcription config (JSON) and its optional key (DPAPI-protected), separate from the
/// chat provider's own `ai.json`/`ai.key`.
pub struct Store {
    config_path: PathBuf,
    key_path: PathBuf,
}

impl Store {
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            config_path: data_dir.join("transcribe.json"),
            key_path: data_dir.join("transcribe.key"),
        }
    }

    #[must_use]
    pub fn load_config(&self) -> TranscribeConfig {
        std::fs::read_to_string(&self.config_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// # Errors
    /// If the file cannot be written.
    pub fn save_config(&self, config: &TranscribeConfig) -> Result<(), String> {
        let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
        std::fs::write(&self.config_path, text).map_err(|e| e.to_string())
    }

    /// Stores the endpoint key encrypted (DPAPI); an empty key clears it.
    ///
    /// # Errors
    /// If encryption or the write fails.
    pub fn set_key(&self, key: &str) -> Result<(), String> {
        if key.is_empty() {
            let _ = std::fs::remove_file(&self.key_path);
            return Ok(());
        }
        let sealed =
            platform::secret::protect(key.as_bytes()).map_err(|e| e.message().to_string())?;
        std::fs::write(&self.key_path, sealed).map_err(|e| e.to_string())
    }

    #[must_use]
    pub fn get_key(&self) -> Option<String> {
        let sealed = std::fs::read(&self.key_path).ok()?;
        let plain = platform::secret::unprotect(&sealed).ok()?;
        String::from_utf8(plain).ok()
    }

    #[must_use]
    pub fn has_key(&self) -> bool {
        self.key_path.exists()
    }
}

/// Runs a local transcription program on `audio` and returns the text it produced.
///
/// The audio is written to a temp file named with `filename`'s extension; `{in}` in the argument
/// template is replaced with that path and `{out}` (if present) with a temp `.txt` path the tool is
/// expected to write. When `{out}` is used the file's contents are returned; otherwise stdout is.
///
/// # Errors
/// If the config is incomplete, the program cannot be started, or it produces no text.
pub async fn run_local(
    config: &TranscribeConfig,
    audio: &[u8],
    filename: &str,
) -> Result<String, String> {
    let bin = config.bin.trim();
    if bin.is_empty() {
        return Err("no transcription program is set (Surey settings → Voice)".into());
    }
    let dir = std::env::temp_dir();
    let stamp = net::endpoint::now_ms();
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("webm");
    let input = dir.join(format!("cowatcher-voice-{stamp}.{ext}"));
    let output = dir.join(format!("cowatcher-voice-{stamp}.txt"));
    std::fs::write(&input, audio).map_err(|e| format!("write temp audio: {e}"))?;

    let uses_out = config.args.contains("{out}");
    let mut args: Vec<String> = if config.args.trim().is_empty() {
        vec![input.display().to_string()]
    } else {
        config
            .args
            .split_whitespace()
            .map(|a| {
                a.replace("{in}", &input.display().to_string())
                    .replace("{out}", &output.display().to_string())
            })
            .collect()
    };
    // If the template never mentioned the input, pass it last so a bare program still gets the file.
    if !config.args.contains("{in}") && !config.args.trim().is_empty() {
        args.push(input.display().to_string());
    }

    let result = tokio::process::Command::new(bin).args(&args).output().await;
    let _ = std::fs::remove_file(&input);
    let out = result.map_err(|e| format!("could not run '{bin}': {e}"))?;
    if !out.status.success() {
        let _ = std::fs::remove_file(&output);
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "the transcription program failed: {}",
            stderr.lines().last().unwrap_or("no output").trim()
        ));
    }
    let text = if uses_out {
        let text = std::fs::read_to_string(&output)
            .map_err(|e| format!("the program wrote no text file: {e}"))?;
        let _ = std::fs::remove_file(&output);
        text
    } else {
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        Err("the transcription program returned no text".into())
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_defaults_to_whisper_1_when_blank() {
        let cfg = TranscribeConfig::default();
        assert_eq!(cfg.model_or_default(), "whisper-1");
        let cfg = TranscribeConfig {
            model: "  base.en  ".to_string(),
            ..TranscribeConfig::default()
        };
        assert_eq!(cfg.model_or_default(), "base.en");
    }

    #[test]
    fn config_round_trips_through_the_store() {
        let dir = std::env::temp_dir().join(format!("cw-tr-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let store = Store::new(&dir);
        let cfg = TranscribeConfig {
            mode: TranscribeMode::Endpoint,
            url: "http://localhost:8080/v1".to_string(),
            model: "whisper-1".to_string(),
            ..TranscribeConfig::default()
        };
        store.save_config(&cfg).expect("save");
        assert_eq!(store.load_config().mode, TranscribeMode::Endpoint);
        assert_eq!(store.load_config().url, "http://localhost:8080/v1");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn local_mode_needs_a_program() {
        let cfg = TranscribeConfig {
            mode: TranscribeMode::Local,
            ..TranscribeConfig::default()
        };
        let err = run_local(&cfg, b"x", "voice.webm")
            .await
            .expect_err("must refuse");
        assert!(err.contains("no transcription program"), "got: {err}");
    }
}
