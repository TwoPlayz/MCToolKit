//! Top-down map rendering; edition detection and chunk lists come from `mctk-world`.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use mctk_anvil::Region;
use mctk_bedrock::{BedrockWorld, ChunkTag, Key, SubChunk, SubChunkBody};
use mctk_console::{ConsoleChunk, ConsoleRegion, ConsoleSave, console_block_index};
use mctk_nbt::Tag;
use mctk_java::{BlockStorage, Chunk, JavaWorld};
#[cfg(test)]
pub use mctk_world::detect;
pub use mctk_world::Edition;

/// Approximate map colour for a block name (no game textures are used).
pub fn block_colour(name: &str) -> [u8; 3] {
    let n = name.trim_start_matches("minecraft:");
    const TABLE: &[(&str, [u8; 3])] = &[
        ("water", [63, 118, 228]),
        ("kelp", [63, 118, 228]),
        ("seagrass", [63, 118, 228]),
        ("lava", [207, 92, 15]),
        ("magma", [140, 60, 20]),
        ("grass_block", [124, 189, 107]),
        ("short_grass", [110, 170, 90]),
        ("tall_grass", [110, 170, 90]),
        ("fern", [100, 160, 80]),
        ("moss", [90, 130, 50]),
        ("leaves", [60, 140, 50]),
        ("vine", [60, 130, 40]),
        ("lily_pad", [50, 130, 50]),
        ("mycelium", [111, 99, 105]),
        ("podzol", [122, 87, 48]),
        ("red_sand", [190, 102, 33]),
        ("sandstone", [216, 203, 155]),
        ("sand", [219, 207, 163]),
        ("gravel", [136, 126, 126]),
        ("powder_snow", [248, 253, 253]),
        ("snow", [249, 254, 254]),
        ("packed_ice", [140, 180, 250]),
        ("ice", [160, 190, 255]),
        ("clay", [160, 166, 179]),
        ("mud", [60, 57, 60]),
        ("terracotta", [152, 94, 67]),
        ("dirt", [134, 96, 67]),
        ("farmland", [110, 75, 45]),
        ("_log", [102, 81, 51]),
        ("_wood", [102, 81, 51]),
        ("planks", [162, 130, 78]),
        ("cactus", [85, 127, 43]),
        ("pumpkin", [198, 118, 24]),
        ("melon", [111, 145, 30]),
        ("netherrack", [111, 54, 52]),
        ("nylium", [130, 30, 30]),
        ("soul_sand", [81, 62, 50]),
        ("soul_soil", [81, 62, 50]),
        ("basalt", [80, 80, 85]),
        ("blackstone", [42, 36, 41]),
        ("end_stone", [219, 222, 158]),
        ("obsidian", [20, 18, 30]),
        ("bedrock", [85, 85, 85]),
        ("deepslate", [80, 80, 82]),
        ("tuff", [108, 109, 102]),
        ("calcite", [223, 224, 220]),
        ("granite", [149, 103, 85]),
        ("diorite", [188, 188, 188]),
        ("andesite", [136, 136, 136]),
        ("stone", [125, 125, 125]),
        ("cobblestone", [122, 122, 122]),
        ("ore", [130, 130, 130]),
        ("flower", [200, 80, 120]),
        ("poppy", [200, 40, 40]),
        ("dandelion", [230, 210, 40]),
        ("glass", [200, 220, 230]),
        ("wool", [220, 220, 220]),
        ("concrete", [180, 180, 180]),
        ("brick", [150, 97, 83]),
    ];
    TABLE
        .iter()
        .find(|(k, _)| n.contains(k))
        .map(|(_, c)| *c)
        .unwrap_or_else(|| {
            // Stable muted colour for anything else.
            let h = n
                .bytes()
                .fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
            [
                90 + (h % 90) as u8,
                90 + (h / 90 % 90) as u8,
                90 + (h / 8100 % 90) as u8,
            ]
        })
}

/// Names for the common pre-1.13 numeric ids (console and old Java chunks).
fn legacy_name(id: u16) -> &'static str {
    match id {
        1 => "stone",
        2 => "grass_block",
        3 => "dirt",
        4 => "cobblestone",
        5 => "planks",
        7 => "bedrock",
        8 | 9 => "water",
        10 | 11 => "lava",
        12 => "sand",
        13 => "gravel",
        14..=16 | 21 | 56 | 73 | 129 => "ore",
        17 | 162 => "_log",
        18 | 161 => "leaves",
        20 => "glass",
        24 => "sandstone",
        31 | 32 => "short_grass",
        35 => "wool",
        37 | 38 | 175 => "flower",
        45 => "brick",
        49 => "obsidian",
        78 | 80 => "snow",
        79 => "ice",
        81 => "cactus",
        82 => "clay",
        86 => "pumpkin",
        87 => "netherrack",
        88 => "soul_sand",
        110 => "mycelium",
        111 => "lily_pad",
        121 => "end_stone",
        159 | 172 => "terracotta",
        174 => "packed_ice",
        _ => "unknown",
    }
}

fn shade(c: [u8; 3], y: i32) -> [u8; 4] {
    let f = (0.75 + (y as f32 - 40.0) / 300.0).clamp(0.6, 1.2);
    let s = |v: u8| (v as f32 * f).min(255.0) as u8;
    [s(c[0]), s(c[1]), s(c[2]), 255]
}

/// A rendered map: one pixel per block column, origin at chunk (`min_x`, `min_z`).
pub struct MapImage {
    pub min_x: i32,
    pub min_z: i32,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Largest map rendered at one pixel per block; bigger areas are capped around the
/// densest region of chunks.
const MAX_CHUNKS_ACROSS: i32 = 256;

/// Progress of a map render, read by the UI while worker threads fill it in.
#[derive(Default)]
pub struct MapProgress {
    pub done: AtomicUsize,
    /// Chunks to draw; 0 while the world is still being scanned.
    pub total: AtomicUsize,
    cancelled: AtomicBool,
    /// Where the picture sits (min chunk x, min chunk z, width, height), once known.
    pub layout: Mutex<Option<(i32, i32, usize, usize)>>,
    /// Chunks drawn since the UI last looked, so the map can fill in while it renders.
    pub fresh: Mutex<Vec<Tile>>,
}

impl MapProgress {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub fn fraction(&self) -> Option<f32> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| (self.done.load(Ordering::Relaxed) as f32 / total as f32).min(1.0))
    }
}

/// One rendered chunk: 16 × 16 RGBA pixels (transparent where nothing was drawn).
pub type Tile = (i32, i32, Vec<u8>);

/// Runs `f` over `items` on every available core; results come back in no fixed order.
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> Vec<R> + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads.min(items.len()).max(1))
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        out.extend(f(item));
                    }
                    out
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap_or_default())
            .collect()
    })
}

/// Renders the top block of every column; with `max_y` (the Slicer), blocks above that height are ignored.
#[cfg(test)]
pub fn render_map(
    path: &Path,
    edition: Edition,
    dimension: &str,
    max_y: Option<i32>,
) -> Result<MapImage, String> {
    render_map_with(path, edition, dimension, max_y, &MapProgress::default())
}

/// Draws the chunks in `all` (chunk x, chunk z, what is needed to read it) on every core.
/// `draw` returns a chunk's 16 × 16 RGBA pixels, or `None` when it cannot be read.
fn draw_chunks<T: Sync>(
    all: &[(i32, i32, T)],
    dimension: &str,
    progress: &MapProgress,
    draw: impl Fn(&T) -> Option<Vec<u8>> + Sync,
) -> Result<MapImage, String> {
    if all.is_empty() {
        return Err(format!("no chunks in {dimension}"));
    }
    let (mut xs, mut zs): (Vec<i32>, Vec<i32>) = all.iter().map(|c| (c.0, c.1)).unzip();
    xs.sort_unstable();
    zs.sort_unstable();
    // Centre on the median chunk and cap the extent.
    let (mx, mz) = (xs[xs.len() / 2], zs[zs.len() / 2]);
    let half = MAX_CHUNKS_ACROSS / 2;
    let min_x = xs[0].max(mx - half);
    let max_x = xs[xs.len() - 1].min(mx + half - 1);
    let min_z = zs[0].max(mz - half);
    let max_z = zs[zs.len() - 1].min(mz + half - 1);
    let wanted: Vec<&(i32, i32, T)> = all
        .iter()
        .filter(|c| c.0 >= min_x && c.0 <= max_x && c.1 >= min_z && c.1 <= max_z)
        .collect();
    let (w, h) = (
        ((max_x - min_x + 1) * 16) as usize,
        ((max_z - min_z + 1) * 16) as usize,
    );
    progress.done.store(0, Ordering::Relaxed);
    progress.total.store(wanted.len(), Ordering::Relaxed);
    *progress.layout.lock().expect("map layout") = Some((min_x, min_z, w, h));
    let tiles: Vec<Tile> = par_map(&wanted, |c| {
        if progress.is_cancelled() {
            return Vec::new();
        }
        let tile = draw(&c.2).map(|px| (c.0, c.1, px));
        if let Some(t) = &tile {
            progress.fresh.lock().expect("map tiles").push(t.clone());
        }
        progress.done.fetch_add(1, Ordering::Relaxed);
        tile.into_iter().collect()
    });
    if progress.is_cancelled() {
        return Err("cancelled".into());
    }
    let mut rgba = vec![0u8; w * h * 4];
    for (cx, cz, px) in tiles {
        let (gx, gz) = ((cx - min_x) as usize * 16, (cz - min_z) as usize * 16);
        for row in 0..16 {
            let dst = ((gz + row) * w + gx) * 4;
            rgba[dst..dst + 64].copy_from_slice(&px[row * 64..row * 64 + 64]);
        }
    }
    Ok(MapImage {
        min_x,
        min_z,
        width: w,
        height: h,
        rgba,
    })
}

/// [`render_map`] on all cores, reporting through `progress` and stopping when it is cancelled.
pub fn render_map_with(
    path: &Path,
    edition: Edition,
    dimension: &str,
    max_y: Option<i32>,
    progress: &MapProgress,
) -> Result<MapImage, String> {
    let max_y = max_y.unwrap_or(i32::MAX);
    progress.done.store(0, Ordering::Relaxed);
    progress.total.store(0, Ordering::Relaxed);
    match edition {
        Edition::Java => {
            let world = JavaWorld::open(path).map_err(|e| e.to_string())?;
            let dim = world.dimension(dimension).ok_or("no such dimension")?;
            let dir = dim.region_dir();
            let rpositions: Vec<_> = dim.regions().map_err(|e| e.to_string())?.into_iter().collect();
            // Region files are read in parallel too.
            let regions: Vec<(_, Region)> = par_map(&rpositions, |rpos| {
                Region::open(dir.join(rpos.file_name()))
                    .ok()
                    .map(|r| (*rpos, r))
                    .into_iter()
                    .collect()
            });
            let mut all = Vec::new();
            for (rpos, region) in &regions {
                for (lx, lz) in region.chunks() {
                    let pos = rpos.chunk(lx, lz);
                    all.push((pos.x, pos.z, (region, lx, lz)));
                }
            }
            draw_chunks(&all, dimension, progress, |(region, lx, lz)| {
                let root = region.read_chunk(*lx, *lz).ok()??;
                let chunk = Chunk::from_nbt_read_only(root).ok()?;
                let mut px = vec![0u8; 16 * 16 * 4];
                for z in 0..16 {
                    for x in 0..16 {
                        if let Some((name, y)) = java_top(&chunk, x, z, max_y) {
                            let i = (z * 16 + x) * 4;
                            px[i..i + 4].copy_from_slice(&shade(block_colour(&name), y));
                        }
                    }
                }
                Some(px)
            })
        }
        Edition::Bedrock => {
            let world = BedrockWorld::open(path).map_err(|e| e.to_string())?;
            let dim_id = match dimension {
                "minecraft:the_nether" => 1,
                "minecraft:the_end" => 2,
                _ => 0,
            };
            // Subchunks grouped per chunk, highest first so the first block found is the top.
            let mut by_chunk: BTreeMap<(i32, i32), Vec<(i8, &Vec<u8>)>> = BTreeMap::new();
            for (k, v) in &world.db.entries {
                if let Key::Chunk(c) = Key::decode(k) {
                    if c.tag == ChunkTag::SubChunkPrefix && c.dimension.id() == dim_id {
                        by_chunk
                            .entry((c.x, c.z))
                            .or_default()
                            .push((c.subchunk.unwrap_or(0), v));
                    }
                }
            }
            let all: Vec<_> = by_chunk
                .into_iter()
                .map(|((cx, cz), mut subs)| {
                    subs.sort_by_key(|(sy, _)| std::cmp::Reverse(*sy));
                    (cx, cz, subs)
                })
                .collect();
            draw_chunks(&all, dimension, progress, |subs| {
                let parsed: Vec<(i32, SubChunkBody)> = subs
                    .iter()
                    .filter_map(|(sy, raw)| Some((*sy as i32, SubChunk::parse(raw).ok()?.body)))
                    .collect();
                let mut px = vec![0u8; 16 * 16 * 4];
                for z in 0..16 {
                    for x in 0..16 {
                        'column: for (sy, body) in &parsed {
                            let SubChunkBody::Palettized(storages) = body else {
                                continue;
                            };
                            let Some(s) = storages.first() else { continue };
                            for y in (0..16).rev() {
                                let by = sy * 16 + y as i32;
                                if by > max_y {
                                    continue;
                                }
                                let name = s.block_name(x, y, z).unwrap_or("minecraft:air");
                                if name != "minecraft:air" {
                                    let i = (z * 16 + x) * 4;
                                    px[i..i + 4].copy_from_slice(&shade(block_colour(name), by));
                                    break 'column;
                                }
                            }
                        }
                    }
                }
                Some(px)
            })
        }
        Edition::Console => {
            let save = ConsoleSave::open(path).map_err(|e| e.to_string())?;
            let want = match dimension {
                "minecraft:the_nether" => -1,
                "minecraft:the_end" => 1,
                _ => 0,
            };
            let entries: Vec<_> = save
                .archive
                .regions()
                .into_iter()
                .filter(|(dim, _, _, e)| *dim == want && !e.data.is_empty())
                .collect();
            // Regions are unpacked in parallel, then their chunks drawn in parallel.
            let parsed: Vec<(i32, i32, ConsoleRegion)> = par_map(&entries, |(_, rx, rz, e)| {
                ConsoleRegion::parse(e.data.clone())
                    .ok()
                    .map(|r| (*rx, *rz, r))
                    .into_iter()
                    .collect()
            });
            let mut all = Vec::new();
            for (rx, rz, region) in &parsed {
                for (lx, lz) in region.chunks() {
                    all.push((rx * 32 + lx as i32, rz * 32 + lz as i32, (region, lx, lz)));
                }
            }
            draw_chunks(&all, dimension, progress, |(region, lx, lz)| {
                let raw = region.raw_chunk(*lx, *lz).ok()??;
                let chunk = ConsoleChunk::from_nbt(raw.nbt().ok()?).ok()?;
                // The block array is looked up once; `ConsoleChunk::block` would search the
                // NBT tree for it on every call.
                let Some(Tag::ByteArray(blocks)) = chunk.to_nbt().tag.get("Level")?.get("Blocks")
                else {
                    return None;
                };
                let top_y = chunk.height.min((max_y.max(-1) + 1) as usize);
                let mut colours = [None; 256];
                let mut px = vec![0u8; 16 * 16 * 4];
                for z in 0..16 {
                    for x in 0..16 {
                        let top = (0..top_y).rev().find_map(|y| {
                            let id = *blocks.get(console_block_index(x, y, z))? as u8;
                            (id != 0).then_some((y, id))
                        });
                        if let Some((y, id)) = top {
                            let c = *colours[id as usize]
                                .get_or_insert_with(|| block_colour(legacy_name(id as u16)));
                            let i = (z * 16 + x) * 4;
                            px[i..i + 4].copy_from_slice(&shade(c, y as i32));
                        }
                    }
                }
                Some(px)
            })
        }
    }
}

fn java_top(chunk: &Chunk, x: usize, z: usize, max_y: i32) -> Option<(String, i32)> {
    for s in chunk
        .sections
        .iter()
        .rev()
        .filter(|s| s.y as i32 * 16 <= max_y)
    {
        match &s.blocks {
            Some(BlockStorage::Palette(p)) => {
                if p.palette.iter().all(|b| b.is_air()) {
                    continue;
                }
                for y in (0..16)
                    .rev()
                    .filter(|y| s.y as i32 * 16 + *y as i32 <= max_y)
                {
                    let b = p.get(mctk_java::block_index(x, y, z));
                    if !b.is_air() {
                        return Some((b.name.clone(), s.y as i32 * 16 + y as i32));
                    }
                }
            }
            Some(BlockStorage::Legacy { ids, .. }) => {
                for y in (0..16)
                    .rev()
                    .filter(|y| s.y as i32 * 16 + *y as i32 <= max_y)
                {
                    let id = ids[mctk_java::block_index(x, y, z)];
                    if id != 0 {
                        return Some((legacy_name(id).to_string(), s.y as i32 * 16 + y as i32));
                    }
                }
            }
            None => {}
        }
    }
    None
}
