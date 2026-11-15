//! What the Java game makes of legacy (1.8-era, numeric) blocks, items and biomes.
//!
//! The tables are not hand-written: `legacy_blocks` and `legacy_items` (examples) write a world
//! holding every id and data value, let a real Java server upgrade it and read back what the game
//! turned each one into. See `mctk-registry/data/legacy`.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use mctk_java::BlockState;
use mctk_registry::builtin;

/// A legacy item as the game upgrades it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemInfo {
    pub name: String,
    /// The damage value is the item's damage (tools, armor), kept as `tag.Damage`.
    pub keeps_damage: bool,
    /// Spawn eggs: the entity they spawn.
    pub entity: Option<String>,
    /// Filled maps: the damage value is the map number.
    pub map: bool,
}

pub struct LegacyTables {
    blocks: HashMap<(u8, u8), BlockState>,
    items: HashMap<(i16, i16), ItemInfo>,
    biomes: Vec<Option<String>>,
    /// The ids the console game itself defines.
    console_blocks: HashSet<u8>,
    /// The other way: a state (without what depends on neighbours) to the lowest id and data.
    by_state: HashMap<BlockState, (u8, u8)>,
    by_name: HashMap<String, Vec<(u8, u8)>>,
    item_by_name: HashMap<String, (i16, i16, bool)>,
    biome_by_name: HashMap<String, u8>,
}

/// The block states Java derives from a block's neighbours (or that the console has no notion of);
/// they say nothing about the legacy id and data, so they are left out when looking one up.
pub fn canonical(state: &BlockState) -> BlockState {
    let name = state.name.as_str();
    let mut s = state.clone();
    let drop = |s: &mut BlockState, keys: &[&str]| keys.iter().for_each(|k| {
        s.properties.shift_remove(*k);
    });
    drop(&mut s, &["waterlogged"]);
    let sides = ["north", "east", "south", "west", "up"];
    if name.ends_with("_stairs") {
        drop(&mut s, &["shape"]);
    } else if name.ends_with("_fence_gate") {
        drop(&mut s, &["in_wall"]);
    } else if name.ends_with("_fence") || name.ends_with("_pane") || name == "minecraft:iron_bars" || name.ends_with("_wall") {
        drop(&mut s, &sides);
    } else if name == "minecraft:redstone_wire" {
        drop(&mut s, &sides);
    } else if name == "minecraft:chest" || name == "minecraft:trapped_chest" {
        drop(&mut s, &["type"]);
    } else if name.ends_with("_leaves") {
        drop(&mut s, &["distance"]);
    } else if matches!(name, "minecraft:grass_block" | "minecraft:podzol" | "minecraft:mycelium") {
        drop(&mut s, &["snowy"]);
    } else if name.ends_with("_door") {
        // The hinge and power live in the upper half's data, facing and open in the lower's.
        if s.properties.get("half").map(String::as_str) == Some("upper") {
            drop(&mut s, &["facing", "open"]);
        } else {
            drop(&mut s, &["hinge", "powered"]);
        }
    } else if s.properties.get("half").map(String::as_str) == Some("upper")
        && matches!(name, "minecraft:sunflower" | "minecraft:lilac" | "minecraft:rose_bush" | "minecraft:peony" | "minecraft:tall_grass" | "minecraft:large_fern")
    {
        // Every double plant's upper half is the same legacy block.
        s = BlockState::new("minecraft:double_plant_top").with("half", "upper");
    }
    s
}

/// `minecraft:oak_stairs[facing=east,half=bottom]` to a state.
pub fn parse_state(text: &str) -> BlockState {
    let (name, props) = match text.split_once('[') {
        Some((n, p)) => (n, p.trim_end_matches(']')),
        None => (text, ""),
    };
    let mut s = BlockState::new(name);
    for pair in props.split(',').filter(|p| !p.is_empty()) {
        if let Some((k, v)) = pair.split_once('=') {
            s.properties.insert(k.to_string(), v.to_string());
        }
    }
    s
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let from = text.find(start)? + start.len();
    let len = text[from..].find(end)?;
    Some(&text[from..from + len])
}

impl LegacyTables {
    pub fn get() -> &'static LegacyTables {
        static TABLES: OnceLock<LegacyTables> = OnceLock::new();
        TABLES.get_or_init(LegacyTables::load)
    }

    fn load() -> LegacyTables {
        let mut blocks = HashMap::new();
        for line in builtin::legacy_blocks_text().lines().filter(|l| !l.starts_with('#')) {
            let mut p = line.split('\t');
            if let (Some(Ok(id)), Some(Ok(data)), Some(state)) =
                (p.next().map(str::parse::<u8>), p.next().map(str::parse::<u8>), p.next())
            {
                blocks.insert((id, data), parse_state(state));
            }
        }
        let mut items = HashMap::new();
        for line in builtin::legacy_items_text().lines().filter(|l| !l.starts_with('#')) {
            let mut p = line.split('\t');
            let (Some(Ok(id)), Some(Ok(damage)), Some(name)) =
                (p.next().map(str::parse::<i16>), p.next().map(str::parse::<i16>), p.next())
            else {
                continue;
            };
            let name = name.trim_matches('"');
            if name == "-" || name.is_empty() {
                continue;
            }
            let comps = p.next().unwrap_or("");
            items.insert(
                (id, damage),
                ItemInfo {
                    name: name.to_string(),
                    keeps_damage: comps.contains("\"minecraft:damage\":"),
                    entity: comps.find("entity_data").and_then(|at| between(&comps[at..], "id:\"", "\"")).map(str::to_string),
                    map: comps.contains("\"minecraft:map_id\":"),
                },
            );
        }
        let mut biomes: Vec<Option<String>> = vec![None; 256];
        for line in builtin::legacy_biomes_text().lines().filter(|l| !l.starts_with('#')) {
            if let Some((id, name)) = line.split_once('\t') {
                if let Ok(id) = id.parse::<usize>() {
                    if id < 256 {
                        biomes[id] = Some(name.trim().to_string());
                    }
                }
            }
        }
        let mut console_blocks = HashSet::from([0u8]);
        let mut console_items = HashSet::new();
        // The source the ids come from is an early update's; the final one (TU73) also holds the rest
        // of the 1.8-era blocks (hoppers, stained clay, slime, prismarine, fences and gates, ...), with
        // the ids Java and Bedrock gave them.
        console_blocks.extend(146..=197u8);
        for line in builtin::legacy_console_ids_text().lines().filter(|l| !l.starts_with('#')) {
            match line.split_once('\t') {
                Some(("block", id)) => {
                    console_blocks.extend(id.trim().parse::<u8>());
                }
                Some(("item", id)) => {
                    console_items.extend(id.trim().parse::<i16>());
                }
                _ => {}
            }
        }
        // Lowest id and data first, so a state several of them give belongs to the first.
        let mut keys: Vec<(u8, u8)> = blocks.keys().copied().collect();
        keys.sort();
        let mut by_state: HashMap<BlockState, (u8, u8)> = HashMap::new();
        let mut by_name: HashMap<String, Vec<(u8, u8)>> = HashMap::new();
        for (id, data) in keys {
            if !console_blocks.contains(&id) {
                continue;
            }
            let state = &blocks[&(id, data)];
            by_state.entry(canonical(state)).or_insert((id, data));
            by_name.entry(state.name.clone()).or_default().push((id, data));
        }
        let mut keys: Vec<(i16, i16)> = items.keys().copied().collect();
        keys.sort();
        let mut item_by_name: HashMap<String, (i16, i16, bool)> = HashMap::new();
        // An item whose other damage values are wear (a sword at damage 0 is unworn).
        let damageable: HashSet<String> = items.values().filter(|i| i.keeps_damage).map(|i| i.name.clone()).collect();
        for (id, damage) in keys {
            let info = &items[&(id, damage)];
            // A spawn egg's damage is the entity; the game's own table has a few eggs for the wrong one.
            if let Some(e) = &info.entity {
                if info.name.trim_start_matches("minecraft:").trim_end_matches("_spawn_egg") != e.trim_start_matches("minecraft:") {
                    continue;
                }
                // Several small damage values fall back to the same egg; the real one is the entity id.
                item_by_name.insert(info.name.clone(), (id, damage, false));
                continue;
            }
            if console_items.contains(&id) || console_blocks.contains(&(id as u8)) && id < 256 {
                item_by_name.entry(info.name.clone()).or_insert((id, damage, damageable.contains(&info.name)));
            }
        }
        let mut biome_by_name = HashMap::new();
        for (id, name) in biomes.iter().enumerate() {
            if let Some(name) = name {
                biome_by_name.entry(name.clone()).or_insert(id as u8);
            }
        }
        LegacyTables { blocks, items, biomes, console_blocks, by_state, by_name, item_by_name, biome_by_name }
    }

    /// The legacy id and data of a Java state, or None when the console game has nothing like it.
    /// States that differ only in what comes from neighbours share an answer; otherwise the block
    /// with the most properties in common is used.
    pub fn console_block(&self, state: &BlockState) -> Option<(u8, u8)> {
        let key = canonical(state);
        if let Some(found) = self.by_state.get(&key) {
            return Some(*found);
        }
        let candidates = self.by_name.get(&state.name)?;
        candidates
            .iter()
            .max_by_key(|(id, data)| {
                let have = &self.blocks[&(*id, *data)];
                let same = key.properties.iter().filter(|(k, v)| have.properties.get(*k) == Some(*v)).count();
                (same, std::cmp::Reverse((*id, *data)))
            })
            .copied()
    }

    /// The legacy id and damage of a Java item and whether the damage is the item's wear.
    pub fn console_item(&self, name: &str) -> Option<(i16, i16, bool)> {
        self.item_by_name.get(name).copied()
    }

    /// The legacy biome id of a Java biome.
    pub fn console_biome(&self, name: &str) -> Option<u8> {
        self.biome_by_name.get(name).copied()
    }

    /// Every state the table has for blocks of the console game, with its legacy id and data.
    pub fn console_states(&self) -> impl Iterator<Item = (&(u8, u8), &BlockState)> {
        self.blocks.iter().filter(|((id, _), _)| self.console_blocks.contains(id))
    }

    /// The block for a legacy id and data value; unknown data values fall back to data 0.
    pub fn block(&self, id: u8, data: u8) -> Option<&BlockState> {
        self.blocks.get(&(id, data)).or_else(|| self.blocks.get(&(id, 0)))
    }

    /// The item for a legacy id and damage value; unknown damage falls back to damage 0.
    pub fn item(&self, id: i16, damage: i16) -> Option<&ItemInfo> {
        self.items.get(&(id, damage)).or_else(|| self.items.get(&(id, 0)))
    }

    /// The Java biome of a legacy biome id.
    pub fn biome(&self, id: u8) -> &str {
        self.biomes[id as usize].as_deref().unwrap_or("minecraft:plains")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_made_these_tables() {
        let t = LegacyTables::get();
        assert_eq!(t.block(35, 14).unwrap().name, "minecraft:red_wool");
        assert_eq!(t.block(1, 1).unwrap().name, "minecraft:granite");
        let stairs = t.block(53, 2).unwrap();
        assert_eq!(stairs.name, "minecraft:oak_stairs");
        assert_eq!(stairs.properties["facing"], "south");
        assert_eq!(t.item(276, 5).unwrap(), &ItemInfo { name: "minecraft:diamond_sword".into(), keeps_damage: true, entity: None, map: false });
        assert_eq!(t.item(351, 1).unwrap().name, "minecraft:red_dye");
        assert_eq!(t.item(383, 90).unwrap().entity.as_deref(), Some("minecraft:pig"));
        assert_eq!(t.biome(1), "minecraft:plains");
        assert_eq!(t.biome(4), "minecraft:forest");
    }
}
