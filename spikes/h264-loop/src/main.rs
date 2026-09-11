//! Spike 0.5 — software H.264 round trip on one PC, with per-stage timings.
//!
//! ```text
//! h264-loop [SECONDS=20] [MIP_LEVEL=1] [KBPS=3000]
//! ```
//!
//! `MIP_LEVEL` scales the captured desktop before encoding: 0 = native, 1 = half (2560x1440 -> 1280x720).
//!
//! Threads:
//! - capture: DXGI Desktop Duplication (<= 30 fps, nothing when the screen is static) -> BGRA -> I420
//!   -> OpenH264 (screen-content mode). This is the Agent's side.
//! - decode: OpenH264 decode -> RGBA -> 0RGB pixels at half size. This is the Console's side.
//! - main: winit + softbuffer window showing the decoded stream.
//!
//! "latency" = capture -> decoded frame on screen, inside this process. Network time is spike 0.6;
//! display scan-out is not included (measure glass-to-glass with a phone stopwatch).
//! Hardware encode (Media Foundation) is a separate spike: this is the OpenH264 fallback path.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    app::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("h264-loop is Windows-only");
}

#[cfg(windows)]
mod app {
    use std::{
        num::NonZeroU32,
        rc::Rc,
        sync::{Arc, Mutex, mpsc},
        thread,
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result};
    use openh264::{
        OpenH264API,
        decoder::Decoder,
        encoder::{BitRate, Encoder, EncoderConfig, FrameRate, RateControlMode, UsageType},
        formats::{BgraSliceU8, YUVBuffer, YUVSource},
    };
    use windows::{
        Win32::{
            Foundation::{FILETIME, HMODULE},
            Graphics::{
                Direct3D::D3D_DRIVER_TYPE_HARDWARE,
                Direct3D11::*,
                Dxgi::{Common::*, *},
            },
            System::Threading::{GetCurrentProcess, GetProcessTimes},
        },
        core::Interface,
    };
    use winit::{
        application::ApplicationHandler,
        dpi::PhysicalSize,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy},
        window::{Window, WindowId},
    };

    const FPS: u32 = 30;

    /// A decoded frame ready to show, plus when its source pixels were captured.
    struct Frame {
        pixels: Vec<u32>,
        width: u32,
        height: u32,
        captured_at: Instant,
    }

    #[derive(Default)]
    struct Stats {
        encoded: u32,
        encode_time: Duration,
        convert_time: Duration,
        bytes: usize,
        decoded: u32,
        decode_time: Duration,
        presented: u32,
        latency: Duration,
        worst_latency: Duration,
    }

    enum UserEvent {
        NewFrame,
        Exit,
    }

    pub fn run() -> Result<()> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let arg = |i: usize, default: u32| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(default);
        let (seconds, mip_level, kbps) = (arg(0, 20), arg(1, 1), arg(2, 3000));

        let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
        let proxy = event_loop.create_proxy();
        let latest = Arc::new(Mutex::new(None::<Frame>));
        let stats = Arc::new(Mutex::new(Stats::default()));
        let (packets_tx, packets_rx) = mpsc::channel::<(Instant, Vec<u8>)>();

        let (width, height) = {
            let capture = Capture::new(&Gpu::new()?, mip_level)?;
            (capture.out_w, capture.out_h)
        };
        println!("encoding {width}x{height} @ <= {FPS} fps, {kbps} kbit/s target, {seconds}s");

        {
            let stats = stats.clone();
            thread::spawn(move || {
                if let Err(err) = capture_and_encode(mip_level, kbps, packets_tx, &stats) {
                    eprintln!("capture thread: {err:#}");
                }
            });
        }
        {
            let (stats, latest, proxy) = (stats.clone(), latest.clone(), proxy.clone());
            thread::spawn(move || {
                if let Err(err) = decode(packets_rx, &latest, &proxy, &stats) {
                    eprintln!("decode thread: {err:#}");
                }
            });
        }
        {
            let stats = stats.clone();
            thread::spawn(move || report(seconds, &stats, &proxy));
        }

        let mut app = App { latest, stats, window: None, surface: None, size: (width / 2, height / 2) };
        event_loop.run_app(&mut app)?;
        Ok(())
    }

    fn capture_and_encode(
        mip_level: u32,
        kbps: u32,
        packets: mpsc::Sender<(Instant, Vec<u8>)>,
        stats: &Mutex<Stats>,
    ) -> Result<()> {
        let gpu = Gpu::new()?;
        let capture = Capture::new(&gpu, mip_level)?;
        let config = EncoderConfig::new()
            .usage_type(UsageType::ScreenContentRealTime)
            .rate_control_mode(RateControlMode::Bitrate)
            .bitrate(BitRate::from_bps(kbps * 1000))
            .max_frame_rate(FrameRate::from_hz(FPS as f32));
        let mut encoder = Encoder::with_api_config(OpenH264API::from_source(), config)?;
        let mut yuv = YUVBuffer::new(capture.out_w as usize, capture.out_h as usize);
        let frame_interval = Duration::from_secs(1) / FPS;

        loop {
            let slot_end = Instant::now() + frame_interval;
            let Some(bgra) = capture.grab(&gpu, frame_interval)? else {
                continue; // static screen: nothing captured, nothing encoded
            };
            let captured_at = Instant::now();

            let started = Instant::now();
            yuv.read_bgra8(BgraSliceU8::new(&bgra, (capture.out_w as usize, capture.out_h as usize)));
            let convert_time = started.elapsed();

            let started = Instant::now();
            let packet = encoder.encode(&yuv)?.to_vec();
            let encode_time = started.elapsed();

            if let Ok(mut s) = stats.lock() {
                s.encoded += 1;
                s.convert_time += convert_time;
                s.encode_time += encode_time;
                s.bytes += packet.len();
            }
            if packets.send((captured_at, packet)).is_err() {
                return Ok(()); // decoder gone: shutting down
            }
            thread::sleep(slot_end.saturating_duration_since(Instant::now()));
        }
    }

    fn decode(
        packets: mpsc::Receiver<(Instant, Vec<u8>)>,
        latest: &Mutex<Option<Frame>>,
        proxy: &EventLoopProxy<UserEvent>,
        stats: &Mutex<Stats>,
    ) -> Result<()> {
        let mut decoder = Decoder::new()?;
        let mut rgba = Vec::new();
        for (captured_at, packet) in packets {
            let started = Instant::now();
            let Some(yuv) = decoder.decode(&packet)? else { continue };
            let (w, h) = yuv.dimensions();
            rgba.resize(yuv.rgba8_len(), 0);
            yuv.write_rgba8(&mut rgba);
            // Half size, nearest neighbour: the window only needs a preview.
            let (out_w, out_h) = (w / 2, h / 2);
            let mut pixels = Vec::with_capacity(out_w * out_h);
            for y in 0..out_h {
                let row = &rgba[(y * 2) * w * 4..];
                pixels.extend((0..out_w).map(|x| {
                    let p = &row[x * 8..x * 8 + 3];
                    (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2])
                }));
            }
            let decode_time = started.elapsed();
            if let Ok(mut s) = stats.lock() {
                s.decoded += 1;
                s.decode_time += decode_time;
            }
            if let Ok(mut slot) = latest.lock() {
                *slot = Some(Frame { pixels, width: out_w as u32, height: out_h as u32, captured_at });
            }
            if proxy.send_event(UserEvent::NewFrame).is_err() {
                return Ok(());
            }
        }
        Ok(())
    }

    fn report(seconds: u32, stats: &Mutex<Stats>, proxy: &EventLoopProxy<UserEvent>) {
        let cores = thread::available_parallelism().map_or(1, |n| n.get()) as f64;
        let mut cpu_before = process_cpu_time();
        for second in 1..=seconds {
            thread::sleep(Duration::from_secs(1));
            let cpu_now = process_cpu_time();
            let cpu = (cpu_now - cpu_before).as_secs_f64();
            cpu_before = cpu_now;
            let Ok(mut s) = stats.lock() else { return };
            let avg = |total: Duration, n: u32| total.as_secs_f64() * 1000.0 / f64::from(n.max(1));
            println!(
                "t={second:>3}s  enc {:>2} fps  convert {:>5.1} ms  encode {:>5.1} ms  {:>6.0} kbit/s | \
                 dec {:>5.1} ms | shown {:>2} fps  latency avg {:>5.1} ms  worst {:>5.1} ms | \
                 CPU {:>5.1} % of one core ({:.1} % total)",
                s.encoded,
                avg(s.convert_time, s.encoded),
                avg(s.encode_time, s.encoded),
                s.bytes as f64 * 8.0 / 1000.0,
                avg(s.decode_time, s.decoded),
                s.presented,
                avg(s.latency, s.presented),
                s.worst_latency.as_secs_f64() * 1000.0,
                cpu * 100.0,
                cpu * 100.0 / cores,
            );
            *s = Stats::default();
        }
        let _ = proxy.send_event(UserEvent::Exit);
    }

    struct App {
        latest: Arc<Mutex<Option<Frame>>>,
        stats: Arc<Mutex<Stats>>,
        window: Option<Rc<Window>>,
        surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
        size: (u32, u32),
    }

    impl ApplicationHandler<UserEvent> for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let attributes = Window::default_attributes()
                .with_title("h264-loop (spike 0.5)")
                .with_inner_size(PhysicalSize::new(self.size.0, self.size.1))
                .with_resizable(false);
            let window = match event_loop.create_window(attributes) {
                Ok(window) => Rc::new(window),
                Err(err) => {
                    eprintln!("window: {err}");
                    event_loop.exit();
                    return;
                }
            };
            let surface = softbuffer::Context::new(window.clone())
                .and_then(|context| softbuffer::Surface::new(&context, window.clone()));
            match surface {
                Ok(surface) => self.surface = Some(surface),
                Err(err) => {
                    eprintln!("softbuffer: {err}");
                    event_loop.exit();
                }
            }
            self.window = Some(window);
        }

        fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
            match event {
                UserEvent::NewFrame => {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                UserEvent::Exit => event_loop.exit(),
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
            let (w, h) = (
                NonZeroU32::new(frame.width).context("zero width")?,
                NonZeroU32::new(frame.height).context("zero height")?,
            );
            surface.resize(w, h).map_err(|e| anyhow::anyhow!("{e}"))?;
            let mut buffer = surface.buffer_mut().map_err(|e| anyhow::anyhow!("{e}"))?;
            buffer.copy_from_slice(&frame.pixels);
            buffer.present().map_err(|e| anyhow::anyhow!("{e}"))?;
            let latency = frame.captured_at.elapsed();
            if let Ok(mut s) = self.stats.lock() {
                s.presented += 1;
                s.latency += latency;
                s.worst_latency = s.worst_latency.max(latency);
            }
            Ok(())
        }
    }

    struct Gpu {
        device: ID3D11Device,
        context: ID3D11DeviceContext,
    }

    impl Gpu {
        fn new() -> Result<Self> {
            let (mut device, mut context) = (None, None);
            // SAFETY: plain D3D11 device creation; out-params are valid Options we own.
            unsafe {
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
            }
            Ok(Self {
                device: device.context("no D3D11 device")?,
                context: context.context("no D3D11 context")?,
            })
        }
    }

    /// Desktop Duplication of monitor 0, scaled on the GPU by mip level before CPU readback.
    struct Capture {
        duplication: IDXGIOutputDuplication,
        mips: ID3D11Texture2D,
        mips_view: ID3D11ShaderResourceView,
        staging: ID3D11Texture2D,
        level: u32,
        out_w: u32,
        out_h: u32,
    }

    impl Capture {
        fn new(gpu: &Gpu, level: u32) -> Result<Self> {
            // SAFETY: COM calls on live interfaces; descriptors are fully initialised structs.
            unsafe {
                let adapter = gpu.device.cast::<IDXGIDevice>()?.GetAdapter()?;
                let output: IDXGIOutput1 = adapter.EnumOutputs(0)?.cast()?;
                let duplication = output.DuplicateOutput(&gpu.device)?;
                let desc = duplication.GetDesc();
                let (width, height) = (desc.ModeDesc.Width, desc.ModeDesc.Height);
                // I420 needs even dimensions.
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
                gpu.device.CreateTexture2D(&mips_desc, None, Some(&mut mips))?;
                let mips = mips.context("no mip texture")?;
                let mut mips_view = None;
                gpu.device.CreateShaderResourceView(&mips, None, Some(&mut mips_view))?;

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
                gpu.device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;

                Ok(Self {
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
        fn grab(&self, gpu: &Gpu, timeout: Duration) -> Result<Option<Vec<u8>>> {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            // SAFETY: COM calls on live interfaces; the acquired frame is released before returning.
            unsafe {
                match self
                    .duplication
                    .AcquireNextFrame(timeout.as_millis() as u32, &mut info, &mut resource)
                {
                    Ok(()) => {}
                    Err(err) if err.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                    Err(err) => return Err(err).context("AcquireNextFrame"),
                }
                let result = match (&resource, info.LastPresentTime) {
                    (Some(resource), present) if present != 0 => self.read_back(gpu, resource).map(Some),
                    _ => Ok(None), // pointer-only update
                };
                self.duplication.ReleaseFrame()?;
                result
            }
        }

        unsafe fn read_back(&self, gpu: &Gpu, resource: &IDXGIResource) -> Result<Vec<u8>> {
            // SAFETY: caller holds the acquired frame; mapped rows are read within
            // RowPitch * out_h bytes of a CPU-readable staging texture.
            unsafe {
                let frame: ID3D11Texture2D = resource.cast()?;
                let ctx = &gpu.context;
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
                Ok(pixels)
            }
        }
    }

    fn process_cpu_time() -> Duration {
        let (mut creation, mut exit, mut kernel, mut user) = Default::default();
        // SAFETY: pseudo-handle of the current process; all out-params are valid FILETIMEs.
        let ok = unsafe {
            GetProcessTimes(GetCurrentProcess(), &mut creation, &mut exit, &mut kernel, &mut user)
        };
        if ok.is_err() {
            return Duration::ZERO;
        }
        let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
        Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
    }
}
