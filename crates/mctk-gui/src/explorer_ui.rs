//! The World Explorer's toolbars, in two layouts the user is choosing between:
//!
//! - **Strip**: a column of tool icons down the left of the view, and one bar on top with
//!   the block and the settings of what is being done (the selection's height, the brush's size,
//!   the paste's turn...).
//! - **Grouped**: small toolbars on top (select, blocks, clipboard, file, history) that wrap to
//!   more rows and drop their names when short of room, with the same settings bar under them.
//!
//! Both run the same actions and show the same settings (see [`context_row`]).

use eframe::egui::{self, Rect, Ui, Vec2, pos2, vec2};

use crate::ctx_menu;
use crate::icons::Icon;
use crate::theme;
use crate::widgets;
use crate::workspace::{EditTool, Workspace};

/// Something a toolbar button asks for.
#[derive(Clone, Debug, PartialEq)]
enum Act {
    Select,
    BoxSelect,
    Brush,
    Biome,
    Cloud,
    Place,
    Paste,
    OpenChunks,
    Fill,
    Replace(String),
    DeleteEntities,
    Copy,
    Cut,
    Import,
    Export,
}

/// Which mode the view is in: selecting (a click selects), making boxes, painting, pasting.
fn mode(ws: &Workspace) -> Act {
    if ws.map.paste.is_some() {
        Act::Paste
    } else if ws.map.brush.as_ref().is_some_and(|b| b.biome.is_some()) {
        Act::Biome
    } else if ws.map.brush.is_some() {
        Act::Brush
    } else if ws.map.cloud.is_some() {
        Act::Cloud
    } else if ws.map.place.is_some() {
        Act::Place
    } else if ws.map.box_mode {
        Act::BoxSelect
    } else {
        Act::Select
    }
}

/// A toolbar button: an icon, and its name when there is room (`text` empty: the icon alone);
/// lit when `on`. Gives its response (with `tip` on hover).
fn button(ui: &mut Ui, icon: Icon, text: &str, enabled: bool, on: bool, tip: &str) -> egui::Response {
    let resp = widgets::button(ui, Some(icon), text, if text.is_empty() { 36.0 * theme::ui_scale() } else { 0.0 }, enabled).on_hover_text(tip);
    if on {
        ui.painter().rect_stroke(resp.rect, theme::radius(4), egui::Stroke::new(1.5_f32, theme::accent()), egui::StrokeKind::Inside);
    }
    resp
}

/// How wide a toolbar button is (as [`button`] draws it).
fn button_width(ui: &Ui, text: &str) -> f32 {
    let k = theme::ui_scale();
    if text.is_empty() {
        return 36.0 * k;
    }
    let w = ui.painter().layout_no_wrap(text.to_string(), theme::regular(14.0 * k), theme::text()).size().x;
    w + 26.0 * k + 24.0 * k
}

/// One button of a toolbar: what it does, its icon, its name (long and short), whether it can
/// be used, whether it is lit, its tooltip.
struct Tool {
    act: Act,
    icon: Icon,
    long: &'static str,
    short: &'static str,
    enabled: bool,
    on: bool,
    tip: &'static str,
}

fn tools(ws: &Workspace) -> Vec<Vec<Tool>> {
    let boxed = ws.map.sel_box.is_some();
    let m = mode(ws);
    let t = |act: Act, icon, long, short, enabled, tip| Tool { on: act == m, act, icon, long, short, enabled, tip };
    let open = t(Act::OpenChunks, Icon::Folder, "Open Chunks", "Open", !ws.map.selected.is_empty(), "Open the selected chunks' data in tabs");
    // On the flat map only chunks are picked: the block tools come with the blocks (Top-down,
    // 3D).
    if !ws.map.shows_blocks() {
        return vec![vec![open]];
    }
    vec![
        vec![open, t(Act::Import, Icon::File, "Schematic\u{2026}", "Schematic", true, "Import a schematic to paste, or export the selection as one")],
        vec![
            t(Act::Select, Icon::Pointer, "Select", "Select", true, "Select: click a block, plant or mob in 3D (Shift+click adds blocks); Esc clears"),
            t(Act::BoxSelect, Icon::ShapeBox, "Box Select", "Box", true, "Box Select: click the first block, move for width and length and click, then move for the height and click (on the flat map: two corners, or Shift+drag)"),
        ],
        vec![
            t(Act::Fill, Icon::Plus, "Fill", "Fill", boxed, "Fill the selection with the block"),
            t(Act::Replace(String::new()), Icon::Refresh, "Replace\u{2026}", "Replace", boxed, "Replace: pick one of the blocks in the selection to turn into the block chosen below (stairs keep facing the same way...)"),
            t(Act::Place, Icon::Place, "Place", "Place", true, "Place one block, mob or item: choose which, then click a block to put it on top (mobs and items in Java worlds); Esc stops"),
            t(Act::Brush, Icon::Rename, "Brush", "Brush", true, "Paint a ball or cube of the block by clicking (Alt+drag paints along); Esc stops"),
            t(Act::Biome, Icon::Biome, "Biome", "Biome", true, "Paint a biome by clicking: the game keeps biomes in cells of 4 x 4 x 4 blocks, so whole cells are painted; choose the biome, the size and the heights (Alt+drag paints along); Esc stops"),
            t(Act::DeleteEntities, Icon::MobRemove, "Delete Entities", "Entities", boxed, "Remove the mobs, items, minecarts, frames... in the selection"),
            t(Act::Cloud, Icon::Particles, "Potion Cloud", "Cloud", ws.edition == crate::world_picker::UiEdition::Java, "Place a lingering potion cloud: choose its size, effects, strength, time and whether its particles show; click a block to put it there (Java worlds); Esc stops"),
        ],
        vec![
            t(Act::Copy, Icon::Copy, "Copy", "Copy", boxed, "Copy the selection (blocks, chest contents, mobs...)"),
            t(Act::Cut, Icon::Cut, "Cut", "Cut", boxed, "Copy the selection, then clear it"),
            t(Act::Paste, Icon::Paste, "Paste", "Paste", ws.map.clipboard.is_some(), "Place the copy: it follows the pointer, click to paste; R turns, M mirrors, arrows and Page Up/Down nudge, Esc stops"),
        ],
    ]
}
/// Draws a tool (with its menu for Replace and Schematic); gives what it asks for.
fn tool_ui(ui: &mut Ui, ws: &Workspace, tool: &Tool, text: &str) -> Option<Act> {
    let resp = button(ui, tool.icon, text, tool.enabled, tool.on, tool.tip);
    match &tool.act {
        Act::Replace(_) => {
            // The blocks in the selection (most first): the one clicked becomes the block chosen.
            let mut from = None;
            if tool.enabled {
                ctx_menu::dropdown(ui, &resp, |ui, _| {
                    ctx_menu::widen(ui, 280.0);
                    let into = crate::item_picker::display_name(&ws.map.block.name);
                    ui.label(egui::RichText::new(format!("Turn which block into {into}?")).color(theme::text_muted()));
                    let list = blocks_in_selection(ui.ctx(), ws);
                    if list.is_empty() {
                        ui.label(egui::RichText::new("The selection is only air").color(theme::text_dim()));
                    }
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for (name, n) in list.iter() {
                            if ctx_menu::item(ui, Icon::Refresh, &crate::item_picker::display_name(name), &n.to_string(), true) {
                                from = Some(name.clone());
                            }
                        }
                    });
                });
            }
            from.map(Act::Replace)
        }
        Act::Import => {
            let (mut import, mut export) = (false, false);
            ctx_menu::dropdown(ui, &resp, |ui, _| {
                ctx_menu::widen(ui, 330.0);
                import = ctx_menu::item(ui, Icon::File, "Import\u{2026}", ".schem .nbt .mcstructure .schematic", true);
                export = ctx_menu::item(ui, Icon::Save, "Export Selection\u{2026}", "", ws.map.sel_box.is_some());
            });
            if import {
                Some(Act::Import)
            } else if export {
                Some(Act::Export)
            } else {
                None
            }
        }
        a => resp.clicked().then(|| a.clone()),
    }
}

/// The kinds of block in the selection with how many of each (air left out), most first:
/// counted when Replace's menu opens (or the selection changes) and kept while it is open.
fn blocks_in_selection(ctx: &egui::Context, ws: &Workspace) -> std::sync::Arc<Vec<(String, usize)>> {
    type Kept = (Option<([i32; 3], [i32; 3])>, u64, std::sync::Arc<Vec<(String, usize)>>);
    let id = egui::Id::new("explorer-replace-blocks");
    let now = ctx.cumulative_pass_nr();
    let sel = ws.map.sel_box;
    if let Some((s, seen, list)) = ctx.data(|d| d.get_temp::<Kept>(id)) {
        if s == sel && seen + 2 >= now {
            ctx.data_mut(|d| d.insert_temp::<Kept>(id, (s, now, list.clone())));
            return list;
        }
    }
    let mut counts: std::collections::HashMap<String, usize> = Default::default();
    if let (Some(b), Ok(base)) = (sel, ws.edit_base(&ws.map.dimension)) {
        let edits = ws.map.edits.clone();
        if let Ok(mut layer) = edits.write() {
            let ed = crate::edit::chunks::Editor { layer: &mut layer, base, dim: ws.map.dimension.clone(), notes: Vec::new() };
            let clip = crate::edit::tools::copy(&ed, b);
            for &i in &clip.blocks {
                if let Some(s) = clip.palette.get(i as usize) {
                    if !s.name.ends_with(":air") && !s.name.ends_with("cave_air") && !s.name.ends_with("void_air") {
                        *counts.entry(s.name.clone()).or_default() += 1;
                    }
                }
            }
        }
    }
    let mut list: Vec<(String, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let list = std::sync::Arc::new(list);
    ctx.data_mut(|d| d.insert_temp::<Kept>(id, (sel, now, list.clone())));
    list
}

/// Does what a button asked for.
fn run(ws: &mut Workspace, act: Act, status: &mut String) {
    let say = |status: &mut String, r: Result<String, String>| {
        *status = match r {
            Ok(m) | Err(m) => m,
        };
    };
    let set_mode = |ws: &mut Workspace, m: &Act| {
        let on = mode(ws) == *m;
        ws.map.box_mode = false;
        ws.map.brush = None;
        ws.map.paste = None;
        ws.map.cloud = None;
        ws.map.place = None;
        ws.map.moving = None;
        ws.map.pick_rider = None;
        // A tool that paints or places is not for the mob that was selected: it is let go.
        if matches!(m, Act::Brush | Act::Biome | Act::Cloud | Act::Paste | Act::Place) {
            ws.map.sel_entity = None;
            ws.map.cloud_edit = None;
        }
        match m {
            Act::BoxSelect if !on => ws.map.box_mode = true,
            Act::Brush if !on => ws.map.brush = Some(Default::default()),
            Act::Biome if !on => ws.map.brush = Some(crate::map_view::BrushMode::for_biome()),
            Act::Cloud if !on => ws.map.cloud = Some(Default::default()),
            Act::Place if !on => ws.map.place = Some(Default::default()),
            Act::Paste if !on => ws.map.paste = Some(Default::default()),
            _ => {}
        }
    };
    match act {
        Act::Select | Act::BoxSelect => {
            // A fresh start: what was selected before is let go.
            set_mode(ws, &act);
            ws.map.sel_box = None;
            ws.map.sel_entity = None;
            ws.map.box_changed();
        }
        Act::Brush | Act::Biome | Act::Paste | Act::Cloud | Act::Place => set_mode(ws, &act),
        Act::OpenChunks => {
            let sel = ws.map.selected.clone();
            crate::editor::open_chunks(ws, &sel, status);
        }
        Act::Fill => say(status, ws.run_edit(EditTool::Fill(blocks_chosen(&ws.map)))),
        Act::Replace(from) => say(status, ws.run_edit(EditTool::Replace { from: vec![from], to: blocks_chosen(&ws.map), keep: true })),
        Act::DeleteEntities => say(status, ws.run_edit(EditTool::DeleteEntities)),
        Act::Copy => say(status, ws.copy_box(false)),
        Act::Cut => say(status, ws.copy_box(true)),
        Act::Import => {
            let picked = rfd::FileDialog::new().set_title("Import a schematic").add_filter("Schematics", &["schem", "nbt", "mcstructure", "schematic"]).pick_file();
            if let Some(p) = picked {
                say(status, ws.import_schematic(&p));
            }
        }
        Act::Export => {
            use crate::edit::schematic::Format;
            let first = match ws.edition {
                crate::world_picker::UiEdition::Java => Format::Sponge,
                crate::world_picker::UiEdition::Bedrock => Format::McStructure,
                _ => Format::Legacy,
            };
            let mut d = rfd::FileDialog::new().set_title("Export the selection").set_file_name(format!("selection.{}", first.extension())).add_filter(first.name(), &[first.extension()]);
            for f in Format::ALL.into_iter().filter(|f| *f != first) {
                d = d.add_filter(f.name(), &[f.extension()]);
            }
            if let Some(p) = d.save_file() {
                say(status, ws.export_selection(&p));
            }
        }
    }
}

/// The blocks Fill, Replace and the brush put in: the palette (the one block chosen if it is empty).
fn blocks_chosen(map: &crate::map_view::MapView) -> Vec<mctk_java::BlockState> {
    if map.block_palette.is_empty() { vec![map.block.clone()] } else { map.block_palette.clone() }
}

/// The palette button: a palette icon that opens the blocks Fill, Replace and the brush put in,
/// mixed evenly (one block is just that block). Each block has its own properties (a click goes
/// to the next value) and Remove in a menu of its own; more are added from the list, air too (to
/// erase). The first block is the one the toolbar calls its own (`block`).
fn palette_button(ui: &mut Ui, map: &mut crate::map_view::MapView) {
    let k = theme::ui_scale();
    let n = map.block_palette.len();
    let chip = widgets::button(ui, Some(Icon::Palette), "", 36.0 * k, true).on_hover_text(format!("The blocks put in, mixed evenly ({n}): choose them, set their properties, add more (air erases)"));
    let palette = &mut map.block_palette;
    ctx_menu::dropdown(ui, &chip, |ui, cx| {
        ctx_menu::widen(ui, 280.0);
        ui.label(egui::RichText::new("Mixed evenly").color(theme::text_muted()));
        let last = palette.len() <= 1;
        let mut remove = None;
        for (i, s) in palette.iter_mut().enumerate() {
            let name = crate::item_picker::display_name(&s.name);
            let id = s.name.clone();
            let icon = move |p: &egui::Painter, r: Rect, ink: egui::Color32| {
                if !crate::icons::item_icon(p, r, &id, egui::Color32::WHITE) {
                    crate::icons::paint(p, r, Icon::Rename, ink);
                }
            };
            // (Taken apart so the block can be changed once the menu is done with it.)
            let props = crate::edit::states::properties(&s.name);
            let current = s.properties.clone();
            let act = ctx_menu::submenu_painted(ui, cx, icon, &name, 260.0, 60.0 + props.len() as f32 * 32.0, |ui| {
                let mut act: Option<(Option<(String, String)>, bool)> = None;
                // Each property: its value now; a click makes it the next one.
                for (key, values) in props {
                    let now = current.get(key).cloned().unwrap_or_default();
                    let next = values.iter().position(|v| *v == now).map_or(0, |at| (at + 1) % values.len());
                    let turn = |p: &egui::Painter, r: Rect, ink: egui::Color32| crate::icons::paint(p, r, Icon::Rename, ink);
                    if ctx_menu::painted_keep(ui, &format!("{}: {now}", capitalized(key)), "", true, turn) {
                        act = Some((Some((key.clone(), values[next].clone())), false));
                    }
                }
                if !props.is_empty() {
                    ctx_menu::separator(ui);
                }
                if ctx_menu::item(ui, Icon::Trash, "Remove from the palette", "", !last) {
                    act = Some((None, true));
                }
                act
            });
            match act {
                Some((Some((key, value)), _)) => {
                    s.properties.insert(key, value);
                }
                Some((None, true)) => remove = Some(i),
                _ => {}
            }
        }
        if let Some(i) = remove {
            palette.remove(i);
        }
        ctx_menu::separator(ui);
        let air = |p: &egui::Painter, r: Rect, ink: egui::Color32| crate::icons::paint(p, r, Icon::Plus, ink);
        if ctx_menu::painted_keep(ui, "Add Air (erases)", "", true, air) {
            palette.push(mctk_java::BlockState::air());
        }
        let got = ctx_menu::submenu_wide(ui, cx, Icon::Plus, "Add a block", crate::item_picker::WIDTH, crate::item_picker::HEIGHT, |ui| crate::item_picker::show_blocks(ui, ""));
        if let Some(id) = got {
            palette.push(crate::edit::states::default_state(&id));
        }
    });
    // The first block stays the toolbar's own block.
    if let Some(first) = map.block_palette.first() {
        map.block = first.clone();
    }
}

/// The Place tool's button: the one list of everything that can be put down (blocks, items and
/// mobs); the picture is of what is picked.
fn place_button(ui: &mut Ui, m: &mut crate::map_view::PlaceMode) {
    let k = theme::ui_scale();
    let shown = crate::item_picker::display_name(m.id());
    let (r, pick) = ui.allocate_exact_size(vec2(40.0 * k, 30.0 * k), egui::Sense::click());
    ui.painter().rect_filled(r, theme::radius(4), if pick.hovered() { theme::button_hover() } else { theme::button() });
    let icon = Rect::from_center_size(r.center(), Vec2::splat(22.0 * k));
    // (A mob by its spawn egg.)
    let picture = if m.pick.starts_with("mob:") { format!("{}_spawn_egg", m.id()) } else { m.id().to_string() };
    if !crate::icons::item_icon(ui.painter(), icon, &picture, egui::Color32::WHITE) {
        let font = theme::regular(10.0 * k);
        let short = widgets::elide(ui, &shown, &font, r.width() - 4.0);
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, short, font, theme::text());
    }
    let pick = pick.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("{shown}: what is put down (a block, an armor stand or item frame, a mob...): click to choose"));
    let mut picked: Option<String> = None;
    ctx_menu::dropdown(ui, &pick, |ui, _| {
        picked = crate::item_picker::show_placeable(ui, &m.pick);
    });
    if let Some(key) = picked {
        m.pick = key;
    }
}
/// The biome (a Java id) at the block under the pointer, read when the pointer moves to another block or the world changes.
fn biome_under_pointer(ui: &Ui, ws: &Workspace) -> Option<String> {
    let at = ws.map.brush_at?;
    let generation = ws.map.edits.read().map_or(0, |l| l.generation);
    let id = egui::Id::new("biome-under-pointer");
    if let Some((k, g, v)) = ui.ctx().data(|d| d.get_temp::<([i32; 3], u64, Option<String>)>(id)) {
        if k == at && g == generation {
            return v;
        }
    }
    let v = ws.biome_at(at);
    ui.ctx().data_mut(|d| d.insert_temp(id, (at, generation, v.clone())));
    v
}

/// The biome tool's settings: the biome to paint (a list to search, with the one under the pointer shown beside it), the size and shape,
/// and the heights it reaches.
fn biome_row(ui: &mut Ui, biome: &mut crate::map_view::BiomeBrush, here: Option<&str>, roomy: bool) {
    let shown = crate::item_picker::display_name(&biome.id);
    let pick = widgets::button(ui, Some(Icon::Biome), &shown, 0.0, true).on_hover_text("The biome to paint: click to choose (search by name)");
    let mut picked: Option<String> = None;
    ctx_menu::dropdown(ui, &pick, |ui, _| {
        picked = crate::item_picker::show_biomes(ui, Some(&biome.id));
    });
    if let Some(id) = picked {
        biome.id = id;
    }
    if roomy {
        let here = here.map_or_else(|| "-".to_string(), crate::item_picker::display_name);
        muted(ui, format!("Here: {here}"));
    }
    ui.separator();
    // The size in cells of 4 blocks, along each way: x, y (height) and z.
    for (i, name) in ["X", "Y", "Z"].into_iter().enumerate() {
        muted(ui, name);
        let max = if i == 1 { 96 } else { 64 };
        let tip = format!("How many cells of 4 blocks the brush is along {}", ["x", "y (height)", "z"][i]);
        if ui.add(egui::DragValue::new(&mut biome.size[i]).range(1..=max).suffix(" cells")).on_hover_text(tip).changed() && i == 1 {
            biome.full = false;
        }
    }
    if ui.selectable_label(biome.full, "Full height").on_hover_text("Every height: from the bottom of the world to the top (the Y size follows)").clicked() {
        biome.full = !biome.full;
        if biome.full {
            biome.size[1] = 96;
        } else {
            biome.size[1] = 1;
        }
    }    ui.separator();
    if button(ui, Icon::Check, "", true, false, "Done (Esc)").clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("biome-done"), true));
    }
    if roomy {
        muted(ui, "Click paints the box shown (cells of 4 x 4 x 4 blocks) \u{b7} Alt+drag paints along");
    }
}

fn muted(ui: &mut Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text.into()).color(theme::text_muted()));
}

/// The settings of a lingering potion cloud about to be placed (or of the one selected): the
/// effects button at the left (the water bottle; its menu has a card for each effect, with its
/// strength and time or for ever), then the cloud's size, how long it lingers (or for ever), the
/// color of its particles and whether they show. Gives whether the button at the end (Done, or
/// Deselect) was clicked.
fn cloud_row(ui: &mut Ui, cloud: &mut crate::cloud::Cloud, roomy: bool, editing: bool) -> bool {
    use crate::cloud::CloudEffect;
    let k = theme::ui_scale();
    // The effects: a bottle of water from the pack, like the block button of the other tools.
    let (r, bottle) = ui.allocate_exact_size(vec2(40.0 * k, 30.0 * k), egui::Sense::click());
    ui.painter().rect_filled(r, theme::radius(4), if bottle.hovered() { theme::button_hover() } else { theme::button() });
    if !crate::icons::item_icon(ui.painter(), Rect::from_center_size(r.center(), Vec2::splat(22.0 * k)), "minecraft:potion", egui::Color32::WHITE) {
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, cloud.effects.len().to_string(), theme::regular(13.0 * k), theme::text());
    }
    let bottle = bottle.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("Effects ({}): what the cloud does to whoever stands in it, each with its strength and time", cloud.effects.len()));
    ctx_menu::dropdown(ui, &bottle, |ui, cx| {
        ctx_menu::widen(ui, 300.0);
        // Every card as wide as the menu: its own width less the card's margin and edge.
        let width = ui.available_width() - 18.0;
        let mut remove = None;
        for (i, e) in cloud.effects.iter_mut().enumerate() {
            egui::Frame::new().fill(theme::field()).stroke(egui::Stroke::new(1.0_f32, theme::border())).corner_radius(theme::radius(6)).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                ui.set_width(width);
                // The effect's picture and name, and the icon that removes it.
                ui.horizontal(|ui| {
                    let (pic, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
                    if let Some(f) = crate::effects::icon_file(&e.id) {
                        crate::icons::file_image(ui.painter(), pic, &f);
                    }
                    ui.label(egui::RichText::new(crate::item_picker::display_name(&e.id)).color(theme::text()).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::button(ui, Some(Icon::Trash), "", 28.0 * k, true).on_hover_text("Remove this effect").clicked() {
                            remove = Some(i);
                        }
                    });
                });
                // Its strength and time, side by side.
                ui.horizontal(|ui| {
                    muted(ui, "Level");
                    ui.add(egui::DragValue::new(&mut e.level).range(1..=255)).on_hover_text("Strength: I is level 1, II is level 2...");
                    muted(ui, "Time");
                    ui.add_enabled(!e.forever, egui::DragValue::new(&mut e.seconds).range(1..=1_000_000).suffix(" s")).on_hover_text("How long the effect lasts on whoever it touches");
                    ui.checkbox(&mut e.forever, "Forever").on_hover_text("It never wears off");
                });
            });
            ui.add_space(4.0);
        }
        if let Some(i) = remove {
            cloud.effects.remove(i);
        }
        let got = ctx_menu::submenu_painted(ui, cx, |p, r, ink| crate::icons::paint(p, r, Icon::Plus, ink), "Add an effect", 260.0, 340.0, |ui| {
            let mut pick = None;
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                for id in crate::effects::cloud_effect_names() {
                    let file = crate::effects::icon_file(id);
                    let icon = move |p: &egui::Painter, r: Rect, _: egui::Color32| {
                        if let Some(f) = &file {
                            crate::icons::file_image(p, r, f);
                        }
                    };
                    if ctx_menu::painted(ui, &crate::item_picker::display_name(id), "", true, icon) {
                        pick = Some((*id).to_string());
                    }
                }
            });
            pick
        });
        if let Some(id) = got {
            cloud.effects.push(CloudEffect::new(&id));
        }
    });
    ui.separator();
    ui.label(egui::RichText::new(if editing { "Potion Cloud (selected)" } else { "Potion Cloud" }).color(theme::text()));
    muted(ui, "Radius");
    ui.add(egui::DragValue::new(&mut cloud.radius).range(0.5..=32.0).speed(0.1).fixed_decimals(1)).on_hover_text("Blocks from the middle to the edge (a thrown potion's is 3)");
    muted(ui, "Lasts");
    ui.add_enabled(!cloud.forever, egui::DragValue::new(&mut cloud.lasts).range(1..=36_000).suffix(" s")).on_hover_text("How long the cloud lingers");
    ui.checkbox(&mut cloud.forever, "Forever").on_hover_text("The cloud never goes");
    // The particles' color: the effects' own mix, or one chosen.
    let mut custom = cloud.color.is_some();
    if ui.checkbox(&mut custom, "Color").on_hover_text("Color the particles yourself, instead of by the effects").changed() {
        cloud.color = custom.then(|| cloud.color());
    }
    if let Some(rgb) = cloud.color.as_mut() {
        ui.color_edit_button_srgb(rgb);
    }
    ui.checkbox(&mut cloud.particles, "Particles").on_hover_text("Show the cloud's particles in the game (the effects work either way; the editor always shows them)");
    ui.checkbox(&mut cloud.shrinks, "Shrinks").on_hover_text("As a thrown potion's cloud does, as it ages and is used");
    let done = button(ui, Icon::Check, "", true, false, if editing { "Deselect (Esc)" } else { "Done (Esc)" }).clicked();
    if roomy {
        muted(ui, if editing { "Changes are made as you set them" } else { "Click a block to place it" });
    }
    done
}
/// The settings of the Place tool: how many, for an item (what is put down is the pick at the
/// left, from the one list). Gives whether Done was clicked.
fn place_row(ui: &mut Ui, m: &mut crate::map_view::PlaceMode, roomy: bool) -> bool {
    use crate::map_view::PlaceKind;
    ui.label(egui::RichText::new("Place").color(theme::text()));
    match m.kind() {
        PlaceKind::Mob if roomy => muted(ui, "(Java worlds)"),
        _ => {}
    }
    let done = button(ui, Icon::Check, "", true, false, "Done (Esc)").clicked();
    if roomy {
        muted(ui, "Click a block to put it on top");
    }
    done
}
/// The settings of what is being done: the mob selected, the paste, the brush, the selection;
/// else a hint. The block chooser first (Fill, Replace and the brush use it). Ends with how
/// many chunks wait to be saved.
fn context_row(ui: &mut Ui, ws: &mut Workspace, roomy: bool, status: &mut String) {
    // What is put in sits at the left, in the same place for every tool: the Place tool's pick (the
    // one list of blocks, items and mobs), or the palette (Fill and Replace with a selection, and
    // the brush). The cloud's effects button takes the place in its own row.
    if let Some(m) = ws.map.place.as_mut() {
        place_button(ui, m);
        ui.separator();
    } else if ws.map.cloud.is_none() && (ws.map.brush.as_ref().is_some_and(|b| b.biome.is_none()) || (ws.map.brush.is_none() && ws.map.sel_box.is_some())) {
        palette_button(ui, &mut ws.map);
        ui.separator();
    }
    // The biome under the pointer (the biome tool shows it), as the world is now.
    let biome_now = if ws.map.brush.as_ref().is_some_and(|b| b.biome.is_some()) { biome_under_pointer(ui, ws) } else { None };
    let map = &mut ws.map;
    let mut run_tool: Option<EditTool> = None;
    if let Some(m) = map.cloud.as_mut() {
        if cloud_row(ui, &mut m.cloud, roomy, false) {
            map.cloud = None;
        }
    } else if let Some(m) = map.place.as_mut() {
        if place_row(ui, m, roomy) {
            map.place = None;
        }
    } else if let Some((e, pos)) = map.sel_entity.clone().and_then(|e| crate::map_view::tag_pos(&e).map(|p| (e, p))).filter(|(e, _)| crate::cloud::is_cloud(e)) {
        // A cloud selected: its settings, changed as they are set (as the ones for a new cloud).
        let key = [(pos[0] * 1000.0) as i64, (pos[1] * 1000.0) as i64, (pos[2] * 1000.0) as i64];
        if map.cloud_edit.as_ref().map(|c| c.0) != Some(key) {
            map.cloud_edit = crate::cloud::Cloud::from_entity(&e).map(|c| (key, c, false));
        }
        let mut deselect = false;
        if let Some((_, working, edited)) = map.cloud_edit.as_mut() {
            let before = working.clone();
            deselect = cloud_row(ui, working, roomy, true);
            if *working != before {
                run_tool = Some(EditTool::UpdateCloud { at: pos, cloud: working.clone(), stroke: *edited });
                *edited = true;
            }
        }
        if deselect {
            map.sel_entity = None;
            map.cloud_edit = None;
        }
    } else if let Some(e) = map.sel_entity.clone() {
        let c = e.as_compound().cloned().unwrap_or_default();
        let id = mctk_convert::view::java_entity_id(&c).unwrap_or_default();
        let name = crate::naming::mob_name(&c);
        let kind = crate::item_picker::display_name(&id);
        let pos: Vec<f64> = c.get("Pos").and_then(mctk_nbt::Tag::as_list).map(|l| l.iter().filter_map(mctk_nbt::Tag::as_f64).collect()).unwrap_or_default();
        ui.label(egui::RichText::new(match &name { Some(n) => format!("{n} ({kind})"), None => kind }).color(theme::text()));
        if pos.len() == 3 && roomy {
            muted(ui, format!("at {:.1}, {:.1}, {:.1}", pos[0], pos[1], pos[2]));
        }
        if ui.button("Delete").on_hover_text("Remove it (Undo brings it back)").clicked() && pos.len() == 3 {
            let mount = c.get("MctkMount").and_then(mctk_nbt::Tag::as_list).map(|l| l.iter().filter_map(mctk_nbt::Tag::as_f64).collect::<Vec<_>>()).filter(|v| v.len() == 3);
            run_tool = Some(match mount {
                Some(m) => {
                    let path: Vec<usize> = match c.get("MctkPath") {
                        Some(mctk_nbt::Tag::IntArray(p)) => p.iter().map(|v| *v as usize).collect(),
                        _ => Vec::new(),
                    };
                    let root = c.get("MctkRoot").and_then(mctk_nbt::Tag::as_list).map(|l| l.iter().filter_map(mctk_nbt::Tag::as_f64).collect::<Vec<_>>()).filter(|v| v.len() == 3).unwrap_or_else(|| m.clone());
                    match path.split_last() {
                        Some((last, parent)) => EditTool::Unmount { mount: [root[0], root[1], root[2]], path: parent.to_vec(), seat: Some(*last), delete: true },
                        None => EditTool::DeleteEntity([pos[0], pos[1], pos[2]]),
                    }
                }
                None => EditTool::DeleteEntity([pos[0], pos[1], pos[2]]),
            });
            map.sel_entity = None;
        }
        if ui.button("Deselect").clicked() {
            map.sel_entity = None;
        }
    } else if let (Some(p), Some(size)) = (&mut map.paste, map.clipboard.as_ref().map(|c| c.size)) {
        let s = p.place.size(size);
        ui.label(egui::RichText::new(format!("Pasting {} \u{d7} {} \u{d7} {}", s[0], s[1], s[2])).color(theme::text()));
        if ui.button(format!("Turn {}\u{b0}", p.place.turns as u32 * 90)).on_hover_text("R").clicked() {
            p.place.turns = (p.place.turns + 1) % 4;
        }
        if ui.selectable_label(p.place.mirror, "Mirror").on_hover_text("M").clicked() {
            p.place.mirror = !p.place.mirror;
        }
        ui.checkbox(&mut p.with_air, "Paste Air").on_hover_text("Also clear what is where the copy has air");
        if ui.button("Done").on_hover_text("Esc").clicked() {
            map.paste = None;
        }
        if roomy {
            muted(ui, "Click to place \u{b7} arrows, Page Up/Down nudge");
        }
    } else if let Some((b, biome)) = map.brush.as_mut().and_then(|b| b.biome.take().map(|bio| (b, bio))) {
        let mut biome = biome;
        biome_row(ui, &mut biome, biome_now.as_deref(), roomy);
        if b.biome.is_none() {
            // (Done: the tool ends, so the biome is not put back.)
        }
        let done = ui.ctx().data_mut(|d| d.remove_temp::<bool>(egui::Id::new("biome-done"))).unwrap_or(false);
        if done {
            map.brush = None;
        } else if let Some(b) = map.brush.as_mut() {
            b.biome = Some(biome);
        }
    } else if let Some(b) = &mut map.brush {
        muted(ui, "Size");
        ui.add(egui::DragValue::new(&mut b.radius).range(0..=32)).on_hover_text("Blocks from the middle to the edge");
        // Its shape, and where it is centered: pictures, with what each is on hover.
        if button(ui, Icon::BrushBall, "", true, b.sphere, "Ball: paint a ball around the block pointed at").clicked() {
            b.sphere = true;
        }
        if button(ui, Icon::BrushCube, "", true, !b.sphere, "Cube: paint a cube around the block pointed at").clicked() {
            b.sphere = false;
        }
        if button(ui, Icon::BrushChunk, "", true, b.snap, "Chunk center: paint around the middle of the chunk pointed at").clicked() {
            b.snap = !b.snap;
        }
        ui.separator();
        // What it paints over.
        use crate::edit::tools::BrushOnly;
        for (only, icon, tip) in [
            (BrushOnly::Anywhere, Icon::PaintAnywhere, "Build anywhere: paint every block in the shape, air too (it adds blocks where there is nothing)"),
            (BrushOnly::Terrain, Icon::PaintExisting, "Existing blocks only: change only blocks that are already there (not air, water or lava): it re-skins land and adds nothing to the air"),
            (BrushOnly::TerrainBelow, Icon::PaintBelow, "Below the pointer only: like Existing blocks, but only at and below the block pointed at, leaving what is above it"),
        ] {
            if button(ui, icon, "", true, b.only == only, tip).clicked() {
                b.only = only;
            }
        }
        ui.separator();
        if button(ui, Icon::Check, "", true, false, "Done (Esc)").clicked() {
            map.brush = None;
        }
        if roomy {
            muted(ui, "Click paints \u{b7} Alt+drag paints along");
        }    } else if let Some((lo, hi)) = map.sel_box {
        let size = [hi[0] - lo[0] + 1, hi[1] - lo[1] + 1, hi[2] - lo[2] + 1];
        let title = if size == [1, 1, 1] {
            format!("{} at {}, {}, {}", map.block_name_at(lo).unwrap_or_else(|| "Block".into()), lo[0], lo[1], lo[2])
        } else {
            format!("{} \u{d7} {} \u{d7} {}", size[0], size[1], size[2])
        };
        ui.label(egui::RichText::new(title).color(theme::text()));
        let (mut y0, mut y1) = (lo[1], hi[1]);
        muted(ui, "Y");
        let a = ui.add(egui::DragValue::new(&mut y0).range(-64..=y1)).changed();
        muted(ui, "to");
        let b = ui.add(egui::DragValue::new(&mut y1).range(y0..=320)).changed();
        if a || b {
            map.sel_box = Some(([lo[0], y0, lo[2]], [hi[0], y1, hi[2]]));
            map.box_changed();
        }
        if ui.button("Full height").on_hover_text("From the bottom of the world to the top").clicked() {
            map.sel_box = Some(([lo[0], -64, lo[2]], [hi[0], 319, hi[2]]));
            map.box_changed();
        }
        if ui.button("Clear").on_hover_text("Esc").clicked() {
            map.sel_box = None;
            map.box_changed();
        }
        if let Some(h) = map.box_hint() {
            muted(ui, h);
        }
    } else {
        muted(ui, map.box_hint().unwrap_or("Click a block or mob to select it \u{b7} Box Select for an area"));
    }
    if let Some(t) = run_tool {
        // (Changes to a cloud come as its settings are dragged: only what went wrong is said.)
        let quiet = matches!(t, EditTool::UpdateCloud { .. });
        match ws.run_edit(t) {
            Ok(_) if quiet => {}
            Ok(m) | Err(m) => *status = m,
        }
    }
}

/// Requests the view makes (a paste placed, the brush clicked, a mob's Delete...) run here.
fn run_view_requests(ws: &mut Workspace, status: &mut String) {
    let mut tool = None;
    // A box being moved (not a copy pasted): it is cleared where it was as it is placed.
    let mut box_moved = None;
    if let Some((origin, place)) = ws.map.paste_request.take() {
        let with_air = ws.map.paste.as_ref().is_some_and(|p| p.with_air);
        tool = Some(match ws.map.paste.as_ref().and_then(|p| p.move_from) {
            Some(from) => {
                box_moved = Some((origin, place));
                EditTool::MoveBox { from, origin, place, with_air: true }
            }
            None => EditTool::Paste { origin, place, with_air },
        });
    }
    if let Some(p) = ws.map.entity_delete_request.take() {
        tool = Some(EditTool::DeleteEntity(p));
    }
    if let Some((center, stroke)) = ws.map.brush_request.take() {
        if let Some(b) = &ws.map.brush {
            tool = Some(match &b.biome {
                Some(bio) => {
                    let (lo, hi) = crate::edit::tools::biome_box(center, bio.size, bio.full);
                    EditTool::BiomeBrush { lo, hi, biome: bio.id.clone(), stroke }
                }
                None => EditTool::Brush { center, radius: b.radius, sphere: b.sphere, palette: blocks_chosen(&ws.map), only: b.only, stroke },
            });
        }
    }
    if let Some((at, cloud)) = ws.map.cloud_request.take() {
        tool = Some(EditTool::PlaceCloud { at, cloud });
    }
    if let Some((mount, path, rider)) = ws.map.ride_request.take() {
        tool = Some(EditTool::MountMob { mount, path, rider });
    }
    // One block, mob or item put down by the Place tool, standing in the block clicked.
    if let Some((kind, at)) = ws.map.place_request.take() {
        use crate::map_view::PlaceKind;
        let m = ws.map.place.clone().unwrap_or_default();
        let stands = [at[0] as f64 + 0.5, at[1] as f64, at[2] as f64 + 0.5];
        tool = Some(match kind {
            PlaceKind::Block => EditTool::SetBlock(at, crate::edit::states::default_state(m.id())),
            PlaceKind::Mob => EditTool::PlaceMob { at: stands, id: m.id().to_string() },
            PlaceKind::Entity => EditTool::PlaceEntity { cell: at, id: m.id().to_string(), facing: m.facing },
        });
    }
    // What was put down after Move: the mob or block goes there, and the selection with it.
    let mut moved = None;
    if let Some(o) = ws.map.move_request.take() {
        use crate::map_view::MoveOrder;
        tool = Some(match &o {
            MoveOrder::Entity { from, to } => EditTool::MoveEntity(*from, *to),
            MoveOrder::Block { from, to } => EditTool::MoveBlock(*from, *to),
        });
        moved = Some(o);
    }
    // The turn handle being dragged: the mob faces the pointer, the block the nearest way it can.
    // (A turn goes on every frame the handle moves: it says nothing, or it would fill the corner
    // with messages; only what went wrong is said.)
    let mut quiet = false;
    if let Some(o) = ws.map.rotate_request.take() {
        use crate::map_view::RotateOrder;
        quiet = true;
        tool = Some(match o {
            RotateOrder::Entity { at, yaw, stroke } => EditTool::SetYaw { at, yaw, stroke },
            RotateOrder::Block { at, key, value } => EditTool::BlockProps(at, vec![(key.to_string(), value)]),
            RotateOrder::Group { from, turns, stroke } => EditTool::TurnBox { from, turns, stroke },
        });
    }
    if let Some(t) = tool {
        let result = ws.run_edit(t);
        if let (Ok(_), Some(o)) = (&result, &moved) {
            ws.map.moved(o);
        }
        // A box that was moved is where it went, selected, and the moving is over.
        if let (Ok(_), Some((origin, place))) = (&result, box_moved) {
            if let Some(size) = ws.map.clipboard.as_ref().map(|c| place.size(c.size)) {
                ws.map.sel_box = Some((origin, [origin[0] + size[0] - 1, origin[1] + size[1] - 1, origin[2] + size[2] - 1]));
                ws.map.box_changed();
            }
            ws.map.paste = None;
        }
        match result {
            Ok(_) if quiet => {}
            Ok(m) | Err(m) => *status = m,
        }
    }
}

/// `Beacon ▸`: its primary and secondary power (any effect). The same effect in both is level II: the game works that
/// out itself, so there is nothing more to set. Shared by the World Explorer's menu and the NBT tree's. Gives the change.
pub(crate) fn beacon_submenu(ui: &mut Ui, cx: &mut ctx_menu::Cx, c: &mctk_nbt::Compound) -> Option<crate::mob_edit::MobEdit> {
    use crate::mob_edit::MobEdit;
    let (primary, secondary) = crate::mob_edit::beacon_powers(c);
    ctx_menu::submenu_painted(ui, cx, ctx_menu::item_icon("beacon"), "Beacon", 260.0, 160.0, |ui| {
        let old = ctx_menu::set_row_width(ui.ctx(), 260.0);
        let mut inner = ctx_menu::Cx::default();
        let mut got = None;
        if let Some(effect) = beacon_power_menu(ui, &mut inner, false, &primary) {
            got = Some(MobEdit::BeaconPower { secondary: false, effect });
        }
        if let Some(effect) = beacon_power_menu(ui, &mut inner, true, &secondary) {
            got = Some(MobEdit::BeaconPower { secondary: true, effect });
        }
        // The game gives the first effect at level I; the same effect in both slots makes it level II.
        ctx_menu::separator(ui);
        // (Wrapped inside the menu's width: a long line would only make the menu wide.)
        ui.scope(|ui| {
            ui.set_max_width(236.0);
            ui.add(egui::Label::new(egui::RichText::new("Effects are level I. The same effect as both the primary and the secondary power is level II.").size(12.0).color(theme::text_dim())).wrap());
        });
        ctx_menu::set_row_width(ui.ctx(), old);
        got
    })
}
/// `Primary Power ▸` or `Secondary Power ▸` of a beacon: every effect there is, in one list, the one it has
/// ticked, and none. Gives the one chosen (`Some(None)`: none).
fn beacon_power_menu(ui: &mut Ui, cx: &mut ctx_menu::Cx, secondary: bool, now: &Option<String>) -> Option<Option<String>> {
    let shown = now.as_deref().map_or_else(|| "None".to_string(), crate::item_picker::display_name);
    let label = format!("{}: {shown}", if secondary { "Secondary Power" } else { "Primary Power" });
    let icon_of = now.clone();
    let icon = move |p: &egui::Painter, r: Rect, ink: egui::Color32| match &icon_of {
        Some(e) => crate::effects::paint_effect(p, r, e, true, ink),
        None => crate::icons::paint(p, r, Icon::Rename, ink),
    };
    ctx_menu::submenu_painted(ui, cx, icon, &label, 240.0, 380.0, |ui| {
        let old = ctx_menu::set_row_width(ui.ctx(), 240.0);
        let mut got = None;
        if ctx_menu::item(ui, Icon::Trash, "None", if now.is_none() { "\u{2713}" } else { "" }, true) && now.is_some() {
            got = Some(None);
        }
        ctx_menu::separator(ui);
        egui::ScrollArea::vertical().max_height(330.0).show(ui, |ui| {
            for effect in crate::effects::cloud_effect_names() {
                let on = now.as_deref() == Some(*effect);
                let id = (*effect).to_string();
                let clicked = ctx_menu::painted(ui, &crate::item_picker::display_name(effect), if on { "\u{2713}" } else { "" }, true, move |p, r, ink| {
                    crate::effects::paint_effect(p, r, &id, false, ink)
                });
                if clicked && !on {
                    got = Some(Some((*effect).to_string()));
                }
            }
        });
        ctx_menu::set_row_width(ui.ctx(), old);
        got
    })
}

/// `Occupation ▸` of a villager: each job with the block it works at, the one it has ticked.
pub fn profession_menu(ui: &mut Ui, cx: &mut ctx_menu::Cx, now: &str) -> Option<String> {
    let shown = crate::item_picker::display_name(now);
    let job_block = crate::mob_edit::PROFESSIONS.iter().find(|(n, _)| *n == now).map_or("barrier", |(_, b)| *b);
    ctx_menu::submenu_painted(
        ui,
        cx,
        ctx_menu::item_icon(job_block),
        &format!("Occupation: {shown}"),
        230.0,
        20.0 + crate::mob_edit::PROFESSIONS.len() as f32 * 26.0,
        |ui| {
            let old = ctx_menu::set_row_width(ui.ctx(), 230.0);
            let mut got = None;
            for (name, block) in crate::mob_edit::PROFESSIONS {
                let on = name == now;
                if ctx_menu::painted(ui, &crate::item_picker::display_name(name), if on { "\u{2713}" } else { "" }, true, ctx_menu::item_icon(block)) && !on {
                    got = Some(name.to_string());
                }
            }
            ctx_menu::set_row_width(ui.ctx(), old);
            got
        },
    )
}

/// `Lock ▸` of a container: the name of the key that opens it (an item named so), and unlocking it.
/// Gives the key typed (`""`: unlocked).
fn lock_menu(ui: &mut Ui, cx: &mut ctx_menu::Cx, key: &str) -> Option<String> {
    let label = if key.is_empty() { "Lock".to_string() } else { format!("Lock: {key}") };
    ctx_menu::submenu_painted(ui, cx, ctx_menu::item_icon("tripwire_hook"), &label, 300.0, 80.0, |ui| {
        let old = ctx_menu::set_row_width(ui.ctx(), 300.0);
        let mut got = None;
        if let Some(k) = ctx_menu::text_row(ui, Icon::Rename, "Key Name", key, "Name of the key item") {
            if k.trim() != key {
                got = Some(k.trim().to_string());
            }
        }
        if !key.is_empty() && ctx_menu::item(ui, Icon::Trash, "Unlock", "", true) {
            got = Some(String::new());
        }
        ctx_menu::set_row_width(ui.ctx(), old);
        got
    })
}

/// "waterlogged" as "Waterlogged".
fn capitalized(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or_else(String::new, |f| f.to_uppercase().collect::<String>() + c.as_str()).replace('_', " ")
}

/// The right-click menu in the blocks' views (Top-down and 3D), for the mob or block clicked:
/// turn it, waterlog it, make it something else, open its chunk, or delete it.
fn view_menu(ui: &Ui, ws: &mut Workspace, resp: &egui::Response, status: &mut String) {
    use crate::map_view::MenuTarget;
    if resp.secondary_clicked() {
        ws.map.menu_target = if ws.map.shows_blocks() { ws.map.pointed() } else { None };
        // The menu is for what is selected: a right-click selects what it is on (a block already
        // inside the box selected keeps the box).
        match ws.map.menu_target.clone() {
            Some(MenuTarget::Entity(e, _)) => {
                ws.map.sel_box = None;
                ws.map.sel_entity = Some(e);
            }
            Some(MenuTarget::Block(p, ..)) => {
                let inside = ws.map.sel_box.is_some_and(|(lo, hi)| (0..3).all(|k| lo[k] <= p[k] && p[k] <= hi[k]));
                if !inside || ws.map.sel_entity.is_some() {
                    ws.map.sel_entity = None;
                    ws.map.sel_box = Some((p, p));
                    ws.map.box_changed();
                }
            }
            None => {}
        }
    }
    let Some(target) = ws.map.menu_target.clone() else { return };
    let mut tool: Option<EditTool> = None;
    let mut open: Option<(i32, i32)> = None;
    let mut start_move: Option<crate::map_view::MoveWhat> = None;
    let mut start_rider: Option<std::sync::Arc<mctk_nbt::Tag>> = None;
    let mut move_box = false;
    // A container whose inventory is to be opened (its block's position).
    let mut edit_inventory: Option<[i32; 3]> = None;
    let mut edit_mob_inventory: Option<[f64; 3]> = None;
    let mut edit_mob_trades: Option<[f64; 3]> = None;
    // Only Java worlds say what rides what in a form we can write.
    let can_ride = ws.edition == crate::world_picker::UiEdition::Java;
    // (The consoles name entities their own old way: changing one's kind is left to its tab.)
    let can_retype = matches!(ws.edition, crate::world_picker::UiEdition::Java | crate::world_picker::UiEdition::Bedrock);
    ctx_menu::context(ui, resp, |ui, cx| {
        ctx_menu::widen(ui, 220.0);
        let title = |ui: &mut Ui, t: String| {
            ui.label(egui::RichText::new(t).color(theme::text_muted()));
        };
        match &target {
            // A lingering cloud is not a mob: it moves and goes, and its settings are in the toolbar.
            MenuTarget::Entity(e, pos) if crate::cloud::is_cloud(e) => {
                title(ui, "Potion cloud".into());
                if ctx_menu::item(ui, Icon::ArrowRight, "Move", "", true) {
                    start_move = Some(crate::map_view::MoveWhat::Entity(e.clone()));
                }
                if ctx_menu::item(ui, Icon::Folder, "Open Chunk", "", true) {
                    open = Some(((pos[0].floor() as i32).div_euclid(16), (pos[2].floor() as i32).div_euclid(16)));
                }
                ctx_menu::separator(ui);
                if ctx_menu::item(ui, Icon::MobRemove, "Delete", "Del", true) {
                    tool = Some(EditTool::DeleteEntity(*pos));
                }
            }
            MenuTarget::Entity(e, pos) => {
                let c = e.as_compound().cloned().unwrap_or_default();
                let id = mctk_convert::view::java_entity_id(&c).unwrap_or_default();
                let bare = id.trim_start_matches("minecraft:").to_string();
                let kind = crate::item_picker::display_name(&id);
                title(ui, crate::naming::mob_name(&c).map_or(kind.clone(), |n| format!("{n} ({kind})")));
                // A rider (seen on its mount) is changed through the mount: where the change goes.
                let on_mount = c.contains_key("MctkMount");
                // Where the mount at the bottom of its stack stands, and the seats that lead from it to
                // this rider (a rider of a rider, as deep as the stack goes, is edited the same way).
                let root = c.get("MctkRoot").and_then(mctk_nbt::Tag::as_list).map(|l| l.iter().filter_map(mctk_nbt::Tag::as_f64).collect::<Vec<_>>()).filter(|v| v.len() == 3).map(|m| [m[0], m[1], m[2]]);
                let seats: Vec<usize> = match c.get("MctkPath") {
                    Some(mctk_nbt::Tag::IntArray(p)) => p.iter().map(|v| *v as usize).collect(),
                    _ => Vec::new(),
                };
                let (at, rpath) = match (on_mount, root) {
                    (true, Some(r)) => (r, seats.clone()),
                    _ => (*pos, Vec::new()),
                };
                let editable = !on_mount || root.is_some();
                if on_mount {
                    if let Some((last, parent)) = rpath.split_last() {
                        if ctx_menu::item(ui, Icon::ArrowRight, "Unmount", "", true) {
                            tool = Some(EditTool::Unmount { mount: at, path: parent.to_vec(), seat: Some(*last), delete: false });
                        }
                    }
                } else if ctx_menu::item(ui, Icon::ArrowRight, "Move", "", true) {
                    start_move = Some(crate::map_view::MoveWhat::Entity(e.clone()));
                }
                if editable {
                    // Which way it faces, in degrees: one of the four ways, or straightened to the
                    // nearest of them when it looks off at an angle.
                    let yaw = e.get("Rotation").and_then(mctk_nbt::Tag::as_list).and_then(|l| l.get(0)).and_then(mctk_nbt::Tag::as_f64).unwrap_or(0.0) as f32;
                    let yaw = yaw.rem_euclid(360.0);
                    let facing = ctx_menu::submenu(ui, cx, Icon::Refresh, &format!("Facing: {}\u{b0}", yaw.round() as i32 % 360), |ui| {
                        let mut pick = None;
                        for (name, y) in [("North", 180.0f32), ("East", 270.0), ("South", 0.0), ("West", 90.0)] {
                            if ctx_menu::item(ui, Icon::ArrowRight, &format!("{name} ({}\u{b0})", y as i32), "", true) {
                                pick = Some(y);
                            }
                        }
                        let straight = ((yaw / 90.0).round() * 90.0).rem_euclid(360.0);
                        let gap = (yaw - straight).abs().min(360.0 - (yaw - straight).abs());
                        ctx_menu::separator(ui);
                        if ctx_menu::item(ui, Icon::Check, &format!("Straighten ({}\u{b0})", straight as i32), "", gap > 0.5) {
                            pick = Some(straight);
                        }
                        pick
                    });
                    if let Some(y) = facing {
                        tool = Some(if on_mount { EditTool::Change { at, path: rpath.clone(), change: crate::workspace::EntityChange::Yaw(y) } } else { EditTool::SetYaw { at: *pos, yaw: y, stroke: false } });
                    }
                    let java = ws.edition == crate::world_picker::UiEdition::Java;
                    // The rows the NBT tree's menu has for a mob (switches, equipment, drops, effects, a villager's job,
                    // trades...), under its name.
                    if java && crate::naming::is_mob(&c) {
                        let key = format!("{at:?}{rpath:?}");
                        // (A mob saved without them is shown with the tags a whole one has, so its slots can be edited.)
                        let mut whole = c.clone();
                        crate::edit::tools::mob_defaults(&mut whole, None);
                        if let Some((edits, trades)) = crate::nbt_view::mob_submenu(ui, cx, &whole, &key) {
                            for edit in edits {
                                tool = Some(EditTool::Change { at, path: rpath.clone(), change: crate::workspace::EntityChange::Mob(edit) });
                            }
                            if trades && !on_mount {
                                edit_mob_trades = Some(*pos);
                            }
                        }
                    }
                    // An armor stand: what is on each slot, and whether it shows.
                    if java && bare == "armor_stand" {
                        use crate::workspace::EntityChange as Ch;
                        ctx_menu::separator(ui);
                        let hidden = crate::mob_edit::invisible(&c, &id).unwrap_or(false);
                        if ctx_menu::item(ui, Icon::Check, if hidden { "Make visible" } else { "Make invisible" }, "", true) {
                            tool = Some(EditTool::Change { at, path: rpath.clone(), change: Ch::Invisible(!hidden) });
                        }
                        // Its pose, and the flags of the stand.
                        let now_pose = crate::edit::tools::stand_pose_name(&c);
                        let pose_label = format!("Pose: {}", now_pose.unwrap_or("Custom"));
                        let posed = ctx_menu::submenu_painted(ui, cx, |p, r, ink| crate::icons::paint(p, r, Icon::Refresh, ink), &pose_label, 200.0, 20.0 + crate::edit::tools::STAND_POSES.len() as f32 * 26.0, |ui| {
                            let old = ctx_menu::set_row_width(ui.ctx(), 200.0);
                            let mut pick = None;
                            // (The one it is in is ticked.)
                            for (name, _) in crate::edit::tools::STAND_POSES {
                                let on = now_pose == Some(*name);
                                if ctx_menu::painted(ui, name, if on { "\u{2713}" } else { "" }, true, |p, r, ink| crate::icons::paint(p, r, Icon::ArrowRight, ink)) && !on {
                                    pick = Some((*name).to_string());
                                }
                            }
                            ctx_menu::set_row_width(ui.ctx(), old);
                            pick
                        });
                        if let Some(name) = posed {
                            tool = Some(EditTool::Change { at, path: rpath.clone(), change: Ch::StandPose(name) });
                        }
                        for (key, label) in [("Small", "Small"), ("ShowArms", "Arms"), ("NoBasePlate", "No base plate")] {
                            let on = c.get(key).and_then(mctk_nbt::Tag::as_i64).unwrap_or(0) != 0;
                            if ctx_menu::item(ui, Icon::Check, &format!("{label}: {}", if on { "on" } else { "off" }), "", true) {
                                tool = Some(EditTool::Change { at, path: rpath.clone(), change: Ch::Flag(key.to_string(), !on) });
                            }
                        }
                    }
                    // A mob's own inventory (a villager's, a donkey's chest): the game's inventory screen.
                    if !on_mount && (c.contains_key("Inventory") || c.contains_key("Items")) && bare != "armor_stand" && ctx_menu::painted(ui, "Edit Inventory", "", true, ctx_menu::item_icon("chest")) {
                        edit_mob_inventory = Some(*pos);
                    }
                    // An item frame: what is in it, and which way up.
                    if java && bare.ends_with("item_frame") {
                        use crate::workspace::EntityChange as Ch;
                        ctx_menu::separator(ui);
                        let now = c.get("Item").and_then(mctk_nbt::Tag::as_compound).and_then(crate::mob_edit::stack_of).map(|(i, _)| i);
                        let label = format!("Item: {}", now.as_ref().map_or("empty".to_string(), |i| crate::item_picker::display_name(i)));
                        let got = ctx_menu::submenu_wide(ui, cx, Icon::Rename, &label, crate::item_picker::WIDTH, crate::item_picker::HEIGHT, |ui| crate::item_picker::show(ui, now.as_deref().unwrap_or(""), false));
                        if let Some(item) = got {
                            tool = Some(EditTool::Change { at, path: rpath.clone(), change: Ch::FrameItem(Some(item)) });
                        }
                        if ctx_menu::item(ui, Icon::Refresh, "Turn the item", "", now.is_some()) {
                            tool = Some(EditTool::Change { at, path: rpath.clone(), change: Ch::FrameTurn });
                        }
                        if ctx_menu::item(ui, Icon::Trash, "Take the item out", "", now.is_some()) {
                            tool = Some(EditTool::Change { at, path: rpath.clone(), change: Ch::FrameItem(None) });
                        }
                        let hidden = crate::mob_edit::invisible(&c, &id).unwrap_or(false);
                        if ctx_menu::item(ui, Icon::Check, if hidden { "Make frame visible" } else { "Make frame invisible" }, "", true) {
                            tool = Some(EditTool::Change { at, path: rpath.clone(), change: Ch::Invisible(!hidden) });
                        }
                        let fixed = c.get("Fixed").and_then(mctk_nbt::Tag::as_i64).unwrap_or(0) != 0;
                        if ctx_menu::item(ui, Icon::Check, &format!("Fixed (can't be turned or emptied): {}", if fixed { "on" } else { "off" }), "", true) {
                            tool = Some(EditTool::Change { at, path: rpath.clone(), change: Ch::Flag("Fixed".into(), !fixed) });
                        }
                        ctx_menu::separator(ui);
                    }
                    let riders = c.get("Passengers").and_then(mctk_nbt::Tag::as_list).map_or(0, |l| l.len());
                    if can_ride && riders > 0 && ctx_menu::item(ui, Icon::ArrowRight, if riders > 1 { "Unmount riders" } else { "Unmount rider" }, "", true) {
                        tool = Some(EditTool::Unmount { mount: at, path: rpath.clone(), seat: None, delete: false });
                    }
                    // (A seat holds one rider: two in a boat or on a camel.) A rider can carry one too.
                    if can_ride && riders < crate::gpu_world::seats(&id) {
                        // A new mob of the kind picked, riding this one; or (first in the list) one
                        // already in the world.
                        let mut pick_one = false;
                        let got = ctx_menu::submenu_wide(ui, cx, Icon::Plus, "Add rider", crate::item_picker::WIDTH, crate::item_picker::HEIGHT + 40.0, |ui| {
                            // (The header is as wide as the list under it.)
                            let old = ctx_menu::set_row_width(ui.ctx(), crate::item_picker::WIDTH);
                            if ctx_menu::item(ui, Icon::ArrowRight, "Pick a rider in the world", "", true) {
                                pick_one = true;
                            }
                            ctx_menu::separator(ui);
                            ctx_menu::set_row_width(ui.ctx(), old);
                            crate::item_picker::show_mobs(ui, None)
                        });
                        if let Some(m) = got {
                            tool = Some(if on_mount {
                                EditTool::Change { at, path: rpath.clone(), change: crate::workspace::EntityChange::AddRider(m) }
                            } else {
                                EditTool::Ride(*pos, m)
                            });
                        }
                        if pick_one {
                            start_rider = Some(e.clone());
                        }
                    }
                    if can_retype && !bare.contains("frame") && bare != "armor_stand" && !bare.ends_with("minecart") && !bare.ends_with("boat") {
                        // Only mobs, by name (zombie to parrot...), not every item and block.
                        let got = ctx_menu::submenu_wide(ui, cx, Icon::Rename, "Change to", crate::item_picker::WIDTH, crate::item_picker::HEIGHT, |ui| crate::item_picker::show_mobs(ui, Some(&id)));
                        if let Some(m) = got {
                            tool = Some(if on_mount { EditTool::Change { at, path: rpath.clone(), change: crate::workspace::EntityChange::Retype(m) } } else { EditTool::RetypeEntity(*pos, m) });
                        }
                    }
                }
                if ctx_menu::item(ui, Icon::Folder, "Open Chunk", "", true) {
                    open = Some(((pos[0].floor() as i32).div_euclid(16), (pos[2].floor() as i32).div_euclid(16)));
                }
                if editable {
                    ctx_menu::separator(ui);
                    if ctx_menu::item(ui, Icon::MobRemove, "Delete", "Del", true) {
                        tool = Some(match rpath.split_last() { Some((last, parent)) if on_mount => EditTool::Unmount { mount: at, path: parent.to_vec(), seat: Some(*last), delete: true }, _ => EditTool::DeleteEntity(*pos) });
                    }
                }
            }
            MenuTarget::Block(p, name, props) => {
                title(ui, format!("{} at {}, {}, {}", crate::item_picker::display_name(name), p[0], p[1], p[2]));
                // (A block saved with no state is in its usual one: that is what is shown.)
                let full = crate::edit::states::full_properties(name, props);
                let prop = |k: &str| full.iter().find(|(pk, _)| pk == k).map(|(_, v)| v.clone());
                // One block follows the pointer; a box of several is placed like a paste (turn with
                // R, mirror with M, nudge with the arrows) and cleared from where it was.
                let single = ws.map.sel_box.is_none_or(|(lo, hi)| lo == hi);
                if ctx_menu::item(ui, Icon::ArrowRight, "Move", "", true) {
                    if single {
                        start_move = Some(crate::map_view::MoveWhat::Block(*p));
                    } else {
                        move_box = true;
                    }
                }
                // What the block holds in its block entity: a beacon's powers, a container's lock.
                let held = if single {
                    ws.map.block_entity_at(*p).as_ref().and_then(|b| b.as_compound().cloned()).or_else(|| {
                        // A beacon or container put down here has no block entity yet: an empty one is
                        // shown, and made when something is set (Java worlds).
                        if ws.edition != crate::world_picker::UiEdition::Java {
                            return None;
                        }
                        let bare = name.trim_start_matches("minecraft:");
                        let id = if bare.ends_with("shulker_box") { "shulker_box" } else { bare };
                        let mut be = mctk_nbt::Compound::new();
                        be.insert("id".into(), mctk_nbt::Tag::String(format!("minecraft:{id}")));
                        (crate::mob_edit::is_beacon(&be) || crate::mob_edit::lockable(&be)).then_some(be)
                    })
                } else {
                    None
                };
                if let Some(c) = &held {
                    use crate::mob_edit::MobEdit;
                    let mut any = false;
                    if crate::mob_edit::is_beacon(c) {
                        any = true;
                        if let Some(edit) = beacon_submenu(ui, cx, c) {
                            tool = Some(EditTool::BlockEntity(*p, edit));
                        }
                    }
                    // A container with items in it: its inventory, as the game's screen shows it.
                    if matches!(c.get("Items"), Some(mctk_nbt::Tag::List(_))) && ctx_menu::painted(ui, "Edit Inventory", "", true, ctx_menu::item_icon("chest")) {
                        edit_inventory = Some(*p);
                    }
                    if crate::mob_edit::lockable(c) {
                        any = true;
                        if let Some(key) = lock_menu(ui, cx, &crate::mob_edit::lock(c)) {
                            tool = Some(EditTool::BlockEntity(*p, MobEdit::Lock(key)));
                        }
                    }
                    if any {
                        ctx_menu::separator(ui);
                    }
                }
                // A group is turned as one (here, and by the handle under it).
                if !single {
                    let turn = ctx_menu::submenu(ui, cx, Icon::Refresh, "Turn", |ui| {
                        let mut pick = None;
                        for (label, n) in [("90\u{b0} clockwise", 1u8), ("180\u{b0}", 2), ("90\u{b0} counter-clockwise", 3)] {
                            if ctx_menu::item(ui, Icon::Refresh, label, "", true) {
                                pick = Some(n);
                            }
                        }
                        pick
                    });
                    if let (Some(n), Some(from)) = (turn, ws.map.sel_box) {
                        tool = Some(EditTool::TurnBox { from, turns: n, stroke: false });
                    }
                }
                // The block's states, one list of values each: what only commands change in the
                // game (a fence's sides, a stair's shape, a door's hinge...).
                let states = crate::edit::states::properties(name);
                if !states.is_empty() {
                    ctx_menu::separator(ui);
                }
                for (key, values) in states {
                    let now = prop(key);
                    let shown = now.as_deref().unwrap_or("?");
                    let label = format!("{}: {shown}", capitalized(key));
                    let got = ctx_menu::submenu(ui, cx, Icon::Rename, &label, |ui| {
                        let mut pick = None;
                        for v in values {
                            let current = now.as_deref() == Some(v.as_str());
                            let on = |p: &egui::Painter, r: Rect, ink: egui::Color32| {
                                if current {
                                    crate::icons::paint(p, r, Icon::Check, ink);
                                }
                            };
                            if ctx_menu::painted(ui, v, "", true, on) {
                                pick = Some((key.clone(), v.clone()));
                            }
                        }
                        pick
                    });
                    if let Some(kv) = got {
                        // (Of a group: every block of it that has the property.)
                        tool = Some(match ws.map.sel_box {
                            Some(bounds) if !single => EditTool::BoxProps { bounds, props: vec![kv] },
                            _ => EditTool::BlockProps(*p, vec![kv]),
                        });
                    }
                }
                if !states.is_empty() {
                    ctx_menu::separator(ui);
                }
                let got = ctx_menu::submenu(ui, cx, Icon::Rename, "Change to", |ui| crate::item_picker::show_blocks(ui, name));
                if let Some(b) = got {
                    tool = Some(EditTool::SetBlock(*p, crate::edit::states::default_state(&b)));
                }
                if ctx_menu::item(ui, Icon::Folder, "Open Chunk", "", true) {
                    open = Some((p[0].div_euclid(16), p[2].div_euclid(16)));
                }
                ctx_menu::separator(ui);
                if ctx_menu::item(ui, Icon::Trash, "Delete", "make it air", true) {
                    tool = Some(EditTool::SetBlock(*p, mctk_java::BlockState::air()));
                }
            }
        }
    });
    if let Some(w) = start_move {
        ws.map.start_move(w);
    }
    if let Some(m) = start_rider {
        ws.map.start_pick_rider(m);
    }
    if move_box {
        // (The copy it places is made now: Paste's own copy is replaced.)
        let from = ws.map.sel_box;
        match ws.copy_box(false) {
            Ok(_) => {
                ws.map.brush = None;
                ws.map.cloud = None;
                ws.map.moving = None;
                ws.map.box_mode = false;
                ws.map.paste = Some(crate::map_view::PasteMode { move_from: from, ..Default::default() });
                *status = "Click where to put it \u{b7} R turns, M mirrors, arrows and Page Up/Down nudge, Esc stops".into();
            }
            Err(m) => *status = m,
        }
    }
    if let Some(t) = tool {
        if matches!(t, EditTool::DeleteEntity(_)) {
            ws.map.sel_entity = None;
        }
        // A group that was turned from the menu is where it ended up, selected.
        if let (EditTool::TurnBox { from, turns, .. }, true) = (&t, true) {
            ws.map.sel_box = Some(crate::edit::tools::turned_box(from.0, from.1, *turns));
            ws.map.box_changed();
        }
        // The mob selected faces the way it was set to (so the next menu says so).
        if let EditTool::SetYaw { yaw, .. } = &t {
            ws.map.selected_turned(*yaw);
        }
        // The outline round what is selected follows what was changed (arms, a pose, a size...) at once.
        if let EditTool::Change { change, .. } = &t {
            if let Some(c) = ws.map.sel_entity.as_ref().and_then(|s| s.as_compound().cloned()) {
                let mut c = c;
                crate::edit::tools::apply_change(&mut c, change, None);
                ws.map.sel_entity = Some(std::sync::Arc::new(mctk_nbt::Tag::Compound(c)));
            }
        }
        // A rider taken off its mount stands beside it, and is picked up to be put somewhere.
        let freed = match &t {
            EditTool::Unmount { mount, delete: false, seat: Some(_), .. } => ws.map.sel_entity.as_ref().and_then(|s| s.as_compound().cloned()).map(|mut c| {
                for k in ["MctkMount", "MctkSeat", "MctkDepth", "MctkRoot", "MctkPath"] {
                    c.shift_remove(k);
                }
                crate::edit::tools::set_entity_pos(&mut c, [mount[0] + 1.5, mount[1], mount[2]]);
                c
            }),
            _ => None,
        };
        let said = ws.run_edit(t);
        if let (Some(c), Ok(_)) = (freed, &said) {
            let tag = std::sync::Arc::new(mctk_nbt::Tag::Compound(c));
            ws.map.sel_entity = Some(tag.clone());
            ws.map.start_move(crate::map_view::MoveWhat::Entity(tag));
        }
        *status = match said {
            Ok(m) | Err(m) => m,
        };
    }
    if let Some(c) = open {
        crate::editor::open_chunks(ws, &[c], status);
    }
    if let Some(p) = edit_mob_trades {
        crate::editor::open_chunks(ws, &[((p[0].floor() as i32).div_euclid(16), (p[2].floor() as i32).div_euclid(16))], status);
        let opened = ws.active.and_then(|i| ws.docs.get_mut(i)).is_some_and(|d| d.tree.open_mob_trades(p));
        if !opened {
            *status = "That villager's trades could not be opened".into();
        }
    }
    if let Some(p) = edit_mob_inventory {
        crate::editor::open_chunks(ws, &[((p[0].floor() as i32).div_euclid(16), (p[2].floor() as i32).div_euclid(16))], status);
        let opened = ws.active.and_then(|i| ws.docs.get_mut(i)).is_some_and(|d| d.tree.open_mob_inventory(p));
        if !opened {
            *status = "That mob's inventory could not be opened".into();
        }
    }
    // Its chunk opens (the changes made here saved first), then the inventory in it.
    if let Some(p) = edit_inventory {
        crate::editor::open_chunks(ws, &[(p[0].div_euclid(16), p[2].div_euclid(16))], status);
        let opened = ws.active.and_then(|i| ws.docs.get_mut(i)).is_some_and(|d| d.tree.open_block_inventory(p));
        if !opened {
            *status = "That block's inventory could not be opened".into();
        }
    }
}

/// The World Explorer: its toolbars and the map.
pub fn show(ui: &mut Ui, ws: &mut Workspace, status: &mut String, prune: &mut crate::pruner::PruneTools) {
    // The sky follows the biome the camera is in.
    ws.update_sky_biome();
    let full = ui.max_rect();
    let roomy = full.width() >= 1100.0;
    let row_h = 40.0;
    let mut acts: Vec<Act> = Vec::new();
    let close_w = 44.0;
    // Groups of tools on top, wrapping to more rows; the settings bar under them (with the
    // blocks shown).
    let groups = tools(ws);
    let avail = full.width() - 16.0 - close_w;
    let gap = 10.0;
    // Names as long as fit on one row, else short, else icons only (then wrapping).
    // A group: its buttons, 4 between each, and 4 of margin at each end.
    let group_w = |g: &Vec<Tool>, pick: &dyn Fn(&Tool) -> &'static str| -> f32 { g.iter().map(|t| button_width(ui, pick(t))).sum::<f32>() + 4.0 * (g.len() as f32 - 1.0) + 8.0 };
    let total = |pick: &dyn Fn(&Tool) -> &'static str| -> f32 { groups.iter().map(|g| group_w(g, pick) + gap).sum() };
    let level = if total(&|t| t.long) <= avail {
        0
    } else if total(&|t| t.short) <= avail {
        1
    } else {
        2
    };
    let text = |t: &Tool| -> &'static str {
        match level {
            0 => t.long,
            1 => t.short,
            _ => "",
        }
    };
    // Each group placed left to right, onto a new row when it does not fit.
    let mut placed: Vec<(usize, Rect)> = Vec::new();
    let (mut x, mut y) = (full.left() + 8.0, full.top() + 4.0);
    for (gi, g) in groups.iter().enumerate() {
        let w = group_w(g, &text);
        if x + w > full.left() + 8.0 + avail && x > full.left() + 8.0 {
            x = full.left() + 8.0;
            y += row_h;
        }
        placed.push((gi, Rect::from_min_size(pos2(x, y), vec2(w, row_h - 4.0))));
        x += w + gap;
    }
    let tools_bottom = y + row_h;
    let blocks = ws.map.shows_blocks();
    // The settings of what is being done wrap onto more rows when the window is narrow, like the
    // tools; the bar is as tall as they were last frame.
    let settings_id = egui::Id::new("explorer-settings-height");
    let settings_h: f32 = ui.ctx().data(|d| d.get_temp(settings_id)).unwrap_or(row_h).max(row_h);
    let bar = Rect::from_min_max(full.min, pos2(full.right(), tools_bottom + if blocks { settings_h } else { 0.0 }));
    ui.painter().rect_filled(bar, 0.0, theme::bg());
    for (gi, r) in placed {
        ui.painter().rect_filled(r, theme::radius(6), theme::deep());
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink2(vec2(4.0, 0.0))));
        child.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for t in &groups[gi] {
                if let Some(a) = tool_ui(ui, ws, t, text(t)) {
                    acts.push(a);
                }
            }
        });
    }
    // On the flat map: what is picked, beside the tools.
    if !blocks {
        let n = ws.map.selected.len();
        let label = match n {
            0 => "Zoom in to select blocks and mobs (and open their chunks); draw shapes on the left to prune chunks".to_string(),
            1 => "1 chunk selected".to_string(),
            n => format!("{n} chunks selected"),
        };
        let r = Rect::from_min_max(pos2(x, full.top() + 4.0), pos2(full.right() - close_w, full.top() + row_h));
        let font = theme::regular(13.0);
        let label = widgets::elide(ui, &label, &font, (r.width() - 8.0).max(0.0));
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, label, font, theme::text_muted());
    } else {
        let ctx_rect = Rect::from_min_max(pos2(full.left() + 8.0, tools_bottom + 3.0), pos2(full.right() - 8.0, tools_bottom + 400.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(ctx_rect));
        child.set_clip_rect(ctx_rect);
        child.horizontal_wrapped(|ui| context_row(ui, ws, roomy, status));
        // Its height: what the rows took, with room around them.
        let used = (child.min_rect().height() + 9.0).max(row_h);
        if (used - settings_h).abs() > 1.0 {
            ui.ctx().data_mut(|d| d.insert_temp(settings_id, used));
            ui.ctx().request_repaint();
        }
    }
    for a in acts {
        run(ws, a, status);
    }
    run_view_requests(ws, status);
    let cr = Rect::from_center_size(pos2(full.right() - 20.0, full.top() + row_h / 2.0), Vec2::splat(28.0));
    if crate::editor::close_button(ui, cr, ui.id().with("explorer-close")).clicked() {
        ws.show_explorer = false;
    }
    // (In a window shorter than the toolbar, which wraps onto more rows when narrow, the map
    // gets no room rather than a rect turned inside out.)
    let body = Rect::from_min_max(pos2(full.left(), bar.bottom().min(full.bottom())), full.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    // A Prune shape tool chosen (on the left) has the map's clicks; the shapes are drawn on the
    // flat map (only when there are any: they animate).
    ws.map.selectable = !prune.drawing_tool();
    // The flat map and Top-down stay north-up (as the Pruner's); only 3D turns, and its turn
    // unwinds on the way back down.
    ws.map.can_rotate = false;
    // (A drag that starts on a shape's dot is the dot's, not the map's.)
    let on_dot = ws.map.hide_hover && !ws.map.shows_blocks();
    let frame = ws.map.show_map(&mut child, prune.map_pans() && !on_dot);
    if !frame.street && !ws.map.shows_blocks() && prune.has_shapes() {
        prune.overlay(&mut child, ws, &frame);
    } else {
        ws.map.hide_hover = false;
    }
    view_menu(&child, ws, &frame.response, status);
    ws.map.show_chrome(&mut child, &frame);
}
