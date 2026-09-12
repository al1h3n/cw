//! Interactive login sessions.
//!
//! The Agent runs as a SYSTEM service in session 0, which cannot see or capture a user's desktop. To
//! do anything visible it must launch a **session helper** inside each interactive login session.
//! This module enumerates those sessions; spawning the helper into one (which needs SYSTEM
//! privileges) is added in Phase 1.4b together with the service, and validated in a VM.

/// An interactive login session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// The Windows session id (0 is the non-interactive service session).
    pub id: u32,
    /// The window-station name (e.g. "Console", "RDP-Tcp#1").
    pub station: String,
    /// Whether a user is actively logged in and connected.
    pub active: bool,
}

/// Errors listing sessions.
#[derive(Debug, thiserror::Error)]
#[error("session enumeration failed: {0}")]
pub struct SessionError(String);

/// Lists the machine's login sessions. Works without elevation.
///
/// # Errors
/// Returns [`SessionError`] if the OS call fails.
pub fn list_sessions() -> Result<Vec<Session>, SessionError> {
    imp::list_sessions()
}

/// Lists only the sessions with a user actively connected — the ones a helper should run in.
///
/// # Errors
/// Returns [`SessionError`] if the OS call fails.
pub fn list_active_sessions() -> Result<Vec<Session>, SessionError> {
    Ok(list_sessions()?.into_iter().filter(|s| s.active).collect())
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::RemoteDesktop::{
        WTS_CONNECTSTATE_CLASS, WTS_CURRENT_SERVER_HANDLE, WTS_SESSION_INFOW, WTSActive,
        WTSEnumerateSessionsW, WTSFreeMemory,
    };

    use super::{Session, SessionError};

    pub fn list_sessions() -> Result<Vec<Session>, SessionError> {
        let mut info: *mut WTS_SESSION_INFOW = std::ptr::null_mut();
        let mut count: u32 = 0;
        // SAFETY: standard WTS enumeration; on success `info` points to `count` structs that we read
        // and then hand back to WTSFreeMemory. The version argument must be 0 per the API contract.
        let sessions = unsafe {
            WTSEnumerateSessionsW(Some(WTS_CURRENT_SERVER_HANDLE), 0, 1, &mut info, &mut count)
                .map_err(|e| SessionError(e.message()))?;
            let slice = std::slice::from_raw_parts(info, count as usize);
            let sessions = slice
                .iter()
                .map(|s| Session {
                    id: s.SessionId,
                    station: pwstr_to_string(s.pWinStationName),
                    active: s.State == WTSActive,
                })
                .collect();
            WTSFreeMemory(info.cast());
            sessions
        };
        Ok(sessions)
    }

    /// Reads a NUL-terminated wide string. Returns empty if null.
    fn pwstr_to_string(p: windows::core::PWSTR) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: WTS guarantees a NUL-terminated string for a non-null pWinStationName.
        unsafe { p.to_string().unwrap_or_default() }
    }

    // Keep the connect-state type referenced so its import is not flagged as the API evolves.
    const _: fn() -> WTS_CONNECTSTATE_CLASS = || WTSActive;
}

#[cfg(not(windows))]
mod imp {
    use super::{Session, SessionError};

    // ponytail: other platforms get a real implementation with their own session model (logind on
    // Linux, launchd/utmpx on macOS) when those Agents land (Phase 5).
    pub fn list_sessions() -> Result<Vec<Session>, SessionError> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn lists_at_least_the_console_session() {
        // Runs unelevated. A desktop Windows box always has at least one session; the machine
        // running this test has an interactive user, so there is at least one active session.
        let all = list_sessions().expect("enumerate sessions");
        assert!(!all.is_empty(), "expected at least one session");
        assert!(
            all.iter().any(|s| s.active),
            "expected at least one active session"
        );
        assert!(
            all.iter().any(|s| s.id == 0),
            "session 0 (services) always exists"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_returns_empty_for_now() {
        assert!(list_sessions().unwrap().is_empty());
    }
}
