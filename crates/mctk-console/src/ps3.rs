//! PlayStation 3 save folders: `PARAM.SFO`, `METADATA`, `PARAM.PFD` and the folder around `GAMEDATA`.
//!
//! A PS3 Legacy Console Edition save is a folder `<TitleID>--<yyMMddHHmmss>` holding
//!
//! | file | content |
//! |---|---|
//! | `GAMEDATA` | the save archive (see [`SaveArchive`](crate::SaveArchive)), unprotected |
//! | `ICON0.png`, `THUMB` | the same PNG twice |
//! | `PARAM.SFO` | the standard SFO metadata; the world name is its `SUB_TITLE` |
//! | `METADATA` | 256 bytes: sizes and CRC-32s of `GAMEDATA`/`THUMB`, encrypted with a per-save key |
//! | `PARAM.PFD` | version-4 protected-file database signing `PARAM.SFO` and `METADATA` |
//!
//! The layouts and algorithms here were reproduced byte for byte against saves written by a
//! real converter (see the integration test `ps3_resign`). The fixed constants are the standard
//! PFD v4 values published in community PS3 save-tool documentation.
//!
//! This is integrity metadata so a console accepts the save, not secrecy: none of the code is
//! constant-time and the random values come from `std`'s `RandomState`, which is enough because
//! they only need to differ between saves.

use std::path::{Path, PathBuf};

use crate::ps3_crypto::{Aes128, hmac_sha1};
use crate::{Error, bad};

const fn nibble(c: u8) -> u8 {
    match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => panic!("not a hex digit"),
    }
}

const fn hex<const N: usize>(s: &str) -> [u8; N] {
    let b = s.as_bytes();
    assert!(b.len() == 2 * N);
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        out[i] = (nibble(b[2 * i]) << 4) | nibble(b[2 * i + 1]);
        i += 1;
    }
    out
}

// --- Fixed values of the PFD v4 scheme ------------------------------------------------------

/// Encrypts the PFD header and (with [`ENTRY_KEY_IV`]) each entry key.
const HEADER_KEY: [u8; 16] = hex("D413B89663E1FE9F75143D3BB4565274");
/// `real_hash_key = HMAC-SHA1(KEYGEN_KEY, hash_key)`.
const KEYGEN_KEY: [u8; 20] = hex("6B1ACEA246B745FD8F93763B920594CD53483B82");
const ENTRY_KEY_IV: [u8; 16] = hex("EE0B0FA9370ECC5B0AD4D90D55ED2531");
/// The four HMAC keys for the four hashes of `PARAM.SFO` (lengths 20, 0, 16 and 8).
const SFO_HASH_KEY_0: [u8; 20] = hex("0C08000E090504040D010F000406020209060D03");
const SFO_HASH_KEY_2: [u8; 16] = hex("D1C1E10B9C547E689B805DCD9710CE8D");
const SFO_HASH_KEY_3: [u8; 8] = hex("1010000001000003");
/// HMAC key for the single hash of `METADATA`.
const METADATA_HASH_KEY: [u8; 20] = hex("EE0B0FA9370ECC5B0AD4D90D55ED2531FA33BDC4");
/// Constant that ends the decrypted header block.
const HEADER_TAIL: [u8; 4] = [0x79, 0x9d, 0x3f, 0xe1];

// --- PARAM.PFD layout -----------------------------------------------------------------------

/// Size of `PARAM.PFD`.
pub const PFD_SIZE: usize = 0x8000;
const PFD_MAGIC: u64 = 0x5046_4442; // "PFDB"
const PFD_VERSION: u64 = 4;
const CAPACITY: usize = 57;
const RESERVED: usize = 114;
const ENTRY_SIZE: usize = 0x110;
const TABLE_OFFSET: usize = 0x60;
const ENTRIES_OFFSET: usize = 0x240;
const BUCKET_HASHES_OFFSET: usize = ENTRIES_OFFSET + RESERVED * ENTRY_SIZE; // 0x7B60
/// Bucket of `PARAM.SFO` and `METADATA` in the 57-bucket table.
const SFO_BUCKET: usize = 8;
const METADATA_BUCKET: usize = 40;
const NAME_LEN: usize = 65;

/// Region of the title the save is for; it only changes the folder name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ps3Region {
    UsDigital,
    UsDisc,
    EuDigital,
    EuDisc,
    JpDigital,
}

impl Ps3Region {
    pub const ALL: [Ps3Region; 5] = [
        Ps3Region::UsDigital,
        Ps3Region::UsDisc,
        Ps3Region::EuDigital,
        Ps3Region::EuDisc,
        Ps3Region::JpDigital,
    ];

    /// The title ID that starts the save folder name.
    pub fn title_id(self) -> &'static str {
        match self {
            Ps3Region::UsDigital => "NPUB31419",
            Ps3Region::UsDisc => "BLUS31426",
            Ps3Region::EuDigital => "NPEB01899",
            Ps3Region::EuDisc => "BLES01976",
            Ps3Region::JpDigital => "NPJB00539",
        }
    }

    pub fn from_title_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.title_id() == id)
    }
}

/// `<TitleID>--<timestamp>`; the timestamp is `yyMMddHHmmss`, supplied by the caller.
pub fn save_directory_name(region: Ps3Region, timestamp: &str) -> String {
    format!("{}--{}", region.title_id(), timestamp)
}

// --- Random inputs --------------------------------------------------------------------------

/// The values that are random in every save. Fixed for tests, [`Ps3Random::generate`] otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ps3Random {
    /// IV of the encrypted PFD header block.
    pub iv: [u8; 16],
    /// Stored in the header; the real HMAC key is derived from it.
    pub hash_key: [u8; 20],
    /// Plaintext entry keys: `[0]` for `PARAM.SFO`, `[1]` for `METADATA`.
    pub entry_keys: [[u8; 64]; 2],
}

impl Ps3Random {
    pub fn generate() -> Self {
        use std::hash::{BuildHasher, RandomState};
        let state = RandomState::new();
        let mut counter = 0u64;
        let mut fill = |buf: &mut [u8]| {
            for chunk in buf.chunks_mut(8) {
                counter += 1;
                let v = state.hash_one(counter).to_le_bytes();
                chunk.copy_from_slice(&v[..chunk.len()]);
            }
        };
        let mut r = Ps3Random { iv: [0; 16], hash_key: [0; 20], entry_keys: [[0; 64]; 2] };
        fill(&mut r.iv);
        fill(&mut r.hash_key);
        fill(&mut r.entry_keys[0]);
        fill(&mut r.entry_keys[1]);
        r
    }
}

// --- PARAM.SFO ------------------------------------------------------------------------------

const SFO_FMT_UTF8: u16 = 0x0204;
const SFO_FMT_BINARY: u16 = 0x0004;
const SFO_FMT_U32: u16 = 0x0404;
const TITLE: &str = "Minecraft: PlayStation\u{ae}3 Edition";

struct SfoField {
    key: &'static str,
    fmt: u16,
    max: u32,
    data: Vec<u8>,
}

fn utf8_field(key: &'static str, max: u32, text: &str) -> SfoField {
    let mut data = text.as_bytes().to_vec();
    data.push(0);
    SfoField { key, fmt: SFO_FMT_UTF8, max, data }
}

/// Builds `PARAM.SFO` for a save. `directory_name` is the folder name; `world_name` is shown
/// by the console as the sub-title (at most 127 UTF-8 bytes).
pub fn build_param_sfo(directory_name: &str, world_name: &str) -> Result<Vec<u8>, Error> {
    if directory_name.len() >= 64 {
        return Err(bad("PS3 save directory name longer than 63 bytes"));
    }
    if world_name.len() >= 128 {
        return Err(bad("PS3 world name longer than 127 bytes"));
    }
    let mut params = vec![0u8; 1016];
    params[..5].copy_from_slice(&[0x01, 0x05, 0x01, 0x03, 0x03]);
    params[20] = 0x01;
    let fields = [
        SfoField { key: "ACCOUNT_ID", fmt: SFO_FMT_BINARY, max: 16, data: vec![0; 8] },
        SfoField { key: "ATTRIBUTE", fmt: SFO_FMT_U32, max: 4, data: vec![0; 4] },
        utf8_field("CATEGORY", 4, "SD"),
        utf8_field("DETAIL", 1024, " "),
        SfoField { key: "PARAMS", fmt: SFO_FMT_BINARY, max: 1024, data: params },
        SfoField { key: "PARAMS2", fmt: SFO_FMT_BINARY, max: 12, data: vec![0; 12] },
        SfoField { key: "PARENTAL_LEVEL", fmt: SFO_FMT_U32, max: 4, data: vec![0; 4] },
        utf8_field("SAVEDATA_DIRECTORY", 64, directory_name),
        utf8_field("SAVEDATA_LIST_PARAM", 8, "4"),
        utf8_field("SUB_TITLE", 128, world_name),
        utf8_field("TITLE", 128, TITLE),
    ];

    let mut key_table = Vec::new();
    let mut key_offsets = Vec::new();
    for f in &fields {
        key_offsets.push(key_table.len() as u16);
        key_table.extend_from_slice(f.key.as_bytes());
        key_table.push(0);
    }
    while key_table.len() % 4 != 0 {
        key_table.push(0);
    }
    let key_table_start = 20 + 16 * fields.len();
    let data_table_start = key_table_start + key_table.len();

    let mut out = Vec::new();
    out.extend_from_slice(b"\0PSF");
    out.extend_from_slice(&0x101u32.to_le_bytes());
    out.extend_from_slice(&(key_table_start as u32).to_le_bytes());
    out.extend_from_slice(&(data_table_start as u32).to_le_bytes());
    out.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    let mut data_offset = 0u32;
    for (f, ko) in fields.iter().zip(&key_offsets) {
        out.extend_from_slice(&ko.to_le_bytes());
        out.extend_from_slice(&f.fmt.to_le_bytes());
        out.extend_from_slice(&(f.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&f.max.to_le_bytes());
        out.extend_from_slice(&data_offset.to_le_bytes());
        data_offset += f.max;
    }
    out.extend_from_slice(&key_table);
    for f in &fields {
        let mut d = f.data.clone();
        d.resize(f.max as usize, 0);
        out.extend_from_slice(&d);
    }
    Ok(out)
}

// --- Account, console and user of a save ------------------------------------------------------

/// Who a save belongs to, as a PlayStation 3 records it in `PARAM.SFO`: the PSN account, the
/// console (its PSID) and the user on it. All are optional; a save with none of them is written
/// as this converter always did, and a console that checks them may refuse it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ps3Ids {
    /// 16 hexadecimal digits, stored as text in `ACCOUNT_ID`; empty for none.
    pub account_id: String,
    /// 32 hexadecimal digits (16 bytes), stored in `PARAMS`; empty for none.
    pub psid: String,
    /// The user's number on the console (`00000001` is 1), stored in `PARAMS`.
    pub user_id: u32,
}

/// Where the user id, the PSID and the user id again lie in `PARAMS` (as in a save of the game).
const PARAMS_USER: usize = 24;
const PARAMS_PSID: usize = 28;
const PARAMS_USER_AGAIN: usize = 44;

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Ps3Ids {
    /// Whether the fields hold what they should: empty or the right number of hexadecimal digits.
    pub fn check(&self) -> Result<(), Error> {
        if !self.account_id.is_empty() && !is_hex(&self.account_id, 16) {
            return Err(bad("the Account ID is 16 hexadecimal digits"));
        }
        if !self.psid.is_empty() && !is_hex(&self.psid, 32) {
            return Err(bad("the PSID is 32 hexadecimal digits"));
        }
        Ok(())
    }
}

/// Where the data of field `key` lies in a `PARAM.SFO`: its offset and size.
fn sfo_field_range(sfo: &[u8], key: &str) -> Option<(usize, usize)> {
    let word = |at: usize| sfo.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize);
    let (key_table, data_table, count) = (word(8)?, word(12)?, word(16)?);
    for i in 0..count {
        let at = 20 + 16 * i;
        let key_offset = u16::from_le_bytes(sfo.get(at..at + 2)?.try_into().ok()?) as usize;
        let name = sfo.get(key_table + key_offset..)?;
        let name = &name[..name.iter().position(|b| *b == 0)?];
        if name == key.as_bytes() {
            let (max, data_offset) = (word(at + 8)?, word(at + 12)?);
            return (data_table + data_offset + max <= sfo.len()).then_some((data_table + data_offset, max));
        }
    }
    None
}

/// Writes `ids` into a `PARAM.SFO`.
pub fn patch_sfo_ids(sfo: &mut [u8], ids: &Ps3Ids) -> Result<(), Error> {
    ids.check()?;
    let (at, max) = sfo_field_range(sfo, "ACCOUNT_ID").ok_or_else(|| bad("PARAM.SFO has no ACCOUNT_ID"))?;
    sfo[at..at + max].fill(0);
    let text = ids.account_id.to_ascii_uppercase();
    sfo[at..at + text.len().min(max)].copy_from_slice(&text.as_bytes()[..text.len().min(max)]);
    let (at, max) = sfo_field_range(sfo, "PARAMS").ok_or_else(|| bad("PARAM.SFO has no PARAMS"))?;
    if max < PARAMS_USER_AGAIN + 4 {
        return Err(bad("PARAMS is too short"));
    }
    let params = &mut sfo[at..at + max];
    let user = if ids.user_id == 0 && ids.psid.is_empty() { [0u8; 4] } else { ids.user_id.to_le_bytes() };
    params[PARAMS_USER..PARAMS_USER + 4].copy_from_slice(&user);
    params[PARAMS_USER_AGAIN..PARAMS_USER_AGAIN + 4].copy_from_slice(&user);
    let mut psid = [0u8; 16];
    for (i, b) in psid.iter_mut().enumerate() {
        if let Some(h) = ids.psid.get(2 * i..2 * i + 2) {
            *b = u8::from_str_radix(h, 16).unwrap_or(0);
        }
    }
    params[PARAMS_PSID..PARAMS_PSID + 16].copy_from_slice(&psid);
    Ok(())
}

/// What a `PARAM.SFO` says of its account, console and user (empty and 0 where it says none).
pub fn read_sfo_ids(sfo: &[u8]) -> Ps3Ids {
    let mut ids = Ps3Ids::default();
    if let Some((at, max)) = sfo_field_range(sfo, "ACCOUNT_ID") {
        let raw = &sfo[at..at + max];
        let text: String = raw.iter().take_while(|b| **b != 0).map(|b| *b as char).collect();
        if is_hex(&text, 16) {
            ids.account_id = text.to_ascii_uppercase();
        }
    }
    if let Some((at, max)) = sfo_field_range(sfo, "PARAMS") {
        if max >= PARAMS_USER_AGAIN + 4 {
            let params = &sfo[at..at + max];
            ids.user_id = u32::from_le_bytes(params[PARAMS_USER..PARAMS_USER + 4].try_into().unwrap());
            let psid = &params[PARAMS_PSID..PARAMS_PSID + 16];
            if psid.iter().any(|b| *b != 0) {
                ids.psid = psid.iter().map(|b| format!("{b:02X}")).collect();
            }
        }
    }
    ids
}

/// Puts `ids` into the `PARAM.SFO` of a save folder made here. A signed folder (it has a
/// `PARAM.PFD`) is signed again with `random`, the values it was built with.
pub fn apply_ids(folder: &mut Ps3SaveFolder, ids: &Ps3Ids, random: Option<&Ps3Random>) -> Result<(), Error> {
    let get = |folder: &Ps3SaveFolder, n: &str| folder.files.iter().find(|(f, _)| *f == n).map(|(_, b)| b.clone());
    let mut sfo = get(folder, "PARAM.SFO").ok_or_else(|| bad("the folder has no PARAM.SFO"))?;
    patch_sfo_ids(&mut sfo, ids)?;
    let metadata = get(folder, "METADATA");
    let signed = get(folder, "PARAM.PFD").is_some();
    for (n, b) in &mut folder.files {
        match *n {
            "PARAM.SFO" => *b = sfo.clone(),
            "PARAM.PFD" if signed => {
                let (Some(random), Some(m)) = (random, metadata.as_ref()) else { return Err(bad("signing again needs the random values")) };
                let m: [u8; METADATA_SIZE] = m.clone().try_into().map_err(|_| bad("METADATA is not 256 bytes"))?;
                *b = build_param_pfd(&sfo, &m, random);
            }
            _ => {}
        }
    }
    Ok(())
}

// --- METADATA -------------------------------------------------------------------------------

/// Size of `METADATA`.
pub const METADATA_SIZE: usize = 256;

fn crc32(data: &[u8]) -> u32 {
    let mut c = flate2::Crc::new();
    c.update(data);
    c.sum()
}

/// The 256 plaintext bytes: `u32 3`, the sizes and CRC-32s (big-endian) of `GAMEDATA` and the
/// icon, then zeros.
pub fn metadata_plaintext(gamedata_len: u32, icon_len: u32, gamedata_crc: u32, icon_crc: u32) -> [u8; METADATA_SIZE] {
    metadata_plaintext_v(METADATA_VERSION, gamedata_len, icon_len, gamedata_crc, icon_crc)
}

/// The `METADATA` version this converter writes. A genuine 2013 save has version 2.
pub const METADATA_VERSION: u32 = 3;

/// [`metadata_plaintext`] with an explicit version word.
pub fn metadata_plaintext_v(
    version: u32,
    gamedata_len: u32,
    icon_len: u32,
    gamedata_crc: u32,
    icon_crc: u32,
) -> [u8; METADATA_SIZE] {
    let mut p = [0u8; METADATA_SIZE];
    for (i, v) in [version, gamedata_len, icon_len, gamedata_crc, icon_crc].into_iter().enumerate() {
        p[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    p
}

/// Encrypts the metadata plaintext. The key is the first 16 bytes of the plaintext entry key.
/// Block *n* is `E(K, plain XOR E(K, n as big-endian u64 followed by 8 zero bytes))`.
pub fn encrypt_metadata(plain: &[u8; METADATA_SIZE], entry_key: &[u8; 64]) -> [u8; METADATA_SIZE] {
    let aes = Aes128::new(entry_key[..16].try_into().unwrap());
    let mut out = [0u8; METADATA_SIZE];
    for n in 0..METADATA_SIZE / 16 {
        let mut counter = [0u8; 16];
        counter[..8].copy_from_slice(&(n as u64).to_be_bytes());
        let tweak = aes.encrypt_block(&counter);
        let mut block = [0u8; 16];
        for i in 0..16 {
            block[i] = plain[16 * n + i] ^ tweak[i];
        }
        out[16 * n..16 * n + 16].copy_from_slice(&aes.encrypt_block(&block));
    }
    out
}

/// `METADATA` for a save with these `GAMEDATA` and icon bytes.
pub fn build_metadata(gamedata: &[u8], icon_png: &[u8], entry_key: &[u8; 64]) -> [u8; METADATA_SIZE] {
    encrypt_metadata(
        &metadata_plaintext(gamedata.len() as u32, icon_png.len() as u32, crc32(gamedata), crc32(icon_png)),
        entry_key,
    )
}

// --- PARAM.PFD ------------------------------------------------------------------------------

fn entry_bytes(name: &str, key_enc: &[u8], hashes: &[[u8; 20]; 4], size: u64) -> (Vec<u8>, Vec<u8>) {
    // The bucket hash covers the entry from `file_name` on, without the leading index and the
    // 7 padding bytes after the name: name[65] key[64] hashes[80] 40 zero size[8] = 257 bytes.
    let mut body = vec![0u8; NAME_LEN];
    body[..name.len()].copy_from_slice(name.as_bytes());
    body.extend_from_slice(key_enc);
    for h in hashes {
        body.extend_from_slice(h);
    }
    body.extend_from_slice(&[0u8; 0x28]);
    body.extend_from_slice(&size.to_be_bytes());

    let mut entry = Vec::with_capacity(ENTRY_SIZE);
    entry.extend_from_slice(&(RESERVED as u64).to_be_bytes());
    entry.extend_from_slice(&body[..NAME_LEN]);
    entry.extend_from_slice(&[0u8; 7]);
    entry.extend_from_slice(&body[NAME_LEN..]);
    debug_assert_eq!(entry.len(), ENTRY_SIZE);
    (entry, body)
}

fn sfo_hashes(sfo: &[u8]) -> [[u8; 20]; 4] {
    [
        hmac_sha1(&SFO_HASH_KEY_0, sfo),
        hmac_sha1(&[], sfo),
        hmac_sha1(&SFO_HASH_KEY_2, sfo),
        hmac_sha1(&SFO_HASH_KEY_3, sfo),
    ]
}

fn metadata_hashes(metadata: &[u8]) -> [[u8; 20]; 4] {
    [hmac_sha1(&METADATA_HASH_KEY, metadata), [0; 20], [0; 20], [0; 20]]
}

fn real_key(hash_key: &[u8; 20]) -> [u8; 20] {
    hmac_sha1(&KEYGEN_KEY, hash_key)
}

/// Builds the 32,768-byte `PARAM.PFD` signing `sfo` and `metadata`.
pub fn build_param_pfd(sfo: &[u8], metadata: &[u8; METADATA_SIZE], random: &Ps3Random) -> Vec<u8> {
    let key = real_key(&random.hash_key);
    let entry_aes = Aes128::new(&HEADER_KEY);

    let mut index = [RESERVED as u64; CAPACITY];
    index[SFO_BUCKET] = 0;
    index[METADATA_BUCKET] = 1;
    let mut table = Vec::with_capacity(24 + 8 * CAPACITY);
    for v in [CAPACITY as u64, RESERVED as u64, 2] {
        table.extend_from_slice(&v.to_be_bytes());
    }
    for v in index {
        table.extend_from_slice(&v.to_be_bytes());
    }

    let enc_key = |k: &[u8; 64]| entry_aes.cbc_encrypt(&ENTRY_KEY_IV, k);
    let (sfo_entry, sfo_body) = entry_bytes("PARAM.SFO", &enc_key(&random.entry_keys[0]), &sfo_hashes(sfo), sfo.len() as u64);
    let (meta_entry, meta_body) =
        entry_bytes("METADATA", &enc_key(&random.entry_keys[1]), &metadata_hashes(metadata), metadata.len() as u64);

    let empty_bucket = hmac_sha1(&key, &[]);
    let mut buckets = Vec::with_capacity(CAPACITY * 20);
    for b in 0..CAPACITY {
        match b {
            SFO_BUCKET => buckets.extend_from_slice(&hmac_sha1(&key, &sfo_body)),
            METADATA_BUCKET => buckets.extend_from_slice(&hmac_sha1(&key, &meta_body)),
            _ => buckets.extend_from_slice(&empty_bucket),
        }
    }

    let bottom = hmac_sha1(&key, &buckets);
    let top = hmac_sha1(&key, &table);
    let mut header_plain = Vec::with_capacity(64);
    header_plain.extend_from_slice(&bottom);
    header_plain.extend_from_slice(&top);
    header_plain.extend_from_slice(&random.hash_key);
    header_plain.extend_from_slice(&HEADER_TAIL);

    let mut out = Vec::with_capacity(PFD_SIZE);
    out.extend_from_slice(&PFD_MAGIC.to_be_bytes());
    out.extend_from_slice(&PFD_VERSION.to_be_bytes());
    out.extend_from_slice(&random.iv);
    out.extend_from_slice(&Aes128::new(&HEADER_KEY).cbc_encrypt(&random.iv, &header_plain));
    out.extend_from_slice(&table);
    out.extend_from_slice(&sfo_entry);
    out.extend_from_slice(&meta_entry);
    out.resize(BUCKET_HASHES_OFFSET, 0);
    out.extend_from_slice(&buckets);
    out.resize(PFD_SIZE, 0);
    out
}

/// What a verified `PARAM.PFD` held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ps3PfdContents {
    pub random: Ps3Random,
}

/// Checks a `PARAM.PFD` against the `PARAM.SFO` and `METADATA` next to it: the header block, the
/// two entries and their hashes, every bucket hash and the bottom and top hashes. Returns the
/// random values it was built from (decrypted), which is also how a save can be re-signed.
pub fn verify_param_pfd(pfd: &[u8], sfo: &[u8], metadata: &[u8]) -> Result<Ps3PfdContents, Error> {
    if pfd.len() != PFD_SIZE {
        return Err(bad(format!("PARAM.PFD is {} bytes, expected {PFD_SIZE}", pfd.len())));
    }
    if u64::from_be_bytes(pfd[0..8].try_into().unwrap()) != PFD_MAGIC {
        return Err(bad("PARAM.PFD has no PFDB magic"));
    }
    let version = u64::from_be_bytes(pfd[8..16].try_into().unwrap());
    if version != PFD_VERSION {
        return Err(bad(format!("PARAM.PFD version {version}, only {PFD_VERSION} is supported")));
    }
    let iv: [u8; 16] = pfd[0x10..0x20].try_into().unwrap();
    // The last 4 header bytes are not checked: this converter writes a fixed value there, a genuine
    // save (2013) has another. The top and bottom hashes below prove the header decrypted correctly.
    let header = Aes128::new(&HEADER_KEY).cbc_decrypt(&iv, &pfd[0x20..0x60]);
    let bottom = &header[0..20];
    let top = &header[20..40];
    let hash_key: [u8; 20] = header[40..60].try_into().unwrap();
    let key = real_key(&hash_key);

    let read_u64 = |off: usize| u64::from_be_bytes(pfd[off..off + 8].try_into().unwrap());
    if (read_u64(TABLE_OFFSET), read_u64(TABLE_OFFSET + 8), read_u64(TABLE_OFFSET + 16)) != (CAPACITY as u64, RESERVED as u64, 2)
    {
        return Err(bad("PARAM.PFD hash table is not 57 buckets, 114 reserved, 2 used"));
    }
    for b in 0..CAPACITY {
        let want = match b {
            SFO_BUCKET => 0,
            METADATA_BUCKET => 1,
            _ => RESERVED as u64,
        };
        if read_u64(TABLE_OFFSET + 24 + 8 * b) != want {
            return Err(bad(format!("PARAM.PFD bucket {b} does not hold {want}")));
        }
    }
    let table_end = TABLE_OFFSET + 24 + 8 * CAPACITY;
    if top != hmac_sha1(&key, &pfd[TABLE_OFFSET..table_end]) {
        return Err(bad("PARAM.PFD top hash is wrong (header does not decrypt, or the table was edited)"));
    }
    let buckets = &pfd[BUCKET_HASHES_OFFSET..BUCKET_HASHES_OFFSET + CAPACITY * 20];
    if bottom != hmac_sha1(&key, buckets) {
        return Err(bad("PARAM.PFD bottom hash is wrong"));
    }

    let entry_aes = Aes128::new(&HEADER_KEY);
    let mut entry_keys = [[0u8; 64]; 2];
    struct Signed<'a> {
        name: &'a str,
        data: &'a [u8],
        bucket: usize,
        hashes: [[u8; 20]; 4],
    }
    let files = [
        Signed { name: "PARAM.SFO", data: sfo, bucket: SFO_BUCKET, hashes: sfo_hashes(sfo) },
        Signed { name: "METADATA", data: metadata, bucket: METADATA_BUCKET, hashes: metadata_hashes(metadata) },
    ];
    for (i, Signed { name, data, bucket, hashes }) in files.iter().enumerate() {
        let off = ENTRIES_OFFSET + i * ENTRY_SIZE;
        let entry = &pfd[off..off + ENTRY_SIZE];
        let stored_name = &entry[8..8 + NAME_LEN];
        if !stored_name.starts_with(name.as_bytes()) || stored_name[name.len()..].iter().any(|b| *b != 0) {
            return Err(bad(format!("PARAM.PFD entry {i} is not {name}")));
        }
        let key_enc = &entry[0x50..0x90];
        for (j, h) in hashes.iter().enumerate() {
            if entry[0x90 + 20 * j..0xa4 + 20 * j] != h[..] {
                return Err(bad(format!("PARAM.PFD hash {j} of {name} does not match the file")));
            }
        }
        if u64::from_be_bytes(entry[0x108..0x110].try_into().unwrap()) != data.len() as u64 {
            return Err(bad(format!("PARAM.PFD size of {name} does not match the file")));
        }
        let mut body = entry[8..8 + NAME_LEN].to_vec();
        body.extend_from_slice(&entry[0x50..0x108]);
        body.extend_from_slice(&entry[0x108..0x110]);
        if buckets[20 * bucket..20 * bucket + 20] != hmac_sha1(&key, &body) {
            return Err(bad(format!("PARAM.PFD bucket hash of {name} is wrong")));
        }
        entry_keys[i] = entry_aes.cbc_decrypt(&ENTRY_KEY_IV, key_enc).try_into().unwrap();
    }
    Ok(Ps3PfdContents { random: Ps3Random { iv, hash_key, entry_keys } })
}

/// Reads the random values out of a `PARAM.PFD` **without** checking the file hashes of its
/// entries, so it also works on a save signed with other (console-bound) keys, such as a genuine
/// console save. It still requires the header to decrypt (the top and bottom hashes must match) and
/// the table to have the usual shape.
pub fn read_pfd_random(pfd: &[u8]) -> Result<Ps3Random, Error> {
    if pfd.len() != PFD_SIZE || u64::from_be_bytes(pfd[0..8].try_into().unwrap()) != PFD_MAGIC {
        return Err(bad("not a PARAM.PFD"));
    }
    if u64::from_be_bytes(pfd[8..16].try_into().unwrap()) != PFD_VERSION {
        return Err(bad("PARAM.PFD is not version 4"));
    }
    let iv: [u8; 16] = pfd[0x10..0x20].try_into().unwrap();
    let aes = Aes128::new(&HEADER_KEY);
    let header = aes.cbc_decrypt(&iv, &pfd[0x20..0x60]);
    let hash_key: [u8; 20] = header[40..60].try_into().unwrap();
    let key = real_key(&hash_key);
    let table_end = TABLE_OFFSET + 24 + 8 * CAPACITY;
    if header[20..40] != hmac_sha1(&key, &pfd[TABLE_OFFSET..table_end])
        || header[..20] != hmac_sha1(&key, &pfd[BUCKET_HASHES_OFFSET..BUCKET_HASHES_OFFSET + CAPACITY * 20])
    {
        return Err(bad("PARAM.PFD header does not decrypt or its hashes are wrong"));
    }
    let mut entry_keys = [[0u8; 64]; 2];
    for (i, k) in entry_keys.iter_mut().enumerate() {
        let off = ENTRIES_OFFSET + i * ENTRY_SIZE + 0x50;
        *k = aes.cbc_decrypt(&ENTRY_KEY_IV, &pfd[off..off + 64]).try_into().unwrap();
    }
    Ok(Ps3Random { iv, hash_key, entry_keys })
}

/// Reverses [`encrypt_metadata`]: the 256 plaintext bytes of a `METADATA` file.
pub fn decrypt_metadata(metadata: &[u8; METADATA_SIZE], entry_key: &[u8; 64]) -> [u8; METADATA_SIZE] {
    let aes = Aes128::new(entry_key[..16].try_into().unwrap());
    let mut out = [0u8; METADATA_SIZE];
    for n in 0..METADATA_SIZE / 16 {
        let mut counter = [0u8; 16];
        counter[..8].copy_from_slice(&(n as u64).to_be_bytes());
        let tweak = aes.encrypt_block(&counter);
        let block: [u8; 16] = metadata[16 * n..16 * n + 16].try_into().unwrap();
        let plain = aes.decrypt_block(&block);
        for i in 0..16 {
            out[16 * n + i] = plain[i] ^ tweak[i];
        }
    }
    out
}

/// Re-signs a save folder whose `GAMEDATA` changed (edited, pruned, converted...): rewrites `METADATA`
/// for the new `GAMEDATA` and the existing `THUMB`, and `PARAM.PFD` over the existing `PARAM.SFO`.
///
/// If the folder's `PARAM.PFD` and `METADATA` can be read (this also holds for a genuine console save)
/// their random values and the `METADATA` version are kept, so a save re-signed twice stays stable;
/// otherwise fresh random values and version 3 are used. The file hashes in the new `PARAM.PFD` use
/// this converter's fixed keys, not the keys bound to a console or user.
pub fn resign_save_folder(dir: impl AsRef<Path>) -> Result<(), Error> {
    let dir = dir.as_ref();
    let read = |n: &str| std::fs::read(dir.join(n)).map_err(|e| bad(format!("{n}: {e}")));
    let gamedata = read("GAMEDATA")?;
    let sfo = read("PARAM.SFO")?;
    let thumb = read("THUMB")?;
    let old_random = read("PARAM.PFD").ok().and_then(|p| read_pfd_random(&p).ok());
    let random = old_random.clone().unwrap_or_else(Ps3Random::generate);
    let version = old_random
        .as_ref()
        .zip(read("METADATA").ok().and_then(|m| <[u8; METADATA_SIZE]>::try_from(m).ok()))
        .map(|(r, m)| u32::from_be_bytes(decrypt_metadata(&m, &r.entry_keys[1])[..4].try_into().unwrap()))
        .filter(|v| (1..=16).contains(v))
        .unwrap_or(METADATA_VERSION);
    let plain = metadata_plaintext_v(
        version,
        gamedata.len() as u32,
        thumb.len() as u32,
        crc32(&gamedata),
        crc32(&thumb),
    );
    let metadata = encrypt_metadata(&plain, &random.entry_keys[1]);
    let pfd = build_param_pfd(&sfo, &metadata, &random);
    std::fs::write(dir.join("METADATA"), metadata)?;
    std::fs::write(dir.join("PARAM.PFD"), pfd)?;
    Ok(())
}

// --- Whole save folder ----------------------------------------------------------------------

/// A PS3 save folder held in memory.
#[derive(Debug, Clone)]
pub struct Ps3SaveFolder {
    pub directory_name: String,
    /// `(file name, bytes)` in the order the converter writes them.
    pub files: Vec<(&'static str, Vec<u8>)>,
}

/// Assembles a save folder. `gamedata` is the encoded save archive, `icon_png` the `ICON0.png`
/// shown on the console's menu (a genuine save: 320x176) and `thumb_png` the world thumbnail `THUMB`
/// that `METADATA` records (genuine: 64x64; this converter used the same 64x64 PNG for both);
/// `timestamp` is `yyMMddHHmmss`.
pub fn build_save_folder(
    gamedata: Vec<u8>,
    icon_png: Vec<u8>,
    thumb_png: Vec<u8>,
    region: Ps3Region,
    world_name: &str,
    timestamp: &str,
    random: &Ps3Random,
) -> Result<Ps3SaveFolder, Error> {
    if timestamp.len() != 12 || !timestamp.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("PS3 save timestamp must be 12 digits (yyMMddHHmmss)"));
    }
    let directory_name = save_directory_name(region, timestamp);
    let sfo = build_param_sfo(&directory_name, world_name)?;
    let metadata = build_metadata(&gamedata, &thumb_png, &random.entry_keys[1]);
    let pfd = build_param_pfd(&sfo, &metadata, random);
    Ok(Ps3SaveFolder {
        directory_name,
        files: vec![
            ("GAMEDATA", gamedata),
            ("ICON0.png", icon_png),
            ("METADATA", metadata.to_vec()),
            ("PARAM.PFD", pfd),
            ("PARAM.SFO", sfo),
            ("THUMB", thumb_png),
        ],
    })
}

/// A save folder for an emulator such as RPCS3, which cannot read the console's protected files: the same files
/// as [`build_save_folder`], but `METADATA` holds its plaintext (a real console decrypts it with the key in
/// `PARAM.PFD` before the game sees it; an emulator hands the game the file as it is) and there is no
/// `PARAM.PFD`. A real console will not accept this folder.
pub fn build_save_folder_unencrypted(
    gamedata: Vec<u8>,
    icon_png: Vec<u8>,
    thumb_png: Vec<u8>,
    region: Ps3Region,
    world_name: &str,
    timestamp: &str,
) -> Result<Ps3SaveFolder, Error> {
    let mut folder = build_save_folder(gamedata, icon_png, thumb_png, region, world_name, timestamp, &Ps3Random::generate())?;
    let plain = {
        let get = |n: &str| folder.files.iter().find(|(f, _)| *f == n).map(|(_, b)| b.clone()).unwrap_or_default();
        let (gamedata, thumb) = (get("GAMEDATA"), get("THUMB"));
        metadata_plaintext(gamedata.len() as u32, thumb.len() as u32, crc32(&gamedata), crc32(&thumb))
    };
    folder.files.retain(|(n, _)| *n != "PARAM.PFD");
    for (n, b) in &mut folder.files {
        if *n == "METADATA" {
            *b = plain.to_vec();
        }
    }
    Ok(folder)
}

/// Turns a signed save folder (made by this converter or a genuine one) into the unencrypted form of
/// [`build_save_folder_unencrypted`], in place: `METADATA` is decrypted with the entry key from `PARAM.PFD`,
/// which is then removed. A folder that is already unencrypted (no `PARAM.PFD`) is left as it is.
pub fn unprotect_save_folder(dir: impl AsRef<Path>) -> Result<(), Error> {
    let dir = dir.as_ref();
    let Ok(pfd) = std::fs::read(dir.join("PARAM.PFD")) else { return Ok(()) };
    let random = read_pfd_random(&pfd)?;
    let meta: [u8; METADATA_SIZE] =
        std::fs::read(dir.join("METADATA"))?.try_into().map_err(|_| bad("METADATA is not 256 bytes"))?;
    std::fs::write(dir.join("METADATA"), decrypt_metadata(&meta, &random.entry_keys[1]))?;
    std::fs::remove_file(dir.join("PARAM.PFD"))?;
    Ok(())
}

/// Checks a save folder on disk: `PARAM.PFD` against `PARAM.SFO` and `METADATA`, and `METADATA`
/// against the sizes and CRC-32s of `GAMEDATA` and `THUMB` (with whatever version word it carries).
/// Returns the size of `GAMEDATA`. Reads files only. `ICON0.png` is not compared with `THUMB`: they
/// are the same file in this converter's saves but not in genuine ones.
pub fn verify_save_folder(dir: impl AsRef<Path>) -> Result<usize, Error> {
    let dir = dir.as_ref();
    let read = |n: &str| std::fs::read(dir.join(n)).map_err(|e| bad(format!("{n}: {e}")));
    let (pfd, sfo, meta) = (read("PARAM.PFD")?, read("PARAM.SFO")?, read("METADATA")?);
    let (gamedata, thumb) = (read("GAMEDATA")?, read("THUMB")?);
    let contents = verify_param_pfd(&pfd, &sfo, &meta)?;
    let meta: [u8; METADATA_SIZE] = meta.try_into().map_err(|_| bad("METADATA is not 256 bytes"))?;
    let plain = decrypt_metadata(&meta, &contents.random.entry_keys[1]);
    let version = u32::from_be_bytes(plain[..4].try_into().unwrap());
    let expect = metadata_plaintext_v(version, gamedata.len() as u32, thumb.len() as u32, crc32(&gamedata), crc32(&thumb));
    if plain != expect {
        return Err(bad("METADATA does not match the sizes and CRCs of GAMEDATA and THUMB"));
    }
    Ok(gamedata.len())
}

/// A time as the console names its saves: UTC, `yyMMddHHmmss`.
pub fn utc_timestamp(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{:02}{:02}{:02}{:02}{:02}{:02}", year.rem_euclid(100), month, day, rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Writes the folder under `parent` (for a USB drive: `<drive>/PS3/SAVEDATA`) and returns its path.
pub fn write_save_folder(parent: impl AsRef<Path>, folder: &Ps3SaveFolder) -> Result<PathBuf, Error> {
    let dir = parent.as_ref().join(&folder.directory_name);
    std::fs::create_dir_all(&dir)?;
    for (name, bytes) in &folder.files {
        std::fs::write(dir.join(name), bytes)?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sfo_layout_is_regular() {
        let sfo = build_param_sfo("NPUB31419--261121141221", "ps3").unwrap();
        assert_eq!(sfo.len(), 2736);
        assert_eq!(&sfo[..4], b"\0PSF");
        assert!(build_param_sfo("x", &"a".repeat(128)).is_err());
    }

    #[test]
    fn the_account_console_and_user_go_into_the_sfo_and_come_back() {
        let ids = Ps3Ids { account_id: "0123456789abcdef".into(), psid: "5BEDF6D4A97E1A25E7879E6C1BF426B0".into(), user_id: 402 };
        let mut sfo = build_param_sfo("NPUB31419--261121141221", "w").unwrap();
        assert_eq!(read_sfo_ids(&sfo), Ps3Ids::default());
        patch_sfo_ids(&mut sfo, &ids).unwrap();
        assert_eq!(sfo.len(), 2736);
        assert_eq!(read_sfo_ids(&sfo), Ps3Ids { account_id: "0123456789ABCDEF".into(), ..ids.clone() });
        assert!(patch_sfo_ids(&mut sfo, &Ps3Ids { psid: "12".into(), ..Ps3Ids::default() }).is_err());
        // A signed folder is signed again over the new PARAM.SFO.
        let random = Ps3Random::generate();
        let mut folder = build_save_folder(vec![7; 300], vec![9; 50], vec![8; 20], Ps3Region::UsDigital, "W", "261121120000", &random).unwrap();
        apply_ids(&mut folder, &ids, Some(&random)).unwrap();
        let get = |n: &str| folder.files.iter().find(|(f, _)| *f == n).unwrap().1.clone();
        assert!(verify_param_pfd(&get("PARAM.PFD"), &get("PARAM.SFO"), &get("METADATA")).is_ok());
        assert_eq!(read_sfo_ids(&get("PARAM.SFO")).user_id, 402);
    }

    #[test]
    fn utc_timestamp_known_instants() {
        use std::time::{Duration, UNIX_EPOCH};
        assert_eq!(utc_timestamp(UNIX_EPOCH), "700101000000");
        // 2026-11-21 14:12:21 UTC: the stamp on the real saves used as fixtures.
        assert_eq!(utc_timestamp(UNIX_EPOCH + Duration::from_secs(1_795_270_341)), "261121141221");
        assert_eq!(utc_timestamp(UNIX_EPOCH + Duration::from_secs(951_782_400)), "000229000000"); // leap day
    }

    #[test]
    fn region_title_ids_roundtrip() {
        for r in Ps3Region::ALL {
            assert_eq!(Ps3Region::from_title_id(r.title_id()), Some(r));
        }
    }

    #[test]
    fn rejects_bad_timestamp() {
        let r = Ps3Random::generate();
        assert!(build_save_folder(vec![1], vec![2], vec![3], Ps3Region::UsDigital, "w", "2611", &r).is_err());
    }

    #[test]
    fn generated_folder_verifies() {
        let r = Ps3Random::generate();
        let f = build_save_folder(vec![7; 1000], vec![9; 50], vec![8; 20], Ps3Region::EuDisc, "My World", "261121120000", &r)
            .unwrap();
        assert_eq!(f.directory_name, "BLES01976--261121120000");
        let get = |n: &str| f.files.iter().find(|(k, _)| *k == n).unwrap().1.clone();
        let contents = verify_param_pfd(&get("PARAM.PFD"), &get("PARAM.SFO"), &get("METADATA")).unwrap();
        assert_eq!(contents.random, r);
        // A single flipped byte anywhere that is signed must be caught.
        let mut bad_sfo = get("PARAM.SFO");
        bad_sfo[2000] ^= 1;
        assert!(verify_param_pfd(&get("PARAM.PFD"), &bad_sfo, &get("METADATA")).is_err());
    }

    #[test]
    fn unencrypted_folder_has_plain_metadata_and_no_pfd() {
        let f = build_save_folder_unencrypted(vec![7; 1000], vec![9; 50], vec![8; 20], Ps3Region::UsDigital, "W", "261121120000").unwrap();
        assert!(f.files.iter().all(|(n, _)| *n != "PARAM.PFD"));
        let meta = &f.files.iter().find(|(n, _)| *n == "METADATA").unwrap().1;
        assert_eq!(meta.as_slice(), metadata_plaintext(1000, 20, crc32(&[7; 1000]), crc32(&[8; 20])).as_slice());
    }

    #[test]
    fn unprotect_matches_the_unencrypted_build() {
        let dir = std::env::temp_dir().join(format!("mctk-unprotect-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let r = Ps3Random::generate();
        let signed = build_save_folder(vec![7; 300], vec![9; 50], vec![8; 20], Ps3Region::UsDigital, "W", "261121120000", &r).unwrap();
        let saved = write_save_folder(&dir, &signed).unwrap();
        unprotect_save_folder(&saved).unwrap();
        assert!(!saved.join("PARAM.PFD").exists());
        assert_eq!(std::fs::read(saved.join("METADATA")).unwrap(), metadata_plaintext(300, 20, crc32(&[7; 300]), crc32(&[8; 20])).to_vec());
        // a second run changes nothing
        unprotect_save_folder(&saved).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
