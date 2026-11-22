//! Golden tests for PS3 save signing, against files written by a real converter.
//! See `tests/fixtures/ps3/README.txt` for where the fixtures come from.

use mctk_console::{
    METADATA_SIZE, Ps3Random, Ps3Region, build_param_pfd, build_param_sfo, build_save_folder, decrypt_metadata,
    encrypt_metadata, metadata_plaintext, metadata_plaintext_v, read_pfd_random, resign_save_folder, verify_param_pfd,
    verify_save_folder, write_save_folder,
};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ps3").join(name))
        .unwrap_or_else(|e| panic!("fixture {name}: {e}"))
}

fn random_from_fixture() -> Ps3Random {
    let b = fixture("a_random.bin");
    assert_eq!(b.len(), 16 + 20 + 64 + 64);
    Ps3Random {
        iv: b[..16].try_into().unwrap(),
        hash_key: b[16..36].try_into().unwrap(),
        entry_keys: [b[36..100].try_into().unwrap(), b[100..164].try_into().unwrap()],
    }
}

#[test]
fn param_sfo_matches_the_real_file() {
    let built = build_param_sfo("NPUB31419--261121141221", "ps3").unwrap();
    assert_eq!(built, fixture("a_param.sfo"));
}

#[test]
fn metadata_matches_the_real_file() {
    // Numbers the second save encoded: GAMEDATA 9,511,997 bytes (crc d6151342), icon 7,093 (1dfa0b85).
    let plain = metadata_plaintext(9_511_997, 7_093, 0xd615_1342, 0x1dfa_0b85);
    let key: [u8; 64] = fixture("e_entry_key.bin").try_into().unwrap();
    let built = encrypt_metadata(&plain, &key);
    assert_eq!(built.len(), METADATA_SIZE);
    assert_eq!(built.to_vec(), fixture("e_metadata"));
}

#[test]
fn param_pfd_matches_the_real_file() {
    let sfo = fixture("a_param.sfo");
    let metadata: [u8; METADATA_SIZE] = fixture("a_metadata").try_into().unwrap();
    let built = build_param_pfd(&sfo, &metadata, &random_from_fixture());
    let real = fixture("a_param.pfd");
    assert_eq!(built.len(), real.len());
    assert_eq!(built, real, "rebuilt PARAM.PFD differs from the converter's");
}

#[test]
fn real_pfd_verifies_and_yields_its_random_values() {
    let contents = verify_param_pfd(&fixture("a_param.pfd"), &fixture("a_param.sfo"), &fixture("a_metadata")).unwrap();
    assert_eq!(contents.random, random_from_fixture());
}

#[test]
fn metadata_decrypts_to_the_numbers_it_encodes() {
    let key: [u8; 64] = fixture("e_entry_key.bin").try_into().unwrap();
    let meta: [u8; METADATA_SIZE] = fixture("e_metadata").try_into().unwrap();
    let plain = decrypt_metadata(&meta, &key);
    let words: Vec<u32> = (0..5).map(|i| u32::from_be_bytes(plain[4 * i..4 * i + 4].try_into().unwrap())).collect();
    assert_eq!(words, [3, 9_511_997, 7_093, 0xd615_1342, 0x1dfa_0b85]);
    assert!(plain[20..].iter().all(|b| *b == 0));
}

#[test]
fn pfd_random_values_can_be_read_back() {
    assert_eq!(read_pfd_random(&fixture("a_param.pfd")).unwrap(), random_from_fixture());
    assert!(read_pfd_random(&fixture("a_param.pfd")[..1000]).is_err());
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("mctk-ps3-resign-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn resigning_a_folder_after_gamedata_changes() {
    let dir = temp_dir("folder");
    let random = random_from_fixture();
    let folder = build_save_folder(
        vec![1; 5000],
        vec![9; 300],
        vec![4; 120],
        Ps3Region::UsDigital,
        "world",
        "261121120000",
        &random,
    )
    .unwrap();
    let saved = write_save_folder(&dir, &folder).unwrap();
    assert_eq!(verify_save_folder(&saved).unwrap(), 5000);

    // A different GAMEDATA (as after pruning or editing) breaks METADATA until the save is re-signed.
    std::fs::write(saved.join("GAMEDATA"), vec![2u8; 7777]).unwrap();
    assert!(verify_save_folder(&saved).is_err());
    resign_save_folder(&saved).unwrap();
    assert_eq!(verify_save_folder(&saved).unwrap(), 7777);
    // The random values are kept, so re-signing is stable.
    let pfd = std::fs::read(saved.join("PARAM.PFD")).unwrap();
    assert_eq!(read_pfd_random(&pfd).unwrap(), random);
    resign_save_folder(&saved).unwrap();
    assert_eq!(std::fs::read(saved.join("PARAM.PFD")).unwrap(), pfd);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn resigning_keeps_the_version_and_keys_of_a_save_signed_with_other_keys() {
    // Like a genuine console save: PFD header and table are valid, but the file hashes of the entries
    // were made with keys this converter does not have.
    let dir = temp_dir("foreign");
    let mut pfd = fixture("a_param.pfd");
    pfd[0x240 + 0x90] ^= 0xff; // PARAM.SFO hash0
    pfd[0x350 + 0x90] ^= 0xff; // METADATA hash0
    assert!(verify_param_pfd(&pfd, &fixture("a_param.sfo"), &fixture("a_metadata")).is_err());
    assert_eq!(read_pfd_random(&pfd).unwrap(), random_from_fixture());

    // A METADATA of version 2 (older game versions), encrypted with the save's own entry key.
    let key = random_from_fixture().entry_keys[1];
    let old_meta = encrypt_metadata(&metadata_plaintext_v(2, 10, 20, 30, 40), &key);
    std::fs::write(dir.join("PARAM.PFD"), &pfd).unwrap();
    std::fs::write(dir.join("PARAM.SFO"), fixture("a_param.sfo")).unwrap();
    std::fs::write(dir.join("METADATA"), old_meta).unwrap();
    std::fs::write(dir.join("GAMEDATA"), vec![5u8; 4096]).unwrap();
    std::fs::write(dir.join("THUMB"), vec![6u8; 100]).unwrap();

    resign_save_folder(&dir).unwrap();
    assert_eq!(verify_save_folder(&dir).unwrap(), 4096);
    let meta: [u8; METADATA_SIZE] = std::fs::read(dir.join("METADATA")).unwrap().try_into().unwrap();
    let plain = decrypt_metadata(&meta, &key);
    assert_eq!(u32::from_be_bytes(plain[..4].try_into().unwrap()), 2, "the version word is kept");
    assert_eq!(read_pfd_random(&std::fs::read(dir.join("PARAM.PFD")).unwrap()).unwrap(), random_from_fixture());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn tampering_is_detected() {
    let pfd = fixture("a_param.pfd");
    let sfo = fixture("a_param.sfo");
    let meta = fixture("a_metadata");
    let mut bad_meta = meta.clone();
    bad_meta[0] ^= 0x80;
    assert!(verify_param_pfd(&pfd, &sfo, &bad_meta).is_err());
    let mut bad_pfd = pfd.clone();
    bad_pfd[0x7b60 + 20 * 8] ^= 1; // the PARAM.SFO bucket hash
    assert!(verify_param_pfd(&bad_pfd, &sfo, &meta).is_err());
    assert!(verify_param_pfd(&pfd[..100], &sfo, &meta).is_err());
}
