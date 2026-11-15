//! MCToolKit's own resource pack folder (`<working dir>/resource_packs/default`), whose
//! block and item textures draw icons and whose `pack.png` is the image of worlds without
//! one.
//!
//! The textures are Mojang's artwork, so MCToolKit does not bundle them: on first run the
//! folder is filled from an installed Java client jar. After that the folder is MCToolKit's
//! own and nothing else is needed.

use std::path::{Path, PathBuf};

/// `<working dir>/resource_packs/default`.
pub fn default_dir(working_dir: &Path) -> PathBuf {
    working_dir.join("resource_packs").join("default")
}

/// Fills `pack` if it has no textures yet, and adds `pack.png` if it is missing.
pub fn ensure(pack: &Path) {
    if !pack.join(r"assets\minecraft\textures").is_dir() {
        if let Some(jar) = newest_jar() {
            let _ = extract_textures(&jar, pack);
        }
    }
    let png = pack.join("pack.png");
    if !png.is_file() {
        if let Some(bytes) = pack_png_from_jar() {
            let _ = std::fs::create_dir_all(pack);
            let _ = std::fs::write(png, bytes);
        }
    }
}

/// The client jar's `assets/minecraft/textures/**`.
fn extract_textures(jar: &Path, pack: &Path) -> Option<()> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(jar).ok()?).ok()?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).ok()?;
        let Some(rel) = f.enclosed_name() else {
            continue;
        };
        if !rel.starts_with("assets/minecraft/textures") || f.is_dir() {
            continue;
        }
        let out = pack.join(rel);
        std::fs::create_dir_all(out.parent()?).ok()?;
        std::io::copy(&mut f, &mut std::fs::File::create(out).ok()?).ok()?;
    }
    Some(())
}

/// The newest installed Java client jar.
fn newest_jar() -> Option<PathBuf> {
    let versions = PathBuf::from(std::env::var_os("APPDATA")?)
        .join(".minecraft")
        .join("versions");
    std::fs::read_dir(versions)
        .ok()?
        .flatten()
        .filter_map(|d| {
            let jar = d
                .path()
                .join(format!("{}.jar", d.file_name().to_string_lossy()));
            let modified = std::fs::metadata(&jar).and_then(|m| m.modified()).ok()?;
            Some((modified, jar))
        })
        .max()
        .map(|(_, jar)| jar)
}

fn pack_png_from_jar() -> Option<Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(newest_jar()?).ok()?).ok()?;
    let mut entry = zip.by_name("pack.png").ok()?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut entry, &mut bytes).ok()?;
    Some(bytes)
}
