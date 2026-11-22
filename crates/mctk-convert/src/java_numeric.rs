//! Java 1.8 to 1.12 as a target: numeric block ids, `Blocks` and `Data` arrays, short entity and
//! block entity names, numeric item ids with a damage value, plain-text names and numeric enchantment
//! ids. Everything arrives here in the 1.20 "classic" form and is taken back through what the game's
//! own data fixers once did the other way (the tables are in `mctk-mapping`).
//!
//! Data that has no older equivalent (attributes with newer names, components, potions before 1.9,
//! flower pot contents, ...) is left out and counted in the notes.

use std::collections::HashMap;

use mctk_java::{BlockState, BlockStorage, Chunk};
use mctk_mapping::legacy::legacy;
use mctk_nbt::{Compound, List, NamedTag, Tag, TagType};

use crate::java_items::Notes;
use crate::java_old::biome_id;

/// The `DataVersion` of 1.12.2: this and anything older is written with numeric ids.
pub const LAST_NUMERIC: i32 = 1343;

pub fn is_numeric(data_version: i32) -> bool {
    data_version <= LAST_NUMERIC
}

/// 8 for 1.8, 9 for 1.9, and so on to 12.
pub fn era(data_version: i32) -> u8 {
    match data_version {
        ..=0 => 8,
        1..=184 => 9,
        185..=512 => 10,
        513..=922 => 11,
        _ => 12,
    }
}

fn max_item(era: u8) -> u16 {
    match era {
        8 => 431,
        9 | 10 => 448,
        11 => 452,
        _ => 453,
    }
}

/// The highest status effect id each era has.
fn max_effect(era: u8) -> i64 {
    match era {
        8 => 23,
        9 => 25,
        _ => 27,
    }
}

fn list(ty: TagType, tags: Vec<Tag>) -> Tag {
    Tag::List(List::from_tags(ty, tags).expect("uniform"))
}

fn num(c: &Compound, k: &str) -> Option<i64> {
    c.get(k).and_then(Tag::as_i64)
}

/// The text of a JSON text component, without its formatting.
pub fn plain_text(t: &Tag) -> String {
    fn walk(v: &serde_json::Value, out: &mut String) {
        match v {
            serde_json::Value::String(s) => out.push_str(s),
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            serde_json::Value::Object(o) => {
                if let Some(serde_json::Value::String(s)) = o.get("text") {
                    out.push_str(s);
                }
                if let Some(e) = o.get("extra") {
                    walk(e, out);
                }
            }
            _ => {}
        }
    }
    let Some(s) = t.as_str() else { return String::new() };
    match serde_json::from_str::<serde_json::Value>(s) {
        Ok(v) => {
            let mut out = String::new();
            walk(&v, &mut out);
            out
        }
        Err(_) => s.to_string(),
    }
}

// ---- items -----------------------------------------------------------------------------------

/// (name, numeric id, first era that has it)
const ENCHANTMENTS: &[(&str, i16, u8)] = &[
    ("protection", 0, 8), ("fire_protection", 1, 8), ("feather_falling", 2, 8), ("blast_protection", 3, 8),
    ("projectile_protection", 4, 8), ("respiration", 5, 8), ("aqua_affinity", 6, 8), ("thorns", 7, 8),
    ("depth_strider", 8, 8), ("frost_walker", 9, 9), ("binding_curse", 10, 11), ("sharpness", 16, 8),
    ("smite", 17, 8), ("bane_of_arthropods", 18, 8), ("knockback", 19, 8), ("fire_aspect", 20, 8),
    ("looting", 21, 8), ("sweeping", 22, 11), ("efficiency", 32, 8), ("silk_touch", 33, 8),
    ("unbreaking", 34, 8), ("fortune", 35, 8), ("power", 48, 8), ("punch", 49, 8), ("flame", 50, 8),
    ("infinity", 51, 8), ("luck_of_the_sea", 61, 8), ("lure", 62, 8), ("mending", 70, 9),
    ("vanishing_curse", 71, 11),
];

fn enchantments(t: &Tag, era: u8, notes: &mut Notes) -> Tag {
    let mut out = Vec::new();
    if let Some(l) = t.as_list() {
        for e in l.iter().filter_map(Tag::as_compound) {
            let name = e.get("id").and_then(Tag::as_str).unwrap_or("").trim_start_matches("minecraft:");
            match ENCHANTMENTS.iter().find(|(n, _, first)| *n == name && *first <= era) {
                Some((_, id, _)) => out.push(Tag::Compound(Compound::from([
                    ("id".to_string(), Tag::Short(*id)),
                    ("lvl".to_string(), Tag::Short(num(e, "lvl").unwrap_or(1) as i16)),
                ]))),
                None => notes.add(format!("enchantment {name} does not exist before 1.{era}")),
            }
        }
    }
    list(TagType::Compound, out)
}
/// The id an old item is saved with: numeric in 1.8, a name from 1.9.
fn item_id(numeric: u16, era: u8) -> Tag {
    if era == 8 {
        Tag::Short(numeric as i16)
    } else {
        Tag::String(legacy().block_name(numeric).unwrap_or("minecraft:air").to_string())
    }
}

/// An item stack in the classic form as the older one. `false` when the version has no such item.
pub fn item(it: &mut Compound, era: u8, notes: &mut Notes) -> bool {
    let name = it.get("id").and_then(Tag::as_str).unwrap_or("minecraft:air").to_string();
    if name == "minecraft:air" {
        return false;
    }
    let Some((nid, variant)) = legacy().numeric_item(&name) else {
        notes.add(format!("item {name} does not exist before 1.13"));
        return false;
    };
    if nid > max_item(era) && !(2256..=2267).contains(&nid) {
        notes.add(format!("item {name} does not exist before 1.{era}"));
        return false;
    }
    it.insert("id".into(), item_id(nid, era));
    let mut damage = variant as i64;
    if let Some(Tag::Compound(tag)) = it.get_mut("tag") {
        if let Some(d) = tag.shift_remove("Damage").and_then(|d| d.as_i64()) {
            if damage == 0 {
                damage = d;
            }
        }
        for (from, to) in [("Enchantments", "ench"), ("StoredEnchantments", "StoredEnchantments")] {
            if let Some(e) = tag.shift_remove(from) {
                let converted = enchantments(&e, era, notes);
                tag.insert(to.into(), converted);
            }
        }
        if let Some(Tag::Compound(display)) = tag.get_mut("display") {
            if let Some(n) = display.get("Name").map(plain_text) {
                display.insert("Name".into(), Tag::String(n));
            }
            if let Some(Tag::List(lore)) = display.get("Lore") {
                let plain = lore.iter().map(|l| Tag::String(plain_text(l))).collect();
                display.insert("Lore".into(), list(TagType::String, plain));
            }
        }
        if era == 8 {
            tag.shift_remove("Potion");
        }
        tag.shift_remove("CustomPotionEffects");
    }
    it.insert("Damage".into(), Tag::Short(damage as i16));
    true
}

fn items_in(c: &mut Compound, key: &str, era: u8, notes: &mut Notes) {
    match c.get_mut(key) {
        Some(Tag::List(l)) => {
            let kept: Vec<Tag> = l
                .iter()
                .filter_map(|t| {
                    let mut it = t.as_compound()?.clone();
                    item(&mut it, era, notes).then(|| Tag::Compound(it))
                })
                .collect();
            *l = List::from_tags(TagType::Compound, kept).expect("compounds");
        }
        Some(Tag::Compound(it)) => {
            if !item(it, era, notes) {
                c.shift_remove(key);
            }
        }
        _ => {}
    }
}

// ---- entities --------------------------------------------------------------------------------

/// The id an entity has in this era: a short name before 1.11, a lower-case id from it.
fn entity_id(old: &str, era: u8) -> String {
    if era < 11 {
        return old.to_string();
    }
    const SPECIAL: [(&str, &str); 22] = [
        ("XPOrb", "xp_orb"), ("EnderCrystal", "ender_crystal"), ("ThrownEnderpearl", "ender_pearl"),
        ("EyeOfEnderSignal", "eye_of_ender_signal"), ("ThrownPotion", "potion"),
        ("ThrownExpBottle", "xp_bottle"), ("FireworksRocketEntity", "fireworks_rocket"),
        ("PrimedTnt", "tnt"), ("FallingSand", "falling_block"), ("MinecartRideable", "minecart"),
        ("MinecartChest", "chest_minecart"), ("MinecartFurnace", "furnace_minecart"),
        ("MinecartTNT", "tnt_minecart"), ("MinecartHopper", "hopper_minecart"),
        ("MinecartSpawner", "spawner_minecart"), ("MinecartCommandBlock", "commandblock_minecart"),
        ("LavaSlime", "magma_cube"), ("PigZombie", "zombie_pigman"), ("MushroomCow", "mooshroom"),
        ("SnowMan", "snowman"), ("Ozelot", "ocelot"), ("WitherBoss", "wither"),
    ];
    if let Some((_, id)) = SPECIAL.iter().find(|(o, _)| *o == old) {
        return format!("minecraft:{id}");
    }
    let id = match old {
        "VillagerGolem" => "villager_golem".to_string(),
        "ThrownEgg" => "egg".to_string(),
        "EntityHorse" => "horse".to_string(),
        other => {
            let mut s = String::new();
            for (i, c) in other.chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    s.push('_');
                }
                s.extend(c.to_lowercase());
            }
            s
        }
    };
    format!("minecraft:{id}")
}

/// Horses, undead and the like that the older versions kept as one entity with a type.
fn folded(id: &str, era: u8) -> Option<(&'static str, &'static str, i64)> {
    if era >= 11 {
        return None;
    }
    Some(match id {
        "minecraft:donkey" => ("EntityHorse", "Type", 1),
        "minecraft:mule" => ("EntityHorse", "Type", 2),
        "minecraft:zombie_horse" => ("EntityHorse", "Type", 3),
        "minecraft:skeleton_horse" => ("EntityHorse", "Type", 4),
        "minecraft:wither_skeleton" => ("Skeleton", "SkeletonType", 1),
        "minecraft:stray" if era < 10 => ("Skeleton", "SkeletonType", 2),
        "minecraft:husk" if era < 10 => ("Zombie", "ZombieType", 6),
        "minecraft:zombie_villager" => ("Zombie", "ZombieType", 1),
        "minecraft:elder_guardian" => ("Guardian", "Elder", 1),
        _ => return None,
    })
}

fn uuid_parts(c: &mut Compound) {
    if let Some(Tag::IntArray(u)) = c.shift_remove("UUID") {
        if u.len() == 4 {
            let most = ((u[0] as i64) << 32) | (u[1] as u32 as i64);
            let least = ((u[2] as i64) << 32) | (u[3] as u32 as i64);
            c.insert("UUIDMost".into(), Tag::Long(most));
            c.insert("UUIDLeast".into(), Tag::Long(least));
        }
    }
}

const PAINTINGS: [(&str, &str); 4] = [
    ("skull_and_roses", "SkullAndRoses"),
    ("burning_skull", "BurningSkull"),
    ("donkey_kong", "DonkeyKong"),
    ("pigscene", "Pigscene"),
];

/// Converts an entity in the classic form (already checked against the target's entity list) to the
/// form of this era. `false` when it cannot be carried over.
pub fn entity(e: &mut Compound, era: u8, notes: &mut Notes) -> bool {
    let id = e.get("id").and_then(Tag::as_str).unwrap_or("").to_string();
    let leg = legacy();
    let (old, fold) = match folded(&id, era) {
        Some((old, key, v)) => (old.to_string(), Some((key, v))),
        None => match id.as_str() {
            "minecraft:boat" | "minecraft:oak_boat" | "minecraft:spruce_boat" | "minecraft:birch_boat" | "minecraft:jungle_boat" | "minecraft:acacia_boat" | "minecraft:dark_oak_boat" => ("Boat".to_string(), None),
            _ => match leg.old_entity_name(&id) {
                Some(o) => (o.to_string(), None),
                None => {
                    notes.add(format!("entity {id} does not exist before 1.13"));
                    return false;
                }
            },
        },
    };
    e.insert("id".into(), Tag::String(entity_id(&old, era)));
    if let Some((key, v)) = fold {
        match key {
            "Elder" => e.insert("Elder".into(), Tag::Byte(1)),
            "ZombieType" if era < 9 => e.insert("IsVillager".into(), Tag::Byte(1)),
            _ => e.insert(key.into(), Tag::Int(v as i32)),
        };
    }
    uuid_parts(e);
    if let Some(n) = e.get("CustomName").map(plain_text) {
        e.insert("CustomName".into(), Tag::String(n));
    }
    // Hit points are a short in 1.8.
    if era == 8 {
        if let Some(h) = e.get("Health").and_then(Tag::as_f64) {
            e.insert("Health".into(), Tag::Short(h as i16));
        }
    }
    if e.shift_remove("Attributes").is_some() {
        notes.add("entity attributes are left to the game before 1.13");
    }
    if let Some(Tag::List(effects)) = e.get("ActiveEffects") {
        let kept: Vec<Tag> = effects
            .iter()
            .filter(|t| t.as_compound().and_then(|c| num(c, "Id")).is_some_and(|i| i <= max_effect(era)))
            .cloned()
            .collect();
        e.insert("ActiveEffects".into(), list(TagType::Compound, kept));
    }
    // Equipment: 1.8 keeps one list of five (hand, boots, leggings, chestplate, helmet).
    for key in ["HandItems", "ArmorItems"] {
        if let Some(Tag::List(l)) = e.get_mut(key) {
            let kept: Vec<Tag> = l
                .iter()
                .map(|t| {
                    let mut it = t.as_compound().cloned().unwrap_or_default();
                    if !it.is_empty() && !item(&mut it, era, notes) {
                        it.clear();
                    }
                    Tag::Compound(it)
                })
                .collect();
            *l = List::from_tags(TagType::Compound, kept).expect("compounds");
        }
    }
    if era == 8 {
        let hand = match e.shift_remove("HandItems") {
            Some(Tag::List(l)) => l.iter().next().cloned(),
            _ => None,
        };
        let armor: Vec<Tag> = match e.shift_remove("ArmorItems") {
            Some(Tag::List(l)) => l.iter().cloned().collect(),
            _ => Vec::new(),
        };
        if hand.is_some() || !armor.is_empty() {
            let mut all = vec![hand.unwrap_or_else(|| Tag::Compound(Compound::new()))];
            all.extend(armor);
            while all.len() < 5 {
                all.push(Tag::Compound(Compound::new()));
            }
            e.insert("Equipment".into(), list(TagType::Compound, all));
        }
        e.shift_remove("HandDropChances");
        e.shift_remove("ArmorDropChances");
    }
    for key in ["Item", "SaddleItem", "ArmorItem", "DecorItem"] {
        items_in(e, key, era, notes);
    }
    // A dropped item whose item does not exist here is nothing.
    if matches!(id.as_str(), "minecraft:item") && !e.contains_key("Item") {
        return false;
    }
    for key in ["Items", "Inventory"] {
        items_in(e, key, era, notes);
    }
    if let Some(Tag::Compound(offers)) = e.get_mut("Offers") {
        if let Some(Tag::List(recipes)) = offers.get_mut("Recipes") {
            for r in recipes.iter_mut().filter_map(Tag::as_compound_mut) {
                for k in ["buy", "buyB", "sell"] {
                    items_in(r, k, era, notes);
                }
            }
        }
    }
    // Villagers: a profession number rather than villager data.
    if let Some(Tag::Compound(v)) = e.shift_remove("VillagerData") {
        let profession = v.get("profession").and_then(Tag::as_str).unwrap_or("").trim_start_matches("minecraft:").to_string();
        let p = match profession.as_str() {
            "farmer" | "fisherman" | "shepherd" | "fletcher" => 0,
            "librarian" | "cartographer" => 1,
            "cleric" => 2,
            "armorer" | "weaponsmith" | "toolsmith" => 3,
            "butcher" | "leatherworker" => 4,
            _ => 0,
        };
        e.insert("Profession".into(), Tag::Int(p));
    }
    match id.as_str() {
        "minecraft:painting" => {
            if let Some(v) = e.shift_remove("variant").or_else(|| e.shift_remove("Motive")) {
                let bare = v.as_str().unwrap_or("kebab").trim_start_matches("minecraft:").to_string();
                let name = PAINTINGS.iter().find(|(k, _)| *k == bare).map(|(_, n)| (*n).to_string()).unwrap_or_else(|| {
                    let mut c = bare.chars();
                    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
                });
                e.insert("Motive".into(), Tag::String(name));
            }
            if let Some(f) = e.get("facing").cloned() {
                e.insert("Facing".into(), f);
            }
        }
        "minecraft:item_frame" | "minecraft:glow_item_frame" => {
            if let Some(f) = e.get("Facing").or_else(|| e.get("facing")).cloned() {
                e.insert("Facing".into(), f);
            }
        }
        "minecraft:falling_block" => {
            if let Some(Tag::Compound(bs)) = e.shift_remove("BlockState") {
                let name = bs.get("Name").and_then(Tag::as_str).unwrap_or("minecraft:sand");
                let mut state = BlockState::new(name);
                if let Some(Tag::Compound(p)) = bs.get("Properties") {
                    for (k, v) in p {
                        if let Some(v) = v.as_str() {
                            state = state.with(k, v);
                        }
                    }
                }
                if let Some((nid, data)) = leg.numeric_state(&state) {
                    e.insert("Block".into(), Tag::String(leg.block_name(nid).unwrap_or("minecraft:sand").to_string()));
                    e.insert("TileID".into(), Tag::Int(nid as i32));
                    e.insert("Data".into(), Tag::Byte(data as i8));
                }
            }
        }
        _ => {}
    }
    true
}

// ---- block entities --------------------------------------------------------------------------

const COLOURS: [&str; 16] = [
    "black", "red", "green", "brown", "blue", "purple", "cyan", "light_gray", "gray", "pink", "lime",
    "yellow", "light_blue", "magenta", "orange", "white",
];

fn colour_of(name: &str) -> Option<i32> {
    let bare = name.trim_start_matches("minecraft:");
    COLOURS.iter().position(|c| bare.starts_with(&format!("{c}_"))).map(|i| i as i32)
}

/// The old id of a block entity in this era.
fn tile_id(current: &str, era: u8) -> Option<String> {
    let bare = current.trim_start_matches("minecraft:");
    const SHORT: [(&str, &str); 24] = [
        ("furnace", "Furnace"), ("chest", "Chest"), ("trapped_chest", "Chest"), ("ender_chest", "EnderChest"),
        ("jukebox", "RecordPlayer"), ("dispenser", "Trap"), ("dropper", "Dropper"), ("sign", "Sign"),
        ("mob_spawner", "MobSpawner"), ("note_block", "Music"), ("piston", "Piston"),
        ("brewing_stand", "Cauldron"), ("enchanting_table", "EnchantTable"), ("end_portal", "Airportal"),
        ("command_block", "Control"), ("beacon", "Beacon"), ("skull", "Skull"),
        ("daylight_detector", "DLDetector"), ("hopper", "Hopper"), ("comparator", "Comparator"),
        ("flower_pot", "FlowerPot"), ("banner", "Banner"), ("end_gateway", "EndGateway"),
        ("structure_block", "Structure"),
    ];
    if era >= 11 {
        let id = match bare {
            "trapped_chest" => "chest",
            "note_block" => "noteblock",
            other => other,
        };
        return SHORT.iter().any(|(m, _)| *m == bare).then(|| format!("minecraft:{id}")).or_else(|| {
            matches!(bare, "shulker_box" | "bed").then(|| format!("minecraft:{bare}"))
        });
    }
    SHORT.iter().find(|(m, _)| *m == bare).map(|(_, o)| (*o).to_string())
}

/// A block entity in the classic form as this era writes it. `block` is the block at its
/// position (its colour and kind are not in the block entity any more). `false` when it has no
/// older equivalent.
pub fn tile_entity(be: &mut Compound, block: Option<&BlockState>, era: u8, notes: &mut Notes) -> bool {
    let current = be.get("id").and_then(Tag::as_str).unwrap_or("").to_string();
    let Some(new_id) = tile_id(&current, era) else {
        notes.add(format!("block entity {current} does not exist before 1.13"));
        return false;
    };
    be.insert("id".into(), Tag::String(new_id));
    if let Some(n) = be.get("CustomName").map(plain_text) {
        be.insert("CustomName".into(), Tag::String(n));
    }
    items_in(be, "Items", era, notes);
    let bare = current.trim_start_matches("minecraft:");
    match bare {
        "jukebox" => {
            if let Some(Tag::Compound(rec)) = be.get_mut("RecordItem") {
                let name = rec.get("id").and_then(Tag::as_str).unwrap_or("").to_string();
                let numeric = legacy().numeric_item(&name).map(|(i, _)| i).unwrap_or(0);
                if item(rec, era, notes) {
                    be.insert("Record".into(), Tag::Int(numeric as i32));
                }
            }
        }
        "mob_spawner" => {
            let fix = |data: &mut Compound, notes: &mut Notes| -> Option<String> {
                let id = data.get("id").and_then(Tag::as_str)?.to_string();
                let legacy_name = match folded(&id, era) {
                    Some((old, _, _)) => old.to_string(),
                    None => legacy().old_entity_name(&id).map(str::to_string).unwrap_or_else(|| {
                        notes.add(format!("spawner entity {id} does not exist before 1.13"));
                        "Pig".to_string()
                    }),
                };
                let out = entity_id(&legacy_name, era);
                data.insert("id".into(), Tag::String(out.clone()));
                Some(out)
            };
            if let Some(Tag::Compound(d)) = be.get_mut("SpawnData") {
                if let Some(id) = fix(d, notes) {
                    if era < 11 {
                        be.insert("EntityId".into(), Tag::String(id));
                    }
                }
            }
            if let Some(Tag::List(p)) = be.get_mut("SpawnPotentials") {
                for entry in p.iter_mut().filter_map(Tag::as_compound_mut) {
                    if let Some(Tag::Compound(ent)) = entry.get_mut("Entity") {
                        fix(ent, notes);
                    }
                }
            }
        }
        "skull" => {
            let kind = block.map_or(0, |b| match b.name.trim_start_matches("minecraft:") {
                n if n.starts_with("wither_skeleton") => 1,
                n if n.starts_with("zombie") => 2,
                n if n.starts_with("player") => 3,
                n if n.starts_with("creeper") => 4,
                n if n.starts_with("dragon") => 5,
                _ => 0,
            });
            be.insert("SkullType".into(), Tag::Byte(kind));
            let rot = block.and_then(|b| b.properties.get("rotation")).and_then(|r| r.parse::<i8>().ok()).unwrap_or(0);
            be.insert("Rot".into(), Tag::Byte(rot));
            if let Some(profile) = be.shift_remove("SkullOwner") {
                if let Some(c) = profile.as_compound() {
                    let mut owner = Compound::new();
                    for k in ["Id", "Name", "Properties"] {
                        if let Some(v) = c.get(k) {
                            owner.insert(k.into(), v.clone());
                        }
                    }
                    be.insert("Owner".into(), Tag::Compound(owner));
                }
            }
        }
        "banner" => {
            let base = block.and_then(|b| colour_of(&b.name)).unwrap_or(15);
            be.insert("Base".into(), Tag::Int(base));
            // Pattern colours count from white in the newer form and from black in the older.
            if let Some(Tag::List(patterns)) = be.get_mut("Patterns") {
                for p in patterns.iter_mut().filter_map(Tag::as_compound_mut) {
                    if let Some(c) = p.get("Color").and_then(Tag::as_i64) {
                        p.insert("Color".into(), Tag::Int(15 - c as i32));
                    }
                }
            }
        }
        "bed" => {
            // A bed's colour is the dye's number counted from white (red is 14).
            let c = block.and_then(|b| colour_of(&b.name)).map_or(14, |d| 15 - d);
            be.insert("color".into(), Tag::Int(c));
        }
        "flower_pot" => {
            be.insert("Item".into(), Tag::String("minecraft:air".into()));
            be.insert("Data".into(), Tag::Int(0));
        }
        _ => {}
    }
    for k in ["components", "keepPacked", "ForgeData"] {
        be.shift_remove(k);
    }
    true
}

// ---- players ---------------------------------------------------------------------------------

/// A player file in the classic form as this era writes it.
pub fn player(p: &mut Compound, era: u8, notes: &mut Notes) {
    uuid_parts(p);
    for key in ["Inventory", "EnderItems"] {
        if let Some(Tag::List(l)) = p.get_mut(key) {
            let kept: Vec<Tag> = l
                .iter()
                .filter_map(|t| {
                    let mut it = t.as_compound()?.clone();
                    // The off hand does not exist in 1.8.
                    if era == 8 && num(&it, "Slot") == Some(-106) {
                        return None;
                    }
                    item(&mut it, era, notes).then(|| Tag::Compound(it))
                })
                .collect();
            *l = List::from_tags(TagType::Compound, kept).expect("compounds");
        }
    }
    if let Some(Tag::String(d)) = p.get("Dimension") {
        let n = match d.trim_start_matches("minecraft:") {
            "the_nether" => -1,
            "the_end" => 1,
            _ => 0,
        };
        p.insert("Dimension".into(), Tag::Int(n));
    }
    if p.shift_remove("Attributes").is_some() {
        notes.add("player attributes are left to the game before 1.13");
    }
    for k in ["recipeBook", "respawn", "SpawnDimension", "SpawnForced", "warden_spawn_tracker", "LastDeathLocation", "current_explosion_impact_pos"] {
        p.shift_remove(k);
    }
}

// ---- chunks ----------------------------------------------------------------------------------

fn nibbles(values: &[u8]) -> Vec<i8> {
    values.chunks(2).map(|c| ((c[0] & 15) | (c[1] << 4)) as i8).collect()
}

/// What a flower pot holds, by the block that is a pot with it in the newer versions: the item
/// and its data value.
fn pot_contents(name: &str) -> (&'static str, i32) {
    match name.trim_start_matches("minecraft:potted_") {
        "poppy" => ("minecraft:red_flower", 0),
        "blue_orchid" => ("minecraft:red_flower", 1),
        "allium" => ("minecraft:red_flower", 2),
        "azure_bluet" => ("minecraft:red_flower", 3),
        "red_tulip" => ("minecraft:red_flower", 4),
        "orange_tulip" => ("minecraft:red_flower", 5),
        "white_tulip" => ("minecraft:red_flower", 6),
        "pink_tulip" => ("minecraft:red_flower", 7),
        "oxeye_daisy" => ("minecraft:red_flower", 8),
        "dandelion" => ("minecraft:yellow_flower", 0),
        "oak_sapling" => ("minecraft:sapling", 0),
        "spruce_sapling" => ("minecraft:sapling", 1),
        "birch_sapling" => ("minecraft:sapling", 2),
        "jungle_sapling" => ("minecraft:sapling", 3),
        "acacia_sapling" => ("minecraft:sapling", 4),
        "dark_oak_sapling" => ("minecraft:sapling", 5),
        "red_mushroom" => ("minecraft:red_mushroom", 0),
        "brown_mushroom" => ("minecraft:brown_mushroom", 0),
        "cactus" => ("minecraft:cactus", 0),
        "dead_bush" => ("minecraft:deadbush", 0),
        "fern" => ("minecraft:tallgrass", 2),
        _ => ("minecraft:air", 0),
    }
}

/// The numeric id and data of the blocks whose state lives in a block entity of the old versions.
fn special_numeric(s: &BlockState) -> Option<(u16, u8)> {
    let name = s.name.trim_start_matches("minecraft:");
    if name.ends_with("_wall_skull") || name.ends_with("_wall_head") {
        let data = match s.properties.get("facing").map(String::as_str) {
            Some("north") => 2,
            Some("south") => 3,
            Some("west") => 4,
            _ => 5,
        };
        return Some((144, data));
    }
    if name.ends_with("_skull") || name.ends_with("_head") {
        return Some((144, 1));
    }
    (name == "flower_pot" || name.starts_with("potted_")).then_some((140, 0))
}

/// A state for a numeric lookup: the colours and kinds that the old versions keep in block
/// entities are folded into the one block they had.
fn base_state(s: &BlockState) -> BlockState {
    let name = s.name.trim_start_matches("minecraft:");
    let fold = |suffix: &str, to: &str| -> Option<BlockState> {
        name.ends_with(suffix).then(|| {
            let mut b = BlockState::new(format!("minecraft:{to}"));
            b.properties = s.properties.clone();
            b
        })
    };
    // Wall variants keep their `facing`; the standing ones their `rotation`.
    if name.ends_with("_wall_banner") {
        return fold("_wall_banner", "white_wall_banner").unwrap();
    }
    if name.ends_with("_banner") {
        return fold("_banner", "white_banner").unwrap();
    }
    if name.ends_with("_bed") {
        return fold("_bed", "red_bed").unwrap();
    }
    for (suffix, to) in [("_wall_skull", "skeleton_wall_skull"), ("_wall_head", "skeleton_wall_skull")] {
        if name.ends_with(suffix) {
            return fold(suffix, to).unwrap();
        }
    }
    for suffix in ["_skull", "_head"] {
        if name.ends_with(suffix) && !name.contains("wall") {
            return fold(suffix, "skeleton_skull").unwrap();
        }
    }
    let renamed = match name {
        "grass" => "short_grass",
        "grass_path" => "dirt_path",
        "chain" => "iron_chain",
        other => other,
    };
    let mut out = s.clone();
    out.name = format!("minecraft:{renamed}");
    out
}

/// The numeric chunk for a converted one: its blocks and biomes (`chunk`), its block entities
/// (`top`, in the classic form) and the entities that belong inside it.
pub fn chunk(chunk: &Chunk, top: &Compound, data_version: i32, entities: Vec<Tag>, notes: &mut Notes) -> NamedTag {
    let era = era(data_version);
    let leg = legacy();
    let mut sections = Vec::new();
    let mut cells = [[1i32; 64]; 16];
    // Highest block per column, for the height map, and every state at a position, for the
    // block entities.
    let mut heights = [0i32; 256];
    let mut blocks_at: HashMap<(i32, i32, i32), BlockState> = HashMap::new();
    let wanted: Vec<(i32, i32, i32)> = match top.get("block_entities") {
        Some(Tag::List(l)) => l
            .iter()
            .filter_map(Tag::as_compound)
            .filter_map(|c| Some((num(c, "x")? as i32, num(c, "y")? as i32, num(c, "z")? as i32)))
            .collect(),
        _ => Vec::new(),
    };
    let mut cut = false;
    let mut unmapped = 0u64;
    let mut pots: Vec<Compound> = Vec::new();
    for s in &chunk.sections {
        if !(0..16).contains(&s.y) {
            if matches!(&s.blocks, Some(BlockStorage::Palette(p)) if p.palette.iter().any(|b| b.name != "minecraft:air" && b.name != "minecraft:cave_air" && b.name != "minecraft:void_air")) {
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
        let mapped: Vec<(u16, u8)> = p
            .palette
            .iter()
            .map(|st| {
                if matches!(st.name.as_str(), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air") {
                    return (0, 0);
                }
                special_numeric(st).or_else(|| leg.numeric_state(&base_state(st))).unwrap_or_else(|| {
                    unmapped += 1;
                    (0, 0)
                })
            })
            .collect();
        if mapped.iter().all(|(id, _)| *id == 0) {
            continue;
        }
        let (mut ids, mut data) = (vec![0u8; 4096], vec![0u8; 4096]);
        for (i, idx) in p.indices.iter().enumerate() {
            let (id, d) = mapped[*idx as usize];
            ids[i] = id as u8;
            data[i] = d;
            if id != 0 {
                let (x, y, z) = ((i % 16) as i32, (i / 256) as i32, ((i / 16) % 16) as i32);
                let h = &mut heights[(z * 16 + x) as usize];
                *h = (*h).max(s.y as i32 * 16 + y + 1);
                let (wx, wy, wz) = (chunk.x * 16 + x, s.y as i32 * 16 + y, chunk.z * 16 + z);
                if wanted.contains(&(wx, wy, wz)) {
                    blocks_at.insert((wx, wy, wz), p.palette[*idx as usize].clone());
                }
                let name = &p.palette[*idx as usize].name;
                if name.starts_with("minecraft:potted_") {
                    let (item, data) = pot_contents(name);
                    pots.push(Compound::from([
                        ("id".to_string(), Tag::String(tile_id("flower_pot", era).unwrap_or_default())),
                        ("x".to_string(), Tag::Int(wx)),
                        ("y".to_string(), Tag::Int(wy)),
                        ("z".to_string(), Tag::Int(wz)),
                        ("Item".to_string(), Tag::String(item.to_string())),
                        ("Data".to_string(), Tag::Int(data)),
                    ]));
                }
            }
        }
        sections.push(Tag::Compound(Compound::from([
            ("Y".to_string(), Tag::Byte(s.y)),
            ("Blocks".to_string(), Tag::ByteArray(ids.iter().map(|b| *b as i8).collect())),
            ("Data".to_string(), Tag::ByteArray(nibbles(&data))),
            ("BlockLight".to_string(), Tag::ByteArray(vec![0; 2048])),
            ("SkyLight".to_string(), Tag::ByteArray(vec![0; 2048])),
        ])));
    }
    if cut {
        notes.add("blocks above 255 or below 0 do not exist before 1.18");
    }
    if unmapped > 0 {
        notes.add("block states with no numeric equivalent were written as air");
    }

    let mut tiles = Vec::new();
    if let Some(Tag::List(l)) = top.get("block_entities") {
        for t in l.iter() {
            let Some(mut be) = t.as_compound().cloned() else { continue };
            let pos = (num(&be, "x").unwrap_or(0) as i32, num(&be, "y").unwrap_or(0) as i32, num(&be, "z").unwrap_or(0) as i32);
            if !(0..256).contains(&pos.1) {
                continue;
            }
            if tile_entity(&mut be, blocks_at.get(&pos), era, notes) {
                tiles.push(Tag::Compound(be));
            }
        }
    }

    tiles.extend(pots.into_iter().map(Tag::Compound));
    let mut level = Compound::new();
    if era == 8 {
        level.insert("V".into(), Tag::Byte(1));
    }
    level.insert("xPos".into(), Tag::Int(chunk.x));
    level.insert("zPos".into(), Tag::Int(chunk.z));
    level.insert("LastUpdate".into(), Tag::Long(top.get("LastUpdate").and_then(Tag::as_i64).unwrap_or(0)));
    level.insert("InhabitedTime".into(), Tag::Long(top.get("InhabitedTime").and_then(Tag::as_i64).unwrap_or(0)));
    level.insert("TerrainPopulated".into(), Tag::Byte(1));
    level.insert("LightPopulated".into(), Tag::Byte(0));
    let biomes: Vec<i8> = (0..256).map(|i| cells[4][((i / 16) / 4) * 4 + (i % 16) / 4] as i8).collect();
    level.insert("Biomes".into(), Tag::ByteArray(biomes));
    level.insert("HeightMap".into(), Tag::IntArray(heights.to_vec()));
    level.insert("Sections".into(), list(TagType::Compound, sections));
    level.insert("Entities".into(), list(TagType::Compound, entities));
    level.insert("TileEntities".into(), list(TagType::Compound, tiles));
    let mut root = Compound::new();
    if data_version > 0 {
        root.insert("DataVersion".into(), Tag::Int(data_version));
    }
    root.insert("Level".into(), Tag::Compound(level));
    NamedTag { name: String::new(), tag: Tag::Compound(root) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eras_follow_the_data_versions() {
        assert_eq!(era(0), 8);
        assert_eq!(era(184), 9);
        assert_eq!(era(512), 10);
        assert_eq!(era(922), 11);
        assert_eq!(era(1343), 12);
    }

    #[test]
    fn text_is_flattened() {
        let t = Tag::String(r#"{"text":"Hi ","extra":[{"text":"there","bold":true}]}"#.into());
        assert_eq!(plain_text(&t), "Hi there");
    }

    #[test]
    fn a_sword_gets_its_numeric_form() {
        let mut it = Compound::from([
            ("id".to_string(), Tag::String("minecraft:diamond_sword".into())),
            ("Count".to_string(), Tag::Byte(1)),
            (
                "tag".to_string(),
                Tag::Compound(Compound::from([
                    ("Damage".to_string(), Tag::Int(5)),
                    (
                        "Enchantments".to_string(),
                        list(TagType::Compound, vec![Tag::Compound(Compound::from([
                            ("id".to_string(), Tag::String("minecraft:sharpness".into())),
                            ("lvl".to_string(), Tag::Short(3)),
                        ]))]),
                    ),
                ])),
            ),
        ]);
        let mut notes = Notes::default();
        assert!(item(&mut it, 12, &mut notes), "{notes:?}");
        assert_eq!(it.get("id"), Some(&Tag::String("minecraft:diamond_sword".into())));
        assert_eq!(it.get("Damage"), Some(&Tag::Short(5)));
        let ench = it.get("tag").and_then(|t| t.get("ench")).and_then(Tag::as_list).unwrap();
        assert_eq!(ench.get(0).and_then(|e| e.get("id")), Some(&Tag::Short(16)));
        let mut one_eight = it.clone();
        one_eight.insert("id".into(), Tag::String("minecraft:diamond_sword".into()));
        assert!(item(&mut one_eight, 8, &mut notes));
        assert_eq!(one_eight.get("id"), Some(&Tag::Short(276)));
    }
}
