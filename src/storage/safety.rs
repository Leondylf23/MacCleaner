//! Guard rails that keep MacCleaner from deleting anything the system or the user's account
//! depends on, no matter what is selected.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::util::home_dir;

/// System locations that are never deleted from.
const SYSTEM_PREFIXES: &[&str] = &[
    "/System", "/usr", "/bin", "/sbin", "/private", "/dev", "/cores", "/Library", "/opt", "/etc",
    "/var", "/tmp",
];

/// Folders inside the home folder whose contents are never deleted.
const HOME_PREFIXES: &[&str] = &[
    "Library/Keychains",
    "Library/Mail",
    "Library/Messages",
    "Library/Accounts",
    "Library/Mobile Documents",
];

/// True for paths that must never be deleted themselves: system locations, the home folder and
/// its top-level folders (Documents, Library, ...), direct children of `~/Library`, and volume
/// mount points. Their contents may still be deletable.
pub fn is_protected(path: &Path) -> bool {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return true;
    }
    // "/" is one component, "/Applications" two.
    let depth = path.components().count();
    if depth <= 2 || (path.starts_with("/Volumes") && depth <= 3) {
        return true;
    }
    if SYSTEM_PREFIXES.iter().any(|p| path.starts_with(p)) {
        return true;
    }
    let home = home_dir();
    let parent = path.parent();
    if path == home
        || parent == Some(home)
        || parent == Some(Path::new("/Users"))
        || parent == Some(home.join("Library").as_path())
    {
        return true;
    }
    HOME_PREFIXES.iter().any(|p| path.starts_with(home.join(p)))
}

/// Checks a folder the user wants to scan. Returns its canonical path.
pub fn validate_custom_root(path: &Path) -> Result<PathBuf, String> {
    let canonical =
        fs::canonicalize(path).map_err(|e| format!("Can't open {}: {e}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!("{} is not a folder", canonical.display()));
    }
    if canonical == Path::new("/") || SYSTEM_PREFIXES.iter().any(|p| canonical.starts_with(p)) {
        return Err(format!(
            "{} is a system location and can't be cleaned",
            canonical.display()
        ));
    }
    Ok(canonical)
}

/// Final check right before a deletion: the path must sit strictly inside one of the scanned
/// roots and must not be protected.
pub fn check_deletable(path: &Path, allowed_roots: &[PathBuf]) -> Result<(), String> {
    if is_protected(path) {
        return Err("protected location".to_owned());
    }
    if !allowed_roots
        .iter()
        .any(|root| path.starts_with(root) && path != root)
    {
        return Err("outside the scanned folders".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protects_system_and_account_folders() {
        let home = home_dir();
        for path in [
            PathBuf::from("/"),
            PathBuf::from("/Applications"),
            PathBuf::from("/System/Library/Caches/x"),
            PathBuf::from("/Library/Caches/x"),
            PathBuf::from("/Volumes/External"),
            home.to_path_buf(),
            home.join("Documents"),
            home.join("Library"),
            home.join("Library/Caches"),
            home.join("Library/Keychains/login.keychain-db"),
            home.join("Library/Caches/../Preferences"),
        ] {
            assert!(
                is_protected(&path),
                "{} should be protected",
                path.display()
            );
        }
    }

    #[test]
    fn allows_items_inside_cleanable_folders() {
        let home = home_dir();
        for path in [
            home.join("Library/Caches/com.example.app"),
            home.join(".Trash/old.zip"),
            home.join("Documents/Projects/build"),
            PathBuf::from("/Applications/Unused.app"),
        ] {
            assert!(
                !is_protected(&path),
                "{} should be deletable",
                path.display()
            );
        }
    }

    #[test]
    fn deletion_requires_path_inside_a_root() {
        let caches = home_dir().join("Library/Caches");
        let roots = vec![caches.clone()];
        assert!(check_deletable(&caches.join("com.example"), &roots).is_ok());
        assert!(check_deletable(&caches, &roots).is_err());
        assert!(check_deletable(&home_dir().join("Downloads/file.zip"), &roots).is_err());
    }

    #[test]
    fn rejects_system_custom_roots() {
        assert!(validate_custom_root(Path::new("/")).is_err());
        assert!(validate_custom_root(Path::new("/System")).is_err());
        assert!(validate_custom_root(Path::new("/tmp")).is_err());
    }
}
