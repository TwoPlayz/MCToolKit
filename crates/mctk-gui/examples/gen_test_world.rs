//! Writes a synthetic Java world for trying the world map without a real save: rolling land,
//! oceans, a river, mountains with snow, and a dozen biomes (each recorded in the chunks, as the
//! game does) with trees, grass and flowers.
//!
//! cargo run --release -p mctk-gui --example gen_test_world -- <out dir> [chunks across]

use std::io::Write;
use std::path::PathBuf;

use mctk_anvil::{ChunkCompression, RawChunk, RegionBuilder, RegionPos};
use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};

const SEA: i32 = 62;
const MIN_Y: i32 = -64;
const SECTIONS: i32 = 24;

fn hash(x: i32, z: i32, seed: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6b343) ^ (z as u32).wrapping_mul(0xd8163841) ^ seed.wrapping_mul(0xcb1ab31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1e995);
    h ^= h >> 15;
    (h & 0xFFFFFF) as f32 / 0xFFFFFF as f32
}

/// Smooth value noise in 0..1.
fn noise(x: f32, z: f32, scale: f32, seed: u32) -> f32 {
    let (fx, fz) = (x / scale, z / scale);
    let (x0, z0) = (fx.floor() as i32, fz.floor() as i32);
    let (tx, tz) = (fx - x0 as f32, fz - z0 as f32);
    let s = |t: f32| t * t * (3.0 - 2.0 * t);
    let (sx, sz) = (s(tx), s(tz));
    let a = hash(x0, z0, seed) + (hash(x0 + 1, z0, seed) - hash(x0, z0, seed)) * sx;
    let b = hash(x0, z0 + 1, seed) + (hash(x0 + 1, z0 + 1, seed) - hash(x0, z0 + 1, seed)) * sx;
    a + (b - a) * sz
}

fn fbm(x: f32, z: f32, scale: f32, seed: u32) -> f32 {
    let (mut sum, mut amp, mut norm, mut sc) = (0.0, 1.0, 0.0, scale);
    for o in 0..5 {
        sum += noise(x, z, sc, seed + o) * amp;
        norm += amp;
        amp *= 0.5;
        sc *= 0.5;
    }
    sum / norm
}

struct Column {
    height: i32,
    biome: &'static str,
    top: &'static str,
    under: &'static str,
    water: bool,
}

fn column(x: i32, z: i32) -> Column {
    let (fx, fz) = (x as f32, z as f32);
    let continent = fbm(fx, fz, 420.0, 1);
    let hills = fbm(fx, fz, 90.0, 7);
    let mut h = 40.0 + continent * 60.0 + (hills - 0.5) * 26.0;
    let mountain = ((fbm(fx, fz, 300.0, 21) - 0.6) * 4.0).clamp(0.0, 1.0);
    h += mountain * mountain * 90.0;
    // A winding river carved through the land.
    let r = (fbm(fx, fz, 260.0, 33) - 0.5).abs();
    let river = r < 0.018 && h > SEA as f32 - 2.0 && h < 100.0;
    if river {
        h = h.min(SEA as f32 - 3.0 + r * 80.0);
    }
    let height = h as i32;
    let temp = fbm(fx, fz, 380.0, 51);
    let wet = fbm(fx, fz, 330.0, 77);
    let water = height < SEA;
    let (biome, top, under): (&str, &str, &str) = if river {
        (if temp < 0.3 { "frozen_river" } else { "river" }, "sand", "sand")
    } else if height < SEA - 14 {
        (if temp < 0.3 { "deep_frozen_ocean" } else if temp > 0.62 { "deep_lukewarm_ocean" } else { "deep_ocean" }, "gravel", "gravel")
    } else if water {
        (if temp < 0.3 { "frozen_ocean" } else if temp > 0.62 { "warm_ocean" } else { "ocean" }, "sand", "sand")
    } else if height <= SEA + 1 {
        (if temp < 0.3 { "snowy_beach" } else { "beach" }, "sand", "sand")
    } else if height > 128 {
        (if temp < 0.45 { "jagged_peaks" } else { "snowy_slopes" }, "snow_block", "stone")
    } else if height > 100 {
        ("windswept_hills", "stone", "stone")
    } else if temp < 0.3 {
        if wet > 0.5 { ("snowy_taiga", "grass_block", "dirt") } else { ("snowy_plains", "grass_block", "dirt") }
    } else if temp < 0.42 {
        ("taiga", "podzol", "dirt")
    } else if temp > 0.66 {
        if wet < 0.38 {
            ("badlands", "red_sand", "terracotta")
        } else if wet < 0.5 {
            ("desert", "sand", "sandstone")
        } else if wet > 0.62 {
            ("jungle", "grass_block", "dirt")
        } else {
            ("savanna", "grass_block", "dirt")
        }
    } else if wet > 0.64 && height < SEA + 6 {
        ("swamp", "grass_block", "dirt")
    } else if wet > 0.56 {
        ("dark_forest", "grass_block", "dirt")
    } else if wet > 0.48 {
        ("forest", "grass_block", "dirt")
    } else if wet > 0.42 {
        ("birch_forest", "grass_block", "dirt")
    } else if hash(x / 64, z / 64, 9) > 0.85 {
        ("cherry_grove", "grass_block", "dirt")
    } else {
        ("plains", "grass_block", "dirt")
    };
    Column { height, biome, top, under, water }
}

/// Trees sit on a jittered grid; how dense depends on the biome.
fn tree_at(cell_x: i32, cell_z: i32) -> Option<(i32, i32, i32, &'static str, &'static str)> {
    const CELL: i32 = 5;
    let x = cell_x * CELL + (hash(cell_x, cell_z, 101) * CELL as f32) as i32;
    let z = cell_z * CELL + (hash(cell_x, cell_z, 102) * CELL as f32) as i32;
    let c = column(x, z);
    if c.water || c.top == "sand" || c.top == "red_sand" || c.top == "stone" || c.top == "snow_block" {
        return None;
    }
    let (density, log, leaves) = match c.biome {
        "forest" => (0.75, "oak_log", "oak_leaves"),
        "dark_forest" => (0.95, "dark_oak_log", "dark_oak_leaves"),
        "birch_forest" => (0.7, "birch_log", "birch_leaves"),
        "taiga" | "snowy_taiga" => (0.7, "spruce_log", "spruce_leaves"),
        "jungle" => (0.9, "jungle_log", "jungle_leaves"),
        "swamp" => (0.4, "oak_log", "oak_leaves"),
        "savanna" => (0.12, "acacia_log", "acacia_leaves"),
        "cherry_grove" => (0.6, "cherry_log", "cherry_leaves"),
        "plains" => (0.04, "oak_log", "oak_leaves"),
        _ => return None,
    };
    if hash(cell_x, cell_z, 103) > density {
        return None;
    }
    let trunk = 4 + (hash(cell_x, cell_z, 104) * 3.0) as i32 + if c.biome == "jungle" { 4 } else { 0 };
    Some((x, z, c.height + trunk, log, leaves))
}

/// Packs palette indices the way 1.16+ chunks do (entries never span two longs).
fn pack(indices: &[usize], bits: usize) -> Vec<i64> {
    let per = 64 / bits;
    let mut out = vec![0i64; indices.len().div_ceil(per)];
    for (i, v) in indices.iter().enumerate() {
        out[i / per] |= (*v as i64) << ((i % per) * bits);
    }
    out
}

fn palette_list(names: &[String], as_blocks: bool) -> Tag {
    let items = names
        .iter()
        .map(|n| {
            if as_blocks {
                // "water[level=3]": a block with its states.
                let (name, states) = n.split_once('[').map_or((n.as_str(), None), |(a, b)| (a, Some(b.trim_end_matches(']'))));
                let mut c = Compound::new();
                c.insert("Name".into(), Tag::String(format!("minecraft:{name}")));
                if let Some(states) = states {
                    let mut props = Compound::new();
                    for kv in states.split(',') {
                        if let Some((k, v)) = kv.split_once('=') {
                            props.insert(k.into(), Tag::String(v.into()));
                        }
                    }
                    c.insert("Properties".into(), Tag::Compound(props));
                }
                Tag::Compound(c)
            } else {
                Tag::String(format!("minecraft:{n}"))
            }
        })
        .collect();
    Tag::List(List::from_tags(if as_blocks { TagType::Compound } else { TagType::String }, items).expect("one type"))
}

fn chunk_nbt(cx: i32, cz: i32) -> NamedTag {
    // The blocks of the chunk, by section.
    let mut blocks: Vec<Vec<&'static str>> = vec![vec!["air"; 4096]; SECTIONS as usize];
    let mut biomes: Vec<[&'static str; 64]> = vec![["plains"; 64]; SECTIONS as usize];
    let set = |x: i32, y: i32, z: i32, b: &'static str, blocks: &mut Vec<Vec<&'static str>>| {
        if !(MIN_Y..MIN_Y + SECTIONS * 16).contains(&y) || !(0..16).contains(&x) || !(0..16).contains(&z) {
            return;
        }
        let s = ((y - MIN_Y) / 16) as usize;
        blocks[s][(((y - MIN_Y) % 16) * 256 + z * 16 + x) as usize] = b;
    };
    for z in 0..16 {
        for x in 0..16 {
            let (wx, wz) = (cx * 16 + x, cz * 16 + z);
            let c = column(wx, wz);
            for y in MIN_Y..=c.height {
                let b = if y == MIN_Y {
                    "bedrock"
                } else if y < 0 {
                    "deepslate"
                } else if y == c.height {
                    c.top
                } else if y > c.height - 4 {
                    c.under
                } else {
                    "stone"
                };
                set(x, y, z, b, &mut blocks);
            }
            if c.water {
                for y in c.height + 1..=SEA {
                    set(x, y, z, if c.biome.contains("frozen") && y == SEA { "ice" } else { "water" }, &mut blocks);
                }
            } else {
                let r = hash(wx, wz, 200);
                let above = c.height + 1;
                match c.biome {
                    "plains" | "savanna" | "cherry_grove" | "meadow" if r < 0.25 => set(x, above, z, "short_grass", &mut blocks),
                    "plains" | "cherry_grove" if r < 0.29 => set(x, above, z, if r < 0.27 { "poppy" } else { "dandelion" }, &mut blocks),
                    "forest" | "birch_forest" | "jungle" | "swamp" if r < 0.2 => set(x, above, z, "short_grass", &mut blocks),
                    "snowy_plains" | "snowy_taiga" => set(x, above, z, "snow", &mut blocks),
                    "desert" if r < 0.01 => set(x, above, z, "cactus", &mut blocks),
                    _ => {}
                }
            }
            // The biome where the column's surface is (one per 4 x 4 x 4 cell, as in the game).
            for s in 0..SECTIONS {
                for ly in (0..4).map(|k| k * 4) {
                    biomes[s as usize][((ly / 4) * 16 + (z / 4) * 4 + x / 4) as usize] = c.biome;
                }
            }
        }
    }
    // Trees whose trunk or crown reaches into this chunk.
    for gx in (cx * 16 - 8).div_euclid(5)..=(cx * 16 + 24).div_euclid(5) {
        for gz in (cz * 16 - 8).div_euclid(5)..=(cz * 16 + 24).div_euclid(5) {
            let Some((tx, tz, top, log, leaves)) = tree_at(gx, gz) else { continue };
            let ground = column(tx, tz).height;
            let (lx, lz) = (tx - cx * 16, tz - cz * 16);
            let r = if log == "spruce_log" { 2 } else { 2 + (hash(gx, gz, 105) > 0.6) as i32 };
            for dy in -2..=1 {
                let rr = if dy == 1 { r - 1 } else { r };
                for dx in -rr..=rr {
                    for dz in -rr..=rr {
                        if dx * dx + dz * dz > rr * rr + 1 {
                            continue;
                        }
                        set(lx + dx, top + dy, lz + dz, leaves, &mut blocks);
                    }
                }
            }
            for y in ground + 1..top {
                set(lx, y, lz, log, &mut blocks);
            }
        }
    }
    // A floor for the mob zoo in chunk (2, 0), level with the showcase.
    if (cx, cz) == (2, 0) {
        let floor = column(24, 8).height.max(SEA + 1);
        for z in 0..16 {
            for x in 0..16 {
                for y in floor - 4..floor {
                    set(x, y, z, "stone", &mut blocks);
                }
                set(x, floor, z, "smooth_stone", &mut blocks);
                for y in floor + 1..floor + 12 {
                    set(x, y, z, "air", &mut blocks);
                }
            }
        }
    }
    // A showcase in chunk (1, 0): blocks in many states on a stone floor, rows of them west to
    // east, to check each is drawn as the game draws it.
    if (cx, cz) == (1, 0) {
        let floor = column(24, 8).height.max(SEA + 1);
        for z in 1..15 {
            for x in 1..15 {
                for y in floor - 4..floor {
                    set(x, y, z, "stone", &mut blocks);
                }
                set(x, floor, z, "smooth_stone", &mut blocks);
                for y in floor + 1..floor + 8 {
                    set(x, y, z, "air", &mut blocks);
                }
            }
        }
        let on = floor + 1;
        let rows: [&[&str]; 7] = [
            // Stairs every way, upside down, and their corners.
            &[
                "oak_stairs[facing=north,half=bottom,shape=straight]", "oak_stairs[facing=east,half=bottom,shape=straight]",
                "oak_stairs[facing=south,half=bottom,shape=straight]", "oak_stairs[facing=west,half=bottom,shape=straight]",
                "stone_brick_stairs[facing=north,half=top,shape=straight]", "stone_brick_stairs[facing=east,half=bottom,shape=inner_left]",
                "stone_brick_stairs[facing=east,half=bottom,shape=outer_right]", "stone_slab[type=top]", "stone_slab[type=bottom]",
                "oak_trapdoor[facing=north,half=bottom,open=false]", "oak_trapdoor[facing=north,half=bottom,open=true]",
            ],
            // Things that join their neighbours.
            &[
                "oak_fence[east=true,west=false]", "oak_fence[east=true,west=true]", "oak_fence[west=true,east=false]", "air",
                "glass_pane[east=true,west=false]", "glass_pane[east=true,west=true]", "glass_pane[west=true,south=true]", "air",
                "cobblestone_wall[east=low,west=none,up=true]", "cobblestone_wall[east=tall,west=low,up=false]", "cobblestone_wall[west=tall,up=true]",
            ],
            // Redstone.
            &[
                "redstone_wire[east=side,west=side,power=15]", "redstone_wire[east=side,west=side,power=8]", "redstone_wire[west=side,north=side,power=1]", "air",
                "repeater[facing=west,delay=2]", "comparator[facing=west]", "lever[face=floor,facing=north,powered=true]",
                "stone_button[face=floor,facing=north]", "redstone_torch[lit=true]", "redstone_lamp[lit=true]", "observer[facing=north]",
            ],
            // On walls and hanging.
            &[
                "ladder[facing=south]", "wall_torch[facing=south]", "lantern[hanging=true]", "chain[axis=y]", "end_rod[facing=up]",
                "lightning_rod[facing=up]", "bell[attachment=floor,facing=north]", "oak_door[facing=south,half=lower,hinge=left,open=false]",
                "iron_bars[east=true,west=true]", "scaffolding", "hopper[facing=down]",
            ],
            // Workstations and odd shapes.
            &[
                "anvil[facing=north]", "grindstone[face=floor,facing=north]", "brewing_stand", "cauldron", "water_cauldron[level=3]",
                "lectern[facing=north]", "enchanting_table", "stonecutter[facing=north]", "composter[level=4]", "cake[bites=2]", "flower_pot",
            ],
            // Plants and growing things.
            &[
                "wheat[age=7]", "carrots[age=3]", "sweet_berry_bush[age=3]", "potted_poppy", "bamboo[age=0,leaves=large]", "sea_pickle[pickles=3,waterlogged=false]",
                "turtle_egg[eggs=3]", "snow[layers=3]", "candle[candles=3,lit=true]", "sunflower[half=lower]", "pink_petals[flower_amount=4,facing=north]",
            ],
            // See-through and lit.
            &[
                "red_stained_glass", "tinted_glass", "ice", "honey_block", "slime_block", "furnace[facing=south,lit=true]",
                "oak_stairs[facing=south,half=bottom,shape=straight,waterlogged=true]", "sea_lantern", "glow_lichen[down=true]", "amethyst_cluster[facing=up]", "beacon",
            ],
        ];
        for (r, row) in rows.iter().enumerate() {
            let z = 2 + r as i32 * 2;
            for (k, b) in row.iter().enumerate() {
                set(2 + k as i32, on, z, b, &mut blocks);
            }
        }
        // Blocks the game draws in code, along the north edge, facing south.
        let drawn = [
            "chest[facing=south,type=single]", "chest[facing=south,type=right]", "chest[facing=south,type=left]", "ender_chest[facing=south]",
            "copper_chest[facing=south,type=single]", "red_shulker_box[facing=up]", "skeleton_skull[rotation=0]", "creeper_head[rotation=2]",
            "player_head[rotation=0]", "piglin_head[rotation=0]", "white_banner[rotation=0]", "decorated_pot[facing=south]", "red_bed[facing=south,part=foot]",
        ];
        for (k, b) in drawn.iter().enumerate() {
            set(2 + k as i32, on, 1, b, &mut blocks);
        }
        set(14, on, 0, "red_bed[facing=south,part=head]", &mut blocks);
        // A wall behind them for item frames and a painting.
        for x in 2..14 {
            for y in on + 1..on + 4 {
                set(x, y, 0, "stone_bricks", &mut blocks);
            }
        }
        set(1, on, 3, "oak_sign[rotation=4]", &mut blocks);
        set(1, on, 4, "oak_hanging_sign[rotation=0,attached=false]", &mut blocks);
        set(1, on + 1, 4, "oak_planks", &mut blocks);
        // Copper golem statues in their four poses, from new to oxidised, facing east.
        for (k, b) in [
            "copper_golem_statue[copper_golem_pose=standing,facing=east,waterlogged=false]",
            "exposed_copper_golem_statue[copper_golem_pose=running,facing=east,waterlogged=false]",
            "waxed_weathered_copper_golem_statue[copper_golem_pose=sitting,facing=east,waterlogged=false]",
            "oxidized_copper_golem_statue[copper_golem_pose=star,facing=east,waterlogged=false]",
        ]
        .iter()
        .enumerate()
        {
            set(1, on, 9 + k as i32 * 2, b, &mut blocks);
        }
        // The upper halves of the door and sunflower.
        set(9, on + 1, 8, "oak_door[facing=south,half=upper,hinge=left,open=false]", &mut blocks);
        set(11, on + 1, 12, "sunflower[half=upper]", &mut blocks);
        // A wall for the ladder and torch, a chain above the lantern, water by the wet stairs.
        set(2, on, 7, "stone_bricks", &mut blocks);
        set(3, on, 7, "stone_bricks", &mut blocks);
        set(4, on + 1, 8, "oak_planks", &mut blocks);
        set(8, on, 15, "water", &mut blocks);
    }
    // A mangrove canopy over a patch of sand with dead bushes, in chunk (1, 1): thick leaves
    // (dark inside if light does not reach in) and plants the drawing has missed.
    if (cx, cz) == (1, 1) {
        let floor = column(8, 8).height.max(SEA + 1);
        for z in 3..13 {
            for x in 3..13 {
                set(x, floor, z, "sand", &mut blocks);
                for y in floor + 1..floor + 14 {
                    set(x, y, z, "air", &mut blocks);
                }
            }
        }
        for (x, z) in [(5, 5), (7, 6), (9, 8), (6, 10), (10, 11)] {
            set(x, floor + 1, z, "dead_bush", &mut blocks);
        }
        set(4, floor + 1, 9, "short_dry_grass", &mut blocks);
        set(11, floor + 1, 5, "tall_dry_grass", &mut blocks);
        for y in floor + 1..floor + 7 {
            set(8, y, 3, "mangrove_log[axis=y]", &mut blocks);
        }
        for z in 0..7 {
            for x in 4..13 {
                for y in floor + 7..floor + 12 {
                    set(x, y, z, "mangrove_leaves[distance=1,persistent=true,waterlogged=false]", &mut blocks);
                }
            }
        }
    }
    // A small camp in chunk (0, 0), to see blocks that are not whole cubes: a planks floor with a
    // campfire, fences, a wall, torches, a lantern, slabs, carpet, a chest, rails and a pane.
    if (cx, cz) == (0, 0) {
        let floor = column(8, 8).height.max(SEA + 1);
        for z in 2..14 {
            for x in 2..14 {
                for y in floor - 6..floor {
                    set(x, y, z, "stone", &mut blocks);
                }
                set(x, floor, z, "oak_planks", &mut blocks);
                for y in floor + 1..floor + 10 {
                    set(x, y, z, "air", &mut blocks);
                }
            }
        }
        let on = floor + 1;
        set(8, on, 8, "campfire", &mut blocks);
        for k in 4..=12 {
            for (x, z) in [(k, 4), (k, 12), (4, k), (12, k)] {
                set(x, on, z, "oak_fence", &mut blocks);
            }
        }
        for (x, z) in [(4, 4), (12, 4), (4, 12), (12, 12)] {
            set(x, on + 1, z, "torch", &mut blocks);
        }
        set(8, on, 12, "air", &mut blocks);
        for z in 13..16 {
            set(8, on - 1, z, "oak_slab", &mut blocks);
        }
        set(6, on, 6, "chest", &mut blocks);
        set(10, on, 6, "red_carpet", &mut blocks);
        set(10, on, 7, "red_carpet", &mut blocks);
        set(6, on, 10, "lantern", &mut blocks);
        for x in 5..12 {
            set(x, on, 2, "rail", &mut blocks);
        }
        for z in 5..12 {
            set(13, on, z, "cobblestone_wall", &mut blocks);
        }
        set(2, on, 8, "glass_pane", &mut blocks);
        set(2, on + 1, 8, "glass_pane", &mut blocks);
        // Dripstone, and a pool of water and one of lava (both animated in the game).
        set(5, on, 8, "dripstone_block", &mut blocks);
        set(5, on + 1, 8, "pointed_dripstone", &mut blocks);
        set(6, on, 8, "pointed_dripstone", &mut blocks);
        // A tall stack standing on the floor (base, middle, frustum, tip), and one hanging from
        // the room's ceiling below.
        for y in on..on + 4 {
            set(3, y, 4, "pointed_dripstone", &mut blocks);
        }
        for y in floor - 4..floor - 1 {
            set(7, y, 7, "pointed_dripstone", &mut blocks);
        }
        for (x, z) in [(10, 9), (11, 9), (10, 10), (11, 10)] {
            set(x, on - 1, z, "water", &mut blocks);
        }
        // Plants growing in the pool (water around them).
        set(10, on - 1, 9, "seagrass", &mut blocks);
        set(11, on - 1, 10, "kelp", &mut blocks);
        set(9, on - 1, 10, "lava", &mut blocks);
        set(9, on - 1, 9, "magma_block", &mut blocks);
        // A closed room under the floor (dark but for a torch, a glowstone and a lantern).
        for z in 4..12 {
            for x in 4..12 {
                for y in floor - 5..floor - 1 {
                    set(x, y, z, "air", &mut blocks);
                }
            }
        }
        set(5, floor - 5, 5, "torch", &mut blocks);
        set(10, floor - 5, 10, "glowstone", &mut blocks);
        set(10, floor - 5, 5, "lantern", &mut blocks);
        // A flower on the floor, for picking plants.
        set(7, on, 10, "poppy", &mut blocks);
        // Water running out of the pool across the floor, getting lower as it goes.
        for (k, x) in (3..9).enumerate() {
            set(x, on, 12, if k == 0 { "water" } else { ["water[level=1]", "water[level=2]", "water[level=3]", "water[level=4]", "water[level=5]"][k - 1] }, &mut blocks);
        }
        // And lava running beside it (lava steps down two levels a block).
        for (k, x) in (3..7).enumerate() {
            set(x, on, 14, ["lava", "lava[level=2]", "lava[level=4]", "lava[level=6]"][k], &mut blocks);
        }
    }
    let mut sections = Vec::new();
    for s in 0..SECTIONS as usize {
        let mut sec = Compound::new();
        sec.insert("Y".into(), Tag::Byte((MIN_Y / 16 + s as i32) as i8));
        let mut names: Vec<String> = Vec::new();
        let idx: Vec<usize> = blocks[s]
            .iter()
            .map(|b| match names.iter().position(|n| n == b) {
                Some(i) => i,
                None => {
                    names.push(b.to_string());
                    names.len() - 1
                }
            })
            .collect();
        let mut bs = Compound::new();
        bs.insert("palette".into(), palette_list(&names, true));
        if names.len() > 1 {
            let bits = (usize::BITS - (names.len() - 1).leading_zeros()).max(4) as usize;
            bs.insert("data".into(), Tag::LongArray(pack(&idx, bits)));
        }
        sec.insert("block_states".into(), Tag::Compound(bs));
        let mut bnames: Vec<String> = Vec::new();
        let bidx: Vec<usize> = biomes[s]
            .iter()
            .map(|b| match bnames.iter().position(|n| n == b) {
                Some(i) => i,
                None => {
                    bnames.push(b.to_string());
                    bnames.len() - 1
                }
            })
            .collect();
        let mut bm = Compound::new();
        bm.insert("palette".into(), palette_list(&bnames, false));
        if bnames.len() > 1 {
            let bits = (usize::BITS - (bnames.len() - 1).leading_zeros()) as usize;
            bm.insert("data".into(), Tag::LongArray(pack(&bidx, bits)));
        }
        sec.insert("biomes".into(), Tag::Compound(bm));
        sections.push(Tag::Compound(sec));
    }
    let mut root = Compound::new();
    root.insert("DataVersion".into(), Tag::Int(3953));
    root.insert("xPos".into(), Tag::Int(cx));
    root.insert("zPos".into(), Tag::Int(cz));
    root.insert("yPos".into(), Tag::Int(MIN_Y / 16));
    root.insert("Status".into(), Tag::String("minecraft:full".into()));
    root.insert("LastUpdate".into(), Tag::Long(0));
    root.insert("InhabitedTime".into(), Tag::Long(0));
    root.insert("sections".into(), Tag::List(List::from_tags(TagType::Compound, sections).expect("compounds")));
    // The showcase's banner patterns and pot sherds, saved as the game saves block entities.
    if (cx, cz) == (1, 0) {
        let on = column(24, 8).height.max(SEA + 1) + 1;
        let s = |v: &str| Tag::String(v.into());
        let entity = |id: &str, x: i32, extra: Vec<(&str, Tag)>| {
            let mut c = Compound::new();
            c.insert("id".into(), s(id));
            c.insert("x".into(), Tag::Int(16 + x));
            c.insert("y".into(), Tag::Int(on));
            c.insert("z".into(), Tag::Int(1));
            for (k, v) in extra {
                c.insert(k.into(), v);
            }
            Tag::Compound(c)
        };
        let pattern = |p: &str, colour: &str| {
            let mut c = Compound::new();
            c.insert("pattern".into(), s(p));
            c.insert("color".into(), s(colour));
            Tag::Compound(c)
        };
        let patterns = List::from_tags(TagType::Compound, vec![pattern("minecraft:stripe_top", "red"), pattern("minecraft:creeper", "black")]).expect("compounds");
        let sherds = ["minecraft:angler_pottery_sherd", "minecraft:brick", "minecraft:heart_pottery_sherd", "minecraft:skull_pottery_sherd"];
        let sherds = List::from_tags(TagType::String, sherds.iter().map(|v| s(v)).collect()).expect("strings");
        // The chests along the north edge (a single one, then the two halves of a large one),
        // as the game saves them: with their stacks.
        let stack = |id: &str, slot: i8, count: i32| {
            let mut c = Compound::new();
            c.insert("id".into(), s(id));
            c.insert("count".into(), Tag::Int(count));
            c.insert("Slot".into(), Tag::Byte(slot));
            Tag::Compound(c)
        };
        let chest = |x: i32, items: Vec<Tag>| {
            let items = if items.is_empty() { List::new(TagType::End) } else { List::from_tags(TagType::Compound, items).expect("compounds") };
            entity("minecraft:chest", x, vec![("Items", Tag::List(items))])
        };
        let list = vec![
            entity("minecraft:banner", 12, vec![("patterns", Tag::List(patterns))]),
            entity("minecraft:decorated_pot", 13, vec![("sherds", Tag::List(sherds))]),
            chest(2, vec![stack("minecraft:apple", 0, 3)]),
            chest(3, vec![stack("minecraft:diamond", 4, 2)]),
            chest(4, vec![]),
        ];
        root.insert("block_entities".into(), Tag::List(List::from_tags(TagType::Compound, list).expect("compounds")));
    }
    NamedTag { name: String::new(), tag: Tag::Compound(root) }
}

/// The showcase's entities (chunk (1, 0)): item frames and a painting on its wall, armour
/// stands and items on the floor, saved as the game saves them in `entities/`.
fn showcase_entities() -> NamedTag {
    let on = column(24, 8).height.max(SEA + 1) + 1;
    let s = |v: &str| Tag::String(v.into());
    let doubles = |v: [f64; 3]| Tag::List(List::from_tags(TagType::Double, v.iter().map(|d| Tag::Double(*d)).collect()).expect("doubles"));
    let floats = |v: &[f32]| Tag::List(List::from_tags(TagType::Float, v.iter().map(|d| Tag::Float(*d)).collect()).expect("floats"));
    // Ids as the game saves them, with their namespace.
    let named = |id: &str| if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") };
    let stack = |id: &str, count: i32, extra: Vec<(&str, Tag)>| {
        let mut c = Compound::new();
        c.insert("id".into(), s(&named(id)));
        c.insert("count".into(), Tag::Int(count));
        for (k, v) in extra {
            c.insert(k.into(), v);
        }
        Tag::Compound(c)
    };
    let entity = |id: &str, pos: [f64; 3], extra: Vec<(&str, Tag)>| {
        let mut c = Compound::new();
        c.insert("id".into(), s(&named(id)));
        c.insert("Pos".into(), doubles(pos));
        c.insert("Rotation".into(), floats(&[0.0, 0.0]));
        for (k, v) in extra {
            c.insert(k.into(), v);
        }
        Tag::Compound(c)
    };
    let (x0, y) = (16.0, on as f64);
    let mut list = Vec::new();
    // Item frames on the wall (facing south), one block up.
    let frames: [(&str, &str, i64); 5] = [("item_frame", "diamond_sword", 0), ("item_frame", "stone", 0), ("item_frame", "apple", 1), ("glow_item_frame", "torch", 0), ("item_frame", "oak_stairs", 2)];
    for (k, (kind, item, rot)) in frames.iter().enumerate() {
        let x = 2 + k as i32;
        list.push(entity(kind, [x0 + x as f64 + 0.5, y + 1.5, 1.03125], vec![
            ("TileX", Tag::Int(16 + x)), ("TileY", Tag::Int(on + 1)), ("TileZ", Tag::Int(1)), ("Facing", Tag::Byte(3)),
            ("Item", stack(item, 1, vec![])), ("ItemRotation", Tag::Byte(*rot as i8)),
        ]));
    }
    // A painting two wide beside them.
    list.push(entity("painting", [x0 + 10.0, y + 2.5, 1.03125], vec![
        ("TileX", Tag::Int(16 + 9)), ("TileY", Tag::Int(on + 2)), ("TileZ", Tag::Int(1)), ("facing", Tag::Byte(0)), ("variant", s("minecraft:pool")),
    ]));
    // Armour stands: one in armour with arms out, one small and bare.
    let pose = {
        let mut p = Compound::new();
        p.insert("RightArm".into(), floats(&[-60.0, 0.0, 10.0]));
        p.insert("LeftArm".into(), floats(&[-10.0, 0.0, -30.0]));
        p.insert("Head".into(), floats(&[0.0, 20.0, 0.0]));
        Tag::Compound(p)
    };
    let mut equipment = Compound::new();
    let mut dye = Compound::new();
    dye.insert("minecraft:dyed_color".into(), Tag::Int(0x3060C0));
    equipment.insert("head".into(), stack("diamond_helmet", 1, vec![]));
    equipment.insert("chest".into(), stack("iron_chestplate", 1, vec![]));
    equipment.insert("legs".into(), stack("leather_leggings", 1, vec![("components", Tag::Compound(dye))]));
    equipment.insert("feet".into(), stack("golden_boots", 1, vec![]));
    list.push(entity("armor_stand", [x0 + 9.5, y, 3.5], vec![("ShowArms", Tag::Byte(1)), ("Pose", pose), ("equipment", Tag::Compound(equipment))]));
    list.push(entity("armor_stand", [x0 + 11.5, y, 3.5], vec![("Small", Tag::Byte(1))]));
    // Items lying on the floor.
    for (k, (item, count)) in [("diamond", 1), ("stone", 1), ("iron_sword", 1), ("oak_log", 20), ("apple", 40)].iter().enumerate() {
        list.push(entity("item", [x0 + 2.5 + k as f64 * 1.5, y, 5.5], vec![("Item", stack(item, *count, vec![]))]));
    }
    let mut root = Compound::new();
    root.insert("DataVersion".into(), Tag::Int(3953));
    root.insert("Position".into(), Tag::IntArray(vec![1, 0]));
    root.insert("Entities".into(), Tag::List(List::from_tags(TagType::Compound, list).expect("compounds")));
    NamedTag { name: String::new(), tag: Tag::Compound(root) }
}

/// The mob zoo (chunk (2, 0)): one of each mob in rows, facing south.
fn zoo_entities() -> NamedTag {
    let on = column(24, 8).height.max(SEA + 1) + 1;
    let s = |v: &str| Tag::String(v.into());
    let doubles = |v: [f64; 3]| Tag::List(List::from_tags(TagType::Double, v.iter().map(|d| Tag::Double(*d)).collect()).expect("doubles"));
    let floats = |v: &[f32]| Tag::List(List::from_tags(TagType::Float, v.iter().map(|d| Tag::Float(*d)).collect()).expect("floats"));
    let mobs: [(&str, Vec<(&str, Tag)>); 42] = [
        ("zombie", vec![]), ("husk", vec![]), ("drowned", vec![]), ("skeleton", vec![]), ("stray", vec![]), ("wither_skeleton", vec![]),
        ("creeper", vec![]), ("spider", vec![]), ("cave_spider", vec![]), ("enderman", vec![]), ("piglin", vec![]), ("zombified_piglin", vec![]),
        ("pig", vec![("variant", s("minecraft:temperate"))]), ("cow", vec![("variant", s("minecraft:temperate"))]), ("sheep", vec![("Color", Tag::Byte(14))]),
        ("chicken", vec![("variant", s("minecraft:temperate"))]), ("wolf", vec![]), ("mooshroom", vec![("Type", s("red"))]),
        ("villager", vec![]), ("wandering_trader", vec![]), ("witch", vec![]), ("pillager", vec![]), ("vindicator", vec![]), ("zombie_villager", vec![]),
        ("iron_golem", vec![]), ("slime", vec![("Size", Tag::Int(1))]), ("magma_cube", vec![("Size", Tag::Int(1))]), ("squid", vec![]),
        ("pig", vec![("variant", s("minecraft:cold")), ("Age", Tag::Int(-1000))]), ("fox", vec![]),
        ("cat", vec![("variant", s("minecraft:calico"))]), ("ocelot", vec![]), ("horse", vec![("Variant", Tag::Int(3 | (2 << 8)))]), ("donkey", vec![]),
        ("bee", vec![]), ("goat", vec![]), ("llama", vec![("Variant", Tag::Int(2))]), ("polar_bear", vec![]), ("panda", vec![]),
        ("parrot", vec![("Variant", Tag::Int(0))]), ("snow_golem", vec![]), ("rabbit", vec![]),
    ];
    let mut list = Vec::new();
    for (k, (id, extra)) in mobs.into_iter().enumerate() {
        let (col, row) = ((k % 7) as f64, (k / 7) as f64);
        let mut c = Compound::new();
        c.insert("id".into(), s(&format!("minecraft:{id}")));
        c.insert("Pos".into(), doubles([32.0 + 1.3 + col * 2.2, on as f64, 1.3 + row * 2.6]));
        c.insert("Rotation".into(), floats(&[0.0, 0.0]));
        c.insert("Health".into(), Tag::Float(20.0));
        if id == "villager" {
            let mut d = Compound::new();
            d.insert("type".into(), s("minecraft:plains"));
            d.insert("profession".into(), s("minecraft:farmer"));
            d.insert("level".into(), Tag::Int(1));
            c.insert("VillagerData".into(), Tag::Compound(d));
        }
        for (k, v) in extra {
            c.insert(k.into(), v);
        }
        list.push(Tag::Compound(c));
    }
    let mut root = Compound::new();
    root.insert("DataVersion".into(), Tag::Int(3953));
    root.insert("Position".into(), Tag::IntArray(vec![2, 0]));
    root.insert("Entities".into(), Tag::List(List::from_tags(TagType::Compound, list).expect("compounds")));
    NamedTag { name: String::new(), tag: Tag::Compound(root) }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = PathBuf::from(std::env::args().nth(1).ok_or("usage: gen_test_world <out dir> [chunks across]")?);
    let across: i32 = std::env::args().nth(2).map_or(Ok(64), |a| a.parse())?;
    let region_dir = out.join("region");
    std::fs::create_dir_all(&region_dir)?;
    // level.dat: just enough for the world to be recognised and named.
    let mut data = Compound::new();
    data.insert("LevelName".into(), Tag::String("Explorer Test".into()));
    data.insert("DataVersion".into(), Tag::Int(3953));
    data.insert("SpawnX".into(), Tag::Int(0));
    data.insert("SpawnY".into(), Tag::Int(80));
    data.insert("SpawnZ".into(), Tag::Int(0));
    data.insert("LastPlayed".into(), Tag::Long(0));
    let mut version = Compound::new();
    version.insert("Name".into(), Tag::String("1.21".into()));
    version.insert("Id".into(), Tag::Int(3953));
    data.insert("Version".into(), Tag::Compound(version));
    let mut root = Compound::new();
    root.insert("Data".into(), Tag::Compound(data));
    let raw = mctk_nbt::to_bytes(&NamedTag { name: String::new(), tag: Tag::Compound(root) }, mctk_nbt::Flavor::Java)?;
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(&raw)?;
    std::fs::write(out.join("level.dat"), gz.finish()?)?;

    let half = across / 2;
    let mut regions: std::collections::BTreeMap<(i32, i32), RegionBuilder> = Default::default();
    for cz in -half..across - half {
        for cx in -half..across - half {
            let nbt = chunk_nbt(cx, cz);
            let chunk = RawChunk::encode(&nbt, ChunkCompression::Zlib, 0)?;
            regions
                .entry((cx.div_euclid(32), cz.div_euclid(32)))
                .or_default()
                .set(cx.rem_euclid(32) as usize, cz.rem_euclid(32) as usize, Some(chunk));
        }
        eprint!("\rrow {} of {across}", cz + half + 1);
    }
    eprintln!();
    for ((rx, rz), b) in &regions {
        let path = b.write(&region_dir, RegionPos { x: *rx, z: *rz })?;
        println!("wrote {}", path.display());
    }
    // The showcase's entities, in their own region file (as 1.17 and later keep them).
    let entities_dir = out.join("entities");
    std::fs::create_dir_all(&entities_dir)?;
    let mut ents = RegionBuilder::default();
    ents.set(1, 0, Some(RawChunk::encode(&showcase_entities(), ChunkCompression::Zlib, 0)?));
    ents.set(2, 0, Some(RawChunk::encode(&zoo_entities(), ChunkCompression::Zlib, 0)?));
    let path = ents.write(&entities_dir, RegionPos { x: 0, z: 0 })?;
    println!("wrote {}", path.display());
    Ok(())
}
