//! Deleting the selected items on a background thread, either to the Trash or permanently.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;

use eframe::egui;
use trash::TrashContext;
use trash::macos::{DeleteMethod, TrashContextExtMacos};

use super::categories::Safety;
use super::safety::check_deletable;
use crate::util::display_path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeleteMode {
    Trash,
    Permanent,
}

#[derive(Clone, Debug)]
pub struct CleanTarget {
    pub path: PathBuf,
    pub size: u64,
    pub category: usize,
    pub safety: Safety,
    /// Deleted permanently even in Trash mode (items already in the Trash).
    pub force_permanent: bool,
}

#[derive(Default, Debug)]
pub struct CleanReport {
    pub removed: Vec<CleanTarget>,
    pub failed: Vec<(PathBuf, String)>,
    /// Bytes released immediately.
    pub freed: u64,
    /// Bytes moved to the Trash, released once it is emptied.
    pub trashed: u64,
}

pub enum CleanEvent {
    Progress {
        done: usize,
        total: usize,
        current: String,
    },
    Finished(CleanReport),
}

pub fn start_clean(
    targets: Vec<CleanTarget>,
    mode: DeleteMode,
    allowed_roots: Vec<PathBuf>,
    tx: Sender<CleanEvent>,
    ctx: egui::Context,
) {
    thread::spawn(move || {
        let mut trash = TrashContext::default();
        trash.set_delete_method(DeleteMethod::NsFileManager);
        let mut report = CleanReport::default();
        let total = targets.len();
        for (done, target) in targets.into_iter().enumerate() {
            let _ = tx.send(CleanEvent::Progress {
                done,
                total,
                current: display_path(&target.path),
            });
            ctx.request_repaint();

            let permanent = mode == DeleteMode::Permanent || target.force_permanent;
            let result = check_deletable(&target.path, &allowed_roots).and_then(|()| {
                if permanent {
                    remove_permanently(&target.path).map_err(|e| e.to_string())
                } else {
                    trash.delete(&target.path).map_err(|e| e.to_string())
                }
            });
            match result {
                Ok(()) => {
                    if permanent {
                        report.freed += target.size;
                    } else {
                        report.trashed += target.size;
                    }
                    report.removed.push(target);
                }
                Err(e) => report.failed.push((target.path, e)),
            }
        }
        let _ = tx.send(CleanEvent::Finished(report));
        ctx.request_repaint();
    });
}

fn remove_permanently(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return fs::remove_file(path);
    }
    match fs::remove_dir_all(path) {
        // Some tools (Go, Bazel, ...) leave read-only folders behind; their contents can only
        // be removed once the folder is writable again.
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            make_folders_writable(path);
            fs::remove_dir_all(path)
        }
        other => other,
    }
}

fn make_folders_writable(path: &Path) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if !meta.is_dir() {
        return;
    }
    let mode = meta.permissions().mode();
    if mode & 0o700 != 0o700 {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode | 0o700));
    }
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            make_folders_writable(&entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_read_only_folders() {
        let dir = std::env::temp_dir().join(format!("maccleaner-ro-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("locked/inner")).unwrap();
        fs::write(dir.join("locked/inner/file"), b"x").unwrap();
        fs::set_permissions(dir.join("locked/inner"), fs::Permissions::from_mode(0o555)).unwrap();
        fs::set_permissions(dir.join("locked"), fs::Permissions::from_mode(0o555)).unwrap();

        remove_permanently(&dir.join("locked")).unwrap();
        assert!(!dir.join("locked").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cleans_only_inside_allowed_roots() {
        let caches = crate::util::home_dir().join("Library/Caches");
        let root = caches.join(format!("maccleaner-selftest-{}", std::process::id()));
        let other = caches.join(format!("maccleaner-selftest-other-{}", std::process::id()));
        fs::create_dir_all(root.join("keep")).unwrap();
        fs::create_dir_all(root.join("remove/nested")).unwrap();
        fs::write(root.join("remove/nested/file"), vec![0u8; 4096]).unwrap();
        fs::create_dir_all(&other).unwrap();
        fs::write(other.join("file"), b"x").unwrap();

        let target = |path: PathBuf, size| CleanTarget {
            path,
            size,
            category: 0,
            safety: Safety::Safe,
            force_permanent: false,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        start_clean(
            vec![
                target(root.join("remove"), 4096),
                target(other.join("file"), 1),
            ],
            DeleteMode::Permanent,
            vec![root.clone()],
            tx,
            egui::Context::default(),
        );
        let report = loop {
            if let CleanEvent::Finished(report) = rx.recv().unwrap() {
                break report;
            }
        };

        assert_eq!(report.removed.len(), 1);
        assert_eq!(report.freed, 4096);
        assert_eq!(report.failed.len(), 1);
        assert!(!root.join("remove").exists());
        assert!(root.join("keep").exists());
        assert!(other.join("file").exists());
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&other).unwrap();
    }
}
