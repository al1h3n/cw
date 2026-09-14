//! The teacher's console binary.
//!
//! ```text
//! cowatcher-console id                       show this console's id and endpoint key
//! cowatcher-console pair                     show a pairing code and enrol one device
//! cowatcher-console devices                  list paired devices
//! cowatcher-console watch <agent-key> [n] [dir]
//!                                            pull n thumbnails from a paired agent into dir
//! cowatcher-console act <agent-key> <action> [delay]
//!                                            lock-screen, shutdown, reboot, log-off, cancel-shutdown
//! cowatcher-console version
//! ```
//!
//! State lives in `%LOCALAPPDATA%\co-watcher\console`, or the directory in `COWATCHER_DIR`.
//! The Tauri/Svelte grid UI replaces this CLI in a later step; the protocol underneath is the same.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // no console window in release

mod gui;
mod i18n;
mod manager;
mod room;

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
    // No arguments: this is a teacher double-clicking the app, so open the window.
    if args.is_empty() {
        return match gui::run(data_dir()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::FAILURE
            }
        };
    }
    // Built as a GUI app so double-clicking shows no black box; attaching to the terminal that
    // launched us is what makes the subcommands print anything.
    platform::console::attach_to_parent();
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
        Some("listen") => block_on(cmd_listen(rest)),
        Some("act") => block_on(cmd_act(rest)),
        Some("block") => block_on(cmd_block(rest)),
        Some("control") => block_on(cmd_control(rest)),
        _ => Err(
            "usage: cowatcher-console [id|pair|devices|watch|listen|act|block|control|version]  (no arguments opens the window)"
                .into(),
        ),
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

    let room = room::load_or_create(&data_dir())?;
    let welcome = room.welcome()?;
    println!("room        : {}", room.name);
    let peer = net::console_accept_pairing(&endpoint, &mut session, &mut trust, &welcome)
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

    std::fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;

    let (endpoint, mut session) = connect_paired(agent_key).await?;
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

/// Listens to a paired PC and plays the sound here, reporting what arrived.
async fn cmd_listen(args: Vec<String>) -> Result<(), String> {
    let Some(agent) = args.first() else {
        return Err("usage: cowatcher-console listen <agent-endpoint-key> [seconds]".into());
    };
    let agent_key: iroh::EndpointId = agent
        .parse()
        .map_err(|_| "invalid agent endpoint key".to_string())?;
    let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);

    let (endpoint, mut session) = connect_paired(agent_key).await?;

    let Some(format) = session.set_audio(true).await.map_err(|e| e.to_string())? else {
        return Err("that PC cannot share audio (no sound card, or capture was refused)".into());
    };
    println!(
        "listening to {} at {} Hz mono — play something on that PC",
        session.peer().device_id,
        format.sample_rate
    );

    let playback = media::audio::AudioPlayback::start(format.sample_rate)
        .map_err(|e| format!("no speakers on this PC: {e}"))?;
    let chunk = format.sample_rate / 5; // 200 ms
    let queue_cap = (format.sample_rate * 3 / 5) as usize;
    let mut total = 0usize;
    let mut loudest = 0i16;

    for _ in 0..(seconds * 5) {
        let samples = session
            .request_audio(chunk)
            .await
            .map_err(|e| e.to_string())?;
        total += samples.len();
        loudest = loudest.max(
            samples
                .iter()
                .copied()
                .map(i16::saturating_abs)
                .max()
                .unwrap_or(0),
        );
        playback.push(&samples, queue_cap);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let _ = session.set_audio(false).await;
    session.close();
    endpoint.close().await;
    println!(
        "received {total} samples ({:.1} s of audio), loudest sample {loudest}",
        total as f32 / format.sample_rate as f32
    );
    if loudest == 0 {
        println!("all silence — nothing was playing on that PC, or its output is muted");
    }
    Ok(())
}

/// Opens a control session to a PC this console has already paired with.
async fn connect_paired(
    agent_key: iroh::EndpointId,
) -> Result<(iroh::Endpoint, ControlSession), String> {
    let identity = load_identity()?;
    let trust = TrustStore::load(&trust_path()).map_err(|e| e.to_string())?;
    if !trust.is_trusted(agent_key.as_bytes()) {
        return Err("that device is not paired with this console — run `pair` first".into());
    }
    let endpoint = net::bind(&identity).await.map_err(|e| e.to_string())?;
    let session = ControlSession::connect(
        &endpoint,
        iroh::EndpointAddr::new(agent_key),
        &trust,
        local_hello(&identity),
    )
    .await
    .map_err(|e| e.to_string())?;
    Ok((endpoint, session))
}

/// Makes a paired PC do one thing: lock it, power it off, restart it, sign out, or cancel.
async fn cmd_act(args: Vec<String>) -> Result<(), String> {
    const USAGE: &str = "usage: cowatcher-console act <agent-endpoint-key> \
        <lock-screen|shutdown|reboot|log-off|cancel-shutdown> [delay-seconds]";
    let (Some(agent), Some(name)) = (args.first(), args.get(1)) else {
        return Err(USAGE.into());
    };
    let agent_key: iroh::EndpointId = agent
        .parse()
        .map_err(|_| "invalid agent endpoint key".to_string())?;
    let delay: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let action = manager::parse_action(name, delay).ok_or(USAGE)?;

    let (endpoint, mut session) = connect_paired(agent_key).await?;
    let outcome = session.perform(action).await.map_err(|e| e.to_string())?;
    println!(
        "{} → {}: {outcome:?}",
        session.peer().device_id,
        action.name()
    );
    session.close();
    endpoint.close().await;
    match outcome {
        proto::ActionOutcome::Started { .. } => Ok(()),
        proto::ActionOutcome::Failed(reason) => Err(reason.to_string()),
    }
}

/// Drives a paired PC's mouse and keyboard from the command line, to prove remote control works
/// without needing the window. Moves the pointer in a square, then types some text.
async fn cmd_control(args: Vec<String>) -> Result<(), String> {
    let Some(agent) = args.first() else {
        return Err("usage: cowatcher-console control <agent-endpoint-key> [text-to-type]".into());
    };
    let agent_key: iroh::EndpointId = agent
        .parse()
        .map_err(|_| "invalid agent endpoint key".to_string())?;
    let text = args.get(1).cloned().unwrap_or_default();

    let (endpoint, mut session) = connect_paired(agent_key).await?;

    // Input is refused until control is explicitly granted; prove that first.
    let (applied, refused) = session
        .send_input(vec![proto::InputEvent::MoveTo { x: 100, y: 100 }])
        .await
        .map_err(|e| e.to_string())?;
    println!("before taking control: applied {applied}, refused {refused}");

    let granted = session.set_control(true).await.map_err(|e| e.to_string())?;
    println!("control granted: {granted}");
    if !granted {
        return Err("that PC did not grant control".into());
    }

    // Walk the pointer around a square, in screen fractions (0..=65535).
    let corners = [
        (16_000u16, 16_000u16),
        (49_000, 16_000),
        (49_000, 49_000),
        (16_000, 49_000),
        (32_767, 32_767),
    ];
    let mut total = 0u16;
    for (x, y) in corners {
        let (applied, _) = session
            .send_input(vec![proto::InputEvent::MoveTo { x, y }])
            .await
            .map_err(|e| e.to_string())?;
        total += applied;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    println!("pointer moves applied: {total}");

    if !text.is_empty() {
        // Click where the pointer ended up first. Typing goes to whatever the student PC has
        // focused, so a real teacher clicks the window they mean — and this exercises buttons too.
        session
            .send_input(vec![
                proto::InputEvent::Button {
                    button: proto::PointerButton::Left,
                    down: true,
                },
                proto::InputEvent::Button {
                    button: proto::PointerButton::Left,
                    down: false,
                },
            ])
            .await
            .map_err(|e| e.to_string())?;
        tokio::time::sleep(Duration::from_millis(300)).await;

        let events: Vec<proto::InputEvent> = text.chars().map(proto::InputEvent::Text).collect();
        let (typed, _) = session
            .send_input(events)
            .await
            .map_err(|e| e.to_string())?;
        println!("characters typed: {typed}");
    }

    let granted = session
        .set_control(false)
        .await
        .map_err(|e| e.to_string())?;
    println!("control released: {}", !granted);
    session.close();
    endpoint.close().await;
    Ok(())
}

/// Sets the blocklist on one paired PC directly (an empty list clears it), for testing without the
/// window. The window pushes the room-wide list to every PC instead.
async fn cmd_block(args: Vec<String>) -> Result<(), String> {
    let Some(agent) = args.first() else {
        return Err("usage: cowatcher-console block <agent-endpoint-key> [prog1 prog2 ...]".into());
    };
    let agent_key: iroh::EndpointId = agent
        .parse()
        .map_err(|_| "invalid agent endpoint key".to_string())?;
    let programs: Vec<String> = args.get(1..).unwrap_or_default().to_vec();

    let (endpoint, mut session) = connect_paired(agent_key).await?;
    let (rules, closed) = session
        .set_blocklist(programs)
        .await
        .map_err(|e| e.to_string())?;
    session.close();
    endpoint.close().await;
    println!("blocklist set: {rules} rule(s) in force");
    if closed.is_empty() {
        println!("nothing was running that had to be closed");
    } else {
        println!("closed on the spot: {}", closed.join(", "));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
