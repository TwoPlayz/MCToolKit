//! Console region files (`.mcr`): the Anvil-style 4 KiB location and timestamp tables,
//! but each chunk record is `length: u32` (top bit = RLE applied), `decompressed: u32`,
//! then the platform-compressed payload.

use std::io::{Read, Write};

use mctk_nbt::{Flavor, NamedTag};

use crate::{Error, bad, rle_decode, rle_encode, xmem_compress, xmem_decompress_all};

const SECTOR: usize = 4096;
const RLE_FLAG: u32 = 0x8000_0000;
const V12_FLAG: u32 = 0x4000_0000;

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

/// Whether `p` is a chain of XMem frames (`u16 BE` block size, or `0xFF` + frame size + block size)
/// that ends exactly at the end of `p` or just before a 5-byte zero terminator.
fn is_xmem_chain(p: &[u8]) -> bool {
    let mut pos = 0;
    let mut frames = 0;
    loop {
        let rest = &p[pos..];
        if rest.is_empty() || (rest.len() == crate::xmem::XMEM_TERMINATOR_LEN && rest.iter().all(|b| *b == 0)) {
            return frames > 0;
        }
        let (header, block) = match rest {
            [0xff, _, _, b0, b1, ..] => (5, u16::from_be_bytes([*b0, *b1]) as usize),
            [0xff, ..] => return false,
            [b0, b1, ..] => (2, u16::from_be_bytes([*b0, *b1]) as usize),
            _ => return false,
        };
        if block == 0 || header + block > rest.len() {
            return false;
        }
        pos += header + block;
        frames += 1;
    }
}

/// A chunk record as stored, still compressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawConsoleChunk {
    pub rle: bool,
    /// Size after full decompression (including RLE expansion).
    pub decompressed_len: u32,
    pub payload: Vec<u8>,
    pub timestamp: u32,
    /// Record flag bit 30: the chunk is in the Version 12 layout (it starts with `u16 BE 12`, not an NBT
    /// compound). Measured on a genuine Wii U save, which mixes 1,770 unflagged NBT chunks with 599
    /// flagged Version 12 ones, and on a converter's saves, where every chunk is flagged. Kept as read so
    /// a rewritten region keeps it.
    pub v12_layout: bool,
}

impl RawConsoleChunk {
    /// Detects the platform codec from the payload. XMem streams begin with an `0xFF` header when
    /// the first frame is short; when it is a full 32 KiB frame (chunks of the Version 12 layout can
    /// be that large) they are recognised by their frame chain instead: every frame's size lands exactly
    /// on the end of the payload or on the 5-byte zero terminator, which a deflate or zlib stream
    /// does not do. Zlib streams begin with a valid `0x78` header, deflate payloads with a small
    /// big-endian length.
    pub fn codec(&self) -> ChunkCodec {
        match self.payload.first() {
            Some(&0xff) => ChunkCodec::XMemLzx,
            _ if is_xmem_chain(&self.payload) => ChunkCodec::XMemLzx,
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
            ChunkCodec::XMemLzx => crate::xmem::terminated(xmem_compress(&rle)),
            ChunkCodec::Zlib => {
                let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&rle)?;
                e.finish()?
            }
        };
        Ok(RawConsoleChunk { rle: true, decompressed_len: raw.len() as u32, payload, timestamp, v12_layout: false })
    }

    /// The chunk in the Version 12 layout, if it is one (see [`ChunkV12`](crate::ChunkV12)), with every
    /// field as stored (the liquid layer and ids above 255 included).
    pub fn v12(&self) -> Result<Option<crate::ChunkV12>, Error> {
        let bytes = self.decompress()?;
        if crate::v12::is_v12(&bytes) { Ok(Some(crate::ChunkV12::parse(&bytes)?)) } else { Ok(None) }
    }

    /// A chunk of the Version 12 layout, built from a `Level` tree of 256-tall blocks and stored with
    /// `codec` (run-length encoding first, then the codec) and record bit 30 set.
    pub fn encode_v12(tag: &NamedTag, codec: ChunkCodec, timestamp: u32) -> Result<Self, Error> {
        let raw = crate::ChunkV12::from_level_tree(tag)?.to_bytes()?;
        let mut chunk = Self::from_stream(&raw, codec, timestamp)?;
        chunk.v12_layout = true;
        Ok(chunk)
    }

    pub fn nbt(&self) -> Result<NamedTag, Error> {
        let bytes = self.decompress()?;
        if crate::v12::is_v12(&bytes) {
            return Ok(crate::ChunkV12::parse(&bytes)?.to_level_tree());
        }
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
            ChunkCodec::XMemLzx => crate::xmem::terminated(xmem_compress(&rle)),
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
            v12_layout: false,
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
        // Bit 30 marks a Version 12 chunk; like the RLE bit it is not part of the length.
        let len = (head & !(RLE_FLAG | V12_FLAG)) as usize;
        let payload = self
            .data
            .get(offset + 8..offset + 8 + len)
            .ok_or_else(|| bad(format!("chunk ({lx}, {lz}) overruns the region")))?;
        Ok(Some(RawConsoleChunk {
            rle: head & RLE_FLAG != 0,
            decompressed_len: word(&self.data, offset + 4, self.little),
            payload: payload.to_vec(),
            timestamp: word(&self.data, SECTOR + 4 * i, self.little),
            v12_layout: head & V12_FLAG != 0,
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
            out.extend_from_slice(&put(len | if c.rle { RLE_FLAG } else { 0 } | if c.v12_layout { V12_FLAG } else { 0 }, self.little));
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
