//! The Converter page: INPUT â†’ OUTPUT â†’ OPTIONS â†’ CONVERT â†’ SAVE.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Ui, Vec2, pos2, vec2};
use mctk_convert::{ConvertOptions, Progress};

use crate::editor::edition_chooser;
use crate::icons::{self, Icon};
use crate::jobs::Job;
use crate::settings::Settings;
use crate::theme;
use crate::widgets;
use crate::workspace::copy_dir;
use crate::world_picker::UiEdition;

/// A card of `size` centred in `area`, never starting above it, so one taller than the area
/// overflows downwards instead of up under the banner.
fn centered(area: Rect, size: Vec2) -> Rect {
    let mut r = Rect::from_center_size(area.center(), size);
    if r.top() < area.top() {
        r = r.translate(vec2(0.0, area.top() - r.top()));
    }
    r
}

const STEPS: [&str; 5] = ["INPUT", "OUTPUT", "OPTIONS", "CONVERT", "SAVE"];

pub struct ConverterPage {
    step: usize,
    pub input: Option<(PathBuf, UiEdition)>,
    output: Option<UiEdition>,
    dims: [bool; 3],
    skip_partial: bool,
    game_mode: usize,
    difficulty: usize,
    center: (f64, f64),
    center_spawn: bool,
    progress: Option<Arc<Progress>>,
    job: Option<Job<String>>,
    started: Option<Instant>,
    elapsed: f64,
    out_dir: Option<PathBuf>,
    result: Option<Result<String, String>>,
    pub status: String,
}

pub enum ConverterRequest {
    None,
    PickWorld(UiEdition),
}

impl ConverterPage {
    pub fn new(settings: &Settings) -> Self {
        let c = if settings.remember_conversion_options {
            settings.conversion
        } else {
            Settings::default().conversion
        };
        ConverterPage {
            step: 0,
            input: None,
            output: None,
            dims: [c[0] != 0, c[1] != 0, c[2] != 0],
            skip_partial: c[3] != 0,
            game_mode: c[4].clamp(0, 3) as usize,
            difficulty: c[5].clamp(0, 4) as usize,
            center: (0.0, 0.0),
            center_spawn: false,
            progress: None,
            job: None,
            started: None,
            elapsed: 0.0,
            out_dir: None,
            result: None,
            status: String::new(),
        }
    }

    pub fn set_input(&mut self, path: PathBuf, edition: UiEdition) {
        self.input = Some((path, edition));
        self.output = None;
        self.step = 1;
    }

    /// Output editions this build can write from the chosen input.
    fn supported(&self, out: UiEdition) -> bool {
        match self.input.as_ref().map(|i| i.1) {
            Some(UiEdition::Java) => out == UiEdition::Bedrock,
            Some(UiEdition::Bedrock) => out == UiEdition::Java,
            // Console to Bedrock also needs the pre-1.13 block upgrade (start() chains it
            // through Java once that exists).
            Some(UiEdition::Xbox360 | UiEdition::Ps3 | UiEdition::WiiU) => out == UiEdition::Java,
            None => false,
        }
    }

    fn options(&self) -> ConvertOptions {
        let mut offset = (0, 0);
        if let Some((path, ed)) = &self.input {
            let (cx, cz) = if self.center_spawn {
                spawn_of(path, *ed).unwrap_or((0, 0))
            } else {
                (self.center.0 as i32, self.center.1 as i32)
            };
            offset = (-(cx.div_euclid(16)), -(cz.div_euclid(16)));
        }
        ConvertOptions {
            overworld: self.dims[0],
            nether: self.dims[1],
            the_end: self.dims[2],
            skip_partial_chunks: self.skip_partial,
            game_mode: self.game_mode.checked_sub(1).map(|g| g as i32),
            difficulty: self.difficulty.checked_sub(1).map(|d| d as i32),
            offset,
        }
    }

    fn start(&mut self, ctx: &egui::Context, settings: &mut Settings) {
        let Some((src, ed)) = self.input.clone() else {
            return;
        };
        let Some(out_ed) = self.output else { return };
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "World".into());
        let out = match settings.new_workspace("CONVERTER", &name) {
            Ok(p) => p,
            Err(e) => {
                self.status = format!("Could not create the working folder: {e}");
                return;
            }
        };
        if settings.remember_conversion_options {
            settings.conversion = [
                self.dims[0] as i64,
                self.dims[1] as i64,
                self.dims[2] as i64,
                self.skip_partial as i64,
                self.game_mode as i64,
                self.difficulty as i64,
            ];
            settings.save();
        }
        let options = self.options();
        let progress = Arc::new(Progress::default());
        let p = Arc::clone(&progress);
        let target = out.clone();
        self.job = Some(Job::spawn_notifying("Converting", ctx, move || {
            let chunks = |r: &mctk_convert::EditionReport| r.chunks.values().sum::<usize>();
            match (ed, out_ed) {
                (UiEdition::Java, UiEdition::Bedrock) => {
                    mctk_convert::java_to_bedrock_with(&src, &target, &options, &p)
                        .map(|r| chunks(&r))
                }
                (UiEdition::Bedrock, UiEdition::Java) => {
                    mctk_convert::bedrock_to_java_with(&src, &target, &options, &p)
                        .map(|r| chunks(&r))
                }
                // Console to Bedrock goes through a temporary Java world.
                (_, UiEdition::Bedrock) => {
                    let stage = target.with_extension("java-stage");
                    let result = mctk_console::ConsoleSave::open(&src)
                        .map_err(mctk_convert::Error::from)
                        .and_then(|s| mctk_convert::console_to_java_with(&s, &stage, &options, &p))
                        .and_then(|_| {
                            // The offset and dimension choice were applied in the first pass.
                            let pass = ConvertOptions {
                                offset: (0, 0),
                                ..options.clone()
                            };
                            mctk_convert::java_to_bedrock_with(&stage, &target, &pass, &p)
                        })
                        .map(|r| chunks(&r));
                    let _ = std::fs::remove_dir_all(&stage);
                    result
                }
                (_, UiEdition::Java) => mctk_console::ConsoleSave::open(&src)
                    .map_err(mctk_convert::Error::from)
                    .and_then(|s| mctk_convert::console_to_java_with(&s, &target, &options, &p))
                    .map(|r| r.chunks.values().sum()),
                _ => Err(mctk_convert::Error::Invalid(
                    "this conversion is not available".into(),
                )),
            }
            .map(|n| format!("{n}"))
            .map_err(|e| e.to_string())
        }));
        self.progress = Some(progress);
        self.started = Some(Instant::now());
        self.out_dir = Some(out);
        self.result = None;
    }

    pub fn ui(&mut self, ui: &mut Ui, settings: &mut Settings) -> ConverterRequest {
        let mut request = ConverterRequest::None;
        if let Some(r) = self.job.as_ref().and_then(Job::take) {
            self.job = None;
            self.elapsed = self.started.map_or(0.0, |s| s.elapsed().as_secs_f64());
            if r.is_ok() {
                self.step = 4;
            }
            self.result = Some(r);
        }
        let full = ui.max_rect();
        // Stepper band.
        let band = Rect::from_min_size(full.min, vec2(full.width(), 112.0));
        ui.painter().rect_filled(band, 0.0, theme::CARD);
        ui.painter().line_segment(
            [band.left_bottom(), band.right_bottom()],
            egui::Stroke::new(1.0_f32, theme::BORDER),
        );
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(band.shrink2(vec2(0.0, 8.0))));
        let reachable = match (&self.input, &self.output) {
            (None, _) => 0,
            (Some(_), None) => 1,
            (Some(_), Some(_)) if self.step >= 3 => self.step,
            _ => 2,
        };
        let running = self.job.is_some();
        if let Some(s) = widgets::stepper(
            &mut child,
            &STEPS,
            self.step,
            if running { self.step } else { reachable.min(3) },
        ) {
            self.step = s;
        }
        let arrow = |ui: &mut Ui, x: f32, icon: Icon, id: &str| -> bool {
            let r = Rect::from_center_size(pos2(x, band.top() + 60.0), Vec2::splat(28.0));
            let resp = ui.interact(r, ui.id().with(id), Sense::click());
            icons::paint(
                ui.painter(),
                r.shrink(6.0),
                icon,
                if resp.hovered() {
                    Color32::WHITE
                } else {
                    theme::TEXT_MUTED
                },
            );
            resp.clicked()
        };
        let cx = band.center().x;
        if self.step > 0
            && !running
            && self.step < 4
            && arrow(ui, cx - 318.0, Icon::ChevronLeft, "back")
        {
            self.step -= 1;
        }
        if self.step < reachable.min(3)
            && !running
            && arrow(ui, cx + 318.0, Icon::ChevronRight, "fwd")
        {
            self.step += 1;
        }
        // Conversion is new and has not been tried in the games yet. The banner sits in its
        // own strip under the stepper, so it never covers the step's content.
        let text = "Early and untested: world conversion has not been verified in Minecraft. \
                    Keep a backup of your world, and expect missing or broken parts.";
        let font = theme::regular(13.0);
        let invite = vec2(150.0, 26.0);
        let margin = 16.0;
        // The banner spans the page between equal margins; the text wraps inside it and the
        // button always sits centred underneath, so nothing jumps while the window resizes.
        let width = (full.width() - 2.0 * margin).max(200.0);
        let galley = ui
            .painter()
            .layout(text.into(), font, theme::YELLOW, width - 28.0);
        let banner = Rect::from_min_size(
            pos2(full.center().x - width / 2.0, band.bottom() + margin),
            vec2(width, galley.size().y + invite.y + 36.0),
        );
        // The same margin all round: below the banner and under the content.
        let body = Rect::from_min_max(
            pos2(full.left(), banner.bottom() + margin),
            pos2(full.right(), full.bottom() - margin),
        );
        // A step taller than the window scrolls instead of being cut off.
        let need = match self.step {
            0 => 0.0,
            1 => 300.0,
            2 => 318.0,
            _ if self.job.is_some() || self.result.is_some() => 420.0,
            _ => 278.0,
        };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
        child.set_clip_rect(body.intersect(ui.clip_rect()));
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(&mut child, |ui| {
                let (area, _) = ui.allocate_exact_size(
                    vec2(body.width(), body.height().max(need)),
                    Sense::hover(),
                );
                let mut page = ui.new_child(egui::UiBuilder::new().max_rect(area));
                match self.step {
                    0 => {
                        if let Some(e) =
                            edition_chooser(&mut page, "Select a World to Convert", None)
                        {
                            request = ConverterRequest::PickWorld(e);
                        }
                    }
                    1 => self.output_step(&mut page),
                    2 => self.options_step(&mut page),
                    _ => self.convert_step(&mut page, settings),
                }
            });
        ui.painter().rect_filled(banner, 6.0, theme::CARD);
        ui.painter().rect_stroke(
            banner,
            6.0,
            egui::Stroke::new(1.0_f32, theme::YELLOW),
            egui::StrokeKind::Inside,
        );
        ui.painter().galley(
            pos2(banner.left() + 14.0, banner.top() + 12.0),
            galley,
            theme::YELLOW,
        );
        let button = Rect::from_center_size(
            pos2(banner.center().x, banner.bottom() - 12.0 - invite.y / 2.0),
            invite,
        );
        let resp = ui
            .interact(button, ui.id().with("discord"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Join the MCToolKit Discord to report problems");
        ui.painter().rect_filled(
            button,
            theme::radius(4),
            if resp.hovered() { theme::BUTTON_HOVER } else { theme::BUTTON },
        );
        ui.painter().text(
            button.center(),
            Align2::CENTER_CENTER,
            "Report on Discord",
            theme::regular(13.0),
            theme::TEXT,
        );
        if resp.clicked() {
            widgets::open_discord(ui);
        }
        if !self.status.is_empty() {
            ui.painter().text(
                full.center_bottom() - vec2(0.0, 18.0),
                Align2::CENTER_CENTER,
                &self.status,
                theme::regular(13.0),
                theme::TEXT_MUTED,
            );
        }
        request
    }

    fn output_step(&mut self, ui: &mut Ui) {
        let card = vec2(680.0, 300.0);
        let rect = centered(ui.max_rect(), card);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            widgets::card(ui, 19, |ui| {
                ui.set_width(card.x - 40.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Select an Output Version")
                            .size(20.0)
                            .color(Color32::WHITE),
                    )
                });
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    for e in UiEdition::ALL {
                        let ok = self.supported(e);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let resp = widgets::tile(
                                ui,
                                e.icon(),
                                e.label(),
                                vec2(120.0, 120.0),
                                self.output == Some(e),
                                ok,
                            );
                            let resp = if ok {
                                resp
                            } else {
                                resp.on_hover_text("Not available in MCToolKit yet")
                            };
                            if resp.clicked() {
                                self.output = Some(e);
                            }
                            let version = match e {
                                UiEdition::Java => "26.3 (Latest)",
                                UiEdition::Bedrock => "1.26.60 (Latest)",
                                _ => "TU â€” (n/a)",
                            };
                            let (r, _) = ui.allocate_exact_size(vec2(120.0, 30.0), Sense::hover());
                            ui.painter().rect_filled(
                                r,
                                egui::CornerRadius {
                                    nw: 0,
                                    ne: 0,
                                    sw: 5,
                                    se: 5,
                                },
                                theme::TILE,
                            );
                            ui.painter().text(
                                r.left_center() + vec2(7.0, 0.0),
                                Align2::LEFT_CENTER,
                                version,
                                theme::regular(13.0),
                                if ok { theme::TEXT } else { theme::TEXT_DIM },
                            );
                            icons::paint(
                                ui.painter(),
                                Rect::from_center_size(
                                    r.right_center() - vec2(12.0, 0.0),
                                    Vec2::splat(9.0),
                                ),
                                Icon::ChevronDown,
                                theme::TEXT_MUTED,
                            );
                        });
                    }
                });
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    if widgets::primary(ui, "Next", vec2(160.0, 50.0), self.output.is_some())
                        .clicked()
                    {
                        self.step = 2;
                    }
                });
            });
        });
    }

    fn options_step(&mut self, ui: &mut Ui) {
        let card = vec2(680.0, 318.0);
        let rect = centered(ui.max_rect(), card);
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            widgets::card(ui, 19, |ui| {
                ui.set_width(card.x - 40.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new("Conversion Options")
                            .size(20.0)
                            .color(Color32::WHITE),
                    )
                });
                ui.add_space(14.0);
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(322.0);
                        ui.horizontal(|ui| {
                            widgets::toggle(ui, &mut self.dims[0], "Overworld", true);
                            widgets::toggle(ui, &mut self.dims[1], "Nether", true);
                            widgets::toggle(ui, &mut self.dims[2], "The End", true);
                        });
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let tip =
                                "Chests, signs, mobs and ticks are not converted by MCToolKit yet";
                            let mut off = false;
                            widgets::toggle(ui, &mut off, "Tile Entities", false)
                                .on_hover_text(tip);
                            widgets::toggle(ui, &mut off, "Entities", false).on_hover_text(tip);
                            widgets::toggle(ui, &mut off, "Ticks", false).on_hover_text(tip);
                        });
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("Game Mode")
                                .size(13.5)
                                .color(theme::TEXT_MUTED),
                        );
                        widgets::segmented(
                            ui,
                            &mut self.game_mode,
                            &["No Change", "Survival", "Creative", "Adventure"],
                            322.0,
                            true,
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("Difficulty")
                                .size(13.5)
                                .color(theme::TEXT_MUTED),
                        );
                        widgets::segmented(
                            ui,
                            &mut self.difficulty,
                            &["No Change", "Peaceful", "Easy", "Normal", "Hard"],
                            322.0,
                            true,
                        );
                    });
                    let x = ui.cursor().left();
                    ui.painter().line_segment(
                        [
                            pos2(x + 2.0, ui.cursor().top()),
                            pos2(x + 2.0, ui.cursor().top() + 160.0),
                        ],
                        egui::Stroke::new(1.0_f32, theme::BORDER),
                    );
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        widgets::toggle(
                            ui,
                            &mut self.skip_partial,
                            "Skip Partially-Generated Chunks",
                            true,
                        )
                        .on_hover_text("Leave out chunks the game had not finished generating");
                        ui.add_space(6.0);
                        let mut older = true;
                        widgets::toggle(ui, &mut older, "Allow Older Chunk Versions", false)
                            .on_hover_text("Older chunk formats are always read when supported");
                        ui.add_space(10.0);
                        ui.label(
                            RichText::new("Depth Fixer")
                                .size(13.5)
                                .color(theme::TEXT_MUTED),
                        );
                        let mut depth = 0;
                        widgets::segmented(
                            ui,
                            &mut depth,
                            &["Clip + Bedrock", "Clip", "Raise"],
                            290.0,
                            false,
                        );
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new("New World Center")
                                .size(13.5)
                                .color(theme::TEXT_MUTED),
                        )
                        .on_hover_text(
                            "Moves the output so this point becomes 0, 0 (snapped to chunks)",
                        );
                        ui.horizontal(|ui| {
                            widgets::labeled_number(ui, "X", &mut self.center.0, 40.0, 1.0);
                            widgets::labeled_number(ui, "Z", &mut self.center.1, 40.0, 1.0);
                            let mut mode = self.center_spawn as usize;
                            if widgets::segmented(
                                ui,
                                &mut mode,
                                &["Coords", "Spawn Point"],
                                150.0,
                                true,
                            ) {
                                self.center_spawn = mode == 1;
                            }
                        });
                    });
                });
                ui.add_space(18.0);
                ui.vertical_centered(|ui| {
                    let any = self.dims.iter().any(|d| *d);
                    if widgets::primary(ui, "Next", vec2(160.0, 50.0), any).clicked() {
                        self.step = 3;
                    }
                });
            });
        });
    }

    fn convert_step(&mut self, ui: &mut Ui, settings: &mut Settings) {
        let running = self.job.is_some();
        let done = matches!(self.result, Some(Ok(_)));
        let tall = running || self.result.is_some();
        let card = vec2(680.0, if tall { 420.0 } else { 278.0 });
        let rect = centered(ui.max_rect(), card);
        let ctx = ui.ctx().clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            widgets::card(ui, 19, |ui| {
                ui.set_width(card.x - 40.0);
                // Source â†’ target.
                let (r, _) = ui.allocate_exact_size(vec2(card.x - 40.0, 92.0), Sense::hover());
                let (src, dst) = (self.input.as_ref().map(|i| i.1), self.output);
                let p = ui.painter();
                if let (Some(s), Some(d)) = (src, dst) {
                    let c = r.center();
                    icons::paint(
                        p,
                        Rect::from_center_size(c - vec2(80.0, 20.0), Vec2::splat(40.0)),
                        s.icon(),
                        Color32::from_rgb(0xC8, 0xCD, 0xD4),
                    );
                    p.text(
                        c - vec2(80.0, -16.0),
                        Align2::CENTER_CENTER,
                        s.label(),
                        theme::regular(14.0),
                        theme::TEXT,
                    );
                    icons::paint(
                        p,
                        Rect::from_center_size(c - vec2(0.0, 20.0), Vec2::splat(64.0)),
                        Icon::ArrowRight,
                        Color32::from_rgb(0xC8, 0xCD, 0xD4),
                    );
                    icons::paint(
                        p,
                        Rect::from_center_size(c + vec2(80.0, -20.0), Vec2::splat(40.0)),
                        d.icon(),
                        Color32::from_rgb(0xC8, 0xCD, 0xD4),
                    );
                    p.text(
                        c + vec2(80.0, 16.0),
                        Align2::CENTER_CENTER,
                        d.label(),
                        theme::regular(14.0),
                        theme::TEXT,
                    );
                    let v = if d == UiEdition::Java {
                        "26.3 (Latest)"
                    } else {
                        "1.26.60 (Latest)"
                    };
                    p.text(
                        c + vec2(80.0, 33.0),
                        Align2::CENTER_CENTER,
                        v,
                        theme::regular(11.5),
                        theme::TEXT_DIM,
                    );
                }
                let prog = self.progress.clone();
                let (n, total) = prog.as_ref().map_or((0, 0), |p| {
                    (
                        p.chunks_done.load(Ordering::Relaxed),
                        p.chunks_total.load(Ordering::Relaxed),
                    )
                });
                let headline = match (&self.result, running) {
                    (_, true) => format!("Converted {} Chunks", fmt_int(n)),
                    (Some(Ok(c)), _) => format!(
                        "Conversion Successful! {} Chunks Converted",
                        fmt_int(c.parse().unwrap_or(n))
                    ),
                    (Some(Err(e)), _) => format!("Conversion Failed: {e}"),
                    (None, false) => "Ready to Convert".into(),
                };
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(headline).size(14.0).color(theme::TEXT))
                });
                ui.add_space(6.0);
                let frac = if done {
                    1.0
                } else if total > 0 {
                    n as f32 / total as f32
                } else {
                    0.0
                };
                widgets::progress_bar(ui, frac, card.x - 40.0);
                ui.add_space(14.0);
                if tall {
                    self.stats(ui, card.x - 40.0, running);
                    ui.add_space(20.0);
                }
                ui.vertical_centered(|ui| {
                    if running {
                        if widgets::big_button(ui, "Stop Conversion", vec2(200.0, 50.0)).clicked() {
                            if let Some(p) = &self.progress {
                                p.cancel();
                            }
                        }
                        ctx.request_repaint_after(std::time::Duration::from_millis(200));
                    } else if done {
                        ui.horizontal(|ui| {
                            ui.add_space((card.x - 40.0 - 420.0) / 2.0);
                            if widgets::big_button(ui, "Start Over", vec2(200.0, 50.0)).clicked() {
                                self.step = 0;
                                self.input = None;
                                self.output = None;
                                self.result = None;
                                self.progress = None;
                            }
                            ui.add_space(20.0);
                            if widgets::primary(ui, "Save World", vec2(200.0, 50.0), true).clicked()
                            {
                                self.save(settings);
                            }
                        });
                    } else if widgets::primary(ui, "Start Conversion", vec2(200.0, 50.0), true)
                        .clicked()
                    {
                        self.start(&ctx, settings);
                    }
                });
            });
        });
    }

    fn stats(&self, ui: &mut Ui, width: f32, running: bool) {
        let (r, _) = ui.allocate_exact_size(vec2(width, 122.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, theme::radius(4), theme::BG);
        // Coverage preview of converted chunks.
        let pv = Rect::from_min_size(r.min + vec2(10.0, 10.0), vec2(102.0, 102.0));
        p.rect_filled(pv, 0.0, Color32::from_rgb(0x0F, 0x11, 0x16));
        let Some(prog) = &self.progress else { return };
        {
            let pts = prog.converted.lock().expect("progress lock");
            if !pts.is_empty() {
                let (mut x0, mut x1, mut z0, mut z1) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
                for (x, z) in pts.iter() {
                    x0 = x0.min(*x);
                    x1 = x1.max(*x);
                    z0 = z0.min(*z);
                    z1 = z1.max(*z);
                }
                let span = ((x1 - x0).max(z1 - z0) + 1) as f32;
                let s = (pv.width() / span).max(1.0);
                let recent = pts.len().saturating_sub(64);
                for (i, (x, z)) in pts.iter().enumerate() {
                    let q = pv.min + vec2((x - x0) as f32 * s, (z - z0) as f32 * s);
                    let c = if i >= recent && running {
                        Color32::from_rgb(0xE6, 0x9A, 0x2E)
                    } else {
                        theme::GREEN
                    };
                    p.rect_filled(Rect::from_min_size(q, Vec2::splat(s.max(1.0))), 0.0, c);
                }
            }
        }
        let elapsed = if running {
            self.started.map_or(0.0, |s| s.elapsed().as_secs_f64())
        } else {
            self.elapsed
        };
        let n = prog.chunks_done.load(Ordering::Relaxed);
        let total = prog.chunks_total.load(Ordering::Relaxed);
        let rate = if elapsed > 0.0 {
            n as f64 / elapsed
        } else {
            0.0
        };
        let remaining = if running && rate > 0.0 {
            format!("~{}", clock((total.saturating_sub(n)) as f64 / rate))
        } else {
            String::new()
        };
        let cores = std::thread::available_parallelism().map_or(1, |c| c.get());
        let ram = if running {
            memory_mb().map_or("â€”".into(), |m| format!("{m}MB"))
        } else {
            String::new()
        };
        let dim = if running {
            prog.dimension
                .lock()
                .expect("progress lock")
                .trim_start_matches("minecraft:")
                .replace('_', " ")
        } else {
            String::new()
        };
        let cols = [
            [
                ("Elapsed time:", clock(elapsed)),
                ("Remaining time:", remaining),
                ("Avg. chunks/sec:", format!("{}", rate.round() as i64)),
            ],
            [
                (
                    "Sections:",
                    format!(
                        "{}/{}",
                        prog.regions_done.load(Ordering::Relaxed),
                        prog.regions_total.load(Ordering::Relaxed)
                    ),
                ),
                ("RAM usage:", ram),
                (
                    "Invalid chunks:",
                    prog.invalid_chunks.load(Ordering::Relaxed).to_string(),
                ),
            ],
            [
                ("Core count:", cores.to_string()),
                ("Dimension:", title_case(&dim)),
                ("Relighting:", "No".into()),
            ],
        ];
        for (ci, col) in cols.iter().enumerate() {
            for (ri, (k, v)) in col.iter().enumerate() {
                let pos = pos2(
                    r.left() + 132.0 + ci as f32 * 166.0,
                    r.top() + 27.0 + ri as f32 * 34.0,
                );
                let kg = p.layout_no_wrap(k.to_string(), theme::regular(13.0), theme::TEXT_MUTED);
                let w = kg.size().x;
                p.galley(pos - vec2(0.0, 8.0), kg, theme::TEXT_MUTED);
                p.text(
                    pos + vec2(w + 4.0, 0.0),
                    Align2::LEFT_CENTER,
                    v,
                    theme::bold(13.0),
                    Color32::WHITE,
                );
            }
        }
    }

    fn save(&mut self, settings: &mut Settings) {
        let Some(out) = self.out_dir.clone() else {
            return;
        };
        let Some(out_ed) = self.output else { return };
        let default_dir = match out_ed {
            UiEdition::Java => &settings.java_worlds,
            _ => &settings.bedrock_worlds,
        };
        let mut dialog = rfd::FileDialog::new().set_title("Save the converted world intoâ€¦");
        if std::path::Path::new(default_dir).is_dir() {
            dialog = dialog.set_directory(default_dir);
        }
        let Some(dest_parent) = dialog.pick_folder() else {
            return;
        };
        let name = self
            .input
            .as_ref()
            .and_then(|i| i.0.file_stem())
            .map_or("Converted World".into(), |n| {
                n.to_string_lossy().to_string()
            });
        let mut dest = dest_parent.join(&name);
        let mut n = 2;
        while dest.exists() {
            dest = dest_parent.join(format!("{name} ({n})"));
            n += 1;
        }
        self.status = match copy_dir(&out, &dest) {
            Ok(()) => format!("Saved to {}", dest.display()),
            Err(e) => format!("Save failed: {e}"),
        };
        crate::jobs::finished();
    }
}

fn fmt_int(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn clock(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn spawn_of(path: &std::path::Path, ed: UiEdition) -> Option<(i32, i32)> {
    match ed {
        UiEdition::Java => mctk_java::JavaWorld::open(path)
            .ok()?
            .level
            .spawn
            .map(|s| (s.x, s.z)),
        UiEdition::Bedrock => {
            let w = mctk_bedrock::BedrockWorld::open(path).ok()?;
            let g = |k: &str| {
                w.level
                    .root
                    .tag
                    .get(k)
                    .and_then(mctk_nbt::Tag::as_i64)
                    .unwrap_or(0) as i32
            };
            Some((g("SpawnX"), g("SpawnZ")))
        }
        _ => None,
    }
}

/// Working set of this process in MB (Windows).
#[cfg(windows)]
fn memory_mb() -> Option<u64> {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
    }
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut Counters,
            cb: u32,
        ) -> i32;
    }
    let mut c = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: plain Win32 call filling a correctly sized struct for the current process.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    (ok != 0).then_some(c.working_set as u64 / (1024 * 1024))
}

#[cfg(not(windows))]
fn memory_mb() -> Option<u64> {
    None
}
