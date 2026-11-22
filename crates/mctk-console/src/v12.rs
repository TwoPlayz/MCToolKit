//! Version 12 console chunks: the layout newer game versions store (a record's bit 30 marks it).
//!
//! Integers are big-endian unless noted. The stream is
//!
//! | part | content |
//! |---|---|
//! | header | `u16` version (12), `i32` x, `i32` z, `i64` last update, `i64` inhabited time |
//! | directory | `u16 LE` total size of all sections, 16 x `u16` section start, 16 x `u8` section size in 256-byte units |
//! | sections | one per 16-tall slice that holds anything, see below |
//! | light | sky light, then block light; each in two halves (8 x-columns each) |
//! | tail | height map (256), `u16` terrain-populated flags, biomes (256), an NBT compound |
//!
//! A section is a table of 64 `u16 LE` cell headers (4x4x4 cells of 4x4x4 blocks, index
//! `x*16 + z*4 + y`) followed by the cells' data, zero-padded so the whole section is a multiple of 256
//! bytes (an exact fit gets a further 256). A block value is `(id << 4) | data`; every cell also has a
//! second *liquid* layer in the same encoding. A cell header is
//!
//! * a value below `0x1000`: all 64 blocks have that value (no liquid), nothing else is stored;
//! * otherwise `code << 13 | has_liquid << 12 | offset / 4`: `code` 1..=4 is a palette of 2, 4, 8 or 16
//!   `u16 LE` entries (padded with `0xFFFF`) followed by `code` `u64` words holding the 64 indices
//!   bit-planar (word *j* has bit *j* of every index, element *i* at bit `63 - i`); `code` 7 is 64 plain
//!   `u16 LE` values. With `has_liquid`, the same again for the liquid layer follows (indices into the
//!   same palette, or 64 more values).
//!
//! The light of a chunk is stored as 128 *tiles* per half: tile *e* holds, for the 8 x 16 columns of the
//! half (x outer, z inner), the nibble at height *e* in its low half and at *e + 128* in its high half. A
//! 128-byte table says per tile: `0x80` all zero, `0x81` all 15 (sky light only), else the index of the stored
//! tile. Each half is `u32` tile count, table, tiles.
//!
//! Every rule here was checked against chunks written by a real converter: all four captured chunks decode and
//! re-encode to the same bytes.

use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};

use crate::{Error, bad, console_block_index};

/// Number of cells in a chunk (16 x 16 x 256).
pub const CELLS: usize = 65_536;
const LIGHT_BYTES: usize = CELLS / 2;
const SECTIONS: usize = 16;
const VERSION: u16 = 12;

/// Bit 15 of a block value: the block sits in water (with the water in the liquid layer). The rest of
/// the value is then an ordinary `(id << 4) | data` for a block that exists in the one-byte-id world
/// (stairs, fences, chests: `0x8352` is oak stairs, data 2, in water), or `0` (`0x8000`: just water).
/// Values with ids from 256 up (`0x9020`, `0x90E0`...: bit 12 set as well) are other blocks that live in
/// water and are not mapped.
pub const IN_WATER: u16 = 0x8000;

/// Optional `Level` key of a tree given to [`ChunkV12::from_level_tree`]: a byte array of 65,536 flags in
/// the tree's block order, non-zero for a block that sits in water (see [`IN_WATER`]).
pub const IN_WATER_KEY: &str = "MctkInWater";

/// Optional `Level` key of a tree given to [`ChunkV12::from_level_tree`]: an `IntArray` of (cell, value) pairs, the
/// cell as an index into the tree's block arrays, for blocks that only exist under an id above 255. The value
/// is written as it is (with its liquid layer when it has bit 15 set).
pub const EXTENDED_KEY: &str = "MctkExtended";

/// What a cell becomes in the one-byte-id `Level` tree: its own block if the id is 1..=255; a block in
/// water keeps the block (the water cannot be kept as well: the tree has one layer); otherwise its
/// liquid layer if that fits (an unmapped extended id is underwater, so this keeps the water); otherwise
/// empty.
pub fn tree_value(block: u16, liquid: u16) -> u16 {
    let fits = |v: u16| v != 0 && v >> 4 <= 255;
    if block >> 4 == 257 {
        // a plain pumpkin: the nearest one-byte block is the pumpkin with a face
        return 0x860;
    }
    if fits(block) {
        block
    } else if block & IN_WATER != 0 && fits(block & !IN_WATER) {
        block & !IN_WATER
    } else if fits(liquid) {
        liquid
    } else {
        0
    }
}

/// Index of block `(x, y, z)` in the arrays of [`ChunkV12`].
pub fn cell(x: usize, y: usize, z: usize) -> usize {
    x * 4096 + z * 256 + y
}

fn nibble(a: &[u8], i: usize) -> u8 {
    if i % 2 == 0 { a[i / 2] & 15 } else { a[i / 2] >> 4 }
}

fn set_nibble(a: &mut [u8], i: usize, v: u8) {
    let b = &mut a[i / 2];
    *b = if i % 2 == 0 { (*b & 0xf0) | (v & 15) } else { (*b & 0x0f) | (v << 4) };
}

/// A Version 12 chunk with its data in the layout of the file.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkV12 {
    pub x: i32,
    pub z: i32,
    pub last_update: i64,
    pub inhabited_time: i64,
    /// `(id << 4) | data` per cell, index [`cell`].
    pub blocks: Vec<u16>,
    /// The second layer of each cell, same encoding (0 = none).
    pub liquid: Vec<u16>,
    /// One nibble per cell, 32,768 bytes, nibble index [`cell`].
    pub sky_light: Vec<u8>,
    pub block_light: Vec<u8>,
    /// 256 bytes.
    pub height_map: Vec<u8>,
    pub terrain_populated: u16,
    /// 256 bytes.
    pub biomes: Vec<u8>,
    /// Root compound holding the lists `TileTicks`, `TileEntities` and `Entities`.
    pub extra: NamedTag,
    /// The tail NBT exactly as read. Entity attribute compounds can hold two tags of the same name with
    /// different types (a `Base` double and a `Base` float were seen), which a name-keyed compound cannot,
    /// so [`to_bytes`](Self::to_bytes) writes these bytes back unchanged while `extra` still equals what
    /// they parse to, and serialises `extra` only after it was edited.
    pub extra_raw: Option<Vec<u8>>,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).filter(|e| *e <= self.b.len()).ok_or_else(|| bad("Version 12 chunk is truncated"))?;
        let s = &self.b[self.at..end];
        self.at = end;
        Ok(s)
    }
    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, Error> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
}

/// Whether decompressed chunk bytes start with the Version 12 header.
pub fn is_v12(bytes: &[u8]) -> bool {
    bytes.len() > 26 && bytes[0] == 0 && bytes[1] == VERSION as u8
}

impl ChunkV12 {
    /// Parses a decompressed Version 12 chunk.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let mut r = Reader { b: bytes, at: 0 };
        let version = r.u16()?;
        if version != VERSION {
            return Err(bad(format!("chunk version {version}, expected {VERSION}")));
        }
        let x = r.u32()? as i32;
        let z = r.u32()? as i32;
        let last_update = r.i64()?;
        let inhabited_time = r.i64()?;
        r.take(2)?; // total section size as u16 LE; the section sizes below are exact
        let mut starts = [0usize; SECTIONS];
        for s in starts.iter_mut() {
            *s = r.u16()? as usize;
        }
        let lens: Vec<usize> = r.take(SECTIONS)?.iter().map(|b| *b as usize * 256).collect();
        let base = r.at;
        let sections = r.take(lens.iter().sum())?;

        let mut blocks = vec![0u16; CELLS];
        let mut liquid = vec![0u16; CELLS];
        for s in 0..SECTIONS {
            if lens[s] == 0 {
                continue;
            }
            let blob = sections
                .get(starts[s]..starts[s] + lens[s])
                .ok_or_else(|| bad(format!("section {s} lies outside the section data (base {base})")))?;
            read_section(blob, s, &mut blocks, &mut liquid)?;
        }

        let mut sky_light = vec![0u8; LIGHT_BYTES];
        let mut block_light = vec![0u8; LIGHT_BYTES];
        read_light(&mut r, &mut sky_light)?;
        read_light(&mut r, &mut block_light)?;
        let height_map = r.take(256)?.to_vec();
        let terrain_populated = r.u16()?;
        let biomes = r.take(256)?.to_vec();
        let extra = mctk_nbt::from_bytes(&bytes[r.at..], Flavor::Java)?;
        let extra_raw = Some(bytes[r.at..].to_vec());
        Ok(ChunkV12 {
            x,
            z,
            last_update,
            inhabited_time,
            blocks,
            liquid,
            sky_light,
            block_light,
            height_map,
            terrain_populated,
            biomes,
            extra,
            extra_raw,
        })
    }

    /// Encodes the chunk the way the real converter does (byte for byte on the chunks checked).
    pub fn to_bytes(&self) -> Result<Vec<u8>, Error> {
        if self.blocks.len() != CELLS || self.liquid.len() != CELLS {
            return Err(bad("Version 12 chunk needs 65,536 block and liquid cells"));
        }
        if self.sky_light.len() != LIGHT_BYTES || self.block_light.len() != LIGHT_BYTES {
            return Err(bad("Version 12 chunk needs 32,768-byte light arrays"));
        }
        if self.height_map.len() != 256 || self.biomes.len() != 256 {
            return Err(bad("Version 12 chunk needs 256-byte height map and biomes"));
        }
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        let mut starts = [0u16; SECTIONS];
        let mut lens = [0u8; SECTIONS];
        let mut total = 0usize;
        for s in 0..SECTIONS {
            starts[s] = u16::try_from(total).map_err(|_| bad("Version 12 sections exceed 64 KiB"))?;
            if let Some(blob) = encode_section(s, &self.blocks, &self.liquid) {
                lens[s] = u8::try_from(blob.len() / 256).map_err(|_| bad("Version 12 section larger than 255 x 256 bytes"))?;
                total += blob.len();
                blobs.push(blob);
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(&VERSION.to_be_bytes());
        out.extend_from_slice(&self.x.to_be_bytes());
        out.extend_from_slice(&self.z.to_be_bytes());
        out.extend_from_slice(&self.last_update.to_be_bytes());
        out.extend_from_slice(&self.inhabited_time.to_be_bytes());
        out.extend_from_slice(&(total as u16).to_le_bytes());
        for s in starts {
            out.extend_from_slice(&s.to_be_bytes());
        }
        out.extend_from_slice(&lens);
        for b in &blobs {
            out.extend_from_slice(b);
        }
        write_light(&self.sky_light, true, &mut out);
        write_light(&self.block_light, false, &mut out);
        out.extend_from_slice(&self.height_map);
        out.extend_from_slice(&self.terrain_populated.to_be_bytes());
        out.extend_from_slice(&self.biomes);
        match &self.extra_raw {
            Some(raw) if mctk_nbt::from_bytes(raw, Flavor::Java).is_ok_and(|t| t == self.extra) => out.extend_from_slice(raw),
            _ => out.extend_from_slice(&mctk_nbt::to_bytes(&self.extra, Flavor::Java)?),
        }
        Ok(out)
    }

    /// The McRegion-style `Level` tree the rest of the crate works with (block ids below 256 and a data nibble
    /// per block, lights in the older half layout). The liquid layer has no place there: see [`tree_value`] for
    /// what each cell becomes (extended ids lose their block). `InhabitedTime` is kept.
    pub fn to_level_tree(&self) -> NamedTag {
        let mut ids = vec![0u8; CELLS];
        let mut data = vec![0u8; LIGHT_BYTES];
        let mut sky = vec![0u8; LIGHT_BYTES];
        let mut light = vec![0u8; LIGHT_BYTES];
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..256 {
                    let (c, t) = (cell(x, y, z), console_block_index(x, y, z));
                    let v = tree_value(self.blocks[c], self.liquid[c]);
                    ids[t] = (v >> 4) as u8;
                    set_nibble(&mut data, t, (v & 15) as u8);
                    set_nibble(&mut sky, t, nibble(&self.sky_light, c));
                    set_nibble(&mut light, t, nibble(&self.block_light, c));
                }
            }
        }
        let signed = |v: Vec<u8>| Tag::ByteArray(v.into_iter().map(|b| b as i8).collect());
        let mut level = Compound::new();
        level.insert("xPos".into(), Tag::Int(self.x));
        level.insert("zPos".into(), Tag::Int(self.z));
        level.insert("LastUpdate".into(), Tag::Long(self.last_update));
        level.insert("InhabitedTime".into(), Tag::Long(self.inhabited_time));
        level.insert("Blocks".into(), signed(ids));
        level.insert("Data".into(), signed(data));
        level.insert("SkyLight".into(), signed(sky));
        level.insert("BlockLight".into(), signed(light));
        level.insert("HeightMap".into(), signed(self.height_map.clone()));
        level.insert("TerrainPopulatedFlags".into(), Tag::Short(self.terrain_populated as i16));
        level.insert("Biomes".into(), signed(self.biomes.clone()));
        if let Tag::Compound(c) = &self.extra.tag {
            for (k, v) in c {
                level.insert(k.clone(), v.clone());
            }
        }
        let mut root = Compound::new();
        root.insert("Level".into(), Tag::Compound(level));
        NamedTag { name: String::new(), tag: Tag::Compound(root) }
    }

    /// Builds a Version 12 chunk from a `Level` tree of 256-tall blocks (the inverse of
    /// [`to_level_tree`](Self::to_level_tree); the liquid layer starts empty).
    pub fn from_level_tree(tree: &NamedTag) -> Result<Self, Error> {
        let level = tree.tag.get("Level").ok_or_else(|| bad("chunk without Level"))?;
        let int = |k: &str| level.get(k).and_then(Tag::as_i64);
        let array = |k: &str, len: usize| -> Result<Vec<u8>, Error> {
            match level.get(k) {
                Some(Tag::ByteArray(a)) if a.len() == len => Ok(a.iter().map(|&b| b as u8).collect()),
                Some(Tag::ByteArray(a)) => Err(bad(format!("{k} has {} bytes, a Version 12 chunk needs {len}", a.len()))),
                _ => Err(bad(format!("chunk without {k}"))),
            }
        };
        let (ids, data) = (array("Blocks", CELLS)?, array("Data", LIGHT_BYTES)?);
        let (sky, light) = (array("SkyLight", LIGHT_BYTES)?, array("BlockLight", LIGHT_BYTES)?);
        let mut blocks = vec![0u16; CELLS];
        let mut sky_light = vec![0u8; LIGHT_BYTES];
        let mut block_light = vec![0u8; LIGHT_BYTES];
        for x in 0..16 {
            for z in 0..16 {
                for y in 0..256 {
                    let (c, t) = (cell(x, y, z), console_block_index(x, y, z));
                    blocks[c] = (u16::from(ids[t]) << 4) | u16::from(nibble(&data, t));
                    set_nibble(&mut sky_light, c, nibble(&sky, t));
                    set_nibble(&mut block_light, c, nibble(&light, t));
                }
            }
        }
        // Blocks flagged as in water get bit 15 and flowing water in the liquid layer, as the original
        // converter writes them.
        let mut liquid = vec![0u16; CELLS];
        if let Some(Tag::ByteArray(w)) = level.get(IN_WATER_KEY) {
            if w.len() == CELLS {
                for x in 0..16 {
                    for z in 0..16 {
                        for y in 0..256 {
                            let (c, t) = (cell(x, y, z), console_block_index(x, y, z));
                            if w[t] != 0 {
                                blocks[c] |= IN_WATER;
                                liquid[c] = 0x80;
                            }
                        }
                    }
                }
            }
        }
        if let Some(Tag::IntArray(pairs)) = level.get(EXTENDED_KEY) {
            for p in pairs.chunks_exact(2) {
                // the inverse of `console_block_index`
                let (t, value) = (p[0] as usize, p[1] as u16);
                let (half, r) = (t / 32768, t % 32768);
                let (x, z, y) = (r / 2048, (r % 2048) / 128, half * 128 + r % 128);
                if x < 16 && y < 256 {
                    let c = cell(x, y, z);
                    blocks[c] = value;
                    liquid[c] = if value & IN_WATER != 0 { 0x80 } else { 0 };
                }
            }
        }
        let mut extra = Compound::new();
        for key in ["TileTicks", "TileEntities", "Entities"] {
            extra.insert(key.into(), level.get(key).cloned().unwrap_or_else(|| Tag::List(List::new(TagType::Compound))));
        }
        Ok(ChunkV12 {
            x: int("xPos").ok_or_else(|| bad("chunk without xPos"))? as i32,
            z: int("zPos").ok_or_else(|| bad("chunk without zPos"))? as i32,
            last_update: int("LastUpdate").unwrap_or(0),
            inhabited_time: int("InhabitedTime").unwrap_or(0),
            blocks,
            liquid,
            sky_light,
            block_light,
            height_map: array("HeightMap", 256)?,
            terrain_populated: int("TerrainPopulatedFlags").unwrap_or(0) as u16,
            biomes: array("Biomes", 256)?,
            extra: NamedTag { name: String::new(), tag: Tag::Compound(extra) },
            extra_raw: None,
        })
    }
}

// --- sections ---------------------------------------------------------------------------------

fn read_section(blob: &[u8], s: usize, blocks: &mut [u16], liquid: &mut [u16]) -> Result<(), Error> {
    if blob.len() < 128 {
        return Err(bad("Version 12 section shorter than its cell table"));
    }
    let data = &blob[128..];
    let u16le = |off: usize| -> Result<u16, Error> {
        data.get(off..off + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).ok_or_else(|| bad("Version 12 cell data is truncated"))
    };
    let u64be = |off: usize| -> Result<u64, Error> {
        data.get(off..off + 8)
            .map(|b| u64::from_be_bytes(b.try_into().unwrap()))
            .ok_or_else(|| bad("Version 12 cell data is truncated"))
    };
    for ci in 0..64 {
        let h = u16::from_le_bytes([blob[2 * ci], blob[2 * ci + 1]]);
        let mut vals = [0u16; 64];
        let mut liq = [0u16; 64];
        if h & 0xF000 == 0 {
            vals = [h; 64];
        } else {
            let code = (h >> 13) as usize;
            let has_liquid = h & 0x1000 != 0;
            let mut at = (h & 0x0FFF) as usize * 4;
            match code {
                7 => {
                    for v in vals.iter_mut() {
                        *v = u16le(at)?;
                        at += 2;
                    }
                    if has_liquid {
                        for v in liq.iter_mut() {
                            *v = u16le(at)?;
                            at += 2;
                        }
                    }
                }
                1..=4 => {
                    let bits = code;
                    let mut palette = [0u16; 16];
                    for p in palette.iter_mut().take(1 << bits) {
                        *p = u16le(at)?;
                        at += 2;
                    }
                    for layer in 0..(1 + usize::from(has_liquid)) {
                        let mut words = [0u64; 4];
                        for w in words.iter_mut().take(bits) {
                            *w = u64be(at)?;
                            at += 8;
                        }
                        for i in 0..64 {
                            let idx = (0..bits).fold(0usize, |acc, j| acc | (((words[j] >> (63 - i)) & 1) as usize) << j);
                            let v = palette[idx];
                            if v == 0xFFFF {
                                return Err(bad("Version 12 cell index points at palette padding"));
                            }
                            if layer == 0 { vals[i] = v } else { liq[i] = v }
                        }
                    }
                }
                _ => return Err(bad(format!("unknown Version 12 cell header {h:#06x}"))),
            }
        }
        let (xc, zc, yc) = (ci / 16, (ci / 4) % 4, ci % 4);
        for i in 0..64 {
            let (a, b, c) = (i / 16, (i / 4) % 4, i % 4);
            let idx = cell(xc * 4 + a, s * 16 + yc * 4 + c, zc * 4 + b);
            blocks[idx] = vals[i];
            liquid[idx] = liq[i];
        }
    }
    Ok(())
}

/// One section, or `None` when it holds neither blocks nor liquid.
fn encode_section(s: usize, blocks: &[u16], liquid: &[u16]) -> Option<Vec<u8>> {
    let at = |xc: usize, zc: usize, yc: usize, i: usize| {
        let (a, b, c) = (i / 16, (i / 4) % 4, i % 4);
        cell(xc * 4 + a, s * 16 + yc * 4 + c, zc * 4 + b)
    };
    let mut any = false;
    for xc in 0..4 {
        for zc in 0..4 {
            for yc in 0..4 {
                any |= (0..64).any(|i| blocks[at(xc, zc, yc, i)] != 0 || liquid[at(xc, zc, yc, i)] != 0);
            }
        }
    }
    if !any {
        return None;
    }
    let mut header = [0u16; 64];
    let mut data: Vec<u8> = Vec::new();
    for xc in 0..4 {
        for zc in 0..4 {
            for yc in 0..4 {
                let ci = xc * 16 + zc * 4 + yc;
                let vals: Vec<u16> = (0..64).map(|i| blocks[at(xc, zc, yc, i)]).collect();
                let liq: Vec<u16> = (0..64).map(|i| liquid[at(xc, zc, yc, i)]).collect();
                // The palette in order of first appearance: each block value, then (when there is liquid)
                // the liquid value and the empty value.
                let mut palette: Vec<u16> = Vec::new();
                let add = |p: &mut Vec<u16>, v: u16| {
                    if !p.contains(&v) {
                        p.push(v);
                    }
                };
                let mut has_liquid = false;
                for i in 0..64 {
                    add(&mut palette, vals[i]);
                    if liq[i] != 0 {
                        has_liquid = true;
                        add(&mut palette, liq[i]);
                        add(&mut palette, 0);
                    }
                }
                if palette.len() == 1 && palette[0] & 0xF000 == 0 {
                    header[ci] = palette[0]; // one value, small enough to be its own header
                    continue;
                }
                if palette.len() == 1 {
                    palette.push(0); // a value too large for a header gets a two-entry palette
                }
                let offset = u16::try_from(data.len() / 4).expect("cell data offset fits 12 bits");
                if palette.len() > 16 {
                    header[ci] = offset.wrapping_add(7 * 0x2000) | if has_liquid { 0x1000 } else { 0 };
                    for v in &vals {
                        data.extend_from_slice(&v.to_le_bytes());
                    }
                    if has_liquid {
                        for v in &liq {
                            data.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                } else {
                    let bits = match palette.len() {
                        2 => 1,
                        3..=4 => 2,
                        5..=8 => 3,
                        _ => 4,
                    };
                    header[ci] = offset + bits as u16 * 0x2000 + if has_liquid { 0x1000 } else { 0 };
                    let mut padded = palette.clone();
                    padded.resize(1 << bits, 0xFFFF);
                    for v in &padded {
                        data.extend_from_slice(&v.to_le_bytes());
                    }
                    let mut layers: Vec<&Vec<u16>> = vec![&vals];
                    if has_liquid {
                        layers.push(&liq);
                    }
                    for layer in layers {
                        let mut words = [0u64; 4];
                        for (i, v) in layer.iter().enumerate() {
                            let idx = palette.iter().position(|p| p == v).expect("value is in its palette");
                            for (j, w) in words.iter_mut().enumerate().take(bits) {
                                *w |= (((idx >> j) & 1) as u64) << (63 - i);
                            }
                        }
                        for w in words.iter().take(bits) {
                            data.extend_from_slice(&w.to_be_bytes());
                        }
                    }
                }
            }
        }
    }
    // 128-byte table, the data, then padding up to the next multiple of 256 past the end (always at least 1 byte more).
    let total = (128 + data.len()) / 256 * 256 + 256;
    let mut blob = Vec::with_capacity(total);
    for h in header {
        blob.extend_from_slice(&h.to_le_bytes());
    }
    blob.extend_from_slice(&data);
    blob.resize(total, 0);
    Some(blob)
}

// --- light ------------------------------------------------------------------------------------

fn read_light(r: &mut Reader, out: &mut [u8]) -> Result<(), Error> {
    for half in 0..2 {
        let count = r.u32()? as usize;
        let table = r.take(128)?;
        let tiles = r.take(count.checked_mul(128).ok_or_else(|| bad("Version 12 tile count overflows"))?)?;
        for (e, &entry) in table.iter().enumerate() {
            let tile: Option<&[u8]> = match entry {
                0x80 => continue,
                0x81 => None,
                i if (i as usize) < count => Some(&tiles[i as usize * 128..i as usize * 128 + 128]),
                i => return Err(bad(format!("Version 12 light tile index {i} out of range ({count} stored)"))),
            };
            for s in 0..128 {
                let (lo, hi) = tile.map_or((15, 15), |t| (t[s] & 15, t[s] >> 4));
                let base = cell(half * 8 + s / 16, 0, s % 16);
                set_nibble(out, base + e, lo);
                set_nibble(out, base + e + 128, hi);
            }
        }
    }
    Ok(())
}

fn write_light(light: &[u8], sky: bool, out: &mut Vec<u8>) {
    for half in 0..2 {
        let mut table = [if sky { 0x81u8 } else { 0x80 }; 128];
        let mut tiles: Vec<u8> = Vec::new();
        let mut count = 0u8;
        for (e, entry) in table.iter_mut().enumerate() {
            let tile: Vec<u8> = (0..128)
                .map(|s| {
                    let base = cell(half * 8 + s / 16, 0, s % 16);
                    nibble(light, base + e) | (nibble(light, base + e + 128) << 4)
                })
                .collect();
            let zero = tile.iter().all(|b| *b == 0);
            let full = tile.iter().all(|b| *b == 0xff);
            if sky {
                if full {
                    continue; // stays 0x81
                }
                if zero {
                    *entry = 0x80;
                    continue;
                }
            } else if zero {
                continue; // stays 0x80
            }
            *entry = count;
            count += 1;
            tiles.extend_from_slice(&tile);
        }
        out.extend_from_slice(&u32::from(count).to_be_bytes());
        out.extend_from_slice(&table);
        out.extend_from_slice(&tiles);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk() -> ChunkV12 {
        let mut c = ChunkV12 {
            x: -3,
            z: 7,
            last_update: 5,
            inhabited_time: 99,
            blocks: vec![0; CELLS],
            liquid: vec![0; CELLS],
            sky_light: vec![0xff; LIGHT_BYTES],
            block_light: vec![0; LIGHT_BYTES],
            height_map: vec![64; 256],
            terrain_populated: 0x7fe,
            biomes: vec![1; 256],
            extra: NamedTag { name: String::new(), tag: Tag::Compound(Compound::new()) },
            extra_raw: None,
        };
        for x in 0..16 {
            for z in 0..16 {
                c.blocks[cell(x, 0, z)] = 0x70; // bedrock
                for y in 1..60 {
                    c.blocks[cell(x, y, z)] = if (x + z + y) % 11 == 0 { 0x11 } else { 0x10 };
                }
                c.blocks[cell(x, 60, z)] = 0x30;
                c.blocks[cell(x, 61, z)] = 0x20;
            }
        }
        c.liquid[cell(2, 61, 3)] = 0x80;
        c.blocks[cell(5, 70, 5)] = 0x1234; // an id above 255 that still fits a raw cell
        c.block_light[100] = 0x35;
        c
    }

    #[test]
    fn round_trips() {
        let c = chunk();
        let bytes = c.to_bytes().unwrap();
        assert!(is_v12(&bytes));
        let mut back = ChunkV12::parse(&bytes).unwrap();
        assert!(back.extra_raw.is_some());
        back.extra_raw = None; // only parsing sets it
        assert_eq!(back, c);
        assert_eq!(ChunkV12::parse(&bytes).unwrap().to_bytes().unwrap(), bytes, "encoding is stable");
    }

    #[test]
    fn blocks_in_water_keep_their_block_in_the_tree() {
        // stairs in water, fence in water, plain water, an unmapped aquatic id with water
        assert_eq!(tree_value(0x8352, 0x80), 0x352);
        assert_eq!(tree_value(0x8550, 0x80), 0x550);
        assert_eq!(tree_value(0x8000, 0x80), 0x80);
        assert_eq!(tree_value(0x9020, 0x80), 0x80);
        assert_eq!(tree_value(0x10, 0), 0x10);
    }

    #[test]
    fn flagged_blocks_are_written_in_water_and_survive_a_round_trip() {
        let c = chunk();
        let mut tree = c.to_level_tree();
        let mut flags = vec![0i8; CELLS];
        let at = console_block_index(5, 61, 9);
        flags[at] = 1;
        tree.tag.as_compound_mut().unwrap().get_mut("Level").unwrap().as_compound_mut().unwrap().insert(IN_WATER_KEY.into(), Tag::ByteArray(flags));
        let v = ChunkV12::from_level_tree(&tree).unwrap();
        let i = cell(5, 61, 9);
        assert_eq!(v.blocks[i], IN_WATER | c.blocks[i]);
        assert_eq!(v.liquid[i], 0x80);
        let again = ChunkV12::parse(&v.to_bytes().unwrap()).unwrap();
        assert_eq!((again.blocks[i], again.liquid[i]), (v.blocks[i], 0x80));
        // reading it back as a tree keeps the block
        let back = again.to_level_tree();
        let level = back.tag.get("Level").unwrap();
        let Some(Tag::ByteArray(b)) = level.get("Blocks") else { panic!() };
        assert_eq!(b[at] as u8, (c.blocks[i] >> 4) as u8);
    }

    #[test]
    fn extended_cells_are_written_as_given_with_their_water() {
        let c = chunk();
        let mut tree = c.to_level_tree();
        let at = console_block_index(3, 62, 4);
        let (seagrass, pumpkin) = (0x90E0u16, 0x1010u16);
        let at2 = console_block_index(7, 70, 2);
        let pairs = vec![at as i32, i32::from(seagrass), at2 as i32, i32::from(pumpkin)];
        tree.tag.as_compound_mut().unwrap().get_mut("Level").unwrap().as_compound_mut().unwrap().insert(EXTENDED_KEY.into(), Tag::IntArray(pairs));
        let v = ChunkV12::parse(&ChunkV12::from_level_tree(&tree).unwrap().to_bytes().unwrap()).unwrap();
        assert_eq!((v.blocks[cell(3, 62, 4)], v.liquid[cell(3, 62, 4)]), (seagrass, 0x80));
        assert_eq!((v.blocks[cell(7, 70, 2)], v.liquid[cell(7, 70, 2)]), (pumpkin, 0));
        // read back as a tree: the seagrass leaves its water, the pumpkin stands in as the faced one
        assert_eq!(tree_value(seagrass, 0x80), 0x80);
        assert_eq!(tree_value(pumpkin, 0), 0x860);
    }

    #[test]
    fn many_distinct_values_use_raw_cells() {
        let mut c = chunk();
        for (i, y) in (0..4).enumerate() {
            for z in 0..4 {
                for x in 0..4 {
                    c.blocks[cell(x, 100 + y, z)] = 0x10 * (1 + (i * 16 + z * 4 + x) as u16 % 40); // 20+ distinct values
                }
            }
        }
        let back = ChunkV12::parse(&c.to_bytes().unwrap()).unwrap();
        assert_eq!(back.blocks, c.blocks);
    }

    #[test]
    fn level_tree_round_trip_keeps_blocks_and_light() {
        let c = chunk();
        let tree = c.to_level_tree();
        let again = ChunkV12::from_level_tree(&tree).unwrap();
        // The liquid layer and ids above 255 have no place in the tree.
        for i in 0..CELLS {
            assert_eq!(again.blocks[i], tree_value(c.blocks[i], c.liquid[i]), "cell {i}");
        }
        assert_eq!(again.sky_light, c.sky_light);
        assert_eq!(again.block_light, c.block_light);
        assert_eq!((again.x, again.z, again.inhabited_time), (c.x, c.z, c.inhabited_time));
    }

    #[test]
    fn rejects_truncated_and_foreign_data() {
        let bytes = chunk().to_bytes().unwrap();
        for cut in [10, 40, 200, bytes.len() / 2, bytes.len() - 3] {
            assert!(ChunkV12::parse(&bytes[..cut]).is_err(), "cut at {cut}");
        }
        let mut other = bytes.clone();
        other[1] = 8;
        assert!(ChunkV12::parse(&other).is_err());
    }
}
