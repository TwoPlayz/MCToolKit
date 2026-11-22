//! Block entities and entities between the classic NBT form (1.20.4 and older, checked
//! against 1.20.1) and the modern one (1.21.5 and later, checked against 26.3).
//!
//! The rules were taken from a real 26.3 server upgrading a real 1.20.1 world. Anything a
//! version cannot express is dropped and counted in [`Notes`]; ids the target version does
//! not know make the whole entity or block entity drop.

use mctk_nbt::{Compound, List, Tag, TagType};
use mctk_registry::Registry;

use crate::java_items::{
    DYES, Notes, attribute_down, attribute_up, effect_down, effect_up, id_to_uuid, item_down,
    item_up, s, uuid_to_id,
};
use crate::java_era::era_up_item;
use crate::java_text::{json_to_nbt, mixed_list, nbt_to_json};

/// Which NBT conventions a version uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// Up to 1.20.4: item `tag`, JSON text, `ArmorItems`/`HandItems`.
    Classic,
    /// 1.21.5 and later: item components, NBT text, `equipment`.
    Modern,
}

impl Form {
    /// The form a source world's data is read as, and whether it is the in-between era (1.20.5 to
    /// 1.21.4: item components, but JSON text and `HandItems`/`ArmorItems`), which is first
    /// brought up to the modern form by [`era_up_entity`] and [`era_up_block_entity`].
    pub fn source(data_version: i32) -> Option<(Form, bool)> {
        match data_version {
            ..3837 => Some((Form::Classic, false)),
            3837..4325 => Some((Form::Modern, true)),
            _ => Some((Form::Modern, false)),
        }
    }

    /// The form for a `DataVersion`, or `None` for 1.20.5 – 1.21.4, where items are already
    /// components but text and equipment are still classic.
    pub fn of(data_version: i32) -> Option<Form> {
        match data_version {
            ..3837 => Some(Form::Classic),
            4325.. => Some(Form::Modern),
            _ => None,
        }
    }
}

const HAND_SLOTS: [&str; 2] = ["mainhand", "offhand"];
/// `ArmorItems` order: feet, legs, chest, head.
const ARMOR_SLOTS: [&str; 4] = ["feet", "legs", "chest", "head"];
const DEFAULT_DROP_CHANCE: f64 = 0.085;

const BANNER_PATTERNS: [(&str, &str); 41] = [
    ("b", "base"), ("bs", "stripe_bottom"), ("ts", "stripe_top"), ("ls", "stripe_left"),
    ("rs", "stripe_right"), ("cs", "stripe_center"), ("ms", "stripe_middle"),
    ("drs", "stripe_downright"), ("dls", "stripe_downleft"), ("ss", "small_stripes"),
    ("cr", "cross"), ("sc", "straight_cross"), ("bt", "triangle_bottom"), ("tt", "triangle_top"),
    ("bts", "triangles_bottom"), ("tts", "triangles_top"), ("ld", "diagonal_left"),
    ("rd", "diagonal_up_right"), ("lud", "diagonal_up_left"), ("rud", "diagonal_right"),
    ("mc", "circle"), ("mr", "rhombus"), ("vh", "half_vertical"), ("hh", "half_horizontal"),
    ("vhr", "half_vertical_right"), ("hhb", "half_horizontal_bottom"), ("bo", "border"),
    ("cbo", "curly_border"), ("gra", "gradient"), ("gru", "gradient_up"), ("bri", "bricks"),
    ("glb", "globe"), ("cre", "creeper"), ("sku", "skull"), ("flo", "flower"), ("moj", "mojang"),
    ("pig", "piglin"), ("bl", "square_bottom_left"), ("br", "square_bottom_right"),
    ("tl", "square_top_left"), ("tr", "square_top_right"),
];

fn num(c: &Compound, k: &str) -> Option<i64> {
    c.get(k).and_then(Tag::as_i64)
}

fn list_of(tags: Vec<Tag>, ty: TagType) -> Tag {
    Tag::List(List::from_tags(ty, tags).expect("uniform"))
}

fn is_air(stack: &Compound) -> bool {
    stack.get("id").and_then(Tag::as_str).is_none_or(|i| i == "minecraft:air")
        || num(stack, "Count").or(num(stack, "count")) == Some(0)
}

fn convert_stack(t: &Tag, to: Form, notes: &mut Notes) -> Tag {
    match t.as_compound() {
        Some(c) if to == Form::Modern => Tag::Compound(item_up(c, notes)),
        Some(c) => Tag::Compound(item_down(c, notes)),
        None => t.clone(),
    }
}

/// Converts every stack of a list (keeping each one's `Slot`).
pub(crate) fn convert_stack_list(t: &Tag, to: Form, notes: &mut Notes) -> Tag {
    match t.as_list() {
        Some(l) => list_of(l.iter().map(|x| convert_stack(x, to, notes)).collect(), TagType::Compound),
        None => t.clone(),
    }
}

// ---- text ------------------------------------------------------------------------------

fn text_to(t: &Tag, to: Form) -> Tag {
    match (to, t) {
        (Form::Modern, Tag::String(j)) => json_to_nbt(j),
        (Form::Classic, Tag::String(_)) => Tag::String(nbt_to_json(t)),
        (Form::Classic, other) => Tag::String(nbt_to_json(other)),
        (Form::Modern, other) => other.clone(),
    }
}

fn convert_text_key(c: &mut Compound, key: &str, to: Form) {
    if let Some(t) = c.get(key) {
        let new = text_to(t, to);
        c.insert(key.to_string(), new);
    }
}

fn convert_sign_side(side: &mut Compound, to: Form) {
    for key in ["messages", "filtered_messages"] {
        let Some(Tag::List(l)) = side.get(key) else { continue };
        let lines: Vec<Tag> = l
            .iter()
            .map(|m| match to {
                Form::Modern => text_to(m, to),
                Form::Classic => Tag::String(nbt_to_json(m)),
            })
            .collect();
        let new = match to {
            Form::Modern => mixed_list(lines),
            Form::Classic => list_of(lines, TagType::String),
        };
        side.insert(key.to_string(), new);
    }
}

// ---- 1.20.5 - 1.21.4 -----------------------------------------------------------------------

fn era_up_items(c: &mut Compound, key: &str) {
    match c.get_mut(key) {
        Some(Tag::List(l)) => l.iter_mut().filter_map(Tag::as_compound_mut).for_each(era_up_item),
        Some(Tag::Compound(i)) => era_up_item(i),
        _ => {}
    }
}

/// A block entity of 1.20.5 - 1.21.4 brought to the modern form (text as NBT).
pub fn era_up_block_entity(be: &mut Compound) {
    for key in ["CustomName", "LastOutput"] {
        convert_text_key(be, key, Form::Modern);
    }
    for key in ["Items", "RecordItem", "Book", "item", "ExtraItem"] {
        era_up_items(be, key);
    }
    for side in ["front_text", "back_text"] {
        if let Some(Tag::Compound(c)) = be.get_mut(side) {
            convert_sign_side(c, Form::Modern);
        }
    }
}

/// An entity of 1.20.5 - 1.21.4 brought to the modern form: text as NBT and `HandItems` and
/// `ArmorItems` as `equipment` (its items are components already).
pub fn era_up_entity(e: &mut Compound, notes: &mut Notes) {
    let id = e.get("id").and_then(Tag::as_str).unwrap_or("").to_string();
    convert_text_key(e, "CustomName", Form::Modern);
    for key in ["Item", "Trident", "Items", "Inventory", "EnderItems", "HandItems", "ArmorItems", "SaddleItem", "ArmorItem", "DecorItem"] {
        era_up_items(e, key);
    }
    for key in ["ShoulderEntityLeft", "ShoulderEntityRight"] {
        if let Some(Tag::Compound(parrot)) = e.get_mut(key) {
            era_up_entity(parrot, notes);
        }
    }
    if let Some(Tag::Compound(offers)) = e.get_mut("Offers") {
        if let Some(Tag::List(recipes)) = offers.get_mut("Recipes") {
            for r in recipes.iter_mut().filter_map(Tag::as_compound_mut) {
                for k in ["buy", "buyB", "sell"] {
                    era_up_items(r, k);
                }
            }
        }
    }
    if let Some(Tag::List(passengers)) = e.get_mut("Passengers") {
        for p in passengers.iter_mut().filter_map(Tag::as_compound_mut) {
            era_up_entity(p, notes);
        }
    }
    entity_up(e, &id, false, notes);
}

// ---- block entities ----------------------------------------------------------------------

/// Converts a block entity in place. Returns `false` when the target version has no such
/// block entity.
pub fn block_entity_convert(be: &mut Compound, to: Form, target: &Registry, notes: &mut Notes) -> bool {
    let id = be.get("id").and_then(Tag::as_str).unwrap_or("").to_string();
    if !target.block_entities.contains(&id) {
        notes.add(format!("block entity {id} does not exist in the target version"));
        return false;
    }
    convert_text_key(be, "CustomName", to);
    convert_text_key(be, "LastOutput", to);
    if let Some(items) = be.get("Items") {
        let new = convert_stack_list(items, to, notes);
        be.insert("Items".into(), new);
    }
    for key in ["RecordItem", "Book", "item", "ExtraItem"] {
        if let Some(t) = be.get(key).cloned() {
            if t.as_compound().is_some_and(|c| !is_air(c)) {
                be.insert(key.to_string(), convert_stack(&t, to, notes));
            }
        }
    }
    for side in ["front_text", "back_text"] {
        if let Some(Tag::Compound(c)) = be.get_mut(side) {
            convert_sign_side(c, to);
        }
    }
    for key in ["SpawnData", "SpawnPotentials"] {
        spawner_entities(be, key, to, target, notes);
    }
    if be.contains_key("Bees") || be.contains_key("bees") {
        notes.add("beehive occupants are not converted");
    }

    match to {
        Form::Modern => {
            if let Some(Tag::List(l)) = be.shift_remove("Patterns") {
                let patterns = l
                    .iter()
                    .filter_map(Tag::as_compound)
                    .filter_map(|p| {
                        let code = p.get("Pattern")?.as_str()?;
                        let name = BANNER_PATTERNS.iter().find(|(c, _)| *c == code)?.1;
                        let color = DYES.get(num(p, "Color")? as usize)?;
                        Some(Tag::Compound(Compound::from([
                            ("color".to_string(), s(color)),
                            ("pattern".to_string(), Tag::String(format!("minecraft:{name}"))),
                        ])))
                    })
                    .collect();
                be.insert("patterns".into(), list_of(patterns, TagType::Compound));
            }
            if let Some(owner) = be.shift_remove("SkullOwner") {
                if let Some(p) = crate::java_items::profile_up(&owner) {
                    be.insert("profile".into(), p);
                }
            }
            if be.contains_key("BurnTime") || be.contains_key("CookTimeTotal") {
                let burn = num(be, "BurnTime").unwrap_or(0) as i32;
                be.shift_remove("BurnTime");
                be.insert("lit_time_remaining".into(), Tag::Int(burn));
                be.insert("lit_total_time".into(), Tag::Int(burn));
                if let Some(t) = be.shift_remove("CookTime") {
                    be.insert("cooking_time_spent".into(), Tag::Int(t.as_i64().unwrap_or(0) as i32));
                }
                if let Some(t) = be.shift_remove("CookTimeTotal") {
                    be.insert("cooking_total_time".into(), Tag::Int(t.as_i64().unwrap_or(0) as i32));
                }
            }
            if id == "minecraft:jukebox" {
                let started = num(be, "RecordStartTick").unwrap_or(0);
                let now = num(be, "TickCount").unwrap_or(0);
                for k in ["RecordStartTick", "TickCount", "IsPlaying"] {
                    be.shift_remove(k);
                }
                if be.contains_key("RecordItem") {
                    be.insert("ticks_since_song_started".into(), Tag::Long((now - started).max(0)));
                }
            }
        }
        Form::Classic => {
            be.shift_remove("components");
            be.shift_remove("allow_op_features");
            if let Some(Tag::List(l)) = be.shift_remove("patterns") {
                let patterns = l
                    .iter()
                    .filter_map(Tag::as_compound)
                    .filter_map(|p| {
                        let name = p.get("pattern")?.as_str()?;
                        let bare = name.strip_prefix("minecraft:").unwrap_or(name);
                        let Some((code, _)) = BANNER_PATTERNS.iter().find(|(_, n)| *n == bare) else {
                            notes.add(format!("banner pattern {name} does not exist before 1.21"));
                            return None;
                        };
                        let color = DYES.iter().position(|d| Some(*d) == p.get("color").and_then(Tag::as_str))?;
                        Some(Tag::Compound(Compound::from([
                            ("Color".to_string(), Tag::Int(color as i32)),
                            ("Pattern".to_string(), s(code)),
                        ])))
                    })
                    .collect();
                be.insert("Patterns".into(), list_of(patterns, TagType::Compound));
            }
            if let Some(p) = be.shift_remove("profile") {
                if let Some(owner) = crate::java_items::profile_down(&p) {
                    be.insert("SkullOwner".into(), owner);
                }
            }
            if be.contains_key("lit_time_remaining") || be.contains_key("cooking_total_time") {
                for (new, old) in [
                    ("lit_time_remaining", "BurnTime"),
                    ("cooking_time_spent", "CookTime"),
                    ("cooking_total_time", "CookTimeTotal"),
                ] {
                    if let Some(t) = be.shift_remove(new) {
                        be.insert(old.into(), Tag::Short(t.as_i64().unwrap_or(0) as i16));
                    }
                }
                for k in ["lit_total_time", "speed_multiplier"] {
                    be.shift_remove(k);
                }
            }
            if id == "minecraft:jukebox" {
                if let Some(t) = be.shift_remove("ticks_since_song_started") {
                    be.insert("RecordStartTick".into(), Tag::Long(0));
                    be.insert("TickCount".into(), Tag::Long(t.as_i64().unwrap_or(0)));
                    be.insert("IsPlaying".into(), Tag::Byte(1));
                }
            }
        }
    }
    true
}

/// Spawner entity templates (`SpawnData.entity` and `SpawnPotentials[].data.entity`).
fn spawner_entities(be: &mut Compound, key: &str, to: Form, target: &Registry, notes: &mut Notes) {
    let fix = |data: &mut Compound, notes: &mut Notes| {
        if let Some(Tag::Compound(e)) = data.get_mut("entity") {
            entity_fields(e, to, target, notes);
        }
    };
    match be.get_mut(key) {
        Some(Tag::Compound(d)) => fix(d, notes),
        Some(Tag::List(l)) => {
            for item in l.iter_mut() {
                if let Some(Tag::Compound(d)) = item.as_compound_mut().and_then(|c| c.get_mut("data")) {
                    fix(d, notes);
                }
            }
        }
        _ => {}
    }
}

// ---- entities ----------------------------------------------------------------------------

/// Converts an entity in place. Returns `false` when the target version has no such entity.
pub fn entity_convert(e: &mut Compound, to: Form, target: &Registry, notes: &mut Notes) -> bool {
    let id = e.get("id").and_then(Tag::as_str).unwrap_or("").to_string();
    if !target.entities.contains(&id) {
        notes.add(format!("entity {id} does not exist in the target version"));
        return false;
    }
    entity_fields(e, to, target, notes);
    true
}

pub(crate) fn entity_fields(e: &mut Compound, to: Form, target: &Registry, notes: &mut Notes) {
    let id = e.get("id").and_then(Tag::as_str).unwrap_or("").to_string();
    convert_text_key(e, "CustomName", to);
    for key in ["Item", "Trident"] {
        if let Some(t) = e.get(key).cloned() {
            e.insert(key.into(), convert_stack(&t, to, notes));
        }
    }
    for key in ["Items", "Inventory"] {
        if let Some(t) = e.get(key).cloned() {
            e.insert(key.into(), convert_stack_list(&t, to, notes));
        }
    }
    if let Some(Tag::Compound(offers)) = e.get_mut("Offers") {
        if let Some(Tag::List(recipes)) = offers.get_mut("Recipes") {
            for r in recipes.iter_mut().filter_map(Tag::as_compound_mut) {
                for k in ["buy", "buyB", "sell"] {
                    if let Some(t) = r.get(k).cloned() {
                        r.insert(k.into(), convert_stack(&t, to, notes));
                    }
                }
                if r.get("buyB").and_then(Tag::as_compound).is_some_and(is_air) {
                    r.shift_remove("buyB");
                }
            }
        }
    }
    if let Some(Tag::List(passengers)) = e.get_mut("Passengers") {
        let mut kept = Vec::new();
        for p in passengers.iter() {
            let mut p = p.clone();
            if let Some(c) = p.as_compound_mut() {
                if !entity_convert(c, to, target, notes) {
                    continue;
                }
            }
            kept.push(p);
        }
        *passengers = List::from_tags(TagType::Compound, kept).expect("compounds");
    }
    match to {
        Form::Modern => entity_up(e, &id, true, notes),
        Form::Classic => entity_down(e, &id, notes),
    }
}

fn is_saddle_entity(id: &str) -> bool {
    matches!(id, "minecraft:pig" | "minecraft:strider")
}

/// `items` is false for data whose items are components already (1.20.5 to 1.21.4).
fn entity_up(e: &mut Compound, id: &str, items: bool, notes: &mut Notes) {
    let mut equipment = Compound::new();
    let put = |equipment: &mut Compound, slot: &str, t: &Tag, notes: &mut Notes| {
        if let Some(c) = t.as_compound() {
            if !is_air(c) {
                let item = if items { item_up(c, notes) } else { c.clone() };
                equipment.insert(slot.to_string(), Tag::Compound(item));
            }
        }
    };
    // Horses and llamas keep their armor in `ArmorItem`/`DecorItem` and mirror it in the
    // chest slot of `ArmorItems`; it belongs to the `body` slot only.
    let horse_body = e.contains_key("ArmorItem") || e.contains_key("DecorItem");
    if let Some(Tag::List(l)) = e.shift_remove("ArmorItems") {
        for (slot, t) in ARMOR_SLOTS.iter().zip(l.iter()) {
            if horse_body && *slot == "chest" {
                continue;
            }
            put(&mut equipment, slot, t, notes);
        }
    }
    if let Some(Tag::List(l)) = e.shift_remove("HandItems") {
        for (slot, t) in HAND_SLOTS.iter().zip(l.iter()) {
            put(&mut equipment, slot, t, notes);
        }
    }
    if num(e, "Saddle") == Some(1) && is_saddle_entity(id) {
        equipment.insert(
            "saddle".into(),
            Tag::Compound(Compound::from([("id".to_string(), s("minecraft:saddle")), ("count".to_string(), Tag::Int(1))])),
        );
    }
    e.shift_remove("Saddle");
    for (old, slot) in [("SaddleItem", "saddle"), ("ArmorItem", "body"), ("DecorItem", "body")] {
        if let Some(t) = e.shift_remove(old) {
            put(&mut equipment, slot, &t, notes);
        }
    }
    let mut drops = Compound::new();
    for (key, slots) in [("ArmorDropChances", &ARMOR_SLOTS[..]), ("HandDropChances", &HAND_SLOTS[..])] {
        if let Some(Tag::List(l)) = e.shift_remove(key) {
            for (slot, t) in slots.iter().zip(l.iter()) {
                let v = t.as_f64().unwrap_or(DEFAULT_DROP_CHANCE);
                let skip = horse_body && *slot == "chest";
                if !skip && (v - DEFAULT_DROP_CHANCE).abs() > 1e-4 {
                    drops.insert((*slot).to_string(), Tag::Float(v as f32));
                }
            }
        }
    }
    // Saddles and horse armor were always dropped; the game records that as a chance of 2.
    for slot in ["saddle", "body"] {
        if equipment.contains_key(slot) {
            drops.insert(slot.to_string(), Tag::Float(2.0));
        }
    }
    if !equipment.is_empty() {
        e.insert("equipment".into(), Tag::Compound(equipment));
    }
    if !drops.is_empty() {
        e.insert("drop_chances".into(), Tag::Compound(drops));
    }

    if let Some(Tag::List(l)) = e.shift_remove("Attributes") {
        let attrs = l
            .iter()
            .filter_map(Tag::as_compound)
            .map(|a| {
                let mut o = Compound::new();
                o.insert("id".into(), Tag::String(attribute_up(a.get("Name").and_then(Tag::as_str).unwrap_or(""))));
                o.insert("base".into(), Tag::Double(a.get("Base").and_then(Tag::as_f64).unwrap_or(0.0)));
                if let Some(Tag::List(mods)) = a.get("Modifiers") {
                    let mods = mods
                        .iter()
                        .filter_map(Tag::as_compound)
                        .map(|m| {
                            let uid = match m.get("UUID") {
                                Some(Tag::IntArray(u)) => uuid_to_id(u),
                                _ => None,
                            }
                            .unwrap_or_else(|| "minecraft:modifier".into());
                            let op = ["add_value", "add_multiplied_base", "add_multiplied_total"]
                                [num(m, "Operation").unwrap_or(0).clamp(0, 2) as usize];
                            Tag::Compound(Compound::from([
                                ("id".to_string(), Tag::String(uid)),
                                ("amount".to_string(), Tag::Double(m.get("Amount").and_then(Tag::as_f64).unwrap_or(0.0))),
                                ("operation".to_string(), s(op)),
                            ]))
                        })
                        .collect();
                    o.insert("modifiers".into(), list_of(mods, TagType::Compound));
                }
                Tag::Compound(o)
            })
            .collect();
        e.insert("attributes".into(), list_of(attrs, TagType::Compound));
    }
    if let Some(Tag::List(l)) = e.shift_remove("ActiveEffects") {
        let effs = l.iter().filter_map(Tag::as_compound).map(|x| Tag::Compound(effect_up(x))).collect();
        e.insert("active_effects".into(), list_of(effs, TagType::Compound));
    }
    if let Some(f) = e.shift_remove("FallDistance") {
        e.insert("fall_distance".into(), Tag::Double(f.as_f64().unwrap_or(0.0)));
    }
}

fn entity_down(e: &mut Compound, id: &str, notes: &mut Notes) {
    if let Some(Tag::Compound(eq)) = e.shift_remove("equipment") {
        let get = |slot: &str, notes: &mut Notes| -> Tag {
            match eq.get(slot).and_then(Tag::as_compound) {
                Some(c) => Tag::Compound(item_down(c, notes)),
                None => Tag::Compound(Compound::new()),
            }
        };
        let armor: Vec<Tag> = ARMOR_SLOTS.iter().map(|sl| get(sl, notes)).collect();
        let hands: Vec<Tag> = HAND_SLOTS.iter().map(|sl| get(sl, notes)).collect();
        e.insert("ArmorItems".into(), list_of(armor, TagType::Compound));
        e.insert("HandItems".into(), list_of(hands, TagType::Compound));
        if eq.contains_key("saddle") {
            if is_saddle_entity(id) {
                e.insert("Saddle".into(), Tag::Byte(1));
            } else {
                e.insert("SaddleItem".into(), get("saddle", notes));
            }
        }
        if eq.contains_key("body") {
            match id {
                "minecraft:horse" | "minecraft:donkey" | "minecraft:mule" => {
                    e.insert("ArmorItem".into(), get("body", notes));
                }
                "minecraft:llama" | "minecraft:trader_llama" => {
                    e.insert("DecorItem".into(), get("body", notes));
                }
                other => notes.add(format!("body equipment of {other} does not exist before 1.20.5")),
            }
        }
    }
    let drops = e.shift_remove("drop_chances");
    let chance = |slot: &str| {
        drops
            .as_ref()
            .and_then(|d| d.get(slot))
            .and_then(Tag::as_f64)
            .unwrap_or(DEFAULT_DROP_CHANCE) as f32
    };
    if e.contains_key("ArmorItems") || drops.is_some() {
        let armor = ARMOR_SLOTS.iter().map(|sl| Tag::Float(chance(sl))).collect();
        let hands = HAND_SLOTS.iter().map(|sl| Tag::Float(chance(sl))).collect();
        e.insert("ArmorDropChances".into(), list_of(armor, TagType::Float));
        e.insert("HandDropChances".into(), list_of(hands, TagType::Float));
    }

    if let Some(Tag::List(l)) = e.shift_remove("attributes") {
        let attrs = l
            .iter()
            .filter_map(Tag::as_compound)
            .filter_map(|a| {
                let aid = a.get("id")?.as_str()?;
                let Some(old) = attribute_down(aid) else {
                    notes.add(format!("attribute {aid} does not exist before 1.21"));
                    return None;
                };
                let mut o = Compound::new();
                // Entity attributes keep the namespace in the classic form (item modifiers do not).
                o.insert("Name".into(), Tag::String(format!("minecraft:{old}")));
                o.insert("Base".into(), Tag::Double(a.get("base").and_then(Tag::as_f64).unwrap_or(0.0)));
                if let Some(Tag::List(mods)) = a.get("modifiers") {
                    let mods = mods
                        .iter()
                        .filter_map(Tag::as_compound)
                        .map(|m| {
                            let mid = m.get("id").and_then(Tag::as_str).unwrap_or("minecraft:modifier");
                            let op = ["add_value", "add_multiplied_base", "add_multiplied_total"]
                                .iter()
                                .position(|x| Some(*x) == m.get("operation").and_then(Tag::as_str))
                                .unwrap_or(0);
                            Tag::Compound(Compound::from([
                                ("Name".to_string(), s(mid)),
                                ("Amount".to_string(), Tag::Double(m.get("amount").and_then(Tag::as_f64).unwrap_or(0.0))),
                                ("Operation".to_string(), Tag::Int(op as i32)),
                                ("UUID".to_string(), Tag::IntArray(id_to_uuid(mid))),
                            ]))
                        })
                        .collect();
                    o.insert("Modifiers".into(), list_of(mods, TagType::Compound));
                }
                Some(Tag::Compound(o))
            })
            .collect();
        e.insert("Attributes".into(), list_of(attrs, TagType::Compound));
    }
    if let Some(Tag::List(l)) = e.shift_remove("active_effects") {
        let effs = l
            .iter()
            .filter_map(Tag::as_compound)
            .filter_map(|x| effect_down(x, notes))
            .map(Tag::Compound)
            .collect();
        e.insert("ActiveEffects".into(), list_of(effs, TagType::Compound));
    }
    if let Some(f) = e.shift_remove("fall_distance") {
        e.insert("FallDistance".into(), Tag::Float(f.as_f64().unwrap_or(0.0) as f32));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::java_text::unwrap_str;
    use mctk_registry::Edition;

    fn registry() -> Registry {
        let mut r = Registry::new(Edition::Java, "t");
        for id in ["minecraft:zombie", "minecraft:pig", "minecraft:horse", "minecraft:villager", "minecraft:item"] {
            r.entities.push(id.into());
        }
        for id in ["minecraft:chest", "minecraft:banner", "minecraft:sign", "minecraft:furnace", "minecraft:jukebox", "minecraft:mob_spawner"] {
            r.block_entities.push(id.into());
        }
        r
    }

    fn stack(id: &str, count: i8) -> Tag {
        Tag::Compound(Compound::from([("id".to_string(), s(id)), ("Count".to_string(), Tag::Byte(count))]))
    }

    #[test]
    fn forms_follow_the_data_version() {
        assert_eq!(Form::of(3465), Some(Form::Classic));
        assert_eq!(Form::of(5023), Some(Form::Modern));
        assert_eq!(Form::of(4000), None);
    }

    #[test]
    fn zombie_equipment_attributes_effects_and_name_match_the_game() {
        let mut z = Compound::from([
            ("id".to_string(), s("minecraft:zombie")),
            ("CustomName".to_string(), s(r#"{"text":"Bob"}"#)),
            ("FallDistance".to_string(), Tag::Float(0.0)),
            (
                "ArmorItems".to_string(),
                list_of(vec![Tag::Compound(Compound::new()), Tag::Compound(Compound::new()), Tag::Compound(Compound::new()), stack("minecraft:diamond_helmet", 1)], TagType::Compound),
            ),
            ("HandItems".to_string(), list_of(vec![stack("minecraft:iron_sword", 1), Tag::Compound(Compound::new())], TagType::Compound)),
            ("ArmorDropChances".to_string(), list_of(vec![Tag::Float(0.5); 4], TagType::Float)),
            ("HandDropChances".to_string(), list_of(vec![Tag::Float(0.085); 2], TagType::Float)),
            (
                "Attributes".to_string(),
                list_of(vec![Tag::Compound(Compound::from([("Name".to_string(), s("minecraft:generic.max_health")), ("Base".to_string(), Tag::Double(30.0))]))], TagType::Compound),
            ),
            (
                "ActiveEffects".to_string(),
                list_of(vec![Tag::Compound(Compound::from([("Id".to_string(), Tag::Byte(1)), ("Amplifier".to_string(), Tag::Byte(1)), ("Duration".to_string(), Tag::Int(6000)), ("ShowParticles".to_string(), Tag::Byte(1))]))], TagType::Compound),
            ),
        ]);
        let mut notes = Notes::default();
        assert!(entity_convert(&mut z, Form::Modern, &registry(), &mut notes));
        assert_eq!(z.get("CustomName"), Some(&s("Bob")));
        assert_eq!(z.get("fall_distance"), Some(&Tag::Double(0.0)));
        let eq = z.get("equipment").and_then(Tag::as_compound).unwrap();
        assert_eq!(eq.get("head").unwrap().get("id"), Some(&s("minecraft:diamond_helmet")));
        assert_eq!(eq.get("mainhand").unwrap().get("count"), Some(&Tag::Int(1)));
        assert!(eq.get("offhand").is_none() && z.get("ArmorItems").is_none());
        let drops = z.get("drop_chances").and_then(Tag::as_compound).unwrap();
        assert_eq!(drops.len(), 4, "hand chances at the default are left out");
        let attr = z.get("attributes").unwrap().as_list().unwrap().get(0).unwrap();
        assert_eq!(attr.get("id"), Some(&s("minecraft:max_health")));
        let eff = z.get("active_effects").unwrap().as_list().unwrap().get(0).unwrap();
        assert_eq!(eff.get("id"), Some(&s("minecraft:speed")));

        // And back: the classic fields return.
        assert!(entity_convert(&mut z, Form::Classic, &registry(), &mut notes));
        assert!(z.get("equipment").is_none());
        let armor = z.get("ArmorItems").and_then(Tag::as_list).unwrap();
        assert_eq!(armor.len(), 4);
        assert_eq!(armor.get(3).unwrap().get("id"), Some(&s("minecraft:diamond_helmet")));
        assert_eq!(z.get("ActiveEffects").unwrap().as_list().unwrap().get(0).unwrap().get("Id"), Some(&Tag::Byte(1)));
        assert_eq!(z.get("Attributes").unwrap().as_list().unwrap().get(0).unwrap().get("Name"), Some(&s("minecraft:generic.max_health")));
        assert!(notes.0.is_empty(), "{notes:?}");
    }

    #[test]
    fn pig_saddle_becomes_equipment_and_returns() {
        let mut p = Compound::from([("id".to_string(), s("minecraft:pig")), ("Saddle".to_string(), Tag::Byte(1))]);
        let mut notes = Notes::default();
        assert!(entity_convert(&mut p, Form::Modern, &registry(), &mut notes));
        assert!(p.get("Saddle").is_none());
        assert!(p.get("equipment").unwrap().get("saddle").is_some());
        assert!(entity_convert(&mut p, Form::Classic, &registry(), &mut notes));
        assert_eq!(p.get("Saddle"), Some(&Tag::Byte(1)));
    }

    #[test]
    fn unknown_entities_and_block_entities_are_dropped_and_counted() {
        let mut notes = Notes::default();
        let mut e = Compound::from([("id".to_string(), s("minecraft:warden"))]);
        assert!(!entity_convert(&mut e, Form::Modern, &registry(), &mut notes));
        let mut b = Compound::from([("id".to_string(), s("minecraft:crafter"))]);
        assert!(!block_entity_convert(&mut b, Form::Modern, &registry(), &mut notes));
        assert_eq!(notes.0.len(), 2);
    }

    #[test]
    fn banner_chest_and_furnace_match_the_game() {
        let mut notes = Notes::default();
        let mut banner = Compound::from([
            ("id".to_string(), s("minecraft:banner")),
            ("CustomName".to_string(), s(r#"{"text":"Flag"}"#)),
            (
                "Patterns".to_string(),
                list_of(
                    vec![
                        Tag::Compound(Compound::from([("Pattern".to_string(), s("bs")), ("Color".to_string(), Tag::Int(1))])),
                        Tag::Compound(Compound::from([("Pattern".to_string(), s("cr")), ("Color".to_string(), Tag::Int(5))])),
                    ],
                    TagType::Compound,
                ),
            ),
        ]);
        assert!(block_entity_convert(&mut banner, Form::Modern, &registry(), &mut notes));
        assert_eq!(banner.get("CustomName"), Some(&s("Flag")));
        let p = banner.get("patterns").unwrap().as_list().unwrap();
        assert_eq!(p.get(0).unwrap().get("pattern"), Some(&s("minecraft:stripe_bottom")));
        assert_eq!(p.get(0).unwrap().get("color"), Some(&s("orange")));
        assert_eq!(p.get(1).unwrap().get("color"), Some(&s("lime")));
        assert!(block_entity_convert(&mut banner, Form::Classic, &registry(), &mut notes));
        assert_eq!(banner.get("Patterns").unwrap().as_list().unwrap().get(1).unwrap().get("Pattern"), Some(&s("cr")));

        let mut furnace = Compound::from([
            ("id".to_string(), s("minecraft:furnace")),
            ("BurnTime".to_string(), Tag::Short(100)),
            ("CookTime".to_string(), Tag::Short(50)),
            ("CookTimeTotal".to_string(), Tag::Short(200)),
            ("Items".to_string(), list_of(vec![stack("minecraft:coal", 4)], TagType::Compound)),
        ]);
        assert!(block_entity_convert(&mut furnace, Form::Modern, &registry(), &mut notes));
        assert_eq!(furnace.get("lit_time_remaining"), Some(&Tag::Int(100)));
        assert_eq!(furnace.get("cooking_total_time"), Some(&Tag::Int(200)));
        let item = furnace.get("Items").unwrap().as_list().unwrap().get(0).unwrap().clone();
        assert_eq!(item.get("count"), Some(&Tag::Int(4)));
        assert!(notes.0.is_empty(), "{notes:?}");
    }

    #[test]
    fn sign_text_changes_form() {
        let side = Compound::from([(
            "messages".to_string(),
            list_of(vec![s(r#"{"text":"Hello"}"#), s(r#"{"color":"red","text":"World"}"#), s(r#"{"text":""}"#), s(r#"{"text":""}"#)], TagType::String),
        )]);
        let mut sign = Compound::from([("id".to_string(), s("minecraft:sign")), ("front_text".to_string(), Tag::Compound(side))]);
        let mut notes = Notes::default();
        assert!(block_entity_convert(&mut sign, Form::Modern, &registry(), &mut notes));
        let msgs = sign.get("front_text").unwrap().get("messages").unwrap().as_list().unwrap();
        assert_eq!(unwrap_str(msgs.get(0).unwrap()), Some("Hello"));
        assert!(msgs.get(1).unwrap().get("color").is_some());
        assert!(block_entity_convert(&mut sign, Form::Classic, &registry(), &mut notes));
        let back = sign.get("front_text").unwrap().get("messages").unwrap().as_list().unwrap();
        assert!(back.get(0).unwrap().as_str().unwrap().contains("Hello"));
    }
}
