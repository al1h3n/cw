//! Generates the Tauri context (window config, icons, command bindings) at compile time.

fn main() {
    // `rust-embed` needs `ui/dist` to exist at compile time even before the front end has been built
    // (a fresh checkout has not run `vite build` yet). Create it empty so the crate always compiles;
    // the real assets land there when the UI is built, and are embedded into the release binary.
    let _ = std::fs::create_dir_all("ui/dist");
    tauri_build::build();
}
