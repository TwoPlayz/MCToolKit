//! Java 26.x item stacks: `{id, count, components}` (`JavaComponentItemStackResolver`).

use std::collections::BTreeMap;

use mctk_nbt::{Compound, List, Tag, TagType};

use super::{Explosion, Fireworks, ItemProps, ItemStack};
use crate::ids::{Effect, Enchantment, FIREWORK_SHAPES, Horn, Potion};
use crate::{Notes, child, compound, compound_list, int, list, string, text};

/// Components this module reads; anything else is reported as not converted.
const HANDLED: [&str; 24] = [
    "minecraft:damage",
    "minecraft:custom_name",
    "minecraft:lore",
    "minecraft:dyed_color",
    "minecraft:can_place_on",
    "minecraft:can_break",
    "minecraft:repair_cost",
    "minecraft:enchantments",
    "minecraft:stored_enchantments",
    "minecraft:entity_data",
    "minecraft:writable_book_content",
    "minecraft:written_book_content",
    "minecraft:instrument",
    "minecraft:potion_contents",
    "minecraft:trim",
    "minecraft:suspicious_stew_effects",
    "minecraft:fireworks",
    "minecraft:firework_explosion",
    "minecraft:ominous_bottle_amplifier",
    "minecraft:bundle_contents",
    "minecraft:container",
    // Values the game derives again by itself.
    "minecraft:max_damage",
    "minecraft:max_stack_size",
    "minecraft:rarity",
];

fn predicates(c: &Compound, key: &str) -> Option<Vec<String>> {
    let component = compound(c, key)?;
    Some(
        list(component, "predicates")
            .filter_map(|p| p.get("blocks").and_then(Tag::as_str))
            .map(str::to_string)
            .collect(),
    )
}

fn colors(tag: Option<&Tag>) -> Vec<u32> {
    match tag {
        Some(Tag::List(l)) => l.iter().filter_map(Tag::as_i64).map(|v| v as u32 & 0xFF_FFFF).collect(),
        Some(Tag::IntArray(a)) => a.iter().map(|v| *v as u32 & 0xFF_FFFF).collect(),
        _ => Vec::new(),
    }
}

fn read_explosion(c: &Compound) -> Explosion {
    Explosion {
        shape: string(c, "shape")
            .and_then(|s| FIREWORK_SHAPES.iter().position(|n| *n == s.trim_start_matches("minecraft:")))
            .unwrap_or(0),
        colors: colors(c.get("colors")),
        fade: colors(c.get("fade_colors")),
        trail: int(c, "has_trail").unwrap_or(0) != 0,
        twinkle: int(c, "has_twinkle").unwrap_or(0) != 0,
    }
}

fn write_explosion(e: &Explosion) -> Compound {
    let ints = |v: &[u32]| {
        if v.is_empty() {
            return Tag::List(List::new(TagType::End));
        }
        Tag::List(
            List::from_tags(TagType::Int, v.iter().map(|c| Tag::Int(*c as i32)).collect())
                .expect("ints"),
        )
    };
    let mut c = Compound::new();
    c.insert("shape".into(), Tag::String(FIREWORK_SHAPES[e.shape.min(4)].into()));
    c.insert("colors".into(), ints(&e.colors));
    c.insert("fade_colors".into(), ints(&e.fade));
    c.insert("has_trail".into(), Tag::Byte(e.trail as i8));
    c.insert("has_twinkle".into(), Tag::Byte(e.twinkle as i8));
    c
}

/// Reads a Java item stack (`None` for empty slots).
pub fn read(item: &Compound, notes: &mut Notes) -> Option<ItemStack> {
    let id = string(item, "id")?.to_string();
    let count = int(item, "count").or_else(|| int(item, "Count")).unwrap_or(1) as i32;
    let mut stack = ItemStack::new(id, count);
    if stack.is_air() {
        return None;
    }
    let Some(c) = compound(item, "components") else {
        return Some(stack);
    };
    let p = &mut stack.props;
    p.damage = int(c, "minecraft:damage").map(|v| v as i32);
    p.name = c.get("minecraft:custom_name").map(text::from_nbt);
    if let Some(Tag::List(l)) = c.get("minecraft:lore") {
        p.lore = Some(l.iter().map(text::from_nbt).collect());
    }
    p.color = match c.get("minecraft:dyed_color") {
        Some(Tag::Compound(d)) => int(d, "rgb").map(|v| v as u32),
        Some(t) => t.as_i64().map(|v| v as u32),
        None => None,
    };
    p.can_place_on = predicates(c, "minecraft:can_place_on");
    p.can_destroy = predicates(c, "minecraft:can_break");
    p.repair_cost = int(c, "minecraft:repair_cost").map(|v| v as i32);

    let ench = compound(c, "minecraft:enchantments").or_else(|| compound(c, "minecraft:stored_enchantments"));
    if let Some(ench) = ench {
        // Before 1.21.5 the levels sat in a `levels` child.
        let levels = compound(ench, "levels").unwrap_or(ench);
        let mut map = BTreeMap::new();
        for (name, level) in levels {
            match Enchantment::from_java(name) {
                Some(e) => {
                    map.insert(e, level.as_i64().unwrap_or(1) as i32);
                }
                None => notes.miss(format!("enchantment {name}")),
            }
        }
        p.enchantments = Some(map);
    }

    if let Some(entity) = compound(c, "minecraft:entity_data") {
        p.spawn_egg = string(entity, "id").map(str::to_string);
    }

    let book = compound(c, "minecraft:written_book_content")
        .map(|b| (b, false))
        .or_else(|| compound(c, "minecraft:writable_book_content").map(|b| (b, true)));
    if let Some((b, writable)) = book {
        p.book_title = compound(b, "title").and_then(|t| string(t, "raw")).map(str::to_string);
        p.book_author = string(b, "author").map(str::to_string);
        p.book_pages = Some(
            list(b, "pages")
                .map(|page| {
                    let raw = page.get("raw").or(Some(page));
                    match raw {
                        // Writable books keep plain text, written books keep components.
                        Some(Tag::String(s)) if writable => text::plain(s),
                        Some(t) => text::from_nbt(t),
                        None => text::plain(""),
                    }
                })
                .collect(),
        );
    }

    p.horn = string(c, "minecraft:instrument").and_then(Horn::from_java);
    p.potion = match c.get("minecraft:potion_contents") {
        Some(Tag::String(s)) => Potion::from_java(s),
        Some(Tag::Compound(pc)) => string(pc, "potion").and_then(Potion::from_java),
        _ => None,
    };
    if let Some(trim) = compound(c, "minecraft:trim") {
        if let (Some(m), Some(pat)) = (string(trim, "material"), string(trim, "pattern")) {
            p.trim = Some((
                m.trim_start_matches("minecraft:").to_string(),
                pat.trim_start_matches("minecraft:").to_string(),
            ));
        }
    }
    if c.contains_key("minecraft:suspicious_stew_effects") {
        p.stew = Some(
            list(c, "minecraft:suspicious_stew_effects")
                .filter_map(Tag::as_compound)
                .filter_map(|e| {
                    let effect = string(e, "id").and_then(Effect::from_java)?;
                    Some((effect, int(e, "duration").unwrap_or(160) as i32))
                })
                .collect(),
        );
    }
    if let Some(f) = compound(c, "minecraft:fireworks") {
        p.fireworks = Some(Fireworks {
            flight: int(f, "flight_duration").unwrap_or(1) as i8,
            explosions: list(f, "explosions")
                .filter_map(Tag::as_compound)
                .map(read_explosion)
                .collect(),
        });
    }
    p.firework_explosion = compound(c, "minecraft:firework_explosion").map(read_explosion);
    p.ominous_amplifier = int(c, "minecraft:ominous_bottle_amplifier").map(|v| v as i32);
    if c.contains_key("minecraft:bundle_contents") {
        p.bundle = Some(
            list(c, "minecraft:bundle_contents")
                .filter_map(Tag::as_compound)
                .filter_map(|i| read(i, notes))
                .collect(),
        );
    }
    if c.contains_key("minecraft:container") {
        p.container = Some(
            list(c, "minecraft:container")
                .filter_map(Tag::as_compound)
                .filter_map(|e| {
                    let slot = int(e, "slot")? as u8;
                    Some((slot, read(compound(e, "item")?, notes)?))
                })
                .collect(),
        );
    }
    for key in c.keys().filter(|k| !HANDLED.contains(&k.as_str())) {
        notes.miss(format!("item component {key}"));
    }
    Some(stack)
}

/// Writes a Java item stack. `slot` adds the `Slot` byte containers use.
pub fn write(stack: &ItemStack, slot: Option<u8>) -> Compound {
    let mut out = Compound::new();
    if let Some(s) = slot {
        out.insert("Slot".into(), Tag::Byte(s as i8));
    }
    out.insert("id".into(), Tag::String(stack.id.clone()));
    out.insert("count".into(), Tag::Int(stack.count));
    let c = write_components(stack);
    if !c.is_empty() {
        out.insert("components".into(), Tag::Compound(c));
    }
    out
}

fn write_components(stack: &ItemStack) -> Compound {
    let p: &ItemProps = &stack.props;
    let mut c = Compound::new();
    if let Some(d) = p.damage.filter(|d| *d != 0) {
        c.insert("minecraft:damage".into(), Tag::Int(d));
    }
    if let Some(n) = &p.name {
        c.insert("minecraft:custom_name".into(), text::to_nbt(n));
    }
    if let Some(lore) = &p.lore {
        let lines: Vec<Tag> = lore.iter().map(text::to_nbt).collect();
        let ty = lines.first().map_or(TagType::End, Tag::tag_type);
        if lines.iter().all(|t| t.tag_type() == ty) {
            c.insert("minecraft:lore".into(), Tag::List(List::from_tags(ty, lines).expect("same type")));
        }
    }
    if let Some(rgb) = p.color {
        c.insert("minecraft:dyed_color".into(), Tag::Int(rgb as i32));
    }
    for (key, blocks) in [
        ("minecraft:can_place_on", &p.can_place_on),
        ("minecraft:can_break", &p.can_destroy),
    ] {
        if let Some(blocks) = blocks {
            let preds = blocks
                .iter()
                .map(|b| {
                    let mut pred = Compound::new();
                    pred.insert("blocks".into(), Tag::String(b.clone()));
                    pred
                })
                .collect();
            child(&mut c, key).insert("predicates".into(), compound_list(preds));
        }
    }
    if let Some(r) = p.repair_cost {
        c.insert("minecraft:repair_cost".into(), Tag::Int(r));
    }
    if let Some(ench) = &p.enchantments {
        let key = if stack.id == "minecraft:enchanted_book" {
            "minecraft:stored_enchantments"
        } else {
            "minecraft:enchantments"
        };
        let levels: Compound = ench
            .iter()
            .filter_map(|(e, l)| Some((e.java()?.to_string(), Tag::Int(*l))))
            .collect();
        c.insert(key.into(), Tag::Compound(levels));
    }
    if let Some(e) = &p.spawn_egg {
        child(&mut c, "minecraft:entity_data").insert("id".into(), Tag::String(e.clone()));
    }
    if p.book_title.is_some() || p.book_author.is_some() || p.book_pages.is_some() {
        let writable = stack.id == "minecraft:writable_book";
        let key = if writable {
            "minecraft:writable_book_content"
        } else {
            "minecraft:written_book_content"
        };
        let b = child(&mut c, key);
        if let Some(t) = &p.book_title {
            child(b, "title").insert("raw".into(), Tag::String(t.clone()));
        }
        if let Some(a) = &p.book_author {
            b.insert("author".into(), Tag::String(a.clone()));
        }
        if let Some(pages) = &p.book_pages {
            let pages = pages
                .iter()
                .map(|t| {
                    let mut page = Compound::new();
                    let raw = if writable {
                        Tag::String(text::to_legacy(t, false))
                    } else {
                        text::to_nbt(t)
                    };
                    page.insert("raw".into(), raw);
                    page
                })
                .collect();
            b.insert("pages".into(), compound_list(pages));
        }
    }
    if let Some(name) = p.horn.and_then(Horn::java) {
        c.insert("minecraft:instrument".into(), Tag::String(name.into()));
    }
    if let Some(name) = p.potion.and_then(Potion::java) {
        child(&mut c, "minecraft:potion_contents").insert("potion".into(), Tag::String(name.into()));
    }
    if let Some((m, pat)) = &p.trim {
        let t = child(&mut c, "minecraft:trim");
        t.insert("material".into(), Tag::String(format!("minecraft:{m}")));
        t.insert("pattern".into(), Tag::String(format!("minecraft:{pat}")));
    }
    if let Some(stew) = &p.stew {
        let effects = stew
            .iter()
            .filter_map(|(e, d)| {
                let mut t = Compound::new();
                t.insert("id".into(), Tag::String(e.java()?.into()));
                t.insert("duration".into(), Tag::Int(*d));
                Some(t)
            })
            .collect();
        c.insert("minecraft:suspicious_stew_effects".into(), compound_list(effects));
    }
    if let Some(f) = &p.fireworks {
        let fw = child(&mut c, "minecraft:fireworks");
        fw.insert("flight_duration".into(), Tag::Byte(f.flight));
        fw.insert(
            "explosions".into(),
            compound_list(f.explosions.iter().map(write_explosion).collect()),
        );
    }
    if let Some(e) = &p.firework_explosion {
        c.insert("minecraft:firework_explosion".into(), Tag::Compound(write_explosion(e)));
    }
    if let Some(a) = p.ominous_amplifier {
        c.insert("minecraft:ominous_bottle_amplifier".into(), Tag::Int(a));
    }
    if let Some(items) = &p.bundle {
        c.insert(
            "minecraft:bundle_contents".into(),
            compound_list(items.iter().map(|i| write(i, None)).collect()),
        );
    }
    if let Some(items) = &p.container {
        let entries = items
            .iter()
            .map(|(slot, i)| {
                let mut e = Compound::new();
                e.insert("slot".into(), Tag::Int(*slot as i32));
                e.insert("item".into(), Tag::Compound(write(i, None)));
                e
            })
            .collect();
        c.insert("minecraft:container".into(), compound_list(entries));
    }
    c
}
