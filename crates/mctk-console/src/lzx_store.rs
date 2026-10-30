//! Writes XMem streams made of LZX *uncompressed* blocks.
//!
//! This is valid LZX that any conforming decoder accepts, without a match-finding
//! compressor; the output is slightly larger than its input. Bit layout follows the LZX
//! specification (as implemented by libmspack and `lzxd`): bits are packed MSB-first into
//! 16-bit little-endian words; the first frame begins with a 1-bit E8-translation flag;
//! each block header is a 3-bit type and a 24-bit size; an uncompressed block then aligns
//! to the next 16-bit word (inserting a full padding word when already aligned), stores
//! the three repeated offsets as u32 LE, the raw bytes, and a pad byte if the size is odd.

const FRAME: usize = 0x8000;
const BLOCK_UNCOMPRESSED: u32 = 3;

struct BitWriter {
    out: Vec<u8>,
    word: u16,
    used: u8,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter {
            out: Vec::new(),
            word: 0,
            used: 0,
        }
    }

    fn bits(&mut self, value: u32, count: u8) {
        for i in (0..count).rev() {
            self.word = (self.word << 1) | ((value >> i) & 1) as u16;
            self.used += 1;
            if self.used == 16 {
                self.out.extend_from_slice(&self.word.to_le_bytes());
                self.word = 0;
                self.used = 0;
            }
        }
    }

    /// Aligns to a 16-bit boundary; when already aligned, LZX expects a whole padding word.
    fn align_for_uncompressed(&mut self) {
        let pad = if self.used == 0 { 16 } else { 16 - self.used };
        self.bits(0, pad);
    }
}

/// Encodes `data` as an XMem stream of uncompressed LZX blocks, one per 32 KiB frame.
pub fn xmem_store(data: &[u8]) -> Vec<u8> {
    let mut stream = Vec::with_capacity(data.len() + data.len() / FRAME * 24 + 32);
    for (i, frame) in data.chunks(FRAME).enumerate() {
        let mut w = BitWriter::new();
        if i == 0 {
            w.bits(0, 1); // no E8 translation
        }
        w.bits(BLOCK_UNCOMPRESSED, 3);
        w.bits(frame.len() as u32, 24);
        w.align_for_uncompressed();
        let mut block = w.out;
        for r in [1u32, 1, 1] {
            block.extend_from_slice(&r.to_le_bytes());
        }
        block.extend_from_slice(frame);
        if frame.len() % 2 == 1 {
            block.push(0);
        }
        if frame.len() == FRAME {
            stream.extend_from_slice(&(block.len() as u16).to_be_bytes());
        } else {
            stream.push(0xff);
            stream.extend_from_slice(&(frame.len() as u16).to_be_bytes());
            stream.extend_from_slice(&(block.len() as u16).to_be_bytes());
        }
        stream.extend_from_slice(&block);
    }
    stream
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{xmem_decompress, xmem_decompress_all};
    use lzxd::WindowSize;

    #[test]
    fn stored_streams_decode() {
        for len in [
            0usize,
            1,
            2,
            7,
            1000,
            FRAME - 1,
            FRAME,
            FRAME + 1,
            3 * FRAME + 12_345,
        ] {
            let data: Vec<u8> = (0..len).map(|i| (i * 31 % 251) as u8).collect();
            let stream = xmem_store(&data);
            assert_eq!(
                xmem_decompress(&stream, data.len(), WindowSize::KB128).unwrap(),
                data,
                "len {len}"
            );
            if len > 0 {
                assert_eq!(
                    xmem_decompress_all(&stream, WindowSize::KB128).unwrap(),
                    data,
                    "len {len} (unsized)"
                );
            }
        }
    }
}
