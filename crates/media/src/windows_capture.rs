//! Windows thumbnails: DXGI Desktop Duplication → GPU mip downscale → JPEG.
//!
//! The duplication is created once and kept warm, because creating it is the expensive part and the
//! first `AcquireNextFrame` after creation usually reports "no new frame yet". A capture waits briefly
//! for a fresh frame; if the screen has not changed it returns the **last** thumbnail, which is the
//! correct answer for a change-only thumbnail feed and costs nothing.

use std::time::{Duration, Instant};

use jpeg_encoder::{ColorType, Encoder};
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

use crate::{CaptureError, MonitorInfo};

/// How long a capture waits for a changed frame before falling back to the cached thumbnail.
const FRAME_WAIT: Duration = Duration::from_millis(400);
/// JPEG quality for thumbnails; 60 measured ~5–7 KB at 320×180 in spike 0.4.
const QUALITY: u8 = 60;

/// Captures downscaled JPEG thumbnails of a monitor.
pub struct ThumbnailCapturer {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    monitors: Vec<MonitorInfo>,
    /// The live duplication, rebuilt when the monitor or scale changes or access is lost.
    active: Option<Duplication>,
    /// Last successfully encoded thumbnail, reused when the screen has not changed.
    last: Option<(u8, u16, Vec<u8>)>,
}

/// One monitor's duplication plus the scratch textures sized for a given output width.
struct Duplication {
    monitor: u8,
    max_width: u16,
    duplication: IDXGIOutputDuplication,
    mips: ID3D11Texture2D,
    mips_view: ID3D11ShaderResourceView,
    staging: ID3D11Texture2D,
    level: u32,
    out_w: u32,
    out_h: u32,
}

impl ThumbnailCapturer {
    /// Creates a capturer bound to the default graphics adapter.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if Direct3D or DXGI are unavailable.
    pub fn new() -> Result<Self, CaptureError> {
        let (mut device, mut context) = (None, None);
        // SAFETY: plain D3D11 device creation; the out-params are Options we own.
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
            )
            .map_err(CaptureError::new)?;
        }
        let device = device.ok_or_else(|| CaptureError("no D3D11 device".into()))?;
        let context = context.ok_or_else(|| CaptureError("no D3D11 context".into()))?;
        let monitors = enumerate_outputs(&device)?;
        Ok(Self {
            device,
            context,
            monitors,
            active: None,
            last: None,
        })
    }

    /// How many monitors are attached.
    #[must_use]
    pub fn monitor_count(&self) -> u8 {
        u8::try_from(self.monitors.len()).unwrap_or(u8::MAX)
    }

    /// Every attached monitor, with its native size.
    #[must_use]
    pub fn monitors(&self) -> Vec<MonitorInfo> {
        self.monitors.clone()
    }

    /// Captures `monitor`, downscaled to at most `max_width` pixels wide, as JPEG bytes.
    ///
    /// If the screen has not changed since the previous call, the previous thumbnail is returned
    /// unchanged (no encode, no GPU work).
    ///
    /// # Errors
    /// Returns [`CaptureError`] if the monitor is missing or capture fails with no cached frame.
    pub fn capture_jpeg(&mut self, monitor: u8, max_width: u16) -> Result<Vec<u8>, CaptureError> {
        if usize::from(monitor) >= self.monitors.len() {
            return Err(CaptureError(format!("monitor {monitor} not attached")));
        }
        // Rebuild the duplication when the target or scale changes.
        let stale = self
            .active
            .as_ref()
            .is_none_or(|d| d.monitor != monitor || d.max_width != max_width);
        if stale {
            self.last = None;
            self.active = Some(Duplication::new(&self.device, monitor, max_width)?);
        }

        match self.grab(monitor, max_width) {
            Ok(Some(jpeg)) => Ok(jpeg),
            // Duplication only reports changes, so an idle screen yields nothing: serve the previous
            // thumbnail if we have one, else fall back to a GDI grab of the current desktop.
            Ok(None) => match self.cached(monitor, max_width) {
                Some(jpeg) => Ok(jpeg),
                None => self.gdi_fallback(monitor, max_width),
            },
            Err(err) => {
                // Access is lost on desktop switches (UAC, lock screen) and mode changes: rebuild once.
                self.active = Some(Duplication::new(&self.device, monitor, max_width)?);
                match self.grab(monitor, max_width) {
                    Ok(Some(jpeg)) => Ok(jpeg),
                    Ok(None) => match self.cached(monitor, max_width) {
                        Some(jpeg) => Ok(jpeg),
                        None => self.gdi_fallback(monitor, max_width).map_err(|_| err),
                    },
                    Err(err) => Err(err),
                }
            }
        }
    }

    /// Captures `monitor` as raw BGRA pixels at (close to) its native size.
    ///
    /// Recording needs the *unscaled* frame, because the exact output size is reached afterwards by
    /// [`crate::resize::area_average`] — resizing twice, once by the GPU's whole-number mip step and
    /// once properly, would throw away detail for nothing.
    ///
    /// `ponytail:` this takes the GDI path, which is a straightforward BitBlt of the monitor's
    /// rectangle and costs a few milliseconds at 1440p — fine at the 1–30 fps a lesson recording
    /// uses. Wiring it into the Desktop Duplication path is the upgrade if a higher rate is ever
    /// wanted; the recorder above would not change.
    ///
    /// # Errors
    /// Returns [`CaptureError`] if the monitor is missing or the grab fails.
    pub fn capture_bgra(&mut self, monitor: u8) -> Result<(Vec<u8>, u32, u32), CaptureError> {
        if usize::from(monitor) >= self.monitors.len() {
            return Err(CaptureError(format!("monitor {monitor} not attached")));
        }
        let area = output_area(&self.device, monitor)
            .ok_or_else(|| CaptureError(format!("monitor {monitor} has no desktop area")))?;
        // u16::MAX as the cap means "do not downscale": the caller resizes properly.
        crate::gdi::capture_area_bgra(area, u16::MAX)
    }

    /// Grabs this monitor's area with GDI and encodes it, caching it like a normal capture.
    ///
    /// The rectangle comes from the output description, so a second monitor gets *its own* pixels
    /// rather than the primary screen's.
    fn gdi_fallback(&mut self, monitor: u8, max_width: u16) -> Result<Vec<u8>, CaptureError> {
        let area = output_area(&self.device, monitor)
            .ok_or_else(|| CaptureError(format!("monitor {monitor} has no desktop area")))?;
        let (pixels, w, h) = crate::gdi::capture_area_bgra(area, max_width)?;
        let jpeg = encode_bgra(&pixels, w, h)?;
        self.last = Some((monitor, max_width, jpeg.clone()));
        Ok(jpeg)
    }

    fn cached(&self, monitor: u8, max_width: u16) -> Option<Vec<u8>> {
        self.last
            .as_ref()
            .filter(|(m, w, _)| *m == monitor && *w == max_width)
            .map(|(_, _, jpeg)| jpeg.clone())
    }

    /// Waits briefly for a changed frame and encodes it. `Ok(None)` means "nothing changed".
    ///
    /// Only the *first* capture waits: after that a cached thumbnail is already available, so an
    /// unchanged screen answers immediately instead of burning the timeout on every request.
    fn grab(&mut self, monitor: u8, max_width: u16) -> Result<Option<Vec<u8>>, CaptureError> {
        let wait = if self.last.is_none() {
            FRAME_WAIT
        } else {
            Duration::ZERO
        };
        let deadline = Instant::now() + wait;
        loop {
            let Some(active) = self.active.as_ref() else {
                return Err(CaptureError("no duplication".into()));
            };
            // With no thumbnail yet we must return *something*, so take the first frame we can get
            // even if the desktop has not changed (a pointer-only update still carries the full image).
            let force = self.last.is_none();
            let pixels = active.next_frame(&self.context, force)?;
            if let Some((pixels, w, h)) = pixels {
                let jpeg = encode_bgra(&pixels, w, h)?;
                self.last = Some((monitor, max_width, jpeg.clone()));
                return Ok(Some(jpeg));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
        }
    }
}

/// Encodes packed BGRA pixels as JPEG.
/// Encodes packed BGRA pixels as JPEG. Public so the recorder can encode a resized frame.
pub fn encode_bgra(pixels: &[u8], width: u32, height: u32) -> Result<Vec<u8>, CaptureError> {
    let mut jpeg = Vec::new();
    Encoder::new(&mut jpeg, QUALITY)
        .encode(pixels, width as u16, height as u16, ColorType::Bgra)
        .map_err(CaptureError::new)?;
    Ok(jpeg)
}

/// Describes every attached output on the device's adapter.
fn enumerate_outputs(device: &ID3D11Device) -> Result<Vec<MonitorInfo>, CaptureError> {
    // SAFETY: COM queries on a live device; EnumOutputs errors once the outputs run out.
    unsafe {
        let adapter = device
            .cast::<IDXGIDevice>()
            .map_err(CaptureError::new)?
            .GetAdapter()
            .map_err(CaptureError::new)?;
        let mut monitors = Vec::new();
        for index in 0..u32::from(u8::MAX) {
            let Ok(output) = adapter.EnumOutputs(index) else {
                break;
            };
            let Ok(desc) = output.GetDesc() else { continue };
            let area = desc.DesktopCoordinates;
            monitors.push(MonitorInfo {
                index: index as u8,
                width: (area.right - area.left).unsigned_abs(),
                height: (area.bottom - area.top).unsigned_abs(),
                // The primary monitor is the one whose top-left is the desktop origin.
                primary: area.left == 0 && area.top == 0,
            });
        }
        Ok(monitors)
    }
}

/// The desktop rectangle of one output, in virtual-screen coordinates (for the GDI fallback).
fn output_area(device: &ID3D11Device, monitor: u8) -> Option<(i32, i32, i32, i32)> {
    // SAFETY: COM queries on a live device.
    unsafe {
        let adapter = device.cast::<IDXGIDevice>().ok()?.GetAdapter().ok()?;
        let output = adapter.EnumOutputs(u32::from(monitor)).ok()?;
        let area = output.GetDesc().ok()?.DesktopCoordinates;
        Some((
            area.left,
            area.top,
            area.right - area.left,
            area.bottom - area.top,
        ))
    }
}

impl Duplication {
    fn new(device: &ID3D11Device, monitor: u8, max_width: u16) -> Result<Self, CaptureError> {
        // SAFETY: COM calls on live interfaces; every descriptor below is fully initialised.
        unsafe {
            let adapter = device
                .cast::<IDXGIDevice>()
                .map_err(CaptureError::new)?
                .GetAdapter()
                .map_err(CaptureError::new)?;
            let output: IDXGIOutput1 = adapter
                .EnumOutputs(u32::from(monitor))
                .map_err(CaptureError::new)?
                .cast()
                .map_err(CaptureError::new)?;
            let duplication = output.DuplicateOutput(device).map_err(CaptureError::new)?;
            let desc = duplication.GetDesc();
            let (width, height) = (desc.ModeDesc.Width, desc.ModeDesc.Height);

            // Smallest GPU mip level whose width fits the request (a cheap box filter).
            let level = (0..16u32)
                .find(|l| width >> l <= u32::from(max_width))
                .unwrap_or(0);
            let (out_w, out_h) = ((width >> level).max(1), (height >> level).max(1));

            let mips_desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: level + 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
            };
            let mut mips = None;
            device
                .CreateTexture2D(&mips_desc, None, Some(&mut mips))
                .map_err(CaptureError::new)?;
            let mips = mips.ok_or_else(|| CaptureError("no mip texture".into()))?;
            let mut mips_view = None;
            device
                .CreateShaderResourceView(&mips, None, Some(&mut mips_view))
                .map_err(CaptureError::new)?;

            let staging_desc = D3D11_TEXTURE2D_DESC {
                Width: out_w,
                Height: out_h,
                MipLevels: 1,
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
                ..mips_desc
            };
            let mut staging = None;
            device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .map_err(CaptureError::new)?;

            Ok(Self {
                monitor,
                max_width,
                duplication,
                mips,
                mips_view: mips_view
                    .ok_or_else(|| CaptureError("no shader resource view".into()))?,
                staging: staging.ok_or_else(|| CaptureError("no staging texture".into()))?,
                level,
                out_w,
                out_h,
            })
        }
    }

    /// Acquires one frame if the desktop changed, returning packed BGRA pixels.
    ///
    /// With `force`, any acquired frame is accepted even when only the pointer moved. That is needed
    /// for the very first capture: a completely static desktop never reports a content change, but the
    /// acquired surface still holds the current screen.
    fn next_frame(
        &self,
        context: &ID3D11DeviceContext,
        force: bool,
    ) -> Result<Option<(Vec<u8>, u32, u32)>, CaptureError> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        // SAFETY: COM calls on live interfaces; the acquired frame is always released below, and the
        // mapped staging rows are read within RowPitch * out_h bytes.
        unsafe {
            match self
                .duplication
                .AcquireNextFrame(50, &mut info, &mut resource)
            {
                Ok(()) => {}
                Err(err) if err.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
                Err(err) => return Err(CaptureError::new(err)),
            }
            let result = match (&resource, info.LastPresentTime) {
                // LastPresentTime == 0 means only the mouse pointer moved: not a real change, so it is
                // skipped unless we still owe the caller a first frame.
                (Some(resource), present) if present != 0 || force => {
                    self.read_back(context, resource).map(Some)
                }
                _ => Ok(None),
            };
            let _ = self.duplication.ReleaseFrame();
            result
        }
    }

    unsafe fn read_back(
        &self,
        context: &ID3D11DeviceContext,
        resource: &IDXGIResource,
    ) -> Result<(Vec<u8>, u32, u32), CaptureError> {
        // SAFETY: the caller holds the acquired frame for the duration of these calls.
        unsafe {
            let frame: ID3D11Texture2D = resource.cast().map_err(CaptureError::new)?;
            if self.level == 0 {
                context.CopyResource(&self.staging, &frame);
            } else {
                context.CopySubresourceRegion(&self.mips, 0, 0, 0, 0, &frame, 0, None);
                context.GenerateMips(&self.mips_view);
                context.CopySubresourceRegion(
                    &self.staging,
                    0,
                    0,
                    0,
                    0,
                    &self.mips,
                    self.level,
                    None,
                );
            }
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context
                .Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(CaptureError::new)?;
            let row_bytes = self.out_w as usize * 4;
            let mut pixels = Vec::with_capacity(row_bytes * self.out_h as usize);
            for row in 0..self.out_h as usize {
                let src = (mapped.pData as *const u8).add(row * mapped.RowPitch as usize);
                pixels.extend_from_slice(std::slice::from_raw_parts(src, row_bytes));
            }
            context.Unmap(&self.staging, 0);
            Ok((pixels, self.out_w, self.out_h))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_a_jpeg_thumbnail_of_the_primary_monitor() {
        let mut capturer = match ThumbnailCapturer::new() {
            Ok(c) => c,
            // A build machine with no GPU/desktop cannot capture; skip rather than fail CI.
            Err(err) => {
                eprintln!("skipping: {err}");
                return;
            }
        };
        assert!(
            capturer.monitor_count() >= 1,
            "at least one monitor expected"
        );

        // Retry a little: the very first duplication may need a screen change to produce a frame.
        let mut jpeg = Vec::new();
        for attempt in 0..5 {
            match capturer.capture_jpeg(0, 320) {
                Ok(bytes) => {
                    jpeg = bytes;
                    break;
                }
                Err(err) => eprintln!("attempt {attempt}: {err}"),
            }
        }
        assert!(!jpeg.is_empty(), "expected a thumbnail within a few tries");
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8], "JPEG SOI marker");
        assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9], "JPEG EOI marker");
        assert!(
            jpeg.len() < 60_000,
            "a 320px thumbnail should be small, got {}",
            jpeg.len()
        );
    }
}
