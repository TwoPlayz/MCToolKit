//! MANIFEST files: a log of VersionEdit records describing which table files are live.

use std::collections::BTreeSet;

use crate::coding::{Reader, put_length_prefixed, put_varint};
use crate::{Error, corrupt, log};

const TAG_COMPARATOR: u64 = 1;
const TAG_LOG_NUMBER: u64 = 2;
const TAG_NEXT_FILE: u64 = 3;
const TAG_LAST_SEQUENCE: u64 = 4;
const TAG_COMPACT_POINTER: u64 = 5;
const TAG_DELETED_FILE: u64 = 6;
const TAG_NEW_FILE: u64 = 7;
const TAG_PREV_LOG_NUMBER: u64 = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMeta {
    pub number: u64,
    pub size: u64,
    pub smallest: Vec<u8>,
    pub largest: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VersionEdit {
    pub comparator: Option<String>,
    pub log_number: Option<u64>,
    pub prev_log_number: Option<u64>,
    pub next_file_number: Option<u64>,
    pub last_sequence: Option<u64>,
    pub deleted_files: Vec<(u64, u64)>,
    pub new_files: Vec<(u64, FileMeta)>,
}

impl VersionEdit {
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        let mut r = Reader::new(data);
        let mut e = VersionEdit::default();
        while !r.is_empty() {
            match r.varint()? {
                TAG_COMPARATOR => {
                    e.comparator = Some(String::from_utf8_lossy(r.length_prefixed()?).into_owned());
                }
                TAG_LOG_NUMBER => e.log_number = Some(r.varint()?),
                TAG_NEXT_FILE => e.next_file_number = Some(r.varint()?),
                TAG_LAST_SEQUENCE => e.last_sequence = Some(r.varint()?),
                TAG_PREV_LOG_NUMBER => e.prev_log_number = Some(r.varint()?),
                TAG_COMPACT_POINTER => {
                    r.varint()?;
                    r.length_prefixed()?;
                }
                TAG_DELETED_FILE => {
                    let level = r.varint()?;
                    e.deleted_files.push((level, r.varint()?));
                }
                TAG_NEW_FILE => {
                    let level = r.varint()?;
                    let number = r.varint()?;
                    let size = r.varint()?;
                    let smallest = r.length_prefixed()?.to_vec();
                    let largest = r.length_prefixed()?.to_vec();
                    e.new_files.push((
                        level,
                        FileMeta {
                            number,
                            size,
                            smallest,
                            largest,
                        },
                    ));
                }
                other => return Err(corrupt(format!("unknown VersionEdit tag {other}"))),
            }
        }
        Ok(e)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(c) = &self.comparator {
            put_varint(&mut out, TAG_COMPARATOR);
            put_length_prefixed(&mut out, c.as_bytes());
        }
        for (tag, value) in [
            (TAG_LOG_NUMBER, self.log_number),
            (TAG_PREV_LOG_NUMBER, self.prev_log_number),
            (TAG_NEXT_FILE, self.next_file_number),
            (TAG_LAST_SEQUENCE, self.last_sequence),
        ] {
            if let Some(v) = value {
                put_varint(&mut out, tag);
                put_varint(&mut out, v);
            }
        }
        for (level, number) in &self.deleted_files {
            put_varint(&mut out, TAG_DELETED_FILE);
            put_varint(&mut out, *level);
            put_varint(&mut out, *number);
        }
        for (level, f) in &self.new_files {
            put_varint(&mut out, TAG_NEW_FILE);
            put_varint(&mut out, *level);
            put_varint(&mut out, f.number);
            put_varint(&mut out, f.size);
            put_length_prefixed(&mut out, &f.smallest);
            put_length_prefixed(&mut out, &f.largest);
        }
        out
    }
}

/// The database state after replaying every edit.
#[derive(Debug, Default)]
pub struct State {
    pub live_files: BTreeSet<u64>,
    pub log_number: u64,
    pub prev_log_number: Option<u64>,
    pub last_sequence: u64,
}

pub fn replay(manifest: &[u8]) -> Result<State, Error> {
    let mut state = State::default();
    let mut live: BTreeSet<(u64, u64)> = BTreeSet::new();
    for record in log::read_records(manifest)? {
        let edit = VersionEdit::decode(&record)?;
        if let Some(c) = &edit.comparator {
            if c != "leveldb.BytewiseComparator" {
                return Err(corrupt(format!("unsupported comparator {c}")));
            }
        }
        if let Some(v) = edit.log_number {
            state.log_number = v;
        }
        if let Some(v) = edit.prev_log_number {
            state.prev_log_number = Some(v).filter(|&n| n != 0);
        }
        if let Some(v) = edit.last_sequence {
            state.last_sequence = v;
        }
        for d in &edit.deleted_files {
            live.remove(d);
        }
        for (level, f) in &edit.new_files {
            live.insert((*level, f.number));
        }
    }
    state.live_files = live.into_iter().map(|(_, n)| n).collect();
    Ok(state)
}
