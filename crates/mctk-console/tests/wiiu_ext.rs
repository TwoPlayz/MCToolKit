//! The Wii U companion file, against the one a real converter wrote (`tests/fixtures/wiiu/dev.ext`).

use mctk_console::{EXT_NAME_BYTES, build_wiiu_ext, parse_wiiu_ext, read_wiiu_ext, write_wiiu_ext};

fn real() -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/wiiu/dev.ext")).unwrap()
}

#[test]
fn rebuilding_the_real_companion_is_byte_exact() {
    let real = real();
    let parsed = parse_wiiu_ext(&real).unwrap();
    assert_eq!(parsed.name, "dev");
    assert_eq!(&parsed.thumbnail[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
    assert_eq!(parsed.thumbnail.len(), 7093);
    assert_eq!(build_wiiu_ext("dev", &parsed.thumbnail), real);
    assert_eq!(real.len(), EXT_NAME_BYTES + 7093);
}

#[test]
fn written_companion_is_found_next_to_the_save_file() {
    let dir = std::env::temp_dir().join(format!("mctk-wiiu-ext-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let thumb = parse_wiiu_ext(&real()).unwrap().thumbnail;
    for file in ["wiiu_save", "savegame.wii"] {
        let save = dir.join(file);
        std::fs::write(&save, b"x").unwrap();
        let ext_path = write_wiiu_ext(&save, "Hello", &thumb).unwrap();
        assert!(ext_path.is_file(), "{file}");
        let found = read_wiiu_ext(&save).expect(file);
        assert_eq!((found.name.as_str(), found.thumbnail.len()), ("Hello", 7093));
    }
    assert_eq!(dir.join("wiiu_save.ext").file_name().unwrap(), "wiiu_save.ext");
    assert_eq!(dir.join("savegame.ext").file_name().unwrap(), "savegame.ext");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn picking_the_companion_opens_the_save_beside_it() {
    use mctk_console::{ArchiveEntry, ConsoleSave, Platform, SaveArchive, compress_wiiu};
    let dir = std::env::temp_dir().join(format!("mctk-wiiu-pick-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let archive = SaveArchive::new(10, vec![ArchiveEntry::new("level.dat", vec![1u8; 64], 0)], false);
    let thumb = parse_wiiu_ext(&real()).unwrap().thumbnail;
    // The converter's naming (suffix-less save + .ext) and the game's (savegame.wii + savegame.ext).
    for file in ["wiiu_save", "savegame.wii", "world.wii"] {
        let save = dir.join(file);
        std::fs::write(&save, compress_wiiu(&archive.encode().unwrap()).unwrap()).unwrap();
        let ext = write_wiiu_ext(&save, "Picked", &thumb).unwrap();
        let opened = ConsoleSave::open(&ext).unwrap();
        assert_eq!(opened.platform, Platform::WiiU, "{file}");
        assert_eq!(opened.source, save, "{file}");
        assert_eq!(opened.archive.entries.len(), 1);
        std::fs::remove_file(&ext).unwrap();
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_save_without_a_companion_has_no_name() {
    let dir = std::env::temp_dir().join(format!("mctk-wiiu-none-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let save = dir.join("alone.wii");
    std::fs::write(&save, b"x").unwrap();
    assert!(read_wiiu_ext(&save).is_none());
    std::fs::remove_dir_all(&dir).ok();
}
