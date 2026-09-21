//! Exam / lockdown mode: a fullscreen lock the student cannot leave (FEATURES #4/#10).
//!
//! The mechanism is the one Safe Exam Browser uses and spike 0.8 proved: create a **separate Win32
//! desktop** (`CreateDesktopW`), put a fullscreen message window on it, and `SwitchDesktop` to it.
//! The student's input now goes to a desktop that contains *only* our window — Alt+Tab shows nothing
//! else, and the Windows key does nothing because `explorer.exe` (the shell) runs on the original
//! desktop, not this one. Ending the exam switches back and closes the desktop.
//!
//! ## What it does and does not stop (be honest)
//!
//! It traps the ordinary desktop: no other windows, no taskbar, no Start menu. It does **not** stop
//! **Ctrl+Alt+Del** (that always goes to the secure Winlogon desktop) and does not yet disable
//! **Task Manager** (Ctrl+Shift+Esc would open on the exam desktop) — locking those down needs the
//! policy engine (`DisableTaskMgr`) and is the next step. Presented so no one over-trusts it.

/// Why starting or ending exam mode failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExamError {
    /// The lock desktop or its window could not be created.
    #[error("could not start exam lock: {0}")]
    Failed(String),
    /// Not implemented on this platform yet.
    #[error("exam lock is not supported on this platform")]
    NotSupported,
}

/// A running exam lock. Dropping it (or [`ExamLock::stop`]) ends the lock and restores the desktop.
pub struct ExamLock {
    // Held only for its Drop, which ends the lock; the underscore keeps dead-code analysis happy.
    _inner: imp::ExamLock,
}

impl ExamLock {
    /// Starts exam mode, showing `message` full-screen on a locked desktop.
    ///
    /// # Errors
    /// [`ExamError`] if the locked desktop or window cannot be created.
    pub fn start(message: &str) -> Result<Self, ExamError> {
        imp::ExamLock::start(message).map(|inner| Self { _inner: inner })
    }

    /// Ends the exam and restores the student's desktop. Same as dropping the value.
    pub fn stop(self) {
        drop(self);
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
            Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM},
            Graphics::Gdi::{
                BeginPaint, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH,
                DEFAULT_QUALITY, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
                EndPaint, FF_SWISS, FW_SEMIBOLD, FillRect, HBRUSH, HFONT, InvalidateRect,
                OUT_TT_PRECIS, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
            },
            System::StationsAndDesktops::{
                CloseDesktop, CreateDesktopW, DESKTOP_ACCESS_FLAGS, HDESK, OpenDesktopW,
                OpenInputDesktop, SetThreadDesktop, SwitchDesktop,
            },
            UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
                GetSystemMetrics, HMENU, KillTimer, MSG, PostMessageW, PostQuitMessage,
                RegisterClassExW, SC_CLOSE, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, SetForegroundWindow,
                SetTimer, ShowWindow, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_PAINT,
                WM_SYSCOMMAND, WM_TIMER, WNDCLASSEXW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
            },
        },
        core::{PCWSTR, w},
    };

    use super::ExamError;

    /// The message shown on the lock, read by the paint handler.
    static MESSAGE: Mutex<String> = Mutex::new(String::new());

    /// The lock desktop (as an `isize`), so the watchdog timer can re-assert it. Zero means "not
    /// locked / stopping": the timer then does nothing and the message loop is free to exit.
    static EXAM_DESKTOP: AtomicIsize = AtomicIsize::new(0);

    /// The watchdog timer id and how often (ms) it re-checks that the lock desktop still has input.
    const WATCHDOG_TIMER: usize = 1;
    const WATCHDOG_MS: u32 = 250;

    pub struct ExamLock {
        window: Arc<AtomicIsize>,
        thread: Option<JoinHandle<()>>,
    }

    impl ExamLock {
        pub fn start(message: &str) -> Result<Self, ExamError> {
            {
                let mut slot = MESSAGE.lock().unwrap_or_else(|e| e.into_inner());
                *slot = message.to_string();
            }
            let window = Arc::new(AtomicIsize::new(0));
            let ready = Arc::new(AtomicBool::new(false));
            let failed = Arc::new(Mutex::new(String::new()));

            let thread = {
                let (window, ready, failed) =
                    (Arc::clone(&window), Arc::clone(&ready), Arc::clone(&failed));
                std::thread::spawn(move || {
                    if let Err(err) = run_lock(&window, &ready) {
                        *failed.lock().unwrap_or_else(|e| e.into_inner()) = err;
                        ready.store(true, Ordering::SeqCst);
                    }
                })
            };

            // Wait for the lock thread to report it is up (or that it failed).
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while !ready.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let problem = failed.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if !problem.is_empty() {
                return Err(ExamError::Failed(problem));
            }
            if window.load(Ordering::SeqCst) == 0 {
                return Err(ExamError::Failed("the lock window did not open".into()));
            }
            Ok(Self {
                window,
                thread: Some(thread),
            })
        }
    }

    impl Drop for ExamLock {
        fn drop(&mut self) {
            // Tell the watchdog to stop re-asserting the lock desktop, so the teardown switch back to
            // the student's real desktop is not immediately undone.
            EXAM_DESKTOP.store(0, Ordering::SeqCst);
            let handle = HWND(self.window.load(Ordering::SeqCst) as *mut std::ffi::c_void);
            if !handle.is_invalid() {
                // DestroyWindow only works on the creating thread; WM_CLOSE is thread-safe and the
                // default handler turns it into the destroy that ends the message loop, after which
                // the lock thread switches the desktop back.
                // SAFETY: a valid window handle and a documented no-parameter message.
                unsafe {
                    let _ = PostMessageW(Some(handle), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    /// Creates the lock desktop, shows the window on it, runs the loop, then restores the desktop.
    fn run_lock(window: &Arc<AtomicIsize>, ready: &Arc<AtomicBool>) -> Result<(), String> {
        // Access to create windows on the desktop and switch to it (read/write/create/enumerate/
        // hook/switch), but **not** the two journal-record/playback bits (0x0010 | 0x0020). Requesting
        // those from `OpenInputDesktop` needs a privilege the per-session helper does not hold, and the
        // whole open then fails with "Access is denied" — the error seen when starting exam mode. We
        // never journal, so dropping them costs nothing and fixes the open. (0x01FF & !0x0030 = 0x01CF.)
        let access = DESKTOP_ACCESS_FLAGS(0x01CF);
        // SAFETY: standard station/desktop calls; every handle is closed before returning, and the
        // window is created only after this thread is attached to the new desktop.
        unsafe {
            // Prefer the desktop that currently owns input; if that cannot be opened (e.g. the secure
            // Winlogon desktop is up for a UAC prompt), fall back to the ordinary "Default" desktop by
            // name so exam mode still starts instead of erroring out.
            let original = match OpenInputDesktop(Default::default(), false, access) {
                Ok(desktop) => desktop,
                Err(_) => OpenDesktopW(w!("Default"), Default::default(), false, access.0)
                    .map_err(|e| format!("open current desktop: {}", e.message()))?,
            };
            let exam = CreateDesktopW(
                w!("CowatcherExam"),
                PCWSTR::null(),
                None,
                Default::default(),
                access.0, // CreateDesktopW takes the raw u32 access mask here
                None,
            )
            .map_err(|e| {
                let _ = CloseDesktop(original);
                format!("create lock desktop: {}", e.message())
            })?;

            // Attach this thread to the lock desktop, so the window we create lives there.
            if let Err(err) = SetThreadDesktop(exam) {
                let _ = CloseDesktop(exam);
                let _ = CloseDesktop(original);
                return Err(format!("attach to lock desktop: {}", err.message()));
            }

            let handle = match create_window() {
                Ok(handle) => handle,
                Err(err) => {
                    let _ = SetThreadDesktop(original);
                    let _ = CloseDesktop(exam);
                    let _ = CloseDesktop(original);
                    return Err(err);
                }
            };
            window.store(handle.0 as isize, Ordering::SeqCst);
            // Publish the lock desktop so the watchdog timer can re-assert it (e.g. after the student
            // hits Win+L: Windows returns input to the Default desktop on unlock, and without this the
            // student would be sitting on their real desktop mid-exam — the escape we must close).
            EXAM_DESKTOP.store(exam.0 as isize, Ordering::SeqCst);
            ready.store(true, Ordering::SeqCst);

            // Make the lock desktop the one that receives input, then pump until the window closes.
            let _ = SwitchDesktop(exam);
            // Swallow Alt+Tab / Win / Ctrl+Esc / Alt+F4 while the lock is up (installed on this
            // pumping thread; dropped when the loop ends). Win+L / Ctrl+Alt+Del cannot be blocked.
            let _guard = crate::keyguard::KeyGuard::install();
            pump_messages();

            // Ended: give the student's real desktop back. The captured `original` handle can be a
            // poor target after a lock/unlock cycle (its SwitchDesktop is silently rejected, leaving a
            // desktop with no shell — "wallpaper, no UI"), so switch to the Default desktop resolved
            // by name, retrying until it takes, and fall back to the captured handle.
            restore_real_desktop(original, access);
            let _ = SetThreadDesktop(original);
            let _ = CloseDesktop(exam);
            let _ = CloseDesktop(original);
        }
        Ok(())
    }

    /// Switches input back to the student's ordinary ("Default") desktop, retrying briefly because a
    /// switch right after a Winlogon lock/unlock can be rejected until the session settles.
    ///
    /// # Safety
    /// Callers pass a valid `original` desktop handle; every desktop opened here is closed before
    /// returning.
    unsafe fn restore_real_desktop(original: HDESK, access: DESKTOP_ACCESS_FLAGS) {
        // SAFETY: standard station/desktop calls; the by-name handle is closed in this function and
        // `original` is owned and closed by the caller.
        unsafe {
            let by_name = OpenDesktopW(w!("Default"), Default::default(), false, access.0).ok();
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

    fn create_window() -> Result<HWND, String> {
        const CLASS: PCWSTR = w!("CowatcherExamLock");
        // SAFETY: a standard class registration and full-screen window creation; every argument is a
        // constant or a valid pointer that outlives the call.
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
                w!("Exam"),
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
            // The watchdog: on each tick, re-assert the lock desktop if the exam is still running.
            let _ = SetTimer(Some(window), WATCHDOG_TIMER, WATCHDOG_MS, None);
            Ok(window)
        }
    }

    fn pump_messages() {
        let mut message = MSG::default();
        // SAFETY: the standard Win32 message loop; GetMessageW returns 0 on WM_QUIT.
        unsafe {
            while GetMessageW(&mut message, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    extern "system" fn window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_PAINT => {
                // SAFETY: BeginPaint/EndPaint are paired; GDI objects are created and deleted here.
                unsafe {
                    let mut paint = PAINTSTRUCT::default();
                    let dc = BeginPaint(window, &mut paint);
                    let mut rect = RECT::default();
                    let _ = GetClientRect(window, &mut rect);

                    // Dark background.
                    let brush = CreateSolidBrush(COLORREF(0x0014_0A05));
                    FillRect(dc, &rect, brush);
                    let _ = DeleteObject(brush.into());

                    // Big centred message.
                    let height = ((rect.bottom - rect.top) / 14).max(24);
                    let font: HFONT = CreateFontW(
                        height,
                        0,
                        0,
                        0,
                        FW_SEMIBOLD.0 as i32,
                        0,
                        0,
                        0,
                        DEFAULT_CHARSET,
                        OUT_TT_PRECIS,
                        Default::default(),
                        DEFAULT_QUALITY,
                        (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
                        w!("Segoe UI"),
                    );
                    let old = SelectObject(dc, font.into());
                    SetBkMode(dc, TRANSPARENT);
                    SetTextColor(dc, COLORREF(0x00F0_F5FA));

                    let text = MESSAGE.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    let mut wide: Vec<u16> = text.encode_utf16().collect();
                    if wide.is_empty() {
                        wide = "Exam in progress".encode_utf16().collect();
                    }
                    DrawTextW(
                        dc,
                        &mut wide,
                        &mut rect,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                    );

                    SelectObject(dc, old);
                    let _ = DeleteObject(font.into());
                    let _ = EndPaint(window, &paint);
                }
                LRESULT(0)
            }
            WM_SYSCOMMAND if (wparam.0 & 0xFFF0) == SC_CLOSE as usize => {
                // Block Alt+F4 / the close command so the exam window cannot be dismissed. Teardown
                // uses an explicit WM_CLOSE from Drop, which is not routed through WM_SYSCOMMAND.
                LRESULT(0)
            }
            WM_TIMER => {
                // Re-assert the lock: if the student escaped to their real desktop (e.g. Win+L then
                // unlock), pull input back to the exam desktop. Zero means the exam is ending.
                let exam = EXAM_DESKTOP.load(Ordering::SeqCst);
                if exam != 0 {
                    // SAFETY: a desktop handle we created and still own until teardown; `window` is our
                    // valid lock window.
                    unsafe {
                        let _ = SwitchDesktop(HDESK(exam as *mut std::ffi::c_void));
                        // After a Win+L / unlock cycle the desktop comes back but our window can lose
                        // the foreground and its client area is not repainted — it showed as a blank
                        // black screen with no message. Pull it back to the front and force a repaint
                        // so the "exam in progress" text is visible again.
                        let _ = SetForegroundWindow(window);
                        let _ = InvalidateRect(Some(window), None, true);
                    }
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: stop the watchdog, then end this thread's message loop.
                unsafe {
                    let _ = KillTimer(Some(window), WATCHDOG_TIMER);
                    PostQuitMessage(0);
                }
                LRESULT(0)
            }
            // SAFETY: the documented default handler for everything else.
            _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::ExamError;

    pub struct ExamLock;

    impl ExamLock {
        pub fn start(_message: &str) -> Result<Self, ExamError> {
            Err(ExamError::NotSupported)
        }
    }
}
