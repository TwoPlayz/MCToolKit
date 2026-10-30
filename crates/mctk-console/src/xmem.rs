//! XMem (Xbox 360 XCompress) LZX streams, as used by Xbox 360 `savegame.dat` files.
//!
//! The stream is a sequence of frames, each decompressing to 32 KiB except possibly the
//! last. Each frame is prefixed by its compressed size as a big-endian u16; a leading
//! `0xFF` byte instead introduces `frame_size: u16 BE, block_size: u16 BE` for short frames.
//! The LZX bitstream itself is decoded by the `lzxd` crate.

use lzxd::{Lzxd, WindowSize};

use crate::{Error, bad};

const FRAME: usize = 0x8000;

/// Decompresses an XMem LZX stream into exactly `output_len` bytes.
pub fn xmem_decompress(
    stream: &[u8],
    output_len: usize,
    window: WindowSize,
) -> Result<Vec<u8>, Error> {
    decode_frames(stream, Some(output_len), window)
}

/// Decompresses every frame of an XMem LZX stream whose total size is not recorded
/// (per-chunk streams). Stops at the end of the stream or at a zero block-size marker.
pub fn xmem_decompress_all(stream: &[u8], window: WindowSize) -> Result<Vec<u8>, Error> {
    decode_frames(stream, None, window)
}

fn decode_frames(
    stream: &[u8],
    output_len: Option<usize>,
    window: WindowSize,
) -> Result<Vec<u8>, Error> {
    let mut decoder = Lzxd::new(window);
    let mut out = Vec::with_capacity(output_len.unwrap_or(stream.len() * 4));
    let mut pos = 0;
    loop {
        match output_len {
            Some(n) if out.len() >= n => break,
            None if pos + 2 > stream.len() => break,
            None if stream[pos] != 0xff
                && u16::from_be_bytes([stream[pos], stream[pos + 1]]) == 0 =>
            {
                break;
            }
            _ => {}
        }
        let byte = |i: usize| {
            stream
                .get(i)
                .copied()
                .ok_or_else(|| bad(format!("XMem stream truncated at {i}")))
        };
        let (frame_len, block_len, header) = if byte(pos)? == 0xff {
            let frame = u16::from_be_bytes([byte(pos + 1)?, byte(pos + 2)?]) as usize;
            let block = u16::from_be_bytes([byte(pos + 3)?, byte(pos + 4)?]) as usize;
            (frame, block, 5)
        } else {
            (
                FRAME,
                u16::from_be_bytes([byte(pos)?, byte(pos + 1)?]) as usize,
                2,
            )
        };
        if block_len == 0 {
            return Err(bad(format!("empty XMem block at {pos}")));
        }
        let frame_len = match output_len {
            Some(n) => frame_len.min(n - out.len()),
            None => frame_len,
        };
        let block = stream
            .get(pos + header..pos + header + block_len)
            .ok_or_else(|| bad("XMem block overruns the stream"))?;
        let decoded = decoder
            .decompress_next(block, frame_len)
            .map_err(|e| bad(format!("LZX frame at {pos} (output {}): {e}", out.len())))?;
        out.extend_from_slice(decoded);
        pos += header + block_len;
    }
    Ok(out)
}

/// The 12-byte header of an Xbox 360 `savegame.dat`, followed by the XMem stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Xbox360SaveHeader {
    pub compressed_len: u32,
    pub reserved: u32,
    pub decompressed_len: u32,
}

impl Xbox360SaveHeader {
    pub const LEN: usize = 12;

    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let w = |o: usize| {
            bytes
                .get(o..o + 4)
                .map(|b| u32::from_be_bytes(b.try_into().unwrap()))
                .ok_or_else(|| bad("savegame.dat shorter than its header"))
        };
        Ok(Xbox360SaveHeader {
            compressed_len: w(0)?,
            reserved: w(4)?,
            decompressed_len: w(8)?,
        })
    }
}
