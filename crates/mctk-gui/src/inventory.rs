//! "Edit Inventory": an inventory list laid out like the game's inventory screen. Each stack sits
//! in the slot its `Slot` says, with its count and, for tools and armor, how much of it is left.
//! Stacks can be dragged to another slot (or onto another stack to swap), and a right-click
//! offers Add Item over an empty slot, Change Item and Delete over a stack.
//!
//! This module only draws and says what the user asked for (as [`Edit`]s); the tree applies them.

use eframe::egui::{self, Align2, Color32, Id, Order, Pos2, Rect, Sense, Ui, pos2, vec2};

use crate::ctx_menu;
use crate::icons::{self, Icon};
use crate::item_picker;

/// One stack, as the screen needs to show it.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotInfo {
    /// The `Slot` tag: 0-8 the hotbar, 9-35 the rest, 100-103 armor (feet to head), -106 offhand.
    pub slot: i8,
    /// Where the stack is in the list.
    pub index: usize,
    /// `minecraft:diamond_sword`.
    pub id: String,
    /// The number a console or old world gives it.
    pub number: Option<i64>,
    pub count: i64,
    /// The most a count can be: 127 where it is a byte.
    pub max_count: i64,
    /// The wear, for items that take any: `Damage`.
    pub damage: Option<i64>,
    /// How much wear the item takes before it breaks.
    pub durability: Option<i64>,
    /// Its enchantments: id and level.
    pub enchants: Vec<(String, i64)>,
    /// Made never to wear out.
    pub unbreakable: bool,
    /// The stack as saved (for its menu: name, attributes...).
    pub stack: mctk_nbt::Compound,
}

/// What the user asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// Put the stack in `from` in the empty slot `to`.
    Move { from: i8, to: i8 },
    /// Trade the places of two stacks.
    Swap(i8, i8),
    Delete(usize),
    /// A new stack of `count` of item `id` in an empty slot.
    Add { slot: i8, id: String, count: i64 },
    /// A copy of a stack copied from a tree (its tags as they were) put in a slot, replacing what is there.
    Paste { slot: i8, stack: mctk_nbt::Compound },
    Change { index: usize, id: String },
    SetCount { index: usize, count: i64 },
    SetDamage { index: usize, damage: i64 },
    /// Give an enchantment a level, or take it off with `None` (the screen's menu now sends
    /// [`Edit::Stack`]; kept for callers that name an enchantment).
    #[allow(dead_code)]
    Enchant { index: usize, id: String, level: Option<i64> },
    /// Anything else the stack's menu changes (name, attributes, adventure-mode blocks...).
    Stack { index: usize, edit: crate::item_edit::StackEdit },
}

/// What the screen remembers between frames.
#[derive(Default)]
pub struct State {
    pub selected: Option<i8>,
    dragging: Option<i8>,
    pending_count: Option<i64>,
    pending_damage: Option<i64>,
}

pub(crate) const SLOT: f32 = 44.0;
const GAP: f32 = 2.0;
const PITCH: f32 = SLOT + GAP;
const PANEL: f32 = 9.0 * PITCH - GAP;

/// The app's colors, in the places the game has its grays.
// (On the light theme the page and panel are near white, so the slots are a clearly darker gray
// with a real dark edge on the top and left, a white one on the bottom and right.)
fn slot_fill() -> Color32 {
    if crate::theme::is_light() { Color32::from_rgb(0xCB, 0xD0, 0xD8) } else { crate::theme::tile() }
}
fn shade() -> Color32 {
    if crate::theme::is_light() { Color32::from_rgb(0x8E, 0x96, 0xA3) } else { crate::theme::deep() }
}
fn light() -> Color32 {
    if crate::theme::is_light() { Color32::WHITE } else { crate::theme::separator() }
}
fn ink() -> Color32 {
    crate::theme::text()
}

/// How much wear an item takes before it breaks, if it takes any.
pub fn durability(id: &str) -> Option<i64> {
    let name = id.rsplit(':').next().unwrap_or(id);
    match name {
        "bow" => return Some(384),
        "crossbow" => return Some(465),
        "fishing_rod" => return Some(64),
        "shears" => return Some(238),
        "flint_and_steel" => return Some(64),
        "shield" => return Some(336),
        "elytra" => return Some(432),
        "trident" => return Some(250),
        "carrot_on_a_stick" => return Some(25),
        "warped_fungus_on_a_stick" => return Some(100),
        "brush" => return Some(64),
        "mace" => return Some(500),
        "turtle_helmet" => return Some(275),
        "wolf_armor" => return Some(64),
        _ => {}
    }
    let (material, piece) = name.split_once('_')?;
    let tool = match material {
        "wooden" => 59,
        "stone" => 131,
        "iron" => 250,
        "golden" => 32,
        "diamond" => 1561,
        "netherite" => 2031,
        "copper" => 190,
        _ => 0,
    };
    if tool > 0 && matches!(piece, "sword" | "pickaxe" | "axe" | "shovel" | "hoe" | "spear") {
        return Some(tool);
    }
    let times = match material {
        "leather" => 5,
        "chainmail" | "iron" => 15,
        "golden" => 7,
        "diamond" => 33,
        "netherite" => 37,
        "copper" => 11,
        _ => return None,
    };
    let piece = match piece {
        "helmet" => 11,
        "chestplate" => 16,
        "leggings" => 15,
        "boots" => 13,
        _ => return None,
    };
    Some(piece * times)
}

/// What kind of screen it is: the player's inventory or one of the containers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Player,
    Ender,
    Chest,
    DoubleChest,
    Hopper,
    Dispenser,
    Furnace,
}

impl Kind {
    /// The kind of container a block entity or minecart id names (`minecraft:chest`, `Chest`).
    pub fn of_container(id: &str) -> Option<Kind> {
        let id = id.rsplit(':').next().unwrap_or(id).to_ascii_lowercase().replace('_', "");
        Some(if id.contains("hopper") {
            Kind::Hopper
        } else if id.contains("dispenser") || id.contains("dropper") {
            Kind::Dispenser
        } else if id.contains("furnace") || id.contains("smoker") {
            Kind::Furnace
        } else if id.contains("chest") || id.contains("barrel") || id.contains("shulker") {
            Kind::Chest
        } else {
            return None;
        })
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::Player => "Inventory",
            Kind::Ender => "Ender Chest",
            Kind::Chest => "Chest",
            Kind::DoubleChest => "Large Chest",
            Kind::Hopper => "Hopper",
            Kind::Dispenser => "Dispenser",
            Kind::Furnace => "Furnace",
        }
    }

    /// How tall the slots are (before the card below them).
    fn body(self) -> f32 {
        match self {
            Kind::Player => TOP + 6.0 + 3.0 * PITCH + 8.0 + PITCH,
            Kind::DoubleChest => 6.0 * PITCH,
            Kind::Hopper => PITCH,
            _ => 3.0 * PITCH,
        }
    }
}

/// How tall the armor column (and the player's window) is.
const TOP: f32 = 4.0 * PITCH - GAP;

/// The dark window where the game shows the player, from the same corner as [`layout`].
fn player_window() -> Rect {
    Rect::from_min_size(pos2(PITCH + 2.0, 26.0), vec2(2.7 * PITCH, TOP))
}

/// Where each slot is, from the top-left corner of the screen's area.
fn layout(kind: Kind) -> Vec<(i8, Pos2)> {
    let mut out = Vec::new();
    let cell = |col: usize, row: f32| pos2(col as f32 * PITCH, row);
    // A container's slots sit in a grid, in the middle of the screen's width.
    let grid = |out: &mut Vec<(i8, Pos2)>, count: i8, cols: usize, left: f32| {
        for s in 0..count {
            out.push((s, pos2(left + (s as usize % cols) as f32 * PITCH, 26.0 + (s as usize / cols) as f32 * PITCH)));
        }
    };
    match kind {
        Kind::Player => {}
        Kind::Ender | Kind::Chest => {
            grid(&mut out, 27, 9, 0.0);
            return out;
        }
        Kind::DoubleChest => {
            grid(&mut out, 54, 9, 0.0);
            return out;
        }
        Kind::Hopper => {
            grid(&mut out, 5, 5, 2.0 * PITCH);
            return out;
        }
        Kind::Dispenser => {
            grid(&mut out, 9, 3, 3.0 * PITCH);
            return out;
        }
        Kind::Furnace => {
            out.push((0, pos2(3.5 * PITCH, 26.0)));
            out.push((1, pos2(3.5 * PITCH, 26.0 + 2.0 * PITCH)));
            out.push((2, pos2(5.5 * PITCH, 26.0 + PITCH)));
            return out;
        }
    }
    // Armor down the left (head first), the player's window beside it, the offhand at the
    // window's bottom right, then the 3 rows, then the hotbar.
    for (i, s) in [103i8, 102, 101, 100].into_iter().enumerate() {
        out.push((s, pos2(0.0, 26.0 + i as f32 * PITCH)));
    }
    let window = player_window();
    out.push((-106, pos2(window.right() + 6.0, 26.0 + 3.0 * PITCH)));
    let rows = 26.0 + TOP + 6.0;
    for s in 9..36i8 {
        out.push((s, cell((s - 9) as usize % 9, rows + ((s - 9) / 9) as f32 * PITCH)));
    }
    for s in 0..9i8 {
        out.push((s, cell(s as usize, rows + 3.0 * PITCH + 8.0)));
    }
    out
}

/// The outline an empty armor or offhand slot shows (a texture of the pack).
/// The picture an empty armor or off-hand slot shows (the game's own, under its sprites since
/// 1.20.2).
fn empty_picture(slot: i8) -> Option<&'static str> {
    Some(match slot {
        103 => "gui/sprites/container/slot/helmet.png",
        102 => "gui/sprites/container/slot/chestplate.png",
        101 => "gui/sprites/container/slot/leggings.png",
        100 => "gui/sprites/container/slot/boots.png",
        -106 => "gui/sprites/container/slot/shield.png",
        _ => return None,
    })
}

fn slot_name(slot: i8) -> String {
    match slot {
        103 => "Head".into(),
        102 => "Chest".into(),
        101 => "Legs".into(),
        100 => "Feet".into(),
        -106 => "Offhand".into(),
        0..=8 => format!("Hotbar {}", slot + 1),
        s => format!("Slot {s}"),
    }
}

/// The game's slot: gray, dark along the top and left, light along the bottom and right.
pub(crate) fn paint_slot(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, 0.0, slot_fill());
    p.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 2.0)), 0.0, shade());
    p.rect_filled(Rect::from_min_size(r.min, vec2(2.0, r.height())), 0.0, shade());
    p.rect_filled(Rect::from_min_size(pos2(r.left(), r.bottom() - 2.0), vec2(r.width(), 2.0)), 0.0, light());
    p.rect_filled(Rect::from_min_size(pos2(r.right() - 2.0, r.top()), vec2(2.0, r.height())), 0.0, light());
}

/// A stack in a slot: its picture, its count and its wear bar.
pub(crate) fn paint_stack(p: &egui::Painter, r: Rect, s: &SlotInfo) {
    let k = r.width() / SLOT;
    let icon = r.shrink(7.0 * k);
    let tint = if s.enchants.is_empty() { Color32::WHITE } else { crate::enchant::PURPLE };
    let drawn = icons::item_icon(p, icon, &s.id, tint);
    if !drawn {
        // No picture in the pack: the first letters of its name.
        let short: String = item_picker::display_name(&s.id).chars().take(2).collect();
        p.text(icon.center(), Align2::CENTER_CENTER, short, crate::theme::bold(15.0 * k), ink());
    }
    if let (Some(damage), Some(max)) = (s.damage, s.durability) {
        if damage > 0 {
            let left = (1.0 - damage as f32 / max as f32).clamp(0.0, 1.0);
            let bar = Rect::from_min_size(pos2(r.left() + 7.0 * k, r.bottom() - 10.0 * k), vec2(30.0 * k, 3.0 * k));
            p.rect_filled(bar.expand2(vec2(0.0, 1.0)), 0.0, Color32::BLACK);
            let color = Color32::from(egui::ecolor::Hsva::new(left / 3.0, 1.0, 1.0, 1.0));
            p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * left, bar.height())), 0.0, color);
        }
    }
    if s.count != 1 {
        let at = r.right_bottom() - vec2(4.0, 3.0) * k;
        let font = crate::theme::bold(13.0 * k.max(0.7));
        p.text(at + vec2(1.0, 1.0), Align2::RIGHT_BOTTOM, s.count.to_string(), font.clone(), Color32::from_rgb(0x3F, 0x3F, 0x3F));
        p.text(at, Align2::RIGHT_BOTTOM, s.count.to_string(), font, Color32::WHITE);
    }
}

/// Draws the screen as a window. Gives what the user asked for and whether it is still open.
pub fn show(
    ui: &mut Ui,
    state: &mut State,
    slots: &[SlotInfo],
    title: &str,
    kind: Kind,
    numeric: bool,
    data_version: Option<i64>,
) -> (Vec<Edit>, bool) {
    let mut edits = Vec::new();
    let mut open = true;
    let full = ui.max_rect();
    {
        {
            // The bar across the top, as the hex editor has: its name, and the way back.
            let bar = Rect::from_min_size(full.min, vec2(full.width(), 40.0));
            ui.painter().rect_filled(bar, 0.0, crate::theme::bg());
            ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], egui::Stroke::new(1.0_f32, crate::theme::border()));
            ui.painter().text(bar.left_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, title, crate::theme::bold(15.0), ink());
            let mut right = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(bar.shrink2(vec2(8.0, 5.0)))
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            if crate::widgets::square_icon_button(&mut right, Icon::Close, "Back to the tags").clicked() {
                open = false;
            }
            // The screen itself, in the middle of what is left.
            // The screen shrinks to fit a short window instead of scrolling.
            let grid = kind.body() + 4.0;
            let k = ((full.bottom() - bar.bottom() - 24.0 - 116.0) / grid).clamp(0.4, 1.0);
            let k = (k * 20.0).floor() / 20.0;
            let left = (full.center().x - PANEL * k / 2.0).max(full.left() + 8.0);
            let mut inner = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_min_max(pos2(left, bar.bottom() + 24.0), pos2(left + PANEL * k, full.bottom())))
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            let ui = &mut inner;
            ui.style_mut().interaction.selectable_labels = false;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(0.0);
            let (area, _) = ui.allocate_exact_size(vec2(PANEL * k, grid * k), Sense::hover());
            // The layout counts from the top of the whole screen, whose first slots are at 26.
            let at = |p: Pos2| area.min + vec2(p.x * k, (p.y - 22.0) * k);            let by_slot = |slot: i8| slots.iter().find(|s| s.slot == slot);
            let mut rects: Vec<(i8, Rect)> = Vec::new();
            let pointer = ui.input(|i| i.pointer.interact_pos());
            let mut released_on = None;
            let mut outline: Option<Rect> = None;

            if kind == Kind::Player {
                let w = player_window();
                let r = Rect::from_min_size(at(w.min), w.size() * k);
                let p = ui.painter();
                p.rect_filled(r, 0.0, crate::theme::field());
                // Dark and light the other way round to a slot: it is sunk in.
                p.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 2.0)), 0.0, shade());
                p.rect_filled(Rect::from_min_size(r.min, vec2(2.0, r.height())), 0.0, shade());
                p.rect_filled(Rect::from_min_size(pos2(r.left(), r.bottom() - 2.0), vec2(r.width(), 2.0)), 0.0, light());
                p.rect_filled(Rect::from_min_size(pos2(r.right() - 2.0, r.top()), vec2(2.0, r.height())), 0.0, light());
            }
            for (slot, pos) in layout(kind) {
                let r = Rect::from_min_size(at(pos), vec2(SLOT, SLOT) * k);
                rects.push((slot, r));
                paint_slot(ui.painter(), r);
                let stack = by_slot(slot);
                if stack.is_none() {
                    if let Some(name) = empty_picture(slot) {
                        if let Some(f) = icons::pack_texture(name) {
                            icons::file_image(ui.painter(), r.shrink(6.0), &f);
                        }
                    }
                }
                if let Some(s) = stack {
                    // The one being dragged is drawn at the pointer, not here.
                    if state.dragging != Some(slot) {
                        paint_stack(ui.painter(), r, s);
                    } else {
                        // Left behind, faint, until it is put down.
                        let mut faint = ui.painter().clone();
                        faint.multiply_opacity(0.3);
                        paint_stack(&faint, r, s);
                    }
                }
                // While a stack is carried, the outline and the highlight are where it would land.
                let over = pointer.is_some_and(|p| r.contains(p));
                let outlined = if state.dragging.is_some() { over } else { state.selected == Some(slot) };
                if outlined {
                    outline = Some(r);
                }
                let resp = ui.interact(r, Id::new(("inv-slot", slot)), Sense::click_and_drag());
                if resp.hovered() && state.dragging.is_none() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                // (A stack being carried keeps its own slot hovered; only where it would land lights up.)
                if (state.dragging.is_none() && resp.hovered()) || (state.dragging.is_some() && over) {
                    ui.painter().rect_filled(r.shrink(2.0), 0.0, Color32::from_white_alpha(46));
                    // A hint with what the stack is; not while carrying one or with a menu open.
                    let menu_open = ui.ctx().data(|d| d.get_temp::<(Id, Pos2)>(Id::new("mctk-ctx-menu"))).is_some();
                    if state.dragging.is_none() && !menu_open {
                        resp.clone().on_hover_ui(|ui| {
                            ui.set_max_width(260.0);
                            let label = |ui: &mut Ui, text: String, color: Color32, size: f32| {
                                ui.add(egui::Label::new(egui::RichText::new(text).color(color).size(size)).selectable(false));
                            };
                            match stack {
                                None => label(ui, slot_name(slot), crate::theme::text_muted(), 12.5),
                                Some(s) => {
                                    let enchanted = !s.enchants.is_empty();
                                    let ink = if enchanted { Color32::from_rgb(0xCF, 0xA8, 0xFF) } else { crate::theme::text() };
                                    label(ui, item_picker::display_name(&s.id), ink, 14.0);
                                    label(ui, format!("{}  |  {}", s.number.map_or(s.id.clone(), |n| n.to_string()), slot_name(slot)), crate::theme::text_muted(), 12.0);
                                    let mut line = format!("Count {}", s.count);
                                    if let (Some(d), Some(max)) = (s.damage, s.durability) {
                                        line.push_str(&format!("   Durability {} / {}", max - d, max));
                                    }
                                    label(ui, line, crate::theme::text_muted(), 12.0);
                                    for (id, level) in &s.enchants {
                                        label(ui, format!("{} {}", crate::enchant::name(id), crate::enchant::level_text(*level)), Color32::from_rgb(0xCF, 0xA8, 0xFF), 12.0);
                                    }
                                }
                            }
                        });
                    }
                }
                if resp.clicked() || resp.secondary_clicked() || resp.drag_started() {
                    state.selected = Some(slot);
                    state.pending_count = None;
                    state.pending_damage = None;
                }
                if resp.drag_started() && stack.is_some() {
                    state.dragging = Some(slot);
                }
                // A right-click: add over nothing; change or delete over a stack.
                ctx_menu::context(ui, &resp, |ui, cx| match stack {
                    None => {
                        // The item, and how many at the top of the list.
                        let picked = ctx_menu::submenu_wide(ui, cx, Icon::Plus, "Add Item", item_picker::WIDTH, item_picker::HEIGHT + 30.0, |ui| {
                            item_picker::show_counted(ui, "", numeric, 1, if numeric { 127 } else { 99 })
                        });
                        if let Some((Some(id), count)) = picked {
                            edits.push(Edit::Add { slot, id, count });
                        }
                        // What was copied in a tree, when that is an item.
                        let copied = crate::nbt_view::clipboard_stack();
                        if ctx_menu::item(ui, Icon::Paste, "Paste Item", "", copied.is_some()) {
                            if let Some(stack) = copied {
                                edits.push(Edit::Paste { slot, stack });
                            }
                        }
                    }
                    Some(s) => {
                        ctx_menu::widen(ui, 232.0);
                        // The item, and how many at the top of the list (the count on its own too).
                        let picked = ctx_menu::submenu_wide(ui, cx, Icon::Refresh, "Change Item", item_picker::WIDTH, item_picker::HEIGHT + 30.0, |ui| {
                            item_picker::show_counted(ui, &s.id, numeric, s.count, s.max_count)
                        });
                        if let Some((id, count)) = picked {
                            if let Some(id) = id {
                                edits.push(Edit::Change { index: s.index, id });
                            }
                            if count != s.count {
                                edits.push(Edit::SetCount { index: s.index, count });
                            }
                        }
                        if let (Some(damage), Some(max)) = (s.damage, s.durability) {
                            let mut left = state.pending_damage.map_or((max - damage).max(0), |d| (max - d).max(0));
                            let (changed, done) = ctx_menu::number_row(ui, Icon::Rename, "Durability", &mut left, (max - 32767)..=(max + 32767), &format!(" / {max}"));
                            if changed {
                                state.pending_damage = Some(max - left);
                            }
                            if done {
                                if let Some(d) = state.pending_damage.take().filter(|d| *d != damage) {
                                    edits.push(Edit::SetDamage { index: s.index, damage: d });
                                }
                            }
                        }
                        // Its enchantments, unbreakable, attributes, adventure-mode blocks and name:
                        // the same rows as in the tree.
                        let key = format!("inv-stack-{}-{}", slot, s.index);
                        if let Some(edit) = crate::item_edit::stack_menu(ui, cx, &key, &s.id, &s.stack, data_version) {
                            edits.push(Edit::Stack { index: s.index, edit });
                        }
                        ctx_menu::separator(ui);
                        let copied = crate::nbt_view::clipboard_stack();
                        if ctx_menu::item(ui, Icon::Paste, "Paste Item", "replaces this one", copied.is_some()) {
                            if let Some(stack) = copied {
                                edits.push(Edit::Paste { slot, stack });
                            }
                        }
                        if ctx_menu::item(ui, Icon::Trash, "Remove", "", true) {
                            edits.push(Edit::Delete(s.index));
                        }
                    }
                });
                if state.dragging.is_some() && ui.input(|i| i.pointer.any_released()) && pointer.is_some_and(|p| r.contains(p)) {
                    released_on = Some(slot);
                }
            }

            // The outline goes over the neighboring slots, so none of it is cut off.
            if let Some(r) = outline {
                ui.painter().rect_stroke(r.expand(1.0), 0.0, egui::Stroke::new(2.0_f32, crate::theme::EDITOR_ACCENT), egui::StrokeKind::Outside);
            }
            // What is being dragged follows the pointer.
            if let (Some(from), Some(p)) = (state.dragging, pointer) {
                if let Some(s) = by_slot(from) {
                    let painter = ui.ctx().layer_painter(egui::LayerId::new(Order::Tooltip, Id::new("inv-drag")));
                    paint_stack(&painter, Rect::from_center_size(p, vec2(SLOT, SLOT) * k), s);
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            }
            if state.dragging.is_some() && ui.input(|i| i.pointer.any_released()) {
                if let (Some(from), Some(to)) = (state.dragging, released_on) {
                    if from != to {
                        edits.push(if by_slot(to).is_some() { Edit::Swap(from, to) } else { Edit::Move { from, to } });
                        state.selected = Some(to);
                    }
                }
                state.dragging = None;
            }

            // The chosen stack, as a card in the app's style; its count and wear are edited from
            // the right-click menu.
            ui.add_space(8.0);
            let chosen = state.selected.and_then(by_slot);
            let tall = chosen.is_some_and(|s| !s.enchants.is_empty());
            let (card, _) = ui.allocate_exact_size(vec2(PANEL * k, if tall { 78.0 } else { 62.0 }), Sense::hover());
            ui.painter().rect_filled(card, crate::theme::radius(4), crate::theme::card());
            ui.painter().rect_stroke(card, crate::theme::radius(4), egui::Stroke::new(1.0_f32, crate::theme::border()), egui::StrokeKind::Inside);
            match chosen {
                Some(s) => {
                    let pic = Rect::from_min_size(card.min + vec2(9.0, 9.0), vec2(44.0, 44.0));
                    paint_slot(ui.painter(), pic);
                    paint_stack(ui.painter(), pic, &SlotInfo { count: 1, damage: None, ..s.clone() });
                    let id = s.number.map_or(s.id.clone(), |n| n.to_string());
                    let mut line = format!("Count {}", s.count);
                    if let (Some(d), Some(max)) = (s.damage, s.durability) {
                        line.push_str(&format!("   Durability {} / {}", (max - d).max(0), max));
                    }
                    let p = ui.painter();
                    p.text(card.left_top() + vec2(64.0, 12.0), Align2::LEFT_TOP, item_picker::display_name(&s.id), crate::theme::bold(14.0), ink());
                    p.text(card.left_top() + vec2(64.0, 31.0), Align2::LEFT_TOP, format!("{id}  |  {}", slot_name(s.slot)), crate::theme::regular(12.0), crate::theme::text_muted());
                    p.text(card.left_top() + vec2(64.0, 45.0), Align2::LEFT_TOP, line, crate::theme::regular(12.0), crate::theme::text_muted());
                    if !s.enchants.is_empty() {
                        let text: Vec<String> = s.enchants.iter().map(|(id, l)| format!("{} {}", crate::enchant::name(id), crate::enchant::level_text(*l))).collect();
                        let font = crate::theme::regular(12.0);
                        let text = crate::widgets::elide(ui, &text.join(", "), &font, card.width() - 74.0);
                        ui.painter().text(card.left_top() + vec2(64.0, 60.0), Align2::LEFT_TOP, text, font, Color32::from_rgb(0xCF, 0xA8, 0xFF));
                    }
                }
                None => {
                    ui.painter().text(
                        card.left_center() + vec2(14.0, 0.0),
                        Align2::LEFT_CENTER,
                        "Click a slot. Drag a stack to move or swap it; right-click for more.",
                        crate::theme::regular(12.0),
                        crate::theme::text_muted(),
                    );
                }
            }
            ui.add_space(16.0);
            });
        }
    }
    (edits, open)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(slot: i8, index: usize, id: &str) -> SlotInfo {
        SlotInfo {
            slot,
            index,
            id: id.into(),
            number: None,
            count: 1,
            max_count: 127,
            damage: None,
            durability: durability(id),
            enchants: Vec::new(),
            unbreakable: false,
            stack: mctk_nbt::Compound::new(),
        }
    }

    #[test]
    fn tools_and_armor_know_how_much_they_take() {
        assert_eq!(durability("minecraft:diamond_sword"), Some(1561));
        assert_eq!(durability("minecraft:wooden_pickaxe"), Some(59));
        assert_eq!(durability("minecraft:iron_chestplate"), Some(240));
        assert_eq!(durability("minecraft:leather_boots"), Some(65));
        assert_eq!(durability("minecraft:netherite_helmet"), Some(407));
        assert_eq!(durability("minecraft:bow"), Some(384));
        assert_eq!(durability("minecraft:apple"), None);
        assert_eq!(durability("minecraft:diamond"), None);
    }

    #[test]
    fn every_inventory_slot_has_its_own_place() {
        for (kind, count) in [(Kind::Player, 41), (Kind::Ender, 27), (Kind::DoubleChest, 54), (Kind::Hopper, 5), (Kind::Dispenser, 9), (Kind::Furnace, 3)] {
            let places = layout(kind);
            assert_eq!(places.len(), count);
            let mut seen = std::collections::HashSet::new();
            assert!(places.iter().all(|(s, _)| seen.insert(*s)));
            let mut spots = std::collections::HashSet::new();
            assert!(places.iter().all(|(_, p)| spots.insert((p.x as i32, p.y as i32))));
        }
    }

    #[test]
    fn the_screen_draws_with_stacks_in_it() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut state = State { selected: Some(0), ..Default::default() };
        let mut sword = stack(0, 0, "minecraft:diamond_sword");
        sword.damage = Some(100);
        let mut apples = stack(9, 1, "minecraft:apple");
        apples.count = 12;
        let slots = vec![sword, apples, stack(103, 2, "minecraft:iron_helmet")];
        for ender in [Kind::Player, Kind::Ender, Kind::DoubleChest, Kind::Furnace] {
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0))),
                    ..Default::default()
                };
                let mut result = (Vec::new(), true);
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        result = show(ui, &mut state, &slots, "Inventory", ender, false, None);
                    });
                });
                assert!(result.1 && result.0.is_empty());
            }
        }
    }

    /// The right-click menu with its sub-menus opened by a pointer sweeping over them keeps
    /// drawing quickly (a stuck layout shows up as a test that never ends or is slow).
    #[test]
    fn the_menu_and_its_submenus_stay_quick_as_the_pointer_sweeps() {
        let ctx = egui::Context::default();
        crate::theme::install(&ctx);
        let mut state = State { selected: Some(0), ..Default::default() };
        let mut sword = stack(0, 0, "minecraft:diamond_sword");
        sword.damage = Some(10);
        sword.enchants = vec![("minecraft:sharpness".into(), 5), ("minecraft:unbreaking".into(), 3)];
        let slots = vec![sword];
        ctx.data_mut(|d| d.insert_temp(Id::new("mctk-ctx-menu"), (Id::new(("inv-slot", 0i8)), pos2(300.0, 200.0))));
        let started = std::time::Instant::now();
        let mut frames = 0;
        for y in (190..420).step_by(6) {
            for x in [310.0f32, 380.0, 450.0, 520.0, 600.0, 700.0] {
                let input = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))),
                    events: vec![egui::Event::PointerMoved(pos2(x, y as f32))],
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let _ = show(ui, &mut state, &slots, "Inventory", Kind::Player, false, None);
                    });
                });
                frames += 1;
            }
        }
        // A click with the menu open (anywhere) must not lock up either.
        for pressed in [true, false] {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))),
                events: vec![egui::Event::PointerButton { pos: pos2(900.0, 600.0), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let _ = show(ui, &mut state, &slots, "Inventory", Kind::Player, false, None);
                });
            });
        }
        let each = started.elapsed().as_secs_f64() / frames as f64;
        eprintln!("{frames} frames, {:.1} ms each", each * 1000.0);
        assert!(each < 0.05, "{each}");
    }
}