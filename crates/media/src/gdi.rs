//! GDI screen grab: the always-available fallback.
//!
//! Desktop Duplication only delivers *changes*, so on a completely idle screen it returns nothing —
//! yet a Console asking for a thumbnail still needs an image. `BitBlt` of the screen DC always
//! produces the current composited desktop, so it covers the first request and any idle period.
//! It is slower than the GPU path (a full-screen blit plus a CPU downscale), which is why it is only
//! used when Duplication has nothing to give.
//!
//! ponytail: primary monitor only, and nearest-neighbour downscale. Per-monitor GDI capture needs
//! monitor rectangles (EnumDisplayMonitors); add it when multi-monitor thumbnails are wired up.

use windows::Win32::{
    Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    },
    UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN},
};

use crate::CaptureError;

/// Captures the primary monitor and returns packed BGRA pixels downscaled to at most `max_width`.
pub fn capture_primary_bgra(max_width: u16) -> Result<(Vec<u8>, u32, u32), CaptureError> {
    // SAFETY: every GDI object created here is selected back and deleted before returning; the
    // pixel buffer is sized width*height*4, exactly what GetDIBits fills for a 32-bit top-down DIB.
    unsafe {
        let (width, height) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        if width <= 0 || height <= 0 {
            return Err(CaptureError("no primary monitor".into()));
        }
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err(CaptureError(
                "could not get the screen device context".into(),
            ));
        }
        let memory = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let previous = SelectObject(memory, bitmap.into());

        let blit = BitBlt(memory, 0, 0, width, height, Some(screen), 0, 0, SRCCOPY);

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // negative: top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
        let copied = if blit.is_ok() {
            GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                Some(pixels.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            )
        } else {
            0
        };

        SelectObject(memory, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);

        if blit.is_err() || copied == 0 {
            return Err(CaptureError("GDI screen copy failed".into()));
        }
        Ok(downscale(&pixels, width as u32, height as u32, max_width))
    }
}

/// Nearest-neighbour downscale by an integer factor, keeping the aspect ratio.
fn downscale(pixels: &[u8], width: u32, height: u32, max_width: u16) -> (Vec<u8>, u32, u32) {
    let max_width = u32::from(max_width).max(1);
    let step = width.div_ceil(max_width).max(1);
    let (out_w, out_h) = ((width / step).max(1), (height / step).max(1));
    let mut out = Vec::with_capacity((out_w * out_h * 4) as usize);
    for y in 0..out_h {
        let row = (y * step) as usize * width as usize * 4;
        for x in 0..out_w {
            let i = row + (x * step) as usize * 4;
            out.extend_from_slice(&pixels[i..i + 4]);
        }
    }
    (out, out_w, out_h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscale_halves_and_keeps_four_channels() {
        // 4x2 image, step 2 -> 2x1.
        let pixels: Vec<u8> = (0..4 * 2 * 4).map(|i| i as u8).collect();
        let (out, w, h) = downscale(&pixels, 4, 2, 2);
        assert_eq!((w, h), (2, 1));
        assert_eq!(out.len(), 2 * 4, "2 pixels x 4 channels");
        assert_eq!(&out[..4], &pixels[..4], "first pixel is sampled as-is");
    }

    #[test]
    fn downscale_never_upscales_or_divides_by_zero() {
        let pixels = vec![9u8; 2 * 2 * 4];
        let (out, w, h) = downscale(&pixels, 2, 2, 4096);
        assert_eq!((w, h), (2, 2), "smaller than the cap stays unchanged");
        assert_eq!(out.len(), pixels.len());
    }
}
