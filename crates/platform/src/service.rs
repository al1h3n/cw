//! Installing the Agent as a Windows **service**, so it starts at boot and a student cannot switch
//! it off from Task Manager — but a school administrator still can.
//!
//! # Why a service, and why this is not "hiding"
//!
//! The goal is: the Agent runs from boot, survives a student trying to disable it, and only an
//! administrator can turn it off. A Windows **service** is exactly that, and it is the *legitimate*
//! mechanism — not a trick:
//!
//! * Services start before anyone logs in and keep running across logouts.
//! * Services **do not appear in Task Manager's "Startup" tab** at all — that tab only lists
//!   `Run`-key and Startup-folder entries a user can toggle. This is normal for every Windows
//!   service, not something we do to hide. So a student cannot disable it there.
//! * An administrator turns it off the standard way: `services.msc`, or `sc config CowatcherAgent
//!   start= disabled`, or `cowatcher-agent uninstall` from an elevated prompt.
//!
//! What we deliberately do **not** do is hide the *process*: it still shows in Task Manager's
//! Details/Services tabs, still shows its tray icon, and still shows the login notice (D3). Hiding
//! the running process would make it spyware, get it flagged by antivirus, and break D3 — so we
//! don't. "Not in the Startup tab" is a property of being a service; "invisible" is not on offer.
//!
//! Installing, starting and stopping a service all require administrator rights, so everything here
//! must be run from an elevated prompt. It therefore **cannot be verified in an unelevated dev
//! session** — this is the same elevation gate as step 1.4b in `docs/PLAN.md`, and the install/run
//! path needs checking on a VM before release.

/// The service's internal name (for `sc`/`services.msc`) and its friendly display name.
pub const SERVICE_NAME: &str = "CowatcherAgent";
/// What an administrator sees in `services.msc`.
pub const DISPLAY_NAME: &str = "Co-watcher Agent";
/// The description shown next to it, so it is never a mystery entry.
pub const DESCRIPTION: &str = "Co-watcher classroom agent: lets a paired teacher console watch and \
     manage this PC. Managed by your school's administrator; see services.msc to disable.";

/// Why a service operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ServiceError {
    /// The caller is not running as administrator.
    #[error("this needs an administrator: run it from an elevated command prompt")]
    NeedsAdmin,
    /// The service is not installed.
    #[error("the service is not installed")]
    NotInstalled,
    /// The Windows service control manager reported an error.
    #[error("service control manager: {0}")]
    Scm(String),
    /// Not supported off Windows.
    #[error("services are only supported on Windows")]
    NotSupported,
}

/// Installs the Agent as an auto-start service that runs `cowatcher-agent run`.
///
/// Idempotent-ish: installing when already installed returns an [`ServiceError::Scm`] the caller can
/// treat as "already there". Requires administrator rights.
///
/// # Errors
/// [`ServiceError`] if not elevated or the SCM refuses.
pub fn install() -> Result<(), ServiceError> {
    imp::install()
}

/// Removes the service. Requires administrator rights.
///
/// # Errors
/// [`ServiceError`] if not elevated, not installed, or the SCM refuses.
pub fn uninstall() -> Result<(), ServiceError> {
    imp::uninstall()
}

/// Whether the service is currently installed. Needs no elevation.
#[must_use]
pub fn is_installed() -> bool {
    imp::is_installed()
}

/// Runs as the service body: hands control to the SCM and calls `work` once running, stopping it
/// when Windows asks the service to stop. Call this only from the service's own process (the
/// `run` subcommand the installer registers).
///
/// `work` receives a `&AtomicBool` that flips to `true` when a stop is requested, so a long-running
/// loop can exit cleanly.
///
/// # Errors
/// [`ServiceError`] if the dispatcher cannot start (e.g. the process was not launched by the SCM).
pub fn run(work: fn(&std::sync::atomic::AtomicBool)) -> Result<(), ServiceError> {
    imp::run(work)
}

#[cfg(windows)]
mod imp {
    use std::{
        ffi::OsString,
        sync::{
            Arc, OnceLock,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use windows_service::{
        service::{
            ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl,
            ServiceExitCode, ServiceInfo, ServiceStartType, ServiceState, ServiceStatus,
            ServiceType,
        },
        service_control_handler::{self, ServiceControlHandlerResult},
        service_dispatcher,
        service_manager::{ServiceManager, ServiceManagerAccess},
    };

    use super::{DESCRIPTION, DISPLAY_NAME, SERVICE_NAME, ServiceError};

    fn scm(e: windows_service::Error) -> ServiceError {
        // A permission error from the SCM is almost always "not elevated"; surface that plainly by
        // digging out the underlying OS error (5 = ERROR_ACCESS_DENIED) rather than matching text.
        if let windows_service::Error::Winapi(io) = &e
            && io.raw_os_error() == Some(5)
        {
            return ServiceError::NeedsAdmin;
        }
        ServiceError::Scm(e.to_string())
    }

    fn current_exe() -> Result<std::path::PathBuf, ServiceError> {
        std::env::current_exe().map_err(|e| ServiceError::Scm(e.to_string()))
    }

    pub fn install() -> Result<(), ServiceError> {
        let manager = ServiceManager::local_computer(
            None::<&str>,
            ServiceManagerAccess::CREATE_SERVICE | ServiceManagerAccess::CONNECT,
        )
        .map_err(scm)?;
        let info = ServiceInfo {
            name: OsString::from(SERVICE_NAME),
            display_name: OsString::from(DISPLAY_NAME),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart, // starts at boot
            error_control: ServiceErrorControl::Normal,
            executable_path: current_exe()?,
            launch_arguments: vec![OsString::from("run")],
            dependencies: vec![],
            account_name: None, // None = LocalSystem, which the wallpaper policy needs
            account_password: None,
        };
        let service = manager
            .create_service(&info, ServiceAccess::CHANGE_CONFIG)
            .map_err(scm)?;
        // A description means an administrator sees what it is, never a mystery service.
        let _ = service.set_description(DESCRIPTION);
        Ok(())
    }

    pub fn uninstall() -> Result<(), ServiceError> {
        let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
            .map_err(scm)?;
        let service = manager
            .open_service(
                SERVICE_NAME,
                ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
            )
            .map_err(|_| ServiceError::NotInstalled)?;
        service.delete().map_err(scm)
    }

    pub fn is_installed() -> bool {
        let Ok(manager) =
            ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        else {
            return false;
        };
        manager
            .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS)
            .is_ok()
    }

    /// The stop flag, shared between the SCM control handler and the work closure.
    static STOP: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    /// The work to run, stashed so the `extern "system"` entry point can reach it.
    static WORK: OnceLock<fn(&AtomicBool)> = OnceLock::new();

    windows_service::define_windows_service!(ffi_service_main, service_main);

    pub fn run(work: fn(&AtomicBool)) -> Result<(), ServiceError> {
        let _ = WORK.set(work);
        service_dispatcher::start(SERVICE_NAME, ffi_service_main).map_err(scm)
    }

    fn service_main(_args: Vec<OsString>) {
        if let Err(err) = run_service() {
            eprintln!("service failed: {err:?}");
        }
    }

    fn run_service() -> Result<(), ServiceError> {
        let stop = Arc::new(AtomicBool::new(false));
        let _ = STOP.set(Arc::clone(&stop));

        let handler_stop = Arc::clone(&stop);
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                handler_stop.store(true, Ordering::SeqCst);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let status_handle =
            service_control_handler::register(SERVICE_NAME, handler).map_err(scm)?;

        let running = |state: ServiceState, accept: ServiceControlAccept| ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        };
        status_handle
            .set_service_status(running(
                ServiceState::Running,
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
            ))
            .map_err(scm)?;

        // Do the actual work until a stop is requested.
        if let Some(work) = WORK.get() {
            work(&stop);
        }

        status_handle
            .set_service_status(running(
                ServiceState::Stopped,
                ServiceControlAccept::empty(),
            ))
            .map_err(scm)?;
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use std::sync::atomic::AtomicBool;

    use super::ServiceError;

    pub fn install() -> Result<(), ServiceError> {
        Err(ServiceError::NotSupported)
    }
    pub fn uninstall() -> Result<(), ServiceError> {
        Err(ServiceError::NotSupported)
    }
    pub fn is_installed() -> bool {
        false
    }
    pub fn run(_work: fn(&AtomicBool)) -> Result<(), ServiceError> {
        Err(ServiceError::NotSupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_service_names_are_stable_and_sensible() {
        // These are the strings an administrator will search for; a rename is a breaking change for
        // anyone who scripted `sc ... CowatcherAgent`, so guard them.
        assert_eq!(SERVICE_NAME, "CowatcherAgent");
        assert!(!DISPLAY_NAME.is_empty());
        assert!(
            DESCRIPTION.to_lowercase().contains("administrator"),
            "the description must tell an admin how it is managed"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn service_operations_are_unsupported_off_windows() {
        assert_eq!(install(), Err(ServiceError::NotSupported));
        assert!(!is_installed());
    }
}
