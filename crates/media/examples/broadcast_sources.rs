//! Smoke test for the broadcast source picker backend: list windows and capture a few thumbnails.
//! Run with: `cargo run -p media --example broadcast_sources`

fn main() {
    let windows = media::window_capture::list_windows();
    println!("{} windows", windows.len());
    for w in windows.iter().take(6) {
        let shot = media::window_capture::capture_window_jpeg(w.id, 320);
        let note = shot
            .map(|j| format!("{} B", j.len()))
            .unwrap_or_else(|e| e.to_string());
        println!("  0x{:x}  {:<40}  thumb: {note}", w.id, w.title);
    }

    match media::ThumbnailCapturer::new() {
        Ok(mut cap) => {
            for m in cap.monitors() {
                let shot = cap.capture_jpeg(m.index, 320, 60);
                let note = shot
                    .map(|j| format!("{} B", j.len()))
                    .unwrap_or_else(|e| e.to_string());
                println!(
                    "monitor {} {}x{} primary={} thumb: {note}",
                    m.index, m.width, m.height, m.primary
                );
            }
        }
        Err(e) => println!("capturer failed: {e}"),
    }
}
