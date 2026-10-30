//! Top-down map rendering; edition detection and chunk lists come from `mctk-world`.

use std::collections::BTreeMap;
use std::path::Path;

use mctk_anvil::Region;
use mctk_bedrock::{BedrockWorld, ChunkTag, Key, SubChunk, SubChunkBody};
use mctk_console::{ConsoleChunk, ConsoleRegion, ConsoleSave};
use mctk_java::{BlockStorage, Chunk, JavaWorld};
#[cfg(test)]
pub use mctk_world::detect;
pub use mctk_world::{Edition, list_chunks};

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

/// Renders the top block of every column; with `max_y` (the Slicer), blocks above that height are ignored.
pub fn render_map(
    path: &Path,
    edition: Edition,
    dimension: &str,
    max_y: Option<i32>,
) -> Result<MapImage, String> {
    let max_y = max_y.unwrap_or(i32::MAX);
    let chunks = list_chunks(path, edition)?;
    let list = chunks.get(dimension).cloned().unwrap_or_default();
    if list.is_empty() {
        return Err(format!("no chunks in {dimension}"));
    }
    let (mut xs, mut zs): (Vec<i32>, Vec<i32>) = list.iter().map(|c| (c.x, c.z)).unzip();
    xs.sort_unstable();
    zs.sort_unstable();
    // Centre on the median chunk and cap the extent.
    let (mx, mz) = (xs[xs.len() / 2], zs[zs.len() / 2]);
    let half = MAX_CHUNKS_ACROSS / 2;
    let min_x = xs[0].max(mx - half);
    let max_x = xs[xs.len() - 1].min(mx + half - 1);
    let min_z = zs[0].max(mz - half);
    let max_z = zs[zs.len() - 1].min(mz + half - 1);
    let (w, h) = (
        ((max_x - min_x + 1) * 16) as usize,
        ((max_z - min_z + 1) * 16) as usize,
    );
    let mut rgba = vec![0u8; w * h * 4];
    let mut put = |cx: i32, cz: i32, x: usize, z: usize, px: [u8; 4]| {
        if cx < min_x || cx > max_x || cz < min_z || cz > max_z {
            return;
        }
        let gx = (cx - min_x) as usize * 16 + x;
        let gz = (cz - min_z) as usize * 16 + z;
        let i = (gz * w + gx) * 4;
        rgba[i..i + 4].copy_from_slice(&px);
    };
    match edition {
        Edition::Java => {
            let world = JavaWorld::open(path).map_err(|e| e.to_string())?;
            let dim = world.dimension(dimension).ok_or("no such dimension")?;
            for rpos in dim.regions().map_err(|e| e.to_string())? {
                let region = Region::open(dim.region_dir().join(rpos.file_name()))
                    .map_err(|e| e.to_string())?;
                for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                    let pos = rpos.chunk(lx, lz);
                    if pos.x < min_x || pos.x > max_x || pos.z < min_z || pos.z > max_z {
                        continue;
                    }
                    let Ok(Some(root)) = region.read_chunk(lx, lz) else {
                        continue;
                    };
                    let Ok(chunk) = Chunk::from_nbt(root) else {
                        continue;
                    };
                    for z in 0..16 {
                        for x in 0..16 {
                            if let Some((name, y)) = java_top(&chunk, x, z, max_y) {
                                put(pos.x, pos.z, x, z, shade(block_colour(&name), y));
                            }
                        }
                    }
                }
            }
        }
        Edition::Bedrock => {
            let world = BedrockWorld::open(path).map_err(|e| e.to_string())?;
            let dim_id = match dimension {
                "minecraft:the_nether" => 1,
                "minecraft:the_end" => 2,
                _ => 0,
            };
            // Highest subchunk first, so the first block found per column is the top.
            let mut subs: BTreeMap<(i32, i32, std::cmp::Reverse<i8>), Vec<u8>> = BTreeMap::new();
            for (k, v) in &world.db.entries {
                if let Key::Chunk(c) = Key::decode(k) {
                    if c.tag == ChunkTag::SubChunkPrefix && c.dimension.id() == dim_id {
                        subs.insert(
                            (c.x, c.z, std::cmp::Reverse(c.subchunk.unwrap_or(0))),
                            v.clone(),
                        );
                    }
                }
            }
            let mut done: std::collections::HashSet<(i32, i32, usize, usize)> = Default::default();
            for ((cx, cz, std::cmp::Reverse(sy)), raw) in &subs {
                let Ok(sub) = SubChunk::parse(raw) else {
                    continue;
                };
                let SubChunkBody::Palettized(storages) = sub.body else {
                    continue;
                };
                let Some(s) = storages.first() else { continue };
                for z in 0..16 {
                    for x in 0..16 {
                        if done.contains(&(*cx, *cz, x, z)) {
                            continue;
                        }
                        for y in (0..16).rev() {
                            if *sy as i32 * 16 + y as i32 > max_y {
                                continue;
                            }
                            let name = s.block_name(x, y, z).unwrap_or("minecraft:air");
                            if name != "minecraft:air" {
                                put(
                                    *cx,
                                    *cz,
                                    x,
                                    z,
                                    shade(block_colour(name), *sy as i32 * 16 + y as i32),
                                );
                                done.insert((*cx, *cz, x, z));
                                break;
                            }
                        }
                    }
                }
            }
        }
        Edition::Console => {
            let save = ConsoleSave::open(path).map_err(|e| e.to_string())?;
            let want = match dimension {
                "minecraft:the_nether" => -1,
                "minecraft:the_end" => 1,
                _ => 0,
            };
            for (dim, _, _, e) in save.archive.regions() {
                if dim != want || e.data.is_empty() {
                    continue;
                }
                let region = ConsoleRegion::parse(e.data.clone()).map_err(|e| e.to_string())?;
                for (lx, lz) in region.chunks().collect::<Vec<_>>() {
                    let Ok(Some(raw)) = region.raw_chunk(lx, lz) else {
                        continue;
                    };
                    let Ok(nbt) = raw.nbt() else { continue };
                    let Ok(chunk) = ConsoleChunk::from_nbt(nbt) else {
                        continue;
                    };
                    for z in 0..16 {
                        for x in 0..16 {
                            let top = (0..chunk.height).rev().find_map(|y| {
                                (y as i32 <= max_y)
                                    .then(|| chunk.block(x, y, z))
                                    .flatten()
                                    .filter(|(id, _)| *id != 0)
                                    .map(|(id, _)| (y, id))
                            });
                            if let Some((y, id)) = top {
                                put(
                                    chunk.x,
                                    chunk.z,
                                    x,
                                    z,
                                    shade(block_colour(legacy_name(id as u16)), y as i32),
                                );
                            }
                        }
                    }
                }
            }
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
