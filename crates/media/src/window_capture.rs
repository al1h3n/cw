//! Enumerating and capturing individual application windows — the "share one app" broadcast source.
//!
//! `EnumWindows` lists the top-level windows a teacher might present (the same set Alt+Tab shows);
//! `PrintWindow` with `PW_RENDERFULLCONTENT` copies one window's pixels — including DWM/GPU-composited
//! content such as a browser — into a bitmap we read back as BGRA. Whole-monitor capture lives in
//! [`crate::ThumbnailCapturer`]; this is its per-window counterpart.

use super::CaptureError;

/// One broadcastable application window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// Opaque handle (the `HWND` as an integer) to pass back to [`capture_window_jpeg`].
    pub id: u64,
    /// The window title, for the picker.
    pub title: String,
}

/// Lists the visible, titled top-level windows a teacher could broadcast.
///
/// Filtered to the Alt+Tab set: visible, non-empty title, not a tool window, and its own root owner.
#[must_use]
pub fn list_windows() -> Vec<WindowInfo> {
    imp::list_windows()
}

/// Captures one window as a JPEG no wider than `max_width`, preserving aspect.
///
/// # Errors
/// [`CaptureError`] if the window is gone or its pixels cannot be read.
pub fn capture_window_jpeg(id: u64, max_width: u16) -> Result<Vec<u8>, CaptureError> {
    let (bgra, width, height) = imp::capture_window_bgra(id)?;
    let (dw, dh) = crate::resize::fit_within(width, height, u32::from(max_width.max(1)), u32::MAX);
    let scaled = if dw == width && dh == height {
        bgra
    } else {
        crate::resize::area_average(&bgra, width, height, dw, dh)
    };
    if scaled.is_empty() {
        return Err(CaptureError::new("the window produced an empty frame"));
    }
    crate::encode_bgra(&scaled, dw, dh)
}

/// Captures one window as raw BGRA at native size, for broadcasting frames.
///
/// # Errors
/// [`CaptureError`] if the window is gone or its pixels cannot be read.
pub fn capture_window_bgra(id: u64) -> Result<(Vec<u8>, u32, u32), CaptureError> {
    imp::capture_window_bgra(id)
}

#[cfg(windows)]
mod imp {
    use super::{CaptureError, WindowInfo};

    use windows::{
        Win32::{
            Foundation::{HWND, LPARAM, RECT},
            Graphics::Gdi::{
                BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
                DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC,
                SelectObject,
            },
            Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow},
            UI::WindowsAndMessaging::{
                EnumWindows, GA_ROOTOWNER, GWL_EXSTYLE, GetAncestor, GetWindowLongW, GetWindowRect,
                GetWindowTextLengthW, GetWindowTextW, IsWindowVisible, PW_RENDERFULLCONTENT,
                WS_EX_TOOLWINDOW,
            },
        },
        core::BOOL,
    };

    /// A sane cap so a machine with hundreds of windows cannot produce an enormous list.
    const MAX_WINDOWS: usize = 100;
    /// The largest window we will read back, so a huge 8K window cannot allocate wildly.
    const MAX_DIM: i32 = 8192;

    pub fn list_windows() -> Vec<WindowInfo> {
        let mut windows: Vec<WindowInfo> = Vec::new();
        // SAFETY: EnumWindows calls our callback synchronously with a pointer to `windows`; the
        // pointer is valid for the duration of the call and used by nothing else.
        unsafe {
            let _ = EnumWindows(
                Some(enum_proc),
                LPARAM(&mut windows as *mut Vec<WindowInfo> as isize),
            );
        }
        windows
    }

    extern "system" fn enum_proc(window: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `&mut Vec` pointer we passed to EnumWindows, still alive.
        let out = unsafe { &mut *(lparam.0 as *mut Vec<WindowInfo>) };
        if out.len() >= MAX_WINDOWS {
            return BOOL(0); // stop enumerating
        }
        // SAFETY: `window` is a live handle supplied by EnumWindows; all calls are read-only queries.
        unsafe {
            if !IsWindowVisible(window).as_bool() {
                return BOOL(1);
            }
            // Only unowned top-level windows — the Alt+Tab set — not child/owned popups.
            if GetAncestor(window, GA_ROOTOWNER) != window {
                return BOOL(1);
            }
            // Skip tool windows (floating palettes, tray helpers).
            let ex_style = GetWindowLongW(window, GWL_EXSTYLE) as u32;
            if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
                return BOOL(1);
            }
            let len = GetWindowTextLengthW(window);
            if len <= 0 {
                return BOOL(1);
            }
            let mut buf = vec![0u16; (len + 1) as usize];
            let written = GetWindowTextW(window, &mut buf);
            if written <= 0 {
                return BOOL(1);
            }
            let title = String::from_utf16_lossy(&buf[..written as usize]);
            if title.trim().is_empty() {
                return BOOL(1);
            }
            out.push(WindowInfo {
                id: window.0 as u64,
                title,
            });
        }
        BOOL(1)
    }

    pub fn capture_window_bgra(id: u64) -> Result<(Vec<u8>, u32, u32), CaptureError> {
        let window = HWND(id as *mut std::ffi::c_void);
        // SAFETY: GDI capture of a window. Every object created is freed on every path; the pixel
        // buffer is sized to width*height*4 before GetDIBits writes it.
        unsafe {
            let mut rect = RECT::default();
            GetWindowRect(window, &mut rect).map_err(CaptureError::new)?;
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            if width <= 0 || height <= 0 || width > MAX_DIM || height > MAX_DIM {
                return Err(CaptureError::new("the window has no drawable area"));
            }

            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let old = SelectObject(mem, HGDIOBJ(bitmap.0));

            // PW_RENDERFULLCONTENT (2) makes DWM/GPU windows (browsers, UWP) render into our DC.
            let printed = PrintWindow(window, mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool();

            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
                    biWidth: width,
                    biHeight: -height, // top-down, matching the rest of the pipeline
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: 0, // BI_RGB
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
            let lines = GetDIBits(
                mem,
                bitmap,
                0,
                u32::try_from(height).unwrap_or(0),
                Some(pixels.as_mut_ptr().cast()),
                &mut bmi,
                DIB_RGB_COLORS,
            );

            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);

            if !printed || lines == 0 {
                return Err(CaptureError::new("the window could not be captured"));
            }
            Ok((
                pixels,
                u32::try_from(width).unwrap_or(0),
                u32::try_from(height).unwrap_or(0),
            ))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{CaptureError, WindowInfo};

    // ponytail: Linux/macOS window capture (XComposite / CGWindowList) lands with the rest of Phase 5.
    pub fn list_windows() -> Vec<WindowInfo> {
        Vec::new()
    }

    pub fn capture_window_bgra(_id: u64) -> Result<(Vec<u8>, u32, u32), CaptureError> {
        Err(CaptureError::new("window capture is not supported on this platform"))
    }
}
