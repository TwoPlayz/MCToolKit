//! Legacy console to the modern Java form.
//!
//! The console games descend from Java 1.8: numeric block ids with data values, numeric item ids
//! with damage, 1.8 entity and block entity names. This turns a console save into a 26.x Java
//! world that the rest of the converter then takes anywhere (older Java versions, Bedrock).
//! Every block, item and biome goes through the tables the Java game itself produced when it
//! upgraded a world holding all of them (`legacy_tables`); only what depends on the neighbours
//! (stair corners, fence connections, door halves, double chests, ...) is worked out here.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::Ordering;

use indexmap::IndexMap;
use mctk_anvil::{ChunkCompression, ChunkPos, RawChunk, RegionBuilder, RegionPos};
use mctk_console::{ConsoleChunk, ConsoleRegion, ConsoleSave};
use mctk_java::{BlockState, BlockStorage, Chunk, PaletteStyle, PalettedContainer, Section};
use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};
use mctk_registry::Registry;

use crate::Error;
use crate::java_entities::{Form, block_entity_convert, entity_convert};
use crate::java_items::Notes;
use crate::legacy_tables::LegacyTables;
use crate::options::{ConvertOptions, Progress};

/// Data version of the chunks written (a 26.3 world); the player files are classic (1.20.1).
const CLASSIC_DV: i32 = 3465;

fn s(v: &str) -> Tag {
    Tag::String(v.into())
}

fn list(ty: TagType, tags: Vec<Tag>) -> Tag {
    Tag::List(List::from_tags(ty, tags).expect("one type"))
}

fn num(c: &Compound, k: &str) -> Option<i64> {
    c.get(k).and_then(Tag::as_i64)
}

/// `{"text":"..."}` unless the text already is JSON (1.8 stores sign and item text as either).
pub(crate) fn json_text(text: &str) -> String {
    let t = text.trim_start();
    if t.starts_with('{') || t.starts_with('[') || (t.starts_with('"') && t.ends_with('"') && t.len() > 1) {
        text.to_string()
    } else {
        serde_json::json!({ "text": text }).to_string()
    }
}

fn snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else if c == ' ' {
            out.push('_');
        } else {
            out.push(c);
        }
    }
    out
}

const ENCHANTMENTS: &[(i64, &str)] = &[
    (0, "protection"),
    (1, "fire_protection"),
    (2, "feather_falling"),
    (3, "blast_protection"),
    (4, "projectile_protection"),
    (5, "respiration"),
    (6, "aqua_affinity"),
    (7, "thorns"),
    (8, "depth_strider"),
    (16, "sharpness"),
    (17, "smite"),
    (18, "bane_of_arthropods"),
    (19, "knockback"),
    (20, "fire_aspect"),
    (21, "looting"),
    (32, "efficiency"),
    (33, "silk_touch"),
    (34, "unbreaking"),
    (35, "fortune"),
    (48, "power"),
    (49, "punch"),
    (50, "flame"),
    (51, "infinity"),
    (61, "luck_of_the_sea"),
    (62, "lure"),
];

fn enchantments(t: &Tag, notes: &mut Notes) -> Tag {
    let mut out = Vec::new();
    if let Tag::List(l) = t {
        for e in l.iter().filter_map(Tag::as_compound) {
            let (Some(id), Some(lvl)) = (num(e, "id"), num(e, "lvl")) else { continue };
            match ENCHANTMENTS.iter().find(|(n, _)| *n == id) {
                Some((_, name)) => out.push(Tag::Compound(Compound::from([
                    ("id".to_string(), s(&format!("minecraft:{name}"))),
                    ("lvl".to_string(), Tag::Short(lvl as i16)),
                ]))),
                None => notes.add(format!("enchantment {id} does not exist in Java")),
            }
        }
    }
    list(TagType::Compound, out)
}

/// The potion name and item for a 1.8 potion damage value.
fn potion(damage: i64) -> (&'static str, String) {
    const BASE: [&str; 16] = [
        "", "regeneration", "swiftness", "fire_resistance", "poison", "healing", "night_vision", "", "weakness",
        "strength", "slowness", "leaping", "harming", "water_breathing", "invisibility", "",
    ];
    let item = if damage & 16384 != 0 { "minecraft:splash_potion" } else { "minecraft:potion" };
    let base = BASE[(damage & 15) as usize];
    let name = if base.is_empty() {
        match damage & 0x7f {
            16 => "awkward",
            32 => "thick",
            64 => "mundane",
            _ if damage & 0x2000 != 0 && damage & 0x7f == 0 => "mundane",
            _ => "water",
        }
        .to_string()
    } else if damage & 32 != 0 && !matches!(base, "fire_resistance" | "night_vision" | "weakness" | "water_breathing" | "invisibility" | "slowness") {
        format!("strong_{base}")
    } else if damage & 64 != 0 && !matches!(base, "healing" | "harming") {
        format!("long_{base}")
    } else {
        base.to_string()
    };
    (item, format!("minecraft:{name}"))
}

fn attribute_name(n: &str) -> String {
    let bare = n.strip_prefix("generic.").unwrap_or(n);
    let s = match bare {
        "maxHealth" => "max_health",
        "followRange" => "follow_range",
        "knockbackResistance" => "knockback_resistance",
        "movementSpeed" => "movement_speed",
        "attackDamage" => "attack_damage",
        "horse.jumpStrength" => "jump_strength",
        "zombie.spawnReinforcements" => "spawn_reinforcements",
        other => other,
    };
    match n {
        "horse.jumpStrength" => "minecraft:horse.jump_strength".into(),
        "zombie.spawnReinforcements" => "minecraft:zombie.spawn_reinforcements".into(),
        _ => format!("minecraft:generic.{s}"),
    }
}

/// A legacy item stack as a 1.20.1 one (`id`, `Count`, `tag`), or None for an item that has no
/// Java counterpart.
pub(crate) fn item(it: &Compound, notes: &mut Notes) -> Option<Compound> {
    let tables = LegacyTables::get();
    let damage = num(it, "Damage").unwrap_or(0);
    let (name, extra): (String, Compound) = match it.get("id") {
        Some(Tag::String(n)) => (n.clone(), Compound::new()),
        Some(t) => {
            let id = t.as_i64()? as i16;
            let mut tag = Compound::new();
            if id == 373 {
                let (item, potion) = potion(damage);
                tag.insert("Potion".into(), s(&potion));
                (item.to_string(), tag)
            } else {
                let Some(info) = tables.item(id, damage as i16) else {
                    notes.add(format!("item {id} does not exist in Java"));
                    return None;
                };
                if info.keeps_damage && damage != 0 {
                    tag.insert("Damage".into(), Tag::Int(damage as i32));
                }
                if let Some(e) = &info.entity {
                    tag.insert("EntityTag".into(), Tag::Compound(Compound::from([("id".to_string(), s(e))])));
                }
                if info.map {
                    tag.insert("map".into(), Tag::Int(damage as i32));
                }
                (info.name.clone(), tag)
            }
        }
        None => return None,
    };
    let mut tag = extra;
    if let Some(Tag::Compound(old)) = it.get("tag") {
        for (k, v) in old {
            match (k.as_str(), v) {
                ("ench", v) => {
                    tag.insert("Enchantments".into(), enchantments(v, notes));
                }
                ("StoredEnchantments", v) => {
                    tag.insert("StoredEnchantments".into(), enchantments(v, notes));
                }
                ("display", Tag::Compound(d)) => {
                    let mut d = d.clone();
                    if let Some(Tag::String(n)) = d.get("Name") {
                        let j = json_text(n);
                        d.insert("Name".into(), s(&j));
                    }
                    if let Some(Tag::List(l)) = d.get("Lore") {
                        let lines = l.iter().filter_map(Tag::as_str).map(|t| s(&json_text(t))).collect();
                        d.insert("Lore".into(), list(TagType::String, lines));
                    }
                    tag.insert("display".into(), Tag::Compound(d));
                }
                ("AttributeModifiers", Tag::List(l)) => {
                    let mods = l
                        .iter()
                        .filter_map(Tag::as_compound)
                        .map(|m| {
                            let mut m = m.clone();
                            if let Some(Tag::String(n)) = m.get("AttributeName") {
                                let n = attribute_name(n);
                                m.insert("AttributeName".into(), s(&n));
                            }
                            if let (Some(most), Some(least)) = (num(&m, "UUIDMost"), num(&m, "UUIDLeast")) {
                                m.shift_remove("UUIDMost");
                                m.shift_remove("UUIDLeast");
                                m.insert("UUID".into(), uuid_tag(most, least));
                            }
                            Tag::Compound(m)
                        })
                        .collect();
                    tag.insert("AttributeModifiers".into(), list(TagType::Compound, mods));
                }
                _ => {
                    tag.insert(k.clone(), v.clone());
                }
            }
        }
    }
    let mut out = Compound::new();
    if let Some(slot) = it.get("Slot") {
        out.insert("Slot".into(), slot.clone());
    }
    out.insert("id".into(), s(&name));
    out.insert("Count".into(), Tag::Byte(num(it, "Count").unwrap_or(1) as i8));
    if !tag.is_empty() {
        out.insert("tag".into(), Tag::Compound(tag));
    }
    Some(out)
}

fn uuid_tag(most: i64, least: i64) -> Tag {
    Tag::IntArray(vec![(most >> 32) as i32, most as i32, (least >> 32) as i32, least as i32])
}

fn items(t: &Tag, notes: &mut Notes) -> Tag {
    let converted: Vec<Tag> = match t {
        Tag::List(l) => l.iter().filter_map(Tag::as_compound).filter_map(|c| item(c, notes)).map(Tag::Compound).collect(),
        _ => Vec::new(),
    };
    list(TagType::Compound, converted)
}

fn stack(t: Option<&Tag>, notes: &mut Notes) -> Option<Tag> {
    item(t?.as_compound()?, notes).map(Tag::Compound)
}

/// 1.8 entity names to Java ids.
const ENTITIES: &[(&str, &str)] = &[
    ("Item", "item"),
    ("XPOrb", "experience_orb"),
    ("LeashKnot", "leash_knot"),
    ("Painting", "painting"),
    ("Arrow", "arrow"),
    ("Snowball", "snowball"),
    ("Fireball", "fireball"),
    ("SmallFireball", "small_fireball"),
    ("ThrownEnderpearl", "ender_pearl"),
    ("EyeOfEnderSignal", "eye_of_ender"),
    ("ThrownPotion", "potion"),
    ("ThrownExpBottle", "experience_bottle"),
    ("ItemFrame", "item_frame"),
    ("WitherSkull", "wither_skull"),
    ("PrimedTnt", "tnt"),
    ("FallingSand", "falling_block"),
    ("FireworksRocketEntity", "firework_rocket"),
    ("Boat", "boat"),
    ("MinecartRideable", "minecart"),
    ("MinecartChest", "chest_minecart"),
    ("MinecartFurnace", "furnace_minecart"),
    ("MinecartTNT", "tnt_minecart"),
    ("MinecartHopper", "hopper_minecart"),
    ("MinecartSpawner", "spawner_minecart"),
    ("MinecartCommandBlock", "command_block_minecart"),
    ("Mob", "mob"),
    ("Monster", "monster"),
    ("Creeper", "creeper"),
    ("Skeleton", "skeleton"),
    ("Spider", "spider"),
    ("Giant", "giant"),
    ("Zombie", "zombie"),
    ("Slime", "slime"),
    ("Ghast", "ghast"),
    ("PigZombie", "zombified_piglin"),
    ("Enderman", "enderman"),
    ("CaveSpider", "cave_spider"),
    ("Silverfish", "silverfish"),
    ("Blaze", "blaze"),
    ("LavaSlime", "magma_cube"),
    ("EnderDragon", "ender_dragon"),
    ("WitherBoss", "wither"),
    ("Bat", "bat"),
    ("Witch", "witch"),
    ("Pig", "pig"),
    ("Sheep", "sheep"),
    ("Cow", "cow"),
    ("Chicken", "chicken"),
    ("Squid", "squid"),
    ("Wolf", "wolf"),
    ("MushroomCow", "mooshroom"),
    ("SnowMan", "snow_golem"),
    ("Ozelot", "ocelot"),
    ("VillagerGolem", "iron_golem"),
    ("EntityHorse", "horse"),
    ("Villager", "villager"),
    ("EnderCrystal", "end_crystal"),
];

fn entity_id(old: &str) -> Option<String> {
    if old.starts_with("minecraft:") {
        return Some(old.to_string());
    }
    ENTITIES.iter().find(|(o, _)| *o == old).map(|(_, n)| format!("minecraft:{n}"))
}

/// Parses `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` into the four ints Java stores.
fn parse_uuid(text: &str) -> Option<Tag> {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let part = |i: usize| u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).ok().map(|v| v as i32);
    Some(Tag::IntArray(vec![part(0)?, part(1)?, part(2)?, part(3)?]))
}

fn attributes(t: &Tag) -> Tag {
    let converted: Vec<Tag> = match t {
        Tag::List(l) => l
            .iter()
            .filter_map(Tag::as_compound)
            .map(|a| {
                let mut a = a.clone();
                if let Some(Tag::String(n)) = a.get("Name") {
                    let n = attribute_name(n);
                    a.insert("Name".into(), s(&n));
                }
                if let Some(Tag::List(mods)) = a.get("Modifiers") {
                    let m: Vec<Tag> = mods
                        .iter()
                        .filter_map(Tag::as_compound)
                        .map(|m| {
                            let mut m = m.clone();
                            if let (Some(most), Some(least)) = (num(&m, "UUIDMost"), num(&m, "UUIDLeast")) {
                                m.shift_remove("UUIDMost");
                                m.shift_remove("UUIDLeast");
                                m.insert("UUID".into(), uuid_tag(most, least));
                            }
                            Tag::Compound(m)
                        })
                        .collect();
                    a.insert("Modifiers".into(), list(TagType::Compound, m));
                }
                Tag::Compound(a)
            })
            .collect(),
        _ => Vec::new(),
    };
    list(TagType::Compound, converted)
}

/// The fields every 1.8 living entity (and the player) has, as 1.20.1 spells them.
fn living(e: &mut Compound, notes: &mut Notes) {
    if let (Some(most), Some(least)) = (num(e, "UUIDMost"), num(e, "UUIDLeast")) {
        e.shift_remove("UUIDMost");
        e.shift_remove("UUIDLeast");
        e.insert("UUID".into(), uuid_tag(most, least));
    }
    if let Some(h) = e.get("HealF").and_then(Tag::as_f64).or_else(|| e.get("Health").and_then(Tag::as_f64)) {
        e.insert("Health".into(), Tag::Float(h as f32));
        e.shift_remove("HealF");
    }
    if let Some(Tag::List(eq)) = e.shift_remove("Equipment") {
        let slot = |i: usize, notes: &mut Notes| stack(eq.get(i), notes).unwrap_or_else(|| Tag::Compound(Compound::new()));
        let hand = vec![slot(0, notes), Tag::Compound(Compound::new())];
        let armor = vec![slot(1, notes), slot(2, notes), slot(3, notes), slot(4, notes)];
        e.insert("HandItems".into(), list(TagType::Compound, hand));
        e.insert("ArmorItems".into(), list(TagType::Compound, armor));
    }
    if let Some(Tag::List(d)) = e.shift_remove("DropChances") {
        let f = |i: usize| d.get(i).and_then(Tag::as_f64).unwrap_or(0.085) as f32;
        e.insert("HandDropChances".into(), list(TagType::Float, vec![Tag::Float(f(0)), Tag::Float(0.085)]));
        e.insert("ArmorDropChances".into(), list(TagType::Float, (1..5).map(|i| Tag::Float(f(i))).collect()));
    }
    if let Some(a) = e.get("Attributes").cloned() {
        e.insert("Attributes".into(), attributes(&a));
    }
    if let Some(Tag::String(owner)) = e.get("OwnerUUID").cloned() {
        e.shift_remove("OwnerUUID");
        if let Some(u) = parse_uuid(&owner) {
            e.insert("Owner".into(), u);
        }
    }
    e.shift_remove("Owner").filter(|t| matches!(t, Tag::IntArray(_))).map(|t| e.insert("Owner".into(), t));
    e.shift_remove("Dimension");
    if let Some(n) = e.get("CustomName").and_then(Tag::as_str).map(json_text) {
        e.insert("CustomName".into(), s(&n));
    }
}

fn facing_item_frame(f: i64) -> i8 {
    // 1.8: 0 south, 1 west, 2 north, 3 east. 1.20: 0 down, 1 up, 2 north, 3 south, 4 west, 5 east.
    match f {
        0 => 3,
        1 => 4,
        2 => 2,
        _ => 5,
    }
}

/// One villager trade as a 1.20.1 one: what is bought (one or two stacks), what is sold, and how
/// often it can be used. Trades whose items do not exist in Java are dropped.
fn trade(r: &Compound, notes: &mut Notes) -> Option<Compound> {
    let stack = |k: &str, notes: &mut Notes| -> Option<Compound> {
        let mut c = item(r.get(k)?.as_compound()?, notes)?;
        c.shift_remove("Slot");
        (num(&c, "Count").unwrap_or(0) > 0).then_some(c)
    };
    let (buy, sell) = (stack("buy", notes)?, stack("sell", notes)?);
    let mut out = Compound::new();
    out.insert("buy".into(), Tag::Compound(buy));
    if let Some(b) = stack("buyB", notes) {
        out.insert("buyB".into(), Tag::Compound(b));
    }
    out.insert("sell".into(), Tag::Compound(sell));
    out.insert("uses".into(), Tag::Int(num(r, "uses").unwrap_or(0) as i32));
    out.insert("maxUses".into(), Tag::Int(num(r, "maxUses").unwrap_or(7) as i32));
    out.insert("rewardExp".into(), Tag::Byte(num(r, "rewardExp").unwrap_or(1) as i8));
    out.insert("xp".into(), Tag::Int(1));
    out.insert("priceMultiplier".into(), Tag::Float(0.05));
    out.insert("specialPrice".into(), Tag::Int(0));
    out.insert("demand".into(), Tag::Int(0));
    Some(out)
}

/// One legacy entity to a 1.20.1 one. False when the entity has no Java counterpart.
pub(crate) fn entity(e: &mut Compound, notes: &mut Notes) -> bool {
    let Some(old) = e.get("id").and_then(Tag::as_str).map(str::to_string) else { return false };
    let Some(mut id) = entity_id(&old) else {
        notes.add(format!("entity {old} does not exist in Java"));
        return false;
    };
    living(e, notes);
    match id.as_str() {
        "minecraft:item" => {
            if let Some(i) = stack(e.get("Item"), notes) {
                e.insert("Item".into(), i);
            } else {
                return false;
            }
        }
        "minecraft:item_frame" => {
            if let Some(i) = stack(e.get("Item"), notes) {
                e.insert("Item".into(), i);
            }
            if let Some(f) = num(e, "Facing") {
                e.insert("Facing".into(), Tag::Byte(facing_item_frame(f)));
            }
        }
        "minecraft:painting" => {
            if let Some(m) = e.shift_remove("Motive").and_then(|t| t.as_str().map(snake)) {
                e.insert("variant".into(), s(&format!("minecraft:{m}")));
            }
            if let Some(d) = num(e, "Direction").or_else(|| num(e, "Dir")) {
                e.shift_remove("Direction");
                e.shift_remove("Dir");
                e.insert("Facing".into(), Tag::Byte(d as i8));
            }
        }
        "minecraft:tnt" => {
            if let Some(f) = num(e, "Fuse") {
                e.insert("fuse".into(), Tag::Short(f as i16));
                e.shift_remove("Fuse");
            }
        }
        "minecraft:falling_block" => {
            let block = match (num(e, "TileID"), e.get("Tile").and_then(Tag::as_str)) {
                (Some(i), _) => LegacyTables::get().block(i as u8, num(e, "Data").unwrap_or(0) as u8).cloned(),
                _ => None,
            };
            if let Some(b) = block {
                let mut state = Compound::from([("Name".to_string(), s(&b.name))]);
                if !b.properties.is_empty() {
                    state.insert(
                        "Properties".into(),
                        Tag::Compound(b.properties.iter().map(|(k, v)| (k.clone(), s(v))).collect()),
                    );
                }
                e.insert("BlockState".into(), Tag::Compound(state));
            }
            for k in ["TileID", "Tile", "Data"] {
                e.shift_remove(k);
            }
        }
        "minecraft:zombie" => {
            if num(e, "IsVillager") == Some(1) {
                id = "minecraft:zombie_villager".into();
            }
            e.shift_remove("IsVillager");
        }
        "minecraft:skeleton" => {
            if num(e, "SkeletonType") == Some(1) {
                id = "minecraft:wither_skeleton".into();
            }
            e.shift_remove("SkeletonType");
        }
        "minecraft:horse" => {
            id = match num(e, "Type").unwrap_or(0) {
                1 => "minecraft:donkey",
                2 => "minecraft:mule",
                3 => "minecraft:zombie_horse",
                4 => "minecraft:skeleton_horse",
                _ => "minecraft:horse",
            }
            .into();
            e.shift_remove("Type");
            if let Some(i) = stack(e.get("SaddleItem"), notes) {
                e.insert("SaddleItem".into(), i);
            }
            if let Some(i) = stack(e.get("ArmorItem"), notes) {
                e.insert("ArmorItem".into(), i);
            }
            if let Some(it) = e.get("Items").cloned() {
                e.insert("Items".into(), items(&it, notes));
            }
        }
        "minecraft:ocelot" => {
            let t = num(e, "CatType").unwrap_or(0);
            if t > 0 {
                id = "minecraft:cat".into();
                e.insert("CatType".into(), Tag::Int(match t { 1 => 10, other => other as i32 }));
            }
        }
        "minecraft:villager" => {
            let profession = ["farmer", "librarian", "cleric", "armorer", "butcher"]
                [(num(e, "Profession").unwrap_or(0).clamp(0, 4)) as usize];
            e.insert(
                "VillagerData".into(),
                Tag::Compound(Compound::from([
                    ("profession".to_string(), s(&format!("minecraft:{profession}"))),
                    ("level".to_string(), Tag::Int(1)),
                    ("type".to_string(), s("minecraft:plains")),
                ])),
            );
            e.shift_remove("Profession");
            e.shift_remove("Career");
            e.shift_remove("CareerLevel");
            if let Some(Tag::Compound(offers)) = e.shift_remove("Offers") {
                let recipes: Vec<Tag> = match offers.get("Recipes") {
                    Some(Tag::List(l)) => l.iter().filter_map(Tag::as_compound).filter_map(|r| trade(r, notes)).map(Tag::Compound).collect(),
                    _ => Vec::new(),
                };
                // A villager with trades must be a level that shows them: two a level, five levels.
                let level = recipes.len().div_ceil(2).clamp(1, 5) as i32;
                if let Some(Tag::Compound(d)) = e.get_mut("VillagerData") {
                    d.insert("level".into(), Tag::Int(level));
                }
                e.insert("Xp".into(), Tag::Int(0));
                e.insert("Offers".into(), Tag::Compound(Compound::from([("Recipes".to_string(), list(TagType::Compound, recipes))])));
            }
        }
        "minecraft:chest_minecart" | "minecraft:hopper_minecart" => {
            if let Some(it) = e.get("Items").cloned() {
                e.insert("Items".into(), items(&it, notes));
            }
        }
        "minecraft:boat" => {
            e.insert("Type".into(), s("oak"));
        }
        _ => {}
    }
    e.insert("id".into(), s(&id));
    true
}

const BLOCK_ENTITIES: &[(&str, &str)] = &[
    ("Furnace", "furnace"),
    ("Chest", "chest"),
    ("EnderChest", "ender_chest"),
    ("RecordPlayer", "jukebox"),
    ("Trap", "dispenser"),
    ("Dropper", "dropper"),
    ("Sign", "sign"),
    ("MobSpawner", "mob_spawner"),
    ("Music", "note_block"),
    ("Piston", "piston"),
    ("Cauldron", "brewing_stand"),
    ("EnchantTable", "enchanting_table"),
    ("Airportal", "end_portal"),
    ("Control", "command_block"),
    ("Beacon", "beacon"),
    ("Skull", "skull"),
    ("DLDetector", "daylight_detector"),
    ("Hopper", "hopper"),
    ("Comparator", "comparator"),
    ("FlowerPot", "flower_pot"),
];

fn block_entity_name(old: &str) -> Option<String> {
    if old.starts_with("minecraft:") {
        return Some(old.to_string());
    }
    BLOCK_ENTITIES.iter().find(|(o, _)| *o == old).map(|(_, n)| format!("minecraft:{n}"))
}

/// One legacy block entity to a 1.20.1 one. False when there is nothing to keep.
pub(crate) fn block_entity(te: &mut Compound, notes: &mut Notes) -> bool {
    let Some(old) = te.get("id").and_then(Tag::as_str).map(str::to_string) else { return false };
    let Some(id) = block_entity_name(&old) else {
        notes.add(format!("block entity {old} does not exist in Java"));
        return false;
    };
    if let Some(it) = te.get("Items").cloned() {
        te.insert("Items".into(), items(&it, notes));
    }
    if let Some(n) = te.get("CustomName").and_then(Tag::as_str).map(json_text) {
        te.insert("CustomName".into(), s(&n));
    }
    match id.as_str() {
        "minecraft:sign" => {
            // 1.20 signs have a front and a back; the console ones are the front.
            let side = |lines: Vec<String>| {
                Tag::Compound(Compound::from([
                    ("messages".to_string(), list(TagType::String, lines.iter().map(|l| s(l)).collect())),
                    ("color".to_string(), s("black")),
                    ("has_glowing_text".to_string(), Tag::Byte(0)),
                ]))
            };
            let front: Vec<String> = ["Text1", "Text2", "Text3", "Text4"]
                .iter()
                .map(|k| json_text(te.shift_remove(*k).as_ref().and_then(Tag::as_str).unwrap_or("")))
                .collect();
            te.insert("front_text".into(), side(front));
            te.insert("back_text".into(), side(vec![json_text(""); 4]));
            te.insert("is_waxed".into(), Tag::Byte(0));
        }
        "minecraft:furnace" => {
            te.entry("CookTimeTotal".into()).or_insert(Tag::Short(200));
        }
        "minecraft:mob_spawner" => {
            if let Some(Tag::String(old)) = te.shift_remove("EntityId") {
                if let Some(new) = entity_id(&old) {
                    te.insert(
                        "SpawnData".into(),
                        Tag::Compound(Compound::from([(
                            "entity".to_string(),
                            Tag::Compound(Compound::from([("id".to_string(), s(&new))])),
                        )])),
                    );
                }
            }
        }
        "minecraft:jukebox" => {
            if let Some(i) = stack(te.get("RecordItem"), notes) {
                te.insert("RecordItem".into(), i);
            }
        }
        "minecraft:skull" => {
            if let Some(Tag::String(n)) = te.shift_remove("ExtraType") {
                te.insert("SkullOwner".into(), s(&n));
            }
        }
        "minecraft:command_block" => {}
        _ => {}
    }
    te.insert("id".into(), s(&id));
    true
}

type Cells<'a> = &'a dyn Fn(i32, i32, i32) -> (u8, u8);

/// A cell outside the chunk being converted.
const UNKNOWN: u8 = 255;
/// Blocks that power or sense redstone, which a wire next to them connects to.
const SIGNAL: &[u8] = &[28, 69, 70, 72, 75, 76, 77, 131, 143, 146, 147, 148, 149, 150, 151, 152];
const STAIRS: &[u8] = &[53, 67, 108, 109, 114, 128, 134, 135, 136, 156, 163, 164, 180];
const FENCES: &[u8] = &[85, 113, 188, 189, 190, 191, 192];
const FENCE_GATES: &[u8] = &[107, 183, 184, 185, 186, 187];
const PANES: &[u8] = &[101, 102, 160];
const SOLID: &[u8] = &[
    1, 2, 3, 4, 5, 7, 12, 13, 14, 15, 16, 17, 19, 21, 22, 23, 24, 25, 35, 41, 42, 43, 45, 46, 47, 48, 49, 52, 56, 57, 58,
    61, 62, 73, 74, 80, 82, 84, 86, 87, 88, 89, 91, 95, 97, 98, 112, 121, 123, 124, 129, 133, 137, 152, 153, 155, 159,
    161, 162, 165, 168, 170, 172, 173, 174, 179, 181,
];

fn horizontal(f: &str) -> (i32, i32) {
    match f {
        "north" => (0, -1),
        "south" => (0, 1),
        "west" => (-1, 0),
        _ => (1, 0),
    }
}

fn rotate_ccw(f: &str) -> &'static str {
    match f {
        "north" => "west",
        "west" => "south",
        "south" => "east",
        _ => "north",
    }
}

fn opposite(f: &str) -> &'static str {
    match f {
        "north" => "south",
        "south" => "north",
        "west" => "east",
        _ => "west",
    }
}

fn axis_of(f: &str) -> u8 {
    u8::from(matches!(f, "east" | "west"))
}

/// The state of the block at `(x, y, z)`, with what depends on its neighbours worked out.
fn state_at(t: &LegacyTables, at: Cells, x: i32, y: i32, z: i32) -> BlockState {
    let (id, data) = at(x, y, z);
    let Some(base) = t.block(id, data).cloned() else { return BlockState::air() };
    let mut st = base;
    let prop = |s: &BlockState, k: &str| s.properties.get(k).cloned().unwrap_or_default();
    if STAIRS.contains(&id) {
        let facing = prop(&st, "facing");
        let half = prop(&st, "half");
        let stair = |dx: i32, dz: i32| -> Option<(String, String)> {
            let (nid, nd) = at(x + dx, y, z + dz);
            if !STAIRS.contains(&nid) {
                return None;
            }
            let n = t.block(nid, nd)?;
            Some((prop(n, "facing"), prop(n, "half")))
        };
        let different = |face: &str| -> bool {
            let (dx, dz) = horizontal(face);
            stair(dx, dz) != Some((facing.clone(), half.clone()))
        };
        let (fx, fz) = horizontal(&facing);
        let (bx, bz) = horizontal(opposite(&facing));
        let mut shape = "straight";
        if let Some((f, h)) = stair(fx, fz) {
            if h == half && axis_of(&f) != axis_of(&facing) && different(opposite(&f)) {
                shape = if f == rotate_ccw(&facing) { "outer_left" } else { "outer_right" };
            }
        }
        if shape == "straight" {
            if let Some((f, h)) = stair(bx, bz) {
                if h == half && axis_of(&f) != axis_of(&facing) && different(&f) {
                    shape = if f == rotate_ccw(&facing) { "inner_left" } else { "inner_right" };
                }
            }
        }
        st.properties.insert("shape".into(), shape.into());
    } else if FENCES.contains(&id) || PANES.contains(&id) || id == 139 {
        for (face, (dx, dz)) in [("north", (0, -1)), ("east", (1, 0)), ("south", (0, 1)), ("west", (-1, 0))] {
            let (n, _) = at(x + dx, y, z + dz);
            let connects = if FENCES.contains(&id) {
                // Nether brick fences only join their own kind.
                (FENCES.contains(&n) && ((n == 113) == (id == 113))) || FENCE_GATES.contains(&n) || SOLID.contains(&n)
            } else {
                PANES.contains(&n) || id == 139 && (n == 139 || FENCE_GATES.contains(&n)) || SOLID.contains(&n)
            };
            if st.properties.contains_key(face) {
                st.properties.insert(face.into(), connects.to_string());
            }
        }
    } else if id == 55 {
        // Which sides the dust runs to: onto a neighbouring source or wire, up a block, or down.
        let solid_above = SOLID.contains(&at(x, y + 1, z).0);
        for (face, (dx, dz)) in [("north", (0, -1)), ("east", (1, 0)), ("south", (0, 1)), ("west", (-1, 0))] {
            let (n, nd) = at(x + dx, y, z + dz);
            let sturdy = SOLID.contains(&n) || n == 154;
            let wire_at = |dy: i32| matches!(at(x + dx, y + dy, z + dz).0, 55 | UNKNOWN);
            let repeater_in_line = matches!(n, 93 | 94)
                && t.block(n, nd).is_some_and(|r| axis_of(r.properties.get("facing").map_or("", String::as_str)) == axis_of(face));
            let value = if n == UNKNOWN {
                "side"
            } else if !solid_above && sturdy && wire_at(1) {
                "up"
            } else if n == 55 || repeater_in_line || SIGNAL.contains(&n) || (!SOLID.contains(&n) && n != UNKNOWN && wire_at(-1)) {
                "side"
            } else {
                "none"
            };
            st.properties.insert(face.into(), value.into());
        }
    } else if matches!(id, 64 | 71 | 193..=197) {
        let upper = data & 8 != 0;
        let (other_y, other_data) = if upper { (y - 1, at(x, y - 1, z)) } else { (y + 1, at(x, y + 1, z)) };
        let _ = other_y;
        if other_data.0 == id {
            let (lower_state, upper_state) = if upper {
                (t.block(id, other_data.1).cloned(), Some(st.clone()))
            } else {
                (Some(st.clone()), t.block(id, other_data.1).cloned())
            };
            if let (Some(lo), Some(up)) = (lower_state, upper_state) {
                if upper {
                    st.properties.insert("facing".into(), prop(&lo, "facing"));
                    st.properties.insert("open".into(), prop(&lo, "open"));
                } else {
                    st.properties.insert("hinge".into(), prop(&up, "hinge"));
                    st.properties.insert("powered".into(), prop(&up, "powered"));
                }
            }
        }
    } else if id == 175 && data & 8 != 0 {
        // The upper half of a double plant takes its kind from the lower half.
        let (lid, ld) = at(x, y - 1, z);
        if lid == 175 {
            if let Some(lo) = t.block(lid, ld) {
                st.name = lo.name.clone();
            }
        }
    } else if matches!(id, 2 | 110) || (id == 3 && data == 2) {
        let (above, _) = at(x, y + 1, z);
        if matches!(above, 78 | 80) {
            st.properties.insert("snowy".into(), "true".into());
        }
    } else if matches!(id, 54 | 146) {
        let facing = prop(&st, "facing");
        // The partner clockwise of the front makes this the right half, counter-clockwise the left.
        let clockwise = rotate_ccw(opposite(&facing));
        let counter = rotate_ccw(&facing);
        let same = |face: &str| {
            let (dx, dz) = horizontal(face);
            let (nid, nd) = at(x + dx, y, z + dz);
            nid == id && t.block(nid, nd).map(|n| prop(n, "facing")) == Some(facing.clone())
        };
        if same(clockwise) {
            st.properties.insert("type".into(), "right".into());
        } else if same(counter) {
            st.properties.insert("type".into(), "left".into());
        }
    }
    st
}

/// How far each leaf block is from a log (1 next to one, 7 for not within reach), as the game works
/// it out when it upgrades a chunk. Logs and wood count as 0.
fn leaf_distances(ids: &[u8], height: usize) -> Vec<u8> {
    let mut dist = vec![7u8; ids.len()];
    let mut queue = std::collections::VecDeque::new();
    for (i, &id) in ids.iter().enumerate() {
        if matches!(id, 17 | 162) {
            dist[i] = 0;
            queue.push_back(i);
        }
    }
    while let Some(i) = queue.pop_front() {
        let (x, z, y) = (i % 16, (i / 16) % 16, i / 256);
        let next = dist[i] + 1;
        if next > 6 && dist[i] >= 6 {
            // A leaf at 7 is as far as it gets.
        }
        let mut visit = |nx: usize, ny: usize, nz: usize| {
            let j = (ny * 16 + nz) * 16 + nx;
            if matches!(ids[j], 18 | 161) && next < dist[j] {
                dist[j] = next;
                queue.push_back(j);
            }
        };
        if x > 0 { visit(x - 1, y, z); }
        if x < 15 { visit(x + 1, y, z); }
        if z > 0 { visit(x, y, z - 1); }
        if z < 15 { visit(x, y, z + 1); }
        if y > 0 { visit(x, y - 1, z); }
        if y + 1 < height { visit(x, y + 1, z); }
    }
    dist
}

fn skull_state(mut st: BlockState, kind: i64, rotation: i64) -> BlockState {
    let wall = st.name.contains("wall");
    let (floor, wall_name) = match kind {
        1 => ("minecraft:wither_skeleton_skull", "minecraft:wither_skeleton_wall_skull"),
        2 => ("minecraft:zombie_head", "minecraft:zombie_wall_head"),
        3 => ("minecraft:player_head", "minecraft:player_wall_head"),
        4 => ("minecraft:creeper_head", "minecraft:creeper_wall_head"),
        _ => ("minecraft:skeleton_skull", "minecraft:skeleton_wall_skull"),
    };
    st.name = if wall { wall_name } else { floor }.into();
    if !wall {
        st.properties.insert("rotation".into(), rotation.clamp(0, 15).to_string());
    }
    st
}

pub(crate) struct ConvertedChunk {
    pub chunk: NamedTag,
    pub entities: Vec<Tag>,
}

/// Converts one console chunk. `dimension` is the console dimension number.
pub(crate) fn chunk(c: &ConsoleChunk, dimension: i32, dv: i32, reg: &Registry, notes: &mut Notes) -> Result<ConvertedChunk, Error> {
    let t = LegacyTables::get();
    let level = c.to_nbt().tag.get("Level").and_then(Tag::as_compound).ok_or_else(|| Error::Invalid("chunk without Level".into()))?;
    let height = c.height;
    let mut ids = vec![0u8; 16 * 16 * height];
    let mut datas = vec![0u8; 16 * 16 * height];
    for y in 0..height {
        for z in 0..16 {
            for x in 0..16 {
                let (id, d) = c.block(x, y, z).unwrap_or((0, 0));
                let i = (y * 16 + z) * 16 + x;
                ids[i] = id;
                datas[i] = d;
            }
        }
    }
    let distances = leaf_distances(&ids, height);
    let at = |x: i32, y: i32, z: i32| -> (u8, u8) {
        // Beyond the chunk nothing is known (id 255); above and below the world is air.
        if !(0..16).contains(&x) || !(0..16).contains(&z) {
            return (UNKNOWN, 0);
        }
        if y < 0 || y as usize >= height {
            return (0, 0);
        }
        let i = (y as usize * 16 + z as usize) * 16 + x as usize;
        (ids[i], datas[i])
    };

    // Block entities first: skulls and flower pots take their look from them.
    let mut block_entities = Vec::new();
    let mut by_pos: HashMap<(i32, i32, i32), Compound> = HashMap::new();
    if let Some(Tag::List(l)) = level.get("TileEntities") {
        for te in l.iter().filter_map(Tag::as_compound) {
            let mut te = te.clone();
            let pos = (num(&te, "x").unwrap_or(0) as i32, num(&te, "y").unwrap_or(0) as i32, num(&te, "z").unwrap_or(0) as i32);
            by_pos.insert(pos, te.clone());
            let id = te.get("id").and_then(Tag::as_str).unwrap_or("").to_string();
            if !block_entity(&mut te, notes) {
                continue;
            }
            // Flower pots have no block entity after the flattening, and skulls keep theirs.
            if id == "FlowerPot" {
                continue;
            }
            if block_entity_convert(&mut te, Form::Modern, reg, notes) {
                block_entities.push(Tag::Compound(te));
            }
        }
    }

    let bx = c.x * 16;
    let bz = c.z * 16;
    let biomes: Vec<u8> = match level.get("Biomes") {
        Some(Tag::ByteArray(b)) if b.len() == 256 => b.iter().map(|&v| v as u8).collect(),
        _ => vec![1; 256],
    };
    let mut sections = Vec::new();
    for sy in 0..height / 16 {
        let mut palette: IndexMap<BlockState, u16> = IndexMap::new();
        let mut indices = Vec::with_capacity(4096);
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let (wx, wy, wz) = (x as i32, (sy * 16 + y) as i32, z as i32);
                    let (id, d) = at(wx, wy, wz);
                    let mut st = if id == 0 { BlockState::air() } else { state_at(t, &at, wx, wy, wz) };
                    if matches!(id, 18 | 161) && st.properties.contains_key("distance") {
                        let i = (wy as usize * 16 + z) * 16 + x;
                        st.properties.insert("distance".into(), distances[i].to_string());
                    }
                    if id == 144 {
                        if let Some(te) = by_pos.get(&(bx + wx, wy, bz + wz)) {
                            st = skull_state(st, num(te, "SkullType").unwrap_or(0), num(te, "Rot").unwrap_or(0));
                        }
                    }
                    if id == 140 {
                        let item = by_pos.get(&(bx + wx, wy, bz + wz)).and_then(|te| {
                            let item_id = te.get("Item").and_then(|i| i.as_i64().or_else(|| None));
                            item_id.and_then(|i| t.item(i as i16, num(te, "Data").unwrap_or(0) as i16)).map(|i| i.name.clone())
                        });
                        if let Some(name) = item {
                            let potted = format!("minecraft:potted_{}", name.trim_start_matches("minecraft:"));
                            if reg.has_block(&potted) {
                                st = BlockState::new(potted);
                            }
                        }
                    }
                    let _ = d;
                    let st = crate::java_java::finalize_state(reg, PaletteStyle::IdString, st);
                    let next = palette.len() as u16;
                    indices.push(*palette.entry(st).or_insert(next));
                }
            }
        }
        // One biome per 4x4x4 cell, taken from the cell's first column.
        let mut bpalette: IndexMap<String, u16> = IndexMap::new();
        let mut bindices = Vec::with_capacity(64);
        for cy in 0..4 {
            let _ = cy;
            for cz in 0..4 {
                for cx in 0..4 {
                    let name = t.biome(biomes[(cz * 4) * 16 + cx * 4]).to_string();
                    let next = bpalette.len() as u16;
                    bindices.push(*bpalette.entry(name).or_insert(next));
                }
            }
        }
        sections.push(Section::new(
            sy as i8,
            Some(BlockStorage::Palette(PalettedContainer { palette: palette.into_keys().collect(), indices })),
            Some(PalettedContainer { palette: bpalette.into_keys().collect(), indices: bindices }),
        ));
    }

    let mut fields = Compound::new();
    fields.insert("Status".into(), s("minecraft:full"));
    fields.insert("LastUpdate".into(), Tag::Long(num(level, "LastUpdate").unwrap_or(0)));
    fields.insert("InhabitedTime".into(), Tag::Long(num(level, "InhabitedTime").unwrap_or(0)));
    fields.insert("isLightOn".into(), Tag::Byte(0));
    fields.insert("yPos".into(), Tag::Int(if dimension == 0 { -4 } else { 0 }));
    fields.insert("block_entities".into(), list(TagType::Compound, block_entities));
    let mut chunk = Chunk::new_modern(c.x, c.z, dv, PaletteStyle::IdString, fields, sections);
    // Beds are one block; the Bed tile entity says which colour.
    let coloured: Vec<Compound> = by_pos
        .values()
        .filter(|te| te.get("id").and_then(Tag::as_str) == Some("Bed"))
        .map(|te| {
            let mut b = te.clone();
            b.insert("id".into(), s("minecraft:bed"));
            b
        })
        .collect();
    crate::legacy::colour_beds(&mut chunk, &coloured);

    let mut entities = Vec::new();
    if let Some(Tag::List(l)) = level.get("Entities") {
        for e in l.iter().filter_map(Tag::as_compound) {
            let mut e = e.clone();
            if entity(&mut e, notes) && entity_convert(&mut e, Form::Modern, reg, notes) {
                entities.push(Tag::Compound(e));
            }
        }
    }
    Ok(ConvertedChunk { chunk: chunk.to_nbt(), entities })
}

/// A legacy player as a 1.20.1 player file.
pub(crate) fn player(p: &Compound, uuid: [i32; 4], notes: &mut Notes) -> Compound {
    let mut p = p.clone();
    living(&mut p, notes);
    for key in ["Inventory", "EnderItems"] {
        if let Some(t) = p.get(key).cloned() {
            p.insert(key.into(), items(&t, notes));
        }
    }
    if let Some(d) = num(&p, "Dimension") {
        let name = match d {
            -1 => "minecraft:the_nether",
            1 => "minecraft:the_end",
            _ => "minecraft:overworld",
        };
        p.insert("Dimension".into(), s(name));
    }
    p.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
    p.insert("DataVersion".into(), Tag::Int(CLASSIC_DV));
    p
}

/// A legacy map (`data` compound) as a 1.20.1 one.
pub(crate) fn map(data: &Compound) -> Compound {
    let mut m = data.clone();
    let dimension = match num(&m, "dimension") {
        Some(-1) => "minecraft:the_nether",
        Some(1) => "minecraft:the_end",
        _ => "minecraft:overworld",
    };
    m.insert("dimension".into(), s(dimension));
    for (k, v) in [("trackingPosition", 1), ("unlimitedTracking", 0), ("locked", 0)] {
        m.entry(k.into()).or_insert(Tag::Byte(v));
    }
    m.entry("banners".into()).or_insert_with(|| list(TagType::Compound, Vec::new()));
    m.entry("frames".into()).or_insert_with(|| list(TagType::Compound, Vec::new()));
    m
}

/// The layers of a 1.8 flat-world preset (`2;7,2*3,2;1;village`: version, layers from the bottom as
/// `[count*]id[:data]`, biome id, structures) and its biome.
fn flat_preset(options: &str) -> (Vec<(String, i32)>, String) {
    let t = LegacyTables::get();
    let mut parts = options.split(';');
    let first = parts.next().unwrap_or("");
    // The version number is optional in the oldest presets.
    let (layers, biome) = if first.chars().all(|c| c.is_ascii_digit()) && !first.is_empty() && options.matches(';').count() >= 2 {
        (parts.next().unwrap_or(""), parts.next())
    } else {
        (first, parts.next())
    };
    let mut out: Vec<(String, i32)> = Vec::new();
    for token in layers.split(',').filter(|t| !t.is_empty()) {
        let (count, block) = token.split_once('*').map_or((1, token), |(c, b)| (c.parse().unwrap_or(1), b));
        let (id, data) = block.split_once(':').map_or((block, "0"), |(i, d)| (i, d));
        let name = match id.parse::<u8>() {
            Ok(n) => t.block(n, data.parse().unwrap_or(0)).map(|b| b.name.clone()),
            Err(_) => Some(if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") }),
        };
        if let Some(name) = name {
            out.push((name, count));
        }
    }
    if out.is_empty() {
        out = vec![("minecraft:bedrock".into(), 1), ("minecraft:dirt".into(), 2), ("minecraft:grass_block".into(), 1)];
    }
    let biome = biome.and_then(|b| b.split(',').next()).and_then(|b| b.parse::<u8>().ok()).map_or("minecraft:plains".to_string(), |b| t.biome(b).to_string());
    (out, biome)
}

/// The dimensions of a console world: the usual three, with a flat or large-biomes overworld when
/// that is what the world was made as.
fn dimensions(generator: &str, options: &str, notes: &mut Notes) -> Compound {
    let mut dims = default_dimensions();
    let c = |fields: Vec<(&str, Tag)>| Compound::from_iter(fields.into_iter().map(|(k, v)| (k.to_string(), v)));
    match generator {
        "flat" => {
            let (layers, biome) = flat_preset(options);
            let layers: Vec<Tag> = layers
                .into_iter()
                .map(|(block, height)| Tag::Compound(c(vec![("block", s(&block)), ("height", Tag::Int(height))])))
                .collect();
            let settings = c(vec![
                ("biome", s(&biome)),
                ("features", Tag::Byte(0)),
                ("lakes", Tag::Byte(0)),
                ("layers", list(TagType::Compound, layers)),
                ("structure_overrides", list(TagType::String, Vec::new())),
            ]);
            dims.insert(
                "minecraft:overworld".into(),
                Tag::Compound(c(vec![
                    ("generator", Tag::Compound(c(vec![("type", s("minecraft:flat")), ("settings", Tag::Compound(settings))]))),
                    ("type", s("minecraft:overworld")),
                ])),
            );
            if options.contains("village") || options.contains("stronghold") || options.contains("mineshaft") {
                notes.add("flat world structures (villages, strongholds, ...) are not carried over");
            }
        }
        "largeBiomes" => {
            if let Some(Tag::Compound(o)) = dims.get_mut("minecraft:overworld") {
                if let Some(Tag::Compound(g)) = o.get_mut("generator") {
                    g.insert("settings".into(), s("minecraft:large_biomes"));
                }
            }
        }
        _ => {}
    }
    dims
}

/// The three dimensions every ordinary 26.x world has, as the game writes them for a new world.
fn default_dimensions() -> Compound {
    let c = |fields: Vec<(&str, Tag)>| Compound::from_iter(fields.into_iter().map(|(k, v)| (k.to_string(), v)));
    let dim = |kind: &str, settings: &str, biomes: Compound| {
        Tag::Compound(c(vec![
            (
                "generator",
                Tag::Compound(c(vec![
                    ("settings", s(settings)),
                    ("biome_source", Tag::Compound(biomes)),
                    ("type", s("minecraft:noise")),
                ])),
            ),
            ("type", s(kind)),
        ]))
    };
    let preset = |p: &str| c(vec![("preset", s(p)), ("type", s("minecraft:multi_noise"))]);
    let mut dims = Compound::new();
    dims.insert("minecraft:overworld".into(), dim("minecraft:overworld", "minecraft:overworld", preset("minecraft:overworld")));
    dims.insert("minecraft:the_nether".into(), dim("minecraft:the_nether", "minecraft:nether", preset("minecraft:nether")));
    dims.insert("minecraft:the_end".into(), dim("minecraft:the_end", "minecraft:end", c(vec![("type", s("minecraft:the_end"))])));
    dims
}

fn dimension_dir(dst: &Path, dim: i32) -> std::path::PathBuf {
    let name = match dim {
        -1 => "the_nether",
        1 => "the_end",
        _ => "overworld",
    };
    dst.join("dimensions/minecraft").join(name)
}

/// What converting a console world produced.
#[derive(Debug, Default, Clone)]
pub struct LegacyReport {
    pub chunks: u64,
    pub entities: u64,
    pub notes: Notes,
    pub players: u64,
    pub maps: u64,
}

/// Converts a console save into a 26.x Java world at `dst`.
pub fn console_to_modern(
    save: &ConsoleSave,
    dst: &Path,
    reg: &Registry,
    options: &ConvertOptions,
    progress: &Progress,
) -> Result<LegacyReport, Error> {
    let dv = reg.data_version.ok_or_else(|| Error::Invalid(format!("registry {} has no data version", reg.version)))?;
    if dst.join("level.dat").exists() || dst.join("data").exists() {
        return Err(Error::Invalid(format!("{} already holds a world", dst.display())));
    }
    std::fs::create_dir_all(dst)?;
    let mut report = LegacyReport::default();

    let mut work = Vec::new();
    for (dim, _, _, entry) in save.archive.regions() {
        if entry.data.is_empty() || !options.includes(dim) {
            continue;
        }
        let region = ConsoleRegion::parse(entry.data.clone())?;
        progress.chunks_total.fetch_add(region.chunks().count(), Ordering::Relaxed);
        work.push((dim, region));
    }
    progress.regions_total.store(work.len(), Ordering::Relaxed);

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as u32);
    let mut regions: HashMap<(i32, RegionPos), RegionBuilder> = HashMap::new();
    let mut entity_regions: HashMap<(i32, RegionPos), RegionBuilder> = HashMap::new();
    let offset = options.offset;
    for (dim, region) in &work {
        progress.set_dimension(match dim {
            -1 => "minecraft:the_nether",
            1 => "minecraft:the_end",
            _ => "minecraft:overworld",
        });
        // Chunks are converted on all cores, a region at a time.
        let coords: Vec<(usize, usize)> = region.chunks().collect();
        let results: Vec<Option<(ChunkPos, RawChunk, Option<RawChunk>, Notes, u64)>> = std::thread::scope(|scope| {
            let workers = std::thread::available_parallelism().map_or(4, |n| n.get());
            let per = coords.len().div_ceil(workers).max(1);
            let handles: Vec<_> = coords
                .chunks(per)
                .map(|batch| {
                    scope.spawn(move || {
                        batch
                            .iter()
                            .map(|&(lx, lz)| convert_one(region, lx, lz, *dim, dv, reg, offset, timestamp, progress, (options.tile_entities, options.entities)))
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().expect("worker")).collect()
        });
        let unreadable = results.iter().filter(|r| r.is_none()).count();
        if unreadable > 0 {
            let what = match dim {
                -1 => "Nether",
                1 => "End",
                _ => "Overworld",
            };
            *report.notes.0.entry(format!("{what} chunks could not be read (storage format not supported) and were left out")).or_default() += unreadable as u64;
        }
        for r in results.into_iter().flatten() {
            let (pos, raw, ents, notes, n) = r;
            report.notes.merge(notes);
            report.chunks += 1;
            report.entities += n;
            let (lx, lz) = pos.local();
            regions.entry((*dim, pos.region())).or_default().set(lx, lz, Some(raw));
            if let Some(e) = ents {
                entity_regions.entry((*dim, pos.region())).or_default().set(lx, lz, Some(e));
            }
        }
    }
    progress.regions_done.store(work.len(), Ordering::Relaxed);
    for ((dim, pos), builder) in &regions {
        let dir = dimension_dir(dst, *dim).join("region");
        std::fs::create_dir_all(&dir)?;
        builder.write(&dir, *pos)?;
    }
    for ((dim, pos), builder) in &entity_regions {
        let dir = dimension_dir(dst, *dim).join("entities");
        std::fs::create_dir_all(&dir)?;
        builder.write(&dir, *pos)?;
    }

    // Level settings, the player and the maps.
    let level_entry = save.archive.get("level.dat").ok_or_else(|| Error::Invalid("save has no level.dat".into()))?;
    let (_, raw) = mctk_nbt::decompress(&level_entry.data)?;
    let legacy = mctk_nbt::from_bytes(&raw, mctk_nbt::Flavor::Java)?;
    let mut level = crate::java_level::LevelData::read_classic(&legacy)?;
    // Console levels keep the seed in `RandomSeed`; the reader looks for the 1.16 place.
    if let Some(seed) = legacy.tag.get("Data").and_then(|d| d.get("RandomSeed")).and_then(Tag::as_i64) {
        level.seed = seed;
    }
    level.time = level.time.max(0);
    level.day_time = level.time;
    if level.last_played > 4_000_000_000_000 {
        level.last_played -= 11_644_473_600_000;
    }
    if let Some(g) = options.game_mode {
        level.game_type = g;
    }
    if let Some(d) = options.difficulty {
        level.difficulty = d as i8;
    }
    level.spawn[0] += offset.0 * 16;
    level.spawn[2] += offset.1 * 16;
    level.embedded_player = None;
    if level.dimensions.is_empty() {
        let data = legacy.tag.get("Data").and_then(Tag::as_compound);
        let text = |k: &str| data.and_then(|d| d.get(k)).and_then(Tag::as_str).unwrap_or("");
        level.dimensions = dimensions(text("generatorName"), text("generatorOptions"), &mut report.notes);
    }

    let host = save.archive.entries.iter().find(|e| {
        let name = e.name.rsplit('/').next().unwrap_or(&e.name);
        name.ends_with(".dat") && (e.name.starts_with("players/") || name.starts_with("P_"))
    });
    let legacy_player = match (legacy.tag.get("Data").and_then(|d| d.get("Player")).and_then(Tag::as_compound), host) {
        (Some(p), _) => Some(p.clone()),
        (None, Some(e)) => {
            let (_, raw) = mctk_nbt::decompress(&e.data)?;
            mctk_nbt::from_bytes(&raw, mctk_nbt::Flavor::Java)?.tag.as_compound().cloned()
        }
        _ => None,
    };
    if let Some(mut p) = legacy_player {
        // Console players have no UUID; the file name needs one.
        let uuid = [0x6d63_746b_u32 as i32, 0x4c65_6761_u32 as i32, 0x6379_0001_u32 as i32, 1];
        if let Some(Tag::List(pos)) = p.get("Pos").cloned() {
            let mut v: Vec<f64> = pos.iter().filter_map(Tag::as_f64).collect();
            if v.len() == 3 {
                v[0] += (offset.0 * 16) as f64;
                v[2] += (offset.1 * 16) as f64;
                p.insert("Pos".into(), list(TagType::Double, v.into_iter().map(Tag::Double).collect()));
            }
        }
        let converted = player(&p, uuid, &mut report.notes);
        let file = crate::java_player::PlayerFile {
            uuid: crate::java_player::uuid_text(uuid),
            nbt: converted,
            advancements: None,
            stats: None,
        };
        crate::java_player::write_players(dst, &[file], true, crate::java_entities::Form::Modern, reg, Some(&uuid), &mut report.notes)?;
        level.singleplayer_uuid = Some(uuid);
        report.players = 1;
    }
    level.write_modern(dst, dv, &reg.version)?;

    for e in &save.archive.entries {
        let Some(name) = e.name.strip_prefix("data/") else { continue };
        let Some(id) = name.strip_prefix("map_").and_then(|n| n.strip_suffix(".dat")).filter(|i| i.parse::<u64>().is_ok()) else { continue };
        let (_, raw) = mctk_nbt::decompress(&e.data)?;
        let root = mctk_nbt::from_bytes(&raw, mctk_nbt::Flavor::Java)?;
        if let Some(Tag::Compound(d)) = root.tag.get("data") {
            let path = dst.join(format!("data/minecraft/maps/{id}.dat"));
            crate::java_level::write_data_file(&path, map(d), CLASSIC_DV)?;
            report.maps += 1;
        }
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn convert_one(
    region: &ConsoleRegion,
    lx: usize,
    lz: usize,
    dim: i32,
    dv: i32,
    reg: &Registry,
    offset: (i32, i32),
    timestamp: u32,
    progress: &Progress,
    keep: (bool, bool),
) -> Option<(ChunkPos, RawChunk, Option<RawChunk>, Notes, u64)> {
    let raw = region.raw_chunk(lx, lz).ok()??;
    let source = ConsoleChunk::from_nbt(raw.nbt().ok()?).ok()?;
    let mut notes = Notes::default();
    let converted = chunk(&source, dim, dv, reg, &mut notes).ok()?;
    let pos = ChunkPos::new(source.x + offset.0, source.z + offset.1);
    let mut tree = converted.chunk;
    if !keep.0 {
        if let Some(top) = tree.tag.as_compound_mut() {
            top.shift_remove("block_entities");
        }
    }
    if offset != (0, 0) {
        if let Some(top) = tree.tag.as_compound_mut() {
            top.insert("xPos".into(), Tag::Int(pos.x));
            top.insert("zPos".into(), Tag::Int(pos.z));
            // Block entity positions move with the chunk.
            if let Some(Tag::List(l)) = top.get_mut("block_entities") {
                for t in l.iter_mut() {
                    if let Some(c) = t.as_compound_mut() {
                        shift(c, "x", offset.0 * 16);
                        shift(c, "z", offset.1 * 16);
                    }
                }
            }
        }
    }
    let entities = if keep.1 { converted.entities } else { Vec::new() };
    let count = entities.len() as u64;
    let entity_chunk = (!entities.is_empty()).then(|| {
        let shifted: Vec<Tag> = entities
            .into_iter()
            .map(|t| {
                let mut c = t.as_compound().cloned().unwrap_or_default();
                if let Some(Tag::List(p)) = c.get("Pos").cloned() {
                    let mut v: Vec<f64> = p.iter().filter_map(Tag::as_f64).collect();
                    if v.len() == 3 {
                        v[0] += (offset.0 * 16) as f64;
                        v[2] += (offset.1 * 16) as f64;
                        c.insert("Pos".into(), list(TagType::Double, v.into_iter().map(Tag::Double).collect()));
                    }
                }
                Tag::Compound(c)
            })
            .collect();
        NamedTag {
            name: String::new(),
            tag: Tag::Compound(Compound::from([
                ("DataVersion".to_string(), Tag::Int(dv)),
                ("Position".to_string(), Tag::IntArray(vec![pos.x, pos.z])),
                ("Entities".to_string(), list(TagType::Compound, shifted)),
            ])),
        }
    });
    let raw_chunk = RawChunk::encode(&tree, ChunkCompression::Zlib, timestamp).ok()?;
    let raw_entities = match entity_chunk {
        Some(t) => Some(RawChunk::encode(&t, ChunkCompression::Zlib, timestamp).ok()?),
        None => None,
    };
    progress.chunk(pos.x, pos.z);
    Some((pos, raw_chunk, raw_entities, notes, count))
}

fn shift(c: &mut Compound, key: &str, by: i32) {
    if let Some(v) = num(c, key) {
        c.insert(key.into(), Tag::Int(v as i32 + by));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_become_java_items() {
        let mut notes = Notes::default();
        let sword = Compound::from([
            ("Slot".to_string(), Tag::Byte(3)),
            ("id".to_string(), Tag::Short(276)),
            ("Count".to_string(), Tag::Byte(1)),
            ("Damage".to_string(), Tag::Short(7)),
            (
                "tag".to_string(),
                Tag::Compound(Compound::from([
                    ("ench".to_string(), list(TagType::Compound, vec![Tag::Compound(Compound::from([("id".to_string(), Tag::Short(16)), ("lvl".to_string(), Tag::Short(3))]))])),
                    ("display".to_string(), Tag::Compound(Compound::from([("Name".to_string(), s("Named"))]))),
                ])),
            ),
        ]);
        let out = item(&sword, &mut notes).unwrap();
        assert_eq!(out.get("id"), Some(&s("minecraft:diamond_sword")));
        assert_eq!(out.get("Slot"), Some(&Tag::Byte(3)));
        let tag = out.get("tag").unwrap();
        assert_eq!(tag.get("Damage"), Some(&Tag::Int(7)));
        assert_eq!(tag.get("display").unwrap().get("Name"), Some(&s("{\"text\":\"Named\"}")));
        let ench = tag.get("Enchantments").and_then(Tag::as_list).unwrap();
        assert_eq!(ench.get(0).unwrap().get("id"), Some(&s("minecraft:sharpness")));
        assert!(notes.0.is_empty());
    }

    #[test]
    fn potions_follow_the_1_8_damage_bits() {
        assert_eq!(potion(8193), ("minecraft:potion", "minecraft:regeneration".into()));
        assert_eq!(potion(16384 + 8193), ("minecraft:splash_potion", "minecraft:regeneration".into()));
        assert_eq!(potion(8194 | 32).1, "minecraft:strong_swiftness");
        assert_eq!(potion(8194 | 64).1, "minecraft:long_swiftness");
        assert_eq!(potion(0).1, "minecraft:water");
    }

    #[test]
    fn entities_get_java_ids_and_fields() {
        let mut notes = Notes::default();
        let mut pig = Compound::from([
            ("id".to_string(), s("Pig")),
            ("Health".to_string(), Tag::Short(7)),
            ("UUIDMost".to_string(), Tag::Long(0x0102030405060708)),
            ("UUIDLeast".to_string(), Tag::Long(0x1112131415161718)),
        ]);
        assert!(entity(&mut pig, &mut notes));
        assert_eq!(pig.get("id"), Some(&s("minecraft:pig")));
        assert_eq!(pig.get("Health"), Some(&Tag::Float(7.0)));
        assert_eq!(pig.get("UUID"), Some(&Tag::IntArray(vec![0x01020304, 0x05060708, 0x11121314, 0x15161718])));
        let mut unknown = Compound::from([("id".to_string(), s("NoSuchMob"))]);
        assert!(!entity(&mut unknown, &mut notes));
    }

    #[test]
    fn redstone_dust_runs_to_what_it_can_reach() {
        let t = LegacyTables::get();
        let state = |cells: &dyn Fn(i32, i32, i32) -> (u8, u8)| state_at(t, cells, 5, 5, 5);
        let alone = |x: i32, y: i32, z: i32| if (x, y, z) == (5, 5, 5) { (55, 7) } else { (0, 0) };
        let s = state(&alone);
        assert_eq!(s.properties["power"], "7");
        assert!(["north", "east", "south", "west"].iter().all(|f| s.properties[*f] == "none"));
        // Wire to the east, a lever to the south, a block with wire on top to the west.
        let around = |x: i32, y: i32, z: i32| match (x, y, z) {
            (5, 5, 5) => (55, 0),
            (6, 5, 5) => (55, 0),
            (5, 5, 6) => (69, 0),
            (4, 5, 5) => (1, 0),
            (4, 6, 5) => (55, 0),
            _ => (0, 0),
        };
        let s = state(&around);
        assert_eq!(s.properties["east"], "side");
        assert_eq!(s.properties["south"], "side");
        assert_eq!(s.properties["west"], "up");
        assert_eq!(s.properties["north"], "none");
        // A cover over the dust stops it climbing.
        let covered = |x: i32, y: i32, z: i32| if (x, y, z) == (5, 6, 5) { (1, 0) } else { around(x, y, z) };
        assert_eq!(state(&covered).properties["west"], "none");
    }

    #[test]
    fn trades_and_flat_presets_are_read() {
        let mut notes = Notes::default();
        let item = |id: i16, count: i8| Tag::Compound(Compound::from([("id".to_string(), Tag::Short(id)), ("Count".to_string(), Tag::Byte(count)), ("Damage".to_string(), Tag::Short(0))]));
        let recipe = Compound::from([
            ("buy".to_string(), item(297, 3)),
            ("sell".to_string(), item(388, 1)),
            ("uses".to_string(), Tag::Int(2)),
            ("maxUses".to_string(), Tag::Int(7)),
            ("rewardExp".to_string(), Tag::Byte(1)),
        ]);
        let mut villager = Compound::from([
            ("id".to_string(), s("Villager")),
            ("Profession".to_string(), Tag::Int(0)),
            ("Offers".to_string(), Tag::Compound(Compound::from([("Recipes".to_string(), list(TagType::Compound, vec![Tag::Compound(recipe)]))]))),
        ]);
        assert!(entity(&mut villager, &mut notes));
        let trade = villager.get("Offers").unwrap().get("Recipes").and_then(Tag::as_list).unwrap().get(0).unwrap().clone();
        assert_eq!(trade.get("buy").unwrap().get("id"), Some(&s("minecraft:bread")));
        assert_eq!(trade.get("sell").unwrap().get("id"), Some(&s("minecraft:emerald")));
        assert_eq!(trade.get("uses"), Some(&Tag::Int(2)));

        // Bedrock, two dirt, grass, in plains.
        let (layers, biome) = flat_preset("2;7,2*3,2;1;village");
        assert_eq!(layers, vec![("minecraft:bedrock".to_string(), 1), ("minecraft:dirt".to_string(), 2), ("minecraft:grass_block".to_string(), 1)]);
        assert_eq!(biome, "minecraft:plains");
        let dims = dimensions("flat", "2;7,2*3,2;1;village", &mut notes);
        assert_eq!(dims.get("minecraft:overworld").unwrap().get("generator").unwrap().get("type"), Some(&s("minecraft:flat")));
        assert!(dimensions("default", "", &mut notes).get("minecraft:the_nether").is_some());
    }

    #[test]
    fn stairs_and_doors_use_their_neighbours() {
        let t = LegacyTables::get();
        // Two oak stairs side by side facing east stay straight.
        let cells = |x: i32, y: i32, z: i32| -> (u8, u8) {
            if y == 0 && (0..2).contains(&x) && z == 0 { (53, 0) } else { (0, 0) }
        };
        assert_eq!(state_at(t, &cells, 0, 0, 0).properties["shape"], "straight");
        // A stair turning round a corner: east-facing with a south-facing one in front of it.
        let corner = |x: i32, y: i32, z: i32| -> (u8, u8) {
            match (x, y, z) {
                (0, 0, 0) => (53, 0),
                (1, 0, 0) => (53, 2),
                _ => (0, 0),
            }
        };
        let shape = &state_at(t, &corner, 0, 0, 0).properties["shape"];
        assert!(shape.starts_with("outer_"), "{shape}");
        // The door's hinge lives in its upper half.
        let door = |x: i32, y: i32, z: i32| -> (u8, u8) {
            match (x, y, z) {
                (0, 0, 0) => (64, 0),
                (0, 1, 0) => (64, 9),
                _ => (0, 0),
            }
        };
        assert_eq!(state_at(t, &door, 0, 0, 0).properties["hinge"], "right");
        assert_eq!(state_at(t, &door, 0, 1, 0).properties["half"], "upper");
    }
}
