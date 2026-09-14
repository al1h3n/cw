//! Translations loaded from plain `.ini` files.
//!
//! English and Russian ship inside the binary. Anyone can add a language without rebuilding: drop an
//! `.ini` next to the app (the `languages` folder in the data directory) and it shows up in the
//! switcher. [`template`] writes the English file out as a starting point for a translator.
//!
//! The format is deliberately boring so a teacher can edit it in Notepad:
//!
//! ```ini
//! [meta]
//! code = kk
//! name = Қазақша
//!
//! [strings]
//! room = Сынып
//! ```
//!
//! Placeholders are `{0}`, `{1}`; a translator may reorder them but must keep them.

use std::collections::BTreeMap;

/// English, always available and used to fill any gap in another language.
const EN: &str = include_str!("../languages/en.ini");
/// Russian, the other first-class language (AGENTS.md D17).
const RU: &str = include_str!("../languages/ru.ini");

/// One language: its code, its display name, and its strings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Catalog {
    /// Short code, e.g. `en`, `ru`, `kk`.
    pub code: String,
    /// Name shown in the switcher, written in that language.
    pub name: String,
    /// key -> translated text.
    pub strings: BTreeMap<String, String>,
}

/// Why a translation file could not be used.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CatalogError {
    /// `[meta] code` was missing or empty.
    #[error("the file has no 'code' in its [meta] section")]
    MissingCode,
    /// `[meta] name` was missing or empty.
    #[error("the file has no 'name' in its [meta] section")]
    MissingName,
    /// The `[strings]` section had no entries.
    #[error("the file has no translated strings")]
    NoStrings,
}

/// Parses one `.ini` translation file.
///
/// Unknown sections and keys are ignored, so a file from a newer version still loads. Lines starting
/// with `;` or `#` are comments; blank lines are skipped; the value is everything after the first `=`.
///
/// # Errors
/// Returns [`CatalogError`] if the meta block or the strings are missing.
pub fn parse(text: &str) -> Result<Catalog, CatalogError> {
    let mut section = String::new();
    let (mut code, mut name) = (String::new(), String::new());
    let mut strings = BTreeMap::new();

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            section = header.trim().to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match section.as_str() {
            "meta" => match key.to_ascii_lowercase().as_str() {
                "code" => code = value.to_owned(),
                "name" => name = value.to_owned(),
                _ => {}
            },
            "strings" if !key.is_empty() => {
                strings.insert(key.to_owned(), value.to_owned());
            }
            _ => {}
        }
    }

    if code.is_empty() {
        return Err(CatalogError::MissingCode);
    }
    if name.is_empty() {
        return Err(CatalogError::MissingName);
    }
    if strings.is_empty() {
        return Err(CatalogError::NoStrings);
    }
    Ok(Catalog {
        code,
        name,
        strings,
    })
}

/// English, which every other language falls back to key by key.
///
/// # Panics
/// Only if the bundled English file is malformed, which a test prevents.
#[must_use]
pub fn english() -> Catalog {
    parse(EN).unwrap_or_else(|e| unreachable!("bundled English file is invalid: {e}"))
}

/// Every available language: the two built in, plus any `.ini` found in `dir`.
///
/// A file whose code matches a built-in replaces it, so a school can correct our wording. Files that
/// fail to parse are skipped rather than breaking the app; the reason is returned alongside.
#[must_use]
pub fn available(dir: &std::path::Path) -> (Vec<Catalog>, Vec<String>) {
    let mut by_code: BTreeMap<String, Catalog> = BTreeMap::new();
    for text in [EN, RU] {
        if let Ok(catalog) = parse(text) {
            by_code.insert(catalog.code.clone(), catalog);
        }
    }

    let mut problems = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("ini"))
            {
                continue;
            }
            match std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|t| parse(&t).map_err(|e| e.to_string()))
            {
                Ok(catalog) => {
                    by_code.insert(catalog.code.clone(), catalog);
                }
                Err(err) => problems.push(format!("{}: {err}", path.display())),
            }
        }
    }
    (by_code.into_values().collect(), problems)
}

/// The strings for `code`, with English filling in anything the translator has not done yet.
#[must_use]
pub fn resolve(catalogs: &[Catalog], code: &str) -> Catalog {
    let english = english();
    let Some(chosen) = catalogs.iter().find(|c| c.code == code) else {
        return english;
    };
    let mut strings = english.strings;
    strings.extend(chosen.strings.clone());
    Catalog {
        code: chosen.code.clone(),
        name: chosen.name.clone(),
        strings,
    }
}

/// The English file, as text, for a translator to copy and edit.
#[must_use]
pub fn template() -> &'static str {
    EN
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_languages_parse() {
        let en = parse(EN).expect("English parses");
        assert_eq!(en.code, "en");
        let ru = parse(RU).expect("Russian parses");
        assert_eq!(ru.code, "ru");
        assert_eq!(ru.name, "Русский");
        // A missing translation is a bug we want to see, so the two must cover the same keys.
        let missing: Vec<_> = en
            .strings
            .keys()
            .filter(|k| !ru.strings.contains_key(*k))
            .collect();
        assert!(missing.is_empty(), "Russian is missing keys: {missing:?}");
    }

    #[test]
    fn parses_comments_sections_and_values_with_equals() {
        let text = "; a comment\n# another\n[meta]\ncode = kk\nname = Қазақша\n\n[strings]\nroom = Сынып\nodd = a = b\n";
        let catalog = parse(text).expect("parses");
        assert_eq!(catalog.code, "kk");
        assert_eq!(catalog.name, "Қазақша");
        assert_eq!(catalog.strings["room"], "Сынып");
        assert_eq!(catalog.strings["odd"], "a = b", "only the first '=' splits");
    }

    #[test]
    fn rejects_files_that_are_missing_required_parts() {
        assert_eq!(
            parse("[strings]\nroom = x\n"),
            Err(CatalogError::MissingCode)
        );
        assert_eq!(
            parse("[meta]\ncode = xx\n[strings]\nroom = x\n"),
            Err(CatalogError::MissingName)
        );
        assert_eq!(
            parse("[meta]\ncode = xx\nname = X\n"),
            Err(CatalogError::NoStrings)
        );
    }

    #[test]
    fn unknown_sections_and_keys_are_ignored() {
        let text = "[meta]\ncode = xx\nname = X\nfuture = whatever\n[strings]\nroom = R\n[newer]\nthing = 1\n";
        let catalog = parse(text).expect("parses");
        assert_eq!(catalog.strings.len(), 1);
    }

    #[test]
    fn resolve_falls_back_to_english_for_untranslated_keys() {
        let partial =
            parse("[meta]\ncode = xx\nname = X\n[strings]\nroom = Room-XX\n").expect("parses");
        let resolved = resolve(&[partial], "xx");
        assert_eq!(resolved.strings["room"], "Room-XX", "translated key wins");
        assert_eq!(
            resolved.strings["startWatching"],
            english().strings["startWatching"],
            "untranslated key falls back to English"
        );
    }

    #[test]
    fn resolve_unknown_code_returns_english() {
        assert_eq!(resolve(&[], "zz").code, "en");
    }

    #[test]
    fn external_file_is_picked_up_and_bad_one_is_reported() {
        let dir = std::env::temp_dir().join(format!("cw-lang-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(
            dir.join("kk.ini"),
            "[meta]\ncode = kk\nname = Qazaqsha\n[strings]\nroom = Synyp\n",
        )
        .expect("write");
        std::fs::write(dir.join("broken.ini"), "nothing useful here").expect("write");

        let (catalogs, problems) = available(&dir);
        assert!(
            catalogs.iter().any(|c| c.code == "kk"),
            "external language is offered"
        );
        assert!(catalogs.iter().any(|c| c.code == "en"), "built-ins remain");
        assert_eq!(problems.len(), 1, "the broken file is reported, not fatal");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
