//! Manual smoke test for exam lock: switches to the locked exam desktop for a few seconds, then
//! restores the real desktop. It always drops the lock, so it self-recovers even if left alone.
//!
//! Run with: `cargo run -p platform --example examlock_smoke`
//!
//! While the dark screen is up you may press Win+L and unlock to check the watchdog pulls you back
//! to the exam desktop instead of leaving you on your real desktop.

fn main() {
    println!("Exam lock in 2s; it auto-restores after 5s.");
    std::thread::sleep(std::time::Duration::from_secs(2));
    match platform::examlock::ExamLock::start("Exam smoke test — restores automatically") {
        Ok(lock) => {
            std::thread::sleep(std::time::Duration::from_secs(5));
            lock.stop();
            println!("Restored to the real desktop.");
        }
        Err(err) => eprintln!("exam lock failed to start: {err}"),
    }
}
