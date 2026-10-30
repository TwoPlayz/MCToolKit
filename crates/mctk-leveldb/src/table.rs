//! Sorted string tables (`.ldb` / `.sst`), per LevelDB's `doc/table_format.md`.

use std::io::{Read, Write};

use flate2::read::{DeflateDecoder, ZlibDecoder};
use flate2::write::{DeflateEncoder, ZlibEncoder};

use crate::coding::{Reader, put_varint};
use crate::crc32c::{crc32c, mask};
use crate::{Error, WriteOptions, corrupt};

/// A raw (key, value) pair as stored in a block.
pub type Entry = (Vec<u8>, Vec<u8>);

const MAGIC: u64 = 0xdb47_7524_8b80_fb57;
const FOOTER: usize = 48;
const TRAILER: usize = 5;
const RESTART_INTERVAL: usize = 16;

/// Compression applied to each table block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockCompression {
    None,
    /// Mojang: zlib with header (type 2).
    Zlib,
    /// Mojang: raw deflate (type 4), used by current Bedrock builds.
    RawDeflate,
}

impl BlockCompression {
    fn id(self) -> u8 {
        match self {
            BlockCompression::None => 0,
            BlockCompression::Zlib => 2,
            BlockCompression::RawDeflate => 4,
        }
    }
}

fn read_handle(r: &mut Reader) -> Result<(usize, usize), Error> {
    Ok((r.varint()? as usize, r.varint()? as usize))
}

/// Reads, verifies and decompresses the block at `(offset, size)`.
fn read_block(data: &[u8], (offset, size): (usize, usize)) -> Result<Vec<u8>, Error> {
    let end = offset
        .checked_add(size + TRAILER)
        .filter(|&e| e <= data.len())
        .ok_or_else(|| corrupt("block handle outside table"))?;
    let contents = &data[offset..offset + size];
    let kind = data[offset + size];
    let stored = u32::from_le_bytes(data[offset + size + 1..end].try_into().unwrap());
    if mask(crc32c(&[contents, &[kind]])) != stored {
        return Err(corrupt(format!(
            "block checksum mismatch at offset {offset}"
        )));
    }
    let mut out = Vec::new();
    match kind {
        0 => out.extend_from_slice(contents),
        2 => {
            ZlibDecoder::new(contents).read_to_end(&mut out)?;
        }
        4 => {
            DeflateDecoder::new(contents).read_to_end(&mut out)?;
        }
        other => return Err(Error::UnsupportedCompression(other)),
    }
    Ok(out)
}

/// Decodes the prefix-compressed entries of a block.
fn block_entries(block: &[u8]) -> Result<Vec<Entry>, Error> {
    if block.len() < 4 {
        return Err(corrupt("block too small"));
    }
    let restarts = u32::from_le_bytes(block[block.len() - 4..].try_into().unwrap()) as usize;
    let entries_end = block
        .len()
        .checked_sub(4 + 4 * restarts)
        .ok_or_else(|| corrupt("bad restart count"))?;
    let mut r = Reader::new(&block[..entries_end]);
    let mut key: Vec<u8> = Vec::new();
    let mut out = Vec::new();
    while !r.is_empty() {
        let shared = r.varint()? as usize;
        let unshared = r.varint()? as usize;
        let value_len = r.varint()? as usize;
        if shared > key.len() {
            return Err(corrupt("shared prefix longer than previous key"));
        }
        key.truncate(shared);
        key.extend_from_slice(r.bytes(unshared)?);
        out.push((key.clone(), r.bytes(value_len)?.to_vec()));
    }
    Ok(out)
}

/// Returns every (internal key, value) pair of a table in order.
pub fn read_table(data: &[u8]) -> Result<Vec<Entry>, Error> {
    if data.len() < FOOTER {
        return Err(corrupt("table smaller than its footer"));
    }
    let footer = &data[data.len() - FOOTER..];
    if u64::from_le_bytes(footer[40..].try_into().unwrap()) != MAGIC {
        return Err(corrupt("bad table magic"));
    }
    let mut r = Reader::new(&footer[..40]);
    let _metaindex = read_handle(&mut r)?;
    let index = read_handle(&mut r)?;
    let mut out = Vec::new();
    for (_, handle) in block_entries(&read_block(data, index)?)? {
        let handle = read_handle(&mut Reader::new(&handle))?;
        out.extend(block_entries(&read_block(data, handle)?)?);
    }
    Ok(out)
}

struct BlockBuilder {
    buf: Vec<u8>,
    restarts: Vec<u32>,
    count: usize,
    last_key: Vec<u8>,
}

impl BlockBuilder {
    fn new() -> Self {
        BlockBuilder {
            buf: Vec::new(),
            restarts: vec![0],
            count: 0,
            last_key: Vec::new(),
        }
    }

    fn add(&mut self, key: &[u8], value: &[u8]) {
        let shared = if self.count % RESTART_INTERVAL == 0 {
            if self.count > 0 {
                self.restarts.push(self.buf.len() as u32);
            }
            0
        } else {
            self.last_key
                .iter()
                .zip(key)
                .take_while(|(a, b)| a == b)
                .count()
        };
        put_varint(&mut self.buf, shared as u64);
        put_varint(&mut self.buf, (key.len() - shared) as u64);
        put_varint(&mut self.buf, value.len() as u64);
        self.buf.extend_from_slice(&key[shared..]);
        self.buf.extend_from_slice(value);
        self.last_key = key.to_vec();
        self.count += 1;
    }

    fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn finish(mut self) -> Vec<u8> {
        for r in &self.restarts {
            self.buf.extend_from_slice(&r.to_le_bytes());
        }
        self.buf
            .extend_from_slice(&(self.restarts.len() as u32).to_le_bytes());
        self.buf
    }
}

pub struct TableBuilder {
    options: WriteOptions,
    out: Vec<u8>,
    block: BlockBuilder,
    index: BlockBuilder,
    raw_size: usize,
}

impl TableBuilder {
    pub fn new(options: WriteOptions) -> Self {
        TableBuilder {
            options,
            out: Vec::new(),
            block: BlockBuilder::new(),
            index: BlockBuilder::new(),
            raw_size: 0,
        }
    }

    pub fn add(&mut self, key: &[u8], value: &[u8]) {
        self.block.add(key, value);
        self.raw_size += key.len() + value.len();
        if self.block.buf.len() >= self.options.block_size {
            self.flush_block();
        }
    }

    pub fn estimated_size(&self) -> usize {
        self.raw_size
    }

    fn write_block(&mut self, contents: &[u8], compression: BlockCompression) -> (usize, usize) {
        let compressed = match compression {
            BlockCompression::None => contents.to_vec(),
            BlockCompression::Zlib => {
                let mut e = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(contents).expect("in-memory write");
                e.finish().expect("in-memory write")
            }
            BlockCompression::RawDeflate => {
                let mut e = DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(contents).expect("in-memory write");
                e.finish().expect("in-memory write")
            }
        };
        let offset = self.out.len();
        let kind = compression.id();
        self.out.extend_from_slice(&compressed);
        self.out.push(kind);
        self.out
            .extend_from_slice(&mask(crc32c(&[&compressed, &[kind]])).to_le_bytes());
        (offset, compressed.len())
    }

    fn flush_block(&mut self) {
        if self.block.is_empty() {
            return;
        }
        let block = std::mem::replace(&mut self.block, BlockBuilder::new());
        // The block's last key is a valid separator: >= every key in the block and
        // < the first key of the next block.
        let last_key = block.last_key.clone();
        let (offset, size) = self.write_block(&block.finish(), self.options.compression);
        let mut handle = Vec::new();
        put_varint(&mut handle, offset as u64);
        put_varint(&mut handle, size as u64);
        self.index.add(&last_key, &handle);
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.flush_block();
        let metaindex = self.write_block(&BlockBuilder::new().finish(), BlockCompression::None);
        let index = std::mem::replace(&mut self.index, BlockBuilder::new()).finish();
        let index = self.write_block(&index, BlockCompression::None);
        let mut footer = Vec::new();
        for (o, s) in [metaindex, index] {
            put_varint(&mut footer, o as u64);
            put_varint(&mut footer, s as u64);
        }
        footer.resize(40, 0);
        footer.extend_from_slice(&MAGIC.to_le_bytes());
        self.out.extend_from_slice(&footer);
        self.out
    }
}
