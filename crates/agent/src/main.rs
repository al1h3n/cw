//! The student-device agent binary.
//!
//! ```text
//! cowatcher-agent id                       show this device's id and endpoint key
//! cowatcher-agent capture <file> [mon] [w] capture one thumbnail to a JPEG file
//! cowatcher-agent pair <console-id> <code> enrol with a Console that is showing a code
//! cowatcher-agent serve                    serve paired Consoles (screens, audio, power, lock)
//! cowatcher-agent room                     show which room this PC is in
//! cowatcher-agent leave <room-password>    take this PC out of its room
//! cowatcher-agent sessions                 list the machine's login sessions
//! cowatcher-agent install                  register the auto-start service (admin)
//! cowatcher-agent uninstall                remove the service (admin)
//! cowatcher-agent status                   is the service installed?
//! cowatcher-agent run                      service entry point (the SCM calls this)
//! cowatcher-agent supervise <prog> [args]  run a program and keep it alive
//! cowatcher-agent version
//! ```
//!
//! State (device key, trust store, room membership) lives machine-wide in
//! `%ProgramData%\co-watcher\agent` so the SYSTEM service, the per-session helper (running as the
//! student) and interactive `pair` all share one identity; `COWATCHER_DIR` overrides it, and an
//! identity from the old per-user `%LOCALAPPDATA%` location is migrated across on first use. The
//! service supervises a per-session `helper` that does the actual capture in the user's session,
//! since a session-0 service cannot see a desktop (Phase 1.4b).

mod audit;
mod blocker;
mod capture_source;
mod membership;
mod record_id;
mod recording;
mod streaming;
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
        Some("wallpaper-selftest") => report(cmd_wallpaper_selftest()),
        Some("room") => report(cmd_room()),
        Some("leave") => report(cmd_leave(rest)),
        Some("supervise") => report(cmd_supervise(rest)),
        Some("install") => report(cmd_install()),
        Some("uninstall") => report(cmd_uninstall()),
        Some("run") => report(cmd_run()),
        Some("status") => report(cmd_status()),
        Some("helper") => report(block_on(cmd_helper(rest))),
        _ => {
            eprintln!(
                "usage: cowatcher-agent <id|capture|pair|serve|sessions|room|leave|install|\
                 uninstall|status|run|supervise|version>"
            );
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

/// Set by the `helper` subcommand so the per-session helper reads exactly the directory the SYSTEM
/// service handed it, whatever the defaults would otherwise pick.
static DIR_OVERRIDE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Forces the data directory for the rest of this process. Call before anything reads [`data_dir`].
fn set_data_dir(dir: PathBuf) {
    let _ = DIR_OVERRIDE.set(dir);
}

/// Where this device keeps its key, trust store and room membership.
///
/// A classroom Agent's identity belongs to the **machine**, not to whoever is logged in: the SYSTEM
/// service, the per-session helper (running as the student) and an interactive `pair` must all read
/// the same key. So the default is machine-wide `%ProgramData%\co-watcher\agent`, readable by every
/// account. `COWATCHER_DIR` (or the `helper` argument) overrides it. If the machine directory cannot
/// be used, we fall back to the old per-user location so nothing hard-breaks. Cached, since it may do
/// a little filesystem work (create + migrate) the first time.
fn data_dir() -> PathBuf {
    if let Some(dir) = DIR_OVERRIDE.get() {
        return dir.clone();
    }
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(choose_data_dir).clone()
}

fn choose_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("COWATCHER_DIR") {
        return PathBuf::from(dir);
    }
    let machine = program_data_dir();
    let legacy = user_local_dir();
    // Already established here: use it.
    if machine.join("device.key").exists() {
        return machine;
    }
    // Try to set the machine-wide directory up, carrying an existing per-user identity across so a PC
    // paired before this change keeps its device id and trust.
    if std::fs::create_dir_all(&machine).is_ok() {
        if let Some(legacy) = &legacy {
            migrate_identity(legacy, &machine);
        }
        if dir_is_writable(&machine) {
            return machine;
        }
    }
    // Could not use the machine directory (permissions): keep working in the per-user one.
    legacy.unwrap_or(machine)
}

fn program_data_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("co-watcher").join("agent")
}

fn user_local_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map(|base| PathBuf::from(base).join("co-watcher").join("agent"))
}

fn dir_is_writable(dir: &Path) -> bool {
    let probe = dir.join(".write-probe");
    let ok = std::fs::write(&probe, b"x").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Copies the identity, trust store and room membership from an old per-user directory into the new
/// machine-wide one, so a PC that paired before this change stays paired. Never overwrites.
fn migrate_identity(from: &Path, to: &Path) {
    for name in ["device.key", "trust.bin", "room.txt", "leave-attempts.txt"] {
        let (src, dst) = (from.join(name), to.join(name));
        if src.exists() && !dst.exists() {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}

fn load_identity() -> Result<Identity, String> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    Identity::load_or_create(&dir.join("device.key")).map_err(|e| e.to_string())
}

fn audit_path() -> PathBuf {
    data_dir().join("audit.log")
}

fn blocklist_path() -> PathBuf {
    data_dir().join("blocklist.txt")
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
    println!(
        "room        : {}",
        membership::load(&data_dir()).map_or_else(|| "(none)".to_string(), |m| m.room)
    );
    Ok(())
}

fn cmd_capture(args: &[String]) -> Result<(), String> {
    let Some(file) = args.first() else {
        return Err("usage: cowatcher-agent capture <file.jpg> [monitor] [max_width]".into());
    };
    let monitor = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(0u8);
    let max_width = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(320u16);

    let capture = ScreenCapture::new(
        &audit_path(),
        &blocklist_path(),
        &recording::directory(&data_dir()),
        &data_dir().join("wallpaper-prev.txt"),
    )
    .map_err(|e| e.to_string())?;
    println!("monitors: {}", capture.monitor_count());
    let started = std::time::Instant::now();
    let jpeg = net::AgentDevice::capture_thumbnail(
        &capture,
        monitor,
        max_width,
        proto::DEFAULT_THUMBNAIL_QUALITY,
    )
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
    if let Some(welcome) = &peer.welcome {
        membership::save(&data_dir(), welcome).map_err(|e| format!("save room: {e}"))?;
        println!("joined room \"{}\"", welcome.room);
        println!("this PC can only be removed with the room password.");
    }
    println!("paired with console {} — trusted and saved", peer.device_id);
    endpoint.close().await;
    Ok(())
}

async fn cmd_serve() -> Result<(), String> {
    run_agent(true).await
}

/// The per-session capture helper. The SYSTEM service launches this **inside the logged-in user's
/// session** — where the desktop actually is — passing the shared data directory as its one
/// argument, and terminates it on stop or a session change. It serves paired consoles exactly like
/// `serve`, only without the banner and with no console window.
async fn cmd_helper(rest: &[String]) -> Result<(), String> {
    if let Some(dir) = rest.first() {
        set_data_dir(PathBuf::from(dir));
    }
    run_agent(false).await
}

/// Binds the endpoint and serves paired consoles until Ctrl+C (or the parent kills this process).
/// Shared by the interactive `serve` and the service's `helper`.
async fn run_agent(banner: bool) -> Result<(), String> {
    let identity = load_identity()?;
    let trust = TrustStore::load(&trust_path()).map_err(|e| e.to_string())?;
    if trust.is_empty() {
        return Err(
            "no paired console yet — run `cowatcher-agent pair <console-key> <code>` first".into(),
        );
    }
    let capture = std::sync::Arc::new(
        ScreenCapture::new(
            &audit_path(),
            &blocklist_path(),
            &recording::directory(&data_dir()),
            &data_dir().join("wallpaper-prev.txt"),
        )
        .map_err(|e| e.to_string())?,
    );
    // Fail loudly at start-up rather than on the teacher's first click.
    if let Err(err) = platform::power::enable_shutdown_privilege() {
        eprintln!("warning: power actions will be refused: {err}");
    }
    let endpoint = net::bind(&identity).await.map_err(|e| e.to_string())?;
    endpoint.online().await;

    if banner {
        println!("{} agent serving", proto::PRODUCT_NAME);
        println!("device id   : {}", identity.device_id());
        println!("endpoint key: {}", identity.public_key());
        println!("monitors    : {}", capture.monitor_count());
        println!("blocklist   : {} rule(s) loaded", capture.blocked_count());
        println!("trusted     : {} console(s). Ctrl+C to stop.", trust.len());
    }

    // Both stop on Ctrl+C, which `accept_and_serve` handles; this flag stays false. The service
    // terminates the helper by other means (a job object) when it needs it gone.
    accept_and_serve(
        &endpoint,
        &trust,
        capture,
        identity.device_id(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .await;
    endpoint.close().await;
    Ok(())
}

/// The capabilities this Agent announces. One place, so `serve` and the service `run` agree.
fn agent_capabilities() -> Capabilities {
    Capabilities::SCREEN_CAPTURE
        .union(Capabilities::AUDIO)
        .union(Capabilities::LOCK)
        .union(Capabilities::POWER)
        .union(Capabilities::BLOCK)
        .union(Capabilities::REMOTE_INPUT)
}

/// Accepts and serves Console sessions until `should_stop` returns true (checked between accepts
/// and on a 1 s tick), or Ctrl+C is pressed. Shared by the interactive `serve` and the service.
async fn accept_and_serve(
    endpoint: &iroh::Endpoint,
    trust: &TrustStore,
    capture: std::sync::Arc<ScreenCapture>,
    device_id: proto::DeviceId,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    use std::sync::atomic::Ordering;

    let local = net::LocalHello {
        role: Role::Agent,
        device_id,
        capabilities: agent_capabilities(),
    };

    // A stop request closes the endpoint, which makes the pending `accept` below return.
    //
    // Doing it this way, rather than racing a timer inside the `select!`, matters: `accept` is
    // **not** cancel-safe, so a timer branch firing part-way through a handshake throws away the
    // connection a console was in the middle of making. That showed up as an intermittent
    // "connection lost" the first time a teacher connected.
    let watcher = tokio::spawn({
        let endpoint = endpoint.clone();
        let stop = std::sync::Arc::clone(&stop);
        async move {
            while !stop.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            endpoint.close().await;
        }
    });

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            session = net::ControlSession::accept(endpoint, trust, local) => match session {
                Ok(session) => {
                    // One task per session, so a teacher can keep the thumbnail grid open *and*
                    // open a full-resolution viewer of the same PC at the same time. Serving
                    // sessions one-at-a-time made the second connection hang until the first ended.
                    let capture = std::sync::Arc::clone(&capture);
                    tokio::spawn(async move {
                        println!("console {} connected", session.peer().device_id);
                        if let Err(err) = session.serve(&*capture).await {
                            eprintln!("session ended: {err}");
                        } else {
                            println!("console disconnected");
                        }
                        // A dropped console must never leave the desktop blacked out.
                        capture.end_session();
                    });
                }
                Err(err) => eprintln!("rejected a connection: {err}"),
            },
        }
    }
    watcher.abort();
}

/// Installs the Agent as an auto-start Windows service (needs an elevated/admin prompt).
///
/// The binary is **copied into a permanent location** (`%ProgramData%\co-watcher\agent\`) and the
/// service registered from there, so the teacher can run `install` from a temporary download folder
/// and then delete it — the service keeps working across reboots because it no longer points at the
/// original file.
fn cmd_install() -> Result<(), String> {
    let installed_exe = install_agent_binary()?;
    platform::service::install(&installed_exe).map_err(|e| e.to_string())?;
    println!(
        "installed the \"{}\" service (auto-start) from {}.",
        platform::service::DISPLAY_NAME,
        installed_exe.display()
    );
    println!("The binary was copied there, so you can delete the one you ran this from.");
    println!("It starts at boot and does not appear in Task Manager's Startup tab, so a student");
    println!("cannot switch it off there. An administrator can, via services.msc or:");
    println!("    cowatcher-agent uninstall   (from an elevated prompt)");
    Ok(())
}

/// Copies this running executable into the permanent agent directory and returns the copy's path. If
/// we are already running from that location, no copy is made.
fn install_agent_binary() -> Result<PathBuf, String> {
    let src = std::env::current_exe().map_err(|e| format!("locate this executable: {e}"))?;
    let dir = program_data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let dest = dir.join("cowatcher-agent.exe");
    if src == dest {
        return Ok(dest);
    }
    // Replace any previous copy. If the old one is running (a reinstall), it is locked; renaming it
    // aside first lets the copy succeed and the stale file is cleaned up on the next boot.
    if dest.exists() {
        let old = dir.join("cowatcher-agent.old.exe");
        let _ = std::fs::remove_file(&old);
        let _ = std::fs::rename(&dest, &old);
    }
    std::fs::copy(&src, &dest).map_err(|e| format!("copy the agent to {}: {e}", dest.display()))?;
    Ok(dest)
}

/// Removes the Agent service (needs an elevated/admin prompt).
fn cmd_uninstall() -> Result<(), String> {
    platform::service::uninstall().map_err(|e| e.to_string())?;
    println!(
        "removed the \"{}\" service.",
        platform::service::DISPLAY_NAME
    );
    Ok(())
}

/// Shows whether the service is installed.
fn cmd_status() -> Result<(), String> {
    if platform::service::is_installed() {
        println!(
            "the \"{}\" service is installed.",
            platform::service::DISPLAY_NAME
        );
    } else {
        println!("the service is not installed (run `install` from an elevated prompt).");
    }
    Ok(())
}

/// The service entry point: the SCM launches `cowatcher-agent run`. Runs as LocalSystem, which is
/// what lets it enforce the constant-wallpaper policy that an unelevated Agent cannot.
fn cmd_run() -> Result<(), String> {
    platform::service::run(service_body).map_err(|e| e.to_string())
}

/// What the service actually does once the control manager says it is running.
///
/// Enables the constant-wallpaper policy (now possible as SYSTEM) and then serves paired consoles
/// until asked to stop.
///
/// ponytail: capturing the interactive desktop from session 0 needs the per-session helper spawn
/// (`WTSQueryUserToken` + `CreateProcessAsUserW`) — the remaining part of PLAN 1.4b. Until that
/// lands, the service still enforces wallpaper/power/blocking; screen capture belongs to the
/// user-session `serve`.
fn service_body(stop: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };

    // Note: the "constant wallpaper" policy is NOT auto-enabled here. It is HKEY_CURRENT_USER, and a
    // service runs as SYSTEM, so this process would only lock *SYSTEM's* profile, never the student's
    // — and auto-locking surprised testers who had not asked for it. Wallpaper lock is now an
    // explicit teacher action (Action::LockWallpaper), applied by the per-session helper as the
    // student, where HKCU is the right hive.

    // Establish / migrate the machine-wide identity now, as SYSTEM, so the helper (running as the
    // student) only has to *read* it.
    let data = data_dir();
    let _ = std::fs::create_dir_all(&data);
    let data_arg = data.to_string_lossy().into_owned();

    let Ok(exe) = std::env::current_exe() else {
        eprintln!("service cannot find its own path");
        return;
    };

    // A service in session 0 cannot capture a user's desktop, so it does not serve directly. It keeps
    // a per-session *helper* alive in whichever session the student is using — relaunching it when the
    // student signs in or out or switches user, and killing it (via a job object) on stop.
    let mut policy = supervisor::RestartPolicy::default();
    let mut helper: Option<(u32, platform::session::SessionProcess)> = None;
    let mut started_at: Option<Instant> = None;

    while !stop.load(Ordering::SeqCst) {
        match platform::session::active_console_session() {
            None => {
                // Login screen, nobody signed in: nothing to capture. Drop any helper and wait.
                if helper.take().is_some() {
                    println!("no interactive user; session helper stopped");
                }
                started_at = None;
                sleep_until_stop(Duration::from_secs(2), &stop);
            }
            Some(session_id) => {
                let alive =
                    matches!(&helper, Some((sid, h)) if *sid == session_id && h.is_running());
                if alive {
                    sleep_until_stop(Duration::from_secs(1), &stop);
                    continue;
                }
                // The helper died or the active session changed. Back off if it had only just
                // started, so a crash loop cannot peg the CPU.
                if let Some(when) = started_at.take() {
                    let delay = policy.record_exit(when.elapsed());
                    if delay > Duration::ZERO && sleep_until_stop(delay, &stop) {
                        break;
                    }
                }
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                helper = None; // drop kills the old one before a new one starts
                match platform::session::launch_in_session(session_id, &exe, &["helper", &data_arg])
                {
                    Ok(child) => {
                        println!(
                            "session helper started in session {session_id} (pid {})",
                            child.pid()
                        );
                        helper = Some((session_id, child));
                        started_at = Some(Instant::now());
                    }
                    Err(err) => {
                        eprintln!("could not start session helper: {err}");
                        started_at = Some(Instant::now()); // an immediate failure: back off next pass
                    }
                }
                sleep_until_stop(Duration::from_millis(500), &stop);
            }
        }
    }
    // Stopping: drop the helper, which the job object kills.
    drop(helper);
}

/// Sleeps up to `dur`, returning `true` early if a stop was requested.
fn sleep_until_stop(
    dur: std::time::Duration,
    stop: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> bool {
    use std::sync::atomic::Ordering;
    let deadline = std::time::Instant::now() + dur;
    while std::time::Instant::now() < deadline {
        if stop.load(Ordering::SeqCst) {
            return true;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        std::thread::sleep(std::time::Duration::from_millis(100).min(remaining));
    }
    stop.load(Ordering::SeqCst)
}

/// Shows which room this PC is in.
fn cmd_room() -> Result<(), String> {
    match membership::load(&data_dir()) {
        Some(m) => {
            println!("room: {}", m.room);
            println!("This PC can only be removed with the room password:");
            println!("    cowatcher-agent leave <room-password>");
        }
        None => println!("this PC is not in a room"),
    }
    Ok(())
}

/// Takes this PC out of its room. Needs the room password the teacher holds.
fn cmd_leave(args: &[String]) -> Result<(), String> {
    let Some(password) = args.first() else {
        return Err("usage: cowatcher-agent leave <room-password>".into());
    };
    let now_s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match membership::leave(&data_dir(), password, now_s) {
        Ok(room) => {
            println!("left room \"{room}\" — this PC is no longer managed");
            Ok(())
        }
        Err(err) => Err(err.to_string()),
    }
}

/// Confirms the black-on-watch wallpaper mechanism works on this PC. Flashes the desktop black for
/// an instant, then restores the real wallpaper, and reports whether it came back.
fn cmd_wallpaper_selftest() -> Result<(), String> {
    let save = data_dir().join("wallpaper-prev.txt");
    std::fs::create_dir_all(data_dir()).map_err(|e| e.to_string())?;
    match platform::wallpaper::selftest(&save).map_err(|e| e.to_string())? {
        true => {
            println!("PASS: the wallpaper went black and the original was restored");
            Ok(())
        }
        false => Err("the wallpaper did not round-trip cleanly".into()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrate_copies_missing_identity_files_and_never_overwrites() {
        // The machine-wide data dir carries an existing per-user identity across on first use, so a
        // PC paired before the switch keeps its device id — but it must never clobber a file that is
        // already there (that would throw away a newer identity).
        let base = std::env::temp_dir().join(format!("cw-migrate-{}", std::process::id()));
        let (from, to) = (base.join("from"), base.join("to"));
        std::fs::create_dir_all(&from).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(from.join("device.key"), b"KEY").unwrap();
        std::fs::write(from.join("trust.bin"), b"OLD").unwrap();
        std::fs::write(to.join("trust.bin"), b"KEEP").unwrap(); // already present in the target

        migrate_identity(&from, &to);

        assert_eq!(
            std::fs::read(to.join("device.key")).unwrap(),
            b"KEY",
            "missing file copied"
        );
        assert_eq!(
            std::fs::read(to.join("trust.bin")).unwrap(),
            b"KEEP",
            "existing file kept"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}
