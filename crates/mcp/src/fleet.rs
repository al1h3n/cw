//! The fleet client the MCP tools drive.
//!
//! This is the same job the Console's `DeviceManager` does — load the console identity and the set of
//! paired devices, then dial one by public key over iroh — but reduced to what an AI agent needs: a
//! one-shot connection per call. It depends only on the OS-agnostic `net` and `proto` crates, so no
//! Windows code leaks into it; every remote effect goes through the same signed, typed
//! [`net::ControlSession`] the teacher's UI uses, and therefore through the same closed action enum.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use net::{ControlSession, Identity, LocalHello, TrustStore};
use proto::{Capabilities, DeviceId, Role};

/// One paired device, as the MCP presents it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceInfo {
    /// The six-character device id a teacher (or the AI) names it by.
    pub device_id: String,
    /// The teacher's own label for the PC, if one was set.
    pub name: Option<String>,
}

/// Owns the console identity and trust store, and lends out connections to paired devices.
pub struct Fleet {
    data_dir: PathBuf,
    identity: Identity,
    trust: TrustStore,
    names: BTreeMap<String, String>,
    endpoint: tokio::sync::Mutex<Option<iroh::Endpoint>>,
}

impl Fleet {
    /// Loads the console identity, trust store and device names from the Console's data directory —
    /// the same files the teacher's Console reads, so the MCP acts as that console.
    ///
    /// # Errors
    /// If the identity or trust store cannot be read (e.g. the Console has never been run here).
    pub fn load(dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(dir)?;
        let identity = Identity::load_or_create(&dir.join("device.key"))
            .map_err(|e| anyhow::anyhow!("read console identity: {e}"))?;
        let trust = TrustStore::load(&dir.join("trust.bin"))
            .map_err(|e| anyhow::anyhow!("read paired devices: {e}"))?;
        let names = load_names(&dir.join("names.txt"));
        Ok(Self {
            data_dir: dir.to_path_buf(),
            identity,
            trust,
            names,
            endpoint: tokio::sync::Mutex::new(None),
        })
    }

    /// This console's own six-character id (useful for the AI to identify itself in logs).
    #[must_use]
    pub fn console_id(&self) -> String {
        self.identity.device_id().to_string()
    }

    /// Every paired device, newest membership order irrelevant — sorted by id for a stable listing.
    #[must_use]
    pub fn devices(&self) -> Vec<DeviceInfo> {
        let mut out: Vec<DeviceInfo> = self
            .trust
            .keys()
            .map(|key| {
                let id = DeviceId::from_public_key(key).to_string();
                let name = self.names.get(&id).cloned();
                DeviceInfo {
                    device_id: id,
                    name,
                }
            })
            .collect();
        out.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        out
    }

    /// Files already saved by the teacher's Console, confined to its known output folders.
    pub fn external_files(&self) -> Vec<serde_json::Value> {
        let mut files = Vec::new();
        for kind in ["screenshots", "recordings", "downloads", "sent"] {
            scan_saved_files(&self.data_dir.join(kind), kind, None, &mut files);
        }
        files
    }

    /// Reads a saved screenshot by its plain name for an MCP image response.
    pub fn external_preview(&self, name: &str) -> Result<Vec<u8>, String> {
        if Path::new(name).file_name().and_then(|n| n.to_str()) != Some(name)
            || !name.to_ascii_lowercase().ends_with(".jpg")
        {
            return Err("invalid screenshot name".into());
        }
        let path = self.data_dir.join("screenshots").join(name);
        let meta = path.symlink_metadata().map_err(|e| e.to_string())?;
        if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 16 * 1024 * 1024 {
            return Err("screenshot cannot be previewed".into());
        }
        std::fs::read(path).map_err(|e| e.to_string())
    }

    /// Reveals a saved file after resolving it from the Console's own output folders.
    pub fn external_show(&self, path: &str) -> Result<(), String> {
        if !self
            .external_files()
            .iter()
            .any(|file| file["path"] == path)
        {
            return Err("saved file was not found".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            std::process::Command::new("explorer.exe")
                .arg(format!("/select,\"{path}\""))
                .creation_flags(0x0800_0000)
                .spawn()
                .map_err(|e| format!("open file location: {e}"))?;
        }
        Ok(())
    }

    /// The stored public key for a device id, if it is paired. Ids are compared case-insensitively so
    /// an agent that lower-cases an id still resolves it.
    fn key_for(&self, device_id: &str) -> Option<[u8; 32]> {
        let wanted = device_id.trim().to_ascii_uppercase();
        self.trust
            .keys()
            .find(|key| DeviceId::from_public_key(key).to_string() == wanted)
            .copied()
    }

    /// Binds the shared iroh endpoint once and reuses it for every dial.
    async fn endpoint(&self) -> anyhow::Result<iroh::Endpoint> {
        let mut slot = self.endpoint.lock().await;
        if let Some(endpoint) = slot.as_ref() {
            return Ok(endpoint.clone());
        }
        let endpoint = net::bind(&self.identity)
            .await
            .map_err(|e| anyhow::anyhow!("open network endpoint: {e}"))?;
        *slot = Some(endpoint.clone());
        Ok(endpoint)
    }

    /// Dials one paired device and returns a live control session, or an error a caller can show the
    /// AI verbatim (unknown device, powered off, unreachable).
    ///
    /// # Errors
    /// The device is not paired, or the connection could not be established.
    pub async fn connect(&self, device_id: &str) -> Result<ControlSession, String> {
        let key = self
            .key_for(device_id)
            .ok_or_else(|| format!("no paired device with id {device_id}"))?;
        let endpoint = self.endpoint().await.map_err(|e| e.to_string())?;
        let local = LocalHello {
            role: Role::Console,
            device_id: self.identity.device_id(),
            capabilities: Capabilities::EMPTY,
        };
        let agent = iroh::EndpointId::from_bytes(&key).map_err(|e| e.to_string())?;
        ControlSession::connect(
            &endpoint,
            iroh::EndpointAddr::new(agent),
            &self.trust,
            local,
        )
        .await
        .map_err(|e| format!("could not reach {device_id}: {e} (is the PC on and online?)"))
    }
}

fn scan_saved_files(
    folder: &Path,
    kind: &str,
    device_id: Option<&str>,
    files: &mut Vec<serde_json::Value>,
) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        if files.len() >= 2000 {
            return;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if file_type.is_dir() && device_id.is_none() {
            scan_saved_files(&path, kind, Some(&name), files);
        } else if file_type.is_file()
            && let Ok(meta) = entry.metadata()
        {
            let id = device_id.map(str::to_owned).or_else(|| {
                (kind == "screenshots")
                    .then(|| name.split_once('-').map(|(id, _)| id.to_owned()))
                    .flatten()
            });
            files.push(serde_json::json!({
                "kind": kind,
                "device_id": id,
                "name": name,
                "path": path.display().to_string(),
                "bytes": meta.len(),
            }));
        }
    }
}

/// Parses `names.txt` (`id = name` per line), matching the Console's own format.
fn load_names(path: &Path) -> BTreeMap<String, String> {
    let mut names = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return names;
    };
    for line in text.lines() {
        if let Some((id, name)) = line.split_once('=') {
            let id = id.trim().to_string();
            let name = name.trim().to_string();
            if !id.is_empty() && !name.is_empty() {
                names.insert(id, name);
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::load_names;

    #[test]
    fn names_file_is_parsed_into_id_to_name() {
        let dir = std::env::temp_dir().join(format!("cw-mcp-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("names.txt");
        std::fs::write(
            &path,
            "K7M2Q9 = Row 3, seat 2\n\nBLANK =\n= orphan\nAB12CD = Front\n",
        )
        .expect("write names");
        let names = load_names(&path);
        assert_eq!(
            names.get("K7M2Q9").map(String::as_str),
            Some("Row 3, seat 2")
        );
        assert_eq!(names.get("AB12CD").map(String::as_str), Some("Front"));
        // A blank name or a blank id is dropped rather than stored.
        assert!(!names.contains_key("BLANK"));
        assert_eq!(names.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_names_file_is_simply_empty() {
        let names = load_names(std::path::Path::new("does-not-exist-cw-mcp.txt"));
        assert!(names.is_empty());
    }
}
