//! The app catalogue: which programs this PC offers, and starting one of them.
//!
//! # Why the Console never sends a path
//!
//! AGENTS.md §5 forbids remote shells and arbitrary command execution in every tier. A naive
//! launcher ("run `C:\…\thing.exe`") is exactly that: whoever holds a Console key could run anything
//! on forty PCs.
//!
//! So the direction is reversed. **The Agent enumerates its own Start Menu** and publishes a
//! catalogue of `(id, name)` pairs. A Console may only say *"start catalogue entry 0x8f2c1a"*. The
//! path never crosses the wire and is never accepted from the wire — an id that is not in this PC's
//! own catalogue simply does not resolve, so there is nothing to inject.
//!
//! Ids are a hash of the shortcut path, so they are stable across re-enumeration and across a
//! reboot, but they are not guessable into anything useful: an unknown id is refused.

use std::path::{Path, PathBuf};

/// One program this PC can start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledApp {
    /// Stable identifier derived from the shortcut path. This is what a Console names.
    pub id: u32,
    /// What to show a teacher, e.g. `Calculator`.
    pub name: String,
    /// Where the shortcut lives. Never sent over the wire.
    pub path: PathBuf,
}

/// Errors listing or starting programs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    /// No catalogue entry has that id on this PC.
    #[error("this PC has no such program")]
    UnknownApp,
    /// The OS refused to start it.
    #[error("the program could not be started: {0}")]
    LaunchFailed(String),
    /// Not implemented on this platform yet.
    #[error("starting programs is not supported on this platform")]
    NotSupported,
}

/// How deep to walk the Start Menu tree. Two levels covers `Programs\Vendor\App.lnk`.
const MAX_DEPTH: usize = 3;
/// A sane cap so a pathological Start Menu cannot produce an enormous message.
pub const MAX_APPS: usize = 250;

/// A stable id for a shortcut path (FNV-1a, case-insensitive).
///
/// Case-insensitive because Windows paths are, so the same shortcut always gets the same id however
/// it was spelled when it was found.
#[must_use]
pub fn app_id(path: &Path) -> u32 {
    let text = path.to_string_lossy().to_ascii_lowercase();
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Turns a shortcut file name into something a teacher would recognise.
#[must_use]
pub fn display_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Lists the programs on this PC's Start Menu, sorted by name, de-duplicated by id.
///
/// Both the all-users and the current-user Start Menu are searched, because a school image usually
/// installs for all users while a teacher's own extras land in their profile.
#[must_use]
pub fn list_apps() -> Vec<InstalledApp> {
    let mut apps: Vec<InstalledApp> = Vec::new();
    for root in start_menu_roots() {
        collect(&root, 0, &mut apps);
    }
    apps.sort_by(|a, b| {
        a.name
            .to_ascii_lowercase()
            .cmp(&b.name.to_ascii_lowercase())
    });
    apps.dedup_by_key(|a| a.id);
    apps.truncate(MAX_APPS);
    apps
}

/// The directories a Start Menu lives in.
fn start_menu_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    // All users, then this user. Both may be absent on a stripped-down image.
    for (var, tail) in [
        ("ProgramData", r"Microsoft\Windows\Start Menu\Programs"),
        ("APPDATA", r"Microsoft\Windows\Start Menu\Programs"),
    ] {
        if let Some(base) = std::env::var_os(var) {
            let path = PathBuf::from(base).join(tail);
            if path.is_dir() {
                roots.push(path);
            }
        }
    }
    roots
}

/// Walks one directory, adding every shortcut found.
fn collect(dir: &Path, depth: usize, out: &mut Vec<InstalledApp>) {
    if depth > MAX_DEPTH || out.len() >= MAX_APPS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // unreadable folder: skip it rather than fail the whole listing
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, depth + 1, out);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
        {
            let name = display_name(&path);
            // "Uninstall …" entries are noise at best and a hazard at worst.
            if name.is_empty() || name.to_ascii_lowercase().starts_with("uninstall") {
                continue;
            }
            out.push(InstalledApp {
                id: app_id(&path),
                name,
                path,
            });
        }
        if out.len() >= MAX_APPS {
            return;
        }
    }
}

/// Starts the catalogue entry with this id.
///
/// Re-enumerates and resolves the id locally: the caller supplies only a number, so there is no path
/// to validate and nothing to inject.
///
/// # Errors
/// [`AppError::UnknownApp`] if this PC has no such entry, or [`AppError::LaunchFailed`].
pub fn launch(id: u32) -> Result<String, AppError> {
    let app = list_apps()
        .into_iter()
        .find(|a| a.id == id)
        .ok_or(AppError::UnknownApp)?;
    imp::open(&app.path)?;
    Ok(app.name)
}

/// The icon for a catalogue entry, as `(width, height, top-down BGRA)`.
///
/// Resolves the id locally exactly as [`launch`] does — the caller supplies only a number — then asks
/// the shell for the shortcut's icon. Returns `None` when the id is unknown or the PC has no icon for
/// it, so a missing icon is never an error, just an absent picture.
#[must_use]
pub fn icon_bgra(id: u32) -> Option<(u16, u16, Vec<u8>)> {
    let app = list_apps().into_iter().find(|a| a.id == id)?;
    imp::icon_bgra(&app.path)
}

#[cfg(windows)]
mod imp {
    use std::{os::windows::ffi::OsStrExt, path::Path};

    use windows::{
        Win32::{
            Graphics::Gdi::{
                BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC,
                GetDIBits, GetObjectW, HGDIOBJ, ReleaseDC,
            },
            UI::{
                Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW, ShellExecuteW},
                WindowsAndMessaging::{DestroyIcon, GetIconInfo, ICONINFO, SW_SHOWNORMAL},
            },
        },
        core::PCWSTR,
    };

    use super::AppError;

    /// The largest icon we will read; a shortcut icon is at most 256×256 and usually 32–48.
    const MAX_ICON: i32 = 256;

    /// Reads a shortcut's icon into top-down BGRA pixels.
    ///
    /// Uses `SHGetFileInfoW` to get the shell icon, then GDI (`GetIconInfo` + `GetDIBits`) to read its
    /// pixels. Returns `None` on any failure, so a program without an icon simply has none.
    pub fn icon_bgra(path: &Path) -> Option<(u16, u16, Vec<u8>)> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut info = SHFILEINFOW::default();
        // SAFETY: `wide` is NUL-terminated and outlives the call; `info` is a valid out-param sized
        // correctly. On success `info.hIcon` is an icon we own and destroy below.
        let got = unsafe {
            SHGetFileInfoW(
                PCWSTR(wide.as_ptr()),
                Default::default(),
                Some(&mut info),
                u32::try_from(std::mem::size_of::<SHFILEINFOW>()).unwrap_or(0),
                SHGFI_ICON | SHGFI_LARGEICON,
            )
        };
        if got == 0 || info.hIcon.is_invalid() {
            return None;
        }
        // SAFETY: `info.hIcon` is a valid icon handle from the call above; it is destroyed here.
        let result = unsafe { hicon_to_bgra(info.hIcon) };
        // SAFETY: destroying the icon we were handed.
        unsafe {
            let _ = DestroyIcon(info.hIcon);
        }
        result
    }

    /// Converts an `HICON` to top-down BGRA. Every GDI object created here is freed before returning.
    ///
    /// # Safety
    /// `hicon` must be a valid icon handle owned by the caller.
    unsafe fn hicon_to_bgra(
        hicon: windows::Win32::UI::WindowsAndMessaging::HICON,
    ) -> Option<(u16, u16, Vec<u8>)> {
        // SAFETY: standard GDI calls; each handle obtained is deleted, and the pixel buffer is sized
        // to width*height*4 before GetDIBits writes into it.
        unsafe {
            let mut icon = ICONINFO::default();
            GetIconInfo(hicon, &mut icon).ok()?;
            let color = icon.hbmColor;
            let mask = icon.hbmMask;

            let mut bitmap = BITMAP::default();
            let read = GetObjectW(
                HGDIOBJ(color.0),
                i32::try_from(std::mem::size_of::<BITMAP>()).unwrap_or(0),
                Some((&mut bitmap as *mut BITMAP).cast()),
            );
            let (w, h) = (bitmap.bmWidth, bitmap.bmHeight);
            let out = if read != 0 && w > 0 && h > 0 && w <= MAX_ICON && h <= MAX_ICON {
                let mut bmi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
                        biWidth: w,
                        biHeight: -h, // negative height => top-down rows, matching the wire format
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: 0, // BI_RGB
                        ..Default::default()
                    },
                    ..Default::default()
                };
                let mut pixels = vec![0u8; (w as usize) * (h as usize) * 4];
                let dc = GetDC(None);
                let lines = GetDIBits(
                    dc,
                    color,
                    0,
                    u32::try_from(h).unwrap_or(0),
                    Some(pixels.as_mut_ptr().cast()),
                    &mut bmi,
                    DIB_RGB_COLORS,
                );
                ReleaseDC(None, dc);
                if lines != 0 {
                    // Some icons come back fully transparent (they carry a 1-bit mask, not alpha).
                    // If every alpha byte is zero, treat the icon as opaque so it is visible.
                    if pixels.iter().skip(3).step_by(4).all(|&a| a == 0) {
                        for a in pixels.iter_mut().skip(3).step_by(4) {
                            *a = 255;
                        }
                    }
                    u16::try_from(w)
                        .ok()
                        .zip(u16::try_from(h).ok())
                        .map(|(w, h)| (w, h, pixels))
                } else {
                    None
                }
            } else {
                None
            };

            let _ = DeleteObject(HGDIOBJ(color.0));
            let _ = DeleteObject(HGDIOBJ(mask.0));
            out
        }
    }

    /// Asks the shell to open a shortcut the Agent itself found.
    ///
    /// `ShellExecuteW` with the `open` verb and **no arguments**: Windows resolves the `.lnk` and
    /// starts its target. Nothing here parses a command line, so there is no place for an argument
    /// or a `&&` to be smuggled in.
    pub fn open(path: &Path) -> Result<(), AppError> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        // SAFETY: both strings are NUL-terminated and outlive the call; the remaining arguments are
        // the documented "no parameters, no working directory" form. The return is a status-like
        // HINSTANCE: values <= 32 mean failure.
        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if result.0 as usize > 32 {
            Ok(())
        } else {
            Err(AppError::LaunchFailed(format!(
                "the shell refused (code {})",
                result.0 as usize
            )))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    use super::AppError;

    // ponytail: Linux reads .desktop files and launches with gio/xdg-open; Phase 5.
    pub fn open(_path: &Path) -> Result<(), AppError> {
        Err(AppError::NotSupported)
    }

    pub fn icon_bgra(_path: &Path) -> Option<(u16, u16, Vec<u8>)> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_stable_for_the_same_path() {
        let path = Path::new(r"C:\Programs\Calculator.lnk");
        assert_eq!(app_id(path), app_id(path));
    }

    #[test]
    fn an_id_ignores_the_case_windows_itself_ignores() {
        assert_eq!(
            app_id(Path::new(r"C:\Programs\Calculator.lnk")),
            app_id(Path::new(r"c:\programs\CALCULATOR.LNK"))
        );
    }

    #[test]
    fn different_programs_get_different_ids() {
        assert_ne!(
            app_id(Path::new(r"C:\Programs\Calculator.lnk")),
            app_id(Path::new(r"C:\Programs\Paint.lnk"))
        );
    }

    #[test]
    fn the_display_name_drops_the_folder_and_the_extension() {
        assert_eq!(
            display_name(Path::new(r"C:\Programs\Accessories\Paint.lnk")),
            "Paint"
        );
    }

    #[test]
    fn an_id_that_is_not_in_the_catalogue_is_refused() {
        // The security property: a Console cannot name anything this PC did not publish.
        assert_eq!(launch(0xDEAD_BEEF), Err(AppError::UnknownApp));
    }

    #[cfg(windows)]
    #[test]
    fn this_pc_publishes_a_catalogue_with_usable_entries() {
        let apps = list_apps();
        assert!(!apps.is_empty(), "a Windows PC has Start Menu shortcuts");
        assert!(apps.len() <= MAX_APPS, "the catalogue is capped");
        assert!(
            apps.iter().all(|a| !a.name.is_empty()),
            "every entry is nameable"
        );
        // Ids must be unique, or "launch entry N" would be ambiguous.
        let mut ids: Vec<u32> = apps.iter().map(|a| a.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len(), "catalogue ids must be unique");
    }

    #[cfg(windows)]
    #[test]
    fn the_catalogue_hides_uninstallers() {
        assert!(
            !list_apps()
                .iter()
                .any(|a| a.name.to_ascii_lowercase().starts_with("uninstall")),
            "an uninstaller must never be one click away for a student"
        );
    }
}
