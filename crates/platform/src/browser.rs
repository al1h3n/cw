//! Opening a URL in the user's default browser.
//!
//! Used to send the teacher to the Co-watcher web dashboard. It only ever opens an `http`/`https`
//! URL — never a local path or an arbitrary shell verb — so it cannot be turned into "run anything".

/// Why opening a URL failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BrowserError {
    /// The string was not an `http(s)` URL.
    #[error("only http(s) links can be opened")]
    NotWebUrl,
    /// No implementation on this platform yet.
    #[error("not supported on this platform")]
    NotSupported,
    /// The OS refused to open it.
    #[error("could not open the link: {0}")]
    Os(String),
}

/// Opens `url` in the default browser, but only if it is an `http`/`https` URL.
///
/// # Errors
/// [`BrowserError::NotWebUrl`] if the URL is not web, or [`BrowserError`] if the OS call fails.
pub fn open(url: &str) -> Result<(), BrowserError> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(BrowserError::NotWebUrl);
    }
    imp::open(url)
}

#[cfg(windows)]
mod imp {
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::PCWSTR,
    };

    use super::BrowserError;

    pub fn open(url: &str) -> Result<(), BrowserError> {
        let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let verb: Vec<u16> = "open\0".encode_utf16().collect();
        // SAFETY: both strings are NUL-terminated and outlive the call; passing the URL as the file
        // with the "open" verb is the documented way to hand a link to the default browser. The
        // return is a status-like HINSTANCE where values <= 32 mean failure.
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
            Err(BrowserError::Os(format!(
                "shell code {}",
                result.0 as usize
            )))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::BrowserError;

    // ponytail: Linux uses `xdg-open`, macOS `open`; wire these up with the respective Console build.
    pub fn open(_url: &str) -> Result<(), BrowserError> {
        Err(BrowserError::NotSupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_urls_are_accepted() {
        assert_eq!(open("file:///etc/passwd"), Err(BrowserError::NotWebUrl));
        assert_eq!(open("javascript:alert(1)"), Err(BrowserError::NotWebUrl));
        assert_eq!(open("  ftp://x  "), Err(BrowserError::NotWebUrl));
        // An http(s) URL passes the guard (the OS call may still fail in CI, which is fine — we only
        // assert the guard here by checking the error is not NotWebUrl).
        assert_ne!(open("https://example.com"), Err(BrowserError::NotWebUrl));
    }
}
