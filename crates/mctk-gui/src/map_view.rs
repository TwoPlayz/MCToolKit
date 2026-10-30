//! The world map used by the Chunk Locator and the Pruner: rulers, pan and zoom, a
//! chunk/block tooltip, the Locate bar, dimension buttons and the Slicer.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use eframe::egui::{
    self, Align2, Color32, Pos2, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2, pos2, vec2,
};

use crate::icons::{self, Icon};
use crate::jobs::Job;
use crate::theme;
use crate::widgets;
use crate::world::{self, Edition, MapImage, MapProgress};

const RULER: f32 = 18.0;
const DIMS: [&str; 3] = [
    "minecraft:overworld",
    "minecraft:the_nether",
    "minecraft:the_end",
];

pub struct MapView {
    pub path: PathBuf,
    pub edition: Edition,
    pub dimension: String,
    image: Option<(MapImage, egui::TextureHandle)>,
    job: Option<Job<MapImage>>,
    progress: Option<Arc<MapProgress>>,
    started: Option<Instant>,
    stream_dirty: bool,
    last_upload: Option<Instant>,
    error: Option<String>,
    /// Camera centre in block coordinates and scale in pixels per block.
    pub center: Vec2,
    pub zoom: f32,
    fitted: bool,
    chunk_mode: bool,
    locate: (f64, f64),
    slicer_open: bool,
    slicer: i32,
    applied_slicer: Option<i32>,
    pub selected: Option<(i32, i32)>,
}

/// Where the map was drawn this frame, for overlays and tools.
pub struct MapFrame {
    pub rect: Rect,
    pub center: Vec2,
    pub zoom: f32,
    pub response: Response,
    /// Block under the pointer.
    pub hover: Option<Vec2>,
}

impl MapFrame {
    pub fn to_screen(&self, b: Vec2) -> Pos2 {
        self.rect.center() + (b - self.center) * self.zoom
    }
    pub fn to_block(&self, p: Pos2) -> Vec2 {
        self.center + (p - self.rect.center()) / self.zoom
    }
}

fn dim_tile(p: &egui::Painter, r: Rect, dim: usize) {
    let (top, side, tint) = match dim {
        0 => (
            "block/grass_block_top.png",
            "block/grass_block_side.png",
            Color32::from_rgb(0x91, 0xBD, 0x59),
        ),
        1 => (
            "block/netherrack.png",
            "block/netherrack.png",
            Color32::WHITE,
        ),
        _ => ("block/end_stone.png", "block/end_stone.png", Color32::WHITE),
    };
    if icons::block(p, r, top, side, tint) {
        return;
    }
    let (top, side, dots): (Color32, Color32, Color32) = match dim {
        0 => (
            Color32::from_rgb(0x5E, 0x9D, 0x3A),
            Color32::from_rgb(0x86, 0x60, 0x43),
            Color32::from_rgb(0x6D, 0x4A, 0x30),
        ),
        1 => (
            Color32::from_rgb(0x7A, 0x2A, 0x2A),
            Color32::from_rgb(0x6A, 0x22, 0x24),
            Color32::from_rgb(0x9A, 0x3A, 0x3A),
        ),
        _ => (
            Color32::from_rgb(0xE6, 0xE2, 0xA8),
            Color32::from_rgb(0xD7, 0xD3, 0x95),
            Color32::from_rgb(0xBE, 0xB9, 0x7C),
        ),
    };
    // A small isometric block of our own drawing.
    let c = r.center();
    let s = r.width() * 0.42;
    let t = [
        pos2(c.x, c.y - s * 0.9),
        pos2(c.x + s, c.y - s * 0.4),
        pos2(c.x, c.y + s * 0.1),
        pos2(c.x - s, c.y - s * 0.4),
    ];
    let l = [
        pos2(c.x - s, c.y - s * 0.4),
        pos2(c.x, c.y + s * 0.1),
        pos2(c.x, c.y + s),
        pos2(c.x - s, c.y + s * 0.5),
    ];
    let rr = [
        pos2(c.x, c.y + s * 0.1),
        pos2(c.x + s, c.y - s * 0.4),
        pos2(c.x + s, c.y + s * 0.5),
        pos2(c.x, c.y + s),
    ];
    p.add(egui::Shape::convex_polygon(t.to_vec(), top, Stroke::NONE));
    p.add(egui::Shape::convex_polygon(l.to_vec(), side, Stroke::NONE));
    p.add(egui::Shape::convex_polygon(
        rr.to_vec(),
        side.gamma_multiply(0.8),
        Stroke::NONE,
    ));
    for (dx, dy) in [(-0.4, 0.4), (0.3, 0.55), (-0.15, 0.75), (0.55, 0.2)] {
        p.circle_filled(pos2(c.x + dx * s, c.y + dy * s), s * 0.07, dots);
    }
}

impl MapView {
    pub fn new(path: PathBuf, edition: Edition) -> Self {
        MapView {
            path,
            edition,
            dimension: DIMS[0].to_string(),
            image: None,
            job: None,
            progress: None,
            started: None,
            stream_dirty: false,
            last_upload: None,
            error: None,
            center: Vec2::ZERO,
            zoom: 1.0,
            fitted: false,
            chunk_mode: false,
            locate: (0.0, 0.0),
            slicer_open: false,
            slicer: 320,
            applied_slicer: None,
            selected: None,
        }
    }

    /// Re-renders (after the world changed on disk).
    pub fn reload(&mut self, path: PathBuf) {
        self.path = path;
        self.invalidate();
    }

    /// Drops the picture and stops any render in flight, so the next frame starts a new one.
    fn invalidate(&mut self) {
        if let Some(p) = self.progress.take() {
            p.cancel();
        }
        self.image = None;
        self.job = None;
        self.error = None;
    }

    /// While a render runs, paints finished chunks into a provisional picture (uploaded at
    /// most ten times a second) so the map fills in instead of appearing all at once.
    fn stream(&mut self, ctx: &egui::Context) {
        let Some(p) = self.progress.clone() else {
            return;
        };
        let Some((min_x, min_z, width, height)) = *p.layout.lock().expect("map layout") else {
            return;
        };
        let fresh = std::mem::take(&mut *p.fresh.lock().expect("map tiles"));
        if self.image.is_none() {
            let img = MapImage {
                min_x,
                min_z,
                width,
                height,
                rgba: vec![0; width * height * 4],
            };
            let color = egui::ColorImage::from_rgba_unmultiplied([width, height], &img.rgba);
            let tex = ctx.load_texture("world-map", color, egui::TextureOptions::NEAREST);
            self.image = Some((img, tex));
        }
        let Some((img, tex)) = &mut self.image else {
            return;
        };
        for (cx, cz, px) in &fresh {
            let (gx, gz) = ((cx - img.min_x) as usize * 16, (cz - img.min_z) as usize * 16);
            for row in 0..16 {
                let dst = ((gz + row) * img.width + gx) * 4;
                img.rgba[dst..dst + 64].copy_from_slice(&px[row * 64..row * 64 + 64]);
            }
        }
        self.stream_dirty |= !fresh.is_empty();
        if self.stream_dirty && self.last_upload.is_none_or(|t| t.elapsed().as_millis() >= 100) {
            let color = egui::ColorImage::from_rgba_unmultiplied([img.width, img.height], &img.rgba);
            tex.set(color, egui::TextureOptions::NEAREST);
            self.stream_dirty = false;
            self.last_upload = Some(Instant::now());
        }
    }

    /// "Rendering Map" card with a progress bar, drawn while the picture is being made.
    fn loading_card(&self, ui: &mut Ui, area: Rect) {
        let card = vec2(420.0f32.min(area.width() - 24.0).max(200.0), 120.0);
        let rect = Rect::from_center_size(area.center(), card);
        let (fraction, done, total) = self.progress.as_ref().map_or((None, 0, 0), |p| {
            (
                p.fraction(),
                p.done.load(Ordering::Relaxed),
                p.total.load(Ordering::Relaxed),
            )
        });
        let elapsed = self.started.map_or(0.0, |s| s.elapsed().as_secs_f32());
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            widgets::card(ui, 19, |ui| {
                ui.set_width(card.x - 40.0);
                ui.vertical_centered(|ui| {
                    let (headline, detail) = match fraction {
                        Some(_) => (
                            "Rendering Map".to_string(),
                            format!("{done} of {total} chunks \u{b7} {elapsed:.0}s"),
                        ),
                        None => (
                            "Reading World".to_string(),
                            format!("Scanning chunks \u{b7} {elapsed:.0}s"),
                        ),
                    };
                    ui.label(RichText::new(headline).size(16.0).color(theme::TEXT));
                    ui.add_space(8.0);
                    // Until the chunk count is known, a bar that keeps filling to show activity.
                    let shown = fraction.unwrap_or_else(|| (elapsed * 0.6).fract());
                    widgets::progress_bar(ui, shown, card.x - 40.0);
                    ui.add_space(8.0);
                    ui.label(RichText::new(detail).size(13.0).color(theme::TEXT_MUTED));
                });
            });
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// Draws the map and handles pan/zoom. `primary_pans` is false while a drawing tool
    /// owns the left button (the right or middle button still pans).
    pub fn show_map(&mut self, ui: &mut Ui, primary_pans: bool) -> MapFrame {
        let ctx = ui.ctx().clone();
        if let Some(r) = self.job.as_ref().and_then(Job::take) {
            self.job = None;
            match r {
                Ok(img) => {
                    let color = egui::ColorImage::from_rgba_unmultiplied(
                        [img.width, img.height],
                        &img.rgba,
                    );
                    let tex = ctx.load_texture("world-map", color, egui::TextureOptions::NEAREST);
                    self.image = Some((img, tex));
                    self.error = None;
                }
                Err(e) => {
                    self.image = None;
                    self.error = Some(e);
                }
            }
        }
        if self.job.is_some() {
            self.stream(&ctx);
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        if self.image.is_none() && self.job.is_none() && self.error.is_none() {
            let (p, e, d, s) = (
                self.path.clone(),
                self.edition,
                self.dimension.clone(),
                self.applied_slicer,
            );
            let progress = Arc::new(MapProgress::default());
            let shared = Arc::clone(&progress);
            self.job = Some(Job::spawn("Rendering map", &ctx, move || {
                world::render_map_with(&p, e, &d, s, &shared)
            }));
            self.progress = Some(progress);
            self.started = Some(Instant::now());
        }

        let outer = ui.available_rect_before_wrap();
        let rect = Rect::from_min_max(outer.min + vec2(RULER, RULER), outer.max);
        let response = ui.allocate_rect(outer, Sense::click_and_drag());
        let painter = ui.painter_at(outer);
        painter.rect_filled(outer, 0.0, theme::BG);

        if let Some((img, tex)) = &self.image {
            if !self.fitted {
                let (w, h) = (img.width as f32, img.height as f32);
                self.zoom = (rect.width() / w).min(rect.height() / h).clamp(0.05, 8.0);
                self.center = vec2(
                    img.min_x as f32 * 16.0 + w / 2.0,
                    img.min_z as f32 * 16.0 + h / 2.0,
                );
                self.fitted = true;
            }
            let frame = MapFrame {
                rect,
                center: self.center,
                zoom: self.zoom,
                response: response.clone(),
                hover: None,
            };
            let min = frame.to_screen(vec2(img.min_x as f32 * 16.0, img.min_z as f32 * 16.0));
            let size = vec2(img.width as f32, img.height as f32) * self.zoom;
            painter.with_clip_rect(rect).image(
                tex.id(),
                Rect::from_min_size(min, size),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else if let Some(e) = &self.error {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                format!("Could not render the map: {e}"),
                theme::regular(15.0),
                theme::TEXT_MUTED,
            );
        } else {
            self.loading_card(ui, rect);
        }

        // A small progress note while the picture is still filling in.
        if self.job.is_some() && self.image.is_some() {
            let pct = self
                .progress
                .as_ref()
                .and_then(|p| p.fraction())
                .map_or(0, |f| (f * 100.0) as u32);
            let r = Rect::from_center_size(
                pos2(rect.center().x, rect.top() + 22.0),
                vec2(170.0, 26.0),
            );
            painter.rect_filled(r, theme::radius(4), theme::CARD);
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                format!("Rendering map\u{2026} {pct}%"),
                theme::regular(13.0),
                theme::TEXT_MUTED,
            );
        }

        // Zoom around the pointer and pan.
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if let Some(p) = ui.input(|i| i.pointer.hover_pos()) {
                    let before = self.center + (p - rect.center()) / self.zoom;
                    self.zoom = (self.zoom * (scroll * 0.002).exp()).clamp(0.03, 24.0);
                    self.center = before - (p - rect.center()) / self.zoom;
                }
            }
        }
        let pans = (primary_pans && response.dragged_by(egui::PointerButton::Primary))
            || response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle);
        if pans {
            self.center -= response.drag_delta() / self.zoom;
        }
        let mut frame = MapFrame {
            rect,
            center: self.center,
            zoom: self.zoom,
            response,
            hover: None,
        };
        frame.hover = frame
            .response
            .hover_pos()
            .filter(|p| rect.contains(*p))
            .map(|p| frame.to_block(p));
        if primary_pans && frame.response.clicked() {
            if let Some(b) = frame.hover {
                self.selected = Some(((b.x / 16.0).floor() as i32, (b.y / 16.0).floor() as i32));
            }
        }
        frame
    }

    /// Rulers, selection, tooltip and the floating controls, drawn over any overlays.
    pub fn show_chrome(&mut self, ui: &mut Ui, frame: &MapFrame) {
        let outer = Rect::from_min_max(frame.rect.min - vec2(RULER, RULER), frame.rect.max);
        let p = ui.painter_at(outer);
        if let Some((cx, cz)) = self.selected {
            let a = frame.to_screen(vec2(cx as f32 * 16.0, cz as f32 * 16.0));
            let b = frame.to_screen(vec2(cx as f32 * 16.0 + 16.0, cz as f32 * 16.0 + 16.0));
            p.with_clip_rect(frame.rect).rect_stroke(
                Rect::from_min_max(a, b),
                0.0,
                Stroke::new(2.0_f32, Color32::WHITE),
                StrokeKind::Outside,
            );
        }
        // Rulers with labels every power-of-two step that leaves at least 60 px.
        let top = Rect::from_min_max(
            pos2(frame.rect.left(), outer.top()),
            pos2(outer.right(), frame.rect.top()),
        );
        let left = Rect::from_min_max(
            pos2(outer.left(), frame.rect.top()),
            pos2(frame.rect.left(), outer.bottom()),
        );
        p.rect_filled(top, 0.0, theme::DEEP);
        p.rect_filled(left, 0.0, theme::DEEP);
        p.rect_filled(
            Rect::from_min_max(outer.min, frame.rect.min),
            0.0,
            theme::DEEP,
        );
        let step = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192]
            .into_iter()
            .find(|s| *s as f32 * frame.zoom >= 60.0)
            .unwrap_or(16384) as f32;
        let b0 = frame.to_block(frame.rect.min);
        let b1 = frame.to_block(frame.rect.max);
        let mut x = (b0.x / step).floor() * step;
        while x <= b1.x {
            let sx = frame.to_screen(vec2(x, 0.0)).x;
            p.line_segment(
                [pos2(sx, top.bottom() - 6.0), pos2(sx, top.bottom())],
                Stroke::new(1.0_f32, theme::TEXT_DIM),
            );
            p.text(
                pos2(sx + 3.0, top.center().y),
                Align2::LEFT_CENTER,
                format!("{}", x as i64),
                theme::regular(11.0),
                theme::TEXT_MUTED,
            );
            x += step;
        }
        let mut z = (b0.y / step).floor() * step;
        while z <= b1.y {
            let sy = frame.to_screen(vec2(0.0, z)).y;
            p.line_segment(
                [pos2(left.right() - 6.0, sy), pos2(left.right(), sy)],
                Stroke::new(1.0_f32, theme::TEXT_DIM),
            );
            p.text(
                pos2(left.center().x, sy + 3.0),
                Align2::LEFT_TOP,
                format!("{}", z as i64),
                egui::FontId::proportional(10.5),
                theme::TEXT_MUTED,
            );
            z += step;
        }
        if let Some(h) = frame.hover {
            // Ruler markers at the pointer.
            let s = frame.to_screen(h);
            p.line_segment(
                [pos2(s.x, top.top()), pos2(s.x, top.bottom())],
                Stroke::new(1.5_f32, theme::BLUE),
            );
            p.line_segment(
                [pos2(left.left(), s.y), pos2(left.right(), s.y)],
                Stroke::new(1.5_f32, theme::BLUE),
            );
            let (bx, bz) = (h.x.floor() as i64, h.y.floor() as i64);
            let text = format!(
                "Chunk:  {}, {}\nBlock:  {}, {}",
                bx.div_euclid(16),
                bz.div_euclid(16),
                bx,
                bz
            );
            let galley = p.layout_no_wrap(text, theme::regular(13.0), theme::TEXT);
            let r = Rect::from_min_size(s + vec2(16.0, 12.0), galley.size() + vec2(16.0, 10.0));
            p.rect_filled(
                r,
                theme::radius(4),
                Color32::from_rgba_unmultiplied(0x16, 0x18, 0x1E, 235),
            );
            p.galley(r.min + vec2(8.0, 5.0), galley, theme::TEXT);
        }

        // Locate bar.
        let bar = Rect::from_min_size(frame.rect.min + vec2(22.0, 22.0), vec2(384.0, 44.0));
        ui.painter().rect_filled(bar, theme::radius(5), theme::DEEP);
        ui.scope_builder(
            UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 8.0))),
            |ui| {
                ui.horizontal_centered(|ui| {
                    let mode = if self.chunk_mode { "Chunk" } else { "Block" };
                    if ui
                        .add(egui::Button::new(format!("○  {mode}")).min_size(vec2(78.0, 28.0)))
                        .clicked()
                    {
                        self.chunk_mode = !self.chunk_mode;
                    }
                    ui.label("X");
                    ui.add_sized(
                        vec2(70.0, 28.0),
                        egui::DragValue::new(&mut self.locate.0).speed(1.0),
                    );
                    ui.label("Z");
                    ui.add_sized(
                        vec2(70.0, 28.0),
                        egui::DragValue::new(&mut self.locate.1).speed(1.0),
                    );
                    if crate::widgets::button(ui, Some(Icon::Search), "Locate", 0.0, true).clicked()
                    {
                        let k = if self.chunk_mode { 16.0 } else { 1.0 };
                        self.center = vec2(
                            self.locate.0 as f32 * k + if self.chunk_mode { 8.0 } else { 0.5 },
                            self.locate.1 as f32 * k + if self.chunk_mode { 8.0 } else { 0.5 },
                        );
                        self.zoom = self.zoom.max(2.0);
                        if self.chunk_mode {
                            self.selected = Some((self.locate.0 as i32, self.locate.1 as i32));
                        }
                    }
                });
            },
        );

        // Dimension buttons and Slicer.
        let mut slicer_rect = Rect::from_min_size(
            pos2(frame.rect.right() - 118.0, frame.rect.top() + 22.0),
            vec2(96.0, 44.0),
        );
        let mut dims_rect = Rect::from_min_size(
            pos2(slicer_rect.left() - 162.0, slicer_rect.top()),
            vec2(150.0, 44.0),
        );
        // Too narrow for one row (small window or a large scale factor): move the
        // dimension buttons and Slicer below the Locate bar instead of over it.
        if dims_rect.left() < bar.right() + 12.0 {
            slicer_rect = slicer_rect.translate(vec2(0.0, 54.0));
            dims_rect = dims_rect.translate(vec2(0.0, 54.0));
        }
        ui.painter()
            .rect_filled(dims_rect, theme::radius(5), theme::DEEP);
        for (i, d) in DIMS.iter().enumerate() {
            let r = Rect::from_min_size(
                dims_rect.min + vec2(4.0 + i as f32 * 48.0, 4.0),
                vec2(46.0, 36.0),
            );
            let resp = ui
                .interact(r, ui.id().with(("dim", i)), Sense::click())
                .on_hover_text(match i {
                    0 => "Overworld",
                    1 => "Nether",
                    _ => "The End",
                });
            let selected = self.dimension == *d;
            if selected {
                ui.painter().rect_filled(r, theme::radius(4), theme::CARD);
                ui.painter().rect_stroke(
                    r,
                    theme::radius(4),
                    Stroke::new(1.2_f32, theme::BLUE),
                    StrokeKind::Inside,
                );
            }
            dim_tile(
                ui.painter(),
                Rect::from_center_size(r.center(), Vec2::splat(30.0)),
                i,
            );
            if resp.clicked() && !selected {
                self.dimension = d.to_string();
                self.invalidate();
                self.error = None;
                self.fitted = false;
                self.selected = None;
            }
        }
        let resp = ui.interact(slicer_rect, ui.id().with("slicer"), Sense::click());
        ui.painter().rect_filled(
            slicer_rect,
            theme::radius(5),
            if resp.hovered() || self.slicer_open {
                theme::BUTTON_HOVER
            } else {
                theme::DEEP
            },
        );
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                slicer_rect.left_center() + vec2(22.0, 0.0),
                Vec2::splat(22.0),
            ),
            Icon::Layers,
            theme::TEXT,
        );
        ui.painter().text(
            slicer_rect.left_center() + vec2(40.0, 0.0),
            Align2::LEFT_CENTER,
            "Slicer",
            theme::regular(14.0),
            theme::TEXT,
        );
        if resp.clicked() {
            self.slicer_open = !self.slicer_open;
        }
        if self.slicer_open {
            let r = Rect::from_min_size(
                pos2(slicer_rect.left(), slicer_rect.bottom() + 8.0),
                vec2(96.0, 260.0),
            );
            ui.painter().rect_filled(r, theme::radius(5), theme::DEEP);
            ui.scope_builder(UiBuilder::new().max_rect(r.shrink(10.0)), |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(format!("Y ≤ {}", self.slicer)).size(12.5));
                    let resp = ui.add(
                        egui::Slider::new(&mut self.slicer, -64..=320)
                            .vertical()
                            .show_value(false),
                    );
                    if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                        self.applied_slicer = (self.slicer < 320).then_some(self.slicer);
                        self.invalidate();
                        self.error = None;
                    }
                });
            });
        }
        if self.job.is_some() {
            ui.painter().text(
                frame.rect.right_bottom() - vec2(12.0, 10.0),
                Align2::RIGHT_BOTTOM,
                "Rendering…",
                theme::regular(12.0),
                theme::TEXT_MUTED,
            );
        }
    }
}
