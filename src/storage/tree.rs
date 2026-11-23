//! The scan result tree and its selection model.
//!
//! Selection lives on the nodes. A folder whose children have been loaded derives its state from
//! them, so the user can open a folder, untick a few entries and delete the rest. A folder whose
//! children are not loaded uses its own flag.

use std::path::{Path, PathBuf};

use super::categories::{CategoryDef, Safety};
use super::safety::is_protected;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Off,
    On,
    Mixed,
}

fn combine(states: impl Iterator<Item = Check>) -> Check {
    let (mut any_on, mut any_off) = (false, false);
    for state in states {
        match state {
            Check::On => any_on = true,
            Check::Off => any_off = true,
            Check::Mixed => return Check::Mixed,
        }
        if any_on && any_off {
            return Check::Mixed;
        }
    }
    if any_on { Check::On } else { Check::Off }
}

#[derive(Debug)]
pub enum Children {
    NotLoaded,
    Loading,
    Loaded(Vec<Node>),
    Failed(String),
}

#[derive(Debug)]
pub struct Node {
    pub name: String,
    pub path: PathBuf,
    /// Space on disk in bytes (allocated blocks, hard links counted once).
    pub size: u64,
    pub is_dir: bool,
    pub safety: Safety,
    /// The node itself can't be deleted (see `safety::is_protected`); its contents may be.
    pub protected: bool,
    pub selected: bool,
    pub expanded: bool,
    pub children: Children,
}

/// One entry the user chose to delete.
#[derive(Clone, Debug)]
pub struct Selected {
    pub path: PathBuf,
    pub size: u64,
    pub safety: Safety,
}

impl Node {
    pub fn new(name: String, path: PathBuf, size: u64, is_dir: bool, safety: Safety) -> Self {
        Self {
            name,
            protected: is_protected(&path),
            path,
            size,
            is_dir,
            safety,
            selected: false,
            expanded: false,
            children: if is_dir {
                Children::NotLoaded
            } else {
                Children::Loaded(Vec::new())
            },
        }
    }

    fn loaded_children(&self) -> Option<&[Node]> {
        match &self.children {
            Children::Loaded(kids) if !kids.is_empty() => Some(kids),
            _ => None,
        }
    }

    /// Whether this row has a checkbox the user can tick (directly or through its children).
    pub fn selectable(&self) -> bool {
        !self.protected || self.loaded_children().is_some()
    }

    pub fn check(&self) -> Check {
        match self.loaded_children() {
            Some(kids) => combine(kids.iter().filter(|k| k.selectable()).map(Node::check)),
            None if self.selected => Check::On,
            None => Check::Off,
        }
    }

    pub fn set_selected(&mut self, on: bool) {
        if !self.protected {
            self.selected = on;
        }
        if let Children::Loaded(kids) = &mut self.children {
            for kid in kids {
                kid.set_selected(on);
            }
        }
    }

    pub fn selected_size(&self) -> u64 {
        match self.check() {
            Check::Off => 0,
            Check::On if !self.protected => self.size,
            _ => self
                .loaded_children()
                .map_or(0, |kids| kids.iter().map(Node::selected_size).sum()),
        }
    }

    /// Appends the minimal set of paths that deletes exactly what is ticked.
    pub fn collect_selected(&self, out: &mut Vec<Selected>) {
        match self.check() {
            Check::Off => {}
            Check::On if !self.protected => out.push(Selected {
                path: self.path.clone(),
                size: self.size,
                safety: self.safety,
            }),
            _ => {
                for kid in self.loaded_children().unwrap_or_default() {
                    kid.collect_selected(out);
                }
            }
        }
    }

    pub fn find_mut(&mut self, path: &Path) -> Option<&mut Node> {
        if self.path == path {
            return Some(self);
        }
        if !path.starts_with(&self.path) {
            return None;
        }
        match &mut self.children {
            Children::Loaded(kids) => kids.iter_mut().find_map(|k| k.find_mut(path)),
            _ => None,
        }
    }
}

/// Removes the node at `path` from `nodes`, shrinking every ancestor by its size.
/// Returns the removed size.
pub fn remove_path(nodes: &mut Vec<Node>, path: &Path) -> Option<u64> {
    if let Some(index) = nodes.iter().position(|n| n.path == path) {
        return Some(nodes.remove(index).size);
    }
    for node in nodes.iter_mut() {
        if !path.starts_with(&node.path) {
            continue;
        }
        if let Children::Loaded(kids) = &mut node.children
            && let Some(removed) = remove_path(kids, path)
        {
            node.size = node.size.saturating_sub(removed);
            return Some(removed);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanState {
    Idle,
    Queued,
    Scanning,
    Done,
}

pub struct Category {
    pub def: CategoryDef,
    pub items: Vec<Node>,
    pub expanded: bool,
    pub state: ScanState,
    /// Entries that couldn't be read, usually for lack of Full Disk Access.
    pub unreadable: u64,
    /// True when none of the category's locations exist on this Mac.
    pub missing: bool,
}

impl Category {
    pub fn new(def: CategoryDef) -> Self {
        Self {
            def,
            items: Vec::new(),
            expanded: false,
            state: ScanState::Idle,
            unreadable: 0,
            missing: false,
        }
    }

    pub fn total_size(&self) -> u64 {
        self.items.iter().map(|n| n.size).sum()
    }

    pub fn check(&self) -> Check {
        combine(
            self.items
                .iter()
                .filter(|n| n.selectable())
                .map(Node::check),
        )
    }

    pub fn set_selected(&mut self, on: bool) {
        for item in &mut self.items {
            item.set_selected(on);
        }
    }

    pub fn selected_size(&self) -> u64 {
        self.items.iter().map(Node::selected_size).sum()
    }

    pub fn collect_selected(&self, out: &mut Vec<Selected>) {
        for item in &self.items {
            item.collect_selected(out);
        }
    }

    pub fn find_mut(&mut self, path: &Path) -> Option<&mut Node> {
        self.items.iter_mut().find_map(|n| n.find_mut(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::home_dir;

    fn dir(name: &str, size: u64, kids: Vec<Node>) -> Node {
        let path = home_dir().join("Library/Caches/test").join(name);
        let mut node = Node::new(name.into(), path, size, true, Safety::Safe);
        node.children = Children::Loaded(kids);
        node
    }

    fn file(parent: &str, name: &str, size: u64) -> Node {
        let path = home_dir()
            .join("Library/Caches/test")
            .join(parent)
            .join(name);
        Node::new(name.into(), path, size, false, Safety::Safe)
    }

    #[test]
    fn parent_state_follows_children() {
        let mut root = dir("app", 30, vec![file("app", "a", 10), file("app", "b", 20)]);
        assert_eq!(root.check(), Check::Off);

        root.set_selected(true);
        assert_eq!(root.check(), Check::On);
        assert_eq!(root.selected_size(), 30);

        if let Children::Loaded(kids) = &mut root.children {
            kids[0].set_selected(false);
        }
        assert_eq!(root.check(), Check::Mixed);
        assert_eq!(root.selected_size(), 20);

        let mut picked = Vec::new();
        root.collect_selected(&mut picked);
        assert_eq!(picked.len(), 1);
        assert!(picked[0].path.ends_with("app/b"));
    }

    #[test]
    fn fully_selected_folder_is_deleted_whole() {
        let mut root = dir("app", 30, vec![file("app", "a", 10), file("app", "b", 20)]);
        root.set_selected(true);
        let mut picked = Vec::new();
        root.collect_selected(&mut picked);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].path, root.path);
    }

    #[test]
    fn protected_node_is_never_collected_itself() {
        let home = home_dir();
        let mut docs = Node::new(
            "Documents".into(),
            home.join("Documents"),
            30,
            true,
            Safety::Review,
        );
        assert!(docs.protected);
        assert!(!docs.selectable());
        docs.set_selected(true);
        assert_eq!(docs.check(), Check::Off);

        let kid = Node::new(
            "old".into(),
            home.join("Documents/old"),
            30,
            true,
            Safety::Review,
        );
        docs.children = Children::Loaded(vec![kid]);
        docs.set_selected(true);
        assert_eq!(docs.check(), Check::On);
        let mut picked = Vec::new();
        docs.collect_selected(&mut picked);
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].path, home.join("Documents/old"));
    }

    #[test]
    fn removing_a_child_shrinks_ancestors() {
        let mut nodes = vec![dir(
            "app",
            30,
            vec![file("app", "a", 10), file("app", "b", 20)],
        )];
        let target = nodes[0].path.join("a");
        assert_eq!(remove_path(&mut nodes, &target), Some(10));
        assert_eq!(nodes[0].size, 20);
        assert!(remove_path(&mut nodes, &target).is_none());
    }
}
