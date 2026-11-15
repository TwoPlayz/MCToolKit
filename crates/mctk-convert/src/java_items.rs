//! Item stacks between the classic NBT form (`{id, Count, tag}`, 1.20.4 and older) and the
//! component form (`{id, count, components}`, 1.20.5 and later).
//!
//! Every rule here was checked against the output of a real 26.3 server upgrading a real
//! 1.20.1 world. Tag entries that have no component of their own go to
//! `minecraft:custom_data` and come back from it, as the game does. What a version cannot
//! express is dropped and counted in [`Notes`].

use indexmap::IndexMap;
use mctk_nbt::{Compound, List, Tag, TagType};

use crate::java_text::{json_to_nbt, mixed_list, nbt_to_json, unwrap_str};

/// Things that could not be carried over, with how often.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Notes(pub IndexMap<String, u64>);

impl Notes {
    pub fn add(&mut self, what: impl Into<String>) {
        *self.0.entry(what.into()).or_insert(0) += 1;
    }

    pub fn merge(&mut self, other: Notes) {
        for (k, v) in other.0 {
            *self.0.entry(k).or_insert(0) += v;
        }
    }
}

pub(crate) const DYES: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray",
    "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// Legacy numeric effect ids 1..=33 (index + 1).
const EFFECTS: [&str; 33] = [
    "speed", "slowness", "haste", "mining_fatigue", "strength", "instant_health", "instant_damage",
    "jump_boost", "nausea", "regeneration", "resistance", "fire_resistance", "water_breathing",
    "invisibility", "blindness", "night_vision", "hunger", "weakness", "poison", "wither",
    "health_boost", "absorption", "saturation", "glowing", "levitation", "luck", "unluck",
    "slow_falling", "conduit_power", "dolphins_grace", "bad_omen", "hero_of_the_village", "darkness",
];

const FIREWORK_SHAPES: [&str; 5] = ["small_ball", "large_ball", "star", "creeper", "burst"];
const OPERATIONS: [&str; 3] = ["add_value", "add_multiplied_base", "add_multiplied_total"];
/// Attributes that carried a `generic.` prefix before 1.21.2.
const GENERIC_ATTRIBUTES: [&str; 11] = [
    "max_health", "follow_range", "knockback_resistance", "movement_speed", "flying_speed",
    "attack_damage", "attack_knockback", "attack_speed", "armor", "armor_toughness", "luck",
];
/// Bits of the old `HideFlags`, as the component each one hides.
const HIDE_FLAGS: [(i64, &str); 6] = [
    (1, "minecraft:enchantments"),
    (2, "minecraft:attribute_modifiers"),
    (4, "minecraft:unbreakable"),
    (8, "minecraft:can_break"),
    (16, "minecraft:can_place_on"),
    (64, "minecraft:dyed_color"),
];

pub(crate) fn s(v: &str) -> Tag {
    Tag::String(v.to_string())
}

fn int(c: &Compound, k: &str) -> Option<i64> {
    c.get(k).and_then(Tag::as_i64)
}

fn list_of(tags: Vec<Tag>, ty: TagType) -> Tag {
    Tag::List(List::from_tags(ty, tags).expect("uniform"))
}

fn strip_ns(id: &str) -> &str {
    id.strip_prefix("minecraft:").unwrap_or(id)
}

// ---- shared pieces: effects, attributes, UUIDs ------------------------------------------

/// `[I;a,b,c,d]` as the dashed hex id the component form uses.
pub(crate) fn uuid_to_id(uuid: &[i32]) -> Option<String> {
    let [a, b, c, d] = <[i32; 4]>::try_from(uuid).ok()?;
    let (a, b, c, d) = (a as u32, b as u32, c as u32, d as u32);
    Some(format!(
        "minecraft:{a:08x}-{:04x}-{:04x}-{:04x}-{:04x}{d:08x}",
        b >> 16,
        b & 0xffff,
        c >> 16,
        c & 0xffff
    ))
}

/// Inverse of [`uuid_to_id`]; other ids get a stable UUID derived from the text.
pub(crate) fn id_to_uuid(id: &str) -> Vec<i32> {
    let hex: String = strip_ns(id).chars().filter(|c| *c != '-').collect();
    if hex.len() == 32 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return (0..4)
            .map(|i| u32::from_str_radix(&hex[i * 8..i * 8 + 8], 16).unwrap_or(0) as i32)
            .collect();
    }
    let mut h: u64 = 0xcbf29ce484222325;
    let mut out = Vec::new();
    for round in 0..4u64 {
        for b in id.bytes().chain(round.to_le_bytes()) {
            h = (h ^ b as u64).wrapping_mul(0x100000001b3);
        }
        out.push((h >> 16) as i32);
    }
    out
}

pub(crate) fn attribute_up(name: &str) -> String {
    let bare = name.strip_prefix("minecraft:").unwrap_or(name);
    let bare = bare
        .strip_prefix("generic.")
        .or_else(|| bare.strip_prefix("zombie."))
        .or_else(|| bare.strip_prefix("horse."))
        .unwrap_or(bare);
    format!("minecraft:{bare}")
}

pub(crate) fn attribute_down(name: &str) -> Option<String> {
    let bare = strip_ns(name);
    if GENERIC_ATTRIBUTES.contains(&bare) {
        Some(format!("generic.{bare}"))
    } else if bare == "spawn_reinforcements" {
        Some("zombie.spawn_reinforcements".into())
    } else if bare == "jump_strength" {
        Some("horse.jump_strength".into())
    } else {
        None
    }
}

pub(crate) fn effect_up(e: &Compound) -> Compound {
    let mut out = Compound::new();
    let id = match e.get("Id").and_then(Tag::as_i64) {
        Some(n) if (1..=33).contains(&n) => format!("minecraft:{}", EFFECTS[n as usize - 1]),
        Some(n) => format!("minecraft:effect_{n}"),
        None => "minecraft:speed".into(),
    };
    out.insert("id".into(), Tag::String(id));
    out.insert("amplifier".into(), Tag::Byte(int(e, "Amplifier").unwrap_or(0) as i8));
    out.insert("duration".into(), Tag::Int(int(e, "Duration").unwrap_or(0) as i32));
    if int(e, "Ambient") == Some(1) {
        out.insert("ambient".into(), Tag::Byte(1));
    }
    if int(e, "ShowParticles") == Some(0) {
        out.insert("show_particles".into(), Tag::Byte(0));
    }
    if int(e, "ShowIcon") == Some(0) {
        out.insert("show_icon".into(), Tag::Byte(0));
    } else {
        out.insert("show_icon".into(), Tag::Byte(1));
    }
    out
}

pub(crate) fn effect_down(e: &Compound, notes: &mut Notes) -> Option<Compound> {
    let id = e.get("id").and_then(Tag::as_str)?;
    let Some(n) = EFFECTS.iter().position(|x| *x == strip_ns(id)) else {
        notes.add(format!("effect {id} does not exist before 1.21"));
        return None;
    };
    let mut out = Compound::new();
    out.insert("Id".into(), Tag::Byte(n as i8 + 1));
    out.insert("Amplifier".into(), Tag::Byte(int(e, "amplifier").unwrap_or(0) as i8));
    out.insert("Duration".into(), Tag::Int(int(e, "duration").unwrap_or(0) as i32));
    out.insert("Ambient".into(), Tag::Byte(int(e, "ambient").unwrap_or(0) as i8));
    out.insert("ShowParticles".into(), Tag::Byte(int(e, "show_particles").unwrap_or(1) as i8));
    out.insert("ShowIcon".into(), Tag::Byte(int(e, "show_icon").unwrap_or(1) as i8));
    Some(out)
}

// ---- classic -> component form ----------------------------------------------------------

fn enchants_up(list: &Tag, notes: &mut Notes) -> Tag {
    let mut m = Compound::new();
    for e in list.as_list().into_iter().flat_map(List::iter) {
        let (Some(c), Some(id)) = (e.as_compound(), e.get("id").and_then(Tag::as_str)) else {
            continue;
        };
        let id = if id == "minecraft:sweeping" { "minecraft:sweeping_edge" } else { id };
        let lvl = int(c, "lvl").unwrap_or(1).clamp(0, 255) as i32;
        if m.insert(id.to_string(), Tag::Int(lvl)).is_some() {
            notes.add("duplicate enchantment merged");
        }
    }
    Tag::Compound(m)
}

/// Firework colors are an int array in the classic form and a list of ints in the modern one.
fn colors_as_list(t: Option<&Tag>) -> Tag {
    let ints: Vec<i32> = match t {
        Some(Tag::IntArray(a)) => a.clone(),
        Some(Tag::List(l)) => l.iter().filter_map(Tag::as_i64).map(|v| v as i32).collect(),
        _ => vec![],
    };
    list_of(ints.into_iter().map(Tag::Int).collect(), TagType::Int)
}

fn colors_as_array(t: Option<&Tag>) -> Tag {
    match colors_as_list(t) {
        Tag::List(l) => Tag::IntArray(l.iter().filter_map(Tag::as_i64).map(|v| v as i32).collect()),
        _ => Tag::IntArray(vec![]),
    }
}

fn explosion_up(e: &Compound) -> Compound {
    let mut out = Compound::new();
    let shape = int(e, "Type").and_then(|t| FIREWORK_SHAPES.get(t as usize)).unwrap_or(&"small_ball");
    out.insert("shape".into(), s(shape));
    out.insert("colors".into(), colors_as_list(e.get("Colors")));
    if let Some(f) = e.get("FadeColors") {
        out.insert("fade_colors".into(), colors_as_list(Some(f)));
    }
    if int(e, "Trail") == Some(1) {
        out.insert("has_trail".into(), Tag::Byte(1));
    }
    if int(e, "Flicker") == Some(1) {
        out.insert("has_twinkle".into(), Tag::Byte(1));
    }
    out
}

pub(crate) fn profile_up(owner: &Tag) -> Option<Tag> {
    let mut p = Compound::new();
    match owner {
        Tag::String(name) => {
            p.insert("name".into(), s(name));
        }
        Tag::Compound(c) => {
            if let Some(n) = c.get("Name").and_then(Tag::as_str) {
                p.insert("name".into(), s(n));
            }
            if let Some(Tag::IntArray(id)) = c.get("Id") {
                p.insert("id".into(), Tag::IntArray(id.clone()));
            }
            if let Some(Tag::List(l)) = c.get("Properties").and_then(|p| p.get("textures")) {
                let props = l
                    .iter()
                    .filter_map(|t| {
                        let t = t.as_compound()?;
                        let mut o = Compound::new();
                        o.insert("name".into(), s("textures"));
                        o.insert("value".into(), t.get("Value")?.clone());
                        if let Some(sig) = t.get("Signature") {
                            o.insert("signature".into(), sig.clone());
                        }
                        Some(Tag::Compound(o))
                    })
                    .collect();
                p.insert("properties".into(), list_of(props, TagType::Compound));
            }
        }
        _ => return None,
    }
    Some(Tag::Compound(p))
}

fn modifiers_up(list: &Tag, notes: &mut Notes) -> Tag {
    let mut out = Vec::new();
    for m in list.as_list().into_iter().flat_map(List::iter) {
        let Some(m) = m.as_compound() else { continue };
        let Some(attr) = m.get("AttributeName").and_then(Tag::as_str) else { continue };
        let mut o = Compound::new();
        o.insert("type".into(), Tag::String(attribute_up(attr)));
        let id = match m.get("UUID") {
            Some(Tag::IntArray(u)) => uuid_to_id(u),
            _ => None,
        }
        .unwrap_or_else(|| {
            notes.add("attribute modifier without UUID");
            format!("minecraft:{}", strip_ns(attr).replace('.', "_"))
        });
        o.insert("id".into(), Tag::String(id));
        o.insert("amount".into(), Tag::Double(m.get("Amount").and_then(Tag::as_f64).unwrap_or(0.0)));
        let op = int(m, "Operation").unwrap_or(0).clamp(0, 2) as usize;
        o.insert("operation".into(), s(OPERATIONS[op]));
        o.insert("slot".into(), s(m.get("Slot").and_then(Tag::as_str).unwrap_or("any")));
        out.push(Tag::Compound(o));
    }
    list_of(out, TagType::Compound)
}

fn potion_effects_up(list: &Tag) -> Tag {
    let effs = list
        .as_list()
        .into_iter()
        .flat_map(List::iter)
        .filter_map(|e| e.as_compound().map(|e| Tag::Compound(effect_up(e))))
        .collect();
    list_of(effs, TagType::Compound)
}

fn filterable(text: Tag) -> Tag {
    Tag::Compound(Compound::from([("raw".to_string(), text)]))
}

/// Container contents (`Items` with a `Slot` byte) as the component form's slot list.
fn container_up(items: &Tag, notes: &mut Notes) -> Tag {
    let entries = items
        .as_list()
        .into_iter()
        .flat_map(List::iter)
        .filter_map(Tag::as_compound)
        .map(|stack| {
            let mut item = item_up(stack, notes);
            let slot = item.shift_remove("Slot").and_then(|t| t.as_i64()).unwrap_or(0);
            Tag::Compound(Compound::from([
                ("item".to_string(), Tag::Compound(item)),
                ("slot".to_string(), Tag::Int(slot as i32)),
            ]))
        })
        .collect();
    list_of(entries, TagType::Compound)
}

/// Converts one classic item stack. Empty stacks (`{}`) are returned unchanged.
pub fn item_up(stack: &Compound, notes: &mut Notes) -> Compound {
    let Some(id) = stack.get("id").and_then(Tag::as_str) else {
        return stack.clone();
    };
    let mut out = Compound::new();
    out.insert("id".into(), s(id));
    out.insert("count".into(), Tag::Int(int(stack, "Count").or(int(stack, "count")).unwrap_or(1) as i32));
    if let Some(slot) = stack.get("Slot") {
        out.insert("Slot".into(), slot.clone());
    }
    let Some(tag) = stack.get("tag").and_then(Tag::as_compound) else {
        return out;
    };

    let mut comps = Compound::new();
    let mut custom = Compound::new();
    let mut potion = Compound::new();
    let book = matches!(strip_ns(id), "written_book" | "writable_book");
    let mut book_content = Compound::new();
    let mut hide_flags = 0i64;
    let mut lore_done = false;
    for (k, v) in tag {
        match (k.as_str(), v) {
            ("Damage", _) => {
                if int(tag, "Damage").unwrap_or(0) > 0 {
                    comps.insert("minecraft:damage".into(), Tag::Int(int(tag, "Damage").unwrap_or(0) as i32));
                }
            }
            ("RepairCost", _) => {
                comps.insert("minecraft:repair_cost".into(), Tag::Int(v.as_i64().unwrap_or(0) as i32));
            }
            ("Unbreakable", _) => {
                if v.as_i64().unwrap_or(0) != 0 {
                    comps.insert("minecraft:unbreakable".into(), Tag::Compound(Compound::new()));
                }
            }
            ("HideFlags", _) => hide_flags = v.as_i64().unwrap_or(0),
            ("CustomModelData", _) => {
                let n = v.as_i64().unwrap_or(0) as f32;
                comps.insert(
                    "minecraft:custom_model_data".into(),
                    Tag::Compound(Compound::from([("floats".to_string(), list_of(vec![Tag::Float(n)], TagType::Float))])),
                );
            }
            ("display", Tag::Compound(d)) => {
                if let Some(name) = d.get("Name").and_then(Tag::as_str) {
                    comps.insert("minecraft:custom_name".into(), json_to_nbt(name));
                }
                if let Some(Tag::List(l)) = d.get("Lore") {
                    let lines = l.iter().filter_map(Tag::as_str).map(json_to_nbt).collect();
                    comps.insert("minecraft:lore".into(), mixed_list(lines));
                    lore_done = true;
                }
                if let Some(c) = d.get("color") {
                    comps.insert("minecraft:dyed_color".into(), c.clone());
                }
                if let Some(c) = d.get("MapColor") {
                    comps.insert("minecraft:map_color".into(), c.clone());
                }
            }
            ("Enchantments", _) => {
                comps.insert("minecraft:enchantments".into(), enchants_up(v, notes));
            }
            ("StoredEnchantments", _) => {
                comps.insert("minecraft:stored_enchantments".into(), enchants_up(v, notes));
            }
            ("Potion", _) => {
                potion.insert("potion".into(), v.clone());
            }
            ("CustomPotionColor", _) => {
                potion.insert("custom_color".into(), v.clone());
            }
            ("CustomPotionEffects", _) => {
                potion.insert("custom_effects".into(), potion_effects_up(v));
            }
            ("Trim", _) => {
                comps.insert("minecraft:trim".into(), v.clone());
            }
            ("SkullOwner", _) => match profile_up(v) {
                Some(p) => {
                    comps.insert("minecraft:profile".into(), p);
                }
                None => {
                    custom.insert(k.clone(), v.clone());
                }
            },
            ("Fireworks", Tag::Compound(f)) => {
                let mut o = Compound::new();
                if let Some(fl) = f.get("Flight") {
                    o.insert("flight_duration".into(), Tag::Byte(fl.as_i64().unwrap_or(1) as i8));
                }
                let ex = f
                    .get("Explosions")
                    .and_then(Tag::as_list)
                    .into_iter()
                    .flat_map(List::iter)
                    .filter_map(Tag::as_compound)
                    .map(|e| Tag::Compound(explosion_up(e)))
                    .collect();
                o.insert("explosions".into(), list_of(ex, TagType::Compound));
                comps.insert("minecraft:fireworks".into(), Tag::Compound(o));
            }
            ("Explosion", Tag::Compound(e)) => {
                comps.insert("minecraft:firework_explosion".into(), Tag::Compound(explosion_up(e)));
            }
            ("map", _) => {
                comps.insert("minecraft:map_id".into(), Tag::Int(v.as_i64().unwrap_or(0) as i32));
            }
            ("BlockEntityTag", Tag::Compound(b)) => {
                for (bk, bv) in b {
                    if bk == "Items" {
                        comps.insert("minecraft:container".into(), container_up(bv, notes));
                    } else {
                        notes.add(format!("item block entity data {bk} (kept in custom_data)"));
                        custom
                            .entry("BlockEntityTag".into())
                            .or_insert_with(|| Tag::Compound(Compound::new()))
                            .as_compound_mut()
                            .expect("compound")
                            .insert(bk.clone(), bv.clone());
                    }
                }
            }
            ("ChargedProjectiles", Tag::List(l)) => {
                let items = l.iter().filter_map(Tag::as_compound).map(|c| Tag::Compound(item_up(c, notes))).collect();
                comps.insert("minecraft:charged_projectiles".into(), list_of(items, TagType::Compound));
            }
            ("Charged", _) => {}
            ("AttributeModifiers", _) => {
                comps.insert("minecraft:attribute_modifiers".into(), modifiers_up(v, notes));
            }
            ("title" | "author" | "generation" | "pages" | "resolved", _) if book => {
                book_content.insert(k.clone(), v.clone());
            }
            _ => {
                custom.insert(k.clone(), v.clone());
            }
        }
    }
    let _ = lore_done;

    if !potion.is_empty() {
        comps.insert("minecraft:potion_contents".into(), Tag::Compound(potion));
    }
    if book {
        let pages = book_content
            .get("pages")
            .and_then(Tag::as_list)
            .into_iter()
            .flat_map(List::iter)
            .filter_map(Tag::as_str)
            .map(|p| {
                let text = if strip_ns(id) == "written_book" { json_to_nbt(p) } else { s(p) };
                filterable(text)
            })
            .collect::<Vec<_>>();
        let mut c = Compound::new();
        if strip_ns(id) == "written_book" {
            if let Some(g) = book_content.get("generation").and_then(Tag::as_i64) {
                c.insert("generation".into(), Tag::Int(g as i32));
            }
            c.insert("pages".into(), list_of(pages, TagType::Compound));
            if let Some(a) = book_content.get("author") {
                c.insert("author".into(), a.clone());
            }
            if let Some(t) = book_content.get("title").and_then(Tag::as_str) {
                c.insert("title".into(), filterable(s(t)));
            }
            if let Some(r) = book_content.get("resolved") {
                c.insert("resolved".into(), r.clone());
            }
            comps.insert("minecraft:written_book_content".into(), Tag::Compound(c));
        } else {
            c.insert("pages".into(), list_of(pages, TagType::Compound));
            comps.insert("minecraft:writable_book_content".into(), Tag::Compound(c));
        }
    }
    if hide_flags != 0 {
        let hidden: Vec<Tag> = HIDE_FLAGS.iter().filter(|(bit, _)| hide_flags & bit != 0).map(|(_, n)| s(n)).collect();
        if !hidden.is_empty() {
            comps.insert(
                "minecraft:tooltip_display".into(),
                Tag::Compound(Compound::from([("hidden_components".to_string(), list_of(hidden, TagType::String))])),
            );
        }
    }
    if !custom.is_empty() {
        comps.insert("minecraft:custom_data".into(), Tag::Compound(custom));
    }
    if !comps.is_empty() {
        out.insert("components".into(), Tag::Compound(comps));
    }
    out
}

// ---- component form -> classic ----------------------------------------------------------

fn enchants_down(tag: &Tag, notes: &mut Notes) -> Tag {
    let map = tag
        .get("levels")
        .and_then(Tag::as_compound)
        .or_else(|| tag.as_compound())
        .cloned()
        .unwrap_or_default();
    let mut list = Vec::new();
    for (id, lvl) in map {
        if id == "minecraft:sweeping_edge" || !matches!(strip_ns(&id), "breach" | "density" | "lunge" | "wind_burst") {
            let old = if id == "minecraft:sweeping_edge" { "minecraft:sweeping".to_string() } else { id };
            list.push(Tag::Compound(Compound::from([
                ("id".to_string(), Tag::String(old)),
                ("lvl".to_string(), Tag::Short(lvl.as_i64().unwrap_or(1) as i16)),
            ])));
        } else {
            notes.add(format!("enchantment {id} does not exist before 1.21"));
        }
    }
    list_of(list, TagType::Compound)
}

fn explosion_down(e: &Compound) -> Compound {
    let mut out = Compound::new();
    let shape = e.get("shape").and_then(Tag::as_str).unwrap_or("small_ball");
    let t = FIREWORK_SHAPES.iter().position(|x| *x == shape).unwrap_or(0);
    out.insert("Type".into(), Tag::Byte(t as i8));
    out.insert("Colors".into(), colors_as_array(e.get("colors")));
    if let Some(f) = e.get("fade_colors") {
        out.insert("FadeColors".into(), colors_as_array(Some(f)));
    }
    if int(e, "has_trail") == Some(1) {
        out.insert("Trail".into(), Tag::Byte(1));
    }
    if int(e, "has_twinkle") == Some(1) {
        out.insert("Flicker".into(), Tag::Byte(1));
    }
    out
}

pub(crate) fn profile_down(p: &Tag) -> Option<Tag> {
    match p {
        Tag::String(n) => Some(Tag::Compound(Compound::from([("Name".to_string(), s(n))]))),
        Tag::Compound(c) => {
            let mut o = Compound::new();
            if let Some(n) = c.get("name") {
                o.insert("Name".into(), n.clone());
            }
            if let Some(i) = c.get("id") {
                o.insert("Id".into(), i.clone());
            }
            if let Some(Tag::List(l)) = c.get("properties") {
                let textures = l
                    .iter()
                    .filter_map(Tag::as_compound)
                    .filter(|t| t.get("name").and_then(Tag::as_str) == Some("textures"))
                    .filter_map(|t| {
                        let mut e = Compound::new();
                        e.insert("Value".into(), t.get("value")?.clone());
                        if let Some(sig) = t.get("signature") {
                            e.insert("Signature".into(), sig.clone());
                        }
                        Some(Tag::Compound(e))
                    })
                    .collect();
                o.insert(
                    "Properties".into(),
                    Tag::Compound(Compound::from([("textures".to_string(), list_of(textures, TagType::Compound))])),
                );
            }
            Some(Tag::Compound(o))
        }
        _ => None,
    }
}

fn modifiers_down(list: &Tag, notes: &mut Notes) -> Tag {
    let mut out = Vec::new();
    for m in list.as_list().into_iter().flat_map(List::iter).filter_map(Tag::as_compound) {
        let Some(ty) = m.get("type").and_then(Tag::as_str) else { continue };
        let Some(old) = attribute_down(ty) else {
            notes.add(format!("attribute {ty} does not exist before 1.21"));
            continue;
        };
        let id = m.get("id").and_then(Tag::as_str).unwrap_or("minecraft:modifier");
        let op = m
            .get("operation")
            .and_then(Tag::as_str)
            .and_then(|o| OPERATIONS.iter().position(|x| *x == o))
            .unwrap_or(0);
        let mut o = Compound::new();
        o.insert("AttributeName".into(), Tag::String(old));
        o.insert("Name".into(), s(id));
        o.insert("Amount".into(), Tag::Double(m.get("amount").and_then(Tag::as_f64).unwrap_or(0.0)));
        o.insert("Operation".into(), Tag::Int(op as i32));
        o.insert("UUID".into(), Tag::IntArray(id_to_uuid(id)));
        if let Some(slot) = m.get("slot").and_then(Tag::as_str) {
            if slot != "any" {
                o.insert("Slot".into(), s(slot));
            }
        }
        out.push(Tag::Compound(o));
    }
    list_of(out, TagType::Compound)
}

fn container_down(list: &Tag, notes: &mut Notes) -> Tag {
    let items = list
        .as_list()
        .into_iter()
        .flat_map(List::iter)
        .filter_map(Tag::as_compound)
        .filter_map(|e| {
            let item = e.get("item")?.as_compound()?;
            let mut stack = item_down(item, notes);
            let slot = int(e, "slot").unwrap_or(0);
            let mut with_slot = Compound::from([("Slot".to_string(), Tag::Byte(slot as i8))]);
            with_slot.append(&mut stack);
            Some(Tag::Compound(with_slot))
        })
        .collect();
    list_of(items, TagType::Compound)
}

/// Converts one component-form item stack to the classic form. Empty stacks are unchanged.
pub fn item_down(stack: &Compound, notes: &mut Notes) -> Compound {
    let Some(id) = stack.get("id").and_then(Tag::as_str) else {
        return stack.clone();
    };
    let mut out = Compound::new();
    out.insert("id".into(), s(id));
    out.insert("Count".into(), Tag::Byte(int(stack, "count").unwrap_or(1).clamp(0, 127) as i8));
    if let Some(slot) = stack.get("Slot") {
        out.insert("Slot".into(), slot.clone());
    }
    let Some(comps) = stack.get("components").and_then(Tag::as_compound) else {
        return out;
    };

    let mut tag = Compound::new();
    let mut display = Compound::new();
    let mut hide = 0i64;
    for (k, v) in comps {
        match k.as_str() {
            "minecraft:damage" => {
                tag.insert("Damage".into(), Tag::Int(v.as_i64().unwrap_or(0) as i32));
            }
            "minecraft:repair_cost" => {
                tag.insert("RepairCost".into(), Tag::Int(v.as_i64().unwrap_or(0) as i32));
            }
            "minecraft:unbreakable" => {
                tag.insert("Unbreakable".into(), Tag::Byte(1));
            }
            "minecraft:tooltip_display" => {
                for h in v.get("hidden_components").and_then(Tag::as_list).into_iter().flat_map(List::iter) {
                    if let Some(name) = h.as_str() {
                        match HIDE_FLAGS.iter().find(|(_, n)| *n == name) {
                            Some((bit, _)) => hide |= bit,
                            None => notes.add(format!("hidden tooltip {name} does not exist before 1.21")),
                        }
                    }
                }
            }
            "minecraft:custom_model_data" => {
                let n = v
                    .get("floats")
                    .and_then(Tag::as_list)
                    .and_then(|l| l.get(0))
                    .and_then(Tag::as_f64)
                    .or_else(|| v.as_f64())
                    .unwrap_or(0.0);
                tag.insert("CustomModelData".into(), Tag::Int(n as i32));
            }
            "minecraft:custom_name" => {
                display.insert("Name".into(), Tag::String(nbt_to_json(v)));
            }
            "minecraft:lore" => {
                let lines = v.as_list().into_iter().flat_map(List::iter).map(|t| match unwrap_str(t) {
                    Some(text) => Tag::String(nbt_to_json(&Tag::String(text.to_string()))),
                    None => Tag::String(nbt_to_json(t)),
                });
                display.insert("Lore".into(), list_of(lines.collect(), TagType::String));
            }
            "minecraft:dyed_color" => {
                let rgb = v.get("rgb").cloned().unwrap_or_else(|| v.clone());
                display.insert("color".into(), rgb);
            }
            "minecraft:map_color" => {
                display.insert("MapColor".into(), v.clone());
            }
            "minecraft:enchantments" => {
                tag.insert("Enchantments".into(), enchants_down(v, notes));
            }
            "minecraft:stored_enchantments" => {
                tag.insert("StoredEnchantments".into(), enchants_down(v, notes));
            }
            "minecraft:potion_contents" => {
                if let Some(p) = v.get("potion") {
                    tag.insert("Potion".into(), p.clone());
                }
                if let Some(c) = v.get("custom_color") {
                    tag.insert("CustomPotionColor".into(), c.clone());
                }
                if let Some(Tag::List(l)) = v.get("custom_effects") {
                    let effs = l
                        .iter()
                        .filter_map(Tag::as_compound)
                        .filter_map(|e| effect_down(e, notes))
                        .map(Tag::Compound)
                        .collect();
                    tag.insert("CustomPotionEffects".into(), list_of(effs, TagType::Compound));
                }
            }
            "minecraft:trim" => {
                tag.insert("Trim".into(), v.clone());
            }
            "minecraft:profile" => {
                if let Some(p) = profile_down(v) {
                    tag.insert("SkullOwner".into(), p);
                }
            }
            "minecraft:fireworks" => {
                let mut f = Compound::new();
                if let Some(d) = v.get("flight_duration") {
                    f.insert("Flight".into(), Tag::Byte(d.as_i64().unwrap_or(1) as i8));
                }
                let ex = v
                    .get("explosions")
                    .and_then(Tag::as_list)
                    .into_iter()
                    .flat_map(List::iter)
                    .filter_map(Tag::as_compound)
                    .map(|e| Tag::Compound(explosion_down(e)))
                    .collect();
                f.insert("Explosions".into(), list_of(ex, TagType::Compound));
                tag.insert("Fireworks".into(), Tag::Compound(f));
            }
            "minecraft:firework_explosion" => {
                if let Some(e) = v.as_compound() {
                    tag.insert("Explosion".into(), Tag::Compound(explosion_down(e)));
                }
            }
            "minecraft:map_id" => {
                tag.insert("map".into(), Tag::Int(v.as_i64().unwrap_or(0) as i32));
            }
            "minecraft:container" => {
                let bet = tag
                    .entry("BlockEntityTag".into())
                    .or_insert_with(|| Tag::Compound(Compound::new()));
                bet.as_compound_mut().expect("compound").insert("Items".into(), container_down(v, notes));
            }
            "minecraft:charged_projectiles" => {
                let items = v
                    .as_list()
                    .into_iter()
                    .flat_map(List::iter)
                    .filter_map(Tag::as_compound)
                    .map(|c| Tag::Compound(item_down(c, notes)))
                    .collect::<Vec<_>>();
                if !items.is_empty() {
                    tag.insert("Charged".into(), Tag::Byte(1));
                }
                tag.insert("ChargedProjectiles".into(), list_of(items, TagType::Compound));
            }
            "minecraft:attribute_modifiers" => {
                if let Some(list) = v.as_list().map(|_| v).or_else(|| v.get("modifiers")) {
                    tag.insert("AttributeModifiers".into(), modifiers_down(list, notes));
                }
            }
            "minecraft:written_book_content" => {
                let pages = v
                    .get("pages")
                    .and_then(Tag::as_list)
                    .into_iter()
                    .flat_map(List::iter)
                    .filter_map(|p| p.get("raw").or(Some(p)))
                    .map(|raw| Tag::String(nbt_to_json(raw)))
                    .collect();
                tag.insert("pages".into(), list_of(pages, TagType::String));
                if let Some(t) = v.get("title") {
                    let title = t.get("raw").unwrap_or(t);
                    tag.insert("title".into(), s(unwrap_str(title).unwrap_or("")));
                }
                for k in ["author", "generation", "resolved"] {
                    if let Some(x) = v.get(k) {
                        tag.insert(k.into(), x.clone());
                    }
                }
            }
            "minecraft:writable_book_content" => {
                let pages = v
                    .get("pages")
                    .and_then(Tag::as_list)
                    .into_iter()
                    .flat_map(List::iter)
                    .filter_map(|p| unwrap_str(p.get("raw").unwrap_or(p)).map(s))
                    .collect();
                tag.insert("pages".into(), list_of(pages, TagType::String));
            }
            "minecraft:custom_data" => {
                if let Some(c) = v.as_compound() {
                    for (ck, cv) in c {
                        tag.insert(ck.clone(), cv.clone());
                    }
                }
            }
            other => notes.add(format!("item component {other} does not exist before 1.20.5")),
        }
    }
    if hide != 0 {
        tag.insert("HideFlags".into(), Tag::Int(hide as i32));
    }
    if !display.is_empty() {
        tag.insert("display".into(), Tag::Compound(display));
    }
    if !tag.is_empty() {
        out.insert("tag".into(), Tag::Compound(tag));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(snbt_fields: Vec<(&str, Tag)>) -> Compound {
        snbt_fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn sword() -> Compound {
        let display = Compound::from([
            ("Name".to_string(), s(r#"{"text":"Blade","color":"red","italic":false}"#)),
            (
                "Lore".to_string(),
                list_of(vec![s(r#"{"text":"first line"}"#), s(r#"{"text":"second","color":"blue"}"#)], TagType::String),
            ),
        ]);
        let ench = list_of(
            vec![
                Tag::Compound(Compound::from([("id".to_string(), s("minecraft:sharpness")), ("lvl".to_string(), Tag::Short(3))])),
                Tag::Compound(Compound::from([("id".to_string(), s("minecraft:sweeping")), ("lvl".to_string(), Tag::Short(2))])),
            ],
            TagType::Compound,
        );
        let tag = Compound::from([
            ("Damage".to_string(), Tag::Int(5)),
            ("Unbreakable".to_string(), Tag::Byte(1)),
            ("HideFlags".to_string(), Tag::Int(4)),
            ("CustomModelData".to_string(), Tag::Int(7)),
            ("display".to_string(), Tag::Compound(display)),
            ("Enchantments".to_string(), ench),
            ("MyMod".to_string(), Tag::Int(9)),
        ]);
        stack(vec![
            ("Slot", Tag::Byte(0)),
            ("id", s("minecraft:diamond_sword")),
            ("Count", Tag::Byte(1)),
            ("tag", Tag::Compound(tag)),
        ])
    }

    #[test]
    fn sword_matches_what_the_game_produced() {
        let mut notes = Notes::default();
        let up = item_up(&sword(), &mut notes);
        assert_eq!(up.get("count"), Some(&Tag::Int(1)));
        assert!(up.get("Count").is_none() && up.get("tag").is_none());
        let c = up.get("components").and_then(Tag::as_compound).unwrap();
        assert_eq!(c.get("minecraft:damage"), Some(&Tag::Int(5)));
        assert_eq!(c.get("minecraft:unbreakable"), Some(&Tag::Compound(Compound::new())));
        let ench = c.get("minecraft:enchantments").and_then(Tag::as_compound).unwrap();
        assert_eq!(ench.get("minecraft:sharpness"), Some(&Tag::Int(3)));
        assert_eq!(ench.get("minecraft:sweeping_edge"), Some(&Tag::Int(2)));
        let hidden = c.get("minecraft:tooltip_display").unwrap().get("hidden_components").unwrap();
        assert_eq!(hidden.as_list().unwrap().get(0), Some(&s("minecraft:unbreakable")));
        let model = c.get("minecraft:custom_model_data").unwrap().get("floats").unwrap();
        assert_eq!(model.as_list().unwrap().get(0), Some(&Tag::Float(7.0)));
        // Unknown tag entries are kept as custom data.
        assert_eq!(c.get("minecraft:custom_data").unwrap().get("MyMod"), Some(&Tag::Int(9)));
    }

    #[test]
    fn sword_round_trips() {
        let mut notes = Notes::default();
        let up = item_up(&sword(), &mut notes);
        let down = item_down(&up, &mut notes);
        let orig = sword();
        assert_eq!(down.get("Count"), orig.get("Count"));
        assert_eq!(down.get("Slot"), orig.get("Slot"));
        let (a, b) = (down.get("tag").unwrap(), orig.get("tag").unwrap());
        for k in ["Damage", "Unbreakable", "HideFlags", "CustomModelData", "MyMod", "Enchantments"] {
            assert_eq!(a.get(k), b.get(k), "{k}");
        }
        let (da, db) = (a.get("display").unwrap(), b.get("display").unwrap());
        // JSON text compares by value, not by key order.
        let name = |d: &Tag| serde_json::from_str::<serde_json::Value>(d.get("Name").and_then(Tag::as_str).unwrap()).unwrap();
        assert_eq!(name(da), name(db));
        assert!(notes.0.is_empty(), "{notes:?}");
    }

    #[test]
    fn firework_colors_change_between_array_and_list() {
        let explosion = Compound::from([
            ("Type".to_string(), Tag::Byte(1)),
            ("Colors".to_string(), Tag::IntArray(vec![16711680, 255])),
            ("Flicker".to_string(), Tag::Byte(1)),
        ]);
        let tag = Compound::from([(
            "Fireworks".to_string(),
            Tag::Compound(Compound::from([
                ("Flight".to_string(), Tag::Byte(2)),
                ("Explosions".to_string(), list_of(vec![Tag::Compound(explosion)], TagType::Compound)),
            ])),
        )]);
        let rocket = stack(vec![("id", s("minecraft:firework_rocket")), ("Count", Tag::Byte(16)), ("tag", Tag::Compound(tag))]);
        let mut notes = Notes::default();
        let up = item_up(&rocket, &mut notes);
        let fw = up.get("components").unwrap().get("minecraft:fireworks").unwrap();
        let ex = fw.get("explosions").unwrap().as_list().unwrap().get(0).unwrap();
        assert_eq!(ex.get("shape"), Some(&s("large_ball")));
        assert_eq!(ex.get("colors").unwrap().as_list().unwrap().len(), 2);
        assert_eq!(ex.get("has_twinkle"), Some(&Tag::Byte(1)));
        let down = item_down(&up, &mut notes);
        let ex = down.get("tag").unwrap().get("Fireworks").unwrap().get("Explosions").unwrap().as_list().unwrap().get(0).unwrap().clone();
        assert_eq!(ex.get("Colors"), Some(&Tag::IntArray(vec![16711680, 255])));
    }

    #[test]
    fn uuids_and_attributes() {
        assert_eq!(uuid_to_id(&[1, 2, 3, 4]).unwrap(), "minecraft:00000001-0000-0002-0000-000300000004");
        assert_eq!(id_to_uuid("minecraft:00000001-0000-0002-0000-000300000004"), vec![1, 2, 3, 4]);
        assert_eq!(attribute_up("generic.attack_damage"), "minecraft:attack_damage");
        assert_eq!(attribute_up("horse.jump_strength"), "minecraft:jump_strength");
        assert_eq!(attribute_down("minecraft:spawn_reinforcements").unwrap(), "zombie.spawn_reinforcements");
        assert_eq!(attribute_down("minecraft:scale"), None);
    }

    #[test]
    fn new_only_content_is_dropped_and_counted_going_down() {
        let comps = Compound::from([
            ("minecraft:food".to_string(), Tag::Compound(Compound::new())),
            (
                "minecraft:enchantments".to_string(),
                Tag::Compound(Compound::from([("minecraft:wind_burst".to_string(), Tag::Int(1))])),
            ),
        ]);
        let up = stack(vec![("id", s("minecraft:stick")), ("count", Tag::Int(3)), ("components", Tag::Compound(comps))]);
        let mut notes = Notes::default();
        let down = item_down(&up, &mut notes);
        assert_eq!(down.get("Count"), Some(&Tag::Byte(3)));
        assert_eq!(notes.0.len(), 2, "{notes:?}");
    }

    #[test]
    fn empty_stack_is_untouched() {
        let mut notes = Notes::default();
        assert_eq!(item_up(&Compound::new(), &mut notes), Compound::new());
        assert_eq!(item_down(&Compound::new(), &mut notes), Compound::new());
    }
}
