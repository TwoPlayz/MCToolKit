//! Java Edition ↔ Bedrock Edition terrain conversion, using the `mctk-mapping` tables
//! (Java 26.3 / Bedrock 1.26.60).
//!
//! Converted both ways: blocks (including waterlogging, which Bedrock stores as a second
//! block layer), biomes, height maps, `level.dat` and the common block entities (see
//! `block_entity`). Entities (mobs, paintings, item frames) and the local player are converted
//! both ways (see `entity`); item frames are blocks on Bedrock and entities on Java (see
//! `frames`). The report counts what was left out.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

use mctk_anvil::{ChunkCompression, ChunkPos, RawChunk, Region, RegionBuilder, RegionPos};
use mctk_bedrock::{
    BedrockWorld, BiomeStorage, BlockStorage as BedrockStorage, ChunkKey, ChunkTag, Data3D,
    Dimension as BedrockDim, Key, LevelDat, SubChunk, SubChunkBody,
};
use mctk_java::{
    BlockState, BlockStorage as JavaStorage, Chunk, ChunkEra, JavaWorld, PaletteStyle,
    PalettedContainer, Section,
};
use mctk_mapping::{BedrockBlock, JAVA_DATA_VERSION, mappings};
use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};
use mctk_registry::{Registry, Renames};
use mctk_replace::{Action, ReplacementMap};

use crate::Error;
use crate::options::{ConvertOptions, Progress, is_complete};
use crate::pipeline::{ChunkStages, Flow, run_pipeline};

/// The user's replacement choices for a Java to Bedrock conversion, with the Java-side view of
/// what the target Bedrock version can hold (see [`crate::bedrock_view`]).
#[derive(Debug, Clone)]
pub struct BedrockReplacements {
    pub map: ReplacementMap,
    pub view: Registry,
    pub renames: Renames,
}

impl BedrockReplacements {
    /// Choices for converting to the Bedrock version of `bedrock`.
    pub fn new(map: ReplacementMap, bedrock: &Registry) -> Self {
        BedrockReplacements {
            map,
            view: crate::bedrock_view(bedrock),
            renames: Renames::java_blocks(),
        }
    }

    /// The state to write for `s`, and what it became when it was replaced.
    fn apply(&self, s: &BlockState) -> (BlockState, Option<String>) {
        match self.map.resolve(s, &self.view, &self.renames) {
            Some((Action::Replace(r), _)) => {
                let text = r.to_string();
                (r, Some(text))
            }
            Some((Action::Air, _)) => (BlockState::air(), Some("air".into())),
            // Already supported, kept on purpose, or (never from `resolve`) a bare block choice.
            _ => (s.clone(), None),
        }
    }
}

/// The registry of the Java version Bedrock worlds are converted to.
fn java_registry() -> Option<&'static Registry> {
    static R: std::sync::OnceLock<Option<Registry>> = std::sync::OnceLock::new();
    R.get_or_init(|| mctk_registry::builtin::java_versions().first().and_then(|v| mctk_registry::builtin::java(v)))
        .as_ref()
}

/// The user's replacement choices for a Bedrock to Java conversion, with the Bedrock-side view of
/// what the target Java version can hold (see [`crate::java_view`]).
#[derive(Debug, Clone)]
pub struct JavaReplacements {
    pub map: ReplacementMap,
    pub view: Registry,
    pub renames: Renames,
    /// The Bedrock registry, to complete a replacement block's states.
    bedrock: Registry,
}

impl JavaReplacements {
    /// Choices for converting to the Java version of `java`.
    pub fn new(map: ReplacementMap, java: &Registry, bedrock: &Registry) -> Self {
        JavaReplacements {
            map,
            view: crate::java_view(java, bedrock),
            renames: Renames::default(),
            bedrock: bedrock.clone(),
        }
    }

    fn state_of(b: &BedrockBlock) -> BlockState {
        let mut s = BlockState::new(b.name.clone());
        for (k, v) in &b.states {
            let text = match v {
                Tag::Byte(n) => format!("b:{n}"),
                Tag::Int(n) => format!("i:{n}"),
                Tag::String(t) => format!("s:{t}"),
                other => format!("?:{other}"),
            };
            s.properties.insert(k.clone(), text);
        }
        s
    }

    /// The Bedrock block for a replacement state, with every state spelled out.
    fn block_from(&self, r: &BlockState) -> Option<BedrockBlock> {
        let full = self.bedrock.complete_state(&r.name, r.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())))?;
        let states: Compound = full
            .into_iter()
            .filter_map(|(k, v)| {
                let (kind, text) = v.split_once(':')?;
                let tag = match kind {
                    "b" => Tag::Byte(text.parse().ok()?),
                    "i" => Tag::Int(text.parse().ok()?),
                    _ => Tag::String(text.to_string()),
                };
                Some((k, tag))
            })
            .collect();
        Some(BedrockBlock { name: r.name.clone(), states })
    }

    fn apply(&self, b: &BedrockBlock) -> (BedrockBlock, Option<String>) {
        match self.map.resolve(&Self::state_of(b), &self.view, &self.renames) {
            Some((Action::Replace(r), _)) => match self.block_from(&r) {
                Some(nb) => (nb, Some(r.to_string())),
                None => (b.clone(), None),
            },
            Some((Action::Air, _)) => (
                BedrockBlock { name: "minecraft:air".into(), states: Compound::new() },
                Some("air".into()),
            ),
            _ => (b.clone(), None),
        }
    }

    /// The column with every block the target cannot hold replaced, counted in `report`.
    fn column(&self, col: &Column, report: &mut EditionReport) -> Column {
        let mut out = col.clone();
        for bytes in out.subchunks.values_mut() {
            let Ok(mut sub) = SubChunk::parse(bytes) else { continue };
            let SubChunkBody::Palettized(storages) = &mut sub.body else { continue };
            let Some(first) = storages.first_mut() else { continue };
            let mut uses = vec![0usize; first.palette.len()];
            for &i in &first.indices {
                uses[i as usize] += 1;
            }
            let mut changed = false;
            for (slot, entry) in first.palette.iter_mut().enumerate() {
                let Some(b) = BedrockBlock::from_nbt(entry) else { continue };
                if let (nb, Some(to)) = self.apply(&b) {
                    *report.replaced.entry(format!("{} -> {to}", Self::state_of(&b))).or_insert(0) += uses[slot];
                    *entry = nb.to_nbt();
                    changed = true;
                }
            }
            if changed {
                if let Ok(enc) = sub.encode() {
                    *bytes = enc;
                }
            }
        }
        out
    }
}

/// What a Java ↔ Bedrock conversion did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EditionReport {
    /// Converted chunk columns per dimension.
    pub chunks: BTreeMap<String, usize>,
    /// Block states without an exact mapping, converted to the closest state of the same
    /// block, with the number of section palettes they appeared in.
    pub approximated: BTreeMap<String, usize>,
    /// Block states with no mapping at all, written as air.
    pub unmapped: BTreeMap<String, usize>,
    /// Data left out, by kind.
    pub skipped: BTreeMap<String, usize>,
    /// Block entities converted, by (source) id.
    pub converted_block_entities: BTreeMap<String, usize>,
    /// Entities converted (Bedrock → Java only), by Java id.
    pub converted_entities: BTreeMap<String, usize>,
    /// Blocks swapped for another because the target cannot hold them (`from -> to`), with the
    /// number of blocks. Only Java → Bedrock with replacement choices fills this.
    pub replaced: BTreeMap<String, usize>,
    pub notes: Vec<String>,
}

impl EditionReport {
    /// Adds another report's counts (and notes) to this one.
    fn merge(&mut self, other: EditionReport) {
        for (dst, src) in [
            (&mut self.chunks, other.chunks),
            (&mut self.approximated, other.approximated),
            (&mut self.unmapped, other.unmapped),
            (&mut self.skipped, other.skipped),
            (
                &mut self.converted_block_entities,
                other.converted_block_entities,
            ),
            (&mut self.converted_entities, other.converted_entities),
            (&mut self.replaced, other.replaced),
        ] {
            for (k, v) in src {
                *dst.entry(k).or_insert(0) += v;
            }
        }
        self.notes.extend(other.notes);
    }

    pub(crate) fn skip(&mut self, what: &str, n: usize) {
        if n > 0 {
            *self.skipped.entry(what.to_string()).or_insert(0) += n;
        }
    }
}

/// Bedrock blocks that sit in water without a `waterlogged` property in Java.
const ALWAYS_IN_WATER: [&str; 5] = [
    "minecraft:kelp",
    "minecraft:kelp_plant",
    "minecraft:seagrass",
    "minecraft:tall_seagrass",
    "minecraft:bubble_column",
];

/// Section ranges per dimension.
struct DimInfo {
    java_id: &'static str,
    /// 0 Overworld, -1 Nether, 1 End.
    number: i32,
    /// Folder of the dimension in a 26.x Java world.
    java_folder: &'static str,
    bedrock: BedrockDim,
    java_sections: std::ops::Range<i8>,
    bedrock_sections: std::ops::Range<i8>,
    default_biome: &'static str,
}

const DIMS: [DimInfo; 3] = [
    DimInfo {
        java_id: "minecraft:overworld",
        number: 0,
        java_folder: "dimensions/minecraft/overworld",
        bedrock: BedrockDim::Overworld,
        java_sections: -4..20,
        bedrock_sections: -4..20,
        default_biome: "minecraft:plains",
    },
    DimInfo {
        java_id: "minecraft:the_nether",
        number: -1,
        java_folder: "dimensions/minecraft/the_nether",
        bedrock: BedrockDim::Nether,
        java_sections: 0..16,
        bedrock_sections: 0..8,
        default_biome: "minecraft:nether_wastes",
    },
    DimInfo {
        java_id: "minecraft:the_end",
        number: 1,
        java_folder: "dimensions/minecraft/the_end",
        bedrock: BedrockDim::End,
        java_sections: 0..16,
        bedrock_sections: 0..16,
        default_biome: "minecraft:the_end",
    },
];

fn fresh_dir(out: &Path) -> Result<(), Error> {
    if out.exists() && std::fs::read_dir(out)?.next().is_some() {
        return Err(Error::Invalid(format!("{} is not empty", out.display())));
    }
    std::fs::create_dir_all(out)?;
    Ok(())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn air() -> BedrockBlock {
    BedrockBlock {
        name: "minecraft:air".into(),
        states: Compound::new(),
    }
}

/// Index of a 4×4×4 biome cell within a Java section.
fn java_biome_cell(x: usize, y: usize, z: usize) -> usize {
    ((y / 4) * 4 + z / 4) * 4 + x / 4
}

/// Index of a column in Bedrock's `Data3D` height map.
fn heightmap_index(x: usize, z: usize) -> usize {
    z * 16 + x
}

// ------------------------------------------------------------------ Java → Bedrock

/// One Java section as Bedrock storages: the block layer and, when any block is
/// waterlogged, the water layer. `None` for an all-air section.
fn section_to_bedrock(
    p: &PalettedContainer<BlockState>,
    frames: &[((usize, usize, usize), BedrockBlock)],
    report: &mut EditionReport,
) -> Option<Vec<BedrockStorage>> {
    let m = mappings();
    if p.palette.iter().all(BlockState::is_air) && frames.is_empty() {
        return None;
    }
    let mut palette: Vec<BedrockBlock> = Vec::new();
    let mut slot_of: HashMap<String, u16> = HashMap::new();
    let mut intern = |b: BedrockBlock| -> u16 {
        let key = format!("{}{:?}", b.name, b.states);
        *slot_of.entry(key).or_insert_with(|| {
            palette.push(b);
            (palette.len() - 1) as u16
        })
    };
    let mapped: Vec<(u16, bool)> = p
        .palette
        .iter()
        .map(|s| {
            let full = m.complete_java(s);
            let water = full
                .properties
                .get("waterlogged")
                .is_some_and(|v| v == "true")
                || ALWAYS_IN_WATER.contains(&full.name.as_str());
            let b = match m.java_to_bedrock(&full) {
                // A block caught mid-move has no entity to go with it, so it becomes air.
                _ if full.name == "minecraft:moving_piston" => air(),
                Some(b) => {
                    if !m.is_known_java(&full) {
                        *report.approximated.entry(full.to_string()).or_insert(0) += 1;
                    }
                    b.clone()
                }
                None => {
                    *report.unmapped.entry(full.to_string()).or_insert(0) += 1;
                    air()
                }
            };
            (intern(b), water)
        })
        .collect();
    let mut indices = vec![0u16; 4096];
    let mut water = vec![0u16; 4096];
    let mut any_water = false;
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                let (slot, wet) = mapped[p.indices[mctk_java::block_index(x, y, z)] as usize];
                let b = mctk_bedrock::block_index(x, y, z);
                indices[b] = slot;
                if wet {
                    water[b] = 1;
                    any_water = true;
                }
            }
        }
    }
    // Item frames are blocks on Bedrock. They take the place of what Java had there (air, or
    // water the frame is under, which stays in the water layer).
    for ((x, y, z), block) in frames {
        let at = mctk_java::block_index(*x, *y, *z);
        let wet = mapped[p.indices[at] as usize].1 || p.get(at).name == "minecraft:water";
        let b = mctk_bedrock::block_index(*x, *y, *z);
        indices[b] = intern(block.clone());
        if wet {
            water[b] = 1;
            any_water = true;
        }
    }
    let mut storages = vec![BedrockStorage::new(
        palette.iter().map(BedrockBlock::to_nbt).collect(),
        indices,
    )];
    if any_water {
        let w = m
            .java_to_bedrock(&BlockState::new("minecraft:water").with("level", "0"))
            .expect("water is mapped");
        storages.push(BedrockStorage::new(vec![air().to_nbt(), w.to_nbt()], water));
    }
    Some(storages)
}

fn biomes_to_bedrock(b: &PalettedContainer<String>, report: &mut EditionReport) -> BiomeStorage {
    let m = mappings();
    let mut palette: Vec<i32> = Vec::new();
    let map: Vec<u16> = b
        .palette
        .iter()
        .map(|name| {
            let id = m.java_biome_to_bedrock(name).unwrap_or_else(|| {
                *report.unmapped.entry(format!("biome {name}")).or_insert(0) += 1;
                1
            });
            match palette.iter().position(|&p| p == id) {
                Some(i) => i as u16,
                None => {
                    palette.push(id);
                    (palette.len() - 1) as u16
                }
            }
        })
        .collect();
    let mut indices = vec![0u16; 4096];
    for x in 0..16 {
        for z in 0..16 {
            for y in 0..16 {
                indices[mctk_bedrock::block_index(x, y, z)] =
                    map[b.indices[java_biome_cell(x, y, z)] as usize];
            }
        }
    }
    BiomeStorage::Palettized {
        palette,
        indices,
        bits: 0,
        flag: true,
    }
}

/// Converts one Java chunk into Bedrock database records.
fn chunk_to_bedrock(
    chunk: &Chunk,
    dim: &DimInfo,
    offset: (i32, i32),
    block_entities: &[Compound],
    frames: &[crate::frames::Frame],
    report: &mut EditionReport,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    chunk_to_bedrock_in(chunk, dim, offset, block_entities, frames, report, crate::bedrock_layout::get())
}

/// [`chunk_to_bedrock`] for a given version's chunk layout.
fn chunk_to_bedrock_in(
    chunk: &Chunk,
    dim: &DimInfo,
    offset: (i32, i32),
    block_entities: &[Compound],
    frames: &[crate::frames::Frame],
    report: &mut EditionReport,
    layout: crate::bedrock_layout::Layout,
) -> Vec<(Vec<u8>, Vec<u8>)> {
    let key = |tag: ChunkTag, subchunk: Option<i8>| {
        ChunkKey {
            x: chunk.x + offset.0,
            z: chunk.z + offset.1,
            dimension: dim.bedrock,
            tag,
            subchunk,
        }
        .encode()
    };
    let m = mappings();
    // Versions before 1.18 stop the overworld at y 0 and 255.
    let sections = if layout.legacy_height && dim.number == 0 { 0..16 } else { dim.bedrock_sections.clone() };
    let min_y = sections.start as i32 * 16;
    let mut records = Vec::new();
    let mut heightmap = vec![0i16; 256];
    let mut biomes = Vec::new();
    let mut dropped = 0;
    // Entities Bedrock needs for blocks that have none on Java (bed colour, cauldron,
    // flower pot), found by scanning the palettes.
    let mut generated: Vec<Compound> = Vec::new();
    let existing: std::collections::HashSet<(i64, i64, i64)> = block_entities
        .iter()
        .filter_map(|be| {
            let at = |k: &str| be.get(k).and_then(Tag::as_i64);
            Some((at("x")?, at("y")?, at("z")?))
        })
        .collect();
    for sy in sections.clone() {
        let section = chunk.section(sy);
        match section.and_then(|s| s.biomes.as_ref()) {
            Some(b) => biomes.push(biomes_to_bedrock(b, report)),
            None if biomes.is_empty() => {
                let id = m.java_biome_to_bedrock(dim.default_biome).unwrap_or(1);
                biomes.push(BiomeStorage::Palettized {
                    palette: vec![id],
                    indices: vec![0; 4096],
                    bits: 0,
                    flag: true,
                });
            }
            // A section without biomes repeats the one below. Written out in full: the Bedrock
            // server crashes on a "same as previous" marker in the lowest storages (found by
            // loading converted console worlds, which have no sections below y 0).
            None => {
                let last = biomes.last().cloned().unwrap_or(BiomeStorage::CopyPrevious);
                biomes.push(last);
            }
        }
        let in_section: Vec<((usize, usize, usize), BedrockBlock)> = frames
            .iter()
            .filter(|f| {
                f.tile.1.div_euclid(16) == sy as i32
                    && f.tile.0.div_euclid(16) == chunk.x
                    && f.tile.2.div_euclid(16) == chunk.z
            })
            .map(|f| {
                let (name, states) = f.bedrock_block();
                let at = (
                    f.tile.0.rem_euclid(16) as usize,
                    f.tile.1.rem_euclid(16) as usize,
                    f.tile.2.rem_euclid(16) as usize,
                );
                (
                    at,
                    BedrockBlock {
                        name: name.to_string(),
                        states,
                    },
                )
            })
            .collect();
        let empty;
        let p = match section.and_then(|s| s.blocks.as_ref()) {
            Some(JavaStorage::Palette(p)) => p,
            // A frame can hang in a section that has no blocks of its own.
            _ if !in_section.is_empty() => {
                empty = PalettedContainer::filled(BlockState::air(), 4096);
                &empty
            }
            // Bedrock always writes the lowest eight subchunks, y -64 to 63, even when they are
            // air; a chunk that lacks them (a world that began at y 0, such as a console world)
            // is written with air there.
            _ if sy < 4 && !layout.legacy_height => {
                empty = PalettedContainer::filled(BlockState::air(), 4096);
                &empty
            }
            _ => continue,
        };
        let Some(storages) = section_to_bedrock(p, &in_section, report) else {
            continue;
        };
        if p.palette
            .iter()
            .any(|s| crate::block_entities_more::generates_entity(&s.name))
        {
            for y in 0..16 {
                for z in 0..16 {
                    for x in 0..16 {
                        let state = p.get(mctk_java::block_index(x, y, z));
                        if !crate::block_entities_more::generates_entity(&state.name) {
                            continue;
                        }
                        let (wx, wy, wz) = (
                            chunk.x * 16 + x as i32,
                            sy as i32 * 16 + y as i32,
                            chunk.z * 16 + z as i32,
                        );
                        // Only where Java has no entity of its own.
                        if existing.contains(&(wx as i64, wy as i64, wz as i64)) {
                            continue;
                        }
                        generated.extend(crate::block_entity::generated_to_bedrock(state, (wx, wy, wz), offset, report));
                    }
                }
            }
        }
        for z in 0..16 {
            for x in 0..16 {
                for y in (0..16).rev() {
                    if !p.get(mctk_java::block_index(x, y, z)).is_air() {
                        let h = (sy as i32 * 16 + y as i32 + 1 - min_y) as i16;
                        let slot = &mut heightmap[heightmap_index(x, z)];
                        *slot = (*slot).max(h);
                        break;
                    }
                }
            }
        }
        let sub = SubChunk {
            version: if layout.legacy_height { 8 } else { 9 },
            y: if layout.legacy_height { None } else { Some(sy) },
            body: SubChunkBody::Palettized(storages),
        };
        records.push((
            key(ChunkTag::SubChunkPrefix, Some(sy)),
            sub.encode().expect("encodable"),
        ));
    }
    for f in frames {
        let section = f.tile.1.div_euclid(16);
        if f.tile.0.div_euclid(16) != chunk.x
            || f.tile.2.div_euclid(16) != chunk.z
            || !sections.contains(&(section as i8))
        {
            report.skip("item frames outside their chunk or the Bedrock height range", 1);
            continue;
        }
        let mut notes = crate::item_codec::Notes::default();
        let mut be = f.block_entity(&mut notes);
        for (key, by) in [("x", offset.0), ("z", offset.1)] {
            if let Some(v) = be.get(key).and_then(Tag::as_i64) {
                be.insert(key.into(), Tag::Int(v as i32 + by * 16));
            }
        }
        generated.push(be);
        crate::block_entity::report_item_notes(notes, report);
        let id = if f.glow {
            crate::frames::JAVA_GLOW_ID
        } else {
            crate::frames::JAVA_ID
        };
        *report.converted_entities.entry(id.to_string()).or_insert(0) += 1;
    }
    for s in &chunk.sections {
        let has_blocks = matches!(&s.blocks, Some(JavaStorage::Palette(p)) if !p.palette.iter().all(BlockState::is_air));
        if !sections.contains(&s.y) && dim.java_sections.contains(&s.y) && has_blocks {
            dropped += 1;
        }
    }
    report.skip("sections outside the Bedrock height range", dropped);
    // Block entities are stored as consecutive little-endian NBT compounds.
    let mut be_bytes = Vec::new();
    let mut converted: Vec<Compound> = block_entities
        .iter()
        .filter_map(|be| {
            // The block the entity sits in decides how double chests are paired.
            let at = |k: &str| be.get(k).and_then(Tag::as_i64);
            let block = match (at("x"), at("y"), at("z")) {
                (Some(x), Some(y), Some(z)) => chunk
                    .block(x.rem_euclid(16) as usize, y as i32, z.rem_euclid(16) as usize)
                    .map(|b| mappings().complete_java(b)),
                _ => None,
            };
            crate::block_entity::java_to_bedrock(be, offset, block.as_ref(), report)
        })
        .collect();
    converted.extend(generated);
    for converted in converted {
        {
            let named = NamedTag {
                name: String::new(),
                tag: Tag::Compound(converted),
            };
            if let Ok(b) = mctk_nbt::to_bytes(&named, Flavor::Bedrock) {
                be_bytes.extend(b);
            }
        }
    }
    if !be_bytes.is_empty() {
        records.push((key(ChunkTag::BlockEntity, None), be_bytes));
    }
    records.push((key(ChunkTag::Version, None), vec![layout.chunk_version]));
    if layout.legacy_height {
        // One biome per column, taken at sea level.
        let at_sea = biomes.get(4).or(biomes.last());
        let column_biomes: Vec<u8> = (0..256)
            .map(|i| {
                let (z, x) = (i / 16, i % 16);
                match at_sea {
                    Some(BiomeStorage::Palettized { palette, indices, .. }) => palette
                        .get(indices[mctk_bedrock::block_index(x, 0, z)] as usize)
                        .map_or(1, |&id| crate::bedrock_layout::legacy_biome(id)),
                    _ => 1,
                }
            })
            .collect();
        records.push((key(ChunkTag::Data2D, None), crate::bedrock_layout::data2d(&heightmap, &column_biomes)));
    } else {
        records.push((key(ChunkTag::Data3D, None), Data3D { heightmap, biomes }.encode()));
    }
    records.push((
        key(ChunkTag::FinalizedState, None),
        2i32.to_le_bytes().to_vec(),
    ));
    if layout.blending {
        records.push((key(ChunkTag::BlendingData, None), vec![0x00, 0x0b]));
    }
    records
}

fn java_seed(world: &JavaWorld) -> Option<i64> {
    let data = world.level.root.tag.get("Data");
    let from_level = data
        .and_then(|d| d.get("WorldGenSettings"))
        .and_then(|w| w.get("seed"))
        .or_else(|| data.and_then(|d| d.get("RandomSeed")))
        .and_then(Tag::as_i64);
    from_level.or_else(|| {
        let path = world.path.join("data/minecraft/world_gen_settings.dat");
        let (_, root) = mctk_nbt::read_file(path, Flavor::Java).ok()?;
        root.tag.get("data")?.get("seed")?.as_i64()
    })
}

fn java_difficulty(data: Option<&Tag>) -> i32 {
    let named = data
        .and_then(|d| d.get("difficulty_settings"))
        .and_then(|d| d.get("difficulty"))
        .and_then(Tag::as_str);
    match named {
        Some("peaceful") => 0,
        Some("easy") => 1,
        Some("normal") => 2,
        Some("hard") => 3,
        _ => data
            .and_then(|d| d.get("Difficulty"))
            .and_then(Tag::as_i64)
            .unwrap_or(2) as i32,
    }
}

fn int_list(values: &[i32]) -> Tag {
    Tag::List(
        List::from_tags(TagType::Int, values.iter().map(|&v| Tag::Int(v)).collect()).expect("ints"),
    )
}

/// The Bedrock version being written, when it is not the newest: the game version, network and
/// storage versions of its `level.dat`. Set by [`set_bedrock_target`].
static TARGET: std::sync::Mutex<Option<([i32; 5], i32, u32, String)>> = std::sync::Mutex::new(None);

/// Chooses the Bedrock version the next conversion writes for, from the registry of that version
/// (see `mctk-gen bedrock-finish` for the facts it carries). A registry without them (the newest
/// version) leaves the defaults.
pub fn set_bedrock_target(registry: &mctk_registry::Registry) {
    let meta = &registry.meta;
    mctk_mapping::set_downgrades(&registry.downgrades);
    crate::bedrock_layout::set(crate::bedrock_layout::Layout::of(registry));
    if let Some(v) = meta.get("block_version").and_then(|v| v.parse().ok()) {
        mctk_mapping::set_block_version(v);
    } else {
        mctk_mapping::set_block_version(mctk_mapping::BEDROCK_BLOCK_VERSION);
    }
    let client: Option<Vec<i32>> = meta.get("client_version").map(|v| v.split(',').filter_map(|n| n.parse().ok()).collect());
    *TARGET.lock().expect("target") = match (client, meta.get("network_version"), meta.get("storage_version")) {
        (Some(c), Some(n), Some(s)) if c.len() == 5 => Some((
            [c[0], c[1], c[2], c[3], c[4]],
            n.parse().unwrap_or(2225),
            s.parse().unwrap_or(10),
            registry.version.clone(),
        )),
        _ => None,
    };
}

fn bedrock_level_dat(world: &JavaWorld, options: &ConvertOptions) -> LevelDat {
    let data = world.level.root.tag.get("Data");
    let flag = |key: &str| {
        data.and_then(|d| d.get(key))
            .and_then(Tag::as_i64)
            .unwrap_or(0) as i8
    };
    let spawn = world.level.spawn.clone();
    let game_type = options
        .game_mode
        .unwrap_or(world.level.game_type.unwrap_or(0));
    let (dx, dz) = (options.offset.0 * 16, options.offset.1 * 16);
    let target = TARGET.lock().expect("target").clone();
    let version = target.as_ref().map_or([1, 26, 60, 0, 0], |t| t.0);
    let (network, storage) = target.as_ref().map_or((2225, 10), |t| (t.1, t.2));
    let inventory = target.as_ref().map_or("1.26.60".to_string(), |t| t.3.clone());
    let mut c = Compound::new();
    let mut put = |k: &str, v: Tag| {
        c.insert(k.to_string(), v);
    };
    put("LevelName", Tag::String(world.level.name.clone()));
    put("StorageVersion", Tag::Int(storage as i32));
    put("NetworkVersion", Tag::Int(network));
    put("Generator", Tag::Int(1));
    put("WorldVersion", Tag::Int(1));
    put("InventoryVersion", Tag::String(inventory));
    put("lastOpenedWithVersion", int_list(&version));
    put("MinimumCompatibleClientVersion", int_list(&version));
    put("RandomSeed", Tag::Long(java_seed(world).unwrap_or(0)));
    put("GameType", Tag::Int(game_type));
    put(
        "Difficulty",
        Tag::Int(options.difficulty.unwrap_or_else(|| java_difficulty(data))),
    );
    put("SpawnX", Tag::Int(spawn.as_ref().map_or(0, |s| s.x) + dx));
    put("SpawnY", Tag::Int(spawn.as_ref().map_or(32767, |s| s.y)));
    put("SpawnZ", Tag::Int(spawn.as_ref().map_or(0, |s| s.z) + dz));
    let time = data
        .and_then(|d| d.get("Time"))
        .and_then(Tag::as_i64)
        .unwrap_or(0);
    put("Time", Tag::Long(time));
    put("currentTick", Tag::Long(time));
    put(
        "LastPlayed",
        Tag::Long(world.level.last_played.map_or(now_secs(), |ms| ms / 1000)),
    );
    put("commandsEnabled", Tag::Byte(flag("allowCommands")));
    put("cheatsEnabled", Tag::Byte(flag("allowCommands")));
    put("hasBeenLoadedInCreative", Tag::Byte((game_type == 1) as i8));
    put("IsHardcore", Tag::Byte(0));
    put("spawnMobs", Tag::Byte(1));
    put("Platform", Tag::Int(2));
    LevelDat {
        storage_version: storage,
        root: NamedTag {
            name: String::new(),
            tag: Tag::Compound(c),
        },
    }
}

/// Converts the Java world `src` into a new Bedrock world folder at `out`.
pub fn java_to_bedrock(src: &Path, out: &Path) -> Result<EditionReport, Error> {
    java_to_bedrock_with(src, out, &ConvertOptions::default(), &Progress::default())
}

/// Chunk status of a Java chunk root (`Status` at the root since 1.18, under `Level` before).
fn java_status(root: &NamedTag) -> Option<String> {
    root.tag
        .get("Status")
        .or_else(|| root.tag.get("Level")?.get("Status"))
        .and_then(Tag::as_str)
        .map(str::to_string)
}

/// The compounds of an NBT list (anything else gives none).
fn compounds(list: Option<&Tag>) -> Vec<Compound> {
    list.and_then(Tag::as_list)
        .map(List::iter)
        .into_iter()
        .flatten()
        .filter_map(|t| t.as_compound().cloned())
        .collect()
}

/// What one Java chunk contributes to a Java → Bedrock conversion.
#[derive(Default)]
struct BedrockChunkOut {
    /// Database records; empty when the chunk was left out.
    records: Vec<(Vec<u8>, Vec<u8>)>,
    report: EditionReport,
    /// Java entities of the chunk; Bedrock stores them apart from the chunk, under ids
    /// handed out in chunk order, so they are converted when the chunks are merged.
    entities: Vec<Compound>,
}

#[derive(Default)]
struct JavaSlot {
    root: Option<NamedTag>,
    chunk: Option<Chunk>,
    block_entities: Vec<Compound>,
    entities: Vec<Compound>,
    out: BedrockChunkOut,
}

/// Per-chunk work of Java → Bedrock, for [`run_pipeline`].
struct JavaToBedrockStages<'a> {
    work: &'a [(&'a mctk_java::Dimension, &'static DimInfo, Region, Option<Region>)],
    /// (index into `work`, local x, local z) of every chunk.
    jobs: &'a [(usize, usize, usize)],
    options: &'a ConvertOptions,
    progress: &'a Progress,
    /// Blocks the target cannot hold are replaced as the user chose, before they are mapped.
    replace: Option<&'a BedrockReplacements>,
}

impl ChunkStages for JavaToBedrockStages<'_> {
    type Slot = JavaSlot;
    type Output = BedrockChunkOut;

    fn load(&self, index: usize, slot: &mut JavaSlot) -> Result<Flow, Error> {
        let (r, lx, lz) = self.jobs[index];
        let (jdim, _, region, entity_region) = &self.work[r];
        self.progress.set_dimension(&jdim.id);
        slot.root = region.read_chunk(lx, lz)?;
        // Since 1.17 entities live in their own region files.
        if let (Some(_), Some(er)) = (&slot.root, entity_region) {
            if let Some(root) = er.read_chunk(lx, lz)? {
                slot.entities = compounds(root.tag.get("Entities"));
            }
        }
        Ok(if slot.root.is_some() {
            Flow::Continue
        } else {
            Flow::Skip
        })
    }

    fn parse(&self, _: usize, slot: &mut JavaSlot) -> Result<Flow, Error> {
        let root = slot.root.take().expect("loaded");
        if self.options.skip_partial_chunks && !is_complete(java_status(&root).as_deref()) {
            slot.out.report.skip("partially generated chunks", 1);
            return Ok(Flow::Skip);
        }
        slot.block_entities = root
            .tag
            .get("block_entities")
            .or_else(|| root.tag.get("Level").and_then(|l| l.get("TileEntities")))
            .and_then(Tag::as_list)
            .map(List::iter)
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_compound().cloned())
            .collect();
        if !self.options.tile_entities {
            slot.block_entities.clear();
        }
        let biomes = crate::legacy::read_biomes(&root);
        // Older worlds keep entities inside the chunk.
        let inline = compounds(
            root.tag
                .get("Entities")
                .or_else(|| root.tag.get("Level").and_then(|l| l.get("Entities"))),
        );
        slot.entities.extend(inline);
        let Ok(chunk) = Chunk::from_nbt(root) else {
            slot.out.report.skip("unreadable chunks", 1);
            return Ok(Flow::Invalid);
        };
        let mut chunk = chunk;
        // Worlds from before 26.x: numeric blocks, old names, old item and entity formats.
        let old = chunk.era == ChunkEra::Legacy;
        crate::legacy::modernize_chunk(&mut chunk, biomes.as_ref(), &mut slot.out.report);
        if old || chunk.data_version.is_none_or(|v| v < crate::legacy::DV_COMPONENTS) {
            let report = &mut slot.out.report;
            slot.block_entities = slot
                .block_entities
                .iter()
                .map(|be| crate::legacy::upgrade_block_entity(be, old, report))
                .collect();
            slot.entities = slot
                .entities
                .iter()
                .map(|e| crate::legacy::upgrade_entity(e, old, report))
                .collect();
        }
        if old {
            crate::legacy::colour_beds(&mut chunk, &slot.block_entities);
        }
        if let Some(r) = self.replace {
            for section in &mut chunk.sections {
                if let Some(JavaStorage::Palette(c)) = &mut section.blocks {
                    let mut replaced = indexmap::IndexMap::new();
                    crate::java_java::remap_blocks(c, |s| r.apply(s), &mut replaced);
                    for (from, rb) in replaced {
                        *slot.out.report.replaced.entry(format!("{from} -> {}", rb.to)).or_insert(0) +=
                            rb.blocks as usize;
                    }
                }
            }
        }
        slot.chunk = Some(chunk);
        Ok(Flow::Continue)
    }

    fn convert(&self, index: usize, slot: &mut JavaSlot) -> Result<Flow, Error> {
        let mut chunk = slot.chunk.take().expect("parsed");
        let dim = self.work[self.jobs[index].0].1;
        // A target whose overworld starts at y 0: deal with the part below it.
        if crate::bedrock_layout::get().legacy_height && dim.number == 0 && crate::depth::applies(&chunk) {
            let by = crate::depth::fix(&mut chunk, self.options.depth);
            if by != 0 {
                slot.block_entities.iter_mut().chain(slot.entities.iter_mut()).for_each(|c| crate::depth::raise(c, by));
            }
        }
        // Item frames become blocks, so they are converted with the chunk; the rest of the
        // entities are converted when the chunks are merged.
        let mut frames = Vec::new();
        let mut rest = Vec::new();
        for e in std::mem::take(&mut slot.entities) {
            match crate::frames::Frame::from_java(&e) {
                Some(f) => frames.push(f),
                None => rest.push(e),
            }
        }
        slot.out.records = chunk_to_bedrock(
            &chunk,
            dim,
            self.options.offset,
            &slot.block_entities,
            &frames,
            &mut slot.out.report,
        );
        slot.out.entities = rest;
        self.progress.chunk(
            chunk.x + self.options.offset.0,
            chunk.z + self.options.offset.1,
        );
        Ok(Flow::Continue)
    }

    fn clean(&self, _: usize, slot: &mut JavaSlot) -> Option<BedrockChunkOut> {
        Some(std::mem::take(slot).out)
    }
}

/// [`java_to_bedrock`] with options and live progress.
pub fn java_to_bedrock_with(
    src: &Path,
    out: &Path,
    options: &ConvertOptions,
    progress: &Progress,
) -> Result<EditionReport, Error> {
    java_to_bedrock_replacing(src, out, options, progress, None)
}

/// [`java_to_bedrock_with`] that first replaces the blocks the target Bedrock version cannot
/// hold, as `replacements` says (the smart default for any block it has no choice for).
pub fn java_to_bedrock_replacing(
    src: &Path,
    out: &Path,
    options: &ConvertOptions,
    progress: &Progress,
    replace: Option<&BedrockReplacements>,
) -> Result<EditionReport, Error> {
    let world = JavaWorld::open(src)?;
    fresh_dir(out)?;
    let mut report = EditionReport::default();
    let mut entries: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    let mut work = Vec::new();
    for jdim in &world.dimensions {
        let Some(dim) = DIMS.iter().find(|d| d.java_id == jdim.id) else {
            report.notes.push(format!(
                "dimension {} has no Bedrock equivalent; skipped",
                jdim.id
            ));
            continue;
        };
        if options.includes(dim.number) {
            for rpos in jdim.regions()? {
                let region = Region::open(jdim.region_dir().join(rpos.file_name()))?;
                progress
                    .chunks_total
                    .fetch_add(region.chunks().count(), Ordering::Relaxed);
                let entity_path = jdim.entities_dir().join(rpos.file_name());
                let entity_region = if entity_path.exists() {
                    Some(Region::open(entity_path)?)
                } else {
                    None
                };
                work.push((jdim, dim, region, entity_region));
            }
        }
    }
    progress.regions_total.store(work.len(), Ordering::Relaxed);
    let jobs: Vec<(usize, usize, usize)> = work
        .iter()
        .enumerate()
        .flat_map(|(r, (_, _, region, _))| region.chunks().map(move |(lx, lz)| (r, lx, lz)))
        .collect();
    let stages = JavaToBedrockStages {
        work: &work,
        jobs: &jobs,
        options,
        progress,
        replace,
    };
    let finished = run_pipeline(&stages, jobs.len(), 0, progress)?;
    // Actors are stored apart from chunks: one record per actor plus, per chunk, the list
    // of the actors in it.
    let mut ids = crate::entity::ActorIds::default();
    let mut digests: BTreeMap<(i32, i32, i32), Vec<u8>> = BTreeMap::new();
    let mut in_chunk: BTreeMap<(i32, i32, i32), Vec<u8>> = BTreeMap::new();
    for (&(r, _, _), done) in jobs.iter().zip(finished) {
        let Some(done) = done else { continue };
        report.merge(done.report);
        if !done.records.is_empty() {
            *report
                .chunks
                .entry(work[r].1.java_id.to_string())
                .or_insert(0) += 1;
        }
        entries.extend(done.records);
        let dimension = work[r].1.bedrock;
        for entity in done.entities.iter().filter(|_| options.entities) {
            let Some(actor) =
                crate::entity::entity_to_actor(entity, options.offset, &mut ids, &mut report)
            else {
                continue;
            };
            let named = NamedTag {
                name: String::new(),
                tag: Tag::Compound(actor.nbt),
            };
            let bytes = mctk_nbt::to_bytes(&named, Flavor::Bedrock)?;
            // Before 1.18.30 a chunk holds its entities itself.
            if crate::bedrock_layout::get().entities_in_chunk {
                in_chunk.entry((dimension.id(), actor.chunk.0, actor.chunk.1)).or_default().extend(bytes);
                continue;
            }
            entries.insert(Key::Actor(actor.key.to_vec()).encode(), bytes);
            digests
                .entry((dimension.id(), actor.chunk.0, actor.chunk.1))
                .or_default()
                .extend(actor.key);
        }
    }
    for ((dim, x, z), list) in digests {
        let dimension = DIMS
            .iter()
            .map(|d| d.bedrock)
            .find(|d| d.id() == dim)
            .unwrap_or(BedrockDim::Overworld);
        entries.insert(Key::ActorDigest { x, z, dimension }.encode(), list);
        entries.insert(
            ChunkKey {
                x,
                z,
                dimension,
                tag: ChunkTag::ActorDigestVersion,
                subchunk: None,
            }
            .encode(),
            vec![0],
        );
    }
    for ((dim, x, z), bytes) in in_chunk {
        let dimension = DIMS.iter().map(|d| d.bedrock).find(|d| d.id() == dim).unwrap_or(BedrockDim::Overworld);
        entries.insert(ChunkKey { x, z, dimension, tag: ChunkTag::Entity, subchunk: None }.encode(), bytes);
    }
    progress.regions_done.store(work.len(), Ordering::Relaxed);
    if let Some(player) = world
        .level
        .root
        .tag
        .get("Data")
        .and_then(|d| d.get("Player"))
        .and_then(Tag::as_compound)
    {
        let game_mode = options
            .game_mode
            .unwrap_or(world.level.game_type.unwrap_or(0));
        let local = crate::entity::player_to_bedrock(player, game_mode, options.offset, &mut report);
        let named = NamedTag {
            name: String::new(),
            tag: Tag::Compound(local),
        };
        entries.insert(
            b"~local_player".to_vec(),
            mctk_nbt::to_bytes(&named, Flavor::Bedrock)?,
        );
    } else {
        report
            .notes
            .push("the world has no single-player data; Bedrock will create a new player".into());
    }
    report.notes.push(
        "entity data beyond type, position, health, age, name, sheep colour and dropped items \
         (owners, trades, equipment, variants) is not converted"
            .into(),
    );

    let level = bedrock_level_dat(&world, options);
    mctk_leveldb::write_db(
        out.join("db"),
        &entries,
        mctk_leveldb::WriteOptions::default(),
    )?;
    std::fs::write(out.join("level.dat"), level.encode()?)?;
    std::fs::write(out.join("levelname.txt"), &world.level.name)?;
    Ok(report)
}

// ------------------------------------------------------------------ Bedrock → Java

#[derive(Default, Clone)]
struct Column {
    subchunks: BTreeMap<i8, Vec<u8>>,
    data3d: Option<Vec<u8>>,
    block_entities: Vec<Compound>,
    /// `Data2D`: height map and biomes of chunks from before 1.18.
    data2d: Option<Vec<u8>>,
    /// `LegacyTerrain`: a whole chunk column in the oldest format.
    legacy_terrain: Option<Vec<u8>>,
    /// Actors that live in this chunk.
    actors: Vec<Compound>,
    /// `FinalizedState`: 2 when generation finished.
    finalized: Option<i32>,
}

impl Column {
    fn has_terrain(&self) -> bool {
        !self.subchunks.is_empty() || self.legacy_terrain.is_some()
    }
}

/// An item frame block found in a subchunk: local x, world y, local z, block name, states.
type FrameBlock = (i32, i32, i32, String, Compound);

fn is_water(tag: &NamedTag) -> bool {
    matches!(
        tag.tag.get("name").and_then(Tag::as_str),
        Some("minecraft:water" | "minecraft:flowing_water")
    )
}

/// One Bedrock subchunk as a Java section palette (26.x spelling: defaults omitted).
fn subchunk_to_java(
    storages: &[BedrockStorage],
    sy: i8,
    frames: &mut Vec<FrameBlock>,
    report: &mut EditionReport,
) -> PalettedContainer<BlockState> {
    let m = mappings();
    let Some(blocks) = storages.first() else {
        return PalettedContainer::filled(BlockState::air(), 4096);
    };
    let base: Vec<BlockState> = blocks
        .palette
        .iter()
        .map(|p| {
            let Some(b) = BedrockBlock::from_nbt(p) else {
                return BlockState::air();
            };
            // Item frames become entities (see `column_to_java`); a block caught mid-move has
            // no entity to go with it and is left as air.
            if crate::frames::is_frame_block(&b.name) || b.name == "minecraft:moving_block" {
                return BlockState::air();
            }
            match m.bedrock_to_java(&b.name, &b.states) {
                Some(j) => {
                    if !m.is_known_bedrock(&b.name, &b.states) {
                        *report
                            .approximated
                            .entry(format!("{} {:?}", b.name, b.states))
                            .or_insert(0) += 1;
                    }
                    j.clone()
                }
                None => {
                    *report.unmapped.entry(b.name.clone()).or_insert(0) += 1;
                    BlockState::air()
                }
            }
        })
        .collect();
    let frame_slots: Vec<Option<(String, Compound)>> = blocks
        .palette
        .iter()
        .map(|p| {
            BedrockBlock::from_nbt(p)
                .filter(|b| crate::frames::is_frame_block(&b.name))
                .map(|b| (b.name, b.states))
        })
        .collect();
    let water_layer = storages.get(1);
    let mut palette: Vec<BlockState> = Vec::new();
    let mut slot_of: HashMap<(u16, bool), u16> = HashMap::new();
    let mut indices = vec![0u16; 4096];
    for x in 0..16 {
        for z in 0..16 {
            for y in 0..16 {
                let b = mctk_bedrock::block_index(x, y, z);
                let slot = blocks.indices[b];
                if let Some((name, states)) = &frame_slots[slot as usize] {
                    frames.push((
                        x as i32,
                        sy as i32 * 16 + y as i32,
                        z as i32,
                        name.clone(),
                        states.clone(),
                    ));
                }
                let wet = water_layer.is_some_and(|w| is_water(&w.palette[w.indices[b] as usize]));
                let j = *slot_of.entry((slot, wet)).or_insert_with(|| {
                    let mut state = base[slot as usize].clone();
                    if wet {
                        if state.is_air() {
                            state = BlockState::new("minecraft:water").with("level", "0");
                        } else if state.properties.contains_key("waterlogged") {
                            state.properties.insert("waterlogged".into(), "true".into());
                        }
                    }
                    let state = m.strip_java_defaults(&state);
                    match palette.iter().position(|s| *s == state) {
                        Some(i) => i as u16,
                        None => {
                            palette.push(state);
                            (palette.len() - 1) as u16
                        }
                    }
                });
                indices[mctk_java::block_index(x, y, z)] = j;
            }
        }
    }
    PalettedContainer { palette, indices }
}

fn biomes_to_java(storage: Option<&BiomeStorage>, default: &str) -> PalettedContainer<String> {
    let m = mappings();
    let Some(BiomeStorage::Palettized {
        palette, indices, ..
    }) = storage
    else {
        return PalettedContainer::filled(default.to_string(), 64);
    };
    let mut out = PalettedContainer::filled(default.to_string(), 64);
    out.palette.clear();
    for cy in 0..4 {
        for cz in 0..4 {
            for cx in 0..4 {
                let id =
                    palette[indices[mctk_bedrock::block_index(cx * 4, cy * 4, cz * 4)] as usize];
                let name = m.bedrock_biome_to_java(id).unwrap_or(default).to_string();
                let slot = match out.palette.iter().position(|p| *p == name) {
                    Some(i) => i,
                    None => {
                        out.palette.push(name);
                        out.palette.len() - 1
                    }
                };
                out.indices[(cy * 4 + cz) * 4 + cx] = slot as u16;
            }
        }
    }
    out
}

fn column_to_java(
    x: i32,
    z: i32,
    col: &Column,
    dim: &DimInfo,
    offset: (i32, i32),
    report: &mut EditionReport,
) -> Result<(Chunk, Vec<Compound>), Error> {
    let data3d = col.data3d.as_deref().map(Data3D::parse).transpose()?;
    // Resolve "same as below" biome storages.
    let mut biome_by_section: BTreeMap<i8, BiomeStorage> = BTreeMap::new();
    if let Some(d) = &data3d {
        let mut last: Option<&BiomeStorage> = None;
        for (i, b) in d.biomes.iter().enumerate() {
            if !matches!(b, BiomeStorage::CopyPrevious) {
                last = Some(b);
            }
            if let Some(l) = last {
                biome_by_section.insert(dim.bedrock_sections.start + i as i8, l.clone());
            }
        }
    }
    let mut sections = Vec::new();
    let mut legacy_terrain: BTreeMap<i8, PalettedContainer<BlockState>> = col
        .legacy_terrain
        .as_deref()
        .map(|t| crate::legacy::bedrock_legacy_terrain(t, report).into_iter().collect())
        .unwrap_or_default();
    // Chunks from before 1.18 keep one biome per column in `Data2D`.
    let column_biomes = match (&data3d, &col.data2d) {
        (None, Some(d)) => crate::legacy::Biomes::from_bedrock_data2d(d),
        _ => None,
    };
    let mut frame_blocks: Vec<FrameBlock> = Vec::new();
    let mut last_biomes: Option<&BiomeStorage> = None;
    for sy in dim.java_sections.clone() {
        let blocks = match col.subchunks.get(&sy) {
            Some(raw) => match SubChunk::parse(raw)?.body {
                SubChunkBody::Palettized(storages) => {
                    subchunk_to_java(&storages, sy, &mut frame_blocks, report)
                }
                SubChunkBody::Legacy(body) => crate::legacy::bedrock_legacy_subchunk(&body, report),
            },
            None => legacy_terrain
                .remove(&sy)
                .unwrap_or_else(|| PalettedContainer::filled(BlockState::air(), 4096)),
        };
        if let Some(b) = biome_by_section.get(&sy) {
            last_biomes = Some(b);
        }
        let biomes = match &column_biomes {
            Some(b) => crate::legacy::section_biomes(b, sy, report),
            None => biomes_to_java(last_biomes, dim.default_biome),
        };
        sections.push(Section::new(
            sy,
            Some(JavaStorage::Palette(blocks)),
            Some(biomes),
        ));
    }
    // The block (with its default properties filled in) at a position of this column.
    let block_at = |bx: i32, by: i32, bz: i32| -> Option<BlockState> {
        let section = sections.iter().find(|s| s.y as i32 == by.div_euclid(16))?;
        let Some(JavaStorage::Palette(p)) = &section.blocks else {
            return None;
        };
        let idx = mctk_java::block_index(
            bx.rem_euclid(16) as usize,
            by.rem_euclid(16) as usize,
            bz.rem_euclid(16) as usize,
        );
        Some(mappings().complete_java(p.get(idx)))
    };
    let mut block_entities = Vec::new();
    // Entities that change their block: chest halves, skull kind, lectern and jukebox flags.
    let mut block_edits = Vec::new();
    for be in col.block_entities.iter() {
        // Item frames are converted with their blocks, below.
        if matches!(
            be.get("id").and_then(Tag::as_str),
            Some("ItemFrame" | "GlowItemFrame")
        ) {
            continue;
        }
        // The entity's own position is in output coordinates once the offset is applied,
        // so look the block up before shifting.
        let pos = (
            be.get("x").and_then(Tag::as_i64),
            be.get("y").and_then(Tag::as_i64),
            be.get("z").and_then(Tag::as_i64),
        );
        let block = match pos {
            (Some(bx), Some(by), Some(bz)) => block_at(bx as i32, by as i32, bz as i32),
            _ => None,
        };
        if let Some(j) = crate::block_entity::bedrock_to_java(be, offset, block.as_ref(), report) {
            if let (false, (Some(bx), Some(by), Some(bz))) = (j.edit.is_empty(), pos) {
                block_edits.push((bx as i32, by as i32, bz as i32, j.edit));
            }
            block_entities.extend(j.nbt);
        }
    }
    for (bx, by, bz, edit) in block_edits {
        let Some(section) = sections.iter_mut().find(|s| s.y as i32 == by.div_euclid(16)) else {
            continue;
        };
        let Some(JavaStorage::Palette(p)) = &mut section.blocks else {
            continue;
        };
        let idx = mctk_java::block_index(
            bx.rem_euclid(16) as usize,
            by.rem_euclid(16) as usize,
            bz.rem_euclid(16) as usize,
        );
        let mut state = mappings().complete_java(p.get(idx));
        // Only rename to a block Java has (a plant may have no potted form).
        let mut edit = edit;
        if let Some(name) = &edit.name {
            if !mappings().is_known_java(&BlockState::new(name.clone())) {
                report.skip("block changes to a block Java does not have", 1);
                edit.name = None;
            }
        }
        edit.apply(&mut state);
        // 26.x palettes leave out properties at their default.
        p.set(idx, mappings().strip_java_defaults(&state));
    }
    let mut fields = Compound::new();
    fields.insert("Status".into(), Tag::String("minecraft:full".into()));
    fields.insert("yPos".into(), Tag::Int(dim.java_sections.start as i32));
    fields.insert("LastUpdate".into(), Tag::Long(0));
    fields.insert("InhabitedTime".into(), Tag::Long(0));
    fields.insert(
        "block_entities".into(),
        Tag::List(
            List::from_tags(
                TagType::Compound,
                block_entities.into_iter().map(Tag::Compound).collect(),
            )
            .expect("compounds"),
        ),
    );
    let mut structures = Compound::new();
    structures.insert("References".into(), Tag::Compound(Compound::new()));
    structures.insert("starts".into(), Tag::Compound(Compound::new()));
    fields.insert("structures".into(), Tag::Compound(structures));
    // Item frames: a frame block with, usually, a block entity holding the item.
    let mut frames = Vec::new();
    if !frame_blocks.is_empty() {
        let mut notes = crate::item_codec::Notes::default();
        let entities: HashMap<(i64, i64, i64), &Compound> = col
            .block_entities
            .iter()
            .filter(|be| {
                matches!(
                    be.get("id").and_then(Tag::as_str),
                    Some("ItemFrame" | "GlowItemFrame")
                )
            })
            .filter_map(|be| {
                let at = |k: &str| be.get(k).and_then(Tag::as_i64);
                Some(((at("x")?, at("y")?, at("z")?), be))
            })
            .collect();
        for (lx, wy, lz, name, states) in &frame_blocks {
            let mut tile = ((x - offset.0) * 16 + lx, *wy, (z - offset.1) * 16 + lz);
            let held = entities.get(&(tile.0 as i64, tile.1 as i64, tile.2 as i64)).copied();
            tile.0 += offset.0 * 16;
            tile.2 += offset.1 * 16;
            let frame = crate::frames::Frame::from_bedrock(name, states, tile, held, &mut notes);
            let uuid = [0x4D43_544Bu32 as i32, tile.0, tile.1, tile.2];
            let java = frame.to_java(uuid);
            let id = java.get("id").and_then(Tag::as_str).unwrap_or_default().to_string();
            *report.converted_entities.entry(id).or_insert(0) += 1;
            frames.push(java);
        }
        crate::block_entity::report_item_notes(notes, report);
    }
    Ok((
        Chunk::new_modern(
            x,
            z,
            JAVA_DATA_VERSION,
            PaletteStyle::IdString,
            fields,
            sections,
        ),
        frames,
    ))
}

/// Top solid-or-not block + 1 at block column (`bx`, `bz`) of a converted chunk.
fn surface(chunk: &Chunk, bx: i32, bz: i32) -> Option<i32> {
    let (x, z) = (bx.rem_euclid(16) as usize, bz.rem_euclid(16) as usize);
    chunk.sections.iter().rev().find_map(|s| {
        let Some(JavaStorage::Palette(p)) = &s.blocks else {
            return None;
        };
        (0..16).rev().find_map(|y| {
            (!p.get(mctk_java::block_index(x, y, z)).is_air())
                .then(|| s.y as i32 * 16 + y as i32 + 1)
        })
    })
}

fn write_gzip_nbt(path: &Path, tag: &NamedTag) -> Result<(), Error> {
    let raw = mctk_nbt::to_bytes(tag, Flavor::Java)?;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&raw)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, e.finish()?)?;
    Ok(())
}

fn java_level_files(
    world: &BedrockWorld,
    spawn_y: i32,
    options: &ConvertOptions,
    player: Option<Compound>,
    out: &Path,
) -> Result<(), Error> {
    let level = &world.level.root.tag;
    let int = |k: &str, d: i64| {
        let overridden = match k {
            "GameType" => options.game_mode.map(i64::from),
            "Difficulty" => options.difficulty.map(i64::from),
            _ => None,
        };
        overridden.unwrap_or_else(|| level.get(k).and_then(Tag::as_i64).unwrap_or(d))
    };
    let (dx, dz) = (options.offset.0 * 16, options.offset.1 * 16);
    let mut data = Compound::new();
    let name = world
        .level_name_txt
        .clone()
        .or_else(|| world.level.level_name().map(str::to_string))
        .unwrap_or_else(|| "Converted World".into());
    data.insert("LevelName".into(), Tag::String(name));
    data.insert("DataVersion".into(), Tag::Int(JAVA_DATA_VERSION));
    data.insert("version".into(), Tag::Int(19133));
    let mut version = Compound::new();
    version.insert("Id".into(), Tag::Int(JAVA_DATA_VERSION));
    version.insert("Name".into(), Tag::String("26.3".into()));
    version.insert("Series".into(), Tag::String("main".into()));
    version.insert("Snapshot".into(), Tag::Byte(0));
    data.insert("Version".into(), Tag::Compound(version));
    data.insert(
        "GameType".into(),
        Tag::Int(int("GameType", 0).clamp(0, 3) as i32),
    );
    data.insert("Time".into(), Tag::Long(int("Time", 0)));
    data.insert("LastPlayed".into(), Tag::Long(now_secs() * 1000));
    data.insert(
        "allowCommands".into(),
        Tag::Byte(int("commandsEnabled", 0) as i8),
    );
    data.insert("initialized".into(), Tag::Byte(1));
    data.insert("WasModded".into(), Tag::Byte(0));
    let mut spawn = Compound::new();
    spawn.insert(
        "pos".into(),
        Tag::IntArray(vec![
            int("SpawnX", 0) as i32 + dx,
            spawn_y,
            int("SpawnZ", 0) as i32 + dz,
        ]),
    );
    spawn.insert(
        "dimension".into(),
        Tag::String("minecraft:overworld".into()),
    );
    spawn.insert("yaw".into(), Tag::Float(0.0));
    spawn.insert("pitch".into(), Tag::Float(0.0));
    data.insert("spawn".into(), Tag::Compound(spawn));
    let mut difficulty = Compound::new();
    let d = ["peaceful", "easy", "normal", "hard"][int("Difficulty", 2).clamp(0, 3) as usize];
    difficulty.insert("difficulty".into(), Tag::String(d.into()));
    difficulty.insert("hardcore".into(), Tag::Byte(int("IsHardcore", 0) as i8));
    difficulty.insert("locked".into(), Tag::Byte(0));
    data.insert("difficulty_settings".into(), Tag::Compound(difficulty));
    let mut packs = Compound::new();
    packs.insert(
        "Enabled".into(),
        Tag::List(List::from_tags(
            TagType::String,
            vec![Tag::String("vanilla".into())],
        )?),
    );
    packs.insert("Disabled".into(), Tag::List(List::new(TagType::String)));
    data.insert("DataPacks".into(), Tag::Compound(packs));
    if let Some(p) = player {
        data.insert("Player".into(), Tag::Compound(p));
    }
    let mut root = Compound::new();
    root.insert("Data".into(), Tag::Compound(data));
    write_gzip_nbt(
        &out.join("level.dat"),
        &NamedTag {
            name: String::new(),
            tag: Tag::Compound(root),
        },
    )?;

    // World generation for chunks beyond the converted area: vanilla presets with the
    // Bedrock seed (the terrain will differ from Bedrock's own generator).
    let dimension = |settings: &str, biome_source: Compound, ty: &str| {
        let mut generator = Compound::new();
        generator.insert("settings".into(), Tag::String(settings.into()));
        generator.insert("biome_source".into(), Tag::Compound(biome_source));
        generator.insert("type".into(), Tag::String("minecraft:noise".into()));
        let mut d = Compound::new();
        d.insert("generator".into(), Tag::Compound(generator));
        d.insert("type".into(), Tag::String(ty.into()));
        Tag::Compound(d)
    };
    let multi_noise = |preset: &str| {
        let mut c = Compound::new();
        c.insert("preset".into(), Tag::String(preset.into()));
        c.insert("type".into(), Tag::String("minecraft:multi_noise".into()));
        c
    };
    let mut end_source = Compound::new();
    end_source.insert("type".into(), Tag::String("minecraft:the_end".into()));
    let mut dims = Compound::new();
    dims.insert(
        "minecraft:overworld".into(),
        dimension(
            "minecraft:overworld",
            multi_noise("minecraft:overworld"),
            "minecraft:overworld",
        ),
    );
    dims.insert(
        "minecraft:the_nether".into(),
        dimension(
            "minecraft:nether",
            multi_noise("minecraft:nether"),
            "minecraft:the_nether",
        ),
    );
    dims.insert(
        "minecraft:the_end".into(),
        dimension("minecraft:end", end_source, "minecraft:the_end"),
    );
    let mut wgs = Compound::new();
    wgs.insert("bonus_chest".into(), Tag::Byte(0));
    wgs.insert("seed".into(), Tag::Long(int("RandomSeed", 0)));
    wgs.insert("generate_structures".into(), Tag::Byte(1));
    wgs.insert("dimensions".into(), Tag::Compound(dims));
    let mut root = Compound::new();
    root.insert("data".into(), Tag::Compound(wgs));
    root.insert("DataVersion".into(), Tag::Int(JAVA_DATA_VERSION));
    write_gzip_nbt(
        &out.join("data/minecraft/world_gen_settings.dat"),
        &NamedTag {
            name: String::new(),
            tag: Tag::Compound(root),
        },
    )
}

/// Converts the Bedrock world `src` (folder or `.mcworld`) into a new Java 26.3 world
/// folder at `out`.
pub fn bedrock_to_java(src: &Path, out: &Path) -> Result<EditionReport, Error> {
    bedrock_to_java_with(src, out, &ConvertOptions::default(), &Progress::default())
}

/// The chunk a Java entity's `Pos` lies in.
fn entity_chunk(entity: &Compound) -> Option<(i32, i32)> {
    let pos = entity.get("Pos").and_then(Tag::as_list)?;
    let coord = |i: usize| match pos.get(i)? {
        Tag::Double(d) => Some(*d),
        Tag::Float(f) => Some(*f as f64),
        _ => None,
    };
    Some((
        (coord(0)?.floor() as i32).div_euclid(16),
        (coord(2)?.floor() as i32).div_euclid(16),
    ))
}

/// A Bedrock chunk column queued for conversion.
struct BedrockJob<'a> {
    dim_index: usize,
    dim_id: i32,
    x: i32,
    z: i32,
    col: &'a Column,
}

struct JavaChunkOut {
    dim_index: usize,
    pos: ChunkPos,
    raw: RawChunk,
    report: EditionReport,
    /// Surface height at the world spawn, when this chunk holds it.
    spawn_y: Option<i32>,
    /// Java entities of the chunk (stored in the dimension's `entities` regions).
    entities: Vec<Compound>,
}

/// Per-chunk work of Bedrock → Java, for [`run_pipeline`]. The database is read up front,
/// so only the Convert stage has work to do.
struct BedrockToJavaStages<'a> {
    jobs: &'a [BedrockJob<'a>],
    options: &'a ConvertOptions,
    progress: &'a Progress,
    timestamp: u32,
    spawn: (i32, i32),
    /// Blocks the target cannot hold are replaced as the user chose, before they are mapped.
    replace: Option<&'a JavaReplacements>,
}

#[derive(Default)]
struct BedrockSlot {
    out: Option<JavaChunkOut>,
}

impl ChunkStages for BedrockToJavaStages<'_> {
    type Slot = BedrockSlot;
    type Output = JavaChunkOut;

    fn convert(&self, index: usize, slot: &mut BedrockSlot) -> Result<Flow, Error> {
        let job = &self.jobs[index];
        let dim = &DIMS[job.dim_index];
        self.progress.set_dimension(dim.java_id);
        let (ox, oz) = (
            job.x + self.options.offset.0,
            job.z + self.options.offset.1,
        );
        let mut report = EditionReport::default();
        let replaced;
        let col = match self.replace {
            Some(r) => {
                replaced = r.column(job.col, &mut report);
                &replaced
            }
            None => job.col,
        };
        let bare;
        let col = if self.options.tile_entities {
            col
        } else {
            bare = Column { block_entities: Vec::new(), ..col.clone() };
            &bare
        };
        let (mut chunk, frames) =
            column_to_java(ox, oz, col, dim, self.options.offset, &mut report)?;
        // Spell every block state the way 26.x does (full states; the default state as the bare
        // id), whatever the mapping tables produced.
        if let Some(reg) = java_registry() {
            for section in &mut chunk.sections {
                if let Some(JavaStorage::Palette(c)) = &mut section.blocks {
                    crate::java_java::remap_blocks(
                        c,
                        |s| (crate::java_java::finalize_state(reg, PaletteStyle::IdString, s.clone()), None),
                        &mut indexmap::IndexMap::new(),
                    );
                }
            }
        }
        let (spawn_x, spawn_z) = self.spawn;
        let spawn_y = (job.dim_id == 0
            && job.x == spawn_x.div_euclid(16)
            && job.z == spawn_z.div_euclid(16))
        .then(|| surface(&chunk, spawn_x, spawn_z))
        .flatten();
        let raw = RawChunk::encode(&chunk.to_nbt(), ChunkCompression::Zlib, self.timestamp)?;
        let mut entities: Vec<Compound> = job
            .col
            .actors
            .iter()
            .filter(|_| self.options.entities)
            .filter_map(|a| crate::entity::actor_to_java(a, self.options.offset, &mut report))
            .collect();
        entities.extend(frames);
        self.progress.chunk(ox, oz);
        slot.out = Some(JavaChunkOut {
            dim_index: job.dim_index,
            pos: ChunkPos::new(ox, oz),
            raw,
            report,
            spawn_y,
            entities,
        });
        Ok(Flow::Continue)
    }

    fn clean(&self, _: usize, slot: &mut BedrockSlot) -> Option<JavaChunkOut> {
        slot.out.take()
    }
}

/// [`bedrock_to_java`] with options and live progress.
pub fn bedrock_to_java_with(
    src: &Path,
    out: &Path,
    options: &ConvertOptions,
    progress: &Progress,
) -> Result<EditionReport, Error> {
    bedrock_to_java_replacing(src, out, options, progress, None)
}

/// [`bedrock_to_java_with`] that first replaces the blocks the target Java version cannot hold,
/// as `replacements` says (the smart default for any block it has no choice for).
pub fn bedrock_to_java_replacing(
    src: &Path,
    out: &Path,
    options: &ConvertOptions,
    progress: &Progress,
    replace: Option<&JavaReplacements>,
) -> Result<EditionReport, Error> {
    let world = BedrockWorld::open(src)?;
    fresh_dir(out)?;
    let mut report = EditionReport::default();
    let mut columns: BTreeMap<(i32, i32, i32), Column> = BTreeMap::new();
    // Actors are stored apart from chunks: `actorprefix<uid>` holds each one and
    // `digp<x><z>[<dim>]` lists the ids in a chunk.
    let mut actor_by_uid: HashMap<Vec<u8>, Compound> = HashMap::new();
    let mut digests: Vec<(i32, i32, i32, &[u8])> = Vec::new();
    let mut player = None;
    for (k, v) in &world.db.entries {
        match Key::decode(k) {
            Key::Actor(uid) => {
                if let Ok(root) = mctk_nbt::from_bytes(v, Flavor::Bedrock) {
                    if let Tag::Compound(c) = root.tag {
                        actor_by_uid.insert(uid, c);
                    }
                }
                continue;
            }
            Key::ActorDigest { x, z, dimension } => {
                digests.push((dimension.id(), x, z, v));
                continue;
            }
            Key::Other(name) if name == b"~local_player" => {
                if let Ok(root) = mctk_nbt::from_bytes(v, Flavor::Bedrock) {
                    if let Tag::Compound(c) = root.tag {
                        player = Some(c);
                    }
                }
                continue;
            }
            _ => {}
        }
        match Key::decode(k) {
            Key::Chunk(c) => {
                let col = columns.entry((c.dimension.id(), c.x, c.z)).or_default();
                match c.tag {
                    ChunkTag::SubChunkPrefix => {
                        col.subchunks.insert(c.subchunk.unwrap_or(0), v.clone());
                    }
                    ChunkTag::Data3D => col.data3d = Some(v.clone()),
                    ChunkTag::Data2D => col.data2d = Some(v.clone()),
                    ChunkTag::LegacyTerrain => col.legacy_terrain = Some(v.clone()),
                    ChunkTag::FinalizedState if v.len() >= 4 => {
                        col.finalized = Some(i32::from_le_bytes(v[..4].try_into().unwrap()));
                    }
                    ChunkTag::BlockEntity => {
                        let mut s = v.as_slice();
                        while !s.is_empty() {
                            let Ok(root) = mctk_nbt::read_root(&mut s, Flavor::Bedrock) else {
                                break;
                            };
                            if let Tag::Compound(c) = root.tag {
                                col.block_entities.push(c);
                            }
                        }
                    }
                    // Before 1.18.30 a chunk holds its entities itself.
                    ChunkTag::Entity => {
                        let mut s = v.as_slice();
                        while !s.is_empty() {
                            let Ok(root) = mctk_nbt::read_root(&mut s, Flavor::Bedrock) else {
                                break;
                            };
                            if let Tag::Compound(c) = root.tag {
                                col.actors.push(c);
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let mut homeless = 0;
    for (dim, x, z, list) in digests {
        for uid in list.chunks_exact(8) {
            let Some(actor) = actor_by_uid.remove(uid) else {
                continue;
            };
            match columns.get_mut(&(dim, x, z)) {
                Some(col) => col.actors.push(actor),
                None => homeless += 1,
            }
        }
    }
    // Actors no chunk list refers to are not placed anywhere.
    report.skip(
        "entities in chunks without terrain or in no chunk",
        homeless + actor_by_uid.len(),
    );

    let timestamp = now_secs() as u32;
    let (spawn_x, spawn_y, spawn_z) = {
        let l = &world.level.root.tag;
        let g = |k: &str| l.get(k).and_then(Tag::as_i64).unwrap_or(0) as i32;
        (g("SpawnX"), g("SpawnY"), g("SpawnZ"))
    };
    let mut java_spawn_y = None;
    let mut regions: HashMap<(usize, RegionPos), RegionBuilder> = HashMap::new();
    let mut entity_regions: HashMap<(usize, RegionPos), RegionBuilder> = HashMap::new();
    let mut entity_chunks: BTreeMap<(usize, i32, i32), Vec<Compound>> = BTreeMap::new();
    let dim_of =|id: i32| DIMS.iter().enumerate().find(|(_, d)| d.bedrock.id() == id);
    let todo: Vec<_> = columns
        .iter()
        .filter(|((d, _, _), c)| {
            c.has_terrain()
                && dim_of(*d).is_some_and(|(_, dim)| options.includes(dim.number))
        })
        .collect();
    progress.chunks_total.store(todo.len(), Ordering::Relaxed);
    // Columns that will be converted; the others are only counted in the report.
    let mut jobs = Vec::new();
    for ((dim_id, x, z), col) in &columns {
        let Some((di, dim)) = dim_of(*dim_id) else {
            continue;
        };
        if !options.includes(dim.number) {
            continue;
        }
        if !col.has_terrain() {
            report.skip("chunk columns without terrain", 1);
            continue;
        }
        if options.skip_partial_chunks && col.finalized.is_some_and(|f| f != 2) {
            report.skip("partially generated chunks", 1);
            continue;
        }
        jobs.push(BedrockJob {
            dim_index: di,
            dim_id: *dim_id,
            x: *x,
            z: *z,
            col,
        });
    }
    let stages = BedrockToJavaStages {
        jobs: &jobs,
        options,
        progress,
        timestamp,
        spawn: (spawn_x, spawn_z),
        replace,
    };
    let finished = run_pipeline(&stages, jobs.len(), 0, progress)?;
    for done in finished.into_iter().flatten() {
        report.merge(done.report);
        if done.spawn_y.is_some() {
            java_spawn_y = done.spawn_y;
        }
        let (lx, lz) = done.pos.local();
        regions
            .entry((done.dim_index, done.pos.region()))
            .or_default()
            .set(lx, lz, Some(done.raw));
        // File each entity under the chunk its final position lies in (a painting's position
        // can move across a chunk edge when it is converted).
        for e in done.entities {
            let at = entity_chunk(&e).unwrap_or((done.pos.x, done.pos.z));
            entity_chunks.entry((done.dim_index, at.0, at.1)).or_default().push(e);
        }
        *report
            .chunks
            .entry(DIMS[done.dim_index].java_id.to_string())
            .or_insert(0) += 1;
    }
    for ((di, cx, cz), entities) in entity_chunks {
        let mut root = Compound::new();
        root.insert("DataVersion".into(), Tag::Int(JAVA_DATA_VERSION));
        root.insert("Position".into(), Tag::IntArray(vec![cx, cz]));
        root.insert(
            "Entities".into(),
            Tag::List(List::from_tags(
                TagType::Compound,
                entities.into_iter().map(Tag::Compound).collect(),
            )?),
        );
        let named = NamedTag {
            name: String::new(),
            tag: Tag::Compound(root),
        };
        let pos = ChunkPos::new(cx, cz);
        let (lx, lz) = pos.local();
        entity_regions.entry((di, pos.region())).or_default().set(
            lx,
            lz,
            Some(RawChunk::encode(&named, ChunkCompression::Zlib, timestamp)?),
        );
    }
    progress.regions_done.store(1, Ordering::Relaxed);
    for ((di, pos), builder) in &regions {
        let dir = out.join(DIMS[*di].java_folder).join("region");
        std::fs::create_dir_all(&dir)?;
        builder.write(&dir, *pos)?;
    }
    for ((di, pos), builder) in &entity_regions {
        let dir = out.join(DIMS[*di].java_folder).join("entities");
        std::fs::create_dir_all(&dir)?;
        builder.write(&dir, *pos)?;
    }
    let spawn_y = if spawn_y == 32767 {
        java_spawn_y.unwrap_or(64)
    } else {
        spawn_y
    };
    let player = player.map(|p| crate::entity::player_to_java(&p, options.offset, &mut report));
    java_level_files(&world, spawn_y, options, player, out)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Java chunk of stone with one air block at (3, 65, 4) for a frame to hang in.
    fn stone_chunk() -> Chunk {
        let mut blocks = PalettedContainer::filled(BlockState::new("minecraft:stone"), 4096);
        blocks.set(mctk_java::block_index(3, 1, 4), BlockState::air());
        let section = Section::new(
            4,
            Some(JavaStorage::Palette(blocks)),
            Some(PalettedContainer::filled("minecraft:plains".to_string(), 64)),
        );
        Chunk::new_modern(
            0,
            0,
            JAVA_DATA_VERSION,
            PaletteStyle::IdString,
            Compound::new(),
            vec![section],
        )
    }

    fn java_frame() -> Compound {
        let mut item = Compound::new();
        item.insert("id".into(), Tag::String("minecraft:diamond".into()));
        item.insert("count".into(), Tag::Int(3));
        let mut e = Compound::new();
        e.insert("id".into(), Tag::String("minecraft:item_frame".into()));
        e.insert("block_pos".into(), Tag::IntArray(vec![3, 65, 4]));
        e.insert("facing".into(), Tag::Byte(3));
        e.insert("ItemRotation".into(), Tag::Byte(2));
        e.insert("Item".into(), Tag::Compound(item));
        e
    }

    #[test]
    fn a_version_before_1_18_gets_data2d_and_an_old_chunk_version() {
        use crate::bedrock_layout::Layout;
        let layout = Layout { legacy_height: true, chunk_version: 25, entities_in_chunk: true, blending: false };
        let mut report = EditionReport::default();
        let records = chunk_to_bedrock_in(&stone_chunk(), &DIMS[0], (0, 0), &[], &[], &mut report, layout);
        let mut col = Column::default();
        let mut tags = Vec::new();
        for (k, v) in &records {
            let Key::Chunk(c) = Key::decode(k) else { continue };
            tags.push(c.tag);
            match c.tag {
                ChunkTag::SubChunkPrefix => {
                    assert_eq!(v[0], 8, "palettized, without a stored y");
                    col.subchunks.insert(c.subchunk.unwrap(), v.clone());
                }
                ChunkTag::Data2D => col.data2d = Some(v.clone()),
                ChunkTag::Version => assert_eq!(v, &vec![25]),
                _ => {}
            }
        }
        assert!(tags.contains(&ChunkTag::Data2D));
        assert!(!tags.contains(&ChunkTag::Data3D) && !tags.contains(&ChunkTag::BlendingData));
        assert_eq!(col.data2d.as_ref().map(Vec::len), Some(768));
        let (chunk, _) = column_to_java(0, 0, &col, &DIMS[0], (0, 0), &mut report).unwrap();
        assert_eq!(chunk.block(0, 64, 0).map(|b| b.name.as_str()), Some("minecraft:stone"));
        assert!(chunk.block(3, 65, 4).unwrap().is_air());
    }

    #[test]
    fn item_frames_round_trip_between_entity_and_block() {
        let frames = vec![
            crate::frames::Frame::from_java(&java_frame()).expect("a frame"),
        ];
        let mut report = EditionReport::default();
        let records = chunk_to_bedrock(&stone_chunk(), &DIMS[0], (0, 0), &[], &frames, &mut report);
        assert_eq!(report.converted_entities.get("minecraft:item_frame"), Some(&1));

        let mut col = Column::default();
        for (k, v) in &records {
            let Key::Chunk(c) = Key::decode(k) else { continue };
            match c.tag {
                ChunkTag::SubChunkPrefix => {
                    col.subchunks.insert(c.subchunk.unwrap(), v.clone());
                }
                ChunkTag::Data3D => col.data3d = Some(v.clone()),
                ChunkTag::BlockEntity => {
                    let mut s = v.as_slice();
                    while let Ok(root) = mctk_nbt::read_root(&mut s, Flavor::Bedrock) {
                        if let Tag::Compound(c) = root.tag {
                            col.block_entities.push(c);
                        }
                    }
                }
                _ => {}
            }
        }

        // Bedrock: a frame block facing south and an `ItemFrame` entity holding 3 diamonds.
        let SubChunkBody::Palettized(storages) = SubChunk::parse(&col.subchunks[&4]).unwrap().body
        else {
            panic!("palettized")
        };
        let at = mctk_bedrock::block_index(3, 1, 4);
        let block = BedrockBlock::from_nbt(&storages[0].palette[storages[0].indices[at] as usize]).unwrap();
        assert_eq!(block.name, "minecraft:frame");
        assert_eq!(block.states.get("facing_direction"), Some(&Tag::Int(3)));
        let be = col
            .block_entities
            .iter()
            .find(|b| b.get("id").and_then(Tag::as_str) == Some("ItemFrame"))
            .expect("the frame's block entity");
        assert_eq!(be.get("ItemRotation"), Some(&Tag::Float(90.0)));
        assert!(be.get("Item").is_some());

        // Back to Java: the block is air again and the entity hangs where it did.
        let mut report = EditionReport::default();
        let (chunk, entities) = column_to_java(0, 0, &col, &DIMS[0], (0, 0), &mut report).unwrap();
        assert!(chunk.block(3, 65, 4).unwrap().is_air());
        assert!(report.unmapped.is_empty(), "{:?}", report.unmapped);
        let [frame] = entities.as_slice() else { panic!("one frame") };
        assert_eq!(frame.get("id").and_then(Tag::as_str), Some("minecraft:item_frame"));
        assert_eq!(frame.get("block_pos"), Some(&Tag::IntArray(vec![3, 65, 4])));
        assert_eq!(frame.get("facing"), Some(&Tag::Byte(3)));
        assert_eq!(frame.get("ItemRotation"), Some(&Tag::Byte(2)));
        let item = frame.get("Item").and_then(Tag::as_compound).expect("the item");
        assert_eq!(item.get("id").and_then(Tag::as_str), Some("minecraft:diamond"));
        assert_eq!(item.get("count").and_then(Tag::as_i64), Some(3));
    }

    #[test]
    fn item_frames_follow_the_chunk_offset() {
        let frames = vec![
            crate::frames::Frame::from_java(&java_frame()).unwrap(),
        ];
        let mut report = EditionReport::default();
        let records = chunk_to_bedrock(&stone_chunk(), &DIMS[0], (1, 2), &[], &frames, &mut report);
        let (_, v) = records
            .iter()
            .find(|(k, _)| matches!(Key::decode(k), Key::Chunk(c) if c.tag == ChunkTag::BlockEntity))
            .expect("block entities");
        let root = mctk_nbt::read_root(&mut v.as_slice(), Flavor::Bedrock).unwrap();
        assert_eq!(root.tag.get("x").and_then(Tag::as_i64), Some(3 + 16));
        assert_eq!(root.tag.get("z").and_then(Tag::as_i64), Some(4 + 32));
    }

    #[test]
    fn pre_1_2_13_bedrock_chunks_are_read() {
        // A version 0 subchunk: 4096 block ids then 2048 data nibbles, in x, z, y order.
        let mut body = vec![0u8; 1 + 4096 + 2048];
        let at = mctk_bedrock::block_index(3, 2, 4);
        for b in &mut body[1..4097] {
            *b = 1;
        }
        body[1 + at] = 35;
        // Data 5 (lime) for that block: odd indices use the high nibble.
        let nib = 4097 + at / 2;
        body[nib] |= if at % 2 == 0 { 5 } else { 5 << 4 };
        let mut col = Column::default();
        col.subchunks.insert(0, body);
        let mut d2 = vec![0u8; 768];
        d2[512..].fill(4);
        col.data2d = Some(d2);
        let mut report = EditionReport::default();
        let (chunk, _) = column_to_java(0, 0, &col, &DIMS[0], (0, 0), &mut report).unwrap();
        assert_eq!(chunk.block(3, 2, 4).unwrap().name, "minecraft:lime_wool");
        assert_eq!(chunk.block(0, 0, 0).unwrap().name, "minecraft:stone");
        assert!(report.unmapped.is_empty(), "{:?}", report.unmapped);
        // The biome column bytes are Bedrock ids: 4 is forest.
        let section = chunk.section(0).unwrap();
        let b = section.biomes.as_ref().unwrap();
        assert_eq!(b.palette[b.indices[0] as usize], "minecraft:forest");
    }
}