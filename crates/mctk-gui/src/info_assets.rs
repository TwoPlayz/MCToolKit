//! The pictures beside some tags (see `info`), kept inside the app and written out to a folder on
//! first use, because they are drawn from files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

mod files {
    include!(concat!(env!("OUT_DIR"), "/info_assets.rs"));
}

/// Writes the pictures into `dir` (once per folder per run), replacing any that differ.
pub fn install(dir: &Path) {
    static DONE: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    let mut done = DONE.lock().expect("info install");
    if !done.get_or_insert_with(HashSet::new).insert(dir.to_path_buf()) {
        return;
    }
    for (rel, bytes) in files::FILES {
        let p = dir.join(rel);
        if std::fs::read(&p).ok().as_deref() == Some(*bytes) {
            continue;
        }
        if let Some(parent) = p.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&p, bytes);
    }
}