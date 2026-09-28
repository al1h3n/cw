//! Publisher-signed Agent updates. A mirror only hosts bytes; it cannot choose executable code.

use std::{io::Read, path::Path, time::Duration};

use ed25519_dalek::{Signature, VerifyingKey};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[cfg(windows)]
const TASK_NAME: &str = "CowatcherAgentUpdate";

const MAX_MANIFEST: u64 = 16 * 1024;
const MAX_BINARY: u64 = 128 * 1024 * 1024;

#[derive(Clone, Deserialize, Serialize)]
pub struct UpdateConfig {
    pub manifest_url: String,
    pub public_key: String,
}

/// SYSTEM runs the installed executable each night. Its `install` command checks the pinned
/// manifest first and exits without touching the service when there is no new release.
#[cfg(windows)]
pub fn schedule(exe: &Path) -> Result<(), String> {
    use std::{os::windows::process::CommandExt, process::Command};
    let command = format!("\"{}\" update", exe.display());
    let output = Command::new("schtasks.exe")
        .args([
            "/Create", "/F", "/SC", "DAILY", "/ST", "03:00", "/RU", "SYSTEM", "/RL", "HIGHEST",
            "/TN", TASK_NAME, "/TR", &command,
        ])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("create update task: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "create update task: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn schedule(_exe: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
pub fn unschedule() -> Result<(), String> {
    use std::{os::windows::process::CommandExt, process::Command};
    let query = Command::new("schtasks.exe")
        .args(["/Query", "/TN", TASK_NAME])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("inspect update task: {e}"))?;
    if !query.status.success() {
        return Ok(());
    }
    let output = Command::new("schtasks.exe")
        .args(["/Delete", "/F", "/TN", TASK_NAME])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("remove update task: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "remove update task: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn unschedule() -> Result<(), String> {
    Ok(())
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    asset: String,
    sha256: String,
    signature: String,
}

impl UpdateConfig {
    pub fn new(manifest_url: String, public_key: String) -> Result<Self, String> {
        let url = reqwest::Url::parse(&manifest_url).map_err(|e| e.to_string())?;
        if url.scheme() != "https" || url.host().is_none() {
            return Err("update manifest must use HTTPS".into());
        }
        let key = decode_hex::<32>(&public_key)?;
        VerifyingKey::from_bytes(&key).map_err(|_| "invalid update public key")?;
        Ok(Self {
            manifest_url,
            public_key,
        })
    }

    pub fn load(dir: &Path) -> Result<Option<Self>, String> {
        let path = dir.join("updates.json");
        if !path.exists() {
            return Ok(None);
        }
        let config: Self = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Self::new(config.manifest_url, config.public_key).map(Some)
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        secure_update_dir(dir)?;
        let text = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("updates.json"), text).map_err(|e| e.to_string())
    }

    /// Fetches, authenticates, then returns a newer executable in memory. No update bytes are
    /// written to disk until both the Ed25519 signature and SHA-256 digest pass.
    pub fn download_newer(&self, current: &str) -> Result<Option<Vec<u8>>, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| e.to_string())?;
        let response = client
            .get(&self.manifest_url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|e| format!("fetch update manifest: {e}"))?;
        let mut manifest_bytes = Vec::new();
        response
            .take(MAX_MANIFEST + 1)
            .read_to_end(&mut manifest_bytes)
            .map_err(|e| e.to_string())?;
        if manifest_bytes.len() as u64 > MAX_MANIFEST {
            return Err("update manifest is too large".into());
        }
        let manifest: Manifest =
            serde_json::from_slice(&manifest_bytes).map_err(|e| e.to_string())?;
        let version = verify_manifest(&manifest, &self.public_key)?;
        if version <= Version::parse(current).map_err(|e| e.to_string())? {
            return Ok(None);
        }
        let base = reqwest::Url::parse(&self.manifest_url).map_err(|e| e.to_string())?;
        let asset_url = base.join(&manifest.asset).map_err(|e| e.to_string())?;
        if asset_url.scheme() != "https" {
            return Err("update asset must use HTTPS".into());
        }
        let response = client
            .get(asset_url)
            .send()
            .and_then(reqwest::blocking::Response::error_for_status)
            .map_err(|e| format!("fetch update executable: {e}"))?;
        let mut bytes = Vec::new();
        response
            .take(MAX_BINARY + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BINARY {
            return Err("update executable is too large".into());
        }
        if !bytes.starts_with(b"MZ") {
            return Err("update asset is not a Windows executable".into());
        }
        let digest = Sha256::digest(&bytes);
        if digest.as_slice() != decode_hex::<32>(&manifest.sha256)? {
            return Err("update executable SHA-256 mismatch".into());
        }
        Ok(Some(bytes))
    }
}

#[cfg(windows)]
pub fn secure_update_dir(dir: &Path) -> Result<(), String> {
    use std::{os::windows::process::CommandExt, process::Command};
    let output = Command::new("icacls.exe")
        .arg(dir)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "*S-1-5-32-544:(OI)(CI)F",
        ])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("protect update key: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "protect update key: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// The session helper must execute the service image, but a student must not replace it.
#[cfg(windows)]
pub fn secure_binary_dir(dir: &Path) -> Result<(), String> {
    use std::{os::windows::process::CommandExt, process::Command};
    let output = Command::new("icacls.exe")
        .arg(dir)
        .args([
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "*S-1-5-32-544:(OI)(CI)F",
            "*S-1-5-32-545:(OI)(CI)RX",
        ])
        .creation_flags(0x0800_0000)
        .output()
        .map_err(|e| format!("protect agent binary folder: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "protect agent binary folder: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn secure_update_dir(_dir: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(not(windows))]
pub fn secure_binary_dir(_dir: &Path) -> Result<(), String> {
    Ok(())
}

fn verify_manifest(manifest: &Manifest, public_key: &str) -> Result<Version, String> {
    let version = Version::parse(&manifest.version).map_err(|e| e.to_string())?;
    if manifest.asset.is_empty()
        || manifest.asset == "."
        || manifest.asset == ".."
        || manifest.asset.contains(['/', '\\', ':'])
    {
        return Err("update asset must be a plain file name".into());
    }
    let _ = decode_hex::<32>(&manifest.sha256)?;
    let key = VerifyingKey::from_bytes(&decode_hex::<32>(public_key)?)
        .map_err(|_| "invalid update public key")?;
    let signature = Signature::try_from(decode_hex::<64>(&manifest.signature)?.as_slice())
        .map_err(|_| "invalid update signature")?;
    let signed = format!(
        "cowatcher-agent-update-v1\n{}\n{}\n{}\n",
        manifest.version, manifest.asset, manifest.sha256
    );
    key.verify_strict(signed.as_bytes(), &signature)
        .map_err(|_| "update manifest signature does not match the pinned key")?;
    Ok(version)
}

fn decode_hex<const N: usize>(text: &str) -> Result<[u8; N], String> {
    if text.len() != N * 2 || !text.is_ascii() {
        return Err(format!("expected {} hexadecimal characters", N * 2));
    }
    let mut out = [0u8; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16)
            .map_err(|_| "invalid hexadecimal value".to_string())?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn signed_manifest_accepts_only_its_exact_asset_and_hash() {
        let secret = SigningKey::from_bytes(&[7u8; 32]);
        let mut manifest = Manifest {
            version: "1.2.3".into(),
            asset: "cowatcher-agent.exe".into(),
            sha256: "00".repeat(32),
            signature: String::new(),
        };
        let signed = format!(
            "cowatcher-agent-update-v1\n{}\n{}\n{}\n",
            manifest.version, manifest.asset, manifest.sha256
        );
        manifest.signature = secret
            .sign(signed.as_bytes())
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let key: String = secret
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            verify_manifest(&manifest, &key).unwrap(),
            Version::new(1, 2, 3)
        );
        manifest.asset = "other.exe".into();
        assert!(verify_manifest(&manifest, &key).is_err());
        manifest.asset = "../evil.exe".into();
        assert!(verify_manifest(&manifest, &key).is_err());
    }
}
