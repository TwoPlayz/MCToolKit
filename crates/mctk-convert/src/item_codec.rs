//! Item stacks between Java (26.x: `id`, `count`, `components`) and Bedrock (`Name`, `Count`,
//! `Damage`, `tag`).
//!
//! The item's identity goes through the generated item table (`mctk-mapping`); the properties
//! both games have are carried by name: wear, custom name and lore, enchantments, unbreakable,
//! repair cost, dye colour, potion type, book text, map number, spawn egg mob, custom model data
//! and the contents of containers (shulker boxes). Anything else on a stack is counted in the
//! report and left out.

use std::collections::BTreeMap;

use mctk_nbt::{Compound, List, Tag, TagType};

use crate::text_codec;

/// What a conversion could not carry over, by kind.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Notes {
    pub missing: BTreeMap<String, usize>,
}

impl Notes {
    pub fn miss(&mut self, what: impl Into<String>) {
        *self.missing.entry(what.into()).or_insert(0) += 1;
    }
}

/// Java enchantment names in Bedrock's numbering.
const ENCHANTMENTS: [&str; 41] = [
    "protection",
    "fire_protection",
    "feather_falling",
    "blast_protection",
    "projectile_protection",
    "thorns",
    "respiration",
    "depth_strider",
    "aqua_affinity",
    "sharpness",
    "smite",
    "bane_of_arthropods",
    "knockback",
    "fire_aspect",
    "looting",
    "efficiency",
    "silk_touch",
    "unbreaking",
    "fortune",
    "power",
    "punch",
    "flame",
    "infinity",
    "luck_of_the_sea",
    "lure",
    "frost_walker",
    "mending",
    "binding_curse",
    "vanishing_curse",
    "impaling",
    "riptide",
    "loyalty",
    "channeling",
    "multishot",
    "piercing",
    "quick_charge",
    "soul_speed",
    "swift_sneak",
    "wind_burst",
    "density",
    "breach",
];

/// Java potion types in the order Bedrock numbers them in the potion's data value.
const POTIONS: [&str; 47] = [
    "water",
    "mundane",
    "long_mundane",
    "thick",
    "awkward",
    "night_vision",
    "long_night_vision",
    "invisibility",
    "long_invisibility",
    "leaping",
    "long_leaping",
    "strong_leaping",
    "fire_resistance",
    "long_fire_resistance",
    "swiftness",
    "long_swiftness",
    "strong_swiftness",
    "slowness",
    "long_slowness",
    "water_breathing",
    "long_water_breathing",
    "healing",
    "strong_healing",
    "harming",
    "strong_harming",
    "poison",
    "long_poison",
    "strong_poison",
    "regeneration",
    "long_regeneration",
    "strong_regeneration",
    "strength",
    "long_strength",
    "strong_strength",
    "weakness",
    "long_weakness",
    "wither",
    "turtle_master",
    "long_turtle_master",
    "strong_turtle_master",
    "slow_falling",
    "long_slow_falling",
    "strong_slowness",
    "wind_charged",
    "weaving",
    "oozing",
    "infested",
];

fn ns(name: &str) -> String {
    if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") }
}

fn bare(name: &str) -> &str {
    name.trim_start_matches("minecraft:")
}

fn int(c: &Compound, key: &str) -> Option<i64> {
    c.get(key).and_then(Tag::as_i64)
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

// ------------------------------------------------------------------------- shared tables

/// The 16 dye colors as Java gives fireworks their colors (RGB), in Bedrock's numbering of dyes.
const FIREWORK_COLORS: [i32; 16] = [
    1_908_001, 11_743_532, 3_887_386, 5_320_730, 2_437_522, 8_073_150, 2_651_799, 11_250_603, 4_408_131, 14_188_952, 4_312_372, 14_602_026, 6_719_955, 12_801_229, 15_435_844, 15_790_320,
];

/// Java's firework shapes, in Bedrock's numbering.
const FIREWORK_SHAPES: [&str; 5] = ["small_ball", "large_ball", "star", "creeper", "burst"];

/// The dye (0 to 15) whose color is nearest an RGB color.
fn nearest_dye(rgb: i32) -> i8 {
    let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
    FIREWORK_COLORS
        .iter()
        .enumerate()
        .min_by_key(|(_, c)| {
            let (cr, cg, cb) = ((**c >> 16) & 0xff, (**c >> 8) & 0xff, **c & 0xff);
            (r - cr).pow(2) + (g - cg).pow(2) + (b - cb).pow(2)
        })
        .map_or(0, |(i, _)| i as i8)
}

/// The block names a Java `can_place_on` / `can_break` allows, from any version's form (one
/// predicate or a list of them, `blocks` one name or several, or the older `predicates`). Tags
/// (`#minecraft:logs`) have no Bedrock form and are counted.
fn java_block_names(value: &Tag, notes: &mut Notes) -> Vec<String> {
    let mut out = Vec::new();
    let mut predicate = |p: &Tag, notes: &mut Notes| match p.get("blocks") {
        Some(Tag::String(s)) if !s.starts_with('#') => out.push(ns(s)),
        Some(Tag::List(l)) => {
            for b in l.iter().filter_map(Tag::as_str) {
                if b.starts_with('#') {
                    notes.miss("item block tag in place/break lists");
                } else {
                    out.push(ns(b));
                }
            }
        }
        Some(Tag::String(_)) => notes.miss("item block tag in place/break lists"),
        _ => {}
    };
    match value {
        Tag::List(l) => l.iter().for_each(|p| predicate(p, notes)),
        Tag::Compound(c) => match c.get("predicates") {
            Some(Tag::List(l)) => l.iter().for_each(|p| predicate(p, notes)),
            _ => predicate(value, notes),
        },
        _ => {}
    }
    out
}

/// A Java firework explosion as Bedrock's `FireworkColor`/`FireworkFade`/... compound.
fn explosion_to_bedrock(e: &Compound) -> Compound {
    let dyes = |key: &str| -> Tag {
        let colors: Vec<i8> = match e.get(key) {
            Some(Tag::IntArray(a)) => a.iter().map(|c| nearest_dye(*c)).collect(),
            Some(Tag::List(l)) => l.iter().filter_map(Tag::as_i64).map(|c| nearest_dye(c as i32)).collect(),
            _ => Vec::new(),
        };
        Tag::ByteArray(colors)
    };
    let shape = e.get("shape").and_then(Tag::as_str).and_then(|s| FIREWORK_SHAPES.iter().position(|n| *n == bare(s))).unwrap_or(0);
    let mut out = Compound::new();
    out.insert("FireworkColor".into(), dyes("colors"));
    out.insert("FireworkFade".into(), dyes("fade_colors"));
    out.insert("FireworkFlicker".into(), Tag::Byte(int(e, "has_twinkle").unwrap_or(0) as i8));
    out.insert("FireworkTrail".into(), Tag::Byte(int(e, "has_trail").unwrap_or(0) as i8));
    out.insert("FireworkType".into(), Tag::Byte(shape as i8));
    out
}

/// Bedrock's `FireworkColor`/... compound as a Java firework explosion.
fn explosion_to_java(e: &Compound) -> Compound {
    let colors = |key: &str| -> Tag {
        let v: Vec<Tag> = match e.get(key) {
            Some(Tag::ByteArray(b)) => b.iter().map(|d| Tag::Int(FIREWORK_COLORS[(*d as usize).min(15)])).collect(),
            Some(Tag::List(l)) => l.iter().filter_map(Tag::as_i64).map(|d| Tag::Int(FIREWORK_COLORS[(d as usize).min(15)])).collect(),
            _ => Vec::new(),
        };
        if v.is_empty() { Tag::List(List::new(TagType::End)) } else { Tag::List(List::from_tags(TagType::Int, v).expect("ints")) }
    };
    let mut out = Compound::new();
    out.insert("shape".into(), Tag::String(FIREWORK_SHAPES[(int(e, "FireworkType").unwrap_or(0) as usize).min(4)].into()));
    out.insert("colors".into(), colors("FireworkColor"));
    out.insert("fade_colors".into(), colors("FireworkFade"));
    out.insert("has_trail".into(), Tag::Byte((int(e, "FireworkTrail").unwrap_or(0) != 0) as i8));
    out.insert("has_twinkle".into(), Tag::Byte((int(e, "FireworkFlicker").unwrap_or(0) != 0) as i8));
    out
}

// ------------------------------------------------------------------------- Java -> Bedrock

/// A Java stack as a Bedrock one. `slot` is written when the stack sits in a container.
pub fn java_to_bedrock(item: &Compound, slot: Option<u8>, notes: &mut Notes) -> Option<Compound> {
    let id = item.get("id").and_then(Tag::as_str)?;
    if bare(id) == "air" {
        return None;
    }
    let count = int(item, "count").or_else(|| int(item, "Count")).unwrap_or(1);
    if count <= 0 {
        return None;
    }
    let Some((name, data)) = mctk_mapping::item_mappings().java_to_bedrock(&ns(id)) else {
        notes.miss(format!("unknown item {}", ns(id)));
        return None;
    };
    let mut out = Compound::new();
    out.insert("Count".into(), Tag::Byte(count.min(127) as i8));
    out.insert("Damage".into(), Tag::Short(data));
    out.insert("Name".into(), Tag::String(name.to_string()));
    out.insert("WasPickedUp".into(), Tag::Byte(0));
    if let Some(slot) = slot {
        out.insert("Slot".into(), Tag::Byte(slot as i8));
    }
    let mut tag = Compound::new();
    if let Some(components) = item.get("components").and_then(Tag::as_compound) {
        for (key, value) in components {
            java_component(&ns(id), key, value, &mut out, &mut tag, notes);
        }
    }
    if !tag.is_empty() {
        out.insert("tag".into(), Tag::Compound(tag));
    }
    Some(out)
}

fn display(tag: &mut Compound) -> &mut Compound {
    if !matches!(tag.get("display"), Some(Tag::Compound(_))) {
        tag.insert("display".into(), Tag::Compound(Compound::new()));
    }
    match tag.get_mut("display") {
        Some(Tag::Compound(c)) => c,
        _ => unreachable!("inserted above"),
    }
}

fn java_enchantments(value: &Tag) -> Vec<(i16, i16)> {
    let mut out = Vec::new();
    let mut add = |name: &str, level: i64| {
        if let Some(n) = ENCHANTMENTS.iter().position(|e| *e == bare(name)) {
            out.push((n as i16, level as i16));
        }
    };
    match value {
        Tag::Compound(c) => {
            // 1.21.5 and later: `{id: level}`, before that `{levels: {id: level}}`.
            let levels = c.get("levels").and_then(Tag::as_compound).unwrap_or(c);
            for (name, level) in levels {
                if let Some(level) = level.as_i64() {
                    add(name, level);
                }
            }
        }
        Tag::List(l) => {
            for e in l.iter() {
                if let (Some(id), Some(level)) = (e.get("id").and_then(Tag::as_str), e.get("lvl").and_then(Tag::as_i64)) {
                    add(id, level);
                }
            }
        }
        _ => {}
    }
    out
}

fn enchant_list(list: Vec<(i16, i16)>) -> Tag {
    compounds(
        list.into_iter()
            .map(|(id, lvl)| {
                let mut c = Compound::new();
                c.insert("id".into(), Tag::Short(id));
                c.insert("lvl".into(), Tag::Short(lvl));
                c
            })
            .collect(),
    )
}

fn java_component(item: &str, key: &str, value: &Tag, out: &mut Compound, tag: &mut Compound, notes: &mut Notes) {
    match bare(key) {
        "damage" => {
            if let Some(d) = value.as_i64() {
                tag.insert("Damage".into(), Tag::Int(d as i32));
            }
        }
        "max_damage" | "max_stack_size" | "rarity" | "item_name" | "item_model" | "tooltip_style" => {}
        "unbreakable" => {
            tag.insert("Unbreakable".into(), Tag::Byte(1));
        }
        "repair_cost" => {
            if let Some(c) = value.as_i64() {
                tag.insert("RepairCost".into(), Tag::Int(c as i32));
            }
        }
        "custom_name" => {
            display(tag).insert("Name".into(), Tag::String(text_codec::to_legacy(value)));
        }
        "lore" => {
            let lines: Vec<String> = value.as_list().map(|l| l.iter().map(text_codec::to_legacy).collect()).unwrap_or_default();
            display(tag).insert("Lore".into(), strings(lines));
        }
        "enchantments" => {
            let list = java_enchantments(value);
            if !list.is_empty() {
                tag.insert("ench".into(), enchant_list(list));
            }
        }
        "stored_enchantments" => {
            let list = java_enchantments(value);
            if !list.is_empty() {
                tag.insert("StoredEnchantments".into(), enchant_list(list));
            }
        }
        "dyed_color" => {
            let rgb = value.as_i64().or_else(|| value.as_compound().and_then(|c| int(c, "rgb")));
            if let Some(rgb) = rgb {
                tag.insert("customColor".into(), Tag::Int((rgb as u32 | 0xff00_0000) as i32));
            }
        }
        "potion_contents" => {
            let potion = value.as_str().or_else(|| value.as_compound().and_then(|c| c.get("potion").and_then(Tag::as_str)));
            if let Some(n) = potion.and_then(|p| POTIONS.iter().position(|t| *t == bare(p))) {
                out.insert("Damage".into(), Tag::Short(n as i16));
            }
        }
        "map_id" => {
            if let Some(id) = value.as_i64() {
                tag.insert("map_uuid".into(), Tag::Long(id));
            }
        }
        "custom_model_data" => {}
        "trim" => {
            // {pattern, material} (names, in any version); a material of one's own has no Bedrock form.
            let name = |k: &str| value.get(k).and_then(Tag::as_str).map(|s| bare(s).to_string());
            match (name("material"), name("pattern")) {
                (Some(material), Some(pattern)) => {
                    let mut t = Compound::new();
                    t.insert("Material".into(), Tag::String(material));
                    t.insert("Pattern".into(), Tag::String(pattern));
                    tag.insert("Trim".into(), Tag::Compound(t));
                }
                _ => notes.miss("item trim of its own"),
            }
        }
        "can_place_on" | "can_break" => {
            let names = java_block_names(value, notes);
            if !names.is_empty() {
                tag.insert(if bare(key) == "can_break" { "CanDestroy" } else { "CanPlaceOn" }.into(), strings(names));
            }
        }
        "fireworks" => {
            if let Some(c) = value.as_compound() {
                let explosions: Vec<Compound> = list_of(c, "explosions").into_iter().filter_map(Tag::as_compound).map(explosion_to_bedrock).collect();
                let mut f = Compound::new();
                f.insert("Explosions".into(), compounds(explosions));
                f.insert("Flight".into(), Tag::Byte(int(c, "flight_duration").unwrap_or(1) as i8));
                tag.insert("Fireworks".into(), Tag::Compound(f));
            }
        }
        "firework_explosion" => {
            if let Some(c) = value.as_compound() {
                tag.insert("FireworksItem".into(), Tag::Compound(explosion_to_bedrock(c)));
            }
        }
        "charged_projectiles" => {
            // A loaded crossbow: its first arrow or rocket.
            if let Some(stack) = list_of_tags(value).into_iter().find_map(Tag::as_compound) {
                if let Some(b) = java_to_bedrock(stack, None, notes) {
                    tag.insert("chargedItem".into(), Tag::Compound(b));
                }
            }
        }
        "entity_data" => {
            // A spawn egg's mob is part of the item in Bedrock.
            if !item.ends_with("_spawn_egg") {
                notes.miss("item entity data");
            }
        }
        "writable_book_content" | "written_book_content" => {
            if let Some(c) = value.as_compound() {
                let mut pages = Vec::new();
                for page in list_of(c, "pages") {
                    let text = match page {
                        Tag::Compound(p) => p.get("raw").map(text_codec::to_plain).or_else(|| p.get("text").map(text_codec::to_plain)).unwrap_or_default(),
                        other => text_codec::to_plain(other),
                    };
                    let mut p = Compound::new();
                    p.insert("photoname".into(), Tag::String(String::new()));
                    p.insert("text".into(), Tag::String(text));
                    pages.push(p);
                }
                tag.insert("pages".into(), compounds(pages));
                if let Some(title) = c.get("title") {
                    let t = match title {
                        Tag::Compound(t) => t.get("raw").map(text_codec::to_plain).unwrap_or_default(),
                        other => text_codec::to_plain(other),
                    };
                    tag.insert("title".into(), Tag::String(t));
                }
                if let Some(a) = c.get("author").and_then(Tag::as_str) {
                    tag.insert("author".into(), Tag::String(a.to_string()));
                }
                if bare(key) == "written_book_content" {
                    tag.insert("generation".into(), Tag::Int(int(c, "generation").unwrap_or(0) as i32));
                }
            }
        }
        "container" => {
            let mut held = Vec::new();
            for entry in list_of_tags(value) {
                let slot = entry.get("slot").and_then(Tag::as_i64).unwrap_or(0);
                if let Some(stack) = entry.get("item").and_then(Tag::as_compound) {
                    if let Some(b) = java_to_bedrock(stack, Some(slot as u8), notes) {
                        held.push(b);
                    }
                }
            }
            if !held.is_empty() {
                tag.insert("Items".into(), compounds(held));
            }
        }
        other => notes.miss(format!("item component {other}")),
    }
}

fn list_of_tags(value: &Tag) -> Vec<&Tag> {
    value.as_list().map(|l| l.iter().collect()).unwrap_or_default()
}

// ------------------------------------------------------------------------- Bedrock -> Java

/// A Bedrock stack as a Java one.
pub fn bedrock_to_java(item: &Compound, slot: Option<u8>, notes: &mut Notes) -> Option<Compound> {
    let name = item.get("Name").and_then(Tag::as_str)?;
    let count = int(item, "Count").unwrap_or(0);
    if count <= 0 || bare(name) == "air" {
        return None;
    }
    let data = int(item, "Damage").unwrap_or(0) as i16;
    let java = mctk_mapping::item_mappings()
        .bedrock_to_java(&ns(name), data)
        .or_else(|| mctk_mapping::item_mappings().bedrock_to_java(&ns(name), 0));
    let Some(java) = java else {
        notes.miss(format!("unknown item {}", ns(name)));
        return None;
    };
    let mut out = Compound::new();
    out.insert("id".into(), Tag::String(java.to_string()));
    out.insert("count".into(), Tag::Int(count as i32));
    if let Some(slot) = slot {
        out.insert("Slot".into(), Tag::Byte(slot as i8));
    }
    let mut components = Compound::new();
    if let Some(tag) = item.get("tag").and_then(Tag::as_compound) {
        bedrock_tag(java, data, tag, &mut components, notes);
    }
    if bare(java).ends_with("potion") && components.get("minecraft:potion_contents").is_none() {
        if let Some(p) = POTIONS.get(data as usize).filter(|_| data > 0) {
            components.insert("minecraft:potion_contents".into(), potion_value(p));
        }
    }
    out.insert("components".into(), Tag::Compound(components));
    Some(out)
}

fn potion_value(p: &str) -> Tag {
    let mut c = Compound::new();
    c.insert("potion".into(), Tag::String(format!("minecraft:{p}")));
    Tag::Compound(c)
}

fn bedrock_enchantments(tag: &Compound, key: &str) -> Option<Tag> {
    let mut levels = Compound::new();
    for e in list_of(tag, key) {
        let Some(lvl) = e.get("lvl").and_then(Tag::as_i64) else { continue };
        // A number (Bedrock's own), or a name (a list written the Java way, as the NBT editor of
        // an earlier version did on Bedrock items).
        let name = match e.get("id") {
            Some(Tag::String(s)) => Some(bare(s).to_string()),
            Some(t) => t.as_i64().and_then(|id| ENCHANTMENTS.get(id as usize)).map(|n| (*n).to_string()),
            None => None,
        };
        if let Some(name) = name {
            levels.insert(format!("minecraft:{name}"), Tag::Int(lvl as i32));
        }
    }
    (!levels.is_empty()).then_some(Tag::Compound(levels))
}

/// An item's `AttributeModifiers` written the old Java way (the NBT editor of an earlier version
/// wrote them on Bedrock items too), as the 1.21.5 `attribute_modifiers` list.
fn java_style_attributes(tag: &Compound) -> Vec<Compound> {
    let snake = |s: &str| {
        let s = s.rsplit('.').next().unwrap_or(s);
        let mut out = String::new();
        for c in s.chars() {
            if c.is_ascii_uppercase() {
                out.push('_');
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    };
    let mut out = Vec::new();
    for m in list_of(tag, "AttributeModifiers").into_iter().filter_map(Tag::as_compound) {
        let (Some(name), Some(amount)) = (m.get("AttributeName").and_then(Tag::as_str), m.get("Amount").and_then(Tag::as_f64)) else { continue };
        let operation = match m.get("Operation").and_then(Tag::as_i64).unwrap_or(0) {
            1 => "add_multiplied_base",
            2 => "add_multiplied_total",
            _ => "add_value",
        };
        let label: String = m
            .get("Name")
            .and_then(Tag::as_str)
            .unwrap_or("modifier")
            .to_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') { c } else { '_' })
            .collect();
        let mut e = Compound::new();
        e.insert("type".into(), Tag::String(format!("minecraft:{}", snake(name))));
        e.insert("id".into(), Tag::String(format!("mctoolkit:{label}")));
        e.insert("amount".into(), Tag::Double(amount));
        e.insert("operation".into(), Tag::String(operation.into()));
        e.insert("slot".into(), Tag::String(m.get("Slot").and_then(Tag::as_str).unwrap_or("any").to_string()));
        out.push(e);
    }
    out
}

fn bedrock_tag(java: &str, _data: i16, tag: &Compound, components: &mut Compound, notes: &mut Notes) {
    for (key, value) in tag {
        match key.as_str() {
            "Damage" => {
                if let Some(d) = value.as_i64() {
                    components.insert("minecraft:damage".into(), Tag::Int(d as i32));
                }
            }
            "Unbreakable" => {
                if value.as_i64() == Some(1) {
                    components.insert("minecraft:unbreakable".into(), Tag::Compound(Compound::new()));
                }
            }
            "RepairCost" => {
                if let Some(c) = value.as_i64() {
                    components.insert("minecraft:repair_cost".into(), Tag::Int(c as i32));
                }
            }
            "display" => {
                if let Some(d) = value.as_compound() {
                    if let Some(n) = d.get("Name").and_then(Tag::as_str) {
                        components.insert("minecraft:custom_name".into(), text_codec::from_legacy(n));
                    }
                    let lore: Vec<Tag> = list_of(d, "Lore").into_iter().filter_map(|l| l.as_str()).map(text_codec::from_legacy).collect();
                    if !lore.is_empty() {
                        components.insert("minecraft:lore".into(), Tag::List(List::from_tags(lore[0].tag_type(), lore).expect("uniform")));
                    }
                }
            }
            "ench" | "Enchantments" => {
                if let Some(e) = bedrock_enchantments(tag, key) {
                    components.entry("minecraft:enchantments".to_string()).or_insert(e);
                }
            }
            "AttributeModifiers" => {
                let list = java_style_attributes(tag);
                if !list.is_empty() {
                    components.insert("minecraft:attribute_modifiers".into(), compounds(list));
                }
            }
            "StoredEnchantments" => {
                if let Some(e) = bedrock_enchantments(tag, "StoredEnchantments") {
                    components.insert("minecraft:stored_enchantments".into(), e);
                }
            }
            "Trim" => {
                if let Some(t) = value.as_compound() {
                    let name = |k: &str| t.get(k).and_then(Tag::as_str).map(ns);
                    if let (Some(material), Some(pattern)) = (name("Material"), name("Pattern")) {
                        let mut c = Compound::new();
                        c.insert("pattern".into(), Tag::String(pattern));
                        c.insert("material".into(), Tag::String(material));
                        components.insert("minecraft:trim".into(), Tag::Compound(c));
                    }
                }
            }
            "CanPlaceOn" | "CanDestroy" => {
                let names: Vec<String> = list_of(tag, key).into_iter().filter_map(Tag::as_str).map(ns).collect();
                if !names.is_empty() {
                    let mut predicate = Compound::new();
                    predicate.insert("blocks".into(), strings(names));
                    components.insert(if key == "CanDestroy" { "minecraft:can_break" } else { "minecraft:can_place_on" }.into(), Tag::Compound(predicate));
                }
            }
            "Fireworks" => {
                if let Some(f) = value.as_compound() {
                    let explosions: Vec<Compound> = list_of(f, "Explosions").into_iter().filter_map(Tag::as_compound).map(explosion_to_java).collect();
                    let mut c = Compound::new();
                    c.insert("flight_duration".into(), Tag::Byte(int(f, "Flight").unwrap_or(1) as i8));
                    c.insert("explosions".into(), compounds(explosions));
                    components.insert("minecraft:fireworks".into(), Tag::Compound(c));
                }
            }
            "FireworksItem" => {
                if let Some(e) = value.as_compound() {
                    components.insert("minecraft:firework_explosion".into(), Tag::Compound(explosion_to_java(e)));
                }
            }
            "chargedItem" => {
                if let Some(stack) = value.as_compound() {
                    if let Some(j) = bedrock_to_java(stack, None, notes) {
                        components.insert("minecraft:charged_projectiles".into(), compounds(vec![j]));
                    }
                }
            }
            "customColor" => {
                if let Some(c) = value.as_i64() {
                    components.insert("minecraft:dyed_color".into(), Tag::Int((c as u32 & 0x00ff_ffff) as i32));
                }
            }
            "map_uuid" => {
                if let Some(id) = value.as_i64() {
                    components.insert("minecraft:map_id".into(), Tag::Int(id as i32));
                }
            }
            "pages" | "title" | "author" | "generation" => {
                if key == "pages" {
                    let mut c = Compound::new();
                    let pages: Vec<Compound> = list_of(tag, "pages")
                        .into_iter()
                        .map(|p| {
                            let text = p.get("text").and_then(Tag::as_str).unwrap_or("");
                            let mut page = Compound::new();
                            page.insert("raw".into(), Tag::String(text.to_string()));
                            page
                        })
                        .collect();
                    let written = bare(java) == "written_book";
                    if written {
                        if let Some(t) = tag.get("title").and_then(Tag::as_str) {
                            let mut title = Compound::new();
                            title.insert("raw".into(), Tag::String(t.to_string()));
                            c.insert("title".into(), Tag::Compound(title));
                        }
                        c.insert("author".into(), Tag::String(tag.get("author").and_then(Tag::as_str).unwrap_or("").to_string()));
                    }
                    c.insert("pages".into(), compounds(pages));
                    let name = if written { "minecraft:written_book_content" } else { "minecraft:writable_book_content" };
                    components.insert(name.into(), Tag::Compound(c));
                }
            }
            "Items" => {
                let mut held = Vec::new();
                for stack in list_of(tag, "Items").into_iter().filter_map(Tag::as_compound) {
                    let slot = int(stack, "Slot").unwrap_or(0);
                    if let Some(mut j) = bedrock_to_java(stack, None, notes) {
                        j.shift_remove("Slot");
                        let mut entry = Compound::new();
                        entry.insert("slot".into(), Tag::Int(slot as i32));
                        entry.insert("item".into(), Tag::Compound(j));
                        held.push(entry);
                    }
                }
                if !held.is_empty() {
                    components.insert("minecraft:container".into(), compounds(held));
                }
            }
            other => notes.miss(format!("item tag {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Tag {
        Tag::String(v.to_string())
    }

    fn sword() -> Compound {
        let mut ench = Compound::new();
        ench.insert("minecraft:sharpness".into(), Tag::Int(5));
        let mut comps = Compound::new();
        comps.insert("minecraft:enchantments".into(), Tag::Compound(ench));
        comps.insert("minecraft:damage".into(), Tag::Int(12));
        comps.insert("minecraft:custom_name".into(), s("Slayer"));
        comps.insert("minecraft:unbreakable".into(), Tag::Compound(Compound::new()));
        let mut item = Compound::new();
        item.insert("id".into(), s("minecraft:diamond_sword"));
        item.insert("count".into(), Tag::Int(1));
        item.insert("components".into(), Tag::Compound(comps));
        item
    }

    #[test]
    fn a_sword_keeps_its_properties_both_ways() {
        let mut notes = Notes::default();
        let b = java_to_bedrock(&sword(), Some(3), &mut notes).unwrap();
        assert_eq!(b.get("Name"), Some(&s("minecraft:diamond_sword")));
        assert_eq!(b.get("Slot"), Some(&Tag::Byte(3)));
        let tag = b.get("tag").and_then(Tag::as_compound).unwrap();
        assert_eq!(tag.get("Damage"), Some(&Tag::Int(12)));
        assert_eq!(tag.get("Unbreakable"), Some(&Tag::Byte(1)));
        let ench = tag.get("ench").and_then(Tag::as_list).unwrap();
        assert_eq!(ench.get(0).unwrap().get("id"), Some(&Tag::Short(9)));
        assert_eq!(ench.get(0).unwrap().get("lvl"), Some(&Tag::Short(5)));
        assert_eq!(tag.get("display").unwrap().get("Name"), Some(&s("Slayer")));
        assert!(notes.missing.is_empty(), "{:?}", notes.missing);

        let back = bedrock_to_java(&b, Some(3), &mut notes).unwrap();
        let c = back.get("components").and_then(Tag::as_compound).unwrap();
        assert_eq!(back.get("id"), Some(&s("minecraft:diamond_sword")));
        assert_eq!(c.get("minecraft:damage"), Some(&Tag::Int(12)));
        assert_eq!(c.get("minecraft:custom_name"), Some(&s("Slayer")));
        assert_eq!(c.get("minecraft:enchantments").unwrap().get("minecraft:sharpness"), Some(&Tag::Int(5)));
        assert!(c.get("minecraft:unbreakable").is_some());
    }

    /// Enchantments and attributes written the Java way on a Bedrock item (as an earlier NBT editor
    /// did) are carried to Java, not left out.
    #[test]
    fn java_style_tags_on_a_bedrock_item_are_carried() {
        let mut ench = Compound::new();
        ench.insert("id".into(), s("minecraft:protection"));
        ench.insert("lvl".into(), Tag::Short(16));
        let mut modifier = Compound::new();
        modifier.insert("AttributeName".into(), s("generic.movementSpeed"));
        modifier.insert("Name".into(), s("mctoolkit movement_speed"));
        modifier.insert("Amount".into(), Tag::Double(15.0));
        modifier.insert("Operation".into(), Tag::Int(0));
        modifier.insert("Slot".into(), s("feet"));
        let mut tag = Compound::new();
        tag.insert("Enchantments".into(), Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(ench)]).unwrap()));
        tag.insert("AttributeModifiers".into(), Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(modifier)]).unwrap()));
        let mut item = Compound::new();
        item.insert("Name".into(), s("minecraft:diamond_boots"));
        item.insert("Count".into(), Tag::Byte(1));
        item.insert("Damage".into(), Tag::Short(0));
        item.insert("tag".into(), Tag::Compound(tag));
        let mut notes = Notes::default();
        let j = bedrock_to_java(&item, None, &mut notes).unwrap();
        let c = j.get("components").and_then(Tag::as_compound).unwrap();
        assert_eq!(c.get("minecraft:enchantments").unwrap().get("minecraft:protection"), Some(&Tag::Int(16)));
        let attr = c.get("minecraft:attribute_modifiers").and_then(Tag::as_list).and_then(|l| l.get(0)).expect("an attribute");
        assert_eq!(attr.get("type"), Some(&s("minecraft:movement_speed")));
        assert_eq!(attr.get("slot"), Some(&s("feet")));
        assert!(notes.missing.is_empty(), "{:?}", notes.missing);
    }

    fn comp(pairs: Vec<(&str, Tag)>) -> Tag {
        let mut c = Compound::new();
        for (k, v) in pairs {
            c.insert(k.into(), v);
        }
        Tag::Compound(c)
    }

    /// Armor trims, the blocks an item can be placed on or break, fireworks, firework stars and a
    /// loaded crossbow go to Bedrock and come back.
    #[test]
    fn trims_block_lists_fireworks_and_loaded_crossbows_round_trip() {
        let explosion = |colors: Vec<i32>| {
            comp(vec![
                ("shape", s("star")),
                ("colors", Tag::List(List::from_tags(TagType::Int, colors.into_iter().map(Tag::Int).collect()).unwrap())),
                ("fade_colors", Tag::List(List::new(TagType::End))),
                ("has_trail", Tag::Byte(1)),
                ("has_twinkle", Tag::Byte(0)),
            ])
        };
        let mut notes = Notes::default();
        // Boots with a trim, that can only be placed on stone.
        let mut boots = Compound::new();
        boots.insert("id".into(), s("minecraft:diamond_boots"));
        boots.insert("count".into(), Tag::Int(1));
        let mut comps = Compound::new();
        comps.insert("minecraft:trim".into(), comp(vec![("pattern", s("minecraft:sentry")), ("material", s("minecraft:iron"))]));
        comps.insert("minecraft:can_place_on".into(), comp(vec![("blocks", Tag::List(List::from_tags(TagType::String, vec![s("minecraft:stone"), s("minecraft:dirt")]).unwrap()))]));
        boots.insert("components".into(), Tag::Compound(comps));
        let b = java_to_bedrock(&boots, None, &mut notes).unwrap();
        let tag = b.get("tag").and_then(Tag::as_compound).unwrap();
        assert_eq!(tag.get("Trim").unwrap().get("Pattern"), Some(&s("sentry")));
        assert_eq!(tag.get("CanPlaceOn").and_then(Tag::as_list).map(|l| l.len()), Some(2));
        let back = bedrock_to_java(&b, None, &mut notes).unwrap();
        let c = back.get("components").and_then(Tag::as_compound).unwrap();
        assert_eq!(c.get("minecraft:trim").unwrap().get("material"), Some(&s("minecraft:iron")));
        assert_eq!(c.get("minecraft:can_place_on").unwrap().get("blocks").and_then(Tag::as_list).map(|l| l.len()), Some(2));
        // A rocket with a red star (the dye's own color comes back).
        let mut rocket = Compound::new();
        rocket.insert("id".into(), s("minecraft:firework_rocket"));
        rocket.insert("count".into(), Tag::Int(1));
        let mut comps = Compound::new();
        comps.insert("minecraft:fireworks".into(), comp(vec![("flight_duration", Tag::Byte(2)), ("explosions", Tag::List(List::from_tags(TagType::Compound, vec![explosion(vec![11_743_532])]).unwrap()))]));
        rocket.insert("components".into(), Tag::Compound(comps));
        let b = java_to_bedrock(&rocket, None, &mut notes).unwrap();
        let f = b.get("tag").and_then(|t| t.get("Fireworks")).unwrap();
        assert_eq!(f.get("Flight"), Some(&Tag::Byte(2)));
        let back = bedrock_to_java(&b, None, &mut notes).unwrap();
        let ex = back.get("components").and_then(|c| c.get("minecraft:fireworks")).and_then(|f| f.get("explosions")).and_then(Tag::as_list).and_then(|l| l.get(0)).unwrap().clone();
        assert_eq!(ex.get("shape"), Some(&s("star")));
        assert_eq!(ex.get("colors").and_then(Tag::as_list).and_then(|l| l.get(0)), Some(&Tag::Int(11_743_532)));
        assert_eq!(ex.get("has_trail"), Some(&Tag::Byte(1)));
        // A crossbow loaded with an arrow.
        let mut arrow = Compound::new();
        arrow.insert("id".into(), s("minecraft:arrow"));
        arrow.insert("count".into(), Tag::Int(1));
        let mut bow = Compound::new();
        bow.insert("id".into(), s("minecraft:crossbow"));
        bow.insert("count".into(), Tag::Int(1));
        let mut comps = Compound::new();
        comps.insert("minecraft:charged_projectiles".into(), Tag::List(List::from_tags(TagType::Compound, vec![Tag::Compound(arrow)]).unwrap()));
        bow.insert("components".into(), Tag::Compound(comps));
        let b = java_to_bedrock(&bow, None, &mut notes).unwrap();
        assert!(b.get("tag").and_then(|t| t.get("chargedItem")).is_some());
        let back = bedrock_to_java(&b, None, &mut notes).unwrap();
        let loaded = back.get("components").and_then(|c| c.get("minecraft:charged_projectiles")).and_then(Tag::as_list).and_then(|l| l.get(0)).unwrap().clone();
        assert_eq!(loaded.get("id"), Some(&s("minecraft:arrow")));
        assert!(notes.missing.is_empty(), "{:?}", notes.missing);
    }

    #[test]
    fn variants_potions_and_unknown_items() {
        let mut notes = Notes::default();
        let mut bed = Compound::new();
        bed.insert("id".into(), s("minecraft:red_bed"));
        bed.insert("count".into(), Tag::Int(2));
        let b = java_to_bedrock(&bed, None, &mut notes).unwrap();
        assert_eq!(b.get("Name"), Some(&s("minecraft:bed")));
        assert_eq!(b.get("Damage"), Some(&Tag::Short(14)));
        assert_eq!(bedrock_to_java(&b, None, &mut notes).unwrap().get("id"), Some(&s("minecraft:red_bed")));

        let mut potion = Compound::new();
        potion.insert("id".into(), s("minecraft:potion"));
        potion.insert("count".into(), Tag::Int(1));
        let mut comps = Compound::new();
        comps.insert("minecraft:potion_contents".into(), potion_value("strong_healing"));
        potion.insert("components".into(), Tag::Compound(comps));
        let b = java_to_bedrock(&potion, None, &mut notes).unwrap();
        assert_eq!(b.get("Damage"), Some(&Tag::Short(22)));
        let j = bedrock_to_java(&b, None, &mut notes).unwrap();
        let pc = j.get("components").unwrap().get("minecraft:potion_contents").unwrap();
        assert_eq!(pc.get("potion"), Some(&s("minecraft:strong_healing")));

        let mut odd = Compound::new();
        odd.insert("id".into(), s("minecraft:not_an_item"));
        odd.insert("count".into(), Tag::Int(1));
        assert!(java_to_bedrock(&odd, None, &mut notes).is_none());
        assert_eq!(notes.missing.get("unknown item minecraft:not_an_item"), Some(&1));
    }

    #[test]
    fn shulker_contents_go_through() {
        let mut inner = Compound::new();
        inner.insert("id".into(), s("minecraft:diamond"));
        inner.insert("count".into(), Tag::Int(5));
        let mut entry = Compound::new();
        entry.insert("slot".into(), Tag::Int(2));
        entry.insert("item".into(), Tag::Compound(inner));
        let mut comps = Compound::new();
        comps.insert("minecraft:container".into(), compounds(vec![entry]));
        let mut shulker = Compound::new();
        shulker.insert("id".into(), s("minecraft:shulker_box"));
        shulker.insert("count".into(), Tag::Int(1));
        shulker.insert("components".into(), Tag::Compound(comps));
        let mut notes = Notes::default();
        let b = java_to_bedrock(&shulker, None, &mut notes).unwrap();
        let items = b.get("tag").unwrap().get("Items").and_then(Tag::as_list).unwrap();
        assert_eq!(items.get(0).unwrap().get("Slot"), Some(&Tag::Byte(2)));
        let back = bedrock_to_java(&b, None, &mut notes).unwrap();
        let held = back.get("components").unwrap().get("minecraft:container").and_then(Tag::as_list).unwrap();
        assert_eq!(held.get(0).unwrap().get("slot"), Some(&Tag::Int(2)));
        assert_eq!(held.get(0).unwrap().get("item").unwrap().get("id"), Some(&s("minecraft:diamond")));
    }

}
