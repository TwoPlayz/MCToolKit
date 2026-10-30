//! A LevelDB implementation for Minecraft Bedrock worlds.
//!
//! Bedrock stores chunks in Mojang's LevelDB fork, which adds zlib compression
//! (block compression types 2 = zlib, 4 = raw deflate) to the on-disk format described in
//! LevelDB's `doc/table_format.md` and `doc/log_format.md`.
//!
//! [`Db`] loads a whole database into memory: it replays the MANIFEST to find live table
//! files, reads every table and every write-ahead log, and resolves sequence numbers and
//! deletions. [`write_db`] writes a fresh, fully compacted database that LevelDB (and
//! Minecraft) can open.

mod coding;
mod crc32c;
mod log;
mod manifest;
mod table;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;

pub use table::BlockCompression;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt database: {0}")]
    Corrupt(String),
    #[error("block compression type {0} is not supported")]
    UnsupportedCompression(u8),
}

pub(crate) fn corrupt(msg: impl Into<String>) -> Error {
    Error::Corrupt(msg.into())
}

/// Value type stored in an internal key's tag byte.
const TYPE_DELETION: u8 = 0;
const TYPE_VALUE: u8 = 1;

/// A database loaded into memory as a sorted map of live keys.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Db {
    pub entries: BTreeMap<Vec<u8>, Vec<u8>>,
    /// Highest sequence number seen.
    pub last_sequence: u64,
}

struct Versioned {
    seq: u64,
    value: Option<Vec<u8>>,
}

impl Db {
    /// Loads the database in `dir` (the folder holding `CURRENT`).
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        let mut files = HashMap::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                files.insert(
                    entry.file_name().to_string_lossy().into_owned(),
                    std::fs::read(entry.path())?,
                );
            }
        }
        Db::from_files(&files)
    }

    /// Loads a database from its files, keyed by file name (e.g. from a zip archive).
    pub fn from_files(files: &HashMap<String, Vec<u8>>) -> Result<Self, Error> {
        let current = files
            .get("CURRENT")
            .ok_or_else(|| corrupt("missing CURRENT"))?;
        let manifest_name = std::str::from_utf8(current)
            .map_err(|_| corrupt("CURRENT is not text"))?
            .trim()
            .to_string();
        let manifest = files
            .get(&manifest_name)
            .ok_or_else(|| corrupt(format!("missing {manifest_name}")))?;
        let state = manifest::replay(manifest)?;

        let mut merged: HashMap<Vec<u8>, Versioned> = HashMap::new();
        let mut last_sequence = state.last_sequence;
        let mut apply = |key: Vec<u8>, seq: u64, value: Option<Vec<u8>>| {
            last_sequence = last_sequence.max(seq);
            match merged.get(&key) {
                Some(existing) if existing.seq >= seq => {}
                _ => {
                    merged.insert(key, Versioned { seq, value });
                }
            }
        };

        for number in &state.live_files {
            let data = [format!("{number:06}.ldb"), format!("{number:06}.sst")]
                .iter()
                .find_map(|n| files.get(n))
                .ok_or_else(|| corrupt(format!("missing table {number:06}")))?;
            for (ikey, value) in table::read_table(data)? {
                let (user, seq, kind) = split_internal_key(&ikey)?;
                apply(user.to_vec(), seq, (kind == TYPE_VALUE).then_some(value));
            }
        }

        // Logs at or after the manifest's log number hold writes not yet in tables.
        let mut logs: Vec<(u64, &Vec<u8>)> = files
            .iter()
            .filter_map(|(name, data)| {
                Some((name.strip_suffix(".log")?.parse::<u64>().ok()?, data))
            })
            .filter(|(n, _)| *n >= state.log_number || Some(*n) == state.prev_log_number)
            .collect();
        logs.sort_by_key(|(n, _)| *n);
        for (_, data) in logs {
            for record in log::read_records(data)? {
                for (key, seq, value) in log::parse_write_batch(&record)? {
                    apply(key, seq, value);
                }
            }
        }

        let entries = merged
            .into_iter()
            .filter_map(|(k, v)| Some((k, v.value?)))
            .collect();
        Ok(Db {
            entries,
            last_sequence,
        })
    }

    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.entries.get(key).map(Vec::as_slice)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn split_internal_key(ikey: &[u8]) -> Result<(&[u8], u64, u8), Error> {
    if ikey.len() < 8 {
        return Err(corrupt("internal key shorter than 8 bytes"));
    }
    let (user, tag) = ikey.split_at(ikey.len() - 8);
    let tag = u64::from_le_bytes(tag.try_into().unwrap());
    Ok((user, tag >> 8, (tag & 0xff) as u8))
}

/// Options for [`write_db`].
#[derive(Debug, Clone, Copy)]
pub struct WriteOptions {
    pub compression: BlockCompression,
    /// Uncompressed size at which a data block is closed.
    pub block_size: usize,
    /// Uncompressed size at which a table file is closed.
    pub table_size: usize,
}

impl Default for WriteOptions {
    fn default() -> Self {
        // Mojang's Bedrock builds use raw deflate blocks.
        WriteOptions {
            compression: BlockCompression::RawDeflate,
            block_size: 160 * 1024,
            table_size: 2 * 1024 * 1024,
        }
    }
}

/// Writes `entries` as a new, compacted database into the empty or missing folder `dir`.
pub fn write_db(
    dir: impl AsRef<Path>,
    entries: &BTreeMap<Vec<u8>, Vec<u8>>,
    options: WriteOptions,
) -> Result<(), Error> {
    let dir = dir.as_ref();
    std::fs::create_dir_all(dir)?;
    if std::fs::read_dir(dir)?.next().is_some() {
        return Err(Error::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "target folder is not empty",
        )));
    }
    for (name, bytes) in build_db_files(entries, options)? {
        std::fs::write(dir.join(name), bytes)?;
    }
    Ok(())
}

/// Builds the files of a fresh database in memory, keyed by file name.
pub fn build_db_files(
    entries: &BTreeMap<Vec<u8>, Vec<u8>>,
    options: WriteOptions,
) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let mut files = Vec::new();
    let mut tables = Vec::new();
    let mut next_file = 2u64;
    let mut seq = 0u64;
    let mut builder = table::TableBuilder::new(options);
    let mut smallest: Option<Vec<u8>> = None;
    let mut largest = Vec::new();

    let mut finish = |builder: &mut table::TableBuilder,
                      smallest: &mut Option<Vec<u8>>,
                      largest: &[u8],
                      files: &mut Vec<(String, Vec<u8>)>,
                      next_file: &mut u64| {
        let Some(small) = smallest.take() else { return };
        let bytes = std::mem::replace(builder, table::TableBuilder::new(options)).finish();
        tables.push(manifest::FileMeta {
            number: *next_file,
            size: bytes.len() as u64,
            smallest: small,
            largest: largest.to_vec(),
        });
        files.push((format!("{:06}.ldb", *next_file), bytes));
        *next_file += 1;
    };

    for (key, value) in entries {
        seq += 1;
        let mut ikey = key.clone();
        ikey.extend_from_slice(&((seq << 8) | TYPE_VALUE as u64).to_le_bytes());
        if smallest.is_none() {
            smallest = Some(ikey.clone());
        }
        builder.add(&ikey, value);
        largest = ikey;
        if builder.estimated_size() >= options.table_size {
            finish(
                &mut builder,
                &mut smallest,
                &largest,
                &mut files,
                &mut next_file,
            );
        }
    }
    finish(
        &mut builder,
        &mut smallest,
        &largest,
        &mut files,
        &mut next_file,
    );

    let manifest_number = next_file;
    let edit = manifest::VersionEdit {
        comparator: Some("leveldb.BytewiseComparator".into()),
        log_number: Some(manifest_number + 1),
        prev_log_number: Some(0),
        next_file_number: Some(manifest_number + 2),
        last_sequence: Some(seq),
        // Tables written here never overlap, so they can sit in level 1.
        new_files: tables.into_iter().map(|t| (1, t)).collect(),
        ..Default::default()
    };
    let manifest_name = format!("MANIFEST-{manifest_number:06}");
    files.push((manifest_name.clone(), log::write_records(&[edit.encode()])));
    files.push(("CURRENT".into(), format!("{manifest_name}\n").into_bytes()));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(n: usize) -> BTreeMap<Vec<u8>, Vec<u8>> {
        (0..n)
            .map(|i| {
                (
                    format!("key-{i:08}").into_bytes(),
                    vec![(i % 251) as u8; i % 700],
                )
            })
            .collect()
    }

    fn as_map(files: Vec<(String, Vec<u8>)>) -> HashMap<String, Vec<u8>> {
        files.into_iter().collect()
    }

    #[test]
    fn writes_and_reads_back_across_blocks_and_tables() {
        let entries = sample(20_000);
        for compression in [
            BlockCompression::None,
            BlockCompression::Zlib,
            BlockCompression::RawDeflate,
        ] {
            let options = WriteOptions {
                compression,
                block_size: 4096,
                table_size: 1 << 20,
            };
            let files = build_db_files(&entries, options).unwrap();
            assert!(
                files.iter().filter(|(n, _)| n.ends_with(".ldb")).count() > 1,
                "expected several tables"
            );
            let db = Db::from_files(&as_map(files)).unwrap();
            assert_eq!(db.entries, entries, "{compression:?}");
            assert_eq!(db.last_sequence, entries.len() as u64);
        }
    }

    #[test]
    fn empty_database() {
        let db = Db::from_files(&as_map(
            build_db_files(&BTreeMap::new(), WriteOptions::default()).unwrap(),
        ))
        .unwrap();
        assert!(db.is_empty());
    }

    #[test]
    fn write_ahead_log_overrides_tables() {
        let entries = sample(100);
        let mut files = as_map(build_db_files(&entries, WriteOptions::default()).unwrap());
        // The manifest says log 4 is current; write updates and a deletion there.
        let batch = log::encode_write_batch(
            1000,
            &[
                (b"key-00000005".to_vec(), Some(b"updated".to_vec())),
                (b"key-00000006".to_vec(), None),
                (b"new-key".to_vec(), Some(b"fresh".to_vec())),
            ],
        );
        let log_name = files
            .keys()
            .find_map(|n| n.strip_prefix("MANIFEST-"))
            .map(|n| n.parse::<u64>().unwrap() + 1)
            .unwrap();
        files.insert(format!("{log_name:06}.log"), log::write_records(&[batch]));
        let db = Db::from_files(&files).unwrap();
        assert_eq!(db.get(b"key-00000005"), Some(&b"updated"[..]));
        assert_eq!(db.get(b"key-00000006"), None);
        assert_eq!(db.get(b"new-key"), Some(&b"fresh"[..]));
        assert_eq!(db.len(), 100);
        assert_eq!(db.last_sequence, 1002);
    }

    #[test]
    fn detects_corrupt_tables() {
        let mut files = as_map(build_db_files(&sample(50), WriteOptions::default()).unwrap());
        let table = files
            .iter_mut()
            .find(|(n, _)| n.ends_with(".ldb"))
            .unwrap()
            .1;
        table[10] ^= 0xff;
        assert!(Db::from_files(&files).is_err());
    }
}
