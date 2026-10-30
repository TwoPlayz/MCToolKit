//! The Pruner page: draw layers on the map, preview the chunks that will be removed
//! (red hatching), then hold Prune for five seconds.

use std::path::Path;

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2, vec2};
use mctk_anvil::ChunkPos;
use mctk_prune::{Comparison, Layer, Selection};

use crate::editor::{LEFT_PANEL, edition_chooser};
use crate::icons::{self, Icon};
use crate::jobs::Job;
use crate::map_view::MapFrame;
use crate::settings::Settings;
use crate::theme;
use crate::widgets;
use crate::workspace::Workspace;
use crate::world_picker::UiEdition;

/// A layer in chunk units, as UMT shows them.
#[derive(Debug, Clone, PartialEq)]
enum PLayer {
    Box { x: f64, z: f64, w: f64, h: f64 },
    Circle { x: f64, z: f64, r: f64 },
    Split { x: f64, z: f64, angle: f64 },
    Polygon { points: Vec<(f64, f64)> },
    Time { cmp: usize, ticks: f64 },
}

const COMPARISONS: [(&str, Comparison); 5] = [
    ("<", Comparison::Less),
    ("≤", Comparison::LessOrEqual),
    ("=", Comparison::Equal),
    ("≥", Comparison::GreaterOrEqual),
    (">", Comparison::Greater),
];

impl PLayer {
    fn title(&self) -> &'static str {
        match self {
            PLayer::Box { .. } => "BOX",
            PLayer::Circle { .. } => "CIRCLE",
            PLayer::Split { .. } => "SPLIT",
            PLayer::Polygon { .. } => "POLYGON",
            PLayer::Time { .. } => "TIME",
        }
    }
    fn icon(&self) -> Icon {
        match self {
            PLayer::Box { .. } => Icon::ShapeBox,
            PLayer::Circle { .. } => Icon::ShapeCircle,
            PLayer::Split { .. } => Icon::ShapeSplit,
            PLayer::Polygon { .. } => Icon::ShapePolygon,
            PLayer::Time { .. } => Icon::Time,
        }
    }
    fn to_layer(&self) -> Option<Layer> {
        let b = |v: f64| v * 16.0;
        Some(match self {
            PLayer::Box { x, z, w, h } => Layer::Box {
                x: b(*x),
                z: b(*z),
                width: b(*w),
                height: b(*h),
            },
            PLayer::Circle { x, z, r } => Layer::Circle {
                x: b(*x),
                z: b(*z),
                radius: b(*r),
            },
            PLayer::Split { x, z, angle } => Layer::Split {
                x: b(*x),
                z: b(*z),
                angle: *angle,
            },
            PLayer::Polygon { points } if points.len() >= 3 => Layer::Polygon {
                points: points.iter().map(|(x, z)| (b(*x), b(*z))).collect(),
            },
            PLayer::Polygon { .. } => return None,
            PLayer::Time { cmp, ticks } => Layer::Time {
                comparison: COMPARISONS[*cmp].1,
                ticks: *ticks as i64,
            },
        })
    }
    /// Handle positions (chunk units) for dragging.
    fn handles(&self) -> Vec<(f64, f64)> {
        match self {
            PLayer::Box { x, z, w, h } => vec![(*x, *z), (x + w, *z), (x + w, z + h), (*x, z + h)],
            PLayer::Circle { x, z, r } => vec![
                (*x, *z),
                (
                    x + r * std::f64::consts::FRAC_1_SQRT_2,
                    z + r * std::f64::consts::FRAC_1_SQRT_2,
                ),
            ],
            PLayer::Split { x, z, angle } => {
                let a = angle.to_radians();
                vec![(*x, *z), (x + 6.0 * a.cos(), z + 6.0 * a.sin())]
            }
            PLayer::Polygon { points } => points.clone(),
            PLayer::Time { .. } => Vec::new(),
        }
    }
    fn drag_handle(&mut self, i: usize, to: (f64, f64)) {
        match self {
            PLayer::Box { x, z, w, h } => {
                let (ax, az) = match i {
                    0 => (*x + *w, *z + *h),
                    1 => (*x, *z + *h),
                    2 => (*x, *z),
                    _ => (*x + *w, *z),
                };
                let (tx, tz) = (to.0.round(), to.1.round());
                *x = ax.min(tx);
                *z = az.min(tz);
                *w = (ax - tx).abs();
                *h = (az - tz).abs();
            }
            PLayer::Circle { x, z, r } => {
                if i == 0 {
                    *x = to.0;
                    *z = to.1;
                } else {
                    *r = ((to.0 - *x).powi(2) + (to.1 - *z).powi(2)).sqrt();
                }
            }
            PLayer::Split { x, z, angle } => {
                if i == 0 {
                    *x = to.0;
                    *z = to.1;
                } else {
                    *angle = (to.1 - *z).atan2(to.0 - *x).to_degrees().rem_euclid(360.0);
                }
            }
            PLayer::Polygon { points } => {
                if let Some(p) = points.get_mut(i) {
                    *p = to;
                }
            }
            PLayer::Time { .. } => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Box,
    Circle,
    Polygon,
    Split,
}

#[derive(Default)]
pub struct PrunerPage {
    pub ws: Option<Workspace>,
    tool: Option<Tool>,
    layers: Vec<PLayer>,
    selected: Option<usize>,
    inverted: bool,
    drawing: Option<((f64, f64), (f64, f64))>,
    polygon: Vec<(f64, f64)>,
    dragging: Option<(usize, usize)>,
    job: Option<Job<std::path::PathBuf>>,
    confirm_close: bool,
    pub status: String,
}

pub enum PrunerRequest {
    None,
    PickWorld(UiEdition),
}

fn selection(layers: &[PLayer], inverted: bool) -> Selection {
    Selection {
        layers: layers.iter().filter_map(PLayer::to_layer).collect(),
        inverted,
    }
}

/// Diagonal hatch lines (direction (1, 1)) clipped to `r`, spaced `s` px in screen space.
fn hatch(p: &egui::Painter, r: Rect, s: f32, stroke: Stroke) {
    let (x0, x1, y0, y1) = (r.left(), r.right(), r.top(), r.bottom());
    let c0 = ((x0 - y1) / s).floor() as i64;
    let c1 = ((x1 - y0) / s).ceil() as i64;
    for k in c0..=c1 {
        let c = k as f32 * s; // line: x - y = c
        let xa = x0.max(y0 + c);
        let xb = x1.min(y1 + c);
        if xa < xb {
            p.line_segment([pos2(xa, xa - c), pos2(xb, xb - c)], stroke);
        }
    }
}

impl PrunerPage {
    pub fn open_world(&mut self, path: &Path, edition: UiEdition, settings: &Settings) {
        match Workspace::open(path, edition, settings, false, false) {
            Ok(ws) => {
                self.status = format!("Opened {}", ws.name);
                self.ws = Some(ws);
                self.layers.clear();
                self.selected = None;
            }
            Err(e) => self.status = format!("Could not open the world: {e}"),
        }
    }

    fn prune(&mut self, ctx: &egui::Context, settings: &Settings) {
        let Some(ws) = &self.ws else { return };
        let sel = selection(&self.layers, self.inverted);
        let src = ws.path.clone();
        let edition = ws.edition;
        let dim = ws.map.dimension.clone();
        let out = match settings.new_workspace("PRUNER", &ws.name) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Could not create the working folder: {e}");
                return;
            }
        };
        self.job = Some(Job::spawn("Pruning", ctx, move || {
            let java_dim = match dim.as_str() {
                "minecraft:the_nether" => -1,
                "minecraft:the_end" => 1,
                _ => 0,
            };
            let result = match edition {
                UiEdition::Java => {
                    mctk_prune::prune_java(&src, &out, &sel, Some(java_dim)).map(|_| out.clone())
                }
                UiEdition::Bedrock => {
                    let bedrock_dim = match java_dim {
                        -1 => 1,
                        1 => 2,
                        _ => 0,
                    };
                    mctk_prune::prune_bedrock_world(&src, &out, &sel, Some(bedrock_dim))
                        .map(|_| out.clone())
                }
                _ => (|| {
                    let mut save = mctk_console::ConsoleSave::open(&src)?;
                    mctk_prune::prune_console(&mut save, &sel, Some(java_dim))?;
                    // A PS3 save folder becomes a folder, an Xbox 360 save a file, of the same name.
                    let target = out.join(src.file_name().unwrap_or_default());
                    mctk_prune::save_console_copy(&save, &target)?;
                    Ok(target)
                })(),
            };
            result.map_err(|e: mctk_prune::Error| e.to_string())
        }));
    }

    pub fn ui(&mut self, ui: &mut Ui, settings: &mut Settings) -> PrunerRequest {
        let mut request = PrunerRequest::None;
        if let Some(r) = self.job.as_ref().and_then(Job::take) {
            self.job = None;
            match r {
                Ok(new_path) => {
                    if let Some(ws) = &mut self.ws {
                        ws.path = new_path.clone();
                        ws.copied = true;
                        ws.world_changed = true;
                        ws.map.reload(new_path);
                    }
                    self.layers.clear();
                    self.selected = None;
                    self.status = "Pruned. Save to write the result back to the world.".into();
                }
                Err(e) => self.status = format!("Prune failed: {e}"),
            }
        }
        if self.ws.is_none() {
            if let Some(e) = edition_chooser(ui, "Select a World to Prune", None) {
                request = PrunerRequest::PickWorld(e);
            }
            return request;
        }
        let full = ui.max_rect();
        self.left_panel(ui, settings);
        let area = Rect::from_min_max(pos2(full.left() + LEFT_PANEL, full.top()), full.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
        self.map(&mut child);
        if !self.status.is_empty() {
            ui.painter().text(
                area.left_bottom() + vec2(30.0, -12.0),
                Align2::LEFT_BOTTOM,
                &self.status,
                theme::regular(13.0),
                Color32::WHITE,
            );
        }
        request
    }

    fn left_panel(&mut self, ui: &mut Ui, settings: &mut Settings) {
        let full = ui.max_rect();
        let panel = Rect::from_min_size(full.min, vec2(LEFT_PANEL, full.height()));
        ui.painter().rect_filled(panel, 0.0, theme::BG);
        ui.painter().line_segment(
            [panel.right_top(), panel.right_bottom()],
            Stroke::new(1.0_f32, theme::BORDER),
        );
        let mut child =
            ui.new_child(egui::UiBuilder::new().max_rect(panel.shrink2(vec2(6.0, 6.0))));
        let ui = &mut child;
        let Some(ws) = &self.ws else { return };
        let (name, icon, dirty, bedrock) = (
            ws.name.clone(),
            ws.edition.icon(),
            ws.dirty(),
            ws.edition == UiEdition::Bedrock,
        );
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(LEFT_PANEL - 50.0, 30.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, theme::radius(4), theme::BUTTON);
            icons::paint(
                ui.painter(),
                Rect::from_center_size(rect.left_center() + vec2(14.0, 0.0), Vec2::splat(16.0)),
                icon,
                theme::TEXT_MUTED,
            );
            ui.painter().text(
                rect.left_center() + vec2(28.0, 0.0),
                Align2::LEFT_CENTER,
                &name,
                theme::regular(14.0),
                theme::TEXT,
            );
            if widgets::button(ui, Some(Icon::Plus), "", 32.0, true)
                .on_hover_text("Open another world")
                .clicked()
            {
                if dirty {
                    self.confirm_close = true;
                } else {
                    self.ws = None;
                }
            }
        });
        ui.add_space(4.0);
        let half = (LEFT_PANEL - 20.0) / 2.0;
        ui.horizontal(|ui| {
            if widgets::button(ui, Some(Icon::Close), "Close", half, true).clicked() {
                if dirty {
                    self.confirm_close = true;
                } else {
                    self.ws = None;
                }
            }
            if widgets::button(
                ui,
                Some(Icon::Save),
                if dirty { "Save*" } else { "Save" },
                half,
                dirty && self.job.is_none(),
            )
            .clicked()
            {
                if let Some(ws) = &mut self.ws {
                    self.status = match ws.sync_back() {
                        Ok(()) => {
                            ws.world_changed = false;
                            format!(
                                "Saved {} (the original was backed up as _MCTBackup)",
                                ws.name
                            )
                        }
                        Err(e) => format!("Save failed: {e}"),
                    };
                }
            }
        });
        if self.confirm_close {
            let mut answer = None;
            egui::Modal::new(egui::Id::new("prune-confirm-close")).show(ui.ctx(), |ui| {
                ui.label("The pruned world has not been saved. Close it anyway?");
                ui.horizontal(|ui| {
                    if ui.button("Close without saving").clicked() {
                        answer = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        answer = Some(false);
                    }
                });
            });
            if let Some(a) = answer {
                self.confirm_close = false;
                if a {
                    self.ws = None;
                    return;
                }
            }
        }
        ui.add_space(6.0);
        // Tool grid.
        let tools: [(Option<Tool>, Icon, &str); 6] = [
            (Some(Tool::Box), Icon::ShapeBox, "Box"),
            (Some(Tool::Circle), Icon::ShapeCircle, "Circle"),
            (Some(Tool::Polygon), Icon::ShapePolygon, "Polygon"),
            (Some(Tool::Split), Icon::ShapeSplit, "Split"),
            (None, Icon::Time, "Time"),
            (None, Icon::Optimize, "Optimize"),
        ];
        let w = (LEFT_PANEL - 12.0 - 8.0) / 3.0;
        for row in tools.chunks(3) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (tool, icon, label) in row {
                    let optimize = *label == "Optimize";
                    let enabled = !optimize || bedrock;
                    let active = tool.is_some() && self.tool == *tool;
                    let (r, resp) = ui.allocate_exact_size(
                        vec2(w, 64.0),
                        if enabled {
                            Sense::click()
                        } else {
                            Sense::hover()
                        },
                    );
                    ui.painter().rect_filled(
                        r,
                        theme::radius(4),
                        if resp.hovered() && enabled {
                            theme::BUTTON_HOVER
                        } else {
                            Color32::from_rgb(0x26, 0x2B, 0x33)
                        },
                    );
                    if active {
                        ui.painter().rect_stroke(
                            r,
                            theme::radius(4),
                            Stroke::new(1.2_f32, theme::BLUE),
                            egui::StrokeKind::Inside,
                        );
                    }
                    let c = if enabled {
                        theme::TEXT
                    } else {
                        theme::TEXT_DIM
                    };
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(r.center() - vec2(0.0, 9.0), Vec2::splat(28.0)),
                        *icon,
                        c,
                    );
                    ui.painter().text(
                        r.center() + vec2(0.0, 19.0),
                        Align2::CENTER_CENTER,
                        *label,
                        theme::regular(13.5),
                        c,
                    );
                    let resp = if optimize {
                        resp.on_hover_text(if bedrock {
                            "Bedrock worlds are always written with a freshly compacted database"
                        } else {
                            "Bedrock worlds only"
                        })
                    } else {
                        resp
                    };
                    if resp.clicked() {
                        match tool {
                            Some(t) => {
                                self.tool = if self.tool == Some(*t) {
                                    None
                                } else {
                                    Some(*t)
                                };
                                self.polygon.clear();
                            }
                            None if *label == "Time" => {
                                self.layers.push(PLayer::Time {
                                    cmp: 0,
                                    ticks: 1200.0,
                                });
                                self.selected = Some(self.layers.len() - 1);
                            }
                            None => {}
                        }
                    }
                }
            });
        }
        ui.add_space(6.0);
        let mut mode = self.inverted as usize;
        if widgets::segmented(
            ui,
            &mut mode,
            &["Normal", "Inverted"],
            LEFT_PANEL - 12.0,
            true,
        ) {
            self.inverted = mode == 1;
        }
        ui.add_space(8.0);
        let list_h = ui.available_height() - 84.0;
        egui::ScrollArea::vertical()
            .max_height(list_h)
            .auto_shrink(false)
            .show(ui, |ui| {
                let mut remove = None;
                for (i, l) in self.layers.iter_mut().enumerate() {
                    let sel = self.selected == Some(i);
                    let frame = egui::Frame::new()
                        .fill(Color32::from_rgb(0x1A, 0x1E, 0x24))
                        .inner_margin(egui::Margin::same(8));
                    let resp = frame.show(ui, |ui| {
                        ui.set_width(LEFT_PANEL - 28.0);
                        let top = ui.max_rect().left_top() - vec2(8.0, 8.0);
                        if sel {
                            ui.painter().line_segment(
                                [top, top + vec2(LEFT_PANEL - 12.0, 0.0)],
                                Stroke::new(1.5_f32, theme::BLUE),
                            );
                        }
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(l.title()).size(12.5).color(if sel {
                                theme::BLUE
                            } else {
                                theme::TEXT_MUTED
                            }));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if widgets::icon_button(ui, Icon::Trash, 20.0, "Remove layer")
                                        .clicked()
                                    {
                                        remove = Some(i);
                                    }
                                },
                            );
                        });
                        ui.horizontal(|ui| {
                            let (pv, _) = ui.allocate_exact_size(Vec2::splat(64.0), Sense::hover());
                            ui.painter().rect_filled(pv, theme::radius(3), theme::DEEP);
                            // UMT's layer thumbnails; polygons have none, so draw one.
                            let thumb = match (&*l, self.inverted) {
                                (PLayer::Box { .. }, false) => Some("THUMB_BOX_DELETE"),
                                (PLayer::Box { .. }, true) => Some("THUMB_BOX_DELETE_INVERSE"),
                                (PLayer::Circle { .. }, false) => Some("THUMB_CIRCLE_DELETE"),
                                (PLayer::Circle { .. }, true) => {
                                    Some("THUMB_CIRCLE_DELETE_INVERSE")
                                }
                                (PLayer::Split { .. }, _) => Some("THUMB_SPLIT_DELETE"),
                                (PLayer::Time { .. }, _) => Some("THUMB_TIME"),
                                (PLayer::Polygon { .. }, _) => None,
                            };
                            let shown = thumb
                                .is_some_and(|t| icons::image(ui.painter(), pv, t, Color32::WHITE));
                            if !shown {
                                let color = if self.inverted {
                                    Color32::BLACK
                                } else {
                                    theme::RED
                                };
                                icons::paint(ui.painter(), pv.shrink(14.0), l.icon(), color);
                            }
                            // Fields share the space left of the thumbnail, so they always fit
                            // inside the card.
                            // (less the gap before the column and a small right margin)
                            let avail = ui.available_width() - ui.spacing().item_spacing.x - 12.0;
                            let half = ((avail - 6.0) / 2.0 - 24.0).max(28.0);
                            let pair = |ui: &mut Ui, a: (&str, &mut f64), b: (&str, &mut f64)| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;
                                    widgets::labeled_number(ui, a.0, a.1, half, 0.1);
                                    widgets::labeled_number(ui, b.0, b.1, half, 0.1);
                                });
                            };
                            ui.vertical(|ui| match l {
                                PLayer::Box { x, z, w, h } => {
                                    pair(ui, ("X", x), ("Z", z));
                                    pair(ui, ("W", w), ("H", h));
                                }
                                PLayer::Circle { x, z, r } => {
                                    pair(ui, ("X", x), ("Z", z));
                                    widgets::labeled_number(ui, "Radius", r, avail - 62.0, 0.1);
                                }
                                PLayer::Split { x, z, angle } => {
                                    pair(ui, ("X", x), ("Z", z));
                                    widgets::labeled_number(ui, "Angle", angle, avail - 56.0, 1.0);
                                }
                                PLayer::Polygon { points } => {
                                    ui.label(
                                        egui::RichText::new(format!("{} points", points.len()))
                                            .color(theme::TEXT),
                                    );
                                    ui.label(
                                        egui::RichText::new("Drag the corners on the map")
                                            .size(12.0)
                                            .color(theme::TEXT_DIM),
                                    );
                                }
                                PLayer::Time { cmp, ticks } => {
                                    ui.label(
                                        egui::RichText::new("InhabitedTime")
                                            .size(12.5)
                                            .color(theme::TEXT_MUTED),
                                    );
                                    ui.horizontal(|ui| {
                                        egui::ComboBox::from_id_salt(("cmp", i))
                                            .width(40.0)
                                            .selected_text(COMPARISONS[*cmp].0)
                                            .show_ui(ui, |ui| {
                                                for (j, (s, _)) in COMPARISONS.iter().enumerate() {
                                                    ui.selectable_value(cmp, j, *s);
                                                }
                                            });
                                        ui.add(
                                            egui::DragValue::new(ticks)
                                                .speed(20.0)
                                                .range(0.0..=f64::MAX)
                                                .suffix(" ticks"),
                                        );
                                    });
                                }
                            });
                        });
                    });
                    if ui
                        .interact(
                            resp.response.rect,
                            ui.id().with(("layer", i)),
                            Sense::click(),
                        )
                        .clicked()
                    {
                        self.selected = Some(i);
                    }
                    ui.add_space(4.0);
                }
                if let Some(i) = remove {
                    self.layers.remove(i);
                    self.selected = None;
                }
            });
        // Hold-to-prune.
        let r = Rect::from_min_size(
            pos2(panel.left() + 6.0, panel.bottom() - 70.0),
            vec2(LEFT_PANEL - 12.0, 62.0),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        let enabled = !self.layers.is_empty() && self.job.is_none();
        let subtitle = if self.job.is_some() {
            "Pruning…"
        } else {
            "Hold for 5 seconds"
        };
        if widgets::hold_button(
            &mut child,
            egui::Id::new("prune-hold"),
            "Prune",
            subtitle,
            r.size(),
            5.0,
            enabled,
        ) {
            let ctx = ui.ctx().clone();
            self.prune(&ctx, settings);
        }
    }

    fn map(&mut self, ui: &mut Ui) {
        let Some(ws) = &mut self.ws else { return };
        let frame = ws
            .map
            .show_map(ui, self.tool.is_none() && self.dragging.is_none());
        let p = ui.painter_at(frame.rect);
        // The shape being drawn is shaded too, so the prune area shows while placing it.
        let mut shown = self.layers.clone();
        shown.extend(self.preview_layer(frame.hover));
        let sel = selection(&shown, self.inverted);
        // Shade removed chunks.
        let b0 = frame.to_block(frame.rect.min);
        let b1 = frame.to_block(frame.rect.max);
        let (cx0, cz0) = ((b0.x / 16.0).floor() as i32, (b0.y / 16.0).floor() as i32);
        let (cx1, cz1) = ((b1.x / 16.0).ceil() as i32, (b1.y / 16.0).ceil() as i32);
        let cells = ((cx1 - cx0) as i64) * ((cz1 - cz0) as i64);
        if !sel.layers.is_empty() && cells < 120_000 {
            let tint = Color32::from_rgba_unmultiplied(170, 40, 40, 80);
            let line = Stroke::new(1.3_f32, Color32::from_rgba_unmultiplied(235, 60, 60, 190));
            for cz in cz0..cz1 {
                for cx in cx0..cx1 {
                    if sel.keeps(ChunkPos::new(cx, cz), None) {
                        continue;
                    }
                    let a = frame.to_screen(vec2(cx as f32 * 16.0, cz as f32 * 16.0));
                    let b = frame.to_screen(vec2(cx as f32 * 16.0 + 16.0, cz as f32 * 16.0 + 16.0));
                    let r = Rect::from_min_max(a, b);
                    p.rect_filled(r, 0.0, tint);
                    if cells < 30_000 {
                        hatch(&p, r, 16.0, line);
                    }
                }
            }
        }
        let to_screen = |c: (f64, f64)| frame.to_screen(vec2(c.0 as f32 * 16.0, c.1 as f32 * 16.0));
        // Outlines march like UMT's (a dashed white line moving over a black one).
        let phase = (ui.input(|i| i.time) * 24.0 % 20.0) as f32;
        ui.ctx().request_repaint();
        let ants = |p: &egui::Painter, pts: &[Pos2]| {
            p.add(Shape::line(
                pts.to_vec(),
                Stroke::new(2.4_f32, Color32::BLACK),
            ));
            p.extend(Shape::dashed_line_with_offset(
                pts,
                Stroke::new(2.4_f32, Color32::WHITE),
                &[10.0],
                &[10.0],
                phase,
            ));
        };
        for (i, l) in self.layers.iter().enumerate() {
            let pts: Vec<Pos2> = match l {
                PLayer::Box { x, z, w, h } => {
                    [(*x, *z), (x + w, *z), (x + w, z + h), (*x, z + h), (*x, *z)]
                        .iter()
                        .map(|c| to_screen(*c))
                        .collect()
                }
                PLayer::Circle { x, z, r } => (0..=64)
                    .map(|k| {
                        let a = k as f64 / 64.0 * std::f64::consts::TAU;
                        to_screen((x + r * a.cos(), z + r * a.sin()))
                    })
                    .collect(),
                PLayer::Split { x, z, angle } => {
                    let a = angle.to_radians();
                    vec![
                        to_screen((x - 4000.0 * a.cos(), z - 4000.0 * a.sin())),
                        to_screen((x + 4000.0 * a.cos(), z + 4000.0 * a.sin())),
                    ]
                }
                PLayer::Polygon { points } => {
                    let mut v: Vec<Pos2> = points.iter().map(|c| to_screen(*c)).collect();
                    if let Some(f) = v.first().copied() {
                        v.push(f);
                    }
                    v
                }
                PLayer::Time { .. } => Vec::new(),
            };
            if pts.len() > 1 {
                ants(&p, &pts);
            }
            if self.selected == Some(i) {
                for h in l.handles() {
                    let s = to_screen(h);
                    p.rect_filled(
                        Rect::from_center_size(s, Vec2::splat(10.0)),
                        0.0,
                        Color32::WHITE,
                    );
                    p.rect_stroke(
                        Rect::from_center_size(s, Vec2::splat(10.0)),
                        0.0,
                        Stroke::new(1.0_f32, Color32::BLACK),
                        egui::StrokeKind::Outside,
                    );
                }
            }
        }
        self.interact(&frame);
        // Shape being drawn.
        if let Some((a, b)) = self.drawing {
            let preview = match self.tool {
                Some(Tool::Box) => Some(vec![
                    to_screen(a),
                    to_screen((b.0, a.1)),
                    to_screen(b),
                    to_screen((a.0, b.1)),
                    to_screen(a),
                ]),
                Some(Tool::Circle) => {
                    let r = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
                    Some(
                        (0..=64)
                            .map(|k| {
                                let t = k as f64 / 64.0 * std::f64::consts::TAU;
                                to_screen((a.0 + r * t.cos(), a.1 + r * t.sin()))
                            })
                            .collect(),
                    )
                }
                Some(Tool::Split) => Some(vec![to_screen(a), to_screen(b)]),
                _ => None,
            };
            if let Some(pts) = preview {
                ants(&p, &pts);
            }
        }
        if !self.polygon.is_empty() {
            // Always drawn closed: the corners so far, the pointer, then back to the first.
            let mut pts: Vec<Pos2> = self.polygon.iter().map(|c| to_screen(*c)).collect();
            if let Some(h) = frame.hover {
                pts.push(frame.to_screen(h));
            }
            let corners = pts.clone();
            pts.push(pts[0]);
            ants(&p, &pts);
            for q in &corners {
                p.circle_filled(*q, 4.0, Color32::WHITE);
            }
        }
        if let Some(tool) = self.tool {
            let hint = match tool {
                Tool::Box => "Drag to draw a box",
                Tool::Circle => "Drag from the centre to draw a circle",
                Tool::Split => "Drag to draw the split line; the right side is selected",
                Tool::Polygon => "Click to add corners; right-click to place the last corner",
            };
            p.text(
                frame.rect.center_bottom() - vec2(0.0, 40.0),
                Align2::CENTER_CENTER,
                hint,
                theme::regular(13.0),
                Color32::WHITE,
            );
        }
        ws_chrome(self.ws.as_mut(), ui, &frame);
    }

    /// The layer the shape being drawn would become, for shading while placing it.
    fn preview_layer(&self, hover: Option<Vec2>) -> Option<PLayer> {
        if !self.polygon.is_empty() {
            let mut points = self.polygon.clone();
            if let Some(h) = hover {
                points.push(((h.x / 16.0) as f64, (h.y / 16.0) as f64));
            }
            return (points.len() >= 3).then_some(PLayer::Polygon { points });
        }
        let (a, c) = self.drawing?;
        Some(match self.tool? {
            Tool::Box => {
                let (x0, x1) = (a.0.min(c.0).floor(), a.0.max(c.0).ceil());
                let (z0, z1) = (a.1.min(c.1).floor(), a.1.max(c.1).ceil());
                PLayer::Box {
                    x: x0,
                    z: z0,
                    w: x1 - x0,
                    h: z1 - z0,
                }
            }
            Tool::Circle => PLayer::Circle {
                x: a.0,
                z: a.1,
                r: ((c.0 - a.0).powi(2) + (c.1 - a.1).powi(2)).sqrt(),
            },
            Tool::Split => PLayer::Split {
                x: a.0,
                z: a.1,
                angle: (c.1 - a.1).atan2(c.0 - a.0).to_degrees().rem_euclid(360.0),
            },
            Tool::Polygon => return None,
        })
    }

    fn interact(&mut self, frame: &MapFrame) {
        let resp = &frame.response;
        let chunk = |b: Vec2| ((b.x / 16.0) as f64, (b.y / 16.0) as f64);
        let pointer = resp.interact_pointer_pos().or(resp.hover_pos());
        // Handle dragging for the selected layer.
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let (Some(i), Some(pos)) = (self.selected, pointer) {
                if let Some(l) = self.layers.get(i) {
                    for (hi, h) in l.handles().into_iter().enumerate() {
                        let s = frame.to_screen(vec2(h.0 as f32 * 16.0, h.1 as f32 * 16.0));
                        if s.distance(pos) < 9.0 {
                            self.dragging = Some((i, hi));
                        }
                    }
                }
            }
        }
        if let Some((i, hi)) = self.dragging {
            if let Some(pos) = pointer {
                let c = chunk(frame.to_block(pos));
                if let Some(l) = self.layers.get_mut(i) {
                    l.drag_handle(hi, c);
                }
            }
            if resp.drag_stopped() || !resp.dragged() {
                self.dragging = None;
            }
            return;
        }
        let Some(tool) = self.tool else { return };
        let Some(pos) = pointer else { return };
        let c = chunk(frame.to_block(pos));
        match tool {
            // As UMT: left-click adds a corner, right-click places the last one and closes
            // the polygon back to the first corner.
            Tool::Polygon => {
                let corner = ((c.0 * 2.0).round() / 2.0, (c.1 * 2.0).round() / 2.0);
                if resp.clicked_by(egui::PointerButton::Primary) {
                    self.polygon.push(corner);
                } else if resp.clicked_by(egui::PointerButton::Secondary) {
                    self.polygon.push(corner);
                    if self.polygon.len() >= 3 {
                        self.layers.push(PLayer::Polygon {
                            points: std::mem::take(&mut self.polygon),
                        });
                        self.selected = Some(self.layers.len() - 1);
                        self.tool = None;
                    } else {
                        self.polygon.pop();
                    }
                }
            }
            _ => {
                if resp.drag_started_by(egui::PointerButton::Primary) {
                    self.drawing = Some((c, c));
                }
                if let Some((a, _)) = self.drawing {
                    self.drawing = Some((a, c));
                    if resp.drag_stopped() {
                        self.drawing = None;
                        let layer = match tool {
                            Tool::Box => {
                                let (x0, x1) = (a.0.min(c.0).floor(), a.0.max(c.0).ceil());
                                let (z0, z1) = (a.1.min(c.1).floor(), a.1.max(c.1).ceil());
                                PLayer::Box {
                                    x: x0,
                                    z: z0,
                                    w: x1 - x0,
                                    h: z1 - z0,
                                }
                            }
                            Tool::Circle => PLayer::Circle {
                                x: a.0,
                                z: a.1,
                                r: ((c.0 - a.0).powi(2) + (c.1 - a.1).powi(2)).sqrt(),
                            },
                            _ => PLayer::Split {
                                x: a.0,
                                z: a.1,
                                angle: (c.1 - a.1).atan2(c.0 - a.0).to_degrees().rem_euclid(360.0),
                            },
                        };
                        self.layers.push(layer);
                        self.selected = Some(self.layers.len() - 1);
                        self.tool = None;
                    }
                }
            }
        }
    }
}

fn ws_chrome(ws: Option<&mut Workspace>, ui: &mut Ui, frame: &MapFrame) {
    if let Some(ws) = ws {
        ws.map.show_chrome(ui, frame);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_convert_from_chunk_units() {
        let c = PLayer::Circle {
            x: -5.5,
            z: 0.5,
            r: 8.0,
        };
        assert_eq!(
            c.to_layer(),
            Some(Layer::Circle {
                x: -88.0,
                z: 8.0,
                radius: 128.0
            })
        );
        let mut b = PLayer::Box {
            x: 0.0,
            z: 0.0,
            w: 4.0,
            h: 2.0,
        };
        b.drag_handle(2, (6.4, -1.2));
        assert_eq!(
            b,
            PLayer::Box {
                x: 0.0,
                z: -1.0,
                w: 6.0,
                h: 1.0
            }
        );
        assert!(
            PLayer::Polygon {
                points: vec![(0.0, 0.0), (1.0, 1.0)]
            }
            .to_layer()
            .is_none()
        );
    }
}
