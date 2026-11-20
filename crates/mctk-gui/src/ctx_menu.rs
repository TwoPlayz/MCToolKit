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
    icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0)), theme::text());
    let font = theme::regular(13.5);
    let text = crate::widgets::elide(ui, label, &font, rect.width() - 34.0 - 64.0 - 34.0);
    ui.painter().text(pos2(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, text, font, theme::text());
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(rect.right() - 34.0 - 60.0, rect.top() + 2.0), pos2(rect.right() - 34.0, rect.bottom() - 2.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let r = right.add(egui::DragValue::new(value).range(range));
    // The remove button is as tall as the level box.
    let button = Rect::from_min_max(pos2(rect.right() - 30.0, r.rect.top()), pos2(rect.right() - 6.0, r.rect.bottom()));
    let remove = ui.interact(button, ui.id().with(("remove", label)), Sense::click());
    let fill = if remove.hovered() { theme::button_hover() } else { theme::button() };
    ui.painter().rect_filled(button, theme::radius(3), fill);
    let ink = if remove.hovered() { theme::text() } else { theme::text_muted() };
    icons::paint(ui.painter(), Rect::from_center_size(button.center(), vec2(14.0, 14.0)), Icon::Trash, ink);
    let remove = remove.on_hover_text("Remove");
    (r.changed(), r.drag_stopped() || r.lost_focus(), remove.clicked())
}

/// A thin line between groups of rows, exactly as wide as the rows.
pub fn separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(row_width(ui.ctx()), 9.0), Sense::hover());
    ui.painter().line_segment(
        [pos2(rect.left() + 4.0, rect.center().y), pos2(rect.right() - 4.0, rect.center().y)],
        egui::Stroke::new(1.0_f32, theme::separator()),
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
    row_inner(ui, label, hint, enabled, icon, true, false)
}

/// The menu being drawn: the sub-menu it is (the innermost open one), else the top menu as
/// opened this time.
fn level_id(ctx: &egui::Context) -> Id {
    let stack: Vec<Id> = ctx.data(|d| d.get_temp(Id::new("mctk-ctx-stack"))).unwrap_or_default();
    stack.last().copied().unwrap_or_else(|| {
        let generation: u64 = ctx.data(|d| d.get_temp(Id::new("mctk-ctx-gen"))).unwrap_or(0);
        Id::new("mctk-ctx-menu").with(("top", generation))
    })
}

/// The sub-menu open in the menu `level`. Forgotten once that menu is not drawn (it was closed).
fn open_child(ctx: &egui::Context, level: Id) -> Option<Id> {
    let pass = ctx.cumulative_pass_nr();
    let key = level.with("open-child");
    let now = ctx.data(|d| d.get_temp::<(Option<Id>, u64)>(key)).and_then(|(o, at)| if at + 2 >= pass { o } else { None });
    ctx.data_mut(|d| d.insert_temp(key, (now, pass)));
    now
}

fn set_open_child(ctx: &egui::Context, level: Id, child: Option<Id>) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(level.with("open-child"), (child, pass)));
}

/// A row; `clears`: a click on it closes the sub-menu open in its menu (all but sub-menu rows),
/// `open`: it is the row of the sub-menu open now (lit as if hovered).
fn row_inner(
    ui: &mut Ui,
    label: &str,
    hint: &str,
    enabled: bool,
    icon: impl FnOnce(&Painter, Rect, egui::Color32),
    clears: bool,
    open: bool,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(row_width(ui.ctx()), ROW), Sense::click());
    if enabled && (resp.hovered() || open) {
        ui.painter().rect_filled(rect, theme::radius(3), theme::button_hover());
    }
    // Choosing another row of this menu closes the sub-menu open beside it.
    if clears && enabled && resp.clicked() {
        let level = level_id(ui.ctx());
        set_open_child(ui.ctx(), level, None);
    }
    let ink = if enabled { theme::text() } else { theme::text_dim() };
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
            theme::text_dim(),
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
        theme::text(),
    );
    ui.painter().text(pos2(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, label, theme::regular(13.5), theme::text());
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(rect.left() + 104.0, rect.top() + 2.0), pos2(rect.right() - 4.0, rect.bottom() - 2.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let r = right.add(egui::DragValue::new(value).range(range).suffix(suffix));
    (r.changed(), r.drag_stopped() || r.lost_focus())
}

/// A row with text to type: `label` on the left, a box on the right that starts with `current`
/// each time the menu opens. Gives the text once it is entered (Enter, or leaving the box after
/// changing it); the menu then closes. `hint` shows in the box while it is empty.
pub fn text_row(ui: &mut Ui, icon: Icon, label: &str, current: &str, hint: &str) -> Option<String> {
    let (rect, _) = ui.allocate_exact_size(vec2(row_width(ui.ctx()), ROW), Sense::hover());
    icons::paint(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0)), icon, theme::text());
    ui.painter().text(pos2(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, label, theme::regular(13.5), theme::text());
    // What is typed, kept from frame to frame; a menu opened afresh starts from `current`.
    let (text_id, seen_id) = (ui.id().with(("text-row", label)), ui.id().with(("text-row-seen", label)));
    let frame = ui.ctx().cumulative_pass_nr();
    let last: u64 = ui.ctx().data(|d| d.get_temp(seen_id)).unwrap_or(0);
    ui.ctx().data_mut(|d| d.insert_temp(seen_id, frame));
    let mut text: String = if frame > last + 2 { current.to_string() } else { ui.ctx().data(|d| d.get_temp(text_id)).unwrap_or_else(|| current.to_string()) };
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(rect.left() + 124.0, rect.top() + 3.0), pos2(rect.right() - 6.0, rect.bottom() - 3.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let w = right.available_width();
    let r = right.add_sized([w, ROW - 6.0], egui::TextEdit::singleline(&mut text).hint_text(hint).margin(egui::Margin::symmetric(6, 2)));
    ui.ctx().data_mut(|d| d.insert_temp(text_id, text.clone()));
    let entered = r.lost_focus() && (ui.input(|i| i.key_pressed(egui::Key::Enter)) || text != current);
    if entered {
        ui.ctx().data_mut(|d| d.insert_temp(close_flag(), true));
        ui.close_menu();
        return Some(text);
    }
    None
}

/// A row that turns something on or off, with a box ticked when it is on; true when clicked.
/// The menu stays open, so several can be set in one go.
pub fn toggle(ui: &mut Ui, label: &str, on: bool) -> bool {
    painted_keep(ui, label, "", true, |p, r, ink| {
        let b = r.shrink(1.5);
        if on {
            p.rect_filled(b, theme::radius(3), theme::EDITOR_ACCENT);
            icons::paint(p, b.shrink(1.0), Icon::Check, egui::Color32::WHITE);
        } else {
            p.rect_stroke(b, theme::radius(3), egui::Stroke::new(1.3_f32, ink), egui::StrokeKind::Inside);
        }
    })
}

/// Like [`number_row`], for a number with a fraction (health). Gives (changed, finished).
pub fn decimal_row(ui: &mut Ui, icon: Icon, label: &str, value: &mut f64, range: std::ops::RangeInclusive<f64>) -> (bool, bool) {
    decimal_row_painted(ui, |p, r, ink| icons::paint(p, r, icon, ink), label, value, range)
}

/// Like [`decimal_row`], with the icon drawn by `icon`.
pub fn decimal_row_painted(
    ui: &mut Ui,
    icon: impl FnOnce(&Painter, Rect, egui::Color32),
    label: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
) -> (bool, bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(row_width(ui.ctx()), ROW), Sense::hover());
    icon(ui.painter(), Rect::from_center_size(pos2(rect.left() + 16.0, rect.center().y), vec2(16.0, 16.0)), theme::text());
    ui.painter().text(pos2(rect.left() + 34.0, rect.center().y), Align2::LEFT_CENTER, label, theme::regular(13.5), theme::text());
    let mut right = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_max(pos2(rect.left() + 104.0, rect.top() + 2.0), pos2(rect.right() - 4.0, rect.bottom() - 2.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    let r = right.add(egui::DragValue::new(value).range(range).speed(0.5).max_decimals(2));
    (r.changed(), r.drag_stopped() || r.lost_focus())
}

/// The icon painter for a row pictured by an item (`chest`, `name_tag`...).
pub fn item_icon(id: &str) -> impl FnOnce(&Painter, Rect, egui::Color32) + '_ {
    move |p, r, ink| {
        // In gray, as the menus' own icons are.
        if !icons::item_icon_gray(p, r, id) {
            icons::paint(p, r, Icon::File, ink);
        }
    }
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
    let ctx = ui.ctx().clone();
    // A menu opened again starts with no sub-menu open, wherever the last one was.
    let generation: u64 = ctx.data(|d| d.get_temp(Id::new("mctk-ctx-gen"))).unwrap_or(0);
    let level = level_id(&ctx);
    // Which of this menu's sub-menus is open (one at most), as of what was last clicked.
    let open_now = open_child(&ctx, level);
    // Known by its menu and its name (the names in a menu differ).
    let id = level.with(("submenu", label, generation));
    let is_open = open_now == Some(id);
    let resp = row_inner(ui, label, "", true, icon, false, is_open);
    let screen = ctx.screen_rect();
    // Opens the way the menu it is in opened (so a chain of them does not fold back over
    // itself), while there is room that way; else to the right, else to the left.
    let room_right = resp.rect.right() + 8.0 + width + FRAME <= screen.right();
    let room_left = resp.rect.left() - 8.0 - width - FRAME >= screen.left();
    let parent_left: bool = ctx.data(|d| d.get_temp(level.with("leftward"))).unwrap_or(false);
    let on_right = if parent_left && room_left { false } else { room_right };
    ctx.data_mut(|d| d.insert_temp(id.with("leftward"), !on_right));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(pos2(resp.rect.right() - 12.0, resp.rect.center().y), vec2(10.0, 10.0)),
        if on_right { Icon::ChevronRight } else { Icon::ChevronLeft },
        theme::text_muted(),
    );
    // A click opens it (closing the one open beside it); a click on it again closes it.
    let open = if resp.clicked() {
        let now = !is_open;
        set_open_child(&ctx, level, now.then_some(id));
        ctx.request_repaint();
        now
    } else {
        is_open
    };
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
        let mut stack: Vec<Id> = d.get_temp(Id::new("mctk-ctx-stack")).unwrap_or_default();
        stack.pop();
        d.insert_temp(Id::new("mctk-ctx-stack"), stack);
    });
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
    let at = if resp.secondary_clicked() { ui.ctx().input(|i| i.pointer.interact_pos()) } else { None };
    menu_from(ui, resp, at, content);
}

/// Like [`context`], opened by a click on `resp` (a button), below it.
/// Whether any menu is open.
pub fn any_open(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<(Id, Pos2)>(Id::new("mctk-ctx-menu"))).is_some()
}

/// Whether the menu opened from `resp` is open.
pub fn is_open_for(ctx: &egui::Context, resp: &Response) -> bool {
    ctx.data(|d| d.get_temp::<(Id, Pos2)>(Id::new("mctk-ctx-menu"))).is_some_and(|(owner, _)| owner == resp.id)
}

pub fn dropdown(ui: &Ui, resp: &Response, content: impl FnOnce(&mut Ui, &mut Cx)) {
    let at = resp.clicked().then(|| resp.rect.left_bottom() + vec2(0.0, 4.0));
    menu_from(ui, resp, at, content);
}

fn menu_from(ui: &Ui, resp: &Response, open_at: Option<Pos2>, content: impl FnOnce(&mut Ui, &mut Cx)) {
    let ctx = ui.ctx().clone();
    let state = Id::new("mctk-ctx-menu");
    if open_at.is_some() {
        if let Some(p) = open_at {
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
        d.insert_temp(Id::new("mctk-ctx-stack"), Vec::<Id>::new());
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