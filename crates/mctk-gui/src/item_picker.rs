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
/// How many rows show before the list scrolls.
const VISIBLE_ROWS: f32 = 9.0;

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
pub fn show(ui: &mut Ui, current: &str, numeric: bool) -> Option<String> {
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
    ui.horizontal(|ui| {
        if widgets::segmented(ui, &mut by_name, &["Id", "Name"], 0.0, true) {
            ctx.data_mut(|d| d.insert_temp(sort_id, by_name));
        }
    });
    let mut shown: Vec<&Item> = items()
        .iter()
        .filter(|i| !numeric || i.number.is_some())
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
        ui.label(egui::RichText::new("No item matches.").color(theme::TEXT_DIM));
        ui.add_space(8.0);
        return None;
    }
    let mut area = egui::ScrollArea::vertical()
        .max_height(ROW * VISIBLE_ROWS)
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
                    p.rect_filled(rect, theme::radius(3), theme::BUTTON_HOVER);
                } else if item.id == current {
                    p.rect_filled(rect, theme::radius(3), Color32::from_white_alpha(14));
                }
                let icon = Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(22.0, 22.0));
                let drawn = icons::item_texture(&item.id).is_some_and(|f| icons::file_image(p, icon, &f));
                if !drawn {
                    p.rect_stroke(icon.shrink(3.0), theme::radius(3), Stroke::new(1.0_f32, theme::TEXT_DIM), egui::StrokeKind::Inside);
                }
                // The id on the right (a number in a world that uses numbers), the name before it.
                let id_text = if numeric { item.number.map(|n| n.to_string()).unwrap_or_default() } else { item.id.clone() };
                let id_font = theme::regular(11.5);
                let id_text = widgets::elide(ui, &id_text, &id_font, rect.width() * 0.5);
                let id_galley = ui.painter().layout_no_wrap(id_text, id_font, theme::TEXT_DIM);
                let id_width = id_galley.size().x;
                ui.painter().galley(
                    pos2(rect.right() - 10.0 - id_width, rect.center().y - id_galley.size().y / 2.0),
                    id_galley,
                    theme::TEXT_DIM,
                );
                let name_font = theme::regular(13.5);
                let name = widgets::elide(ui, &item.name, &name_font, rect.width() - 42.0 - id_width - 22.0);
                ui.painter().text(pos2(rect.left() + 40.0, rect.center().y), Align2::LEFT_CENTER, name, name_font, theme::TEXT);
                if resp.clicked() {
                    picked = Some(item.id.clone());
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
    use super::*;

    /// Lists the items the resource pack has no picture for (how many). Needs `MCTK_TEST_PACK`,
    /// the pack's `assets/minecraft/textures` folder.
    #[test]
    fn items_without_a_picture() {
        let Ok(pack) = std::env::var("MCTK_TEST_PACK") else {
            return;
        };
        icons::set_texture_pack(Some(pack.into()));
        let missing = items().iter().filter(|i| icons::item_texture(&i.id).is_none()).count();
        eprintln!("{missing} of {} have no picture", items().len());
        assert!(missing < items().len() / 4, "{missing}");
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
