//! Console region files (`.mcr`): the Anvil-style 4 KiB location and timestamp tables,
//! but each chunk record is `length: u32` (top bit = RLE applied), `decompressed: u32`,
//! then the platform-compressed payload.

use std::io::{Read, Write};

use mctk_nbt::{Flavor, NamedTag};

use crate::{Error, bad, rle_decode, rle_encode, xmem_decompress_all, xmem_store};

const SECTOR: usize = 4096;
const RLE_FLAG: u32 = 0x8000_0000;
const WIIU_FLAG: u32 = 0x4000_0000;

/// How a platform compresses chunk payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkCodec {
    /// PS3: `inner_len: u32 BE` + raw deflate.
    Deflate,
    /// Xbox 360: XMem LZX frames.
    XMemLzx,
    /// Windows 64: a zlib stream.
    Zlib,
}

/// A chunk record as stored, still compressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawConsoleChunk {
    pub rle: bool,
    /// Size after full decompression (including RLE expansion).
    pub decompressed_len: u32,
    pub payload: Vec<u8>,
    pub timestamp: u32,
    /// The Wii U game's second record flag (bit 30), kept so a rewritten region keeps it.
    pub wiiu_flag: bool,
}

impl RawConsoleChunk {
    /// Detects the platform codec from the payload: XMem streams begin with an `0xFF`
    /// frame header (chunk data is shorter than a full 32 KiB frame), zlib streams with
    /// `0x78`, while deflate payloads begin with a small big-endian length.
    pub fn codec(&self) -> ChunkCodec {
        match self.payload.first() {
            Some(&0xff) => ChunkCodec::XMemLzx,
            Some(&0x78) if self.payload.get(1).is_some_and(|b| (0x78 * 256 + *b as usize) % 31 == 0) => {
                ChunkCodec::Zlib
            }
            _ => ChunkCodec::Deflate,
        }
    }

    /// Fully decompresses the chunk (codec, then RLE when flagged).
    pub fn decompress(&self) -> Result<Vec<u8>, Error> {
        let inner = match self.codec() {
            ChunkCodec::Deflate => {
                let len = self
                    .payload
                    .get(..4)
                    .map(|b| u32::from_be_bytes(b.try_into().unwrap()) as usize)
                    .ok_or_else(|| bad("chunk payload too short"))?;
                let mut out = Vec::with_capacity(len);
                flate2::read::DeflateDecoder::new(&self.payload[4..]).read_to_end(&mut out)?;
                if out.len() != len {
                    return Err(bad(format!(
                        "deflate produced {} bytes, expected {len}",
                        out.len()
                    )));
                }
                out
            }
            ChunkCodec::XMemLzx => xmem_decompress_all(&self.payload, lzxd::WindowSize::KB128)?,
            ChunkCodec::Zlib => {
                let mut out = Vec::new();
                flate2::read::ZlibDecoder::new(&self.payload[..]).read_to_end(&mut out)?;
                out
            }
        };
        if self.rle {
            rle_decode(&inner, Some(self.decompressed_len as usize))
        } else if inner.len() == self.decompressed_len as usize {
            Ok(inner)
        } else {
            Err(bad(format!(
                "chunk decompressed to {} bytes, expected {}",
                inner.len(),
                self.decompressed_len
            )))
        }
    }

    /// The chunk as a McRegion-style `Level` tree. Saves from version 8 on store chunks in the
    /// game's compressed binary form instead of NBT; it is turned into the same tree.
    /// A chunk in the compressed binary form of saves from version 8 on, built from a Level
    /// tree and stored with codec.
    pub fn encode_compressed(tag: &NamedTag, version: i16, codec: ChunkCodec, timestamp: u32) -> Result<Self, Error> {
        let raw = crate::compressed::encode(tag, version)?;
        Self::from_stream(&raw, codec, timestamp)
    }

    /// Compresses an uncompressed chunk stream the way `codec`'s platform stores it: run-length
    /// encoding, then the codec.
    fn from_stream(raw: &[u8], codec: ChunkCodec, timestamp: u32) -> Result<Self, Error> {
        let rle = rle_encode(raw);
        let payload = match codec {
            ChunkCodec::Deflate => {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&rle)?;
                let deflated = e.finish()?;
                let mut p = (rle.len() as u32).to_be_bytes().to_vec();
                p.extend_from_slice(&deflated);
                p
            }
            ChunkCodec::XMemLzx => xmem_store(&rle),
            ChunkCodec::Zlib => {
                let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&rle)?;
                e.finish()?
            }
        };
        Ok(RawConsoleChunk { rle: true, decompressed_len: raw.len() as u32, payload, timestamp, wiiu_flag: false })
    }

    pub fn nbt(&self) -> Result<NamedTag, Error> {
        let bytes = self.decompress()?;
        if crate::compressed::is_compressed(&bytes) {
            return crate::compressed::decode(&bytes);
        }
        Ok(mctk_nbt::from_bytes(&bytes, Flavor::Java)?)
    }

    /// Compresses chunk NBT the way `codec`'s platform stores it (RLE, then the codec).
    pub fn encode(tag: &NamedTag, codec: ChunkCodec, timestamp: u32) -> Result<Self, Error> {
        let raw = mctk_nbt::to_bytes(tag, Flavor::Java)?;
        let rle = rle_encode(&raw);
        let payload = match codec {
            ChunkCodec::Deflate => {
                let mut e =
                    flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&rle)?;
                let deflated = e.finish()?;
                let mut p = (rle.len() as u32).to_be_bytes().to_vec();
                p.extend_from_slice(&deflated);
                p
            }
            ChunkCodec::XMemLzx => xmem_store(&rle),
            ChunkCodec::Zlib => {
                let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&rle)?;
                e.finish()?
            }
        };
        Ok(RawConsoleChunk {
            rle: true,
            decompressed_len: raw.len() as u32,
            payload,
            timestamp,
            wiiu_flag: false,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ConsoleRegion {
    data: Vec<u8>,
    /// Windows 64, PS4, Vita and Xbox One regions are little-endian; PS3 and Xbox 360 big-endian.
    little: bool,
}

/// Whether every location in the table points past the headers and inside the file.
fn locations_fit(data: &[u8], little: bool) -> bool {
    (0..1024).all(|i| {
        let w = word(data, 4 * i, little);
        let offset = (w >> 8) as usize * SECTOR;
        w == 0 || (offset >= 2 * SECTOR && offset < data.len())
    })
}

fn word(b: &[u8], o: usize, little: bool) -> u32 {
    let bytes = b[o..o + 4].try_into().unwrap();
    if little { u32::from_le_bytes(bytes) } else { u32::from_be_bytes(bytes) }
}

impl ConsoleRegion {
    pub fn parse(data: Vec<u8>) -> Result<Self, Error> {
        if !data.is_empty() && data.len() < 2 * SECTOR {
            return Err(bad("console region shorter than its header"));
        }
        // The byte order is the one whose location table makes sense.
        let little = !data.is_empty() && !locations_fit(&data, false) && locations_fit(&data, true);
        Ok(ConsoleRegion { data, little })
    }

    pub fn chunks(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        (0..1024)
            .filter(|&i| !self.data.is_empty() && word(&self.data, 4 * i, self.little) != 0)
            .map(|i| (i % 32, i / 32))
    }

    pub fn raw_chunk(&self, lx: usize, lz: usize) -> Result<Option<RawConsoleChunk>, Error> {
        assert!(
            lx < 32 && lz < 32,
            "local chunk coordinates must be below 32"
        );
        if self.data.is_empty() {
            return Ok(None);
        }
        let i = lx + lz * 32;
        let loc = word(&self.data, 4 * i, self.little);
        if loc == 0 {
            return Ok(None);
        }
        let offset = (loc >> 8) as usize * SECTOR;
        if offset < 2 * SECTOR || offset + 8 > self.data.len() {
            return Err(bad(format!("chunk ({lx}, {lz}) points outside the region")));
        }
        let head = word(&self.data, offset, self.little);
        // The Wii U game sets a second flag (bit 30) on some records; it is not part of the length.
        let len = (head & !(RLE_FLAG | WIIU_FLAG)) as usize;
        let payload = self
            .data
            .get(offset + 8..offset + 8 + len)
            .ok_or_else(|| bad(format!("chunk ({lx}, {lz}) overruns the region")))?;
        Ok(Some(RawConsoleChunk {
            rle: head & RLE_FLAG != 0,
            decompressed_len: word(&self.data, offset + 4, self.little),
            payload: payload.to_vec(),
            timestamp: word(&self.data, SECTOR + 4 * i, self.little),
            wiiu_flag: head & WIIU_FLAG != 0,
        }))
    }
}

fn put(v: u32, little: bool) -> [u8; 4] {
    if little { v.to_le_bytes() } else { v.to_be_bytes() }
}

/// Assembles a console region file from chunk records.
#[derive(Debug, Clone)]
pub struct ConsoleRegionBuilder {
    chunks: Vec<Option<RawConsoleChunk>>,
    /// Write the tables and chunk headers little-endian (Windows 64, PS4, Vita, Xbox One).
    pub little: bool,
}

impl Default for ConsoleRegionBuilder {
    fn default() -> Self {
        ConsoleRegionBuilder {
            chunks: vec![None; 1024],
            little: false,
        }
    }
}

impl ConsoleRegionBuilder {
    pub fn set(&mut self, lx: usize, lz: usize, chunk: Option<RawConsoleChunk>) {
        assert!(
            lx < 32 && lz < 32,
            "local chunk coordinates must be below 32"
        );
        self.chunks[lx + lz * 32] = chunk;
    }

    pub fn build(&self) -> Result<Vec<u8>, Error> {
        let mut out = vec![0u8; 2 * SECTOR];
        for (i, slot) in self.chunks.iter().enumerate() {
            let Some(c) = slot else { continue };
            let start = out.len() / SECTOR;
            let len = u32::try_from(c.payload.len())
                .ok()
                .filter(|&l| l & RLE_FLAG == 0)
                .ok_or_else(|| bad("chunk payload too large"))?;
            out.extend_from_slice(&put(len | if c.rle { RLE_FLAG } else { 0 } | if c.wiiu_flag { WIIU_FLAG } else { 0 }, self.little));
            out.extend_from_slice(&put(c.decompressed_len, self.little));
            out.extend_from_slice(&c.payload);
            out.resize(out.len().div_ceil(SECTOR) * SECTOR, 0);
            let sectors = out.len() / SECTOR - start;
            if sectors > 255 {
                return Err(bad("chunk larger than 255 sectors"));
            }
            out[4 * i..4 * i + 4]
                .copy_from_slice(&put(((start as u32) << 8) | sectors as u32, self.little));
            out[SECTOR + 4 * i..SECTOR + 4 * i + 4].copy_from_slice(&put(c.timestamp, self.little));
        }
        Ok(out)
    }
}
