//! Finding and removing reclaimable files: caches, logs, developer artifacts, the Trash, and any
//! folder the user adds.

pub mod categories;
pub mod cleaner;
pub mod safety;
pub mod scanner;
pub mod tree;
