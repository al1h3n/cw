//! `cowatcher-viewer` — the teacher's native, full-resolution remote-view window (ADR D12).
//!
//! The Tauri Console shows a grid of small JPEG thumbnails; to actually *watch and drive* one PC a
//! teacher opens this: a resizable window that pulls a live **H.264** stream from the Agent, decodes
//! it in Rust (a WebView cannot — 1080p is hundreds of megabytes a second of raw pixels, far too
//! much for its IPC bridge, which is exactly why D12 calls for a native window), and forwards mouse
//! and keyboard back over the same trusted control session.
//!
//! ```text
//! cowatcher-viewer <agent-endpoint-key> [width height fps kbps] [monitor N] [control]
//! ```
//!
//! It dials as the **same paired console** whose keys live in `%LOCALAPPDATA%\co-watcher\console`
//! (or `COWATCHER_DIR`), so an Agent trusts it exactly as it trusts the grid.
//!
//! In the window: **Ctrl+Alt+Esc toggles control** (matching the release chord the Agent's key gate
//! uses on the student side). While controlling, every key and click goes to that PC; releasing, or
//! the window losing focus, sends "release everything" so no key is ever left stuck down.

// Release builds are a GUI app, so launching the viewer never flashes a console window next to the
// teacher's screen. Debug builds keep the console for the connection log and decode diagnostics.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::{
    num::NonZeroU32,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use proto::{InputEvent, PointerButton};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
    keyboard::{KeyCode, PhysicalKey},
    window::{Window, WindowId},
};

/// A decoded frame ready to blit: packed BGRA pixels (as OpenH264 gives them) plus its size.
struct Frame {
    bgra: Vec<u8>,
    width: u32,
    height: u32,
}

/// Where on the window the last frame was drawn, so a cursor position maps back to a screen
/// fraction the Agent understands.
#[derive(Clone, Copy)]
struct ImageRect {
    off_x: f64,
    off_y: f64,
    draw_w: f64,
    draw_h: f64,
}

/// Wakes the window when a new frame has been decoded.
enum UserEvent {
    NewFrame,
    /// The stream ended (teacher stopped watching) — switch to the bouncing "disabled" screen.
    StreamEnded,
    /// The keyboard grab captured the release chord (Ctrl+Alt+Esc): toggle control off.
    ToggleControl,
    /// The keyboard grab captured a key while controlling; forward it to the student PC.
    GrabbedKey {
        /// Windows virtual-key code.
        vk: u16,
        /// True on press.
        down: bool,
    },
}

/// What the window asks the network task to do.
enum InputCmd {
    /// Take or release control of the student's mouse and keyboard.
    Control(bool),
    /// Deliver these input events (coalesced into a batch before sending).
    Events(Vec<InputEvent>),
}

/// Parsed command line.
struct Args {
    agent_key: iroh::EndpointId,
    settings: proto::VideoSettings,
    monitor: u8,
    control: bool,
}

fn main() -> std::process::ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&raw) {
        Ok(args) => args,
        Err(err) => {
            eprintln!("error: {err}");
            eprintln!(
                "usage: cowatcher-viewer <agent-endpoint-key> [width height fps kbps] [monitor N] [control]"
            );
            return std::process::ExitCode::FAILURE;
        }
    };
    match run(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn parse_args(raw: &[String]) -> Result<Args> {
    let key = raw.first().context("need the agent's endpoint key")?;
    let agent_key: iroh::EndpointId = key
        .parse()
        .map_err(|_| anyhow!("that is not a valid endpoint key"))?;

    // Positional numbers (like the `stream` subcommand), then optional keywords.
    let positional: Vec<u32> = raw
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with(|c: char| c.is_ascii_alphabetic()))
        .filter_map(|a| a.parse().ok())
        .collect();
    let at = |i: usize, default: u32| positional.get(i).copied().unwrap_or(default);
    let settings = proto::VideoSettings {
        width: at(0, 1280),
        height: at(1, 720),
        fps: at(2, 30),
        kbps: at(3, 4_000),
    };

    let control = raw.iter().any(|a| a.eq_ignore_ascii_case("control"));
    let monitor = raw
        .iter()
        .position(|a| a.eq_ignore_ascii_case("monitor"))
        .and_then(|i| raw.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    Ok(Args {
        agent_key,
        settings,
        monitor,
        control,
    })
}

/// The console's state directory — the same one the Tauri Console uses, so this dials with the
/// identity an Agent already trusts.
fn console_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("COWATCHER_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("co-watcher").join("console")
}

fn run(args: Args) -> Result<()> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .context("create event loop")?;
    let proxy = event_loop.create_proxy();
    let net_proxy = event_loop.create_proxy();

    let latest: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
    let (packets_tx, packets_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (input_tx, input_rx) = unbounded_channel::<InputCmd>();

    let initial_control = args.control;
    // Launched to control: queue the grant now so the input loop opens its gate as soon as it runs.
    if initial_control {
        let _ = input_tx.send(InputCmd::Control(true));
    }
    // Network: connect, start the stream, pump encoded packets to the decoder, apply input.
    {
        thread::spawn(move || {
            if let Err(err) = network_main(args, &packets_tx, input_rx, &net_proxy) {
                eprintln!("network: {err:#}");
                std::process::exit(1);
            }
        });
    }
    // Decode: H.264 -> BGRA -> packed pixels -> hand to the window.
    {
        let latest = Arc::clone(&latest);
        thread::spawn(move || decode_main(&packets_rx, &latest, &proxy));
    }

    let grab_proxy = event_loop.create_proxy();
    let mut app = App::new(latest, input_tx, initial_control, grab_proxy);
    event_loop.run_app(&mut app).context("run window")?;
    // Give the network task a moment to deliver the "release control" it was sent on close, so no
    // modifier is left held down on the student's PC.
    thread::sleep(Duration::from_millis(150));
    Ok(())
}

/// The network thread: one Tokio runtime that reconnects the control session as needed.
///
/// If the student's signal drops, the video reader ends; we tell the window (which shows the
/// bouncing "disabled" sign) and keep retrying. As soon as a session comes back and frames flow, the
/// decode thread's `NewFrame` events make the window switch back to the live screen on its own.
fn network_main(
    args: Args,
    packets_tx: &std::sync::mpsc::Sender<Vec<u8>>,
    mut input_rx: UnboundedReceiver<InputCmd>,
    ended: &EventLoopProxy<UserEvent>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build runtime")?;
    let packets_tx = packets_tx.clone();
    let ended = ended.clone();
    runtime.block_on(async move {
        let dir = console_dir();
        let identity = net::Identity::load_or_create(&dir.join("device.key"))
            .map_err(|e| anyhow!("load console identity from {}: {e}", dir.display()))?;
        let trust = net::TrustStore::load(&dir.join("trust.bin"))
            .map_err(|e| anyhow!("load trust store: {e}"))?;
        if !trust.is_trusted(args.agent_key.as_bytes()) {
            return Err(anyhow!(
                "that PC is not paired with this console — pair it first"
            ));
        }
        let endpoint = net::bind(&identity)
            .await
            .map_err(|e| anyhow!("bind endpoint: {e}"))?;
        endpoint.online().await;
        let local = net::LocalHello {
            role: proto::Role::Console,
            device_id: identity.device_id(),
            capabilities: proto::Capabilities::EMPTY,
        };

        // Whether the teacher currently holds control — kept across reconnects so a dropped signal
        // does not silently give it up.
        let mut controlling = false;
        loop {
            match session_once(
                &endpoint,
                &args,
                &trust,
                local,
                &packets_tx,
                &mut input_rx,
                &mut controlling,
            )
            .await
            {
                SessionEnd::WindowClosed => break,
                SessionEnd::Lost => {
                    // Signal lost: show the bouncing sign and try again shortly.
                    let _ = ended.send_event(UserEvent::StreamEnded);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
        Ok(())
    })
}

/// Why a single connection attempt ended.
enum SessionEnd {
    /// The window closed (the input channel was dropped): stop for good.
    WindowClosed,
    /// The stream/connection dropped: reconnect.
    Lost,
}

/// One connection: dial, start the stream, and pump video + input until it drops or the window
/// closes.
async fn session_once(
    endpoint: &iroh::Endpoint,
    args: &Args,
    trust: &net::TrustStore,
    local: net::LocalHello,
    packets_tx: &std::sync::mpsc::Sender<Vec<u8>>,
    input_rx: &mut UnboundedReceiver<InputCmd>,
    controlling: &mut bool,
) -> SessionEnd {
    let mut session = match net::ControlSession::connect(
        endpoint,
        iroh::EndpointAddr::new(args.agent_key),
        trust,
        local,
    )
    .await
    {
        Ok(session) => session,
        Err(err) => {
            eprintln!("connect: {err}");
            return SessionEnd::Lost;
        }
    };
    let actual = match session.start_stream(args.monitor, args.settings).await {
        Ok(actual) => actual,
        Err(err) => {
            eprintln!("start stream: {err}");
            return SessionEnd::Lost;
        }
    };
    println!(
        "streaming {}x{} @ {} fps, {} kbit/s",
        actual.width, actual.height, actual.fps, actual.kbps
    );
    let mut video = match session.accept_video().await {
        Ok(video) => video,
        Err(err) => {
            eprintln!("open video: {err}");
            return SessionEnd::Lost;
        }
    };

    // Re-assert control on a fresh session if the teacher held it before the drop.
    if *controlling {
        let _ = session.set_control(true).await;
    }

    let reader_done = std::sync::Arc::new(tokio::sync::Notify::new());
    let reader = {
        let packets_tx = packets_tx.clone();
        let done = std::sync::Arc::clone(&reader_done);
        tokio::spawn(async move {
            // Ends on Ok(None) (clean close) or Err (dropped): either way the stream is over.
            while let Ok(Some(packet)) = video.next_frame().await {
                if packets_tx.send(packet).is_err() {
                    break; // decoder gone
                }
            }
            done.notify_one();
        })
    };

    let result = loop {
        tokio::select! {
            biased;
            () = reader_done.notified() => break SessionEnd::Lost,
            cmd = input_rx.recv() => match cmd {
                None => break SessionEnd::WindowClosed,
                Some(cmd) => apply_input(&mut session, input_rx, cmd, controlling).await,
            },
        }
    };
    reader.abort();
    let _ = session.stop_stream().await;
    result
}

/// Applies one input command, coalescing any queued pointer moves into the same batch so a fast
/// mouse does not become a queue of round-trips.
async fn apply_input(
    session: &mut net::ControlSession,
    rx: &mut UnboundedReceiver<InputCmd>,
    cmd: InputCmd,
    controlling: &mut bool,
) {
    match cmd {
        InputCmd::Control(on) => match session.set_control(on).await {
            Ok(state) => {
                *controlling = state;
                println!("control: {}", if state { "ON" } else { "OFF" });
            }
            Err(err) => eprintln!("control toggle failed: {err}"),
        },
        InputCmd::Events(mut events) => {
            loop {
                if events.len() >= proto::MAX_INPUT_BATCH {
                    break;
                }
                match rx.try_recv() {
                    Ok(InputCmd::Events(more)) => {
                        for event in more {
                            if events.len() < proto::MAX_INPUT_BATCH {
                                events.push(event);
                            }
                        }
                    }
                    Ok(InputCmd::Control(on)) => {
                        if *controlling && !events.is_empty() {
                            let _ = session.send_input(std::mem::take(&mut events)).await;
                        }
                        if let Ok(state) = session.set_control(on).await {
                            *controlling = state;
                            println!("control: {}", if state { "ON" } else { "OFF" });
                        }
                        break;
                    }
                    Err(_) => break,
                }
            }
            if *controlling
                && !events.is_empty()
                && let Err(err) = session.send_input(events).await
            {
                eprintln!("input send failed: {err}");
            }
        }
    }
}

/// The decode thread: encoded packets in, packed pixels out, waking the window on each frame.
fn decode_main(
    packets: &std::sync::mpsc::Receiver<Vec<u8>>,
    latest: &Mutex<Option<Frame>>,
    proxy: &EventLoopProxy<UserEvent>,
) {
    let mut decoder = match media::h264::Decoder::new() {
        Ok(decoder) => decoder,
        Err(err) => {
            eprintln!("decoder: {err}");
            return;
        }
    };
    while let Ok(packet) = packets.recv() {
        let frame = match decoder.decode(&packet) {
            Ok(Some((bgra, width, height))) => Frame {
                bgra,
                width,
                height,
            },
            Ok(None) => continue, // parameter sets, no picture yet
            Err(err) => {
                eprintln!("decode: {err}");
                continue;
            }
        };
        if let Ok(mut slot) = latest.lock() {
            *slot = Some(frame);
        }
        if proxy.send_event(UserEvent::NewFrame).is_err() {
            return; // window gone
        }
    }
}

/// The window: presents the latest frame, and turns local input into remote input.
struct App {
    latest: Arc<Mutex<Option<Frame>>>,
    input_tx: UnboundedSender<InputCmd>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    image: Option<ImageRect>,
    controlling: bool,
    ctrl_down: bool,
    alt_down: bool,
    /// A proxy the low-level keyboard grab uses to hand captured keys back to the window thread.
    grab_proxy: EventLoopProxy<UserEvent>,
    /// The installed keyboard grab (RAII: removed on exit). `None` if it could not be installed, in
    /// which case control still works but the shell keeps eating Win/Alt+Tab (partial capture).
    grab: Option<platform::keygrab::KeyGrab>,
    /// Once the stream ends, the window shows a bouncing "CO-WATCHER DISABLED" sign (a DVD-logo
    /// easter egg) instead of frozen video.
    bounce: Option<Bounce>,
    last_tick: std::time::Instant,
}

/// The bouncing-sign animation state.
struct Bounce {
    x: f64,
    y: f64,
    vx: f64,
    vy: f64,
    color: u32,
}

impl App {
    fn new(
        latest: Arc<Mutex<Option<Frame>>>,
        input_tx: UnboundedSender<InputCmd>,
        controlling: bool,
        grab_proxy: EventLoopProxy<UserEvent>,
    ) -> Self {
        Self {
            latest,
            input_tx,
            window: None,
            surface: None,
            image: None,
            controlling,
            ctrl_down: false,
            alt_down: false,
            grab_proxy,
            grab: None,
            bounce: None,
            last_tick: std::time::Instant::now(),
        }
    }

    fn send(&self, event: InputEvent) {
        let _ = self.input_tx.send(InputCmd::Events(vec![event]));
    }

    /// Confines the mouse to the window and makes the keyboard grab swallow+forward keys, or lifts
    /// both. This is the "full capture" half: while `on`, every key and the pointer belong to the
    /// remote PC (except Ctrl+Alt+Del / Win+L, which the OS reserves).
    fn set_capture(&self, on: bool) {
        platform::keygrab::set_active(on);
        if on {
            if let Some(window) = &self.window
                && let (Ok(pos), size) = (window.inner_position(), window.inner_size())
            {
                let _ = platform::input::confine_cursor(
                    pos.x,
                    pos.y,
                    pos.x + size.width as i32,
                    pos.y + size.height as i32,
                );
            }
        } else {
            platform::input::release_cursor();
        }
    }

    fn set_control(&mut self, on: bool) {
        self.controlling = on;
        let _ = self.input_tx.send(InputCmd::Control(on));
        if !on {
            self.send(InputEvent::ReleaseAll);
        }
        // Grab (or release) the teacher's whole keyboard + mouse so control is full, not just the keys
        // a normal window sees (RustDesk-style). Only meaningful while the window has focus.
        self.set_capture(on);
        self.retitle();
        // Toggling with Ctrl+Alt+Esc must show up at once — the green "you are driving" frame and the
        // bottom hint are only redrawn on a present, and a still student screen sends no new frame. So
        // ask the window to repaint now; otherwise taking control over a frozen picture looked like it
        // did nothing (bug report: "no green colours, no information that control started").
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn retitle(&self) {
        if let Some(window) = &self.window {
            let mode = if self.controlling {
                "controlling — Ctrl+Alt+Esc to release"
            } else {
                "view only — Ctrl+Alt+Esc to take control"
            };
            window.set_title(&format!("Co-watcher viewer ({mode})"));
        }
    }

    /// Presents the latest decoded frame, letterboxed to fill the window, and records where it
    /// landed so pointer input maps back to a screen fraction.
    fn present(&mut self) -> Result<()> {
        let Some(window) = self.window.clone() else {
            return Ok(());
        };
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));

        // Advance the "disabled" easter egg (if active) before borrowing the surface.
        let now = std::time::Instant::now();
        let dt = (now - self.last_tick).as_secs_f64().min(0.1);
        self.last_tick = now;
        if let Some(bounce) = self.bounce.as_mut() {
            advance_bounce(bounce, f64::from(width), f64::from(height), dt);
        }

        let Some(surface) = self.surface.as_mut() else {
            return Ok(());
        };
        let nz_w = NonZeroU32::new(width).context("zero width")?;
        let nz_h = NonZeroU32::new(height).context("zero height")?;
        surface.resize(nz_w, nz_h).map_err(|e| anyhow!("{e}"))?;
        let mut buffer = surface.buffer_mut().map_err(|e| anyhow!("{e}"))?;
        buffer.fill(0); // black background / letterbox bars

        if let Some(bounce) = &self.bounce {
            let buf: &mut [u32] = &mut buffer;
            draw_sign(buf, width, height, bounce);
            buffer.present().map_err(|e| anyhow!("{e}"))?;
            return Ok(());
        }

        let guard = self.latest.lock().map_err(|_| anyhow!("frame lock"))?;
        if let Some(frame) = guard.as_ref()
            && frame.width > 0
            && frame.height > 0
        {
            let scale = (f64::from(width) / f64::from(frame.width))
                .min(f64::from(height) / f64::from(frame.height));
            let draw_w = ((f64::from(frame.width) * scale).round() as u32).clamp(1, width);
            let draw_h = ((f64::from(frame.height) * scale).round() as u32).clamp(1, height);
            let off_x = (width - draw_w) / 2;
            let off_y = (height - draw_h) / 2;
            // Resample the frame to the drawn size. Shrinking with a plain nearest pick drops whole
            // rows/columns of pixels, which is what made text look "half visible"; an area average
            // keeps every glyph readable. At the same size we copy 1:1 (pixel-perfect text), and when
            // enlarging we nearest-fill (upscaled screen text can't be sharper than its source).
            let scaled: std::borrow::Cow<[u8]> = if draw_w == frame.width && draw_h == frame.height
            {
                std::borrow::Cow::Borrowed(&frame.bgra)
            } else if draw_w < frame.width || draw_h < frame.height {
                std::borrow::Cow::Owned(media::resize::area_average(
                    &frame.bgra,
                    frame.width,
                    frame.height,
                    draw_w,
                    draw_h,
                ))
            } else {
                std::borrow::Cow::Owned(nearest_resample(
                    &frame.bgra,
                    frame.width,
                    frame.height,
                    draw_w,
                    draw_h,
                ))
            };
            for dy in 0..draw_h {
                let src_row = (dy * draw_w) as usize * 4;
                let dst_row = ((off_y + dy) * width + off_x) as usize;
                for dx in 0..draw_w {
                    let p = src_row + dx as usize * 4;
                    buffer[dst_row + dx as usize] = (u32::from(scaled[p + 2]) << 16)
                        | (u32::from(scaled[p + 1]) << 8)
                        | u32::from(scaled[p]);
                }
            }
            self.image = Some(ImageRect {
                off_x: f64::from(off_x),
                off_y: f64::from(off_y),
                draw_w: f64::from(draw_w),
                draw_h: f64::from(draw_h),
            });
        }
        drop(guard);
        let buf: &mut [u32] = &mut buffer;
        // A bottom strip always shows the control chord; a green frame makes "you are driving this
        // PC" unmistakable at a glance.
        draw_hint(buf, width, height, self.controlling);
        if self.controlling {
            let (c, t) = (0x00_2ECC71, 4);
            fill_rect(buf, width, height, (0, 0, width, t), c);
            fill_rect(
                buf,
                width,
                height,
                (0, height.saturating_sub(t), width, t),
                c,
            );
            fill_rect(buf, width, height, (0, 0, t, height), c);
            fill_rect(
                buf,
                width,
                height,
                (width.saturating_sub(t), 0, t, height),
                c,
            );
        }
        buffer.present().map_err(|e| anyhow!("{e}"))?;
        Ok(())
    }

    /// Sends a pointer move for a cursor position over the window, mapped to `0..=65535` across the
    /// student's screen. Outside the image area it clamps to the nearest edge.
    fn move_pointer(&self, x: f64, y: f64) {
        let Some(rect) = self.image else { return };
        if rect.draw_w <= 0.0 || rect.draw_h <= 0.0 {
            return;
        }
        let fx = ((x - rect.off_x) / rect.draw_w).clamp(0.0, 1.0);
        let fy = ((y - rect.off_y) / rect.draw_h).clamp(0.0, 1.0);
        self.send(InputEvent::MoveTo {
            x: (fx * 65_535.0).round() as u16,
            y: (fy * 65_535.0).round() as u16,
        });
    }

    fn on_key(&mut self, code: KeyCode, pressed: bool) {
        // Track modifiers for the release chord and so shortcuts compose on the student side.
        match code {
            KeyCode::ControlLeft | KeyCode::ControlRight => self.ctrl_down = pressed,
            KeyCode::AltLeft | KeyCode::AltRight => self.alt_down = pressed,
            _ => {}
        }
        // Ctrl+Alt+Esc toggles control locally; it is never forwarded.
        if pressed && code == KeyCode::Escape && self.ctrl_down && self.alt_down {
            self.set_control(!self.controlling);
            return;
        }
        if !self.controlling {
            return;
        }
        if let Some(vk) = keycode_to_vk(code) {
            self.send(InputEvent::Key {
                virtual_key: vk,
                down: pressed,
            });
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attributes = Window::default_attributes()
            .with_title("Co-watcher viewer")
            .with_inner_size(PhysicalSize::new(1280, 720))
            // Open large: at (or near) the student's own resolution the frame is drawn 1:1, so text
            // is pixel-perfect instead of being scaled.
            .with_maximized(true);
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Rc::new(window),
            Err(err) => {
                eprintln!("create window: {err}");
                event_loop.exit();
                return;
            }
        };
        match softbuffer::Context::new(window.clone())
            .and_then(|context| softbuffer::Surface::new(&context, window.clone()))
        {
            Ok(surface) => self.surface = Some(surface),
            Err(err) => {
                eprintln!("surface: {err}");
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);

        // Install the low-level keyboard grab on this (the event-loop) thread, routing captured keys
        // back to us as user events. It stays inactive until we take control, so the teacher's own PC
        // is unaffected until then. If it fails to install, control still works — the shell just keeps
        // eating Win/Alt+Tab (partial capture), which we note rather than fail over.
        if self.grab.is_none() {
            let proxy = self.grab_proxy.clone();
            self.grab = platform::keygrab::KeyGrab::install(move |event| match event {
                platform::keygrab::GrabEvent::Release => {
                    let _ = proxy.send_event(UserEvent::ToggleControl);
                }
                platform::keygrab::GrabEvent::Key { vk, down } => {
                    let _ = proxy.send_event(UserEvent::GrabbedKey { vk, down });
                }
            });
        }
        // If we launched straight into control, start capturing now that the window exists.
        if self.controlling {
            self.set_capture(true);
        }
        self.retitle();
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: UserEvent) {
        match event {
            // A decoded frame means the signal is back — leave the bouncing sign and show the screen.
            UserEvent::NewFrame => self.bounce = None,
            UserEvent::ToggleControl => self.set_control(!self.controlling),
            UserEvent::GrabbedKey { vk, down } => {
                if self.controlling {
                    self.send(InputEvent::Key {
                        virtual_key: vk,
                        down,
                    });
                }
            }
            UserEvent::StreamEnded => {
                if self.bounce.is_none() {
                    // Signal lost: start the bouncing "CO-WATCHER DISABLED" sign.
                    self.bounce = Some(Bounce {
                        x: 60.0,
                        y: 60.0,
                        vx: 240.0,
                        vy: 176.0,
                        color: PALETTE[0],
                    });
                    self.last_tick = std::time::Instant::now();
                }
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // While the sign is bouncing, drive a ~30 fps animation loop.
        if self.bounce.is_some() {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                std::time::Instant::now() + Duration::from_millis(33),
            ));
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                if self.controlling {
                    self.set_control(false);
                }
                event_loop.exit();
            }
            WindowEvent::Resized(_) => {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(err) = self.present() {
                    eprintln!("present: {err:#}");
                }
            }
            WindowEvent::Focused(focused) => {
                // Alt-tabbing away must not leave a key held on the student's PC, and an unfocused
                // viewer must not keep the teacher's keyboard/mouse grabbed — pause capture while it
                // is in the background, and resume it (still controlling) when focus returns.
                self.ctrl_down = false;
                self.alt_down = false;
                if self.controlling {
                    if focused {
                        self.set_capture(true);
                    } else {
                        self.send(InputEvent::ReleaseAll);
                        self.set_capture(false);
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.controlling {
                    self.move_pointer(position.x, position.y);
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if self.controlling
                    && let Some(button) = pointer_button(button)
                {
                    self.send(InputEvent::Button {
                        button,
                        down: state == ElementState::Pressed,
                    });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if self.controlling {
                    let notches = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y.round() as i16,
                        MouseScrollDelta::PixelDelta(p) => (p.y / 40.0).round() as i16,
                    };
                    if notches != 0 {
                        self.send(InputEvent::Scroll { delta: notches });
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    self.on_key(code, event.state == ElementState::Pressed);
                }
            }
            _ => {}
        }
    }
}

/// Nearest-neighbour resample of packed BGRA, used only when enlarging (screen text cannot be made
/// sharper than its source, so averaging would only blur it).
fn nearest_resample(src: &[u8], src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Vec<u8> {
    let mut out = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];
    for dy in 0..dst_h {
        let sy = (u64::from(dy) * u64::from(src_h) / u64::from(dst_h)).min(u64::from(src_h - 1));
        for dx in 0..dst_w {
            let sx =
                (u64::from(dx) * u64::from(src_w) / u64::from(dst_w)).min(u64::from(src_w - 1));
            let s = ((sy * u64::from(src_w) + sx) as usize) * 4;
            let d = ((dy * dst_w + dx) as usize) * 4;
            out[d..d + 4].copy_from_slice(&src[s..s + 4]);
        }
    }
    out
}

// --- The "disabled" easter egg: a DVD-logo-style bouncing sign shown when watching stops. ---

/// The text on the sign.
const SIGN: &str = "CO-WATCHER DISABLED";
/// Pixels per font dot (the font is 5x7 dots per glyph).
const DOT: u32 = 5;
const GLYPH_COLS: u32 = 5;
const GLYPH_ROWS: u32 = 7;
/// Dot-columns of gap between glyphs, and the sign's inner padding in pixels.
const GAP: u32 = 1;
const PAD: u32 = 22;
/// Colours the sign takes each time it bounces off a wall (0x00RRGGBB), like the old DVD logo.
const PALETTE: [u32; 6] = [
    0x00_E5FF, 0xFF_3DC4, 0xFF_D500, 0x3D_FF7A, 0xFF_7A1A, 0xFF_FFFF,
];

/// The sign's pixel size (box including padding).
fn sign_dims() -> (u32, u32) {
    let n = u32::try_from(SIGN.chars().count()).unwrap_or(0);
    let text_w = n * (GLYPH_COLS + GAP) * DOT - GAP * DOT;
    let text_h = GLYPH_ROWS * DOT;
    (text_w + 2 * PAD, text_h + 2 * PAD)
}

/// Moves the sign by its velocity, bouncing off the window edges and changing colour on each hit.
fn advance_bounce(b: &mut Bounce, w: f64, h: f64, dt: f64) {
    let (sw, sh) = sign_dims();
    let (sw, sh) = (f64::from(sw), f64::from(sh));
    b.x += b.vx * dt;
    b.y += b.vy * dt;
    let mut hit = false;
    if b.x <= 0.0 {
        b.x = 0.0;
        b.vx = b.vx.abs();
        hit = true;
    }
    if b.y <= 0.0 {
        b.y = 0.0;
        b.vy = b.vy.abs();
        hit = true;
    }
    if b.x + sw >= w {
        b.x = (w - sw).max(0.0);
        b.vx = -b.vx.abs();
        hit = true;
    }
    if b.y + sh >= h {
        b.y = (h - sh).max(0.0);
        b.vy = -b.vy.abs();
        hit = true;
    }
    if hit {
        let idx = ((b.x + b.y) as u64 % PALETTE.len() as u64) as usize;
        b.color = PALETTE[idx];
    }
}

/// Fills an axis-aligned rectangle `(x, y, width, height)` in the framebuffer, clipped to its bounds.
fn fill_rect(buf: &mut [u32], w: u32, h: u32, rect: (u32, u32, u32, u32), color: u32) {
    let (x, y, rw, rh) = rect;
    for py in y..(y + rh).min(h) {
        let row = (py * w) as usize;
        for px in x..(x + rw).min(w) {
            buf[row + px as usize] = color;
        }
    }
}

/// Draws `text` in the 5x7 font at `at = (x, y)`, each dot `dot` pixels, in `color`.
fn draw_text(buf: &mut [u32], w: u32, h: u32, at: (u32, u32), text: &str, dot: u32, color: u32) {
    let (x, y) = at;
    let mut cx = x;
    for ch in text.chars() {
        let glyph = font(ch);
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..GLYPH_COLS {
                if bits & (1 << (GLYPH_COLS - 1 - col)) != 0 {
                    let px = cx + col * dot;
                    let py = y + u32::try_from(row).unwrap_or(0) * dot;
                    fill_rect(buf, w, h, (px, py, dot, dot), color);
                }
            }
        }
        cx += (GLYPH_COLS + GAP) * dot;
    }
}

/// Draws the bouncing sign: a dark box with the coloured 5x7 text inside.
fn draw_sign(buf: &mut [u32], w: u32, h: u32, b: &Bounce) {
    let (sw, sh) = sign_dims();
    let (x0, y0) = (b.x.max(0.0) as u32, b.y.max(0.0) as u32);
    fill_rect(buf, w, h, (x0, y0, sw, sh), 0x0F_1420); // dark plaque
    draw_text(buf, w, h, (x0 + PAD, y0 + PAD), SIGN, DOT, b.color);
}

/// Draws a one-line control hint along the bottom, on a dark strip, so the Ctrl+Alt+Esc chord is
/// discoverable even with the window maximized and its title bar out of view.
fn draw_hint(buf: &mut [u32], w: u32, h: u32, controlling: bool) {
    const DOT: u32 = 2;
    let text = if controlling {
        "CONTROL ON - CTRL-ALT-ESC TO RELEASE"
    } else {
        "VIEW ONLY - CTRL-ALT-ESC TO CONTROL"
    };
    let strip = GLYPH_ROWS * DOT + 12;
    let y = h.saturating_sub(strip);
    fill_rect(buf, w, h, (0, y, w, strip), 0x0C_1018);
    let color = if controlling { 0x2E_CC71 } else { 0x9A_A6B2 };
    draw_text(buf, w, h, (10, y + 6), text, DOT, color);
}

/// A 5x7 uppercase bitmap font. It covers the full A–Z / 0–9 set plus the punctuation the on-screen
/// hints use, so every label draws in full: an earlier version had only the sign's own letters, which
/// silently dropped the `N`, `V` and `Y` in "VIEW ONLY" / "CONTROL ON" and left ugly gaps.
fn font(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01110,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b11111,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '0' => [
            0b01110, 0b10011, 0b10101, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        '-' => [0, 0, 0, 0b01110, 0, 0, 0],
        '.' => [0, 0, 0, 0, 0, 0, 0b00100],
        ',' => [0, 0, 0, 0, 0, 0b00100, 0b01000],
        ':' => [0, 0, 0b00100, 0, 0b00100, 0, 0],
        // Space and anything unmapped render blank.
        _ => [0; 7],
    }
}

/// Maps a winit mouse button to the three the wire carries; extra buttons are ignored.
fn pointer_button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Left),
        MouseButton::Right => Some(PointerButton::Right),
        MouseButton::Middle => Some(PointerButton::Middle),
        _ => None,
    }
}

/// Maps a physical key position to a Windows virtual-key code, so the **student's** layout decides
/// the character (the choice RDP and VNC make). Unmapped keys are simply not forwarded.
fn keycode_to_vk(code: KeyCode) -> Option<u16> {
    use KeyCode::*;
    let vk: u16 = match code {
        KeyA => 0x41,
        KeyB => 0x42,
        KeyC => 0x43,
        KeyD => 0x44,
        KeyE => 0x45,
        KeyF => 0x46,
        KeyG => 0x47,
        KeyH => 0x48,
        KeyI => 0x49,
        KeyJ => 0x4A,
        KeyK => 0x4B,
        KeyL => 0x4C,
        KeyM => 0x4D,
        KeyN => 0x4E,
        KeyO => 0x4F,
        KeyP => 0x50,
        KeyQ => 0x51,
        KeyR => 0x52,
        KeyS => 0x53,
        KeyT => 0x54,
        KeyU => 0x55,
        KeyV => 0x56,
        KeyW => 0x57,
        KeyX => 0x58,
        KeyY => 0x59,
        KeyZ => 0x5A,
        Digit0 => 0x30,
        Digit1 => 0x31,
        Digit2 => 0x32,
        Digit3 => 0x33,
        Digit4 => 0x34,
        Digit5 => 0x35,
        Digit6 => 0x36,
        Digit7 => 0x37,
        Digit8 => 0x38,
        Digit9 => 0x39,
        Enter | NumpadEnter => 0x0D,
        Escape => 0x1B,
        Backspace => 0x08,
        Tab => 0x09,
        Space => 0x20,
        Minus => 0xBD,
        Equal => 0xBB,
        BracketLeft => 0xDB,
        BracketRight => 0xDD,
        Backslash => 0xDC,
        Semicolon => 0xBA,
        Quote => 0xDE,
        Backquote => 0xC0,
        Comma => 0xBC,
        Period => 0xBE,
        Slash => 0xBF,
        CapsLock => 0x14,
        ControlLeft => 0xA2,
        ControlRight => 0xA3,
        ShiftLeft => 0xA0,
        ShiftRight => 0xA1,
        AltLeft => 0xA4,
        AltRight => 0xA5,
        SuperLeft => 0x5B,
        SuperRight => 0x5C,
        ArrowLeft => 0x25,
        ArrowUp => 0x26,
        ArrowRight => 0x27,
        ArrowDown => 0x28,
        Home => 0x24,
        End => 0x23,
        PageUp => 0x21,
        PageDown => 0x22,
        Insert => 0x2D,
        Delete => 0x2E,
        F1 => 0x70,
        F2 => 0x71,
        F3 => 0x72,
        F4 => 0x73,
        F5 => 0x74,
        F6 => 0x75,
        F7 => 0x76,
        F8 => 0x77,
        F9 => 0x78,
        F10 => 0x79,
        F11 => 0x7A,
        F12 => 0x7B,
        Numpad0 => 0x60,
        Numpad1 => 0x61,
        Numpad2 => 0x62,
        Numpad3 => 0x63,
        Numpad4 => 0x64,
        Numpad5 => 0x65,
        Numpad6 => 0x66,
        Numpad7 => 0x67,
        Numpad8 => 0x68,
        Numpad9 => 0x69,
        NumpadMultiply => 0x6A,
        NumpadAdd => 0x6B,
        NumpadSubtract => 0x6D,
        NumpadDecimal => 0x6E,
        NumpadDivide => 0x6F,
        _ => return None,
    };
    Some(vk)
}
