//! Tidying what the app leaves behind: working copies and conversion results in the working
//! folder that nobody came back for, and the temporary folders of runs that did not finish (or
//! of older versions of the app).

use std::path::Path;
use std::time::{Duration, SystemTime};

/// How long a working copy or an unsaved conversion result is kept.
pub const KEEP: Duration = Duration::from_secs(7 * 24 * 3600);

/// The most recent change anywhere in `path` (a file, or a folder and what is in it, a few
/// levels down).
pub fn newest(path: &Path) -> Option<SystemTime> {
    fn walk(path: &Path, depth: u32, budget: &mut u32, best: &mut Option<SystemTime>) {
        if let Ok(t) = std::fs::metadata(path).and_then(|m| m.modified()) {
            if best.is_none_or(|b| t > b) {
                *best = Some(t);
            }
        }
        if depth == 0 || !path.is_dir() {
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else { return };
        for e in entries.flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            walk(&e.path(), depth - 1, budget, best);
        }
    }
    let (mut best, mut budget) = (None, 20_000);
    walk(path, 6, &mut budget, &mut best);
    best
}

/// Removes the working copies and conversion results under `working` that have not been touched
/// for `older_than`, and the folder of the old information files. Gives how many folders went.
pub fn working_folder(working: &Path, older_than: Duration) -> usize {
    let mut gone = 0;
    for app in ["CONVERTER", "PRUNER", "EDITOR"] {
        let Ok(entries) = std::fs::read_dir(working.join(app).join("workspaces")) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            let old = newest(&path).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > older_than);
            if path.is_dir() && old && std::fs::remove_dir_all(&path).is_ok() {
                gone += 1;
            }
        }
    }
    // Pictures and notes an older version kept here; nothing reads them now.
    if std::fs::remove_dir_all(working.join("info")).is_ok() {
        gone += 1;
    }
    gone
}

/// Removes the temporary folders (`mctk-...`) left in the system's by runs that did not finish (a conversion scan or a route
/// check, the unprotect step, an older version's script host), once they are a day old. (The pack the snapshot tests share stays.)
pub fn temp_folders() -> usize {
    let mut gone = 0;
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else { return 0 };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let ours = name.starts_with("mctk-") && name != "mctk-snapshot-pack";
        // (Not one a run going on now made: only those untouched for a day.)
        let stale = newest(&e.path()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(24 * 3600));
        if ours && stale && e.path().is_dir() && std::fs::remove_dir_all(e.path()).is_ok() {
            gone += 1;
        }
    }
    gone
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_copies_go_and_new_ones_stay() {
        let base = std::env::temp_dir().join(format!("mctk-cleanup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let ws = base.join("CONVERTER").join("workspaces");
        std::fs::create_dir_all(ws.join("a 1")).unwrap();
        std::fs::write(ws.join("a 1").join("level.dat"), b"x").unwrap();
        std::fs::create_dir_all(base.join("info")).unwrap();
        // Nothing is old yet.
        assert_eq!(working_folder(&base, Duration::from_secs(3600)), 1, "only the old information folder goes");
        assert!(ws.join("a 1").exists());
        // Everything is older than none at all.
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(working_folder(&base, Duration::ZERO), 1);
        assert!(!ws.join("a 1").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn newest_sees_a_change_inside() {
        let base = std::env::temp_dir().join(format!("mctk-newest-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("db")).unwrap();
        let before = newest(&base).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::write(base.join("db").join("000001.log"), b"x").unwrap();
        assert!(newest(&base).unwrap() > before);
        let _ = std::fs::remove_dir_all(&base);
    }
}