//! Bringing an existing application window to the front.
//!
//! Used for single-instance-per-classroom: when a teacher switches to a classroom that already has a
//! console window open, focus that window instead of spawning a second one. The window handle is
//! passed as a plain `u64` so the shared console code never has to name a Windows type.

/// Focuses the window with handle `handle`, restoring it if minimised. Returns `false` if the handle
/// is not a live window (e.g. the instance that recorded it has since closed), so the caller can fall
/// back to opening a fresh window.
#[must_use]
pub fn focus(handle: u64) -> bool {
    imp::focus(handle)
}

#[cfg(windows)]
mod imp {
    use windows::Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            IsIconic, IsWindow, SC_RESTORE, SW_RESTORE, SW_SHOW, SendMessageW, SetForegroundWindow,
            ShowWindow, WM_SYSCOMMAND,
        },
    };

    pub fn focus(handle: u64) -> bool {
        let hwnd = HWND(handle as usize as *mut core::ffi::c_void);
        // SAFETY: IsWindow accepts any handle value and simply reports whether it is a live window; a
        // stale handle from a closed instance returns false and we do nothing further.
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return false;
        }
        // SAFETY: standard show/restore/foreground calls on a validated live window handle. A minimised
        // window is restored via the same syscommand a user's click on the taskbar would send.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                let _ = SendMessageW(
                    hwnd,
                    WM_SYSCOMMAND,
                    Some(WPARAM(SC_RESTORE as usize)),
                    Some(LPARAM(0)),
                );
                let _ = ShowWindow(hwnd, SW_RESTORE);
            } else {
                let _ = ShowWindow(hwnd, SW_SHOW);
            }
            // SetForegroundWindow succeeds because the caller (the console the teacher just clicked in)
            // is the current foreground process, which Windows allows to hand focus on.
            let _ = SetForegroundWindow(hwnd);
        }
        true
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn focus(_handle: u64) -> bool {
        // ponytail: Linux would raise the window via the window manager / EWMH; macOS via NSRunning
        // application activation. Wired up with those console builds.
        false
    }
}
