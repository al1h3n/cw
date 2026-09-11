//! Spike 0.6 — live screen video over iroh, recovering from congestion without growing latency.
//!
//! ```text
//! media-iroh send [KBPS=2500] [MIP_LEVEL=1]   # Agent side: prints its endpoint id, streams to viewers
//! media-iroh view <ENDPOINT_ID> [stall] [SECONDS]  # Console side: window + stats; exits after SECONDS
//! ```
//!
//! Wire design (Media-over-QUIC style):
//! - one QUIC **uni-stream per GOP**; a GOP starts with an IDR frame. Frame = u32 len | u64 capture µs | H.264.
//! - sender: if writing a frame blocks > 200 ms, **reset** the GOP stream (queued stale data is dropped)
//!   and force a keyframe. The next frame opens a fresh stream.
//! - viewer: always decodes the newest GOP only, and asks for a keyframe on the control bi-stream when
//!   latency drifts > 250 ms above its best-seen value (clock-offset independent).
//! - `stall` makes the viewer's reader sleep 700 ms every 5 s, to exercise recovery without clumsy.
//!   For real loss/jitter use clumsy (https://jagt.github.io/clumsy/) on either PC: 5 % drop + 50 ms lag.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize, default: u32| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(default);
    match args.first().map(String::as_str) {
        Some("send") => sender::run(num(1, 2500), num(2, 1)),
        Some("view") => {
            let id = args.get(1).ok_or_else(|| anyhow::anyhow!("view needs an endpoint id"))?;
            let stall = args.iter().any(|a| a == "stall");
            let seconds = args.iter().skip(2).find_map(|a| a.parse().ok());
            viewer::run(id, stall, seconds)
        }
        _ => anyhow::bail!(
            "usage: media-iroh send [KBPS] [MIP_LEVEL] | media-iroh view <ENDPOINT_ID> [stall] [SECONDS]"
        ),
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("media-iroh is Windows-only");
}

#[cfg(windows)]
const ALPN: &[u8] = b"cowatcher/spike-media/0";
#[cfg(windows)]
const HEADER_LEN: usize = 12;
/// Trust boundary: a peer must never make us allocate an arbitrary amount.
#[cfg(windows)]
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

#[cfg(windows)]
async fn bind() -> anyhow::Result<iroh::Endpoint> {
    use anyhow::Context;
    iroh::Endpoint::builder(iroh::endpoint::presets::N0)
        .address_lookup(iroh_mdns_address_lookup::MdnsAddressLookup::builder())
        .bind()
        .await
        .context("bind endpoint")
}

#[cfg(windows)]
fn now_micros() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_micros() as u64)
}

#[cfg(windows)]
mod sender {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result};
    use iroh::{
        endpoint::{Connection, SendStream},
        protocol::{AcceptError, ProtocolHandler, Router},
    };
    use openh264::{
        OpenH264API,
        encoder::{BitRate, Encoder, EncoderConfig, FrameRate, FrameType, RateControlMode, UsageType},
        formats::{BgraSliceU8, YUVBuffer},
    };
    use tokio::{io::AsyncReadExt, sync::mpsc};

    use crate::{ALPN, HEADER_LEN, bind, capture::Capture, now_micros};

    const FPS: u32 = 30;

    pub fn run(kbps: u32, mip_level: u32) -> Result<()> {
        tokio::runtime::Runtime::new()?.block_on(async move {
            let endpoint = bind().await?;
            println!("endpoint id: {}", endpoint.id());
            println!("on the Console PC run:  media-iroh view {}", endpoint.id());
            let router = Router::builder(endpoint).accept(ALPN, Streamer { kbps, mip_level }).spawn();
            tokio::signal::ctrl_c().await?;
            router.shutdown().await.context("shutdown")?;
            Ok(())
        })
    }

    #[derive(Debug, Clone)]
    struct Streamer {
        kbps: u32,
        mip_level: u32,
    }

    impl ProtocolHandler for Streamer {
        async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
            println!("viewer connected: {}", connection.remote_id());
            match stream_to(connection, self.kbps, self.mip_level).await {
                Ok(()) => println!("viewer left"),
                Err(err) => println!("stream ended: {err:#}"),
            }
            Ok(())
        }
    }

    struct EncodedFrame {
        data: Vec<u8>,
        keyframe: bool,
        captured_us: u64,
    }

    async fn stream_to(conn: Connection, kbps: u32, mip_level: u32) -> Result<()> {
        let force_keyframe = Arc::new(AtomicBool::new(false));
        let reset_now = Arc::new(AtomicBool::new(false));
        let (frames_tx, mut frames) = mpsc::channel::<EncodedFrame>(4);
        {
            let force_keyframe = force_keyframe.clone();
            thread::spawn(move || {
                if let Err(err) = encode_loop(kbps, mip_level, &frames_tx, &force_keyframe) {
                    eprintln!("encoder: {err:#}");
                }
            });
        }

        // Control stream: the viewer sends b'K' when it wants to skip ahead to a fresh keyframe.
        let (_control_send, mut control) = conn.accept_bi().await.context("control stream")?;
        {
            let (force_keyframe, reset_now) = (force_keyframe.clone(), reset_now.clone());
            tokio::spawn(async move {
                while let Ok(byte) = control.read_u8().await {
                    if byte == b'K' {
                        reset_now.store(true, Ordering::Relaxed);
                        force_keyframe.store(true, Ordering::Relaxed);
                    }
                }
            });
        }

        let mut stream: Option<SendStream> = None;
        let (mut gops, mut drops, mut last_report) = (0u32, 0u32, Instant::now());
        while let Some(frame) = frames.recv().await {
            if reset_now.swap(false, Ordering::Relaxed)
                && let Some(mut old) = stream.take()
            {
                let _ = old.reset(0u32.into()); // viewer asked to skip: discard queued stale data
                drops += 1;
            }
            if frame.keyframe {
                if let Some(mut old) = stream.take() {
                    let _ = old.finish();
                }
                stream = Some(conn.open_uni().await.context("open GOP stream")?);
                gops += 1;
            }
            let Some(gop) = stream.as_mut() else {
                force_keyframe.store(true, Ordering::Relaxed); // mid-GOP without a stream: wait for IDR
                continue;
            };
            let mut header = [0u8; HEADER_LEN];
            header[..4].copy_from_slice(&(frame.data.len() as u32).to_le_bytes());
            header[4..].copy_from_slice(&frame.captured_us.to_le_bytes());
            let write = async {
                gop.write_all(&header).await?;
                gop.write_all(&frame.data).await
            };
            match tokio::time::timeout(Duration::from_millis(200), write).await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => return Err(err).context("write frame"),
                Err(_) => {
                    let _ = gop.reset(0u32.into());
                    stream = None;
                    force_keyframe.store(true, Ordering::Relaxed);
                    drops += 1;
                }
            }
            if last_report.elapsed() >= Duration::from_secs(5) {
                println!("sender: {gops} GOP streams opened, {drops} dropped (congestion or viewer skip)");
                last_report = Instant::now();
            }
        }
        Ok(())
    }

    fn encode_loop(
        kbps: u32,
        mip_level: u32,
        frames: &mpsc::Sender<EncodedFrame>,
        force_keyframe: &AtomicBool,
    ) -> Result<()> {
        let capture = Capture::new(mip_level)?;
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(kbps * 1000))
            .max_frame_rate(FrameRate::from_hz(FPS as f32));
        let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config)?;
        let (w, h) = (capture.out_w as usize, capture.out_h as usize);
        let mut yuv = YUVBuffer::new(w, h);
        let interval = Duration::from_secs(1) / FPS;
        println!("capturing {w}x{h} @ <= {FPS} fps, {kbps} kbit/s");
        loop {
            let slot_end = Instant::now() + interval;
            let Some(bgra) = capture.grab(interval)? else { continue };
            let captured_us = now_micros();
            yuv.read_bgra8(BgraSliceU8::new(&bgra, (w, h)));
            if force_keyframe.swap(false, Ordering::Relaxed) {
                encoder.force_intra_frame();
            }
            let bitstream = encoder.encode(&yuv)?;
            let keyframe = matches!(bitstream.frame_type(), FrameType::IDR | FrameType::I);
            let data = bitstream.to_vec();
            if data.is_empty() {
                continue; // rate control skipped this frame
            }
            // Blocking send = backpressure: capture slows down instead of queueing stale frames.
            if frames.blocking_send(EncodedFrame { data, keyframe, captured_us }).is_err() {
                return Ok(()); // viewer gone
            }
            thread::sleep(slot_end.saturating_duration_since(Instant::now()));
        }
    }
}

#[cfg(windows)]
mod viewer {
    use std::{
        num::NonZeroU32,
        rc::Rc,
        sync::{Arc, Mutex, mpsc},
        thread,
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result};
    use iroh::{
        EndpointId,
        endpoint::{Connection, RecvStream},
    };
    use openh264::{decoder::Decoder, formats::YUVSource};
    use tokio::sync::mpsc as tokio_mpsc;
    use winit::{
        application::ApplicationHandler,
        dpi::PhysicalSize,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
        window::{Window, WindowId},
    };

    use crate::{ALPN, HEADER_LEN, MAX_FRAME_BYTES, bind, now_micros};

    /// (GOP generation, capture timestamp µs, H.264 access unit)
    type Packet = (u64, u64, Vec<u8>);

    struct Frame {
        pixels: Vec<u32>,
        width: u32,
        height: u32,
    }

    #[derive(Default)]
    struct Stats {
        shown: u32,
        bytes: usize,
        latency_raw_ms: f64,
        excess_ms: f64,
        worst_excess_ms: f64,
        gops: u64,
        stale_dropped: u32,
        keyframe_requests: u32,
    }

    enum UserEvent {
        NewFrame,
    }

    pub fn run(id: &str, stall: bool, seconds: Option<u64>) -> Result<()> {
        let remote: EndpointId = id.parse().context("invalid endpoint id")?;
        let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
        let proxy = event_loop.create_proxy();
        let latest = Arc::new(Mutex::new(None::<Frame>));
        let stats = Arc::new(Mutex::new(Stats::default()));
        let connection = Arc::new(Mutex::new(None::<Connection>));
        let (packets_tx, packets_rx) = mpsc::channel::<Packet>();
        let (keyframe_tx, keyframe_rx) = tokio_mpsc::unbounded_channel::<()>();

        {
            let (stats, connection) = (stats.clone(), connection.clone());
            thread::spawn(move || {
                let runtime = match tokio::runtime::Runtime::new() {
                    Ok(runtime) => runtime,
                    Err(err) => return eprintln!("runtime: {err}"),
                };
                let result = runtime.block_on(receive(remote, packets_tx, keyframe_rx, stats, connection, stall));
                if let Err(err) = result {
                    eprintln!("network: {err:#}");
                }
            });
        }
        {
            let (stats, latest) = (stats.clone(), latest.clone());
            thread::spawn(move || {
                if let Err(err) = decode(packets_rx, keyframe_tx, &latest, &proxy, &stats) {
                    eprintln!("decode: {err:#}");
                }
            });
        }
        {
            let stats = stats.clone();
            thread::spawn(move || report(&stats, &connection));
        }
        if let Some(seconds) = seconds {
            thread::spawn(move || {
                thread::sleep(Duration::from_secs(seconds));
                std::process::exit(0);
            });
        }

        let mut app = App { latest, window: None, surface: None };
        event_loop.run_app(&mut app)?;
        Ok(())
    }

    async fn receive(
        remote: EndpointId,
        packets: mpsc::Sender<Packet>,
        mut keyframe_requests: tokio_mpsc::UnboundedReceiver<()>,
        stats: Arc<Mutex<Stats>>,
        connection: Arc<Mutex<Option<Connection>>>,
        stall: bool,
    ) -> Result<()> {
        let endpoint = bind().await?;
        let started = Instant::now();
        let conn = endpoint.connect(remote, ALPN).await.context("connect")?;
        println!("connected in {:?}", started.elapsed());
        if let Ok(mut slot) = connection.lock() {
            *slot = Some(conn.clone());
        }

        let (mut control, _control_recv) = conn.open_bi().await.context("control stream")?;
        control.write_all(b"H").await?; // a QUIC stream becomes visible to the peer on first byte
        tokio::spawn(async move {
            while keyframe_requests.recv().await.is_some() {
                if control.write_all(b"K").await.is_err() {
                    break;
                }
            }
        });

        let mut generation = 0u64;
        loop {
            let gop = conn.accept_uni().await.context("accept GOP stream")?;
            generation += 1;
            if let Ok(mut s) = stats.lock() {
                s.gops = generation;
            }
            tokio::spawn(read_gop(gop, generation, packets.clone(), stats.clone(), stall));
        }
    }

    async fn read_gop(mut gop: RecvStream, generation: u64, packets: mpsc::Sender<Packet>, stats: Arc<Mutex<Stats>>, stall: bool) {
        let mut next_stall = Instant::now() + Duration::from_secs(5);
        loop {
            let mut header = [0u8; HEADER_LEN];
            if gop.read_exact(&mut header).await.is_err() {
                return; // finished (next GOP started) or reset (skipped)
            }
            let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
            if len > MAX_FRAME_BYTES {
                eprintln!("frame of {len} bytes rejected");
                return;
            }
            let captured_us = u64::from_le_bytes(header[4..].try_into().unwrap_or_default());
            let mut data = vec![0u8; len];
            if gop.read_exact(&mut data).await.is_err() {
                return;
            }
            if let Ok(mut s) = stats.lock() {
                s.bytes += HEADER_LEN + len;
            }
            if stall && Instant::now() >= next_stall {
                tokio::time::sleep(Duration::from_millis(700)).await; // simulated slow viewer
                next_stall = Instant::now() + Duration::from_secs(5);
            }
            if packets.send((generation, captured_us, data)).is_err() {
                return;
            }
        }
    }

    fn decode(
        packets: mpsc::Receiver<Packet>,
        keyframe_requests: tokio_mpsc::UnboundedSender<()>,
        latest: &Mutex<Option<Frame>>,
        proxy: &EventLoopProxy<UserEvent>,
        stats: &Mutex<Stats>,
    ) -> Result<()> {
        let mut decoder = Decoder::new()?;
        let (mut current, mut best_ms, mut last_request) = (0u64, f64::MAX, Instant::now());
        let mut rgba = Vec::new();
        for (generation, captured_us, data) in packets {
            if generation < current {
                if let Ok(mut s) = stats.lock() {
                    s.stale_dropped += 1;
                }
                continue;
            }
            current = generation;
            // Raw latency includes any clock offset between the PCs; "excess" above the best-seen
            // value does not, so it drives the skip decision.
            let raw_ms = (now_micros() as f64 - captured_us as f64) / 1000.0;
            best_ms = best_ms.min(raw_ms);
            let excess_ms = raw_ms - best_ms;
            if excess_ms > 250.0 && last_request.elapsed() > Duration::from_secs(1) {
                let _ = keyframe_requests.send(());
                last_request = Instant::now();
                if let Ok(mut s) = stats.lock() {
                    s.keyframe_requests += 1;
                }
            }
            let Some(yuv) = decoder.decode(&data)? else { continue };
            let (w, h) = yuv.dimensions();
            rgba.resize(yuv.rgba8_len(), 0);
            yuv.write_rgba8(&mut rgba);
            let pixels = rgba.chunks_exact(4).map(|p| (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2])).collect();
            if let Ok(mut slot) = latest.lock() {
                *slot = Some(Frame { pixels, width: w as u32, height: h as u32 });
            }
            if let Ok(mut s) = stats.lock() {
                s.shown += 1;
                s.latency_raw_ms += raw_ms;
                s.excess_ms += excess_ms;
                s.worst_excess_ms = s.worst_excess_ms.max(excess_ms);
            }
            if proxy.send_event(UserEvent::NewFrame).is_err() {
                return Ok(());
            }
        }
        Ok(())
    }

    fn report(stats: &Mutex<Stats>, connection: &Mutex<Option<Connection>>) {
        loop {
            thread::sleep(Duration::from_secs(1));
            let path = connection
                .lock()
                .ok()
                .and_then(|c| c.clone())
                .and_then(|conn| {
                    conn.paths().iter().find(|p| p.is_selected()).map(|p| {
                        format!("{} rtt {:.1} ms", if p.is_relay() { "relay" } else { "direct" }, p.rtt().as_secs_f64() * 1000.0)
                    })
                })
                .unwrap_or_else(|| "connecting".into());
            let Ok(mut s) = stats.lock() else { return };
            let n = f64::from(s.shown.max(1));
            println!(
                "shown {:>2} fps  {:>6.0} kbit/s | latency raw {:>6.1} ms  excess avg {:>5.1} worst {:>6.1} ms | \
                 GOP #{} stale-dropped {} keyframe-req {} | {path}",
                s.shown,
                s.bytes as f64 * 8.0 / 1000.0,
                s.latency_raw_ms / n,
                s.excess_ms / n,
                s.worst_excess_ms,
                s.gops,
                s.stale_dropped,
                s.keyframe_requests,
            );
            let gops = s.gops;
            *s = Stats { gops, ..Stats::default() };
        }
    }

    struct App {
        latest: Arc<Mutex<Option<Frame>>>,
        window: Option<Rc<Window>>,
        surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    }

    impl ApplicationHandler<UserEvent> for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let attributes = Window::default_attributes()
                .with_title("media-iroh viewer (spike 0.6)")
                .with_inner_size(PhysicalSize::new(1280, 720));
            let Ok(window) = event_loop.create_window(attributes) else {
                event_loop.exit();
                return;
            };
            let window = Rc::new(window);
            match softbuffer::Context::new(window.clone()).and_then(|c| softbuffer::Surface::new(&c, window.clone())) {
                Ok(surface) => self.surface = Some(surface),
                Err(err) => {
                    eprintln!("softbuffer: {err}");
                    event_loop.exit();
                }
            }
            self.window = Some(window);
        }

        fn user_event(&mut self, _: &ActiveEventLoop, UserEvent::NewFrame: UserEvent) {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }

        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::RedrawRequested => {
                    if let Err(err) = self.present() {
                        eprintln!("present: {err:#}");
                    }
                }
                _ => {}
            }
        }
    }

    impl App {
        fn present(&mut self) -> Result<()> {
            let Some(frame) = self.latest.lock().ok().and_then(|mut slot| slot.take()) else {
                return Ok(());
            };
            let surface = self.surface.as_mut().context("no surface")?;
            let w = NonZeroU32::new(frame.width).context("zero width")?;
            let h = NonZeroU32::new(frame.height).context("zero height")?;
            surface.resize(w, h).map_err(|e| anyhow::anyhow!("{e}"))?;
            let mut buffer = surface.buffer_mut().map_err(|e| anyhow::anyhow!("{e}"))?;
            buffer.copy_from_slice(&frame.pixels);
            buffer.present().map_err(|e| anyhow::anyhow!("{e}"))
        }
    }
}

/// Desktop Duplication of monitor 0, scaled on the GPU by mip level before CPU readback.
#[cfg(windows)]
mod capture {
    use std::time::Duration;

    use anyhow::{Context, Result};
    use windows::{
        Win32::{
            Foundation::HMODULE,
            Graphics::{
                Direct3D::D3D_DRIVER_TYPE_HARDWARE,
                Direct3D11::*,
                Dxgi::{Common::*, *},
            },
        },
        core::Interface,
    };

    pub struct Capture {
        context: ID3D11DeviceContext,
        duplication: IDXGIOutputDuplication,
        mips: ID3D11Texture2D,
        mips_view: ID3D11ShaderResourceView,
        staging: ID3D11Texture2D,
        level: u32,
        pub out_w: u32,
        pub out_h: u32,
    }

    impl Capture {
        pub fn new(level: u32) -> Result<Self> {
            // SAFETY: COM calls on live interfaces; descriptors are fully initialised structs.
            unsafe {
                let (mut device, mut context) = (None, None);
                D3D11CreateDevice(
                    None,
                    D3D_DRIVER_TYPE_HARDWARE,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut device),
                    None,
                    Some(&mut context),
                )?;
                let device: ID3D11Device = device.context("no D3D11 device")?;
                let adapter = device.cast::<IDXGIDevice>()?.GetAdapter()?;
                let output: IDXGIOutput1 = adapter.EnumOutputs(0)?.cast()?;
                let duplication = output.DuplicateOutput(&device)?;
                let desc = duplication.GetDesc();
                let (width, height) = (desc.ModeDesc.Width, desc.ModeDesc.Height);
                let (out_w, out_h) = ((width >> level) & !1, (height >> level) & !1);

                let mips_desc = D3D11_TEXTURE2D_DESC {
                    Width: width,
                    Height: height,
                    MipLevels: level + 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
                };
                let mut mips = None;
                device.CreateTexture2D(&mips_desc, None, Some(&mut mips))?;
                let mips = mips.context("no mip texture")?;
                let mut mips_view = None;
                device.CreateShaderResourceView(&mips, None, Some(&mut mips_view))?;
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: width >> level,
                    Height: height >> level,
                    MipLevels: 1,
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                    ..mips_desc
                };
                let mut staging = None;
                device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;

                Ok(Self {
                    context: context.context("no D3D11 context")?,
                    duplication,
                    mips,
                    mips_view: mips_view.context("no SRV")?,
                    staging: staging.context("no staging texture")?,
                    level,
                    out_w,
                    out_h,
                })
            }
        }

        /// Waits up to `timeout` for a changed desktop image; returns packed BGRA pixels.
        pub fn grab(&self, timeout: Duration) -> Result<Option<Vec<u8>>> {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            // SAFETY: COM calls on live interfaces; the acquired frame is released before returning,
            // mapped rows are read within RowPitch * out_h bytes of a CPU-readable staging texture.
            unsafe {
                match self.duplication.AcquireNextFrame(timeout.as_millis() as u32, &mut info, &mut resource) {
                    Ok(()) => {}
                    Err(err) if err.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                    Err(err) => return Err(err).context("AcquireNextFrame"),
                }
                let pixels = match (&resource, info.LastPresentTime) {
                    (Some(resource), present) if present != 0 => {
                        let frame: ID3D11Texture2D = resource.cast()?;
                        let ctx = &self.context;
                        if self.level == 0 {
                            ctx.CopyResource(&self.staging, &frame);
                        } else {
                            ctx.CopySubresourceRegion(&self.mips, 0, 0, 0, 0, &frame, 0, None);
                            ctx.GenerateMips(&self.mips_view);
                            ctx.CopySubresourceRegion(&self.staging, 0, 0, 0, 0, &self.mips, self.level, None);
                        }
                        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                        ctx.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                        let row_bytes = self.out_w as usize * 4;
                        let mut pixels = Vec::with_capacity(row_bytes * self.out_h as usize);
                        for row in 0..self.out_h as usize {
                            let src = (mapped.pData as *const u8).add(row * mapped.RowPitch as usize);
                            pixels.extend_from_slice(std::slice::from_raw_parts(src, row_bytes));
                        }
                        ctx.Unmap(&self.staging, 0);
                        Some(pixels)
                    }
                    _ => None, // pointer-only update
                };
                self.duplication.ReleaseFrame()?;
                Ok(pixels)
            }
        }
    }
}
