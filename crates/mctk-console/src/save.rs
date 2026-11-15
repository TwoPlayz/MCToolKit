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
    /// `saveData.ms` of the Windows 64 build: two zeroes-and-length words and a zlib stream of the
    /// little-endian archive. It is not offered to users; the build shares its code with the
    /// PS3, Xbox 360 and Wii U games, which makes it the way to make test worlds without a console.
    Windows64,
    /// `*.wii` of the Wii U game: two zeroes-and-length words (big-endian) and a zlib stream of
    /// the big-endian archive.
    WiiU,
}

impl Platform {
    pub fn chunk_codec(self) -> ChunkCodec {
        match self {
            Platform::Xbox360 => ChunkCodec::XMemLzx,
            Platform::Ps3 => ChunkCodec::Deflate,
            Platform::Windows64 | Platform::WiiU => ChunkCodec::Zlib,
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
        let is_win64 = file
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ms"));
        let is_wiiu = file.extension().is_some_and(|e| e.eq_ignore_ascii_case("wii") || e.eq_ignore_ascii_case("ext"))
            || (!is_win64 && bytes.get(..4) == Some(&[0, 0, 0, 0]) && bytes.get(8) == Some(&0x78));
        let (platform, archive) = if is_wiiu {
            (Platform::WiiU, SaveArchive::parse(&decompress_wiiu(&bytes)?)?)
        } else if is_win64 {
            (Platform::Windows64, SaveArchive::parse(&decompress_windows64(&bytes)?)?)
        } else if is_gamedata {
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
            Platform::Windows64 => compress_windows64(&archive)?,
            Platform::WiiU => compress_wiiu(&archive)?,
        })
    }
}

/// `saveData.ms` to the archive: `u32 0`, `u32` archive length (little-endian), zlib stream.
pub fn decompress_windows64(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    use std::io::Read;
    let len = bytes
        .get(4..8)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
        .ok_or_else(|| bad("saveData.ms shorter than its header"))?;
    let mut out = Vec::with_capacity(len);
    flate2::read::ZlibDecoder::new(&bytes[8..]).read_to_end(&mut out)?;
    if out.len() != len {
        return Err(bad(format!("saveData.ms holds {} bytes, expected {len}", out.len())));
    }
    Ok(out)
}

pub fn compress_windows64(archive: &[u8]) -> Result<Vec<u8>, Error> {
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(archive)?;
    let mut out = vec![0u8; 4];
    out.extend_from_slice(&(archive.len() as u32).to_le_bytes());
    out.extend_from_slice(&e.finish()?);
    Ok(out)
}

/// A Wii U `.wii` save to the archive: `u32 0`, `u32` archive length (big-endian), zlib stream.
pub fn decompress_wiiu(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    use std::io::Read;
    let len = bytes
        .get(4..8)
        .map(|b| u32::from_be_bytes(b.try_into().unwrap()) as usize)
        .ok_or_else(|| bad("Wii U save shorter than its header"))?;
    let mut out = Vec::with_capacity(len);
    flate2::read::ZlibDecoder::new(&bytes[8..]).read_to_end(&mut out)?;
    if out.len() != len {
        return Err(bad(format!("Wii U save holds {} bytes, expected {len}", out.len())));
    }
    Ok(out)
}

pub fn compress_wiiu(archive: &[u8]) -> Result<Vec<u8>, Error> {
    use std::io::Write;
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(archive)?;
    let mut out = vec![0u8; 4];
    out.extend_from_slice(&(archive.len() as u32).to_be_bytes());
    out.extend_from_slice(&e.finish()?);
    Ok(out)
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
