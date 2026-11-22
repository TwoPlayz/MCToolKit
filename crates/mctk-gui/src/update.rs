//! Updates: asks GitHub for the latest release when the app starts and, when it is newer than
//! the running version, downloads the Windows build from that release and puts it in the place
//! of the running one (the old file is kept beside it until the next start, then removed), so the
//! app restarts as the new version. Where that cannot be done (another system, a folder the
//! user cannot write to) the release page is offered instead.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;

/// `owner/name` of the GitHub repository that publishes MCToolKit releases.
pub const REPO: &str = "TwoPlayz/MCToolKit";

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub version: String,
    pub url: String,
    /// The release's Windows build (a zip with `MCToolKit.exe`): where to get it and its size.
    pub asset: Option<(String, u64)>,
}

/// How far a download has come (for the splash screen).
#[derive(Default)]
pub struct Progress {
    pub done: AtomicU64,
    pub total: AtomicU64,
}

pub fn releases_url() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}

/// Whether this build can replace itself with `u`'s download.
pub fn can_install(u: &Update) -> bool {
    cfg!(windows) && u.asset.is_some()
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(8)).timeout_read(Duration::from_secs(30)).build()
}

/// The newest release when it is newer than this build, `None` when up to date.
pub fn check() -> Result<Option<Update>, String> {
    let body = agent()
        .get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("User-Agent", concat!("MCToolKit/", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tag = v.get("tag_name").and_then(Value::as_str).ok_or("the release has no tag")?;
    let url = v.get("html_url").and_then(Value::as_str).map_or_else(releases_url, str::to_string);
    Ok(is_newer(tag, env!("CARGO_PKG_VERSION")).then(|| Update { version: tag.trim_start_matches(['v', 'V']).to_string(), url, asset: pick_asset(&v) }))
}

/// The release's Windows zip, from its `assets`.
fn pick_asset(release: &Value) -> Option<(String, u64)> {
    release.get("assets")?.as_array()?.iter().find_map(|a| {
        let name = a.get("name")?.as_str()?.to_lowercase();
        if !(name.ends_with(".zip") && name.contains("windows")) {
            return None;
        }
        Some((a.get("browser_download_url")?.as_str()?.to_string(), a.get("size").and_then(Value::as_u64).unwrap_or(0)))
    })
}

/// The running program's file, and the names beside it the update uses.
fn files() -> Result<(PathBuf, PathBuf, PathBuf, PathBuf), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let (zip, new, old) = (exe.with_extension("download"), exe.with_extension("new"), exe.with_extension("old"));
    Ok((exe, zip, new, old))
}

/// Removes what an update left beside the program (the replaced file, a half download).
pub fn clean_up() {
    if let Ok((_, zip, new, old)) = files() {
        for f in [zip, new, old] {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// Downloads `u`'s build and puts it in the running program's place (the running file is renamed
/// aside, which Windows allows); the caller then starts the program again.
pub fn install(u: &Update, progress: &Progress) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("updates are installed on Windows".into());
    }
    let (url, size) = u.asset.clone().ok_or("the release has no Windows download")?;
    let (exe, zip_path, new_path, old_path) = files()?;
    // Only from this project's own releases.
    if !url.starts_with(&format!("https://github.com/{REPO}/releases/download/")) {
        return Err("the download is not from the project's releases".into());
    }
    let response = agent().get(&url).set("User-Agent", concat!("MCToolKit/", env!("CARGO_PKG_VERSION"))).call().map_err(|e| format!("could not download it: {e}"))?;
    let total = response.header("Content-Length").and_then(|s| s.parse().ok()).unwrap_or(size);
    progress.total.store(total, Ordering::Relaxed);
    let mut reader = response.into_reader();
    let mut file = std::fs::File::create(&zip_path).map_err(|e| format!("could not save the download beside the program: {e}"))?;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("the download stopped: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        progress.done.fetch_add(n as u64, Ordering::Relaxed);
    }
    drop(file);
    let result = unpack_and_swap(&exe, &zip_path, &new_path, &old_path);
    let _ = std::fs::remove_file(&zip_path);
    let _ = std::fs::remove_file(&new_path);
    result
}

fn unpack_and_swap(exe: &std::path::Path, zip_path: &std::path::Path, new_path: &std::path::Path, old_path: &std::path::Path) -> Result<(), String> {
    let name = exe.file_name().and_then(|n| n.to_str()).ok_or("no program name")?.to_lowercase();
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip_path).map_err(|e| e.to_string())?).map_err(|e| format!("the download is not a zip: {e}"))?;
    let mut found = false;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let is_exe = entry.name().rsplit(['/', '\\']).next().is_some_and(|n| n.to_lowercase() == name);
        if !is_exe {
            continue;
        }
        let mut out = std::fs::File::create(new_path).map_err(|e| format!("could not write the new program beside the old: {e}"))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
        found = true;
        break;
    }
    if !found {
        return Err("the download has no program in it".into());
    }
    // A program, and not a stub: the right start and a sensible size.
    let mut head = [0u8; 2];
    let size = std::fs::metadata(new_path).map(|m| m.len()).unwrap_or(0);
    if size < 1_000_000 || std::fs::File::open(new_path).and_then(|mut f| f.read_exact(&mut head)).is_err() || &head != b"MZ" {
        return Err("the downloaded program looks damaged".into());
    }
    let _ = std::fs::remove_file(old_path);
    std::fs::rename(exe, old_path).map_err(|e| format!("could not replace the program (is it in a folder you can write to?): {e}"))?;
    if let Err(e) = std::fs::rename(new_path, exe) {
        let _ = std::fs::rename(old_path, exe);
        return Err(format!("could not put the new program in place: {e}"));
    }
    Ok(())
}

/// Starts the program again (after an update); the caller then exits.
pub fn relaunch() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).arg("--updated").spawn();
    }
}

/// Numeric `major.minor.patch` comparison; a pre-release suffix is ignored.
fn parse(v: &str) -> Option<Vec<u64>> {
    let core = v.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    core.split('.').map(|p| p.parse().ok()).collect()
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse(latest), parse(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_newer, pick_asset};

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }

    #[test]
    fn the_windows_zip_is_the_download() {
        let release: serde_json::Value = serde_json::from_str(
            r#"{"assets":[{"name":"notes.txt","browser_download_url":"https://x/notes","size":1},
            {"name":"MCToolKit-v0.2.0-windows-x64.zip","browser_download_url":"https://github.com/o/r/releases/download/v0.2.0/a.zip","size":123}]}"#,
        )
        .unwrap();
        assert_eq!(pick_asset(&release), Some(("https://github.com/o/r/releases/download/v0.2.0/a.zip".into(), 123)));
        assert_eq!(pick_asset(&serde_json::json!({"assets": []})), None);
    }
}