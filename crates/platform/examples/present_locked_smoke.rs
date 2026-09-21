//! Smoke test for the locked broadcast desktop: shows a gray full-screen on a locked desktop for a
//! few seconds, then restores. It always drops the presenter, so it self-recovers if left alone.
//! Run with: `cargo run -p platform --example present_locked_smoke`

fn main() {
    println!("Locked broadcast window in 2s; auto-restores after 4s.");
    std::thread::sleep(std::time::Duration::from_secs(2));
    match platform::present::Presenter::open_locked() {
        Ok(presenter) => {
            let (w, h) = (320u32, 180u32);
            presenter.show(vec![80u8; (w * h * 4) as usize], w, h);
            std::thread::sleep(std::time::Duration::from_secs(4));
            drop(presenter);
            println!("restored to the real desktop.");
        }
        Err(err) => eprintln!("locked broadcast failed to start: {err}"),
    }
}
