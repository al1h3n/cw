//! The Agent's local audit log: one line per remote action, whether it worked or not (D3).
//!
//! It lives on the student PC, not the Console, so a teacher cannot quietly do something and leave
//! no trace on the machine it was done to. The file is plain tab-separated text so school IT can read
//! it with Notepad and grep it without any tool of ours.
//!
//! ponytail: an append-only text file. It moves into the Agent's SQLite store (D14) and gets uploaded
//! to the Hub (PLAN 3.4) when those exist; the line format stays the same.

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

use proto::{Action, ActionOutcome, DeviceId};

/// Appends audit lines to a file. Writes are serialised so lines never interleave.
pub struct AuditLog {
    path: PathBuf,
    lock: Mutex<()>,
}

impl AuditLog {
    /// An audit log stored at `path`; the file is created on the first record.
    #[must_use]
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            lock: Mutex::new(()),
        }
    }

    /// Records one action. A failure to write is reported to the caller, never swallowed: an action
    /// that cannot be audited is logged to stderr by the caller at minimum.
    ///
    /// # Errors
    /// The file cannot be opened or written.
    pub fn record(
        &self,
        at_ms: u64,
        console: DeviceId,
        action: Action,
        outcome: ActionOutcome,
    ) -> std::io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", line(at_ms, console, action, outcome))
    }
}

/// Formats one audit line: `time_ms  console  action  detail  result`.
#[must_use]
pub fn line(at_ms: u64, console: DeviceId, action: Action, outcome: ActionOutcome) -> String {
    let detail = match action {
        Action::Shutdown { delay_seconds } | Action::Reboot { delay_seconds } => {
            format!("delay={delay_seconds}s")
        }
        Action::LogOff
        | Action::LockScreen
        | Action::CancelShutdown
        | Action::LockWallpaper
        | Action::UnlockWallpaper => "-".to_string(),
    };
    let result = match outcome {
        ActionOutcome::Started { delay_seconds } => format!("started in {delay_seconds}s"),
        ActionOutcome::Failed(reason) => format!("failed: {reason}"),
    };
    format!("{at_ms}\t{console}\t{}\t{detail}\t{result}", action.name())
}

#[cfg(test)]
mod tests {
    use proto::ActionFailure;

    use super::*;

    fn console() -> DeviceId {
        DeviceId::from_public_key(&[9u8; 32])
    }

    #[test]
    fn a_started_shutdown_records_requested_and_actual_delay() {
        let text = line(
            1_000,
            console(),
            Action::Shutdown {
                delay_seconds: 9_000,
            },
            ActionOutcome::Started { delay_seconds: 300 },
        );
        assert_eq!(
            text,
            format!(
                "1000\t{}\tshutdown\tdelay=9000s\tstarted in 300s",
                console()
            )
        );
    }

    #[test]
    fn a_refusal_is_recorded_with_its_reason() {
        let text = line(
            5,
            console(),
            Action::CancelShutdown,
            ActionOutcome::Failed(ActionFailure::NothingScheduled),
        );
        assert!(text.ends_with("cancel-shutdown\t-\tfailed: nothing was scheduled to cancel"));
    }

    #[test]
    fn records_append_rather_than_overwrite() {
        let dir = std::env::temp_dir().join(format!("cw-audit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.log");
        let _ = std::fs::remove_file(&path);
        let log = AuditLog::new(&path);
        let started = ActionOutcome::Started { delay_seconds: 0 };
        log.record(1, console(), Action::LockScreen, started)
            .unwrap();
        log.record(2, console(), Action::LockScreen, started)
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
