//! Subchunk (`SubChunkPrefix`) and 3D biome (`Data3D`) records.
//!
//! Palettized subchunks (versions 1, 8 and 9) hold one or more block storages; the second
//! storage, when present, is the water layer for waterlogged blocks. Each storage is a
//! header byte (`bits_per_block << 1 | runtime_flag`), the packed indices as little-endian
//! u32 words (`32 / bits` indices per word, low bits first, in XZY order), then a palette
//! of little-endian NBT block states.

use mctk_nbt::{Flavor, NamedTag, Tag};

use crate::Error;

/// Bit widths Bedrock uses for palettized storage.
const WIDTHS: [u8; 8] = [1, 2, 3, 4, 5, 6, 8, 16];
const BLOCKS: usize = 4096;
const BIOMES: usize = 4096;

fn bad(msg: impl Into<String>) -> Error {
    Error::Malformed(msg.into())
}

/// Index of a block within a subchunk: XZY order.
pub fn block_index(x: usize, y: usize, z: usize) -> usize {
    (x * 16 + z) * 16 + y
}

/// Smallest Bedrock width that fits `len` palette entries (0 for a single entry).
fn width_for(len: usize) -> u8 {
    if len <= 1 {
        return 0;
    }
    WIDTHS
        .into_iter()
        .find(|&w| (1usize << w) >= len)
        .unwrap_or(16)
}

fn words_for(bits: u8, count: usize) -> usize {
    count.div_ceil(32 / bits as usize)
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8, Error> {
        let b = *self
            .data
            .get(self.pos)
            .ok_or_else(|| bad("truncated record"))?;
        self.pos += 1;
        Ok(b)
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let s = self
            .data
            .get(self.pos..self.pos + n)
            .ok_or_else(|| bad("truncated record"))?;
        self.pos += n;
        Ok(s)
    }

    fn i32(&mut self) -> Result<i32, Error> {
        Ok(i32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    fn rest(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }
}

fn unpack(words: &[u8], bits: u8, count: usize) -> Vec<u16> {
    let per_word = 32 / bits as usize;
    let mask = (1u32 << bits) - 1;
    (0..count)
        .map(|i| {
            let w = i / per_word;
            let word = u32::from_le_bytes(words[w * 4..w * 4 + 4].try_into().unwrap());
            ((word >> ((i % per_word) * bits as usize)) & mask) as u16
        })
        .collect()
}

fn pack(indices: &[u16], bits: u8, out: &mut Vec<u8>) {
    let per_word = 32 / bits as usize;
    for chunk in indices.chunks(per_word) {
        let mut word = 0u32;
        for (j, &v) in chunk.iter().enumerate() {
            word |= (v as u32) << (j * bits as usize);
        }
        out.extend_from_slice(&word.to_le_bytes());
    }
}

/// One palettized block layer.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockStorage {
    /// Runtime-id flag; always false on disk.
    pub runtime: bool,
    /// Block states as stored: `{name, states, version}` compounds.
    pub palette: Vec<NamedTag>,
    /// 4096 palette indices in XZY order.
    pub indices: Vec<u16>,
    /// Width read from disk, reused on write when it still fits the palette.
    bits: u8,
    /// Whether a single-entry, zero-width storage spelled out its palette size.
    zero_width_has_count: bool,
}

impl BlockStorage {
    /// A storage from a palette and 4096 XZY indices; the width is chosen on write.
    pub fn new(palette: Vec<NamedTag>, indices: Vec<u16>) -> Self {
        assert_eq!(indices.len(), BLOCKS, "a block storage holds 4096 indices");
        BlockStorage {
            runtime: false,
            palette,
            indices,
            bits: 0,
            zero_width_has_count: true,
        }
    }

    /// A storage where every block is `state`.
    pub fn filled(state: NamedTag) -> Self {
        BlockStorage {
            runtime: false,
            palette: vec![state],
            indices: vec![0; BLOCKS],
            bits: 0,
            zero_width_has_count: true,
        }
    }

    fn parse(c: &mut Cursor) -> Result<Self, Error> {
        let header = c.u8()?;
        let (bits, runtime) = (header >> 1, header & 1 == 1);
        if runtime {
            return Err(bad("runtime-id palettes are network-only"));
        }
        let mut zero_width_has_count = false;
        let (indices, palette_len) = if bits == 0 {
            // Single block type. Newer files still write a palette size of 1; detect it by
            // the NBT compound tag (0x0a) that must follow.
            let rest = c.rest();
            if rest.len() >= 5 && rest[..4] == 1i32.to_le_bytes() && rest[4] == 0x0a {
                c.bytes(4)?;
                zero_width_has_count = true;
            }
            (vec![0u16; BLOCKS], 1)
        } else {
            if !WIDTHS.contains(&bits) {
                return Err(bad(format!("unsupported block width {bits}")));
            }
            let words = c.bytes(words_for(bits, BLOCKS) * 4)?;
            let indices = unpack(words, bits, BLOCKS);
            let len = c.i32()?;
            (
                indices,
                usize::try_from(len).map_err(|_| bad("negative palette size"))?,
            )
        };
        let mut palette = Vec::with_capacity(palette_len);
        for _ in 0..palette_len {
            let mut slice = c.rest();
            let before = slice.len();
            palette.push(mctk_nbt::read_root(&mut slice, Flavor::Bedrock)?);
            c.pos += before - slice.len();
        }
        if let Some(&max) = indices.iter().max() {
            if max as usize >= palette.len() {
                return Err(bad(format!(
                    "index {max} outside palette of {}",
                    palette.len()
                )));
            }
        }
        Ok(BlockStorage {
            runtime,
            palette,
            indices,
            bits,
            zero_width_has_count,
        })
    }

    fn encode(&self, out: &mut Vec<u8>) -> Result<(), Error> {
        let needed = width_for(self.palette.len());
        let bits = if self.bits >= needed && (self.bits != 0 || self.palette.len() == 1) {
            self.bits
        } else {
            needed
        };
        out.push(bits << 1);
        if bits == 0 {
            if self.zero_width_has_count {
                out.extend_from_slice(&1i32.to_le_bytes());
            }
        } else {
            pack(&self.indices, bits, out);
            out.extend_from_slice(&(self.palette.len() as i32).to_le_bytes());
        }
        for state in &self.palette {
            out.extend_from_slice(&mctk_nbt::to_bytes(state, Flavor::Bedrock)?);
        }
        Ok(())
    }

    /// The block name at a palette slot.
    pub fn name(&self, slot: usize) -> Option<&str> {
        self.palette.get(slot)?.tag.get("name")?.as_str()
    }

    /// The block name at subchunk-local coordinates.
    pub fn block_name(&self, x: usize, y: usize, z: usize) -> Option<&str> {
        self.name(self.indices[block_index(x, y, z)] as usize)
    }
}

/// Contents of a subchunk record.
#[derive(Debug, Clone, PartialEq)]
pub enum SubChunkBody {
    /// Versions 1, 8, 9: palettized storages.
    Palettized(Vec<BlockStorage>),
    /// Pre-palette versions (0, 2–7), kept verbatim.
    Legacy(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubChunk {
    pub version: u8,
    /// Y index stored in version 9 records.
    pub y: Option<i8>,
    pub body: SubChunkBody,
}

impl SubChunk {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let mut c = Cursor { data, pos: 0 };
        let version = c.u8()?;
        let (y, body) = match version {
            1 => (
                None,
                SubChunkBody::Palettized(vec![BlockStorage::parse(&mut c)?]),
            ),
            8 | 9 => {
                let count = c.u8()?;
                let y = if version == 9 {
                    Some(c.u8()? as i8)
                } else {
                    None
                };
                let storages = (0..count)
                    .map(|_| BlockStorage::parse(&mut c))
                    .collect::<Result<_, _>>()?;
                (y, SubChunkBody::Palettized(storages))
            }
            0 | 2..=7 => (None, SubChunkBody::Legacy(data[1..].to_vec())),
            other => return Err(bad(format!("unknown subchunk version {other}"))),
        };
        if matches!(body, SubChunkBody::Palettized(_)) && c.pos != data.len() {
            return Err(bad(format!(
                "{} trailing bytes after subchunk",
                data.len() - c.pos
            )));
        }
        Ok(SubChunk { version, y, body })
    }

    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        let mut out = vec![self.version];
        match &self.body {
            SubChunkBody::Legacy(bytes) => out.extend_from_slice(bytes),
            SubChunkBody::Palettized(storages) => {
                if self.version != 1 {
                    out.push(storages.len() as u8);
                    if self.version == 9 {
                        out.push(self.y.unwrap_or(0) as u8);
                    }
                }
                for s in storages {
                    s.encode(&mut out)?;
                }
            }
        }
        Ok(out)
    }

    /// The primary (non-water) block layer.
    pub fn blocks(&self) -> Option<&BlockStorage> {
        match &self.body {
            SubChunkBody::Palettized(s) => s.first(),
            SubChunkBody::Legacy(_) => None,
        }
    }
}

/// One subchunk's biomes: 16×16×16 biome ids, or a marker meaning "same as below".
///
/// Unlike block storages, biome headers are written with the low bit set
/// (`bits << 1 | 1`); `flag` keeps that bit so records round-trip.
#[derive(Debug, Clone, PartialEq)]
pub enum BiomeStorage {
    Palettized {
        palette: Vec<i32>,
        indices: Vec<u16>,
        bits: u8,
        flag: bool,
    },
    /// Header byte 0xff: identical to the subchunk below.
    CopyPrevious,
}

/// `Data3D`: a 16×16 heightmap followed by one biome storage per subchunk.
#[derive(Debug, Clone, PartialEq)]
pub struct Data3D {
    pub heightmap: Vec<i16>,
    pub biomes: Vec<BiomeStorage>,
}

impl Data3D {
    pub fn parse(data: &[u8]) -> Result<Self, Error> {
        let mut c = Cursor { data, pos: 0 };
        let heightmap = c
            .bytes(512)?
            .chunks(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        let mut biomes = Vec::new();
        while c.pos < data.len() {
            let header = c.u8()?;
            if header == 0xff {
                biomes.push(BiomeStorage::CopyPrevious);
                continue;
            }
            let bits = header >> 1;
            let indices = if bits == 0 {
                vec![0u16; BIOMES]
            } else {
                if !WIDTHS.contains(&bits) {
                    return Err(bad(format!("unsupported biome width {bits}")));
                }
                unpack(c.bytes(words_for(bits, BIOMES) * 4)?, bits, BIOMES)
            };
            let len = if bits == 0 {
                1
            } else {
                usize::try_from(c.i32()?).map_err(|_| bad("negative biome palette size"))?
            };
            let palette = (0..len).map(|_| c.i32()).collect::<Result<Vec<_>, _>>()?;
            if indices.iter().any(|&i| i as usize >= palette.len()) {
                return Err(bad("biome index outside palette"));
            }
            biomes.push(BiomeStorage::Palettized {
                palette,
                indices,
                bits,
                flag: header & 1 == 1,
            });
        }
        Ok(Data3D { heightmap, biomes })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(512 + self.biomes.len() * 64);
        for h in &self.heightmap {
            out.extend_from_slice(&h.to_le_bytes());
        }
        for b in &self.biomes {
            match b {
                BiomeStorage::CopyPrevious => out.push(0xff),
                BiomeStorage::Palettized {
                    palette,
                    indices,
                    bits,
                    flag,
                } => {
                    let needed = width_for(palette.len());
                    let bits = if *bits >= needed { *bits } else { needed };
                    out.push(bits << 1 | *flag as u8);
                    if bits > 0 {
                        pack(indices, bits, &mut out);
                        out.extend_from_slice(&(palette.len() as i32).to_le_bytes());
                    }
                    for id in palette {
                        out.extend_from_slice(&id.to_le_bytes());
                    }
                }
            }
        }
        out
    }
}

/// Reads `name` and `states` from a Bedrock block-state compound.
pub fn block_state_name(state: &NamedTag) -> Option<&str> {
    state.tag.get("name").and_then(Tag::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mctk_nbt::Compound;

    fn state(name: &str) -> NamedTag {
        let mut c = Compound::new();
        c.insert("name".into(), Tag::String(name.into()));
        c.insert("states".into(), Tag::Compound(Compound::new()));
        c.insert("version".into(), Tag::Int(18_100_737));
        NamedTag {
            name: String::new(),
            tag: Tag::Compound(c),
        }
    }

    #[test]
    fn widths() {
        assert_eq!(width_for(1), 0);
        assert_eq!(width_for(2), 1);
        assert_eq!(width_for(5), 3);
        assert_eq!(width_for(33), 6);
        assert_eq!(width_for(65), 8);
        assert_eq!(width_for(300), 16);
        assert_eq!(words_for(3, 4096), 410);
        assert_eq!(words_for(5, 4096), 683);
    }

    #[test]
    fn subchunk_round_trip_with_water_layer() {
        let mut blocks = BlockStorage::filled(state("minecraft:stone"));
        for (i, name) in [
            "minecraft:dirt",
            "minecraft:grass_block",
            "minecraft:air",
            "minecraft:sand",
            "minecraft:gravel",
        ]
        .iter()
        .enumerate()
        {
            blocks.palette.push(state(name));
            blocks.indices[i * 100] = (i + 1) as u16;
        }
        let water = BlockStorage::filled(state("minecraft:water"));
        let sub = SubChunk {
            version: 9,
            y: Some(-3),
            body: SubChunkBody::Palettized(vec![blocks, water]),
        };
        let bytes = sub.encode().unwrap();
        let back = SubChunk::parse(&bytes).unwrap();
        assert_eq!(back.encode().unwrap(), bytes);
        let blocks = back.blocks().unwrap();
        assert_eq!(
            blocks.name(blocks.indices[200] as usize),
            Some("minecraft:air")
        );
        assert_eq!(
            blocks.name(blocks.indices[300] as usize),
            Some("minecraft:sand")
        );
        assert_eq!(blocks.block_name(0, 1, 0), Some("minecraft:stone"));
    }

    #[test]
    fn data3d_round_trip() {
        let d = Data3D {
            heightmap: (0..256).map(|i| (i % 50) as i16 + 60).collect(),
            biomes: vec![
                BiomeStorage::Palettized {
                    palette: vec![1],
                    indices: vec![0; 4096],
                    bits: 0,
                    flag: true,
                },
                BiomeStorage::Palettized {
                    palette: vec![1, 4, 7],
                    indices: (0..4096).map(|i| (i % 3) as u16).collect(),
                    bits: 2,
                    flag: true,
                },
                BiomeStorage::CopyPrevious,
            ],
        };
        assert_eq!(Data3D::parse(&d.encode()).unwrap(), d);
    }
}
