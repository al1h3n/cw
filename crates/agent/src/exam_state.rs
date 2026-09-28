//! Exam intent saved across per-session helper restarts. Durations use OS uptime, never UTC.

use std::{io::Read, path::Path};

use serde::{Deserialize, Serialize};

const MAX_INTENT_BYTES: usize = 16 * 1024;

#[derive(Serialize, Deserialize)]
struct ExamState {
    message: String,
    started_ms: u64,
    deadline_ms: Option<u64>,
}

impl ExamState {
    fn remaining_seconds(&self, now: u64) -> Option<u32> {
        match self.deadline_ms {
            None => Some(0),
            Some(deadline) if now < self.started_ms => {
                Some(u32::try_from((deadline - self.started_ms).div_ceil(1000)).unwrap_or(u32::MAX))
            }
            Some(deadline) if deadline <= now => None,
            Some(deadline) => {
                Some(u32::try_from((deadline - now).div_ceil(1000)).unwrap_or(u32::MAX))
            }
        }
    }
}

pub fn save(path: &Path, message: &str, duration_seconds: u32) -> Result<(), String> {
    let now = platform::uptime::milliseconds();
    let state = ExamState {
        message: message.to_owned(),
        started_ms: now,
        deadline_ms: (duration_seconds > 0)
            .then(|| now.saturating_add(u64::from(duration_seconds) * 1000)),
    };
    let bytes = serde_json::to_vec(&state).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_INTENT_BYTES {
        return Err("exam message is too large".into());
    }
    std::fs::write(path, bytes).map_err(|e| format!("save exam intent: {e}"))
}

/// Returns the message and remaining seconds. A detected uptime reset restarts the timer.
pub fn load(path: &Path) -> Result<Option<(String, u32)>, String> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };
    let mut bytes = Vec::new();
    file.take((MAX_INTENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_INTENT_BYTES {
        return Err("saved exam intent is too large".into());
    }
    let state: ExamState = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let now = platform::uptime::milliseconds();
    let Some(remaining) = state.remaining_seconds(now) else {
        clear(path);
        return Ok(None);
    };
    Ok(Some((state.message, remaining)))
}

pub fn clear(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_survives_reload_and_release_clears_it() {
        let path = std::env::temp_dir().join(format!("cw-exam-{}", std::process::id()));
        save(&path, "Read carefully", 30).unwrap();
        let (message, seconds) = load(&path).unwrap().unwrap();
        assert_eq!(message, "Read carefully");
        assert!((1..=30).contains(&seconds));
        clear(&path);
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn timed_intent_counts_uptime_and_handles_detected_reset() {
        let state = ExamState {
            message: "Exam".into(),
            started_ms: 1000,
            deadline_ms: Some(11_000),
        };
        assert_eq!(state.remaining_seconds(4000), Some(7));
        assert_eq!(state.remaining_seconds(11_000), None);
        assert_eq!(state.remaining_seconds(100), Some(10));
    }
}
