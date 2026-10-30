//! Bedrock 1.26.x item stacks: `{Name, Count, Damage, Slot, WasPickedUp, tag}`
//! (`BedrockItemStackResolver`).

use std::collections::BTreeMap;

use mctk_mapping::item_mappings;
use mctk_nbt::{Compound, Tag};

use super::{Explosion, Fireworks, ItemProps, ItemStack};
use crate::ids::{DYES, Enchantment, Horn, Potion, STEWS, closest_dye};
use crate::{Notes, child, compound, compound_list, int, list, string, string_list, text};

fn dye_to_rgb(byte: i8) -> u32 {
    // Bedrock numbers dyes in the reverse order.
    let id = 15usize.saturating_sub(byte as u8 as usize).min(15);
    DYES[id].2
}

fn rgb_to_dye(rgb: u32) -> i8 {
    (15 - closest_dye(rgb)) as i8
}

fn byte_colors(tag: Option<&Tag>) -> Vec<u32> {
    match tag {
        Some(Tag::ByteArray(a)) => a.iter().map(|b| dye_to_rgb(*b)).collect(),
        _ => Vec::new(),
    }
}

fn read_explosion(c: &Compound) -> Explosion {
    Explosion {
        shape: (int(c, "FireworkType").unwrap_or(0).max(0) as usize).min(4),
        colors: byte_colors(c.get("FireworkColor")),
        fade: byte_colors(c.get("FireworkFade")),
        trail: int(c, "FireworkTrail").unwrap_or(0) != 0,
        twinkle: int(c, "FireworkFlicker").unwrap_or(0) != 0,
    }
}

fn write_explosion(e: &Explosion) -> Compound {
    let bytes = |v: &[u32], default_black: bool| {
        // Bedrock does not like an empty colour array.
        let mut b: Vec<i8> = v.iter().map(|c| rgb_to_dye(*c)).collect();
        if b.is_empty() && default_black {
            b.push(rgb_to_dye(DYES[15].2));
        }
        Tag::ByteArray(b)
    };
    let mut c = Compound::new();
    c.insert("FireworkType".into(), Tag::Byte(e.shape as i8));
    c.insert("FireworkColor".into(), bytes(&e.colors, true));
    c.insert("FireworkFade".into(), bytes(&e.fade, false));
    c.insert("FireworkTrail".into(), Tag::Byte(e.trail as i8));
    c.insert("FireworkFlicker".into(), Tag::Byte(e.twinkle as i8));
    c
}

/// Reads a Bedrock item stack (`None` for empty slots and items without a Java equivalent).
pub fn read(item: &Compound, notes: &mut Notes) -> Option<ItemStack> {
    let name = string(item, "Name").unwrap_or_default();
    let count = int(item, "Count").unwrap_or(0) as i32;
    if name.is_empty() || count <= 0 {
        return None;
    }
    let data = int(item, "Damage").unwrap_or(0) as i16;
    let mut props = ItemProps::default();

    let java: String = match name {
        "minecraft:potion" | "minecraft:splash_potion" | "minecraft:lingering_potion" => {
            props.potion = Potion::from_bedrock(data);
            name.to_string()
        }
        "minecraft:arrow" if data >= 2 => {
            props.potion = Potion::from_bedrock(data - 1);
            "minecraft:tipped_arrow".to_string()
        }
        "minecraft:goat_horn" => {
            props.horn = Horn::from_bedrock(data);
            name.to_string()
        }
        "minecraft:suspicious_stew" => {
            props.stew = STEWS.get(data as usize).map(|(e, d)| vec![(*e, *d)]);
            name.to_string()
        }
        _ => match item_mappings().bedrock_to_java(name, data) {
            Some(j) => j.to_string(),
            None => {
                notes.miss(format!("item {name}"));
                return None;
            }
        },
    };

    if let Some(tag) = compound(item, "tag") {
        props.damage = int(tag, "Damage").map(|v| v as i32);
        if let Some(d) = compound(tag, "display") {
            props.name = string(d, "Name").map(text::plain);
            let lore: Vec<_> = list(d, "Lore").filter_map(Tag::as_str).map(text::plain).collect();
            if !lore.is_empty() {
                props.lore = Some(lore);
            }
        }
        props.color = int(tag, "customColor").map(|v| v as u32 & 0xFF_FFFF);
        let names = |key: &str| {
            tag.contains_key(key)
                .then(|| list(tag, key).filter_map(Tag::as_str).map(str::to_string).collect())
        };
        props.can_place_on = names("CanPlaceOn");
        props.can_destroy = names("CanDestroy");
        props.repair_cost = int(tag, "RepairCost").map(|v| v as i32);
        if tag.contains_key("ench") {
            let mut map = BTreeMap::new();
            for e in list(tag, "ench").filter_map(Tag::as_compound) {
                let (Some(id), lvl) = (int(e, "id"), int(e, "lvl").unwrap_or(1)) else {
                    continue;
                };
                match Enchantment::from_bedrock(id as i16) {
                    Some(en) => {
                        map.insert(en, lvl as i32);
                    }
                    None => notes.miss(format!("enchantment id {id}")),
                }
            }
            props.enchantments = Some(map);
        }
        props.book_title = string(tag, "title").map(str::to_string);
        props.book_author = string(tag, "author").map(str::to_string);
        if tag.contains_key("pages") {
            props.book_pages = Some(
                list(tag, "pages")
                    .filter_map(Tag::as_compound)
                    .filter_map(|p| string(p, "text"))
                    .map(text::plain)
                    .collect(),
            );
        }
        if let Some(t) = compound(tag, "Trim") {
            if let (Some(m), Some(p)) = (string(t, "Material"), string(t, "Pattern")) {
                props.trim = Some((m.to_string(), p.to_string()));
            }
        }
        if let Some(f) = compound(tag, "Fireworks") {
            props.fireworks = Some(Fireworks {
                flight: int(f, "Flight").unwrap_or(1) as i8,
                explosions: list(f, "Explosions")
                    .filter_map(Tag::as_compound)
                    .map(read_explosion)
                    .collect(),
            });
        }
        props.firework_explosion = compound(tag, "FireworksItem").map(read_explosion);
        if tag.contains_key("storage_item_component_content") {
            props.bundle = Some(
                list(tag, "storage_item_component_content")
                    .filter_map(Tag::as_compound)
                    .filter_map(|i| read(i, notes))
                    .collect(),
            );
        }
    }
    let mut stack = ItemStack::new(java, count);
    stack.props = props;
    Some(stack)
}

/// Bedrock name and data value of a Java stack (potions, arrows, horns and stew keep their
/// variant in the data value).
fn identify(stack: &ItemStack, notes: &mut Notes) -> Option<(String, i16)> {
    let p = &stack.props;
    Some(match stack.id.as_str() {
        id @ ("minecraft:potion" | "minecraft:splash_potion" | "minecraft:lingering_potion") => {
            (id.to_string(), p.potion.and_then(Potion::bedrock).unwrap_or(0))
        }
        "minecraft:tipped_arrow" => (
            "minecraft:arrow".to_string(),
            p.potion
                .and_then(Potion::bedrock)
                .filter(|id| *id >= 1)
                .map_or(1, |id| id + 1),
        ),
        "minecraft:goat_horn" => (
            "minecraft:goat_horn".to_string(),
            p.horn.and_then(Horn::bedrock).unwrap_or(0),
        ),
        "minecraft:suspicious_stew" => {
            let first = p.stew.as_ref().and_then(|s| s.first());
            let index = first.and_then(|(e, d)| {
                STEWS
                    .iter()
                    .position(|s| s == &(*e, *d))
                    .or_else(|| STEWS.iter().position(|(se, _)| se == e))
            });
            ("minecraft:suspicious_stew".to_string(), index.unwrap_or(0) as i16)
        }
        id => match item_mappings().java_to_bedrock(id) {
            Some((n, d)) => (n.to_string(), d),
            None => {
                notes.miss(format!("item {id}"));
                return None;
            }
        },
    })
}

/// Writes a Bedrock item stack. `slot` adds the `Slot` byte containers use.
pub fn write(stack: &ItemStack, slot: Option<u8>, notes: &mut Notes) -> Option<Compound> {
    let (name, data) = identify(stack, notes)?;
    let mut out = Compound::new();
    out.insert("Count".into(), Tag::Byte(stack.count.clamp(1, 127) as i8));
    out.insert("Damage".into(), Tag::Short(data));
    out.insert("Name".into(), Tag::String(name));
    if let Some(s) = slot {
        out.insert("Slot".into(), Tag::Byte(s as i8));
    }
    out.insert("WasPickedUp".into(), Tag::Byte(0));

    let p = &stack.props;
    let mut tag = Compound::new();
    if let Some(d) = p.damage.filter(|d| *d != 0) {
        tag.insert("Damage".into(), Tag::Int(d));
    }
    if p.name.is_some() || p.lore.is_some() {
        let d = child(&mut tag, "display");
        if let Some(n) = &p.name {
            d.insert("Name".into(), Tag::String(text::to_legacy(n, true)));
        }
        if let Some(lore) = &p.lore {
            d.insert(
                "Lore".into(),
                string_list(lore.iter().map(|t| text::to_legacy(t, true)).collect()),
            );
        }
    }
    if let Some(rgb) = p.color {
        tag.insert("customColor".into(), Tag::Int((0xFF00_0000 | rgb) as i32));
    }
    for (key, blocks) in [("CanPlaceOn", &p.can_place_on), ("CanDestroy", &p.can_destroy)] {
        if let Some(b) = blocks {
            tag.insert(key.into(), string_list(b.clone()));
        }
    }
    if let Some(r) = p.repair_cost {
        tag.insert("RepairCost".into(), Tag::Int(r));
    }
    if let Some(ench) = &p.enchantments {
        let mut entries = Vec::new();
        for (e, lvl) in ench {
            match e.bedrock() {
                Some(id) => {
                    let mut c = Compound::new();
                    c.insert("id".into(), Tag::Short(id));
                    c.insert("lvl".into(), Tag::Short(*lvl as i16));
                    entries.push(c);
                }
                None => notes.miss(format!("enchantment {e:?} (not on Bedrock)")),
            }
        }
        if !entries.is_empty() {
            tag.insert("ench".into(), compound_list(entries));
        }
    }
    if let Some(t) = &p.book_title {
        tag.insert("title".into(), Tag::String(t.clone()));
    }
    if let Some(a) = &p.book_author {
        tag.insert("author".into(), Tag::String(a.clone()));
    }
    if let Some(pages) = &p.book_pages {
        let pages = pages
            .iter()
            .map(|t| {
                let mut page = Compound::new();
                page.insert("photoname".into(), Tag::String(String::new()));
                page.insert("text".into(), Tag::String(text::to_legacy(t, true)));
                page
            })
            .collect();
        tag.insert("pages".into(), compound_list(pages));
    }
    if let Some((m, pat)) = &p.trim {
        let t = child(&mut tag, "Trim");
        t.insert("Material".into(), Tag::String(m.clone()));
        t.insert("Pattern".into(), Tag::String(pat.clone()));
    }
    if let Some(f) = &p.fireworks {
        let fw = child(&mut tag, "Fireworks");
        fw.insert("Flight".into(), Tag::Byte(f.flight));
        fw.insert(
            "Explosions".into(),
            compound_list(f.explosions.iter().map(write_explosion).collect()),
        );
    }
    if let Some(e) = &p.firework_explosion {
        tag.insert("FireworksItem".into(), Tag::Compound(write_explosion(e)));
    }
    if let Some(items) = &p.bundle {
        let mut entries = Vec::new();
        for (i, item) in items.iter().enumerate() {
            if let Some(c) = write(item, Some(i as u8), notes) {
                entries.push(c);
            }
        }
        tag.insert("storage_item_component_content".into(), compound_list(entries));
    }
    for (set, what) in [
        (p.spawn_egg.is_some(), "spawn egg entity"),
        (p.ominous_amplifier.is_some(), "ominous bottle amplifier"),
        (p.container.is_some(), "shulker box contents carried as an item"),
    ] {
        if set {
            notes.miss(format!("item data: {what}"));
        }
    }
    if !tag.is_empty() {
        out.insert("tag".into(), Tag::Compound(tag));
    }
    Some(out)
}

/// An empty inventory slot as the games write it.
pub fn empty_slot(slot: u8) -> Compound {
    let mut e = Compound::new();
    e.insert("Count".into(), Tag::Byte(0));
    e.insert("Damage".into(), Tag::Short(0));
    e.insert("Name".into(), Tag::String(String::new()));
    e.insert("Slot".into(), Tag::Byte(slot as i8));
    e.insert("WasPickedUp".into(), Tag::Byte(0));
    e
}
