//! Keeping a child process alive: the restart policy, and a runner that applies it.
//!
//! In production the SYSTEM service supervises the per-session helper; if the helper crashes it must
//! come back, but a helper that dies instantly must not be respawned in a tight loop. The policy here
//! is a small, deterministic state machine (unit-tested without any real processes); [`supervise`]
//! wraps it around a real spawn/wait loop.

use std::time::Duration;

/// Decides how long to wait before restarting a child, based on how long it just ran.
///
/// Exponential backoff on repeated fast failures, capped, and **reset** once a run lasts long enough
/// to count as healthy — so a normal restart (e.g. user logoff/logon) does not accumulate delay,
/// while a crash loop backs off instead of pegging the CPU. The agent never gives up; it keeps
/// retrying at the cap.
#[derive(Debug, Clone, Copy)]
pub struct RestartPolicy {
    /// Delay after the first failure.
    pub base: Duration,
    /// Maximum delay between restarts.
    pub cap: Duration,
    /// A run lasting at least this long is "healthy" and resets the backoff.
    pub healthy_after: Duration,
    consecutive_failures: u32,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self::new(
            Duration::from_secs(1),
            Duration::from_secs(30),
            Duration::from_secs(60),
        )
    }
}

impl RestartPolicy {
    /// Creates a policy. `base` doubles each consecutive fast failure up to `cap`; a run of at least
    /// `healthy_after` resets the count.
    #[must_use]
    pub const fn new(base: Duration, cap: Duration, healthy_after: Duration) -> Self {
        Self {
            base,
            cap,
            healthy_after,
            consecutive_failures: 0,
        }
    }

    /// Records that a child ran for `run_duration` and exited, and returns how long to wait before
    /// restarting it.
    pub fn record_exit(&mut self, run_duration: Duration) -> Duration {
        if run_duration >= self.healthy_after {
            self.consecutive_failures = 0;
        }
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        // base * 2^(failures-1), saturating, then capped.
        let shift = self.consecutive_failures - 1;
        let factor = 1u64.checked_shl(shift).unwrap_or(u64::MAX);
        let delay = self
            .base
            .checked_mul(u32::try_from(factor).unwrap_or(u32::MAX))
            .unwrap_or(self.cap);
        delay.min(self.cap)
    }

    /// Consecutive fast failures since the last healthy run (for logging/metrics).
    #[must_use]
    pub const fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }
}

/// A signal the supervisor checks to know when to stop restarting and return.
pub trait StopSignal {
    /// True once the supervisor should stop.
    fn should_stop(&self) -> bool;
}

impl<F: Fn() -> bool> StopSignal for F {
    fn should_stop(&self) -> bool {
        self()
    }
}

mod runner {
    use std::{
        process::Command,
        thread,
        time::{Duration, Instant},
    };

    use super::{RestartPolicy, StopSignal};

    /// Runs `command`, restarting it per `policy`, until `stop` says to stop. Returns the number of
    /// times the child was (re)started.
    ///
    /// `poll` is how often the stop signal is checked while waiting between restarts, so a stop is
    /// noticed promptly instead of after a full backoff delay.
    pub fn supervise(
        mut command: impl FnMut() -> Command,
        mut policy: RestartPolicy,
        stop: &impl StopSignal,
        poll: Duration,
    ) -> u32 {
        let mut starts = 0u32;
        while !stop.should_stop() {
            let started = Instant::now();
            starts += 1;
            match command().spawn() {
                Ok(mut child) => {
                    // Wait for the child, but wake periodically to notice a stop request.
                    loop {
                        if stop.should_stop() {
                            let _ = child.kill();
                            let _ = child.wait();
                            return starts;
                        }
                        match child.try_wait() {
                            Ok(Some(_status)) => break,
                            Ok(None) => thread::sleep(poll),
                            Err(_) => break, // treat a wait error like an exit; back off and retry
                        }
                    }
                }
                Err(_) => { /* spawn failed; fall through to backoff and retry */ }
            }
            let ran = started.elapsed();
            let delay = policy.record_exit(ran);
            eprintln!(
                "supervised child exited after {ran:?}; consecutive failures {}, restarting in {delay:?}",
                policy.consecutive_failures()
            );
            if sleep_until_stop(delay, stop, poll) {
                break;
            }
        }
        starts
    }

    /// Sleeps up to `delay`, returning early with `true` if a stop is requested.
    fn sleep_until_stop(delay: Duration, stop: &impl StopSignal, poll: Duration) -> bool {
        let deadline = Instant::now() + delay;
        while Instant::now() < deadline {
            if stop.should_stop() {
                return true;
            }
            thread::sleep(poll.min(deadline.saturating_duration_since(Instant::now())));
        }
        stop.should_stop()
    }
}

pub use runner::supervise;

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn backoff_doubles_and_caps() {
        let mut policy = RestartPolicy::new(secs(1), secs(30), secs(60));
        // Each exit is a fast failure (ran 0s): 1, 2, 4, 8, 16, 30 (capped), 30...
        assert_eq!(policy.record_exit(secs(0)), secs(1));
        assert_eq!(policy.record_exit(secs(0)), secs(2));
        assert_eq!(policy.record_exit(secs(0)), secs(4));
        assert_eq!(policy.record_exit(secs(0)), secs(8));
        assert_eq!(policy.record_exit(secs(0)), secs(16));
        assert_eq!(policy.record_exit(secs(0)), secs(30));
        assert_eq!(policy.record_exit(secs(0)), secs(30));
    }

    #[test]
    fn healthy_run_resets_backoff() {
        let mut policy = RestartPolicy::new(secs(1), secs(30), secs(60));
        policy.record_exit(secs(0)); // 1
        policy.record_exit(secs(0)); // 2
        assert_eq!(policy.consecutive_failures(), 2);
        // A run past healthy_after resets, so this exit is treated as the first failure again.
        assert_eq!(policy.record_exit(secs(120)), secs(1));
        assert_eq!(policy.consecutive_failures(), 1);
    }

    #[test]
    fn never_overflows_on_many_failures() {
        let mut policy = RestartPolicy::new(secs(1), secs(30), secs(60));
        let mut last = secs(0);
        for _ in 0..200 {
            last = policy.record_exit(secs(0));
        }
        assert_eq!(last, secs(30), "stays capped, no panic/overflow");
    }

    #[test]
    fn supervise_restarts_then_stops() {
        use std::sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        };

        // A child that exits immediately, so every run is a "fast failure".
        let spawns = Arc::new(AtomicU32::new(0));
        let spawns_in = spawns.clone();
        let make = move || {
            spawns_in.fetch_add(1, Ordering::SeqCst);
            trivial_command()
        };
        // Stop after the child has been started at least 3 times.
        let stop_spawns = spawns.clone();
        let stop = move || stop_spawns.load(Ordering::SeqCst) >= 3;

        let policy = RestartPolicy::new(
            Duration::from_millis(5),
            Duration::from_millis(20),
            secs(60),
        );
        let starts = supervise(make, policy, &stop, Duration::from_millis(2));
        assert!(starts >= 3, "expected at least 3 restarts, got {starts}");
    }

    #[cfg(windows)]
    fn trivial_command() -> std::process::Command {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "exit", "0"]);
        c
    }

    #[cfg(not(windows))]
    fn trivial_command() -> std::process::Command {
        std::process::Command::new("true")
    }
}
