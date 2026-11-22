//! The two layouts of a PS3 `GAMEDATA`: the raw archive of older game versions and the compressed
//! one of newer versions (`u32 0`, length twice, raw deflate, Adler-32), as written by a real converter.

use mctk_console::{
    ArchiveEntry, ConsoleSave, Platform, SaveArchive, compress_ps3_gamedata, decompress_ps3_gamedata,
};

fn archive() -> SaveArchive {
    let files = vec![
        ArchiveEntry::new("level.dat", vec![7u8; 300], 0),
        ArchiveEntry::new("r.0.0.mcr", (0..5000u32).map(|i| (i % 251) as u8).collect(), 0),
    ];
    SaveArchive::new(10, files, false)
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("mctk-ps3-gamedata-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn compressed_layout_has_the_measured_shape() {
    let raw = archive().encode().unwrap();
    let packed = compress_ps3_gamedata(&raw).unwrap();
    assert_eq!(packed[..4], [0, 0, 0, 0]);
    assert_eq!(packed[4..8], (raw.len() as u32).to_be_bytes());
    assert_eq!(packed[8..12], (raw.len() as u32).to_be_bytes());
    assert_eq!(decompress_ps3_gamedata(&packed).unwrap().unwrap(), raw);
}

#[test]
fn raw_archives_are_not_mistaken_for_compressed_ones() {
    assert!(decompress_ps3_gamedata(&archive().encode().unwrap()).unwrap().is_none());
}

#[test]
fn a_wrong_checksum_is_rejected() {
    let mut packed = compress_ps3_gamedata(&archive().encode().unwrap()).unwrap();
    let n = packed.len();
    packed[n - 1] ^= 1;
    assert!(decompress_ps3_gamedata(&packed).is_err());
}

#[test]
fn compressed_gamedata_opens_and_round_trips_through_a_save_folder() {
    let dir = temp_dir("compressed");
    let packed = compress_ps3_gamedata(&archive().encode().unwrap()).unwrap();
    std::fs::write(dir.join("GAMEDATA"), &packed).unwrap();
    let save = ConsoleSave::open(&dir).unwrap();
    assert_eq!(save.platform, Platform::Ps3);
    assert!(save.gamedata_compressed);
    assert_eq!(save.archive.version, 10);
    assert_eq!(save.archive.entries.len(), 2);
    // Writing it back keeps the compressed layout and decodes to the same archive.
    let again = save.encode().unwrap();
    assert_eq!(decompress_ps3_gamedata(&again).unwrap().unwrap(), archive().encode().unwrap());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn raw_gamedata_still_opens_and_stays_raw() {
    let dir = temp_dir("raw");
    std::fs::write(dir.join("GAMEDATA"), archive().encode().unwrap()).unwrap();
    let save = ConsoleSave::open(&dir).unwrap();
    assert_eq!(save.platform, Platform::Ps3);
    assert!(!save.gamedata_compressed);
    assert_eq!(save.encode().unwrap(), archive().encode().unwrap());
    std::fs::remove_dir_all(&dir).ok();
}
