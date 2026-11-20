//! The list that "Change Item" and "Add Item" open in right-click menus: every item the game
//! has, with its picture, its name and its id (right-aligned), and a search box on top that
//! finds by name or id. Worlds that name items by number show and search that number.

use std::sync::OnceLock;

use eframe::egui::{self, Align2, Color32, Id, Rect, Sense, Stroke, Ui, pos2, vec2};

use crate::icons;
use crate::theme;
use crate::widgets;

/// The list is this wide, and about this tall (for placing it so it fits on the screen).
pub const WIDTH: f32 = 300.0;
pub const HEIGHT: f32 = 400.0;
const ROW: f32 = 32.0;
/// How many rows show before the list scrolls (at the usual window size).
const VISIBLE_ROWS: f32 = 9.0;

/// How many rows a list shows: as many as `count` needs, up to what fits the window (fewer in
/// a short one, more in a tall one), and never under four.
fn list_rows(ctx: &egui::Context, count: usize) -> f32 {
    // (Room for the search box, the sort and the menu's frame besides; the menu moves up to fit.)
    let fits = ((ctx.screen_rect().height() - 100.0) / ROW).floor().clamp(4.0, 12.0);
    let usual = if ctx.screen_rect().height() >= 640.0 { fits.max(VISIBLE_ROWS) } else { fits };
    usual.min(count.max(1) as f32)
}

pub struct Item {
    /// `minecraft:diamond_sword`
    pub id: String,
    /// `Diamond Sword`
    pub name: String,
    /// What a console or old world calls it: `264`.
    pub number: Option<u16>,
    /// Where the game lists it (its id order).
    order: usize,
    /// The name and id, lower case, to search in.
    search: String,
    /// The name and number, for worlds that use numbers.
    search_numbered: String,
}

/// `diamond_sword` as "Diamond Sword".
pub fn display_name(id: &str) -> String {
    id.rsplit(':')
        .next()
        .unwrap_or(id)
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            c.next().map_or(String::new(), |f| f.to_uppercase().chain(c).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every item of the newest Java version, by id.
pub fn items() -> &'static [Item] {
    static ITEMS: OnceLock<Vec<Item>> = OnceLock::new();
    ITEMS.get_or_init(|| {
        let newest = mctk_registry::builtin::java_versions()[0];
        let mut ids: Vec<String> = mctk_registry::builtin::java(newest)
            .map(|r| r.items.iter().map(|i| if i.contains(':') { i.clone() } else { format!("minecraft:{i}") }).collect())
            .unwrap_or_default();
        // In the game's order, then the blocks that are no item (fire, lava, water ...).
        let mut seen = std::collections::HashSet::new();
        ids.retain(|i| seen.insert(i.clone()));
        // Every item the latest game's own assets define (`items/<id>.json`), which can be newer
        // than the built-in list: those it lacks follow, by name.
        let mut newer: Vec<String> = crate::assets::list("assets/minecraft/items")
            .into_iter()
            .filter_map(|f| f.strip_suffix(".json").map(|n| format!("minecraft:{n}")))
            .filter(|id| !seen.contains(id))
            .collect();
        newer.sort();
        for id in newer {
            seen.insert(id.clone());
            ids.push(id);
        }
        if let Some(r) = mctk_registry::builtin::java(newest) {
            for b in r.blocks.keys() {
                let id = if b.contains(':') { b.clone() } else { format!("minecraft:{b}") };
                let n = id.rsplit(':').next().unwrap_or("");
                let noise = n.ends_with("air") || n.starts_with("potted_") || n.contains("wall_");
                if !noise && seen.insert(id.clone()) {
                    ids.push(id);
                }
            }
        }
        let legacy = mctk_mapping::legacy::legacy();
        ids.into_iter()
            .enumerate()
            .map(|(order, id)| {
                let name = display_name(&id);
                let number = legacy.numeric_item(&id).map(|(n, _)| n);
                let search = format!("{} {}", name.to_lowercase(), id.to_lowercase());
                let search_numbered = format!("{} {}", name.to_lowercase(), number.map(|n| n.to_string()).unwrap_or_default());
                Item { id, name, number, order, search, search_numbered }
            })
            .collect()
    })
}

/// The search box and the list. Gives the id of the item picked, once, and closes the menu.
/// `numeric` is for worlds that name items by number: only items that have one are listed, with
/// that number, and the search finds by it.
#[allow(dead_code)]
pub fn show(ui: &mut Ui, current: &str, numeric: bool) -> Option<String> {
    show_list(ui, current, numeric, false, None).and_then(|(id, _)| id)
}

/// Like [`show`], with how many at the top beside the sort (`count` now, and the most it can
/// be). Gives the item picked with the count, or (no item) a count changed on its own.
pub fn show_counted(ui: &mut Ui, current: &str, numeric: bool, count: i64, most: i64) -> Option<(Option<String>, i64)> {
    show_list(ui, current, numeric, false, Some((count, most)))
}

/// Like [`show`], listing only blocks (for the blocks an item can break or be placed on).
pub fn show_blocks(ui: &mut Ui, current: &str) -> Option<String> {
    show_list(ui, current, false, true, None).and_then(|(id, _)| id)
}

/// Whether an id is a block of the newest Java version.
pub fn is_block(id: &str) -> bool {
    static BLOCKS: OnceLock<std::collections::HashSet<String>> = OnceLock::new();
    let blocks = BLOCKS.get_or_init(|| {
        let newest = mctk_registry::builtin::java_versions()[0];
        mctk_registry::builtin::java(newest).map(|r| r.blocks.keys().map(|b| b.trim_start_matches("minecraft:").to_string()).collect()).unwrap_or_default()
    });
    blocks.contains(id.trim_start_matches("minecraft:"))
}

fn show_list(ui: &mut Ui, current: &str, numeric: bool, blocks_only: bool, count: Option<(i64, i64)>) -> Option<(Option<String>, i64)> {
    let ctx = ui.ctx().clone();
    // A fresh opening starts with an empty search and the box ready to type in.
    let (text_id, frame_id) = (Id::new("mctk-item-search"), Id::new("mctk-item-search-frame"));
    let frame = ctx.cumulative_pass_nr();
    let last: u64 = ctx.data(|d| d.get_temp(frame_id)).unwrap_or(0);
    let fresh = frame > last + 2;
    ctx.data_mut(|d| d.insert_temp(frame_id, frame));
    let mut query: String = if fresh { String::new() } else { ctx.data(|d| d.get_temp(text_id)).unwrap_or_default() };

    // Exactly as wide as the list, so the menu's frame fits it and nothing sticks out.
    ui.set_width(WIDTH);
    ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
    let search = ui.add_sized(
        [WIDTH, 30.0],
        egui::TextEdit::singleline(&mut query)
            .hint_text("Search by name or id")
            .margin(egui::Margin::symmetric(8, 6)),
    );
    if fresh {
        search.request_focus();
    }
    ctx.data_mut(|d| d.insert_temp(text_id, query.clone()));

    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    // Listed by id (the number, in a world that has them), or by name; the choice is kept.
    let sort_id = Id::new("mctk-item-sort");
    let mut by_name: usize = ctx.data(|d| d.get_temp(sort_id)).unwrap_or(0);
    // How many (when asked), beside the sort: kept while the list is open, from `count` when it
    // opens afresh.
    let count_id = Id::new("mctk-item-count");
    let mut chosen_count = count.map(|(c, _)| c).unwrap_or(1);
    let mut count_done = None;
    ui.horizontal(|ui| {
        if widgets::segmented(ui, &mut by_name, &["Id", "Name"], 0.0, true) {
            ctx.data_mut(|d| d.insert_temp(sort_id, by_name));
        }
        if let Some((start, most)) = count {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut v: i64 = if fresh { start } else { ctx.data(|d| d.get_temp(count_id)).unwrap_or(start) };
                let r = ui.add(egui::DragValue::new(&mut v).range(1..=most));
                ui.label(egui::RichText::new("Count").color(theme::text_muted()));
                ctx.data_mut(|d| d.insert_temp(count_id, v));
                chosen_count = v;
                if (r.drag_stopped() || r.lost_focus()) && v != start {
                    count_done = Some(v);
                }
            });
        }
    });
    let mut shown: Vec<&Item> = items()
        .iter()
        .filter(|i| !numeric || i.number.is_some())
        .filter(|i| !blocks_only || is_block(&i.id))
        .filter(|i| {
            let hay = if numeric { &i.search_numbered } else { &i.search };
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .collect();
    if by_name == 1 {
        shown.sort_by(|a, b| a.name.cmp(&b.name));
    } else if numeric {
        shown.sort_by_key(|i| (i.number.unwrap_or(u16::MAX), i.order));
    }
    let mut picked = None;
    if shown.is_empty() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("No item matches.").color(theme::text_dim()));
        ui.add_space(8.0);
        return count_done.map(|c| (None, c));
    }
    // Never squeezed by the room under where it opened (the menu moves up instead).
    let rows = list_rows(&ctx, shown.len());
    let mut area = egui::ScrollArea::vertical()
        .max_height(ROW * rows)
        .min_scrolled_height(ROW * rows)
        .auto_shrink([false, true])
        .id_salt("mctk-item-list");
    // A fresh opening, or a new search, starts at the top.
    let seen_query: String = ctx.data(|d| d.get_temp(Id::new("mctk-item-seen"))).unwrap_or_default();
    if fresh || seen_query != query {
        area = area.vertical_scroll_offset(0.0);
        ctx.data_mut(|d| d.insert_temp(Id::new("mctk-item-seen"), query.clone()));
    }
    area.show_rows(ui, ROW, shown.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for i in range {
                let item = shown[i];
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(rect, theme::radius(3), theme::button_hover());
                } else if item.id == current {
                    p.rect_filled(rect, theme::radius(3), theme::wash(14));
                }
                let icon = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(22.0, 22.0));
                let drawn = icons::item_icon(p, icon, &item.id, Color32::WHITE);
                if !drawn {
                    p.rect_stroke(icon.shrink(3.0), theme::radius(3), Stroke::new(1.0_f32, theme::text_dim()), egui::StrokeKind::Inside);
                }
                // The id on the right (a number in a world that uses numbers), the name before it.
                let id_text = if numeric { item.number.map(|n| n.to_string()).unwrap_or_default() } else { item.id.clone() };
                let id_font = theme::regular(11.5);
                let id_text = widgets::elide(ui, &id_text, &id_font, rect.width() * 0.5);
                let id_galley = ui.painter().layout_no_wrap(id_text, id_font, theme::text_dim());
                let id_width = id_galley.size().x;
                ui.painter().galley(
                    pos2(rect.right() - 10.0 - id_width, rect.center().y - id_galley.size().y / 2.0),
                    id_galley,
                    theme::text_dim(),
                );
                let name_font = theme::regular(13.5);
                let name = widgets::elide(ui, &item.name, &name_font, rect.width() - 42.0 - id_width - 22.0);
                ui.painter().text(pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER, name, name_font, theme::text());
                if resp.clicked() {
                    picked = Some(item.id.clone());
                }
            }
        });
    if picked.is_some() {
        crate::ctx_menu::chosen(ui);
    }
    match picked {
        Some(id) => Some((Some(id), chosen_count)),
        None => count_done.map(|c| (None, c)),
    }
}

// ---------------------------------------------------------------------------------------------
// Loot tables (for a mob's `DeathLootTable`)
// ---------------------------------------------------------------------------------------------

pub struct LootTable {
    /// `minecraft:entities/zombie`
    pub id: String,
    /// "Zombie"
    pub name: String,
    /// The item it is pictured by.
    icon: String,
    search: String,
}

const CHEST_TABLES: &[(&str, &str)] = &[
    ("abandoned_mineshaft", "chest_minecart"),
    ("ancient_city", "sculk"),
    ("ancient_city_ice_box", "packed_ice"),
    ("bastion_bridge", "gilded_blackstone"),
    ("bastion_hoglin_stable", "gilded_blackstone"),
    ("bastion_other", "gilded_blackstone"),
    ("bastion_treasure", "gilded_blackstone"),
    ("buried_treasure", "heart_of_the_sea"),
    ("desert_pyramid", "sandstone"),
    ("end_city_treasure", "purpur_block"),
    ("igloo_chest", "snow_block"),
    ("jungle_temple", "mossy_cobblestone"),
    ("jungle_temple_dispenser", "dispenser"),
    ("nether_bridge", "nether_bricks"),
    ("pillager_outpost", "crossbow"),
    ("ruined_portal", "crying_obsidian"),
    ("shipwreck_map", "map"),
    ("shipwreck_supply", "oak_planks"),
    ("shipwreck_treasure", "emerald"),
    ("simple_dungeon", "spawner"),
    ("spawn_bonus_chest", "chest"),
    ("stronghold_corridor", "stone_bricks"),
    ("stronghold_crossing", "stone_bricks"),
    ("stronghold_library", "bookshelf"),
    ("trial_chambers/reward", "trial_key"),
    ("trial_chambers/reward_ominous", "ominous_trial_key"),
    ("trial_chambers/supply", "chest"),
    ("trial_chambers/corridor", "chest"),
    ("underwater_ruin_big", "prismarine"),
    ("underwater_ruin_small", "prismarine"),
    ("village/village_armorer", "blast_furnace"),
    ("village/village_butcher", "smoker"),
    ("village/village_cartographer", "cartography_table"),
    ("village/village_desert_house", "chest"),
    ("village/village_fisher", "barrel"),
    ("village/village_fletcher", "fletching_table"),
    ("village/village_mason", "stonecutter"),
    ("village/village_plains_house", "chest"),
    ("village/village_savanna_house", "chest"),
    ("village/village_shepherd", "loom"),
    ("village/village_snowy_house", "chest"),
    ("village/village_taiga_house", "chest"),
    ("village/village_tannery", "cauldron"),
    ("village/village_temple", "brewing_stand"),
    ("village/village_toolsmith", "smithing_table"),
    ("village/village_weaponsmith", "grindstone"),
    ("woodland_mansion", "dark_oak_planks"),
];

const GAMEPLAY_TABLES: &[(&str, &str)] = &[
    ("fishing", "fishing_rod"),
    ("fishing/fish", "cod"),
    ("fishing/junk", "leather_boots"),
    ("fishing/treasure", "name_tag"),
    ("cat_morning_gift", "string"),
    ("piglin_bartering", "gold_ingot"),
    ("sniffer_digging", "sniffer_egg"),
    ("panda_sneeze", "slime_ball"),
    ("armadillo_shed", "armadillo_scute"),
    ("chicken_lay", "egg"),
];

const COLORS: [&str; 16] = [
    "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green", "red", "black",
];

/// Every loot table a mob can be given: each mob's own (by its spawn egg), the sheep's by
/// color, the chests' and the gameplay ones, and the empty one.
pub fn loot_tables() -> &'static [LootTable] {
    static TABLES: OnceLock<Vec<LootTable>> = OnceLock::new();
    TABLES.get_or_init(|| {
        let mut out: Vec<LootTable> = Vec::new();
        let mut push = |path: String, name: String, icon: String| {
            let id = format!("minecraft:{path}");
            let search = format!("{} {}", name.to_lowercase(), id);
            out.push(LootTable { id, name, icon, search });
        };
        push("empty".into(), "No Loot".into(), "barrier".into());
        let mut mobs: Vec<String> = items().iter().filter_map(|i| i.id.strip_prefix("minecraft:")?.strip_suffix("_spawn_egg").map(str::to_string)).collect();
        for m in ["ender_dragon", "wither", "iron_golem", "snow_golem", "giant", "illusioner"] {
            if !mobs.iter().any(|x| x == m) {
                mobs.push(m.to_string());
            }
        }
        mobs.sort();
        for m in mobs {
            let egg = format!("{m}_spawn_egg");
            let icon = if items().iter().any(|i| i.id == format!("minecraft:{egg}")) { egg } else { "spawner".into() };
            push(format!("entities/{m}"), display_name(&m), icon);
            if m == "sheep" {
                for c in COLORS {
                    push(format!("entities/sheep/{c}"), format!("Sheep ({})", display_name(c)), format!("{c}_wool"));
                }
            }
        }
        for (t, icon) in CHEST_TABLES {
            push(format!("chests/{t}"), format!("Chest: {}", display_name(t.rsplit('/').next().unwrap_or(t))), (*icon).into());
        }
        for (t, icon) in GAMEPLAY_TABLES {
            push(format!("gameplay/{t}"), display_name(&t.replace('/', "_")), (*icon).into());
        }
        out
    })
}

/// The search box and the list of loot tables, with "Default" (the mob's own, `own`) at the top.
/// Gives the table picked (`None`: the default) and closes the menu.
pub fn show_loot_tables(ui: &mut Ui, current: Option<&str>, own: &str) -> Option<Option<String>> {
    let default_name = format!("Default ({})", display_name(own.rsplit('/').next().unwrap_or(own)));
    let entries: Vec<(&str, &str, &str, &str)> = loot_tables().iter().map(|t| (t.id.as_str(), t.name.as_str(), t.icon.as_str(), t.search.as_str())).collect();
    show_choices(ui, "loot", current, &entries, Some((default_name.as_str(), own)))
}

/// Every mob a spawner can spawn: those with a spawn egg, and a few with none.
pub fn mobs() -> &'static [(String, String, String, String)] {
    static MOBS: OnceLock<Vec<(String, String, String, String)>> = OnceLock::new();
    MOBS.get_or_init(|| {
        let mut names: Vec<String> = items().iter().filter_map(|i| i.id.strip_prefix("minecraft:")?.strip_suffix("_spawn_egg").map(str::to_string)).collect();
        for m in ["ender_dragon", "wither", "iron_golem", "snow_golem", "giant", "illusioner"] {
            if !names.iter().any(|x| x == m) {
                names.push(m.to_string());
            }
        }
        names.sort();
        names
            .into_iter()
            .map(|m| {
                let id = format!("minecraft:{m}");
                let name = display_name(&m);
                let egg = format!("minecraft:{m}_spawn_egg");
                let icon = if items().iter().any(|i| i.id == egg) { egg } else { "minecraft:spawner".into() };
                let search = format!("{} {}", name.to_lowercase(), id);
                (id, name, icon, search)
            })
            .collect()
    })
}

/// The search box and the list of mobs (for a spawner). Gives the mob picked and closes the menu.
pub fn show_mobs(ui: &mut Ui, current: Option<&str>) -> Option<String> {
    let entries: Vec<(&str, &str, &str, &str)> = mobs().iter().map(|(a, b, c, d)| (a.as_str(), b.as_str(), c.as_str(), d.as_str())).collect();
    show_choices(ui, "mobs", current, &entries, None).flatten()
}

/// A search box over a list of (id, name, icon, text to search), and above them, if given, a
/// "default" row (its name and the id shown beside it). Gives the entry picked (`None`: the
/// default) and closes the menu.
fn show_choices(ui: &mut Ui, key: &str, current: Option<&str>, entries: &[(&str, &str, &str, &str)], default: Option<(&str, &str)>) -> Option<Option<String>> {
    let ctx = ui.ctx().clone();
    let (text_id, frame_id) = (Id::new(("mctk-choice-search", key)), Id::new(("mctk-choice-search-frame", key)));
    let frame = ctx.cumulative_pass_nr();
    let last: u64 = ctx.data(|d| d.get_temp(frame_id)).unwrap_or(0);
    let fresh = frame > last + 2;
    ctx.data_mut(|d| d.insert_temp(frame_id, frame));
    let mut query: String = if fresh { String::new() } else { ctx.data(|d| d.get_temp(text_id)).unwrap_or_default() };
    ui.set_width(WIDTH);
    ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
    let search = ui.add_sized([WIDTH, 30.0], egui::TextEdit::singleline(&mut query).hint_text("Search by name or id").margin(egui::Margin::symmetric(8, 6)));
    if fresh {
        search.request_focus();
    }
    ctx.data_mut(|d| d.insert_temp(text_id, query.clone()));
    let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
    // The default first (while the search finds it), then the entries found.
    let default_shown = default.is_some_and(|(name, _)| words.iter().all(|w| name.to_lowercase().contains(w.as_str()) || "default".contains(w.as_str())));
    let shown: Vec<&(&str, &str, &str, &str)> = entries.iter().filter(|t| words.iter().all(|w| t.3.contains(w.as_str()))).collect();
    let total = shown.len() + usize::from(default_shown);
    if total == 0 {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Nothing matches.").color(theme::text_dim()));
        ui.add_space(8.0);
        return None;
    }
    let rows = list_rows(&ctx, total);
    let mut area = egui::ScrollArea::vertical().max_height(ROW * rows).min_scrolled_height(ROW * rows).auto_shrink([false, true]).id_salt(("mctk-choice-list", key));
    let seen_id = Id::new(("mctk-choice-seen", key));
    let seen_query: String = ctx.data(|d| d.get_temp(seen_id)).unwrap_or_default();
    if fresh || seen_query != query {
        area = area.vertical_scroll_offset(0.0);
        ctx.data_mut(|d| d.insert_temp(seen_id, query.clone()));
    }
    let mut picked: Option<Option<String>> = None;
    area.show_rows(ui, ROW, total, |ui, range| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for i in range {
            let entry = if default_shown { i.checked_sub(1).map(|k| shown[k]) } else { Some(shown[i]) };
            let (name, id, icon, now) = match (entry, default) {
                (Some(t), _) => (t.1, t.0, t.2, current == Some(t.0)),
                (None, Some((dn, did))) => (dn, did, "spawner", current.is_none()),
                (None, None) => continue,
            };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
            let p = ui.painter();
            if resp.hovered() {
                p.rect_filled(rect, theme::radius(3), theme::button_hover());
            } else if now {
                p.rect_filled(rect, theme::radius(3), theme::wash(14));
            }
            let icon_rect = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(22.0, 22.0));
            if !icons::item_icon(p, icon_rect, icon, Color32::WHITE) {
                p.rect_stroke(icon_rect.shrink(3.0), theme::radius(3), Stroke::new(1.0_f32, theme::text_dim()), egui::StrokeKind::Inside);
            }
            let id_font = theme::regular(11.5);
            let id_text = widgets::elide(ui, id.trim_start_matches("minecraft:"), &id_font, rect.width() * 0.5);
            let id_galley = ui.painter().layout_no_wrap(id_text, id_font, theme::text_dim());
            let id_width = id_galley.size().x;
            ui.painter().galley(pos2(rect.right() - 10.0 - id_width, rect.center().y - id_galley.size().y / 2.0), id_galley, theme::text_dim());
            let name_font = theme::regular(13.5);
            let name = widgets::elide(ui, name, &name_font, rect.width() - 42.0 - id_width - 22.0);
            ui.painter().text(pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER, name, name_font, theme::text());
            if resp.clicked() {
                picked = Some(entry.map(|t| t.0.to_string()));
            }
        }
    });
    if picked.is_some() {
        crate::ctx_menu::chosen(ui);
    }
    picked
}

#[cfg(test)]
mod tests {
    #[test]
    fn loot_tables_have_the_mobs_and_the_chests() {
        let all = loot_tables();
        assert!(all.iter().any(|t| t.id == "minecraft:entities/zombie"));
        assert!(all.iter().any(|t| t.id == "minecraft:entities/sheep/white"));
        assert!(all.iter().any(|t| t.id == "minecraft:chests/simple_dungeon"));
        assert!(all.iter().any(|t| t.id == "minecraft:empty"));
    }

    use super::*;

    /// Lists the items the game's assets have no picture for (how many). Fetches the item
    /// pictures, so only with `MCTK_TEST_ITEMS` set.
    #[test]
    fn items_without_a_picture() {
        if std::env::var("MCTK_TEST_ITEMS").is_err() {
            return;
        }
        // Every entry of the list (items, and blocks that are no item) drawn as the list draws
        // it; air is nothing to draw, in the game too.
        let ctx = egui::Context::default();
        let mut missing: Vec<String> = Vec::new();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let r = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(32.0, 32.0));
                for i in items() {
                    let bare = i.id.trim_start_matches("minecraft:");
                    if !bare.ends_with("air") && !icons::item_icon(ui.painter(), r, &i.id, Color32::WHITE) {
                        missing.push(i.id.clone());
                    }
                }
            });
        });
        eprintln!("{} of {} have no picture: {missing:?}", missing.len(), items().len());        assert!(missing.len() < 10, "{missing:?}");
    }

    #[test]
    fn the_list_draws_and_a_search_narrows_it() {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        for numeric in [false, true] {
            let frame = |ctx: &egui::Context| {
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))),
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        assert_eq!(show(ui, "minecraft:stone", numeric), None);
                    });
                });
            };
            frame(&ctx);
            // Typing into the box (the stored text) leaves a shorter list that still draws.
            ctx.data_mut(|d| d.insert_temp(Id::new("mctk-item-search"), "diamond".to_string()));
            frame(&ctx);
            frame(&ctx);
            let kept: String = ctx.data(|d| d.get_temp(Id::new("mctk-item-search"))).unwrap_or_default();
            assert_eq!(kept, "diamond");
        }
    }

    #[test]
    fn numbers_are_found_in_a_world_that_uses_them() {
        let diamond = items().iter().find(|i| i.id == "minecraft:diamond").expect("diamond");
        let number = diamond.number.expect("a number");
        assert!(diamond.search_numbered.contains(&number.to_string()));
        assert!(!diamond.search.contains(&number.to_string()) || number < 10);
    }

    #[test]
    fn names_come_from_ids() {
        assert_eq!(display_name("minecraft:diamond_sword"), "Diamond Sword");
        assert_eq!(display_name("tnt"), "Tnt");
    }

    #[test]
    fn the_list_has_the_items_and_finds_by_name_or_id() {
        let all = items();
        assert!(all.len() > 1000, "{}", all.len());
        let find = |q: &str| all.iter().filter(|i| q.split_whitespace().all(|w| i.search.contains(w))).count();
        assert!(find("diamond sword") >= 1);
        assert!(find("minecraft:netherite_ingot") == 1);
        assert!(find("zzzz-not-an-item") == 0);
        assert!(all.windows(2).all(|w| w[0].order < w[1].order));
    }
}
