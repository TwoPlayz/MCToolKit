//! Java to the legacy console games.
//!
//! The reverse of `legacy_up`: a 26.x Java world becomes the save of a console game, using the
//! tables the Java game itself made (`legacy_tables`, read the other way). A console world holds
//! only what the console game defines (`console_ids.tsv`, read from its source), so a block it
//! has no id for goes through the replacement engine first, like a block Bedrock cannot hold.
//!
//! The container follows the platform: Xbox 360 `savegame.dat` (big-endian, LZX), PS3 `GAMEDATA`
//! (big-endian, deflate). A Windows 64 `saveData.ms` (little-endian, zlib) is the same code with
//! another byte order; it is what the converter is tested with, since that game runs on a PC.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;

use indexmap::IndexMap;
use mctk_anvil::{ChunkPos, Region};
use mctk_console::{
    ArchiveEntry, ConsoleRegionBuilder, ConsoleSave, Platform, RawConsoleChunk, SaveArchive, console_block_index,
};
use mctk_java::{BlockState, BlockStorage, Chunk, JavaWorld};
use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};
use mctk_registry::{BlockDef, Edition, Registry, Renames};
use mctk_replace::{Action, ReplacementMap};

use crate::Error;
use crate::java_entities::{Form, block_entity_convert, entity_convert};
use crate::java_items::{Notes, item_down};
use crate::java_level::LevelData;
use crate::legacy_tables::{LegacyTables, canonical};
use crate::options::{ConsoleFormat, ConvertOptions, Progress};

fn s(v: &str) -> Tag {
    Tag::String(v.into())
}

fn list(ty: TagType, tags: Vec<Tag>) -> Tag {
    Tag::List(List::from_tags(ty, tags).expect("one type"))
}

fn num(c: &Compound, k: &str) -> Option<i64> {
    c.get(k).and_then(Tag::as_i64)
}

fn float(c: &Compound, k: &str) -> Option<f64> {
    c.get(k).and_then(Tag::as_f64)
}

/// What a console world can hold, as a Java registry: the blocks and states of the table whose
/// legacy block the console game defines. Properties that Java works out from the neighbours keep
/// every value, since the console stores none of them.
pub fn legacy_registry(java: &Registry) -> Registry {
    legacy_registry_in(java, false)
}

/// [`legacy_registry`], for the newer console games when `newer` (see [`crate::legacy_tables::NEWER_BLOCKS`]).
pub fn legacy_registry_in(java: &Registry, newer: bool) -> Registry {
    let t = LegacyTables::get();
    let mut reg = Registry::new(Edition::Java, "Legacy console");
    let mut values: IndexMap<String, IndexMap<String, Vec<String>>> = IndexMap::new();
    for (_, state) in t.console_states_in(newer) {
        let entry = values.entry(state.name.clone()).or_default();
        for (k, v) in &state.properties {
            let list = entry.entry(k.clone()).or_default();
            if !list.contains(v) {
                list.push(v.clone());
            }
        }
    }
    for (name, mut props) in values {
        let Some(def) = java.blocks.get(&name) else { continue };
        // A property the canonical form drops is not the console's to limit.
        let probe = {
            let mut s = BlockState::new(name.clone());
            for (k, vs) in &props {
                s.properties.insert(k.clone(), vs[0].clone());
            }
            s
        };
        let kept = canonical(&probe);
        for (k, vs) in props.iter_mut() {
            if !kept.properties.contains_key(k) {
                if let Some(all) = def.properties.get(k) {
                    *vs = all.clone();
                }
            }
        }
        // The console has no waterlogging; a waterlogged block loses its water.
        // (It is dropped and the water kept, so either value is carried over.)
        if let Some(all) = def.properties.get("waterlogged") {
            props.insert("waterlogged".into(), all.clone());
        }
        let defaults = def.defaults.clone();
        reg.blocks.insert(name, BlockDef { properties: props, defaults });
    }
    // Beds of every colour: the colour lives in a tile entity, so each is the one console bed.
    if let Some(red) = reg.blocks.get("minecraft:red_bed").cloned() {
        for dye in crate::block_entities_more::DYES.iter() {
            reg.blocks.entry(format!("minecraft:{dye}_bed")).or_insert_with(|| red.clone());
        }
    }
    // Properties the table never mentions (waterlogged) are not part of the console's blocks.
    reg
}

const ENCHANTMENTS: &[(&str, i64)] = &[
    ("protection", 0),
    ("fire_protection", 1),
    ("feather_falling", 2),
    ("blast_protection", 3),
    ("projectile_protection", 4),
    ("respiration", 5),
    ("aqua_affinity", 6),
    ("thorns", 7),
    ("sharpness", 16),
    ("smite", 17),
    ("bane_of_arthropods", 18),
    ("knockback", 19),
    ("fire_aspect", 20),
    ("looting", 21),
    ("efficiency", 32),
    ("silk_touch", 33),
    ("unbreaking", 34),
    ("fortune", 35),
    ("power", 48),
    ("punch", 49),
    ("flame", 50),
    ("infinity", 51),
];

/// The text of a JSON chat component, as plain text.
fn plain_text(json: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::String(t)) => t,
        Ok(serde_json::Value::Object(o)) => {
            let mut out = o.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string();
            if let Some(extra) = o.get("extra").and_then(|e| e.as_array()) {
                for part in extra {
                    out += part.get("text").and_then(|t| t.as_str()).unwrap_or("");
                }
            }
            out
        }
        _ => json.to_string(),
    }
}

/// The damage value of a 1.8 potion for a Java potion name, and whether it is thrown.
fn potion_damage(name: &str, splash: bool) -> i64 {
    let bare = name.trim_start_matches("minecraft:");
    let (kind, base) = match bare.split_once('_') {
        Some(("strong", rest)) => (32, rest),
        Some(("long", rest)) => (64, rest),
        _ => (0, bare),
    };
    const BASE: [&str; 15] = [
        "", "regeneration", "swiftness", "fire_resistance", "poison", "healing", "night_vision", "", "weakness", "strength",
        "slowness", "leaping", "harming", "water_breathing", "invisibility",
    ];
    let low = BASE.iter().position(|b| !b.is_empty() && *b == base);
    let damage = match (low, bare) {
        (Some(i), _) => 8192 + i as i64 + kind,
        (None, "awkward") => 16,
        (None, "thick") => 32,
        (None, "mundane") => 8192,
        _ => 0,
    };
    damage + if splash { 16384 } else { 0 }
}

/// The text id the current console games hold for the item with this legacy number: the number for a block
/// (below 256) and the 1.12 name for any other item the converter knows by name. Used by the editors to write
/// the stacks they add the way the converter does.
pub fn current_item_text(id: i16) -> String {
    match umt().item_names.get(&id) {
        Some(n) if id >= 256 => n.clone(),
        _ => id.to_string(),
    }
}

/// The item ids of the door items: the original converter writes the door's block id (64, 71, 193-197), which is
/// no item and which the game drops from a chest (checked in RPCS3).
fn door_item(name: &str) -> Option<i16> {
    Some(match name.trim_start_matches("minecraft:") {
        "oak_door" => 324,
        "iron_door" => 330,
        "spruce_door" => 427,
        "birch_door" => 428,
        "jungle_door" => 429,
        "acacia_door" => 430,
        "dark_oak_door" => 431,
        _ => return None,
    })
}

thread_local! {
    /// Set while a chunk of the current console games is converted: their items, entities and block entities
    /// are written the way the original converter does (ids as text, the 1.12 names, the fuller entity fields),
    /// where the legacy format keeps numeric item ids and the old entity names.
    static CURRENT_DIALECT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn current_dialect() -> bool {
    CURRENT_DIALECT.with(std::cell::Cell::get)
}

/// What the original converter writes for items and entities (`data/umt_items.tsv`, `data/umt_entities.tsv`,
/// read off a save it made from a probe world), keyed by the Java name without `minecraft:`.
struct Umt {
    items: HashMap<String, (String, i16)>,
    entities: HashMap<String, String>,
    /// The name it writes for an item the legacy table gives a number above 255 (apple 260 is `minecraft:apple`).
    item_names: HashMap<i16, String>,
}

fn umt() -> &'static Umt {
    static UMT: std::sync::OnceLock<Umt> = std::sync::OnceLock::new();
    UMT.get_or_init(|| {
        let rows = |text: &'static str| text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).map(|l| l.trim_end().split('\t').collect::<Vec<_>>()).collect::<Vec<_>>();
        let t = LegacyTables::get();
        let mut items = HashMap::new();
        let mut item_names = HashMap::new();
        for r in rows(include_str!("../data/umt_items.tsv")) {
            if r.len() < 3 {
                continue;
            }
            let damage = r[2].parse::<i16>().unwrap_or(0);
            if r[1].starts_with("minecraft:") {
                if let Some((id, _, _)) = t.console_item(&format!("minecraft:{}", r[0])) {
                    item_names.entry(id).or_insert_with(|| r[1].to_string());
                }
            }
            items.insert(r[0].to_string(), (r[1].to_string(), damage));
        }
        let entities = rows(include_str!("../data/umt_entities.tsv")).into_iter().filter(|r| r.len() >= 2).map(|r| (r[0].to_string(), r[1].to_string())).collect();
        Umt { items, entities, item_names }
    })
}

/// A 1.20.1 item stack as a legacy one (numeric `id`, `Damage`, `tag`), or None when the console
/// game has no such item.
pub(crate) fn item_to_legacy(it: &Compound, notes: &mut Notes) -> Option<Compound> {
    let t = LegacyTables::get();
    let name = it.get("id")?.as_str()?;
    let tag = it.get("tag").and_then(Tag::as_compound).cloned().unwrap_or_default();
    // The id the original converter writes for an item the legacy table lacks (current dialect only).
    let mut named: Option<String> = None;
    let (id, mut damage, keeps) = if matches!(name, "minecraft:potion" | "minecraft:splash_potion") {
        let potion = tag.get("Potion").and_then(Tag::as_str).unwrap_or("minecraft:water");
        (373, potion_damage(potion, name == "minecraft:splash_potion") as i16, false)
    } else if name == "minecraft:filled_map" {
        (358, num(&tag, "map").unwrap_or(0) as i16, false)
    } else if let Some(door) = current_dialect().then(|| door_item(name)).flatten() {
        (door, 0, false)
    } else if let Some((sid, dmg)) = current_dialect()
        .then(|| umt().items.get(name.trim_start_matches("minecraft:")))
        .flatten()
        .filter(|(sid, _)| !name.ends_with("_spawn_egg") && (sid.chars().all(|c| c.is_ascii_digit()) || sid.starts_with("minecraft:")))
        .filter(|_| !t.console_item(name).is_some_and(|(_, _, keeps)| keeps))
        // The original converter writes a door as its block id (64, 71), which is no item: the game drops such a
        // stack (seen in RPCS3). Where the table has a real item above 255 for the name, that wins.
        .filter(|(sid, _)| match (t.console_item(name), sid.parse::<i16>()) {
            (Some((legacy, _, _)), Ok(umt_id)) => !(legacy >= 256 && umt_id < 256),
            _ => true,
        })
    {
        // What the original converter writes for the item, where the damage is no wear.
        named = Some(sid.clone());
        (0, *dmg, false)
    } else if let Some(found) = t.console_item(name) {
        found
    } else if let Some((sid, dmg)) = current_dialect().then(|| umt().items.get(name.trim_start_matches("minecraft:"))).flatten() {
        named = Some(sid.clone());
        (0, *dmg, false)
    } else {
        notes.add(format!("{name} does not exist in the console game"));
        return None;
    };
    let mut out = Compound::new();
    if let Some(slot) = it.get("Slot") {
        out.insert("Slot".into(), slot.clone());
    }
    let mut legacy_tag = Compound::new();
    for (k, v) in &tag {
        match (k.as_str(), v) {
            ("Damage", v) => {
                if keeps {
                    damage = v.as_i64().unwrap_or(0) as i16;
                }
            }
            ("Potion" | "CustomPotionEffects" | "map" | "EntityTag", _) => {}
            ("Enchantments" | "StoredEnchantments", Tag::List(l)) => {
                let mut converted = Vec::new();
                for e in l.iter().filter_map(Tag::as_compound) {
                    let ench = e.get("id").and_then(Tag::as_str).map(|n| n.trim_start_matches("minecraft:"));
                    match ench.and_then(|n| ENCHANTMENTS.iter().find(|(name, _)| *name == n)) {
                        Some((_, id)) => converted.push(Tag::Compound(Compound::from([
                            ("id".to_string(), Tag::Short(*id as i16)),
                            ("lvl".to_string(), Tag::Short(num(e, "lvl").unwrap_or(1) as i16)),
                        ]))),
                        None => notes.add(format!("enchantment {} does not exist in the console game", ench.unwrap_or("?"))),
                    }
                }
                let key = if k == "Enchantments" { "ench" } else { "StoredEnchantments" };
                legacy_tag.insert(key.into(), list(TagType::Compound, converted));
            }
            ("display", Tag::Compound(d)) => {
                let mut d = d.clone();
                if let Some(Tag::String(n)) = d.get("Name") {
                    let n = plain_text(n);
                    d.insert("Name".into(), s(&n));
                }
                if let Some(Tag::List(l)) = d.get("Lore") {
                    let lines = l.iter().filter_map(Tag::as_str).map(|t| s(&plain_text(t))).collect();
                    d.insert("Lore".into(), list(TagType::String, lines));
                }
                legacy_tag.insert("display".into(), Tag::Compound(d));
            }
            _ => {
                legacy_tag.insert(k.clone(), v.clone());
            }
        }
    }
    if current_dialect() {
        // The current games hold an item's id as text: the number for a block, the name for any other item.
        let text = named.unwrap_or_else(|| match umt().item_names.get(&id) {
            Some(n) if id >= 256 => n.clone(),
            _ => id.to_string(),
        });
        out.insert("id".into(), s(&text));
    } else {
        out.insert("id".into(), Tag::Short(id));
    }
    out.insert("Count".into(), Tag::Byte(num(it, "Count").unwrap_or(1) as i8));
    out.insert("Damage".into(), Tag::Short(damage));
    if !legacy_tag.is_empty() {
        out.insert("tag".into(), Tag::Compound(legacy_tag));
    }
    Some(out)
}

fn items_to_legacy(t: Option<&Tag>, notes: &mut Notes) -> Tag {
    let converted: Vec<Tag> = match t {
        Some(Tag::List(l)) => l.iter().filter_map(Tag::as_compound).filter_map(|c| item_to_legacy(c, notes)).map(Tag::Compound).collect(),
        _ => Vec::new(),
    };
    list(TagType::Compound, converted)
}

/// Java entity ids to the console game's, for the ones it has.
const ENTITIES: &[(&str, &str)] = &[
    ("item", "Item"),
    ("falling_block", "FallingSand"),
    ("experience_orb", "XPOrb"),
    ("painting", "Painting"),
    ("arrow", "Arrow"),
    ("snowball", "Snowball"),
    ("fireball", "Fireball"),
    ("small_fireball", "SmallFireball"),
    ("ender_pearl", "ThrownEnderpearl"),
    ("eye_of_ender", "EyeOfEnderSignal"),
    ("experience_bottle", "ThrownExpBottle"),
    ("item_frame", "ItemFrame"),
    ("tnt", "PrimedTnt"),
    ("minecart", "Minecart"),
    ("chest_minecart", "Minecart"),
    ("furnace_minecart", "Minecart"),
    ("boat", "Boat"),
    ("creeper", "Creeper"),
    ("skeleton", "Skeleton"),
    ("wither_skeleton", "Skeleton"),
    ("spider", "Spider"),
    ("giant", "Giant"),
    ("zombie", "Zombie"),
    ("zombie_villager", "Zombie"),
    ("slime", "Slime"),
    ("ghast", "Ghast"),
    ("zombified_piglin", "PigZombie"),
    ("enderman", "Enderman"),
    ("cave_spider", "CaveSpider"),
    ("silverfish", "Silverfish"),
    ("blaze", "Blaze"),
    ("magma_cube", "LavaSlime"),
    ("ender_dragon", "EnderDragon"),
    ("pig", "Pig"),
    ("sheep", "Sheep"),
    ("cow", "Cow"),
    ("chicken", "Chicken"),
    ("squid", "Squid"),
    ("wolf", "Wolf"),
    ("mooshroom", "MushroomCow"),
    ("snow_golem", "SnowMan"),
    ("ocelot", "Ozelot"),
    ("cat", "Ozelot"),
    ("iron_golem", "VillagerGolem"),
    ("villager", "Villager"),
    ("end_crystal", "EnderCrystal"),
];

fn uuid_longs(c: &Compound) -> Option<(i64, i64)> {
    match c.get("UUID") {
        Some(Tag::IntArray(u)) if u.len() == 4 => {
            Some((((u[0] as i64) << 32) | (u[1] as u32 as i64), ((u[2] as i64) << 32) | (u[3] as u32 as i64)))
        }
        _ => None,
    }
}

/// One 1.20.1 entity as a legacy one, with the fields the console game reads. False when the
/// console game has no such entity.
pub(crate) fn entity_to_legacy(e: &Compound, notes: &mut Notes) -> Option<Compound> {
    let id = e.get("id")?.as_str()?.trim_start_matches("minecraft:").to_string();
    let current = current_dialect();
    let old_name: &str = match ENTITIES.iter().find(|(n, _)| *n == id) {
        Some((_, old)) => old,
        // The newer games have more mobs (bats, horses, the aquatic ones, ...) under their own names.
        None => match current.then(|| umt().entities.get(&id)).flatten() {
            Some(name) => name,
            None => {
                notes.add(format!("entity {id} does not exist in the console game"));
                return None;
            }
        },
    };
    let old = &old_name;
    let mut out = Compound::new();
    // The ones the original converter left out get their 1.12 names, which the newer games read too.
    let written = if current { umt().entities.get(&id).cloned().unwrap_or_else(|| format!("minecraft:{id}")) } else { old_name.to_string() };
    out.insert("id".into(), s(&written));
    for k in ["Pos", "Motion", "Rotation"] {
        if let Some(v) = e.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    out.insert("FallDistance".into(), Tag::Float(float(e, "FallDistance").unwrap_or(0.0) as f32));
    out.insert("Fire".into(), Tag::Short(num(e, "Fire").unwrap_or(-1) as i16));
    out.insert("Air".into(), Tag::Short(num(e, "Air").unwrap_or(300) as i16));
    out.insert("OnGround".into(), Tag::Byte(num(e, "OnGround").unwrap_or(1) as i8));
    if let Some((most, least)) = uuid_longs(e) {
        out.insert("UUIDMost".into(), Tag::Long(most));
        out.insert("UUIDLeast".into(), Tag::Long(least));
    }
    let living = !matches!(
        *old,
        "Item" | "FallingSand" | "XPOrb" | "Painting" | "Arrow" | "Snowball" | "Fireball" | "SmallFireball" | "ThrownEnderpearl" | "EyeOfEnderSignal"
            | "ThrownExpBottle" | "ItemFrame" | "PrimedTnt" | "Minecart" | "Boat" | "EnderCrystal"
    );
    if living {
        out.insert("Health".into(), Tag::Short(float(e, "Health").unwrap_or(10.0).round() as i16));
        for k in ["HurtTime", "DeathTime", "AttackTime"] {
            out.insert(k.into(), Tag::Short(num(e, k).unwrap_or(0) as i16));
        }
        if let Some(Tag::List(effects)) = e.get("ActiveEffects") {
            let converted: Vec<Tag> = effects
                .iter()
                .filter_map(Tag::as_compound)
                .map(|x| {
                    Tag::Compound(Compound::from([
                        ("Id".to_string(), Tag::Byte(num(x, "Id").unwrap_or(0) as i8)),
                        ("Amplifier".to_string(), Tag::Byte(num(x, "Amplifier").unwrap_or(0) as i8)),
                        ("Duration".to_string(), Tag::Int(num(x, "Duration").unwrap_or(0) as i32)),
                    ]))
                })
                .collect();
            out.insert("ActiveEffects".into(), list(TagType::Compound, converted));
        }
    }
    if let Some(a) = num(e, "Age") {
        out.insert("Age".into(), Tag::Int(a as i32));
    }
    if let Some(l) = num(e, "InLove") {
        out.insert("InLove".into(), Tag::Int(l as i32));
    }
    let flag = |k: &str| num(e, k).unwrap_or(0) as i8;
    match *old {
        "Pig" => {
            out.insert("Saddle".into(), Tag::Byte(flag("Saddle")));
        }
        "Sheep" => {
            out.insert("Sheared".into(), Tag::Byte(flag("Sheared")));
            out.insert("Color".into(), Tag::Byte(flag("Color")));
        }
        "Wolf" => {
            out.insert("Angry".into(), Tag::Byte(flag("Angry")));
            out.insert("CollarColor".into(), Tag::Byte(num(e, "CollarColor").unwrap_or(14) as i8));
            out.insert("Sitting".into(), Tag::Byte(flag("Sitting")));
            out.insert("Owner".into(), s(""));
        }
        "Creeper" => {
            out.insert("powered".into(), Tag::Byte(flag("powered")));
        }
        "Slime" | "LavaSlime" => {
            out.insert("Size".into(), Tag::Int(num(e, "Size").unwrap_or(1) as i32));
        }
        "Zombie" => {
            out.insert("IsBaby".into(), Tag::Byte(flag("IsBaby")));
            out.insert("IsVillager".into(), Tag::Byte(i8::from(id == "zombie_villager")));
            out.insert("ConversionTime".into(), Tag::Int(-1));
        }
        "Ozelot" => {
            let t = num(e, "CatType").unwrap_or(0);
            out.insert("CatType".into(), Tag::Int(if id == "cat" { (t as i32).clamp(1, 3) } else { 0 }));
        }
        "Villager" => {
            let profession = match e.get("VillagerData").and_then(|d| d.get("profession")).and_then(Tag::as_str).unwrap_or("") {
                "minecraft:librarian" => 1,
                "minecraft:cleric" => 2,
                "minecraft:armorer" | "minecraft:toolsmith" | "minecraft:weaponsmith" => 3,
                "minecraft:butcher" | "minecraft:leatherworker" => 4,
                _ => 0,
            };
            out.insert("Profession".into(), Tag::Int(profession));
            out.insert("Riches".into(), Tag::Int(0));
        }
        "Item" => {
            let stack = e.get("Item").and_then(Tag::as_compound).and_then(|i| item_to_legacy(i, notes))?;
            out.insert("Item".into(), Tag::Compound(stack));
            out.insert("Health".into(), Tag::Short(num(e, "Health").unwrap_or(5) as i16));
            out.insert("Age".into(), Tag::Short(num(e, "Age").unwrap_or(0) as i16));
        }
        "FallingSand" => {
            // The block it is made of, as the console game's id and data.
            let bs = e.get("BlockState").or_else(|| e.get("block_state")).and_then(Tag::as_compound)?;
            let mut state = BlockState::new(bs.get("Name").and_then(Tag::as_str).unwrap_or("minecraft:sand"));
            if let Some(props) = bs.get("Properties").and_then(Tag::as_compound) {
                for (k, v) in props {
                    if let Some(v) = v.as_str() {
                        state.properties.insert(k.clone(), v.to_string());
                    }
                }
            }
            let Some((tile, data)) = LegacyTables::get().console_block(&state) else {
                notes.add(format!("falling block {} does not exist in the console game", state.name));
                return None;
            };
            out.insert("TileID".into(), Tag::Int(i32::from(tile)));
            out.insert("Data".into(), Tag::Byte(data as i8));
            out.insert("Time".into(), Tag::Byte(num(e, "Time").unwrap_or(0).clamp(0, 127) as i8));
        }
        "XPOrb" => {
            out.insert("Value".into(), Tag::Short(num(e, "Value").unwrap_or(1) as i16));
            out.insert("Health".into(), Tag::Short(5));
            out.insert("Age".into(), Tag::Short(num(e, "Age").unwrap_or(0) as i16));
        }
        "PrimedTnt" => {
            out.insert("Fuse".into(), Tag::Byte(num(e, "fuse").or_else(|| num(e, "Fuse")).unwrap_or(80) as i8));
        }
        "Minecart" => {
            let kind = match id.as_str() {
                "chest_minecart" => 1,
                "furnace_minecart" => 2,
                _ => 0,
            };
            out.insert("Type".into(), Tag::Int(kind));
            if kind == 1 {
                out.insert("Items".into(), items_to_legacy(e.get("Items"), notes));
            }
        }
        "ItemFrame" | "Painting" => {
            // Java 1.20 facing: 2 north, 3 south, 4 west, 5 east; the console uses 0 south, 1 west, 2 north, 3 east.
            let facing = num(e, "Facing").unwrap_or(2);
            let dir = match facing {
                3 => 0,
                4 => 1,
                5 => 3,
                _ => 2,
            };
            out.insert("Direction".into(), Tag::Byte(dir));
            out.insert("Dir".into(), Tag::Byte(dir));
            for (k, from) in [("TileX", "TileX"), ("TileY", "TileY"), ("TileZ", "TileZ")] {
                out.insert(k.into(), Tag::Int(num(e, from).unwrap_or(0) as i32));
            }
            if *old == "ItemFrame" {
                if let Some(i) = e.get("Item").and_then(Tag::as_compound).and_then(|i| item_to_legacy(i, notes)) {
                    out.insert("Item".into(), Tag::Compound(i));
                    out.insert("ItemRotation".into(), Tag::Byte(flag("ItemRotation")));
                }
            } else {
                let motive = e.get("variant").and_then(Tag::as_str).unwrap_or("kebab").trim_start_matches("minecraft:").to_string();
                let camel: String = motive.split('_').map(|w| w[..1].to_uppercase() + &w[1..]).collect();
                out.insert("Motive".into(), s(&camel));
            }
        }
        _ => {}
    }
    if current {
        current_entity_fields(&mut out, living, e);
    }
    Some(out)
}

/// The fields the original converter gives every entity of the current games (a text UUID, the flags, and for
/// the living the armour and hand slots and the health as a float), added to what the legacy form has.
fn current_entity_fields(out: &mut Compound, living: bool, e: &Compound) {
    if let (Some(Tag::Long(most)), Some(Tag::Long(least))) = (out.shift_remove("UUIDMost"), out.shift_remove("UUIDLeast")) {
        out.insert("UUID".into(), s(&format!("{:016x}{:016x}", most as u64, least as u64)));
    }
    out.insert("Invulnerable".into(), Tag::Byte(num(e, "Invulnerable").unwrap_or(0) as i8));
    out.insert("Dimension".into(), Tag::Int(0));
    out.insert("PortalCooldown".into(), Tag::Int(num(e, "PortalCooldown").unwrap_or(0) as i32));
    if living {
        for k in ["CanPickUpLoot", "FallFlying", "LeftHanded", "PersistenceRequired"] {
            out.insert(k.into(), Tag::Byte(num(e, k).unwrap_or(0) as i8));
        }
        out.shift_remove("AttackTime");
        out.insert("HurtByTimestamp".into(), Tag::Short(0));
        out.insert("AbsorptionAmount".into(), Tag::Float(float(e, "AbsorptionAmount").unwrap_or(0.0) as f32));
        out.insert("Health".into(), Tag::Float(float(e, "Health").unwrap_or(20.0) as f32));
        let empty = |n: usize| list(TagType::Compound, (0..n).map(|_| Tag::Compound(Compound::new())).collect());
        let chances = |n: usize| list(TagType::Float, (0..n).map(|_| Tag::Float(0.085)).collect());
        out.insert("ArmorItems".into(), empty(4));
        out.insert("HandItems".into(), empty(2));
        out.insert("ArmorDropChances".into(), chances(4));
        out.insert("HandDropChances".into(), chances(2));
    }
}

const BLOCK_ENTITIES: &[(&str, &str)] = &[
    ("furnace", "Furnace"),
    ("chest", "Chest"),
    ("trapped_chest", "Chest"),
    ("ender_chest", "EnderChest"),
    ("jukebox", "RecordPlayer"),
    ("dispenser", "Trap"),
    ("sign", "Sign"),
    ("mob_spawner", "MobSpawner"),
    ("note_block", "Music"),
    ("brewing_stand", "Cauldron"),
    ("enchanting_table", "EnchantTable"),
    ("end_portal", "Airportal"),
    ("skull", "Skull"),
];

/// One 1.20.1 block entity as a legacy one; None when the console game has no such block entity.
pub(crate) fn block_entity_to_legacy(te: &Compound, notes: &mut Notes) -> Option<Compound> {
    let id = te.get("id")?.as_str()?.trim_start_matches("minecraft:");
    let Some((_, old)) = BLOCK_ENTITIES.iter().find(|(n, _)| *n == id) else {
        notes.add(format!("block entity {id} does not exist in the console game"));
        return None;
    };
    let mut out = Compound::new();
    // The current games name block entities the way 1.12 does.
    let written = match (current_dialect(), *old) {
        (true, "Furnace") => "minecraft:furnace",
        (true, "Chest") => "minecraft:chest",
        (true, "EnderChest") => "minecraft:ender_chest",
        (true, "RecordPlayer") => "minecraft:jukebox",
        (true, "Trap") => "minecraft:dispenser",
        (true, "Sign") => "minecraft:sign",
        (true, "MobSpawner") => "minecraft:mob_spawner",
        (true, "Music") => "minecraft:noteblock",
        (true, "Cauldron") => "minecraft:brewing_stand",
        (true, "EnchantTable") => "minecraft:enchanting_table",
        (true, "Airportal") => "minecraft:end_portal",
        (true, "Skull") => "minecraft:skull",
        _ => *old,
    };
    out.insert("id".into(), s(written));
    for k in ["x", "y", "z"] {
        out.insert(k.into(), Tag::Int(num(te, k)? as i32));
    }
    match *old {
        "Furnace" => {
            for k in ["BurnTime", "CookTime"] {
                out.insert(k.into(), Tag::Short(num(te, k).unwrap_or(0) as i16));
            }
            out.insert("Items".into(), items_to_legacy(te.get("Items"), notes));
        }
        "Chest" | "Trap" | "Cauldron" => {
            if *old == "Cauldron" {
                out.insert("BrewTime".into(), Tag::Short(num(te, "BrewTime").unwrap_or(0) as i16));
            }
            out.insert("Items".into(), items_to_legacy(te.get("Items"), notes));
        }
        "Sign" => {
            let lines: Vec<String> = match te.get("front_text").and_then(|f| f.get("messages")) {
                Some(Tag::List(l)) => l.iter().map(|m| m.as_str().map(plain_text).unwrap_or_default()).collect(),
                _ => Vec::new(),
            };
            for (i, k) in ["Text1", "Text2", "Text3", "Text4"].iter().enumerate() {
                out.insert((*k).into(), s(lines.get(i).map_or("", String::as_str)));
            }
        }
        "MobSpawner" => {
            let entity = te
                .get("SpawnData")
                .and_then(|d| d.get("entity"))
                .and_then(|e| e.get("id"))
                .and_then(Tag::as_str)
                .map(|n| n.trim_start_matches("minecraft:"))
                .and_then(|n| ENTITIES.iter().find(|(j, _)| *j == n))
                .map_or("Pig", |(_, o)| *o);
            out.insert("EntityId".into(), s(entity));
            for k in ["Delay", "MinSpawnDelay", "MaxSpawnDelay", "SpawnCount", "MaxNearbyEntities", "RequiredPlayerRange", "SpawnRange"] {
                if let Some(v) = te.get(k) {
                    out.insert(k.into(), v.clone());
                }
            }
        }
        "Music" => {
            out.insert("note".into(), Tag::Byte(num(te, "note").unwrap_or(0) as i8));
        }
        "RecordPlayer" => {
            if let Some(i) = te.get("RecordItem").and_then(Tag::as_compound).and_then(|i| item_to_legacy(i, notes)) {
                out.insert("RecordItem".into(), Tag::Compound(i));
            }
        }
        _ => {}
    }
    Some(out)
}

/// Blocks that let the sky light through.
const CLEAR: &[u8] = &[
    0, 6, 8, 9, 18, 20, 27, 28, 30, 31, 32, 37, 38, 39, 40, 50, 51, 55, 59, 63, 64, 65, 66, 68, 69, 70, 71, 72, 75, 76, 77,
    83, 90, 92, 101, 102, 104, 105, 106, 107, 111, 115, 131, 132, 140, 141, 142, 143, 171,
];

/// The light a block gives (by its numeric id, as the console game has them).
fn light_emitted(id: u8) -> u8 {
    match id {
        10 | 11 | 51 | 89 | 91 | 119 | 124 | 138 | 169 => 15,
        50 | 198 => 14,
        62 => 13,
        90 => 11,
        74 | 94 | 150 => 9,
        76 | 130 => 7,
        213 => 3,
        39 | 117 | 120 | 122 => 1,
        _ => 0,
    }
}

/// How much more than a step's 1 light loses going into a see-through block: water 3,
/// leaves 1 (as the game dims them).
fn light_cost(id: u8) -> u8 {
    match id {
        8 | 9 => 3,
        18 | 161 => 1,
        _ => 0,
    }
}

/// Spreads light (one value per block, in the console's block order) from where it is to the
/// see-through blocks around, a step dimmer each block (more through water and leaves), within
/// the chunk: as the game's lighting fills in from the sky and from glowing blocks.
fn spread_light(light: &mut [u8], ids: &[u8], h: usize) {
    let mut queue: std::collections::VecDeque<(usize, usize, usize)> = std::collections::VecDeque::new();
    for y in 0..h {
        for z in 0..16 {
            for x in 0..16 {
                if light[console_block_index(x, y, z)] > 1 {
                    queue.push_back((x, y, z));
                }
            }
        }
    }
    while let Some((x, y, z)) = queue.pop_front() {
        let here = light[console_block_index(x, y, z)];
        let near = [
            (x.wrapping_sub(1), y, z),
            (x + 1, y, z),
            (x, y.wrapping_sub(1), z),
            (x, y + 1, z),
            (x, y, z.wrapping_sub(1)),
            (x, y, z + 1),
        ];
        for (nx, ny, nz) in near {
            if nx >= 16 || nz >= 16 || ny >= h {
                continue;
            }
            let at = console_block_index(nx, ny, nz);
            let id = ids[at];
            if !CLEAR.contains(&id) && light_emitted(id) == 0 {
                continue;
            }
            let v = here.saturating_sub(1 + light_cost(id));
            if v > light[at] {
                light[at] = v;
                if v > 1 {
                    queue.push_back((nx, ny, nz));
                }
            }
        }
    }
}

/// Flower pot contents: the item the pot holds to the data value the console game keeps.
fn potted(name: &str) -> Option<u8> {
    // An empty pot is the console's pot with no plant.
    if name == "minecraft:flower_pot" {
        return Some(0);
    }
    Some(match name.trim_start_matches("minecraft:potted_") {
        "poppy" => 1,
        "dandelion" => 2,
        "oak_sapling" => 3,
        "spruce_sapling" => 4,
        "birch_sapling" => 5,
        "jungle_sapling" => 6,
        "red_mushroom" => 7,
        "brown_mushroom" => 8,
        "cactus" => 9,
        "dead_bush" => 10,
        "fern" => 11,
        _ => return None,
    })
}

struct Ctx<'a> {
    map: &'a ReplacementMap,
    registry: &'a Registry,
    renames: &'a Renames,
    java: &'a Registry,
    classic: &'a Registry,
    platform: Platform,
    /// Writing for the newer console games (more block ids).
    newer: bool,
}

/// What converting one chunk gave: nothing (left out, with what it counted) or the chunk as it will be saved.
enum Chunked {
    Skip(ConsoleReport),
    Done(ChunkPos, RawConsoleChunk, ConsoleReport),
}

/// A report of what a conversion to a console game changed or left out.
#[derive(Debug, Default, Clone)]
pub struct ConsoleReport {
    pub chunks: u64,
    pub entities: u64,
    pub block_entities: u64,
    /// Source state to what it became, with the number of blocks.
    pub replaced: IndexMap<String, (String, u64)>,
    pub notes: Notes,
    /// Chunks outside the area a console world can have.
    pub outside: u64,
    /// Chunks left out because the game had not finished generating them (see `skip_partial_chunks`).
    pub partial: u64,
}

/// Whether the console games write this Java block state as it is: the table has it, or the conversion
/// writes it itself (a block under an id above 255, a skull, a flower pot, a bed). The REPLACE step
/// leaves such states out, and the conversion keeps them.
pub fn console_supports(java: &Registry, state: &BlockState, newer: bool) -> bool {
    let t = LegacyTables::get();
    let mut full = java
        .complete_state(&state.name, state.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .map_or_else(|| state.clone(), |p| BlockState { name: state.name.clone(), properties: p });
    if is_plain_air(&full.name) {
        return true;
    }
    alias_block(&mut full);
    if full.name.ends_with("_skull") || full.name.ends_with("_head") && full.name != "minecraft:piston_head" || potted(&full.name).is_some() {
        return true;
    }
    if newer && extended_value(state).is_some() {
        return true;
    }
    if full.name.ends_with("_bed") {
        full.name = "minecraft:red_bed".into();
    }
    t.console_block_in(&full, newer).is_some()
}

/// Air the game only names differently (`cave_air` and `void_air` are air to the console).
fn is_plain_air(name: &str) -> bool {
    matches!(name, "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
}

/// Java blocks the console has under another name: its stone slab (the one with the smooth top) is Java's
/// `smooth_stone_slab`, and Java's own `stone_slab` is the same slab to a console game.
fn alias_block(state: &mut BlockState) {
    if state.name == "minecraft:stone_slab" {
        state.name = "minecraft:smooth_stone_slab".into();
    }
}

impl ConsoleReport {
    /// Adds what another report counted (the reports of chunks converted apart).
    fn merge(&mut self, o: ConsoleReport) {
        self.chunks += o.chunks;
        self.entities += o.entities;
        self.block_entities += o.block_entities;
        for (k, (to, n)) in o.replaced {
            self.replaced.entry(k).and_modify(|(_, m)| *m += n).or_insert((to, n));
        }
        self.notes.merge(o.notes);
        self.outside += o.outside;
        self.partial += o.partial;
    }
}

fn state_to_console(state: &BlockState, ctx: &Ctx, report: &mut ConsoleReport, uses: u64) -> (u8, u8, Option<BlockState>) {
    let t = LegacyTables::get();
    let mut full = ctx
        .java
        .complete_state(&state.name, state.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .map_or_else(|| state.clone(), |p| BlockState { name: state.name.clone(), properties: p });
    // Cave air and void air are plain air to the console, not something replaced.
    if matches!(full.name.as_str(), "minecraft:cave_air" | "minecraft:void_air") {
        return (0, 0, None);
    }
    alias_block(&mut full);
    // Beds are one block on the console; the colour goes in a Bed tile entity.
    if full.name.ends_with("_bed") && full.name != "minecraft:red_bed" {
        let mut red = full.clone();
        red.name = "minecraft:red_bed".into();
        if let Some((id, data)) = t.console_block_in(&red, ctx.newer) {
            let data = if ctx.newer { current_data(state, &full, id, data) } else { data };
            return (id, data, Some(full));
        }
    }
    // Skulls and flower pots are the console's blocks with data of their own.
    if full.name.ends_with("_skull") || full.name.ends_with("_head") && full.name != "minecraft:piston_head" {
        let wall = full.name.contains("wall");
        let data = if wall {
            match full.properties.get("facing").map(String::as_str) {
                Some("south") => 3,
                Some("west") => 4,
                Some("east") => 5,
                _ => 2,
            }
        } else {
            1
        };
        return (144, data, Some(full));
    }
    if let Some(data) = potted(&full.name) {
        return (140, data, None);
    }
    // A state the console has as it is stays, unless the user chose something else for it.
    let chosen = ctx.map.get(&full.to_string()).or_else(|| ctx.map.get(&full.name)).is_some();
    let direct = !chosen && t.console_block_in(&full, ctx.newer).is_some();
    let resolved = match ctx.map.resolve(&full, ctx.registry, ctx.renames) {
        _ if direct => full.clone(),
        None | Some((Action::Keep, _)) => full.clone(),
        Some((Action::Replace(r), _)) => r,
        Some((Action::Air, _)) => BlockState::air(),
        Some((Action::Block(_), _)) => full.clone(),
    };
    if resolved != full {
        report
            .replaced
            .entry(full.to_string())
            .and_modify(|(_, n)| *n += uses)
            .or_insert((resolved.to_string(), uses));
    }
    match t.console_block_in(&resolved, ctx.newer) {
        Some((id, data)) => {
            // The newer games keep still water and lava where the legacy table has the flowing ones.
            let id = if ctx.newer { still(id) } else { id };
            let data = if ctx.newer { current_data(state, &full, id, data) } else { data };
            (id, data, None)
        }
        None => {
            report.notes.add(format!("{} has no console block; left as air", resolved.name));
            (0, 0, None)
        }
    }
}

/// Converts one chunk of a 26.x Java world (blocks, biomes, block entities) to the console's
/// `Level` compound.
fn chunk_to_console(chunk: &Chunk, entities: &[Tag], dim: i32, raised: i32, ctx: &Ctx, report: &mut ConsoleReport) -> Option<NamedTag> {
    let t = LegacyTables::get();
    CURRENT_DIALECT.with(|c| c.set(ctx.newer));
    // The console stores 256 layers in every dimension, though the Nether plays in the lower half.
    let height = 256;
    let mut ids = vec![0u8; 65536];
    let mut datas = vec![0u8; 65536];
    // Blocks of a waterlogged state: the console keeps their water in a second layer.
    let mut in_water = vec![0u8; 65536];
    // Blocks the console games only have under an id above 255, as pairs of (cell, value).
    let mut extended: Vec<i32> = Vec::new();
    let mut extra_tiles: HashMap<(i32, i32, i32), BlockState> = HashMap::new();
    let mut flower_pots: Vec<(i32, i32, i32, u8)> = Vec::new();
    for section in &chunk.sections {
        let Some(BlockStorage::Palette(p)) = &section.blocks else { continue };
        // Each palette entry is converted once.
        let mut uses = vec![0u64; p.palette.len()];
        for &i in &p.indices {
            uses[i as usize] += 1;
        }
        let converted: Vec<(u8, u8, Option<BlockState>)> =
            p.palette.iter().zip(&uses).map(|(st, n)| state_to_console(st, ctx, report, *n)).collect();
        let wet: Vec<bool> = p.palette.iter().map(|st| st.properties.get("waterlogged").is_some_and(|v| v == "true")).collect();
        let wide: Vec<Option<u16>> = p.palette.iter().map(extended_value).collect();
        for y in 0..16usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    let wy = section.y as i32 * 16 + y as i32;
                    if wy < 0 || wy >= height {
                        continue;
                    }
                    let i = p.indices[mctk_java::block_index(x, y, z)] as usize;
                    let (id, data, tile) = &converted[i];
                    let at = console_block_index(x, wy as usize, z);
                    if let Some(v) = wide[i] {
                        extended.push(at as i32);
                        extended.push(i32::from(v));
                        // Opaque blocks stand in as a one-byte block for the light and the heights.
                        match (v & 0x7fff) >> 4 {
                            257 => ids[at] = 86,
                            259 | 269 | 273 => ids[at] = 1,
                            295..=300 => ids[at] = 17,
                            _ => {}
                        }
                        continue;
                    }
                    if *id == 0 {
                        // A block the console lacks that sat in water leaves the water.
                        in_water[at] = u8::from(wet[i]);
                        continue;
                    }
                    ids[at] = *id;
                    datas[at] = *data;
                    in_water[at] = u8::from(wet[i]);
                    if let Some(st) = tile {
                        extra_tiles.insert((chunk.x * 16 + x as i32, wy, chunk.z * 16 + z as i32), st.clone());
                    }
                    let _ = &mut flower_pots;
                }
            }
        }
    }

    // Heights, and the light as the game would work it out (within the chunk): the sky's down
    // each column (dimmed by water and leaves) and spread sideways and down under overhangs, and
    // the glowing blocks' spread out from them.
    let mut heightmap = vec![0i8; 256];
    let set = |arr: &mut [u8], i: usize, v: u8| {
        if i % 2 == 0 {
            arr[i / 2] = (arr[i / 2] & 0xf0) | v;
        } else {
            arr[i / 2] = (arr[i / 2] & 0x0f) | (v << 4);
        }
    };
    let h = height as usize;
    let cells = 16 * 16 * h;
    let mut sky_v = vec![0u8; cells];
    let mut lamp_v = vec![0u8; cells];
    let cell = |x: usize, y: usize, z: usize| console_block_index(x, y, z);
    for z in 0..16usize {
        for x in 0..16usize {
            let mut top = 0usize;
            let mut light = if dim == -1 { 0u8 } else { 15u8 };
            for y in (0..h).rev() {
                let at = cell(x, y, z);
                let id = ids[at];
                if id != 0 && top == 0 {
                    top = y + 1;
                }
                if light > 0 {
                    if !CLEAR.contains(&id) {
                        light = 0;
                    } else {
                        light = light.saturating_sub(light_cost(id));
                    }
                }
                sky_v[at] = light;
                let glow = light_emitted(id);
                if glow > 0 {
                    lamp_v[at] = glow;
                }
            }
            heightmap[z * 16 + x] = top.min(255) as i8;
        }
    }
    spread_light(&mut sky_v, &ids, h);
    spread_light(&mut lamp_v, &ids, h);
    let mut sky = vec![0u8; 32768];
    let mut lamp = vec![0u8; 32768];
    for i in 0..cells.min(32768 * 2) {
        set(&mut sky, i, sky_v[i]);
        set(&mut lamp, i, lamp_v[i]);
    }

    // Biomes: one per column, from the cell at the surface section.
    let biome_of = |x: usize, z: usize| -> u8 {
        let section = chunk.sections.iter().find(|s| s.y == 4).or_else(|| chunk.sections.iter().find(|s| s.biomes.is_some()));
        section
            .and_then(|s| s.biomes.as_ref())
            .and_then(|b| b.palette.get(*b.indices.get((z / 4) * 4 + x / 4)? as usize))
            .and_then(|name| if ctx.newer { current_biome(name) } else { t.console_biome(name) })
            .unwrap_or(1)
    };
    let biomes: Vec<i8> = (0..256).map(|i| biome_of(i % 16, i / 16) as i8).collect();

    // Block entities (skulls get theirs from the block when the chunk has none).
    let mut tiles = Vec::new();
    let mut seen: std::collections::HashSet<(i32, i32, i32)> = std::collections::HashSet::new();
    let root = chunk.to_nbt();
    if let Some(Tag::List(l)) = root.tag.get("block_entities") {
        for te in l.iter().filter_map(Tag::as_compound) {
            let mut te = te.clone();
            if raised != 0 {
                crate::depth::raise(&mut te, raised);
            }
            if !block_entity_convert(&mut te, Form::Classic, ctx.classic, &mut report.notes) {
                continue;
            }
            if let Some(legacy) = block_entity_to_legacy(&te, &mut report.notes) {
                seen.insert((num(&legacy, "x").unwrap_or(0) as i32, num(&legacy, "y").unwrap_or(0) as i32, num(&legacy, "z").unwrap_or(0) as i32));
                report.block_entities += 1;
                tiles.push(Tag::Compound(legacy));
            }
        }
    }
    // In position order, so that converting the same world twice gives the same bytes (a HashMap's
    // iteration order differs from run to run).
    let mut extra_sorted: Vec<_> = extra_tiles.iter().collect();
    extra_sorted.sort_by_key(|((x, y, z), _)| (*y, *z, *x));
    for ((x, y, z), st) in extra_sorted {
        if seen.contains(&(*x, *y, *z)) {
            continue;
        }
        if let Some(dye) = st.name.strip_prefix("minecraft:").and_then(|n| n.strip_suffix("_bed")) {
            let color = crate::block_entities_more::DYES.iter().position(|d| *d == dye).unwrap_or(14);
            tiles.push(Tag::Compound(Compound::from([
                ("id".to_string(), s("Bed")),
                ("x".to_string(), Tag::Int(*x)),
                ("y".to_string(), Tag::Int(*y)),
                ("z".to_string(), Tag::Int(*z)),
                ("color".to_string(), Tag::Int(color as i32)),
            ])));
            report.block_entities += 1;
            continue;
        }
        let kind = match st.name.trim_start_matches("minecraft:").trim_end_matches("_wall_skull").trim_end_matches("_wall_head").trim_end_matches("_skull").trim_end_matches("_head") {
            "wither_skeleton" => 1,
            "zombie" => 2,
            "player" => 3,
            "creeper" => 4,
            _ => 0,
        };
        let rot = st.properties.get("rotation").and_then(|r| r.parse::<i8>().ok()).unwrap_or(0);
        tiles.push(Tag::Compound(Compound::from([
            ("id".to_string(), s("Skull")),
            ("x".to_string(), Tag::Int(*x)),
            ("y".to_string(), Tag::Int(*y)),
            ("z".to_string(), Tag::Int(*z)),
            ("SkullType".to_string(), Tag::Byte(kind)),
            ("Rot".to_string(), Tag::Byte(rot)),
            ("ExtraType".to_string(), s("")),
        ])));
        report.block_entities += 1;
    }

    let mut legacy_entities = Vec::new();
    for e in entities.iter().filter_map(Tag::as_compound) {
        let mut e = e.clone();
        if raised != 0 {
            crate::depth::raise(&mut e, raised);
        }
        if entity_convert(&mut e, Form::Classic, ctx.classic, &mut report.notes) {
            if let Some(legacy) = entity_to_legacy(&e, &mut report.notes) {
                report.entities += 1;
                legacy_entities.push(Tag::Compound(legacy));
            }
        }
    }

    let nibbles = |v: &[u8]| Tag::ByteArray(v.iter().map(|&b| b as i8).collect());
    let mut packed_data = vec![0u8; 32768];
    for (i, d) in datas.iter().enumerate() {
        set(&mut packed_data, i, *d & 15);
    }
    let mut level = Compound::new();
    level.insert("xPos".into(), Tag::Int(chunk.x));
    level.insert("zPos".into(), Tag::Int(chunk.z));
    level.insert("LastUpdate".into(), Tag::Long(0));
    level.insert("Blocks".into(), Tag::ByteArray(ids.iter().map(|&b| b as i8).collect()));
    level.insert("Data".into(), nibbles(&packed_data));
    level.insert("SkyLight".into(), nibbles(&sky));
    level.insert("BlockLight".into(), nibbles(&lamp));
    level.insert("HeightMap".into(), Tag::ByteArray(heightmap));
    level.insert("TerrainPopulatedFlags".into(), Tag::Short(2046));
    level.insert("Biomes".into(), Tag::ByteArray(biomes));
    level.insert("Entities".into(), list(TagType::Compound, legacy_entities));
    level.insert("TileEntities".into(), list(TagType::Compound, tiles));
    if !extended.is_empty() {
        level.insert(EXTENDED_KEY.into(), Tag::IntArray(extended));
    }
    if in_water.iter().any(|&w| w != 0) {
        // For the Version 12 writer (the older layouts have no place for it; `java_to_console` removes it).
        level.insert(IN_WATER_KEY.into(), Tag::ByteArray(in_water.iter().map(|&b| b as i8).collect()));
    }
    let mut root = Compound::new();
    root.insert("Level".into(), Tag::Compound(level));
    let _ = ctx.platform;
    Some(NamedTag { name: String::new(), tag: Tag::Compound(root) })
}

/// Private `Level` key carrying, per block (console index order), whether it sits in water.
const IN_WATER_KEY: &str = mctk_console::V12_IN_WATER_KEY;
const EXTENDED_KEY: &str = mctk_console::V12_EXTENDED_KEY;

/// The data value the original converter writes for the newer console games where it differs from the table's
/// (found by comparing its saves of two Java worlds cell by cell; `state` is the state as the chunk holds it,
/// `full` with its default properties filled in):
/// * the upper half of a double plant is 10 whatever the plant;
/// * chests, furnaces and ladders facing north are 2 (the table also has 0 for them), and a torch on a wall
///   facing east is 1;
/// * a button on the floor is 5, on the ceiling 0, on a wall its facing (east 1, west 2, south 3, north 4), plus 8 when powered;
/// * leaves that the chunk holds without properties carry the decay-check bit (8), a dispenser or a
///   jack o' lantern held without properties is 0;
/// * the water in a cauldron is its level doubled.
fn current_data(state: &BlockState, full: &BlockState, id: u8, data: u8) -> u8 {
    let prop = |k: &str| full.properties.get(k).map(String::as_str);
    let side = |f: Option<&str>| match f {
        Some("east") => Some(1),
        Some("west") => Some(2),
        Some("south") => Some(3),
        Some("north") => Some(4),
        _ => None,
    };
    match id {
        175 if prop("half") == Some("upper") => 10,
        54 | 61 | 62 | 65 | 68 | 130 | 146 if prop("facing") == Some("north") => 2,
        // End rods: down 0, up 1, south 2, north 3, east 4, west 5.
        198 => match prop("facing") {
            Some("up") => 1,
            Some("south") => 2,
            Some("north") => 3,
            Some("east") => 4,
            Some("west") => 5,
            _ => 0,
        },
        // Anvils: north 0, east 1, south 2, west 3 (the damage stays in the upper bits).
        145 => {
            let f = match prop("facing") {
                Some("east") => 1,
                Some("south") => 2,
                Some("west") => 3,
                _ => 0,
            };
            (data & !3) | f
        }
        // Fence gates: facing and open from the table, 8 when in a wall (not when powered).
        107 | 183..=187 => (data & 7) | if prop("in_wall") == Some("true") { 8 } else { 0 },
        // The stems that grow a fruit sideways are data 7 whichever way they face.
        104 | 105 if full.name.contains("attached_") => 7,
        // Pistons' heads: facing (down 0, up 1, north 2, south 3, west 4, east 5), 8 when sticky.
        34 => {
            let f = match prop("facing") {
                Some("up") => 1,
                Some("north") => 2,
                Some("south") => 3,
                Some("west") => 4,
                Some("east") => 5,
                _ => 0,
            };
            f | if prop("type") == Some("sticky") { 8 } else { 0 }
        }
        // The Java stone slab is the console's plain one (the table has it as the cobblestone one too).
        43 | 44 if full.name == "minecraft:stone_slab" => data & 8,
        26 if prop("occupied") == Some("true") => data | 4,
        50 | 75 | 76 if full.name.contains("wall_torch") => side(prop("facing")).unwrap_or(data),
        77 | 143 => {
            let base = match prop("face") {
                Some("floor") => 5,
                Some("ceiling") => 0,
                _ => side(prop("facing")).unwrap_or(data & 7),
            };
            base | if prop("powered") == Some("true") { 8 } else { 0 }
        }
        18 | 161 if state.properties.is_empty() || prop("distance") == Some("7") => data | 8,
        23 | 91 if state.properties.is_empty() => 0,
        118 if full.name.ends_with("water_cauldron") => prop("level").and_then(|l| l.parse::<u8>().ok()).map_or(data, |l| l * 2),
        _ => data,
    }
}

/// Flowing water and lava (8, 10) as the still blocks the original converter writes (9, 11) for every level.
fn still(id: u8) -> u8 {
    match id {
        8 => 9,
        10 => 11,
        other => other,
    }
}

/// Value (id << 4 | data) of a plain pumpkin: the console's id 257, which is no one-byte block.
const PUMPKIN: u16 = 0x1010;

/// The blocks the console games hold under an id above 255, as the original converter writes them
/// (read off a save it made from a Java world, cell by cell): a plain pumpkin is id 257, and the water
/// plants and the bubble column sit in water, so they carry bit 15 as well (seagrass 270, tall seagrass 270
/// with data 1 for the upper half and 2 for the lower, bubble column 272).
#[allow(clippy::precedence)] // id << 4 | data reads as the value's layout
fn extended_value(state: &BlockState) -> Option<u16> {
    let prop = |k: &str| state.properties.get(k).map(String::as_str);
    let value = match state.name.as_str() {
        "minecraft:pumpkin" => PUMPKIN,
        "minecraft:seagrass" => 0x90E0,
        "minecraft:tall_seagrass" => {
            if prop("half") == Some("upper") {
                0x90E1
            } else {
                0x90E2
            }
        }
        "minecraft:bubble_column" => 0x9100,
        // Kelp: the plant is id 258 data 0 and the head is id 258 with its age as data (to 15), in water.
        "minecraft:kelp_plant" => 0x9020,
        "minecraft:kelp" => 0x9020 | prop("age").and_then(|a| a.parse::<u16>().ok()).unwrap_or(0).min(15),
        "minecraft:dried_kelp_block" => 0x10D0,
        "minecraft:conduit" => 0x1000,
        "minecraft:blue_ice" => 0x1110,
        "minecraft:sea_pickle" => 0x10F0 | prop("pickles").and_then(|n| n.parse::<u16>().ok()).unwrap_or(1).clamp(1, 4) - 1,
        // Eggs - 1 in the low two bits, the hatch stage (0 to 2) above them.
        "minecraft:turtle_egg" => {
            let n = |k: &str, d: u16| prop(k).and_then(|n| n.parse::<u16>().ok()).unwrap_or(d);
            0x1170 | (n("eggs", 1).clamp(1, 4) - 1) | (n("hatch", 0).min(2) << 2)
        }
        name => extended_family(name.trim_start_matches("minecraft:"), state)?,
    };
    // A block that is in water carries bit 15 (the plants and the bubble column already do).
    Some(if prop("waterlogged") == Some("true") { value | 0x8000 } else { value })
}

/// Biome ids of the newer console games. The original converter's ocean variants (read off a save it made from
/// a world with every biome) are warm 40, lukewarm 42, deep lukewarm 43, cold 44, deep cold 45, frozen 46 and deep
/// frozen 47; every biome the games lack it writes as 0 (ocean). Here those get the nearest biome the games have.
fn current_biome(name: &str) -> Option<u8> {
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    let fixed = match n {
        "warm_ocean" => 40,
        "lukewarm_ocean" => 42,
        "deep_lukewarm_ocean" => 43,
        "cold_ocean" => 44,
        "deep_cold_ocean" => 45,
        "frozen_ocean" => 46,
        "deep_frozen_ocean" => 47,
        "bamboo_jungle" => 21,
        "basalt_deltas" | "crimson_forest" | "warped_forest" | "soul_sand_valley" => 8,
        "end_barrens" | "end_highlands" | "end_midlands" | "small_end_islands" => 9,
        "frozen_peaks" | "jagged_peaks" | "snowy_slopes" => 13,
        "grove" => 30,
        "stony_peaks" => 3,
        "mangrove_swamp" => 6,
        "pale_garden" => 29,
        "dappled_forest" => 4,
        "cherry_grove" | "meadow" | "dripstone_caves" | "lush_caves" | "deep_dark" | "sulfur_caves" => 1,
        _ => return LegacyTables::get().console_biome(name),
    };
    Some(fixed)
}

/// The families of blocks the original converter writes under ids above 255 (read off a save it made from a
/// probe world holding every block state): the wooden buttons, pressure plates and trapdoors of the woods the
/// 1.8-era game lacks, the stripped logs, the corals, the prismarine stairs and slabs.
#[allow(clippy::precedence)] // id << 4 | data reads as the value's layout
fn extended_family(name: &str, state: &BlockState) -> Option<u16> {
    let prop = |k: &str| state.properties.get(k).map(String::as_str);
    let facing4 = |west: u16, east: u16, north: u16, south: u16| match prop("facing") {
        Some("west") => west,
        Some("east") => east,
        Some("south") => south,
        _ => north,
    };
    let wood = |w: &str, ids: [u16; 5]| ["spruce", "birch", "jungle", "acacia", "dark_oak"].iter().position(|x| *x == w).map(|i| ids[i]);
    let on = |k: &str| u16::from(prop(k) == Some("true"));
    if let Some(w) = name.strip_suffix("_button") {
        let base = wood(w, [310, 307, 309, 306, 308])?;
        let face = match prop("face") {
            Some("floor") => 5,
            Some("ceiling") => 0,
            _ => facing4(2, 1, 4, 3),
        };
        return Some(base << 4 | face | on("powered") * 8);
    }
    if let Some(w) = name.strip_suffix("_pressure_plate") {
        return Some(wood(w, [305, 302, 304, 301, 303])? << 4 | on("powered"));
    }
    if let Some(w) = name.strip_suffix("_trapdoor") {
        // facing north 0, south 1, west 2, east 3, 4 when open, 8 on the top half
        let facing = facing4(2, 3, 0, 1);
        let top = if prop("half") == Some("top") { 8 } else { 0 };
        return Some(wood(w, [274, 275, 276, 277, 278])? << 4 | facing | on("open") * 4 | top);
    }
    if let Some(w) = name.strip_prefix("stripped_").and_then(|n| n.strip_suffix("_log")) {
        let id = if w == "oak" { 300 } else { wood(w, [295, 296, 297, 298, 299])? };
        let axis = match prop("axis") {
            Some("x") => 4,
            Some("z") => 8,
            _ => 0,
        };
        return Some(id << 4 | axis);
    }
    // Prismarine: stairs 291 (plain), 292 (bricks), 293 (dark) with the usual stair data; slabs 311 (double) and
    // 312 (half, 8 for the top) with the kind as data.
    let kind = |n: &str| match n {
        "prismarine" => Some(0u16),
        "prismarine_brick" => Some(1),
        "dark_prismarine" => Some(2),
        _ => None,
    };
    if let Some(k) = name.strip_suffix("_stairs").and_then(kind) {
        let dir = facing4(1, 0, 3, 2);
        return Some((291 + k) << 4 | dir | if prop("half") == Some("top") { 4 } else { 0 });
    }
    if let Some(k) = name.strip_suffix("_slab").and_then(kind) {
        let low = match prop("type") {
            Some("double") => 311 << 4 | k,
            Some("top") => 312 << 4 | 8 | k,
            _ => 312 << 4 | k,
        };
        return Some(low);
    }
    // Corals: kind (tube 0, brain 1, bubble 2, fire 3, horn 4) and dead (8 for the plants, in the low bits for the
    // others). The dead tube coral block is written with data 1 by the original converter, which collides with the
    // brain coral block; this writes the data 8 the other dead blocks follow.
    let (dead, rest) = match name.strip_prefix("dead_") {
        Some(r) => (true, r),
        None => (false, name),
    };
    let (kind_name, shape) = ["tube", "brain", "bubble", "fire", "horn"]
        .iter()
        .enumerate()
        .find_map(|(i, k)| rest.strip_prefix(k)?.strip_prefix("_coral").map(|s| (i as u16, s)))?;
    let d = u16::from(dead);
    let value = match shape {
        "" => 263 << 4 | kind_name | d * 8,
        "_block" => 259 << 4 | kind_name | d * 8,
        "_fan" => (264 + d) << 4,
        "_wall_fan" => {
            let id = 266 + kind_name / 2;
            id << 4 | facing4(0, 4, 8, 12) | (kind_name % 2) * 2 | d
        }
        _ => return None,
    };
    // Corals are plants of the water: without the property they are waterlogged, and the blocks sit in it too.
    let wet = match shape {
        "_block" => false,
        _ => prop("waterlogged") != Some("false"),
    };
    Some(if wet { value | 0x8000 } else { value })
}

fn region_name(dim: i32, rx: i32, rz: i32) -> String {
    match dim {
        -1 => format!("DIM-1r.{rx}.{rz}.mcr"),
        1 => format!("DIM1/r.{rx}.{rz}.mcr"),
        _ => format!("r.{rx}.{rz}.mcr"),
    }
}

/// Half the width of a dimension in a world of size class (`size`, `scale`) when the world is clipped to it. The
/// Nether is `scale` times smaller. The End is 18 chunks across (-9..8): its four regions hold 81 chunks
/// each in the genuine Wii U and Xbox 360 saves measured.
fn classic_half(dimension: i32, size: i32, scale: i32) -> i32 {
    match dimension {
        0 => size / 2,
        -1 => size / scale / 2,
        _ => END_HALF,
    }
}

/// Whether a source chunk (`cx`, `cz`) of a Java dimension (0, -1 Nether, 1 End) is in the classic 54-chunk
/// console world once the output is moved by `offset` chunks: the chunks a console conversion keeps.
pub fn classic_keeps(dimension: i32, cx: i32, cz: i32, offset: (i32, i32)) -> bool {
    let half = classic_half(dimension, WORLD_SIZES[0].0, WORLD_SIZES[0].1);
    let (x, z) = (cx + offset.0, cz + offset.1);
    (-half..half).contains(&x) && (-half..half).contains(&z)
}

/// Half the width of the console End in chunks (it is 18 across).
const END_HALF: i32 = 9;

/// The largest world a console game has: 320 chunks across.
const HALF_SIZE: i32 = 160;

/// The world size classes of the console games: chunks across (`XZSize`) and the Nether's scale against
/// the overworld (`HellScale`). The classic size and a scale of 3 were measured on a genuine Wii U save
/// and in the original converter's output (54 / 3), 320 / 8 is the largest world; the 64 and 192 classes
/// and their scales (3 and 6) follow the games' published sizes and have not been checked against a save.
const WORLD_SIZES: [(i32, i32); 4] = [(54, 3), (64, 3), (192, 6), (320, 8)];

/// The smallest size class (`XZSize`, `HellScale`) that holds what was converted. `need` is, per dimension
/// (0 overworld, -1 Nether), the half-width in chunks the converted chunks reach: a chunk `c` needs
/// `max(-c, c + 1)`. The overworld must fit `size / 2` and the Nether `size / scale / 2`. Nothing fits
/// beyond the largest class, which is returned then.
fn world_size_class(need: &HashMap<i32, i32>) -> (i32, i32) {
    let get = |d: i32| need.get(&d).copied().unwrap_or(0);
    WORLD_SIZES
        .into_iter()
        .find(|&(size, scale)| get(0) <= size / 2 && get(-1) <= size / scale / 2)
        .unwrap_or(WORLD_SIZES[WORLD_SIZES.len() - 1])
}

/// How the `level.dat` of a [`ConsoleFormat::Current`] save differs from the legacy one.
struct CurrentLevel {
    hell_scale: i32,
    difficulty: i8,
}

fn level_dat(level: &LevelData, size: i32, current: Option<CurrentLevel>) -> Result<Vec<u8>, Error> {
    let mut d = Compound::new();
    d.insert("RandomSeed".into(), Tag::Long(level.seed));
    d.insert("generatorName".into(), s("default"));
    d.insert("generatorVersion".into(), Tag::Int(1));
    d.insert("GameType".into(), Tag::Int(level.game_type));
    d.insert("MapFeatures".into(), Tag::Byte(i8::from(level.generate_structures)));
    d.insert("spawnBonusChest".into(), Tag::Byte(0));
    for (k, v) in ["SpawnX", "SpawnY", "SpawnZ"].iter().zip(level.spawn) {
        d.insert((*k).into(), Tag::Int(v));
    }
    d.insert("Time".into(), Tag::Long(level.day_time.max(0)));
    d.insert("SizeOnDisk".into(), Tag::Long(0));
    d.insert("LastPlayed".into(), Tag::Long(level.last_played));
    d.insert("LevelName".into(), s(&level.name));
    // 19132 is the McRegion storage version the console games use (19133 would be Anvil).
    d.insert("version".into(), Tag::Int(19132));
    d.insert("rainTime".into(), Tag::Int(level.rain_time));
    d.insert("raining".into(), Tag::Byte(i8::from(level.raining)));
    d.insert("thunderTime".into(), Tag::Int(level.thunder_time));
    d.insert("thundering".into(), Tag::Byte(i8::from(level.thundering)));
    d.insert("hardcore".into(), Tag::Byte(i8::from(level.hardcore)));
    d.insert("allowCommands".into(), Tag::Byte(i8::from(level.allow_commands)));
    d.insert("initialized".into(), Tag::Byte(1));
    d.insert("newSeaLevel".into(), Tag::Byte(1));
    d.insert("hasBeenInCreative".into(), Tag::Byte(i8::from(level.game_type == 1)));
    d.insert("hasStronghold".into(), Tag::Byte(0));
    d.insert("StrongholdX".into(), Tag::Int(0));
    d.insert("StrongholdY".into(), Tag::Int(0));
    d.insert("StrongholdZ".into(), Tag::Int(0));
    d.insert("hasStrongholdEndPortal".into(), Tag::Byte(0));
    d.insert("StrongholdEndPortalX".into(), Tag::Int(0));
    d.insert("StrongholdEndPortalZ".into(), Tag::Int(0));
    d.insert("XZSize".into(), Tag::Int(size));
    d.insert("HellScale".into(), Tag::Int(current.as_ref().map_or(8, |c| c.hell_scale)));
    if let Some(c) = &current {
        // What the current game versions' own level.dat holds beyond the legacy fields (DataVersion 922
        // is the value of both title updates the original converter targets).
        d.insert("DataVersion".into(), Tag::Int(922));
        d.insert("ModernEnd".into(), Tag::Byte(1));
        d.insert("Difficulty".into(), Tag::Byte(c.difficulty));
        d.insert("DifficultyLocked".into(), Tag::Byte(i8::from(level.difficulty_locked)));
        d.insert("Time".into(), Tag::Long(level.time.max(0)));
        d.insert("DayTime".into(), Tag::Long(level.day_time.max(0)));
        d.insert("clearWeatherTime".into(), Tag::Int(level.clear_weather_time));
        d.insert("BiomeCentreXChunk".into(), Tag::Int(0));
        d.insert("BiomeCentreZChunk".into(), Tag::Int(0));
        d.insert("BiomeScale".into(), Tag::Int(0));
    }
    let root = NamedTag { name: String::new(), tag: Tag::Compound(Compound::from([("Data".to_string(), Tag::Compound(d))])) };
    // The archive holds level.dat as plain NBT, not gzip as a Java world does.
    Ok(mctk_nbt::to_bytes(&root, mctk_nbt::Flavor::Java)?)
}

/// Converts the 26.x Java world `src` into the console save file `dst` for `platform`.
pub fn java_to_console(
    src: &Path,
    dst: &Path,
    platform: Platform,
    map: &ReplacementMap,
    options: &ConvertOptions,
    progress: &Progress,
) -> Result<ConsoleReport, Error> {
    let java = mctk_registry::builtin::java(mctk_registry::builtin::java_versions()[0])
        .ok_or_else(|| Error::Invalid("no Java registry".into()))?;
    let classic = mctk_registry::builtin::java("1.20.1").ok_or_else(|| Error::Invalid("no 1.20.1 registry".into()))?;
    let current = options.console_format == ConsoleFormat::Current;
    let registry = legacy_registry_in(&java, current);
    let renames = Renames::java_blocks();
    let ctx = Ctx { map, registry: &registry, renames: &renames, java: &java, classic: &classic, platform, newer: current };
    let codec = platform.chunk_codec();
    let little = platform == Platform::Windows64;
    let classic = (current && options.classic_world_size).then_some(WORLD_SIZES[0]);
    // How far the converted chunks reach in each dimension, to pick the world size class.
    let mut need: HashMap<i32, i32> = HashMap::new();
    let world = JavaWorld::open(src)?;
    let level = read_level(src)?;
    let mut report = ConsoleReport::default();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    let offset = options.offset;

    let mut files: Vec<ArchiveEntry> = Vec::new();
    for dim in &world.dimensions {
        let number = match dim.id.as_str() {
            "minecraft:the_nether" => -1,
            "minecraft:the_end" => 1,
            "minecraft:overworld" => 0,
            _ => continue,
        };
        if !options.includes(number) {
            continue;
        }
        progress.set_dimension(&dim.id);
        // Entities live in their own region files; they are read by chunk.
        let mut entity_chunks: HashMap<(i32, i32), Vec<Tag>> = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(dim.entities_dir()) {
            for entry in rd.flatten() {
                let Some(rpos) = entry.file_name().to_str().and_then(mctk_anvil::RegionPos::from_file_name) else { continue };
                let region = Region::open(entry.path())?;
                for (lx, lz) in region.chunks() {
                    if let Some(raw) = region.raw_chunk(lx, lz)? {
                        if let Some(Tag::List(l)) = raw.nbt()?.tag.get("Entities") {
                            let pos = rpos.chunk(lx, lz);
                            entity_chunks.insert((pos.x, pos.z), l.iter().cloned().collect());
                        }
                    }
                }
            }
        }
        let regions = dim.regions()?;
        progress.regions_total.fetch_add(regions.len(), Ordering::Relaxed);
        let mut builders: HashMap<(i32, i32), ConsoleRegionBuilder> = HashMap::new();
        for rpos in regions {
            let region = Region::open(dim.region_dir().join(rpos.file_name()))?;
            let coords: Vec<(usize, usize)> = region.chunks().collect();
            progress.chunks_total.fetch_add(coords.len(), Ordering::Relaxed);
            // What the chunks of the region need: read here (the disk), converted on every core.
            let mut raws: Vec<(i32, i32, mctk_anvil::RawChunk)> = Vec::with_capacity(coords.len());
            for (lx, lz) in coords {
                if progress.is_cancelled() {
                    return Err(Error::Invalid("conversion stopped".into()));
                }
                let at = rpos.chunk(lx, lz);
                let to = ChunkPos::new(at.x + offset.0, at.z + offset.1);
                // A chunk outside the world is left out before it is even read.
                let outside = !(-HALF_SIZE..HALF_SIZE).contains(&to.x)
                    || !(-HALF_SIZE..HALF_SIZE).contains(&to.z)
                    || classic.is_some_and(|(size, scale)| {
                        let half = classic_half(number, size, scale);
                        !(-half..half).contains(&to.x) || !(-half..half).contains(&to.z)
                    });
                let Some(raw) = region.raw_chunk(lx, lz)? else { continue };
                if outside {
                    report.outside += 1;
                    continue;
                }
                raws.push((at.x, at.z, raw));
            }
            let work = |raw: &mctk_anvil::RawChunk| -> Result<Chunked, Error> {
                let mut report = ConsoleReport::default();
                let tree = raw.nbt()?;
                if options.skip_partial_chunks && !crate::options::is_complete(crate::java_bedrock::java_status(&tree).as_deref()) {
                    report.partial += 1;
                    return Ok(Chunked::Skip(report));
                }
                let Ok(chunk) = Chunk::from_nbt(tree) else { return Ok(Chunked::Skip(report)) };
                let pos = ChunkPos::new(chunk.x + offset.0, chunk.z + offset.1);
                let entities = entity_chunks.get(&(chunk.x, chunk.z)).cloned().filter(|_| options.entities).unwrap_or_default();
                let mut chunk = chunk;
                let raised = if crate::depth::applies(&chunk) { crate::depth::fix(&mut chunk, options.depth) } else { 0 };
                let Some(mut tree) = chunk_to_console(&chunk, &entities, number, raised, &ctx, &mut report) else { return Ok(Chunked::Skip(report)) };
                if !options.tile_entities {
                    if let Some(level) = tree.tag.as_compound_mut().and_then(|c| c.get_mut("Level")).and_then(Tag::as_compound_mut) {
                        level.insert("TileEntities".into(), list(TagType::Compound, Vec::new()));
                    }
                }
                if offset != (0, 0) {
                    shift_chunk(&mut tree, offset);
                }
                if !current {
                    if let Some(level) = tree.tag.as_compound_mut().and_then(|c| c.get_mut("Level")).and_then(Tag::as_compound_mut) {
                        level.shift_remove(IN_WATER_KEY);
                        level.shift_remove(EXTENDED_KEY);
                    }
                }
                let raw = if current {
                    RawConsoleChunk::encode_v12(&tree, codec, timestamp)
                } else {
                    RawConsoleChunk::encode_compressed(&tree, 8, codec, timestamp)
                }
                .map_err(Error::from)?;
                report.chunks += 1;
                progress.chunk(pos.x, pos.z);
                Ok(Chunked::Done(pos, raw, report))
            };
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(raws.len().max(1));
            let next = std::sync::atomic::AtomicUsize::new(0);
            let failed = std::sync::atomic::AtomicBool::new(false);
            let mut finished: Vec<(usize, Result<Chunked, Error>)> = Vec::with_capacity(raws.len());
            std::thread::scope(|s| {
                let workers: Vec<_> = (0..threads)
                    .map(|_| {
                        s.spawn(|| {
                            let mut mine = Vec::new();
                            loop {
                                let i = next.fetch_add(1, Ordering::Relaxed);
                                let Some((_, _, raw)) = raws.get(i) else { break };
                                if failed.load(Ordering::Relaxed) {
                                    break;
                                }
                                let out = if progress.is_cancelled() { Err(Error::Invalid("conversion stopped".into())) } else { work(raw) };
                                if out.is_err() {
                                    failed.store(true, Ordering::Relaxed);
                                }
                                mine.push((i, out));
                            }
                            mine
                        })
                    })
                    .collect();
                for w in workers {
                    finished.extend(w.join().expect("a conversion thread"));
                }
            });
            // In the order the chunks were read, so the result does not depend on which core was quicker.
            finished.sort_by_key(|(i, _)| *i);
            for (_, out) in finished {
                let (pos, raw, r) = match out? {
                    Chunked::Skip(r) => {
                        report.merge(r);
                        continue;
                    }
                    Chunked::Done(pos, raw, r) => (pos, raw, r),
                };
                report.merge(r);
                let reach = [-pos.x, pos.x + 1, -pos.z, pos.z + 1].into_iter().max().unwrap_or(0);
                let slot = need.entry(number).or_insert(0);
                *slot = (*slot).max(reach);
                let (rx, rz) = (pos.x.div_euclid(32), pos.z.div_euclid(32));
                let builder = builders.entry((rx, rz)).or_insert_with(|| {
                    let mut b = ConsoleRegionBuilder::default();
                    b.little = little;
                    b
                });
                builder.set(pos.x.rem_euclid(32) as usize, pos.z.rem_euclid(32) as usize, Some(raw));
            }
            progress.regions_done.fetch_add(1, Ordering::Relaxed);
        }
        for ((rx, rz), b) in builders {
            files.push(ArchiveEntry::new(region_name(number, rx, rz), b.build().map_err(Error::from)?, 0));
        }
    }
    // The game opens the four regions around the origin of every dimension whether or not they
    // hold chunks; its own saves list them empty.
    for dim in [0, -1, 1] {
        for (rx, rz) in [(-1, -1), (0, -1), (0, 0), (-1, 0)] {
            let name = region_name(dim, rx, rz);
            if !files.iter().any(|f| f.name == name) {
                files.push(ArchiveEntry::new(name, Vec::new(), 0));
            }
        }
    }
    let mut level = level;
    if let Some(g) = options.game_mode {
        level.game_type = g;
    }
    level.spawn[0] += offset.0 * 16;
    level.spawn[2] += offset.1 * 16;
    let (level_bytes, archive_version) = if current {
        let (size, hell_scale) = world_size_class(&need);
        let difficulty = options.difficulty.map_or(level.difficulty, |d| d.clamp(0, 3) as i8);
        (level_dat(&level, size, Some(CurrentLevel { hell_scale, difficulty }))?, 10)
    } else {
        (level_dat(&level, HALF_SIZE * 2, None)?, 8)
    };
    files.push(ArchiveEntry::new("level.dat", level_bytes, 0));

    let archive = SaveArchive::new(archive_version, files, little);
    // Legacy keeps the raw archive, readable by older game versions; only the current layout compresses a
    // PS3 GAMEDATA (newer PS3 game versions read both).
    let save = ConsoleSave {
        platform,
        archive,
        source: dst.to_path_buf(),
        gamedata_compressed: current && platform == Platform::Ps3,
    };
    std::fs::write(dst, save.encode().map_err(Error::from)?)?;
    Ok(report)
}

fn shift_chunk(tree: &mut NamedTag, offset: (i32, i32)) {
    let Some(level) = tree.tag.as_compound_mut().and_then(|c| c.get_mut("Level")).and_then(Tag::as_compound_mut) else { return };
    let (dx, dz) = (offset.0 * 16, offset.1 * 16);
    for key in ["TileEntities", "Entities"] {
        if let Some(Tag::List(l)) = level.get_mut(key) {
            for t in l.iter_mut() {
                let Some(c) = t.as_compound_mut() else { continue };
                for (k, by) in [("x", dx), ("z", dz), ("TileX", dx), ("TileZ", dz)] {
                    if let Some(v) = num(c, k) {
                        c.insert(k.into(), Tag::Int(v as i32 + by));
                    }
                }
                if let Some(Tag::List(p)) = c.get("Pos").cloned() {
                    let mut v: Vec<f64> = p.iter().filter_map(Tag::as_f64).collect();
                    if v.len() == 3 {
                        v[0] += dx as f64;
                        v[2] += dz as f64;
                        c.insert("Pos".into(), list(TagType::Double, v.into_iter().map(Tag::Double).collect()));
                    }
                }
            }
        }
    }
}

fn read_level(src: &Path) -> Result<LevelData, Error> {
    if src.join("data/minecraft/world_gen_settings.dat").is_file() {
        LevelData::read_modern(src)
    } else {
        let (_, root) = mctk_nbt::read_file(src.join("level.dat"), mctk_nbt::Flavor::Java)?;
        LevelData::read_classic(&NamedTag { name: String::new(), tag: root.tag })
    }
}

#[allow(dead_code)]
fn unused(_: &Tag) {
    let _ = item_down;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reach(entries: &[(i32, i32)]) -> HashMap<i32, i32> {
        entries.iter().copied().collect()
    }

    #[test]
    fn blocks_only_the_newer_games_have_get_the_ids_the_original_converter_writes() {
        let tall = |half: &str| {
            let mut s = BlockState::new("minecraft:tall_seagrass");
            s.properties.insert("half".into(), half.into());
            s
        };
        assert_eq!(extended_value(&BlockState::new("minecraft:pumpkin")), Some(0x1010));
        assert_eq!(extended_value(&BlockState::new("minecraft:seagrass")), Some(0x90E0));
        assert_eq!(extended_value(&tall("upper")), Some(0x90E1));
        assert_eq!(extended_value(&tall("lower")), Some(0x90E2));
        assert_eq!(extended_value(&BlockState::new("minecraft:bubble_column")), Some(0x9100));
        assert_eq!(extended_value(&BlockState::new("minecraft:stone")), None);
        assert_eq!((still(8), still(10), still(9), still(1)), (9, 11, 9, 1));
        let t = LegacyTables::get();
        let magma = BlockState::new("minecraft:magma_block");
        assert_eq!(t.console_block_in(&magma, true), Some((213, 0)));
        assert_eq!(t.console_block(&magma), None, "the legacy format keeps to the ids every game build has");
        let white_concrete = BlockState::new("minecraft:white_concrete");
        assert_eq!(t.console_block_in(&white_concrete, true), Some((251, 0)));
    }

    #[test]
    fn kelp_and_the_spruce_trapdoor_get_the_ids_the_original_converter_writes() {
        let st = |name: &str, props: &[(&str, &str)]| {
            let mut s = BlockState::new(name);
            for (k, v) in props {
                s.properties.insert((*k).into(), (*v).into());
            }
            s
        };
        assert_eq!(extended_value(&st("minecraft:kelp_plant", &[])), Some(0x9020));
        assert_eq!(extended_value(&st("minecraft:kelp", &[("age", "3")])), Some(0x9023));
        assert_eq!(extended_value(&st("minecraft:kelp", &[("age", "22")])), Some(0x902F));
        assert_eq!(extended_value(&st("minecraft:dried_kelp_block", &[])), Some(0x10D0));
        assert_eq!(extended_value(&st("minecraft:spruce_trapdoor", &[])), Some(0x1120));
        let open_top_east = st("minecraft:spruce_trapdoor", &[("half", "top"), ("facing", "east"), ("open", "true")]);
        assert_eq!(extended_value(&open_top_east), Some(0x112F));
        let wet = st("minecraft:spruce_trapdoor", &[("waterlogged", "true"), ("facing", "south")]);
        assert_eq!(extended_value(&wet), Some(0x9121));
    }

    /// Values read off the original converter's save of a world holding every block state.
    #[test]
    fn the_blocks_above_255_match_the_original_converters_values() {
        let st = |name: &str, props: &[(&str, &str)]| {
            let mut s = BlockState::new(name);
            for (k, v) in props {
                s.properties.insert((*k).into(), (*v).into());
            }
            s
        };
        let v = |name: &str, props: &[(&str, &str)]| extended_value(&st(name, props));
        assert_eq!(v("minecraft:acacia_button", &[("face", "wall"), ("facing", "south")]), Some(0x1323));
        assert_eq!(v("minecraft:acacia_button", &[("face", "wall"), ("facing", "north"), ("powered", "true")]), Some(0x132c));
        assert_eq!(v("minecraft:acacia_button", &[("face", "floor")]), Some(0x1325));
        assert_eq!(v("minecraft:birch_pressure_plate", &[("powered", "true")]), Some(0x12e1));
        assert_eq!(v("minecraft:birch_trapdoor", &[("facing", "west"), ("open", "true"), ("half", "top")]), Some(0x113e));
        assert_eq!(v("minecraft:birch_trapdoor", &[("waterlogged", "true")]), Some(0x9130));
        assert_eq!(v("minecraft:stripped_oak_log", &[("axis", "z")]), Some(0x12c8));
        assert_eq!(v("minecraft:stripped_spruce_log", &[("axis", "x")]), Some(0x1274));
        assert_eq!(v("minecraft:stripped_oak_wood", &[]), None);
        assert_eq!(v("minecraft:prismarine_stairs", &[("facing", "south"), ("half", "top")]), Some(0x1236));
        assert_eq!(v("minecraft:dark_prismarine_slab", &[("type", "double")]), Some(0x1372));
        assert_eq!(v("minecraft:prismarine_brick_slab", &[("type", "top")]), Some(0x1389));
        assert_eq!(v("minecraft:prismarine_slab", &[("type", "bottom"), ("waterlogged", "true")]), Some(0x9380));
        assert_eq!(v("minecraft:tube_coral", &[("waterlogged", "true")]), Some(0x9070));
        assert_eq!(v("minecraft:dead_horn_coral", &[("waterlogged", "false")]), Some(0x107c));
        assert_eq!(v("minecraft:fire_coral_block", &[]), Some(0x1033));
        assert_eq!(v("minecraft:dead_tube_coral_fan", &[("waterlogged", "false")]), Some(0x1090));
        assert_eq!(v("minecraft:brain_coral_wall_fan", &[("facing", "north"), ("waterlogged", "false")]), Some(0x10aa));
        assert_eq!(v("minecraft:dead_horn_coral_wall_fan", &[("facing", "east"), ("waterlogged", "true")]), Some(0x90c5));
        assert_eq!(v("minecraft:bubble_coral_wall_fan", &[("facing", "south"), ("waterlogged", "false")]), Some(0x10bc));
        assert_eq!(v("minecraft:turtle_egg", &[("eggs", "1"), ("hatch", "2")]), Some(0x1178));
        assert_eq!(v("minecraft:turtle_egg", &[("eggs", "4"), ("hatch", "0")]), Some(0x1173));
        assert_eq!(v("minecraft:sea_pickle", &[("pickles", "3"), ("waterlogged", "true")]), Some(0x90f2));
        assert_eq!(v("minecraft:conduit", &[("waterlogged", "false")]), Some(0x1000));
        assert_eq!(v("minecraft:blue_ice", &[]), Some(0x1110));
    }

    #[test]
    fn the_data_the_original_converter_writes_where_the_table_differs() {
        let st = |name: &str, props: &[(&str, &str)]| {
            let mut s = BlockState::new(name);
            for (k, v) in props {
                s.properties.insert((*k).into(), (*v).into());
            }
            s
        };
        let d = |s: &BlockState, id: u8, data: u8| current_data(s, s, id, data);
        assert_eq!(d(&st("minecraft:chest", &[("facing", "north")]), 54, 0), 2);
        assert_eq!(d(&st("minecraft:ladder", &[("facing", "south")]), 65, 3), 3);
        assert_eq!(d(&st("minecraft:wall_torch", &[("facing", "east")]), 50, 0), 1);
        assert_eq!(d(&st("minecraft:oak_button", &[("face", "floor"), ("facing", "west")]), 143, 2), 5);
        assert_eq!(d(&st("minecraft:oak_button", &[("face", "wall"), ("facing", "north"), ("powered", "true")]), 143, 0), 12);
        assert_eq!(d(&st("minecraft:tall_grass", &[("half", "upper")]), 175, 2), 10);
        assert_eq!(d(&st("minecraft:water_cauldron", &[("level", "3")]), 118, 3), 6);
        // held without properties
        let bare = |name: &str| BlockState::new(name);
        assert_eq!(current_data(&bare("minecraft:oak_leaves"), &bare("minecraft:oak_leaves"), 18, 0), 8);
        assert_eq!(current_data(&bare("minecraft:spruce_leaves"), &bare("minecraft:spruce_leaves"), 18, 1), 9);
        assert_eq!(current_data(&bare("minecraft:dispenser"), &bare("minecraft:dispenser"), 23, 2), 0);
        assert_eq!(current_data(&bare("minecraft:jack_o_lantern"), &bare("minecraft:jack_o_lantern"), 91, 2), 0);
    }

    #[test]
    fn basic_blocks_the_console_has_are_not_offered_for_replacing() {
        let java = mctk_registry::builtin::java(mctk_registry::builtin::java_versions()[0]).unwrap();
        let state = |name: &str, props: &[(&str, &str)]| {
            let mut s = BlockState::new(name);
            for (k, v) in props {
                s.properties.insert((*k).into(), (*v).into());
            }
            s
        };
        for s in [
            state("minecraft:stone_slab", &[]),
            state("minecraft:stone_slab", &[("type", "top"), ("waterlogged", "true")]),
            state("minecraft:stone_slab", &[("type", "double"), ("waterlogged", "false")]),
            state("minecraft:cave_air", &[]),
            state("minecraft:vine", &[("east", "true"), ("north", "false"), ("south", "false"), ("up", "false"), ("west", "false")]),
            state("minecraft:chest", &[("facing", "north"), ("type", "single"), ("waterlogged", "false")]),
            state("minecraft:pumpkin", &[]),
            state("minecraft:flower_pot", &[]),
        ] {
            assert!(console_supports(&java, &s, true), "{s}");
        }
        assert!(!console_supports(&java, &state("minecraft:deepslate", &[("axis", "y")]), true));
    }

    #[test]
    fn the_classic_area_is_what_the_replace_scan_keeps() {
        assert!(classic_keeps(0, -27, 26, (0, 0)) && !classic_keeps(0, 27, 0, (0, 0)));
        assert!(classic_keeps(-1, -9, 8, (0, 0)) && !classic_keeps(-1, -10, 0, (0, 0)));
        // moved by the new center: chunk 30 lands at 30 - 10 = 20
        assert!(classic_keeps(0, 30, 0, (-10, 0)));
    }

    #[test]
    fn the_classic_world_clips_each_dimension_to_its_own_width() {
        let (size, scale) = WORLD_SIZES[0];
        assert_eq!(classic_half(0, size, scale), 27); // chunks -27..26
        assert_eq!(classic_half(-1, size, scale), 9); // chunks -9..8
        assert_eq!(classic_half(1, size, scale), 9); // chunks -9..8
    }

    #[test]
    fn the_smallest_world_size_that_holds_the_chunks_is_chosen() {
        // A chunk c needs max(-c, c + 1) on each side of the origin.
        assert_eq!(world_size_class(&reach(&[])), (54, 3));
        assert_eq!(world_size_class(&reach(&[(0, 27)])), (54, 3), "chunks -27..=26 fit the classic size exactly");
        assert_eq!(world_size_class(&reach(&[(0, 28)])), (64, 3));
        assert_eq!(world_size_class(&reach(&[(0, 32)])), (64, 3), "-32..=31 is the small size");
        assert_eq!(world_size_class(&reach(&[(0, 33)])), (192, 6));
        assert_eq!(world_size_class(&reach(&[(0, 97)])), (320, 8));
        assert_eq!(world_size_class(&reach(&[(0, 5000)])), (320, 8), "nothing is larger");
        // The Nether must fit size / scale: 54 / 3 / 2 = 9 chunks each way.
        assert_eq!(world_size_class(&reach(&[(0, 10), (-1, 9)])), (54, 3));
        assert_eq!(world_size_class(&reach(&[(0, 10), (-1, 10)])), (64, 3));
        assert_eq!(world_size_class(&reach(&[(0, 10), (-1, 16)])), (192, 6), "the 192 size's Nether reaches 16");
        assert_eq!(world_size_class(&reach(&[(0, 10), (-1, 17)])), (320, 8), "17 is beyond it, so the largest size");
    }

    #[test]
    fn current_level_dat_carries_the_newer_fields() {
        let level = LevelData::default();
        let read = |bytes: Vec<u8>| {
            let root = mctk_nbt::from_bytes(&bytes, mctk_nbt::Flavor::Java).unwrap();
            root.tag.get("Data").cloned().unwrap()
        };
        let legacy = read(level_dat(&level, HALF_SIZE * 2, None).unwrap());
        assert_eq!(legacy.get("XZSize").and_then(Tag::as_i64), Some(320));
        assert_eq!(legacy.get("HellScale").and_then(Tag::as_i64), Some(8));
        assert!(legacy.get("DataVersion").is_none(), "the legacy level.dat is unchanged");

        let current = read(level_dat(&level, 54, Some(CurrentLevel { hell_scale: 3, difficulty: 2 })).unwrap());
        assert_eq!(current.get("XZSize").and_then(Tag::as_i64), Some(54));
        assert_eq!(current.get("HellScale").and_then(Tag::as_i64), Some(3));
        assert_eq!(current.get("DataVersion").and_then(Tag::as_i64), Some(922));
        assert_eq!(current.get("version").and_then(Tag::as_i64), Some(19132));
        assert_eq!(current.get("Difficulty").and_then(Tag::as_i64), Some(2));
        for key in ["ModernEnd", "DayTime", "clearWeatherTime", "BiomeScale", "BiomeCentreXChunk", "DifficultyLocked"] {
            assert!(current.get(key).is_some(), "{key}");
        }
    }

    /// Light spreads a step dimmer each block through air, not through stone, more slowly
    /// through water; sky light reaches in under an overhang.
    #[test]
    fn light_spreads_as_the_game_does() {
        let h = 128;
        let mut ids = vec![0u8; 16 * 16 * h];
        // A stone wall along x = 10.
        for y in 0..h {
            for z in 0..16 {
                ids[console_block_index(10, y, z)] = 1;
            }
        }
        let mut lamp = vec![0u8; 16 * 16 * h];
        lamp[console_block_index(5, 60, 5)] = 14;
        spread_light(&mut lamp, &ids, h);
        assert_eq!(lamp[console_block_index(6, 60, 5)], 13);
        assert_eq!(lamp[console_block_index(9, 60, 5)], 10);
        assert_eq!(lamp[console_block_index(11, 60, 5)], 0, "not through stone");
        // Through water: a step and its cost.
        ids[console_block_index(5, 61, 5)] = 9;
        let mut lamp = vec![0u8; 16 * 16 * h];
        lamp[console_block_index(5, 60, 5)] = 14;
        spread_light(&mut lamp, &ids, h);
        assert_eq!(lamp[console_block_index(5, 61, 5)], 10);
        // Sky light under an overhang: a stone roof over (0..4, 70, 0..4) with the column under
        // it dark from above, lit from the open side.
        let mut ids = vec![0u8; 16 * 16 * h];
        let mut sky = vec![0u8; 16 * 16 * h];
        for z in 0..16 {
            for x in 0..16 {
                for y in (0..h).rev() {
                    let roof = x < 4 && z < 4 && y == 70;
                    if roof {
                        ids[console_block_index(x, y, z)] = 1;
                    }
                    let covered = x < 4 && z < 4 && y <= 70;
                    sky[console_block_index(x, y, z)] = if covered { 0 } else { 15 };
                }
            }
        }
        spread_light(&mut sky, &ids, h);
        assert_eq!(sky[console_block_index(3, 69, 2)], 14, "beside the open side");
        assert_eq!(sky[console_block_index(0, 69, 0)], 11, "four blocks in");
    }

    /// A falling block and a chest minecart keep what they carry going to the console game.
    #[test]
    fn falling_blocks_and_chest_minecarts_keep_their_contents_for_the_console() {
        let mut notes = Notes::default();
        let mut sand = Compound::new();
        sand.insert("id".into(), Tag::String("minecraft:falling_block".into()));
        let mut bs = Compound::new();
        bs.insert("Name".into(), Tag::String("minecraft:sand".into()));
        sand.insert("BlockState".into(), Tag::Compound(bs));
        sand.insert("Time".into(), Tag::Int(9));
        let out = entity_to_legacy(&sand, &mut notes).expect("a falling block");
        assert_eq!(out.get("id").and_then(Tag::as_str), Some("FallingSand"));
        assert_eq!(out.get("TileID").and_then(Tag::as_i64), Some(12), "sand is block 12");
        assert_eq!(out.get("Time").and_then(Tag::as_i64), Some(9));
        let mut stack = Compound::new();
        stack.insert("id".into(), Tag::String("minecraft:diamond".into()));
        stack.insert("count".into(), Tag::Int(2));
        stack.insert("Slot".into(), Tag::Byte(1));
        let mut cart = Compound::new();
        cart.insert("id".into(), Tag::String("minecraft:chest_minecart".into()));
        cart.insert("Items".into(), Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(stack)]).unwrap()));
        let out = entity_to_legacy(&cart, &mut notes).expect("a minecart");
        assert_eq!(out.get("Items").and_then(Tag::as_list).map(|l| l.len()), Some(1));
    }

    #[test]
    fn states_go_back_to_the_ids_they_came_from() {
        let t = LegacyTables::get();
        // Every state of the console's blocks maps to an id and data whose state is the same
        // (ignoring what depends on neighbours), and for most to the very ones it came from.
        let mut exact = 0;
        let mut total = 0;
        for ((id, data), state) in t.console_states_in(false) {
            total += 1;
            let (i, d) = t.console_block(state).unwrap_or_else(|| panic!("{state} has no way back"));
            assert_eq!(canonical(t.block(i, d).unwrap()), canonical(state), "{id}:{data} {state} -> {i}:{d}");
            exact += usize::from((i, d) == (*id, *data));
        }
        // Many data values mean the same block (stone has seven blank ones), so not all return.
        assert!(exact * 3 > total, "{exact} of {total}");
        assert_eq!(t.console_block(&BlockState::new("minecraft:stone")), Some((1, 0)));
        assert_eq!(t.console_block(&BlockState::new("minecraft:red_wool")), Some((35, 14)));
        // Beyond what the console game has.
        assert_eq!(t.console_block(&BlockState::new("minecraft:crimson_stem")), None);
    }

    /// The current games' dialect, read off the original converter's save of probe worlds.
    #[test]
    fn the_current_dialect_writes_text_ids_and_the_1_12_names() {
        CURRENT_DIALECT.with(|c| c.set(true));
        let mut notes = Notes::default();
        let stack = |id: &str| Compound::from([("id".to_string(), s(id)), ("Count".to_string(), Tag::Byte(1))]);
        let id = |c: Compound| c.get("id").and_then(Tag::as_str).unwrap_or("").to_string();
        assert_eq!(id(item_to_legacy(&stack("minecraft:apple"), &mut notes).unwrap()), "minecraft:apple");
        assert_eq!(id(item_to_legacy(&stack("minecraft:acacia_button"), &mut notes).unwrap()), "306");
        assert_eq!(id(item_to_legacy(&stack("minecraft:stone"), &mut notes).unwrap()), "1");
        assert_eq!(id(item_to_legacy(&stack("minecraft:oak_leaves"), &mut notes).unwrap()), "minecraft:leaves");
        let zombie = Compound::from([("id".to_string(), s("minecraft:zombified_piglin"))]);
        assert_eq!(id(entity_to_legacy(&zombie, &mut notes).unwrap()), "minecraft:zombie_pigman");
        let bat = Compound::from([("id".to_string(), s("minecraft:bat"))]);
        let bat = entity_to_legacy(&bat, &mut notes).unwrap();
        assert_eq!(id(bat.clone()), "minecraft:bat");
        assert!(matches!(bat.get("Health"), Some(Tag::Float(_))));
        let chest = Compound::from([("id".to_string(), s("minecraft:chest")), ("x".to_string(), Tag::Int(1)), ("y".to_string(), Tag::Int(2)), ("z".to_string(), Tag::Int(3))]);
        assert_eq!(id(block_entity_to_legacy(&chest, &mut notes).unwrap()), "minecraft:chest");
        CURRENT_DIALECT.with(|c| c.set(false));
        assert_eq!(id(entity_to_legacy(&zombie, &mut notes).unwrap()), "PigZombie");
    }

    #[test]
    fn items_and_potions_go_back() {
        let mut notes = Notes::default();
        let stack = |id: &str, tag: Vec<(&str, Tag)>| {
            Compound::from([
                ("id".to_string(), s(id)),
                ("Count".to_string(), Tag::Byte(2)),
                ("tag".to_string(), Tag::Compound(tag.into_iter().map(|(k, v)| (k.to_string(), v)).collect())),
            ])
        };
        let sword = item_to_legacy(&stack("minecraft:diamond_sword", vec![("Damage", Tag::Int(7))]), &mut notes).unwrap();
        assert_eq!((num(&sword, "id"), num(&sword, "Damage")), (Some(276), Some(7)));
        let potion = item_to_legacy(&stack("minecraft:splash_potion", vec![("Potion", s("minecraft:strong_swiftness"))]), &mut notes).unwrap();
        assert_eq!(num(&potion, "Damage"), Some(16384 + 8192 + 2 + 32));
        let egg = item_to_legacy(&stack("minecraft:pig_spawn_egg", vec![]), &mut notes).unwrap();
        assert_eq!((num(&egg, "id"), num(&egg, "Damage")), (Some(383), Some(90)));
        assert!(item_to_legacy(&stack("minecraft:netherite_ingot", vec![]), &mut notes).is_none());
        assert_eq!(potion_damage("minecraft:long_swiftness", false), 8192 + 2 + 64);
    }

    #[test]
    fn the_console_registry_holds_only_its_own_blocks() {
        let java = mctk_registry::builtin::java(mctk_registry::builtin::java_versions()[0]).unwrap();
        let reg = legacy_registry(&java);
        assert!(reg.has_block("minecraft:oak_stairs") && reg.has_block("minecraft:red_wool"));
        // Later updates' blocks are held too; the 1.9+ ones (observer, shulker boxes) are not.
        assert!(reg.has_block("minecraft:hopper") && reg.has_block("minecraft:red_terracotta"));
        assert!(!reg.has_block("minecraft:observer"));
        // Stairs keep every corner shape, though the table only knows "straight".
        assert!(reg.blocks["minecraft:oak_stairs"].properties["shape"].len() > 1);
    }

    #[test]
    fn blocks_the_console_lacks_get_a_nearby_block_instead_of_air() {
        let java = mctk_registry::builtin::java(mctk_registry::builtin::java_versions()[0]).unwrap();
        let reg = legacy_registry(&java);
        let renames = mctk_registry::Renames::java_blocks();
        // Every bed colour is held (as the one bed plus a Bed tile entity).
        assert!(mctk_replace::suggest(&BlockState::new("minecraft:cyan_bed").with("facing", "north").with("part", "head"), &reg, &renames).is_none());
        for name in ["minecraft:red_terracotta", "minecraft:cherry_stairs", "minecraft:white_concrete", "minecraft:waxed_oxidized_cut_copper_stairs"] {
            let state = BlockState::new(name);
            let g = mctk_replace::suggest(&state, &reg, &renames);
            eprintln!("{name}: {g:?}");
            if let Some(g) = g {
                assert_ne!(g.action, mctk_replace::Action::Air, "{name}");
            }
        }
    }
}
