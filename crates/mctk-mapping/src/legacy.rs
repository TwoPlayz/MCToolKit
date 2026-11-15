//! Block and item ids from before Java 1.13 ("the flattening"): numeric id plus data value
//! to the modern name and block state. The tables come from the games themselves (see
//! `mctk-registry/data/legacy`): a real Java server upgraded a world holding every id and data
//! value, a real Bedrock server did the same, and the old string ids are read from the Java
//! game's own data fixers.

use std::collections::HashMap;
use std::sync::OnceLock;

use mctk_java::BlockState;
use mctk_nbt::{Compound, Tag};
use mctk_registry::builtin;

use crate::{BedrockBlock, mappings};

fn typed(v: &str) -> Tag {
    match v.split_once(':') {
        Some(("b", n)) => Tag::Byte(n.parse().unwrap_or(0)),
        Some(("i", n)) => Tag::Int(n.parse().unwrap_or(0)),
        Some((_, s)) => Tag::String(s.to_string()),
        None => Tag::String(v.to_string()),
    }
}

/// What the Bedrock server made of a numeric block id and data value of a world from before block
/// states (ids and data values are the same as Java's up to id 197).
pub fn legacy_bedrock_numeric(id: u16, data: u8) -> Option<BedrockBlock> {
    static T: OnceLock<HashMap<(u16, u8), BedrockBlock>> = OnceLock::new();
    let table = T.get_or_init(|| {
        let mut t = HashMap::new();
        for line in builtin::bedrock_legacy_blocks_text().lines().filter(|l| !l.starts_with('#')) {
            let mut f = line.split('\t');
            let (Some(Ok(id)), Some(Ok(data)), Some(name)) = (f.next().map(str::parse::<u16>), f.next().map(str::parse::<u8>), f.next()) else {
                continue;
            };
            let mut compound = Compound::new();
            for kv in f.next().unwrap_or("").split(',').filter(|s| !s.is_empty()) {
                if let Some((k, v)) = kv.split_once('=') {
                    compound.insert(k.to_string(), typed(v));
                }
            }
            t.insert((id, data), BedrockBlock { name: name.to_string(), states: compound });
        }
        t
    });
    table.get(&(id, data)).or_else(|| table.get(&(id, 0))).cloned()
}

/// The current Bedrock block for a block from before block states (Bedrock 1.2.13 - 1.16.0 stored
/// `{name, val}` palette entries): the old name and its data value. A name Java also had goes
/// through its numeric id; one only Bedrock had is taken as the current block of that name.
pub fn legacy_bedrock_block(name: &str, val: u16) -> Option<BedrockBlock> {
    if let Some(id) = legacy().numeric_id(name).filter(|id| *id < 256) {
        if let Some(b) = legacy_bedrock_numeric(id, (val & 15) as u8) {
            return Some(b);
        }
    }
    let states = Compound::new();
    mappings().is_known_bedrock(name, &states).then(|| BedrockBlock { name: name.to_string(), states })
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
    /// The other way round: a block state to the numeric id and data value that makes it, and every
    /// numeric entry of a block name (for the nearest state when there is no exact one).
    state_ids: HashMap<BlockState, (u16, u8)>,
    by_name: HashMap<String, Vec<(BlockState, u16, u8)>>,
    item_ids: HashMap<&'static str, (u16, u16)>,
    entity_names: HashMap<&'static str, &'static str>,
}

fn parse_state(s: &str) -> BlockState {
    let (name, props) = match s.split_once('[') {
        Some((n, p)) => (n, p.trim_end_matches(']')),
        None => (s, ""),
    };
    let mut state = BlockState::new(name);
    for kv in props.split(',').filter(|kv| !kv.is_empty()) {
        if let Some((k, v)) = kv.split_once('=') {
            state = state.with(k, v);
        }
    }
    state
}

/// The legacy tables, parsed on first use.
pub fn legacy() -> &'static Legacy {
    static L: OnceLock<Legacy> = OnceLock::new();
    L.get_or_init(|| {
        let data_lines = |text: &'static str| text.lines().filter(|l| !l.starts_with('#'));
        let mut blocks = vec![None; 256 * 16];
        let mut known = vec![0u16; 256];
        for line in data_lines(builtin::legacy_blocks_text()) {
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
        // Block ids the game upgrades to a block that has no item (water, fire) are not items.
        let java_items: std::collections::HashSet<String> = builtin::java(builtin::java_versions()[0])
            .map(|r| r.items.into_iter().collect())
            .unwrap_or_default();
        let mut items = HashMap::new();
        for line in data_lines(builtin::legacy_items_text()) {
            let mut f = line.split('\t');
            let (Some(Ok(id)), Some(Ok(damage)), Some(name)) =
                (f.next().map(str::parse::<u16>), f.next().map(str::parse::<u16>), f.next())
            else {
                continue;
            };
            let name = name.trim_matches('"');
            if name != "-" && java_items.contains(name) {
                items.insert((id, damage), name);
            }
        }
        let two = |text: &'static str| -> Vec<(&'static str, &'static str)> {
            data_lines(text).filter_map(|l| l.split_once('\t')).collect()
        };
        let biomes = two(builtin::legacy_biomes_text())
            .into_iter()
            .filter_map(|(i, n)| Some((i.parse().ok()?, n.trim())))
            .collect();
        let ids: HashMap<&'static str, u16> = two(builtin::legacy_ids_text())
            .into_iter()
            .filter_map(|(n, i)| Some((n, i.trim().parse().ok()?)))
            .collect();
        let entities: HashMap<&'static str, &'static str> = two(builtin::legacy_entities_text()).into_iter().collect();
        // The name of a block id (blocks come first in the table, so the block's own name wins).
        let mut names: HashMap<u16, &'static str> = HashMap::new();
        let mut ordered: Vec<_> = ids.iter().map(|(n, i)| (*i, *n)).collect();
        ordered.sort();
        for (i, n) in ordered {
            names.entry(i).or_insert(n);
        }
        let mut state_ids = HashMap::new();
        let mut by_name: HashMap<String, Vec<(BlockState, u16, u8)>> = HashMap::new();
        for id in 0..256u16 {
            for data in 0..16u8 {
                if known[id as usize] & (1 << data) == 0 {
                    continue;
                }
                if let Some(state) = &blocks[(id as usize) << 4 | data as usize] {
                    state_ids.entry(state.clone()).or_insert((id, data));
                    by_name.entry(state.name.clone()).or_default().push((state.clone(), id, data));
                }
            }
        }
        let mut item_ids: HashMap<&'static str, (u16, u16)> = HashMap::new();
        for (&(id, damage), &name) in &items {
            let best = item_ids.entry(name).or_insert((id, damage));
            if (damage, id) < (best.1, best.0) {
                *best = (id, damage);
            }
        }
        let mut entity_names: HashMap<&'static str, &'static str> = HashMap::new();
        let mut old: Vec<_> = entities.iter().collect();
        old.sort();
        for (&oldname, &new) in old {
            entity_names.entry(new).or_insert(oldname);
        }
        Legacy {
            names,
            blocks,
            known,
            items,
            biomes,
            ids,
            entities,
            state_ids,
            by_name,
            item_ids,
            entity_names,
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

    /// The numeric id and data value that make a block state, or (when no entry is exactly it) the
    /// entry of the same block that agrees with it on most properties.
    pub fn numeric_state(&self, state: &BlockState) -> Option<(u16, u8)> {
        if let Some(found) = self.state_ids.get(state) {
            return Some(*found);
        }
        let candidates = self.by_name.get(&state.name)?;
        let score = |s: &BlockState| s.properties.iter().filter(|(k, v)| state.properties.get(*k) == Some(*v)).count();
        candidates.iter().max_by_key(|(s, id, data)| (score(s), std::cmp::Reverse((*id, *data)))).map(|(_, id, data)| (*id, *data))
    }

    /// The numeric id and damage value of a current item name (the lowest damage that makes it).
    pub fn numeric_item(&self, name: &str) -> Option<(u16, u16)> {
        self.item_ids.get(name).copied()
    }

    /// The pre-1.11 name of a current entity id (`Pig` for `minecraft:pig`).
    pub fn old_entity_name(&self, id: &str) -> Option<&'static str> {
        self.entity_names.get(id).copied()
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
