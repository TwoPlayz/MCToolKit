//! The other block entity kinds: banners, skulls, beds, flower pots, jukeboxes, lecterns, beacons,
//! brewing stands, command blocks, decorated pots, chiseled bookshelves, crafters, beehives, vaults,
//! trial spawners, brushable blocks, and the data-less ones (bells, conduits, sculk blocks, ...).
//!
//! Shapes of the Bedrock side were read from entities the Bedrock server saved when the blocks were
//! placed; the Java side follows the game's own field names. Some Java
//! blocks have no block entity at all but Bedrock needs one (beds, flower pots, note blocks,
//! pistons): [`from_block`] makes those from the block.

use mctk_java::BlockState;
use mctk_mapping::mappings;
use mctk_nbt::{Compound, List, Tag, TagType};

use crate::block_entities::{BlockEdit, ToJava};
use crate::item_codec::{self, Notes};
use crate::text_codec;

/// The sixteen dyes in the order both games number them.
pub const DYES: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// The skulls and heads: (Java floor block, Java wall block) in Bedrock's `SkullType` order.
const SKULLS: [(&str, &str); 7] = [
    ("minecraft:skeleton_skull", "minecraft:skeleton_wall_skull"),
    ("minecraft:wither_skeleton_skull", "minecraft:wither_skeleton_wall_skull"),
    ("minecraft:zombie_head", "minecraft:zombie_wall_head"),
    ("minecraft:player_head", "minecraft:player_wall_head"),
    ("minecraft:creeper_head", "minecraft:creeper_wall_head"),
    ("minecraft:dragon_head", "minecraft:dragon_wall_head"),
    ("minecraft:piglin_head", "minecraft:piglin_wall_head"),
];

/// Banner patterns: (Java id without namespace, Bedrock code).
const PATTERNS: [(&str, &str); 42] = [
    ("base", "b"),
    ("square_bottom_left", "bl"),
    ("square_bottom_right", "br"),
    ("square_top_left", "tl"),
    ("square_top_right", "tr"),
    ("stripe_bottom", "bs"),
    ("stripe_top", "ts"),
    ("stripe_left", "ls"),
    ("stripe_right", "rs"),
    ("stripe_center", "cs"),
    ("stripe_middle", "ms"),
    ("stripe_downright", "drs"),
    ("stripe_downleft", "dls"),
    ("small_stripes", "ss"),
    ("cross", "cr"),
    ("straight_cross", "sc"),
    ("triangle_bottom", "bt"),
    ("triangle_top", "tt"),
    ("triangles_bottom", "bts"),
    ("triangles_top", "tts"),
    ("diagonal_left", "ld"),
    ("diagonal_right", "rd"),
    ("diagonal_up_left", "lud"),
    ("diagonal_up_right", "rud"),
    ("circle", "mc"),
    ("rhombus", "mr"),
    ("half_vertical", "vh"),
    ("half_horizontal", "hh"),
    ("half_vertical_right", "vhr"),
    ("half_horizontal_bottom", "hhb"),
    ("border", "bo"),
    ("curly_border", "cbo"),
    ("bricks", "bri"),
    ("gradient", "gra"),
    ("gradient_up", "gru"),
    ("creeper", "cre"),
    ("skull", "sku"),
    ("flower", "flo"),
    ("mojang", "moj"),
    ("globe", "glb"),
    ("piglin", "pig"),
    ("flow", "flw"),
];

/// Effects a beacon can give, by Bedrock's effect number (1-based).
const EFFECTS: [&str; 23] = [
    "speed",
    "slowness",
    "haste",
    "mining_fatigue",
    "strength",
    "instant_health",
    "instant_damage",
    "jump_boost",
    "nausea",
    "regeneration",
    "resistance",
    "fire_resistance",
    "water_breathing",
    "invisibility",
    "blindness",
    "night_vision",
    "hunger",
    "weakness",
    "poison",
    "wither",
    "health_boost",
    "absorption",
    "saturation",
];

/// Kinds that carry no data worth converting: (Java id, Bedrock id).
const BARE: [(&str, &str); 11] = [
    ("minecraft:bell", "Bell"),
    ("minecraft:conduit", "Conduit"),
    ("minecraft:daylight_detector", "DaylightDetector"),
    ("minecraft:enchanting_table", "EnchantTable"),
    ("minecraft:end_portal", "EndPortal"),
    ("minecraft:end_gateway", "EndGateway"),
    ("minecraft:sculk_sensor", "SculkSensor"),
    ("minecraft:calibrated_sculk_sensor", "CalibratedSculkSensor"),
    ("minecraft:sculk_shrieker", "SculkShrieker"),
    ("minecraft:sculk_catalyst", "SculkCatalyst"),
    ("minecraft:beehive", "Beehive"),
];

/// Bedrock entities that only exist there; they are dropped without a note.
pub fn is_bedrock_only(id: &str) -> bool {
    matches!(id, "PistonArm" | "Lodestone" | "SporeBlossom" | "PotentSulfurBlock" | "MovingBlock" | "Bed" | "Cauldron" | "Music" | "FlowerPot")
}

/// Java entities with no Bedrock form, dropped without a note.
pub fn is_java_only(id: &str) -> bool {
    matches!(id, "DUMMY" | "minecraft:piston" | "minecraft:moving_piston")
}

fn int(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_i64)
}

fn text<'a>(c: &'a Compound, key: &str) -> Option<&'a str> {
    c.get(key).and_then(Tag::as_str)
}

fn compounds(items: Vec<Compound>) -> Tag {
    if items.is_empty() {
        return Tag::List(List::new(TagType::End));
    }
    Tag::List(List::from_tags(TagType::Compound, items.into_iter().map(Tag::Compound).collect()).expect("compounds"))
}

fn strings(items: Vec<String>) -> Tag {
    if items.is_empty() {
        return Tag::List(List::new(TagType::End));
    }
    Tag::List(List::from_tags(TagType::String, items.into_iter().map(Tag::String).collect()).expect("strings"))
}

fn list_of<'a>(c: &'a Compound, key: &str) -> Vec<&'a Tag> {
    c.get(key).and_then(Tag::as_list).map(|l| l.iter().collect()).unwrap_or_default()
}

fn bare(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

fn set_position(from: &Compound, to: &mut Compound) {
    for k in ["x", "y", "z"] {
        if let Some(v) = int(from, k) {
            to.insert(k.into(), Tag::Int(v as i32));
        }
    }
}

fn dye_index(name: &str) -> Option<usize> {
    DYES.iter().position(|d| *d == name)
}

/// The dye a block name starts with, when it ends with `suffix` (`red_bed`, `light_blue_wall_banner`).
fn dye_of(name: &str, suffix: &str) -> Option<usize> {
    bare(name).strip_suffix(suffix).and_then(dye_index)
}

fn prop<'a>(block: Option<&'a BlockState>, key: &str) -> Option<&'a str> {
    block.and_then(|b| b.properties.get(key)).map(String::as_str)
}

/// Whether [`from_block`] can make an entity for a block of this name (a cheap check for scanning
/// palettes).
pub fn generates_entity(name: &str) -> bool {
    let n = bare(name);
    n.ends_with("_bed")
        || n.ends_with("cauldron")
        || n.ends_with("_banner")
        || n.ends_with("_skull")
        || n.ends_with("_head")
        || n.starts_with("potted_")
        || matches!(n, "flower_pot" | "note_block" | "piston" | "sticky_piston" | "lodestone" | "spore_blossom")
}

fn base_entity(id: &str) -> Compound {
    let mut out = Compound::new();
    out.insert("id".into(), Tag::String(id.into()));
    out.insert("isMovable".into(), Tag::Byte(1));
    out
}

/// A Bedrock entity (without a position) for a Java block that has none on Java.
pub fn from_block(block: &BlockState) -> Option<Compound> {
    let name = bare(&block.name);
    if let Some(color) = dye_of(name, "_bed") {
        let mut out = base_entity("Bed");
        out.insert("color".into(), Tag::Byte(color as i8));
        return Some(out);
    }
    if name.ends_with("_banner") {
        return banner_from_block(block, None);
    }
    if let Some(skull) = skull_from_block(block) {
        return Some(skull);
    }
    match name {
        "cauldron" | "water_cauldron" | "lava_cauldron" | "powder_snow_cauldron" => {
            let mut out = base_entity("Cauldron");
            out.insert("Items".into(), Tag::List(List::new(TagType::End)));
            out.insert("PotionId".into(), Tag::Short(-1));
            out.insert("PotionType".into(), Tag::Short(-1));
            Some(out)
        }
        "flower_pot" => Some(base_entity("FlowerPot")),
        "note_block" => {
            let mut out = base_entity("Music");
            let note = prop(Some(block), "note").and_then(|n| n.parse::<i8>().ok()).unwrap_or(0);
            out.insert("note".into(), Tag::Byte(note));
            Some(out)
        }
        "piston" | "sticky_piston" => {
            let extended = prop(Some(block), "extended") == Some("true");
            let mut out = base_entity("PistonArm");
            out.insert("AttachedBlocks".into(), Tag::List(List::new(TagType::End)));
            out.insert("BreakBlocks".into(), Tag::List(List::new(TagType::End)));
            let progress = if extended { 1.0 } else { 0.0 };
            out.insert("LastProgress".into(), Tag::Float(progress));
            out.insert("Progress".into(), Tag::Float(progress));
            out.insert("NewState".into(), Tag::Byte(if extended { 2 } else { 0 }));
            out.insert("State".into(), Tag::Byte(if extended { 2 } else { 0 }));
            out.insert("Sticky".into(), Tag::Byte((name == "sticky_piston") as i8));
            Some(out)
        }
        "lodestone" => Some(base_entity("Lodestone")),
        "spore_blossom" => Some(base_entity("SporeBlossom")),
        n if n.starts_with("potted_") => {
            let mut out = base_entity("FlowerPot");
            if let Some(plant) = plant_block(n) {
                out.insert("PlantBlock".into(), plant);
            }
            Some(out)
        }
        _ => None,
    }
}

/// The plant a `potted_*` Java block holds, as Bedrock's `PlantBlock`.
fn plant_block(potted: &str) -> Option<Tag> {
    let plant = potted.strip_prefix("potted_")?;
    let plant = match plant {
        "azalea_bush" => "azalea",
        "flowering_azalea_bush" => "flowering_azalea",
        other => other,
    };
    let b = mappings().java_to_bedrock(&BlockState::new(format!("minecraft:{plant}")))?;
    Some(Tag::Compound(
        b.to_nbt().tag.as_compound().cloned().unwrap_or_default(),
    ))
}

/// The Java `potted_*` block that holds a Bedrock plant block.
fn potted_block(plant: &Compound) -> Option<String> {
    let name = text(plant, "name")?;
    let states = plant.get("states").and_then(Tag::as_compound).cloned().unwrap_or_default();
    let java = mappings().bedrock_to_java(name, &states)?;
    let bare_name = bare(&java.name);
    let bare_name = match bare_name {
        "azalea" => "azalea_bush",
        "flowering_azalea" => "flowering_azalea_bush",
        other => other,
    };
    let candidate = format!("minecraft:potted_{bare_name}");
    mappings().java_to_bedrock(&BlockState::new(candidate.clone())).map(|_| candidate)
}

fn skull_type_of(block: &str) -> Option<(usize, bool)> {
    SKULLS.iter().position(|(f, w)| *f == block || *w == block).map(|i| (i, SKULLS[i].1 == block))
}

fn skull_from_block(block: &BlockState) -> Option<Compound> {
    let (ty, wall) = skull_type_of(&block.name)?;
    let mut out = base_entity("Skull");
    out.insert("SkullType".into(), Tag::Byte(ty as i8));
    let rotation = if wall { 0.0 } else { prop(Some(block), "rotation").and_then(|r| r.parse::<f32>().ok()).unwrap_or(0.0) * 22.5 };
    out.insert("Rotation".into(), Tag::Float(rotation));
    out.insert("DoingAnimation".into(), Tag::Byte(0));
    out.insert("MouthTickCount".into(), Tag::Int(0));
    Some(out)
}

fn banner_from_block(block: &BlockState, java: Option<&Compound>) -> Option<Compound> {
    let name = bare(&block.name);
    let base = dye_of(name, "_wall_banner").or_else(|| dye_of(name, "_banner"))?;
    let mut out = base_entity("Banner");
    out.insert("Base".into(), Tag::Int(15 - base as i32));
    out.insert("Type".into(), Tag::Int(0));
    let mut patterns = Vec::new();
    if let Some(java) = java {
        for p in list_of(java, "patterns") {
            let Some(c) = p.as_compound() else { continue };
            let id = c.get("pattern").and_then(Tag::as_str).map(bare);
            let color = text(c, "color").and_then(dye_index);
            if let (Some(id), Some(color)) = (id, color) {
                if let Some((_, code)) = PATTERNS.iter().find(|(j, _)| *j == id) {
                    let mut e = Compound::new();
                    e.insert("Color".into(), Tag::Int(15 - color as i32));
                    e.insert("Pattern".into(), Tag::String((*code).into()));
                    patterns.push(e);
                }
            }
        }
        if let Some(n) = java.get("CustomName") {
            out.insert("CustomName".into(), Tag::String(text_codec::to_legacy(n)));
        }
    }
    if !patterns.is_empty() {
        out.insert("Patterns".into(), compounds(patterns));
    }
    Some(out)
}

fn effect_number(effect: &str) -> i32 {
    EFFECTS.iter().position(|e| *e == bare(effect)).map_or(0, |i| i as i32 + 1)
}

fn effect_name(number: i64) -> Option<String> {
    usize::try_from(number - 1).ok().and_then(|i| EFFECTS.get(i)).map(|e| format!("minecraft:{e}"))
}

/// Brewing stand slots: Java has the three bottles first, then the ingredient and the fuel; Bedrock
/// has the ingredient first, then the bottles.
fn brewing_slot_to_bedrock(slot: u8) -> u8 {
    match slot {
        0..=2 => slot + 1,
        3 => 0,
        other => other,
    }
}

fn brewing_slot_to_java(slot: u8) -> u8 {
    match slot {
        0 => 3,
        1..=3 => slot - 1,
        other => other,
    }
}

fn java_items(c: &Compound, key: &str, map: impl Fn(u8) -> u8, notes: &mut Notes) -> Tag {
    let mut out = Vec::new();
    for item in list_of(c, key).into_iter().filter_map(Tag::as_compound) {
        let slot = int(item, "Slot").map(|s| map(s as u8));
        if let Some(b) = item_codec::java_to_bedrock(item, slot, notes) {
            out.push(b);
        }
    }
    compounds(out)
}

fn bedrock_items(c: &Compound, key: &str, map: impl Fn(u8) -> u8, notes: &mut Notes) -> Tag {
    let mut out = Vec::new();
    for item in list_of(c, key).into_iter().filter_map(Tag::as_compound) {
        let slot = int(item, "Slot").map(|s| map(s as u8));
        if let Some(mut j) = item_codec::bedrock_to_java(item, slot, notes) {
            if let Some(s) = slot {
                j.insert("Slot".into(), Tag::Byte(s as i8));
            }
            out.push(j);
        }
    }
    compounds(out)
}

fn brushable_loot_to_bedrock(table: &str) -> String {
    let path = table.trim_start_matches("minecraft:");
    let name = path.strip_prefix("archaeology/").unwrap_or(path);
    format!("loot_tables/entities/{name}_brushable_block.json")
}

fn brushable_loot_to_java(table: &str) -> String {
    let name = table
        .trim_start_matches("loot_tables/entities/")
        .trim_end_matches(".json")
        .trim_end_matches("_brushable_block");
    format!("minecraft:archaeology/{name}")
}

fn bedrock_spawn_entity(java_entity: &str) -> Option<String> {
    crate::entity::bedrock_entity_id(java_entity)
}

/// A Java block entity as a Bedrock one, for the kinds in this module. `block` is the Java block the
/// entity sits in.
pub fn java_to_bedrock(java: &Compound, block: Option<&BlockState>, notes: &mut Notes) -> Option<Compound> {
    let id = text(java, "id")?;
    if let Some((_, b)) = BARE.iter().find(|(j, _)| *j == id) {
        let mut out = base_entity(b);
        set_position(java, &mut out);
        match *b {
            "Bell" => {
                out.insert("Direction".into(), Tag::Int(0));
                out.insert("Ringing".into(), Tag::Byte(0));
                out.insert("Ticks".into(), Tag::Int(0));
            }
            "Conduit" => {
                out.insert("Active".into(), Tag::Byte(0));
                out.insert("Target".into(), Tag::Long(-1));
            }
            "EnchantTable" => {
                out.insert("rott".into(), Tag::Float(0.0));
            }
            "SculkSensor" | "CalibratedSculkSensor" | "SculkShrieker" => {
                let mut listener = Compound::new();
                listener.insert("selector".into(), Tag::Compound(Compound::new()));
                out.insert("VibrationListener".into(), Tag::Compound(listener));
            }
            "Beehive" => {
                let mut occupants = Vec::new();
                for bee in list_of(java, "bees").into_iter().filter_map(Tag::as_compound) {
                    let min = int(bee, "min_ticks_in_hive").unwrap_or(600);
                    let spent = int(bee, "ticks_in_hive").unwrap_or(0);
                    let mut o = Compound::new();
                    o.insert("ActorIdentifier".into(), Tag::String("minecraft:bee<>".into()));
                    o.insert("SaveData".into(), Tag::Compound(Compound::new()));
                    o.insert("TicksLeftToStay".into(), Tag::Int((min - spent).max(0) as i32));
                    occupants.push(o);
                }
                out.insert("Occupants".into(), compounds(occupants));
                out.insert("ShouldSpawnBees".into(), Tag::Byte(0));
            }
            _ => {}
        }
        return Some(out);
    }
    let mut out = base_entity("");
    set_position(java, &mut out);
    match id {
        "minecraft:banner" => {
            let mut banner = banner_from_block(block?, Some(java))?;
            set_position(java, &mut banner);
            return Some(banner);
        }
        "minecraft:skull" => {
            let mut skull = skull_from_block(block?)?;
            set_position(java, &mut skull);
            return Some(skull);
        }
        "minecraft:jukebox" => {
            out.insert("id".into(), Tag::String("Jukebox".into()));
            if let Some(b) = java.get("RecordItem").and_then(Tag::as_compound).and_then(|r| item_codec::java_to_bedrock(r, None, notes)) {
                out.insert("RecordItem".into(), Tag::Compound(b));
            }
        }
        "minecraft:lectern" => {
            out.insert("id".into(), Tag::String("Lectern".into()));
            match java.get("Book").and_then(Tag::as_compound) {
                Some(book) => {
                    let pages = book
                        .get("components")
                        .and_then(|c| c.get("minecraft:written_book_content").or_else(|| c.get("minecraft:writable_book_content")))
                        .and_then(|c| c.get("pages"))
                        .and_then(Tag::as_list)
                        .map_or(1, |p| p.len().max(1));
                    out.insert("hasBook".into(), Tag::Byte(1));
                    if let Some(b) = item_codec::java_to_bedrock(book, None, notes) {
                        out.insert("book".into(), Tag::Compound(b));
                    }
                    out.insert("page".into(), Tag::Int(int(java, "Page").unwrap_or(0) as i32));
                    out.insert("totalPages".into(), Tag::Int(pages as i32));
                }
                None => {
                    out.insert("hasBook".into(), Tag::Byte(0));
                }
            }
        }
        "minecraft:beacon" => {
            out.insert("id".into(), Tag::String("Beacon".into()));
            out.insert("primary".into(), Tag::Int(text(java, "primary_effect").map_or(0, effect_number)));
            out.insert("secondary".into(), Tag::Int(text(java, "secondary_effect").map_or(0, effect_number)));
        }
        "minecraft:brewing_stand" => {
            out.insert("id".into(), Tag::String("BrewingStand".into()));
            out.insert("Items".into(), java_items(java, "Items", brewing_slot_to_bedrock, notes));
            out.insert("CookTime".into(), Tag::Short(int(java, "BrewTime").unwrap_or(0) as i16));
            out.insert("FuelAmount".into(), Tag::Short(int(java, "Fuel").unwrap_or(0) as i16));
            out.insert("FuelTotal".into(), Tag::Short(20));
        }
        "minecraft:command_block" => {
            let name = block.map(|b| bare(&b.name)).unwrap_or("command_block");
            out.insert("id".into(), Tag::String("CommandBlock".into()));
            out.insert("Command".into(), Tag::String(text(java, "Command").unwrap_or("").into()));
            out.insert("CustomName".into(), Tag::String(java.get("CustomName").map(text_codec::to_legacy).unwrap_or_default()));
            out.insert("TrackOutput".into(), Tag::Byte(int(java, "TrackOutput").unwrap_or(1) as i8));
            out.insert("SuccessCount".into(), Tag::Int(int(java, "SuccessCount").unwrap_or(0) as i32));
            out.insert("LastOutput".into(), Tag::String(java.get("LastOutput").map(text_codec::to_plain).unwrap_or_default()));
            out.insert("LastOutputParams".into(), Tag::List(List::new(TagType::End)));
            out.insert("TickDelay".into(), Tag::Int(0));
            out.insert("LastExecution".into(), Tag::Long(0));
            out.insert("auto".into(), Tag::Byte(int(java, "auto").unwrap_or(0) as i8));
            out.insert("conditionMet".into(), Tag::Byte(int(java, "conditionMet").unwrap_or(0) as i8));
            out.insert("powered".into(), Tag::Byte(int(java, "powered").unwrap_or(0) as i8));
            out.insert("ExecuteOnFirstTick".into(), Tag::Byte((name == "repeating_command_block") as i8));
            out.insert("LPCommandMode".into(), Tag::Int(0));
            out.insert("LPCondionalMode".into(), Tag::Byte((prop(block, "conditional") == Some("true")) as i8));
            out.insert("LPRedstoneMode".into(), Tag::Byte(0));
            out.insert("Version".into(), Tag::Int(55));
        }
        "minecraft:decorated_pot" => {
            out.insert("id".into(), Tag::String("DecoratedPot".into()));
            let sherds: Vec<String> = list_of(java, "sherds").into_iter().filter_map(Tag::as_str).map(str::to_string).collect();
            if !sherds.is_empty() {
                out.insert("sherds".into(), strings(sherds));
            }
            out.insert("animation".into(), Tag::Byte(0));
            let held = java.get("item").and_then(Tag::as_compound).and_then(|i| item_codec::java_to_bedrock(i, None, notes));
            out.insert("item".into(), Tag::Compound(held.unwrap_or_else(empty_item)));
            if let Some(t) = text(java, "LootTable") {
                out.insert("LootTable".into(), Tag::String(format!("loot_tables/{}.json", t.trim_start_matches("minecraft:"))));
                out.insert("LootTableSeed".into(), Tag::Int(int(java, "LootTableSeed").unwrap_or(0) as i32));
            }
        }
        "minecraft:brushable_block" => {
            out.insert("id".into(), Tag::String("BrushableBlock".into()));
            let ty = block.and_then(|b| mappings().java_to_bedrock(b)).map_or("minecraft:suspicious_sand".to_string(), |b| b.name.clone());
            out.insert("type".into(), Tag::String(ty));
            out.insert("brush_count".into(), Tag::Int(0));
            out.insert("brush_direction".into(), Tag::Byte(6));
            if let Some(t) = text(java, "LootTable") {
                out.insert("LootTable".into(), Tag::String(brushable_loot_to_bedrock(t)));
                out.insert("LootTableSeed".into(), Tag::Int(int(java, "LootTableSeed").unwrap_or(0) as i32));
            }
            if let Some(b) = java.get("item").and_then(Tag::as_compound).and_then(|i| item_codec::java_to_bedrock(i, None, notes)) {
                out.insert("item".into(), Tag::Compound(b));
            }
        }
        "minecraft:chiseled_bookshelf" => {
            out.insert("id".into(), Tag::String("ChiseledBookshelf".into()));
            out.insert("Items".into(), java_items(java, "Items", |s| s, notes));
            out.insert("LastInteractedSlot".into(), Tag::Int(int(java, "last_interacted_slot").unwrap_or(-1) as i32));
        }
        "minecraft:crafter" => {
            out.insert("id".into(), Tag::String("Crafter".into()));
            out.insert("Items".into(), java_items(java, "Items", |s| s, notes));
            let mask = match java.get("disabled_slots") {
                Some(Tag::IntArray(a)) => a.iter().filter(|s| (0..9).contains(*s)).fold(0i16, |m, s| m | (1 << s)),
                _ => 0,
            };
            out.insert("disabled_slots".into(), Tag::Short(mask));
        }
        "minecraft:vault" => {
            out.insert("id".into(), Tag::String("Vault".into()));
            let ominous = prop(block, "ominous") == Some("true");
            let config = java.get("config").and_then(Tag::as_compound);
            let (default_key, default_loot) = if ominous {
                ("minecraft:ominous_trial_key", "loot_tables/chests/trial_chambers/reward_ominous.json")
            } else {
                ("minecraft:trial_key", "loot_tables/chests/trial_chambers/reward.json")
            };
            let key = config
                .and_then(|c| c.get("key_item"))
                .and_then(Tag::as_compound)
                .and_then(|i| item_codec::java_to_bedrock(i, None, notes))
                .unwrap_or_else(|| {
                    let mut k = empty_item();
                    k.insert("Name".into(), Tag::String(default_key.into()));
                    k.insert("Count".into(), Tag::Byte(1));
                    k
                });
            let loot = config
                .and_then(|c| text(c, "loot_table"))
                .map(|t| format!("loot_tables/{}.json", t.trim_start_matches("minecraft:")))
                .unwrap_or_else(|| default_loot.into());
            let mut cfg = Compound::new();
            cfg.insert("activation_range".into(), Tag::Float(config.and_then(|c| c.get("activation_range")).and_then(Tag::as_f64).unwrap_or(4.0) as f32));
            cfg.insert("deactivation_range".into(), Tag::Float(config.and_then(|c| c.get("deactivation_range")).and_then(Tag::as_f64).unwrap_or(4.5) as f32));
            cfg.insert("key_item".into(), Tag::Compound(key));
            cfg.insert("loot_table".into(), Tag::String(loot));
            cfg.insert("override_loot_table_to_display".into(), Tag::String(String::new()));
            out.insert("config".into(), Tag::Compound(cfg));
            let mut data = Compound::new();
            data.insert("items_to_eject".into(), Tag::List(List::new(TagType::End)));
            data.insert("rewarded_players".into(), Tag::List(List::new(TagType::End)));
            data.insert("state_updating_resumes_at".into(), Tag::Long(0));
            out.insert("data".into(), Tag::Compound(data));
        }
        "minecraft:trial_spawner" => {
            out.insert("id".into(), Tag::String("TrialSpawner".into()));
            out.insert("cooldown_end_at".into(), Tag::Long(0));
            out.insert("next_mob_spawns_at".into(), Tag::Long(0));
            out.insert("current_mobs".into(), Tag::List(List::new(TagType::End)));
            out.insert("registered_players".into(), Tag::List(List::new(TagType::End)));
            out.insert("total_mobs_spawned".into(), Tag::Int(0));
            out.insert("required_player_range".into(), Tag::Int(int(java, "required_player_range").unwrap_or(14) as i32));
            let entity = java.get("spawn_data").and_then(|d| d.get("entity")).and_then(|e| e.get("id")).and_then(Tag::as_str).and_then(bedrock_spawn_entity);
            if let Some(entity) = entity {
                let mut spawn = Compound::new();
                spawn.insert("TypeId".into(), Tag::String(entity));
                spawn.insert("Weight".into(), Tag::Int(1));
                out.insert("spawn_data".into(), Tag::Compound(spawn));
            }
        }
        _ => return None,
    }
    Some(out)
}

fn empty_item() -> Compound {
    let mut c = Compound::new();
    c.insert("Count".into(), Tag::Byte(0));
    c.insert("Damage".into(), Tag::Short(0));
    c.insert("Name".into(), Tag::String(String::new()));
    c.insert("WasPickedUp".into(), Tag::Byte(0));
    c
}

/// A Bedrock block entity as a Java one, for the kinds in this module. `block` is the Java block the
/// entity sits in.
pub fn bedrock_to_java(bedrock: &Compound, block: Option<&BlockState>, notes: &mut Notes) -> Option<ToJava> {
    let id = text(bedrock, "id")?;
    let block_name = block.map(|b| bare(&b.name)).unwrap_or("");
    let mut edit = BlockEdit::default();
    // Entities that only change the block.
    match id {
        "Bed" => {
            if let (Some(c), true) = (int(bedrock, "color"), block_name.ends_with("_bed")) {
                edit.name = Some(format!("minecraft:{}_bed", DYES[c.clamp(0, 15) as usize]));
            }
            return Some(ToJava { nbt: None, edit });
        }
        "Music" => {
            if block_name == "note_block" {
                edit.props.push(("note", Some(int(bedrock, "note").unwrap_or(0).clamp(0, 24).to_string())));
            }
            return Some(ToJava { nbt: None, edit });
        }
        "FlowerPot" => {
            if block_name == "flower_pot" || block_name.starts_with("potted_") {
                if let Some(potted) = bedrock.get("PlantBlock").and_then(Tag::as_compound).and_then(potted_block) {
                    edit.name = Some(potted);
                } else {
                    edit.name = Some("minecraft:flower_pot".into());
                }
            }
            return Some(ToJava { nbt: None, edit });
        }
        _ => {}
    }
    if let Some((java_id, _)) = BARE.iter().find(|(_, b)| *b == id) {
        let mut out = Compound::new();
        out.insert("id".into(), Tag::String((*java_id).into()));
        set_position(bedrock, &mut out);
        out.insert("keepPacked".into(), Tag::Byte(0));
        if id == "Beehive" {
            let mut bees = Vec::new();
            for o in list_of(bedrock, "Occupants").into_iter().filter_map(Tag::as_compound) {
                let mut data = Compound::new();
                data.insert("id".into(), Tag::String("minecraft:bee".into()));
                let mut bee = Compound::new();
                bee.insert("entity_data".into(), Tag::Compound(data));
                bee.insert("min_ticks_in_hive".into(), Tag::Int(600));
                bee.insert("ticks_in_hive".into(), Tag::Int((600 - int(o, "TicksLeftToStay").unwrap_or(0)).clamp(0, 600) as i32));
                bees.push(bee);
            }
            out.insert("bees".into(), compounds(bees));
        }
        return Some(ToJava { nbt: Some(out), edit });
    }
    let mut out = Compound::new();
    set_position(bedrock, &mut out);
    out.insert("keepPacked".into(), Tag::Byte(0));
    match id {
        "Banner" => {
            out.insert("id".into(), Tag::String("minecraft:banner".into()));
            let wall = block_name.contains("_wall_banner");
            let base = int(bedrock, "Base").map(|b| 15 - b.clamp(0, 15) as usize);
            if let (Some(base), true) = (base, block_name.ends_with("banner")) {
                edit.name = Some(format!("minecraft:{}_{}", DYES[base], if wall { "wall_banner" } else { "banner" }));
            }
            let mut patterns = Vec::new();
            for p in list_of(bedrock, "Patterns").into_iter().filter_map(Tag::as_compound) {
                let code = text(p, "Pattern").unwrap_or("");
                let color = int(p, "Color").unwrap_or(15).clamp(0, 15) as usize;
                if let Some((java, _)) = PATTERNS.iter().find(|(_, c)| *c == code) {
                    let mut e = Compound::new();
                    e.insert("pattern".into(), Tag::String(format!("minecraft:{java}")));
                    e.insert("color".into(), Tag::String(DYES[15 - color].into()));
                    patterns.push(e);
                }
            }
            if !patterns.is_empty() {
                out.insert("patterns".into(), compounds(patterns));
            }
            if let Some(n) = text(bedrock, "CustomName").filter(|n| !n.is_empty()) {
                out.insert("CustomName".into(), text_codec::from_legacy(n));
            }
        }
        "Skull" => {
            out.insert("id".into(), Tag::String("minecraft:skull".into()));
            let ty = int(bedrock, "SkullType").unwrap_or(-1);
            if (0..SKULLS.len() as i64).contains(&ty) && skull_type_of(&format!("minecraft:{block_name}")).is_some() {
                let wall = block_name.contains("wall");
                let (floor, wall_name) = SKULLS[ty as usize];
                edit.name = Some((if wall { wall_name } else { floor }).to_string());
                if !wall {
                    let rotation = bedrock.get("Rotation").and_then(Tag::as_f64).unwrap_or(0.0);
                    edit.props.push(("rotation", Some(((rotation / 22.5).round() as i64).rem_euclid(16).to_string())));
                }
            }
            // Heads of other players carry no profile on Bedrock.
            return Some(ToJava { nbt: None, edit });
        }
        "Jukebox" => {
            out.insert("id".into(), Tag::String("minecraft:jukebox".into()));
            let record = bedrock.get("RecordItem").and_then(Tag::as_compound).and_then(|r| item_codec::bedrock_to_java(r, None, notes));
            edit.props.push(("has_record", Some(record.is_some().to_string())));
            if let Some(r) = record {
                out.insert("RecordItem".into(), Tag::Compound(r));
                out.insert("IsPlaying".into(), Tag::Byte(0));
                out.insert("RecordStartTick".into(), Tag::Long(0));
                out.insert("TickCount".into(), Tag::Long(0));
            }
        }
        "Lectern" => {
            out.insert("id".into(), Tag::String("minecraft:lectern".into()));
            let book = if int(bedrock, "hasBook").unwrap_or(0) != 0 {
                bedrock.get("book").and_then(Tag::as_compound).and_then(|b| item_codec::bedrock_to_java(b, None, notes))
            } else {
                None
            };
            edit.props.push(("has_book", Some(book.is_some().to_string())));
            if let Some(b) = book {
                out.insert("Book".into(), Tag::Compound(b));
                out.insert("Page".into(), Tag::Int(int(bedrock, "page").unwrap_or(0) as i32));
            }
        }
        "Beacon" => {
            out.insert("id".into(), Tag::String("minecraft:beacon".into()));
            out.insert("Levels".into(), Tag::Int(0));
            if let Some(e) = effect_name(int(bedrock, "primary").unwrap_or(0)) {
                out.insert("primary_effect".into(), Tag::String(e));
            }
            if let Some(e) = effect_name(int(bedrock, "secondary").unwrap_or(0)) {
                out.insert("secondary_effect".into(), Tag::String(e));
            }
        }
        "BrewingStand" => {
            out.insert("id".into(), Tag::String("minecraft:brewing_stand".into()));
            out.insert("Items".into(), bedrock_items(bedrock, "Items", brewing_slot_to_java, notes));
            out.insert("BrewTime".into(), Tag::Short(int(bedrock, "CookTime").unwrap_or(0) as i16));
            out.insert("Fuel".into(), Tag::Byte(int(bedrock, "FuelAmount").unwrap_or(0) as i8));
        }
        "CommandBlock" => {
            out.insert("id".into(), Tag::String("minecraft:command_block".into()));
            out.insert("Command".into(), Tag::String(text(bedrock, "Command").unwrap_or("").into()));
            if let Some(n) = text(bedrock, "CustomName").filter(|n| !n.is_empty()) {
                out.insert("CustomName".into(), text_codec::from_legacy(n));
            }
            out.insert("TrackOutput".into(), Tag::Byte(int(bedrock, "TrackOutput").unwrap_or(1) as i8));
            out.insert("SuccessCount".into(), Tag::Int(int(bedrock, "SuccessCount").unwrap_or(0) as i32));
            out.insert("auto".into(), Tag::Byte(int(bedrock, "auto").unwrap_or(0) as i8));
            out.insert("conditionMet".into(), Tag::Byte(int(bedrock, "conditionMet").unwrap_or(0) as i8));
            out.insert("powered".into(), Tag::Byte(int(bedrock, "powered").unwrap_or(0) as i8));
            out.insert("UpdateLastExecution".into(), Tag::Byte(1));
        }
        "DecoratedPot" => {
            out.insert("id".into(), Tag::String("minecraft:decorated_pot".into()));
            let sherds: Vec<String> = list_of(bedrock, "sherds").into_iter().filter_map(Tag::as_str).map(str::to_string).collect();
            if sherds.len() == 4 {
                out.insert("sherds".into(), strings(sherds));
            }
            if let Some(i) = bedrock.get("item").and_then(Tag::as_compound).and_then(|i| item_codec::bedrock_to_java(i, None, notes)) {
                out.insert("item".into(), Tag::Compound(i));
            }
            if let Some(t) = text(bedrock, "LootTable").filter(|t| !t.is_empty()) {
                let path = t.trim_start_matches("loot_tables/").trim_end_matches(".json");
                out.insert("LootTable".into(), Tag::String(format!("minecraft:{path}")));
                out.insert("LootTableSeed".into(), Tag::Long(int(bedrock, "LootTableSeed").unwrap_or(0)));
            }
        }
        "BrushableBlock" => {
            out.insert("id".into(), Tag::String("minecraft:brushable_block".into()));
            if let Some(t) = text(bedrock, "LootTable").filter(|t| !t.is_empty() && !t.contains("empty_brushable")) {
                out.insert("LootTable".into(), Tag::String(brushable_loot_to_java(t)));
                out.insert("LootTableSeed".into(), Tag::Long(int(bedrock, "LootTableSeed").unwrap_or(0)));
            }
            if let Some(i) = bedrock.get("item").and_then(Tag::as_compound).and_then(|i| item_codec::bedrock_to_java(i, None, notes)) {
                out.insert("item".into(), Tag::Compound(i));
            }
        }
        "ChiseledBookshelf" => {
            out.insert("id".into(), Tag::String("minecraft:chiseled_bookshelf".into()));
            out.insert("Items".into(), bedrock_items(bedrock, "Items", |s| s, notes));
            out.insert("last_interacted_slot".into(), Tag::Int(int(bedrock, "LastInteractedSlot").unwrap_or(-1) as i32));
        }
        "Crafter" => {
            out.insert("id".into(), Tag::String("minecraft:crafter".into()));
            out.insert("Items".into(), bedrock_items(bedrock, "Items", |s| s, notes));
            let mask = int(bedrock, "disabled_slots").unwrap_or(0);
            out.insert("disabled_slots".into(), Tag::IntArray((0..9).filter(|s| mask & (1 << s) != 0).collect()));
            out.insert("crafting_ticks_remaining".into(), Tag::Int(0));
            out.insert("triggered".into(), Tag::Int(0));
        }
        "Vault" => {
            out.insert("id".into(), Tag::String("minecraft:vault".into()));
            if let Some(cfg) = bedrock.get("config").and_then(Tag::as_compound) {
                let mut config = Compound::new();
                if let Some(k) = cfg.get("key_item").and_then(Tag::as_compound).and_then(|i| item_codec::bedrock_to_java(i, None, notes)) {
                    config.insert("key_item".into(), Tag::Compound(k));
                }
                if let Some(t) = text(cfg, "loot_table").filter(|t| !t.is_empty()) {
                    let path = t.trim_start_matches("loot_tables/").trim_end_matches(".json");
                    config.insert("loot_table".into(), Tag::String(format!("minecraft:{path}")));
                }
                for key in ["activation_range", "deactivation_range"] {
                    if let Some(v) = cfg.get(key).and_then(Tag::as_f64) {
                        config.insert(key.into(), Tag::Double(v));
                    }
                }
                out.insert("config".into(), Tag::Compound(config));
            }
        }
        "TrialSpawner" => {
            out.insert("id".into(), Tag::String("minecraft:trial_spawner".into()));
            if let Some(r) = int(bedrock, "required_player_range") {
                out.insert("required_player_range".into(), Tag::Int(r as i32));
            }
            let entity = bedrock.get("spawn_data").and_then(Tag::as_compound).and_then(|d| text(d, "TypeId")).and_then(crate::entity::java_entity_id_of);
            if let Some(entity) = entity {
                let mut e = Compound::new();
                e.insert("id".into(), Tag::String(entity));
                let mut data = Compound::new();
                data.insert("entity".into(), Tag::Compound(e));
                out.insert("spawn_data".into(), Tag::Compound(data));
            }
        }
        _ => return None,
    }
    Some(ToJava { nbt: Some(out), edit })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Tag {
        Tag::String(v.into())
    }

    fn entity(id: &str) -> Compound {
        let mut c = Compound::new();
        c.insert("id".into(), s(id));
        for k in ["x", "y", "z"] {
            c.insert(k.into(), Tag::Int(1));
        }
        c
    }

    #[test]
    fn beds_and_note_blocks_come_from_their_blocks() {
        let bed = from_block(&BlockState::new("minecraft:pink_bed")).unwrap();
        assert_eq!(bed.get("color"), Some(&Tag::Byte(6)));
        let j = bedrock_to_java(&bed, Some(&BlockState::new("minecraft:white_bed")), &mut Notes::default()).unwrap();
        assert!(j.nbt.is_none());
        assert_eq!(j.edit.name.as_deref(), Some("minecraft:pink_bed"));
        let note = from_block(&BlockState::new("minecraft:note_block").with("note", "7")).unwrap();
        assert_eq!((note.get("id"), note.get("note")), (Some(&s("Music")), Some(&Tag::Byte(7))));
    }

    #[test]
    fn banners_keep_base_and_patterns() {
        let mut j = entity("minecraft:banner");
        let mut p = Compound::new();
        p.insert("pattern".into(), s("minecraft:stripe_bottom"));
        p.insert("color".into(), s("red"));
        j.insert("patterns".into(), compounds(vec![p]));
        let block = BlockState::new("minecraft:blue_wall_banner");
        let b = java_to_bedrock(&j, Some(&block), &mut Notes::default()).unwrap();
        assert_eq!(b.get("Base"), Some(&Tag::Int(4)));
        let pattern = b.get("Patterns").and_then(Tag::as_list).unwrap().get(0).unwrap();
        assert_eq!((pattern.get("Pattern"), pattern.get("Color")), (Some(&s("bs")), Some(&Tag::Int(1))));
        let back = bedrock_to_java(&b, Some(&BlockState::new("minecraft:white_wall_banner")), &mut Notes::default()).unwrap();
        assert_eq!(back.edit.name.as_deref(), Some("minecraft:blue_wall_banner"));
        let patterns = back.nbt.unwrap();
        let first = patterns.get("patterns").and_then(Tag::as_list).unwrap().get(0).unwrap().clone();
        assert_eq!(first.get("color"), Some(&s("red")));
    }

    #[test]
    fn skulls_take_their_type_from_the_block() {
        let block = BlockState::new("minecraft:creeper_head").with("rotation", "4");
        let b = from_block(&block).unwrap();
        assert_eq!((b.get("SkullType"), b.get("Rotation")), (Some(&Tag::Byte(4)), Some(&Tag::Float(90.0))));
        let j = bedrock_to_java(&b, Some(&BlockState::new("minecraft:skeleton_skull")), &mut Notes::default()).unwrap();
        assert_eq!(j.edit.name.as_deref(), Some("minecraft:creeper_head"));
        assert_eq!(j.edit.props, vec![("rotation", Some("4".to_string()))]);
    }

    #[test]
    fn flower_pots_hold_their_plant() {
        let b = from_block(&BlockState::new("minecraft:potted_cactus")).unwrap();
        assert_eq!(b.get("PlantBlock").and_then(|p| p.get("name")), Some(&s("minecraft:cactus")));
        let j = bedrock_to_java(&b, Some(&BlockState::new("minecraft:flower_pot")), &mut Notes::default()).unwrap();
        assert_eq!(j.edit.name.as_deref(), Some("minecraft:potted_cactus"));
    }

    #[test]
    fn brewing_stand_slots_are_rearranged() {
        let mut j = entity("minecraft:brewing_stand");
        let mut item = Compound::new();
        item.insert("id".into(), s("minecraft:blaze_powder"));
        item.insert("count".into(), Tag::Int(2));
        item.insert("Slot".into(), Tag::Byte(4));
        let mut ing = item.clone();
        ing.insert("Slot".into(), Tag::Byte(3));
        j.insert("Items".into(), compounds(vec![item, ing]));
        let b = java_to_bedrock(&j, None, &mut Notes::default()).unwrap();
        let slots: Vec<i64> = b.get("Items").and_then(Tag::as_list).unwrap().iter().filter_map(|i| i.get("Slot").and_then(Tag::as_i64)).collect();
        assert_eq!(slots, vec![4, 0]);
        let back = bedrock_to_java(&b, None, &mut Notes::default()).unwrap().nbt.unwrap();
        let slots: Vec<i64> = back.get("Items").and_then(Tag::as_list).unwrap().iter().filter_map(|i| i.get("Slot").and_then(Tag::as_i64)).collect();
        assert_eq!(slots, vec![4, 3]);
    }

    #[test]
    fn data_less_kinds_and_unknown_kinds() {
        let b = java_to_bedrock(&entity("minecraft:bell"), None, &mut Notes::default()).unwrap();
        assert_eq!(b.get("id"), Some(&s("Bell")));
        assert!(java_to_bedrock(&entity("minecraft:not_a_thing"), None, &mut Notes::default()).is_none());
        let j = bedrock_to_java(&entity("SculkShrieker"), None, &mut Notes::default()).unwrap();
        assert_eq!(j.nbt.unwrap().get("id"), Some(&s("minecraft:sculk_shrieker")));
        assert!(is_bedrock_only("PistonArm") && is_java_only("DUMMY"));
    }

    #[test]
    fn beacons_use_bedrock_effect_numbers() {
        let mut j = entity("minecraft:beacon");
        j.insert("primary_effect".into(), s("minecraft:haste"));
        let b = java_to_bedrock(&j, None, &mut Notes::default()).unwrap();
        assert_eq!(b.get("primary"), Some(&Tag::Int(3)));
        let back = bedrock_to_java(&b, None, &mut Notes::default()).unwrap().nbt.unwrap();
        assert_eq!(back.get("primary_effect"), Some(&s("minecraft:haste")));
    }
}
