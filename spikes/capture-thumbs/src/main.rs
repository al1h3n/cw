//! Spike 0.4 — how cheap can change-only thumbnails be?
//!
//! ```text
//! capture-thumbs [SECONDS=30] [MAX_WIDTH=480] [QUALITY=60]
//! ```
//!
//! Pipeline, once per second:
//! 1. DXGI Desktop Duplication `AcquireNextFrame(0)`. Timeout or pointer-only update = nothing changed,
//!    nothing to send. DDA accumulates changes between calls, so polling at 1 Hz loses nothing.
//! 2. Copy the frame into a texture with a mip chain, `GenerateMips` on the GPU (box filter).
//! 3. Copy one mip level (width <= MAX_WIDTH) to a small staging texture, map it, JPEG-encode on the CPU.
//!
//! Thumbnails go to `spikes/out/`. The report prints CPU use of this process. Monitor 0 only;
//! rotation ignored.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    windows_impl::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("capture-thumbs is Windows-only");
}

#[cfg(windows)]
mod windows_impl {
    use std::{
        path::Path,
        thread::sleep,
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result};
    use jpeg_encoder::{ColorType, Encoder};
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

    pub fn run() -> Result<()> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let arg = |i: usize, default: u32| args.get(i).and_then(|s| s.parse().ok()).unwrap_or(default);
        let (seconds, max_width, quality) = (arg(0, 30), arg(1, 480), arg(2, 60) as u8);

        let out_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../out");
        std::fs::create_dir_all(&out_dir)?;

        let gpu = Gpu::new()?;
        let mut capture = Capture::new(&gpu, max_width)?;
        println!(
            "monitor {}x{} -> thumbnail {}x{} (mip level {}), {seconds}s, quality {quality}",
            capture.width, capture.height, capture.thumb_w, capture.thumb_h, capture.level
        );

        let cpu_start = process_cpu_time();
        let wall_start = Instant::now();
        let (mut sent, mut bytes, mut encode_time) = (0u32, 0usize, Duration::ZERO);

        for tick in 0..seconds {
            let next_tick = wall_start + Duration::from_secs(u64::from(tick) + 1);
            match capture.grab_if_changed(&gpu) {
                Ok(Some(pixels)) => {
                    let started = Instant::now();
                    let mut jpeg = Vec::new();
                    Encoder::new(&mut jpeg, quality).encode(
                        &pixels,
                        capture.thumb_w as u16,
                        capture.thumb_h as u16,
                        ColorType::Bgra,
                    )?;
                    encode_time += started.elapsed();
                    std::fs::write(out_dir.join(format!("thumb-{tick:03}.jpg")), &jpeg)?;
                    sent += 1;
                    bytes += jpeg.len();
                    println!("t={tick:>3}s  changed  {:>6} bytes", jpeg.len());
                }
                Ok(None) => println!("t={tick:>3}s  unchanged"),
                Err(err) if err.code() == DXGI_ERROR_ACCESS_LOST => {
                    // Desktop switch (UAC, lock screen) or mode change: recreate the duplication.
                    println!("t={tick:>3}s  access lost, recreating");
                    capture = Capture::new(&gpu, max_width)?;
                }
                Err(err) => return Err(err).context("capture"),
            }
            sleep(next_tick.saturating_duration_since(Instant::now()));
        }

        let wall = wall_start.elapsed();
        let cpu = process_cpu_time() - cpu_start;
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        println!("\n--- report ---");
        println!("thumbnails sent: {sent}/{seconds}, avg size {} bytes", bytes / sent.max(1) as usize);
        println!(
            "avg bandwidth: {:.1} kbit/s",
            bytes as f64 * 8.0 / 1000.0 / wall.as_secs_f64()
        );
        println!("avg JPEG encode: {:?}", encode_time / sent.max(1));
        // GetProcessTimes advances in scheduler ticks (~15.6 ms), so tiny loads can read as 0.
        let cpu_or_tick = cpu.max(Duration::from_micros(15_625));
        println!(
            "process CPU: {} ms in {:.1} s  =>  <= {:.3} % of one core, <= {:.4} % of all {cores} cores",
            cpu.as_millis(),
            wall.as_secs_f64(),
            cpu_or_tick.as_secs_f64() / wall.as_secs_f64() * 100.0,
            cpu_or_tick.as_secs_f64() / wall.as_secs_f64() * 100.0 / cores as f64
        );
        Ok(())
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

    struct Capture {
        duplication: IDXGIOutputDuplication,
        mips: ID3D11Texture2D,
        mips_view: ID3D11ShaderResourceView,
        staging: ID3D11Texture2D,
        width: u32,
        height: u32,
        level: u32,
        thumb_w: u32,
        thumb_h: u32,
    }

    impl Capture {
        fn new(gpu: &Gpu, max_width: u32) -> Result<Self> {
            // SAFETY: COM calls on live interfaces; descriptors are fully initialised structs.
            unsafe {
                let adapter = gpu.device.cast::<IDXGIDevice>()?.GetAdapter()?;
                let output: IDXGIOutput1 = adapter.EnumOutputs(0)?.cast()?;
                let duplication = output.DuplicateOutput(&gpu.device)?;
                let desc = duplication.GetDesc();
                let (width, height) = (desc.ModeDesc.Width, desc.ModeDesc.Height);

                // Smallest mip level whose width fits: 1920 -> level 2 (480), 1366 -> level 2 (341).
                let level = (0..16).find(|l| width >> l <= max_width).unwrap_or(0);
                let (thumb_w, thumb_h) = ((width >> level).max(1), (height >> level).max(1));

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
                    Width: thumb_w,
                    Height: thumb_h,
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
                    width,
                    height,
                    level,
                    thumb_w,
                    thumb_h,
                })
            }
        }

        /// Returns tightly packed BGRA thumbnail pixels, or `None` when the screen image did not change.
        fn grab_if_changed(&self, gpu: &Gpu) -> windows::core::Result<Option<Vec<u8>>> {
            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;
            // SAFETY: COM calls on live interfaces; every acquired frame is released below.
            unsafe {
                match self.duplication.AcquireNextFrame(0, &mut info, &mut resource) {
                    Ok(()) => {}
                    Err(err) if err.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                    Err(err) => return Err(err),
                }
                // LastPresentTime == 0 means only the mouse pointer moved.
                let result = if info.LastPresentTime == 0 {
                    Ok(None)
                } else {
                    self.downscale(gpu, resource.as_ref()).map(Some)
                };
                self.duplication.ReleaseFrame()?;
                result
            }
        }

        unsafe fn downscale(
            &self,
            gpu: &Gpu,
            resource: Option<&IDXGIResource>,
        ) -> windows::core::Result<Vec<u8>> {
            let Some(resource) = resource else {
                return Ok(Vec::new());
            };
            // SAFETY: caller holds the acquired frame; staging texture is CPU-readable and
            // mapped rows are read within RowPitch * thumb_h bytes.
            unsafe {
                let frame: ID3D11Texture2D = resource.cast()?;
                let ctx = &gpu.context;
                ctx.CopySubresourceRegion(&self.mips, 0, 0, 0, 0, &frame, 0, None);
                ctx.GenerateMips(&self.mips_view);
                ctx.CopySubresourceRegion(&self.staging, 0, 0, 0, 0, &self.mips, self.level, None);

                let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
                ctx.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
                let row_bytes = self.thumb_w as usize * 4;
                let mut pixels = Vec::with_capacity(row_bytes * self.thumb_h as usize);
                for row in 0..self.thumb_h as usize {
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
