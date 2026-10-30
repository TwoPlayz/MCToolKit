//! Custom-painted controls in UMT's style.

use eframe::egui::{
    self, Align2, Color32, CursorIcon, Id, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui,
    Vec2, vec2,
};

use crate::icons::{self, Icon};
use crate::theme;

/// A rounded panel (`#232830` with a hairline border).
pub fn card<R>(ui: &mut Ui, margin: i8, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
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
pub const DISCORD_URL: &str = "https://discord.gg/y5mECmtban";

/// Opens the Discord invite in the default browser.
pub fn open_discord(ui: &Ui) {
    ui.ctx().open_url(egui::OpenUrl::new_tab(DISCORD_URL));
}

/// Grey button with an optional icon; `width` 0 sizes it to its content.
pub fn button(ui: &mut Ui, icon: Option<Icon>, text: &str, width: f32, enabled: bool) -> Response {
    let font = theme::regular(14.0);
    let content = text_width(ui, text, &font) + if icon.is_some() { 26.0 } else { 0.0 };
    let size = vec2(if width > 0.0 { width } else { content + 24.0 }, 32.0);
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click()
        } else {
            Sense::hover()
        },
    );
    let fill = if !enabled {
        theme::TILE
    } else if resp.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON
    };
    ui.painter().rect_filled(rect, theme::radius(4), fill);
    let color = if enabled {
        theme::TEXT
    } else {
        theme::TEXT_DIM
    };
    let start = rect.center().x - content / 2.0;
    if let Some(i) = icon {
        let cx = if text.is_empty() {
            rect.center().x
        } else {
            start + 9.0
        };
        let r = Rect::from_center_size(egui::pos2(cx, rect.center().y), Vec2::splat(18.0));
        icons::paint(ui.painter(), r, i, color);
    }
    let tx = if icon.is_some() { start + 26.0 } else { start };
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
        theme::BUTTON
    } else if resp.hovered() {
        theme::GREEN_HOVER
    } else {
        theme::GREEN
    };
    ui.painter().rect_filled(rect, theme::radius(5), fill);
    let color = if enabled {
        Color32::WHITE
    } else {
        theme::TEXT_DIM
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

/// Large grey button with big text (Start Over, Stop Conversion).
pub fn big_button(ui: &mut Ui, text: &str, size: Vec2) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let fill = if resp.hovered() {
        theme::BUTTON_HOVER
    } else {
        theme::BUTTON
    };
    ui.painter().rect_filled(rect, theme::radius(5), fill);
    ui.painter().rect_stroke(
        rect,
        theme::radius(5),
        Stroke::new(1.0_f32, theme::CHIP),
        StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        text,
        theme::regular(18.0),
        theme::TEXT,
    );
    resp
}

/// Small square icon-only button.
pub fn icon_button(ui: &mut Ui, icon: Icon, size: f32, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, theme::radius(4), theme::BUTTON_HOVER);
    }
    let color = if resp.hovered() {
        theme::TEXT
    } else {
        theme::TEXT_MUTED
    };
    icons::paint(ui.painter(), rect.shrink(size * 0.22), icon, color);
    if tooltip.is_empty() {
        resp
    } else {
        resp.on_hover_text(tooltip)
    }
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
        theme::CHIP
    } else {
        Color32::from_rgb(
            (0x3A as f32 + (0x43 - 0x3A) as f32 * t) as u8,
            (0x40 as f32 + (0xA9 - 0x40) as f32 * t) as u8,
            (0x49 as f32 + (0x75 - 0x49) as f32 * t) as u8,
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
            theme::TEXT_DIM
        },
    );
    let color = if enabled {
        theme::TEXT
    } else {
        theme::TEXT_DIM
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
    let font = theme::regular(13.5);
    // Segments fit their text; a fixed width is shared out in proportion to it.
    let mut widths: Vec<f32> = options
        .iter()
        .map(|o| text_width(ui, o, &font) + 22.0)
        .collect();
    if width > 0.0 {
        let natural: f32 = widths.iter().sum();
        widths.iter_mut().for_each(|w| *w *= width / natural);
    }
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(vec2(total, 30.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, theme::radius(4), theme::DEEP);
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
                .rect_filled(r.shrink(1.0), theme::radius(4), theme::CARD);
            ui.painter().rect_stroke(
                r.shrink(1.0),
                theme::radius(4),
                Stroke::new(1.2_f32, theme::BLUE),
                StrokeKind::Inside,
            );
        } else if resp.hovered() && enabled {
            ui.painter()
                .rect_filled(r.shrink(1.0), theme::radius(4), theme::TILE);
        }
        let color = match (sel, enabled) {
            (true, true) => Color32::WHITE,
            (_, false) => theme::TEXT_DIM,
            _ => theme::TEXT_MUTED,
        };
        let f = if sel { theme::bold(13.5) } else { font.clone() };
        ui.painter()
            .text(r.center(), Align2::CENTER_CENTER, *opt, f, color);
        x += w;
    }
    changed
}

/// Edition / app tile: icon above a label.
pub fn tile(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    size: Vec2,
    selected: bool,
    enabled: bool,
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
        theme::GREEN
    } else if resp.hovered() && enabled {
        theme::BUTTON_HOVER
    } else {
        theme::TILE
    };
    ui.painter().rect_filled(rect, theme::radius(5), fill);
    ui.painter().rect_stroke(
        rect,
        theme::radius(5),
        Stroke::new(1.0_f32, theme::BORDER),
        StrokeKind::Inside,
    );
    let color = match (selected, enabled, resp.hovered()) {
        (true, _, _) => Color32::WHITE,
        (_, false, _) => theme::TEXT_DIM,
        (_, _, true) => Color32::WHITE,
        _ => Color32::from_rgb(0xC8, 0xCD, 0xD4),
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

/// Grey label chip followed by a number field (`X [  0]`).
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
    let (lr, _) = ui.allocate_exact_size(
        vec2(text_width(ui, label, &theme::regular(13.0)) + 16.0, 28.0),
        Sense::hover(),
    );
    ui.painter().rect_filled(
        lr,
        egui::CornerRadius {
            nw: 4,
            sw: 4,
            ne: 0,
            se: 0,
        },
        theme::BUTTON,
    );
    ui.painter().text(
        lr.center(),
        Align2::CENTER_CENTER,
        label,
        theme::regular(13.0),
        theme::TEXT,
    );
    ui.add_sized(
        vec2(width, 28.0),
        egui::DragValue::new(value).speed(speed).max_decimals(2),
    )
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
        theme::TEXT_MUTED,
    );
    let color = if resp.hovered() {
        theme::TEXT
    } else {
        Color32::from_rgb(0x8C, 0x96, 0xA3)
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
pub fn stepper(ui: &mut Ui, labels: &[&str], current: usize, reachable: usize) -> Option<usize> {
    let spacing = 120.0;
    let width = spacing * (labels.len() - 1) as f32;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 80.0), Sense::hover());
    let x0 = rect.center().x - width / 2.0;
    let y = rect.top() + 52.0;
    let p = ui.painter();
    for i in 0..labels.len() - 1 {
        let done = i < current;
        let a = egui::pos2(x0 + i as f32 * spacing, y);
        let b = egui::pos2(x0 + (i + 1) as f32 * spacing, y);
        p.line_segment(
            [a, b],
            Stroke::new(5.0_f32, if done { theme::GREEN } else { theme::CHIP }),
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
        let resp = ui.interact(
            Rect::from_center_size(c, Vec2::splat(36.0)),
            Id::new(("step", i)),
            Sense::click(),
        );
        if resp.clicked() && i <= reachable && i != current {
            clicked = Some(i);
        }
        let hex = icons::hexagon(c, 16.0);
        let p = ui.painter();
        let image = match state {
            StepState::Done => "STEP_COMPLETE",
            StepState::Current => "STEP_IN_PROGRESS",
            StepState::Pending => "STEP_INCOMPLETE",
        };
        let drawn = icons::image(
            p,
            Rect::from_center_size(c, vec2(34.0, 40.0)),
            image,
            Color32::WHITE,
        );
        match state {
            _ if drawn => {}
            StepState::Done => {
                p.add(Shape::convex_polygon(hex, theme::GREEN, Stroke::NONE));
                icons::paint(
                    p,
                    Rect::from_center_size(c, Vec2::splat(16.0)),
                    Icon::Check,
                    theme::BG,
                );
            }
            StepState::Current => {
                p.add(Shape::convex_polygon(
                    hex,
                    theme::CARD,
                    Stroke::new(2.5_f32, theme::GREEN),
                ));
            }
            StepState::Pending => {
                p.add(Shape::convex_polygon(hex, theme::CHIP, Stroke::NONE));
            }
        }
        let color = if state == StepState::Pending {
            theme::TEXT_DIM
        } else {
            theme::GREEN
        };
        p.text(
            egui::pos2(c.x, rect.top() + 18.0),
            Align2::CENTER_CENTER,
            *label,
            theme::regular(14.0),
            color,
        );
    }
    clicked
}

/// A button that fires only after being held for `seconds` (the Pruner's safety catch).
pub fn hold_button(
    ui: &mut Ui,
    id: Id,
    title: &str,
    subtitle: &str,
    size: Vec2,
    seconds: f64,
    enabled: bool,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(
        size,
        if enabled {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        },
    );
    let now = ui.input(|i| i.time);
    let mut fired = false;
    let start: Option<f64> = ui.data(|d| d.get_temp(id));
    if enabled && resp.is_pointer_button_down_on() {
        let s = start.unwrap_or(now);
        if start.is_none() {
            ui.data_mut(|d| d.insert_temp(id, s));
        }
        if now - s >= seconds {
            fired = true;
            ui.data_mut(|d| d.remove::<f64>(id));
        } else {
            ui.ctx().request_repaint();
        }
    } else if start.is_some() {
        ui.data_mut(|d| d.remove::<f64>(id));
    }
    let progress = ui
        .data(|d| d.get_temp::<f64>(id))
        .map_or(0.0, |s| ((now - s) / seconds).clamp(0.0, 1.0)) as f32;
    let p = ui.painter();
    p.rect_filled(
        rect,
        theme::radius(5),
        if resp.hovered() && enabled {
            theme::BUTTON_HOVER
        } else {
            theme::CARD
        },
    );
    if progress > 0.0 {
        let fill = Rect::from_min_size(rect.min, vec2(rect.width() * progress, rect.height()));
        p.rect_filled(fill, theme::radius(5), theme::PRUNER_ACCENT);
    }
    p.rect_stroke(
        rect,
        theme::radius(5),
        Stroke::new(1.0_f32, theme::BORDER),
        StrokeKind::Inside,
    );
    let color = if enabled {
        Color32::WHITE
    } else {
        theme::TEXT_DIM
    };
    p.text(
        rect.center() - vec2(0.0, 9.0),
        Align2::CENTER_CENTER,
        title,
        theme::bold(17.0),
        color,
    );
    p.text(
        rect.center() + vec2(0.0, 11.0),
        Align2::CENTER_CENTER,
        subtitle,
        theme::bold(12.0),
        color,
    );
    fired
}

/// Thin progress bar.
pub fn progress_bar(ui: &mut Ui, fraction: f32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 12.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, theme::radius(6), theme::CHIP);
    if fraction > 0.0 {
        let r = Rect::from_min_size(
            rect.min,
            vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()),
        );
        ui.painter().rect_filled(r, theme::radius(6), theme::GREEN);
    }
}

/// Dark single-line text field.
pub fn text_field(ui: &mut Ui, value: &mut String, width: f32) -> Response {
    ui.add_sized(
        vec2(width, 30.0),
        egui::TextEdit::singleline(value)
            .background_color(theme::DEEP)
            .margin(egui::Margin::symmetric(8, 6)),
    )
}
