//! Enforcing the "no games" list: a background thread that ends blocked programs on a timer.
//!
//! Blocking must keep working after the teacher's Console disconnects and after the PC reboots with
//! no network (D9: enforce the last policy fully offline). So the rules live in shared state and on
//! disk, and a plain OS thread — not the control session — does the enforcing. The control session
//! only updates the rules and reads back what was closed.
//!
//! ponytail: this is a poll every second, which is what the brief's "programs list" needs and what
//! spike 2.5 planned. ETW real-time process-start events are the upgrade if a game must die in
//! milliseconds rather than within a second; the rule format would not change.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use proto::MAX_BLOCKLIST;

/// How often the blocker sweeps the process list.
const SWEEP: Duration = Duration::from_secs(1);
/// Most recently-closed names remembered for reporting, so the buffer cannot grow without bound if
/// a Console never reads it.
const CLOSED_MEMORY: usize = 64;

/// Shared, thread-safe blocking state.
#[derive(Default)]
struct Shared {
    rules: Vec<String>,
    /// Names closed since a Console last collected them (newest last).
    closed: Vec<String>,
}

/// The running blocker: owns its worker thread and the rule file.
pub struct Blocker {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    path: PathBuf,
    worker: Option<JoinHandle<()>>,
}

impl Blocker {
    /// Starts the blocker, loading any rules saved from a previous run so blocking survives a reboot.
    #[must_use]
    pub fn start(rules_path: &Path) -> Self {
        let rules = load(rules_path);
        let shared = Arc::new(Mutex::new(Shared {
            rules,
            closed: Vec::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));

        let worker = {
            let shared = Arc::clone(&shared);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || run(&shared, &stop))
        };

        Self {
            shared,
            stop,
            path: rules_path.to_path_buf(),
            worker: Some(worker),
        }
    }

    /// How many rules are in force right now.
    #[must_use]
    pub fn rule_count(&self) -> u16 {
        let shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        u16::try_from(shared.rules.len()).unwrap_or(u16::MAX)
    }

    /// Replaces the rules (an empty list turns blocking off) and saves them for next boot.
    /// Returns the number kept after capping and dropping blanks.
    pub fn set_rules(&self, programs: Vec<String>) -> u16 {
        let cleaned = clean(programs);
        {
            let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            shared.rules = cleaned.clone();
        }
        if let Err(err) = save(&self.path, &cleaned) {
            eprintln!("could not save blocklist: {err}");
        }
        u16::try_from(cleaned.len()).unwrap_or(u16::MAX)
    }

    /// Drains the names blocked since the last call, for reporting to the Console.
    pub fn take_closed(&self) -> Vec<String> {
        let mut shared = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut shared.closed)
    }
}

impl Drop for Blocker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Normalises a rule list: trims, lower-cases, drops blanks and duplicates, caps the length.
fn clean(programs: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in programs {
        let name = entry.trim().to_ascii_lowercase();
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
        if out.len() == MAX_BLOCKLIST {
            break;
        }
    }
    out
}

/// The worker loop: sweep, sleep, until asked to stop. Idle (no listing) while there are no rules.
fn run(shared: &Arc<Mutex<Shared>>, stop: &Arc<AtomicBool>) {
    while !stop.load(Ordering::SeqCst) {
        let rules = {
            let shared = shared.lock().unwrap_or_else(|e| e.into_inner());
            shared.rules.clone()
        };
        if !rules.is_empty()
            && let Ok(closed) = platform::process::enforce_blocklist(&rules)
            && !closed.is_empty()
        {
            let mut shared = shared.lock().unwrap_or_else(|e| e.into_inner());
            for name in closed {
                println!("blocked {name}");
                shared.closed.push(name);
            }
            let overflow = shared.closed.len().saturating_sub(CLOSED_MEMORY);
            if overflow > 0 {
                shared.closed.drain(0..overflow);
            }
        }
        // Sleep in short slices so stop is noticed quickly.
        for _ in 0..10 {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(SWEEP / 10);
        }
    }
}

/// Loads saved rules, one per line. A missing file just means "nothing blocked".
fn load(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|text| clean(text.lines().map(str::to_string).collect()))
        .unwrap_or_default()
}

/// Saves rules one per line (atomic write, so a crash cannot leave a half-written list).
fn save(path: &Path, rules: &[String]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, rules.join("\n"))?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_trims_lowercases_dedupes_and_drops_blanks() {
        let got = clean(vec![
            " Steam.exe ".into(),
            "STEAM.EXE".into(),
            String::new(),
            "roblox".into(),
        ]);
        assert_eq!(got, vec!["steam.exe".to_string(), "roblox".to_string()]);
    }

    #[test]
    fn clean_caps_at_the_protocol_limit() {
        let many: Vec<String> = (0..MAX_BLOCKLIST + 50)
            .map(|i| format!("game{i}.exe"))
            .collect();
        assert_eq!(clean(many).len(), MAX_BLOCKLIST);
    }

    #[test]
    fn rules_survive_a_restart_through_the_file() {
        let dir = std::env::temp_dir().join(format!("cw-block-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("blocklist.txt");
        let _ = std::fs::remove_file(&path);

        let first = Blocker::start(&path);
        assert_eq!(
            first.set_rules(vec!["steam.exe".into(), "roblox".into()]),
            2
        );
        drop(first);

        let second = Blocker::start(&path);
        assert_eq!(
            second.rule_count(),
            2,
            "rules reload from disk on the next start"
        );
        drop(second);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
