//! The teacher's console binary.
//!
//! ```text
//! cowatcher-console id                       show this console's id and endpoint key
//! cowatcher-console pair                     show a pairing code and enrol one device
//! cowatcher-console devices                  list paired devices
//! cowatcher-console watch <agent-key> [n] [dir]
//!                                            pull n thumbnails from a paired agent into dir
//! cowatcher-console version
//! ```
//!
//! State lives in `%LOCALAPPDATA%\co-watcher\console`, or the directory in `COWATCHER_DIR`.
//! The Tauri/Svelte grid UI replaces this CLI in a later step; the protocol underneath is the same.

use std::{
    path::PathBuf,
    process::ExitCode,
    time::{Duration, Instant},
};

use net::{ControlSession, Identity, LocalHello, PairingCode, PairingSession, TrustStore};
use proto::{Capabilities, Role};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default().to_vec();
    let result = match args.first().map(String::as_str) {
        Some("version") => {
            println!(
                "{} console {}",
                proto::PRODUCT_NAME,
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
        Some("id") => cmd_id(),
        Some("devices") => cmd_devices(),
        Some("pair") => block_on(cmd_pair()),
        Some("watch") => block_on(cmd_watch(rest)),
        _ => Err("usage: cowatcher-console <id|pair|devices|watch|version>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn block_on<F: std::future::Future<Output = Result<(), String>>>(fut: F) -> Result<(), String> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(fut)
}

fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("COWATCHER_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("co-watcher").join("console")
}

fn trust_path() -> PathBuf {
    data_dir().join("trust.bin")
}

fn load_identity() -> Result<Identity, String> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    Identity::load_or_create(&dir.join("device.key")).map_err(|e| e.to_string())
}

fn local_hello(identity: &Identity) -> LocalHello {
    LocalHello {
        role: Role::Console,
        device_id: identity.device_id(),
        capabilities: Capabilities::EMPTY,
    }
}

fn cmd_id() -> Result<(), String> {
    let identity = load_identity()?;
    println!("device id   : {}", identity.device_id());
    println!("endpoint key: {}", identity.public_key());
    println!("state dir   : {}", data_dir().display());
    Ok(())
}

fn cmd_devices() -> Result<(), String> {
    let trust = TrustStore::load(&trust_path()).map_err(|e| e.to_string())?;
    println!("{} paired device(s):", trust.len());
    for key in trust.keys() {
        println!(
            "  device {}  key {}",
            proto::DeviceId::from_public_key(key),
            hex(key)
        );
    }
    Ok(())
}

/// Shows a code and enrols one device that dials in with it.
async fn cmd_pair() -> Result<(), String> {
    let identity = load_identity()?;
    let mut trust = TrustStore::load(&trust_path()).map_err(|e| e.to_string())?;
    let endpoint = net::bind(&identity).await.map_err(|e| e.to_string())?;
    endpoint.online().await;

    let code = PairingCode::generate();
    let mut session = PairingSession::new(code, net::endpoint::now_ms());
    println!("On the student PC run:\n");
    println!(
        "    cowatcher-agent pair {} {}\n",
        identity.public_key(),
        code
    );
    println!("code {code} is valid for 5 minutes; waiting...");

    let peer = net::console_accept_pairing(&endpoint, &mut session, &mut trust)
        .await
        .map_err(|e| e.to_string())?;
    trust.save(&trust_path()).map_err(|e| e.to_string())?;
    println!("paired device {} — trusted and saved", peer.device_id);
    endpoint.close().await;
    Ok(())
}

/// Pulls thumbnails from a paired agent and writes them as JPEG files.
async fn cmd_watch(args: Vec<String>) -> Result<(), String> {
    let Some(agent) = args.first() else {
        return Err("usage: cowatcher-console watch <agent-endpoint-key> [count] [out-dir]".into());
    };
    let agent_key: iroh::EndpointId = agent
        .parse()
        .map_err(|_| "invalid agent endpoint key".to_string())?;
    let count: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let out_dir = args
        .get(2)
        .map_or_else(|| PathBuf::from("thumbnails"), PathBuf::from);

    let identity = load_identity()?;
    let trust = TrustStore::load(&trust_path()).map_err(|e| e.to_string())?;
    if !trust.is_trusted(agent_key.as_bytes()) {
        return Err("that device is not paired with this console — run `pair` first".into());
    }
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;

    let endpoint = net::bind(&identity).await.map_err(|e| e.to_string())?;
    let mut session = ControlSession::connect(
        &endpoint,
        iroh::EndpointAddr::new(agent_key),
        &trust,
        local_hello(&identity),
    )
    .await
    .map_err(|e| e.to_string())?;
    let peer = session.peer();
    println!(
        "connected to device {} (role {:?})",
        peer.device_id, peer.role
    );

    // One frame per second is the thumbnail-grid pace; the agent captures only when asked.
    for index in 0..count {
        let started = Instant::now();
        let jpeg = session
            .request_thumbnail(0, 320)
            .await
            .map_err(|e| e.to_string())?;
        let file = out_dir.join(format!("{}-{index:03}.jpg", peer.device_id));
        std::fs::write(&file, &jpeg).map_err(|e| format!("write {}: {e}", file.display()))?;
        println!(
            "  {} ({} bytes, {:?})",
            file.display(),
            jpeg.len(),
            started.elapsed()
        );
        if index + 1 < count {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    session.close();
    endpoint.close().await;
    println!("done: {count} thumbnail(s) in {}", out_dir.display());
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
