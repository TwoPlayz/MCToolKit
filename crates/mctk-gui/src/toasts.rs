//! The messages shown in the top-right corner ("toasts"). They stack, each unrolls on arrival and
//! rolls up after a few seconds. They are a little see-through and clicks pass through them;
//! with the pointer over one it all but fades away. Every page uses them.

use eframe::egui::{self, Align2, Color32, Sense, pos2, vec2};

use crate::theme;
/// One message in the corner, and when it appeared (egui time).
pub struct Toast {
    id: u64,
    text: String,
    since: f64,
    /// Frames drawn so far; the unrolling starts only once the toast is really on screen.
    frames: u32,
    /// Seconds to wait before unrolling, so the toasts it pushes down have made room first.
    delay: f64,
    /// Where it was drawn last frame (the pointer over it makes it fainter).
    rect: Option<egui::Rect>,
}

/// How long a message stays in the corner.
const TOAST_SECS: f64 = 3.5;
/// The widest a toast gets, and the room its text leaves for the margin and the X.
const TOAST_WIDTH: f32 = 320.0;
const PAD: f32 = 12.0 + 12.0;
/// Seconds the toast takes to unroll and to roll back up.
const OPEN_SECS: f64 = 0.45;
/// How long older toasts take to move down, before the new one starts to unroll.
const PUSH_SECS: f64 = 0.18;
/// Most messages shown at once; the oldest goes when another arrives.
const MAX_TOASTS: usize = 5;
const CLOSE_SECS: f64 = 0.35;

/// Whether a message reports something that went wrong, which is shown in red.
fn is_error(text: &str) -> bool {
    let t = text.to_lowercase();
    ["could not", "invalid", "fail", "error", "cannot", "can't", " not ", "no longer", "already"]
        .iter()
        .any(|w| t.contains(w))
}

/// Set from the settings: only messages about something that went wrong are shown.
static ONLY_WARNINGS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_only_warnings(on: bool) {
    ONLY_WARNINGS.store(on, std::sync::atomic::Ordering::Relaxed);
}

#[derive(Default)]
pub struct Toasts {
    list: Vec<Toast>,
    next: u64,
}

impl Toasts {
    /// Adds a message on top. Every message gets its own toast, even a repeat of one still
    /// showing: the new one pushes the others down.
    pub fn push(&mut self, ctx: &egui::Context, text: String) {
        if ONLY_WARNINGS.load(std::sync::atomic::Ordering::Relaxed) && !is_error(&text) {
            return;
        }
        let now = ctx.input(|i| i.time);
        let delay = if self.list.is_empty() { 0.0 } else { PUSH_SECS };
        self.list.push(Toast { id: self.next, text, since: now, frames: 0, delay, rect: None });
        self.next += 1;
        if self.list.len() > MAX_TOASTS {
            self.list.remove(0);
        }
    }

    /// Draws the messages; call once a frame.
    pub fn show(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);        if self.list.is_empty() {
            return;
        }
        // Asked outside `input`, which holds the context's lock.
        let pointer = ctx.input(|i| i.pointer.hover_pos());
        // Under the title bar.
        let mut top = crate::TITLE_BAR + 12.0;
        let mut animating = false;
        // It eases out at both ends: unrolled from the right edge, rolled back up the same way.
        let ease = |x: f64| 1.0 - (1.0 - x.clamp(0.0, 1.0)).powi(3);
        // The newest is at the top and pushes the older ones down.
        for t in self.list.iter_mut().rev() {
            let area_id = egui::Id::new(("editor-toast", t.id));
            // Clicks go through it to what is under it; with the pointer over it, it all but
            // fades away (to see what it covers) and waits (to be read when the pointer leaves).
            let over = pointer.is_some_and(|p| t.rect.is_some_and(|r| r.contains(p)));
            if over {
                t.since = t.since.max(now - (TOAST_SECS - 2.0)).min(now - OPEN_SECS);
            }
            // The first frames only lay the toast out (and may arrive late when the app was
            // idle), so its clock starts after them and the unrolling is always seen from the start.
            if t.frames < 2 {
                t.since = now;
                t.frames += 1;
                ctx.request_repaint();
            }
            let age = now - t.since;
            if age >= TOAST_SECS {
                continue;
            }
            animating |= age < t.delay + OPEN_SECS || TOAST_SECS - age < CLOSE_SECS;
            let open = ease((age - t.delay) / OPEN_SECS).min(ease((TOAST_SECS - age) / CLOSE_SECS)) as f32;
            let error = is_error(&t.text);
            let y = ctx.animate_value_with_time(egui::Id::new(("editor-toast-y", t.id)), top, PUSH_SECS as f32);
            let fade = if over { 0.25 } else { 0.85 };
            let shown = egui::Area::new(area_id)
                .interactable(false)
                // Under the tooltips, so a tooltip on a toast's own button shows above it.
                .order(egui::Order::Foreground)
                .anchor(Align2::RIGHT_TOP, vec2(-12.0, y))
                .show(ctx, |ui| {
                    let (fill, text_color) = if error { (Color32::from_rgb(0xC8, 0x3C, 0x3C), Color32::WHITE) } else { (theme::BLUE, Color32::WHITE) };
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let galley = ui.fonts(|f| {
                        f.layout(t.text.clone(), font, text_color, TOAST_WIDTH - PAD)
                    });
                    let height = (galley.size().y + 16.0).max(38.0);
                    // As wide as its message needs, up to a limit.
                    let width = (galley.size().x + PAD).max(150.0);
                    let (full, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
                    // The card itself grows leftward from its right edge, like a scroll being
                    // unrolled: its left edge, corners included, travels, and what it passes is
                    // uncovered. Nothing here is masked.
                    let card = egui::Rect::from_min_max(
                        pos2(full.right() - width * open, full.top()),
                        full.max,
                    );
                    ui.painter().rect_filled(card, theme::radius(6), fill.gamma_multiply(fade));
                    let inside = ui.painter().with_clip_rect(card);
                    inside.galley(
                        pos2(full.left() + 12.0, full.center().y - galley.size().y / 2.0),
                        galley,
                        text_color.gamma_multiply(fade),
                    );
                });
            // A new toast is drawn over the older ones it pushes down.
            if t.frames <= 2 {
                ctx.move_to_top(shown.response.layer_id);
            }
            t.rect = Some(shown.response.rect);
            top += shown.response.rect.height() + 8.0;
        }
        self.list
            .retain(|t| now - t.since < TOAST_SECS);
        if animating {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
