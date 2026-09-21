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
        imp::Presenter::open(false).map(Self)
    }

    /// Like [`Presenter::open`], but on a **locked separate Win32 desktop** the student cannot leave:
    /// Alt+Tab, the Windows key and Ctrl+Esc do nothing while the broadcast is up, because the shell
    /// runs on the original desktop. A watchdog re-asserts the lock if the student escapes (Win+L).
    /// Dropping the presenter restores the student's desktop.
    ///
    /// # Errors
    /// [`PresentError`] if the locked desktop or the window cannot be created.
    pub fn open_locked() -> Result<Self, PresentError> {
        imp::Presenter::open(true).map(Self)
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
            System::StationsAndDesktops::{
                CloseDesktop, CreateDesktopW, DESKTOP_ACCESS_FLAGS, HDESK, OpenDesktopW,
                OpenInputDesktop, SetThreadDesktop, SwitchDesktop,
            },
            UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetSystemMetrics,
                HMENU, HWND_TOPMOST, KillTimer, MSG, PostMessageW, PostQuitMessage,
                RegisterClassExW, SC_CLOSE, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, SWP_NOACTIVATE,
                SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow,
                TranslateMessage, WM_CLOSE, WM_DESTROY, WM_PAINT, WM_SYSCOMMAND, WM_TIMER,
                WNDCLASSEXW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
            },
        },
        core::{PCWSTR, w},
    };

    use super::PresentError;

    /// The locked broadcast desktop (as an `isize`) so the watchdog can re-assert it; 0 = unlocked or
    /// tearing down. Mirrors the exam lock's mechanism (`platform::examlock`).
    static LOCK_DESKTOP: AtomicIsize = AtomicIsize::new(0);
    /// Watchdog timer id and interval (ms), matching the exam lock.
    const WATCHDOG_TIMER: usize = 1;
    const WATCHDOG_MS: u32 = 250;
    /// Broad desktop access: create windows on it and switch to it.
    const DESKTOP_ACCESS: DESKTOP_ACCESS_FLAGS = DESKTOP_ACCESS_FLAGS(0x01FF);

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
        pub fn open(locked: bool) -> Result<Self, PresentError> {
            let window = Arc::new(AtomicIsize::new(0));
            let closing = Arc::new(AtomicBool::new(false));
            let ready = Arc::new(AtomicBool::new(false));
            let failed = Arc::new(Mutex::new(String::new()));

            let thread = {
                let window = Arc::clone(&window);
                let ready = Arc::clone(&ready);
                let failed = Arc::clone(&failed);
                std::thread::spawn(move || {
                    if let Err(err) = run_window(locked, &window, &ready) {
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
            // Stop the watchdog re-asserting the lock desktop, so the teardown switch-back holds.
            LOCK_DESKTOP.store(0, Ordering::SeqCst);
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

    /// The presentation window thread: optionally lock a fresh desktop, create the window, pump
    /// messages, then (if locked) restore the student's real desktop.
    fn run_window(
        locked: bool,
        window: &Arc<AtomicIsize>,
        ready: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        // SAFETY: standard station/desktop + window calls; every desktop handle is closed before
        // returning, and the window is created only after this thread is on the target desktop.
        unsafe {
            let desktops = if locked {
                Some(enter_lock_desktop()?)
            } else {
                None
            };

            let handle = match create_window() {
                Ok(handle) => handle,
                Err(err) => {
                    if let Some((original, lock)) = desktops {
                        let _ = SetThreadDesktop(original);
                        let _ = CloseDesktop(lock);
                        let _ = CloseDesktop(original);
                    }
                    return Err(err);
                }
            };
            window.store(handle.0 as isize, Ordering::SeqCst);

            // The watchdog runs for every broadcast: locked → re-assert the desktop, unlocked →
            // re-assert "topmost" so clicking elsewhere on the student PC cannot bury the broadcast.
            let _ = SetTimer(Some(handle), WATCHDOG_TIMER, WATCHDOG_MS, None);
            let _guard = if let Some((_, lock)) = desktops {
                LOCK_DESKTOP.store(lock.0 as isize, Ordering::SeqCst);
                let _ = SwitchDesktop(lock);
                // Swallow the escape keys while locked (installed on this pumping thread).
                crate::keyguard::KeyGuard::install()
            } else {
                None
            };
            ready.store(true, Ordering::SeqCst);

            pump_messages();
            drop(_guard);

            if let Some((original, lock)) = desktops {
                LOCK_DESKTOP.store(0, Ordering::SeqCst);
                restore_real_desktop(original);
                let _ = SetThreadDesktop(original);
                let _ = CloseDesktop(lock);
                let _ = CloseDesktop(original);
            }
        }
        Ok(())
    }

    /// Creates a fresh locked desktop and attaches this thread to it. Returns `(original, lock)`.
    ///
    /// # Safety
    /// Both returned desktop handles must be closed by the caller.
    unsafe fn enter_lock_desktop() -> Result<(HDESK, HDESK), String> {
        // SAFETY: documented station/desktop calls; handles are cleaned up on every failure path.
        unsafe {
            let original = OpenInputDesktop(Default::default(), false, DESKTOP_ACCESS)
                .map_err(|e| format!("open current desktop: {}", e.message()))?;
            let lock = CreateDesktopW(
                w!("CowatcherBroadcast"),
                PCWSTR::null(),
                None,
                Default::default(),
                DESKTOP_ACCESS.0,
                None,
            )
            .map_err(|e| {
                let _ = CloseDesktop(original);
                format!("create broadcast desktop: {}", e.message())
            })?;
            if let Err(err) = SetThreadDesktop(lock) {
                let _ = CloseDesktop(lock);
                let _ = CloseDesktop(original);
                return Err(format!("attach to broadcast desktop: {}", err.message()));
            }
            Ok((original, lock))
        }
    }

    /// Switches input back to the student's ordinary ("Default") desktop, retrying briefly (a switch
    /// right after a lock/unlock can be rejected until the session settles).
    ///
    /// # Safety
    /// `original` is a valid desktop handle owned by the caller.
    unsafe fn restore_real_desktop(original: HDESK) {
        // SAFETY: the by-name handle is closed here; `original` is closed by the caller.
        unsafe {
            let by_name =
                OpenDesktopW(w!("Default"), Default::default(), false, DESKTOP_ACCESS.0).ok();
            for _ in 0..40 {
                if let Some(desktop) = by_name
                    && SwitchDesktop(desktop).is_ok()
                {
                    let _ = CloseDesktop(desktop);
                    return;
                }
                if SwitchDesktop(original).is_ok() {
                    if let Some(desktop) = by_name {
                        let _ = CloseDesktop(desktop);
                    }
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if let Some(desktop) = by_name {
                let _ = CloseDesktop(desktop);
            }
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
            WM_SYSCOMMAND if (wparam.0 & 0xFFF0) == SC_CLOSE as usize => {
                // Block Alt+F4 / the close command; the broadcast is dismissed only by the teacher
                // (Drop posts an explicit WM_CLOSE, which does not come through WM_SYSCOMMAND).
                LRESULT(0)
            }
            WM_TIMER => {
                let lock = LOCK_DESKTOP.load(Ordering::SeqCst);
                if lock != 0 {
                    // Locked: re-assert the broadcast desktop if the student escaped (Win+L unlock).
                    // SAFETY: a desktop handle we created and own until teardown.
                    unsafe {
                        let _ = SwitchDesktop(HDESK(lock as *mut std::ffi::c_void));
                    }
                } else {
                    // Unlocked: keep the broadcast on top even if the student clicks another window.
                    // SAFETY: a valid window handle; NOACTIVATE means we do not steal focus.
                    unsafe {
                        let _ = SetWindowPos(
                            window,
                            Some(HWND_TOPMOST),
                            0,
                            0,
                            0,
                            0,
                            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                        );
                    }
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: stop the watchdog, then end this thread's loop.
                unsafe {
                    let _ = KillTimer(Some(window), WATCHDOG_TIMER);
                    PostQuitMessage(0);
                }
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
        pub fn open(_locked: bool) -> Result<Self, PresentError> {
            Err(PresentError::NotSupported)
        }

        pub fn show(&self, _pixels: Vec<u8>, _width: u32, _height: u32) {}
    }
}
