//! Chunks as Java 1.13 to 1.17 store them, for converting to those versions.
//!
//! Before 1.18 a chunk keeps everything under `Level`, is 256 blocks tall (sections 0 to 15),
//! stores biomes as numbers (one per column in 1.13 and 1.14, one per 4x4x4 cell from 1.15) and
//! has its entities inside it (they have their own files from 1.17). The layout was checked
//! against chunks the real 1.15.2, 1.16.5 and 1.17.1 servers saved.

use std::collections::HashMap;

use mctk_java::{BlockState, BlockStorage, Chunk};
use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};

use crate::java_items::Notes;

/// First `DataVersion` of the new chunk layout (1.18).
pub const FIRST_NEW_LAYOUT: i32 = 2844;
/// First version whose entities live in their own region files (1.17).
pub const FIRST_ENTITY_FILES: i32 = 2724;
/// First version with `WorldGenSettings` in `level.dat` (1.16).
pub const FIRST_WORLD_GEN_SETTINGS: i32 = 2566;
/// First version whose packed block indices never straddle two longs (1.16).
const FIRST_NO_SPANNING: i32 = 2527;
/// First version with one biome per 4x4x4 cell (1.15), and the first with `isLightOn` (1.14).
const FIRST_BIOME_CELLS: i32 = 2225;
const FIRST_LIGHT_FLAG: i32 = 1952;

/// Entities by the chunk they stand in (chunk x, chunk z), for versions that keep them there.
pub type EntityMap = HashMap<(i32, i32), Vec<Tag>>;

/// The 1.18 names of biomes that were called something else before, with the old name.
const OLD_NAMES: &[(&str, &str)] = &[
    ("windswept_hills", "mountains"),
    ("windswept_gravelly_hills", "gravelly_mountains"),
    ("windswept_forest", "wooded_mountains"),
    ("windswept_savanna", "shattered_savanna"),
    ("snowy_plains", "snowy_tundra"),
    ("sparse_jungle", "jungle_edge"),
    ("old_growth_birch_forest", "tall_birch_forest"),
    ("old_growth_pine_taiga", "giant_tree_taiga"),
    ("old_growth_spruce_taiga", "giant_spruce_taiga"),
    ("stony_shore", "stone_shore"),
    ("wooded_badlands", "wooded_badlands_plateau"),
    ("frozen_peaks", "snowy_mountains"),
    ("jagged_peaks", "snowy_mountains"),
    ("stony_peaks", "mountains"),
];

/// A biome's name as versions before 1.18 knew it.
pub fn old_biome_name(name: &str) -> String {
    let bare = name.strip_prefix("minecraft:").unwrap_or(name);
    match OLD_NAMES.iter().find(|(new, _)| *new == bare) {
        Some((_, old)) => format!("minecraft:{old}"),
        None => name.to_string(),
    }
}

/// The numbers biomes were saved as before 1.18.
const BIOME_IDS: &[(&str, i32)] = &[
    ("ocean", 0), ("plains", 1), ("desert", 2), ("mountains", 3), ("forest", 4), ("taiga", 5),
    ("swamp", 6), ("river", 7), ("nether_wastes", 8), ("the_end", 9), ("frozen_ocean", 10),
    ("frozen_river", 11), ("snowy_tundra", 12), ("snowy_mountains", 13), ("mushroom_fields", 14),
    ("mushroom_field_shore", 15), ("beach", 16), ("desert_hills", 17), ("wooded_hills", 18),
    ("taiga_hills", 19), ("mountain_edge", 20), ("jungle", 21), ("jungle_hills", 22),
    ("jungle_edge", 23), ("deep_ocean", 24), ("stone_shore", 25), ("snowy_beach", 26),
    ("birch_forest", 27), ("birch_forest_hills", 28), ("dark_forest", 29), ("snowy_taiga", 30),
    ("snowy_taiga_hills", 31), ("giant_tree_taiga", 32), ("giant_tree_taiga_hills", 33),
    ("wooded_mountains", 34), ("savanna", 35), ("savanna_plateau", 36), ("badlands", 37),
    ("wooded_badlands_plateau", 38), ("badlands_plateau", 39), ("small_end_islands", 40),
    ("end_midlands", 41), ("end_highlands", 42), ("end_barrens", 43), ("warm_ocean", 44),
    ("lukewarm_ocean", 45), ("cold_ocean", 46), ("deep_warm_ocean", 47),
    ("deep_lukewarm_ocean", 48), ("deep_cold_ocean", 49), ("deep_frozen_ocean", 50),
    ("the_void", 127), ("sunflower_plains", 129), ("desert_lakes", 130),
    ("gravelly_mountains", 131), ("flower_forest", 132), ("taiga_mountains", 133),
    ("swamp_hills", 134), ("ice_spikes", 140), ("modified_jungle", 149),
    ("modified_jungle_edge", 151), ("tall_birch_forest", 155), ("tall_birch_hills", 156),
    ("dark_forest_hills", 157), ("snowy_taiga_mountains", 158), ("giant_spruce_taiga", 160),
    ("giant_spruce_taiga_hills", 161), ("modified_gravelly_mountains", 162),
    ("shattered_savanna", 163), ("shattered_savanna_plateau", 164), ("eroded_badlands", 165),
    ("modified_wooded_badlands_plateau", 166), ("modified_badlands_plateau", 167),
    ("bamboo_jungle", 168), ("bamboo_jungle_hills", 169), ("soul_sand_valley", 170),
    ("crimson_forest", 171), ("warped_forest", 172), ("basalt_deltas", 173),
    ("dripstone_caves", 174), ("lush_caves", 175),
];

/// The number a biome is saved as (plains for one that has none).
pub fn biome_id(name: &str) -> i32 {
    let bare = name.strip_prefix("minecraft:").unwrap_or(name);
    BIOME_IDS.iter().find(|(n, _)| *n == bare).map_or(1, |(_, id)| *id)
}

fn is_air(s: &BlockState) -> bool {
    matches!(s.name.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
}

fn list(ty: TagType, tags: Vec<Tag>) -> Tag {
    Tag::List(List::from_tags(ty, tags).expect("uniform"))
}

/// The pre-1.18 chunk for a converted chunk: its blocks and biomes (`chunk`), the rest of the
/// converted 1.18-layout NBT (`top`), and the entities that belong inside it.
pub fn old_chunk(chunk: &Chunk, top: &Compound, data_version: i32, entities: Vec<Tag>, notes: &mut Notes) -> NamedTag {
    let spanning = data_version < FIRST_NO_SPANNING;
    let mut sections = Vec::new();
    let mut cells: Vec<[i32; 64]> = vec![[1; 64]; 16];
    let mut cut = false;
    for s in &chunk.sections {
        if !(0..16).contains(&s.y) {
            // The old world is 256 blocks tall.
            if matches!(&s.blocks, Some(BlockStorage::Palette(p)) if p.palette.iter().any(|b| !is_air(b))) {
                cut = true;
            }
            continue;
        }
        if let Some(b) = &s.biomes {
            for (i, cell) in cells[s.y as usize].iter_mut().enumerate() {
                *cell = biome_id(&b.palette[b.indices[i] as usize]);
            }
        }
        let Some(BlockStorage::Palette(p)) = &s.blocks else { continue };
        if p.palette.iter().all(is_air) {
            continue;
        }
        let mut data = p.encode(4, spanning);
        if data.is_empty() {
            data = vec![0; 256];
        }
        let palette = p.palette.iter().map(BlockState::to_named_compound).collect();
        let mut section = Compound::from([
            ("Y".to_string(), Tag::Byte(s.y)),
            ("Palette".to_string(), list(TagType::Compound, palette)),
            ("BlockStates".to_string(), Tag::LongArray(data)),
        ]);
        // Until 1.15 a section has to carry its light (dark here; the chunk is marked unlit, so the
        // game works it out).
        if data_version < FIRST_BIOME_CELLS {
            for key in ["BlockLight", "SkyLight"] {
                section.insert(key.into(), Tag::ByteArray(vec![0; 2048]));
            }
        }
        sections.push(Tag::Compound(section));
    }
    if cut {
        notes.add("blocks above 255 or below 0 do not exist before 1.18");
    }

    let mut level = Compound::new();
    level.insert("xPos".into(), Tag::Int(chunk.x));
    level.insert("zPos".into(), Tag::Int(chunk.z));
    level.insert("LastUpdate".into(), Tag::Long(top.get("LastUpdate").and_then(Tag::as_i64).unwrap_or(0)));
    level.insert("InhabitedTime".into(), Tag::Long(top.get("InhabitedTime").and_then(Tag::as_i64).unwrap_or(0)));
    level.insert(
        "Status".into(),
        Tag::String(if data_version < FIRST_LIGHT_FLAG { "postprocessed" } else { "full" }.into()),
    );
    if data_version >= FIRST_LIGHT_FLAG {
        level.insert("isLightOn".into(), Tag::Byte(0));
    } else {
        level.insert("LightPopulated".into(), Tag::Byte(0));
        level.insert("TerrainPopulated".into(), Tag::Byte(1));
    }
    if data_version >= FIRST_BIOME_CELLS {
        level.insert("Biomes".into(), Tag::IntArray(cells.iter().flatten().copied().collect()));
    } else {
        // One biome per column, taken from the cell at sea level.
        let column = |x: usize, z: usize| cells[4][(z / 4) * 4 + x / 4];
        let all = (0..256).map(|i| column(i % 16, i / 16));
        level.insert(
            "Biomes".into(),
            if data_version < FIRST_LIGHT_FLAG {
                Tag::ByteArray(all.map(|b| b as i8).collect())
            } else {
                Tag::IntArray(all.collect())
            },
        );
    }
    level.insert("Sections".into(), list(TagType::Compound, sections));

    let tile_entities: Vec<Tag> = match top.get("block_entities") {
        Some(Tag::List(l)) => l
            .iter()
            .filter(|t| t.as_compound().and_then(|c| c.get("y")).and_then(Tag::as_i64).is_some_and(|y| (0..256).contains(&y)))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    level.insert("TileEntities".into(), list(TagType::Compound, tile_entities));
    level.insert("Entities".into(), list(TagType::Compound, entities));
    level.insert("TileTicks".into(), Tag::List(List::new(TagType::Compound)));
    level.insert("LiquidTicks".into(), Tag::List(List::new(TagType::Compound)));
    level.insert(
        "Structures".into(),
        Tag::Compound(Compound::from([
            ("Starts".to_string(), Tag::Compound(Compound::new())),
            ("References".to_string(), Tag::Compound(Compound::new())),
        ])),
    );
    NamedTag {
        name: String::new(),
        tag: Tag::Compound(Compound::from([
            ("DataVersion".to_string(), Tag::Int(data_version)),
            ("Level".to_string(), Tag::Compound(level)),
        ])),
    }
}

/// `WorldGenSettings` for 1.16 and 1.17: every generator carries the seed.
pub fn world_gen_settings(seed: i64, generate_structures: bool, bonus_chest: bool) -> Compound {
    let generator = |settings: &str, source: Compound| {
        Tag::Compound(Compound::from([
            ("type".to_string(), Tag::String("minecraft:noise".into())),
            ("seed".to_string(), Tag::Long(seed)),
            ("settings".to_string(), Tag::String(format!("minecraft:{settings}"))),
            ("biome_source".to_string(), Tag::Compound(source)),
        ]))
    };
    let source = |kind: &str, extra: Vec<(&str, Tag)>| {
        let mut c = Compound::from([
            ("type".to_string(), Tag::String(format!("minecraft:{kind}"))),
            ("seed".to_string(), Tag::Long(seed)),
        ]);
        for (k, v) in extra {
            c.insert(k.to_string(), v);
        }
        c
    };
    let dim = |name: &str, generator: Tag| {
        (
            format!("minecraft:{name}"),
            Tag::Compound(Compound::from([
                ("type".to_string(), Tag::String(format!("minecraft:{name}"))),
                ("generator".to_string(), generator),
            ])),
        )
    };
    let dimensions = Compound::from([
        dim(
            "overworld",
            generator("overworld", source("vanilla_layered", vec![("large_biomes", Tag::Byte(0))])),
        ),
        dim(
            "the_nether",
            generator("nether", source("multi_noise", vec![("preset", Tag::String("minecraft:nether".into()))])),
        ),
        dim("the_end", generator("end", source("the_end", vec![]))),
    ]);
    Compound::from([
        ("seed".to_string(), Tag::Long(seed)),
        ("generate_features".to_string(), Tag::Byte(generate_structures as i8)),
        ("bonus_chest".to_string(), Tag::Byte(bonus_chest as i8)),
        ("dimensions".to_string(), Tag::Compound(dimensions)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn biomes_have_their_old_numbers() {
        assert_eq!(biome_id("minecraft:plains"), 1);
        assert_eq!(biome_id("minecraft:basalt_deltas"), 173);
        assert_eq!(biome_id("minecraft:nothing_like_it"), 1);
        assert_eq!(old_biome_name("minecraft:windswept_hills"), "minecraft:mountains");
        assert_eq!(old_biome_name("minecraft:plains"), "minecraft:plains");
    }
}
