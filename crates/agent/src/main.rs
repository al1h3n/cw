//! The student-device agent binary.
//!
//! ```text
//! cowatcher-agent id                       show this device's id and endpoint key
//! cowatcher-agent capture <file> [mon] [w] capture one thumbnail to a JPEG file
//! cowatcher-agent pair <console-id> <code> enrol with a Console that is showing a code
//! cowatcher-agent serve                    serve paired Consoles (screens, audio, power, lock)
//! cowatcher-agent sessions                 list the machine's login sessions
//! cowatcher-agent supervise <prog> [args]  run a program and keep it alive
//! cowatcher-agent version
//! ```
//!
//! State (device key, trust store) lives in `%LOCALAPPDATA%\co-watcher\agent`, or the directory in
//! `COWATCHER_DIR`. The SYSTEM service and per-session helper arrive in Phase 1.4b.

mod audit;
mod capture_source;
mod supervisor;

use std::{
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::Duration,
};

use capture_source::ScreenCapture;
use net::{Identity, PairingCode, TrustStore};
use proto::{Capabilities, Role};
use supervisor::{RestartPolicy, supervise};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("version") => {
            println!(
                "{} agent {}",
                proto::PRODUCT_NAME,
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        Some("id") => report(cmd_id()),
        Some("capture") => report(cmd_capture(rest)),
        Some("pair") => report(block_on(cmd_pair(rest.to_vec()))),
        Some("serve") => report(block_on(cmd_serve())),
        Some("sessions") => report(cmd_sessions()),
        Some("supervise") => report(cmd_supervise(rest)),
        Some("install" | "uninstall" | "run" | "helper") => {
            eprintln!(
                "'{}' arrives in Phase 1.4b (Windows service + session helper).",
                args[0]
            );
            ExitCode::FAILURE
        }
        _ => {
            eprintln!("usage: cowatcher-agent <id|capture|pair|serve|sessions|supervise|version>");
            ExitCode::FAILURE
        }
    }
}

/// Prints an error and maps it to an exit code.
fn report(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Runs an async command on a multi-threaded runtime (iroh needs one).
fn block_on<F: std::future::Future<Output = Result<(), String>>>(fut: F) -> Result<(), String> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(fut)
}

/// Where this device keeps its key and trust store.
fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("COWATCHER_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("co-watcher").join("agent")
}

fn load_identity() -> Result<Identity, String> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    Identity::load_or_create(&dir.join("device.key")).map_err(|e| e.to_string())
}

fn audit_path() -> PathBuf {
    data_dir().join("audit.log")
}

fn trust_path() -> PathBuf {
    data_dir().join("trust.bin")
}

fn cmd_id() -> Result<(), String> {
    let identity = load_identity()?;
    println!("device id   : {}", identity.device_id());
    println!("endpoint key: {}", identity.public_key());
    println!("state dir   : {}", data_dir().display());
    let trust = TrustStore::load(&trust_path()).map_err(|e| e.to_string())?;
    println!("paired with : {} console(s)", trust.len());
    Ok(())
}

fn cmd_capture(args: &[String]) -> Result<(), String> {
    let Some(file) = args.first() else {
        return Err("usage: cowatcher-agent capture <file.jpg> [monitor] [max_width]".into());
    };
    let monitor = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0u8);
    let max_width = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(320u16);

    let capture = ScreenCapture::new(&audit_path()).map_err(|e| e.to_string())?;
    println!("monitors: {}", capture.monitor_count());
    let started = std::time::Instant::now();
    let jpeg = net::AgentDevice::capture_thumbnail(&capture, monitor, max_width)
        .map_err(|e| e.to_string())?;
    let elapsed = started.elapsed();
    std::fs::write(Path::new(file), &jpeg).map_err(|e| format!("write {file}: {e}"))?;
    println!("wrote {} ({} bytes) in {elapsed:?}", file, jpeg.len());
    Ok(())
}

async fn cmd_pair(args: Vec<String>) -> Result<(), String> {
    let (Some(console), Some(code)) = (args.first(), args.get(1)) else {
        return Err("usage: cowatcher-agent pair <console-endpoint-key> <6-digit-code>".into());
    };
    let console: iroh::EndpointId = console
        .parse()
        .map_err(|_| "invalid console endpoint key".to_string())?;
    let code: PairingCode = code
        .parse()
        .map_err(|e: net::PairingCodeParseError| e.to_string())?;

    let identity = load_identity()?;
    let endpoint = net::bind(&identity).await.map_err(|e| e.to_string())?;
    let mut trust = TrustStore::load(&trust_path()).map_err(|e| e.to_string())?;

    println!("pairing with console {console}...");
    let peer = net::agent_request_pairing(
        &endpoint,
        iroh::EndpointAddr::new(console),
        code,
        &mut trust,
    )
    .await
    .map_err(|e| e.to_string())?;
    trust.save(&trust_path()).map_err(|e| e.to_string())?;
    println!("paired with console {} — trusted and saved", peer.device_id);
    endpoint.close().await;
    Ok(())
}

async fn cmd_serve() -> Result<(), String> {
    let identity = load_identity()?;
    let trust = TrustStore::load(&trust_path()).map_err(|e| e.to_string())?;
    if trust.is_empty() {
        return Err(
            "no paired console yet — run `cowatcher-agent pair <console-key> <code>` first".into(),
        );
    }
    let capture = ScreenCapture::new(&audit_path()).map_err(|e| e.to_string())?;
    // Fail loudly at start-up rather than on the teacher's first click.
    if let Err(err) = platform::power::enable_shutdown_privilege() {
        eprintln!("warning: power actions will be refused: {err}");
    }
    let endpoint = net::bind(&identity).await.map_err(|e| e.to_string())?;
    endpoint.online().await;

    println!("{} agent serving", proto::PRODUCT_NAME);
    println!("device id   : {}", identity.device_id());
    println!("endpoint key: {}", identity.public_key());
    println!("monitors    : {}", capture.monitor_count());
    println!("trusted     : {} console(s). Ctrl+C to stop.", trust.len());

    let local = net::LocalHello {
        role: Role::Agent,
        device_id: identity.device_id(),
        capabilities: Capabilities::SCREEN_CAPTURE
            .union(Capabilities::AUDIO)
            .union(Capabilities::LOCK)
            .union(Capabilities::POWER),
    };
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            session = net::ControlSession::accept(&endpoint, &trust, local) => match session {
                Ok(session) => {
                    println!("console {} connected", session.peer().device_id);
                    if let Err(err) = session.serve(&capture).await {
                        eprintln!("session ended: {err}");
                    } else {
                        println!("console disconnected");
                    }
                }
                Err(err) => eprintln!("rejected a connection: {err}"),
            },
        }
    }
    endpoint.close().await;
    Ok(())
}

fn cmd_sessions() -> Result<(), String> {
    let sessions = platform::session::list_sessions().map_err(|e| e.to_string())?;
    println!("{} session(s):", sessions.len());
    for s in sessions {
        println!(
            "  id={:<3} {:<14} {}",
            s.id,
            s.station,
            if s.active { "ACTIVE" } else { "-" }
        );
    }
    Ok(())
}

fn cmd_supervise(args: &[String]) -> Result<(), String> {
    let Some((program, rest)) = args.split_first() else {
        return Err("usage: cowatcher-agent supervise <program> [args...]".into());
    };
    let (program, rest) = (program.clone(), rest.to_vec());
    let make = move || {
        let mut command = Command::new(&program);
        command.args(&rest);
        command
    };
    // Runs until the process is killed, like a service; the SCM stop control drives this in 1.4b.
    let starts = supervise(
        make,
        RestartPolicy::default(),
        &(|| false),
        Duration::from_millis(200),
    );
    println!("supervisor stopped after {starts} start(s)");
    Ok(())
}
