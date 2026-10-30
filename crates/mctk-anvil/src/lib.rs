//! Java Edition Anvil region files.
//!
//! A region file `r.<rx>.<rz>.mca` stores up to 32×32 chunks. It starts with two 4 KiB
//! tables: 1024 big-endian location entries (3-byte sector offset, 1-byte sector count)
//! and 1024 big-endian timestamps. Each chunk starts on a 4 KiB sector boundary with a
//! 4-byte length, a compression byte and the compressed NBT. Chunks too large for the
//! 255-sector limit set bit 0x80 of the compression byte and live in `c.<x>.<z>.mcc`.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use flate2::read::{GzDecoder, ZlibDecoder};
use flate2::write::{GzEncoder, ZlibEncoder};
use mctk_nbt::{Flavor, NamedTag};

pub const SECTOR: usize = 4096;
const HEADER_SECTORS: usize = 2;
const MAX_SECTORS_PER_CHUNK: usize = 255;
const EXTERNAL_FLAG: u8 = 0x80;

/// Chunk position in chunk coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkPos {
    pub x: i32,
    pub z: i32,
}

/// Region position in region coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RegionPos {
    pub x: i32,
    pub z: i32,
}

impl ChunkPos {
    pub fn new(x: i32, z: i32) -> Self {
        ChunkPos { x, z }
    }

    /// The region containing this chunk (floor division by 32).
    pub fn region(self) -> RegionPos {
        RegionPos {
            x: self.x.div_euclid(32),
            z: self.z.div_euclid(32),
        }
    }

    /// Position inside the region, both in `0..32`.
    pub fn local(self) -> (usize, usize) {
        (
            self.x.rem_euclid(32) as usize,
            self.z.rem_euclid(32) as usize,
        )
    }
}

impl RegionPos {
    pub fn file_name(self) -> String {
        format!("r.{}.{}.mca", self.x, self.z)
    }

    /// Parses `r.<x>.<z>.mca`.
    pub fn from_file_name(name: &str) -> Option<Self> {
        let mut parts = name.strip_prefix("r.")?.strip_suffix(".mca")?.split('.');
        let x = parts.next()?.parse().ok()?;
        let z = parts.next()?.parse().ok()?;
        parts.next().is_none().then_some(RegionPos { x, z })
    }

    /// Absolute position of a local chunk slot.
    pub fn chunk(self, lx: usize, lz: usize) -> ChunkPos {
        ChunkPos {
            x: self.x * 32 + lx as i32,
            z: self.z * 32 + lz as i32,
        }
    }
}

/// Compression of a stored chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkCompression {
    Gzip,
    Zlib,
    None,
    Lz4,
    /// Any other id, kept so unknown data can be copied verbatim.
    Other(u8),
}

impl ChunkCompression {
    fn from_id(id: u8) -> Self {
        match id {
            1 => ChunkCompression::Gzip,
            2 => ChunkCompression::Zlib,
            3 => ChunkCompression::None,
            4 => ChunkCompression::Lz4,
            other => ChunkCompression::Other(other),
        }
    }

    fn id(self) -> u8 {
        match self {
            ChunkCompression::Gzip => 1,
            ChunkCompression::Zlib => 2,
            ChunkCompression::None => 3,
            ChunkCompression::Lz4 => 4,
            ChunkCompression::Other(id) => id,
        }
    }
}

/// A chunk exactly as stored: still compressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawChunk {
    pub compression: ChunkCompression,
    pub data: Vec<u8>,
    pub timestamp: u32,
}

impl RawChunk {
    pub fn decompress(&self) -> Result<Vec<u8>, Error> {
        // Chunk NBT inflates to several times its stored size.
        let mut out = Vec::with_capacity(self.data.len() * 6);
        match self.compression {
            ChunkCompression::Gzip => {
                GzDecoder::new(&self.data[..]).read_to_end(&mut out)?;
            }
            ChunkCompression::Zlib => {
                ZlibDecoder::new(&self.data[..]).read_to_end(&mut out)?;
            }
            ChunkCompression::None => out.extend_from_slice(&self.data),
            other => return Err(Error::UnsupportedCompression(other.id())),
        }
        Ok(out)
    }

    pub fn nbt(&self) -> Result<NamedTag, Error> {
        Ok(mctk_nbt::from_bytes(&self.decompress()?, Flavor::Java)?)
    }

    /// Compresses an NBT chunk for storage.
    pub fn encode(
        tag: &NamedTag,
        compression: ChunkCompression,
        timestamp: u32,
    ) -> Result<Self, Error> {
        let raw = mctk_nbt::to_bytes(tag, Flavor::Java)?;
        let data = match compression {
            ChunkCompression::Zlib => {
                let mut e = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&raw)?;
                e.finish()?
            }
            ChunkCompression::Gzip => {
                let mut e = GzEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(&raw)?;
                e.finish()?
            }
            ChunkCompression::None => raw,
            other => return Err(Error::UnsupportedCompression(other.id())),
        };
        Ok(RawChunk {
            compression,
            data,
            timestamp,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("NBT error: {0}")]
    Nbt(#[from] mctk_nbt::Error),
    #[error("region file is shorter than its header")]
    TruncatedHeader,
    #[error("chunk ({0}, {1}) points outside the file")]
    BadLocation(usize, usize),
    #[error("chunk ({0}, {1}) has an invalid length")]
    BadLength(usize, usize),
    #[error("chunk compression type {0} is not supported")]
    UnsupportedCompression(u8),
    #[error("chunk ({0}, {1}) is stored externally but the region has no directory")]
    NoExternalDir(usize, usize),
}

/// A region file loaded into memory.
#[derive(Debug, Clone)]
pub struct Region {
    data: Vec<u8>,
    /// Directory holding external `c.x.z.mcc` files, if known.
    dir: Option<PathBuf>,
    pos: Option<RegionPos>,
}

impl Region {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let data = std::fs::read(path)?;
        let mut region = Region::from_bytes(data)?;
        region.dir = path.parent().map(Path::to_path_buf);
        region.pos = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(RegionPos::from_file_name);
        Ok(region)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Self, Error> {
        // An empty file is a valid region with no chunks.
        if !data.is_empty() && data.len() < HEADER_SECTORS * SECTOR {
            return Err(Error::TruncatedHeader);
        }
        Ok(Region {
            data,
            dir: None,
            pos: None,
        })
    }

    pub fn position(&self) -> Option<RegionPos> {
        self.pos
    }

    fn entry(&self, table: usize, lx: usize, lz: usize) -> u32 {
        let i = table * SECTOR + 4 * (lx + lz * 32);
        match self.data.get(i..i + 4) {
            Some(b) => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            None => 0,
        }
    }

    /// Local coordinates of every chunk present in the file.
    pub fn chunks(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        (0..32)
            .flat_map(move |lz| (0..32).map(move |lx| (lx, lz)))
            .filter(|&(lx, lz)| self.entry(0, lx, lz) != 0)
    }

    /// Reads a stored chunk without decompressing it.
    pub fn raw_chunk(&self, lx: usize, lz: usize) -> Result<Option<RawChunk>, Error> {
        assert!(
            lx < 32 && lz < 32,
            "local chunk coordinates must be below 32"
        );
        let loc = self.entry(0, lx, lz);
        if loc == 0 {
            return Ok(None);
        }
        let offset = (loc >> 8) as usize * SECTOR;
        let sectors = (loc & 0xff) as usize;
        if offset < HEADER_SECTORS * SECTOR || offset + 5 > self.data.len() || sectors == 0 {
            return Err(Error::BadLocation(lx, lz));
        }
        let len = u32::from_be_bytes(self.data[offset..offset + 4].try_into().unwrap()) as usize;
        if len == 0 || offset + 4 + len > self.data.len() || len > sectors * SECTOR {
            return Err(Error::BadLength(lx, lz));
        }
        let id = self.data[offset + 4];
        let compression = ChunkCompression::from_id(id & !EXTERNAL_FLAG);
        let timestamp = self.entry(1, lx, lz);
        let data = if id & EXTERNAL_FLAG != 0 {
            let dir = self.dir.as_ref().ok_or(Error::NoExternalDir(lx, lz))?;
            let pos = self.pos.ok_or(Error::NoExternalDir(lx, lz))?.chunk(lx, lz);
            std::fs::read(dir.join(format!("c.{}.{}.mcc", pos.x, pos.z)))?
        } else {
            self.data[offset + 5..offset + 4 + len].to_vec()
        };
        Ok(Some(RawChunk {
            compression,
            data,
            timestamp,
        }))
    }

    /// Reads and decodes a chunk's NBT.
    pub fn read_chunk(&self, lx: usize, lz: usize) -> Result<Option<NamedTag>, Error> {
        self.raw_chunk(lx, lz)?.map(|c| c.nbt()).transpose()
    }
}

/// Assembles a region file from chunks.
#[derive(Debug, Clone)]
pub struct RegionBuilder {
    chunks: Vec<Option<RawChunk>>,
}

impl Default for RegionBuilder {
    fn default() -> Self {
        RegionBuilder {
            chunks: vec![None; 1024],
        }
    }
}

impl RegionBuilder {
    pub fn new() -> Self {
        RegionBuilder::default()
    }

    /// Starts from every chunk of an existing region.
    pub fn from_region(region: &Region) -> Result<Self, Error> {
        let mut b = RegionBuilder::new();
        for (lx, lz) in region.chunks().collect::<Vec<_>>() {
            b.chunks[lx + lz * 32] = region.raw_chunk(lx, lz)?;
        }
        Ok(b)
    }

    pub fn set(&mut self, lx: usize, lz: usize, chunk: Option<RawChunk>) {
        assert!(
            lx < 32 && lz < 32,
            "local chunk coordinates must be below 32"
        );
        self.chunks[lx + lz * 32] = chunk;
    }

    pub fn get(&self, lx: usize, lz: usize) -> Option<&RawChunk> {
        self.chunks[lx + lz * 32].as_ref()
    }

    /// Serializes the region. Oversized chunks are written to `c.x.z.mcc` files in
    /// `external_dir` (required only if such chunks exist) for region `pos`.
    pub fn build(&self, pos: RegionPos, external_dir: Option<&Path>) -> Result<Vec<u8>, Error> {
        let mut out = vec![0u8; HEADER_SECTORS * SECTOR];
        for (i, slot) in self.chunks.iter().enumerate() {
            let Some(chunk) = slot else { continue };
            let (lx, lz) = (i % 32, i / 32);
            let mut id = chunk.compression.id();
            let mut payload: &[u8] = &chunk.data;
            if (chunk.data.len() + 5).div_ceil(SECTOR) > MAX_SECTORS_PER_CHUNK {
                let dir = external_dir.ok_or(Error::NoExternalDir(lx, lz))?;
                let c = pos.chunk(lx, lz);
                std::fs::write(dir.join(format!("c.{}.{}.mcc", c.x, c.z)), &chunk.data)?;
                id |= EXTERNAL_FLAG;
                payload = &[];
            }
            let start = out.len() / SECTOR;
            out.extend_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
            out.push(id);
            out.extend_from_slice(payload);
            out.resize(out.len().div_ceil(SECTOR) * SECTOR, 0);
            let sectors = out.len() / SECTOR - start;
            let loc = ((start as u32) << 8) | sectors as u32;
            out[4 * i..4 * i + 4].copy_from_slice(&loc.to_be_bytes());
            out[SECTOR + 4 * i..SECTOR + 4 * i + 4].copy_from_slice(&chunk.timestamp.to_be_bytes());
        }
        Ok(out)
    }

    /// Writes the region to `dir/r.x.z.mca`, placing external chunks beside it.
    pub fn write(&self, dir: impl AsRef<Path>, pos: RegionPos) -> Result<PathBuf, Error> {
        let dir = dir.as_ref();
        let path = dir.join(pos.file_name());
        std::fs::write(&path, self.build(pos, Some(dir))?)?;
        Ok(path)
    }
}
