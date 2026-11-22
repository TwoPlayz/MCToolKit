//! The companion file of a Wii U save: its name and thumbnail.
//!
//! A Wii U save is `savegame.wii` (see [`compress_wiiu`](crate::compress_wiiu)) plus a small companion
//! with the extension `.ext`: a 256-byte name field followed by a PNG thumbnail. This was measured on a
//! file written by a real converter (7,349 bytes: the name "dev" in the field, a 64x64 RGB PNG of 7,093
//! bytes at offset 256). The field holds up to 128 UTF-16BE code units, zero-padded; the converter
//! reads it back by taking every second byte up to the first zero, which is the same for ASCII names.

use std::path::{Path, PathBuf};

use crate::{Error, bad};

/// Bytes of the name field at the start of an `.ext` file.
pub const EXT_NAME_BYTES: usize = 256;

/// A parsed companion file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WiiUExt {
    pub name: String,
    /// The PNG thumbnail (64x64 in files written by the converter).
    pub thumbnail: Vec<u8>,
}

/// Builds the companion file: the name as UTF-16BE (truncated to 128 code units, never in the
/// middle of a surrogate pair) padded with zeros to 256 bytes, then the PNG.
pub fn build_wiiu_ext(world_name: &str, thumbnail_png: &[u8]) -> Vec<u8> {
    let mut units: Vec<u16> = world_name.encode_utf16().take(EXT_NAME_BYTES / 2).collect();
    if units.len() == EXT_NAME_BYTES / 2 && units.last().is_some_and(|u| (0xD800..0xDC00).contains(u)) {
        units.pop(); // a high surrogate cut off from its pair
    }
    let mut out = Vec::with_capacity(EXT_NAME_BYTES + thumbnail_png.len());
    for u in &units {
        out.extend_from_slice(&u.to_be_bytes());
    }
    out.resize(EXT_NAME_BYTES, 0);
    out.extend_from_slice(thumbnail_png);
    out
}

/// Parses a companion file. The name ends at the first zero code unit.
pub fn parse_wiiu_ext(bytes: &[u8]) -> Result<WiiUExt, Error> {
    if bytes.len() < EXT_NAME_BYTES {
        return Err(bad("Wii U companion file shorter than its 256-byte name field"));
    }
    let units: Vec<u16> = bytes[..EXT_NAME_BYTES]
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .take_while(|u| *u != 0)
        .collect();
    Ok(WiiUExt { name: String::from_utf16_lossy(&units), thumbnail: bytes[EXT_NAME_BYTES..].to_vec() })
}

/// Where the companion of `save_file` goes: the same name with the extension `.ext`
/// (`wiiu_save` gives `wiiu_save.ext`, `savegame.wii` gives `savegame.ext`).
pub fn wiiu_ext_path(save_file: &Path) -> PathBuf {
    save_file.with_extension("ext")
}

/// The save file a companion belongs to, when `companion` (a `.ext` path) was picked instead of the
/// save itself: `<name>.wii`, the suffix-less `<name>`, or `savegame.wii` beside it.
pub(crate) fn save_for_companion(companion: &Path) -> Option<PathBuf> {
    [companion.with_extension("wii"), companion.with_extension(""), companion.with_file_name("savegame.wii")]
        .into_iter()
        .find(|p| p != companion && p.is_file())
}

/// Writes the companion next to `save_file` and returns its path.
pub fn write_wiiu_ext(save_file: &Path, world_name: &str, thumbnail_png: &[u8]) -> Result<PathBuf, Error> {
    let path = wiiu_ext_path(save_file);
    std::fs::write(&path, build_wiiu_ext(world_name, thumbnail_png))?;
    Ok(path)
}

/// Reads the companion of `save_file`, if there is one: `<name>.ext`, `<file name>.ext` or a
/// `savegame.ext` beside it.
pub fn read_wiiu_ext(save_file: &Path) -> Option<WiiUExt> {
    let mut with_suffix = save_file.as_os_str().to_owned();
    with_suffix.push(".ext");
    let candidates = [wiiu_ext_path(save_file), PathBuf::from(with_suffix), save_file.with_file_name("savegame.ext")];
    candidates.iter().find_map(|p| std::fs::read(p).ok().and_then(|b| parse_wiiu_ext(&b).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_round_trips() {
        let png = [0x89, b'P', b'N', b'G'];
        for name in ["dev", "My World", "", "Wélt \u{4e16}\u{754c}", "\u{1f30d} Earth"] {
            let bytes = build_wiiu_ext(name, &png);
            assert_eq!(bytes.len(), EXT_NAME_BYTES + 4);
            let ext = parse_wiiu_ext(&bytes).unwrap();
            assert_eq!(ext.name, name);
            assert_eq!(ext.thumbnail, png);
        }
    }

    #[test]
    fn long_names_are_cut_without_splitting_a_pair() {
        let name = "a".repeat(127) + "\u{1f30d}"; // the emoji would straddle the 128-unit limit
        let ext = parse_wiiu_ext(&build_wiiu_ext(&name, &[])).unwrap();
        assert_eq!(ext.name, "a".repeat(127));
        let ext = parse_wiiu_ext(&build_wiiu_ext(&"b".repeat(300), &[])).unwrap();
        assert_eq!(ext.name.len(), 128);
    }

    #[test]
    fn companion_path_rules() {
        assert_eq!(wiiu_ext_path(Path::new("x/wiiu_save")), Path::new("x/wiiu_save.ext"));
        assert_eq!(wiiu_ext_path(Path::new("x/savegame.wii")), Path::new("x/savegame.ext"));
    }

    #[test]
    fn short_files_are_rejected() {
        assert!(parse_wiiu_ext(&[0u8; 100]).is_err());
    }
}
