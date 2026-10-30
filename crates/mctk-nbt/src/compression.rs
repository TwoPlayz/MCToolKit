use std::io::Read;
use std::path::Path;

use flate2::read::{GzDecoder, ZlibDecoder};

use crate::{Error, Flavor, NamedTag, read_root};

/// Outer compression of an NBT file or blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Zlib,
}

impl Compression {
    /// Detects the compression from the first bytes of a blob.
    pub fn detect(bytes: &[u8]) -> Self {
        match bytes {
            [0x1f, 0x8b, ..] => Compression::Gzip,
            // zlib header: CM = 8 (deflate) and the header checksum is a multiple of 31.
            [cmf, flg, ..] if cmf & 0x0f == 8 && ((*cmf as u16) << 8 | *flg as u16) % 31 == 0 => {
                Compression::Zlib
            }
            _ => Compression::None,
        }
    }
}

/// Decompresses a blob according to its detected compression.
pub fn decompress(bytes: &[u8]) -> Result<(Compression, Vec<u8>), Error> {
    let kind = Compression::detect(bytes);
    let mut out = Vec::new();
    match kind {
        Compression::None => out.extend_from_slice(bytes),
        Compression::Gzip => {
            GzDecoder::new(bytes).read_to_end(&mut out)?;
        }
        Compression::Zlib => {
            ZlibDecoder::new(bytes).read_to_end(&mut out)?;
        }
    }
    Ok((kind, out))
}

/// Reads an NBT file such as `level.dat`, detecting its compression.
///
/// Bedrock `level.dat` files carry an 8-byte header (format version and payload length)
/// before the little-endian NBT; it is skipped automatically for [`Flavor::Bedrock`].
pub fn read_file(path: impl AsRef<Path>, flavor: Flavor) -> Result<(Compression, NamedTag), Error> {
    let raw = std::fs::read(path)?;
    let (kind, data) = decompress(&raw)?;
    let body = match flavor {
        Flavor::Bedrock if has_bedrock_header(&data) => &data[8..],
        _ => &data[..],
    };
    Ok((kind, read_root(&mut &body[..], flavor)?))
}

fn has_bedrock_header(data: &[u8]) -> bool {
    if data.len() < 9 {
        return false;
    }
    let len = u32::from_le_bytes([data[4], data[5], data[6], data[7]]) as usize;
    len == data.len() - 8 && matches!(data[8], 9 | 10)
}
