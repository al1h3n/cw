//! Attaching a GUI program to the terminal that started it.
//!
//! The Console is built as a Windows GUI application so double-clicking it does not flash a black
//! box. The side effect is that its standard output goes nowhere, which silently breaks the command
//! line subcommands. Attaching to the parent process's console restores them when, and only when,
//! the program was started from a terminal.

/// Attaches this process to the console of whatever started it, if there is one.
///
/// Returns `true` when output will now appear in that terminal. Safe to call unconditionally: it
/// does nothing when the program was double-clicked, and nothing at all on other platforms.
pub fn attach_to_parent() -> bool {
    imp::attach_to_parent()
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};

    pub fn attach_to_parent() -> bool {
        // SAFETY: a plain call with the documented constant; failure just means there was no parent
        // console (the program was double-clicked), which is reported as `false`.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS).is_ok() }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn attach_to_parent() -> bool {
        // Other platforms keep their streams when built as a GUI program.
        true
    }
}
