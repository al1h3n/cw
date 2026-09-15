//! OS-specific building blocks. This is one of the two crates (with `media`) allowed to contain
//! `unsafe` for FFI; every `unsafe` block carries a `// SAFETY:` comment and the FFI is kept thin.
//!
//! So far it holds only [`secret`] (at-rest key protection). Input injection, power control, the
//! lock desktop and wallpaper enforcement land here in later phases.
#![allow(unsafe_code)] // sanctioned here per AGENTS.md §5; FFI only, each block documented

pub mod apps;
pub mod console;
pub mod input;
pub mod power;
pub mod present;
pub mod process;
pub mod secret;
pub mod session;
pub mod wallpaper;
pub mod wol;
