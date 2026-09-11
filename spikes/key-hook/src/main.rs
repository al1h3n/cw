//! Spike 0.9 — can the Console swallow Win, Alt+Tab, Ctrl+Shift+Esc so they reach the *remote* PC?
//!
//! ```text
//! key-hook [SECONDS=30]
//! ```
//!
//! Manual test with a physical keyboard: focus the window, press Win, Alt+Tab, Win+D, Win+Tab,
//! Ctrl+Shift+Esc. Each should be logged as "swallowed" and do nothing locally (in the product it is
//! forwarded to the student PC). **Win+Esc** releases capture; clicking the window captures again.
//! OS-reserved keys a hook can never block: Ctrl+Alt+Del and Win+L — the viewer needs toolbar buttons.
//! Failsafe: the window closes itself after SECONDS, and capture only applies while it is focused.
//!
//! Finding: keys injected with SendInput are NOT a valid test. The shell handles injected Win/Alt+Tab
//! before or around the hook, so automated tests must target a pure "swallow or pass" decision function,
//! with physical-keyboard checks in the release checklist.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    hook::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("key-hook is Windows-only");
}

#[cfg(windows)]
mod hook {
    use std::{
        sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
        time::{Duration, Instant},
    };

    use anyhow::{Context, Result};
    use windows::Win32::{
        Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_ESCAPE, VK_LWIN, VK_RWIN},
            WindowsAndMessaging::{
                CallNextHookEx, GetForegroundWindow, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_UP, SetWindowsHookExW,
                UnhookWindowsHookEx, WH_KEYBOARD_LL,
            },
        },
    };
    use winit::{
        application::ApplicationHandler,
        dpi::LogicalSize,
        event::WindowEvent,
        event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
        raw_window_handle::{HasWindowHandle, RawWindowHandle},
        window::{Window, WindowId},
    };

    static CAPTURING: AtomicBool = AtomicBool::new(true);
    static WIN_DOWN: AtomicBool = AtomicBool::new(false);
    static OUR_WINDOW: AtomicIsize = AtomicIsize::new(0);
    static SWALLOWED: AtomicU32 = AtomicU32::new(0);

    /// Runs on the thread that installed it, inside its message loop. Must return fast
    /// (Windows silently removes hooks that exceed LowLevelHooksTimeout).
    unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            // SAFETY: for WH_KEYBOARD_LL with HC_ACTION, lparam points to a valid KBDLLHOOKSTRUCT.
            let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let up = key.flags.contains(LLKHF_UP);
            let vk = VIRTUAL_KEY(key.vkCode as u16);
            if vk == VK_LWIN || vk == VK_RWIN {
                WIN_DOWN.store(!up, Ordering::Relaxed);
            }
            // SAFETY: plain query of the foreground window handle.
            let focused = unsafe { GetForegroundWindow() }.0 as isize == OUR_WINDOW.load(Ordering::Relaxed);
            if focused && CAPTURING.load(Ordering::Relaxed) {
                if !up && vk == VK_ESCAPE && WIN_DOWN.load(Ordering::Relaxed) {
                    CAPTURING.store(false, Ordering::Relaxed);
                    println!("Win+Esc: capture released (click the window to capture again)");
                } else {
                    println!("swallowed  {:<4} vk=0x{:02X}", if up { "up" } else { "down" }, vk.0);
                }
                SWALLOWED.fetch_add(1, Ordering::Relaxed);
                return LRESULT(1);
            }
        }
        // SAFETY: forwarding to the next hook with the arguments we received.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    pub fn run() -> Result<()> {
        let seconds = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(30u64);

        // SAFETY: installing a global LL hook for this thread's message loop; removed before returning.
        let hook = unsafe {
            let module = GetModuleHandleW(None).context("GetModuleHandleW")?;
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), Some(HINSTANCE(module.0)), 0)
                .context("SetWindowsHookExW")?
        };

        let event_loop = EventLoop::new()?;
        let mut app = App { window: None, deadline: Instant::now() + Duration::from_secs(seconds) };
        let result = event_loop.run_app(&mut app);
        // SAFETY: the handle came from SetWindowsHookExW above.
        let _ = unsafe { UnhookWindowsHookEx(hook) };
        result?;
        println!("\nswallowed {} key events", SWALLOWED.load(Ordering::Relaxed));
        Ok(())
    }

    struct App {
        window: Option<Window>,
        deadline: Instant,
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let attributes = Window::default_attributes()
                .with_title("key-hook spike: keys are captured while focused; Win+Esc releases")
                .with_inner_size(LogicalSize::new(640.0, 240.0));
            let Ok(window) = event_loop.create_window(attributes) else {
                event_loop.exit();
                return;
            };
            if let Ok(handle) = window.window_handle()
                && let RawWindowHandle::Win32(win32) = handle.as_raw()
            {
                OUR_WINDOW.store(win32.hwnd.get(), Ordering::Relaxed);
            }
            self.window = Some(window);
            event_loop.set_control_flow(ControlFlow::WaitUntil(self.deadline));
        }

        fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::MouseInput { .. } if !CAPTURING.swap(true, Ordering::Relaxed) => {
                    println!("capture on");
                }
                _ => {}
            }
        }

        fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
            if Instant::now() >= self.deadline {
                event_loop.exit();
            }
        }
    }
}
