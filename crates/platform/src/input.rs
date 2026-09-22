//! Injecting mouse and keyboard input, and deciding which keys a viewer window swallows.
//!
//! Two halves, deliberately separated:
//!
//! * [`inject`] and friends — the thin `SendInput` FFI that moves the remote pointer and presses
//!   remote keys.
//! * [`KeyGate`] — the *pure* decision "does this keystroke go to the remote PC, or stay here?".
//!   Spike 0.9 established that injected keys cannot test a low-level keyboard hook (the shell
//!   handles Win and Alt+Tab before the hook sees them), so the policy lives in a plain function
//!   with real tests and only the hook plumbing is checked by hand on a physical keyboard.
//!
//! Keys that **cannot** be captured by any hook, documented in `docs/FEATURES.md`: `Ctrl+Alt+Del`
//! (the Secure Attention Sequence, handled by the kernel) and `Win+L`. The viewer offers toolbar
//! buttons for those instead of pretending to forward them.

/// A mouse button, as the wire and the UI name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    /// Primary button.
    Left,
    /// Context-menu button.
    Right,
    /// Wheel click.
    Middle,
}

/// What happened to a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    /// Pressed down.
    Down,
    /// Released.
    Up,
}

/// Errors injecting input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InputError {
    /// The OS refused the injection. Usually UIPI: a normal-privilege process cannot send input to
    /// an elevated window, which is why the session helper runs at the right level (step 1.4b).
    #[error("the PC refused the input (a more privileged window may have focus)")]
    Refused,
    /// No implementation on this platform yet.
    #[error("remote input is not supported on this platform")]
    NotSupported,
}

/// Moves the pointer to a position given as a fraction of the screen.
///
/// Fractions, not pixels: the teacher's screen and the student's are rarely the same size, and a
/// fraction survives a different resolution, a scaled display, and a monitor change. `x` and `y` are
/// clamped to `0.0..=1.0`, so a bad value can never push the pointer off-screen.
///
/// # Errors
/// [`InputError`] if the OS refuses.
pub fn move_pointer(x: f32, y: f32) -> Result<(), InputError> {
    imp::move_pointer(clamp_fraction(x), clamp_fraction(y))
}

/// Presses or releases a mouse button at the current pointer position.
///
/// # Errors
/// [`InputError`] if the OS refuses.
pub fn mouse_button(button: MouseButton, state: ButtonState) -> Result<(), InputError> {
    imp::mouse_button(button, state)
}

/// Scrolls the wheel. Positive scrolls up/away from the user, as Windows counts it.
///
/// # Errors
/// [`InputError`] if the OS refuses.
pub fn scroll(delta: i16) -> Result<(), InputError> {
    imp::scroll(delta)
}

/// Presses or releases one key, identified by its **Windows virtual-key code**.
///
/// A virtual-key code rather than a character: the teacher and the student may have different
/// keyboard layouts, and forwarding physical keys (as RDP and VNC do) keeps the student's own layout
/// in charge of what letter actually appears.
///
/// # Errors
/// [`InputError`] if the OS refuses.
pub fn key(virtual_key: u16, state: ButtonState) -> Result<(), InputError> {
    imp::key(virtual_key, state)
}

/// Types one Unicode character directly, bypassing the layout.
///
/// Used for characters the teacher's layout can produce but the student's cannot.
///
/// # Errors
/// [`InputError`] if the OS refuses.
pub fn unicode_char(ch: char, state: ButtonState) -> Result<(), InputError> {
    imp::unicode_char(ch, state)
}

/// Releases every modifier we may have pressed on the remote PC.
///
/// Called when a remote-control session ends, and when the viewer loses focus. Without it a student
/// is left with a "stuck" Ctrl or Alt because the key-up never arrived — the classic remote-desktop
/// bug. Best effort: a failure on one key must not stop the others.
pub fn release_all_modifiers() {
    for vk in MODIFIER_KEYS {
        let _ = imp::key(vk, ButtonState::Up);
    }
}

/// Confines the mouse pointer to a screen rectangle (the controller's viewer window), so while a
/// teacher drives a PC their cursor cannot slide onto their own desktop mid-control — every movement
/// stays in the window and maps to the remote screen. This is the mouse half of full capture.
///
/// Coordinates are in physical screen pixels. Windows also releases the clip automatically on a
/// desktop switch (Ctrl+Alt+Del), so a teacher can never be permanently trapped.
///
/// # Errors
/// [`InputError`] if the OS refuses.
pub fn confine_cursor(left: i32, top: i32, right: i32, bottom: i32) -> Result<(), InputError> {
    imp::confine_cursor(left, top, right, bottom)
}

/// Releases any cursor confinement set by [`confine_cursor`]. Called on release, and whenever the
/// viewer loses focus, so the pointer is free again.
pub fn release_cursor() {
    imp::release_cursor();
}

/// Blocks or unblocks the student's own physical mouse and keyboard.
///
/// While the teacher is driving a PC the student must not be able to fight for the pointer or type
/// over them, so this turns the student's *local* input off. Injected remote input (`SendInput`) still
/// passes through, so the teacher keeps full control. Windows lifts the block automatically if
/// Ctrl+Alt+Del is pressed or the process exits, so a student can never be permanently locked out.
///
/// Best-effort by design: a refusal (for example because a more privileged desktop currently owns
/// input) is returned so the caller can log it, but it must not stop control from working.
///
/// # Errors
/// [`InputError`] if the OS refuses the request.
pub fn set_local_input_blocked(blocked: bool) -> Result<(), InputError> {
    imp::set_local_input_blocked(blocked)
}

/// Virtual-key codes for every modifier the gate can forward.
pub const MODIFIER_KEYS: [u16; 8] = [
    VK_SHIFT,
    VK_CONTROL,
    VK_MENU,
    VK_LWIN,
    VK_RWIN,
    VK_LSHIFT,
    VK_LCONTROL,
    VK_LMENU,
];

// The handful of virtual-key codes this module names. Full list lives in the Windows SDK.
/// Shift.
pub const VK_SHIFT: u16 = 0x10;
/// Ctrl.
pub const VK_CONTROL: u16 = 0x11;
/// Alt (Windows calls it MENU).
pub const VK_MENU: u16 = 0x12;
/// Escape.
pub const VK_ESCAPE: u16 = 0x1B;
/// Left Windows key.
pub const VK_LWIN: u16 = 0x5B;
/// Right Windows key.
pub const VK_RWIN: u16 = 0x5C;
/// Left Shift.
pub const VK_LSHIFT: u16 = 0xA0;
/// Left Ctrl.
pub const VK_LCONTROL: u16 = 0xA2;
/// Left Alt.
pub const VK_LMENU: u16 = 0xA4;
/// Delete.
pub const VK_DELETE: u16 = 0x2E;

/// Clamps a screen fraction into `0.0..=1.0`, mapping NaN to the middle of the screen.
fn clamp_fraction(value: f32) -> f32 {
    if value.is_nan() {
        0.5
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// One key event as the viewer sees it, before the gate decides where it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// Windows virtual-key code.
    pub virtual_key: u16,
    /// Down or up.
    pub state: ButtonState,
    /// Whether Ctrl was held.
    pub ctrl: bool,
    /// Whether Alt was held.
    pub alt: bool,
    /// Whether Shift was held.
    pub shift: bool,
}

/// What the viewer should do with a keystroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRouting {
    /// Send it to the student PC and do not let this PC act on it.
    Remote,
    /// Let this PC handle it normally (the viewer is not capturing).
    Local,
    /// Stop capturing and hand the keyboard back — the exit chord.
    Release,
}

/// Decides where each keystroke goes while a remote-control window is open.
///
/// The rule a teacher needs to remember is one line: **while you are controlling a PC, every key
/// goes to that PC, and `Ctrl+Alt+Esc` gives your keyboard back.**
///
/// The exit chord is deliberately *not* a single key (a student watching over a shoulder could press
/// it by accident), not `Ctrl+Alt+Del` (impossible to capture) and not `Esc` alone (needed by the
/// remote program).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyGate {
    capturing: bool,
}

impl KeyGate {
    /// A gate that is not capturing: everything stays local.
    #[must_use]
    pub const fn new() -> Self {
        Self { capturing: false }
    }

    /// Whether keys are currently being sent to the remote PC.
    #[must_use]
    pub const fn is_capturing(self) -> bool {
        self.capturing
    }

    /// Starts forwarding keys to the remote PC.
    pub fn start_capturing(&mut self) {
        self.capturing = true;
    }

    /// Stops forwarding keys.
    pub fn stop_capturing(&mut self) {
        self.capturing = false;
    }

    /// Whether this event is the exit chord, `Ctrl+Alt+Esc`.
    #[must_use]
    pub fn is_exit_chord(event: KeyEvent) -> bool {
        event.state == ButtonState::Down
            && event.virtual_key == VK_ESCAPE
            && event.ctrl
            && event.alt
    }

    /// Routes one keystroke, updating the gate when the exit chord fires.
    pub fn route(&mut self, event: KeyEvent) -> KeyRouting {
        if !self.capturing {
            return KeyRouting::Local;
        }
        if Self::is_exit_chord(event) {
            self.capturing = false;
            return KeyRouting::Release;
        }
        KeyRouting::Remote
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        BlockInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBD_EVENT_FLAGS, KEYBDINPUT,
        KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, MOUSE_EVENT_FLAGS, MOUSEEVENTF_ABSOLUTE,
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
        MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK,
        MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput, VIRTUAL_KEY,
    };

    use super::{ButtonState, InputError, MouseButton};

    pub fn set_local_input_blocked(blocked: bool) -> Result<(), InputError> {
        // SAFETY: BlockInput takes a plain bool and keeps no state of ours. Injected SendInput still
        // works while a block is active, which is exactly what lets the teacher keep control.
        unsafe { BlockInput(blocked) }.map_err(|_| InputError::Refused)
    }

    pub fn confine_cursor(left: i32, top: i32, right: i32, bottom: i32) -> Result<(), InputError> {
        let rect = windows::Win32::Foundation::RECT {
            left,
            top,
            right,
            bottom,
        };
        // SAFETY: ClipCursor with a valid rectangle pointer that outlives the call.
        unsafe { windows::Win32::UI::WindowsAndMessaging::ClipCursor(Some(&rect)) }
            .map_err(|_| InputError::Refused)
    }

    pub fn release_cursor() {
        // SAFETY: passing null lifts any active clip; documented and side-effect-free.
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::ClipCursor(None);
        }
    }

    /// How Windows counts one wheel notch.
    const WHEEL_DELTA: i32 = 120;
    /// `SendInput`'s absolute coordinates are always 0..=65535 across the virtual desktop.
    const ABSOLUTE_RANGE: f32 = 65_535.0;

    /// Sends one prepared event, turning "0 events accepted" into a typed error.
    fn send(input: INPUT) -> Result<(), InputError> {
        // SAFETY: one fully initialised INPUT of the documented size. SendInput copies it; it keeps
        // no pointer to our memory. A return of 0 means the OS blocked it (usually UIPI).
        let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        if sent == 1 {
            Ok(())
        } else {
            Err(InputError::Refused)
        }
    }

    fn mouse(flags: MOUSE_EVENT_FLAGS, dx: i32, dy: i32, data: i32) -> INPUT {
        // mouseData is declared unsigned but carries a signed wheel delta; the bits are the same.
        let data = data as u32;
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    pub fn move_pointer(x: f32, y: f32) -> Result<(), InputError> {
        // VIRTUALDESK makes the fraction span every monitor, so controlling a second screen works.
        let dx = (x * ABSOLUTE_RANGE) as i32;
        let dy = (y * ABSOLUTE_RANGE) as i32;
        send(mouse(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            dx,
            dy,
            0,
        ))
    }

    pub fn mouse_button(button: MouseButton, state: ButtonState) -> Result<(), InputError> {
        let flags = match (button, state) {
            (MouseButton::Left, ButtonState::Down) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, ButtonState::Up) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, ButtonState::Down) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, ButtonState::Up) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, ButtonState::Down) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, ButtonState::Up) => MOUSEEVENTF_MIDDLEUP,
        };
        send(mouse(flags, 0, 0, 0))
    }

    pub fn scroll(delta: i16) -> Result<(), InputError> {
        send(mouse(
            MOUSEEVENTF_WHEEL,
            0,
            0,
            i32::from(delta) * WHEEL_DELTA / 120,
        ))
    }

    fn keyboard(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    pub fn key(virtual_key: u16, state: ButtonState) -> Result<(), InputError> {
        let flags = match state {
            ButtonState::Down => KEYBD_EVENT_FLAGS(0),
            ButtonState::Up => KEYEVENTF_KEYUP,
        };
        send(keyboard(virtual_key, 0, flags))
    }

    pub fn unicode_char(ch: char, state: ButtonState) -> Result<(), InputError> {
        let mut buffer = [0u16; 2];
        let units = ch.encode_utf16(&mut buffer);
        for unit in units {
            let mut flags = KEYEVENTF_UNICODE;
            if state == ButtonState::Up {
                flags |= KEYEVENTF_KEYUP;
            }
            send(keyboard(0, *unit, flags))?;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{ButtonState, InputError, MouseButton};

    // ponytail: Linux gets uinput (works on X11 and Wayland), macOS CGEvent; Phase 5.
    pub fn move_pointer(_x: f32, _y: f32) -> Result<(), InputError> {
        Err(InputError::NotSupported)
    }
    pub fn mouse_button(_b: MouseButton, _s: ButtonState) -> Result<(), InputError> {
        Err(InputError::NotSupported)
    }
    pub fn scroll(_delta: i16) -> Result<(), InputError> {
        Err(InputError::NotSupported)
    }
    pub fn key(_vk: u16, _s: ButtonState) -> Result<(), InputError> {
        Err(InputError::NotSupported)
    }
    pub fn unicode_char(_ch: char, _s: ButtonState) -> Result<(), InputError> {
        Err(InputError::NotSupported)
    }
    pub fn set_local_input_blocked(_blocked: bool) -> Result<(), InputError> {
        Err(InputError::NotSupported)
    }
    pub fn confine_cursor(_l: i32, _t: i32, _r: i32, _b: i32) -> Result<(), InputError> {
        Err(InputError::NotSupported)
    }
    pub fn release_cursor() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn down(vk: u16, ctrl: bool, alt: bool) -> KeyEvent {
        KeyEvent {
            virtual_key: vk,
            state: ButtonState::Down,
            ctrl,
            alt,
            shift: false,
        }
    }

    #[test]
    fn a_fraction_outside_the_screen_is_clamped_not_wrapped() {
        assert_eq!(clamp_fraction(-5.0), 0.0);
        assert_eq!(clamp_fraction(5.0), 1.0);
        assert_eq!(clamp_fraction(0.25), 0.25);
    }

    #[test]
    fn a_nan_fraction_lands_in_the_middle_rather_than_anywhere() {
        assert_eq!(clamp_fraction(f32::NAN), 0.5);
    }

    #[test]
    fn nothing_is_forwarded_while_the_gate_is_not_capturing() {
        let mut gate = KeyGate::new();
        assert!(!gate.is_capturing());
        assert_eq!(
            gate.route(down(b'A'.into(), false, false)),
            KeyRouting::Local
        );
        // Even the exit chord is local when we were never capturing.
        assert_eq!(gate.route(down(VK_ESCAPE, true, true)), KeyRouting::Local);
    }

    #[test]
    fn every_ordinary_key_goes_to_the_remote_pc_while_capturing() {
        let mut gate = KeyGate::new();
        gate.start_capturing();
        for vk in [b'A'.into(), VK_ESCAPE, VK_DELETE, VK_LWIN, VK_SHIFT] {
            assert_eq!(gate.route(down(vk, false, false)), KeyRouting::Remote);
        }
    }

    #[test]
    fn the_windows_key_is_forwarded_rather_than_opening_the_local_start_menu() {
        // The headline requirement from the brief: pressing Win opens Start on the *remote* PC.
        let mut gate = KeyGate::new();
        gate.start_capturing();
        assert_eq!(gate.route(down(VK_LWIN, false, false)), KeyRouting::Remote);
        assert_eq!(gate.route(down(VK_RWIN, false, false)), KeyRouting::Remote);
    }

    #[test]
    fn the_exit_chord_releases_the_keyboard_and_stops_capturing() {
        let mut gate = KeyGate::new();
        gate.start_capturing();
        assert_eq!(gate.route(down(VK_ESCAPE, true, true)), KeyRouting::Release);
        assert!(!gate.is_capturing(), "the gate must let go");
        assert_eq!(
            gate.route(down(b'A'.into(), false, false)),
            KeyRouting::Local
        );
    }

    #[test]
    fn escape_alone_still_reaches_the_remote_program() {
        // A student's full-screen game or dialog needs plain Esc; only the full chord exits.
        let mut gate = KeyGate::new();
        gate.start_capturing();
        assert_eq!(
            gate.route(down(VK_ESCAPE, false, false)),
            KeyRouting::Remote
        );
        assert_eq!(gate.route(down(VK_ESCAPE, true, false)), KeyRouting::Remote);
        assert_eq!(gate.route(down(VK_ESCAPE, false, true)), KeyRouting::Remote);
        assert!(gate.is_capturing());
    }

    #[test]
    fn releasing_the_chord_keys_does_not_re_trigger_the_exit() {
        let mut gate = KeyGate::new();
        gate.start_capturing();
        let key_up = KeyEvent {
            virtual_key: VK_ESCAPE,
            state: ButtonState::Up,
            ctrl: true,
            alt: true,
            shift: false,
        };
        assert!(!KeyGate::is_exit_chord(key_up), "key-up is not the chord");
        assert_eq!(gate.route(key_up), KeyRouting::Remote);
        assert!(gate.is_capturing());
    }

    #[test]
    fn the_modifier_list_covers_both_sides_of_the_keyboard() {
        // Whatever we may have pressed remotely must be releasable, or the student is left stuck.
        for vk in [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN] {
            assert!(MODIFIER_KEYS.contains(&vk), "missing {vk:#X}");
        }
    }
}
