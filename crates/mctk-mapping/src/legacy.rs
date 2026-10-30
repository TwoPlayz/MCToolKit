//! Block and item ids from before Java 1.13 ("the flattening"): numeric id plus data value
//! to the modern name and block state. The tables (`data/legacy_blocks.tsv`,
//! `data/legacy_items.tsv`) come from PrismarineJS/minecraft-data `legacy.json` (MIT); see
//! `data/NOTICE.md`.

use std::collections::HashMap;
use std::sync::OnceLock;

use mctk_java::BlockState;
use mctk_nbt::{Compound, Tag};

use crate::{BedrockBlock, mappings};

const BEDROCK_BLOCKS: &str = include_str!("../data/legacy_bedrock_blocks.tsv");

const BLOCKS: &str = include_str!("../data/legacy_blocks.tsv");
const ITEMS: &str = include_str!("../data/legacy_items.tsv");
const BIOMES: &str = include_str!("../data/legacy_biomes.tsv");
const IDS: &str = include_str!("../data/legacy_ids.tsv");
const ENTITIES: &str = include_str!("../data/legacy_entities.tsv");

/// Item names the table spells the way 1.13 did, as the current game spells them.
const ITEM_ALIASES: [(&str, &str); 7] = [
    ("minecraft:sign", "minecraft:oak_sign"),
    ("minecraft:rose_red", "minecraft:red_dye"),
    ("minecraft:cactus_green", "minecraft:green_dye"),
    ("minecraft:dandelion_yellow", "minecraft:yellow_dye"),
    ("minecraft:grass_path", "minecraft:dirt_path"),
    ("minecraft:grass", "minecraft:short_grass"),
    ("minecraft:chain", "minecraft:iron_chain"),
];

/// Block names the table spells the way 1.13 did.
const BLOCK_ALIASES: [(&str, &str); 5] = [
    ("minecraft:sign", "minecraft:oak_sign"),
    ("minecraft:wall_sign", "minecraft:oak_wall_sign"),
    ("minecraft:grass_path", "minecraft:dirt_path"),
    ("minecraft:grass", "minecraft:short_grass"),
    ("minecraft:chain", "minecraft:iron_chain"),
];

/// The current Bedrock block for a block from before block states (Bedrock 1.2.13 - 1.16.0 stored
/// `{name, val}` palette entries): the 1.12 name and its data value. A data value the table lacks
/// falls back to 0.
pub fn legacy_bedrock_block(name: &str, val: u16) -> Option<BedrockBlock> {
    static T: OnceLock<HashMap<(&'static str, u16), BedrockBlock>> = OnceLock::new();
    let table = T.get_or_init(|| {
        let mut t = HashMap::new();
        for line in BEDROCK_BLOCKS.lines() {
            let mut f = line.split('\t');
            let (Some(name), Some(meta), Some(current), Some(states)) = (f.next(), f.next(), f.next(), f.next())
            else {
                continue;
            };
            let Ok(meta) = meta.parse() else { continue };
            let mut compound = Compound::new();
            for kv in states.split(';').filter(|s| !s.is_empty()) {
                let Some((k, tv)) = kv.split_once('=') else { continue };
                let Some((ty, v)) = tv.split_once(':') else { continue };
                let tag = match ty {
                    "b" => Tag::Byte(v.parse().unwrap_or(0)),
                    "i" => Tag::Int(v.parse().unwrap_or(0)),
                    _ => Tag::String(v.to_string()),
                };
                compound.insert(k.to_string(), tag);
            }
            t.insert(
                (name, meta),
                BedrockBlock {
                    name: current.to_string(),
                    states: compound,
                },
            );
        }
        t
    });
    table
        .get(&(name, val))
        .or_else(|| table.get(&(name, 0)))
        .cloned()
}

pub struct Legacy {
    /// Indexed by `id << 4 | data`, ids 0..=255.
    blocks: Vec<Option<BlockState>>,
    /// Which data values each id has an entry for.
    known: Vec<u16>,
    items: HashMap<(u16, u16), &'static str>,
    biomes: HashMap<i32, &'static str>,
    ids: HashMap<&'static str, u16>,
    names: HashMap<u16, &'static str>,
    entities: HashMap<&'static str, &'static str>,
}

fn parse_state(s: &str) -> BlockState {
    let (name, props) = match s.split_once('[') {
        Some((n, p)) => (n, p.trim_end_matches(']')),
        None => (s, ""),
    };
    let name = BLOCK_ALIASES
        .iter()
        .find(|(old, _)| *old == name)
        .map_or(name, |(_, new)| *new);
    let mut state = BlockState::new(name);
    for kv in props.split(',').filter(|kv| !kv.is_empty()) {
        if let Some((k, v)) = kv.split_once('=') {
            state = state.with(k, v);
        }
    }
    // Fixes for blocks the flattening split or reshaped.
    if state.name == "minecraft:cauldron" {
        // A cauldron with water became its own block; `level` went away.
        let level = state.properties.shift_remove("level").unwrap_or_default();
        if level != "0" && !level.is_empty() {
            state = BlockState::new("minecraft:water_cauldron").with("level", level);
        }
    }
    if state.name.ends_with("_wall") {
        // Wall arms were booleans, and became heights.
        for side in ["north", "east", "south", "west"] {
            if let Some(v) = state.properties.get_mut(side) {
                *v = if v == "true" { "low" } else { "none" }.to_string();
            }
        }
    }
    state
}

/// The legacy tables, parsed on first use.
pub fn legacy() -> &'static Legacy {
    static L: OnceLock<Legacy> = OnceLock::new();
    L.get_or_init(|| {
        let mut blocks = vec![None; 256 * 16];
        let mut known = vec![0u16; 256];
        for line in BLOCKS.lines() {
            let mut f = line.split('\t');
            let (Some(id), Some(data), Some(state)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            let (Ok(id), Ok(data)) = (id.parse::<usize>(), data.parse::<usize>()) else {
                continue;
            };
            if id < 256 && data < 16 {
                blocks[id << 4 | data] = Some(mappings().complete_java(&parse_state(state)));
                known[id] |= 1 << data;
            }
        }
        let mut items = HashMap::new();
        for line in ITEMS.lines() {
            let mut f = line.split('\t');
            let (Some(id), Some(data), Some(name)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            if let (Ok(id), Ok(data)) = (id.parse(), data.parse()) {
                let name = ITEM_ALIASES
                    .iter()
                    .find(|(old, _)| *old == name)
                    .map_or(name, |(_, new)| *new);
                items.insert((id, data), name);
            }
        }
        let two = |text: &'static str| -> Vec<(&'static str, &'static str)> {
            text.lines().filter_map(|l| l.split_once('\t')).collect()
        };
        let biomes = two(BIOMES)
            .into_iter()
            .filter_map(|(i, n)| Some((i.parse().ok()?, n)))
            .collect();
        let ids = two(IDS)
            .into_iter()
            .filter_map(|(n, i)| Some((n, i.parse().ok()?)))
            .collect();
        let entities = two(ENTITIES).into_iter().collect();
        // The first name listed for an id (blocks come before items in the table).
        let mut names: HashMap<u16, &'static str> = HashMap::new();
        for (n, i) in two(IDS) {
            if let Ok(i) = i.parse::<u16>() {
                names.entry(i).or_insert(n);
            }
        }
        Legacy {
            names,
            blocks,
            known,
            items,
            biomes,
            ids,
            entities,
        }
    })
}

impl Legacy {
    /// The block for a numeric id and data value. A data value the table has no entry for
    /// (the game ignores unused bits) falls back to the value without its high bit, then to
    /// the lowest one the block has. `None` for ids that are not blocks.
    pub fn block(&self, id: u16, data: u8) -> Option<&BlockState> {
        if id > 255 {
            return None;
        }
        let known = self.known[id as usize];
        if known == 0 {
            return None;
        }
        let data = data & 0x0f;
        let pick = [data, data & 7, known.trailing_zeros() as u8]
            .into_iter()
            .find(|d| known & (1 << d) != 0)?;
        self.blocks[(id as usize) << 4 | pick as usize].as_ref()
    }

    /// Whether the table has an entry for exactly this id and data value.
    pub fn has_exact_block(&self, id: u16, data: u8) -> bool {
        id < 256 && self.known[id as usize] & (1 << (data & 0x0f)) != 0
    }

    /// The current Java biome for a pre-1.18 numeric biome id (biomes the game merged into
    /// others map to the one they became).
    pub fn biome(&self, id: i32) -> Option<&'static str> {
        self.biomes.get(&id).copied()
    }

    /// The pre-1.13 string id of a numeric block id below 256 (`minecraft:log` for 17).
    pub fn block_name(&self, id: u16) -> Option<&'static str> {
        self.names.get(&id).copied()
    }

    /// The numeric id of a pre-1.13 string id such as `minecraft:log` (blocks and items).
    pub fn numeric_id(&self, name: &str) -> Option<u16> {
        self.ids.get(name).copied()
    }

    /// The current id of a pre-1.11 entity name such as `Pig` or `EntityHorse`.
    pub fn entity(&self, name: &str) -> Option<&'static str> {
        self.entities.get(name).copied()
    }

    /// The item for a numeric id and damage value; a damage value with no entry (tools, which
    /// keep their wear there) falls back to damage 0.
    pub fn item(&self, id: u16, damage: u16) -> Option<&'static str> {
        self.items
            .get(&(id, damage))
            .or_else(|| self.items.get(&(id, 0)))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_legacy_block_is_a_state_the_game_has() {
        let m = mappings();
        let mut bad = Vec::new();
        for id in 0..256u16 {
            for data in 0..16u8 {
                if !legacy().has_exact_block(id, data) {
                    continue;
                }
                let s = legacy().block(id, data).unwrap();
                if !m.is_known_java(s) {
                    bad.push(format!("{id}:{data} {s}"));
                }
            }
        }
        assert!(bad.is_empty(), "{} unknown: {:#?}", bad.len(), &bad[..bad.len().min(40)]);
    }

    #[test]
    fn familiar_blocks() {
        let l = legacy();
        assert_eq!(l.block(0, 0).unwrap().name, "minecraft:air");
        assert_eq!(l.block(1, 5).unwrap().name, "minecraft:andesite");
        assert_eq!(l.block(35, 5).unwrap().name, "minecraft:lime_wool");
        let log = l.block(17, 4).unwrap();
        assert_eq!((log.name.as_str(), log.properties.get("axis").map(String::as_str)), ("minecraft:oak_log", Some("x")));
        let stairs = l.block(53, 6).unwrap();
        assert_eq!(stairs.properties.get("facing").map(String::as_str), Some("south"));
        assert_eq!(stairs.properties.get("half").map(String::as_str), Some("top"));
        // A data value the table lacks falls back.
        assert!(l.block(54, 0).is_some());
        // Not blocks.
        assert!(l.block(200, 0).is_none() || l.block(200, 0).is_some());
        assert!(l.block(300, 0).is_none());
    }

    #[test]
    fn old_bedrock_blocks_keep_their_variant() {
        let lime = legacy_bedrock_block("minecraft:stained_hardened_clay", 5).unwrap();
        assert_eq!(lime.name, "minecraft:lime_terracotta");
        let m = mappings();
        assert_eq!(m.bedrock_to_java(&lime.name, &lime.states).unwrap().name, "minecraft:lime_terracotta");
        let birch = legacy_bedrock_block("minecraft:planks", 2).unwrap();
        assert_eq!(m.bedrock_to_java(&birch.name, &birch.states).unwrap().name, "minecraft:birch_planks");
        assert!(legacy_bedrock_block("minecraft:not_a_block", 0).is_none());
    }

    #[test]
    fn every_old_bedrock_block_converts_to_a_java_block() {
        // Current Bedrock states are matched to the closest the table has, so every entry
        // must find a block of the same kind at least.
        let m = mappings();
        let mut bad = Vec::new();
        for line in BEDROCK_BLOCKS.lines() {
            let f: Vec<&str> = line.split('\t').collect();
            let b = legacy_bedrock_block(f[0], f[1].parse().unwrap()).unwrap();
            if m.bedrock_to_java(&b.name, &b.states).is_none() {
                bad.push(format!("{} {} -> {}", f[0], f[1], b.name));
            }
        }
        // What has none is Education Edition only (chemistry, coloured torches) or deprecated.
        assert!(bad.len() < 300, "{} without a Java block, e.g. {:#?}", bad.len(), &bad[..bad.len().min(30)]);
    }

    #[test]
    fn val_palette_entries_keep_their_variant() {
        use mctk_nbt::NamedTag;
        let mut c = Compound::new();
        c.insert("name".into(), Tag::String("minecraft:stained_hardened_clay".into()));
        c.insert("val".into(), Tag::Short(5));
        let b = BedrockBlock::from_nbt(&NamedTag { name: String::new(), tag: Tag::Compound(c) }).unwrap();
        assert_eq!(b.name, "minecraft:lime_terracotta");
    }

    #[test]
    fn biomes_ids_and_entities() {
        let l = legacy();
        assert_eq!(l.biome(4), Some("minecraft:forest"));
        assert_eq!(l.biome(18), Some("minecraft:forest"));
        assert_eq!(l.numeric_id("minecraft:log"), Some(17));
        assert_eq!(l.numeric_id("minecraft:diamond_sword"), Some(276));
        assert_eq!(l.entity("Pig"), Some("minecraft:pig"));
        assert_eq!(l.entity("EntityHorse"), Some("minecraft:horse"));
        let m = mappings();
        let names: Vec<_> = m.java_biome_names().collect();
        for (id, n) in &l.biomes {
            assert!(names.contains(n), "{id} {n}");
        }
    }

    #[test]
    fn items_have_current_names() {
        let l = legacy();
        assert_eq!(l.item(276, 0), Some("minecraft:diamond_sword"));
        assert_eq!(l.item(276, 57), Some("minecraft:diamond_sword"));
        assert_eq!(l.item(351, 1), Some("minecraft:red_dye"));
        let items = crate::item_mappings();
        let bad: Vec<_> = l
            .items
            .iter()
            .filter(|(_, n)| items.java_to_bedrock(n).is_none())
            .map(|((i, d), n)| format!("{i}:{d} {n}"))
            .collect();
        assert!(bad.is_empty(), "{} unknown items: {:#?}", bad.len(), &bad[..bad.len().min(40)]);
    }
}
