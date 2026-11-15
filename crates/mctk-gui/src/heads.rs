//! Player names and heads from a UUID, as the original tool shows them: the UUID is looked up
//! on Mojang's public profile service for the player's name and skin, and the face of the skin
//! becomes the player's icon. Nothing is read from the Minecraft launcher.

use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;

use serde_json::Value;

/// What is known about a player.
pub struct Profile {
    pub name: String,
    pub head: Option<PathBuf>,
}

/// A UUID as the profile service writes it: lowercase, no dashes; `None` if `text` is not one.
pub fn normalise(text: &str) -> Option<String> {
    let id: String = text
        .chars()
        .filter(|c| *c != '-')
        .flat_map(|c| c.to_lowercase())
        .collect();
    (id.len() == 32 && id.chars().all(|c| c.is_ascii_hexdigit())).then_some(id)
}

/// Where lookups are kept, so a tab can show a player's name without being handed the folder.
static CACHE: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_cache(dir: PathBuf) {
    *CACHE.lock().expect("heads cache") = Some(dir);
}

/// The name of the player behind `uuid`, if it has already been looked up.
pub fn cached_name(uuid: &str) -> Option<String> {
    let id = normalise(uuid)?;
    let dir = CACHE.lock().expect("heads cache").clone()?;
    std::fs::read_to_string(dir.join(format!("{id}.txt"))).ok()
}

/// Looks the players up before a world opens, so their names and heads are there from the
/// first frame. It waits a few seconds at most; without a connection it gives up at once, and
/// anything not answered in time carries on in the background.
pub fn prefetch(cache: &Path, uuids: impl IntoIterator<Item = String>) {
    static STARTED_NOW: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let (tx, rx) = mpsc::channel();
    let mut waiting = 0;
    for text in uuids {
        let Some(id) = normalise(&text) else { continue };
        if cache.join(format!("{id}.txt")).is_file() || cache.join(format!("{id}.none")).is_file() {
            continue;
        }
        let fresh = STARTED_NOW
            .get_or_init(Default::default)
            .lock()
            .expect("heads")
            .insert(id.clone());
        if !fresh {
            continue;
        }
        waiting += 1;
        let (tx, cache) = (tx.clone(), cache.to_path_buf());
        std::thread::spawn(move || {
            lookup(&cache, &id);
            let _ = tx.send(());
        });
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    for _ in 0..waiting {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if rx.recv_timeout(left).is_err() {
            break;
        }
    }
}

/// Fetches `id`, remembering when the service does not know it.
fn lookup(cache: &Path, id: &str) {
    if fetch(cache, id).is_none() {
        let _ = std::fs::create_dir_all(cache);
        let _ = std::fs::write(cache.join(format!("{id}.none")), "");
    }
}

/// The player behind `uuid`, once it has been looked up. The first call starts the lookup in
/// the background and returns `None`; players the service does not know stay `None`.
pub fn profile(cache: &Path, uuid: &str, ctx: &eframe::egui::Context) -> Option<Profile> {
    let id = normalise(uuid)?;
    let info = cache.join(format!("{id}.txt"));
    if let Ok(name) = std::fs::read_to_string(&info) {
        let head = cache.join(format!("{id}.png"));
        return Some(Profile {
            name,
            head: head.is_file().then_some(head),
        });
    }
    if cache.join(format!("{id}.none")).is_file() {
        return None;
    }
    static STARTED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    // A lookup that `prefetch` already began is not begun twice.
    let fresh = STARTED
        .get_or_init(Default::default)
        .lock()
        .expect("heads")
        .insert(id.clone());
    if fresh {
        let (ctx, cache) = (ctx.clone(), cache.to_path_buf());
        std::thread::spawn(move || {
            // A failure is remembered, so an offline-mode player is not asked about again.
            lookup(&cache, &id);
            ctx.request_repaint();
        });
    }
    None
}

/// Asks for the profile of `id`, saves its name and the face of its skin, hat layer included.
fn fetch(cache: &Path, id: &str) -> Option<()> {
    let get = |url: &str| ureq::get(url).timeout(Duration::from_secs(10)).call().ok();
    let profile: Value = serde_json::from_str(
        &get(&format!("https://sessionserver.mojang.com/session/minecraft/profile/{id}"))?
            .into_string()
            .ok()?,
    )
    .ok()?;
    let name = profile["name"].as_str()?.to_string();
    std::fs::create_dir_all(cache).ok()?;
    // The head is optional: without a readable skin the name still shows.
    let _ = head_picture(&get, &profile, &cache.join(format!("{id}.png")));
    std::fs::write(cache.join(format!("{id}.txt")), name).ok()
}

fn head_picture(
    get: &impl Fn(&str) -> Option<ureq::Response>,
    profile: &Value,
    out: &Path,
) -> Option<()> {
    let textures: Value =
        serde_json::from_slice(&base64(profile["properties"][0]["value"].as_str()?)?).ok()?;
    let url = textures["textures"]["SKIN"]["url"].as_str()?;
    let mut bytes = Vec::new();
    get(url)?.into_reader().take(1 << 20).read_to_end(&mut bytes).ok()?;
    let skin = image::load_from_memory(&bytes).ok()?.to_rgba8();
    if skin.width() < 64 || skin.height() < 16 {
        return None;
    }
    let mut face = image::RgbaImage::new(8, 8);
    for y in 0..8 {
        for x in 0..8 {
            let mut px = *skin.get_pixel(8 + x, 8 + y);
            let hat = skin.get_pixel(40 + x, 8 + y);
            if hat[3] > 0 {
                px = *hat;
            }
            face.put_pixel(x, y, px);
        }
    }
    image::imageops::resize(&face, 64, 64, image::imageops::FilterType::Nearest)
        .save(out)
        .ok()
}

/// Standard base64, without padding checks.
fn base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes().filter(|c| *c != b'=') {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_base64_and_recognises_uuids() {
        assert_eq!(base64("aGVsbG8gd29ybGQ=").unwrap(), b"hello world");
        assert_eq!(base64("YQ==").unwrap(), b"a");
        assert_eq!(
            normalise("0F8FAD5B-D9CB-469F-A165-70867728950E").as_deref(),
            Some("0f8fad5bd9cb469fa16570867728950e")
        );
        assert_eq!(normalise("~local_player"), None);
        assert_eq!(normalise("P_Steve"), None);
    }
}