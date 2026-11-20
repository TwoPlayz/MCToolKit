//! The world viewer's 3D drawn on the GPU with wgpu: the chunks around the camera are read as
//! real blocks (caves, overhangs and the insides of buildings included), made into meshes of
//! the block faces that touch air on worker threads, kept on the GPU, and drawn with a depth
//! buffer each frame. Nothing is built or sent again while the camera only moves, and nothing
//! needs sorting.
//!
//! The map view falls back to its own drawing (see `street_view`) when the app is not drawing
//! with wgpu.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use eframe::egui::{self, Pos2, Rect};
use eframe::egui_wgpu::{self, wgpu};
use wgpu::util::DeviceExt;

use crate::street_view::{Atlas, Camera, Face, Form, Hit, Slot, form_of, liquid_height};
use crate::world::{self, Edition};

/// How many chunks around the camera are drawn as blocks (the user can choose more).
pub const RADIUS: i32 = 32;

// ---------------------------------------------------------------------------------------------
// Setting up wgpu
// ---------------------------------------------------------------------------------------------

/// eframe's wgpu settings: any adapter will do, a real GPU before the software one (Windows'
/// WARP), which the default choice would pass over.
pub fn wgpu_options() -> egui_wgpu::WgpuConfiguration {
    let mut options = egui_wgpu::WgpuConfiguration::default();
    if let egui_wgpu::WgpuSetup::CreateNew(setup) = &mut options.wgpu_setup {
        setup.instance_descriptor.backends = wgpu::Backends::VULKAN | wgpu::Backends::DX12 | wgpu::Backends::METAL;
        setup.native_adapter_selector = Some(Arc::new(|adapters: &[wgpu::Adapter], surface: Option<&wgpu::Surface<'_>>| {
            let rank = |a: &wgpu::Adapter| match a.get_info().device_type {
                wgpu::DeviceType::DiscreteGpu => 0,
                wgpu::DeviceType::IntegratedGpu => 1,
                wgpu::DeviceType::VirtualGpu => 2,
                wgpu::DeviceType::Other => 3,
                wgpu::DeviceType::Cpu => 4,
            };
            // The same card through Direct3D 12 before Vulkan on Windows (minimizing the window
            // is handled better there).
            let backend = |a: &wgpu::Adapter| u8::from(!(cfg!(windows) && a.get_info().backend == wgpu::Backend::Dx12));
            adapters
                .iter()
                .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
                .min_by_key(|a| (rank(a), backend(a)))
                .cloned()
                .ok_or_else(|| format!("none of the {} graphics adapters can draw to the window", adapters.len()))
        }));
    }
    options
}

/// Marks in egui's memory that the app draws with wgpu, so the map view uses the GPU world.
#[derive(Clone)]
pub struct GpuReady;

/// Sets the GPU world up when the app draws with wgpu: its pipelines and block texture, kept
/// with egui's renderer for the paint callbacks.
pub fn install(cc: &eframe::CreationContext) {
    let Some(rs) = cc.wgpu_render_state.as_ref() else { return };
    let info = rs.adapter.get_info();
    eprintln!("drawing with wgpu: {} ({:?}, {:?})", info.name, info.backend, info.device_type);
    // A GPU error (one frame drawn wrong: a window shrunk to nothing, a size the driver does
    // not take) is written down, not the end of the app (wgpu's own handler panics).
    rs.device.on_uncaptured_error(Box::new(|e| {
        eprintln!("GPU error (frame skipped): {e}");
        crate::crash_note(&format!("GPU error (frame skipped): {e}"));
    }));
    let res = Res::new(&rs.device, rs.target_format);
    rs.renderer.write().callback_resources.insert(res);
    cc.egui_ctx.data_mut(|d| d.insert_temp(egui::Id::new("gpu-world"), GpuReady));
}

/// Whether the app draws with wgpu (and the GPU world can be used).
pub fn ready(ctx: &egui::Context) -> Option<GpuReady> {
    ctx.data(|d| d.get_temp::<GpuReady>(egui::Id::new("gpu-world")))
}

// ---------------------------------------------------------------------------------------------
// The time of day
// ---------------------------------------------------------------------------------------------

/// The time of day the world was saved at (game ticks, 0 sunrise, 6000 noon, 18000 midnight),
/// from its level.dat; `None` when it does not say.
pub fn world_time(path: &std::path::Path, edition: Edition) -> Option<i64> {
    let day = |t: &mctk_nbt::Tag| t.as_i64();
    match edition {
        Edition::Java => {
            let (_, root) = mctk_nbt::read_file(path.join("level.dat"), mctk_nbt::Flavor::Java).ok()?;
            day(root.tag.get("Data")?.get("DayTime")?)
        }
        Edition::Bedrock => {
            let bytes = std::fs::read(path.join("level.dat")).ok()?;
            let level = mctk_bedrock::LevelDat::parse(&bytes).ok()?;
            day(level.root.tag.get("Time")?)
        }
        Edition::Console => {
            let save = mctk_console::ConsoleSave::open(path).ok()?;
            let entry = save.archive.get("level.dat")?;
            let root = mctk_nbt::from_bytes(&entry.data, mctk_nbt::Flavor::Java).ok()?;
            day(root.tag.get("Data")?.get("DayTime")?)
        }
    }
}

/// How bright the sky's light is at a time of day (1 by day, about a quarter at night, as the
/// game dims it), easing through dusk and dawn.
pub fn daylight(time: i64) -> f32 {
    let t = time.rem_euclid(24000) as f32;
    let night = 4.0 / 15.0;
    let ease = |a: f32, b: f32, x: f32| a + (b - a) * (x * x * (3.0 - 2.0 * x));
    match t {
        t if t < 12000.0 => 1.0,
        t if t < 13800.0 => ease(1.0, night, (t - 12000.0) / 1800.0),
        t if t < 22200.0 => night,
        t => ease(night, 1.0, (t - 22200.0) / 1800.0),
    }
}

// ---------------------------------------------------------------------------------------------
// Reading chunks
// ---------------------------------------------------------------------------------------------

/// A chunk's blocks as the map's block numbers (see `world::block_name`; 0 is air), section by
/// section from `min_section` up, each 16 x 16 x 16 in y, z, x order.
pub struct Blocks {
    min_section: i32,
    sections: Vec<Option<Box<[u16]>>>,
    /// The saved data of its block entities (banner patterns, pot sherds...), by the block's
    /// place in the chunk (x, y, z).
    extras: HashMap<(u8, i32, u8), Arc<mctk_nbt::Tag>>,
    /// The entities saved in it (item frames, armor stands, items, paintings, mobs...).
    entities: Vec<Arc<mctk_nbt::Tag>>,
    /// Where its barrier blocks are (world places): never drawn as blocks, shown by their
    /// icon as the game does (see `World::barriers_near`).
    barriers: Vec<[i32; 3]>,
}

/// The entities in a list of them (an `Entities` tag), those within the Slicer's heights.
/// Whether a ray from `eye` along `dir` (ahead of the eye) passes through the box `lo`..`hi`.
fn ray_hits_box(eye: [f32; 3], dir: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> bool {
    let (mut near, mut far) = (0.0f32, f32::INFINITY);
    for k in 0..3 {
        if dir[k].abs() < 1e-9 {
            if eye[k] < lo[k] || eye[k] > hi[k] {
                return false;
            }
            continue;
        }
        let (a, b) = ((lo[k] - eye[k]) / dir[k], (hi[k] - eye[k]) / dir[k]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    near <= far
}

fn entity_list(list: Option<&mctk_nbt::Tag>, keep: &dyn Fn(i32) -> bool) -> Vec<Arc<mctk_nbt::Tag>> {
    let Some(list) = list.and_then(mctk_nbt::Tag::as_list) else { return Vec::new() };
    list.iter()
        .filter(|e| {
            let y = e.get("Pos").and_then(mctk_nbt::Tag::as_list).and_then(|p| p.get(1)).and_then(mctk_nbt::Tag::as_f64);
            y.is_none_or(|y| keep(y.floor() as i32))
        })
        .map(|e| Arc::new(e.clone()))
        .collect()
}

/// A Java chunk's block entities' data, by place in the chunk.
fn block_entities(root: &mctk_nbt::Tag) -> HashMap<(u8, i32, u8), Arc<mctk_nbt::Tag>> {
    let list = root.get("block_entities").or_else(|| root.get("Level")?.get("TileEntities")).and_then(mctk_nbt::Tag::as_list);
    let mut out = HashMap::new();
    for e in list.into_iter().flat_map(|l| l.iter()) {
        let n = |k: &str| e.get(k).and_then(mctk_nbt::Tag::as_i64);
        if let (Some(x), Some(y), Some(z)) = (n("x"), n("y"), n("z")) {
            out.insert((x.rem_euclid(16) as u8, y as i32, z.rem_euclid(16) as u8), Arc::new(e.clone()));
        }
    }
    out
}

impl Blocks {
    fn get(&self, x: usize, y: i32, z: usize) -> u16 {
        let s = y.div_euclid(16) - self.min_section;
        if s < 0 || s as usize >= self.sections.len() {
            return 0;
        }
        match &self.sections[s as usize] {
            Some(b) => b[(y.rem_euclid(16) as usize) * 256 + z * 16 + x],
            None => 0,
        }
    }
}

/// Where chunks are read from, opened once.
enum Source {
    Java {
        dir: PathBuf,
        regions: Mutex<HashMap<(i32, i32), Option<Arc<mctk_anvil::Region>>>>,
        /// The entities' own region files (1.17 and later keep them apart from the blocks).
        ent_dir: PathBuf,
        ent_regions: Mutex<HashMap<(i32, i32), Option<Arc<mctk_anvil::Region>>>>,
    },
    Bedrock { world: mctk_bedrock::BedrockWorld, dim: mctk_bedrock::Dimension },
    /// `dim` is the console dimension number (0, -1 the Nether, 1 the End).
    Console { entries: HashMap<(i32, i32), Vec<u8>>, regions: Mutex<HashMap<(i32, i32), Option<Arc<mctk_console::ConsoleRegion>>>>, dim: i32 },
}

fn open_source(path: &std::path::Path, edition: Edition, dimension: &str) -> Result<Source, String> {
    match edition {
        Edition::Java => {
            let w = mctk_java::JavaWorld::open(path).map_err(|e| e.to_string())?;
            let dim = w.dimension(dimension).ok_or("no such dimension")?;
            Ok(Source::Java { dir: dim.region_dir(), regions: Mutex::new(HashMap::new()), ent_dir: dim.entities_dir(), ent_regions: Mutex::new(HashMap::new()) })
        }
        Edition::Bedrock => {
            let world = mctk_bedrock::BedrockWorld::open(path).map_err(|e| e.to_string())?;
            let dim = match dimension {
                "minecraft:the_nether" => mctk_bedrock::Dimension::Nether,
                "minecraft:the_end" => mctk_bedrock::Dimension::End,
                _ => mctk_bedrock::Dimension::Overworld,
            };
            Ok(Source::Bedrock { world, dim })
        }
        Edition::Console => {
            let save = mctk_console::ConsoleSave::open(path).map_err(|e| e.to_string())?;
            let want = match dimension {
                "minecraft:the_nether" => -1,
                "minecraft:the_end" => 1,
                _ => 0,
            };
            let entries = save
                .archive
                .regions()
                .into_iter()
                .filter(|(d, _, _, e)| *d == want && !e.data.is_empty())
                .map(|(_, rx, rz, e)| ((rx, rz), e.data.clone()))
                .collect();
            Ok(Source::Console { entries, regions: Mutex::new(HashMap::new()), dim: want })
        }
    }
}

thread_local! {
    /// Block names already turned into numbers on this thread.
    static IDS: std::cell::RefCell<HashMap<String, u16>> = Default::default();
}

/// The map's number for a block name (0 for air).
fn id_of(name: &str) -> u16 {
    id_of_state(name, "")
}

/// The map's number for a block name in a full state (`props`, see `world::intern_state`).
fn id_of_state(name: &str, props: &str) -> u16 {
    let n = name.trim_start_matches("minecraft:");
    // (Barriers are kept: never drawn as blocks, but shown by their icon and pickable.)
    if matches!(n, "air" | "cave_air" | "void_air" | "structure_void" | "light") {
        return 0;
    }
    IDS.with(|c| {
        let key = if props.is_empty() { n.to_string() } else { format!("{n}[{props}]") };
        if let Some(i) = c.borrow().get(&key) {
            return *i;
        }
        let id = world::intern_state(n, props);
        c.borrow_mut().insert(key, id);
        id
    })
}

/// The map's number for a Java block state, with its biome for grass and leaves (0 for air).
fn id_of_java(b: &mctk_java::BlockState, biome: Option<&str>) -> u16 {
    if b.is_air() {
        return 0;
    }
    id_of_state(&world::java_map_name(b, biome), &state_props(b))
}

/// The map's number for a numeric block id and data value (console worlds, Java before 1.13).
fn id_of_legacy(id: u16, data: u8) -> u16 {
    thread_local! {
        static LEGACY: std::cell::RefCell<HashMap<(u16, u8), u16>> = Default::default();
    }
    if let Some(n) = LEGACY.with(|c| c.borrow().get(&(id, data)).copied()) {
        return n;
    }
    let n = id_of_java(&world::legacy_java_state(id, data), None);
    LEGACY.with(|c| c.borrow_mut().insert((id, data), n));
    n
}

/// A Java chunk's blocks (and those of a console chunk once made a Java one), with its block
/// entities and entities; only blocks at heights `keep` allows.
fn java_blocks(root: mctk_nbt::NamedTag, extras: HashMap<(u8, i32, u8), Arc<mctk_nbt::Tag>>, entities: Vec<Arc<mctk_nbt::Tag>>, keep: &dyn Fn(i32) -> bool) -> Option<Blocks> {
    let chunk = mctk_java::Chunk::from_nbt_read_only(root).ok()?;
    let min = chunk.sections.iter().map(|s| s.y as i32).min()?;
    let max = chunk.sections.iter().map(|s| s.y as i32).max()?;
    let mut sections: Vec<Option<Box<[u16]>>> = vec![None; (max - min + 1) as usize];
    for s in &chunk.sections {
        let sy = s.y as i32;
        let ids: Option<Box<[u16]>> = match &s.blocks {
            Some(mctk_java::BlockStorage::Palette(p)) => {
                if p.palette.iter().all(|b| b.is_air()) {
                    None
                } else {
                    // Each palette entry named once; grass and leaves take their biome,
                    // so those are named per biome cell.
                    let plain: Vec<u16> = p.palette.iter().map(|b| if b.is_air() { 0 } else { id_of_state(&world::java_map_name(b, None), &state_props(b)) }).collect();
                    let mut by_biome: HashMap<(u16, usize), u16> = HashMap::new();
                    let mut out = vec![0u16; 4096];
                    for (i, slot) in out.iter_mut().enumerate() {
                        let k = p.indices[i] as usize;
                        let id = plain[k];
                        if id == 0 {
                            continue;
                        }
                        let y = i / 256;
                        if !keep(sy * 16 + y as i32) {
                            continue;
                        }
                        let named = world::java_map_name(&p.palette[k], Some("x"));
                        *slot = if named.contains('@') {
                            let (x, z) = (i % 16, (i / 16) % 16);
                            let cell = (y / 4) * 16 + (z / 4) * 4 + x / 4;
                            *by_biome.entry((k as u16, cell)).or_insert_with(|| {
                                let biome = s.biomes.as_ref().map(|bm| bm.get(cell).as_str());
                                id_of_state(&world::java_map_name(&p.palette[k], biome), &state_props(&p.palette[k]))
                            })
                        } else {
                            id
                        };
                    }
                    Some(out.into_boxed_slice())
                }
            }
            Some(mctk_java::BlockStorage::Legacy { ids, data }) => {
                let mut out = vec![0u16; 4096];
                for (i, slot) in out.iter_mut().enumerate() {
                    if ids[i] != 0 && keep(sy * 16 + (i / 256) as i32) {
                        *slot = id_of_legacy(ids[i], data.get(i).copied().unwrap_or(0));
                    }
                }
                Some(out.into_boxed_slice())
            }
            None => None,
        };
        sections[(sy - min) as usize] = ids;
    }
    Some(Blocks { min_section: min, sections, extras, entities, barriers: Vec::new() })
}

/// Whether an entity stands at a height `keep` allows (one with no position is kept).
fn entity_kept(e: &mctk_nbt::Tag, keep: &dyn Fn(i32) -> bool) -> bool {
    let y = e.get("Pos").and_then(mctk_nbt::Tag::as_list).and_then(|p| p.get(1)).and_then(mctk_nbt::Tag::as_f64);
    y.is_none_or(|y| keep(y.floor() as i32))
}

/// The NBT compounds stored one after another in a Bedrock record (block entities, old entities).
fn bedrock_compounds(raw: Option<&[u8]>) -> Vec<mctk_nbt::Compound> {
    let mut out = Vec::new();
    let mut s = raw.unwrap_or(&[]);
    while !s.is_empty() {
        let Ok(root) = mctk_nbt::read_root(&mut s, mctk_nbt::Flavor::Bedrock) else { break };
        if let mctk_nbt::Tag::Compound(c) = root.tag {
            out.push(c);
        }
    }
    out
}

/// One Bedrock subchunk as Java sees it: the Java states of its palette, each block's palette
/// slot (in this file's y, z, x order), whether each block stands in water (empty when none
/// does) and, for palette slots that are item frames, the Bedrock frame block.
struct BedrockSection {
    states: Vec<mctk_java::BlockState>,
    idx: Vec<u16>,
    wet: Vec<bool>,
    frames: Vec<Option<(String, mctk_nbt::Compound)>>,
}

impl BedrockSection {
    /// From Java states already in Java order (old Bedrock chunks, which the converter reads).
    fn from_java(c: mctk_java::PalettedContainer<mctk_java::BlockState>) -> Self {
        let frames = vec![None; c.palette.len()];
        BedrockSection { states: c.palette, idx: c.indices, wet: Vec::new(), frames }
    }
}

/// A kelp or vine block that more of the plant grows on from (up for kelp and twisting vines,
/// down for weeping and cave vines) as Java has it: its stem (`kelp_plant`...), which Bedrock
/// does not tell apart from the tip.
fn stem_of<'a>(state: &mctk_java::BlockState, at: impl Fn(i32) -> Option<&'a mctk_java::BlockState>) -> Option<mctk_java::BlockState> {
    let (family, dy) = match state.name.as_str() {
        "minecraft:kelp" => ("minecraft:kelp", 1),
        "minecraft:twisting_vines" => ("minecraft:twisting_vines", 1),
        "minecraft:weeping_vines" => ("minecraft:weeping_vines", -1),
        "minecraft:cave_vines" => ("minecraft:cave_vines", -1),
        _ => return None,
    };
    let next = at(dy)?;
    if next.name != family && next.name != format!("{family}_plant") {
        return None;
    }
    let mut stem = mctk_java::BlockState::new(format!("{family}_plant"));
    if let Some(b) = state.properties.get("berries") {
        stem.properties.insert("berries".into(), b.clone());
    }
    Some(stem)
}

/// Reads a Bedrock chunk: its blocks (under their Java names and states), its block entities
/// and the changes they make to their blocks (bed colors, chest halves, skulls, pots), its
/// entities and its item frames (blocks in Bedrock, entities in Java).
fn bedrock_blocks<'a>(get: &dyn Fn(&[u8]) -> Option<&'a [u8]>, dim: mctk_bedrock::Dimension, cx: i32, cz: i32, keep: &dyn Fn(i32) -> bool) -> Option<Blocks> {
    use mctk_bedrock::{ChunkKey, ChunkTag, SubChunkBody};
    use mctk_nbt::Tag;
    let key = |tag: ChunkTag, subchunk: Option<i8>| ChunkKey { x: cx, z: cz, dimension: dim, tag, subchunk }.encode();
    let (lo, hi) = if dim == mctk_bedrock::Dimension::Overworld { (-4, 19) } else { (0, 15) };
    let mut secs: Vec<Option<BedrockSection>> = (lo..=hi).map(|_| None).collect();
    let mut any = false;
    for sy in lo..=hi {
        let Some(raw) = get(&key(ChunkTag::SubChunkPrefix, Some(sy as i8))) else { continue };
        any = true;
        let Ok(sub) = mctk_bedrock::SubChunk::parse(raw) else { continue };
        let sec = match sub.body {
            SubChunkBody::Palettized(storages) => {
                let Some(st) = storages.first() else { continue };
                let mut idx = vec![0u16; 4096];
                for (i, slot) in idx.iter_mut().enumerate() {
                    *slot = st.indices.get(mctk_bedrock::block_index(i % 16, i / 256, (i / 16) % 16)).copied().unwrap_or(0);
                }
                // A second layer holds the water a block stands in (Java's `waterlogged`).
                let wet = storages.get(1).map_or_else(Vec::new, |w| {
                    let water: Vec<bool> = w.palette.iter().map(|p| matches!(p.tag.get("name").and_then(Tag::as_str), Some("minecraft:water" | "minecraft:flowing_water"))).collect();
                    if !water.iter().any(|f| *f) {
                        return Vec::new();
                    }
                    (0..4096)
                        .map(|i| w.indices.get(mctk_bedrock::block_index(i % 16, i / 256, (i / 16) % 16)).is_some_and(|k| water.get(*k as usize).copied().unwrap_or(false)))
                        .collect()
                });
                let frames = st
                    .palette
                    .iter()
                    .map(|p| mctk_mapping::BedrockBlock::from_nbt(p).filter(|b| mctk_convert::view::is_frame_block(&b.name)).map(|b| (b.name, b.states)))
                    .collect();
                // Bedrock's names and states differ from Java's (`deadbush`, `tallgrass`,
                // `pillar_axis`...): each palette entry goes through the converter's table.
                let states = st.palette.iter().map(world::bedrock_java_state).collect();
                BedrockSection { states, idx, wet, frames }
            }
            SubChunkBody::Legacy(body) => BedrockSection::from_java(mctk_convert::view::bedrock_legacy_subchunk(&body)),
        };
        secs[(sy - lo) as usize] = Some(sec);
    }
    // Chunks from before subchunks keep all their blocks in one record.
    if !any {
        if let Some(raw) = get(&key(ChunkTag::LegacyTerrain, None)) {
            for (sy, c) in mctk_convert::view::bedrock_legacy_terrain(raw) {
                if let Some(slot) = secs.get_mut((sy as i32 - lo) as usize) {
                    *slot = Some(BedrockSection::from_java(c));
                    any = true;
                }
            }
        }
    }
    if !any {
        return None;
    }
    let state_at = |x: usize, y: i32, z: usize| -> Option<&mctk_java::BlockState> {
        let s = secs.get((y.div_euclid(16) - lo) as usize)?.as_ref()?;
        s.states.get(*s.idx.get(y.rem_euclid(16) as usize * 256 + z * 16 + x)? as usize)
    };

    // Block entities, and the blocks they change.
    let m = mctk_mapping::mappings();
    let mut extras = HashMap::new();
    let mut changed: HashMap<(usize, i32, usize), mctk_java::BlockState> = HashMap::new();
    let mut frame_items: HashMap<(i32, i32, i32), mctk_nbt::Compound> = HashMap::new();
    for be in bedrock_compounds(get(&key(ChunkTag::BlockEntity, None))) {
        let n = |k: &str| be.get(k).and_then(Tag::as_i64);
        let (Some(bx), Some(by), Some(bz)) = (n("x"), n("y"), n("z")) else { continue };
        let (lx, y, lz) = (bx.rem_euclid(16) as usize, by as i32, bz.rem_euclid(16) as usize);
        if !keep(y) {
            continue;
        }
        if matches!(be.get("id").and_then(Tag::as_str), Some("ItemFrame" | "GlowItemFrame")) {
            frame_items.insert((bx as i32, y, bz as i32), be);
            continue;
        }
        let block = state_at(lx, y, lz).map(|s| m.complete_java(s));
        let Some(mut j) = mctk_convert::view::bedrock_block_entity(&be, block.as_ref()) else { continue };
        if let Some(nbt) = j.nbt.take() {
            extras.insert((lx as u8, y, lz as u8), Arc::new(Tag::Compound(nbt)));
        }
        // Only to a block Java has (a plant may have no potted form).
        if j.rename.as_ref().is_some_and(|r| !m.is_known_java(&mctk_java::BlockState::new(r.clone()))) {
            j.rename = None;
        }
        if let (Some(mut s), true) = (block, j.rename.is_some() || !j.props.is_empty()) {
            j.apply(&mut s);
            changed.insert((lx, y, lz), m.strip_java_defaults(&s));
        }
    }

    // Entities: listed per chunk (`digp`) and stored apart since 1.18.30, in the chunk before.
    let mut actors = bedrock_compounds(get(&key(ChunkTag::Entity, None)));
    if let Some(list) = get(&mctk_bedrock::Key::ActorDigest { x: cx, z: cz, dimension: dim }.encode()) {
        for uid in list.chunks_exact(8) {
            let Some(raw) = get(&mctk_bedrock::Key::Actor(uid.to_vec()).encode()) else { continue };
            if let Ok(mctk_nbt::NamedTag { tag: Tag::Compound(c), .. }) = mctk_nbt::from_bytes(raw, mctk_nbt::Flavor::Bedrock) {
                actors.push(c);
            }
        }
    }
    let mut entities: Vec<Arc<Tag>> = actors
        .iter()
        .filter_map(mctk_convert::view::bedrock_actor)
        .map(Tag::Compound)
        .filter(|e| entity_kept(e, keep))
        .map(Arc::new)
        .collect();

    // Biomes (for grass and leaf colors), one per block.
    let d3 = get(&key(ChunkTag::Data3D, None)).and_then(|raw| mctk_bedrock::Data3D::parse(raw).ok());
    let biome_floor = lo * 16;
    let mut sections: Vec<Option<Box<[u16]>>> = vec![None; secs.len()];
    for (si, sec) in secs.iter().enumerate() {
        let Some(sec) = sec else { continue };
        let sy = lo + si as i32;
        let mut ids: HashMap<(u16, bool, Option<&str>), u16> = HashMap::new();
        let mut out = vec![0u16; 4096];
        for (i, slot) in out.iter_mut().enumerate() {
            let (x, y, z) = (i % 16, (i / 256) as i32, (i / 16) % 16);
            let wy = sy * 16 + y;
            if !keep(wy) {
                continue;
            }
            let k = sec.idx[i];
            if let Some((name, states)) = sec.frames.get(k as usize).and_then(Option::as_ref) {
                let tile = (cx * 16 + x as i32, wy, cz * 16 + z as i32);
                let frame = mctk_convert::view::bedrock_frame(name, states, tile, frame_items.get(&tile));
                entities.push(Arc::new(Tag::Compound(frame)));
                continue;
            }
            if let Some(s) = changed.get(&(x, wy, z)) {
                *slot = id_of_java(s, None);
                continue;
            }
            let Some(state) = sec.states.get(k as usize) else { continue };
            if state.is_air() {
                continue;
            }
            if let Some(s) = stem_of(state, |dy| state_at(x, wy + dy, z)) {
                *slot = id_of_java(&s, None);
                continue;
            }
            let soaked = sec.wet.get(i).copied().unwrap_or(false) && state.properties.get("waterlogged").is_some_and(|v| v == "false");
            let biome = if world::java_map_name(state, Some("x")).contains('@') {
                d3.as_ref().and_then(|d| world::bedrock_biome_name(d, biome_floor, x, z, wy))
            } else {
                None
            };
            *slot = *ids.entry((k, soaked, biome)).or_insert_with(|| {
                let mut s = state.clone();
                if soaked {
                    s.properties.insert("waterlogged".into(), "true".into());
                }
                id_of_java(&s, biome)
            });
        }
        sections[si] = Some(out.into_boxed_slice());
    }
    Some(Blocks { min_section: lo, sections, extras, entities, barriers: Vec::new() })
}

/// The most blocks the paste preview draws (a bigger copy shows as its box only).
const GHOST_MOST: usize = 300_000;

/// The blocks a paste would put down, by the chunk they would be in: the clipboard placed with
/// its lowest north-west corner at `origin`, turned and mirrored. None for a copy too big to
/// show.
pub fn ghost_chunks(clip: &crate::edit::tools::Clipboard, origin: [i32; 3], place: crate::edit::transform::Placement) -> Vec<(Key, Blocks)> {
    if clip.solid() > GHOST_MOST {
        return Vec::new();
    }
    let ids: Vec<u16> = clip.palette.iter().map(|s| if s.is_air() { 0 } else { id_of_java(&place.state(s), None) }).collect();
    let mut out: HashMap<Key, Blocks> = HashMap::new();
    for y in 0..clip.size[1] {
        for z in 0..clip.size[2] {
            for x in 0..clip.size[0] {
                let id = ids[clip.blocks[((y * clip.size[2] + z) * clip.size[0] + x) as usize] as usize];
                if id == 0 {
                    continue;
                }
                let t = place.pos(clip.size, [x, y, z]);
                let (wx, wy, wz) = (origin[0] + t[0], origin[1] + t[1], origin[2] + t[2]);
                let key = (wx.div_euclid(16), wz.div_euclid(16));
                let b = out.entry(key).or_insert_with(|| Blocks { min_section: wy.div_euclid(16), sections: Vec::new(), extras: HashMap::new(), entities: Vec::new(), barriers: Vec::new() });
                let sy = wy.div_euclid(16);
                // Sections from the lowest to the highest used.
                if sy < b.min_section {
                    let more = (b.min_section - sy) as usize;
                    let mut s: Vec<Option<Box<[u16]>>> = vec![None; more];
                    s.append(&mut b.sections);
                    b.sections = s;
                    b.min_section = sy;
                }
                let i = (sy - b.min_section) as usize;
                if i >= b.sections.len() {
                    b.sections.resize(i + 1, None);
                }
                let sec = b.sections[i].get_or_insert_with(|| vec![0u16; 4096].into_boxed_slice());
                sec[wy.rem_euclid(16) as usize * 256 + wz.rem_euclid(16) as usize * 16 + wx.rem_euclid(16) as usize] = id;
            }
        }
    }
    out.into_iter().collect()
}

/// The paste preview's chunks made into meshes (each with its neighbors in the preview, so the
/// faces between them are left out), all drawn see-through.
fn ghost_meshes(chunks: &[(Key, Arc<Blocks>)], atlas: &Mutex<Atlas>) -> Vec<(Key, MeshData)> {
    let by: HashMap<Key, Arc<Blocks>> = chunks.iter().cloned().collect();
    chunks
        .iter()
        .map(|(k, b)| {
            let near = [(-1, 0), (1, 0), (0, -1), (0, 1)].map(|(dx, dz)| by.get(&(k.0 + dx, k.1 + dz)).cloned());
            let mut m = mesh(k.0, k.1, b, &near, atlas);
            let mut all = std::mem::take(&mut m.solid);
            all.append(&mut m.water);
            m.water = all;
            (*k, m)
        })
        .collect()
}

/// Something the search found: what it is, where, how far from where the search was made, and
/// (for an entity) the entity as the 3D view has it.
#[derive(Clone, Debug)]
pub struct Found {
    pub name: String,
    pub pos: [f64; 3],
    pub dist: f32,
    pub entity: Option<Arc<mctk_nbt::Tag>>,
}

/// The most results a search keeps (the nearest).
const FOUND_MOST: usize = 500;

/// Searches the world around `center` (within `radius` blocks) for blocks and/or entities whose
/// name or id holds `query` (a Display Name too, for entities), reading every chunk in range
/// (nearest first) through the editing tools' changes; with `center`'s height NaN, only the
/// distance across counts. `progress` counts the chunks done of the
/// total it is set to; `stop` ends it early. The nearest results first.
#[allow(clippy::too_many_arguments)]
pub fn search(
    path: &std::path::Path,
    edition: Edition,
    dim: &str,
    edits: &std::sync::RwLock<crate::edit::EditLayer>,
    center: [f32; 3],
    radius: f32,
    query: &str,
    blocks: bool,
    entities: bool,
    progress: &std::sync::atomic::AtomicUsize,
    total: &std::sync::atomic::AtomicUsize,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<Vec<Found>, String> {
    use std::sync::atomic::Ordering;
    let source = open_source(path, edition, dim)?;
    let q = query.trim().to_lowercase().replace(' ', "_");
    // "minecraft:<id>" asks for exactly that block or entity; other text for any whose name holds it.
    let exact = q.starts_with("minecraft:");
    let q = q.trim_start_matches("minecraft:").to_string();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let hits = |name: &str| if exact { name == q } else { name.contains(&q) };
    let (ccx, ccz) = ((center[0] / 16.0).floor() as i32, (center[2] / 16.0).floor() as i32);
    let reach = (radius / 16.0).ceil() as i32 + 1;
    let mut chunks: Vec<(i32, Key)> = Vec::new();
    for dz in -reach..=reach {
        for dx in -reach..=reach {
            // Only chunks some part of which is within the radius (across).
            let (x0, z0) = ((ccx + dx) as f32 * 16.0, (ccz + dz) as f32 * 16.0);
            let nx = center[0].clamp(x0, x0 + 16.0) - center[0];
            let nz = center[2].clamp(z0, z0 + 16.0) - center[2];
            if nx * nx + nz * nz <= radius * radius {
                chunks.push((dx * dx + dz * dz, (ccx + dx, ccz + dz)));
            }
        }
    }
    chunks.sort_unstable();
    total.store(chunks.len(), Ordering::Relaxed);
    let mut out: Vec<Found> = Vec::new();
    let mut matches: HashMap<u16, Option<String>> = HashMap::new();
    for (_, (cx, cz)) in chunks {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let b = {
            let layer = edits.read().map_err(|_| "the edits are locked")?;
            load(&source, cx, cz, None, &layer, dim)
        };
        progress.fetch_add(1, Ordering::Relaxed);
        let Some(b) = b else { continue };
        // (A height that isn't a number: only the distance across counts, as on the flat map.)
        let near = |p: [f32; 3]| {
            let dy = if center[1].is_nan() { 0.0 } else { p[1] - center[1] };
            let d = ((p[0] - center[0]).powi(2) + dy * dy + (p[2] - center[2]).powi(2)).sqrt();
            (d <= radius).then_some(d)
        };
        if blocks {
            for (si, sec) in b.sections.iter().enumerate() {
                let Some(sec) = sec else { continue };
                let sy = (b.min_section + si as i32) * 16;
                for (i, &id) in sec.iter().enumerate() {
                    if id == 0 {
                        continue;
                    }
                    let name = matches.entry(id).or_insert_with(|| {
                        let n = world::block_name(id)?;
                        let bare = n.split(['#', '@']).next().unwrap_or(&n).trim_start_matches("minecraft:").to_string();
                        hits(&bare).then_some(bare)
                    });
                    let Some(name) = name else { continue };
                    let (x, y, z) = (cx * 16 + (i % 16) as i32, sy + (i / 256) as i32, cz * 16 + ((i / 16) % 16) as i32);
                    if let Some(d) = near([x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5]) {
                        out.push(Found { name: crate::item_picker::display_name(name), pos: [x as f64, y as f64, z as f64], dist: d, entity: None });
                    }
                }
            }
        }
        if entities {
            for e in &b.entities {
                let Some(c) = e.as_compound() else { continue };
                let id = mctk_convert::view::java_entity_id(c).unwrap_or_default();
                let custom = crate::naming::mob_name(c);
                let hit = hits(id.trim_start_matches("minecraft:")) || (!exact && custom.as_ref().is_some_and(|n| n.to_lowercase().replace(' ', "_").contains(&q)));
                let Some(p) = entity_pos(e).filter(|_| hit) else { continue };
                if let Some(d) = near(p) {
                    let kind = crate::item_picker::display_name(&id);
                    let name = match custom {
                        Some(n) => format!("{n} ({kind})"),
                        None => kind,
                    };
                    out.push(Found { name, pos: [p[0] as f64, p[1] as f64, p[2] as f64], dist: d, entity: Some(e.clone()) });
                }
            }
        }
        if out.len() > FOUND_MOST * 8 {
            out.sort_by(|a, b| a.dist.total_cmp(&b.dist));
            out.truncate(FOUND_MOST);
        }
    }
    out.sort_by(|a, b| a.dist.total_cmp(&b.dist));
    out.truncate(FOUND_MOST);
    Ok(out)
}

/// For tests: where the entities in a chunk stand, as the 3D view shows them.
#[cfg(test)]
pub(crate) fn entity_places(path: &std::path::Path, edition: Edition, edits: &crate::edit::EditLayer, cx: i32, cz: i32) -> Vec<[f64; 3]> {
    let dim = "minecraft:overworld";
    let Ok(source) = open_source(path, edition, dim) else { return Vec::new() };
    let Some(b) = load(&source, cx, cz, None, edits, dim) else { return Vec::new() };
    b.entities.iter().filter_map(|e| entity_pos(e)).map(|p| [p[0] as f64, p[1] as f64, p[2] as f64]).collect()
}

/// For tests: what the 3D view reads in a box of a world (through `edits`): its solid blocks,
/// the entities standing in it, and how many of its chunks exist.
#[cfg(test)]
pub(crate) fn read_box(path: &std::path::Path, edition: Edition, edits: &crate::edit::EditLayer, lo: [i32; 3], hi: [i32; 3]) -> (usize, usize, usize) {
    let dim = "minecraft:overworld";
    let Ok(source) = open_source(path, edition, dim) else { return (0, 0, 0) };
    let (mut solid, mut ents, mut chunks) = (0, 0, 0);
    for cz in lo[2].div_euclid(16)..=hi[2].div_euclid(16) {
        for cx in lo[0].div_euclid(16)..=hi[0].div_euclid(16) {
            let Some(b) = load(&source, cx, cz, None, edits, dim) else { continue };
            chunks += 1;
            for y in lo[1]..=hi[1] {
                for z in lo[2].max(cz * 16)..=hi[2].min(cz * 16 + 15) {
                    for x in lo[0].max(cx * 16)..=hi[0].min(cx * 16 + 15) {
                        solid += usize::from(b.get((x - cx * 16) as usize, y, (z - cz * 16) as usize) != 0);
                    }
                }
            }
            ents += b.entities.iter().filter(|e| entity_pos(e).is_some_and(|p| (0..3).all(|i| p[i].floor() as i32 >= lo[i] && p[i].floor() as i32 <= hi[i]))).count();
        }
    }
    (solid, ents, chunks)
}

/// Reads one chunk's blocks; with `slicer` only those between its heights. What the World
/// Explorer changed (`edits`, in dimension `dim`) is read instead of what is saved.
fn load(source: &Source, cx: i32, cz: i32, slicer: Option<(i32, i32)>, edits: &crate::edit::EditLayer, dim: &str) -> Option<Blocks> {
    let mut b = load_raw(source, cx, cz, slicer, edits, dim)?;
    // Where its barriers are, found once here (they are drawn by their icon, not meshed).
    for (si, sec) in b.sections.iter().enumerate() {
        let Some(sec) = sec else { continue };
        let sy = (b.min_section + si as i32) * 16;
        for (i, &id) in sec.iter().enumerate() {
            if id != 0 && is_barrier(id) {
                b.barriers.push([cx * 16 + (i % 16) as i32, sy + (i / 256) as i32, cz * 16 + ((i / 16) % 16) as i32]);
            }
        }
    }
    Some(b)
}

/// Whether a block number is a barrier (in any state), worked out once each.
pub(crate) fn is_barrier(id: u16) -> bool {
    thread_local! {
        static SEEN: std::cell::RefCell<HashMap<u16, bool>> = Default::default();
    }
    SEEN.with(|s| {
        *s.borrow_mut().entry(id).or_insert_with(|| world::block_name(id).is_some_and(|n| n.trim_start_matches("minecraft:").split(['#', '@', '[']).next() == Some("barrier")))
    })
}

fn load_raw(source: &Source, cx: i32, cz: i32, slicer: Option<(i32, i32)>, edits: &crate::edit::EditLayer, dim: &str) -> Option<Blocks> {
    let keep = |y: i32| slicer.is_none_or(|(lo, hi)| y >= lo && y <= hi);
    let id = (dim.to_string(), cx, cz);
    match source {
        Source::Java { dir, regions, ent_dir, ent_regions } => {
            let (rx, rz) = (cx.div_euclid(32), cz.div_euclid(32));
            let edited = edits.java.get(&id);
            let root = match edited {
                Some(e) => e.root.clone()?,
                None => {
                    let region = {
                        let mut r = regions.lock().ok()?;
                        r.entry((rx, rz))
                            .or_insert_with(|| {
                                let pos = mctk_anvil::RegionPos { x: rx, z: rz };
                                mctk_anvil::Region::open(dir.join(pos.file_name())).ok().map(Arc::new)
                            })
                            .clone()?
                    };
                    region.read_chunk(cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize).ok()??
                }
            };
            let extras = block_entities(&root.tag);
            // Entities: in the chunk itself (before 1.17), else in their own region file (or as
            // edited).
            let mut entities = entity_list(root.tag.get("Level").and_then(|l| l.get("Entities")), &keep);
            if let Some(Some(record)) = edited.and_then(|e| e.entities.as_ref()).filter(|_| entities.is_empty()) {
                entities = entity_list(record.tag.get("Entities"), &keep);
            } else if entities.is_empty() && edited.is_none_or(|e| e.entities.is_none()) {
                let ent_region = {
                    let mut r = ent_regions.lock().ok()?;
                    r.entry((rx, rz))
                        .or_insert_with(|| {
                            let pos = mctk_anvil::RegionPos { x: rx, z: rz };
                            mctk_anvil::Region::open(ent_dir.join(pos.file_name())).ok().map(Arc::new)
                        })
                        .clone()
                };
                if let Some(er) = ent_region {
                    if let Ok(Some(e)) = er.read_chunk(cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize) {
                        entities = entity_list(e.tag.get("Entities"), &keep);
                    }
                }
            }
            java_blocks(root, extras, entities, &keep)
        }
        Source::Bedrock { world, dim } => {
            let get = |k: &[u8]| -> Option<&[u8]> {
                match edits.bedrock.get(k) {
                    Some(v) => v.as_deref(),
                    None => world.db.get(k),
                }
            };
            bedrock_blocks(&get, *dim, cx, cz, &keep)
        }
        Source::Console { entries, regions, dim } => {
            let root = match edits.console.get(&id) {
                Some(r) => r.clone()?,
                None => {
                    let (rx, rz) = (cx.div_euclid(32), cz.div_euclid(32));
                    let region = {
                        let mut r = regions.lock().ok()?;
                        r.entry((rx, rz))
                            .or_insert_with(|| entries.get(&(rx, rz)).and_then(|d| mctk_console::ConsoleRegion::parse(d.clone()).ok()).map(Arc::new))
                            .clone()?
                    };
                    let raw = region.raw_chunk(cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize).ok()??;
                    raw.nbt().ok()?
                }
            };
            let chunk = mctk_console::ConsoleChunk::from_nbt(root).ok()?;
            // Made a current Java chunk by the converter's own code: block states from their
            // neighbors (doors, fences, stairs, chests), skulls, pots and beds from their block
            // entities, and the block entities and entities upgraded.
            let (root, list) = mctk_convert::view::console_chunk(&chunk, *dim)?;
            let extras = block_entities(&root.tag);
            let entities = list.into_iter().filter(|e| entity_kept(e, &keep)).map(Arc::new).collect();
            java_blocks(root, extras, entities, &keep)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// What each block is drawn as
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Air,
    /// A full-width block this tall.
    Cube(f32),
    /// Water (see-through) or lava.
    Liquid { water: bool },
    /// Two crossed pictures (plants).
    Cross,
    /// A narrow upright piece: half its width, its height.
    Post(f32, f32),
    /// Lying flat on the block below (rails).
    Decal,
}

#[derive(Clone, Copy)]
struct Info {
    shape: Shape,
    /// Hides the faces of the blocks beside it (a whole, opaque block); it also stops light.
    solid: bool,
    /// Faces between two blocks of the same group are hidden (water against water, glass
    /// against glass); 0 for none.
    group: u16,
    /// Its bottom shows dirt (grass, podzol...).
    dirt_bottom: bool,
    /// Lying along x (1) or z (2), with `ends` the block as drawn standing (for its end faces).
    axis: u8,
    ends: u16,
    /// Light passing through it fades a step more (water, leaves).
    dims: bool,
    /// The light it gives (0 to 15), as the game's glowing blocks.
    glow: u8,
    /// Pointed dripstone: drawn by its place in a stack (tip, frustum, middle, base).
    drip: bool,
    /// A plant growing in water (seagrass, kelp): drawn as water with the plant standing in it.
    sea_plant: bool,
    /// Drawn from the game's own model for its state (see `models`).
    model: bool,
    /// Waterlogged: water fills the rest of its block.
    wet: bool,
    /// Drawn in code, as the game draws it (chests, banners, heads...; see `entities`).
    entity: bool,
}

/// The game's models for a block's state, by map number (once worked out).
fn baked_of(id: u16) -> Option<Arc<crate::models::Baked>> {
    static BY_ID: std::sync::LazyLock<Mutex<HashMap<u16, Option<Arc<crate::models::Baked>>>>> = std::sync::LazyLock::new(Default::default);
    if let Some(b) = BY_ID.lock().ok()?.get(&id) {
        return b.clone();
    }
    let made = world::block_name(id).and_then(|name| {
        let base = name.split(['#', '@']).next().unwrap_or(&name).to_string();
        let props = world::block_props(id);
        crate::models::baked(&base, props.as_deref().unwrap_or(""))
    });
    BY_ID.lock().ok()?.insert(id, made.clone());
    made
}

/// What a block is drawn as: the game's model for its state where there is one (with how much
/// of its block it fills, for hiding its neighbors' faces and stopping light), else a shape
/// guessed from its name.
fn info_of(id: u16) -> Info {
    let mut i = info_guess(id);
    if matches!(i.shape, Shape::Air | Shape::Liquid { .. }) {
        return i;
    }
    let base = world::block_name(id).map(|n| n.split(['#', '@']).next().unwrap_or(&n).to_string()).unwrap_or_default();
    if crate::entities::is_block_entity(&base) {
        i.entity = true;
        i.solid = false;
        i.wet = world::block_props(id).is_some_and(|p| p.split(',').any(|x| x == "waterlogged=true"));
        return i;
    }
    if let Some(b) = baked_of(id) {
        i.model = true;
        i.drip = false;
        i.solid &= b.full;
        let props = world::block_props(id);
        let has = |kv: &str| props.as_deref().is_some_and(|p| p.split(',').any(|x| x == kv));
        i.wet = has("waterlogged=true");
        i.dims |= i.wet;
        // Lit or not, by its state (furnaces, lamps, campfires, candles...).
        if has("lit=false") {
            i.glow = 0;
        } else if has("lit=true") && i.glow == 0 {
            let lamp = world::block_name(id).is_some_and(|n| n.contains("lamp"));
            i.glow = if lamp { 15 } else { 13 };
        }
    }
    i
}

/// Blocks' [`Info`] by map number, worked out once each (a list by number: looked up a dozen
/// times a face, a hash map was a good part of meshing).
#[derive(Default)]
struct Infos(Vec<Option<Info>>);

impl Infos {
    fn get(&mut self, id: u16) -> Info {
        let i = id as usize;
        if i >= self.0.len() {
            self.0.resize(i + 1, None);
        }
        *self.0[i].get_or_insert_with(|| info_of(id))
    }
}

/// A block state's properties as the map keeps them: `key=value` pairs joined by commas.
fn state_props(b: &mctk_java::BlockState) -> String {
    let mut out = String::new();
    for (k, v) in &b.properties {
        if !out.is_empty() {
            out.push(',');
        }
        out.push_str(k);
        out.push('=');
        out.push_str(v);
    }
    out
}

/// The light a block gives, as in the game (0 for none).
fn glow_of(base: &str) -> u8 {
    match base {
        "lava" | "flowing_lava" | "glowstone" | "sea_lantern" | "jack_o_lantern" | "lantern" | "campfire" | "fire" | "shroomlight" | "beacon" | "conduit"
        | "end_portal" | "ochre_froglight" | "verdant_froglight" | "pearlescent_froglight" | "lit_pumpkin" => 15,
        "torch" | "wall_torch" | "end_rod" => 14,
        "soul_torch" | "soul_wall_torch" | "soul_lantern" | "soul_campfire" | "soul_fire" | "crying_obsidian" => 10,
        "redstone_torch" | "redstone_wall_torch" | "glow_lichen" | "sculk_catalyst" => 7,
        "magma_block" | "candle" | "brewing_stand" => 3,
        _ if base.ends_with("_candle") => 3,
        _ => 0,
    }
}

fn info_guess(id: u16) -> Info {
    let air = Info { shape: Shape::Air, solid: false, group: 0, dirt_bottom: false, axis: 0, ends: id, dims: false, glow: 0, drip: false, sea_plant: false, model: false, wet: false, entity: false };
    if id == 0 {
        return air;
    }
    let Some(name) = world::block_name(id) else { return air };
    let plain = name.split('@').next().unwrap_or(&name).to_string();
    let (base, suffix) = plain.split_once('#').map_or((plain.as_str(), None), |(b, s)| (b, Some(s)));
    let mut info = Info { shape: Shape::Cube(1.0), solid: true, group: 0, dirt_bottom: false, axis: 0, ends: id, dims: false, glow: glow_of(base), drip: false, sea_plant: false, model: false, wet: false, entity: false };
    if base == "pointed_dripstone" {
        return Info { shape: Shape::Cross, solid: false, drip: true, ..info };
    }
    // Barriers are not drawn as blocks (their icon shows them), hide nothing and let light by.
    if base == "barrier" {
        return air;
    }
    info.dirt_bottom = matches!(base, "grass_block" | "podzol" | "mycelium" | "dirt_path" | "farmland");
    match suffix {
        Some("x") => info.axis = 1,
        Some("z") => info.axis = 2,
        _ => {}
    }
    if info.axis != 0 {
        info.ends = world::intern(base);
    }
    if matches!(base, "water" | "flowing_water" | "bubble_column") || world::is_water(base) {
        let sea_plant = !matches!(base, "water" | "flowing_water" | "bubble_column");
        return Info { shape: Shape::Liquid { water: true }, solid: false, group: 1, dims: true, sea_plant, ..info };
    }
    if matches!(base, "lava" | "flowing_lava") {
        return Info { shape: Shape::Liquid { water: false }, solid: false, group: 2, ..info };
    }
    // Leaves have gaps: every face is drawn, the ones behind seen through the gaps.
    if base.ends_with("_leaves") || base == "leaves" {
        return Info { solid: false, dims: true, ..info };
    }
    if world::is_plant(base) {
        return Info { shape: Shape::Cross, solid: false, ..info };
    }
    match form_of(&name) {
        Form::Cross => return Info { shape: Shape::Cross, solid: false, ..info },
        Form::Post(w, h) => return Info { shape: Shape::Post(w, h), solid: false, ..info },
        Form::Decal => return Info { shape: Shape::Decal, solid: false, ..info },
        Form::Cube(h) if h < 1.0 => return Info { shape: Shape::Cube(h), solid: false, ..info },
        _ => {}
    }
    // See-through whole blocks hide only their own kind.
    if base.contains("glass") || matches!(base, "ice" | "frosted_ice" | "slime_block" | "honey_block" | "spawner") {
        return Info { solid: false, group: id, ..info };
    }
    info
}

// ---------------------------------------------------------------------------------------------
// Light
// ---------------------------------------------------------------------------------------------

/// The light in each cell of a chunk (0 to 15): the brighter of the sky's and the glowing
/// blocks'. The sky lights everything above the land and comes down undimmed through air,
/// spreading sideways a step dimmer at a time (into overhangs and cave mouths); glowing blocks
/// light around them the same way. Worked out over the chunk and a border of `LIGHT_PAD` blocks
/// of the chunks beside it, so light coming in across a side (under a tree just over the edge,
/// into an overhang) is right where faces meet the next chunk.
struct LightGrid {
    min_y: i32,
    h: i32,
    /// The sky's light and the glowing blocks' (kept apart: the time of day dims only the sky).
    sky: Vec<u8>,
    lamp: Vec<u8>,
}

/// How far into the chunks beside it a chunk's light is worked out.
const LIGHT_PAD: i32 = 6;
/// The light grid's width: the chunk and its border.
const LIGHT_W: i32 = 16 + 2 * LIGHT_PAD;

impl LightGrid {
    /// The sky's light and the glowing blocks' in a cell.
    fn at(&self, x: i32, y: i32, z: i32) -> (u8, u8) {
        if y >= self.min_y + self.h {
            return (15, 0);
        }
        if y < self.min_y {
            return (0, 0);
        }
        let (gx, gz) = ((x + LIGHT_PAD).clamp(0, LIGHT_W - 1), (z + LIGHT_PAD).clamp(0, LIGHT_W - 1));
        let i = ((y - self.min_y) * LIGHT_W * LIGHT_W + gz * LIGHT_W + gx) as usize;
        (self.sky[i], self.lamp[i])
    }
}


fn light_grid(me: &Blocks, near: &[Option<Arc<Blocks>>; 4], info: &dyn Fn(u16) -> Info) -> LightGrid {
    let min_y = me.min_section * 16;
    let h = me.sections.len() as i32 * 16;
    let (w, layer) = (LIGHT_W, LIGHT_W * LIGHT_W);
    let n = (h * layer) as usize;
    // Grid cells are numbered from the border's corner (chunk x, z from -LIGHT_PAD).
    let idx = |x: i32, y: i32, z: i32| ((y - min_y) * layer + (z + LIGHT_PAD) * w + x + LIGHT_PAD) as usize;
    // The column of blocks at a spot in the grid: this chunk's, or the one beside it on that
    // side (taken as this chunk's own edge where there is none: missing, or a corner).
    let column = |x: i32, z: i32| -> (&Blocks, usize, usize) {
        let own = (me, x.clamp(0, 15) as usize, z.clamp(0, 15) as usize);
        let side = match (x, z) {
            (0..=15, 0..=15) => return own,
            (..0, 0..=15) => near[0].as_deref().map(|b| (b, (x + 16) as usize, z as usize)),
            (16.., 0..=15) => near[1].as_deref().map(|b| (b, (x - 16) as usize, z as usize)),
            (0..=15, ..0) => near[2].as_deref().map(|b| (b, x as usize, (z + 16) as usize)),
            (0..=15, 16..) => near[3].as_deref().map(|b| (b, x as usize, (z - 16) as usize)),
            _ => None,
        };
        side.unwrap_or(own)
    };
    // What each cell lets through: 0 stops light, 1 passes it, 2 dims it a step more.
    let mut pass = vec![1u8; n];
    let mut glow: Vec<(usize, u8)> = Vec::new();
    // The sky: full above the land in each column.
    let mut sky = vec![0u8; n];
    let mut queue = std::collections::VecDeque::new();
    // Each column's highest block that stops the sky (below the grid where none does).
    let mut tops = vec![min_y - 1; (w * w) as usize];
    for z in -LIGHT_PAD..16 + LIGHT_PAD {
        for x in -LIGHT_PAD..16 + LIGHT_PAD {
            let (b, bx, bz) = column(x, z);
            let mut lit = true;
            for y in (min_y..min_y + h).rev() {
                let id = b.get(bx, y, bz);
                let j = idx(x, y, z);
                if id != 0 {
                    let inf = info(id);
                    pass[j] = if inf.solid { 0 } else if inf.dims { 2 } else { 1 };
                    if lit && (inf.solid || inf.dims) {
                        lit = false;
                        tops[((z + LIGHT_PAD) * w + x + LIGHT_PAD) as usize] = y;
                    }
                    if inf.glow > 0 {
                        glow.push((j, inf.glow));
                    }
                }
                if lit {
                    sky[j] = 15;
                }
            }
        }
    }
    // Only open sky that can spread sideways into shade needs spreading: at most as high as a
    // neighboring column's top (open sky above every neighbor only meets more open sky).
    // (Spreading every open cell made lighting most of the time a chunk took.)
    for z in -LIGHT_PAD..16 + LIGHT_PAD {
        for x in -LIGHT_PAD..16 + LIGHT_PAD {
            let top = tops[((z + LIGHT_PAD) * w + x + LIGHT_PAD) as usize];
            let mut reach = top;
            for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                let (nx, nz) = (x + dx, z + dz);
                if (-LIGHT_PAD..16 + LIGHT_PAD).contains(&nx) && (-LIGHT_PAD..16 + LIGHT_PAD).contains(&nz) {
                    reach = reach.max(tops[((nz + LIGHT_PAD) * w + nx + LIGHT_PAD) as usize]);
                }
            }
            for y in (top + 1).max(min_y)..(reach + 2).min(min_y + h) {
                queue.push_back(idx(x, y, z));
            }
        }
    }
    let spread = |light: &mut Vec<u8>, queue: &mut std::collections::VecDeque<usize>, sky: bool| {
        while let Some(c) = queue.pop_front() {
            let l = light[c];
            if l <= 1 {
                continue;
            }
            let (x, y, z) = ((c as i32 % w) - LIGHT_PAD, c as i32 / layer + min_y, ((c as i32 / w) % w) - LIGHT_PAD);
            for (dx, dy, dz) in [(0, -1, 0), (0, 1, 0), (-1, 0, 0), (1, 0, 0), (0, 0, -1), (0, 0, 1)] {
                let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                let span = -LIGHT_PAD..16 + LIGHT_PAD;
                if !span.contains(&nx) || !span.contains(&nz) || ny < min_y || ny >= min_y + h {
                    continue;
                }
                let j = idx(nx, ny, nz);
                if pass[j] == 0 {
                    continue;
                }
                // The sky comes straight down undimmed through clear air; every other step fades
                // it by one (through water and leaves too, as in the game: they only stop the
                // sky's full light coming straight down).
                let v = if sky && dy == -1 && l == 15 && pass[j] == 1 { 15 } else { l - 1 };
                if v > light[j] {
                    light[j] = v;
                    queue.push_back(j);
                }
            }
        }
    };
    spread(&mut sky, &mut queue, true);
    // Glowing blocks.
    let mut lamp = vec![0u8; n];
    for (k, g) in glow {
        lamp[k] = g;
        queue.push_back(k);
    }
    spread(&mut lamp, &mut queue, false);
    LightGrid { min_y, h, sky, lamp }
}

/// For saving console chunks the editing tools changed: each one's light worked out again (the
/// sky's and the glowing blocks', as the 3D view lights them) and written into its `SkyLight`
/// and `BlockLight`, with the chunks beside each (whose light near the edit changes too). The
/// console games keep the light as saved, so it has to be right in the file. Gives each chunk
/// (x, z) relit, as it is now.
pub fn console_relight(save: &std::path::Path, dim: &str, edits: &crate::edit::EditLayer, chunks: &[(i32, i32)]) -> Vec<((i32, i32), mctk_nbt::NamedTag)> {
    let Ok(source) = open_source(save, Edition::Console, dim) else { return Vec::new() };
    let Source::Console { entries, regions, .. } = &source else { return Vec::new() };
    // The chunk as it is now: edited, or as saved.
    let root_of = |cx: i32, cz: i32| -> Option<mctk_nbt::NamedTag> {
        if let Some(r) = edits.console.get(&(dim.to_string(), cx, cz)) {
            return r.clone();
        }
        let (rx, rz) = (cx.div_euclid(32), cz.div_euclid(32));
        let region = {
            let mut r = regions.lock().ok()?;
            r.entry((rx, rz)).or_insert_with(|| entries.get(&(rx, rz)).and_then(|d| mctk_console::ConsoleRegion::parse(d.clone()).ok()).map(Arc::new)).clone()?
        };
        region.raw_chunk(cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize).ok()??.nbt().ok()
    };
    let mut todo: Vec<(i32, i32)> = Vec::new();
    for &(x, z) in chunks {
        for k in [(x, z), (x - 1, z), (x + 1, z), (x, z - 1), (x, z + 1)] {
            if !todo.contains(&k) {
                todo.push(k);
            }
        }
    }
    let mut cache: HashMap<Key, Option<Arc<Blocks>>> = HashMap::new();
    let mut blocks = |k: Key| -> Option<Arc<Blocks>> { cache.entry(k).or_insert_with(|| load(&source, k.0, k.1, None, edits, dim).map(Arc::new)).clone() };
    let mut infos = Infos::default();
    let mut out = Vec::new();
    for (cx, cz) in todo {
        let Some(mut root) = root_of(cx, cz) else { continue };
        let Some(me) = blocks((cx, cz)) else { continue };
        let near = [(-1, 0), (1, 0), (0, -1), (0, 1)].map(|(dx, dz)| blocks((cx + dx, cz + dz)));
        let infos = std::cell::RefCell::new(&mut infos);
        let grid = light_grid(&me, &near, &|id| infos.borrow_mut().get(id));
        let Some(level) = root.tag.as_compound_mut().and_then(|c| c.get_mut("Level")).and_then(mctk_nbt::Tag::as_compound_mut) else { continue };
        let height = match level.get("Blocks") {
            Some(mctk_nbt::Tag::ByteArray(b)) => b.len() / 256,
            _ => continue,
        };
        let mut sky = vec![0i8; height * 128];
        let mut lamp = vec![0i8; height * 128];
        let put = |arr: &mut Vec<i8>, i: usize, v: u8| {
            let b = arr[i / 2] as u8;
            arr[i / 2] = if i % 2 == 0 { (b & 0xf0) | (v & 0x0f) } else { (b & 0x0f) | (v << 4) } as i8;
        };
        for y in 0..height {
            for z in 0..16 {
                for x in 0..16 {
                    let (s, l) = grid.at(x as i32, y as i32, z as i32);
                    let i = mctk_console::console_block_index(x, y, z);
                    put(&mut sky, i, s);
                    put(&mut lamp, i, l);
                }
            }
        }
        level.insert("SkyLight".into(), mctk_nbt::Tag::ByteArray(sky));
        level.insert("BlockLight".into(), mctk_nbt::Tag::ByteArray(lamp));
        out.push(((cx, cz), root));
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Making meshes
// ---------------------------------------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Vert {
    pos: [f32; 3],
    uv: [f32; 2],
    color: [u8; 4],
    /// The sky's light and the glowing blocks' at the corner (0 to 255 for 0 to 15) and how
    /// open the corner is (0 to 255: tucked into a corner of blocks to out in the open).
    light: [u8; 4],
}

/// The color that marks a line to be drawn inverted over what is behind it (the chunk under
/// the pointer), as the game's crosshair is.
pub const INVERT_LINE: egui::Color32 = egui::Color32::from_rgba_premultiplied(254, 254, 254, 254);

/// Full light and an open corner: for what has no light of its own (the low-detail land,
/// lines).
const LIT: [u8; 4] = [255, 0, 255, 0];

/// A chunk's mesh: its corners, the faces drawn solid (with see-through holes cut out) and then
/// the see-through ones (water), and how high it reaches (for leaving out what is off screen).
pub struct MeshData {
    verts: Vec<Vert>,
    solid: Vec<u32>,
    water: Vec<u32>,
    lo: f32,
    hi: f32,
}

/// Light on each face, as the game shades them.
const SHADE: [f32; 6] = [1.0, 0.5, 0.8, 0.8, 0.6, 0.6];

/// Each face's outward direction and the two directions across it (top, bottom, north, south,
/// west, east).
const NORMALS: [[i32; 3]; 6] = [[0, 1, 0], [0, -1, 0], [0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]];
const ACROSS: [[usize; 2]; 6] = [[0, 2], [0, 2], [0, 1], [0, 1], [2, 1], [2, 1]];

struct Mesher<'a> {
    atlas: &'a Mutex<Atlas>,
    infos: &'a std::cell::RefCell<Infos>,
    slots: HashMap<(u16, u8), Option<Slot>>,
    files: HashMap<Arc<str>, Option<(Slot, bool)>>,
    models: HashMap<u16, Option<ChunkModel>>,
    out: MeshData,
}

/// A block's models as a chunk's mesher uses them: the models, the tint of their tinted faces,
/// and each face's place in the block texture (by model, by face).
#[derive(Clone)]
struct ChunkModel {
    baked: Arc<crate::models::Baked>,
    tint: [f32; 3],
    slots: Arc<Vec<Vec<Option<(Slot, bool)>>>>,
}

impl Mesher<'_> {
    fn info(&self, id: u16) -> Info {
        self.infos.borrow_mut().get(id)
    }

    fn slot(&mut self, id: u16, face: Face) -> Option<Slot> {
        let k = match face {
            Face::Top => 0,
            Face::Side => 1,
            Face::Under => 2,
            Face::Extra => 3,
        };
        let atlas = self.atlas;
        *self.slots.entry((id, k)).or_insert_with(|| atlas.lock().ok().and_then(|mut a| a.slot(id, face)))
    }

    /// A block's models and the tint of its tinted faces (once a chunk: the shared tables are
    /// locked once per kind of block, not once per block).
    fn model_of(&mut self, id: u16) -> Option<ChunkModel> {
        if let Some(m) = self.models.get(&id) {
            return m.clone();
        }
        let made = baked_of(id).map(|b| {
            let name = world::block_name(id).unwrap_or_default();
            let tint = world::block_tint(&name, world::block_props(id).as_deref().unwrap_or(""));
            // Each face's place in the block texture, worked out here once for the chunk.
            let slots = b.alts.iter().map(|(m, _)| m.quads.iter().map(|q| self.file_slot(&q.tex)).collect()).collect();
            ChunkModel { baked: b, tint, slots: Arc::new(slots) }
        });
        self.models.insert(id, made.clone());
        made
    }

    /// A texture file's place in the block texture, and whether it is partly see-through (once
    /// looked up).
    fn file_slot(&mut self, file: &Arc<str>) -> Option<(Slot, bool)> {
        let atlas = self.atlas;
        *self.files.entry(file.clone()).or_insert_with(|| {
            // Fetched (if need be) before the shared texture is locked, so no one waits on the
            // download.
            crate::icons::pack_texture(file)?;
            let s = atlas.lock().ok().and_then(|mut a| a.file_slot(file))?;
            Some((s, crate::models::translucent(file)))
        })
    }

    /// A four-cornered face (corners top-left, top-right, bottom-right, bottom-left as seen),
    /// with part of a picture and each corner's light.
    fn quad(&mut self, p: [[f32; 3]; 4], uv: Rect, rgb: [f32; 3], alpha: f32, water: bool, light: [[u8; 3]; 4]) {
        self.quad_uvs(p, [uv.left_top(), uv.right_top(), uv.right_bottom(), uv.left_bottom()], rgb, alpha, water, light);
    }

    /// As [`Mesher::quad`], with each corner's place in the picture given.
    fn quad_uvs(&mut self, p: [[f32; 3]; 4], uvs: [egui::Pos2; 4], rgb: [f32; 3], alpha: f32, water: bool, light: [[u8; 3]; 4]) {
        let c = [
            (rgb[0].clamp(0.0, 1.0) * 255.0) as u8,
            (rgb[1].clamp(0.0, 1.0) * 255.0) as u8,
            (rgb[2].clamp(0.0, 1.0) * 255.0) as u8,
            (alpha.clamp(0.0, 1.0) * 255.0) as u8,
        ];
        let i = self.out.verts.len() as u32;
        for k in 0..4 {
            self.out.verts.push(Vert { pos: p[k], uv: [uvs[k].x, uvs[k].y], color: c, light: [light[k][0], light[k][1], light[k][2], 0] });
            self.out.lo = self.out.lo.min(p[k][1]);
            self.out.hi = self.out.hi.max(p[k][1]);
        }
        // Split along the brighter diagonal, so the shading of a corner does not streak.
        let sum = |a: usize, b: usize| (0..3).map(|q| light[a][q] as u32 + light[b][q] as u32).sum::<u32>();
        let tris = if sum(0, 2) < sum(1, 3) { [i + 1, i + 2, i + 3, i + 1, i + 3, i] } else { [i, i + 1, i + 2, i, i + 2, i + 3] };
        let list = if water { &mut self.out.water } else { &mut self.out.solid };
        list.extend_from_slice(&tris);
    }

    /// The faces of a box from `lo` to `hi`, those in `show` (top, bottom, north, south, west,
    /// east), each with its picture (a picture's part matching the face's size), and each
    /// corner's light from `light` (given the face and the corner).
    #[allow(clippy::too_many_arguments)]
    fn boxed(&mut self, lo: [f32; 3], hi: [f32; 3], show: [bool; 6], pics: [Option<Slot>; 6], alpha: f32, water: bool, light: &dyn Fn(usize, [f32; 3]) -> [u8; 3]) {
        let (x0, y0, z0) = (lo[0], lo[1], lo[2]);
        let (x1, y1, z1) = (hi[0], hi[1], hi[2]);
        let (w, h, d) = (x1 - x0, y1 - y0, z1 - z0);
        let faces: [[[f32; 3]; 4]; 6] = [
            [[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]],
            [[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0]],
            [[x1, y1, z0], [x0, y1, z0], [x0, y0, z0], [x1, y0, z0]],
            [[x0, y1, z1], [x1, y1, z1], [x1, y0, z1], [x0, y0, z1]],
            [[x0, y1, z0], [x0, y1, z1], [x0, y0, z1], [x0, y0, z0]],
            [[x1, y1, z1], [x1, y1, z0], [x1, y0, z0], [x1, y0, z1]],
        ];
        for k in 0..6 {
            if !show[k] {
                continue;
            }
            let Some(s) = pics[k] else { continue };
            // The part of the picture as big as the face: tops by width and depth, sides by
            // width and height (the bottom part of the picture for a shorter block).
            let (u, v) = match k {
                0 | 1 => (w, d),
                2 | 3 => (w, h),
                _ => (d, h),
            };
            let (u0, u1) = (0.5 - u.min(1.0) * 0.5, 0.5 + u.min(1.0) * 0.5);
            let uv = if k < 2 { s.part(u0, u1, 0.5 - v.min(1.0) * 0.5, 0.5 + v.min(1.0) * 0.5) } else { s.part(u0, u1, 1.0 - v.min(1.0), 1.0) };
            let lights = faces[k].map(|c| light(k, c));
            self.quad(faces[k], uv, s.tint.map(|c| c * SHADE[k]), alpha, water, lights);
        }
    }

    /// Two crossed pictures (a plant), each seen from both sides.
    fn cross(&mut self, x: f32, y: f32, z: f32, h: f32, s: Slot, light: [u8; 3]) {
        let flip = Rect::from_min_max(egui::pos2(s.uv.max.x, s.uv.min.y), egui::pos2(s.uv.min.x, s.uv.max.y));
        for (a, b, uv) in [
            ([x, z], [x + 1.0, z + 1.0], s.uv),
            ([x + 1.0, z], [x, z + 1.0], s.uv),
            ([x + 1.0, z + 1.0], [x, z], flip),
            ([x, z + 1.0], [x + 1.0, z], flip),
        ] {
            let s = Slot { uv, ..s };
            self.quad([[a[0], y + h, a[1]], [b[0], y + h, b[1]], [b[0], y, b[1]], [a[0], y, a[1]]], s.uv, s.tint.map(|c| c * 0.9), 1.0, false, [light; 4]);
        }
    }
}

/// Makes a chunk's mesh: the faces of its blocks that touch air (or see-through blocks of
/// another kind), each corner with its light and how tucked in among blocks it is. `near` are
/// the chunks to the west, east, north and south, for the faces at its edges (a chunk not read
/// yet counts as air there).
fn mesh(cx: i32, cz: i32, me: &Blocks, near: &[Option<Arc<Blocks>>; 4], atlas: &Mutex<Atlas>) -> MeshData {
    let infos = std::cell::RefCell::new(Infos::default());
    let info = |id: u16| infos.borrow_mut().get(id);
    let at = |x: i32, y: i32, z: i32| -> u16 {
        match (x, z) {
            (0..=15, 0..=15) => me.get(x as usize, y, z as usize),
            (..0, 0..=15) => near[0].as_ref().map_or(0, |b| b.get((x + 16) as usize, y, z as usize)),
            (16.., 0..=15) => near[1].as_ref().map_or(0, |b| b.get((x - 16) as usize, y, z as usize)),
            (0..=15, ..0) => near[2].as_ref().map_or(0, |b| b.get(x as usize, y, (z + 16) as usize)),
            (0..=15, 16..) => near[3].as_ref().map_or(0, |b| b.get(x as usize, y, (z - 16) as usize)),
            _ => 0,
        }
    };
    let lit_at = std::time::Instant::now();
    let grid = light_grid(me, near, &info);
    profile(2, lit_at);
    let solid =|x: i32, y: i32, z: i32| info(at(x, y, z)).solid;
    let mut m = Mesher { atlas, infos: &infos, slots: HashMap::new(), files: HashMap::new(), models: HashMap::new(), out: MeshData { verts: Vec::new(), solid: Vec::new(), water: Vec::new(), lo: f32::MAX, hi: f32::MIN } };
    for (s, sec) in me.sections.iter().enumerate() {
        let Some(sec) = sec else { continue };
        let sy = (me.min_section + s as i32) * 16;
        for (i, &id) in sec.iter().enumerate() {
            if id == 0 {
                continue;
            }
            let inf = m.info(id);
            if inf.shape == Shape::Air {
                continue;
            }
            let (lx, ly, lz) = ((i % 16) as i32, (i / 256) as i32, ((i / 16) % 16) as i32);
            let y = sy + ly;
            let (wx, wy, wz) = ((cx * 16 + lx) as f32, y as f32, (cz * 16 + lz) as f32);
            let b = [lx, y, lz];
            // The light a face gets is that of the cell it faces; each corner is darkened the
            // more it is tucked in among the blocks around it (the game's smooth shading).
            let corner_light = |k: usize, c: [f32; 3]| -> [u8; 3] {
                let nrm = NORMALS[k];
                let f = [b[0] + nrm[0], b[1] + nrm[1], b[2] + nrm[2]];
                let (sky, lamp) = grid.at(f[0], f[1], f[2]);
                let local = [c[0] - (cx * 16) as f32, c[1], c[2] - (cz * 16) as f32];
                let [t1, t2] = ACROSS[k];
                let sign = |t: usize| if local[t] - (b[t] as f32) < 0.5 { -1 } else { 1 };
                let mut p1 = f;
                p1[t1] += sign(t1);
                let mut p2 = f;
                p2[t2] += sign(t2);
                let mut pc = p1;
                pc[t2] += sign(t2);
                let (s1, s2, sc) = (solid(p1[0], p1[1], p1[2]), solid(p2[0], p2[1], p2[2]), solid(pc[0], pc[1], pc[2]));
                let open = if s1 && s2 { 0 } else { 3 - (s1 as u8 + s2 as u8 + sc as u8) };
                [sky * 17, lamp.max(inf.glow) * 17, open * 85]
            };
            let (sky, lamp) = grid.at(lx, y, lz);
            let own = [sky * 17, lamp.max(inf.glow) * 17, 255];
            // Drawn in code as the game does (chests, banners, heads...), lit by its own cell.
            if inf.entity {
                let name = world::block_name(id).unwrap_or_default();
                let base = name.split(['#', '@']).next().unwrap_or(&name);
                let props = world::block_props(id);
                let extra = me.extras.get(&(lx as u8, y, lz as u8));
                if let Some(quads) = crate::entities::block_entity(base, props.as_deref().unwrap_or(""), extra.map(|e| e.as_ref())) {
                    for q in quads {
                        let Some((s, see_through)) = m.file_slot(&q.tex) else { continue };
                        let uvs = q.uv.map(|[u, v]| egui::pos2(s.uv.min.x + u * s.uv.width(), s.uv.min.y + v * s.uv.height()));
                        let p = q.pos.map(|c| [wx + c[0], wy + c[1], wz + c[2]]);
                        m.quad_uvs(p, uvs, q.tint.map(|c| c * q.shade), 1.0, see_through, [own; 4]);
                    }
                }
                if inf.wet {
                    if let Some(ws) = m.slot(world::intern("water"), Face::Top) {
                        let mut show = [false; 6];
                        for (k, d) in NORMALS.iter().enumerate() {
                            let n = info(at(lx + d[0], y + d[1], lz + d[2]));
                            show[k] = !(n.solid || n.group == 1 || n.wet);
                        }
                        let top = if info(at(lx, y + 1, lz)).group == 1 || info(at(lx, y + 1, lz)).wet { 1.0 } else { 0.875 };
                        // A hair inside the block, so its own faces at its sides never fight the water's.
                        let e = 0.002;
                        m.boxed([wx + e, wy + e, wz + e], [wx + 1.0 - e, wy + top - e, wz + 1.0 - e], show, [Some(ws); 6], 0.72, true, &corner_light);
                    }
                }
                continue;
            }
            // The game's own model for the block's state.
            if inf.model {
                if let Some(cm) = m.model_of(id) {
                    let tint = cm.tint;
                    let alt = cm.baked.pick_index(cx * 16 + lx, y, cz * 16 + lz);
                    let model = &cm.baked.alts[alt].0;
                    let slots = &cm.slots[alt];
                    for (q, slot) in model.quads.iter().zip(slots.iter()) {
                        // Hidden against a whole block (or its own kind of see-through block).
                        if let Some(c) = q.cull {
                            let d = NORMALS[c];
                            let n = info(at(lx + d[0], y + d[1], lz + d[2]));
                            if n.solid || (inf.group != 0 && n.group == inf.group) {
                                continue;
                            }
                        }
                        let Some((s, see_through)) = *slot else { continue };
                        let uvs = q.uv.map(|[u, v]| egui::pos2(s.uv.min.x + u * s.uv.width(), s.uv.min.y + v * s.uv.height()));
                        let p = q.pos.map(|c| [wx + c[0], wy + c[1], wz + c[2]]);
                        // On the block's side: the light beyond it (shaded in the corners where
                        // the model is); inside the block: its own.
                        let lights = match (q.face, q.on_side) {
                            (Some(k), true) if model.ao => p.map(|c| corner_light(k, c)),
                            (Some(k), true) => {
                                let d = NORMALS[k];
                                let (sk, lp) = grid.at(lx + d[0], y + d[1], lz + d[2]);
                                [[sk * 17, lp.max(inf.glow) * 17, 255]; 4]
                            }
                            _ => [own; 4],
                        };
                        let rgb = if q.tint { tint } else { [1.0; 3] }.map(|c| c * q.shade);
                        m.quad_uvs(p, uvs, rgb, 1.0, see_through, lights);
                    }
                    // Waterlogged: water around it, where no water or whole block is beside.
                    if inf.wet {
                        let water_id = world::intern("water");
                        if let Some(ws) = m.slot(water_id, Face::Top) {
                            let mut show = [false; 6];
                            for (k, d) in NORMALS.iter().enumerate() {
                                let n = info(at(lx + d[0], y + d[1], lz + d[2]));
                                show[k] = !(n.solid || n.group == 1 || n.wet);
                            }
                            let top = if info(at(lx, y + 1, lz)).group == 1 || info(at(lx, y + 1, lz)).wet { 1.0 } else { 0.875 };
                            // A hair inside the block, so its own faces at its sides never fight the water's.
                        let e = 0.002;
                        m.boxed([wx + e, wy + e, wz + e], [wx + 1.0 - e, wy + top - e, wz + 1.0 - e], show, [Some(ws); 6], 0.72, true, &corner_light);
                        }
                    }
                    continue;
                }
            }
            match inf.shape {
                Shape::Liquid { water } => {
                    // The height of the same liquid in a cell (full with more of it above), or
                    // `None` for anything else.
                    let level = |x: i32, y: i32, z: i32| -> Option<f32> {
                        let n = at(x, y, z);
                        if water && info(n).wet {
                            return Some(if info(at(x, y + 1, z)).group == inf.group { 1.0 } else { 0.875 });
                        }
                        if info(n).group != inf.group {
                            return None;
                        }
                        if info(at(x, y + 1, z)).group == inf.group {
                            return Some(1.0);
                        }
                        Some(world::block_name(n).and_then(|s| liquid_height(&s)).unwrap_or(0.875))
                    };
                    let full = info(at(lx, y + 1, lz)).group == inf.group;
                    // Each top corner: the mean height of the liquid in the four cells meeting
                    // there (full where any has the liquid above it), so neighbors join in one
                    // sloping surface. Corners in order: north-west, north-east, south-east,
                    // south-west.
                    let corner = |i: i32, j: i32| -> f32 {
                        if full {
                            return 1.0;
                        }
                        let (mut sum, mut n) = (0.0, 0.0);
                        for (dx, dz) in [(i - 1, j - 1), (i, j - 1), (i - 1, j), (i, j)] {
                            if info(at(lx + dx, y + 1, lz + dz)).group == inf.group {
                                return 1.0;
                            }
                            if let Some(h) = level(lx + dx, y, lz + dz) {
                                sum += h;
                                n += 1.0;
                            }
                        }
                        if n > 0.0 { sum / n } else { 0.875 }
                    };
                    let hs = [corner(0, 0), corner(1, 0), corner(1, 1), corner(0, 1)];
                    // Seagrass and kelp stand in the water, upright.
                    if inf.sea_plant {
                        // Tall seagrass is two blocks: its bottom picture over anything else, its
                        // top over its own bottom.
                        let tall = world::block_name(id).is_some_and(|n| n.starts_with("tall_seagrass"));
                        let half = tall.then(|| if at(lx, y - 1, lz) == id { "block/tall_seagrass_top.png" } else { "block/tall_seagrass_bottom.png" });
                        let pic = half.and_then(|f| m.atlas.lock().ok().and_then(|mut a| a.file_slot(f))).or_else(|| m.slot(id, Face::Side));
                        if let Some(p) = pic {
                            m.cross(wx, wy, wz, 1.0, p, own);
                        }
                    }
                    let id = if inf.sea_plant { world::intern("water") } else { id };
                    let mut show = [false; 6];
                    for (k, d) in NORMALS.iter().enumerate() {
                        let n = info(at(lx + d[0], y + d[1], lz + d[2]));
                        let same = n.group == inf.group || (water && n.wet);
                        show[k] = !same && (!n.solid || (k == 0 && hs.iter().any(|&h| h < 1.0)));
                    }
                    if show == [false; 6] {
                        continue;
                    }
                    let Some(s) = m.slot(id, Face::Top) else { continue };
                    // Water as see-through as its own picture makes it (as the game draws it:
                    // the blue shows, and what lies under shallow water).
                    let alpha = 1.0;
                    let (x0, x1, y0, z0, z1) = (wx, wx + 1.0, wy, wz, wz + 1.0);
                    let [nw, ne, se, sw] = hs.map(|h| wy + h);
                    let faces: [([[f32; 3]; 4], f32); 6] = [
                        ([[x0, nw, z0], [x1, ne, z0], [x1, se, z1], [x0, sw, z1]], 1.0),
                        ([[x0, y0, z1], [x1, y0, z1], [x1, y0, z0], [x0, y0, z0]], 1.0),
                        ([[x1, ne, z0], [x0, nw, z0], [x0, y0, z0], [x1, y0, z0]], hs[0].max(hs[1])),
                        ([[x0, sw, z1], [x1, se, z1], [x1, y0, z1], [x0, y0, z1]], hs[2].max(hs[3])),
                        ([[x0, nw, z0], [x0, sw, z1], [x0, y0, z1], [x0, y0, z0]], hs[0].max(hs[3])),
                        ([[x1, se, z1], [x1, ne, z0], [x1, y0, z0], [x1, y0, z1]], hs[1].max(hs[2])),
                    ];
                    // Which way the liquid runs on top: down the slope of its surface. The
                    // flowing picture moves down itself as it plays, so it is turned to point
                    // that way (a little of its middle taken when turned at a slant, so it
                    // stays inside its own picture).
                    let (sx, sz) = ((hs[1] + hs[2]) - (hs[0] + hs[3]), (hs[2] + hs[3]) - (hs[0] + hs[1]));
                    let top_uvs = (sx.abs() + sz.abs() > 0.01).then(|| {
                        let len = (sx * sx + sz * sz).sqrt();
                        let (fx, fz) = (-sx / len, -sz / len);
                        let k = 1.0 / (fx.abs() + fz.abs());
                        [(-0.5f32, -0.5f32), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)].map(|(ox, oz)| {
                            let (u, v) = ((ox * fz - oz * fx) * k, (ox * fx + oz * fz) * k);
                            s.uv.min + egui::vec2((0.5 + u) * s.uv.width(), (0.5 + v) * s.uv.height())
                        })
                    });
                    for (k, (p, tall)) in faces.into_iter().enumerate() {
                        if !show[k] {
                            continue;
                        }
                        if let (0, Some(uvs)) = (k, top_uvs) {
                            let lights = p.map(|c| corner_light(k, c));
                            m.quad_uvs(p, uvs, s.tint.map(|c| c * SHADE[k]), alpha, water, lights);
                            continue;
                        }
                        let uv = if k < 2 { s.uv } else { s.part(0.0, 1.0, 1.0 - tall.min(1.0), 1.0) };
                        let lights = p.map(|c| corner_light(k, c));
                        m.quad(p, uv, s.tint.map(|c| c * SHADE[k]), alpha, water, lights);
                    }
                }
                Shape::Cube(_) => {
                    let liquid = matches!(inf.shape, Shape::Liquid { .. });
                    let water = matches!(inf.shape, Shape::Liquid { water: true });
                    // How tall: a partial block its own height; a liquid its level, or full
                    // with more of it above.
                    let h = match inf.shape {
                        Shape::Cube(h) => h,
                        _ if m.info(at(lx, y + 1, lz)).group == inf.group => 1.0,
                        _ => world::block_name(id).and_then(|n| liquid_height(&n)).unwrap_or(0.875),
                    };
                    let mut show = [false; 6];
                    for (k, d) in NORMALS.iter().enumerate() {
                        let n = m.info(at(lx + d[0], y + d[1], lz + d[2]));
                        let same = inf.group != 0 && n.group == inf.group;
                        // A lower top (a slab, a liquid's surface) is always seen.
                        show[k] = (k == 0 && h < 1.0 && !same) || !(n.solid || same);
                    }
                    if show == [false; 6] {
                        continue;
                    }
                    let (top, side) = if liquid { (m.slot(id, Face::Top), m.slot(id, Face::Top)) } else { (m.slot(id, Face::Top), m.slot(id, Face::Side)) };
                    let bottom = if inf.dirt_bottom { m.slot(id, Face::Under) } else { top };
                    // A log lying down shows its ends at its two ends, its bark elsewhere.
                    let (end, bark) = if inf.axis != 0 { (m.slot(inf.ends, Face::Top), m.slot(inf.ends, Face::Side)) } else { (top, side) };
                    let mut pics = match inf.axis {
                        1 => [bark, bark, bark, bark, end, end],
                        2 => [bark, bark, end, end, bark, bark],
                        _ => [top, bottom, side, side, side, side],
                    };
                    // A block with no picture found (a name the pack does not know) is still
                    // drawn, plain in its map color: left out, it would leave a hole in the land
                    // (its neighbors' faces toward it are hidden).
                    if pics.iter().any(Option::is_none) {
                        let flat = world::block_name(id).map_or([128; 3], |n| world::surface_color(&n));
                        let plain = Slot { uv: Atlas::white(), tint: flat.map(|c| c as f32 / 255.0) };
                        for p in &mut pics {
                            p.get_or_insert(plain);
                        }
                    }
                    let alpha = if water { 0.72 } else { 1.0 };
                    m.boxed([wx, wy, wz], [wx + 1.0, wy + h, wz + 1.0], show, pics, alpha, water, &corner_light);
                    if !liquid {
                        if let Some(fire) = m.slot(id, Face::Extra) {
                            m.cross(wx, wy + h, wz, 1.0, fire, [255, 255, 255]);
                        }
                    }
                }
                Shape::Cross if inf.drip => {
                    // A stack of dripstone tapers from its thick base where it hangs (or stands)
                    // to its tip: which way it points, and how far along this one is, from the
                    // stack around it.
                    let is_drip = |y: i32| info(at(lx, y, lz)).drip;
                    let (mut top, mut bottom) = (y, y);
                    while is_drip(top + 1) {
                        top += 1;
                    }
                    while is_drip(bottom - 1) {
                        bottom -= 1;
                    }
                    let hung = solid(lx, top + 1, lz);
                    let stood = solid(lx, bottom - 1, lz);
                    let down = hung && !stood;
                    // From the tip (0) toward the base, and the stack's length.
                    let (from_tip, len) = (if down { y - bottom } else { top - y }, top - bottom + 1);
                    let part = match from_tip {
                        0 => "tip",
                        1 => "frustum",
                        k if k == len - 1 => "base",
                        _ => "middle",
                    };
                    let file = format!("block/pointed_dripstone_{}_{part}.png", if down { "down" } else { "up" });
                    let s = m.atlas.lock().ok().and_then(|mut a| a.file_slot(&file)).or_else(|| m.slot(id, Face::Side));
                    if let Some(s) = s {
                        m.cross(wx, wy, wz, 1.0, s, own);
                    }
                }
                Shape::Cross => {
                    if let Some(s) = m.slot(id, Face::Side) {
                        m.cross(wx, wy, wz, 1.0, s, own);
                    }
                }
                Shape::Post(w, h) => {
                    let pics = [m.slot(id, Face::Top), None, m.slot(id, Face::Side), m.slot(id, Face::Side), m.slot(id, Face::Side), m.slot(id, Face::Side)];
                    m.boxed([wx + 0.5 - w, wy, wz + 0.5 - w], [wx + 0.5 + w, wy + h, wz + 0.5 + w], [true, false, true, true, true, true], pics, 1.0, false, &|_, _| own);
                }
                Shape::Decal => {
                    if let Some(s) = m.slot(id, Face::Top) {
                        let y = wy + 0.03;
                        m.quad([[wx, y, wz], [wx + 1.0, y, wz], [wx + 1.0, y, wz + 1.0], [wx, y, wz + 1.0]], s.uv, s.tint, 1.0, false, [own; 4]);
                    }
                }
                Shape::Air => {}
            }
        }
    }
    // Its entities (item frames, armor stands, items on the ground, paintings), lit by the
    // light where each is.
    let origin = [(cx * 16) as f32, 0.0, (cz * 16) as f32];
    for e in &me.entities {
        let Some((quads, at)) = crate::entities::entity(e, origin) else { continue };
        let (lx, ly, lz) = (at[0].floor() as i32 - cx * 16, at[1].floor() as i32, at[2].floor() as i32 - cz * 16);
        let (sky, lamp) = grid.at(lx, ly, lz);
        let light = [sky * 17, lamp * 17, 255];
        for q in quads {
            let Some((s, see_through)) = m.file_slot(&q.tex) else { continue };
            let uvs = q.uv.map(|[u, v]| egui::pos2(s.uv.min.x + u * s.uv.width(), s.uv.min.y + v * s.uv.height()));
            let p = q.pos.map(|c| [c[0] + origin[0], c[1], c[2] + origin[2]]);
            m.quad_uvs(p, uvs, q.tint.map(|c| c * q.shade), 1.0, see_through, [light; 4]);
        }
    }
    m.out
}

// ---------------------------------------------------------------------------------------------
// The low-detail land further out
// ---------------------------------------------------------------------------------------------

/// Chunks across a low-detail tile, how far out (in chunks) low-detail land is drawn, and the
/// blocks across one of its cells.
pub const LOD_TILE: i32 = 4;
/// Levels of the block texture: 16 pixels a picture down to 1.
const MIPS: u32 = 5;
pub const LOD_REACH: i32 = 64;
/// Water's color as the real blocks come out (the pack's gray water tinted toward the game's
/// blue, measured off the drawn blocks), so the low-detail water meets theirs without a seam.
const WATER_BLUE: [f32; 3] = [71.0, 108.0, 140.0];

/// The blocks across a low-detail cell at a distance (in chunks) past the real blocks' edge:
/// single blocks next to them, coarser further out.
fn lod_cell(past: f32) -> usize {
    if past < 4.0 {
        1
    } else if past < 12.0 {
        2
    } else if past < 28.0 {
        4
    } else {
        8
    }
}

/// One run of blocks up a low-detail column, from `lo` up to (not including) `hi`: its color,
/// and whether it is water.
#[derive(Clone, Copy, PartialEq)]
struct Run {
    lo: i16,
    hi: i16,
    rgb: [u8; 3],
    water: bool,
}

/// A chunk boiled down for the low-detail land (after the way Distant Horizons keeps distant
/// land): per cell `cell` blocks across, the runs of blocks up one column standing for it, from
/// the top down to just below where the land around can be seen from. Kept small for far-off
/// chunks (one column for each 8 x 8 blocks out there).
pub struct LodChunk {
    cell: usize,
    /// Each cell's runs (bottom up) are `runs[starts[i]..starts[i + 1]]`, cells z-major.
    starts: Vec<u32>,
    runs: Vec<Run>,
}

impl LodChunk {
    fn cell_runs(&self, cx: usize, cz: usize) -> &[Run] {
        let n = 16 / self.cell;
        let i = cz * n + cx;
        &self.runs[self.starts[i] as usize..self.starts[i + 1] as usize]
    }
}

/// How a block shows in the low-detail land: `None` where it is left out (air, plants, rails,
/// thin posts and the like, too small to see from afar), else its color and whether it is water.
fn lod_kind(id: u16) -> Option<([u8; 3], bool)> {
    static KINDS: std::sync::LazyLock<Mutex<HashMap<u16, Option<([u8; 3], bool)>>>> = std::sync::LazyLock::new(Default::default);
    if let Some(k) = KINDS.lock().ok().and_then(|m| m.get(&id).copied()) {
        return k;
    }
    let made = (|| {
        if id == 0 {
            return None;
        }
        let info = info_of(id);
        let name = world::block_name(id)?;
        let water = name.starts_with("water") || name.contains("bubble_column");
        match info.shape {
            Shape::Air | Shape::Cross | Shape::Decal => None,
            Shape::Liquid { .. } if water => Some((WATER_BLUE.map(|v| v as u8), true)),
            Shape::Post(w, _) if w < 0.3 => None,
            Shape::Cube(h) if h < 0.4 => None,
            _ => Some((world::surface_color(&name), false)),
        }
    })();
    if let Ok(mut m) = KINDS.lock() {
        m.insert(id, made);
    }
    made
}

/// Reduces a chunk read from the save to its low-detail columns, `cell` blocks across.
fn lod_chunk(b: &Blocks, cell: usize) -> LodChunk {
    let top_y = (b.min_section + b.sections.len() as i32) * 16 - 1;
    let bottom_y = b.min_section * 16;
    // Each column's surface (its highest block that shows).
    let mut surface = [bottom_y - 1; 256];
    for z in 0..16 {
        for x in 0..16 {
            let mut y = top_y;
            while y >= bottom_y {
                if lod_kind(b.get(x, y, z)).is_some() {
                    break;
                }
                y -= 1;
            }
            surface[z * 16 + x] = y;
        }
    }
    let n = 16 / cell;
    let (mut starts, mut runs) = (Vec::with_capacity(n * n + 1), Vec::new());
    let mut pick: Vec<(i32, usize, usize)> = Vec::with_capacity(cell * cell);
    for cz in 0..n {
        for cx in 0..n {
            starts.push(runs.len() as u32);
            // The column of middle height stands for the cell (not an average: the land keeps
            // whole-block steps and real colors).
            pick.clear();
            for dz in 0..cell {
                for dx in 0..cell {
                    let (x, z) = (cx * cell + dx, cz * cell + dz);
                    pick.push((surface[z * 16 + x], x, z));
                }
            }
            pick.sort_unstable();
            let (top, x, z) = pick[pick.len() / 2];
            if top < bottom_y {
                continue;
            }
            // Down to a little below the lowest land around (lower than that it is hidden by the
            // land around, unless it is a cliff into the neighboring chunk: further there).
            let mut floor = top;
            for (dx, dz) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let (nx, nz) = (x as i32 + dx * cell as i32, z as i32 + dz * cell as i32);
                if (0..16).contains(&nx) && (0..16).contains(&nz) {
                    floor = floor.min(surface[nz as usize * 16 + nx as usize]);
                } else {
                    floor = floor.min(top - 16);
                }
            }
            let floor = (floor - 3).max(bottom_y);
            let first = runs.len();
            let mut y = top;
            let mut cur: Option<(i32, i32, [u8; 3], bool)> = None;
            while y >= floor {
                let kind = lod_kind(b.get(x, y, z));
                match (kind, cur) {
                    (Some((rgb, w)), Some((lo, hi, c, cw))) if c == rgb && cw == w && lo == y + 1 => cur = Some((y, hi, c, cw)),
                    (k, c) => {
                        if let Some((lo, hi, rgb, water)) = c {
                            runs.push(Run { lo: lo as i16, hi: hi as i16, rgb, water });
                        }
                        cur = k.map(|(rgb, w)| (y, y + 1, rgb, w));
                    }
                }
                y -= 1;
            }
            if let Some((lo, hi, rgb, water)) = cur {
                runs.push(Run { lo: lo as i16, hi: hi as i16, rgb, water });
            }
            // Bottom up.
            runs[first..].reverse();
        }
    }
    starts.push(runs.len() as u32);
    LodChunk { cell, starts, runs }
}

/// A low-detail tile's mesh, from its chunks (`LOD_TILE` x `LOD_TILE`, by place in the tile;
/// `None` where nothing is saved): each cell's runs as boxes, with only the faces that can be
/// seen (tops, the undersides of overhangs and canopies, and sides where the next column does not
/// cover them), in each block's color, shaded by face as the game shades blocks, with a little
/// variation from block to block so wide land does not look painted. Water goes in see-through.
fn lod_mesh(tile: Key, cell: usize, chunks: &[Option<Arc<LodChunk>>]) -> MeshData {
    let mut out = MeshData { verts: Vec::new(), solid: Vec::new(), water: Vec::new(), lo: f32::MAX, hi: f32::MIN };
    let white = { let c = Atlas::white().center(); [c.x, c.y] };
    let per = 16 / cell;
    let n = LOD_TILE as usize * per;
    let at = |cx: i32, cz: i32| -> &[Run] {
        if cx < 0 || cz < 0 || cx >= n as i32 || cz >= n as i32 {
            return &[];
        }
        let (cx, cz) = (cx as usize, cz as usize);
        match &chunks[(cz / per) * LOD_TILE as usize + cx / per] {
            Some(c) if c.cell == cell => c.cell_runs(cx % per, cz % per),
            _ => &[],
        }
    };
    let (x0, z0) = ((tile.0 * LOD_TILE * 16) as f32, (tile.1 * LOD_TILE * 16) as f32);
    let s = cell as f32;
    let mut quad = |p: [[f32; 3]; 4], rgb: [u8; 3], shade: f32, sky: u8, water: bool| {
        // A little lighter or darker block to block (the same each time it is built).
        let h = (p[0][0] as i32).wrapping_mul(73_856_093) ^ (p[0][1] as i32).wrapping_mul(19_349_663) ^ (p[0][2] as i32).wrapping_mul(83_492_791);
        let vary = 0.94 + (h.rem_euclid(1000) as f32 / 1000.0) * 0.12;
        let k = shade * if water { 1.0 } else { vary };
        let color = [(rgb[0] as f32 * k).min(255.0) as u8, (rgb[1] as f32 * k).min(255.0) as u8, (rgb[2] as f32 * k).min(255.0) as u8, if water { 170 } else { 255 }];
        let i = out.verts.len() as u32;
        for v in p {
            out.verts.push(Vert { pos: v, uv: white, color: color, light: [sky, 0, 255, 0] });
            out.lo = out.lo.min(v[1]);
            out.hi = out.hi.max(v[1]);
        }
        let list = if water { &mut out.water } else { &mut out.solid };
        list.extend_from_slice(&[i, i + 1, i + 2, i, i + 2, i + 3]);
    };
    for cz in 0..n as i32 {
        for cx in 0..n as i32 {
            let col = at(cx, cz);
            let (xa, za) = (x0 + cx as f32 * s, z0 + cz as f32 * s);
            let (xb, zb) = (xa + s, za + s);
            for (j, r) in col.iter().enumerate() {
                let (y0, y1) = (r.lo as f32, r.hi as f32);
                let above = col.get(j + 1);
                let below = j.checked_sub(1).map(|b| col[b]);
                // (Water is only its surface, seen from above.)
                if r.water {
                    if above.is_none_or(|a| a.lo > r.hi) {
                        let y = y1 - 0.125;
                        quad([[xa, y, za], [xb, y, za], [xb, y, zb], [xa, y, zb]], r.rgb, 1.0, 255, true);
                    }
                    continue;
                }
                // The top, unless a solid block sits right on it.
                if above.is_none_or(|a| a.lo > r.hi || a.water) {
                    quad([[xa, y1, za], [xb, y1, za], [xb, y1, zb], [xa, y1, zb]], r.rgb, SHADE[0], 255, false);
                }
                // The underside of an overhang or a canopy (air below it, not the cut-off bottom).
                if below.is_some_and(|b| b.hi < r.lo) {
                    quad([[xa, y0, zb], [xb, y0, zb], [xb, y0, za], [xa, y0, za]], r.rgb, SHADE[1], 150, false);
                }
                // Sides, where the next column's solid runs do not cover this one.
                for (dx, dz, shade) in [(0i32, -1i32, SHADE[2]), (0, 1, SHADE[3]), (-1, 0, SHADE[4]), (1, 0, SHADE[5])] {
                    let next = at(cx + dx, cz + dz);
                    let mut y = r.lo;
                    let mut open: Vec<(i16, i16)> = Vec::new();
                    for q in next.iter().filter(|q| !q.water && q.hi > r.lo && q.lo < r.hi) {
                        if q.lo > y {
                            open.push((y, q.lo));
                        }
                        y = y.max(q.hi);
                    }
                    if y < r.hi {
                        open.push((y, r.hi));
                    }
                    for (a, b) in open {
                        let (ya, yb) = (a as f32, b as f32);
                        let p = match (dx, dz) {
                            (0, -1) => [[xb, yb, za], [xa, yb, za], [xa, ya, za], [xb, ya, za]],
                            (0, _) => [[xa, yb, zb], [xb, yb, zb], [xb, ya, zb], [xa, ya, zb]],
                            (-1, _) => [[xa, yb, za], [xa, yb, zb], [xa, ya, zb], [xa, ya, za]],
                            _ => [[xb, yb, zb], [xb, yb, za], [xb, ya, za], [xb, ya, zb]],
                        };
                        quad(p, r.rgb, shade, 230, false);
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Loading chunks around the camera
// ---------------------------------------------------------------------------------------------

type Key = (i32, i32);

enum Job {
    Load(Key),
    Mesh { key: Key, version: u32, me: Arc<Blocks>, near: [Option<Arc<Blocks>>; 4] },
    /// A chunk read and boiled down for the low-detail land, `usize` blocks a cell.
    LodRead(Key, usize),
    /// A low-detail tile made into a mesh from its chunks.
    Lod(Key, usize, Vec<Option<Arc<LodChunk>>>),
    /// The blocks a paste would put down, made into see-through meshes (by its version).
    Ghost(u64, Vec<(Key, Arc<Blocks>)>),
}

enum Done {
    Loaded(Key, Option<Blocks>),
    Meshed(Key, u32, MeshData),
    LodRead(Key, usize, Option<LodChunk>),
    Lod(Key, usize, MeshData),
    Ghost(u64, Vec<(Key, MeshData)>),
}

/// What the GPU is to take on its next frame: meshes to add (or replace), chunks to drop, or
/// everything to drop (a new world or Slicer).
#[derive(Default)]
pub struct Uploads {
    clear: bool,
    add: Vec<(Key, MeshData)>,
    remove: Vec<Key>,
    /// The low-detail land further out (by tile of `LOD_TILE` chunks).
    lod_add: Vec<(Key, MeshData)>,
    lod_remove: Vec<Key>,
    /// The paste preview's meshes, all at once (Some(empty): none any more).
    ghost: Option<Vec<(Key, MeshData)>>,
}

struct Shared {
    path: PathBuf,
    edition: Edition,
    dimension: String,
    slicer: Option<(i32, i32)>,
    /// The world opened (again, after an edit: see [`World::reload_chunks`]).
    source: Mutex<Option<Arc<Result<Source, String>>>>,
    atlas: Arc<Mutex<Atlas>>,
    /// The World Explorer's unsaved edits, read before the world.
    edits: Arc<std::sync::RwLock<crate::edit::EditLayer>>,
}

impl Shared {
    /// One chunk, as edited when the World Explorer changed it.
    fn load(&self, source: &Source, key: Key) -> Option<Blocks> {
        let edits = self.edits.read().unwrap_or_else(|e| e.into_inner());
        load(source, key.0, key.1, self.slicer, &edits, &self.dimension)
    }

    fn source(&self) -> Arc<Result<Source, String>> {
        let mut s = self.source.lock().unwrap_or_else(|e| e.into_inner());
        s.get_or_insert_with(|| Arc::new(open_source(&self.path, self.edition, &self.dimension))).clone()
    }
}

/// The chunks around the camera: read and made into meshes on worker threads, nearest first,
/// and handed to the GPU; those that fall out of range dropped again.
/// With `MCTK_PROFILE` set: how long reading (0), meshing (1) and lighting (2) chunks take on
/// average, printed every 50 of each.
fn profile(kind: usize, started: std::time::Instant) {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var_os("MCTK_PROFILE").is_some());
    static SUMS: Mutex<[(u32, f64); 3]> = Mutex::new([(0, 0.0); 3]);
    if !*ON {
        return;
    }
    if let Ok(mut s) = SUMS.lock() {
        let e = &mut s[kind];
        e.0 += 1;
        e.1 += started.elapsed().as_secs_f64() * 1000.0;
        if e.0 % 50 == 0 {
            eprintln!("profile: {} x{} avg {:.1} ms", ["read", "mesh", "light"][kind], e.0, e.1 / e.0 as f64);
        }
    }
}

pub struct World {
    shared: Arc<Shared>,
    tx: Option<Sender<Job>>,
    rx: Receiver<Done>,
    loaded: HashMap<Key, Arc<Blocks>>,
    absent: HashSet<Key>,
    loading: HashSet<Key>,
    /// Chunks waiting to be meshed (until their neighbors are read).
    pending: HashSet<Key>,
    gens: HashMap<Key, u32>,
    busy: usize,
    workers: usize,
    infos: HashMap<u16, Info>,
    /// Barriers shown (by their icon) and pointed at; else passed through as air.
    pub show_barriers: bool,
    /// The box around each block's shape, for pointing at it (see `shape_box`).
    shapes: HashMap<u16, Option<([f32; 3], [f32; 3])>>,
    /// The low-detail tiles asked for, with the blocks across their cells.
    lods: HashMap<Key, usize>,
    /// Chunks boiled down for the low-detail land, at the blocks a cell they were read for
    /// (`None`: nothing saved there); and those being read.
    lod_chunks: HashMap<Key, (usize, Option<Arc<LodChunk>>)>,
    lod_reading: HashSet<Key>,
    /// The chunk the real blocks are centerd on and how far they reach (chunks), for the GPU to
    /// leave the low-detail land out of those chunks.
    near_center: (f32, f32, f32),
    /// Lines in the world for this frame (the block outline, chunk walls, the selection), drawn
    /// by the GPU among the blocks.
    lines: Arc<Mutex<Vec<([f32; 3], [f32; 3], egui::Color32)>>>,
    pub uploads: Arc<Mutex<Uploads>>,
    atlas: Arc<Mutex<Atlas>>,
    /// This world's number among those made (for the GPU to tell whose chunks it holds), and
    /// set by the GPU when it dropped them for another world's.
    id: u64,
    lost: Arc<std::sync::atomic::AtomicBool>,
    /// The paste preview asked for last (older meshes coming in are not taken).
    ghost_version: u64,
    /// Chunks changed by the editing tools, to be read again (see refresh_chunks).
    reread: HashSet<Key>,
    /// The mesh version on the GPU for each chunk (a newer one is taken as it comes).
    shown: HashMap<Key, u32>,
}

impl World {
    pub fn new(path: PathBuf, edition: Edition, dimension: &str, slicer: Option<(i32, i32)>, edits: Arc<std::sync::RwLock<crate::edit::EditLayer>>) -> Self {
        // Leaves keep their gaps here (the depth buffer shows the leaves behind), as the game's
        // Fancy leaves.
        let atlas = Arc::new(Mutex::new({
            let mut a = Atlas::new();
            a.solid_leaves = false;
            a
        }));
        let shared = Arc::new(Shared { path, edition, dimension: dimension.to_string(), slicer, source: Mutex::new(None), atlas: atlas.clone(), edits });
        let (tx, jobs) = channel::<Job>();
        let (done, rx) = channel::<Done>();
        let jobs = Arc::new(Mutex::new(jobs));
        let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).saturating_sub(1).clamp(1, 6);
        for _ in 0..workers {
            let (jobs, done, shared) = (jobs.clone(), done.clone(), shared.clone());
            std::thread::spawn(move || {
                loop {
                    let job = match jobs.lock() {
                        Ok(j) => j.recv(),
                        Err(_) => return,
                    };
                    let Ok(job) = job else { return };
                    let started = std::time::Instant::now();
                    let result = match job {
                        Job::Load(key) => {
                            let source = shared.source();
                            let blocks = source.as_ref().as_ref().ok().and_then(|s| shared.load(s, key));
                            profile(0, started);
                            Done::Loaded(key, blocks)
                        }
                        Job::Mesh { key, version, me, near } => {
                            let m = mesh(key.0, key.1, &me, &near, &shared.atlas);
                            profile(1, started);
                            Done::Meshed(key, version, m)
                        }
                        Job::LodRead(key, cell) => {
                            let source = shared.source();
                            let blocks = source.as_ref().as_ref().ok().and_then(|s| shared.load(s, key));
                            Done::LodRead(key, cell, blocks.map(|b| lod_chunk(&b, cell)))
                        }
                        Job::Lod(key, cell, chunks) => Done::Lod(key, cell, lod_mesh(key, cell, &chunks)),
                        Job::Ghost(version, chunks) => Done::Ghost(version, ghost_meshes(&chunks, &shared.atlas)),
                    };
                    if done.send(result).is_err() {
                        return;
                    }
                }
            });
        }
        let uploads = Arc::new(Mutex::new(Uploads { clear: true, ..Default::default() }));
        World {
            shared,
            tx: Some(tx),
            rx,
            loaded: HashMap::new(),
            absent: HashSet::new(),
            loading: HashSet::new(),
            pending: HashSet::new(),
            gens: HashMap::new(),
            busy: 0,
            workers,
            infos: HashMap::new(),
            show_barriers: true,
            shapes: HashMap::new(),
            lods: HashMap::new(),
            lod_chunks: HashMap::new(),
            lod_reading: HashSet::new(),
            near_center: (0.0, 0.0, 0.0),
            lines: Arc::new(Mutex::new(Vec::new())),
            uploads,
            atlas,
            id: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
            lost: Arc::default(),
            ghost_version: 0,
            reread: HashSet::new(),
            shown: HashMap::new(),
        }
    }

    /// After the GPU dropped this world's chunks for another's: its pictures and every chunk and
    /// low-detail tile it had sent, sent again.
    fn resend_if_lost(&mut self) {
        if !self.lost.swap(false, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        if let Ok(mut a) = self.atlas.lock() {
            a.resend_all();
        }
        if let Ok(mut up) = self.uploads.lock() {
            // What was waiting went with the rest (the GPU cleared after taking it).
            up.remove.clear();
            up.lod_remove.clear();
        }
        self.shown.clear();
        let keys: Vec<Key> = self.loaded.keys().copied().collect();
        for k in keys {
            self.remesh(k);
        }
        // Asked for again by `update_lod` (their chunks are still at hand).
        self.lods.clear();
    }

    /// Whether this was made for the same world, dimension and Slicer.
    pub fn is_for(&self, path: &std::path::Path, dimension: &str, slicer: Option<(i32, i32)>) -> bool {
        self.shared.path == path && self.shared.dimension == dimension && self.shared.slicer == slicer
    }

    fn send(&mut self, job: Job) {
        if let Some(tx) = &self.tx {
            if tx.send(job).is_ok() {
                self.busy += 1;
            }
        }
    }

    /// After chunks were changed on disk (an edit saved): the world opened afresh and those
    /// chunks read again (their old meshes drawn till the new ones come).
    pub fn reload_chunks(&mut self, keys: &[Key]) {
        if let Ok(mut s) = self.shared.source.lock() {
            *s = None;
        }
        for k in keys {
            self.loaded.remove(k);
            self.absent.remove(k);
            self.pending.remove(k);
            // A mesh of the old chunk still in the works is not taken.
            *self.gens.entry(*k).or_insert(0) += 1;
        }
    }

    /// Shows (or, with none, takes away) the paste preview: these blocks, see-through. Made into
    /// meshes on a worker; a newer preview asked for meanwhile wins.
    pub fn set_ghost(&mut self, chunks: Vec<(Key, Blocks)>) {
        self.ghost_version += 1;
        if chunks.is_empty() {
            if let Ok(mut up) = self.uploads.lock() {
                up.ghost = Some(Vec::new());
            }
            return;
        }
        let chunks = chunks.into_iter().map(|(k, b)| (k, Arc::new(b))).collect();
        self.send(Job::Ghost(self.ghost_version, chunks));
    }

    /// After the World Explorer changed chunks in memory: those chunks read again through its
    /// edits (the world stays open), and the low-detail land over them made again.
    pub fn refresh_chunks(&mut self, keys: &[Key]) {
        // Read again, while what is drawn stays till the new mesh comes (a brush stroke
        // changes a chunk again and again: each newer mesh is shown as it comes). A chunk being
        // read already is read once more after.
        for k in keys {
            self.absent.remove(k);
            self.reread.insert(*k);
            self.lod_chunks.remove(k);
        }
        // The tiles holding them are asked for again.
        self.lods.retain(|tile, _| !keys.iter().any(|k| (k.0.div_euclid(LOD_TILE), k.1.div_euclid(LOD_TILE)) == *tile));
    }

    fn remesh(&mut self, key: Key) {
        let Some(me) = self.loaded.get(&key).cloned() else { return };
        let version = self.gens.entry(key).or_insert(0);
        *version += 1;
        let version = *version;
        let near = [(-1, 0), (1, 0), (0, -1), (0, 1)].map(|(dx, dz)| self.loaded.get(&(key.0 + dx, key.1 + dz)).cloned());
        self.send(Job::Mesh { key, version, me, near });
    }

    /// Takes in what the workers finished and asks for the chunks within `radius` of `at` (in
    /// blocks) not read yet, nearest first; drops those well out of range. Asks egui to draw
    /// again while there is work going on.
    pub fn update(&mut self, ctx: &egui::Context, at: (f32, f32), radius: i32) {
        self.resend_if_lost();
        while let Ok(done) = self.rx.try_recv() {
            self.busy = self.busy.saturating_sub(1);
            match done {
                Done::Loaded(key, Some(blocks)) => {
                    self.loading.remove(&key);
                    self.loaded.insert(key, Arc::new(blocks));
                    self.pending.insert(key);
                    // Its neighbors' faces toward it may now be hidden (and their light changes).
                    for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        let k = (key.0 + dx, key.1 + dz);
                        if self.loaded.contains_key(&k) {
                            self.pending.insert(k);
                        }
                    }
                }
                Done::Loaded(key, None) => {
                    self.loading.remove(&key);
                    self.absent.insert(key);
                    // A chunk that was drawn and is gone now (deleted): its mesh goes, and its
                    // neighbors' faces toward it show again.
                    if self.shown.remove(&key).is_some() || self.loaded.remove(&key).is_some() {
                        if let Ok(mut up) = self.uploads.lock() {
                            up.remove.push(key);
                        }
                        for (dx, dz) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                            let k = (key.0 + dx, key.1 + dz);
                            if self.loaded.contains_key(&k) {
                                self.pending.insert(k);
                            }
                        }
                    }
                }
                Done::LodRead(key, cell, data) => {
                    self.lod_reading.remove(&key);
                    self.lod_chunks.insert(key, (cell, data.map(Arc::new)));
                }
                Done::Ghost(version, meshes) => {
                    if version == self.ghost_version {
                        if let Ok(mut up) = self.uploads.lock() {
                            up.ghost = Some(meshes);
                        }
                    }
                }
                Done::Lod(key, cell, data) => {
                    if self.lods.get(&key) == Some(&cell) {
                        if let Ok(mut up) = self.uploads.lock() {
                            up.lod_add.push((key, data));
                        }
                    }
                }
                Done::Meshed(key, version, data) => {
                    if self.loaded.contains_key(&key) && self.shown.get(&key).is_none_or(|v| version > *v) {
                        self.shown.insert(key, version);
                        if let Ok(mut up) = self.uploads.lock() {
                            up.add.push((key, data));
                        }
                    }
                }
            }
        }
        let (ccx, ccz) = ((at.0 / 16.0).floor() as i32, (at.1 / 16.0).floor() as i32);
        // Chunks to mesh (again): each once all its neighbors that will be read have been
        // (read, found empty, or out of range), so a chunk is meshed about once rather than
        // again as each neighbor comes in.
        if !self.pending.is_empty() {
            let settled = |k: Key| self.loaded.contains_key(&k) || self.absent.contains(&k) || (k.0 - ccx).pow(2) + (k.1 - ccz).pow(2) > radius * radius;
            let ready: Vec<Key> = self.pending.iter().copied().filter(|k| [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().all(|(dx, dz)| settled((k.0 + dx, k.1 + dz)))).collect();
            for k in ready {
                self.pending.remove(&k);
                self.remesh(k);
            }
            self.pending.retain(|k| self.loaded.contains_key(k));
        }
        // Chunks the editing tools changed: read again (once each read in flight is back).
        let again: Vec<Key> = self.reread.iter().copied().filter(|k| !self.loading.contains(k)).collect();
        for k in again {
            self.reread.remove(&k);
            self.loading.insert(k);
            self.send(Job::Load(k));
        }
        let mut wanted: Vec<(i32, Key)> = Vec::new();
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let d = dx * dx + dz * dz;
                if d > radius * radius {
                    continue;
                }
                let key = (ccx + dx, ccz + dz);
                if !self.loaded.contains_key(&key) && !self.absent.contains(&key) && !self.loading.contains(&key) {
                    wanted.push((d, key));
                }
            }
        }
        wanted.sort_unstable();
        let room = (self.workers * 3).saturating_sub(self.busy);
        for (_, key) in wanted.into_iter().take(room) {
            self.loading.insert(key);
            self.send(Job::Load(key));
        }
        // Out of range (with a margin, so moving back and forth does not read them again).
        let keep = (radius + 3) * (radius + 3);
        let far: Vec<Key> = self.loaded.keys().copied().filter(|k| (k.0 - ccx).pow(2) + (k.1 - ccz).pow(2) > keep).collect();
        if !far.is_empty() {
            if let Ok(mut up) = self.uploads.lock() {
                for k in &far {
                    up.remove.push(*k);
                }
            }
            for k in far {
                self.loaded.remove(&k);
                self.gens.remove(&k);
                self.shown.remove(&k);
            }
        }
        self.absent.retain(|k| (k.0 - ccx).pow(2) + (k.1 - ccz).pow(2) <= keep);
        if self.busy > 0 || !self.pending.is_empty() {
            ctx.request_repaint();
        }
    }

    /// The low-detail land from `radius` chunks out to [`LOD_REACH`]: the chunks there read and
    /// boiled down on the workers (when they are not busy with the real blocks), nearest first,
    /// coarser further out; each tile of `LOD_TILE` x `LOD_TILE` chunks made into a mesh once all
    /// its chunks are in (the old mesh stays drawn till then); those out of reach dropped. The
    /// real blocks' circle is left out of it on the GPU.
    pub fn update_lod(&mut self, at: (f32, f32), radius: i32) {
        let (ccx, ccz) = ((at.0 / 16.0).floor() as i32, (at.1 / 16.0).floor() as i32);
        // The blocks reach the circle of chunks loaded around the camera's chunk.
        self.near_center = (ccx as f32, ccz as f32, radius as f32);
        let (tcx, tcz) = (ccx.div_euclid(LOD_TILE), ccz.div_euclid(LOD_TILE));
        let lod_reach = LOD_REACH.max(radius + 48);
        let reach = lod_reach / LOD_TILE + 1;
        // Tiles wanted (nearest first) and the detail each wants.
        let mut tiles: Vec<(i32, Key, usize)> = Vec::new();
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let key = (tcx + dx, tcz + dz);
                // The tile's middle in chunks from the camera's chunk.
                let (mx, mz) = ((key.0 * LOD_TILE + LOD_TILE / 2 - ccx) as f32, (key.1 * LOD_TILE + LOD_TILE / 2 - ccz) as f32);
                let d = (mx * mx + mz * mz).sqrt();
                // Wholly inside the blocks' circle: not needed.
                if d + LOD_TILE as f32 * 0.75 < radius as f32 || d > lod_reach as f32 {
                    continue;
                }
                let past = d - radius as f32;
                let cell = match self.lods.get(&key) {
                    // Kept at its detail near the line between two, so a tile on the line does
                    // not flip back and forth.
                    Some(&c) if lod_cell(past + 2.0) == c || lod_cell(past - 2.0) == c => c,
                    _ => lod_cell(past),
                };
                if self.lods.get(&key) != Some(&cell) {
                    tiles.push(((d * 10.0) as i32, key, cell));
                }
            }
        }
        tiles.sort_unstable();
        // Reads only with the workers' room to spare (the real blocks come first), and meshes
        // for the tiles whose chunks are all in.
        let mut room = (self.workers * 2).saturating_sub(self.busy + self.loading.len());
        let mut meshes = 0;
        for (_, key, cell) in tiles {
            let mut ready = true;
            let mut chunks = Vec::with_capacity((LOD_TILE * LOD_TILE) as usize);
            for z in 0..LOD_TILE {
                for x in 0..LOD_TILE {
                    let ck = (key.0 * LOD_TILE + x, key.1 * LOD_TILE + z);
                    match self.lod_chunks.get(&ck) {
                        Some((c, data)) if *c == cell => chunks.push(data.clone()),
                        _ => {
                            ready = false;
                            if room > 0 && !self.lod_reading.contains(&ck) {
                                self.lod_reading.insert(ck);
                                self.send(Job::LodRead(ck, cell));
                                room -= 1;
                            }
                        }
                    }
                }
            }
            if ready && meshes < 4 {
                self.lods.insert(key, cell);
                if chunks.iter().all(Option::is_none) {
                    if let Ok(mut up) = self.uploads.lock() {
                        up.lod_remove.push(key);
                    }
                } else {
                    self.send(Job::Lod(key, cell, chunks));
                    meshes += 1;
                }
            }
        }
        // Out of reach (with a margin).
        let far_tile = |k: &Key| {
            let (mx, mz) = ((k.0 * LOD_TILE + LOD_TILE / 2 - ccx) as f32, (k.1 * LOD_TILE + LOD_TILE / 2 - ccz) as f32);
            (mx * mx + mz * mz).sqrt() > (lod_reach + LOD_TILE * 2) as f32
        };
        let far: Vec<Key> = self.lods.keys().copied().filter(|k| far_tile(k)).collect();
        if !far.is_empty() {
            if let Ok(mut up) = self.uploads.lock() {
                up.lod_remove.extend(far.iter().copied());
            }
            for k in far {
                self.lods.remove(&k);
            }
        }
        self.lod_chunks.retain(|k, _| !far_tile(&(k.0.div_euclid(LOD_TILE), k.1.div_euclid(LOD_TILE))));
    }

    /// The lines in the world for this frame (see `street_view::gather_lines`).
    pub fn set_lines(&self, lines: Vec<([f32; 3], [f32; 3], egui::Color32)>) {
        if let Ok(mut l) = self.lines.lock() {
            *l = lines;
        }
    }

    /// The block (as a map block number) at a spot, where its chunk is loaded.
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> Option<u16> {
        let b = self.loaded.get(&(x.div_euclid(16), z.div_euclid(16)))?;
        Some(b.get(x.rem_euclid(16) as usize, y, z.rem_euclid(16) as usize))
    }

    /// The box around a block's shape, in the block (0 to 1 a side; `None` for air and water):
    /// the game's model's box, the box around what is drawn for a chest, head or the like, else
    /// the shape guessed from its name. Once worked out per block.
    fn shape_box(&mut self, id: u16) -> Option<([f32; 3], [f32; 3])> {
        if let Some(b) = self.shapes.get(&id) {
            return *b;
        }
        let info = *self.infos.entry(id).or_insert_with(|| info_of(id));
        let found = shape_box_of(id, info);
        self.shapes.insert(id, found);
        found
    }
}

/// See [`World::shape_box`].
fn shape_box_of(id: u16, info: Info) -> Option<([f32; 3], [f32; 3])> {
    if matches!(info.shape, Shape::Air | Shape::Liquid { .. }) {
        return None;
    }
    // Drawn in code (chests, heads, banners...): the box around what is drawn.
    if info.entity {
        let drawn = world::block_name(id).and_then(|name| {
            let base = name.split(['#', '@']).next().unwrap_or(&name).to_string();
            crate::entities::block_entity(&base, world::block_props(id).as_deref().unwrap_or(""), None)
        });
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for q in drawn.iter().flatten() {
            for c in q.pos {
                for k in 0..3 {
                    lo[k] = lo[k].min(c[k]);
                    hi[k] = hi[k].max(c[k]);
                }
            }
        }
        if lo[0] <= hi[0] {
            return Some((lo.map(|v| v.clamp(-1.0, 2.0)), hi.map(|v| v.clamp(-1.0, 2.0))));
        }
    }
    // The game's model for its state (signs keep a block entity for their text but are drawn
    // from their model, so whatever `info` says).
    if let Some(b) = baked_of(id) {
        if b.lo[0] <= b.hi[0] {
            return Some((b.lo, [b.hi[0], b.hi[1].max(b.lo[1] + 0.02), b.hi[2]]));
        }
    }
    match info.shape {
        Shape::Air | Shape::Liquid { .. } => None,
        Shape::Cross => Some(([0.2, 0.0, 0.2], [0.8, 0.8, 0.8])),
        Shape::Post(w, h) => Some(([0.5 - w, 0.0, 0.5 - w], [0.5 + w, h, 0.5 + w])),
        Shape::Decal => Some(([0.0; 3], [1.0, 0.06, 1.0])),
        Shape::Cube(h) => Some(([0.0; 3], [1.0, h, 1.0])),
    }
}

/// Where along a ray from `eye` along `dir` it enters the box `lo`..`hi` (`None`: it misses).
fn ray_box_t(eye: [f32; 3], dir: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> Option<f32> {
    let (mut near, mut far) = (0.0f32, f32::INFINITY);
    for k in 0..3 {
        if dir[k].abs() < 1e-9 {
            if eye[k] < lo[k] || eye[k] > hi[k] {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo[k] - eye[k]) / dir[k], (hi[k] - eye[k]) / dir[k]);
        near = near.max(a.min(b));
        far = far.min(a.max(b));
    }
    (near <= far).then_some(near)
}

/// An entity's box, as the game sizes it: half its width and its height (blocks), roughly.
pub fn entity_size(e: &mctk_nbt::Tag) -> (f32, f32) {
    let id = e.get("id").and_then(mctk_nbt::Tag::as_str).unwrap_or("").trim_start_matches("minecraft:");
    let baby = e.get("IsBaby").and_then(mctk_nbt::Tag::as_i64).unwrap_or(0) != 0 || e.get("Age").and_then(mctk_nbt::Tag::as_i64).is_some_and(|a| a < 0);
    let (w, h): (f32, f32) = match id {
        "zombie" | "husk" | "drowned" | "skeleton" | "stray" | "bogged" | "parched" | "zombified_piglin" | "piglin" | "piglin_brute" | "player" | "villager" | "zombie_villager" | "wandering_trader" | "witch" | "pillager" | "vindicator" | "evoker" | "illusioner" | "creaking" => (0.6, 1.95),
        "wither_skeleton" => (0.7, 2.4),
        "enderman" => (0.6, 2.9),
        "creeper" => (0.6, 1.7),
        "armor_stand" => (0.5, 1.975),
        "iron_golem" => (1.4, 2.7),
        "snow_golem" => (0.7, 1.9),
        "cow" | "mooshroom" | "pig" | "sheep" | "goat" => (0.9, 1.4),
        "horse" | "donkey" | "mule" | "skeleton_horse" | "zombie_horse" | "camel" => (1.4, 1.6),
        "chicken" | "parrot" => (0.4, 0.7),
        "cat" | "ocelot" | "rabbit" | "fox" | "wolf" | "armadillo" => (0.6, 0.85),
        "spider" => (1.4, 0.9),
        "cave_spider" => (0.7, 0.5),
        "cod" | "salmon" | "pufferfish" | "tropical_fish" | "bat" | "bee" | "allay" | "vex" | "silverfish" | "endermite" | "frog" | "tadpole" | "axolotl" => (0.5, 0.5),
        "slime" | "magma_cube" => {
            let s = (e.get("Size").and_then(mctk_nbt::Tag::as_i64).unwrap_or(0) + 1) as f32 * 0.52;
            (s, s)
        }
        "ghast" | "happy_ghast" => (4.0, 4.0),
        "item" => (0.25, 0.25),
        _ => (0.8, 1.2),
    };
    let k = if baby { 0.5 } else { 1.0 };
    (w * k / 2.0, h * k)
}

impl World {
    /// The saved data of the block entity at a spot (a chest's items...), where loaded.
    pub fn block_entity_at(&self, x: i32, y: i32, z: i32) -> Option<Arc<mctk_nbt::Tag>> {
        let b = self.loaded.get(&(x.div_euclid(16), z.div_euclid(16)))?;
        b.extras.get(&(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)).cloned()
    }

    /// The entity under a point on the screen, nearer than `before` blocks along the pointer's
    /// ray (the block it is on), with how far it is.
    pub fn pick_entity(&self, cam: &Camera, at: Pos2, before: f32) -> Option<Arc<mctk_nbt::Tag>> {
        let (eye, dir) = cam.ray(at);
        let mut best: Option<(f32, Arc<mctk_nbt::Tag>)> = None;
        for b in self.loaded.values() {
            for e in &b.entities {
                let Some(p) = entity_pos(e) else { continue };
                let (r, h) = entity_size(e);
                if let Some(t) = ray_box_t(eye, dir, [p[0] - r, p[1], p[2] - r], [p[0] + r, p[1] + h, p[2] + r]) {
                    if t < before && best.as_ref().is_none_or(|(bt, _)| t < *bt) {
                        best = Some((t, e.clone()));
                    }
                }
            }
        }
        best.map(|(_, e)| e)
    }

    /// The barriers within `reach` blocks of `at` that can be seen from `eye` (nothing but air,
    /// see-through blocks and other barriers between: a barrier shows through a barrier, as
    /// glass does), nearest last (so the nearest is drawn on top). At most `most`.
    pub fn barriers_near(&mut self, eye: [f32; 3], at: [f32; 3], reach: f32, most: usize) -> Vec<[i32; 3]> {
        let mut found: Vec<(f32, [i32; 3])> = Vec::new();
        let (cx, cz) = ((at[0] / 16.0).floor() as i32, (at[2] / 16.0).floor() as i32);
        let r = (reach / 16.0).ceil() as i32;
        for dz in -r..=r {
            for dx in -r..=r {
                let Some(b) = self.loaded.get(&(cx + dx, cz + dz)) else { continue };
                for p in &b.barriers {
                    let c = [p[0] as f32 + 0.5, p[1] as f32 + 0.5, p[2] as f32 + 0.5];
                    let d2 = (0..3).map(|k| (c[k] - at[k]).powi(2)).sum::<f32>();
                    if d2 <= reach * reach {
                        found.push(((0..3).map(|k| (c[k] - eye[k]).powi(2)).sum::<f32>(), *p));
                    }
                }
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.truncate(most * 2);
        let mut seen = Vec::new();
        for (_, p) in found {
            if seen.len() >= most {
                break;
            }
            if self.clear_between(eye, [p[0] as f32 + 0.5, p[1] as f32 + 0.5, p[2] as f32 + 0.5], p) {
                seen.push(p);
            }
        }
        seen.reverse();
        seen
    }

    /// Whether nothing that hides what is behind it (a whole, opaque block) is between `from`
    /// and the block `to` (whose middle is `end`).
    fn clear_between(&mut self, from: [f32; 3], end: [f32; 3], to: [i32; 3]) -> bool {
        let d = [end[0] - from[0], end[1] - from[1], end[2] - from[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let steps = (len * 3.0).ceil() as i32;
        let mut last = [i32::MIN; 3];
        for s in 1..steps {
            let t = s as f32 / steps as f32;
            let q = [(from[0] + d[0] * t).floor() as i32, (from[1] + d[1] * t).floor() as i32, (from[2] + d[2] * t).floor() as i32];
            if q == last || q == to {
                continue;
            }
            last = q;
            if let Some(id) = self.block_at(q[0], q[1], q[2]) {
                if id != 0 && self.infos.entry(id).or_insert_with(|| info_of(id)).solid {
                    return false;
                }
            }
        }
        true
    }

    /// The entities with names in view (where their names show), and the names.
    pub fn named_entities(&self) -> Vec<([f32; 3], String)> {
        let mut out = Vec::new();
        for b in self.loaded.values() {
            for e in &b.entities {
                let Some(c) = e.as_compound() else { continue };
                let Some(name) = crate::naming::mob_name(c) else { continue };
                let Some(p) = entity_pos(e) else { continue };
                let (_, h) = entity_size(e);
                out.push(([p[0], p[1] + h + 0.3, p[2]], name));
            }
        }
        out
    }
}

/// Where an entity stands (its `Pos`).
fn entity_pos(e: &mctk_nbt::Tag) -> Option<[f32; 3]> {
    let l = e.get("Pos")?.as_list()?;
    let n = |i: usize| l.get(i).and_then(mctk_nbt::Tag::as_f64).map(|v| v as f32);
    Some([n(0)?, n(1)?, n(2)?])
}

/// How far along the pointer's ray a block hit is (for an entity to be in front of it or not).
pub fn hit_distance(cam: &Camera, at: Pos2, hit: &Hit) -> f32 {
    let (eye, dir) = cam.ray(at);
    let (x, z) = (hit.x as f32, hit.z as f32);
    ray_box_t(eye, dir, [x + hit.xz[0], hit.y0, z + hit.xz[2]], [x + hit.xz[1], hit.y1, z + hit.xz[3]]).unwrap_or(f32::INFINITY)
}

impl World {
    /// The block under a point on the screen: the first block along the ray from the eye (as a
    /// walk through the blocks), water passed through.
    /// With `liquids`, water and lava stop the ray too (as whole blocks).
    pub fn pick_with(&mut self, cam: &Camera, at: Pos2, liquids: bool) -> Option<Hit> {
        let (eye, dir) = cam.ray(at);
        let mut p = [eye[0].floor() as i32, eye[1].floor() as i32, eye[2].floor() as i32];
        let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
        let next = |k: usize| {
            if dir[k].abs() < 1e-9 {
                f32::INFINITY
            } else if dir[k] > 0.0 {
                (p[k] as f32 + 1.0 - eye[k]) / dir[k]
            } else {
                (p[k] as f32 - eye[k]) / dir[k]
            }
        };
        let mut t = [next(0), next(1), next(2)];
        let delta = dir.map(|d| if d.abs() < 1e-9 { f32::INFINITY } else { 1.0 / d.abs() });
        for _ in 0..1200 {
            if let Some(id) = self.block_at(p[0], p[1], p[2]) {
                if id != 0 {
                    // Outlined around the block's own shape, which the pointer must be on (past
                    // a button or a head on a wall, the ray goes on to what is behind); water
                    // and lava as whole blocks when they are to be picked too.
                    let liquid = liquids && matches!(self.infos.entry(id).or_insert_with(|| info_of(id)).shape, Shape::Liquid { .. });
                    // A barrier, a whole block while barriers are shown.
                    let barrier = is_barrier(id);
                    let shape = if liquid || (barrier && self.show_barriers) {
                        Some(([0.0; 3], [1.0; 3]))
                    } else if barrier {
                        None
                    } else {
                        self.shape_box(id)
                    };
                    if let Some((lo, hi)) = shape {
                        let o = [p[0] as f32, p[1] as f32, p[2] as f32];
                        let (a, b) = ([o[0] + lo[0], o[1] + lo[1], o[2] + lo[2]], [o[0] + hi[0], o[1] + hi[1], o[2] + hi[2]]);
                        if ray_hits_box(eye, dir, a, b) {
                            return Some(Hit { x: p[0], y: p[1], z: p[2], y0: a[1], y1: b[1], xz: [lo[0], hi[0], lo[2], hi[2]] });
                        }
                    }
                }
            }
            let k = if t[0] < t[1] { if t[0] < t[2] { 0 } else { 2 } } else if t[1] < t[2] { 1 } else { 2 };
            p[k] += step[k];
            t[k] += delta[k];
        }
        None
    }

    /// Whether there is room to draw (a window shrunk to nothing, minimized, has none: drawing
    /// into it is what the GPU refuses).
    pub fn can_paint(cam: &Camera) -> bool {
        cam.rect.width() >= 2.0 && cam.rect.height() >= 2.0 && cam.rect.is_finite()
    }

    /// The paint callback for this frame: the camera, and what the GPU is to take on.
    /// With `full_bright` everything is evenly lit (to see into dark places), else lit by the
    /// sky and the glowing blocks.
    pub fn paint(&self, cam: &Camera, time: f64, full_bright: bool, daylight: f32) -> egui::PaintCallback {
        egui_wgpu::Callback::new_paint_callback(
            cam.rect,
            Paint { owner: self.id, lost: self.lost.clone(), view_proj: cam.view_proj(), uploads: self.uploads.clone(), atlas: self.atlas.clone(), time, near: self.near_center, lines: self.lines.clone(), full_bright, daylight },
        )
    }
}

impl Drop for World {
    fn drop(&mut self) {
        // Closing the job channel lets the workers end.
        self.tx = None;
    }
}

// ---------------------------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------------------------

const SHADER: &str = r#"
struct U { view_proj: mat4x4<f32>, near: vec4<f32>, look: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var pics: texture_2d<f32>;
@group(0) @binding(2) var pick: sampler;

struct V {
    // (The same corners give exactly the same depth in every pipeline: inverted lines rely on it.)
    @builtin(position) @invariant pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) world: vec3<f32>,
    @location(3) bright: f32,
};

@vertex
fn vs(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) color: vec4<f32>, @location(3) light: vec4<f32>) -> V {
    var o: V;
    o.pos = u.view_proj * vec4<f32>(p, 1.0);
    o.uv = uv;
    o.color = color;
    o.world = p;
    // How bright the corner is: its light as the game dims it (a step of light about a fifth
    // darker each), unless Full Bright (look.x); and darker the more it is tucked in among blocks.
    // The sky's light dimmed by the time of day (look.y: 1 at noon), the glowing blocks' not.
    let lit = max(light.x * u.look.y, light.y);
    let level = select(pow(0.8, (1.0 - lit) * 15.0), 1.0, u.look.x > 0.5);
    o.bright = max(level, 0.04) * (0.55 + 0.45 * light.z);
    return o;
}

@fragment
fn fs_solid(i: V) -> @location(0) vec4<f32> {
    let t = textureSample(pics, pick, i.uv);
    if (t.a < 0.5) {
        discard;
    }
    return vec4<f32>(t.rgb * i.color.rgb * i.bright, 1.0);
}

@fragment
fn fs_line(i: V) -> @location(0) vec4<f32> {
    return vec4<f32>(i.color.rgb * i.color.a, i.color.a);
}

@fragment
fn fs_lod(i: V) -> @location(0) vec4<f32> {
    // Where the real blocks are, the low-detail land is left out.
    // (By chunk, as the real blocks are loaded: the chunks within the reach of the camera's.)
    let d = floor(i.world.xz / 16.0) - u.near.xy;
    if (dot(d, d) <= u.near.z * u.near.z) {
        discard;
    }
    // Each block's color, lit as the real blocks (by the time of day too).
    return vec4<f32>(i.color.rgb * i.bright, 1.0);
}

@fragment
fn fs_lod_water(i: V) -> @location(0) vec4<f32> {
    // (By chunk, as the real blocks are loaded: the chunks within the reach of the camera's.)
    let d = floor(i.world.xz / 16.0) - u.near.xy;
    if (dot(d, d) <= u.near.z * u.near.z) {
        discard;
    }
    let a = i.color.a;
    return vec4<f32>(i.color.rgb * i.bright * a, a);
}

@fragment
fn fs_ghost(i: V) -> @location(0) vec4<f32> {
    let t = textureSample(pics, pick, i.uv);
    if (t.a < 0.1) { discard; }
    let a = 0.55;
    let c = mix(t.rgb * i.color.rgb, vec3<f32>(0.35, 0.85, 1.0), 0.3);
    return vec4<f32>(c * a, a);
}

@fragment
fn fs_water(i: V) -> @location(0) vec4<f32> {
    let t = textureSample(pics, pick, i.uv);
    let a = i.color.a * t.a;
    return vec4<f32>(t.rgb * i.color.rgb * i.bright * a, a);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniform {
    view_proj: [[f32; 4]; 4],
    /// The real blocks' center chunk and reach: chunk x, chunk z, chunks, unused.
    near: [f32; 4],
    /// Full Bright (1) or lit by the light (0), how bright the sky is for the time of day (1 at
    /// noon), and unused.
    look: [f32; 4],
}

struct GpuChunk {
    vb: wgpu::Buffer,
    ib: wgpu::Buffer,
    solid: u32,
    water: u32,
    lo: f32,
    hi: f32,
}

/// The GPU world's pipelines, block texture and chunk meshes, kept with egui's renderer.
struct Res {
    solid: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    lod: wgpu::RenderPipeline,
    lod_water: wgpu::RenderPipeline,
    lods: HashMap<Key, GpuChunk>,
    /// The paste preview: see-through blocks.
    ghost_pipe: wgpu::RenderPipeline,
    ghost: Vec<(Key, GpuChunk)>,
    line: wgpu::RenderPipeline,
    line_mark: wgpu::RenderPipeline,
    line_invert: wgpu::RenderPipeline,
    /// This frame's lines, and how many corners; and those drawn inverted.
    lines: Option<(wgpu::Buffer, u32)>,
    inverted: Option<(wgpu::Buffer, u32)>,
    uniform: wgpu::Buffer,
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
    chunks: HashMap<Key, GpuChunk>,
    /// Which world's chunks and pictures these are (see `World::id`): one set is kept for all
    /// open worlds, so another world drawing here takes it over.
    owner: u64,
}

impl Res {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gpu-world"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu-world"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gpu-world"), bind_group_layouts: &[&layout], push_constant_ranges: &[] });
        let vertex = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vert>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Unorm8x4, 3 => Unorm8x4],
        };
        let depth_format = egui_wgpu::depth_format_from_bits(24, 0).unwrap_or(wgpu::TextureFormat::Depth24Plus);
        let pipeline_masked = |entry: &str, write: bool, blend: Option<wgpu::BlendState>, topology: wgpu::PrimitiveTopology, compare: wgpu::CompareFunction, write_mask: wgpu::ColorWrites| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("gpu-world"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[vertex.clone()], compilation_options: Default::default() },
                // Faces seen from behind are left out (the land's tops seen from under it, from
                // a camera in a cave); see-through water and lines are seen from both sides.
                primitive: wgpu::PrimitiveState {
                    topology,
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: matches!(entry, "fs_solid" | "fs_lod").then_some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth_format,
                    depth_write_enabled: write,
                    depth_compare: compare,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask })],
                    compilation_options: Default::default(),
                }),
                multiview: None,
                cache: None,
            })
        };
        let pipeline_for = |entry: &str, write: bool, blend: Option<wgpu::BlendState>, topology: wgpu::PrimitiveTopology, compare: wgpu::CompareFunction| {
            pipeline_masked(entry, write, blend, topology, compare, wgpu::ColorWrites::ALL)
        };
        let pipeline = |entry: &str, write: bool, blend: Option<wgpu::BlendState>| pipeline_for(entry, write, blend, wgpu::PrimitiveTopology::TriangleList, wgpu::CompareFunction::Less);
        let solid = pipeline("fs_solid", true, None);
        // Lines in the world: hidden behind blocks, drawn on their surfaces.
        let line = pipeline_for("fs_line", false, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING), wgpu::PrimitiveTopology::LineList, wgpu::CompareFunction::LessEqual);
        // Lines that show the colors behind them inverted, as the game's crosshair does: seen
        // on any ground, light or dark.
        let invert = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::OneMinusDst, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent::OVER,
        };
        // Inverting twice gives back what was there, so where lines cross (or meet end to end)
        // they would leave holes: first each pixel's nearest line goes into the depth buffer
        // (nothing drawn), then a pixel is inverted only by the line whose depth is the one kept:
        // once, however many lines cover it.
        let line_mark = pipeline_masked("fs_line", true, None, wgpu::PrimitiveTopology::LineList, wgpu::CompareFunction::LessEqual, wgpu::ColorWrites::empty());
        let line_invert = pipeline_for("fs_line", false, Some(invert), wgpu::PrimitiveTopology::LineList, wgpu::CompareFunction::Equal);
        let water = pipeline("fs_water", false, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING));
        let lod = pipeline("fs_lod", true, None);
        let lod_water = pipeline("fs_lod_water", false, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING));
        let ghost_pipe = pipeline("fs_ghost", false, Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING));
        let uniform = device.create_buffer(&wgpu::BufferDescriptor { label: Some("gpu-world"), size: std::mem::size_of::<Uniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let size = crate::street_view::SIZE as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpu-world-blocks"),
            size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
            mip_level_count: MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gpu-world"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu-world"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        Res { solid, water, lod, lod_water, lods: HashMap::new(), ghost_pipe, ghost: Vec::new(), line, line_mark, line_invert, lines: None, inverted: None, uniform, texture, bind, chunks: HashMap::new(), owner: 0 }
    }

    /// Puts pixels in the block texture at (x, y), and the smaller copies of them in its mip
    /// levels (see [`mip_levels`]; the pictures sit in cells of 16, so no level mixes two).
    fn write_pixels(&self, queue: &wgpu::Queue, x: usize, y: usize, w: usize, h: usize, px: &[u8]) {
        let (mut x, mut y) = (x, y);
        for (mip, (w, h, level)) in mip_levels(px, w, h).into_iter().enumerate() {
            self.write_level(queue, mip as u32, x, y, w, h, &level);
            (x, y) = (x / 2, y / 2);
        }
    }
}

/// A picture and its smaller copies (up to [`MIPS`] levels), each half the last across. Colors
/// come from the pixels that show only (the clear ones are black in the game's pictures: mixed
/// in, leaves turned black at a distance), and a cut-out picture keeps as much of itself drawn in
/// every copy as up close (averaged, a sparse one fell under half opaque and vanished at a
/// distance, as dead bushes did).
fn mip_levels(px: &[u8], w: usize, h: usize) -> Vec<(usize, usize, Vec<u8>)> {
    // How much of a picture is drawn (its pixels at least half opaque, as blocks are cut out).
    let covered = |px: &[u8], scale: f32| px.chunks_exact(4).filter(|p| p[3] as f32 * scale >= 127.5).count() as f32 / (px.len() / 4).max(1) as f32;
    let cut_out = px.chunks_exact(4).any(|p| p[3] < 255);
    let coverage = covered(px, 1.0);
    let mut out = vec![(w, h, px.to_vec())];
    let (mut w, mut h) = (w, h);
    for _ in 1..MIPS {
        {
            if w < 2 || h < 2 {
                break;
            }
            let level = &out.last().expect("a level").2;
            let (nw, nh) = (w / 2, h / 2);
            let mut next = vec![0u8; nw * nh * 4];
            for j in 0..nh {
                for i in 0..nw {
                    let four = [(2 * i, 2 * j), (2 * i + 1, 2 * j), (2 * i, 2 * j + 1), (2 * i + 1, 2 * j + 1)].map(|(a, b)| (b * w + a) * 4);
                    // Color from the pixels that show only (the clear ones are black in the
                    // pictures: mixed in, leaves darkened to black at a distance).
                    let alpha: u32 = four.iter().map(|o| level[o + 3] as u32).sum();
                    for c in 0..3 {
                        next[(j * nw + i) * 4 + c] = if alpha > 0 {
                            (four.iter().map(|o| level[o + c] as u32 * level[o + 3] as u32).sum::<u32>() / alpha) as u8
                        } else {
                            (four.iter().map(|o| level[o + c] as u32).sum::<u32>() / 4) as u8
                        };
                    }
                    next[(j * nw + i) * 4 + 3] = (alpha / 4) as u8;
                }
            }
            // Averaged, a sparse picture's pixels fall under half opaque and it vanished at a
            // distance (dead bushes): its opacity scaled so as much of it is drawn as up close.
            if cut_out && coverage > 0.0 {
                let (mut lo, mut hi) = (0.0f32, 8.0f32);
                for _ in 0..16 {
                    let mid = (lo + hi) * 0.5;
                    if covered(&next, mid) < coverage {
                        lo = mid;
                    } else {
                        hi = mid;
                    }
                }
                for p in next.chunks_exact_mut(4) {
                    p[3] = (p[3] as f32 * hi).round().min(255.0) as u8;
                }
            }
            out.push((nw, nh, next));
            (w, h) = (nw, nh);
        }
    }
    out
}

impl Res {
    #[allow(clippy::too_many_arguments)]
    fn write_level(&self, queue: &wgpu::Queue, mip: u32, x: usize, y: usize, w: usize, h: usize, px: &[u8]) {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.texture, mip_level: mip, origin: wgpu::Origin3d { x: x as u32, y: y as u32, z: 0 }, aspect: wgpu::TextureAspect::All },
            px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w as u32 * 4), rows_per_image: Some(h as u32) },
            wgpu::Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        );
    }
}

struct Paint {
    /// The world drawing (`World::id`), and where to tell it its chunks were dropped.
    owner: u64,
    lost: Arc<std::sync::atomic::AtomicBool>,
    view_proj: [[f32; 4]; 4],
    uploads: Arc<Mutex<Uploads>>,
    atlas: Arc<Mutex<Atlas>>,
    time: f64,
    /// The real blocks' center chunk and reach (chunk x, chunk z, chunks).
    near: (f32, f32, f32),
    lines: Arc<Mutex<Vec<([f32; 3], [f32; 3], egui::Color32)>>>,
    full_bright: bool,
    daylight: f32,
}

/// Whether a box is at least partly inside the view (tested against the six sides of the view
/// taken from its matrix).
fn in_view(m: &[[f32; 4]; 4], lo: [f32; 3], hi: [f32; 3]) -> bool {
    let row = |r: usize| [m[0][r], m[1][r], m[2][r], m[3][r]];
    let (r0, r1, r2, r3) = (row(0), row(1), row(2), row(3));
    let planes = [
        [r3[0] + r0[0], r3[1] + r0[1], r3[2] + r0[2], r3[3] + r0[3]],
        [r3[0] - r0[0], r3[1] - r0[1], r3[2] - r0[2], r3[3] - r0[3]],
        [r3[0] + r1[0], r3[1] + r1[1], r3[2] + r1[2], r3[3] + r1[3]],
        [r3[0] - r1[0], r3[1] - r1[1], r3[2] - r1[2], r3[3] - r1[3]],
        [r2[0], r2[1], r2[2], r2[3]],
    ];
    planes.iter().all(|p| {
        // The corner furthest along the plane's normal.
        let x = if p[0] >= 0.0 { hi[0] } else { lo[0] };
        let y = if p[1] >= 0.0 { hi[1] } else { lo[1] };
        let z = if p[2] >= 0.0 { hi[2] } else { lo[2] };
        p[0] * x + p[1] * y + p[2] * z + p[3] >= 0.0
    })
}

impl egui_wgpu::CallbackTrait for Paint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(res) = resources.get_mut::<Res>() else { return Vec::new() };
        // Another world drew here last (an other open world's view): what is on the GPU is its,
        // so it goes, and this world sends its chunks and pictures again.
        if res.owner != self.owner {
            res.chunks.clear();
            res.lods.clear();
            res.owner = self.owner;
            self.lost.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if let Ok(mut up) = self.uploads.lock() {
            if up.clear {
                res.chunks.clear();
                res.lods.clear();
                up.clear = false;
            }
            for k in up.remove.drain(..) {
                res.chunks.remove(&k);
            }
            if let Some(ghost) = up.ghost.take() {
                res.ghost = ghost
                    .into_iter()
                    .filter(|(_, m)| !m.verts.is_empty())
                    .map(|(k, m)| {
                        let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("gpu-world-ghost"), contents: bytemuck::cast_slice(&m.verts), usage: wgpu::BufferUsages::VERTEX });
                        let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("gpu-world-ghost"), contents: bytemuck::cast_slice(&m.water), usage: wgpu::BufferUsages::INDEX });
                        (k, GpuChunk { vb, ib, solid: 0, water: m.water.len() as u32, lo: m.lo, hi: m.hi })
                    })
                    .collect();
            }
            for k in up.lod_remove.drain(..) {
                res.lods.remove(&k);
            }
            for (k, m) in up.lod_add.drain(..) {
                if m.verts.is_empty() {
                    res.lods.remove(&k);
                    continue;
                }
                let mut indices = m.solid;
                let solid = indices.len() as u32;
                indices.extend_from_slice(&m.water);
                let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("gpu-world-lod"), contents: bytemuck::cast_slice(&m.verts), usage: wgpu::BufferUsages::VERTEX });
                let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("gpu-world-lod"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
                res.lods.insert(k, GpuChunk { vb, ib, solid, water: m.water.len() as u32, lo: m.lo, hi: m.hi });
            }
            for (k, m) in up.add.drain(..) {
                if m.verts.is_empty() {
                    res.chunks.remove(&k);
                    continue;
                }
                let mut indices = m.solid;
                let solid = indices.len() as u32;
                indices.extend_from_slice(&m.water);
                let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("gpu-world-chunk"), contents: bytemuck::cast_slice(&m.verts), usage: wgpu::BufferUsages::VERTEX });
                let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("gpu-world-chunk"), contents: bytemuck::cast_slice(&indices), usage: wgpu::BufferUsages::INDEX });
                res.chunks.insert(k, GpuChunk { vb, ib, solid, water: m.water.len() as u32, lo: m.lo, hi: m.hi });
            }
        }
        // (Never waited for: a worker may hold it while it reads a picture; what is new then goes
        // up next frame. Waiting here froze the window.)
        if let Ok(mut atlas) = self.atlas.try_lock() {
            for (x, y, w, h, px) in atlas.take_uploads() {
                res.write_pixels(queue, x, y, w, h, &px);
            }
            let (cells, _) = atlas.due_frames(self.time);
            for (x, y, px) in cells {
                res.write_pixels(queue, x, y, crate::street_view::CELL, crate::street_view::CELL, &px);
            }
        }
        // This frame's lines in the world.
        res.lines = None;
        res.inverted = None;
        if let Ok(lines) = self.lines.lock() {
            let uv = { let c = Atlas::white().center(); [c.x, c.y] };
            // The plain lines, and those drawn inverted (marked by their color).
            for invert in [false, true] {
                let verts: Vec<Vert> = lines
                    .iter()
                    .filter(|(_, _, c)| (*c == INVERT_LINE) == invert)
                    .flat_map(|(a, b, c)| {
                        let color = if invert { [255, 255, 255, 255] } else { c.to_srgba_unmultiplied() };
                        [Vert { pos: *a, uv, color: color, light: LIT }, Vert { pos: *b, uv, color: color, light: LIT }]
                    })
                    .collect();
                if verts.is_empty() {
                    continue;
                }
                let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("gpu-world-lines"), contents: bytemuck::cast_slice(&verts), usage: wgpu::BufferUsages::VERTEX });
                let n = verts.len() as u32;
                if invert {
                    res.inverted = Some((vb, n));
                } else {
                    res.lines = Some((vb, n));
                }
            }
        }
        queue.write_buffer(&res.uniform, 0, bytemuck::bytes_of(&Uniform { view_proj: self.view_proj, near: [self.near.0, self.near.1, self.near.2, 0.0], look: [if self.full_bright { 1.0 } else { 0.0 }, self.daylight, 0.0, 0.0] }));
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let Some(res) = resources.get::<Res>() else { return };
        let seen: Vec<&GpuChunk> = res
            .chunks
            .iter()
            .filter(|((cx, cz), g)| in_view(&self.view_proj, [(*cx * 16) as f32, g.lo, (*cz * 16) as f32], [(*cx * 16 + 16) as f32, g.hi, (*cz * 16 + 16) as f32]))
            .map(|(_, g)| g)
            .collect();
        pass.set_bind_group(0, &res.bind, &[]);
        pass.set_pipeline(&res.solid);
        for g in &seen {
            if g.solid > 0 {
                pass.set_vertex_buffer(0, g.vb.slice(..));
                pass.set_index_buffer(g.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..g.solid, 0, 0..1);
            }
        }
        // The low-detail land further out (left out where the blocks are).
        pass.set_pipeline(&res.lod);
        let span = (LOD_TILE * 16) as f32;
        for ((tx, tz), g) in &res.lods {
            let (x0, z0) = (*tx as f32 * span, *tz as f32 * span);
            if g.solid > 0 && in_view(&self.view_proj, [x0, g.lo, z0], [x0 + span, g.hi, z0 + span]) {
                pass.set_vertex_buffer(0, g.vb.slice(..));
                pass.set_index_buffer(g.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..g.solid, 0, 0..1);
            }
        }
        // See-through water: the low-detail land's further out first, then the blocks'.
        pass.set_pipeline(&res.lod_water);
        for ((tx, tz), g) in &res.lods {
            let (x0, z0) = (*tx as f32 * span, *tz as f32 * span);
            if g.water > 0 && in_view(&self.view_proj, [x0, g.lo, z0], [x0 + span, g.hi, z0 + span]) {
                pass.set_vertex_buffer(0, g.vb.slice(..));
                pass.set_index_buffer(g.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(g.solid..g.solid + g.water, 0, 0..1);
            }
        }
        pass.set_pipeline(&res.water);
        for g in &seen {
            if g.water > 0 {
                pass.set_vertex_buffer(0, g.vb.slice(..));
                pass.set_index_buffer(g.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(g.solid..g.solid + g.water, 0, 0..1);
            }
        }
        // The paste preview, see-through, over the world.
        if !res.ghost.is_empty() {
            pass.set_pipeline(&res.ghost_pipe);
            for (_, g) in &res.ghost {
                pass.set_vertex_buffer(0, g.vb.slice(..));
                pass.set_index_buffer(g.ib.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..g.water, 0, 0..1);
            }
        }
        if let Some((vb, n)) = &res.lines {
            pass.set_pipeline(&res.line);
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.draw(0..*n, 0..1);
        }
        if let Some((vb, n)) = &res.inverted {
            pass.set_vertex_buffer(0, vb.slice(..));
            pass.set_pipeline(&res.line_mark);
            pass.draw(0..*n, 0..1);
            pass.set_pipeline(&res.line_invert);
            pass.draw(0..*n, 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chunk with every section of one block (or air) from section `lo` for `n` sections.
    fn chunk(lo: i32, n: usize, fill: impl Fn(usize, i32, usize) -> u16) -> Blocks {
        let sections = (0..n)
            .map(|s| {
                let mut b = vec![0u16; 4096].into_boxed_slice();
                for y in 0..16 {
                    for z in 0..16 {
                        for x in 0..16 {
                            b[y * 256 + z * 16 + x] = fill(x, (lo + s as i32) * 16 + y as i32, z);
                        }
                    }
                }
                Some(b)
            })
            .collect();
        Blocks { min_section: lo, sections, extras: HashMap::new(), entities: Vec::new(), barriers: Vec::new() }
    }

    /// A cliff face on a chunk's east edge, under a tree's canopy over the border in the chunk
    /// beside it (open sky a few blocks further on): the air it faces is lit from the open side,
    /// not black (as it was when the light beyond a side was taken from the cliff's own blocks).
    /// Far off (in the smaller copies of the pictures) leaves keep their color and a dead bush
    /// stays drawn: the clear pixels' black is not mixed in, and as much of a cut-out picture is
    /// drawn as up close.
    #[test]
    fn distant_pictures_keep_color_and_shape() {
        for (name, rel) in [("mangrove_leaves", "assets/minecraft/textures/block/mangrove_leaves.png"), ("dead_bush", "assets/minecraft/textures/block/dead_bush.png")] {
            let path = crate::assets::fetch(rel).expect("the picture");
            let img = image::open(path).expect("a picture").to_rgba8();
            let (w, h) = (img.width() as usize, img.height() as usize);
            let levels = mip_levels(img.as_raw(), w, h);
            let drawn = |px: &[u8]| px.chunks_exact(4).filter(|p| p[3] >= 128).count() as f32 / (px.len() / 4) as f32;
            let bright = |px: &[u8]| {
                let shown: Vec<&[u8]> = px.chunks_exact(4).filter(|p| p[3] >= 128).collect();
                shown.iter().map(|p| p[0] as f32 + p[1] as f32 + p[2] as f32).sum::<f32>() / shown.len().max(1) as f32
            };
            let (c0, b0) = (drawn(&levels[0].2), bright(&levels[0].2));
            for (k, (lw, _, px)) in levels.iter().enumerate().skip(1) {
                let (c, b) = (drawn(px), bright(px));
                println!("{name} level {k} ({lw} px): drawn {c:.2} (up close {c0:.2}), brightness {b:.0} (up close {b0:.0})");
                if *lw >= 4 {
                    assert!((c - c0).abs() < 0.2, "{name}: as much drawn far off as up close");
                }
                assert!(b > b0 * 0.85, "{name}: not darkened far off");
            }
        }
    }

    /// The test world made a Bedrock world and a console save by the app's own converters reads
    /// back as Java does: the same blocks under their Java names (dead bushes, not cubes named
    /// `deadbush`), and its block entities (chests, signs, banners...), item frames and mobs.
    #[test]
    fn bedrock_and_console_worlds_read_as_java_does() {
        let Ok(src) = std::env::var("MCTK_SNAPSHOT_WORLD") else { return };
        let src = std::path::PathBuf::from(src);
        let tmp = std::env::temp_dir().join(format!("mctk-gpu-editions-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let bedrock = tmp.join("bedrock");
        mctk_convert::java_to_bedrock(&src, &bedrock).expect("to Bedrock");
        let console = tmp.join("savegame.dat");
        mctk_convert::java_to_console(&src, &console, mctk_console::Platform::Xbox360, &Default::default(), &Default::default(), &Default::default()).expect("to console");
        let dim = "minecraft:overworld";
        let read = |path: &std::path::Path, ed: Edition| {
            let s = open_source(path, ed, dim).expect("opens");
            let mut names = std::collections::BTreeMap::<String, usize>::new();
            let (mut extras, mut entities) = (std::collections::BTreeMap::<String, usize>::new(), std::collections::BTreeMap::<String, usize>::new());
            for cx in -1..4 {
                for cz in -1..3 {
                    let Some(b) = load(&s, cx, cz, None, &Default::default(), dim) else { continue };
                    for sec in b.sections.iter().flatten() {
                        for id in sec.iter().filter(|i| **i != 0) {
                            *names.entry(world::block_name(*id).unwrap_or_default()).or_default() += 1;
                        }
                    }
                    for e in b.extras.values() {
                        *extras.entry(e.get("id").and_then(mctk_nbt::Tag::as_str).unwrap_or("?").to_string()).or_default() += 1;
                    }
                    for e in &b.entities {
                        *entities.entry(e.get("id").and_then(mctk_nbt::Tag::as_str).unwrap_or("?").to_string()).or_default() += 1;
                    }
                }
            }
            (names, extras, entities)
        };
        let java = read(&src, Edition::Java);
        for (label, path, ed) in [("Bedrock", bedrock.as_path(), Edition::Bedrock), ("console", console.as_path(), Edition::Console)] {
            let got = read(path, ed);
            let missing: Vec<&String> = java.0.keys().filter(|n| !got.0.contains_key(*n)).collect();
            let unknown: Vec<&String> = got.0.keys().filter(|n| !java.0.contains_key(*n)).collect();
            println!("{label}: {} block kinds ({} in Java); not found {missing:?}; not in Java {unknown:?}", got.0.len(), java.0.len());
            println!("{label}: block entities {:?}\n  Java {:?}", got.1, java.1);
            println!("{label}: entities {:?}\n  Java {:?}", got.2, java.2);
            assert!(got.0.contains_key("dead_bush"), "{label}: dead bushes by their Java name");
            assert!(!got.0.keys().any(|n| n == "deadbush" || n == "frame" || n == "glow_frame"), "{label}: no Bedrock names left");
            // The console save holds only what the console editions had (no pots, copper,
            // bees...): its block entities are its own.
            if ed == Edition::Bedrock {
                assert_eq!(got.1.keys().collect::<Vec<_>>(), java.1.keys().collect::<Vec<_>>(), "{label}: the same block entities");
            } else {
                assert!(!got.1.is_empty() && got.2.contains_key("minecraft:item"), "{label}: block entities and items");
            }
            assert!(got.2.contains_key("minecraft:item_frame"), "{label}: item frames");
            assert!(got.2.len() >= java.2.len() / 2, "{label}: most kinds of entity");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Every block the game has a blockstate file for is drawn from its model (with faces), or
    /// in code: none falls back to a guessed shape or to nothing. Lists any that do.
    #[test]
    fn every_block_has_a_model() {
        crate::assets::prefetch(&["assets/minecraft/blockstates", "assets/minecraft/models/block"]);
        let names: Vec<String> = crate::assets::list("assets/minecraft/blockstates").into_iter().filter_map(|f| f.strip_suffix(".json").map(str::to_string)).collect();
        assert!(names.len() > 500, "the game's block list ({} found)", names.len());
        // A state the block can be in, from its blockstate file: the first variant's, or for one
        // built of parts each property's first value its parts are shown for.
        let state_of = |name: &str| -> String {
            let Some(path) = crate::assets::fetch(&format!("assets/minecraft/blockstates/{name}.json")) else { return String::new() };
            let Ok(json) = std::fs::read_to_string(path).map(|t| serde_json::from_str::<serde_json::Value>(&t).unwrap_or_default()) else { return String::new() };
            if let Some(v) = json.get("variants").and_then(|v| v.as_object()) {
                return v.keys().next().cloned().unwrap_or_default();
            }
            let mut props: Vec<(String, String)> = Vec::new();
            let mut add = |when: &serde_json::Value| {
                for (k, v) in when.as_object().into_iter().flatten() {
                    let first = v.as_str().map(|s| s.split('|').next().unwrap_or(s).to_string());
                    if let (Some(first), false) = (first, props.iter().any(|(p, _)| p == k)) {
                        props.push((k.clone(), first));
                    }
                }
            };
            for part in json.get("multipart").and_then(|m| m.as_array()).into_iter().flatten() {
                if let Some(when) = part.get("when") {
                    match when.get("OR").or_else(|| when.get("AND")).and_then(|o| o.as_array()) {
                        Some(list) => list.iter().for_each(&mut add),
                        None => add(when),
                    }
                }
            }
            props.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(",")
        };
        let invisible = ["air", "cave_air", "void_air", "barrier", "light", "structure_void", "moving_piston"];
        let mut missing = Vec::new();
        for name in names.iter().filter(|n| !invisible.contains(&n.as_str())) {
            let id = world::intern_state(name, &state_of(name));
            let info = info_of(id);
            if matches!(info.shape, Shape::Air | Shape::Liquid { .. }) || info.entity {
                continue;
            }
            let ok = info.model && baked_of(id).is_some_and(|b| b.alts.iter().any(|(m, _)| !m.quads.is_empty()));
            if !ok {
                missing.push(name.clone());
            }
        }
        // Whole cubes by their box that do not count as solid (their neighbors' faces against
        // them are drawn, as against air).
        let mut not_solid = Vec::new();
        for name in names.iter().filter(|n| !invisible.contains(&n.as_str())) {
            let id = world::intern_state(name, &state_of(name));
            if let Some(b) = baked_of(id) {
                let cube = b.lo.iter().all(|v| v.abs() < 1e-3) && b.hi.iter().all(|v| (v - 1.0).abs() < 1e-3);
                if cube && !info_of(id).solid && info_guess(id).solid {
                    not_solid.push(name.clone());
                }
            }
        }
        println!("whole cubes not counted solid ({}): {not_solid:?}", not_solid.len());
        println!("{} blocks, {} without a model: {missing:?}", names.len(), missing.len());
        assert!(missing.is_empty(), "blocks not drawn from their model: {missing:?}");
    }

    /// The box the pointer outlines fits the block's shape: a button on a wall, a sign, a hanging
    /// sign, a head on a wall and a chest are each smaller than the block (and the button and
    /// head sit against the wall they hang on).
    #[test]
    fn outlines_fit_the_block() {
        let shape = |name: &str, props: &str| {
            let id = world::intern_state(name, props);
            let b = shape_box_of(id, info_of(id));
            println!("{name}[{props}]: {b:?}");
            b.expect("a box")
        };
        let thin = |b: ([f32; 3], [f32; 3])| (0..3).any(|k| b.1[k] - b.0[k] < 0.5);
        let button = shape("stone_button", "face=wall,facing=north,powered=false");
        assert!((button.1[0] - button.0[0]) < 0.5 && (button.1[2] - button.0[2]) < 0.2, "a button is small");
        assert!(button.0[2] > 0.8, "a button facing north hangs on the wall to its south");
        assert!(thin(shape("oak_sign", "rotation=0,waterlogged=false")), "a sign is thin");
        assert!(thin(shape("oak_wall_sign", "facing=north,waterlogged=false")), "a wall sign is thin");
        let hanging = shape("oak_hanging_sign", "attached=false,rotation=0,waterlogged=false");
        assert!(thin(hanging), "a hanging sign is thin");
        let head = shape("player_wall_head", "facing=north,powered=false");
        assert!(head.1[1] - head.0[1] < 0.6 && head.0[2] > 0.4, "a head on a wall is small and against the wall");
        let chest = shape("chest", "facing=north,type=single,waterlogged=false");
        assert!(chest.0[0] > 0.0 && chest.1[1] < 1.0, "a chest is a little smaller than the block");
    }

    #[test]
    fn light_comes_in_across_a_chunk_side() {
        let (stone, leaves) = (world::intern("stone"), world::intern("oak_leaves"));
        let me = chunk(4, 1, |x, y, _| if y <= 64 || (x == 15 && y <= 72) { stone } else { 0 });
        let east = chunk(4, 1, |x, y, _| if y <= 64 { stone } else if y == 70 && x < 3 { leaves } else { 0 });
        let near = [None, Some(Arc::new(east)), None, None];
        let grid = light_grid(&me, &near, &|id| info_of(id));
        let (sky, _) = grid.at(16, 66, 8);
        println!("light beside the cliff, under the canopy: {sky}");
        assert!(sky >= 10, "the air under a canopy over the border should be lit from the open sky beside it (got {sky})");
        // Inside the chunk, open sky is full and inside stone is dark.
        assert_eq!(grid.at(8, 70, 8).0, 15);
        assert_eq!(grid.at(8, 60, 8).0, 0);
    }
}
