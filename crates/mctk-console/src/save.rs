//! Platform save files around the archive.

use std::path::{Path, PathBuf};

use lzxd::WindowSize;

use crate::xmem::XMEM_TERMINATOR_LEN;
use crate::{ChunkCodec, Error, SaveArchive, Xbox360SaveHeader, bad, xmem_compress, xmem_decompress};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// `savegame.dat`: a 12-byte header and an XMem LZX stream of the archive.
    Xbox360,
    /// `GAMEDATA` inside a PS3 save folder: the archive, either uncompressed (older game versions) or,
    /// in newer ones, `u32 0`, the archive length twice (`u32` big-endian), a raw deflate stream
    /// and the Adler-32 of the archive (see [`ConsoleSave::gamedata_compressed`]).
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
    /// PS3 only: `GAMEDATA` was (and will be written) in the compressed layout of newer game versions.
    pub gamedata_compressed: bool,
}

impl ConsoleSave {
    /// Opens a PS3 save folder, a PS3 `GAMEDATA` file, or an Xbox 360 `savegame.dat`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        let file = if path.is_dir() {
            path.join("GAMEDATA")
        } else if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("ext")) {
            // The Wii U companion (name + thumbnail) was picked: the save is the file beside it.
            crate::wiiu::save_for_companion(path).unwrap_or_else(|| path.to_path_buf())
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
        let mut gamedata_compressed = false;
        let (platform, archive) = if is_wiiu {
            (Platform::WiiU, SaveArchive::parse(&decompress_wiiu(&bytes)?)?)
        } else if is_win64 {
            (Platform::Windows64, SaveArchive::parse(&decompress_windows64(&bytes)?)?)
        } else if is_gamedata {
            match decompress_ps3_gamedata(&bytes)? {
                Some(raw) => {
                    gamedata_compressed = true;
                    (Platform::Ps3, SaveArchive::parse(&raw)?)
                }
                None => (Platform::Ps3, SaveArchive::parse(&bytes)?),
            }
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
            gamedata_compressed,
        })
    }

    /// The platform file's bytes (`GAMEDATA` or `savegame.dat`).
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let archive = self.archive.encode()?;
        Ok(match self.platform {
            Platform::Ps3 if self.gamedata_compressed => compress_ps3_gamedata(&archive)?,
            Platform::Ps3 => archive,
            Platform::Xbox360 => compress_xbox360(&archive),
            Platform::Windows64 => compress_windows64(&archive)?,
            Platform::WiiU => compress_wiiu(&archive)?,
        })
    }
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    (b << 16) | a
}

/// The compressed `GAMEDATA` of newer PS3 game versions: `u32 0`, the archive length as a `u32`
/// big-endian twice, a raw deflate stream (zlib level 6) and the Adler-32 of the archive. Returns
/// `None` when `bytes` is not in this layout (an uncompressed archive starts with its index offset,
/// never with a zero word). Measured on a save written by a real converter.
pub fn decompress_ps3_gamedata(bytes: &[u8]) -> Result<Option<Vec<u8>>, Error> {
    use std::io::Read;
    if bytes.len() < 16 || bytes[..4] != [0; 4] || bytes[4..8] != bytes[8..12] {
        return Ok(None);
    }
    let len = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let mut decoder = flate2::bufread::DeflateDecoder::new(&bytes[12..]);
    let mut out = Vec::with_capacity(len);
    decoder.read_to_end(&mut out)?;
    if out.len() != len {
        return Err(bad(format!("PS3 GAMEDATA holds {} bytes, expected {len}", out.len())));
    }
    let rest: &[u8] = decoder.get_ref();
    if rest.len() >= 4 && rest[..4] != adler32(&out).to_be_bytes() {
        return Err(bad("PS3 GAMEDATA checksum does not match its archive"));
    }
    Ok(Some(out))
}

pub fn compress_ps3_gamedata(archive: &[u8]) -> Result<Vec<u8>, Error> {
    use std::io::Write;
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(archive)?;
    let stream = e.finish()?;
    let mut out = Vec::with_capacity(stream.len() + 16);
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&(archive.len() as u32).to_be_bytes());
    out.extend_from_slice(&(archive.len() as u32).to_be_bytes());
    out.extend_from_slice(&stream);
    out.extend_from_slice(&adler32(archive).to_be_bytes());
    Ok(out)
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
    // The first word counts everything after itself: the other 8 header bytes, the frames and the
    // 5-byte terminator. Genuine console saves are padded beyond that, so only the lower bound is
    // checked and the padding is ignored.
    let end = header.end_offset();
    if end < Xbox360SaveHeader::LEN || end > bytes.len() {
        return Err(bad("savegame.dat shorter than its recorded size"));
    }
    xmem_decompress(
        &bytes[Xbox360SaveHeader::LEN..end],
        header.decompressed_len as usize,
        WindowSize::KB128,
    )
}

pub fn compress_xbox360(archive: &[u8]) -> Vec<u8> {
    let stream = xmem_compress(archive);
    let header = Xbox360SaveHeader::for_stream(stream.len(), archive.len());
    let mut out = Vec::with_capacity(stream.len() + Xbox360SaveHeader::LEN + XMEM_TERMINATOR_LEN);
    out.extend_from_slice(&header.compressed_len.to_be_bytes());
    out.extend_from_slice(&header.reserved.to_be_bytes());
    out.extend_from_slice(&header.decompressed_len.to_be_bytes());
    out.extend_from_slice(&crate::xmem::terminated(stream));
    out
}
