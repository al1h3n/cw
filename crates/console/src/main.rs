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
//! cowatcher-console wake <MAC>                 broadcast a Wake-on-LAN packet
//! cowatcher-console version
//! ```
//!
//! State lives in `%LOCALAPPDATA%\co-watcher\console`, or the directory in `COWATCHER_DIR`.
//! The Tauri/Svelte grid UI replaces this CLI in a later step; the protocol underneath is the same.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // no console window in release

mod ai;
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
        Some("apps") => block_on(cmd_apps(rest)),
        Some("record") => block_on(cmd_record(rest)),
        Some("broadcast") => block_on(cmd_broadcast(rest)),
        Some("wake") => cmd_wake(&rest),
        Some("stream") => block_on(cmd_stream(rest)),
        _ => Err(
            "usage: cowatcher-console [id|pair|devices|watch|listen|act|block|control|apps|record|broadcast|wake|stream|version]  (no arguments opens the window)"
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

/// Streams a paired PC's screen as H.264 for a few seconds and reports what arrived.
///
/// Decodes every frame, so this proves the whole pipeline — capture, resize, encode, QUIC
/// uni-stream, decode — not merely that bytes moved.
async fn cmd_stream(args: Vec<String>) -> Result<(), String> {
    let Some(agent) = args.first() else {
        return Err(
            "usage: cowatcher-console stream <agent-endpoint-key> [seconds] [width] [height] [fps] [kbps]"
                .into(),
        );
    };
    let agent_key: iroh::EndpointId = agent
        .parse()
        .map_err(|_| "invalid agent endpoint key".to_string())?;
    let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let settings = proto::VideoSettings {
        width: args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1280),
        height: args.get(3).and_then(|s| s.parse().ok()).unwrap_or(720),
        fps: args.get(4).and_then(|s| s.parse().ok()).unwrap_or(30),
        kbps: args.get(5).and_then(|s| s.parse().ok()).unwrap_or(2_000),
    };

    let (endpoint, mut session) = connect_paired(agent_key).await?;
    let actual = session
        .start_stream(0, settings)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "asked for {}x{} @ {} fps; streaming {}x{} @ {} fps at {} kbit/s",
        settings.width,
        settings.height,
        settings.fps,
        actual.width,
        actual.height,
        actual.fps,
        actual.kbps
    );

    let mut video = session.accept_video().await.map_err(|e| e.to_string())?;
    let mut decoder = media::h264::Decoder::new().map_err(|e| e.to_string())?;
    let started = Instant::now();
    let deadline = started + Duration::from_secs(seconds);
    let (mut packets, mut frames, mut bytes) = (0u32, 0u32, 0usize);
    let mut last_size = (0u32, 0u32);

    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let Ok(next) = tokio::time::timeout(remaining, video.next_frame()).await else {
            break; // ran out of time
        };
        match next.map_err(|e| e.to_string())? {
            Some(packet) => {
                packets += 1;
                bytes += packet.len();
                if let Some((_, w, h)) = decoder.decode(&packet).map_err(|e| e.to_string())? {
                    frames += 1;
                    last_size = (w, h);
                }
            }
            None => break, // the agent closed the stream
        }
    }

    let elapsed = started.elapsed().as_secs_f32().max(0.001);
    println!("received {packets} packet(s), decoded {frames} frame(s) in {elapsed:.1}s");
    println!(
        "  {:.1} fps, {:.0} kbit/s, last decoded frame {}x{}",
        frames as f32 / elapsed,
        (bytes as f32 * 8.0 / 1000.0) / elapsed,
        last_size.0,
        last_size.1
    );
    if frames == 0 {
        return Err("no frames decoded — the stream did not work".into());
    }

    session.stop_stream().await.map_err(|e| e.to_string())?;
    session.close();
    endpoint.close().await;
    Ok(())
}

/// Broadcasts a Wake-on-LAN packet for a MAC address, to wake a switched-off PC on this LAN.
fn cmd_wake(args: &[String]) -> Result<(), String> {
    let Some(mac) = args.first() else {
        return Err("usage: cowatcher-console wake <AA:BB:CC:DD:EE:FF>".into());
    };
    let mac = platform::wol::MacAddress::parse(mac).map_err(|e| e.to_string())?;
    platform::wol::wake(mac).map_err(|e| e.to_string())?;
    println!("sent a wake packet to {}", mac.to_hex());
    println!("(the PC wakes only if its BIOS/UEFI and network card have Wake-on-LAN enabled)");
    Ok(())
}

/// Broadcasts this console's screen to a paired PC for a few seconds.
async fn cmd_broadcast(args: Vec<String>) -> Result<(), String> {
    let Some(agent) = args.first() else {
        return Err(
            "usage: cowatcher-console broadcast <agent-endpoint-key> [seconds] [width]".into(),
        );
    };
    let agent_key: iroh::EndpointId = agent
        .parse()
        .map_err(|_| "invalid agent endpoint key".to_string())?;
    let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let width: u16 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1280);

    // Capture this console's own screen. Sending a downscaled frame is deliberate: a student's
    // screen is rarely bigger than the teacher's, and the window stretches whatever it is given.
    let mut capturer = media::ThumbnailCapturer::new().map_err(|e| e.to_string())?;

    let (endpoint, mut session) = connect_paired(agent_key).await?;
    println!(
        "broadcasting this screen to {} for {seconds}s",
        session.peer().device_id
    );

    // Roughly 5 frames a second is enough for slides and a demonstration, and it is what the
    // per-frame JPEG approach comfortably sustains.
    let frames = seconds * 5;
    let mut sent = 0u32;
    for _ in 0..frames {
        let jpeg = capturer.capture_jpeg(0, width).map_err(|e| e.to_string())?;
        let (showing, problem) = session
            .show_broadcast(jpeg, false)
            .await
            .map_err(|e| e.to_string())?;
        if !showing {
            session.close();
            endpoint.close().await;
            return Err(format!("that PC did not show the broadcast: {problem}"));
        }
        sent += 1;
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let (showing, _) = session.stop_broadcast().await.map_err(|e| e.to_string())?;
    println!("sent {sent} frame(s); still showing: {showing}");
    session.close();
    endpoint.close().await;
    Ok(())
}

/// Records a paired PC's screen for a few seconds at a chosen size and frame rate.
async fn cmd_record(args: Vec<String>) -> Result<(), String> {
    let Some(agent) = args.first() else {
        return Err(
            "usage: cowatcher-console record <agent-endpoint-key> [seconds] [width] [height] [fps]"
                .into(),
        );
    };
    let agent_key: iroh::EndpointId = agent
        .parse()
        .map_err(|_| "invalid agent endpoint key".to_string())?;
    let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    let width: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1920);
    let height: u32 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(1080);
    let fps: u32 = args.get(4).and_then(|s| s.parse().ok()).unwrap_or(30);

    let options = proto::RecordOptions {
        max_width: width,
        max_height: height,
        fps,
        ..proto::RecordOptions::default()
    };
    let (endpoint, mut session) = connect_paired(agent_key).await?;
    let started = session
        .start_recording(0, options)
        .await
        .map_err(|e| e.to_string())?;
    if !started.active {
        session.close();
        endpoint.close().await;
        return Err(format!("recording did not start: {}", started.problem));
    }
    println!(
        "asked for {width}x{height} @ {fps} fps; recording {}x{} @ {} fps into {}",
        started.width, started.height, started.fps, started.file
    );

    tokio::time::sleep(Duration::from_secs(seconds)).await;
    let mid = session
        .recording_status()
        .await
        .map_err(|e| e.to_string())?;
    println!("after {seconds}s: {} frames written", mid.frames);

    let done = session.stop_recording().await.map_err(|e| e.to_string())?;
    println!(
        "stopped: {} frames ({:.1} s of video at {} fps)",
        done.frames,
        done.frames as f32 / done.fps.max(1) as f32,
        done.fps
    );
    if !done.problem.is_empty() {
        println!("note: {}", done.problem);
    }

    for recording in session.list_recordings().await.map_err(|e| e.to_string())? {
        println!("  {}  {} KB", recording.file, recording.bytes / 1024);
    }
    session.close();
    endpoint.close().await;
    Ok(())
}

/// Lists what a paired PC can start and what is running, and optionally starts or closes one.
async fn cmd_apps(args: Vec<String>) -> Result<(), String> {
    const USAGE: &str = "usage: cowatcher-console apps <agent-endpoint-key> [launch <id> | close <pid> | find <text>]";
    let Some(agent) = args.first() else {
        return Err(USAGE.into());
    };
    let agent_key: iroh::EndpointId = agent
        .parse()
        .map_err(|_| "invalid agent endpoint key".to_string())?;

    let (endpoint, mut session) = connect_paired(agent_key).await?;
    let result = match args.get(1).map(String::as_str) {
        Some("launch") => {
            let id = args
                .get(2)
                .and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok())
                .ok_or("launch needs an app id, e.g. 0x8f2c1a")?;
            let (name, started) = session.launch_app(id).await.map_err(|e| e.to_string())?;
            if started {
                println!("started \"{name}\"");
                Ok(())
            } else {
                Err("that PC has no program with that id".to_string())
            }
        }
        Some("close") => {
            let pid: u32 = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .ok_or("close needs a process id")?;
            let closed = session.close_app(pid).await.map_err(|e| e.to_string())?;
            println!("closed: {closed}");
            Ok(())
        }
        Some("find") => {
            let needle = args.get(2).cloned().unwrap_or_default().to_lowercase();
            let apps = session.request_apps().await.map_err(|e| e.to_string())?;
            for app in apps
                .iter()
                .filter(|a| a.name.to_lowercase().contains(&needle))
            {
                println!("  {:#010x}  {}", app.id, app.name);
            }
            Ok(())
        }
        None => {
            let apps = session.request_apps().await.map_err(|e| e.to_string())?;
            println!("{} program(s) this PC can start:", apps.len());
            for app in apps.iter().take(15) {
                println!("  {:#010x}  {}", app.id, app.name);
            }
            if apps.len() > 15 {
                println!("  ... and {} more", apps.len() - 15);
            }
            let running = session.request_running().await.map_err(|e| e.to_string())?;
            println!("{} closable program(s) running", running.len());
            Ok(())
        }
        _ => Err(USAGE.to_string()),
    };
    session.close();
    endpoint.close().await;
    result
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
