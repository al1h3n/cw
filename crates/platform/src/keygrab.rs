//! Grabbing the **controller's** keyboard so every key goes to the remote PC, like RustDesk.
//!
//! When a teacher takes full control of a student PC, the keys they press must reach that PC — *all*
//! of them, including the ones the local shell would otherwise eat: the Windows keys, Alt+Tab,
//! Ctrl+Esc, Alt+F4. A normal window (even a native winit one) never sees those, because the OS shell
//! handles them first. The only user-space way to intercept them is a low-level keyboard hook
//! (`WH_KEYBOARD_LL`), which sees every key before the shell and can **swallow it locally** and
//! **forward it** to the remote instead. This is exactly how RustDesk/TeamViewer capture the keyboard.
//!
//! This is the forwarding sibling of [`crate::keyguard`] (which only *drops* escape keys for exam
//! lock). Here we drop the key locally *and* report it so the caller can send it on.
//!
//! **Honest limits (same as `keyguard`):** **Ctrl+Alt+Del** and **Win+L** are handled by the kernel
//! and cannot be intercepted from user space, so they always act on the teacher's own PC. Everything
//! else — including the Windows key, Alt+Tab and Ctrl+Esc — is captured and forwarded while the grab
//! is active.
//!
//! **Host key = Right Ctrl** (the VirtualBox convention). It toggles control on and off: while the
//! viewer window is focused (armed) *or* already capturing, a Right-Ctrl press is swallowed and
//! reported as [`GrabEvent::Toggle`] instead of reaching either PC. A single key is used, not a chord,
//! because `Ctrl+Alt+Esc` failed in practice: `Alt+Esc` is a shell shortcut the OS consumes before any
//! window or hook sees it, so control could never be taken (and the Windows key therefore never
//! forwarded). Right Ctrl is not a shell shortcut, so both the hook and the window reliably see it.
//!
//! Two states drive the hook: **armed** ([`set_focused`], the viewer has focus, so the host key is
//! live) and **active** ([`set_active`], control is on, so every key is swallowed + forwarded). The
//! hook must be installed on a thread that pumps messages (the viewer's event-loop thread); [`KeyGrab`]
//! is an RAII handle that removes it on drop and can never outlive the process.

/// What the grabbed keyboard produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrabEvent {
    /// A key changed state (Windows virtual-key code), to be sent to the remote PC.
    Key {
        /// Windows virtual-key code.
        vk: u16,
        /// True on press, false on release.
        down: bool,
    },
    /// The host key (Right Ctrl) was pressed: toggle control on/off.
    Toggle,
}

/// While alive, a low-level keyboard hook is installed. It only *acts* while [`set_active`] is true;
/// installed-but-inactive it passes every key through untouched, so the teacher's own PC works
/// normally until they actually take control.
pub struct KeyGrab {
    // Held only for its Drop, which removes the hook; the underscore keeps dead-code analysis happy.
    _inner: imp::KeyGrab,
}

impl KeyGrab {
    /// Installs the hook on the current thread, routing every captured event to `on_event`.
    ///
    /// The callback runs on the thread that pumps messages (the event loop), so it must be quick and
    /// non-blocking — send on a channel, do not do work inline. Returns `None` if the hook could not
    /// be installed, in which case the caller simply runs without full capture.
    #[must_use]
    pub fn install<F>(on_event: F) -> Option<Self>
    where
        F: Fn(GrabEvent) + Send + Sync + 'static,
    {
        imp::install(Box::new(on_event)).map(|inner| Self { _inner: inner })
    }
}

/// Turns capturing on or off without removing the hook. Off = keys pass straight through to the local
/// PC; on = keys are swallowed locally and reported. Called when control is taken/released and when
/// the viewer window gains/loses focus (so an unfocused viewer never eats the teacher's keys).
pub fn set_active(active: bool) {
    imp::set_active(active);
}

/// Arms or disarms the host key (Right Ctrl). Armed while the viewer window has focus, so the teacher
/// can *take* control with the host key even before capturing starts — and so the host key is never
/// swallowed while the viewer is in the background (it belongs to whatever they alt-tabbed to).
pub fn set_focused(focused: bool) {
    imp::set_focused(focused);
}

#[cfg(windows)]
mod imp {
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    };

    use windows::{
        Win32::{
            Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Input::KeyboardAndMouse::VK_RCONTROL,
                WindowsAndMessaging::{
                    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, SetWindowsHookExW,
                    UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN,
                    WM_SYSKEYUP,
                },
            },
        },
        core::PCWSTR,
    };

    use super::GrabEvent;

    /// Whether the grab is currently swallowing + forwarding every key (control is on).
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    /// Whether the viewer window has focus, so the host key (Right Ctrl) is live.
    static FOCUSED: AtomicBool = AtomicBool::new(false);
    /// The sink for captured events. Set on install, cleared on drop.
    #[allow(clippy::type_complexity)]
    static SINK: Mutex<Option<Box<dyn Fn(GrabEvent) + Send + Sync>>> = Mutex::new(None);

    pub fn set_active(active: bool) {
        ACTIVE.store(active, Ordering::SeqCst);
    }

    pub fn set_focused(focused: bool) {
        FOCUSED.store(focused, Ordering::SeqCst);
    }

    fn fire(event: GrabEvent) {
        if let Ok(guard) = SINK.lock()
            && let Some(sink) = guard.as_ref()
        {
            sink(event);
        }
    }

    pub struct KeyGrab {
        hook: HHOOK,
    }

    pub fn install(on_event: Box<dyn Fn(GrabEvent) + Send + Sync>) -> Option<KeyGrab> {
        *SINK.lock().unwrap_or_else(|e| e.into_inner()) = Some(on_event);
        // SAFETY: standard hook installation; the proc is a real `extern "system"` fn and the module
        // handle is this process's own. The hook is removed in Drop on this same thread.
        unsafe {
            let module = GetModuleHandleW(PCWSTR::null()).ok()?;
            let hook = SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(hook_proc),
                Some(HINSTANCE(module.0)),
                0,
            );
            match hook {
                Ok(hook) => Some(KeyGrab { hook }),
                Err(_) => {
                    *SINK.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    None
                }
            }
        }
    }

    impl Drop for KeyGrab {
        fn drop(&mut self) {
            ACTIVE.store(false, Ordering::SeqCst);
            // SAFETY: unhooking a hook we installed on this thread.
            unsafe {
                let _ = UnhookWindowsHookEx(self.hook);
            }
            *SINK.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let active = ACTIVE.load(Ordering::SeqCst);
        let focused = FOCUSED.load(Ordering::SeqCst);
        if code == HC_ACTION as i32 && (active || focused) {
            // SAFETY: for HC_ACTION, lparam points to a KBDLLHOOKSTRUCT owned by the OS.
            let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let msg = wparam.0 as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            if down || up {
                let vk = event.vkCode as u16;
                // Right Ctrl is the host key: swallow it (both edges, so neither PC sees a stray Ctrl)
                // and, on press, toggle control. Live whenever the viewer is focused or capturing.
                if vk == VK_RCONTROL.0 {
                    if down {
                        fire(GrabEvent::Toggle);
                    }
                    return LRESULT(1);
                }
                // Every other key is only swallowed + forwarded while actually capturing; when merely
                // focused (view-only) the teacher's keys reach their own PC normally.
                if active {
                    fire(GrabEvent::Key { vk, down });
                    return LRESULT(1); // swallow locally: the key only reaches the remote PC
                }
            }
        }
        // SAFETY: the documented pass-through for everything we do not capture.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::GrabEvent;

    pub struct KeyGrab;

    pub fn install(_on_event: Box<dyn Fn(GrabEvent) + Send + Sync>) -> Option<KeyGrab> {
        None
    }

    pub fn set_active(_active: bool) {}

    pub fn set_focused(_focused: bool) {}
}
