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
use crate::options::{ConvertOptions, Progress};

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
    let t = LegacyTables::get();
    let mut reg = Registry::new(Edition::Java, "Legacy console");
    let mut values: IndexMap<String, IndexMap<String, Vec<String>>> = IndexMap::new();
    for (_, state) in t.console_states() {
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
        if props.contains_key("waterlogged") || def.properties.contains_key("waterlogged") {
            props.insert("waterlogged".into(), vec!["false".into()]);
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

/// A 1.20.1 item stack as a legacy one (numeric `id`, `Damage`, `tag`), or None when the console
/// game has no such item.
pub(crate) fn item_to_legacy(it: &Compound, notes: &mut Notes) -> Option<Compound> {
    let t = LegacyTables::get();
    let name = it.get("id")?.as_str()?;
    let tag = it.get("tag").and_then(Tag::as_compound).cloned().unwrap_or_default();
    let (id, mut damage, keeps) = if matches!(name, "minecraft:potion" | "minecraft:splash_potion") {
        let potion = tag.get("Potion").and_then(Tag::as_str).unwrap_or("minecraft:water");
        (373, potion_damage(potion, name == "minecraft:splash_potion") as i16, false)
    } else if name == "minecraft:filled_map" {
        (358, num(&tag, "map").unwrap_or(0) as i16, false)
    } else {
        let Some(found) = t.console_item(name) else {
            notes.add(format!("{name} does not exist in the console game"));
            return None;
        };
        found
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
    out.insert("id".into(), Tag::Short(id));
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
    let Some((_, old)) = ENTITIES.iter().find(|(n, _)| *n == id) else {
        notes.add(format!("entity {id} does not exist in the console game"));
        return None;
    };
    let mut out = Compound::new();
    out.insert("id".into(), s(old));
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
        old.as_ref(),
        "Item" | "XPOrb" | "Painting" | "Arrow" | "Snowball" | "Fireball" | "SmallFireball" | "ThrownEnderpearl" | "EyeOfEnderSignal"
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
    match old.as_ref() {
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
    Some(out)
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
    out.insert("id".into(), s(old));
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

/// Flower pot contents: the item the pot holds to the data value the console game keeps.
fn potted(name: &str) -> Option<u8> {
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
}

fn state_to_console(state: &BlockState, ctx: &Ctx, report: &mut ConsoleReport, uses: u64) -> (u8, u8, Option<BlockState>) {
    let t = LegacyTables::get();
    let full = ctx
        .java
        .complete_state(&state.name, state.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .map_or_else(|| state.clone(), |p| BlockState { name: state.name.clone(), properties: p });
    // Beds are one block on the console; the colour goes in a Bed tile entity.
    if full.name.ends_with("_bed") && full.name != "minecraft:red_bed" {
        let mut red = full.clone();
        red.name = "minecraft:red_bed".into();
        if let Some((id, data)) = t.console_block(&red) {
            return (id, data, Some(full));
        }
    }
    // Skulls and flower pots are the console's blocks with data of their own.
    if full.name.ends_with("_skull") || full.name.ends_with("_head") {
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
    let resolved = match ctx.map.resolve(&full, ctx.registry, ctx.renames) {
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
    match t.console_block(&resolved) {
        Some((id, data)) => (id, data, None),
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
    // The console stores 256 layers in every dimension, though the Nether plays in the lower half.
    let height = 256;
    let mut ids = vec![0u8; 65536];
    let mut datas = vec![0u8; 65536];
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
        for y in 0..16usize {
            for z in 0..16usize {
                for x in 0..16usize {
                    let wy = section.y as i32 * 16 + y as i32;
                    if wy < 0 || wy >= height {
                        continue;
                    }
                    let i = p.indices[mctk_java::block_index(x, y, z)] as usize;
                    let (id, data, tile) = &converted[i];
                    if *id == 0 {
                        continue;
                    }
                    let at = console_block_index(x, wy as usize, z);
                    ids[at] = *id;
                    datas[at] = *data;
                    if let Some(st) = tile {
                        extra_tiles.insert((chunk.x * 16 + x as i32, wy, chunk.z * 16 + z as i32), st.clone());
                    }
                    let _ = &mut flower_pots;
                }
            }
        }
    }

    // Heights and a plain sky light: full above the highest opaque block, none below.
    let mut heightmap = vec![0i8; 256];
    let mut sky = vec![0u8; 32768];
    let set = |arr: &mut [u8], i: usize, v: u8| {
        if i % 2 == 0 {
            arr[i / 2] = (arr[i / 2] & 0xf0) | v;
        } else {
            arr[i / 2] = (arr[i / 2] & 0x0f) | (v << 4);
        }
    };
    for z in 0..16usize {
        for x in 0..16usize {
            let mut lit = true;
            let mut top = 0usize;
            for y in (0..height as usize).rev() {
                let at = console_block_index(x, y, z);
                let id = ids[at];
                if id != 0 && top == 0 {
                    top = y + 1;
                }
                if lit && !CLEAR.contains(&id) {
                    lit = false;
                }
                if lit && dim != -1 {
                    set(&mut sky, at, 15);
                }
            }
            heightmap[z * 16 + x] = top.min(255) as i8;
        }
    }

    // Biomes: one per column, from the cell at the surface section.
    let biome_of = |x: usize, z: usize| -> u8 {
        let section = chunk.sections.iter().find(|s| s.y == 4).or_else(|| chunk.sections.iter().find(|s| s.biomes.is_some()));
        section
            .and_then(|s| s.biomes.as_ref())
            .and_then(|b| b.palette.get(*b.indices.get((z / 4) * 4 + x / 4)? as usize))
            .and_then(|name| t.console_biome(name))
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
    for ((x, y, z), st) in &extra_tiles {
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
    level.insert("BlockLight".into(), nibbles(&vec![0u8; 32768]));
    level.insert("HeightMap".into(), Tag::ByteArray(heightmap));
    level.insert("TerrainPopulatedFlags".into(), Tag::Short(2046));
    level.insert("Biomes".into(), Tag::ByteArray(biomes));
    level.insert("Entities".into(), list(TagType::Compound, legacy_entities));
    level.insert("TileEntities".into(), list(TagType::Compound, tiles));
    let mut root = Compound::new();
    root.insert("Level".into(), Tag::Compound(level));
    let _ = ctx.platform;
    Some(NamedTag { name: String::new(), tag: Tag::Compound(root) })
}

fn region_name(dim: i32, rx: i32, rz: i32) -> String {
    match dim {
        -1 => format!("DIM-1r.{rx}.{rz}.mcr"),
        1 => format!("DIM1/r.{rx}.{rz}.mcr"),
        _ => format!("r.{rx}.{rz}.mcr"),
    }
}

/// The largest world a console game has: 320 chunks across.
const HALF_SIZE: i32 = 160;

fn level_dat(level: &LevelData, size: i32) -> Result<Vec<u8>, Error> {
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
    d.insert("HellScale".into(), Tag::Int(8));
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
    let registry = legacy_registry(&java);
    let renames = Renames::java_blocks();
    let ctx = Ctx { map, registry: &registry, renames: &renames, java: &java, classic: &classic, platform };
    let codec = platform.chunk_codec();
    let little = platform == Platform::Windows64;
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
            for (lx, lz) in coords {
                let Some(raw) = region.raw_chunk(lx, lz)? else { continue };
                let Ok(chunk) = Chunk::from_nbt(raw.nbt()?) else { continue };
                let pos = ChunkPos::new(chunk.x + offset.0, chunk.z + offset.1);
                if !(-HALF_SIZE..HALF_SIZE).contains(&pos.x) || !(-HALF_SIZE..HALF_SIZE).contains(&pos.z) {
                    report.outside += 1;
                    continue;
                }
                let entities = entity_chunks.get(&(chunk.x, chunk.z)).cloned().filter(|_| options.entities).unwrap_or_default();
                let mut chunk = chunk;
                let raised = if crate::depth::applies(&chunk) { crate::depth::fix(&mut chunk, options.depth) } else { 0 };
                let Some(mut tree) = chunk_to_console(&chunk, &entities, number, raised, &ctx, &mut report) else { continue };
                if !options.tile_entities {
                    if let Some(level) = tree.tag.as_compound_mut().and_then(|c| c.get_mut("Level")).and_then(Tag::as_compound_mut) {
                        level.insert("TileEntities".into(), list(TagType::Compound, Vec::new()));
                    }
                }
                if offset != (0, 0) {
                    shift_chunk(&mut tree, offset);
                }
                let raw = RawConsoleChunk::encode_compressed(&tree, 8, codec, timestamp).map_err(Error::from)?;
                let (rx, rz) = (pos.x.div_euclid(32), pos.z.div_euclid(32));
                let builder = builders.entry((rx, rz)).or_insert_with(|| {
                    let mut b = ConsoleRegionBuilder::default();
                    b.little = little;
                    b
                });
                builder.set(pos.x.rem_euclid(32) as usize, pos.z.rem_euclid(32) as usize, Some(raw));
                report.chunks += 1;
                progress.chunk(pos.x, pos.z);
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
    files.push(ArchiveEntry::new("level.dat", level_dat(&level, HALF_SIZE * 2)?, 0));

    let archive = SaveArchive::new(8, files, little);
    let save = ConsoleSave { platform, archive, source: dst.to_path_buf() };
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

    #[test]
    fn states_go_back_to_the_ids_they_came_from() {
        let t = LegacyTables::get();
        // Every state of the console's blocks maps to an id and data whose state is the same
        // (ignoring what depends on neighbours), and for most to the very ones it came from.
        let mut exact = 0;
        let mut total = 0;
        for ((id, data), state) in t.console_states() {
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
