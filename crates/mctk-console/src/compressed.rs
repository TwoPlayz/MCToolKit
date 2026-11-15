//! Chunks of saves from version 8 on: the game's compressed binary form instead of NBT.
//!
//! The stream (integers big-endian) is `version: i16`, `x: i32`, `z: i32`, `time: i64`, then
//! for the lower (y 0-127) and upper (y 128-255) half of each of blocks, data, sky light and
//! block light a compressed storage, then `heightmap: [u8; 256]`, `terrain_populated: i16`,
//! `biomes: [u8; 256]` and an NBT compound with `Entities`, `TileEntities` and `TileTicks`.
//!
//! Block storage is `size: i32` and the bytes: 512 little-endian `u16` indices (one per 4x4x4
//! cell) followed by the cells' data. Light and data storage is `planes: i32` and
//! `128 + planes * 128` bytes: a plane index per y, then the stored planes of nibbles.
//! Decoding gives the McRegion-style `Level` tree the older saves store directly.

use mctk_nbt::{Compound, Flavor, NamedTag, Tag};

use crate::{Error, bad};

/// Whether a decompressed chunk is in this form rather than NBT, which starts with a compound
/// tag (`0x0A`) and an empty name.
pub fn is_compressed(bytes: &[u8]) -> bool {
    bytes.len() > 18 && !(bytes[0] == 0x0a && bytes[1] == 0 && bytes[2] == 0)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let s = self
            .bytes
            .get(self.at..self.at + n)
            .ok_or_else(|| bad("compressed chunk is truncated"))?;
        self.at += n;
        Ok(s)
    }

    fn int(&mut self) -> Result<i32, Error> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
}

/// Where cell `block` (0-511) and tile `tile` (0-63) sit in the 32768-byte half chunk.
fn index(block: usize, tile: usize) -> usize {
    ((block & 0x180) << 6)
        | ((block & 0x060) << 4)
        | ((block & 0x01f) << 2)
        | ((tile & 0x30) << 7)
        | ((tile & 0x0c) << 5)
        | (tile & 0x03)
}

fn blocks(data: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = vec![0u8; 32768];
    if data.is_empty() {
        return Ok(out);
    }
    if data.len() < 1024 {
        return Err(bad("block storage shorter than its index"));
    }
    let cells = &data[1024..];
    let get = |o: usize, n: usize| {
        cells
            .get(o..o + n)
            .ok_or_else(|| bad("block storage cell outside the data"))
    };
    for i in 0..512 {
        let idx = u16::from_le_bytes([data[2 * i], data[2 * i + 1]]) as usize;
        let kind = idx & 3;
        for j in 0..64 {
            out[index(i, j)] = 0;
        }
        if kind == 3 {
            if idx & 4 != 0 {
                let v = ((idx >> 8) & 0xff) as u8;
                (0..64).for_each(|j| out[index(i, j)] = v);
            } else {
                let packed = get((idx >> 1) & 0x7ffe, 64)?;
                (0..64).for_each(|j| out[index(i, j)] = packed[j]);
            }
        } else {
            let bits = 1usize << kind;
            let types = 1usize << bits;
            let shift = 3 - kind;
            let off = (idx >> 1) & 0x7ffe;
            let tile_types = get(off, types)?;
            let packed = get(off + types, 64 * bits / 8)?;
            for j in 0..64 {
                let byte = packed[(j >> shift) & (62 >> shift)];
                let bit = (j & (7 >> kind)) * bits;
                out[index(i, j)] = tile_types[(byte as usize >> bit) & (types - 1)];
            }
        }
    }
    Ok(out)
}

/// One half's nibbles (16384 bytes). `all15` is the plane index that means "every nibble 15"
/// (sky light only); index 128 always means all zero.
fn nibbles(data: &[u8], all15: Option<u8>) -> Vec<u8> {
    let mut out = vec![0u8; 16384];
    let (planes, stored) = data.split_at(128);
    for y in 0..128 {
        let p = planes[y];
        let shift = 4 * (y & 1);
        for xz in 0..256 {
            let value = if p == 128 {
                0
            } else if Some(p) == all15 {
                15
            } else {
                let b = stored.get(p as usize * 128 + xz / 2).copied().unwrap_or(0);
                (b >> (4 * (xz & 1))) & 15
            };
            out[(y >> 1) + xz * 64] |= value << shift;
        }
    }
    out
}

fn light(r: &mut Reader, all15: Option<u8>) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(32768);
    for _ in 0..2 {
        let planes = usize::try_from(r.int()?).map_err(|_| bad("negative plane count"))?;
        out.extend(nibbles(r.take(128 + planes * 128)?, all15));
    }
    Ok(out)
}

fn signed(v: Vec<u8>) -> Tag {
    Tag::ByteArray(v.into_iter().map(|b| b as i8).collect())
}

/// Block storage with every cell stored as 64 plain bytes (the game's uncompressed form).
fn store_blocks(half: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(&(1024i32 + 32768).to_be_bytes());
    for i in 0..512usize {
        let idx = 3 | ((i * 64) << 1);
        out.extend_from_slice(&(idx as u16).to_le_bytes());
    }
    for i in 0..512 {
        out.extend((0..64).map(|j| half[index(i, j)]));
    }
}

/// Light or data storage of one half: all-zero planes (and all-15 planes when `all15`) take no
/// space, every other plane is stored.
fn store_nibbles(half: &[u8], all15: bool, out: &mut Vec<u8>) {
    let mut planes = [0u8; 128];
    let mut stored: Vec<u8> = Vec::new();
    for y in 0..128 {
        let value = |xz: usize| (half[(y >> 1) + xz * 64] >> (4 * (y & 1))) & 15;
        if (0..256).all(|xz| value(xz) == 0) {
            planes[y] = 128;
        } else if all15 && (0..256).all(|xz| value(xz) == 15) {
            planes[y] = 129;
        } else {
            planes[y] = (stored.len() / 128) as u8;
            stored.extend((0..128).map(|k| value(2 * k) | (value(2 * k + 1) << 4)));
        }
    }
    out.extend_from_slice(&((stored.len() / 128) as i32).to_be_bytes());
    out.extend_from_slice(&planes);
    out.extend_from_slice(&stored);
}

/// The inverse of [`decode`]: a McRegion-style `Level` tree to the compressed chunk stream.
pub fn encode(tree: &NamedTag, version: i16) -> Result<Vec<u8>, Error> {
    let level = tree.tag.get("Level").ok_or_else(|| bad("chunk without Level"))?;
    let int = |k: &str| level.get(k).and_then(Tag::as_i64).ok_or_else(|| bad(format!("chunk without {k}")));
    let array = |k: &str, len: usize| -> Result<Vec<u8>, Error> {
        match level.get(k) {
            Some(Tag::ByteArray(a)) if a.len() == len => Ok(a.iter().map(|&b| b as u8).collect()),
            _ => Err(bad(format!("chunk without a {len}-byte {k}"))),
        }
    };
    let mut out = Vec::new();
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&(int("xPos")? as i32).to_be_bytes());
    out.extend_from_slice(&(int("zPos")? as i32).to_be_bytes());
    out.extend_from_slice(&int("LastUpdate").unwrap_or(0).to_be_bytes());
    let blocks = array("Blocks", 65536)?;
    for half in blocks.chunks(32768) {
        store_blocks(half, &mut out);
    }
    for (key, all15) in [("Data", false), ("SkyLight", true), ("BlockLight", true)] {
        for half in array(key, 32768)?.chunks(16384) {
            store_nibbles(half, all15, &mut out);
        }
    }
    out.extend(array("HeightMap", 256)?);
    out.extend_from_slice(&(int("TerrainPopulatedFlags").unwrap_or(0) as i16).to_be_bytes());
    out.extend(array("Biomes", 256)?);
    let mut extra = Compound::new();
    for key in ["Entities", "TileEntities", "TileTicks"] {
        if let Some(v) = level.get(key) {
            extra.insert(key.into(), v.clone());
        }
    }
    let extra = NamedTag { name: String::new(), tag: Tag::Compound(extra) };
    out.extend(mctk_nbt::to_bytes(&extra, Flavor::Java)?);
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<NamedTag, Error> {
    let mut r = Reader { bytes, at: 0 };
    let _version = r.take(2)?;
    let x = r.int()?;
    let z = r.int()?;
    let time = i64::from_be_bytes(r.take(8)?.try_into().unwrap());

    let mut ids = Vec::with_capacity(65536);
    for _ in 0..2 {
        let size = usize::try_from(r.int()?).map_err(|_| bad("negative storage size"))?;
        ids.extend(blocks(r.take(size)?)?);
    }
    let data = light(&mut r, None)?;
    let sky = light(&mut r, Some(129))?;
    let block_light = light(&mut r, Some(129))?;
    let height = r.take(256)?.to_vec();
    let populated = i16::from_be_bytes(r.take(2)?.try_into().unwrap());
    let biomes = r.take(256)?.to_vec();
    let extra = mctk_nbt::from_bytes(&bytes[r.at..], Flavor::Java)?;

    let mut level = Compound::new();
    level.insert("xPos".into(), Tag::Int(x));
    level.insert("zPos".into(), Tag::Int(z));
    level.insert("LastUpdate".into(), Tag::Long(time));
    level.insert("Blocks".into(), signed(ids));
    level.insert("Data".into(), signed(data));
    level.insert("SkyLight".into(), signed(sky));
    level.insert("BlockLight".into(), signed(block_light));
    level.insert("HeightMap".into(), signed(height));
    level.insert("TerrainPopulatedFlags".into(), Tag::Short(populated));
    level.insert("Biomes".into(), signed(biomes));
    if let Tag::Compound(c) = extra.tag {
        for (k, v) in c {
            level.insert(k, v);
        }
    }
    let mut root = Compound::new();
    root.insert("Level".into(), Tag::Compound(level));
    Ok(NamedTag { name: String::new(), tag: Tag::Compound(root) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> NamedTag {
        let bytes = |n: usize, f: &dyn Fn(usize) -> i8| Tag::ByteArray((0..n).map(f).collect());
        let mut level = Compound::new();
        level.insert("xPos".into(), Tag::Int(-15));
        level.insert("zPos".into(), Tag::Int(11));
        level.insert("LastUpdate".into(), Tag::Long(99));
        level.insert("Blocks".into(), bytes(65536, &|i| (i * 7 % 251) as i8));
        level.insert("Data".into(), bytes(32768, &|i| (i % 13) as i8));
        level.insert("SkyLight".into(), bytes(32768, &|i| if i < 8000 { -1 } else { (i % 7) as i8 }));
        level.insert("BlockLight".into(), bytes(32768, &|_| 0));
        level.insert("HeightMap".into(), bytes(256, &|i| i as i8));
        level.insert("TerrainPopulatedFlags".into(), Tag::Short(2046));
        level.insert("Biomes".into(), bytes(256, &|i| (i % 5) as i8));
        let mut root = Compound::new();
        root.insert("Level".into(), Tag::Compound(level));
        NamedTag { name: String::new(), tag: Tag::Compound(root) }
    }

    #[test]
    fn encoded_chunks_decode_to_the_same_tree() {
        let t = tree();
        let bytes = encode(&t, 8).unwrap();
        assert!(is_compressed(&bytes));
        let mut back = decode(&bytes).unwrap();
        // The encoder adds the empty entity/tile entity lists only when present.
        assert_eq!(back, t);
        back = decode(&encode(&back, 8).unwrap()).unwrap();
        assert_eq!(back, t);
    }
}
