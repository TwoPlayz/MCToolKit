//! What the tree's menu changes on item stacks beyond names and enchantments: attribute
//! modifiers, and the adventure-mode lists of blocks an item can break or be placed on, each in
//! the layout and spelling of the item's edition and version.
//!
//! | | Java 1.21.5 on | 1.20.5 to 1.21.4 | 1.9 to 1.20.4 | console, older | Bedrock |
//! |---|---|---|---|---|---|
//! | attributes | `minecraft:attribute_modifiers` [..] | `{modifiers: [..]}` | `tag.AttributeModifiers` | as 1.9, camel-case names | (none) |
//! | can break | `minecraft:can_break` [{blocks}] | `{predicates: [..]}` | `tag.CanDestroy` | `tag.CanDestroy` | `CanDestroy` |
//! | can place on | `minecraft:can_place_on` | `{predicates: [..]}` | `tag.CanPlaceOn` | `tag.CanPlaceOn` | `CanPlaceOn` |

use eframe::egui;
use mctk_nbt::{Compound, List, Tag, TagType};

/// Java 1.21.5, where component lists lost their wrapping compounds.
const DV_BARE_LISTS: i64 = 4325;
/// Java 1.21.2, where attribute names lost their `generic.` (and `player.`) part.
const DV_SHORT_NAMES: i64 = 4080;
/// Java 1.21, where a modifier is named by an `id` rather than a UUID and a name.
const DV_MODIFIER_IDS: i64 = 3953;
/// Java 1.20.5, item components.
const DV_COMPONENTS: i64 = 3837;
/// Java 1.16, namespaced snake-case attribute names and UUIDs as four ints.
const DV_NAMESPACED: i64 = 2566;

fn is_bedrock(c: &Compound) -> bool {
    matches!(c.get("Name"), Some(Tag::String(_))) && !c.contains_key("id")
}

/// Whether a stack keeps its data in components (Java 1.20.5 on).
fn uses_components(c: &Compound, data_version: Option<i64>) -> bool {
    let numeric = matches!(c.get("id"), Some(Tag::Short(_) | Tag::Int(_)));
    c.get("components").is_some_and(|t| t.as_compound().is_some())
        || (!numeric && !is_bedrock(c) && (c.contains_key("count") || data_version.is_some_and(|v| v >= DV_COMPONENTS)))
}

fn components(c: &mut Compound) -> &mut Compound {
    let t = c.entry("components".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
    if t.as_compound().is_none() {
        *t = Tag::Compound(Compound::new());
    }
    t.as_compound_mut().expect("a compound")
}

fn tag_of(c: &mut Compound) -> &mut Compound {
    let t = c.entry("tag".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
    if t.as_compound().is_none() {
        *t = Tag::Compound(Compound::new());
    }
    t.as_compound_mut().expect("a compound")
}

/// Takes away `components` or `tag` left empty.
fn tidy(c: &mut Compound) {
    for k in ["components", "tag"] {
        if c.get(k).and_then(Tag::as_compound).is_some_and(|t| t.is_empty()) {
            c.shift_remove(k);
        }
    }
}

fn compounds(items: Vec<Compound>) -> Tag {
    if items.is_empty() {
        Tag::List(List::new(TagType::End))
    } else {
        Tag::List(List::from_tags(TagType::Compound, items.into_iter().map(Tag::Compound).collect()).expect("compounds"))
    }
}

// ---------------------------------------------------------------------------------------------
// Attributes
// ---------------------------------------------------------------------------------------------

/// Every attribute an item can change, by its current name.
pub const ATTRIBUTES: &[&str] = &[
    "armor", "armor_toughness", "attack_damage", "attack_knockback", "attack_speed", "block_break_speed",
    "block_interaction_range", "burning_time", "entity_interaction_range", "explosion_knockback_resistance",
    "fall_damage_multiplier", "flying_speed", "follow_range", "gravity", "jump_strength", "knockback_resistance",
    "luck", "max_absorption", "max_health", "mining_efficiency", "movement_efficiency", "movement_speed",
    "oxygen_bonus", "safe_fall_distance", "scale", "sneaking_speed", "spawn_reinforcements", "step_height",
    "submerged_mining_speed", "sweeping_damage_ratio", "tempt_range", "water_movement_efficiency",
];

/// The slots a modifier can work in (`None`: any).
pub const SLOTS: &[(Option<&str>, &str)] = &[
    (None, "Any Slot"),
    (Some("mainhand"), "Main Hand"),
    (Some("offhand"), "Off Hand"),
    (Some("hand"), "Either Hand"),
    (Some("head"), "Head"),
    (Some("chest"), "Chest"),
    (Some("legs"), "Legs"),
    (Some("feet"), "Feet"),
    (Some("armor"), "Any Armor"),
    (Some("body"), "Body"),
];

pub const OPERATIONS: [&str; 3] = ["Add", "Multiply Base", "Multiply Total"];

/// One attribute modifier as the menu shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Modifier {
    /// The attribute's current name (`attack_damage`).
    pub attribute: String,
    pub amount: f64,
    /// 0 add, 1 multiply the base, 2 multiply the total.
    pub operation: u8,
    /// Where it works; `None` in any slot.
    pub slot: Option<String>,
}

/// A change to an item's attribute modifiers.
#[derive(Clone, Debug, PartialEq)]
pub enum AttrEdit {
    Add(String),
    Remove(usize),
    Amount(usize, f64),
    Operation(usize, u8),
    Slot(usize, Option<String>),
}

/// An attribute name of any version as its current one: `generic.attackDamage`,
/// `minecraft:generic.attack_damage` and `minecraft:attack_damage` are all `attack_damage`.
pub fn current_name(name: &str) -> String {
    let n = name.trim_start_matches("minecraft:");
    let n = n.split_once('.').map_or(n, |(_, rest)| rest);
    let mut out = String::new();
    for (i, ch) in n.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// An attribute's name as a version spells it.
fn versioned_name(attribute: &str, data_version: Option<i64>) -> String {
    let group = match attribute {
        "block_break_speed" | "block_interaction_range" | "entity_interaction_range" | "mining_efficiency" | "sneaking_speed" | "submerged_mining_speed" | "sweeping_damage_ratio" => "player",
        "spawn_reinforcements" => "zombie",
        "jump_strength" if data_version.is_some_and(|v| v < DV_COMPONENTS) => "horse",
        _ => "generic",
    };
    match data_version {
        Some(v) if v >= DV_SHORT_NAMES => format!("minecraft:{attribute}"),
        Some(v) if v >= DV_NAMESPACED => format!("minecraft:{group}.{attribute}"),
        // Before 1.16: `generic.attackDamage`.
        _ => {
            let mut camel = String::new();
            let mut up = false;
            for ch in attribute.chars() {
                if ch == '_' {
                    up = true;
                } else if up {
                    camel.push(ch.to_ascii_uppercase());
                    up = false;
                } else {
                    camel.push(ch);
                }
            }
            format!("{group}.{camel}")
        }
    }
}

/// Where a stack keeps its modifiers: in components (and whether as a bare list), or in `tag`.
enum AttrHome {
    Component { bare: bool },
    Tag,
}

fn attr_home(c: &Compound, data_version: Option<i64>) -> AttrHome {
    if uses_components(c, data_version) {
        let existing = c.get("components").and_then(|t| t.get("minecraft:attribute_modifiers"));
        let bare = match existing {
            Some(Tag::List(_)) => true,
            Some(_) => false,
            None => data_version.is_none_or(|v| v >= DV_BARE_LISTS),
        };
        AttrHome::Component { bare }
    } else {
        AttrHome::Tag
    }
}

/// The modifiers as they are saved.
fn raw_modifiers(c: &Compound, data_version: Option<i64>) -> Vec<Compound> {
    let list = match attr_home(c, data_version) {
        AttrHome::Component { .. } => c.get("components").and_then(|t| t.get("minecraft:attribute_modifiers")).and_then(|m| match m {
            Tag::List(l) => Some(l),
            other => other.get("modifiers").and_then(Tag::as_list),
        }),
        AttrHome::Tag => c.get("tag").and_then(|t| t.get("AttributeModifiers")).and_then(Tag::as_list),
    };
    list.map(|l| l.iter().filter_map(Tag::as_compound).cloned().collect()).unwrap_or_default()
}

fn write_modifiers(c: &mut Compound, list: Vec<Compound>, data_version: Option<i64>) {
    match attr_home(c, data_version) {
        AttrHome::Component { bare } => {
            let comps = components(c);
            if list.is_empty() {
                comps.shift_remove("minecraft:attribute_modifiers");
            } else if bare {
                comps.insert("minecraft:attribute_modifiers".into(), compounds(list));
            } else {
                // The wrapping compound keeps what else it has (whether the tooltip shows them).
                let mut wrap = comps.get("minecraft:attribute_modifiers").and_then(Tag::as_compound).cloned().unwrap_or_default();
                wrap.insert("modifiers".into(), compounds(list));
                comps.insert("minecraft:attribute_modifiers".into(), Tag::Compound(wrap));
            }
        }
        AttrHome::Tag => {
            let tag = tag_of(c);
            if list.is_empty() {
                tag.shift_remove("AttributeModifiers");
            } else {
                tag.insert("AttributeModifiers".into(), compounds(list));
            }
        }
    }
    tidy(c);
}

/// An item's attribute modifiers; `None` where its edition keeps none (Bedrock).
pub fn modifiers(c: &Compound, data_version: Option<i64>) -> Option<Vec<Modifier>> {
    if is_bedrock(c) {
        return None;
    }
    let ops = ["add_value", "add_multiplied_base", "add_multiplied_total"];
    Some(
        raw_modifiers(c, data_version)
            .iter()
            .map(|m| {
                let name = m.get("type").or_else(|| m.get("AttributeName")).and_then(Tag::as_str).unwrap_or("");
                let operation = match m.get("operation").or_else(|| m.get("Operation")) {
                    Some(Tag::String(s)) => ops.iter().position(|o| o == s).unwrap_or(0) as u8,
                    Some(t) => t.as_i64().unwrap_or(0).clamp(0, 2) as u8,
                    None => 0,
                };
                let slot = m.get("slot").or_else(|| m.get("Slot")).and_then(Tag::as_str).filter(|s| *s != "any").map(str::to_string);
                Modifier {
                    attribute: current_name(name),
                    amount: m.get("amount").or_else(|| m.get("Amount")).and_then(Tag::as_f64).unwrap_or(0.0),
                    operation,
                    slot,
                }
            })
            .collect(),
    )
}

/// A new modifier for `attribute`, laid out as the version does (number `n` among the item's,
/// to give it a name or id of its own).
fn new_modifier(attribute: &str, n: usize, component: bool, data_version: Option<i64>) -> Compound {
    let mut m = Compound::new();
    let name = versioned_name(attribute, data_version);
    // A UUID of its own: made from what it is and where.
    let seed = attribute.bytes().fold(0x4D43_544Bu32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let uuid = [0x4D43_544Bu32 as i32, seed as i32, n as i32, 0x1D00_0000];
    if component {
        m.insert("type".into(), Tag::String(name));
        if data_version.is_none_or(|v| v >= DV_MODIFIER_IDS) {
            m.insert("id".into(), Tag::String(format!("mctoolkit:{attribute}_{n}")));
        } else {
            m.insert("uuid".into(), Tag::IntArray(uuid.to_vec()));
            m.insert("name".into(), Tag::String(format!("mctoolkit {attribute}")));
        }
        m.insert("amount".into(), Tag::Double(1.0));
        m.insert("operation".into(), Tag::String("add_value".into()));
        m.insert("slot".into(), Tag::String("any".into()));
    } else {
        m.insert("AttributeName".into(), Tag::String(name));
        m.insert("Name".into(), Tag::String(format!("mctoolkit {attribute}")));
        m.insert("Amount".into(), Tag::Double(1.0));
        m.insert("Operation".into(), Tag::Int(0));
        if data_version.is_some_and(|v| v >= DV_NAMESPACED) {
            m.insert("UUID".into(), Tag::IntArray(uuid.to_vec()));
        } else {
            m.insert("UUIDMost".into(), Tag::Long(((uuid[0] as i64) << 32) | (uuid[1] as u32 as i64)));
            m.insert("UUIDLeast".into(), Tag::Long(((uuid[2] as i64) << 32) | (uuid[3] as u32 as i64)));
        }
    }
    m
}

/// Applies a change to an item's attribute modifiers.
pub fn edit_attributes(c: &mut Compound, edit: AttrEdit, data_version: Option<i64>) {
    if is_bedrock(c) {
        return;
    }
    let component = matches!(attr_home(c, data_version), AttrHome::Component { .. });
    let mut list = raw_modifiers(c, data_version);
    let ops = ["add_value", "add_multiplied_base", "add_multiplied_total"];
    match edit {
        AttrEdit::Add(attribute) => list.push(new_modifier(&attribute, list.len(), component, data_version)),
        AttrEdit::Remove(i) => {
            if i < list.len() {
                list.remove(i);
            }
        }
        AttrEdit::Amount(i, v) => {
            if let Some(m) = list.get_mut(i) {
                m.insert(if component { "amount" } else { "Amount" }.into(), Tag::Double(v));
            }
        }
        AttrEdit::Operation(i, op) => {
            if let Some(m) = list.get_mut(i) {
                let op = op.min(2);
                if component {
                    m.insert("operation".into(), Tag::String(ops[op as usize].into()));
                } else {
                    m.insert("Operation".into(), Tag::Int(op as i32));
                }
            }
        }
        AttrEdit::Slot(i, slot) => {
            if let Some(m) = list.get_mut(i) {
                if component {
                    m.insert("slot".into(), Tag::String(slot.unwrap_or_else(|| "any".into())));
                } else {
                    // No `Slot`: any slot.
                    match slot {
                        Some(s) => {
                            m.insert("Slot".into(), Tag::String(s));
                        }
                        None => {
                            m.shift_remove("Slot");
                        }
                    }
                }
            }
        }
    }
    write_modifiers(c, list, data_version);
}

// ---------------------------------------------------------------------------------------------
// Adventure mode: blocks an item can break, or be placed on
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// "Can Break" (`CanDestroy`, `minecraft:can_break`).
    Break,
    /// "Can Place On" (`CanPlaceOn`, `minecraft:can_place_on`).
    Place,
}

impl Rule {
    pub fn name(self) -> &'static str {
        match self {
            Rule::Break => "Can Break",
            Rule::Place => "Can Place On",
        }
    }
    fn old_key(self) -> &'static str {
        match self {
            Rule::Break => "CanDestroy",
            Rule::Place => "CanPlaceOn",
        }
    }
    fn component(self) -> &'static str {
        match self {
            Rule::Break => "minecraft:can_break",
            Rule::Place => "minecraft:can_place_on",
        }
    }
}

/// The blocks in a block predicate's `blocks` (a name, a `#tag`, or a list of them).
fn predicate_blocks(p: &Tag, out: &mut Vec<String>) {
    match p.get("blocks") {
        Some(Tag::String(s)) => out.push(s.clone()),
        Some(Tag::List(l)) => out.extend(l.iter().filter_map(Tag::as_str).map(str::to_string)),
        _ => {}
    }
}

/// The blocks an item can break or be placed on in adventure mode, as Java names (or `#tags`).
pub fn rule_blocks(c: &Compound, rule: Rule, data_version: Option<i64>) -> Vec<String> {
    let mut out = Vec::new();
    if is_bedrock(c) {
        for b in c.get(rule.old_key()).and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()).filter_map(Tag::as_str) {
            let java = mctk_mapping::mappings().bedrock_to_java(b, &Compound::new()).map_or_else(|| b.to_string(), |s| s.name.clone());
            out.push(java);
        }
        return out;
    }
    if uses_components(c, data_version) {
        match c.get("components").and_then(|t| t.get(rule.component())) {
            Some(Tag::List(l)) => l.iter().for_each(|p| predicate_blocks(p, &mut out)),
            Some(t @ Tag::Compound(w)) => match w.get("predicates").and_then(Tag::as_list) {
                Some(l) => l.iter().for_each(|p| predicate_blocks(p, &mut out)),
                None => predicate_blocks(t, &mut out),
            },
            _ => {}
        }
        return out;
    }
    c.get("tag").and_then(|t| t.get(rule.old_key())).and_then(Tag::as_list).into_iter().flat_map(|l| l.iter()).filter_map(Tag::as_str).map(str::to_string).collect()
}

/// Sets the blocks an item can break or be placed on, in its edition's layout (Bedrock's own
/// block names there); none takes the list away.
pub fn set_rule_blocks(c: &mut Compound, rule: Rule, blocks: &[String], data_version: Option<i64>) {
    let strings = |names: Vec<String>| {
        if names.is_empty() {
            Tag::List(List::new(TagType::End))
        } else {
            Tag::List(List::from_tags(TagType::String, names.into_iter().map(Tag::String).collect()).expect("strings"))
        }
    };
    if is_bedrock(c) {
        if blocks.is_empty() {
            c.shift_remove(rule.old_key());
            return;
        }
        let names = blocks
            .iter()
            .map(|b| mctk_mapping::mappings().java_to_bedrock(&mctk_java::BlockState::new(b.clone())).map_or_else(|| b.clone(), |bb| bb.name.clone()))
            .collect();
        c.insert(rule.old_key().into(), strings(names));
        return;
    }
    if uses_components(c, data_version) {
        let bare = match c.get("components").and_then(|t| t.get(rule.component())) {
            Some(Tag::List(_)) => true,
            Some(_) => false,
            None => data_version.is_none_or(|v| v >= DV_BARE_LISTS),
        };
        let predicates: Vec<Compound> = blocks
            .iter()
            .map(|b| {
                let mut p = Compound::new();
                p.insert("blocks".into(), Tag::String(b.clone()));
                p
            })
            .collect();
        let comps = components(c);
        if blocks.is_empty() {
            comps.shift_remove(rule.component());
        } else if bare {
            comps.insert(rule.component().into(), compounds(predicates));
        } else {
            let mut wrap = comps.get(rule.component()).and_then(Tag::as_compound).cloned().unwrap_or_default();
            wrap.insert("predicates".into(), compounds(predicates));
            comps.insert(rule.component().into(), Tag::Compound(wrap));
        }
        tidy(c);
        return;
    }
    let tag = tag_of(c);
    if blocks.is_empty() {
        tag.shift_remove(rule.old_key());
    } else {
        tag.insert(rule.old_key().into(), strings(blocks.to_vec()));
    }
    tidy(c);
}

/// One change from the menu to an item stack.
#[derive(Clone, Debug, PartialEq)]
pub enum ItemEdit {
    Attribute(AttrEdit),
    /// Add a block to (true) or take one from a rule's list.
    Rule(Rule, String, bool),
    /// A potion's kind or custom effects.
    Potion(crate::effects::PotionEdit),
    /// Another item of the same family (a shulker box of another color), keeping everything
    /// the stack has (its contents, name, enchantments...).
    Recolor(String),
}

/// The colors a shulker box comes in, and the undyed one (by Java id).
pub fn shulker_colors() -> Vec<(String, String)> {
    let mut out = vec![("minecraft:shulker_box".to_string(), "Undyed".to_string())];
    for c in COLOR_NAMES {
        out.push((format!("minecraft:{c}_shulker_box"), crate::item_picker::display_name(c)));
    }
    out
}

const COLOR_NAMES: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// Whether an item (by Java id) is a shulker box of any color.
pub fn is_shulker_box(id: &str) -> bool {
    id.trim_start_matches("minecraft:").ends_with("shulker_box")
}

/// Makes a stack another item (by Java id) in its own form, keeping everything else on it.
pub fn set_item_id(c: &mut Compound, java_id: &str) {
    match c.get("id").cloned() {
        Some(Tag::String(_)) => {
            c.insert("id".into(), Tag::String(java_id.to_string()));
        }
        Some(t @ (Tag::Short(_) | Tag::Int(_))) => {
            if let Some((n, damage)) = mctk_mapping::legacy::legacy().numeric_item(java_id) {
                c.insert("id".into(), if matches!(t, Tag::Int(_)) { Tag::Int(i32::from(n)) } else { Tag::Short(n as i16) });
                c.insert("Damage".into(), Tag::Short(damage as i16));
            }
        }
        _ => {
            if c.contains_key("Name") {
                let (name, data) = mctk_mapping::item_mappings().java_to_bedrock(java_id).map_or((java_id.to_string(), 0), |(n, d)| (n.to_string(), d));
                c.insert("Name".into(), Tag::String(name));
                c.insert("Damage".into(), Tag::Short(data));
            }
        }
    }
}

impl ItemEdit {
    pub fn note(&self) -> String {
        let pretty = |s: &str| crate::item_picker::display_name(s);
        match self {
            ItemEdit::Potion(e) => e.note(),
            ItemEdit::Recolor(id) => format!("Made it a {}", pretty(id)),
            ItemEdit::Attribute(AttrEdit::Add(a)) => format!("Added {}", pretty(a)),
            ItemEdit::Attribute(AttrEdit::Remove(_)) => "Removed an attribute".into(),
            ItemEdit::Attribute(_) => "Changed an attribute".into(),
            ItemEdit::Rule(r, b, true) => format!("{}: added {}", r.name(), pretty(b)),
            ItemEdit::Rule(r, b, false) => format!("{}: removed {}", r.name(), pretty(b)),
        }
    }

    pub fn apply(&self, c: &mut Compound, data_version: Option<i64>) {
        match self {
            ItemEdit::Potion(e) => e.apply(c, data_version),
            ItemEdit::Recolor(id) => set_item_id(c, id),
            ItemEdit::Attribute(e) => edit_attributes(c, e.clone(), data_version),
            ItemEdit::Rule(rule, block, add) => {
                let mut list = rule_blocks(c, *rule, data_version);
                if *add {
                    if !list.contains(block) {
                        list.push(block.clone());
                    }
                } else {
                    list.retain(|b| b != block);
                }
                set_rule_blocks(c, *rule, &list, data_version);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Tag {
        Tag::String(v.into())
    }
    fn of(pairs: &[(&str, Tag)]) -> Compound {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    #[test]
    fn names_of_every_version_read_as_one() {
        for n in ["generic.attackDamage", "minecraft:generic.attack_damage", "minecraft:attack_damage"] {
            assert_eq!(current_name(n), "attack_damage");
        }
        assert_eq!(versioned_name("attack_damage", Some(4500)), "minecraft:attack_damage");
        assert_eq!(versioned_name("attack_damage", Some(3700)), "minecraft:generic.attack_damage");
        assert_eq!(versioned_name("attack_damage", None), "generic.attackDamage");
        assert_eq!(versioned_name("block_break_speed", Some(3953)), "minecraft:player.block_break_speed");
    }

    #[test]
    fn attributes_are_added_changed_and_removed_in_each_layout() {
        // Java 26.x: a bare list in the component, ops and slots by name.
        let mut c = of(&[("id", s("minecraft:diamond_sword")), ("count", Tag::Int(1))]);
        edit_attributes(&mut c, AttrEdit::Add("attack_damage".into()), Some(4500));
        edit_attributes(&mut c, AttrEdit::Amount(0, 7.5), Some(4500));
        edit_attributes(&mut c, AttrEdit::Operation(0, 2), Some(4500));
        edit_attributes(&mut c, AttrEdit::Slot(0, Some("mainhand".into())), Some(4500));
        let m = c.get("components").and_then(|t| t.get("minecraft:attribute_modifiers")).and_then(Tag::as_list).and_then(|l| l.get(0)).cloned().unwrap();
        assert_eq!((m.get("type"), m.get("operation"), m.get("slot")), (Some(&s("minecraft:attack_damage")), Some(&s("add_multiplied_total")), Some(&s("mainhand"))));
        assert!(m.get("id").is_some());
        assert_eq!(modifiers(&c, Some(4500)).unwrap()[0], Modifier { attribute: "attack_damage".into(), amount: 7.5, operation: 2, slot: Some("mainhand".into()) });
        edit_attributes(&mut c, AttrEdit::Remove(0), Some(4500));
        assert!(!c.contains_key("components"));
        // 1.21: wrapped in `modifiers`.
        let mut c = of(&[("id", s("minecraft:diamond_sword")), ("count", Tag::Int(1))]);
        edit_attributes(&mut c, AttrEdit::Add("max_health".into()), Some(3953));
        assert!(c.get("components").and_then(|t| t.get("minecraft:attribute_modifiers")).and_then(|w| w.get("modifiers")).is_some());
        // 1.20.4: `tag.AttributeModifiers`, any slot by having no `Slot`.
        let mut c = of(&[("id", s("minecraft:diamond_sword")), ("Count", Tag::Byte(1))]);
        edit_attributes(&mut c, AttrEdit::Add("attack_speed".into()), Some(3700));
        edit_attributes(&mut c, AttrEdit::Slot(0, Some("offhand".into())), Some(3700));
        edit_attributes(&mut c, AttrEdit::Slot(0, None), Some(3700));
        let m = c.get("tag").and_then(|t| t.get("AttributeModifiers")).and_then(Tag::as_list).and_then(|l| l.get(0)).cloned().unwrap();
        assert_eq!(m.get("AttributeName"), Some(&s("minecraft:generic.attack_speed")));
        assert!(m.get("Slot").is_none() && matches!(m.get("UUID"), Some(Tag::IntArray(_))));
        // Console: camel-case names, the UUID in two longs; Bedrock: none at all.
        let mut c = of(&[("id", Tag::Short(276)), ("Count", Tag::Byte(1)), ("Damage", Tag::Short(0))]);
        edit_attributes(&mut c, AttrEdit::Add("attack_damage".into()), None);
        let m = c.get("tag").and_then(|t| t.get("AttributeModifiers")).and_then(Tag::as_list).and_then(|l| l.get(0)).cloned().unwrap();
        assert_eq!(m.get("AttributeName"), Some(&s("generic.attackDamage")));
        assert!(m.get("UUIDMost").is_some());
        assert!(modifiers(&of(&[("Name", s("minecraft:diamond_sword")), ("Count", Tag::Byte(1))]), None).is_none());
    }

    #[test]
    fn adventure_lists_go_where_each_edition_keeps_them() {
        let add = |c: &mut Compound, r: Rule, b: &str, dv: Option<i64>| ItemEdit::Rule(r, b.into(), true).apply(c, dv);
        // Java 26.x: a bare list of predicates.
        let mut c = of(&[("id", s("minecraft:diamond_pickaxe")), ("count", Tag::Int(1))]);
        add(&mut c, Rule::Break, "minecraft:stone", Some(4500));
        add(&mut c, Rule::Break, "minecraft:dirt", Some(4500));
        let l = c.get("components").and_then(|t| t.get("minecraft:can_break")).and_then(Tag::as_list).unwrap();
        assert_eq!(l.get(0).and_then(|p| p.get("blocks")), Some(&s("minecraft:stone")));
        assert_eq!(rule_blocks(&c, Rule::Break, Some(4500)), vec!["minecraft:stone".to_string(), "minecraft:dirt".to_string()]);
        ItemEdit::Rule(Rule::Break, "minecraft:stone".into(), false).apply(&mut c, Some(4500));
        ItemEdit::Rule(Rule::Break, "minecraft:dirt".into(), false).apply(&mut c, Some(4500));
        assert!(!c.contains_key("components"));
        // 1.21: wrapped in `predicates`.
        let mut c = of(&[("id", s("minecraft:stone")), ("count", Tag::Int(1))]);
        add(&mut c, Rule::Place, "minecraft:grass_block", Some(3953));
        assert!(c.get("components").and_then(|t| t.get("minecraft:can_place_on")).and_then(|w| w.get("predicates")).is_some());
        // 1.20.4 and console: `tag.CanDestroy` names.
        let mut c = of(&[("id", s("minecraft:diamond_pickaxe")), ("Count", Tag::Byte(1))]);
        add(&mut c, Rule::Break, "minecraft:stone", Some(3700));
        assert_eq!(c.get("tag").and_then(|t| t.get("CanDestroy")).and_then(Tag::as_list).and_then(|l| l.get(0)), Some(&s("minecraft:stone")));
        // Bedrock: on the stack itself, by Bedrock's names.
        let mut b = of(&[("Name", s("minecraft:diamond_pickaxe")), ("Count", Tag::Byte(1)), ("Damage", Tag::Short(0))]);
        add(&mut b, Rule::Place, "minecraft:short_grass", None);
        assert!(b.get("CanPlaceOn").and_then(Tag::as_list).is_some_and(|l| l.len() == 1));
        assert_eq!(rule_blocks(&b, Rule::Place, None), vec!["minecraft:short_grass".to_string()]);
    }
}

// ---------------------------------------------------------------------------------------------
// The menu rows (shared by the tree, the inventory screen and the trades screen)
// ---------------------------------------------------------------------------------------------

/// Anything a menu changes on an item stack.
#[derive(Clone, Debug, PartialEq)]
pub enum StackEdit {
    Enchant(crate::enchant::EnchantEdit),
    Name(String),
    Unbreakable(bool),
    Item(ItemEdit),
}

impl StackEdit {
    pub fn note(&self) -> String {
        match self {
            StackEdit::Enchant(e) => e.note(&[]),
            StackEdit::Name(n) if n.trim().is_empty() => "Took its name away".into(),
            StackEdit::Name(n) => format!("Named it \u{201c}{}\u{201d}", n.trim()),
            StackEdit::Unbreakable(on) => if *on { "Made it unbreakable" } else { "Made it breakable" }.into(),
            StackEdit::Item(e) => e.note(),
        }
    }

    pub fn apply(&self, c: &mut Compound, data_version: Option<i64>) {
        match self {
            StackEdit::Enchant(e) => crate::enchant::apply(c, e, data_version),
            StackEdit::Name(n) => crate::naming::set_item_name(c, Some(n.as_str()), data_version),
            StackEdit::Unbreakable(on) => crate::naming::set_unbreakable(c, *on, data_version),
            StackEdit::Item(e) => e.apply(c, data_version),
        }
    }
}

/// The menu rows for a stack (`id` its Java id): its enchantments, whether it is unbreakable
/// (if it wears out), its attributes, the blocks it can break and be placed on, and its name.
/// `key` keeps what is typed apart from other stacks' menus.
pub fn stack_menu(ui: &mut egui::Ui, cx: &mut crate::ctx_menu::Cx, key: &str, id: &str, stack: &Compound, data_version: Option<i64>) -> Option<StackEdit> {
    use crate::ctx_menu;
    let mut out = None;
    if let Some(e) = special_rows(ui, cx, key, id, stack, data_version) {
        out = Some(StackEdit::Item(e));
    }
    if let Some(e) = crate::enchant::menu(ui, cx, &crate::enchant::read(stack)) {
        out = Some(StackEdit::Enchant(e));
    }
    if crate::inventory::durability(id).is_some() {
        let on = crate::naming::is_unbreakable(stack);
        if ctx_menu::toggle(ui, "Unbreakable", on) {
            out = Some(StackEdit::Unbreakable(!on));
        }
    }
    let mods = modifiers(stack, data_version);
    let breaks = rule_blocks(stack, Rule::Break, data_version);
    let places = rule_blocks(stack, Rule::Place, data_version);
    if let Some(e) = menu_rows(ui, cx, key, mods.as_deref(), &breaks, &places) {
        out = Some(StackEdit::Item(e));
    }
    let current = crate::naming::item_name(stack).unwrap_or_default();
    if let Some(n) = name_menu(ui, cx, &current) {
        out = Some(StackEdit::Name(n));
    }
    out
}

/// The rows only some items have: a potion's kind and effects, a shulker box's color.
pub fn special_rows(ui: &mut egui::Ui, cx: &mut crate::ctx_menu::Cx, key: &str, id: &str, stack: &Compound, data_version: Option<i64>) -> Option<ItemEdit> {
    if let Some(e) = crate::effects::potion_menu(ui, cx, key, id, stack, data_version) {
        return Some(ItemEdit::Potion(e));
    }
    if is_shulker_box(id) {
        return color_menu(ui, cx, id);
    }
    None
}

/// `Color ▸` for a shulker box: each color (and undyed), pictured in its color.
pub fn color_menu(ui: &mut egui::Ui, cx: &mut crate::ctx_menu::Cx, id: &str) -> Option<ItemEdit> {
    use crate::ctx_menu;
    let colors = shulker_colors();
    let now = if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") };
    let shown = now.clone();
    ctx_menu::submenu_painted(ui, cx, move |p, r, ink| {
        if !crate::icons::item_icon_gray(p, r, &shown) {
            crate::icons::paint(p, r, crate::icons::Icon::File, ink);
        }
    }, "Color", 220.0, 20.0 + colors.len() as f32 * 26.0, |ui| {
        let old = ctx_menu::set_row_width(ui.ctx(), 220.0);
        let mut out = None;
        for (cid, name) in &colors {
            let current = *cid == now;
            let icon = cid.clone();
            if ctx_menu::painted(ui, name, if current { "\u{2713}" } else { "" }, true, move |p, r, ink| {
                if !crate::icons::item_icon(p, r, &icon, egui::Color32::WHITE) {
                    crate::icons::paint(p, r, crate::icons::Icon::File, ink);
                }
            }) && !current
            {
                out = Some(ItemEdit::Recolor(cid.clone()));
            }
        }
        ctx_menu::set_row_width(ui.ctx(), old);
        out
    })
}

/// `Display Name ▸`: a box to type the name in, and taking it away when there is one. Gives the
/// new name (empty: none) once it is entered or taken away.
pub fn name_menu(ui: &mut egui::Ui, cx: &mut crate::ctx_menu::Cx, current: &str) -> Option<String> {
    use crate::ctx_menu;
    
    ctx_menu::submenu_painted(ui, cx, ctx_menu::item_icon("name_tag"), "Display Name", 280.0, 80.0, |ui| {
        let old = ctx_menu::set_row_width(ui.ctx(), 280.0);
        let mut out = None;
        if let Some(n) = ctx_menu::text_row(ui, crate::icons::Icon::Rename, "Name", current, "Type a name") {
            if n.trim() != current.trim() {
                out = Some(n);
            }
        }
        if !current.trim().is_empty() && ctx_menu::item(ui, crate::icons::Icon::Trash, "Remove Name", "", true) {
            out = Some(String::new());
        }
        ctx_menu::set_row_width(ui.ctx(), old);
        out
    })
}

/// A stack's attribute and adventure-mode rows: its attribute modifiers (each its own sub-menu
/// of amount, operation, slot and removing it, and adding one), and the blocks it can break and
/// be placed on (removing one, adding one from the blocks).
pub fn menu_rows(ui: &mut egui::Ui, cx: &mut crate::ctx_menu::Cx, key: &str, mods: Option<&[Modifier]>, breaks: &[String], places: &[String]) -> Option<ItemEdit> {
    use crate::ctx_menu;
    use crate::icons::{self, Icon};
    use crate::item_picker::display_name;
    use crate::theme;
    let mut result = None;
    if let Some(mods) = mods {
        let height = 60.0 + (mods.len() as f32 + 1.0) * 32.0;
        let picked = ctx_menu::submenu_painted(ui, cx, ctx_menu::item_icon("golden_apple"), "Attributes", 300.0, height, |ui| {
            let old = ctx_menu::set_row_width(ui.ctx(), 300.0);
            let mut inner = ctx_menu::Cx::default();
            let mut out = None;
            if mods.is_empty() {
                ui.label(egui::RichText::new("No attributes").color(theme::text_dim()));
            }
            for (i, m) in mods.iter().enumerate() {
                let amount = match m.operation {
                    0 => format!("{:+}", m.amount),
                    _ => format!("{:+}%", m.amount * 100.0),
                };
                let slot = SLOTS.iter().find(|(s, _)| *s == m.slot.as_deref()).map_or(m.slot.as_deref().unwrap_or(""), |(_, n)| n);
                let label = format!("{} {amount} ({slot})", display_name(&m.attribute));
                let chosen = ctx_menu::submenu_wide(ui, &mut inner, Icon::Rename, &label, 240.0, 560.0, |ui| {
                    let old = ctx_menu::set_row_width(ui.ctx(), 240.0);
                    let mut out = None;
                    let id = egui::Id::new(("attr-amount", key, i));
                    let mut v: f64 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(m.amount);
                    let (changed, done) = ctx_menu::decimal_row(ui, Icon::Rename, "Amount", &mut v, -1_000_000.0..=1_000_000.0);
                    if changed {
                        ui.ctx().data_mut(|d| d.insert_temp(id, v));
                    }
                    if done {
                        ui.ctx().data_mut(|d| d.remove_temp::<f64>(id));
                        if (v - m.amount).abs() > 1e-9 {
                            out = Some(AttrEdit::Amount(i, v));
                        }
                    }
                    ctx_menu::separator(ui);
                    for (k, name) in OPERATIONS.iter().enumerate() {
                        if ctx_menu::toggle(ui, name, m.operation as usize == k) {
                            out = Some(AttrEdit::Operation(i, k as u8));
                        }
                    }
                    ctx_menu::separator(ui);
                    for (s, name) in SLOTS {
                        if ctx_menu::toggle(ui, name, m.slot.as_deref() == *s) {
                            out = Some(AttrEdit::Slot(i, s.map(str::to_string)));
                        }
                    }
                    ctx_menu::separator(ui);
                    if ctx_menu::item(ui, Icon::Trash, "Remove", "", true) {
                        out = Some(AttrEdit::Remove(i));
                    }
                    ctx_menu::set_row_width(ui.ctx(), old);
                    out
                });
                if chosen.is_some() {
                    out = chosen;
                }
            }
            ctx_menu::separator(ui);
            let added = ctx_menu::submenu_wide(ui, &mut inner, Icon::Plus, "Add Attribute", 240.0, 340.0, |ui| {
                let old = ctx_menu::set_row_width(ui.ctx(), 240.0);
                let mut out = None;
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for a in ATTRIBUTES {
                        if ctx_menu::item(ui, Icon::Plus, &display_name(a), "", true) {
                            out = Some(AttrEdit::Add((*a).to_string()));
                        }
                    }
                });
                ctx_menu::set_row_width(ui.ctx(), old);
                out
            });
            ctx_menu::set_row_width(ui.ctx(), old);
            added.or(out)
        });
        if let Some(e) = picked {
            result = Some(ItemEdit::Attribute(e));
        }
    }
    for (rule, list) in [(Rule::Break, breaks), (Rule::Place, places)] {
        let height = 60.0 + (list.len() as f32 + 1.0) * 32.0;
        let icon = if rule == Rule::Break { "diamond_pickaxe" } else { "grass_block" };
        let picked = ctx_menu::submenu_painted(ui, cx, ctx_menu::item_icon(icon), rule.name(), 260.0, height, |ui| {
            let old = ctx_menu::set_row_width(ui.ctx(), 260.0);
            let mut inner = ctx_menu::Cx::default();
            let mut out = None;
            if list.is_empty() {
                ui.label(egui::RichText::new("Any block (no list)").color(theme::text_dim()));
            }
            for b in list {
                let shown = b.clone();
                let clicked = ctx_menu::painted(ui, &format!("Remove {}", display_name(b)), "", true, move |p, r, ink| {
                    if !icons::item_icon(p, r, &shown, egui::Color32::WHITE) {
                        icons::paint(p, r, Icon::Trash, ink);
                    }
                });
                if clicked {
                    out = Some((b.clone(), false));
                }
            }
            ctx_menu::separator(ui);
            let added = ctx_menu::submenu_wide(ui, &mut inner, Icon::Plus, "Add Block", crate::item_picker::WIDTH, crate::item_picker::HEIGHT, |ui| crate::item_picker::show_blocks(ui, ""));
            ctx_menu::set_row_width(ui.ctx(), old);
            added.map(|b| (b, true)).or(out)
        });
        if let Some((block, add)) = picked {
            result = Some(ItemEdit::Rule(rule, block, add));
        }
    }
    result
}
