//! Java -> Java conversion between versions.
//!
//! Every block state goes through the replacement engine ([`mctk_replace`]) against the
//! target version's registry, then is written in the target's palette style with the
//! properties that style needs (full states for 1.18 – 1.21, defaults omitted for 26.x).
//! Lighting is not converted: light data is dropped and the chunk marked unlit, so the game
//! computes it itself, which avoids the stale-light seams converted worlds are known for.
//!
//! Sources and targets both run from 1.8 to 26.x. A source older than 1.18 is first brought to the\n//! 1.18 layout and the modern form (upgrade_old_chunk, with the tables in legacy); worlds of\n//! 1.20.5 to 1.21.4 are read through the modern form (see Form::source). A target of 1.13 to 1.17\n//! is written in the old chunk layout (java_old), one of 1.8 to 1.12 with numeric ids\n//! (java_numeric), and one in the ranges 1.20.5 to 1.21.4 and 1.21.5 to 1.21.11 in the layout below\n//! it, which the game upgrades when the world is opened (see [JavaTarget]).

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use indexmap::IndexMap;
use mctk_anvil::{ChunkCompression, RawChunk, Region, RegionBuilder};
use mctk_java::{BlockState, BlockStorage, Chunk, ChunkEra, JavaWorld, PaletteStyle, PalettedContainer};
use mctk_nbt::{Compound, Flavor, List, NamedTag, Tag, TagType};
use mctk_registry::{Registry, Renames};
use mctk_replace::{Action, ReplacementMap};

use crate::Error;
use crate::java_entities::{Form, block_entity_convert, entity_convert};
use crate::java_items::Notes;
use crate::java_level::LevelData;
use crate::options::{ConvertOptions, Progress, is_complete};
use crate::pipeline::{ChunkStages, Flow, run_pipeline};

/// The version a world is converted to.
///
/// The world is written in the newest layout this converter can write that the target version
/// still reads, stamped with that layout's `DataVersion`: the game upgrades older data itself when
/// it opens the world, so a target between two layouts (1.20.5 to 1.21.4, 1.21.6 and later in the
/// 1.21 line) is written as the layout below it. `registry` is always the target's own, so
/// every block, biome and entity is checked against the real version.
#[derive(Debug, Clone)]
pub struct JavaTarget {
    pub registry: Registry,
    pub renames: Renames,
    pub palette_style: PaletteStyle,
    /// The NBT conventions the data is written in.
    pub form: Form,
    /// The `DataVersion` and version name written into the world.
    pub stamp: (i32, String),
}

impl JavaTarget {
    fn stamped(registry: Registry, palette_style: PaletteStyle, form: Form, stamp: (i32, &str)) -> Self {
        JavaTarget {
            registry,
            renames: Renames::java_blocks(),
            palette_style,
            form,
            stamp: (stamp.0, stamp.1.to_string()),
        }
    }

    /// 1.18 – 1.20.4 storage: `{Name, Properties}` palettes holding full states.
    pub fn classic(registry: Registry) -> Self {
        let stamp = (registry.data_version.unwrap_or(3465), registry.version.clone());
        JavaTarget {
            renames: Renames::java_blocks(),
            palette_style: PaletteStyle::NameProperties,
            form: Form::Classic,
            stamp,
            registry,
        }
    }

    /// The right layout for the registry's version (see the type's documentation).
    pub fn for_version(registry: Registry) -> Result<Self, Error> {
        let dv = registry
            .data_version
            .ok_or_else(|| Error::Invalid(format!("registry {} has no data version", registry.version)))?;
        Ok(match dv {
            ..3837 => Self::classic(registry),
            // 1.20.5 - 1.21.4 read 1.20.4 data and upgrade it.
            3837..4325 => Self::stamped(registry, PaletteStyle::NameProperties, Form::Classic, (3700, "1.20.4")),
            // 1.21.5 up to the 26 line: component items and NBT text in the folder layout of 1.20.
            4325..4786 => Self::stamped(registry, PaletteStyle::NameProperties, Form::Modern, (4325, "1.21.5")),
            _ => Self::modern(registry),
        })
    }

    /// 26.x storage: id-string palettes without default properties, dimensions under
    /// `dimensions/<namespace>/<name>/`, level settings split over `data/minecraft/*.dat`.
    pub fn modern(registry: Registry) -> Self {
        let stamp = (registry.data_version.unwrap_or(5023), registry.version.clone());
        JavaTarget {
            renames: Renames::java_blocks(),
            palette_style: PaletteStyle::IdString,
            form: Form::Modern,
            stamp,
            registry,
        }
    }

    /// Whether the world is written in the 26.x folder layout.
    fn is_modern(&self) -> bool {
        self.palette_style == PaletteStyle::IdString
    }

    fn data_version(&self) -> Result<i32, Error> {
        Ok(self.stamp.0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacedBlock {
    pub to: String,
    pub blocks: u64,
}

/// What a conversion changed or could not carry over.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JavaReport {
    pub chunks: u64,
    /// Chunks in a layout older than 1.18, left out.
    pub unsupported_chunks: u64,
    /// Source block state -> what it became, with the number of blocks.
    pub replaced: IndexMap<String, ReplacedBlock>,
    /// Source biome -> number of 4x4x4 cells moved to the fallback biome.
    pub biomes_replaced: IndexMap<String, u64>,
    /// Block entities, entities and item data that could not be carried over, with counts.
    pub notes: Notes,
    /// Entities and block entities that were converted.
    pub entities_converted: u64,
    pub block_entities_converted: u64,
    /// Players whose data, advancements and statistics were carried over.
    pub players: u64,
    /// Saved-data files (scoreboard, maps, ...) copied unchanged between worlds of one layout.
    pub saved_data_files: u64,
}

impl JavaReport {
    fn merge(&mut self, other: JavaReport) {
        self.chunks += other.chunks;
        self.unsupported_chunks += other.unsupported_chunks;
        for (k, v) in other.replaced {
            self.replaced
                .entry(k)
                .and_modify(|r| r.blocks += v.blocks)
                .or_insert(v);
        }
        for (k, v) in other.biomes_replaced {
            *self.biomes_replaced.entry(k).or_insert(0) += v;
        }
        self.notes.merge(other.notes);
        self.entities_converted += other.entities_converted;
        self.block_entities_converted += other.block_entities_converted;
    }
}

const FALLBACK_BIOME: &str = "minecraft:plains";

/// The biome to use where the target version has no `name`: the same biome with leading words
/// dropped (`dappled_forest` becomes `forest`, `old_growth_pine_taiga` becomes `taiga`), else plains.
fn fallback_biome(name: &str, known: &[String]) -> String {
    let bare = name.strip_prefix("minecraft:").unwrap_or(name);
    let words: Vec<&str> = bare.split('_').collect();
    for skip in 1..words.len() {
        let candidate = format!("minecraft:{}", words[skip..].join("_"));
        if known.contains(&candidate) {
            return candidate;
        }
    }
    FALLBACK_BIOME.to_string()
}
/// Per-section lighting; the game recomputes it for a chunk marked unlit.
const LIGHT_KEYS: [&str; 2] = ["SkyLight", "BlockLight"];

struct Ctx<'a> {
    target: &'a JavaTarget,
    map: &'a ReplacementMap,
    data_version: i32,
    to_form: Form,
    /// Whether tile entities and mobs are converted (the Converter's options).
    tiles: bool,
    mobs: bool,
    ticks: bool,
    depth: crate::depth::DepthMode,
    /// Chunks with an older DataVersion are left out (0 keeps all).
    min_dv: i32,
}

impl Ctx<'_> {
    /// The state as the target stores it (see [`finalize_state`]).
    fn finalize(&self, s: BlockState) -> BlockState {
        finalize_state(&self.target.registry, self.target.palette_style, s)
    }

    /// The target state for a source state, and the replacement made, if any.
    fn block(&self, s: &BlockState) -> (BlockState, Option<String>) {
        match self.map.resolve(s, &self.target.registry, &self.target.renames) {
            None => (self.finalize(s.clone()), None),
            Some((Action::Replace(r), _)) => {
                let r = self.finalize(r);
                let text = r.to_string();
                (r, Some(text))
            }
            Some((Action::Air, _)) => (BlockState::air(), Some("air".into())),
            // `resolve` turns `Block` into `Replace`, so only `Keep` is left to write as is.
            Some((Action::Keep | Action::Block(_), _)) => (self.finalize(s.clone()), None),
        }
    }
}

/// A state as the version stores it: always the full state, except that 26.x writes the block's
/// default state as the bare id (verified against a real 26.3 server's saves). A state the
/// registry does not know is returned as it is.
pub(crate) fn finalize_state(registry: &Registry, style: PaletteStyle, s: BlockState) -> BlockState {
    let props = s.properties.iter().map(|(k, v)| (k.as_str(), v.as_str()));
    let Some(full) = registry.complete_state(&s.name, props) else {
        return s;
    };
    let is_default = registry.default_state(&s.name) == Some(&full);
    let properties = match style {
        PaletteStyle::IdString if is_default => Default::default(),
        _ => full,
    };
    BlockState { name: s.name, properties }
}

/// Rewrites a section's block palette through `f`, which returns the new state and, when it
/// replaced the block with something else, a description of what it became. States that end up
/// equal share one palette entry, and every replacement is counted (by block) in `replaced`.
pub(crate) fn remap_blocks(
    c: &mut PalettedContainer<BlockState>,
    mut f: impl FnMut(&BlockState) -> (BlockState, Option<String>),
    replaced: &mut IndexMap<String, ReplacedBlock>,
) {
    let mut uses = vec![0u64; c.palette.len()];
    for &i in &c.indices {
        uses[i as usize] += 1;
    }
    let mut palette: IndexMap<BlockState, u16> = IndexMap::new();
    let mut remap = Vec::with_capacity(c.palette.len());
    for (state, n) in c.palette.iter().zip(&uses) {
        let (new, replaced_with) = f(state);
        if let (Some(to), true) = (replaced_with, *n > 0) {
            replaced
                .entry(state.to_string())
                .and_modify(|r| r.blocks += n)
                .or_insert(ReplacedBlock { to, blocks: *n });
        }
        let next = palette.len() as u16;
        remap.push(*palette.entry(new).or_insert(next));
    }
    *c = PalettedContainer {
        palette: palette.into_keys().collect(),
        indices: c.indices.iter().map(|&i| remap[i as usize]).collect(),
    };
}

/// A chunk from before 1.18 as a 1.18-layout chunk in the modern form, with the entities it held.
fn upgrade_old_chunk(nbt: NamedTag, report: &mut JavaReport) -> Result<(Chunk, Vec<Compound>), Error> {
    let level = nbt.tag.get("Level").and_then(Tag::as_compound).ok_or_else(|| Error::Invalid("chunk without Level".into()))?;
    let compounds = |key: &str| -> Vec<Compound> {
        match level.get(key) {
            Some(Tag::List(l)) => l.iter().filter_map(|t| t.as_compound().cloned()).collect(),
            _ => Vec::new(),
        }
    };
    let (tiles, entities) = (compounds("TileEntities"), compounds("Entities"));
    let at = |k: &str| level.get(k).and_then(Tag::as_i64).unwrap_or(0);
    let (last_update, inhabited) = (at("LastUpdate"), at("InhabitedTime"));
    let biomes = crate::legacy::read_biomes(&nbt);
    let mut chunk = Chunk::from_nbt(nbt)?;
    let numeric = chunk.era == ChunkEra::Legacy;
    let mut rep = crate::java_bedrock::EditionReport::default();
    crate::legacy::modernize_chunk(&mut chunk, biomes.as_ref(), &mut rep);
    let tiles: Vec<Compound> = tiles.iter().map(|b| crate::legacy::upgrade_block_entity(b, numeric, &mut rep)).collect();
    let entities: Vec<Compound> = entities.iter().map(|e| crate::legacy::upgrade_entity(e, numeric, &mut rep)).collect();
    if numeric {
        crate::legacy::colour_beds(&mut chunk, &tiles);
    }
    for (what, n) in rep.unmapped.iter().chain(rep.skipped.iter()) {
        report.notes.add(format!("{what} ({n}x) could not be carried over"));
    }
    let fields = Compound::from([
        ("Status".to_string(), Tag::String("minecraft:full".into())),
        ("LastUpdate".to_string(), Tag::Long(last_update)),
        ("InhabitedTime".to_string(), Tag::Long(inhabited)),
        ("isLightOn".to_string(), Tag::Byte(0)),
        (
            "block_entities".to_string(),
            Tag::List(List::from_tags(TagType::Compound, tiles.into_iter().map(Tag::Compound).collect()).expect("compounds")),
        ),
    ]);
    let sections = std::mem::take(&mut chunk.sections);
    // A modern DataVersion: the data above is in the modern form.
    let modern = Chunk::new_modern(chunk.x, chunk.z, 5023, PaletteStyle::NameProperties, fields, sections);
    Ok((modern, entities))
}

/// What converting one chunk produced.
enum Converted {
    /// The chunk, and (when its entities came from inside the old chunk and the target keeps
    /// them in files) the entity chunk that goes with it.
    Done(NamedTag, Option<NamedTag>),
    /// Older than 1.18; nothing written.
    Unsupported,
}

fn convert_chunk(
    nbt: NamedTag,
    ctx: &Ctx,
    entities: Option<&crate::java_old::EntityMap>,
    report: &mut JavaReport,
) -> Result<Converted, Error> {
    let old = ctx.data_version < crate::java_old::FIRST_NEW_LAYOUT;
    if nbt.tag.get("DataVersion").and_then(Tag::as_i64).is_some_and(|dv| (dv as i32) < ctx.min_dv) {
        report.unsupported_chunks += 1;
        return Ok(Converted::Unsupported);
    }
    // A chunk from before 1.18 is first brought to the 1.18 layout and the modern form; its
    // entities, which it keeps inside it, come out separately.
    let mut inline: Vec<Compound> = Vec::new();
    let mut chunk = if nbt.tag.get("Level").is_some() && nbt.tag.get("sections").is_none() {
        let (c, entities) = upgrade_old_chunk(nbt, report)?;
        inline = entities;
        c
    } else {
        Chunk::from_nbt(nbt)?
    };
    let chunk_dv = chunk.data_version;
    let source = chunk.data_version.and_then(Form::source);
    let Some((from_form, era)) = source.filter(|_| chunk.era == ChunkEra::Modern) else {
        // Older than 1.18.
        report.unsupported_chunks += 1;
        return Ok(Converted::Unsupported);
    };
    chunk.palette_style = ctx.target.palette_style;
    // A target whose world starts at y 0: deal with the part below it.
    let raised = if old && crate::depth::applies(&chunk) { crate::depth::fix(&mut chunk, ctx.depth) } else { 0 };

    for section in &mut chunk.sections {
        if let Some(BlockStorage::Palette(c)) = &mut section.blocks {
            remap_blocks(c, |s| ctx.block(s), &mut report.replaced);
        }
        if let Some(b) = &mut section.biomes {
            let mut cells = vec![0u64; b.palette.len()];
            for &i in &b.indices {
                cells[i as usize] += 1;
            }
            let mut palette: IndexMap<String, u16> = IndexMap::new();
            let mut remap = Vec::with_capacity(b.palette.len());
            for (name, n) in b.palette.iter().zip(cells) {
                let name = &if old { crate::java_old::old_biome_name(name) } else { name.clone() };
                let new = if ctx.target.registry.biomes.contains(name) {
                    name.clone()
                } else {
                    let to = fallback_biome(name, &ctx.target.registry.biomes);
                    *report.biomes_replaced.entry(format!("{name} -> {to}")).or_insert(0) += n;
                    to
                };
                let next = palette.len() as u16;
                remap.push(*palette.entry(new).or_insert(next));
            }
            // Several biomes may now share the fallback, so equal entries are merged.
            *b = PalettedContainer {
                palette: palette.into_keys().collect(),
                indices: b.indices.iter().map(|&i| remap[i as usize]).collect(),
            };
        }
        section.remove_keys(&LIGHT_KEYS);
    }

    let mut out = chunk.to_nbt();
    let top = out.tag.as_compound_mut().expect("chunk root is a compound");
    // Data that was only filtered (same conventions, older version) keeps its own DataVersion, so
    // the game upgrades it when it opens the world; anything else is stamped as the target.
    let stamp = match chunk_dv {
        Some(dv) if from_form == ctx.to_form && !era && dv < ctx.data_version => dv,
        _ => ctx.data_version,
    };
    if stamp == ctx.data_version {
        keep_known_structures(top, &ctx.target.registry);
    }
    top.insert("DataVersion".into(), Tag::Int(stamp));
    top.insert("isLightOn".into(), Tag::Byte(0));
    convert_nbt_lists(top, from_form, era, ctx, report);
    if !ctx.ticks {
        for key in ["block_ticks", "fluid_ticks", "TileTicks", "LiquidTicks"] {
            top.shift_remove(key);
        }
    }
    if raised != 0 {
        if let Some(Tag::List(l)) = top.get_mut("block_entities") {
            for t in l.iter_mut() {
                if let Some(c) = t.as_compound_mut() {
                    crate::depth::raise(c, raised);
                }
            }
        }
    }
    report.chunks += 1;
    // The entities that were inside the source chunk, in the target's form.
    let inline_tags = if inline.is_empty() || !ctx.mobs {
        Vec::new()
    } else {
        let list = List::from_tags(TagType::Compound, inline.into_iter().map(Tag::Compound).collect()).expect("compounds");
        convert_entity_list(&list, Form::Modern, false, ctx, report).iter().cloned().collect()
    };
    if old {
        // 1.13 - 1.17: the chunk is written in the old layout, with its entities inside it.
        let mut own = entities.filter(|_| ctx.mobs).and_then(|m| m.get(&(chunk.x, chunk.z))).cloned().unwrap_or_default();
        own.extend(inline_tags);
        if raised != 0 {
            crate::depth::raise_entities(&mut own, raised);
        }
        let top = out.tag.as_compound().expect("chunk root is a compound");
        let nbt = if crate::java_numeric::is_numeric(ctx.data_version) {
            crate::java_numeric::chunk(&chunk, top, ctx.data_version, own, &mut report.notes)
        } else {
            crate::java_old::old_chunk(&chunk, top, ctx.data_version, own, &mut report.notes)
        };
        return Ok(Converted::Done(nbt, None));
    }
    let entity_chunk = (!inline_tags.is_empty() && ctx.data_version >= crate::java_old::FIRST_ENTITY_FILES).then(|| NamedTag {
        name: String::new(),
        tag: Tag::Compound(Compound::from([
            ("DataVersion".to_string(), Tag::Int(ctx.data_version)),
            ("Position".to_string(), Tag::IntArray(vec![chunk.x, chunk.z])),
            ("Entities".to_string(), Tag::List(List::from_tags(TagType::Compound, inline_tags).expect("compounds"))),
        ])),
    });
    Ok(Converted::Done(out, entity_chunk))
}

/// Drops the structure starts and references the target version does not have (the game logs an
/// error for each and discards them anyway).
fn keep_known_structures(top: &mut mctk_nbt::Compound, target: &Registry) {
    if target.structures.is_empty() {
        return;
    }
    let Some(Tag::Compound(structures)) = top.get_mut("structures") else { return };
    for key in ["starts", "References"] {
        if let Some(Tag::Compound(m)) = structures.get_mut(key) {
            m.retain(|name, _| target.structures.iter().any(|s| s == name));
        }
    }
}

/// A sculk sensor's `listener` in the newer shape (a selector without `range`) is not read by the
/// older versions; the game rebuilds it, so it is left out.
fn drop_new_listener(be: &mut mctk_nbt::Compound) {
    let new_shape = matches!(
        be.get("listener"),
        Some(Tag::Compound(l)) if matches!(l.get("selector"), Some(Tag::Compound(s)) if !s.contains_key("range"))
    );
    if new_shape {
        be.shift_remove("listener");
    }
}

/// The `DataVersion` of 1.18.2, where a spawner's entities got their own wrapper.
const FIRST_ENTITY_WRAPPED_SPAWNER: i32 = 2975;

/// A mob spawner in the shape the target reads: from 1.18.2 `SpawnData` is `{entity: {id}}` and
/// `SpawnPotentials` entries are `{data: {entity}, weight}`; before, the entity stands bare and
/// the entries are `{Entity, Weight}`.
fn spawner_shape(be: &mut Compound, wrapped: bool) {
    if be.get("id").and_then(Tag::as_str) != Some("minecraft:mob_spawner") {
        return;
    }
    if let Some(Tag::Compound(data)) = be.get_mut("SpawnData") {
        match (wrapped, data.contains_key("entity")) {
            (true, false) => *data = Compound::from([("entity".to_string(), Tag::Compound(data.clone()))]),
            (false, true) => {
                if let Some(Tag::Compound(inner)) = data.get("entity").cloned() {
                    *data = inner;
                }
            }
            _ => {}
        }
    }
    if let Some(Tag::List(potentials)) = be.get_mut("SpawnPotentials") {
        for p in potentials.iter_mut().filter_map(Tag::as_compound_mut) {
            if wrapped {
                if let Some(entity) = p.shift_remove("Entity") {
                    p.insert("data".into(), Tag::Compound(Compound::from([("entity".to_string(), entity)])));
                }
                if let Some(w) = p.shift_remove("Weight") {
                    p.insert("weight".into(), w);
                }
            } else {
                if let Some(Tag::Compound(mut d)) = p.shift_remove("data") {
                    if let Some(e) = d.shift_remove("entity") {
                        p.insert("Entity".into(), e);
                    }
                }
                if let Some(w) = p.shift_remove("weight") {
                    p.insert("Weight".into(), w);
                }
            }
        }
    }
}

/// The `DataVersion` of 1.20, where signs got a front and a back side.
const FIRST_SIGN_SIDES: i32 = 3463;

/// A sign of 1.20 and later as the older single-sided one (`Text1` to `Text4`).
fn sign_before_1_20(be: &mut mctk_nbt::Compound, report: &mut JavaReport) {
    let Some(Tag::Compound(front)) = be.shift_remove("front_text") else { return };
    if let Some(Tag::List(lines)) = front.get("messages") {
        for (i, line) in lines.iter().take(4).enumerate() {
            be.insert(format!("Text{}", i + 1), line.clone());
        }
    }
    if let Some(c) = front.get("color") {
        be.insert("Color".into(), c.clone());
    }
    if let Some(g) = front.get("has_glowing_text") {
        be.insert("GlowingText".into(), g.clone());
    }
    if be.shift_remove("back_text").is_some() {
        report.notes.add("the back side of signs does not exist before 1.20");
    }
    be.shift_remove("is_waxed");
}

/// Converts (or, between versions of the same form, just filters) the chunk's block entities
/// and its proto-chunk entity list for the target.
fn convert_nbt_lists(top: &mut mctk_nbt::Compound, from: Form, era: bool, ctx: &Ctx, report: &mut JavaReport) {
    let reg = &ctx.target.registry;
    if !ctx.tiles {
        top.shift_remove("block_entities");
    }
    if !ctx.mobs {
        top.shift_remove("entities");
    }
    if let Some(Tag::List(list)) = top.get_mut("block_entities") {
        let mut kept = Vec::new();
        for t in list.iter() {
            let Some(mut be) = t.as_compound().cloned() else { continue };
            if era {
                crate::java_entities::era_up_block_entity(&mut be);
            }
            let ok = if from == ctx.to_form {
                let id = be.get("id").and_then(Tag::as_str).unwrap_or("");
                let known = reg.block_entities.iter().any(|b| b == id);
                if !known {
                    report.notes.add(format!("block entity {id} does not exist in the target version"));
                }
                known
            } else {
                block_entity_convert(&mut be, ctx.to_form, reg, &mut report.notes)
            };
            if ok {
                if ctx.data_version < FIRST_SIGN_SIDES {
                    sign_before_1_20(&mut be, report);
                }
                if ctx.to_form == Form::Classic {
                    drop_new_listener(&mut be);
                }
                spawner_shape(&mut be, ctx.data_version >= FIRST_ENTITY_WRAPPED_SPAWNER);
                report.block_entities_converted += 1;
                kept.push(Tag::Compound(be));
            }
        }
        *list = List::from_tags(TagType::Compound, kept).expect("compounds");
    }
    if let Some(Tag::List(list)) = top.get_mut("entities") {
        *list = convert_entity_list(list, from, era, ctx, report);
    }
}

fn convert_entity_list(list: &List, from: Form, era: bool, ctx: &Ctx, report: &mut JavaReport) -> List {
    let reg = &ctx.target.registry;
    let mut kept = Vec::new();
    for t in list.iter() {
        let Some(mut e) = t.as_compound().cloned() else { continue };
        if era {
            crate::java_entities::era_up_entity(&mut e, &mut report.notes);
        }
        let ok = if from == ctx.to_form {
            let id = e.get("id").and_then(Tag::as_str).unwrap_or("");
            let known = reg.entities.iter().any(|x| x == id);
            if !known {
                report.notes.add(format!("entity {id} does not exist in the target version"));
            }
            known
        } else {
            entity_convert(&mut e, ctx.to_form, reg, &mut report.notes)
        };
        let ok = ok && (!crate::java_numeric::is_numeric(ctx.data_version) || crate::java_numeric::entity(&mut e, crate::java_numeric::era(ctx.data_version), &mut report.notes));
        if ok {
            report.entities_converted += 1;
            kept.push(Tag::Compound(e));
        }
    }
    List::from_tags(TagType::Compound, kept).expect("compounds")
}

/// One chunk of an `entities/` region file.
fn convert_entity_chunk(mut nbt: NamedTag, ctx: &Ctx, report: &mut JavaReport) -> Option<NamedTag> {
    let top = nbt.tag.as_compound_mut()?;
    let from = top.get("DataVersion").and_then(Tag::as_i64).and_then(|v| Form::source(v as i32));
    let Some((from, era)) = from else {
        report.notes.add("entity chunk of an unknown version");
        return None;
    };
    if let Some(Tag::List(list)) = top.get("Entities") {
        let new = convert_entity_list(list, from, era, ctx, report);
        top.insert("Entities".into(), Tag::List(new));
    }
    top.insert("DataVersion".into(), Tag::Int(ctx.data_version));
    Some(nbt)
}

#[derive(Default)]
struct Slot {
    nbt: Option<NamedTag>,
    timestamp: u32,
    out: Option<NamedTag>,
    entity_out: Option<NamedTag>,
}

/// One region file's chunks, converted in parallel through the shared pipeline.
struct RegionJob<'a> {
    entities: Option<&'a crate::java_old::EntityMap>,
    region: &'a Region,
    chunks: Vec<(usize, usize)>,
    ctx: &'a Ctx<'a>,
    skip_partial: bool,
    progress: &'a Progress,
    report: Mutex<JavaReport>,
}

impl ChunkStages for RegionJob<'_> {
    type Slot = Slot;
    type Output = (usize, usize, RawChunk, Option<RawChunk>);

    fn load(&self, index: usize, slot: &mut Slot) -> Result<Flow, Error> {
        let (lx, lz) = self.chunks[index];
        let Some(raw) = self.region.raw_chunk(lx, lz)? else {
            return Ok(Flow::Skip);
        };
        slot.timestamp = raw.timestamp;
        slot.nbt = Some(raw.nbt()?);
        Ok(Flow::Continue)
    }

    fn parse(&self, _: usize, slot: &mut Slot) -> Result<Flow, Error> {
        let status = slot
            .nbt
            .as_ref()
            .and_then(|n| n.tag.get("Status"))
            .and_then(Tag::as_str);
        if self.skip_partial && !is_complete(status) {
            return Ok(Flow::Skip);
        }
        Ok(Flow::Continue)
    }

    fn convert(&self, _: usize, slot: &mut Slot) -> Result<Flow, Error> {
        let nbt = slot.nbt.take().expect("loaded");
        let mut local = JavaReport::default();
        let result = convert_chunk(nbt, self.ctx, self.entities, &mut local);
        self.report.lock().expect("report lock").merge(local);
        match result {
            Ok(Converted::Done(out, entity_chunk)) => {
                slot.out = Some(out);
                slot.entity_out = entity_chunk;
                Ok(Flow::Continue)
            }
            Ok(Converted::Unsupported) => Ok(Flow::Skip),
            Err(Error::Java(_)) => Ok(Flow::Invalid),
            Err(e) => Err(e),
        }
    }

    fn clean(&self, index: usize, slot: &mut Slot) -> Option<Self::Output> {
        let out = slot.out.take()?;
        let (lx, lz) = self.chunks[index];
        let raw = RawChunk::encode(&out, ChunkCompression::Zlib, slot.timestamp).ok()?;
        let entity_raw = slot
            .entity_out
            .take()
            .and_then(|e| RawChunk::encode(&e, ChunkCompression::Zlib, slot.timestamp).ok());
        let pos = self.region.position()?.chunk(lx, lz);
        self.progress.chunk(pos.x, pos.z);
        Some((lx, lz, raw, entity_raw))
    }
}

fn dimension_dir(dst: &Path, id: &str, modern: bool) -> PathBuf {
    match id {
        "minecraft:overworld" if !modern => dst.to_path_buf(),
        "minecraft:the_nether" if !modern => dst.join("DIM-1"),
        "minecraft:the_end" if !modern => dst.join("DIM1"),
        other => {
            let (ns, name) = other.split_once(':').unwrap_or(("minecraft", other));
            dst.join("dimensions").join(ns).join(name)
        }
    }
}

fn dimension_number(id: &str) -> i32 {
    match id {
        "minecraft:the_nether" => -1,
        "minecraft:the_end" => 1,
        _ => 0,
    }
}

fn write_gzip_nbt(path: &Path, tag: &NamedTag) -> Result<(), Error> {
    use std::io::Write;
    let raw = mctk_nbt::to_bytes(tag, Flavor::Java)?;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(&raw)?;
    std::fs::write(path, e.finish()?)?;
    Ok(())
}

/// Reads the level settings of a Java world in either layout.
fn read_level(src: &Path) -> Result<LevelData, Error> {
    if src.join("data/minecraft/world_gen_settings.dat").is_file() {
        LevelData::read_modern(src)
    } else {
        let (_, root) = mctk_nbt::read_file(src.join("level.dat"), Flavor::Java)?;
        LevelData::read_classic(&NamedTag { name: String::new(), tag: root.tag })
    }
}

/// Converts every `entities/r.x.z.mca` of a dimension and gathers the entities by chunk.
fn collect_entities(src: &Path, ctx: &Ctx, report: &mut JavaReport) -> crate::java_old::EntityMap {
    let mut map = crate::java_old::EntityMap::new();
    let Ok(entries) = std::fs::read_dir(src) else { return map };
    for entry in entries.flatten() {
        let Some(_) = entry.file_name().to_str().and_then(mctk_anvil::RegionPos::from_file_name) else { continue };
        let Ok(region) = Region::open(entry.path()) else { continue };
        for (lx, lz) in region.chunks() {
            let Ok(Some(raw)) = region.raw_chunk(lx, lz) else { continue };
            let Ok(nbt) = raw.nbt() else { continue };
            let Some(converted) = convert_entity_chunk(nbt, ctx, report) else { continue };
            let Some(top) = converted.tag.as_compound() else { continue };
            let (Some(Tag::IntArray(pos)), Some(Tag::List(list))) = (top.get("Position"), top.get("Entities")) else { continue };
            if pos.len() == 2 {
                map.entry((pos[0], pos[1])).or_default().extend(list.iter().cloned());
            }
        }
    }
    map
}

/// Converts every `entities/r.x.z.mca` of a dimension into `out`.
fn convert_entity_regions(src: &Path, out: &Path, ctx: &Ctx, report: &mut JavaReport) -> Result<(), Error> {
    let Ok(entries) = std::fs::read_dir(src) else {
        return Ok(());
    };
    std::fs::create_dir_all(out)?;
    for entry in entries.flatten() {
        let Some(rpos) = entry.file_name().to_str().and_then(mctk_anvil::RegionPos::from_file_name) else {
            continue;
        };
        let region = Region::open(entry.path())?;
        let mut builder = RegionBuilder::new();
        for (lx, lz) in region.chunks() {
            let Some(raw) = region.raw_chunk(lx, lz)? else { continue };
            let Some(converted) = convert_entity_chunk(raw.nbt()?, ctx, report) else { continue };
            builder.set(lx, lz, Some(RawChunk::encode(&converted, ChunkCompression::Zlib, raw.timestamp)?));
        }
        builder.write(out, rpos)?;
    }
    Ok(())
}

fn copy_tree(src: &Path, dst: &Path) -> Result<usize, Error> {
    let mut n = 0;
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            n += copy_tree(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to)?;
            n += 1;
        }
    }
    Ok(n)
}

fn count_files(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |it| {
        it.flatten().map(|e| if e.path().is_dir() { count_files(&e.path()) } else { 1 }).sum()
    })
}

/// The saved-data files that sit beside the chunks: scoreboard, maps, raids, world border, dragon
/// fight, command storage. Between worlds of the same layout they are copied as they are. The two
/// layouts name and shape them differently, so between layouts they are counted in the notes
/// instead of being written wrong.
fn copy_saved_data(
    src: &Path,
    dst: &Path,
    world: &JavaWorld,
    src_modern: bool,
    dst_modern: bool,
    options: &ConvertOptions,
    data_version: i32,
    report: &mut JavaReport,
) -> Result<(), Error> {
    // (folder in the source, folder in the output)
    let mut pairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    if src_modern {
        pairs.push((src.join("data"), dst.join("data")));
    }
    for dim in &world.dimensions {
        if options.includes(dimension_number(&dim.id)) {
            pairs.push((dim.root.join("data"), dimension_dir(dst, &dim.id, dst_modern).join("data")));
        }
    }
    if src_modern == dst_modern {
        for (from, to) in pairs {
            if from.is_dir() {
                report.saved_data_files += copy_tree(&from, &to)? as u64;
            }
        }
        return Ok(());
    }
    // Between the layouts: scoreboard, command storage, maps and raids are moved (up) or
    // converted (down); the world border, boss bars, dragon fight and trader come with the level
    // data. Anything else in the folders (e.g. random sequences) is left to the game to recreate.
    let included: Vec<&str> = world
        .dimensions
        .iter()
        .filter(|d| options.includes(dimension_number(&d.id)))
        .map(|d| d.id.as_str())
        .collect();
    report.saved_data_files += if dst_modern {
        crate::java_saved::up(src, dst, &included)?
    } else {
        crate::java_saved::down(src, dst, &included, data_version, &mut report.notes)?
    };
    let _ = count_files;
    Ok(())
}

/// Converts the Java world `src` into a new world folder `dst` for `target`.
///
/// `dst` must not already hold a world. The output layout follows `target` (classic 1.18 – 1.21
/// or modern 26.x).
pub fn java_to_java(
    src: &Path,
    dst: &Path,
    target: &JavaTarget,
    map: &ReplacementMap,
    options: &ConvertOptions,
    progress: &Progress,
) -> Result<JavaReport, Error> {
    if dst.join("level.dat").exists() {
        return Err(Error::Invalid(format!(
            "{} already holds a world; choose an empty folder",
            dst.display()
        )));
    }
    let data_version = target.data_version()?;
    let to_form = target.form;
    let world = JavaWorld::open(src)?;
    let world_dv = world.level.data_version.unwrap_or(0);
    let ctx = Ctx { target, map, data_version, to_form, tiles: options.tile_entities, mobs: options.entities, ticks: options.ticks, depth: options.depth, min_dv: if options.older_chunks { 0 } else { world_dv } };

    let mut level = read_level(src)?;
    if let Some(g) = options.game_mode {
        level.game_type = g;
    }
    if let Some(d) = options.difficulty {
        level.difficulty = d as i8;
    }

    let mut report = JavaReport::default();
    for dim in &world.dimensions {
        if !options.includes(dimension_number(&dim.id)) {
            continue;
        }
        progress.set_dimension(&dim.id);
        // Before 1.17 entities are part of their chunk, so they are converted first.
        let entity_map = (data_version < crate::java_old::FIRST_ENTITY_FILES)
            .then(|| if options.entities { collect_entities(&dim.entities_dir(), &ctx, &mut report) } else { Default::default() });
        let out_dir = dimension_dir(dst, &dim.id, target.is_modern()).join("region");
        std::fs::create_dir_all(&out_dir)?;
        let regions = dim.regions()?;
        progress.regions_total.fetch_add(regions.len(), std::sync::atomic::Ordering::Relaxed);
        for rpos in regions {
            let region = Region::open(dim.region_dir().join(rpos.file_name()))?;
            let chunks: Vec<(usize, usize)> = region.chunks().collect();
            progress
                .chunks_total
                .fetch_add(chunks.len(), std::sync::atomic::Ordering::Relaxed);
            let job = RegionJob {
                entities: entity_map.as_ref(),
                region: &region,
                chunks,
                ctx: &ctx,
                skip_partial: options.skip_partial_chunks,
                progress,
                report: Mutex::new(JavaReport::default()),
            };
            let results = run_pipeline(&job, job.chunks.len(), 0, progress)?;
            let mut builder = RegionBuilder::new();
            let mut entity_builder = RegionBuilder::new();
            let mut any_entities = false;
            for (lx, lz, raw, entity_raw) in results.into_iter().flatten() {
                builder.set(lx, lz, Some(raw));
                if let Some(e) = entity_raw {
                    entity_builder.set(lx, lz, Some(e));
                    any_entities = true;
                }
            }
            builder.write(&out_dir, rpos)?;
            if any_entities {
                let dir = dimension_dir(dst, &dim.id, target.is_modern()).join("entities");
                std::fs::create_dir_all(&dir)?;
                entity_builder.write(&dir, rpos)?;
            }
            report.merge(job.report.into_inner().expect("report lock"));
            progress.regions_done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if options.entities && data_version >= crate::java_old::FIRST_ENTITY_FILES {
            let out_entities = dimension_dir(dst, &dim.id, target.is_modern()).join("entities");
            convert_entity_regions(&dim.entities_dir(), &out_entities, &ctx, &mut report)?;
        }
        if dim.poi_dir().is_dir() {
            report.notes.add("point-of-interest data is not converted (the game rebuilds it)");
        }
    }

    std::fs::create_dir_all(dst)?;

    // Players: their files, advancements and statistics, and the host of a single-player world.
    let src_modern = src.join("data/minecraft/world_gen_settings.dat").is_file();
    let level_dv = mctk_nbt::read_file(src.join("level.dat"), Flavor::Java)
        .ok()
        .and_then(|(_, r)| r.tag.get("Data")?.get("DataVersion")?.as_i64())
        .unwrap_or(data_version as i64) as i32;
    let host = match (&level.singleplayer_uuid, &level.embedded_player) {
        (Some(u), Some(p)) => Some((u, p)),
        _ => None,
    };
    let players = crate::java_player::read_players(src, src_modern, host, level_dv)?;
    // Players are stamped with the layout's own DataVersion, like everything else.
    let mut player_registry = target.registry.clone();
    player_registry.data_version = Some(data_version);
    let embedded = crate::java_player::write_players(
        dst,
        &players,
        target.is_modern(),
        target.form,
        &player_registry,
        level.singleplayer_uuid.as_ref(),
        &mut report.notes,
    )?;
    level.embedded_player = if target.is_modern() { None } else { embedded };
    report.players = players.len() as u64;

    copy_saved_data(src, dst, &world, src_modern, target.is_modern(), options, data_version, &mut report)?;

    let version_name = target.stamp.1.clone();
    if target.is_modern() {
        level.write_modern(dst, data_version, &version_name)?;
    } else {
        write_gzip_nbt(&dst.join("level.dat"), &level.to_classic(data_version, &version_name))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mctk_registry::{BlockDef, Edition};

    fn def(props: &[(&str, &[&str])]) -> BlockDef {
        BlockDef {
            properties: props
                .iter()
                .map(|(k, vs)| (k.to_string(), vs.iter().map(|v| v.to_string()).collect()))
                .collect(),
            defaults: props.iter().map(|(k, vs)| (k.to_string(), vs[0].to_string())).collect(),
        }
    }

    fn target() -> JavaTarget {
        let mut r = Registry::new(Edition::Java, "test");
        r.data_version = Some(3465);
        r.biomes.push("minecraft:plains".into());
        r.block_entities.push("minecraft:sign".into());
        for (n, d) in [
            ("minecraft:air", def(&[])),
            ("minecraft:stone", def(&[])),
            ("minecraft:grass", def(&[])),
            ("minecraft:oak_log", def(&[("axis", &["y", "x", "z"])])),
        ] {
            r.blocks.insert(n.into(), d);
        }
        JavaTarget::classic(r)
    }

    #[test]
    fn missing_biomes_become_the_closest_known_one() {
        let known: Vec<String> = ["minecraft:plains", "minecraft:forest", "minecraft:taiga", "minecraft:birch_forest"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(fallback_biome("minecraft:dappled_forest", &known), "minecraft:forest");
        assert_eq!(fallback_biome("minecraft:old_growth_pine_taiga", &known), "minecraft:taiga");
        assert_eq!(fallback_biome("minecraft:old_growth_birch_forest", &known), "minecraft:birch_forest");
        // Nothing comparable: plains.
        assert_eq!(fallback_biome("minecraft:deep_dark", &known), "minecraft:plains");
    }

    #[test]
    fn finalize_completes_states_for_classic_and_strips_for_modern() {
        let t = target();
        let map = ReplacementMap::default();
        let ctx = Ctx { target: &t, map: &map, data_version: 3465, to_form: Form::Classic, tiles: true, mobs: true, ticks: true, depth: Default::default(), min_dv: 0 };
        // 26.x leaves out a default `axis=y`; classic storage needs it spelled out.
        let (s, rep) = ctx.block(&BlockState::new("minecraft:oak_log"));
        assert_eq!(s.properties["axis"], "y");
        assert_eq!(rep, None);

        let mut modern = t.clone();
        modern.palette_style = PaletteStyle::IdString;
        let ctx = Ctx { target: &modern, map: &map, data_version: 1, to_form: Form::Modern, tiles: true, mobs: true, ticks: true, depth: Default::default(), min_dv: 0 };
        // The default state is written as the bare id...
        let (s, _) = ctx.block(&BlockState::new("minecraft:oak_log").with("axis", "y"));
        assert!(s.properties.is_empty());
        // ...any other state spells out every property.
        let (s, _) = ctx.block(&BlockState::new("minecraft:oak_log").with("axis", "x"));
        assert_eq!(s.properties["axis"], "x");
    }

    #[test]
    fn replaced_blocks_merge_into_one_palette_entry_and_are_reported() {
        let t = target();
        let map = ReplacementMap::default();
        let ctx = Ctx { target: &t, map: &map, data_version: 3465, to_form: Form::Classic, tiles: true, mobs: true, ticks: true, depth: Default::default(), min_dv: 0 };

        // Palette: air, short_grass (renamed to grass), grass (already target name).
        let mut container = PalettedContainer::filled(BlockState::air(), 4096);
        container.palette.push(BlockState::new("minecraft:short_grass"));
        container.palette.push(BlockState::new("minecraft:grass"));
        container.indices[0] = 1;
        container.indices[1] = 1;
        container.indices[2] = 2;
        let section = mctk_java::Section::new(0, Some(BlockStorage::Palette(container)), None);
        let chunk = Chunk::new_modern(
            0,
            0,
            5023,
            PaletteStyle::IdString,
            mctk_nbt::Compound::new(),
            vec![section],
        );
        let mut report = JavaReport::default();
        let Converted::Done(out, _) = convert_chunk(chunk.to_nbt(), &ctx, None, &mut report).unwrap() else {
            panic!("chunk should convert")
        };
        assert_eq!(report.replaced["minecraft:short_grass"].blocks, 2);
        assert_eq!(report.replaced["minecraft:short_grass"].to, "minecraft:grass");
        assert_eq!(out.tag.get("DataVersion").and_then(Tag::as_i64), Some(3465));
        assert_eq!(out.tag.get("isLightOn").and_then(Tag::as_i64), Some(0));

        let back = Chunk::from_nbt(out).unwrap();
        let Some(BlockStorage::Palette(p)) = &back.sections[0].blocks else { panic!() };
        // short_grass and grass collapsed into one palette entry: air + grass.
        assert_eq!(p.palette.len(), 2);
        assert_eq!(p.indices[0], p.indices[2]);
        assert_eq!(back.palette_style, PaletteStyle::NameProperties);
    }
}
