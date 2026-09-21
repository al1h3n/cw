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

/// The session of the user physically at the console, or `None` when the login screen is up and no
/// one is signed in there. This is the session a per-session helper should run in.
#[must_use]
pub fn active_console_session() -> Option<u32> {
    imp::active_console_session()
}

/// A process launched into another login session by [`launch_in_session`].
///
/// Dropping it **terminates** the process (it is held in a job object with kill-on-close), so the
/// SYSTEM service can guarantee the helper never outlives it: a stop, a crash or a session change
/// all drop this and take the helper with them, and a fresh one is launched.
pub struct SessionProcess(imp::SessionProcess);

impl SessionProcess {
    /// Whether the process is still running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.0.is_running()
    }

    /// The process id, for logging.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.0.pid()
    }
}

/// Launches `exe` with `args` **inside** `session_id`, as that session's logged-in user, attached to
/// its interactive desktop, with no console window. This is how the SYSTEM service (session 0, which
/// cannot capture a desktop) gets a helper running where the screen actually is.
///
/// Requires the caller to hold `SE_TCB` — i.e. to be running as LocalSystem (the service). It will
/// fail for an ordinary elevated process, which is expected.
///
/// # Errors
/// Returns [`SessionError`] if the user token cannot be obtained (no one is logged in, or the caller
/// is not SYSTEM) or the process cannot be created.
pub fn launch_in_session(
    session_id: u32,
    exe: &std::path::Path,
    args: &[&str],
) -> Result<SessionProcess, SessionError> {
    imp::launch_in_session(session_id, exe, args).map(SessionProcess)
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    use windows::{
        Win32::{
            Foundation::{CloseHandle, HANDLE, WAIT_TIMEOUT},
            Security::{DuplicateTokenEx, SecurityImpersonation, TOKEN_ALL_ACCESS, TokenPrimary},
            System::{
                Environment::{CreateEnvironmentBlock, DestroyEnvironmentBlock},
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
                    SetInformationJobObject,
                },
                RemoteDesktop::{WTSGetActiveConsoleSessionId, WTSQueryUserToken},
                Threading::{
                    CREATE_BREAKAWAY_FROM_JOB, CREATE_NO_WINDOW, CREATE_SUSPENDED,
                    CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, PROCESS_INFORMATION,
                    ResumeThread, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
                },
            },
        },
        core::{PCWSTR, PWSTR},
    };

    use windows::Win32::System::RemoteDesktop::{
        WTS_CONNECTSTATE_CLASS, WTS_CURRENT_SERVER_HANDLE, WTS_SESSION_INFOW, WTSActive,
        WTSEnumerateSessionsW, WTSFreeMemory,
    };

    use super::{Session, SessionError};

    pub fn active_console_session() -> Option<u32> {
        // SAFETY: no arguments; returns the console session id or 0xFFFFFFFF when none is attached.
        let id = unsafe { WTSGetActiveConsoleSessionId() };
        if id == u32::MAX { None } else { Some(id) }
    }

    /// Closes a Windows HANDLE on drop, so early returns never leak tokens.
    struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_invalid() {
                // SAFETY: we own this handle and close it exactly once.
                unsafe {
                    let _ = CloseHandle(self.0);
                }
            }
        }
    }

    pub struct SessionProcess {
        process: HANDLE,
        thread: HANDLE,
        job: HANDLE,
        pid: u32,
    }

    // The handles are only ever touched from the one supervisor thread that owns the value.
    unsafe impl Send for SessionProcess {}

    impl SessionProcess {
        pub fn is_running(&self) -> bool {
            // SAFETY: `process` is a valid handle we own until Drop.
            unsafe { WaitForSingleObject(self.process, 0) == WAIT_TIMEOUT }
        }

        pub fn pid(&self) -> u32 {
            self.pid
        }
    }

    impl Drop for SessionProcess {
        fn drop(&mut self) {
            // Terminate directly too, in case the process broke away from the job: belt and braces.
            // SAFETY: all three are handles we created and own; each is closed exactly once.
            unsafe {
                let _ = TerminateProcess(self.process, 1);
                if !self.job.is_invalid() {
                    let _ = CloseHandle(self.job); // kill-on-close also fires here
                }
                if !self.thread.is_invalid() {
                    let _ = CloseHandle(self.thread);
                }
                if !self.process.is_invalid() {
                    let _ = CloseHandle(self.process);
                }
            }
        }
    }

    fn wide_cmdline(exe: &std::path::Path, args: &[&str]) -> Vec<u16> {
        let mut line = format!("\"{}\"", exe.display());
        for arg in args {
            line.push_str(&format!(" \"{arg}\""));
        }
        line.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn launch_in_session(
        session_id: u32,
        exe: &std::path::Path,
        args: &[&str],
    ) -> Result<SessionProcess, SessionError> {
        // SAFETY: a standard WTSQueryUserToken -> DuplicateTokenEx -> CreateProcessAsUserW sequence.
        // Every handle is owned and released; the descriptors below are fully initialised; the
        // command-line and desktop buffers outlive the call.
        unsafe {
            let mut user_token = HANDLE::default();
            WTSQueryUserToken(session_id, &mut user_token)
                .map_err(|e| SessionError(format!("query user token: {}", e.message())))?;
            let user_token = OwnedHandle(user_token);

            let mut primary = HANDLE::default();
            DuplicateTokenEx(
                user_token.0,
                TOKEN_ALL_ACCESS,
                None,
                SecurityImpersonation,
                TokenPrimary,
                &mut primary,
            )
            .map_err(|e| SessionError(format!("duplicate token: {}", e.message())))?;
            let primary = OwnedHandle(primary);

            // The user's environment, so the helper sees the student's profile, not SYSTEM's.
            let mut environment: *mut c_void = std::ptr::null_mut();
            let have_env = CreateEnvironmentBlock(&mut environment, Some(primary.0), false).is_ok();

            // A job with kill-on-close: when the service drops this handle, Windows kills the helper.
            let job = CreateJobObjectW(None, PCWSTR::null())
                .map_err(|e| SessionError(format!("create job: {}", e.message())))?;
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let _ = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0),
            );

            let mut cmdline = wide_cmdline(exe, args);
            let mut desktop: Vec<u16> = "winsta0\\default\0".encode_utf16().collect();
            let startup = STARTUPINFOW {
                cb: u32::try_from(size_of::<STARTUPINFOW>()).unwrap_or(0),
                lpDesktop: PWSTR(desktop.as_mut_ptr()),
                ..Default::default()
            };

            let mut flags = CREATE_SUSPENDED | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB;
            let env_ptr = if have_env {
                flags |= CREATE_UNICODE_ENVIRONMENT;
                Some(environment.cast_const())
            } else {
                None
            };

            let mut info = PROCESS_INFORMATION::default();
            let created = CreateProcessAsUserW(
                Some(primary.0),
                PCWSTR::null(),
                Some(PWSTR(cmdline.as_mut_ptr())),
                None,
                None,
                false,
                flags,
                env_ptr,
                PCWSTR::null(),
                &startup,
                &mut info,
            );
            if have_env {
                let _ = DestroyEnvironmentBlock(environment);
            }
            created.map_err(|e| {
                let _ = CloseHandle(job);
                SessionError(format!("create process as user: {}", e.message()))
            })?;

            // Put it in the job before it runs a single instruction, then let it go.
            let _ = AssignProcessToJobObject(job, info.hProcess);
            ResumeThread(info.hThread);

            Ok(SessionProcess {
                process: info.hProcess,
                thread: info.hThread,
                job,
                pid: info.dwProcessId,
            })
        }
    }

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

    pub fn active_console_session() -> Option<u32> {
        None
    }

    pub struct SessionProcess;
    impl SessionProcess {
        pub fn is_running(&self) -> bool {
            false
        }
        pub fn pid(&self) -> u32 {
            0
        }
    }

    pub fn launch_in_session(
        _session_id: u32,
        _exe: &std::path::Path,
        _args: &[&str],
    ) -> Result<SessionProcess, SessionError> {
        Err(SessionError(
            "launching a session helper is only supported on Windows".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn there_is_a_console_session_on_this_dev_machine() {
        // A desktop Windows box always has a session attached to the physical console (even at the
        // login screen), so this resolves to Some; it returns None only when nothing is attached.
        // This is the session id the SYSTEM service would launch a helper into.
        assert!(
            active_console_session().is_some(),
            "expected a console session id on an interactive machine"
        );
    }

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
