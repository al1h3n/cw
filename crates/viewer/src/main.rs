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

/// A decoded frame ready to blit: packed `0x00RRGGBB` pixels plus its size.
struct Frame {
    pixels: Vec<u32>,
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

    let latest: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
    let (packets_tx, packets_rx) = std::sync::mpsc::channel::<Vec<u8>>();
    let (input_tx, input_rx) = unbounded_channel::<InputCmd>();

    let initial_control = args.control;
    // Network: connect, start the stream, pump encoded packets to the decoder, apply input.
    {
        thread::spawn(move || {
            if let Err(err) = network_main(args, &packets_tx, input_rx, initial_control) {
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

    let mut app = App::new(latest, input_tx, initial_control);
    event_loop.run_app(&mut app).context("run window")?;
    // Give the network task a moment to deliver the "release control" it was sent on close, so no
    // modifier is left held down on the student's PC.
    thread::sleep(Duration::from_millis(150));
    Ok(())
}

/// The network thread: one Tokio runtime that owns the control session.
fn network_main(
    args: Args,
    packets_tx: &std::sync::mpsc::Sender<Vec<u8>>,
    input_rx: UnboundedReceiver<InputCmd>,
    want_control: bool,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build runtime")?;
    let packets_tx = packets_tx.clone();
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
        let mut session = net::ControlSession::connect(
            &endpoint,
            iroh::EndpointAddr::new(args.agent_key),
            &trust,
            local,
        )
        .await
        .map_err(|e| anyhow!("connect: {e}"))?;

        let actual = session
            .start_stream(args.monitor, args.settings)
            .await
            .map_err(|e| anyhow!("start stream: {e}"))?;
        println!(
            "streaming {}x{} @ {} fps, {} kbit/s (asked {}x{} @ {})",
            actual.width,
            actual.height,
            actual.fps,
            actual.kbps,
            args.settings.width,
            args.settings.height,
            args.settings.fps
        );

        let mut video = session
            .accept_video()
            .await
            .map_err(|e| anyhow!("open video: {e}"))?;
        let reader = tokio::spawn(async move {
            loop {
                match video.next_frame().await {
                    Ok(Some(packet)) => {
                        if packets_tx.send(packet).is_err() {
                            break; // decoder gone
                        }
                    }
                    Ok(None) => break, // agent closed the stream
                    Err(err) => {
                        eprintln!("video ended: {err}");
                        break;
                    }
                }
            }
        });

        if want_control {
            match session.set_control(true).await {
                Ok(true) => println!("control: ON"),
                Ok(false) => println!("control: refused by that PC"),
                Err(err) => eprintln!("control request failed: {err}"),
            }
        }

        input_loop(&mut session, input_rx).await;
        reader.abort();
        let _ = session.stop_stream().await;
        Ok(())
    })
}

/// Applies input commands from the window, coalescing bursts of pointer moves into one batch so a
/// fast mouse does not become a queue of round-trips.
async fn input_loop(session: &mut net::ControlSession, mut rx: UnboundedReceiver<InputCmd>) {
    let mut controlling = false;
    while let Some(cmd) = rx.recv().await {
        match cmd {
            InputCmd::Control(on) => match session.set_control(on).await {
                Ok(state) => {
                    controlling = state;
                    println!("control: {}", if state { "ON" } else { "OFF" });
                }
                Err(err) => eprintln!("control toggle failed: {err}"),
            },
            InputCmd::Events(mut events) => {
                // Drain anything already queued into the same batch (bounded by the wire cap).
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
                            // A control change interrupts the batch: send what we have, then apply it.
                            if controlling && !events.is_empty() {
                                let batch = std::mem::take(&mut events);
                                let _ = session.send_input(batch).await;
                            }
                            if let Ok(state) = session.set_control(on).await {
                                controlling = state;
                                println!("control: {}", if state { "ON" } else { "OFF" });
                            }
                            break;
                        }
                        Err(_) => break,
                    }
                }
                if controlling
                    && !events.is_empty()
                    && let Err(err) = session.send_input(events).await
                {
                    eprintln!("input send failed: {err}");
                }
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
            Ok(Some((bgra, width, height))) => {
                let pixels = bgra
                    .chunks_exact(4)
                    .map(|p| (u32::from(p[2]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[0]))
                    .collect();
                Frame {
                    pixels,
                    width,
                    height,
                }
            }
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
}

impl App {
    fn new(
        latest: Arc<Mutex<Option<Frame>>>,
        input_tx: UnboundedSender<InputCmd>,
        controlling: bool,
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
        }
    }

    fn send(&self, event: InputEvent) {
        let _ = self.input_tx.send(InputCmd::Events(vec![event]));
    }

    fn set_control(&mut self, on: bool) {
        self.controlling = on;
        let _ = self.input_tx.send(InputCmd::Control(on));
        if !on {
            self.send(InputEvent::ReleaseAll);
        }
        self.retitle();
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
        let Some(surface) = self.surface.as_mut() else {
            return Ok(());
        };
        let nz_w = NonZeroU32::new(width).context("zero width")?;
        let nz_h = NonZeroU32::new(height).context("zero height")?;
        surface.resize(nz_w, nz_h).map_err(|e| anyhow!("{e}"))?;
        let mut buffer = surface.buffer_mut().map_err(|e| anyhow!("{e}"))?;
        buffer.fill(0); // black letterbox bars

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
            for dy in 0..draw_h {
                let sy = ((u64::from(dy) * u64::from(frame.height)) / u64::from(draw_h)) as u32;
                let sy = sy.min(frame.height - 1);
                let src_row = (sy * frame.width) as usize;
                let dst_row = ((off_y + dy) * width + off_x) as usize;
                for dx in 0..draw_w {
                    let sx = ((u64::from(dx) * u64::from(frame.width)) / u64::from(draw_w)) as u32;
                    let sx = sx.min(frame.width - 1);
                    buffer[dst_row + dx as usize] = frame.pixels[src_row + sx as usize];
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
            .with_inner_size(PhysicalSize::new(1280, 720));
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
        self.retitle();
    }

    fn user_event(&mut self, _: &ActiveEventLoop, _: UserEvent) {
        if let Some(window) = &self.window {
            window.request_redraw();
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
            WindowEvent::Focused(false) => {
                // Alt-tabbing away must not leave a key held on the student's PC.
                self.ctrl_down = false;
                self.alt_down = false;
                if self.controlling {
                    self.send(InputEvent::ReleaseAll);
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
        KeyA => 0x41, KeyB => 0x42, KeyC => 0x43, KeyD => 0x44, KeyE => 0x45, KeyF => 0x46,
        KeyG => 0x47, KeyH => 0x48, KeyI => 0x49, KeyJ => 0x4A, KeyK => 0x4B, KeyL => 0x4C,
        KeyM => 0x4D, KeyN => 0x4E, KeyO => 0x4F, KeyP => 0x50, KeyQ => 0x51, KeyR => 0x52,
        KeyS => 0x53, KeyT => 0x54, KeyU => 0x55, KeyV => 0x56, KeyW => 0x57, KeyX => 0x58,
        KeyY => 0x59, KeyZ => 0x5A,
        Digit0 => 0x30, Digit1 => 0x31, Digit2 => 0x32, Digit3 => 0x33, Digit4 => 0x34,
        Digit5 => 0x35, Digit6 => 0x36, Digit7 => 0x37, Digit8 => 0x38, Digit9 => 0x39,
        Enter | NumpadEnter => 0x0D,
        Escape => 0x1B,
        Backspace => 0x08,
        Tab => 0x09,
        Space => 0x20,
        Minus => 0xBD, Equal => 0xBB,
        BracketLeft => 0xDB, BracketRight => 0xDD, Backslash => 0xDC,
        Semicolon => 0xBA, Quote => 0xDE, Backquote => 0xC0,
        Comma => 0xBC, Period => 0xBE, Slash => 0xBF,
        CapsLock => 0x14,
        ControlLeft => 0xA2, ControlRight => 0xA3,
        ShiftLeft => 0xA0, ShiftRight => 0xA1,
        AltLeft => 0xA4, AltRight => 0xA5,
        SuperLeft => 0x5B, SuperRight => 0x5C,
        ArrowLeft => 0x25, ArrowUp => 0x26, ArrowRight => 0x27, ArrowDown => 0x28,
        Home => 0x24, End => 0x23, PageUp => 0x21, PageDown => 0x22,
        Insert => 0x2D, Delete => 0x2E,
        F1 => 0x70, F2 => 0x71, F3 => 0x72, F4 => 0x73, F5 => 0x74, F6 => 0x75,
        F7 => 0x76, F8 => 0x77, F9 => 0x78, F10 => 0x79, F11 => 0x7A, F12 => 0x7B,
        Numpad0 => 0x60, Numpad1 => 0x61, Numpad2 => 0x62, Numpad3 => 0x63, Numpad4 => 0x64,
        Numpad5 => 0x65, Numpad6 => 0x66, Numpad7 => 0x67, Numpad8 => 0x68, Numpad9 => 0x69,
        NumpadMultiply => 0x6A, NumpadAdd => 0x6B, NumpadSubtract => 0x6D,
        NumpadDecimal => 0x6E, NumpadDivide => 0x6F,
        _ => return None,
    };
    Some(vk)
}
