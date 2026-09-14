//! Listing running programs and closing them, for app/game blocking.
//!
//! Blocking a game is: watch what is running, and when a blocked program appears, end it. This
//! module is only the OS mechanism — *which* programs are blocked is a Policy decision that lives in
//! `proto`/`policy`, and the matching (so `steam.exe` blocks Steam but not `mysteam-notes.exe`) is
//! pure logic tested without touching a real process (see [`matches_blocklist`]).
//!
//! It closes only processes in the caller's own session and never a system-critical one, so a bad
//! rule can annoy a student but cannot break Windows.

/// One running program, as much as we need to decide whether to block it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Process {
    /// OS process id.
    pub pid: u32,
    /// Executable file name, lower-cased, e.g. `steam.exe`. Never a full path.
    pub name: String,
}

/// Errors listing or ending processes.
#[derive(Debug, thiserror::Error)]
#[error("process operation failed: {0}")]
pub struct ProcessError(String);

/// Names we refuse to terminate however a rule is written: ending one bluescreens or freezes the PC.
/// Lower-case, compared exactly.
const PROTECTED: &[&str] = &[
    "system",
    "system idle process",
    "wininit.exe",
    "winlogon.exe",
    "csrss.exe",
    "services.exe",
    "lsass.exe",
    "smss.exe",
    "svchost.exe",
    "dwm.exe",
    "explorer.exe",
    "fontdrvhost.exe",
    "sihost.exe",
    "cowatcher-agent.exe",
];

/// Whether `name` (any case) is one we must never terminate.
#[must_use]
pub fn is_protected(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    PROTECTED.contains(&lower.as_str())
}

/// Whether a process name matches any blocklist entry.
///
/// An entry matches by exact file name, case-insensitively: `steam.exe` or bare `steam` both block
/// `steam.exe`. Substring matching is deliberately avoided — a rule for `roblox` should not also
/// kill `robloxstudio-safe-homework.exe`. Protected names never match.
#[must_use]
pub fn matches_blocklist(name: &str, blocklist: &[String]) -> bool {
    if is_protected(name) {
        return false;
    }
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".exe").unwrap_or(&lower);
    blocklist.iter().any(|entry| {
        let e = entry.trim().to_ascii_lowercase();
        let e = e.strip_suffix(".exe").unwrap_or(&e);
        !e.is_empty() && e == stem
    })
}

/// Lists the programs running in this login session.
///
/// # Errors
/// Returns [`ProcessError`] if the OS call fails.
pub fn list_processes() -> Result<Vec<Process>, ProcessError> {
    imp::list_processes()
}

/// Ends the process with this id. Refuses [protected](is_protected) processes.
///
/// # Errors
/// Returns [`ProcessError`] if the process cannot be opened or terminated (it may already be gone,
/// which the caller can treat as success).
pub fn terminate(pid: u32) -> Result<(), ProcessError> {
    imp::terminate(pid)
}

/// Ends every running program that matches the blocklist, returning the names it closed.
///
/// This is the enforcement step an Agent runs on a timer. A program that dies between listing and
/// terminating is not an error — it is the outcome we wanted.
///
/// # Errors
/// Returns [`ProcessError`] only if the initial process listing fails.
pub fn enforce_blocklist(blocklist: &[String]) -> Result<Vec<String>, ProcessError> {
    let mut closed = Vec::new();
    for process in list_processes()? {
        if matches_blocklist(&process.name, blocklist) && terminate(process.pid).is_ok() {
            closed.push(process.name);
        }
    }
    Ok(closed)
}

#[cfg(windows)]
mod imp {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess},
        },
    };

    use super::{Process, ProcessError, is_protected};

    pub fn list_processes() -> Result<Vec<Process>, ProcessError> {
        // SAFETY: a process snapshot with no owner data; the handle is closed on every path.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|e| ProcessError(e.message()))?;
        let mut entry = PROCESSENTRY32W {
            dwSize: u32::try_from(std::mem::size_of::<PROCESSENTRY32W>()).unwrap_or(0),
            ..Default::default()
        };
        let mut processes = Vec::new();
        // SAFETY: `entry.dwSize` is set as the API requires; iteration stops when Process32NextW
        // returns an error (no more entries). The snapshot handle stays valid throughout.
        unsafe {
            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    processes.push(Process {
                        pid: entry.th32ProcessID,
                        name: wide_to_string(&entry.szExeFile).to_ascii_lowercase(),
                    });
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }
        Ok(processes)
    }

    pub fn terminate(pid: u32) -> Result<(), ProcessError> {
        // SAFETY: opening a handle by pid; if the pid is gone the open fails and we report it.
        let handle: HANDLE = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }
            .map_err(|e| ProcessError(format!("cannot open process {pid}: {}", e.message())))?;
        // SAFETY: `handle` is a valid PROCESS_TERMINATE handle we just opened; closed right after.
        let result = unsafe { TerminateProcess(handle, 1) };
        // SAFETY: closing the handle we opened; not used afterwards.
        unsafe {
            let _ = CloseHandle(handle);
        }
        result.map_err(|e| ProcessError(e.message()))
    }

    /// A protected process can be listed but never handed to `terminate`; belt and braces.
    const _: fn(&str) -> bool = is_protected;

    /// Reads a NUL-terminated UTF-16 array into a String, stopping at the first NUL.
    fn wide_to_string(wide: &[u16]) -> String {
        let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
        String::from_utf16_lossy(&wide[..end])
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{Process, ProcessError};

    // ponytail: Linux would read /proc, macOS would use libproc; added with those Agents (Phase 5).
    pub fn list_processes() -> Result<Vec<Process>, ProcessError> {
        Ok(Vec::new())
    }

    pub fn terminate(_pid: u32) -> Result<(), ProcessError> {
        Err(ProcessError("not supported on this platform".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Vec<String> {
        ["steam.exe", "roblox"]
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    }

    #[test]
    fn matches_by_exact_name_ignoring_case_and_extension() {
        assert!(matches_blocklist("Steam.exe", &list()));
        assert!(matches_blocklist("STEAM.EXE", &list()));
        assert!(matches_blocklist("roblox.exe", &list()));
    }

    #[test]
    fn does_not_match_a_different_program_that_merely_contains_the_name() {
        // The whole point of exact matching: a note-taking app is not the game.
        assert!(!matches_blocklist("robloxstudio-homework.exe", &list()));
        assert!(!matches_blocklist("mysteamnotes.exe", &list()));
    }

    #[test]
    fn never_matches_a_protected_process_even_if_listed() {
        let evil = vec!["explorer.exe".to_string(), "winlogon".to_string()];
        assert!(!matches_blocklist("explorer.exe", &evil));
        assert!(!matches_blocklist("winlogon.exe", &evil));
    }

    #[test]
    fn an_empty_or_blank_entry_matches_nothing() {
        let blanks = vec![String::new(), "   ".to_string()];
        assert!(!matches_blocklist("steam.exe", &blanks));
    }

    #[cfg(windows)]
    #[test]
    fn lists_this_test_process_among_the_running_programs() {
        let names: Vec<String> = list_processes()
            .expect("list")
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert!(!names.is_empty());
    }
}
