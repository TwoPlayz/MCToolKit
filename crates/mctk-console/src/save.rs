//! Platform save files around the archive.

use std::path::{Path, PathBuf};

use lzxd::WindowSize;

use crate::{ChunkCodec, Error, SaveArchive, Xbox360SaveHeader, bad, xmem_decompress, xmem_store};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// `savegame.dat`: a 12-byte header and an XMem LZX stream of the archive.
    Xbox360,
    /// `GAMEDATA` inside a PS3 save folder: the archive uncompressed.
    Ps3,
}

impl Platform {
    pub fn chunk_codec(self) -> ChunkCodec {
        match self {
            Platform::Xbox360 => ChunkCodec::XMemLzx,
            Platform::Ps3 => ChunkCodec::Deflate,
        }
    }
}

/// A console save loaded into memory.
#[derive(Debug, Clone)]
pub struct ConsoleSave {
    pub platform: Platform,
    pub archive: SaveArchive,
    pub source: PathBuf,
}

impl ConsoleSave {
    /// Opens a PS3 save folder, a PS3 `GAMEDATA` file, or an Xbox 360 `savegame.dat`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let file = if path.is_dir() {
            path.join("GAMEDATA")
        } else {
            path.to_path_buf()
        };
        let bytes = std::fs::read(&file)?;
        let is_gamedata = file
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("GAMEDATA"));
        let (platform, archive) = if is_gamedata {
            (Platform::Ps3, SaveArchive::parse(&bytes)?)
        } else {
            (
                Platform::Xbox360,
                SaveArchive::parse(&decompress_xbox360(&bytes)?)?,
            )
        };
        Ok(ConsoleSave {
            platform,
            archive,
            source: file,
        })
    }

    /// The platform file's bytes (`GAMEDATA` or `savegame.dat`).
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let archive = self.archive.encode()?;
        Ok(match self.platform {
            Platform::Ps3 => archive,
            Platform::Xbox360 => compress_xbox360(&archive),
        })
    }
}

pub fn decompress_xbox360(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let header = Xbox360SaveHeader::parse(bytes)?;
    let stream = &bytes[Xbox360SaveHeader::LEN..];
    if header.compressed_len as usize > stream.len() {
        return Err(bad("savegame.dat shorter than its compressed size"));
    }
    xmem_decompress(
        &stream[..header.compressed_len as usize],
        header.decompressed_len as usize,
        WindowSize::KB128,
    )
}

pub fn compress_xbox360(archive: &[u8]) -> Vec<u8> {
    let stream = xmem_store(archive);
    let mut out = Vec::with_capacity(stream.len() + Xbox360SaveHeader::LEN);
    out.extend_from_slice(&(stream.len() as u32).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&(archive.len() as u32).to_be_bytes());
    out.extend_from_slice(&stream);
    out
}
