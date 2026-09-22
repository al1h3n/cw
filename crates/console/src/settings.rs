//! Console-wide preferences: whether AI features are on, and the colour theme.
//!
//! These are a teacher's personal choices rather than per-classroom state, so they live in the base
//! directory and apply to every classroom this Windows account opens. The file is small, human-
//! readable JSON; an unreadable or missing file falls back to the defaults rather than failing.

use std::path::{Path, PathBuf};

/// One colour theme. `dark` and `light` are built in; `custom` uses [`Settings::custom`].
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Dark,
    Light,
    Custom,
}

/// A custom palette. Each field is a CSS colour the front end drops straight into a CSS variable, so
/// the whole app re-themes without any Rust knowing what the colours mean. Empty strings fall back to
/// the dark defaults in the UI.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CustomTheme {
    pub bg: String,
    pub panel: String,
    pub panel2: String,
    pub line: String,
    pub text: String,
    pub muted: String,
    pub accent: String,
}

/// Everything on the settings surface that Rust persists.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Settings {
    /// When false, every AI (Surey) feature is hidden and no AI request is ever made.
    pub ai_enabled: bool,
    /// Whether previews linger after watching stops (moved here from ad-hoc localStorage).
    pub keep_previews: bool,
    pub theme: Theme,
    pub custom: CustomTheme,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ai_enabled: true,
            keep_previews: true,
            theme: Theme::default(),
            custom: CustomTheme::default(),
        }
    }
}

/// Loads and saves [`Settings`] as JSON in the base directory.
pub struct Store {
    path: PathBuf,
}

impl Store {
    #[must_use]
    pub fn new(base_dir: &Path) -> Self {
        Self {
            path: base_dir.join("settings.json"),
        }
    }

    /// The current settings, or the defaults if the file is missing or unreadable.
    #[must_use]
    pub fn get(&self) -> Settings {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Replaces the stored settings.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn set(&self, settings: &Settings) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, json).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_ai_on_and_dark_theme() {
        let s = Settings::default();
        assert!(s.ai_enabled);
        assert_eq!(s.theme, Theme::Dark);
    }

    #[test]
    fn missing_file_falls_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("cw-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(&dir);
        assert_eq!(store.get(), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_then_get_round_trips_including_custom_theme() {
        let dir = std::env::temp_dir().join(format!("cw-settings-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::new(&dir);
        let s = Settings {
            ai_enabled: false,
            theme: Theme::Custom,
            custom: CustomTheme {
                accent: "#ff8800".to_string(),
                ..CustomTheme::default()
            },
            ..Settings::default()
        };
        store.set(&s).expect("save");
        assert_eq!(store.get(), s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
