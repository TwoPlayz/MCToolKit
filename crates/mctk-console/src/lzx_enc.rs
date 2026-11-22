//! An LZX compressor for XMem streams (the Xbox 360 console format, 128 KiB window).
//!
//! The real compressor ships as a Microsoft DLL; this one is written from the LZX format
//! description (as implemented by libmspack and `lzxd`) and only has to produce streams any
//! conforming decoder reads back. Each 32 KiB frame is one *verbatim* block: a hash-chain
//! match finder with repeat-offset reuse and one step of lazy matching feeds per-frame
//! canonical Huffman trees (main 528 symbols, length 249). A frame that would not shrink is
//! written as an uncompressed block instead. Matches never cross a frame boundary, and the
//! repeated-offset registers and tree lengths carry from frame to frame exactly as the
//! decoder's do.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

const FRAME: usize = 0x8000;
const POSITION_SLOTS: usize = 34; // 128 KiB window
const MAIN_SYMBOLS: usize = 256 + POSITION_SLOTS * 8;
const LENGTH_SYMBOLS: usize = 249;
const MIN_MATCH: usize = 2;
const MAX_MATCH: usize = 257;
/// Farthest back a match may reach (window size minus the decoder's slack).
const MAX_DIST: usize = 0x20000 - 262;
const CHAIN: usize = 64;
const HASH_BITS: u32 = 16;

struct BitWriter {
    out: Vec<u8>,
    word: u16,
    used: u8,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter { out: Vec::new(), word: 0, used: 0 }
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

    fn pad(&mut self) {
        if self.used != 0 {
            self.bits(0, 16 - self.used);
        }
    }

    /// Alignment before the 12 bytes of an uncompressed block: a whole word when already aligned.
    fn align_for_uncompressed(&mut self) {
        let pad = if self.used == 0 { 16 } else { 16 - self.used };
        self.bits(0, pad);
    }
}

fn slot_tables() -> ([u32; POSITION_SLOTS], [u8; POSITION_SLOTS]) {
    let mut base = [0u32; POSITION_SLOTS];
    let mut extra = [0u8; POSITION_SLOTS];
    for s in 0..POSITION_SLOTS {
        extra[s] = if s < 4 { 0 } else { (s / 2 - 1).min(17) as u8 };
        if s > 0 {
            base[s] = base[s - 1] + (1u32 << extra[s - 1]);
        }
    }
    (base, extra)
}

/// A match as symbols: main symbol, length symbol, extra bits value, extra bits count.
type Described = (usize, Option<usize>, u32, u8);

#[derive(Clone, Copy)]
enum Token {
    Literal(u8),
    /// Length, and either a repeat index (0..3) or a real distance.
    Match { len: u16, repeat: Option<u8>, dist: u32 },
}

/// Length-limited Huffman code lengths (0 for unused symbols). At least two symbols always get a
/// code when any is used, so the tree is complete.
fn huffman_lengths(freqs: &[u32], max_len: u8) -> Vec<u8> {
    let mut f: Vec<u32> = freqs.to_vec();
    let used = f.iter().filter(|&&x| x > 0).count();
    if used == 0 {
        return vec![0; freqs.len()];
    }
    if used == 1 {
        let lone = f.iter().position(|&x| x > 0).unwrap();
        f[if lone == 0 { 1 } else { 0 }] = 1;
    }
    loop {
        let mut parent: Vec<usize> = vec![usize::MAX; f.len()];
        let mut heap = BinaryHeap::new();
        for (i, &x) in f.iter().enumerate() {
            if x > 0 {
                heap.push(Reverse((x as u64, i)));
            }
        }
        let mut next = f.len();
        while heap.len() > 1 {
            let Reverse((wa, a)) = heap.pop().unwrap();
            let Reverse((wb, b)) = heap.pop().unwrap();
            if parent.len() <= next {
                parent.resize(next + 1, usize::MAX);
            }
            parent[a] = next;
            parent[b] = next;
            heap.push(Reverse((wa + wb, next)));
            next += 1;
        }
        let mut lens = vec![0u8; freqs.len()];
        let mut too_long = false;
        for i in 0..f.len() {
            if f[i] == 0 {
                continue;
            }
            let mut d = 0u32;
            let mut n = i;
            while parent[n] != usize::MAX {
                n = parent[n];
                d += 1;
            }
            if d > max_len as u32 {
                too_long = true;
            }
            lens[i] = d.min(255) as u8;
        }
        if !too_long {
            return lens;
        }
        for x in f.iter_mut() {
            if *x > 0 {
                *x = (*x).div_ceil(2);
            }
        }
    }
}

/// Canonical codes (shorter first, then by symbol), as the decoders build them.
fn canonical_codes(lens: &[u8]) -> Vec<u32> {
    let mut codes = vec![0u32; lens.len()];
    let mut code = 0u32;
    for l in 1..=16u8 {
        for (i, &x) in lens.iter().enumerate() {
            if x == l {
                codes[i] = code;
                code += 1;
            }
        }
        code <<= 1;
    }
    codes
}

/// Writes the pretree-coded lengths of `lens[range]`, relative to `prev`, and updates `prev`.
fn write_tree(w: &mut BitWriter, lens: &[u8], prev: &mut [u8], from: usize, to: usize) {
    // (pretree symbol, extra value, extra bit count)
    let mut syms: Vec<(u8, u32, u8)> = Vec::new();
    let mut z = from;
    while z < to {
        if lens[z] == 0 {
            let mut run = 1;
            while z + run < to && lens[z + run] == 0 && run < 51 {
                run += 1;
            }
            if run >= 20 {
                syms.push((18, (run - 20) as u32, 5));
                z += run;
                continue;
            }
            if run >= 4 {
                syms.push((17, (run - 4) as u32, 4));
                z += run;
                continue;
            }
        }
        syms.push((((prev[z] as i32 - lens[z] as i32 + 17) % 17) as u8, 0, 0));
        z += 1;
    }
    let mut freqs = [0u32; 20];
    for &(s, _, _) in &syms {
        freqs[s as usize] += 1;
    }
    let plens = huffman_lengths(&freqs, 15);
    let pcodes = canonical_codes(&plens);
    for &l in &plens {
        w.bits(l as u32, 4);
    }
    for &(s, extra, nb) in &syms {
        w.bits(pcodes[s as usize], plens[s as usize]);
        if nb > 0 {
            w.bits(extra, nb);
        }
    }
    prev[from..to].copy_from_slice(&lens[from..to]);
}

struct Encoder<'a> {
    data: &'a [u8],
    head: Vec<i32>,
    chain: Vec<i32>,
    inserted: usize,
    r: [u32; 3],
    prev_main: Vec<u8>,
    prev_len: Vec<u8>,
    base: [u32; POSITION_SLOTS],
    extra: [u8; POSITION_SLOTS],
}

impl<'a> Encoder<'a> {
    fn new(data: &'a [u8]) -> Self {
        let (base, extra) = slot_tables();
        Encoder {
            data,
            head: vec![-1; 1 << HASH_BITS],
            chain: vec![-1; data.len()],
            inserted: 0,
            r: [1, 1, 1],
            prev_main: vec![0; MAIN_SYMBOLS],
            prev_len: vec![0; LENGTH_SYMBOLS],
            base,
            extra,
        }
    }

    fn hash(&self, p: usize) -> usize {
        let d = self.data;
        let v = (d[p] as u32) | ((d[p + 1] as u32) << 8) | ((d[p + 2] as u32) << 16);
        (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
    }

    /// Makes every position below `upto` findable.
    fn insert_to(&mut self, upto: usize) {
        while self.inserted < upto {
            let p = self.inserted;
            if p + 2 < self.data.len() {
                let h = self.hash(p);
                self.chain[p] = self.head[h];
                self.head[h] = p as i32;
            }
            self.inserted += 1;
        }
    }

    fn match_len(&self, pos: usize, dist: usize, limit: usize) -> usize {
        let d = self.data;
        let mut n = 0;
        while n < limit && d[pos + n] == d[pos + n - dist] {
            n += 1;
        }
        n
    }

    /// Best match at `pos` (not beyond `end`), as (length, repeat index, distance).
    fn find(&mut self, pos: usize, end: usize) -> Option<(usize, Option<u8>, usize)> {
        let limit = (end - pos).min(MAX_MATCH);
        if limit < MIN_MATCH {
            return None;
        }
        let mut best: Option<(usize, Option<u8>, usize)> = None;
        let mut score = 0usize;
        for (i, &rd) in self.r.iter().enumerate() {
            let rd = rd as usize;
            if rd <= pos {
                let n = self.match_len(pos, rd, limit);
                if n >= MIN_MATCH && n * 2 + 1 > score {
                    score = n * 2 + 1;
                    best = Some((n, Some(i as u8), rd));
                }
            }
        }
        if pos + 2 < self.data.len() && limit >= 3 {
            self.insert_to(pos);
            let mut cand = self.head[self.hash(pos)];
            let mut steps = 0;
            while cand >= 0 && steps < CHAIN {
                let c = cand as usize;
                let dist = pos - c;
                if dist > MAX_DIST {
                    break;
                }
                let n = self.match_len(pos, dist, limit);
                // a short match far away costs more than its literals
                let worth = n >= 4 || (n == 3 && dist <= 4096);
                if worth && n * 2 > score {
                    score = n * 2;
                    best = Some((n, None, dist));
                    if n == limit {
                        break;
                    }
                }
                cand = self.chain[c];
                steps += 1;
            }
        }
        best
    }

    fn apply_repeats(r: &mut [u32; 3], repeat: Option<u8>, dist: u32) {
        match repeat {
            Some(0) => {}
            Some(1) => r.swap(0, 1),
            Some(_) => r.swap(0, 2),
            None => {
                r[2] = r[1];
                r[1] = r[0];
                r[0] = dist;
            }
        }
    }

    fn tokenize(&mut self, start: usize, end: usize) -> Vec<Token> {
        let mut tokens = Vec::with_capacity(end - start);
        let mut pos = start;
        while pos < end {
            let here = self.find(pos, end);
            let Some((len, repeat, dist)) = here else {
                tokens.push(Token::Literal(self.data[pos]));
                pos += 1;
                continue;
            };
            // lazy step: a clearly longer match one byte on beats this one
            if len < 32 && pos + 1 < end {
                if let Some((l2, _, _)) = self.find(pos + 1, end) {
                    if l2 > len + 1 {
                        tokens.push(Token::Literal(self.data[pos]));
                        pos += 1;
                        continue;
                    }
                }
            }
            Self::apply_repeats(&mut self.r, repeat, dist as u32);
            tokens.push(Token::Match { len: len as u16, repeat, dist: dist as u32 });
            pos += len;
        }
        self.insert_to(end);
        tokens
    }

    fn slot_of(&self, formatted: u32) -> usize {
        let mut s = POSITION_SLOTS - 1;
        while self.base[s] > formatted {
            s -= 1;
        }
        s
    }

    /// Symbol of a match: (main symbol, length symbol if any, extra bits value and count).
    fn describe(&self, len: u16, repeat: Option<u8>, dist: u32) -> Described {
        let lm = len as usize - MIN_MATCH;
        let header = lm.min(7);
        let (slot, extra_val, extra_n) = match repeat {
            Some(i) => (i as usize, 0, 0),
            None => {
                let f = dist + 2;
                let s = self.slot_of(f);
                (s, f - self.base[s], self.extra[s])
            }
        };
        let main = 256 + slot * 8 + header;
        let len_sym = if header == 7 { Some(lm - 7) } else { None };
        (main, len_sym, extra_val, extra_n)
    }

    /// One frame as a verbatim block, or `None` when it would not beat an uncompressed block.
    fn verbatim_block(&mut self, tokens: &[Token], frame_len: usize, first: bool) -> Option<Vec<u8>> {
        let described: Vec<Option<Described>> = tokens
            .iter()
            .map(|t| match *t {
                Token::Literal(_) => None,
                Token::Match { len, repeat, dist } => Some(self.describe(len, repeat, dist)),
            })
            .collect();
        let mut mf = vec![0u32; MAIN_SYMBOLS];
        let mut lf = vec![0u32; LENGTH_SYMBOLS];
        for (t, d) in tokens.iter().zip(&described) {
            match (t, d) {
                (Token::Literal(b), _) => mf[*b as usize] += 1,
                (_, Some((m, l, _, _))) => {
                    mf[*m] += 1;
                    if let Some(l) = l {
                        lf[*l] += 1;
                    }
                }
                _ => unreachable!(),
            }
        }
        let main_lens = huffman_lengths(&mf, 16);
        let len_lens = huffman_lengths(&lf, 16);
        let main_codes = canonical_codes(&main_lens);
        let len_codes = canonical_codes(&len_lens);

        let mut w = BitWriter::new();
        if first {
            w.bits(0, 1);
        }
        w.bits(1, 3);
        w.bits(frame_len as u32, 24);
        let mut pm = self.prev_main.clone();
        let mut pl = self.prev_len.clone();
        write_tree(&mut w, &main_lens, &mut pm, 0, 256);
        write_tree(&mut w, &main_lens, &mut pm, 256, MAIN_SYMBOLS);
        write_tree(&mut w, &len_lens, &mut pl, 0, LENGTH_SYMBOLS);
        for (t, d) in tokens.iter().zip(&described) {
            match (t, d) {
                (Token::Literal(b), _) => w.bits(main_codes[*b as usize], main_lens[*b as usize]),
                (_, Some((m, l, ev, en))) => {
                    w.bits(main_codes[*m], main_lens[*m]);
                    if let Some(l) = l {
                        w.bits(len_codes[*l], len_lens[*l]);
                    }
                    if *en > 0 {
                        w.bits(*ev, *en);
                    }
                }
                _ => unreachable!(),
            }
        }
        w.pad();
        if w.out.len() >= frame_len + 24 || w.out.len() > 0xFFFF {
            return None;
        }
        self.prev_main = pm;
        self.prev_len = pl;
        Some(w.out)
    }
}

fn stored_block(frame: &[u8], first: bool) -> Vec<u8> {
    let mut w = BitWriter::new();
    if first {
        w.bits(0, 1);
    }
    w.bits(3, 3);
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
    block
}

/// Compresses `data` into XMem frames (128 KiB window), without the end-of-stream terminator.
pub fn xmem_compress(data: &[u8]) -> Vec<u8> {
    let mut enc = Encoder::new(data);
    let mut stream = Vec::with_capacity(data.len() / 2 + 64);
    for (i, frame) in data.chunks(FRAME).enumerate() {
        let start = i * FRAME;
        let end = start + frame.len();
        let tokens = enc.tokenize(start, end);
        let block = match enc.verbatim_block(&tokens, frame.len(), i == 0) {
            Some(b) => b,
            None => {
                // an uncompressed block loads the three registers with the values it carries
                enc.r = [1, 1, 1];
                stored_block(frame, i == 0)
            }
        };
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

    fn roundtrip(data: &[u8]) -> usize {
        let stream = xmem_compress(data);
        assert_eq!(
            xmem_decompress(&stream, data.len(), WindowSize::KB128).unwrap(),
            data,
            "len {}",
            data.len()
        );
        if !data.is_empty() {
            assert_eq!(xmem_decompress_all(&stream, WindowSize::KB128).unwrap(), data);
        }
        stream.len()
    }

    fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                (s >> 56) as u8
            })
            .collect()
    }

    #[test]
    fn edge_sizes_roundtrip() {
        for len in [0usize, 1, 2, 3, 7, 100, FRAME - 1, FRAME, FRAME + 1, 3 * FRAME + 777] {
            roundtrip(&(0..len).map(|i| (i * 7 % 13) as u8).collect::<Vec<_>>());
            roundtrip(&noise(len, len as u64 + 1));
        }
    }

    #[test]
    fn repetitive_data_shrinks() {
        let data: Vec<u8> = (0..200_000).map(|i| (i % 97) as u8).collect();
        assert!(roundtrip(&data) < data.len() / 20);
        let zeros = vec![0u8; 300_000];
        assert!(roundtrip(&zeros) < 3_000);
    }

    #[test]
    fn mixed_structured_data_roundtrips_across_frames() {
        // text-ish runs with far repeats (exercises repeat offsets and the 128 KiB window)
        let mut data = Vec::new();
        let words = [&b"minecraft:stone "[..], b"grass_block ", b"{x:12,z:-4} ", b"\0\0\0\0\x10\x20"];
        let mut s = 12345u32;
        while data.len() < 400_000 {
            s = s.wrapping_mul(1103515245).wrapping_add(12345);
            data.extend_from_slice(words[(s >> 16) as usize % words.len()]);
            if s % 17 == 0 {
                data.push((s >> 8) as u8);
            }
        }
        let n = roundtrip(&data);
        assert!(n < data.len() / 3, "{n} of {}", data.len());
    }

    #[test]
    fn incompressible_falls_back_to_stored_and_mixes() {
        let mut data = noise(FRAME * 2, 9);
        data.extend(std::iter::repeat_n(b'a', FRAME));
        data.extend(noise(FRAME + 5, 11));
        data.extend((0..FRAME).map(|i| (i % 5) as u8));
        let n = roundtrip(&data);
        assert!(n < data.len());
    }
}
