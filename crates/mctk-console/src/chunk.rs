//! Console chunk NBT: McRegion-style `Level` compound with full-height block columns.
//!
//! `Blocks` holds one numeric id per block and `Data`/`SkyLight`/`BlockLight` one nibble
//! per block. Worlds are 128 blocks tall (older saves) or 256 (newer). Indices are stored
//! in 128-tall halves, each in XZY order: `half * 32768 + (y % 128) + z * 128 + x * 2048`
//! (verified against bedrock at y = 0 and the HeightMap on Xbox 360 and PS3 saves).

use mctk_nbt::{NamedTag, Tag};

use crate::{Error, bad};

pub fn console_block_index(x: usize, y: usize, z: usize) -> usize {
    (y / 128) * 32768 + (y % 128) + z * 128 + x * 2048
}

/// A console chunk, kept as its NBT tree with typed access to blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleChunk {
    pub x: i32,
    pub z: i32,
    /// World height: 128 or 256.
    pub height: usize,
    root: NamedTag,
}

impl ConsoleChunk {
    pub fn from_nbt(root: NamedTag) -> Result<Self, Error> {
        let level = root
            .tag
            .get("Level")
            .ok_or_else(|| bad("chunk without Level"))?;
        let x = level
            .get("xPos")
            .and_then(Tag::as_i64)
            .ok_or_else(|| bad("chunk without xPos"))? as i32;
        let z = level
            .get("zPos")
            .and_then(Tag::as_i64)
            .ok_or_else(|| bad("chunk without zPos"))? as i32;
        let height = match level.get("Blocks") {
            Some(Tag::ByteArray(b)) if b.len() == 32768 || b.len() == 65536 => b.len() / 256,
            Some(Tag::ByteArray(b)) => {
                return Err(bad(format!("unexpected Blocks length {}", b.len())));
            }
            _ => return Err(bad("chunk without Blocks")),
        };
        Ok(ConsoleChunk { x, z, height, root })
    }

    pub fn to_nbt(&self) -> &NamedTag {
        &self.root
    }

    fn array(&self, key: &str) -> Option<&[i8]> {
        match self.root.tag.get("Level")?.get(key)? {
            Tag::ByteArray(a) => Some(a),
            _ => None,
        }
    }

    fn array_mut(&mut self, key: &str) -> Option<&mut Vec<i8>> {
        match self
            .root
            .tag
            .as_compound_mut()?
            .get_mut("Level")?
            .as_compound_mut()?
            .get_mut(key)?
        {
            Tag::ByteArray(a) => Some(a),
            _ => None,
        }
    }

    fn nibble(arr: &[i8], i: usize) -> u8 {
        let b = arr.get(i / 2).copied().unwrap_or(0) as u8;
        if i % 2 == 0 { b & 0x0f } else { b >> 4 }
    }

    /// Block id and data value at chunk-local x/z (0–15) and y.
    pub fn block(&self, x: usize, y: usize, z: usize) -> Option<(u8, u8)> {
        if x >= 16 || z >= 16 || y >= self.height {
            return None;
        }
        let i = console_block_index(x, y, z);
        let id = *self.array("Blocks")?.get(i)? as u8;
        let data = self.array("Data").map_or(0, |d| Self::nibble(d, i));
        Some((id, data))
    }

    pub fn set_block(
        &mut self,
        x: usize,
        y: usize,
        z: usize,
        id: u8,
        data: u8,
    ) -> Result<(), Error> {
        if x >= 16 || z >= 16 || y >= self.height {
            return Err(bad("block position outside the chunk"));
        }
        let i = console_block_index(x, y, z);
        self.array_mut("Blocks")
            .ok_or_else(|| bad("chunk without Blocks"))?[i] = id as i8;
        if let Some(d) = self.array_mut("Data") {
            let b = d[i / 2] as u8;
            d[i / 2] = if i % 2 == 0 {
                (b & 0xf0) | (data & 0x0f)
            } else {
                (b & 0x0f) | (data << 4)
            } as i8;
        }
        Ok(())
    }

    /// Highest non-air block in a column.
    pub fn top_block(&self, x: usize, z: usize) -> Option<(usize, u8)> {
        (0..self.height).rev().find_map(|y| {
            self.block(x, y, z)
                .filter(|(id, _)| *id != 0)
                .map(|(id, _)| (y, id))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mctk_nbt::Compound;

    fn chunk(height: usize) -> ConsoleChunk {
        let mut level = Compound::new();
        level.insert("xPos".into(), Tag::Int(3));
        level.insert("zPos".into(), Tag::Int(-4));
        level.insert("Blocks".into(), Tag::ByteArray(vec![0; height * 256]));
        level.insert("Data".into(), Tag::ByteArray(vec![0; height * 128]));
        let mut root = Compound::new();
        root.insert("Level".into(), Tag::Compound(level));
        ConsoleChunk::from_nbt(NamedTag {
            name: String::new(),
            tag: Tag::Compound(root),
        })
        .unwrap()
    }

    #[test]
    fn index_layout() {
        assert_eq!(console_block_index(0, 0, 0), 0);
        assert_eq!(console_block_index(0, 1, 0), 1);
        assert_eq!(console_block_index(0, 0, 1), 128);
        assert_eq!(console_block_index(1, 0, 0), 2048);
        assert_eq!(console_block_index(0, 128, 0), 32768);
        assert_eq!(console_block_index(15, 255, 15), 65535);
    }

    #[test]
    fn set_and_get_blocks_in_both_heights() {
        for height in [128, 256] {
            let mut c = chunk(height);
            c.set_block(5, height - 1, 9, 17, 2).unwrap();
            c.set_block(5, 0, 9, 7, 0).unwrap();
            assert_eq!(c.block(5, height - 1, 9), Some((17, 2)));
            assert_eq!(c.block(5, 0, 9), Some((7, 0)));
            assert_eq!(c.top_block(5, 9), Some((height - 1, 17)));
            assert!(c.block(0, height, 0).is_none());
        }
    }
}
