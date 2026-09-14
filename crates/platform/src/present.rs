//! Showing the teacher's screen full-screen on a student PC (broadcast / presentation mode).
//!
//! A borderless, always-on-top window covering the whole screen, owned by its own thread with its
//! own message loop — a Windows window must be pumped by the thread that created it, and the Agent's
//! other work must not stall while a broadcast is on screen.
//!
//! Frames arrive as BGRA and are drawn with `StretchDIBits`, which scales the teacher's resolution
//! to the student's for free: a 1440p teacher broadcasting to a 1080p lab needs no special case.
//!
//! ## What this does and does not stop
//!
//! The window covers the screen, takes focus and stays on top, so a student sees the broadcast and
//! their own work is hidden. It does **not** yet make the PC impossible to escape: Alt+Tab and the
//! Windows key still work, because truly trapping a session needs the separate Win32 desktop from
//! spike 0.8 (`CreateDesktop` + `SwitchDesktop`) that the exam lock screen will use — that is
//! FEATURES #4/#10, not this. Presented honestly rather than implied: this is "everyone look at my
//! screen", not "nobody can do anything else".

/// Errors showing a broadcast.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PresentError {
    /// The window could not be created.
    #[error("could not open the presentation window: {0}")]
    Failed(String),
    /// Not implemented on this platform yet.
    #[error("presentation is not supported on this platform")]
    NotSupported,
}

/// A full-screen window showing frames pushed into it.
pub struct Presenter(imp::Presenter);

impl Presenter {
    /// Opens the full-screen window. It shows a "waiting" black screen until the first frame.
    ///
    /// # Errors
    /// [`PresentError`] if the window cannot be created.
    pub fn open() -> Result<Self, PresentError> {
        imp::Presenter::open().map(Self)
    }

    /// Replaces what is on screen. `pixels` is packed BGRA, `width` × `height`.
    ///
    /// Dropping a frame is always better than queueing one: the newest picture is the only one
    /// worth showing, so this replaces rather than appends.
    pub fn show(&self, pixels: Vec<u8>, width: u32, height: u32) {
        self.0.show(pixels, width, height);
    }
}

#[cfg(windows)]
mod imp {
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicIsize, Ordering},
        },
        thread::JoinHandle,
    };

    use windows::{
        Win32::{
            Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM},
            Graphics::Gdi::{
                BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, DIB_RGB_COLORS, EndPaint, HBRUSH,
                InvalidateRect, PAINTSTRUCT, SRCCOPY, StretchDIBits,
            },
            UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetSystemMetrics,
                HMENU, MSG, PostMessageW, PostQuitMessage, RegisterClassExW, SM_CXSCREEN,
                SM_CYSCREEN, SW_SHOW, SetForegroundWindow, ShowWindow, TranslateMessage, WM_CLOSE,
                WM_DESTROY, WM_PAINT, WNDCLASSEXW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
            },
        },
        core::{PCWSTR, w},
    };

    use super::PresentError;

    /// The frame currently on screen, shared with the window thread.
    struct Frame {
        pixels: Vec<u8>,
        width: i32,
        height: i32,
    }

    /// The one frame any presentation window is showing.
    ///
    /// A single global rather than per-window state, because a student PC shows at most one
    /// broadcast at a time — a second would have nowhere to go.
    static FRAME: Mutex<Option<Frame>> = Mutex::new(None);

    pub struct Presenter {
        /// The window handle, as an integer so it can cross threads; 0 until created.
        window: Arc<AtomicIsize>,
        closing: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Presenter {
        pub fn open() -> Result<Self, PresentError> {
            let window = Arc::new(AtomicIsize::new(0));
            let closing = Arc::new(AtomicBool::new(false));
            let ready = Arc::new(AtomicBool::new(false));
            let failed = Arc::new(Mutex::new(String::new()));

            let thread = {
                let window = Arc::clone(&window);
                let ready = Arc::clone(&ready);
                let failed = Arc::clone(&failed);
                std::thread::spawn(move || match create_window() {
                    Ok(handle) => {
                        window.store(handle.0 as isize, Ordering::SeqCst);
                        ready.store(true, Ordering::SeqCst);
                        pump_messages();
                    }
                    Err(err) => {
                        *failed.lock().unwrap_or_else(|e| e.into_inner()) = err;
                        ready.store(true, Ordering::SeqCst);
                    }
                })
            };

            // Wait for the window thread to say whether it got a window.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !ready.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let problem = failed.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if !problem.is_empty() {
                return Err(PresentError::Failed(problem));
            }
            if window.load(Ordering::SeqCst) == 0 {
                return Err(PresentError::Failed("the window did not open".into()));
            }

            Ok(Self {
                window,
                closing,
                thread: Some(thread),
            })
        }

        pub fn show(&self, pixels: Vec<u8>, width: u32, height: u32) {
            if self.closing.load(Ordering::SeqCst) {
                return;
            }
            {
                let mut frame = FRAME.lock().unwrap_or_else(|e| e.into_inner());
                *frame = Some(Frame {
                    pixels,
                    width: width as i32,
                    height: height as i32,
                });
            }
            let handle = HWND(self.window.load(Ordering::SeqCst) as *mut std::ffi::c_void);
            if !handle.is_invalid() {
                // SAFETY: a valid window handle; asking for a repaint is thread-safe by design —
                // Windows posts the paint to the owning thread.
                unsafe {
                    let _ = InvalidateRect(Some(handle), None, false);
                }
            }
        }
    }

    impl Drop for Presenter {
        fn drop(&mut self) {
            self.closing.store(true, Ordering::SeqCst);
            let handle = HWND(self.window.load(Ordering::SeqCst) as *mut std::ffi::c_void);
            if !handle.is_invalid() {
                // `DestroyWindow` only works on the thread that created the window — calling it
                // from here would quietly fail and leave a full-screen window covering the
                // student's desktop with no way to close it. `PostMessageW` is explicitly
                // thread-safe, and the default handler turns WM_CLOSE into the destroy we want.
                // SAFETY: a valid window handle and a documented message with no parameters.
                unsafe {
                    let _ = PostMessageW(Some(handle), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            *FRAME.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    /// Creates the borderless full-screen window on the calling thread.
    fn create_window() -> Result<HWND, String> {
        const CLASS: PCWSTR = w!("CowatcherPresent");
        // SAFETY: a standard window-class registration and creation. Registering twice is harmless
        // (the second call fails and we proceed), and every argument is a constant or a valid
        // pointer that outlives the call.
        unsafe {
            let class = WNDCLASSEXW {
                cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(0),
                lpfnWndProc: Some(window_proc),
                lpszClassName: CLASS,
                hbrBackground: HBRUSH(std::ptr::null_mut()),
                ..Default::default()
            };
            let _ = RegisterClassExW(&class);

            let width = GetSystemMetrics(SM_CXSCREEN);
            let height = GetSystemMetrics(SM_CYSCREEN);
            let window = CreateWindowExW(
                WS_EX_TOPMOST,
                CLASS,
                w!("Presentation"),
                WS_POPUP | WS_VISIBLE,
                0,
                0,
                width,
                height,
                None,
                None::<HMENU>,
                None,
                None,
            )
            .map_err(|e| e.message())?;

            let _ = ShowWindow(window, SW_SHOW);
            let _ = SetForegroundWindow(window);
            Ok(window)
        }
    }

    /// Runs the window's message loop until the window is destroyed.
    fn pump_messages() {
        let mut message = MSG::default();
        // SAFETY: the standard Win32 message loop; GetMessageW returns 0 when WM_QUIT arrives.
        unsafe {
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    /// Paints the newest frame, stretched to fill the window.
    extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                // SAFETY: BeginPaint/EndPaint are paired; the device context is only used between
                // them, and the bitmap header describes the buffer we pass exactly.
                unsafe {
                    let dc = BeginPaint(window, &mut paint);
                    if let Some(frame) = FRAME.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                        let info = BITMAPINFO {
                            bmiHeader: BITMAPINFOHEADER {
                                biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>())
                                    .unwrap_or(0),
                                biWidth: frame.width,
                                // Negative height means top-down, which is how our rows are stored.
                                biHeight: -frame.height,
                                biPlanes: 1,
                                biBitCount: 32,
                                biCompression: BI_RGB.0,
                                ..Default::default()
                            },
                            ..Default::default()
                        };
                        let target = paint.rcPaint;
                        StretchDIBits(
                            dc,
                            0,
                            0,
                            target.right - target.left,
                            target.bottom - target.top,
                            0,
                            0,
                            frame.width,
                            frame.height,
                            Some(frame.pixels.as_ptr().cast()),
                            &info,
                            DIB_RGB_COLORS,
                            SRCCOPY,
                        );
                    }
                    let _ = EndPaint(window, &paint);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: posting the quit message ends this thread's loop.
                unsafe { PostQuitMessage(0) };
                LRESULT(0)
            }
            // SAFETY: the documented default handler for everything we do not handle.
            _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
        }
    }

    /// Keeps the colour type referenced as the bindings evolve.
    const _: fn() -> COLORREF = || COLORREF(0);
}

#[cfg(not(windows))]
mod imp {
    use super::PresentError;

    // ponytail: Linux would use a fullscreen X11/Wayland surface; Phase 5.
    pub struct Presenter;

    impl Presenter {
        pub fn open() -> Result<Self, PresentError> {
            Err(PresentError::NotSupported)
        }

        pub fn show(&self, _pixels: Vec<u8>, _width: u32, _height: u32) {}
    }
}
