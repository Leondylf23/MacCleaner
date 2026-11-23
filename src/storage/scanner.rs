//! Background scanning: sizing every cleanable item, and lazily listing a folder's contents when
//! the user expands it.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use rayon::prelude::*;

use super::categories::{CategoryDef, RootMode, Safety};
use super::tree::Node;

/// Live counters the UI reads while a scan runs.
#[derive(Default)]
pub struct ScanProgress {
    pub files: AtomicU64,
    pub bytes: AtomicU64,
    pub cancel: AtomicBool,
    pub current: Mutex<String>,
}

impl ScanProgress {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

pub enum ScanEvent {
    CategoryStarted(usize),
    CategoryDone {
        index: usize,
        items: Vec<Node>,
        unreadable: u64,
        missing: bool,
    },
    ScanFinished {
        cancelled: bool,
        elapsed: Duration,
    },
    ChildrenLoaded {
        path: PathBuf,
        result: Result<Vec<Node>, String>,
    },
}

/// Walks a tree and adds up allocated space, counting hard-linked files once and staying on the
/// starting volume.
struct Sizer<'a> {
    device: u64,
    seen_inodes: Mutex<HashSet<(u64, u64)>>,
    unreadable: AtomicU64,
    progress: &'a ScanProgress,
}

impl<'a> Sizer<'a> {
    fn new(root: &Path, progress: &'a ScanProgress) -> Self {
        Self {
            device: fs::symlink_metadata(root).map(|m| m.dev()).unwrap_or(0),
            seen_inodes: Mutex::default(),
            unreadable: AtomicU64::new(0),
            progress,
        }
    }

    fn size_of(&self, path: &Path) -> u64 {
        if self.progress.cancelled() {
            return 0;
        }
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(_) => {
                self.unreadable.fetch_add(1, Ordering::Relaxed);
                return 0;
            }
        };
        if meta.dev() != self.device {
            return 0;
        }
        let own = meta.blocks() * 512;
        if !meta.is_dir() {
            if meta.nlink() > 1
                && !self
                    .seen_inodes
                    .lock()
                    .unwrap()
                    .insert((meta.dev(), meta.ino()))
            {
                return 0;
            }
            self.progress.files.fetch_add(1, Ordering::Relaxed);
            self.progress.bytes.fetch_add(own, Ordering::Relaxed);
            return own;
        }
        let entries: Vec<PathBuf> = match fs::read_dir(path) {
            Ok(entries) => entries.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => {
                self.unreadable.fetch_add(1, Ordering::Relaxed);
                return own;
            }
        };
        own + entries.par_iter().map(|p| self.size_of(p)).sum::<u64>()
    }
}

struct Entry {
    name: String,
    path: PathBuf,
    safety: Safety,
}

fn entry_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Sizes entries in parallel and returns them as nodes, largest first.
fn size_entries(entries: Vec<Entry>, sizer: &Sizer, progress: &ScanProgress) -> Vec<Node> {
    let mut nodes: Vec<Node> = entries
        .into_par_iter()
        .map(|entry| {
            if let Ok(mut current) = progress.current.try_lock() {
                current.clone_from(&entry.name);
            }
            let is_dir = fs::symlink_metadata(&entry.path).is_ok_and(|m| m.is_dir());
            let size = sizer.size_of(&entry.path);
            Node::new(entry.name, entry.path, size, is_dir, entry.safety)
        })
        .collect();
    nodes.sort_by(|a, b| b.size.cmp(&a.size).then_with(|| a.name.cmp(&b.name)));
    nodes
}

fn scan_category(def: &CategoryDef, progress: &ScanProgress) -> (Vec<Node>, u64, bool) {
    let mut entries = Vec::new();
    let mut unreadable = 0;
    let mut any_exists = false;
    for location in &def.locations {
        match location.mode {
            RootMode::Whole => {
                if fs::symlink_metadata(&location.path).is_ok() {
                    any_exists = true;
                    entries.push(Entry {
                        name: location
                            .label
                            .map_or_else(|| entry_name(&location.path), str::to_owned),
                        path: location.path.clone(),
                        safety: location.safety,
                    });
                }
            }
            RootMode::Children => match fs::read_dir(&location.path) {
                Ok(dir) => {
                    any_exists = true;
                    for entry in dir.flatten() {
                        let path = entry.path();
                        // Finder's folder metadata isn't worth listing.
                        if path
                            .file_name()
                            .is_some_and(|n| n == ".DS_Store" || n == ".localized")
                        {
                            continue;
                        }
                        entries.push(Entry {
                            name: entry_name(&path),
                            path,
                            safety: location.safety,
                        });
                    }
                }
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(_) => {
                    any_exists = true;
                    unreadable += 1;
                }
            },
        }
    }

    let root = def
        .locations
        .iter()
        .map(|l| l.path.as_path())
        .find(|p| p.exists())
        .unwrap_or(crate::util::home_dir());
    let sizer = Sizer::new(root, progress);
    let nodes = size_entries(entries, &sizer, progress);
    unreadable += sizer.unreadable.load(Ordering::Relaxed);
    (nodes, unreadable, !any_exists)
}

/// Scans the given categories one after another on a background thread.
pub fn start_scan(
    categories: Vec<(usize, CategoryDef)>,
    tx: Sender<ScanEvent>,
    ctx: egui::Context,
) -> Arc<ScanProgress> {
    let progress = Arc::new(ScanProgress::default());
    let shared = Arc::clone(&progress);
    thread::spawn(move || {
        let started = Instant::now();
        for (index, def) in categories {
            if shared.cancelled() {
                break;
            }
            let _ = tx.send(ScanEvent::CategoryStarted(index));
            let (items, unreadable, missing) = scan_category(&def, &shared);
            let _ = tx.send(ScanEvent::CategoryDone {
                index,
                items,
                unreadable,
                missing,
            });
            ctx.request_repaint();
        }
        let _ = tx.send(ScanEvent::ScanFinished {
            cancelled: shared.cancelled(),
            elapsed: started.elapsed(),
        });
        ctx.request_repaint();
    });
    progress
}

/// Lists and sizes the contents of `path` on a background thread.
pub fn load_children(path: PathBuf, safety: Safety, tx: Sender<ScanEvent>, ctx: egui::Context) {
    thread::spawn(move || {
        let progress = ScanProgress::default();
        let result = fs::read_dir(&path).map_err(|e| e.to_string()).map(|dir| {
            let entries = dir
                .flatten()
                .map(|e| {
                    let child = e.path();
                    Entry {
                        name: entry_name(&child),
                        path: child,
                        safety,
                    }
                })
                .collect();
            size_entries(entries, &Sizer::new(&path, &progress), &progress)
        });
        let _ = tx.send(ScanEvent::ChildrenLoaded { path, result });
        ctx.request_repaint();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_nested_folders_and_counts_hard_links_once() {
        let dir = std::env::temp_dir().join(format!("maccleaner-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("a/one.bin"), vec![1u8; 64 * 1024]).unwrap();
        fs::write(dir.join("a/b/two.bin"), vec![2u8; 64 * 1024]).unwrap();
        fs::hard_link(dir.join("a/one.bin"), dir.join("a/one-link.bin")).unwrap();

        let progress = ScanProgress::default();
        let sizer = Sizer::new(&dir, &progress);
        let total = sizer.size_of(&dir);
        assert!(total >= 128 * 1024, "total was {total}");
        assert!(total < 192 * 1024, "hard link counted twice: {total}");
        assert_eq!(progress.files.load(Ordering::Relaxed), 2);

        fs::remove_dir_all(&dir).unwrap();
    }
}
