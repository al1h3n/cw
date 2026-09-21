//! Swallowing the keys a student could use to escape a locked exam or broadcast (FEATURES #4/#10).
//!
//! A low-level keyboard hook (`WH_KEYBOARD_LL`) sees every key before the foreground app and can drop
//! the dangerous combinations — Alt+Tab, Alt+Esc, Ctrl+Esc (Start), the Windows keys, and Alt+F4 —
//! so a locked screen cannot be switched or closed from the keyboard. Spike 0.9 proved this works.
//!
//! **Honest limits (documented, not hidden):** two sequences are handled by the OS itself and cannot
//! be blocked from user space — **Win+L** (lock workstation) and **Ctrl+Alt+Del** (the Secure
//! Attention Sequence). Blocking those needs a keyboard *filter driver* or a kiosk/assigned-access
//! configuration. The exam/broadcast watchdog instead re-asserts the lock desktop after a Win+L
//! unlock, which is the best a user-space program can do.
//!
//! The hook must be installed **on a thread that pumps messages** (the exam/broadcast window thread),
//! and removed on the same thread — [`KeyGuard`] is an RAII handle that does exactly that on drop.

/// While alive, drops the escape keys listed above. Dropping it removes the hook.
pub struct KeyGuard {
    // Held only for its Drop, which removes the hook; the underscore keeps dead-code analysis happy.
    _inner: imp::KeyGuard,
}

impl KeyGuard {
    /// Installs the keyboard guard on the current thread. Returns `None` if the hook could not be set
    /// (the caller then simply runs without it rather than failing the whole lock).
    #[must_use]
    pub fn install() -> Option<Self> {
        imp::KeyGuard::install().map(|inner| Self { _inner: inner })
    }
}

#[cfg(windows)]
mod imp {
    use windows::{
        Win32::{
            Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM},
            System::LibraryLoader::GetModuleHandleW,
            UI::{
                Input::KeyboardAndMouse::{
                    GetAsyncKeyState, VK_CONTROL, VK_ESCAPE, VK_F4, VK_LWIN, VK_RWIN, VK_TAB,
                },
                WindowsAndMessaging::{
                    CallNextHookEx, HC_ACTION, HHOOK, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN,
                    SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
                },
            },
        },
        core::PCWSTR,
    };

    pub struct KeyGuard {
        hook: HHOOK,
    }

    impl KeyGuard {
        pub fn install() -> Option<Self> {
            // SAFETY: standard hook installation; the proc is a real `extern "system"` fn and the
            // module handle is this process's own.
            unsafe {
                let module = GetModuleHandleW(PCWSTR::null()).ok()?;
                let hook =
                    SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), Some(HINSTANCE(module.0)), 0)
                        .ok()?;
                Some(Self { hook })
            }
        }
    }

    impl Drop for KeyGuard {
        fn drop(&mut self) {
            // SAFETY: unhooking a hook we installed on this thread.
            unsafe {
                let _ = UnhookWindowsHookEx(self.hook);
            }
        }
    }

    /// Decides whether a key event is one of the escape combinations we drop. Pure and unit-tested.
    fn should_block(vk: u32, alt_down: bool, ctrl_down: bool) -> bool {
        let lwin = u32::from(VK_LWIN.0);
        let rwin = u32::from(VK_RWIN.0);
        let tab = u32::from(VK_TAB.0);
        let esc = u32::from(VK_ESCAPE.0);
        let f4 = u32::from(VK_F4.0);
        vk == lwin
            || vk == rwin
            || (alt_down && vk == tab) // Alt+Tab
            || (alt_down && vk == esc) // Alt+Esc
            || (ctrl_down && vk == esc) // Ctrl+Esc (Start menu)
            || (alt_down && vk == f4) // Alt+F4
    }

    extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            // SAFETY: for HC_ACTION, lparam points to a KBDLLHOOKSTRUCT owned by the OS.
            let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let alt_down = (event.flags & LLKHF_ALTDOWN).0 != 0;
            // Ctrl is not in the hook flags; read its live state.
            // SAFETY: a documented, side-effect-free key-state query.
            let ctrl_down = unsafe { GetAsyncKeyState(i32::from(VK_CONTROL.0)) } as u16 & 0x8000 != 0;
            if should_block(event.vkCode, alt_down, ctrl_down) {
                return LRESULT(1); // swallow: do not pass to the next hook or the app
            }
        }
        // SAFETY: the documented pass-through for everything we do not block.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn blocks_the_escape_combos_only() {
            let win = u32::from(VK_LWIN.0);
            let tab = u32::from(VK_TAB.0);
            let esc = u32::from(VK_ESCAPE.0);
            let f4 = u32::from(VK_F4.0);
            let a = u32::from(b'A');

            assert!(should_block(win, false, false), "Windows key");
            assert!(should_block(tab, true, false), "Alt+Tab");
            assert!(should_block(esc, true, false), "Alt+Esc");
            assert!(should_block(esc, false, true), "Ctrl+Esc");
            assert!(should_block(f4, true, false), "Alt+F4");

            assert!(!should_block(tab, false, false), "plain Tab is fine");
            assert!(!should_block(esc, false, false), "plain Esc is fine");
            assert!(!should_block(a, true, true), "Ctrl+Alt+A is not ours");
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub struct KeyGuard;

    impl KeyGuard {
        pub fn install() -> Option<Self> {
            None
        }
    }
}
