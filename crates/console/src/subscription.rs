//! A placeholder for a hosted **Co-watcher subscription** with a web dashboard.
//!
//! The product will offer a paid tier managed from a web dashboard (fleet analytics, licences, Pro
//! features — see AGENTS.md D1). The dashboard's endpoint does not exist yet, so this stores only what
//! the teacher enters — the dashboard URL and a licence key (sealed with DPAPI, like the AI key) — and
//! opens the dashboard in a browser. When the real endpoint is known, licence *verification* slots in
//! behind [`Store::view`] without changing the UI or the stored shape.

use std::path::{Path, PathBuf};

/// What the subscription card shows (never the licence key itself).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubscriptionView {
    /// The dashboard URL the teacher configured (empty until set).
    pub dashboard_url: String,
    /// Whether a licence key is stored.
    pub has_license: bool,
    /// The plan label to show. A placeholder until the endpoint can verify it: `free` with no key,
    /// `custom` (pending verification) once a key is entered.
    pub plan: String,
}

/// Stores the dashboard URL (JSON) and the licence key (DPAPI-protected bytes).
pub struct Store {
    config_path: PathBuf,
    key_path: PathBuf,
}

impl Store {
    /// Uses `subscription.json` and `subscription.key` in the Console data directory.
    #[must_use]
    pub fn new(data_dir: &Path) -> Self {
        Self {
            config_path: data_dir.join("subscription.json"),
            key_path: data_dir.join("subscription.key"),
        }
    }

    /// The stored dashboard URL, or an empty string.
    #[must_use]
    fn dashboard_url(&self) -> String {
        std::fs::read_to_string(&self.config_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| {
                v.get("dashboard_url")
                    .and_then(|u| u.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_default()
    }

    /// The current subscription state for the UI.
    #[must_use]
    pub fn view(&self) -> SubscriptionView {
        let has_license = self.key_path.exists();
        SubscriptionView {
            dashboard_url: self.dashboard_url(),
            has_license,
            plan: if has_license { "custom" } else { "free" }.to_string(),
        }
    }

    /// Saves the dashboard URL and, when `license` is `Some`, the licence key (`Some("")` clears it).
    ///
    /// # Errors
    /// If the config or key cannot be written.
    pub fn set(&self, dashboard_url: &str, license: Option<String>) -> Result<(), String> {
        let json = serde_json::json!({ "dashboard_url": dashboard_url.trim() });
        std::fs::write(
            &self.config_path,
            serde_json::to_string_pretty(&json).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if let Some(license) = license {
            let license = license.trim();
            if license.is_empty() {
                let _ = std::fs::remove_file(&self.key_path);
            } else {
                let sealed = platform::secret::protect(license.as_bytes())
                    .map_err(|e| e.message().to_string())?;
                std::fs::write(&self.key_path, sealed).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Opens the configured dashboard URL in the default browser.
    ///
    /// # Errors
    /// If no URL is set, or the browser could not be opened.
    pub fn open_dashboard(&self) -> Result<(), String> {
        let url = self.dashboard_url();
        if url.is_empty() {
            return Err("no dashboard URL set yet".into());
        }
        platform::browser::open(&url).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_is_free_without_a_licence_and_custom_with_one() {
        let dir = std::env::temp_dir().join(format!("cw-sub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let store = Store::new(&dir);
        assert_eq!(store.view().plan, "free");
        assert!(store.view().dashboard_url.is_empty());

        store
            .set("https://dash.example/team", None)
            .expect("save url");
        assert_eq!(store.view().dashboard_url, "https://dash.example/team");
        assert!(!store.view().has_license);

        #[cfg(windows)]
        {
            store
                .set("https://dash.example/team", Some("LIC-123".into()))
                .expect("save licence");
            assert!(store.view().has_license);
            assert_eq!(store.view().plan, "custom");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
