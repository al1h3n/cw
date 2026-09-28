//! Small student-facing notice after a teacher sends a workspace file.

/// Shows a notice in the logged-in student's session without blocking the network loop.
pub fn file_received(name: &str) {
    #[cfg(windows)]
    {
        let name = name.to_owned();
        std::thread::spawn(move || {
            use windows::{
                Win32::UI::WindowsAndMessaging::{MB_ICONINFORMATION, MB_OK, MessageBoxW},
                core::{PCWSTR, w},
            };
            let message: Vec<u16> = format!("A file was sent to your Co-watcher folder:\n{name}")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: NUL-terminated UTF-16 text is owned for the whole synchronous Win32 call.
            unsafe {
                MessageBoxW(
                    None,
                    PCWSTR(message.as_ptr()),
                    w!("Co-watcher"),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
        });
    }
    #[cfg(not(windows))]
    let _ = name;
}
