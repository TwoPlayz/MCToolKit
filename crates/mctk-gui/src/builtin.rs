//! UMT's NBT editor extensions, built into the app (each is BSD 2-Clause; the licences are
//! in `assets/extensions/*/LICENSE`). They are written out to the extensions folder on
//! first use, because scripts and their images are loaded from files.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

mod files {
    include!(concat!(env!("OUT_DIR"), "/builtin_extensions.rs"));
}

/// Folder names of the built-in extensions.
fn ids() -> HashSet<&'static str> {
    files::FILES
        .iter()
        .filter_map(|(rel, _)| rel.split_once('/').map(|(id, _)| id))
        .collect()
}

/// Whether `id` names one of the built-in extensions (folder names compare without regard to
/// case, as Windows treats them).
pub fn is_builtin(id: &str) -> bool {
    ids().iter().any(|b| b.eq_ignore_ascii_case(id))
}

/// Writes the built-in extensions into `dir` (once per folder per run), replacing any file
/// that differs.
fn install(dir: &Path) {
    static DONE: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);
    let mut done = DONE.lock().expect("builtin install");
    if !done
        .get_or_insert_with(HashSet::new)
        .insert(dir.to_path_buf())
    {
        return;
    }
    write_all(dir);
}

/// Puts the built-in extensions back as shipped (after an import with the same name).
pub fn restore(dir: &Path) {
    write_all(dir);
}

fn write_all(dir: &Path) {
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

/// Every installed extension: the built-in ones first, then those the user imported.
pub fn list(dir: &Path) -> Vec<mctk_lua::Extension> {
    install(dir);
    let mut all: Vec<mctk_lua::Extension> = mctk_lua::list(dir).into_iter().flatten().collect();
    all.sort_by_key(|e| !is_builtin(&e.id));
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_names_clash_regardless_of_case() {
        assert!(is_builtin("PlayerInfo"));
        assert!(is_builtin("playerinfo"));
        assert!(!is_builtin("my-own-extension"));
    }

    #[test]
    fn imported_extensions_are_listed_after_the_built_in_ones() {
        let dir = std::env::temp_dir().join(format!("mctk-ext-test-{}", std::process::id()));
        let src = dir.join("src").join("hello");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("manifest.json"),
            r#"{"name":"Hello","description":"d","author":"a","version":"1.0.0","editions":["JAVA"],"extension_types":["WORLD_TOOL"]}"#,
        )
        .unwrap();
        std::fs::write(src.join("main.lua"), "function run(ctx) return 'hi' end").unwrap();
        let ext_dir = dir.join("installed");
        std::fs::create_dir_all(&ext_dir).unwrap();
        mctk_lua::install_folder(&src, &ext_dir).unwrap();
        let all = list(&ext_dir);
        assert!(all.iter().any(|e| e.id == "hello"));
        let first_imported = all.iter().position(|e| !is_builtin(&e.id)).unwrap();
        assert!(all[..first_imported].iter().all(|e| is_builtin(&e.id)));
        mctk_lua::remove(&ext_dir, "hello").unwrap();
        assert!(!list(&ext_dir).iter().any(|e| e.id == "hello"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}