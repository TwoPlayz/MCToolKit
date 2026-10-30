//! Update check: asks GitHub for the latest release when the app starts and reports it when
//! it is newer than the running version. Nothing is downloaded or installed.

use std::time::Duration;

use serde_json::Value;

/// `owner/name` of the GitHub repository that publishes MCToolKit releases.
pub const REPO: &str = "TwoPlayz/MCToolKit";

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub version: String,
    pub url: String,
}

pub fn releases_url() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}

/// The newest release when it is newer than this build, `None` when up to date.
pub fn check() -> Result<Option<Update>, String> {
    let body = ureq::get(&format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .set("User-Agent", concat!("MCToolKit/", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tag = v
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or("the release has no tag")?;
    let url = v
        .get("html_url")
        .and_then(Value::as_str)
        .map_or_else(releases_url, str::to_string);
    Ok(is_newer(tag, env!("CARGO_PKG_VERSION")).then(|| Update {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url,
    }))
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
    use super::is_newer;

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("nightly", "0.1.0"));
    }
}
