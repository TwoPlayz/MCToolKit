//! Custom-painted controls.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui,
    Vec2, vec2,
};

use crate::icons::{self, Icon};
use crate::theme;

/// A rounded panel (`#232830` with a hairline border).
pub fn card<R>(ui: &mut Ui, margin: i8, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::card())
        .stroke(Stroke::new(1.0_f32, theme::border()))
        .corner_radius(theme::radius(6))
        .inner_margin(egui::Margin::same(margin))
        .show(ui, add)
        .inner
}

fn text_width(ui: &Ui, text: &str, font: &egui::FontId) -> f32 {
    ui.fonts(|f| {
        f.layout_no_wrap(text.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    })
}

/// `text` shortened with "…" so it fits in `max` points.
pub fn elide(ui: &Ui, text: &str, font: &egui::FontId, max: f32) -> String {
    if text_width(ui, text, font) <= max {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let s: String = chars[..mid].iter().chain(['…'].iter()).collect();
        if text_width(ui, &s, font) <= max {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().chain(['…'].iter()).collect()
}

/// Invite link to the MCToolKit Discord server.
pub const DISCORD_URL: &str = "https://discord.gg/wC6zC93fTS";

/// Opens the Discord invite in the default browser.
pub fn open_discord(ui: &Ui) {
    ui.ctx().open_url(egui::OpenUrl::new_tab(DISCORD_URL));
}

/// Gray button with an optional icon; `width` 0 sizes it to its content.
pub fn button(ui: &mut Ui, icon: Option<Icon>, text: &str, width: f32, enabled: bool) -> Response {
    button_colored(ui, icon, text, width, enabled, false)
}

/// The same button, green while it is usable: the next thing to do.
#[allow(dead_code)]
pub fn green_button(ui: &mut Ui, icon: Option<Icon>, text: &str, width: f32, enabled: bool) -> Response {
    button_colored(ui, icon, text, width, enabled, true)
}

/// A button as tall as the other controls in a toolbar (28 px rather than 32).
pub fn compact_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    button_sized(ui, None, text, 0.0, enabled, false, 28.0)
}

fn button_colored(
    ui: &mut Ui,
    icon: Option<Icon>,
    text: &str,
    width: f32,
    enabled: bool,
    green: bool,
) -> Response {
    button_sized(ui, icon, text, width, enabled, green, 32.0)
}

fn button_sized(
    ui: &mut Ui,
    icon: Option<Icon>,
    text: &str,
    width: f32,
    enabled: bool,
    green: bool,
    height: f32,
) -> Response {
    let k = theme::ui_scale();
    let font = theme::regular(14.0 * k);
    let content = text_width(ui, text, &font) + if icon.is_some() { 26.0 * k } else { 0.0 };
    let size = vec2(if width > 0.0 { width } else { content + 24.0 * k }, height * k);
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let fill = if !enabled {
        theme::tile()
    } else if green {
        if resp.hovered() { theme::accent_hover() } else { theme::accent() }
    } else if resp.hovered() {
        theme::button_hover()
    } else {
        theme::button()
    };
    ui.painter().rect_filled(rect, theme::radius(4), fill);
    let color = if enabled && green {
        Color32::WHITE
    } else if enabled {
        theme::text()
    } else {
        theme::text_dim()
    };
    let start = rect.center().x - content / 2.0;
    if let Some(i) = icon {
        let cx = if text.is_empty() {
            rect.center().x
        } else {
            start + 9.0 * k
        };
        let r = Rect::from_center_size(egui::pos2(cx, rect.center().y), Vec2::splat(18.0 * k));
        icons::paint(ui.painter(), r, i, color);
    }
    let tx = if icon.is_some() { start + 26.0 * k } else { start };
    ui.painter().text(
        egui::pos2(tx, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        font,
        color,
    );
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Green call-to-action button.
pub fn primary(ui: &mut Ui, text: &str, size: Vec2, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let fill = if !enabled {
        theme::button()
    } else if resp.hovered() {
        theme::accent_hover()
    } else {
        theme::accent()
    };
    ui.painter().rect_filled(rect, theme::radius(5), fill);
    let color = if enabled {
        Color32::WHITE
    } else {
        theme::text_dim()
    };
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        theme::regular(18.0),
        color,
    );
    resp
}

/// Large gray button with big text (Start Over, Stop Conversion).
pub fn big_button(ui: &mut Ui, text: &str, size: Vec2) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        theme::button_hover()
    } else {
        theme::button()
    };
    ui.painter().rect_filled(rect, theme::radius(5), fill);
    ui.painter().rect_stroke(
        rect,
        theme::radius(5),
        Stroke::new(1.0_f32, theme::chip()),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        theme::regular(18.0),
        theme::text(),
    );
    resp
}

/// Green pill switch with a label.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str, enabled: bool) -> Response {
    toggle_with(ui, on, label, enabled, theme::regular(15.0))
}

/// The Settings page's switch, with a bold label.
pub fn toggle_bold(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    toggle_with(ui, on, label, true, theme::bold(16.0))
}

fn toggle_with(
    ui: &mut Ui,
    on: &mut bool,
    label: &str,
    enabled: bool,
    font: egui::FontId,
) -> Response {
    let size = vec2(40.0 + 10.0 + text_width(ui, label, &font), 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let track = Rect::from_min_size(rect.min + vec2(0.0, 2.0), vec2(40.0, 18.0));
    let t = ui.ctx().animate_bool(resp.id, *on);
    let fill = if !enabled {
        theme::chip()
    } else {
        let (on, off) = (theme::accent(), theme::chip());
        Color32::from_rgb(
            (off.r() as f32 + (on.r() as f32 - off.r() as f32) * t) as u8,
            (off.g() as f32 + (on.g() as f32 - off.g() as f32) * t) as u8,
            (off.b() as f32 + (on.b() as f32 - off.b() as f32) * t) as u8,
        )
    };
    ui.painter().rect_filled(track, theme::radius(9), fill);
    let knob = egui::pos2(track.left() + 9.0 + t * 22.0, track.center().y);
    ui.painter().circle_filled(
        knob,
        7.0,
        if enabled {
            Color32::WHITE
        } else {
            theme::text_dim()
        },
    );
    let color = if enabled {
        theme::text()
    } else {
        theme::text_dim()
    };
    ui.painter().text(
        egui::pos2(track.right() + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        font,
        color,
    );
    resp
}

/// Segmented choice ("No Change | Survival | …"). Returns true when the choice changed.
pub fn segmented(
    ui: &mut Ui,
    selected: &mut usize,
    options: &[&str],
    width: f32,
    enabled: bool,
) -> bool {
    let k = theme::ui_scale();
    let font = theme::regular(13.5 * k);
    // Segments fit their text; a fixed width is shared out in proportion to it.
    let mut widths: Vec<f32> = options
        .iter()
        .map(|o| text_width(ui, o, &font) + 22.0 * k)
        .collect();
    if width > 0.0 {
        let natural: f32 = widths.iter().sum();
        widths.iter_mut().for_each(|w| *w *= width / natural);
    }
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(vec2(total, 30.0 * k), Sense::hover());
    ui.painter()
        .rect_filled(rect, theme::radius(4), theme::deep());
    let mut x = rect.left();
    let mut changed = false;
    for (i, (opt, w)) in options.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(egui::pos2(x, rect.top()), vec2(*w, rect.height()));
        let resp = ui.interact(
            r,
            ui.id()
                .with(("seg", opt, i, rect.left() as i32, rect.top() as i32)),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );
        if resp.clicked() && *selected != i {
            *selected = i;
            changed = true;
        }
        let sel = *selected == i;
        if sel {
            ui.painter()
                .rect_filled(r.shrink(1.0), theme::radius(4), theme::card());
            ui.painter().rect_stroke(
                r.shrink(1.0),
                theme::radius(4),
                Stroke::new(1.2_f32, theme::accent()),
                StrokeKind::Inside,
            );
        } else if resp.hovered() && enabled {
            ui.painter()
                .rect_filled(r.shrink(1.0), theme::radius(4), theme::tile());
        }
        let color = match (sel, enabled) {
            (true, true) => theme::strong(),
            (_, false) => theme::text_dim(),
            _ => theme::text_muted(),
        };
        let f = if sel { theme::bold(13.5 * k) } else { font.clone() };
        ui.painter()
            .text(r.center(), Align2::CENTER_CENTER, *opt, f, color);
        x += w;
    }
    changed
}

/// The corners of a drop-down button: square along the bottom while its list is open, so the two
/// look like one piece.
pub fn dropdown_corners(open: bool) -> egui::CornerRadius {
    if open {
        egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }
    } else {
        theme::radius(4)
    }
}

/// Whether the drop-down list `id` is showing.
pub fn dropdown_open(ui: &Ui, id: Id) -> bool {
    ui.memory(|m| m.is_popup_open(id))
}

/// Tells drop-downs that the page being drawn sits in a scaled layer (see the converter), so
/// their lists, which are layers of their own, take the same scale and place.
pub fn set_page_transform(ctx: &egui::Context, transform: Option<egui::emath::TSTransform>) {
    let key = Id::new("page-transform");
    ctx.data_mut(|d| match transform {
        Some(t) => d.insert_temp(key, t),
        None => {
            d.remove_temp::<egui::emath::TSTransform>(key);
        }
    });
}

/// The scale in force for the page being drawn, if any.
pub fn page_transform(ctx: &egui::Context) -> Option<egui::emath::TSTransform> {
    ctx.data(|d| d.get_temp(Id::new("page-transform")))
}

/// The list of a drop-down, hung straight under `anchor` with a flat top, the same color and
/// width, so it joins the button. A click anywhere but on the button closes it.
pub fn dropdown(
    ui: &mut Ui,
    id: Id,
    anchor: &Response,
    fill: Color32,
    border: Option<Stroke>,
    add: impl FnOnce(&mut Ui, f32),
) {
    dropdown_pinned(ui, id, anchor, fill, border, add, 0.0, |_, _| {});
}

/// A drop-down whose list scrolls but whose last row, `pinned` points tall, stays in view under
/// it however many rows there are (for an "Open another world" that must always be reachable).
/// The list is as long as its rows need, up to the room the window has.
#[allow(clippy::too_many_arguments)]
pub fn dropdown_pinned(
    ui: &mut Ui,
    id: Id,
    anchor: &Response,
    fill: Color32,
    border: Option<Stroke>,
    add: impl FnOnce(&mut Ui, f32),
    pinned: f32,
    add_pinned: impl FnOnce(&mut Ui, f32),
) {
    if !dropdown_open(ui, id) {
        return;
    }
    let width = anchor.rect.width();
    let transform = page_transform(ui.ctx());
    if let Some(t) = transform {
        ui.ctx()
            .set_transform_layer(egui::LayerId::new(egui::Order::Foreground, id.with("list")), t);
    }
    egui::Area::new(id.with("list"))
        .order(egui::Order::Foreground)
        .fixed_pos(anchor.rect.left_bottom())
        .constrain(transform.is_none())
        .show(ui.ctx(), |ui| {
            let corners = egui::CornerRadius { nw: 0, ne: 0, sw: 4, se: 4 };
            let shown = egui::Frame::new()
                .fill(fill)
                .corner_radius(corners)
                .inner_margin(egui::Margin::ZERO)
                .show(ui, |ui| {
                    ui.set_width(width);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    // Plain rows (selectable labels) stretch across the whole list.
                    // A long list (many versions) scrolls instead of running off the window.
                    // (In a scaled page the window's edge is further off, in the page's own units.)
                    let bottom = match transform {
                        Some(t) => t.inverse() * ui.ctx().screen_rect().right_bottom(),
                        None => ui.ctx().screen_rect().right_bottom(),
                    }
                    .y;
                    let room = (bottom - anchor.rect.bottom() - 12.0 - pinned).max(84.0);
                    egui::ScrollArea::vertical()
                        .max_height(room)
                        .auto_shrink([false, true])
                        .id_salt(id.with("scroll"))
                        .show(ui, |ui| {
                            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                                add(ui, width);
                            });
                        });
                    if pinned > 0.0 {
                        add_pinned(ui, width);
                    }
                });
            if let Some(stroke) = border {
                // Drawn a little above the list and clipped to it, so there is no line along the
                // top where it joins the button.
                let rect = shown.response.rect;
                ui.painter().with_clip_rect(rect).rect_stroke(
                    rect.with_min_y(rect.top() - 2.0),
                    corners,
                    stroke,
                    StrokeKind::Inside,
                );
            }
        });
    let click = ui.input(|i| i.pointer.any_click().then(|| i.pointer.interact_pos()).flatten());
    let click = click.map(|p| transform.map_or(p, |t| t.inverse() * p));
    if click.is_some_and(|p| !anchor.rect.contains(p)) {
        ui.memory_mut(|m| m.close_popup());
    }
}

/// A drop-down for choosing one of `labels`: the button is as wide as the longest of them (so the
/// list is never wider than it), and the list drops straight from it. Gives the index picked.
pub fn choice(ui: &mut Ui, id_salt: &str, current: usize, labels: &[&str]) -> Option<usize> {
    let font = theme::regular(14.0);
    let widest = labels.iter().map(|l| text_width(ui, l, &font)).fold(0.0, f32::max);
    let id = ui.make_persistent_id(("choice", id_salt));
    let (r, resp) = ui.allocate_exact_size(vec2(widest + 44.0, 28.0), Sense::click());
    let open = dropdown_open(ui, id);
    ui.painter().rect_filled(
        r,
        dropdown_corners(open),
        if resp.hovered() { theme::button_hover() } else { theme::button() },
    );
    ui.painter().text(
        r.left_center() + vec2(10.0, 0.0),
        Align2::LEFT_CENTER,
        labels.get(current).copied().unwrap_or(""),
        font,
        theme::text(),
    );
    icons::paint(
        ui.painter(),
        Rect::from_center_size(r.right_center() - vec2(12.0, 0.0), Vec2::splat(10.0)),
        if open { Icon::ChevronUp } else { Icon::ChevronDown },
        theme::text_muted(),
    );
    if resp.clicked() {
        ui.memory_mut(|m| m.toggle_popup(id));
    }
    let mut picked = None;
    let tints = (theme::button_hover(), theme::wash(14));
    dropdown(ui, id, &resp, theme::button(), None, |ui, _| {
        for (i, l) in labels.iter().enumerate() {
            if dropdown_item(ui, i == current, i + 1 == labels.len(), tints, l).clicked() {
                picked = Some(i);
            }
        }
    });
    picked
}

/// The three dimension buttons of the World Explorer (Overworld, Nether, The End) in their dark
/// panel; `current` is the one lit. Gives the one clicked.
pub fn dimension_buttons(ui: &mut Ui, current: usize, enabled: bool) -> Option<usize> {
    let (rect, _) = ui.allocate_exact_size(vec2(150.0, 44.0), Sense::hover());
    ui.painter().rect_filled(rect, theme::radius(5), theme::deep());
    let mut picked = None;
    for (i, label) in ["Overworld", "Nether", "The End"].into_iter().enumerate() {
        let r = Rect::from_min_size(rect.min + vec2(4.0 + i as f32 * 48.0, 4.0), vec2(46.0, 36.0));
        let resp = ui.interact(r, ui.id().with(("dim-button", i)), Sense::click()).on_hover_text(label);
        let selected = current == i;
        if resp.hovered() && !selected && enabled {
            ui.painter().rect_filled(r, theme::radius(4), theme::button_hover());
        }
        if selected {
            ui.painter().rect_filled(r, theme::radius(4), theme::card());
            ui.painter().rect_stroke(
                r,
                theme::radius(4),
                Stroke::new(1.2_f32, theme::accent()),
                StrokeKind::Inside,
            );
        }
        crate::map_view::dim_tile(ui.painter(), Rect::from_center_size(r.center(), Vec2::splat(30.0)), i);
        if resp.clicked() && !selected && enabled {
            picked = Some(i);
        }
    }
    picked
}

/// One text row of a drop-down list: square, tinted when hovered or when it is the current choice,
/// like the rows of the world drop-down.
pub fn dropdown_item(
    ui: &mut Ui,
    selected: bool,
    last: bool,
    (hover, current): (Color32, Color32),
    text: &str,
) -> Response {
    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    // The last row follows the list's rounded bottom corners.
    let corners = egui::CornerRadius { nw: 0, ne: 0, sw: if last { 4 } else { 0 }, se: if last { 4 } else { 0 } };
    if resp.hovered() && selected {
        // The current choice keeps its color and only lightens.
        ui.painter().rect_filled(row, corners, current);
        ui.painter().rect_filled(row, corners, theme::wash(18));
    } else if resp.hovered() {
        ui.painter().rect_filled(row, corners, hover);
    } else if selected {
        ui.painter().rect_filled(row, corners, current);
    }
    ui.painter().text(
        row.left_center() + vec2(7.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        theme::regular(13.0),
        theme::text(),
    );
    resp
}

/// Edition / app tile: icon above a label. `attached` squares its bottom for a strip under it.
pub fn tile(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    size: Vec2,
    selected: bool,
    enabled: bool,
    attached: bool,
) -> Response {
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let fill = if selected {
        theme::accent()
    } else if resp.hovered() && enabled {
        theme::button_hover()
    } else {
        theme::tile()
    };
    let corners = if attached {
        egui::CornerRadius { nw: 5, ne: 5, sw: 0, se: 0 }
    } else {
        theme::radius(5)
    };
    ui.painter().rect_filled(rect, corners, fill);
    ui.painter().rect_stroke(
        rect,
        corners,
        Stroke::new(1.0_f32, theme::border()),
        StrokeKind::Inside,
    );
    let color = match (selected, enabled, resp.hovered()) {
        (true, _, _) => theme::strong(),
        (_, false, _) => theme::text_dim(),
        (_, _, true) => theme::strong(),
        _ => theme::text(),
    };
    let icon_size = (size.y * 0.32).min(42.0);
    let ir = Rect::from_center_size(
        egui::pos2(rect.center().x, rect.top() + size.y * 0.36),
        Vec2::splat(icon_size),
    );
    icons::paint(ui.painter(), ir, icon, color);
    let f = if selected {
        theme::bold(14.0)
    } else {
        theme::regular(14.0)
    };
    ui.painter().text(
        egui::pos2(rect.center().x, rect.top() + size.y * 0.72),
        Align2::CENTER_CENTER,
        label,
        f,
        color,
    );
    if enabled {
        resp.on_hover_cursor(CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Gray label chip followed by a number field (`X [  0]`).
pub fn labeled_number(
    ui: &mut Ui,
    label: &str,
    value: &mut f64,
    width: f32,
    speed: f64,
) -> Response {
    ui.horizontal(|ui| labeled_number_row(ui, label, value, width, speed))
        .inner
}

fn labeled_number_row(
    ui: &mut Ui,
    label: &str,
    value: &mut f64,
    width: f32,
    speed: f64,
) -> Response {
    ui.spacing_mut().item_spacing.x = 0.0;
    // Styled like the Locate bar: a lighter label cell beside a dark field with the number at
    // its right.
    let label_w = text_width(ui, label, &theme::regular(14.0)) + 20.0;
    let (row, _) = ui.allocate_exact_size(vec2(label_w + width, 28.0), Sense::hover());
    let cell = egui::Rect::from_min_size(row.min, vec2(label_w, 28.0));
    ui.painter().rect_filled(
        cell,
        egui::CornerRadius { nw: 5, sw: 5, ne: 0, se: 0 },
        theme::cell(),
    );
    ui.painter().rect_filled(
        egui::Rect::from_min_max(egui::pos2(cell.right(), row.top()), row.max),
        egui::CornerRadius { nw: 0, sw: 0, ne: 5, se: 5 },
        theme::well(),
    );
    ui.painter().text(
        cell.center(),
        Align2::CENTER_CENTER,
        label,
        theme::regular(14.0),
        theme::strong(),
    );
    let field = egui::Rect::from_min_max(
        egui::pos2(cell.right() + 4.0, row.top()),
        egui::pos2(row.right() - 8.0, row.bottom()),
    );
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(field)
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
        |ui| {
            let s = ui.style_mut();
            for w in [
                &mut s.visuals.widgets.inactive,
                &mut s.visuals.widgets.hovered,
                &mut s.visuals.widgets.active,
            ] {
                w.weak_bg_fill = Color32::TRANSPARENT;
                w.bg_fill = Color32::TRANSPARENT;
                w.bg_stroke = Stroke::NONE;
            }
            s.spacing.interact_size = vec2(1.0, 28.0);
            ui.add(egui::DragValue::new(value).speed(speed).max_decimals(2))
        },
    )
    .inner
}

/// A number field whose whole width, label cell included, is `total`.
pub fn labeled_field(ui: &mut Ui, label: &str, value: &mut f64, total: f32, speed: f64) -> Response {
    let label_w = text_width(ui, label, &theme::regular(14.0)) + 20.0;
    labeled_number(ui, label, value, (total - label_w).max(24.0), speed)
}

/// A small square icon button that lights up on hover, like the close crosses of tabs and toasts.
pub fn square_icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> Response {
    let k = theme::ui_scale();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(24.0 * k), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, theme::radius(4), theme::wash(28));
    }
    icons::paint(
        ui.painter(),
        rect.shrink(6.0 * k),
        icon,
        if resp.hovered() { theme::strong() } else { theme::text_muted() },
    );
    resp.on_hover_text(tooltip)
}

/// Small caps section label with a chevron (PLAYERS, MAPS, ALL FILES).
pub fn section_header(ui: &mut Ui, label: &str, open: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let icon = if open {
        Icon::ChevronDown
    } else {
        Icon::ChevronRight
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            egui::pos2(rect.left() + 10.0, rect.center().y),
            Vec2::splat(10.0),
        ),
        icon,
        theme::text_muted(),
    );
    let color = if resp.hovered() {
        theme::text()
    } else {
        theme::text_muted()
    };
    ui.painter().text(
        egui::pos2(rect.left() + 22.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        theme::regular(13.5),
        color,
    );
    resp
}

/// Converter step marker state.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Done,
    Current,
    Pending,
}

/// The INPUT → SAVE hexagon stepper; returns the step that was clicked, if any.
pub fn stepper(ui: &mut Ui, labels: &[&str], current: usize, reachable: usize, k: f32) -> Option<usize> {
    let spacing = 120.0 * k;
    let width = spacing * (labels.len() - 1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 80.0 * k), Sense::hover());
    let x0 = rect.center().x - width / 2.0;
    let y = rect.top() + 52.0 * k;
    let p = ui.painter();
    for i in 0..labels.len() - 1 {
        let done = i < current;
        let a = egui::pos2(x0 + i as f32 * spacing, y);
        let b = egui::pos2(x0 + (i + 1) as f32 * spacing, y);
        p.line_segment(
            [a, b],
            Stroke::new(5.0_f32 * k, if done { theme::accent() } else { theme::chip() }),
        );
    }
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let c = egui::pos2(x0 + i as f32 * spacing, y);
        let state = if i < current {
            StepState::Done
        } else if i == current {
            StepState::Current
        } else {
            StepState::Pending
        };
        // Only a step that can be gone to is a button: no hand or highlight on the others.
        let usable = i <= reachable && i != current;
        let resp = ui.interact(
            Rect::from_center_size(c, Vec2::splat(36.0 * k)),
            Id::new(("step", i)),
            if usable { Sense::click() } else { Sense::hover() },
        );
        if usable && resp.hovered() {
            // The step's own hexagon, a little larger.
            ui.painter().add(Shape::convex_polygon(
                icons::hexagon(c, 26.0 * k),
                theme::wash(28),
                Stroke::NONE,
            ));
        }
        if resp.clicked() && usable {
            clicked = Some(i);
        }
        let hex = icons::hexagon(c, 16.0 * k);
        let p = ui.painter();
        let image = match state {
            StepState::Done => "STEP_COMPLETE",
            StepState::Current => "STEP_IN_PROGRESS",
            StepState::Pending => "STEP_INCOMPLETE",
        };
        // The step artwork is green; on a page with another accent the hexagons are drawn instead.
        let drawn = theme::current_accent() == theme::Accent::Green
            && icons::image(
                p,
                Rect::from_center_size(c, vec2(34.0, 40.0) * k),
                image,
                Color32::WHITE,
            );
        match state {
            _ if drawn => {}
            StepState::Done => {
                p.add(Shape::convex_polygon(hex, theme::accent(), Stroke::NONE));
                // A tick cut out of the filled hexagon (the Check icon is green artwork).
                let at = |x: f32, y: f32| egui::pos2(c.x - 8.0 * k + x * 16.0 * k, c.y - 8.0 * k + y * 16.0 * k);
                p.add(Shape::line(
                    vec![at(0.2, 0.52), at(0.42, 0.72), at(0.8, 0.3)],
                    Stroke::new(2.5_f32 * k, theme::card()),
                ));
            }
            StepState::Current => {
                p.add(Shape::convex_polygon(
                    hex,
                    theme::card(),
                    Stroke::new(2.5_f32 * k, theme::accent()),
                ));
            }
            StepState::Pending => {
                p.add(Shape::convex_polygon(hex, theme::chip(), Stroke::NONE));
            }
        }
        let color = if state == StepState::Pending {
            theme::text_dim()
        } else {
            theme::accent()
        };
        p.text(
            egui::pos2(c.x, rect.top() + 18.0 * k),
            Align2::CENTER_CENTER,
            *label,
            theme::regular(14.0 * k),
            color,
        );
    }
    clicked
}

/// Thin progress bar.
pub fn progress_bar(ui: &mut Ui, fraction: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 12.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, theme::radius(6), theme::chip());
    if fraction > 0.0 {
        let r = Rect::from_min_size(
            rect.min,
            vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()),
        );
        ui.painter().rect_filled(r, theme::radius(6), theme::accent());
    }
}

/// Dark single-line text field.
pub fn text_field(ui: &mut Ui, value: &mut String, width: f32) -> Response {
    ui.add_sized(
        vec2(width, 30.0),
        egui::TextEdit::singleline(value)
            .background_color(theme::deep())
            .margin(egui::Margin::symmetric(8, 6)),
    )
}
