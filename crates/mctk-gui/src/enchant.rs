//! Enchantments on an item stack: reading them, and writing them the way the stack already
//! keeps them. Java 1.20.5+ has them in `components`, 1.13 to 1.20.4 in `tag.Enchantments`
//! with names, and old and console worlds in `tag.ench` with numbers.

use eframe::egui;
use mctk_nbt::{Compound, List, Tag, TagType};

/// (name, the highest level the game gives it, the number old worlds use)
pub const ALL: &[(&str, i64, i16)] = &[
    ("protection", 4, 0),
    ("fire_protection", 4, 1),
    ("feather_falling", 4, 2),
    ("blast_protection", 4, 3),
    ("projectile_protection", 4, 4),
    ("respiration", 3, 5),
    ("aqua_affinity", 1, 6),
    ("thorns", 3, 7),
    ("depth_strider", 3, 8),
    ("frost_walker", 2, 9),
    ("binding_curse", 1, 10),
    ("sharpness", 5, 16),
    ("smite", 5, 17),
    ("bane_of_arthropods", 5, 18),
    ("knockback", 2, 19),
    ("fire_aspect", 2, 20),
    ("looting", 3, 21),
    ("sweeping_edge", 3, 22),
    ("efficiency", 5, 32),
    ("silk_touch", 1, 33),
    ("unbreaking", 3, 34),
    ("fortune", 3, 35),
    ("power", 5, 48),
    ("punch", 2, 49),
    ("flame", 1, 50),
    ("infinity", 1, 51),
    ("luck_of_the_sea", 3, 61),
    ("lure", 3, 62),
    ("loyalty", 3, 65),
    ("impaling", 5, 66),
    ("riptide", 3, 67),
    ("channeling", 1, 68),
    ("multishot", 1, 69),
    ("piercing", 4, 70),
    ("quick_charge", 3, 71),
    ("soul_speed", 3, -1),
    ("swift_sneak", 3, -1),
    ("density", 5, -1),
    ("breach", 4, -1),
    ("wind_burst", 3, -1),
    ("mending", 1, 72),
    ("vanishing_curse", 1, 73),
];

fn short(id: &str) -> &str {
    id.rsplit(':').next().unwrap_or(id)
}

/// `minecraft:fire_aspect` as "Fire Aspect".
pub fn name(id: &str) -> String {
    crate::item_picker::display_name(id)
}

/// A level as the game writes it: V, or the number past X.
pub fn level_text(level: i64) -> String {
    const ROMAN: [&str; 10] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    if (1..=10).contains(&level) { ROMAN[level as usize - 1].into() } else { level.to_string() }
}

/// Bedrock numbers its enchantments in its own order.
fn bedrock_number(name: &str) -> Option<i16> {
    const NAMES: [&str; 41] = [
        "protection", "fire_protection", "feather_falling", "blast_protection", "projectile_protection", "thorns",
        "respiration", "depth_strider", "aqua_affinity", "sharpness", "smite", "bane_of_arthropods", "knockback",
        "fire_aspect", "looting", "efficiency", "silk_touch", "unbreaking", "fortune", "power", "punch", "flame",
        "infinity", "luck_of_the_sea", "lure", "frost_walker", "mending", "binding_curse", "vanishing_curse",
        "impaling", "riptide", "loyalty", "channeling", "multishot", "piercing", "quick_charge", "soul_speed",
        "swift_sneak", "wind_burst", "density", "breach",
    ];
    NAMES.iter().position(|n| *n == name).map(|i| i as i16)
}

/// Worlds from before the names (a number for an item id), and Bedrock, keep numbers here too.
fn from_number(n: i64, bedrock: bool) -> Option<String> {
    if bedrock {
        return ALL.iter().find(|e| bedrock_number(e.0).is_some_and(|b| i64::from(b) == n)).map(|e| format!("minecraft:{}", e.0));
    }
    ALL.iter().find(|e| i64::from(e.2) == n && e.2 >= 0).map(|e| format!("minecraft:{}", e.0))
}

fn to_number(id: &str, bedrock: bool) -> Option<i16> {
    if bedrock {
        return bedrock_number(short(id));
    }
    ALL.iter().find(|e| e.0 == short(id) && e.2 >= 0).map(|e| e.2)
}

/// A Bedrock item names itself with `Name`, not `id`.
fn is_bedrock(c: &Compound) -> bool {
    matches!(c.get("Name"), Some(Tag::String(_))) && !c.contains_key("id")
}

fn is_book(c: &Compound) -> bool {
    matches!(c.get("id"), Some(Tag::String(s)) if s.ends_with("enchanted_book"))
}

/// The enchantments on the stack, with their levels.
pub fn read(c: &Compound) -> Vec<(String, i64)> {
    let book = is_book(c);
    let bedrock = is_bedrock(c);
    let mut out = Vec::new();
    // 1.20.5 and later.
    let key = if book { "minecraft:stored_enchantments" } else { "minecraft:enchantments" };
    if let Some(e) = c.get("components").and_then(|t| t.get(key)).and_then(Tag::as_compound) {
        let levels = e.get("levels").and_then(Tag::as_compound).unwrap_or(e);
        for (k, v) in levels {
            if k == "show_in_tooltip" {
                continue;
            }
            if let Some(l) = v.as_i64() {
                out.push((if k.contains(':') { k.clone() } else { format!("minecraft:{k}") }, l));
            }
        }
        return out;
    }
    // Before: a list of { id, lvl }.
    let tag = c.get("tag").and_then(Tag::as_compound);
    let keys: &[&str] = if book { &["StoredEnchantments"] } else { &["Enchantments", "ench"] };
    for k in keys {
        if let Some(Tag::List(l)) = tag.and_then(|t| t.get(*k)) {
            for e in l.iter().filter_map(Tag::as_compound) {
                let id = match e.get("id") {
                    Some(Tag::String(s)) => Some(if s.contains(':') { s.clone() } else { format!("minecraft:{s}") }),
                    Some(t) => t.as_i64().and_then(|n| from_number(n, bedrock)),
                    None => None,
                };
                if let (Some(id), Some(l)) = (id, e.get("lvl").and_then(Tag::as_i64)) {
                    out.push((id, l));
                }
            }
            if !out.is_empty() {
                break;
            }
        }
    }
    out
}

/// Sets the level of an enchantment (adds it), or takes it off with `None`, in the form the
/// stack already uses, or the form of the world's version when it has none.
pub fn set(c: &mut Compound, id: &str, level: Option<i64>, data_version: Option<i64>) {
    let id = if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") };
    let book = is_book(c);
    let bedrock = is_bedrock(c);
    let numeric = matches!(c.get("id"), Some(Tag::Short(_) | Tag::Int(_))) || bedrock;
    let has_components = c.get("components").is_some_and(|t| t.as_compound().is_some());
    let has_tag_list = c.get("tag").and_then(Tag::as_compound).is_some_and(|t| t.contains_key("Enchantments") || t.contains_key("ench") || t.contains_key("StoredEnchantments"));
    let components = has_components || (!numeric && !has_tag_list && (c.contains_key("count") || data_version.is_some_and(|v| v >= 3837)));
    if components {
        let key = if book { "minecraft:stored_enchantments" } else { "minecraft:enchantments" };
        let comps = c.entry("components".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
        let Some(comps) = comps.as_compound_mut() else { return };
        let existing = comps.get(key).and_then(Tag::as_compound);
        // 1.21.5 dropped the `levels` wrapper.
        let wrapped = match existing {
            Some(e) => e.contains_key("levels"),
            None => data_version.is_none_or(|v| v < 4325),
        };
        let mut levels = existing.map(|e| e.get("levels").and_then(Tag::as_compound).cloned().unwrap_or_else(|| e.clone())).unwrap_or_default();
        levels.shift_remove("show_in_tooltip");
        match level {
            Some(l) => {
                levels.insert(id, Tag::Int(l as i32));
            }
            None => {
                levels.shift_remove(&id);
            }
        }
        if levels.is_empty() {
            comps.shift_remove(key);
        } else if wrapped {
            let mut e = Compound::new();
            e.insert("levels".into(), Tag::Compound(levels));
            comps.insert(key.into(), Tag::Compound(e));
        } else {
            comps.insert(key.into(), Tag::Compound(levels));
        }
        return;
    }
    let key = if book {
        "StoredEnchantments"
    } else if numeric {
        "ench"
    } else {
        "Enchantments"
    };
    let tag = c.entry("tag".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
    let Some(tag) = tag.as_compound_mut() else { return };
    let mut entries: Vec<(String, i64)> = match tag.get(key) {
        Some(Tag::List(l)) => l
            .iter()
            .filter_map(Tag::as_compound)
            .filter_map(|e| {
                let id = match e.get("id")? {
                    Tag::String(s) => if s.contains(':') { s.clone() } else { format!("minecraft:{s}") },
                    t => from_number(t.as_i64()?, bedrock)?,
                };
                Some((id, e.get("lvl")?.as_i64()?))
            })
            .collect(),
        _ => Vec::new(),
    };
    match level {
        Some(l) => match entries.iter_mut().find(|e| e.0 == id) {
            Some(e) => e.1 = l,
            None => entries.push((id, l)),
        },
        None => entries.retain(|e| e.0 != id),
    }
    if entries.is_empty() {
        tag.shift_remove(key);
        return;
    }
    let items: Vec<Tag> = entries
        .into_iter()
        .filter_map(|(id, l)| {
            let mut e = Compound::new();
            if numeric {
                e.insert("id".into(), Tag::Short(to_number(&id, bedrock)?));
            } else {
                e.insert("id".into(), Tag::String(id));
            }
            e.insert("lvl".into(), Tag::Short(l.clamp(-32768, 32767) as i16));
            Some(Tag::Compound(e))
        })
        .collect();
    if let Ok(list) = List::from_tags(TagType::Compound, items) {
        tag.insert(key.into(), Tag::List(list));
    }
}

/// A change to a stack's enchantments, by their place in the list (as [`read`] gives it), so two
/// of the same enchantment are told apart.
#[derive(Clone, Debug, PartialEq)]
pub enum EnchantEdit {
    /// A new enchantment at level 1, after the others (even one the stack has: older Java,
    /// Bedrock and the console editions keep a list, which can hold it twice; Java 1.20.5 and
    /// later keep one level for each, so there the stack is left as it is).
    Add(String),
    Level(usize, i64),
    Remove(usize),
}

impl EnchantEdit {
    pub fn note(&self, current: &[(String, i64)]) -> String {
        match self {
            EnchantEdit::Add(id) => format!("Gave {} {}", name(id), level_text(1)),
            EnchantEdit::Level(i, l) => format!("Gave {} {}", current.get(*i).map_or_else(String::new, |e| name(&e.0)), level_text(*l)),
            EnchantEdit::Remove(i) => format!("Removed {}", current.get(*i).map_or_else(String::new, |e| name(&e.0))),
        }
    }
}

/// Whether the stack keeps its enchantments as one level for each (Java 1.20.5 on) rather than
/// a list.
fn keeps_levels(c: &Compound, data_version: Option<i64>) -> bool {
    let bedrock = is_bedrock(c);
    let numeric = matches!(c.get("id"), Some(Tag::Short(_) | Tag::Int(_))) || bedrock;
    let has_components = c.get("components").is_some_and(|t| t.as_compound().is_some());
    let has_tag_list = c.get("tag").and_then(Tag::as_compound).is_some_and(|t| t.contains_key("Enchantments") || t.contains_key("ench") || t.contains_key("StoredEnchantments"));
    has_components || (!numeric && !has_tag_list && (c.contains_key("count") || data_version.is_some_and(|v| v >= 3837)))
}

/// Applies a change to a stack's enchantments, in the form it (or its world's version) keeps them.
pub fn apply(c: &mut Compound, edit: &EnchantEdit, data_version: Option<i64>) {
    let mut list = read(c);
    let levels = keeps_levels(c, data_version);
    match edit {
        EnchantEdit::Add(id) => {
            let id = if id.contains(':') { id.clone() } else { format!("minecraft:{id}") };
            if levels && list.iter().any(|e| e.0 == id) {
                return;
            }
            list.push((id, 1));
        }
        EnchantEdit::Level(i, l) => match list.get_mut(*i) {
            Some(e) => e.1 = *l,
            None => return,
        },
        EnchantEdit::Remove(i) => {
            if *i >= list.len() {
                return;
            }
            list.remove(*i);
        }
    }
    if levels {
        // One level for each: written one by one.
        for (id, _) in read(c) {
            set(c, &id, None, data_version);
        }
        for (id, l) in list {
            set(c, &id, Some(l), data_version);
        }
        return;
    }
    // A list, written as it is (two of the same kept).
    let book = is_book(c);
    let bedrock = is_bedrock(c);
    let numeric = matches!(c.get("id"), Some(Tag::Short(_) | Tag::Int(_))) || bedrock;
    let key = if book {
        "StoredEnchantments"
    } else if numeric {
        "ench"
    } else {
        "Enchantments"
    };
    let tag = c.entry("tag".to_string()).or_insert_with(|| Tag::Compound(Compound::new()));
    let Some(tag) = tag.as_compound_mut() else { return };
    if list.is_empty() {
        tag.shift_remove(key);
        if tag.is_empty() {
            c.shift_remove("tag");
        }
        return;
    }
    let items: Vec<Tag> = list
        .into_iter()
        .filter_map(|(id, l)| {
            let mut e = Compound::new();
            if numeric {
                e.insert("id".into(), Tag::Short(to_number(&id, bedrock)?));
            } else {
                e.insert("id".into(), Tag::String(id));
            }
            e.insert("lvl".into(), Tag::Short(l.clamp(-32768, 32767) as i16));
            Some(Tag::Compound(e))
        })
        .collect();
    if let Ok(l) = List::from_tags(TagType::Compound, items) {
        tag.insert(key.into(), Tag::List(l));
    }
}

/// What an enchanted item's picture is multiplied by.
pub const PURPLE: egui::Color32 = egui::Color32::from_rgb(0xDC, 0xA6, 0xFF);

/// The icon of an enchantment (an enchanted book from the pack, tinted purple), or a plain mark
/// without a pack.
pub fn paint_icon(p: &egui::Painter, r: egui::Rect, ink: egui::Color32) {
    if crate::icons::item_texture("enchanted_book").is_some_and(|f| crate::icons::file_image_tinted(p, r, &f, PURPLE)) {
        return;
    }
    crate::icons::paint(p, r, crate::icons::Icon::Optimize, ink);
}

/// The same in gray, for the menu row that opens the list.
pub fn paint_icon_gray(p: &egui::Painter, r: egui::Rect, ink: egui::Color32) {
    if crate::icons::item_texture("enchanted_book").is_some_and(|f| crate::icons::gray_image(p, r, &f)) {
        return;
    }
    crate::icons::paint(p, r, crate::icons::Icon::Optimize, ink);
}

/// The "Enchantments" rows of a right-click menu: each enchantment the item has with its level
/// to change and a button to remove it, and "Add Enchantment" at the bottom, a list with a search
/// box. Gives the enchantment to change (its new level, or `None` to take it off).
pub fn menu(ui: &mut egui::Ui, cx: &mut crate::ctx_menu::Cx, current: &[(String, i64)]) -> Option<EnchantEdit> {
    use crate::icons::Icon;
    const ROW: f32 = 26.0;
    // As wide as the longest name needs, and no wider.
    let font = crate::theme::regular(13.5);
    let widest = |names: &[String]| names.iter().map(|n| ui.painter().layout_no_wrap(n.clone(), font.clone(), egui::Color32::WHITE).size().x).fold(0.0, f32::max);
    let current_names: Vec<String> = current.iter().map(|(id, _)| name(id)).collect();
    let free_names: Vec<String> = ALL.iter().map(|e| name(e.0)).collect();
    let w_current = (34.0 + widest(&current_names) + 10.0 + 60.0 + 34.0).clamp(190.0, 300.0);
    let w_add = (34.0 + widest(&free_names) + 24.0).clamp(170.0, 260.0);
    let rows = current.len().max(1) as f32 + 1.0;
    crate::ctx_menu::submenu_painted(ui, cx, paint_icon_gray, "Enchantments", w_current, 50.0 + rows * ROW, |ui| {
        ui.set_min_width(w_current);
        let ctx = ui.ctx().clone();
        let old = crate::ctx_menu::set_row_width(&ctx, w_current);
        let mut out: Option<EnchantEdit> = None;
        if current.is_empty() {
            ui.label(egui::RichText::new("No enchantments").color(crate::theme::text_dim()));
        }
        for (i, (id, level)) in current.iter().enumerate() {
            // A level being dragged is kept until it is let go (each by its place: two of the
            // same are apart).
            let key = egui::Id::new(("enchant-level", i, id.as_str()));
            let mut shown: i64 = ctx.data(|d| d.get_temp(key)).unwrap_or(*level);
            let (changed, done, remove) = crate::ctx_menu::value_row(ui, paint_icon, &name(id), &mut shown, 1..=255);
            if changed {
                ctx.data_mut(|d| d.insert_temp(key, shown));
            }
            if done {
                let v: Option<i64> = ctx.data_mut(|d| d.remove_temp(key));
                if let Some(v) = v.filter(|v| v != level) {
                    out = Some(EnchantEdit::Level(i, v));
                }
            }
            if remove {
                ctx.data_mut(|d| d.remove_temp::<i64>(key));
                out = Some(EnchantEdit::Remove(i));
            }
        }
        crate::ctx_menu::separator(ui);
        let mut inner = crate::ctx_menu::Cx::default();
        let free: Vec<&(&str, i64, i16)> = ALL.iter().collect();
        let list_height = 40.0 + free.len().min(9) as f32 * ROW;
        let added = crate::ctx_menu::submenu_wide(ui, &mut inner, Icon::Plus, "Add Enchantment", w_add, list_height + 8.0, |ui| {
            let old = crate::ctx_menu::set_row_width(ui.ctx(), w_add);
            let r = add_list(ui, &free, w_add);
            crate::ctx_menu::set_row_width(ui.ctx(), old);
            r
        });
        crate::ctx_menu::set_row_width(&ctx, old);
        // Picked: another enchantment in the list, at level 1 (even one it has).
        added.map(|(id, _)| EnchantEdit::Add(id)).or(out)
    })
}
/// The list "Add Enchantment" opens: a search box over the enchantments the item lacks.
fn add_list(ui: &mut egui::Ui, free: &[&(&str, i64, i16)], w: f32) -> Option<(String, Option<i64>)> {
    const ROW: f32 = 26.0;
    let ctx = ui.ctx().clone();
    let (text_id, frame_id) = (egui::Id::new("mctk-ench-search"), egui::Id::new("mctk-ench-search-frame"));
    let frame = ctx.cumulative_pass_nr();
    let last: u64 = ctx.data(|d| d.get_temp(frame_id)).unwrap_or(0);
    let fresh = frame > last + 2;
    ctx.data_mut(|d| d.insert_temp(frame_id, frame));
    let mut query: String = if fresh { String::new() } else { ctx.data(|d| d.get_temp(text_id)).unwrap_or_default() };
    ui.set_width(w);
    ui.spacing_mut().item_spacing = egui::vec2(0.0, 4.0);
    let search = ui.add_sized([w, 28.0], egui::TextEdit::singleline(&mut query).hint_text("Search enchantments").margin(egui::Margin::symmetric(8, 5)));
    if fresh {
        search.request_focus();
    }
    ctx.data_mut(|d| d.insert_temp(text_id, query.clone()));
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    let shown: Vec<&&(&str, i64, i16)> = free.iter().filter(|e| words.iter().all(|w| e.0.replace('_', " ").contains(w.as_str()))).collect();
    if shown.is_empty() {
        ui.label(egui::RichText::new("No enchantment matches.").color(crate::theme::text_dim()));
        return None;
    }
    let mut pick = None;
    egui::ScrollArea::vertical()
        .max_height(ROW * 9.0)
        .auto_shrink([false, true])
        .id_salt("mctk-ench-list")
        .show_rows(ui, ROW, shown.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let e = shown[i];
                let id = format!("minecraft:{}", e.0);
                if crate::ctx_menu::painted_keep(ui, &name(&id), "", true, paint_icon) {
                    pick = Some((id, Some(1)));
                }
            }
        });
    pick
}

#[cfg(test)]
pub fn max_level(id: &str) -> i64 {
    ALL.iter().find(|e| e.0 == short(id)).map_or(5, |e| e.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sword(extra: Option<(&str, Tag)>) -> Compound {
        let mut c = Compound::new();
        c.insert("id".into(), Tag::String("minecraft:diamond_sword".into()));
        if let Some((k, v)) = extra {
            c.insert(k.into(), v);
        }
        c
    }

    #[test]
    fn enchantments_round_trip_in_every_style() {
        // Names in a tag list (1.13 to 1.20.4).
        let mut c = sword(Some(("Count", Tag::Byte(1))));
        set(&mut c, "minecraft:sharpness", Some(5), Some(3465));
        set(&mut c, "unbreaking", Some(3), Some(3465));
        assert_eq!(read(&c), vec![("minecraft:sharpness".into(), 5), ("minecraft:unbreaking".into(), 3)]);
        set(&mut c, "minecraft:sharpness", None, None);
        assert_eq!(read(&c), vec![("minecraft:unbreaking".into(), 3)]);
        set(&mut c, "minecraft:unbreaking", None, None);
        assert!(read(&c).is_empty());
        assert!(c.get("tag").and_then(Tag::as_compound).is_some_and(|t| t.is_empty()));

        // Components, with and without the wrapper.
        for (version, wrapped) in [(4000, true), (4325, false)] {
            let mut c = sword(Some(("count", Tag::Int(1))));
            set(&mut c, "minecraft:mending", Some(1), Some(version));
            let e = c.get("components").and_then(|t| t.get("minecraft:enchantments")).and_then(Tag::as_compound).expect("component");
            assert_eq!(e.contains_key("levels"), wrapped);
            assert_eq!(read(&c), vec![("minecraft:mending".into(), 1)]);
            set(&mut c, "minecraft:mending", None, Some(version));
            assert!(read(&c).is_empty());
        }

        // Numbers in an old world.
        let mut c = Compound::new();
        c.insert("id".into(), Tag::Short(276));
        set(&mut c, "minecraft:sharpness", Some(2), None);
        let tag = c.get("tag").and_then(Tag::as_compound).expect("tag");
        let first = tag.get("ench").and_then(Tag::as_list).and_then(|l| l.get(0)).and_then(Tag::as_compound).expect("entry");
        assert_eq!(first.get("id"), Some(&Tag::Short(16)));
        assert_eq!(read(&c), vec![("minecraft:sharpness".into(), 2)]);
    }

    #[test]
    fn levels_read_as_roman_numerals() {
        assert_eq!(level_text(5), "V");
        assert_eq!(level_text(255), "255");
        assert_eq!(max_level("minecraft:sharpness"), 5);
        assert_eq!(name("minecraft:fire_aspect"), "Fire Aspect");
    }
    /// Adding an enchantment the stack has adds another to the list where the edition keeps a
    /// list (each then changed or taken off by its place); where it keeps one level for each
    /// (Java 1.20.5 on), the stack is left as it is.
    #[test]
    fn the_same_enchantment_twice_where_a_list_allows() {
        let s = |v: &str| Tag::String(v.into());
        let mut old = Compound::new();
        old.insert("id".into(), s("minecraft:diamond_sword"));
        old.insert("Count".into(), Tag::Byte(1));
        apply(&mut old, &EnchantEdit::Add("minecraft:sharpness".into()), Some(3700));
        apply(&mut old, &EnchantEdit::Add("minecraft:sharpness".into()), Some(3700));
        apply(&mut old, &EnchantEdit::Level(1, 5), Some(3700));
        assert_eq!(read(&old), vec![("minecraft:sharpness".to_string(), 1), ("minecraft:sharpness".to_string(), 5)]);
        apply(&mut old, &EnchantEdit::Remove(0), Some(3700));
        assert_eq!(read(&old), vec![("minecraft:sharpness".to_string(), 5)]);
        let mut new = Compound::new();
        new.insert("id".into(), s("minecraft:diamond_sword"));
        new.insert("count".into(), Tag::Int(1));
        apply(&mut new, &EnchantEdit::Add("minecraft:sharpness".into()), Some(4500));
        apply(&mut new, &EnchantEdit::Level(0, 3), Some(4500));
        apply(&mut new, &EnchantEdit::Add("minecraft:sharpness".into()), Some(4500));
        assert_eq!(read(&new), vec![("minecraft:sharpness".to_string(), 3)]);
    }
}