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

/// Builds a tiny all-black 24-bit BMP.
///
/// A file, not a colour, because `SystemParametersInfo(SPI_SETDESKWALLPAPER)` takes an image path.
/// 2×2 is the smallest that some Windows versions will accept; it is stretched to fill the screen,
/// and black stretched is still black. Pure and testable.
#[must_use]
pub fn black_bmp() -> Vec<u8> {
    const W: i32 = 2;
    const H: i32 = 2;
    let row = (W * 3) as usize; // 6 bytes, already a multiple of 4 — no padding needed
    let pixels = row * H as usize;
    let file_size = 14 + 40 + pixels;
    let mut bmp = Vec::with_capacity(file_size);
    // BITMAPFILEHEADER
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes()); // reserved
    bmp.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
    // BITMAPINFOHEADER
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&W.to_le_bytes());
    bmp.extend_from_slice(&H.to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
    bmp.extend_from_slice(&24u16.to_le_bytes()); // bits per pixel
    bmp.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB, no compression
    bmp.extend_from_slice(&(pixels as u32).to_le_bytes());
    bmp.extend_from_slice(&2835u32.to_le_bytes()); // 72 DPI x
    bmp.extend_from_slice(&2835u32.to_le_bytes()); // 72 DPI y
    bmp.extend_from_slice(&0u32.to_le_bytes()); // colours used
    bmp.extend_from_slice(&0u32.to_le_bytes()); // colours important
    bmp.resize(file_size, 0); // pixel data: all zero = black
    bmp
}

/// Replaces the desktop wallpaper with black while a teacher is watching, and puts the student's own
/// wallpaper back afterwards (D11: "wallpaper is replaced with black while streaming").
///
/// The previous wallpaper path is written to `save_path` before the swap, so even if the Agent
/// crashes, the next start can [`restore`] it. Unlike the [`lock`] policy this works unelevated,
/// because setting *your own* session's wallpaper needs no privilege.
///
/// # Errors
/// [`WallpaperError::Os`] if the OS calls fail.
pub fn set_black(save_path: &std::path::Path) -> Result<(), WallpaperError> {
    imp::set_black(save_path)
}

/// Puts the student's own wallpaper back, using the path saved by [`set_black`]. A no-op (Ok) if no
/// wallpaper was saved, so calling it "just in case" on start-up or disconnect is safe.
///
/// # Errors
/// [`WallpaperError::Os`] if the OS call fails.
pub fn restore(save_path: &std::path::Path) -> Result<(), WallpaperError> {
    imp::restore(save_path)
}

/// Sets the desktop wallpaper to `image` (raw PNG, JPEG or BMP bytes), a lasting choice a teacher
/// pushes to a student PC.
///
/// The bytes are written to a stable file next to `save_path` (the same file [`set_black`] uses to
/// remember the student's wallpaper) and the desktop is pointed at it. If a teacher is *currently*
/// watching — so the wallpaper is blacked out — the new image is recorded as the wallpaper to
/// restore rather than shown immediately, so it appears the moment watching ends instead of being
/// overwritten by the black-out. An empty `image` clears the wallpaper to a plain background.
///
/// # Errors
/// [`WallpaperError`] if the format is not one the OS accepts, or a file/registry step fails.
pub fn set_image(image: &[u8], save_path: &std::path::Path) -> Result<(), WallpaperError> {
    imp::set_image(image, save_path)
}

/// The file extension for a wallpaper image, chosen from its magic bytes. Defaults to `bmp` so an
/// unrecognised blob is at least written with a concrete extension.
#[must_use]
pub fn image_extension(image: &[u8]) -> &'static str {
    if image.starts_with(&[0x89, b'P', b'N', b'G']) {
        "png"
    } else if image.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "jpg"
    } else if image.starts_with(b"BM") {
        "bmp"
    } else if image.starts_with(b"GIF8") {
        "gif"
    } else {
        "bmp"
    }
}

/// Snapshots the current wallpaper, blacks it out, then restores it, and reports whether the
/// original came back. A quick, self-contained way to confirm the black-on-watch mechanism works on
/// this PC — it does briefly flash the desktop black, so it is only run on request.
///
/// # Errors
/// [`WallpaperError`] if any OS step fails.
pub fn selftest(save_path: &std::path::Path) -> Result<bool, WallpaperError> {
    imp::selftest(save_path)
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
    use std::{os::windows::ffi::OsStrExt, path::Path};

    use windows::{
        Win32::{
            System::Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE,
                RegCloseKey, RegCreateKeyExW, RegSetValueExW,
            },
            UI::WindowsAndMessaging::{
                SPI_GETDESKWALLPAPER, SPI_SETDESKWALLPAPER, SPIF_SENDCHANGE, SPIF_UPDATEINIFILE,
                SystemParametersInfoW,
            },
        },
        core::w,
    };

    use super::{WallpaperError, black_bmp};

    /// The current wallpaper's path, or empty if none is set.
    fn current_wallpaper() -> String {
        let mut buffer = [0u16; 520]; // MAX_PATH is plenty
        // SAFETY: SPI_GETDESKWALLPAPER fills `buffer` up to its length with a NUL-terminated path.
        let ok = unsafe {
            SystemParametersInfoW(
                SPI_GETDESKWALLPAPER,
                buffer.len() as u32,
                Some(buffer.as_mut_ptr().cast()),
                windows::Win32::UI::WindowsAndMessaging::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        if ok.is_err() {
            return String::new();
        }
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }

    /// Points the desktop wallpaper at `path` (empty string removes the wallpaper).
    fn apply_wallpaper(path: &str) -> Result<(), WallpaperError> {
        let mut wide: Vec<u16> = Path::new(path).as_os_str().encode_wide().collect();
        wide.push(0);
        // SAFETY: `wide` is a NUL-terminated path that outlives the call; the update flags ask
        // Windows to persist the choice and notify running apps.
        unsafe {
            SystemParametersInfoW(
                SPI_SETDESKWALLPAPER,
                0,
                Some(wide.as_ptr() as *mut _),
                SPIF_UPDATEINIFILE | SPIF_SENDCHANGE,
            )
        }
        .map_err(|e| WallpaperError::Os(e.message()))
    }

    pub fn set_black(save_path: &Path) -> Result<(), WallpaperError> {
        // If we already saved a wallpaper (black is already up), do not overwrite the save with the
        // black path — that would lose the student's real wallpaper.
        if !save_path.exists() {
            let current = current_wallpaper();
            std::fs::write(save_path, &current)
                .map_err(|e| WallpaperError::Os(format!("save wallpaper path: {e}")))?;
        }
        let black = save_path.with_extension("bmp");
        std::fs::write(&black, black_bmp())
            .map_err(|e| WallpaperError::Os(format!("write black bitmap: {e}")))?;
        apply_wallpaper(&black.to_string_lossy())
    }

    pub fn restore(save_path: &Path) -> Result<(), WallpaperError> {
        let Ok(previous) = std::fs::read_to_string(save_path) else {
            return Ok(()); // nothing was blacked out
        };
        let result = apply_wallpaper(previous.trim());
        // Whether or not it worked, forget the save so a later set_black snapshots afresh.
        let _ = std::fs::remove_file(save_path);
        let _ = std::fs::remove_file(save_path.with_extension("bmp"));
        result
    }

    pub fn set_image(image: &[u8], save_path: &Path) -> Result<(), WallpaperError> {
        // An empty image means "no wallpaper": clear it and stop remembering any earlier choice.
        if image.is_empty() {
            let _ = std::fs::remove_file(save_path);
            return apply_wallpaper("");
        }
        // Write the chosen image beside the save file with a concrete extension the OS understands.
        // A fixed name (per extension) keeps the agent dir from filling with old wallpapers.
        let ext = super::image_extension(image);
        let chosen = save_path.with_file_name(format!("wallpaper-chosen.{ext}"));
        for other in ["png", "jpg", "bmp", "gif"] {
            if other != ext {
                let _ = std::fs::remove_file(
                    save_path.with_file_name(format!("wallpaper-chosen.{other}")),
                );
            }
        }
        std::fs::write(&chosen, image)
            .map_err(|e| WallpaperError::Os(format!("write wallpaper image: {e}")))?;
        let chosen = chosen.to_string_lossy().to_string();

        // If black is currently shown (a teacher is watching), don't fight the black-out: record the
        // new image as the wallpaper to restore, so it shows the instant watching ends.
        if save_path.exists() {
            std::fs::write(save_path, &chosen)
                .map_err(|e| WallpaperError::Os(format!("update saved wallpaper: {e}")))?;
            return Ok(());
        }
        apply_wallpaper(&chosen)
    }

    pub fn selftest(save_path: &Path) -> Result<bool, WallpaperError> {
        let before = current_wallpaper();
        set_black(save_path)?;
        let blacked = current_wallpaper();
        restore(save_path)?;
        let after = current_wallpaper();
        // Restored to the original, and it really did change in between.
        Ok(after == before && blacked != before)
    }

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

#[cfg(test)]
mod bmp_tests {
    use super::*;

    #[test]
    fn the_black_bitmap_is_a_valid_bmp_header() {
        let bmp = black_bmp();
        assert_eq!(&bmp[..2], b"BM", "BMP magic");
        let declared = u32::from_le_bytes(bmp[2..6].try_into().unwrap());
        assert_eq!(declared as usize, bmp.len(), "file size field matches");
        let offset = u32::from_le_bytes(bmp[10..14].try_into().unwrap());
        assert_eq!(offset, 54, "pixel data starts after the two headers");
        let bpp = u16::from_le_bytes(bmp[28..30].try_into().unwrap());
        assert_eq!(bpp, 24);
    }

    #[test]
    fn every_pixel_byte_is_zero_so_it_is_actually_black() {
        let bmp = black_bmp();
        assert!(bmp[54..].iter().all(|&b| b == 0), "pixels must be black");
    }

    #[test]
    fn wallpaper_extension_is_read_from_the_magic_bytes() {
        assert_eq!(image_extension(&[0x89, b'P', b'N', b'G', 0x0D]), "png");
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), "jpg");
        assert_eq!(image_extension(b"BM..."), "bmp");
        assert_eq!(image_extension(b"GIF89a"), "gif");
        // Our own black bitmap round-trips as a BMP.
        assert_eq!(image_extension(&black_bmp()), "bmp");
        // An unknown blob falls back to a concrete extension rather than panicking.
        assert_eq!(image_extension(&[0, 1, 2, 3]), "bmp");
        assert_eq!(image_extension(&[]), "bmp");
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "swaps the real desktop wallpaper to black and back; run with \
                `cowatcher-agent wallpaper-selftest` when you can watch the flicker"]
    fn black_then_restore_returns_the_original_wallpaper() {
        // The real live check lives in the `wallpaper-selftest` agent command, which round-trips and
        // asserts the original returns. This placeholder documents where to find it.
        let dir = std::env::temp_dir().join(format!("cw-wp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let save = dir.join("prev.txt");
        assert!(selftest(&save).expect("selftest"));
        let _ = std::fs::remove_dir_all(&dir);
    }

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

    pub fn set_black(_save_path: &std::path::Path) -> Result<(), WallpaperError> {
        Err(WallpaperError::NotSupported)
    }

    // ponytail: GNOME/KDE set the wallpaper via gsettings/plasma-apply-wallpaperimage; added with
    // the Linux Agent.
    pub fn set_image(_image: &[u8], _save_path: &std::path::Path) -> Result<(), WallpaperError> {
        Err(WallpaperError::NotSupported)
    }

    pub fn restore(_save_path: &std::path::Path) -> Result<(), WallpaperError> {
        Ok(())
    }

    pub fn selftest(_save_path: &std::path::Path) -> Result<bool, WallpaperError> {
        Err(WallpaperError::NotSupported)
    }
}
