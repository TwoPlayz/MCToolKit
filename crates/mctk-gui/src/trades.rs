//! "Edit Trades": a villager's (or wandering trader's) trades, one to a row as the game's trading
//! screen shows them: what is paid (one or two stacks), an arrow, what is got; then how many times
//! it has been and can be made. A right-click on a stack changes the item or how many; the
//! buttons at the end of a row move it up or down or delete it, and "Add Trade" adds one.
//!
//! This module only draws and says what the user asked for (as [`TradeEdit`]s); the tree applies
//! them (see `mob_edit`).

use eframe::egui::{self, Align2, Color32, Id, Rect, Sense, Ui, pos2, vec2};

use crate::ctx_menu;
use crate::icons::{self, Icon};
use crate::inventory::{SLOT, SlotInfo, paint_slot, paint_stack};
use crate::item_picker;
use crate::mob_edit::{Trade, TradeEdit};

const ROW: f32 = SLOT + 12.0;
const WIDTH: f32 = 620.0;

/// A stack as the inventory screen's painter wants it.
fn slot_info(id: &str, count: i64) -> SlotInfo {
    SlotInfo {
        slot: 0,
        index: 0,
        id: id.to_string(),
        number: None,
        count,
        max_count: 64,
        damage: None,
        durability: None,
        enchants: Vec::new(),
        unbreakable: false,
        stack: mctk_nbt::Compound::new(),
    }
}

/// Draws the screen. Gives what the user asked for and whether it is still open.
pub fn show(ui: &mut Ui, trades: &[Trade], title: &str, data_version: Option<i64>) -> (Vec<TradeEdit>, bool) {
    let mut edits = Vec::new();
    let mut open = true;
    let full = ui.max_rect();
    // The bar across the top, as the inventory screen has: its name, and the way back.
    let bar = Rect::from_min_size(full.min, vec2(full.width(), 40.0));
    ui.painter().rect_filled(bar, 0.0, crate::theme::bg());
    ui.painter().line_segment([bar.left_bottom(), bar.right_bottom()], egui::Stroke::new(1.0_f32, crate::theme::border()));
    ui.painter().text(bar.left_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, title, crate::theme::bold(15.0), crate::theme::text());
    let mut right = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 5.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    if crate::widgets::square_icon_button(&mut right, Icon::Close, "Back to the tags").clicked() {
        open = false;
    }
    let left = (full.center().x - WIDTH / 2.0).max(full.left() + 8.0);
    let mut inner = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(left, bar.bottom() + 16.0), pos2(left + WIDTH, full.bottom())))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let ui = &mut inner;
    ui.style_mut().interaction.selectable_labels = false;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        if trades.is_empty() {
            ui.label(egui::RichText::new("No trades yet.").color(crate::theme::text_dim()));
        }
        for (n, t) in trades.iter().enumerate() {
            row(ui, n, t, trades.len(), data_version, &mut edits);
        }
        ui.add_space(8.0);
        if crate::widgets::button(ui, Some(Icon::Plus), "Add Trade", 140.0, true).clicked() {
            edits.push(TradeEdit::Add);
        }
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Right-click a stack to change the item or how many.").color(crate::theme::text_muted()).size(12.0));
        ui.add_space(16.0);
    });
    (edits, open)
}

/// One trade's row.
fn row(ui: &mut Ui, n: usize, t: &Trade, total: usize, data_version: Option<i64>, edits: &mut Vec<TradeEdit>) {
    let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, ROW), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(rect, crate::theme::radius(4), crate::theme::card());
    p.text(pos2(rect.left() + 12.0, rect.center().y), Align2::LEFT_CENTER, format!("{}", n + 1), crate::theme::bold(13.0), crate::theme::text_dim());
    let y = rect.center().y - SLOT / 2.0;
    let slot_at = |x: f32| Rect::from_min_size(pos2(rect.left() + x, y), vec2(SLOT, SLOT));
    let places = [(0usize, slot_at(36.0)), (1, slot_at(36.0 + SLOT + 22.0)), (2, slot_at(36.0 + 2.0 * SLOT + 22.0 + 46.0))];
    p.text(pos2(places[0].1.right() + 11.0, rect.center().y), Align2::CENTER_CENTER, "+", crate::theme::bold(15.0), crate::theme::text_dim());
    icons::paint(&p, Rect::from_center_size(pos2(places[1].1.right() + 23.0, rect.center().y), vec2(20.0, 20.0)), Icon::ArrowRight, crate::theme::text_muted());
    for (which, r) in places {
        paint_slot(&p, r);
        let stack = t.items[which].as_ref();
        if let Some((id, count)) = stack {
            // Enchanted: tinted purple, as in the inventory.
            let mut info = slot_info(id, *count);
            if let Some(s) = &t.stacks[which] {
                info.enchants = crate::enchant::read(s);
            }
            paint_stack(&p, r, &info);
        }
        let resp = ui.interact(r, Id::new(("trade-slot", n, which)), Sense::click());
        if resp.hovered() {
            p.rect_filled(r.shrink(2.0), 0.0, Color32::from_white_alpha(40));
            let what = ["Paid", "Also paid", "Got"][which];
            let text = stack.map_or_else(|| format!("{what}: nothing"), |(id, c)| format!("{what}: {} \u{d7}{c}", item_picker::display_name(id)));
            resp.clone().on_hover_text(text);
        }
        slot_menu(ui, &resp, n, which, stack, t.stacks[which].as_ref(), t.max_count, data_version, edits);
    }
    // How often: made so far, and at most.
    let mut x = places[2].1.right() + 24.0;
    let mut number = |ui: &mut Ui, label: &str, value: i64, key: &str, make: &dyn Fn(i64) -> TradeEdit| {
        p.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, label, crate::theme::regular(12.5), crate::theme::text_muted());
        x += p.layout_no_wrap(label.into(), crate::theme::regular(12.5), Color32::WHITE).size().x + 6.0;
        let id = Id::new(("trade-number", key, n));
        let mut v: i64 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(value);
        let r = ui.put(Rect::from_min_size(pos2(x, rect.center().y - 11.0), vec2(52.0, 22.0)), egui::DragValue::new(&mut v).range(0..=9999));
        if r.changed() {
            ui.ctx().data_mut(|d| d.insert_temp(id, v));
        }
        if r.drag_stopped() || r.lost_focus() {
            ui.ctx().data_mut(|d| d.remove_temp::<i64>(id));
            if v != value {
                edits.push(make(v));
            }
        }
        x += 60.0;
    };
    number(ui, "Uses", t.uses, "uses", &|v| TradeEdit::Uses { n, uses: v });
    number(ui, "of", t.max_uses, "max", &|v| TradeEdit::MaxUses { n, max: v });
    // Up, down, delete.
    let mut buttons = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(pos2(rect.right() - 110.0, rect.top()), pos2(rect.right() - 8.0, rect.bottom()))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    if crate::widgets::square_icon_button(&mut buttons, Icon::Trash, "Delete this trade").clicked() {
        edits.push(TradeEdit::Delete(n));
    }
    if n + 1 < total && crate::widgets::square_icon_button(&mut buttons, Icon::ArrowDown, "Move down").clicked() {
        edits.push(TradeEdit::Move { n, up: false });
    }
    if n > 0 && crate::widgets::square_icon_button(&mut buttons, Icon::ArrowUp, "Move up").clicked() {
        edits.push(TradeEdit::Move { n, up: true });
    }
}

/// A stack's right-click menu: change the item, how many (up to what the edition allows), its
/// enchantments, name and the rest as in the tree, or (the second payment) take it away.
#[allow(clippy::too_many_arguments)]
fn slot_menu(ui: &Ui, resp: &egui::Response, n: usize, which: usize, stack: Option<&(String, i64)>, saved: Option<&mctk_nbt::Compound>, max_count: i64, data_version: Option<i64>, edits: &mut Vec<TradeEdit>) {
    ctx_menu::context(ui, resp, |ui, cx| {
        ctx_menu::widen(ui, 232.0);
        let (label, current) = match stack {
            Some((id, _)) => ("Change Item", id.as_str()),
            None => ("Set Item", ""),
        };
        // The item, and how many at the top of the list (the count on its own too), up to what
        // the edition saves.
        let now = stack.map_or(1, |(_, n)| *n);
        let picked = ctx_menu::submenu_wide(ui, cx, if stack.is_some() { Icon::Refresh } else { Icon::Plus }, label, item_picker::WIDTH, item_picker::HEIGHT + 30.0, |ui| {
            item_picker::show_counted(ui, current, false, now, max_count)
        });
        if let Some((id, count)) = picked {
            if let Some(id) = id {
                edits.push(TradeEdit::Item { n, which, id: Some(id) });
            }
            if count != now {
                edits.push(TradeEdit::Count { n, which, count });
            }
        }
        if stack.is_some() {
            if let (Some((id, _)), Some(s)) = (stack, saved) {
                let key = format!("trade-stack-{n}-{which}");
                if let Some(edit) = crate::item_edit::stack_menu(ui, cx, &key, id, s, data_version) {
                    edits.push(TradeEdit::Stack { n, which, edit });
                }
            }
            if which == 1 && ctx_menu::item(ui, Icon::Trash, "Remove", "", true) {
                edits.push(TradeEdit::Item { n, which, id: None });
            }
        }
    });
}
