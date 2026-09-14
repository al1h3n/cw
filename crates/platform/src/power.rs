//! Powering a PC down, restarting it, signing the student out, and locking the session.
//!
//! These are the first things a Console can *do* to a student PC rather than just watch. They are
//! deliberately thin wrappers over the OS's own facilities (AGENTS.md §5: prefer OS policy over
//! fighting the OS) — Windows already shows the student a localised countdown dialog and already
//! knows how to cancel one, so we do not reimplement either.
//!
//! Shutting down needs `SeShutdownPrivilege`. On a normal workstation the interactive user has it,
//! but it is disabled in the token until asked for, which is what [`enable_shutdown_privilege`]
//! does. Locking needs no privilege at all.

use std::time::Duration;

/// The longest countdown we will start, five minutes.
///
/// A remote peer picks the delay, so it is clamped here rather than trusted: an hour-long countdown
/// would leave a PC in a state the teacher has forgotten about and cannot easily explain.
pub const MAX_DELAY: Duration = Duration::from_secs(300);

/// Why a power action did not happen.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PowerError {
    /// The account running the Agent may not shut this PC down.
    #[error("not permitted to do that on this PC")]
    NotPermitted,
    /// Nothing was counting down when a cancel arrived.
    #[error("no shutdown is scheduled")]
    NothingScheduled,
    /// This platform has no implementation yet.
    #[error("not supported on this platform")]
    NotSupported,
    /// The OS refused for some other reason; the text is for logs, never for a remote peer.
    #[error("the operating system refused: {0}")]
    Os(String),
}

/// How a shutdown request is turned into OS arguments.
///
/// Pure data so the rules can be tested without powering anything off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownPlan {
    /// Seconds of countdown, after clamping.
    pub timeout_seconds: u32,
    /// Whether the PC restarts afterwards.
    pub reboot: bool,
    /// Whether programs are closed without letting them ask to save.
    ///
    /// Only true for an immediate shutdown: if the teacher gave the class no warning at all they
    /// have decided the lesson is over, and a single unsaved Notepad must not veto the whole room.
    /// With any countdown at all, students get the chance to save.
    pub force_apps_closed: bool,
}

/// Works out the OS arguments for a shutdown or reboot request.
#[must_use]
pub fn plan_shutdown(delay: Duration, reboot: bool) -> ShutdownPlan {
    let clamped = delay.min(MAX_DELAY);
    let timeout_seconds = u32::try_from(clamped.as_secs()).unwrap_or(0);
    ShutdownPlan {
        timeout_seconds,
        reboot,
        force_apps_closed: timeout_seconds == 0,
    }
}

/// Locks the session, exactly as Win+L does. The student's programs keep running.
///
/// # Errors
/// Returns [`PowerError`] if the OS refuses, or on a platform without an implementation.
pub fn lock_screen() -> Result<(), PowerError> {
    imp::lock_screen()
}

/// Starts a shutdown or restart with a countdown the student can see.
///
/// # Errors
/// [`PowerError::NotPermitted`] without the shutdown privilege, otherwise [`PowerError::Os`].
pub fn shutdown(delay: Duration, reboot: bool) -> Result<(), PowerError> {
    imp::shutdown(plan_shutdown(delay, reboot))
}

/// Signs the current user out, closing their programs.
///
/// # Errors
/// [`PowerError::NotPermitted`] without the shutdown privilege, otherwise [`PowerError::Os`].
pub fn log_off() -> Result<(), PowerError> {
    imp::log_off()
}

/// Calls off a shutdown or restart that is still counting down.
///
/// # Errors
/// [`PowerError::NothingScheduled`] if no countdown was running.
pub fn cancel_shutdown() -> Result<(), PowerError> {
    imp::cancel_shutdown()
}

/// Asks the OS to enable this process's shutdown privilege.
///
/// Call it once at start-up: the privilege is present but disabled in an ordinary user's token, and
/// enabling it early means a shutdown request later fails fast and visibly rather than half-way.
///
/// # Errors
/// [`PowerError::NotPermitted`] if the account does not hold the privilege at all.
pub fn enable_shutdown_privilege() -> Result<(), PowerError> {
    imp::enable_shutdown_privilege()
}

#[cfg(windows)]
mod imp {
    use windows::{
        Win32::{
            Foundation::{CloseHandle, ERROR_NO_SHUTDOWN_IN_PROGRESS, ERROR_NOT_ALL_ASSIGNED},
            Security::{
                AdjustTokenPrivileges, LUID_AND_ATTRIBUTES, LookupPrivilegeValueW,
                SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
            },
            System::{
                Shutdown::{
                    AbortSystemShutdownW, EWX_LOGOFF, ExitWindowsEx, InitiateSystemShutdownExW,
                    LockWorkStation, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER,
                    SHTDN_REASON_MINOR_OTHER,
                },
                Threading::{GetCurrentProcess, OpenProcessToken},
            },
        },
        core::{PCWSTR, w},
    };

    use super::{PowerError, ShutdownPlan};

    /// The reason code written to the event log: a planned, administrative action.
    const REASON: windows::Win32::System::Shutdown::SHUTDOWN_REASON =
        windows::Win32::System::Shutdown::SHUTDOWN_REASON(
            SHTDN_REASON_MAJOR_OTHER.0 | SHTDN_REASON_MINOR_OTHER.0 | SHTDN_REASON_FLAG_PLANNED.0,
        );

    fn os(error: &windows::core::Error) -> PowerError {
        PowerError::Os(error.message())
    }

    pub fn lock_screen() -> Result<(), PowerError> {
        // SAFETY: no arguments, no buffers; the call either locks the station or reports an error.
        unsafe { LockWorkStation() }.map_err(|e| os(&e))
    }

    pub fn shutdown(plan: ShutdownPlan) -> Result<(), PowerError> {
        enable_shutdown_privilege()?;
        // SAFETY: null machine name = this PC, null message = Windows' own localised countdown
        // text. The remaining arguments are plain values validated by `plan_shutdown`.
        unsafe {
            InitiateSystemShutdownExW(
                PCWSTR::null(),
                PCWSTR::null(),
                plan.timeout_seconds,
                plan.force_apps_closed,
                plan.reboot,
                REASON,
            )
        }
        .map_err(|e| os(&e))
    }

    pub fn log_off() -> Result<(), PowerError> {
        enable_shutdown_privilege()?;
        // SAFETY: a documented call with constant flags; it affects only this session.
        unsafe { ExitWindowsEx(EWX_LOGOFF, REASON) }.map_err(|e| os(&e))
    }

    pub fn cancel_shutdown() -> Result<(), PowerError> {
        // SAFETY: null machine name = this PC.
        match unsafe { AbortSystemShutdownW(PCWSTR::null()) } {
            Ok(()) => Ok(()),
            // Windows reports "nothing to abort" as an error; to a teacher that is information,
            // not a failure, so it gets its own variant.
            Err(e) if e.code() == ERROR_NO_SHUTDOWN_IN_PROGRESS.to_hresult() => {
                Err(PowerError::NothingScheduled)
            }
            Err(e) => Err(os(&e)),
        }
    }

    pub fn enable_shutdown_privilege() -> Result<(), PowerError> {
        adjust(w!("SeShutdownPrivilege"))
    }

    /// Enables one named privilege in this process's token.
    fn adjust(name: PCWSTR) -> Result<(), PowerError> {
        let mut token = windows::Win32::Foundation::HANDLE::default();
        // SAFETY: `token` is a valid out-pointer; the handle is closed on every path below.
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            )
        }
        .map_err(|e| os(&e))?;

        let result = (|| {
            let mut luid = windows::Win32::Foundation::LUID::default();
            // SAFETY: null system name = the local PC; `luid` is a valid out-pointer.
            unsafe { LookupPrivilegeValueW(PCWSTR::null(), name, &mut luid) }
                .map_err(|e| os(&e))?;

            let privileges = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [LUID_AND_ATTRIBUTES {
                    Luid: luid,
                    Attributes: SE_PRIVILEGE_ENABLED,
                }],
            };
            // SAFETY: `privileges` is a fully initialised, correctly sized structure that outlives
            // the call; the three null/zero arguments are the documented "no previous state" form.
            unsafe { AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None) }
                .map_err(|e| os(&e))?;

            // AdjustTokenPrivileges reports success even when it changed nothing, so the real
            // answer is in the last error: the account simply does not hold the privilege.
            let last = windows::core::Error::from_thread();
            if last.code() == ERROR_NOT_ALL_ASSIGNED.to_hresult() {
                return Err(PowerError::NotPermitted);
            }
            Ok(())
        })();

        // SAFETY: `token` came from a successful OpenProcessToken and is not used afterwards.
        let _ = unsafe { CloseHandle(token) };
        result
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{PowerError, ShutdownPlan};

    pub fn lock_screen() -> Result<(), PowerError> {
        Err(PowerError::NotSupported)
    }

    pub fn shutdown(_plan: ShutdownPlan) -> Result<(), PowerError> {
        Err(PowerError::NotSupported)
    }

    pub fn log_off() -> Result<(), PowerError> {
        Err(PowerError::NotSupported)
    }

    pub fn cancel_shutdown() -> Result<(), PowerError> {
        Err(PowerError::NotSupported)
    }

    pub fn enable_shutdown_privilege() -> Result<(), PowerError> {
        Err(PowerError::NotSupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_immediate_shutdown_forces_apps_closed() {
        let plan = plan_shutdown(Duration::ZERO, false);
        assert_eq!(plan.timeout_seconds, 0);
        assert!(plan.force_apps_closed);
        assert!(!plan.reboot);
    }

    #[test]
    fn any_countdown_lets_students_save() {
        let plan = plan_shutdown(Duration::from_secs(1), false);
        assert_eq!(plan.timeout_seconds, 1);
        assert!(!plan.force_apps_closed);
    }

    #[test]
    fn an_absurd_delay_is_clamped_not_trusted() {
        let plan = plan_shutdown(Duration::from_secs(86_400), true);
        assert_eq!(plan.timeout_seconds, MAX_DELAY.as_secs() as u32);
        assert!(plan.reboot);
    }

    #[test]
    fn the_maximum_delay_itself_survives_unchanged() {
        assert_eq!(
            plan_shutdown(MAX_DELAY, false).timeout_seconds,
            MAX_DELAY.as_secs() as u32
        );
    }
}
