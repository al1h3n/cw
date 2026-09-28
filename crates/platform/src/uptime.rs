//! Monotonic Windows uptime for policies that must ignore changes to the wall clock.

/// Milliseconds since this OS boot. Preserved across sign-out, reset on reboot.
#[must_use]
pub fn milliseconds() -> u64 {
    #[cfg(windows)]
    {
        // SAFETY: GetTickCount64 takes no pointers or handles.
        unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
    }
    #[cfg(not(windows))]
    {
        static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
        START
            .get_or_init(std::time::Instant::now)
            .elapsed()
            .as_millis() as u64
    }
}
