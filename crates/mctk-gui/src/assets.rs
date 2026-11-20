//! The game's assets (textures, models, blockstates, item definitions...) for the latest
//! release, from mcasset.cloud: each file downloaded the first time it is wanted and kept in
//! MCToolKit's own resource pack folder, laid out as a resource pack (`assets/minecraft/...`),
//! so it is read from disk after that. When a newer release comes out the kept files are
//! cleared and fetched again as they are wanted.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

const BASE: &str = "https://assets.mcasset.cloud/latest";

struct State {
    /// The folder files are kept in (MCToolKit's own resource pack).
    root: Option<PathBuf>,
    /// The root the version was checked for.
    checked: Option<PathBuf>,
    /// Files the server does not have (so they are not asked for again).
    missing: HashSet<String>,
    /// The server could not be reached: only what is kept is used.
    offline: bool,
    /// Each folder's files, from its listing (`_list.json`, kept beside them), so a file the
    /// game lacks is known without asking for it.
    lists: std::collections::HashMap<String, Option<std::sync::Arc<HashSet<String>>>>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| {
    // (Tests share one folder, whichever runs first.)
    let root = if cfg!(test) { Some(std::env::temp_dir().join("mctk-snapshot-pack")) } else { None };
    Mutex::new(State { root, checked: None, missing: HashSet::new(), offline: false, lists: Default::default() })
});

/// The files in a folder of the assets (`assets/minecraft/textures/item`), from its listing.
fn listing(root: &Path, dir: &str, offline: bool) -> Option<std::sync::Arc<HashSet<String>>> {
    if let Some(l) = STATE.lock().ok()?.lists.get(dir) {
        return l.clone();
    }
    let kept = root.join(dir).join("_list.json");
    let text = std::fs::read_to_string(&kept).ok().or_else(|| {
        if offline {
            return None;
        }
        let t = agent().get(&format!("{BASE}/{dir}/_list.json")).call().ok()?.into_string().ok()?;
        let _ = std::fs::create_dir_all(root.join(dir));
        let _ = std::fs::write(&kept, &t);
        Some(t)
    });
    let list = text.and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).map(|v| {
        let names = |k: &str| v[k].as_array().into_iter().flatten().filter_map(|f| f.as_str().map(str::to_string)).collect::<Vec<_>>();
        std::sync::Arc::new(names("files").into_iter().chain(names("directories")).collect::<HashSet<String>>())
    });
    // (Not remembered when it could not be had, offline, so it is tried again later.)
    if list.is_some() || !offline {
        STATE.lock().ok()?.lists.insert(dir.to_string(), list.clone());
    }
    list
}

/// The files a folder of the assets holds (`assets/minecraft/textures/item`), by name.
pub fn list(dir: &str) -> Vec<String> {
    let Some(root) = root() else { return Vec::new() };
    let offline = {
        let Ok(mut s) = STATE.lock() else { return Vec::new() };
        check(&mut s, &root);
        s.offline
    };
    listing(&root, dir.trim_end_matches('/'), offline).map_or(Vec::new(), |l| l.iter().cloned().collect())
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(6)).timeout(Duration::from_secs(20)).build()
}

/// Sets the folder the assets are kept in.
pub fn set_root(root: PathBuf) {
    if let Ok(mut s) = STATE.lock() {
        if s.root.as_ref() != Some(&root) {
            s.root = Some(root);
            s.checked = None;
            s.missing.clear();
            s.lists.clear();
            s.offline = false;
        }
    }
}

/// Checks the kept files are for the latest release (clearing them if not); for a background
/// thread at start, so the first files wanted do not wait on it.
pub fn prepare() {
    if let Ok(mut s) = STATE.lock() {
        if let Some(root) = s.root.clone() {
            check(&mut s, &root);
        }
    }
}

/// Fetches every file of some folders not kept yet (sixteen at a time), so the 3D view does not
/// wait on them one by one the first time. For a background thread.
pub fn prefetch(dirs: &[&str]) {
    let mut want: Vec<String> = Vec::new();
    for dir in dirs {
        for f in list(dir) {
            if f.contains('.') && f != "_list.json" {
                want.push(format!("{dir}/{f}"));
            }
        }
    }
    let Some(root) = root() else { return };
    want.retain(|rel| !root.join(rel).is_file());
    if want.is_empty() {
        return;
    }
    eprintln!("assets: fetching {} files", want.len());
    let queue = Mutex::new(want);
    std::thread::scope(|s| {
        for _ in 0..16 {
            s.spawn(|| {
                while let Some(rel) = queue.lock().ok().and_then(|mut q| q.pop()) {
                    fetch(&rel);
                }
            });
        }
    });
    eprintln!("assets: fetched");
}

/// The folder the assets are kept in, if set.
pub fn root() -> Option<PathBuf> {
    STATE.lock().ok()?.root.clone()
}

/// Once per folder: whether the kept files are for the latest release (else they are
/// cleared), and the list of files the server lacks.
fn check(s: &mut State, root: &Path) {
    if s.checked.as_deref() == Some(root) {
        return;
    }
    s.checked = Some(root.to_path_buf());
    // The release kept: its name and when it came out (`26.3`, `2026-09-15T11:23:02+00:00`).
    let mark = root.join("mcasset_version.txt");
    let kept = std::fs::read_to_string(&mark).ok().and_then(|t| {
        let mut l = t.lines();
        Some((l.next()?.trim().to_string(), l.next().map(|x| x.trim().to_string())))
    });
    // One small request a launch: which release the server has now.
    let latest = agent()
        .get(&format!("{BASE}/version.json"))
        .call()
        .ok()
        .and_then(|r| r.into_string().ok())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| Some((v["id"].as_str()?.to_string(), v["releaseTime"].as_str()?.to_string())));
    match (latest, kept) {
        (None, _) => s.offline = true,
        // Only a newer release replaces the kept files (the same one, or an older one the
        // server may briefly offer, never does).
        (Some((_, at)), Some((_, Some(kept_at)))) if release_order(&at) <= release_order(&kept_at) => {}
        // Kept from before release times were noted: the same release is kept as it is.
        (Some((id, at)), Some((kept_id, None))) if kept_id == id => {
            let _ = std::fs::write(&mark, format!("{id}\n{at}\n"));
        }
        (Some((id, at)), _) => {
            let _ = std::fs::remove_dir_all(root.join("assets"));
            let _ = std::fs::remove_file(root.join("mcasset_missing.txt"));
            let _ = std::fs::create_dir_all(root);
            let _ = std::fs::write(&mark, format!("{id}\n{at}\n"));
            eprintln!("assets: Minecraft {id} from mcasset.cloud");
        }
    }
    if let Ok(t) = std::fs::read_to_string(root.join("mcasset_missing.txt")) {
        s.missing.extend(t.lines().map(str::to_string));
    }
}

/// A release time (`2026-09-15T11:23:02+00:00`) as seconds-ish for ordering (UTC times as the
/// server gives them; the offset is taken into account).
fn release_order(t: &str) -> i64 {
    let n = |a: usize, b: usize| t.get(a..b).and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
    let (y, mo, d, h, mi, s) = (n(0, 4), n(5, 7), n(8, 10), n(11, 13), n(14, 16), n(17, 19));
    let off = match t.get(19..20) {
        Some(sign @ ("+" | "-")) => (n(20, 22) * 60 + n(23, 25)) * if sign == "+" { 1 } else { -1 },
        _ => 0,
    };
    ((((y * 12 + mo) * 31 + d) * 24 + h) * 60 + mi - off) * 60 + s
}

/// The release the kept assets are for (`26.3`), once known.
pub fn version() -> Option<String> {
    let root = root()?;
    std::fs::read_to_string(root.join("mcasset_version.txt")).ok().and_then(|v| v.lines().next().map(|l| l.trim().to_string()))
}

/// A file of the assets by its path in a resource pack (`assets/minecraft/textures/block/stone.png`):
/// the kept copy, downloaded first if need be. `None` if the game has no such file (or it
/// cannot be fetched now).
pub fn fetch(rel: &str) -> Option<PathBuf> {
    let root = {
        let mut s = STATE.lock().ok()?;
        let root = s.root.clone()?;
        check(&mut s, &root);
        let p = root.join(rel);
        if p.is_file() {
            return Some(p);
        }
        if s.offline || s.missing.contains(rel) || rel.contains("..") {
            return None;
        }
        root
    };
    // Not in its folder's listing: the game has no such file.
    if let Some((dir, name)) = rel.rsplit_once('/') {
        if listing(&root, dir, false).is_some_and(|l| !l.contains(name)) {
            return None;
        }
    }
    let p = root.join(rel);
    // A failure that is not "no such file" (a dropped connection, too many at once) is tried
    // again a few times.
    let mut tries = 0;
    let got = loop {
        let r = agent().get(&format!("{BASE}/{rel}")).call();
        match r {
            Err(ureq::Error::Status(404, _)) | Ok(_) => break r,
            Err(e) if tries >= 3 => {
                eprintln!("assets: could not fetch {rel}: {e}");
                break Err(e);
            }
            Err(_) => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(250 << tries));
            }
        }
    };
    match got {
        Ok(resp) => {
            let mut bytes = Vec::new();
            resp.into_reader().take(64 << 20).read_to_end(&mut bytes).ok()?;
            std::fs::create_dir_all(p.parent()?).ok()?;
            // Written beside it first (under a name of its own: two threads may fetch the same
            // file at once), so a half-written file is never read.
            static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let tmp = p.with_extension(format!("part{}-{n}", std::process::id()));
            std::fs::write(&tmp, &bytes).ok()?;
            if std::fs::rename(&tmp, &p).is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            p.is_file().then_some(p)
        }
        Err(ureq::Error::Status(404, _)) => {
            if let Ok(mut s) = STATE.lock() {
                if s.missing.insert(rel.to_string()) {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(root.join("mcasset_missing.txt")) {
                        let _ = writeln!(f, "{rel}");
                    }
                }
            }
            None
        }
        Err(_) => None,
    }
}
