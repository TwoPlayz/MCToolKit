use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};

use crate::{BlockState, Error, PalettedContainer};

/// First data version that stores sections at the chunk root (21w43a, 1.18).
const DV_MODERN: i32 = 2844;
/// First data version whose packed indices no longer span longs (20w17a, 1.16).
const DV_NO_SPANNING: i32 = 2527;

const SECTION_BLOCKS: usize = 4096;
const SECTION_BIOMES: usize = 64;

/// Chunk storage layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkEra {
    /// Before 1.13: numeric ids under `Level`.
    Legacy,
    /// 1.13 – 1.17: palettes under `Level`; `spanning` before 1.16.
    Flattened { spanning: bool },
    /// 1.18 and later: palettes at the root.
    Modern,
}

/// How a modern palette spells block states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteStyle {
    /// `{Name, Properties}` compounds (1.13 – 1.21).
    NameProperties,
    /// `"minecraft:x"` strings or `{id, properties}` compounds (26.x).
    IdString,
}

/// Block contents of one 16×16×16 section.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockStorage {
    Palette(PalettedContainer<BlockState>),
    /// Pre-1.13 numeric ids (`Blocks` plus the `Add` nibble) and `Data` values,
    /// one entry per block in YZX order.
    Legacy {
        ids: Vec<u16>,
        data: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub y: i8,
    pub blocks: Option<BlockStorage>,
    /// 4×4×4 biome cells (1.18 and later).
    pub biomes: Option<PalettedContainer<String>>,
    /// The section compound as read; block and biome keys are re-encoded in place on write.
    original: Compound,
}

impl Section {
    /// A new section; its compound is built from scratch on write.
    pub fn new(
        y: i8,
        blocks: Option<BlockStorage>,
        biomes: Option<PalettedContainer<String>>,
    ) -> Self {
        Section {
            y,
            blocks,
            biomes,
            original: Compound::from([("Y".to_string(), Tag::Byte(y))]),
        }
    }
}

/// Index of a block within a section: YZX order.
pub fn block_index(x: usize, y: usize, z: usize) -> usize {
    (y * 16 + z) * 16 + x
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub x: i32,
    pub z: i32,
    pub era: ChunkEra,
    pub data_version: Option<i32>,
    pub palette_style: PaletteStyle,
    pub sections: Vec<Section>,
    root: NamedTag,
}

impl Chunk {
    pub fn from_nbt(root: NamedTag) -> Result<Self, Error> {
        let top = root
            .tag
            .as_compound()
            .ok_or_else(|| bad("root is not a compound"))?;
        let data_version = top
            .get("DataVersion")
            .and_then(Tag::as_i64)
            .map(|v| v as i32);
        let modern = top.contains_key("sections") || data_version.is_some_and(|v| v >= DV_MODERN);
        let body = if modern {
            top
        } else {
            top.get("Level")
                .and_then(Tag::as_compound)
                .ok_or_else(|| bad("missing Level"))?
        };
        let x = body
            .get("xPos")
            .and_then(Tag::as_i64)
            .ok_or_else(|| bad("missing xPos"))? as i32;
        let z = body
            .get("zPos")
            .and_then(Tag::as_i64)
            .ok_or_else(|| bad("missing zPos"))? as i32;
        let raw_sections = body
            .get(if modern { "sections" } else { "Sections" })
            .and_then(Tag::as_list);

        let mut era = if modern {
            ChunkEra::Modern
        } else {
            ChunkEra::Legacy
        };
        let mut palette_style = PaletteStyle::NameProperties;
        let mut sections = Vec::new();
        for s in raw_sections.into_iter().flat_map(List::iter) {
            let c = s
                .as_compound()
                .ok_or_else(|| bad("section is not a compound"))?;
            let y = c
                .get("Y")
                .and_then(Tag::as_i64)
                .ok_or_else(|| bad("section without Y"))? as i8;
            let mut section = Section {
                y,
                blocks: None,
                biomes: None,
                original: c.clone(),
            };
            if modern {
                if let Some(bs) = c.get("block_states").and_then(Tag::as_compound) {
                    let (container, style) = decode_modern_blocks(bs)?;
                    palette_style = style;
                    section.blocks = Some(BlockStorage::Palette(container));
                }
                if let Some(b) = c.get("biomes").and_then(Tag::as_compound) {
                    section.biomes = Some(decode_biomes(b)?);
                }
            } else if let Some(palette) = c.get("Palette").and_then(Tag::as_list) {
                let spanning = data_version.is_none_or(|v| v < DV_NO_SPANNING);
                era = ChunkEra::Flattened { spanning };
                let palette = palette
                    .iter()
                    .map(|t| {
                        BlockState::from_palette_entry(t).ok_or_else(|| bad("bad palette entry"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let data = long_array(c.get("BlockStates"));
                section.blocks = Some(BlockStorage::Palette(PalettedContainer::decode(
                    palette,
                    data,
                    SECTION_BLOCKS,
                    4,
                    spanning,
                )?));
            } else if let Some(Tag::ByteArray(blocks)) = c.get("Blocks") {
                section.blocks = Some(decode_legacy(blocks, c.get("Add"), c.get("Data"))?);
            }
            sections.push(section);
        }
        sections.sort_by_key(|s| s.y);
        Ok(Chunk {
            x,
            z,
            era,
            data_version,
            palette_style,
            sections,
            root,
        })
    }

    /// A new 1.18+ chunk. `fields` holds the other top-level entries (`Status`,
    /// `block_entities`, ...); position, `DataVersion` and the sections are added here and
    /// in [`Chunk::to_nbt`].
    pub fn new_modern(
        x: i32,
        z: i32,
        data_version: i32,
        palette_style: PaletteStyle,
        mut fields: Compound,
        sections: Vec<Section>,
    ) -> Self {
        fields.insert("DataVersion".into(), Tag::Int(data_version));
        fields.insert("xPos".into(), Tag::Int(x));
        fields.insert("zPos".into(), Tag::Int(z));
        fields.insert("sections".into(), Tag::List(List::new(TagType::Compound)));
        Chunk {
            x,
            z,
            era: ChunkEra::Modern,
            data_version: Some(data_version),
            palette_style,
            sections,
            root: NamedTag {
                name: String::new(),
                tag: Tag::Compound(fields),
            },
        }
    }

    pub fn section(&self, y: i8) -> Option<&Section> {
        self.sections.iter().find(|s| s.y == y)
    }

    /// Block at chunk-local x/z (0–15) and absolute y, for palette-based chunks.
    pub fn block(&self, x: usize, y: i32, z: usize) -> Option<&BlockState> {
        let section = self.section(y.div_euclid(16) as i8)?;
        match section.blocks.as_ref()? {
            BlockStorage::Palette(p) => Some(p.get(block_index(x, y.rem_euclid(16) as usize, z))),
            BlockStorage::Legacy { .. } => None,
        }
    }

    /// The complete chunk NBT, with every section's blocks and biomes re-encoded.
    pub fn to_nbt(&self) -> NamedTag {
        let mut root = self.root.clone();
        let top = root.tag.as_compound_mut().expect("validated on read");
        let (body, key) = match self.era {
            ChunkEra::Modern => (top, "sections"),
            _ => (
                top.get_mut("Level")
                    .and_then(Tag::as_compound_mut)
                    .expect("validated on read"),
                "Sections",
            ),
        };
        let encoded: Vec<Tag> = self
            .sections
            .iter()
            .map(|s| Tag::Compound(self.encode_section(s)))
            .collect();
        let list = List::from_tags(TagType::Compound, encoded).expect("all compounds");
        if let Some(slot) = body.get_mut(key) {
            *slot = Tag::List(list);
        } else if !self.sections.is_empty() {
            body.insert(key.into(), Tag::List(list));
        }
        root
    }

    fn encode_section(&self, s: &Section) -> Compound {
        let mut c = s.original.clone();
        match (&s.blocks, self.era) {
            (Some(BlockStorage::Palette(p)), ChunkEra::Modern) => {
                c.insert(
                    "block_states".into(),
                    Tag::Compound(encode_modern_blocks(p, self.palette_style)),
                );
            }
            (Some(BlockStorage::Palette(p)), ChunkEra::Flattened { spanning }) => {
                let palette = p
                    .palette
                    .iter()
                    .map(BlockState::to_named_compound)
                    .collect();
                c.insert(
                    "Palette".into(),
                    Tag::List(List::from_tags(TagType::Compound, palette).expect("compounds")),
                );
                c.insert("BlockStates".into(), Tag::LongArray(p.encode(4, spanning)));
            }
            (Some(BlockStorage::Legacy { ids, data }), _) => encode_legacy(&mut c, ids, data),
            _ => {}
        }
        if let Some(b) = &s.biomes {
            c.insert("biomes".into(), Tag::Compound(encode_biomes(b)));
        }
        c
    }
}

fn bad(msg: &str) -> Error {
    Error::BadChunk(msg.into())
}

fn long_array(tag: Option<&Tag>) -> &[i64] {
    match tag {
        Some(Tag::LongArray(v)) => v,
        _ => &[],
    }
}

fn decode_modern_blocks(
    bs: &Compound,
) -> Result<(PalettedContainer<BlockState>, PaletteStyle), Error> {
    let list = bs
        .get("palette")
        .and_then(Tag::as_list)
        .ok_or_else(|| bad("block_states without palette"))?;
    let style = match list.get(0) {
        Some(Tag::String(_)) => PaletteStyle::IdString,
        Some(Tag::Compound(c)) if c.contains_key("id") || c.contains_key("") => {
            PaletteStyle::IdString
        }
        _ => PaletteStyle::NameProperties,
    };
    let palette = list
        .iter()
        .map(|t| BlockState::from_palette_entry(t).ok_or_else(|| bad("bad palette entry")))
        .collect::<Result<Vec<_>, _>>()?;
    let container = PalettedContainer::decode(
        palette,
        long_array(bs.get("data")),
        SECTION_BLOCKS,
        4,
        false,
    )?;
    Ok((container, style))
}

fn encode_modern_blocks(p: &PalettedContainer<BlockState>, style: PaletteStyle) -> Compound {
    let palette = match style {
        PaletteStyle::NameProperties => List::from_tags(
            TagType::Compound,
            p.palette
                .iter()
                .map(BlockState::to_named_compound)
                .collect(),
        )
        .expect("compounds"),
        PaletteStyle::IdString => {
            let entries: Vec<Tag> = p.palette.iter().map(BlockState::to_id_entry).collect();
            if entries.iter().all(|t| matches!(t, Tag::String(_))) {
                List::from_tags(TagType::String, entries).expect("strings")
            } else {
                // Mixed list: wrap bare strings as {"": value}.
                let wrapped = entries
                    .into_iter()
                    .map(|t| match t {
                        Tag::String(_) => Tag::Compound(Compound::from([(String::new(), t)])),
                        other => other,
                    })
                    .collect();
                List::from_tags(TagType::Compound, wrapped).expect("compounds")
            }
        }
    };
    let mut c = Compound::new();
    let data = p.encode(4, false);
    // Mojang writes `data` before `palette`.
    if !data.is_empty() {
        c.insert("data".into(), Tag::LongArray(data));
    }
    c.insert("palette".into(), Tag::List(palette));
    c
}

fn decode_biomes(b: &Compound) -> Result<PalettedContainer<String>, Error> {
    let list = b
        .get("palette")
        .and_then(Tag::as_list)
        .ok_or_else(|| bad("biomes without palette"))?;
    let palette = list
        .iter()
        .map(|t| match t {
            Tag::String(s) => Some(s.clone()),
            Tag::Compound(c) => c.get("").and_then(Tag::as_str).map(str::to_string),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| bad("bad biome palette entry"))?;
    PalettedContainer::decode(palette, long_array(b.get("data")), SECTION_BIOMES, 1, false)
}

fn encode_biomes(b: &PalettedContainer<String>) -> Compound {
    let mut c = Compound::new();
    let data = b.encode(1, false);
    if !data.is_empty() {
        c.insert("data".into(), Tag::LongArray(data));
    }
    let palette = b.palette.iter().map(|s| Tag::String(s.clone())).collect();
    c.insert(
        "palette".into(),
        Tag::List(List::from_tags(TagType::String, palette).expect("strings")),
    );
    c
}

fn nibble(arr: &[i8], i: usize) -> u8 {
    let b = arr.get(i / 2).copied().unwrap_or(0) as u8;
    if i % 2 == 0 { b & 0x0f } else { b >> 4 }
}

fn decode_legacy(
    blocks: &[i8],
    add: Option<&Tag>,
    data: Option<&Tag>,
) -> Result<BlockStorage, Error> {
    if blocks.len() != SECTION_BLOCKS {
        return Err(bad("legacy Blocks must hold 4096 entries"));
    }
    let add = match add {
        Some(Tag::ByteArray(a)) => Some(a.as_slice()),
        _ => None,
    };
    let data = match data {
        Some(Tag::ByteArray(d)) => d.as_slice(),
        _ => &[],
    };
    let ids = (0..SECTION_BLOCKS)
        .map(|i| blocks[i] as u8 as u16 | add.map_or(0, |a| (nibble(a, i) as u16) << 8))
        .collect();
    let data = (0..SECTION_BLOCKS).map(|i| nibble(data, i)).collect();
    Ok(BlockStorage::Legacy { ids, data })
}

fn pack_nibbles(values: impl Iterator<Item = u8>) -> Vec<i8> {
    let mut out = vec![0i8; SECTION_BLOCKS / 2];
    for (i, v) in values.enumerate() {
        let shift = if i % 2 == 0 { 0 } else { 4 };
        out[i / 2] = ((out[i / 2] as u8) | ((v & 0x0f) << shift)) as i8;
    }
    out
}

fn encode_legacy(c: &mut Compound, ids: &[u16], data: &[u8]) {
    c.insert(
        "Blocks".into(),
        Tag::ByteArray(ids.iter().map(|&id| id as u8 as i8).collect()),
    );
    if ids.iter().any(|&id| id > 0xff) || c.contains_key("Add") {
        c.insert(
            "Add".into(),
            Tag::ByteArray(pack_nibbles(ids.iter().map(|&id| (id >> 8) as u8))),
        );
    }
    c.insert(
        "Data".into(),
        Tag::ByteArray(pack_nibbles(data.iter().copied())),
    );
}
