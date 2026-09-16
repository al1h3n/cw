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
                EndPaint, FF_SWISS, FW_SEMIBOLD, FillRect, HBRUSH, HFONT, OUT_TT_PRECIS,
                PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
            },
            System::StationsAndDesktops::{
                CloseDesktop, CreateDesktopW, DESKTOP_ACCESS_FLAGS, OpenInputDesktop,
                SetThreadDesktop, SwitchDesktop,
            },
            UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
                GetSystemMetrics, HMENU, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
                SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, SetForegroundWindow, ShowWindow,
                TranslateMessage, WM_CLOSE, WM_DESTROY, WM_PAINT, WNDCLASSEXW, WS_EX_TOPMOST,
                WS_POPUP, WS_VISIBLE,
            },
        },
        core::{PCWSTR, w},
    };

    use super::ExamError;

    /// The message shown on the lock, read by the paint handler.
    static MESSAGE: Mutex<String> = Mutex::new(String::new());

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
        // Broad access so we can create windows on it and switch to it (create/switch/read/write).
        let access = DESKTOP_ACCESS_FLAGS(0x01FF);
        // SAFETY: standard station/desktop calls; every handle is closed before returning, and the
        // window is created only after this thread is attached to the new desktop.
        unsafe {
            let original = OpenInputDesktop(Default::default(), false, access)
                .map_err(|e| format!("open current desktop: {}", e.message()))?;
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
            ready.store(true, Ordering::SeqCst);

            // Make the lock desktop the one that receives input, then pump until the window closes.
            let _ = SwitchDesktop(exam);
            pump_messages();

            // Ended: give the student's real desktop back, detach, and close both handles.
            let _ = SwitchDesktop(original);
            let _ = SetThreadDesktop(original);
            let _ = CloseDesktop(exam);
            let _ = CloseDesktop(original);
        }
        Ok(())
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
                    DrawTextW(dc, &mut wide, &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

                    SelectObject(dc, old);
                    let _ = DeleteObject(font.into());
                    let _ = EndPaint(window, &paint);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // SAFETY: ends this thread's message loop.
                unsafe { PostQuitMessage(0) };
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
