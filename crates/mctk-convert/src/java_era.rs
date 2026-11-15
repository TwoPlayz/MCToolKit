//! Item components of 1.20.5 to 1.21.4 brought to the form 1.21.5 and later read.
//!
//! The components of those versions changed from release to release (attribute modifiers got
//! an `id` instead of a UUID in 1.21, `food` split into `consumable` and `use_remainder` in
//! 1.21.2, `custom_model_data` became an object in 1.21.4) and 1.21.5 dropped every
//! `show_in_tooltip` for one `tooltip_display`. The rules look at the shape of each component,
//! not at the version, so a component that is already in the newer form passes unchanged.
//!
//! Every rule was checked against a real 1.21.11 server upgrading chests that real 1.20.5,
//! 1.21.1, 1.21.3 and 1.21.4 servers had filled (the `tests` hold some of those stacks).

use mctk_nbt::{Compound, List, Tag, TagType};

use crate::java_items::{attribute_up, uuid_to_id};
use crate::java_text::{json_to_nbt, mixed_list};

/// The `consume_seconds` a `consumable` has when it says nothing.
const DEFAULT_CONSUME_SECONDS: f32 = 1.6;

/// Components that `hide_additional_tooltip` hid, in the order 1.21.5 lists them. Only the ones
/// on the stack are written to `hidden_components`.
const ADDITIONAL_TOOLTIP: [&str; 19] = [
    "minecraft:banner_patterns", "minecraft:bees", "minecraft:block_entity_data",
    "minecraft:block_state", "minecraft:bundle_contents", "minecraft:charged_projectiles",
    "minecraft:container", "minecraft:container_loot", "minecraft:firework_explosion",
    "minecraft:fireworks", "minecraft:instrument", "minecraft:map_decorations",
    "minecraft:map_id", "minecraft:note_block_sound", "minecraft:pot_decorations",
    "minecraft:potion_contents", "minecraft:suspicious_stew_effects",
    "minecraft:writable_book_content", "minecraft:written_book_content",
];

fn list_of(tags: Vec<Tag>, ty: TagType) -> Tag {
    Tag::List(List::from_tags(ty, tags).expect("uniform"))
}

fn text(t: &Tag) -> Tag {
    match t {
        Tag::String(j) => json_to_nbt(j),
        other => other.clone(),
    }
}

/// Converts the components of one stack (and the stacks inside it) in place.
pub(crate) fn era_up_item(item: &mut Compound) {
    let Some(Tag::Compound(comps)) = item.get_mut("components") else { return };
    nested_stacks(comps);
    for key in ["minecraft:custom_name", "minecraft:item_name"] {
        if let Some(t) = comps.get(key) {
            let new = text(t);
            comps.insert(key.into(), new);
        }
    }
    if let Some(Tag::List(lore)) = comps.get("minecraft:lore") {
        let lines: Vec<Tag> = lore.iter().map(text).collect();
        comps.insert("minecraft:lore".into(), mixed_list(lines));
    }
    attribute_modifiers(comps);
    food(comps);
    if comps.shift_remove("minecraft:fire_resistant").is_some() {
        comps.insert(
            "minecraft:damage_resistant".into(),
            Tag::Compound(Compound::from([("types".to_string(), Tag::String("#minecraft:is_fire".into()))])),
        );
    }
    if let Some(n) = comps.get("minecraft:custom_model_data").and_then(Tag::as_f64) {
        let floats = list_of(vec![Tag::Float(n as f32)], TagType::Float);
        comps.insert(
            "minecraft:custom_model_data".into(),
            Tag::Compound(Compound::from([("floats".to_string(), floats)])),
        );
    }
    fireworks(comps);
    book_pages(comps);
    tooltip(comps);
}

/// The stacks that sit in a component: container slots, a bundle, loaded projectiles and what
/// is left after use.
fn nested_stacks(comps: &mut Compound) {
    for key in ["minecraft:container", "minecraft:bundle_contents", "minecraft:charged_projectiles"] {
        if let Some(Tag::List(l)) = comps.get_mut(key) {
            for entry in l.iter_mut().filter_map(Tag::as_compound_mut) {
                match entry.get_mut("item") {
                    Some(Tag::Compound(i)) => era_up_item(i),
                    _ => era_up_item(entry),
                }
            }
        }
    }
    if let Some(Tag::Compound(i)) = comps.get_mut("minecraft:use_remainder") {
        era_up_item(i);
    }
}

/// `uuid` and `name` became an `id` (1.21), and `generic.armor` became `armor` (1.21.2).
fn attribute_modifiers(comps: &mut Compound) {
    let list = match comps.get_mut("minecraft:attribute_modifiers") {
        Some(Tag::Compound(c)) => c.get_mut("modifiers"),
        Some(other) => Some(other),
        None => None,
    };
    let Some(Tag::List(list)) = list else { return };
    for m in list.iter_mut().filter_map(Tag::as_compound_mut) {
        if let Some(ty) = m.get("type").and_then(Tag::as_str) {
            let ty = attribute_up(ty);
            m.insert("type".into(), Tag::String(ty));
        }
        let uuid = m.shift_remove("uuid");
        m.shift_remove("name");
        if !m.contains_key("id") {
            let id = match &uuid {
                Some(Tag::IntArray(u)) => uuid_to_id(u),
                _ => None,
            };
            m.insert("id".into(), Tag::String(id.unwrap_or_else(|| "minecraft:modifier".into())));
        }
    }
}

/// `food` kept only what feeds you; how long it takes and what it does moved to `consumable`
/// and `use_remainder` (1.21.2).
fn food(comps: &mut Compound) {
    let Some(Tag::Compound(food)) = comps.get_mut("minecraft:food") else { return };
    let seconds = food.shift_remove("eat_seconds").and_then(|t| t.as_f64()).map(|v| v as f32);
    let effects = food.shift_remove("effects");
    let remainder = food.shift_remove("using_converts_to");

    let mut consumable = Compound::new();
    if let Some(s) = seconds.filter(|s| *s != DEFAULT_CONSUME_SECONDS) {
        consumable.insert("consume_seconds".into(), Tag::Float(s));
    }
    let entries: Vec<Tag> = effects
        .as_ref()
        .and_then(Tag::as_list)
        .into_iter()
        .flat_map(List::iter)
        .filter_map(Tag::as_compound)
        .filter_map(|e| {
            let effect = e.get("effect")?.clone();
            let mut o = Compound::new();
            o.insert("effects".into(), list_of(vec![effect], TagType::Compound));
            if let Some(p) = e.get("probability").and_then(Tag::as_f64).filter(|p| *p != 1.0) {
                o.insert("probability".into(), Tag::Float(p as f32));
            }
            o.insert("type".into(), Tag::String("minecraft:apply_effects".into()));
            Some(Tag::Compound(o))
        })
        .collect();
    if !entries.is_empty() {
        consumable.insert("on_consume_effects".into(), list_of(entries, TagType::Compound));
    }
    if !consumable.is_empty() && !comps.contains_key("minecraft:consumable") {
        comps.insert("minecraft:consumable".into(), Tag::Compound(consumable));
    }
    if let Some(Tag::Compound(mut r)) = remainder {
        r.entry("count".into()).or_insert(Tag::Int(1));
        if !comps.contains_key("minecraft:use_remainder") {
            comps.insert("minecraft:use_remainder".into(), Tag::Compound(r));
        }
    }
}

/// Firework colors are int lists from 1.21.5 on, int arrays before.
fn fireworks(comps: &mut Compound) {
    fn explosion(e: &mut Compound) {
        for key in ["colors", "fade_colors"] {
            if let Some(Tag::IntArray(a)) = e.get(key) {
                let l = list_of(a.iter().map(|v| Tag::Int(*v)).collect(), TagType::Int);
                e.insert(key.into(), l);
            }
        }
    }
    if let Some(Tag::Compound(f)) = comps.get_mut("minecraft:fireworks") {
        if let Some(Tag::List(l)) = f.get_mut("explosions") {
            l.iter_mut().filter_map(Tag::as_compound_mut).for_each(explosion);
        }
    }
    if let Some(Tag::Compound(e)) = comps.get_mut("minecraft:firework_explosion") {
        explosion(e);
    }
}

/// Written book pages are text in JSON until 1.21.5 and in NBT after.
fn book_pages(comps: &mut Compound) {
    let Some(Tag::Compound(book)) = comps.get_mut("minecraft:written_book_content") else { return };
    let Some(Tag::List(pages)) = book.get_mut("pages") else { return };
    for page in pages.iter_mut().filter_map(Tag::as_compound_mut) {
        for key in ["raw", "filtered"] {
            if let Some(t) = page.get(key) {
                let new = text(t);
                page.insert(key.into(), new);
            }
        }
    }
}

/// Every `show_in_tooltip` and the two `hide_*` flags became one `tooltip_display`; some
/// components lost their wrapper with it.
fn tooltip(comps: &mut Compound) {
    let mut hidden: Vec<&str> = Vec::new();
    let shown = |c: &mut Compound, key: &'static str, hidden: &mut Vec<&str>| {
        if c.shift_remove("show_in_tooltip").and_then(|t| t.as_i64()) == Some(0) {
            hidden.push(key);
        }
    };

    for key in ["minecraft:enchantments", "minecraft:stored_enchantments"] {
        let Some(Tag::Compound(c)) = comps.get_mut(key) else { continue };
        shown(c, if key.ends_with("stored_enchantments") { "minecraft:stored_enchantments" } else { "minecraft:enchantments" }, &mut hidden);
        if let Some(levels) = c.shift_remove("levels") {
            comps.insert(key.into(), levels);
        }
    }
    if let Some(Tag::Compound(c)) = comps.get_mut("minecraft:attribute_modifiers") {
        shown(c, "minecraft:attribute_modifiers", &mut hidden);
        if let Some(m) = c.shift_remove("modifiers") {
            comps.insert("minecraft:attribute_modifiers".into(), m);
        }
    }
    if let Some(Tag::Compound(c)) = comps.get_mut("minecraft:unbreakable") {
        shown(c, "minecraft:unbreakable", &mut hidden);
    }
    for key in ["minecraft:can_break", "minecraft:can_place_on"] {
        let Some(Tag::Compound(c)) = comps.get_mut(key) else { continue };
        shown(c, if key.ends_with("break") { "minecraft:can_break" } else { "minecraft:can_place_on" }, &mut hidden);
        let Some(predicates) = c.shift_remove("predicates") else { continue };
        let flat = match predicates {
            Tag::List(l) if l.len() == 1 => l.iter().next().cloned().expect("one"),
            other => other,
        };
        comps.insert(key.into(), flat);
    }
    if let Some(Tag::Compound(c)) = comps.get_mut("minecraft:dyed_color") {
        shown(c, "minecraft:dyed_color", &mut hidden);
        if let Some(rgb) = c.shift_remove("rgb") {
            comps.insert("minecraft:dyed_color".into(), rgb);
        }
    }
    if let Some(Tag::Compound(c)) = comps.get_mut("minecraft:trim") {
        shown(c, "minecraft:trim", &mut hidden);
    }
    if let Some(Tag::Compound(c)) = comps.get_mut("minecraft:jukebox_playable") {
        shown(c, "minecraft:jukebox_playable", &mut hidden);
        if let Some(song) = c.shift_remove("song") {
            comps.insert("minecraft:jukebox_playable".into(), song);
        }
    }

    let hide_tooltip = comps.shift_remove("minecraft:hide_tooltip").is_some();
    if comps.shift_remove("minecraft:hide_additional_tooltip").is_some() {
        for key in ADDITIONAL_TOOLTIP {
            if comps.contains_key(key) {
                hidden.push(key);
            }
        }
    }
    if hide_tooltip || !hidden.is_empty() {
        let mut t = Compound::new();
        if hide_tooltip {
            t.insert("hide_tooltip".into(), Tag::Byte(1));
        }
        if !hidden.is_empty() {
            t.insert(
                "hidden_components".into(),
                list_of(hidden.into_iter().map(|k| Tag::String(k.into())).collect(), TagType::String),
            );
        }
        comps.insert("minecraft:tooltip_display".into(), Tag::Compound(t));
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Tag {
        Tag::String(v.into())
    }

    fn comp(fields: Vec<(&str, Tag)>) -> Tag {
        Tag::Compound(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }

    fn list(tags: Vec<Tag>) -> Tag {
        let ty = tags.first().map_or(TagType::Compound, Tag::tag_type);
        list_of(tags, ty)
    }

    /// The components of a stack after conversion.
    fn up(components: Vec<(&str, Tag)>) -> Compound {
        let mut stack = Compound::new();
        stack.insert("id".into(), s("minecraft:diamond_sword"));
        stack.insert("count".into(), Tag::Int(1));
        stack.insert("components".into(), comp(components));
        era_up_item(&mut stack);
        stack.get("components").and_then(Tag::as_compound).expect("components").clone()
    }

    fn hidden(c: &Compound) -> Vec<String> {
        c.get("minecraft:tooltip_display")
            .and_then(|t| t.get("hidden_components"))
            .and_then(Tag::as_list)
            .map(|l| l.iter().filter_map(Tag::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    }

    #[test]
    fn enchantments_lose_their_wrapper_and_the_hidden_flag_moves() {
        let c = up(vec![(
            "minecraft:enchantments",
            comp(vec![("show_in_tooltip", Tag::Byte(0)), ("levels", comp(vec![("minecraft:sharpness", Tag::Int(3))]))]),
        )]);
        assert_eq!(c.get("minecraft:enchantments"), Some(&comp(vec![("minecraft:sharpness", Tag::Int(3))])));
        assert_eq!(hidden(&c), ["minecraft:enchantments"]);
    }

    #[test]
    fn attribute_modifiers_from_1_20_5_get_an_id_from_the_uuid() {
        let modifier = comp(vec![
            ("name", s("x")),
            ("type", s("minecraft:generic.armor")),
            ("uuid", Tag::IntArray(vec![1, 2, 3, 4])),
            ("amount", Tag::Double(3.0)),
        ]);
        let c = up(vec![(
            "minecraft:attribute_modifiers",
            comp(vec![("show_in_tooltip", Tag::Byte(0)), ("modifiers", list(vec![modifier]))]),
        )]);
        let m = c.get("minecraft:attribute_modifiers").and_then(Tag::as_list).expect("a list").get(0).expect("one").clone();
        assert_eq!(m.get("type").and_then(Tag::as_str), Some("minecraft:armor"));
        assert_eq!(m.get("id").and_then(Tag::as_str), Some("minecraft:00000001-0000-0002-0000-000300000004"));
        assert!(m.get("uuid").is_none() && m.get("name").is_none());
        assert_eq!(hidden(&c), ["minecraft:attribute_modifiers"]);
    }

    #[test]
    fn food_splits_into_consumable_and_use_remainder() {
        let effect = |id: &str| comp(vec![("id", s(id)), ("duration", Tag::Int(100))]);
        let c = up(vec![(
            "minecraft:food",
            comp(vec![
                ("nutrition", Tag::Int(6)),
                ("eat_seconds", Tag::Float(3.0)),
                ("using_converts_to", comp(vec![("id", s("minecraft:bowl"))])),
                (
                    "effects",
                    list(vec![
                        comp(vec![("probability", Tag::Float(0.5)), ("effect", effect("minecraft:regeneration"))]),
                        comp(vec![("effect", effect("minecraft:hunger"))]),
                    ]),
                ),
            ]),
        )]);
        assert_eq!(c.get("minecraft:food"), Some(&comp(vec![("nutrition", Tag::Int(6))])));
        assert_eq!(
            c.get("minecraft:use_remainder"),
            Some(&comp(vec![("id", s("minecraft:bowl")), ("count", Tag::Int(1))]))
        );
        let consumable = c.get("minecraft:consumable").expect("consumable");
        assert_eq!(consumable.get("consume_seconds"), Some(&Tag::Float(3.0)));
        let effects = consumable.get("on_consume_effects").and_then(Tag::as_list).expect("effects");
        assert_eq!(effects.len(), 2);
        assert_eq!(effects.get(0).and_then(|e| e.get("probability")), Some(&Tag::Float(0.5)));
        assert!(effects.get(1).and_then(|e| e.get("probability")).is_none());
        assert_eq!(effects.get(1).and_then(|e| e.get("type")).and_then(Tag::as_str), Some("minecraft:apply_effects"));
    }

    #[test]
    fn plain_food_gets_no_consumable() {
        let c = up(vec![("minecraft:food", comp(vec![("nutrition", Tag::Int(4)), ("eat_seconds", Tag::Float(1.6))]))]);
        assert!(!c.contains_key("minecraft:consumable"));
    }

    #[test]
    fn custom_model_data_becomes_an_object_and_fire_resistance_a_damage_type() {
        let c = up(vec![
            ("minecraft:custom_model_data", Tag::Int(7)),
            ("minecraft:fire_resistant", comp(vec![])),
        ]);
        assert_eq!(
            c.get("minecraft:custom_model_data"),
            Some(&comp(vec![("floats", list(vec![Tag::Float(7.0)]))]))
        );
        assert_eq!(c.get("minecraft:damage_resistant"), Some(&comp(vec![("types", s("#minecraft:is_fire"))])));
        assert!(!c.contains_key("minecraft:fire_resistant"));
    }

    #[test]
    fn can_break_keeps_one_predicate_as_an_object_and_two_as_a_list() {
        let stone = || comp(vec![("blocks", s("minecraft:stone"))]);
        let dirt = || comp(vec![("blocks", s("minecraft:dirt"))]);
        let one = up(vec![("minecraft:can_break", comp(vec![("predicates", list(vec![stone()]))]))]);
        assert_eq!(one.get("minecraft:can_break"), Some(&stone()));
        let two = up(vec![("minecraft:can_break", comp(vec![("predicates", list(vec![stone(), dirt()]))]))]);
        assert_eq!(two.get("minecraft:can_break"), Some(&list(vec![stone(), dirt()])));
    }

    #[test]
    fn hide_additional_tooltip_hides_the_components_the_stack_has() {
        let c = up(vec![
            ("minecraft:hide_additional_tooltip", comp(vec![])),
            ("minecraft:hide_tooltip", comp(vec![])),
            ("minecraft:potion_contents", comp(vec![("potion", s("minecraft:water"))])),
        ]);
        assert_eq!(hidden(&c), ["minecraft:potion_contents"]);
        assert_eq!(
            c.get("minecraft:tooltip_display").and_then(|t| t.get("hide_tooltip")),
            Some(&Tag::Byte(1))
        );
        assert!(!c.contains_key("minecraft:hide_tooltip") && !c.contains_key("minecraft:hide_additional_tooltip"));
    }

    #[test]
    fn dyed_color_jukebox_book_pages_and_firework_colors_follow_1_21_5() {
        let c = up(vec![
            ("minecraft:dyed_color", comp(vec![("rgb", Tag::Int(255)), ("show_in_tooltip", Tag::Byte(0))])),
            ("minecraft:jukebox_playable", comp(vec![("song", s("minecraft:cat"))])),
            (
                "minecraft:written_book_content",
                comp(vec![("pages", list(vec![comp(vec![("raw", s("{\"text\":\"p1\",\"bold\":true}"))])]))]),
            ),
            (
                "minecraft:fireworks",
                comp(vec![("explosions", list(vec![comp(vec![("colors", Tag::IntArray(vec![255]))])]))]),
            ),
        ]);
        assert_eq!(c.get("minecraft:dyed_color"), Some(&Tag::Int(255)));
        assert_eq!(hidden(&c), ["minecraft:dyed_color"]);
        assert_eq!(c.get("minecraft:jukebox_playable"), Some(&s("minecraft:cat")));
        let page = c
            .get("minecraft:written_book_content")
            .and_then(|b| b.get("pages"))
            .and_then(Tag::as_list)
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("raw"))
            .expect("raw");
        assert_eq!(page, &comp(vec![("text", s("p1")), ("bold", Tag::Byte(1))]));
        let colors = c
            .get("minecraft:fireworks")
            .and_then(|f| f.get("explosions"))
            .and_then(Tag::as_list)
            .and_then(|e| e.get(0))
            .and_then(|e| e.get("colors"))
            .expect("colors");
        assert_eq!(colors, &list(vec![Tag::Int(255)]));
    }

    #[test]
    fn stacks_inside_a_container_are_converted_too() {
        let inner = comp(vec![
            ("id", s("minecraft:stone")),
            ("count", Tag::Int(2)),
            (
                "components",
                comp(vec![("minecraft:enchantments", comp(vec![("levels", comp(vec![("minecraft:mending", Tag::Int(1))]))]))]),
            ),
        ]);
        let c = up(vec![(
            "minecraft:container",
            list(vec![comp(vec![("slot", Tag::Int(0)), ("item", inner)])]),
        )]);
        let slot = c.get("minecraft:container").and_then(Tag::as_list).and_then(|l| l.get(0)).expect("slot");
        let enchants = slot
            .get("item")
            .and_then(|i| i.get("components"))
            .and_then(|c| c.get("minecraft:enchantments"))
            .expect("enchantments");
        assert_eq!(enchants, &comp(vec![("minecraft:mending", Tag::Int(1))]));
    }

    #[test]
    fn a_stack_already_in_the_new_form_is_unchanged() {
        let components = vec![
            ("minecraft:enchantments", comp(vec![("minecraft:sharpness", Tag::Int(3))])),
            ("minecraft:custom_model_data", comp(vec![("floats", list(vec![Tag::Float(7.0)]))])),
            ("minecraft:attribute_modifiers", list(vec![comp(vec![("id", s("a:b")), ("type", s("minecraft:armor"))])])),
        ];
        let expected: Compound = components.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        assert_eq!(up(components), expected);
    }
}
