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
    } else if ws.map.brush.is_some() {
        Act::Brush
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
            t(Act::Select, Icon::ArrowRight, "Select", "Select", true, "Select: click a block, plant or mob in 3D (Shift+click adds blocks); Esc clears"),
            t(Act::BoxSelect, Icon::ShapeBox, "Box Select", "Box", true, "Box Select: click the first block, move for width and length and click, then move for the height and click (on the flat map: two corners, or Shift+drag)"),
        ],
        vec![
            t(Act::Fill, Icon::Plus, "Fill", "Fill", boxed, "Fill the selection with the block"),
            t(Act::Replace(String::new()), Icon::Refresh, "Replace\u{2026}", "Replace", boxed, "Replace: pick one of the blocks in the selection to turn into the block chosen below (stairs keep facing the same way...)"),
            t(Act::Brush, Icon::Rename, "Brush", "Brush", true, "Paint a ball or cube of the block by clicking (Alt+drag paints along); Esc stops"),
            t(Act::DeleteEntities, Icon::MobRemove, "Delete Entities", "Entities", boxed, "Remove the mobs, items, minecarts, frames... in the selection"),
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
        match m {
            Act::BoxSelect if !on => ws.map.box_mode = true,
            Act::Brush if !on => ws.map.brush = Some(Default::default()),
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
        Act::Brush | Act::Paste => set_mode(ws, &act),
        Act::OpenChunks => {
            let sel = ws.map.selected.clone();
            crate::editor::open_chunks(ws, &sel, status);
        }
        Act::Fill => say(status, ws.run_edit(EditTool::Fill(ws.map.block.clone()))),
        Act::Replace(from) => say(status, ws.run_edit(EditTool::Replace { from: vec![from], to: ws.map.block.clone(), keep: true })),
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

/// The block Fill, Replace and the brush put in, and its properties (each a button when
/// `roomy`, else all in one menu).
fn block_chooser(ui: &mut Ui, ws: &mut Workspace, roomy: bool) {
    // The block's own picture (its name on hover; the name itself where it has no picture).
    let shown = crate::item_picker::display_name(&ws.map.block.name);
    let k = theme::ui_scale();
    let (r, pick) = ui.allocate_exact_size(vec2(40.0 * k, 30.0 * k), egui::Sense::click());
    ui.painter().rect_filled(r, theme::radius(4), if pick.hovered() { theme::button_hover() } else { theme::button() });
    let icon = Rect::from_center_size(r.center(), Vec2::splat(22.0 * k));
    if !crate::icons::item_icon(ui.painter(), icon, &ws.map.block.name, egui::Color32::WHITE) {
        let font = theme::regular(10.0 * k);
        let short = widgets::elide(ui, &shown, &font, r.width() - 4.0);
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, short, font, theme::text());
    }
    let pick = pick.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("{shown}: the block Fill, Replace and the brush put in (click to choose)"));
    let mut picked: Option<String> = None;
    ctx_menu::dropdown(ui, &pick, |ui, _| {
        picked = crate::item_picker::show_blocks(ui, &ws.map.block.name);
    });
    if let Some(id) = picked {
        ws.map.block = crate::edit::states::default_state(&id);
    }
    let props = crate::edit::states::properties(&ws.map.block.name);
    if props.is_empty() {
        return;
    }
    let mut chosen: Option<(String, String)> = None;
    if roomy {
        for (k, values) in props {
            let now = ws.map.block.properties.get(k).cloned().unwrap_or_default();
            let chip = widgets::button(ui, None, &format!("{k}: {now}\u{2026}"), 0.0, true);
            ctx_menu::dropdown(ui, &chip, |ui, _| {
                for v in values {
                    if ctx_menu::item(ui, if *v == now { Icon::Check } else { Icon::Rename }, v, "", true) {
                        chosen = Some((k.clone(), v.clone()));
                    }
                }
            });
        }
    } else {
        let menu = widgets::button(ui, Some(Icon::Gear), "", 36.0 * theme::ui_scale(), true).on_hover_text("The block's properties (facing, half...)");
        ctx_menu::dropdown(ui, &menu, |ui, cx| {
            ctx_menu::widen(ui, 220.0);
            for (k, values) in props {
                let now = ws.map.block.properties.get(k).cloned().unwrap_or_default();
                let got = ctx_menu::submenu(ui, cx, Icon::Rename, &format!("{k}: {now}"), |ui| {
                    let mut got = None;
                    for v in values {
                        if ctx_menu::item(ui, if *v == now { Icon::Check } else { Icon::Rename }, v, "", true) {
                            got = Some(v.clone());
                        }
                    }
                    got
                });
                if let Some(v) = got {
                    chosen = Some((k.clone(), v));
                }
            }
        });
    }
    if let Some((k, v)) = chosen {
        ws.map.block.properties.insert(k, v);
    }
}

fn muted(ui: &mut Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text.into()).color(theme::text_muted()));
}

/// The settings of what is being done: the mob selected, the paste, the brush, the selection;
/// else a hint. The block chooser first (Fill, Replace and the brush use it). Ends with how
/// many chunks wait to be saved.
fn context_row(ui: &mut Ui, ws: &mut Workspace, roomy: bool, status: &mut String) {
    block_chooser(ui, ws, roomy);
    ui.separator();
    let map = &mut ws.map;
    let mut run_tool: Option<EditTool> = None;
    if let Some(e) = map.sel_entity.clone() {
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
            run_tool = Some(EditTool::DeleteEntity([pos[0], pos[1], pos[2]]));
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
    } else if let Some(b) = &mut map.brush {
        ui.label(egui::RichText::new(if b.erase { "Erasing" } else { "Brush" }).color(theme::text()));
        muted(ui, "Size");
        ui.add(egui::DragValue::new(&mut b.radius).range(0..=32)).on_hover_text("Blocks from the middle to the edge");
        if ui.selectable_label(b.sphere, "Ball").clicked() {
            b.sphere = true;
        }
        if ui.selectable_label(!b.sphere, "Cube").clicked() {
            b.sphere = false;
        }
        ui.checkbox(&mut b.erase, "Erase");
        ui.checkbox(&mut b.snap, "Chunk center").on_hover_text("Paint around the middle of the chunk pointed at");
        if ui.button("Done").on_hover_text("Esc").clicked() {
            map.brush = None;
        }
        if roomy {
            muted(ui, "Click paints \u{b7} Alt+drag paints along");
        }
    } else if let Some((lo, hi)) = map.sel_box {
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
        *status = match ws.run_edit(t) {
            Ok(m) | Err(m) => m,
        };
    }
}

/// Requests the view makes (a paste placed, the brush clicked, a mob's Delete...) run here.
fn run_view_requests(ws: &mut Workspace, status: &mut String) {
    let mut tool = None;
    if let Some((origin, place)) = ws.map.paste_request.take() {
        let with_air = ws.map.paste.as_ref().is_some_and(|p| p.with_air);
        tool = Some(EditTool::Paste { origin, place, with_air });
    }
    if let Some(p) = ws.map.entity_delete_request.take() {
        tool = Some(EditTool::DeleteEntity(p));
    }
    if let Some((center, stroke)) = ws.map.brush_request.take() {
        if let Some(b) = &ws.map.brush {
            let state = if b.erase { mctk_java::BlockState::air() } else { ws.map.block.clone() };
            tool = Some(EditTool::Brush { center, radius: b.radius, sphere: b.sphere, state, stroke });
        }
    }
    if let Some(t) = tool {
        *status = match ws.run_edit(t) {
            Ok(m) | Err(m) => m,
        };
    }
}

/// The right-click menu in the blocks' views (Top-down and 3D), for the mob or block clicked:
/// turn it, waterlog it, make it something else, open its chunk, or delete it.
fn view_menu(ui: &Ui, ws: &mut Workspace, resp: &egui::Response, status: &mut String) {
    use crate::map_view::MenuTarget;
    if resp.secondary_clicked() {
        ws.map.menu_target = if ws.map.shows_blocks() { ws.map.pointed() } else { None };
    }
    let Some(target) = ws.map.menu_target.clone() else { return };
    let mut tool: Option<EditTool> = None;
    let mut open: Option<(i32, i32)> = None;
    let mut select: Option<MenuTarget> = None;
    // (The consoles name entities their own old way: changing one's kind is left to its tab.)
    let can_retype = matches!(ws.edition, crate::world_picker::UiEdition::Java | crate::world_picker::UiEdition::Bedrock);
    ctx_menu::context(ui, resp, |ui, cx| {
        ctx_menu::widen(ui, 220.0);
        let title = |ui: &mut Ui, t: String| {
            ui.label(egui::RichText::new(t).color(theme::text_muted()));
        };
        match &target {
            MenuTarget::Entity(e, pos) => {
                let c = e.as_compound().cloned().unwrap_or_default();
                let id = mctk_convert::view::java_entity_id(&c).unwrap_or_default();
                let kind = crate::item_picker::display_name(&id);
                title(ui, crate::naming::mob_name(&c).map_or(kind.clone(), |n| format!("{n} ({kind})")));
                if ctx_menu::item(ui, Icon::ArrowRight, "Select", "", true) {
                    select = Some(target.clone());
                }
                if ctx_menu::item(ui, Icon::Refresh, "Turn 90\u{b0}", "", true) {
                    tool = Some(EditTool::TurnEntity(*pos));
                }
                if can_retype {
                    let got = ctx_menu::submenu(ui, cx, Icon::Rename, "Change to", |ui| {
                        // A mob, by its spawn egg in the item list.
                        crate::item_picker::show(ui, "", false).and_then(|id| id.strip_suffix("_spawn_egg").map(str::to_string))
                    });
                    if let Some(m) = got {
                        tool = Some(EditTool::RetypeEntity(*pos, m));
                    }
                }
                if ctx_menu::item(ui, Icon::Folder, "Open Chunk", "its data, with this entity, in a tab", true) {
                    open = Some(((pos[0].floor() as i32).div_euclid(16), (pos[2].floor() as i32).div_euclid(16)));
                }
                ctx_menu::separator(ui);
                if ctx_menu::item(ui, Icon::MobRemove, "Delete", "Del", true) {
                    tool = Some(EditTool::DeleteEntity(*pos));
                }
            }
            MenuTarget::Block(p, name, props) => {
                title(ui, format!("{} at {}, {}, {}", crate::item_picker::display_name(name), p[0], p[1], p[2]));
                let prop = |k: &str| props.split(',').find_map(|kv| kv.strip_prefix(k)?.strip_prefix('=').map(str::to_string));
                if ctx_menu::item(ui, Icon::ArrowRight, "Select", "", true) {
                    select = Some(target.clone());
                }
                let turns = prop("facing").is_some() || prop("axis").is_some() || prop("rotation").is_some();
                if ctx_menu::item(ui, Icon::Refresh, "Turn 90\u{b0}", "", turns) {
                    tool = Some(EditTool::TurnBlock(*p));
                }
                if let Some(wet) = prop("waterlogged") {
                    let on = wet == "true";
                    if ctx_menu::item(ui, if on { Icon::Check } else { Icon::Plus }, "Waterlogged", "", true) {
                        tool = Some(EditTool::BlockProps(*p, vec![("waterlogged".into(), (!on).to_string())]));
                    }
                }
                let got = ctx_menu::submenu(ui, cx, Icon::Rename, "Change to", |ui| crate::item_picker::show_blocks(ui, name));
                if let Some(b) = got {
                    tool = Some(EditTool::SetBlock(*p, crate::edit::states::default_state(&b)));
                }
                if ctx_menu::item(ui, Icon::Folder, "Open Chunk", "its data in a tab", true) {
                    open = Some((p[0].div_euclid(16), p[2].div_euclid(16)));
                }
                ctx_menu::separator(ui);
                if ctx_menu::item(ui, Icon::Trash, "Delete", "make it air", true) {
                    tool = Some(EditTool::SetBlock(*p, mctk_java::BlockState::air()));
                }
            }
        }
    });
    match select {
        Some(MenuTarget::Entity(e, _)) => {
            ws.map.sel_box = None;
            ws.map.sel_entity = Some(e);
        }
        Some(MenuTarget::Block(p, ..)) => {
            ws.map.sel_entity = None;
            ws.map.sel_box = Some((p, p));
            ws.map.box_changed();
        }
        None => {}
    }
    if let Some(t) = tool {
        if matches!(t, EditTool::DeleteEntity(_)) {
            ws.map.sel_entity = None;
        }
        *status = match ws.run_edit(t) {
            Ok(m) | Err(m) => m,
        };
    }
    if let Some(c) = open {
        crate::editor::open_chunks(ws, &[c], status);
    }
}

/// The World Explorer: its toolbars and the map.
pub fn show(ui: &mut Ui, ws: &mut Workspace, status: &mut String, prune: &mut crate::pruner::PruneTools) {
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
    let bar = Rect::from_min_max(full.min, pos2(full.right(), tools_bottom + if blocks { row_h } else { 0.0 }));
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
        let ctx_rect = Rect::from_min_max(pos2(full.left() + 8.0, tools_bottom), pos2(full.right() - 8.0, tools_bottom + row_h - 4.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(ctx_rect));
        child.set_clip_rect(ctx_rect);
        child.horizontal_centered(|ui| context_row(ui, ws, roomy, status));
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