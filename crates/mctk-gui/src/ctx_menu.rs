//! Right-click menus: rows with an icon, the action's name and its keyboard shortcut, and a
//! popup that opens its sub-menu on the left when there is no room on the right.

use eframe::egui::{self, Align2, Area, Frame, Id, Painter, Pos2, Rect, Response, Sense, Ui, pos2, vec2};

use crate::icons::{self, Icon};
use crate::theme;

const ROW: f32 = 26.0;
/// Wide enough for the longest label plus a shortcut hint, and no wider.
const WIDTH: f32 = 168.0;
/// A menu's frame, added to its width to decide which side a sub-menu opens on.
const FRAME: f32 = 16.0;

/// Ctrl on Windows and Linux, ⌘ on macOS.
pub fn command_key() -> &'static str {
    if cfg!(target_os = "macos") { "\u{2318}" } else { "Ctrl+" }
}

/// Shortcut text such as "Ctrl+C".
pub fn shortcut(key: &str) -> String {
    format!("{}{key}", command_key())
}

/// How wide rows are in the open menu (a menu can ask for more than the usual width).
fn row_width(ctx: &egui::Context) -> f32 {
    ctx.data(|d| d.get_temp(Id::new("mctk-ctx-width"))).unwrap_or(WIDTH)
}

/// Makes the menu being built (and its rows) at least `width` wide.
pub fn widen(ui: &mut Ui, width: f32) {
    ui.ctx().data_mut(|d| d.insert_temp(Id::new("mctk-ctx-width"), width));
    ui.set_min_width(width);
}

/// A row with a name, a level to type or drag, and a button of its own that removes it. Gives
/// (changed, finished, remove).
pub fn value_row(
    ui: &mut Ui,
    icon: impl FnOnce(&Painter, Rect, egui::Color32),
    label: &str,
    value: &mut i64,
    range: std::ops::RangeInclusive<i64>,
) -> (bool, bool, bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(row_width(ui.ctx()), ROW), Sense::hover());
    icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0)), theme::TEXT);
    let font = theme::regular(13.5);
    let text = crate::widgets::elide(ui, label, &font, rect.width() - 34.0 - 64.0 - 34.0);
    ui.painter().text(pos2(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, text, font, theme::TEXT);
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(rect.right() - 34.0 - 60.0, rect.top() + 2.0), pos2(rect.right() - 34.0, rect.bottom() - 2.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let r = right.add(egui::DragValue::new(value).range(range));
    // The remove button is as tall as the level box.
    let button = Rect::from_min_max(pos2(rect.right() - 30.0, r.rect.top()), pos2(rect.right() - 6.0, r.rect.bottom()));
    let remove = ui.interact(button, ui.id().with(("remove", label)), Sense::click());
    let fill = if remove.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON };
    ui.painter().rect_filled(button, theme::radius(3), fill);
    let ink = if remove.hovered() { theme::TEXT } else { theme::TEXT_MUTED };
    icons::paint(ui.painter(), Rect::from_center_size(button.center(), vec2(14.0, 14.0)), Icon::Trash, ink);
    let remove = remove.on_hover_text("Remove");
    (r.changed(), r.drag_stopped() || r.lost_focus(), remove.clicked())
}

/// A thin line between groups of rows, exactly as wide as the rows.
pub fn separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(row_width(ui.ctx()), 9.0), Sense::hover());
    ui.painter().line_segment(
        [pos2(rect.left() + 4.0, rect.center().y), pos2(rect.right() - 4.0, rect.center().y)],
        egui::Stroke::new(1.0_f32, theme::SEPARATOR),
    );
}

/// Sets the width of the rows drawn next (inside a list of a set width); gives the old one to
/// put back with the same call.
pub fn set_row_width(ctx: &egui::Context, width: f32) -> f32 {
    let old = row_width(ctx);
    ctx.data_mut(|d| d.insert_temp(Id::new("mctk-ctx-width"), width));
    old
}

/// Where the sub-menus are (this pass, and the one before), for clicks and for staying open.
fn sub_rects(ctx: &egui::Context, previous: bool) -> Vec<Rect> {
    let key = Id::new(if previous { "mctk-ctx-subs-prev" } else { "mctk-ctx-subs" });
    ctx.data(|d| d.get_temp(key)).unwrap_or_default()
}

fn close_flag() -> Id {
    Id::new("mctk-ctx-menu-close")
}

/// One menu row; true when clicked (the menu closes). A disabled row is dimmed and inert.
pub fn item(ui: &mut Ui, icon: Icon, label: &str, hint: &str, enabled: bool) -> bool {
    painted(ui, label, hint, enabled, |p, r, ink| icons::paint(p, r, icon, ink))
}

/// Like [`painted`], but the menu stays open after the click (for rows that add things up).
pub fn painted_keep(ui: &mut Ui, label: &str, hint: &str, enabled: bool, icon: impl FnOnce(&Painter, Rect, egui::Color32)) -> bool {
    let resp = row(ui, label, hint, enabled, icon);
    enabled && resp.clicked()
}

/// Like [`item`], with the icon drawn by `icon` into the 16 px box it is given.
pub fn painted(
    ui: &mut Ui,
    label: &str,
    hint: &str,
    enabled: bool,
    icon: impl FnOnce(&Painter, Rect, egui::Color32),
) -> bool {
    let resp = row(ui, label, hint, enabled, icon);
    let clicked = enabled && resp.clicked();
    if clicked {
        ui.ctx().data_mut(|d| d.insert_temp(close_flag(), true));
        ui.close_menu();
    }
    clicked
}

fn row(
    ui: &mut Ui,
    label: &str,
    hint: &str,
    enabled: bool,
    icon: impl FnOnce(&Painter, Rect, egui::Color32),
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(row_width(ui.ctx()), ROW), Sense::click());
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, theme::radius(3), theme::BUTTON_HOVER);
    }
    let ink = if enabled { theme::TEXT } else { theme::TEXT_DIM };
    icon(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0)),
        ink,
    );
    ui.painter().text(
        pos2(rect.left() + 34.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::regular(13.5),
        ink,
    );
    if !hint.is_empty() {
        ui.painter().text(
            pos2(rect.right() - 10.0, rect.center().y),
            Align2::RIGHT_CENTER,
            hint,
            theme::regular(12.0),
            theme::TEXT_DIM,
        );
    }
    resp
}

/// A row with a number to type or drag: `label` on the left, the value on the right. Gives
/// (changed, finished): the value moved, and the user let go or left the box.
pub fn number_row(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    value: &mut i64,
    range: std::ops::RangeInclusive<i64>,
    suffix: &str,
) -> (bool, bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(row_width(ui.ctx()), ROW), Sense::hover());
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0)),
        icon,
        theme::TEXT,
    );
    ui.painter().text(pos2(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, label, theme::regular(13.5), theme::TEXT);
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(rect.left() + 104.0, rect.top() + 2.0), pos2(rect.right() - 4.0, rect.bottom() - 2.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let r = right.add(egui::DragValue::new(value).range(range).suffix(suffix));
    (r.changed(), r.drag_stopped() || r.lost_focus())
}

/// What a menu's content can use besides plain rows.
#[derive(Default)]
pub struct Cx {
    /// Where the open sub-menu is, so clicks inside it do not close the menu.
    sub: Option<Rect>,
}

/// A row that opens a sub-menu of `rows` beside it: to the right, or to the left when the
/// screen has no room on the right. Returns what `rows` returns when something is picked.
pub fn submenu<T>(
    ui: &mut Ui,
    cx: &mut Cx,
    icon: Icon,
    label: &str,
    rows: impl FnOnce(&mut Ui) -> Option<T>,
) -> Option<T> {
    submenu_wide(ui, cx, icon, label, WIDTH, 0.0, rows)
}

/// Closes the menu after a choice made in a list of its own (see [`submenu_wide`]).
pub fn chosen(ui: &mut Ui) {
    ui.ctx().data_mut(|d| d.insert_temp(close_flag(), true));
    ui.close_menu();
}

/// Like [`submenu`], for a sub-menu `width` points wide and about `height` tall, such as a list with
/// a search box: it is moved up as far as it takes to fit on the screen, so it never has to shrink.
pub fn submenu_wide<T>(
    ui: &mut Ui,
    cx: &mut Cx,
    icon: Icon,
    label: &str,
    width: f32,
    height: f32,
    rows: impl FnOnce(&mut Ui) -> Option<T>,
) -> Option<T> {
    submenu_painted(ui, cx, |p, r, ink| icons::paint(p, r, icon, ink), label, width, height, rows)
}

/// Like [`submenu_wide`], with the row's icon drawn by `icon`.
pub fn submenu_painted<T>(
    ui: &mut Ui,
    cx: &mut Cx,
    icon: impl FnOnce(&Painter, Rect, egui::Color32),
    label: &str,
    width: f32,
    height: f32,
    rows: impl FnOnce(&mut Ui) -> Option<T>,
) -> Option<T> {
    let resp = row(ui, label, "", true, icon);
    let ctx = ui.ctx().clone();
    let screen = ctx.screen_rect();
    let on_right = resp.rect.right() + 8.0 + width + FRAME <= screen.right();
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(resp.rect.right() - 12.0, resp.rect.center().y), vec2(10.0, 10.0)),
        if on_right { Icon::ChevronRight } else { Icon::ChevronLeft },
        theme::TEXT_MUTED,
    );
    // A menu opened again starts with no sub-menu open, wherever the last one was.
    let generation: u64 = ctx.data(|d| d.get_temp(Id::new("mctk-ctx-gen"))).unwrap_or(0);
    let id = resp.id.with(("submenu", generation));
    // Open as of the last pass or the one before: a menu not drawn since (its parent was closed)
    // starts closed.
    let pass = ctx.cumulative_pass_nr();
    let was_open: bool = ctx.data(|d| d.get_temp::<(bool, u64)>(id)).is_some_and(|(o, at)| o && at + 2 >= pass);
    let last: Option<Rect> = ctx.data(|d| d.get_temp(id.with("rect")));
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    // Stays open while the pointer crosses the gap to it or is inside it, or in a sub-menu of it.
    // Only its own rect and those of the menus opened from it keep it open (not its parent).
    let kids_key = id.with("kids");
    let mut near_rects: Vec<Rect> = ctx.data(|d| d.get_temp(kids_key)).unwrap_or_default();
    near_rects.extend(last);
    let near = pointer.is_some_and(|p| near_rects.iter().any(|r| r.expand(10.0).contains(p)));
    // A level dragged out past the edge of the menu does not close it.
    let held = was_open && ctx.input(|i| i.pointer.any_down());
    // After such a drag, it stays until the pointer is over another row of the menu.
    let sticky_key = id.with("sticky");
    let sticky: bool = ctx.data(|d| d.get_temp::<(bool, u64)>(sticky_key)).is_some_and(|(s, at)| s && at + 2 >= pass);
    let parent = ctx.memory(|m| m.area_rect(Id::new("mctk-ctx-menu").with("area")));
    let in_parent = pointer.zip(parent).is_some_and(|(p, r)| r.contains(p));
    let sticky_active = sticky && was_open && !(in_parent && !resp.hovered());
    let open = resp.hovered() || (was_open && near) || held || sticky_active;
    ctx.data_mut(|d| d.insert_temp(sticky_key, (held || sticky_active, pass)));
    ctx.data_mut(|d| d.insert_temp(id, (open, pass)));
    if !open {
        return None;
    }
    let top = (resp.rect.top() - 6.0).min(screen.bottom() - height - FRAME - 8.0).max(screen.top() + 8.0);
    let (pos, pivot) = if on_right {
        (pos2(resp.rect.right() + 8.0, top), Align2::LEFT_TOP)
    } else {
        (pos2(resp.rect.left() - 8.0, top), Align2::RIGHT_TOP)
    };
    ctx.data_mut(|d| {
        d.insert_temp(kids_key, Vec::<Rect>::new());
        let mut stack: Vec<Id> = d.get_temp(Id::new("mctk-ctx-stack")).unwrap_or_default();
        stack.push(id);
        d.insert_temp(Id::new("mctk-ctx-stack"), stack);
    });
    let shown = Area::new(id.with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .pivot(pivot)
        .show(&ctx, |ui| {
            Frame::menu(ui.style())
                .show(ui, |ui| {
                    ui.set_min_width(width);
                    rows(ui)
                })
                .inner
        });
    ctx.data_mut(|d| {
        // This menu and what it opened are kept open by the one that opened it.
        let mut stack: Vec<Id> = d.get_temp(Id::new("mctk-ctx-stack")).unwrap_or_default();
        stack.pop();
        if let Some(parent) = stack.last() {
            let mut mine: Vec<Rect> = d.get_temp(kids_key).unwrap_or_default();
            mine.push(shown.response.rect);
            let key = parent.with("kids");
            let mut theirs: Vec<Rect> = d.get_temp(key).unwrap_or_default();
            theirs.extend(mine);
            d.insert_temp(key, theirs);
        }
        d.insert_temp(Id::new("mctk-ctx-stack"), stack);
    });
    ctx.data_mut(|d| d.insert_temp(id.with("rect"), shown.response.rect));
    cx.sub = Some(shown.response.rect);
    ctx.data_mut(|d| {
        let key = Id::new("mctk-ctx-subs");
        let mut all: Vec<Rect> = d.get_temp(key).unwrap_or_default();
        all.push(shown.response.rect);
        d.insert_temp(key, all);
    });
    shown.inner
}

/// Opens `content` as a popup at the pointer when `resp` is right-clicked; it closes when an
/// item is chosen, on Escape, or on a click anywhere outside it.
pub fn context(ui: &Ui, resp: &Response, content: impl FnOnce(&mut Ui, &mut Cx)) {
    let ctx = ui.ctx().clone();
    let state = Id::new("mctk-ctx-menu");
    if resp.secondary_clicked() {
        if let Some(p) = ctx.input(|i| i.pointer.interact_pos()) {
            ctx.data_mut(|d| {
                let g: u64 = d.get_temp(Id::new("mctk-ctx-gen")).unwrap_or(0);
                d.insert_temp(Id::new("mctk-ctx-gen"), g + 1);
                d.insert_temp(state, (resp.id, p));
            });
        }
    }
    let Some((owner, pos)) = ctx.data(|d| d.get_temp::<(Id, Pos2)>(state)) else {
        return;
    };
    if owner != resp.id {
        return;
    }
    let mut cx = Cx::default();
    ctx.data_mut(|d| {
        let now: Vec<Rect> = d.get_temp(Id::new("mctk-ctx-subs")).unwrap_or_default();
        d.insert_temp(Id::new("mctk-ctx-subs-prev"), now);
        d.insert_temp(Id::new("mctk-ctx-subs"), Vec::<Rect>::new());
        d.insert_temp(Id::new("mctk-ctx-width"), WIDTH);
    });
    let shown = Area::new(state.with("area"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(&ctx, |ui| {
            Frame::menu(ui.style()).show(ui, |ui| {
                ui.set_min_width(WIDTH);
                content(ui, &mut cx);
            });
        });
    let chosen = ctx.data_mut(|d| d.remove_temp::<bool>(close_flag())).unwrap_or(false);
    // Read before the input is: reading the data inside the input's closure locks up the app.
    let subs = sub_rects(&ctx, false);
    let outside = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer.interact_pos().is_some_and(|p| {
                !shown.response.rect.contains(p) && !cx.sub.is_some_and(|r| r.contains(p)) && !subs.iter().any(|r| r.contains(p))
            })
    });
    if chosen || outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        ctx.data_mut(|d| d.remove::<(Id, Pos2)>(state));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu_width(with_enchants: bool, hover_it: bool) -> f32 {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let owner = Id::new("owner");
        ctx.data_mut(|d| d.insert_temp(Id::new("mctk-ctx-menu"), (owner, pos2(100.0, 100.0))));
        let mut width = 0.0;
        for i in 0..4 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))),
                events: if hover_it && i > 0 { vec![egui::Event::PointerMoved(pos2(150.0, 125.0))] } else { vec![] },
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let resp = ui.interact(Rect::from_min_size(pos2(0.0, 0.0), vec2(10.0, 10.0)), owner, Sense::click());
                    context(ui, &resp, |ui, cx| {
                        let _ = item(ui, Icon::Copy, "Copy", "Ctrl+C", true);
                        if with_enchants {
                            let _ = crate::enchant::menu(ui, cx, &[("minecraft:sharpness".into(), 5)]);
                        }
                        let _ = item(ui, Icon::Trash, "Delete", "Del", true);
                    });
                });
            });
            width = ctx.memory(|m| m.area_rect(Id::new("mctk-ctx-menu").with("area"))).map_or(0.0, |r| r.width());
        }
        width
    }

    #[test]
    fn the_menu_is_as_wide_as_its_rows() {
        let plain = menu_width(false, false);
        let with = menu_width(true, false);
        let hovered = menu_width(true, true);
        eprintln!("{plain} {with} {hovered}");
        assert!((plain - with).abs() < 1.0, "{plain} vs {with}");
        assert!((plain - hovered).abs() < 1.0, "{plain} vs {hovered}");
    }
}