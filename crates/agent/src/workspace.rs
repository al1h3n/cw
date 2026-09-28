//! The shared **class workspace** folder that file transfer is confined to.
//!
//! A teacher can send files to a student PC and download files the student made (e.g. a script to
//! assess). Letting a Console read or write *anywhere* on a student PC — the Agent may run as SYSTEM —
//! is exactly the kind of power AGENTS.md §5 forbids ("file wipe code may only touch paths inside the
//! configured workspace scope, and tests prove it"). So every transfer is confined to one directory,
//! the *workspace*, and any path that could escape it (absolute, a drive prefix, or a `..` component)
//! is refused. The guard [`safe_join`] is pure and unit-tested; the I/O helpers build on it.

use std::path::{Component, Path, PathBuf};

use proto::{FileEntry, MAX_FILE_LIST, MAX_FILE_PATH};

/// The workspace root for this Agent: `COWATCHER_WORKSPACE` if set, else `Co-watcher` in the user's
/// profile, else a temp fallback. The helper runs in the logged-in student's session, so the profile
/// is the student's.
#[must_use]
pub fn default_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("COWATCHER_WORKSPACE") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map_or_else(std::env::temp_dir, PathBuf::from);
    base.join("Co-watcher")
}

/// Resolves a workspace-relative path to an absolute one **inside** `root`, or `None` if it escapes.
///
/// Only ordinary path segments (and `.`) are allowed: a leading `/`, a drive letter, or any `..`
/// component is rejected outright, so a Console can never step out of the workspace. Pure, so it is
/// tested without touching the disk; the I/O helpers add a canonicalised re-check for symlinks.
#[must_use]
pub fn safe_join(root: &Path, rel: &str) -> Option<PathBuf> {
    if rel.len() > MAX_FILE_PATH {
        return None;
    }
    let mut out = root.to_path_buf();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(segment) => out.push(segment),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    // Lexically out is always within root after the above, but keep the check as a clear invariant.
    out.starts_with(root).then_some(out)
}

/// Whether `name` is a plain file name (no separators, not `.`/`..`, not empty or over-long).
#[must_use]
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_FILE_PATH
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains('\0')
}

/// Confirms `path` really sits inside `root` after resolving symlinks (defence in depth for existing
/// paths). If either cannot be canonicalised, falls back to the lexical guarantee `safe_join` gave.
fn confined(root: &Path, path: &Path) -> bool {
    if root
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return false;
    }
    match (root.canonicalize(), path.canonicalize()) {
        (Ok(root), Ok(path)) => path.starts_with(root),
        _ => true,
    }
}

/// Lists the workspace directory `dir` (workspace-relative; empty = root), directories first then by
/// name, capped at [`MAX_FILE_LIST`].
///
/// # Errors
/// If `dir` escapes the workspace or cannot be read.
pub fn list(root: &Path, dir: &str) -> Result<Vec<FileEntry>, String> {
    let target = safe_join(root, dir).ok_or("that path is outside the workspace")?;
    // Make the root itself on first use so a fresh PC lists an empty folder instead of erroring.
    if dir.is_empty() {
        let _ = std::fs::create_dir_all(&target);
    }
    if !confined(root, &target) {
        return Err("that path is outside the workspace".into());
    }
    let mut entries: Vec<FileEntry> = Vec::new();
    for entry in std::fs::read_dir(&target).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        entries.push(FileEntry {
            name: entry.file_name().to_string_lossy().to_string(),
            is_dir: meta.is_dir(),
            bytes: if meta.is_dir() { 0 } else { meta.len() },
        });
        if entries.len() >= MAX_FILE_LIST {
            break;
        }
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    Ok(entries)
}

/// The absolute path of a workspace file to **download**, or `None` if it escapes the workspace or is
/// not an existing regular file.
#[must_use]
pub fn read_path(root: &Path, rel: &str) -> Option<PathBuf> {
    let path = safe_join(root, rel)?;
    (path.is_file() && confined(root, &path)).then_some(path)
}

/// The absolute path to **write** `name` into workspace directory `dir`, creating the directory. Returns
/// `None` if the destination escapes the workspace or `name` is not a plain file name.
#[must_use]
pub fn write_path(root: &Path, dir: &str, name: &str) -> Option<PathBuf> {
    if !is_plain_name(name) {
        return None;
    }
    std::fs::create_dir_all(root).ok()?;
    if root.symlink_metadata().ok()?.file_type().is_symlink() {
        return None;
    }
    let dir_path = safe_join(root, dir)?;
    std::fs::create_dir_all(&dir_path).ok()?;
    if !confined(root, &dir_path) {
        return None;
    }
    let dest = dir_path.join(name);
    // File::create follows an existing symlink; refuse one even when its parent is safe.
    if dest
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return None;
    }
    Some(dest)
}

/// Recursively lists **every file** in the workspace (directories excluded), each entry's `name`
/// carrying its workspace-relative path with `/` separators. Symlinks are skipped (never followed out
/// of the workspace). Capped at [`MAX_FILE_LIST`].
///
/// # Errors
/// If the workspace root cannot be read.
pub fn manifest(root: &Path) -> Result<Vec<FileEntry>, String> {
    let _ = std::fs::create_dir_all(root);
    if root
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err("the workspace root is a symlink".into());
    }
    let mut out = Vec::new();
    walk_files(root, root, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Depth-first walk collecting files (not directories), never following symlinks.
fn walk_files(root: &Path, dir: &Path, out: &mut Vec<FileEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILE_LIST {
            return;
        }
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        // symlink_metadata via file_type on DirEntry does not follow the link.
        let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        if is_symlink {
            continue; // never descend through or report a symlink
        }
        if meta.is_dir() {
            walk_files(root, &path, out);
        } else if meta.is_file()
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(FileEntry {
                name: rel.to_string_lossy().replace('\\', "/"),
                is_dir: false,
                bytes: meta.len(),
            });
        }
    }
}

/// Deletes one file from the workspace. Refuses a path that escapes the workspace, a missing file, or a
/// directory (only files are deletable here; use [`clear`] to wipe everything).
///
/// # Errors
/// If the path is outside the workspace, is not an existing file, or the OS refuses the delete.
pub fn delete(root: &Path, rel: &str) -> Result<(), String> {
    let path = safe_join(root, rel).ok_or("that path is outside the workspace")?;
    if !path.is_file() || !confined(root, &path) {
        return Err("no such file in the workspace".into());
    }
    std::fs::remove_file(&path).map_err(|e| e.to_string())
}

/// Wipes the whole workspace — every file and sub-directory inside `root`, leaving `root` itself. Never
/// follows a symlink out of the workspace (a symlink is removed as a link, its target untouched), so a
/// wipe can only ever delete inside the configured scope (AGENTS.md §5). Returns how many entries went.
///
/// # Errors
/// If the workspace cannot be read.
pub fn clear(root: &Path) -> Result<u32, String> {
    if root
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err("the workspace root is a symlink".into());
    }
    if !root.is_dir() {
        return Ok(0);
    }
    let mut removed = 0u32;
    remove_children(root, &mut removed)?;
    Ok(removed)
}

/// Removes everything inside `dir` (not `dir` itself), recursing into real sub-directories only and
/// removing symlinks as links. Confined by construction: it only ever touches paths under `dir`.
fn remove_children(dir: &Path, removed: &mut u32) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        if is_symlink {
            // Remove the link itself; never follow it. Try file then dir form (Windows dir-symlinks).
            let _ = std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir(&path));
            *removed += 1;
        } else if path.is_dir() {
            remove_children(&path, removed)?;
            let _ = std::fs::remove_dir(&path);
            *removed += 1;
        } else {
            let _ = std::fs::remove_file(&path);
            *removed += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cw-ws-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn safe_join_keeps_ordinary_paths_inside_the_workspace() {
        let root = root();
        assert_eq!(safe_join(&root, "hw.py"), Some(root.join("hw.py")));
        assert_eq!(
            safe_join(&root, "sub/dir/f"),
            Some(root.join("sub").join("dir").join("f"))
        );
        assert_eq!(safe_join(&root, ""), Some(root.clone()));
        assert_eq!(safe_join(&root, "./a"), Some(root.join("a")));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn safe_join_refuses_anything_that_escapes() {
        let root = root();
        assert_eq!(safe_join(&root, "../secret"), None);
        assert_eq!(safe_join(&root, "a/../../b"), None);
        assert_eq!(safe_join(&root, "/etc/passwd"), None);
        // A Windows drive-absolute path and a UNC prefix both carry a Prefix/RootDir component.
        assert_eq!(safe_join(&root, "C:\\Windows\\System32\\x"), None);
        assert_eq!(safe_join(&root, "\\\\server\\share\\x"), None);
        // Absurdly long paths are refused before any work.
        assert_eq!(safe_join(&root, &"a/".repeat(MAX_FILE_PATH)), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn plain_name_rejects_separators_and_dot_names() {
        assert!(is_plain_name("notes.txt"));
        assert!(!is_plain_name(""));
        assert!(!is_plain_name("."));
        assert!(!is_plain_name(".."));
        assert!(!is_plain_name("a/b"));
        assert!(!is_plain_name("a\\b"));
    }

    #[test]
    fn write_path_and_read_path_stay_in_the_workspace() {
        let root = root();
        // A traversal name never yields a writable path.
        assert_eq!(write_path(&root, "..", "x"), None);
        assert_eq!(write_path(&root, "sub", "../x"), None);
        // A legitimate write path is inside the workspace and its directory now exists.
        let dest = write_path(&root, "sub", "notes.txt").expect("write path");
        assert!(dest.starts_with(&root));
        assert!(dest.parent().unwrap().is_dir());
        std::fs::write(&dest, b"hi").unwrap();
        // read_path finds the file we just wrote, and refuses a directory or a missing file.
        assert_eq!(read_path(&root, "sub/notes.txt"), Some(dest));
        assert_eq!(
            read_path(&root, "sub"),
            None,
            "a directory is not downloadable"
        );
        assert_eq!(read_path(&root, "missing.txt"), None);
        assert_eq!(read_path(&root, "../notes.txt"), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn write_path_rejects_an_existing_file_symlink() {
        let root = root();
        let outside = root.with_extension("outside");
        std::fs::write(&outside, b"keep").unwrap();
        if std::os::windows::fs::symlink_file(&outside, root.join("link.txt")).is_ok() {
            assert_eq!(write_path(&root, "", "link.txt"), None);
            assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
        }
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&outside);
    }

    #[cfg(windows)]
    #[test]
    fn a_symlink_workspace_root_cannot_redirect_transfers() {
        let parent = root();
        let outside = parent.join("outside");
        let link = parent.join("workspace-link");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), b"keep").unwrap();
        if std::os::windows::fs::symlink_dir(&outside, &link).is_ok() {
            assert_eq!(write_path(&link, "sub", "new.txt"), None);
            assert_eq!(read_path(&link, "secret.txt"), None);
            assert!(manifest(&link).is_err());
            assert!(!outside.join("sub").exists());
        }
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn delete_removes_a_file_and_refuses_escaping_or_missing_paths() {
        let root = root();
        std::fs::write(root.join("keep.txt"), b"k").unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("x.py"), b"x").unwrap();
        assert!(delete(&root, "sub/x.py").is_ok());
        assert!(!root.join("sub").join("x.py").exists());
        assert!(
            root.join("keep.txt").exists(),
            "unrelated files are untouched"
        );
        assert!(delete(&root, "../evil").is_err(), "escaping is refused");
        assert!(delete(&root, "missing").is_err(), "missing file is refused");
        assert!(
            delete(&root, "sub").is_err(),
            "a directory is not a deletable file"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn clear_wipes_the_workspace_but_nothing_outside_it() {
        let parent = root();
        let root = parent.join("ws");
        std::fs::create_dir_all(root.join("a").join("b")).unwrap();
        std::fs::write(root.join("top.txt"), b"t").unwrap();
        std::fs::write(root.join("a").join("mid.txt"), b"m").unwrap();
        std::fs::write(root.join("a").join("b").join("deep.txt"), b"d").unwrap();
        // A file that lives OUTSIDE the workspace, next to it, must survive the wipe.
        std::fs::write(parent.join("outside.txt"), b"safe").unwrap();

        let removed = clear(&root).expect("clear");
        assert!(removed >= 5, "removed files and dirs, got {removed}");
        assert!(root.is_dir(), "the workspace root itself remains");
        assert_eq!(
            std::fs::read_dir(&root).unwrap().count(),
            0,
            "workspace is empty"
        );
        assert!(
            parent.join("outside.txt").exists(),
            "files outside the workspace are never touched"
        );
        let _ = std::fs::remove_dir_all(&parent);
    }

    #[test]
    fn manifest_lists_every_file_with_its_relative_path() {
        let root = root();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("a.txt"), b"aa").unwrap();
        std::fs::write(root.join("sub").join("hw.py"), b"print").unwrap();
        let files = manifest(&root).expect("manifest");
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"a.txt"));
        assert!(
            names.contains(&"sub/hw.py"),
            "paths use / and are workspace-relative"
        );
        assert!(files.iter().all(|f| !f.is_dir), "manifest is files only");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn list_returns_entries_directories_first() {
        let root = root();
        std::fs::create_dir_all(root.join("zsub")).unwrap();
        std::fs::write(root.join("a.txt"), b"aa").unwrap();
        let entries = list(&root, "").expect("list");
        assert_eq!(entries[0].name, "zsub");
        assert!(entries[0].is_dir);
        assert!(entries.iter().any(|e| e.name == "a.txt" && e.bytes == 2));
        assert!(list(&root, "../..").is_err(), "escaping is refused");
        let _ = std::fs::remove_dir_all(&root);
    }
}
