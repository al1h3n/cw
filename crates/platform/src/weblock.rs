//! Blocking websites through the **browsers' own policy** system, not a proxy or extension.
//!
//! Chrome and Edge read a `URLBlocklist` policy from the registry; Firefox reads a `WebsiteFilter`
//! from a `policies.json` next to the executable. Setting those is the documented, reversible way to
//! block sites (AGENTS §5: "prefer OS/vendor policy over fighting the OS"), needs no extension, and
//! the browser keeps enforcing it after a reboot on its own.
//!
//! The policy locations are machine-wide (HKLM / Program Files), so writing them needs administrator
//! rights — i.e. the Agent running as the SYSTEM service. Unelevated it returns an error, exactly like
//! the wallpaper-lock policy.

/// Why setting the website policy failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WeblockError {
    /// No implementation on this platform yet.
    #[error("website blocking is not supported on this platform")]
    NotSupported,
    /// A registry or file write failed (often "access denied" when not running as SYSTEM/admin).
    #[error("could not write the browser policy: {0}")]
    Os(String),
}

/// Applies `patterns` as the website blocklist across the installed browsers (an empty list clears it).
/// Returns how many patterns were written.
///
/// # Errors
/// [`WeblockError`] if the policy could not be written (typically needs the SYSTEM service).
pub fn set_url_blocklist(patterns: &[String]) -> Result<u16, WeblockError> {
    imp::set_url_blocklist(patterns)
}

/// Turns a teacher-entered domain into a Firefox `WebsiteFilter` match pattern. A bare domain becomes
/// `*://*.domain/*`; anything that already looks like a pattern (has `*` or `://`) is kept as-is.
#[must_use]
pub fn firefox_pattern(entry: &str) -> String {
    let entry = entry.trim();
    if entry.contains('*') || entry.contains("://") {
        entry.to_string()
    } else {
        format!("*://*.{entry}/*")
    }
}

#[cfg(test)]
mod tests {
    use super::firefox_pattern;

    #[test]
    fn bare_domains_become_match_patterns_and_patterns_pass_through() {
        assert_eq!(firefox_pattern("youtube.com"), "*://*.youtube.com/*");
        assert_eq!(firefox_pattern("  tiktok.com "), "*://*.tiktok.com/*");
        assert_eq!(
            firefox_pattern("*://*.example.com/*"),
            "*://*.example.com/*"
        );
        assert_eq!(firefox_pattern("https://x.com/y"), "https://x.com/y");
    }
}

#[cfg(windows)]
mod imp {
    use windows::{
        Win32::System::Registry::{
            HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
            RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
        },
        core::PCWSTR,
    };

    use super::{WeblockError, firefox_pattern};

    /// Chrome and Edge both read a numbered `URLBlocklist` under these HKLM policy keys.
    const CHROME_KEY: &str = "SOFTWARE\\Policies\\Google\\Chrome\\URLBlocklist";
    const EDGE_KEY: &str = "SOFTWARE\\Policies\\Microsoft\\Edge\\URLBlocklist";

    pub fn set_url_blocklist(patterns: &[String]) -> Result<u16, WeblockError> {
        let patterns: Vec<String> = patterns
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect();

        // Chrome and Edge take the domains as entered; a bare domain blocks the whole site.
        write_registry_list(CHROME_KEY, &patterns)?;
        write_registry_list(EDGE_KEY, &patterns)?;
        // Firefox wants match patterns and reads a file; best-effort (it may not be installed).
        write_firefox_policy(&patterns);

        Ok(u16::try_from(patterns.len()).unwrap_or(u16::MAX))
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Replaces the numbered values under one browser's `URLBlocklist` key with `patterns`.
    fn write_registry_list(subkey: &str, patterns: &[String]) -> Result<(), WeblockError> {
        let key_w = wide(subkey);
        // Clear any previous list so a shorter new list never leaves stale numbered values behind.
        // SAFETY: deletes the subtree under HKLM; a missing key is not an error we care about.
        unsafe {
            let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(key_w.as_ptr()));
        }
        if patterns.is_empty() {
            return Ok(());
        }
        let mut key = HKEY::default();
        // SAFETY: create-or-open the policy key; closed on every path below.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key_w.as_ptr()),
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
            return Err(WeblockError::Os(status.to_hresult().message()));
        }
        let mut result = Ok(());
        for (i, pattern) in patterns.iter().enumerate() {
            let name = wide(&(i + 1).to_string());
            let value = wide(pattern);
            let bytes =
                unsafe { std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2) };
            // SAFETY: REG_SZ value whose byte length includes the trailing NUL; both buffers outlive it.
            let set =
                unsafe { RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) };
            if set.is_err() {
                result = Err(WeblockError::Os(set.to_hresult().message()));
                break;
            }
        }
        // SAFETY: closing the key we opened.
        unsafe {
            let _ = RegCloseKey(key);
        }
        result
    }

    /// Writes (or clears) Firefox's `policies.json` in every Firefox install found. Best-effort.
    fn write_firefox_policy(patterns: &[String]) {
        let blocks: Vec<String> = patterns.iter().map(|p| firefox_pattern(p)).collect();
        // Manual JSON (platform stays free of serde); the strings are simple match patterns.
        let list = blocks
            .iter()
            .map(|p| format!("\"{}\"", p.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(",");
        let json = format!(
            "{{\"policies\":{{\"WebsiteFilter\":{{\"Block\":[{list}],\"Exceptions\":[]}}}}}}"
        );
        for base in firefox_dirs() {
            let dist = base.join("distribution");
            if std::fs::create_dir_all(&dist).is_ok() {
                let _ = std::fs::write(dist.join("policies.json"), &json);
            }
        }
    }

    /// Firefox install directories that actually exist (64- and 32-bit Program Files).
    fn firefox_dirs() -> Vec<std::path::PathBuf> {
        ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|p| std::path::PathBuf::from(p).join("Mozilla Firefox"))
            .filter(|dir| dir.join("firefox.exe").exists())
            .collect()
    }
}

#[cfg(not(windows))]
mod imp {
    use super::WeblockError;

    // ponytail: Linux writes Chrome/Chromium managed policy JSON under /etc/opt/chrome/policies and
    // Firefox policies.json under /etc/firefox; macOS uses configuration profiles. Added with those Agents.
    pub fn set_url_blocklist(_patterns: &[String]) -> Result<u16, WeblockError> {
        Err(WeblockError::NotSupported)
    }
}
