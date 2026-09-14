//! Generates the Tauri context (window config, icons, command bindings) at compile time.

fn main() {
    tauri_build::build();
}
