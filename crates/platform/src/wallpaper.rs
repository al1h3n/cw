//! Stopping a student changing the desktop wallpaper.
//!
//! The brief wants "a constant wallpaper nobody can change". Windows already has the exact switch for
//! this — the `NoChangingWallPaper` user policy — so we set it rather than fight the OS (AGENTS.md
//! §5). Because it is a registry value under HKEY_CURRENT_USER it persists across reboots on its own,
//! which is why wallpaper lock works offline without a policy engine yet.
//!
//! We deliberately do **not** push an image over the wire here: shipping a school's wallpaper file
//! needs the file-transfer channel (not built), so for now this locks whatever wallpaper is set.

/// Why a wallpaper operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WallpaperError {
    /// No implementation on this platform yet.
    #[error("not supported on this platform")]
    NotSupported,
    /// The registry write failed.
    #[error("could not change the wallpaper policy: {0}")]
    Os(String),
}

/// Prevents the user changing their wallpaper (Personalisation greys the option out).
///
/// # Errors
/// [`WallpaperError::Os`] if the policy cannot be written.
pub fn lock() -> Result<(), WallpaperError> {
    imp::set_locked(true)
}

/// Allows the user to change their wallpaper again.
///
/// # Errors
/// [`WallpaperError::Os`] if the policy cannot be written.
pub fn unlock() -> Result<(), WallpaperError> {
    imp::set_locked(false)
}

#[cfg(windows)]
mod imp {
    use windows::{
        Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, RegCloseKey,
            RegCreateKeyExW, RegSetValueExW,
        },
        core::w,
    };

    use super::WallpaperError;

    /// `HKCU\Software\Microsoft\Windows\CurrentVersion\Policies\ActiveDesktop`, where the
    /// `NoChangingWallPaper` policy lives.
    const SUBKEY: windows::core::PCWSTR =
        w!("Software\\Microsoft\\Windows\\CurrentVersion\\Policies\\ActiveDesktop");

    pub fn set_locked(locked: bool) -> Result<(), WallpaperError> {
        let mut key = HKEY::default();
        // SAFETY: RegCreateKeyExW with a valid out-pointer; the key is closed on every path. Creating
        // an existing key just opens it. The two `None`s are the documented "no class, default
        // security" form.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                SUBKEY,
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut key,
                None,
            )
        };
        if status.is_err() {
            return Err(WallpaperError::Os(status.to_hresult().message()));
        }

        let value: u32 = u32::from(locked);
        // SAFETY: writing a 4-byte DWORD; `value` outlives the call and its byte length is exact.
        let set = unsafe {
            RegSetValueExW(
                key,
                w!("NoChangingWallPaper"),
                None,
                REG_DWORD,
                Some(&value.to_ne_bytes()),
            )
        };
        // SAFETY: closing the key we opened; not used afterwards.
        unsafe {
            let _ = RegCloseKey(key);
        }
        if set.is_err() {
            return Err(WallpaperError::Os(set.to_hresult().message()));
        }
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs elevation: the HKCU Policies hive is admin/GPO-writable only, so this passes \
                only when the Agent runs as the SYSTEM service (step 1.4b)"]
    fn lock_then_unlock_both_succeed_when_elevated() {
        // Writes a real HKCU policy value (harmless, per-user) and puts it back. Unelevated this is
        // "Access is denied", which is the whole reason wallpaper lock waits for the SYSTEM service.
        lock().expect("lock wallpaper");
        unlock().expect("unlock wallpaper");
    }
}

#[cfg(not(windows))]
mod imp {
    use super::WallpaperError;

    // ponytail: GNOME/KDE lock the wallpaper through dconf/kiosk keys; added with the Linux Agent.
    pub fn set_locked(_locked: bool) -> Result<(), WallpaperError> {
        Err(WallpaperError::NotSupported)
    }
}
