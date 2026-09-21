//! Which AI endpoint Surey talks to, and how its API key is kept.
//!
//! Per AGENTS.md D22 the teacher brings their own OpenAI/Anthropic-compatible endpoint: a base URL, a
//! model and (usually) an API key. The key is stored **locally under DPAPI** (`platform::secret`) and
//! never sent to the web layer or logged; every request is proxied through Rust. One config covers
//! four shapes — official OpenAI, official Anthropic, any OpenAI-compatible custom proxy, and a local
//! server (Ollama/LM Studio, which both expose an OpenAI-compatible `/v1`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The provider families Surey understands. Only [`ProviderKind::Anthropic`] uses the native
/// Anthropic Messages wire shape; every other kind uses the OpenAI chat-completions shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    /// Official OpenAI (`https://api.openai.com/v1`).
    OpenAi,
    /// Official Anthropic (`https://api.anthropic.com/v1`), native Messages API.
    Anthropic,
    /// Any other OpenAI-compatible endpoint (a proxy/gateway such as Omniroute).
    Custom,
    /// A local server exposing an OpenAI-compatible `/v1` (Ollama, LM Studio, …); usually no key.
    Local,
}

impl ProviderKind {
    /// Whether this kind speaks the native Anthropic Messages API rather than OpenAI chat-completions.
    #[must_use]
    pub fn is_anthropic(self) -> bool {
        matches!(self, ProviderKind::Anthropic)
    }
}

/// The teacher's AI endpoint choice (everything except the secret key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Which provider family.
    pub kind: ProviderKind,
    /// Base URL up to and including `/v1` (no trailing slash needed).
    pub base_url: String,
    /// Model id to request (e.g. `gpt-4o-mini`, `kr/claude-sonnet-4.5`, `llama3.1`).
    pub model: String,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            kind: ProviderKind::OpenAi,
            base_url: "https://api.openai.com/v1".to_string(),
            model: "gpt-4o-mini".to_string(),
        }
    }
}

impl ProviderConfig {
    /// The base URL with any trailing slash removed, so joining a path is unambiguous.
    #[must_use]
    pub fn trimmed_base(&self) -> String {
        self.base_url.trim().trim_end_matches('/').to_string()
    }
}

/// Stores and retrieves the provider config (JSON) and its API key (DPAPI-protected bytes).
pub struct Store {
    config_path: PathBuf,
    key_path: PathBuf,
}

impl Store {
    /// Uses `ai.json` and `ai.key` inside the Console data directory.
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            config_path: data_dir.join("ai.json"),
            key_path: data_dir.join("ai.key"),
        }
    }

    /// Loads the saved config, or the default if none is stored or it cannot be read.
    #[must_use]
    pub fn load_config(&self) -> ProviderConfig {
        std::fs::read_to_string(&self.config_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Saves the config as JSON.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn save_config(&self, config: &ProviderConfig) -> Result<(), String> {
        let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
        std::fs::write(&self.config_path, text).map_err(|e| e.to_string())
    }

    /// Stores the API key encrypted at rest (DPAPI). An empty key deletes the stored key.
    ///
    /// # Errors
    /// If encryption or the file write fails.
    pub fn set_key(&self, key: &str) -> Result<(), String> {
        if key.is_empty() {
            let _ = std::fs::remove_file(&self.key_path);
            return Ok(());
        }
        let sealed =
            platform::secret::protect(key.as_bytes()).map_err(|e| e.message().to_string())?;
        std::fs::write(&self.key_path, sealed).map_err(|e| e.to_string())
    }

    /// Reads and decrypts the stored API key, if any.
    #[must_use]
    pub fn get_key(&self) -> Option<String> {
        let sealed = std::fs::read(&self.key_path).ok()?;
        let plain = platform::secret::unprotect(&sealed).ok()?;
        String::from_utf8(plain).ok()
    }

    /// Whether an API key is stored (without revealing it).
    #[must_use]
    pub fn has_key(&self) -> bool {
        self.key_path.exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_trims_trailing_slash_and_space() {
        let cfg = ProviderConfig {
            base_url: "  https://omniroute.example/v1/  ".to_string(),
            ..ProviderConfig::default()
        };
        assert_eq!(cfg.trimmed_base(), "https://omniroute.example/v1");
    }

    #[test]
    fn config_round_trips_through_the_store() {
        let dir = std::env::temp_dir().join(format!("cw-ai-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let store = Store::new(&dir);
        assert!(!store.has_key());
        let cfg = ProviderConfig {
            kind: ProviderKind::Custom,
            base_url: "https://omniroute.mercat-pride.ts.net/v1".to_string(),
            model: "kr/claude-sonnet-4.5".to_string(),
        };
        store.save_config(&cfg).expect("save");
        let back = store.load_config();
        assert_eq!(back.model, "kr/claude-sonnet-4.5");
        assert_eq!(back.kind, ProviderKind::Custom);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_key_is_stored_encrypted_and_read_back() {
        let dir = std::env::temp_dir().join(format!("cw-ai-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let store = Store::new(&dir);
        store.set_key("sk-secret-123").expect("set key");
        assert!(store.has_key());
        // On disk it is not the plaintext.
        let raw = std::fs::read(dir.join("ai.key")).expect("read");
        assert!(
            !raw.windows(3).any(|w| w == b"sk-"),
            "key must be encrypted at rest"
        );
        assert_eq!(store.get_key().as_deref(), Some("sk-secret-123"));
        store.set_key("").expect("clear key");
        assert!(!store.has_key());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
