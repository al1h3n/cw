//! Multiple classrooms, stored as separate profile directories.
//!
//! A **classroom** is one lab: its own console identity, paired devices, room password and blocklist.
//! Because every one of those already lives in a single data directory (see [`crate::manager`]), a
//! classroom is simply a data directory — switching classrooms means pointing the console at a
//! different one. A teacher runs one console instance per classroom and can have several open at once
//! (`cowatcher-console --classroom <slug>`), which is exactly what the [`COWATCHER_DIR`] override
//! already made possible for one directory.
//!
//! Layout under the base directory (`%LOCALAPPDATA%\co-watcher\console`, or `COWATCHER_DIR`):
//!
//! ```text
//! <base>/                 the "default" classroom (also where old single-room installs already are)
//! <base>/classrooms/<slug>/   every additional classroom
//! ```
//!
//! The base directory *is* the default classroom, on purpose: an existing install already keeps its
//! `device.key`/`trust.bin` there, so treating it as "default" means multi-classroom support needs no
//! risky migration of the very files that hold every pairing. New classrooms are created as siblings.

use std::path::{Path, PathBuf};

/// The slug of the always-present classroom that lives in the base directory itself.
pub const DEFAULT_SLUG: &str = "default";

/// One classroom for the switcher UI.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ClassroomInfo {
    /// Stable folder-safe id, used on the command line and to switch.
    pub slug: String,
    /// The teacher-facing label.
    pub name: String,
    /// True for the classroom this console instance is currently showing.
    pub active: bool,
}

/// The base directory that holds every classroom (honours `COWATCHER_DIR`).
#[must_use]
pub fn base_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("COWATCHER_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("co-watcher").join("console")
}

/// The data directory for one classroom slug. The default classroom is the base directory itself.
#[must_use]
pub fn dir_for(base: &Path, slug: &str) -> PathBuf {
    if slug == DEFAULT_SLUG || slug.is_empty() {
        base.to_path_buf()
    } else {
        base.join("classrooms").join(sanitize(slug))
    }
}

/// The file inside a classroom directory that stores its teacher-facing label.
fn label_path(dir: &Path) -> PathBuf {
    dir.join("classroom.txt")
}

/// The label for a classroom directory, falling back to a sensible default.
#[must_use]
pub fn label_of(dir: &Path, slug: &str) -> String {
    std::fs::read_to_string(label_path(dir))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if slug == DEFAULT_SLUG {
                "Classroom".to_string()
            } else {
                slug.to_string()
            }
        })
}

/// Every classroom that exists, the default first, marking `active_slug` as active.
#[must_use]
pub fn list(base: &Path, active_slug: &str) -> Vec<ClassroomInfo> {
    let mut out = vec![ClassroomInfo {
        slug: DEFAULT_SLUG.to_string(),
        name: label_of(base, DEFAULT_SLUG),
        active: active_slug == DEFAULT_SLUG || active_slug.is_empty(),
    }];
    if let Ok(entries) = std::fs::read_dir(base.join("classrooms")) {
        let mut extra: Vec<ClassroomInfo> = entries
            .flatten()
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().into_string().ok())
            .map(|slug| ClassroomInfo {
                name: label_of(&dir_for(base, &slug), &slug),
                active: active_slug == slug,
                slug,
            })
            .collect();
        extra.sort_by_key(|c| c.name.to_lowercase());
        out.extend(extra);
    }
    out
}

/// Creates a new classroom from a teacher-typed name and returns it. The name becomes the label; a
/// folder-safe, unique slug is derived from it.
///
/// # Errors
/// If the name is blank/too long, or the directory cannot be created.
pub fn create(base: &Path, name: &str) -> Result<ClassroomInfo, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("the classroom needs a name".into());
    }
    if name.chars().count() > proto::MAX_ROOM_NAME {
        return Err(format!(
            "the classroom name may be at most {} characters",
            proto::MAX_ROOM_NAME
        ));
    }
    let slug = unique_slug(base, name);
    let dir = dir_for(base, &slug);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    std::fs::write(label_path(&dir), name).map_err(|e| e.to_string())?;
    Ok(ClassroomInfo {
        slug,
        name: name.to_string(),
        active: false,
    })
}

/// Turns a name into a folder-safe slug: lowercase ASCII letters and digits, other runs become a
/// single dash. Empty results become `room` so a directory can always be made.
fn sanitize(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !slug.is_empty() {
            slug.push('-');
            last_dash = true;
        }
    }
    let trimmed = slug.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "room".to_string()
    } else {
        trimmed
    }
}

/// A slug that is not `default` and not already taken, suffixing `-2`, `-3`, … if needed.
fn unique_slug(base: &Path, name: &str) -> String {
    let mut base_slug = sanitize(name);
    if base_slug == DEFAULT_SLUG {
        base_slug = format!("{base_slug}-room");
    }
    let mut slug = base_slug.clone();
    let mut n = 2;
    while dir_for(base, &slug).exists() {
        slug = format!("{base_slug}-{n}");
        n += 1;
    }
    slug
}

/// Which classroom slug this instance was launched for: the `--classroom <slug>` argument, else the
/// remembered last choice, else the default. `COWATCHER_DIR` forces the default (it *is* the dir).
#[must_use]
pub fn active_slug(base: &Path, arg: Option<&str>) -> String {
    if std::env::var_os("COWATCHER_DIR").is_some() {
        return DEFAULT_SLUG.to_string();
    }
    if let Some(slug) = arg.map(str::trim).filter(|s| !s.is_empty()) {
        return sanitize(slug);
    }
    std::fs::read_to_string(base.join("last-classroom.txt"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && dir_for(base, s).exists())
        .unwrap_or_else(|| DEFAULT_SLUG.to_string())
}

/// Remembers the classroom last opened, so a plain double-click reopens it.
pub fn remember(base: &Path, slug: &str) {
    let _ = std::fs::create_dir_all(base);
    let _ = std::fs::write(base.join("last-classroom.txt"), slug);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cw-class-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn default_classroom_is_the_base_directory_itself() {
        let base = temp("default-dir");
        assert_eq!(dir_for(&base, DEFAULT_SLUG), base);
        assert_eq!(dir_for(&base, ""), base);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn create_makes_a_unique_slug_and_stores_the_label() {
        let base = temp("create");
        let a = create(&base, "Lab 7").expect("create");
        assert_eq!(a.slug, "lab-7");
        assert_eq!(a.name, "Lab 7");
        // The same name again gets a distinct slug rather than colliding.
        let b = create(&base, "Lab 7").expect("create 2");
        assert_eq!(b.slug, "lab-7-2");
        assert_eq!(label_of(&dir_for(&base, "lab-7"), "lab-7"), "Lab 7");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn list_includes_default_plus_created_ones_and_marks_active() {
        let base = temp("list");
        create(&base, "Physics").expect("create");
        let rooms = list(&base, "physics");
        assert_eq!(rooms[0].slug, DEFAULT_SLUG, "default is always first");
        assert!(rooms.iter().any(|r| r.slug == "physics" && r.active));
        assert!(!rooms[0].active, "default is not active when another is");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn a_name_slugging_to_default_is_kept_distinct() {
        let base = temp("reserved");
        let made = create(&base, "Default").expect("create");
        assert_ne!(made.slug, DEFAULT_SLUG);
        let _ = std::fs::remove_dir_all(&base);
    }
}
