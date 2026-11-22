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

/// Approximate map color for a block name (no game textures are used).
pub fn block_color(name: &str) -> [u8; 3] {
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
            // Stable muted color for anything else.
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
pub(crate) fn legacy_name(id: u16) -> &'static str {
    match id {
        1 => "stone",
        2 => "grass_block",
        3 => "dirt",
        4 => "cobblestone",
        5 => "planks",
        7 => "bedrock",
        // Flowing water and lava (8, 10), level not known: drawn as flowing halfway.
        8 => "water#4",
        9 => "water",
        10 => "lava#4",
        11 => "lava",
        12 => "sand",
        13 => "gravel",
        14..=16 | 21 | 56 | 73 | 129 => "ore",
        17 | 162 => "_log",
        18 | 161 => "leaves",
        20 => "glass",
        24 => "sandstone",
        31 => "short_grass",
        32 => "dead_bush",
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

/// What kind of thing a block is for the map: ground, something that grows on it (shown over the
/// ground beneath), leaves (which are see-through), or water.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Solid,
    Plant,
    Leaves,
    Water,
}

/// A block name worked out once: its number on the map, its color and its kind.
#[derive(Clone, Copy)]
struct Resolved {
    id: u16,
    rgb: [u8; 3],
    kind: Kind,
    cover: f32,
    /// Water itself (a source or flowing), not something growing in it.
    liquid: bool,
}

/// One block column of a chunk as the map sees it: the color, how high it is, how deep the water
/// over it is, the block it shows and (for a plant or leaves) the ground showing through below.
#[derive(Clone, Copy)]
struct Column {
    rgb: [u8; 3],
    y: i32,
    water: i32,
    /// The block drawn at the bottom: the top block, or the ground under a plant or leaves.
    base: u16,
    /// A plant or leaves drawn over `base`; 0 when there is none.
    over: u16,
    /// The height of `base` (the ground, or the bottom under water) and of `over`.
    gy: i32,
    oy: i32,
}

/// Names of the blocks on the map, shared by every chunk drawn; id 0 is "nothing here".
static NAMES: std::sync::LazyLock<Mutex<(Vec<String>, std::collections::HashMap<String, u16>)>> =
    std::sync::LazyLock::new(|| Mutex::new((Vec::new(), std::collections::HashMap::new())));

/// The number a block name is known by on the map (the same name always gets the same number).
pub(crate) fn intern(name: &str) -> u16 {
    let n = name.trim_start_matches("minecraft:");
    let mut g = NAMES.lock().expect("block names");
    if let Some(i) = g.1.get(n) {
        return *i;
    }
    g.0.push(n.to_string());
    let id = g.0.len() as u16;
    g.1.insert(n.to_string(), id);
    id
}

/// The block name a map number stands for.
pub fn block_name(id: u16) -> Option<String> {
    let g = NAMES.lock().ok()?;
    g.0.get((id as usize).checked_sub(1)?).cloned()
}

/// The block states (`facing=east,half=bottom`) of numbers given out by [`intern_state`].
static STATES: std::sync::LazyLock<Mutex<std::collections::HashMap<u16, std::sync::Arc<str>>>> = std::sync::LazyLock::new(Default::default);

/// The number for a block name with its full block state (`props` as `key=value` pairs joined
/// by commas, in any order): a number of its own, whose [`block_name`] is `name` as usual and
/// whose [`block_props`] are the state. For the 3D view, which draws every state as the game does.
/// (Once numbers run short, the name alone.)
pub(crate) fn intern_state(name: &str, props: &str) -> u16 {
    if props.is_empty() {
        return intern(name);
    }
    let n = name.trim_start_matches("minecraft:");
    let key = format!("{n}\u{1}{props}");
    let mut g = NAMES.lock().expect("block names");
    if let Some(i) = g.1.get(&key) {
        return *i;
    }
    if g.0.len() >= 64_000 {
        drop(g);
        return intern(name);
    }
    g.0.push(n.to_string());
    let id = g.0.len() as u16;
    g.1.insert(key, id);
    drop(g);
    STATES.lock().expect("block states").insert(id, props.into());
    id
}

/// The block state a number was given with (see [`intern_state`]), if any.
pub(crate) fn block_props(id: u16) -> Option<std::sync::Arc<str>> {
    STATES.lock().ok()?.get(&id).cloned()
}

/// What the game multiplies a block's tinted faces by (0 to 1 a channel): its biome's grass or
/// leaf color, water's blue, redstone dust's red by its power, a stem's color by its age.
/// `name` is the map name (with the biome after `@` for grass and leaves).
pub(crate) fn block_tint(name: &str, props: &str) -> [f32; 3] {
    let n = name.trim_start_matches("minecraft:");
    let plain = bare(n);
    let biome = n.split_once('@').map(|(_, b)| b);
    let (grass, foliage) = biome.map_or(([145.0, 189.0, 89.0], [119.0, 171.0, 47.0]), biome_colors);
    let prop = |k: &str| props.split(',').find_map(|p| p.strip_prefix(k)?.strip_prefix('=')).and_then(|v| v.parse::<f32>().ok());
    let hex = |c: u32| [(c >> 16) as f32, ((c >> 8) & 255) as f32, (c & 255) as f32];
    let rgb = match plain {
        "birch_leaves" => hex(0x80A755),
        "spruce_leaves" => hex(0x619961),
        "lily_pad" => hex(0x208030),
        "water" | "flowing_water" | "water_cauldron" | "bubble_column" => {
            if biome.is_some_and(|b| b.contains("swamp")) { hex(0x617B64) } else { hex(0x3F76E4) }
        }
        "redstone_wire" => {
            let f = prop("power").unwrap_or(0.0) / 15.0;
            let r = if f > 0.0 { f * 0.6 + 0.4 } else { 0.3 };
            return [r, (f * f * 0.7 - 0.5).max(0.0), (f * f * 0.6 - 0.7).max(0.0)];
        }
        "melon_stem" | "pumpkin_stem" => {
            let a = prop("age").unwrap_or(0.0);
            [a * 32.0, 255.0 - a * 8.0, a * 4.0]
        }
        "attached_melon_stem" | "attached_pumpkin_stem" => hex(0xE0C71C),
        "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves" | "mangrove_leaves" | "vine" => foliage,
        "grass_block" | "short_grass" | "tall_grass" | "fern" | "large_fern" | "potted_fern" | "sugar_cane" | "bush" | "pink_petals" | "wildflowers" | "leaf_litter" => grass,
        // Only the blocks the game gives a color are tinted: a tinted face of any other (the
        // stonecutter's saw) is drawn as it is.
        _ => [255.0; 3],
    };
    rgb.map(|c| c / 255.0)
}

/// Whether a block grows on the ground rather than being the ground: flowers, grass, ferns...
pub(crate) fn is_plant(n: &str) -> bool {
    const EXACT: &[&str] = &[
        "short_grass", "tall_grass", "fern", "large_fern", "dead_bush", "vine", "lily_pad", "sugar_cane",
        "flower", "hanging_roots", "glow_lichen", "sweet_berry_bush", "brown_mushroom", "red_mushroom",
        "crimson_roots", "warped_roots", "nether_sprouts", "crimson_fungus", "warped_fungus",
        "spore_blossom", "pink_petals", "leaf_litter", "wildflowers", "bush", "firefly_bush",
        "cactus_flower", "torchflower", "pitcher_plant", "small_dripleaf", "poppy", "dandelion",
        "blue_orchid", "allium", "azure_bluet", "oxeye_daisy", "cornflower", "lily_of_the_valley",
        "wither_rose", "sunflower", "lilac", "rose_bush", "peony", "eyeblossom", "open_eyeblossom",
        "closed_eyeblossom", "seagrass_top", "fire", "snow_layer",
    ];
    EXACT.contains(&n) || n.ends_with("_sapling") || n.ends_with("_tulip") || n.starts_with("potted_")
}

/// A block name without its axis ("#x") or biome ("@swamp") suffix.
fn bare(n: &str) -> &str {
    n.split(['#', '@']).next().unwrap_or(n)
}

fn kind_of(name: &str) -> Kind {
    // A name may carry an axis ("oak_log#x" for a log lying along x): the kind is the block's.
    let n = bare(name.trim_start_matches("minecraft:"));
    if is_water(n) {
        Kind::Water
    } else if n.ends_with("_leaves") || n == "leaves" {
        Kind::Leaves
    } else if is_plant(n) {
        Kind::Plant
    } else {
        Kind::Solid
    }
}

thread_local! {
    /// Block names already worked out on this thread, so the shared tables are not touched for
    /// every column.
    static RESOLVED: std::cell::RefCell<std::collections::HashMap<String, Resolved>> = Default::default();
}

fn resolve(name: &str) -> Resolved {
    RESOLVED.with(|r| {
        let mut r = r.borrow_mut();
        if let Some(x) = r.get(name) {
            return *x;
        }
        let n = name.trim_start_matches("minecraft:");
        let kind = kind_of(n);
        // How much of its square a plant or leaves covers (the rest shows the ground).
        let cover = texture_info(n.split('@').next().unwrap_or(n)).map_or(
            if kind == Kind::Leaves { 0.8 } else { 0.3 },
            |(_, c)| c,
        );
        let liquid = matches!(bare(n), "water" | "flowing_water");
        let res = Resolved { id: intern(n), rgb: surface_color(n), kind, cover, liquid };
        r.insert(name.to_string(), res);
        res
    })
}

/// Works out a column from the blocks `each` reports from the top down (`each` calls the closure
/// with a name and a height for every block that is not air, and stops when it returns false).
/// A plant or leaves is shown over the ground beneath it; water over what is beneath, deeper
/// the more of it there is.
fn make_column(each: impl FnOnce(&mut dyn FnMut(&str, i32) -> bool)) -> Option<Column> {
    let mut top: Option<(Resolved, i32)> = None;
    let mut ground: Option<(Resolved, i32)> = None;
    let (mut water, mut below) = (0, 0);
    each(&mut |name, y| {
        // Blocks that cannot be seen (barriers, light blocks, structure voids) are left out, as
        // the game shows them: a world floored with them reads as the void it looks like.
        let bare = name.trim_start_matches("minecraft:");
        let bare = bare.split(['#', '@', '[']).next().unwrap_or(bare);
        if matches!(bare, "barrier" | "light" | "structure_void" | "air" | "cave_air" | "void_air" | "moving_piston") {
            return true;
        }
        let r = resolve(name);
        match top {
            None => {
                top = Some((r, y));
                if r.kind == Kind::Water {
                    water = 1;
                }
                r.kind != Kind::Solid
            }
            Some((t, _)) => {
                below += 1;
                if t.kind == Kind::Water {
                    if r.kind == Kind::Water && water < 80 {
                        water += 1;
                        return true;
                    }
                    // What is under the water shows through it.
                    if r.kind != Kind::Water {
                        ground = Some((r, y));
                    }
                    return false;
                }
                if matches!(r.kind, Kind::Plant | Kind::Leaves) {
                    return below < 40;
                }
                ground = Some((r, y));
                false
            }
        }
    });
    let (t, y) = top?;
    Some(match (t.kind, ground) {
        (Kind::Plant | Kind::Leaves, Some((g, gy))) => {
            // Leaves hide most of what is under them; a flower only a little.
            let k = t.cover.clamp(0.1, 0.9);
            let rgb = [0, 1, 2].map(|i| (g.rgb[i] as f32 * (1.0 - k) + t.rgb[i] as f32 * k) as u8);
            // A flower does not change the lie of the land; a canopy stands a little above it.
            let relief = if t.kind == Kind::Leaves { gy + 3 } else { gy };
            Column { rgb, y: relief, water: 0, base: g.id, over: t.id, gy, oy: y }
        }
        // Water is always drawn as water (even over kelp), deeper the more of it there is.
        (Kind::Water, g) => {
            let gy = g.map_or(y - water, |(_, gy)| gy);
            // The water block itself (a source, or flowing at some level); kelp and the like
            // under the surface are drawn as water.
            let base = if t.liquid { t.id } else { intern("water") };
            Column { rgb: g.map_or(t.rgb, |(g, _)| g.rgb), y, water, base, over: 0, gy, oy: y }
        }
        _ => Column { rgb: t.rgb, y, water: 0, base: t.id, over: 0, gy: y, oy: y },
    })
}

/// A drawn chunk: its pixels (lit as far as the chunk itself allows, for showing while the rest is
/// still being drawn) and what is needed to light it properly and to draw it in detail.
#[derive(Clone)]
pub struct Painted {
    /// 16 x 16 RGBA.
    pub px: Vec<u8>,
    /// The block at the bottom of each column, and the plant or leaves over it (0 for none).
    pub base: Vec<u16>,
    pub over: Vec<u16>,
    /// The unlit color of each column (RGB), its height for lighting, and its water depth.
    pub albedo: Vec<u8>,
    pub y: Vec<i16>,
    pub water: Vec<u8>,
    /// The height of the ground block, of the plant or leaves over it (see [`MapImage`]), and the
    /// biome the column stands in (see [`biome_name`]; 0 when the save does not say).
    pub ground: Vec<i16>,
    pub top: Vec<i16>,
    pub biome: Vec<u16>,
}

/// Names of the biomes on the map, as for blocks; id 0 is "not known".
static BIOME_NAMES: std::sync::LazyLock<Mutex<(Vec<String>, std::collections::HashMap<String, u16>)>> =
    std::sync::LazyLock::new(|| Mutex::new((Vec::new(), std::collections::HashMap::new())));

thread_local! {
    static BIOME_CACHE: std::cell::RefCell<std::collections::HashMap<String, u16>> = Default::default();
}

/// The number a biome name is known by on the map.
fn intern_biome(name: &str) -> u16 {
    let n = name.trim_start_matches("minecraft:");
    BIOME_CACHE.with(|c| {
        if let Some(i) = c.borrow().get(n) {
            return *i;
        }
        let id = {
            let mut g = BIOME_NAMES.lock().expect("biome names");
            match g.1.get(n) {
                Some(i) => *i,
                None => {
                    g.0.push(n.to_string());
                    let id = g.0.len() as u16;
                    g.1.insert(n.to_string(), id);
                    id
                }
            }
        };
        c.borrow_mut().insert(n.to_string(), id);
        id
    })
}

/// The number a biome name is known by on the map (for patching the picture after biome edits).
pub(crate) fn map_biome(name: &str) -> u16 {
    intern_biome(name)
}

/// The biome name a map number stands for (without "minecraft:").
pub fn biome_name(id: u16) -> Option<String> {
    let g = BIOME_NAMES.lock().ok()?;
    g.0.get((id as usize).checked_sub(1)?).cloned()
}

/// Biome names by numeric id: Bedrock's own where known, else the legacy (console and pre-1.13
/// Java) numbering, which Bedrock kept for the older biomes.
fn numeric_biome(id: i32) -> Option<&'static str> {
    static TABLE: std::sync::LazyLock<std::collections::HashMap<i32, &'static str>> = std::sync::LazyLock::new(|| {
        let mut m = std::collections::HashMap::new();
        for l in mctk_registry::builtin::legacy_biomes_text().lines().filter(|l| !l.starts_with('#')) {
            if let Some((id, name)) = l.split_once('\t') {
                if let Ok(id) = id.trim().parse() {
                    m.insert(id, name.trim());
                }
            }
        }
        for (name, id) in mctk_registry::builtin::bedrock_biome_ids("1.26.61.1") {
            m.insert(id, name);
        }
        m
    });
    TABLE.get(&id).copied()
}

/// The color of a column once lit. Ground is lit from the north-west, as on a vanilla map: lighter
/// where it is higher than the columns north and west of it, darker where lower, and a touch
/// lighter the higher it is. Water goes from clear shallows to deep blue and is not lit by slope.
/// `soften` (1 for single blocks) weakens the slope when each column stands for several.
pub fn lit(albedo: [u8; 3], y: i32, water: i32, north: Option<i32>, west: Option<i32>, soften: f32) -> [u8; 3] {
    if water > 0 {
        // Water of the color its depth gives over the bottom, which shows more the shallower it is.
        let t = (water as f32 / 24.0).clamp(0.0, 1.0);
        let (shallow, deep) = ([70.0, 120.0, 225.0], [24.0, 46.0, 128.0]);
        let opaque = 0.4 + 0.55 * (1.0 - (-(water as f32) / 7.0).exp());
        return [0, 1, 2].map(|k| {
            let w = shallow[k] + (deep[k] - shallow[k]) * t;
            (w * opaque + albedo[k] as f32 * 0.8 * (1.0 - opaque)) as u8
        });
    }
    let d = 0.6 * north.map_or(0, |n| y - n) as f32 + 0.4 * west.map_or(0, |w| y - w) as f32;
    let light = 1.0 + (d * soften).clamp(-5.0, 5.0) * 0.09 + ((y as f32 - 62.0) / 220.0).clamp(-0.22, 0.3);
    albedo.map(|c| (c as f32 * light).clamp(0.0, 255.0) as u8)
}

/// The chunk's 16 x 16 columns made into a [`Painted`].
/// `biome_at(x, z, y)` gives the biome number at a column's surface.
fn paint(cols: &[Option<Column>; 256], biome_at: impl Fn(usize, usize, i32) -> u16) -> Painted {
    let mut px = vec![0u8; 16 * 16 * 4];
    let (mut base, mut over) = (vec![0u16; 256], vec![0u16; 256]);
    let (mut albedo, mut ys, mut water) = (vec![0u8; 768], vec![0i16; 256], vec![0u8; 256]);
    let (mut ground, mut top, mut biome) = (vec![0i16; 256], vec![0i16; 256], vec![0u16; 256]);
    for z in 0..16 {
        for x in 0..16 {
            let Some(c) = cols[z * 16 + x] else { continue };
            let n = (z > 0).then(|| cols[(z - 1) * 16 + x]).flatten().map(|c| c.y);
            let w = (x > 0).then(|| cols[z * 16 + x - 1]).flatten().map(|c| c.y);
            let rgb = lit(c.rgb, c.y, c.water, n, w, 1.0);
            let i = (z * 16 + x) * 4;
            px[i..i + 3].copy_from_slice(&rgb);
            px[i + 3] = 255;
            let k = z * 16 + x;
            base[k] = c.base;
            over[k] = c.over;
            albedo[k * 3..k * 3 + 3].copy_from_slice(&c.rgb);
            ys[k] = c.y.clamp(-32768, 32767) as i16;
            water[k] = c.water.clamp(0, 255) as u8;
            ground[k] = c.gy.clamp(-32768, 32767) as i16;
            top[k] = c.oy.clamp(-32768, 32767) as i16;
            // The biome where the column meets the air (the surface of water, the ground of land).
            biome[k] = biome_at(x, z, if c.water > 0 { c.y } else { c.gy });
        }
    }
    Painted { px, base, over, albedo, y: ys, water, ground, top, biome }
}
/// The average color of a block's texture in the resource pack, if the pack has one. Worked
/// out once per name (the decoding is done without holding the shared list).
fn texture_info(name: &str) -> Option<([u8; 3], f32)> {
    type Averages = std::collections::HashMap<String, Option<([u8; 3], f32)>>;
    static CACHE: std::sync::LazyLock<Mutex<(Option<std::path::PathBuf>, Averages)>> =
        std::sync::LazyLock::new(|| Mutex::new((None, Averages::new())));
    let pack = crate::icons::pack_texture("block/stone.png");
    {
        let mut cache = CACHE.lock().ok()?;
        if cache.0 != pack {
            *cache = (pack.clone(), Averages::new());
        }
        if let Some(c) = cache.1.get(name) {
            return *c;
        }
    }
    // "oak_log#x": a block on its side, seen from above as its side texture.
    let (base, sided) = name.split_once('#').map_or((name, false), |(b, _)| (b, true));
    let files = texture_files(base, sided);
    let found = files
        .iter()
        .find_map(|f| crate::icons::pack_texture(f))
        .and_then(|f| image::open(f).ok())
        .and_then(|img| {
            let img = img.to_rgba8();
            let (mut sum, mut n) = ([0u64; 3], 0u64);
            for p in img.pixels().filter(|p| p[3] > 128) {
                for k in 0..3 {
                    sum[k] += p[k] as u64;
                }
                n += 1;
            }
            let cover = n as f32 / img.pixels().len().max(1) as f32;
            (n > 0).then(|| ([0, 1, 2].map(|k| (sum[k] / n) as u8), cover))
        });
    CACHE.lock().ok()?.1.insert(name.to_string(), found);
    found
}

/// The pack files a block's top might be drawn from, best first (a block on its side shows its
/// side; stairs, slabs and the like show the block they are made of).
pub fn texture_files(base: &str, sided: bool) -> Vec<String> {
    let mut f = Vec::new();
    let mut add = |n: String| f.push(format!("block/{n}.png"));
    if sided {
        add(base.to_string());
        add(format!("{base}_side"));
        return f;
    }
    for suffix in ["_top", "", "_still", "_side", "_front", "_0"] {
        add(format!("{base}{suffix}"));
    }
    // Blocks whose picture is filed under another name.
    match base {
        "magma_block" => add("magma".into()),
        "chest" | "trapped_chest" | "ender_chest" | "barrel" => add("oak_planks".into()),
        _ => {}
    }
    for suffix in ["_stage3", "_up_tip", "_log", "_log_lit", "_planks", "_bottom"] {
        add(format!("{base}{suffix}"));
    }
    for tail in ["_stairs", "_slab", "_wall", "_fence_gate", "_fence", "_button", "_pressure_plate", "_carpet", "_pane"] {
        if let Some(m) = base.strip_suffix(tail) {
            // Carpets are their wool.
            for n in [format!("{m}_top"), m.to_string(), format!("{m}_planks"), format!("{m}s"), format!("{m}_block"), format!("{m}_wool"), format!("{m}_side")] {
                add(n);
            }
        }
    }
    f
}

/// The average color of a block's texture, if the pack has one.
fn texture_average(name: &str) -> Option<[u8; 3]> {
    texture_info(name).map(|(c, _)| c)
}

/// The grass and foliage colors a biome gives (vanilla's, close enough for a map).
fn biome_colors(biome: &str) -> ([f32; 3], [f32; 3]) {
    let b = biome.trim_start_matches("minecraft:");
    let c = |g: u32, f: u32| ([(g >> 16) as f32, ((g >> 8) & 255) as f32, (g & 255) as f32], [(f >> 16) as f32, ((f >> 8) & 255) as f32, (f & 255) as f32]);
    if b.contains("dappled") { c(0xFF7D2D, 0xF0641E) }
    else if b.contains("mangrove") { c(0x8DB127, 0x8DB127) }
    else if b.contains("swamp") { c(0x6A7039, 0x6A7039) }
    else if b.contains("jungle") || b.contains("bamboo") { c(0x59C93C, 0x30BB0B) }
    else if b.contains("mushroom") { c(0x55C93F, 0x2BBB0F) }
    else if b.contains("wooded_badlands") || b.contains("eroded") || b.contains("badlands") { c(0x90814D, 0x9E814D) }
    else if b.contains("desert") || b.contains("savanna") { c(0xBFB755, 0xAEA42A) }
    else if b.contains("snowy") || b.contains("frozen") || b.contains("ice") || b.contains("grove") && !b.contains("cherry") || b.contains("cold") { c(0x80B497, 0x60A17B) }
    else if b.contains("cherry") { c(0xB6DB61, 0x9EDE36) }
    else if b.contains("pale_garden") { c(0x778272, 0x878D76) }
    else if b.contains("meadow") { c(0x83BB6D, 0x63A948) }
    else if b.contains("taiga") { c(0x86B783, 0x68A464) }
    else if b.contains("dark_forest") { c(0x507A32, 0x59AE30) }
    else if b.contains("birch") { c(0x88BB67, 0x6BA941) }
    else if b.contains("flower_forest") || b == "forest" { c(0x79C05A, 0x59AE30) }
    else if b.contains("windswept") || b.contains("hills") || b.contains("peaks") || b.contains("stony") || b.contains("gravelly") { c(0x8AB689, 0x6DA36B) }
    else if b.contains("ocean") || b.contains("river") || b.contains("beach") { c(0x8EB971, 0x71A74D) }
    else { c(0x91BD59, 0x77AB2F) }
}

/// The color the game tints grass with for a block's map name ("grass_block@plains"): its
/// biome's, or the plains' when the name gives none.
pub(crate) fn grass_color(name: &str) -> [f32; 3] {
    name.split_once('@').map_or([145.0, 189.0, 89.0], |(_, b)| biome_colors(b).0)
}

/// The color a block's top is drawn with: its texture's average when the pack has the texture
/// (tinted green for grass and leaves, which the pack draws gray), else a table's guess.
pub fn surface_color(name: &str) -> [u8; 3] {
    let n = name.trim_start_matches("minecraft:");
    let plain = bare(n);
    let biome = n.split_once('@').map(|(_, b)| b);
    let tinted = plain == "grass_block"
        || plain.ends_with("grass")
        || plain.ends_with("fern")
        || plain == "vine"
        || plain == "lily_pad"
        || matches!(plain, "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves" | "mangrove_leaves" | "birch_leaves" | "spruce_leaves" | "leaves");
    if let Some(t) = texture_average(n.split('@').next().unwrap_or(n)) {
        if tinted {
            // The pack draws these gray; the game multiplies them by the biome's color.
            let leaves = plain.ends_with("_leaves") || plain == "vine";
            let (g, f) = biome.map_or(([145.0, 189.0, 89.0], [119.0, 171.0, 47.0]), biome_colors);
            let tint = match plain {
                "birch_leaves" => [128.0, 167.0, 85.0],
                "spruce_leaves" => [97.0, 153.0, 97.0],
                _ if leaves => f,
                _ => g,
            };
            return [0, 1, 2].map(|k| (t[k] as f32 * tint[k] / 255.0).clamp(0.0, 255.0) as u8);
        }
        return t;
    }
    // Leaves the pack has no picture of (autumn trees...) go by the color in their name.
    if plain.ends_with("_leaves") {
        for (word, rgb) in [("red", [150, 40, 28]), ("orange", [200, 100, 25]), ("yellow", [200, 165, 40]), ("brown", [120, 80, 40])] {
            if plain.contains(word) {
                return rgb;
            }
        }
    }
    block_color(plain)
}
/// A rendered map: one pixel per block column, origin at chunk (`min_x`, `min_z`).
pub struct MapImage {
    pub min_x: i32,
    pub min_z: i32,
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    /// The block at the bottom of each column (see `block_name`), 0 where nothing was drawn.
    pub blocks: Vec<u16>,
    /// The plant or leaves drawn over it, 0 for none.
    pub over: Vec<u16>,
    /// The unlit color of each column (RGB), its height for lighting, and its water depth.
    pub albedo: Vec<u8>,
    pub heights: Vec<i16>,
    pub water: Vec<u8>,
    /// The true height of the ground block and of the plant or leaves over it (for the street
    /// view), and the biome of each column (0 when not known).
    pub ground: Vec<i16>,
    pub top: Vec<i16>,
    pub biome: Vec<u16>,
}

impl MapImage {
    /// An empty picture of the given size.
    pub fn blank(min_x: i32, min_z: i32, width: usize, height: usize) -> Self {
        let n = width * height;
        MapImage {
            min_x,
            min_z,
            width,
            height,
            rgba: vec![0; n * 4],
            blocks: vec![0; n],
            over: vec![0; n],
            albedo: vec![0; n * 3],
            heights: vec![0; n],
            water: vec![0; n],
            ground: vec![0; n],
            top: vec![0; n],
            biome: vec![0; n],
        }
    }

    /// Copies a drawn chunk into the picture.
    pub fn put(&mut self, cx: i32, cz: i32, p: &Painted) {
        let w = self.width;
        let (gx, gz) = ((cx - self.min_x) as usize * 16, (cz - self.min_z) as usize * 16);
        for row in 0..16 {
            let dst = ((gz + row) * w + gx) * 4;
            self.rgba[dst..dst + 64].copy_from_slice(&p.px[row * 64..row * 64 + 64]);
            let b = (gz + row) * w + gx;
            let r = row * 16..row * 16 + 16;
            self.blocks[b..b + 16].copy_from_slice(&p.base[r.clone()]);
            self.over[b..b + 16].copy_from_slice(&p.over[r.clone()]);
            self.albedo[b * 3..(b + 16) * 3].copy_from_slice(&p.albedo[row * 48..row * 48 + 48]);
            self.heights[b..b + 16].copy_from_slice(&p.y[r.clone()]);
            self.water[b..b + 16].copy_from_slice(&p.water[r.clone()]);
            self.ground[b..b + 16].copy_from_slice(&p.ground[r.clone()]);
            self.top[b..b + 16].copy_from_slice(&p.top[r.clone()]);
            self.biome[b..b + 16].copy_from_slice(&p.biome[r]);
        }
    }
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

/// One rendered chunk: 16 × 16 RGBA pixels (transparent where nothing was drawn) and the block
/// each column shows.
pub type Tile = (i32, i32, Painted);

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
    render_map_with(path, edition, dimension, max_y, i32::MIN, &MapProgress::default())
}

/// Draws the chunks in `all` (chunk x, chunk z, what is needed to read it) on every core.
/// `draw` returns a chunk's 16 × 16 RGBA pixels, or `None` when it cannot be read.
fn draw_chunks<T: Sync>(
    all: &[(i32, i32, T)],
    dimension: &str,
    progress: &MapProgress,
    draw: impl Fn(&T) -> Option<Painted> + Sync,
) -> Result<MapImage, String> {
    if all.is_empty() {
        return Err(format!("no chunks in {dimension}"));
    }
    let (mut xs, mut zs): (Vec<i32>, Vec<i32>) = all.iter().map(|c| (c.0, c.1)).unzip();
    xs.sort_unstable();
    zs.sort_unstable();
    // Center on the median chunk and cap the extent.
    let (mx, mz) = (xs[xs.len() / 2], zs[zs.len() / 2]);
    let half = MAX_CHUNKS_ACROSS / 2;
    let min_x = xs[0].max(mx - half);
    let max_x = xs[xs.len() - 1].min(mx + half - 1);
    let min_z = zs[0].max(mz - half);
    let max_z = zs[zs.len() - 1].min(mz + half - 1);
    let mut wanted: Vec<&(i32, i32, T)> = all
        .iter()
        .filter(|c| c.0 >= min_x && c.0 <= max_x && c.1 >= min_z && c.1 <= max_z)
        .collect();
    // From the middle outward, so what the user looks at first appears first.
    wanted.sort_by_key(|c| (c.0 - mx).pow(2) as i64 + (c.1 - mz).pow(2) as i64);
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
        let tile = draw(&c.2).map(|p| (c.0, c.1, p));
        if let Some(t) = &tile {
            progress.fresh.lock().expect("map tiles").push(t.clone());
        }
        progress.done.fetch_add(1, Ordering::Relaxed);
        tile.into_iter().collect()
    });
    if progress.is_cancelled() {
        return Err("cancelled".into());
    }
    let mut img = MapImage::blank(min_x, min_z, w, h);
    for (cx, cz, p) in &tiles {
        img.put(*cx, *cz, p);
    }
    // Lit again over the whole picture, so the slopes carry across chunk borders without seams.
    let MapImage { rgba, blocks, albedo, heights, water, .. } = &mut img;
    for z in 0..h {
        for x in 0..w {
            let i = z * w + x;
            if blocks[i] == 0 {
                continue;
            }
            let n = (z > 0 && blocks[i - w] != 0).then(|| heights[i - w] as i32);
            let we = (x > 0 && blocks[i - 1] != 0).then(|| heights[i - 1] as i32);
            let rgb = lit([albedo[i * 3], albedo[i * 3 + 1], albedo[i * 3 + 2]], heights[i] as i32, water[i] as i32, n, we, 1.0);
            rgba[i * 4..i * 4 + 3].copy_from_slice(&rgb);
        }
    }
    Ok(img)
}

/// [`render_map`] on all cores, reporting through `progress` and stopping when it is cancelled.
/// The Slicer keeps only the blocks from `min_y` up to `max_y`.
pub fn render_map_with(
    path: &Path,
    edition: Edition,
    dimension: &str,
    max_y: Option<i32>,
    min_y: i32,
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
                let mut cols = [None; 256];
                for z in 0..16 {
                    for x in 0..16 {
                        cols[z * 16 + x] = make_column(|visit| java_visit(&chunk, x, z, max_y, min_y, visit));
                    }
                }
                Some(paint(&cols, |x, z, y| java_biome(&chunk, x, z, y)))
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
            let mut data3d: std::collections::HashMap<(i32, i32), &Vec<u8>> = Default::default();
            for (k, v) in &world.db.entries {
                if let Key::Chunk(c) = Key::decode(k) {
                    if c.tag == ChunkTag::SubChunkPrefix && c.dimension.id() == dim_id {
                        by_chunk
                            .entry((c.x, c.z))
                            .or_default()
                            .push((c.subchunk.unwrap_or(0), v));
                    } else if c.tag == ChunkTag::Data3D && c.dimension.id() == dim_id {
                        data3d.insert((c.x, c.z), v);
                    }
                }
            }
            let all: Vec<_> = by_chunk
                .into_iter()
                .map(|((cx, cz), mut subs)| {
                    subs.sort_by_key(|(sy, _)| std::cmp::Reverse(*sy));
                    (cx, cz, (subs, data3d.get(&(cx, cz)).copied()))
                })
                .collect();
            // The overworld's biome storages start at y = -64; the other dimensions' at 0.
            let biome_floor = if dim_id == 0 { -64 } else { 0 };
            draw_chunks(&all, dimension, progress, |(subs, d3)| {
                let d3 = d3.and_then(|raw| mctk_bedrock::Data3D::parse(raw).ok());
                // Each subchunk's blocks with their palette under the Java names (Bedrock spells
                // many differently: `deadbush`, `tallgrass`...).
                let parsed: Vec<(i32, Vec<String>, Vec<u16>)> = subs
                    .iter()
                    .filter_map(|(sy, raw)| {
                        let SubChunkBody::Palettized(storages) = SubChunk::parse(raw).ok()?.body else { return None };
                        let s = storages.into_iter().next()?;
                        let names = s.palette.iter().map(|p| bedrock_java_state(p).name).collect();
                        Some((*sy as i32, names, s.indices))
                    })
                    .collect();
                let mut cols = [None; 256];
                for z in 0..16 {
                    for x in 0..16 {
                        cols[z * 16 + x] = make_column(|visit| {
                            'column: for (sy, names, indices) in &parsed {
                                for y in (0..16).rev() {
                                    let by = sy * 16 + y as i32;
                                    if by > max_y || by < min_y {
                                        continue;
                                    }
                                    let k = indices.get(mctk_bedrock::block_index(x, y, z)).copied().unwrap_or(0) as usize;
                                    let name = names.get(k).map_or("minecraft:air", String::as_str);
                                    if !matches!(name, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air") && !visit(name, by) {
                                        break 'column;
                                    }
                                }
                            }
                        });
                    }
                }
                Some(paint(&cols, |x, z, y| d3.as_ref().map_or(0, |d| bedrock_biome(d, biome_floor, x, z, y))))
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
                // One legacy biome number per column, z-major.
                let biomes: Option<Vec<u8>> = match chunk.to_nbt().tag.get("Level")?.get("Biomes") {
                    Some(Tag::ByteArray(b)) => Some(b.iter().map(|v| *v as u8).collect()),
                    _ => None,
                };
                let data = match chunk.to_nbt().tag.get("Level")?.get("Data") {
                    Some(Tag::ByteArray(d)) => d.clone(),
                    _ => Vec::new(),
                };
                let mut named: std::collections::HashMap<(u8, u8), String> = Default::default();
                let top_y = chunk.height.min(max_y.max(-1).saturating_add(1) as usize);
                let mut cols = [None; 256];
                for z in 0..16 {
                    for x in 0..16 {
                        cols[z * 16 + x] = make_column(|visit| {
                            for y in (0..top_y).rev().take_while(|y| *y as i32 >= min_y) {
                                let i = console_block_index(x, y, z);
                                let id = blocks.get(i).map_or(0, |b| *b as u8);
                                if id == 0 {
                                    continue;
                                }
                                let d = data.get(i / 2).map_or(0, |b| if i % 2 == 0 { *b as u8 & 15 } else { *b as u8 >> 4 });
                                let name = named.entry((id, d)).or_insert_with(|| java_map_name(&legacy_java_state(id as u16, d), None)).clone();
                                if !visit(&name, y as i32) {
                                    break;
                                }
                            }
                        });
                    }
                }
                Some(paint(&cols, |x, z, _| {
                    biomes.as_ref().and_then(|b| b.get(z * 16 + x)).and_then(|id| numeric_biome(*id as i32)).map_or(0, intern_biome)
                }))
            })
        }
    }
}

/// The Java block state for a Bedrock palette entry (`{name, states}`, or `{name, val}` from
/// older worlds), as the converter maps it; a block with no Java match keeps its Bedrock name.
pub(crate) fn bedrock_java_state(entry: &mctk_nbt::NamedTag) -> mctk_java::BlockState {
    let Some(b) = mctk_mapping::BedrockBlock::from_nbt(entry) else { return mctk_java::BlockState::air() };
    match mctk_mapping::mappings().bedrock_to_java(&b.name, &b.states) {
        Some(j) => j.clone(),
        // A Bedrock-only block (item frames are blocks there): its states kept as properties,
        // numbers marked with their type, so it is written back as it was.
        None => {
            let mut s = mctk_java::BlockState::new(&b.name);
            for (k, v) in &b.states {
                let text = match v {
                    mctk_nbt::Tag::Byte(n) => format!("b:{n}"),
                    mctk_nbt::Tag::Short(n) => format!("s:{n}"),
                    mctk_nbt::Tag::Int(n) => format!("i:{n}"),
                    mctk_nbt::Tag::String(t) => t.clone(),
                    _ => continue,
                };
                s.properties.insert(k.clone(), text);
            }
            s
        }
    }
}

/// The Bedrock palette entry for a Java block state; a Bedrock-only block (see
/// [`bedrock_java_state`]) as it was. `None` where Bedrock has no such block.
pub(crate) fn java_bedrock_state(state: &mctk_java::BlockState) -> Option<mctk_nbt::NamedTag> {
    let m = mctk_mapping::mappings();
    if let Some(b) = m.java_to_bedrock(state) {
        return Some(b.to_nbt());
    }
    if m.is_known_java(&mctk_java::BlockState::new(state.name.clone())) {
        return None;
    }
    let mut states = mctk_nbt::Compound::new();
    for (k, v) in &state.properties {
        let tag = match v.split_once(':') {
            Some(("b", n)) => n.parse().ok().map(mctk_nbt::Tag::Byte),
            Some(("s", n)) => n.parse().ok().map(mctk_nbt::Tag::Short),
            Some(("i", n)) => n.parse().ok().map(mctk_nbt::Tag::Int),
            _ => Some(mctk_nbt::Tag::String(v.clone())),
        };
        if let Some(t) = tag {
            states.insert(k.clone(), t);
        }
    }
    Some(mctk_mapping::BedrockBlock { name: state.name.clone(), states }.to_nbt())
}

/// The Java block state for a numeric id and data value (console worlds and Java chunks from
/// before 1.13), from the game's own upgrade tables; the rough name table when it has no entry.
pub(crate) fn legacy_java_state(id: u16, data: u8) -> mctk_java::BlockState {
    match mctk_mapping::legacy::legacy().block(id, data) {
        Some(s) => s.clone(),
        None => mctk_java::BlockState::new(legacy_name(id)),
    }
}

/// Whether a block name is water or grows in it, so the map shows water over it.
pub(crate) fn is_water(name: &str) -> bool {
    let n = name.trim_start_matches("minecraft:");
    n == "water" || n == "flowing_water" || n == "bubble_column" || n.contains("kelp") || n.contains("seagrass")
}

/// The map's name for a Java block: its id, with the states the map draws by ("oak_log#x" for a
/// log lying along x, "water#3" for flowing liquid; falling liquid is 8 and up) and, for grass
/// and leaves, the biome whose color they take ("grass_block@plains").
pub(crate) fn java_map_name(b: &mctk_java::BlockState, biome: Option<&str>) -> String {
    // Logs and pillars lying on their side look different.
    if let Some(a) = b.properties.get("axis").map(String::as_str).filter(|a| *a != "y") {
        return format!("{}#{a}", b.name);
    }
    if matches!(b.name.as_str(), "minecraft:water" | "minecraft:lava") {
        if let Some(level) = b.properties.get("level").filter(|l| *l != "0") {
            return format!("{}#{level}", b.name);
        }
    }
    let n = b.name.trim_start_matches("minecraft:");
    let tinted = matches!(n, "grass_block" | "short_grass" | "tall_grass" | "fern" | "large_fern" | "vine" | "sugar_cane")
        || matches!(n, "oak_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves" | "mangrove_leaves");
    match biome.filter(|_| tinted) {
        Some(bm) => format!("{n}@{}", bm.trim_start_matches("minecraft:")),
        None => b.name.clone(),
    }
}

/// The biome of a Java chunk at a column and height (1.18 and later keep one per 4 x 4 x 4 cell).
fn java_biome(chunk: &Chunk, x: usize, z: usize, y: i32) -> u16 {
    let sy = y.div_euclid(16);
    let Some(bm) = chunk.sections.iter().find(|s| s.y as i32 == sy).and_then(|s| s.biomes.as_ref()) else {
        return 0;
    };
    let ly = y.rem_euclid(16) as usize;
    intern_biome(bm.get((ly / 4) * 16 + (z / 4) * 4 + x / 4))
}

/// The biome of a Bedrock chunk at a column and height, from its `Data3D` record.
fn bedrock_biome(d: &mctk_bedrock::Data3D, floor: i32, x: usize, z: usize, y: i32) -> u16 {
    bedrock_biome_name(d, floor, x, z, y).map_or(0, intern_biome)
}

/// The name of the biome of a Bedrock chunk at a column and height (see [`bedrock_biome`]).
pub(crate) fn bedrock_biome_name(d: &mctk_bedrock::Data3D, floor: i32, x: usize, z: usize, y: i32) -> Option<&'static str> {
    use mctk_bedrock::BiomeStorage;
    if d.biomes.is_empty() {
        return None;
    }
    let section = ((y - floor).div_euclid(16)).clamp(0, d.biomes.len() as i32 - 1) as usize;
    // A "same as below" storage repeats the nearest one beneath it.
    let found = (0..=section).rev().find_map(|i| match &d.biomes[i] {
        BiomeStorage::Palettized { palette, indices, .. } => Some((palette, indices)),
        BiomeStorage::CopyPrevious => None,
    });
    let (palette, indices) = found?;
    let i = mctk_bedrock::block_index(x, y.rem_euclid(16) as usize, z);
    // Under its Java name (Bedrock's `roofed_forest` is Java's `dark_forest`).
    let id = *indices.get(i).and_then(|k| palette.get(*k as usize))?;
    mctk_mapping::mappings().bedrock_biome_to_java(id).or_else(|| numeric_biome(id))
}

/// Reports a Java chunk column's blocks from the top down (see `make_column`).
fn java_visit(chunk: &Chunk, x: usize, z: usize, max_y: i32, min_y: i32, visit: &mut dyn FnMut(&str, i32) -> bool) {
    for s in chunk
        .sections
        .iter()
        .rev()
        .filter(|s| s.y as i32 * 16 <= max_y && s.y as i32 * 16 + 15 >= min_y)
    {
        match &s.blocks {
            Some(BlockStorage::Palette(p)) => {
                if p.palette.iter().all(|b| b.is_air()) {
                    continue;
                }
                for y in (0..16)
                    .rev()
                    .filter(|y| (min_y..=max_y).contains(&(s.y as i32 * 16 + *y as i32)))
                {
                    let b = p.get(mctk_java::block_index(x, y, z));
                    if b.is_air() {
                        continue;
                    }
                    let biome = s.biomes.as_ref().map(|bm| bm.get((y / 4) * 16 + (z / 4) * 4 + x / 4).as_str());
                    let name = java_map_name(b, biome);
                    if !visit(&name, s.y as i32 * 16 + y as i32) {
                        return;
                    }
                }
            }
            Some(BlockStorage::Legacy { ids, data }) => {
                for y in (0..16)
                    .rev()
                    .filter(|y| (min_y..=max_y).contains(&(s.y as i32 * 16 + *y as i32)))
                {
                    let i = mctk_java::block_index(x, y, z);
                    let id = ids[i];
                    if id == 0 {
                        continue;
                    }
                    let name = java_map_name(&legacy_java_state(id, data.get(i).copied().unwrap_or(0)), None);
                    if !visit(&name, s.y as i32 * 16 + y as i32) {
                        return;
                    }
                }
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod edition_names {
    use super::*;
    use mctk_nbt::{Compound, NamedTag, Tag};

    fn entry(name: &str, states: &[(&str, Tag)]) -> NamedTag {
        let mut s = Compound::new();
        for (k, v) in states {
            s.insert((*k).into(), v.clone());
        }
        let mut c = Compound::new();
        c.insert("name".into(), Tag::String(name.into()));
        c.insert("states".into(), Tag::Compound(s));
        NamedTag { name: String::new(), tag: Tag::Compound(c) }
    }

    #[test]
    fn bedrock_and_legacy_blocks_get_java_names() {
        assert_eq!(bedrock_java_state(&entry("minecraft:deadbush", &[])).name, "minecraft:dead_bush");
        let log = bedrock_java_state(&entry("minecraft:oak_log", &[("pillar_axis", Tag::String("x".into()))]));
        assert_eq!((log.name.as_str(), log.properties.get("axis").map(String::as_str)), ("minecraft:oak_log", Some("x")));
        // Console / pre-1.13 numeric ids keep their data value.
        assert_eq!(legacy_java_state(32, 0).name, "minecraft:dead_bush");
        assert_eq!(legacy_java_state(31, 1).name, "minecraft:short_grass");
        assert_eq!(legacy_java_state(35, 14).name, "minecraft:red_wool");
        let stairs = legacy_java_state(53, 6);
        assert_eq!(stairs.properties.get("half").map(String::as_str), Some("top"));
    }
}
