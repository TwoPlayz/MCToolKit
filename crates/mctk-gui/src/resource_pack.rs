//! MCToolKit's own resource pack folder (`<working dir>/resource_packs/default`), whose
//! block and item textures draw icons and whose `pack.png` is the image of worlds without
//! one.
//!
//! The textures are Mojang's artwork, so MCToolKit does not bundle them: on first run the
//! folder is filled from an installed Universal Minecraft Tool (its downloaded default pack,
//! and `pack.png` from its executable) or from an installed Java client jar. After that the
//! folder is MCToolKit's own and UMT is not needed.

use std::path::{Path, PathBuf};

/// `<working dir>/resource_packs/default`.
pub fn default_dir(working_dir: &Path) -> PathBuf {
    working_dir.join("resource_packs").join("default")
}

/// UMT's downloaded default pack, when UMT is installed.
pub fn umt_pack() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("APPDATA")?)
        .join(r"Pryze Software\Universal Minecraft Tool\APP\resource_packs\default");
    p.join("assets").is_dir().then_some(p)
}

/// Fills `pack` if it has no textures yet, and adds `pack.png` if it is missing.
pub fn ensure(pack: &Path) {
    if !pack.join(r"assets\minecraft\textures").is_dir() {
        if let Some(src) = umt_pack() {
            let _ = copy_dir(&src.join("assets"), &pack.join("assets"));
        } else if let Some(jar) = newest_jar() {
            let _ = extract_textures(&jar, pack);
        }
    }
    let png = pack.join("pack.png");
    if !png.is_file() {
        if let Some(bytes) = pack_png_from_umt().or_else(pack_png_from_jar) {
            let _ = std::fs::create_dir_all(pack);
            let _ = std::fs::write(png, bytes);
        }
    }
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)?.flatten() {
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
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

/// UMT's copy of `pack.png` (`:/img/misc/pack.png`), stored verbatim in its executable's Qt
/// resources: a PNG of this length and CRC-32, preceded by rcc's big-endian length prefix.
fn pack_png_from_umt() -> Option<Vec<u8>> {
    const LEN: usize = 27_267;
    const CRC: u32 = 0xB268_43D3;
    const SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    let exe = ["ProgramFiles(x86)", "ProgramFiles"].iter().find_map(|v| {
        let p = PathBuf::from(std::env::var_os(v)?)
            .join("Universal Minecraft Tool")
            .join("UniversalMinecraftTool.exe");
        p.is_file().then_some(p)
    })?;
    let b = std::fs::read(exe).ok()?;
    let len_prefix = (LEN as u32).to_be_bytes();
    b.windows(12)
        .enumerate()
        .filter(|(_, w)| w[..4] == len_prefix && &w[4..] == SIG)
        .map(|(i, _)| &b[i + 4..])
        .filter(|rest| rest.len() >= LEN)
        .map(|rest| &rest[..LEN])
        .find(|png| crc32fast::hash(png) == CRC)
        .map(<[u8]>::to_vec)
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
